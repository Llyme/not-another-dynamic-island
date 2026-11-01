//! The launcher: what Start was. It pops up in the middle of the screen (springing a little past its size, and back), takes the
//! keyboard, and finds an app, an open window, a setting or a recent file. Before anything is typed it shows the PC's own buttons,
//! round: the radios and the like, its tools, and its power (restart and shut down are held until their ring fills); each says what
//! it is in a tip. As one types, a single list, the open windows first (switching beats starting again). Enter opens the one picked; the arrows move between them; Esc, Win or a click elsewhere puts it
//! away. A window of its own, since it takes the focus; drawn like the rest of the frame.

use crate::apps::{self, App};
use serde_json::Value;
pub use crate::config::AppPin;
use crate::draw::{self, Painter, FACE};
use crate::gfx::Gfx;
use crate::paint::{fill_shape, Weight};
use crate::shape::{self, Rect};
use crate::winlist;
use nadi_core::pc::Pc;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// its width, and the most it is tall (logical px)
const W: f32 = 460.0;
const H_MAX: f32 = 600.0;
/// room round it in its window: it springs a little past its size
const M: f32 = 20.0;
const PAD: f32 = 20.0;
const FIELD: f32 = 42.0;
const GAP: f32 = 18.0;
const ROW: f32 = 44.0;
const HOLD: f32 = 1.2;
const RADIUS: f32 = 28.0;

/// One thing it can open: an app, a window that is open already, a recent file.
#[derive(Clone, Debug)]
enum Hit {
    App(App),
    Window(isize, String, String),
    File(String, String, SystemTime),
    /// a page of Windows' settings: its name, its address
    Setting(&'static str, &'static str),
    /// one of the PC's own buttons
    Sys(Sys),
    /// a pin (its place among the pins); a folder of pins (its name)
    Pin(usize),
    Folder(String),
    /// every app, A to Z; back to the pins; a letter of A to Z, to jump to
    AllApps,
    Back,
    Letter(char),
    /// the folder's name, at its top: pressed, it is renamed
    Rename,
    /// a pin of the foot's (its place there)
    FootPin(usize),
}

/// What is carried: a pin, one of the foot's, or a whole folder.
#[derive(Clone, Debug, PartialEq)]
enum Carry {
    Pin(usize),
    Foot(usize),
    Folder(String),
}

/// The PC's own buttons, in their order: four a row.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sys {
    Wifi,
    Bluetooth,
    NightLight,
    Focus,
    Desktop,
    Settings,
    Files,
    Tasks,
    Terminal,
    Windows,
    Lock,
    Sleep,
    Restart,
    ShutDown,
}


/// Press keys together (the first held while the rest are pressed), as if typed.
pub fn combo(keys: &[u16]) {
    let mut seq = Vec::new();
    let k = |vk: u16, up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } } };
    for vk in keys {
        seq.push(k(*vk, false));
    }
    for vk in keys.iter().rev() {
        seq.push(k(*vk, true));
    }
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}

impl Sys {
    fn icon(self) -> &'static str {
        match self {
            Sys::Wifi => "wifi",
            Sys::Bluetooth => "bluetooth",
            Sys::NightLight => "contrast",
            Sys::Focus => "focus",
            Sys::Desktop => "desktop",
            Sys::Windows => "grid",
            Sys::Settings => "gear",
            Sys::Files => "folder",
            Sys::Tasks => "wave",
            Sys::Terminal => "term",
            Sys::Lock => "lock",
            Sys::Sleep => "moon",
            Sys::Restart => "refresh",
            Sys::ShutDown => "power",
        }
    }

    /// What its tip says (a radio: whether it is on).
    fn tip(self) -> String {
        let radio = |name: &str, on: Option<bool>| match on {
            Some(true) => format!("Turn Off {name}"),
            Some(false) => format!("Turn On {name}"),
            None => name.to_string(),
        };
        let r = crate::radios::now();
        match self {
            Sys::Wifi => radio("Wi-Fi", r.wifi),
            Sys::Bluetooth => radio("Bluetooth", r.bluetooth),
            Sys::NightLight => "Night Light".into(),
            Sys::Focus => "Focus".into(),
            Sys::Desktop => "Show Desktop".into(),
            Sys::Windows => "All Windows".into(),
            Sys::Settings => "Settings".into(),
            Sys::Files => "File Explorer".into(),
            Sys::Tasks => "Task Manager".into(),
            Sys::Terminal => "Terminal".into(),
            Sys::Lock => "Lock".into(),
            Sys::Sleep => "Sleep".into(),
            Sys::Restart => "Restart".into(),
            Sys::ShutDown => "Shut Down".into(),
        }
    }

    /// On (a radio that is on)?
    fn on(self) -> bool {
        let r = crate::radios::now();
        match self {
            Sys::Wifi => r.wifi == Some(true),
            Sys::Bluetooth => r.bluetooth == Some(true),
            _ => false,
        }
    }

    /// Held until its ring fills, not pressed.
    fn held(self) -> bool {
        matches!(self, Sys::Restart | Sys::ShutDown)
    }

    /// Do it. True when the launcher goes away after.
    fn run(self) -> bool {
        let shell = |what: &str| unsafe {
            windows::Win32::UI::Shell::ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(what), None, None, SW_SHOWNORMAL).0 as isize > 32
        };
        match self {
            Sys::Wifi => {
                crate::radios::set("wifi", !self.on());
                false
            }
            Sys::Bluetooth => {
                crate::radios::set("bluetooth", !self.on());
                false
            }
            Sys::NightLight => {
                apps::launch_uri("ms-settings:nightlight");
                true
            }
            Sys::Focus => {
                apps::launch_uri("ms-settings:quiethours");
                true
            }
            Sys::Desktop => {
                combo(&[VK_LWIN.0, 0x44]);
                true
            }
            Sys::Windows => {
                combo(&[VK_LWIN.0, VK_TAB.0]);
                true
            }
            Sys::Settings => {
                apps::launch_uri("ms-settings:");
                true
            }
            Sys::Files => {
                shell("explorer.exe");
                true
            }
            Sys::Tasks => {
                shell("taskmgr.exe");
                true
            }
            Sys::Terminal => {
                if !shell("wt.exe") {
                    shell("cmd.exe");
                }
                true
            }
            Sys::Lock => {
                crate::builtin::power("lock");
                true
            }
            Sys::Sleep => {
                crate::builtin::power("sleep");
                true
            }
            Sys::Restart => {
                crate::builtin::power("restart");
                true
            }
            Sys::ShutDown => {
                crate::builtin::power("shutdown");
                true
            }
        }
    }
}

/// Windows' settings pages, by the name people look for them by.
const SETTINGS: &[(&str, &str)] = &[
    ("Display", "ms-settings:display"),
    ("Sound", "ms-settings:sound"),
    ("Notifications", "ms-settings:notifications"),
    ("Power and Battery", "ms-settings:powersleep"),
    ("Storage", "ms-settings:storagesense"),
    ("Multitasking", "ms-settings:multitasking"),
    ("Bluetooth and Devices", "ms-settings:bluetooth"),
    ("Printers and Scanners", "ms-settings:printers"),
    ("Mouse", "ms-settings:mousetouchpad"),
    ("Network and Internet", "ms-settings:network"),
    ("Wi-Fi", "ms-settings:network-wifi"),
    ("VPN", "ms-settings:network-vpn"),
    ("Personalization", "ms-settings:personalization"),
    ("Background", "ms-settings:personalization-background"),
    ("Colors", "ms-settings:colors"),
    ("Themes", "ms-settings:themes"),
    ("Lock Screen", "ms-settings:lockscreen"),
    ("Taskbar Settings", "ms-settings:taskbar"),
    ("Installed Apps", "ms-settings:appsfeatures"),
    ("Default Apps", "ms-settings:defaultapps"),
    ("Startup Apps", "ms-settings:startupapps"),
    ("Accounts", "ms-settings:yourinfo"),
    ("Sign-in Options", "ms-settings:signinoptions"),
    ("Date and Time", "ms-settings:dateandtime"),
    ("Language and Region", "ms-settings:regionlanguage"),
    ("Keyboard", "ms-settings:keyboard"),
    ("Gaming", "ms-settings:gaming-gamebar"),
    ("Accessibility", "ms-settings:easeofaccess"),
    ("Privacy and Security", "ms-settings:privacy"),
    ("Windows Update", "ms-settings:windowsupdate"),
    ("Windows Security", "ms-settings:windowsdefender"),
    ("About This PC", "ms-settings:about"),
    ("Night Light", "ms-settings:nightlight"),
    ("Focus", "ms-settings:focus"),
];

/// How a thing is shown: a row, a pin's tile, a ghost button at the foot (round), half a row (what was used lately), a small ghost
/// button (every app, back, a letter).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Look {
    Row,
    Tile,
    Foot,
    Half,
    Small,
}

