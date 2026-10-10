//! Page Reader: "what are you browsing". A plugin (see native.rs): a card for each page you have open, in the expanded
//! island. It shows nothing on the collapsed island.
//!
//! The window title only names the page, so the rest comes from the browser extension (the `nadi-chrome` repo): the
//! page describes itself, exactly, and costs the browser next to nothing (see `ext`, the link to the extension, which
//! this plugin owns). Without the extension connected, a browser is not read at all: the card shows what the window
//! title says and nothing more.
//!
//! Private windows, blocked sites (banking, mail, health, your own list) and pages the extension did not describe (a
//! sign-in page) are never shown.

mod ext;
mod page;
mod pagekind;
mod pagetext;

use crate::browsers::is_browser_exe;
use crate::exeinfo;
use crate::native::{manifest_of, old_flag, Adopted, Ctx, Native};
use crate::plugins::Manifest;
use crate::winutil::{exe_stem, foreground_info};
use pagekind::{Evidence, PageKind};
use pagetext::PagePreview;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// how often open tabs are re-enumerated to drop closed ones from the MRU list
const PRUNE_EVERY_S: u64 = 5;
/// a page missing from every tab strip must miss this many re-checks in a row
/// before it is dropped: mid-load titles flicker, closed tabs do not come back
const PRUNE_GRACE_S: u64 = 10;
/// how many pages get their own card, across all browsers: the ones focused last, and then the other open tabs the
/// browser extension has read (see `seed`)
pub const BROWSE_MRU_CAP: usize = 8;
/// a page not focused for this long drops out of the MRU list
const BROWSE_MRU_TTL_S: u64 = 180;
/// the cards are shown while a browser window was seen this recently
const FRESH_S: u64 = 30;

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("pages.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin
pub fn adopted(old: &Value, default_on: bool) -> Adopted {
    let mut values = serde_json::Map::new();
    values.insert("images".into(), Value::Bool(old_flag(old, "page_images", true)));
    Adopted { on: old_flag(old, "page_preview", default_on), values }
}

#[derive(Serialize, Clone, Default)]
pub struct BrowseInfo {
    /// site of the page in front of you
    pub domain: Option<String>,
    /// what the page says
    pub preview: Option<PagePreview>,
    /// what kind of page it is, and what is worth saying about it
    pub kind: Option<PageKind>,
    /// the page's main picture: an id for `page_image`, so the card is not sent the bytes every time
    pub image: Option<u64>,
    /// a private window or a blocked site: nothing is read, and the card shows no page
    pub blocked: bool,
}

impl BrowseInfo {
    /// nothing was read (yet): no card worth showing
    pub fn is_empty(&self) -> bool {
        !self.blocked && self.domain.is_none() && self.preview.is_none() && self.kind.is_none() && self.image.is_none()
    }
}

pub(crate) fn is_new_tab_or_empty(url: Option<&str>, title: &str) -> bool {
    const EMPTY_TITLES: &[&str] = &["new tab", "new tab page", "start page", "home", "about:blank", ""];
    let t = title.to_lowercase();
    if EMPTY_TITLES.contains(&t.as_str()) {
        return true;
    }
    if let Some(u) = url {
        let ul = u.to_lowercase();
        const NEW_TAB_URLS: &[&str] = &[
            "about:blank", "chrome://newtab", "chrome://new-tab-page", "chrome://home",
            "edge://newtab", "edge://new-tab-page", "brave://newtab",
            "vivaldi://newtab", "vivaldi://startpage", "opera://newtab", "opera://startpage",
            "firefox://newtab", "about:newtab", "about:home",
        ];
        if NEW_TAB_URLS.iter().any(|&prefix| ul.starts_with(prefix)) || ul == "about:newtab" || ul == "about:home" {
            return true;
        }
    }
    false
}

/// "Rust docs - Google Chrome - Michael" -> "Rust docs"
pub fn clean_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut cut = title.len();
    for marker in [
        " - google chrome",
        " - microsoft edge",
        " - microsoft\u{200b} edge",
        " - brave",
        " - vivaldi",
        " - opera",
        " \u{2014} mozilla firefox",
        " - mozilla firefox",
    ] {
        if let Some(i) = lower.find(marker) {
            cut = cut.min(i);
        }
    }
    let mut t = title[..cut].trim().to_string();
    // Edge and Chrome put the profile's name before their own: "Zeppelin - Wikipedia - Personal"
    if let Some(i) = t.rfind(" - ") {
        let profile = t[i + 3..].trim().to_lowercase();
        if matches!(profile.as_str(), "personal" | "work" | "school" | "default" | "guest") || profile.starts_with("profile ") {
            t.truncate(i);
        }
    }
    // Edge: "Page and 3 more pages - Profile" (the tab count and profile trail the title)
    if let Some(i) = t.rfind(" and ") {
        let mut words = t[i + 5..].split_whitespace();
        let counted = words.next().map_or(false, |n| n.chars().all(|c| c.is_ascii_digit()));
        if counted && words.next() == Some("more") {
            t.truncate(i);
        }
    }
    t
}

