//! The icons: the filled set (SVG paths in a 20 box), the line icons (a 16 box, stroked), and a plugin's own (an SVG file of its, its
//! paths filled), turned into Direct2D geometry once each. A name that is none of them is the dot.

use crate::icons_data::LINE;
use crate::icons_fluent::FILLED;
use crate::paint::{brush, Color};
use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::Result;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

/// icons the frame needs that the old island had not
const EXTRA: &[(&str, &str)] = &[
    // (a cog: teeth round a ring, so it never reads as the sun)
    ("gear", "M12.68 6.55 14.27 6.73 14.27 9.27 12.68 9.45 12.33 10.29 13.33 11.54 11.54 13.33 10.29 12.33 9.45 12.68 9.27 14.27 6.73 14.27 6.55 12.68 5.71 12.33 4.46 13.33 2.67 11.54 3.67 10.29 3.32 9.45 1.73 9.27 1.73 6.73 3.32 6.55 3.67 5.71 2.67 4.46 4.46 2.67 5.71 3.67 6.55 3.32 6.73 1.73 9.27 1.73 9.45 3.32 10.29 3.67 11.54 2.67 13.33 4.46 12.33 5.71zM8 5.9a2.1 2.1 0 1 0 0 4.2 2.1 2.1 0 0 0 0-4.2z"),
    ("bolt", "M9 2.5 4.5 9h3.5l-1 4.5L11.5 7H8z"),
    ("chev-down", "M4.5 6.5 8 10l3.5-3.5"),
    ("volume", "M2.5 6.2h2.3L8 3.6v8.8L4.8 9.8H2.5zM10.6 6a2.6 2.6 0 0 1 0 4M12.2 4.4a5 5 0 0 1 0 7.2"),
    ("volume-x", "M2.5 6.2h2.3L8 3.6v8.8L4.8 9.8H2.5zM10.5 6.5l3 3M13.5 6.5l-3 3"),
    ("wifi", "M2.3 6.3a8 8 0 0 1 11.4 0M4.4 8.5a5 5 0 0 1 7.2 0M6.5 10.6a2 2 0 0 1 3 0M8 12.6h.01"),
    ("chev-up", "M4.5 9.5 8 6l3.5 3.5"),
    ("grid", "M3 3h4v4H3zM9 3h4v4H9zM3 9h4v4H3zM9 9h4v4H9z"),
    ("power", "M8 2.5V8M4.8 4.6a4.6 4.6 0 1 0 6.4 0"),
    ("search", "M7 3a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM10 10l3.5 3.5"),
];

fn line(name: &str) -> Option<&'static str> {
    EXTRA.iter().chain(LINE.iter()).find(|(n, _)| *n == name).map(|(_, d)| *d)
}

struct Icon {
    geometry: ID2D1PathGeometry1,
    /// the box it is drawn in (16 for a line icon)
    box_size: f32,
    filled: bool,
}

thread_local! {
    static CACHE: RefCell<HashMap<String, std::rc::Rc<Icon>>> = RefCell::new(HashMap::new());
    static STROKE: RefCell<Option<ID2D1StrokeStyle>> = const { RefCell::new(None) };
}

/// Forget what was made with the drawing devices (they were lost: the graphics driver was reset).
pub fn forget() {
    CACHE.with(|c| c.borrow_mut().clear());
    STROKE.with(|s| *s.borrow_mut() = None);
}

