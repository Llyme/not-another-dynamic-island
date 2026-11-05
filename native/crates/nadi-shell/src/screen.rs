//! The screen, focus and the power mode, as widgets of NADI's own (what Windows' Quick Settings and its Win+K and Win+P do).
//!
//! - Screen: each display's brightness (an external one's over DDC/CI, where it answers), night light, HDR where a display has it,
//!   a snip of the screen and casting (Windows' own, Win+Shift+S and Win+K), and the four ways to use a second screen (Win+P).
//! - Focus: a focus session (Windows' own, which silences notifications), its time left; NADI's banners wait while it runs.
//! - Power: the power mode (best efficiency, balanced, best performance), where Windows offers it.
//!
//! What they read is looked at on a thread of their own (a display's brightness is slow to ask); what is asked of them is done there
//! too, in order.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use windows::core::GUID;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::*;

pub const SCREEN: &str = "screen";
pub const FOCUS: &str = "focus";
pub const POWER: &str = "power";

const AMBER: &str = "#e8b04a";

/// One display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Display {
    /// its name, as the display gives it ("DELL U2720Q"), else "Display n"
    pub name: String,
    /// its device name (`\\.\DISPLAY1`)
    pub device: String,
    /// its brightness (0..100), where it can be read and set
    pub brightness: Option<u32>,
    /// HDR: on or off, where the display has it
    pub hdr: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub displays: Vec<Display>,
    /// how the screens are used now (Windows' topology: 1 this one only, 2 duplicated, 4 extended, 8 the second only), and how many
    /// can be (a second screen plugged in: the four ways shown)
    pub topology: u32,
    pub connected: usize,
    /// night light: on or off (none: not read)
    pub night: Option<bool>,
    /// the power mode (0 best efficiency, 1 balanced, 2 best performance; none: Windows offers none here) and energy saver on
    pub mode: Option<u8>,
    pub saver: bool,
    /// a focus session running, and till when (none: no end)
    pub focus: bool,
    pub focus_end: Option<SystemTime>,
}

enum Ask {
    Brightness(String, u32),
    Hdr(bool),
    Night(bool),
    Topology(u32),
    Mode(u8),
    Focus(Option<u32>),
    FocusOff,
}

static NOW: Mutex<Option<State>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
static ASK: OnceLock<Mutex<Sender<Ask>>> = OnceLock::new();
/// the focus session's end, as asked here (Windows does not say it)
static FOCUS_END: Mutex<Option<SystemTime>> = Mutex::new(None);

/// Start looking (once).
pub fn start() {
    ASK.get_or_init(|| {
        let (tx, rx) = channel::<Ask>();
        std::thread::Builder::new()
            .name("nadi-screen".into())
            .spawn(move || {
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                let mut bright_at: Option<Instant> = None;
                let mut bright: Vec<(String, Option<u32>)> = Vec::new();
                loop {
                    // (what was asked, done; then looked at again)
                    let mut asked = false;
                    while let Ok(a) = rx.try_recv() {
                        asked = true;
                        match a {
                            Ask::Brightness(dev, v) => {
                                set_brightness(&dev, v);
                                if let Some(b) = bright.iter_mut().find(|(d, _)| *d == dev) {
                                    b.1 = Some(v);
                                }
                            }
                            Ask::Hdr(on) => set_hdr(on),
                            Ask::Night(on) => set_night(on),
                            Ask::Topology(t) => set_topology(t),
                            Ask::Mode(m) => set_mode(m),
                            Ask::Focus(min) => focus_start(min),
                            Ask::FocusOff => focus_stop(),
                        }
                    }
                    // (a display's brightness: slow to ask, so every quarter minute)
                    if bright_at.is_none_or(|t| t.elapsed() > Duration::from_secs(15)) {
                        bright_at = Some(Instant::now());
                        bright = crate::overlay::monitors().iter().map(|m| (device_of(&m.device), brightness_of(m.handle))).collect();
                    }
                    let s = look(&bright);
                    let busy = s.focus;
                    let mut now = NOW.lock().unwrap();
                    if now.as_ref() != Some(&s) {
                        *now = Some(s);
                        DIRTY.store(true, Ordering::Relaxed);
                    }
                    drop(now);
                    let wait = if asked { Duration::from_millis(300) } else if busy { Duration::from_secs(1) } else { Duration::from_secs(3) };
                    if let Ok(a) = rx.recv_timeout(wait) {
                        // (asked while waiting: done at the top, at once)
                        let _ = asked;
                        match a {
                            Ask::Brightness(dev, v) => {
                                set_brightness(&dev, v);
                                if let Some(b) = bright.iter_mut().find(|(d, _)| *d == dev) {
                                    b.1 = Some(v);
                                }
                            }
                            Ask::Hdr(on) => set_hdr(on),
                            Ask::Night(on) => set_night(on),
                            Ask::Topology(t) => set_topology(t),
                            Ask::Mode(m) => set_mode(m),
                            Ask::Focus(min) => focus_start(min),
                            Ask::FocusOff => focus_stop(),
                        }
                    }
                }
            })
            .ok();
        Mutex::new(tx)
    });
}