/// What is drawn but not picked: a heading's pictogram, a letter of A to Z, a folder's name, a line, where the foot is.
#[derive(Clone, Debug)]
enum Deco {
    Icon(&'static str),
    Letter(char),
    Title(String),
    Line,
    /// where a pin can be let go to go to the foot, while one is carried and the foot is empty
    Drop,
}

/// What it shows with nothing typed: the pins (and what was used lately), every app, or a folder of pins.
#[derive(Clone, Debug, PartialEq)]
enum View {
    Home,
    All,
    Folder(String),
}

/// What a right-click on something offers.
#[derive(Clone, Debug)]
enum CtxAct {
    PinIt(AppPin),
    PinFoot(AppPin),
    Unpin(usize),
    Front(usize),
    ToDock(App),
    Location(String),
    Admin(String),
    OutOfFolder(usize),
    Ungroup(String),
    Rename(String),
    UnpinFoot(usize),
    ToFoot(usize),
    FromFoot(usize),
}

#[derive(Clone, Debug)]
struct CtxMenu {
    /// its top left (logical, inside), its rows (pictogram, words, what it does), the one under the mouse
    at: (f32, f32),
    rows: Vec<(&'static str, String, CtxAct)>,
    sel: usize,
}

const CTX_W: f32 = 210.0;
const CTX_ROW: f32 = 30.0;
/// a pin's tile: how tall; how many a row
const TILE_H: f32 = 78.0;
const TILE_COLS: usize = 6;
const FOOT: f32 = 34.0;

/// The foot's pins it starts with: lock, sleep, restart, shut down.
pub fn default_foot() -> Vec<AppPin> {
    [("lock", "Lock"), ("sleep", "Sleep"), ("restart", "Restart"), ("shutdown", "Shut Down")]
        .iter()
        .map(|(id, name)| AppPin { kind: "sys".into(), id: id.to_string(), name: name.to_string(), folder: String::new() })
        .collect()
}

/// The pins it starts with: the PC's own tools, each one press away.
pub fn default_pins() -> Vec<AppPin> {
    [("settings", "Settings"), ("files", "File Explorer"), ("terminal", "Terminal"), ("tasks", "Task Manager"), ("nightlight", "Night Light"), ("windows", "All Windows")]
        .iter()
        .map(|(id, name)| AppPin { kind: "sys".into(), id: id.to_string(), name: name.to_string(), folder: String::new() })
        .collect()
}

fn sys_of(id: &str) -> Option<Sys> {
    Some(match id {
        "wifi" => Sys::Wifi,
        "bluetooth" => Sys::Bluetooth,
        "nightlight" => Sys::NightLight,
        "focus" => Sys::Focus,
        "desktop" => Sys::Desktop,
        "settings" => Sys::Settings,
        "files" => Sys::Files,
        "tasks" => Sys::Tasks,
        "terminal" => Sys::Terminal,
        "windows" => Sys::Windows,
        "lock" => Sys::Lock,
        "sleep" => Sys::Sleep,
        "restart" => Sys::Restart,
        "shutdown" => Sys::ShutDown,
        _ => return None,
    })
}

/// A pin's picture's name (none: a pictogram instead), and its pictogram.
fn pin_look(p: &AppPin) -> (String, &'static str) {
    match p.kind.as_str() {
        "app" => (app_image(&p.id), "grid"),
        "file" => (format!("shell:{}", p.id), "file"),
        "setting" => (String::new(), "gear"),
        _ => (String::new(), sys_of(&p.id).map_or("dot", |s| s.icon())),
    }
}

/// Open what a pin is.
fn run_pin(p: &AppPin) {
    match p.kind.as_str() {
        "app" => apps::launch(&p.id),
        "setting" => apps::launch_uri(&p.id),
        "file" => shell_open(&p.id, "open"),
        _ => {
            if let Some(s) = sys_of(&p.id) {
                s.run();
            }
        }
    }
}

fn shell_open(what: &str, verb: &str) {
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(None, &HSTRING::from(verb), &HSTRING::from(what), None, None, SW_SHOWNORMAL);
    }
}

/// Held until its ring fills, not pressed (restart, shut down).
fn pin_held(p: &AppPin) -> bool {
    p.kind == "sys" && sys_of(&p.id).is_some_and(|s| s.held())
}

/// The PC's tools, by their names (found as they are typed, and pinned from there).
const TOOLS: &[(Sys, &str, &str)] = &[
    (Sys::Settings, "settings", "Settings"),
    (Sys::Files, "files", "File Explorer"),
    (Sys::Terminal, "terminal", "Terminal"),
    (Sys::Tasks, "tasks", "Task Manager"),
    (Sys::NightLight, "nightlight", "Night Light"),
    (Sys::Windows, "windows", "All Windows"),
    (Sys::Desktop, "desktop", "Show Desktop"),
    (Sys::Focus, "focus", "Focus"),
    (Sys::Lock, "lock", "Lock"),
    (Sys::Sleep, "sleep", "Sleep"),
    (Sys::Restart, "restart", "Restart"),
    (Sys::ShutDown, "shutdown", "Shut Down"),
];

fn tool_of(x: Sys) -> Option<(&'static str, &'static str)> {
    TOOLS.iter().find(|(s, _, _)| *s == x).map(|(_, id, name)| (*id, *name))
}

/// A program's own file (not a store app): it has a folder to show and can be run as administrator.
fn is_program(id: &str) -> bool {
    id.contains(":\\") && !id.contains('!') && std::path::Path::new(id).is_file()
}

pub struct Launcher {
    pub hwnd: HWND,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    surface: IDCompositionSurface,
    s: f32,
    apps: Vec<App>,
    /// the dock's pinned apps (opened to pick one more for it: these are left out)
    pinned: Vec<App>,
    /// its own pins, and, when they changed, to be kept
    pins: Vec<AppPin>,
    foot: Vec<AppPin>,
    pins_out: Option<(Vec<AppPin>, Vec<AppPin>)>,
    /// where the foot's row is (a pin let go below it goes there)
    foot_y: f32,
    recent: Vec<(String, String, SystemTime)>,
    query: String,
    view: View,
    /// what scrolls (the pins, every app): its top and bottom, and the kind of thing in it (cut to it as it is drawn)
    clip: Option<(f32, f32, Look)>,
    /// how far the pins (or every app) are scrolled, and how far they can go
    scroll: f32,
    scroll_max: f32,
    /// every app: where each letter's heading is (before scrolling)
    letters: Vec<(char, f32)>,
    sel: usize,
    /// what it shows now, how, and where (logical px, from the top of its content)
    items: Vec<(Hit, Look, Rect)>,
    deco: Vec<(Deco, Rect)>,
    /// how tall its content is
    h: f32,
    pub shown: bool,
    /// how far it is out (0..1), springing
    pub t: f32,
    vel: f32,
    to: f32,
    /// when it was shown (it is put away if it never got the focus)
    since: Option<Instant>,
    /// where its window goes (physical px: x, y, width, height)
    at: (i32, i32, i32, i32),
    /// opened to pick an app to pin to the dock (Edit Mode): apps only, and the one picked is kept, not opened
    pick: bool,
    picked: Option<App>,
    /// when it was opened (its things pop in, one after another)
    opened: Instant,
    /// the box round the one picked, springing from one to the next (x, y, w, h), and how fast each side moves
    hi: [f32; 4],
    hv: [f32; 4],
    /// a button held (restart, shut down): which, since when
    hold: Option<(usize, Instant)>,
    /// the one picked, since when (its tip comes after a moment), and how far its tip is out (springing)
    sel_at: Instant,
    tip: f32,
    tip_v: f32,
    /// a pin pressed (where), and, once moved, carried (where it is now)
    press: Option<(usize, f32, f32)>,
    drag: Option<(Carry, f32, f32)>,
    ctx: Option<CtxMenu>,
    /// the folder being renamed: what is typed so far
    naming: Option<String>,
    /// the mouse left (or is over nothing): no box round the one picked until the keys or the mouse pick one again
    unlit: bool,
    /// something in it moves by itself (a name gliding, the caret blinking): drawn again every frame while it is out
    alive: bool,
    /// a question to ask before doing it (restart, shut down), for the frame to put in a modal; where it was asked (logical, inside)
    ask: Option<(Value, (f32, f32))>,
    /// the pins' grid: its top left and how wide a tile is (where a tile at a place in it goes)
    grid: (f32, f32, f32),
    /// where each pin and folder is drawn now (top left, and how fast it moves), by what it is: they spring to their places
    anim: std::collections::HashMap<String, [f32; 4]>,
    anim_live: bool,
}

/// While carrying: what it would go onto (a thing's place among them), or into the foot, and the place it would take there.
#[derive(Clone, Copy, Debug)]
struct Aim {
    onto: Option<usize>,
    foot: bool,
    slot: usize,
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_ACTIVATE as isize);
    }
    if let Some(r) = crate::events::window_message(hwnd, msg, wp, lp) {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// (for checking: with `NADI_LAUNCHER_LOG` set, what happens to the launcher is written there)
pub fn log(what: &str) {
    if let Some(p) = std::env::var_os("NADI_LAUNCHER_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100000).unwrap_or(0);
            let _ = writeln!(f, "{t:>6} {what}");
        }
    }
}

/// The last files and folders opened (Windows' Recent folder), newest first.
fn recent_files(max: usize) -> Vec<(String, String, SystemTime)> {
    let Some(dir) = std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Microsoft\\Windows\\Recent")) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, String, SystemTime)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()?.to_str()? != "lnk" {
                return None;
            }
            let name = p.file_stem()?.to_string_lossy().to_string();
            let when = e.metadata().ok()?.modified().ok()?;
            Some((p.to_string_lossy().to_string(), name, when))
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2));
    out.truncate(max);
    out
}

fn ago(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=59 => "Now".into(),
        60..=3599 => format!("{} min", s / 60),
        3600..=86399 => format!("{} h", s / 3600),
        86400..=172799 => "Yesterday".into(),
        _ => format!("{} days", s / 86400),
    }
}

/// An app's picture's name (a program's icon, or a store app's).
fn app_image(id: &str) -> String {
    if id.contains(":\\") && !id.contains('!') {
        format!("exe:{}", id.to_lowercase())
    } else {
        format!("app:{id}")
    }
}

