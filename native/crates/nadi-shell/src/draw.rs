//! What the pieces are drawn with: a painter over the device context, in logical px (it scales by the monitor), with the frame's few
//! looks: words, icons, rounded boxes, rings, bars, the vessel, pictures. Flat colours, no outlines.

use crate::gfx::Gfx;
use crate::icons;
use crate::images;
use crate::paint::{brush, words, Align, Color, Weight, Words};
use crate::shape::Rect;
use nadi_core::pc::Pc;
use std::sync::Arc;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

pub const FACE: Color = Color(0.0, 0.0, 0.0, 1.0);
pub const TEXT: Color = Color(0.949, 0.949, 0.961, 1.0);
pub const DIM: Color = Color(0.604, 0.604, 0.643, 1.0);
pub const FAINT: Color = Color(1.0, 1.0, 1.0, 0.08);
pub const SOFT: Color = Color(1.0, 1.0, 1.0, 0.14);
pub const GREEN: Color = Color(0.353, 0.784, 0.549, 1.0);
pub const AMBER: Color = Color(0.910, 0.690, 0.290, 1.0);
pub const RED: Color = Color(0.910, 0.478, 0.478, 1.0);
/// a warm orange (something at work)
pub const ORANGE: Color = Color(0.851, 0.467, 0.341, 1.0);

/// the theme colour (the person's, in the settings): what is lit, chosen, now, and a glow that names no colour of its own
static ACCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x5ac88c);
static ACCENT2: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0xe8b04a);

thread_local! {
    /// the theme colour, for now, in its place (a banner of a plugin with a colour of its own: that, not the theme's)
    static ACCENT_NOW: std::cell::Cell<Option<Color>> = const { std::cell::Cell::new(None) };
}

/// The theme colour, for what is drawn until it is set back (none: the theme's own); what it was.
pub fn set_accent_now(c: Option<Color>) -> Option<Color> {
    ACCENT_NOW.with(|a| a.replace(c))
}

/// words too long for their room scroll there and back: so long at each end, so fast (logical px a second)
const MARQUEE_WAIT: f32 = 1.6;
const MARQUEE_SPEED: f32 = 28.0;

