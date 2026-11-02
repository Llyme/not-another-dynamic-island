//! Plugins from a GitHub repository: one repository may hold several, and you choose which to install.
//!
//! "Look" downloads the repository once (a release's `nadi-plugins.tar.gz` when it has one, else the archive of the release
//! tag, else of the default branch), reads it in memory, and finds the plugins in it: every folder with a `manifest.json`, up
//! to three folders down. Each is written to a cache folder and checked as the app checks the plugins in its own folder, so
//! what the answer says (what a plugin asks for, what is wrong with it) is what switching it on would say. Nothing is
//! installed, and nothing is run. "Install" copies the plugins you chose from the cache into the plugins folder (switched off:
//! that is a plugin's way in), with a note of where they came from (`.source.json`) so that a later look knows them.
//!
//! The network is touched only when you ask, only for github.com (and what it redirects a download to), over https, with a
//! size limit; nothing the repository holds can name another address for the app to read.

use crate::manifest::{self, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// the most a download, or what it unpacks to, may be
const MAX_BYTES: usize = 160 * 1024 * 1024;
const MAX_FILES: usize = 6000;
/// what one plugin may hold: its files directly in its folder
const MAX_PLUGIN_FILES: usize = 40;
/// (one with a program of its own, and what it needs to run, may be this big)
const MAX_PLUGIN_BYTES: usize = 96 * 1024 * 1024;
const MAX_DEPTH: usize = 3;
const SOURCE_FILE: &str = ".source.json";
const INDEX_FILE: &str = "nadi-plugins.json";
const ASSET: &str = "nadi-plugins.tar.gz";

// ---------------------------------------------------------------------------------------------------- the link

#[derive(Debug, PartialEq)]
struct Link {
    owner: String,
    repo: String,
    reference: Option<String>,
}

fn word_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) && s != "." && s != ".."
}

/// (a GitHub account is letters, digits and hyphens: a dot means the text was an address of somewhere else)
fn owner_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn ref_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && !s.contains("..") && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
}

/// `owner/repo`, `github.com/owner/repo`, a full address, or one into a branch (`/tree/x`) or a release (`/releases/tag/x`).
fn parse_link(text: &str) -> Result<Link, String> {
    let mut t = text.trim().trim_end_matches('/').trim_end_matches(".git");
    for pre in ["https://", "http://"] {
        if let Some(r) = t.strip_prefix(pre) {
            t = r;
        }
    }
    let t = t.strip_prefix("www.").unwrap_or(t);
    let t = t.strip_prefix("github.com/").unwrap_or(t);
    if t.contains("://") || t.contains('@') || t.contains(' ') {
        return Err("Only a github.com repository can be read.".into());
    }
    let parts: Vec<&str> = t.split(['/', '?', '#']).collect();
    if parts.len() < 2 || !owner_ok(parts[0]) || !word_ok(parts[1]) {
        return Err("Write it as owner/repo, or paste the repository's address.".into());
    }
    let mut reference = None;
    match parts.get(2).copied() {
        Some("tree") if parts.len() > 3 => reference = Some(parts[3..].join("/")),
        Some("releases") if parts.get(3) == Some(&"tag") && parts.len() > 4 => reference = Some(parts[4..].join("/")),
        _ => {}
    }
    if let Some(r) = &reference {
        if !ref_ok(r) {
            return Err("That version name has characters it cannot have.".into());
        }
    }
    Ok(Link { owner: parts[0].to_string(), repo: parts[1].to_string(), reference })
}

// ---------------------------------------------------------------------------------------------------- the network

enum Fetch {
    NotFound,
    Other(String),
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    Some(host.split(':').next()?.to_lowercase())
}

/// What may be asked: GitHub, and the places it keeps a download.
fn host_allowed(host: &str) -> bool {
    matches!(host, "github.com" | "api.github.com" | "codeload.github.com") || host.ends_with(".githubusercontent.com")
}

fn resolve(from: &str, location: &str) -> Option<String> {
    if location.starts_with("https://") {
        return Some(location.to_string());
    }
    let host = host_of(from)?;
    location.starts_with('/').then(|| format!("https://{host}{location}"))
}

/// A page, read whole up to `cap` bytes. A redirect is followed by hand, and only to an address that may be asked.
fn http_get(url: &str, accept: &str, cap: usize) -> Result<Vec<u8>, Fetch> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(40)).user_agent("NADI-plugin-installer").build();
    let mut url = url.to_string();
    for _ in 0..6 {
        let host = host_of(&url).ok_or_else(|| Fetch::Other("The address is not a secure one.".into()))?;
        if !host_allowed(&host) {
            return Err(Fetch::Other(format!("GitHub sent the download to {host}, which NADI does not read.")));
        }
        match agent.get(&url).set("Accept", accept).call() {
            // (with no redirect followed, ureq hands a 3xx back as an answer, not an error)
            Ok(resp) if (301..=308).contains(&resp.status()) => {
                url = resp.header("Location").and_then(|l| resolve(&url, l)).ok_or_else(|| Fetch::Other("GitHub sent no place to go.".into()))?;
            }
            Ok(resp) => {
                let mut out = Vec::new();
                resp.into_reader().take(cap as u64 + 1).read_to_end(&mut out).map_err(|e| Fetch::Other(format!("The download broke off ({e}).")))?;
                if out.len() > cap {
                    return Err(Fetch::Other(format!("It is more than {} MB. Pick a release that carries only the plugins.", cap / 1024 / 1024)));
                }
                return Ok(out);
            }
            Err(ureq::Error::Status(c, resp)) if matches!(c, 301 | 302 | 303 | 307 | 308) => {
                let to = resp.header("Location").and_then(|l| resolve(&url, l)).ok_or_else(|| Fetch::Other("GitHub sent no place to go.".into()))?;
                url = to;
            }
            Err(ureq::Error::Status(404, _)) => return Err(Fetch::NotFound),
            Err(ureq::Error::Status(c @ (403 | 429), _)) => return Err(Fetch::Other(format!("GitHub says to wait a while ({c}). Try again in a few minutes."))),
            Err(ureq::Error::Status(c, _)) => return Err(Fetch::Other(format!("GitHub said {c}."))),
            Err(ureq::Error::Transport(t)) => return Err(Fetch::Other(format!("Cannot reach GitHub ({}).", t.kind()))),
        }
    }
    Err(Fetch::Other("GitHub sent it round in circles.".into()))
}

