//! The PC's small things, as widgets of the frame's own: the microphone or camera in use (and by which apps), the USB drives
//! plugged in (how full; eject), the keyboard's languages (switch), the virtual desktops (switch). Looked at on a thread of its
//! own a few times a second; the frame takes what changed.

use serde_json::{json, Value};
use std::collections::BTreeMap;
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
    /// on USB (it can be ejected); the one Windows is on
    pub usb: bool,
    pub system: bool,
    /// how fast it reads and writes now (bytes a second), and the last minute of each, oldest first
    pub rs: f32,
    pub ws: f32,
    pub reads: Vec<f32>,
    pub writes: Vec<f32>,
}

/// how many of its reads and writes are kept (one a look: about a minute)
const IO_KEPT: usize = 40;

/// each drive's bytes read and written so far, when, and the rates since
struct Io {
    read: i64,
    written: i64,
    at: std::time::Instant,
    reads: std::collections::VecDeque<f32>,
    writes: std::collections::VecDeque<f32>,
}

static IO: Mutex<Option<std::collections::HashMap<char, Io>>> = Mutex::new(None);

/// what an eject came to, and when: ejected (and the drive as it was), or refused (something has a file open on it)
static EJECTS: Mutex<Option<std::collections::HashMap<char, (bool, std::time::Instant, Option<Drive>)>>> = Mutex::new(None);

/// How fast a drive reads and writes now (bytes a second), and the last minute of each: from what Windows counts for it.
fn io_of(letter: char) -> (f32, f32, Vec<f32>, Vec<f32>) {
    let Some(h) = open_volume(letter, false) else { return (0.0, 0.0, Vec::new(), Vec::new()) };
    let mut perf = DISK_PERFORMANCE::default();
    let mut got = 0u32;
    let ok = unsafe { DeviceIoControl(h, IOCTL_DISK_PERFORMANCE, None, 0, Some(&mut perf as *mut _ as *mut _), std::mem::size_of::<DISK_PERFORMANCE>() as u32, Some(&mut got), None) }.is_ok();
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok {
        return (0.0, 0.0, Vec::new(), Vec::new());
    }
    let now = std::time::Instant::now();
    let mut all = IO.lock().unwrap();
    let map = all.get_or_insert_with(Default::default);
    let io = map.entry(letter).or_insert(Io { read: perf.BytesRead, written: perf.BytesWritten, at: now, reads: Default::default(), writes: Default::default() });
    let dt = now.duration_since(io.at).as_secs_f32();
    let (mut rs, mut ws) = (0.0, 0.0);
    if dt > 0.2 {
        rs = ((perf.BytesRead - io.read).max(0) as f32 / dt).max(0.0);
        ws = ((perf.BytesWritten - io.written).max(0) as f32 / dt).max(0.0);
        io.read = perf.BytesRead;
        io.written = perf.BytesWritten;
        io.at = now;
        io.reads.push_back(rs);
        io.writes.push_back(ws);
        while io.reads.len() > IO_KEPT {
            io.reads.pop_front();
        }
        while io.writes.len() > IO_KEPT {
            io.writes.pop_front();
        }
    } else if let (Some(r), Some(w)) = (io.reads.back(), io.writes.back()) {
        (rs, ws) = (*r, *w);
    }
    (rs, ws, io.reads.iter().copied().collect(), io.writes.iter().copied().collect())
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
    /// each desktop, in order: its name, its wallpaper, its windows; and the monitors (physical px, on the desktop)
    pub maps: Vec<DeskMap>,
    pub screens: Vec<(i32, i32, i32, i32)>,
}

