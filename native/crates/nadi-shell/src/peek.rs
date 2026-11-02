//! A widget at rest: its peek, drawn in its piece's body. Along a horizontal edge (and in a corner) it is a row — what leads it (a picture,
//! an icon, a ring, the vessel, bars), the words, the bars of the sound, a button; along a vertical edge it is a column, the words short.
//! Shrunk to a dot, only what leads it.

use crate::draw::*;
use crate::paint::Weight;
use crate::shape::Rect;
use serde_json::Value;

const PAD: f32 = 12.0;
const LEAD: f32 = 18.0;
const GAP: f32 = 8.0;
const TEXT_SIZE: f32 = 12.5;
const MAX_TEXT: f32 = 170.0;
/// a clock's room at each end (its words are all it has)
const CLOCK_PAD: f32 = 16.0;
/// a large clock: the time over the whole date
const BIG_TIME: f32 = 15.0;
const BIG_DATE: f32 = 11.0;

/// The whole date, as a large clock says it: "Wednesday, October 28".
fn long_date() -> String {
    use chrono::Datelike;
    let now = chrono::Local::now();
    format!("{}, {} {}", now.format("%A"), now.format("%B"), now.day())
}

/// A large clock along a horizontal edge: two rows (not drawn bigger).
fn two_rows(peek: &Value, horizontal: bool) -> bool {
    horizontal && peek["clock"].is_object() && peek["image"].is_null() && peek["scale"].as_f64().unwrap_or(1.0) > 1.01
}

fn two_rows_w(p: &Painter, peek: &Value) -> f32 {
    let (t, _) = clock_text(peek["clock"]["h24"].as_bool() == Some(true));
    let tw = p.measure(&t, BIG_TIME, Weight::Semibold, 240.0);
    let dw = if peek["clock"]["date"].as_bool() == Some(true) { p.measure(&long_date(), BIG_DATE, Weight::Regular, 240.0) } else { 0.0 };
    CLOCK_PAD * 2.0 + tw.max(dw) + next_w(p, peek)
}

/// A large clock's next event, on its right (a bar of its colour, its title, how soon): how wide (none: 0).
fn next_w(p: &Painter, peek: &Value) -> f32 {
    let n = &peek["clock"]["next"];
    if !n.is_object() {
        return 0.0;
    }
    let s = |k: &str| n[k].as_str().unwrap_or("").to_string();
    18.0 + 4.0 + 10.0 + p.measure(&s("title"), 13.0, Weight::Semibold, 160.0).max(p.measure(&s("when"), 11.0, Weight::Regular, 160.0))
}

fn draw_two_rows(p: &Painter, peek: &Value, body: Rect) {
    let (t, _) = clock_text(peek["clock"]["h24"].as_bool() == Some(true));
    // (what is next, on the right: the clock in what is left)
    let nw = next_w(p, peek);
    if nw > 0.0 {
        let n = &peek["clock"]["next"];
        let s = |k: &str| n[k].as_str().unwrap_or("").to_string();
        let c = crate::draw::color_of(&n["color"], crate::draw::accent());
        let x0 = body.right() - CLOCK_PAD - nw + 18.0;
        let cy = body.y + body.h / 2.0;
        p.round(Rect::new(x0, cy - 16.0, 4.0, 32.0), 2.0, c);
        p.text(&s("title"), x0 + 14.0, cy - 7.0, 13.0, Weight::Semibold, 160.0, TEXT);
        p.text(&s("when"), x0 + 14.0, cy + 9.0, 11.0, Weight::Regular, 160.0, c);
    }
    let body = Rect::new(body.x, body.y, body.w - nw, body.h);
    let cx = body.x + body.w / 2.0;
    let cy = body.y + body.h / 2.0;
    let tw = p.measure(&t, BIG_TIME, Weight::Semibold, 240.0);
    if peek["clock"]["date"].as_bool() != Some(true) {
        p.text(&t, cx - tw / 2.0, cy, BIG_TIME, Weight::Semibold, 240.0, TEXT);
        return;
    }
    let d = long_date();
    let dw = p.measure(&d, BIG_DATE, Weight::Regular, 240.0);
    p.text(&t, cx - tw / 2.0, cy - 7.0, BIG_TIME, Weight::Semibold, 240.0, TEXT);
    p.text(&d, cx - dw / 2.0, cy + 9.0, BIG_DATE, Weight::Regular, 240.0, DIM);
}

