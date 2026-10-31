//! A widget open: its panel. A head (its tile, its title, a line under it) and its body, a column of blocks the plugin describes (words,
//! facts, numbers, chips, meters, rings, lists, a graph, a seek bar, buttons, a month, sections of their own). Each block is measured and
//! drawn by one function (drawn or not), so what is measured is what is drawn; what can be pressed leaves a box behind.

use crate::draw::*;
use crate::paint::Weight;
use crate::shape::Rect;
use serde_json::{json, Value};
use std::collections::HashSet;

/// What a press on a box of the panel does.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// the plugin is told (`action`, with `args`)
    Action(String, Value),
    /// a section opens or closes
    Toggle(String),
    /// a bar to seek on: the plugin is told where (`fraction`)
    Seek(String),
    /// a field to type in: the plugin is told what was typed (`text`); live, at every change. Its hint, and what it holds.
    Field(String, bool, String, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub rect: Rect,
    pub press: Press,
    /// what its tip says (words, or a panel's blocks), when it has one
    pub tip: Value,
    /// held until its ring fills, not pressed (it does something that cannot be undone)
    pub hold: bool,
    /// what a right-click on it offers (each: icon, label, action, hold)
    pub menu: Value,
}

pub struct Ctx<'a, 'b> {
    pub p: &'a Painter<'b>,
    pub draw: bool,
    pub hits: Vec<Hit>,
    /// the sections that are open (by id); the first is open until the person says otherwise
    pub open: &'a HashSet<String>,
    pub closed: &'a HashSet<String>,
    /// where the live pictures of windows go (a window, its box): the frame puts them there
    pub thumbs: Vec<(isize, Rect)>,
}

/// a window in the dock's row (its live picture's width, with its meters beside it) and the room between two
pub const WIN_PIC: f32 = 132.0;
pub const WIN_W: f32 = WIN_PIC + crate::peek::METERS + crate::peek::PROGRESS;
pub const WIN_GAP: f32 = 12.0;
const SP: f32 = 10.0;

fn s<'v>(v: &'v Value, k: &str) -> &'v str {
    v[k].as_str().unwrap_or("")
}

fn f(v: &Value, k: &str) -> f32 {
    v[k].as_f64().unwrap_or(0.0) as f32
}

