//! Plugins: a feature that is a folder with a `manifest.json`, not code in the app.
//!
//! The island is a window manager for cards; a plugin hands it data and the core draws it from a few building
//! blocks (progress, bars, list, facts, text, badge, buttons, image). A plugin never draws pixels and never reaches
//! Windows or the network by itself: it says in its manifest what it may see, the user switches it on, and the host
//! enforces it.
//!
//! Two kinds. A declarative plugin names a source (an address that is polled: JSON, or a calendar), and its card, its
//! pill, its rings and its attention rule are written as templates over what the source answers: no plugin code runs.
//! A module plugin is a WebAssembly file that the host calls with events (what is playing, the calendar, which programs
//! are open, a file that changed, a button that was pressed) and that answers with what the island is to do (`Out`: a
//! card, a pill, rings, banners, events for the calendar); it is run by `pluginwasm.rs`, has no network, and is cut off
//! when it is slow. A third kind, a native plugin (native.rs), is Rust inside the app; it reaches the screen the same way.
//! Some plugins are carried by the app (bundled.rs) and are read from what is embedded; they are plugins like the rest.
//!
//! Where they live: `%APPDATA%\NADI\plugins\<id>\manifest.json`. What is switched on, each plugin's settings, who
//! is muted and do not disturb are kept in `%APPDATA%\NADI\plugins.json`. A plugin that was just dropped in is off
//! until the user turns it on.
//!
//! Budgets (so a plugin cannot make the island lag): one request at a time per plugin, a 5 s time limit, at most 1 MB
//! of answer, no redirects, a poll no faster than every second, and polling only while a card could be seen (the hub
//! is open, or a card floats). A request that fails keeps the last good card on screen.

use crate::IslandState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const MAX_ANSWER: u64 = 1_000_000;
const MIN_EVERY: Duration = Duration::from_secs(1);
const DEFAULT_EVERY: Duration = Duration::from_secs(30);
const TIME_LIMIT: Duration = Duration::from_secs(5);
const MAX_ROWS: usize = 12;
const MAX_BARS: usize = 32;
const MAX_TEXT: usize = 200;
/// a live card, a pill or a ring goes to the screen at most this often
const LIVE_GAP: Duration = Duration::from_millis(50);

// ------------------------------------------------------------------------------------------------- the manifest

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// "declarative" (this file is all there is) or "wasm" (a module too)
    pub kind: String,
    /// a module plugin: its file, in the plugin's folder
    pub module: String,
    pub permissions: Permissions,
    pub source: Source,
    pub card: CardSpec,
    pub attention: Option<AttentionSpec>,
    /// what the user can set (shown in Settings > Plugins; read by the templates and the module as `settings`)
    pub settings: BTreeMap<String, SettingSpec>,
    /// a module plugin: how often it is called with nothing else to say ("5s"); 0 = only when something happens
    pub tick: String,
    /// a module plugin: its card is sent to the screen as it changes (up to about 20 times a second), for a meter
    pub live: bool,
    pub budget: Budget,
    /// a native plugin: it starts switched on in a new install
    pub default_on: bool,
    /// a native plugin: it says things in banners (so there is something to mute)
    pub banners: bool,
    /// a native plugin: the pills it may offer (see `Ctx::offer_pill`)
    pub pills: Vec<PillSpec>,
    /// buttons that go in the plugin's settings (a native plugin answers them by `Native::call`, a module by an
    /// `action` event)
    pub actions: Vec<ActionSpec>,
    /// a declarative plugin: the pill it shows while its `when` holds (templates over the data)
    pub pill: Option<PillTpl>,
    /// a declarative plugin: rings in the hub (templates over the data)
    pub rings: Vec<RingTpl>,
    /// a declarative plugin: a line about how it is doing, shown under its name (a template over the data)
    pub status: String,
    /// a declarative plugin: settings that must be filled in before it asks anything, and what to say until then
    pub needs: Vec<String>,
    pub setup: String,
}

/// The pill of a declarative plugin: shown while `when` is true (any text but "", "0" and "false").
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct PillTpl {
    pub when: String,
    pub icon: String,
    pub title: String,
    pub sub: String,
    pub right: String,
    pub progress: String,
    pub rank: i32,
    pub w: f64,
    pub h: f64,
}