fn ask(a: Ask) {
    start();
    if let Some(tx) = ASK.get() {
        let _ = tx.lock().unwrap().send(a);
    }
}

/// What it saw, when that changed since last asked.
pub fn take() -> Option<State> {
    if !DIRTY.swap(false, Ordering::Relaxed) {
        return None;
    }
    NOW.lock().unwrap().clone()
}

fn device_of(d: &[u16]) -> String {
    let n = d.iter().position(|c| *c == 0).unwrap_or(d.len());
    String::from_utf16_lossy(&d[..n])
}

fn look(bright: &[(String, Option<u32>)]) -> State {
    let (displays, topology, connected) = displays(bright);
    let (focus, focus_end) = focus_now();
    let (mode, saver) = mode_now();
    State { displays, topology, connected, night: night_now(), mode, saver, focus, focus_end }
}

// ---------------------------------------------------------------------------------------------------------------- displays

/// Each display (its name, its brightness, its HDR), how the screens are used now, and how many are plugged in.
fn displays(bright: &[(String, Option<u32>)]) -> (Vec<Display>, u32, usize) {
    let mut out: Vec<Display> = Vec::new();
    let mut topology = 0u32;
    let mut connected = 0usize;
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_ok() {
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).is_ok() {
                for p in &paths[..np as usize] {
                    let device = source_name(p);
                    let name = target_name(p).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Display {}", out.len() + 1));
                    let hdr = hdr_of(p);
                    let brightness = bright.iter().find(|(d, _)| *d == device).and_then(|(_, b)| *b);
                    if !out.iter().any(|d| d.device == device) {
                        out.push(Display { name, device, brightness, hdr });
                    }
                }
            }
        }
        // (how they are used, as Windows keeps it)
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_DATABASE_CURRENT, &mut np, &mut nm).is_ok() {
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            let mut t = DISPLAYCONFIG_TOPOLOGY_ID::default();
            if QueryDisplayConfig(QDC_DATABASE_CURRENT, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), Some(&mut t)).is_ok() {
                topology = t.0 as u32;
            }
        }
        // (plugged in: every target that is there, used or not)
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ALL_PATHS, &mut np, &mut nm).is_ok() {
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            if QueryDisplayConfig(QDC_ALL_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).is_ok() {
                let mut seen: Vec<(i32, u32, u32)> = Vec::new();
                for p in &paths[..np as usize] {
                    let t = &p.targetInfo;
                    if t.targetAvailable.as_bool() {
                        let id = (t.adapterId.HighPart, t.adapterId.LowPart, t.id);
                        if !seen.contains(&id) {
                            seen.push(id);
                        }
                    }
                }
                connected = seen.len();
            }
        }
    }
    (out, topology, connected)
}

unsafe fn source_name(p: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut n = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    n.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    n.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    n.header.adapterId = p.sourceInfo.adapterId;
    n.header.id = p.sourceInfo.id;
    if DisplayConfigGetDeviceInfo(&mut n.header) == 0 {
        device_of(&n.viewGdiDeviceName)
    } else {
        String::new()
    }
}

unsafe fn target_name(p: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut n = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    n.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    n.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
    n.header.adapterId = p.targetInfo.adapterId;
    n.header.id = p.targetInfo.id;
    (DisplayConfigGetDeviceInfo(&mut n.header) == 0).then(|| device_of(&n.monitorFriendlyDeviceName))
}

