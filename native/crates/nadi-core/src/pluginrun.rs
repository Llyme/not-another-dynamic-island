//! What a plugin may run, and what it may keep. A module that asks for `permissions.run` may have the host run a command for it
//! and tell it what it printed; one that asks for `permissions.storage` has a small JSON file of its own.
//!
//! A command is allowed only in the shape its manifest lists: `"run": ["git -C * status --porcelain"]` names the program and
//! every argument, and `*` stands for one argument the module chooses (a folder, a file) that does not start with `-`. So a
//! module can ask `git` about a folder, and cannot give it an option of its own (`-c alias.x=!...` would run anything).

use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const NO_WINDOW: u32 = 0x0800_0000;
const MAX_OUT: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(8);
/// the most a plugin may keep
pub const MAX_STORE: usize = 64 * 1024;

/// Does this command line (program first) fit one of the shapes the manifest allows?
pub fn allowed(templates: &[String], line: &[String]) -> bool {
    if line.is_empty() || line.len() > 24 || line.iter().any(|t| t.len() > 400 || t.chars().any(|c| c == '\0' || c == '\n' || c == '\r')) {
        return false;
    }
    templates.iter().any(|t| {
        let toks: Vec<&str> = t.split_whitespace().collect();
        toks.len() == line.len()
            && toks.iter().zip(line).all(|(want, got)| {
                if *want == "*" {
                    !got.is_empty() && !got.starts_with('-')
                } else {
                    want == got
                }
            })
    })
}

/// Run it (program first) with no window, wait up to a few seconds, and say what came of it.
pub fn run(id: &str, line: &[String]) -> Value {
    let mut cmd = Command::new(&line[0]);
    cmd.args(&line[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(NO_WINDOW);
    }
    let Ok(mut child) = cmd.spawn() else {
        return json!({ "id": id, "code": -1, "stdout": "" });
    };
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.take(MAX_OUT as u64).read_to_end(&mut buf);
        }
        buf
    });
    let start = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break -2;
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    json!({ "id": id, "code": code, "stdout": String::from_utf8_lossy(&bytes).trim_end().to_string() })
}

fn store_path(id: &str) -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("plugin-data").join(format!("{id}.json")))
}

/// What a plugin kept (null when it kept nothing). It is sealed for this Windows user; a file from before that is read as it is.
pub fn store_of(id: &str) -> Value {
    let Some(bytes) = store_path(id).and_then(|p| std::fs::read(p).ok()) else { return Value::Null };
    let plain = crate::seal::unprotect(&bytes).unwrap_or(bytes);
    serde_json::from_slice(&plain).unwrap_or(Value::Null)
}

/// Keep this for a plugin (refused when it is more than it may keep).
pub fn keep(id: &str, value: &Value) {
    let Ok(text) = serde_json::to_string(value) else { return };
    if text.len() > MAX_STORE {
        return;
    }
    if let Some(p) = store_path(id) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        // (written whole, then moved into place: a crash never leaves half of it)
        let Some(sealed) = crate::seal::protect(text.as_bytes()) else { return };
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, sealed).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

