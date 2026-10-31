//! The sound light: an aura inside a widget that dances to what plays. Soft lights, one per frequency band, drift through it; the vibe
//! picks how they dance (talk: a huddled murmur, chill: a slow drift, groove: an orbit, bang: a slam, calm: barely there). A voice gets a
//! warm halo that breathes with the syllables. When the sound stops, the light fades out.

use crate::director::{Audio, BANDS};
use crate::draw::Painter;
use crate::eyes::hsl;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

const FADE_MS: f32 = 700.0;

pub struct Aura {
    hue: f32,
    lights: Vec<Option<(f32, f32, f32, f32)>>,
    on: bool,
    fade_from: Option<f32>,
    pub brightness: f32,
}

fn var(i: usize, salt: f32) -> f32 {
    let x = ((i as f32) * 127.1 + salt * 311.7).sin() * 43758.547;
    x - x.floor()
}

impl Aura {
    pub fn new() -> Aura {
        Aura { hue: 150.0, lights: vec![None; BANDS], on: false, fade_from: None, brightness: 1.0 }
    }

    fn pose(i: usize, n: usize, t: f32, vibe: &str, w: f32, h: f32, kick: f32, env: f32, a: &Audio) -> (f32, f32) {
        let frac = i as f32 / n as f32;
        let tau = std::f32::consts::TAU;
        let (v1, v2, v3, v4) = (var(i, 1.0), var(i, 2.0), var(i, 3.0), var(i, 4.0));
        let fi = i as f32;
        match vibe {
            "talk" => {
                let (ax, fx, ay, fy) = (0.34 * (0.55 + 0.45 * v1), 0.9 * (0.7 + 0.6 * v2), 0.28 * (0.55 + 0.45 * v4), 1.1 * (0.7 + 0.6 * v1));
                (w * (0.5 + ax * (t * fx + fi * 0.55 + v3 * tau).sin() + a.pan * 0.1), h * (0.5 + ay * (t * fy + fi * 0.9 + v2 * tau).sin()))
            }
            "chill" => {
                let (ax, fx, ay, fy) = (0.42 * (0.55 + 0.45 * v1), 0.25 * (0.6 + 0.9 * v2), 0.36 * (0.55 + 0.45 * v3), 0.3 * (0.6 + 0.9 * v4));
                (w * (0.5 + ax * (t * fx + fi * 2.4 + v3 * tau).sin()), h * (0.5 + ay * (t * fy + fi * 1.8 + 1.0 + v4 * tau).sin()))
            }
            "bang" => {
                let ang = t * 2.2 * (0.7 + 0.6 * v1) + frac * tau + v2 * tau;
                let slam = kick * 0.16 * (0.6 + 0.8 * v4);
                let (rx, ry) = (0.44 * (0.6 + 0.4 * v3), 0.38 * (0.6 + 0.4 * v4));
                (w * (0.5 + rx * (ang + (t * (5.0 + 3.0 * v1) + fi + v2 * 6.0).sin() * 0.35).cos() + a.pan * 0.14), h * (0.5 + ry * (ang * (1.1 + 0.5 * v2) + fi + v3 * 6.0).sin() - slam))
            }
            "calm" => {
                let (ax, fx, ay, fy) = (0.26 * (0.55 + 0.45 * v1), 0.3 * (0.6 + 0.9 * v2), 0.2 * (0.55 + 0.45 * v3), 0.35 * (0.6 + 0.9 * v4));
                (w * (0.5 + ax * (t * fx + fi * 1.1 + v3 * tau).sin()), h * (0.5 + ay * (t * fy + fi * 1.7 + v4 * tau).sin()))
            }
            _ => {
                let sp = (0.5 + std::f32::consts::PI * (a.bpm / 60.0) * 0.04) * (0.7 + 0.6 * v1);
                let an = t * sp + frac * tau + v2 * tau + (a.sway + v3 * 6.0).sin() * 0.35;
                let bounce = env * 0.1 + kick * 0.05;
                let (gx, gy) = (0.38 * (0.6 + 0.4 * v3), (0.3 + bounce) * (0.6 + 0.4 * v4));
                (w * (0.5 + gx * an.cos() + 0.04 * (t * (1.3 + v1) + fi * 2.1).sin() + a.pan * 0.12), h * (0.5 + gy * an.sin() + 0.04 * (t * (1.7 + v2) + fi * 1.3).sin()))
            }
        }
    }