/// A display's HDR: on or off, where it has it.
unsafe fn hdr_of(p: &DISPLAYCONFIG_PATH_INFO) -> Option<bool> {
    let mut i = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
    i.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
    i.header.size = std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
    i.header.adapterId = p.targetInfo.adapterId;
    i.header.id = p.targetInfo.id;
    if DisplayConfigGetDeviceInfo(&mut i.header) != 0 {
        return None;
    }
    let v = i.Anonymous.value;
    // (bit 0: it has it; bit 1: on; bit 3: forced off)
    (v & 1 != 0 && v & 8 == 0).then_some(v & 2 != 0)
}

/// HDR on or off, on every display that has it.
fn set_hdr(on: bool) {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_err() {
            return;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).is_err() {
            return;
        }
        for p in &paths[..np as usize] {
            if hdr_of(p).is_none() {
                continue;
            }
            let mut s = DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE::default();
            s.header.r#type = DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE;
            s.header.size = std::mem::size_of::<DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE>() as u32;
            s.header.adapterId = p.targetInfo.adapterId;
            s.header.id = p.targetInfo.id;
            s.Anonymous.value = on as u32;
            let _ = DisplayConfigSetDeviceInfo(&s.header);
        }
    }
}

/// How the screens are used: this one only (1), duplicated (2), extended (4), the second only (8).
fn set_topology(t: u32) {
    let flag = match t {
        1 => SDC_TOPOLOGY_INTERNAL,
        2 => SDC_TOPOLOGY_CLONE,
        4 => SDC_TOPOLOGY_EXTEND,
        8 => SDC_TOPOLOGY_EXTERNAL,
        _ => return,
    };
    unsafe {
        let _ = SetDisplayConfig(None, None, SDC_APPLY | flag);
    }
}

