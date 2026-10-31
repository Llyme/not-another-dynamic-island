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
    CLOCK_PAD * 2.0 + tw.max(dw)
}

fn draw_two_rows(p: &Painter, peek: &Value, body: Rect) {
    let (t, _) = clock_text(peek["clock"]["h24"].as_bool() == Some(true));
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
    use chrono::{Datelike, Timelike};
    let (h, m) = (now.hour(), now.minute());
    let time = if h24 {
        format!("{h:02}:{m:02}")
    } else {
        format!("{}:{m:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
    };
    let date = format!("{} {}", now.format("%a"), now.day());
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
    } else if peek["roll"].is_object() {
        roll_w(peek)
    } else if peek["clock"].is_object() && peek["image"].is_null() {
        0.0
    } else {
        lead_size(peek)
    }
}

/// Its words alone: its icon is none ("").
fn bare(peek: &Value) -> bool {
    peek["icon"] == "" && peek["image"].is_null() && peek["live"].is_null() && peek["ring"].is_null() && peek["vessel"].is_null() && peek["bars"].is_null() && peek["pad"].is_null() && peek["roll"].is_null()
}

/// The virtual desktops: a little box each, side by side (one over another on a vertical edge), the one shown lit. Each pressed
/// shows its desktop.
const DESK_W: f32 = 16.0;
const DESK_H: f32 = 11.0;
const DESK_GAP: f32 = 5.0;