/// NADI's own repository: its releases carry its installer (named `NADI-Setup-<version>.exe`).
pub const NADI_REPO: &str = "Llyme/not-another-dynamic-island";

fn fetch_text(f: Fetch) -> String {
    match f {
        Fetch::NotFound => "Not found on GitHub.".into(),
        Fetch::Other(s) => s,
    }
}

/// NADI's newest release: its version ("1.2.0") and where its installer is; none when there is no release.
pub fn latest_nadi() -> Result<Option<(String, String)>, String> {
    let tag = latest_tag(NADI_REPO).map_err(fetch_text)?;
    Ok(tag.map(|t| {
        let v = t.trim_start_matches('v').to_string();
        let url = format!("https://github.com/{NADI_REPO}/releases/download/{t}/NADI-Setup-{v}.exe");
        (v, url)
    }))
}

/// Where a release's installer is (tag "v1.2.0": `NADI-Setup-1.2.0.exe`).
pub fn installer_url(tag: &str) -> String {
    let v = tag.trim_start_matches('v');
    format!("https://github.com/{NADI_REPO}/releases/download/{tag}/NADI-Setup-{v}.exe")
}

/// One of NADI's releases: its version, its tag, the day it came out ("2026-11-02"), its notes.
#[derive(Clone, Debug)]
pub struct NadiRelease {
    pub version: String,
    pub tag: String,
    pub date: String,
    pub body: String,
}

// (read from GitHub's feeds, not its API: the API allows a PC 60 questions an hour, shared with everything else asking from it)

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

/// The words between `<name ...>` and `</name>` in a bit of a feed.
fn xml_text<'a>(chunk: &'a str, name: &str) -> Option<&'a str> {
    let open = chunk.find(&format!("<{name}"))?;
    let start = open + chunk[open..].find('>')? + 1;
    let end = start + chunk[start..].find(&format!("</{name}>"))?;
    Some(&chunk[start..end])
}

/// HTML as plain lines: paragraphs and breaks a line each, list items "- " lines, the rest of its tags taken out.
fn html_lines(html: &str) -> String {
    let mut t = html.replace("<li>", "\n- ").replace("</p>", "\n").replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    let mut out = String::new();
    let mut inside = false;
    for c in t.drain(..) {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    let out = xml_unescape(&out);
    // (a long first line GitHub cut in two, "…" at the cut: put back together)
    let c: Vec<char> = out.chars().collect();
    let mut joined = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\u{2026}' {
            let mut k = i + 1;
            while k < c.len() && c[k].is_whitespace() {
                k += 1;
            }
            if k < c.len() && c[k] == '\u{2026}' {
                i = k + 1;
                continue;
            }
        }
        joined.push(c[i]);
        i += 1;
    }
    joined
}

fn entries(feed: &str) -> impl Iterator<Item = &str> {
    feed.split("<entry>").skip(1)
}

/// NADI's releases on GitHub, newest first (its releases feed: the last ten or so).
pub fn nadi_releases() -> Result<Vec<NadiRelease>, String> {
    let bytes = http_get(&format!("https://github.com/{NADI_REPO}/releases.atom"), "application/atom+xml", 4 * 1024 * 1024).map_err(fetch_text)?;
    let feed = String::from_utf8_lossy(&bytes);
    Ok(entries(&feed)
        .filter_map(|e| {
            let tag = xml_text(e, "id")?.rsplit('/').next()?.trim().to_string();
            ref_ok(&tag).then_some(())?;
            Some(NadiRelease {
                version: tag.trim_start_matches('v').to_string(),
                date: xml_text(e, "updated").unwrap_or("").trim().chars().take(10).collect(),
                body: html_lines(&xml_unescape(xml_text(e, "content").unwrap_or(""))),
                tag,
            })
        })
        .collect())
}

/// A tag's latest commits (its commits feed: the last twenty): each one's id and whole message, newest first.
fn commits_of(tag: &str) -> Result<Vec<(String, String)>, String> {
    let bytes = http_get(&format!("https://github.com/{NADI_REPO}/commits/{tag}.atom"), "application/atom+xml", 4 * 1024 * 1024).map_err(fetch_text)?;
    let feed = String::from_utf8_lossy(&bytes);
    Ok(entries(&feed)
        .filter_map(|e| {
            let sha = xml_text(e, "id")?.rsplit('/').next()?.trim().to_string();
            let msg = html_lines(&xml_unescape(xml_text(e, "content").unwrap_or("")));
            Some((sha, msg.trim().to_string()))
        })
        .collect())
}

