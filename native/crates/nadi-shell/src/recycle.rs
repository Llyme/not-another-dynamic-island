//! The Recycle Bin widget: how full Windows' Recycle Bin is (a bin that fills), what was thrown away lately (each put back with a
//! press), emptied with a hold; files dropped on it are thrown away. Read now and then on a thread of its own (how many things, how
//! big: cheap; what they are only when that changed).

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::UI::Shell::*;

pub const BIN: &str = "bin";

/// how many of what was thrown away are listed
const LISTED: usize = 40;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// its name, as it was; the folder it came from; when (seconds since 1970); how big (bytes)
    pub name: String,
    pub from: String,
    pub when: i64,
    pub size: u64,
    /// where it is in the bin (its picture is read from there), and how the shell names it (to put it back, or delete it for good)
    pub path: String,
    pub parse: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub count: u64,
    pub size: u64,
    /// the room Windows keeps for it (bytes)
    pub room: u64,
    pub items: Vec<Item>,
}

static NOW: Mutex<Option<State>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Watch it (a thread of its own).
pub fn start() {
    std::thread::spawn(|| {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let mut last = (u64::MAX, u64::MAX);
        loop {
            let (count, size) = query();
            if (count, size) != last {
                last = (count, size);
                let s = State { count, size, room: room(), items: if count == 0 { Vec::new() } else { items() } };
                *NOW.lock().unwrap() = Some(s);
                DIRTY.store(true, Ordering::Relaxed);
            }
            let (m, cv) = &WAKE;
            let g = m.lock().unwrap();
            let (mut g, _) = cv.wait_timeout_while(g, std::time::Duration::from_secs(3), |w| !*w).unwrap();
            *g = false;
        }
    });
}

/// Look again now (something was put back, thrown away, emptied).
pub fn poke() {
    let (m, cv) = &WAKE;
    *m.lock().unwrap() = true;
    cv.notify_one();
}

/// What changed since last asked.
pub fn take() -> Option<State> {
    if DIRTY.swap(false, Ordering::Relaxed) {
        NOW.lock().unwrap().clone()
    } else {
        None
    }
}

fn query() -> (u64, u64) {
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe {
        if SHQueryRecycleBinW(PCWSTR::null(), &mut info).is_err() {
            return (0, 0);
        }
    }
    (info.i64NumItems.max(0) as u64, info.i64Size.max(0) as u64)
}

/// The room Windows keeps for it: each drive's (as set in its properties, in MB), together; else a twentieth of the system drive.
fn room() -> u64 {
    let mut total = 0u64;
    if let Ok(k) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume") {
        for name in k.enum_keys().flatten() {
            if let Ok(v) = k.open_subkey(&name).and_then(|s| s.get_value::<u32, _>("MaxCapacity")) {
                total += v as u64 * 1024 * 1024;
            }
        }
    }
    if total > 0 {
        return total;
    }
    let mut all = 0u64;
    unsafe {
        let _ = windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(&HSTRING::from("C:\\"), None, Some(&mut all), None);
    }
    (all / 20).max(1)
}

fn key(fmt: u128, pid: u32) -> windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
    windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY { fmtid: windows::core::GUID::from_u128(fmt), pid }
}

/// What is in it, newest first.
fn items() -> Vec<Item> {
    use windows::core::Interface;
    const RECYCLE: u128 = 0x9B174B33_40FF_11D2_A27E_00C04FC30871;
    const STORAGE: u128 = 0xB725F130_47EF_101A_A5F1_02608C9EEBAC;
    let mut out = Vec::new();
    unsafe {
        let Ok(bin) = SHGetKnownFolderItem::<_, IShellItem>(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, windows::Win32::Foundation::HANDLE::default()) else { return out };
        let Ok(e) = bin.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut got = [None];
            let mut n = 0u32;
            if e.Next(&mut got, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(item) = got[0].take() else { break };
            let Ok(i2) = item.cast::<IShellItem2>() else { continue };
            let text = |p: windows::core::Result<PWSTR>| -> String {
                match p {
                    Ok(p) => {
                        let s = p.to_string().unwrap_or_default();
                        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                        s
                    }
                    Err(_) => String::new(),
                }
            };
            // (its name alone: the shell may give where it was too)
            let name = text(item.GetDisplayName(SIGDN_NORMALDISPLAY));
            let name = if name.contains('\\') { std::path::Path::new(&name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(name) } else { name };
            let path = text(item.GetDisplayName(SIGDN_FILESYSPATH));
            let parse = text(item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING));
            let from = text(i2.GetString(&key(RECYCLE, 2)));
            let when = i2.GetFileTime(&key(RECYCLE, 3)).map(|f| (((f.dwHighDateTime as i64) << 32 | f.dwLowDateTime as i64) / 10_000_000) - 11_644_473_600).unwrap_or(0);
            let size = i2.GetUInt64(&key(STORAGE, 12)).unwrap_or(0);
            out.push(Item { name, from, when, size, path, parse });
        }
    }
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out.truncate(LISTED);
    out
}

/// Something in it, put back where it came from.
pub fn put_back(parse: String) {
    on_thread(move || unsafe {
        let _ = verb(&parse, windows::core::s!("undelete"));
    });
}

/// One of its things, as the Recycle Bin has it (by where it is kept in the bin): what is done to it is done as in the Recycle
/// Bin (made from its path alone, it would be a file like any other, with nothing to put it back).
unsafe fn in_bin(path: &str) -> Option<IShellItem> {
    let bin = SHGetKnownFolderItem::<_, IShellItem>(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, windows::Win32::Foundation::HANDLE::default()).ok()?;
    let e = bin.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems).ok()?;
    loop {
        let mut got = [None];
        let mut n = 0u32;
        if e.Next(&mut got, Some(&mut n)).is_err() || n == 0 {
            return None;
        }
        let Some(item) = got[0].take() else { return None };
        if let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH) {
            let s = p.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            if s.eq_ignore_ascii_case(path) {
                return Some(item);
            }
        }
    }
}