/// A ring in the hub (next to the CPU and memory ones) of a declarative plugin.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct RingTpl {
    pub id: String,
    pub color: String,
    /// 0 to 100 (a template that gives a number); a ring with none is not shown
    pub pct: String,
    pub tip: String,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct PillSpec {
    pub id: String,
    /// the size at the standard collapsed width (260 px), logical px
    pub w: f64,
    pub h: f64,
    /// who wins when several are offered (the highest)
    pub rank: i32,
    /// it comes by itself and is looked at (like a banner): it does not slide away on its own, and a pinned island does
    /// not shrink under it
    pub brief: bool,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct ActionSpec {
    pub id: String,
    pub label: String,
    /// the action needs a word from the user (a code to paste): this is what the field asks, and `then` is the call
    /// that takes it (as `{"text": ...}`) once the action itself has run
    pub ask: String,
    pub then: String,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct OptionSpec {
    pub value: Value,
    pub label: String,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct SettingSpec {
    pub label: String,
    /// "text", "number" or "toggle"
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Value,
    /// "choice": what it is one of
    pub options: Vec<OptionSpec>,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Budget {
    /// the time one call into a module may take (default 4 ms, most 20)
    pub call_ms: u64,
    /// the memory a module may have (default 8 MB, most 64)
    pub memory_mb: u64,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Permissions {
    /// hosts a declarative plugin may ask (`host` or `host:port`), and nothing else
    pub net: Vec<String>,
    /// a module may hear what is playing (levels and bands, never the sound)
    pub audio: bool,
    /// a module may see which programs have a window, and which is in front
    pub processes: bool,
    /// ... and the titles of those windows
    pub titles: bool,
    /// a module may read these files when they change (`~` and `%VARIABLES%` are filled in)
    pub files: Vec<String>,
    /// a module may read the events of the island's calendar (whoever brought them)
    pub calendar: bool,
    /// a module may see what is playing (title, artist, position; never the picture)
    pub media: bool,
    /// ... and press play, pause, next and previous for it
    pub media_control: bool,
    /// a module may read how busy the CPU and the memory are
    pub system: bool,
    /// a native plugin says in words what it looks at (shown when it is switched on)
    pub sees: Vec<Sees>,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Sees {
    pub name: String,
    /// what it is of (`use CLI`: the command)
    pub detail: String,
    pub why: String,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Source {
    pub http: String,
    /// "2s", "500ms", "15m"
    pub every: String,
    /// where in the answer the data is (dotted), when it is not the whole answer
    pub pick: String,
    /// "visible" (the default: only while a card could be seen) or "always"
    pub when: String,
    /// what the address answers: "json" (the default) or "ics" (a calendar: its events go to the island's calendar)
    pub format: String,
    /// the most the answer may be, in KB (1000 by default, 8000 at most)
    pub max_kb: u64,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct CardSpec {
    pub icon: String,
    /// where the card sits among the others (a lower number comes first; 70 by default)
    pub rank: i32,
    pub title: String,
    pub sub: String,
    /// what the collapsed card says at its right
    pub peek: String,
    pub body: Vec<Value>,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct AttentionSpec {
    /// a list in the data to watch, by the field that tells its items apart (`id`); without `list`, one value
    pub list: String,
    pub id: String,
    pub field: String,
    /// the field reached this number (a progress that got to 1)
    pub reaches: Option<f64>,
    /// the field became this text
    pub becomes: Option<String>,
    pub priority: i32,
    pub banner: String,
}

// ------------------------------------------------------------------------------------------------ what it draws

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Block {
    Progress { value: f64 },
    /// a row of meters, each 0 to 1 (a level, the bands of a sound)
    Bars { values: Vec<f64> },
    Text { text: String },
    Badge { text: String },
    Facts { rows: Vec<(String, String)> },
    List { rows: Vec<Row> },
    /// buttons: each one calls the plugin back (`action`)
    Buttons { items: Vec<Button> },
    /// a picture (a `data:` address of a png, jpeg, webp or gif; nothing else is drawn)
    Image { src: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Row {
    pub title: String,
    pub sub: String,
    pub progress: Option<f64>,
}

/// A button on a card or a pill. Pressing it tells the plugin (an `action` event for a module).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Button {
    pub label: String,
    /// one of the island's line icons (shown instead of the label when there is one; the label stays as its tooltip)
    pub icon: String,
    pub action: String,
}

/// What the island lists for a plugin.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct CardOut {
    pub plugin: String,
    pub name: String,
    pub icon: String,
    /// where the card sits among the others (a lower number comes first)
    pub rank: i32,
    pub title: String,
    pub sub: String,
    pub peek: String,
    pub body: Vec<Block>,
}

/// The pill a plugin offers while something is going on (what is playing, a game, a timer). The collapsed island shows
/// the highest `rank` of what the plugins offer, unless a banner or the hub has it. It is the plugin's to take back.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct PillOut {
    pub plugin: String,
    pub name: String,
    /// a line icon
    pub icon: String,
    pub title: String,
    pub sub: String,
    /// a short text at the right (a time, a count)
    pub right: String,
    /// a thin bar under the text, 0 to 1
    pub progress: Option<f64>,
    /// a row of meters at the right, each 0 to 1
    pub bars: Vec<f64>,
    pub buttons: Vec<Button>,
    /// who wins when several plugins offer a pill (0 to 99; the island's own views are in between)
    pub rank: i32,
    /// the size at the standard width (260): 200 to 360 wide, 32 to 72 high
    pub w: f64,
    pub h: f64,
    /// "" or "big" (large light digits, as the time is shown)
    pub look: String,
}

/// A ring a plugin adds to the hub's top right.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct RingOut {
    pub id: String,
    /// 0 to 100
    pub pct: f64,
    pub tip: String,
    /// `#rrggbb`
    pub color: String,
}

// -------------------------------------------------------------------------------------------------- templates

/// Look a dotted path up (`a.b.0.c`).
fn lookup<'a>(ctx: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = ctx;
    for seg in path.split('.').map(str::trim).filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(*b as u8 as f64),
        _ => None,
    }
}

fn fmt_num(n: f64) -> String {
    if !n.is_finite() {
        return String::new();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{:.2}", n);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_f64().map(fmt_num).unwrap_or_default(),
        Value::Array(a) => a.iter().map(text_of).collect::<Vec<_>>().join(", "),
        Value::Object(_) => String::new(),
    }
}

fn operand(s: &str, ctx: &Value) -> Value {
    let s = s.trim();
    if let Ok(n) = s.parse::<f64>() {
        return json!(n);
    }
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        return Value::String(s[1..s.len() - 1].to_string());
    }
    lookup(ctx, s).cloned().unwrap_or(Value::Null)
}

/// One operator between two operands (`position / duration`), or just an operand.
fn arith(s: &str, ctx: &Value) -> Value {
    for op in [" + ", " - ", " * ", " / "] {
        if let Some(i) = s.find(op) {
            let (l, r) = (num(&operand(&s[..i], ctx)), num(&operand(&s[i + op.len()..], ctx)));
            return match (l, r) {
                (Some(a), Some(b)) => {
                    let v = match op.trim() {
                        "+" => a + b,
                        "-" => a - b,
                        "*" => a * b,
                        _ => {
                            if b == 0.0 {
                                return Value::Null;
                            }
                            a / b
                        }
                    };
                    json!(v)
                }
                _ => Value::Null,
            };
        }
    }
    operand(s, ctx)
}

fn bytes_text(n: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n.max(0.0);
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 || v >= 100.0 {
        format!("{:.0} {}", v, units[i])
    } else {
        format!("{:.1} {}", v, units[i])
    }
}

fn duration_text(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match (s / 3600, (s % 3600) / 60) {
        (0, 0) => format!("{s}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn filter(spec: &str, v: Value) -> Value {
    let (name, arg) = match spec.split_once(':') {
        Some((n, a)) => (n.trim(), a.trim().trim_matches(|c| c == '"' || c == '\'')),
        None => (spec.trim(), ""),
    };
    match name {
        "percent" => num(&v).map(|n| Value::String(format!("{}%", (n * 100.0).round() as i64))).unwrap_or(Value::Null),
        "round" => {
            let d: i32 = arg.parse().unwrap_or(0);
            num(&v).map(|n| json!((n * 10f64.powi(d)).round() / 10f64.powi(d))).unwrap_or(Value::Null)
        }
        "bytes" => num(&v).map(|n| Value::String(bytes_text(n))).unwrap_or(Value::Null),
        "speed" => num(&v).map(|n| Value::String(format!("{}/s", bytes_text(n)))).unwrap_or(Value::Null),
        "duration" => num(&v).map(|n| Value::String(duration_text(n))).unwrap_or(Value::Null),
        "upper" => Value::String(text_of(&v).to_uppercase()),
        "lower" => Value::String(text_of(&v).to_lowercase()),
        "trim" => Value::String(text_of(&v).trim().to_string()),
        "len" => match &v {
            Value::Array(a) => json!(a.len()),
            other => json!(text_of(other).chars().count()),
        },
        "join" => match &v {
            Value::Array(a) => Value::String(a.iter().map(text_of).collect::<Vec<_>>().join(if arg.is_empty() { ", " } else { arg })),
            other => other.clone(),
        },
        "truncate" => {
            let n: usize = arg.parse().unwrap_or(40);
            let t = text_of(&v);
            if t.chars().count() > n {
                Value::String(t.chars().take(n.saturating_sub(1)).collect::<String>() + "…")
            } else {
                Value::String(t)
            }
        }
        "default" => {
            if matches!(v, Value::Null) || text_of(&v).is_empty() {
                Value::String(arg.to_string())
            } else {
                v
            }
        }
        _ => v,
    }
}

/// `path | filter | filter:arg`, or `a / b`.
fn eval(expr: &str, ctx: &Value) -> Value {
    let mut parts = expr.split('|');
    let mut v = arith(parts.next().unwrap_or("").trim(), ctx);
    for f in parts {
        v = filter(f, v);
    }
    v
}

/// A text with `{{expr}}` holes in it.
pub fn render(tpl: &str, ctx: &Value) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i + 2..].find("}}") else {
            out.push_str(&rest[i..]);
            return out;
        };
        out.push_str(&text_of(&eval(&rest[i + 2..i + 2 + j], ctx)));
        rest = &rest[i + 2 + j + 2..];
    }
    out.push_str(rest);
    out
}

/// A template that is only one hole, as the value it has (a number stays a number).
fn value_of(tpl: &str, ctx: &Value) -> Value {
    let t = tpl.trim();
    if t.starts_with("{{") && t.ends_with("}}") && t[2..t.len() - 2].find("{{").is_none() {
        return eval(&t[2..t.len() - 2], ctx);
    }
    Value::String(render(tpl, ctx))
}

// ----------------------------------------------------------------------------------------------------- the card

/// What the templates read: the data picked out of the answer. A list becomes `items`, with `count` and `first`.
fn context_of(root: &Value, pick: &str) -> Value {
    let data = if pick.trim().is_empty() { Some(root) } else { lookup(root, pick) };
    match data {
        Some(Value::Array(a)) => json!({ "items": a, "count": a.len(), "first": a.first().cloned().unwrap_or(Value::Null) }),
        Some(Value::Object(m)) => {
            let mut m = m.clone();
            // (a field named `items` that is a list also gets its count and first, unless the data has them)
            if let Some(Value::Array(a)) = m.get("items").cloned() {
                m.entry("count").or_insert(json!(a.len()));
                m.entry("first").or_insert(a.first().cloned().unwrap_or(Value::Null));
            }
            Value::Object(m)
        }
        Some(other) => json!({ "value": other }),
        None => Value::Null,
    }
}

fn item_ctx(item: &Value, index: usize) -> Value {
    let mut m = match item {
        Value::Object(m) => m.clone(),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("value".into(), other.clone());
            m
        }
    };
    m.insert("index".into(), json!(index + 1));
    Value::Object(m)
}

fn progress_of(v: &Value) -> Option<f64> {
    num(v).map(|n| n.clamp(0.0, 1.0))
}

fn block_of(spec: &Value, ctx: &Value) -> Option<Block> {
    let o = spec.as_object()?;
    if let Some(t) = o.get("progress").and_then(|x| x.as_str()) {
        return Some(Block::Progress { value: progress_of(&value_of(t, ctx))? });
    }
    if let Some(t) = o.get("bars").and_then(|x| x.as_str()) {
        let v = value_of(t, ctx);
        let values: Vec<f64> = v.as_array()?.iter().filter_map(num).map(|n| n.clamp(0.0, 1.0)).take(MAX_BARS).collect();
        return (!values.is_empty()).then_some(Block::Bars { values });
    }
    if let Some(t) = o.get("text").and_then(|x| x.as_str()) {
        let text = render(t, ctx);
        return (!text.trim().is_empty()).then_some(Block::Text { text });
    }
    if let Some(t) = o.get("badge").and_then(|x| x.as_str()) {
        let text = render(t, ctx);
        return (!text.trim().is_empty()).then_some(Block::Badge { text });
    }
    if let Some(rows) = o.get("facts").and_then(|x| x.as_array()) {
        let rows: Vec<(String, String)> = rows
            .iter()
            .filter_map(|r| {
                let r = r.as_array()?;
                let (k, v) = (r.first()?.as_str()?, r.get(1)?.as_str()?);
                let v = render(v, ctx);
                (!v.trim().is_empty()).then(|| (k.to_string(), v))
            })
            .take(MAX_ROWS)
            .collect();
        return (!rows.is_empty()).then_some(Block::Facts { rows });
    }
    if let Some(t) = o.get("image").and_then(|x| x.as_str()) {
        let src = render(t, ctx);
        return image_ok(&src).then_some(Block::Image { src });
    }
    if let Some(path) = o.get("list").and_then(|x| x.as_str()) {
        let items = lookup(ctx, path)?.as_array()?;
        let row = o.get("row")?.as_object()?;
        let limit = o.get("limit").and_then(|x| x.as_u64()).unwrap_or(6).min(MAX_ROWS as u64) as usize;
        let get = |k: &str| row.get(k).and_then(|x| x.as_str());
        let rows: Vec<Row> = items
            .iter()
            .take(limit)
            .enumerate()
            .map(|(i, it)| {
                let c = item_ctx(it, i);
                Row {
                    title: get("title").map(|t| render(t, &c)).unwrap_or_default(),
                    sub: get("sub").map(|t| render(t, &c)).unwrap_or_default(),
                    progress: get("progress").and_then(|t| progress_of(&value_of(t, &c))),
                }
            })
            .collect();
        return (!rows.is_empty()).then_some(Block::List { rows });
    }
    None
}

const DEFAULT_CARD_RANK: i32 = 70;
const MAX_IMAGE: usize = 200_000;

/// A picture the island will draw: a `data:` address of a raster image, not too big. (Never an address to fetch, never
/// an svg: a plugin cannot make the island ask the network, or run a script.)
fn image_ok(src: &str) -> bool {
    src.len() <= MAX_IMAGE && ["data:image/png;base64,", "data:image/jpeg;base64,", "data:image/webp;base64,", "data:image/gif;base64,"].iter().any(|p| src.starts_with(p))
}

fn cut_text(s: &mut String) {
    if s.chars().count() > MAX_TEXT {
        *s = s.chars().take(MAX_TEXT - 1).collect::<String>() + "…";
    }
}

fn unit(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub fn card_of(m: &Manifest, ctx: &Value) -> CardOut {
    CardOut {
        plugin: m.id.clone(),
        name: m.name.clone(),
        icon: if m.card.icon.is_empty() { "dot".into() } else { m.card.icon.clone() },
        rank: if m.card.rank == 0 { DEFAULT_CARD_RANK } else { m.card.rank.clamp(1, 100) },
        title: render(&m.card.title, ctx),
        sub: render(&m.card.sub, ctx),
        peek: render(&m.card.peek, ctx),
        body: m.card.body.iter().filter_map(|b| block_of(b, ctx)).collect(),
    }
}

/// What a module answered as its card, kept to what the island draws: short texts, a few rows, numbers in range.
pub fn clean_card(m: &Manifest, v: &Value) -> Option<CardOut> {
    let mut c: CardOut = serde_json::from_value(v.clone()).ok()?;
    let cut = cut_text;
    c.plugin = m.id.clone();
    c.name = m.name.clone();
    if c.icon.is_empty() {
        c.icon = "dot".into();
    }
    c.rank = if c.rank == 0 { DEFAULT_CARD_RANK } else { c.rank.clamp(1, 100) };
    for s in [&mut c.icon, &mut c.title, &mut c.sub, &mut c.peek] {
        cut(s);
    }
    c.body.truncate(8);
    for b in c.body.iter_mut() {
        match b {
            Block::Progress { value } => *value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 },
            Block::Bars { values } => {
                values.truncate(MAX_BARS);
                for v in values.iter_mut() {
                    *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                }
            }
            Block::Text { text } | Block::Badge { text } => cut(text),
            Block::Facts { rows } => {
                rows.truncate(MAX_ROWS);
                for (k, v) in rows.iter_mut() {
                    cut(k);
                    cut(v);
                }
            }
            Block::List { rows } => {
                rows.truncate(MAX_ROWS);
                for r in rows.iter_mut() {
                    cut(&mut r.title);
                    cut(&mut r.sub);
                    r.progress = r.progress.filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 1.0));
                }
            }
            Block::Buttons { items } => clean_buttons(items),
            Block::Image { src } => {
                if !image_ok(src) {
                    src.clear();
                }
            }
        }
    }
    // (a picture that is not one is left out)
    c.body.retain(|b| !matches!(b, Block::Image { src } if src.is_empty()));
    c.body.retain(|b| !matches!(b, Block::Buttons { items } if items.is_empty()));
    Some(c)
}

