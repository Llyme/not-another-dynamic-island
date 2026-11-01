//! Drawing helpers over a Direct2D device context: colours, shapes filled, words laid out (DirectWrite, trimmed with an ellipsis).

use crate::shape::{Rect, Shape};
use windows::core::{Result, HSTRING};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    /// `#rrggbb` (else white)
    pub fn hex(s: &str) -> Color {
        let h = s.trim_start_matches('#');
        let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f32 / 255.0;
        if h.len() == 6 {
            Color(p(0), p(2), p(4), 1.0)
        } else {
            Color(1.0, 1.0, 1.0, 1.0)
        }
    }
    pub fn alpha(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }
    pub fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}


pub fn brush(dc: &ID2D1DeviceContext, c: Color) -> Result<ID2D1SolidColorBrush> {
    unsafe { dc.CreateSolidColorBrush(&c.d2d(), None) }
}

pub fn fill_shape(dc: &ID2D1DeviceContext, shape: &Shape, c: Color) -> Result<()> {
    let b = brush(dc, c)?;
    unsafe { dc.FillGeometry(&shape.geometry, &b, None) };
    Ok(())
}

/// A shape filled from its top in `c`, fading to nothing by `to_y` (its own pixels down).
pub fn fill_shape_fade(dc: &ID2D1DeviceContext, shape: &Shape, c: Color, to_y: f32) -> Result<()> {
    unsafe {
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 } },
            D2D1_GRADIENT_STOP { position: 1.0, color: D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: 0.0 } },
        ];
        let coll = dc.CreateGradientStopCollection(&stops, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT)?;
        let coll: ID2D1GradientStopCollection = windows::core::Interface::cast(&coll)?;
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 }, endPoint: D2D_POINT_2F { x: 0.0, y: to_y.max(1.0) } };
        let br = dc.CreateLinearGradientBrush(&props, None, &coll)?;
        dc.FillGeometry(&shape.geometry, &br, None);
    }
    Ok(())
}

pub fn fill_round(dc: &ID2D1DeviceContext, r: Rect, radius: f32, c: Color) -> Result<()> {
    let b = brush(dc, c)?;
    unsafe { dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, &b) };
    Ok(())
}

pub fn fill_circle(dc: &ID2D1DeviceContext, cx: f32, cy: f32, radius: f32, c: Color) -> Result<()> {
    let b = brush(dc, c)?;
    unsafe { dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: radius, radiusY: radius }, &b) };
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub enum Weight {
    Light,
    Regular,
    Semibold,
    Bold,
}

impl Weight {
    fn dw(self) -> DWRITE_FONT_WEIGHT {
        match self {
            Weight::Light => DWRITE_FONT_WEIGHT_LIGHT,
            Weight::Regular => DWRITE_FONT_WEIGHT_NORMAL,
            Weight::Semibold => DWRITE_FONT_WEIGHT_SEMI_BOLD,
            Weight::Bold => DWRITE_FONT_WEIGHT_BOLD,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub enum Align {
    Start,
    Center,
    End,
}

/// Words, laid out in a box: one line, trimmed with an ellipsis when they do not fit; their size, for placing what comes after them.
pub struct Words {
    pub layout: IDWriteTextLayout,
    pub width: f32,
    pub height: f32,
}

// ---------------------------------------------------------------------------------------------------------- its typefaces

/// NADI's own typefaces, inside it (not installed on the PC): Inter for what is read, Outfit for what is big and seen at a glance (the
/// clock, titles, a drive's letter, big numbers). Both under the SIL Open Font License (assets/fonts/OFL-*.txt).
static FONT_FILES: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/inter-400.ttf"),
    include_bytes!("../assets/fonts/inter-500.ttf"),
    include_bytes!("../assets/fonts/inter-600.ttf"),
    include_bytes!("../assets/fonts/inter-700.ttf"),
    include_bytes!("../assets/fonts/outfit-500.ttf"),
    include_bytes!("../assets/fonts/outfit-600.ttf"),
    include_bytes!("../assets/fonts/outfit-700.ttf"),
];

/// the words' typeface for what is read, and for what is big
const TEXT_FACE: &str = "Inter";
const DISPLAY_FACE: &str = "Outfit";
/// Windows' own, when NADI's could not be loaded
const SYSTEM_FACE: &str = "Segoe UI Variable Text";

/// A collection of NADI's own typefaces, made from the files inside it.
unsafe fn own_fonts(dw: &IDWriteFactory) -> Result<IDWriteFontCollection> {
    use windows::core::Interface;
    let f5: IDWriteFactory5 = dw.cast()?;
    let loader = f5.CreateInMemoryFontFileLoader()?;
    f5.RegisterFontFileLoader(&loader)?;
    let builder: IDWriteFontSetBuilder1 = f5.CreateFontSetBuilder()?.cast()?;
    for data in FONT_FILES {
        let file = loader.CreateInMemoryFontFileReference(&f5, data.as_ptr() as *const _, data.len() as u32, None)?;
        builder.AddFontFile(&file)?;
    }
    let set = builder.CreateFontSet()?;
    let coll = f5.CreateFontCollectionFromFontSet(&set)?;
    let coll: IDWriteFontCollection = coll.cast()?;
    // (both there, by the start of their name (the display face's files name their family "Outfit Thin"); or none of it)
    let mut names = Vec::new();
    for i in 0..coll.GetFontFamilyCount() {
        if let Ok(ns) = coll.GetFontFamily(i).and_then(|f| f.GetFamilyNames()) {
            let mut buf = [0u16; 128];
            let _ = ns.GetString(0, &mut buf);
            names.push(String::from_utf16_lossy(&buf[..buf.iter().position(|c| *c == 0).unwrap_or(0)]));
        }
    }
    let find = |start: &str| names.iter().find(|n| n.starts_with(start)).cloned();
    let (Some(text), Some(display)) = (find(TEXT_FACE), find(DISPLAY_FACE)) else {
        return Err(windows::core::Error::from(windows::Win32::Foundation::E_FAIL));
    };
    FACES.with(|f| *f.borrow_mut() = (text, display));
    Ok(coll)
}

thread_local! {
    /// the two typefaces' family names, as their files give them (for what is read, for what is big)
    static FACES: std::cell::RefCell<(String, String)> = std::cell::RefCell::new((TEXT_FACE.to_string(), DISPLAY_FACE.to_string()));
    /// the scale words are drawn at now (physical px a logical px): sizes come in physical px
    pub static SCALE: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
    /// NADI's typefaces (made once; none: they could not be, and Windows' own is used)
    static FONTS: std::cell::RefCell<Option<Option<IDWriteFontCollection>>> = const { std::cell::RefCell::new(None) };
    /// numbers all as wide (so a number that changes does not jiggle): the typography every line is set with
    static TABULAR: std::cell::RefCell<Option<IDWriteTypography>> = const { std::cell::RefCell::new(None) };
}

fn fonts(dw: &IDWriteFactory) -> Option<IDWriteFontCollection> {
    FONTS.with(|f| {
        if let Some(c) = &*f.borrow() {
            return c.clone();
        }
        let c = unsafe { own_fonts(dw) }.ok();
        *f.borrow_mut() = Some(c.clone());
        c
    })
}

/// Which typeface, and from where, for words of a size and weight: big and strong ones (14 px and more, semibold or bold), Outfit;
/// the rest, Inter.
pub fn face(dw: &IDWriteFactory, size_px: f32, weight_bold: bool, scale: f32) -> (String, Option<IDWriteFontCollection>) {
    match fonts(dw) {
        Some(c) => (FACES.with(|f| if weight_bold && size_px / scale.max(0.5) >= 14.0 { f.borrow().1.clone() } else { f.borrow().0.clone() }), Some(c)),
        None => (SYSTEM_FACE.to_string(), None),
    }
}

fn tabular(dw: &IDWriteFactory) -> Option<IDWriteTypography> {
    TABULAR.with(|t| {
        if let Some(x) = &*t.borrow() {
            return Some(x.clone());
        }
        let x = unsafe {
            let typo = dw.CreateTypography().ok()?;
            typo.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 }).ok()?;
            typo
        };
        *t.borrow_mut() = Some(x.clone());
        Some(x)
    })
}

