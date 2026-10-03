//! "What are you browsing": for the browsing card and the work pill. The window title only names the page,
//! so the rest is read in tiers, cheapest first:
//!
//! 1. the address and the title (free): what kind of page it probably is,
//! 2. the page's structure through UI Automation, while the browser is in front or the hub is open,
//!    at most every few seconds (see `uia`),
//! 3. OCR of the window, only while the hub is open and there was no tree to read (see `pagetext`),
//! 4. the browser's own history database (Chromium family), only while the hub is open: the full address and
//!    the page you came from. The live file is locked by the browser, so a copy is read, and only when the
//!    file changed since the last copy.
//!
//! Private windows and blocked sites (banking, mail, health, your own list) are never read at all.

use crate::IslandState;
use crate::pagekind::{self, Evidence, PageKind, Trail};
use crate::pagetext::{self, PagePreview};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// the page has to stay this long before its structure is read (it may still be loading, or you may be passing by)
const SETTLE_S: f64 = 2.5;
/// at most one full read of a page's structure per this long (a long page costs the browser about half a second)
const FULL_READ_COOLDOWN_S: u64 = 10;
/// a read this old is redone when the hub opens
const STALE_S: u64 = 60;
/// how often the scroll position of the page in front is looked at (one cheap call)
const SCROLL_EVERY_S: u64 = 2;
/// how often the history is looked at again while the hub is open
const REFRESH_S: u64 = 12;
/// gap between the Windows FILETIME-style epoch Chromium uses (1601) and Unix
const CHROME_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;
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
    /// site of the page in front of you, when it could be matched in the history
    pub domain: Option<String>,
    /// what the page says, read from the window
    pub preview: Option<PagePreview>,
    /// what kind of page it is, and what is worth saying about it
    pub kind: Option<PageKind>,
    /// a private window or a blocked site: nothing is read, and the card shows no page
    pub blocked: bool,
}

impl BrowseInfo {
    /// nothing was read (yet): no card worth showing
    pub fn is_empty(&self) -> bool {
        !self.blocked && self.domain.is_none() && self.preview.is_none() && self.kind.is_none()
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

/// Newest `History` file among a Chromium browser's profiles.
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

fn midnight_chrome_us() -> i64 {
    use chrono::{Local, TimeZone};
    let midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
    let unix = Local.from_local_datetime(&midnight).earliest().map_or(0, |t| t.timestamp());
    unix * 1_000_000 + CHROME_EPOCH_OFFSET_US
}

/// Copy the locked database (and its WAL, if any) so it can be opened.
fn snapshot(src: &Path) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("nadi-history");
    std::fs::create_dir_all(&dir).ok()?;
    let dst = dir.join("History");
    std::fs::copy(src, &dst).ok()?;
    let wal = src.with_file_name("History-wal");
    let dst_wal = dir.join("History-wal");
    if wal.exists() {
        let _ = std::fs::copy(&wal, &dst_wal);
    } else {
        let _ = std::fs::remove_file(&dst_wal);
    }
    Some(dst)
}

/// The newest visit today whose title matches the page in front of you: its address, and the trail
/// (the page you came from, and whether this address was visited before).
fn query(db: &Path, page_title: &str) -> Option<(String, Trail)> {
    if page_title.is_empty() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let (url, url_id, at): (String, i64, i64) = conn
        .prepare(
            "SELECT u.url, u.id, v.visit_time FROM visits v JOIN urls u ON u.id = v.url \
             WHERE v.visit_time >= ?1 AND u.title = ?2 AND (v.transition & 255) NOT IN (3, 4) \
             ORDER BY v.visit_time DESC LIMIT 1",
        )
        .ok()?
        .query_row(rusqlite::params![midnight_chrome_us(), page_title], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .ok()?;
    let mut trail = Trail::default();
    // the page before this one, within half an hour, on the same site
    let thirty_min_us = 30 * 60 * 1_000_000_i64;
    if let Ok(mut st) = conn.prepare(
        "SELECT u.url, u.title FROM visits v JOIN urls u ON u.id = v.url \
         WHERE v.visit_time < ?1 AND v.visit_time >= ?2 AND u.id != ?3 AND (v.transition & 255) NOT IN (3, 4) \
         ORDER BY v.visit_time DESC LIMIT 1",
    ) {
        if let Ok((pu, pt)) = st.query_row(rusqlite::params![at, at - thirty_min_us, url_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            if domain_of(&pu) == domain_of(&url) {
                trail.prev_url = Some(pu);
                trail.prev_title = Some(pt).filter(|t| !t.is_empty());
            }
        }
    }
    trail.seen_before = conn
        .query_row("SELECT COUNT(*) FROM visits WHERE url = ?1 AND visit_time < ?2", rusqlite::params![url_id, at], |r| r.get::<_, i64>(0))
        .map_or(false, |n| n > 0);
    Some((url, trail))
}

/// Card button -> click the page's button of that name (see `pagetext::click_button`).
#[tauri::command]
pub async fn click_page_button(window: tauri::WebviewWindow, label: String, x: f32, y: f32) -> bool {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some((exe, title)) = state.activity.active_browser() else { return false };
        let target = crate::scan::visible_windows().into_iter().find(|w| {
            !w.minimized && w.exe.eq_ignore_ascii_case(&exe) && (title.is_empty() || w.title.contains(&title))
        });
        target.map_or(false, |w| pagetext::click_button(w.hwnd, &label, x, y))
    })
    .await
    .unwrap_or(false)
}

fn foreground() -> isize {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as isize }
}