/// A display's brightness (0..100) over DDC/CI, where it answers (each of its physical monitors: the first that does).
fn brightness_of(h: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<u32> {
    unsafe {
        let mut n = 0u32;
        GetNumberOfPhysicalMonitorsFromHMONITOR(h, &mut n).ok()?;
        if n == 0 {
            return None;
        }
        let mut mons = vec![PHYSICAL_MONITOR::default(); n as usize];
        GetPhysicalMonitorsFromHMONITOR(h, &mut mons).ok()?;
        let mut out = None;
        for m in &mons {
            let (mut lo, mut cur, mut hi) = (0u32, 0u32, 0u32);
            if GetMonitorBrightness(m.hPhysicalMonitor, &mut lo, &mut cur, &mut hi) != 0 && hi > lo {
                out = Some(((cur.saturating_sub(lo)) as f32 / (hi - lo) as f32 * 100.0).round() as u32);
                break;
            }
        }
        let _ = DestroyPhysicalMonitors(&mons);
        out
    }
}

fn set_brightness(device: &str, v: u32) {
    let Some(m) = crate::overlay::monitors().into_iter().find(|m| device_of(&m.device) == device) else { return };
    unsafe {
        let mut n = 0u32;
        if GetNumberOfPhysicalMonitorsFromHMONITOR(m.handle, &mut n).is_err() || n == 0 {
            return;
        }
        let mut mons = vec![PHYSICAL_MONITOR::default(); n as usize];
        if GetPhysicalMonitorsFromHMONITOR(m.handle, &mut mons).is_err() {
            return;
        }
        for pm in &mons {
            let (mut lo, mut cur, mut hi) = (0u32, 0u32, 0u32);
            if GetMonitorBrightness(pm.hPhysicalMonitor, &mut lo, &mut cur, &mut hi) != 0 && hi > lo {
                let _ = SetMonitorBrightness(pm.hPhysicalMonitor, lo + (hi - lo) * v.min(100) / 100);
            }
        }
        let _ = DestroyPhysicalMonitors(&mons);
    }
}

// ---------------------------------------------------------------------------------------------------------------- night light

const NIGHT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate";

/// Night light, on or off, as Windows keeps it (none: not found, or not as expected).
fn night_now() -> Option<bool> {
    let data = night_data()?;
    night_of(&data)
}

fn night_data() -> Option<Vec<u8>> {
    use winreg::enums::HKEY_CURRENT_USER;
    let key = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(NIGHT_KEY).ok()?;
    key.get_raw_value("Data").ok().map(|v| v.bytes)
}

/// What Windows' night light record says. Its inner record (its length at 18, then "CB" 1 0) holds 2 1 while on; an older form says
/// it at 18 itself (0x15 on, 0x13 off). Anything else: not known.
fn night_of(data: &[u8]) -> Option<bool> {
    if data.len() > 24 && data[19..23] == [0x43, 0x42, 0x01, 0x00] {
        return Some(data[23] == 0x02 && data[24] == 0x01);
    }
    match data.get(18) {
        Some(0x15) => Some(true),
        Some(0x13) => Some(false),
        _ => None,
    }
}

/// The record changed to say on or off (none: a form not known).
fn night_with(data: &[u8], on: bool) -> Option<Vec<u8>> {
    let now = night_of(data)?;
    let mut d = data.to_vec();
    if now == on {
        return Some(d);
    }
    if data.len() > 24 && data[19..23] == [0x43, 0x42, 0x01, 0x00] {
        if on {
            d.splice(23..23, [0x02u8, 0x01]);
            d[18] = d[18].checked_add(2)?;
        } else {
            d.drain(23..25);
            d[18] = d[18].checked_sub(2)?;
        }
    } else if data.len() >= 25 {
        if on {
            d[18] = 0x15;
            d.splice(23..23, [0x10u8, 0x00]);
        } else {
            d[18] = 0x13;
            if d[23] == 0x10 && d[24] == 0x00 {
                d.drain(23..25);
            }
        }
    } else {
        return None;
    }
    // (its time, a little later: the newer record wins)
    for b in d.iter_mut().take(15).skip(10) {
        if *b != 0xff {
            *b += 1;
            break;
        }
    }
    Some(d)
}

/// Night light on or off, as Windows' own switch does it (its record written again, a newer one: Windows takes it at once). A record
/// not as expected is left alone, and its settings page opened instead.
fn set_night(on: bool) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    let Some(d) = night_data().and_then(|data| night_with(&data, on)) else {
        crate::apps::launch_uri("ms-settings:nightlight");
        return;
    };
    let Ok(key) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(NIGHT_KEY, KEY_READ | KEY_WRITE) else { return };
    let _ = key.set_raw_value("Data", &winreg::RegValue { bytes: d, vtype: winreg::enums::RegType::REG_BINARY });
}

// ---------------------------------------------------------------------------------------------------------------- power mode

const EFFICIENCY: GUID = GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);
const BALANCED: GUID = GUID::zeroed();
const PERFORMANCE: GUID = GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238);

type GetOverlay = unsafe extern "system" fn(*mut GUID) -> u32;
type SetOverlay = unsafe extern "system" fn(GUID) -> u32;

fn powrprof(name: &str) -> Option<unsafe extern "system" fn() -> isize> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let lib = LoadLibraryW(windows::core::w!("powrprof.dll")).ok()?;
        let n = std::ffi::CString::new(name).ok()?;
        GetProcAddress(lib, windows::core::PCSTR(n.as_ptr() as *const u8))
    }
}

/// The power mode now (none: Windows offers none here, as on some plans), and energy saver on.
fn mode_now() -> (Option<u8>, bool) {
    let saver = unsafe {
        let mut s = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        windows::Win32::System::Power::GetSystemPowerStatus(&mut s).is_ok() && s.SystemStatusFlag == 1
    };
    let Some(f) = powrprof("PowerGetEffectiveOverlayScheme") else { return (None, saver) };
    let get: GetOverlay = unsafe { std::mem::transmute(f) };
    let mut g = GUID::zeroed();
    if unsafe { get(&mut g) } != 0 {
        return (None, saver);
    }
    let m = if g == EFFICIENCY {
        0
    } else if g == PERFORMANCE {
        2
    } else {
        1
    };
    (Some(m), saver)
}