thread_local! {
    /// the text formats, made once each (by size in tenths of a px, weight, alignment)
    static FORMATS: std::cell::RefCell<std::collections::HashMap<(u32, u8, u8), IDWriteTextFormat>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

fn format(dw: &IDWriteFactory, size: f32, weight: Weight, align: Align) -> Result<IDWriteTextFormat> {
    let key = ((size * 10.0).round() as u32 * 16 + (SCALE.with(|s| s.get()) * 4.0).round() as u32, weight as u8, align as u8);
    if let Some(f) = FORMATS.with(|f| f.borrow().get(&key).cloned()) {
        return Ok(f);
    }
    unsafe {
        let (name, coll) = face(dw, size, matches!(weight, Weight::Semibold | Weight::Bold), SCALE.with(|s| s.get()));
        let format = dw.CreateTextFormat(&HSTRING::from(name.as_str()), coll.as_ref(), weight.dw(), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, &HSTRING::from("en-us"))?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        format.SetTextAlignment(match align {
            Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
        })?;
        let sign = dw.CreateEllipsisTrimmingSign(&format)?;
        format.SetTrimming(&DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 }, &sign)?;
        FORMATS.with(|f| f.borrow_mut().insert(key, format.clone()));
        Ok(format)
    }
}

pub fn words(dw: &IDWriteFactory, text: &str, size: f32, weight: Weight, max_w: f32, align: Align) -> Result<Words> {
    unsafe {
        let format = format(dw, size, weight, align)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = dw.CreateTextLayout(&wide, &format, max_w.max(1.0), size * 2.0)?;
        // (mostly a number (a time, a size, a speed): its digits all as wide, so it does not jiggle as it changes; names keep theirs)
        let (digits, alnum) = text.chars().fold((0, 0), |(d, a), c| (d + c.is_ascii_digit() as usize, a + c.is_alphanumeric() as usize));
        if digits * 10 >= alnum.max(1) * 3 {
            if let Some(t) = tabular(dw) {
                let _ = layout.SetTypography(&t, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 });
            }
        }
        let mut m = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut m)?;
        Ok(Words { layout, width: m.width, height: m.height })
    }
}

/// Draw words with their top-left at `x`, `y` (the layout's own box; an aligned one is placed in its box).
pub fn draw_words(dc: &ID2D1DeviceContext, w: &Words, x: f32, y: f32, c: Color) -> Result<()> {
    let b = brush(dc, c)?;
    unsafe { dc.DrawTextLayout(D2D_POINT_2F { x, y }, &w.layout, &b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT) };
    Ok(())
}
