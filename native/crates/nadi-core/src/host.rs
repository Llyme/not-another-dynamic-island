//! The plugins, run: the ones in the plugins folder, switched on or off, their settings, and what each is told (by its permissions) and
//! says back (its widgets, its banners, what it asks the PC to do). It runs on a thread of its own (a plugin's script is never shared
//! between threads); the shell talks to it by commands and hears from it by updates.

use crate::luau::{Limits, Script};
use crate::manifest::{self, Manifest, WidgetSpec};
use crate::paths;
use crate::pc::Pc;
use crate::sensors;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// the most a plugin's widget, banner or store may weigh (as JSON)
const MAX_JSON: usize = 64 * 1024;
const MAX_SAYS: usize = 3;
/// a plugin that fails this many calls in a row is stopped
const MAX_STRIKES: u32 = 3;
const MAX_FILE: u64 = 256 * 1024;
const BATTERY_EVERY: Duration = Duration::from_secs(5);
const BATTERY_TOLD: Duration = Duration::from_secs(30);
const AUDIO_GAP: Duration = Duration::from_millis(40);
/// a plugin none of whose widgets can be seen: what comes often (the sound, its own ticks) comes no oftener than this
const UNSEEN_GAP: Duration = Duration::from_secs(1);
const PROCESSES_EVERY: Duration = Duration::from_secs(2);
const SYSTEM_EVERY: Duration = Duration::from_secs(2);
const USAGE_EVERY: Duration = Duration::from_secs(1);
const DOWNLOADS_EVERY: Duration = Duration::from_millis(1500);
const STEAM_EVERY: Duration = Duration::from_secs(3);
const GEAR_EVERY: Duration = Duration::from_secs(5);
const CLAUDE_EVERY: Duration = Duration::from_secs(2);
/// one banner every few seconds from a plugin, and the same one not again within a minute (one with a key of its own is the plugin's to repeat)
const SAY_GAP: Duration = Duration::from_secs(3);
const SAY_REPEAT: Duration = Duration::from_secs(60);
/// a live banner said within this long is still the same one: what changes of it comes as it changes
const LIVE_FOR: Duration = Duration::from_secs(10);
/// (a panel opened or closed, queued with the presses: no action is named this)
const OPEN: &str = "\u{0}open";
/// in a plugin's queue: a call from another plugin, an event of a service it hears
const CALL: &str = "\u{0}call";
const SERVICE: &str = "\u{0}service";
const HELPER_LINE: &str = "\u{0}helper";
/// its program stopping this many times in a minute: it is left stopped
const HELPER_RESTARTS: u32 = 5;

/// What the shell hears.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// the list of plugins changed (one was switched on, a setting changed, a note)
    Plugins(Vec<PluginInfo>),
    /// what a widget shows now (`Null`: nothing, it is gone)
    Widget { plugin: String, widget: String, content: Value },
    /// a banner
    Say { plugin: String, say: Value },
    /// a banner it said is not needed any more (by its key)
    Cancel { plugin: String, key: String },
    /// it asks that ambient banners wait (a game runs), or no longer
    Quiet { plugin: String, on: bool },
    /// it asks something in a modal (only as the answer to something pressed)
    Modal { plugin: String, spec: Value },
}

/// What the shell asks.
#[derive(Clone, Debug)]
pub enum Command {
    SetOn { id: String, on: bool },
    SetValue { id: String, key: String, value: Value },
    /// a button of a widget (or of its settings) was pressed
    Action { plugin: String, action: String, args: Value },
    /// the panel of a widget was opened or closed (the plugin is told, and sees which are open with every event)
    Open { plugin: String, widget: String, open: bool },
    /// whether any widget of a plugin can be seen now (none: what comes often comes to it seldom)
    Seen { plugin: String, seen: bool },
    /// a banner of the plugin that names an address was pressed: it opens, when the plugin may open it
    OpenUrl { plugin: String, url: String },
    /// look in the plugins folder again
    Rescan,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SettingInfo {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: Value,
    pub options: Vec<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub unit: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Perm {
    pub name: String,
    pub detail: String,
    pub why: String,
}

/// A plugin, as the settings list it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub on: bool,
    pub problem: Option<String>,
    pub note: Option<String>,
    pub permissions: Vec<Perm>,
    pub settings: Vec<SettingInfo>,
    pub actions: Vec<manifest::ActionSpec>,
    pub widgets: Vec<WidgetSpec>,
    /// the hosts it may reach (for the pictures on the web its widgets name)
    #[serde(skip)]
    pub net: Vec<String>,
    pub banners: bool,
    /// a face's look ("eyes", "aura"), drawn by the frame while the plugin is on
    pub look: String,
    /// where it came from ("Llyme/nadi-plugins v2.0.0"), when from a repository
    pub from: Option<String>,
    /// the services it gives, and the ones it uses
    pub provides: BTreeMap<String, String>,
    pub needs: BTreeMap<String, String>,
    /// its own program: its file, and whether it runs
    pub helper: Option<Value>,
}

/// What is kept of the plugins between runs (what a plugin keeps of its own is sealed apart: see `pluginrun`).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Saved {
    pub on: Vec<String>,
    /// the ones the person switched off (a plugin that is on by default is not switched back on)
    pub off: Vec<String>,
    pub values: BTreeMap<String, Map<String, Value>>,
}

struct Slot {
    script: Script,
    started: bool,
    strikes: u32,
    last_tick: Instant,
    wake_at: Option<Instant>,
    told_battery: (Value, Option<Instant>),
    audio_seq: u64,
    last_audio: Option<Instant>,
    media_seq: Option<u64>,
    last_proc: Option<Instant>,
    last_sys: Option<Instant>,
    toast_seen: Option<u64>,
    downloads: sensors::downloadsensor::Sensor,
    last_downloads: Option<Instant>,
    last_steam: Option<Instant>,
    last_gear: Option<Instant>,
    revealable: HashSet<String>,
    last_claude: Option<Instant>,
    watch: Vec<u32>,
    meter: sensors::usage::Meter,
    last_usage: Option<Instant>,
    files: HashMap<PathBuf, SystemTime>,
    /// when its files were last looked at (once a second)
    last_files: Option<Instant>,
    /// what it has been told of the browsers' pages, and which it watches
    browser: sensors::browsersensor::Feed,
    /// what came back of its requests and commands, for its next call
    results: Arc<Mutex<Vec<Value>>>,
    said: HashMap<String, Instant>,
    last_say: Option<Instant>,
}

impl Slot {
    fn new(script: Script) -> Slot {
        Slot {
            script,
            started: false,
            strikes: 0,
            last_tick: Instant::now(),
            wake_at: None,
            told_battery: (Value::Null, None),
            audio_seq: 0,
            last_audio: None,
            media_seq: None,
            last_proc: None,
            last_sys: None,
            toast_seen: None,
            downloads: sensors::downloadsensor::Sensor::default(),
            last_downloads: None,
            last_steam: None,
            last_gear: None,
            revealable: HashSet::new(),
            last_claude: None,
            watch: Vec::new(),
            meter: sensors::usage::Meter::default(),
            last_usage: None,
            files: HashMap::new(),
            last_files: None,
            browser: Default::default(),
            results: Arc::default(),
            said: HashMap::new(),
            last_say: None,
        }
    }
}

struct Plugin {
    manifest: Manifest,
    dir: PathBuf,
    problem: Option<String>,
    note: Option<String>,
    slot: Option<Slot>,
    widgets: BTreeMap<String, Value>,
    pressed: VecDeque<(String, Value, bool)>,
    /// its widgets whose panels are open
    open: BTreeMap<String, bool>,
    /// can any of its widgets be seen (till the shell says otherwise: yes)
    seen: bool,
    quiet: bool,
    /// its own program, while it is on, and its stops this minute (how many, since when)
    helper: Option<crate::helper::Helper>,
    helper_stops: (u32, Instant),
    /// the events of other plugins' services it hears
    subs: HashSet<String>,
}

pub struct Host {
    dir: PathBuf,
    saved_path: Option<PathBuf>,
    saved: Saved,
    plugins: Vec<Plugin>,
    pub pc: Arc<Pc>,
    /// how the battery is read (a test puts its own)
    pub battery_reader: Box<dyn FnMut() -> Value + Send>,
    battery_now: Value,
    battery_at: Option<Instant>,
    system_now: Value,
    system_at: Option<Instant>,
    /// what the PC is asked to do is not done (tests)
    pub dry: bool,
    /// the windows, as last read (titles, programs): one look serves every plugin of that step
    windows_seen: Option<(Instant, bool, bool, Value)>,
    /// (with `NADI_PROFILE` set: how long each plugin's events took to gather and to run, written there now and then)
    profile: Option<(PathBuf, Instant, BTreeMap<String, (f64, f64, u64)>)>,
    out: Vec<Update>,
}

