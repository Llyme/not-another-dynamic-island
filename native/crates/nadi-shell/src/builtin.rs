//! The frame's own widgets, which take the taskbar's place: the dock (the launcher's button, the pinned apps, the apps that are open;
//! held under the mouse, the windows of one, as live pictures) and quick settings (the volume, locking and turning off the PC, the
//! last notifications). They are widgets like the plugins', drawn by the same frame; what they are pressed for is done here.

use crate::apps;
use crate::config::{Config, Pin};
use crate::winlist::{self, AppWindow};
use nadi_core::pc::Pc;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const PLUGIN: &str = "nadi";
pub const DOCK: &str = "dock";
pub const QUICK: &str = "quick";
pub const TRAY: &str = "tray";
/// the launcher's button, a widget of its own (placed where you like, beside the dock or not)
pub const APPS: &str = "apps";

/// The launcher's button, as a widget: pressed, the launcher opens.
pub fn apps_widget() -> Value {
    json!({ "peek": { "icon": "grid" }, "press": "launcher", "tip": "Apps" })
}

/// The tray as a widget: at rest its first icons (the pinned ones first) and how many more; open, every one.
/// The icons the Tray widget shows: the apps' (Windows' own, such as the speaker and removable drives, are not shown, nor NADI's
/// own: its menu is a right-click on any widget).
pub fn tray_icons() -> Vec<crate::trayhost::Icon> {
    let me = std::process::id();
    let windows = crate::trayhost::explorer_pid();
    crate::trayhost::icons().into_iter().filter(|i| !i.hidden && i.pid != me && Some(i.pid) != windows).collect()
}

/// A process's program (asked of Windows once, then kept a while: a tray that changes twice a second would ask every process each time).
fn exe_of(pid: u32) -> String {
    thread_local! {
        static EXES: std::cell::RefCell<std::collections::HashMap<u32, (String, std::time::Instant)>> = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    EXES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((e, _)) = m.get(&pid).filter(|(_, at)| at.elapsed().as_secs() < 300) {
            return e.clone();
        }
        let e = nadi_core::win::exe_path_for_pid(pid).unwrap_or_default();
        if m.len() > 256 {
            m.clear();
        }
        m.insert(pid, (e.clone(), std::time::Instant::now()));
        e
    })
}

/// What names a tray icon from one start to the next: its program, and its number there.
pub fn tray_key(i: &crate::trayhost::Icon) -> String {
    format!("{}:{}", crate::winlist::app_name(&exe_of(i.pid)).to_lowercase(), i.uid)
}

/// The icons in the Tray widget's row: the pinned ones in their order, else the first few.
pub fn tray_row(list: &[crate::trayhost::Icon], pinned: &[String]) -> Vec<String> {
    let row: Vec<String> = pinned.iter().filter(|p| list.iter().any(|i| &tray_key(i) == *p)).cloned().collect();
    if row.is_empty() {
        list.iter().take(4).map(tray_key).collect()
    } else {
        row
    }
}