thread_local! {
    static MARQUEE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Did anything drawn since last asked scroll (words too long for their room)? Then it is to be drawn again soon. Asking clears it.
/// Something drawn moves on its own (a wave, a spinning ring): the widget is drawn again at the next frame.
pub fn keep_moving() {
    MARQUEE.with(|m| m.set(true));
}

/// how wide a flow's chevrons are, beside words of `size`, and the gap after them
fn flow_w(size: f32) -> f32 {
    (size * 0.52).round()
}
const FLOW_GAP: f32 = 4.0;

/// Words that begin with an arrow up or down ("↑ 85 MB/s"): which way, and the rest.
fn flow_of(text: &str) -> Option<(bool, &str)> {
    let up = if let Some(r) = text.strip_prefix('↑') { Some((true, r)) } else { text.strip_prefix('↓').map(|r| (false, r)) };
    up.map(|(u, r)| (u, r.trim_start()))
}

pub fn take_marquee() -> bool {
    MARQUEE.with(|m| m.replace(false))
}

pub fn accent() -> Color {
    if let Some(c) = ACCENT_NOW.with(|a| a.get()) {
        return c;
    }
    let v = ACCENT.load(std::sync::atomic::Ordering::Relaxed);
    Color(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0)
}

/// The theme's second colour: what waits for you.
pub fn accent2() -> Color {
    let v = ACCENT2.load(std::sync::atomic::Ordering::Relaxed);
    Color(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0)
}

/// The theme's second colour, from "#rrggbb".
pub fn set_accent2(hex: &str) {
    if let Some(v) = hex.strip_prefix('#').filter(|h| h.len() == 6).and_then(|h| u32::from_str_radix(h, 16).ok()) {
        ACCENT2.store(v, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The theme colour, from "#rrggbb" (anything else: as it was).
pub fn set_accent(hex: &str) {
    if let Some(v) = hex.strip_prefix('#').filter(|h| h.len() == 6).and_then(|h| u32::from_str_radix(h, 16).ok()) {
        ACCENT.store(v, std::sync::atomic::Ordering::Relaxed);
    }
}

/// a colour a plugin names (`#rrggbb`), else `or`
pub fn color_of(v: &serde_json::Value, or: Color) -> Color {
    match v.as_str() {
        Some(s) if s.starts_with('#') && s.len() == 7 => Color::hex(s),
        Some("theme") => accent(),
        Some("theme2") => accent2(),
        _ => or,
    }
}

/// what a state looks like (a session's, a banner's tone)
pub fn tone_color(tone: &str) -> Color {
    match tone {
        "working" => ORANGE,
        "waiting" | "permission" | "reply" | "low" | "saver" | "hot" => AMBER,
        "finished" | "charging" | "full" | "ok" => GREEN,
        "stopped" | "critical" | "no" => RED,
        _ => DIM,
    }
}

pub struct Painter<'a> {
    pub g: &'a Gfx,
    pub dc: &'a ID2D1DeviceContext,
    /// physical px per logical px
    pub s: f32,
    pub pc: &'a Arc<Pc>,
    /// seconds, for what moves
    pub t: f32,
    /// the three bars of the sound, now (0..1 each; all 0 when nothing is heard)
    pub viz: [f32; 3],
}

impl<'a> Painter<'a> {
    pub fn words(&self, text: &str, size: f32, weight: Weight, max_w: f32) -> Result<Words> {
        crate::paint::SCALE.with(|s| s.set(self.s));
        words(&self.g.dwrite, text, size * self.s, weight, (max_w * self.s).max(1.0), Align::Start)
    }

    /// How wide words are, logical px.
    pub fn measure(&self, text: &str, size: f32, weight: Weight, max_w: f32) -> f32 {
        if let Some((_, rest)) = flow_of(text) {
            let fw = flow_w(size) + FLOW_GAP;
            return fw + self.words(rest, size, weight, (max_w - fw).max(1.0)).map(|w| w.width / self.s).unwrap_or(0.0);
        }
        self.words(text, size, weight, max_w).map(|w| w.width / self.s).unwrap_or(0.0)
    }

    /// Something going one way, as a pictogram that moves: chevrons rising (up: sent, written) or falling (down: received, read)
    /// through a small box, fading in at one end and out at the other, as tall as words of `size`; `x` its left, `cy` its middle;
    /// how wide. Words that begin with "↑" or "↓" are drawn with it in place of the arrow.
    pub fn flow(&self, x: f32, cy: f32, up: bool, size: f32, c: Color) -> f32 {
        let w = flow_w(size);
        let h = size * 1.05;
        let top = cy - h / 2.0;
        let gap = h * 0.36;
        let rise = w * 0.28;
        let ph = (self.t * 1.6).fract() * gap;
        for i in -1..5 {
            let off = i as f32 * gap + ph;
            let along = if up { h - off } else { off };
            if !(0.0..=h).contains(&along) {
                continue;
            }
            let k = (along / h * std::f32::consts::PI).sin().clamp(0.0, 1.0);
            let y = top + along;
            let pts = if up { [(x, y + rise), (x + w / 2.0, y - rise), (x + w, y + rise)] } else { [(x, y - rise), (x + w / 2.0, y + rise), (x + w, y - rise)] };
            self.line(&pts, (size * 0.14).max(1.4), c.alpha(c.3 * k));
        }
        keep_moving();
        w
    }

    /// Words with their left at `x`, centred on `cy`; how wide they came out (logical). Longer than `max_w`: never cut with dots;
    /// they scroll there and back, a while at each end (what is drawn so asks to be drawn again).
    pub fn text(&self, text: &str, x: f32, cy: f32, size: f32, weight: Weight, max_w: f32, c: Color) -> f32 {
        if text.is_empty() || max_w <= 1.0 {
            return 0.0;
        }
        // (going up or down: its moving chevrons in place of the arrow)
        if let Some((up, rest)) = flow_of(text) {
            let fw = self.flow(x, cy, up, size, c) + FLOW_GAP;
            return fw + self.text(rest, x + fw, cy, size, weight, max_w - fw, c);
        }
        let Ok(full) = self.words(text, size, weight, 100_000.0) else { return 0.0 };
        let fw = full.width / self.s;
        if fw <= max_w + 0.5 {
            let _ = crate::paint::draw_words(self.dc, &full, x * self.s, cy * self.s - full.height / 2.0, c);
            return fw;
        }
        // (there and back: a while at its start, along to its end, a while there, back to its start; eased at each turn)
        let over = fw - max_w;
        let travel = over / MARQUEE_SPEED;
        let period = 2.0 * (MARQUEE_WAIT + travel);
        let ph = self.t % period;
        let ease = |k: f32| k * k * (3.0 - 2.0 * k);
        let off = if ph < MARQUEE_WAIT {
            0.0
        } else if ph < MARQUEE_WAIT + travel {
            over * ease((ph - MARQUEE_WAIT) / travel)
        } else if ph < 2.0 * MARQUEE_WAIT + travel {
            over
        } else {
            over * (1.0 - ease((ph - 2.0 * MARQUEE_WAIT - travel) / travel))
        };
        // (each end fades softly where words go on past it: the start once they have moved off it, the end while there is more)
        let fade = (size * 1.6).min(max_w * 0.25);
        let a0 = (off / fade).clamp(0.0, 1.0);
        let a1 = ((over - off) / fade).clamp(0.0, 1.0);
        let s = self.s;
        let bounds = D2D_RECT_F { left: x * s, top: (cy - size * 1.2) * s, right: (x + max_w) * s, bottom: (cy + size * 1.2) * s };
        let mask = unsafe {
            let stop = |position: f32, a: f32| D2D1_GRADIENT_STOP { position, color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a } };
            let k = fade / max_w;
            let stops = [stop(0.0, 1.0 - a0), stop(k, 1.0), stop(1.0 - k, 1.0), stop(1.0, 1.0 - a1)];
            self.dc
                .CreateGradientStopCollection(&stops, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT)
                .ok()
                .and_then(|coll| windows::core::Interface::cast::<ID2D1GradientStopCollection>(&coll).ok())
                .and_then(|coll| {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: D2D_POINT_2F { x: bounds.left, y: 0.0 }, endPoint: D2D_POINT_2F { x: bounds.right, y: 0.0 } };
                    self.dc.CreateLinearGradientBrush(&props, None, &coll).ok()
                })
        };
        unsafe {
            match &mask {
                Some(br) => {
                    let params = D2D1_LAYER_PARAMETERS1 {
                        contentBounds: bounds,
                        geometricMask: std::mem::ManuallyDrop::new(None),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(),
                        opacity: 1.0,
                        opacityBrush: std::mem::ManuallyDrop::new(Some(br.clone().into())),
                        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                    };
                    self.dc.PushLayer(&params, None);
                    let _ = crate::paint::draw_words(self.dc, &full, (x - off) * s, cy * s - full.height / 2.0, c);
                    self.dc.PopLayer();
                    let _ = std::mem::ManuallyDrop::into_inner(params.opacityBrush);
                }
                None => {
                    self.dc.PushAxisAlignedClip(&bounds, D2D1_ANTIALIAS_MODE_ALIASED);
                    let _ = crate::paint::draw_words(self.dc, &full, (x - off) * s, cy * s - full.height / 2.0, c);
                    self.dc.PopAxisAlignedClip();
                }
            }
        }
        MARQUEE.with(|m| m.set(true));
        max_w
    }

    /// Words, right-aligned to `right`.
    pub fn text_end(&self, text: &str, right: f32, cy: f32, size: f32, weight: Weight, max_w: f32, c: Color) -> f32 {
        let w = self.measure(text, size, weight, max_w);
        self.text(text, right - w, cy, size, weight, max_w, c)
    }

    /// Words, wrapped in a box `max_w` wide, at most `lines` lines; how tall (logical).
    pub fn paragraph(&self, text: &str, x: f32, y: f32, size: f32, max_w: f32, lines: usize, c: Color, draw: bool) -> f32 {
        use windows::core::HSTRING;
        use windows::Win32::Graphics::DirectWrite::*;
        unsafe {
            let Ok(format) = ({ let (face, coll) = crate::paint::face(&self.g.dwrite, size * self.s, false, self.s); self.g.dwrite.CreateTextFormat(&HSTRING::from(face.as_str()), coll.as_ref(), DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * self.s, &HSTRING::from("en-us")) }) else { return 0.0 };
            let line_h = size * 1.35 * self.s;
            let _ = format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_h, line_h * 0.8);
            if let Ok(sign) = self.g.dwrite.CreateEllipsisTrimmingSign(&format) {
                let _ = format.SetTrimming(&DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_WORD, delimiter: 0, delimiterCount: 0 }, &sign);
            }
            let wide: Vec<u16> = text.encode_utf16().collect();
            let max_h = line_h * lines as f32 + 1.0;
            let Ok(layout) = self.g.dwrite.CreateTextLayout(&wide, &format, max_w * self.s, max_h) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            if draw {
                if let Ok(b) = brush(self.dc, c) {
                    self.dc.PushAxisAlignedClip(&D2D_RECT_F { left: x * self.s, top: y * self.s, right: (x + max_w) * self.s, bottom: y * self.s + max_h }, D2D1_ANTIALIAS_MODE_ALIASED);
                    self.dc.DrawTextLayout(D2D_POINT_2F { x: x * self.s, y: y * self.s }, &layout, &b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
                    self.dc.PopAxisAlignedClip();
                }
            }
            (m.height.min(max_h)) / self.s
        }
    }

    /// A paragraph whose first `lit` characters (UTF-16) are in `lit_c`, the rest in `c`, with a caret after its end (`caret`); how
    /// tall. (Words read as they are heard, played back; a line still being said.)
    pub fn paragraph_lit(&self, text: &str, x: f32, y: f32, size: f32, max_w: f32, lit: usize, c: Color, lit_c: Color, caret: Option<Color>) -> f32 {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::Graphics::DirectWrite::*;
        unsafe {
            let Ok(format) = ({ let (face, coll) = crate::paint::face(&self.g.dwrite, size * self.s, false, self.s); self.g.dwrite.CreateTextFormat(&HSTRING::from(face.as_str()), coll.as_ref(), DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * self.s, &HSTRING::from("en-us")) }) else { return 0.0 };
            let line_h = size * 1.35 * self.s;
            let _ = format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_h, line_h * 0.8);
            let wide: Vec<u16> = text.encode_utf16().collect();
            let Ok(layout) = self.g.dwrite.CreateTextLayout(&wide, &format, max_w * self.s, 100_000.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            if let (Ok(b), Ok(lb)) = (brush(self.dc, c), brush(self.dc, lit_c)) {
                let n = lit.min(wide.len());
                if n > 0 {
                    if let Ok(u) = lb.cast::<windows::core::IUnknown>() {
                        let _ = layout.SetDrawingEffect(&u, DWRITE_TEXT_RANGE { startPosition: 0, length: n as u32 });
                    }
                }
                self.dc.DrawTextLayout(D2D_POINT_2F { x: x * self.s, y: y * self.s }, &layout, &b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
            }
            if let Some(cc) = caret {
                let (mut px, mut py) = (0.0f32, 0.0f32);
                let mut hm = DWRITE_HIT_TEST_METRICS::default();
                let at = wide.len().saturating_sub(1) as u32;
                if !wide.is_empty() && layout.HitTestTextPosition(at, windows::Win32::Foundation::BOOL::from(true), &mut px, &mut py, &mut hm).is_ok() {
                    let hh = hm.height / self.s;
                    self.round(Rect::new(x + px / self.s + 2.0, y + py / self.s + hh * 0.16, 2.0, hh * 0.68), 1.0, cc);
                } else {
                    self.round(Rect::new(x, y + 2.0, 2.0, size), 1.0, cc);
                }
            }
            m.height / self.s
        }
    }

    /// A graph in `r`: `vals` (each 0..1, the oldest first) as a line, with a faint fill under it.
    pub fn spark(&self, vals: &[f32], r: Rect, c: Color) {
        let n = vals.len();
        if n < 2 {
            self.round(Rect::new(r.x, r.bottom() - 1.5, r.w, 1.5), 0.75, c.alpha(0.5));
            return;
        }
        let step = r.w / (n - 1) as f32;
        let y = |v: f32| r.bottom() - 1.0 - (r.h - 2.0) * v.clamp(0.0, 1.0);
        // (its fill: a thin column under each point)
        for (i, v) in vals.iter().enumerate() {
            let top = y(*v);
            let x = r.x + i as f32 * step - step / 2.0;
            let (x0, x1) = (x.max(r.x), (x + step).min(r.right()));
            if x1 > x0 {
                self.round(Rect::new(x0, top, x1 - x0, r.bottom() - top), 0.0, c.alpha(0.16));
            }
        }
        let pts: Vec<(f32, f32)> = vals.iter().enumerate().map(|(i, v)| (r.x + i as f32 * step, y(*v))).collect();
        self.line(&pts, 1.6, c);
    }

    pub fn icon(&self, name: &str, x: f32, y: f32, size: f32, c: Color) {
        let _ = icons::draw(self.dc, &self.g.d2d_factory, name, x * self.s, y * self.s, size * self.s, c);
    }

    /// An icon filled from the left as far as `part` (0..1) in `c`, the rest in `track`: a shape that is its own meter.
    pub fn icon_fill(&self, name: &str, x: f32, y: f32, size: f32, part: f32, c: Color, track: Color) {
        self.icon(name, x, y, size, track);
        let s = self.s;
        unsafe {
            self.dc.PushAxisAlignedClip(&D2D_RECT_F { left: x * s, top: y * s, right: (x + size * part.clamp(0.0, 1.0)) * s, bottom: (y + size) * s }, D2D1_ANTIALIAS_MODE_ALIASED);
            self.icon(name, x, y, size, c);
            self.dc.PopAxisAlignedClip();
        }
    }

    pub fn round(&self, r: Rect, radius: f32, c: Color) {
        if c.3 <= 0.0 {
            return;
        }
        let _ = crate::paint::fill_round(self.dc, self.px(r), radius * self.s, c);
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, c: Color) {
        let _ = crate::paint::fill_circle(self.dc, cx * self.s, cy * self.s, radius * self.s, c);
    }

    pub fn px(&self, r: Rect) -> Rect {
        Rect::new(r.x * self.s, r.y * self.s, r.w * self.s, r.h * self.s)
    }

    /// A box filled `c` at its top, fading smoothly to nothing at its foot.
    pub fn fade_down(&self, r: Rect, c: Color) {
        let q = self.px(r);
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 } },
                D2D1_GRADIENT_STOP { position: 0.55, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 * 0.35 } },
                D2D1_GRADIENT_STOP { position: 1.0, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: 0.0 } },
            ];
            let Ok(coll) = self.dc.CreateGradientStopCollection(&stops, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT) else { return };
            let Ok(coll) = windows::core::Interface::cast::<ID2D1GradientStopCollection>(&coll) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: D2D_POINT_2F { x: q.x, y: q.y }, endPoint: D2D_POINT_2F { x: q.x, y: q.y + q.h.max(1.0) } };
            let Ok(br) = self.dc.CreateLinearGradientBrush(&props, None, &coll) else { return };
            self.dc.FillRectangle(&D2D_RECT_F { left: q.x, top: q.y, right: q.x + q.w, bottom: q.y + q.h }, &br);
        }
    }

    /// A ring round `cx`, `cy`: its track, and `part` of it lit from the top, clockwise (from `start`, 0..1, for one that turns).
    pub fn ring(&self, cx: f32, cy: f32, radius: f32, width: f32, part: f32, start: f32, c: Color, track: Color) {
        let s = self.s;
        unsafe {
            if let Ok(b) = brush(self.dc, track) {
                self.dc.DrawEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx * s, y: cy * s }, radiusX: radius * s, radiusY: radius * s }, &b, width * s, None);
            }
        }
        let part = part.clamp(0.0, 1.0);
        if part <= 0.001 {
            return;
        }
        if part >= 0.999 {
            unsafe {
                if let Ok(b) = brush(self.dc, c) {
                    self.dc.DrawEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx * s, y: cy * s }, radiusX: radius * s, radiusY: radius * s }, &b, width * s, None);
                }
            }
            return;
        }
        let at = |f: f32| {
            let a = (f * std::f32::consts::TAU) - std::f32::consts::FRAC_PI_2;
            D2D_POINT_2F { x: (cx + radius * a.cos()) * s, y: (cy + radius * a.sin()) * s }
        };
        unsafe {
            let Ok(g) = self.g.d2d_factory.CreatePathGeometry() else { return };
            let Ok(sink) = g.Open() else { return };
            sink.BeginFigure(at(start), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: at(start + part),
                size: D2D_SIZE_F { width: radius * s, height: radius * s },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if part > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            let props = D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, miterLimit: 10.0, ..Default::default() };
            let style: Option<ID2D1StrokeStyle> = (&*self.g.d2d_factory as &ID2D1Factory).CreateStrokeStyle(&props, None).ok();
            if let Ok(b) = brush(self.dc, c) {
                self.dc.DrawGeometry(&g, &b, width * s, style.as_ref());
            }
        }
    }

    /// A line through points (logical px), `width` thick, round at its joins and ends.
    pub fn line(&self, pts: &[(f32, f32)], width: f32, c: Color) {
        if pts.len() < 2 {
            return;
        }
        let s = self.s;
        unsafe {
            let Ok(g) = self.g.d2d_factory.CreatePathGeometry() else { return };
            let Ok(sink) = g.Open() else { return };
            sink.BeginFigure(D2D_POINT_2F { x: pts[0].0 * s, y: pts[0].1 * s }, D2D1_FIGURE_BEGIN_HOLLOW);
            for (x, y) in &pts[1..] {
                sink.AddLine(D2D_POINT_2F { x: x * s, y: y * s });
            }
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            let props = D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, miterLimit: 10.0, ..Default::default() };
            let style: Option<ID2D1StrokeStyle> = (&*self.g.d2d_factory as &ID2D1Factory).CreateStrokeStyle(&props, None).ok();
            if let Ok(b) = brush(self.dc, c) {
                self.dc.DrawGeometry(&g, &b, width * s, style.as_ref());
            }
        }
    }

    /// A bar: its track, and `part` of it filled, from the left.
    pub fn bar(&self, r: Rect, part: f32, c: Color) {
        let rad = r.h / 2.0;
        self.round(r, rad, FAINT);
        let w = (r.w * part.clamp(0.0, 1.0)).max(if part > 0.0 { r.h } else { 0.0 });
        if w > 0.0 {
            self.round(Rect::new(r.x, r.y, w, r.h), rad, c);
        }
    }

    /// The vessel of a battery, lying down: a shell, a nub, and the liquid filled to `level` from the left (a bolt while it charges).
    pub fn vessel(&self, r: Rect, level: f32, charging: bool, c: Color) {
        let nub = (r.h * 0.18).max(2.0);
        let shell = Rect::new(r.x, r.y, r.w - nub - 1.0, r.h);
        let rad = r.h * 0.3;
        self.round(shell, rad, SOFT);
        self.round(Rect::new(shell.right() + 1.0, r.y + r.h * 0.3, nub, r.h * 0.4), nub / 2.0, SOFT);
        let inset = (r.h * 0.14).max(1.5);
        let inner = Rect::new(shell.x + inset, shell.y + inset, shell.w - 2.0 * inset, shell.h - 2.0 * inset);
        let w = inner.w * level.clamp(0.0, 1.0);
        if w > 0.5 {
            self.round(Rect::new(inner.x, inner.y, w.max(inner.h * 0.6), inner.h), (rad - inset).max(1.0), c);
        }
        if charging {
            let z = inner.h * 1.1;
            self.icon("bolt", inner.x + inner.w / 2.0 - z / 2.0, inner.y + inner.h / 2.0 - z / 2.0, z, TEXT);
        }
    }

    /// The battery's round tank: dark, filled to `level` with a waving liquid (no rim; while it charges, a rim of its colour and
    /// bubbles rising in it; full, still to the brim with a check in it).
    /// A bin (the Recycle Bin), drawn in a box of 24: its body narrowing to its foot, filling from there as far as `part` in `c`;
    /// its lid, with its handle, over it, lifted and tipped back about its left end by `lid` (0..1: something dragged over it).
    pub fn bin(&self, r: Rect, part: f32, lid: f32, c: Color) {
        let z = r.w.min(r.h);
        let (x, y) = ((r.x + (r.w - z) / 2.0) * self.s, (r.y + (r.h - z) / 2.0) * self.s);
        let size = z * self.s;
        let f = &self.g.d2d_factory;
        let part = part.clamp(0.0, 1.0);
        // (what it holds: the body, from its foot up so far, its sides narrowing as the body does)
        if part > 0.005 {
            let (top, foot) = (8.6, 20.4);
            let yf = foot - (foot - top) * part;
            let at = |yy: f32| ((6.5 + (yy - 8.0) / 13.0) + 0.6, (17.5 - (yy - 8.0) / 13.0) - 0.6);
            let (l0, r0) = at(yf);
            let (l1, r1) = at(foot);
            let d = format!("M{l0:.2} {yf:.2}L{r0:.2} {yf:.2}L{r1:.2} {foot:.2}L{l1:.2} {foot:.2}Z");
            let _ = crate::icons::draw_path(self.dc, f, &d, 24.0, x, y, size, Some(c.alpha(0.92)), None, (0.0, 0.0, 0.0));
        }
        let _ = crate::icons::draw_path(self.dc, f, "M6.5 8h11l-1 13h-9z", 24.0, x, y, size, None, Some((TEXT, 1.7)), (0.0, 0.0, 0.0));
        let _ = crate::icons::draw_path(self.dc, f, "M4.5 6h15M10 6V4.5h4V6", 24.0, x, y - lid * 1.6 * self.s, size, None, Some((TEXT, 1.7)), (-28.0 * lid, 4.5, 6.0));
    }

    pub fn tank(&self, r: Rect, level: f32, charging: bool, full: bool, c: Color) {
        let z = r.w.min(r.h);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rr = z / 2.0;
        self.circle(cx, cy, rr, crate::paint::Color(0.106, 0.106, 0.125, 1.0));
        let inner = if charging { rr - 1.6 } else { rr };
        let b = Rect::new(cx - inner, cy - inner, inner * 2.0, inner * 2.0);
        if full {
            self.circle(cx, cy, inner, c);
            let k = z * 0.6;
            self.icon("check", cx - k / 2.0, cy - k / 2.0, k, crate::paint::Color(0.016, 0.075, 0.043, 1.0));
        } else {
            let level = level.clamp(0.0, 1.0);
            // (never empty to the eye: a thin layer at the foot)
            let shown = if level > 0.0 { 0.08 + level * 0.92 } else { 0.0 };
            self.wave_fill(b, inner, shown, self.t * 0.45, (z * 0.06).max(0.8), c);
            if charging {
                // (bubbles: each rising from the foot, fading as it goes)
                for i in 0..3 {
                    let ph = (self.t * 0.55 + i as f32 / 3.0).fract();
                    let bx = cx + (i as f32 - 1.0) * inner * 0.45;
                    let by = cy + inner * 0.85 - ph * inner * 1.2 * shown.max(0.3);
                    let a = (1.0 - ph) * (ph * 4.0).min(1.0) * 0.7;
                    self.circle(bx, by, (z * 0.05).max(0.9), crate::paint::Color(1.0, 1.0, 1.0, a));
                }
            }
            keep_moving();
        }
        if charging {
            self.ring(cx, cy, rr - 0.7, 1.4, 1.0, 0.0, c, c);
        }
    }

    /// Draw only where a shape (an SVG path, logical px) is: what `f` draws is cut to it.
    pub fn within_path(&self, d: &str, f: impl FnOnce()) {
        let Ok(g) = crate::icons::path_of(&self.g.d2d_factory, d, true) else { return };
        unsafe {
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: std::mem::ManuallyDrop::new(Some(g.into())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: windows::Foundation::Numerics::Matrix3x2 { M11: self.s, M12: 0.0, M21: 0.0, M22: self.s, M31: 0.0, M32: 0.0 },
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None);
            f();
            self.dc.PopLayer();
            let _ = std::mem::ManuallyDrop::into_inner(params.geometricMask);
        }
    }

    /// The front wave of a waving fill (as `wave_fill` draws it), as a path: what is under its surface.
    pub fn wave_path(r: Rect, part: f32, phase: f32, amp: f32) -> String {
        let top = r.y + r.h * (1.0 - part.clamp(0.0, 1.0));
        let mut d = format!("M {} {}", r.x, r.bottom());
        let n = 24;
        for i in 0..=n {
            let fx = i as f32 / n as f32;
            let y = top + amp * ((fx + phase) * std::f32::consts::TAU).sin();
            d.push_str(&format!(" L {} {}", r.x + r.w * fx, y));
        }
        d.push_str(&format!(" L {} {} Z", r.right(), r.bottom()));
        d
    }

    /// A picture in a rounded box (cover, program icon); false when there is none.
    pub fn picture(&self, name: &str, r: Rect, radius: f32) -> bool {
        let Some(bmp) = images::get_at(self.dc, self.pc, name, self.t) else { return false };
        // (a moving picture: drawn again at the next frame)
        if images::animated(name) {
            keep_moving();
        }
        let p = self.px(r);
        unsafe {
            let size = bmp.GetSize();
            // (filled and cut to the box, kept in proportion)
            let k = (p.w / size.width).max(p.h / size.height);
            let (dw, dh) = (size.width * k, size.height * k);
            let dest = D2D_RECT_F { left: p.x + (p.w - dw) / 2.0, top: p.y + (p.h - dh) / 2.0, right: p.x + (p.w + dw) / 2.0, bottom: p.y + (p.h + dh) / 2.0 };
            let Ok(clip) = self.g.d2d_factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: p.d2d(), radiusX: radius * self.s, radiusY: radius * self.s }) else { return false };
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: p.d2d(),
                geometricMask: std::mem::ManuallyDrop::new(Some(clip.into())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None);
            self.dc.DrawBitmap(&bmp, Some(&dest), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
            self.dc.PopLayer();
            let _ = std::mem::ManuallyDrop::into_inner(params.geometricMask);
        }
        true
    }

    /// A box with round corners filled from its bottom up to `part` of it, its top a wave (two, the one behind paler, rolling
    /// against each other: `phase` in turns, `amp` how high, logical px).
    pub fn wave_fill(&self, r: Rect, radius: f32, part: f32, phase: f32, amp: f32, c: Color) {
        let part = part.clamp(0.0, 1.0);
        if part <= 0.0 {
            return;
        }
        let p = self.px(r);
        let top = r.y + r.h * (1.0 - part);
        let wave = |ph: f32, a: f32, k: f32| -> String {
            let mut d = format!("M {} {}", r.x, r.bottom());
            let n = 24;
            for i in 0..=n {
                let fx = i as f32 / n as f32;
                let y = top + a * ((fx * k + ph) * std::f32::consts::TAU).sin();
                d.push_str(&format!(" L {} {}", r.x + r.w * fx, y));
            }
            d.push_str(&format!(" L {} {} Z", r.right(), r.bottom()));
            d
        };
        unsafe {
            let Ok(clip) = self.g.d2d_factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: p.d2d(), radiusX: radius * self.s, radiusY: radius * self.s }) else { return };
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: p.d2d(),
                geometricMask: std::mem::ManuallyDrop::new(Some(clip.into())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None);
            // (full: no wave at its top)
            if part >= 0.995 {
                self.round(r, 0.0, c);
            } else {
                self.path(&wave(-phase * 0.7 + 0.3, amp * 0.8, 1.3), c.alpha(c.3 * 0.55), None);
                self.path(&wave(phase, amp, 1.0), c, None);
            }
            self.dc.PopLayer();
            let _ = std::mem::ManuallyDrop::into_inner(params.geometricMask);
        }
    }

    /// A line through values (0..1 each, oldest first) across a box, softly filled under it, its newest point a dot.
    pub fn line_graph(&self, vals: &[f32], r: Rect, c: Color, fill: bool, dot: bool) {
        if vals.len() < 2 {
            return;
        }
        let n = (vals.len() - 1) as f32;
        let pts: Vec<(f32, f32)> = vals.iter().enumerate().map(|(i, v)| (r.x + r.w * i as f32 / n, r.bottom() - 1.0 - (r.h - 2.0) * v.clamp(0.0, 1.0))).collect();
        if fill {
            let mut d = format!("M {} {}", r.x, r.bottom());
            for (x, y) in &pts {
                d.push_str(&format!(" L {x} {y}"));
            }
            d.push_str(&format!(" L {} {} Z", r.right(), r.bottom()));
            self.path(&d, c.alpha(c.3 * 0.18), None);
        }
        self.line(&pts, 1.8, c);
        if dot {
            if let Some((x, y)) = pts.last() {
                self.circle(*x, *y, 2.6, c);
            }
        }
    }

    /// A picture's size, in its own pixels (none: not here yet).
    pub fn picture_size(&self, name: &str) -> Option<(f32, f32)> {
        let bmp = images::get_at(self.dc, self.pc, name, self.t)?;
        let size = unsafe { bmp.GetPixelSize() };
        (size.width > 0 && size.height > 0).then_some((size.width as f32, size.height as f32))
    }

    /// A picture looked at closely, in a box with round corners: whole at `zoom` 1 (fit inside it), `zoom` times as big otherwise,
    /// the point of it at (`ox`, `oy`) (0..1 across and down it) in the box's middle.
    pub fn picture_view(&self, name: &str, r: Rect, radius: f32, zoom: f32, ox: f32, oy: f32) -> bool {
        let Some(bmp) = images::get_at(self.dc, self.pc, name, self.t) else { return false };
        let p = self.px(r);
        unsafe {
            let px = bmp.GetPixelSize();
            let (iw, ih) = (px.width.max(1) as f32, px.height.max(1) as f32);
            let k = (p.w / iw).min(p.h / ih) * zoom;
            let (dw, dh) = (iw * k, ih * k);
            let (left, top) = (p.x + p.w / 2.0 - ox * dw, p.y + p.h / 2.0 - oy * dh);
            let dest = D2D_RECT_F { left, top, right: left + dw, bottom: top + dh };
            let Ok(clip) = self.g.d2d_factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: p.d2d(), radiusX: radius * self.s, radiusY: radius * self.s }) else { return false };
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: p.d2d(),
                geometricMask: std::mem::ManuallyDrop::new(Some(clip.into())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None);
            // (shrunk: smoothed well; blown up: smoothly too, but quicker)
            let mode = if k < 1.0 { D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC } else { D2D1_INTERPOLATION_MODE_LINEAR };
            self.dc.DrawBitmap(&bmp, Some(&dest), 1.0, mode, None, None);
            self.dc.PopLayer();
            let _ = std::mem::ManuallyDrop::into_inner(params.geometricMask);
        }
        true
    }

    /// A box with only its top corners round (a page's top).
    pub fn round_top(&self, r: Rect, radius: f32, c: Color) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        unsafe {
            self.dc.PushAxisAlignedClip(&self.px(r).d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        self.round(Rect::new(r.x, r.y, r.w, r.h + radius), radius, c);
        unsafe {
            self.dc.PopAxisAlignedClip();
        }
    }

    /// A box with round corners, its outline only.
    pub fn outline(&self, r: Rect, radius: f32, width: f32, c: Color) {
        let p = self.px(r);
        unsafe {
            let Ok(br) = brush(self.dc, c) else { return };
            let radius = crate::paint::rounded(radius * self.s, p.w, p.h);
            let rr = D2D1_ROUNDED_RECT { rect: p.d2d(), radiusX: radius, radiusY: radius };
            self.dc.DrawRoundedRectangle(&rr, &br, width * self.s, None);
        }
    }

    /// A soft light: an ellipse (`rx` by `ry` about `cx`, `cy`) of `c` in its middle, fading to nothing at its edge.
    pub fn radial(&self, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
        let s = self.s;
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 } },
                D2D1_GRADIENT_STOP { position: 0.45, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 * 0.45 } },
                D2D1_GRADIENT_STOP { position: 1.0, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: 0.0 } },
            ];
            let Ok(coll) = self.dc.CreateGradientStopCollection(&stops, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT) else { return };
            let Ok(coll) = windows::core::Interface::cast::<ID2D1GradientStopCollection>(&coll) else { return };
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: D2D_POINT_2F { x: cx * s, y: cy * s }, gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 }, radiusX: rx * s, radiusY: ry * s };
            let Ok(br) = self.dc.CreateRadialGradientBrush(&props, None, &coll) else { return };
            self.dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx * s, y: cy * s }, radiusX: rx * s, radiusY: ry * s }, &br);
        }
    }

    /// A shape given as an SVG path (logical px), filled, and its outline stroked when asked.
    pub fn path(&self, d: &str, fill: Color, stroke: Option<(Color, f32)>) {
        let Ok(g) = crate::icons::path_of(&self.g.d2d_factory, d, true) else { return };
        unsafe {
            let mut base = windows::Foundation::Numerics::Matrix3x2::default();
            self.dc.GetTransform(&mut base);
            self.dc.SetTransform(&(windows::Foundation::Numerics::Matrix3x2 { M11: self.s, M12: 0.0, M21: 0.0, M22: self.s, M31: 0.0, M32: 0.0 } * base));
            if let Ok(b) = brush(self.dc, fill) {
                self.dc.FillGeometry(&g, &b, None);
            }
            if let Some((c, w)) = stroke {
                if let Ok(b) = brush(self.dc, c) {
                    self.dc.DrawGeometry(&g, &b, w, None);
                }
            }
            self.dc.SetTransform(&base);
        }
    }

    /// A box with round corners, its edge dashed.
    pub fn dashed(&self, r: Rect, radius: f32, width: f32, c: Color) {
        let p = self.px(r);
        unsafe {
            let props = D2D1_STROKE_STYLE_PROPERTIES1 {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 1.0,
                dashStyle: D2D1_DASH_STYLE_CUSTOM,
                dashOffset: 0.0,
                transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
            };
            let Ok(style) = self.g.d2d_factory.CreateStrokeStyle(&props, Some(&[3.0, 3.0])) else { return };
            let Ok(br) = brush(self.dc, c) else { return };
            let radius = crate::paint::rounded(radius * self.s, p.w, p.h);
            let rr = D2D1_ROUNDED_RECT { rect: p.d2d(), radiusX: radius, radiusY: radius };
            self.dc.DrawRoundedRectangle(&rr, &br, width * self.s, &style);
        }
    }

    /// A picture whole, `h` tall and as wide as its proportion makes it (at most three times as wide), from `x`, centred on `cy`;
    /// how wide it went (none here yet: 0).
    pub fn picture_fit(&self, name: &str, x: f32, cy: f32, h: f32, draw: bool) -> f32 {
        let Some(bmp) = images::get_at(self.dc, self.pc, name, self.t) else { return 0.0 };
        let size = unsafe { bmp.GetSize() };
        if size.height <= 0.0 {
            return 0.0;
        }
        let w = (h * size.width / size.height).clamp(h * 0.3, h * 3.0);
        if draw {
            let p = self.px(Rect::new(x, cy - h / 2.0, w, h));
            unsafe {
                self.dc.DrawBitmap(&bmp, Some(&p.d2d()), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
            }
        }
        w
    }

    /// A tile: a rounded square with a picture, or an icon (or a letter) on its colour, a ring round it, a bolt; `size` square.
    pub fn tile(&self, tile: &serde_json::Value, x: f32, y: f32, size: f32, fallback_icon: &str) {
        let c = color_of(&tile["color"], accent());
        let r = Rect::new(x, y, size, size);
        let image = tile["image"].as_str().unwrap_or("");
        let ring = tile["ring"].as_f64();
        let pad = if ring.is_some() { size * 0.12 } else { 0.0 };
        let inner = Rect::new(x + pad, y + pad, size - 2.0 * pad, size - 2.0 * pad);
        let drew = !image.is_empty() && self.picture(image, inner, inner.w * 0.28);
        if drew {
        } else if tile["tank"].as_bool() == Some(true) {
            let lv = ring.unwrap_or(0.0) as f32;
            self.tank(inner, lv, tile["bolt"].as_bool() == Some(true), lv >= 0.995, c);
        } else if tile["vessel"].as_bool() == Some(true) {
            self.round(inner, inner.w * 0.28, c.alpha(0.18));
            let lv = Rect::new(inner.x + inner.w * 0.14, inner.y + inner.h * 0.3, inner.w * 0.72, inner.h * 0.4);
            self.vessel(lv, ring.unwrap_or(0.0) as f32, tile["bolt"].as_bool() == Some(true), c);
        } else {
            self.round(inner, inner.w * 0.28, c.alpha(0.2));
            let text = tile["text"].as_str().unwrap_or("");
            let icon = tile["icon"].as_str().unwrap_or(fallback_icon);
            if !text.is_empty() && text.chars().count() <= 4 {
                let fs = if text.chars().count() <= 1 { inner.h * 0.48 } else { inner.h * 0.32 };
                let w = self.measure(text, fs, Weight::Semibold, inner.w);
                self.text(text, inner.x + inner.w / 2.0 - w / 2.0, inner.y + inner.h / 2.0, fs, Weight::Semibold, inner.w, c);
            } else {
                let z = inner.w * 0.56;
                self.icon(icon, inner.x + (inner.w - z) / 2.0, inner.y + (inner.h - z) / 2.0, z, c);
            }
        }
        if let (Some(part), false) = (ring, tile["vessel"].as_bool() == Some(true) || tile["tank"].as_bool() == Some(true)) {
            let rc = color_of(&tile["ring_color"], c);
            self.ring(x + size / 2.0, y + size / 2.0, size / 2.0 - size * 0.05, size * 0.07, part as f32, 0.0, rc, FAINT);
        }
        let _ = r;
    }

    /// Three bars that dance with the sound, `h` tall, from `x`, centred on `cy`; `parts` 0..1 each.
    /// The three bars of the sound, from `x`, centred on `cy`, at most `h` tall: as tall as their bands, and lifted by the bass
    /// (the middle one most, a little hop). Paused (`FLAT`): still, low. Its width.
    pub fn viz(&self, x: f32, cy: f32, h: f32, parts: [f32; 3], c: Color) -> f32 {
        let lift = if parts == FLAT { 0.0 } else { LIFT.with(|l| l.get()) };
        self.bars(x, cy, h, parts, lift, c)
    }

    /// The three bars, lifted so much (0..1) — what every set of three bars is drawn with.
    pub fn bars(&self, x: f32, cy: f32, h: f32, parts: [f32; 3], lift: f32, c: Color) -> f32 {
        let w = (h * 0.24).max(2.0);
        let gap = (h * 0.16).max(1.5);
        for (i, p) in parts.iter().enumerate() {
            if *p <= 0.01 {
                continue;
            }
            let bh = (h * p).max(w);
            let up = lift.clamp(0.0, 1.0) * h * 0.3 * [0.8, 1.0, 0.8][i];
            self.round(Rect::new(x + i as f32 * (w + gap), cy - bh / 2.0 - up, w, bh), w / 2.0, c);
        }
        bars_w(h)
    }
}

