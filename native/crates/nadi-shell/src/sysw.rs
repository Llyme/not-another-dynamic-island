//! The PC's small things, as widgets of the frame's own: the microphone or camera in use (and by which apps), the USB drives
//! plugged in (how full; eject), the keyboard's languages (switch), the virtual desktops (switch). Looked at on a thread of its
//! own a few times a second; the frame takes what changed.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Ioctl::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const PRIVACY: &str = "privacy";
pub const DRIVES: &str = "drives";
pub const LANG: &str = "lang";
pub const DESKS: &str = "desks";
pub const DEVICES: &str = "devices";

/// how often it looks (sooner when poked: after a press)
const EVERY: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drive {
    pub letter: char,
    pub label: String,
    pub free: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lang {
    pub hkl: isize,
    pub short: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// the apps using the microphone, the camera now
    pub mic: Vec<String>,
    pub cam: Vec<String>,
    pub drives: Vec<Drive>,
    pub langs: Vec<Lang>,
    /// the language of the window in front
    pub lang: isize,
    /// how many desktops, and which is shown
    pub desks: usize,
    pub desk: usize,
    /// the controllers on, and the Bluetooth devices that tell their battery (`{ controllers, devices }`)
    pub gear: Value,
}

static NOW: Mutex<Option<State>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Start looking (once).
pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        // (a card reader with no card: no "insert a disk" box)
        unsafe {
            windows::Win32::System::Diagnostics::Debug::SetErrorMode(windows::Win32::System::Diagnostics::Debug::SEM_FAILCRITICALERRORS | windows::Win32::System::Diagnostics::Debug::SEM_NOOPENFILEERRORBOX);
        }
        loop {
            let (n, at) = desks();
            let s = State { mic: in_use("microphone"), cam: in_use("webcam"), drives: drives(), langs: langs(), lang: lang_now(), desks: n, desk: at, gear: nadi_core::sensors::gear::state() };
            let mut now = NOW.lock().unwrap();
            if now.as_ref() != Some(&s) {
                *now = Some(s);
                DIRTY.store(true, Ordering::Relaxed);
            }
            drop(now);
            let (m, cv) = &WAKE;
            let g = m.lock().unwrap();
            let (mut g, _) = cv.wait_timeout_while(g, EVERY, |poked| !*poked).unwrap();
            *g = false;
        }
    });
}

/// Look again soon (something was pressed).
pub fn poke() {
    let (m, cv) = &WAKE;
    *m.lock().unwrap() = true;
    cv.notify_one();
}

