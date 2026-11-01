//! How the person set the frame up (`%APPDATA%\NADI\nadi-frame.json`): where each widget goes and how it looks, and which monitors have a
//! frame. A widget the file does not name goes where its manifest says. The file is read again when it changes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct WidgetConfig {
    /// "top", "left", "top-right", ...; "off": not shown
    pub place: Option<String>,
    pub rank: Option<i32>,
    /// "merged" (into the edge, the default) or "floating" (a piece of its own, a little off the edge)
    pub style: Option<String>,
    /// in a corner: "down" (in the column beside the corner, along its side edge) or none (in the row, along its top or bottom edge)
    pub stack: Option<String>,
    /// pinned to a monitor (its device name, `\\.\DISPLAY2`): shown on that one only (when the frame is there, or always, below)
    pub monitor: Option<String>,
    /// always out: not slid away as "reveal" says, nor for a maximized window (a full-screen app still sends it away); pinned to a
    /// monitor, shown there even when the frame is on another
    pub always_shown: Option<bool>,
    /// always out over a full-screen app too (a game, a video)
    pub over_full: Option<bool>,
    /// its panel always open
    pub always_open: Option<bool>,
    /// shown even while its plugin says it is idle (nothing going on); by default an idle widget is hidden
    pub show_idle: Option<bool>,
    /// it comes out of its edge for a moment when something worth a glance happens (none: as it is made to; the tray: off)
    pub bump: Option<bool>,
    /// at rest, its icon alone (none: as its manifest says)
    pub icon_only: Option<bool>,
    /// "small" or "large" (none: as its manifest says)
    pub size: Option<String>,
    /// joined to this widget (its key), the one before it on its edge (dragged against it in Edit Mode): no gap between them, so
    /// the liquid runs them together; only while that one is just before it (anything else read: none)
    #[serde(deserialize_with = "join_key")]
    pub join: Option<String>,
    /// its panel popped out as a window: its background ("see": see-through, "none"; none: solid)
    pub pop_bg: Option<String>,
    /// where its window stood last, on each monitor (by name): its top left, logical
    pub pop_at: Option<BTreeMap<String, (f32, f32)>>,
    /// its panel made bigger or smaller by hand: its width and height on its edge, and as a window (none: the size it asks for)
    pub panel_size: Option<(f32, f32)>,
    pub pop_size: Option<(f32, f32)>,
}

/// A thing pinned to the launcher: an app, a file, a page of Settings, one of the PC's own tools; in a folder of pins, or not.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct AppPin {
    /// "app" (a program's path or a store app's id), "file" (its path), "setting" (its address), "sys" (a tool's name)
    pub kind: String,
    pub id: String,
    pub name: String,
    /// the folder of pins it is in (none: loose)
    pub folder: String,
}

/// An app pinned to the dock.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Pin {
    pub name: String,
    /// the program's path, or a store app's id
    pub id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// by "plugin/widget"
    pub widgets: BTreeMap<String, WidgetConfig>,
    /// "follow" (the frame is on the monitor the mouse is on), "primary", or "all"
    pub monitors: String,
    /// "keep" the Windows taskbar (the frame stays clear of it), keep it "under" the widgets (they stand at the screen's edge over
    /// it), or "hide" it
    pub taskbar: String,
    /// the frame's size, times the monitor's own scale
    pub scale: f32,
    /// when the widgets get out of the way: "never", "fullscreen" (an app is full screen on that monitor: a game, a video), "maximized"
    /// (also while a maximized window is in front). Out of the way, they slide into their edges and come out as `reveal` says (with
    /// "all": when the mouse comes to any edge).
    pub auto_hide: String,
    pub pinned: Vec<Pin>,
    /// the launcher's pins, in their order (none: the ones it starts with)
    pub launcher_pins: Option<Vec<AppPin>>,
    /// the launcher's foot: pins of its own, small, in a row (none: lock, sleep, restart, shut down)
    pub launcher_foot: Option<Vec<AppPin>>,
    /// the Windows key: "auto" (the launcher while the taskbar is hidden, else Start), "launcher", "start"
    pub win_key: String,
    /// which widgets show: "all" (always), "edges" (all of them, when the mouse comes to any edge), "edge" (the edge the mouse comes
    /// to), "near" (the ones near the mouse)
    pub reveal: String,
    /// while out of the way: a key held for the mouse at an edge to bring them out ("none": the edge is enough, "alt", "ctrl")
    pub hold_key: String,
    /// how a widget's panel opens: "hover" (the mouse rests on it; it shuts when the mouse leaves) or "click" (a click opens it; a
    /// click anywhere else shuts it)
    pub open_on: String,
    /// the tray icons in the Tray widget's row, in order (by program and number)
    pub tray_pinned: Vec<String>,
    /// the small banners wait while something full screen is in front (a game, a video, a talk), as they do while a plugin asks
    /// for quiet
    pub quiet_full: bool,
    /// the plugins whose banners come out while it is quiet too (by id)
    pub quiet_banners: Vec<String>,
    /// the widgets and their edge as one liquid, how much: 0 (none) to 100 (anything else read: the middle, so the rest of the
    /// config still reads)
    #[serde(deserialize_with = "liquid_amount")]
    pub liquid: f32,
    /// the theme colour, "#rrggbb": what is lit or chosen, and a glow that names no colour
    pub theme: String,
    /// the theme's second colour: what waits for you (something elsewhere, a bump, a widget asking to be seen)
    pub theme2: String,
    /// the desktops' map: where each desktop is (by its id), as columns and rows; and whether Win + Ctrl + arrows follow it
    pub desk_map: BTreeMap<String, (i32, i32)>,
    pub desk_keys: bool,
}

impl Config {
    /// Does the Windows key open the launcher?
    pub fn win_launcher(&self) -> bool {
        match self.win_key.as_str() {
            "launcher" => true,
            "start" => false,
            _ => self.taskbar == "hide",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { widgets: BTreeMap::new(), monitors: "follow".into(), taskbar: "keep".into(), scale: 1.0, auto_hide: "fullscreen".into(), pinned: Vec::new(), launcher_pins: None, launcher_foot: None, win_key: "auto".into(), reveal: "all".into(), hold_key: "none".into(), open_on: "hover".into(), tray_pinned: Vec::new(), quiet_full: true, quiet_banners: Vec::new(), liquid: 50.0, theme: "#5ac88c".into(), theme2: "#e8b04a".into(), desk_map: BTreeMap::new(), desk_keys: true }
    }
}

pub fn path() -> PathBuf {
    nadi_core::paths::data_dir().unwrap_or_default().join("nadi-frame.json")
}

/// The config, and when its file last changed (to know when to read it again).
pub fn load() -> (Config, Option<SystemTime>) {
    let p = path();
    let when = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let c = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
    (c, when)
}

pub fn changed_since(when: Option<SystemTime>) -> bool {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok() != when
}

pub fn save(c: &Config) {
    let _ = nadi_core::paths::write_atomic(&path(), serde_json::to_string_pretty(c).unwrap_or_default().as_bytes());
}

fn liquid_amount<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().map_or(50.0, |n| n as f32))
}

fn join_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_str().map(String::from))
}