/// The messages of the commits between two of NADI's tags, oldest first.
pub fn nadi_commits(base: &str, head: &str) -> Result<Vec<String>, String> {
    if !ref_ok(base) || !ref_ok(head) {
        return Err("Not a tag.".into());
    }
    let stop = commits_of(base)?.into_iter().next().map(|(sha, _)| sha);
    let mut out: Vec<String> = commits_of(head)?.into_iter().take_while(|(sha, _)| Some(sha) != stop.as_ref()).map(|(_, m)| m).collect();
    out.reverse();
    Ok(out)
}

/// A release's installer, fetched whole.
pub fn fetch_installer(url: &str) -> Result<Vec<u8>, String> {
    http_get(url, "application/octet-stream", 200 * 1024 * 1024).map_err(fetch_text)
}

/// A release's installer, fetched whole, `progress` told how far along (0..1) as it comes; `stop` set: given up.
pub fn fetch_installer_with(url: &str, progress: &dyn Fn(f32), stop: &std::sync::atomic::AtomicBool) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(120)).user_agent("NADI-plugin-installer").build();
    let mut url = url.to_string();
    for _ in 0..6 {
        let host = host_of(&url).ok_or_else(|| "The address is not a secure one.".to_string())?;
        if !host_allowed(&host) {
            return Err(format!("GitHub sent the download to {host}, which NADI does not read."));
        }
        let resp = match agent.get(&url).set("Accept", "application/octet-stream").call() {
            Ok(r) if (301..=308).contains(&r.status()) => {
                url = r.header("Location").and_then(|l| resolve(&url, l)).ok_or_else(|| "GitHub sent no place to go.".to_string())?;
                continue;
            }
            Ok(r) => r,
            Err(ureq::Error::Status(c, r)) if matches!(c, 301 | 302 | 303 | 307 | 308) => {
                url = r.header("Location").and_then(|l| resolve(&url, l)).ok_or_else(|| "GitHub sent no place to go.".to_string())?;
                continue;
            }
            Err(ureq::Error::Status(404, _)) => return Err("Not found on GitHub.".into()),
            Err(ureq::Error::Status(c, _)) => return Err(format!("GitHub said {c}.")),
            Err(ureq::Error::Transport(t)) => return Err(format!("Cannot reach GitHub ({}).", t.kind())),
        };
        let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let mut reader = resp.into_reader().take(200 * 1024 * 1024);
        let mut out = Vec::with_capacity(total as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Stopped.".into());
            }
            let n = reader.read(&mut buf).map_err(|e| format!("The download broke off ({e})."))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if total > 0 {
                progress((out.len() as f64 / total as f64).min(1.0) as f32);
            }
        }
        return Ok(out);
    }
    Err("GitHub sent it round in circles.".into())
}

/// The tag in the address the page of the latest release sends you on to.
fn tag_from(location: &str) -> Option<String> {
    let tag = location.split("/releases/tag/").nth(1).map(|t| t.split(['?', '#']).next().unwrap_or(t).to_string());
    tag.filter(|t| ref_ok(t))
}

/// The tag of a repository's latest release: its `/releases/latest` page sends you on to `/releases/tag/<tag>`. None when
/// it has no release (that page then sends you to the list of releases, or nowhere).
fn latest_tag(repo: &str) -> Result<Option<String>, Fetch> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(30)).user_agent("NADI-plugin-installer").build();
    // (with no redirect followed, ureq hands a 3xx back as an answer, not an error)
    let tag_of = |resp: &ureq::Response| tag_from(resp.header("Location").unwrap_or(""));
    // (GitHub's edge keeps the answer of this page for a while after a release: a new query and no-cache ask for a fresh one)
    let fresh = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match agent.get(&format!("https://github.com/{repo}/releases/latest?nadi={fresh}")).set("Cache-Control", "no-cache").call() {
        Ok(resp) => Ok(if (301..=308).contains(&resp.status()) { tag_of(&resp) } else { None }),
        Err(ureq::Error::Status(c, resp)) if matches!(c, 301 | 302 | 303 | 307 | 308) => Ok(tag_of(&resp)),
        Err(ureq::Error::Status(404, _)) => Err(Fetch::NotFound),
        Err(ureq::Error::Status(c, _)) => Err(Fetch::Other(format!("GitHub said {c}."))),
        Err(ureq::Error::Transport(t)) => Err(Fetch::Other(format!("Cannot reach GitHub ({}).", t.kind()))),
    }
}

// ---------------------------------------------------------------------------------------------------- the archive

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).to_string()
}

fn octal(b: &[u8]) -> Option<usize> {
    let t = cstr(b);
    let t = t.trim();
    if t.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(t, 8).ok()
}

/// the `path=` of a pax header (`<length> key=value\n` records)
fn pax_path(data: &[u8]) -> Option<String> {
    let mut i = 0;
    while i < data.len() {
        let sp = data[i..].iter().position(|c| *c == b' ')? + i;
        let len: usize = std::str::from_utf8(&data[i..sp]).ok()?.parse().ok()?;
        if len == 0 || i + len > data.len() {
            return None;
        }
        let rec = std::str::from_utf8(&data[sp + 1..i + len]).ok()?.trim_end_matches('\n');
        if let Some(p) = rec.strip_prefix("path=") {
            return Some(p.to_string());
        }
        i += len;
    }
    None
}

/// A file's path made safe: no `..`, no drive, no backslash. None for one that is not.
fn clean_path(p: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." || c.contains(['\\', ':', '\0']) {
            return None;
        }
        out.push(c);
    }
    (!out.is_empty()).then(|| out.join("/"))
}