fn clean_buttons(items: &mut Vec<Button>) {
    items.retain(|b| !b.action.trim().is_empty() && b.action.len() <= 64);
    items.truncate(6);
    for b in items.iter_mut() {
        cut_text(&mut b.label);
        b.icon.truncate(24);
    }
}

/// What a module answered as its pill, kept to what the island draws.
pub fn clean_pill(m: &Manifest, v: &Value) -> Option<PillOut> {
    let mut p: PillOut = serde_json::from_value(v.clone()).ok()?;
    p.plugin = m.id.clone();
    p.name = m.name.clone();
    for s in [&mut p.icon, &mut p.title, &mut p.sub, &mut p.right] {
        cut_text(s);
    }
    if p.icon.is_empty() {
        p.icon = "dot".into();
    }
    // (below the games and the like, never above a peek that has to be seen)
    p.rank = p.rank.clamp(0, 99);
    p.w = if p.w > 0.0 { p.w.clamp(200.0, 360.0) } else { 260.0 };
    p.h = if p.h > 0.0 { p.h.clamp(32.0, 72.0) } else if p.sub.is_empty() { 40.0 } else { 48.0 };
    p.progress = p.progress.filter(|x| x.is_finite()).map(unit);
    p.bars.truncate(MAX_BARS);
    for b in p.bars.iter_mut() {
        *b = unit(*b);
    }
    clean_buttons(&mut p.buttons);
    p.buttons.truncate(3);
    if p.look != "big" {
        p.look.clear();
    }
    Some(p)
}

/// What a module answered as its rings (at most three, each with a name of its own).
pub fn clean_rings(v: &Value) -> Vec<RingOut> {
    let mut out: Vec<RingOut> = serde_json::from_value(v.clone()).unwrap_or_default();
    out.retain(|r| valid_id(&r.id));
    out.truncate(3);
    for r in out.iter_mut() {
        r.pct = if r.pct.is_finite() { r.pct.clamp(0.0, 100.0) } else { 0.0 };
        cut_text(&mut r.tip);
        // (a colour is `#rrggbb`: it goes into a style)
        let ok = r.color.len() == 7 && r.color.starts_with('#') && r.color[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            r.color = "#9a9aa2".into();
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------- attention

/// What the plugin watches between two answers, so it can say when something happened.
#[derive(Default)]
struct Watch {
    started: bool,
    numbers: HashMap<String, f64>,
    text: Option<String>,
}

/// The banners an answer calls for (the first answer after the plugin was switched on never does).
fn attention(spec: &AttentionSpec, ctx: &Value, watch: &mut Watch) -> Vec<String> {
    let mut out = Vec::new();
    let was = watch.started;
    watch.started = true;
    if !spec.list.is_empty() {
        let Some(items) = lookup(ctx, &spec.list).and_then(|v| v.as_array()) else { return out };
        let mut seen: HashMap<String, f64> = HashMap::new();
        for (i, it) in items.iter().enumerate() {
            let c = item_ctx(it, i);
            let id = lookup(&c, &spec.id).map(text_of).unwrap_or_else(|| i.to_string());
            let Some(now) = lookup(&c, &spec.field).and_then(num) else { continue };
            if was {
                if let (Some(goal), Some(before)) = (spec.reaches, watch.numbers.get(&id)) {
                    if *before < goal && now >= goal {
                        out.push(render(&spec.banner, &c));
                    }
                }
            }
            seen.insert(id, now);
        }
        watch.numbers = seen;
    } else {
        let now = lookup(ctx, &spec.field).map(text_of);
        if was {
            if let (Some(before), Some(now)) = (&watch.text, &now) {
                let arrived = match &spec.becomes {
                    Some(goal) => now == goal && before != goal,
                    None => now != before,
                };
                if arrived {
                    out.push(render(&spec.banner, ctx));
                }
            }
        }
        watch.text = now;
    }
    out
}

// ------------------------------------------------------------------------------------------------------ fetch

pub fn parse_every(s: &str) -> Duration {
    let s = s.trim().to_lowercase();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len()));
    let Ok(n) = n.parse::<f64>() else { return DEFAULT_EVERY };
    let secs = match unit.trim() {
        "ms" => n / 1000.0,
        "s" | "" => n,
        "m" | "min" => n * 60.0,
        "h" => n * 3600.0,
        _ => return DEFAULT_EVERY,
    };
    Duration::from_secs_f64(secs.max(0.0)).max(MIN_EVERY)
}

/// `host` or `host:port` of an address, lower case.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let end = rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let hp = &rest[..end];
    let hp = hp.rsplit_once('@').map(|x| x.1).unwrap_or(hp);
    (!hp.is_empty()).then(|| hp.to_lowercase())
}

/// The address may be asked: it is on the manifest's list (a listed `host` stands for any port of it), and it is
/// https, or plain http to this PC only.
pub fn allowed(url: &str, net: &[String]) -> Result<(), String> {
    let hp = host_of(url).ok_or("The address is not valid.")?;
    let host = hp.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|x| x.0).unwrap_or(&hp).to_string();
    let local = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
    if !(url.starts_with("https://") || (url.starts_with("http://") && local)) {
        return Err("Only https (or http to this PC) may be asked.".into());
    }
    if net.iter().any(|n| {
        let n = n.trim().to_lowercase();
        n == hp || n == host
    }) {
        Ok(())
    } else {
        Err(format!("{hp} is not on the list of hosts this plugin may ask."))
    }
}

const MAX_REDIRECTS: usize = 3;
/// the longest answer a source may be allowed (`source.max_kb`)
const MAX_ANSWER_CAP: u64 = 8_000_000;
/// a source that is not on this PC is not asked more often than this (it is somebody's server)
const REMOTE_MIN_EVERY: Duration = Duration::from_secs(30);

fn is_local(url: &str) -> bool {
    host_of(url).map_or(false, |hp| {
        let host = hp.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|x| x.0).unwrap_or(&hp).to_string();
        host == "127.0.0.1" || host == "localhost" || host == "[::1]"
    })
}

/// The address a redirect points to (it may be relative).
fn resolve(from: &str, location: &str) -> Option<String> {
    let loc = location.trim();
    if loc.contains("://") {
        return Some(loc.to_string());
    }
    let scheme_end = from.find("://")? + 3;
    let host_end = from[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(from.len());
    if let Some(path) = loc.strip_prefix('/') {
        return Some(format!("{}/{}", &from[..host_end], path));
    }
    None
}

/// Ask the address. A redirect is followed (up to three times) only to an address that is on the manifest's list too: it
/// can lead nowhere the plugin could not have gone.
fn fetch_text(m: &Manifest, url: &str, net: &[String]) -> Result<String, String> {
    let limit = if m.source.max_kb == 0 { MAX_ANSWER } else { (m.source.max_kb * 1000).min(MAX_ANSWER_CAP) };
    let timeout = if m.source.format == "ics" { Duration::from_secs(15) } else { TIME_LIMIT };
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(timeout).user_agent("NADI-plugin").build();
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        allowed(&url, net)?;
        let resp = agent.get(&url).call().map_err(|e| match e {
            ureq::Error::Status(c, _) => format!("The server said {c}."),
            ureq::Error::Transport(t) => format!("No answer ({}).", t.kind()),
        })?;
        if (300..400).contains(&resp.status()) {
            let next = resp.header("location").and_then(|l| resolve(&url, l)).ok_or("The server sent the plugin somewhere else, and not to an address it could follow.")?;
            url = next;
            continue;
        }
        let mut text = String::new();
        resp.into_reader().take(limit).read_to_string(&mut text).map_err(|_| "The answer could not be read.".to_string())?;
        return Ok(text);
    }
    Err("Too many redirects.".into())
}

/// What the address answered, as the data the templates read: JSON as it is, or a calendar as its events. A calendar's events
/// also come back as events for the island's calendar.
fn fetch(m: &Manifest, url: &str, net: &[String]) -> Result<(Value, Option<Vec<crate::calendar::CalendarEvent>>), String> {
    let text = fetch_text(m, url, net)?;
    if m.source.format == "ics" {
        let events = crate::formats::parse_ics(&text);
        let data = json!({ "events": events, "items": events, "count": events.len() });
        return Ok((data, Some(events)));
    }
    serde_json::from_str(&text).map(|v| (v, None)).map_err(|_| "The answer is not JSON.".to_string())
}

fn truthy(s: &str) -> bool {
    !matches!(s.trim().to_lowercase().as_str(), "" | "0" | "false" | "null" | "no")
}

/// The pill a declarative plugin shows now, from its `pill` templates (none while `when` does not hold).
fn pill_of(m: &Manifest, tpl: &PillTpl, ctx: &Value) -> Option<PillOut> {
    if !tpl.when.trim().is_empty() && !truthy(&render(&tpl.when, ctx)) {
        return None;
    }
    let raw = PillOut {
        icon: tpl.icon.clone(),
        title: render(&tpl.title, ctx),
        sub: render(&tpl.sub, ctx),
        right: render(&tpl.right, ctx),
        progress: if tpl.progress.trim().is_empty() { None } else { progress_of(&value_of(&tpl.progress, ctx)) },
        rank: tpl.rank,
        w: tpl.w,
        h: tpl.h,
        ..Default::default()
    };
    clean_pill(m, &serde_json::to_value(raw).ok()?)
}

fn rings_of(tpls: &[RingTpl], ctx: &Value) -> Vec<RingOut> {
    let list: Vec<RingOut> = tpls
        .iter()
        .filter_map(|t| {
            let pct = num(&value_of(&t.pct, ctx))?;
            Some(RingOut { id: t.id.clone(), pct, tip: render(&t.tip, ctx), color: t.color.clone() })
        })
        .collect();
    clean_rings(&serde_json::to_value(list).unwrap_or(Value::Null))
}

// ------------------------------------------------------------------------------------------------------- host

pub struct Plugin {
    pub manifest: Manifest,
    /// the folder it was read from
    pub dir: PathBuf,
    /// why it cannot run (a manifest that is wrong, a module that is missing)
    pub problem: Option<String>,
    /// it ships inside the app (see bundled.rs): the same kind of plugin as one in a folder, only not in one
    pub bundled: Option<&'static crate::bundled::Bundled>,
    card: Mutex<Option<CardOut>>,
    pill: Mutex<Option<PillOut>>,
    rings: Mutex<Vec<RingOut>>,
    error: Mutex<Option<String>>,
    /// a line about how it is doing (a declarative plugin's `status`)
    note: Mutex<Option<String>>,
    due: Mutex<Instant>,
    busy: AtomicBool,
    watch: Mutex<Watch>,
    /// what the module cost: calls made, microseconds spent in them
    pub calls: AtomicU64,
    pub spent_us: AtomicU64,
    /// the last time a live card was sent to the screen
    pub sent: Mutex<Instant>,
    /// what the screen has not been told yet (a pill or rings that changed), and when it was last told
    pub dirty: AtomicBool,
    pub pushed: Mutex<Instant>,
    /// buttons the user pressed, for a module to be told (see `Host::act`)
    pub actions: Mutex<std::collections::VecDeque<(String, Value)>>,
}

