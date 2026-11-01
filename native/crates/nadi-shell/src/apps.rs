//! The apps on this PC, as the Start menu knows them (the Apps folder: desktop programs and store apps alike), and launching one.

use windows::core::{HSTRING, PWSTR};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    /// what the Apps folder calls it: a store app's id, or a program's path
    pub id: String,
}

fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<App>)>> = std::sync::Mutex::new(None);
static LOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The apps, as last found (finding them takes most of a second: it is done on a thread of its own, at the start and now and then).
pub fn cached() -> Vec<App> {
    let (fresh, list) = match &*CACHE.lock().unwrap() {
        Some((at, list)) => (at.elapsed().as_secs() < 120, list.clone()),
        None => (false, Vec::new()),
    };
    if !fresh {
        refresh_in_background();
    }
    list
}

pub fn refresh_in_background() {
    if LOADING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let list = all();
        *CACHE.lock().unwrap() = Some((std::time::Instant::now(), list));
        LOADING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// Every app, by name.
pub fn all() -> Vec<App> {
    let mut out = Vec::new();
    unsafe {
        let Ok(folder) = SHGetKnownFolderItem::<_, IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(list) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut one: [Option<IShellItem>; 1] = [None];
            let mut got = 0u32;
            if list.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(item) = one[0].take() else { break };
            let name = item.GetDisplayName(SIGDN_NORMALDISPLAY).map(take).unwrap_or_default();
            let id = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(take).unwrap_or_default();
            if !name.is_empty() && !id.is_empty() {
                out.push(App { name, id });
            }
        }
    }
    // (the Steam games installed: Steam puts them in the Start menu only when asked to, so many are not there)
    let have: std::collections::HashSet<String> = out.iter().map(|a| a.name.to_lowercase()).collect();
    for g in nadi_core::sensors::steamlib::state()["games"].as_array().cloned().unwrap_or_default() {
        let (Some(id), Some(name)) = (g["id"].as_u64(), g["name"].as_str()) else { continue };
        if !have.contains(&name.to_lowercase()) {
            out.push(App { name: name.to_string(), id: format!("{STEAM}{id}") });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// how a Steam game is named among the apps (run through Steam)
pub const STEAM: &str = "steam://rungameid/";

/// The shell item of an app (for its picture).
pub fn item(id: &str) -> Option<IShellItem> {
    unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(format!("shell:AppsFolder\\{id}")), None).ok() }
}

/// Launch an app by its id; a path (a program, a shortcut) is opened as it is.
pub fn launch(id: &str) {
    if id.starts_with(STEAM) {
        return launch_uri(id);
    }
    let target = if id.contains(":\\") && !id.contains('!') { id.to_string() } else { format!("shell:AppsFolder\\{id}") };
    // (a program started in its own folder, as a shortcut to it would: a game that looks for its files beside it finds them)
    let dir = program_dir(&target);
    unsafe {
        let _ = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(target), None, dir.as_ref().map_or(windows::core::PCWSTR::null(), |d| windows::core::PCWSTR(d.as_ptr())), SW_SHOWNORMAL);
    }
}

/// A program's own folder (as words for Windows), when `path` is a program.
pub fn program_dir(path: &str) -> Option<HSTRING> {
    let p = std::path::Path::new(path);
    if !path.to_lowercase().ends_with(".exe") || !p.is_file() {
        return None;
    }
    p.parent().map(|d| HSTRING::from(d.as_os_str()))
}

/// Open an address (`ms-settings:`, a web page) as the PC opens it.
pub fn launch_uri(uri: &str) {
    unsafe {
        let _ = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(uri), None, None, SW_SHOWNORMAL);
    }
}

/// How well a name matches what was typed: the start of the name, the start of a word, a part, or the letters in order (higher is
/// better; none: 0).
pub fn score(name: &str, q: &str) -> u32 {
    let (n, q) = (name.to_lowercase(), q.trim().to_lowercase());
    if q.is_empty() {
        return 1;
    }
    if n.starts_with(&q) {
        return 100 - (n.len() as u32).min(60) / 3;
    }
    if n.split([' ', '-', '_', '.']).any(|w| w.starts_with(&q)) {
        return 70;
    }
    if n.contains(&q) {
        return 50;
    }
    // (the letters in order: "vsc" finds "Visual Studio Code")
    let mut it = n.chars();
    if q.chars().all(|c| it.any(|x| x == c)) {
        return 20;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_typed_finds_names() {
        assert!(score("Visual Studio Code", "vis") > score("Visual Studio Code", "code"));
        assert!(score("Visual Studio Code", "code") > score("Visual Studio Code", "dio"));
        assert!(score("Visual Studio Code", "vsc") > 0);
        assert_eq!(score("Paint", "xyz"), 0);
    }
}