/// What it saw, when that changed since last asked.
pub fn take() -> Option<State> {
    if !DIRTY.swap(false, Ordering::Relaxed) {
        return None;
    }
    NOW.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------------------------------------------- microphone, camera

/// The apps using a device now (Windows keeps when each started and stopped; started and not stopped: using it).
fn in_use(what: &str) -> Vec<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    let mut out = Vec::new();
    let Ok(store) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{what}")) else { return out };
    let using = |k: &winreg::RegKey| {
        let start: u64 = k.get_value("LastUsedTimeStart").unwrap_or(0);
        let stop: u64 = k.get_value("LastUsedTimeStop").unwrap_or(1);
        start != 0 && stop == 0
    };
    for name in store.enum_keys().flatten() {
        let Ok(k) = store.open_subkey(&name) else { continue };
        if name == "NonPackaged" {
            for exe in k.enum_keys().flatten() {
                if k.open_subkey(&exe).is_ok_and(|e| using(&e)) {
                    out.push(crate::winlist::app_name(&exe.replace('#', "\\").to_lowercase()));
                }
            }
        } else if using(&k) {
            // (a store app: "Publisher.AppName_hash")
            let id = name.split('_').next().unwrap_or(&name);
            out.push(id.rsplit('.').next().unwrap_or(id).to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------------------------------------------- drives

fn wide(s: &str) -> HSTRING {
    HSTRING::from(s)
}

/// Open a volume (`\\.\E:`) for asking it things, or for locking it.
fn open_volume(letter: char, write: bool) -> Option<HANDLE> {
    let access = if write { (GENERIC_READ | GENERIC_WRITE).0 } else { 0 };
    unsafe { CreateFileW(&wide(&format!(r"\\.\{letter}:")), access, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), HANDLE::default()).ok() }
}

/// Is a volume on USB (a stick, a card reader, or a disk in a box)?
fn on_usb(letter: char) -> bool {
    let Some(h) = open_volume(letter, false) else { return false };
    let q = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty, QueryType: PropertyStandardQuery, AdditionalParameters: [0] };
    let mut buf = [0u8; 1024];
    let mut got = 0u32;
    let ok = unsafe { DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(&q as *const _ as *const _), std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32, Some(buf.as_mut_ptr() as *mut _), buf.len() as u32, Some(&mut got), None) }.is_ok();
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok || (got as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return false;
    }
    let d = unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    d.BusType == BusTypeUsb
}

/// The drives plugged in by USB, with media in them (the PC's own disk never).
fn drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let system = std::env::var("SystemDrive").ok().and_then(|s| s.chars().next()).unwrap_or('C').to_ascii_uppercase();
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        if letter == system {
            continue;
        }
        let root = wide(&format!("{letter}:\\"));
        let kind = unsafe { GetDriveTypeW(&root) };
        // (2: removable; 3: fixed, which a USB disk in a box says it is)
        if kind != 2 && !(kind == 3 && on_usb(letter)) {
            continue;
        }
        let mut name = [0u16; 128];
        if unsafe { GetVolumeInformationW(&root, Some(&mut name), None, None, None, None) }.is_err() {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) }.is_err() || total == 0 {
            continue;
        }
        let label = String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)]);
        out.push(Drive { letter, label: if label.is_empty() { "USB Drive".into() } else { label }, free, total });
    }
    out
}

