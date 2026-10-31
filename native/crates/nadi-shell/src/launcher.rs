//! The launcher: what Start was. It pops up in the middle of the screen (springing a little past its size, and back), takes the
//! keyboard, and finds an app, an open window, a setting or a recent file. Before anything is typed it shows the PC's own buttons,
//! round: the radios and the like, its tools, and its power (restart and shut down are held until their ring fills); each says what
//! it is in a tip. As one types, a single list, the open windows first (switching beats starting again). Enter opens the one picked; the arrows move between them; Esc, Win or a click elsewhere puts it
//! away. A window of its own, since it takes the focus; drawn like the rest of the frame.

use crate::apps::{self, App};
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
/// the PC's own buttons: how many to a row, how big, how tall a row is, and how long one is held to restart or shut down
const SYS_COLS: usize = 5;
const SYS_D: f32 = 58.0;
const SYS_ROW: f32 = 76.0;
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
    SignOut,
    Restart,
    ShutDown,
}

const SYS: [Sys; 15] = [
    Sys::Wifi, Sys::Bluetooth, Sys::NightLight, Sys::Focus, Sys::Desktop,
    Sys::Settings, Sys::Files, Sys::Tasks, Sys::Terminal, Sys::Windows,
    Sys::Lock, Sys::Sleep, Sys::SignOut, Sys::Restart, Sys::ShutDown,
];

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
            Sys::SignOut => "signout",
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
            Sys::SignOut => "Hold to Sign Out".into(),
            Sys::Settings => "Settings".into(),
            Sys::Files => "File Explorer".into(),
            Sys::Tasks => "Task Manager".into(),
            Sys::Terminal => "Terminal".into(),
            Sys::Lock => "Lock".into(),
            Sys::Sleep => "Sleep".into(),
            Sys::Restart => "Hold to Restart".into(),
            Sys::ShutDown => "Hold to Shut Down".into(),
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
        matches!(self, Sys::Restart | Sys::ShutDown | Sys::SignOut)
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
            Sys::SignOut => {
                unsafe {
                    let _ = windows::Win32::System::Shutdown::ExitWindowsEx(windows::Win32::System::Shutdown::EWX_LOGOFF, windows::Win32::System::Shutdown::SHUTDOWN_REASON(0));
                }
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

/// How a thing is shown: a tile of the grid (pinned apps), or a row.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Row,
    /// one of the PC's own buttons: round
    Round,
}

