//! The eyes: they blink (and now and then wink, or blink twice), look at the cursor, squash and stretch as it moves, glance around when
//! it rests, and show moods: happy (the cursor over them, a press), sleepy (left alone a long while), wide (startled awake), and with
//! music, the vibe's: > < for punchy, u u for chill, arches for a groove. They bob to the kick, sway with the tempo, and talk with a
//! voice. Each eye is one closed shape whose numbers morph between moods.

use crate::director::{Audio, Spring};
use crate::draw::Painter;
use crate::paint::{brush, Color};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::*;

const EYE: Color = Color(0.976, 0.976, 0.988, 1.0);
const STEPS: usize = 32;

#[derive(Clone, Copy)]
struct Shape {
    rx: f32,
    top: f32,
    bot: f32,
    c0: f32,
    k: f32,
    pw: f32,
}

const NEUTRAL: Shape = Shape { rx: 5.1, top: 5.8, bot: 5.8, c0: 0.0, k: 0.0, pw: 2.0 };
const HAPPY: Shape = Shape { rx: 6.0, top: 2.0, bot: 2.0, c0: -2.6, k: 5.2, pw: 2.0 };
const SLEEPY: Shape = Shape { rx: 5.6, top: 0.6, bot: 5.0, c0: -2.0, k: 0.0, pw: 2.0 };
const WIDE: Shape = Shape { rx: 4.6, top: 8.0, bot: 8.0, c0: 0.0, k: 0.0, pw: 2.0 };
const XD: Shape = Shape { rx: 6.8, top: 1.5, bot: 1.5, c0: -3.5, k: 7.0, pw: 1.1 };
const BLISS: Shape = Shape { rx: 6.0, top: 2.0, bot: 2.0, c0: 2.6, k: -5.2, pw: 2.0 };

fn mix(h: f32, s: f32, w: f32, x: f32, b: f32) -> Shape {
    let f = |g: fn(&Shape) -> f32| {
        let n = g(&NEUTRAL);
        n + h * (g(&HAPPY) - n) + s * (g(&SLEEPY) - n) + w * (g(&WIDE) - n) + x * (g(&XD) - n) + b * (g(&BLISS) - n)
    };
    Shape { rx: f(|s| s.rx), top: f(|s| s.top).max(0.4), bot: f(|s| s.bot).max(0.4), c0: f(|s| s.c0), k: f(|s| s.k), pw: f(|s| s.pw) }
}

thread_local! {
    static SEED: std::cell::Cell<u64> = std::cell::Cell::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(88172645463325252, |d| d.as_nanos() as u64 | 1));
}

/// a number between `lo` and `hi` (xorshift: the eyes need whim, not secrecy)
fn rand(lo: f32, hi: f32) -> f32 {
    let x = SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    });
    lo + ((x >> 40) as f32 / (1u64 << 24) as f32) * (hi - lo)
}

fn spring_to(s: &mut Spring, target: f32, dt: f32) {
    // underdamped on purpose: an expression pops past its target and settles
    let a = 340.0 * (target - s.p) - 19.0 * s.v;
    s.v += a * dt;
    s.p += s.v * dt;
}

#[derive(PartialEq, Clone, Copy)]
enum Blink {
    Open,
    Closing,
    Opening,
}

pub struct Eyes {
    blink: Blink,
    blink_start: f32,
    next_blink: f32,
    openness: f32,
    squish: f32,
    squish_dir: (f32, f32),
    cursor: (f32, f32),
    last_cursor: Option<(f32, f32)>,
    double: bool,
    wink: i32,
    happy: Spring,
    sleepy: Spring,
    wide: Spring,
    xd: Spring,
    bliss: Spring,
    pub hover: bool,
    happy_until: f32,
    wide_until: f32,
    last_move: f32,
    glance: (f32, f32),
    glance_target: (f32, f32),
    glance_w: f32,
    next_glance: f32,
    emote: &'static str,
    emote_vibe: &'static str,
    emote_until: f32,
    started: bool,
}

