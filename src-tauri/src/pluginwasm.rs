//! Module plugins: WebAssembly files that the host calls, and the sensors that call them.
//!
//! A module has no network and no way to reach Windows. The host reads what the manifest was allowed to see (the audio
//! as levels and bands, the programs that have a window, a file that changed, the calendar, what is playing, how busy
//! the PC is), turns it into an event and calls the module's one function with it. The module answers with what it wants
//! the island to do (a card, a pill, rings, banners, events for the calendar: see `plugins::Out`). Everything crosses as
//! JSON in the module's memory, so a plugin can be written in any language that makes WebAssembly.
//!
//! The module exports `memory`, `nadi_alloc(len) -> ptr` (where the host puts the event), `nadi_call(ptr, len) ->
//! i64` (the answer's pointer in the high 32 bits and its length in the low 32), and, if it likes, `nadi_free(ptr,
//! len)` (the host calls it for both buffers after a call). It may import nothing.
//!
//! What keeps it from lagging the island: one driver thread at low priority, a call is cut off when it has used its
//! fuel (about the time the manifest allows, 4 ms unless it asks for less), memory is capped, a module that fails
//! five calls in a row is switched off, and a banner is said at most every few seconds. A module's card is replaced
//! as a whole; the island draws it from its blocks.

use crate::plugins::{card_visible, out_of, Out, Plugin};
use crate::IslandState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use wasmi::{Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// Fuel is what wasmi counts down as it runs (about one for each instruction). This many for each millisecond a
/// manifest allows. Measured here, the interpreter spends about 3.4 million in a millisecond; this is less than half
/// of that, so a slower PC still stops a runaway call in the time it was given (a call is cut off by the fuel it
/// has used, never by a clock, so it is the same on every PC).
const FUEL_PER_MS: u64 = 1_500_000;
const DEFAULT_CALL_MS: u64 = 4;
const MAX_CALL_MS: u64 = 20;
const DEFAULT_MEMORY_MB: u64 = 8;
const MAX_MEMORY_MB: u64 = 64;
const MAX_EVENT: usize = 600_000;
const MAX_REPLY: usize = 100_000;
const MAX_FILE: u64 = 256 * 1024;
/// events of the calendar a module is told of (the ones not over yet, first ones first)
const CALENDAR_LISTED: usize = 100;
const STRIKES: u32 = 5;
const AUDIO_GAP: Duration = Duration::from_millis(40);
const PROCESSES_EVERY: Duration = Duration::from_secs(2);
const SYSTEM_EVERY: Duration = Duration::from_secs(2);
const SAY_GAP: Duration = Duration::from_secs(3);
const SAY_REPEAT: Duration = Duration::from_secs(60);

// ------------------------------------------------------------------------------------------------------ runtime

pub struct Runtime {
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    call: TypedFunc<(i32, i32), i64>,
    free: Option<TypedFunc<(i32, i32), ()>>,
    fuel: u64,
    /// the first call makes the module's memory (its allocator grows): it gets more
    warm: bool,
}

fn trap(e: wasmi::Error) -> String {
    let t = e.to_string();
    if t.to_lowercase().contains("fuel") {
        "It used up its time for one call.".to_string()
    } else {
        format!("The module stopped: {t}")
    }
}

impl Runtime {
    pub fn load(bytes: &[u8], call_ms: u64, memory_mb: u64) -> Result<Runtime, String> {
        let call_ms = if call_ms == 0 { DEFAULT_CALL_MS } else { call_ms.min(MAX_CALL_MS) };
        let memory_mb = if memory_mb == 0 { DEFAULT_MEMORY_MB } else { memory_mb.min(MAX_MEMORY_MB) };
        let mut cfg = Config::default();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg);
        let module = Module::new(&engine, bytes).map_err(|e| format!("The module is not valid: {e}"))?;
        if module.imports().next().is_some() {
            return Err("The module wants to import something; a module may import nothing.".into());
        }
        let limits = StoreLimitsBuilder::new().memory_size((memory_mb * 1024 * 1024) as usize).instances(1).memories(1).trap_on_grow_failure(true).build();
        let mut store = Store::new(&engine, limits);
        store.limiter(|l| l);
        let fuel = call_ms * FUEL_PER_MS;
        // (the module's own start code gets a few calls' worth)
        store.set_fuel(fuel * 8).map_err(|e| e.to_string())?;
        let linker = <Linker<StoreLimits>>::new(&engine);
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(trap)?;
        let memory = instance.get_memory(&store, "memory").ok_or("The module exports no memory.")?;
        let alloc = instance.get_typed_func::<i32, i32>(&store, "nadi_alloc").map_err(|_| "The module has no nadi_alloc.".to_string())?;
        let call = instance.get_typed_func::<(i32, i32), i64>(&store, "nadi_call").map_err(|_| "The module has no nadi_call.".to_string())?;
        let free = instance.get_typed_func::<(i32, i32), ()>(&store, "nadi_free").ok();
        Ok(Runtime { store, memory, alloc, call, free, fuel, warm: false })
    }

    /// One call. Returns what the module answered (JSON), or why it did not.
    pub fn invoke(&mut self, input: &Value) -> Result<Value, String> {
        let bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_EVENT {
            return Err("The event was too big.".into());
        }
        self.store.set_fuel(if self.warm { self.fuel } else { self.fuel * 8 }).map_err(|e| e.to_string())?;
        self.warm = true;
        let len = bytes.len() as i32;
        let ptr = self.alloc.call(&mut self.store, len).map_err(trap)?;
        self.memory.write(&mut self.store, ptr as u32 as usize, &bytes).map_err(|_| "The module gave a place outside its memory.".to_string())?;
        let packed = self.call.call(&mut self.store, (ptr, len)).map_err(trap)? as u64;
        let (out_ptr, out_len) = ((packed >> 32) as u32 as usize, (packed & 0xFFFF_FFFF) as usize);
        let out = if out_len == 0 {
            Value::Null
        } else if out_len > MAX_REPLY {
            return Err("The answer was too big.".into());
        } else {
            let mut buf = vec![0u8; out_len];
            self.memory.read(&self.store, out_ptr, &mut buf).map_err(|_| "The answer is outside the module's memory.".to_string())?;
            serde_json::from_slice(&buf).map_err(|_| "The answer is not JSON.".to_string())?
        };
        if let Some(f) = self.free.clone() {
            let _ = f.call(&mut self.store, (ptr, len));
            if out_len > 0 {
                let _ = f.call(&mut self.store, (out_ptr as i32, out_len as i32));
            }
        }
        Ok(out)
    }

    /// How much of its time one call took, in fuel (for measuring a module).
    #[cfg(test)]
    fn left(&self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }
}