/// The tray as a widget: at rest its row of icons and how many more; open, every one. While editing, every one in its peek, the row
/// first (a mark after it), so they can be dragged in and out of the row.
pub fn tray(pinned: &[String], editing: bool) -> Value {
    let list = tray_icons();
    let item = |i: &crate::trayhost::Icon| json!({ "image": format!("trayicon:{}", i.icon), "action": format!("tray:{}:{}", i.owner, i.uid), "label": i.tip.lines().next().unwrap_or(""), "news": tray_news(&i.tip, &tray_words(i).1) });
    let keys = tray_row(&list, pinned);
    let mut row: Vec<&crate::trayhost::Icon> = keys.iter().filter_map(|k| list.iter().find(|i| &tray_key(i) == k)).collect();
    let split = row.len();
    if editing {
        row.extend(list.iter().filter(|i| !keys.contains(&tray_key(i))));
    }
    let more = list.len().saturating_sub(row.len());
    // (open: a tile each, three to a row; the panel as narrow as they are)
    let cols = list.len().clamp(1, crate::blocks::TRAY_COLS) as f32;
    let width = cols * crate::blocks::TRAY_TILE_W + (cols - 1.0) * 8.0 + 2.0 * 16.0;
    let tile = |i: &crate::trayhost::Icon| {
        let (name, status) = tray_words(i);
        json!({ "image": format!("trayicon:{}", i.icon), "action": format!("tray:{}:{}", i.owner, i.uid), "name": name, "status": status, "tip": i.tip.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n") })
    };
    json!({
        "peek": { "tray": { "items": row.iter().map(|i| item(i)).collect::<Vec<_>>(), "more": more, "split": if editing { split } else { 0 } } },
        "panel": { "width": width, "body": [{ "kind": "traytiles", "items": list.iter().map(tile).collect::<Vec<_>>() }] },
        // (the ones not in its row, by name)
        "tip": list.iter().filter(|i| !row.iter().any(|r| r.owner == i.owner && r.uid == i.uid)).map(|i| i.tip.lines().next().unwrap_or("").to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n"),
    })
}

/// A tray icon's app, by name, and what its tip says besides its name ("Sunset in 2 hours"; none: nothing more).
pub fn tray_words(i: &crate::trayhost::Icon) -> (String, String) {
    let exe = exe_of(i.pid);
    let lines: Vec<&str> = i.tip.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let name = if exe.is_empty() { lines.first().map(|l| l.to_string()).unwrap_or_default() } else { crate::winlist::app_name(&exe.to_lowercase()) };
    let low = name.to_lowercase();
    // (the first line that says more than its name; its name at its start taken off)
    let status = lines
        .iter()
        .filter_map(|l| {
            let ll = l.to_lowercase();
            if ll == low {
                return None;
            }
            let rest = if ll.starts_with(&low) && l.is_char_boundary(name.len().min(l.len())) { l[name.len().min(l.len())..].trim_start_matches([' ', '-', ':', '–', '—', '|']).trim() } else { l };
            (!rest.is_empty()).then(|| rest.to_string())
        })
        .next()
        .unwrap_or_default();
    (name, status)
}

/// A menu's item for what a window works on: its folder opened, or its document shown in its folder (none: it is not found).
fn context_item(w: &winlist::AppWindow) -> Option<Value> {
    match crate::context::of(w.hwnd, &w.exe, &w.title)? {
        crate::context::Context::Folder(_) => Some(json!({ "icon": "folder", "label": "Open Folder", "action": format!("ctx:{}", w.hwnd) })),
        crate::context::Context::File(_) => Some(json!({ "icon": "file", "label": "Show Document", "action": format!("ctx:{}", w.hwnd) })),
    }
}

/// A window's title less its app's name ("main.rs - NADI - Visual Studio Code" is "main.rs - NADI"); the name alone: nothing.
pub fn short_title(title: &str, app: &str) -> String {
    let t = title.trim();
    let low = t.to_lowercase();
    let name = app.trim().to_lowercase();
    if name.is_empty() || low == name {
        return if low == name { String::new() } else { t.to_string() };
    }
    for sep in [" - ", " — ", " – ", " | ", ": "] {
        let tail = format!("{sep}{name}");
        if low.ends_with(&tail) {
            return t[..t.len() - tail.len()].trim().to_string();
        }
        let head = format!("{name}{sep}");
        if low.starts_with(&head) {
            return t[head.len()..].trim().to_string();
        }
    }
    t.to_string()
}

/// What a tray icon's app has to say worth a glance, short: connected or not ({ "mark": "ok" | "off" }), how fast it moves things
/// ({ "text": "↓ 43 KB/s" }), a count ({ "text": "3" }); else nothing (a version, its own name, anything longer).
pub fn tray_news(tip: &str, status: &str) -> Value {
    let low = status.to_lowercase();
    if ["disconnected", "not connected", "offline", "no connection"].iter().any(|w| low.contains(w)) {
        return json!({ "mark": "off" });
    }
    if low == "connected" || low.starts_with("connected") || low == "online" || low.starts_with("protected") {
        return json!({ "mark": "ok" });
    }
    // (speeds, down and up, anywhere in its tip)
    let (mut down, mut up) = (None::<f64>, None::<f64>);
    for line in tip.lines() {
        let l = line.to_lowercase();
        let Some(v) = speed_of(&l) else { continue };
        if ["dl", "down", "↓", "receiv"].iter().any(|w| l.contains(w)) {
            down = Some(down.unwrap_or(0.0) + v);
        } else if ["ul", "up", "↑", "send"].iter().any(|w| l.contains(w)) {
            up = Some(up.unwrap_or(0.0) + v);
        }
    }
    let rate = |b: f64| {
        let (v, u) = if b >= 1048576.0 { (b / 1048576.0, "MB/s") } else { (b / 1024.0, "KB/s") };
        if v >= 10.0 { format!("{v:.0} {u}") } else { format!("{v:.1} {u}") }
    };
    let mut parts = Vec::new();
    if let Some(d) = down.filter(|d| *d >= 1024.0) {
        parts.push(format!("↓ {}", rate(d)));
    }
    if let Some(u) = up.filter(|u| *u >= 10240.0) {
        parts.push(format!("↑ {}", rate(u)));
    }
    if !parts.is_empty() {
        return json!({ "text": parts.join("  ") });
    }
    // (a count)
    if !status.is_empty() && status.len() <= 4 && status.chars().all(|c| c.is_ascii_digit()) {
        return json!({ "text": status });
    }
    Value::Null
}

/// A speed in a line ("43.2 KiB/s", "1,2 MB/s"), in bytes a second.
fn speed_of(l: &str) -> Option<f64> {
    let units = [("gib/s", 1073741824.0), ("gb/s", 1e9), ("mib/s", 1048576.0), ("mb/s", 1e6), ("kib/s", 1024.0), ("kb/s", 1e3), ("b/s", 1.0)];
    for (u, k) in units {
        if let Some(at) = l.find(u) {
            let before = l[..at].trim_end();
            let num: String = before.chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect::<Vec<_>>().into_iter().rev().collect();
            if let Ok(v) = num.replace(',', ".").parse::<f64>() {
                return Some(v * k);
            }
        }
    }
    None
}

/// Memory, short: "480 MB", "1.4 GB".
fn mem_text(b: u64) -> String {
    let mb = b as f64 / 1048576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

/// How far what each program downloads is (by its file name, lower case: "chrome.exe"), all its downloads together; from the
/// downloads sensor's last look (one older than a few seconds is gone: nothing reads it any more).
fn downloads_by_program(pc: &Arc<Pc>) -> std::collections::HashMap<String, f32> {
    let (at, list) = pc.downloads.lock().unwrap().clone();
    let mut sums: std::collections::HashMap<String, (f64, f64)> = std::collections::HashMap::new();
    if at.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
        for d in &list {
            let Some(total) = d["total"].as_f64().filter(|t| *t > 0.0) else { continue };
            let got = d["received"].as_f64().unwrap_or(0.0);
            let programs: &[&str] = match d["source"].as_str().unwrap_or("") {
                "Chrome" => &["chrome.exe"],
                "Edge" => &["msedge.exe"],
                "Brave" => &["brave.exe"],
                "Vivaldi" => &["vivaldi.exe"],
                "Opera" => &["opera.exe"],
                "Firefox" => &["firefox.exe"],
                "Steam" => &["steam.exe", "steamwebhelper.exe"],
                "qBittorrent" => &["qbittorrent.exe"],
                _ => &[],
            };
            for p in programs {
                let e = sums.entry(p.to_string()).or_default();
                e.0 += got;
                e.1 += total;
            }
        }
    }
    sums.into_iter().map(|(k, (g, t))| (k, (g / t).clamp(0.0, 1.0) as f32)).collect()
}

/// One app on the dock: pinned, open, or both.
#[derive(Clone, Debug, PartialEq)]
pub struct DockApp {
    /// the program's path (lower case), or a store app's id
    pub key: String,
    pub name: String,
    pub image: String,
    pub pinned: bool,
    pub windows: Vec<AppWindow>,
    /// what is opened for it (and what it is pinned as): the program, or a shortcut pinned as itself
    pub open: String,
}

/// What a press asks of the frame (beyond what is done here).
pub enum Effect {
    None,
    /// draw again
    Changed,
    /// open or shut the launcher
    Launcher,
    /// open NADI's settings
    Settings,
}

pub struct Builtins {
    pub windows: Vec<AppWindow>,
    /// the order the open apps came in (their keys): new ones go last, as on a taskbar
    order: Vec<String>,
    scanned: Option<Instant>,
    pub volume: Option<(f32, bool)>,
    /// what each program uses (by its path), looked at about once a second
    usage: std::collections::HashMap<String, crate::procstats::Use>,
    used_at: Option<Instant>,
    /// which programs make sound (their sessions, looked for once a second)
    pub audio: crate::appaudio::AppAudio,
    /// windows that flash for you (Windows says so), till they come to the front
    pub flashing: std::collections::HashSet<isize>,
    /// Win held: the dock shows its numbers
    pub numbers: bool,
    /// the PC's small things (microphone and camera, USB drives, languages, desktops)
    pub sys: crate::sysw::State,
    /// the network (Wi-Fi in reach, Bluetooth paired, online), and the radios as last drawn
    pub net: crate::net::State,
    pub radios: crate::radios::States,
    /// the network asking for its password now (its field under its row), and a field to open by itself
    pub net_ask: Option<String>,
    /// where the next new desktop goes on the map (its column and row), when one was asked for at a place
    pub desk_pending: Option<(i32, i32)>,
    pub focus: Option<String>,
    /// the sound's devices (outputs, microphones), the microphone's volume, each program's own (once a second)
    outputs: Vec<crate::audiodev::Device>,
    inputs: Vec<crate::audiodev::Device>,
    mic: Option<(f32, bool)>,
    app_volumes: Vec<crate::appaudio::AppVolume>,
    sounding: std::collections::HashSet<String>,
    /// how many times the programs were looked at (some things are looked at every few)
    looks: u32,
    /// each running program's share of the processor and of the memory, the last minute or so (once a second)
    history: std::collections::HashMap<String, std::collections::VecDeque<(f32, f32)>>,
}

impl Builtins {
    pub fn new() -> Builtins {
        Builtins { windows: Vec::new(), order: Vec::new(), scanned: None, volume: crate::volume::get(), usage: Default::default(), used_at: None, audio: crate::appaudio::AppAudio::new(), sounding: Default::default(), looks: 0, history: Default::default(), flashing: Default::default(), numbers: false, sys: Default::default(), net: Default::default(), radios: Default::default(), net_ask: None, desk_pending: None, focus: None, outputs: Vec::new(), inputs: Vec::new(), mic: None, app_volumes: Vec::new() }
    }

    /// Look at the windows again (at most twice a second); true when what the dock shows changed.
    pub fn scan(&mut self, force: bool) -> bool {
        if !force && self.scanned.is_some_and(|t| t.elapsed() < Duration::from_millis(500)) {
            return false;
        }
        self.scanned = Some(Instant::now());
        let now = winlist::app_windows();
        for w in &now {
            if !w.exe.is_empty() && !self.order.contains(&w.exe) {
                self.order.push(w.exe.clone());
            }
        }
        self.order.retain(|k| now.iter().any(|w| &w.exe == k));
        let vol = crate::volume::get();
        let mut changed = now != self.windows || vol != self.volume;
        // (what the programs use: once a second; a change that shows on the dock is a change)
        if self.used_at.map_or(true, |t| t.elapsed() >= Duration::from_millis(1000)) {
            self.used_at = Some(Instant::now());
            crate::procstats::start();
            let usage = crate::procstats::latest().unwrap_or_else(|| self.usage.clone());
            let shows = |a: &crate::procstats::Use, b: &crate::procstats::Use| (a.cpu - b.cpu).abs() > 0.01 || a.mem.abs_diff(b.mem) > a.mem / 20;
            if now.iter().any(|w| match (usage.get(&w.exe), self.usage.get(&w.exe)) {
                (Some(a), Some(b)) => shows(a, b),
                (a, b) => a.is_some() != b.is_some(),
            }) {
                changed = true;
            }
            let total = crate::procstats::total_memory().max(1) as f64;
            // (once each program, however many windows it has)
            let programs: std::collections::HashSet<&String> = now.iter().map(|w| &w.exe).collect();
            for exe in programs {
                let h = self.history.entry(exe.clone()).or_default();
                h.push_back(usage.get(exe).map_or((0.0, 0.0), |u| (u.cpu, (u.mem as f64 / total) as f32)));
                while h.len() > 60 {
                    h.pop_front();
                }
            }
            self.history.retain(|k, _| now.iter().any(|w| &w.exe == k));
            self.usage = usage;
            // (who makes sound: its sessions found again, every other look; one that starts or stops is a change)
            self.looks = self.looks.wrapping_add(1);
            if self.looks % 2 == 0 {
                self.audio.refresh();
            }
            let sounding: std::collections::HashSet<String> = self.audio.peaks().into_iter().filter(|(_, v)| *v > 0.005).map(|(k, _)| k).collect();
            if sounding != self.sounding {
                changed = true;
                self.sounding = sounding;
            }
            // (the sound's devices, the microphone, each program's volume: quick settings shows them)
            // (the devices change seldom: every few looks)
            let fresh = self.looks % 4 == 1;
            let outputs = if fresh { crate::audiodev::list(windows::Win32::Media::Audio::eRender) } else { self.outputs.clone() };
            let inputs = if fresh { crate::audiodev::list(windows::Win32::Media::Audio::eCapture) } else { self.inputs.clone() };
            let mic = if fresh { crate::audiodev::mic_get() } else { self.mic };
            let app_volumes = self.audio.apps();
            if outputs != self.outputs || inputs != self.inputs || mic != self.mic || app_volumes != self.app_volumes {
                changed = true;
                self.outputs = outputs;
                self.inputs = inputs;
                self.mic = mic;
                self.app_volumes = app_volumes;
            }
        }
        self.windows = now;
        self.volume = vol;
        changed
    }

    /// The apps of the dock: the pinned ones first, in their order, then the others that are open, in the order they came.
    pub fn dock_apps(&self, cfg: &Config) -> Vec<DockApp> {
        let mut out: Vec<DockApp> = Vec::new();
        for p in &cfg.pinned {
            // (a shortcut: its windows are its program's; opened, the shortcut, with what it starts with)
            let key = if p.id.to_lowercase().ends_with(".lnk") { crate::explorer::shortcut_program_kept(&p.id).unwrap_or_else(|| p.id.clone()).to_lowercase() } else { p.id.to_lowercase() };
            let windows: Vec<AppWindow> = self.windows.iter().filter(|w| w.exe == key).cloned().collect();
            // (a program: its icon; a file or a folder pinned from File Explorer: the icon Explorer shows for it)
            let image = if !p.icon.is_empty() {
                format!("img:{}", p.icon)
            } else if p.id.to_lowercase().ends_with(".exe") {
                format!("exe:{}", p.id)
            } else if p.id.contains(":\\") {
                format!("shell:{}", p.id)
            } else {
                format!("app:{}", p.id)
            };
            out.push(DockApp { key, name: p.name.clone(), image, pinned: true, windows, open: p.id.clone() });
        }
        for k in &self.order {
            if out.iter().any(|a| &a.key == k) {
                continue;
            }
            let windows: Vec<AppWindow> = self.windows.iter().filter(|w| &w.exe == k).cloned().collect();
            if windows.is_empty() {
                continue;
            }
            out.push(DockApp { key: k.clone(), name: winlist::app_name(k), image: format!("exe:{k}"), pinned: false, windows, open: k.clone() });
        }
        out
    }

    /// The dock, as a widget. Each running app: what it uses (its share of the processor, of the memory), how far what it
    /// downloads is, and the sound it makes. Open, its icons become their windows: each window
    /// its live picture, an app pinned with none its picture; the launcher first.
    pub fn dock(&self, cfg: &Config, pc: &Arc<Pc>) -> Value {
        let apps = self.dock_apps(cfg);
        let used = |k: &str| self.usage.get(k).copied().unwrap_or_default();
        let total = crate::procstats::total_memory().max(1) as f64;
        let progress = downloads_by_program(pc);
        let items: Vec<Value> = apps
            .iter()
            .map(|a| {
                let u = used(&a.key);
                let file = a.key.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                let running = !a.windows.is_empty();
                let in_apps = cfg.launcher_pins.as_ref().is_some_and(|l| l.iter().any(|p| p.kind == "app" && p.id.eq_ignore_ascii_case(&a.key)));
                let mut menu = vec![
                    json!({ "icon": "window-new", "label": if running { "New Window" } else { "Open" }, "action": format!("launch:{}", a.open) }),
                    json!({ "icon": if a.pinned { "unpin" } else { "pin" }, "label": if a.pinned { "Unpin" } else { "Pin" }, "action": format!("pin:{}", a.open) }),
                    json!({ "icon": "grid", "label": if in_apps { "Unpin from Apps" } else { "Pin to Apps" }, "action": format!("apppin:{}", a.key) }),
                ];
                if std::path::Path::new(&a.key).is_file() {
                    menu.push(json!({ "icon": "folder", "label": "Reveal in File Explorer", "action": format!("reveal:{}", a.key) }));
                }
                // (pinned: its name and its picture made one's own)
                if a.pinned {
                    menu.push(json!({ "icon": "pen", "label": "Rename", "action": format!("pinrename:dock:{}", a.open) }));
                    menu.push(json!({ "icon": "image", "label": "Change Icon", "action": format!("pinicon:dock:{}", a.open) }));
                    if a.image.starts_with("img:") {
                        menu.push(json!({ "icon": "reset", "label": "Reset Icon", "action": format!("pinreset:dock:{}", a.open) }));
                    }
                }
                // (what its window works on: its folder, or its document, when it can be found)
                if let Some(w) = a.windows.iter().find(|w| w.front).or(a.windows.first()) {
                    if let Some(item) = context_item(w) {
                        menu.push(item);
                    }
                }
                // (only an open one can be closed)
                if running {
                    menu.push(json!({ "icon": "x", "label": if a.windows.len() > 1 { "Close All" } else { "Close" }, "action": format!("closeall:{}", a.key), "hold": true, "tone": "no" }));
                }
                let front_i = {
                    let mut hw: Vec<isize> = a.windows.iter().map(|w| w.hwnd).collect();
                    hw.sort();
                    a.windows.iter().find(|w| w.front).and_then(|f| hw.iter().position(|h| *h == f.hwnd))
                };
                json!({
                    "key": a.key, "image": a.image, "name": a.name, "windows": a.windows.len(), "front": a.windows.iter().any(|w| w.front),
                    "cpu": if running { json!(u.cpu) } else { Value::Null },
                    "ram": if running { json!((u.mem as f64 / total) as f32) } else { Value::Null },
                    "progress": progress.get(&file).copied(),
                    "sound": self.sounding.contains(&a.key),
                    "attention": a.windows.iter().any(|w| self.flashing.contains(&w.hwnd)),
                    // (its right-click: new window, pin, its file in Explorer (a program on disk, not a store app's), close all (held))
                    "menu": menu,
                    // (the app in front: its window, and what it used lately, for the dock at its largest)
                    "front_hwnd": a.windows.iter().find(|w| w.front).map(|w| w.hwnd),
                    // (which of its windows is in front, in the order its dots are in (none: none of them))
                    "front_i": front_i,
                    // (what you are in, in it: its front window's title, less the app's own name)
                    "title": a.windows.iter().find(|w| w.front).map(|w| short_title(&w.title, &a.name)),
                    "hist": if a.windows.iter().any(|w| w.front) { json!(self.history.get(&a.key).map(|h| h.iter().map(|(c, r)| [*c, *r]).collect::<Vec<[f32; 2]>>()).unwrap_or_default()) } else { Value::Null },
                    "mem_text": if running { json!(mem_text(u.mem)) } else { Value::Null },
                })
            })
            .collect();
        let mut tiles: Vec<Value> = Vec::new();
        let mut groups: Vec<Value> = Vec::new();
        let mut pins: Vec<Value> = Vec::new();
        for a in &apps {
            if a.windows.is_empty() {
                pins.push(json!({ "image": a.image, "name": a.name, "action": format!("launch:{}", a.key) }));
                continue;
            }
            let first = tiles.len();
            for w in &a.windows {
                // (a window put away has no picture to show: its app's, in its place)
                let title = if w.title.is_empty() { a.name.clone() } else { w.title.clone() };
                let u = used(&a.key);
                // (its menu: close it, pin its app, end its task; ending is held)
                let in_apps = cfg.launcher_pins.as_ref().is_some_and(|l| l.iter().any(|p| p.kind == "app" && p.id.eq_ignore_ascii_case(&a.key)));
                let mut menu = vec![
                    json!({ "icon": "x", "label": "Close", "action": format!("close:{}", w.hwnd) }),
                    json!({ "icon": if a.pinned { "unpin" } else { "pin" }, "label": if a.pinned { "Unpin" } else { "Pin" }, "action": format!("pin:{}", a.open) }),
                    json!({ "icon": "grid", "label": if in_apps { "Unpin from Apps" } else { "Pin to Apps" }, "action": format!("apppin:{}", a.key) }),
                ];
                if std::path::Path::new(&a.key).is_file() {
                    menu.push(json!({ "icon": "folder", "label": "Reveal in File Explorer", "action": format!("reveal:{}", a.key) }));
                }
                // (pinned: its name and its picture made one's own)
                if a.pinned {
                    menu.push(json!({ "icon": "pen", "label": "Rename", "action": format!("pinrename:dock:{}", a.open) }));
                    menu.push(json!({ "icon": "image", "label": "Change Icon", "action": format!("pinicon:dock:{}", a.open) }));
                    if a.image.starts_with("img:") {
                        menu.push(json!({ "icon": "reset", "label": "Reset Icon", "action": format!("pinreset:dock:{}", a.open) }));
                    }
                }
                if let Some(item) = context_item(w) {
                    menu.push(item);
                }
                menu.push(json!({ "icon": "power", "label": "End Task", "action": format!("kill:{}", w.hwnd), "hold": true, "tone": "no", "confirm": "What it has not saved will be lost." }));
                let file = a.key.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                tiles.push(json!({ "key": a.key, "hwnd": if w.minimized { 0 } else { w.hwnd }, "image": a.image, "title": title, "short": short_title(&title, &a.name), "action": format!("win:{}", w.hwnd), "front": w.front, "cpu": u.cpu, "ram": (u.mem as f64 / total) as f32, "progress": progress.get(&file).copied(), "sound": self.sounding.contains(&a.key), "attention": self.flashing.contains(&w.hwnd), "menu": menu }));
            }
            groups.push(json!({ "name": a.name, "image": a.image, "windows": tiles[first..].to_vec() }));
        }
        // (open: each app its name over its windows, as wide as they are side by side, the apps in rows; its pinned ones closed at
        // its foot)
        let per = |g: &Value| {
            let n = g["windows"].as_array().map_or(1, |w| w.len()) as f32;
            n * crate::blocks::APPWIN_W + (n - 1.0) * 8.0
        };
        let total: f32 = groups.iter().map(per).sum::<f32>() + (groups.len().max(1) - 1) as f32 * 16.0;
        // (the windows at their own size; the panel as wide as they are)
        // (and room at both sides for the ring round the window you were in)
        let width = (2.0 * 16.0 + 6.0 + total).clamp(200.0, 940.0);
        json!({
            "peek": { "dock": { "items": items, "numbers": self.numbers } },
            "panel": { "width": width, "body": [{ "kind": "appwindows", "groups": groups, "pins": pins }] },
            "tiles": tiles,
            "tip": "Apps",
        })
    }

    /// Quick settings, as a widget.
    pub fn quick(&self) -> Value {
        let (vol, muted) = self.volume.unwrap_or((0.0, true));
        let pct = (vol * 100.0).round() as i32;
        let vol_icon = if muted || vol <= 0.0 { "volume-x" } else { "volume" };
        let headset = self.outputs.iter().find(|d| d.on).is_some_and(|d| crate::audiodev::is_headset(&d.name));
        let slider_icon = if muted || vol <= 0.0 { "volume-x" } else if headset { "headphones" } else { "volume" };
        let pc = |v: f32| format!("{}%", (v * 100.0).round() as i32);
        let peaks = self.audio.peaks();
        let playing = |k: &str| peaks.get(k).is_some_and(|v| *v > 0.001);
        let mut body = Vec::new();
        // (where the sound goes: each output a chip, the one used filled)
        if self.outputs.len() > 1 {
            let chips: Vec<Value> = self.outputs.iter().map(|d| json!({ "icon": if crate::audiodev::is_headset(&d.name) { "headphones" } else { "speaker" }, "text": d.name, "on": d.on, "action": format!("out:{}", d.id) })).collect();
            body.push(json!({ "kind": "chips", "items": chips }));
        }
        body.push(json!({ "kind": "slider", "icon": slider_icon, "value": vol, "muted": muted, "text": format!("{pct}%"), "action": "volume", "press": "mute", "meter": "master" }));
        // (each program that played lately, the ones playing first: its own volume, its name over it)
        let mut apps: Vec<&crate::appaudio::AppVolume> = self.app_volumes.iter().collect();
        apps.sort_by_key(|a| !playing(&a.key));
        if !apps.is_empty() {
            body.push(json!({ "kind": "rule" }));
        }
        for a in apps.into_iter().take(6) {
            body.push(json!({ "kind": "slider", "image": format!("exe:{}", a.key), "name": crate::winlist::app_name(&a.key), "playing": playing(&a.key), "muted": a.muted, "value": a.volume, "text": pc(a.volume), "action": format!("appvol:{}", a.key), "press": format!("appmute:{}", a.key), "meter": format!("app:{}", a.key) }));
        }
        // (the microphones, and the one used: its volume)
        if self.mic.is_some() || self.inputs.len() > 1 {
            body.push(json!({ "kind": "rule" }));
        }
        if self.inputs.len() > 1 {
            let chips: Vec<Value> = self.inputs.iter().map(|d| json!({ "icon": "mic", "text": d.name, "on": d.on, "action": format!("in:{}", d.id) })).collect();
            body.push(json!({ "kind": "chips", "items": chips }));
        }
        if let Some((mv, mm)) = self.mic {
            body.push(json!({ "kind": "slider", "icon": "mic", "value": mv, "muted": mm, "text": pc(mv), "action": "micvol", "press": "micmute", "meter": "mic" }));
        }
        body.push(json!({ "kind": "buttons", "items": [{ "icon": "gear", "label": "Sound Settings", "action": "sound_settings" }] }));
        let any = peaks.values().any(|v| *v > 0.001);
        json!({
            // (at rest: its icon and a small line as long as its volume, how loud it is now faint over it; muted, its icon alone, red)
            "peek": { "icon": vol_icon, "level": { "value": vol, "muted": muted, "meter": "master", "playing": any && !muted }, "label": format!("Volume {pct}%") },
            "panel": { "body": body },
            "tip": if muted { "Muted".to_string() } else { format!("Volume {pct}%") },
        })
    }

    /// A press on one of them.
    pub fn press(&mut self, cfg: &mut Config, action: &str, args: &Value) -> (Effect, bool) {
        // (lock, sleep, restart, shut down: asked first, in a modal)
        if let Some(what) = action.strip_prefix("power:") {
            power(what);
            return (Effect::None, false);
        }
        let apps = self.dock_apps(cfg);
        let find = |k: &str| apps.iter().find(|a| a.key == k).cloned();
        // (the second value: the config changed, and is kept)
        if action == "launcher" {
            return (Effect::Launcher, false);
        }
        // (what a window works on: its folder opened, or its document shown)
        if let Some(h) = action.strip_prefix("ctx:").and_then(|h| h.parse::<isize>().ok()) {
            if let Some(w) = apps.iter().flat_map(|a| a.windows.iter()).find(|w| w.hwnd == h) {
                if let Some(c) = crate::context::of(w.hwnd, &w.exe, &w.title) {
                    crate::context::open(&c);
                }
            }
            return (Effect::None, false);
        }
        // (its program's file, chosen in a File Explorer window)
        if let Some(path) = action.strip_prefix("reveal:") {
            if std::path::Path::new(path).is_file() {
                // (as Explorer reads it: /select, then the path in quotes of its own (a path with spaces is otherwise not found, and
                // Explorer opens Documents))
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn();
            }
            return (Effect::None, false);
        }
        // (the Now Card's cover: what plays there, paused or played)
        if let Some(source) = action.strip_prefix("mediatoggle:") {
            nadi_core::sensors::media::media_play_pause(Some(source.to_string()));
            return (Effect::None, false);
        }
        // a tray icon: its app hears the click, as from the taskbar
        if let Some(rest) = action.strip_prefix("tray:") {
            if let Some((owner, uid)) = rest.split_once(':') {
                let mut pt = windows::Win32::Foundation::POINT::default();
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
                }
                crate::trayhost::click(owner.parse().unwrap_or(0), uid.parse().unwrap_or(0), crate::trayhost::Click::Left, pt.x, pt.y);
            }
            return (Effect::None, false);
        }
        if let Some(i) = action.strip_prefix("app:").and_then(|i| i.parse::<usize>().ok()) {
            let Some(a) = apps.get(i) else { return (Effect::None, false) };
            for w in &a.windows {
                self.flashing.remove(&w.hwnd);
            }
            match a.windows.len() {
                0 => apps::launch(&a.open),
                // one: in front, it goes down; else it comes up
                1 if a.windows[0].front => winlist::minimize(a.windows[0].hwnd),
                1 => winlist::activate(a.windows[0].hwnd),
                // several: one in front, the next of them in turn (each in its turn, all of them); none, the one used last
                _ => {
                    let mut hw: Vec<isize> = a.windows.iter().map(|w| w.hwnd).collect();
                    hw.sort();
                    match a.windows.iter().find(|w| w.front).and_then(|f| hw.iter().position(|h| *h == f.hwnd)) {
                        Some(i) => winlist::activate(hw[(i + 1) % hw.len()]),
                        None => winlist::activate(a.windows[0].hwnd),
                    }
                }
            }
            self.scanned = None;
            return (Effect::Changed, false);
        }
        // (the wheel over an app: its windows one after another, up or down)
        if let Some((i, dir)) = action.strip_prefix("appwheel:").and_then(|r| r.split_once(':')).and_then(|(i, d)| Some((i.parse::<usize>().ok()?, d == "up"))) {
            let Some(a) = apps.get(i) else { return (Effect::None, false) };
            let mut hw: Vec<isize> = a.windows.iter().map(|w| w.hwnd).collect();
            hw.sort();
            if !hw.is_empty() {
                let at = a.windows.iter().find(|w| w.front).and_then(|f| hw.iter().position(|h| *h == f.hwnd));
                let n = hw.len();
                let next = match at {
                    Some(i) if dir => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                    None => 0,
                };
                winlist::activate(hw[next]);
                self.scanned = None;
                return (Effect::Changed, false);
            }
            return (Effect::None, false);
        }
        if let Some(h) = action.strip_prefix("win:").and_then(|h| h.parse::<isize>().ok()) {
            winlist::activate(h);
            self.scanned = None;
            return (Effect::Changed, false);
        }
        if let Some(h) = action.strip_prefix("close:").and_then(|h| h.parse::<isize>().ok()) {
            winlist::close(h);
            self.scanned = None;
            return (Effect::Changed, false);
        }
        if let Some(h) = action.strip_prefix("kill:").and_then(|h| h.parse::<isize>().ok()) {
            crate::procstats::end_task(h);
            self.scanned = None;
            return (Effect::Changed, false);
        }
        if let Some(k) = action.strip_prefix("closeall:") {
            if let Some(a) = find(k) {
                for w in &a.windows {
                    winlist::close(w.hwnd);
                }
            }
            return (Effect::Changed, false);
        }
        // a USB drive: opened, ejected (on a thread of its own: its files may take a moment to let go)
        if let Some(l) = action.strip_prefix("opendrive:").and_then(|l| l.chars().next()) {
            crate::sysw::open_drive(l);
            return (Effect::None, false);
        }
        if let Some(l) = action.strip_prefix("driveterm:").and_then(|l| l.chars().next()) {
            crate::sysw::drive_terminal(l);
            return (Effect::None, false);
        }
        if let Some(l) = action.strip_prefix("driveprops:").and_then(|l| l.chars().next()) {
            crate::sysw::drive_properties(l);
            return (Effect::None, false);
        }
        if let Some(l) = action.strip_prefix("eject:").and_then(|l| l.chars().next()) {
            std::thread::spawn(move || crate::sysw::eject(l));
            return (Effect::None, false);
        }
        // a desktop shown, or a new one made; a language to type in
        // (the map: a card dropped at a place (one there swaps with it); a new desktop at a place; the map to a place)
        if let Some((id, c, r)) = action.strip_prefix("deskmove:").and_then(|x| {
            let mut p = x.rsplitn(3, ':');
            let r = p.next()?.parse::<i32>().ok()?;
            let c = p.next()?.parse::<i32>().ok()?;
            Some((p.next()?.to_string(), c, r))
        }) {
            let from = cfg.desk_map.get(&id).copied();
            if let Some(other) = cfg.desk_map.iter().find(|(k, v)| **v == (c, r) && **k != id).map(|(k, _)| k.clone()) {
                if let Some(f) = from {
                    cfg.desk_map.insert(other, f);
                }
            }
            cfg.desk_map.insert(id, (c, r));
            return (Effect::Changed, true);
        }
        if let Some((c, r)) = action.strip_prefix("desknew:").and_then(|x| x.split_once(':')).and_then(|(c, r)| Some((c.parse::<i32>().ok()?, r.parse::<i32>().ok()?))) {
            self.desk_pending = Some((c, r));
            crate::sysw::new_desk();
            return (Effect::None, false);
        }
        if let Some((c, r)) = action.strip_prefix("deskgo:").and_then(|x| x.split_once(':')).and_then(|(c, r)| Some((c.parse::<i32>().ok()?, r.parse::<i32>().ok()?))) {
            crate::blocks::desk_go(c, r);
            return (Effect::None, false);
        }
        if action == "desk:new" {
            crate::sysw::new_desk();
            return (Effect::None, false);
        }
        if let Some((i, h)) = action.strip_prefix("deskwin:").and_then(|r| r.split_once(':')).and_then(|(i, h)| Some((i.parse::<usize>().ok()?, h.parse::<isize>().ok()?))) {
            crate::sysw::show_window_on(self.sys.desk, i, h);
            return (Effect::None, false);
        }
        if let Some(i) = action.strip_prefix("deskclose:").and_then(|i| i.parse::<usize>().ok()) {
            crate::sysw::close_desk(self.sys.desk, i);
            return (Effect::None, false);
        }
        if let Some(i) = action.strip_prefix("desk:").and_then(|i| i.parse::<usize>().ok()) {
            crate::sysw::switch_desk(self.sys.desk, i);
            return (Effect::None, false);
        }
        // the sound: an output, a microphone made the one used; a program's volume; the microphone's
        if let Some(id) = action.strip_prefix("out:").or_else(|| action.strip_prefix("in:")) {
            crate::audiodev::use_device(id);
            self.outputs = crate::audiodev::list(windows::Win32::Media::Audio::eRender);
            self.inputs = crate::audiodev::list(windows::Win32::Media::Audio::eCapture);
            self.used_at = None;
            self.scanned = None;
            return (Effect::Changed, false);
        }
        if let Some(k) = action.strip_prefix("appvol:") {
            self.audio.set_volume(k, args["fraction"].as_f64().unwrap_or(0.0) as f32);
            // (set while muted: heard again, as Windows' own does)
            if self.app_volumes.iter().any(|a| a.key == k && a.muted) {
                self.audio.toggle_mute(k);
            }
            self.app_volumes = self.audio.apps();
            return (Effect::Changed, false);
        }
        if let Some(k) = action.strip_prefix("appmute:") {
            self.audio.toggle_mute(k);
            self.app_volumes = self.audio.apps();
            return (Effect::Changed, false);
        }
        if action == "micvol" {
            crate::audiodev::mic_set(args["fraction"].as_f64().unwrap_or(0.0) as f32);
            if self.mic.is_some_and(|(_, m)| m) {
                crate::audiodev::mic_mute();
            }
            self.mic = crate::audiodev::mic_get();
            return (Effect::Changed, false);
        }
        if action == "micmute" {
            crate::audiodev::mic_mute();
            self.mic = crate::audiodev::mic_get();
            return (Effect::Changed, false);
        }
        // the hotspot on or off; its password copied
        if action == "hotspot" {
            let on = self.net.hotspot.as_ref().is_some_and(|h| h.on);
            if let Some(h) = self.net.hotspot.as_mut() {
                h.on = !on;
                h.busy = true;
            }
            crate::hotspot::set(!on);
            return (Effect::Changed, false);
        }
        if action == "hotspotcopy" {
            if let Some(h) = &self.net.hotspot {
                crate::clip::copy_text(&h.pass);
            }
            return (Effect::None, false);
        }
        // the network: a radio on or off (airplane: all off, or all on again); a network joined; Windows' own pages
        if let Some(r) = action.strip_prefix("radio:") {
            let st = crate::radios::now();
            match r {
                "wifi" => crate::radios::set("wifi", st.wifi != Some(true)),
                "bluetooth" => crate::radios::set("bluetooth", st.bluetooth != Some(true)),
                _ => {
                    let on = st.wifi != Some(true) && st.bluetooth != Some(true);
                    crate::radios::set("wifi", on);
                    crate::radios::set("bluetooth", on);
                }
            }
            crate::net::poke();
            return (Effect::Changed, false);
        }
        // (a network pressed: a saved one, or one with no password, joined at once; one with a password asks for it, under its row; one
        // that signs in another way (a work network) opens Windows' Wi-Fi settings. Pressed again while it asks: it stops asking)
        if let Some(ssid) = action.strip_prefix("wifi:") {
            let Some(w) = self.net.wifi.iter().find(|w| w.ssid == ssid).cloned() else { return (Effect::None, false) };
            if self.net_ask.as_deref() == Some(ssid) {
                self.net_ask = None;
                return (Effect::Changed, false);
            }
            self.net_ask = None;
            match w.auth {
                _ if w.known => crate::net::join(ssid),
                crate::net::Auth::Open => crate::net::join_new(&w, None),
                crate::net::Auth::Key(..) => {
                    self.net_ask = Some(ssid.to_string());
                    self.focus = Some(format!("wifipass:{ssid}"));
                }
                crate::net::Auth::Other => apps::launch_uri("ms-settings:network-wifi"),
            }
            return (Effect::Changed, false);
        }
        if let Some(ssid) = action.strip_prefix("wifipass:") {
            let pass = args["text"].as_str().unwrap_or("").to_string();
            if let Some(w) = self.net.wifi.iter().find(|w| w.ssid == ssid).cloned().filter(|_| !pass.is_empty()) {
                self.net_ask = None;
                crate::net::join_new(&w, Some(pass));
            }
            return (Effect::Changed, false);
        }
        if action == "btsettings" {
            apps::launch_uri("ms-settings:bluetooth");
            return (Effect::None, false);
        }
        if action == "netsettings" {
            apps::launch_uri("ms-settings:network");
            return (Effect::None, false);
        }
        if action == "lang" {
            if let Some(h) = args["value"].as_str().and_then(|v| v.parse::<isize>().ok()) {
                crate::sysw::switch_lang(h);
            }
            return (Effect::None, false);
        }
        if let Some(k) = action.strip_prefix("launch:") {
            apps::launch(k);
            return (Effect::None, false);
        }
        // (an app pinned to the launcher, or taken off it)
        if let Some(k) = action.strip_prefix("apppin:") {
            let pins = cfg.launcher_pins.get_or_insert_with(crate::launcher::default_pins);
            if let Some(i) = pins.iter().position(|p| p.kind == "app" && p.id.eq_ignore_ascii_case(k)) {
                pins.remove(i);
            } else if let Some(a) = find(k) {
                pins.push(crate::config::AppPin { kind: "app".into(), id: a.key.clone(), name: a.name.clone(), folder: String::new(), icon: String::new() });
            }
            return (Effect::Changed, true);
        }
        if let Some(k) = action.strip_prefix("pin:") {
            if let Some(i) = cfg.pinned.iter().position(|p| p.id.eq_ignore_ascii_case(k)) {
                cfg.pinned.remove(i);
            } else if let Some(a) = find(k) {
                cfg.pinned.push(Pin { name: a.name.clone(), id: a.key.clone(), icon: String::new() });
            }
            return (Effect::Changed, true);
        }
        match action {
            "volume" => {
                let f = args["fraction"].as_f64().unwrap_or(0.0) as f32;
                crate::volume::set(f);
                if self.volume.is_some_and(|(_, m)| m) {
                    crate::volume::toggle_mute();
                }
                self.volume = crate::volume::get();
                (Effect::Changed, false)
            }
            "mute" => {
                crate::volume::toggle_mute();
                self.volume = crate::volume::get();
                (Effect::Changed, false)
            }
            "settings" => {
                apps::launch_uri("ms-settings:");
                (Effect::None, false)
            }
            "sound_settings" => {
                apps::launch_uri("ms-settings:sound");
                (Effect::None, false)
            }
            "tray" => {
                crate::taskbar::peek();
                (Effect::None, false)
            }
            "nadi_settings" => (Effect::Settings, false),
            p if p.starts_with("power:") => {
                power(&p[6..]);
                (Effect::None, false)
            }
            _ => (Effect::None, false),
        }
    }

    /// Turn the volume up or down a step (the wheel over quick settings).
    pub fn nudge_volume(&mut self, up: bool) {
        let (v, _) = self.volume.unwrap_or((0.0, false));
        crate::volume::set(v + if up { 0.02 } else { -0.02 });
        self.volume = crate::volume::get();
    }
}

/// Lock, sleep, restart or shut down the PC.
pub fn power(what: &str) {
    match what {
        "lock" => unsafe {
            let _ = windows::Win32::System::Shutdown::LockWorkStation();
        },
        "sleep" => unsafe {
            let _ = windows::Win32::System::Power::SetSuspendState(false, false, false);
        },
        "restart" => {
            let _ = std::process::Command::new("shutdown").args(["/r", "/t", "0"]).spawn();
        }
        "shutdown" => {
            let _ = std::process::Command::new("shutdown").args(["/s", "/t", "0"]).spawn();
        }
        _ => {}
    }
}