thread_local! {
    /// where the mouse is over the panel being drawn (its own space), if it is: its buttons are ghosts, with a ground only under it
    static HOVER: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// Where the mouse is over the panel about to be drawn (none: not on it).
pub fn set_hover(at: Option<(f32, f32)>) {
    HOVER.with(|h| h.set(at));
}

/// Is the mouse on this box of the panel being drawn?
pub fn hovered(r: Rect) -> bool {
    HOVER.with(|h| h.get()).is_some_and(|(x, y)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
}

/// a ghost button's ground, under the mouse
const LIT: crate::paint::Color = crate::paint::Color(0.15, 0.15, 0.17, 1.0);
/// solid greys: a toggle group's track, its chosen one, a mark's ground
const TRACK: crate::paint::Color = crate::paint::Color(0.106, 0.106, 0.125, 1.0);
const CHOSEN: crate::paint::Color = crate::paint::Color(0.204, 0.204, 0.231, 1.0);
const GROUND: crate::paint::Color = crate::paint::Color(0.149, 0.149, 0.173, 1.0);
const SOFT_ICON: crate::paint::Color = crate::paint::Color(0.79, 0.79, 0.82, 1.0);
const RED_DOT: crate::paint::Color = crate::paint::Color(0.91, 0.416, 0.416, 1.0);

/// now, in ms since 1970 (what plugins say times in)
fn now_ms() -> f64 {
    nadi_core::paths::now_ms() as f64
}

/// "m:ss", or "h:mm:ss"
fn span(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// A ghost button's ground: only while the mouse is on it.
    fn ghost(&self, r: Rect, rad: f32) {
        if hovered(r) {
            self.round(r, rad, LIT);
        }
    }

    fn hit(&mut self, rect: Rect, press: Press) {
        self.hit_tip(rect, press, Value::Null);
    }

    /// Something to press that says what it is, in a tip, as the mouse rests on it.
    fn hit_tip(&mut self, rect: Rect, press: Press, tip: Value) {
        if self.draw {
            self.hits.push(Hit { rect, press, tip, hold: false, menu: Value::Null });
        }
    }

    /// The last thing to press: held, and what a right-click on it offers.
    fn last_hit(&mut self, hold: bool, menu: Value) {
        if let Some(h) = self.hits.last_mut().filter(|_| self.draw) {
            h.hold = hold;
            h.menu = menu;
        }
    }

    fn text(&self, t: &str, x: f32, cy: f32, size: f32, w: Weight, max: f32, c: crate::paint::Color) -> f32 {
        if self.draw {
            self.p.text(t, x, cy, size, w, max, c)
        } else {
            self.p.measure(t, size, w, max)
        }
    }

    fn text_end(&self, t: &str, right: f32, cy: f32, size: f32, w: Weight, max: f32, c: crate::paint::Color) {
        if self.draw {
            self.p.text_end(t, right, cy, size, w, max, c);
        }
    }

    fn round(&self, r: Rect, rad: f32, c: crate::paint::Color) {
        if self.draw {
            self.p.round(r, rad, c);
        }
    }

    fn icon(&self, name: &str, x: f32, y: f32, z: f32, c: crate::paint::Color) {
        if self.draw {
            self.p.icon(name, x, y, z, c);
        }
    }

    /// A person's mark: their animal (any words: an emoji) on a disc of their tint, `d` across, its middle at `cx`, `cy`; a ring of the
    /// theme's colour round it while they talk.
    fn avatar(&self, a: &str, tint: crate::paint::Color, cx: f32, cy: f32, d: f32, ring: bool) {
        if !self.draw {
            return;
        }
        if ring {
            self.p.circle(cx, cy, d / 2.0 + 3.5, accent());
            self.p.circle(cx, cy, d / 2.0 + 1.6, FACE);
        }
        self.p.circle(cx, cy, d / 2.0, tint);
        let size = d * 0.54;
        let aw = self.p.measure(a, size, Weight::Regular, d * 2.0);
        self.p.paragraph(a, cx - aw / 2.0 - 0.5, cy - size * 1.35 / 2.0, size, aw + 6.0, 1, TEXT, true);
    }

    /// A bar of marks across the top of a panel (`items`, each one of: `rec` (a dot: grey, red and breathing while on), `clock` (how
    /// long since `since`), `text`, `grow` (the room left), `people` (their animals, one over the next), `toggles` (a group with its
    /// track), `icon` (a ghost button), `meter`); a light running under it (`shimmer`). How tall.
    fn toolbar(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        const H: f32 = 34.0;
        const GAP: f32 = 6.0;
        let items = b["items"].as_array().cloned().unwrap_or_default();
        let width = |c: &Self, it: &Value| -> f32 {
            match s(it, "kind") {
                "rec" | "icon" => 34.0,
                "clock" => c.p.measure("0:00:00", 13.0, Weight::Semibold, 100.0).min(c.p.measure(&span((now_ms() - it["since"].as_f64().unwrap_or(0.0)) / 1000.0), 13.0, Weight::Semibold, 100.0) + 2.0),
                "text" => c.p.measure(s(it, "text"), 12.0, Weight::Semibold, 200.0) + 2.0,
                "people" => {
                    let n = it["items"].as_array().map_or(0, |a| a.len().min(6));
                    if n == 0 { 0.0 } else { 26.0 + (n - 1) as f32 * 19.0 + 4.0 }
                }
                "toggles" => {
                    let n = it["items"].as_array().map_or(0, |a| a.len());
                    n as f32 * 30.0 + (n.max(1) - 1) as f32 * 2.0 + 4.0
                }
                "meter" => it["w"].as_f64().unwrap_or(90.0) as f32,
                _ => 0.0,
            }
        };
        let fixed: f32 = items.iter().map(|it| width(self, it)).sum::<f32>() + GAP * (items.len().max(1) - 1) as f32;
        let grows = items.iter().filter(|it| s(it, "kind") == "grow").count().max(1) as f32;
        let room = ((w - fixed) / grows).max(0.0);
        let cy = y + H / 2.0;
        let t = now_ms();
        let mut cx = x;
        for it in &items {
            let kind = s(it, "kind");
            let iw = if kind == "grow" { room } else { width(self, it) };
            let action = s(it, "action").to_string();
            let tip = it["tip"].clone();
            match kind {
                "rec" => {
                    let r = Rect::new(cx, y, 34.0, 34.0);
                    if self.draw && hovered(r) {
                        self.p.circle(cx + 17.0, cy, 17.0, LIT);
                    }
                    if self.draw {
                        if it["on"] == true {
                            let breathe = 0.5 + 0.5 * ((t / 1600.0 * std::f64::consts::TAU).sin() as f32);
                            self.p.circle(cx + 17.0, cy, 8.0 + 3.0 * breathe, RED_DOT.alpha(0.18 + 0.1 * breathe));
                            self.p.circle(cx + 17.0, cy, 6.0, RED_DOT);
                        } else {
                            self.p.circle(cx + 17.0, cy, 6.0, SOFT_ICON);
                        }
                    }
                    if !action.is_empty() {
                        self.hit_tip(r, Press::Action(action, it["args"].clone()), tip);
                    }
                }
                "clock" => {
                    let el = (t - it["since"].as_f64().unwrap_or(t)) / 1000.0;
                    self.text(&span(el), cx, cy, 13.0, Weight::Semibold, 100.0, TEXT);
                }
                "text" => {
                    self.text(s(it, "text"), cx, cy, 12.0, Weight::Semibold, 200.0, if s(it, "tone") == "muted" { DIM } else { TEXT });
                }
                "people" => {
                    let ps = it["items"].as_array().cloned().unwrap_or_default();
                    let mut px = cx + 13.0;
                    for pp in ps.iter().take(6) {
                        if self.draw {
                            self.p.circle(px, cy, 15.0, FACE);
                        }
                        self.avatar(s(pp, "avatar"), color_of(&pp["tint"], GROUND), px, cy, 26.0, pp["ring"] == true);
                        px += 19.0;
                    }
                    if !action.is_empty() {
                        self.hit_tip(Rect::new(cx, y, iw, H), Press::Action(action, it["args"].clone()), tip);
                    }
                }
                "toggles" => {
                    let ts = it["items"].as_array().cloned().unwrap_or_default();
                    self.round(Rect::new(cx, cy - 16.0, iw, 32.0), 11.0, TRACK);
                    let mut tx = cx + 2.0;
                    for tg in &ts {
                        let r = Rect::new(tx, cy - 14.0, 30.0, 28.0);
                        let on = tg["on"] == true;
                        if on {
                            self.round(r, 9.0, CHOSEN);
                        } else if self.draw && hovered(r) {
                            self.round(r, 9.0, LIT);
                        }
                        self.icon(s(tg, "icon"), tx + 7.5, cy - 7.5, 15.0, if on { TEXT } else { DIM });
                        if !s(tg, "action").is_empty() {
                            self.hit_tip(r, Press::Action(s(tg, "action").to_string(), tg["args"].clone()), tg["tip"].clone());
                        }
                        tx += 32.0;
                    }
                }
                "icon" => {
                    let r = Rect::new(cx, y, 34.0, 34.0);
                    let on = it["on"] == true;
                    if !action.is_empty() {
                        self.ghost(r, 10.0);
                    }
                    let c = color_of(&it["color"], if on { accent() } else { SOFT_ICON });
                    self.icon(s(it, "icon"), cx + 8.5, cy - 8.5, 17.0, c);
                    if on && self.draw {
                        self.p.circle(cx + 17.0, y + H - 4.0, 2.0, accent());
                    }
                    if !action.is_empty() {
                        self.hit_tip(r, Press::Action(action, it["args"].clone()), tip);
                    } else if !tip.is_null() {
                        self.hit_tip(r, Press::Action(String::new(), Value::Null), tip);
                    }
                }
                "meter" => {
                    if self.draw {
                        self.p.round(Rect::new(cx, cy - 2.0, iw, 4.0), 2.0, GROUND);
                        let v = f(it, "value").clamp(0.0, 1.0);
                        if v > 0.0 {
                            self.p.round(Rect::new(cx, cy - 2.0, iw * v, 4.0), 2.0, color_of(&it["color"], accent()));
                        }
                    }
                }
                _ => {}
            }
            cx += iw + GAP;
        }
        if b["shimmer"] == true {
            if self.draw {
                let ly = y + H + 5.0;
                self.p.round(Rect::new(x, ly, w, 2.0), 1.0, TRACK);
                let phase = ((t % 1800.0) / 1800.0) as f32;
                let seg = w * 0.32;
                let sx = x - seg + (w + seg) * phase;
                let (a, z) = (sx.max(x), (sx + seg).min(x + w));
                if z > a {
                    self.p.round(Rect::new(a, ly, z - a, 2.0), 1.0, accent());
                }
            }
            return H + 8.0;
        }
        H
    }

    /// What was said, as a script: each person's animal and the time in a gutter, their name, then their lines. Consecutive lines of
    /// one person go together; `{ day }` items are headings. A line may be pressed (`action`): its ground lights, a play mark in the
    /// gutter; one playing (`playing = { since, secs, words? }`) shows a pause mark, a bar, and its words lit as they were said; one
    /// still being said (`draft`) is dim with a caret; one just made better (`fresh`, ms) is underlined for a moment.
    fn script(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        const GUT: f32 = 44.0;
        let size = b["size"].as_f64().unwrap_or(13.0) as f32;
        let items = b["items"].as_array().cloned().unwrap_or_default();
        let tx = x + GUT + 10.0;
        let tw = (w - GUT - 10.0).max(40.0);
        let t = now_ms();
        let mut h = 0.0f32;
        let mut i = 0;
        while i < items.len() {
            let it = &items[i];
            if !s(it, "day").is_empty() {
                let dw = self.p.measure(s(it, "day"), 10.5, Weight::Semibold, w);
                self.text(s(it, "day"), x + w / 2.0 - dw / 2.0, y + h + 8.0, 10.5, Weight::Semibold, w, DIM);
                h += 16.0 + 12.0;
                i += 1;
                continue;
            }
            // (the run of one person's lines)
            let who = |v: &Value| format!("{}|{}", s(v, "speaker"), s(v, "name"));
            let mut j = i + 1;
            while j < items.len() && s(&items[j], "day").is_empty() && who(&items[j]) == who(it) {
                j += 1;
            }
            let run = &items[i..j];
            let gy = y + h;
            let name = s(it, "name");
            let name_h = if name.is_empty() { 0.0 } else { 18.0 };
            // (where each line is, and how tall)
            let mut rows: Vec<(f32, f32)> = Vec::new();
            let mut ly = gy + name_h;
            for l in run {
                let lh = self.p.paragraph(s(l, "text"), 0.0, 0.0, size, tw, 99, TEXT, false).max(size * 1.35);
                rows.push((ly, lh));
                ly += lh + 6.0;
            }
            let body_h = ly - 6.0 - gy;
            let talking = run.iter().any(|l| l["draft"] == true);
            // (the gutter: the animal, the time under it; a line's play mark over it, when that line is the first)
            let mark_y = |r: (f32, f32)| r.0 + size * 0.68;
            let first_marked = {
                let r = Rect::new(tx - 6.0, rows[0].0 - 3.0, tw + 12.0, rows[0].1 + 6.0);
                let pl = &run[0]["playing"];
                let playing = pl.is_object() && (t - pl["since"].as_f64().unwrap_or(0.0)) / 1000.0 <= pl["secs"].as_f64().unwrap_or(0.0) + 0.3;
                (!s(&run[0], "action").is_empty() && hovered(r)) || playing
            };
            let ax = x + GUT / 2.0;
            if !first_marked {
                self.avatar(s(it, "avatar"), color_of(&it["tint"], GROUND), ax, gy + 13.0, 26.0, talking);
            }
            if !s(it, "avatar_action").is_empty() {
                self.hit_tip(Rect::new(ax - 15.0, gy - 2.0, 30.0, 30.0), Press::Action(s(it, "avatar_action").to_string(), it["args"].clone()), it["avatar_tip"].clone());
            }
            if !s(it, "time").is_empty() {
                let tm = s(it, "time");
                let tmw = self.p.measure(tm, 10.5, Weight::Regular, GUT);
                self.text(tm, ax - tmw / 2.0, gy + 36.0, 10.5, Weight::Regular, GUT, DIM);
            }
            if name_h > 0.0 {
                self.text(name, tx, gy + 8.0, 12.0, Weight::Semibold, tw, if it["me"] == true { accent() } else { TEXT });
            }
            for (li, (l, r)) in run.iter().zip(rows.iter().copied()).enumerate() {
                let text = s(l, "text");
                let rect = Rect::new(tx - 6.0, r.0 - 3.0, tw + 12.0, r.1 + 6.0);
                let action = s(l, "action");
                let pl = &l["playing"];
                let el = (t - pl["since"].as_f64().unwrap_or(0.0)) / 1000.0;
                let secs = pl["secs"].as_f64().unwrap_or(0.0).max(0.1);
                let playing = pl.is_object() && el >= 0.0 && el <= secs + 0.3;
                let hover = !action.is_empty() && hovered(rect);
                if hover || playing {
                    self.round(rect, 8.0, LIT);
                }
                // (its mark in the gutter: play, or pause while it plays)
                if (hover || playing) && self.draw {
                    // (the first line's mark takes the animal's place)
                    let my = if li == 0 { gy + 13.0 } else { mark_y(r) };
                    self.p.circle(ax, my, 13.0, GROUND);
                    if playing {
                        self.p.round(Rect::new(ax - 4.0, my - 5.0, 3.0, 10.0), 1.0, accent());
                        self.p.round(Rect::new(ax + 1.0, my - 5.0, 3.0, 10.0), 1.0, accent());
                    } else {
                        self.p.icon("play", ax - 6.0, my - 6.0, 12.0, TEXT);
                    }
                }
                if self.draw {
                    let base = if l["draft"] == true { DIM } else { TEXT.alpha(0.92) };
                    if playing {
                        let n = text.encode_utf16().count();
                        let lit = match pl["words"].as_array() {
                            Some(ws) if !ws.is_empty() => ws.iter().filter(|wd| wd[0].as_f64().unwrap_or(1e9) <= el).filter_map(|wd| wd[1].as_u64()).max().unwrap_or(0) as usize,
                            _ => ((el / secs).clamp(0.0, 1.0) * n as f64) as usize,
                        };
                        self.p.paragraph_lit(text, tx, r.0, size, tw, lit, DIM, TEXT, None);
                        self.p.round(Rect::new(tx, r.0 + r.1 + 1.0, tw, 2.0), 1.0, GROUND);
                        self.p.round(Rect::new(tx, r.0 + r.1 + 1.0, tw * (el / secs).clamp(0.0, 1.0) as f32, 2.0), 1.0, accent());
                    } else if l["draft"] == true {
                        let blink = (t / 530.0) as u64 % 2 == 0;
                        self.p.paragraph_lit(text, tx, r.0, size, tw, 0, base, base, blink.then(accent));
                    } else {
                        self.p.paragraph(text, tx, r.0, size, tw, 99, base, true);
                    }
                    // (just made better: underlined, fading)
                    if let Some(fr) = l["fresh"].as_f64() {
                        let age = ((t - fr) / 1000.0) as f32;
                        if (0.0..3.0).contains(&age) {
                            let uw = self.p.measure(text, size, Weight::Regular, tw);
                            self.p.round(Rect::new(tx, r.0 + size * 1.22, uw, 2.0), 1.0, accent().alpha(0.55 * (1.0 - age / 3.0)));
                        }
                    }
                }
                if !action.is_empty() {
                    self.hit_tip(rect, Press::Action(action.to_string(), l["args"].clone()), l["tip"].clone());
                }
            }
            let gut_h = if s(it, "time").is_empty() { 28.0 } else { 44.0 };
            h += body_h.max(gut_h) + 14.0;
            i = j;
        }
        (h - 14.0).max(0.0)
    }

    /// A round button with an icon (or a pill with words when it has no icon); its box.
    fn button(&mut self, b: &Value, x: f32, cy: f32, big: bool) -> f32 {
        let icon = s(b, "icon");
        let label = s(b, "label");
        let action = s(b, "action").to_string();
        let tone = s(b, "tone");
        let bg = match tone {
            "ok" => GREEN.alpha(0.22),
            "no" => RED.alpha(0.2),
            _ => FAINT,
        };
        let fg = match tone {
            "ok" => GREEN,
            "no" => RED,
            _ => TEXT,
        };
        let h = if big { 34.0 } else { 26.0 };
        let bg = if action.is_empty() { bg } else { bg.alpha(1.25) };
        let w = if !icon.is_empty() {
            let d = if big { 34.0 } else { 26.0 };
            if self.draw && (action.is_empty() || hovered(Rect::new(x, cy - h / 2.0, d, h))) {
                self.p.circle(x + d / 2.0, cy, d / 2.0, bg);
            }
            let z = d * 0.5;
            self.icon(icon, x + d / 2.0 - z / 2.0, cy - z / 2.0, z, fg);
            d
        } else {
            let tw = self.p.measure(label, 11.5, Weight::Semibold, 200.0);
            let w = tw + 22.0;
            if action.is_empty() || hovered(Rect::new(x, cy - 13.0, w, 26.0)) {
                self.round(Rect::new(x, cy - 13.0, w, 26.0), 13.0, bg);
            }
            self.text(label, x + 11.0, cy, 11.5, Weight::Semibold, 200.0, fg);
            w
        };
        if !action.is_empty() {
            // (an icon alone says what it does in its tip: its own, else its label)
            let tip = if !b["tip"].is_null() { b["tip"].clone() } else if !icon.is_empty() && !label.is_empty() { Value::String(label.to_string()) } else { Value::Null };
            self.hit_tip(Rect::new(x, cy - h / 2.0, w, h), Press::Action(action, b["args"].clone()), tip);
            self.last_hit(b["hold"] == true, b["menu"].clone());
        }
        w
    }

    /// One block at `x`, `y`, `w` wide; how tall it is.
    pub fn block(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        match s(b, "kind") {
            "text" if !s(b, "action").is_empty() && self.draw => {
                // (pressable: its ground lit while the mouse is on it)
                let was = self.draw;
                self.draw = false;
                let mut plain = b.clone();
                plain["action"] = Value::Null;
                let h = self.block(&plain, x, y, w);
                self.draw = was;
                let r = Rect::new(x - 6.0, y - 2.0, w + 12.0, h + 4.0);
                self.ghost(r, 8.0);
                self.block(&plain, x, y, w);
                let tip = if s(b, "tip").is_empty() { Value::Null } else { b["tip"].clone() };
                self.hit_tip(r, Press::Action(s(b, "action").to_string(), b["args"].clone()), tip);
                h
            }
            "text" => {
                let mut h = 0.0;
                if !s(b, "label").is_empty() {
                    self.text(s(b, "label"), x, y + 7.0, 10.5, Weight::Semibold, w, DIM);
                    h += 16.0;
                }
                let c = if s(b, "tone") == "muted" { DIM } else { TEXT };
                // (a heading alone)
                if s(b, "text").is_empty() {
                    return h;
                }
                h + self.p.paragraph(s(b, "text"), x, y + h, 12.0, w, b["lines"].as_u64().unwrap_or(4) as usize, c, self.draw)
            }
            "table" => self.table(b, x, y, w),
            "toolbar" => self.toolbar(b, x, y, w),
            "script" => self.script(b, x, y, w),
            // (things to pick from, in a grid: animals, people; the chosen one on a grey of its own)
            "picker" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let cell = b["size"].as_f64().unwrap_or(30.0) as f32;
                let cols = ((w + 4.0) / (cell + 4.0)).floor().max(1.0) as usize;
                let cw = (w - 4.0 * (cols - 1) as f32) / cols as f32;
                for (n, it) in items.iter().enumerate() {
                    let (cx, cy) = (x + (n % cols) as f32 * (cw + 4.0), y + (n / cols) as f32 * (cell + 4.0));
                    let r = Rect::new(cx, cy, cw, cell);
                    if it["on"] == true {
                        self.round(r, 9.0, CHOSEN);
                    } else if !s(it, "action").is_empty() {
                        self.ghost(r, 9.0);
                    }
                    if !s(it, "icon").is_empty() {
                        self.icon(s(it, "icon"), cx + cw / 2.0 - 8.0, cy + cell / 2.0 - 8.0, 16.0, if it["on"] == true { TEXT } else { DIM });
                    } else if it["tint"].is_string() {
                        self.avatar(s(it, "text"), color_of(&it["tint"], GROUND), cx + cw / 2.0, cy + cell / 2.0, cell - 6.0, false);
                    } else if self.draw {
                        let size = cell * 0.52;
                        let tw = self.p.measure(s(it, "text"), size, Weight::Regular, cw);
                        self.p.paragraph(s(it, "text"), cx + cw / 2.0 - tw / 2.0 - 0.5, cy + cell / 2.0 - size * 0.675, size, tw + 6.0, 1, TEXT, true);
                    }
                    if !s(it, "action").is_empty() {
                        self.hit_tip(r, Press::Action(s(it, "action").to_string(), it["args"].clone()), it["tip"].clone());
                    }
                }
                let rows = items.len().div_ceil(cols);
                (rows as f32 * (cell + 4.0) - 4.0).max(0.0)
            }
            // (a person: their animal, big, and a field for their name beside it)
            "person" => {
                self.avatar(s(b, "avatar"), color_of(&b["tint"], GROUND), x + 20.0, y + 20.0, 40.0, false);
                if b["field"].is_object() {
                    let mut fb = b["field"].clone();
                    fb["kind"] = json!("field");
                    self.block(&fb, x + 52.0, y + 5.0, w - 52.0);
                }
                40.0
            }
            // (how long each spoke: their animal, a bar, the time)
            "shares" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                for (n, r) in rows.iter().enumerate() {
                    let cy = y + n as f32 * 32.0 + 15.0;
                    self.avatar(s(r, "avatar"), color_of(&r["tint"], GROUND), x + 13.0, cy, 26.0, false);
                    let bw = (w - 34.0 - 52.0).max(10.0) * f(r, "value").clamp(0.0, 1.0);
                    self.round(Rect::new(x + 34.0, cy - 3.0, bw.max(6.0), 6.0), 3.0, color_of(&r["color"], accent()));
                    self.text(s(r, "text"), x + 34.0 + bw.max(6.0) + 8.0, cy, 11.5, Weight::Regular, 60.0, DIM);
                }
                (rows.len() as f32 * 32.0 - 4.0).max(0.0)
            }
            // (sessions: by day, each who was in it, when, how long, its first words)
            "sessions" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let mut h = 0.0;
                for it in &items {
                    if !s(it, "day").is_empty() {
                        self.text(s(it, "day"), x, y + h + 9.0, 10.5, Weight::Semibold, w, DIM);
                        h += 20.0;
                        continue;
                    }
                    let r = Rect::new(x - 6.0, y + h, w + 12.0, 44.0);
                    if !s(it, "action").is_empty() {
                        self.ghost(r, 10.0);
                        self.hit_tip(r, Press::Action(s(it, "action").to_string(), it["args"].clone()), it["tip"].clone());
                    }
                    let ps = it["people"].as_array().cloned().unwrap_or_default();
                    let mut px = x + 13.0;
                    for pp in ps.iter().take(3) {
                        if self.draw {
                            self.p.circle(px, y + h + 22.0, 15.0, FACE);
                        }
                        self.avatar(s(pp, "avatar"), color_of(&pp["tint"], GROUND), px, y + h + 22.0, 26.0, false);
                        px += 19.0;
                    }
                    let lx = x + 13.0 + 19.0 * 2.0 + 18.0;
                    let lw = w - (lx - x);
                    let tw = self.text(s(it, "title"), lx, y + h + 14.0, 12.0, Weight::Semibold, lw, TEXT);
                    self.text(s(it, "right"), lx + tw + 8.0, y + h + 14.0, 11.0, Weight::Regular, (lw - tw - 8.0).max(0.0), DIM);
                    self.text(s(it, "sub"), lx, y + h + 31.0, 12.0, Weight::Regular, lw, DIM);
                    h += 46.0;
                }
                (h - 2.0).max(0.0)
            }
            "subtitle" => {
                let mut h = 0.0;
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let size = b["size"].as_f64().unwrap_or(18.0) as f32;
                    let name = s(&it, "name");
                    let draft = it["draft"] == true;
                    let nw = if name.is_empty() { 0.0 } else { self.p.measure(name, size, Weight::Semibold, 120.0) + 8.0 };
                    if self.draw && !name.is_empty() {
                        self.p.text(name, x, y + h + size * 0.68, size, Weight::Semibold, 120.0, if draft { DIM } else { TEXT });
                    }
                    let tw = w - nw;
                    let shadow = crate::paint::Color(0.0, 0.0, 0.0, 0.85);
                    if self.draw {
                        self.p.paragraph(s(&it, "text"), x + nw + 1.0, y + h + 1.5, size, tw, 3, shadow, true);
                    }
                    let lh = self.p.paragraph(s(&it, "text"), x + nw, y + h, size, tw, 3, if draft { TEXT.alpha(0.72) } else { TEXT }, self.draw);
                    h += lh.max(size * 1.35) + 6.0;
                }
                h
            }
            "facts" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                for (i, r) in rows.iter().enumerate() {
                    let cy = y + i as f32 * 22.0 + 11.0;
                    self.text(r[0].as_str().unwrap_or(""), x, cy, 12.0, Weight::Regular, w * 0.5, DIM);
                    let v = r[1].as_str().unwrap_or("");
                    let vw = self.p.measure(v, 12.0, Weight::Semibold, w * 0.6);
                    self.text(v, x + w - vw, cy, 12.0, Weight::Semibold, w * 0.6, TEXT);
                }
                rows.len() as f32 * 22.0
            }
            "stats" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let n = items.len().clamp(1, 4) as f32;
                let cw = w / n;
                for (i, it) in items.iter().take(4).enumerate() {
                    let cx = x + i as f32 * cw;
                    self.round(Rect::new(cx + 2.0, y, cw - 4.0, 44.0), 10.0, FAINT);
                    let v = s(it, "value");
                    let vw = self.p.measure(v, 15.0, Weight::Semibold, cw - 10.0);
                    self.text(v, cx + cw / 2.0 - vw / 2.0, y + 17.0, 15.0, Weight::Semibold, cw - 10.0, TEXT);
                    let l = s(it, "label");
                    let lw = self.p.measure(l, 10.0, Weight::Regular, cw - 10.0);
                    self.text(l, cx + cw / 2.0 - lw / 2.0, y + 33.0, 10.0, Weight::Regular, cw - 10.0, DIM);
                }
                44.0
            }
            "chips" => {
                let mut cx = x;
                let mut cy = y + 11.0;
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let icon = s(&it, "icon");
                    let text = s(&it, "text");
                    let lit = s(&it, "tone") == "hot" || s(&it, "tone") == "ok";
                    let c = if lit { if s(&it, "tone") == "ok" { GREEN } else { AMBER } } else { TEXT };
                    let tw = self.p.measure(text, 11.0, Weight::Regular, w - 30.0);
                    let cw = tw + 20.0 + if icon.is_empty() { 0.0 } else { 16.0 };
                    if cx + cw > x + w && cx > x {
                        cx = x;
                        cy += 26.0;
                    }
                    // (one to press: a ghost, the one chosen in the theme colour; one that only says something: its ground)
                    let on = it["on"] == true;
                    let pressed = !s(&it, "action").is_empty();
                    let c = if on { accent() } else { c };
                    let cr = Rect::new(cx, cy - 11.0, cw, 22.0);
                    if pressed {
                        self.ghost(cr, 11.0);
                    } else {
                        self.round(cr, 11.0, if lit { c.alpha(0.16) } else { FAINT });
                    }
                    let mut tx = cx + 10.0;
                    if !icon.is_empty() {
                        self.icon(icon, tx, cy - 6.5, 13.0, c);
                        tx += 16.0;
                    }
                    self.text(text, tx, cy, 11.0, if on { Weight::Semibold } else { Weight::Regular }, w - 30.0, c);
                    let action = s(&it, "action");
                    if !action.is_empty() {
                        self.hit(Rect::new(cx, cy - 11.0, cw, 22.0), Press::Action(action.to_string(), it["args"].clone()));
                    }
                    cx += cw + 6.0;
                }
                cy + 11.0 - y
            }
            "meter" => {
                // (its label; then, fainter, what more it says)
                let lw = self.text(s(b, "label"), x, y + 7.0, 11.0, Weight::Regular, w * 0.5, DIM);
                if !s(b, "sub").is_empty() {
                    self.text(s(b, "sub"), x + lw + 8.0, y + 7.0, 10.5, Weight::Regular, w * 0.5 - lw, SOFT);
                }
                let t = s(b, "text");
                let tw = self.p.measure(t, 11.0, Weight::Semibold, w * 0.5);
                self.text(t, x + w - tw, y + 7.0, 11.0, Weight::Semibold, w * 0.5, TEXT);
                if self.draw {
                    self.p.bar(Rect::new(x, y + 18.0, w, 5.0), f(b, "part"), color_of(&b["color"], GREEN));
                }
                24.0
            }
            "bars" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let mut h = 0.0;
                for it in &items {
                    let mut m = it.clone();
                    m["kind"] = json!("meter");
                    h += self.block(&m, x, y + h, w) + 8.0;
                }
                (h - 8.0).max(0.0)
            }
            "rings" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let n = items.len().clamp(1, 4) as f32;
                let cw = w / n;
                for (i, it) in items.iter().take(4).enumerate() {
                    let cx = x + i as f32 * cw + cw / 2.0;
                    let c = color_of(&it["color"], GREEN);
                    if self.draw {
                        self.p.ring(cx, y + 26.0, 22.0, 4.0, f(it, "part"), 0.0, c, FAINT);
                    }
                    let t = s(it, "text");
                    let tw = self.p.measure(t, 12.0, Weight::Semibold, 40.0);
                    self.text(t, cx - tw / 2.0, y + 26.0, 12.0, Weight::Semibold, 40.0, TEXT);
                    let l = s(it, "label");
                    let lw = self.p.measure(l, 10.5, Weight::Regular, cw);
                    self.text(l, cx - lw / 2.0, y + 60.0, 10.5, Weight::Regular, cw, DIM);
                }
                68.0
            }
            "list" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                let mut h = 0.0;
                for r in &rows {
                    let has_sub = !s(r, "sub").is_empty();
                    let has_bar = r["progress"].is_number();
                    let rh = 20.0 + if has_sub { 14.0 } else { 0.0 } + if has_bar { 8.0 } else { 0.0 };
                    let action = s(r, "action");
                    if !action.is_empty() {
                        self.hit(Rect::new(x - 4.0, y + h, w + 8.0, rh), Press::Action(action.to_string(), r["args"].clone()));
                    }
                    // (what is on its right: when, how much)
                    let right = s(r, "right");
                    let lit = r["lit"] == true;
                    let rw = if right.is_empty() { 0.0 } else { self.p.measure(right, 10.5, Weight::Regular, 80.0) + 8.0 };
                    if !right.is_empty() {
                        self.text_end(right, x + w, y + h + 9.0, 10.5, Weight::Regular, 80.0, if lit { GREEN } else { DIM });
                    }
                    let ix = if s(r, "icon").is_empty() { 0.0 } else { 22.0 };
                    if ix > 0.0 {
                        self.icon(s(r, "icon"), x, y + h + 2.0, 14.0, if lit { GREEN } else { DIM });
                    }
                    self.text(s(r, "title"), x + ix, y + h + 9.0, 12.0, Weight::Semibold, w - rw - ix, TEXT);
                    if has_sub {
                        self.text(s(r, "sub"), x + ix, y + h + 24.0, 10.5, Weight::Regular, w - ix, DIM);
                    }
                    if has_bar && self.draw {
                        self.p.bar(Rect::new(x, y + h + rh - 6.0, w, 3.0), f(r, "progress"), color_of(&b["color"], TEXT.alpha(0.8)));
                    }
                    h += rh + 4.0;
                }
                (h - 4.0).max(0.0)
            }
            "rows" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                for (i, r) in rows.iter().enumerate() {
                    let cy = y + i as f32 * 24.0 + 12.0;
                    let past = r["past"].as_bool() == Some(true);
                    self.round(Rect::new(x, cy - 8.0, 3.0, 16.0), 1.5, if past { SOFT } else { accent() });
                    let tw = self.text(s(r, "time"), x + 10.0, cy, 11.0, Weight::Regular, 130.0, DIM);
                    self.text(s(r, "title"), x + 10.0 + tw.max(96.0) + 8.0, cy, 12.0, Weight::Semibold, w - tw.max(96.0) - 18.0, if past { DIM } else { TEXT });
                }
                rows.len() as f32 * 24.0
            }
            "lines" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                for (i, r) in rows.iter().enumerate() {
                    let cy = y + i as f32 * 22.0 + 11.0;
                    let now = r["now"].as_bool() == Some(true);
                    let c = if now { TEXT } else { DIM };
                    self.icon(s(r, "icon"), x, cy - 7.0, 14.0, if now { accent() } else { DIM });
                    self.text(s(r, "text"), x + 22.0, cy, 11.5, Weight::Regular, w - 22.0, c);
                }
                rows.len() as f32 * 22.0
            }
            "strip" => {
                let c = tone_color(s(b, "tone"));
                self.round(Rect::new(x, y, w, 28.0), 9.0, c.alpha(0.14));
                let side = s(b, "side");
                let sw = self.p.measure(side, 10.5, Weight::Regular, w * 0.4);
                self.text(s(b, "text"), x + 10.0, y + 14.0, 11.5, Weight::Semibold, w - sw - 28.0, c);
                self.text(side, x + w - 10.0 - sw, y + 14.0, 10.5, Weight::Regular, w * 0.4, DIM);
                28.0
            }
            "checklist" => {
                let mut h = 0.0;
                if !s(b, "title").is_empty() {
                    self.text(s(b, "title"), x, y + 7.0, 10.5, Weight::Semibold, w, DIM);
                    h += 18.0;
                }
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let cy = y + h + 10.0;
                    let state = s(&it, "state");
                    let (icon, c) = match state {
                        "done" => ("check", GREEN),
                        "now" => ("dot", accent()),
                        _ => ("dot", DIM),
                    };
                    if state == "now" && self.draw {
                        self.p.ring(x + 7.0, cy, 5.5, 1.4, 0.3, (self.p.t * 0.9).fract(), accent(), FAINT);
                    } else {
                        self.icon(icon, x, cy - 7.0, 14.0, c);
                    }
                    self.text(s(&it, "text"), x + 22.0, cy, 11.5, Weight::Regular, w - 22.0, if state == "done" { DIM } else { TEXT });
                    h += 20.0;
                }
                h
            }
            "files" => {
                let mut h = 0.0;
                if !s(b, "title").is_empty() {
                    self.text(s(b, "title"), x, y + 7.0, 10.5, Weight::Semibold, w, DIM);
                    h += 18.0;
                }
                for r in b["rows"].as_array().cloned().unwrap_or_default() {
                    let cy = y + h + 10.0;
                    let add = format!("+{}", r["added"].as_u64().unwrap_or(0));
                    let rem = format!("−{}", r["removed"].as_u64().unwrap_or(0));
                    let rw = self.p.measure(&rem, 11.0, Weight::Semibold, 60.0);
                    let aw = self.p.measure(&add, 11.0, Weight::Semibold, 60.0);
                    self.text(&rem, x + w - rw, cy, 11.0, Weight::Semibold, 60.0, RED);
                    self.text(&add, x + w - rw - 8.0 - aw, cy, 11.0, Weight::Semibold, 60.0, GREEN);
                    self.text(s(&r, "name"), x, cy, 11.5, Weight::Regular, w - rw - aw - 20.0, TEXT);
                    h += 20.0;
                }
                h
            }
            "graph" | "spectrum" => {
                let vals: Vec<f32> = b["values"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                let h = 56.0;
                if self.draw && !vals.is_empty() {
                    self.p.round(Rect::new(x, y, w, h), 10.0, FAINT);
                    let max = b["max"].as_f64().map(|m| m as f32).unwrap_or_else(|| vals.iter().cloned().fold(1e-6, f32::max)).max(1e-6);
                    let c = color_of(&b["color"], GREEN);
                    if s(b, "kind") == "spectrum" {
                        let n = vals.len() as f32;
                        let bw = (w - 16.0) / n;
                        for (i, v) in vals.iter().enumerate() {
                            let bh = ((v / max).clamp(0.0, 1.0) * (h - 12.0)).max(2.0);
                            self.p.round(Rect::new(x + 8.0 + i as f32 * bw + 1.0, y + h - 6.0 - bh, (bw - 2.0).max(1.0), bh), 1.5, c);
                        }
                    } else {
                        for m in b["marks"].as_array().cloned().unwrap_or_default() {
                            let my = y + h - 6.0 - (m.as_f64().unwrap_or(0.0) as f32 / max).clamp(0.0, 1.0) * (h - 12.0);
                            self.p.round(Rect::new(x + 6.0, my, w - 12.0, 1.0), 0.5, SOFT);
                        }
                        let n = vals.len().max(2) as f32 - 1.0;
                        // (each value a thin column: no line to join, and no outline)
                        let bw = ((w - 12.0) / (n + 1.0)).max(1.0);
                        for (i, v) in vals.iter().enumerate() {
                            let bh = ((v / max).clamp(0.0, 1.0) * (h - 12.0)).max(1.5);
                            self.p.round(Rect::new(x + 6.0 + i as f32 * bw, y + h - 6.0 - bh, (bw - 1.0).max(1.0), bh), 1.0, c.alpha(0.85));
                        }
                    }
                }
                h
            }
            "seek" => {
                let before = b["before"].as_array().cloned().unwrap_or_default();
                let after = b["after"].as_array().cloned().unwrap_or_default();
                let cy = y + 13.0;
                let mut lx = x;
                for bt in &before {
                    lx += self.button(bt, lx, cy, false) + 8.0;
                }
                let mut rx = x + w;
                let widths: Vec<f32> = after.iter().map(|_| 26.0).collect();
                for (i, bt) in after.iter().enumerate().rev() {
                    rx -= widths[i];
                    self.button(bt, rx, cy, false);
                    rx -= 8.0;
                }
                let bar = Rect::new(lx + 4.0, cy - 2.5, (rx - lx - 8.0).max(10.0), 5.0);
                if self.draw {
                    self.p.bar(bar, f(b, "value"), TEXT);
                }
                self.hit(Rect::new(bar.x, cy - 10.0, bar.w, 20.0), Press::Seek(s(b, "action").to_string()));
                let fmt = |sec: f32| {
                    let t = sec.max(0.0) as u64;
                    if t >= 3600 {
                        format!("{}:{:02}:{:02}", t / 3600, (t % 3600) / 60, t % 60)
                    } else {
                        format!("{}:{:02}", t / 60, t % 60)
                    }
                };
                if b["duration"].is_number() {
                    let a = fmt(f(b, "position"));
                    let z = fmt(f(b, "duration"));
                    self.text(&a, bar.x, cy + 15.0, 10.0, Weight::Regular, 60.0, DIM);
                    let zw = self.p.measure(&z, 10.0, Weight::Regular, 60.0);
                    self.text(&z, bar.right() - zw, cy + 15.0, 10.0, Weight::Regular, 60.0, DIM);
                    return 40.0;
                }
                26.0
            }
            "controls" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let total = items.len() as f32 * 34.0 + (items.len().saturating_sub(1)) as f32 * 14.0;
                let mut cx = x + w / 2.0 - total / 2.0;
                for it in &items {
                    cx += self.button(it, cx, y + 17.0, true) + 14.0;
                }
                34.0
            }
            "buttons" => {
                let mut cx = x;
                let mut cy = y + 13.0;
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let bw = if s(&it, "icon").is_empty() { self.p.measure(s(&it, "label"), 11.5, Weight::Semibold, 200.0) + 22.0 } else { 26.0 };
                    if cx + bw > x + w && cx > x {
                        cx = x;
                        cy += 32.0;
                    }
                    cx += self.button(&it, cx, cy, false) + 8.0;
                }
                cy + 13.0 - y
            }
            "choice" => {
                let mut h = 0.0;
                if !s(b, "label").is_empty() {
                    self.text(s(b, "label"), x, y + 7.0, 10.5, Weight::Semibold, w, DIM);
                    h += 18.0;
                }
                let action = s(b, "action").to_string();
                let mut cx = x;
                let mut cy = y + h + 12.0;
                for o in b["options"].as_array().cloned().unwrap_or_default() {
                    let label = s(&o, "label");
                    let on = o["value"] == b["value"] || (o["value"].as_f64().is_some() && o["value"].as_f64() == b["value"].as_f64());
                    let ow = self.p.measure(label, 11.0, Weight::Semibold, 120.0) + 18.0;
                    if cx + ow > x + w && cx > x {
                        cx = x;
                        cy += 28.0;
                    }
                    // (a toggle group: each on its ground, the one chosen filled)
                    let or = Rect::new(cx, cy - 12.0, ow, 24.0);
                    self.round(or, 12.0, if on { TEXT } else if hovered(or) { LIT } else { FAINT });
                    self.text(label, cx + 9.0, cy, 11.0, Weight::Semibold, 120.0, if on { FACE } else { TEXT });
                    self.hit(Rect::new(cx, cy - 12.0, ow, 24.0), Press::Action(action.clone(), json!({ "value": o["value"] })));
                    cx += ow + 6.0;
                }
                cy + 12.0 - y
            }
            "ask" => {
                if self.draw {
                    // (its box first, measured, so what is in it is drawn over it)
                    self.draw = false;
                    let h = self.block(b, x, y, w);
                    self.draw = true;
                    self.p.round(Rect::new(x, y, w, h), 12.0, AMBER.alpha(0.1));
                }
                let inner = w - 20.0;
                let mut h = 10.0;
                self.text(s(b, "label"), x + 10.0, y + h + 7.0, 11.0, Weight::Semibold, inner * 0.6, AMBER);
                let side = s(b, "side");
                let sw = self.p.measure(side, 10.5, Weight::Regular, inner * 0.4);
                self.text(side, x + 10.0 + inner - sw, y + h + 7.0, 10.5, Weight::Regular, inner * 0.4, DIM);
                h += 20.0;
                if !s(b, "text").is_empty() {
                    self.icon(if s(b, "icon").is_empty() { "term" } else { s(b, "icon") }, x + 10.0, y + h + 1.0, 14.0, DIM);
                    h += self.p.paragraph(s(b, "text"), x + 30.0, y + h, 11.5, inner - 20.0, 3, TEXT, self.draw) + 6.0;
                }
                let buttons = b["buttons"].as_array().cloned().unwrap_or_default();
                if !buttons.is_empty() {
                    let mut bx = x + 10.0;
                    for bt in &buttons {
                        let mut bt = bt.clone();
                        bt["icon"] = json!(if s(&bt, "tone") == "ok" { "check" } else if s(&bt, "tone") == "no" { "x" } else { "" });
                        bx += self.button(&bt, bx, y + h + 17.0, true) + 10.0;
                    }
                    h += 40.0;
                }
                h + 4.0
            }
            "field" => {
                // (pressed: a box to type in opens over it; what it holds, else its hint)
                let r = Rect::new(x, y, w, 30.0);
                self.round(r, 10.0, FAINT);
                // (a button of its own at its end: words, as "Aa")
                let extra = if b["button"].is_object() {
                    let bt = &b["button"];
                    let br = Rect::new(x + w - 34.0, y + 3.0, 30.0, 24.0);
                    self.ghost(br, 8.0);
                    let t = s(bt, "text");
                    let tw = self.p.measure(t, 11.0, Weight::Semibold, 30.0);
                    self.text(t, br.x + br.w / 2.0 - tw / 2.0, br.y + 12.0, 11.0, Weight::Semibold, 30.0, TEXT);
                    self.hit_tip(br, Press::Action(s(bt, "action").to_string(), Value::Null), bt["tip"].clone());
                    34.0
                } else {
                    0.0
                };
                let hint = if s(b, "placeholder").is_empty() { "Type here" } else { s(b, "placeholder") };
                if s(b, "value").is_empty() {
                    self.text(hint, x + 10.0, y + 15.0, 11.5, Weight::Regular, w - 20.0 - extra, DIM);
                } else {
                    self.text(s(b, "value"), x + 10.0, y + 15.0, 11.5, Weight::Regular, w - 44.0 - extra, TEXT);
                    // (an x: cleared, told at once)
                    if !s(b, "action").is_empty() {
                        let xr = Rect::new(x + w - 28.0 - extra, y + 3.0, 24.0, 24.0);
                        if self.draw {
                            self.p.circle(xr.x + 12.0, xr.y + 12.0, 9.0, FAINT);
                        }
                        self.icon("x", xr.x + 7.0, xr.y + 7.0, 10.0, DIM);
                        self.hit(xr, Press::Action(s(b, "action").to_string(), json!({ "text": "" })));
                    }
                }
                if !s(b, "action").is_empty() {
                    self.hit(r, Press::Field(s(b, "action").to_string(), b["live"] == true, hint.to_string(), s(b, "value").to_string()));
                }
                30.0
            }
            "head" => {
                let cy = y + 14.0;
                let t = s(b, "title");
                let tw = self.p.measure(t, 13.5, Weight::Semibold, w - 70.0);
                self.text(t, x + w / 2.0 - tw / 2.0, cy, 13.5, Weight::Semibold, w - 70.0, TEXT);
                if !s(b, "press").is_empty() {
                    self.hit(Rect::new(x + w / 2.0 - tw / 2.0 - 6.0, cy - 12.0, tw + 12.0, 24.0), Press::Action(s(b, "press").to_string(), Value::Null));
                }
                for (k, icon, bx) in [("prev", "prev", x), ("next", "next", x + w - 26.0)] {
                    let a = s(b, k);
                    if !a.is_empty() {
                        if self.draw {
                            self.p.circle(bx + 13.0, cy, 13.0, FAINT);
                        }
                        self.icon(icon, bx + 6.0, cy - 7.0, 14.0, TEXT);
                        self.hit(Rect::new(bx, cy - 13.0, 26.0, 26.0), Press::Action(a.to_string(), Value::Null));
                    }
                }
                28.0
            }
            "grid" => {
                let labels = b["labels"].as_array().cloned().unwrap_or_default();
                let cells = b["cells"].as_array().cloned().unwrap_or_default();
                let cw = w / 7.0;
                for (i, l) in labels.iter().enumerate().take(7) {
                    let t = l.as_str().unwrap_or("");
                    let tw = self.p.measure(t, 10.0, Weight::Semibold, cw);
                    self.text(t, x + i as f32 * cw + cw / 2.0 - tw / 2.0, y + 8.0, 10.0, Weight::Semibold, cw, DIM);
                }
                let top = y + 18.0;
                let ch = 30.0;
                for (i, c) in cells.iter().enumerate() {
                    let (col, row) = ((i % 7) as f32, (i / 7) as f32);
                    let cx = x + col * cw + cw / 2.0;
                    let cy = top + row * ch + ch / 2.0 - 2.0;
                    let state = s(c, "state");
                    if state == "sel" {
                        if self.draw {
                            self.p.circle(cx, cy, 12.0, TEXT);
                        }
                    } else if state == "today" && self.draw {
                        self.p.circle(cx, cy, 12.0, accent().alpha(0.3));
                    }
                    let t = s(c, "t");
                    let tw = self.p.measure(t, 11.5, Weight::Semibold, cw);
                    let col_c = match state {
                        "sel" => FACE,
                        "other" => DIM.alpha(0.5),
                        _ => TEXT,
                    };
                    self.text(t, cx - tw / 2.0, cy, 11.5, Weight::Semibold, cw, col_c);
                    let dots = c["dots"].as_u64().unwrap_or(0).min(3) as f32;
                    if self.draw {
                        for d in 0..dots as usize {
                            self.p.circle(cx - (dots - 1.0) * 2.5 + d as f32 * 5.0, cy + 11.5, 1.5, if state == "sel" { accent() } else { accent().alpha(0.9) });
                        }
                    }
                    let id = s(c, "id");
                    if !id.is_empty() {
                        self.hit(Rect::new(cx - cw / 2.0, cy - ch / 2.0, cw, ch), Press::Action(id.to_string(), Value::Null));
                    }
                }
                18.0 + (cells.len() as f32 / 7.0).ceil() * ch
            }
            "section" => self.section(b, x, y, w),
            "icons" => {
                // small pictures in a grid, as many to a row as fit, each pressed for what it names
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let (tw, gap, th) = (ICON_TILE, ICON_GAP, ICON_TILE);
                let cols = ((w + gap) / (tw + gap)).floor().max(1.0);
                for (i, it) in items.iter().enumerate() {
                    let (col, row) = ((i as f32 % cols).floor(), (i as f32 / cols).floor());
                    let r = Rect::new(x + col * (tw + gap), y + row * (th + gap), tw, th);
                    self.ghost(r, 8.0);
                    let z = 18.0;
                    let pr = Rect::new(r.x + r.w / 2.0 - z / 2.0, r.y + th / 2.0 - z / 2.0, z, z);
                    if self.draw && !self.p.picture(s(it, "image"), pr, 4.0) {
                        let letter: String = s(it, "label").chars().take(1).collect();
                        let lw = self.p.measure(&letter, 12.0, Weight::Semibold, z);
                        self.p.text(&letter, pr.x + z / 2.0 - lw / 2.0, pr.y + z / 2.0, 12.0, Weight::Semibold, z, TEXT);
                    }
                    let tip = if it["tip"].is_null() { it["label"].clone() } else { it["tip"].clone() };
                    self.hit_tip(r, Press::Action(s(it, "action").to_string(), Value::Null), tip);
                }
                let rows = (items.len() as f32 / cols).ceil();
                (rows * th + (rows - 1.0).max(0.0) * gap).max(0.0)
            }
            "infocard" => {
                // a wiki page's card: its picture (tall), its name and its facts beside it
                let has_pic = !s(b, "image").is_empty();
                let (pw, ph) = (96.0, 120.0);
                let tx = if has_pic { x + pw + 12.0 } else { x };
                let tw = w - (tx - x);
                if has_pic && self.draw {
                    let r = Rect::new(x, y, pw, ph);
                    if !self.p.picture(s(b, "image"), r, 12.0) {
                        self.p.round(r, 12.0, FAINT);
                    }
                }
                self.text(s(b, "title"), tx, y + 10.0, 15.0, Weight::Semibold, tw, TEXT);
                let mut h = 28.0;
                let facts = b["facts"].as_array().cloned().unwrap_or_default();
                let lw = facts.iter().map(|f| self.p.measure(f[0].as_str().unwrap_or(""), 11.5, Weight::Regular, tw * 0.45)).fold(0.0f32, f32::max);
                for f in &facts {
                    self.text(f[0].as_str().unwrap_or(""), tx, y + h + 8.0, 11.5, Weight::Regular, tw * 0.45, DIM);
                    self.text(f[1].as_str().unwrap_or(""), tx + lw + 12.0, y + h + 8.0, 11.5, Weight::Regular, tw - lw - 12.0, TEXT);
                    h += 19.0;
                }
                if !s(b, "action").is_empty() {
                    self.hit(Rect::new(x, y, w, h.max(if has_pic { ph } else { 0.0 })), Press::Action(s(b, "action").to_string(), b["args"].clone()));
                }
                h.max(if has_pic { ph } else { 0.0 })
            }
            "article" => {
                let has_pic = !s(b, "image").is_empty();
                let pic = 64.0;
                let tx = if has_pic { x + pic + 12.0 } else { x };
                let tw = w - (tx - x);
                if has_pic && self.draw {
                    let r = Rect::new(x, y, pic, pic);
                    if !self.p.picture(s(b, "image"), r, 10.0) {
                        self.p.round(r, 10.0, FAINT);
                    }
                }
                let mut h = 0.0;
                self.text(s(b, "title"), tx, y + 9.0, 13.0, Weight::Semibold, tw, TEXT);
                h += 20.0;
                let lines = b["lines"].as_u64().unwrap_or(6) as usize;
                h += self.p.paragraph(s(b, "text"), tx, y + h, 11.5, tw, lines, TEXT.alpha(0.82), self.draw);
                if !s(b, "action").is_empty() {
                    self.hit(Rect::new(x, y, w, h.max(if has_pic { pic } else { 0.0 })), Press::Action(s(b, "action").to_string(), b["args"].clone()));
                }
                h.max(if has_pic { pic } else { 0.0 })
            }
            "traytiles" => {
                // the tray's apps, a tile each: its icon (as small as the app made it), its name, what it says now
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let cols = (items.len().clamp(1, TRAY_COLS)) as f32;
                let gap = 8.0;
                let tw = (w - (cols - 1.0) * gap) / cols;
                for (i, it) in items.iter().enumerate() {
                    let (col, row) = ((i as f32 % cols).floor(), (i as f32 / cols).floor());
                    let r = Rect::new(x + col * (tw + gap), y + row * (TRAY_TILE_H + gap), tw, TRAY_TILE_H);
                    self.ghost(r, 14.0);
                    let z = 18.0;
                    let pr = Rect::new(r.x + r.w / 2.0 - z / 2.0, r.y + 12.0, z, z);
                    if self.draw && !self.p.picture(s(it, "image"), pr, 4.0) {
                        let letter: String = s(it, "name").chars().take(1).collect();
                        let lw = self.p.measure(&letter, 12.0, Weight::Semibold, z);
                        self.p.text(&letter, pr.x + z / 2.0 - lw / 2.0, pr.y + z / 2.0, 12.0, Weight::Semibold, z, TEXT);
                    }
                    let inner = r.w - 12.0;
                    let name = s(it, "name");
                    let nw = self.p.measure(name, 11.0, Weight::Semibold, inner);
                    self.text(name, r.x + r.w / 2.0 - nw / 2.0, r.y + 42.0, 11.0, Weight::Semibold, inner, TEXT);
                    let st = s(it, "status");
                    if !st.is_empty() {
                        let sw = self.p.measure(st, 9.5, Weight::Regular, inner);
                        self.text(st, r.x + r.w / 2.0 - sw / 2.0, r.y + 56.0, 9.5, Weight::Regular, inner, DIM);
                    }
                    self.hit_tip(r, Press::Action(s(it, "action").to_string(), Value::Null), it["tip"].clone());
                }
                let rows = (items.len() as f32 / cols).ceil();
                (rows * TRAY_TILE_H + (rows - 1.0).max(0.0) * gap).max(0.0)
            }
            "covers" => {
                // covers (a game's tall picture) side by side, four to a row: each pressed for what it does; one that updates, a
                // bar along its foot; its name under it
                let items = b["items"].as_array().cloned().unwrap_or_default();
                if items.is_empty() {
                    return 0.0;
                }
                let cols = (s(b, "cols").parse::<usize>().ok()).or(b["cols"].as_u64().map(|c| c as usize)).unwrap_or(4).max(1);
                let gap = 8.0;
                let cw = (w - gap * (cols - 1) as f32) / cols as f32;
                let ch = (cw * 1.5).round();
                let row_h = ch + 20.0;
                for (i, it) in items.iter().enumerate() {
                    let (col, row) = (i % cols, i / cols);
                    let r = Rect::new(x + col as f32 * (cw + gap), y + row as f32 * (row_h + 10.0), cw, ch);
                    if self.draw && !self.p.picture(s(it, "image"), r, 10.0) {
                        self.p.round(r, 10.0, FAINT);
                        let letter: String = s(it, "title").chars().take(1).collect();
                        let lw = self.p.measure(&letter, 22.0, Weight::Semibold, cw);
                        self.p.text(&letter, r.x + cw / 2.0 - lw / 2.0, r.y + ch / 2.0, 22.0, Weight::Semibold, cw, DIM);
                    }
                    if let (Some(f), true) = (it["progress"].as_f64(), self.draw) {
                        let bar = Rect::new(r.x + 6.0, r.bottom() - 10.0, r.w - 12.0, 4.0);
                        self.p.round(bar, 2.0, crate::paint::Color(0.0, 0.0, 0.0, 0.55));
                        self.p.round(Rect::new(bar.x, bar.y, bar.w * f.clamp(0.0, 1.0) as f32, bar.h), 2.0, crate::paint::Color(0.31, 0.56, 0.94, 1.0));
                    }
                    self.hit_tip(r, Press::Action(s(it, "action").to_string(), Value::Null), it["title"].clone());
                    self.text(s(it, "title"), r.x + 1.0, r.bottom() + 10.0, 11.0, Weight::Regular, cw - 2.0, TEXT);
                }
                let rows = items.len().div_ceil(cols);
                rows as f32 * row_h + (rows - 1) as f32 * 10.0
            }
            "windows" => {
                // windows side by side, centred, as many rows as they need: each its live picture (else its app's picture), and under
                // it its app's small picture and its title
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let n = items.len();
                if n == 0 {
                    return 0.0;
                }
                let cols = (((w + WIN_GAP) / (WIN_W + WIN_GAP)).floor() as usize).max(1);
                let rows = n.div_ceil(cols);
                let ph = (WIN_PIC * 0.625).round();
                let row_h = ph + 24.0;
                for (i, it) in items.iter().enumerate() {
                    let (row, col) = (i / cols, i % cols);
                    let in_row = (n - row * cols).min(cols) as f32;
                    let x0 = x + (w - (in_row * WIN_W + (in_row - 1.0) * WIN_GAP)) / 2.0;
                    let cell = x0 + col as f32 * (WIN_W + WIN_GAP);
                    let by = y + row as f32 * (row_h + WIN_GAP);
                    // (its use on the picture's left, how far its download is on its right)
                    let bx = cell + crate::peek::METERS;
                    let r = Rect::new(bx, by, WIN_PIC, ph);
                    // (it wants you: it glows)
                    if self.draw && it["attention"] == true {
                        self.p.round(Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0), 13.0, crate::draw::AMBER.alpha(0.35));
                    }
                    if self.draw && !it["cpu"].is_null() {
                        crate::peek::use_bars(self.p, it, cell, r.bottom() - 6.0, ph - 12.0);
                    }
                    if let (Some(f), true) = (it["progress"].as_f64(), self.draw) {
                        crate::peek::progress_bar(self.p, f as f32, r.right() + 5.0, r.bottom() - 6.0, ph - 12.0);
                    }
                    let hwnd = it["hwnd"].as_i64().unwrap_or(0) as isize;
                    // (a ghost; the window in front: a dot under it, in the theme colour)
                    self.ghost(r, 10.0);
                    if it["front"] == true && self.draw {
                        self.p.circle(r.x + r.w / 2.0, r.bottom() + 3.0, 1.8, accent());
                    }
                    let img = s(it, "image");
                    if hwnd != 0 {
                        if self.draw {
                            self.thumbs.push((hwnd, Rect::new(r.x + 3.0, r.y + 3.0, r.w - 6.0, r.h - 6.0)));
                        }
                    } else if self.draw {
                        let z = 40.0;
                        let pr = Rect::new(r.x + r.w / 2.0 - z / 2.0, r.y + r.h / 2.0 - z / 2.0, z, z);
                        if img.is_empty() || !self.p.picture(img, pr, 9.0) {
                            let icon = if s(it, "icon").is_empty() { "dot" } else { s(it, "icon") };
                            self.icon(icon, pr.x + 8.0, pr.y + 8.0, 24.0, TEXT);
                        }
                    }
                    self.hit_tip(r, Press::Action(s(it, "action").to_string(), Value::Null), it["title"].clone());
                    self.last_hit(false, it["menu"].clone());
                    let cy = by + ph + 13.0;
                    let mut tx = bx + 2.0;
                    // (making sound: its small bars at the end of its title)
                    let mw = if it["audio"].is_array() { 18.0 } else { 0.0 };
                    if let (Some(v), true) = (it["audio"].as_array(), self.draw) {
                        crate::peek::sound_bars(self.p, v, r.right() - 8.0, cy, 9.0);
                    }
                    if hwnd != 0 && !img.is_empty() {
                        if self.draw {
                            self.p.picture(img, Rect::new(tx, cy - 7.0, 14.0, 14.0), 3.0);
                        }
                        tx += 19.0;
                    }
                    self.text(s(it, "title"), tx, cy, 11.0, Weight::Regular, r.right() - tx - mw, TEXT);
                }
                rows as f32 * row_h + (rows - 1) as f32 * WIN_GAP
            }
            "thumbs" => {
                // the windows of an app, side by side, each a live picture with its title under it (and a button to close it)
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let n = items.len().clamp(1, 3) as f32;
                let gap = 8.0;
                let tw = (w - gap * (n - 1.0)) / n;
                let th = tw * 0.62;
                let mut h = 0.0;
                for (i, it) in items.iter().enumerate() {
                    let (col, row) = ((i % 3) as f32, (i / 3) as f32);
                    let bx = x + col * (tw + gap);
                    let by = y + row * (th + 28.0);
                    let r = Rect::new(bx, by, tw, th);
                    self.ghost(r, 10.0);
                    if self.draw {
                        self.thumbs.push((it["hwnd"].as_i64().unwrap_or(0) as isize, Rect::new(r.x + 4.0, r.y + 4.0, r.w - 8.0, r.h - 8.0)));
                    }
                    self.hit(r, Press::Action(s(it, "action").to_string(), Value::Null));
                    self.text(s(it, "title"), bx + 2.0, by + th + 11.0, 11.0, Weight::Regular, tw - 24.0, TEXT);
                    let close = s(it, "close");
                    if !close.is_empty() {
                        let cx = bx + tw - 10.0;
                        self.icon("x", cx - 6.0, by + th + 5.0, 12.0, DIM);
                        self.hit(Rect::new(cx - 10.0, by + th + 1.0, 20.0, 20.0), Press::Action(close.to_string(), Value::Null));
                    }
                    h = (by - y) + th + 22.0;
                }
                h
            }
            "slider" => {
                // a bar to set a value on (pressed where it should be), with its icon (which may do something of its own)
                let cy = y + 14.0;
                let icon = s(b, "icon");
                let image = s(b, "image");
                let mut bx = x;
                if !icon.is_empty() || !image.is_empty() {
                    if self.draw {
                        self.p.circle(x + 13.0, cy, 13.0, FAINT);
                    }
                    let muted = b["muted"] == true;
                    if !image.is_empty() && self.draw {
                        if !self.p.picture(image, Rect::new(x + 5.0, cy - 8.0, 16.0, 16.0), 3.0) {
                            self.icon("music", x + 5.0, cy - 8.0, 16.0, TEXT);
                        }
                        // (muted: a line through it)
                        if muted {
                            self.p.round(Rect::new(x + 3.0, cy - 1.0, 20.0, 2.0), 1.0, RED);
                        }
                    } else if !icon.is_empty() {
                        self.icon(icon, x + 5.0, cy - 8.0, 16.0, TEXT);
                    }
                    if !s(b, "press").is_empty() {
                        self.hit(Rect::new(x, cy - 13.0, 26.0, 26.0), Press::Action(s(b, "press").to_string(), Value::Null));
                    }
                    bx += 36.0;
                }
                let t = s(b, "text");
                let tw = self.p.measure(t, 11.5, Weight::Semibold, 70.0).max(34.0);
                let bar = Rect::new(bx, cy - 3.0, w - (bx - x) - tw - 10.0, 6.0);
                if self.draw {
                    self.p.bar(bar, f(b, "value"), TEXT);
                    let kx = bar.x + bar.w * f(b, "value").clamp(0.0, 1.0);
                    self.p.circle(kx, cy, 7.0, TEXT);
                }
                self.hit(Rect::new(bar.x - 6.0, cy - 13.0, bar.w + 12.0, 26.0), Press::Seek(s(b, "action").to_string()));
                self.text(t, x + w - tw, cy, 11.5, Weight::Semibold, 70.0, TEXT);
                28.0
            }
            _ => 0.0,
        }
    }

    /// A table (a wiki's): rows of cells, each words, or small pictures each with its words ("8", "1.6K"); a cell may take
    /// several columns (`span`). The first row its head when `head`. Its columns as wide as their widest, shrunk to fit.
    fn table(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        const ROW: f32 = 26.0;
        const PIC: f32 = 20.0;
        const FS: f32 = 10.5;
        let rows = b["rows"].as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            return 0.0;
        }
        let head = b["head"] == true;
        // (a cell: its pieces, each a picture (maybe none) and words)
        let pieces = |c: &Value| -> Vec<(String, String)> {
            if let Some(items) = c["items"].as_array() {
                items.iter().map(|i| (s(i, "image").to_string(), s(i, "text").to_string())).collect()
            } else if let Some(t) = c.as_str() {
                vec![(String::new(), t.to_string())]
            } else {
                vec![(s(c, "image").to_string(), s(c, "text").to_string())]
            }
        };
        let span = |c: &Value| c["span"].as_u64().unwrap_or(1).max(1) as usize;
        let cols = rows.iter().map(|r| r.as_array().map_or(0, |cs| cs.iter().map(span).sum::<usize>())).max().unwrap_or(0).clamp(1, 8);
        // (a picture whole, in its proportion; one not here yet as wide as it is tall)
        let pic_w = |p: &Self, img: &str| -> f32 {
            let w = p.p.picture_fit(img, 0.0, 0.0, PIC, false);
            if w > 0.0 { w } else { PIC }
        };
        let piece_w = |p: &Self, ps: &[(String, String)]| -> f32 {
            ps.iter().map(|(img, t)| (if img.is_empty() { 0.0 } else { pic_w(p, img) + 3.0 }) + if t.is_empty() { 0.0 } else { p.p.measure(t, FS, Weight::Regular, 200.0) } + 6.0).sum::<f32>()
        };
        // (each column as wide as its widest single cell; then all of them shrunk to fit, none narrower than a little)
        let mut cw = vec![24.0f32; cols];
        for r in &rows {
            let mut c = 0;
            for cell in r.as_array().into_iter().flatten() {
                let n = span(cell);
                if n == 1 && c < cols {
                    cw[c] = cw[c].max(piece_w(self, &pieces(cell)) + 8.0);
                }
                c += n;
            }
        }
        let total: f32 = cw.iter().sum();
        if total > w {
            let k = w / total;
            cw.iter_mut().for_each(|v| *v = (*v * k).max(22.0));
        } else {
            // (room to spare: to the columns of words, the more the wider they are; a column of a picture alone stays as it is)
            let weights: Vec<f32> = cw.iter().map(|v| (v - 40.0).max(0.0)).collect();
            let sum: f32 = weights.iter().sum();
            let spare = w - total;
            for (v, wt) in cw.iter_mut().zip(&weights) {
                *v += if sum > 0.0 { spare * wt / sum } else { spare / cols as f32 };
            }
        }
        let mut cy = y;
        for (ri, r) in rows.iter().enumerate() {
            let is_head = head && ri == 0;
            // (its head on a band of its own; the rows after it striped)
            if self.draw && (is_head || (ri + usize::from(head)) % 2 == 0) {
                self.p.round(Rect::new(x, cy, w, ROW), 6.0, if is_head { FAINT } else { FAINT.alpha(0.45) });
            }
            let mut c = 0;
            let mut cx = x;
            for cell in r.as_array().into_iter().flatten() {
                if c >= cols {
                    break;
                }
                let n = span(cell).min(cols - c);
                let width: f32 = cw[c..c + n].iter().sum();
                let ps = pieces(cell);
                // (one alone, and numbers: in the middle; else from the left)
                let pw = piece_w(self, &ps);
                // (short words and numbers in the middle; a sentence from the left)
                let short = pw <= 72.0;
                let mut px = if short && (n > 1 || (ps.len() == 1 && ps[0].0.is_empty() && c > 0)) { cx + ((width - pw) / 2.0).max(4.0) } else { cx + 4.0 };
                let room = cx + width - 2.0;
                for (img, t) in &ps {
                    if px >= room {
                        break;
                    }
                    if !img.is_empty() {
                        let pw = self.p.picture_fit(img, px, cy + ROW / 2.0, PIC, self.draw);
                        px += if pw > 0.0 { pw } else { PIC } + 3.0;
                    }
                    if !t.is_empty() {
                        px += self.text(t, px, cy + ROW / 2.0, FS, if is_head || c == 0 { Weight::Semibold } else { Weight::Regular }, (room - px).max(0.0), if is_head { DIM } else { TEXT });
                    }
                    px += 6.0;
                }
                cx += width;
                c += n;
            }
            cy += ROW;
        }
        cy - y
    }

    /// A section: a head of its own (a tile, a title, a line), which opens and closes it, and its body under it.
    fn section(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        let id = s(b, "id").to_string();
        let open = self.open.contains(&id) || (!self.closed.contains(&id) && b["first"].as_bool() == Some(true));
        let head = 40.0;
        let cy = y + head / 2.0;
        let mut tx = x;
        if let Some(live) = b["live"].as_object() {
            let tone = live.get("tone").and_then(|t| t.as_str()).unwrap_or("");
            let c = tone_color(tone);
            if self.draw {
                // (asking for you: the row tinted)
                if matches!(tone, "waiting" | "permission") {
                    self.p.round(Rect::new(x - 6.0, y + 1.0, w + 12.0, head - 2.0), 14.0, crate::draw::AMBER.alpha(0.12));
                }
                // (its state is the tile's own: what it shows says it; asking for you, a pulse round it)
                if matches!(tone, "waiting" | "permission") {
                    let ph = (self.p.t * 0.8).fract();
                    self.p.circle(x + 15.0, cy, 15.0 * (1.0 + 0.45 * ph), c.alpha(0.4 * (1.0 - ph)));
                }
                if b["tile"].is_object() {
                    self.p.tile(&b["tile"], x, cy - 15.0, 30.0, "sparkle");
                } else {
                    self.p.circle(x + 15.0, cy, 15.0, c.alpha(0.18));
                }
            }
            tx += 40.0;
        } else if b["tile"].is_object() {
            if self.draw {
                self.p.tile(&b["tile"], x, cy - 15.0, 30.0, "dot");
            }
            tx += 40.0;
        }
        if !s(b, "press").is_empty() {
            self.hit(Rect::new(x, cy - 15.0, 30.0, 30.0), Press::Action(s(b, "press").to_string(), Value::Null));
        }
        let has_body = b["body"].as_array().is_some_and(|a| !a.is_empty());
        let chevron = if has_body { 18.0 } else { 0.0 };
        // (on its right: its buttons, else how long)
        let mut rx = x + w - chevron;
        if let Some(bts) = b["buttons"].as_array().filter(|a| !a.is_empty()) {
            for bt in bts.iter().rev() {
                let label = s(bt, "label");
                let bw = self.p.measure(label, 11.0, Weight::Semibold, 100.0) + 20.0;
                rx -= bw;
                let (bg, fg) = match s(bt, "tone") {
                    "ok" => (GREEN.alpha(0.22), GREEN),
                    "no" => (RED.alpha(0.2), RED),
                    _ => (FAINT, TEXT),
                };
                let r = Rect::new(rx, cy - 12.0, bw, 24.0);
                self.round(r, 12.0, bg);
                self.text(label, rx + 10.0, cy, 11.0, Weight::Semibold, 100.0, fg);
                self.hit(r, Press::Action(s(bt, "action").to_string(), Value::Null));
                rx -= 6.0;
            }
        } else if !s(b, "right").is_empty() {
            let rw = self.p.measure(s(b, "right"), 10.5, Weight::Regular, 90.0);
            self.text_end(s(b, "right"), rx - 2.0, cy, 10.5, Weight::Regular, 90.0, DIM);
            rx -= rw + 10.0;
        }
        let room = rx - tx;
        self.text(s(b, "title"), tx, cy - 7.5, 12.5, Weight::Semibold, room, TEXT);
        self.text(s(b, "sub"), tx, cy + 8.5, 10.5, Weight::Regular, room, DIM);
        if has_body {
            self.icon(if open { "chev-up" } else { "chev-down" }, x + w - 14.0, cy - 7.0, 14.0, DIM);
            self.hit(Rect::new(tx, y, w - (tx - x), head), Press::Toggle(id.clone()));
        }
        let mut h = head;
        if open && has_body {
            h += 6.0;
            for blk in b["body"].as_array().cloned().unwrap_or_default() {
                let bh = self.block(&blk, x, y + h, w);
                if bh > 0.0 {
                    h += bh + SP;
                }
            }
            h += 2.0;
        }
        h
    }

    /// The body: every block, one under the other; how tall.
    /// A panel's body as big as it asks (`zoom`: its text and all else that much bigger); how tall.
    pub fn body_zoomed(&mut self, body: &[Value], x: f32, y: f32, w: f32, zoom: f32) -> f32 {
        if (zoom - 1.0).abs() < 0.01 {
            return self.body(body, x, y, w);
        }
        let zp = Painter { g: self.p.g, dc: self.p.dc, s: self.p.s * zoom, pc: self.p.pc, t: self.p.t, viz: self.p.viz };
        let mut c = Ctx { p: &zp, draw: self.draw, hits: Vec::new(), open: self.open, closed: self.closed, thumbs: Vec::new() };
        let h = c.body(body, x / zoom, y / zoom, w / zoom);
        let up = |r: Rect| Rect::new(r.x * zoom, r.y * zoom, r.w * zoom, r.h * zoom);
        self.hits.extend(c.hits.into_iter().map(|h| Hit { rect: up(h.rect), ..h }));
        self.thumbs.extend(c.thumbs.into_iter().map(|(w, r)| (w, up(r))));
        h * zoom
    }

    pub fn body(&mut self, body: &[Value], x: f32, y: f32, w: f32) -> f32 {
        let mut h = 0.0;
        let mut first_section = true;
        let mut prev_section = false;
        for b in body {
            let mut b = b.clone();
            let section = s(&b, "kind") == "section";
            if section && first_section {
                b["first"] = json!(true);
                first_section = false;
            }
            if section && prev_section && self.draw {
                self.p.round(Rect::new(x, y + h - SP / 2.0 - 0.5, w, 1.0), 0.5, FAINT);
            }
            let bh = self.block(&b, x, y + h, w);
            if bh > 0.0 {
                h += bh + SP;
            }
            prev_section = section;
        }
        (h - SP).max(0.0)
    }
}

