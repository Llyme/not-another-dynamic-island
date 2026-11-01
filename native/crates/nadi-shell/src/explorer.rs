//! File Explorer's right-click menu: "NADI", and in it "Pin to Launcher" and "Pin to Dock", on files, shortcuts and folders (in the full menu: on
//! Windows 11, under "Show more options"). Written in the person's own part of the registry, so no administrator is needed; taken
//! away when the setting is turned off, and by the uninstaller. Pressed, a new NADI is started with the path: it leaves the request in
//! NADI's folder and ends, and the NADI that runs pins it.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Registry::{RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};

/// where the menu's item is (files of every kind, folders): "NADI", and in it, "Pin to Launcher" and "Pin to Dock" (keys named so that
/// Explorer keeps them in that order)
/// (shortcuts have a menu of their own: there "%1" is the shortcut itself, where every file's would give what it leads to)
const PLACES: [&str; 3] = ["Software\\Classes\\*\\shell", "Software\\Classes\\Directory\\shell", "Software\\Classes\\lnkfile\\shell"];
const MENU: &str = "NADI";
/// each: its key, its words, what it starts NADI with, which list it is about (0: the launcher's, 1: the dock's), and whether it is
/// shown for what is in that list (unpin) or for what is not (pin)
const ITEMS: [(&str, &str, &str, usize, bool); 4] = [
    ("1.PinLauncher", "Pin to Launcher", "--pin-apps", 0, false),
    ("2.UnpinLauncher", "Unpin from Launcher", "--unpin-apps", 0, true),
    ("3.PinDock", "Pin to Dock", "--pin-dock", 1, false),
    ("4.UnpinDock", "Unpin from Dock", "--unpin-dock", 1, true),
];
/// (the items' keys of before)
const OLD_ITEMS: [&str; 2] = ["1.PinLauncher", "2.PinDock"];
/// (the items of before: one each, not in a menu of their own)
const OLD: [&str; 2] = ["NADI.PinDock", "NADI.PinApps"];

fn set_string(key: &str, name: Option<&str>, value: &str) {
    let v: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let name = name.map(HSTRING::from);
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            REG_SZ.0,
            Some(v.as_ptr() as *const _),
            (v.len() * 2) as u32,
        );
    }
}

fn delete(key: &str) {
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key));
    }
}

/// What it was last written as (the same again: nothing written).
static LAST: std::sync::Mutex<Option<(bool, Vec<String>, Vec<String>, String)>> = std::sync::Mutex::new(None);

/// Which of what Explorer shows a menu item is for: each path, as File Explorer's search words (`not`: all but those).
fn applies_to(paths: &[String], not: bool) -> String {
    let any = paths.iter().map(|p| format!("System.ItemPathDisplay:=\"{p}\"")).collect::<Vec<_>>().join(" OR ");
    if not { format!("NOT ({any})") } else { any }
}

