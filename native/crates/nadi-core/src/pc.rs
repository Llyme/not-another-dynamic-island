//! What NADI knows of the PC right now, shared between the sensors (each on a thread of its own, writing here) and whoever reads it (the
//! plugins' host, and the shell: the bars of the sound, the media of the dock). A sensor that nobody wants does not run: the host says
//! what is wanted from the plugins that are on.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What is wanted from the sensors (a plugin that may see it is on, or the shell draws it).
#[derive(Default)]
pub struct Wants {
    pub media: AtomicBool,
    pub media_art: AtomicBool,
    pub audio: AtomicBool,
    /// the shell draws the sound (the bars of a widget): it listens whatever the plugins want
    pub shell_audio: AtomicBool,
    /// the shell shows what plays (the dock's Now Card): looked at whatever the plugins want, with its pictures
    pub shell_media: AtomicBool,
    pub notifications: AtomicBool,
}

#[derive(Default)]
pub struct Pc {
    pub want: Wants,
    /// what plays, as the plugins are told it (the pictures by id, in `media_arts`), and a count that moves at each change
    pub media_seen: Mutex<Value>,
    pub media_seq: AtomicU64,
    pub media_arts: Mutex<HashMap<String, (u64, String)>>,
    /// the sound, as the analyzer last read it: for the plugins (JSON) and the shell's bars; and a count that moves at each tick
    pub audio_last: Mutex<Value>,
    pub audio_seq: AtomicU64,
    /// CPU and memory (the sampler keeps its last reading: CPU use is a difference between two)
    pub sys: Mutex<Option<sysinfo::System>>,
    pub gpu: crate::sensors::gpu::GpuState,
    /// the notifications of other apps, numbered, the last few
    pub toasts: Mutex<Vec<Value>>,
    pub toast_seq: AtomicU64,
    /// what each plugin has cost since NADI started (by its id; "~ name": a sensor gathered for them): ms gathering its events, ms
    /// running them, how many events
    pub plugin_cost: Mutex<std::collections::BTreeMap<String, (f64, f64, u64)>>,
}

impl Pc {
    pub fn new() -> Arc<Pc> {
        Arc::new(Pc::default())
    }

    /// a tick of the sound analyzer
    pub fn audio_put(&self, tick: &crate::sensors::audio::AudioTick) {
        let v = serde_json::json!({
            "level": tick.level, "bands": tick.bands, "spectrum": tick.spectrum, "spectrum_l": tick.spectrum_l, "spectrum_r": tick.spectrum_r, "beat": tick.beat, "kick": tick.kick, "hit": tick.hit,
            "kind": tick.kind, "voice": tick.voice, "bpm": tick.bpm, "energy": tick.energy, "pan": tick.pan,
        });
        *self.audio_last.lock().unwrap() = v;
        self.audio_seq.fetch_add(1, Ordering::Relaxed);
    }
}

/// Start every sensor's thread (each idles while it is not wanted).
pub fn start(pc: &Arc<Pc>) {
    crate::sensors::media::spawn(pc.clone());
    crate::sensors::audio::spawn(pc.clone());
    crate::sensors::notifysensor::spawn(pc.clone());
}
