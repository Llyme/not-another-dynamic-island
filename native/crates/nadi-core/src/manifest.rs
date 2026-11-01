//! A plugin's `manifest.json`: what it is, what it may see (its permissions), what can be set, and the widgets it offers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// What a plugin may see and do. It is told nothing it has not asked for here, and the person is shown this list before it is switched on.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Permissions {
    /// the hosts it may ask (`host` or `host:port`)
    pub net: Vec<String>,
    /// what is playing, as a level and bands (never the sound)
    pub audio: bool,
    /// how full the battery is, the charger, the time left, the wear
    pub battery: bool,
    /// ... and how much of it each app used (NADI's own count)
    pub app_power: bool,
    /// the island listens for it, to draw the sound as bars on its widgets (it is told nothing of the sound)
    pub viz: bool,
    /// which programs have a window, which is in front
    pub processes: bool,
    /// ... and their windows' titles
    pub titles: bool,
    /// files it may read (a path each, `~` and `%VARS%` expanded)
    pub files: Vec<String>,
    pub calendar: bool,
    pub calendar_past_days: u32,
    /// what is playing (the title, the artist, how far along)
    pub media: bool,
    /// ... its cover picture
    pub media_art: bool,
    /// ... and press its buttons
    pub media_control: bool,
    /// how busy the PC is
    pub system: bool,
    pub cursor: bool,
    /// the notifications of other apps
    pub notifications: bool,
    /// how long since the keyboard or the mouse was used
    pub idle: bool,
    /// a program's path and icon, and bringing its window to the front
    pub programs: bool,
    /// a little JSON of its own, kept between runs
    pub storage: bool,
    /// how much CPU and memory the programs it watches use
    pub usage: bool,
    /// what the browsers, Steam and torrent clients download
    pub downloads: bool,
    /// the Claude Code sessions on this PC
    pub claude: bool,
    /// the Steam games installed, the one running, and their pictures
    pub steam: bool,
    /// the controllers on, and the batteries of the Bluetooth devices
    pub gear: bool,
    /// ... and answering what they ask
    pub claude_answer: bool,
    /// the pages open in the browsers (with the NADI extension)
    pub browser: bool,
    /// commands it may run (each line as given, nothing else)
    pub run: Vec<String>,
    /// links it may open (they must start with one of these)
    pub open: Vec<String>,
    /// (its helper) hear the microphone
    pub microphone: bool,
    /// (its helper) hear what the computer plays
    pub system_audio: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct OptionSpec {
    pub value: Value,
    pub label: String,
}

/// One setting of a plugin: a `toggle`, a `choice` (of `options`), a `range` (`min`..`max` by `step`, with a `unit`), `number` or `text`.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct SettingSpec {
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Value,
    pub options: Vec<OptionSpec>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub unit: String,
}

/// A button in the plugin's settings (Sign In); `ask` makes it a field, whose text goes with the action.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct ActionSpec {
    pub id: String,
    pub label: String,
    pub ask: String,
    pub then: String,
}

/// A widget the plugin offers: the person puts it on an edge or in a corner. `place` is where it goes when it is first switched on.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct WidgetSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    /// "top", "bottom", "left", "right", "top-left", ..., or "" (not placed until the person does)
    pub place: String,
    /// the order among the widgets of one edge (lower first)
    pub rank: i32,
    /// at rest, its icon alone (its words in its tip): the person can change it
    pub compact: bool,
    /// at rest, large (a deeper piece, everything in it bigger): the person can change it
    pub large: bool,
    /// its icon alone, always (it has nothing more to say at rest): the person cannot change it
    pub icon_only: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Budget {
    /// one call, ms
    pub call_ms: u64,
    pub memory_mb: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { call_ms: 50, memory_mb: 16 }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// "luau" (a script), or "face" (one of the frame's own looks, which it switches on: `look`); anything else is not a plugin this
    /// NADI runs
    pub kind: String,
    /// a face's look: "eyes", "aura"
    pub look: String,
    /// the script (`main.luau`)
    pub main: String,
    pub permissions: Permissions,
    pub settings: BTreeMap<String, SettingSpec>,
    pub actions: Vec<ActionSpec>,
    pub widgets: Vec<WidgetSpec>,
    /// it is called this often with a `tick` ("5s", "1m"); "" for never
    pub tick: String,
    pub budget: Budget,
    pub default_on: bool,
    /// it says things in banners
    pub banners: bool,
    /// the plugins it takes the place of
    pub replaces: Vec<String>,
    /// the services it gives other plugins (name: version, "1.0")
    pub provides: BTreeMap<String, String>,
    /// the services it uses (name: the versions that will do, "^1.0", "1")
    pub needs: BTreeMap<String, String>,
    /// its own program, for each system ("windows": "bin/helper.exe"), which NADI runs while it is on
    pub helper: BTreeMap<String, String>,
}