impl Eyes {
    pub fn new() -> Eyes {
        Eyes {
            blink: Blink::Open,
            blink_start: 0.0,
            next_blink: 0.0,
            openness: 1.0,
            squish: 0.0,
            squish_dir: (0.0, 0.0),
            cursor: (0.0, 0.0),
            last_cursor: None,
            double: false,
            wink: 0,
            happy: Spring::default(),
            sleepy: Spring::default(),
            wide: Spring::default(),
            xd: Spring::default(),
            bliss: Spring::default(),
            hover: false,
            happy_until: 0.0,
            wide_until: 0.0,
            last_move: 0.0,
            glance: (0.0, 0.0),
            glance_target: (0.0, 0.0),
            glance_w: 0.0,
            next_glance: 0.0,
            emote: "none",
            emote_vibe: "",
            emote_until: 0.0,
            started: false,
        }
    }

    /// A press on them: a little happy blip; a second one soon after: boop (wide).
    pub fn poke(&mut self, now: f32) {
        if now < self.happy_until {
            self.wide_until = now + 320.0;
        } else {
            self.happy_until = now + 900.0;
        }
    }

    /// Happy to see you (their panel opened).
    pub fn greet(&mut self, now: f32) {
        self.happy_until = now + 1600.0;
    }

    fn pick_emote(&mut self, now: f32, vibe: &'static str) {
        if vibe == self.emote_vibe && now < self.emote_until {
            return;
        }
        self.emote_vibe = vibe;
        let options: &[&'static str] = match vibe {
            "bang" => &["xd", "arch", "xd", "arch", "open"],
            "chill" => &["bliss", "sleepy", "bliss", "open"],
            "groove" => &["open", "open", "arch"],
            _ => &["none"],
        };
        let mut next = options[(rand(0.0, options.len() as f32) as usize).min(options.len() - 1)];
        if next == self.emote && options.len() > 1 {
            let i = options.iter().position(|o| *o == next).unwrap_or(0);
            next = options[(i + 1) % options.len()];
        }
        self.emote = next;
        self.emote_until = now + rand(5000.0, 9000.0);
    }

    /// One frame: `now` ms, `cursor` where the mouse is (logical px, from the middle of the eyes), the sound.
    pub fn update(&mut self, now: f32, dt: f32, cursor: (f32, f32), audio: &Audio) {
        if !self.started {
            self.started = true;
            self.next_blink = now + rand(2500.0, 5500.0);
            self.last_move = now;
        }
        // the cursor: how far it moved, and how it moves the eyes
        let (dx, dy) = self.last_cursor.map_or((0.0, 0.0), |(x, y)| (cursor.0 - x, cursor.1 - y));
        self.last_cursor = Some(cursor);
        let speed = (dx * dx + dy * dy).sqrt();
        if speed > 2.0 {
            if now - self.last_move > 25000.0 {
                self.wide_until = now + 500.0;
            }
            self.last_move = now;
        }
        let target_sq = (speed / 140.0).min(1.0);
        if target_sq > self.squish {
            self.squish = target_sq;
            if speed > 0.5 {
                self.squish_dir = (dx / speed, dy / speed);
            }
        } else {
            self.squish *= 0.78;
        }
        self.cursor.0 += (cursor.0 - self.cursor.0) * 0.35;
        self.cursor.1 += (cursor.1 - self.cursor.1) * 0.35;
        // the mood
        let idle = now - self.last_move;
        let wide = if now < self.wide_until { 1.0 } else { 0.0 };
        let happy = if wide == 0.0 && (self.hover || now < self.happy_until) { 1.0 } else { 0.0 };
        let sleepy: f32 = if wide == 0.0 && happy == 0.0 && idle > 25000.0 && audio.silent { 1.0 } else { 0.0 };
        spring_to(&mut self.wide, wide, dt);
        self.pick_emote(now, audio.state);
        let e = if happy == 0.0 && wide == 0.0 { self.emote } else { "none" };
        spring_to(&mut self.happy, if e == "arch" { 1.0f32.max(happy) } else { happy }, dt);
        spring_to(&mut self.xd, if e == "xd" { 1.0 } else { 0.0 }, dt);
        spring_to(&mut self.bliss, if e == "bliss" { 1.0 } else { 0.0 }, dt);
        let sl = sleepy.max(if e == "sleepy" { 0.75 } else { 0.3 * audio.chill * if e == "bliss" { 0.0 } else { 1.0 } });
        spring_to(&mut self.sleepy, sl, dt);
        // glances: the cursor parked a while, they look around
        let glancing = idle > 4000.0 && happy == 0.0 && wide == 0.0;
        if glancing && now >= self.next_glance {
            const SPOTS: [(f32, f32); 7] = [(-1.0, 0.1), (1.0, 0.1), (0.7, -0.8), (-0.7, -0.8), (0.0, 0.0), (-0.9, 0.5), (0.9, 0.5)];
            self.glance_target = SPOTS[(rand(0.0, 7.0) as usize).min(6)];
            self.next_glance = now + rand(1400.0, 3400.0);
        }
        let k = 1.0 - (-dt * 14.0).exp();
        self.glance.0 += (self.glance_target.0 - self.glance.0) * k;
        self.glance.1 += (self.glance_target.1 - self.glance.1) * k;
        self.glance_w += ((if glancing { 1.0 } else { 0.0 }) - self.glance_w) * (1.0 - (-dt * 8.0).exp());
        self.tick_blink(now);
    }