// ------------------------------------------------------------------------------------------------------ sensors

/// The loopback audio the analyzer made, for the modules that may hear it. Only what the island already works out
/// (levels, bands, beats): never the sound.
#[derive(Default)]
pub struct AudioBus {
    /// a module that is on listens: the analyzer's ticks are kept
    pub wanted: AtomicBool,
    seq: AtomicU64,
    last: Mutex<Value>,
}

impl AudioBus {
    pub fn put(&self, tick: &crate::audio::AudioTick) {
        if !self.wanted.load(Ordering::Relaxed) {
            return;
        }
        let v = json!({
            "level": tick.level, "bands": tick.bands, "spectrum": tick.spectrum, "beat": tick.beat, "kick": tick.kick, "hit": tick.hit,
            "kind": tick.kind, "voice": tick.voice, "bpm": tick.bpm, "energy": tick.energy, "pan": tick.pan,
        });
        *self.last.lock().unwrap() = v;
        self.seq.fetch_add(1, Ordering::Relaxed);
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The programs that have a window, the one in front first (the titles only for a module that may read them).
fn windows_now(titles: bool) -> Value {
    let front = crate::winutil::foreground();
    let mut list: Vec<(bool, Value)> = crate::scan::visible_windows()
        .into_iter()
        .filter(|w| !w.minimized)
        .map(|w| {
            let exe = std::path::Path::new(&w.exe).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let is_front = w.hwnd.0 as isize == front;
            (is_front, json!({ "exe": exe, "pid": w.pid, "title": if titles { w.title } else { String::new() }, "front": is_front }))
        })
        .collect();
    list.sort_by_key(|(f, _)| !*f); // (stable: the rest keep their order)
    Value::Array(list.into_iter().map(|(_, v)| v).collect())
}

/// `~` and `%VARIABLE%` filled in.
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

/// The last part of a file (a log grows at its end).
fn tail_of(path: &std::path::Path) -> Option<(SystemTime, String)> {
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

// ------------------------------------------------------------------------------------------------------ driver

struct Slot {
    id: usize,
    rt: Runtime,
    started: bool,
    strikes: u32,
    last_audio: Instant,
    audio_seq: u64,
    last_proc: Instant,
    last_sys: Instant,
    last_tick: Instant,
    /// the module asked to be called again at this time (`wake_ms`)
    wake_at: Option<Instant>,
    /// the version of the calendar and the count of the media sensor it was last told of
    cal_version: Option<u64>,
    media_seq: Option<u64>,
    files: HashMap<PathBuf, SystemTime>,
    said: HashMap<String, Instant>,
    last_say: Instant,
}

fn load_slot(p: &Arc<Plugin>) -> Result<Slot, String> {
    let rt = match p.bundled.and_then(|b| b.module) {
        // (a plugin the app carries: its module is in the app)
        Some((_, bytes)) => Runtime::load(bytes, p.manifest.budget.call_ms, p.manifest.budget.memory_mb)?,
        None => {
            let bytes = std::fs::read(p.dir.join(&p.manifest.module)).map_err(|_| "The module file could not be read.".to_string())?;
            Runtime::load(&bytes, p.manifest.budget.call_ms, p.manifest.budget.memory_mb)?
        }
    };
    let long_ago = Instant::now() - Duration::from_secs(3600);
    Ok(Slot {
        id: Arc::as_ptr(p) as usize,
        rt,
        started: false,
        strikes: 0,
        last_audio: long_ago,
        audio_seq: 0,
        last_proc: long_ago,
        last_sys: long_ago,
        last_tick: Instant::now(),
        wake_at: None,
        cal_version: None,
        media_seq: None,
        files: HashMap::new(),
        said: HashMap::new(),
        last_say: long_ago,
    })
}

fn event(kind: &str, values: &Value, extra: Value) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("event".into(), json!(kind));
    o.insert("now".into(), json!(now_ms()));
    o.insert("settings".into(), values.clone());
    // (a module has no time zone: this is how far this PC's clock is from UTC, in minutes, now)
    o.insert("tz_offset".into(), json!(chrono::Local::now().offset().local_minus_utc() / 60));
    if let Value::Object(m) = extra {
        o.extend(m);
    }
    Value::Object(o)
}

/// The events a module is due, from what it may see. The flag is true for the answer to a button the user pressed.
fn due_events(p: &Plugin, slot: &mut Slot, values: &Value, state: &IslandState) -> Vec<(Value, bool)> {
    let perms = &p.manifest.permissions;
    let bus = &state.plugins.audio;
    let mut out: Vec<(Value, bool)> = Vec::new();
    if !slot.started {
        slot.started = true;
        out.push((event("start", values, json!({})), false));
    }
    if perms.audio {
        let seq = bus.seq.load(Ordering::Relaxed);
        if seq != slot.audio_seq && slot.last_audio.elapsed() >= AUDIO_GAP {
            slot.audio_seq = seq;
            slot.last_audio = Instant::now();
            let a = bus.last.lock().unwrap().clone();
            if !a.is_null() {
                out.push((event("audio", values, json!({ "audio": a })), false));
            }
        }
    }
    if perms.processes && slot.last_proc.elapsed() >= PROCESSES_EVERY {
        slot.last_proc = Instant::now();
        out.push((event("processes", values, json!({ "windows": windows_now(perms.titles) })), false));
    }
    if perms.calendar {
        // (when the calendar changed, and again each minute: what has finished drops out of the list)
        let v = state.calendar.version();
        if slot.cal_version != Some(v) {
            slot.cal_version = Some(v);
            let now = now_ms() as i64;
            let events: Vec<_> = state.calendar.snapshot().events.into_iter().filter(|e| e.end_ms >= now - 3_600_000).take(CALENDAR_LISTED).collect();
            out.push((event("calendar", values, json!({ "events": events })), false));
        }
    }
    if perms.media {
        let seq = state.media_seq.load(Ordering::Relaxed);
        if slot.media_seq != Some(seq) {
            slot.media_seq = Some(seq);
            let sessions = state.media_seen.lock().unwrap().clone();
            let sessions = if sessions.is_null() { json!([]) } else { sessions };
            out.push((event("media", values, json!({ "sessions": sessions })), false));
        }
    }
    if perms.system && slot.last_sys.elapsed() >= SYSTEM_EVERY {
        slot.last_sys = Instant::now();
        let s = crate::stats::read(state);
        out.push((event("system", values, json!({ "system": { "cpu": s.cpu_pct, "ram": s.ram_pct, "ram_used_gb": s.ram_used_gb, "ram_total_gb": s.ram_total_gb } })), false));
    }
    let tick_due = !p.manifest.tick.trim().is_empty() && slot.last_tick.elapsed() >= crate::plugins::parse_every(&p.manifest.tick);
    let wake_due = slot.wake_at.map_or(false, |t| Instant::now() >= t);
    if tick_due || wake_due {
        slot.last_tick = Instant::now();
        slot.wake_at = None;
        out.push((event("tick", values, json!({ "wake": wake_due })), false));
    }
    for f in &perms.files {
        let path = expand(f);
        if let Some((mtime, text)) = tail_of(&path) {
            if slot.files.get(&path) != Some(&mtime) {
                slot.files.insert(path.clone(), mtime);
                // (what the file held when the plugin was switched on is told too: it is the state, not news)
                out.push((event("file", values, json!({ "path": path.to_string_lossy(), "text": text })), false));
            }
        }
    }
    // the buttons the user pressed
    let pressed: Vec<(String, Value)> = p.actions.lock().unwrap().drain(..).collect();
    for (action, args) in pressed {
        out.push((event("action", values, json!({ "action": action, "args": args })), true));
    }
    out
}

/// A module that talks too much is not let to fill the island: one banner every few seconds, none repeated within a minute.
fn limit_says(slot: &mut Slot, out: &mut Out, by_user: bool) {
    if by_user {
        return;
    }
    out.say.retain(|s| {
        if slot.last_say.elapsed() < SAY_GAP || slot.said.get(&s.text).map_or(false, |t| t.elapsed() < SAY_REPEAT) {
            return false;
        }
        slot.last_say = Instant::now();
        slot.said.insert(s.text.clone(), Instant::now());
        true
    });
    slot.said.retain(|_, t| t.elapsed() < SAY_REPEAT);
}

fn run(p: &Arc<Plugin>, slot: &mut Slot, app: &AppHandle, state: &IslandState) {
    let values = state.plugins.values(p);
    for (ev, by_user) in due_events(p, slot, &values, state) {
        let t = Instant::now();
        let res = slot.rt.invoke(&ev);
        p.calls.fetch_add(1, Ordering::Relaxed);
        p.spent_us.fetch_add(t.elapsed().as_micros() as u64, Ordering::Relaxed);
        match res {
            Ok(reply) => {
                slot.strikes = 0;
                p.set_error(None);
                let mut out = out_of(&p.manifest, &reply);
                limit_says(slot, &mut out, by_user);
                if let Some(ms) = out.wake_ms {
                    slot.wake_at = Some(Instant::now() + Duration::from_millis(ms));
                }
                state.plugins.apply(app, state, p, out, by_user);
            }
            Err(e) => {
                slot.strikes += 1;
                p.set_error(Some(e.clone()));
                if slot.strikes >= STRIKES {
                    p.set_error(Some(format!("Switched off after {STRIKES} failed calls. {e}")));
                    state.plugins.set(&p.manifest.id, false);
                    crate::plugins::sync_audio(state);
                    return;
                }
            }
        }
    }
}

pub fn spawn(app: AppHandle, state: Arc<IslandState>) {
    std::thread::spawn(move || {
        // background work: never compete with the UI for CPU
        unsafe {
            let _ = windows::Win32::System::Threading::SetThreadPriority(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL,
            );
        }
        let mut slots: HashMap<String, Slot> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(20));
            let active = state.plugins.wasm_plugins();
            state.plugins.flush(&app);
            // (a plugin that was switched off, or read again, is let go; what it brought to the calendar goes with it, see
            // `Host::reconcile`)
            slots.retain(|id, s| active.iter().any(|p| &p.manifest.id == id && Arc::as_ptr(p) as usize == s.id));
            let seen = card_visible(&state);
            for p in active {
                if !(seen || p.manifest.source.when == "always") {
                    continue;
                }
                let id = p.manifest.id.clone();
                if !slots.contains_key(&id) {
                    match load_slot(&p) {
                        Ok(s) => {
                            p.set_error(None);
                            slots.insert(id.clone(), s);
                        }
                        Err(e) => {
                            // (it is tried again after a while: the file may be being written)
                            p.set_error(Some(e));
                            std::thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    }
                }
                if let Some(slot) = slots.get_mut(&id) {
                    run(&p, slot, &app, &state);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::Manifest;

    /// A module written by hand: it answers the same JSON each time.
    fn wat_module(answer: &str) -> Vec<u8> {
        let n = answer.len();
        format!(
            r#"(module
              (memory (export "memory") 2)
              (global $bump (mut i32) (i32.const 4096))
              (func (export "nadi_alloc") (param $len i32) (result i32)
                (local $p i32)
                (local.set $p (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $len)))
                (local.get $p))
              (data (i32.const 64) "{}")
              (func (export "nadi_call") (param i32 i32) (result i64)
                (i64.or (i64.shl (i64.const 64) (i64.const 32)) (i64.const {n}))))"#,
            answer.replace('\\', "\\\\").replace('"', "\\\"")
        )
        .into_bytes()
    }

    #[test]
    fn a_module_answers_with_json() {
        let mut rt = Runtime::load(&wat_module(r#"{"card":{"title":"hi"}}"#), 4, 8).unwrap();
        let v = rt.invoke(&json!({ "event": "start" })).unwrap();
        assert_eq!(v["card"]["title"], "hi");
        // and again: nothing is left over
        assert_eq!(rt.invoke(&json!({ "event": "tick" })).unwrap()["card"]["title"], "hi");
    }

    // (it times the interpreter, which runs at debug speed in a debug test: `cargo test --release`)
    #[cfg_attr(debug_assertions, ignore)]
    #[test]
    fn a_module_that_never_stops_is_cut_off() {
        let wat = r#"(module (memory (export "memory") 1)
          (func (export "nadi_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "nadi_call") (param i32 i32) (result i64) (loop $l (br $l)) (i64.const 0)))"#;
        let mut rt = Runtime::load(wat.as_bytes(), 4, 8).unwrap();
        let t = Instant::now();
        let e = rt.invoke(&json!({})).unwrap_err();
        assert!(e.contains("time"), "{e}");
        assert!(t.elapsed() < Duration::from_millis(500), "it took {:?}", t.elapsed());
        // it can be called again, with its fuel back
        assert!(rt.invoke(&json!({})).is_err());
    }

    #[test]
    fn a_module_may_import_nothing_and_must_have_the_two_functions() {
        let imports = r#"(module (import "env" "x" (func)) (memory (export "memory") 1))"#;
        assert!(Runtime::load(imports.as_bytes(), 4, 8).err().unwrap().contains("import"));
        let bare = r#"(module (memory (export "memory") 1))"#;
        assert!(Runtime::load(bare.as_bytes(), 4, 8).err().unwrap().contains("nadi_alloc"));
        assert!(Runtime::load(b"not wasm", 4, 8).is_err());
    }

    #[test]
    fn memory_is_capped() {
        // asks for 100 pages (6.4 MB) of a 1 MB allowance
        let wat = r#"(module (memory (export "memory") 100)
          (func (export "nadi_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "nadi_call") (param i32 i32) (result i64) (i64.const 0)))"#;
        assert!(Runtime::load(wat.as_bytes(), 4, 1).is_err());
        assert!(Runtime::load(wat.as_bytes(), 4, 8).is_ok());
    }

    fn manifest(v: Value) -> Manifest {
        serde_json::from_value(v).unwrap()
    }

    fn bundled(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins-bundled").join(name)).unwrap()
    }

    /// An example's module, as `plugins/build.ps1` builds it (the examples are not carried by the app). `None` when it was
    /// not built: the test then has nothing to run, and says so.
    fn example(dir: &str, file: &str) -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("plugins").join("dist").join(dir).join(file);
        let bytes = std::fs::read(&path).ok();
        if bytes.is_none() {
            eprintln!("skipped: {} is not built (run plugins/build.ps1)", path.display());
        }
        bytes
    }

    // (it times the interpreter, which runs at debug speed in a debug test: `cargo test --release`)
    #[cfg_attr(debug_assertions, ignore)]
    #[test]
    fn the_audio_meter_draws_bands_and_stays_inside_its_budget() {
        let Some(wasm) = example("audio-meter", "audio-meter.wasm") else { return };
        let mut rt = Runtime::load(&wasm, 4, 8).unwrap();
        let m = manifest(json!({ "id": "example-audio-meter", "name": "Audio meter", "live": true }));
        let ev = json!({
            "event": "audio", "now": 1000, "settings": { "show_bpm": true },
            "audio": { "level": 0.5, "spectrum": [0.1, 0.2, 0.9, 0.4, 0.3, 0.2, 0.1, 0.0, 0.5, 0.6, 0.7, 0.2, 0.1, 0.0, 0.3, 0.2], "beat": true,
                       "kick": false, "hit": 0.4, "kind": "music", "voice": 0.1, "bpm": 123.4, "energy": 0.6, "pan": 0.0 }
        });
        let before = rt.fuel;
        let reply = rt.invoke(&ev).unwrap();
        let used = before - rt.left();
        let card = crate::plugins::clean_card(&m, &reply["card"]).expect("a card");
        assert_eq!(card.sub, "Music · 123 bpm");
        assert_eq!(card.peek, "50%");
        match &card.body[0] {
            crate::plugins::Block::Bars { values } => {
                assert_eq!(values.len(), 16);
                assert!((values[2] - 0.9).abs() < 1e-3, "it rises at once");
            }
            other => panic!("{other:?}"),
        }
        // what a call costs once it is warm (the first one makes the module's memory), in fuel and in time
        let t = Instant::now();
        let mut warm = 0;
        for _ in 0..200 {
            rt.invoke(&ev).unwrap();
            warm = rt.fuel - rt.left();
        }
        println!("audio call: first {used}, warm {warm} fuel (of {before}); {:?} each", t.elapsed() / 200);
        assert!(warm < before / 2, "a call used {warm} of {before}");
        // the bars fall slowly
        let zeros = vec![0.0f32; 16];
        let quiet = json!({ "event": "audio", "now": 1050, "settings": {}, "audio": { "spectrum": zeros, "kind": "silent", "level": 0.0 } });
        let low = crate::plugins::clean_card(&m, &rt.invoke(&quiet).unwrap()["card"]).unwrap();
        match &low.body[0] {
            crate::plugins::Block::Bars { values } => assert!(values[2] > 0.5 && values[2] < 0.9, "{}", values[2]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_game_clock_tells_the_time_once_in_a_while() {
        let Some(wasm) = example("game-clock", "game-clock.wasm") else { return };
        let mut rt = Runtime::load(&wasm, 4, 8).unwrap();
        let settings = json!({ "games": "endfield.exe, hl2.exe", "remind_min": 60 });
        let at = |ms: u64, exe: &str| {
            json!({ "event": "processes", "now": ms, "settings": settings, "windows": [
                { "exe": "chrome.exe", "pid": 1, "title": "", "front": false }, { "exe": exe, "pid": 2, "title": "", "front": true }] })
        };
        let r = rt.invoke(&at(1_000_000, "endfield.exe")).unwrap();
        assert_eq!(r["card"]["title"], "Endfield");
        assert_eq!(r["card"]["sub"], "Playing · 0m");
        assert!(r["say"].as_array().unwrap().is_empty());
        // 61 minutes later: it says so, once
        let r = rt.invoke(&at(1_000_000 + 61 * 60_000, "endfield.exe")).unwrap();
        assert_eq!(r["card"]["sub"], "Playing · 1h 1m");
        assert_eq!(r["say"][0]["priority"], 40);
        assert!(r["say"][0]["text"].as_str().unwrap().contains("Endfield"));
        let r = rt.invoke(&at(1_000_000 + 62 * 60_000, "endfield.exe")).unwrap();
        assert!(r["say"].as_array().unwrap().is_empty(), "not again");
        // the game is closed: the card goes
        let r = rt.invoke(&at(1_000_000 + 63 * 60_000, "notepad.exe")).unwrap();
        assert!(r["card"].is_null() && r.get("card").is_some());
        // another kind of event: nothing to say
        assert!(rt.invoke(&json!({ "event": "tick", "now": 0, "settings": settings })).unwrap().as_object().map_or(true, |o| !o.contains_key("card")));
    }

    #[test]
    fn the_time_plugin_says_the_hour_in_a_pill() {
        let mut rt = Runtime::load(&bundled("time/time.wasm"), 4, 8).unwrap();
        let ev = |kind: &str, now: u64, extra: Value| {
            let mut e = json!({ "event": kind, "now": now, "tz_offset": 0, "settings": { "every": 60, "h24": false } });
            e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            e
        };
        // 2026-10-09 15:00:00 UTC: a mark for every hour
        let at = 1_791_558_000_000u64;
        let m = manifest(json!({ "id": "time", "name": "Time", "kind": "wasm" }));
        let out = crate::plugins::out_of(&m, &rt.invoke(&ev("tick", at, json!({}))).unwrap());
        assert_eq!(out.say.len(), 1);
        assert_eq!(out.say[0].text, "3:00 PM");
        assert!(out.say[0].ambient && out.say[0].pill.as_ref().unwrap().look == "big");
        assert_eq!(out.say[0].pill.as_ref().unwrap().right, "Fri, Oct 9");
        assert!(out.wake_ms.unwrap() <= 60_020, "it asks to be woken at the next minute");
        // the same minute: nothing; a minute later: nothing (not a mark)
        assert!(crate::plugins::out_of(&m, &rt.invoke(&ev("tick", at + 3000, json!({}))).unwrap()).say.is_empty());
        assert!(crate::plugins::out_of(&m, &rt.invoke(&ev("tick", at + 60_000, json!({}))).unwrap()).say.is_empty());
        // the button: the time, now
        let out = crate::plugins::out_of(&m, &rt.invoke(&ev("action", at + 90_000, json!({ "action": "preview" }))).unwrap());
        assert_eq!(out.say[0].text, "3:01 PM");
        assert!(!out.say[0].ambient);
    }

    #[test]
    fn the_ics_calendar_plugin_lists_the_calendar_and_reminds_once() {
        let mut rt = Runtime::load(&bundled("ics-calendar/ics-calendar.wasm"), 4, 8).unwrap();
        let m = manifest(json!({ "id": "ics-calendar", "name": "ICS Calendar", "kind": "wasm" }));
        let now = 1_791_558_000_000u64;
        let events = json!([
            { "uid": "a", "summary": "Standup", "start_ms": now as i64 + 10 * 60_000, "end_ms": now as i64 + 40 * 60_000, "all_day": false },
            { "uid": "b", "summary": "Lunch", "start_ms": now as i64 + 300 * 60_000, "end_ms": now as i64 + 360 * 60_000, "all_day": false }
        ]);
        let call = |kind: &str, at: u64| json!({ "event": kind, "now": at, "tz_offset": 0, "settings": { "remind": true, "lead_min": 15 }, "events": events });
        let out = crate::plugins::out_of(&m, &rt.invoke(&call("calendar", now)).unwrap());
        let card = out.card.unwrap().unwrap();
        assert_eq!((card.title.as_str(), card.rank), ("Standup", 45));
        assert_eq!(card.sub, format!("{{when:{}}}", now as i64 + 10 * 60_000));
        assert_eq!(out.say.len(), 1);
        assert_eq!(out.say[0].text, "Standup in 10 min");
        assert_eq!(out.wake_ms, Some(15_000));
        assert!(crate::plugins::out_of(&m, &rt.invoke(&call("tick", now + 15_000)).unwrap()).say.is_empty(), "once");
    }

    #[test]
    fn files_and_paths() {
        std::env::set_var("NADI_TEST_DIR", "C:/x");
        assert_eq!(expand("%NADI_TEST_DIR%/a.log"), PathBuf::from("C:/x/a.log"));
        let dir = std::env::temp_dir().join(format!("nadi-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("big.log");
        std::fs::write(&f, "x".repeat(300 * 1024) + "END").unwrap();
        let (_, t) = tail_of(&f).unwrap();
        assert_eq!(t.len() as u64, MAX_FILE);
        assert!(t.ends_with("END"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