impl Manifest {
    /// Its helper on this system, as a path in its folder (none: it has none).
    pub fn helper_path(&self) -> Option<&str> {
        self.helper.get("windows").map(String::as_str).filter(|h| !h.is_empty())
    }
}

/// Does a version ("1.2") meet what is asked ("^1.0": the same major, at least that; "1": that major; "1.2.0": at least that, same
/// major; "*" or "": any)?
pub fn version_meets(have: &str, want: &str) -> bool {
    let want = want.trim().trim_start_matches(['^', '~', '=', 'v']);
    if want.is_empty() || want == "*" {
        return true;
    }
    let parse = |s: &str| s.trim().trim_start_matches('v').split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (h, w) = (parse(have), parse(want));
    if h.first() != w.first() {
        return false;
    }
    let get = |v: &Vec<u64>, i: usize| v.get(i).copied().unwrap_or(0);
    (get(&h, 1), get(&h, 2)) >= (get(&w, 1), get(&w, 2))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_')) && id != "." && id != ".."
}

/// "5s", "1m", "250ms", "2h" (else none)
pub fn parse_every(s: &str) -> Option<std::time::Duration> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len()));
    let n: f64 = num.parse().ok()?;
    let ms = match unit {
        "ms" => n,
        "s" | "" => n * 1000.0,
        "m" => n * 60_000.0,
        "h" => n * 3_600_000.0,
        _ => return None,
    };
    (ms >= 100.0).then(|| std::time::Duration::from_millis(ms as u64))
}

/// Does the manifest ask for a permission this NADI does not know? (it would run, and see nothing: a newer NADI is needed)
pub fn unknown_permissions(raw: &Value) -> bool {
    let Some(Value::Object(known)) = serde_json::to_value(Permissions::default()).ok() else { return false };
    raw.get("permissions").and_then(|p| p.as_object()).is_some_and(|p| p.keys().any(|k| !known.contains_key(k)))
}

/// Read a plugin's folder: its manifest, and what is wrong with it (it is listed all the same, and cannot be switched on).
pub fn read(dir: &Path) -> Option<(Manifest, Option<String>)> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    // (a file saved by Notepad or PowerShell may begin with a byte-order mark)
    let text = text.trim_start_matches('\u{feff}').to_string();
    let raw: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let id = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return Some((Manifest { id: id.clone(), name: id, ..Default::default() }, Some(format!("The manifest is not valid JSON ({e})."))));
        }
    };
    let mut m: Manifest = match serde_json::from_value(raw.clone()) {
        Ok(m) => m,
        Err(e) => {
            let id = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return Some((Manifest { id: id.clone(), name: id, ..Default::default() }, Some(format!("The manifest has a mistake ({e})."))));
        }
    };
    if m.name.is_empty() {
        m.name = m.id.clone();
    }
    if m.main.is_empty() {
        m.main = "main.luau".into();
    }
    let problem = if !valid_id(&m.id) {
        Some("The id must be lower case letters, digits, '.', '-' or '_'.".to_string())
    } else if m.kind == "face" {
        if matches!(m.look.as_str(), "eyes" | "aura") {
            None
        } else {
            Some("It is a face this NADI does not know.".to_string())
        }
    } else if m.kind != "luau" {
        Some(format!("It is a {} plugin, which this NADI does not run.", if m.kind.is_empty() { "declarative" } else { &m.kind }))
    } else if unknown_permissions(&raw) {
        Some("It needs a newer NADI: it asks for something this one does not have.".to_string())
    } else if !dir.join(&m.main).is_file() || m.main.contains("..") || m.main.contains(['/', '\\']) {
        Some(format!("{} is not in its folder.", m.main))
    } else if m.helper_path().is_some_and(|h| h.contains("..") || Path::new(h).is_absolute() || !dir.join(h).is_file()) {
        Some(format!("Its program ({}) is not in its folder.", m.helper_path().unwrap_or("")))
    } else {
        None
    };
    Some((m, problem))
}

/// What a plugin may do, in the words the person is shown, its own program first (when it has one).
pub fn manifest_words(m: &Manifest) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    if let Some(h) = m.helper_path() {
        let file = Path::new(h).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        out.push(("own program".to_string(), file, "Run a program it brings, while it is on.".to_string()));
    }
    out.extend(permission_words(&m.permissions));
    out
}