/// A window on a desktop's map: its handle, its program, its box (physical px), whether it is the one in front there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeskWin {
    pub hwnd: isize,
    pub exe: String,
    pub title: String,
    pub rect: (i32, i32, i32, i32),
    pub front: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeskMap {
    /// its id, as Windows keeps it (hex)
    pub id: String,
    pub name: String,
    pub wall: String,
    pub wins: Vec<DeskWin>,
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
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        loop {
            let (n, at) = desks();
            let screens = crate::overlay::monitors().iter().map(|m| (m.rect.left, m.rect.top, m.rect.right, m.rect.bottom)).collect();
            let s = State { mic: in_use("microphone"), cam: in_use("webcam"), drives: drives(), langs: langs(), lang: lang_now(), desks: n, desk: at, gear: nadi_core::sensors::gear::state(), maps: maps(at), screens };
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
                // (NADI's own, and its plugins' helpers (Speech): not another app listening)
                let path = exe.replace('#', "\\").to_lowercase();
                if path.contains("\\nadi\\plugins\\") || path.ends_with("\\nadi.exe") {
                    continue;
                }
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

/// Every drive: the PC's own (the one Windows is on first), then the ones on USB, each with media in it; how fast each reads and
/// writes.
fn drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let system = std::env::var("SystemDrive").ok().and_then(|s| s.chars().next()).unwrap_or('C').to_ascii_uppercase();
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        let kind = unsafe { GetDriveTypeW(&root) };
        // (2: removable; 3: fixed, the PC's own, or a USB disk in a box, which says it is)
        if kind != 2 && kind != 3 {
            continue;
        }
        let usb = kind == 2 || (letter != system && on_usb(letter));
        let mut name = [0u16; 128];
        if unsafe { GetVolumeInformationW(&root, Some(&mut name), None, None, None, None) }.is_err() {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) }.is_err() || total == 0 {
            continue;
        }
        let label = String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)]);
        let (rs, ws, reads, writes) = io_of(letter);
        let label = if !label.is_empty() {
            label
        } else if usb {
            "USB Drive".into()
        } else if letter == system {
            "Windows".into()
        } else {
            "Local Disk".into()
        };
        out.push(Drive { letter, label, free, total, usb, system: letter == system, rs, ws, reads, writes });
    }
    // (the PC's own first, Windows' at the top; then the USB ones)
    out.sort_by_key(|d| (d.usb, !d.system, d.letter));
    out
}

/// Bytes a second, short: "85 MB/s".
fn rate_text(b: f32) -> String {
    let mb = b / 1_048_576.0;
    if mb >= 100.0 {
        format!("{mb:.0} MB/s")
    } else if mb >= 1.0 {
        format!("{mb:.1} MB/s")
    } else {
        format!("{:.0} KB/s", (b / 1024.0).max(1.0))
    }
}

/// Busy enough to show: reading or writing more than a little.
const BUSY: f32 = 512.0 * 1024.0;

/// Open a drive's Properties, as Explorer does.
pub fn drive_properties(letter: char) {
    unsafe {
        let _ = windows::Win32::UI::Shell::SHObjectProperties(HWND::default(), windows::Win32::UI::Shell::SHOP_FILEPATH, &wide(&format!("{letter}:\\")), PCWSTR::null());
    }
}

/// Open a terminal at a drive's top.
pub fn drive_terminal(letter: char) {
    let dir = format!("{letter}:\\");
    let wt = std::process::Command::new("wt.exe").args(["-d", &dir]).spawn();
    if wt.is_err() {
        let _ = std::process::Command::new("powershell.exe").args(["-NoExit", "-Command", &format!("Set-Location '{dir}'")]).current_dir(&dir).spawn();
    }
}

/// Eject a drive, as Explorer's Eject does: its volume locked and let go of, then its device asked to go. False: something
/// holds it.
pub fn eject(letter: char) -> bool {
    let was = NOW.lock().unwrap().as_ref().and_then(|s| s.drives.iter().find(|d| d.letter == letter).cloned());
    let ok = eject_now(letter);
    EJECTS.lock().unwrap().get_or_insert_with(Default::default).insert(letter, (ok, std::time::Instant::now(), was));
    // (the widget says so now, and says no more once it is old)
    DIRTY.store(true, Ordering::Relaxed);
    let _ = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(4200));
        DIRTY.store(true, Ordering::Relaxed);
    });
    ok
}