fn source_of(dir: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".source.json")).ok()?).ok()?;
    let repo = v["repo"].as_str().filter(|r| !r.is_empty())?;
    Some(match v["reference"].as_str().filter(|r| !r.is_empty()) {
        Some(r) => format!("{repo} {r}"),
        None => repo.to_string(),
    })
}

fn weight(v: &Value) -> usize {
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(usize::MAX)
}

/// `~` and `%VARS%` in a path, expanded
fn expand(path: &str) -> PathBuf {
    let mut s = path.trim().to_string();
    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = std::env::var_os("USERPROFILE") {
            s = format!("{}{}", home.to_string_lossy(), rest);
        }
    }
    while let Some(i) = s.find('%') {
        let Some(j) = s[i + 1..].find('%') else { break };
        let name = &s[i + 1..i + 1 + j];
        let val = std::env::var(name).unwrap_or_default();
        s = format!("{}{}{}", &s[..i], val, &s[i + j + 2..]);
    }
    PathBuf::from(s)
}

/// the end of a file (at most `MAX_FILE`), and when it changed
fn tail_of(path: &Path) -> Option<(SystemTime, String)> {
    use std::io::{Read, Seek, SeekFrom};
    let meta = std::fs::metadata(path).ok()?;
    let mut f = std::fs::File::open(path).ok()?;
    if meta.len() > MAX_FILE {
        f.seek(SeekFrom::Start(meta.len() - MAX_FILE)).ok()?;
    }
    let mut buf = Vec::new();
    f.take(MAX_FILE).read_to_end(&mut buf).ok()?;
    Some((meta.modified().ok()?, String::from_utf8_lossy(&buf).into_owned()))
}

/// The windows as a plugin may see them, from one scan with everything: no titles (nor tool windows) without `titles`, no program
/// details without `programs`.
fn windows_for(all: &Value, titles: bool, programs: bool) -> Value {
    let list = all.as_array().cloned().unwrap_or_default();
    Value::Array(
        list.into_iter()
            .filter(|w| titles || w["tool"] != true)
            .map(|mut w| {
                if !titles {
                    w["title"] = json!("");
                }
                if !programs {
                    if let Some(o) = w.as_object_mut() {
                        o.remove("full");
                        o.remove("path");
                        o.remove("name");
                    }
                }
                w
            })
            .collect(),
    )
}

/// the windows on the screen, for a plugin that may see them: the one in front first
/// (a program's icon is not told: a widget names it, "exe:<path>", and the shell draws it)
fn windows_now(titles: bool, programs: bool) -> Value {
    let front = crate::win::foreground();
    let mut list: Vec<(bool, Value)> = sensors::scan::visible_windows_with(titles)
        .into_iter()
        .filter(|w| !w.minimized)
        .map(|w| {
            let exe = Path::new(&w.exe).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let is_front = w.hwnd.0 as isize == front;
            let mut v = json!({ "exe": exe, "pid": w.pid, "title": if titles { w.title.clone() } else { String::new() }, "front": is_front, "id": w.hwnd.0 as isize, "w": w.w, "h": w.h, "tool": w.tool });
            if programs {
                v["full"] = json!(w.full);
                let info = sensors::exeinfo::lookup(&w.exe);
                v["path"] = json!(w.exe);
                v["name"] = json!(info.name.unwrap_or_default());
            }
            (is_front, v)
        })
        .collect();
    list.sort_by_key(|(f, _)| !*f);
    Value::Array(list.into_iter().map(|(_, v)| v).collect())
}

impl Host {
    /// The plugins in `dir`, with what was kept at `saved` (none: nothing is kept, for tests), reading the PC through `pc`.
    pub fn new(dir: PathBuf, saved_path: Option<PathBuf>, pc: Arc<Pc>) -> Host {
        let saved: Saved = saved_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
        let mut h = Host {
            dir,
            saved_path,
            saved,
            plugins: Vec::new(),
            pc,
            battery_reader: Box::new(sensors::battery::read),
            battery_now: Value::Null,
            battery_at: None,
            system_now: Value::Null,
            system_at: None,
            dry: false,
            windows_seen: None,
            profile: std::env::var_os("NADI_PROFILE").map(|p| (PathBuf::from(p), Instant::now(), BTreeMap::new())),
            out: Vec::new(),
        };
        h.scan();
        h
    }

    fn save(&self) {
        if let Some(p) = &self.saved_path {
            let _ = paths::write_atomic(p, serde_json::to_string_pretty(&self.saved).unwrap_or_default().as_bytes());
        }
    }

    fn is_on(&self, id: &str) -> bool {
        self.saved.on.iter().any(|x| x == id)
    }

