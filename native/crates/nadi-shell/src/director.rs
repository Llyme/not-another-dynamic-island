//! The sound, understood: the analyzer's ticks (loudness, 16 bands, beats, kicks, a speech-vs-music call) turned into the numbers the
//! faces move by (`Audio`), and a call of the music's vibe once a second (silent, talk, chill, groove, bang, calm) made from a rolling
//! profile of the track, so it holds steady. A bass hit is told as a beat (for the rings).

use nadi_core::pc::Pc;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub const BANDS: usize = 16;
const PROFILE_SECS: usize = 20;

#[derive(Clone, Copy, Default, Debug)]
pub struct Spring {
    pub p: f32,
    pub v: f32,
}

#[derive(Clone, Default, Debug)]
struct Sample {
    voice: f32,
    energy: f32,
    level: f32,
    beats: f32,
    kicks: f32,
}

#[derive(Clone, Default)]
struct Acc {
    n: u32,
    voice: f32,
    energy: f32,
    level: f32,
    beats: u32,
    kicks: u32,
}

/// What the faces read of the sound.
#[derive(Clone, Debug)]
pub struct Audio {
    pub level: f32,
    pub bands: [f32; BANDS],
    pub pan: f32,
    pub bpm: f32,
    pub voice_sm: f32,
    pub bang: f32,
    pub chill: f32,
    pub dance: f32,
    pub talk: f32,
    pub talk_env: f32,
    pub kick: Spring,
    pub nod: Spring,
    pub sway: f32,
    /// silent | talk | chill | groove | bang | calm
    pub state: &'static str,
    pub silent: bool,
}

impl Default for Audio {
    fn default() -> Self {
        Audio { level: 0.0, bands: [0.0; BANDS], pan: 0.0, bpm: 100.0, voice_sm: 0.0, bang: 0.0, chill: 0.0, dance: 0.0, talk: 0.0, talk_env: 0.0, kick: Spring::default(), nod: Spring::default(), sway: 0.0, state: "silent", silent: true }
    }
}

pub struct Director {
    pub audio: Audio,
    seq: u64,
    media_seen: u64,
    target: f32,
    band_target: [f32; BANDS],
    pan_target: f32,
    last_tick: Option<Instant>,
    last_beat: Option<Instant>,
    last_kick: Option<Instant>,
    acc: Acc,
    history: Vec<Sample>,
    cand: &'static str,
    count: u32,
    since: Instant,
    last_sample: Instant,
    track: String,
    video: bool,
    profiles: HashMap<String, Sample>,
    /// the bass hits since they were last taken: (strength, pan)
    beats: Vec<(f32, f32)>,
}

fn smooth(cur: f32, target: f32, dt: f32, up: f32, down: f32) -> f32 {
    cur + (target - cur) * (1.0 - (-dt * if target > cur { up } else { down }).exp())
}

fn avg(list: &[Sample]) -> Sample {
    let n = list.len().max(1) as f32;
    let mut o = Sample::default();
    for h in list {
        o.voice += h.voice;
        o.energy += h.energy;
        o.level += h.level;
        o.beats += h.beats;
        o.kicks += h.kicks;
    }
    Sample { voice: o.voice / n, energy: o.energy / n, level: o.level / n, beats: o.beats / n, kicks: o.kicks / n }
}

impl Director {
    pub fn new() -> Director {
        let now = Instant::now();
        Director {
            audio: Audio::default(),
            seq: 0,
            media_seen: u64::MAX,
            target: 0.0,
            band_target: [0.0; BANDS],
            pan_target: 0.0,
            last_tick: None,
            last_beat: None,
            last_kick: None,
            acc: Acc::default(),
            history: Vec::new(),
            cand: "silent",
            count: 0,
            since: now,
            last_sample: now,
            track: String::new(),
            video: false,
            profiles: HashMap::new(),
            beats: Vec::new(),
        }
    }