pub struct Launcher {
    pub hwnd: HWND,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    surface: IDCompositionSurface,
    s: f32,
    apps: Vec<App>,
    pinned: Vec<App>,
    recent: Vec<(String, String, SystemTime)>,
    query: String,
    sel: usize,
    /// what it shows now, how, and where (logical px, from the top of its content)
    items: Vec<(Hit, Look, Rect)>,
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
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        86400..=172799 => "Yesterday".into(),
        _ => format!("{}d", s / 86400),
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
                recent: Vec::new(),
                query: String::new(),
                sel: 0,
                items: Vec::new(),
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
            })
        }
    }

    /// Ready it on a monitor: in the middle of `area` (physical px), a little above. `pinned`: the dock's pinned apps.
    pub fn show(&mut self, g: &Gfx, area: RECT, scale: f32, pinned: Vec<App>, pick: bool) {
        self.pick = pick;
        self.picked = None;
        self.apps = apps::cached();
        self.pinned = pinned;
        self.recent = recent_files(40);
        self.s = scale;
        self.query.clear();
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

    /// Whether the one picked shows its tip now (one of the PC's buttons, picked a moment ago).
    fn tip_wanted(&self) -> bool {
        self.shown && matches!(self.items.get(self.sel), Some((Hit::Sys(_), _, _))) && self.sel_at.elapsed().as_secs_f32() > 0.4 && self.opened.elapsed().as_secs_f32() > 0.5
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
        let tip_moves = (self.tip - if self.tip_wanted() { 1.0 } else { 0.0 }).abs() > 0.002 || self.tip_v.abs() > 0.02 || (self.shown && matches!(self.items.get(self.sel), Some((Hit::Sys(_), _, _))) && self.sel_at.elapsed().as_secs_f32() < 0.6);
        (self.t != self.to || self.vel != 0.0) || (self.shown && self.opened.elapsed().as_secs_f32() < 0.9) || (self.shown && hi_moves) || (self.shown && tip_moves) || self.hold.is_some()
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
        if !self.moving() {
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
                if let Some((Hit::Sys(x), _, _)) = self.items.get(i) {
                    if x.run() {
                        self.hide();
                    }
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

    /// What it shows for what is typed, laid out.
    fn search(&mut self) {
        let q = self.query.trim().to_string();
        let mut items: Vec<(Hit, Look)> = Vec::new();
        if self.pick {
            // (picking an app to pin: the apps alone, the ones not pinned yet)
            let mut found: Vec<(u32, Hit)> = self
                .apps
                .iter()
                .filter(|a| !self.pinned.iter().any(|p| p.id.eq_ignore_ascii_case(&a.id)))
                .filter_map(|a| {
                    let sc = if q.is_empty() { 1 } else { apps::score(&a.name, &q) };
                    (sc > 0).then(|| (sc, Hit::App(a.clone())))
                })
                .collect();
            if !q.is_empty() {
                found.sort_by(|a, b| b.0.cmp(&a.0));
            }
            items = found.into_iter().take(9).map(|(_, h)| (h, Look::Row)).collect();
        } else if q.is_empty() {
            for x in SYS {
                items.push((Hit::Sys(x), Look::Round));
            }
        } else {
            let mut found: Vec<(u32, Hit)> = Vec::new();
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
            found.sort_by(|a, b| b.0.cmp(&a.0));
            items = found.into_iter().take(9).map(|(_, h)| (h, Look::Row)).collect();
        }
        // where each goes
        let inner = W - 2.0 * PAD;
        let mut y = PAD + FIELD + GAP;
        let mut out = Vec::new();
        let mut last: Option<Look> = None;
        let mut col = 0usize;
        for (hit, look) in items {
            if last.is_some_and(|l| l != look) {
                if col > 0 {
                    y += match last.unwrap() {
                        Look::Row => ROW,
                        Look::Round => SYS_ROW,
                    };
                }
                y += GAP;
                col = 0;
            }
            let r = match look {
                Look::Row => {
                    let r = Rect::new(PAD, y, inner, ROW);
                    y += ROW;
                    r
                }
                Look::Round => {
                    // (the round button itself, in the middle of its cell)
                    let cw = inner / SYS_COLS as f32;
                    let r = Rect::new(PAD + col as f32 * cw + cw / 2.0 - SYS_D / 2.0, y + (SYS_ROW - SYS_D) / 2.0, SYS_D, SYS_D);
                    col += 1;
                    if col == SYS_COLS {
                        col = 0;
                        y += SYS_ROW;
                    }
                    r
                }
            };
            out.push((hit, look, r));
            last = Some(look);
        }
        if col > 0 {
            y += match last.unwrap_or(Look::Row) {
                Look::Row => ROW,
                Look::Round => SYS_ROW,
            };
        }
        if out.is_empty() {
            y += 24.0;
        }
        self.h = (y + PAD).min(H_MAX);
        self.items = out;
        self.sel = 0;
        self.sel_at = Instant::now();
        self.hold = None;
        if self.shown {
            self.clip();
        }
    }

    /// The app picked to pin (once).
    pub fn take_picked(&mut self) -> Option<App> {
        self.picked.take()
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
            Some(Hit::Sys(x)) if x.held() => return,
            Some(Hit::Sys(x)) => {
                if !x.run() {
                    return;
                }
            }
            Some(Hit::Setting(_, uri)) => apps::launch_uri(uri),
            Some(Hit::App(a)) => apps::launch(&a.id),
            Some(Hit::Window(h, _, _)) => winlist::activate(h),
            Some(Hit::File(path, _, _)) => unsafe {
                let _ = windows::Win32::UI::Shell::ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(path), None, None, SW_SHOWNORMAL);
            },
            None => return,
        }
        self.hide();
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
        }
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
                match vk {
                    VK_ESCAPE => self.hide(),
                    VK_RETURN => {
                        // (a held one: held while Enter is)
                        match self.items.get(self.sel) {
                            Some((Hit::Sys(x), _, _)) if x.held() => {
                                if self.hold.is_none() && (lp.0 >> 30) & 1 == 0 {
                                    self.hold = Some((self.sel, Instant::now()));
                                }
                            }
                            _ => self.open_selected(),
                        }
                    }
                    VK_DOWN => self.go(0.0, 1.0),
                    VK_UP => self.go(0.0, -1.0),
                    VK_RIGHT => self.go(1.0, 0.0),
                    VK_LEFT => self.go(-1.0, 0.0),
                    VK_BACK => {
                        self.query.pop();
                        self.search();
                    }
                    _ => return None,
                }
                Some(LRESULT(0))
            }
            WM_CHAR => {
                let c = char::from_u32(wp.0 as u32).unwrap_or('\0');
                if !c.is_control() && self.query.chars().count() < 60 {
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
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as f32 / self.s - M, ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.s - M);
                // (the field's x, while it holds something)
                let xr = clear_box();
                if !self.query.is_empty() && x >= xr.x && x < xr.right() && y >= xr.y && y < xr.bottom() {
                    if msg == WM_LBUTTONUP {
                        self.query.clear();
                        self.search();
                    }
                    return Some(LRESULT(0));
                }
                let under = self.items.iter().position(|(_, _, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom());
                // (a held one let go of, or left: not done)
                if self.hold.is_some_and(|(i, _)| msg == WM_LBUTTONUP || under != Some(i)) {
                    self.hold = None;
                    return Some(LRESULT(0));
                }
                if let Some(i) = under {
                    self.pick_item(i);
                    let held = matches!(self.items.get(i), Some((Hit::Sys(x), _, _)) if x.held());
                    if msg == WM_LBUTTONDOWN && held {
                        self.hold = Some((i, Instant::now()));
                    } else if msg == WM_LBUTTONUP && !held {
                        self.open_selected();
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
        let (query, items, hi) = (self.query.clone(), self.items.clone(), self.hi);
        let held = self.hold.map(|(i, at)| (i, (at.elapsed().as_secs_f32() / HOLD).min(1.0)));
        let tip = (self.sel, self.tip);
        let _ = g.draw_surface(&self.surface, |dc| {
            paint(&p, dc, &shp, s, t, h, age, &query, hi, &items, held, tip);
        });
    }
}

/// Draw the launcher: its shape `h` tall, scaled from its middle by how far out it is (`t`, springing past 1), and what is in it,
/// each thing popping in after the one before (`age`: since it opened); the box round the one picked at `hi`.
#[allow(clippy::too_many_arguments)]
fn paint(p: &Painter, dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, shp: &shape::Shape, s: f32, t: f32, h: f32, age: f32, query: &str, hi: [f32; 4], items: &[(Hit, Look, Rect)], held: Option<(usize, f32)>, tip: (usize, f32)) {
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
        paint_inside(p, age, query, hi, items, held, tip);
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

fn paint_inside(p: &Painter, age: f32, query: &str, hi: [f32; 4], items: &[(Hit, Look, Rect)], held: Option<(usize, f32)>, tip: (usize, f32)) {
    let dc = p.dc;
    let w = W - 2.0 * PAD;
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
    // (the box round the one picked: it slides from one to the next)
    if !items.is_empty() {
        // (round round a round one)
        let rad = if (hi[2] - hi[3]).abs() < 1.0 { hi[2] / 2.0 } else { 16.0 };
        let grow = if (hi[2] - hi[3]).abs() < 1.0 { 5.0 } else { 0.0 };
        p.round(Rect::new(hi[0] - grow, hi[1] - grow, hi[2] + 2.0 * grow, hi[3] + 2.0 * grow), rad + grow, draw::FAINT);
    }
    if items.is_empty() {
        p.text("Nothing found", PAD + 4.0, PAD + FIELD + GAP + 12.0, 13.0, Weight::Regular, w, draw::DIM);
    }
    for (i, (hit, look, r)) in items.iter().enumerate() {
        // (popping in: from a little smaller, and below, to its place)
        let e = pop(age, i);
        if e <= 0.001 {
            continue;
        }
        let (ix, iy) = ((r.x + r.w / 2.0) * p.s, (r.y + r.h / 2.0) * p.s);
        let kk = 0.7 + 0.3 * e;
        let mut before = windows::Foundation::Numerics::Matrix3x2::default();
        unsafe {
            dc.GetTransform(&mut before);
            dc.SetTransform(&(windows::Foundation::Numerics::Matrix3x2 { M11: kk, M12: 0.0, M21: 0.0, M22: kk, M31: ix * (1.0 - kk), M32: iy * (1.0 - kk) + (1.0 - e) * 10.0 * p.s } * before));
        }
        let ring = held.filter(|(h, _)| *h == i).map(|(_, f)| f);
        draw::faded(dc, e.clamp(0.0, 1.0), || paint_item(p, hit, *look, r, ring));
        unsafe { dc.SetTransform(&before) };
    }
    // (the tip of the one picked: it springs up out of it)
    let (ti, tg) = tip;
    if tg > 0.01 {
        if let Some((Hit::Sys(x), _, r)) = items.get(ti) {
            let words = x.tip();
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
}

fn paint_item(p: &Painter, hit: &Hit, look: Look, r: &Rect, ring: Option<f32>) {
    // (one of the PC's own buttons: round; a radio that is on, blue; restart and shut down, red, their ring filling as they are held)
    if let Hit::Sys(x) = hit {
        let (cx, cy, rr) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0);
        let red = crate::paint::Color(0.91, 0.42, 0.42, 1.0);
        let bg = if x.on() { crate::paint::Color(0.31, 0.56, 0.94, 0.45) } else if x.held() { red.alpha(0.2) } else { draw::FAINT };
        p.circle(cx, cy, rr, bg);
        let z = 22.0;
        p.icon(x.icon(), cx - z / 2.0, cy - z / 2.0, z, if x.held() { crate::paint::Color(0.98, 0.7, 0.7, 1.0) } else { draw::TEXT });
        if let Some(f) = ring {
            p.ring(cx, cy, rr - 1.5, 3.0, f, 0.0, red, crate::paint::Color(0.0, 0.0, 0.0, 0.0));
        }
        return;
    }
    {
        let (image, name, side) = match hit {
            Hit::App(a) => (app_image(&a.id), a.name.clone(), String::new()),
            Hit::Window(hw, title, exe) => (if exe.is_empty() { format!("hwnd:{hw}") } else { format!("exe:{exe}") }, title.clone(), "Open window".to_string()),
            Hit::File(path, name, when) => (format!("shell:{path}"), name.clone(), ago(*when)),
            Hit::Setting(name, _) => (String::new(), name.to_string(), "Settings".to_string()),
            Hit::Sys(_) => return,
        };
        match look {
            Look::Round => {}
            Look::Row => {
                let z = 28.0;
                let pr = Rect::new(r.x + 8.0, r.y + (ROW - z) / 2.0, z, z);
                if image.is_empty() {
                    p.round(pr, 7.0, draw::FAINT);
                    p.icon("gear", pr.x + 6.0, pr.y + 6.0, 14.0, draw::TEXT);
                } else if !p.picture(&image, pr, 7.0) {
                    p.round(pr, 7.0, draw::SOFT);
                }
                let sw = if side.is_empty() { 0.0 } else { p.measure(&side, 11.0, Weight::Regular, 120.0) + 12.0 };
                p.text(&name, r.x + 8.0 + z + 12.0, r.y + ROW / 2.0, 13.0, Weight::Semibold, r.w - z - 32.0 - sw, draw::TEXT);
                if !side.is_empty() {
                    p.text_end(&side, r.right() - 10.0, r.y + ROW / 2.0, 11.0, Weight::Regular, 120.0, draw::DIM);
                }
            }
        }
    }
}

/// (for checking) The launcher with what is typed, drawn offscreen; its size (physical px) and pixels.
pub fn preview(g: &Gfx, pc: &Arc<Pc>, query: &str) -> windows::core::Result<(u32, u32, Vec<u8>)> {
    let s = 1.25;
    let mut l = Launcher::new(g)?;
    l.apps = apps::all();
    l.pinned = l.apps.iter().take(6).cloned().collect();
    l.recent = recent_files(40);
    l.query = query.to_string();
    l.search();
    let shp = shape::floating(&g.d2d_factory, W * s, l.h * s, RADIUS * s)?;
    let p = Painter { g, dc: &g.dc, s, pc, t: 2.0, viz: [0.0; 3] };
    let (pw, ph) = (((W + 2.0 * M) * s) as u32, ((H_MAX + 2.0 * M) * s) as u32);
    let hi = l.target();
    let px = g.snapshot(pw, ph, [0.17, 0.2, 0.3, 1.0], |dc| {
        paint(&p, dc, &shp, s, 1.0, l.h, 2.0, &l.query, hi, &l.items, Some((11, 0.6)), (11, 1.0));
    })?;
    Ok((pw, ph, px))
}
