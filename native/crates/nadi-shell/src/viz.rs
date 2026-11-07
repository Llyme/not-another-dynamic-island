//! The bars of the sound: the analyzer's spectrum in three bands (low, middle, high) for each speaker (the left, the right), each band
//! against a range of its own that follows the music (so a loud song and a quiet one both dance), the two speakers against the same
//! range (so the louder side stands taller), springing up and settling. Silence: the bars go to nothing.

use nadi_core::pc::Pc;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub struct Viz {
    seq: u64,
    hi: [f32; 3],
    lo: [f32; 3],
    /// the left speaker's low, middle and high, then the right's (0..1 each)
    pub now: [f32; 6],
    target: [f32; 6],
    /// the bass: how far it lifts the bars (0..1), and its own range
    pub lift: f32,
    lift_to: f32,
    bass_hi: f32,
    bass_lo: f32,
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
            let side = |k: &str| -> Vec<f32> { a[k].as_array().map(|v| v.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default() };
            let (sl, sr) = (side("spectrum_l"), side("spectrum_r"));
            if level < 0.01 || spec.is_empty() {
                self.target = [0.0; 6];
                self.lift_to = 0.0;
            } else {
                let n = spec.len();
                // (the bass, the lowest twelfth: it lifts the bars; the bands above it, their heights)
                let b0 = (n / 12).max(1).min(n.saturating_sub(3));
                let bass = &spec[..b0.max(1)];
                let x = bass.iter().sum::<f32>() / bass.len() as f32;
                self.bass_hi = x.max(self.bass_hi - 0.003);
                self.bass_lo = if self.bass_lo == 0.0 { x } else { x.min(self.bass_lo + 0.002) };
                let k = ((x - self.bass_lo) / (self.bass_hi - self.bass_lo).max(0.05)).clamp(0.0, 1.0);
                self.lift_to = k * k;
                let cuts = [b0, b0 + (n - b0) / 5, b0 + (n - b0) / 2, n];
                let avg = |s: &[f32], b: usize| -> Option<f32> {
                    let part = s.get(cuts[b]..cuts[b + 1].max(cuts[b] + 1).min(n))?;
                    Some(part.iter().sum::<f32>() / part.len().max(1) as f32)
                };
                for b in 0..3 {
                    let x = avg(&spec, b).unwrap_or(0.0);
                    self.hi[b] = x.max(self.hi[b] - 0.003);
                    self.lo[b] = if self.lo[b] == 0.0 { x } else { x.min(self.lo[b] + 0.002) };
                    let range = (self.hi[b] - self.lo[b]).max(0.05);
                    // (how loud the band is, against its range; then where it is: how much more of it one speaker has than the other,
                    // from their own meters (true dB), taken off the quieter side. Loud music never squashes the direction away)
                    let level = (0.25 + 0.75 * ((x - self.lo[b]) / range)).clamp(0.2, 1.0);
                    let lin = |v: f32| 10f32.powf((v * 66.0 - 72.0) / 20.0);
                    let bal = match (avg(&sl, b), avg(&sr, b)) {
                        (Some(l), Some(r)) => {
                            let (l, r) = (lin(l), lin(r));
                            ((r - l) / (r + l + 1e-9)).clamp(-1.0, 1.0)
                        }
                        _ => 0.0,
                    };
                    self.target[b] = (level * (1.0 - bal.max(0.0))).max(0.12);
                    self.target[3 + b] = (level * (1.0 + bal.min(0.0))).max(0.12);
                }
            }
        }
        let mut moving = false;
        for b in 0..6 {
            let k = if self.target[b] > self.now[b] { 22.0 } else { 9.0 };
            self.now[b] += (self.target[b] - self.now[b]) * (1.0 - (-dt * k).exp());
            if (self.target[b] - self.now[b]).abs() > 0.004 {
                moving = true;
            } else {
                self.now[b] = self.target[b];
            }
        }
        // (it hops up quickly with a beat, and drifts down)
        let k = if self.lift_to > self.lift { 30.0 } else { 7.0 };
        self.lift += (self.lift_to - self.lift) * (1.0 - (-dt * k).exp());
        if (self.lift_to - self.lift).abs() > 0.004 {
            moving = true;
        } else {
            self.lift = self.lift_to;
        }
        moving || self.now.iter().any(|v| *v > 0.0)
    }
}
