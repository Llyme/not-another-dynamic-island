//! Clean Up (Settings › About): what NADI left behind that it no longer needs. Installers it downloaded to update itself, the copies
//! the installer and the Windows-key helper ran from, anything else of NADI's in the temporary folder, old copies of nadi.exe set
//! aside while it was replaced, the plugin downloads kept from repositories (fetched again when needed), and its logs. Never the
//! settings, the plugins, or what plugins keep (a speech model is theirs).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// how much there is to clean up, as last looked (bytes); none: not looked yet
static SIZE: Mutex<Option<u64>> = Mutex::new(None);
static BUSY: Mutex<bool> = Mutex::new(false);

fn size_of(p: &Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| size_of(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// What can go.
fn found() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // (the temporary folder: NADI's own)
    for e in std::fs::read_dir(std::env::temp_dir()).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if n.starts_with("nadi-") || n.starts_with("nadi_") {
            out.push(e.path());
        }
    }
    // (old copies of nadi.exe, set aside as it was replaced)
    if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Programs").join("NADI")) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n.starts_with("nadi.old-") || n.starts_with("nadi.running-") {
                out.push(e.path());
            }
        }
    }
    if let Some(data) = nadi_core::paths::data_dir() {
        // (the plugins downloaded from repositories: fetched again when needed)
        out.push(data.join("repo-cache"));
        // (its logs)
        for e in std::fs::read_dir(&data).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "log") {
                out.push(e.path());
            }
        }
    }
    out.retain(|p| p.exists());
    out
}

fn tell(notify: HWND) {
    unsafe {
        let _ = PostMessageW(notify, crate::update::WM_UPDATE, WPARAM(0), LPARAM(0));
    }
}

/// How much there is to clean up (looked for on a thread of its own; `notify` told when known).
pub fn look(notify: HWND) {
    let h = notify.0 as isize;
    std::thread::spawn(move || {
        let n = found().iter().map(|p| size_of(p)).sum();
        *SIZE.lock().unwrap() = Some(n);
        tell(HWND(h as *mut _));
    });
}

/// Cleaned up (what is in use stays), then looked at again.
pub fn clean(notify: HWND) {
    if std::mem::replace(&mut *BUSY.lock().unwrap(), true) {
        return;
    }
    let h = notify.0 as isize;
    std::thread::spawn(move || {
        for p in found() {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
        *BUSY.lock().unwrap() = false;
        let n = found().iter().map(|p| size_of(p)).sum();
        *SIZE.lock().unwrap() = Some(n);
        tell(HWND(h as *mut _));
    });
}

/// For the settings: how much (bytes), whether it is being cleaned.
pub fn json() -> serde_json::Value {
    serde_json::json!({ "bytes": *SIZE.lock().unwrap(), "busy": *BUSY.lock().unwrap() })
}