/// The head of a panel: its tile, its title, the line under it. How tall.
/// An icon grid's tile, and the gap between tiles.
pub const ICON_TILE: f32 = 30.0;
/// the tray open: its tiles, so many to a row, so tall, so wide
pub const TRAY_COLS: usize = 3;
pub const TRAY_TILE_H: f32 = 70.0;
pub const TRAY_TILE_W: f32 = 100.0;
pub const ICON_GAP: f32 = 6.0;

pub fn head(c: &mut Ctx, panel: &Value, x: f32, y: f32, w: f32) -> f32 {
    let tile = &panel["tile"];
    // (no title: no head; the panel starts with what is in it)
    if s(panel, "title").is_empty() && s(panel, "sub").is_empty() {
        return 0.0;
    }
    let mut tx = x;
    if tile.is_object() {
        if c.draw {
            c.p.tile(tile, x, y, 40.0, "dot");
        }
        tx += 50.0;
    }
    let title = s(panel, "title");
    let sub = s(panel, "sub");
    c.text(title, tx, y + if sub.is_empty() { 20.0 } else { 12.0 }, 15.0, Weight::Semibold, w - (tx - x), TEXT);
    if !sub.is_empty() {
        c.text(sub, tx, y + 30.0, 11.5, Weight::Regular, w - (tx - x), DIM);
    }
    if !s(panel, "press").is_empty() {
        c.hit(Rect::new(x, y, w, 40.0), Press::Action(s(panel, "press").to_string(), Value::Null));
    }
    40.0
}
