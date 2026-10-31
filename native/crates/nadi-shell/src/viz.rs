//! The three bars of the sound: the analyzer's spectrum in three bands (low, middle, high), each against a range of its own that follows
//! the music (so a loud song and a quiet one both dance), springing up and settling. Silence: the bars go to nothing.

use nadi_core::pc::Pc;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub struct Viz {
    seq: u64,
    hi: [f32; 3],
    lo: [f32; 3],
    pub now: [f32; 3],
    target: [f32; 3],
}

impl Viz {
    /// Read the newest tick of the analyzer and move the bars one frame (`dt` seconds) toward it. True while they move.
    pub fn step(&mut self, pc: &Pc, dt: f32) -> bool {
        let seq = pc.audio_seq.load(Ordering::Relaxed);
        if seq != self.seq {
            self.seq = seq;
            let a = pc.audio_last.lock().unwrap().clone();
            let level = a["level"].as_f64().unwrap_or(0.0) as f32;
            let spec: Vec<f32> = a["spectrum"].as_array().map(|v| v.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
            if level < 0.01 || spec.is_empty() {
                self.target = [0.0; 3];
            } else {
                let n = spec.len();
                let cuts = [0, n / 6, n / 2, n];
                for b in 0..3 {
                    let part = &spec[cuts[b]..cuts[b + 1].max(cuts[b] + 1).min(n)];
                    let x = part.iter().sum::<f32>() / part.len().max(1) as f32;
                    self.hi[b] = x.max(self.hi[b] - 0.003);
                    self.lo[b] = if self.lo[b] == 0.0 { x } else { x.min(self.lo[b] + 0.002) };
                    let range = (self.hi[b] - self.lo[b]).max(0.05);
                    self.target[b] = (0.25 + 0.75 * ((x - self.lo[b]) / range)).clamp(0.2, 1.0);
                }
            }
        }
        let mut moving = false;
        for b in 0..3 {
            let k = if self.target[b] > self.now[b] { 22.0 } else { 9.0 };
            self.now[b] += (self.target[b] - self.now[b]) * (1.0 - (-dt * k).exp());
            if (self.target[b] - self.now[b]).abs() > 0.004 {
                moving = true;
            } else {
                self.now[b] = self.target[b];
            }
        }
        moving || self.now.iter().any(|v| *v > 0.0)
    }
}