fn set_mode(m: u8) {
    let Some(f) = powrprof("PowerSetActiveOverlayScheme") else { return };
    let set: SetOverlay = unsafe { std::mem::transmute(f) };
    let g = match m {
        0 => EFFICIENCY,
        2 => PERFORMANCE,
        _ => BALANCED,
    };
    unsafe {
        let _ = set(g);
    }
}

// ---------------------------------------------------------------------------------------------------------------- focus

/// A focus session: running, and its end (Windows' own, when it lets one be started from here; else NADI's alone).
fn focus_now() -> (bool, Option<SystemTime>) {
    let mut end = FOCUS_END.lock().unwrap();
    // (NADI's own end passed: over)
    if end.is_some_and(|e| e <= SystemTime::now()) {
        *end = None;
    }
    let ours = end.is_some();
    let theirs = windows_focus().is_some_and(|m| m.IsFocusActive().unwrap_or(false));
    (ours || theirs, *end)
}

fn windows_focus() -> Option<windows::UI::Shell::FocusSessionManager> {
    use windows::UI::Shell::FocusSessionManager;
    if !FocusSessionManager::IsSupported().unwrap_or(false) {
        return None;
    }
    FocusSessionManager::GetDefault().ok()
}

/// Start one: so many minutes (none: till it is stopped).
fn focus_start(min: Option<u32>) {
    let end = min.map(|m| SystemTime::now() + Duration::from_secs(m as u64 * 60));
    // (NADI's own: an end far off, for one with none)
    *FOCUS_END.lock().unwrap() = Some(end.unwrap_or_else(|| SystemTime::now() + Duration::from_secs(24 * 3600)));
    if let Some(m) = windows_focus() {
        let r = match end {
            Some(e) => {
                let ticks = e.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos() as i64 / 100).unwrap_or(0) + 116_444_736_000_000_000;
                m.TryStartFocusSession2(windows::Foundation::DateTime { UniversalTime: ticks })
            }
            None => m.TryStartFocusSession(),
        };
        let _ = r;
    }
}

fn focus_stop() {
    *FOCUS_END.lock().unwrap() = None;
    if let Some(m) = windows_focus() {
        let _ = m.DeactivateFocus();
    }
}

/// Is a focus session running (NADI's banners wait)?
pub fn focusing(s: &State) -> bool {
    s.focus
}

// ---------------------------------------------------------------------------------------------------------------- the widgets

