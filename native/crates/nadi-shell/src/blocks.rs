//! A widget open: its panel. A head (its tile, its title, a line under it) and its body, a column of blocks the plugin describes (words,
//! facts, numbers, chips, meters, rings, lists, a graph, a seek bar, buttons, a month, sections of their own). Each block is measured and
//! drawn by one function (drawn or not), so what is measured is what is drawn; what can be pressed leaves a box behind.

use crate::draw::*;
use crate::paint::Weight;
use crate::shape::Rect;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// What a press on a box of the panel does.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// the plugin is told (`action`, with `args`)
    Action(String, Value),
    /// a section opens or closes
    Toggle(String),
    /// a bar to seek on: the plugin is told where (`fraction`)
    Seek(String),
    /// a field to type in: the plugin is told what was typed (`text`); live, at every change. Its hint, what it holds, whether it is
    /// secret (a password: dots shown, never what is typed), and how its box looks (`{ ground, ink, size, bold, area }`: in a card's
    /// colours, words of several lines; none: the plain box)
    Field(String, bool, String, String, bool, Value),
    /// a picture to look at closely (its name): the wheel zooms it, dragging moves it, a double-click fits it again
    View(String),
    /// a card that can be carried (held a moment, then dragged): pressed quickly, its action; its id
    Card(String, String),
    /// a point on a curve (its id; none: the curve's ground, where a double-click adds one) and its curve's block (its axes, its
    /// actions, and `area`: where it is drawn, in the panel)
    Point(String, Value),
    /// a menu, opened under it (its rows as a thing's own menu's)
    Menu(Value),
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
pub const WIN_GAP: f32 = 12.0;
/// the open dock's tiles: how wide at least (wider to fill a row), how far apart
pub const TILE_W: f32 = 210.0;
pub const TILE_GAP: f32 = 12.0;

thread_local! {
    /// what is drawn over each window's live picture in the panel just drawn (Windows draws the pictures over the panel: these go on
    /// the glass over it): its window, its box, its tile, its app
    static OVERS: std::cell::RefCell<Vec<(isize, Rect, Value, Value)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What was drawn over the live pictures of the panel just drawn (taken).
pub fn take_overs() -> Vec<(isize, Rect, Value, Value)> {
    OVERS.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

/// A window's tile in the open dock, what goes over its picture: a dark fade down from its top, its app's icon and its title at its
/// top left; what its app uses at its top right (two thin bars: the processor, the memory); at its foot, how far its download is, or,
/// making sound, its volume (how loud it is now lighter over it, as the mixer shows it); the mouse on it, that grows into a bar: a
/// speaker that mutes it, its volume to drag, and what it plays, back, play or pause, next. (`hover`: where the mouse is, in the
/// same space.)
pub fn tile_over(p: &crate::draw::Painter, r: Rect, it: &Value, g: &Value, hover: Option<(f32, f32)>) {
    let on = hover.is_some_and(|(x, y)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h);
    let black = |a: f32| crate::paint::Color(0.0, 0.0, 0.0, a);
    // (its corners rounded: Windows draws its picture square, over the panel; the panel's colour laid over the picture's corners)
    let rad = crate::paint::rounded(12.0, r.w, r.h);
    if rad > 0.5 {
        let band = 6.0;
        for (cx, cy, a0) in [(r.x + rad, r.y + rad, 180.0f32), (r.right() - rad, r.y + rad, 270.0), (r.right() - rad, r.bottom() - rad, 0.0), (r.x + rad, r.bottom() - rad, 90.0)] {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let a = (a0 + i as f32 * 7.5).to_radians();
                (cx + (rad + band / 2.0) * a.cos(), cy + (rad + band / 2.0) * a.sin())
            }).collect();
            p.line(&pts, band, FACE);
        }
    }
    // (the one you were in: its ring, over its picture's corners)
    if it["front"] == true {
        p.outline(Rect::new(r.x - 1.5, r.y - 1.5, r.w + 3.0, r.h + 3.0), 13.5, 2.0, accent());
    }
    // (the fade, darkest at its top)
    p.fade_down(Rect::new(r.x, r.y, r.w, (r.h * 0.5).round()), black(0.75));
    let busy = g["cpu"].is_number();
    // its use: two thin bars lying down, on a dark chip
    if busy {
        let c = Rect::new(r.right() - 6.0 - 44.0, r.y + 6.0, 44.0, 18.0);
        p.round(c, 9.0, black(0.8));
        let cpu = s_num(g, "cpu");
        let ram = s_num(g, "ram");
        for (j, (v, col)) in [((cpu / 0.4).min(1.0), crate::peek::load_color(cpu)), ((ram / 0.25).min(1.0), crate::paint::Color(0.71, 0.49, 0.9, 1.0))].into_iter().enumerate() {
            let y = c.y + 5.5 + j as f32 * 5.0;
            p.round(Rect::new(c.x + 7.0, y, c.w - 14.0, 3.0), 1.5, crate::paint::Color(1.0, 1.0, 1.0, 0.15));
            p.round(Rect::new(c.x + 7.0, y, ((c.w - 14.0) * v).max(3.0), 3.0), 1.5, col);
        }
    }
    // its icon and its title
    let pic = Rect::new(r.x + 8.0, r.y + 7.0, 16.0, 16.0);
    if !p.picture(s(it, "image"), pic, 4.0) {
        p.round(pic, 4.0, GROUND);
    }
    let t = if s(it, "short").is_empty() { s(it, "title") } else { s(it, "short") };
    p.text(t, pic.right() + 6.0, pic.y + 8.0, 11.0, Weight::Semibold, (r.right() - 8.0 - if busy { 50.0 } else { 0.0 } - pic.right() - 6.0).max(20.0), TEXT);
    // its foot
    if let Some(f) = g["progress"].as_f64() {
        let lr = Rect::new(r.x + 8.0, r.bottom() - 9.0, r.w - 16.0, 3.0);
        p.round(lr, 1.5, crate::paint::Color(1.0, 1.0, 1.0, 0.16));
        p.round(Rect::new(lr.x, lr.y, lr.w * (f as f32).clamp(0.0, 1.0), 3.0), 1.5, crate::paint::Color(0.31, 0.56, 0.94, 1.0));
        return;
    }
    if g["sound"] != true || !g["volume"].is_number() {
        return;
    }
    let key = s(g, "key");
    let action = format!("appvol:{key}");
    let v = sliding(&action).unwrap_or(s_num(g, "volume")).clamp(0.0, 1.0);
    let muted = g["muted"] == true;
    let live = if muted { 0.0 } else { (live_level(meter(&format!("app:{key}"))) * v).clamp(0.0, v) };
    let line = |p: &crate::draw::Painter, lr: Rect| {
        p.round(lr, lr.h / 2.0, crate::paint::Color(1.0, 1.0, 1.0, 0.2));
        if v > 0.0 {
            p.round(Rect::new(lr.x, lr.y, (lr.w * v).max(lr.h), lr.h), lr.h / 2.0, if muted { SLIDER_DIM } else { accent() });
        }
        if live > 0.005 {
            p.round(Rect::new(lr.x, lr.y, (lr.w * live).max(lr.h), lr.h), lr.h / 2.0, LIVE);
        }
    };
    if !on {
        line(p, Rect::new(r.x + 8.0, r.bottom() - 9.0, r.w - 16.0, 3.0));
        return;
    }
    let (bar, mute, track, media) = tile_bar(r, g);
    p.round(bar, bar.h / 2.0, black(0.8));
    p.icon(if muted { "volume-x" } else { "volume" }, mute.x + 4.0, mute.y + 4.0, 14.0, if muted { RED } else { TEXT });
    line(p, Rect::new(track.x + SEEK_INSET, track.y + track.h / 2.0 - 2.0, track.w - 2.0 * SEEK_INSET, 4.0));
    p.circle(track.x + SEEK_INSET + (track.w - 2.0 * SEEK_INSET) * v, track.y + track.h / 2.0, 5.0, if muted { DIM } else { TEXT });
    if let Some([prev, play, next]) = media {
        let playing = g["media"]["playing"] == true;
        for (b, icon) in [(prev, "prev"), (play, if playing { "pause" } else { "play" }), (next, "next")] {
            p.icon(icon, b.x + 3.0, b.y + 3.0, 14.0, TEXT);
        }
    }
}

/// A tile's bar while the mouse is on it: the bar, its speaker, its volume's track, and what it plays (back, play, next), if it does.
fn tile_bar(r: Rect, g: &Value) -> (Rect, Rect, Rect, Option<[Rect; 3]>) {
    let bar = Rect::new(r.x + 6.0, r.bottom() - 36.0, r.w - 12.0, 30.0);
    let cy = bar.y + bar.h / 2.0;
    let mute = Rect::new(bar.x + 6.0, cy - 11.0, 22.0, 22.0);
    let media = g["media"]["source"].is_string();
    let end = if media { bar.right() - 6.0 - 3.0 * 20.0 } else { bar.right() - 8.0 };
    let track = Rect::new(mute.right() + 2.0, cy - 10.0, (end - mute.right() - 4.0).max(20.0), 20.0);
    let buttons = media.then(|| [0, 1, 2].map(|i| Rect::new(end + i as f32 * 20.0, cy - 10.0, 20.0, 20.0)));
    (bar, mute, track, buttons)
}

fn s_num(v: &Value, k: &str) -> f32 {
    v[k].as_f64().unwrap_or(0.0) as f32
}
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

/// a bar to seek on, or a slider: its press area this much wider than its track at each end (its knob goes that far past it)
pub const SEEK_INSET: f32 = 8.0;
const SLIDER_TRACK: crate::paint::Color = crate::paint::Color(0.165, 0.165, 0.188, 1.0);
const SLIDER_DIM: crate::paint::Color = crate::paint::Color(0.29, 0.29, 0.32, 1.0);

thread_local! {
    /// the bar being dragged (its action) and where its knob is, ahead of its plugin
    static SLIDING: std::cell::RefCell<Option<(String, f32)>> = const { std::cell::RefCell::new(None) };
    /// each slider drawn: its value, as its plugin gave it (the wheel steps from it)
    static SLIDERS: std::cell::RefCell<HashMap<String, f32>> = std::cell::RefCell::new(HashMap::new());
}

thread_local! {
    /// how loud each thing with a meter is now (0..1, by its name: "master", "mic", "app:<program>")
    static METERS: std::cell::RefCell<HashMap<String, f32>> = std::cell::RefCell::new(HashMap::new());
}

/// How loud each thing with a meter is now.
pub fn set_meters(m: HashMap<String, f32>) {
    METERS.with(|x| *x.borrow_mut() = m);
}

/// How loud a thing with a meter is now (0: not known).
pub fn meter(name: &str) -> f32 {
    METERS.with(|x| x.borrow().get(name).copied().unwrap_or(0.0))
}

/// How a picture is looked at: how near (1: whole), the point of it in the middle (0..1 across and down), when that last changed.
#[derive(Clone, Copy)]
struct Look {
    zoom: f32,
    ox: f32,
    oy: f32,
    at: Option<std::time::Instant>,
}

thread_local! {
    static LOOKS: std::cell::RefCell<HashMap<String, Look>> = std::cell::RefCell::new(HashMap::new());
    /// each picture drawn: its box (in its panel) and its size
    static VIEWS: std::cell::RefCell<HashMap<String, (Rect, f32, f32)>> = std::cell::RefCell::new(HashMap::new());
}