fn domain_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// key for per-page info: `exe|title`
pub(crate) fn browse_key(exe: &str, title: &str) -> String {
    format!("{exe}|{title}")
}

// ---------------------------------------------------------------------------------------- the pages that have cards

#[derive(Clone)]
struct BrowseMru {
    exe: String,
    title: String,
    app: String,
    first_seen: Instant,
    last_seen: Instant,
}

/// Most-recent-first push with dedup + cap. Pure so it is unit-testable.
fn mru_push(mru: &mut VecDeque<BrowseMru>, exe: String, title: String, app: String, now: Instant) {
    if let Some(pos) = mru.iter().position(|e| e.exe == exe && e.title == title) {
        let mut e = mru.remove(pos).unwrap();
        e.last_seen = now;
        if !app.is_empty() {
            e.app = app;
        }
        mru.push_front(e);
        return;
    }
    mru.push_front(BrowseMru { exe, title, app, first_seen: now, last_seen: now });
    while mru.len() > BROWSE_MRU_CAP {
        mru.pop_back();
    }
}

#[derive(Default)]
struct Reader {
    ext: Arc<ext::ExtState>,
    /// recently focused pages, most recent first, across all browser windows
    mru: Mutex<VecDeque<BrowseMru>>,
    /// per-page info keyed by `browse_key`
    info: Mutex<HashMap<String, BrowseInfo>>,
    /// the browser in front is a private / incognito window
    private: AtomicBool,
    /// a browser window was last seen
    seen: Mutex<Option<Instant>>,
    /// the link to the extension is being served
    serving: AtomicBool,
}

impl Reader {
    /// (browser exe, page title) of the most recently focused page, while browsing is recent
    fn active(&self) -> Option<(String, String)> {
        if !self.fresh() {
            return None;
        }
        self.mru.lock().unwrap().front().map(|e| (e.exe.clone(), e.title.clone()))
    }

    fn fresh(&self) -> bool {
        self.seen.lock().unwrap().map_or(false, |t| t.elapsed() <= Duration::from_secs(FRESH_S))
    }

    /// The tabs the extension has read are open, so they have cards as well, behind the ones that were visited: they
    /// go to the back of the list (a page that is already there keeps its place) while there is room, and stay
    /// as long as the tab does (the caller says so every few seconds).
    fn seed(&self, exe: &str, titles: &[String]) {
        let mut mru = self.mru.lock().unwrap();
        let now = Instant::now();
        let app = mru.iter().find(|e| e.exe.eq_ignore_ascii_case(exe)).map(|e| e.app.clone()).unwrap_or_default();
        for t in titles {
            if let Some(e) = mru.iter_mut().find(|e| e.exe.eq_ignore_ascii_case(exe) && e.title == *t) {
                e.last_seen = now;
            } else if mru.len() < BROWSE_MRU_CAP && !is_new_tab_or_empty(None, t) {
                mru.push_back(BrowseMru { exe: exe.to_string(), title: t.clone(), app: app.clone(), first_seen: now, last_seen: now });
            }
        }
    }