/// The menu (on), or gone (off), pointing at this NADI where it is now: "Pin to" for what is not pinned there yet, "Unpin from" for
/// what is (`launcher`, `dock`: the paths pinned to each). Written again only when something of it changed.
pub fn set(on: bool, launcher: &[String], dock: &[String]) {
    let exe_now = std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_default();
    {
        let mut last = LAST.lock().unwrap();
        let now = (on, launcher.to_vec(), dock.to_vec(), exe_now);
        if last.as_ref() == Some(&now) {
            return;
        }
        *last = Some(now);
    }
    for place in PLACES {
        for old in OLD {
            delete(&format!("{place}\\{old}"));
        }
        let menu = format!("{place}\\{MENU}");
        if !on {
            delete(&menu);
            continue;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let exe = exe.display().to_string();
        // (NADI's logo, as a file of its own in NADI's folder: Explorer shows it beside the words)
        let icon = icon_file().unwrap_or_else(|| format!("{exe},0"));
        set_string(&menu, Some("MUIVerb"), "NADI");
        set_string(&menu, Some("Icon"), &icon);
        set_string(&menu, Some("SubCommands"), "");
        // (every file's menu: not on a shortcut, whose own menu has it)
        if place.contains("\\*\\") {
            set_string(&menu, Some("AppliesTo"), "NOT System.FileExtension:=\".lnk\"");
        }
        // (several chosen at once: shown, and each pinned)
        set_string(&menu, Some("MultiSelectModel"), "Player");
        for old in OLD_ITEMS {
            delete(&format!("{menu}\\shell\\{old}"));
        }
        for (key, label, arg, list, unpin) in ITEMS {
            let k = format!("{menu}\\shell\\{key}");
            let paths = if list == 0 { launcher } else { dock };
            // (nothing pinned there: no "Unpin from"; "Pin to" for everything)
            if unpin && paths.is_empty() {
                delete(&k);
                continue;
            }
            delete(&k);
            set_string(&k, Some("MUIVerb"), label);
            set_string(&k, Some("MultiSelectModel"), "Player");
            if !paths.is_empty() {
                set_string(&k, Some("AppliesTo"), &applies_to(paths, !unpin));
            }
            set_string(&format!("{k}\\command"), None, &format!("\"{exe}\" {arg} \"%1\""));
        }
    }
}

/// NADI's logo, written beside its settings (the same picture the exe carries).
fn icon_file() -> Option<String> {
    let f = nadi_core::paths::data_dir()?.join("nadi.ico");
    let logo: &[u8] = include_bytes!("../assets/nadi.ico");
    if std::fs::metadata(&f).map_or(true, |m| m.len() != logo.len() as u64) {
        std::fs::write(&f, logo).ok()?;
    }
    Some(f.display().to_string())
}

fn requests() -> Option<std::path::PathBuf> {
    Some(nadi_core::paths::data_dir()?.join("pin-requests.txt"))
}

/// Left by a NADI started from the menu: where to pin what (`dock` or `apps`, and the path).
pub fn leave(to: &str, path: &str) {
    let Some(f) = requests() else { return };
    use std::io::Write;
    if let Ok(mut o) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
        let _ = writeln!(o, "{to}|{path}");
    }
}

/// What is chosen in a folder's view (as Windows' own scripting sees it).
unsafe fn chosen_in(view: &windows::Win32::UI::Shell::IShellFolderViewDual) -> Vec<String> {
    use windows::core::VARIANT;
    let Ok(items) = view.SelectedItems() else { return Vec::new() };
    let count = items.Count().unwrap_or(0);
    let mut paths = Vec::new();
    for j in 0..count {
        if let Ok(it) = items.Item(&VARIANT::from(j)) {
            if let Ok(p) = it.Path() {
                paths.push(p.to_string());
            }
        }
    }
    paths
}

/// What is chosen on the desktop.
unsafe fn chosen_on_desktop(sw: &windows::Win32::UI::Shell::IShellWindows) -> Option<Vec<String>> {
    use windows::core::{Interface, VARIANT};
    use windows::Win32::System::Com::{IDispatch, IServiceProvider};
    use windows::Win32::UI::Shell::{IShellBrowser, IShellFolderViewDual, IShellView, SID_STopLevelBrowser, CSIDL_DESKTOP, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH};
    let mut hwnd = 0i32;
    let disp = sw.FindWindowSW(&VARIANT::from(CSIDL_DESKTOP as i32), &VARIANT::default(), SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH).ok()?;
    let sp: IServiceProvider = disp.cast().ok()?;
    let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser).ok()?;
    let view: IShellView = browser.QueryActiveShellView().ok()?;
    let d: IDispatch = view.GetItemObject(SVGIO_BACKGROUND).ok()?;
    let fv: IShellFolderViewDual = d.cast().ok()?;
    Some(chosen_in(&fv))
}

/// What is chosen on the desktop now (for checking).
pub fn desktop_selection() -> Option<Vec<String>> {
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::Shell::{IShellWindows, ShellWindows};
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        chosen_on_desktop(&sw)
    }
}

