//! "What are you browsing": for the browsing card and the work pill. The window title only names the page, so
//! the rest comes from the browser extension (the `nadi-chrome` repo): the page describes itself, exactly, and
//! costs the browser next to nothing (see `ext`). Without the extension connected, a browser is not read at all:
//! the card shows what the window title says and nothing more.
//!
//! Private windows, blocked sites (banking, mail, health, your own list) and pages the extension did not
//! describe (a sign-in page) are never shown.

use crate::IslandState;
use crate::pagekind::{self, Evidence, PageKind};
use crate::pagetext::{self, PagePreview};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// how often open tabs are re-enumerated to drop closed ones from the MRU list
const PRUNE_EVERY_S: u64 = 5;
/// a page missing from every tab strip must miss this many re-checks in a row
/// before it is dropped: mid-load titles flicker, closed tabs do not come back
const PRUNE_GRACE_S: u64 = 10;

/// same browser list as `work::BROWSER_EXES`, by exe file name
fn is_browser_exe(exe: &str) -> bool {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_lowercase();
    let base = file.strip_suffix(".exe").unwrap_or(&file);
    matches!(base, "chrome" | "msedge" | "firefox" | "brave" | "opera" | "vivaldi" | "arc")
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

/// Newest `History` file among a Chromium browser's profiles (the downloads card reads it).
pub(crate) fn history_path(exe: &str) -> Option<PathBuf> {
    let file = exe.rsplit(['\\', '/']).next()?.to_lowercase();
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
    let data_dir = match file.strip_suffix(".exe").unwrap_or(&file) {
        "chrome" => local.join("Google/Chrome/User Data"),
        "msedge" => local.join("Microsoft/Edge/User Data"),
        "brave" => local.join("BraveSoftware/Brave-Browser/User Data"),
        "vivaldi" => local.join("Vivaldi/User Data"),
        "opera" => roaming?.join("Opera Software/Opera Stable"),
        _ => return None,
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let mut consider = |p: PathBuf| {
        if let Ok(t) = std::fs::metadata(&p).and_then(|m| m.modified()) {
            if best.as_ref().map_or(true, |(bt, _)| t > *bt) {
                best = Some((t, p));
            }
        }
    };
    consider(data_dir.join("History")); // Opera keeps it at the top
    if let Ok(rd) = std::fs::read_dir(&data_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == "Default" || name.starts_with("Profile ") {
                consider(e.path().join("History"));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Card button -> the browser extension presses the page's button of that name.
#[tauri::command]
pub async fn click_page_button(window: tauri::WebviewWindow, label: String) -> bool {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some((exe, _)) = state.activity.active_browser() else { return false };
        state.ext.covers(&exe) && state.ext.press(&exe, &label)
    })
    .await
    .unwrap_or(false)
}

/// Card button -> the tab goes to another section of the guide (the browser extension takes it there, within the
/// same site). The card is found by its window and its title. The page is read again once it has loaded, and the
/// card follows the tab: what it shows is always what the page has.
#[tauri::command]
pub async fn guide_go(window: tauri::WebviewWindow, exe_path: String, page: String, url: String) -> bool {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.ext.covers(&exe_path) && state.ext.navigate(&exe_path, &page, &url))
        .await
        .unwrap_or(false)
}

pub fn spawn(app: tauri::AppHandle, state: Arc<IslandState>) {
    std::thread::spawn(move || {
        // background work: never compete with the UI for CPU
        unsafe {
            let _ = windows::Win32::System::Threading::SetThreadPriority(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL,
            );
        }
        // the version of each card's page that was last worked out
        let mut versions: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        let mut last_prune = std::time::Instant::now() - Duration::from_secs(PRUNE_EVERY_S);
        // pages missing from the tab lists, and since when: grace before dropping
        let mut absent: std::collections::HashMap<String, std::time::Instant> = std::collections::HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            // a tab that was sent to another section of its guide has arrived: its card moves on with it
            for (browser, old, new) in state.ext.take_moves() {
                for exe in state.activity.rename_browse(&browser, &old, &new) {
                    // (the key a card has in the island: see `browsingCard` in main.js)
                    crate::floats::rekey(&app, &state, &format!("web:{exe}:{old}"), &format!("web:{exe}:{new}"));
                }
            }
            // closed tabs and windows drop out of the MRU list: the extension lists every open tab, so a page
            // no window names anymore is gone (background tabs stay listed and are kept). Missing pages get a
            // grace period first, so mid-load titles do not flicker the card.
            if last_prune.elapsed() >= Duration::from_secs(PRUNE_EVERY_S) {
                last_prune = std::time::Instant::now();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut seeded: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut tabs: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
                for w in crate::scan::visible_windows().iter().filter(|w| is_browser_exe(&w.exe)) {
                    let k = w.exe.to_lowercase();
                    seen.insert(k.clone());
                    // the tabs that are open but were never in front have cards too (when reading is on)
                    if seeded.insert(k.clone()) && state.settings.lock().unwrap().page_preview && !state.activity.browse_private() {
                        state.activity.seed_browse(&w.exe, &state.ext.open_pages(&w.exe));
                    }
                    if let Some(titles) = state.ext.tab_titles(&w.exe) {
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
                    let keys = state.activity.browse_keys();
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
                        let key = crate::activity::browse_key(&exe, &title);
                        if open {
                            absent.remove(&key);
                        } else if absent.get(&key).map_or(false, |t| t.elapsed() >= Duration::from_secs(PRUNE_GRACE_S)) {
                            drop.push(key.clone());
                            absent.remove(&key);
                        } else {
                            absent.entry(key).or_insert_with(std::time::Instant::now);
                        }
                    }
                    // entries re-observed while grace was pending are back: forget them
                    absent.retain(|k, _| keys.iter().any(|(e, t)| &crate::activity::browse_key(e, t) == k));
                    if !drop.is_empty() {
                        state.activity.prune_browse(|exe, title| !drop.contains(&crate::activity::browse_key(exe, title)));
                    }
                }
            }
            if state.activity.browse_keys().is_empty() {
                versions.clear();
                continue;
            }
            // a private window in front: nothing is read meanwhile, and the other cards stay as they are
            if state.activity.browse_private() {
                continue;
            }
            let reading = state.settings.lock().unwrap().page_preview;
            if !reading {
                continue;
            }
            // the extension reads every tab, but only the pages that have a card (the last few visited) are worked out
            let keys = state.activity.browse_keys();
            versions.retain(|k, _| keys.iter().any(|(e, t)| &crate::activity::browse_key(e, t) == k));
            for (exe, title) in keys {
                // no extension, no reading: the card keeps to the window title
                if !state.ext.covers(&exe) {
                    continue;
                }
                let key = crate::activity::browse_key(&exe, &title);
                let first = !versions.contains_key(&key);
                // a page the extension did not describe (a sign-in page, a blocked site) is not shown at all
                let Some(snap) = state.ext.snapshot(&exe, &title) else {
                    if first {
                        versions.insert(key.clone(), 0);
                        state.activity.remove_browsing(&key);
                    }
                    continue;
                };
                if versions.get(&key) == Some(&snap.version) {
                    continue;
                }
                versions.insert(key.clone(), snap.version);
                if pagekind::is_blocked(Some(&snap.url), &title) || is_new_tab_or_empty(Some(&snap.url), &title) {
                    state.activity.remove_browsing(&key);
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
                    state.activity.upsert_browsing(key, info);
                }
            }
        }
    });
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
}