/// What is kept between runs: what is on, the settings of each plugin, who is muted, do not disturb.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub(crate) struct Saved {
    pub(crate) on: Vec<String>,
    pub(crate) muted: Vec<String>,
    pub(crate) dnd: bool,
    pub(crate) values: HashMap<String, HashMap<String, Value>>,
    /// the examples that were put in the plugins folder once (one that was deleted is not put back)
    pub(crate) seeded: Vec<String>,
    /// the native plugins that took over what the old settings said about them (once each)
    pub(crate) adopted: Vec<String>,
}

impl Plugin {
    pub(crate) fn new(manifest: Manifest, dir: PathBuf, problem: Option<String>, bundled: Option<&'static crate::bundled::Bundled>) -> Plugin {
        Plugin {
            manifest,
            dir,
            problem,
            bundled,
            card: Mutex::new(None),
            pill: Mutex::new(None),
            rings: Mutex::new(Vec::new()),
            error: Mutex::new(None),
            note: Mutex::new(None),
            due: Mutex::new(Instant::now()),
            busy: AtomicBool::new(false),
            watch: Mutex::new(Watch::default()),
            calls: AtomicU64::new(0),
            spent_us: AtomicU64::new(0),
            sent: Mutex::new(Instant::now()),
            dirty: AtomicBool::new(false),
            pushed: Mutex::new(Instant::now()),
            actions: Mutex::new(Default::default()),
        }
    }

    /// A plugin that is part of the app, written in Rust (see native.rs): no folder, nothing to check.
    pub(crate) fn built_in(manifest: Manifest) -> Plugin {
        Plugin::new(manifest, PathBuf::new(), None, None)
    }

    pub fn card_slot(&self) -> std::sync::MutexGuard<'_, Option<CardOut>> {
        self.card.lock().unwrap()
    }

    pub fn set_note(&self, n: Option<String>) {
        *self.note.lock().unwrap() = n;
    }

    pub fn set_error(&self, e: Option<String>) {
        *self.error.lock().unwrap() = e;
    }
}

#[derive(Default)]
pub struct Host {
    pub(crate) plugins: Mutex<Vec<Arc<Plugin>>>,
    pub(crate) saved: Mutex<Saved>,
    loaded: AtomicBool,
    /// the loopback audio, for the plugins that may hear it (see pluginwasm.rs)
    pub audio: crate::pluginwasm::AudioBus,
    /// the plugins that are part of the app (see native.rs)
    pub(crate) natives: Mutex<Vec<crate::native::Entry>>,
    /// the pill each plugin offers right now (see `native::Ctx::offer_pill`)
    pub(crate) offers: Mutex<HashMap<String, Offer>>,
    /// plugins that ask the island to stay out of the way (a game is on the screen)
    pub(crate) quiet: Mutex<std::collections::HashSet<String>>,
    /// the plugins that brought events to the calendar (so they can be taken back when a plugin is switched off)
    cal_sources: Mutex<std::collections::HashSet<String>>,
}

/// A pill a plugin asks for: the island shows the highest one while nothing outranks it (a banner, the hub).
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub id: String,
    pub w: f64,
    pub h: f64,
    pub rank: i32,
    pub brief: bool,
}

#[derive(Serialize)]
pub struct Perm {
    pub name: String,
    /// what it is of: the host, the file, the command
    pub detail: String,
    pub why: String,
}

fn perm(name: &str, detail: impl Into<String>) -> Perm {
    let detail = detail.into();
    Perm { why: crate::permissions::grants(name, &detail).unwrap_or_default(), name: name.into(), detail }
}

/// What a plugin asks for, as the permissions the app knows (and what each grants, in the app's words).
pub fn permissions_of(m: &Manifest) -> Vec<Perm> {
    let mut out: Vec<Perm> = Vec::new();
    for h in &m.permissions.net {
        // (a host that is a setting: it is the address the user gave, under that setting's name)
        let setting = h.strip_prefix("{{settings.").and_then(|r| r.strip_suffix("}}")).map(str::trim).and_then(|k| m.settings.get(k));
        match setting {
            Some(s) => out.push(perm("network", format!("the address you give it (\"{}\")", s.label))),
            None => out.push(perm("network", h.clone())),
        }
    }
    for (on, name) in [
        (m.permissions.calendar, "calendar"),
        (m.permissions.media, "media"),
        (m.permissions.media_control, "media control"),
        (m.permissions.system, "system"),
        (m.permissions.audio, "audio"),
        (m.permissions.processes, "processes"),
        (m.permissions.titles, "titles"),
    ] {
        if on {
            out.push(perm(name, ""));
        }
    }
    for f in &m.permissions.files {
        out.push(perm("read files and folders", f.clone()));
    }
    for x in &m.permissions.sees {
        let mut p = perm(&x.name, x.detail.clone());
        if p.why.is_empty() {
            p.why = x.why.clone();
        }
        out.push(p);
    }
    out
}

#[derive(Serialize)]
pub struct SettingInfo {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: Value,
    pub options: Vec<Value>,
}

#[derive(Serialize)]
pub struct ActionSpec2 {
    pub id: String,
    pub label: String,
    pub ask: String,
    pub then: String,
    /// its field has something kept (see `Native::filled`)
    pub filled: bool,
}

