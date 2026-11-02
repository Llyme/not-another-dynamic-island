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

/// What the plugins add to the apps (by plugin): each one's name, the address that opens it (its id among the apps), its picture.
/// An app a plugin adds: its name, its address, its picture, and its badge (where it is from: a picture of the plugin's, or a
/// pictogram) with the badge's tip.
#[derive(Clone)]
struct PluginApp {
    name: String,
    open: String,
    image: String,
    badge: String,
    tip: String,
}

static FROM_PLUGINS: std::sync::Mutex<std::collections::BTreeMap<String, Vec<PluginApp>>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// What a plugin adds to the apps now (`{ name, open, image, badge, tip }` each).
pub fn set_from_plugin(plugin: &str, apps: &[serde_json::Value]) {
    let s = |a: &serde_json::Value, k: &str| a[k].as_str().unwrap_or("").to_string();
    let list: Vec<PluginApp> = apps
        .iter()
        .filter_map(|a| Some(PluginApp { name: a["name"].as_str()?.to_string(), open: a["open"].as_str()?.to_string(), image: s(a, "image"), badge: s(a, "badge"), tip: s(a, "tip") }))
        .collect();
    let mut all = FROM_PLUGINS.lock().unwrap();
    if list.is_empty() {
        all.remove(plugin);
    } else {
        all.insert(plugin.to_string(), list);
    }
}

/// The picture a plugin gave one of its apps (by its address).
pub fn plugin_image(id: &str) -> Option<String> {
    FROM_PLUGINS.lock().unwrap().values().flatten().find(|a| a.open == id && !a.image.is_empty()).map(|a| a.image.clone())
}

/// The badge a plugin gave an app (by its address, or by its name: the PC's own entry for one of a plugin's), and its tip.
pub fn plugin_badge(id: &str, name: &str) -> Option<(String, String)> {
    FROM_PLUGINS.lock().unwrap().values().flatten().find(|a| !a.badge.is_empty() && (a.open == id || same_name(&a.name) == same_name(name))).map(|a| (a.badge.clone(), a.tip.clone()))
}

/// The apps, as last found (finding them takes most of a second: it is done on a thread of its own, at the start and now and then),
/// with what the plugins add (one the PC has already, by its name, not twice).
pub fn cached() -> Vec<App> {
    let (fresh, mut list) = match &*CACHE.lock().unwrap() {
        Some((at, list)) => (at.elapsed().as_secs() < 120, list.clone()),
        None => (false, Vec::new()),
    };
    if !fresh {
        refresh_in_background();
    }
    let have: std::collections::HashSet<String> = list.iter().map(|a| same_name(&a.name)).collect();
    for a in FROM_PLUGINS.lock().unwrap().values().flatten() {
        if !have.contains(&same_name(&a.name)) {
            list.push(App { name: a.name.clone(), id: a.open.clone() });
        }
    }
    // (stable: the PC's own entry before a plugin's, the one kept)
    list.sort_by_key(|a| same_name(&a.name));
    // (one app once: by its address, or by its name (a game's shortcut and its Start menu entry; marks and spaces aside))
    list.dedup_by(|a, b| a.id == b.id || same_name(&a.name) == same_name(&b.name));
    list
}

/// A name as it is compared: its letters and digits alone, small ("A Game: Part 3™" and "a game part 3" are one).
pub fn same_name(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
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
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.id == b.id);
    out
}


/// The shell item of an app (for its picture).
pub fn item(id: &str) -> Option<IShellItem> {
    unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(format!("shell:AppsFolder\\{id}")), None).ok() }
}

/// Launch an app by its id; a path (a program, a shortcut) is opened as it is.
pub fn launch(id: &str) {
    // (an address (a plugin's, as `steam://...`): opened as one)
    if id.contains("://") {
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
    // (the letters in order: "ntpd" finds "Notepad")
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
        assert!(score("Vista Studio Code", "vis") > score("Vista Studio Code", "code"));
        assert!(score("Vista Studio Code", "code") > score("Vista Studio Code", "dio"));
        assert!(score("Vista Studio Code", "vsc") > 0);
        assert_eq!(score("Paint", "xyz"), 0);
    }
}
