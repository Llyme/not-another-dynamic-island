//! Games: the game you are playing, for how long, and how it runs. A plugin (see native.rs).
//!
//! Detection is the Rust port of `foreground_fullscreen_pid`
//! and friends from `platform_backend/windows.py`. A "game" candidate is a
//! foreground window that covers its whole monitor AND has no title bar
//! (`WS_CAPTION`): a maximized normal app (browser, editor) still carries
//! `WS_CAPTION` even filling the screen, while exclusive-fullscreen/
//! borderless games don't.

use crate::exeinfo;
use crate::native::{manifest_of, old_flag, Adopted, Ctx, Native};
use crate::plugins::Manifest;
use crate::winutil::{exe_path_for_pid, pid_alive};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, GWL_STYLE,
    WS_CAPTION,
};

mod fps;
mod stats;

const POLL_MS: u64 = 1500;

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("games.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin
pub fn adopted(old: &Value, default_on: bool) -> Adopted {
    Adopted { on: old_flag(old, "game_detection", default_on), values: Default::default() }
}

#[derive(Serialize, Clone, Default)]
pub struct GameSnapshot {
    pub has_game: bool,
    pub name: String,
    /// data: URL of the exe's icon
    pub icon: Option<String>,
    #[serde(skip)]
    pub exe_path: String,
    /// how many games are running in total (the pill shows "+N" past one)
    pub count: usize,
    /// process id of this game, to measure it (FPS, GPU, ...)
    pub pid: u32,
}

/// Covers its whole monitor and has no title bar: exclusive-fullscreen or
/// borderless-fullscreen, as games are (a maximized browser/IDE keeps its caption).
fn caption_less_fullscreen(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return false;
    }
    let hmon: HMONITOR = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        return false;
    }
    let mon = mi.rcMonitor;
    let is_fullscreen = rect.left <= mon.left
        && rect.top <= mon.top
        && rect.right >= mon.right
        && rect.bottom >= mon.bottom;
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    is_fullscreen && style & WS_CAPTION.0 == 0
}

