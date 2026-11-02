//! A newer NADI: looked for now and then (its repository's latest release, on GitHub), and, when the person says so, fetched and
//! installed without a browser (its installer run quietly: it closes this NADI, puts the new one in its place, and starts it).

use serde_json::{json, Value};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// something changed (a newer one found, its download done or failed): the frame tells the person
pub const WM_UPDATE: u32 = WM_APP + 15;

/// how often it is looked for
const EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// none newer (or not looked for yet)
    #[default]
    Current,
    Looking,
    /// a newer one: its version, where its installer is
    Ready(String, String),
    /// its installer coming down (its version)
    Fetching(String),
    /// its installer started: this NADI is about to close
    Installing(String),
    Failed(String),
}

static NOW: Mutex<Status> = Mutex::new(Status::Current);
/// the newer one last found (its version, where its installer is): kept, so a download that failed can be tried again
static FOUND: Mutex<Option<(String, String)>> = Mutex::new(None);
static NOTIFY: Mutex<isize> = Mutex::new(0);
/// how far its installer has come (0..1, in thousandths), and whether to stop fetching it
static FRACTION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How far the installer being fetched has come (0..1).
pub fn fraction() -> f32 {
    FRACTION.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0
}

/// The installer being fetched: given up.
pub fn stop() {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn post() {
    let h = *NOTIFY.lock().unwrap();
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), WM_UPDATE, WPARAM(0), LPARAM(0));
        }
    }
}

fn set(s: Status) {
    *NOW.lock().unwrap() = s;
    post();
}

pub fn status() -> Status {
    NOW.lock().unwrap().clone()
}

/// "1.10.2" as numbers, to be compared.
pub fn parts(v: &str) -> Vec<u32> {
    v.trim_start_matches('v').split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect()
}

/// Is `theirs` newer than this NADI?
pub fn newer(theirs: &str) -> bool {
    parts(theirs) > parts(env!("CARGO_PKG_VERSION"))
}

/// Look for a newer one (on a thread of its own).
pub fn check() {
    if matches!(status(), Status::Looking | Status::Fetching(_) | Status::Installing(_)) {
        return;
    }
    set(Status::Looking);
    std::thread::spawn(|| {
        let found = nadi_core::repo::latest_nadi();
        // (every version's changes looked at too: a newer one's shown before it is installed)
        crate::changes::refresh();
        match found {
            Ok(Some((v, url))) if newer(&v) => {
                *FOUND.lock().unwrap() = Some((v.clone(), url.clone()));
                set(Status::Ready(v, url));
            }
            Ok(_) => set(Status::Current),
            Err(e) => set(Status::Failed(e)),
        }
    });
}

/// Looked for a while after NADI starts, then every few hours.
pub fn start(notify: HWND) {
    *NOTIFY.lock().unwrap() = notify.0 as isize;
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(20));
        loop {
            check();
            std::thread::sleep(EVERY);
        }
    });
}

/// The newer one fetched and installed (its installer, run quietly, closes this NADI and starts the new one).
pub fn install() {
    if matches!(status(), Status::Fetching(_) | Status::Installing(_)) {
        return;
    }
    let Some((v, url)) = FOUND.lock().unwrap().clone() else { return };
    run_installer(v, url);
}

/// Another version installed, an older one too (going back): its installer fetched and run, as an update's is.
pub fn install_version(v: &str) {
    if matches!(status(), Status::Fetching(_) | Status::Installing(_)) || v == env!("CARGO_PKG_VERSION") {
        return;
    }
    let Some(tag) = crate::changes::tag_of(v) else { return };
    run_installer(v.to_string(), nadi_core::repo::installer_url(&tag));
}

fn run_installer(v: String, url: String) {
    FRACTION.store(0, std::sync::atomic::Ordering::Relaxed);
    STOP.store(false, std::sync::atomic::Ordering::Relaxed);
    set(Status::Fetching(v.clone()));
    std::thread::spawn(move || {
        let r = (|| -> Result<(), String> {
            let bytes = nadi_core::repo::fetch_installer_with(&url, &|f| FRACTION.store((f * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed), &STOP)?;
            // (an installer is a program: what came is checked to be one before it is run)
            if bytes.len() < 1024 || &bytes[..2] != b"MZ" {
                return Err("What came was not an installer.".into());
            }
            let path = nadi_core::paths::temp_dir().join(format!("NADI-Setup-{v}.exe"));
            std::fs::write(&path, &bytes).map_err(|e| format!("Could not keep the installer ({e})."))?;
            std::process::Command::new(&path).arg("--quiet").spawn().map_err(|e| format!("Could not start the installer ({e})."))?;
            Ok(())
        })();
        match r {
            Ok(()) => set(Status::Installing(v)),
            // (given up: as it was, a newer one still found)
            Err(_) if STOP.load(std::sync::atomic::Ordering::Relaxed) => set(match FOUND.lock().unwrap().clone() {
                Some((v, url)) if newer(&v) => Status::Ready(v, url),
                _ => Status::Current,
            }),
            // (it can be tried again: what was found is kept)
            Err(e) => set(Status::Failed(e)),
        }
    });
}

/// For the settings: where it stands.
pub fn json() -> Value {
    match status() {
        Status::Current => json!({ "state": "current" }),
        Status::Looking => json!({ "state": "looking" }),
        Status::Ready(v, _) => json!({ "state": "ready", "version": v }),
        Status::Fetching(v) => json!({ "state": "fetching", "version": v }),
        Status::Installing(v) => json!({ "state": "installing", "version": v }),
        Status::Failed(e) => json!({ "state": "failed", "error": e, "version": FOUND.lock().unwrap().as_ref().map(|(v, _)| v.clone()) }),
    }
}