/// Something in it, deleted for good.
pub fn forget(parse: String) {
    on_thread(move || unsafe {
        let Some(item) = in_bin(&parse) else { return };
        let Ok(op) = windows::Win32::System::Com::CoCreateInstance::<_, IFileOperation>(&FileOperation, None, windows::Win32::System::Com::CLSCTX_ALL) else { return };
        let _ = op.SetOperationFlags(FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI);
        let _ = op.DeleteItem(&item, None);
        let _ = op.PerformOperations();
    });
}

/// All of it, deleted for good (the hold was the asking).
pub fn empty() {
    on_thread(|| unsafe {
        let _ = SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI);
    });
}

/// Files and folders dropped on it: thrown away (into the Recycle Bin, as Delete does in File Explorer).
pub fn throw_away(paths: Vec<String>) {
    on_thread(move || unsafe {
        let Ok(op) = windows::Win32::System::Com::CoCreateInstance::<_, IFileOperation>(&FileOperation, None, windows::Win32::System::Com::CLSCTX_ALL) else { return };
        let _ = op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOFX_RECYCLEONDELETE);
        for p in &paths {
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(p.as_str()), None) {
                let _ = op.DeleteItem(&item, None);
            }
        }
        let _ = op.PerformOperations();
    });
}

/// The Recycle Bin in File Explorer.
pub fn open() {
    crate::apps::launch_uri("shell:RecycleBinFolder");
}

fn on_thread(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        f();
        poke();
    });
}

/// A verb of an item's own menu (as its right-click menu would do it), by its name.
unsafe fn verb(parse: &str, name: windows::core::PCSTR) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu};
    let item = in_bin(parse).ok_or_else(|| "not in the Recycle Bin".to_string())?;
    let cm = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject).map_err(|e| format!("menu: {e}"))?;
    let menu = CreatePopupMenu().map_err(|e| format!("popup: {e}"))?;
    let r = cm.QueryContextMenu(menu, 0, 1, 0x7fff, CMF_NORMAL).map_err(|e| format!("query: {e}")).and_then(|_| {
        let info = CMINVOKECOMMANDINFO { cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32, lpVerb: name, nShow: 1, ..Default::default() };
        cm.InvokeCommand(&info).map_err(|e| format!("invoke: {e}"))
    });
    let _ = DestroyMenu(menu);
    r
}

/// (for checking) what is in it, and something put back now: what went wrong, if anything.
pub fn check_list() -> Vec<Item> {
    items()
}
pub fn check_put_back(parse: &str) -> Result<(), String> {
    unsafe { verb(parse, windows::core::s!("undelete")) }
}

fn size_text(b: u64) -> String {
    let b = b as f64;
    if b >= 1e12 {
        format!("{:.1} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{b:.0} B")
    }
}

fn ago(when: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let s = (now - when).max(0);
    match s {
        0..=59 => "Now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86399 => format!("{} h ago", s / 3600),
        86400..=172799 => "Yesterday".into(),
        _ => format!("{} days ago", s / 86400),
    }
}

/// The widget: the bin filling at rest (with its size beside it when large); opened, what was thrown away, newest first, each put
/// back with a press; at its foot how much of the room is used, File Explorer, and Empty (held). Empty, it is idle, unless files
/// are being dragged (somewhere to drop them). `lid`: something is dragged over it.
pub fn widget(s: &State, dragging: bool, lid: bool, large: bool) -> Value {
    let part = (s.size as f64 / s.room.max(1) as f64).clamp(0.0, 1.0);
    let full = part > 0.85;
    let mut peek = json!({ "bin": { "part": if s.count == 0 { 0.0 } else { part.max(0.08) }, "amber": full, "lid": lid }, "label": "Recycle Bin" });
    if large && s.count > 0 {
        peek["text"] = json!(size_text(s.size));
    }
    let rows: Vec<Value> = s
        .items
        .iter()
        .map(|it| {
            let from = std::path::Path::new(&it.from).file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| it.from.clone());
            json!({
                "image": if it.path.is_empty() { String::new() } else { format!("shell:{}", it.path) },
                "name": it.name, "sub": from, "when": ago(it.when), "size": if it.size > 0 { size_text(it.size) } else { String::new() },
                "back": "bin_back", "args": { "parse": it.parse },
                "menu": [
                    { "icon": "back", "label": "Restore", "action": "bin_back", "args": { "parse": it.parse } },
                    { "icon": "folder", "label": "Show in File Explorer", "action": "bin_open" },
                    { "icon": "x", "label": "Delete Permanently", "action": "bin_forget", "args": { "parse": it.parse }, "hold": true, "tone": "no" },
                ],
            })
        })
        .collect();
    let more = s.count.saturating_sub(s.items.len() as u64);
    let body = json!([{
        "kind": "binrows", "rows": rows, "more": if more > 0 { format!("{more} more") } else { String::new() },
        "used": part, "used_text": format!("{} of {}", size_text(s.size), size_text(s.room)), "open": "bin_open", "empty": "bin_empty",
    }]);
    json!({
        "idle": s.count == 0 && !dragging,
        "peek": peek,
        "drop": "bin_drop",
        "tip": if s.count == 0 { "Recycle Bin".to_string() } else { format!("Recycle Bin\n{} {}, {}", s.count, if s.count == 1 { "thing" } else { "things" }, size_text(s.size)) },
        "panel": { "width": 420, "body": if s.count == 0 { json!([{ "kind": "text", "text": "Nothing in the Recycle Bin." }]) } else { body } },
    })
}