/// Is there an icon of this name?
#[cfg(test)]
pub fn known(name: &str) -> bool {
    line(name).is_some() || FILLED.iter().any(|(n, _)| *n == name)
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

/// The numbers and commands of an SVG path, in order.
fn tokens(d: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() {
            let start = i;
            let mut seen_dot = c == '.';
            i += 1;
            while i < b.len() {
                let x = b[i] as char;
                if x.is_ascii_digit() {
                    i += 1;
                } else if x == '.' && !seen_dot {
                    seen_dot = true;
                    i += 1;
                } else if (x == 'e' || x == 'E') && i + 1 < b.len() {
                    i += 1;
                    if b[i] == b'-' || b[i] == b'+' {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            out.push(Tok::Num(d[start..i].parse().unwrap_or(0.0)));
        } else {
            i += 1;
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Cmd(char),
    Num(f32),
}

/// An SVG path into a geometry (M L H V C S Q T A Z, both cases).
pub fn path_of(factory: &ID2D1Factory1, d: &str, filled: bool) -> Result<ID2D1PathGeometry1> {
    let toks = tokens(d);
    unsafe {
        let g = factory.CreatePathGeometry()?;
        let sink = g.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let begin = if filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW };
        let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
        // (the last control point, for S and T)
        let (mut lcx, mut lcy) = (0f32, 0f32);
        let mut last = ' ';
        let mut open = false;
        let mut i = 0;
        let mut cmd = 'M';
        let num = |i: &mut usize| -> Option<f32> {
            match toks.get(*i) {
                Some(Tok::Num(n)) => {
                    *i += 1;
                    Some(*n)
                }
                _ => None,
            }
        };
        while i < toks.len() {
            if let Tok::Cmd(c) = toks[i] {
                cmd = c;
                i += 1;
                if c == 'Z' || c == 'z' {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    cx = sx;
                    cy = sy;
                    last = 'Z';
                    continue;
                }
            }
            let rel = cmd.is_ascii_lowercase();
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            match cmd.to_ascii_uppercase() {
                'M' => {
                    let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else { break };
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    cx = ox + x;
                    cy = oy + y;
                    sx = cx;
                    sy = cy;
                    sink.BeginFigure(pt(cx, cy), begin);
                    open = true;
                    // (more pairs after a move are lines)
                    cmd = if rel { 'l' } else { 'L' };
                }
                'L' => {
                    let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else { break };
                    cx = ox + x;
                    cy = oy + y;
                    sink.AddLine(pt(cx, cy));
                }
                'H' => {
                    let Some(x) = num(&mut i) else { break };
                    cx = ox + x;
                    sink.AddLine(pt(cx, cy));
                }
                'V' => {
                    let Some(y) = num(&mut i) else { break };
                    cy = oy + y;
                    sink.AddLine(pt(cx, cy));
                }
                'C' => {
                    let v: Vec<f32> = (0..6).filter_map(|_| num(&mut i)).collect();
                    if v.len() < 6 {
                        break;
                    }
                    let (x1, y1, x2, y2, x, y) = (ox + v[0], oy + v[1], ox + v[2], oy + v[3], ox + v[4], oy + v[5]);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: pt(x1, y1), point2: pt(x2, y2), point3: pt(x, y) });
                    lcx = x2;
                    lcy = y2;
                    cx = x;
                    cy = y;
                }
                'S' => {
                    let v: Vec<f32> = (0..4).filter_map(|_| num(&mut i)).collect();
                    if v.len() < 4 {
                        break;
                    }
                    let (x1, y1) = if matches!(last.to_ascii_uppercase(), 'C' | 'S') { (2.0 * cx - lcx, 2.0 * cy - lcy) } else { (cx, cy) };
                    let (x2, y2, x, y) = (ox + v[0], oy + v[1], ox + v[2], oy + v[3]);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: pt(x1, y1), point2: pt(x2, y2), point3: pt(x, y) });
                    lcx = x2;
                    lcy = y2;
                    cx = x;
                    cy = y;
                }
                'Q' => {
                    let v: Vec<f32> = (0..4).filter_map(|_| num(&mut i)).collect();
                    if v.len() < 4 {
                        break;
                    }
                    let (x1, y1, x, y) = (ox + v[0], oy + v[1], ox + v[2], oy + v[3]);
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(x1, y1), point2: pt(x, y) });
                    lcx = x1;
                    lcy = y1;
                    cx = x;
                    cy = y;
                }
                'T' => {
                    let v: Vec<f32> = (0..2).filter_map(|_| num(&mut i)).collect();
                    if v.len() < 2 {
                        break;
                    }
                    let (x1, y1) = if matches!(last.to_ascii_uppercase(), 'Q' | 'T') { (2.0 * cx - lcx, 2.0 * cy - lcy) } else { (cx, cy) };
                    let (x, y) = (ox + v[0], oy + v[1]);
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(x1, y1), point2: pt(x, y) });
                    lcx = x1;
                    lcy = y1;
                    cx = x;
                    cy = y;
                }
                'A' => {
                    let v: Vec<f32> = (0..7).filter_map(|_| num(&mut i)).collect();
                    if v.len() < 7 {
                        break;
                    }
                    let (x, y) = (ox + v[5], oy + v[6]);
                    if (x - cx).abs() > 1e-4 || (y - cy).abs() > 1e-4 {
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: pt(x, y),
                            size: D2D_SIZE_F { width: v[0].abs(), height: v[1].abs() },
                            rotationAngle: v[2],
                            sweepDirection: if v[4] != 0.0 { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                            arcSize: if v[3] != 0.0 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                        });
                    }
                    cx = x;
                    cy = y;
                }
                _ => {
                    i += 1;
                }
            }
            last = cmd;
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        Ok(g)
    }
}