    /// Look in the plugins folder: new ones are added, gone ones go (with their widgets), changed ones are loaded again.
    pub fn scan(&mut self) {
        let mut found: Vec<(Manifest, PathBuf, Option<String>)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if let Some((m, problem)) = manifest::read(&p) {
                        found.push((m, p, problem));
                    }
                }
            }
        }
        found.sort_by(|a, b| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase()));
        let gone: Vec<String> = self.plugins.iter().filter(|p| !found.iter().any(|f| f.0.id == p.manifest.id)).map(|p| p.manifest.id.clone()).collect();
        for id in gone {
            self.unload(&id);
            self.plugins.retain(|p| p.manifest.id != id);
        }
        for (m, dir, problem) in found {
            if m.default_on && problem.is_none() && !self.is_on(&m.id) && !self.saved.off.contains(&m.id) {
                self.saved.on.push(m.id.clone());
            }
            let fresh = || Plugin { manifest: m.clone(), dir: dir.clone(), problem: problem.clone(), note: None, slot: None, widgets: BTreeMap::new(), pressed: VecDeque::new(), open: BTreeMap::new(), quiet: false, helper: None, helper_stops: (0, Instant::now()), subs: HashSet::new(), seen: true };
            match self.plugins.iter().position(|p| p.manifest.id == m.id) {
                Some(i) if self.plugins[i].manifest == m && self.plugins[i].problem == problem => {}
                Some(i) => {
                    self.unload(&m.id);
                    self.plugins[i] = fresh();
                }
                None => self.plugins.push(fresh()),
            }
        }
        let ids: Vec<String> = self.plugins.iter().map(|p| p.manifest.id.clone()).collect();
        for id in ids {
            if self.is_on(&id) {
                self.load(&id);
            }
        }
        self.wants();
        self.out.push(Update::Plugins(self.info()));
    }

    fn load(&mut self, id: &str) {
        let Some(p) = self.plugins.iter_mut().find(|p| p.manifest.id == id) else { return };
        if p.slot.is_some() || p.problem.is_some() {
            return;
        }
        // (a face has no script: the frame draws it)
        if p.manifest.kind == "face" {
            return;
        }
        let src = match std::fs::read_to_string(p.dir.join(&p.manifest.main)) {
            Ok(s) => s,
            Err(e) => {
                p.note = Some(format!("Its script could not be read ({e})."));
                return;
            }
        };
        let limits = Limits { call: Duration::from_millis(p.manifest.budget.call_ms.clamp(5, 2000)), memory: (p.manifest.budget.memory_mb.clamp(1, 256) as usize) << 20 };
        match Script::load(&p.manifest.id, &src, limits) {
            Ok(script) => {
                p.note = None;
                p.slot = Some(Slot::new(script));
                p.helper_stops = (0, Instant::now());
            }
            Err(e) => p.note = Some(e),
        }
        self.start_helper(id);
    }

    /// Start a plugin's own program (when it has one, and is on).
    fn start_helper(&mut self, id: &str) {
        if self.dry {
            return;
        }
        let Some(p) = self.plugins.iter_mut().find(|p| p.manifest.id == id) else { return };
        if p.slot.is_none() || p.helper.is_some() {
            return;
        }
        let Some(rel) = p.manifest.helper_path().map(str::to_string) else { return };
        let data = paths::data_dir().unwrap_or_default().join("plugin-data").join(id);
        match crate::helper::Helper::start(&p.dir.join(&rel), &p.dir, &data) {
            Ok(h) => p.helper = Some(h),
            Err(e) => p.note = Some(format!("Its program could not be started ({e}).")),
        }
    }

    fn unload(&mut self, id: &str) {
        let Some(p) = self.plugins.iter_mut().find(|p| p.manifest.id == id) else { return };
        p.slot = None;
        p.open.clear();
        p.helper = None;
        p.subs.clear();
        if std::mem::take(&mut p.quiet) {
            self.out.push(Update::Quiet { plugin: id.to_string(), on: false });
        }
        let p = self.plugins.iter_mut().find(|p| p.manifest.id == id).unwrap();
        for (w, _) in std::mem::take(&mut p.widgets) {
            self.out.push(Update::Widget { plugin: id.to_string(), widget: w, content: Value::Null });
        }
    }

    /// what the sensors are wanted for, from the plugins that are on
    fn wants(&self) {
        let reads = self.plugins.iter().any(|p| p.slot.is_some() && p.manifest.permissions.browser);
        if !reads && !self.dry {
            sensors::browsersensor::state().set_config(sensors::browsersensor::disabled());
        }
        let any = |f: fn(&manifest::Permissions) -> bool| self.plugins.iter().any(|p| p.slot.is_some() && f(&p.manifest.permissions));
        self.pc.want.media.store(any(|p| p.media), Ordering::Relaxed);
        self.pc.want.media_art.store(any(|p| p.media_art), Ordering::Relaxed);
        self.pc.want.audio.store(any(|p| p.audio || p.viz), Ordering::Relaxed);
        self.pc.want.notifications.store(any(|p| p.notifications), Ordering::Relaxed);
    }

    pub fn set_on(&mut self, id: &str, on: bool) {
        if !self.plugins.iter().any(|p| p.manifest.id == id) {
            return;
        }
        self.switch(id, on, 0);
        self.save();
        self.wants();
        self.out.push(Update::Plugins(self.info()));
    }

    /// Switch one on or off, with what follows: on, the providers of the services it needs (installed and off) come on first; off, the
    /// plugins that need a service only it gives go off too.
    fn switch(&mut self, id: &str, on: bool, depth: u32) {
        if depth > 8 {
            return;
        }
        let Some(m) = self.plugins.iter().find(|p| p.manifest.id == id).map(|p| p.manifest.clone()) else { return };
        if on {
            for (service, want) in &m.needs {
                if self.provider_of(service, want).is_none() {
                    let off = self.plugins.iter().find(|p| p.problem.is_none() && p.manifest.provides.get(service).is_some_and(|v| manifest::version_meets(v, want))).map(|p| p.manifest.id.clone());
                    if let Some(pid) = off {
                        self.switch(&pid, true, depth + 1);
                    }
                }
            }
        }
        self.saved.on.retain(|x| x != id);
        self.saved.off.retain(|x| x != id);
        if on {
            self.saved.on.push(id.to_string());
            self.load(id);
        } else {
            self.saved.off.push(id.to_string());
            self.unload(id);
            for service in m.provides.keys() {
                let still = self.plugins.iter().any(|p| p.manifest.id != id && self.is_on(&p.manifest.id) && p.manifest.provides.contains_key(service));
                if still {
                    continue;
                }
                let deps: Vec<String> = self.plugins.iter().filter(|p| self.is_on(&p.manifest.id) && p.manifest.needs.contains_key(service)).map(|p| p.manifest.id.clone()).collect();
                for d in deps {
                    self.switch(&d, false, depth + 1);
                }
            }
        }
    }

    /// The plugin that gives a service now (on, whole, a version that will do).
    fn provider_of(&self, service: &str, want: &str) -> Option<usize> {
        self.plugins.iter().position(|p| p.problem.is_none() && self.is_on(&p.manifest.id) && p.manifest.provides.get(service).is_some_and(|v| manifest::version_meets(v, want)))
    }

    pub fn set_value(&mut self, id: &str, key: &str, value: Value) {
        let Some(p) = self.plugins.iter().find(|p| p.manifest.id == id) else { return };
        if !p.manifest.settings.contains_key(key) {
            return;
        }
        self.saved.values.entry(id.to_string()).or_default().insert(key.to_string(), value);
        self.save();
        // (it hears of it with its next event; a tick now, so that it is soon)
        if let Some(s) = self.plugins.iter_mut().find(|p| p.manifest.id == id).and_then(|p| p.slot.as_mut()) {
            s.wake_at = Some(Instant::now());
        }
        self.out.push(Update::Plugins(self.info()));
    }

    /// A button the person pressed (on a widget, or in the plugin's settings): the plugin is told with its next step.
    pub fn press(&mut self, plugin: &str, action: &str, args: Value) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.manifest.id == plugin) {
            if p.pressed.len() < 16 {
                p.pressed.push_back((action.chars().take(100).collect(), args, true));
            }
        }
    }

    /// May this plugin open this address (its manifest's `open` starts it)?
    pub fn may_open(&self, plugin: &str, url: &str) -> bool {
        let Some(p) = self.plugins.iter().find(|p| p.manifest.id == plugin && p.slot.is_some()) else { return false };
        let low = url.to_lowercase();
        url.len() <= 1000 && !url.chars().any(|c| c.is_control() || c == ' ') && p.manifest.permissions.open.iter().any(|o| !o.is_empty() && low.starts_with(&o.to_lowercase()))
    }

    /// A widget's panel was opened or closed: the plugin is told with its next step (`open`, with `widget` and `open`).
    /// Whether any widget of a plugin can be seen now: none, and the sound and its own ticks come to it once a second at most.
    pub fn set_seen(&mut self, plugin: &str, seen: bool) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.manifest.id == plugin) {
            p.seen = seen;
        }
    }

    pub fn set_open(&mut self, plugin: &str, widget: &str, open: bool) {
        let Some(p) = self.plugins.iter_mut().find(|p| p.manifest.id == plugin) else { return };
        if !p.manifest.widgets.iter().any(|w| w.id == widget) {
            return;
        }
        if open {
            p.open.insert(widget.to_string(), true);
        } else {
            p.open.remove(widget);
        }
        if p.slot.is_some() && p.pressed.len() < 16 {
            p.pressed.push_back((OPEN.into(), json!({ "widget": widget, "open": open }), true));
        }
    }

    fn values_of(&self, p: &Plugin) -> Value {
        let mut out = Map::new();
        let set = self.saved.values.get(&p.manifest.id);
        for (k, s) in &p.manifest.settings {
            out.insert(k.clone(), set.and_then(|m| m.get(k)).cloned().unwrap_or_else(|| s.default.clone()));
        }
        Value::Object(out)
    }

    pub fn info(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| {
                let values = self.values_of(p);
                PluginInfo {
                    id: p.manifest.id.clone(),
                    name: p.manifest.name.clone(),
                    description: p.manifest.description.clone(),
                    version: p.manifest.version.clone(),
                    on: self.is_on(&p.manifest.id) && p.problem.is_none(),
                    problem: p.problem.clone(),
                    note: p.note.clone(),
                    permissions: manifest::manifest_words(&p.manifest).into_iter().map(|(name, detail, why)| Perm { name, detail, why }).collect(),
                    provides: p.manifest.provides.clone(),
                    needs: p.manifest.needs.clone(),
                    helper: p.manifest.helper_path().map(|h| json!({ "file": Path::new(h).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), "running": p.helper.is_some() })),
                    net: p.manifest.permissions.net.iter().filter(|h| !h.contains("{{")).cloned().collect(),
                    settings: p
                        .manifest
                        .settings
                        .iter()
                        .map(|(k, s)| SettingInfo {
                            key: k.clone(),
                            label: if s.label.is_empty() { k.clone() } else { s.label.clone() },
                            kind: if s.kind.is_empty() { "text".into() } else { s.kind.clone() },
                            value: values[k].clone(),
                            options: s.options.iter().map(|o| json!({ "value": o.value, "label": o.label })).collect(),
                            min: s.min,
                            max: s.max,
                            step: s.step,
                            unit: s.unit.clone(),
                        })
                        .collect(),
                    actions: p.manifest.actions.clone(),
                    widgets: p.manifest.widgets.clone(),
                    banners: p.manifest.banners,
                    look: if p.manifest.kind == "face" { p.manifest.look.clone() } else { String::new() },
                    from: source_of(&p.dir),
                }
            })
            .collect()
    }

    pub fn take_updates(&mut self) -> Vec<Update> {
        std::mem::take(&mut self.out)
    }

    pub fn widget(&self, plugin: &str, widget: &str) -> Option<&Value> {
        self.plugins.iter().find(|p| p.manifest.id == plugin)?.widgets.get(widget)
    }

    /// Every widget that shows something now: (plugin, widget, content).
    pub fn widgets(&self) -> Vec<(String, String, Value)> {
        self.plugins.iter().flat_map(|p| p.widgets.iter().map(|(w, c)| (p.manifest.id.clone(), w.clone(), c.clone()))).collect()
    }

    /// Read what is due from the PC, and tell each plugin that is on what it is due.
    pub fn step(&mut self, now_ms: u64) {
        let want = |f: fn(&manifest::Permissions) -> bool| self.plugins.iter().any(|p| p.slot.is_some() && f(&p.manifest.permissions));
        if want(|p| p.battery) && self.battery_at.map_or(true, |t| t.elapsed() >= BATTERY_EVERY) {
            self.battery_now = (self.battery_reader)();
            self.battery_at = Some(Instant::now());
            // (what each app drew of it, counted as it is read)
            if want(|p| p.app_power) {
                sensors::apppower::sample(&self.battery_now);
            }
        }
        if want(|p| p.system) && self.system_at.map_or(true, |t| t.elapsed() >= SYSTEM_EVERY) {
            let s = sensors::stats::read(&self.pc);
            self.system_now = json!({ "cpu": s.cpu_pct, "ram": s.ram_pct, "ram_used_gb": s.ram_used_gb, "ram_total_gb": s.ram_total_gb, "gpu": sensors::gpu::pct(&self.pc) });
            self.system_at = Some(Instant::now());
        }
        for i in 0..self.plugins.len() {
            if self.plugins[i].slot.is_none() {
                continue;
            }
            self.keep_helper(i);
            let t0 = Instant::now();
            let events = self.due(i, now_ms);
            let t1 = Instant::now();
            let n = events.len() as u64;
            for (ev, by_user) in events {
                if self.plugins[i].slot.is_none() {
                    break;
                }
                self.run(i, ev, by_user);
            }
            let (g, r) = ((t1 - t0).as_secs_f64() * 1000.0, t1.elapsed().as_secs_f64() * 1000.0);
            {
                let mut cost = self.pc.plugin_cost.lock().unwrap();
                let e = cost.entry(self.plugins[i].manifest.id.clone()).or_default();
                e.0 += g;
                e.1 += r;
                e.2 += n;
            }
            if let Some((_, _, map)) = &mut self.profile {
                let e = map.entry(self.plugins[i].manifest.id.clone()).or_default();
                e.0 += g;
                e.1 += r;
                e.2 += n;
            }
        }
        if let Some((path, at, map)) = &mut self.profile {
            if at.elapsed() >= Duration::from_secs(10) {
                let text: String = map.iter().map(|(k, (g, r, n))| format!("{k:24} gather {g:8.1} ms  run {r:8.1} ms  events {n}\n")).collect();
                let _ = std::fs::write(path, text);
            }
        }
    }

    /// A plugin's program, stopped: started again (five times a minute at most; more, and it is left stopped, with a note).
    fn keep_helper(&mut self, i: usize) {
        let p = &mut self.plugins[i];
        let Some(h) = p.helper.as_mut() else {
            return;
        };
        if h.alive() {
            return;
        }
        // (what it said before it stopped is still told)
        let last = h.take();
        p.pressed.extend(last.into_iter().map(|v| (HELPER_LINE.to_string(), v, false)));
        p.helper = None;
        if p.helper_stops.1.elapsed() > Duration::from_secs(60) {
            p.helper_stops = (0, Instant::now());
        }
        p.helper_stops.0 += 1;
        p.pressed.push_back((HELPER_LINE.to_string(), json!({ "kind": "stopped" }), false));
        if p.helper_stops.0 >= HELPER_RESTARTS {
            p.note = Some("Its program keeps stopping, and was left stopped.".into());
            self.out.push(Update::Plugins(self.info()));
            return;
        }
        let id = p.manifest.id.clone();
        self.start_helper(&id);
    }

    fn event(&self, i: usize, name: &str, now_ms: u64, extra: Value) -> Value {
        let p = &self.plugins[i];
        let mut o = Map::new();
        o.insert("event".into(), json!(name));
        o.insert("now".into(), json!(now_ms));
        o.insert("settings".into(), self.values_of(p));
        o.insert("tz_offset".into(), json!(chrono::Local::now().offset().local_minus_utc() / 60));
        o.insert("open".into(), json!(p.open));
        if let Value::Object(m) = extra {
            o.extend(m);
        }
        Value::Object(o)
    }

    /// the events a plugin is due, from what it may see (the flag: the answer to something the person pressed)
    fn due(&mut self, i: usize, now_ms: u64) -> Vec<(Value, bool)> {
        let mut out: Vec<(&'static str, Value, bool)> = Vec::new();
        let perms = self.plugins[i].manifest.permissions.clone();
        let id = self.plugins[i].manifest.id.clone();
        // (none of its widgets to be seen: what comes often, seldom)
        let seen = self.plugins[i].seen;
        let tick_every = manifest::parse_every(&self.plugins[i].manifest.tick).map(|e| if seen { e } else { e.max(UNSEEN_GAP) });
        let audio_gap = if seen { AUDIO_GAP } else { UNSEEN_GAP };
        let usage_every = if seen { USAGE_EVERY } else { UNSEEN_GAP * 5 };
        let battery = self.battery_now.clone();
        let system = self.system_now.clone();
        let pc = self.pc.clone();
        let due_proc = perms.processes && self.plugins[i].slot.as_ref().is_some_and(|s| s.last_proc.map_or(true, |t| t.elapsed() >= PROCESSES_EVERY));
        // (a panel of it open: the busiest programs too, with what the PC uses)
        let panel_open = self.plugins[i].open.values().any(|o| *o);
        let mut timed: Vec<(&'static str, f64)> = Vec::new();
        let tw = Instant::now();
        let windows = if due_proc {
            let all = match &self.windows_seen {
                Some((at, _, _, v)) if at.elapsed() < Duration::from_millis(900) => v.clone(),
                _ => {
                    let v = windows_now(true, true);
                    self.windows_seen = Some((Instant::now(), true, true, v.clone()));
                    v
                }
            };
            windows_for(&all, perms.titles, perms.programs)
        } else {
            Value::Null
        };
        if due_proc {
            timed.push(("windows", tw.elapsed().as_secs_f64() * 1000.0));
        }
        let s = self.plugins[i].slot.as_mut().unwrap();
        if !s.started {
            s.started = true;
            let extra = if perms.storage { json!({ "store": crate::pluginrun::store_of(&id) }) } else { json!({}) };
            out.push(("start", extra, false));
        }
        if perms.battery && !battery.is_null() && (sensors::battery::changed(&s.told_battery.0, &battery) || s.told_battery.1.map_or(true, |t| t.elapsed() >= BATTERY_TOLD)) {
            s.told_battery = (battery.clone(), Some(Instant::now()));
            let mut b = battery.clone();
            if perms.app_power {
                b["apps"] = sensors::apppower::summary();
            }
            out.push(("battery", json!({ "battery": b }), false));
        }
        if perms.audio {
            let seq = pc.audio_seq.load(Ordering::Relaxed);
            if seq != s.audio_seq && s.last_audio.map_or(true, |t| t.elapsed() >= audio_gap) {
                s.audio_seq = seq;
                s.last_audio = Some(Instant::now());
                let a = pc.audio_last.lock().unwrap().clone();
                if !a.is_null() {
                    out.push(("audio", json!({ "audio": a }), false));
                }
            }
        }
        if due_proc {
            s.last_proc = Some(Instant::now());
            let mut extra = json!({ "windows": windows });
            if perms.idle {
                extra["idle_s"] = json!(crate::win::idle_seconds().unwrap_or(0.0));
            }
            out.push(("processes", extra, false));
        }
        if perms.media {
            let seq = pc.media_seq.load(Ordering::Relaxed);
            if s.media_seq != Some(seq) {
                s.media_seq = Some(seq);
                // (the cover is not told: a widget names it, "media:<player>:<id>", and the shell draws it; one that may not
                // show it is not told which it is)
                let mut list = pc.media_seen.lock().unwrap().clone();
                if list.is_null() {
                    list = json!([]);
                }
                if !perms.media_art {
                    for m in list.as_array_mut().into_iter().flatten() {
                        m["art_id"] = json!(0);
                    }
                }
                out.push(("media", json!({ "sessions": list }), false));
            }
        }
        // what its requests and commands came back with
        for r in std::mem::take(&mut *s.results.lock().unwrap()) {
            if r.get("op").is_some() {
                // (a button pressed, or a tab sent on)
                out.push(("browser_done", json!({ "id": r["id"], "op": r["op"], "ok": r["ok"] }), false));
            } else if r.get("status").is_some() {
                out.push(("fetched", json!({ "id": r["id"], "status": r["status"], "body": r["body"], "error": r["error"].as_str().unwrap_or(""), "location": r["location"] }), false));
            } else {
                out.push(("command", json!({ "id": r["id"], "code": r["code"], "stdout": r["stdout"] }), false));
            }
        }
        if perms.notifications {
            if s.toast_seen.is_none() {
                // (what was there before it started is not news)
                s.toast_seen = Some(pc.toast_seq.load(Ordering::Relaxed));
            }
            let seen = s.toast_seen.unwrap_or(0);
            let fresh: Vec<Value> = pc.toasts.lock().unwrap().iter().filter(|t| t["seq"].as_u64().unwrap_or(0) > seen).cloned().collect();
            if let Some(last) = fresh.last() {
                s.toast_seen = last["seq"].as_u64();
                out.push(("notifications", json!({ "notifications": fresh }), false));
            }
        }
        if perms.system && !system.is_null() && s.last_sys.map_or(true, |t| t.elapsed() >= SYSTEM_EVERY) {
            s.last_sys = Some(Instant::now());
            let mut system = system.clone();
            if panel_open {
                system["top"] = sensors::stats::top(&pc, 3);
            }
            out.push(("system", json!({ "system": system }), false));
        }
        if perms.usage && !s.watch.is_empty() && s.last_usage.map_or(true, |t| t.elapsed() >= usage_every) {
            s.last_usage = Some(Instant::now());
            let tu = Instant::now();
            let list = sensors::usage::read(&mut s.meter, &s.watch);
            timed.push(("usage", tu.elapsed().as_secs_f64() * 1000.0));
            out.push(("usage", json!({ "usage": list }), false));
        }
        if perms.steam && s.last_steam.map_or(true, |t| t.elapsed() >= STEAM_EVERY) {
            s.last_steam = Some(Instant::now());
            out.push(("steam", sensors::steamlib::state(), false));
        }
        if perms.gear && s.last_gear.map_or(true, |t| t.elapsed() >= GEAR_EVERY) {
            s.last_gear = Some(Instant::now());
            let tg = Instant::now();
            out.push(("gear", sensors::gear::state(), false));
            timed.push(("gear", tg.elapsed().as_secs_f64() * 1000.0));
        }
        if perms.downloads && s.last_downloads.map_or(true, |t| t.elapsed() >= DOWNLOADS_EVERY) {
            s.last_downloads = Some(Instant::now());
            let td = Instant::now();
            let seen = s.downloads.poll();
            timed.push(("downloads", td.elapsed().as_secs_f64() * 1000.0));
            *pc.downloads.lock().unwrap() = (Some(Instant::now()), seen["observed"].as_array().cloned().unwrap_or_default());
            for e in seen["ended"].as_array().into_iter().flatten() {
                if let Some(path) = e["path"].as_str() {
                    s.revealable.insert(path.to_string());
                }
            }
            if s.revealable.len() > 200 {
                s.revealable.clear();
            }
            out.push(("downloads", seen, false));
        }
        if perms.browser {
            if let Some(b) = s.browser.poll(sensors::browsersensor::state()) {
                out.push(("browser", json!({ "browser": b }), false));
            }
        }
        if perms.claude && s.last_claude.map_or(true, |t| t.elapsed() >= CLAUDE_EVERY) {
            s.last_claude = Some(Instant::now());
            let hooks = if perms.claude_answer { sensors::claudehook::status() } else { Value::Null };
            out.push(("claude", json!({ "claude": sensors::claudesensor::sessions(), "claude_hooks": hooks }), false));
        }
        let files_due = !perms.files.is_empty() && s.last_files.map_or(true, |t| t.elapsed() >= Duration::from_secs(1));
        if files_due {
            s.last_files = Some(Instant::now());
        }
        for f in perms.files.iter().filter(|_| files_due) {
            let path = expand(f);
            // (changed since it was read? Asked of its time first: read only when it did)
            let Ok(changed) = std::fs::metadata(&path).and_then(|m| m.modified()) else { continue };
            if s.files.get(&path) == Some(&changed) {
                continue;
            }
            if let Some((mtime, text)) = tail_of(&path) {
                if s.files.get(&path) != Some(&mtime) {
                    s.files.insert(path.clone(), mtime);
                    out.push(("file", json!({ "path": path.to_string_lossy(), "text": text }), false));
                }
            }
        }
        let tick_due = tick_every.is_some_and(|e| s.last_tick.elapsed() >= e);
        let wake_due = s.wake_at.is_some_and(|t| Instant::now() >= t);
        if tick_due || wake_due {
            s.last_tick = Instant::now();
            s.wake_at = None;
            out.push(("tick", json!({ "wake": wake_due }), false));
        }
        if let Some(h) = self.plugins[i].helper.as_ref() {
            let said = h.take();
            self.plugins[i].pressed.extend(said.into_iter().map(|v| (HELPER_LINE.to_string(), v, false)));
        }
        let pressed: Vec<(String, Value, bool)> = self.plugins[i].pressed.drain(..).collect();
        for (action, args, by_user) in pressed {
            if action == OPEN {
                out.push(("open", args, by_user));
            } else if action == HELPER_LINE {
                out.push(("helper", json!({ "data": args }), false));
            } else if action == CALL {
                out.push(("call", args, false));
            } else if action == SERVICE {
                // (its name is the event's: "speech.line")
                out.push(("service", args, false));
            } else {
                out.push(("action", json!({ "action": action, "args": args }), by_user));
            }
        }
        if !timed.is_empty() {
            let mut cost = self.pc.plugin_cost.lock().unwrap();
            for (what, ms) in &timed {
                let e = cost.entry(format!("~ {what}")).or_default();
                e.0 += ms;
                e.2 += 1;
            }
        }
        if let Some((_, _, map)) = &mut self.profile {
            for (what, ms) in timed {
                let e = map.entry(format!("~ {what}")).or_default();
                e.0 += ms;
                e.2 += 1;
            }
        }
        out.into_iter().map(|(name, extra, by)| (self.event(i, name, now_ms, extra), by)).collect()
    }

    fn run(&mut self, i: usize, ev: Value, by_user: bool) {
        let id = self.plugins[i].manifest.id.clone();
        let reply = {
            let Some(s) = self.plugins[i].slot.as_mut() else { return };
            match s.script.call(&ev) {
                Ok(r) => {
                    s.strikes = 0;
                    r
                }
                Err(e) => {
                    s.strikes += 1;
                    let stop = s.strikes >= MAX_STRIKES;
                    self.plugins[i].note = Some(format!("It failed: {e}"));
                    if stop {
                        self.unload(&id);
                        self.wants();
                        self.plugins[i].note = Some(format!("Stopped after failing {MAX_STRIKES} times in a row: {e}"));
                    }
                    self.out.push(Update::Plugins(self.info()));
                    return;
                }
            }
        };
        self.apply(i, reply, by_user);
    }

    /// What one answer of a plugin does: its widgets (only the ones its manifest offers), banners, a note, what it keeps, when to be called,
    /// and what it asks of the PC (each only with the permission for it).
    fn apply(&mut self, i: usize, reply: Value, by_user: bool) {
        let Some(o) = reply.as_object() else { return };
        let id = self.plugins[i].manifest.id.clone();
        let perms = self.plugins[i].manifest.permissions.clone();
        if let Some(ws) = o.get("widgets").and_then(|w| w.as_object()) {
            for (wid, content) in ws {
                if !self.plugins[i].manifest.widgets.iter().any(|w| &w.id == wid) || weight(content) > MAX_JSON {
                    continue;
                }
                let content = if content.as_object().is_some_and(|m| m.is_empty()) { Value::Null } else { content.clone() };
                let same = self.plugins[i].widgets.get(wid) == Some(&content) || (content.is_null() && !self.plugins[i].widgets.contains_key(wid));
                if same {
                    continue;
                }
                if content.is_null() {
                    self.plugins[i].widgets.remove(wid);
                } else {
                    self.plugins[i].widgets.insert(wid.clone(), content.clone());
                }
                self.out.push(Update::Widget { plugin: id.clone(), widget: wid.clone(), content });
            }
        }
        // (a question in a modal: only as the answer to a press, never by itself)
        if let Some(m) = o.get("modal").filter(|m| m.is_object() && weight(m) <= MAX_JSON) {
            if by_user {
                self.out.push(Update::Modal { plugin: id.clone(), spec: m.clone() });
            }
        }
        if let Some(says) = o.get("say").and_then(|s| s.as_array()) {
            for s in says.iter().take(MAX_SAYS) {
                let text_ok = s.get("text").and_then(|t| t.as_str()).is_some_and(|t| !t.trim().is_empty() && t.chars().count() <= 200);
                if !text_ok || weight(s) > MAX_JSON {
                    continue;
                }
                if !by_user && !self.say_allowed(i, s) {
                    continue;
                }
                self.out.push(Update::Say { plugin: id.clone(), say: s.clone() });
            }
        }
        for k in o.get("cancel").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|k| k.as_str()).take(8) {
            self.out.push(Update::Cancel { plugin: id.clone(), key: k.chars().take(60).collect() });
        }
        if let Some(n) = o.get("note") {
            let note = n.as_str().map(|t| t.trim().chars().take(200).collect::<String>()).filter(|t| !t.is_empty());
            if self.plugins[i].note != note {
                self.plugins[i].note = note;
                self.out.push(Update::Plugins(self.info()));
            }
        }
        if perms.storage {
            if let Some(store) = o.get("store").filter(|v| !v.is_null() && weight(v) <= MAX_JSON) {
                if !self.dry {
                    crate::pluginrun::keep(&id, store);
                }
            }
        }
        // (a picture it had pasted, let go of: its file gone from its folder; only those, never another of its files)
        if let Some(f) = o.get("forget").and_then(|v| v.as_str()) {
            let ok = f.starts_with("paste-") && f.ends_with(".png") && !f.contains(['/', '\\', ':']) && !f.contains("..");
            if ok && !self.dry {
                if let Some(d) = crate::paths::plugins_dir() {
                    let _ = std::fs::remove_file(d.join(&id).join(f));
                }
            }
        }
        if let Some(q) = o.get("quiet").and_then(|q| q.as_bool()) {
            if self.plugins[i].quiet != q {
                self.plugins[i].quiet = q;
                self.out.push(Update::Quiet { plugin: id.clone(), on: q });
            }
        }
        if let Some(ms) = o.get("wake_ms").and_then(|w| w.as_u64()) {
            if let Some(s) = self.plugins[i].slot.as_mut() {
                s.wake_at = Some(Instant::now() + Duration::from_millis(ms.clamp(50, 86_400_000)));
            }
        }
        if perms.usage {
            if let Some(list) = o.get("watch").and_then(|w| w.as_array()) {
                if let Some(s) = self.plugins[i].slot.as_mut() {
                    s.watch = list.iter().filter_map(|p| p.as_u64().map(|p| p as u32)).filter(|p| *p != 0).take(4).collect();
                }
            }
        }
        // (to its own program: each a line of JSON)
        if let Some(h) = self.plugins[i].helper.as_mut() {
            let lines: Vec<Value> = match o.get("helper") {
                Some(Value::Array(a)) => a.iter().take(32).cloned().collect(),
                Some(v @ Value::Object(_)) => vec![v.clone()],
                _ => Vec::new(),
            };
            for l in lines {
                if weight(&l) <= MAX_JSON {
                    h.send(&l);
                }
            }
        }
        // (what it hears: the events of the services it needs or gives, by name: "speech.line")
        let services: Vec<String> = self.plugins[i].manifest.needs.keys().chain(self.plugins[i].manifest.provides.keys()).cloned().collect();
        let of_service = |name: &str| services.iter().any(|s| name == s || name.starts_with(&format!("{s}.")));
        for name in o.get("subscribe").and_then(|v| v.as_array()).into_iter().flatten().take(32).filter_map(|v| v.as_str()) {
            if of_service(name) {
                self.plugins[i].subs.insert(name.to_string());
            }
        }
        for name in o.get("unsubscribe").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()) {
            self.plugins[i].subs.remove(name);
        }
        // (a call to the plugin that gives a service it needs: that one is told, with who called)
        let calls: Vec<Value> = match o.get("call") {
            Some(Value::Array(a)) => a.iter().take(8).cloned().collect(),
            Some(v @ Value::Object(_)) => vec![v.clone()],
            _ => Vec::new(),
        };
        for c in calls {
            let service = c.get("service").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let Some(want) = self.plugins[i].manifest.needs.get(&service).cloned() else { continue };
            if weight(&c) > MAX_JSON {
                continue;
            }
            if let Some(j) = self.provider_of(&service, &want) {
                let call = json!({ "from": id, "service": service, "what": c.get("what").cloned().unwrap_or(Value::Null), "args": c.get("args").cloned().unwrap_or(Value::Null) });
                self.plugins[j].pressed.push_back((CALL.to_string(), call, false));
            }
        }
        // (an event of a service it gives: to every plugin that hears it, its data as the event)
        let emits: Vec<Value> = match o.get("emit") {
            Some(Value::Array(a)) => a.iter().take(32).cloned().collect(),
            Some(v @ Value::Object(_)) => vec![v.clone()],
            _ => Vec::new(),
        };
        for e in emits {
            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let gives = self.plugins[i].manifest.provides.keys().any(|s| name.starts_with(&format!("{s}.")));
            if !gives || weight(&e) > MAX_JSON {
                continue;
            }
            let mut ev = match e.get("data") {
                Some(Value::Object(m)) => m.clone(),
                Some(v) => {
                    let mut m = Map::new();
                    m.insert("data".into(), v.clone());
                    m
                }
                None => Map::new(),
            };
            ev.insert("event".into(), json!(name));
            ev.insert("from".into(), json!(id));
            let ev = Value::Object(ev);
            for k in 0..self.plugins.len() {
                if k != i && self.plugins[k].slot.is_some() && self.plugins[k].subs.contains(&name) {
                    self.plugins[k].pressed.push_back((SERVICE.to_string(), ev.clone(), false));
                }
            }
        }
        if self.dry {
            return;
        }
        // what it asks the PC to do
        if perms.media_control {
            if let Some(m) = o.get("media").and_then(|m| m.as_object()) {
                let action = m.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string();
                let source = m.get("source").and_then(|a| a.as_str()).map(str::to_string).filter(|s| !s.is_empty());
                let value = m.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
                // (each on a thread of its own: a player that never answers does not hold the plugins up)
                std::thread::spawn(move || {
                    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
                    use sensors::media::*;
                    match action.as_str() {
                        "play_pause" => media_play_pause(source),
                        "next" => media_next(source),
                        "previous" => media_previous(source),
                        "seek" => media_seek(value.max(0.0), source),
                        "seek_by" => media_seek_by(value.clamp(-3600.0, 3600.0), source),
                        "set_rate" => {
                            media_set_rate(value, source);
                        }
                        "focus" => {
                            crate::focus::focus_source(None, None, source);
                        }
                        _ => {}
                    }
                });
            }
        }
        if perms.programs || perms.processes {
            if let Some(f) = o.get("focus").and_then(|f| f.as_object()) {
                let s = |k: &str| f.get(k).and_then(|v| v.as_str()).map(|v| v.chars().take(260).collect::<String>()).filter(|v| !v.is_empty());
                let (exe, title) = (s("exe"), s("title"));
                std::thread::spawn(move || {
                    crate::focus::focus_source(exe, title, None);
                });
            }
        }
        if let Some(url) = o.get("open").and_then(|u| u.as_str()) {
            let low = url.to_lowercase();
            if url.len() <= 1000 && !url.chars().any(|c| c.is_control() || c == ' ') && perms.open.iter().any(|p| !p.is_empty() && (low.starts_with(&p.to_lowercase()) || open_under(p, url))) {
                crate::win::open_url(url);
            }
        }
        if perms.browser {
            if let Some(b) = o.get("browser").and_then(|b| b.as_object()) {
                let ext = sensors::browsersensor::state();
                if let Some(w) = b.get("watch").and_then(|w| w.as_array()) {
                    if let Some(slot) = self.plugins[i].slot.as_mut() {
                        slot.browser.watch = w.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect();
                    }
                }
                if let Some(c) = b.get("config") {
                    // (what the extensions are told to read; the link is served from now on)
                    ext.serve();
                    ext.set_config(sensors::browsersensor::config_json(c));
                }
                // (a button pressed or a tab sent on waits for the browser: on a thread, and the answer is the plugin's next event)
                if let Some(results) = self.plugins[i].slot.as_ref().map(|s| s.results.clone()) {
                    for op in ["press", "go", "scroll"] {
                        let Some(r) = b.get(op).cloned() else { continue };
                        let q = results.clone();
                        std::thread::spawn(move || {
                            let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
                            let ext = sensors::browsersensor::state();
                            let ok = ext.covers(&s("browser"))
                                && match op {
                                    "press" => ext.press(&s("browser"), &s("label")),
                                    "scroll" => ext.scroll(&s("browser"), &s("title"), &s("heading")),
                                    _ => ext.navigate(&s("browser"), &s("title"), &s("url")),
                                };
                            let mut q = q.lock().unwrap();
                            q.push(json!({ "op": op, "id": s("id"), "ok": ok }));
                            if q.len() > 16 {
                                q.remove(0);
                            }
                        });
                    }
                }
            }
        }
        if perms.downloads {
            if let Some(path) = o.get("reveal").and_then(|r| r.as_str()) {
                if self.plugins[i].slot.as_ref().is_some_and(|s| s.revealable.contains(path)) {
                    crate::win::reveal_file(path);
                }
            }
            // (a finished download opened, in the program that opens it: only one it was told of)
            if let Some(path) = o.get("open_file").and_then(|r| r.as_str()) {
                if self.plugins[i].slot.as_ref().is_some_and(|s| s.revealable.contains(path)) {
                    crate::win::open_url(path);
                }
            }
        }
        if perms.claude_answer {
            if let Some(a) = o.get("answer").and_then(|a| a.as_object()) {
                let s = |k: &str, n: usize| a.get(k).and_then(|v| v.as_str()).unwrap_or("").chars().take(n).collect::<String>();
                let (aid, behavior, text) = (s("id", 40), s("behavior", 10), s("text", 2000));
                // (a question's answers: each question, the choice made)
                let answers: serde_json::Map<String, Value> = a.get("answers").and_then(|v| v.as_object()).map(|m| m.iter().take(8).filter_map(|(k, v)| v.as_str().map(|v| (k.chars().take(500).collect(), Value::String(v.chars().take(1000).collect())))).collect()).unwrap_or_default();
                if !aid.is_empty() {
                    sensors::claudehook::answer_with(&aid, &behavior, &text, (!answers.is_empty()).then_some(Value::Object(answers)));
                }
            }
            if let Some(h) = o.get("claude_hooks").and_then(|h| h.as_object()) {
                let action = h.get("action").and_then(|v| v.as_str()).unwrap_or("");
                // (only when the person pressed the button for it)
                if by_user && matches!(action, "install" | "remove") {
                    let flag = |k: &str| h.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
                    sensors::claudehook::set_up(action, flag("permission"), flag("stop"), h.get("wait_s").and_then(|v| v.as_u64()).unwrap_or(20));
                }
            }
        }
        let results = self.plugins[i].slot.as_ref().map(|s| s.results.clone());
        if let Some(results) = results {
            for r in o.get("run").and_then(|r| r.as_array()).into_iter().flatten().take(4) {
                let rid: String = r.get("id").and_then(|i| i.as_str()).unwrap_or("").chars().take(40).collect();
                let line: Vec<String> = r.get("line").and_then(|l| l.as_array()).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default();
                if !rid.is_empty() && crate::pluginrun::allowed(&perms.run, &line) {
                    let q = results.clone();
                    std::thread::spawn(move || {
                        let r = crate::pluginrun::run(&rid, &line);
                        let mut q = q.lock().unwrap();
                        q.push(r);
                        if q.len() > 16 {
                            q.remove(0);
                        }
                    });
                }
            }
            if !perms.net.is_empty() {
                let hosts = self.hosts_of(i);
                for req in o.get("fetch").and_then(|f| f.as_array()).into_iter().flatten().take(2) {
                    let (q, hosts, req) = (results.clone(), hosts.clone(), req.clone());
                    std::thread::spawn(move || {
                        let r = crate::pluginrun::fetch(&hosts, &req);
                        let mut q = q.lock().unwrap();
                        q.push(r);
                        if q.len() > 16 {
                            q.remove(0);
                        }
                    });
                }
            }
        }
    }

    /// The hosts a plugin may reach: its manifest's, with `{{settings.key}}` read from that setting (the host of the address in it).
    fn hosts_of(&self, i: usize) -> Vec<String> {
        let p = &self.plugins[i];
        let values = self.values_of(p);
        p.manifest
            .permissions
            .net
            .iter()
            .filter_map(|h| match h.strip_prefix("{{settings.").and_then(|r| r.strip_suffix("}}")) {
                Some(key) => {
                    let url = values[key].as_str().unwrap_or("").trim();
                    let rest = url.split_once("://").map_or(url, |(_, r)| r);
                    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
                    (!host.is_empty() && !host.contains('@')).then(|| host.to_string())
                }
                None => Some(h.clone()),
            })
            .collect()
    }

    /// One banner every few seconds from a plugin, and the same words not again within a minute (one with a key of its own is the
    /// plugin's to repeat); what asks something of the person (priority 90 and more) is never held back by the gap.
    fn say_allowed(&mut self, i: usize, s: &Value) -> bool {
        let Some(slot) = self.plugins[i].slot.as_mut() else { return false };
        let pill = s.get("pill");
        let key = format!(
            "{}|{}|{}",
            s["text"].as_str().unwrap_or(""),
            pill.and_then(|p| p["tone"].as_str()).unwrap_or(""),
            pill.and_then(|p| p["sub"].as_str()).unwrap_or("")
        );
        let named = pill.and_then(|p| p["key"].as_str()).is_some_and(|k| !k.is_empty());
        // (a live one, out a moment ago: its words go on changing, as they change)
        if named && pill.is_some_and(|p| p["live"] == true) {
            let lk = format!("\u{0}live|{}", pill.and_then(|p| p["key"].as_str()).unwrap_or(""));
            if slot.said.get(&lk).is_some_and(|t| t.elapsed() < LIVE_FOR) {
                slot.said.insert(lk, Instant::now());
                return true;
            }
            let gap = slot.last_say.is_some_and(|t| t.elapsed() < SAY_GAP);
            if gap {
                return false;
            }
            slot.last_say = Some(Instant::now());
            slot.said.insert(lk, Instant::now());
            return true;
        }
        let urgent = s["priority"].as_i64().unwrap_or(0) >= 90;
        let gap = !urgent && slot.last_say.is_some_and(|t| t.elapsed() < SAY_GAP);
        if gap || (!named && slot.said.get(&key).is_some_and(|t| t.elapsed() < SAY_REPEAT)) {
            return false;
        }
        slot.last_say = Some(Instant::now());
        slot.said.insert(key, Instant::now());
        slot.said.retain(|_, t| t.elapsed() < SAY_REPEAT);
        true
    }
}