    fn hue_of(i: usize, n: usize, t: f32, vibe: &str, base: f32) -> f32 {
        let frac = i as f32 / n as f32;
        match vibe {
            "talk" => 28.0 + frac * 40.0 + (t * 0.7 + i as f32).sin() * 5.0,
            "chill" => 195.0 + (t * 0.2 + i as f32 * 0.5).sin() * 25.0 + frac * 30.0,
            "bang" => (8.0 + frac * 45.0 + t * 40.0 + (t * 5.0 + i as f32).sin() * 12.0 + 360.0) % 360.0,
            "calm" => (base + frac * 40.0) % 360.0,
            _ => (base + frac * 300.0) % 360.0,
        }
    }

    /// Paint it in a box `w` × `h` at `x`, `y` (logical px), with `eyes` where a voice's halo goes. False when there is nothing to show.
    pub fn paint(&mut self, p: &Painter, x: f32, y: f32, w: f32, h: f32, now: f32, dt: f32, a: &Audio, eyes: (f32, f32)) -> bool {
        let peak = a.bands.iter().cloned().fold(0.0, f32::max);
        let live = a.level > 0.03 || peak > 0.04;
        let mut fade = 1.0;
        if !live {
            if !self.on {
                return false;
            }
            let from = *self.fade_from.get_or_insert(now);
            fade = 1.0 - (now - from) / FADE_MS;
            if fade <= 0.0 {
                self.on = false;
                self.fade_from = None;
                return false;
            }
        } else {
            self.fade_from = None;
            self.on = true;
        }
        self.hue = (self.hue + dt * 7.0 * (1.0 - 0.75 * a.chill)) % 360.0;
        let vibe = a.state;
        let t = now / 1000.0 * if vibe == "chill" { 0.45 } else { 1.0 };
        let size = w.max(h);
        let crowd = (w / 260.0).clamp(0.5, 1.0);
        let kick = a.kick.p.clamp(0.0, 1.2);
        let musicish = 0.35 + 0.65 * (1.0 - a.voice_sm);
        let n = 8;
        let s = p.s;
        unsafe {
            p.dc.PushAxisAlignedClip(&D2D_RECT_F { left: x * s, top: y * s, right: (x + w) * s, bottom: (y + h) * s }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        for i in 0..n {
            let env = (a.bands[2 * i] + a.bands[2 * i + 1]) / 2.0;
            let (px, py) = Self::pose(i, n, t, vibe, w, h, kick, env, a);
            let (tx, ty) = ((px / w.max(1.0)).clamp(0.02, 0.98), (py / h.max(1.0)).clamp(0.02, 0.98));
            let cur = self.lights[i].get_or_insert((tx, ty, 0.0, 0.0));
            cur.2 += ((tx - cur.0) * 20.0 - cur.2 * 9.0) * dt;
            cur.3 += ((ty - cur.1) * 20.0 - cur.3 * 9.0) * dt;
            cur.0 = (cur.0 + cur.2 * dt).clamp(-0.05, 1.05);
            cur.1 = (cur.1 + cur.3 * dt).clamp(-0.05, 1.05);
            let (lx, ly) = (x + cur.0 * w, y + cur.1 * h);
            let hue = Self::hue_of(i, n, now / 1000.0, vibe, self.hue);
            let size_mul = match vibe {
                "talk" | "calm" => 0.16,
                "chill" => 0.3,
                "bang" => 0.2,
                _ => 0.17,
            };
            let r = (size * size_mul * (0.35 + 2.1 * env) * crowd * 0.65).max(1.0);
            let alpha = (0.03 + 0.32 * env) * a.level * musicish * 1.4 * crowd * crowd * self.brightness * fade;
            if alpha < 0.004 {
                continue;
            }
            glow(p, lx, ly, r, hsl(hue, 0.9, 0.62).alpha(alpha.min(0.5)));
        }
        if a.talk > 0.03 {
            let tr = size * (0.16 + 0.3 * a.talk_env) * 0.65;
            glow(p, eyes.0, eyes.1, tr.max(1.0), hsl(38.0, 0.95, 0.7).alpha((0.3 * a.talk * (0.4 + a.talk_env) * self.brightness * fade).min(0.5)));
        }
        unsafe { p.dc.PopAxisAlignedClip() };
        true
    }
}

/// A soft round light: its colour in the middle, gone at its edge.
fn glow(p: &Painter, x: f32, y: f32, r: f32, c: crate::paint::Color) {
    let s = p.s;
    unsafe {
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: c.d2d() },
            D2D1_GRADIENT_STOP { position: 1.0, color: c.alpha(0.0).d2d() },
        ];
        let rt: &ID2D1RenderTarget = p.dc;
        let Ok(coll) = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: D2D_POINT_2F { x: x * s, y: y * s }, gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 }, radiusX: r * s, radiusY: r * s };
        let Ok(b) = rt.CreateRadialGradientBrush(&props, None, &coll) else { return };
        p.dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: x * s, y: y * s }, radiusX: r * s, radiusY: r * s }, &b);
    }
}