/// The files of a `.tar.gz`, in memory: regular files only, the paths made safe, the folder the archive wraps everything in
/// (GitHub's `repo-v1.2.0/`) taken off.
fn untar(gz: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(gz).take(MAX_BYTES as u64 + 1).read_to_end(&mut raw).map_err(|_| "The download is not an archive.".to_string())?;
    if raw.len() > MAX_BYTES {
        return Err(format!("It unpacks to more than {} MB. Pick a release that carries only the plugins.", MAX_BYTES / 1024 / 1024));
    }
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let (mut pos, mut long, mut pax) = (0usize, None::<String>, None::<String>);
    while pos + 512 <= raw.len() {
        let h = &raw[pos..pos + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let size = octal(&h[124..136]).ok_or("The archive is damaged.")?;
        let flag = h[156];
        let mut name = cstr(&h[0..100]);
        if &h[257..262] == b"ustar" {
            let prefix = cstr(&h[345..500]);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        pos += 512;
        let end = pos.checked_add(size).filter(|e| *e <= raw.len()).ok_or("The archive is damaged.")?;
        let data = &raw[pos..end];
        pos += size.div_ceil(512) * 512;
        match flag {
            b'x' => pax = pax_path(data),
            b'L' => long = Some(cstr(data)),
            b'0' | 0 => {
                let path = pax.take().or_else(|| long.take()).unwrap_or(name);
                long = None;
                if files.len() >= MAX_FILES {
                    return Err("There are too many files in it.".into());
                }
                if let Some(p) = clean_path(&path) {
                    files.insert(p, data.to_vec());
                }
            }
            b'g' => {}
            _ => {
                pax = None;
                long = None;
            }
        }
    }
    // (a folder that wraps all of it comes off)
    let first: Option<String> = files.keys().next().and_then(|k| k.split('/').next().map(str::to_string));
    if let Some(top) = first {
        if files.keys().all(|k| k.split('/').next() == Some(top.as_str()) && k.contains('/')) {
            files = files.into_iter().map(|(k, v)| (k[top.len() + 1..].to_string(), v)).collect();
        }
    }
    Ok(files)
}

// ---------------------------------------------------------------------------------------------------- finding the plugins

fn cache_root() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("repo-cache"))
}

/// the ids a plugin folder says it replaces
fn incoming_replaces(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|j| j["replaces"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())).unwrap_or_default()
}

#[derive(Serialize, Clone, Debug)]
pub struct FoundPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    /// "luau"
    pub kind: String,
    pub version: String,
    pub permissions: Vec<crate::host::Perm>,
    /// its widgets' names
    pub adds: Vec<String>,
    /// the version of it that is installed, when one is (from this repository or another source)
    pub installed: Option<String>,
    /// the plugins of this repository it takes the place of (they go when it comes)
    pub replaces: Vec<String>,
    /// the services it gives other plugins, and the ones it uses
    #[serde(default)]
    pub provides: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub needs: std::collections::BTreeMap<String, String>,
    /// why it cannot be installed (the manifest is wrong, an id that is not free): the rest of the repository still can
    pub problem: Option<String>,
    /// it asks for a permission this NADI does not know: a newer NADI is needed for it to do anything
    pub needs_newer: bool,
    /// the repository did not have it the last time it was looked at (only in what is kept of a look: see `repo_available`)
    pub new: bool,
}

#[derive(Serialize, Debug)]
pub struct Found {
    pub token: String,
    pub repo: String,
    pub reference: String,
    /// the name and the line the repository's index gives itself, when it has one
    pub name: Option<String>,
    pub about: Option<String>,
    pub plugins: Vec<FoundPlugin>,
    pub size_kb: usize,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Index {
    name: Option<String>,
    about: Option<String>,
    order: Vec<String>,
    hidden: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
struct Source {
    repo: String,
    reference: String,
    at: String,
}

fn read_source(dir: &Path) -> Option<Source> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(SOURCE_FILE)).ok()?).ok()
}

/// "Llyme/nadi-plugins v1.2.0": where a plugin in the plugins folder came from, for the list.
pub fn source_of(dir: &Path) -> Option<String> {
    let s = read_source(dir)?;
    (!s.repo.is_empty()).then(|| if s.reference.is_empty() { s.repo } else { format!("{} {}", s.repo, s.reference) })
}

/// The folders with a manifest, each with the files directly in it.
fn plugin_dirs(files: &BTreeMap<String, Vec<u8>>) -> Vec<(String, Vec<(String, &Vec<u8>)>)> {
    let mut dirs: Vec<String> = Vec::new();
    for k in files.keys() {
        let dir = match k.rsplit_once('/') {
            Some((d, "manifest.json")) => d.to_string(),
            None if k == "manifest.json" => String::new(),
            _ => continue,
        };
        if dir.split('/').filter(|c| !c.is_empty()).count() <= MAX_DEPTH {
            dirs.push(dir);
        }
    }
    dirs.into_iter()
        .map(|d| {
            let mine: Vec<(String, &Vec<u8>)> = files
                .iter()
                .filter_map(|(k, v)| {
                    let (parent, name) = k.rsplit_once('/').unwrap_or(("", k.as_str()));
                    // (its own files, and its program's: `bin/`, what it needs to run)
                    let bin = if d.is_empty() { "bin".to_string() } else { format!("{d}/bin") };
                    if parent == bin && !name.starts_with('.') {
                        return Some((format!("bin/{name}"), v));
                    }
                    (parent == d && !name.starts_with('.')).then(|| (name.to_string(), v))
                })
                .collect();
            (d, mine)
        })
        .collect()
}

fn stage(root: &Path, id: &str, files: &[(String, &Vec<u8>)]) -> Result<PathBuf, String> {
    let to = root.join(id);
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    for (name, bytes) in files {
        let at = to.join(name);
        if let Some(parent) = at.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(at, bytes).map_err(|e| e.to_string())?;
    }
    Ok(to)
}