    fn tick_blink(&mut self, now: f32) {
        let expressive = self.happy.p.max(self.xd.p).max(self.bliss.p).max(self.sleepy.p) > 0.35;
        if expressive && self.blink == Blink::Open {
            self.next_blink = self.next_blink.max(now + 1200.0);
            return;
        }
        let slow = 1.0 + self.sleepy.p.clamp(0.0, 1.0) * 1.8;
        match self.blink {
            Blink::Open if now >= self.next_blink => {
                self.blink = Blink::Closing;
                self.blink_start = now;
                if !self.double && self.happy.p < 0.3 && self.sleepy.p < 0.3 && rand(0.0, 1.0) < 0.07 {
                    self.wink = if rand(0.0, 1.0) < 0.5 { -1 } else { 1 };
                } else if !self.double {
                    self.wink = 0;
                }
            }
            Blink::Closing => {
                let t = (now - self.blink_start) / (80.0 * slow);
                if t >= 1.0 {
                    self.blink = Blink::Opening;
                    self.blink_start = now;
                    self.openness = 0.0;
                } else {
                    self.openness = 1.0 - t;
                }
            }
            Blink::Opening => {
                let hold = if self.wink != 0 { 260.0 } else { 0.0 };
                let u = (now - self.blink_start - hold) / (120.0 * slow);
                if u < 0.0 {
                    self.openness = 0.0;
                } else if u >= 1.0 {
                    self.blink = Blink::Open;
                    self.openness = 1.0;
                    if !self.double && self.wink == 0 && rand(0.0, 1.0) < 0.22 {
                        self.double = true;
                        self.next_blink = now + 110.0;
                    } else {
                        self.double = false;
                        self.wink = 0;
                        self.next_blink = now + if self.sleepy.p > 0.5 { rand(1800.0, 3400.0) } else { rand(2500.0, 5500.0) };
                    }
                } else {
                    self.openness = u;
                }
            }
            _ => {}
        }
    }

    fn gaze(&self, ex: f32, ey: f32) -> (f32, f32) {
        let cx = ((self.cursor.0 - ex) / 60.0).clamp(-1.0, 1.0);
        let cy = ((self.cursor.1 - ey) / 40.0).clamp(-1.0, 1.0);
        let w = self.glance_w;
        (cx + (self.glance.0 - cx) * w, cy + (self.glance.1 - cy) * w)
    }