    /// A tab went to another page of its guide (see `ExtState::navigate`): the card of the page it left is the card
    /// of the page it is on now, in the same place in the list. Returns the browsers' exes, one for each card that moved.
    fn rename(&self, browser: &str, old: &str, new: &str) -> Vec<String> {
        let mut moved: Vec<String> = Vec::new();
        {
            let mut mru = self.mru.lock().unwrap();
            let mut i = 0;
            while i < mru.len() {
                let e = &mru[i];
                if e.title == old && ext::browser_of(&e.exe) == browser {
                    let exe = e.exe.clone();
                    if mru.iter().any(|x| x.exe == exe && x.title == new) {
                        mru.remove(i); // (the new page is in the list already)
                        moved.push(exe);
                        continue;
                    }
                    mru[i].title = new.to_string();
                    moved.push(exe);
                }
                i += 1;
            }
        }
        let mut info = self.info.lock().unwrap();
        for exe in &moved {
            if let Some(i) = info.remove(&browse_key(exe, old)) {
                info.entry(browse_key(exe, new)).or_insert(i);
            }
        }
        moved
    }

    /// store a finished page read under its key, dropping info for evicted pages
    fn upsert(&self, key: String, i: BrowseInfo) {
        let keys: Vec<String> = self.mru.lock().unwrap().iter().map(|e| browse_key(&e.exe, &e.title)).collect();
        let mut info = self.info.lock().unwrap();
        info.insert(key, i);
        info.retain(|k, _| keys.contains(k));
    }

    /// forget one page (new tab, blocked, private): the other MRU cards stay
    fn remove(&self, key: &str) {
        self.info.lock().unwrap().remove(key);
    }

    /// (exe, title) of every MRU page, most recent first
    fn keys(&self) -> Vec<(String, String)> {
        self.mru.lock().unwrap().iter().map(|e| (e.exe.clone(), e.title.clone())).collect()
    }

    /// drop MRU pages `keep` rejects (closed tab or window), then forget their info.
    /// Entries with no data (a browser that exposes no tab strip) are kept by the
    /// caller returning true, so the TTL stays the fallback there.
    fn prune<F: FnMut(&str, &str) -> bool>(&self, mut keep: F) {
        let keys: Vec<String> = {
            let mut mru = self.mru.lock().unwrap();
            mru.retain(|e| keep(&e.exe, &e.title));
            mru.iter().map(|e| browse_key(&e.exe, &e.title)).collect()
        };
        self.info.lock().unwrap().retain(|k, _| keys.contains(k));
    }

    fn clear(&self) {
        self.mru.lock().unwrap().clear();
        self.info.lock().unwrap().clear();
        *self.seen.lock().unwrap() = None;
        self.private.store(false, Ordering::Relaxed);
    }