fn foreground_fullscreen_pid() -> Option<u32> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() || !caption_less_fullscreen(hwnd) {
        return None;
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// Fallback when the exe has no ProductName/FileDescription: filename minus
/// extension, title-cased on separators.
fn display_name_from_path(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".exe").or(file.strip_suffix(".EXE")).unwrap_or(file);
    let mut out = String::new();
    let mut cap_next = true;
    for ch in stem.chars() {
        if ch == '_' || ch == '-' {
            out.push(' ');
            cap_next = true;
        } else if cap_next {
            out.extend(ch.to_uppercase());
            cap_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

const IGNORE_EXE: &[&str] = &[
    "explorer.exe", "shellexperiencehost.exe", "searchhost.exe", "searchapp.exe", "startmenuexperiencehost.exe",
    "applicationframehost.exe", "textinputhost.exe", "lockapp.exe", "systemsettings.exe", "screenclippinghost.exe",
    "gamebar.exe", "widgets.exe",
];

/// Fullscreen video players, browsers (F11 / fullscreen video), presentations and
/// remote-desktop clients are caption-less fullscreen windows too, but they
/// are not games. Matched as substrings of the exe's file name.
const NOT_GAMES: &[&str] = &[
    "potplayer", "vlc", "mpv", "mpc-hc", "mpc-be", "wmplayer", "kmplayer", "gomplayer", "smplayer",
    "video.ui", "moviesandtv", "chrome", "msedge", "firefox", "brave", "opera", "vivaldi", "zoom",
    "teams", "powerpnt", "acrord", "mstsc", "vmconnect", "obs64", "obs32",
];

fn is_not_game(exe_path: &str) -> bool {
    let file = exe_path.rsplit(['\\', '/']).next().unwrap_or(exe_path).to_lowercase();
    IGNORE_EXE.contains(&file.as_str()) || NOT_GAMES.iter().any(|n| file.contains(n))
}

fn snapshot_for(path: &str) -> GameSnapshot {
    let info = exeinfo::lookup(path);
    GameSnapshot {
        has_game: true,
        name: info.name.unwrap_or_else(|| display_name_from_path(path)),
        icon: info.icon,
        exe_path: path.to_string(),
        count: 1,
        pid: 0,
    }
}

/// Every fullscreen, caption-less window on any screen is "a game" -- focused
/// or not (a game on the other monitor while you use the island stays
/// detected). The focused one comes first, then front-to-back z-order.
fn detect_games() -> Vec<(u32, GameSnapshot)> {
    let mut out: Vec<(u32, GameSnapshot)> = Vec::new();
    if let Some(pid) = foreground_fullscreen_pid() {
        if let Some(path) = exe_path_for_pid(pid).filter(|p| !is_not_game(p)) {
            out.push((pid, snapshot_for(&path)));
        }
    }
    for w in crate::scan::visible_windows() {
        if w.minimized || out.iter().any(|(p, _)| *p == w.pid) || is_not_game(&w.exe) {
            continue;
        }
        if caption_less_fullscreen(w.hwnd) {
            out.push((w.pid, snapshot_for(&w.exe)));
        }
    }
    out
}

struct Running {
    pid: u32,
    snap: GameSnapshot,
    since: Instant,
}

#[derive(Default)]
struct Inner {
    /// running games, the one most recently in front first
    running: Mutex<Vec<Running>>,
    meter: Mutex<stats::GameMeter>,
}

#[derive(Default)]
pub struct Games {
    inner: Arc<Inner>,
}

/// One pass: look at the screens, and keep the list of running games up to date. A game outlives focus (alt-tabbing to
/// a browser mid-game must not end one): it ends when its process exits. Several can run at once, the most recently
/// focused first; the pill shows the front one, the hub lists them all.
fn pass(running: &mut Vec<Running>) {
    // back to front, so the frontmost game ends up first
    for (pid, snap) in detect_games().into_iter().rev() {
        match running.iter().position(|r| r.pid == pid) {
            Some(i) => {
                let mut r = running.remove(i);
                r.snap = snap;
                running.insert(0, r);
            }
            None => running.insert(0, Running { pid, snap, since: Instant::now() }),
        }
    }
    running.retain(|r| pid_alive(r.pid) && !is_not_game(&r.snap.exe_path));
}

fn publish(inner: &Inner, ctx: &Ctx) {
    let (snap, any) = {
        let running = inner.running.lock().unwrap();
        let mut snap = running.first().map(|r| r.snap.clone()).unwrap_or_default();
        snap.count = running.len();
        snap.pid = running.first().map_or(0, |r| r.pid);
        (snap, !running.is_empty())
    };
    ctx.offer_pill(any.then_some("game"));
    ctx.set_quiet(any);
    ctx.emit("game-tick", snap);
}

impl Native for Games {
    fn manifest(&self) -> Manifest {
        manifest()
    }

    fn start(&self, ctx: &Ctx) {
        let (ctx, inner) = (ctx.clone(), self.inner.clone());
        std::thread::spawn(move || loop {
            if ctx.on() {
                pass(&mut inner.running.lock().unwrap());
                publish(&inner, &ctx);
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        });
    }

    fn switched(&self, ctx: &Ctx, on: bool) {
        if !on {
            self.inner.running.lock().unwrap().clear();
            ctx.emit("game-tick", GameSnapshot::default());
        }
    }

    fn cards(&self, _ctx: &Ctx) -> Option<Value> {
        let running = self.inner.running.lock().unwrap();
        Some(json!(running
            .iter()
            .map(|r| json!({
                "pid": r.pid,
                "name": r.snap.name,
                "exe_path": r.snap.exe_path,
                "icon": exeinfo::lookup(&r.snap.exe_path).icon,
                "playtime_secs": r.since.elapsed().as_secs_f64(),
            }))
            .collect::<Vec<_>>()))
    }

    fn call(&self, _ctx: &Ctx, cmd: &str, args: &Value) -> Result<Value, String> {
        match cmd {
            // what the running games use (CPU, memory, GPU, video memory, frames); an empty list lets the counters go
            "stats" => {
                let pids: Vec<u32> = args.get("pids").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                serde_json::to_value(stats::read(&self.inner.meter, pids)).map_err(|e| e.to_string())
            }
            _ => Err(format!("no such call: {cmd}")),
        }
    }

    fn adopt(&self, old: &Value, default_on: bool) -> Adopted {
        adopted(old, default_on)
    }
}
