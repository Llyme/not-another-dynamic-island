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
/// Claude's own colour (a session at work)
pub const CLAUDE: Color = Color(0.851, 0.467, 0.341, 1.0);

/// the theme colour (the person's, in the settings): what is lit, chosen, now, and a glow that names no colour of its own
static ACCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x5ac88c);

thread_local! {
    /// the theme colour, for now, in its place (a banner of Claude's: Claude's own colour, not the theme's)
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
        _ => or,
    }
}

/// what a state looks like (a session's, a banner's tone)
pub fn tone_color(tone: &str) -> Color {
    match tone {
        "working" => CLAUDE,
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
        words(&self.g.dwrite, text, size * self.s, weight, (max_w * self.s).max(1.0), Align::Start)
    }

    /// How wide words are, logical px.
    pub fn measure(&self, text: &str, size: f32, weight: Weight, max_w: f32) -> f32 {
        self.words(text, size, weight, max_w).map(|w| w.width / self.s).unwrap_or(0.0)
    }

    /// Words with their left at `x`, centred on `cy`; how wide they came out (logical). Longer than `max_w`: never cut with dots;
    /// they scroll there and back, a while at each end (what is drawn so asks to be drawn again).
    pub fn text(&self, text: &str, x: f32, cy: f32, size: f32, weight: Weight, max_w: f32, c: Color) -> f32 {
        if text.is_empty() || max_w <= 1.0 {
            return 0.0;
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
        unsafe {
            self.dc.PushAxisAlignedClip(&D2D_RECT_F { left: x * self.s, top: (cy - size * 1.2) * self.s, right: (x + max_w) * self.s, bottom: (cy + size * 1.2) * self.s }, D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let _ = crate::paint::draw_words(self.dc, &full, (x - off) * self.s, cy * self.s - full.height / 2.0, c);
        unsafe {
            self.dc.PopAxisAlignedClip();
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
            let Ok(format) = self.g.dwrite.CreateTextFormat(&HSTRING::from("Segoe UI Variable Text"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * self.s, &HSTRING::from("en-us")) else { return 0.0 };
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
            let Ok(format) = self.g.dwrite.CreateTextFormat(&HSTRING::from("Segoe UI Variable Text"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * self.s, &HSTRING::from("en-us")) else { return 0.0 };
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

    /// A picture in a rounded box (cover, program icon); false when there is none.
    pub fn picture(&self, name: &str, r: Rect, radius: f32) -> bool {
        let Some(bmp) = images::get_at(self.dc, self.pc, name, self.t) else { return false };
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
        if let (Some(part), false) = (ring, tile["vessel"].as_bool() == Some(true)) {
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

/// The three bars while paused: still, low.
pub const FLAT: [f32; 3] = [0.18, 0.18, 0.18];

/// How wide the three bars are, `h` tall.
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