fn eject_now(letter: char) -> bool {
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
    // (a thousand GB and more: in TB, "1.8 TB", "4.5 TB"; ten and more without a decimal)
    if gb >= 1000.0 {
        let tb = gb / 1024.0;
        if tb >= 10.0 { format!("{tb:.0} TB") } else { format!("{tb:.1} TB") }
    } else if gb >= 100.0 {
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

/// Each desktop, in Windows' order: its name (as named in Task View, else "Desktop n"), its wallpaper, and its windows, top first,
/// the one in front marked.
fn maps(at: usize) -> Vec<DeskMap> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hk = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let Ok(k) = hk.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops") else { return Vec::new() };
    let ids = k.get_raw_value("VirtualDesktopIDs").map(|v| v.bytes).unwrap_or_default();
    let ids: Vec<[u8; 16]> = ids.chunks(16).filter(|c| c.len() == 16).map(|c| c.try_into().unwrap_or([0; 16])).collect();
    let common = wallpaper_now();
    let wins = crate::winlist::desktop_windows();
    let fg = unsafe { GetForegroundWindow() }.0 as isize;
    ids.iter()
        .enumerate()
        .map(|(i, id)| {
            let g = format!(
                "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
                u32::from_le_bytes([id[0], id[1], id[2], id[3]]),
                u16::from_le_bytes([id[4], id[5]]),
                u16::from_le_bytes([id[6], id[7]]),
                id[8], id[9], id[10], id[11], id[12], id[13], id[14], id[15]
            );
            let own = k.open_subkey(format!(r"Desktops\{g}")).ok();
            let name = own.as_ref().and_then(|d| d.get_value::<String, _>("Name").ok()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Desktop {}", i + 1));
            let wall = own.as_ref().and_then(|d| d.get_value::<String, _>("Wallpaper").ok()).filter(|w| !w.is_empty()).unwrap_or_else(|| common.clone());
            let mine: Vec<&(isize, String, RECT, [u8; 16], String)> = wins.iter().filter(|w| w.3 == *id).collect();
            // (the one in front: the window in front of all, on the desktop shown; else its top one)
            let front = if i == at && mine.iter().any(|w| w.0 == fg) { fg } else { mine.first().map_or(0, |w| w.0) };
            let wins = mine.iter().map(|w| DeskWin { hwnd: w.0, exe: w.1.clone(), title: w.4.clone(), rect: (w.2.left, w.2.top, w.2.right, w.2.bottom), front: w.0 == front }).collect();
            DeskMap { id: id.iter().map(|b| format!("{b:02x}")).collect(), name, wall, wins }
        })
        .collect()
}

/// The wallpaper every desktop has unless it has its own.
fn wallpaper_now() -> String {
    let mut buf = [0u16; 520];
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr() as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let n = buf.iter().position(|c| *c == 0).unwrap_or(0);
    String::from_utf16_lossy(&buf[..n])
}

/// Close a desktop: shown first, then Win + Ctrl + F4 (its windows go to the one beside it, as Windows does).
pub fn close_desk(from: usize, to: usize) {
    std::thread::spawn(move || {
        let times = from.abs_diff(to);
        let key = if to > from { VK_RIGHT } else { VK_LEFT };
        for _ in 0..times {
            crate::launcher::combo(&[VK_LWIN.0, VK_LCONTROL.0, key.0]);
            std::thread::sleep(Duration::from_millis(180));
        }
        std::thread::sleep(Duration::from_millis(250));
        crate::launcher::combo(&[VK_LWIN.0, VK_LCONTROL.0, VK_F4.0]);
        std::thread::sleep(Duration::from_millis(250));
        poke();
    });
}

/// Show a desktop and bring one of its windows to the front.
pub fn show_window_on(from: usize, to: usize, hwnd: isize) {
    switch_desk(from, to);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(220 * (from.abs_diff(to) as u64 + 1)));
        crate::winlist::activate(hwnd);
        poke();
    });
}