/// Does this address belong to one of the hosts the manifest names (`host` or `host:port`), over https?
pub fn net_allows(hosts: &[String], url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    if url.len() > 2000 || url.chars().any(|c| c.is_control() || c == ' ' || c == '\\' || c == '@') {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.split(':').next().unwrap_or("");
    let host_low = host.to_ascii_lowercase();
    // ("*.fandom.com": any one name under it, as "deeprockgalactic.fandom.com"; not "fandom.com" itself, nor deeper)
    let under = |h: &str| {
        h.strip_prefix("*.").is_some_and(|base| {
            let base = base.to_ascii_lowercase();
            host_low.strip_suffix(&base).and_then(|l| l.strip_suffix('.')).is_some_and(|label| !label.is_empty() && !label.contains('.'))
        })
    };
    !host.is_empty() && hosts.iter().any(|h| h.eq_ignore_ascii_case(authority) || h.eq_ignore_ascii_case(host) || under(h))
}

/// The bytes at an address the hosts allow (a picture a widget names), up to `cap`; none when it cannot be had.
pub fn fetch_bytes(hosts: &[String], url: &str, cap: u64) -> Option<Vec<u8>> {
    if !net_allows(hosts, url) {
        return None;
    }
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(20)).build();
    let resp = agent.get(url).set("User-Agent", "nadi/1.1").call().ok()?;
    if resp.status() != 200 {
        return None;
    }
    let mut buf = Vec::new();
    resp.into_reader().take(cap).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// One request a module asked for, to a host its manifest names. The answer is `{ id, status, body, error }`: a status of 0 is a
/// network that did not answer (`error` says why); what came back is cut at 256 KB. Redirects are not followed (they could
/// lead to a host the manifest does not name): the module sees the 3xx.
pub fn fetch(hosts: &[String], req: &Value) -> Value {
    let id = req["id"].as_str().unwrap_or("").to_string();
    let url = req["url"].as_str().unwrap_or("");
    if !net_allows(hosts, url) {
        return json!({ "id": id, "status": 0, "body": "", "error": "That address is not one the plugin may reach." });
    }
    let method = match req["method"].as_str().unwrap_or("GET").to_ascii_uppercase().as_str() {
        "POST" => "POST",
        "PATCH" => "PATCH",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        _ => "GET",
    };
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(20)).build();
    let mut r = agent.request(method, url).set("User-Agent", "nadi/1.1");
    if let Some(h) = req["headers"].as_object() {
        for (k, v) in h.iter().take(12) {
            let k_low = k.to_ascii_lowercase();
            if matches!(k_low.as_str(), "host" | "cookie" | "content-length" | "transfer-encoding" | "connection") {
                continue;
            }
            if let Some(v) = v.as_str() {
                r = r.set(k, v);
            }
        }
    }
    let sent = match req["body"].as_str() {
        Some(b) if method != "GET" => r.send_string(b),
        _ => r.call(),
    };
    let (status, resp) = match sent {
        Ok(resp) => (resp.status(), Some(resp)),
        Err(ureq::Error::Status(code, resp)) => (code, Some(resp)),
        Err(e) => return json!({ "id": id, "status": 0, "body": "", "error": format!("Network error: {e}") }),
    };
    let mut buf = Vec::new();
    // (a redirect is not followed: where it points is told, for the plugin to ask there itself if its manifest lets it)
    let location = resp.as_ref().filter(|_| (300..400).contains(&status)).and_then(|r| r.header("Location").map(str::to_string));
    if let Some(resp) = resp {
        // (a request may ask for more, up to 8 MB: a calendar feed is big)
        let cap = req["max_kb"].as_u64().map_or(MAX_OUT as u64, |k| (k.min(8192) * 1024).max(MAX_OUT as u64));
        let _ = resp.into_reader().take(cap).read_to_end(&mut buf);
    }
    json!({ "id": id, "status": status, "body": String::from_utf8_lossy(&buf), "location": location })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn a_command_must_have_the_shape_the_manifest_lists() {
        let t = vec!["git -C * rev-parse --abbrev-ref HEAD".to_string(), "git -C * diff --numstat HEAD".to_string()];
        assert!(allowed(&t, &line("git -C C:/proj rev-parse --abbrev-ref HEAD")));
        assert!(allowed(&t, &line("git -C C:/proj diff --numstat HEAD")));
        // an option of its own, another subcommand, another program, a longer line
        assert!(!allowed(&t, &line("git -C C:/proj -c alias.x=!calc x")));
        assert!(!allowed(&t, &line("git -C C:/proj push")));
        assert!(!allowed(&t, &line("powershell -C x rev-parse --abbrev-ref HEAD")));
        assert!(!allowed(&t, &line("git -C C:/proj rev-parse --abbrev-ref HEAD --extra")));
        // what stands for the folder cannot be an option
        assert!(!allowed(&t, &line("git -C --upload-pack=calc rev-parse --abbrev-ref HEAD")));
        assert!(!allowed(&[], &line("git status")));
    }

    #[test]
    fn a_star_reaches_one_name_under_its_host() {
        let hosts = vec!["*.fandom.com".to_string()];
        assert!(net_allows(&hosts, "https://deeprockgalactic.fandom.com/api.php?x=1"));
        assert!(net_allows(&hosts, "https://Hades.Fandom.com/api.php"));
        assert!(!net_allows(&hosts, "https://fandom.com/api.php"));
        assert!(!net_allows(&hosts, "https://a.b.fandom.com/"));
        assert!(!net_allows(&hosts, "https://evilfandom.com/"));
        assert!(!net_allows(&hosts, "https://x.fandom.com.evil.io/"));
    }

    #[test]
    fn a_module_reaches_only_the_hosts_its_manifest_names_over_https() {
        let hosts = vec!["api.anthropic.com".to_string(), "localhost:8080".to_string()];
        assert!(net_allows(&hosts, "https://api.anthropic.com/api/oauth/usage"));
        assert!(net_allows(&hosts, "https://API.anthropic.com/x?y=1"));
        assert!(net_allows(&hosts, "https://localhost:8080/a"));
        // another host, plain http, a name that only starts like it, a login in the address, a space
        assert!(!net_allows(&hosts, "https://evil.example.com/api.anthropic.com"));
        assert!(!net_allows(&hosts, "http://api.anthropic.com/"));
        assert!(!net_allows(&hosts, "https://api.anthropic.com.evil.com/"));
        assert!(!net_allows(&hosts, "https://api.anthropic.com@evil.com/"));
        assert!(!net_allows(&hosts, "https://api.anthropic.com/a b"));
        assert!(!net_allows(&[], "https://api.anthropic.com/"));
        // a request to a host that is not named is not sent
        let r = fetch(&hosts, &json!({ "id": "x", "url": "https://example.org/" }));
        assert_eq!((r["id"].as_str(), r["status"].as_i64()), (Some("x"), Some(0)));
    }

    /// Manual: asks the real host. `cargo test --release a_request_reaches_the_host -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_request_reaches_the_host() {
        let r = fetch(&["api.anthropic.com".to_string()], &json!({ "id": "x", "url": "https://api.anthropic.com/api/oauth/usage", "headers": { "Authorization": "Bearer nope", "anthropic-beta": "oauth-2025-04-20" } }));
        println!("{r}");
        assert_eq!(r["status"], 401);
    }

    #[test]
    fn a_command_runs_and_says_what_it_printed() {
        let r = run("a", &["cmd".into(), "/C".into(), "echo".into(), "hello".into()]);
        assert_eq!(r["id"], "a");
        assert_eq!(r["code"], 0);
        assert_eq!(r["stdout"], "hello");
        assert_eq!(run("b", &["definitely-not-a-program-nadi".into()])["code"], -1);
    }
}