fn desks(p: &Painter, peek: &Value, body: Rect, horizontal: bool) -> Vec<(Rect, String)> {
    let n = peek["desks"]["n"].as_u64().unwrap_or(1) as usize;
    let at = peek["desks"]["at"].as_u64().unwrap_or(0) as usize;
    let mut out = Vec::new();
    let (cx, cy) = (body.x + body.w / 2.0, body.y + body.h / 2.0);
    let along = n as f32 * if horizontal { DESK_W } else { DESK_H } + (n as f32 - 1.0) * DESK_GAP;
    for i in 0..n {
        let r = if horizontal {
            Rect::new(cx - along / 2.0 + i as f32 * (DESK_W + DESK_GAP), cy - DESK_H / 2.0, DESK_W, DESK_H)
        } else {
            Rect::new(cx - DESK_W / 2.0, cy - along / 2.0 + i as f32 * (DESK_H + DESK_GAP), DESK_W, DESK_H)
        };
        p.round(r, 3.0, if i == at { TEXT } else { FAINT });
        out.push((r, format!("desk:{i}")));
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

/// the tray, large: each icon its own size, its word under it
const TRAY_GAP_LARGE: f32 = 16.0;
const TRAY_WORD: f32 = 9.5;
const TRAY_WORD_MAX: f32 = 70.0;

fn tray_item_w(p: &Painter, it: &Value) -> f32 {
    let word = it["word"].as_str().unwrap_or("");
    TRAY_ICON.max(if word.is_empty() { 0.0 } else { p.measure(word, TRAY_WORD, Weight::Regular, TRAY_WORD_MAX) })
}

const DOCK_ICON: f32 = 30.0;
const TRAY_ICON: f32 = 18.0;
const TRAY_GAP: f32 = 10.0;

/// The tray at rest: its first icons (each pressed for its app), then how many more.
fn tray(p: &Painter, peek: &Value, body: Rect, horizontal: bool) -> Vec<(Rect, String)> {
    let items = peek["tray"]["items"].as_array().cloned().unwrap_or_default();
    let more = peek["tray"]["more"].as_u64().unwrap_or(0);
    let mut out = Vec::new();
    let z = TRAY_ICON;
    let mut at = if horizontal { body.x + 12.0 } else { body.y + 12.0 };
    let cross = if horizontal { body.y + body.h / 2.0 } else { body.x + body.w / 2.0 };
    let place = |at: f32, w: f32| if horizontal { Rect::new(at, cross - z / 2.0, w, z) } else { Rect::new(cross - w / 2.0, at, w, z) };
    let split = peek["tray"]["split"].as_u64().unwrap_or(0) as usize;
    // (large: each icon its own size, with what its app says under it)
    if peek["tray"]["large"] == true && horizontal {
        let mut at = body.x + 14.0;
        let cy = body.y + body.h / 2.0;
        for it in &items {
            let w = tray_item_w(p, it);
            let r = Rect::new(at + w / 2.0 - z / 2.0, cy - z / 2.0 - 7.0, z, z);
            if !p.picture(it["image"].as_str().unwrap_or(""), r, 3.0) {
                p.round(r, 5.0, SOFT);
            }
            let word = it["word"].as_str().unwrap_or("");
            if !word.is_empty() {
                let ww = p.measure(word, TRAY_WORD, Weight::Regular, TRAY_WORD_MAX);
                p.text(word, at + w / 2.0 - ww / 2.0, cy + 11.0, TRAY_WORD, Weight::Regular, TRAY_WORD_MAX, DIM);
            }
            out.push((Rect::new(at - 4.0, cy - z / 2.0 - 11.0, w + 8.0, z + 22.0), it["action"].as_str().unwrap_or("").to_string()));
            at += w + TRAY_GAP_LARGE;
        }
        if more > 0 {
            p.icon("dots", at + z / 2.0 - 8.0, cy - 8.0, 16.0, DIM);
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
        if !p.picture(it["image"].as_str().unwrap_or(""), r, 3.0) {
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
pub const LARGE: f32 = 1.375;

/// How long the body of a peek is along its edge (logical px); a large one's (its `scale`), that much longer.
pub fn measure(p: &Painter, peek: &Value, horizontal: bool) -> f32 {
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
        if !p.picture(it["image"].as_str().unwrap_or(""), r, 2.5) {
            p.round(r, 3.0, SOFT);
        }
    }
}

fn measure_at(p: &Painter, peek: &Value, horizontal: bool) -> f32 {
    if peek["tray_grid"].is_object() {
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
            return 14.0 * 2.0 + ws + (n - 1.0).max(0.0) * TRAY_GAP_LARGE + more;
        }
        return 12.0 * 2.0 + n * TRAY_ICON + (n - 1.0).max(0.0) * TRAY_GAP + more;
    }
    if let Some(n) = peek["desks"]["n"].as_u64() {
        let n = n as f32;
        return 12.0 * 2.0 + n * if horizontal { DESK_W } else { DESK_H } + (n - 1.0) * DESK_GAP;
    }
    if let Some(items) = peek["dock"]["items"].as_array() {
        // (the launcher's button, then each app: a running one with its meters beside it)
        let n = items.len() as f32 + 1.0;
        let _ = horizontal;
        let sides: f32 = items.iter().filter(|i| !staged(peek, i)).map(|i| side_room(i)).sum();
        let stages: f32 = items.iter().filter(|i| staged(peek, i)).map(|_| STAGE_W - DOCK_ICON).sum();
        return 12.0 * 2.0 + n * DOCK_ICON + (n - 1.0) * DOCK_GAP + 8.0 + sides + stages;
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
        p.ring(x + r, cy, r - 1.0, 2.2, part, 0.0, c, FAINT);
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

/// The room beside a dock app for its meters: its use on the left (running), how far its download is on the right.
fn side_room(it: &Value) -> f32 {
    (if it["cpu"].is_null() { 0.0 } else { METERS }) + (if it["progress"].is_null() { 0.0 } else { PROGRESS })
}

/// the dock at its largest: the app in front on a stage (its name, its sound, what it used lately, and now)
pub const STAGE_W: f32 = 14.0 + 200.0 + 14.0;
pub const STAGE_H: f32 = 84.0;

/// Is this app on the stage (the dock at its largest, the app in front)?
fn staged(peek: &Value, it: &Value) -> bool {
    peek["dock"]["large"] == true && it["front"] == true
}

fn stage(p: &Painter, it: &Value, x: f32, cy: f32) -> Vec<(Rect, String)> {
    let r = Rect::new(x, cy - STAGE_H / 2.0, STAGE_W, STAGE_H);
    p.round(r, 18.0, FAINT);
    // its name, its sound
    let tx = r.x + 14.0;
    let tw = 200.0;
    let z = 22.0;
    if !p.picture(it["image"].as_str().unwrap_or(""), Rect::new(tx, r.y + 10.0, z, z), 6.0) {
        p.round(Rect::new(tx, r.y + 10.0, z, z), 6.0, SOFT);
    }
    let sw = if it["audio"].is_array() { 20.0 } else { 0.0 };
    p.text(it["name"].as_str().unwrap_or(""), tx + z + 8.0, r.y + 10.0 + z / 2.0, 13.0, Weight::Semibold, tw - z - 8.0 - sw, TEXT);
    if let Some(v) = it["audio"].as_array() {
        sound_bars(p, v, tx + tw - 8.0, r.y + 10.0 + z / 2.0, 11.0);
    }
    // what it used lately: two lines, its share of the processor (the colour of how heavy it is now) and of the memory
    let hist: Vec<(f32, f32)> = it["hist"].as_array().map(|a| a.iter().map(|v| (v[0].as_f64().unwrap_or(0.0) as f32, v[1].as_f64().unwrap_or(0.0) as f32)).collect()).unwrap_or_default();
    let (gy, gh) = (r.y + 38.0, 22.0);
    let n = 60usize;
    let step = tw / (n - 1) as f32;
    let first = n.saturating_sub(hist.len());
    let at = |i: usize, v: f32, full: f32| (tx + (first + i) as f32 * step, gy + gh - gh * (v / full).min(1.0));
    let cpu_line: Vec<(f32, f32)> = hist.iter().enumerate().map(|(i, (c, _))| at(i, *c, 0.4)).collect();
    let ram_line: Vec<(f32, f32)> = hist.iter().enumerate().map(|(i, (_, m))| at(i, *m, 0.5)).collect();
    p.line(&ram_line, 1.6, PURPLE);
    p.line(&cpu_line, 1.6, load_color(it["cpu"].as_f64().unwrap_or(0.0) as f32));
    // now: its share of the processor, its memory
    let cpu = it["cpu"].as_f64().unwrap_or(0.0) as f32;
    let a = format!("{:.0}%", cpu * 100.0);
    let aw = p.text(&a, tx, r.bottom() - 13.0, 12.0, Weight::Semibold, 60.0, load_color(cpu));
    p.text("CPU", tx + aw + 4.0, r.bottom() - 13.0, 10.5, Weight::Regular, 40.0, DIM);
    let mem = it["mem_text"].as_str().unwrap_or("");
    p.text(mem, tx + 70.0, r.bottom() - 13.0, 12.0, Weight::Semibold, 90.0, PURPLE);
    Vec::new()
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
    for (i, b) in v.iter().take(3).enumerate() {
        let len = (h * b.as_f64().unwrap_or(0.0) as f32).clamp(2.0, h);
        p.round(Rect::new(cx - 6.5 + i as f32 * 5.0, cy - len / 2.0, 3.0, len), 1.5, c);
    }
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
    // (the grip: on the edge away from the screen's)
    if peek["dock"]["grip"] == true {
        let (gw, gh) = (34.0, 4.0);
        let c = TEXT.alpha(0.22);
        let at = peek["dock"]["grip_at"].as_str().unwrap_or("top");
        let g = match at {
            "bottom" => Rect::new(body.x + body.w / 2.0 - gw / 2.0, body.bottom() - 3.0 - gh, gw, gh),
            _ => Rect::new(body.x + body.w / 2.0 - gw / 2.0, body.y + 3.0, gw, gh),
        };
        p.round(g, 2.0, c);
    }
    let mut out = Vec::new();
    let z = DOCK_ICON;
    let mut at = if horizontal { body.x + 12.0 } else { body.y + 12.0 };
    // (a grip on the inner edge: the row clear of it)
    let cross = match peek["dock"]["grip_at"].as_str() {
        _ if !horizontal => body.x + body.w / 2.0,
        Some("top") if peek["dock"]["grip"].is_boolean() && items.iter().any(|i| staged(peek, i)) => body.y + 14.0 + STAGE_H / 2.0,
        Some("top") if peek["dock"]["grip"].is_boolean() => body.y + 14.0 + z / 2.0,
        Some("bottom") if peek["dock"]["grip"].is_boolean() => body.bottom() - 14.0 - z / 2.0,
        _ => body.y + body.h / 2.0,
    };
    let place = |at: f32| if horizontal { Rect::new(at, cross - z / 2.0, z, z) } else { Rect::new(cross - z / 2.0, at, z, z) };
    // the launcher
    let r = place(at);
    p.round(r, z * 0.3, FAINT);
    p.icon("grid", r.x + z * 0.22, r.y + z * 0.22, z * 0.56, TEXT);
    out.push((r, "launcher".to_string()));
    at += z + DOCK_GAP + 4.0;
    for (i, it) in items.iter().enumerate() {
        // (the app in front, at the dock's largest: on its stage)
        if staged(peek, it) && horizontal {
            for (r, a) in stage(p, it, at, cross) {
                out.push((r, a));
            }
            out.push((Rect::new(at, cross - STAGE_H / 2.0, STAGE_W, STAGE_H), format!("app:{i}")));
            at += STAGE_W + DOCK_GAP;
            continue;
        }
        // (running: its use on its left; its download on its right; only as tall as each is)
        let meters = !it["cpu"].is_null();
        let downloading = !it["progress"].is_null();
        // (a little shorter than the icon at both ends)
        let foot = cross + z / 2.0 - 3.0;
        if meters && horizontal {
            use_bars(p, it, at, foot, z - 6.0);
        }
        if meters {
            at += METERS;
        }
        let mut r = place(at);
        // (it wants you: a soft amber glow behind it, and now and then a little hop)
        if it["attention"] == true {
            let ph = (p.t * 0.62).fract();
            let hop = if ph > 0.6 { ((ph - 0.6) / 0.4 * std::f32::consts::PI * 2.0).sin().max(0.0) * 6.0 * (1.0 - (ph - 0.6) / 0.4) } else { 0.0 };
            if horizontal {
                r.y -= hop;
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
        if downloading && horizontal {
            progress_bar(p, it["progress"].as_f64().unwrap_or(0.0) as f32, at + z + 5.0, foot, z - 6.0);
        }
        // (making sound: small bars under it, moving with it)
        if let (Some(v), true) = (it["audio"].as_array(), horizontal) {
            sound_bars(p, v, r.x + z / 2.0, r.bottom() + 6.5, 8.0);
        }
        if it["key"].as_str() == Some(hovered) {
            p.round(Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0), (z + 6.0) * 0.3, FAINT);
        }
        let img = it["image"].as_str().unwrap_or("");
        if !p.picture(img, r, z * 0.22) {
            p.round(r, z * 0.3, SOFT);
            let name = it["name"].as_str().unwrap_or("?");
            let letter: String = name.chars().take(1).collect();
            let w = p.measure(&letter, 13.0, Weight::Semibold, z);
            p.text(&letter, r.x + z / 2.0 - w / 2.0, r.y + z / 2.0, 13.0, Weight::Semibold, z, TEXT);
        }
        // (Win held: its number, under it)
        if peek["dock"]["numbers"] == true && i < 9 && horizontal {
            let t = format!("{}", i + 1);
            let k = Rect::new(r.x + z / 2.0 - 8.0, r.bottom() + 2.0, 16.0, 14.0);
            p.round(k, 4.0, TEXT.alpha(0.9));
            let tw = p.measure(&t, 9.5, Weight::Bold, 16.0);
            p.text(&t, k.x + 8.0 - tw / 2.0, k.y + 7.0, 9.5, Weight::Bold, 16.0, crate::paint::Color(0.0, 0.0, 0.0, 1.0));
        }
        out.push((r, format!("app:{i}")));
        at += z + DOCK_GAP + if downloading { PROGRESS } else { 0.0 };
    }
    out
}

/// The peek of a widget shown as its icon alone: what leads it; no words, no marks (something new, it glows instead). (A peek with
/// nothing to lead it stays as it is.)
pub fn icon_only(peek: &Value) -> Value {
    // (the tray, its icon alone: its first four icons, small, in a square)
    if let Some(items) = peek["tray"]["items"].as_array() {
        let firsts: Vec<Value> = items.iter().take(4).map(|i| serde_json::json!({ "image": i["image"] })).collect();
        return serde_json::json!({ "tray_grid": { "items": firsts } });
    }
    if peek["dock"].is_object() || peek["tray"].is_object() || peek["face"].is_string() {
        return peek.clone();
    }
    let lead = ["icon", "image", "live", "ring", "vessel", "bars", "pad", "roll"];
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
    if peek["face"].is_string() {
        return Vec::new();
    }
    if peek["subtitle"].is_object() {
        subtitle(p, &peek["subtitle"], body);
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
        lead(p, &serde_json::json!({ "icon": peek["icon"], "image": peek["image"], "live": peek["live"], "ring": peek["ring"], "color": peek["color"], "pad": peek["pad"] }), body.x + body.w / 2.0 - z / 2.0, body.y + body.h / 2.0, z);
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
            lead(p, peek, cx - z / 2.0, y + z / 2.0, z);
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