fn left_text(end: Option<SystemTime>) -> String {
    let Some(e) = end else { return String::new() };
    let s = e.duration_since(SystemTime::now()).map(|d| d.as_secs()).unwrap_or(0);
    if s >= 3600 * 12 {
        return String::new();
    }
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn widgets(s: &State) -> Vec<(&'static str, Value)> {
    // the screen: the brightness of the one it is on (as a level), its displays' brightness, night light, HDR, cast, the four ways
    let lit: Vec<&Display> = s.displays.iter().filter(|d| d.brightness.is_some()).collect();
    let first = lit.first().and_then(|d| d.brightness).map(|b| b as f64 / 100.0);
    let mut body: Vec<Value> = Vec::new();
    for d in &lit {
        let b = d.brightness.unwrap_or(0);
        body.push(json!({
            "kind": "slider", "icon": "sun", "name": if lit.len() > 1 { d.name.clone() } else { String::new() },
            "value": b as f64 / 100.0, "text": format!("{b}%"), "action": format!("bright:{}", d.device), "wheel": false,
            "range": { "min": 0, "max": 100, "unit": "%", "step": 1 },
        }));
    }
    let mut row: Vec<Value> = Vec::new();
    if let Some(on) = s.night {
        row.push(json!({ "kind": "button", "icon": "moon", "tip": "Night Light", "on": on, "action": "night" }));
    }
    if let Some(on) = s.displays.iter().find_map(|d| d.hdr) {
        row.push(json!({ "kind": "button", "icon": "hdr", "tip": "HDR", "on": on, "action": "hdr" }));
    }
    row.push(json!({ "kind": "space" }));
    row.push(json!({ "kind": "button", "icon": "snip", "tip": "Snip", "action": "snip" }));
    row.push(json!({ "kind": "button", "icon": "cast", "tip": "Cast", "action": "cast" }));
    body.push(json!({ "kind": "row", "items": row }));
    if s.connected >= 2 {
        body.push(json!({ "kind": "rule" }));
        body.push(json!({ "kind": "row", "items": [{ "kind": "space" }, { "kind": "seg", "value": s.topology, "action": "project", "options": [
            { "icon": "proj-pc", "value": 1, "tip": "This Screen Only" },
            { "icon": "proj-dup", "value": 2, "tip": "Duplicate" },
            { "icon": "proj-ext", "value": 4, "tip": "Extend" },
            { "icon": "proj-second", "value": 8, "tip": "Second Screen Only" },
        ] }, { "kind": "space" }] }));
    }
    let screen = json!({
        "peek": if let Some(v) = first { json!({ "icon": "sun", "level": { "value": v } }) } else { json!({ "icon": "sun" }) },
        "panel": { "body": body },
        "wheel": if first.is_some() { json!("brightwheel") } else { Value::Null },
        "tip": first.map(|v| format!("Brightness {}%", (v * 100.0).round())).unwrap_or_else(|| "Screen".into()),
    });

    // focus: a session's time left, amber; started from its panel
    let left = left_text(s.focus_end);
    let focus = json!({
        "peek": if s.focus { json!({ "icon": "bell-off", "color": AMBER, "text": left }) } else { json!({ "icon": "bell" }) },
        "glow": if s.focus { json!(AMBER) } else { Value::Null },
        "panel": { "body": [
            { "kind": "row", "items": [
                { "kind": "button", "icon": "bell-off", "tip": "Do Not Disturb", "on": s.focus, "action": if s.focus { "focusoff" } else { "focus:0" } },
                { "kind": "space" },
                { "kind": "seg", "value": Value::Null, "action": "focusfor", "options": [
                    { "text": "15", "value": 15, "tip": "15 Minutes" },
                    { "text": "25", "value": 25, "tip": "25 Minutes" },
                    { "text": "45", "value": 45, "tip": "45 Minutes" },
                    { "text": "60", "value": 60, "tip": "An Hour" },
                ] },
            ] },
        ] },
        "tip": if s.focus { if left.is_empty() { "Do Not Disturb".to_string() } else { format!("Focus, {left} Left") } } else { "Focus".into() },
    });

    // the power mode: its pictogram; its three ways (energy saver on: a leaf, amber)
    let power = match s.mode {
        None => json!({ "idle": true, "peek": { "icon": "leaf" } }),
        Some(m) => {
            let icon = ["leaf", "scales", "bolt"][m as usize];
            let name = ["Best Efficiency", "Balanced", "Best Performance"][m as usize];
            json!({
                "peek": { "icon": if s.saver { "leaf" } else { icon }, "color": if s.saver { json!(AMBER) } else { Value::Null } },
                "panel": { "body": [{ "kind": "row", "items": [{ "kind": "space" }, { "kind": "seg", "value": m, "action": "pmode", "options": [
                    { "icon": "leaf", "value": 0, "tip": "Best Efficiency" },
                    { "icon": "scales", "value": 1, "tip": "Balanced" },
                    { "icon": "bolt", "value": 2, "tip": "Best Performance" },
                ] }, { "kind": "space" }] }] },
                "tip": name,
            })
        }
    };
    vec![(SCREEN, screen), (FOCUS, focus), (POWER, power)]
}

// ---------------------------------------------------------------------------------------------------------------- the touch keyboard

#[windows::core::interface("37c994e7-432b-4834-a2f7-dce1f13b834b")]
unsafe trait ITipInvocation: windows::core::IUnknown {
    fn toggle(&self, hwnd: HWND) -> windows::core::HRESULT;
}

const TIP_HOST: GUID = GUID::from_u128(0x4ce576fa_83dc_4f88_951c_9d0782b4e376);

/// The touch keyboard shown, or hidden (its program started first when it is not running), on a thread of its own.
pub fn touch_keyboard() {
    std::thread::spawn(|| unsafe {
        use windows::Win32::System::Com::*;
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetDesktopWindow};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if FindWindowW(windows::core::w!("IPTip_Main_Window"), None).is_err() {
            let path = std::env::var("CommonProgramFiles").unwrap_or_else(|_| r"C:\Program Files\Common Files".into()) + r"\microsoft shared\ink\TabTip.exe";
            crate::apps::launch_uri(&path);
            std::thread::sleep(Duration::from_millis(400));
        }
        if let Ok(t) = CoCreateInstance::<_, ITipInvocation>(&TIP_HOST, None, CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER) {
            let _ = t.toggle(GetDesktopWindow());
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------- presses

/// A press on one of them (its action, its args); true: it was theirs.
pub fn press(s: &State, action: &str, args: &Value) -> bool {
    if let Some(dev) = action.strip_prefix("bright:") {
        let v = (args["fraction"].as_f64().unwrap_or(0.0) * 100.0).round().clamp(0.0, 100.0) as u32;
        ask(Ask::Brightness(dev.to_string(), v));
        return true;
    }
    match action {
        "brightwheel" => {
            let up = args["up"] == true;
            for d in s.displays.iter().filter(|d| d.brightness.is_some()) {
                let b = d.brightness.unwrap_or(0) as i32 + if up { 5 } else { -5 };
                ask(Ask::Brightness(d.device.clone(), b.clamp(0, 100) as u32));
            }
        }
        "night" => ask(Ask::Night(!s.night.unwrap_or(false))),
        "hdr" => ask(Ask::Hdr(!s.displays.iter().find_map(|d| d.hdr).unwrap_or(false))),
        "project" => ask(Ask::Topology(args["value"].as_u64().unwrap_or(4) as u32)),
        "cast" => crate::keyhook::chord(&[0x5B, 0x4B]),
        // (Windows' own snip, Win+Shift+S: an area, a window, the whole screen, freehand, or recording, chosen there)
        "snip" => crate::keyhook::chord(&[0x5B, 0x10, 0x53]),
        "pmode" => ask(Ask::Mode(args["value"].as_u64().unwrap_or(1) as u8)),
        "focusfor" => ask(Ask::Focus(Some(args["value"].as_u64().unwrap_or(25) as u32))),
        "focusoff" => ask(Ask::FocusOff),
        a if a.starts_with("focus:") => ask(Ask::Focus(a[6..].parse::<u32>().ok().filter(|m| *m > 0))),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_light_in_its_newer_form() {
        let off: Vec<u8> = [0x43, 0x42, 0x01, 0x00, 0x0a, 0x02, 0x01, 0x00, 0x2a, 0x06, 0x8b, 0x94, 0xdb, 0xc9, 0x06, 0x2a, 0x2b, 0x0e, 0x10, 0x43, 0x42, 0x01, 0x00, 0xc6, 0x14, 0xc1, 0xa5, 0x87, 0x8d, 0x98, 0x88, 0x9a, 0xee, 0x01, 0x00, 0x00, 0x00, 0x00].to_vec();
        assert_eq!(night_of(&off), Some(false));
        let on = night_with(&off, true).unwrap();
        assert_eq!(night_of(&on), Some(true));
        assert_eq!(on.len(), off.len() + 2);
        assert_eq!(on[18], 0x12);
        assert_eq!(on[10], 0x8c, "a newer time");
        let back = night_with(&on, false).unwrap();
        assert_eq!(night_of(&back), Some(false));
        assert_eq!(back.len(), off.len());
    }

    #[test]
    fn night_light_is_read_from_its_byte() {
        let mut d = vec![0u8; 41];
        d[18] = 0x15;
        assert_eq!(night_of(&d), Some(true));
        d[18] = 0x13;
        assert_eq!(night_of(&d), Some(false));
        d[18] = 0x00;
        assert_eq!(night_of(&d), None);
    }

    #[test]
    fn time_left_reads_as_minutes() {
        let end = SystemTime::now() + Duration::from_secs(24 * 60 + 12);
        let t = left_text(Some(end));
        assert!(t == "24:12" || t == "24:11", "{t}");
        assert_eq!(left_text(None), "");
        assert_eq!(left_text(Some(SystemTime::now() + Duration::from_secs(20 * 3600))), "");
    }

    /// (on this PC: what it reads; changes nothing)
    #[test]
    #[ignore]
    fn reads_this_pc() {
        let s = look(&crate::overlay::monitors().iter().map(|m| (device_of(&m.device), brightness_of(m.handle))).collect::<Vec<_>>());
        println!("{s:#?}");
    }
}