    /// The bass hits since the last look (each: strength 0..1, pan -1..1).
    pub fn take_beats(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.beats)
    }

    /// What plays (for the video call and the per-song profile).
    fn see_media(&mut self, pc: &Pc) {
        let list = pc.media_seen.lock().unwrap().clone();
        let arr = list.as_array().cloned().unwrap_or_default();
        let m = arr.iter().find(|s| s["playing"].as_bool() == Some(true)).or(arr.first()).cloned();
        let Some(m) = m else { return };
        let source = m["source"].as_str().unwrap_or("").to_lowercase();
        self.video = m["duration"].as_f64().unwrap_or(0.0) > 900.0
            || ["potplayer", "vlc", "mpv", "mpc", "wmplayer", "kmplayer", "gom", "video", "movies", "netflix", "primevideo", "disney", "hulu"].iter().any(|k| source.contains(k));
        let key = format!("{}|{}", m["title"].as_str().unwrap_or(""), m["artist"].as_str().unwrap_or(""));
        if key != self.track {
            self.track = key.clone();
            self.history = match self.profiles.get(&key) {
                Some(p) => vec![p.clone(); 8],
                None => Vec::new(),
            };
            self.since = Instant::now() - std::time::Duration::from_secs(10);
        }
    }

    fn tick(&mut self, a: &Value) {
        let f = |k: &str| a[k].as_f64().unwrap_or(0.0) as f32;
        let level = f("level");
        let bands: Vec<f32> = a["bands"].as_array().map(|b| b.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
        let beat = a["beat"].as_bool().unwrap_or(false);
        let kick = a["kick"].as_bool().unwrap_or(false);
        self.acc.n += 1;
        self.acc.voice += f("voice");
        self.acc.energy += f("energy");
        self.acc.level += level;
        self.acc.beats += beat as u32;
        self.acc.kicks += kick as u32;
        self.target = level;
        for i in 0..BANDS {
            self.band_target[i] = if bands.len() == BANDS { bands[i] } else { bands.get(i * bands.len().max(1) / BANDS).copied().unwrap_or(0.0) };
        }
        if f("bpm") > 0.0 {
            self.audio.bpm = f("bpm");
        }
        self.pan_target = f("pan");
        let now = Instant::now();
        self.last_tick = Some(now);
        let a2 = &mut self.audio;
        if kick && a2.bang > 0.25 && self.last_kick.map_or(true, |t| t.elapsed().as_millis() > 170) {
            self.last_kick = Some(now);
            a2.nod.v += 40.0 * a2.bang;
        }
        if beat && level > 0.15 && self.last_beat.map_or(true, |t| t.elapsed().as_millis() > 220) {
            self.last_beat = Some(now);
            a2.kick.v += (14.0 + 8.0 * level) * (1.0 - 0.88 * a2.chill) * (1.0 - 0.9 * a2.bang);
        }
        // rings are bass-only: a kick, or a strong low-end hit
        let bass = (self.band_target[0] + self.band_target[1] + self.band_target[2]) / 3.0;
        let calm = matches!(a2.state, "calm" | "talk");
        if kick && a2.chill < 0.55 && !calm {
            self.beats.push((0.25 + 0.75 * (0.4 + 0.6 * level).min(1.0), f("pan")));
        } else if f("hit") > 0.25 && bass > 0.45 && a2.chill < 0.55 && !calm {
            self.beats.push((0.12 + 0.88 * f("hit") * (0.45 + 0.55 * level), f("pan")));
        }
    }

    fn sample(&mut self, alive: bool) {
        let a = std::mem::take(&mut self.acc);
        let secs = self.last_sample.elapsed().as_secs_f32().max(0.5);
        self.last_sample = Instant::now();
        let n = a.n as f32;
        let mut cand = "silent";
        let cur = self.audio.state;
        if alive && a.n > 0 && a.level / n >= 0.05 {
            self.history.push(Sample { voice: a.voice / n, energy: a.energy / n, level: a.level / n, beats: a.beats as f32 / secs, kicks: a.kicks as f32 / secs });
            while self.history.len() > PROFILE_SECS {
                self.history.remove(0);
            }
            let p = avg(&self.history);
            let recent = avg(&self.history[self.history.len().saturating_sub(4)..]).voice;
            cand = if self.video {
                if recent > if cur == "talk" { 0.12 } else { 0.2 } {
                    "talk"
                } else {
                    "calm"
                }
            } else if recent > if cur == "talk" { 0.3 } else { 0.5 } {
                "talk"
            } else if p.kicks >= if cur == "bang" { 1.2 } else { 1.5 } && p.energy >= if cur == "bang" { 0.25 } else { 0.3 } && p.level > 0.5 {
                "bang"
            } else if p.energy < if cur == "chill" { 0.27 } else { 0.2 } && p.beats < 1.4 {
                "chill"
            } else {
                "groove"
            };
            if self.history.len() >= 12 && !self.track.is_empty() {
                if self.profiles.len() > 200 {
                    self.profiles.clear();
                }
                self.profiles.insert(self.track.clone(), p);
            }
        }
        if cand == cur {
            self.count = 0;
            return;
        }
        if cand == "silent" {
            self.audio.state = "silent";
            self.count = 0;
            self.since = Instant::now();
            return;
        }
        self.count = if cand == self.cand { self.count + 1 } else { 1 };
        self.cand = cand;
        if self.count >= 2 && self.since.elapsed().as_millis() >= 2000 {
            self.audio.state = cand;
            self.count = 0;
            self.since = Instant::now();
        }
    }

    /// One frame (`dt` seconds): read what the analyzer said since the last, and move every number. True while there is sound (or it
    /// is still settling).
    pub fn step(&mut self, pc: &Pc, dt: f32) -> bool {
        let seq = pc.audio_seq.load(Ordering::Relaxed);
        if seq != self.seq {
            self.seq = seq;
            let a = pc.audio_last.lock().unwrap().clone();
            if !a.is_null() {
                self.tick(&a);
            }
            let ms = pc.media_seq.load(Ordering::Relaxed);
            if ms != self.media_seen {
                self.media_seen = ms;
                self.see_media(pc);
            }
        }
        let alive = self.last_tick.is_some_and(|t| t.elapsed().as_millis() < 700);
        let au = &mut self.audio;
        let tgt = if alive { self.target } else { 0.0 };
        au.level = if tgt > au.level { smooth(au.level, tgt, dt, 30.0, 8.0) } else { tgt.max(au.level - dt) };
        for i in 0..BANDS {
            let bt = if alive { self.band_target[i] } else { 0.0 };
            let cur = au.bands[i];
            au.bands[i] = if bt > cur { smooth(cur, bt, dt, 40.0, 10.0) } else { bt.max(cur - 0.1 * dt) };
        }
        au.pan = smooth(au.pan, if alive { self.pan_target } else { 0.0 }, dt, 18.0, 6.0);
        if !alive {
            self.acc = Acc::default();
            if au.state != "silent" {
                au.state = "silent";
                self.since = Instant::now();
            }
        } else if self.last_sample.elapsed().as_millis() >= 1000 {
            self.sample(alive);
        }
        let au = &mut self.audio;
        let vibe = au.state;
        au.voice_sm = smooth(au.voice_sm, if vibe == "talk" { 1.0 } else { 0.0 }, dt, 3.0, 2.5);
        au.bang = smooth(au.bang, if vibe == "bang" { 1.0 } else { 0.0 }, dt, 3.5, 2.5);
        au.chill = smooth(au.chill, if vibe == "chill" { 1.0 } else { 0.0 }, dt, 1.2, 3.0);
        au.silent = !alive || au.level < 0.05;
        au.dance = smooth(au.dance, au.level * (1.0 - au.voice_sm) * if vibe == "calm" { 0.2 } else { 1.0 }, dt, 12.0, 4.0);
        au.talk = smooth(au.talk, au.level * au.voice_sm, dt, 12.0, 4.0);
        let mid = (au.bands[8] + au.bands[9] + au.bands[10] + au.bands[11] + au.bands[12]) / 5.0;
        au.talk_env = smooth(au.talk_env, 0.5 * au.level + 0.5 * mid, dt, 60.0, 16.0);
        au.nod.v += (420.0 * -au.nod.p - 26.0 * au.nod.v) * dt;
        au.nod.p += au.nod.v * dt;
        au.kick.v += (260.0 * -au.kick.p - 15.0 * au.kick.v) * dt;
        au.kick.p += au.kick.v * dt;
        if au.dance > 0.04 {
            au.sway += dt * std::f32::consts::PI * (au.bpm / 60.0) * (1.0 - 0.55 * au.chill);
        }
        alive || au.level > 0.001 || au.kick.p.abs() > 0.001 || au.bands.iter().any(|b| *b > 0.002)
    }
}