#[derive(Serialize)]
pub struct Info {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub kind: String,
    pub enabled: bool,
    pub muted: bool,
    pub permissions: Vec<Perm>,
    pub settings: Vec<SettingInfo>,
    pub actions: Vec<ActionSpec2>,
    /// it can say things in banners: the mute button has something to do
    pub banners: bool,
    /// it ships inside the app (written in Rust, or a plugin like any other that the app carries)
    pub bundled: bool,
    /// what the module costs when it runs ("0.4 ms a call")
    pub cost: Option<String>,
    pub error: Option<String>,
    /// a native plugin's line about how it is doing ("waiting for the browser extension")
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct Listing {
    pub plugins: Vec<Info>,
    pub dnd: bool,
}

fn dir() -> Option<PathBuf> {
    Some(crate::settings::data_dir()?.join("plugins"))
}

fn state_path() -> Option<PathBuf> {
    Some(crate::settings::data_dir()?.join("plugins.json"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 48 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}

/// The settings a plugin has: what its manifest gives, over which what was saved.
pub(crate) fn values_of(m: &Manifest, saved: &Saved) -> Value {
    let mut out = serde_json::Map::new();
    for (k, spec) in &m.settings {
        out.insert(k.clone(), spec.default.clone());
    }
    if let Some(v) = saved.values.get(&m.id) {
        for (k, val) in v {
            if m.settings.contains_key(k) {
                out.insert(k.clone(), val.clone());
            }
        }
    }
    Value::Object(out)
}

fn with_settings(mut ctx: Value, values: &Value) -> Value {
    match &mut ctx {
        Value::Object(m) => {
            m.insert("settings".into(), values.clone());
            ctx
        }
        _ => json!({ "settings": values }),
    }
}

fn check(m: &Manifest, dir: &std::path::Path, bundled: bool) -> Option<String> {
    if !valid_id(&m.id) {
        return Some("The id must be lower case letters, digits, '.', '-' or '_'.".into());
    }
    if m.name.trim().is_empty() {
        return Some("The manifest has no name.".into());
    }
    let sees_the_pc = m.permissions.audio
        || m.permissions.processes
        || m.permissions.titles
        || !m.permissions.files.is_empty()
        || m.permissions.calendar
        || m.permissions.media
        || m.permissions.media_control
        || m.permissions.system;
    match m.kind.as_str() {
        "wasm" => {
            let f = m.module.trim();
            if f.is_empty() || f.contains(['/', '\\']) || !f.ends_with(".wasm") {
                return Some("A module plugin names its file (\"module\": \"name.wasm\"), in its own folder.".into());
            }
            if !bundled && !dir.join(f).is_file() {
                return Some(format!("The module {f} is not in the folder."));
            }
            if !m.permissions.net.is_empty() {
                // (no module can reach the network: what it sees stays on this PC)
                return Some("A module cannot ask the network. Use a declarative plugin for that.".into());
            }
            if !m.source.http.is_empty() {
                return Some("A module has no source address.".into());
            }
            if m.permissions.media_control && !m.permissions.media {
                return Some("To control what is playing, a module must also be allowed to see it (\"media\").".into());
            }
            None
        }
        "declarative" | "" => {
            if sees_the_pc {
                return Some("Only a module can listen to the PC. A declarative plugin asks the network.".into());
            }
            if m.source.http.trim().is_empty() {
                return Some("The manifest names no source.".into());
            }
            if !matches!(m.source.format.as_str(), "" | "json" | "ics") {
                return Some(format!("Unknown source format \"{}\".", m.source.format));
            }
            for k in &m.needs {
                if !m.settings.contains_key(k) {
                    return Some(format!("\"needs\" names the setting {k}, which the manifest does not have."));
                }
            }
            // (a source can hold settings: it is looked at again, filled in, each time it is asked; one that is not
            // filled in yet is waited for, see `needs`)
            let defaults = values_of(m, &Saved::default());
            let probe = render(&m.source.http, &json!({ "settings": defaults }));
            if !m.needs.is_empty() && !probe.contains("://") {
                // (the address is a setting that is not filled in yet)
                return None;
            }
            allowed(&probe, &net_hosts(m, &defaults)).err()
        }
        other => Some(format!("Unknown kind \"{other}\".")),
    }
}

/// The hosts a plugin may ask: what its manifest lists. An entry may be a template over the settings (`{{settings.url}}`
/// is the address the user pastes: its host is the one allowed); one that comes out empty allows nothing.
pub fn net_hosts(m: &Manifest, values: &Value) -> Vec<String> {
    let ctx = json!({ "settings": values });
    m.permissions
        .net
        .iter()
        .filter_map(|n| {
            if !n.contains("{{") {
                return Some(n.clone());
            }
            let t = render(n, &ctx);
            let t = t.trim();
            if t.is_empty() {
                None
            } else if t.contains("://") {
                host_of(t)
            } else {
                Some(t.to_lowercase())
            }
        })
        .collect()
}

fn read_one(path: &std::path::Path) -> Option<Plugin> {
    let text = std::fs::read_to_string(path).ok()?;
    let folder = path.parent()?;
    let fallback = folder.file_name()?.to_string_lossy().to_string();
    let (manifest, problem) = match serde_json::from_str::<Manifest>(&text) {
        Ok(mut m) => {
            if m.id.is_empty() {
                m.id = fallback;
            }
            let p = check(&m, folder, false);
            (m, p)
        }
        Err(e) => (Manifest { id: fallback.clone(), name: fallback, ..Default::default() }, Some(format!("The manifest is not valid: {e}."))),
    };
    Some(Plugin::new(manifest, folder.to_path_buf(), problem, None))
}

/// A plugin that ships inside the app: read from what is embedded, checked like any other.
fn read_bundled(b: &'static crate::bundled::Bundled) -> Plugin {
    match serde_json::from_str::<Manifest>(b.manifest) {
        Ok(mut m) => {
            m.id = b.id.to_string();
            let p = check(&m, std::path::Path::new(""), true);
            Plugin::new(m, PathBuf::new(), p, Some(b))
        }
        Err(e) => Plugin::new(Manifest { id: b.id.to_string(), name: b.id.to_string(), ..Default::default() }, PathBuf::new(), Some(format!("The manifest is not valid: {e}.")), Some(b)),
    }
}

/// What a new install starts with, to show what a plugin looks like. Each is written once (a folder you delete is not
/// put back).
struct Example {
    id: &'static str,
    manifest: &'static str,
    /// the module's file name and bytes, for a module plugin
    module: Option<(&'static str, &'static [u8])>,
}

const EXAMPLES: &[Example] = &[
    Example {
        id: "example-weather",
        manifest: include_str!("../plugins-bundled/example-weather/manifest.json"),
        module: None,
    },
    Example {
        id: "example-audio-meter",
        manifest: include_str!("../plugins-bundled/example-audio-meter/manifest.json"),
        module: Some(("audio-meter.wasm", include_bytes!("../plugins-bundled/example-audio-meter/audio-meter.wasm"))),
    },
    Example {
        id: "example-game-clock",
        manifest: include_str!("../plugins-bundled/example-game-clock/manifest.json"),
        module: Some(("game-clock.wasm", include_bytes!("../plugins-bundled/example-game-clock/game-clock.wasm"))),
    },
];

impl Host {
    /// Read the plugins folder (putting the examples in the first time) and what is switched on.
    pub fn rescan(&self) {
        let Some(root) = dir() else { return };
        if !self.loaded.swap(true, Ordering::Relaxed) {
            if let Some(text) = state_path().and_then(|p| std::fs::read_to_string(p).ok()) {
                // (the first version kept only the list of what is on)
                if let Ok(s) = serde_json::from_str::<Saved>(&text) {
                    *self.saved.lock().unwrap() = s;
                } else if let Ok(list) = serde_json::from_str::<Vec<String>>(&text) {
                    self.saved.lock().unwrap().on = list;
                }
            }
        }
        let mut changed = false;
        {
            let mut saved = self.saved.lock().unwrap();
            for ex in EXAMPLES {
                if saved.seeded.iter().any(|s| s == ex.id) {
                    continue;
                }
                let d = root.join(ex.id);
                if !d.exists() && std::fs::create_dir_all(&d).is_ok() {
                    let _ = std::fs::write(d.join("manifest.json"), ex.manifest);
                    if let Some((name, bytes)) = ex.module {
                        let _ = std::fs::write(d.join(name), bytes);
                    }
                }
                saved.seeded.push(ex.id.to_string());
                changed = true;
            }
        }
        if changed {
            self.save();
        }
        // (the plugins that are part of the app come first: the ones written in Rust, then the ones inside the exe)
        let mut all: Vec<Arc<Plugin>> = self.natives.lock().unwrap().iter().map(|e| e.plugin.clone()).collect();
        for b in crate::bundled::ALL {
            all.push(Arc::new(read_bundled(b)));
        }
        let mut found: Vec<Arc<Plugin>> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten() {
                if let Some(p) = read_one(&e.path().join("manifest.json")) {
                    // (one of the app's own ids is the app's: a folder cannot stand in for it)
                    if !found.iter().any(|x| x.manifest.id == p.manifest.id) && !all.iter().any(|x| x.manifest.id == p.manifest.id) {
                        found.push(Arc::new(p));
                    }
                }
            }
        }
        found.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
        all.extend(found);
        // (a plugin that is still there keeps what it has drawn)
        {
            let old = self.plugins.lock().unwrap();
            for p in all.iter().filter(|p| p.manifest.kind != "native") {
                if let Some(o) = old.iter().find(|o| o.manifest.id == p.manifest.id) {
                    if o.manifest.source.http == p.manifest.source.http && o.manifest.module == p.manifest.module {
                        *p.card.lock().unwrap() = o.card.lock().unwrap().clone();
                        *p.pill.lock().unwrap() = o.pill.lock().unwrap().clone();
                        *p.rings.lock().unwrap() = o.rings.lock().unwrap().clone();
                        *p.note.lock().unwrap() = o.note.lock().unwrap().clone();
                    }
                }
            }
        }
        *self.plugins.lock().unwrap() = all;
        // (what a plugin offered that is gone is gone with it)
        let ids: Vec<String> = self.plugins.lock().unwrap().iter().map(|p| p.manifest.id.clone()).collect();
        self.offers.lock().unwrap().retain(|id, _| ids.contains(id));
    }

    pub(crate) fn save(&self) {
        let Some(p) = state_path() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(&*self.saved.lock().unwrap()) {
            let _ = std::fs::write(p, t);
        }
    }

    pub(crate) fn reset(p: &Plugin) {
        *p.card.lock().unwrap() = None;
        *p.pill.lock().unwrap() = None;
        p.rings.lock().unwrap().clear();
        *p.note.lock().unwrap() = None;
        p.actions.lock().unwrap().clear();
        p.dirty.store(true, Ordering::Relaxed);
        *p.error.lock().unwrap() = None;
        *p.watch.lock().unwrap() = Watch::default();
        *p.due.lock().unwrap() = Instant::now();
        p.calls.store(0, Ordering::Relaxed);
        p.spent_us.store(0, Ordering::Relaxed);
    }

    pub fn set(&self, id: &str, on: bool) {
        {
            let mut s = self.saved.lock().unwrap();
            s.on.retain(|x| x != id);
            if on {
                s.on.push(id.to_string());
            }
        }
        // (on again later it starts over: no stale card, no banner for what happened meanwhile)
        for p in self.plugins.lock().unwrap().iter().filter(|p| p.manifest.id == id) {
            Self::reset(p);
        }
        self.save();
        if !on {
            self.offers.lock().unwrap().remove(id);
            self.quiet.lock().unwrap().remove(id);
        }
        let entry = self.natives.lock().unwrap().iter().find(|e| e.plugin.manifest.id == id).map(|e| (e.native.clone(), e.ctx.clone()));
        if let Some((n, ctx)) = entry {
            n.switched(&ctx, on);
        }
    }

    pub fn set_value(&self, id: &str, key: &str, value: Value) {
        let known = self.plugins.lock().unwrap().iter().any(|p| p.manifest.id == id && p.manifest.settings.contains_key(key));
        if !known {
            return;
        }
        self.saved.lock().unwrap().values.entry(id.to_string()).or_default().insert(key.to_string(), value);
        // (the source may hold it: ask again soon)
        for p in self.plugins.lock().unwrap().iter().filter(|p| p.manifest.id == id) {
            *p.due.lock().unwrap() = Instant::now();
        }
        self.save();
        let entry = self.natives.lock().unwrap().iter().find(|e| e.plugin.manifest.id == id).map(|e| (e.native.clone(), e.ctx.clone()));
        if let Some((n, ctx)) = entry {
            n.changed(&ctx, key);
        }
    }

    pub fn set_muted(&self, id: &str, muted: bool) {
        {
            let mut s = self.saved.lock().unwrap();
            s.muted.retain(|x| x != id);
            if muted {
                s.muted.push(id.to_string());
            }
        }
        self.save();
    }

    pub fn set_dnd(&self, on: bool) {
        self.saved.lock().unwrap().dnd = on;
        self.save();
    }

    pub(crate) fn is_on(&self, p: &Plugin) -> bool {
        p.problem.is_none() && self.saved.lock().unwrap().on.contains(&p.manifest.id)
    }

    /// The settings of a plugin, as its templates and its module read them.
    pub fn values(&self, p: &Plugin) -> Value {
        values_of(&p.manifest, &self.saved.lock().unwrap())
    }

    /// Some module that is on listens to the audio (so the capture has to run).
    /// Some module that is on may see what is playing (so the media sensor keeps a copy of it).
    pub fn needs_media(&self) -> bool {
        self.plugins.lock().unwrap().iter().any(|p| self.is_on(p) && p.manifest.kind == "wasm" && p.manifest.permissions.media)
    }

    pub fn needs_audio(&self) -> bool {
        self.plugins.lock().unwrap().iter().any(|p| self.is_on(p) && p.manifest.kind == "wasm" && p.manifest.permissions.audio)
    }

    pub fn wasm_plugins(&self) -> Vec<Arc<Plugin>> {
        self.plugins.lock().unwrap().iter().filter(|p| self.is_on(p) && p.manifest.kind == "wasm").cloned().collect()
    }

    pub fn listing(&self) -> Listing {
        let saved = self.saved.lock().unwrap().clone();
        // (a copy of the list: the notes ask the host about the plugins, which must not find the list locked)
        let all = self.plugins.lock().unwrap().clone();
        let plugins = all
            .iter()
            .map(|p| {
                let m = &p.manifest;
                let vals = values_of(m, &saved);
                let permissions = permissions_of(m);
                let calls = p.calls.load(Ordering::Relaxed);
                let cost = (calls > 0).then(|| format!("{:.2} ms a call", p.spent_us.load(Ordering::Relaxed) as f64 / calls as f64 / 1000.0));
                Info {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                    version: m.version.clone(),
                    kind: if m.kind.is_empty() { "declarative".into() } else { m.kind.clone() },
                    enabled: saved.on.contains(&m.id),
                    muted: saved.muted.contains(&m.id),
                    permissions,
                    settings: m
                        .settings
                        .iter()
                        .map(|(k, s)| SettingInfo {
                            key: k.clone(),
                            label: if s.label.is_empty() { k.clone() } else { s.label.clone() },
                            kind: if s.kind.is_empty() { "text".into() } else { s.kind.clone() },
                            value: vals.get(k).cloned().unwrap_or(Value::Null),
                            options: s.options.iter().map(|o| json!({ "value": o.value, "label": o.label })).collect(),
                        })
                        .collect(),
                    actions: m.actions.iter().map(|a| ActionSpec2 { id: a.id.clone(), label: a.label.clone(), ask: a.ask.clone(), then: a.then.clone(), filled: !a.ask.is_empty() && self.filled_of(&m.id, &a.id) }).collect(),
                    banners: m.banners || m.kind == "wasm" || m.attention.is_some(),
                    bundled: p.bundled.is_some() || m.kind == "native",
                    cost,
                    error: p.problem.clone().or_else(|| p.error.lock().unwrap().clone()),
                    note: if saved.on.contains(&m.id) { self.note_of(&m.id).or_else(|| p.note.lock().unwrap().clone()) } else { None },
                }
            })
            .collect();
        Listing { plugins, dnd: saved.dnd }
    }

    /// The cards of the plugins that are on, in the list's order.
    pub fn cards(&self) -> Vec<CardOut> {
        self.plugins.lock().unwrap().iter().filter(|p| self.is_on(p)).filter_map(|p| p.card.lock().unwrap().clone()).collect()
    }

    fn due(&self, seen: bool) -> Vec<Arc<Plugin>> {
        let now = Instant::now();
        self.plugins
            .lock()
            .unwrap()
            .iter()
            .filter(|p| self.is_on(p) && p.manifest.kind != "wasm" && p.manifest.kind != "native")
            .filter(|p| seen || p.manifest.source.when == "always")
            .filter(|p| *p.due.lock().unwrap() <= now && !p.busy.load(Ordering::Relaxed))
            .cloned()
            .collect()
    }

    /// Say something on the island for a plugin: it waits by its priority; a muted plugin says nothing; with do not
    /// disturb only what is 90 or more rings (the rest is in the scrollback). `force`: the user asked for it (they pressed
    /// the plugin's button), so neither of those applies.
    pub fn say(&self, app: &AppHandle, state: &IslandState, p: &Plugin, text: String, priority: i32, force: bool) {
        let (muted, dnd) = {
            let s = self.saved.lock().unwrap();
            (s.muted.contains(&p.manifest.id), s.dnd)
        };
        if muted && !force {
            return;
        }
        let history = {
            let mut notif = state.notif.lock().unwrap();
            notif.push_priority(p.manifest.name.clone(), text, priority.clamp(0, 100), dnd && priority < 90 && !force);
            notif.history()
        };
        let _ = app.emit("notification-history-tick", history);
    }

    /// What one answer of a plugin does to the island (see `Out`). `by_user`: it answers a button the user pressed, so what
    /// it says is shown whatever mute, do not disturb and a busy screen say.
    pub fn apply(&self, app: &AppHandle, state: &IslandState, p: &Arc<Plugin>, out: Out, by_user: bool) {
        let id = p.manifest.id.clone();
        if let Some(card) = out.card {
            let changed = *p.card.lock().unwrap() != card;
            *p.card.lock().unwrap() = card.clone();
            // a live card goes to the screen as it changes, not when the screen next asks
            if let (true, true, Some(card)) = (p.manifest.live, changed, card) {
                if p.sent.lock().unwrap().elapsed() >= LIVE_GAP {
                    *p.sent.lock().unwrap() = Instant::now();
                    let _ = app.emit("plugin-frame", &card);
                }
            }
        }
        if let Some(pill) = out.pill {
            if *p.pill.lock().unwrap() != pill {
                *p.pill.lock().unwrap() = pill.clone();
                p.dirty.store(true, Ordering::Relaxed);
            }
            match pill {
                Some(pl) => {
                    // (the page hears what the pill says before the island moves to it)
                    self.flush(app);
                    let o = Offer { id: pill_node(&id), w: pl.w, h: pl.h, rank: pl.rank, brief: false };
                    self.offers.lock().unwrap().insert(id.clone(), o);
                }
                None => {
                    self.offers.lock().unwrap().remove(&id);
                }
            }
        }
        if let Some(rings) = out.rings {
            if *p.rings.lock().unwrap() != rings {
                *p.rings.lock().unwrap() = rings;
                p.dirty.store(true, Ordering::Relaxed);
            }
        }
        // the calendar is the island's; a plugin may bring events to it (`null` takes them away again)
        match out.events {
            Some(Some(events)) => {
                crate::calendar::set_source(app, state, &format!("plugin:{id}"), Ok(events));
                self.track_calendar(&id);
            }
            Some(None) => {
                crate::calendar::drop_source(app, state, &format!("plugin:{id}"));
                self.cal_sources.lock().unwrap().remove(&id);
            }
            None => {}
        }
        if let Some(note) = out.note {
            p.set_note(note);
        }
        if let Some(q) = out.quiet {
            let mut quiet = self.quiet.lock().unwrap();
            if q {
                quiet.insert(id.clone());
            } else {
                quiet.remove(&id);
            }
        }
        if let Some((action, source)) = out.media {
            if p.manifest.permissions.media_control {
                let source = Some(source).filter(|s| !s.is_empty());
                match action.as_str() {
                    "play_pause" => crate::media::media_play_pause(source),
                    "next" => crate::media::media_next(source),
                    "previous" => crate::media::media_previous(source),
                    _ => {}
                }
            }
        }
        for s in out.say {
            // (ambient: only when nothing fills the screen: not over the hub, not while a game is on)
            if s.ambient && !by_user && (state.hub_is_open() || self.quiet()) {
                continue;
            }
            match s.pill {
                None => self.say(app, state, p, s.text, s.priority, by_user),
                Some(pill) => {
                    if !by_user && self.gate(&id, s.priority) != crate::native::Gate::Open {
                        continue;
                    }
                    let brief = crate::notify::Brief {
                        id: format!("plugin:{id}"),
                        state: "plugin",
                        host_icon: pill.icon,
                        project: pill.right,
                        look: if pill.look == "big" { "big" } else { "" },
                        dwell_ms: pill.ms,
                        ..Default::default()
                    };
                    state.notif.lock().unwrap().push_brief_priority(s.text, brief, s.priority.clamp(0, 100));
                }
            }
        }
    }

    pub(crate) fn track_calendar(&self, id: &str) {
        self.cal_sources.lock().unwrap().insert(id.to_string());
    }

    /// What a plugin that is switched off (or gone) left behind is taken away: its events in the calendar.
    pub fn reconcile(&self, app: &AppHandle, state: &IslandState) {
        let stale: Vec<String> = {
            let tracked = self.cal_sources.lock().unwrap();
            tracked.iter().filter(|id| !self.is_on_id(id)).cloned().collect()
        };
        for id in stale {
            crate::calendar::drop_source(app, state, &format!("plugin:{id}"));
            self.cal_sources.lock().unwrap().remove(&id);
        }
    }

    /// Tell the screen what changed in the pills and the rings (at most about 20 times a second for one plugin).
    pub fn flush(&self, app: &AppHandle) {
        let all = self.plugins.lock().unwrap().clone();
        for p in all.iter().filter(|p| p.dirty.load(Ordering::Relaxed)) {
            if p.pushed.lock().unwrap().elapsed() < LIVE_GAP {
                continue;
            }
            *p.pushed.lock().unwrap() = Instant::now();
            p.dirty.store(false, Ordering::Relaxed);
            let on = self.is_on(p);
            let pill = if on { p.pill.lock().unwrap().clone() } else { None };
            let rings = if on { p.rings.lock().unwrap().clone() } else { Vec::new() };
            let _ = app.emit("plugin-pill", json!({ "plugin": p.manifest.id, "pill": pill }));
            let _ = app.emit("plugin-rings", json!({ "plugin": p.manifest.id, "rings": rings }));
        }
    }

    /// The pills plugins offer now, and the rings they show (for a page that has just opened).
    pub fn pills_and_rings(&self) -> (Vec<PillOut>, serde_json::Map<String, Value>) {
        let all = self.plugins.lock().unwrap().clone();
        let mut pills = Vec::new();
        let mut rings = serde_json::Map::new();
        for p in all.iter().filter(|p| self.is_on(p)) {
            if let Some(pl) = p.pill.lock().unwrap().clone() {
                pills.push(pl);
            }
            let r = p.rings.lock().unwrap().clone();
            if !r.is_empty() {
                rings.insert(p.manifest.id.clone(), serde_json::to_value(r).unwrap_or(Value::Null));
            }
        }
        (pills, rings)
    }

    /// A button the user pressed on a plugin's card or pill: a module is told by an `action` event; a native plugin gets
    /// the call. (An action a plugin does not know is ignored by it.)
    pub fn act(&self, id: &str, action: &str, args: Value) -> Result<Value, String> {
        let p = self.plugin(id).ok_or_else(|| format!("no plugin {id}"))?;
        if !self.is_on(&p) {
            return Err(format!("{id} is off"));
        }
        if p.manifest.kind == "wasm" {
            let mut q = p.actions.lock().unwrap();
            if q.len() < 16 {
                q.push_back((action.chars().take(64).collect(), args));
            }
            return Ok(Value::Null);
        }
        if p.manifest.kind == "native" {
            return self.call(id, action, &args);
        }
        Err(format!("{id} takes no actions"))
    }
}

// ------------------------------------------------------------------------------------------ what an answer does

/// A banner a plugin says, and the pill it may come with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Say {
    pub text: String,
    pub priority: i32,
    /// shown as a pill (the text is its title) instead of a banner
    pub pill: Option<SayPill>,
    /// only when nothing fills the screen: not over the hub, not while a game is on
    pub ambient: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SayPill {
    pub icon: String,
    /// a short text at its right
    pub right: String,
    /// "" or "big" (large light digits, as the time is shown)
    pub look: String,
    /// how long it stays, ms (0 for the usual)
    pub ms: u64,
}

/// What one answer of a plugin asks of the island, from a module's reply or from a declarative plugin's data run through
/// its templates. Everything is optional: what an answer does not mention stays as it was.
#[derive(Default)]
pub struct Out {
    pub card: Option<Option<CardOut>>,
    pub pill: Option<Option<PillOut>>,
    pub rings: Option<Vec<RingOut>>,
    pub events: Option<Option<Vec<crate::calendar::CalendarEvent>>>,
    pub say: Vec<Say>,
    pub quiet: Option<bool>,
    /// a line about how the plugin is doing, shown under its name in Settings > Plugins (`Some(None)` takes it away)
    pub note: Option<Option<String>>,
    /// a module wants to be called again after this many ms (it keeps time by it)
    pub wake_ms: Option<u64>,
    /// a button of the player, for a module that may control it: (what, which player)
    pub media: Option<(String, String)>,
}

/// the name the page gives a plugin's pill (`#pg-<id>`)
pub fn pill_node(id: &str) -> String {
    format!("pg-{id}")
}

const MAX_WAKE_MS: u64 = 86_400_000;

/// A module's reply, kept to what the island does: short texts, few rows, numbers in range.
pub fn out_of(m: &Manifest, reply: &Value) -> Out {
    let mut out = Out::default();
    let Some(o) = reply.as_object() else { return out };
    if let Some(c) = o.get("card") {
        out.card = if c.is_null() { Some(None) } else { clean_card(m, c).map(Some) };
    }
    if let Some(p) = o.get("pill") {
        out.pill = if p.is_null() { Some(None) } else { clean_pill(m, p).map(Some) };
    }
    if let Some(r) = o.get("rings") {
        out.rings = Some(if r.is_null() { Vec::new() } else { clean_rings(r) });
    }
    if let Some(e) = o.get("events") {
        out.events = Some(if e.is_null() { None } else { Some(crate::calendar::events_from_json(e)) });
    }
    if let Some(say) = o.get("say").and_then(|s| s.as_array()) {
        for s in say.iter().take(3) {
            let text = s.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            if text.is_empty() || text.chars().count() > MAX_TEXT {
                continue;
            }
            let get = |k: &str| s.get("pill").and_then(|p| p.get(k)).and_then(|v| v.as_str()).unwrap_or("").chars().take(MAX_TEXT).collect::<String>();
            let pill = s.get("pill").filter(|p| p.is_object()).map(|p| SayPill {
                icon: get("icon").chars().take(24).collect(),
                right: get("right"),
                look: get("look"),
                ms: p.get("ms").and_then(|v| v.as_u64()).unwrap_or(0),
            });
            out.say.push(Say {
                text,
                priority: s.get("priority").and_then(|x| x.as_i64()).unwrap_or(0).clamp(0, 100) as i32,
                pill,
                ambient: s.get("ambient").and_then(|b| b.as_bool()).unwrap_or(false),
            });
        }
    }
    out.quiet = o.get("quiet").and_then(|b| b.as_bool());
    if let Some(n) = o.get("note") {
        out.note = Some(n.as_str().map(|t| t.trim().chars().take(MAX_TEXT).collect::<String>()).filter(|t| !t.is_empty()));
    }
    out.wake_ms = o.get("wake_ms").and_then(|v| v.as_u64()).map(|n| n.clamp(50, MAX_WAKE_MS));
    if let Some(mc) = o.get("media").and_then(|v| v.as_object()) {
        let s = |k: &str| mc.get(k).and_then(|v| v.as_str()).unwrap_or("").chars().take(200).collect::<String>();
        out.media = Some((s("action"), s("source")));
    }
    out
}

/// The settings the manifest says must be filled in, and are not.
fn missing(m: &Manifest, values: &Value) -> bool {
    m.needs.iter().any(|k| values.get(k).map_or(true, |v| text_of(v).trim().is_empty()))
}

fn has_card(m: &Manifest) -> bool {
    !(m.card.title.trim().is_empty() && m.card.sub.trim().is_empty() && m.card.body.is_empty())
}

fn poll(p: Arc<Plugin>, app: AppHandle, state: Arc<IslandState>) {
    let m = &p.manifest;
    let values = state.plugins.values(&p);
    let sctx = json!({ "settings": values });
    let mut every = parse_every(&render(&m.source.every, &sctx));
    let id = p.manifest.id.clone();
    if missing(m, &values) {
        // (not set up yet: it asks nothing, and says what is left to do)
        *p.error.lock().unwrap() = None;
        p.set_note(Some(if m.setup.is_empty() { "Fill in its settings below.".to_string() } else { m.setup.clone() }));
        // (a calendar that is switched on and has no link yet is an empty calendar, not a missing one)
        state.plugins.apply(&app, &state, &p, Out { card: Some(None), pill: Some(None), rings: Some(Vec::new()), events: m.source.format.eq("ics").then_some(Some(Vec::new())), ..Default::default() }, false);
        *p.due.lock().unwrap() = Instant::now() + Duration::from_secs(5);
        p.busy.store(false, Ordering::Relaxed);
        return;
    }
    let url = render(&m.source.http, &sctx);
    if !is_local(&url) {
        every = every.max(REMOTE_MIN_EVERY);
    }
    match fetch(m, &url, &net_hosts(m, &values)) {
        Ok((root, events)) => {
            let ctx = with_settings(context_of(&root, &m.source.pick), &values);
            let mut out = Out::default();
            if has_card(m) {
                out.card = Some(Some(card_of(m, &ctx)));
            }
            if let Some(tpl) = &m.pill {
                out.pill = Some(pill_of(m, tpl, &ctx));
            }
            if !m.rings.is_empty() {
                out.rings = Some(rings_of(&m.rings, &ctx));
            }
            if let Some(events) = events {
                out.events = Some(Some(events));
            }
            *p.error.lock().unwrap() = None;
            p.set_note(if m.status.is_empty() { None } else { Some(render(&m.status, &ctx)) });
            if let Some(spec) = &m.attention {
                let said = attention(spec, &ctx, &mut p.watch.lock().unwrap());
                out.say = said.into_iter().map(|text| Say { text, priority: spec.priority, ..Default::default() }).collect();
            }
            state.plugins.apply(&app, &state, &p, out, false);
        }
        // (the card it had stays; a calendar's month view hears that its source is down)
        Err(e) => {
            if m.source.format == "ics" {
                crate::calendar::set_source(&app, &state, &format!("plugin:{id}"), Err(e.clone()));
                state.plugins.track_calendar(&id);
            }
            *p.error.lock().unwrap() = Some(e);
        }
    }
    *p.due.lock().unwrap() = Instant::now() + every;
    p.busy.store(false, Ordering::Relaxed);
}

/// A card could be seen: the hub is open, the island is showing, or a card floats on the screen.
pub fn card_visible(state: &IslandState) -> bool {
    state.shown.load(Ordering::Relaxed) || state.floats.any()
}

/// The audio capture runs for the eyes (the setting) and for a module that listens.
pub fn sync_audio(state: &IslandState) {
    let want = state.settings.lock().unwrap().react_to_audio || state.plugins.needs_audio();
    state.audio_enabled.store(want, Ordering::Relaxed);
    state.plugins.audio.wanted.store(state.plugins.needs_audio(), Ordering::Relaxed);
}

pub fn spawn(app: AppHandle, state: Arc<IslandState>) {
    state.plugins.register(&app, &state, crate::builtin::all());
    state.plugins.rescan();
    state.plugins.adopt_all(&crate::native::old_settings());
    sync_audio(&state);
    state.plugins.start_natives();
    crate::pluginwasm::spawn(app.clone(), state.clone());
    std::thread::spawn(move || {
        // background work: never compete with the UI for CPU
        unsafe {
            let _ = windows::Win32::System::Threading::SetThreadPriority(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL,
            );
        }
        loop {
            std::thread::sleep(Duration::from_millis(500));
            state.plugins.reconcile(&app, &state);
            state.plugins.flush(&app);
            for p in state.plugins.due(card_visible(&state)) {
                // one request at a time for a plugin; each runs on its own short-lived thread
                if p.busy.swap(true, Ordering::Relaxed) {
                    continue;
                }
                let (app, state) = (app.clone(), state.clone());
                std::thread::spawn(move || poll(p, app, state));
            }
        }
    });
}

// ----------------------------------------------------------------------------------------------- commands

#[tauri::command]
pub fn plugin_list(window: tauri::WebviewWindow) -> Listing {
    use tauri::Manager;
    window.state::<Arc<IslandState>>().plugins.listing()
}

#[tauri::command]
pub fn plugin_set(window: tauri::WebviewWindow, id: String, on: bool) {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>();
    state.plugins.set(&id, on);
    sync_audio(&state);
}

#[tauri::command]
pub fn plugin_value(window: tauri::WebviewWindow, id: String, key: String, value: Value) {
    use tauri::Manager;
    window.state::<Arc<IslandState>>().plugins.set_value(&id, &key, value);
}

#[tauri::command]
pub fn plugin_mute(window: tauri::WebviewWindow, id: String, muted: bool) {
    use tauri::Manager;
    window.state::<Arc<IslandState>>().plugins.set_muted(&id, muted);
}

#[tauri::command]
pub fn plugin_dnd(window: tauri::WebviewWindow, on: bool) {
    use tauri::Manager;
    window.state::<Arc<IslandState>>().plugins.set_dnd(on);
}

#[tauri::command]
pub fn plugin_rescan(window: tauri::WebviewWindow) -> Listing {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>();
    state.plugins.rescan();
    sync_audio(&state);
    state.plugins.listing()
}

/// Open the plugins folder in Explorer (made first if it is not there).
#[tauri::command]
pub fn plugin_folder() {
    if let Some(d) = dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::process::Command::new("explorer").arg(d).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Value {
        context_of(
            &json!([
                { "hash": "a", "name": "fedora.iso", "progress": 0.61, "size": 2147483648u64 },
                { "hash": "b", "name": "notes.zip", "progress": 0.18, "size": 5000 }
            ]),
            "",
        )
    }

    #[test]
    fn templates_read_paths_and_filters() {
        let c = ctx();
        assert_eq!(render("{{count}} downloading", &c), "2 downloading");
        assert_eq!(render("{{first.name}} · {{first.progress | percent}}", &c), "fedora.iso · 61%");
        assert_eq!(render("{{first.size | bytes}}", &c), "2.0 GB");
        assert_eq!(render("{{items.1.name | upper}}", &c), "NOTES.ZIP");
        assert_eq!(render("{{missing | default:none}}", &c), "none");
        assert_eq!(render("no holes", &c), "no holes");
        assert_eq!(render("open {{ hole", &c), "open {{ hole");
    }

    #[test]
    fn arithmetic_and_numbers_stay_numbers() {
        let c = json!({ "position": 30, "duration": 120, "t": 27.34 });
        assert_eq!(value_of("{{position / duration}}", &c), json!(0.25));
        assert_eq!(render("{{t | round}}", &c), "27");
        assert_eq!(render("{{t | round:1}}", &c), "27.3");
        assert_eq!(render("{{position / 0}}", &c), "");
    }

    #[test]
    fn the_card_is_made_from_blocks() {
        let m: Manifest = serde_json::from_value(json!({
            "id": "t", "name": "T",
            "card": {
                "icon": "download", "title": "{{count}} downloading", "sub": "{{first.name}}",
                "body": [
                    { "progress": "{{first.progress}}" },
                    { "list": "items", "limit": 1, "row": { "title": "{{name}}", "sub": "{{size | bytes}}", "progress": "{{progress}}" } },
                    { "facts": [["First", "{{first.name}}"], ["Nothing", "{{nope}}"]] },
                    { "text": "{{nope}}" }
                ]
            }
        }))
        .unwrap();
        let card = card_of(&m, &ctx());
        assert_eq!(card.title, "2 downloading");
        assert_eq!(card.body.len(), 3, "an empty text is left out, and so is a fact with no value");
        assert_eq!(card.body[0], Block::Progress { value: 0.61 });
        assert_eq!(card.body[2], Block::Facts { rows: vec![("First".into(), "fedora.iso".into())] });
        match &card.body[1] {
            Block::List { rows } => {
                assert_eq!(rows.len(), 1);
                assert_eq!((rows[0].title.as_str(), rows[0].sub.as_str()), ("fedora.iso", "2.0 GB"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn attention_comes_when_a_progress_arrives_not_before() {
        let spec = AttentionSpec { list: "items".into(), id: "hash".into(), field: "progress".into(), reaches: Some(1.0), priority: 50, banner: "Finished: {{name}}".into(), ..Default::default() };
        let mut w = Watch::default();
        let at = |p: f64| context_of(&json!([{ "hash": "a", "name": "fedora.iso", "progress": p }]), "");
        assert!(attention(&spec, &at(0.9), &mut w).is_empty(), "the first answer says nothing");
        assert!(attention(&spec, &at(0.95), &mut w).is_empty());
        assert_eq!(attention(&spec, &at(1.0), &mut w), vec!["Finished: fedora.iso".to_string()]);
        assert!(attention(&spec, &at(1.0), &mut w).is_empty(), "once");
        // one that was finished when the plugin started never speaks
        let mut w = Watch::default();
        assert!(attention(&spec, &at(1.0), &mut w).is_empty());
        assert!(attention(&spec, &at(1.0), &mut w).is_empty());
    }

    #[test]
    fn a_value_that_becomes_something() {
        let spec = AttentionSpec { field: "status".into(), becomes: Some("down".into()), banner: "{{name}} is down".into(), priority: 70, ..Default::default() };
        let mut w = Watch::default();
        let at = |s: &str| json!({ "name": "site", "status": s });
        assert!(attention(&spec, &at("up"), &mut w).is_empty());
        assert_eq!(attention(&spec, &at("down"), &mut w), vec!["site is down".to_string()]);
        assert!(attention(&spec, &at("down"), &mut w).is_empty());
        assert!(attention(&spec, &at("up"), &mut w).is_empty());
    }

    #[test]
    fn a_plugin_may_only_ask_what_it_listed() {
        let net = vec!["api.open-meteo.com".to_string(), "127.0.0.1:8080".to_string()];
        assert!(allowed("https://api.open-meteo.com/v1/forecast?x=1", &net).is_ok());
        assert!(allowed("http://127.0.0.1:8080/api", &net).is_ok());
        assert!(allowed("http://127.0.0.1:9090/api", &net).is_err(), "another port");
        assert!(allowed("https://evil.example.com/", &net).is_err());
        assert!(allowed("http://api.open-meteo.com/", &net).is_err(), "plain http to another machine");
        assert!(allowed("https://api.open-meteo.com@evil.example.com/", &net).is_err(), "a user name is not the host");
        assert!(allowed("ftp://api.open-meteo.com/", &net).is_err());
    }

    #[test]
    fn manifests_are_checked() {
        let m = |v: Value| serde_json::from_value::<Manifest>(v).unwrap();
        let ok = json!({ "id": "a-b", "name": "A", "permissions": { "net": ["x.com"] }, "source": { "http": "https://x.com/a" } });
        let here = std::path::Path::new(".");
        assert!(check(&m(ok.clone()), here, false).is_none());
        let mut bad = ok.clone();
        bad["id"] = json!("Bad Id!");
        assert!(check(&m(bad), here, false).is_some());
        let mut wasm = ok.clone();
        wasm["kind"] = json!("wasm");
        assert!(check(&m(wasm), here, false).unwrap().contains("names its file"));
        let mut far = ok.clone();
        far["source"]["http"] = json!("https://y.com/a");
        assert!(check(&m(far), here, false).unwrap().contains("not on the list"));
        // a declarative plugin cannot listen to the PC
        let mut ears = ok;
        ears["permissions"]["audio"] = json!(true);
        assert!(check(&m(ears), here, false).unwrap().contains("Only a module"));
    }

    fn manifest_of_json(v: Value) -> Manifest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_reply_is_read_into_what_the_island_does() {
        let m = manifest_of_json(json!({ "id": "p", "name": "P", "kind": "wasm" }));
        let out = out_of(
            &m,
            &json!({
                "card": { "icon": "clock", "title": "T", "body": [
                    { "kind": "buttons", "items": [{ "label": "Pause", "action": "pause" }, { "label": "No action" }] },
                    { "kind": "image", "src": "https://evil.example/x.png" },
                    { "kind": "image", "src": "data:image/png;base64,AAAA" }
                ] },
                "pill": { "icon": "music", "title": "Song", "rank": 500, "w": 9000.0, "h": 1.0, "bars": [2.0, -1.0], "look": "huge" },
                "rings": [{ "id": "disk", "pct": 140.0, "tip": "Disk", "color": "red" }, { "id": "Bad Id", "pct": 1.0 }],
                "say": [{ "text": "3:30 PM", "priority": 500, "pill": { "icon": "clock", "right": "Fri", "look": "big", "ms": 4000 }, "ambient": true }, { "text": "" }],
                "events": null, "quiet": true, "note": " Connected ", "wake_ms": 5, "media": { "action": "next", "source": "spotify" }
            }),
        );
        let card = out.card.unwrap().unwrap();
        assert_eq!(card.rank, 70, "a card with no rank sits at 70");
        // a button with no action and a picture that is not a data: raster are left out
        assert_eq!(card.body.len(), 2);
        assert_eq!(card.body[0], Block::Buttons { items: vec![Button { label: "Pause".into(), icon: "".into(), action: "pause".into() }] });
        assert_eq!(card.body[1], Block::Image { src: "data:image/png;base64,AAAA".into() });
        let pill = out.pill.unwrap().unwrap();
        assert_eq!((pill.rank, pill.w, pill.h, pill.look.as_str()), (99, 360.0, 32.0, ""));
        assert_eq!(pill.bars, vec![1.0, 0.0]);
        let rings = out.rings.unwrap();
        assert_eq!(rings.len(), 1, "a ring needs a valid id");
        assert_eq!((rings[0].pct, rings[0].color.as_str()), (100.0, "#9a9aa2"));
        assert_eq!(out.say.len(), 1);
        assert_eq!(out.say[0].priority, 100);
        assert!(out.say[0].ambient);
        let sp = out.say[0].pill.as_ref().unwrap();
        assert_eq!((sp.icon.as_str(), sp.right.as_str(), sp.look.as_str(), sp.ms), ("clock", "Fri", "big", 4000));
        assert!(matches!(out.events, Some(None)));
        assert_eq!(out.quiet, Some(true));
        assert_eq!(out.note, Some(Some("Connected".to_string())));
        assert_eq!(out.wake_ms, Some(50), "never faster than 50 ms");
        assert_eq!(out.media, Some(("next".into(), "spotify".into())));
        // a reply that says nothing changes nothing
        let nothing = out_of(&m, &json!({ "say": [] }));
        assert!(nothing.card.is_none() && nothing.pill.is_none() && nothing.rings.is_none() && nothing.events.is_none());
        assert!(nothing.quiet.is_none() && nothing.wake_ms.is_none() && nothing.media.is_none());
        // ... and `null` takes a thing away
        let gone = out_of(&m, &json!({ "card": null, "pill": null, "rings": null }));
        assert!(gone.card == Some(None) && gone.pill == Some(None) && gone.rings == Some(vec![]));
    }

    #[test]
    fn a_picture_is_a_data_raster_and_nothing_else() {
        assert!(image_ok("data:image/png;base64,AAAA"));
        assert!(image_ok("data:image/jpeg;base64,AAAA"));
        assert!(!image_ok("data:image/svg+xml;base64,AAAA"), "an svg can carry a script");
        assert!(!image_ok("https://example.com/a.png"), "the island does not fetch for a plugin");
        assert!(!image_ok("file:///C:/Users/a.png"));
        assert!(!image_ok(&format!("data:image/png;base64,{}", "A".repeat(MAX_IMAGE))));
    }

    #[test]
    fn a_host_can_be_a_setting() {
        let m = manifest_of_json(json!({ "id": "c", "name": "C", "permissions": { "net": ["{{settings.url}}", "api.example.com"] } }));
        let none = net_hosts(&m, &json!({ "url": "" }));
        assert_eq!(none, vec!["api.example.com".to_string()], "a setting that is empty allows nothing");
        let some = net_hosts(&m, &json!({ "url": "https://Calendar.Google.com:443/ical/x/basic.ics?a=b" }));
        assert!(some.contains(&"calendar.google.com:443".to_string()));
        assert!(allowed("https://calendar.google.com:443/ical/x/basic.ics", &some).is_ok());
        assert!(allowed("https://evil.example.com/", &some).is_err(), "the address the user gave is the only one");
    }

    #[test]
    fn a_redirect_may_be_relative_and_is_checked_like_any_address() {
        assert_eq!(resolve("https://a.example.com/x/y.ics", "/z.ics"), Some("https://a.example.com/z.ics".into()));
        assert_eq!(resolve("https://a.example.com/x", "https://b.example.com/q"), Some("https://b.example.com/q".into()));
        assert_eq!(resolve("https://a.example.com/x", "q.ics"), None);
        // (the answer to a redirect to somewhere not on the list is the same refusal as for any other address)
        assert!(allowed("https://b.example.com/q", &["a.example.com".into()]).is_err());
        assert!(is_local("http://127.0.0.1:8080/api") && is_local("http://localhost/x") && !is_local("https://example.com"));
    }

    #[test]
    fn a_declarative_pill_and_rings_come_from_templates() {
        let m = manifest_of_json(json!({ "id": "q", "name": "Q", "pill": { "when": "{{count}}", "icon": "download", "title": "{{count}} downloading", "right": "{{first.progress | percent}}", "progress": "{{first.progress}}", "rank": 40 },
            "rings": [{ "id": "disk", "color": "#ff8800", "pct": "{{first.progress * 100}}", "tip": "Disk {{first.progress | percent}}" }, { "id": "none", "color": "#fff", "pct": "{{nope}}" }] }));
        let c = ctx();
        let pill = pill_of(&m, m.pill.as_ref().unwrap(), &c).unwrap();
        assert_eq!((pill.title.as_str(), pill.right.as_str(), pill.progress, pill.rank), ("2 downloading", "61%", Some(0.61), 40));
        let rings = rings_of(&m.rings, &c);
        assert_eq!(rings.len(), 1, "a ring with no number is not shown");
        assert_eq!((rings[0].pct, rings[0].tip.as_str(), rings[0].color.as_str()), (61.0, "Disk 61%", "#ff8800"));
        // while `when` does not hold there is no pill
        let none = context_of(&json!([]), "");
        assert!(pill_of(&m, m.pill.as_ref().unwrap(), &none).is_none());
        assert!(truthy("yes") && !truthy("0") && !truthy(" false ") && !truthy(""));
    }

    #[test]
    fn the_ics_calendar_that_ships_with_the_app_is_a_plugin_like_any_other() {
        let b = crate::bundled::find("ics-calendar").unwrap();
        let m: Manifest = serde_json::from_str(b.manifest).unwrap();
        assert_eq!(m.kind, "declarative");
        // switched on with no link yet: waiting for the link is not a fault
        assert!(check(&m, std::path::Path::new(""), true).is_none());
        assert!(missing(&m, &json!({ "url": "  " })) && !missing(&m, &json!({ "url": "https://x.example/a.ics" })));
        // with a link, the only host it may ask is the link's
        let v = json!({ "url": "https://calendar.google.com/calendar/ical/x/basic.ics", "poll_min": 10 });
        let hosts = net_hosts(&m, &v);
        assert_eq!(hosts, vec!["calendar.google.com".to_string()]);
        assert_eq!(render(&m.source.every, &json!({ "settings": v })), "10m");
        assert!(allowed(&render(&m.source.http, &json!({ "settings": v })), &hosts).is_ok());
        // a module-only permission on a declarative plugin is refused
        let mut bad = m.clone();
        bad.permissions.calendar = true;
        assert!(check(&bad, std::path::Path::new(""), true).unwrap().contains("Only a module"));
    }

    #[test]
    fn intervals() {
        assert_eq!(parse_every("2s"), Duration::from_secs(2));
        assert_eq!(parse_every("15m"), Duration::from_secs(900));
        assert_eq!(parse_every("100ms"), MIN_EVERY, "never faster than a second");
        assert_eq!(parse_every("soon"), DEFAULT_EVERY);
        assert_eq!(parse_every(""), DEFAULT_EVERY);
    }
}