    /// Draw the eyes centred at `cx`, `cy` (logical px), `sc` times their size.
    /// `turn`: the face turned about its middle (degrees, clockwise): on a side edge its head is toward that edge, so one eye is
    /// over the other and every look turns with it.
    pub fn paint(&self, p: &Painter, cx: f32, cy: f32, sc: f32, turn: f32, audio: &Audio, t_ms: f32) {
        let s = p.s;
        let wide = self.wide.p.clamp(0.0, 1.3);
        let happy = self.happy.p.clamp(0.0, 1.25);
        let sleepy = self.sleepy.p.clamp(0.0, 1.1);
        let xd = self.xd.p.clamp(0.0, 1.15);
        let bliss = self.bliss.p.clamp(0.0, 1.15);
        let (ax, ay) = (self.squish_dir.0.abs(), self.squish_dir.1.abs());
        let sq = self.squish;
        let stretch_x = (1.0 + sq * (0.25 * ax - 0.12 * ay)).max(0.6);
        let stretch_y = (1.0 + sq * (0.25 * ay - 0.12 * ax)).max(0.6);
        // the whole face moves with the sound
        let kick = audio.kick.p.clamp(0.0, 1.2);
        let chill = audio.chill;
        let xd_w = xd.min(1.0);
        let bang = audio.bang;
        let nod = audio.nod.p.clamp(-0.4, 1.4);
        let bob = -(audio.bands[0] * 1.5 * (1.0 - 0.8 * chill) + kick * 1.7) * audio.dance * sc * (1.0 - bang) + nod * 6.5 * bang * sc;
        let tt = t_ms / 1000.0;
        let shake = ((tt * 8.2).sin() + 0.3 * (tt * 4.6 + 1.3).sin()) * (1.3 + 0.5 * audio.level) * xd_w * sc;
        let sway = audio.sway.sin() * 2.6 * (1.0 - 0.35 * chill) * (1.0 - 0.8 * bang) * audio.dance * sc + shake;
        let tilt = audio.sway.sin() * 0.11 * (1.0 - 0.6 * chill) * audio.dance * (1.0 - xd_w) * (1.0 - bang);
        let kick_sq = 1.0 + kick * 0.16 * audio.dance * (1.0 - bang) - nod * 0.16 * bang;
        let talk = (audio.talk_env - 0.32).clamp(-0.32, 0.68) * audio.talk * 1.8;
        let (talk_y, talk_x) = (1.0 + talk * 0.8, 1.0 - talk * 0.24);
        // breathing
        let cy = cy + (t_ms / 3600.0 * std::f32::consts::TAU).sin() * 0.55;
        let mut before = Matrix3x2::default();
        unsafe { p.dc.GetTransform(&mut before) };
        let before = if turn != 0.0 { Matrix3x2::rotation(turn, cx * s, cy * s) * before } else { before };
        let Ok(b) = brush(p.dc, EYE) else { return };
        // the head: translated to its bob and sway, tilted about its middle
        let head = Matrix3x2::translation(-cx * s, -cy * s) * Matrix3x2::rotation(tilt.to_degrees(), 0.0, 0.0) * Matrix3x2::translation((cx + sway) * s, (cy + bob) * s);
        for side in [-1.0f32, 1.0] {
            let (ex, ey) = (cx + side * 13.5 * sc, cy);
            let (gx, gy) = self.gaze(ex - cx, ey - cy);
            let gx = gx + audio.pan * 0.6 * (audio.level * 2.0).min(1.0);
            let winking = self.wink as f32 == side;
            let eye_happy = if winking { happy.max(1.0 - self.openness) } else { happy };
            let sh = mix(eye_happy, sleepy, wide, xd, bliss);
            let blink = if winking { 1.0 } else { self.openness.max(0.06) };
            let calm = 1.0 - eye_happy.max(sleepy).max(xd).max(bliss).min(1.0);
            let sx = sc * talk_x * (1.0 + (stretch_x - 1.0) * calm);
            let sy = sc * talk_y * kick_sq * blink * (1.0 + (stretch_y - 1.0) * calm);
            let rot = if xd > 0.01 { -side * 90.0 * xd.min(1.0) } else { 0.0 };
            let m = Matrix3x2::rotation(rot, 0.0, 0.0) * Matrix3x2 { M11: sx * s, M12: 0.0, M21: 0.0, M22: sy * s, M31: 0.0, M32: 0.0 } * Matrix3x2::translation((ex + gx * 3.2 * sc) * s, (ey + gy * 1.7 * sc) * s) * head * before;
            unsafe {
                let Ok(g) = p.g.d2d_factory.CreatePathGeometry() else { continue };
                let Ok(sink) = g.Open() else { continue };
                for i in 0..=STEPS {
                    let x = -1.0 + 2.0 * i as f32 / STEPS as f32;
                    let r = (1.0 - x * x).max(0.0).sqrt();
                    let y = sh.c0 + sh.k * x.abs().powf(sh.pw) - sh.top * r;
                    let pt = D2D_POINT_2F { x: x * sh.rx, y };
                    if i == 0 {
                        sink.BeginFigure(pt, D2D1_FIGURE_BEGIN_FILLED);
                    } else {
                        sink.AddLine(pt);
                    }
                }
                for i in (0..=STEPS).rev() {
                    let x = -1.0 + 2.0 * i as f32 / STEPS as f32;
                    let r = (1.0 - x * x).max(0.0).sqrt();
                    sink.AddLine(D2D_POINT_2F { x: x * sh.rx, y: sh.c0 + sh.k * x.abs().powf(sh.pw) + sh.bot * r });
                }
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                let _ = sink.Close();
                p.dc.SetTransform(&m);
                p.dc.FillGeometry(&g, &b, None);
            }
        }
        unsafe { p.dc.SetTransform(&before) };
    }
}