fn unknown_permissions(manifest_bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(manifest_bytes).map(|j| manifest::unknown_permissions(&j)).unwrap_or(false)
}

fn adds_of(m: &Manifest) -> Vec<String> {
    m.widgets.iter().map(|w| if w.name.is_empty() { w.id.clone() } else { w.name.clone() }).collect()
}

fn look(text: String, wanted: Option<String>) -> Result<Found, String> {
    let link = parse_link(&text)?;
    let repo = format!("{}/{}", link.owner, link.repo);
    let wanted = wanted.filter(|r| !r.is_empty()).or(link.reference.clone());
    if let Some(r) = &wanted {
        if !ref_ok(r) {
            return Err("That version name has characters it cannot have.".into());
        }
    }
    let not_found = || format!("GitHub has no repository {repo}. Check the spelling. A private one cannot be read: NADI never asks for a sign-in.");
    let other = |f: Fetch| match f {
        Fetch::NotFound => not_found(),
        Fetch::Other(s) => s,
    };

    // which version, and where its files are. No use of GitHub's API (it allows few questions an hour to one address, and many
    // share one): the page of the latest release sends you on to the page of its tag, and a release's file has an address
    // that can be guessed.
    let mut asset: Option<String> = None;
    let reference = match wanted {
        Some(r) => r,
        None => match latest_tag(&repo).map_err(&other)? {
            Some(tag) => tag,
            // (no release: the repository's own default branch is `HEAD`)
            None => "HEAD".to_string(),
        },
    };
    if reference != "HEAD" {
        // (a release that carries only the plugins, when it has one)
        asset = Some(format!("https://github.com/{repo}/releases/download/{reference}/{ASSET}"));
    }
    let tarball = format!("https://codeload.github.com/{repo}/tar.gz/{reference}");
    // (whether it is a release's built file: a repository's own source holds no built module)
    let (fetched, built) = match &asset {
        Some(u) => match http_get(u, "application/octet-stream", MAX_BYTES) {
            Err(Fetch::NotFound) => (http_get(&tarball, "application/octet-stream", MAX_BYTES), false),
            other => (other, true),
        },
        None => (http_get(&tarball, "application/octet-stream", MAX_BYTES), false),
    };
    let bytes = fetched.map_err(|f| match f {
        Fetch::NotFound => format!("{repo} has no version called {reference}."),
        Fetch::Other(s) => s,
    })?;
    let files = untar(&bytes)?;

    let index: Index = files.get(INDEX_FILE).and_then(|b| serde_json::from_slice(b).ok()).unwrap_or_default();

    // the cache: a folder for this look, and no older ones
    let root = cache_root().ok_or("There is no place to keep the download.")?;
    let _ = std::fs::remove_dir_all(&root);
    let mut token = [0u8; 8];
    getrandom::getrandom(&mut token).map_err(|_| "No luck.".to_string())?;
    let token: String = token.iter().map(|b| format!("{b:02x}")).collect();
    let stage_root = root.join(&token);

    let plugins_dir = crate::paths::plugins_dir();
    let mut found: Vec<FoundPlugin> = Vec::new();
    for (dir, mine) in plugin_dirs(&files) {
        let fallback = dir.rsplit('/').next().unwrap_or("").to_string();
        let total: usize = mine.iter().map(|(_, b)| b.len()).sum();
        if mine.len() > MAX_PLUGIN_FILES || total > MAX_PLUGIN_BYTES {
            found.push(FoundPlugin { id: fallback.clone(), name: fallback, description: String::new(), kind: String::new(), version: String::new(), permissions: vec![], adds: vec![], installed: None, replaces: vec![], provides: Default::default(), needs: Default::default(), problem: Some("It holds more than a plugin should.".into()), needs_newer: false, new: false });
            continue;
        }
        // (staged under a name of its own first: its id is only known once its manifest has been read)
        let slot = format!("p{}", found.len());
        let Ok(folder) = stage(&stage_root, &slot, &mine) else { continue };
        let Some((m, mut problem)) = manifest::read(&folder) else { continue };
        let id = if m.id.is_empty() { fallback.clone() } else { m.id.clone() };
        if !built && problem.as_deref().is_some_and(|p| p.contains("is not in the folder")) {
            problem = Some(format!("Not built: {repo} has no release with {ASSET} yet."));
        }
        if !manifest::valid_id(&id) {
            problem.get_or_insert("The id must be lower case letters, digits, '.', '-' or '_'.".into());
        }
        if index.hidden.iter().any(|h| *h == id) {
            continue;
        }
        if found.iter().any(|f| f.id == id) {
            problem.get_or_insert(format!("Another plugin in this repository has the id \"{id}\"."));
        }
        let mine_dir = plugins_dir.as_ref().map(|d| d.join(&id));
        let installed = mine_dir.as_ref().filter(|d| d.join("manifest.json").is_file()).map(|d| {
            let v = std::fs::read_to_string(d.join("manifest.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|j| j["version"].as_str().map(str::to_string)).unwrap_or_default();
            let from_here = read_source(d).map_or(false, |s| s.repo.eq_ignore_ascii_case(&repo));
            if !from_here {
                problem.get_or_insert("A plugin with this id is already here, from somewhere else.".into());
            }
            v
        });
        let needs_newer = mine.iter().find(|(n, _)| n == "manifest.json").map_or(false, |(_, b)| unknown_permissions(b));
        let perms: Vec<crate::host::Perm> = manifest::manifest_words(&m).into_iter().map(|(name, detail, why)| crate::host::Perm { name, detail, why }).collect();
        // (a plugin that is wrong keeps its folder out of the install: only what is whole gets in)
        found.push(FoundPlugin { adds: adds_of(&m), kind: m.kind.clone(), permissions: perms, installed, replaces: m.replaces.clone(), provides: m.provides.clone(), needs: m.needs.clone(), problem, name: if m.name.is_empty() { id.clone() } else { m.name.clone() }, description: m.description.clone(), version: m.version.clone(), id, needs_newer, new: false });
        // (the slot is renamed to the id, for the install)
        let _ = std::fs::rename(stage_root.join(&slot), stage_root.join(&found.last().unwrap().id));
    }
    // (the order the index gives, the rest by name)
    found.sort_by(|a, b| {
        let ia = index.order.iter().position(|x| *x == a.id).unwrap_or(usize::MAX);
        let ib = index.order.iter().position(|x| *x == b.id).unwrap_or(usize::MAX);
        ia.cmp(&ib).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    // (the source of each is written beside it now, so that the install is only a copy)
    for f in found.iter().filter(|f| f.problem.is_none()) {
        let src = Source { repo: repo.clone(), reference: if reference == "HEAD" { String::new() } else { reference.clone() }, at: chrono::Utc::now().to_rfc3339() };
        let _ = std::fs::write(stage_root.join(&f.id).join(SOURCE_FILE), serde_json::to_string(&src).unwrap_or_default());
    }
    Ok(Found { token, repo, reference, name: index.name, about: index.about, plugins: found, size_kb: bytes.len() / 1024 })
}

/// Open a repository's page on GitHub in the browser (the link is read as any link is: only github.com, only a repository).
pub fn open_page(repo: &str) -> Result<(), String> {
    let link = parse_link(repo)?;
    crate::win::open_url(&format!("https://github.com/{}/{}", link.owner, link.repo));
    Ok(())
}

/// Remove every plugin that came from a repository (the ones it put in the plugins folder, by their marker). What the person made or got
/// from elsewhere is not touched. The ids that went (the host is to be told to look again).
pub fn remove_repo(repo: &str) -> Result<Vec<String>, String> {
    let link = parse_link(repo)?;
    let name = format!("{}/{}", link.owner, link.repo);
    let root = crate::paths::plugins_dir().ok_or("There is no plugins folder.")?;
    let mut gone = Vec::new();
    for e in std::fs::read_dir(&root).map_err(|e| e.to_string())?.flatten() {
        let path = e.path();
        let id = e.file_name().to_string_lossy().to_string();
        if !path.is_dir() || !manifest::valid_id(&id) || !read_source(&path).map_or(false, |s| s.repo.eq_ignore_ascii_case(&name)) {
            continue;
        }
        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        gone.push(id);
    }
    let mut all = load_repos();
    all.remove(&name);
    if let Some(file) = repos_file() {
        let _ = std::fs::write(file, serde_json::to_string(&Value::Object(all)).unwrap_or_default());
    }
    Ok(gone)
}

/// Remove one plugin from the plugins folder (one that came from a repository, or any other).
pub fn remove_plugin(id: &str) -> Result<(), String> {
    if !manifest::valid_id(id) {
        return Err("No such plugin.".into());
    }
    let dir = crate::paths::plugins_dir().ok_or("There is no plugins folder.")?.join(id);
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

/// The tag of the latest release of a repository, for the update button: one small request. None when it has no release.
pub fn latest(repo: &str) -> Result<Option<String>, String> {
    let link = parse_link(repo)?;
    match latest_tag(&format!("{}/{}", link.owner, link.repo)) {
        Ok(t) => Ok(t),
        Err(Fetch::NotFound) => Ok(None),
        Err(Fetch::Other(e)) => Err(e),
    }
}

/// What an install did: the plugins that came in, and whether each is to be on (`is_on` says what was on before).
pub struct Installed {
    pub ids: Vec<String>,
    pub on: Vec<(String, bool)>,
}

/// The install of a look: its plugins you chose are copied into the plugins folder. `drop_cache` takes the look's files away when it is
/// done. With `keep_on` (an update) a plugin that was on stays on; a plugin that takes the place of others is on when one of them was.
fn install_from(token: &str, ids: Vec<String>, keep_on: bool, drop_cache: bool, is_on: &dyn Fn(&str) -> bool) -> Result<Installed, String> {
    if token.len() != 16 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("That look is over. Look again.".into());
    }
    let from = cache_root().ok_or("There is no place the download was kept.")?.join(token);
    if !from.is_dir() {
        return Err("That look is over. Look again.".into());
    }
    let to = crate::paths::plugins_dir().ok_or("There is no plugins folder.")?;
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    let mut on = Vec::new();
    for id in ids {
        if !manifest::valid_id(&id) {
            continue;
        }
        let src = from.join(&id);
        if !src.join("manifest.json").is_file() {
            continue;
        }
        let dst = to.join(&id);
        // (an older one of this repository is replaced; one from anywhere else was kept out by the look)
        let incoming = read_source(&src);
        if dst.exists() {
            let theirs = read_source(&dst);
            let same = matches!((&incoming, &theirs), (Some(a), Some(b)) if a.repo.eq_ignore_ascii_case(&b.repo));
            if !same {
                continue;
            }
            std::fs::remove_dir_all(&dst).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(&src).map_err(|e| e.to_string())?.flatten() {
            if e.path().is_file() {
                std::fs::copy(e.path(), dst.join(e.file_name())).map_err(|e| e.to_string())?;
            } else if e.file_name() == "bin" && e.path().is_dir() {
                // (its program, and what it needs to run)
                std::fs::create_dir_all(dst.join("bin")).map_err(|e| e.to_string())?;
                for b in std::fs::read_dir(e.path()).map_err(|e| e.to_string())?.flatten() {
                    if b.path().is_file() {
                        std::fs::copy(b.path(), dst.join("bin").join(b.file_name())).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        // (two plugins that became one: the old ones of this repository go, and the new one is on when one of them was)
        let mut was_on = false;
        for old in incoming_replaces(&src) {
            let gone = to.join(&old);
            if old != id && manifest::valid_id(&old) && read_source(&gone).zip(incoming.as_ref()).map_or(false, |(a, b)| a.repo.eq_ignore_ascii_case(&b.repo)) {
                was_on |= is_on(&old);
                on.push((old.clone(), false));
                std::fs::remove_dir_all(&gone).map_err(|e| e.to_string())?;
            }
        }
        on.push((id.clone(), was_on || (keep_on && is_on(&id))));
        done.push(id);
    }
    if drop_cache {
        let _ = std::fs::remove_dir_all(&from);
    }
    Ok(Installed { ids: done, on })
}

// ---------------------------------------------------------------------------------------------------- what the repositories have

/// the looks are one at a time: each wipes the cache of the one before
static BUSY: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// the last look: (the repository, its token, when), so that ticking several plugins in a row downloads once
static LAST: std::sync::Mutex<Option<(String, String, std::time::Instant)>> = std::sync::Mutex::new(None);
const LAST_FOR: Duration = Duration::from_secs(600);

fn repos_file() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("repos.json"))
}

fn load_repos() -> serde_json::Map<String, Value> {
    repos_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// What a look found, kept for the settings page (which shows what a repository has that is not installed, with no network): the plugins
/// with their words and permissions, and which ones are new (the repository had not got them the last time it was looked at).
fn keep_found(f: &Found) -> Value {
    let mut all = load_repos();
    let before: Option<Vec<String>> = all.get(&f.repo).and_then(|e| e["plugins"].as_array()).map(|a| a.iter().filter_map(|p| p["id"].as_str().map(str::to_string)).collect());
    let plugins: Vec<Value> = f
        .plugins
        .iter()
        .map(|p| {
            let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
            // (the first time a repository is looked at, nothing in it is news)
            v["new"] = json!(before.as_ref().map_or(false, |b| !b.contains(&p.id)));
            v
        })
        .collect();
    let entry = json!({ "reference": f.reference, "at": chrono::Utc::now().to_rfc3339(), "name": f.name, "plugins": plugins });
    all.insert(f.repo.clone(), entry.clone());
    if let Some(file) = repos_file() {
        let _ = std::fs::write(file, serde_json::to_string(&Value::Object(all)).unwrap_or_default());
    }
    entry
}

/// What the repositories had the last time they were looked at, by name (`Llyme/nadi-plugins`): no network.
pub fn available() -> Value {
    Value::Object(load_repos())
}

/// Look in a repository (its latest release) and keep what is in it, for the list of plugins you do not have: its entry, as `available`
/// has it. (It downloads: call it off the window's thread.)
pub fn sync(repo: &str) -> Result<Value, String> {
    let _one = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let found = look(repo.to_string(), None)?;
    *LAST.lock().unwrap() = Some((found.repo.clone(), found.token.clone(), std::time::Instant::now()));
    Ok(keep_found(&found))
}

/// Install some plugins of a repository (the ones you ticked): its latest release is looked in (once for a few in a row), and they are
/// copied into the plugins folder.
pub fn get(repo: &str, ids: Vec<String>, keep_on: bool, is_on: &dyn Fn(&str) -> bool) -> Result<Installed, String> {
    let _one = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let name = parse_link(repo).map(|l| format!("{}/{}", l.owner, l.repo))?;
    // (the look of a moment ago, if it is still there)
    let fresh = LAST
        .lock()
        .unwrap()
        .clone()
        .filter(|(r, token, at)| r.eq_ignore_ascii_case(&name) && at.elapsed() < LAST_FOR && cache_root().map_or(false, |c| ids.iter().all(|id| c.join(token).join(id).join("manifest.json").is_file())))
        .map(|(_, token, _)| token);
    let token = match fresh {
        Some(t) => t,
        None => {
            let found = look(name.clone(), None)?;
            keep_found(&found);
            *LAST.lock().unwrap() = Some((found.repo.clone(), found.token.clone(), std::time::Instant::now()));
            found.token
        }
    };
    install_from(&token, ids, keep_on, false, is_on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_that_asks_for_a_permission_this_nadi_does_not_know_needs_a_newer_one() {
        assert!(!unknown_permissions(br#"{ "id": "a", "permissions": { "media": true, "battery": true } }"#));
        assert!(unknown_permissions(br#"{ "id": "a", "permissions": { "media": true, "teleport": true } }"#));
        assert!(!unknown_permissions(br#"{ "id": "a" }"#));
        assert!(!unknown_permissions(b"not json"));
    }

    #[test]
    fn the_real_release_downloads() {
        if std::env::var_os("NADI_TEST_NET").is_none() {
            return;
        }
        let url = "https://github.com/Llyme/nadi-plugins/releases/download/v1.0.0/nadi-plugins.tar.gz";
        match http_get(url, "application/octet-stream", MAX_BYTES) {
            Ok(b) => assert!(untar(&b).unwrap().keys().any(|k| k.ends_with("ics-calendar/manifest.json")), "{} bytes", b.len()),
            Err(Fetch::NotFound) => panic!("not found"),
            Err(Fetch::Other(e)) => panic!("{e}"),
        }
    }

    #[test]
    fn a_real_release_file_is_read() {
        let Some(path) = std::env::var_os("NADI_TEST_TGZ") else { return };
        let bytes = std::fs::read(path).unwrap();
        let files = untar(&bytes).unwrap();
        assert!(files.keys().any(|k| k.ends_with("ics-calendar/manifest.json")), "{:?}", files.keys().collect::<Vec<_>>());
    }

    #[test]
    fn the_latest_release_is_read_from_where_its_page_sends_you() {
        assert_eq!(tag_from("https://github.com/Llyme/nadi-plugins/releases/tag/v1.0.0").as_deref(), Some("v1.0.0"));
        assert_eq!(tag_from("/Llyme/nadi-plugins/releases/tag/v2.1?x=1").as_deref(), Some("v2.1"));
        // no release: the page sends you to the list of them
        assert_eq!(tag_from("https://github.com/Llyme/nadi-plugins/releases"), None);
        assert_eq!(tag_from(""), None);
    }

    #[test]
    fn a_link_is_read_in_the_ways_people_write_it() {
        let want = |o: &str, r: &str, v: Option<&str>| Link { owner: o.into(), repo: r.into(), reference: v.map(str::to_string) };
        assert_eq!(parse_link("Llyme/nadi-plugins").unwrap(), want("Llyme", "nadi-plugins", None));
        assert_eq!(parse_link(" github.com/Llyme/nadi-plugins/ ").unwrap(), want("Llyme", "nadi-plugins", None));
        assert_eq!(parse_link("https://github.com/Llyme/nadi-plugins.git").unwrap(), want("Llyme", "nadi-plugins", None));
        assert_eq!(parse_link("https://github.com/Llyme/nadi-plugins/tree/v1.2.0").unwrap(), want("Llyme", "nadi-plugins", Some("v1.2.0")));
        assert_eq!(parse_link("https://github.com/Llyme/nadi-plugins/releases/tag/v2").unwrap(), want("Llyme", "nadi-plugins", Some("v2")));
        assert!(parse_link("https://evil.example/a/b").is_err());
        assert!(parse_link("just-one-word").is_err());
        assert!(parse_link("a/b/tree/../../etc").is_err());
        assert!(parse_link("../x").is_err());
    }

    #[test]
    fn only_github_may_be_asked_and_a_redirect_is_not_trusted() {
        assert!(host_allowed("github.com"));
        assert!(host_allowed("codeload.github.com"));
        assert!(host_allowed("objects.githubusercontent.com"));
        assert!(host_allowed("release-assets.githubusercontent.com"));
        assert!(!host_allowed("githubusercontent.com.evil.example"));
        assert!(!host_allowed("evil.example"));
        assert_eq!(host_of("https://github.com/a/b").as_deref(), Some("github.com"));
        assert_eq!(host_of("https://user@evil.example/x").as_deref(), Some("evil.example"));
        assert_eq!(host_of("http://github.com/a"), None);
        assert_eq!(resolve("https://github.com/a/b", "/x/y").as_deref(), Some("https://github.com/x/y"));
        assert_eq!(resolve("https://github.com/a/b", "ftp://x"), None);
    }

    #[test]
    fn paths_in_an_archive_cannot_leave_their_folder() {
        assert_eq!(clean_path("repo-v1/plugins/weather/manifest.json").as_deref(), Some("repo-v1/plugins/weather/manifest.json"));
        assert_eq!(clean_path("./a//b").as_deref(), Some("a/b"));
        assert_eq!(clean_path("a/../../etc/passwd"), None);
        assert_eq!(clean_path("C:/Windows/x"), None);
        assert_eq!(clean_path("a\\b"), None);
    }

    /// a tar with the given files, as `tar` writes it (ustar), gzipped
    fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut tar = Vec::new();
        for (name, data) in files {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..107].copy_from_slice(b"0000644");
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[156] = b'0';
            h[257..262].copy_from_slice(b"ustar");
            tar.extend_from_slice(&h);
            tar.extend_from_slice(data);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.extend_from_slice(&[0u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn an_archive_is_read_and_the_folder_around_it_comes_off() {
        let gz = tar_gz(&[("repo-v1/plugins/a/manifest.json", b"{}"), ("repo-v1/plugins/a/a.wasm", b"\0asm"), ("repo-v1/README.md", b"hi"), ("repo-v1/../evil", b"x")]);
        let files = untar(&gz).unwrap();
        assert_eq!(files.keys().map(String::as_str).collect::<Vec<_>>(), vec!["README.md", "plugins/a/a.wasm", "plugins/a/manifest.json"]);
        assert_eq!(files["plugins/a/a.wasm"], b"\0asm");
    }

    #[test]
    fn plugins_are_the_folders_with_a_manifest_up_to_three_down() {
        let gz = tar_gz(&[
            ("r/plugins/weather/manifest.json", b"{}"),
            ("r/plugins/audio/manifest.json", b"{}"),
            ("r/plugins/audio/audio.wasm", b"x"),
            ("r/a/b/c/d/deep/manifest.json", b"{}"),
            ("r/sdk/lib.rs", b"x"),
        ]);
        let files = untar(&gz).unwrap();
        let dirs: Vec<String> = plugin_dirs(&files).into_iter().map(|(d, _)| d).collect();
        assert_eq!(dirs, vec!["plugins/audio", "plugins/weather"]);
        let audio = plugin_dirs(&files).into_iter().find(|(d, _)| d == "plugins/audio").unwrap().1;
        assert_eq!(audio.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["audio.wasm", "manifest.json"]);
    }
}