impl Launcher {
    pub fn new(g: &Gfx) -> windows::core::Result<Launcher> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("NadiLauncher"), hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("NadiLauncher"), w!("NADI Launcher"), WS_POPUP, 0, 0, 10, 10, None, None, inst, None)?;
            let target = g.target_for(hwnd)?;
            let visual = g.dcomp.CreateVisual()?;
            let surface = g.surface(1, 1)?;
            visual.SetContent(&surface)?;
            target.SetRoot(&visual)?;
            Ok(Launcher {
                hwnd,
                _target: target,
                visual,
                surface,
                s: 1.0,
                apps: Vec::new(),
                pinned: Vec::new(),
                pins: default_pins(),
                foot: default_foot(),
                pins_out: None,
                foot_y: f32::MAX,
                recent: Vec::new(),
                query: String::new(),
                view: View::Home,
                scroll: 0.0,
                clip: None,
                scroll_max: 0.0,
                letters: Vec::new(),
                sel: 0,
                items: Vec::new(),
                deco: Vec::new(),
                h: 0.0,
                shown: false,
                t: 0.0,
                vel: 0.0,
                to: 0.0,
                since: None,
                at: (0, 0, 0, 0),
                pick: false,
                picked: None,
                opened: Instant::now(),
                hi: [0.0; 4],
                hv: [0.0; 4],
                hold: None,
                sel_at: Instant::now(),
                tip: 0.0,
                tip_v: 0.0,
                press: None,
                drag: None,
                ctx: None,
                naming: None,
                alive: true,
                ask: None,
                unlit: false,
                grid: (PAD, 0.0, (W - 2.0 * PAD) / TILE_COLS as f32),
                anim: std::collections::HashMap::new(),
                anim_live: false,
            })
        }
    }

    /// Ready it on a monitor: in the middle of `area` (physical px), a little above. `pinned`: the dock's pinned apps; `pins`: its own.
    pub fn show(&mut self, g: &Gfx, area: RECT, scale: f32, pinned: Vec<App>, pins: Vec<AppPin>, foot: Vec<AppPin>, pick: bool) {
        self.pick = pick;
        self.picked = None;
        self.apps = apps::cached();
        self.pinned = pinned;
        self.pins = pins;
        self.foot = foot;
        self.pins_out = None;
        self.apps_tidy();
        self.recent = recent_files(40);
        self.s = scale;
        self.query.clear();
        self.view = View::Home;
        self.scroll = 0.0;
        self.ctx = None;
        self.press = None;
        self.drag = None;
        self.naming = None;
        // (nothing lit as it opens: the keys or the mouse light one)
        self.unlit = true;
        self.anim.clear();
        self.search();
        let (pw, ph) = (((W + 2.0 * M) * scale) as i32, ((H_MAX + 2.0 * M) * scale) as i32);
        let cx = (area.left + area.right) / 2;
        let top = area.top + ((area.bottom - area.top) as f32 * 0.14) as i32 - (M * scale) as i32;
        let x = (cx - pw / 2).clamp(area.left, (area.right - pw).max(area.left));
        if let Ok(surface) = g.surface(pw as u32 + 2, ph as u32 + 2) {
            unsafe {
                let _ = self.visual.SetContent(&surface);
            }
            self.surface = surface;
        }
        self.shown = true;
        self.to = 1.0;
        self.t = 0.0;
        self.vel = 0.0;
        self.at = (x, top.max(area.top), pw, ph.min(area.bottom - top.max(area.top)));
        self.opened = Instant::now();
        self.hi = self.target();
        self.hv = [0.0; 4];
        self.hold = None;
        self.sel_at = Instant::now();
        self.tip = 0.0;
        self.tip_v = 0.0;
        crate::radios::refresh();
    }

    /// A question to ask before doing what was pressed (once), and where it was pressed (physical px, on the screen).
    pub fn take_ask(&mut self) -> Option<(Value, (i32, i32))> {
        let (spec, (x, y)) = self.ask.take()?;
        Some((spec, (self.at.0 + ((x + M) * self.s) as i32, self.at.1 + ((y + M) * self.s) as i32)))
    }

    /// Restart or shut down pressed: asked first (counting down), and put away.
    fn ask_held(&mut self, i: usize) {
        let id = match self.items.get(i).map(|(h, _, _)| h) {
            Some(Hit::Sys(Sys::Restart)) => "restart",
            Some(Hit::Sys(Sys::ShutDown)) => "shutdown",
            Some(Hit::Pin(k)) => self.pins.get(*k).map_or("", |p| p.id.as_str()),
            Some(Hit::FootPin(k)) => self.foot.get(*k).map_or("", |p| p.id.as_str()),
            _ => "",
        };
        let (icon, title, now, word) = match id {
            "restart" => ("refresh", "Restart?", "Restart", "Restarting"),
            "shutdown" => ("power", "Shut Down?", "Shut Down", "Shutting down"),
            _ => return,
        };
        let r = self.items.get(i).map(|(_, _, r)| *r).unwrap_or(Rect::new(W / 2.0, self.h / 2.0, 0.0, 0.0));
        let _ = word;
        let spec = serde_json::json!({ "icon": icon, "tone": "red", "title": title, "text": "Apps that are open will close.", "buttons": [{ "label": "Cancel" }, { "label": now, "action": format!("power:{id}"), "tone": "red" }] });
        self.ask = Some((spec, (r.x + r.w / 2.0, r.y + r.h / 2.0)));
        self.hide();
    }

    /// The PC's tools pinned as tools that are apps too (File Explorer, Terminal, Task Manager, Settings): the apps themselves; Night
    /// Light and Focus: their pages of Settings. A pin there twice: once. True when they changed.
    fn tidy_pins(&mut self) -> bool {
        if self.apps.is_empty() {
            return false;
        }
        let page = |id: &str| match id {
            "nightlight" => Some(("ms-settings:nightlight", "Night Light")),
            "focus" => Some(("ms-settings:focus", "Focus")),
            _ => None,
        };
        let mut changed = false;
        for list in [&mut self.pins, &mut self.foot] {
            for p in list.iter_mut().filter(|p| p.kind == "sys") {
                if let Some(a) = self.apps.iter().find(|a| a.name.eq_ignore_ascii_case(&p.name)) {
                    p.kind = "app".into();
                    p.id = a.id.clone();
                    changed = true;
                } else if let Some((uri, name)) = page(&p.id) {
                    p.kind = "setting".into();
                    p.id = uri.into();
                    p.name = name.into();
                    changed = true;
                }
            }
            let mut seen: Vec<(String, String)> = Vec::new();
            let before = list.len();
            list.retain(|p| {
                let k = (p.kind.clone(), p.id.to_lowercase());
                if seen.contains(&k) {
                    false
                } else {
                    seen.push(k);
                    true
                }
            });
            changed |= list.len() != before;
        }
        if changed {
            self.pins_out = Some((self.pins.clone(), self.foot.clone()));
        }
        changed
    }

    fn apps_tidy(&mut self) {
        let _ = self.tidy_pins();
    }

    /// Its pins and the foot's, when they changed (once): to be kept.
    pub fn take_pins(&mut self) -> Option<(Vec<AppPin>, Vec<AppPin>)> {
        self.pins_out.take()
    }

    /// Is the `i`th thing held until its ring fills, not pressed?
    fn held(&self, i: usize) -> bool {
        match self.items.get(i).map(|(h, _, _)| h) {
            Some(Hit::Sys(x)) => x.held(),
            Some(Hit::Pin(k)) => self.pins.get(*k).is_some_and(pin_held),
            Some(Hit::FootPin(k)) => self.foot.get(*k).is_some_and(pin_held),
            _ => false,
        }
    }

    /// Whether the one picked shows its tip now (one of the foot's, picked a moment ago).
    fn tip_wanted(&self) -> bool {
        self.shown && self.ctx.is_none() && self.drag.is_none() && !self.unlit && matches!(self.items.get(self.sel), Some((Hit::Sys(_) | Hit::FootPin(_), _, _))) && self.sel_at.elapsed().as_secs_f32() > 0.4 && self.opened.elapsed().as_secs_f32() > 0.5
    }

    /// Pick another (its tip starts again).
    fn pick_item(&mut self, i: usize) {
        if i != self.sel {
            self.sel = i;
            self.sel_at = Instant::now();
            self.tip = 0.0;
            self.tip_v = 0.0;
            self.hold = None;
        }
    }

    /// The box round the one picked, where it should be.
    fn target(&self) -> [f32; 4] {
        self.items.get(self.sel).map_or(self.hi, |(_, _, r)| [r.x, r.y, r.w, r.h])
    }

    /// Something in it moves (it springs out or away, its things pop in, the box round the one picked slides).
    pub fn moving(&self) -> bool {
        let hi_moves = self.hv.iter().any(|v| v.abs() > 0.5) || self.hi.iter().zip(self.target()).any(|(a, b)| (a - b).abs() > 0.3);
        let tip_moves = (self.tip - if self.tip_wanted() { 1.0 } else { 0.0 }).abs() > 0.002 || self.tip_v.abs() > 0.02 || (self.shown && matches!(self.items.get(self.sel), Some((Hit::Sys(_) | Hit::FootPin(_), _, _))) && self.sel_at.elapsed().as_secs_f32() < 0.6);
        (self.t != self.to || self.vel != 0.0) || (self.shown && self.opened.elapsed().as_secs_f32() < 0.9) || (self.shown && hi_moves) || (self.shown && tip_moves) || self.hold.is_some() || self.drag.is_some() || self.anim_live || (self.shown && self.alive)
    }

    /// Put it on the screen, once its first frame is drawn (shown before that, its surface would flash as it was last left), and
    /// take the keyboard.
    pub fn reveal(&mut self) {
        log("reveal");
        let (x, y, w, h) = self.at;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
            // (Windows lets only the app in front take the focus: joined to its input for a moment, NADI may)
            let fg = GetForegroundWindow();
            let theirs = GetWindowThreadProcessId(fg, None);
            let mine = windows::Win32::System::Threading::GetCurrentThreadId();
            let joined = theirs != 0 && theirs != mine && windows::Win32::System::Threading::AttachThreadInput(mine, theirs, true).as_bool();
            let _ = BringWindowToTop(self.hwnd);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(self.hwnd);
            if joined {
                let _ = windows::Win32::System::Threading::AttachThreadInput(mine, theirs, false);
            }
        }
        self.since = Some(Instant::now());
        self.clip();
    }

    /// Back on top of the frame's windows (they are put on top now and then).
    pub fn raise(&self) {
        if self.shown {
            unsafe {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    pub fn hide(&mut self) {
        log("hide");
        self.to = 0.0;
        self.shown = false;
        self.since = None;
        self.ctx = None;
        self.drag = None;
        self.press = None;
    }

    /// The window takes clicks only where it is drawn (elsewhere, they go to what is under it).
    fn clip(&self) {
        let s = self.s;
        unsafe {
            let rgn = CreateRectRgn(0, 0, ((W + 2.0 * M) * s).ceil() as i32, ((self.h + 2.0 * M) * s).ceil() as i32);
            let _ = SetWindowRgn(self.hwnd, rgn, true);
        }
    }

    /// While it is out: the apps, once they are found (the first time they are not yet), and put away when another window took the
    /// focus without it hearing (or it never got it). True when it changed.
    pub fn check(&mut self) -> bool {
        if !self.shown {
            return false;
        }
        if self.apps.is_empty() {
            let list = apps::cached();
            if !list.is_empty() {
                self.apps = list;
                self.apps_tidy();
                self.search();
                return true;
            }
        }
        if self.since.is_some_and(|t| t.elapsed().as_millis() > 600) && unsafe { GetForegroundWindow() } != self.hwnd {
            self.hide();
            return true;
        }
        false
    }

    /// One frame of its spring; true while it moves.
    pub fn step(&mut self, dt: f32) -> bool {
        let places_move = self.step_places(dt);
        if !self.moving() && !places_move {
            return false;
        }
        let n = (dt.min(0.1) / (1.0 / 240.0)).ceil().max(1.0) as usize;
        let h = dt.min(0.1) / n as f32;
        let to = self.target();
        for _ in 0..n {
            let a = -520.0 * (self.t - self.to) - 24.0 * self.vel;
            self.vel += a * h;
            self.t += self.vel * h;
            for i in 0..4 {
                let a = -700.0 * (self.hi[i] - to[i]) - 38.0 * self.hv[i];
                self.hv[i] += a * h;
                self.hi[i] += self.hv[i] * h;
            }
        }
        if self.hv.iter().all(|v| v.abs() < 0.5) && self.hi.iter().zip(to).all(|(a, b)| (a - b).abs() < 0.3) {
            self.hi = to;
            self.hv = [0.0; 4];
        }
        // (the tip of the one picked)
        let tip_to = if self.tip_wanted() { 1.0 } else { 0.0 };
        for _ in 0..n {
            let a = -650.0 * (self.tip - tip_to) - 30.0 * self.tip_v;
            self.tip_v += a * h;
            self.tip += self.tip_v * h;
        }
        if (self.tip - tip_to).abs() < 0.002 && self.tip_v.abs() < 0.02 {
            self.tip = tip_to;
            self.tip_v = 0.0;
        }
        // (held long enough: done)
        if let Some((i, at)) = self.hold {
            if at.elapsed().as_secs_f32() >= HOLD {
                self.hold = None;
                match self.items.get(i).map(|(h, _, _)| h.clone()) {
                    Some(Hit::Sys(x)) => {
                        if x.run() {
                            self.hide();
                        }
                    }
                    Some(Hit::Pin(k)) => {
                        if let Some(p) = self.pins.get(k).cloned() {
                            run_pin(&p);
                            self.hide();
                        }
                    }
                    Some(Hit::FootPin(k)) => {
                        if let Some(p) = self.foot.get(k).cloned() {
                            run_pin(&p);
                            self.hide();
                        }
                    }
                    _ => {}
                }
            }
        }
        if (self.t - self.to).abs() < 0.002 && self.vel.abs() < 0.02 {
            self.t = self.to;
            self.vel = 0.0;
            if self.to == 0.0 {
                log("hidden (SW_HIDE)");
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
        }
        true
    }

    /// The pins as they stand at the top: each loose pin, and each folder once (where its first pin is).
    fn entries(&self) -> Vec<Hit> {
        let mut out = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for (i, p) in self.pins.iter().enumerate() {
            if p.folder.is_empty() {
                out.push(Hit::Pin(i));
            } else if !seen.contains(&p.folder.as_str()) {
                seen.push(&p.folder);
                out.push(Hit::Folder(p.folder.clone()));
            }
        }
        out
    }

    /// Is this app (or file, or page) pinned? Where.
    fn pinned_at(&self, kind: &str, id: &str) -> Option<usize> {
        self.pins.iter().position(|p| p.kind == kind && p.id.eq_ignore_ascii_case(id))
    }

    /// What it shows for what is typed (or, with nothing typed, its pins, every app, or a folder), laid out.
    fn search(&mut self) {
        let q = self.query.trim().to_string();
        let inner = W - 2.0 * PAD;
        let mut out: Vec<(Hit, Look, Rect)> = Vec::new();
        let mut deco: Vec<(Deco, Rect)> = Vec::new();
        let mut y = PAD + FIELD + GAP;
        self.letters.clear();
        self.scroll_max = 0.0;
        self.foot_y = f32::MAX;
        self.clip = None;
        if self.pick || !q.is_empty() {
            let mut found: Vec<(u32, Hit)> = Vec::new();
            if self.pick {
                // (picking an app to pin to the dock: the apps alone, the ones not pinned yet)
                for a in self.apps.iter().filter(|a| !self.pinned.iter().any(|p| p.id.eq_ignore_ascii_case(&a.id))) {
                    let sc = if q.is_empty() { 1 } else { apps::score(&a.name, &q) };
                    if sc > 0 {
                        found.push((sc, Hit::App(a.clone())));
                    }
                }
            } else {
                for w in winlist::app_windows() {
                    let sc = apps::score(&w.title, &q).max(apps::score(&winlist::app_name(&w.exe), &q));
                    if sc > 0 {
                        found.push((sc + 5, Hit::Window(w.hwnd, w.title, w.exe)));
                    }
                }
                for a in &self.apps {
                    let sc = apps::score(&a.name, &q);
                    if sc > 0 {
                        found.push((sc, Hit::App(a.clone())));
                    }
                }
                for (path, name, when) in &self.recent {
                    let sc = apps::score(name, &q);
                    if sc > 0 {
                        found.push((sc.saturating_sub(1), Hit::File(path.clone(), name.clone(), *when)));
                    }
                }
                for (name, uri) in SETTINGS {
                    let sc = apps::score(name, &q);
                    if sc > 0 {
                        found.push((sc.saturating_sub(1), Hit::Setting(name, uri)));
                    }
                }
                for (x, _, name) in TOOLS {
                    // (one that is an app too, or a page of Settings: found as that, not twice)
                    if self.apps.iter().any(|a| a.name.eq_ignore_ascii_case(name)) || SETTINGS.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                        continue;
                    }
                    let sc = apps::score(name, &q);
                    if sc > 0 {
                        found.push((sc, Hit::Sys(*x)));
                    }
                }
            }
            if !q.is_empty() {
                found.sort_by(|a, b| b.0.cmp(&a.0));
            }
            for (_, h) in found.into_iter().take(9) {
                out.push((h, Look::Row, Rect::new(PAD, y, inner, ROW)));
                y += ROW;
            }
            if out.is_empty() {
                y += 24.0;
            }
        } else {
            match self.view.clone() {
                View::Home => {
                    // (every app: a ghost button in the field, at its right; the pins under it)
                    out.push((Hit::AllApps, Look::Small, Rect::new(PAD + inner - 36.0, PAD + (FIELD - 30.0) / 2.0, 30.0, 30.0)));
                    y -= 4.0;
                    // (the pins: as many rows as they need, the launcher growing with them; past its tallest, they scroll)
                    let entries = self.entries();
                    let cw = inner / TILE_COLS as f32;
                    let rows = entries.len().div_ceil(TILE_COLS).max(1);
                    let full = rows as f32 * TILE_H;
                    let recent_h = if self.recent.is_empty() { 0.0 } else { 38.0 + self.recent.len().min(4).div_ceil(2) as f32 * ROW };
                    let carrying = matches!(self.drag, Some((Carry::Pin(_) | Carry::Foot(_), _, _)));
                    let foot_h = if !self.foot.is_empty() || carrying { 24.0 + FOOT } else { 0.0 };
                    let room = (H_MAX - PAD - y - recent_h - foot_h).max(TILE_H);
                    let vis = full.min(room);
                    self.scroll_max = (full - vis).max(0.0);
                    self.scroll = self.scroll.clamp(0.0, self.scroll_max);
                    self.grid = (PAD, y - self.scroll, cw);
                    for (k, h) in entries.into_iter().enumerate() {
                        let r = self.grid_rect(k);
                        // (only the ones that can be seen, at least half)
                        if r.y + r.h / 2.0 >= y && r.y + r.h / 2.0 <= y + vis {
                            out.push((h, Look::Tile, r));
                        }
                    }
                    self.clip = (self.scroll_max > 0.0).then_some((y, y + vis, Look::Tile));
                    y += vis;
                    // (what was used lately: two a row)
                    let recent: Vec<(String, String, SystemTime)> = self.recent.iter().take(4).cloned().collect();
                    if !recent.is_empty() {
                        y += 12.0;
                        deco.push((Deco::Icon("clock"), Rect::new(PAD + 6.0, y + 4.0, 14.0, 14.0)));
                        y += 26.0;
                        let hw = inner / 2.0;
                        for (k, (path, name, when)) in recent.into_iter().enumerate() {
                            out.push((Hit::File(path, name, when), Look::Half, Rect::new(PAD + (k % 2) as f32 * hw, y + (k / 2) as f32 * ROW, hw, ROW)));
                        }
                        y += (self.recent.len().min(4).div_ceil(2)) as f32 * ROW;
                    }
                    // (the foot: pins of its own, small, in a row in its middle (lock, sleep, restart, shut down to start with); empty, it
                    // shows only while a pin is carried, as somewhere to let it go)
                    if !self.foot.is_empty() || carrying {
                        y += 12.0;
                        deco.push((Deco::Line, Rect::new(PAD, y, inner, 1.0)));
                        y += 12.0;
                        self.foot_y = y;
                        if self.foot.is_empty() {
                            deco.push((Deco::Drop, Rect::new(PAD, y, inner, FOOT)));
                        }
                        let n = self.foot.len() as f32;
                        let mut fx = PAD + (inner - (n * (FOOT + 4.0) - 4.0)) / 2.0;
                        for k in 0..self.foot.len() {
                            out.push((Hit::FootPin(k), Look::Foot, Rect::new(fx, y, FOOT, FOOT)));
                            fx += FOOT + 4.0;
                        }
                        y += FOOT;
                    } else {
                        self.foot_y = f32::MAX;
                    }
                }
                View::Folder(name) => {
                    out.push((Hit::Back, Look::Small, Rect::new(PAD, y - 4.0, 30.0, 30.0)));
                    out.push((Hit::Rename, Look::Small, Rect::new(PAD + 36.0, y - 4.0, inner - 36.0, 30.0)));
                    deco.push((Deco::Title(self.naming.clone().unwrap_or(name.clone())), Rect::new(PAD + 46.0, y, inner - 56.0, 22.0)));
                    y += 34.0;
                    let cw = inner / TILE_COLS as f32;
                    self.grid = (PAD, y, cw);
                    let list: Vec<usize> = self.pins.iter().enumerate().filter(|(_, p)| p.folder == name).map(|(i, _)| i).collect();
                    for (k, i) in list.iter().enumerate() {
                        let (c, r) = (k % TILE_COLS, k / TILE_COLS);
                        out.push((Hit::Pin(*i), Look::Tile, Rect::new(PAD + c as f32 * cw, y + r as f32 * TILE_H, cw, TILE_H)));
                    }
                    y += list.len().div_ceil(TILE_COLS).max(1) as f32 * TILE_H;
                }
                View::All => {
                    // (back, then the letters there are, to jump to)
                    out.push((Hit::Back, Look::Small, Rect::new(PAD, y - 4.0, 30.0, 30.0)));
                    let mut apps_sorted = self.apps.clone();
                    apps_sorted.sort_by_key(|a| a.name.to_lowercase());
                    let first = |a: &App| a.name.chars().next().map(|c| c.to_ascii_uppercase()).filter(|c| c.is_ascii_alphabetic()).unwrap_or('#');
                    let mut letters: Vec<char> = apps_sorted.iter().map(first).collect();
                    letters.dedup();
                    let lw = ((inner - 40.0) / letters.len().max(1) as f32).min(18.0);
                    for (k, c) in letters.iter().enumerate() {
                        out.push((Hit::Letter(*c), Look::Small, Rect::new(PAD + 40.0 + k as f32 * lw, y - 2.0, lw, 26.0)));
                    }
                    y += 34.0;
                    // (every app, under its letter; scrolled)
                    let top = y;
                    let bottom = H_MAX - PAD;
                    let mut ly = top;
                    let mut last = '\0';
                    for a in apps_sorted {
                        let c = first(&a);
                        if c != last {
                            last = c;
                            self.letters.push((c, ly - top));
                            let r = Rect::new(PAD + 8.0, ly - self.scroll, 40.0, 24.0);
                            if r.bottom() > top && r.y < bottom {
                                deco.push((Deco::Letter(c), r));
                            }
                            ly += 24.0;
                        }
                        let r = Rect::new(PAD, ly - self.scroll, inner, ROW);
                        if r.bottom() > top + 4.0 && r.y < bottom - 4.0 {
                            out.push((Hit::App(a), Look::Row, r));
                        }
                        ly += ROW;
                    }
                    self.scroll_max = (ly - top - (bottom - top)).max(0.0);
                    self.clip = Some((top, bottom, Look::Row));
                    y = bottom;
                }
            }
        }
        self.h = (y + PAD).min(H_MAX);
        let keep = self.items.get(self.sel).map(|(h, _, _)| format!("{h:?}"));
        self.items = out;
        self.deco = deco;
        // (the one picked stays picked when it is still there: a page turned, a list scrolled)
        self.sel = keep.and_then(|k| self.items.iter().position(|(h, _, _)| format!("{h:?}") == k)).unwrap_or(0);
        self.sel_at = Instant::now();
        self.hold = None;
        self.anim_live = true;
        if self.shown {
            self.clip();
        }
    }

    /// What a pin or a folder is, the same wherever it is (a pin in the foot or among the pins): what it is drawn by as it moves.
    fn key_of(&self, h: &Hit) -> Option<String> {
        let pin = |p: &AppPin| format!("{}|{}", p.kind, p.id);
        match h {
            Hit::Pin(k) => self.pins.get(*k).map(pin),
            Hit::FootPin(k) => self.foot.get(*k).map(pin),
            Hit::Folder(n) => Some(format!("folder|{n}")),
            _ => None,
        }
    }

    /// Where the carried one is among what is shown.
    fn carried_item(&self, c: &Carry) -> Option<usize> {
        self.items.iter().position(|(h, _, _)| match (c, h) {
            (Carry::Pin(a), Hit::Pin(b)) => a == b,
            (Carry::Foot(a), Hit::FootPin(b)) => a == b,
            (Carry::Folder(a), Hit::Folder(b)) => a == b,
            _ => false,
        })
    }

    /// A place in the pins' grid: its box.
    fn grid_rect(&self, k: usize) -> Rect {
        let (x0, y0, cw) = self.grid;
        Rect::new(x0 + (k % TILE_COLS) as f32 * cw, y0 + (k / TILE_COLS) as f32 * TILE_H, cw, TILE_H)
    }

    /// Carrying `c` at (x, y): what it would go onto, or the place it would take (among the pins, or in the foot).
    fn aim_at(&self, c: &Carry, x: f32, y: f32) -> Aim {
        let carried = self.carried_item(c);
        if self.view == View::Home && !matches!(c, Carry::Folder(_)) && y >= self.foot_y - 10.0 {
            let slot = self.items.iter().enumerate().filter(|(i, (h, _, r))| Some(*i) != carried && matches!(h, Hit::FootPin(_)) && x > r.x + r.w / 2.0).count();
            return Aim { onto: None, foot: true, slot };
        }
        let tiles: Vec<(usize, Rect)> = self.items.iter().enumerate().filter(|(i, (_, l, _))| Some(*i) != carried && *l == Look::Tile).map(|(i, (_, _, r))| (i, *r)).collect();
        let Some((p, (idx, r))) = tiles.iter().enumerate().min_by(|a, b| {
            let d = |r: &Rect| (r.x + r.w / 2.0 - x).powi(2) + (r.y + r.h / 2.0 - y).powi(2);
            d(&(a.1).1).total_cmp(&d(&(b.1).1))
        }) else {
            return Aim { onto: None, foot: false, slot: 0 };
        };
        let near = (x - (r.x + r.w / 2.0)).abs() < r.w * 0.3 && (y - (r.y + r.h / 2.0)).abs() < r.h * 0.3;
        let target = &self.items[*idx].0;
        let can_onto = match (c, target) {
            (Carry::Pin(_) | Carry::Foot(_), Hit::Pin(_)) => self.view == View::Home,
            (Carry::Pin(_) | Carry::Foot(_), Hit::Folder(_)) => true,
            (Carry::Folder(_), Hit::Pin(_) | Hit::Folder(_)) => true,
            _ => false,
        };
        if near && can_onto {
            return Aim { onto: Some(*idx), foot: false, slot: p };
        }
        Aim { onto: None, foot: false, slot: if x < r.x + r.w / 2.0 { p } else { p + 1 } }
    }

    /// Where each thing shown should be now: while one is carried, the others make room where it would go (and close up where it
    /// was).
    fn places(&self) -> Vec<Rect> {
        let mut out: Vec<Rect> = self.items.iter().map(|(_, _, r)| *r).collect();
        let Some((c, x, y)) = &self.drag else { return out };
        let a = self.aim_at(c, *x, *y);
        let carried = self.carried_item(c);
        let gap_grid = !a.foot && a.onto.is_none();
        let mut p = 0;
        for (i, (_, l, _)) in self.items.iter().enumerate() {
            if Some(i) == carried || *l != Look::Tile {
                continue;
            }
            let k = if gap_grid && p >= a.slot { p + 1 } else { p };
            out[i] = self.grid_rect(k);
            p += 1;
        }
        if self.foot_y < f32::MAX {
            let feet: Vec<usize> = self.items.iter().enumerate().filter(|(i, (h, _, _))| Some(*i) != carried && matches!(h, Hit::FootPin(_))).map(|(i, _)| i).collect();
            let n = feet.len() + usize::from(a.foot);
            let inner = W - 2.0 * PAD;
            let x0 = PAD + (inner - (n as f32 * (FOOT + 4.0) - 4.0)) / 2.0;
            for (p, i) in feet.into_iter().enumerate() {
                let k = if a.foot && p >= a.slot { p + 1 } else { p };
                out[i] = Rect::new(x0 + k as f32 * (FOOT + 4.0), self.foot_y, FOOT, FOOT);
            }
        }
        out
    }

    /// One frame of the pins springing to their places; true while they move.
    fn step_places(&mut self, dt: f32) -> bool {
        if !self.anim_live && self.drag.is_none() {
            return false;
        }
        let targets = self.places();
        let keys: Vec<Option<String>> = self.items.iter().map(|(h, _, _)| self.key_of(h)).collect();
        let mut moving = false;
        let n = (dt.min(0.1) / (1.0 / 240.0)).ceil().max(1.0) as usize;
        let h = dt.min(0.1) / n as f32;
        for (i, k) in keys.into_iter().enumerate() {
            let Some(k) = k else { continue };
            let t = targets[i];
            let a = self.anim.entry(k).or_insert([t.x, t.y, 0.0, 0.0]);
            for _ in 0..n {
                let ax = -520.0 * (a[0] - t.x) - 34.0 * a[2];
                let ay = -520.0 * (a[1] - t.y) - 34.0 * a[3];
                a[2] += ax * h;
                a[3] += ay * h;
                a[0] += a[2] * h;
                a[1] += a[3] * h;
            }
            if (a[0] - t.x).abs() < 0.3 && (a[1] - t.y).abs() < 0.3 && a[2].abs() < 1.0 && a[3].abs() < 1.0 {
                *a = [t.x, t.y, 0.0, 0.0];
            } else {
                moving = true;
            }
        }
        self.anim_live = moving;
        moving
    }

    /// Where each thing shown is drawn now: a pin or folder where it has sprung to; the rest in their places.
    fn drawn_places(&self) -> Vec<Rect> {
        let targets = self.places();
        self.items
            .iter()
            .enumerate()
            .map(|(i, (h, _, _))| match self.key_of(h).and_then(|k| self.anim.get(&k)) {
                Some(a) => Rect::new(a[0], a[1], targets[i].w, targets[i].h),
                None => targets[i],
            })
            .collect()
    }

    /// The app picked to pin (once).
    pub fn take_picked(&mut self) -> Option<App> {
        self.picked.take()
    }

    fn pins_changed(&mut self) {
        // (a folder with one pin left: no folder)
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for p in &self.pins {
            if !p.folder.is_empty() {
                *counts.entry(p.folder.clone()).or_default() += 1;
            }
        }
        for p in &mut self.pins {
            if counts.get(&p.folder).is_some_and(|n| *n < 2) {
                p.folder.clear();
            }
        }
        if let View::Folder(f) = &self.view {
            if !self.pins.iter().any(|p| &p.folder == f) {
                self.view = View::Home;
            }
        }
        self.pins_out = Some((self.pins.clone(), self.foot.clone()));
        self.search();
    }

    fn open_selected(&mut self) {
        if self.pick {
            if let Some((Hit::App(a), _, _)) = self.items.get(self.sel) {
                self.picked = Some(a.clone());
                self.hide();
            }
            return;
        }
        match self.items.get(self.sel).map(|(h, _, _)| h.clone()) {
            // (held ones are held, not pressed)
            _ if self.held(self.sel) => return,
            Some(Hit::Sys(x)) => {
                if !x.run() {
                    return;
                }
            }
            Some(Hit::Setting(_, uri)) => apps::launch_uri(uri),
            Some(Hit::App(a)) => apps::launch(&a.id),
            Some(Hit::Window(h, _, _)) => winlist::activate(h),
            Some(Hit::File(path, _, _)) => shell_open(&path, "open"),
            Some(Hit::Pin(i)) => {
                if let Some(p) = self.pins.get(i).cloned() {
                    run_pin(&p);
                }
            }
            Some(Hit::FootPin(i)) => {
                if let Some(p) = self.foot.get(i).cloned() {
                    run_pin(&p);
                }
            }
            Some(Hit::Folder(name)) => {
                self.view = View::Folder(name);
                self.sel = 0;
                self.search();
                return;
            }
            Some(Hit::AllApps) => {
                self.view = View::All;
                self.scroll = 0.0;
                self.sel = 0;
                self.search();
                return;
            }
            Some(Hit::Back) => {
                self.back();
                return;
            }
            Some(Hit::Rename) => {
                if let View::Folder(name) = &self.view {
                    self.naming = Some(name.clone());
                    self.search();
                }
                return;
            }
            Some(Hit::Letter(c)) => {
                if let Some((_, y)) = self.letters.iter().find(|(l, _)| *l == c) {
                    self.scroll = y.min(self.scroll_max);
                    self.search();
                }
                return;
            }
            None => return,
        }
        self.hide();
    }

    /// The folder renamed to what was typed (none typed, or one by that name already: as it was).
    fn finish_naming(&mut self) {
        let (Some(new), View::Folder(old)) = (self.naming.take(), self.view.clone()) else { return };
        let new = new.trim().to_string();
        if !new.is_empty() && new != old && !self.pins.iter().any(|p| p.folder == new) {
            for p in &mut self.pins {
                if p.folder == old {
                    p.folder = new.clone();
                }
            }
            self.view = View::Folder(new);
            self.pins_changed();
        } else {
            self.search();
        }
    }

    /// Back to the pins.
    fn back(&mut self) {
        self.view = View::Home;
        self.scroll = 0.0;
        self.sel = 0;
        self.search();
    }

    /// The arrows: the nearest thing that way.
    fn go(&mut self, dx: f32, dy: f32) {
        let Some((_, _, r)) = self.items.get(self.sel) else { return };
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let best = self
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != self.sel)
            .filter_map(|(i, (_, _, o))| {
                let (ox, oy) = (o.x + o.w / 2.0 - cx, o.y + o.h / 2.0 - cy);
                let along = ox * dx + oy * dy;
                if along <= 1.0 {
                    return None;
                }
                let across = (ox * dy).abs() + (oy * dx).abs();
                Some((i, along + across * 2.0))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = best {
            self.pick_item(i);
        } else if self.view == View::All && dy != 0.0 {
            // (past the last one shown: the list scrolls)
            self.scroll = (self.scroll + dy * ROW).clamp(0.0, self.scroll_max);
            self.search();
        }
    }

    /// What a right-click on the `i`th thing offers.
    fn menu_for(&self, i: usize) -> Vec<(&'static str, String, CtxAct)> {
        let mut rows: Vec<(&'static str, String, CtxAct)> = Vec::new();
        let pin_toggle = |rows: &mut Vec<(&'static str, String, CtxAct)>, kind: &str, id: &str, name: &str| match self.pinned_at(kind, id) {
            Some(k) => rows.push(("unpin", "Unpin".into(), CtxAct::Unpin(k))),
            None => rows.push(("pin", "Pin".into(), CtxAct::PinIt(AppPin { kind: kind.into(), id: id.into(), name: name.into(), folder: String::new() }))),
        };
        let app_more = |rows: &mut Vec<(&'static str, String, CtxAct)>, id: &str, name: &str| {
            if !self.pinned.iter().any(|p| p.id.eq_ignore_ascii_case(id)) {
                rows.push(("island", "Pin to the Dock".into(), CtxAct::ToDock(App { name: name.into(), id: id.into() })));
            }
            if is_program(id) {
                rows.push(("folder", "Open File Location".into(), CtxAct::Location(id.into())));
                rows.push(("shield", "Run as Administrator".into(), CtxAct::Admin(id.into())));
            }
        };
        match self.items.get(i).map(|(h, _, _)| h) {
            Some(Hit::Pin(k)) => {
                let Some(p) = self.pins.get(*k) else { return rows };
                rows.push(("unpin", "Unpin".into(), CtxAct::Unpin(*k)));
                if *k > 0 {
                    rows.push(("up", "Move to the Front".into(), CtxAct::Front(*k)));
                }
                if !p.folder.is_empty() {
                    rows.push(("left", "Take Out of the Folder".into(), CtxAct::OutOfFolder(*k)));
                }
                rows.push(("minus", "Move to the Foot".into(), CtxAct::ToFoot(*k)));
                if p.kind == "app" {
                    app_more(&mut rows, &p.id, &p.name);
                }
            }
            Some(Hit::FootPin(k)) => {
                rows.push(("unpin", "Unpin".into(), CtxAct::UnpinFoot(*k)));
                rows.push(("up", "Move to the Pins".into(), CtxAct::FromFoot(*k)));
            }
            Some(Hit::Folder(name)) => {
                rows.push(("pen", "Rename".into(), CtxAct::Rename(name.clone())));
                rows.push(("grid", "Ungroup".into(), CtxAct::Ungroup(name.clone())));
            }
            Some(Hit::App(a)) => {
                pin_toggle(&mut rows, "app", &a.id, &a.name);
                app_more(&mut rows, &a.id, &a.name);
            }
            Some(Hit::Window(_, title, exe)) if !exe.is_empty() => {
                let name = winlist::app_name(exe);
                pin_toggle(&mut rows, "app", exe, if name.is_empty() { title } else { &name });
            }
            Some(Hit::File(path, name, _)) => pin_toggle(&mut rows, "file", path, name),
            Some(Hit::Setting(name, uri)) => pin_toggle(&mut rows, "setting", uri, name),
            Some(Hit::Sys(x)) => {
                if let Some((id, name)) = tool_of(*x) {
                    pin_toggle(&mut rows, "sys", id, name);
                    match self.foot.iter().position(|p| p.kind == "sys" && p.id == id) {
                        Some(k) => rows.push(("unpin", "Take Off the Foot".into(), CtxAct::UnpinFoot(k))),
                        None => rows.push(("minus", "Pin to the Foot".into(), CtxAct::PinFoot(AppPin { kind: "sys".into(), id: id.into(), name: name.into(), folder: String::new() }))),
                    }
                }
            }
            _ => {}
        }
        rows
    }

    fn run_ctx(&mut self, act: CtxAct) {
        self.ctx = None;
        match act {
            CtxAct::PinIt(p) => {
                self.pins.push(p);
                self.pins_changed();
            }
            CtxAct::PinFoot(p) => {
                self.foot.push(p);
                self.pins_changed();
            }
            CtxAct::Unpin(k) => {
                if k < self.pins.len() {
                    self.pins.remove(k);
                    self.pins_changed();
                }
            }
            CtxAct::Front(k) => {
                if k < self.pins.len() {
                    let mut p = self.pins.remove(k);
                    // (in a folder: the front of the folder)
                    let at = if p.folder.is_empty() { 0 } else { self.pins.iter().position(|x| x.folder == p.folder).unwrap_or(0) };
                    p.folder = self.pins.get(at).filter(|x| !p.folder.is_empty() && x.folder == p.folder).map(|x| x.folder.clone()).unwrap_or(p.folder.clone());
                    self.pins.insert(at, p);
                    self.pins_changed();
                }
            }
            CtxAct::OutOfFolder(k) => {
                if let Some(p) = self.pins.get_mut(k) {
                    p.folder.clear();
                    self.pins_changed();
                }
            }
            CtxAct::UnpinFoot(k) => {
                if k < self.foot.len() {
                    self.foot.remove(k);
                    self.pins_changed();
                }
            }
            CtxAct::ToFoot(k) => {
                if k < self.pins.len() {
                    let mut p = self.pins.remove(k);
                    p.folder.clear();
                    self.foot.push(p);
                    self.pins_changed();
                }
            }
            CtxAct::FromFoot(k) => {
                if k < self.foot.len() {
                    let p = self.foot.remove(k);
                    self.pins.push(p);
                    self.pins_changed();
                }
            }
            CtxAct::Rename(name) => {
                self.view = View::Folder(name.clone());
                self.naming = Some(name);
                self.search();
            }
            CtxAct::Ungroup(name) => {
                for p in &mut self.pins {
                    if p.folder == name {
                        p.folder.clear();
                    }
                }
                self.pins_changed();
            }
            CtxAct::ToDock(a) => {
                self.pinned.push(a.clone());
                self.picked = Some(a);
            }
            CtxAct::Location(path) => {
                nadi_core::win::reveal_file(&path);
                self.hide();
            }
            CtxAct::Admin(path) => {
                shell_open(&path, "runas");
                self.hide();
            }
        }
    }

    /// A new folder's name: "Folder", else "Folder 2", and so on.
    fn new_folder(&self) -> String {
        let mut n = 1;
        loop {
            let nm = if n == 1 { "Folder".to_string() } else { format!("Folder {n}") };
            if !self.pins.iter().any(|p| p.folder == nm) {
                return nm;
            }
            n += 1;
        }
    }

    /// Something carried and let go at (x, y). Below the foot's line: into the foot, where it was let go. Among the pins: onto a
    /// pin, the two make a folder; onto a folder, it goes in (a folder onto a folder: the two become one); between, it moves there.
    fn drop_carry(&mut self, c: Carry, x: f32, y: f32) {
        let aim = self.aim_at(&c, x, y);
        let carried = self.carried_item(&c);
        // (to the foot: a pin, at the place it would take there)
        if aim.foot {
            let p = match &c {
                Carry::Pin(i) if *i < self.pins.len() => self.pins.remove(*i),
                Carry::Foot(i) if *i < self.foot.len() => self.foot.remove(*i),
                _ => return self.search(),
            };
            let mut p = p;
            p.folder.clear();
            self.foot.insert(aim.slot.min(self.foot.len()), p);
            return self.pins_changed();
        }
        // (what it was let go onto, or the one it goes before (or, past the last, after))
        let tiles: Vec<Hit> = self.items.iter().enumerate().filter(|(i, (_, l, _))| Some(*i) != carried && *l == Look::Tile).map(|(_, (h, _, _))| h.clone()).collect();
        let (target, onto, before) = match aim.onto {
            Some(i) => (self.items[i].0.clone(), true, true),
            None if tiles.is_empty() => (Hit::Back, false, true),
            None if aim.slot < tiles.len() => (tiles[aim.slot].clone(), false, true),
            None => (tiles[tiles.len() - 1].clone(), false, false),
        };
        // (from the foot to the pins: it is a pin now, last, and goes on as one)
        let c = match c {
            Carry::Foot(i) if i < self.foot.len() => {
                let p = self.foot.remove(i);
                self.pins.push(p);
                Carry::Pin(self.pins.len() - 1)
            }
            other => other,
        };
        let target = &target;
        // (the place to put things: before or after the one let go on (a folder: its first pin))
        let anchor_of = |pins: &[AppPin], t: &Hit| match t {
            Hit::Pin(j) => Some(*j),
            Hit::Folder(name) => pins.iter().position(|p| &p.folder == name),
            _ => None,
        };
        match (&c, target) {
            (Carry::Pin(i), Hit::Pin(j)) if i == j => {}
            (Carry::Folder(f), Hit::Folder(g)) if f == g => {}
            (Carry::Pin(_), Hit::Back) => {}
            (Carry::Pin(i), Hit::Pin(j)) if onto && self.view == View::Home => {
                let name = self.new_folder();
                let j = *j;
                self.pins[j].folder = name.clone();
                self.pins[*i].folder = name;
            }
            (Carry::Pin(i), Hit::Folder(name)) if onto => {
                self.pins[*i].folder = name.clone();
            }
            (Carry::Folder(f), Hit::Folder(g)) if onto => {
                for p in &mut self.pins {
                    if &p.folder == f {
                        p.folder = g.clone();
                    }
                }
            }
            (Carry::Folder(f), Hit::Pin(j)) if onto => {
                let j = *j;
                self.pins[j].folder = f.clone();
            }
            (Carry::Pin(i), t) => {
                let i = *i;
                let Some(anchor) = anchor_of(&self.pins, t) else { return self.pins_changed() };
                let mut p = self.pins.remove(i);
                if self.view == View::Home {
                    p.folder.clear();
                }
                let anchor = if anchor > i { anchor - 1 } else { anchor };
                let at = if before { anchor } else { anchor + 1 };
                self.pins.insert(at.min(self.pins.len()), p);
            }
            (Carry::Folder(f), t) => {
                // (a folder moved: all its pins together, before or after the one it was let go on)
                let f = f.clone();
                let near: Option<AppPin> = anchor_of(&self.pins, t).and_then(|j| self.pins.get(j).cloned());
                let near_folder = match t {
                    Hit::Folder(g) => Some(g.clone()),
                    _ => None,
                };
                let moving: Vec<AppPin> = self.pins.iter().filter(|p| p.folder == f).cloned().collect();
                self.pins.retain(|p| p.folder != f);
                let at = match (near_folder, near) {
                    (Some(g), _) => {
                        let first = self.pins.iter().position(|p| p.folder == g).unwrap_or(self.pins.len());
                        if before { first } else { self.pins.iter().rposition(|p| p.folder == g).map_or(first, |l| l + 1) }
                    }
                    (None, Some(n)) => {
                        let j = self.pins.iter().position(|p| *p == n).unwrap_or(self.pins.len());
                        if before { j } else { j + 1 }
                    }
                    _ => self.pins.len(),
                };
                let at = at.min(self.pins.len());
                for (k, p) in moving.into_iter().enumerate() {
                    self.pins.insert(at + k, p);
                }
            }
            _ => {}
        }
        self.pins_changed();
    }

    /// What its window hears.
    pub fn message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        log(&format!("msg {msg:#06x} wp={:#x} shown={} t={:.2} fg_is_me={}", wp.0, self.shown, self.t, unsafe { GetForegroundWindow() } == self.hwnd));
        match msg {
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE && self.shown {
                    self.hide();
                }
                None
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                self.unlit = false;
                // (a menu out: the arrows move in it, Enter does it, Esc puts it away)
                if let Some(c) = &mut self.ctx {
                    match vk {
                        VK_ESCAPE => self.ctx = None,
                        VK_DOWN => c.sel = (c.sel + 1) % c.rows.len().max(1),
                        VK_UP => c.sel = (c.sel + c.rows.len().max(1) - 1) % c.rows.len().max(1),
                        VK_RETURN => {
                            let act = c.rows.get(c.sel).map(|r| r.2.clone());
                            if let Some(a) = act {
                                self.run_ctx(a);
                            }
                        }
                        _ => {}
                    }
                    return Some(LRESULT(0));
                }
                // (a folder being named: what is typed goes into its name; Enter keeps it, Esc leaves it as it was)
                if let Some(n) = &mut self.naming {
                    match vk {
                        VK_ESCAPE => {
                            self.naming = None;
                            self.search();
                        }
                        VK_RETURN => self.finish_naming(),
                        VK_BACK => {
                            n.pop();
                            self.search();
                        }
                        _ => {}
                    }
                    return Some(LRESULT(0));
                }
                match vk {
                    VK_ESCAPE => {
                        if !self.query.is_empty() {
                            self.query.clear();
                            self.search();
                        } else if self.view != View::Home {
                            self.back();
                        } else {
                            self.hide();
                        }
                    }
                    VK_RETURN => {
                        // (a held one: held while Enter is)
                        if self.held(self.sel) {
                            self.ask_held(self.sel);
                        } else {
                            self.open_selected();
                        }
                    }
                    VK_DOWN => self.go(0.0, 1.0),
                    VK_UP => self.go(0.0, -1.0),
                    VK_RIGHT => self.go(1.0, 0.0),
                    VK_LEFT => self.go(-1.0, 0.0),
                    VK_BACK => {
                        if self.query.is_empty() && self.view != View::Home {
                            self.back();
                        } else {
                            self.query.pop();
                            self.search();
                        }
                    }
                    VK_APPS => {
                        let (_, _, r) = self.items.get(self.sel).cloned().unwrap_or((Hit::Back, Look::Small, Rect::new(PAD, PAD, 0.0, 0.0)));
                        let rows = self.menu_for(self.sel);
                        if !rows.is_empty() {
                            self.ctx = Some(CtxMenu { at: (r.x + r.w / 2.0, r.y + r.h / 2.0), rows, sel: 0 });
                        }
                    }
                    _ => return None,
                }
                Some(LRESULT(0))
            }
            WM_CHAR => {
                let c = char::from_u32(wp.0 as u32).unwrap_or('\0');
                if let Some(n) = &mut self.naming {
                    if !c.is_control() && n.chars().count() < 30 {
                        n.push(c);
                        self.search();
                    }
                    return Some(LRESULT(0));
                }
                if self.ctx.is_none() && !c.is_control() && self.query.chars().count() < 60 {
                    self.query.push(c);
                    self.search();
                }
                Some(LRESULT(0))
            }
            WM_KEYUP => {
                if VIRTUAL_KEY(wp.0 as u16) == VK_RETURN {
                    self.hold = None;
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
                self.ctx = None;
                if self.query.is_empty() && matches!(self.view, View::All | View::Home) && self.scroll_max > 0.0 {
                    let step = if self.view == View::All { ROW * 1.5 } else { TILE_H * 0.75 };
                    self.scroll = (self.scroll - d * step).clamp(0.0, self.scroll_max);
                    self.search();
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as f32 / self.s - M, ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.s - M);
                if let Some(i) = self.items.iter().position(|(_, _, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()) {
                    self.pick_item(i);
                    let rows = self.menu_for(i);
                    self.ctx = if rows.is_empty() { None } else { Some(CtxMenu { at: (x, y), rows, sel: 0 }) };
                } else {
                    self.ctx = None;
                }
                Some(LRESULT(0))
            }
            // (the mouse left the window)
            0x02A3 => {
                if self.drag.is_none() {
                    self.unlit = true;
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as f32 / self.s - M, ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.s - M);
                // (told when the mouse leaves)
                if msg == WM_MOUSEMOVE {
                    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                }
                // (a menu out: its rows; a press elsewhere puts it away)
                if let Some(c) = &mut self.ctx {
                    let (mx, my) = ctx_origin(c, self.h);
                    let inside = x >= mx && x < mx + CTX_W && y >= my && y < my + c.rows.len() as f32 * CTX_ROW + 10.0;
                    if inside {
                        let row = ((y - my - 5.0) / CTX_ROW).floor().max(0.0) as usize;
                        c.sel = row.min(c.rows.len().saturating_sub(1));
                        if msg == WM_LBUTTONUP {
                            let act = c.rows.get(c.sel).map(|r| r.2.clone());
                            if let Some(a) = act {
                                self.run_ctx(a);
                            }
                        }
                    } else if msg == WM_LBUTTONDOWN {
                        self.ctx = None;
                    }
                    return Some(LRESULT(0));
                }
                // (a folder being named, a press elsewhere: the name kept)
                if self.naming.is_some() && msg == WM_LBUTTONDOWN {
                    let on_name = self.items.iter().any(|(h, _, r)| matches!(h, Hit::Rename) && x >= r.x && x < r.right() && y >= r.y && y < r.bottom());
                    if !on_name {
                        self.finish_naming();
                    }
                }
                // (the field's x, while it holds something)
                let xr = clear_box();
                if !self.query.is_empty() && x >= xr.x && x < xr.right() && y >= xr.y && y < xr.bottom() {
                    if msg == WM_LBUTTONUP {
                        self.query.clear();
                        self.search();
                    }
                    return Some(LRESULT(0));
                }
                // (a pin carried: it follows; let go, it goes where it was let go)
                if let Some((c, _, _)) = self.drag.clone() {
                    if msg == WM_LBUTTONUP {
                        self.drag = None;
                        self.press = None;
                        self.drop_carry(c, x, y);
                    } else {
                        self.drag = Some((c, x, y));
                    }
                    return Some(LRESULT(0));
                }
                if let Some((i, px, py)) = self.press {
                    let down = wp.0 & 0x0001 != 0;
                    if msg == WM_MOUSEMOVE && down && ((x - px).powi(2) + (y - py).powi(2)) > 36.0 {
                        let carry = match self.items.get(i).map(|(h, _, _)| h) {
                            Some(Hit::Pin(k)) => Some(Carry::Pin(*k)),
                            Some(Hit::FootPin(k)) => Some(Carry::Foot(*k)),
                            Some(Hit::Folder(name)) if self.view == View::Home => Some(Carry::Folder(name.clone())),
                            _ => None,
                        };
                        if let Some(c) = carry {
                            self.hold = None;
                            self.drag = Some((c, x, y));
                            // (laid out again: an empty foot shows where it can go)
                            self.search();
                            return Some(LRESULT(0));
                        }
                    }
                    if msg == WM_MOUSEMOVE && !down {
                        self.press = None;
                    }
                }
                let under = self.items.iter().position(|(_, _, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom());
                // (a held one let go of, or left: not done)
                if self.hold.is_some_and(|(i, _)| msg == WM_LBUTTONUP || under != Some(i)) {
                    self.hold = None;
                    return Some(LRESULT(0));
                }
                // (over nothing: nothing lit)
                if under.is_none() && msg == WM_MOUSEMOVE {
                    self.unlit = true;
                }
                if let Some(i) = under {
                    self.unlit = false;
                    self.pick_item(i);
                    let held = self.held(i);
                    if msg == WM_LBUTTONDOWN {
                        // (a held one is held, and can be carried too: moved far enough, its hold is let go of)
                        self.press = Some((i, x, y));
                    } else if msg == WM_LBUTTONUP {
                        self.press = None;
                        if held {
                            self.ask_held(i);
                        } else {
                            self.open_selected();
                        }
                    }
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// Draw it as it is now.
    pub fn render(&mut self, g: &Gfx, pc: &Arc<Pc>) {
        log(&format!("render t={:.3}", self.t));
        let s = self.s;
        let t = self.t.max(0.0);
        let h = self.h.max(2.0);
        let Ok(shp) = shape::floating(&g.d2d_factory, W * s, h * s, RADIUS.min(h / 2.0) * s) else { return };
        let age = self.opened.elapsed().as_secs_f32();
        let p = Painter { g, dc: &g.dc, s, pc, t: age, viz: [0.0; 3] };
        let places = self.drawn_places();
        let onto = self.drag.as_ref().and_then(|(c, x, y)| self.aim_at(c, *x, *y).onto);
        let view = Paint { places: &places, onto, query: &self.query, hi: self.hi, items: &self.items, deco: &self.deco, pins: &self.pins, foot: &self.foot, held: self.hold.map(|(i, at)| (i, (at.elapsed().as_secs_f32() / HOLD).min(1.0))), tip: (self.sel, self.tip), drag: self.drag.clone(), unlit: self.unlit, ctx: self.ctx.as_ref(), clip: self.clip, naming: self.naming.is_some() };
        let _ = g.draw_surface(&self.surface, |dc| {
            paint(&p, dc, &shp, s, t, h, age, &view);
        });
        // (a name gliding, or the caret blinking (always, while it is out): drawn again at the next frame)
        let _ = crate::draw::take_marquee();
        self.alive = true;
    }
}

/// Where a menu stands (inside): at the mouse, kept inside.
fn ctx_origin(c: &CtxMenu, h: f32) -> (f32, f32) {
    let mh = c.rows.len() as f32 * CTX_ROW + 10.0;
    ((c.at.0).clamp(8.0, W - CTX_W - 8.0), (c.at.1).clamp(8.0, (h - mh - 8.0).max(8.0)))
}

/// What is drawn, as it is now.
struct Paint<'a> {
    /// where each thing is drawn now (springing to its place)
    places: &'a [Rect],
    /// the one a carried thing would go onto: ringed
    onto: Option<usize>,
    query: &'a str,
    hi: [f32; 4],
    items: &'a [(Hit, Look, Rect)],
    deco: &'a [(Deco, Rect)],
    pins: &'a [AppPin],
    foot: &'a [AppPin],
    held: Option<(usize, f32)>,
    tip: (usize, f32),
    drag: Option<(Carry, f32, f32)>,
    ctx: Option<&'a CtxMenu>,
    /// what scrolls: its top and bottom, and the kind of thing in it
    clip: Option<(f32, f32, Look)>,
    /// a folder being named: its caret
    naming: bool,
    /// the mouse gone: no box round the one picked
    unlit: bool,
}

/// Draw the launcher: its shape `h` tall, scaled from its middle by how far out it is (`t`, springing past 1), and what is in it,
/// each thing popping in after the one before (`age`: since it opened); the box round the one picked at `hi`.
#[allow(clippy::too_many_arguments)]
fn paint(p: &Painter, dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, shp: &shape::Shape, s: f32, t: f32, h: f32, age: f32, v: &Paint) {
    use windows::Foundation::Numerics::Matrix3x2;
    let k = 0.86 + 0.14 * t;
    let (cx, cy) = ((M + W / 2.0) * s, (M + h / 2.0) * s);
    let mut base = Matrix3x2::default();
    unsafe {
        dc.GetTransform(&mut base);
        let m = Matrix3x2::translation(M * s, M * s) * Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: cx * (1.0 - k), M32: cy * (1.0 - k) } * base;
        dc.SetTransform(&m);
    }
    draw::faded(dc, t.clamp(0.0, 1.0), || {
        let _ = fill_shape(dc, shp, FACE);
        paint_inside(p, age, h, v);
    });
    unsafe { dc.SetTransform(&base) };
    let _ = s;
}

/// How far in a thing is (0..1, springing a little past): the `i`th pops in a moment after the one before.
fn pop(age: f32, i: usize) -> f32 {
    let x = ((age - 0.06 - i as f32 * 0.025) / 0.3).clamp(0.0, 1.0);
    // (back out: past 1, then back)
    let c = 1.9;
    1.0 + (c + 1.0) * (x - 1.0).powi(3) + c * (x - 1.0).powi(2)
}

/// Where the field's x is (inside, as drawn).
fn clear_box() -> Rect {
    Rect::new(PAD + (W - 2.0 * PAD) - 40.0, PAD + FIELD / 2.0 - 14.0, 28.0, 28.0)
}

fn paint_inside(p: &Painter, age: f32, h: f32, v: &Paint) {
    let dc = p.dc;
    let w = W - 2.0 * PAD;
    let query = v.query;
    // the field
    p.round(Rect::new(PAD, PAD, w, FIELD), FIELD / 2.0, draw::FAINT);
    p.icon("search", PAD + 16.0, PAD + 13.0, 16.0, draw::DIM);
    let blink = (age * 1.0).fract() < 0.5;
    if query.is_empty() {
        if blink {
            p.round(Rect::new(PAD + 42.0, PAD + 12.0, 1.5, 18.0), 0.75, draw::accent());
        }
    } else {
        let tw = p.text(query, PAD + 42.0, PAD + FIELD / 2.0, 14.0, Weight::Regular, w - 84.0, draw::TEXT);
        if blink {
            p.round(Rect::new(PAD + 44.0 + tw, PAD + 12.0, 1.5, 18.0), 0.75, draw::accent());
        }
        // (its x: emptied)
        let xr = clear_box();
        p.circle(xr.x + xr.w / 2.0, xr.y + xr.h / 2.0, 11.0, draw::FAINT);
        p.icon("x", xr.x + xr.w / 2.0 - 6.0, xr.y + xr.h / 2.0 - 6.0, 12.0, draw::DIM);
    }
    // (what scrolls (the pins, every app's list): cut to its place)
    let _ = h;
    let (ct, cb, clook) = v.clip.unwrap_or((0.0, 0.0, Look::Small));
    let clip = v.clip.is_some();
    let push_clip = || unsafe {
        dc.PushAxisAlignedClip(&p.px(Rect::new(0.0, ct - 2.0, W, cb - ct + 4.0)).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
    };
    // (the box round the one picked: it slides from one to the next)
    if !v.items.is_empty() && v.ctx.is_none() && !v.unlit && v.drag.is_none() {
        let hi = v.hi;
        // (round round a round one)
        let rad = if (hi[2] - hi[3]).abs() < 1.0 { hi[2] / 2.0 } else { 14.0 };
        let in_list = clip && v.items.get(v.tip.0).is_some_and(|(_, l, _)| *l == clook);
        if in_list {
            push_clip();
        }
        p.round(Rect::new(hi[0], hi[1], hi[2], hi[3]), rad, crate::paint::Color(0.15, 0.15, 0.17, 1.0));
        if in_list {
            unsafe { dc.PopAxisAlignedClip() };
        }
    }
    if v.items.is_empty() {
        p.text("Nothing found", PAD + 4.0, PAD + FIELD + GAP + 12.0, 13.0, Weight::Regular, w, draw::DIM);
    }
    // (what is drawn but not picked)
    for (d, r) in v.deco {
        let in_list = clip && matches!(d, Deco::Letter(_));
        if in_list {
            push_clip();
        }
        match d {
            Deco::Icon(name) => p.icon(name, r.x, r.y, r.w, draw::DIM),
            Deco::Letter(c) => {
                p.text(&c.to_string(), r.x, r.y + r.h / 2.0, 12.0, Weight::Bold, 30.0, draw::accent());
            }
            Deco::Drop => p.dashed(*r, r.h / 2.0, 1.5, draw::DIM.alpha(0.6)),
            Deco::Title(t) => {
                // (being named: a caret at its end; else a pen, to say it can be)
                let tw = p.text(t, r.x, r.y + r.h / 2.0, 13.0, Weight::Semibold, r.w, draw::TEXT);
                if v.naming {
                    if (age * 1.0).fract() < 0.5 {
                        p.round(Rect::new(r.x + tw + 2.0, r.y + 2.0, 1.5, r.h - 4.0), 0.75, draw::accent());
                    }
                } else {
                    p.icon("pen", r.x + tw + 8.0, r.y + r.h / 2.0 - 6.0, 12.0, draw::DIM);
                }
            }
            Deco::Line => p.round(*r, 0.5, crate::paint::Color(0.106, 0.106, 0.125, 1.0)),
        }
        if in_list {
            unsafe { dc.PopAxisAlignedClip() };
        }
    }
    for (i, (hit, look, _)) in v.items.iter().enumerate() {
        let r = &v.places[i];
        // (popping in: from a little smaller, and below, to its place)
        let e = pop(age, i.min(30));
        if e <= 0.001 {
            continue;
        }
        // (the one carried: drawn where the mouse is, after the rest)
        let carried = match (&v.drag, hit) {
            (Some((Carry::Pin(d), _, _)), Hit::Pin(k)) => d == k,
            (Some((Carry::Foot(d), _, _)), Hit::FootPin(k)) => d == k,
            (Some((Carry::Folder(f), _, _)), Hit::Folder(g)) => f == g,
            _ => false,
        };
        if carried {
            continue;
        }
        let (ix, iy) = ((r.x + r.w / 2.0) * p.s, (r.y + r.h / 2.0) * p.s);
        let kk = 0.7 + 0.3 * e;
        let mut before = windows::Foundation::Numerics::Matrix3x2::default();
        let in_list = clip && *look == clook;
        if in_list {
            push_clip();
        }
        unsafe {
            dc.GetTransform(&mut before);
            dc.SetTransform(&(windows::Foundation::Numerics::Matrix3x2 { M11: kk, M12: 0.0, M21: 0.0, M22: kk, M31: ix * (1.0 - kk), M32: iy * (1.0 - kk) + (1.0 - e) * 10.0 * p.s } * before));
        }
        let ring = v.held.filter(|(h, _)| *h == i).map(|(_, f)| f);
        draw::faded(dc, e.clamp(0.0, 1.0), || paint_item(p, hit, *look, r, ring, v.pins, v.foot));
        unsafe { dc.SetTransform(&before) };
        if in_list {
            unsafe { dc.PopAxisAlignedClip() };
        }
    }
    // (the one a carried thing would go onto: ringed in the theme's colour)
    if let Some(r) = v.onto.and_then(|i| v.places.get(i)) {
        p.outline(Rect::new(r.x + 4.0, r.y + 2.0, r.w - 8.0, r.h - 4.0), 14.0, 2.0, draw::accent());
    }
    // (what is carried: under the mouse, on a ground of its own)
    if let Some((c, x, y)) = &v.drag {
        let tw = (W - 2.0 * PAD) / TILE_COLS as f32;
        let r = Rect::new(x - tw / 2.0, y - TILE_H / 2.0, tw, TILE_H);
        let hit = match c {
            Carry::Pin(k) => Hit::Pin(*k),
            Carry::Folder(f) => Hit::Folder(f.clone()),
            Carry::Foot(k) => {
                let fr = Rect::new(x - FOOT / 2.0, y - FOOT / 2.0, FOOT, FOOT);
                p.circle(*x, *y, FOOT / 2.0, crate::paint::Color(0.15, 0.15, 0.17, 0.92));
                paint_item(p, &Hit::FootPin(*k), Look::Foot, &fr, None, v.pins, v.foot);
                Hit::Back
            }
        };
        if !matches!(hit, Hit::Back) {
            p.round(Rect::new(r.x + 4.0, r.y + 2.0, r.w - 8.0, r.h - 4.0), 14.0, crate::paint::Color(0.15, 0.15, 0.17, 0.92));
            paint_item(p, &hit, Look::Tile, &r, None, v.pins, v.foot);
        }
    }
    // (the tip of the one picked: it springs up out of it)
    let (ti, tg) = v.tip;
    if tg > 0.01 {
        let words = match v.items.get(ti) {
            Some((Hit::Sys(x), _, r)) => Some((x.tip(), *r)),
            Some((Hit::FootPin(k), _, r)) => v.foot.get(*k).map(|p| (p.name.clone(), *r)),
            _ => None,
        };
        if let Some((words, r)) = words {
            let tw = p.measure(&words, 12.0, Weight::Semibold, 240.0) + 26.0;
            let th = 30.0;
            let (cx, by) = (r.x + r.w / 2.0, r.y - 10.0);
            let k = (0.5 + 0.5 * tg).max(0.05);
            let mut before = windows::Foundation::Numerics::Matrix3x2::default();
            unsafe {
                dc.GetTransform(&mut before);
                dc.SetTransform(&(windows::Foundation::Numerics::Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: cx * p.s * (1.0 - k), M32: by * p.s * (1.0 - k) } * before));
            }
            let x0 = (cx - tw / 2.0).clamp(4.0, W - tw - 4.0);
            draw::faded(dc, tg.clamp(0.0, 1.0), || {
                p.round(Rect::new(x0, by - th, tw, th), th / 2.0, crate::paint::Color(0.2, 0.2, 0.23, 1.0));
                p.text(&words, x0 + 13.0, by - th / 2.0, 12.0, Weight::Semibold, 240.0, draw::TEXT);
            });
            unsafe { dc.SetTransform(&before) };
        }
    }
    // (a right-click's menu, over all of it)
    if let Some(c) = v.ctx {
        let (mx, my) = ctx_origin(c, h);
        let mh = c.rows.len() as f32 * CTX_ROW + 10.0;
        p.round(Rect::new(mx, my, CTX_W, mh), 12.0, crate::paint::Color(0.11, 0.11, 0.13, 1.0));
        for (i, (icon, label, _)) in c.rows.iter().enumerate() {
            let ry = my + 5.0 + i as f32 * CTX_ROW;
            if i == c.sel {
                p.round(Rect::new(mx + 5.0, ry, CTX_W - 10.0, CTX_ROW), 8.0, crate::paint::Color(0.15, 0.15, 0.17, 1.0));
            }
            p.icon(icon, mx + 14.0, ry + CTX_ROW / 2.0 - 7.0, 14.0, draw::DIM);
            p.text(label, mx + 36.0, ry + CTX_ROW / 2.0, 12.0, Weight::Regular, CTX_W - 46.0, draw::TEXT);
        }
    }
}

fn paint_item(p: &Painter, hit: &Hit, look: Look, r: &Rect, ring: Option<f32>, pins: &[AppPin], foot: &[AppPin]) {
    // (one of the foot's: its picture, small, or its pictogram; held ones red, their ring filling as they are held)
    if let Hit::FootPin(k) = hit {
        let Some(pin) = foot.get(*k) else { return };
        let (cx, cy, rr) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0);
        let (image, icon) = pin_look(pin);
        let z = 18.0;
        if image.is_empty() || !p.picture(&image, Rect::new(cx - z / 2.0, cy - z / 2.0, z, z), 4.0) {
            let c = if pin_held(pin) { crate::paint::Color(0.98, 0.6, 0.6, 1.0) } else { crate::paint::Color(0.79, 0.79, 0.82, 1.0) };
            p.icon(icon, cx - 8.0, cy - 8.0, 16.0, c);
        }
        if let Some(f) = ring {
            p.ring(cx, cy, rr - 1.5, 2.5, f, 0.0, crate::paint::Color(0.91, 0.42, 0.42, 1.0), crate::paint::Color(0.0, 0.0, 0.0, 0.0));
        }
        return;
    }
    // (a tool found by its name: a row, its pictogram in a tile; held ones, their ring filling round it as they are held)
    if let (Hit::Sys(x), Look::Row) = (hit, look) {
        let z = 28.0;
        let pr = Rect::new(r.x + 8.0, r.y + (ROW - z) / 2.0, z, z);
        p.round(pr, 7.0, draw::FAINT);
        let red = crate::paint::Color(0.98, 0.6, 0.6, 1.0);
        p.icon(x.icon(), pr.x + 7.0, pr.y + 7.0, 14.0, if x.held() { red } else { draw::TEXT });
        if let Some(f) = ring {
            p.ring(pr.x + z / 2.0, pr.y + z / 2.0, z / 2.0 + 2.0, 2.5, f, 0.0, crate::paint::Color(0.91, 0.42, 0.42, 1.0), crate::paint::Color(0.0, 0.0, 0.0, 0.0));
        }
        let name = tool_of(*x).map_or("", |t| t.1);
        let side = "";
        let sw = if side.is_empty() { 0.0 } else { p.measure(side, 11.0, Weight::Regular, 80.0) + 12.0 };
        p.text(name, r.x + 8.0 + z + 12.0, r.y + ROW / 2.0, 13.0, Weight::Semibold, r.w - z - 32.0 - sw, draw::TEXT);
        if !side.is_empty() {
            p.text_end(side, r.right() - 10.0, r.y + ROW / 2.0, 11.0, Weight::Regular, 80.0, draw::DIM);
        }
        return;
    }
    // (a ghost button: its pictogram alone; at the foot, restart and shut down red, their ring filling as they are held)
    if let Hit::Sys(x) = hit {
        let (cx, cy, rr) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0);
        let red = crate::paint::Color(0.91, 0.42, 0.42, 1.0);
        let z = 16.0;
        p.icon(x.icon(), cx - z / 2.0, cy - z / 2.0, z, if x.held() { crate::paint::Color(0.98, 0.6, 0.6, 1.0) } else { crate::paint::Color(0.79, 0.79, 0.82, 1.0) });
        if let Some(f) = ring {
            p.ring(cx, cy, rr - 1.5, 2.5, f, 0.0, red, crate::paint::Color(0.0, 0.0, 0.0, 0.0));
        }
        return;
    }
    match hit {
        Hit::AllApps => {
            p.icon("grid", r.x + r.w / 2.0 - 7.0, r.y + r.h / 2.0 - 7.0, 14.0, draw::DIM);
            return;
        }
        Hit::Back => {
            p.icon("left", r.x + r.w / 2.0 - 7.0, r.y + r.h / 2.0 - 7.0, 14.0, draw::DIM);
            return;
        }
        Hit::Rename => return,
        Hit::Letter(c) => {
            let t = c.to_string();
            let tw = p.measure(&t, 11.0, Weight::Semibold, 20.0);
            p.text(&t, r.x + r.w / 2.0 - tw / 2.0, r.y + r.h / 2.0, 11.0, Weight::Semibold, 20.0, draw::accent());
            return;
        }
        _ => {}
    }
    // (a tile: its picture, its name under it)
    if look == Look::Tile {
        let z = 36.0;
        let pr = Rect::new(r.x + r.w / 2.0 - z / 2.0, r.y + 10.0, z, z);
        let name = match hit {
            Hit::Pin(k) => {
                let Some(pin) = pins.get(*k) else { return };
                let (image, icon) = pin_look(pin);
                if image.is_empty() || !p.picture(&image, pr, 9.0) {
                    p.round(pr, 10.0, draw::FAINT);
                    p.icon(icon, pr.x + 9.0, pr.y + 9.0, 18.0, draw::TEXT);
                }
                pin.name.clone()
            }
            Hit::Folder(name) => {
                // (a folder: its first four, small, in a square)
                p.round(pr, 10.0, draw::FAINT);
                let mini = 13.0;
                for (n, pin) in pins.iter().filter(|x| &x.folder == name).take(4).enumerate() {
                    let mr = Rect::new(pr.x + 4.0 + (n % 2) as f32 * (mini + 2.0), pr.y + 4.0 + (n / 2) as f32 * (mini + 2.0), mini, mini);
                    let (image, icon) = pin_look(pin);
                    if image.is_empty() || !p.picture(&image, mr, 3.0) {
                        p.icon(icon, mr.x, mr.y, mini, draw::DIM);
                    }
                }
                name.clone()
            }
            _ => return,
        };
        let nw = p.measure(&name, 10.5, Weight::Regular, r.w - 6.0);
        p.text(&name, r.x + r.w / 2.0 - nw / 2.0, r.y + 60.0, 10.5, Weight::Regular, r.w - 6.0, crate::paint::Color(0.84, 0.84, 0.86, 1.0));
        return;
    }
    let (image, name, side) = match hit {
        Hit::App(a) => (app_image(&a.id), a.name.clone(), if pins.iter().any(|p| p.kind == "app" && p.id.eq_ignore_ascii_case(&a.id)) { "\u{1}pin".to_string() } else { String::new() }),
        Hit::Window(hw, title, exe) => (if exe.is_empty() { format!("hwnd:{hw}") } else { format!("exe:{exe}") }, title.clone(), "Open Window".to_string()),
        Hit::File(path, name, when) => (format!("shell:{path}"), name.clone(), ago(*when)),
        Hit::Setting(name, _) => (String::new(), name.to_string(), "Settings".to_string()),
        _ => return,
    };
    // (half a row: what was used lately, its name and how long ago, one under the other)
    if look == Look::Half {
        let z = 30.0;
        let pr = Rect::new(r.x + 8.0, r.y + (ROW - z) / 2.0, z, z);
        if !p.picture(&image, pr, 8.0) {
            p.round(pr, 8.0, draw::FAINT);
            p.icon("file", pr.x + 8.0, pr.y + 8.0, 14.0, draw::DIM);
        }
        p.text(&name, pr.right() + 10.0, r.y + ROW / 2.0 - 7.0, 12.0, Weight::Regular, r.w - z - 26.0, draw::TEXT);
        p.text(&side, pr.right() + 10.0, r.y + ROW / 2.0 + 8.0, 10.5, Weight::Regular, r.w - z - 26.0, draw::DIM);
        return;
    }
    let z = 28.0;
    let pr = Rect::new(r.x + 8.0, r.y + (ROW - z) / 2.0, z, z);
    if image.is_empty() {
        p.round(pr, 7.0, draw::FAINT);
        p.icon("gear", pr.x + 6.0, pr.y + 6.0, 14.0, draw::TEXT);
    } else if !p.picture(&image, pr, 7.0) {
        p.round(pr, 7.0, draw::SOFT);
    }
    // (on its right: pinned, a pin; else what it is, or how long ago)
    let sw = if side.is_empty() { 0.0 } else if side == "\u{1}pin" { 24.0 } else { p.measure(&side, 11.0, Weight::Regular, 120.0) + 12.0 };
    p.text(&name, r.x + 8.0 + z + 12.0, r.y + ROW / 2.0, 13.0, Weight::Semibold, r.w - z - 32.0 - sw, draw::TEXT);
    if side == "\u{1}pin" {
        p.icon("pin", r.right() - 24.0, r.y + ROW / 2.0 - 7.0, 14.0, draw::accent());
    } else if !side.is_empty() {
        p.text_end(&side, r.right() - 10.0, r.y + ROW / 2.0, 11.0, Weight::Regular, 120.0, draw::DIM);
    }
}

/// (for checking) The launcher with what is typed (`@all`: every app; `@menu`: a right-click's menu on the first pin), drawn
/// offscreen; its size (physical px) and pixels.
pub fn preview(g: &Gfx, pc: &Arc<Pc>, query: &str) -> windows::core::Result<(u32, u32, Vec<u8>)> {
    let s = 1.25;
    let mut l = Launcher::new(g)?;
    l.apps = apps::all();
    l.pinned = l.apps.iter().take(3).cloned().collect();
    let mut pins = default_pins();
    for a in l.apps.iter().take(8) {
        pins.push(AppPin { kind: "app".into(), id: a.id.clone(), name: a.name.clone(), folder: String::new() });
    }
    for p in pins.iter_mut().skip(10).take(3) {
        p.folder = "Games".into();
    }
    l.pins = pins;
    l.recent = recent_files(40);
    match query {
        "@all" => l.view = View::All,
        _ => l.query = query.to_string(),
    }
    l.search();
    if query == "@menu" {
        l.query.clear();
        l.search();
        l.ctx = Some(CtxMenu { at: (140.0, 150.0), rows: l.menu_for(1), sel: 0 });
    }
    let shp = shape::floating(&g.d2d_factory, W * s, l.h * s, RADIUS * s)?;
    let p = Painter { g, dc: &g.dc, s, pc, t: 2.0, viz: [0.0; 3] };
    let (pw, ph) = (((W + 2.0 * M) * s) as u32, ((H_MAX + 2.0 * M) * s) as u32);
    let hi = l.target();
    let places = l.drawn_places();
    let v = Paint { places: &places, onto: None, query: &l.query, hi, items: &l.items, deco: &l.deco, pins: &l.pins, foot: &l.foot, held: None, tip: (0, 0.0), drag: None, ctx: l.ctx.as_ref(), clip: l.clip, naming: false, unlit: false };
    let px = g.snapshot(pw, ph, [0.17, 0.2, 0.3, 1.0], |dc| {
        paint(&p, dc, &shp, s, 1.0, l.h, 2.0, &v);
    })?;
    Ok((pw, ph, px))
}