pub fn spawn(state: Arc<IslandState>) {
    std::thread::spawn(move || {
        // background work: never compete with the UI for CPU
        unsafe {
            let _ = windows::Win32::System::Threading::SetThreadPriority(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL,
            );
        }
        let reader = crate::uia::Reader::new();
        let mut last_key = String::new();
        let mut last_full = std::time::Instant::now() - Duration::from_secs(FULL_READ_COOLDOWN_S);
        let mut full_at: Option<std::time::Instant> = None;
        let mut changed_at = std::time::Instant::now();
        let mut last_scroll = std::time::Instant::now();
        let mut last_scroll_poll = std::time::Instant::now();
        let mut last_address = std::time::Instant::now() - Duration::from_secs(10);
        let mut hub_before = false;
        let mut last_hist = std::time::Instant::now() - Duration::from_secs(REFRESH_S);
        let mut last_mtime: Option<SystemTime> = None;
        let mut db: Option<PathBuf> = None;
        let mut last_fp: Option<u64> = None;
        let mut preview: Option<PagePreview> = None;
        let mut url: Option<String> = None;
        let mut trail = Trail::default();
        let mut page: Option<crate::uia::Page> = None;
        let mut ocr: Vec<String> = Vec::new();
        let mut info = BrowseInfo::default();
        let mut last_prune = std::time::Instant::now() - Duration::from_secs(PRUNE_EVERY_S);
        // pages missing from the tab strips, and since when: grace before dropping
        let mut absent: std::collections::HashMap<String, std::time::Instant> = std::collections::HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            let hub = state.hub_open.load(Ordering::Relaxed);
            // closed tabs and windows drop out of the MRU list: the tab strip
            // names every open tab, so a page no window names anymore is gone
            // (background tabs stay named and are kept). Missing pages get a
            // grace period first, so mid-load titles do not flicker the card.
            if last_prune.elapsed() >= Duration::from_secs(PRUNE_EVERY_S) {
                last_prune = std::time::Instant::now();
                if let Some(r) = &reader {
                    r.attach();
                    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                    let mut tabs: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
                    for w in crate::scan::visible_windows().iter().filter(|w| is_browser_exe(&w.exe)) {
                        let k = w.exe.to_lowercase();
                        seen.insert(k.clone());
                        // a failed read is no data, not zero tabs: only replace on success
                        if let Some(names) = r.open_tabs(w.hwnd) {
                            let entry = tabs.entry(k).or_default();
                            for n in names {
                                let c = clean_title(&n).to_lowercase();
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
                                    _ => true, // no tab data (e.g. Firefox): TTL stays the fallback
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
            }
            let Some((exe, title)) = state.activity.active_browser() else {
                // nothing recent: MRU cards expire on their own, nothing to clear
                last_key.clear();
                if let Some(r) = &reader {
                    r.detach();
                }
                continue;
            };
            // a private window: nothing about it is read, kept or shown; other cards stay
            if state.activity.browse_private() {
                last_key.clear();
                continue;
            }
            let key = crate::activity::browse_key(&exe, &title);
            // new tab or empty page: forget this page only, other MRU cards stay
            if is_new_tab_or_empty(url.as_deref(), &title) {
                state.activity.remove_browsing(&key);
                last_key.clear();
                continue;
            }
            let page_changed = key != last_key;
            let target = crate::scan::visible_windows().into_iter().find(|w| {
                !w.minimized && w.exe.eq_ignore_ascii_case(&exe) && (title.is_empty() || w.title.contains(&title))
            });
            let in_front = target.as_ref().map_or(false, |w| w.hwnd.0 as isize == foreground());
            // nothing to do when nobody is looking: the hub is closed and the browser is not in front
            if !hub && !in_front && !page_changed {
                // not needed for a while: let the browser drop its accessibility tree again
                if last_scroll.elapsed() > Duration::from_secs(20) {
                    if let Some(r) = &reader {
                        r.detach();
                    }
                }
                continue;
            }
            let (settings_reading, blocklist) = {
                let s = state.settings.lock().unwrap();
                (s.page_preview, s.page_blocklist.clone())
            };
            if page_changed {
                last_key = key.clone();
                last_fp = None;
                preview = None;
                url = None;
                trail = Trail::default();
                page = None;
                ocr.clear();
                info = BrowseInfo::default();
                changed_at = std::time::Instant::now();
                full_at = None;
                last_address = std::time::Instant::now() - Duration::from_secs(10);
                last_hist = std::time::Instant::now() - Duration::from_secs(REFRESH_S);
            }

            // history: the full address and the trail (only with the hub open: it copies a file)
            let mut domain = info.domain.clone();
            if hub && last_hist.elapsed() >= Duration::from_secs(REFRESH_S) {
                last_hist = std::time::Instant::now();
                if let Some(src) = history_path(&exe) {
                    let mtime = std::fs::metadata(&src).and_then(|m| m.modified()).ok();
                    // the copy is the expensive part: redo it only when the file changed
                    if mtime != last_mtime || db.is_none() {
                        db = snapshot(&src);
                        last_mtime = mtime;
                    }
                    if let Some((u, t)) = db.as_deref().and_then(|d| query(d, &title)) {
                        domain = domain_of(&u);
                        url.get_or_insert(u);
                        trail = t;
                    }
                }
            }

            // reading the page: the structure through UI Automation (never while a game runs). The address comes
            // first and cheap, because the block list decides before anything of the page is read.
            let reading = settings_reading && !state.has_game.load(Ordering::Relaxed) && (hub || in_front);
            let mut fresh = page_changed;
            if let (true, Some(w), Some(r)) = (reading, &target, &reader) {
                r.attach();
                last_scroll = std::time::Instant::now();
                if last_address.elapsed() >= Duration::from_secs(10) {
                    last_address = std::time::Instant::now();
                    if let Some(u) = r.url(w.hwnd) {
                        url = Some(u);
                    }
                }
            }
            if domain.is_none() {
                domain = url.as_deref().and_then(domain_of);
            }
            // new tab or empty page: forget this page only, other MRU cards stay
            if is_new_tab_or_empty(url.as_deref(), &title) {
                state.activity.remove_browsing(&key);
                last_key.clear();
                continue;
            }
            // blocked sites are never read: this page is forgotten, the rest stays
            if pagekind::is_blocked(url.as_deref(), &title, &blocklist) {
                preview = None;
                page = None;
                ocr.clear();
                state.activity.remove_browsing(&key);
                hub_before = hub;
                continue;
            }
            if let (true, Some(w), Some(r)) = (reading, &target, &reader) {
                // the whole structure, once per page: when it has settled, and again when the hub opens on an old reading
                let settled = changed_at.elapsed().as_secs_f64() >= SETTLE_S;
                let first = page.is_none() && settled;
                let stale = hub && !hub_before && full_at.map_or(false, |t| t.elapsed() >= Duration::from_secs(STALE_S));
                if (first || stale) && last_full.elapsed() >= Duration::from_secs(FULL_READ_COOLDOWN_S) {
                    last_full = std::time::Instant::now();
                    if let Some(p) = r.read(w.hwnd) {
                        if p.url.is_some() {
                            url = p.url.clone();
                        }
                        if is_new_tab_or_empty(url.as_deref(), &title) {
                            state.activity.remove_browsing(&key);
                            last_key.clear();
                            continue;
                        }
                        preview = pagetext::preview_from_page(&p, w.hwnd).or(preview);
                        page = Some(p);
                        full_at = Some(std::time::Instant::now());
                        fresh = true;
                    } else {
                        // the browser is only now switching its tree on: look again in a moment
                        last_full = std::time::Instant::now() - Duration::from_secs(FULL_READ_COOLDOWN_S - 2);
                    }
                } else if let Some(p) = page.as_mut() {
                    // how far down you are: one cheap call, every couple of seconds
                    if full_at.map_or(false, |t| t.elapsed() > Duration::from_millis(500)) && last_scroll_poll.elapsed() >= Duration::from_secs(SCROLL_EVERY_S) {
                        last_scroll_poll = std::time::Instant::now();
                        if let Some(now) = r.scroll(w.hwnd) {
                            if p.scroll_now.map_or(true, |old| (old - now).abs() >= 0.005) {
                                p.scroll_now = Some(now);
                                fresh = true;
                            }
                        }
                    }
                }
            }
            hub_before = hub;

            // OCR, only with the hub open and when there was no tree to read
            if hub && settings_reading && page.as_ref().map_or(true, |p| p.nodes.len() < 8) {
                if let Some(w) = &target {
                    if let Some((p, lines, fp)) = pagetext::read_window_lines(w.hwnd, &title, last_fp) {
                        last_fp = Some(fp);
                        if p.is_some() || page_changed {
                            preview = p.or(preview);
                        }
                        if !lines.is_empty() {
                            ocr = lines;
                            fresh = true;
                        }
                    }
                }
            }

            if fresh {
                let ev = Evidence { url: url.as_deref(), title: &title, page: page.as_ref(), ocr: &ocr, trail: &trail };
                info.kind = pagekind::read(&ev);
            }
            info.domain = domain;
            info.preview = preview.clone();
            info.blocked = false;
            // publish only once there is something to show: the first loops
            // after a switch still read an empty page, and that blank must not
            // hide (then unhide) the card
            if !info.is_empty() {
                state.activity.upsert_browsing(key.clone(), info.clone());
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