fn icon(factory: &ID2D1Factory1, name: &str) -> Result<std::rc::Rc<Icon>> {
    if let Some(i) = CACHE.with(|c| c.borrow().get(name).cloned()) {
        return Ok(i);
    }
    // (a plugin's own SVG; the filled set; the line icons for what it has not)
    let own = name.strip_prefix("file:").filter(|p| p.to_ascii_lowercase().ends_with(".svg")).and_then(svg_paths);
    let made = if let Some((b, d)) = own {
        Icon { geometry: path_of(factory, &d, true)?, box_size: b, filled: true }
    } else if let Some((_, d)) = FILLED.iter().find(|(n, _)| *n == name) {
        Icon { geometry: path_of(factory, d, true)?, box_size: 20.0, filled: true }
    } else {
        let d = line(name).or_else(|| line("dot")).unwrap_or("M8 8h.01");
        Icon { geometry: path_of(factory, d, false)?, box_size: 16.0, filled: false }
    };
    let rc = std::rc::Rc::new(made);
    CACHE.with(|c| c.borrow_mut().insert(name.to_string(), rc.clone()));
    Ok(rc)
}

/// A plugin's own icon: the paths of its SVG file (all of them, filled, in its own colour) and the size of its box (from its viewBox).
/// Read as data: only the numbers of its viewBox and its paths' `d`.
fn svg_paths(path: &str) -> Option<(f32, String)> {
    let f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(f, 512 * 1024), &mut s).ok()?;
    let attr = |text: &str, name: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find(name) {
            let before_ok = i == 0 || rest.as_bytes()[i - 1].is_ascii_whitespace();
            rest = &rest[i + name.len()..];
            if !before_ok {
                continue;
            }
            let q = rest.chars().next();
            if let Some(q @ ('"' | '\'')) = q {
                if let Some(end) = rest[1..].find(q) {
                    out.push(rest[1..1 + end].to_string());
                }
            }
        }
        out
    };
    let vb: Vec<f32> = attr(&s, "viewBox=").first()?.split([' ', ',']).filter_map(|n| n.parse().ok()).collect();
    let b = if vb.len() == 4 { vb[2].max(vb[3]) } else { return None };
    let d = attr(&s, "d=").join(" ");
    (b > 0.0 && !d.is_empty()).then_some((b, d))
}

fn stroke(factory: &ID2D1Factory1) -> Result<ID2D1StrokeStyle> {
    if let Some(s) = STROKE.with(|s| s.borrow().clone()) {
        return Ok(s);
    }
    let props = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: D2D1_CAP_STYLE_ROUND,
        endCap: D2D1_CAP_STYLE_ROUND,
        dashCap: D2D1_CAP_STYLE_ROUND,
        lineJoin: D2D1_LINE_JOIN_ROUND,
        miterLimit: 10.0,
        dashStyle: D2D1_DASH_STYLE_SOLID,
        dashOffset: 0.0,
    };
    let s: ID2D1StrokeStyle = unsafe { (&**factory as &ID2D1Factory).CreateStrokeStyle(&props, None)? };
    STROKE.with(|x| *x.borrow_mut() = Some(s.clone()));
    Ok(s)
}

/// Draw the icon `name`, `size` px square, its top-left at `x`, `y`.
pub fn draw(dc: &ID2D1DeviceContext, factory: &ID2D1Factory1, name: &str, x: f32, y: f32, size: f32, c: Color) -> Result<()> {
    let i = icon(factory, name)?;
    let b = brush(dc, c)?;
    unsafe {
        let mut before = Matrix3x2::default();
        dc.GetTransform(&mut before);
        let k = size / i.box_size;
        let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y } * before;
        dc.SetTransform(&m);
        if i.filled {
            dc.FillGeometry(&i.geometry, &b, None);
        } else {
            dc.DrawGeometry(&i.geometry, &b, 1.5, &stroke(factory)?);
        }
        dc.SetTransform(&before);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_read_into_its_commands() {
        assert_eq!(tokens("M6 2.5v3M10-2.5.5.5"), vec![Tok::Cmd('M'), Tok::Num(6.0), Tok::Num(2.5), Tok::Cmd('v'), Tok::Num(3.0), Tok::Cmd('M'), Tok::Num(10.0), Tok::Num(-2.5), Tok::Num(0.5), Tok::Num(0.5)]);
        assert_eq!(tokens("a1 1 0 0 1-7 0z").len(), 9);
        assert!(known("music") && known("gear") && !known("nope"));
    }
}
