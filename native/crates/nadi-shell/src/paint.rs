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

thread_local! {
    /// the text formats, made once each (by size in tenths of a px, weight, alignment)
    static FORMATS: std::cell::RefCell<std::collections::HashMap<(u32, u8, u8), IDWriteTextFormat>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

fn format(dw: &IDWriteFactory, size: f32, weight: Weight, align: Align) -> Result<IDWriteTextFormat> {
    let key = ((size * 10.0).round() as u32, weight as u8, align as u8);
    if let Some(f) = FORMATS.with(|f| f.borrow().get(&key).cloned()) {
        return Ok(f);
    }
    unsafe {
        let format = dw.CreateTextFormat(&HSTRING::from("Segoe UI Variable Text"), None, weight.dw(), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, &HSTRING::from("en-us"))?;
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