/// Eject a drive, as Explorer's Eject does: its volume locked and let go of, then its device asked to go. False: something
/// holds it.
pub fn eject(letter: char) -> bool {
    let Some(h) = open_volume(letter, true) else { return false };
    let mut ok = false;
    unsafe {
        let mut got = 0u32;
        // (a file open on it: a few tries, then give up)
        for _ in 0..10 {
            if DeviceIoControl(h, FSCTL_LOCK_VOLUME, None, 0, None, 0, Some(&mut got), None).is_ok() {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if ok {
            let _ = DeviceIoControl(h, FSCTL_DISMOUNT_VOLUME, None, 0, None, 0, Some(&mut got), None);
            let keep = PREVENT_MEDIA_REMOVAL { PreventMediaRemoval: BOOLEAN(0) };
            let _ = DeviceIoControl(h, IOCTL_STORAGE_MEDIA_REMOVAL, Some(&keep as *const _ as *const _), std::mem::size_of::<PREVENT_MEDIA_REMOVAL>() as u32, None, 0, Some(&mut got), None);
        }
        let number = device_number(h);
        if ok {
            // (the device asked to go, as safely removing it; else only its media out)
            let gone = number.is_some_and(|n| eject_device(n));
            if !gone {
                let _ = DeviceIoControl(h, IOCTL_STORAGE_EJECT_MEDIA, None, 0, None, 0, Some(&mut got), None);
            }
        }
        let _ = CloseHandle(h);
    }
    poke();
    ok
}

fn device_number(h: HANDLE) -> Option<u32> {
    let mut n = STORAGE_DEVICE_NUMBER::default();
    let mut got = 0u32;
    unsafe { DeviceIoControl(h, IOCTL_STORAGE_GET_DEVICE_NUMBER, None, 0, Some(&mut n as *mut _ as *mut _), std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32, Some(&mut got), None) }.ok()?;
    Some(n.DeviceNumber)
}

/// The disk of that number: the device it hangs from asked to be ejected.
fn eject_device(number: u32) -> bool {
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(Some(&GUID_DEVINTERFACE_DISK), PCWSTR::null(), HWND::default(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) else { return false };
        let mut done = false;
        let mut i = 0;
        loop {
            let mut ifd = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVINTERFACE_DISK, i, &mut ifd).is_err() {
                break;
            }
            i += 1;
            let mut need = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &ifd, None, 0, Some(&mut need), None);
            if need == 0 {
                continue;
            }
            let mut buf = vec![0u64; (need as usize + 7) / 8];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, Some(detail), need, None, Some(&mut info)).is_err() {
                continue;
            }
            let path = PCWSTR((*detail).DevicePath.as_ptr());
            let Ok(h) = CreateFileW(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), HANDLE::default()) else { continue };
            let n = device_number(h);
            let _ = CloseHandle(h);
            if n != Some(number) {
                continue;
            }
            let mut parent = 0u32;
            if CM_Get_Parent(&mut parent, info.DevInst, 0) == CR_SUCCESS {
                for _ in 0..3 {
                    let mut veto = PNP_VETO_TYPE(0);
                    if CM_Request_Device_EjectW(parent, Some(&mut veto), None, 0) == CR_SUCCESS && veto.0 == 0 {
                        done = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
            break;
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        done
    }
}

/// Open a drive in Explorer.
pub fn open_drive(letter: char) {
    crate::apps::launch_uri(&format!("{letter}:\\"));
}

fn size_text(b: u64) -> String {
    let gb = b as f64 / 1_073_741_824.0;
    if gb >= 100.0 {
        format!("{gb:.0} GB")
    } else if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", b as f64 / 1_048_576.0)
    }
}

// ---------------------------------------------------------------------------------------------------------------- languages

fn locale_of(hkl: isize) -> String {
    let mut name = [0u16; 85];
    let n = unsafe { windows::Win32::Globalization::LCIDToLocaleName((hkl as usize & 0xffff) as u32, Some(&mut name), 0) };
    String::from_utf16_lossy(&name[..(n.max(1) - 1) as usize])
}

fn locale_info(locale: &str, what: u32) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { windows::Win32::Globalization::GetLocaleInfoEx(&wide(locale), what, Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

/// The keyboard's languages (as many as Windows has for typing).
fn langs() -> Vec<Lang> {
    let n = unsafe { GetKeyboardLayoutList(None) };
    if n <= 0 {
        return Vec::new();
    }
    let mut list = vec![HKL::default(); n as usize];
    let n = unsafe { GetKeyboardLayoutList(Some(&mut list)) };
    list.truncate(n.max(0) as usize);
    let mut out: Vec<Lang> = list
        .iter()
        .map(|h| {
            let loc = locale_of(h.0 as isize);
            let short = locale_info(&loc, windows::Win32::Globalization::LOCALE_SISO639LANGNAME).to_uppercase();
            // (its language alone: "English"; with its country when two share it)
            let name = locale_info(&loc, 0x6f);
            Lang { hkl: h.0 as isize, short, name }
        })
        .collect();
    let names: Vec<String> = out.iter().map(|l| l.name.clone()).collect();
    for l in &mut out {
        if names.iter().filter(|n| **n == l.name).count() > 1 {
            l.name = locale_info(&locale_of(l.hkl), windows::Win32::Globalization::LOCALE_SLOCALIZEDDISPLAYNAME);
        }
    }
    out
}

/// The language the window in front types in.
fn lang_now() -> isize {
    unsafe {
        let t = GetWindowThreadProcessId(GetForegroundWindow(), None);
        GetKeyboardLayout(t).0 as isize
    }
}

/// The window in front types in another language.
pub fn switch_lang(hkl: isize) {
    unsafe {
        let fg = GetForegroundWindow();
        let _ = PostMessageW(fg, WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(hkl));
    }
    poke();
}

// ---------------------------------------------------------------------------------------------------------------- desktops

/// How many virtual desktops, and which is shown (Explorer keeps their ids, 16 bytes each, and the current one's).
fn desks() -> (usize, usize) {
    use winreg::enums::HKEY_CURRENT_USER;
    let hk = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let Ok(k) = hk.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops") else { return (1, 0) };
    let ids = k.get_raw_value("VirtualDesktopIDs").map(|v| v.bytes).unwrap_or_default();
    let n = (ids.len() / 16).max(1);
    let cur = k.get_raw_value("CurrentVirtualDesktop").map(|v| v.bytes).ok().or_else(|| {
        // (older Windows: kept for the session)
        let mut sid = 0u32;
        unsafe {
            let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(std::process::id(), &mut sid);
        }
        hk.open_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{sid}\VirtualDesktops")).ok()?.get_raw_value("CurrentVirtualDesktop").map(|v| v.bytes).ok()
    });
    let at = cur.and_then(|c| ids.chunks(16).position(|id| id == c.as_slice())).unwrap_or(0);
    (n, at)
}

/// Show another desktop: Windows' own keys (Win+Ctrl+arrow), as many times as it is away.
pub fn switch_desk(from: usize, to: usize) {
    if from == to {
        return;
    }
    let key = if to > from { VK_RIGHT } else { VK_LEFT };
    let times = from.abs_diff(to);
    std::thread::spawn(move || {
        for _ in 0..times {
            crate::launcher::combo(&[VK_LWIN.0, VK_LCONTROL.0, key.0]);
            std::thread::sleep(Duration::from_millis(180));
        }
        poke();
    });
}

// ---------------------------------------------------------------------------------------------------------------- devices

/// What a device is drawn as: a controller, a headset, a mouse, else Bluetooth's mark.
fn device_icon(kind: &str) -> &'static str {
    match kind {
        "headset" => "headphones",
        "controller" => "gamepad",
        "mouse" => "cursor",
        _ => "bluetooth",
    }
}

/// The Devices widget: what is connected (controllers, Bluetooth devices), each with how full its battery is. At rest: the one
/// lowest, its pictogram and cells (amber, glowing, when low). None connected: idle.
pub fn devices(s: &State, paired: &[crate::net::Paired]) -> Value {
    let mut rows: Vec<(Option<f64>, Value)> = Vec::new();
    for c in s.gear["controllers"].as_array().into_iter().flatten() {
        let level = c["level"].as_f64();
        let word = if c["wired"] == true {
            "Wired".to_string()
        } else {
            match level {
                Some(l) if l >= 0.99 => "Full".into(),
                Some(l) if l >= 0.6 => "Medium".into(),
                Some(l) if l >= 0.3 => "Low".into(),
                Some(_) => "Empty".into(),
                None => String::new(),
            }
        };
        rows.push((level, json!({ "icon": "gamepad", "title": format!("Controller {}", c["slot"].as_u64().unwrap_or(1)), "right": word, "progress": level, "lit": level.is_some_and(|l| l > 0.25) })));
    }
    let mut names: Vec<String> = Vec::new();
    for d in s.gear["devices"].as_array().into_iter().flatten() {
        let name = d["name"].as_str().unwrap_or("").to_string();
        let level = d["level"].as_f64();
        names.push(name.to_lowercase());
        rows.push((level, json!({ "icon": device_icon(d["kind"].as_str().unwrap_or("")), "title": name, "right": level.map(|l| format!("{}%", (l * 100.0).round() as i32)).unwrap_or_default(), "progress": level, "lit": level.is_some_and(|l| l > 0.25) })));
    }
    // (connected, but its battery not told)
    for p in paired.iter().filter(|p| p.connected && !names.contains(&p.name.to_lowercase())) {
        let kind = if crate::audiodev::is_headset(&p.name) { "headset" } else { "" };
        rows.push((None, json!({ "icon": device_icon(kind), "title": p.name, "right": "", "lit": true })));
    }
    if rows.is_empty() {
        return json!({ "idle": true, "peek": { "icon": "bluetooth" } });
    }
    let low = rows.iter().filter(|(l, _)| l.is_some()).min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (lead, level) = match low {
        Some((l, r)) => (r["icon"].as_str().unwrap_or("bluetooth").to_string(), *l),
        None => (rows[0].1["icon"].as_str().unwrap_or("bluetooth").to_string(), None),
    };
    let lowish = level.is_some_and(|l| l <= 0.2);
    let tip = rows.iter().map(|(_, r)| { let right = r["right"].as_str().unwrap_or(""); if right.is_empty() { r["title"].as_str().unwrap_or("").to_string() } else { format!("{}, {right}", r["title"].as_str().unwrap_or("")) } }).collect::<Vec<_>>().join("\n");
    let mut peek = json!({ "icon": lead, "label": tip.lines().next().unwrap_or("") });
    if level.is_some() {
        peek["cells"] = json!({ "icon": "", "level": level });
    }
    let mut out = json!({ "peek": peek, "panel": { "body": [{ "kind": "list", "rows": rows.into_iter().map(|(_, r)| r).collect::<Vec<_>>() }] }, "tip": tip });
    if lowish {
        out["glow"] = json!(AMBER);
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------- the widgets

const AMBER: &str = "#E8B04A";

/// Each of its widgets, as it is now (one with nothing to show is idle: hidden).
pub fn widgets(s: &State) -> Vec<(&'static str, Value)> {
    let idle = |icon: &str| json!({ "idle": true, "peek": { "icon": icon } });
    // the microphone, the camera: amber, glowing; its tip names who uses it
    let privacy = if s.mic.is_empty() && s.cam.is_empty() {
        idle("mic")
    } else {
        let mut who: Vec<String> = s.cam.iter().chain(s.mic.iter()).cloned().collect();
        who.sort();
        who.dedup();
        let what = match (s.cam.is_empty(), s.mic.is_empty()) {
            (false, false) => "Camera and Microphone",
            (false, true) => "Camera",
            _ => "Microphone",
        };
        json!({
            "peek": { "icon": if s.cam.is_empty() { "mic" } else { "camera" }, "color": AMBER, "label": format!("{what} In Use") },
            "glow": AMBER,
            "tip": format!("{what}\n{}", who.join(", ")),
        })
    };
    // the USB drives: how full the first; open each, eject it (held)
    let drives = if s.drives.is_empty() {
        idle("usb")
    } else {
        let d0 = &s.drives[0];
        let used = |d: &Drive| (1.0 - d.free as f64 / d.total as f64).clamp(0.0, 1.0);
        let mut body = Vec::new();
        for d in &s.drives {
            let u = used(d);
            body.push(json!({ "kind": "meter", "label": d.label, "sub": format!("{}:", d.letter), "text": format!("{} Free", size_text(d.free)), "part": u, "color": if u > 0.9 { "#E87A7A" } else { "#5AC88C" } }));
            body.push(json!({ "kind": "buttons", "items": [
                { "label": "Open", "action": format!("opendrive:{}", d.letter) },
                { "label": "Eject", "action": format!("eject:{}", d.letter), "hold": true, "tip": "Hold to Eject" },
            ] }));
        }
        let tip = s.drives.iter().map(|d| format!("{} ({}:)\n{} Free of {}", d.label, d.letter, size_text(d.free), size_text(d.total))).collect::<Vec<_>>().join("\n");
        json!({
            "peek": { "icon": "usb", "ring": { "part": used(d0) }, "label": d0.label },
            "panel": { "body": body },
            "tip": tip,
        })
    };
    // the languages: its short name; the others to switch to
    let lang = if s.langs.len() < 2 {
        idle("language")
    } else {
        let now = s.langs.iter().find(|l| l.hkl == s.lang).or(s.langs.first()).unwrap();
        let options: Vec<Value> = s.langs.iter().map(|l| json!({ "label": l.name, "value": l.hkl.to_string() })).collect();
        json!({
            "peek": { "icon": "", "text": now.short, "label": now.name },
            "panel": { "body": [{ "kind": "choice", "action": "lang", "value": now.hkl.to_string(), "options": options }] },
            "tip": now.name,
        })
    };
    // the desktops: a little box each, the one shown lit
    let desks = if s.desks < 2 {
        idle("desktop")
    } else {
        json!({
            "peek": { "icon": "desktop", "desks": { "n": s.desks, "at": s.desk }, "label": format!("Desktop {}", s.desk + 1) },
            "tip": format!("Desktop {} of {}", s.desk + 1, s.desks),
        })
    };
    vec![(PRIVACY, privacy), (DRIVES, drives), (LANG, lang), (DESKS, desks)]
}

#[cfg(test)]
mod tests {
    /// (on this PC: what it reads; changes nothing)
    #[test]
    #[ignore]
    fn reads_this_pc() {
        let (n, at) = super::desks();
        println!("mic {:?} cam {:?}", super::in_use("microphone"), super::in_use("webcam"));
        println!("drives {:?}", super::drives());
        println!("langs {:?} now {:x}", super::langs(), super::lang_now());
        println!("desks {n} at {at}");
    }
}