    /// the cards: one for every recently focused page, most recent first, across all browsers
    fn cards(&self) -> Vec<Value> {
        if !self.fresh() {
            return Vec::new();
        }
        let entries: Vec<(String, String, String, f64)> = self
            .mru
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.last_seen.elapsed() <= Duration::from_secs(BROWSE_MRU_TTL_S))
            .map(|e| (e.exe.clone(), e.title.clone(), e.app.clone(), e.first_seen.elapsed().as_secs_f64()))
            .collect();
        let infos = self.info.lock().unwrap();
        let mut out = Vec::new();
        for (exe, title, app, going) in entries.into_iter().take(BROWSE_MRU_CAP) {
            if title.is_empty() {
                continue;
            }
            let info = infos.get(&browse_key(&exe, &title)).cloned().unwrap_or_default();
            // (a video has no page card: the media card covers it)
            if info.blocked || info.is_empty() || info.kind.as_ref().map_or(false, |k| k.id == "video") {
                continue;
            }
            let app_name = if app.is_empty() { exe_stem(&exe) } else { app };
            out.push(json!({
                "exe_path": (!exe.is_empty()).then_some(exe.clone()),
                "category": "browsing",
                "app_name": app_name,
                "icon": (!exe.is_empty()).then(|| exeinfo::lookup(&exe).icon).flatten(),
                "going_secs": going,
                "page": title,
                "browse": info,
            }));
        }
        out
    }

    /// Look at the browser windows: each one's page leads the list in its turn, and the one in front last (so it
    /// leads). A private window never enters the list.
    fn observe(&self) {
        let wins = crate::scan::visible_windows();
        let mut any = false;
        let push = |exe: &str, title: &str| {
            let cleaned = clean_title(title);
            if !is_browser_exe(exe) || pagekind::is_private_title(title) || is_new_tab_or_empty(None, &cleaned) {
                return;
            }
            let app = exeinfo::lookup(exe).name.unwrap_or_else(|| exe_stem(exe));
            mru_push(&mut self.mru.lock().unwrap(), exe.to_string(), cleaned, app, Instant::now());
        };
        for w in wins.iter().rev().filter(|w| !w.minimized && is_browser_exe(&w.exe)) {
            any = true;
            push(&w.exe, &w.title);
        }
        if let Some((_, Some(path), title)) = foreground_info() {
            if is_browser_exe(&path) {
                any = true;
                self.private.store(pagekind::is_private_title(&title), Ordering::Relaxed);
                push(&path, &title);
            }
        }
        if any {
            *self.seen.lock().unwrap() = Some(Instant::now());
        }
    }
}

// ------------------------------------------------------------------------------------------------------- the thread