/// Three bars (and their lift) for something whose loudness alone is known (0..1): as tall as it is loud, quiet sound still standing
/// up, wavering with the time `t`.
pub fn loud_bars(loud: f32, t: f32) -> ([f32; 3], f32) {
    let v = (loud.sqrt() * 1.6).min(1.0);
    let parts = std::array::from_fn(|i| (v * (0.6 + 0.4 * (t * (7.0 + i as f32 * 2.3) + i as f32 * 1.9).sin())).max(0.25));
    (parts, v * v * (0.55 + 0.45 * (t * 4.1).sin().max(0.0)))
}

/// how loud a thing is now, over its volume's fill: light, plain against the theme's colour
pub const LIVE: Color = Color(1.0, 1.0, 1.0, 0.62);

/// How far along its volume a peak (0..1) reads: as the ear hears it, so quiet sound still shows.
pub fn live_level(peak: f32) -> f32 {
    peak.clamp(0.0, 1.0).sqrt()
}

/// The three bars while paused: still, low.
pub const FLAT: [f32; 3] = [0.18, 0.18, 0.18];

/// How wide the three bars are, `h` tall.
/// A grip, centred on (`cx`, `cy`): a pill that springs out of a dot (`show`, a spring, a little past 1 as it comes), wider and
/// brighter under the mouse (`over`), the theme colour and glowing while held (`held`), stretched as it is dragged (`stretch`, px);
/// `a` its opacity.
pub fn grip(p: &Painter, cx: f32, cy: f32, g: [f32; 4], a: f32) {
    let [show, over, held, stretch] = g;
    if show <= 0.01 || a <= 0.01 {
        return;
    }
    let over = over.clamp(0.0, 1.0);
    let held = held.clamp(0.0, 1.0);
    let w = (4.0 + 32.0 * show + 14.0 * over * (1.0 - held) + 6.0 * held + stretch.max(0.0)).max(2.0);
    let h = 4.0 + over * (1.0 - held) + 2.0 * held;
    let mix = |x: Color, y: Color, k: f32| Color(x.0 + (y.0 - x.0) * k, x.1 + (y.1 - x.1) * k, x.2 + (y.2 - x.2) * k, x.3 + (y.3 - x.3) * k);
    let rest = DIM.alpha(0.85);
    let lit = Color(0.9, 0.9, 0.92, 1.0);
    let c = mix(mix(rest, lit, over), accent(), held);
    let fade = (show * 3.0).clamp(0.0, 1.0) * a;
    // (held: a soft glow round it)
    if held > 0.01 {
        let ac = accent();
        for (grow, k) in [(8.0, 0.07), (4.0, 0.14)] {
            p.round(Rect::new(cx - w / 2.0 - grow, cy - h / 2.0 - grow, w + 2.0 * grow, h + 2.0 * grow), h / 2.0 + grow, ac.alpha(k * held * fade));
        }
    }
    p.round(Rect::new(cx - w / 2.0, cy - h / 2.0, w, h), h / 2.0, c.alpha(c.3 * fade));
}

pub fn bars_w(h: f32) -> f32 {
    3.0 * (h * 0.24).max(2.0) + 2.0 * (h * 0.16).max(1.5)
}

thread_local! {
    /// how far the bass lifts the bars of the sound now (0..1)
    static LIFT: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// How far the bass lifts the bars of the sound now.
pub fn set_lift(v: f32) {
    LIFT.with(|l| l.set(v));
}

/// Draw `f` see-through (`a` 0..1) as one: a layer over all of it.
pub fn faded(dc: &ID2D1DeviceContext, a: f32, f: impl FnOnce()) {
    if a >= 0.999 {
        f();
        return;
    }
    let params = D2D1_LAYER_PARAMETERS1 {
        contentBounds: D2D_RECT_F { left: -1.0e6, top: -1.0e6, right: 1.0e6, bottom: 1.0e6 },
        geometricMask: std::mem::ManuallyDrop::new(None),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(),
        opacity: a.max(0.0),
        opacityBrush: std::mem::ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    unsafe {
        dc.PushLayer(&params, None);
        f();
        dc.PopLayer();
    }
}