/// Everything chosen where `path` is chosen (a File Explorer window, one of its tabs, or the desktop: several files or folders at
/// once); else `path` alone.
pub fn selection_with(path: &str) -> Vec<String> {
    use windows::core::{Interface, VARIANT};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::Shell::{IShellFolderViewDual, IShellWindows, IWebBrowserApp, ShellWindows};
    let has = |paths: &[String]| paths.iter().any(|p| p.eq_ignore_ascii_case(path));
    let found = (|| -> Option<Vec<String>> {
        unsafe {
            let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
            if let Some(paths) = chosen_on_desktop(&sw).filter(|p| has(p)) {
                return Some(paths);
            }
            let n = sw.Count().ok()?;
            for i in 0..n {
                let Ok(d) = sw.Item(&VARIANT::from(i)) else { continue };
                let Ok(app) = d.cast::<IWebBrowserApp>() else { continue };
                let Ok(doc) = app.Document() else { continue };
                let Ok(view) = doc.cast::<IShellFolderViewDual>() else { continue };
                let paths = chosen_in(&view);
                if has(&paths) {
                    return Some(paths);
                }
            }
            None
        }
    })();
    found.unwrap_or_else(|| vec![path.to_string()])
}

/// What was left to pin since last looked (and the file gone).
pub fn take() -> Vec<(String, String)> {
    let Some(f) = requests() else { return Vec::new() };
    // (moved aside first, then read: one left while it is read goes into a new file, not lost)
    let taken = f.with_extension("taking");
    if std::fs::rename(&f, &taken).is_err() {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(&taken) else { return Vec::new() };
    let _ = std::fs::remove_file(&taken);
    // (each once: several chosen, each NADI started for them leaves them all)
    let mut out: Vec<(String, String)> = Vec::new();
    for (a, b) in text.lines().filter_map(|l| l.split_once('|')).map(|(a, b)| (a.to_string(), b.trim().to_string())).filter(|(_, p)| !p.is_empty()) {
        if !out.iter().any(|(x, y)| *x == a && y.eq_ignore_ascii_case(&b)) {
            out.push((a, b));
        }
    }
    out
}

fn sources_file() -> Option<std::path::PathBuf> {
    Some(nadi_core::paths::data_dir()?.join("explorer-pins.txt"))
}

/// What was pinned from File Explorer's menu that is not itself what is pinned (a shortcut, pinned as its program): where (`dock` or
/// `apps`), the path pressed on, and what was pinned.
pub fn sources() -> Vec<(String, String, String)> {
    let Some(f) = sources_file() else { return Vec::new() };
    std::fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            Some((it.next()?.to_string(), it.next()?.to_string(), it.next()?.to_string()))
        })
        .collect()
}

/// Kept: `source` was pinned to `to` as `id` (a shortcut, as NADI's own copy of it).
pub fn remember(to: &str, source: &str, id: &str) {
    let mut all: Vec<_> = sources().into_iter().filter(|(t, s, _)| !(t == to && s.eq_ignore_ascii_case(source))).collect();
    all.push((to.to_string(), source.to_string(), id.to_string()));
    let Some(f) = sources_file() else { return };
    let text: String = all.iter().map(|(t, s, i)| format!("{t}|{s}|{i}\n")).collect();
    let _ = std::fs::write(f, text);
}

/// A shortcut pinned, copied into NADI's own folder (so the pin still works when the one pinned is deleted or moved): where the
/// copy is. The same shortcut again: the copy there already.
pub fn keep_shortcut(from: &str) -> Option<String> {
    let dir = nadi_core::paths::data_dir()?.join("shortcuts");
    let src = std::path::Path::new(from);
    if src.starts_with(&dir) {
        return Some(from.to_string());
    }
    std::fs::create_dir_all(&dir).ok()?;
    let bytes = std::fs::read(src).ok()?;
    let stem = src.file_stem()?.to_string_lossy().to_string();
    let mut to = dir.join(format!("{stem}.lnk"));
    let mut n = 2;
    while to.exists() {
        if std::fs::read(&to).ok().as_deref() == Some(bytes.as_slice()) {
            return Some(to.display().to_string());
        }
        to = dir.join(format!("{stem} ({n}).lnk"));
        n += 1;
    }
    std::fs::write(&to, &bytes).ok()?;
    Some(to.display().to_string())
}