fn run(ctx: Ctx, r: Arc<Reader>) {
    std::thread::spawn(move || {
        // background work: never compete with the UI for CPU
        unsafe {
            let _ = windows::Win32::System::Threading::SetThreadPriority(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL,
            );
        }
        // the version of each card's page that was last worked out
        let mut versions: HashMap<String, u64> = HashMap::new();
        let mut last_prune = Instant::now() - Duration::from_secs(PRUNE_EVERY_S);
        let mut last_observe = Instant::now() - Duration::from_secs(10);
        // pages missing from the tab lists, and since when: grace before dropping
        let mut absent: HashMap<String, Instant> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            if !ctx.on() {
                continue;
            }
            // a tab that was sent to another section of its guide has arrived: its card moves on with it
            for (browser, old, new) in r.ext.take_moves() {
                for exe in r.rename(&browser, &old, &new) {
                    // (the key a card has in the island: see `browsingCard` in ui/plugins/page-reader.js)
                    crate::floats::rekey(&ctx.app, &ctx.state, &format!("web:{exe}:{old}"), &format!("web:{exe}:{new}"));
                }
            }
            if last_observe.elapsed() >= Duration::from_secs(2) {
                last_observe = Instant::now();
                r.observe();
            }
            // closed tabs and windows drop out of the MRU list: the extension lists every open tab, so a page
            // no window names anymore is gone (background tabs stay listed and are kept). Missing pages get a
            // grace period first, so mid-load titles do not flicker the card.
            if last_prune.elapsed() >= Duration::from_secs(PRUNE_EVERY_S) {
                last_prune = Instant::now();
                let mut seen: HashSet<String> = HashSet::new();
                let mut seeded: HashSet<String> = HashSet::new();
                let mut tabs: HashMap<String, Vec<String>> = HashMap::new();
                for w in crate::scan::visible_windows().iter().filter(|w| is_browser_exe(&w.exe)) {
                    let k = w.exe.to_lowercase();
                    seen.insert(k.clone());
                    // the tabs that are open but were never in front have cards too
                    if seeded.insert(k.clone()) && !r.private.load(Ordering::Relaxed) {
                        r.seed(&w.exe, &r.ext.open_pages(&w.exe));
                    }
                    if let Some(titles) = r.ext.tab_titles(&w.exe) {
                        let entry = tabs.entry(k).or_default();
                        for c in titles {
                            if !c.is_empty() && !entry.contains(&c) {
                                entry.push(c);
                            }
                        }
                    }
                }
                // an empty scan is a hiccup, not "everything closed": prune nothing
                if !seen.is_empty() {
                    let keys = r.keys();
                    let mut drop: Vec<String> = Vec::new();
                    for (exe, title) in &keys {
                        let k = exe.to_lowercase();
                        let open = if !seen.contains(&k) {
                            false // no window of this browser left
                        } else {
                            match tabs.get(&k) {
                                Some(list) if !list.is_empty() => list.iter().any(|t| *t == title.to_lowercase()),
                                _ => true, // no tab list (no extension): TTL stays the fallback
                            }
                        };
                        let key = browse_key(exe, title);
                        if open {
                            absent.remove(&key);
                        } else if absent.get(&key).map_or(false, |t| t.elapsed() >= Duration::from_secs(PRUNE_GRACE_S)) {
                            drop.push(key.clone());
                            absent.remove(&key);
                        } else {
                            absent.entry(key).or_insert_with(Instant::now);
                        }
                    }
                    // entries re-observed while grace was pending are back: forget them
                    absent.retain(|k, _| keys.iter().any(|(e, t)| &browse_key(e, t) == k));
                    if !drop.is_empty() {
                        r.prune(|exe, title| !drop.contains(&browse_key(exe, title)));
                    }
                }
            }
            if r.keys().is_empty() {
                versions.clear();
                continue;
            }
            // a private window in front: nothing is read meanwhile, and the other cards stay as they are
            if r.private.load(Ordering::Relaxed) {
                continue;
            }
            // the extension reads every tab, but only the pages that have a card (the last few visited) are worked out
            let keys = r.keys();
            versions.retain(|k, _| keys.iter().any(|(e, t)| &browse_key(e, t) == k));
            for (exe, title) in keys {
                // no extension, no reading: the card keeps to the window title
                if !r.ext.covers(&exe) {
                    continue;
                }
                let key = browse_key(&exe, &title);
                let first = !versions.contains_key(&key);
                // a page the extension did not describe (a sign-in page, a blocked site) is not shown at all
                let Some(snap) = r.ext.snapshot(&exe, &title) else {
                    if first {
                        versions.insert(key.clone(), 0);
                        r.remove(&key);
                    }
                    continue;
                };
                if versions.get(&key) == Some(&snap.version) {
                    continue;
                }
                versions.insert(key.clone(), snap.version);
                if pagekind::is_blocked(Some(&snap.url), &title) || is_new_tab_or_empty(Some(&snap.url), &title) {
                    r.remove(&key);
                    continue;
                }
                let ev = Evidence { url: Some(&snap.url), title: &title, page: Some(&snap.page), ext: Some(&snap.data) };
                let info = BrowseInfo {
                    kind: pagekind::read(&ev),
                    domain: domain_of(&snap.url),
                    preview: pagetext::preview_from_ext(&snap.page),
                    image: snap.image.as_ref().map(|i| i.id),
                    blocked: false,
                };
                if std::env::var_os("NADI_DEBUG_PAGES").is_some() {
                    match &info.kind {
                        Some(k) => eprintln!("page[ext] {} {:.2} main={:?} sub={:?} fields={:?}", k.id, k.confidence, k.main, k.sub, k.fields.iter().map(|f| format!("{}={}", f.key, f.value)).collect::<Vec<_>>()),
                        None => eprintln!("page[ext] no kind for {:?}", title),
                    }
                    if let Some(i) = &snap.image {
                        eprintln!("page[ext] image #{} {} bytes", i.id, i.data.len());
                    }
                }
                if !info.is_empty() {
                    r.upsert(key, info);
                }
            }
        }
    });
}

// ------------------------------------------------------------------------------------------------------- the plugin

