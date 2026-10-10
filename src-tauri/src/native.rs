//! Native plugins: the features that need what a sandbox must not hand out, written in Rust inside the app.
//!
//! The island itself is a window manager: the pill, the hub, the banners and the floating cards, the calendar view, the
//! eyes and the sensors (audio, media session, which programs have a window). What it shows beyond that comes from
//! plugins. A native plugin is one of those, written in Rust and shipped inside the app (the ones that a module or a
//! declarative plugin can be are carried by the app as such: see bundled.rs); it goes through the same host as a folder
//! plugin: the same list in Settings > Plugins, the same switch (and the question before it is switched on), its own
//! settings, mute and do not disturb, and it reaches the screen only through what the host offers it here:
//!
//! * cards: `Native::cards` (rich ones, drawn by the plugin's own file in `ui/plugins`) or a `CardOut` of blocks
//! * a pill: `Ctx::offer_pill` (the collapsed island shows the highest offer when nothing outranks it)
//! * banners: `Ctx::say`
//! * calls from its own UI: `plugin_call`
//! * a quiet mode for the others: `Ctx::set_quiet` (a game is on the screen, no clock announcements)
//!
//! A native plugin that is switched off does nothing: its threads sleep, its cards and its pill are gone.

use crate::plugins::{Host, Manifest, Offer, Plugin, Saved};
use crate::settings;
use crate::IslandState;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

/// What a native plugin gets to work with: itself, as the host knows it.
#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
    pub state: Arc<IslandState>,
    pub id: &'static str,
    pills: Arc<Vec<crate::plugins::PillSpec>>,
}

impl Ctx {
    /// The plugin is switched on.
    pub fn on(&self) -> bool {
        self.state.plugins.is_on_id(self.id)
    }

    /// One of the plugin's settings (what the user set, or the manifest's default).
    pub fn value(&self, key: &str) -> Value {
        self.state.plugins.value(self.id, key)
    }

    pub fn text(&self, key: &str) -> String {
        match self.value(key) {
            Value::String(s) => s,
            Value::Null => String::new(),
            v => v.to_string(),
        }
    }

    pub fn number(&self, key: &str, default: f64) -> f64 {
        match self.value(key) {
            Value::Number(n) => n.as_f64().unwrap_or(default),
            Value::String(s) => s.trim().parse().unwrap_or(default),
            _ => default,
        }
    }

    pub fn flag(&self, key: &str, default: bool) -> bool {
        self.value(key).as_bool().unwrap_or(default)
    }

    pub fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self.app.emit(event, payload);
    }

    /// Say something in a banner, under the plugin's name (it waits by its priority; muted and do not disturb apply).
    pub fn say(&self, text: String, priority: i32) {
        if !self.on() {
            return;
        }
        let p = self.state.plugins.plugin(self.id);
        if let Some(p) = p {
            self.state.plugins.say(&self.app, &self.state, &p, text, priority, false);
        }
    }

    /// May a banner of this priority be shown? A muted plugin says nothing; with do not disturb only what is 90 or more
    /// rings (the rest is for the notification list, if the plugin wants it there).
    pub fn gate(&self, priority: i32) -> Gate {
        self.state.plugins.gate(self.id, priority)
    }

    /// Ask for the collapsed island to show this pill (one of the manifest's `pills`), or take the offer back.
    pub fn offer_pill(&self, pill: Option<&str>) {
        let offer = pill.and_then(|id| self.pills.iter().find(|p| p.id == id)).map(|p| Offer { id: p.id.clone(), w: p.w, h: p.h, rank: p.rank, brief: p.brief });
        let mut offers = self.state.plugins.offers.lock().unwrap();
        match offer {
            Some(o) if self.on() => {
                offers.insert(self.id.to_string(), o);
            }
            _ => {
                offers.remove(self.id);
            }
        }
    }

    /// Ask the other plugins to hold their peace (something fills the screen).
    pub fn set_quiet(&self, quiet: bool) {
        let mut q = self.state.plugins.quiet.lock().unwrap();
        if quiet && self.on() {
            q.insert(self.id.to_string());
        } else {
            q.remove(self.id);
        }
    }
}

/// What a native plugin takes over from the settings the app had before it was a plugin.
pub struct Adopted {
    pub on: bool,
    pub values: Map<String, Value>,
}