/// A new desktop, as Win + Ctrl + D makes one (it is shown).
pub fn new_desk() {
    std::thread::spawn(|| {
        crate::launcher::combo(&[VK_LWIN.0, VK_LCONTROL.0, VK_D.0]);
        std::thread::sleep(Duration::from_millis(250));
        poke();
    });
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

/// The Bluetooth widget: what is connected (Bluetooth devices, controllers), each with how full its battery is. At rest: up to three
/// of them, each its pictogram in a ring as full as its battery (amber when low, and it glows); its icon alone: the lowest. Opened:
/// Bluetooth on or off, each connected one as a card, the paired ones not connected (pressed: Windows' Bluetooth page), and its
/// settings. None connected: its mark (grey while Bluetooth is off), never idle, so Bluetooth can always be turned on from it.
pub fn devices(s: &State, paired: &[crate::net::Paired], bt: Option<bool>) -> Value {
    // (each connected: its pictogram, its name, its battery (none told: none))
    let mut devs: Vec<(String, String, Option<f64>)> = Vec::new();
    for c in s.gear["controllers"].as_array().into_iter().flatten() {
        devs.push(("gamepad".into(), format!("Controller {}", c["slot"].as_u64().unwrap_or(1)), if c["wired"] == true { None } else { c["level"].as_f64() }));
    }
    let mut names: Vec<String> = Vec::new();
    for d in s.gear["devices"].as_array().into_iter().flatten() {
        let name = d["name"].as_str().unwrap_or("").to_string();
        names.push(name.to_lowercase());
        devs.push((device_icon(d["kind"].as_str().unwrap_or("")).into(), name, d["level"].as_f64()));
    }
    for p in paired.iter().filter(|p| p.connected && !names.contains(&p.name.to_lowercase())) {
        let kind = if crate::audiodev::is_headset(&p.name) { "headset" } else { "" };
        devs.push((device_icon(kind).into(), p.name.clone(), None));
    }
    let low = |l: Option<f64>| l.is_some_and(|l| l <= 0.2);
    let mut body: Vec<Value> = Vec::new();
    if let Some(on) = bt {
        body.push(json!({ "kind": "toggles", "items": [{ "icon": "bluetooth", "label": "Bluetooth", "on": on, "action": "radio:bluetooth" }] }));
    }
    if !devs.is_empty() {
        let cards: Vec<Value> = devs.iter().map(|(icon, name, level)| json!({ "icon": icon, "name": name, "level": level, "low": low(*level), "sub": level.map(|l| format!("{}%", (l * 100.0).round() as i32)).unwrap_or_else(|| "Connected".into()) })).collect();
        body.push(json!({ "kind": "devcards", "items": cards }));
    }
    let idle_ones: Vec<&crate::net::Paired> = paired.iter().filter(|p| !p.connected).collect();
    if bt != Some(false) && !idle_ones.is_empty() {
        body.push(json!({ "kind": "text", "label": "Paired" }));
        let rows: Vec<Value> = idle_ones.iter().map(|p| json!({ "icon": device_icon(if crate::audiodev::is_headset(&p.name) { "headset" } else { "" }), "title": p.name, "dim": true, "action": "btsettings" })).collect();
        body.push(json!({ "kind": "list", "rows": rows }));
    }
    body.push(json!({ "kind": "buttons", "items": [{ "icon": "gear", "label": "Bluetooth Settings", "tip": "Bluetooth Settings", "action": "btsettings" }] }));
    let tip = if devs.is_empty() {
        if bt == Some(false) { "Bluetooth Off".to_string() } else { "Nothing Connected".to_string() }
    } else {
        devs.iter().map(|(_, n, l)| match l { Some(l) => format!("{n}, {}%", (l * 100.0).round() as i32), None => n.clone() }).collect::<Vec<_>>().join("\n")
    };
    if devs.is_empty() {
        // (never idle: hidden, Bluetooth could not be turned on from here; off, its mark grey)
        let mut peek = json!({ "icon": "bluetooth", "label": tip });
        if bt == Some(false) {
            peek["color"] = json!("#8e8e96");
        }
        return json!({ "peek": peek, "panel": { "width": 340, "body": body }, "tip": tip });
    }
    // (at rest: up to three, the lowest first)
    let mut rest: Vec<&(String, String, Option<f64>)> = devs.iter().collect();
    rest.sort_by(|a, b| a.2.unwrap_or(2.0).partial_cmp(&b.2.unwrap_or(2.0)).unwrap_or(std::cmp::Ordering::Equal));
    let items: Vec<Value> = rest.iter().take(3).map(|(icon, _, level)| json!({ "icon": icon, "level": level, "low": low(*level) })).collect();
    let mut out = json!({ "peek": { "devs": { "items": items }, "label": tip.lines().next().unwrap_or("") }, "panel": { "width": 340, "body": body }, "tip": tip });
    if devs.iter().any(|d| low(d.2)) {
        out["glow"] = json!(AMBER);
    }
    out
}

/// What happens on the windows of every desktop (the frame knows it): a window asking for you (its taskbar button would flash), the
/// programs making sound, and the count in a window's title ("(3) Discord").
#[derive(Default)]
pub struct Activity {
    pub flashing: std::collections::HashSet<isize>,
    pub sounding: std::collections::HashSet<String>,
}

/// A count at the start of a window's title, as "(3) Discord".
fn title_count(t: &str) -> Option<u64> {
    let rest = t.trim_start().strip_prefix('(')?;
    let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if n.is_empty() || !rest[n.len()..].starts_with(')') {
        return None;
    }
    n.parse().ok().filter(|n| *n > 0)
}

/// The desktops' widget: a minimap each, on the map as it is laid out (each at its place, `pos`: column, row), the one shown
/// ringed, what waits marked; opened, the map itself (see the `deskmap` block). Never idle.
pub fn desk_widget(s: &State, places: &BTreeMap<String, (i32, i32)>, act: &Activity) -> Value {
    // (the monitors' box: what every map is drawn in, in proportion)
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (l, t, r, b) in &s.screens {
        x0 = x0.min(*l);
        y0 = y0.min(*t);
        x1 = x1.max(*r);
        y1 = y1.max(*b);
    }
    if s.screens.is_empty() {
        (x0, y0, x1, y1) = (0, 0, 1920, 1080);
    }
    let (bw, bh) = ((x1 - x0).max(1) as f64, (y1 - y0).max(1) as f64);
    let norm = |(l, t, r, b): (i32, i32, i32, i32)| -> Value {
        let (l, t, r, b) = (l.max(x0), t.max(y0), r.min(x1), b.min(y1));
        json!([(l - x0) as f64 / bw, (t - y0) as f64 / bh, ((r - l).max(0)) as f64 / bw, ((b - t).max(0)) as f64 / bh])
    };
    let screens: Vec<Value> = s.screens.iter().map(|r| norm(*r)).collect();
    let n = s.maps.len().max(s.desks).max(1);
    let map = |i: usize| -> Value {
        let m = s.maps.get(i);
        let mut waits = false;
        let wins: Vec<Value> = m
            .map(|m| {
                m.wins
                    .iter()
                    .rev()
                    .map(|w| {
                        let wait = act.flashing.contains(&w.hwnd) || (i != s.desk && title_count(&w.title).is_some());
                        waits |= wait && i != s.desk;
                        json!({ "r": norm(w.rect), "front": w.front, "image": format!("exe:{}", w.exe), "action": format!("deskwin:{i}:{}", w.hwnd), "wait": wait, "sound": act.sounding.contains(&w.exe), "count": title_count(&w.title) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let id = m.map(|m| m.id.clone()).unwrap_or_default();
        let pos = places.get(&id).copied().unwrap_or((i as i32, 0));
        // (what waits there: its app's picture, for its pin)
        let pin = m.and_then(|m| m.wins.iter().find(|w| act.flashing.contains(&w.hwnd) || title_count(&w.title).is_some())).map(|w| format!("exe:{}", w.exe)).unwrap_or_default();
        json!({
            "id": id,
            "pos": [pos.0, pos.1],
            "on": i == s.desk,
            "waits": waits,
            "pin": pin,
            "name": m.map_or_else(|| format!("Desktop {}", i + 1), |m| m.name.clone()),
            "count": m.map_or(0, |m| m.wins.len()),
            "wall": m.map(|m| m.wall.clone()).filter(|w| !w.is_empty()).map(|w| format!("wall:{w}")).unwrap_or_default(),
            "wins": wins,
            "action": format!("desk:{i}"),
            "close": if n > 1 { format!("deskclose:{i}") } else { String::new() },
        })
    };
    let maps: Vec<Value> = (0..n).map(map).collect();
    let aspect = (bw / bh).clamp(1.0, 4.0);
    let waiting = maps.iter().any(|m| m["waits"] == true);
    let mut out = json!({
        "peek": { "desks": { "n": n, "at": s.desk, "maps": maps, "screens": screens, "aspect": aspect }, "label": format!("Desktop {}", s.desk + 1) },
        "panel": { "width": 520, "body": [{ "kind": "deskmap", "items": maps, "screens": screens, "aspect": aspect, "at": s.desk, "res": [bw, bh] }] },
        "tip": format!("{} of {}", s.maps.get(s.desk).map_or_else(|| format!("Desktop {}", s.desk + 1), |m| m.name.clone()), n),
    });
    // (something waits on another desktop: it glows in the theme's second colour)
    if waiting {
        out["glow"] = json!({ "style": "breathe", "second": true });
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
    // the drives: each letter in a ring as full as it; open one (pressed), its menu (right-click): open, a terminal, properties, eject
    let drives = if s.drives.is_empty() {
        idle("usb")
    } else {
        let used = |d: &Drive| (1.0 - d.free as f64 / d.total.max(1) as f64).clamp(0.0, 1.0);
        let ejects: std::collections::HashMap<char, (bool, std::time::Instant, Option<Drive>)> = EJECTS.lock().unwrap().clone().unwrap_or_default();
        let recent = |l: char| ejects.get(&l).filter(|(_, t, _)| t.elapsed().as_secs_f32() < 4.0).map(|(ok, _, _)| *ok);
        let peak = |v: &[f32]| v.iter().cloned().fold(BUSY * 4.0, f32::max);
        let item = |d: &Drive| {
            let busy = d.rs.max(d.ws) >= BUSY;
            json!({ "letter": d.letter.to_string(), "part": used(d), "red": used(d) > 0.9, "busy": busy })
        };
        let mut body = Vec::new();
        for d in &s.drives {
            let u = used(d);
            let top = peak(&d.reads).max(peak(&d.writes));
            let busy = d.rs.max(d.ws) >= BUSY;
            let rate = if !busy { String::new() } else if d.ws >= d.rs { format!("↑ {}", rate_text(d.ws)) } else { format!("↓ {}", rate_text(d.rs)) };
            let mut menu = vec![
                json!({ "icon": "folder", "label": "Open", "action": format!("opendrive:{}", d.letter) }),
                json!({ "icon": "term", "label": "Open in Terminal", "action": format!("driveterm:{}", d.letter) }),
                json!({ "icon": "info", "label": "Properties", "action": format!("driveprops:{}", d.letter) }),
            ];
            if d.usb {
                menu.push(json!({ "icon": "eject", "label": "Eject", "action": format!("eject:{}", d.letter), "hold": true }));
            }
            body.push(json!({
                "kind": "drive", "letter": d.letter.to_string(), "name": d.label, "free": size_text(d.free), "total": size_text(d.total),
                "part": u, "red": u > 0.9, "busy": busy, "rate": rate, "write": d.ws >= d.rs,
                "reads": d.reads.iter().map(|v| v / top).collect::<Vec<_>>(), "writes": d.writes.iter().map(|v| v / top).collect::<Vec<_>>(),
                "state": match recent(d.letter) { Some(false) => "refused", _ => "" },
                "action": format!("opendrive:{}", d.letter), "menu": menu,
            }));
        }
        // (one just ejected: a moment more, green, with a check)
        for (l, (ok, t, was)) in &ejects {
            if *ok && t.elapsed().as_secs_f32() < 3.0 && !s.drives.iter().any(|d| d.letter == *l) {
                if let Some(d) = was {
                    body.push(json!({ "kind": "drive", "letter": l.to_string(), "name": d.label, "free": size_text(d.free), "total": size_text(d.total), "part": used(d), "state": "ejected" }));
                }
            }
        }
        // (its tip: each drive a line, its letter in its ring; how fast it reads and writes, else the room it has left)
        let tip = json!({ "width": 300, "body": [{ "kind": "drivelines", "rows": s.drives.iter().map(|d| {
            let busy = d.rs.max(d.ws) >= BUSY;
            json!({
                "letter": d.letter.to_string(), "name": d.label, "part": used(d), "red": used(d) > 0.9, "busy": busy,
                "free": size_text(d.free), "total": size_text(d.total),
                "read": if d.rs >= BUSY / 4.0 { format!("↓ {}", rate_text(d.rs)) } else { String::new() },
                "write": if d.ws >= BUSY / 4.0 { format!("↑ {}", rate_text(d.ws)) } else { String::new() },
            })
        }).collect::<Vec<_>>() }] });
        let first = s.drives.iter().find(|d| d.rs.max(d.ws) >= BUSY).or_else(|| s.drives.iter().max_by(|a, b| used(a).total_cmp(&used(b)))).unwrap_or(&s.drives[0]);
        json!({
            "peek": { "drives": { "items": s.drives.iter().map(item).collect::<Vec<_>>(), "first": first.letter.to_string() }, "label": format!("{} ({}:)", first.label, first.letter) },
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
    // the desktops: a minimap each (its monitors, its windows where they are, the one in front lit), the one shown ringed, and one
    // more to make a new one; opened, a card each. Never idle: one alone, a new one is made from here
    let desks = desk_widget(s, &BTreeMap::new(), &Activity::default());
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