/// The ripples of the bass: thin rings that grow from the eyes across the screen, one for each hit (strength, pan, hue, born at ms).
pub struct Ripples {
    pub rings: Vec<(f32, f32, f32, f32)>,
}

impl Ripples {
    pub fn new() -> Ripples {
        Ripples { rings: Vec::new() }
    }

    pub fn add(&mut self, now: f32, strength: f32, pan: f32) {
        let hue = (now / 1000.0 * 7.0) % 360.0 + 20.0;
        self.rings.push((strength, pan, hue, now));
        if self.rings.len() > 12 {
            self.rings.remove(0);
        }
    }

    /// Draw them round `cx`, `cy` on a screen `size` across; false when there are none left.
    pub fn paint(&mut self, p: &Painter, now: f32, cx: f32, cy: f32, size: f32) -> bool {
        self.rings.retain(|r| (now - r.3) / 900.0 < 1.0);
        for &(strength, _pan, hue, born) in &self.rings {
            let age = ((now - born) / 900.0).max(0.0);
            let ease = 1.0 - (1.0 - age).powf(2.2);
            let reach = 0.12 + 0.62 * strength;
            let radius = (ease * size * reach).max(0.0);
            let alpha = ((0.6 + 0.4 * strength) * (1.0 - age).powf(0.7)).min(1.0);
            let c = hsl(hue, 0.95, 0.76).alpha(alpha);
            if radius > 1.0 {
                p.ring(cx, cy, radius, 0.9, 1.0, 0.0, c, Color(0.0, 0.0, 0.0, 0.0));
            }
        }
        !self.rings.is_empty()
    }
}

/// A colour from hue (degrees), saturation and lightness (0..1).
pub fn hsl(h: f32, s: f32, l: f32) -> Color {
    let h = (h % 360.0 + 360.0) % 360.0 / 360.0;
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let pp = 2.0 * l - q;
    let f = |t: f32| {
        let t = (t % 1.0 + 1.0) % 1.0;
        if t < 1.0 / 6.0 {
            pp + (q - pp) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            pp + (q - pp) * (2.0 / 3.0 - t) * 6.0
        } else {
            pp
        }
    };
    Color(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0), 1.0)
}