/// What a plugin's banner may do (see `Ctx::gate`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gate {
    /// the plugin is muted
    Muted,
    /// do not disturb: not now
    Held,
    Open,
}

fn gate_for(saved: &Saved, id: &str, priority: i32) -> Gate {
    if saved.muted.iter().any(|m| m == id) {
        Gate::Muted
    } else if saved.dnd && priority < 90 {
        Gate::Held
    } else {
        Gate::Open
    }
}

/// the pill the island should show: the highest of what the plugins offer
fn highest(offers: &HashMap<String, Offer>) -> Option<Offer> {
    offers.values().max_by_key(|o| o.rank).cloned()
}

pub trait Native: Send + Sync {
    /// Its manifest: the same file a folder plugin has (`src/builtin/<id>.json`), with `"kind": "native"`.
    fn manifest(&self) -> Manifest;

    /// The app has started: start what the plugin runs in the background (each thread checks `ctx.on()`).
    fn start(&self, _ctx: &Ctx) {}

    /// The plugin was switched on or off.
    fn switched(&self, _ctx: &Ctx, _on: bool) {}

    /// A setting of the plugin was changed.
    fn changed(&self, _ctx: &Ctx, _key: &str) {}

    /// The plugin's cards for the hub and the floating cards, in whatever shape its file in `ui/plugins` reads.
    fn cards(&self, _ctx: &Ctx) -> Option<Value> {
        None
    }

    /// An action that asks for a word (`ask` in its manifest) has its field filled: there is something kept already
    /// (a sign-in), and emptying the field takes it back.
    fn filled(&self, _ctx: &Ctx, _action: &str) -> bool {
        false
    }

    /// A line about how the plugin is doing, shown under its name in Settings > Plugins.
    fn status(&self, _ctx: &Ctx) -> Option<String> {
        None
    }

    /// A call from the plugin's own UI (`plugin_call`), or one of its manifest's `actions`.
    fn call(&self, _ctx: &Ctx, cmd: &str, _args: &Value) -> Result<Value, String> {
        Err(format!("no such call: {cmd}"))
    }

    /// First start after the app learned this plugin: switched on or off, and its settings, from what the old settings
    /// file said (`old` is that file, `null` when there is none).
    fn adopt(&self, _old: &Value, default_on: bool) -> Adopted {
        Adopted { on: default_on, values: Map::new() }
    }
}

pub struct Entry {
    pub plugin: Arc<Plugin>,
    pub native: Arc<dyn Native>,
    pub ctx: Ctx,
}

pub fn manifest_of(json: &str) -> Manifest {
    let mut m: Manifest = serde_json::from_str(json).expect("a native manifest is valid");
    m.kind = "native".into();
    m
}

impl Host {
    /// Make the native plugins known (before `rescan`, which puts them in the list).
    pub fn register(&self, app: &AppHandle, state: &Arc<IslandState>, natives: Vec<Arc<dyn Native>>) {
        let mut entries = Vec::new();
        for native in natives {
            let manifest = native.manifest();
            let id: &'static str = Box::leak(manifest.id.clone().into_boxed_str());
            let ctx = Ctx { app: app.clone(), state: state.clone(), id, pills: Arc::new(manifest.pills.clone()) };
            entries.push(Entry { plugin: Arc::new(Plugin::built_in(manifest)), native, ctx });
        }
        *self.natives.lock().unwrap() = entries;
    }