thread_local! {
    /// how much taller the panel being drawn is than its content (made so by hand): its picture takes it
    static FILL: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// How much taller the panel about to be drawn is than its content.
pub fn set_fill(v: f32) {
    FILL.with(|f| f.set(v));
}

const LOOK_WHOLE: Look = Look { zoom: 1.0, ox: 0.5, oy: 0.5, at: None };
/// how near a picture may be looked at
const ZOOM_MAX: f32 = 10.0;

fn look_of(id: &str) -> Look {
    LOOKS.with(|l| l.borrow().get(id).copied().unwrap_or(LOOK_WHOLE))
}

/// A look kept inside its picture: never past its edges, in the middle where it is smaller than its box.
fn keep_in(mut l: Look, r: Rect, iw: f32, ih: f32) -> Look {
    l.zoom = l.zoom.clamp(1.0, ZOOM_MAX);
    let k = (r.w / iw).min(r.h / ih) * l.zoom;
    let (dw, dh) = (iw * k, ih * k);
    l.ox = if dw <= r.w { 0.5 } else { let m = r.w / 2.0 / dw; l.ox.clamp(m, 1.0 - m) };
    l.oy = if dh <= r.h { 0.5 } else { let m = r.h / 2.0 / dh; l.oy.clamp(m, 1.0 - m) };
    l
}

fn set_look(id: &str, f: impl FnOnce(Look, Rect, f32, f32) -> Look) -> bool {
    let Some((r, iw, ih)) = VIEWS.with(|v| v.borrow().get(id).copied()) else { return false };
    let l = keep_in(f(look_of(id), r, iw, ih), r, iw, ih);
    LOOKS.with(|x| x.borrow_mut().insert(id.to_string(), Look { at: Some(std::time::Instant::now()), ..l }));
    true
}

/// The wheel over a picture (`lx`, `ly` in its panel): nearer or further, the point under the mouse staying under it.
pub fn view_wheel(id: &str, lx: f32, ly: f32, up: bool) -> bool {
    if id == DESKMAP {
        desk_zoom(lx, ly, up);
        return true;
    }
    set_look(id, |l, r, iw, ih| {
        let k = (r.w / iw).min(r.h / ih);
        let (mx, my) = (lx - (r.x + r.w / 2.0), ly - (r.y + r.h / 2.0));
        let (ux, uy) = (l.ox + mx / (iw * k * l.zoom), l.oy + my / (ih * k * l.zoom));
        let zoom = (l.zoom * if up { 1.25 } else { 0.8 }).clamp(1.0, ZOOM_MAX);
        Look { zoom, ox: ux - mx / (iw * k * zoom), oy: uy - my / (ih * k * zoom), ..l }
    })
}

/// A picture dragged by (`dx`, `dy`): it moves with the mouse.
pub fn view_pan(id: &str, dx: f32, dy: f32) -> bool {
    if id == DESKMAP {
        desk_pan(dx, dy);
        return true;
    }
    set_look(id, |l, r, iw, ih| {
        let k = (r.w / iw).min(r.h / ih) * l.zoom;
        Look { ox: l.ox - dx / (iw * k), oy: l.oy - dy / (ih * k), ..l }
    })
}

/// A picture fitted again, whole.
pub fn view_reset(id: &str) -> bool {
    set_look(id, |_, _, _, _| LOOK_WHOLE)
}

/// the desktops' map, as a picture to move about on
pub const DESKMAP: &str = "\u{0}deskmap";
/// a desktop's card on the map at its own size: its width; its place, a little more each way
const DESK_CW: f32 = 150.0;
const DESK_GAP: f32 = 16.0;

/// How the map is looked at: how near (its own size: 1), the point of the map (logical, its own space) in the middle of the panel,
/// and when it was last drawn (a while since: it starts on the desktop shown again).
#[derive(Clone, Copy)]
struct DeskView {
    scale: f32,
    ox: f32,
    oy: f32,
    drawn: Option<std::time::Instant>,
}

/// Where the map was last drawn: its box in its panel, a place's size (its own space), its first column and row, and how it was
/// looked at.
#[derive(Clone, Copy)]
pub struct DeskGeom {
    pub rect: Rect,
    pub cell: (f32, f32),
    pub min: (i32, i32),
    pub view: (f32, f32, f32),
}

thread_local! {
    static DESK_VIEW: std::cell::Cell<DeskView> = const { std::cell::Cell::new(DeskView { scale: 1.0, ox: 0.0, oy: 0.0, drawn: None }) };
    static DESK_GEOM: std::cell::Cell<Option<DeskGeom>> = const { std::cell::Cell::new(None) };
    /// a card being carried: its id, where the mouse is (in its panel), and where on the card it was held
    static DESK_CARRY: std::cell::RefCell<Option<(String, f32, f32, f32, f32)>> = const { std::cell::RefCell::new(None) };
    /// the map asked to go to a place (its column and row), springing there
    static DESK_GO: std::cell::Cell<Option<(i32, i32)>> = const { std::cell::Cell::new(None) };
    /// where the map glides to (its own space), each card where it is now (gliding to its place) and how far it has grown in, and
    /// the ring round the desktop shown, sliding
    static DESK_TO: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
    static DESK_ANIM: std::cell::RefCell<HashMap<String, (f32, f32, f32)>> = std::cell::RefCell::new(HashMap::new());
    static DESK_RING: std::cell::Cell<Option<Rect>> = const { std::cell::Cell::new(None) };
}

fn desk_pan(dx: f32, dy: f32) {
    DESK_VIEW.with(|v| {
        let mut d = v.get();
        d.ox -= dx / d.scale;
        d.oy -= dy / d.scale;
        v.set(d);
    });
}

fn desk_zoom(lx: f32, ly: f32, up: bool) {
    let Some(g) = DESK_GEOM.with(|x| x.get()) else { return };
    DESK_VIEW.with(|v| {
        let mut d = v.get();
        let (mx, my) = (lx - (g.rect.x + g.rect.w / 2.0), ly - (g.rect.y + g.rect.h / 2.0));
        let (ux, uy) = (d.ox + mx / d.scale, d.oy + my / d.scale);
        d.scale = (d.scale * if up { 1.15 } else { 1.0 / 1.15 }).clamp(0.35, 1.6);
        d.ox = ux - mx / d.scale;
        d.oy = uy - my / d.scale;
        v.set(d);
    });
}

/// The map moved by so much (logical px): a card carried to its edge.
pub fn desk_scroll(dx: f32, dy: f32) {
    desk_pan(-dx, -dy);
}

/// Where the map was last drawn.
pub fn desk_geom() -> Option<DeskGeom> {
    DESK_GEOM.with(|x| x.get())
}

/// A card carried (its id, the mouse in its panel, where on it it was held), or none.
pub fn set_desk_carry(c: Option<(String, f32, f32, f32, f32)>) {
    DESK_CARRY.with(|x| *x.borrow_mut() = c);
}

/// The map, to a place (springing there).
pub fn desk_go(c: i32, r: i32) {
    DESK_GO.with(|x| x.set(Some((c, r))));
}

/// The place (column, row) under a point of the panel, on the map as last drawn.
pub fn desk_cell_at(lx: f32, ly: f32) -> Option<(i32, i32)> {
    let g = desk_geom()?;
    let (scale, ox, oy) = g.view;
    let x = ox + (lx - (g.rect.x + g.rect.w / 2.0)) / scale;
    let y = oy + (ly - (g.rect.y + g.rect.h / 2.0)) / scale;
    Some(((x / g.cell.0).floor() as i32 + g.min.0, (y / g.cell.1).floor() as i32 + g.min.1))
}

thread_local! {
    /// a curve's point being dragged: its id, and where it is now (in its axes' values)
    static POINTING: std::cell::RefCell<Option<(String, f32, f32)>> = const { std::cell::RefCell::new(None) };
}

pub fn set_pointing(at: Option<(String, f32, f32)>) {
    POINTING.with(|s| *s.borrow_mut() = at);
}

/// Where a place in a curve's box is, in its axes' values (`spec`: the curve's block, its `area` in it).
pub fn curve_value(spec: &Value, lx: f32, ly: f32) -> (f32, f32) {
    let a = &spec["area"];
    let (ax, ay, aw, ah) = (f(a, "x"), f(a, "y"), f(a, "w").max(1.0), f(a, "h").max(1.0));
    let fx = ((lx - ax) / aw).clamp(0.0, 1.0);
    let fy = ((ly - ay) / ah).clamp(0.0, 1.0);
    let (x0, x1) = (f(&spec["x"], "min"), f(&spec["x"], "max"));
    let xv = if spec["x"]["log"] == true && x0 > 0.0 && x1 > 0.0 { x0 * (x1 / x0).powf(fx) } else { x0 + (x1 - x0) * fx };
    let (y0, y1) = (f(&spec["y"], "min"), f(&spec["y"], "max"));
    (xv, y1 - (y1 - y0) * fy)
}

/// The bar being dragged, and where its knob is (none: none is).
pub fn set_sliding(at: Option<(String, f32)>) {
    SLIDING.with(|s| *s.borrow_mut() = at);
}

/// Where a dragged (or wheeled) bar's knob is now, ahead of its plugin.
pub fn sliding_at(action: &str) -> Option<f32> {
    sliding(action)
}

fn sliding(action: &str) -> Option<f32> {
    SLIDING.with(|s| s.borrow().as_ref().filter(|(a, _)| a == action).map(|(_, v)| *v))
}

fn note_slider(action: &str, v: f32) {
    SLIDERS.with(|s| {
        s.borrow_mut().insert(action.to_string(), v);
    });
}

/// A slider's value, as last drawn (none: it is not a slider, a bar to seek on).
pub fn slider_value(action: &str) -> Option<f32> {
    SLIDERS.with(|s| s.borrow().get(action).copied())
}

thread_local! {
    /// the sliders the wheel does not turn (`wheel = false`: the wheel scrolls their panel instead)
    static NO_WHEEL: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
}

/// Does the wheel turn this slider?
pub fn slider_wheel(action: &str) -> bool {
    NO_WHEEL.with(|s| !s.borrow().contains(action))
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
/// a card's ground, inside a panel
const CARD: crate::paint::Color = crate::paint::Color(0.078, 0.082, 0.102, 1.0);
/// the faintest words: an axis's marks
const SOFT_DIM: crate::paint::Color = crate::paint::Color(0.43, 0.43, 0.47, 1.0);

/// now, in ms since 1970 (what plugins say times in)
pub fn now_ms() -> f64 {
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

    /// Pictures to choose from: a row of small ones, the one shown ringed in the theme's colour (each pressed: its action; its
    /// right-click: its `menu`); at its end, a button (`add`). How tall.
    fn album(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        // (in from its edges by its ring, so the ring is never cut off)
        let (x, y, w) = (x + 2.0, y + 2.0, w - 4.0);
        let (tw, th, gap) = (44.0, 30.0, 6.0);
        let mut cx = x;
        let mut cy = y;
        for it in b["items"].as_array().cloned().unwrap_or_default() {
            if cx + tw > x + w && cx > x {
                cx = x;
                cy += th + gap;
            }
            let r = Rect::new(cx, cy, tw, th);
            if self.draw {
                if it["on"] == true {
                    self.p.round(Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), 9.0, accent());
                    self.p.round(r, 7.0, FACE);
                }
                let inner = if it["on"] == true { Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0) } else { r };
                if !self.p.picture(s(&it, "image"), inner, 6.0) {
                    self.p.round(inner, 6.0, FAINT);
                }
                if hovered(r) && it["on"] != true {
                    self.p.round(r, 7.0, crate::paint::Color(1.0, 1.0, 1.0, 0.12));
                }
            }
            self.hit_tip(r, Press::Action(s(&it, "action").to_string(), it["args"].clone()), it["tip"].clone());
            self.last_hit(false, it["menu"].clone());
            cx += tw + gap;
        }
        let add = &b["add"];
        if add.is_object() {
            if cx + 30.0 > x + w && cx > x {
                cx = x;
                cy += th + gap;
            }
            let r = Rect::new(cx, cy, 30.0, 30.0);
            if self.draw {
                if hovered(r) {
                    self.p.circle(r.x + 15.0, r.y + 15.0, 15.0, LIT);
                }
                self.icon(if s(add, "icon").is_empty() { "plus" } else { s(add, "icon") }, r.x + 7.0, r.y + 7.0, 16.0, DIM);
            }
            self.hit_tip(r, Press::Action(s(add, "action").to_string(), add["args"].clone()), add["tip"].clone());
        }
        cy + th - y + 4.0
    }

    /// The desktops on their map: each a card at its place (its column and row), no words on it (its name in its tip), the one shown
    /// ringed in the theme's colour, one where something waits ringed in the second colour; moved about by dragging where no card
    /// is, nearer or further by the wheel. A card held a moment is carried (dropped: it goes there, or swaps with the one there).
    /// Out of view: the desktop shown is a glow on the edge it lies past (pressed: the map goes back to it), one where something
    /// waits a pin there (pressed: the map goes to it). Under the mouse, a free place beside a card offers a new desktop there.
    /// Everything moves: cards glide to their places, the ring slides to the desktop shown, a new card grows in, the map glides
    /// where it is sent. Its dots are the monitors' pixels: one for every so many, so the map feels as fine as the screen is.
    fn deskmap(&mut self, b: &Value, x: f32, y: f32, w: f32) -> f32 {
        let items = b["items"].as_array().cloned().unwrap_or_default();
        let screens = b["screens"].as_array().cloned().unwrap_or_default();
        let aspect = (b["aspect"].as_f64().unwrap_or(16.0 / 9.0) as f32).clamp(1.0, 4.0);
        let res_w = b["res"][0].as_f64().unwrap_or(1920.0) as f32;
        let mh = (DESK_CW / aspect).clamp(48.0, 110.0);
        let cell = (DESK_CW + DESK_GAP, mh + DESK_GAP);
        let pos = |it: &Value| (it["pos"][0].as_i64().unwrap_or(0) as i32, it["pos"][1].as_i64().unwrap_or(0) as i32);
        let (mut c0, mut r0, mut c1, mut r1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for it in &items {
            let (c, r) = pos(it);
            c0 = c0.min(c);
            r0 = r0.min(r);
            c1 = c1.max(c);
            r1 = r1.max(r);
        }
        if items.is_empty() {
            (c0, r0, c1, r1) = (0, 0, 0, 0);
        }
        let (cols, rows) = ((c1 - c0 + 1) as f32, (r1 - r0 + 1) as f32);
        let h = (rows * cell.1 + 24.0).clamp(220.0, 380.0);
        let rect = Rect::new(x, y, w, h);
        let place = |c: i32, r: i32| Rect::new((c - c0) as f32 * cell.0 + DESK_GAP / 2.0, (r - r0) as f32 * cell.1 + DESK_GAP / 2.0, DESK_CW, mh);
        let at = b["at"].as_u64().unwrap_or(0) as usize;
        let cur = items.get(at).map(&pos).unwrap_or((c0, r0));
        let centre = |c: i32, r: i32| {
            let pr = place(c, r);
            (pr.x + pr.w / 2.0, pr.y + pr.h / 2.0)
        };
        // (time since last drawn: what moves, moves by it)
        let now = std::time::Instant::now();
        let mut view = DESK_VIEW.with(|v| v.get());
        let fresh = view.drawn.is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(1500));
        let dt = view.drawn.map_or(0.0, |t| t.elapsed().as_secs_f32().min(0.05));
        let k = 1.0 - (-dt * 12.0).exp();
        let fit = ((w - 24.0) / (cols * cell.0)).min((h - 24.0) / (rows * cell.1)).min(1.0).max(0.35);
        if fresh {
            let (ox, oy) = if fit >= 0.999 { ((cols * cell.0) / 2.0, (rows * cell.1) / 2.0) } else { centre(cur.0, cur.1) };
            view = DeskView { scale: fit.max(0.6).min(1.0), ox, oy, drawn: None };
            DESK_TO.with(|t| t.set(None));
            DESK_ANIM.with(|a| a.borrow_mut().clear());
            DESK_RING.with(|r| r.set(None));
        }
        if let Some((c, r)) = DESK_GO.with(|g| g.take()) {
            DESK_TO.with(|t| t.set(Some(centre(c, r))));
            view.scale = view.scale.max(fit);
        }
        // (sent somewhere: it glides there)
        if let Some((tx, ty)) = DESK_TO.with(|t| t.get()) {
            if fresh {
                view.ox = tx;
                view.oy = ty;
            } else {
                view.ox += (tx - view.ox) * k;
                view.oy += (ty - view.oy) * k;
            }
            if (tx - view.ox).abs() + (ty - view.oy).abs() < 0.5 {
                DESK_TO.with(|t| t.set(None));
            }
        }
        // (never further past the map's ends than a little: a map smaller than the panel sits in its middle)
        let (cw, chh) = (cols * cell.0, rows * cell.1);
        let m = 24.0 / view.scale;
        let (hw, hh) = (w / 2.0 / view.scale, h / 2.0 / view.scale);
        view.ox = if cw + 2.0 * m <= 2.0 * hw { cw / 2.0 } else { view.ox.clamp(hw - m, cw - hw + m) };
        view.oy = if chh + 2.0 * m <= 2.0 * hh { chh / 2.0 } else { view.oy.clamp(hh - m, chh - hh + m) };
        if self.draw {
            view.drawn = Some(now);
        }
        DESK_VIEW.with(|v| v.set(view));
        DESK_GEOM.with(|g| g.set(Some(DeskGeom { rect, cell, min: (c0, r0), view: (view.scale, view.ox, view.oy) })));
        let to_panel = |r: Rect| Rect::new(rect.x + rect.w / 2.0 + (r.x - view.ox) * view.scale, rect.y + rect.h / 2.0 + (r.y - view.oy) * view.scale, r.w * view.scale, r.h * view.scale);
        let carry = DESK_CARRY.with(|c| c.borrow().clone());
        let a2 = crate::draw::accent2();
        // (each card where it is now: gliding to its place; a new one growing in)
        let mut glide: Vec<(Rect, f32)> = Vec::new();
        DESK_ANIM.with(|an| {
            let mut an = an.borrow_mut();
            for it in &items {
                let (c, r) = pos(it);
                let target = place(c, r);
                let id = s(it, "id").to_string();
                let e = an.entry(id).or_insert((target.x, target.y, if fresh { 1.0 } else { 0.0 }));
                if self.draw {
                    e.0 += (target.x - e.0) * k;
                    e.1 += (target.y - e.1) * k;
                    e.2 = (e.2 + dt * 5.0).min(1.0);
                }
                glide.push((Rect::new(e.0, e.1, target.w, target.h), e.2));
            }
        });
        if self.draw {
            self.p.round(rect, 14.0, crate::paint::Color(0.043, 0.043, 0.055, 1.0));
            unsafe {
                self.p.dc.PushAxisAlignedClip(&self.p.px(rect).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
            // (the dots: one for every 320 of the monitors' pixels, as fine as the screens are; plainer while carrying)
            let step = (DESK_CW * 320.0 / res_w.max(320.0) * view.scale).max(6.0);
            let dot = if carry.is_some() { crate::paint::Color(0.2, 0.2, 0.235, 1.0) } else { crate::paint::Color(0.12, 0.12, 0.14, 1.0) };
            let origin = to_panel(Rect::new(DESK_GAP / 2.0, DESK_GAP / 2.0, 0.0, 0.0));
            let sx = origin.x - ((origin.x - rect.x) / step).floor() * step;
            let sy = origin.y - ((origin.y - rect.y) / step).floor() * step;
            let mut yy = sy;
            while yy < rect.bottom() {
                let mut xx = sx;
                while xx < rect.right() {
                    self.p.round(Rect::new(xx - 0.75, yy - 0.75, 1.5, 1.5), 0.75, dot);
                    xx += step;
                }
                yy += step;
            }
        }
        let mut hits_cards: Vec<Hit> = Vec::new();
        let mut taken: Vec<(i32, i32)> = Vec::new();
        // (the ring round the desktop shown: sliding to it; drawn over the cards)
        let mut ring_at: Option<Rect> = None;
        if let Some(cur_i) = items.get(at).map(|_| at) {
            let target = glide.get(cur_i).map(|g| g.0).unwrap_or(place(cur.0, cur.1));
            let ring = DESK_RING.with(|r| {
                let mut v = r.get().unwrap_or(target);
                if self.draw {
                    v = Rect::new(v.x + (target.x - v.x) * k, v.y + (target.y - v.y) * k, target.w, target.h);
                    r.set(Some(v));
                }
                v
            });
            let carried_cur = carry.as_ref().is_some_and(|(cid, ..)| items.get(cur_i).is_some_and(|it| s(it, "id") == cid));
            if !carried_cur {
                ring_at = Some(to_panel(ring));
            }
        }
        for (i, it) in items.iter().enumerate() {
            let (c, r) = pos(it);
            taken.push((c, r));
            let id = s(it, "id").to_string();
            let carried = carry.as_ref().is_some_and(|(cid, ..)| *cid == id);
            if carried {
                continue;
            }
            let (g, grow) = glide.get(i).copied().unwrap_or((place(c, r), 1.0));
            let mr = to_panel(g);
            // (a new one: growing from a little smaller)
            let gk = 0.9 + 0.1 * grow;
            let mr = Rect::new(mr.x + mr.w * (1.0 - gk) / 2.0, mr.y + mr.h * (1.0 - gk) / 2.0, mr.w * gk, mr.h * gk);
            let visible = mr.right() > rect.x && mr.x < rect.right() && mr.bottom() > rect.y && mr.y < rect.bottom();
            if !visible {
                continue;
            }
            let on = it["on"] == true;
            let waits = it["waits"] == true;
            let over = hovered(mr);
            if self.draw {
                crate::draw::faded(self.p.dc, grow.min(0.999), || {
                    if waits && !on {
                        let pulse = 0.55 + 0.45 * (self.p.t * 3.0).sin().abs();
                        self.p.round(Rect::new(mr.x - 6.0, mr.y - 6.0, mr.w + 12.0, mr.h + 12.0), 16.0, a2.alpha(0.12 * pulse));
                        self.p.round(Rect::new(mr.x - 2.0, mr.y - 2.0, mr.w + 4.0, mr.h + 4.0), 12.0, a2);
                    }
                    crate::peek::draw_map(self.p, it, &screens, mr, 10.0 * view.scale.min(1.0), on, true);
                    // (the desktop shown: its own border, bright)
                    if on {
                        self.p.outline(Rect::new(mr.x + 1.0, mr.y + 1.0, mr.w - 2.0, mr.h - 2.0), 9.0 * view.scale.min(1.0), 2.0, accent());
                    }
                    if over && !on {
                        self.p.round(mr, 10.0, crate::paint::Color(1.0, 1.0, 1.0, 0.06));
                    }
                    // (what each window has: a count, its sound)
                    for wv in it["wins"].as_array().into_iter().flatten() {
                        let f = |k: usize| wv["r"][k].as_f64().unwrap_or(0.0) as f32;
                        let wr = Rect::new(mr.x + f(0) * mr.w, mr.y + f(1) * mr.h, f(2) * mr.w, f(3) * mr.h);
                        if wv["wait"] == true && !on {
                            self.p.round(Rect::new(wr.x - 1.0, wr.y - 1.0, wr.w + 2.0, wr.h + 2.0), 4.0, a2.alpha(0.85));
                            self.p.round(wr, 3.0, crate::paint::Color(0.165, 0.165, 0.196, 1.0));
                        }
                        if let Some(n) = wv["count"].as_u64().filter(|_| !on) {
                            let t = n.to_string();
                            let tw = self.p.measure(&t, 9.0, Weight::Semibold, 30.0).max(6.0);
                            let br = Rect::new(wr.right() - tw / 2.0 - 7.0, wr.y - 5.0, tw + 8.0, 13.0);
                            self.p.round(br, 6.5, a2);
                            self.p.text(&t, br.x + 4.0, br.y + 6.5, 9.0, Weight::Semibold, 30.0, crate::paint::Color(0.16, 0.11, 0.02, 1.0));
                        }
                        if wv["sound"] == true && wr.w > 14.0 {
                            let (parts, lift) = loud_bars(0.4, self.p.t);
                            self.p.bars(wr.x + 4.0, wr.bottom() - 6.0, 7.0, parts, lift, crate::paint::Color(0.35, 0.78, 0.9, 1.0));
                        }
                    }
                });
            }
            // (its x, under the mouse: closed)
            let close = s(it, "close");
            if over && !close.is_empty() && carry.is_none() {
                let xr = Rect::new(mr.right() - 24.0, mr.y + 4.0, 20.0, 20.0);
                if self.draw {
                    self.p.circle(xr.x + 10.0, xr.y + 10.0, 10.0, crate::paint::Color(0.0, 0.0, 0.0, 0.66));
                }
                self.icon("x", xr.x + 5.0, xr.y + 5.0, 10.0, TEXT);
                hits_cards.push(Hit { rect: xr, press: Press::Card(close.to_string(), String::new()), tip: Value::String("Close Desktop".into()), hold: false, menu: Value::Null });
            }
            for wv in it["wins"].as_array().cloned().unwrap_or_default().iter().rev() {
                let f = |k: usize| wv["r"][k].as_f64().unwrap_or(0.0) as f32;
                let wr = Rect::new(mr.x + f(0) * mr.w, mr.y + f(1) * mr.h, f(2) * mr.w, f(3) * mr.h);
                if wr.w > 8.0 && wr.h > 8.0 {
                    hits_cards.push(Hit { rect: wr, press: Press::Card(s(wv, "action").to_string(), id.clone()), tip: Value::Null, hold: false, menu: Value::Null });
                }
            }
            // (its name: in its tip)
            hits_cards.push(Hit { rect: mr, press: Press::Card(s(it, "action").to_string(), id.clone()), tip: it["name"].clone(), hold: false, menu: Value::Null });
        }
        let _ = ring_at;
        // (a free place beside a card, under the mouse: a new desktop there)
        if carry.is_none() {
            let hov = HOVER.with(|h| h.get());
            if let Some((hc, hr)) = hov.and_then(|(hx, hy)| desk_cell_at(hx, hy)) {
                let free = !taken.contains(&(hc, hr));
                let beside = taken.iter().any(|(c, r)| (c - hc).abs() + (r - hr).abs() == 1);
                if free && beside {
                    let gr = to_panel(place(hc, hr));
                    if self.draw && gr.right() > rect.x && gr.x < rect.right() {
                        self.p.dashed(gr, 10.0, 1.5, crate::paint::Color(0.204, 0.204, 0.227, 1.0));
                        self.icon("plus", gr.x + gr.w / 2.0 - 9.0, gr.y + gr.h / 2.0 - 9.0, 18.0, DIM);
                    }
                    hits_cards.insert(0, Hit { rect: gr, press: Press::Card(format!("desknew:{hc}:{hr}"), String::new()), tip: Value::String("New Desktop".into()), hold: false, menu: Value::Null });
                }
            }
        }
        // (the card carried: where it would land, dashed; it, under the mouse, lifted)
        if let Some((cid, mx, my, gx, gy)) = carry.as_ref() {
            if let Some(it) = items.iter().find(|it| s(it, "id") == cid) {
                if let Some((lc, lr)) = desk_cell_at(*mx, *my) {
                    let lr_ = to_panel(place(lc, lr));
                    if self.draw {
                        self.p.round(lr_, 10.0, accent().alpha(0.08));
                        self.p.dashed(lr_, 10.0, 1.5, accent());
                    }
                }
                let (cw, ch) = (DESK_CW * view.scale * 1.04, mh * view.scale * 1.04);
                let cr = Rect::new(mx - gx * 1.04, my - gy * 1.04, cw, ch);
                if self.draw {
                    self.p.round(Rect::new(cr.x + 3.0, cr.y + 8.0, cr.w, cr.h), 12.0, crate::paint::Color(0.0, 0.0, 0.0, 0.45));
                    crate::peek::draw_map(self.p, it, &screens, cr, 10.0, it["on"] == true, true);
                    if it["on"] == true {
                        self.p.outline(Rect::new(cr.x + 1.0, cr.y + 1.0, cr.w - 2.0, cr.h - 2.0), 9.0, 2.0, accent());
                    }
                }
            }
        }
        // (out of view: where the desktop shown is, a glow on the edge it lies past; where something waits, a pin)
        let inner = Rect::new(rect.x + 4.0, rect.y + 4.0, rect.w - 8.0, rect.h - 8.0);
        let edge_point = |tx: f32, ty: f32| -> (f32, f32, f32) {
            let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            let (dx, dy) = (tx - cx, ty - cy);
            let k = ((inner.w / 2.0) / dx.abs().max(0.001)).min((inner.h / 2.0) / dy.abs().max(0.001));
            (cx + dx * k, cy + dy * k, dy.atan2(dx))
        };
        if let Some(it) = items.get(at).filter(|_| carry.is_none()) {
            let (c, r) = pos(it);
            let mr = to_panel(place(c, r));
            let out = mr.right() < rect.x || mr.x > rect.right() || mr.bottom() < rect.y || mr.y > rect.bottom();
            if out {
                let (ex, ey, _) = edge_point(mr.x + mr.w / 2.0, mr.y + mr.h / 2.0);
                let far = ((mr.x + mr.w / 2.0 - ex).hypot(mr.y + mr.h / 2.0 - ey) / 400.0).clamp(0.0, 1.0);
                let len = 70.0 + 90.0 * far;
                let near_x = (ex - rect.x).min(rect.right() - ex) < 30.0;
                let near_y = (ey - rect.y).min(rect.bottom() - ey) < 30.0;
                let band = Rect::new(ex - len / 2.0, ey - len / 2.0, len, len);
                let lit = hovered(band);
                if self.draw {
                    // (a soft light on the edge it lies past, centred on its way, and a thin brighter line on the edge itself; off a
                    // corner, round the corner; under the mouse, brighter)
                    let c = accent();
                    let kk = if lit { 1.5 } else { 1.0 };
                    let (lx, ly) = (if near_x { if ex - rect.x < rect.right() - ex { rect.x } else { rect.right() } } else { ex }, if near_y { if ey - rect.y < rect.bottom() - ey { rect.y } else { rect.bottom() } } else { ey });
                    let (rx, ry) = if near_x && near_y { (len * 0.55, len * 0.55) } else if near_x { (28.0, len / 2.0) } else { (len / 2.0, 28.0) };
                    self.p.radial(lx, ly, rx, ry, c.alpha(0.32 * kk));
                    let (tx, ty) = if near_x && near_y { (len * 0.4, len * 0.4) } else if near_x { (3.0, len * 0.32) } else { (len * 0.32, 3.0) };
                    self.p.radial(lx, ly, tx, ty, c.alpha(0.9 * kk.min(1.1)));
                }
                hits_cards.insert(0, Hit { rect: band, press: Press::Card(format!("deskgo:{c}:{r}"), String::new()), tip: it["name"].clone(), hold: false, menu: Value::Null });
            }
        }
        for it in &items {
            if it["waits"] != true || it["on"] == true {
                continue;
            }
            let (c, r) = pos(it);
            let mr = to_panel(place(c, r));
            let out = mr.right() < rect.x || mr.x > rect.right() || mr.bottom() < rect.y || mr.y > rect.bottom();
            if !out {
                continue;
            }
            let (ex, ey, ang) = edge_point(mr.x + mr.w / 2.0, mr.y + mr.h / 2.0);
            let (px, py) = (ex - ang.cos() * 22.0, ey - ang.sin() * 22.0);
            let (px, py) = (px.clamp(rect.x + 18.0, rect.right() - 18.0), py.clamp(rect.y + 18.0, rect.bottom() - 18.0));
            if self.draw {
                let rr = 13.0;
                let phi = (rr / 20.0f32).acos();
                let tip = (px + ang.cos() * 20.0, py + ang.sin() * 20.0);
                let p1 = (px + (ang + phi).cos() * rr, py + (ang + phi).sin() * rr);
                let p2 = (px + (ang - phi).cos() * rr, py + (ang - phi).sin() * rr);
                let d = format!("M {} {} L {} {} A {rr} {rr} 0 1 1 {} {} Z", tip.0, tip.1, p1.0, p1.1, p2.0, p2.1);
                self.p.path(&d, crate::paint::Color(0.106, 0.106, 0.125, 1.0), Some((a2, 1.6)));
                let img = s(it, "pin");
                if img.is_empty() || !self.p.picture(img, Rect::new(px - 7.0, py - 7.0, 14.0, 14.0), 3.5) {
                    self.p.circle(px, py, 3.0, a2);
                }
            }
            hits_cards.insert(0, Hit { rect: Rect::new(px - 14.0, py - 14.0, 28.0, 28.0), press: Press::Card(format!("deskgo:{c}:{r}"), String::new()), tip: it["name"].clone(), hold: false, menu: Value::Null });
        }
        if self.draw {
            unsafe {
                self.p.dc.PopAxisAlignedClip();
            }
            self.hits.extend(hits_cards);
        }
        self.hit(rect, Press::View(DESKMAP.to_string()));
        h
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

    /// A chevron pointing left or right, centred on (cx, cy).
    fn chev(&self, cx: f32, cy: f32, left: bool, c: crate::paint::Color) {
        if self.draw {
            let d = if left { 1.0 } else { -1.0 };
            self.p.line(&[(cx + 2.5 * d, cy - 5.0), (cx - 2.5 * d, cy), (cx + 2.5 * d, cy + 5.0)], 1.8, c);
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

    /// A block's colour: its own (`color`), else its picture's (`color_from`, a cover's), else `or`.
    fn tint(&self, b: &Value, or: crate::paint::Color) -> crate::paint::Color {
        if let Some((r, g, bl)) = b["color_from"].as_str().and_then(|i| crate::images::tone(self.p.pc, i)) {
            return crate::paint::Color(r, g, bl, 1.0);
        }
        color_of(&b["color"], or)
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
                "seg" => {
                    let ts = it["items"].as_array().cloned().unwrap_or_default();
                    ts.iter().map(|t| c.p.measure(s(t, "label"), 11.0, Weight::Semibold, 80.0) + 18.0).sum::<f32>() + 2.0 * (ts.len().max(1) - 1) as f32 + 4.0
                }
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
                "seg" => {
                    // (a choice of a few words, on its track, the chosen one filled)
                    let ts = it["items"].as_array().cloned().unwrap_or_default();
                    self.round(Rect::new(cx, cy - 14.0, iw, 28.0), 10.0, TRACK);
                    let mut tx = cx + 2.0;
                    for t in &ts {
                        let lw = self.p.measure(s(t, "label"), 11.0, Weight::Semibold, 80.0) + 18.0;
                        let r = Rect::new(tx, cy - 12.0, lw, 24.0);
                        let on = t["on"] == true;
                        if on {
                            self.round(r, 8.0, CHOSEN);
                        } else if self.draw && hovered(r) {
                            self.round(r, 8.0, LIT);
                        }
                        self.text(s(t, "label"), tx + 9.0, cy, 11.0, Weight::Semibold, 80.0, if on { TEXT } else { DIM });
                        if !s(t, "action").is_empty() {
                            self.hit(r, Press::Action(s(t, "action").to_string(), t["args"].clone()));
                        }
                        tx += lw + 2.0;
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
            // (the players playing: a chip each, its cover, its app, its bars (or a pause mark); the one shown filled. The chip:
            // shown; its bars: played or paused)
            "players" => {
                let mut cx = x;
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let name = s(&it, "name");
                    let nw = self.p.measure(name, 11.0, Weight::Semibold, 120.0);
                    let cw = 4.0 + 20.0 + 6.0 + nw + 6.0 + 16.0 + 8.0;
                    let r = Rect::new(cx, y, cw, 28.0);
                    let on = it["on"] == true;
                    self.round(r, 14.0, if on { CHOSEN } else if hovered(r) { LIT } else { TRACK });
                    if self.draw && !self.p.picture(s(&it, "image"), Rect::new(cx + 4.0, y + 4.0, 20.0, 20.0), 5.0) {
                        self.p.round(Rect::new(cx + 4.0, y + 4.0, 20.0, 20.0), 5.0, GROUND);
                    }
                    self.text(name, cx + 30.0, y + 14.0, 11.0, Weight::Semibold, 120.0, if on { TEXT } else { DIM });
                    let mx = cx + 30.0 + nw + 6.0;
                    if self.draw {
                        let c = color_of(&it["color"], accent());
                        if it["playing"] == true {
                            self.p.viz(mx, y + 14.0, 10.0, self.p.viz, if on { c } else { DIM });
                        } else {
                            self.p.round(Rect::new(mx + 2.0, y + 9.5, 3.0, 9.0), 1.0, DIM);
                            self.p.round(Rect::new(mx + 7.5, y + 9.5, 3.0, 9.0), 1.0, DIM);
                        }
                    }
                    // (its bars, pressed: played or paused, not shown)
                    if !s(&it, "toggle").is_empty() {
                        self.hit_tip(Rect::new(mx - 3.0, y, 20.0, 28.0), Press::Action(s(&it, "toggle").to_string(), it["args"].clone()), json!(if it["playing"] == true { "Pause" } else { "Play" }));
                    }
                    self.hit(r, Press::Action(s(&it, "action").to_string(), it["args"].clone()));
                    cx += cw + 6.0;
                }
                28.0
            }
            // (a cover, large, in the middle, glowing softly in its colours)
            "cover" => {
                let z = b["size"].as_f64().map_or(w.min(200.0), |v| (v as f32).min(w));
                let r = Rect::new(x + w / 2.0 - z / 2.0, y, z, z);
                if self.draw {
                    if let Some((cr, cg, cb)) = crate::images::tone(self.p.pc, s(b, "image")) {
                        let c = crate::paint::Color(cr, cg, cb, 1.0);
                        for i in 0..8 {
                            let e = 2.0 + i as f32 * 3.0;
                            self.p.round(Rect::new(r.x - e, r.y - e + 6.0, r.w + 2.0 * e, r.h + 2.0 * e), 16.0 + e, c.alpha(0.05));
                        }
                    }
                    if !self.p.picture(s(b, "image"), r, 16.0) {
                        self.p.round(r, 16.0, GROUND);
                        self.icon("music", r.x + z / 2.0 - 16.0, r.y + z / 2.0 - 16.0, 32.0, DIM);
                    }
                }
                if !s(b, "press").is_empty() {
                    self.hit(r, Press::Action(s(b, "press").to_string(), Value::Null));
                }
                z
            }
            // (a song's head: its name, big, who sings it and its album under it; its bars at its end)
            "songhead" => {
                let bw = if b["viz"].is_boolean() { 22.0 } else { 0.0 };
                self.text(s(b, "title"), x, y + 11.0, 16.0, Weight::Semibold, w - bw, TEXT);
                self.text(s(b, "sub"), x, y + 31.0, 12.5, Weight::Regular, w - bw, DIM);
                if self.draw && b["viz"].is_boolean() {
                    let parts = if b["viz"] == true { self.p.viz } else { FLAT };
                    self.p.viz(x + w - 16.0, y + 20.0, 14.0, parts, self.tint(b, accent()));
                }
                42.0
            }
            // (a graph of a while: its name, its number now in its colour, a word at its end; then the graph)
            "chart" => {
                let c = color_of(&b["color"], TEXT);
                let nw = self.text(s(b, "label"), x, y + 12.0, 10.5, Weight::Semibold, 40.0, DIM).max(30.0);
                self.text(s(b, "text"), x + nw + 6.0, y + 12.0, 20.0, Weight::Semibold, 100.0, c);
                self.text_end(s(b, "sub"), x + w, y + 12.0, 11.0, Weight::Regular, w * 0.5, DIM);
                let gh = b["height"].as_f64().unwrap_or(40.0) as f32;
                if self.draw {
                    let vals: Vec<f32> = b["values"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64().map(|v| v as f32)).collect()).unwrap_or_default();
                    self.p.spark(&vals, Rect::new(x, y + 26.0, w, gh), c);
                }
                26.0 + gh
            }
            // (programs and how much each uses: its picture, its name, a bar, a number)
            "usage" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                for (n, r) in rows.iter().enumerate() {
                    let cy = y + n as f32 * 28.0 + 13.0;
                    let pic = Rect::new(x, cy - 11.0, 22.0, 22.0);
                    if self.draw && !self.p.picture(s(r, "image"), pic, 5.0) {
                        self.p.round(pic, 6.0, GROUND);
                    }
                    let bw = 90.0;
                    let tx = x + 30.0;
                    self.text(s(r, "name"), tx, cy, 12.5, Weight::Regular, (w - 30.0 - bw - 50.0).max(20.0), TEXT);
                    let bx = x + w - bw - 44.0;
                    self.round(Rect::new(bx, cy - 3.0, bw, 6.0), 3.0, TRACK);
                    let v = f(r, "value").clamp(0.0, 1.0);
                    if v > 0.0 {
                        self.round(Rect::new(bx, cy - 3.0, (bw * v).max(6.0), 6.0), 3.0, color_of(&r["color"], accent()));
                    }
                    self.text_end(s(r, "text"), x + w, cy, 12.0, Weight::Semibold, 44.0, TEXT);
                }
                (rows.len() as f32 * 28.0 - 4.0).max(0.0)
            }
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
                // (its label; then, quieter but readable, what more it says)
                let lw = self.text(s(b, "label"), x, y + 7.0, 11.5, Weight::Semibold, w * 0.5, TEXT);
                if !s(b, "sub").is_empty() {
                    self.text(s(b, "sub"), x + lw + 8.0, y + 7.0, 11.0, Weight::Regular, w * 0.5 - lw, DIM);
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
                    let mut rw = if right.is_empty() { 0.0 } else { self.p.measure(right, 10.5, Weight::Regular, 80.0) + 8.0 };
                    if !right.is_empty() {
                        self.text_end(right, x + w, y + h + 9.0, 10.5, Weight::Regular, 80.0, if lit { GREEN } else { DIM });
                    }
                    // (small marks at its right: saved, secured)
                    for m in r["marks"].as_array().cloned().unwrap_or_default().iter().rev() {
                        self.icon(m.as_str().unwrap_or(""), x + w - rw - 13.0, y + h + 3.0, 12.0, DIM);
                        rw += 18.0;
                    }
                    // (under the mouse: a ground of its own)
                    if !action.is_empty() && self.draw && hovered(Rect::new(x - 4.0, y + h, w + 8.0, rh)) {
                        self.p.round(Rect::new(x - 6.0, y + h - 3.0, w + 12.0, rh + 6.0), 9.0, LIT);
                    }
                    let ix = if s(r, "icon").is_empty() && !r["signal"].is_number() { 0.0 } else { 22.0 };
                    if let Some(level) = r["signal"].as_u64() {
                        if self.draw {
                            for i in 0..4u64 {
                                let bh = 4.0 + 3.0 * i as f32;
                                self.p.round(Rect::new(x + i as f32 * 4.0, y + h + 16.0 - bh, 2.5, bh), 1.25, if i < level { TEXT } else { FAINT });
                            }
                        }
                    } else if ix > 0.0 {
                        self.icon(s(r, "icon"), x, y + h + 2.0, 14.0, if lit { GREEN } else { DIM });
                    }
                    let tc = if r["dim"] == true { DIM } else { TEXT };
                    self.text(s(r, "title"), x + ix, y + h + 9.0, 12.0, Weight::Semibold, w - rw - ix, tc);
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
            // (cards in two columns, each as tall as it needs, the next in the shorter column: notes on a wall. Each: `color` its ground,
            // `ink` its words ("#rrggbb"), `title`, `text` (wrapped), `items` (a checklist: `state` "done" or not, each pressed: its
            // `action`), `badge` (a picture or a pictogram in its corner: where it is from), `tip` (the badge's), `action` (pressed),
            // `menu` (right-clicked))
            "cards" => {
                let gap = 8.0;
                let n = b["columns"].as_u64().unwrap_or(2).clamp(1, 3) as usize;
                let cw = ((w - gap * (n - 1) as f32) / n as f32).max(40.0);
                let mut cols = vec![0.0f32; n];
                let hex = |v: &str| (v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit())).then(|| crate::paint::Color::hex(v));
                for card in b["cards"].as_array().cloned().unwrap_or_default() {
                    let ink = hex(s(&card, "ink")).unwrap_or(TEXT);
                    let ground = hex(s(&card, "color")).unwrap_or(FAINT);
                    let pad = 10.0;
                    let iw = cw - 2.0 * pad;
                    let title = s(&card, "title");
                    let text = s(&card, "text");
                    let items = card["items"].as_array().cloned().unwrap_or_default();
                    let badge = s(&card, "badge");
                    // (being written: `edit = { title, text, item }` (actions), its title and words typed in where they are)
                    let ed = &card["edit"];
                    let editing = ed.is_object();
                    let (title_size, text_lines) = if editing { (15.0, 24) } else { (12.5, 8) };
                    // (how tall: its title, its words (up to 8 lines; written: room for three at least), its list, room for its badge)
                    let th = if title.is_empty() && !editing { 0.0 } else if editing { 26.0 } else { 19.0 };
                    let xh = if editing {
                        (if text.is_empty() { 0.0 } else { self.p.paragraph(text, 0.0, 0.0, 12.5, iw, text_lines, ink, false) }).max(54.0) + 4.0
                    } else if text.is_empty() {
                        0.0
                    } else {
                        self.p.paragraph(text, 0.0, 0.0, 12.0, iw, text_lines, ink, false) + 2.0
                    };
                    let lh = items.len() as f32 * 20.0 + if editing && !s(ed, "item").is_empty() { 22.0 } else { 0.0 };
                    let bh = if badge.is_empty() { 0.0 } else { 14.0 };
                    let ch = (pad * 2.0 + th + xh + lh + bh).max(44.0);
                    let col = (0..n).fold(0, |best, i| if cols[i] < cols[best] { i } else { best });
                    let r = Rect::new(x + col as f32 * (cw + gap), y + cols[col], cw, ch);
                    cols[col] += ch + gap;
                    if self.draw {
                        self.p.round(r, 12.0, ground);
                        if hovered(r) {
                            self.p.round(r, 12.0, crate::paint::Color(1.0, 1.0, 1.0, 0.05));
                        }
                    }
                    let mut cy = r.y + pad;
                    let style = |size: f32, bold: bool, area: bool| json!({ "ground": s(&card, "color"), "ink": s(&card, "ink"), "size": size, "bold": bold, "area": area });
                    if editing {
                        // (its title, else its hint, faint; pressed: typed in, in place)
                        let tr = Rect::new(r.x + pad - 4.0, cy, iw + 8.0, th - 4.0);
                        if title.is_empty() {
                            self.text(if s(ed, "title_hint").is_empty() { "Title" } else { s(ed, "title_hint") }, r.x + pad, cy + 11.0, title_size, Weight::Semibold, iw, ink.alpha(0.4));
                        } else {
                            self.text(title, r.x + pad, cy + 11.0, title_size, Weight::Semibold, iw, ink);
                        }
                        if self.draw && hovered(tr) {
                            self.p.round(tr, 6.0, crate::paint::Color(1.0, 1.0, 1.0, 0.05));
                        }
                        if !s(ed, "title").is_empty() {
                            self.hit(tr, Press::Field(s(ed, "title").to_string(), false, "Title".into(), title.to_string(), false, style(title_size, true, false)));
                        }
                        cy += th;
                        let xr = Rect::new(r.x + pad - 4.0, cy - 2.0, iw + 8.0, xh);
                        if text.is_empty() {
                            self.text(if s(ed, "text_hint").is_empty() { "Write something" } else { s(ed, "text_hint") }, r.x + pad, cy + 9.0, 12.5, Weight::Regular, iw, ink.alpha(0.4));
                        } else if self.draw {
                            self.p.paragraph(text, r.x + pad, cy, 12.5, iw, text_lines, ink, true);
                        }
                        if self.draw && hovered(xr) {
                            self.p.round(xr, 6.0, crate::paint::Color(1.0, 1.0, 1.0, 0.05));
                        }
                        if !s(ed, "text").is_empty() {
                            self.hit(xr, Press::Field(s(ed, "text").to_string(), false, "Write something".into(), text.to_string(), false, style(12.5, false, true)));
                        }
                        cy += xh;
                    } else {
                        if !title.is_empty() {
                            self.text(title, r.x + pad, cy + 8.0, 12.5, Weight::Semibold, iw, ink);
                            cy += th;
                        }
                        if !text.is_empty() {
                            if self.draw {
                                self.p.paragraph(text, r.x + pad, cy, 12.0, iw, text_lines, ink, true);
                            }
                            cy += xh;
                        }
                    }
                    for it in &items {
                        let done = s(it, "state") == "done";
                        let ic = cy + 10.0;
                        let c = if done { ink.alpha(0.5) } else { ink };
                        if self.draw {
                            let bx = Rect::new(r.x + pad, ic - 5.5, 11.0, 11.0);
                            if done {
                                self.p.round(bx, 3.0, c);
                            } else {
                                self.p.round(bx, 3.0, c.alpha(0.25));
                                self.p.round(Rect::new(bx.x + 1.5, bx.y + 1.5, 8.0, 8.0), 2.0, ground);
                            }
                        }
                        self.text(s(it, "text"), r.x + pad + 18.0, ic, 12.0, Weight::Regular, iw - 18.0, c);
                        if !s(it, "action").is_empty() {
                            self.hit(Rect::new(r.x + 4.0, cy, r.w - 8.0, 20.0), Press::Action(s(it, "action").to_string(), it["args"].clone()));
                        }
                        cy += 20.0;
                    }
                    // (being written: a line to add an item to its list)
                    if editing && !s(ed, "item").is_empty() {
                        let ar = Rect::new(r.x + 4.0, cy, r.w - 8.0, 20.0);
                        if self.draw {
                            self.icon("plus", r.x + pad, cy + 4.0, 12.0, ink.alpha(0.5));
                            if hovered(ar) {
                                self.p.round(ar, 6.0, crate::paint::Color(1.0, 1.0, 1.0, 0.05));
                            }
                        }
                        self.text("Add an item", r.x + pad + 18.0, cy + 10.0, 12.0, Weight::Regular, iw - 18.0, ink.alpha(0.45));
                        self.hit(ar, Press::Field(s(ed, "item").to_string(), false, "Add an item".into(), String::new(), false, style(12.0, false, false)));
                    }
                    if !badge.is_empty() {
                        let br = Rect::new(r.right() - pad - 14.0, r.bottom() - pad - 14.0, 14.0, 14.0);
                        if self.draw && (!badge.contains(':') || !self.p.picture(badge, br, 3.5)) {
                            self.icon(if badge.contains(':') { "dot" } else { badge }, br.x, br.y, 14.0, ink.alpha(0.7));
                        }
                        if !card["tip"].is_null() {
                            self.hit_tip(br, Press::Action(s(&card, "action").to_string(), card["args"].clone()), card["tip"].clone());
                        }
                    }
                    if !s(&card, "action").is_empty() {
                        self.hit(r, Press::Action(s(&card, "action").to_string(), card["args"].clone()));
                        self.last_hit(false, card["menu"].clone());
                    }
                }
                cols.iter().copied().fold(0.0f32, f32::max) - if b["cards"].as_array().is_some_and(|c| !c.is_empty()) { gap } else { 0.0 }
            }
            // (colours to pick from: a dot each (`color`), the one picked ringed; each pressed: its `action` with its `args`)
            "colors" => {
                let z = 18.0;
                let mut cx = x + 3.0;
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let c = s(&it, "color");
                    let col = if c.len() == 7 && c.starts_with('#') { crate::paint::Color::hex(c) } else { FAINT };
                    let r = Rect::new(cx, y + 5.0, z, z);
                    if self.draw {
                        if it["on"] == true {
                            self.p.round(Rect::new(r.x - 3.0, r.y - 3.0, z + 6.0, z + 6.0), (z + 6.0) / 2.0, TEXT);
                            self.p.round(Rect::new(r.x - 1.5, r.y - 1.5, z + 3.0, z + 3.0), (z + 3.0) / 2.0, FACE);
                        }
                        self.p.round(r, z / 2.0, col);
                    }
                    self.hit_tip(Rect::new(r.x - 3.0, r.y - 3.0, z + 6.0, z + 6.0), Press::Action(s(&it, "action").to_string(), it["args"].clone()), it["tip"].clone());
                    cx += z + 12.0;
                }
                z + 10.0
            }
            "checklist" => {
                let mut h = 0.0;
                if !s(b, "title").is_empty() {
                    self.text(s(b, "title"), x, y + 7.0, 10.5, Weight::Semibold, w, DIM);
                    h += 18.0;
                }
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let cy = y + h + 10.0;
                    if !s(&it, "action").is_empty() {
                        self.ghost(Rect::new(x - 6.0, cy - 10.0, w + 12.0, 20.0), 8.0);
                    }
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
                    if !s(&it, "action").is_empty() {
                        let r = Rect::new(x - 6.0, cy - 10.0, w + 12.0, 20.0);
                        self.hit(r, Press::Action(s(&it, "action").to_string(), it["args"].clone()));
                    }
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
                let bar = Rect::new(lx + 4.0, cy - 3.0, (rx - lx - 8.0).max(10.0), 6.0);
                if self.draw {
                    // (its own colour, when it gives one (a cover's); a knob where it is)
                    let c = self.tint(b, TEXT);
                    let v = sliding(s(b, "action")).unwrap_or(f(b, "value")).clamp(0.0, 1.0);
                    self.p.round(bar, 3.0, GROUND);
                    if v > 0.0 {
                        self.p.round(Rect::new(bar.x, bar.y, (bar.w * v).max(6.0), bar.h), 3.0, c);
                    }
                    if b["color"].is_string() || b["color_from"].is_string() {
                        self.p.circle(bar.x + bar.w * v, cy, 6.0, TEXT);
                    }
                }
                self.hit(Rect::new(bar.x - SEEK_INSET, cy - 10.0, bar.w + 2.0 * SEEK_INSET, 20.0), Press::Seek(s(b, "action").to_string()));
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
                // (one of them the main one (play): big, filled, in the middle)
                if items.iter().any(|i| i["main"] == true) {
                    let size = |i: &Value| if i["main"] == true { 52.0 } else { 40.0 };
                    let total: f32 = items.iter().map(size).sum::<f32>() + (items.len().saturating_sub(1)) as f32 * 18.0;
                    let mut cx = x + w / 2.0 - total / 2.0;
                    let cy = y + 26.0;
                    for it in &items {
                        let d = size(it);
                        let r = Rect::new(cx, cy - d / 2.0, d, d);
                        if it["main"] == true {
                            if self.draw {
                                self.p.circle(cx + d / 2.0, cy, d / 2.0, if hovered(r) { TEXT.alpha(0.85) } else { TEXT });
                            }
                            self.icon(s(it, "icon"), cx + d / 2.0 - 11.0, cy - 11.0, 22.0, FACE);
                        } else {
                            if self.draw && hovered(r) {
                                self.p.circle(cx + d / 2.0, cy, d / 2.0, LIT);
                            }
                            self.icon(s(it, "icon"), cx + d / 2.0 - 10.0, cy - 10.0, 20.0, TEXT);
                        }
                        self.hit_tip(r, Press::Action(s(it, "action").to_string(), it["args"].clone()), json!(s(it, "label")));
                        cx += d + 18.0;
                    }
                    return 52.0;
                }
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
                    self.hit(r, Press::Field(s(b, "action").to_string(), b["live"] == true, hint.to_string(), s(b, "value").to_string(), b["secret"] == true, Value::Null));
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
            // (a goal: a ring of how far toward it, a pictogram in it (a paler part after it: what counts too); a big number and a line
            // under it; small chips stacked on its right)
            "goal" => {
                let ringed = b["ring"] != false;
                let z = 58.0;
                let (cx, cy) = (x + z / 2.0, y + z / 2.0);
                let c = color_of(&b["color"], accent());
                if self.draw && ringed {
                    let none = crate::paint::Color(0.0, 0.0, 0.0, 0.0);
                    let extra = f(b, "extra");
                    self.p.ring(cx, cy, z / 2.0 - 3.0, 5.0, f(b, "part") + extra, 0.0, if extra > 0.0 { c.alpha(0.4) } else { c }, TRACK);
                    if extra > 0.0 {
                        self.p.ring(cx, cy, z / 2.0 - 3.0, 5.0, f(b, "part"), 0.0, c, none);
                    }
                }
                if ringed && !s(b, "icon").is_empty() {
                    self.icon(s(b, "icon"), cx - 9.0, cy - 9.0, 18.0, c);
                }
                let chips = b["chips"].as_array().cloned().unwrap_or_default();
                let n = chips.len() as f32;
                let mut cw_max: f32 = 0.0;
                let top = cy - (n * 24.0 + (n - 1.0).max(0.0) * 5.0) / 2.0;
                for (i, it) in chips.iter().enumerate() {
                    let (icon, text) = (s(it, "icon"), s(it, "text"));
                    let tc = match s(it, "tone") {
                        "" => DIM,
                        t => tone_color(t),
                    };
                    let cw = self.p.measure(text, 11.5, Weight::Semibold, 120.0) + 18.0 + if icon.is_empty() { 0.0 } else { 17.0 };
                    cw_max = cw_max.max(cw);
                    let r = Rect::new(x + w - cw, top + i as f32 * 29.0, cw, 24.0);
                    self.round(r, 12.0, FAINT);
                    let mut tx = r.x + 9.0;
                    if !icon.is_empty() {
                        self.icon(icon, tx, r.y + 5.0, 14.0, tc);
                        tx += 17.0;
                    }
                    self.text(text, tx, r.y + 12.0, 11.5, Weight::Semibold, 120.0, tc);
                }
                let lead = if ringed { z + 14.0 } else { 0.0 };
                let tx = x + lead;
                let room = (w - lead - cw_max - 8.0).max(20.0);
                self.text(s(b, "text"), tx, cy - 8.0, 26.0, Weight::Semibold, room, TEXT);
                if !s(b, "sub").is_empty() {
                    self.text(s(b, "sub"), tx, cy + 15.0, 12.0, Weight::Regular, room, DIM);
                }
                z
            }
            // (columns, one a day: each split into its parts from the foot up, a dashed line at the goal, one ringed (`mark`), its letter
            // under it; pressed: its action)
            "columns" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let gh = b["height"].as_f64().unwrap_or(96.0) as f32;
                let n = items.len().max(1) as f32;
                // (room on every side for the ring round a column)
                let (x, y, w) = (x + 2.0, y + 2.0, w - 4.0);
                let gap = 7.0;
                let cw = (w - gap * (n - 1.0)) / n;
                let total = |it: &Value| it["parts"].as_array().map_or(0.0, |a| a.iter().map(|p| f(p, "v")).sum::<f32>());
                let goal = f(b, "goal");
                let top = items.iter().map(total).fold(goal, f32::max).max(1e-6) * 1.06;
                for (i, it) in items.iter().enumerate() {
                    let bx = x + i as f32 * (cw + gap);
                    let t = total(it);
                    let bh = (gh * t / top).max(4.0);
                    let bar = Rect::new(bx, y + gh - bh, cw, bh);
                    self.round(bar, 5.0f32.min(cw / 2.0), TRACK);
                    let mut yy = y + gh;
                    for p in it["parts"].as_array().cloned().unwrap_or_default() {
                        let sh = gh * f(&p, "v") / top;
                        if sh < 1.0 {
                            continue;
                        }
                        let r = Rect::new(bx, yy - sh, cw, (sh - 2.0).max(1.0));
                        self.round(r, 3.0f32.min(r.h / 2.0), color_of(&p["color"], accent()));
                        yy -= sh;
                    }
                    let mark = match s(it, "mark") {
                        "theme" => Some(accent()),
                        "second" => Some(accent2()),
                        _ => None,
                    };
                    if let (Some(c), true) = (mark, self.draw) {
                        self.p.outline(Rect::new(bar.x - 2.0, bar.y - 2.0, bar.w + 4.0, bar.h + 4.0), 7.0, 1.5, c);
                    }
                    let l = s(it, "label");
                    let lw = self.p.measure(l, 10.0, Weight::Regular, cw + gap);
                    self.text(l, bx + cw / 2.0 - lw / 2.0, y + gh + 11.0, 10.0, if mark.is_some() { Weight::Semibold } else { Weight::Regular }, cw + gap, if s(it, "mark") == "theme" { TEXT } else { DIM });
                    if !s(it, "action").is_empty() {
                        let tip = if s(it, "tip").is_empty() { Value::Null } else { it["tip"].clone() };
                        self.hit_tip(Rect::new(bx - gap / 2.0, y, cw + gap, gh + 18.0), Press::Action(s(it, "action").to_string(), it["args"].clone()), tip);
                    }
                }
                if goal > 0.0 && self.draw {
                    let gy = y + gh - gh * goal / top;
                    self.p.dashed(Rect::new(x, gy, w, 0.0), 0.0, 1.5, TEXT.alpha(0.28));
                }
                gh + 20.0
            }
            // (one bar split into its parts, and under it a key: a dot of each part's colour and its words)
            "split" => {
                let parts: Vec<Value> = b["parts"].as_array().cloned().unwrap_or_default().into_iter().filter(|p| f(p, "v") > 0.0).collect();
                let bh = b["height"].as_f64().unwrap_or(10.0) as f32;
                let sum: f32 = parts.iter().map(|p| f(p, "v")).sum::<f32>().max(1e-6);
                let avail = w - 2.0 * (parts.len().max(1) - 1) as f32;
                let mut px = x;
                if parts.is_empty() {
                    self.round(Rect::new(x, y, w, bh), bh / 2.0, TRACK);
                }
                for p in &parts {
                    let pw = (avail * f(p, "v") / sum).max(2.0);
                    self.round(Rect::new(px, y, pw, bh), (bh / 2.0).min(pw / 2.0), color_of(&p["color"], accent()));
                    px += pw + 2.0;
                }
                let keyed: Vec<&Value> = parts.iter().filter(|p| !s(p, "label").is_empty()).collect();
                if keyed.is_empty() {
                    return bh;
                }
                let mut kx = x;
                let mut ky = y + bh + 8.0 + 8.0;
                for p in keyed {
                    let l = s(p, "label");
                    let lw = self.p.measure(l, 11.0, Weight::Semibold, w - 14.0) + 14.0;
                    if kx + lw > x + w && kx > x {
                        kx = x;
                        ky += 18.0;
                    }
                    if self.draw {
                        self.p.circle(kx + 4.0, ky, 4.0, color_of(&p["color"], accent()));
                    }
                    self.text(l, kx + 13.0, ky, 11.0, Weight::Semibold, w - 14.0, DIM);
                    kx += lw + 12.0;
                }
                ky + 8.0 - y
            }
            // (a day hour by hour: a bar an hour, as tall as it was busy)
            "hours" => {
                let vals: Vec<f32> = b["values"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                let n = vals.len().max(1) as f32;
                let gh = 22.0;
                let cw = (w - 2.0 * (n - 1.0)) / n;
                let c = color_of(&b["color"], accent());
                for (i, v) in vals.iter().enumerate() {
                    let bx = x + i as f32 * (cw + 2.0);
                    if *v > 0.0 {
                        let h = (gh * v.min(1.0)).max(3.0);
                        self.round(Rect::new(bx, y + gh - h, cw, h), 2.0f32.min(cw / 2.0), c);
                    } else {
                        self.round(Rect::new(bx, y + gh - 2.0, cw, 2.0), 1.0, TRACK);
                    }
                }
                let marks = ["0", "6", "12", "18", "24"];
                for (i, m) in marks.iter().enumerate() {
                    let at = x + w * i as f32 / 4.0;
                    let mw = self.p.measure(m, 10.0, Weight::Regular, 30.0);
                    let mx = if i == 0 { at } else if i == 4 { at - mw } else { at - mw / 2.0 };
                    self.text(m, mx, y + gh + 10.0, 10.0, Weight::Regular, 30.0, SOFT_DIM);
                }
                gh + 17.0
            }
            // (when, over days: a row a day, a square an hour, brighter for more)
            "heat" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                let c = color_of(&b["color"], accent());
                let lab = 14.0;
                for (r, row) in rows.iter().enumerate() {
                    let vals: Vec<f32> = row["values"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                    let n = vals.len().max(1) as f32;
                    let cw = (w - lab - 2.0 * (n - 1.0)) / n;
                    let ry = y + r as f32 * 11.0;
                    self.text(s(row, "label"), x, ry + 4.5, 9.0, Weight::Regular, lab, SOFT_DIM);
                    for (i, v) in vals.iter().enumerate() {
                        let cell = Rect::new(x + lab + i as f32 * (cw + 2.0), ry, cw, 9.0);
                        self.round(cell, 2.0, if *v > 0.0 { c.alpha(0.25 + 0.75 * v.min(1.0)) } else { TRACK });
                    }
                }
                (rows.len() as f32 * 11.0 - 2.0).max(0.0)
            }
            // (a month: a square a day in weeks of seven, brighter for more; a light ring on a day that met its goal, the theme's on
            // today; the days of the week under it; pressed: its action)
            "calendar" => {
                let days = b["days"].as_array().cloned().unwrap_or_default();
                let lead = b["lead"].as_u64().unwrap_or(0) as usize;
                let c = color_of(&b["color"], accent());
                let gap = 5.0;
                // (room on every side for the ring round today and the one under the mouse)
                let (x, y, w) = (x + 2.0, y + 2.0, w - 4.0);
                let cell = (w - gap * 6.0) / 7.0;
                let cells = lead + days.len();
                let rows = cells.div_ceil(7).max(1);
                for (i, d) in days.iter().enumerate() {
                    let k = lead + i;
                    let r = Rect::new(x + (k % 7) as f32 * (cell + gap), y + (k / 7) as f32 * (cell + gap), cell, cell);
                    let v = f(d, "v");
                    let ground = if d["future"] == true {
                        crate::paint::Color(0.059, 0.063, 0.075, 1.0)
                    } else if v > 0.0 {
                        c.alpha(0.18 + 0.82 * v.min(1.0))
                    } else {
                        TRACK
                    };
                    self.round(r, 5.0, ground);
                    if self.draw && d["ring"] == true {
                        self.p.outline(Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0), 4.5, 1.5, TEXT.alpha(0.85));
                    }
                    if self.draw && d["today"] == true {
                        self.p.outline(Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), 7.0, 1.5, accent());
                    }
                    if self.draw && !s(d, "action").is_empty() && hovered(r) {
                        self.p.outline(Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), 7.0, 1.5, DIM);
                    }
                    if !s(d, "action").is_empty() {
                        let tip = if s(d, "tip").is_empty() { Value::Null } else { d["tip"].clone() };
                        self.hit_tip(r, Press::Action(s(d, "action").to_string(), d["args"].clone()), tip);
                    }
                }
                let ly = y + rows as f32 * (cell + gap) - gap + 12.0;
                for (i, l) in b["labels"].as_array().cloned().unwrap_or_default().iter().enumerate().take(7) {
                    let l = l.as_str().unwrap_or("");
                    let lw = self.p.measure(l, 10.0, Weight::Regular, cell);
                    self.text(l, x + i as f32 * (cell + gap) + cell / 2.0 - lw / 2.0, ly, 10.0, Weight::Regular, cell, SOFT_DIM);
                }
                rows as f32 * (cell + gap) - gap + 22.0
            }
            // (things ranked: a pictogram, a name and a bar of its share under it, how much on the right; pressed: its action)
            "ranks" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                let rh = 30.0;
                for (i, r) in rows.iter().enumerate() {
                    let ry = y + i as f32 * (rh + 4.0);
                    let action = s(r, "action");
                    let whole = Rect::new(x - 6.0, ry - 2.0, w + 12.0, rh + 4.0);
                    if !action.is_empty() {
                        self.ghost(whole, 10.0);
                        self.hit(whole, Press::Action(action.to_string(), r["args"].clone()));
                    }
                    let ix = if s(r, "icon").is_empty() { 0.0 } else { 24.0 };
                    if ix > 0.0 {
                        self.icon(s(r, "icon"), x, ry + 7.0, 15.0, DIM);
                    }
                    let vw = 64.0;
                    self.text_end(s(r, "text"), x + w, ry + rh / 2.0, 12.0, Weight::Semibold, vw, DIM);
                    let room = w - ix - vw - 8.0;
                    self.text(s(r, "title"), x + ix, ry + 9.0, 12.0, Weight::Regular, room, TEXT);
                    let br = Rect::new(x + ix, ry + 20.0, room, 5.0);
                    self.round(br, 2.5, TRACK);
                    let part = f(r, "part").clamp(0.0, 1.0);
                    if part > 0.0 {
                        self.round(Rect::new(br.x, br.y, (room * part).max(5.0), 5.0), 2.5, color_of(&r["color"], accent()));
                    }
                }
                (rows.len() as f32 * (rh + 4.0) - 4.0).max(0.0)
            }
            // (a drive: its letter its icon, in a square filled as full as the drive, its top a rolling wave; its name, the room left of
            // how much; the last minute of what it read (blue) and wrote (amber); pressed: its action; right-clicked: its menu. A ghost: no
            // ground until the mouse is on it)
            "drive" => {
                let h = 58.0;
                let row = Rect::new(x - 2.0, y, w + 4.0, h);
                // (its things a little in from its ground's sides)
                let (x, w) = (x + 10.0, w - 20.0);
                let state = s(b, "state");
                let blue = crate::paint::Color(0.31, 0.56, 0.94, 1.0);
                let busy = b["busy"] == true;
                if state == "ejected" {
                    self.round(row, 14.0, GREEN.alpha(0.12));
                } else if !s(b, "action").is_empty() {
                    self.ghost(row, 14.0);
                }
                let tile = Rect::new(x, y + 6.0, 46.0, 46.0);
                self.round(tile, 12.0, TRACK);
                let fill = if state == "ejected" { GREEN } else if busy { blue } else if b["red"] == true { RED } else { GREEN };
                if self.draw {
                    let speed = if busy { 0.9 } else { 0.35 };
                    self.p.wave_fill(tile, 12.0, f(b, "part"), (self.p.t * speed).fract(), 2.2, fill.alpha(0.32));
                    crate::draw::keep_moving();
                }
                let l = s(b, "letter");
                if state == "ejected" {
                    self.icon("check", tile.x + 13.0, tile.y + 13.0, 20.0, GREEN);
                } else {
                    let lw = self.p.measure(l, 20.0, Weight::Semibold, 40.0);
                    self.text(l, tile.x + 23.0 - lw / 2.0, tile.y + 23.0, 20.0, Weight::Semibold, 40.0, if busy { crate::paint::Color(0.81, 0.88, 1.0, 1.0) } else { TEXT });
                }
                let tx = x + 58.0;
                let room = w - 58.0;
                // (the room left, of how much)
                let of = format!(" of {}", s(b, "total"));
                let ow = self.p.measure(&of, 11.5, Weight::Regular, 90.0);
                self.text_end(&of, x + w, y + 17.0, 11.5, Weight::Regular, 90.0, DIM);
                let fw = self.p.measure(s(b, "free"), 11.5, Weight::Semibold, 90.0);
                self.text_end(s(b, "free"), x + w - ow, y + 17.0, 11.5, Weight::Semibold, 90.0, if b["red"] == true { RED } else { TEXT });
                self.text(s(b, "name"), tx, y + 17.0, 13.0, Weight::Semibold, (room - ow - fw - 10.0).max(20.0), TEXT);
                // (what it read and wrote, the last minute; busy: rising, filled, the speed at its end; refused an eject: says so)
                let amber = crate::draw::AMBER;
                let mut gw = room;
                let note = if state == "refused" { "In Use" } else { s(b, "rate") };
                if !note.is_empty() {
                    let c = if state == "refused" { amber } else if b["write"] == true { amber } else { blue };
                    let nw = self.p.measure(note, 11.5, Weight::Semibold, 100.0);
                    self.text_end(note, x + w, y + 40.0, 11.5, Weight::Semibold, 100.0, c);
                    gw -= nw + 8.0;
                }
                if self.draw && state != "ejected" {
                    let vals = |k: &str| b[k].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect::<Vec<f32>>()).unwrap_or_default();
                    let (reads, writes) = (vals("reads"), vals("writes"));
                    let gr = Rect::new(tx, y + 30.0, gw.max(20.0), 20.0);
                    let writing = b["write"] == true;
                    if reads.len() >= 2 {
                        self.p.line_graph(&reads, gr, blue.alpha(if busy && !writing { 1.0 } else { 0.5 }), busy && !writing, busy && !writing);
                    } else {
                        self.p.round(Rect::new(gr.x, gr.bottom() - 1.5, gr.w, 1.5), 0.75, blue.alpha(0.4));
                    }
                    if writes.len() >= 2 {
                        self.p.line_graph(&writes, gr, amber.alpha(if busy && writing { 1.0 } else { 0.45 }), busy && writing, busy && writing);
                    }
                }
                if !s(b, "action").is_empty() {
                    let tip = if s(b, "tip").is_empty() { Value::Null } else { b["tip"].clone() };
                    self.hit_tip(row, Press::Action(s(b, "action").to_string(), b["args"].clone()), tip);
                    self.last_hit(false, b["menu"].clone());
                }
                h
            }
            // (drives, a line each, for a tip: its letter in its ring, its name, how much room or how fast on the right)
            "drivelines" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                let blue = crate::paint::Color(0.31, 0.56, 0.94, 1.0);
                for (i, d) in rows.iter().enumerate() {
                    let cy = y + i as f32 * 32.0 + 14.0;
                    let busy = d["busy"] == true;
                    let part = f(d, "part");
                    if self.draw {
                        if busy {
                            self.p.circle(x + 12.0, cy, 13.0, blue.alpha(0.14));
                            self.p.ring(x + 12.0, cy, 10.5, 2.2, part.max(0.25), (self.p.t * 0.8).fract(), blue, FAINT);
                            crate::draw::keep_moving();
                        } else {
                            self.p.ring(x + 12.0, cy, 10.5, 2.2, part, 0.0, if d["red"] == true { RED } else { GREEN }, FAINT);
                        }
                    }
                    let l = s(d, "letter");
                    let lw = self.p.measure(l, 10.0, Weight::Semibold, 20.0);
                    self.text(l, x + 12.0 - lw / 2.0, cy, 10.0, Weight::Semibold, 20.0, if busy { crate::paint::Color(0.81, 0.88, 1.0, 1.0) } else { TEXT });
                    // (on the right: reading and writing, each in its colour; else the room left, of how much)
                    let mut rx = x + w;
                    if busy {
                        for (k, c) in [("write", crate::draw::AMBER), ("read", blue)] {
                            let t = s(d, k);
                            if t.is_empty() {
                                continue;
                            }
                            let tw = self.p.measure(t, 11.5, Weight::Semibold, 100.0);
                            self.text_end(t, rx, cy, 11.5, Weight::Semibold, 100.0, c);
                            rx -= tw + 12.0;
                        }
                    } else {
                        let of = format!(" of {}", s(d, "total"));
                        let ow = self.p.measure(&of, 11.5, Weight::Regular, 90.0);
                        self.text_end(&of, rx, cy, 11.5, Weight::Regular, 90.0, DIM);
                        let fw = self.p.measure(s(d, "free"), 11.5, Weight::Semibold, 90.0);
                        self.text_end(s(d, "free"), rx - ow, cy, 11.5, Weight::Semibold, 90.0, if d["red"] == true { RED } else { TEXT });
                        rx -= ow + fw + 10.0;
                    }
                    self.text(s(d, "name"), x + 32.0, cy, 12.0, Weight::Semibold, (rx - x - 32.0).max(20.0), TEXT);
                }
                (rows.len() as f32 * 32.0 - 4.0).max(0.0)
            }
            // (how fast, over time: a line, softly filled under it, its newest point a dot; how fast now at its end)
            // (the battery's tank: a box of liquid at the charge, its surface waving (bubbles rising while it charges); over it the
            // number, what it means, and a few small numbers at its foot. The words are dark where the liquid is, white above it)
            "tank" => {
                let h = b["height"].as_f64().unwrap_or(150.0) as f32;
                if self.draw {
                    let r = Rect::new(x, y, w, h);
                    let c = color_of(&b["color"], GREEN);
                    let part = b["part"].as_f64().unwrap_or(0.0) as f32;
                    let shown = if part > 0.0 { 0.06 + part.clamp(0.0, 1.0) * 0.94 } else { 0.0 };
                    let (ph, amp) = (self.p.t * 0.22, 4.5);
                    self.p.round(r, 18.0, TRACK);
                    self.p.wave_fill(r, 18.0, shown, ph, amp, c);
                    if b["charging"] == true {
                        for i in 0..6 {
                            let k = (self.p.t * 0.35 + i as f32 / 6.0).fract();
                            let bx = x + w * (0.18 + 0.13 * i as f32);
                            let depth = h * shown;
                            let by = y + h - 6.0 - k * (depth - 10.0).max(8.0);
                            let a = (1.0 - k) * (k * 5.0).min(1.0) * 0.45;
                            self.p.circle(bx, by, 2.0 + (i % 3) as f32 * 0.6, crate::paint::Color(1.0, 1.0, 1.0, a));
                        }
                    }
                    crate::draw::keep_moving();
                    let words = |ink: crate::paint::Color| {
                        self.p.text(s(b, "title"), x + 16.0, y + 72.0, 42.0, Weight::Bold, w - 32.0, ink);
                        self.p.text(s(b, "sub"), x + 16.0, y + 103.0, 13.0, Weight::Semibold, w - 32.0, ink);
                        let mut fx = x + 16.0;
                        for f in b["foot"].as_array().cloned().unwrap_or_default() {
                            let t = f.as_str().unwrap_or("");
                            fx += self.p.text(t, fx, y + h - 17.0, 12.0, Weight::Bold, 200.0, ink) + 14.0;
                        }
                    };
                    words(TEXT);
                    let under = Painter::wave_path(r, shown, ph, amp);
                    self.p.within_path(&under, || words(crate::paint::Color(0.016, 0.075, 0.043, 0.92)));
                    if b["tip"].is_string() {
                        self.hit_tip(r, Press::Action(String::new(), Value::Null), b["tip"].clone());
                    }
                }
                h
            }
            // (the apps that drew the most: a switch of how far back on its right; each app its picture, its name, its share and a bar;
            // what no app accounts for last, grey)
            // (the Recycle Bin: each thing thrown away (its picture, its name, where it came from, how long ago, how big), Put Back on
            // its right, its menu on a right-click; at the foot, how much of the room is used, File Explorer, and Empty, held)
            "binrows" => {
                let mut yy = y;
                for row in b["rows"].as_array().cloned().unwrap_or_default() {
                    let rh = 38.0;
                    let rr = Rect::new(x - 6.0, yy, w + 12.0, rh);
                    let bb = Rect::new(x + w - 28.0, yy + 5.0, 28.0, 28.0);
                    if self.draw && hovered(rr) {
                        self.p.round(rr, 9.0, LIT);
                    }
                    if self.draw && hovered(bb) {
                        self.p.round(bb, 8.0, SOFT);
                    }
                    self.icon("back", bb.x + 7.0, bb.y + 7.0, 14.0, if hovered(bb) { TEXT } else { DIM });
                    self.hit_tip(bb, Press::Action(s(&row, "back").to_string(), row["args"].clone()), json!("Restore"));
                    self.hit(rr, Press::Action(String::new(), Value::Null));
                    self.last_hit(false, row["menu"].clone());
                    let ic = Rect::new(x, yy + 5.0, 28.0, 28.0);
                    if self.draw && !self.p.picture(s(&row, "image"), ic, 7.0) {
                        self.p.round(ic, 7.0, TRACK);
                        self.p.icon("file", ic.x + 7.0, ic.y + 7.0, 14.0, DIM);
                    }
                    let (sw, ww) = (56.0, 76.0);
                    self.text_end(s(&row, "size"), x + w - 36.0, yy + 19.0, 11.0, Weight::Regular, sw, TEXT.alpha(0.8));
                    self.text_end(s(&row, "when"), x + w - 36.0 - sw - 6.0, yy + 19.0, 11.0, Weight::Regular, ww, DIM);
                    let tx = x + 38.0;
                    let tw = (w - 38.0 - 36.0 - sw - ww - 12.0).max(40.0);
                    self.text(s(&row, "name"), tx, yy + 13.0, 12.5, Weight::Semibold, tw, TEXT);
                    self.text(s(&row, "sub"), tx, yy + 27.0, 10.5, Weight::Regular, tw, DIM);
                    yy += rh + 2.0;
                }
                if !s(b, "more").is_empty() {
                    self.text(s(b, "more"), x, yy + 9.0, 10.5, Weight::Regular, w, DIM);
                    yy += 20.0;
                }
                yy += 6.0;
                if self.draw {
                    self.p.round(Rect::new(x, yy, w, 1.0), 0.5, FAINT);
                }
                yy += 10.0;
                let part = f(b, "used").clamp(0.0, 1.0);
                if self.draw {
                    let bar = Rect::new(x, yy + 12.5, 90.0, 5.0);
                    self.p.round(bar, 2.5, TRACK);
                    if part > 0.0 {
                        self.p.round(Rect::new(bar.x, bar.y, (bar.w * part).max(5.0), bar.h), 2.5, if part > 0.85 { AMBER } else { accent() });
                    }
                }
                self.text(s(b, "used_text"), x + 100.0, yy + 15.0, 11.0, Weight::Regular, w - 180.0, DIM);
                let ob = Rect::new(x + w - 64.0, yy, 30.0, 30.0);
                let eb = Rect::new(x + w - 30.0, yy, 30.0, 30.0);
                for (r, ic, act, tip, hold, col) in [(ob, "folder", s(b, "open").to_string(), "Open in File Explorer", false, DIM), (eb, "trash", s(b, "empty").to_string(), "Empty", true, RED)] {
                    if self.draw && hovered(r) {
                        self.p.round(r, 9.0, SOFT);
                    }
                    self.icon(ic, r.x + 8.0, r.y + 8.0, 14.0, col);
                    self.hit_tip(r, Press::Action(act, Value::Null), json!(tip));
                    self.last_hit(hold, Value::Null);
                }
                yy += 30.0;
                yy - y
            }
            "appshares" => {
                let mut yy = y;
                let opts = b["options"].as_array().cloned().unwrap_or_default();
                if !opts.is_empty() {
                    let ow: Vec<f32> = opts.iter().map(|o| self.p.measure(s(o, "label"), 11.0, Weight::Semibold, 80.0) + 18.0).collect();
                    let total: f32 = ow.iter().sum::<f32>() + 6.0;
                    let mut ox = x + w - total + 3.0;
                    self.round(Rect::new(x + w - total, yy, total, 26.0), 10.0, crate::paint::Color(0.09, 0.09, 0.1, 1.0));
                    for (o, ww) in opts.iter().zip(ow) {
                        let r = Rect::new(ox, yy + 3.0, ww, 20.0);
                        let on = o["value"] == b["value"];
                        if on {
                            self.round(r, 7.0, crate::paint::Color(0.22, 0.22, 0.25, 1.0));
                        } else if self.draw && hovered(r) {
                            self.round(r, 7.0, LIT);
                        }
                        self.text(s(o, "label"), r.x + 9.0, r.y + 10.0, 11.0, Weight::Semibold, 80.0, if on { TEXT } else { DIM });
                        if !on {
                            self.hit(r, Press::Action(s(b, "action").to_string(), json!({ "value": o["value"] })));
                        }
                        ox += ww;
                    }
                    yy += 36.0;
                }
                for row in b["rows"].as_array().cloned().unwrap_or_default() {
                    let sys = row["system"] == true;
                    let ic = Rect::new(x, yy + 3.0, 30.0, 30.0);
                    if self.draw && (sys || !self.p.picture(s(&row, "image"), ic, 8.0)) {
                        self.p.round(ic, 8.0, TRACK);
                        self.p.icon(if sys { "desktop" } else { "grid" }, ic.x + 7.0, ic.y + 7.0, 16.0, DIM);
                    }
                    let tx = x + 42.0;
                    let right = s(&row, "text");
                    let rw = self.p.measure(right, 12.5, Weight::Semibold, 80.0);
                    self.text(s(&row, "name"), tx, yy + 11.0, 13.0, Weight::Semibold, w - 42.0 - rw - 10.0, if sys { DIM } else { TEXT });
                    self.text_end(right, x + w, yy + 11.0, 12.5, Weight::Semibold, 80.0, DIM);
                    if self.draw {
                        let br = Rect::new(tx, yy + 25.0, w - 42.0, 4.0);
                        self.p.round(br, 2.0, crate::paint::Color(0.15, 0.15, 0.17, 1.0));
                        let part = row["part"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                        if part > 0.0 {
                            self.p.round(Rect::new(br.x, br.y, (br.w * part).max(4.0), br.h), 2.0, if sys { crate::paint::Color(0.36, 0.36, 0.4, 1.0) } else { color_of(&row["color"], GREEN) });
                        }
                    }
                    if row["tip"].is_string() {
                        self.hit_tip(Rect::new(x, yy, w, 36.0), Press::Action(String::new(), Value::Null), row["tip"].clone());
                    }
                    yy += 42.0;
                }
                (yy - y - 6.0).max(0.0)
            }
            "linegraph" => {
                let vals: Vec<f32> = b["values"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                let h = b["height"].as_f64().unwrap_or(40.0) as f32;
                let c = color_of(&b["color"], accent());
                let t = s(b, "text");
                let tw = if t.is_empty() { 0.0 } else { self.p.measure(t, 12.0, Weight::Semibold, 120.0) + 10.0 };
                if !t.is_empty() {
                    self.text_end(t, x + w, y + h - 8.0, 12.0, Weight::Semibold, 120.0, TEXT);
                }
                if self.draw && vals.len() >= 2 {
                    let max = b["max"].as_f64().map(|m| m as f32).unwrap_or_else(|| vals.iter().cloned().fold(1e-6, f32::max)).max(1e-6);
                    let norm: Vec<f32> = vals.iter().map(|v| v / max).collect();
                    self.p.line_graph(&norm, Rect::new(x, y + 2.0, w - tw, h - 4.0), c, true, true);
                }
                h
            }
            // (a download: its picture, its name and how fast, the time left on the right, its bar; done: a line, a check, its name
            // (pressed: its action) and ghost buttons)
            "transfer" => {
                let c = color_of(&b["color"], crate::paint::Color(0.31, 0.56, 0.94, 1.0));
                let action = s(b, "action");
                if b["done"] == true {
                    let h = 32.0;
                    let row = Rect::new(x - 6.0, y, w + 12.0, h);
                    if !action.is_empty() {
                        self.ghost(row, 10.0);
                    }
                    self.icon("check", x, y + 8.0, 16.0, GREEN);
                    let buttons = b["buttons"].as_array().cloned().unwrap_or_default();
                    let bw = buttons.len() as f32 * 30.0;
                    self.text(s(b, "title"), x + 24.0, y + h / 2.0, 12.0, Weight::Regular, w - 24.0 - bw - 6.0, TEXT);
                    if !action.is_empty() {
                        self.hit(Rect::new(row.x, row.y, row.w - bw - 6.0, row.h), Press::Action(action.to_string(), b["args"].clone()));
                    }
                    for (i, bt) in buttons.iter().enumerate() {
                        let r = Rect::new(x + w - bw + i as f32 * 30.0, y + 1.0, 30.0, 30.0);
                        if self.draw && hovered(r) {
                            self.p.circle(r.x + 15.0, r.y + 15.0, 14.0, LIT);
                        }
                        self.icon(s(bt, "icon"), r.x + 8.0, r.y + 8.0, 14.0, if hovered(r) { TEXT } else { DIM });
                        let tip = if s(bt, "tip").is_empty() { Value::Null } else { bt["tip"].clone() };
                        self.hit_tip(r, Press::Action(s(bt, "action").to_string(), bt["args"].clone()), tip);
                    }
                    return h;
                }
                let h = 52.0;
                // (its picture: a game's art, wide; else its own icon; neither: none, its words from its start)
                let img = s(b, "image");
                let icon = s(b, "icon");
                let wide = b["wide"] == true;
                let pr = if wide { Rect::new(x, y + 4.0, 64.0, 30.0) } else { Rect::new(x, y + 3.0, 32.0, 32.0) };
                let pictured = !img.is_empty() || !icon.is_empty();
                if pictured && !(self.draw && !img.is_empty() && self.p.picture(img, pr, 7.0)) {
                    self.round(Rect::new(x, y + 3.0, 32.0, 32.0), 9.0, TRACK);
                    self.icon(if icon.is_empty() { "download" } else { icon }, x + 8.0, y + 11.0, 16.0, DIM);
                }
                let tx = x + if !pictured { 0.0 } else if wide { 74.0 } else { 42.0 };
                let right = s(b, "right");
                let rw = if right.is_empty() { 0.0 } else { self.p.measure(right, 12.0, Weight::Semibold, 90.0) + 8.0 };
                self.text_end(right, x + w, y + 19.0, 12.0, Weight::Semibold, 90.0, TEXT);
                self.text(s(b, "title"), tx, y + 12.0, 12.5, Weight::Semibold, w - (tx - x) - rw, TEXT);
                self.text(s(b, "sub"), tx, y + 28.0, 11.0, Weight::Regular, w - (tx - x) - rw, DIM);
                let bar = Rect::new(x, y + h - 8.0, w, 4.0);
                self.round(bar, 2.0, TRACK);
                if let Some(part) = b["part"].as_f64() {
                    self.round(Rect::new(bar.x, bar.y, (bar.w * part.clamp(0.0, 1.0) as f32).max(4.0), 4.0), 2.0, c);
                } else if self.draw {
                    // (how much is not known: a short piece sliding along)
                    let k = (self.p.t * 0.6).fract();
                    self.p.round(Rect::new(bar.x + (bar.w - 60.0) * k, bar.y, 60.0, 4.0), 2.0, c.alpha(0.7));
                    crate::draw::keep_moving();
                }
                if !action.is_empty() {
                    self.hit(Rect::new(x - 6.0, y, w + 12.0, h), Press::Action(action.to_string(), b["args"].clone()));
                }
                h
            }
            // (a day's head: the time, big; the day and the date on its right; a ghost button beside them (the month))
            "dayhead" => {
                let h = 46.0;
                let cy = y + h / 2.0;
                let time = s(b, "time");
                let ampm = s(b, "ampm");
                let tw = self.text(time, x, cy, 34.0, Weight::Semibold, w * 0.6, TEXT);
                if !ampm.is_empty() {
                    self.text(ampm, x + tw + 4.0, cy + 6.0, 14.0, Weight::Semibold, 40.0, DIM);
                }
                let mut rx = x + w;
                if !s(b, "toggle").is_empty() {
                    let r = Rect::new(rx - 30.0, cy - 15.0, 30.0, 30.0);
                    if self.draw && hovered(r) {
                        self.p.circle(r.x + 15.0, cy, 15.0, LIT);
                    }
                    self.icon(if s(b, "toggle_icon").is_empty() { "calendar" } else { s(b, "toggle_icon") }, r.x + 8.0, cy - 7.0, 14.0, if hovered(r) { TEXT } else { DIM });
                    let tip = if s(b, "toggle_tip").is_empty() { Value::Null } else { b["toggle_tip"].clone() };
                    self.hit_tip(r, Press::Action(s(b, "toggle").to_string(), Value::Null), tip);
                    rx -= 36.0;
                }
                self.text_end(s(b, "day"), rx, cy - 8.0, 12.0, Weight::Regular, w * 0.4, DIM);
                self.text_end(s(b, "date"), rx, cy + 9.0, 13.0, Weight::Semibold, w * 0.4, TEXT);
                h
            }
            // (a week, a day each: its letter, its number, a dot for each of its events; the day picked lit, today's number in the theme's
            // colour; ‹ › at its ends)
            "weekstrip" => {
                let h = 60.0;
                let days = b["days"].as_array().cloned().unwrap_or_default();
                let side = 24.0;
                let cy = y + h / 2.0;
                for (k, a, icon) in [("prev", x, "left"), ("next", x + w - side, "chevron")] {
                    let act = s(b, k);
                    if act.is_empty() {
                        continue;
                    }
                    let r = Rect::new(a, cy - 15.0, side, 30.0);
                    if self.draw && hovered(r) {
                        self.p.round(r, 12.0, LIT);
                    }
                    self.chev(a + side / 2.0, cy, icon == "left", if hovered(r) { TEXT } else { DIM });
                    self.hit(r, Press::Action(act.to_string(), Value::Null));
                }
                let (x0, iw) = (x + side + 2.0, w - 2.0 * side - 4.0);
                let n = days.len().max(1) as f32;
                let cw = iw / n;
                for (i, d) in days.iter().enumerate() {
                    let r = Rect::new(x0 + i as f32 * cw + 2.0, y, cw - 4.0, h);
                    let on = d["on"] == true;
                    let today = d["today"] == true;
                    if on {
                        self.round(r, 14.0, TEXT);
                    } else if !s(d, "action").is_empty() && self.draw && hovered(r) {
                        self.p.round(r, 14.0, LIT);
                    }
                    let mid = r.x + r.w / 2.0;
                    let l = s(d, "label");
                    let lw = self.p.measure(l, 10.0, Weight::Regular, cw);
                    self.text(l, mid - lw / 2.0, y + 13.0, 10.0, Weight::Regular, cw, if on { crate::paint::Color(0.16, 0.16, 0.19, 1.0) } else { DIM });
                    let num = s(d, "num");
                    let nw = self.p.measure(num, 16.0, Weight::Semibold, cw);
                    let nc = if on { crate::paint::Color(0.0, 0.0, 0.0, 1.0) } else if today { accent() } else { TEXT };
                    self.text(num, mid - nw / 2.0, y + 32.0, 16.0, Weight::Semibold, cw, nc);
                    let dots = d["dots"].as_array().cloned().unwrap_or_default();
                    let dn = dots.len().min(3) as f32;
                    let dx = mid - (dn * 4.0 + (dn - 1.0).max(0.0) * 2.0) / 2.0;
                    for (k, c) in dots.iter().take(3).enumerate() {
                        if self.draw {
                            self.p.circle(dx + k as f32 * 6.0 + 2.0, y + 48.0, 2.0, color_of(c, accent()));
                        }
                    }
                    if !s(d, "action").is_empty() {
                        self.hit(r, Press::Action(s(d, "action").to_string(), Value::Null));
                    }
                }
                h
            }
            // (what is next: a ring running down to it, its words inside; its title and when; a button to join it)
            "nextup" => {
                let h = 64.0;
                self.round(Rect::new(x, y, w, h), 16.0, CARD);
                let c = color_of(&b["color"], accent());
                let (rx, cy) = (x + 12.0 + 22.0, y + h / 2.0);
                if self.draw {
                    self.p.ring(rx, cy, 19.0, 3.0, f(b, "part"), 0.0, c, TRACK);
                }
                let t = s(b, "text");
                let tw = self.p.measure(t, 11.0, Weight::Bold, 40.0);
                self.text(t, rx - tw / 2.0, cy, 11.0, Weight::Bold, 40.0, c);
                let mut right = x + w - 12.0;
                if !s(b, "join").is_empty() {
                    let label = "Join";
                    let lw = self.p.measure(label, 12.0, Weight::Semibold, 60.0) + 34.0;
                    let r = Rect::new(right - lw, cy - 15.0, lw, 30.0);
                    self.round(r, 15.0, accent());
                    self.icon("play", r.x + 10.0, cy - 6.0, 12.0, crate::paint::Color(0.02, 0.07, 0.04, 1.0));
                    self.text(label, r.x + 26.0, cy, 12.0, Weight::Semibold, 60.0, crate::paint::Color(0.02, 0.07, 0.04, 1.0));
                    self.hit(r, Press::Action(s(b, "join").to_string(), Value::Null));
                    right = r.x - 8.0;
                }
                let tx = x + 12.0 + 44.0 + 12.0;
                self.text(s(b, "title"), tx, cy - 8.0, 13.0, Weight::Semibold, right - tx, TEXT);
                self.text(s(b, "sub"), tx, cy + 9.0, 11.5, Weight::Regular, right - tx, DIM);
                h
            }
            // (a day's events in time: each a bar of its colour on a ground of it, its time beside it; past ones faded; a red line where
            // now is)
            "timeline" => {
                let rows = b["rows"].as_array().cloned().unwrap_or_default();
                let now_at = b["now"].as_i64().unwrap_or(-1);
                let red = crate::paint::Color(0.91, 0.42, 0.42, 1.0);
                let mut yy = y;
                let tcol = 44.0;
                for (i, r) in rows.iter().enumerate() {
                    if i as i64 == now_at {
                        if self.draw {
                            self.p.round(Rect::new(x + tcol + 10.0, yy + 1.0, w - tcol - 10.0, 1.5), 0.75, red);
                            self.p.circle(x + tcol + 10.0, yy + 1.75, 4.0, red);
                        }
                        yy += 8.0;
                    }
                    let c = color_of(&r["color"], accent());
                    let past = r["past"] == true;
                    let k = if past { 0.5 } else { 1.0 };
                    let has_sub = !s(r, "sub").is_empty();
                    let rh = if has_sub { 46.0 } else { 32.0 };
                    self.text_end(s(r, "time"), x + tcol, yy + 13.0, 11.0, Weight::Regular, tcol, DIM.alpha(k));
                    let card = Rect::new(x + tcol + 10.0, yy, w - tcol - 10.0, rh);
                    self.round(card, 12.0, c.alpha(0.13 * k));
                    self.round(Rect::new(card.x + 1.0, card.y + 7.0, 3.0, rh - 14.0), 1.5, c.alpha(k));
                    self.text(s(r, "title"), card.x + 12.0, yy + if has_sub { 14.0 } else { rh / 2.0 }, 12.5, Weight::Semibold, card.w - 20.0, TEXT.alpha(k));
                    if has_sub {
                        self.text(s(r, "sub"), card.x + 12.0, yy + 31.0, 11.0, Weight::Regular, card.w - 20.0, DIM.alpha(k));
                    }
                    if !s(r, "action").is_empty() {
                        self.hit(card, Press::Action(s(r, "action").to_string(), Value::Null));
                    }
                    yy += rh + 6.0;
                }
                if now_at >= rows.len() as i64 && !rows.is_empty() {
                    if self.draw {
                        self.p.round(Rect::new(x + tcol + 10.0, yy + 1.0, w - tcol - 10.0, 1.5), 0.75, red);
                        self.p.circle(x + tcol + 10.0, yy + 1.75, 4.0, red);
                    }
                    yy += 8.0;
                }
                (yy - y - 6.0).max(0.0)
            }
            // (a month: its name between ‹ ›, a ghost to go back, the days of the week, then its days (only its own): today in the theme's
            // colour, the day picked ringed, weekends softer, a dot for each event under its number)
            "monthgrid" => {
                let mut h = 0.0;
                let hy = y + 15.0;
                for (k, a, icon) in [("prev", x, "left"), ("next", x + w - 30.0, "chevron")] {
                    let act = s(b, k);
                    if act.is_empty() {
                        continue;
                    }
                    let r = Rect::new(a, y, 30.0, 30.0);
                    if self.draw && hovered(r) {
                        self.p.circle(r.x + 15.0, hy, 15.0, LIT);
                    }
                    self.chev(a + 15.0, hy, icon == "left", if hovered(r) { TEXT } else { DIM });
                    self.hit(r, Press::Action(act.to_string(), Value::Null));
                }
                let title = s(b, "title");
                let tw = self.p.measure(title, 16.0, Weight::Semibold, w - 80.0);
                let tr = Rect::new(x + w / 2.0 - tw / 2.0 - 8.0, y, tw + 16.0, 30.0);
                if !s(b, "press").is_empty() {
                    self.ghost(tr, 12.0);
                    self.hit(tr, Press::Action(s(b, "press").to_string(), Value::Null));
                }
                self.text(title, x + w / 2.0 - tw / 2.0, hy, 16.0, Weight::Semibold, w - 80.0, TEXT);
                h += 38.0;
                let cw = w / 7.0;
                for (i, l) in b["labels"].as_array().cloned().unwrap_or_default().iter().enumerate().take(7) {
                    let l = l.as_str().unwrap_or("");
                    let lw = self.p.measure(l, 10.0, Weight::Regular, cw);
                    self.text(l, x + i as f32 * cw + cw / 2.0 - lw / 2.0, y + h + 6.0, 10.0, Weight::Regular, cw, SOFT_DIM);
                }
                h += 16.0;
                let lead = b["lead"].as_u64().unwrap_or(0) as usize;
                let rh = 42.0;
                let days = b["days"].as_array().cloned().unwrap_or_default();
                for (i, d) in days.iter().enumerate() {
                    let k = lead + i;
                    let r = Rect::new(x + (k % 7) as f32 * cw + 2.0, y + h + (k / 7) as f32 * rh + 2.0, cw - 4.0, rh - 4.0);
                    let today = d["today"] == true;
                    let sel = d["sel"] == true;
                    if today {
                        self.round(r, 12.0, accent());
                    } else if !s(d, "action").is_empty() && self.draw && hovered(r) {
                        self.p.round(r, 12.0, LIT);
                    }
                    if sel && self.draw {
                        self.p.outline(r, 12.0, 1.5, TEXT);
                    }
                    let num = s(d, "num");
                    let nw = self.p.measure(num, 14.0, Weight::Semibold, cw);
                    let nc = if today { crate::paint::Color(0.02, 0.07, 0.04, 1.0) } else if d["weekend"] == true { DIM } else { TEXT };
                    let dots = d["dots"].as_array().cloned().unwrap_or_default();
                    let ny = if dots.is_empty() { r.y + r.h / 2.0 } else { r.y + r.h / 2.0 - 4.0 };
                    self.text(num, r.x + r.w / 2.0 - nw / 2.0, ny, 14.0, Weight::Semibold, cw, nc);
                    let dn = dots.len().min(3) as f32;
                    let dx = r.x + r.w / 2.0 - (dn * 4.0 + (dn - 1.0).max(0.0) * 2.0) / 2.0;
                    for (j, c) in dots.iter().take(3).enumerate() {
                        if self.draw {
                            self.p.circle(dx + j as f32 * 6.0 + 2.0, r.y + r.h / 2.0 + 9.0, 2.0, if today { crate::paint::Color(0.02, 0.07, 0.04, 1.0) } else { color_of(c, accent()) });
                        }
                    }
                    if !s(d, "action").is_empty() {
                        self.hit(r, Press::Action(s(d, "action").to_string(), Value::Null));
                    }
                }
                let rows = (lead + days.len()).div_ceil(7);
                h + rows as f32 * rh
            }
            // (blocks on a ground of their own)
            "card" => {
                let body = b["body"].as_array().cloned().unwrap_or_default();
                let pad = 12.0;
                let was = self.draw;
                self.draw = false;
                let ih = self.body(&body, x + pad, y + pad, w - 2.0 * pad);
                self.draw = was;
                self.round(Rect::new(x, y, w, ih + 2.0 * pad), 16.0, CARD);
                self.body(&body, x + pad, y + pad, w - 2.0 * pad);
                ih + 2.0 * pad
            }
            // (where a page is from: a pictogram, the site, and on the right a ring of how far you are or a chip)
            "site" => {
                let cy = y + 10.0;
                let mut tx = x;
                let img = s(b, "image");
                let drawn = !img.is_empty() && self.draw && self.p.picture(img, Rect::new(x, cy - 8.0, 16.0, 16.0), 4.0);
                if !drawn && !s(b, "icon").is_empty() {
                    self.icon(s(b, "icon"), x, cy - 8.0, 16.0, color_of(&b["color"], DIM));
                }
                if drawn || !s(b, "icon").is_empty() {
                    tx += 24.0;
                }
                let mut rw = 0.0;
                if let Some(part) = b["ring"].as_f64() {
                    if self.draw {
                        self.p.ring(x + w - 10.0, cy, 8.5, 2.4, part as f32, 0.0, accent(), TRACK);
                    }
                    rw = 26.0;
                } else if let Some(ch) = b["chip"].as_object() {
                    let chip = Value::Object(ch.clone());
                    let (icon, text) = (s(&chip, "icon"), s(&chip, "text"));
                    let cw = self.p.measure(text, 11.5, Weight::Semibold, 120.0) + 18.0 + if icon.is_empty() { 0.0 } else { 17.0 };
                    let r = Rect::new(x + w - cw, cy - 11.0, cw, 22.0);
                    let tc = match s(&chip, "tone") {
                        "" => DIM,
                        t => tone_color(t),
                    };
                    self.round(r, 11.0, FAINT);
                    let mut ix = r.x + 9.0;
                    if !icon.is_empty() {
                        self.icon(icon, ix, cy - 7.0, 14.0, tc);
                        ix += 17.0;
                    }
                    self.text(text, ix, cy, 11.5, Weight::Semibold, 120.0, tc);
                    rw = cw + 8.0;
                }
                self.text(s(b, "text"), tx, cy, 11.0, Weight::Regular, (w - (tx - x) - rw).max(10.0), DIM);
                20.0
            }
            // (a title, on a line of its own, larger)
            "title" => {
                let action = s(b, "action");
                if !action.is_empty() {
                    let r = Rect::new(x - 6.0, y - 3.0, w + 12.0, 26.0);
                    self.ghost(r, 8.0);
                    self.hit(r, Press::Action(action.to_string(), b["args"].clone()));
                }
                self.text(s(b, "text"), x, y + 10.0, 14.5, Weight::Semibold, w, TEXT);
                20.0
            }
            // (steps as a strip: the ones done faint, the one you are on lit; pressed: its action with its number)
            "segs" => {
                let n = b["n"].as_u64().unwrap_or(0) as usize;
                let at = b["at"].as_u64().unwrap_or(0) as usize;
                if n == 0 {
                    return 0.0;
                }
                let c = accent();
                if n > 40 {
                    self.round(Rect::new(x, y, w, 5.0), 2.5, TRACK);
                    self.round(Rect::new(x, y, (w * at as f32 / n as f32).max(5.0), 5.0), 2.5, c);
                    return 5.0;
                }
                let gap = 3.0;
                let sw = (w - gap * (n - 1) as f32) / n as f32;
                let action = s(b, "action");
                for i in 0..n {
                    let r = Rect::new(x + i as f32 * (sw + gap), y, sw, 5.0);
                    let col = if i + 1 < at { c.alpha(0.45) } else if i + 1 == at { c } else { TRACK };
                    let hit = Rect::new(r.x - gap / 2.0, y - 6.0, sw + gap, 17.0);
                    let col = if !action.is_empty() && self.draw && hovered(hit) && i + 1 != at { DIM } else { col };
                    self.round(r, 2.5, col);
                    if !action.is_empty() {
                        self.hit(hit, Press::Action(format!("{action}{}", i + 1), Value::Null));
                    }
                }
                5.0
            }
            // (a step read out: its heading, then its words)
            "step" => {
                let mut h = 0.0;
                if !s(b, "head").is_empty() {
                    self.text(s(b, "head"), x, y + 9.0, 13.0, Weight::Semibold, w, TEXT);
                    h += 22.0;
                }
                if !s(b, "text").is_empty() {
                    h += self.p.paragraph(s(b, "text"), x, y + h, 12.5, w, b["lines"].as_u64().unwrap_or(10) as usize, SOFT_ICON, self.draw);
                }
                h
            }
            // (the step before and after: a ghost button each (only where there is one), the next one's name between)
            "stepnav" => {
                let cy = y + 15.0;
                for (k, icon, side) in [("prev", "left", 0.0), ("next", "chevron", 1.0)] {
                    let a = s(b, k);
                    if a.is_empty() {
                        continue;
                    }
                    let r = Rect::new(x + side * (w - 30.0), y, 30.0, 30.0);
                    if self.draw && hovered(r) {
                        self.p.circle(r.x + 15.0, cy, 15.0, LIT);
                    }
                    self.icon(icon, r.x + 8.0, cy - 7.0, 14.0, if hovered(r) { TEXT } else { DIM });
                    self.hit(r, Press::Action(a.to_string(), Value::Null));
                }
                let l = s(b, "label");
                let lw = self.p.measure(l, 11.5, Weight::Regular, w - 80.0);
                self.text(l, x + w / 2.0 - lw / 2.0, cy, 11.5, Weight::Regular, w - 80.0, DIM);
                30.0
            }
            // (the answer to a question: its votes, then its opening, on a green ground)
            "answer" => {
                let pad = 9.0;
                let head = format!("{}", s(b, "votes"));
                let was = self.draw;
                self.draw = false;
                let th = self.p.paragraph(s(b, "text"), x + pad, y + pad + 20.0, 12.0, w - 2.0 * pad, b["lines"].as_u64().unwrap_or(5) as usize, SOFT_ICON, false);
                self.draw = was;
                let h = pad * 2.0 + 20.0 + th;
                self.round(Rect::new(x, y, w, h), 10.0, GREEN.alpha(0.1));
                if b["accepted"] != false {
                    self.icon("check", x + pad, y + pad + 1.0, 14.0, GREEN);
                }
                self.text(&head, x + pad + if b["accepted"] != false { 18.0 } else { 0.0 }, y + pad + 8.0, 12.0, Weight::Semibold, 80.0, GREEN);
                self.p.paragraph(s(b, "text"), x + pad, y + pad + 20.0, 12.0, w - 2.0 * pad, b["lines"].as_u64().unwrap_or(5) as usize, SOFT_ICON, self.draw);
                h
            }
            // (a price, large, the old one struck through; whether it is in stock on the right)
            "price" => {
                let cy = y + 15.0;
                let nw = self.text(s(b, "now"), x, cy, 24.0, Weight::Semibold, w * 0.6, TEXT);
                if !s(b, "was").is_empty() {
                    let ww = self.text(s(b, "was"), x + nw + 10.0, cy + 3.0, 12.0, Weight::Regular, 100.0, DIM);
                    if self.draw {
                        self.p.line(&[(x + nw + 9.0, cy + 3.5), (x + nw + 11.0 + ww, cy + 3.5)], 1.2, DIM);
                    }
                }
                let st = s(b, "stock");
                if !st.is_empty() {
                    let c = if b["ok"] != false { GREEN } else { AMBER };
                    let tw = self.p.measure(st, 11.5, Weight::Semibold, 120.0);
                    let r = Rect::new(x + w - tw - 34.0, cy - 11.0, tw + 34.0, 22.0);
                    self.round(r, 11.0, FAINT);
                    if self.draw {
                        self.p.circle(r.x + 13.0, cy, 3.5, c);
                    }
                    self.text(st, r.x + 23.0, cy, 11.5, Weight::Semibold, 120.0, c);
                }
                30.0
            }
            // (a picture across, as tall as asked)
            "picture" => {
                let h = b["height"].as_f64().unwrap_or(120.0) as f32;
                if s(b, "image").is_empty() {
                    return 0.0;
                }
                if self.draw && !self.p.picture(s(b, "image"), Rect::new(x, y, w, h), 12.0) {
                    self.p.round(Rect::new(x, y, w, h), 12.0, TRACK);
                }
                h
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
            // (the dock open: each app, its name over its windows (each its live picture, its title under it), the apps in rows; at
            // the foot, its pinned ones that are closed)
            "curve" => {
                // a curve over two axes (along: `x`, `{ min, max, log, marks = { { v, t } } }`; up: `y`, the same), drawn from `line`
                // (its values, evenly across), and its `points` (`{ id, x, y }`) to take hold of: dragged, `drag` is told where (`{ id,
                // x, y }`, and `done` when let go); clicked, `pick` (`{ id }`); the wheel over one, `wheel` (`{ id, up }`); a
                // double-click on the ground, `add` (`{ x, y }`). `selected`: the one picked, lit. `ghost`: a second line (its values
                // too), faint, behind it (an equalizer: what is heard, its preamp in it)
                let h = b["height"].as_f64().unwrap_or(200.0) as f32;
                let area = Rect::new(x + 8.0, y + 8.0, w - 16.0, h - 26.0);
                let (x0, x1) = (f(&b["x"], "min"), f(&b["x"], "max"));
                let (y0, y1) = (f(&b["y"], "min"), f(&b["y"], "max"));
                let log = b["x"]["log"] == true && x0 > 0.0 && x1 > 0.0;
                let px = |v: f32| {
                    let fx = if log { (v.max(1e-6) / x0).ln() / (x1 / x0).ln() } else { (v - x0) / (x1 - x0).max(1e-6) };
                    area.x + area.w * fx.clamp(0.0, 1.0)
                };
                let py = |v: f32| area.y + area.h * ((y1 - v) / (y1 - y0).max(1e-6)).clamp(0.0, 1.0);
                let mut spec = b.clone();
                spec["area"] = json!({ "x": area.x, "y": area.y, "w": area.w, "h": area.h });
                if let Some(o) = spec.as_object_mut() {
                    o.remove("line");
                    o.remove("ghost");
                }
                let selected = s(b, "selected").to_string();
                let dragging = POINTING.with(|p| p.borrow().clone());
                let points: Vec<(String, f32, f32)> = b["points"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|pt| {
                                let id = pt["id"].as_str().map(str::to_string).unwrap_or_else(|| pt["id"].to_string());
                                match &dragging {
                                    Some((d, dx, dy)) if *d == id => (id, *dx, *dy),
                                    _ => (id, f(pt, "x"), f(pt, "y")),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if self.draw {
                    self.round(Rect::new(x, y, w, h), 14.0, crate::paint::Color(0.043, 0.047, 0.059, 1.0));
                    let grid = crate::paint::Color(1.0, 1.0, 1.0, 0.07);
                    for m in b["x"]["marks"].as_array().cloned().unwrap_or_default() {
                        let gx = px(f(&m, "v"));
                        self.p.round(Rect::new(gx, area.y, 1.0, area.h), 0.0, grid);
                        self.text(s(&m, "t"), gx + 3.0, area.bottom() + 9.0, 9.5, Weight::Regular, 40.0, DIM);
                    }
                    for m in b["y"]["marks"].as_array().cloned().unwrap_or_default() {
                        let v = f(&m, "v");
                        let gy = py(v);
                        self.p.round(Rect::new(area.x, gy, area.w, 1.0), 0.0, if v == 0.0 { crate::paint::Color(1.0, 1.0, 1.0, 0.14) } else { grid });
                        let t = s(&m, "t");
                        if !t.is_empty() {
                            self.text(t, area.x + 3.0, gy - 7.0, 9.5, Weight::Regular, 40.0, DIM);
                        }
                    }
                    // (its ghost: faint, thin, behind)
                    let ghost: Vec<f32> = b["ghost"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                    if ghost.len() >= 2 {
                        let n = ghost.len();
                        let pts: Vec<(f32, f32)> = ghost.iter().enumerate().map(|(i, v)| (area.x + area.w * i as f32 / (n - 1) as f32, py(*v))).collect();
                        self.p.line(&pts, 1.4, TEXT.alpha(0.28));
                    }
                    // (the curve: filled faintly down to its middle, its line over it)
                    let line: Vec<f32> = b["line"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                    if line.len() >= 2 {
                        let n = line.len();
                        let zero = py(0.0_f32.clamp(y0.min(y1), y0.max(y1)));
                        let pts: Vec<(f32, f32)> = line.iter().enumerate().map(|(i, v)| (area.x + area.w * i as f32 / (n - 1) as f32, py(*v))).collect();
                        let col = if b["off"] == true { DIM } else { accent() };
                        let step = area.w / (n - 1) as f32;
                        for (lx, ly) in &pts {
                            let (top, bot) = if *ly < zero { (*ly, zero) } else { (zero, *ly) };
                            self.p.round(Rect::new(lx - step / 2.0, top, step + 0.4, (bot - top).max(0.0)), 0.0, col.alpha(0.13));
                        }
                        self.p.line(&pts, 2.0, col);
                    }
                    for (id, vx, vy) in &points {
                        let (cx, cy) = (px(*vx), py(*vy));
                        if *id == selected || dragging.as_ref().is_some_and(|(d, ..)| d == id) {
                            self.p.circle(cx, cy, 10.0, accent().alpha(0.25));
                            self.p.circle(cx, cy, 6.0, TEXT);
                        } else {
                            let over = hovered(Rect::new(cx - 8.0, cy - 8.0, 16.0, 16.0));
                            self.p.circle(cx, cy, if over { 6.0 } else { 5.0 }, TEXT);
                            self.p.circle(cx, cy, if over { 4.4 } else { 3.6 }, crate::paint::Color(0.043, 0.047, 0.059, 1.0));
                        }
                    }
                }
                // (its points first, so they are found before its ground)
                for (id, vx, vy) in points.iter().rev() {
                    let (cx, cy) = (px(*vx), py(*vy));
                    self.hit(Rect::new(cx - 9.0, cy - 9.0, 18.0, 18.0), Press::Point(id.clone(), spec.clone()));
                }
                self.hit(area, Press::Point(String::new(), spec));
                h
            }
            "appwindows" => {
                // every window a tile, its live picture filling it; what is drawn over the picture (its name, its use, its sound) is
                // kept for the glass over the panel, where Windows does not draw over it; here too, for while the pictures are not out
                // yet. Its pinned ones, closed, at its foot
                // (room at both sides for the ring round the window you were in)
                let (x, w) = (x + 3.0, w - 6.0);
                let groups = b["groups"].as_array().cloned().unwrap_or_default();
                let pins = b["pins"].as_array().cloned().unwrap_or_default();
                let tiles: Vec<(Value, Value)> = groups.iter().flat_map(|g| g["windows"].as_array().cloned().unwrap_or_default().into_iter().map(move |t| (t, g.clone()))).collect();
                let tile_w = b["tile"].as_f64().map_or(TILE_W, |t| t as f32);
                let cols = (((w + TILE_GAP) / (tile_w + TILE_GAP)).floor() as usize).max(1).min(tiles.len().max(1));
                let tw = ((w - (cols - 1) as f32 * TILE_GAP) / cols as f32).floor();
                let th = (tw * 0.62).round();
                let rows = tiles.len().div_ceil(cols);
                let top = y + 3.0;
                for (i, (it, g)) in tiles.iter().enumerate() {
                    let (row, col) = (i / cols, i % cols);
                    let r = Rect::new(x + col as f32 * (tw + TILE_GAP), top + row as f32 * (th + TILE_GAP), tw, th);
                    let hwnd = it["hwnd"].as_i64().unwrap_or(0) as isize;
                    if self.draw {
                        if it["attention"] == true {
                            self.p.round(Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0), 15.0, crate::draw::AMBER.alpha(0.35));
                        }
                        // (the one you were in: ringed in the theme's colour)
                        if it["front"] == true {
                            self.p.round(Rect::new(r.x - 2.5, r.y - 2.5, r.w + 5.0, r.h + 5.0), 14.0, accent());
                            self.p.round(Rect::new(r.x - 0.5, r.y - 0.5, r.w + 1.0, r.h + 1.0), 12.0, FACE);
                        }
                        self.p.round(r, 12.0, if hovered(r) { LIT } else { GROUND });
                        if hwnd != 0 {
                            self.thumbs.push((hwnd, r));
                        } else {
                            // (put away: no picture, its app's icon in its middle)
                            let z = 34.0;
                            let pr = Rect::new(r.x + r.w / 2.0 - z / 2.0, r.y + r.h * 0.58 - z / 2.0, z, z);
                            if !self.p.picture(s(it, "image"), pr, 8.0) {
                                self.icon("dot", pr.x + 9.0, pr.y + 9.0, 16.0, DIM);
                            }
                        }
                        tile_over(self.p, r, it, g, HOVER.with(|h| h.get()));
                        if hwnd != 0 {
                            OVERS.with(|o| o.borrow_mut().push((hwnd, r, it.clone(), g.clone())));
                        }
                    }
                    // (making sound: its bar's speaker, volume, what it plays; before the tile, so they are found first)
                    if g["sound"] == true && g["volume"].is_number() && g["progress"].is_null() {
                        let key = s(g, "key");
                        let (_, mute, track, media) = tile_bar(r, g);
                        if self.draw {
                            note_slider(&format!("appvol:{key}"), s_num(g, "volume"));
                        }
                        let muted = g["muted"] == true;
                        self.hit_tip(mute, Press::Action(format!("appmute:{key}"), Value::Null), Value::String(if muted { "Unmute" } else { "Mute" }.into()));
                        self.hit(track, Press::Seek(format!("appvol:{key}")));
                        if let Some([prev, play, next]) = media {
                            let src = s(&g["media"], "source");
                            self.hit(prev, Press::Action(format!("mediaprev:{src}"), Value::Null));
                            self.hit(play, Press::Action(format!("mediatoggle:{src}"), Value::Null));
                            self.hit(next, Press::Action(format!("medianext:{src}"), Value::Null));
                        }
                    }
                    self.hit_tip(r, Press::Action(s(it, "action").to_string(), Value::Null), it["title"].clone());
                    self.last_hit(false, it["menu"].clone());
                }
                let mut h = if tiles.is_empty() { 0.0 } else { 3.0 + rows as f32 * th + (rows - 1) as f32 * TILE_GAP + 3.0 };
                // its pinned ones, closed: rows of icons, as many as they need
                if !pins.is_empty() {
                    if !tiles.is_empty() {
                        h += 10.0;
                    }
                    let per = (((w - 4.0 + 10.0) / 40.0).floor() as usize).max(1);
                    let prow = pins.len().div_ceil(per);
                    for (i, p) in pins.iter().enumerate() {
                        let r = Rect::new(x + 4.0 + (i % per) as f32 * 40.0, y + h + (i / per) as f32 * 40.0, 30.0, 30.0);
                        self.ghost(Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0), 9.0);
                        if self.draw && !self.p.picture(s(p, "image"), Rect::new(r.x + 3.0, r.y + 3.0, 24.0, 24.0), 6.0) {
                            self.round(Rect::new(r.x + 3.0, r.y + 3.0, 24.0, 24.0), 6.0, GROUND);
                        }
                        self.hit_tip(r, Press::Action(s(p, "action").to_string(), Value::Null), p["name"].clone());
                    }
                    h += 30.0 + (prow - 1) as f32 * 40.0;
                }
                h
            }
            "windows" => {
                // windows side by side, centred, as many rows as they need: each its live picture (else its app's picture), and under
                // it its app's small picture and its title
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let n = items.len();
                if n == 0 {
                    return 0.0;
                }
                // (each picture this wide: the dock's own, else as big as it asks (a peek of an app's windows))
                let pic = b["pic"].as_f64().map_or(WIN_PIC, |v| (v as f32).clamp(80.0, 480.0));
                let win_w = pic + crate::peek::METERS + crate::peek::PROGRESS;
                let cols = (((w + WIN_GAP) / (win_w + WIN_GAP)).floor() as usize).max(1);
                let rows = n.div_ceil(cols);
                let ph = (pic * 0.625).round();
                let row_h = ph + 24.0;
                for (i, it) in items.iter().enumerate() {
                    let (row, col) = (i / cols, i % cols);
                    let in_row = (n - row * cols).min(cols) as f32;
                    let x0 = x + (w - (in_row * win_w + (in_row - 1.0) * WIN_GAP)) / 2.0;
                    let cell = x0 + col as f32 * (win_w + WIN_GAP);
                    // (in from the top by the ring round the one in front, so the ring is never cut off)
                    let by = y + 3.0 + row as f32 * (row_h + WIN_GAP);
                    // (its use on the picture's left, how far its download is on its right)
                    let bx = cell + crate::peek::METERS;
                    let r = Rect::new(bx, by, pic, ph);
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
                    // (a ghost; the window in front: ringed in the theme's colour)
                    if it["front"] == true && self.draw {
                        self.p.round(Rect::new(r.x - 2.5, r.y - 2.5, r.w + 5.0, r.h + 5.0), 12.0, accent());
                        self.p.round(Rect::new(r.x - 0.5, r.y - 0.5, r.w + 1.0, r.h + 1.0), 10.0, FACE);
                    }
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
                rows as f32 * row_h + (rows - 1) as f32 * WIN_GAP + 6.0
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
            // a picture to look at closely: as wide as the panel, as tall as its proportion makes it (within `max_h`); the wheel zooms
            // it at the mouse, dragging moves it, a double-click fits it again; how near shows a moment after it changes. Its buttons
            // (`buttons`) over its corner, on dark grounds
            "view" => {
                let img = s(b, "image");
                let id = if s(b, "id").is_empty() { img } else { s(b, "id") }.to_string();
                let max_h = if f(b, "max_h") > 0.0 { f(b, "max_h") } else { 420.0 };
                let size = self.p.picture_size(img);
                // (the panel made taller than its content by hand: the picture takes the rest)
                let h = size.map_or(200.0, |(iw, ih)| (w * ih / iw).clamp(120.0, max_h)) + FILL.with(|f| f.replace(0.0));
                let r = Rect::new(x, y, w, h);
                if let Some((iw, ih)) = size {
                    VIEWS.with(|v| v.borrow_mut().insert(id.clone(), (r, iw, ih)));
                }
                let l = size.map_or(LOOK_WHOLE, |(iw, ih)| keep_in(look_of(&id), r, iw, ih));
                if self.draw {
                    self.p.round(r, 14.0, GROUND);
                    self.p.picture_view(img, r, 14.0, l.zoom, l.ox, l.oy);
                    // (how near, a moment after it changed)
                    let since = l.at.map_or(9.0, |t| t.elapsed().as_secs_f32());
                    if since < 1.4 && l.zoom > 1.001 {
                        let a = ((1.4 - since) / 0.3).min(1.0);
                        let t = format!("{}%", (l.zoom * 100.0).round() as i32);
                        let tw = self.p.measure(&t, 11.0, Weight::Semibold, 80.0);
                        self.p.round(Rect::new(x + 8.0, y + h - 30.0, tw + 16.0, 22.0), 11.0, crate::paint::Color(0.0, 0.0, 0.0, 0.66 * a));
                        self.p.text(&t, x + 16.0, y + h - 19.0, 11.0, Weight::Semibold, 80.0, DIM.alpha(a));
                    }
                }
                // (its pictures to choose from, over its foot, on a dark ground: only while the mouse is on the panel)
                let album = &b["album"];
                if album.is_object() && HOVER.with(|h| h.get()).is_some() {
                    let n = album["items"].as_array().map_or(0, |a| a.len()) as f32;
                    let add = if album["add"].is_object() { 36.0 } else { 0.0 };
                    let aw = (n * 50.0 - 6.0 + add + 4.0).max(34.0).min(w - 16.0);
                    let ar = Rect::new(x + w / 2.0 - aw / 2.0 - 6.0, y + h - 54.0, aw + 12.0, 46.0);
                    if self.draw {
                        self.p.round(ar, 14.0, crate::paint::Color(0.0, 0.0, 0.0, 0.72));
                    }
                    self.album(album, ar.x + 6.0, ar.y + 6.0, aw);
                }
                let mut bx = x + w - 8.0;
                for bt in b["buttons"].as_array().cloned().unwrap_or_default().iter().rev() {
                    bx -= 30.0;
                    let br = Rect::new(bx, y + 8.0, 30.0, 30.0);
                    if self.draw {
                        self.p.circle(br.x + 15.0, br.y + 15.0, 15.0, if hovered(br) { crate::paint::Color(0.15, 0.15, 0.17, 0.92) } else { crate::paint::Color(0.0, 0.0, 0.0, 0.66) });
                        self.icon(s(bt, "icon"), br.x + 7.0, br.y + 7.0, 16.0, if bt["on"] == true { accent() } else { TEXT });
                    }
                    self.hit_tip(br, Press::Action(s(bt, "action").to_string(), bt["args"].clone()), bt["tip"].clone());
                    bx -= 4.0;
                }
                self.hit(r, Press::View(id));
                h
            }
            // pictures to choose from: a row of small ones, the one shown ringed in the theme's colour (each pressed: its action; its
            // right-click: its `menu`); at its end, a button (`add`)
            "album" => self.album(b, x, y, w),
            // a space to press, dashed round, a pictogram in its middle and a line under it
            "drop" => {
                let h = if f(b, "h") > 0.0 { f(b, "h") } else { 200.0 };
                let r = Rect::new(x, y, w, h);
                if self.draw {
                    if hovered(r) {
                        self.p.round(r, 16.0, LIT);
                    }
                    self.p.dashed(Rect::new(x + 1.0, y + 1.0, w - 2.0, h - 2.0), 16.0, 1.5, crate::paint::Color(0.204, 0.204, 0.227, 1.0));
                    let icon = if s(b, "icon").is_empty() { "clipboard" } else { s(b, "icon") };
                    self.icon(icon, x + w / 2.0 - 20.0, y + h / 2.0 - 30.0, 40.0, DIM);
                    let t = s(b, "text");
                    if !t.is_empty() {
                        let tw = self.p.measure(t, 12.0, Weight::Regular, w - 24.0);
                        self.text(t, x + w / 2.0 - tw / 2.0, y + h / 2.0 + 26.0, 12.0, Weight::Regular, w - 24.0, DIM);
                    }
                }
                self.hit_tip(r, Press::Action(s(b, "action").to_string(), b["args"].clone()), b["tip"].clone());
                h
            }
            // what it is connected to, on its own ground: a big round mark (an icon, or four bars), its name, how (a small icon and
            // words), and lines at its right (how fast)
            "hero" => {
                let h = 68.0;
                let r = Rect::new(x, y, w, h);
                let cy = y + h / 2.0;
                if self.draw {
                    self.p.round(r, 16.0, crate::paint::Color(0.078, 0.082, 0.102, 1.0));
                    let (bg, fg) = match s(b, "tone") {
                        "ok" => (accent().alpha(0.18), accent()),
                        "no" => (RED.alpha(0.18), RED),
                        _ => (GROUND, TEXT),
                    };
                    self.p.circle(x + 34.0, cy, 22.0, bg);
                    if let Some(level) = b["signal"].as_u64() {
                        for i in 0..4u64 {
                            let bh = 6.0 + 4.0 * i as f32;
                            self.p.round(Rect::new(x + 24.0 + i as f32 * 5.5, cy + 9.0 - bh, 3.5, bh), 1.75, if i < level { fg } else { fg.alpha(0.3) });
                        }
                    } else {
                        self.p.icon(s(b, "icon"), x + 24.0, cy - 10.0, 20.0, fg);
                    }
                }
                let lines = b["lines"].as_array().cloned().unwrap_or_default();
                let lw = lines.iter().map(|l| self.p.measure(s(l, "text"), 11.5, Weight::Semibold, 120.0)).fold(0.0f32, f32::max);
                let tx = x + 66.0;
                let tw = (w - 66.0 - 12.0 - if lw > 0.0 { lw + 10.0 } else { 0.0 }).max(40.0);
                self.text(s(b, "title"), tx, cy - 8.0, 14.0, Weight::Semibold, tw, TEXT);
                let mut sx = tx;
                if !s(b, "sub_icon").is_empty() {
                    self.icon(s(b, "sub_icon"), sx, cy + 3.0, 12.0, DIM);
                    sx += 16.0;
                }
                self.text(s(b, "sub"), sx, cy + 9.0, 11.5, Weight::Regular, tw - (sx - tx), DIM);
                for (i, l) in lines.iter().enumerate() {
                    let ly = cy + if lines.len() > 1 { if i == 0 { -8.0 } else { 9.0 } } else { 0.0 };
                    if self.draw {
                        self.p.text_end(s(l, "text"), x + w - 12.0, ly, 11.5, Weight::Semibold, 120.0, color_of(&l["color"], DIM));
                    }
                }
                h
            }
            // switches side by side in a track, each on or off on its own: on, on a lighter ground, its icon in the theme's colour
            "toggles" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let h = 36.0;
                let r = Rect::new(x, y, w, h);
                if self.draw {
                    self.p.round(r, 14.0, TRACK);
                }
                let n = items.len().max(1) as f32;
                let iw = (w - 6.0 - (n - 1.0) * 4.0) / n;
                for (i, it) in items.iter().enumerate() {
                    let ir = Rect::new(x + 3.0 + i as f32 * (iw + 4.0), y + 3.0, iw, h - 6.0);
                    let on = it["on"] == true;
                    if self.draw {
                        if on {
                            self.p.round(ir, 11.0, CHOSEN);
                        } else if hovered(ir) {
                            self.p.round(ir, 11.0, LIT);
                        }
                    }
                    let label = s(it, "label");
                    let lw = self.p.measure(label, 11.5, Weight::Semibold, iw - 30.0);
                    let cx = ir.x + ir.w / 2.0 - (lw + 22.0) / 2.0;
                    self.icon(s(it, "icon"), cx, ir.y + ir.h / 2.0 - 8.0, 16.0, if on { accent() } else { DIM });
                    self.text(label, cx + 22.0, ir.y + ir.h / 2.0, 11.5, Weight::Semibold, iw - 30.0, if on { TEXT } else { DIM });
                    let tip = if on { format!("Turn Off {label}") } else { format!("Turn On {label}") };
                    self.hit_tip(ir, Press::Action(s(it, "action").to_string(), Value::Null), Value::String(tip));
                }
                h
            }
            // the desktops, a card each: its minimap (its monitors with its wallpaper, its windows where they are, each with its app's
            // icon, the one in front ringed), its name and how many windows; the one shown ringed. A card pressed: its desktop shown;
            // a window on it pressed: shown, and the window brought to the front; its x (under the mouse): closed. A dashed one adds one
            "deskmaps" => {
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let screens = b["screens"].as_array().cloned().unwrap_or_default();
                let aspect = (b["aspect"].as_f64().unwrap_or(16.0 / 9.0) as f32).clamp(1.0, 4.0);
                // (in from its edges by the ring round the one shown, so the ring is never cut off)
                let (x, y, w) = (x + 3.0, y + 1.0, w - 6.0);
                let gap = 14.0;
                let cw = (w - gap) / 2.0;
                let mh = (cw / aspect).clamp(60.0, 120.0);
                let ch = mh + 22.0;
                let n = items.len() + 1;
                for i in 0..n {
                    let (row, col) = (i / 2, i % 2);
                    let mr = Rect::new(x + col as f32 * (cw + gap), y + row as f32 * (ch + gap) + 2.0, cw, mh);
                    if i == items.len() {
                        if self.draw {
                            if hovered(mr) {
                                self.p.round(mr, 10.0, LIT);
                            }
                            self.p.dashed(Rect::new(mr.x + 1.0, mr.y + 1.0, mr.w - 2.0, mr.h - 2.0), 10.0, 1.5, crate::paint::Color(0.204, 0.204, 0.227, 1.0));
                        }
                        self.icon("plus", mr.x + mr.w / 2.0 - 9.0, mr.y + mr.h / 2.0 - 9.0, 18.0, DIM);
                        self.hit_tip(mr, Press::Action(s(b, "add").to_string(), Value::Null), Value::String("New Desktop".into()));
                        continue;
                    }
                    let it = &items[i];
                    let on = it["on"] == true;
                    let over = hovered(mr);
                    if self.draw {
                        if on {
                            self.p.round(Rect::new(mr.x - 2.0, mr.y - 2.0, mr.w + 4.0, mr.h + 4.0), 12.0, accent());
                        }
                        crate::peek::draw_map(self.p, it, &screens, mr, 10.0, on, true);
                        if over && !on {
                            self.p.round(mr, 10.0, crate::paint::Color(1.0, 1.0, 1.0, 0.06));
                        }
                    }
                    // (its x, under the mouse: closed)
                    let close = s(it, "close");
                    if over && !close.is_empty() {
                        let xr = Rect::new(mr.right() - 24.0, mr.y + 4.0, 20.0, 20.0);
                        if self.draw {
                            self.p.circle(xr.x + 10.0, xr.y + 10.0, 10.0, crate::paint::Color(0.0, 0.0, 0.0, 0.66));
                        }
                        self.icon("x", xr.x + 5.0, xr.y + 5.0, 10.0, TEXT);
                        self.hit_tip(xr, Press::Action(close.to_string(), Value::Null), Value::String("Close Desktop".into()));
                    }
                    // (each window on it: pressed, it comes to the front there; the top one first)
                    let wins = it["wins"].as_array().cloned().unwrap_or_default();
                    for wv in wins.iter().rev() {
                        let f = |k: usize| wv["r"][k].as_f64().unwrap_or(0.0) as f32;
                        let wr = Rect::new(mr.x + f(0) * mr.w, mr.y + f(1) * mr.h, f(2) * mr.w, f(3) * mr.h);
                        if wr.w > 6.0 && wr.h > 6.0 {
                            self.hit(wr, Press::Action(s(wv, "action").to_string(), Value::Null));
                        }
                    }
                    self.hit(mr, Press::Action(s(it, "action").to_string(), Value::Null));
                    let count = it["count"].as_u64().unwrap_or(0);
                    let ny = mr.bottom() + 12.0;
                    let cw2 = if count > 0 { self.p.measure(&count.to_string(), 11.0, Weight::Semibold, 40.0) + 4.0 } else { 0.0 };
                    self.text(s(it, "name"), mr.x + 2.0, ny, 11.5, Weight::Semibold, mr.w - cw2 - 4.0, if on { TEXT } else { DIM });
                    if count > 0 && self.draw {
                        self.p.text_end(&count.to_string(), mr.right() - 2.0, ny, 11.0, Weight::Semibold, 40.0, DIM);
                    }
                }
                let rows = n.div_ceil(2) as f32;
                rows * ch + (rows - 1.0) * gap + 4.0
            }
            // devices, a card each: its pictogram in a ring as full as its battery (amber when low), its name, and under it how full
            "devcards" => {
                let mut h = 0.0;
                for it in b["items"].as_array().cloned().unwrap_or_default() {
                    let r = Rect::new(x, y + h, w, 56.0);
                    let cy = r.y + 28.0;
                    let low = it["low"] == true;
                    if self.draw {
                        self.p.round(r, 14.0, crate::paint::Color(0.078, 0.082, 0.102, 1.0));
                        if let Some(l) = it["level"].as_f64() {
                            self.p.ring(x + 32.0, cy, 17.0, 3.0, l as f32, 0.0, if low { AMBER } else { accent() }, FAINT);
                        }
                        self.p.icon(s(&it, "icon"), x + 24.0, cy - 8.0, 16.0, TEXT);
                    }
                    self.text(s(&it, "name"), x + 62.0, cy - 7.0, 13.0, Weight::Semibold, w - 74.0, TEXT);
                    self.text(s(&it, "sub"), x + 62.0, cy + 9.0, 11.5, Weight::Regular, w - 74.0, if low { AMBER } else { DIM });
                    h += 56.0 + 8.0;
                }
                (h - 8.0).max(0.0)
            }
            "deskmap" => self.deskmap(b, x, y, w),
            // a thin line across, between groups
            "row" => {
                // a row of small things, left to right, one line tall; each `{ kind = ... }`:
                // `button` (an icon, a ghost: `icon`, `tip`, `on` (its icon in the theme colour), `dot` (a small amber dot on its corner),
                // `tone` ("no": red), `action` and `args`, or `menu` (pressed: that menu, its rows as a thing's own menu's)); `switch`
                // (`on`, `action`, `args`, `tip`); `pill` (`text`, `image` or `icon`, `dot`, `action` and `args` or `menu`: a chevron after
                // it); `seg` (`options = { { text | icon, value, tip } }`, `value`, `action`: sent `{ value }`); `num` (`text`, `unit`,
                // `action`: pressed, typed, sent `{ text }`); `slider` (a slider block's fields: it takes what room is left, as a space does);
                // `label` (`text`: words as a heading's); `space` (what room is left, shared by every space and slider)
                let items = b["items"].as_array().cloned().unwrap_or_default();
                let h = 34.0;
                let cy = y + h / 2.0;
                const SEG_PAD: f32 = 3.0;
                let width = |it: &Value| -> f32 {
                    match s(it, "kind") {
                        "button" => 32.0,
                        "switch" => 42.0,
                        "pill" => {
                            let pic = if !s(it, "image").is_empty() || !s(it, "icon").is_empty() { 24.0 } else { 0.0 };
                            let dot = if it["dot"] == true { 13.0 } else { 0.0 };
                            let chev = if it["menu"].is_array() { 18.0 } else { 0.0 };
                            12.0 + pic + self.p.measure(s(it, "text"), 12.5, Weight::Semibold, 220.0) + dot + chev + 12.0
                        }
                        "seg" => {
                            let opts = it["options"].as_array().cloned().unwrap_or_default();
                            2.0 * SEG_PAD + opts.iter().map(|o| if !s(o, "icon").is_empty() { 32.0 } else { self.p.measure(s(o, "text"), 11.5, Weight::Semibold, 120.0) + 22.0 }).sum::<f32>() + 2.0 * (opts.len().max(1) - 1) as f32
                        }
                        "num" => 20.0 + self.p.measure(s(it, "text"), 12.5, Weight::Semibold, 100.0) + if s(it, "unit").is_empty() { 0.0 } else { 4.0 + self.p.measure(s(it, "unit"), 10.5, Weight::Regular, 40.0) },
                        "label" => self.p.measure(s(it, "text"), 10.5, Weight::Semibold, 240.0),
                        _ => 0.0,
                    }
                };
                let gap = 4.0;
                let fixed: f32 = items.iter().map(width).sum::<f32>() + gap * (items.len().max(1) - 1) as f32;
                let flexible = |it: &Value| matches!(s(it, "kind"), "space" | "slider");
                let spaces = items.iter().filter(|i| flexible(i)).count().max(1) as f32;
                let spare = ((w - fixed) / spaces).max(0.0);
                let mut cx = x;
                for it in &items {
                    let kind = s(it, "kind");
                    let iw = if flexible(it) { spare } else { width(it) };
                    let action = s(it, "action").to_string();
                    let tip = if s(it, "tip").is_empty() { Value::Null } else { Value::String(s(it, "tip").to_string()) };
                    let press = |action: &str, args: &Value| if it["menu"].is_array() { Press::Menu(it["menu"].clone()) } else { Press::Action(action.to_string(), args.clone()) };
                    match kind {
                        "button" => {
                            let br = Rect::new(cx, cy - 16.0, 32.0, 32.0);
                            if self.draw {
                                if hovered(br) {
                                    self.p.circle(cx + 16.0, cy, 16.0, LIT);
                                }
                                let c = if s(it, "tone") == "no" { RED } else if it["on"] == true { accent() } else { TEXT };
                                self.icon(s(it, "icon"), cx + 8.0, cy - 8.0, 16.0, c);
                                if it["dot"] == true {
                                    self.p.circle(cx + 25.0, cy - 9.0, 4.6, FACE);
                                    self.p.circle(cx + 25.0, cy - 9.0, 3.4, AMBER);
                                }
                            }
                            if !action.is_empty() || it["menu"].is_array() {
                                self.hit_tip(br, press(&action, &it["args"]), tip);
                            }
                        }
                        "switch" => {
                            let on = it["on"] == true;
                            let tr = Rect::new(cx + 1.0, cy - 12.0, 40.0, 24.0);
                            if self.draw {
                                self.p.round(tr, 12.0, if on { accent() } else { SLIDER_TRACK });
                                let kx = if on { tr.right() - 12.0 } else { tr.x + 12.0 };
                                let kc = if on { crate::paint::Color(1.0, 1.0, 1.0, 1.0) } else if hovered(tr) { TEXT } else { DIM };
                                self.p.circle(kx, cy, 9.0, kc);
                            }
                            if !action.is_empty() {
                                self.hit_tip(tr, Press::Action(action.clone(), it["args"].clone()), tip);
                            }
                        }
                        "pill" => {
                            let pr = Rect::new(cx, cy - 16.0, iw, 32.0);
                            self.round(pr, 16.0, if hovered(pr) { LIT } else { TRACK });
                            let mut tx = cx + 12.0;
                            let image = s(it, "image");
                            if !image.is_empty() {
                                if self.draw && !self.p.picture(image, Rect::new(tx, cy - 8.0, 16.0, 16.0), 4.0) {
                                    self.icon("dot", tx, cy - 8.0, 16.0, DIM);
                                }
                                tx += 24.0;
                            } else if !s(it, "icon").is_empty() {
                                self.icon(s(it, "icon"), tx, cy - 8.0, 16.0, accent());
                                tx += 24.0;
                            }
                            let tw = self.text(s(it, "text"), tx, cy, 12.5, Weight::Semibold, 220.0, TEXT);
                            tx += tw;
                            if it["dot"] == true {
                                if self.draw {
                                    self.p.circle(tx + 9.0, cy, 3.5, AMBER);
                                }
                                tx += 13.0;
                            }
                            if it["menu"].is_array() {
                                self.icon("chev-down", tx + 4.0, cy - 6.0, 12.0, DIM);
                            }
                            if !action.is_empty() || it["menu"].is_array() {
                                self.hit_tip(pr, press(&action, &it["args"]), tip);
                            }
                        }
                        "seg" => {
                            let tr = Rect::new(cx, cy - 15.0, iw, 30.0);
                            self.round(tr, 10.0, TRACK);
                            let mut ox = cx + SEG_PAD;
                            for o in it["options"].as_array().cloned().unwrap_or_default() {
                                let icon = s(&o, "icon");
                                let ow = if !icon.is_empty() { 32.0 } else { self.p.measure(s(&o, "text"), 11.5, Weight::Semibold, 120.0) + 22.0 };
                                let orr = Rect::new(ox, cy - 12.0, ow, 24.0);
                                let on = o["value"] == it["value"];
                                if on {
                                    self.round(orr, 7.5, CHOSEN);
                                } else if hovered(orr) {
                                    self.round(orr, 7.5, LIT);
                                }
                                if !icon.is_empty() {
                                    self.icon(icon, ox + 7.0, cy - 9.0, 18.0, if on { accent() } else { DIM });
                                } else {
                                    let t = s(&o, "text");
                                    let tw = self.p.measure(t, 11.5, Weight::Semibold, 120.0);
                                    self.text(t, ox + (ow - tw) / 2.0, cy, 11.5, if on { Weight::Semibold } else { Weight::Regular }, 120.0, if on { TEXT } else { DIM });
                                }
                                if !action.is_empty() {
                                    let otip = if s(&o, "tip").is_empty() { Value::Null } else { Value::String(s(&o, "tip").to_string()) };
                                    self.hit_tip(orr, Press::Action(action.clone(), json!({ "value": o["value"] })), otip);
                                }
                                ox += ow + 2.0;
                            }
                        }
                        "slider" => {
                            self.block(it, cx, cy - 16.0, iw);
                        }
                        "label" => {
                            self.text(s(it, "text"), cx, cy, 10.5, Weight::Semibold, iw, DIM);
                        }
                        "num" => {
                            let nr = Rect::new(cx, cy - 14.0, iw, 28.0);
                            self.round(nr, 9.0, if hovered(nr) { LIT } else { TRACK });
                            let t = s(it, "text");
                            let tw = self.text(t, cx + 10.0, cy, 12.5, Weight::Semibold, 100.0, TEXT);
                            let unit = s(it, "unit");
                            if !unit.is_empty() {
                                self.text(unit, cx + 10.0 + tw + 4.0, cy + 0.5, 10.5, Weight::Regular, 40.0, DIM);
                            }
                            if !action.is_empty() {
                                self.hit_tip(nr, Press::Field(action.clone(), false, unit.to_string(), t.to_string(), false, Value::Null), tip);
                            }
                        }
                        _ => {}
                    }
                    cx += iw + gap;
                }
                h
            }
            "rule" => {
                if self.draw {
                    self.p.round(Rect::new(x, y + 5.0, w, 1.0), 0.5, crate::paint::Color(0.118, 0.118, 0.137, 1.0));
                }
                11.0
            }
            "slider" => {
                // a thin track with a white knob, pressed or dragged where it should be (the knob follows the mouse at once); under the
                // mouse it thickens, held it rings; its icon a ghost button of its own, left of it (pressed: muted); its value at the
                // right. Muted: the level kept, dim, the icon red
                // (named (an app's): its name small over it, and its three bars while it plays)
                let name = s(b, "name");
                let top = if name.is_empty() { 0.0 } else { 14.0 };
                let mtr = s(b, "meter");
                if !name.is_empty() && self.draw {
                    let has = !s(b, "icon").is_empty() || !s(b, "image").is_empty();
                    let nx = if has { x + 40.0 } else { x };
                    let nw = self.p.measure(name, 11.0, Weight::Semibold, w - 120.0);
                    self.text(name, nx, y + 7.0, 11.0, Weight::Semibold, w - 120.0, DIM);
                    if b["playing"] == true {
                        let (parts, lift) = loud_bars(meter(mtr), self.p.t);
                        self.p.bars(nx + nw + 6.0, y + 7.0, 9.0, parts, lift, accent());
                    }
                }
                let y = y + top;
                let h = 32.0;
                let cy = y + h / 2.0;
                let action = s(b, "action");
                let given = f(b, "value").clamp(0.0, 1.0);
                note_slider(action, given);
                NO_WHEEL.with(|s| {
                    let mut s = s.borrow_mut();
                    if b["wheel"] == false {
                        s.insert(action.to_string());
                    } else {
                        s.remove(action);
                    }
                });
                let held = sliding(action);
                // (held, with steps of its own (`range.step`): the knob goes from step to step, as its value does)
                let held = held.map(|h| {
                    let r = &b["range"];
                    let (lo, hi) = (f(r, "min"), f(r, "max"));
                    match r["step"].as_f64().map(|x| x as f32).filter(|x| *x > 0.0 && hi > lo) {
                        Some(step) => {
                            let val = (lo + (hi - lo) * h.clamp(0.0, 1.0)) / step;
                            ((val.round() * step - lo) / (hi - lo)).clamp(0.0, 1.0)
                        }
                        None => h,
                    }
                });
                let v = held.unwrap_or(given).clamp(0.0, 1.0);
                let icon = s(b, "icon");
                let image = s(b, "image");
                let muted = b["muted"] == true;
                let has_icon = !icon.is_empty() || !image.is_empty();
                let left = if has_icon { x + 40.0 } else { x };
                // (a button after it, its own: `after = { icon, on, action, tip }`; the rest of it that much narrower)
                let after = b["after"].clone();
                let w = if after.is_object() {
                    let br = Rect::new(x + w - 32.0, y, 32.0, 32.0);
                    if self.draw {
                        if hovered(br) {
                            self.p.circle(br.x + 16.0, br.y + 16.0, 16.0, LIT);
                        }
                        self.icon(s(&after, "icon"), br.x + 8.0, br.y + 8.0, 16.0, if after["on"] == true { accent() } else { TEXT });
                    }
                    let tip = if s(&after, "tip").is_empty() { Value::Null } else { Value::String(s(&after, "tip").to_string()) };
                    self.hit_tip(br, Press::Action(s(&after, "action").to_string(), after["args"].clone()), tip);
                    w - 40.0
                } else {
                    w
                };
                let vw = 36.0;
                let hit = Rect::new(left, y, (x + w - vw - 6.0 - left).max(20.0), h);
                let track = Rect::new(hit.x + SEEK_INSET, cy, (hit.w - 2.0 * SEEK_INSET).max(4.0), 0.0);
                let over = hovered(hit) && held.is_none();
                if self.draw {
                    let th = if held.is_some() || over { 6.0 } else { 4.0 };
                    self.p.round(Rect::new(track.x, cy - th / 2.0, track.w, th), th / 2.0, SLIDER_TRACK);
                    let fill = if muted { SLIDER_DIM } else { accent() };
                    let kx = track.x + track.w * v;
                    if v > 0.0 {
                        self.p.round(Rect::new(track.x, cy - th / 2.0, kx - track.x, th), th / 2.0, fill);
                    }
                    // (how loud it is now: a faint lighter line over its fill, never past its knob)
                    if !mtr.is_empty() && !muted {
                        let lv = (live_level(meter(mtr)) * v).clamp(0.0, v);
                        if lv > 0.005 {
                            self.p.round(Rect::new(track.x, cy - th / 2.0, (track.w * lv).max(th), th), th / 2.0, LIVE);
                        }
                    }
                    let kr = if held.is_some() { 9.0 } else if over { 8.0 } else { 7.0 };
                    if held.is_some() {
                        self.p.circle(kx, cy, kr + 5.0, accent().alpha(0.22));
                    }
                    self.p.circle(kx, cy + 1.0, kr + 0.5, crate::paint::Color(0.0, 0.0, 0.0, 0.35));
                    self.p.circle(kx, cy, kr, if muted { DIM } else { crate::paint::Color(1.0, 1.0, 1.0, 1.0) });
                }
                if has_icon {
                    let ib = Rect::new(x, y, 32.0, 32.0);
                    let press = s(b, "press");
                    if self.draw {
                        if !press.is_empty() && hovered(ib) {
                            self.p.circle(x + 16.0, cy, 16.0, LIT);
                        }
                        let c = if muted { RED } else { TEXT };
                        if !image.is_empty() {
                            if !self.p.picture(image, Rect::new(x + 7.0, cy - 9.0, 18.0, 18.0), 4.0) {
                                self.icon("music", x + 8.0, cy - 8.0, 16.0, c);
                            }
                        } else {
                            self.icon(icon, x + 8.0, cy - 8.0, 16.0, c);
                        }
                        // (muted: a line through it, but where its icon says so itself)
                        if muted && icon != "volume-x" {
                            self.p.round(Rect::new(x + 5.0, cy - 1.0, 22.0, 2.0), 1.0, RED);
                        }
                    }
                    if !press.is_empty() {
                        self.hit_tip(ib, Press::Action(press.to_string(), Value::Null), Value::String(if muted { "Unmute" } else { "Mute" }.into()));
                    }
                }
                // (held: where it is as it goes, in its own units when it says what they are (`range = { min, max, unit, sign }`: "-9.5 dB"),
                // else a percentage)
                let t = match (held, b["range"].is_object()) {
                    (Some(_), true) => {
                        let r = &b["range"];
                        let val = f(r, "min") + (f(r, "max") - f(r, "min")) * v;
                        let step = r["step"].as_f64().map(|x| x as f32).filter(|x| *x > 0.0).unwrap_or(0.1);
                        let n = ((val / step).round() * step * 100.0).round() / 100.0;
                        // (as fine as its steps: 0.25 dB steps, two decimals; whole ones, none)
                        let num = if n.fract() == 0.0 { format!("{n:.0}") } else if ((n * 10.0).round() - n * 10.0).abs() < 0.001 { format!("{n:.1}") } else { format!("{n:.2}") };
                        let sign = if r["sign"] == true && n > 0.0 { "+" } else { "" };
                        let unit = s(r, "unit");
                        if unit.is_empty() { format!("{sign}{num}") } else { format!("{sign}{num} {unit}") }
                    }
                    (Some(_), false) => format!("{}%", (v * 100.0).round() as i32),
                    _ => s(b, "text").to_string(),
                };
                if self.draw {
                    // (muted: a speaker struck out, red, where its value was)
                    if muted && held.is_none() {
                        self.icon("volume-x", x + w - 16.0, cy - 8.0, 16.0, RED);
                    } else {
                        let c = if held.is_some() || over { TEXT } else { DIM };
                        self.p.text_end(&t, x + w, cy, 11.5, Weight::Semibold, vw + 20.0, c);
                    }
                }
                self.hit(hit, Press::Seek(action.to_string()));
                top + h + 2.0
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