/// What the plugin may see, in the words the person is shown: (the name of the permission, a detail, why it matters).
pub fn permission_words(p: &Permissions) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for h in &p.net {
        out.push(("network".into(), h.clone(), "Reach this address on the internet.".into()));
    }
    let flags: [(bool, &str, &str); 26] = [
        (p.audio, "audio", "Hear what is playing, as a level and bands. Never the sound itself."),
        (p.viz, "sound bars", "Show what is playing as bars that move with the sound."),
        (p.battery, "battery", "See how full the battery is, the charger, the time left and the wear."),
        (p.app_power, "app power", "See how much of the battery each app used."),
        (p.processes, "processes", "See which programs have a window, and which one is in front."),
        (p.titles, "titles", "See the titles of the windows."),
        (p.calendar, "calendar", "See the events of your calendar."),
        (p.media, "media", "See what is playing: the title, the artist and how far along it is."),
        (p.media_art, "media pictures", "See the cover picture of what is playing."),
        (p.media_control, "media control", "Press play, pause, next and previous for the player."),
        (p.system, "system", "See how busy the PC is."),
        (p.cursor, "cursor position", "See where the pointer is."),
        (p.notifications, "notifications", "See the notifications other apps show."),
        (p.idle, "idle time", "See how long since the keyboard or the mouse was used."),
        (p.programs, "program details", "See a program's path and icon, and bring its window to the front."),
        (p.storage, "storage", "Keep a little of its own between runs."),
        (p.usage, "performance", "See how much CPU and memory the programs it watches use."),
        (p.downloads, "downloads", "See what your browsers, Steam and torrent clients download."),
        (p.claude, "claude sessions", "See your Claude Code sessions on this PC."),
        (p.steam, "steam library", "See the Steam games installed and the one you play."),
        (p.gear, "controllers", "See your controllers and Bluetooth devices, and how full their batteries are."),
        (p.claude_answer, "answer claude sessions", "Answer what a Claude Code session asks, from its card."),
        (p.browser, "browser extension", "See the pages open in your browsers, with the NADI extension."),
        (!p.run.is_empty(), "commands", "Run the commands its manifest lists."),
        (p.microphone, "microphone", "Hear the microphone."),
        (p.system_audio, "system audio", "Hear what the computer plays."),
    ];
    for (on, name, why) in flags {
        if on {
            out.push((name.into(), String::new(), why.into()));
        }
    }
    for f in &p.files {
        out.push(("files".into(), f.clone(), "Read this file.".into()));
    }
    for o in &p.open {
        out.push(("open links".into(), o.clone(), "Open links that start with this.".into()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn folder(name: &str, manifest: &str, main: bool) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nadi-manifest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), manifest).unwrap();
        if main {
            std::fs::write(d.join("main.luau"), "function handle(ev) end").unwrap();
        }
        d
    }

    #[test]
    fn a_luau_plugin_is_read() {
        let d = folder("ok", r#"{ "id": "battery", "name": "Battery", "kind": "luau", "permissions": { "battery": true }, "widgets": [{ "id": "battery", "name": "Battery", "place": "right" }] }"#, true);
        let (m, problem) = read(&d).unwrap();
        assert_eq!((m.id.as_str(), m.main.as_str(), problem), ("battery", "main.luau", None));
        assert!(m.permissions.battery);
        assert_eq!(m.widgets[0].place, "right");
    }

    #[test]
    fn what_cannot_run_says_why() {
        let (_, p) = read(&folder("wasm", r#"{ "id": "x", "kind": "wasm" }"#, true)).unwrap();
        assert!(p.unwrap().contains("wasm"));
        let (_, p) = read(&folder("nomain", r#"{ "id": "x", "kind": "luau" }"#, false)).unwrap();
        assert!(p.unwrap().contains("main.luau"));
        let (_, p) = read(&folder("newer", r#"{ "id": "x", "kind": "luau", "permissions": { "teleport": true } }"#, true)).unwrap();
        assert!(p.unwrap().contains("newer"));
        let (_, p) = read(&folder("badid", r#"{ "id": "Bad Id", "kind": "luau" }"#, true)).unwrap();
        assert!(p.unwrap().contains("lower case"));
        let (m, p) = read(&folder("broken", "{ nope", true)).unwrap();
        assert!(p.unwrap().contains("JSON"));
        assert!(m.id.starts_with("nadi-manifest-broken"));
    }

    #[test]
    fn times_are_read() {
        assert_eq!(parse_every("5s"), Some(std::time::Duration::from_secs(5)));
        assert_eq!(parse_every("1m"), Some(std::time::Duration::from_secs(60)));
        assert_eq!(parse_every("250ms"), Some(std::time::Duration::from_millis(250)));
        assert_eq!(parse_every("10ms"), None, "never faster than 100 ms");
        assert_eq!(parse_every(""), None);
    }

    #[test]
    fn permissions_are_said_in_words() {
        let p = Permissions { battery: true, net: vec!["api.example.com".into()], ..Default::default() };
        let w = permission_words(&p);
        assert_eq!(w[0].0, "network");
        assert_eq!(w[1].0, "battery");
        assert!(!unknown_permissions(&json!({ "permissions": { "battery": true, "claude_answer": true } })));
    }
}