    /// A native plugin that was not here before gets its place: what the old settings said, or its default.
    pub fn adopt_all(&self, old: &Value) {
        // The usage rings were a plugin of their own (`claude-usage`), and are a setting of Claude Code now: one that was
        // switched off stays off.
        {
            let mut saved = self.saved.lock().unwrap();
            if saved.adopted.iter().any(|x| x == "claude-usage") {
                let was_on = saved.on.iter().any(|x| x == "claude-usage");
                saved.adopted.retain(|x| x != "claude-usage");
                saved.on.retain(|x| x != "claude-usage");
                saved.muted.retain(|x| x != "claude-usage");
                saved.values.remove("claude-usage");
                if !was_on {
                    saved.values.entry("claude-code".to_string()).or_default().insert("usage".into(), Value::Bool(false));
                }
                drop(saved);
                self.save();
            }
        }
        let todo: Vec<(Arc<dyn Native>, String, bool)> = {
            let saved = self.saved.lock().unwrap();
            self.natives
                .lock()
                .unwrap()
                .iter()
                .filter(|e| !saved.adopted.contains(&e.plugin.manifest.id))
                .map(|e| (e.native.clone(), e.plugin.manifest.id.clone(), e.plugin.manifest.default_on))
                .collect()
        };
        // (the plugins the app carries that are not in Rust, the same way)
        let carried: Vec<(String, bool)> = {
            let saved = self.saved.lock().unwrap();
            self.plugins
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.bundled.is_some() && !saved.adopted.contains(&p.manifest.id))
                .map(|p| (p.manifest.id.clone(), p.manifest.default_on))
                .collect()
        };
        if todo.is_empty() && carried.is_empty() {
            return;
        }
        {
            let mut saved = self.saved.lock().unwrap();
            for (id, default_on) in carried {
                let a = crate::bundled::adopt(&id, old, default_on);
                saved.on.retain(|x| *x != id);
                if a.on {
                    saved.on.push(id.clone());
                }
                if !a.values.is_empty() {
                    let slot = saved.values.entry(id.clone()).or_default();
                    for (k, v) in a.values {
                        slot.insert(k, v);
                    }
                }
                saved.adopted.push(id);
            }
            for (native, id, default_on) in todo {
                let a = native.adopt(old, default_on);
                saved.on.retain(|x| *x != id);
                if a.on {
                    saved.on.push(id.clone());
                }
                if !a.values.is_empty() {
                    let slot = saved.values.entry(id.clone()).or_default();
                    for (k, v) in a.values {
                        slot.insert(k, v);
                    }
                }
                saved.adopted.push(id);
            }
        }
        self.save();
    }

    pub fn start_natives(&self) {
        let list: Vec<(Arc<dyn Native>, Ctx)> = self.natives.lock().unwrap().iter().map(|e| (e.native.clone(), e.ctx.clone())).collect();
        for (n, ctx) in list {
            n.start(&ctx);
        }
    }

    pub(crate) fn plugin(&self, id: &str) -> Option<Arc<Plugin>> {
        self.plugins.lock().unwrap().iter().find(|p| p.manifest.id == id).cloned()
    }

    pub fn gate(&self, id: &str, priority: i32) -> Gate {
        gate_for(&self.saved.lock().unwrap(), id, priority)
    }

    pub fn is_on_id(&self, id: &str) -> bool {
        match self.plugin(id) {
            Some(p) => self.is_on(&p),
            None => false,
        }
    }

    /// One setting of a plugin.
    pub fn value(&self, id: &str, key: &str) -> Value {
        match self.plugin(id) {
            Some(p) => self.values(&p).get(key).cloned().unwrap_or(Value::Null),
            None => Value::Null,
        }
    }

    /// The cards of the native plugins that are on, by plugin id.
    pub fn native_cards(&self) -> Map<String, Value> {
        let list: Vec<(Arc<dyn Native>, Ctx)> = self.natives.lock().unwrap().iter().map(|e| (e.native.clone(), e.ctx.clone())).collect();
        let mut out = Map::new();
        for (n, ctx) in list {
            if !ctx.on() {
                continue;
            }
            if let Some(v) = n.cards(&ctx) {
                out.insert(ctx.id.to_string(), v);
            }
        }
        out
    }

    /// The pill the island should show now, if a plugin asks for one.
    pub fn pill(&self) -> Option<Offer> {
        highest(&self.offers.lock().unwrap())
    }

    /// A plugin asks the others to be quiet (a game is on the screen).
    pub fn quiet(&self) -> bool {
        !self.quiet.lock().unwrap().is_empty()
    }

    pub fn filled_of(&self, id: &str, action: &str) -> bool {
        let entry = self.natives.lock().unwrap().iter().find(|e| e.plugin.manifest.id == id).map(|e| (e.native.clone(), e.ctx.clone()));
        entry.map_or(false, |(n, ctx)| n.filled(&ctx, action))
    }

    pub fn note_of(&self, id: &str) -> Option<String> {
        let entry = self.natives.lock().unwrap().iter().find(|e| e.plugin.manifest.id == id).map(|e| (e.native.clone(), e.ctx.clone()));
        let (n, ctx) = entry?;
        if !ctx.on() {
            return None;
        }
        n.status(&ctx)
    }

    pub fn call(&self, id: &str, cmd: &str, args: &Value) -> Result<Value, String> {
        let entry = self.natives.lock().unwrap().iter().find(|e| e.plugin.manifest.id == id).map(|e| (e.native.clone(), e.ctx.clone()));
        // (a plugin that is not in Rust is told by an event: see `act`)
        let Some((n, ctx)) = entry else { return self.act(id, cmd, args.clone()) };
        if !ctx.on() {
            return Err(format!("{id} is off"));
        }
        n.call(&ctx, cmd, args)
    }
}