#[derive(Default)]
pub struct Pages {
    r: Arc<Reader>,
}

const BROWSER_NAME: &[(&str, &str)] = &[("msedge", "Edge"), ("chrome", "Chrome"), ("brave", "Brave"), ("opera", "Opera"), ("vivaldi", "Vivaldi")];

impl Pages {
    /// what the extensions are told: whether to read, and whether to send the pictures
    fn tell(&self, ctx: &Ctx) {
        self.r.ext.set_config(ext::config_json(ctx.on(), ctx.flag("images", true)));
    }

    /// the link to the extension is served while the plugin is on (once started, it stays: it is a socket on this PC)
    fn serve(&self) {
        if !self.r.serving.swap(true, Ordering::Relaxed) {
            ext::spawn(self.r.ext.clone());
        }
    }
}

impl Native for Pages {
    fn manifest(&self) -> Manifest {
        manifest()
    }

    fn start(&self, ctx: &Ctx) {
        self.tell(ctx);
        if ctx.on() {
            self.serve();
        }
        run(ctx.clone(), self.r.clone());
    }

    fn switched(&self, ctx: &Ctx, on: bool) {
        if on {
            self.serve();
        } else {
            self.r.clear();
        }
        self.tell(ctx);
    }

    fn changed(&self, ctx: &Ctx, key: &str) {
        if key == "images" {
            self.tell(ctx);
        }
    }

    fn status(&self, _ctx: &Ctx) -> Option<String> {
        let b = self.r.ext.browsers();
        if b.is_empty() {
            return Some("Waiting for the browser extension (without it only the window title is shown).".into());
        }
        let names: Vec<String> = b.iter().map(|x| BROWSER_NAME.iter().find(|(k, _)| k == x).map_or(x.clone(), |(_, n)| n.to_string())).collect();
        Some(format!("Reading pages in {}.", names.join(", ")))
    }

    fn cards(&self, _ctx: &Ctx) -> Option<Value> {
        Some(Value::Array(self.r.cards()))
    }

    fn call(&self, _ctx: &Ctx, cmd: &str, args: &Value) -> Result<Value, String> {
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let num = |k: &str| args.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        match cmd {
            // a card's button: the extension presses the page's button of that name
            "press" => {
                let Some((exe, _)) = self.r.active() else { return Ok(json!(false)) };
                Ok(json!(self.r.ext.covers(&exe) && self.r.ext.press(&exe, &text("label"))))
            }
            // the tab goes to another section of the guide (within the same site); the card follows the tab
            "go" => {
                let exe = text("exe_path");
                Ok(json!(self.r.ext.covers(&exe) && self.r.ext.navigate(&exe, &text("page"), &text("url"))))
            }
            "guide" => Ok(serde_json::to_value(self.r.ext.guide(&text("key"))).unwrap_or(Value::Null)),
            "guide_image" => Ok(json!(self.r.ext.guide_picture(&text("key"), num("no") as u32, num("max") as u32))),
            "image" => Ok(json!(self.r.ext.image(num("id")))),
            _ => Err(format!("no such call: {cmd}")),
        }
    }