/// NADI's own copies (shortcuts, pictures) that no pin uses any more (`used`: every path the pins name): gone.
pub fn prune_kept(used: &[String]) {
    let Some(data) = nadi_core::paths::data_dir() else { return };
    for sub in ["shortcuts", "icons"] {
        let Ok(rd) = std::fs::read_dir(data.join(sub)) else { continue };
        for e in rd.flatten() {
            let p = e.path().display().to_string();
            if !used.iter().any(|u| u.eq_ignore_ascii_case(&p)) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Tell every NADI running that something was left to pin (it is pinned now, not a moment later).
pub fn nudge() {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, PostMessageW};
    unsafe {
        let mut after = windows::Win32::Foundation::HWND::default();
        while let Ok(h) = FindWindowExW(None, after, windows::core::w!("NadiTray"), None) {
            if h.0.is_null() {
                break;
            }
            let _ = PostMessageW(h, crate::tray::WM_PINS, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
            after = h;
        }
    }
}

/// A picture chosen in Windows' own Open box (PNG, JPEG, GIF, icon, ...); none: put away.
pub fn pick_picture() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH};
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let types = [COMDLG_FILTERSPEC { pszName: w!("Pictures"), pszSpec: w!("*.png;*.jpg;*.jpeg;*.gif;*.ico;*.bmp;*.webp") }];
        d.SetFileTypes(&types).ok()?;
        let _ = d.SetTitle(w!("Choose a Picture"));
        if let Ok(o) = d.GetOptions() {
            let _ = d.SetOptions(o | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM);
        }
        d.Show(None).ok()?;
        let item = d.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// A picture chosen for a pin, kept in NADI's own folder (so it stays when the one chosen is moved): where it is now.
pub fn keep_picture(from: &str) -> Option<String> {
    let dir = nadi_core::paths::data_dir()?.join("icons");
    std::fs::create_dir_all(&dir).ok()?;
    let ext = std::path::Path::new(from).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let to = dir.join(format!("{ms}.{ext}"));
    std::fs::copy(from, &to).ok()?;
    Some(to.display().to_string())
}

/// A picture of one's own no longer used: gone (only one NADI kept itself).
pub fn drop_picture(path: &str) {
    if let Some(dir) = nadi_core::paths::data_dir().map(|d| d.join("icons")) {
        if std::path::Path::new(path).starts_with(&dir) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The same, remembered (asked of Windows once for each shortcut).
pub fn shortcut_program_kept(path: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static KEPT: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
    let k = path.to_lowercase();
    if let Some(v) = KEPT.lock().unwrap().get_or_insert_with(HashMap::new).get(&k) {
        return v.clone();
    }
    let v = shortcut_program(path);
    KEPT.lock().unwrap().get_or_insert_with(HashMap::new).insert(k, v.clone());
    v
}

/// A shortcut's target, when it is a program (a shortcut to an app pins the app, so its windows are found on the dock).
pub fn shortcut_program(path: &str) -> Option<String> {
    shortcut_target(path).filter(|t| t.to_lowercase().ends_with(".exe") && std::path::Path::new(t).is_file())
}

/// What a shortcut leads to (a program, a file, a folder), when it is there.
pub fn shortcut_target(path: &str) -> Option<String> {
    use windows::core::Interface;
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    if !path.to_lowercase().ends_with(".lnk") {
        return None;
    }
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(&HSTRING::from(path), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let t = String::from_utf16_lossy(&buf[..n]);
        (!t.is_empty() && std::path::Path::new(&t).exists()).then_some(t)
    }
}
