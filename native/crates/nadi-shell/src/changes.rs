//! Every NADI version's changes, kept on disk (`nadi-changes.json`): its release's notes, or the tidied titles of the commits since
//! the version before it (see `nadi_core::changelog`). Looked at again with each look for a newer NADI; only what is missing is asked
//! of GitHub. Shown in the settings (every version), and once in a modal after an update (the versions since the last one run).

use nadi_core::changelog::{self, Line};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// the changes since the last version run are ready to be shown (the frame asks for them)
pub const WM_WHATS_NEW: u32 = WM_APP + 16;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    pub tag: String,
    pub date: String,
    pub lines: Vec<Line>,
    /// where its lines came from: "notes", "commits", or "" (not worked out yet: tried again next time)
    #[serde(default)]
    pub from: String,
}

/// newest first
static KNOWN: Mutex<Option<Vec<Version>>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
/// the version run before this one, its changes waiting to be shown
static SINCE: Mutex<Option<String>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    nadi_core::paths::data_dir().unwrap_or_default().join("nadi-changes.json")
}

/// Every version known, newest first.
pub fn known() -> Vec<Version> {
    let mut k = KNOWN.lock().unwrap();
    if k.is_none() {
        *k = Some(std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    }
    k.clone().unwrap_or_default()
}

/// Ask GitHub for what is missing: the list of releases (their notes come with it), and the commits of each version with no notes
/// whose lines are not worked out yet. Blocking: on a thread of its own.
pub fn refresh() {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(mut rels) = nadi_core::repo::nadi_releases() {
        rels.sort_by_key(|r| crate::update::parts(&r.version));
        let old = known();
        let mut out: Vec<Version> = Vec::new();
        for (i, r) in rels.iter().enumerate() {
            let notes = changelog::from_notes(&r.body);
            let kept = old.iter().find(|o| o.version == r.version && o.from == "commits" && !o.lines.is_empty());
            let (lines, from) = if !notes.is_empty() {
                (notes, "notes")
            } else if let Some(k) = kept {
                (k.lines.clone(), "commits")
            } else if i == 0 {
                (Vec::new(), "commits")
            } else {
                match nadi_core::repo::nadi_commits(&rels[i - 1].tag, &r.tag) {
                    Ok(m) => (changelog::from_commits(&m), "commits"),
                    Err(_) => (Vec::new(), ""),
                }
            };
            out.push(Version { version: r.version.clone(), tag: r.tag.clone(), date: r.date.clone(), lines, from: from.into() });
        }
        out.reverse();
        let _ = nadi_core::paths::write_atomic(&path(), serde_json::to_string_pretty(&out).unwrap_or_default().as_bytes());
        *KNOWN.lock().unwrap() = Some(out);
    }
    BUSY.store(false, Ordering::SeqCst);
}

/// A version's tag, when it is known.
pub fn tag_of(version: &str) -> Option<String> {
    known().into_iter().find(|v| v.version == version).map(|v| v.tag)
}

/// For the settings: every version, newest first.
pub fn json() -> Value {
    json!(known().iter().map(|v| json!({ "version": v.version, "date": v.date, "lines": v.lines })).collect::<Vec<_>>())
}

/// The versions after `since` up to and including `now`, newest first, those with lines only (`since` empty: `now` alone).
pub fn between(since: &str, now: &str) -> Vec<Version> {
    let (lo, hi) = (crate::update::parts(since), crate::update::parts(now));
    known()
        .into_iter()
        .filter(|v| {
            let p = crate::update::parts(&v.version);
            !v.lines.is_empty() && p <= hi && if since.is_empty() { p == hi } else { p > lo }
        })
        .collect()
}

/// NADI was updated (from `since`, empty when not known): its changes looked for (waiting a moment for the island to settle, and for
/// GitHub when they are not kept yet), then the frame told to show them.
pub fn after_update(notify: HWND, since: String) {
    *SINCE.lock().unwrap() = Some(since);
    let h = notify.0 as isize;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let now = env!("CARGO_PKG_VERSION");
        if !known().iter().any(|v| v.version == now && !v.from.is_empty()) {
            // (the look for a newer NADI may be asking already: waited for, up to 10 s)
            if BUSY.load(Ordering::SeqCst) {
                for _ in 0..40 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if !BUSY.load(Ordering::SeqCst) {
                        break;
                    }
                }
            } else {
                refresh();
            }
        }
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), WM_WHATS_NEW, WPARAM(0), LPARAM(0));
        }
    });
}

/// The modal of what is new since the last version run, once (none: nothing to show).
pub fn take_modal() -> Option<Value> {
    let since = SINCE.lock().unwrap().take()?;
    let now = env!("CARGO_PKG_VERSION");
    let versions = between(&since, now);
    if versions.is_empty() {
        return None;
    }
    let mut list = Vec::new();
    for (i, v) in versions.iter().enumerate() {
        if i > 0 {
            list.push(json!({ "sep": v.version }));
        }
        list.extend(v.lines.iter().map(|l| json!({ "kind": l.kind, "text": l.text })));
    }
    let mut spec = json!({ "icon": "up", "tone": "theme", "title": now, "list": list, "buttons": [{ "label": "OK", "tone": "theme" }] });
    if !since.is_empty() {
        spec["text"] = json!(format!("from {since}"));
    }
    Some(spec)
}