/// The local time, as a clock says it.
pub fn clock_text(h24: bool) -> (String, String) {
    let now = chrono::Local::now();
    use chrono::Timelike;
    let (h, m) = (now.hour(), now.minute());
    let time = if h24 {
        format!("{h:02}:{m:02}")
    } else {
        format!("{}:{m:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
    };
    let date = now.format("%a").to_string();
    (time, date)
}

/// what the words of a peek are (a clock's are the time)
fn words_of(peek: &Value) -> (String, String) {
    if peek["clock"].is_object() {
        let (t, d) = clock_text(peek["clock"]["h24"].as_bool() == Some(true));
        let date = if peek["clock"]["date"].as_bool() == Some(true) { d } else { String::new() };
        return (t, date);
    }
    (peek["text"].as_str().unwrap_or("").to_string(), String::new())
}

/// how wide what leads it is
fn lead_w(peek: &Value) -> f32 {
    if bare(peek) {
        0.0
    } else if peek["bars"].is_array() {
        44.0
    } else if peek["vessel"].is_object() {
        26.0
    } else if peek["tank"].is_object() {
        20.0
    } else if peek["bin"].is_object() {
        22.0
    } else if let Some(n) = peek["devs"]["items"].as_array().map(|a| a.len().clamp(1, 3) as f32) {
        n * DEV_RING + (n - 1.0) * 6.0
    } else if let Some(n) = peek["drives"]["items"].as_array().map(|a| a.len().clamp(1, DRIVES_MAX) as f32) {
        n * DEV_RING + (n - 1.0) * 5.0
    } else if peek["roll"].is_object() {
        roll_w(peek)
    } else if peek["page"].is_object() {
        PAGE_W
    } else if peek["clock"].is_object() && peek["image"].is_null() {
        0.0
    } else {
        lead_size(peek)
    }
}

/// a device's ring at rest
const DEV_RING: f32 = 24.0;
/// a calendar's little page, at rest: how wide
const PAGE_W: f32 = 24.0;
/// how many drives' rings at rest at most
const DRIVES_MAX: usize = 8;

/// Its words alone: its icon is none ("").
fn bare(peek: &Value) -> bool {
    peek["icon"] == "" && peek["image"].is_null() && peek["live"].is_null() && peek["ring"].is_null() && peek["vessel"].is_null() && peek["tank"].is_null() && peek["bin"].is_null() && peek["bars"].is_null() && peek["pad"].is_null() && peek["roll"].is_null() && peek["signal"].is_null() && peek["devs"].is_null() && peek["drives"].is_null() && peek["page"].is_null()
}



/// how long the desktops' row may be at most (logical px): many of them, each smaller
const DESKS_MAX: f32 = 520.0;


/// A desktop's minimap in a box: its ground, its monitors (more than one), its windows where they are (the one in front lit: in the
/// theme's colour on the desktop shown).
pub fn draw_map(p: &Painter, m: &Value, screens: &[Value], r: Rect, radius: f32, on: bool, pictures: bool) {
    let at = |v: &Value| -> Rect {
        let f = |i: usize| v[i].as_f64().unwrap_or(0.0) as f32;
        Rect::new(r.x + f(0) * r.w, r.y + f(1) * r.h, f(2) * r.w, f(3) * r.h)
    };
    p.round(r, radius, crate::paint::Color(0.106, 0.106, 0.133, 1.0));
    let wall = m["wall"].as_str().unwrap_or("");
    // (its monitors, each its wallpaper; a wallpaper that cannot be shown: the desktop's own colour, dim)
    if screens.len() > 1 || pictures {
        let tint = crate::paint::Color(0.137, 0.137, 0.169, 1.0);
        for sc in screens {
            let sr = at(sc);
            let sr = Rect::new(sr.x + 1.0, sr.y + 1.0, (sr.w - 2.0).max(1.0), (sr.h - 2.0).max(1.0));
            if !(pictures && !wall.is_empty() && p.picture(wall, sr, (radius - 1.0).max(1.0))) {
                p.round(sr, (radius - 1.0).max(1.0), if pictures { tint } else { crate::paint::Color(0.137, 0.137, 0.169, 1.0) });
            }
        }
    }
    for w in m["wins"].as_array().into_iter().flatten() {
        let wr = at(&w["r"]);
        if wr.w < 1.0 || wr.h < 1.0 {
            continue;
        }
        let front = w["front"] == true;
        let c = if front && on { crate::draw::accent() } else if front { crate::paint::Color(0.557, 0.557, 0.588, 1.0) } else { crate::paint::Color(0.235, 0.235, 0.275, 1.0) };
        if pictures {
            p.round(wr, 3.0, crate::paint::Color(0.165, 0.165, 0.196, 0.94));
            if front {
                p.round(Rect::new(wr.x - 1.0, wr.y - 1.0, wr.w + 2.0, wr.h + 2.0), 4.0, c);
                p.round(wr, 3.0, crate::paint::Color(0.165, 0.165, 0.196, 1.0));
            }
            let z = (wr.w.min(wr.h) * 0.45).clamp(6.0, 16.0);
            if wr.w > 10.0 && wr.h > 10.0 {
                p.picture(w["image"].as_str().unwrap_or(""), Rect::new(wr.x + wr.w / 2.0 - z / 2.0, wr.y + wr.h / 2.0 - z / 2.0, z, z), z * 0.22);
            }
        } else {
            p.round(wr, 1.5, c);
        }
    }
}

/// (the desktop shown is not pressed at rest: the widget opens; another desktop pressed is shown)
/// The desktops' map at rest: its first column and row, how many columns and rows, and each tiny screen's size: as many rows as
/// fit its depth (two at most across a top or bottom edge, two columns at most down a side), each in proportion to the monitors.
pub fn desk_layout(peek: &Value, horizontal: bool, depth: f32) -> (f32, f32, i32, i32, i32, i32) {
    let aspect = peek["desks"]["aspect"].as_f64().unwrap_or(16.0 / 9.0) as f32;
    let maps = peek["desks"]["maps"].as_array().cloned().unwrap_or_default();
    let (mut c0, mut r0, mut c1, mut r1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (i, m) in maps.iter().enumerate() {
        let c = m["pos"][0].as_i64().unwrap_or(i as i64) as i32;
        let r = m["pos"][1].as_i64().unwrap_or(0) as i32;
        c0 = c0.min(c);
        r0 = r0.min(r);
        c1 = c1.max(c);
        r1 = r1.max(r);
    }
    if maps.is_empty() {
        (c0, r0, c1, r1) = (0, 0, 0, 0);
    }
    let (cols, rows) = (c1 - c0 + 1, r1 - r0 + 1);
    // (each tiny screen its own fixed size: the widget grows to hold them all, both ways)
    let _ = depth;
    let (tw, th) = if horizontal {
        let th = 12.0;
        ((th * aspect).clamp(16.0, 40.0), th)
    } else {
        let tw = 24.0;
        (tw, (tw / aspect).clamp(8.0, 20.0))
    };
    (tw, th, c0, r0, cols, rows)
}

/// How deep the desktops' widget is to hold every row of its map (across its edge; large, that much more).
pub fn desk_depth(peek: &Value, horizontal: bool) -> f32 {
    let (tw, th, _, _, cols, rows) = desk_layout(peek, horizontal, 0.0);
    let across = if horizontal { rows as f32 * (th + DESK_GAP_REST) - DESK_GAP_REST } else { cols as f32 * (tw + DESK_GAP_REST) - DESK_GAP_REST };
    (across + 14.0) * peek["scale"].as_f64().unwrap_or(1.0) as f32
}

/// the gap between tiny screens at rest
const DESK_GAP_REST: f32 = 3.0;

/// (the desktop shown is not pressed at rest: the widget opens; another desktop pressed is shown)
/// At rest: the part of the map around the desktop shown, each tiny screen at its own size; where the map goes on past the widget,
/// it fades out at that side.
fn desks(p: &Painter, peek: &Value, body: Rect, horizontal: bool) -> Vec<(Rect, String)> {
    let at = peek["desks"]["at"].as_u64().unwrap_or(0) as usize;
    let maps = peek["desks"]["maps"].as_array().cloned().unwrap_or_default();
    let screens = peek["desks"]["screens"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    let (cx, cy) = (body.x + body.w / 2.0, body.y + body.h / 2.0);
    // (its icon alone: the desktop shown, its tiny screen alone, as big as the square lets it be)
    if peek["desks"]["alone"] == true {
        let aspect = peek["desks"]["aspect"].as_f64().unwrap_or(16.0 / 9.0) as f32;
        let w = (body.w.min(body.h) * 0.78).max(12.0);
        let h = (w / aspect).max(8.0);
        let r = Rect::new(cx - w / 2.0, cy - h / 2.0, w, h);
        if let Some(m) = maps.get(at) {
            draw_map(p, m, &screens, r, 3.0, true, false);
        }
        p.outline(Rect::new(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0), 2.5, 1.5, crate::draw::accent());
        return out;
    }
    // (the same size its widget was measured for: the widget fits them; large, all of it bigger)
    let depth = if horizontal { 26.0 } else { 30.0 };
    let (tw, th, c0, r0, cols, rows) = desk_layout(peek, horizontal, depth);
    let g = DESK_GAP_REST;
    let (gw, gh) = (cols as f32 * (tw + g) - g, rows as f32 * (th + g) - g);
    let pos = |i: usize, m: &Value| (m["pos"][0].as_i64().unwrap_or(i as i64) as i32, m["pos"][1].as_i64().unwrap_or(0) as i32);
    let cur = maps.get(at).map(|m| pos(at, m)).unwrap_or((c0, r0));
    let (cur_x, cur_y) = ((cur.0 - c0) as f32 * (tw + g) + tw / 2.0, (cur.1 - r0) as f32 * (th + g) + th / 2.0);
    // (the window on the map: as much as fits, the desktop shown in its middle, never past the map's ends)
    let inner = Rect::new(body.x + 4.0, body.y + 4.0, body.w - 8.0, body.h - 8.0);
    let ox = if gw <= inner.w { (gw - inner.w) / 2.0 } else { (cur_x - inner.w / 2.0).clamp(0.0, gw - inner.w) };
    let oy = if gh <= inner.h { (gh - inner.h) / 2.0 } else { (cur_y - inner.h / 2.0).clamp(0.0, gh - inner.h) };
    unsafe {
        p.dc.PushAxisAlignedClip(&p.px(inner).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }
    let a2 = crate::draw::accent2();
    for (i, m) in maps.iter().enumerate() {
        let (c, r) = pos(i, m);
        let rr = Rect::new(inner.x - ox + (c - c0) as f32 * (tw + g), inner.y - oy + (r - r0) as f32 * (th + g), tw, th);
        if rr.right() < inner.x - 2.0 || rr.x > inner.right() + 2.0 || rr.bottom() < inner.y - 2.0 || rr.y > inner.bottom() + 2.0 {
            continue;
        }
        let on = i == at;
        if m["waits"] == true && !on {
            let pulse = 0.6 + 0.4 * (p.t * 3.0).sin().abs();
            p.round(Rect::new(rr.x - 1.5, rr.y - 1.5, rr.w + 3.0, rr.h + 3.0), 4.5, a2.alpha(pulse));
        }
        draw_map(p, m, &screens, rr, 3.0, on, false);
        // (the desktop shown: its own border, bright)
        if on {
            p.outline(Rect::new(rr.x + 0.5, rr.y + 0.5, rr.w - 1.0, rr.h - 1.0), 2.5, 1.5, crate::draw::accent());
        }
        if !on {
            out.push((rr, format!("desk:{i}")));
        }
    }
    // (where the map goes on past it: that side fades into the widget's ground)
    let fade = |r: Rect, horizontal_fade: bool, from_start: bool| {
        for k in 0..6 {
            let f = k as f32 / 6.0;
            let a = 0.85 * (1.0 - f);
            let band = if horizontal_fade {
                let w = r.w / 6.0;
                Rect::new(if from_start { r.x + k as f32 * w } else { r.right() - (k as f32 + 1.0) * w }, r.y, w + 0.5, r.h)
            } else {
                let h = r.h / 6.0;
                Rect::new(r.x, if from_start { r.y + k as f32 * h } else { r.bottom() - (k as f32 + 1.0) * h }, r.w, h + 0.5)
            };
            p.round(band, 0.0, FACE.alpha(a));
        }
    };
    if ox > 0.5 {
        fade(Rect::new(inner.x, inner.y, 14.0, inner.h), true, true);
    }
    if gw - ox - inner.w > 0.5 {
        fade(Rect::new(inner.right() - 14.0, inner.y, 14.0, inner.h), true, false);
    }
    if oy > 0.5 {
        fade(Rect::new(inner.x, inner.y, inner.w, 8.0), false, true);
    }
    if gh - oy - inner.h > 0.5 {
        fade(Rect::new(inner.x, inner.bottom() - 8.0, inner.w, 8.0), false, false);
    }
    unsafe {
        p.dc.PopAxisAlignedClip();
    }
    out
}

/// How big what leads a peek is: a live icon with rings round it takes more room.
fn lead_size(peek: &Value) -> f32 {
    if peek["roll"].is_object() {
        LEAD
    } else if peek["live"]["rings"].as_array().is_some_and(|a| !a.is_empty()) {
        RINGED
    } else if peek["big"] == true {
        // (a picture that asks to be big: a logo alone)
        BIG
    } else {
        LEAD
    }
}

const RINGED: f32 = 28.0;
/// a picture that asks to be big
const BIG: f32 = 30.0;

/// A number whose digits roll, each to its next one, like an odometer's (up as it grows, down as it falls). The frame says
/// what it was (`from`) and how far the roll has come (`k`, 0..1); in the colour of how smooth it runs.
fn roll(p: &Painter, g: &serde_json::Map<String, Value>, x: f32, cy: f32, w: f32, h: f32) {
    let to = g.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0).round().max(0.0) as i64;
    let from = g.get("from").and_then(|v| v.as_f64()).map(|v| v.round().max(0.0) as i64).unwrap_or(to);
    let k = g.get("k").and_then(|v| v.as_f64()).unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    let c = color_of(g.get("color").unwrap_or(&Value::Null), GREEN);
    let (a, b) = (from.to_string(), to.to_string());
    let n = a.len().max(b.len());
    let pad = |t: &str| format!("{}{}", " ".repeat(n - t.len()), t);
    let (a, b): (Vec<char>, Vec<char>) = (pad(&a).chars().collect(), pad(&b).chars().collect());
    let fs = (h * 0.72).min(12.5);
    let dw = p.measure("0", fs, Weight::Bold, 40.0);
    let left = x + w / 2.0 - dw * n as f32 / 2.0;
    let lh = fs * 1.15;
    let up = to >= from;
    let s = p.s;
    unsafe {
        p.dc.PushAxisAlignedClip(&windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: (left - 2.0) * s, top: (cy - lh / 2.0) * s, right: (left + dw * n as f32 + 2.0) * s, bottom: (cy + lh / 2.0) * s }, windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
    }
    for i in 0..n {
        let dx = left + i as f32 * dw;
        let (d0, d1) = (a[i], b[i]);
        if d0 == d1 || k >= 1.0 {
            let t = d1.to_string();
            let tw = p.measure(&t, fs, Weight::Bold, 40.0);
            p.text(&t, dx + dw / 2.0 - tw / 2.0, cy, fs, Weight::Bold, 40.0, c);
            continue;
        }
        // (a digit further right starts a little later: they roll in a wave)
        let ki = ((k - 0.08 * (n - 1 - i) as f32) / (1.0 - 0.08 * (n - 1) as f32)).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - ki).powi(3);
        let dir = if up { -1.0 } else { 1.0 };
        for (t, off) in [(d0, dir * e * lh), (d1, dir * (e - 1.0) * lh)] {
            let t = t.to_string();
            let tw = p.measure(&t, fs, Weight::Bold, 40.0);
            p.text(&t, dx + dw / 2.0 - tw / 2.0, cy + off, fs, Weight::Bold, 40.0, c);
        }
    }
    unsafe {
        p.dc.PopAxisAlignedClip();
    }
}

/// How wide a rolling number is (its digits, at most four).
fn roll_w(peek: &Value) -> f32 {
    let n = peek["roll"]["value"].as_f64().map_or(3, |v| v.round().max(0.0).to_string().len()).clamp(2, 4);
    n as f32 * 7.8 + 4.0
}

/// a battery's cells after the words: its pictogram, four cells
const CELLS_W: f32 = 16.0 + 5.0 + 4.0 * 5.0 + 3.0 * 2.0;

/// A battery: its pictogram (a controller, a headset), then four cells, as many lit as it is full; amber when low; none told
/// (wired): the cells all lit, faint.
fn cells(p: &Painter, icon: &str, level: Option<f64>, x: f32, cy: f32) {
    let x = if icon.is_empty() {
        x - 21.0
    } else {
        p.icon(icon, x, cy - 8.0, 16.0, DIM);
        x
    };
    let lit = level.map(|l| (l * 4.0).ceil().clamp(1.0, 4.0) as usize);
    let c = match level {
        Some(l) if l <= 0.25 => AMBER,
        Some(_) => GREEN,
        None => SOFT,
    };
    for i in 0..4 {
        let r = Rect::new(x + 21.0 + i as f32 * 7.0, cy - 5.0, 5.0, 10.0);
        p.round(r, 1.5, if lit.map_or(true, |n| i < n) { c } else { FAINT });
    }
}

/// the tray, large: one even row of icons; one with news worth a glance, a chip (its icon and that, short)
const TRAY_GAP_LARGE: f32 = 8.0;
const TRAY_ICON_LARGE: f32 = 22.0;
const TRAY_NEWS: f32 = 11.5;
const TRAY_CHIP_H: f32 = 30.0;

fn tray_item_w(p: &Painter, it: &Value) -> f32 {
    let news = &it["news"];
    if news["mark"].is_string() {
        4.0 + TRAY_ICON_LARGE + 6.0 + 12.0 + 9.0
    } else if let Some(t) = news["text"].as_str() {
        4.0 + TRAY_ICON_LARGE + 6.0 + p.measure(t, TRAY_NEWS, Weight::Semibold, 140.0) + 10.0
    } else {
        TRAY_ICON_LARGE
    }
}

const TRAY_ICON: f32 = 18.0;
const TRAY_GAP: f32 = 10.0;

/// The tray at rest: its first icons (each pressed for its app), then how many more.
/// how big a tray icon is drawn, every one, small or large: Windows' own small icon (the size the apps make them for)
const TRAY_PIC: f32 = 16.0;

/// A tray icon in its box: all of them one size (Windows' small icon, which is what the apps draw them at: blown up, they blur), on
/// whole pixels, in the box's middle. Drawn?
fn tray_icon(p: &Painter, image: &str, r: Rect) -> bool {
    let s = p.s;
    let z = TRAY_PIC.min(r.w.min(r.h));
    let snap = |v: f32| (v * s).round() / s;
    let (x, y) = (snap(r.x + (r.w - z) / 2.0), snap(r.y + (r.h - z) / 2.0));
    p.picture(image, Rect::new(x, y, snap(z), snap(z)), 3.0)
}

fn tray(p: &Painter, peek: &Value, body: Rect, horizontal: bool) -> Vec<(Rect, String)> {
    let items = peek["tray"]["items"].as_array().cloned().unwrap_or_default();
    let more = peek["tray"]["more"].as_u64().unwrap_or(0);
    let mut out = Vec::new();
    let z = TRAY_ICON;
    let mut at = if horizontal { body.x + 12.0 } else { body.y + 12.0 };
    let cross = if horizontal { body.y + body.h / 2.0 } else { body.x + body.w / 2.0 };
    let place = |at: f32, w: f32| if horizontal { Rect::new(at, cross - z / 2.0, w, z) } else { Rect::new(cross - w / 2.0, at, w, z) };
    let split = peek["tray"]["split"].as_u64().unwrap_or(0) as usize;
    // (large: one even row of icons, bigger; one whose app has news worth a glance grows into a chip: its icon and that, short)
    if peek["tray"]["large"] == true && horizontal {
        let mut at = body.x + 14.0;
        let cy = body.y + body.h / 2.0;
        let zl = TRAY_ICON_LARGE;
        for it in &items {
            let w = tray_item_w(p, it);
            let news = &it["news"];
            let chip = !news.is_null();
            let ix = if chip { at + 4.0 } else { at };
            if chip {
                p.round(Rect::new(at, cy - TRAY_CHIP_H / 2.0, w, TRAY_CHIP_H), TRAY_CHIP_H / 2.0, crate::paint::Color(0.106, 0.106, 0.125, 1.0));
            }
            let r = Rect::new(ix, cy - zl / 2.0, zl, zl);
            if !tray_icon(p, it["image"].as_str().unwrap_or(""), r) {
                p.round(r, 6.0, SOFT);
            }
            let tx = ix + zl + 6.0;
            match news["mark"].as_str() {
                Some("ok") => p.icon("check", tx, cy - 6.0, 12.0, GREEN),
                Some(_) => p.icon("x", tx, cy - 6.0, 12.0, crate::draw::RED),
                None => {
                    if let Some(t) = news["text"].as_str() {
                        let c = if t.starts_with('↓') || t.starts_with('↑') { crate::paint::Color(0.35, 0.78, 0.9, 1.0) } else { TEXT };
                        p.text(t, tx, cy, TRAY_NEWS, Weight::Semibold, 140.0, c);
                    }
                }
            }
            out.push((Rect::new(at - 3.0, cy - TRAY_CHIP_H / 2.0 - 2.0, w + 6.0, TRAY_CHIP_H + 4.0), it["action"].as_str().unwrap_or("").to_string()));
            at += w + TRAY_GAP_LARGE;
        }
        if more > 0 {
            p.icon("dots", at + 2.0, cy - 8.0, 16.0, DIM);
        }
        return out;
    }
    for (n, it) in items.iter().enumerate() {
        // (while editing: a mark after the row's icons; the rest follow it)
        if split > 0 && n == split {
            let m = place(at - TRAY_GAP / 2.0 - 1.0, 2.0);
            p.round(Rect::new(m.x, m.y - 2.0, m.w, m.h + 4.0), 1.0, SOFT);
        }
        let r = place(at, z);
        if !tray_icon(p, it["image"].as_str().unwrap_or(""), r) {
            p.round(r, 5.0, SOFT);
        }
        out.push((Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0), it["action"].as_str().unwrap_or("").to_string()));
        at += z + TRAY_GAP;
    }
    if more > 0 {
        let w = z + 8.0;
        let r = place(at, w);
        p.icon("dots", r.x + w / 2.0 - 8.0, r.y + z / 2.0 - 8.0, 16.0, DIM);
    }
    out
}
const DOCK_GAP: f32 = 8.0;

/// How long the body of a peek is along its edge (logical px).
/// How much bigger a large widget's peek is drawn.

/// a song at rest: its cover so big, its words at most so wide, its stacked covers so far apart
const SONG_COVER: f32 = 24.0;
const SONG_TEXT: f32 = 150.0;
const SONG_STACK: f32 = 6.0;

fn song_text_w(p: &Painter, s: &Value) -> f32 {
    let t = p.measure(s["title"].as_str().unwrap_or(""), 12.0, Weight::Semibold, 10_000.0);
    let a = p.measure(s["artist"].as_str().unwrap_or(""), 11.0, Weight::Regular, 10_000.0);
    t.max(a).min(SONG_TEXT)
}

fn song_w(p: &Painter, peek: &Value) -> f32 {
    let s = &peek["song"];
    if s["alone"] == true {
        return METERS_SQUARE;
    }
    let stack = s["stack"].as_array().map_or(0, |a| a.len().min(2)) as f32 * SONG_STACK;
    12.0 + SONG_COVER + stack + 10.0 + song_text_w(p, s) + 10.0 + 13.0 + 12.0
}

/// A song at rest: its cover (another player's covers just behind it), the song over who sings it, its three bars in its cover's
/// colour (flat while paused); large, how long is left under them. Its icon alone: the bars alone, in the middle.
fn song(p: &Painter, peek: &Value, body: Rect) {
    let s = &peek["song"];
    let c = color_of(&peek["color"], TEXT);
    let playing = s["playing"] == true;
    let parts = if playing { p.viz } else { crate::draw::FLAT };
    let cy = body.y + body.h / 2.0;
    if s["alone"] == true {
        let h: f32 = 16.0;
        p.viz(body.x + body.w / 2.0 - crate::draw::bars_w(h) / 2.0, cy + 1.5, h, parts, c);
        return;
    }
    let mut x = body.x + 12.0;
    let stack: Vec<String> = s["stack"].as_array().map(|a| a.iter().take(2).filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    // (the others' covers just behind the one shown, fainter the further back)
    for (i, cov) in stack.iter().enumerate().rev() {
        let off = (i + 1) as f32 * SONG_STACK;
        let a = if i == 0 { 0.75 } else { 0.5 };
        crate::draw::faded(p.dc, a, || {
            if !p.picture(cov, Rect::new(x + off, cy - SONG_COVER / 2.0, SONG_COVER, SONG_COVER), 6.0) {
                p.round(Rect::new(x + off, cy - SONG_COVER / 2.0, SONG_COVER, SONG_COVER), 6.0, SOFT);
            }
        });
    }
    let cr = Rect::new(x, cy - SONG_COVER / 2.0, SONG_COVER, SONG_COVER);
    if !p.picture(s["cover"].as_str().unwrap_or(""), cr, 6.0) {
        p.round(cr, 6.0, SOFT);
        p.icon("music", cr.x + 5.0, cr.y + 5.0, 14.0, TEXT);
    }
    x += SONG_COVER + stack.len() as f32 * SONG_STACK + 10.0;
    let tw = song_text_w(p, s);
    let artist = s["artist"].as_str().unwrap_or("");
    if artist.is_empty() {
        p.text(s["title"].as_str().unwrap_or(""), x, cy, 12.0, Weight::Semibold, tw, TEXT);
    } else {
        p.text(s["title"].as_str().unwrap_or(""), x, cy - 7.0, 12.0, Weight::Semibold, tw, TEXT);
        p.text(artist, x, cy + 7.5, 11.0, Weight::Regular, tw, DIM);
    }
    x += tw + 10.0;
    // (large: how long is left, under its bars)
    let large = peek["scale"].as_f64().unwrap_or(1.0) > 1.01;
    let left = s["left"].as_str().unwrap_or("");
    if large && !left.is_empty() {
        p.viz(x, cy - 4.0, 10.0, parts, c);
        let lw = p.measure(left, 7.5, Weight::Regular, 40.0);
        p.text(left, x + 6.5 - lw / 2.0, cy + 6.5, 7.5, Weight::Regular, 40.0, DIM);
    } else {
        p.viz(x, cy, 12.0, parts, c);
    }
}

thread_local! {
    /// how loud the microphone is now (0..1), springing: Speech's bars move with it
    static VOICE: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// How loud the microphone is now.
pub fn set_voice(v: f32) {
    VOICE.with(|x| x.set(v));
}

/// a voice at rest: its bars, the gap to its time
const VOICE_BARS: usize = 6;
const VOICE_H: f32 = 18.0;

fn voice_time(v: &Value) -> String {
    let Some(since) = v["since"].as_f64() else { return String::new() };
    let t = ((crate::blocks::now_ms() - since) / 1000.0).max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t % 3600) / 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

fn voice_w(p: &Painter, peek: &Value) -> f32 {
    let v = &peek["voice"];
    if v["alone"] == true {
        return METERS_SQUARE;
    }
    let bars = VOICE_BARS as f32 * 4.5;
    let state = v["state"].as_str().unwrap_or("off");
    let words = match state {
        "listening" => p.measure("0:00:00", 12.0, Weight::Semibold, 80.0).min(p.measure(&voice_time(v), 12.0, Weight::Semibold, 80.0) + 2.0),
        "review" => p.measure("Going Over", 12.0, Weight::Semibold, 120.0),
        _ => 0.0,
    };
    let who = if v["who"]["avatar"].is_string() && peek["scale"].as_f64().unwrap_or(1.0) > 1.01 { 28.0 } else { 0.0 };
    if words == 0.0 && who == 0.0 {
        return METERS_SQUARE;
    }
    12.0 + who + bars + if words > 0.0 { 9.0 + words } else { 0.0 } + 12.0
}

/// A voice at rest (Speech): thin bars that move with the microphone in recording red while it listens (quiet: low, breathing),
/// and how long it has listened; still and grey when not; amber and slow while it goes over a session again; in a filling ring
/// while its model comes; with a red dot when something went wrong. Large: who is talking first. Its icon alone: the bars alone.
fn voice(p: &Painter, peek: &Value, body: Rect, horizontal: bool) {
    let v = &peek["voice"];
    let state = v["state"].as_str().unwrap_or("off");
    let alone = v["alone"] == true || !horizontal;
    let cy = body.y + body.h / 2.0;
    let red = crate::paint::Color(0.910, 0.416, 0.416, 1.0);
    let grey = crate::paint::Color(0.29, 0.29, 0.32, 1.0);
    let n = if alone { 5 } else { VOICE_BARS };
    let (bw, gap) = (2.5, 2.0);
    let bars_w = n as f32 * bw + (n as f32 - 1.0) * gap;
    let words = match state {
        "listening" if !alone => voice_time(v),
        "review" if !alone => "Going Over".to_string(),
        _ => String::new(),
    };
    let ww = if words.is_empty() { 0.0 } else { p.measure(&words, 12.0, Weight::Semibold, 120.0) };
    let who = !alone && v["who"]["avatar"].is_string() && peek["scale"].as_f64().unwrap_or(1.0) > 1.01;
    let total = (if who { 28.0 } else { 0.0 }) + bars_w + if ww > 0.0 { 9.0 + ww } else { 0.0 };
    let mut x = body.x + body.w / 2.0 - total / 2.0;
    if who {
        p.circle(x + 11.0, cy, 11.0, color_of(&v["who"]["tint"], crate::paint::Color(0.149, 0.149, 0.173, 1.0)));
        let a = v["who"]["avatar"].as_str().unwrap_or("");
        let aw = p.measure(a, 12.0, Weight::Regular, 30.0);
        p.paragraph(a, x + 11.0 - aw / 2.0 - 0.5, cy - 12.0 * 0.675, 12.0, aw + 6.0, 1, TEXT, true);
        x += 28.0;
    }
    // (each bar: how loud it is now, each a little apart from the others; quiet, a slow breath)
    let t = p.t;
    let loud = VOICE.with(|l| l.get());
    let (c, k) = match state {
        "listening" => (red, 1.0),
        "review" => (crate::draw::AMBER, 0.35),
        _ => (grey, 0.0),
    };
    let shape = [0.45, 0.8, 1.0, 0.65, 0.9, 0.5];
    for i in 0..n {
        let s = shape[i % shape.len()];
        let wob = 0.5 + 0.5 * (t * (5.0 + i as f32 * 1.7) + i as f32 * 1.3).sin();
        let breath = 0.18 + 0.08 * (t * 2.2 + i as f32 * 0.6).sin();
        let live = if state == "listening" { (loud.sqrt() * 1.6).min(1.0) * (0.55 + 0.45 * wob) } else if state == "review" { 0.35 + 0.25 * (t * 1.4 + i as f32 * 0.8).sin() } else { 0.0 };
        let part = if k > 0.0 { (breath + live * s).clamp(0.15, 1.0) } else { s * 0.7 };
        let h = (VOICE_H * part).max(3.0);
        p.round(Rect::new(x + i as f32 * (bw + gap), cy - h / 2.0, bw, h), bw / 2.0, c);
    }
    // (its model coming: a ring round the bars, filling)
    if state == "loading" {
        let r = bars_w.max(VOICE_H) / 2.0 + 5.0;
        p.ring(x + bars_w / 2.0, cy, r, 2.0, v["progress"].as_f64().unwrap_or(0.0) as f32, 0.0, crate::draw::accent(), FAINT);
    }
    if state == "error" {
        p.circle(x + bars_w + 2.0, cy - VOICE_H / 2.0, 2.5, crate::draw::RED);
    }
    if ww > 0.0 {
        p.text(&words, x + bars_w + 9.0, cy, 12.0, Weight::Semibold, 120.0, if state == "review" { crate::draw::AMBER } else { TEXT });
    }
}

/// a volume at rest: its icon, its line, the gaps
const LEVEL_ICON: f32 = 16.0;
const LEVEL_LINE: f32 = 46.0;

fn level_w(peek: &Value) -> f32 {
    let l = &peek["level"];
    if l["muted"] == true {
        return METERS_SQUARE;
    }
    let large = peek["scale"].as_f64().unwrap_or(1.0) > 1.01;
    let bars = if large && l["playing"] == true { 8.0 + crate::draw::bars_w(12.0) } else { 0.0 };
    12.0 + LEVEL_ICON + 8.0 + LEVEL_LINE + bars + 12.0
}

/// A volume at rest: its icon, a small line as long as its volume (how loud it is now, faint over it); large, the sound's bars after
/// it while anything plays. Muted: its icon alone, red. On a side edge: its icon alone.
fn level(p: &Painter, peek: &Value, body: Rect, horizontal: bool) {
    let l = &peek["level"];
    let cy = body.y + body.h / 2.0;
    let icon = peek["icon"].as_str().unwrap_or("volume");
    if l["muted"] == true || !horizontal {
        let c = if l["muted"] == true { crate::draw::RED } else { TEXT };
        p.icon(icon, body.x + body.w / 2.0 - LEVEL_ICON / 2.0, cy - LEVEL_ICON / 2.0, LEVEL_ICON, c);
        return;
    }
    let mut x = body.x + body.w / 2.0 - (level_w(peek) - 24.0) / 2.0;
    p.icon(icon, x, cy - LEVEL_ICON / 2.0, LEVEL_ICON, TEXT);
    x += LEVEL_ICON + 8.0;
    let v = l["value"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    p.round(Rect::new(x, cy - 2.0, LEVEL_LINE, 4.0), 2.0, crate::paint::Color(0.165, 0.165, 0.188, 1.0));
    if v > 0.0 {
        p.round(Rect::new(x, cy - 2.0, (LEVEL_LINE * v).max(4.0), 4.0), 2.0, crate::draw::accent());
        let lv = (crate::draw::live_level(crate::blocks::meter(l["meter"].as_str().unwrap_or(""))) * v).clamp(0.0, v);
        if lv > 0.005 {
            p.round(Rect::new(x, cy - 2.0, (LEVEL_LINE * lv).max(4.0), 4.0), 2.0, crate::draw::LIVE);
        }
    }
    x += LEVEL_LINE;
    if peek["scale"].as_f64().unwrap_or(1.0) > 1.01 && l["playing"] == true {
        p.viz(x + 8.0, cy, 12.0, p.viz, crate::draw::accent());
    }
}

/// bars in a square (graphs, their icon alone): so long a side; each bar so thick, so far apart
const METERS_SQUARE: f32 = 38.0;
const METER_T: f32 = 4.0;
const METER_GAP: f32 = 4.0;

/// Bars in a square, each as full as it is now: standing up on an edge along the top or the bottom, lying down on a side edge.
fn meters(p: &Painter, peek: &Value, body: Rect, horizontal: bool) {
    let bars = peek["meters"].as_array().cloned().unwrap_or_default();
    let n = bars.len().max(1) as f32;
    let span = n * METER_T + (n - 1.0) * METER_GAP;
    let track = crate::paint::Color(0.106, 0.106, 0.125, 1.0);
    for (i, b) in bars.iter().enumerate() {
        let c = crate::draw::color_of(&b["color"], TEXT);
        let part = b["part"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        if horizontal {
            let len = (body.h - 16.0).max(8.0);
            let x = body.x + body.w / 2.0 - span / 2.0 + i as f32 * (METER_T + METER_GAP);
            let foot = body.y + body.h / 2.0 + len / 2.0;
            p.round(Rect::new(x, foot - len, METER_T, len), METER_T / 2.0, track);
            let f = (len * part).max(METER_T);
            p.round(Rect::new(x, foot - f, METER_T, f), METER_T / 2.0, c);
        } else {
            let len = (body.w - 16.0).max(8.0);
            let y = body.y + body.h / 2.0 - span / 2.0 + i as f32 * (METER_T + METER_GAP);
            let x0 = body.x + body.w / 2.0 - len / 2.0;
            p.round(Rect::new(x0, y, len, METER_T), METER_T / 2.0, track);
            p.round(Rect::new(x0, y, (len * part).max(METER_T), METER_T), METER_T / 2.0, c);
        }
    }
}

/// a peek of graphs: each so wide (large: wider, its name over it), so far apart
const GRAPH_W: f32 = 44.0;
const GRAPH_W_LARGE: f32 = 52.0;
const GRAPH_GAP: f32 = 12.0;

fn graph_vals(it: &Value) -> Vec<f32> {
    it["values"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64().map(|v| v as f32)).collect()).unwrap_or_default()
}

/// How long a peek of graphs is (its items: values, colour, text, label; large: its name over each).
fn graphs_w(p: &Painter, peek: &Value) -> f32 {
    let items = peek["graphs"]["items"].as_array().cloned().unwrap_or_default();
    let large = peek["scale"].as_f64().unwrap_or(1.0) > 1.01;
    let n = items.len().max(1) as f32;
    let each: f32 = items
        .iter()
        .map(|it| if large { GRAPH_W_LARGE } else { GRAPH_W + 5.0 + p.measure(it["text"].as_str().unwrap_or(""), 12.0, Weight::Semibold, 60.0) })
        .sum();
    12.0 * 2.0 + each + (n - 1.0) * GRAPH_GAP
}

/// A peek of graphs: each the last while of what it measures, its number now beside it (large: its name and number over it).
fn graphs(p: &Painter, peek: &Value, body: Rect) {
    let items = peek["graphs"]["items"].as_array().cloned().unwrap_or_default();
    let large = peek["scale"].as_f64().unwrap_or(1.0) > 1.01;
    let cy = body.y + body.h / 2.0;
    let mut x = body.x + 12.0;
    for it in &items {
        let c = crate::draw::color_of(&it["color"], TEXT);
        let vals = graph_vals(it);
        let text = it["text"].as_str().unwrap_or("");
        if large {
            let lw = p.text(it["label"].as_str().unwrap_or(""), x, cy - 6.5, 8.0, Weight::Semibold, 30.0, DIM);
            p.text(text, x + lw + 4.0, cy - 6.5, 9.5, Weight::Semibold, 40.0, c);
            p.spark(&vals, Rect::new(x, cy - 1.0, GRAPH_W_LARGE, 10.0), c);
            x += GRAPH_W_LARGE + GRAPH_GAP;
        } else {
            p.spark(&vals, Rect::new(x, cy - 9.0, GRAPH_W, 18.0), c);
            let tw = p.text(text, x + GRAPH_W + 5.0, cy, 12.0, Weight::Semibold, 60.0, c);
            x += GRAPH_W + 5.0 + tw + GRAPH_GAP;
        }
    }
}

/// How long the body of a peek is along its edge (logical px); a large one's (its `scale`), that much longer.
pub fn measure(p: &Painter, peek: &Value, horizontal: bool) -> f32 {
    if peek["voice"].is_object() {
        let w = if horizontal { voice_w(p, peek) } else { METERS_SQUARE };
        return w * peek["scale"].as_f64().unwrap_or(1.0) as f32;
    }
    if peek["level"].is_object() {
        let w = if horizontal { level_w(peek) } else { METERS_SQUARE };
        return w * peek["scale"].as_f64().unwrap_or(1.0) as f32;
    }
    if peek["song"].is_object() {
        let w = if horizontal { song_w(p, peek) } else { METERS_SQUARE };
        return w * peek["scale"].as_f64().unwrap_or(1.0) as f32;
    }
    if peek["meters"].is_array() {
        return METERS_SQUARE * peek["scale"].as_f64().unwrap_or(1.0) as f32;
    }
    if peek["graphs"].is_object() && horizontal {
        return graphs_w(p, peek) * peek["scale"].as_f64().unwrap_or(1.0) as f32;
    }
    if two_rows(peek, horizontal) {
        return two_rows_w(p, peek);
    }
    measure_at(p, peek, horizontal) * peek["scale"].as_f64().unwrap_or(1.0) as f32
}

/// Draw a peek in its body; a large one (its `scale`) drawn that much bigger. The boxes that can be pressed in it, and what each does.
pub fn draw(p: &Painter, peek: &Value, body: Rect, horizontal: bool, dot: bool) -> Vec<(Rect, String)> {
    let k = peek["scale"].as_f64().unwrap_or(1.0) as f32;
    if two_rows(peek, horizontal) && !dot {
        draw_two_rows(p, peek, body);
        return Vec::new();
    }
    if (k - 1.0).abs() < 0.01 {
        return draw_at(p, peek, body, horizontal, dot);
    }
    // (drawn with everything k times bigger: the body as many times smaller in the painter's units, which are k times bigger)
    let big = Painter { g: p.g, dc: p.dc, s: p.s * k, pc: p.pc, t: p.t, viz: p.viz };
    let b = Rect::new(body.x / k, body.y / k, body.w / k, body.h / k);
    draw_at(&big, peek, b, horizontal, dot).into_iter().map(|(r, a)| (Rect::new(r.x * k, r.y * k, r.w * k, r.h * k), a)).collect()
}

/// a subtitle at rest: its words this big, at most this long, in so many lines, so far in from its edges
const SUB_SIZE: f32 = 14.0;
const SUB_MAX: f32 = 560.0;
const SUB_LINES: f32 = 3.0;
const SUB_PAD: f32 = 12.0;
const SUB_PAD_Y: f32 = 7.0;

fn sub_name_w(p: &Painter, s: &Value) -> f32 {
    let name = s["name"].as_str().unwrap_or("");
    if name.is_empty() {
        0.0
    } else {
        p.measure(name, SUB_SIZE, Weight::Semibold, 160.0) + 8.0
    }
}

/// The newest of its words that fit in its lines, `w` wide: a long one is shown by its end, never cut with dots.
fn subtitle_fit(p: &Painter, text: &str, w: f32) -> String {
    let lh = SUB_SIZE * 1.35;
    let mut words: Vec<&str> = text.split_whitespace().collect();
    loop {
        let t = words.join(" ");
        let h = p.paragraph(&t, 0.0, 0.0, SUB_SIZE, w, 99, TEXT, false);
        if h <= lh * SUB_LINES + 1.0 || words.len() <= 1 {
            return t;
        }
        words.remove(0);
    }
}

/// The newest of its words that fit in so many lines, `w` wide, at `size`: shown by its end, never cut with dots.
pub fn tail_fit(p: &Painter, text: &str, size: f32, w: f32, lines: f32) -> String {
    let lh = size * 1.35;
    let mut words: Vec<&str> = text.split_whitespace().collect();
    loop {
        let t = words.join(" ");
        let h = p.paragraph(&t, 0.0, 0.0, size, w, 99, TEXT, false);
        if h <= lh * lines + 1.0 || words.len() <= 1 {
            return t;
        }
        words.remove(0);
    }
}

/// How long a subtitle stands along its edge: as long as its words in one line, up to so long.
fn subtitle_along(p: &Painter, s: &Value) -> f32 {
    let tw = p.measure(s["text"].as_str().unwrap_or(""), SUB_SIZE, Weight::Regular, 10_000.0);
    (2.0 * SUB_PAD + sub_name_w(p, s) + tw + 2.0).clamp(120.0, SUB_MAX)
}

/// How deep a peek stands out of its edge, when its words ask for more than one line (a subtitle), `along` long; none: the edge's own.
pub fn depth_of(p: &Painter, peek: &Value, along: f32) -> Option<f32> {
    let s = &peek["subtitle"];
    if !s.is_object() {
        return None;
    }
    let w = (along - 2.0 * SUB_PAD - sub_name_w(p, s)).max(40.0);
    let t = subtitle_fit(p, s["text"].as_str().unwrap_or(""), w);
    let h = p.paragraph(&t, 0.0, 0.0, SUB_SIZE, w, 99, TEXT, false);
    Some((h + 2.0 * SUB_PAD_Y).max(32.0).round())
}

/// A subtitle at rest: who is speaking, and the newest of what they said, in as many lines as it takes (three at most).
fn subtitle(p: &Painter, s: &Value, body: Rect) {
    let name = s["name"].as_str().unwrap_or("");
    let nw = sub_name_w(p, s);
    let w = (body.w - 2.0 * SUB_PAD - nw).max(40.0);
    let t = subtitle_fit(p, s["text"].as_str().unwrap_or(""), w);
    let h = p.paragraph(&t, 0.0, 0.0, SUB_SIZE, w, 99, TEXT, false);
    let y = body.y + (body.h - h) / 2.0;
    if !name.is_empty() {
        p.text(name, body.x + SUB_PAD, y + SUB_SIZE * 0.68, SUB_SIZE, Weight::Semibold, 160.0, TEXT);
    }
    p.paragraph(&t, body.x + SUB_PAD + nw, y, SUB_SIZE, w, 99, if s["draft"] == true { TEXT.alpha(0.78) } else { TEXT }, true);
}

/// The tray as its icon alone: up to four of its icons in a square, as big as fit (one: all of it; two: side by side; three: two
/// over one).
fn tray_grid(p: &Painter, items: &[Value], body: Rect) {
    let n = items.len().min(4);
    if n == 0 {
        p.icon("grid", body.x + body.w / 2.0 - 8.0, body.y + body.h / 2.0 - 8.0, 16.0, DIM);
        return;
    }
    let side = (body.w.min(body.h) - 12.0).max(10.0);
    let (cx, cy) = (body.x + body.w / 2.0, body.y + body.h / 2.0);
    let gap = 2.0;
    let cells: Vec<(f32, f32, f32)> = match n {
        1 => vec![(cx, cy, side * 0.75)],
        2 => {
            let z = (side - gap) / 2.0;
            vec![(cx - (z + gap) / 2.0, cy, z), (cx + (z + gap) / 2.0, cy, z)]
        }
        _ => {
            let z = (side - gap) / 2.0;
            let d = (z + gap) / 2.0;
            let mut v = vec![(cx - d, cy - d, z), (cx + d, cy - d, z)];
            if n == 3 {
                v.push((cx, cy + d, z));
            } else {
                v.push((cx - d, cy + d, z));
                v.push((cx + d, cy + d, z));
            }
            v
        }
    };
    for (it, (x, y, z)) in items.iter().zip(cells) {
        let r = Rect::new(x - z / 2.0, y - z / 2.0, z, z);
        if !tray_icon(p, it["image"].as_str().unwrap_or(""), r) {
            p.round(r, 3.0, SOFT);
        }
    }
}

fn measure_at(p: &Painter, peek: &Value, horizontal: bool) -> f32 {
    if peek["tray_grid"].is_object() || peek["dock_front"].is_object() {
        return 32.0;
    }
    if peek["subtitle"].is_object() {
        return subtitle_along(p, &peek["subtitle"]);
    }
    // (a face is drawn by the frame: it takes this much room)
    if peek["face"].is_string() {
        return if horizontal { 76.0 } else { 56.0 };
    }
    if let Some(items) = peek["tray"]["items"].as_array() {
        let n = items.len() as f32;
        let more = if peek["tray"]["more"].as_u64().unwrap_or(0) > 0 { TRAY_ICON + 8.0 + TRAY_GAP } else { 0.0 };
        if peek["tray"]["large"] == true && horizontal {
            let ws: f32 = items.iter().map(|i| tray_item_w(p, i)).sum();
            let more = if peek["tray"]["more"].as_u64().unwrap_or(0) > 0 { 20.0 + TRAY_GAP_LARGE } else { 0.0 };
            return 14.0 * 2.0 + ws + (n - 1.0).max(0.0) * TRAY_GAP_LARGE + more;
        }
        return 12.0 * 2.0 + n * TRAY_ICON + (n - 1.0).max(0.0) * TRAY_GAP + more;
    }
    if peek["desks"]["n"].as_u64().is_some() {
        // (its map, each tiny screen at its own size, as long as the map is, up to `DESKS_MAX`; large, that much bigger)
        let depth = if horizontal { 26.0 } else { 30.0 };
        let (tw, th, _, _, cols, rows) = desk_layout(peek, horizontal, depth);
        let along = if horizontal { cols as f32 * (tw + DESK_GAP_REST) - DESK_GAP_REST } else { rows as f32 * (th + DESK_GAP_REST) - DESK_GAP_REST };
        return 8.0 * 2.0 + along.min(DESKS_MAX);
    }
    if let Some(items) = peek["dock"]["items"].as_array() {
        // (each app its icon, its marks over it or under it; nothing beside it)
        let n = (items.len() as f32).max(1.0);
        let icon = dock_icon(dock_depth(peek));
        // (at its ends, its grips)
        let stages: f32 = if horizontal { items.iter().filter(|i| staged(peek, i)).map(|i| stage_w(p, peek, i) - icon).sum() } else { 0.0 };
        return DOCK_END * 2.0 + n * icon + (n - 1.0) * DOCK_GAP + stages;
    }
    if !horizontal {
        let (text, _) = words_of(peek);
        return 10.0 + lead_size(peek) + if text.is_empty() { 0.0 } else { 18.0 } + if peek["bars"].is_array() { 10.0 } else { 0.0 } + 10.0;
    }
    let (text, sub) = words_of(peek);
    let pad = if peek["clock"].is_object() && lead_w(peek) == 0.0 { CLOCK_PAD } else { PAD };
    let mut w = pad + lead_w(peek);
    if !text.is_empty() {
        if w > pad {
            w += GAP;
        }
        w += p.measure(&text, TEXT_SIZE, Weight::Semibold, MAX_TEXT);
    }
    if !sub.is_empty() {
        w += 6.0 + p.measure(&sub, TEXT_SIZE - 1.5, Weight::Regular, 80.0);
    }
    if peek["viz"].as_bool() == Some(true) {
        w += GAP + 14.0;
    }
    if peek["button"].is_object() {
        w += GAP + 20.0;
    }
    if peek["dot"].as_bool() == Some(true) {
        w += GAP + 6.0;
    }
    if peek["cells"].is_object() {
        w += GAP + CELLS_W - if peek["cells"]["icon"] == "" { 21.0 } else { 0.0 };
    }
    // (an icon alone hugs it; with words, never shorter than a short pill)
    let alone = text.is_empty() && sub.is_empty() && peek["viz"].as_bool() != Some(true) && !peek["button"].is_object() && peek["dot"].as_bool() != Some(true) && !peek["cells"].is_object();
    if alone && lead_w(peek) > 0.0 {
        w + pad
    } else {
        (w + pad).max(56.0)
    }
}

/// What leads it: a picture, the state of something live, a ring, the vessel, bars, or its icon; in a box `size` tall at `x`, centred on `cy`.
fn lead(p: &Painter, peek: &Value, x: f32, cy: f32, size: f32) -> f32 {
    if bare(peek) {
        return 0.0;
    }
    let c = color_of(&peek["color"], TEXT);
    let icon = peek["icon"].as_str().unwrap_or("dot");
    let y = cy - size / 2.0;
    // (four bars, as strong as it is; offline: hollow, a red dot)
    if let Some(sig) = peek["signal"].as_object() {
        let level = sig.get("level").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let off = sig.get("off").and_then(|v| v.as_bool()) == Some(true);
        let bw = (size * 0.15).max(2.0);
        let gap = (size * 0.1).max(1.5);
        let total = 4.0 * bw + 3.0 * gap;
        let x0 = x + size / 2.0 - total / 2.0;
        let foot = cy + size * 0.38;
        for i in 0..4 {
            let h = size * (0.25 + 0.17 * i as f32) * 1.05;
            p.round(Rect::new(x0 + i as f32 * (bw + gap), foot - h, bw, h), bw / 2.0, if i < level { c } else { FAINT });
        }
        if off {
            p.circle(x0 + total - bw / 2.0, foot - size * 0.78 - 1.0, 2.5, crate::draw::RED);
        }
        return size;
    }
    // (a calendar's day as a little page: the month on its top, red; an event coming, the top takes its colour and drains as it comes)
    if let Some(pg) = peek["page"].as_object() {
        let (w, h) = (PAGE_W, PAGE_W * 1.08);
        let r = Rect::new(x, cy - h / 2.0, w, h);
        p.round(r, 6.0, crate::paint::Color(0.12, 0.12, 0.145, 1.0));
        let top = Rect::new(r.x, r.y, w, 8.0);
        let red = crate::paint::Color(0.91, 0.42, 0.42, 1.0);
        let gs = |k: &str| pg.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        match pg.get("part").and_then(|v| v.as_f64()) {
            Some(part) => {
                let c = crate::draw::color_of(pg.get("color").unwrap_or(&Value::Null), crate::draw::AMBER);
                p.round_top(top, 6.0, c.alpha(0.35));
                p.round_top(Rect::new(top.x, top.y, (w * part as f32).max(0.0), top.h), 6.0, c);
            }
            None => p.round_top(top, 6.0, red),
        }
        let mon = gs("mon");
        let mw = p.measure(&mon, 5.5, Weight::Bold, w);
        p.text(&mon, r.x + w / 2.0 - mw / 2.0, top.y + 4.2, 5.5, Weight::Bold, w, crate::paint::Color(1.0, 1.0, 1.0, 0.95));
        let day = gs("day");
        let dw = p.measure(&day, 11.5, Weight::Semibold, w);
        p.text(&day, r.x + w / 2.0 - dw / 2.0, r.y + 8.0 + (h - 8.0) / 2.0, 11.5, Weight::Semibold, w, TEXT);
        return w;
    }
    // (each drive, its letter in a ring as full as it (red past 90%); one being read or written, blue, its arc spinning)
    if let Some(items) = peek["drives"]["items"].as_array() {
        let mut dx = x;
        let _ = size;
        let z = DEV_RING;
        for it in items.iter().take(DRIVES_MAX) {
            let r = z / 2.0;
            let (rx, ry) = (dx + r, cy);
            let busy = it["busy"] == true;
            let part = it["part"].as_f64().unwrap_or(0.0) as f32;
            if busy {
                let blue = crate::paint::Color(0.31, 0.56, 0.94, 1.0);
                p.circle(rx, ry, r + 1.0, blue.alpha(0.14));
                p.ring(rx, ry, r - 1.5, 2.2, part.max(0.25), (p.t * 0.8).fract(), blue, FAINT);
                crate::draw::keep_moving();
            } else {
                let col = if it["red"] == true { crate::draw::RED } else { crate::draw::accent() };
                p.ring(rx, ry, r - 1.5, 2.2, part, 0.0, col, FAINT);
            }
            let l = it["letter"].as_str().unwrap_or("");
            let fs = z * 0.42;
            let lw = p.measure(l, fs, crate::paint::Weight::Semibold, z);
            p.text(l, rx - lw / 2.0, ry, fs, crate::paint::Weight::Semibold, z, if busy { crate::paint::Color(0.75, 0.84, 1.0, 1.0) } else { c });
            dx += z + 5.0;
        }
        return dx - x - 5.0;
    }
    // (each device, its pictogram in a ring as full as its battery; amber when low; none told: its pictogram alone)
    if let Some(items) = peek["devs"]["items"].as_array() {
        let mut dx = x;
        for it in items.iter().take(3) {
            let r = DEV_RING / 2.0;
            let (rx, ry) = (dx + r, cy);
            if let Some(l) = it["level"].as_f64() {
                let col = if it["low"] == true { crate::draw::AMBER } else { crate::draw::accent() };
                p.ring(rx, ry, r - 1.5, 2.2, l as f32, 0.0, col, FAINT);
            }
            let z = DEV_RING * 0.48;
            p.icon(it["icon"].as_str().unwrap_or("bluetooth"), rx - z / 2.0, ry - z / 2.0, z, c);
            dx += DEV_RING + 6.0;
        }
        return dx - x - 6.0;
    }
    if let Some(bars) = peek["bars"].as_array() {
        let n = bars.len().clamp(1, 4) as f32;
        let h = 3.0;
        let gap = 3.0;
        let top = cy - (n * h + (n - 1.0) * gap) / 2.0;
        for (i, b) in bars.iter().take(4).enumerate() {
            p.bar(Rect::new(x, top + i as f32 * (h + gap), 44.0, h), b["part"].as_f64().unwrap_or(0.0) as f32, color_of(&b["color"], c));
        }
        return 44.0;
    }
    if let Some(img) = peek["image"].as_str() {
        if p.picture(img, Rect::new(x, y, size, size), size * 0.28) {
            return size;
        }
    }
    if let Some(v) = peek["bin"].as_object() {
        let part = v.get("part").and_then(|l| l.as_f64()).unwrap_or(0.0) as f32;
        let lid = v.get("lid").and_then(|l| l.as_bool()) == Some(true);
        let z = 22.0f32.max(size.min(30.0));
        let c = if v.get("amber").and_then(|a| a.as_bool()) == Some(true) { crate::draw::AMBER } else { crate::draw::accent() };
        p.bin(Rect::new(x, cy - z / 2.0, z, z), part, if lid { 1.0 } else { 0.0 }, c);
        return z;
    }
    if let Some(v) = peek["tank"].as_object() {
        let level = v.get("level").and_then(|l| l.as_f64()).unwrap_or(0.0) as f32;
        let z = 20.0f32.min(size.max(16.0));
        p.tank(Rect::new(x, cy - z / 2.0, z, z), level, v.get("charging").and_then(|c| c.as_bool()) == Some(true), v.get("full").and_then(|c| c.as_bool()) == Some(true), color_of(&peek["color"], GREEN));
        return z;
    }
    if let Some(v) = peek["vessel"].as_object() {
        let level = v.get("level").and_then(|l| l.as_f64()).unwrap_or(0.0) as f32;
        p.vessel(Rect::new(x, cy - 6.0, 26.0, 12.0), level, v.get("charging").and_then(|c| c.as_bool()) == Some(true), c);
        return 26.0;
    }
    if let Some(live) = peek["live"].as_object() {
        let tone = live.get("tone").and_then(|t| t.as_str()).unwrap_or("");
        let tc = tone_color(tone);
        let r = size / 2.0;
        // (rings round it, one in another: how much of each is used; its icon smaller, inside them)
        let rings = live.get("rings").and_then(|v| v.as_array()).filter(|a| !a.is_empty());
        // (one ring in two halves, a little gap at the top and the bottom between them: the first fills the right half from the top,
        // the second the left half from the bottom)
        if let Some(rings) = rings {
            const GAP: f32 = 0.03;
            let half = 0.5 - 2.0 * GAP;
            let none = crate::paint::Color(0.0, 0.0, 0.0, 0.0);
            for (n, ring) in rings.iter().take(2).enumerate() {
                let start = if n == 0 { GAP } else { 0.5 + GAP };
                let part = ring["part"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                p.ring(x + r, cy, r - 1.2, 1.6, half, start, FAINT, none);
                p.ring(x + r, cy, r - 1.2, 1.6, half * part, start, color_of(&ring["color"], accent()), none);
            }
        }
        let inner = if rings.is_some() { 0.52 } else { 1.0 };
        // (its own picture, when it names one: a GIF moves)
        if let Some(img) = live.get("image").and_then(|i| i.as_str()) {
            let z = size * 1.3 * inner;
            if p.picture(img, Rect::new(x + r - z / 2.0, cy - z / 2.0, z, z), 0.0) {
                return size;
            }
        }
        if rings.is_some() {
            if matches!(tone, "waiting" | "permission") {
                let ph = (p.t * 0.8).fract();
                p.circle(x + r, cy, r * 0.5 * (1.0 + 0.6 * ph), tc.alpha(0.35 * (1.0 - ph)));
            }
            let name = live.get("icon").and_then(|i| i.as_str()).unwrap_or(icon);
            let z = size * 0.5;
            // (finished: the star is green, its colour for it; no badge over the rings)
            p.icon(name, x + r - z / 2.0, cy - z / 2.0, z, if tone == "idle" { DIM } else { tc });
            return size;
        }
        // (waiting on you: a soft pulse round it)
        if matches!(tone, "waiting" | "permission") {
            let ph = (p.t * 0.8).fract();
            p.circle(x + r, cy, r * (1.0 + 0.45 * ph), tc.alpha(0.35 * (1.0 - ph)));
        }
        // (working: a part of the ring sweeps round)
        if tone == "working" {
            p.ring(x + r, cy, r - 1.0, 1.6, 0.28, (p.t * 0.9).fract(), tc, FAINT);
        } else {
            p.ring(x + r, cy, r - 1.0, 1.6, if tone == "idle" { 0.0 } else { 1.0 }, 0.0, tc, FAINT);
        }
        let name = live.get("icon").and_then(|i| i.as_str()).unwrap_or(icon);
        let z = size * 0.56;
        p.icon(name, x + r - z / 2.0, cy - z / 2.0, z, tc);
        return size;
    }
    if let Some(ring) = peek["ring"].as_object() {
        let part = ring.get("part").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let r = size / 2.0;
        let rc = if ring.get("amber") == Some(&Value::Bool(true)) { crate::draw::AMBER } else if ring.get("theme") == Some(&Value::Bool(true)) { crate::draw::accent() } else { c };
        // (a paler part after it: what counts toward it too)
        let extra = ring.get("extra").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        if extra > 0.0 {
            p.ring(x + r, cy, r - 1.0, 2.2, part + extra, 0.0, rc.alpha(0.4), FAINT);
            p.ring(x + r, cy, r - 1.0, 2.2, part, 0.0, rc, crate::paint::Color(0.0, 0.0, 0.0, 0.0));
        } else {
            p.ring(x + r, cy, r - 1.0, 2.2, part, 0.0, rc, FAINT);
        }
        if peek["icon"].is_string() {
            let z = size * 0.5;
            p.icon(icon, x + r - z / 2.0, cy - z / 2.0, z, c);
        }
        return size;
    }
    if peek["clock"].is_object() {
        return 0.0;
    }
    if let Some(g) = peek["roll"].as_object() {
        let w = roll_w(peek);
        roll(p, g, x, cy, w, size);
        return w;
    }
    if let Some(pad) = peek["pad"].as_object() {
        // (none told, wired: whole, faint)
        let level = pad.get("level").and_then(|l| l.as_f64());
        let fill = match level {
            Some(l) if l <= 0.25 => AMBER,
            Some(_) => GREEN,
            None => TEXT,
        };
        let z = size * 1.25;
        p.icon_fill(pad.get("icon").and_then(|i| i.as_str()).unwrap_or("gamepad"), x + size / 2.0 - z / 2.0, cy - z / 2.0, z, level.unwrap_or(1.0) as f32, fill, FAINT);
        return size;
    }
    p.icon(icon, x, y, size, c);
    size
}

/// the dock at its largest: the app in front on its card (its icon, or the cover of what it plays, its mark over it as any app's; one
/// line: what you are in, or the song; how far the song is along its foot)
pub const STAGE_W: f32 = 150.0;

/// How deep the dock stands (the even depth of its size, as the frame tells it).
fn dock_depth(peek: &Value) -> f32 {
    peek["dock"]["depth"].as_f64().map_or(32.0, |d| d as f32)
}

/// The dock's ends, past its first and last icon: its grips (a short bar in each while the mouse is on it, where a press takes hold of
/// it).
pub const DOCK_END: f32 = 14.0;

/// The dock's margins: toward the middle of the screen (thin), and toward its edge (its apps' marks: their windows' dots, their sound).
fn dock_margins(depth: f32) -> (f32, f32) {
    // (the same each side: its icons in the middle of its depth)
    let m = (depth * 0.15).round().clamp(4.0, 8.0);
    (m, m)
}

/// Its icons: what the depth leaves past its margins.
fn dock_icon(depth: f32) -> f32 {
    let (a, b) = dock_margins(depth);
    (depth - a - b).round().max(14.0)
}

/// Is this app on its card (the dock at its largest, the app in front)?
fn staged(peek: &Value, it: &Value) -> bool {
    peek["dock"]["large"] == true && (it["front"] == true || leaving_front(peek, it))
}

/// how far the app in front has come since it changed (0..1: its card grows, the last one's shrinks); and was this the last one?
fn front_k(peek: &Value) -> f32 {
    peek["dock"]["front_k"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32
}

fn leaving_front(peek: &Value, it: &Value) -> bool {
    front_k(peek) < 1.0 && it["front"] != true && !it["key"].is_null() && it["key"] == peek["dock"]["front_prev"]
}

/// How wide a card is now: the new one growing out of its icon, the last one shrinking back into its own.
fn stage_w(p: &Painter, peek: &Value, it: &Value) -> f32 {
    let k = front_k(peek);
    let e = 1.0 - (1.0 - k).powi(3);
    let icon = dock_icon(dock_depth(peek));
    let full = stage_full(p, it, icon);
    if it["front"] == true {
        icon + (full - icon) * e
    } else {
        full - (full - icon) * e
    }
}

/// A card's one line: what it plays, else what you are in (its window's title), else its name.
fn stage_line(it: &Value) -> String {
    let title = it["title"].as_str().unwrap_or("");
    if it["media"].is_object() {
        it["media"]["title"].as_str().unwrap_or("").to_string()
    } else if !title.is_empty() {
        title.to_string()
    } else {
        it["name"].as_str().unwrap_or("").to_string()
    }
}

/// How wide a card is at its widest: as long as its line (a short one, short), at most `STAGE_W`.
fn stage_full(p: &Painter, it: &Value, z: f32) -> f32 {
    let h = z + 4.0;
    let pz = (z - 6.0).max(14.0);
    let inset = (h - pz) / 2.0;
    let size = (h * 0.4).clamp(10.5, 12.5);
    let tw = p.measure(&stage_line(it), size, Weight::Semibold, STAGE_W);
    // (drawn bigger (the dock opening), its icon may be wider than a card at its widest: then as wide as its icon)
    (inset + pz + 8.0 + tw + 10.0).clamp(z, STAGE_W.max(z))
}

fn stage(p: &Painter, it: &Value, x: f32, cy: f32, w: f32, z: f32) -> Vec<(Rect, String)> {
    let h = z + 4.0;
    let r = Rect::new(x, cy - h / 2.0, w, h);
    let mut out = Vec::new();
    p.round(r, (h * 0.3).min(w / 2.0), crate::paint::Color(0.106, 0.106, 0.125, 1.0));
    unsafe {
        p.dc.PushAxisAlignedClip(&p.px(r).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
    }
    let out = stage_inner(p, it, r, cy, z, &mut out);
    unsafe {
        p.dc.PopAxisAlignedClip();
    }
    out
}

fn stage_inner(p: &Painter, it: &Value, r: Rect, cy: f32, z: f32, out: &mut Vec<(Rect, String)>) -> Vec<(Rect, String)> {
    let mut out = std::mem::take(out);
    // (its picture a little smaller than the card is deep, as far in from its left as from its top; growing out of its icon, it moves
    // in to its place as the card widens)
    let pz = (z - 6.0).max(14.0);
    let inset = ((r.w - pz) / 2.0).clamp(0.0, (r.h - pz) / 2.0);
    let r = Rect::new(r.x, r.y, stage_full(p, it, z), r.h);
    let media = &it["media"];
    let playing = media.is_object();
    // its picture: the cover of what it plays, else its icon
    let pic = Rect::new(r.x + inset, r.y + (r.h - pz) / 2.0, pz, pz);
    let cover = media["art"].as_str().unwrap_or("");
    let shown = playing && !cover.is_empty() && p.picture(cover, pic, 6.0);
    if !shown {
        if !p.picture(it["image"].as_str().unwrap_or(""), pic, pz * 0.22) {
            p.round(pic, pz * 0.3, SOFT);
            let letter: String = it["name"].as_str().unwrap_or("?").chars().take(1).collect();
            let w = p.measure(&letter, 15.0, Weight::Semibold, pz);
            p.text(&letter, pic.x + pz / 2.0 - w / 2.0, pic.y + pz / 2.0, 15.0, Weight::Semibold, pz, TEXT);
        }
    }
    // (its one mark over it, as every app's: over the cover of what it plays too)
    icon_mark(p, it, pic, if shown { 6.0 } else { pz * 0.22 });
    if playing {
        if let Some(src) = media["source"].as_str() {
            out.push((pic, format!("mediatoggle:{src}")));
        }
    }
    // its words, one line: what it plays, else what you are in (its window's title), else its name
    let tx = pic.right() + 8.0;
    let tw = r.right() - 10.0 - tx;
    let title = it["title"].as_str().unwrap_or("");
    let line = if playing {
        media["title"].as_str().unwrap_or("").to_string()
    } else if !title.is_empty() {
        title.to_string()
    } else {
        it["name"].as_str().unwrap_or("").to_string()
    };
    let size = (r.h * 0.4).clamp(10.5, 12.5);
    p.text(&line, tx, cy, size, Weight::Semibold, tw, TEXT);
    // (what it plays: how far along, a thin line along its foot)
    if let Some(f) = media["progress"].as_f64().map(|f| f as f32) {
        let yb = r.bottom() - 2.5;
        p.round(Rect::new(tx, yb - 1.0, tw, 2.0), 1.0, crate::paint::Color(0.149, 0.149, 0.173, 1.0));
        p.round(Rect::new(tx, yb - 1.0, tw * f.clamp(0.0, 1.0), 2.0), 1.0, crate::draw::accent());
    }
    out
}

/// Under an app with several windows: a dot for each (three at most), centred on `cx`, the one in front in the theme's colour.
fn window_dots(p: &Painter, it: &Value, cx: f32, cy: f32) {
    let n = it["windows"].as_u64().unwrap_or(0) as usize;
    if n < 2 {
        return;
    }
    let shown = n.min(3);
    let front = it["front_i"].as_u64().map(|i| (i as usize).min(shown - 1));
    let gap = 6.0;
    let x0 = cx - (shown - 1) as f32 * gap / 2.0;
    for i in 0..shown {
        let c = if Some(i) == front { crate::draw::accent() } else { crate::paint::Color(0.43, 0.43, 0.46, 1.0) };
        p.circle(x0 + i as f32 * gap, cy, 1.8, c);
    }
}

/// Has a dock app a mark over its icon (downloading, or making sound)?
fn has_mark(it: &Value) -> bool {
    !it["progress"].is_null() || it["audio"].is_array()
}

/// A dock app's one mark, over its picture (`pic`, rounded by `radius`): downloading, the picture dimmed and a ring over it filling
/// as the download goes; else making sound, dimmed and its sound's bars over its middle. A download comes first.
fn icon_mark(p: &Painter, it: &Value, pic: Rect, radius: f32) {
    if !has_mark(it) {
        return;
    }
    p.round(pic, radius, crate::paint::Color(0.0, 0.0, 0.0, 0.62));
    let (cx, cy) = (pic.x + pic.w / 2.0, pic.y + pic.h / 2.0);
    let z = pic.w.min(pic.h);
    if let Some(f) = it["progress"].as_f64() {
        p.ring(cx, cy, z * 0.34, (z * 0.09).clamp(1.8, 2.6), (f as f32).clamp(0.0, 1.0), 0.0, BLUE, TEXT.alpha(0.2));
    } else if let Some(v) = it["audio"].as_array() {
        sound_bars(p, v, cx, cy, (z * 0.42).max(7.0));
    }
}

/// The dock as its icon alone: the app in front on its ground, and in the band toward the middle of the screen a dot for each other
/// open app (three at most). Nothing in it to press: a press opens the dock.
fn dock_front(p: &Painter, d: &Value, body: Rect, horizontal: bool) {
    let inner = d["inner"].as_str().unwrap_or(if horizontal { "top" } else { "right" });
    let k = body.w.min(body.h);
    let band = (k * 0.25).round();
    let rest = match inner {
        "bottom" => Rect::new(body.x, body.y, body.w, body.h - band),
        "right" => Rect::new(body.x, body.y, body.w - band, body.h),
        "left" => Rect::new(body.x + band, body.y, body.w - band, body.h),
        _ => Rect::new(body.x, body.y + band, body.w, body.h - band),
    };
    let g = rest.w.min(rest.h) - 2.0;
    let (cx, cy) = (rest.x + rest.w / 2.0, rest.y + rest.h / 2.0);
    let ground = Rect::new(cx - g / 2.0, cy - g / 2.0, g, g);
    if d["front"] == true {
        p.round(ground, g * 0.3, crate::draw::accent().alpha(0.22));
    }
    let z = g - 6.0;
    let r = Rect::new(cx - z / 2.0, cy - z / 2.0, z, z);
    if !p.picture(d["image"].as_str().unwrap_or(""), r, z * 0.22) {
        p.round(r, z * 0.3, SOFT);
        let letter: String = d["name"].as_str().unwrap_or("?").chars().take(1).collect();
        let w = p.measure(&letter, 11.0, Weight::Semibold, z);
        p.text(&letter, cx - w / 2.0, cy, 11.0, Weight::Semibold, z, TEXT);
    }
    let n = d["others"].as_u64().unwrap_or(0).min(3) as usize;
    let c = if d["attention"] == true { crate::draw::AMBER } else { crate::paint::Color(0.43, 0.43, 0.46, 1.0) };
    for i in 0..n {
        let o = (i as f32 - (n - 1) as f32 / 2.0) * 5.0;
        let (x, y) = match inner {
            "bottom" => (cx + o, body.bottom() - band / 2.0),
            "right" => (body.right() - band / 2.0, cy + o),
            "left" => (body.x + band / 2.0, cy + o),
            _ => (cx + o, body.y + band / 2.0),
        };
        p.circle(x, y, 1.6, c);
    }
}

/// Beside an app on a side edge with several windows: a dot for each (three at most), one under the other, centred on `cy`.
fn window_dots_down(p: &Painter, it: &Value, cx: f32, cy: f32) {
    let n = it["windows"].as_u64().unwrap_or(0) as usize;
    if n < 2 {
        return;
    }
    let shown = n.min(3);
    let front = it["front_i"].as_u64().map(|i| (i as usize).min(shown - 1));
    let y0 = cy - (shown - 1) as f32 * 6.0 / 2.0;
    for i in 0..shown {
        let c = if Some(i) == front { crate::draw::accent() } else { crate::paint::Color(0.43, 0.43, 0.46, 1.0) };
        p.circle(cx, y0 + i as f32 * 6.0, 1.8, c);
    }
}

/// a dock app's meters (two thin bars, their gap, and the room to its icon), and its download's bar
pub const METERS: f32 = 13.0;
pub const PROGRESS: f32 = 8.0;

const PURPLE: crate::paint::Color = crate::paint::Color(0.71, 0.49, 0.9, 1.0);
const BLUE: crate::paint::Color = crate::paint::Color(0.31, 0.56, 0.94, 1.0);

/// What a program uses, as two thin bars standing up beside what shows it (`h` tall, from `x`, their foot at `foot`): its share of
/// the processor, then of the memory; only as tall as each is.
pub fn use_bars(p: &Painter, it: &Value, x: f32, foot: f32, h: f32) {
    let level = |v: f32| (h * v).clamp(2.0, h);
    let cpu = it["cpu"].as_f64().unwrap_or(0.0) as f32;
    let ram = it["ram"].as_f64().unwrap_or(0.0) as f32;
    for (j, (len, c)) in [(level((cpu / 0.4).min(1.0)), load_color(cpu)), (level((ram / 0.25).min(1.0)), PURPLE)].iter().enumerate() {
        p.round(Rect::new(x + j as f32 * 5.0, foot - len, 3.0, *len), 1.5, *c);
    }
}

/// How far a download is: a thin bar standing up, filling from its foot.
pub fn progress_bar(p: &Painter, f: f32, x: f32, foot: f32, h: f32) {
    p.round(Rect::new(x, foot - h, 3.0, h), 1.5, FAINT);
    p.round(Rect::new(x, foot - h * f.clamp(0.0, 1.0), 3.0, h * f.clamp(0.0, 1.0)), 1.5, BLUE);
}

/// The sound a program makes: three small bars that move (`v`: each 0..1), `w` wide, centred on `cx`, their middle at `cy`.
pub fn sound_bars(p: &Painter, v: &[Value], cx: f32, cy: f32, h: f32) {
    let c = crate::paint::Color(0.35, 0.78, 0.9, 1.0);
    let at = |i: usize| v.get(i).and_then(|b| b.as_f64()).unwrap_or(0.0) as f32;
    // (the same three bars as the sound's everywhere: as tall as it is loud, lifted as it swells)
    p.bars(cx - crate::draw::bars_w(h) / 2.0, cy, h, [at(0), at(1), at(2)], at(3), c);
}

/// How heavy a share of the processor reads: blue when light, amber when busy, red when heavy.
pub fn load_color(cpu: f32) -> crate::paint::Color {
    if cpu >= 0.25 {
        crate::paint::Color(0.91, 0.48, 0.48, 1.0)
    } else if cpu >= 0.08 {
        crate::paint::Color(0.91, 0.69, 0.29, 1.0)
    } else {
        crate::paint::Color(0.35, 0.78, 0.9, 1.0)
    }
}

/// The dock: the launcher's button, then each app (its picture; under a running one, a bar of what it uses). Where a click opens
/// it, a grip on its inner edge (lit when the mouse is on it).
fn dock(p: &Painter, peek: &Value, body: Rect, horizontal: bool) -> Vec<(Rect, String)> {
    let items = peek["dock"]["items"].as_array().cloned().unwrap_or_default();
    let hovered = peek["dock"]["hovered"].as_str().unwrap_or("");
    // (its icons fill its depth but for two thin margins: toward the middle of the screen, and toward its edge, where each app's marks
    // are (its windows' dots, its sound). Where a click opens widgets, a grip along its inner side while the mouse is on it, drawn over
    // it, taking no room)
    let inner = peek["dock"]["inner"].as_str().unwrap_or(if horizontal { "top" } else { "right" });
    let depth = if horizontal { body.h } else { body.w };
    let (m_in, m_out) = dock_margins(depth);
    let icon = dock_icon(depth);
    // (its side toward the screen's middle, and the way into the dock from it)
    let (side, into) = match inner {
        "bottom" => (body.bottom(), -1.0),
        "right" => (body.right(), -1.0),
        "left" => (body.x, 1.0),
        _ => (body.y, 1.0),
    };
    let band = match inner {
        "bottom" => Rect::new(body.x, body.bottom() - m_in, body.w, m_in),
        "right" => Rect::new(body.right() - m_in, body.y, m_in, body.h),
        "left" => Rect::new(body.x, body.y, m_in, body.h),
        _ => Rect::new(body.x, body.y, body.w, m_in),
    };
    let grip = peek["dock"]["grip"] == true;
    let mut out = Vec::new();
    let n = items.len().max(1) as f32;
    // (on a side edge, more apps than fit: each smaller, down to a size)
    let z = if horizontal { icon } else { ((body.h - 2.0 * DOCK_END - (n - 1.0) * DOCK_GAP) / n).clamp(14.0, icon) };
    // (past its ends, its grips)
    let mut at = if horizontal { body.x + DOCK_END } else { body.y + DOCK_END };
    // (on a side edge: a column of icons, as wide as every widget there; their names in their tips)
    let rows = false;
    // (the icons between the margins; the marks in the outer one)
    let cross = side + into * (m_in + (depth - m_in - m_out) / 2.0);
    let mark = side + into * (depth - m_out / 2.0);
    // (toward the middle of the screen: which way a hop goes)
    let inward = if inner == "bottom" || inner == "right" { 1.0 } else { -1.0 };
    let place = |at: f32| if horizontal { Rect::new(at, cross - z / 2.0, z, z) } else { Rect::new(cross - z / 2.0, at, z, z) };
    for (i, it) in items.iter().enumerate() {
        // (the app in front, at the dock's largest: on its stage)
        if staged(peek, it) && horizontal {
            let sw = stage_w(p, peek, it);
            for (r, a) in stage(p, it, at, cross, sw, z) {
                out.push((r, a));
            }
            out.push((Rect::new(at, cross - (z + 4.0) / 2.0, sw, z + 4.0), format!("app:{i}")));
            at += sw + DOCK_GAP;
            continue;
        }
        let large = peek["dock"]["large"] == true;
        let mut r = place(at);
        // (it wants you: a soft amber glow behind it, and now and then a little hop)
        if it["attention"] == true {
            let ph = (p.t * 0.62).fract();
            let hop = if ph > 0.6 { ((ph - 0.6) / 0.4 * std::f32::consts::PI * 2.0).sin().max(0.0) * 6.0 * (1.0 - (ph - 0.6) / 0.4) } else { 0.0 };
            if horizontal {
                r.y += hop * inward;
            } else {
                r.x += hop * inward;
            }
            // (a soft light behind it, breathing: many faint discs, most where they overlap, at its middle)
            let (cx, cy) = (r.x + z / 2.0, r.y + z / 2.0);
            let breath = 0.7 + 0.3 * (p.t * 2.4).sin();
            // (in the colour of its own icon, when it has one; else amber)
            let tint = it["image"].as_str().and_then(|i| crate::images::tone(p.pc, i)).map(|(r, g, b)| crate::paint::Color(r, g, b, 1.0)).unwrap_or(crate::draw::AMBER);
            const RINGS: usize = 12;
            for i in 0..RINGS {
                let k = i as f32 / (RINGS - 1) as f32;
                p.circle(cx, cy + hop * 0.3, z * (0.42 + 0.5 * k), tint.alpha(0.055 * breath));
            }
            // (and now and then a ring goes out from it, fading)
            let ping = (p.t / 1.6).fract();
            if ping < 0.7 {
                let k = ping / 0.7;
                let none = crate::paint::Color(0.0, 0.0, 0.0, 0.0);
                p.ring(cx, cy, z * (0.55 + 0.5 * k), 1.6 * (1.0 - k) + 0.4, 1.0, 0.0, tint.alpha(0.55 * (1.0 - k).powf(1.5)), none);
            }
        }
        // (as a row: the one in front grounded the whole row, its window's title under its name; the one under the mouse, grey)
        let row = match inner {
            "left" => Rect::new(band.right() + 2.0, r.y - 4.0, body.right() - 4.0 - band.right() - 2.0, z + 8.0),
            _ => Rect::new(body.x + 4.0, r.y - 4.0, band.x - 2.0 - body.x - 4.0, z + 8.0),
        };
        if rows {
            if it["front"] == true {
                p.round(row, 11.0, crate::draw::accent().alpha(0.2));
            } else if it["key"].as_str() == Some(hovered) {
                p.round(row, 11.0, FAINT);
            }
            let tx = r.right() + 10.0;
            let tw = row.right() - 8.0 - tx;
            let name = it["name"].as_str().unwrap_or("");
            let title = if it["front"] == true { it["title"].as_str().unwrap_or("") } else { "" };
            let cy = r.y + z / 2.0;
            if title.is_empty() {
                p.text(name, tx, cy, 12.5, Weight::Semibold, tw, TEXT);
            } else {
                p.text(name, tx, cy - 7.0, 12.5, Weight::Semibold, tw, TEXT);
                p.text(title, tx, cy + 8.0, 11.0, Weight::Regular, tw, DIM);
            }
        }
        // (in the band beside it, when nothing is over its icon: several windows, a dot for each, the one in front lit; on a side
        // edge, the dots stand in a column)
        let (mx, my) = if horizontal { (r.x + z / 2.0, mark) } else { (mark, r.y + z / 2.0) };
        if !has_mark(it) {
            if horizontal {
                window_dots(p, it, mx, my);
            } else {
                window_dots_down(p, it, mx, my);
            }
        }
        // (the app in front: a ground of the theme's colour behind it; the one under the mouse, a grey one)
        // (as it comes to the front: its ground pops in, a little too far, and settles; the last one's fades)
        let fk = front_k(peek);
        if it["front"] == true || leaving_front(peek, it) {
            let (sc, a) = if it["front"] == true {
                let t = fk - 1.0;
                (0.6 + 0.4 * (1.0 + 2.70158 * t * t * t + 1.70158 * t * t), fk.min(1.0))
            } else {
                (1.0, 1.0 - fk)
            };
            let (gw, gh) = ((r.w + 8.0) * sc, (r.h + 8.0) * sc);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            p.round(Rect::new(cx - gw / 2.0, cy - gh / 2.0, gw, gh), gh * 0.3, crate::draw::accent().alpha(0.22 * a));
        } else if it["key"].as_str() == Some(hovered) && !rows {
            p.round(Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0), (z + 6.0) * 0.3, FAINT);
        }
        let img = it["image"].as_str().unwrap_or("");
        // (its picture a little inside its place, clear of the ground behind it (more so large); the place, and all round it, as it
        // was)
        let pin = if large { 3.0 } else { 1.0 };
        let pic = Rect::new(r.x + pin, r.y + pin, r.w - 2.0 * pin, r.h - 2.0 * pin);
        if !p.picture(img, pic, pic.w * 0.22) {
            p.round(pic, pic.w * 0.3, SOFT);
            let name = it["name"].as_str().unwrap_or("?");
            let letter: String = name.chars().take(1).collect();
            let w = p.measure(&letter, 13.0, Weight::Semibold, z);
            p.text(&letter, r.x + z / 2.0 - w / 2.0, r.y + z / 2.0, 13.0, Weight::Semibold, z, TEXT);
        }
        // (downloading, or making sound: its one mark over it)
        icon_mark(p, it, pic, pic.w * 0.22);
        // (Win held: its number, under it)
        if peek["dock"]["numbers"] == true && i < 9 {
            let t = format!("{}", i + 1);
            let k = if horizontal { Rect::new(r.x + z / 2.0 - 8.0, mark - 7.0, 16.0, 14.0) } else { Rect::new(mark - 7.0, r.y + z / 2.0 - 7.0, 14.0, 14.0) };
            p.round(k, 4.0, TEXT.alpha(0.9));
            let tw = p.measure(&t, 9.5, Weight::Bold, 16.0);
            p.text(&t, k.x + k.w / 2.0 - tw / 2.0, k.y + 7.0, 9.5, Weight::Bold, 16.0, crate::paint::Color(0.0, 0.0, 0.0, 1.0));
        }
        out.push((if rows { row } else { r }, format!("app:{i}")));
        at += z + DOCK_GAP;
    }
    // (its grips: a small dot in the middle of each end while the mouse is on it; the one the mouse is over, brighter and a touch
    // bigger)
    if grip {
        let c = side + into * depth / 2.0;
        let lit = peek["dock"]["grip_at"].as_u64();
        for (i, e) in [DOCK_END / 2.0, if horizontal { body.w } else { body.h } - DOCK_END / 2.0].into_iter().enumerate() {
            let on = lit == Some(i as u64);
            let (cx, cy) = if horizontal { (body.x + e, c) } else { (c, body.y + e) };
            p.circle(cx, cy, if on { 3.0 } else { 2.5 }, if on { TEXT.alpha(0.9) } else { DIM.alpha(0.7) });
        }
    }
    out
}

/// The peek of a widget shown as its icon alone: what leads it; no words, no marks (something new, it glows instead). (A peek with
/// nothing to lead it stays as it is.)
pub fn icon_only(peek: &Value) -> Value {
    // (a calendar, its icon alone: the day as a little page)
    if peek["page"].is_object() {
        return serde_json::json!({ "page": peek["page"] });
    }
    // (a voice, its icon alone: its bars alone)
    if peek["voice"].is_object() {
        let mut o = peek.clone();
        o["voice"]["alone"] = serde_json::json!(true);
        return o;
    }
    // (the desktops, their icon alone: the one shown, its tiny screen alone)
    if peek["desks"].is_object() {
        let mut o = peek.clone();
        o["desks"]["alone"] = serde_json::json!(true);
        return o;
    }
    // (drives, their icon alone: the one being read or written, else the fullest, its letter in its ring)
    if let Some(items) = peek["drives"]["items"].as_array() {
        let first = peek["drives"]["first"].as_str().unwrap_or("");
        let one = items.iter().find(|i| i["letter"].as_str() == Some(first)).or(items.first()).cloned().unwrap_or(Value::Null);
        return serde_json::json!({ "drives": { "items": [one] } });
    }
    // (devices, their icon alone: the lowest one, in its ring)
    if let Some(first) = peek["devs"]["items"].as_array().and_then(|a| a.first()) {
        let mut o = serde_json::json!({ "icon": first["icon"] });
        if let Some(l) = first["level"].as_f64() {
            o["ring"] = serde_json::json!({ "part": l, "theme": first["low"] != true, "amber": first["low"] == true });
        }
        return o;
    }
    // (a volume, its icon alone: its icon in a ring as full as it is loud; muted, red)
    if peek["level"].is_object() {
        let l = &peek["level"];
        let muted = l["muted"] == true;
        return serde_json::json!({ "icon": peek["icon"], "ring": { "part": if muted { 0.0 } else { l["value"].as_f64().unwrap_or(0.0) }, "theme": true }, "color": if muted { serde_json::json!("#e87a7a") } else { Value::Null } });
    }
    // (a song, its icon alone: its bars alone)
    if peek["song"].is_object() {
        let mut o = peek.clone();
        o["song"]["alone"] = serde_json::json!(true);
        return o;
    }
    // (graphs, their icon alone: a bar for each, as full as it is now)
    if let Some(items) = peek["graphs"]["items"].as_array() {
        let bars: Vec<Value> = items.iter().map(|it| serde_json::json!({ "part": graph_vals(it).last().copied().unwrap_or(0.0), "color": it["color"] })).collect();
        return serde_json::json!({ "meters": bars });
    }
    // (the tray, its icon alone: its first four icons, small, in a square)
    if let Some(items) = peek["tray"]["items"].as_array() {
        let firsts: Vec<Value> = items.iter().take(4).map(|i| serde_json::json!({ "image": i["image"] })).collect();
        return serde_json::json!({ "tray_grid": { "items": firsts } });
    }
    // (the dock, its icon alone: the app in front, and how many others are open)
    if let Some(items) = peek["dock"]["items"].as_array() {
        let open = |i: &&Value| i["windows"].as_u64().unwrap_or(0) > 0 || !i["cpu"].is_null();
        let front = items.iter().find(|i| i["front"] == true).or_else(|| items.iter().find(open)).or(items.first()).cloned().unwrap_or(Value::Null);
        let others = items.iter().filter(open).filter(|i| i["key"] != front["key"] || front["key"].is_null()).count();
        let attention = items.iter().any(|i| i["attention"] == true);
        return serde_json::json!({ "dock_front": { "image": front["image"], "name": front["name"], "front": front["front"] == true, "others": others, "attention": attention } });
    }
    if peek["tray"].is_object() || peek["face"].is_string() {
        return peek.clone();
    }
    let lead = ["icon", "image", "live", "ring", "vessel", "tank", "bin", "bars", "pad", "roll", "signal"];
    if !lead.iter().any(|k| !peek[*k].is_null()) {
        return peek.clone();
    }
    let mut o = serde_json::Map::new();
    // (its words alone: they stay, small)
    let keep: &[&str] = if bare(peek) { &["color", "label", "text"] } else { &["color", "label"] };
    for k in lead.iter().chain(keep.iter()) {
        if !peek[*k].is_null() {
            o.insert(k.to_string(), peek[*k].clone());
        }
    }
    // (a picture of its own for its icon alone, when it gives one: a mark that fits a square better)
    if let Some(img) = peek["image_icon"].as_str().filter(|i| !i.is_empty()) {
        o.insert("image".into(), Value::String(img.to_string()));
    }
    Value::Object(o)
}

fn draw_at(p: &Painter, peek: &Value, body: Rect, horizontal: bool, dot: bool) -> Vec<(Rect, String)> {
    if peek["voice"].is_object() {
        voice(p, peek, body, horizontal);
        return Vec::new();
    }
    if peek["level"].is_object() {
        level(p, peek, body, horizontal);
        return Vec::new();
    }
    if peek["song"].is_object() {
        // (on a side edge: its bars alone)
        if !horizontal && peek["song"]["alone"] != true {
            let mut o = peek.clone();
            o["song"]["alone"] = serde_json::json!(true);
            song(p, &o, body);
        } else {
            song(p, peek, body);
        }
        return Vec::new();
    }
    if peek["meters"].is_array() {
        meters(p, peek, body, horizontal);
        return Vec::new();
    }
    if peek["graphs"].is_object() && horizontal {
        graphs(p, peek, body);
        return Vec::new();
    }
    if peek["face"].is_string() {
        return Vec::new();
    }
    if peek["subtitle"].is_object() {
        subtitle(p, &peek["subtitle"], body);
        return Vec::new();
    }
    if peek["dock_front"].is_object() {
        dock_front(p, &peek["dock_front"], body, horizontal);
        return Vec::new();
    }
    if let Some(items) = peek["tray_grid"]["items"].as_array() {
        tray_grid(p, items, body);
        return Vec::new();
    }
    if peek["dock"].is_object() {
        return dock(p, peek, body, horizontal);
    }
    if peek["tray"].is_object() {
        return tray(p, peek, body, horizontal);
    }
    let c = color_of(&peek["color"], TEXT);
    // (its words alone, shrunk: the words, small, in the middle)
    if dot && bare(peek) {
        let (text, _) = words_of(peek);
        let w = p.measure(&text, 11.0, Weight::Semibold, body.w - 4.0);
        p.text(&text, body.x + body.w / 2.0 - w / 2.0, body.y + body.h / 2.0, 11.0, Weight::Semibold, body.w - 4.0, c);
        return Vec::new();
    }
    if dot {
        let z = (body.w.min(body.h) * 0.5).min(LEAD);
        // (a dial wants the room: as big as the piece lets it be)
        // (a rolling number: its digits as big as the piece lets them be, in its middle)
        if let Some(g) = peek["roll"].as_object() {
            let h = ((body.h.min(body.w) - 12.0) * 0.55).clamp(10.0, 15.0) * 0.76;
            roll(p, g, body.x + 2.0, body.y + body.h / 2.0, body.w - 4.0, h);
            return Vec::new();
        }
        lead(p, &serde_json::json!({ "icon": peek["icon"], "image": peek["image"], "live": peek["live"], "ring": peek["ring"], "color": peek["color"], "pad": peek["pad"], "signal": peek["signal"] }), body.x + body.w / 2.0 - z / 2.0, body.y + body.h / 2.0, z);
        return Vec::new();
    }
    if peek["desks"].is_object() {
        return desks(p, peek, body, horizontal);
    }
    let (text, sub) = words_of(peek);
    if !horizontal {
        let cx = body.x + body.w / 2.0;
        let mut y = body.y + 10.0;
        if peek["bars"].is_array() {
            // (the bars stand up on a vertical edge)
            let bars = peek["bars"].as_array().unwrap();
            let n = bars.len().clamp(1, 4) as f32;
            let left = cx - (n * 4.0 + (n - 1.0) * 3.0) / 2.0;
            for (i, b) in bars.iter().take(4).enumerate() {
                let r = Rect::new(left + i as f32 * 7.0, y, 4.0, LEAD + 10.0);
                p.round(r, 2.0, FAINT);
                let part = b["part"].as_f64().unwrap_or(0.0) as f32;
                let h = (r.h * part.clamp(0.0, 1.0)).max(4.0);
                p.round(Rect::new(r.x, r.bottom() - h, r.w, h), 2.0, color_of(&b["color"], c));
            }
            y += LEAD + 10.0;
        } else if bare(peek) {
            y = body.y + body.h / 2.0 - 11.0;
        } else {
            let z = lead_size(peek);
            // (its icon alone: in the middle of its piece)
            if text.is_empty() {
                y = body.y + (body.h - z) / 2.0;
            }
            // (drives on a side edge: the one that matters, alone; a row of them does not fit across)
            if peek["drives"].is_object() {
                let one = icon_only(peek);
                lead(p, &one, cx - DEV_RING / 2.0, y + z / 2.0, z);
            } else {
                lead(p, peek, cx - z / 2.0, y + z / 2.0, z);
            }
            y += z;
        }
        if !text.is_empty() {
            let max = body.w - 6.0;
            let w = p.measure(&text, 10.5, Weight::Semibold, max);
            p.text(&text, cx - w / 2.0, y + 4.0 + 7.0, 10.5, Weight::Semibold, max, TEXT);
        }
        return Vec::new();
    }
    let cy = body.y + body.h / 2.0;
    let z = lead_size(peek);
    // (an icon alone sits in the middle of its piece, which is never narrower than a short pill)
    let alone = text.is_empty() && sub.is_empty() && peek["viz"].as_bool() != Some(true) && !peek["button"].is_object() && peek["dot"].as_bool() != Some(true) && !peek["cells"].is_object();
    // (bars alone, in a square: they stand up, side by side, in its middle)
    if alone {
        if let Some(bars) = peek["bars"].as_array() {
            let n = bars.len().clamp(1, 4) as f32;
            let (bw, gap) = (4.0, 3.0);
            let h = (body.h - 12.0).clamp(10.0, LEAD + 4.0);
            let left = body.x + body.w / 2.0 - (n * bw + (n - 1.0) * gap) / 2.0;
            for (i, b) in bars.iter().take(4).enumerate() {
                let r = Rect::new(left + i as f32 * (bw + gap), cy - h / 2.0, bw, h);
                p.round(r, 2.0, FAINT);
                let part = b["part"].as_f64().unwrap_or(0.0) as f32;
                let fill = (r.h * part.clamp(0.0, 1.0)).max(bw);
                p.round(Rect::new(r.x, r.bottom() - fill, r.w, fill), 2.0, color_of(&b["color"], c));
            }
            return Vec::new();
        }
    }
    let lead_width = lead_w(peek);
    let pad = if peek["clock"].is_object() && lead_width == 0.0 { CLOCK_PAD } else { PAD };
    let mut x = if alone && lead_width > 0.0 { body.x + (body.w - lead_width) / 2.0 } else { body.x + pad };
    let lw = lead(p, peek, x, cy, z);
    if lw > 0.0 {
        x += lw + GAP;
    }
    let after_lead = x;
    let right_room = if peek["button"].is_object() { GAP + 20.0 } else { 0.0 } + pad;
    if !text.is_empty() {
        let max = (body.right() - right_room - x).clamp(0.0, MAX_TEXT);
        x += p.text(&text, x, cy, TEXT_SIZE, Weight::Semibold, max, TEXT);
    }
    if !sub.is_empty() {
        x += 6.0;
        x += p.text(&sub, x, cy, TEXT_SIZE - 1.5, Weight::Regular, 80.0, DIM);
    }
    if peek["viz"].as_bool() == Some(true) {
        x += GAP;
        x += p.viz(x, cy, 12.0, p.viz, c);
    }
    if peek["dot"].as_bool() == Some(true) {
        x += GAP;
        // (breathing, when it asks: a soft ring round it, swelling and fading)
        if peek["dot_pulse"] == true {
            let k = (p.t * 1.4).fract();
            p.circle(x + 3.0, cy, 3.0 + 4.0 * k, c.alpha(0.35 * (1.0 - k)));
            crate::draw::keep_moving();
        }
        p.circle(x + 3.0, cy, 3.0, c);
        x += 6.0;
    }
    if let Some(cl) = peek["cells"].as_object() {
        // (a gap from what came before it: the lead's is there already)
        if x > after_lead || lw == 0.0 {
            x += GAP;
        }
        cells(p, cl.get("icon").and_then(|i| i.as_str()).unwrap_or("bolt"), cl.get("level").and_then(|l| l.as_f64()), x, cy);
    }
    let _ = x;
    if let Some(b) = peek["button"].as_object() {
        let r = Rect::new(body.right() - PAD - 20.0, cy - 10.0, 20.0, 20.0);
        p.circle(r.x + 10.0, cy, 10.0, FAINT);
        let icon = b.get("icon").and_then(|i| i.as_str()).unwrap_or("dot");
        p.icon(icon, r.x + 3.0, cy - 7.0, 14.0, TEXT);
        return vec![(r, b.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string())];
    }
    Vec::new()
}