/// Run the plugins on a thread of their own: the commands come in, the updates go out, and `wake` is called when there are some (the
/// shell's window wakes up to read them).
pub fn spawn(dir: PathBuf, saved: Option<PathBuf>, pc: Arc<Pc>, wake: impl Fn() + Send + 'static) -> (Sender<Command>, Receiver<Update>) {
    let (tx_cmd, rx_cmd) = std::sync::mpsc::channel::<Command>();
    let (tx_up, rx_up) = std::sync::mpsc::channel::<Update>();
    std::thread::Builder::new()
        .name("nadi-plugins".into())
        .spawn(move || {
            let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
            let mut host = Host::new(dir, saved, pc);
            loop {
                match rx_cmd.recv_timeout(Duration::from_millis(50)) {
                    Ok(c) => match c {
                        Command::SetOn { id, on } => host.set_on(&id, on),
                        Command::SetValue { id, key, value } => host.set_value(&id, &key, value),
                        Command::Action { plugin, action, args } => host.press(&plugin, &action, args),
                        Command::Open { plugin, widget, open } => host.set_open(&plugin, &widget, open),
                        Command::Seen { plugin, seen } => host.set_seen(&plugin, seen),
                        Command::OpenUrl { plugin, url } => {
                            if host.may_open(&plugin, &url) {
                                crate::win::open_url(&url);
                            }
                        }
                        Command::Rescan => host.scan(),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                host.step(paths::now_ms());
                let ups = host.take_updates();
                if !ups.is_empty() {
                    for u in ups {
                        if tx_up.send(u).is_err() {
                            return;
                        }
                    }
                    wake();
                }
            }
        })
        .expect("the plugins' thread");
    (tx_cmd, rx_up)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_dir(name: &str, plugins: &[(&str, &str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nadi-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (id, manifest, main) in plugins {
            let p = d.join(id);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("manifest.json"), manifest).unwrap();
            std::fs::write(p.join("main.luau"), main).unwrap();
        }
        d
    }

    const BAT: &str = r#"{ "id": "bat", "name": "Bat", "kind": "luau", "permissions": { "battery": true }, "widgets": [{ "id": "w", "name": "W" }],
        "settings": { "low": { "label": "Low", "type": "range", "default": 20 } } }"#;
    const BAT_MAIN: &str = r#"
        function handle(ev)
            if ev.event == "battery" then
                local out = { widgets = { w = { text = ev.battery.percent .. "%" } } }
                if ev.battery.percent <= ev.settings.low then out.say = { { text = "Battery low", priority = 60 } } end
                return out
            elseif ev.event == "action" then
                return { note = "pressed " .. ev.action }
            end
        end"#;

    fn host(name: &str) -> (Host, PathBuf) {
        let d = plugin_dir(name, &[("bat", BAT, BAT_MAIN), ("old", r#"{ "id": "old", "kind": "wasm" }"#, "")]);
        let saved = d.join("saved.json");
        let mut h = Host::new(d.clone(), Some(saved.clone()), Pc::new());
        h.dry = true;
        h.battery_reader = Box::new(|| json!({ "present": true, "percent": 64.0, "plugged": false, "charging": false, "saver": false }));
        (h, saved)
    }

    #[test]
    fn a_plugin_that_is_on_is_told_the_battery_and_its_widget_shows() {
        let (mut h, _) = host("widget");
        assert!(h.info().iter().any(|p| p.id == "old" && p.problem.as_deref().is_some_and(|x| x.contains("wasm"))));
        h.step(1000);
        assert!(h.widget("bat", "w").is_none(), "off: nothing");
        h.set_on("bat", true);
        h.take_updates();
        h.step(2000);
        let ups = h.take_updates();
        assert!(ups.contains(&Update::Widget { plugin: "bat".into(), widget: "w".into(), content: json!({ "text": "64%" }) }), "{ups:?}");
        h.step(3000);
        assert!(h.take_updates().is_empty(), "told again only when it changes");
    }

    #[test]
    fn a_service_is_called_and_its_events_heard_and_what_needs_it_follows_it() {
        let spk = r#"{ "id": "spk", "name": "Spk", "kind": "luau", "provides": { "speech": "1.2" } }"#;
        let spk_main = r#"
            function handle(ev)
                if ev.event == "call" and ev.what == "listen" then
                    return { emit = { name = "speech.line", data = { text = "hello " .. ev.from } } }
                end
            end"#;
        let cap = r#"{ "id": "cap", "name": "Cap", "kind": "luau", "needs": { "speech": "^1.0" }, "widgets": [{ "id": "w", "name": "W" }] }"#;
        let cap_main = r#"
            function handle(ev)
                if ev.event == "start" then
                    return { call = { service = "speech", what = "listen" }, subscribe = { "speech.line", "battery.x" } }
                elseif ev.event == "speech.line" then
                    return { widgets = { w = { text = ev.text .. " from " .. ev.from } } }
                end
            end"#;
        let d = plugin_dir("service", &[("spk", spk, spk_main), ("cap", cap, cap_main)]);
        let mut h = Host::new(d.clone(), None, Pc::new());
        h.dry = true;
        h.set_on("cap", true);
        assert!(h.is_on("spk"), "what it needs came on with it");
        for t in 1..6 {
            h.step(t * 1000);
        }
        assert_eq!(h.widget("cap", "w"), Some(&json!({ "text": "hello cap from spk" })));
        assert!(!h.plugins.iter().find(|p| p.manifest.id == "cap").unwrap().subs.contains("battery.x"), "only the events of what it needs");
        h.set_on("spk", false);
        assert!(!h.is_on("cap"), "what needs it went off with it");
        assert!(manifest::version_meets("1.2", "^1.0") && !manifest::version_meets("2.0", "^1.0") && !manifest::version_meets("1.0", "1.1"));
    }

    #[test]
    fn settings_are_its_values_and_a_banner_is_said() {
        let (mut h, saved) = host("settings");
        h.set_on("bat", true);
        h.set_value("bat", "low", json!(70));
        h.take_updates();
        h.step(1000);
        let ups = h.take_updates();
        assert!(ups.iter().any(|u| matches!(u, Update::Say { plugin, say } if plugin == "bat" && say["text"] == "Battery low")), "{ups:?}");
        let kept: Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
        assert_eq!(kept["on"], json!(["bat"]));
        assert_eq!(kept["values"]["bat"]["low"], 70);
    }

    #[test]
    fn a_press_reaches_it_and_switching_off_takes_its_widgets() {
        let (mut h, _) = host("press");
        h.set_on("bat", true);
        h.step(1000);
        h.press("bat", "go", json!({}));
        h.step(1100);
        assert_eq!(h.info().iter().find(|p| p.id == "bat").unwrap().note.as_deref(), Some("pressed go"));
        h.take_updates();
        h.set_on("bat", false);
        assert!(h.take_updates().contains(&Update::Widget { plugin: "bat".into(), widget: "w".into(), content: Value::Null }));
    }

    #[test]
    fn a_plugin_that_keeps_failing_is_stopped() {
        let d = plugin_dir("fail", &[("bad", r#"{ "id": "bad", "kind": "luau", "tick": "100ms" }"#, "function handle(ev) error('boom') end")]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.set_on("bad", true);
        for k in 0..5 {
            std::thread::sleep(Duration::from_millis(110));
            h.step(k * 200);
        }
        let note = h.info()[0].note.clone().unwrap();
        assert!(note.starts_with("Stopped after failing 3 times"), "{note}");
    }

    #[test]
    fn a_widget_it_does_not_offer_is_ignored() {
        let d = plugin_dir("offer", &[("p", r#"{ "id": "p", "kind": "luau", "widgets": [{ "id": "a" }] }"#, "function handle(ev) return { widgets = { a = { text = 'x' }, b = { text = 'y' } } } end")]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.set_on("p", true);
        h.step(0);
        assert!(h.widget("p", "a").is_some());
        assert!(h.widget("p", "b").is_none());
    }

    #[test]
    fn banners_are_held_to_one_every_few_seconds_and_not_repeated() {
        let d = plugin_dir("say", &[("p", r#"{ "id": "p", "kind": "luau", "tick": "100ms" }"#, "local n = 0\nfunction handle(ev) n += 1 return { say = { { text = 'hello' } } } end")]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.set_on("p", true);
        let mut said = 0;
        for k in 0..4 {
            std::thread::sleep(Duration::from_millis(110));
            h.step(k * 100);
            said += h.take_updates().iter().filter(|u| matches!(u, Update::Say { .. })).count();
        }
        assert_eq!(said, 1);
    }

    #[test]
    fn a_plugin_none_of_whose_widgets_can_be_seen_is_ticked_seldom() {
        let m = r#"{ "id": "p", "kind": "luau", "tick": "100ms", "widgets": [{ "id": "w" }] }"#;
        let d = plugin_dir("seen", &[("p", m, "local n = 0
function handle(ev) if ev.event == 'tick' then n += 1 end return { note = tostring(n) } end")]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.set_on("p", true);
        h.set_seen("p", false);
        let mut ticks = 0;
        for k in 0..6 {
            std::thread::sleep(Duration::from_millis(110));
            h.step(k * 100);
            ticks += h.take_updates().len();
        }
        let unseen = ticks;
        h.set_seen("p", true);
        let mut ticks = 0;
        for k in 6..12 {
            std::thread::sleep(Duration::from_millis(110));
            h.step(k * 100);
            ticks += h.take_updates().len();
        }
        // (unseen: its start, and one tick at most; seen: a tick each step)
        assert!(unseen <= 3 && ticks >= 5, "unseen {unseen}, seen {ticks}");
    }

    #[test]
    fn a_live_banner_follows_its_words_as_they_change() {
        let main = "local n = 0
function handle(ev) n += 1 return { say = { { text = 'words ' .. n, pill = { key = 'talking', live = true } } } } end";
        let d = plugin_dir("live", &[("p", r#"{ "id": "p", "kind": "luau", "tick": "100ms" }"#, main)]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.set_on("p", true);
        let mut said = 0;
        for k in 0..4 {
            std::thread::sleep(Duration::from_millis(110));
            h.step(k * 100);
            said += h.take_updates().iter().filter(|u| matches!(u, Update::Say { .. })).count();
        }
        assert!(said >= 3, "{said}");
    }

    #[test]
    fn an_open_panel_is_told_and_seen_with_every_event() {
        let m = r#"{ "id": "p", "kind": "luau", "widgets": [{ "id": "w" }] }"#;
        let main = "function handle(ev) if ev.event == 'open' then return { note = ev.widget .. tostring(ev.open), quiet = true } elseif ev.event == 'action' then return { note = tostring(ev.open.w) } end end";
        let d = plugin_dir("open", &[("p", m, main)]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.set_on("p", true);
        h.step(0);
        h.set_open("p", "w", true);
        h.step(1);
        assert_eq!(h.info()[0].note.as_deref(), Some("wtrue"));
        assert!(h.take_updates().contains(&Update::Quiet { plugin: "p".into(), on: true }));
        h.press("p", "x", json!({}));
        h.step(2);
        assert_eq!(h.info()[0].note.as_deref(), Some("true"));
        h.set_on("p", false);
        assert!(h.take_updates().contains(&Update::Quiet { plugin: "p".into(), on: false }));
    }

    #[test]
    fn a_host_can_come_from_a_setting() {
        let m = r#"{ "id": "p", "kind": "luau", "permissions": { "net": ["{{settings.url}}", "api.example.com"] },
            "settings": { "url": { "type": "text", "default": "" } } }"#;
        let d = plugin_dir("hosts", &[("p", m, "function handle(ev) end")]);
        let mut h = Host::new(d, None, Pc::new());
        assert_eq!(h.hosts_of(0), vec!["api.example.com".to_string()]);
        h.set_value("p", "url", json!("webcal://calendar.google.com/calendar/ical/x/basic.ics"));
        assert_eq!(h.hosts_of(0), vec!["calendar.google.com".to_string(), "api.example.com".to_string()]);
    }

    #[test]
    fn what_it_sees_is_by_its_permissions() {
        // (no battery permission: never told the battery)
        let d = plugin_dir("perm", &[("p", r#"{ "id": "p", "kind": "luau", "widgets": [{ "id": "w" }] }"#, "function handle(ev) if ev.event == 'battery' then return { widgets = { w = { text = 'saw it' } } } end end")]);
        let mut h = Host::new(d, None, Pc::new());
        h.dry = true;
        h.battery_reader = Box::new(|| json!({ "present": true, "percent": 50.0 }));
        h.set_on("p", true);
        h.step(0);
        assert!(h.widget("p", "w").is_none());
    }
}

/// A link a manifest allows with a star for its name ("https://*.fandom.com/wiki/"): any one name under that host, the rest of the
/// link starting as the manifest says.
fn open_under(prefix: &str, url: &str) -> bool {
    let Some(rest) = prefix.strip_prefix("https://*.") else { return false };
    let (base, path) = rest.split_once('/').unwrap_or((rest, ""));
    let Some(after) = url.strip_prefix("https://") else { return false };
    let url_path = after.split_once('/').map(|(_, p)| p).unwrap_or("");
    crate::pluginrun::net_allows(&[format!("*.{base}")], url) && url_path.to_lowercase().starts_with(&path.to_lowercase())
}