    fn adopt(&self, old: &Value, default_on: bool) -> Adopted {
        adopted(old, default_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_lose_the_profile_name_too() {
        assert_eq!(clean_title("Zeppelin - Wikipedia - Personal - Microsoft\u{200b} Edge"), "Zeppelin - Wikipedia");
        assert_eq!(clean_title("Zeppelin - Wikipedia - Microsoft\u{200b} Edge"), "Zeppelin - Wikipedia");
    }

    #[test]
    fn titles_lose_the_browser_suffix() {
        assert_eq!(clean_title("Rust docs - Google Chrome"), "Rust docs");
        assert_eq!(clean_title("Inbox and 3 more pages - Personal - Microsoft\u{200b} Edge"), "Inbox");
        assert_eq!(clean_title("Plain"), "Plain");
    }

    #[test]
    fn empty_info_has_nothing_to_show() {
        assert!(BrowseInfo::default().is_empty());
        assert!(!BrowseInfo { domain: Some("example.com".into()), ..Default::default() }.is_empty());
        assert!(!BrowseInfo { blocked: true, ..Default::default() }.is_empty());
    }

    #[test]
    fn domains_are_bare_hosts() {
        assert_eq!(domain_of("https://www.Example.com:8080/a?b#c").as_deref(), Some("example.com"));
        assert_eq!(domain_of("chrome://settings"), None);
    }

    #[test]
    fn mru_keeps_the_most_recent_across_browsers() {
        let mut mru = VecDeque::new();
        let now = Instant::now();
        for i in 0..BROWSE_MRU_CAP + 2 {
            let exe = ["chrome.exe", "msedge.exe", "brave.exe"][i % 3];
            mru_push(&mut mru, exe.into(), format!("P{i}"), "App".into(), now);
        }
        let titles: Vec<String> = mru.iter().map(|e| e.title.clone()).collect();
        let want: Vec<String> = (2..BROWSE_MRU_CAP + 2).rev().map(|i| format!("P{i}")).collect();
        assert_eq!(titles, want);
    }

    #[test]
    fn mru_refocus_moves_to_front() {
        let mut mru = VecDeque::new();
        let now = Instant::now();
        mru_push(&mut mru, "chrome.exe".into(), "A".into(), "Chrome".into(), now);
        mru_push(&mut mru, "msedge.exe".into(), "B".into(), "Edge".into(), now);
        mru_push(&mut mru, "chrome.exe".into(), "A".into(), "Chrome".into(), now);
        let titles: Vec<&str> = mru.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["A", "B"]);
        assert_eq!(mru.len(), 2);
    }

    #[test]
    fn a_card_follows_its_tab_to_the_next_page() {
        let r = Reader::default();
        {
            let mut mru = r.mru.lock().unwrap();
            let now = Instant::now();
            mru_push(&mut mru, "C:/b/msedge.exe".into(), "Day 1".into(), "Edge".into(), now);
            mru_push(&mut mru, "C:/b/msedge.exe".into(), "Other".into(), "Edge".into(), now);
        }
        r.info.lock().unwrap().insert(browse_key("C:/b/msedge.exe", "Day 1"), BrowseInfo { blocked: true, ..Default::default() });
        let moved = r.rename("msedge", "Day 1", "Day 2");
        assert_eq!(moved.len(), 1);
        let keys = r.keys();
        assert_eq!(keys.iter().map(|k| k.1.as_str()).collect::<Vec<_>>(), vec!["Other", "Day 2"]);
        assert!(r.info.lock().unwrap().contains_key(&browse_key("C:/b/msedge.exe", "Day 2")));
        // the new page was in the list already: the old card just goes
        r.rename("msedge", "Day 2", "Other");
        assert_eq!(r.keys().len(), 1);
    }

    #[test]
    fn prune_drops_closed_tabs_but_keeps_open_ones() {
        let r = Reader::default();
        {
            let mut mru = r.mru.lock().unwrap();
            let now = Instant::now();
            mru_push(&mut mru, "C:\\x\\chrome.exe".into(), "Page A".into(), "Chrome".into(), now);
            mru_push(&mut mru, "C:\\x\\chrome.exe".into(), "Page B".into(), "Chrome".into(), now);
        }
        *r.seen.lock().unwrap() = Some(Instant::now());
        r.upsert(browse_key("C:\\x\\chrome.exe", "Page A"), BrowseInfo::default());
        r.upsert(browse_key("C:\\x\\chrome.exe", "Page B"), BrowseInfo::default());
        r.prune(|_, title| title == "Page B");
        assert_eq!(r.info.lock().unwrap().len(), 1);
        assert_eq!(r.active().map(|t| t.1).as_deref(), Some("Page B"));
        r.prune(|_, _| false);
        assert_eq!(r.active(), None);
        assert!(r.info.lock().unwrap().is_empty());
    }
}