/// The old settings file as it was written (before its fields moved to the plugins), for `Native::adopt`.
pub fn old_settings() -> Value {
    settings::raw().unwrap_or(Value::Null)
}

/// `get(old, "game_detection", true)`
pub fn old_flag(old: &Value, key: &str, default: bool) -> bool {
    old.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

pub fn old_value(old: &Value, key: &str) -> Option<Value> {
    old.get(key).filter(|v| !v.is_null()).cloned()
}

/// What the hub and the floating cards draw: the cards of the plugins that are folders (blocks), and what each native
/// plugin that is on has to show (in the shape its file in `ui/plugins` reads).
#[derive(Serialize)]
pub struct Snapshot {
    plugins: Vec<crate::plugins::CardOut>,
    cards: Map<String, Value>,
    /// the rings the plugins show in the hub, by plugin id
    rings: Map<String, Value>,
    /// the pills the plugins offer
    pills: Vec<crate::plugins::PillOut>,
}

#[tauri::command]
pub fn get_activity(window: tauri::WebviewWindow) -> Snapshot {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>();
    let (pills, rings) = state.plugins.pills_and_rings();
    Snapshot { plugins: state.plugins.cards(), cards: state.plugins.native_cards(), rings, pills }
}

/// The pills the plugins offer and the rings they show, for a page that has just opened or a pill that has just come.
#[tauri::command]
pub fn plugin_pills(window: tauri::WebviewWindow) -> Value {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>();
    let (pills, rings) = state.plugins.pills_and_rings();
    json!({ "pills": pills, "rings": rings })
}

/// Run a plugin call off the UI thread: some of them wait for the browser or the disk.
#[tauri::command]
pub async fn plugin_call(window: tauri::WebviewWindow, id: String, cmd: String, args: Option<Value>) -> Result<Value, String> {
    use tauri::Manager;
    let state = window.state::<Arc<IslandState>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.plugins.call(&id, &cmd, &args.unwrap_or(json!(null))))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    // (no `Host` is made here: it holds what the window code needs, which a test program cannot load)

    fn offer(id: &str, rank: i32, brief: bool) -> Offer {
        Offer { id: id.into(), w: 260.0, h: 40.0, rank, brief }
    }

    #[test]
    fn a_banner_is_muted_or_held_or_open() {
        let mut saved = Saved::default();
        assert_eq!(gate_for(&saved, "a", 0), Gate::Open);
        saved.muted.push("a".into());
        assert_eq!(gate_for(&saved, "a", 100), Gate::Muted);
        assert_eq!(gate_for(&saved, "b", 0), Gate::Open);
        // do not disturb holds back all but what rings (90 and more)
        saved.dnd = true;
        assert_eq!(gate_for(&saved, "b", 89), Gate::Held);
        assert_eq!(gate_for(&saved, "b", 90), Gate::Open);
        // (muted stays muted)
        assert_eq!(gate_for(&saved, "a", 90), Gate::Muted);
    }

    #[test]
    fn the_island_shows_the_highest_pill_offered() {
        let mut offers: HashMap<String, Offer> = HashMap::new();
        assert_eq!(highest(&offers), None);
        offers.insert("work".into(), offer("work", 10, false));
        offers.insert("games".into(), offer("game", 20, false));
        assert_eq!(highest(&offers).map(|o| o.id), Some("game".to_string()));
        offers.insert("claude-code".into(), offer("usage_peek", 100, true));
        assert_eq!(highest(&offers).map(|o| o.id), Some("usage_peek".to_string()));
        assert!(highest(&offers).unwrap().brief);
        offers.remove("claude-code");
        offers.remove("games");
        assert_eq!(highest(&offers).map(|o| o.id), Some("work".to_string()));
    }
}
