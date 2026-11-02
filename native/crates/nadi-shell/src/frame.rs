//! The frame: the widgets round the edges of the screens, merged into them. Each widget is a piece (a visual with its own surface); at
//! rest it shows its peek, and held under the mouse (or pressed) it opens into its panel, growing out of its edge. Banners drop from the
//! top of the main screen. What the plugins say comes in as updates; what the person does goes out as commands.

use crate::blocks::{self, Ctx, Hit, Press};
use crate::config::{self, Config};
use crate::draw::{self, Painter, FACE};
use crate::gfx::Gfx;
use crate::layout::{self, Item as LayoutItem, Metrics, Place, Slot};
use crate::overlay::{self, Overlay, Piece};
use crate::paint::{fill_shape, Weight};
use crate::peek;
use crate::shape::{self, Corner, Edge, Rect, Shape};
use crate::viz::Viz;
use nadi_core::host::{Command, PluginInfo, Update};
use nadi_core::pc::Pc;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

mod edit;
mod pop;
mod size;
use pop::{is_pop, pop_key, unpop, PopMarks, PopMeta, POP_TOP};

const PANEL_W: f32 = 360.0;
const PANEL_W_SIDE: f32 = 340.0;
const PAD: f32 = 16.0;
const RADIUS_OPEN: f32 = 24.0;
/// how deep a widget's stump is while its panel is popped out as a window
const STUMP: f32 = 16.0;
/// the room over an open panel's content for its drag handle (shown while the mouse is on it; pulled, the panel comes off as a window)
const HANDLE: f32 = 10.0;
const HOVER_OPEN: Duration = Duration::from_millis(260);
const LEAVE_CLOSE: Duration = Duration::from_millis(380);
const BANNER_W: f32 = 380.0;
/// how far a floating piece stands off its edge
pub const FLOAT_OFF: f32 = 8.0;
/// a press must be held this long, without moving more than `DRAG_START`, to pick its widget up
const HOLD: Duration = Duration::from_millis(450);
const DRAG_START: f32 = 6.0;
/// let go this near an edge, a widget squishes into it (farther, it floats)
const SQUISH: f32 = 36.0;
/// this near two edges at once, it is a corner's
const CORNER_ZONE: f32 = 90.0;

/// A value that springs toward where it is going: a mass on a spring, damped a little short of still, so it overshoots once and settles.
#[derive(Clone, Copy, Debug, Default)]
struct Spring {
    now: f32,
    to: f32,
    vel: f32,
}

/// how stiff, and how damped (a damping ratio of about 0.65: one small overshoot)
const STIFF: f32 = 600.0;
const DAMP: f32 = 36.0;
/// a widget coming out of its edge, or going back: bouncier (it comes out a little too far, and settles)
const SHOW_STIFF: f32 = 420.0;
const SHOW_DAMP: f32 = 42.0;
/// going back: no bounce, a little quicker
const HIDE_STIFF: f32 = 520.0;
const HIDE_DAMP: f32 = 46.0;
/// a widget swelling under the mouse: quick, one soft overshoot
const SWELL_STIFF: f32 = 700.0;
const SWELL_DAMP: f32 = 34.0;
/// the longest a widget waits for its turn in the wave (seconds), the farthest from the mouse
const WAVE: f32 = 0.22;

impl Spring {
    fn at(v: f32) -> Spring {
        Spring { now: v, to: v, vel: 0.0 }
    }
    fn step(&mut self, dt: f32) -> bool {
        self.step_with(dt, STIFF, DAMP)
    }
    /// A step with a spring of its own: softer, and less held back, it overshoots and bounces.
    fn step_with(&mut self, dt: f32, stiff: f32, damp: f32) -> bool {
        if !self.moving() {
            return false;
        }
        // (small steps: steady at any frame rate)
        let n = (dt / (1.0 / 240.0)).ceil().max(1.0) as usize;
        let h = dt / n as f32;
        for _ in 0..n {
            let a = -stiff * (self.now - self.to) - damp * self.vel;
            self.vel += a * h;
            self.now += self.vel * h;
        }
        let scale = self.to.abs().max(1.0);
        if (self.now - self.to).abs() < 0.0015 * scale && self.vel.abs() < 0.02 * scale {
            self.now = self.to;
            self.vel = 0.0;
            return false;
        }
        true
    }
    fn moving(&self) -> bool {
        self.now != self.to || self.vel != 0.0
    }
}

/// Where a held widget would go if it were let go now: a shadow of it there, under the others.
struct Ghost {
    si: usize,
    piece: Piece,
    rest: SpringBox,
    /// what it was last drawn as
    drawn: Option<(Bx, Place, bool)>,
}

/// A tip out of something the mouse rests on: words, or blocks like a panel's; it springs out of the side away from its edge.
struct Tip {
    si: usize,
    /// what it is the tip of (a widget's key, or a thing in a panel)
    owner: String,
    content: Value,
    /// what it comes out of (logical, in its screen), and the way it comes out (`Edge::Bottom`: under it)
    anchor: Bx,
    out: Edge,
    piece: Piece,
    grow: Spring,
    drawn: Option<(Value, Bx)>,
    /// the live pictures of windows in it (the window, its picture), once it is out
    thumbs: Vec<(isize, isize)>,
    /// something in it moves on its own (a spinning ring, chevrons): drawn again every frame
    moving: bool,
}

/// A tip's live pictures gone.
fn drop_tip_thumbs(t: &mut Tip) {
    for (_, th) in t.thumbs.drain(..) {
        crate::thumbs::unregister(th);
    }
}

/// how long the mouse rests on something before its tip comes, and before the next one comes while one is out
const TIP_WAIT: Duration = Duration::from_millis(450);
const TIP_NEXT: Duration = Duration::from_millis(90);
const TIP_W: f32 = 260.0;

/// Something held until its ring fills (it cannot be undone): a thing in a widget's panel, or a row of a menu.
/// A slider or a bar to seek on, being dragged: its widget, its action, its press area (in its panel), where its knob is, and when
/// its plugin was last told (a slider is told as it moves, a bar to seek on when let go).
struct Slide {
    si: usize,
    key: String,
    plugin: String,
    action: String,
    rect: Rect,
    frac: f32,
    told: Instant,
    told_frac: f32,
    live: bool,
}

/// how often a dragged slider tells its plugin
const SLIDE_EVERY: Duration = Duration::from_millis(40);

struct Hold {
    si: usize,
    /// the widget it is in (none: the menu's row `row`)
    key: Option<String>,
    row: Option<usize>,
    /// where it is (logical, in its widget's piece; for a row, in the menu)
    rect: Rect,
    plugin: String,
    action: String,
    /// what is asked before it is done
    spec: Value,
}


/// A menu open on the screen: a widget's, or the short one (no widget), or a thing's own (its rows, from its plugin).
struct Menu {
    si: usize,
    key: Option<String>,
    /// a thing's own menu: its plugin, and its rows
    items: Option<(String, Vec<Value>)>,
    piece: Piece,
    /// where it is (logical, in its screen)
    at: Bx,
    /// the point it was asked at (the short one goes by it)
    point: (f32, f32),
    hits: Vec<(Rect, crate::menu::Act)>,
    /// each one's name, as its tip says it (a widget's menu)
    names: Vec<&'static str>,
    hover: Option<usize>,
    /// words in it too long for their room, scrolling: drawn again each frame
    marquee: bool,
    /// how far it is out (springing: it pops out of the side toward its widget, and back in as it goes)
    grow: Spring,
    /// going (it shrinks away, then it is gone)
    closing: bool,
    /// the point it grows from (logical, in it)
    pivot: (f32, f32),
}

/// Where a held widget would go if let go now: its place, floating there or not, and in a corner, whether it is the corner's own
/// (`head`) or one of its column (`down`) or its row.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    place: Place,
    float: bool,
    down: bool,
    head: bool,
}

/// A box whose every side springs (a widget sliding to its place, growing or shrinking as its peek changes).
#[derive(Clone, Copy, Debug, Default)]
struct SpringBox {
    x: Spring,
    y: Spring,
    w: Spring,
    h: Spring,
}

impl SpringBox {
    fn now(&self) -> Bx {
        Bx { x: self.x.now, y: self.y.now, w: self.w.now, h: self.h.now }
    }
    fn set(&mut self, b: Bx) {
        self.x.to = b.x;
        self.y.to = b.y;
        self.w.to = b.w;
        self.h.to = b.h;
    }
    fn step(&mut self, dt: f32) -> bool {
        let a = self.x.step(dt);
        let b = self.y.step(dt);
        let c = self.w.step(dt);
        let d = self.h.step(dt);
        a || b || c || d
    }
    fn moving(&self) -> bool {
        self.x.moving() || self.y.moving() || self.w.moving() || self.h.moving()
    }
}

struct Widget {
    plugin: String,
    id: String,
    content: Value,
    /// when its glow last changed (a flash fades from then)
    glow_at: Instant,
}

/// How far a glow reaches out of its piece (logical px).
const GLOW: f32 = 18.0;
/// how much a widget swells with the mouse on it: out of its edge, and along it (logical px)
const SWELL_OUT: f32 = 5.0;
const SWELL_ALONG: f32 = 6.0;

/// A box, logical px, in the monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bx {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bx {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A widget on a screen.
struct Item {
    piece: Piece,
    /// its glow, on the glass under the frame's window (made when it glows)
    glow: Option<Piece>,
    slot: Slot,
    open: Spring,
    /// where it is at rest, springing to its slot
    rest: SpringBox,
    content_h: f32,
    scroll: f32,
    hits: Vec<Hit>,
    buttons: Vec<(Rect, String)>,
    /// where its panel wants live pictures of windows (window, box: logical, in the piece), and the ones made (window, picture)
    thumb_boxes: Vec<(isize, Rect)>,
    thumbs: Vec<(isize, isize)>,
    sections_open: HashSet<String>,
    sections_closed: HashSet<String>,
    hover_since: Option<Instant>,
    left_at: Option<Instant>,
    /// the mouse on it, shut: it swells a little out of its edge (1), and settles back as the mouse goes (0)
    swell: Spring,
    /// what moves in it (a spinner, the bars of the sound): drawn each frame
    alive: bool,
    /// opened by a press: it stays open until the mouse goes and comes back, or another press
    pinned: bool,
    /// a piece of its own, a little off the edge (else merged into it)
    floating: bool,
    /// how much of the panel shows (its body's height, less the padding)
    visible: f32,
    /// when its panel was last scrolled (its scrollbar shows a moment after, while the mouse is on it)
    scrolled_at: Option<Instant>,
    /// while it is dragged: the edge it would go on, and whether it would float there
    drag_place: Option<Place>,
    drag_float: bool,
    /// held in a corner's column (not its row)
    drag_down: bool,
    /// held in a corner itself (not beside it)
    drag_head: bool,
    /// how far out of its edge it is (1: shown; 0: slid into the edge, hidden), and since when it is not wanted out
    shown: Spring,
    unwanted_since: Option<Instant>,
    /// since when it is wanted out, while it waits its turn in the wave
    wanted_since: Option<Instant>,
}

struct Banner {
    plugin: String,
    say: Value,
    since: Instant,
    ms: u64,
    slide: Spring,
    hits: Vec<(Rect, Press)>,
    leaving: bool,
    h: f32,
    /// where it is now (logical, in the monitor)
    at: Option<Bx>,
    /// where it is going: its widget's place, as the layout moves it (springing there)
    home: Option<SpringBox>,
    /// the screen it was last drawn on
    si: Option<usize>,
    /// a live one's words, still being typed in
    typing: bool,
    /// words of it too long for their room, scrolling
    marquee: bool,
    /// its glow still settling (or its rings going out)
    glowing: bool,
    /// when it came out (its glow's clock: not wound back as the mouse holds it)
    born: Instant,
}

impl Screen {
    /// The banner put away on this screen: its piece and its glow.
    fn hide_banner(&self) {
        for p in [&self.banner_piece, &self.banner_glow].into_iter().flatten() {
            unsafe {
                let _ = p.visual.SetOffsetY2(-1000.0);
            }
        }
    }
}

struct Screen {
    ov: Overlay,
    s: f32,
    w: f32,
    h: f32,
    /// where the frame may go: the monitor less the taskbar (logical, in the monitor)
    area: Bx,
    /// the taskbar under the widgets ("under"): its edge and how deep it is; the widgets on that edge as deep as it (logical)
    bar: Option<(Edge, f32)>,
    primary: bool,
    items: BTreeMap<String, Item>,
    banner_piece: Option<Piece>,
    /// the banner's glow, on the glass under it
    banner_glow: Option<Piece>,
    /// where the banner comes out: the Notifications widget's place, laid out with the banner's size (so the others make room)
    banner_slot: Option<(Place, Bx, bool)>,
    /// when the frame was last put back over a full-screen app that came over it
    raised_at: Option<Instant>,
    /// an app is full screen in front on this monitor (a game, a video)
    full: bool,
    /// the frame's window is shown (over a full-screen app, only while widgets are out)
    win_shown: bool,
    /// a maximized window is the top one on this monitor (focused or not)
    maxed: bool,
    /// the liquid under its widgets (their shapes and its edges, run together), and whether it is to be drawn again
    liquid: Option<Piece>,
    liquid_dirty: bool,
    /// the shapes it was last drawn from (the same again: nothing to draw)
    liquid_shapes: Vec<(Bx, f32)>,
}

pub struct Frame {
    pub g: Gfx,
    pub pc: Arc<Pc>,
    pub tx: Sender<Command>,
    pub msg_hwnd: HWND,
    config: Config,
    config_when: Option<std::time::SystemTime>,
    pub plugins: Vec<PluginInfo>,
    widgets: BTreeMap<String, Widget>,
    screens: Vec<Screen>,
    queue: Vec<(String, Value)>,
    banner: Option<Banner>,
    quiet: HashSet<String>,
    start: Instant,
    last: Instant,
    viz: Viz,
    hover: Option<(usize, String)>,
    minute: String,
    slow_at: Instant,
    /// the left button was down at the last tick (a click outside the pieces is seen as it goes down)
    lbutton: bool,
    /// when the top windows of the monitors were last looked at
    fronts_at: Instant,
    /// where the taskbars were (one that hides itself slides out and away: the frame keeps clear of it)
    bars_at: Vec<RECT>,
    pub timer_ms: u32,
    /// the screen the frame is on, when it follows the mouse
    active: usize,
    /// the left button went down on a widget: (screen, widget, where, when)
    press: Option<(usize, String, f32, f32, Instant)>,
    /// where the mouse is (screen, logical px)
    mouse: (usize, f32, f32),
    /// the tip out, and what the mouse rests on that has one (since when)
    tip: Option<Tip>,
    tip_want: Option<(String, Instant)>,
    /// something held until its ring fills
    hold: Option<Hold>,
    /// a slider (or a bar to seek on) being dragged, and one just let go of (its knob kept where it was let go until its plugin says)
    slide: Option<Slide>,
    slid: Option<(usize, String, Instant)>,
    /// the Volume Mixer's meters as drawn (by name)
    meter_now: HashMap<String, f32>,
    /// a field to open by itself as soon as it is drawn (its action), and since when it is waited for
    focus_field: Option<(String, Instant)>,
    /// the network's bytes in and out when last read, and how fast it moved (down, up; bytes a second)
    net_octets: Option<(Instant, u64, u64)>,
    net_speed: Option<(f64, f64)>,
    /// the microphone's level for Speech's bars, springing
    voice_now: f32,
    /// widgets out for a moment on their own (until when), when each last did, and what each plugin last asked to bump with
    bumps: HashMap<String, Instant>,
    bumped_at: HashMap<String, Instant>,
    bump_asked: HashMap<String, Value>,
    /// a desktop's card pressed on the map (screen, widget, its id, its action, where the mouse went down, when, whether it is carried)
    card_press: Option<(usize, String, String, String, f32, f32, Instant, bool)>,
    /// the glows hidden (one of Windows' see-through panels open)
    glass_hidden: bool,
    /// the widgets glowing, put over the ones beside them
    glow_raised: HashSet<String>,
    /// since when the mouse has been in an edge's reach, while the widgets wait to come (the setting's wait)
    edge_since: std::cell::Cell<Option<Instant>>,
    /// while the edge reach is being set: its band shown on the screen (which screen, the piece, when last moved)
    reach_shown: Option<(usize, Piece, Instant)>,
    /// what NADI costs (the Settings window's Performance page): its samples; all the frame has spent (ms); what each widget's
    /// drawing has cost (times, ms); and whether the page is watching, over how many seconds
    perf: crate::perf::Perf,
    pub busy_ms: f64,
    render_cost: HashMap<String, (u64, f64)>,
    perf_on: bool,
    perf_range: usize,
    /// the Recycle Bin as last read, and whether files are being dragged now (from File Explorer: an empty bin comes out to take
    /// them)
    bin: crate::recycle::State,
    files_dragging: bool,
    /// a picture being dragged (screen, widget, picture, where the mouse was, whether it moved), and the last one let go of unmoved
    /// (a second press soon after fits it again)
    pan: Option<(usize, String, String, f32, f32, bool)>,
    view_click: Option<(String, Instant)>,
    /// the popped-out windows over a full-screen app, kept on the glass over the frame's window (which the app may come over)
    lifted: HashSet<String>,
    /// a panel being resized; the panel and sides the mouse is near; the last press on a side (twice in a moment: its own size again)
    sizing: Option<size::Sizing>,
    size_hover: Option<(usize, String, u8)>,
    size_click: Option<(String, Instant)>,
    /// what is drawn over all while resizing or moving a window (the size it asks for, the lines it snapped to), and those lines
    guides: Option<(usize, Piece)>,
    snap_lines: Vec<(bool, f32)>,
    /// each panel made taller than its content: how much more its picture (or the like) may take
    panel_fill: std::cell::RefCell<HashMap<String, f32>>,
    /// the box to type in, over a field (made the first time)
    input: Option<crate::inputbox::InputBox>,
    /// the dock's meters as drawn (each program's use and download, springing to what it is), and how loud each program is now
    dock_springs: std::collections::HashMap<String, [Spring; 3]>,
    dock_loud: std::collections::HashMap<String, f32>,
    /// the dock's sound bars as last drawn (in steps)
    dock_sig: Vec<i32>,
    /// each rolling number (a widget's): what it showed, what it shows now, since when it rolls
    rolls: std::cell::RefCell<std::collections::HashMap<String, (f64, f64, Instant)>>,
    /// a widget is being dragged: (screen, widget, where the mouse holds it within its box)
    drag: Option<(usize, String, f32, f32)>,
    /// where the held widget would go when the others last made room for it
    drag_last: Option<(Spot, f32)>,
    /// where the held widget was last over its own screen: where it would go, and the mouse there (what the ghost shows; let go,
    /// it goes there, wherever the mouse is let go)
    drag_at: Option<(Spot, f32, f32)>,
    /// while one is held: whether it would be joined to the one before it, and the one after it to it (its key), if let go now
    drag_join: Option<(Option<String>, Option<(String, bool)>)>,
    /// the panels popped out as windows (by their pieces' keys), the one in front, the one held by its grip (screen, key, where it
    /// is held), an open panel pressed that may be pulled off its edge (screen, key, where), and the grip and switch shown
    pops: BTreeMap<String, PopMeta>,
    pop_top: Option<String>,
    pop_drag: Option<(usize, String, f32, f32)>,
    tear: Option<(usize, String, f32, f32)>,
    pop_marks: Option<PopMarks>,
    /// windows going back, as drops in the liquid (screen, window, its box, how round)
    pop_drops: Vec<(usize, String, Bx, f32)>,
    /// the dock was to be seen last tick (as it comes back, what it shows is looked at again at once)
    dock_was_seen: bool,
    /// changes that would lay the widgets out again, made while all were tucked away: done as one comes out
    layout_pending: bool,
    /// the dock's grip the mouse is on (its first end or its last): its dot lit
    grip_at: Option<u8>,
    /// the app in front on the dock, the one before it, and when it changed (its card grows, its ground pops in)
    dock_front: Option<(String, Option<String>, Instant)>,
    /// when each widget last came out of its edge (its glow plays again from strong)
    shown_at: HashMap<String, Instant>,
    /// what the plugins host was last told of each plugin: can any of its widgets be seen
    told_seen: HashMap<String, bool>,
    /// how long and deep each widget was laid out ("screen|key"): one whose words change but not its size is only drawn again
    sizes: HashMap<String, (f32, f32)>,
    /// the panels scrolled back from their end (one that keeps to its end stays where it was put), and how tall each was then
    wheeled: HashMap<String, Instant>,
    seen_h: HashMap<String, f32>,
    /// each subtitle's words as they are typed in: all of them, how many shown, when last
    typed: HashMap<String, (String, f32, Instant)>,
    /// the page the settings window is to open on (until it says it has)
    settings_goto: Option<String>,
    /// the liquid on the graphics card (none yet: made the first time; it cannot be: drawn the old way)
    liquid_gpu: Option<Option<crate::liquidgpu::LiquidGpu>>,
    /// the thing in an open panel the mouse is on (screen, widget, which; none of them: MAX), so its ghost button is lit
    hover_hit: Option<(usize, String, usize)>,
    /// a panel being pulled off its edge: how far it is stretched (logical), and the neck of liquid behind it (screen, box)
    tear_pull: f32,
    tear_off: (f32, f32),
    tear_neck: Option<(usize, Vec<(Bx, f32)>)>,
    /// the dock and quick settings
    bi: crate::builtin::Builtins,
    pub launcher: Option<crate::launcher::Launcher>,
    /// the modal: one question over everything
    pub modal: Option<crate::modal::Modal>,
    /// the settings window, while it is open
    pub settings: Option<crate::settings::Settings>,
    /// what the repositories are doing (looking, installing), and what went wrong with the last look
    repo_busy: HashSet<String>,
    repo_errors: BTreeMap<String, String>,
    /// what the settings' long jobs (downloads) came back with
    jobs: Arc<std::sync::Mutex<Vec<Job>>>,
    /// the faces: the sound understood, the eyes, their ripples (on a layer under the widgets, per screen), the aura
    director: crate::director::Director,
    eyes: crate::eyes::Eyes,
    ripples: crate::eyes::Ripples,
    aura: crate::aura::Aura,
    fx: Vec<Option<Piece>>,
    /// while a widget is held: where it would go
    ghost: Option<Ghost>,
    /// a menu, open
    menu: Option<Menu>,
    /// Edit Mode, while it lasts
    edit: Option<edit::Edit>,
    /// the right button was down at the last tick
    rbutton: bool,
    /// the window in front keeps its keyboard from NADI (a key held in it cannot be seen)
    keys_hidden: bool,
    /// Esc was down at the last tick, and it went down at this one
    esc_down: bool,
    esc: bool,
    /// the face plugins that are on: (plugin, look)
    faces: Vec<(String, String)>,
    /// (for checking: time spent in the ticks, and which pieces were drawn, since the last dump)
    prof: std::collections::BTreeMap<String, (u32, f64)>,
    /// when the profile was last written (or cleared)
    prof_at: Instant,
    /// something was drawn since the last commit
    drew: bool,
    aura_alive: bool,
}

/// A long job of the settings, done.
pub enum Job {
    /// a picture chosen for a pin (where it is pinned, which, the picture's path; none: put away)
    Icon(String, String, Option<String>),
    Synced(String, Result<Value, String>),
    Got(String, Result<nadi_core::repo::Installed, String>),
}

pub const WM_JOB: u32 = WM_APP + 5;

fn key(plugin: &str, widget: &str) -> String {
    format!("{plugin}/{widget}")
}

fn s_of<'v>(v: &'v Value, k: &str) -> &'v str {
    v[k].as_str().unwrap_or("")
}

impl Frame {
    pub fn new(g: Gfx, pc: Arc<Pc>, tx: Sender<Command>, msg_hwnd: HWND) -> Frame {
        let (mut config, config_when) = config::load();
        // (Sign Out, once, in a launcher's foot set before it was)
        if !config.foot_signout {
            config.foot_signout = true;
            if let Some(foot) = config.launcher_foot.as_mut() {
                if !foot.iter().any(|p| p.kind == "sys" && p.id == "signout") {
                    let at = foot.iter().position(|p| p.kind == "sys" && p.id == "sleep").map_or(foot.len(), |i| i + 1);
                    foot.insert(at, crate::launcher::AppPin { kind: "sys".into(), id: "signout".into(), name: "Sign Out".into(), folder: String::new(), icon: String::new() });
                }
            }
            config::save(&config);
        }
        if keep_shortcuts(&mut config) {
            config::save(&config);
        }
        crate::draw::set_accent(&config.theme);
        crate::draw::set_accent2(&config.theme2);
        crate::paint::set_roundness(config.roundness);
        crate::keyhook::set_desk_keys(config.desk_keys);
        // (File Explorer's menu, pointing at this NADI where it is now)
        if config.explorer_menu {
            let (l, d) = explorer_paths(&config);
            crate::explorer::set(true, &l, &d);
        }
        let mut f = Frame {
            g,
            pc,
            tx,
            msg_hwnd,
            config,
            config_when,
            plugins: Vec::new(),
            widgets: BTreeMap::new(),
            screens: Vec::new(),
            queue: Vec::new(),
            banner: None,
            quiet: HashSet::new(),
            start: Instant::now(),
            last: Instant::now(),
            viz: Viz::default(),
            hover: None,
            minute: String::new(),
            slow_at: Instant::now(),
            lbutton: false,
            fronts_at: Instant::now(),
            bars_at: Vec::new(),
            timer_ms: 250,
            active: 0,
            press: None,
            mouse: (0, 0.0, 0.0),
            tip: None,
            tip_want: None,
            hold: None,
            slide: None,
            slid: None,
            meter_now: HashMap::new(),
            net_octets: None,
            focus_field: None,
            net_speed: None,
            voice_now: 0.0,
            bumps: HashMap::new(),
            bumped_at: HashMap::new(),
            bump_asked: HashMap::new(),
            card_press: None,
            glass_hidden: false,
            glow_raised: HashSet::new(),
            edge_since: std::cell::Cell::new(None),
            reach_shown: None,
            perf: crate::perf::Perf::new(),
            busy_ms: 0.0,
            render_cost: HashMap::new(),
            perf_on: false,
            perf_range: 60,
            bin: crate::recycle::State::default(),
            files_dragging: false,
            pan: None,
            view_click: None,
            lifted: HashSet::new(),
            sizing: None,
            size_hover: None,
            size_click: None,
            guides: None,
            snap_lines: Vec::new(),
            panel_fill: Default::default(),
            input: None,
            dock_springs: Default::default(),
            dock_loud: Default::default(),
            dock_sig: Vec::new(),
            rolls: Default::default(),
            drag: None,
            drag_last: None,
            drag_at: None,
            drag_join: None,
            pops: BTreeMap::new(),
            pop_top: None,
            pop_drag: None,
            tear: None,
            pop_marks: None,
            pop_drops: Vec::new(),
            wheeled: HashMap::new(),
            dock_was_seen: false,
            layout_pending: false,
            grip_at: None,
            dock_front: None,
            shown_at: HashMap::new(),
            told_seen: HashMap::new(),
            sizes: HashMap::new(),
            seen_h: HashMap::new(),
            typed: HashMap::new(),
            settings_goto: None,
            liquid_gpu: None,
            hover_hit: None,
            tear_pull: 0.0,
            tear_off: (0.0, 0.0),
            tear_neck: None,
            bi: crate::builtin::Builtins::new(),
            launcher: None,
            modal: None,
            settings: None,
            repo_busy: HashSet::new(),
            repo_errors: BTreeMap::new(),
            jobs: Arc::default(),
            director: crate::director::Director::new(),
            eyes: crate::eyes::Eyes::new(),
            ripples: crate::eyes::Ripples::new(),
            aura: crate::aura::Aura::new(),
            fx: Vec::new(),
            ghost: None,
            menu: None,
            edit: None,
            rbutton: false,
            keys_hidden: false,
            esc_down: false,
            esc: false,
            faces: Vec::new(),
            prof: std::collections::BTreeMap::new(),
            prof_at: Instant::now(),
            drew: false,
            aura_alive: false,
        };
        f.launcher = crate::launcher::Launcher::new(&f.g).ok();
        f.modal = crate::modal::Modal::new(&f.g).ok();
        if std::env::var_os("NADI_OPEN_SETTINGS").is_some() {
            f.open_settings();
        }
        crate::apps::refresh_in_background();
        f.make_screens();
        f.sync_builtins(true);
        crate::sysw::start();
        crate::net::start();
        crate::recycle::start();
        crate::update::start(f.msg_hwnd);
        // (updated since NADI last ran: what is new shown once, in a modal; not on its first run, nor going back)
        let now = env!("CARGO_PKG_VERSION");
        if f.config.last_run != now {
            let since = f.config.last_run.clone();
            let ran_before = config_when.is_some();
            f.change_config(|c| c.last_run = now.into());
            if ran_before && (since.is_empty() || crate::update::parts(now) > crate::update::parts(&since)) {
                crate::changes::after_update(f.msg_hwnd, since);
            }
        }
        crate::drop::set_notify(f.msg_hwnd);
        crate::radios::refresh();
        f.sync_sys();
        f.sync_tray();
        crate::keyhook::set_enabled(f.config.win_launcher());
        f
    }

    /// The dock and quick settings, as they are now: widgets like any other (the windows are looked at only while the dock is placed).
    fn sync_builtins(&mut self, force: bool) {
        use crate::builtin::{DOCK, PLUGIN, QUICK};
        let changed = (force || self.dock_placed() || self.place_of(PLUGIN, QUICK).is_some()) && self.bi.scan(force);
        let have = self.widgets.contains_key(&key(PLUGIN, DOCK));
        if !changed && !force && have {
            return;
        }
        let dock = self.bi.dock(&self.config);
        let quick = self.bi.quick();
        self.apply(vec![
            Update::Widget { plugin: PLUGIN.into(), widget: DOCK.into(), content: dock },
            Update::Widget { plugin: PLUGIN.into(), widget: QUICK.into(), content: quick },
            Update::Widget { plugin: PLUGIN.into(), widget: crate::builtin::APPS.into(), content: crate::builtin::apps_widget() },
        ]);
    }

    /// The PC's small things (microphone and camera, USB drives, languages, desktops), as widgets of the frame's own.
    fn sync_sys(&mut self) {
        let mut ups: Vec<Update> = crate::sysw::widgets(&self.bi.sys).into_iter().map(|(id, content)| Update::Widget { plugin: crate::builtin::PLUGIN.into(), widget: id.into(), content }).collect();
        let places = self.desk_places();
        let act = crate::sysw::Activity { flashing: self.bi.flashing.clone(), sounding: self.bi.audio.peaks().into_iter().filter(|(_, v)| *v > 0.01).map(|(k, _)| k).collect() };
        let desks = crate::sysw::desk_widget(&self.bi.sys, &places, &act);
        for u in ups.iter_mut() {
            if let Update::Widget { widget, content, .. } = u {
                if widget == crate::sysw::DESKS {
                    *content = desks.clone();
                }
            }
        }
        ups.push(Update::Widget { plugin: crate::builtin::PLUGIN.into(), widget: crate::net::NET.into(), content: crate::net::widget(&self.bi.net, self.bi.radios, self.net_speed, self.bi.net_ask.as_deref()) });
        ups.push(Update::Widget { plugin: crate::builtin::PLUGIN.into(), widget: crate::sysw::DEVICES.into(), content: crate::sysw::devices(&self.bi.sys, &self.bi.net.paired, self.bi.radios.bluetooth) });
        ups.push(Update::Widget { plugin: crate::builtin::PLUGIN.into(), widget: crate::recycle::BIN.into(), content: self.bin_widget() });
        self.apply(ups);
    }

    /// The Recycle Bin's widget as it is now: idle when empty, unless files are being dragged; its lid up while a drag is over it.
    fn bin_widget(&self) -> Value {
        let k = key(crate::builtin::PLUGIN, crate::recycle::BIN);
        crate::recycle::widget(&self.bin, self.files_dragging, crate::drop::hover().as_deref() == Some(k.as_str()), self.large(&k))
    }

    /// The Recycle Bin's widget drawn again (it changed, a drag came or went).
    fn sync_bin(&mut self) {
        let content = self.bin_widget();
        self.apply(vec![Update::Widget { plugin: crate::builtin::PLUGIN.into(), widget: crate::recycle::BIN.into(), content }]);
    }

    /// A newer NADI found (told once, in a banner: Update fetches and installs it), or its download done or failed: the settings
    /// show where it stands.
    pub fn update_changed(&mut self) {
        // (an installer coming down: a modal of how far, with a way to stop it; done or failed, put away)
        let fetching = matches!(crate::update::status(), crate::update::Status::Fetching(_));
        let showing = self.modal.as_ref().is_some_and(|m| m.shows_progress("nadi-update"));
        if fetching && !showing {
            if let crate::update::Status::Fetching(v) = crate::update::status() {
                let spec = json!({ "icon": "download", "tone": "theme", "title": v, "progress": "nadi-update", "buttons": [{ "label": "Cancel", "action": "nadi_stop_update" }] });
                self.ask(spec, crate::builtin::PLUGIN.into(), None);
            }
        } else if !fetching && showing {
            if let Some(m) = &mut self.modal {
                m.close_progress("nadi-update");
            }
            self.tick_rate();
        }
        if let crate::update::Status::Ready(v, _) = crate::update::status() {
            if self.config.update_told != v {
                let told = v.clone();
                self.change_config(|c| c.update_told = told);
                let say = json!({
                    "title": format!("NADI {v}"),
                    "text": "A new version is ready.",
                    "pill": { "icon": "download", "ms": 12000, "key": "nadi-update", "buttons": [{ "label": "Update", "action": "nadi_update" }] },
                });
                self.say(crate::builtin::PLUGIN.into(), say);
            }
        }
        self.push_settings();
    }

    /// What is new since the last version run: a modal, once.
    pub fn whats_new(&mut self) {
        if let Some(spec) = crate::changes::take_modal() {
            self.ask(spec, crate::builtin::PLUGIN.into(), None);
        }
    }

    /// A drag came over a widget that takes files, or left it.
    pub fn drop_hover_changed(&mut self) {
        self.sync_bin();
    }

    /// Files let go on a widget that takes them: its action told their paths.
    pub fn dropped(&mut self) {
        for (k, paths, text) in crate::drop::take() {
            let Some((plugin, _)) = k.split_once('/') else { continue };
            let Some(action) = self.widgets.get(&k).and_then(|w| w.content["drop"].as_str().map(str::to_string)) else { continue };
            let args = if paths.is_empty() { json!({ "text": text }) } else { json!({ "paths": paths }) };
            self.act(plugin.to_string(), action, args);
        }
    }

    /// Is a widget to be seen anywhere (shown, or showing, or open)? One hidden at its edge is not.
    fn seen_anywhere(&self, k: &str) -> bool {
        let pk = pop_key(k);
        self.screens.iter().any(|sc| sc.items.iter().any(|(ik, it)| (ik == k || *ik == pk) && (it.shown.now > 0.01 || it.shown.moving() || it.open.now > 0.0)))
    }

    /// The dock's meters, a frame on: each springs to what its program uses; the sound bars follow how loud each is. Drawn again
    /// while they move.
    fn step_dock(&mut self, dt: f32) {
        let k = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
        let Some(w) = self.widgets.get(unpop(&k)) else { return };
        let mut moving = false;
        let mut seen = HashSet::new();
        for it in w.content["peek"]["dock"]["items"].as_array().into_iter().flatten() {
            let Some(name) = it["key"].as_str() else { continue };
            seen.insert(name.to_string());
            let to = [it["cpu"].as_f64().unwrap_or(0.0) as f32, it["ram"].as_f64().unwrap_or(0.0) as f32, it["progress"].as_f64().unwrap_or(0.0) as f32];
            let sp = self.dock_springs.entry(name.to_string()).or_insert_with(|| [Spring::at(0.0), Spring::at(0.0), Spring::at(0.0)]);
            for (s, t) in sp.iter_mut().zip(to) {
                // (a meter's wiggle, less than a pixel or so: it is not drawn again for that; a real change springs)
                if !s.moving() && (t - s.now).abs() < METER_STILL {
                    continue;
                }
                s.to = t;
                moving |= s.step(dt);
            }
        }
        self.dock_springs.retain(|n, _| seen.contains(n));
        let loud = if w.content["peek"]["dock"]["items"].as_array().is_some_and(|a| a.iter().any(|i| i["sound"] == true)) { self.bi.audio.peaks() } else { Default::default() };
        let attention = w.content["peek"]["dock"]["items"].as_array().is_some_and(|a| a.iter().any(|i| i["attention"] == true));
        self.dock_loud = loud;
        // (the bars as they would be drawn, in steps of half a pixel: the same steps, nothing to draw again)
        let t = self.start.elapsed().as_secs_f32();
        let sig: Vec<i32> = w.content["peek"]["dock"]["items"].as_array().into_iter().flatten().filter(|i| i["sound"] == true).flat_map(|i| {
            let name = i["key"].as_str().unwrap_or("");
            sound_bars(self.dock_loud.get(name).copied().unwrap_or(0.0), t).into_iter().map(|b| (b * 8.0 * 2.0).round() as i32)
        }).collect();
        let bars_moved = sig != self.dock_sig;
        self.dock_sig = sig;
        if moving || bars_moved || attention {
            for si in 0..self.screens.len() {
                if self.screens[si].items.contains_key(&k) {
                    self.render(si, &k);
                }
            }
        }
    }

    /// The dock's things as drawn now: its meters where their springs are, the sound bars of each program that makes sound.
    fn dock_live(&self, content: &mut Value) {
        let t = self.start.elapsed().as_secs_f32();
        let live = |it: &mut Value| {
            let Some(name) = it["key"].as_str().map(str::to_string) else { return };
            if let Some(sp) = self.dock_springs.get(&name) {
                if !it["cpu"].is_null() {
                    it["cpu"] = json!(sp[0].now.max(0.0));
                    it["ram"] = json!(sp[1].now.max(0.0));
                }
                if !it["progress"].is_null() {
                    it["progress"] = json!(sp[2].now.clamp(0.0, 1.0));
                }
            }
            if it["sound"] == true {
                // (as loud as it sounds: quiet sound still stands up)
                it["audio"] = json!(sound_bars(self.dock_loud.get(&name).copied().unwrap_or(0.0), t));
            }
        };
        for it in content["peek"]["dock"]["items"].as_array_mut().into_iter().flatten() {
            live(it);
        }
        for b in content["panel"]["body"].as_array_mut().into_iter().flatten() {
            for it in b["items"].as_array_mut().into_iter().flatten() {
                live(it);
            }
        }
    }

    /// Is the dock on a screen (not put away)?
    pub(super) fn dock_placed(&self) -> bool {
        self.place_of(crate::builtin::PLUGIN, crate::builtin::DOCK).is_some()
    }

    /// Where a plugin's banner comes out: its widget, which becomes the banner for a while (the one the banner names, else the first
    /// of its widgets that shows, else the first it has). (Its key, its place and rank, floating.) A plugin with no widget: the top.
    fn banner_home(&self, plugin: &str, say: &Value) -> (String, Place, i32, bool, bool) {
        let ids: Vec<String> = self.plugins.iter().find(|p| p.id == plugin).map(|p| p.widgets.iter().map(|w| w.id.clone()).collect()).unwrap_or_default();
        let placed = |w: &String| self.place_of(plugin, w).is_some();
        let pick = say["widget"]
            .as_str()
            .map(String::from)
            .filter(|w| ids.contains(w) && placed(w))
            .or_else(|| ids.iter().find(|w| placed(w) && self.widgets.get(&key(plugin, w)).is_some_and(|x| x.content["peek"].is_object())).cloned())
            .or_else(|| ids.iter().find(|w| placed(w)).cloned());
        if let Some(w) = pick {
            let k = key(plugin, &w);
            if let Some((p, r)) = self.place_of(plugin, &w) {
                let f = self.floating(&k);
                let d = self.stacked_down(&k);
                return (k, p, r, f, d);
            }
        }
        (String::new(), Place::Edge(Edge::Top), 0, false, false)
    }

    /// The faces of the plugins that are on: the eyes are a widget (the frame draws it), the aura a light behind one.
    fn sync_faces(&mut self) {
        let faces: Vec<(String, String)> = self.plugins.iter().filter(|p| p.on && !p.look.is_empty()).map(|p| (p.id.clone(), p.look.clone())).collect();
        for (id, look) in &faces {
            if look == "eyes" {
                let k = key(id, "eyes");
                if !self.widgets.contains_key(&k) {
                    self.widgets.insert(k, Widget { plugin: id.clone(), id: "eyes".into(), content: json!({ "peek": { "face": "eyes" }, "tip": "Eyes" }), glow_at: Instant::now() });
                }
            }
            if look == "aura" {
                let b = self.plugins.iter().find(|p| &p.id == id).and_then(|p| p.settings.iter().find(|s| s.key == "brightness")).and_then(|s| s.value.as_f64()).unwrap_or(100.0);
                self.aura.brightness = (b / 100.0).clamp(0.0, 1.0) as f32;
            }
        }
        // (a face switched off: its widget goes)
        let gone: Vec<String> = self.widgets.iter().filter(|(_, w)| w.content["peek"]["face"].is_string() && !faces.iter().any(|(id, l)| *id == w.plugin && l == "eyes")).map(|(k, _)| k.clone()).collect();
        for k in gone {
            self.widgets.remove(&k);
        }
        self.faces = faces;
        let want = !self.faces.is_empty();
        if want {
            self.pc.want.shell_audio.store(true, Ordering::Relaxed);
        }
    }

    fn face_setting(&self, look: &str, setting: &str) -> bool {
        let Some((id, _)) = self.faces.iter().find(|(_, l)| l == look) else { return false };
        self.plugins.iter().find(|p| &p.id == id).and_then(|p| p.settings.iter().find(|s| s.key == setting)).map_or(true, |s| s.value.as_bool() != Some(false))
    }

    /// The widget the aura lights: the eyes', else the one on the top edge nearest its middle.
    fn aura_target(&self) -> Option<(usize, String)> {
        if !self.faces.iter().any(|(_, l)| l == "aura") {
            return None;
        }
        let si = self.banner_screen()?;
        let sc = &self.screens[si];
        if let Some(k) = sc.items.keys().find(|k| self.widgets.get(*k).is_some_and(|w| w.content["peek"]["face"] == "eyes")) {
            return Some((si, k.clone()));
        }
        sc.items
            .iter()
            .filter(|(_, it)| it.slot.place == Place::Edge(Edge::Top))
            .min_by(|a, b| ((a.1.slot.x + a.1.slot.w / 2.0) - sc.w / 2.0).abs().total_cmp(&((b.1.slot.x + b.1.slot.w / 2.0) - sc.w / 2.0).abs()))
            .map(|(k, _)| (si, k.clone()))
    }

    /// Slide widgets out of their edges, or back in, as the setting asks and the mouse goes; true when one moved.
    fn reveal(&mut self, dt: f32) -> bool {
        let mode = self.config.reveal.clone();
        let all = mode.is_empty() || mode == "all";
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        let m = Metrics::default();
        let mut moved_any = false;
        // (a key held in a program that keeps its keyboard from NADI cannot be seen: there, the cursor showing stands for it. A game
        // that hides the cursor while it is played shows it while its own key is held, as some do with Alt)
        let held = if self.keys_hidden { crate::winlist::cursor_showing() } else { unsafe {
            match self.config.hold_key.as_str() {
                "alt" => GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0,
                "ctrl" => GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0,
                _ => true,
            }
        } };
        for si in 0..self.screens.len() {
            let sc = &self.screens[si];
            let mon = sc.ov.monitor.rect;
            let on_screen = cursor.x >= mon.left && cursor.x < mon.right && cursor.y >= mon.top && cursor.y < mon.bottom;
            let (cx, cy) = ((cursor.x - mon.left) as f32 / sc.s, (cursor.y - mon.top) as f32 / sc.s);
            let a = sc.area;
            // the edges the mouse is at (within reach of a widget's depth there, so that it stays out while the mouse goes into it)
            let reach = |e: Edge| -> f32 {
                let deepest = sc.items.values().filter(|i| i.slot.place.horizontal() == matches!(e, Edge::Top | Edge::Bottom) && edge_of(i.slot.place).contains(&e)).map(|i| if i.slot.place.horizontal() { i.slot.h } else { i.slot.w }).fold(0.0f32, f32::max);
                let out = sc.items.values().any(|i| edge_of(i.slot.place).contains(&e) && i.shown.now > 0.5);
                if out { deepest + 14.0 } else { self.config.edge_reach.clamp(2.0, 64.0) }
            };
            let at = |e: Edge| on_screen && match e {
                Edge::Top => cy <= a.y + reach(Edge::Top),
                Edge::Bottom => cy >= a.y + a.h - reach(Edge::Bottom),
                Edge::Left => cx <= a.x + reach(Edge::Left),
                Edge::Right => cx >= a.x + a.w - reach(Edge::Right),
            };
            let edges_at: Vec<Edge> = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right].into_iter().filter(|e| at(*e)).collect();
            // (a maximized window in front on this monitor, and the setting says so: the widgets get out of its way till the mouse
            // comes to an edge)
            // (the launcher open on this monitor: over a game too, the widgets come as they do on the desktop, the mouse being yours)
            let launcher_here = si == self.active && self.launcher.as_ref().is_some_and(|l| l.shown);
            let maxed = !launcher_here && ((self.config.auto_hide != "never" && sc.full) || (self.config.auto_hide == "maximized" && sc.maxed));
            let (all, mode) = if all && maxed { (false, "edges".to_string()) } else { (all, mode.clone()) };
            let all = all || (launcher_here && self.config.launcher_reveal);
            // (out of the way, and a key is to be held: the edges bring nothing out without it. Once they are out, the key is no
            // longer needed: they stay while the mouse is at an edge or on them, as on the desktop, until they have gone again)
            // (only those that come and go count: a window popped out, a panel kept open, one always shown stay out whatever, and
            // would keep the rest from ever needing the key or getting out of the way)
            let out_now = sc.items.iter().any(|(k, i)| i.shown.to > 0.5 && !is_pop(k) && !self.always_open(k) && !self.always_shown(k) && i.open.to < 0.5);
            let edges_at = if maxed && !held && !out_now { Vec::new() } else { edges_at };
            // (the setting's wait: the mouse rests in an edge's reach so long before they come; once out, no wait)
            let wait = self.config.edge_wait.min(1000) as u128;
            let edges_at = if on_screen && wait > 0 && !out_now {
                if edges_at.is_empty() {
                    self.edge_since.set(None);
                    edges_at
                } else {
                    let since = self.edge_since.get().unwrap_or_else(Instant::now);
                    self.edge_since.set(Some(since));
                    if since.elapsed().as_millis() < wait {
                        moved_any = true;
                        Vec::new()
                    } else {
                        edges_at
                    }
                }
            } else {
                edges_at
            };
            // (only edges with a widget on them that is not idle, when the setting says so: an empty or quiet edge brings nothing out)
            let edges_at = if self.config.edge_busy_only && mode == "edges" {
                let sc = &self.screens[si];
                edges_at.into_iter().filter(|e| sc.items.iter().any(|(k, i)| !is_pop(k) && edge_of(i.slot.place).contains(e) && self.widgets.get(k).is_some_and(|w| w.content["idle"] != true))).collect()
            } else {
                edges_at
            };
            let sc = &self.screens[si];
            let keys: Vec<String> = sc.items.keys().cloned().collect();
            let always: HashSet<String> = keys.iter().filter(|k| self.always_shown(k)).cloned().collect();
            // (a menu open on this screen: every widget on it stays out while it is, over a full-screen app too)
            let menu_here = self.menu.as_ref().is_some_and(|m| m.si == si);
            let editing = self.edit.is_some();
            // (a panel kept open for good brings nothing else out)
            let any_out = self.hover.as_ref().is_some_and(|(hs, hk)| *hs == si && !self.always_open(hk)) || sc.items.iter().any(|(k, i)| (i.open.to == 1.0 && !self.always_open(k)) || i.drag_place.is_some());
            for k in keys {
                // (a popped-out window: out for as long as it is)
                if is_pop(&k) {
                    continue;
                }
                let mode = mode.clone();
                // (something that asks for you keeps it out; a dot that says something new does not)
                let attention = self.widgets.get(unpop(&k)).is_some_and(|w| matches!(w.content["peek"]["live"]["tone"].as_str(), Some("waiting" | "permission")));
                let hovered = self.hover.as_ref().is_some_and(|(hs, hk)| *hs == si && *hk == k);
                let sc = &self.screens[si];
                let it = &sc.items[&k];
                // (a panel kept open for good still comes and goes with the others; only one opened now keeps its widget out)
                let bumping = self.bumps.get(&k).is_some_and(|t| Instant::now() < *t);
                // (files being dragged: the Recycle Bin out, to take them, for as long as they are)
                let takes_drag = self.files_dragging && k == key(crate::builtin::PLUGIN, crate::recycle::BIN);
                let busy = bumping || takes_drag || (it.open.to == 1.0 && !self.always_open(&k)) || it.pinned || it.drag_place.is_some() || hovered || menu_here || editing || (always.contains(&k) && !sc.full) || self.over_full(&k);
                let want = all
                    || busy
                    || attention
                    || match mode.as_str() {
                        "edge" => edge_of(it.slot.place).iter().any(|e| edges_at.contains(e)),
                        // (any edge: every widget, while the mouse is at an edge or on one of them)
                        "edges" => !edges_at.is_empty() || any_out,
                        _ => {
                            // near: the mouse within a reach of its place
                            let b = Self::closed_box(&it.slot);
                            let near = 90.0 + if it.shown.now > 0.5 { 30.0 } else { 0.0 };
                            on_screen && cx >= b.x - near && cx <= b.x + b.w + near && cy >= b.y - near && cy <= b.y + b.h + near
                        }
                    };
                let it = self.screens[si].items.get_mut(&k).unwrap();
                let mut came = false;
                if want {
                    it.unwanted_since = None;
                    if it.shown.to < 1.0 {
                        // (in a wave: the nearer the mouse, the sooner)
                        let since = *it.wanted_since.get_or_insert_with(Instant::now);
                        let b = Self::closed_box(&it.slot);
                        let d = (b.x + b.w / 2.0 - cx).abs() + (b.y + b.h / 2.0 - cy).abs();
                        let delay = if busy || attention { 0.0 } else { (d / 1000.0 * WAVE).min(WAVE) };
                        if since.elapsed().as_secs_f32() >= delay {
                            // (out again from its edge: its glow plays again, strong, then settles)
                            came = it.shown.now < 0.3;
                            it.shown.to = 1.0;
                            it.wanted_since = None;
                        } else {
                            moved_any = true;
                        }
                    }
                } else {
                    it.wanted_since = None;
                    // (gone a moment after the mouse left: a pass across it does not flicker it)
                    let since = *it.unwanted_since.get_or_insert_with(Instant::now);
                    if since.elapsed().as_millis() > 450 {
                        it.shown.to = 0.0;
                    }
                }
                let (stiff, damp) = if it.shown.to < 0.5 { (HIDE_STIFF, HIDE_DAMP) } else { (SHOW_STIFF, SHOW_DAMP) };
                let was_moving = it.shown.moving();
                if it.shown.step_with(dt, stiff, damp) {
                    moved_any = true;
                    self.render(si, &k);
                } else if was_moving {
                    // (just come to rest: the liquid once more, without it if it is gone)
                    self.screens[si].liquid_dirty = true;
                    moved_any = true;
                }
                if came {
                    self.shown_at.insert(k.clone(), Instant::now());
                }
            }
            // over a full-screen app: the window is there only while something is out (brought to the top once, as it shows)
            let banner_here = self.banner.is_some() && Some(si) == self.banner_screen();
            let sc = &mut self.screens[si];
            if sc.full && self.config.auto_hide != "never" {
                let out = sc.items.values().any(|i| i.shown.to > 0.0 || i.shown.now > 0.01) || banner_here;
                if out != sc.win_shown {
                    sc.win_shown = out;
                    sc.ov.show(out);
                    if out {
                        sc.ov.raise();
                        if let Some(ib) = &self.input {
                            ib.raise();
                        }
                        // (the launcher stays over them)
                        if let Some(l) = &self.launcher {
                            l.raise();
                        }
                    }
                }
            }
        }
        let _ = m;
        moved_any
    }

    /// One frame of the faces: the sound, the eyes, the ripples, the aura. Which pieces to draw again.
    fn step_faces(&mut self, dt: f32) -> Vec<(usize, String)> {
        let mut redraw = Vec::new();
        if self.faces.is_empty() {
            return redraw;
        }
        let react = !self.faces.iter().any(|(_, l)| l == "eyes") || self.face_setting("eyes", "react");
        self.director.step(&self.pc, dt);
        if !react {
            self.director.audio = crate::director::Audio::default();
        }
        let now = self.start.elapsed().as_secs_f32() * 1000.0;
        // the eyes: where the cursor is, from their middle
        let eyes_at = self.screens.iter().enumerate().find_map(|(si, sc)| sc.items.iter().find(|(k, _)| self.widgets.get(*k).is_some_and(|w| w.content["peek"]["face"] == "eyes")).map(|(k, it)| (si, k.clone(), it.rest.now(), eyes_turn(it.slot.place))));
        if let Some((si, k, b, turn)) = &eyes_at {
            let sc = &self.screens[*si];
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            let m = sc.ov.monitor.rect;
            let (lx, ly) = ((pt.x - m.left) as f32 / sc.s, (pt.y - m.top) as f32 / sc.s);
            self.eyes.hover = self.hover.as_ref().is_some_and(|(hs, hk)| hs == si && hk == k);
            let audio = self.director.audio.clone();
            // (where the mouse is, as the face sees it: turned back by as much as the face is turned)
            let (dx, dy) = (lx - (b.x + b.w / 2.0), ly - (b.y + b.h / 2.0));
            let seen = if *turn > 0.0 { (dy, -dx) } else if *turn < 0.0 { (-dy, dx) } else { (dx, dy) };
            self.eyes.update(now, dt, seen, &audio);
            // (slid into its edge: nothing to draw)
            if sc.items.get(k).is_some_and(|i| i.shown.now > 0.01) {
                redraw.push((*si, k.clone()));
            }
            // the bass: a ripple from the eyes across the screen
            let beats = self.director.take_beats();
            if self.face_setting("eyes", "rings") && react {
                for (st, pan) in beats {
                    self.ripples.add(now, st, pan);
                }
            }
            self.paint_ripples(*si, (b.x + b.w / 2.0, b.y + b.h / 2.0), now);
        } else {
            self.director.take_beats();
        }
        // the aura
        if let Some((si, k)) = self.aura_target() {
            let lit = self.director.audio.level > 0.03 || self.director.audio.bands.iter().any(|b| *b > 0.04);
            let out = self.screens[si].items.get(&k).is_some_and(|i| i.shown.now > 0.01);
            if out && (lit || self.aura_alive) {
                if !redraw.contains(&(si, k.clone())) {
                    redraw.push((si, k));
                }
            }
        }
        redraw
    }

    /// The ripples, on the layer under the widgets of a screen (made when there are some, let go when they are gone).
    fn paint_ripples(&mut self, si: usize, (cx, cy): (f32, f32), now: f32) {
        while self.fx.len() < self.screens.len() {
            self.fx.push(None);
        }
        if self.ripples.rings.is_empty() {
            if let Some(p) = self.fx[si].take() {
                let _ = self.screens[si].ov.remove(&p);
            }
            return;
        }
        let sc = &self.screens[si];
        let (w, h) = (sc.w, sc.h);
        if self.fx[si].is_none() {
            let m = sc.ov.monitor.rect;
            self.fx[si] = sc.ov.piece_below(&self.g, (m.right - m.left) as u32, (m.bottom - m.top) as u32).ok();
        }
        let Some(piece) = &self.fx[si] else { return };
        let surface = piece.surface.clone();
        let p = Painter { g: &self.g, dc: &self.g.dc, s: sc.s, pc: &self.pc, t: 0.0, viz: [0.0; 3] };
        let ripples = &mut self.ripples;
        let _ = self.g.draw_surface(&surface, |_| {
            ripples.paint(&p, now, cx, cy, w.max(h));
        });
    }

    /// A press on a widget: the plugin's is told; the frame's own is done here.
    fn act(&mut self, plugin: String, action: String, args: Value) {
        // (its args say `paste`: the clipboard's picture kept in the plugin's folder first, and the plugin told its name and size)
        if plugin != crate::builtin::PLUGIN && args["paste"] == true {
            let tx = self.tx.clone();
            let Some(dir) = nadi_core::paths::plugins_dir().map(|d| d.join(&plugin)) else { return };
            std::thread::spawn(move || {
                let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
                let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
                let name = format!("paste-{ms}.png");
                let mut args = args;
                if let Some((w, h)) = crate::clip::paste_into(&dir, &name) {
                    args["file"] = json!(name);
                    args["w"] = json!(w);
                    args["h"] = json!(h);
                }
                if let Some(o) = args.as_object_mut() {
                    o.remove("paste");
                }
                let _ = tx.send(Command::Action { plugin, action, args });
            });
            return;
        }
        if plugin != crate::builtin::PLUGIN {
            let _ = self.tx.send(Command::Action { plugin, action, args });
            return;
        }
        // (a newer NADI, from its banner: fetched and installed)
        if action == "nadi_update" {
            crate::update::install();
            return;
        }
        // (its installer coming down: stopped)
        if action == "nadi_stop_update" {
            crate::update::stop();
            return;
        }
        // (another version, from its modal: fetched and installed)
        if action == "nadi_install" {
            crate::update::install_version(args["version"].as_str().unwrap_or(""));
            return;
        }
        // (the Recycle Bin's: put back, deleted for good, emptied, opened, files dropped on it thrown away)
        if action.starts_with("bin_") {
            let parse = args["parse"].as_str().unwrap_or("").to_string();
            match action.as_str() {
                "bin_back" if !parse.is_empty() => crate::recycle::put_back(parse),
                "bin_forget" if !parse.is_empty() => crate::recycle::forget(parse),
                "bin_empty" => crate::recycle::empty(),
                "bin_open" => crate::recycle::open(),
                "bin_drop" => {
                    let paths: Vec<String> = args["paths"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
                    if !paths.is_empty() {
                        crate::recycle::throw_away(paths);
                    }
                }
                _ => {}
            }
            return;
        }
        // (a pin (the dock's, or the launcher's) renamed, or given a picture of one's own)
        if let Some(rest) = ["pinrename:", "pinrenamed:", "pinicon:", "pinreset:"].iter().find(|p| action.starts_with(**p)).map(|p| (p.trim_end_matches(':'), &action[p.len()..])) {
            let (what, rest) = rest;
            let Some((at, ident)) = rest.split_once(':') else { return };
            let (at, ident) = (at.to_string(), ident.to_string());
            match what {
                "pinrename" => {
                    let now = self.pin_name(&at, &ident).unwrap_or_default();
                    let spec = json!({ "title": "Rename", "field": { "value": now, "hint": now }, "buttons": [{ "label": "Cancel" }, { "label": "Rename", "action": format!("pinrenamed:{at}:{ident}") }] });
                    self.ask(spec, crate::builtin::PLUGIN.into(), None);
                }
                "pinrenamed" => {
                    let name = args["text"].as_str().unwrap_or("").trim().to_string();
                    if !name.is_empty() {
                        self.edit_pin(&at, &ident, |n, _| *n = name);
                    }
                }
                "pinicon" => {
                    // (the launcher put away: Windows' own box comes over everything else)
                    if self.launcher.as_ref().is_some_and(|l| l.shown) {
                        self.toggle_launcher();
                    }
                    let (jobs, notify) = (self.jobs.clone(), self.msg_hwnd.0 as isize);
                    std::thread::spawn(move || {
                        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
                        let picked = crate::explorer::pick_picture();
                        jobs.lock().unwrap().push(Job::Icon(at, ident, picked));
                        unsafe {
                            let _ = PostMessageW(HWND(notify as *mut _), WM_JOB, WPARAM(0), LPARAM(0));
                        }
                    });
                }
                _ => {
                    self.edit_pin(&at, &ident, |_, icon| {
                        crate::explorer::drop_picture(icon);
                        icon.clear();
                    });
                }
            }
            return;
        }
        let mut cfg = self.config.clone();
        let (effect, keep) = self.bi.press(&mut cfg, &action, &args);
        let desk = action.starts_with("desk");
        if let Some(f) = self.bi.focus.take() {
            self.focus_field = Some((f, Instant::now()));
        }
        if keep {
            self.change_config(|c| *c = cfg);
        }
        if desk {
            self.sync_sys();
        }
        match effect {
            crate::builtin::Effect::Launcher => self.toggle_launcher(),
            crate::builtin::Effect::Changed => self.sync_builtins(true),
            crate::builtin::Effect::Settings => self.open_settings(),
            crate::builtin::Effect::None => {}
        }
    }

    /// Win and a number: the dock's app of that number (as a press of it).
    pub fn dock_key(&mut self, n: usize) {
        if n >= 1 && self.dock_placed() {
            self.act(crate::builtin::PLUGIN.into(), format!("app:{}", n - 1), json!({}));
        }
    }

    /// What Windows says of the other windows: one flashes for you (its app on the dock wants you), one came to the front (it no
    /// longer does).
    pub fn shell_event(&mut self, what: usize, hwnd: isize) {
        const HSHELL_FLASH: usize = 0x8006;
        const HSHELL_WINDOWACTIVATED: usize = 4;
        const HSHELL_RUDEAPPACTIVATED: usize = 0x8004;
        const HSHELL_WINDOWDESTROYED: usize = 2;
        let changed = match what {
            HSHELL_FLASH => {
                let new = self.bi.flashing.insert(hwnd);
                if new {
                    self.bump(&key(crate::builtin::PLUGIN, crate::builtin::DOCK));
                }
                new
            }
            HSHELL_WINDOWACTIVATED | HSHELL_RUDEAPPACTIVATED | HSHELL_WINDOWDESTROYED => self.bi.flashing.remove(&hwnd),
            _ => false,
        };
        if changed {
            self.sync_builtins(true);
        }
    }

    /// Open the launcher on the screen the frame is on (or put it away).
    pub fn toggle_launcher(&mut self) {
        self.open_launcher(false);
    }

    /// The launcher (or, `pick`: opened to pick an app to pin to the dock).
    pub(super) fn open_launcher(&mut self, pick: bool) {
        // (on the monitor the mouse is on)
        let Some(si) = self.mouse_screen().filter(|si| self.shows(*si)).or_else(|| self.banner_screen()) else { return };
        let sc = &self.screens[si];
        let (s, a, m) = (sc.s, sc.area, sc.ov.monitor.rect);
        let area = RECT { left: m.left + (a.x * s) as i32, top: m.top + (a.y * s) as i32, right: m.left + ((a.x + a.w) * s) as i32, bottom: m.top + ((a.y + a.h) * s) as i32 };
        let pinned: Vec<crate::apps::App> = self.config.pinned.iter().map(|p| crate::apps::App { name: p.name.clone(), id: p.id.clone() }).collect();
        let g = &self.g;
        if let Some(l) = &mut self.launcher {
            if l.shown {
                l.dismiss();
            } else {
                let pins = self.config.launcher_pins.clone().unwrap_or_else(crate::launcher::default_pins);
                let foot = self.config.launcher_foot.clone().unwrap_or_else(crate::launcher::default_foot);
                l.show(g, area, s, pinned, pins, foot, pick);
                // (its first frame drawn, then shown)
                l.render(g, &self.pc);
                unsafe {
                    let _ = self.g.dcomp.Commit();
                }
                l.reveal();
            }
        }
        self.tick_rate();
    }

    // ------------------------------------------------------------------------------------------------------------- settings

    /// Open the settings window (or bring it forward).
    pub fn open_settings(&mut self) {
        self.open_settings_at(None);
    }

    /// Open the settings window on a page ("plugin:<id>", "look", "frame"; none: where it was).
    pub fn open_settings_at(&mut self, goto: Option<String>) {
        self.settings_goto = goto;
        if let Some(s) = &self.settings {
            s.raise();
            self.push_settings();
            return;
        }
        if let Ok(s) = crate::settings::Settings::open(self.msg_hwnd) {
            self.settings = Some(s);
            self.push_settings();
        }
    }

    pub fn settings_hwnd(&self) -> Option<HWND> {
        self.settings.as_ref().map(|s| s.hwnd)
    }

    pub fn close_settings(&mut self) {
        if let Some(s) = self.settings.take() {
            s.close();
        }
    }

    pub fn settings_resized(&self) {
        if let Some(s) = &self.settings {
            s.resized();
        }
    }

    /// Every widget there could be (of the plugins that are on, and the frame's own), where it goes and how it looks.
    fn widget_list(&self) -> Vec<Value> {
        let mut out = Vec::new();
        let mut add = |plugin: &str, plugin_name: &str, id: &str, name: &str| {
            let k = key(plugin, id);
            let cfg = self.config.widgets.get(&k);
            let place = self.place_of(plugin, id).map(|(p, _)| p.name().to_string()).unwrap_or_else(|| "off".into());
            let rank = self.place_of(plugin, id).map(|(_, r)| r).unwrap_or(0);
            out.push(json!({
                "key": k, "name": name, "plugin": plugin, "plugin_name": plugin_name, "place": place, "rank": rank,
                "style": cfg.and_then(|c| c.style.clone()).unwrap_or_else(|| "merged".into()), "live": self.widgets.contains_key(&k),
            }));
        };
        for p in self.plugins.iter().filter(|p| p.on) {
            // (a `many` widget is the plugin's to show, each as a window: not one to place)
            for w in p.widgets.iter().filter(|w| !w.many) {
                add(&p.id, &p.name, &w.id, if w.name.is_empty() { &w.id } else { &w.name });
            }
        }
        add(crate::builtin::PLUGIN, "NADI", crate::builtin::APPS, "Apps");
        add(crate::builtin::PLUGIN, "NADI", crate::builtin::DOCK, "Dock");
        add(crate::builtin::PLUGIN, "NADI", crate::builtin::QUICK, "Volume Mixer");
        add(crate::builtin::PLUGIN, "NADI", crate::builtin::TRAY, "Tray");
        add(crate::builtin::PLUGIN, "NADI", crate::sysw::PRIVACY, "Mic and Camera");
        add(crate::builtin::PLUGIN, "NADI", crate::sysw::DRIVES, "Drives");
        add(crate::builtin::PLUGIN, "NADI", crate::sysw::LANG, "Keyboard Language");
        add(crate::builtin::PLUGIN, "NADI", crate::sysw::DESKS, "Desktops");
        add(crate::builtin::PLUGIN, "NADI", crate::net::NET, "Network");
        add(crate::builtin::PLUGIN, "NADI", crate::sysw::DEVICES, "Bluetooth");
        add(crate::builtin::PLUGIN, "NADI", crate::recycle::BIN, "Recycle Bin");
        out
    }

    /// Tell the settings page everything it draws.
    /// A plugin's own colour, when its manifest names one.
    fn plugin_accent(&self, plugin: &str) -> Option<crate::paint::Color> {
        self.plugins.iter().find(|p| p.id == plugin).and_then(|p| p.accent.as_deref()).map(crate::paint::Color::hex)
    }

    /// A pin's name now: the dock's (`at` "dock", by its program) or the launcher's ("apps", by "kind|id").
    fn pin_name(&self, at: &str, ident: &str) -> Option<String> {
        if at == "dock" {
            return self.config.pinned.iter().find(|p| p.id.eq_ignore_ascii_case(ident)).map(|p| p.name.clone());
        }
        let (kind, id) = ident.split_once('|')?;
        self.config.launcher_pins.as_ref()?.iter().find(|p| p.kind == kind && p.id.eq_ignore_ascii_case(id)).map(|p| p.name.clone())
    }

    /// A pin's name and picture changed (and kept); the launcher, open, and the dock show it so.
    fn edit_pin(&mut self, at: &str, ident: &str, f: impl FnOnce(&mut String, &mut String)) {
        let (at, ident) = (at.to_string(), ident.to_string());
        self.change_config(|c| {
            if at == "dock" {
                if let Some(p) = c.pinned.iter_mut().find(|p| p.id.eq_ignore_ascii_case(&ident)) {
                    f(&mut p.name, &mut p.icon);
                }
            } else if let Some((kind, id)) = ident.split_once('|') {
                if let Some(p) = c.launcher_pins.as_mut().and_then(|l| l.iter_mut().find(|p| p.kind == kind && p.id.eq_ignore_ascii_case(id))) {
                    f(&mut p.name, &mut p.icon);
                }
            }
        });
        let pinned: Vec<crate::apps::App> = self.config.pinned.iter().map(|p| crate::apps::App { name: p.name.clone(), id: p.id.clone() }).collect();
        let pins = self.config.launcher_pins.clone().unwrap_or_else(crate::launcher::default_pins);
        if let Some(l) = self.launcher.as_mut().filter(|l| l.shown) {
            l.set_pins(pinned, pins);
        }
        self.sync_builtins(true);
    }

    /// What NADI costs, to the Performance page: the graphs over its range, and each plugin and widget over the last minute.
    fn push_perf(&self, plugins: &BTreeMap<String, (f64, f64, u64)>) {
        let Some(s) = &self.settings else { return };
        let series = self.perf.series(self.perf_range);
        let now = self.perf.samples.back().copied().unwrap_or([0.0; 3]);
        let (pm, wm) = self.perf.minute(plugins, &self.render_cost);
        let name_of = |id: &str| self.plugins.iter().find(|p| p.id == id).map(|p| p.name.clone());
        let total = |v: &Value| v["gather"].as_f64().unwrap_or(0.0) + v["run"].as_f64().unwrap_or(0.0);
        let mut prows: Vec<Value> = pm
            .iter()
            .filter_map(|(id, (g, r, n))| {
                // (a sensor gathered for the plugins: by its own name)
                let (name, shared) = match id.strip_prefix("~ ") {
                    Some(x) => (x.chars().take(1).flat_map(char::to_uppercase).chain(x.chars().skip(1)).collect::<String>(), true),
                    None => (name_of(id)?, false),
                };
                Some(json!({ "id": id, "name": name, "shared": shared, "gather": (g * 10.0).round() / 10.0, "run": (r * 10.0).round() / 10.0, "events": n, "hot": (g + r) / 60000.0 > 0.05 }))
            })
            .filter(|v| total(v) > 0.05 || v["events"].as_u64().unwrap_or(0) > 0)
            .collect();
        prows.sort_by(|a, b| total(b).total_cmp(&total(a)));
        let widget_name = |k: &str| -> Option<(String, String)> {
            let (pl, id) = k.split_once('/')?;
            let p = self.plugins.iter().find(|p| p.id == pl)?;
            let w = p.widgets.iter().find(|w| w.id == id)?;
            Some((if w.name.is_empty() { p.name.clone() } else { w.name.clone() }, pl.to_string()))
        };
        let mut wrows: Vec<Value> = wm
            .iter()
            .filter(|(k, (n, _))| *n > 0 && !is_pop(k))
            .filter_map(|(k, (n, ms))| widget_name(k).map(|(name, plugin)| json!({ "key": k, "name": name, "plugin": plugin, "times": n, "ms": (ms * 10.0).round() / 10.0 })))
            .collect();
        wrows.sort_by(|a, b| b["ms"].as_f64().unwrap_or(0.0).total_cmp(&a["ms"].as_f64().unwrap_or(0.0)));
        let r = |v: f32, k: f32| (v * k).round() / k;
        // (memory, each program apart: NADI, its settings window, each plugin's helper)
        let memory: Vec<Value> = crate::perf::memory_parts()
            .into_iter()
            .map(|(k, mb)| {
                let name = match k.as_str() {
                    "nadi" => "NADI".to_string(),
                    "webview" => "Settings Window".to_string(),
                    "watchdog" => "Watchdog".to_string(),
                    _ => match k.strip_prefix("plugin:") {
                        Some(id) => name_of(id).unwrap_or_else(|| id.to_string()),
                        None => k.clone(),
                    },
                };
                json!({ "key": k, "name": name, "mb": r(mb, 10.0) })
            })
            .collect();
        let msg = json!({
            "type": "perf",
            "memory": memory,
            "range": self.perf_range,
            "series": series.iter().map(|v| [r(v[0], 100.0), r(v[1], 10.0), r(v[2], 10.0)]).collect::<Vec<_>>(),
            "now": [r(now[0], 100.0), r(now[1], 1.0), r(now[2], 10.0)],
            "plugins": prows,
            "widgets": wrows,
        });
        s.post(&msg.to_string());
    }

    pub fn push_settings(&self) {
        let Some(s) = &self.settings else { return };
        let busy: serde_json::Map<String, Value> = self.repo_busy.iter().map(|r| (r.clone(), json!(true))).collect();
        let state = json!({
            "type": "state",
            "version": env!("CARGO_PKG_VERSION"),
            "update": crate::update::json(),
            "changes": crate::changes::json(),
            "cleanup": crate::cleanup::json(),
            "admin_keys": { "installed": crate::adminkeys::installed(), "available": crate::adminkeys::available() },
            "autostart": crate::autostart::on(),
            "plugins": self.plugins,
            "config": self.config,
            "widgets": self.widget_list(),
            "repos": nadi_core::repo::available(),
            "busy": busy,
            "errors": self.repo_errors,
            "goto": self.settings_goto,
        });
        s.post(&state.to_string());
    }

    /// What the settings page said.
    pub fn settings_messages(&mut self) {
        // (only the edge reach's band moved: the page is not drawn again, which would let go of the slider being dragged)
        let mut quiet = true;
        for text in crate::settings::take_messages() {
            let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
            quiet &= m["cmd"] == "reach_preview" || m["cmd"] == "perf" || m["cmd"] == "round_preview" || m["cmd"] == "depth_preview";
            let st = |k: &str| m[k].as_str().unwrap_or("").to_string();
            match m["cmd"].as_str().unwrap_or("") {
                "ready" => {
                    self.perf_on = false;
                    // (the first time: what the main repository has, looked at; after that, each one looked at again once its list is an
                    // hour old, so a new release shows without pressing Refresh)
                    // (the official repository is always in the list, never taken out: looked at whenever it is not there)
                    let known = nadi_core::repo::available();
                    if !known.as_object().is_some_and(|o| o.contains_key("Llyme/nadi-plugins")) {
                        self.repo_job_sync("Llyme/nadi-plugins".into());
                    }
                    let stale: Vec<String> = known
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, e)| e["at"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map_or(true, |t| chrono::Utc::now().signed_duration_since(t) > chrono::Duration::hours(1)))
                        .map(|(r, _)| r.clone())
                        .collect();
                    for r in stale {
                        self.repo_job_sync(r);
                    }
                }
                "exit" => unsafe { PostQuitMessage(0) },
                "goto_done" => self.settings_goto = None,
                // (the edge reach being dragged in the settings: its band on the screen itself, as it is moved)
                "reach_preview" => self.show_reach(m["px"].as_f64().unwrap_or(0.0) as f32),
                // (the roundness being dragged: everything drawn as it would be, not kept until it is let go)
                // (the depths being dragged: every widget as deep as it would be, not kept until let go)
                "depth_preview" => {
                    let mut c = self.config.clone();
                    if let Some(v) = m["small"].as_f64() {
                        c.depth_small = v as f32;
                    }
                    if let Some(v) = m["large"].as_f64() {
                        c.depth_large = v as f32;
                    }
                    self.set_config(c);
                }
                "round_preview" => {
                    let mut c = self.config.clone();
                    c.roundness = m["k"].as_f64().unwrap_or(1.0).clamp(0.0, 1.5) as f32;
                    self.set_config(c);
                }
                // (a newer NADI: fetched and installed; or looked for again now)
                "update" => crate::update::install(),
                "update_check" => crate::update::check(),
                // (Clean Up: how much, looked at as the settings open; cleaned when pressed)
                "cleanup_look" => crate::cleanup::look(self.msg_hwnd),
                "cleanup" => {
                    crate::cleanup::clean(self.msg_hwnd);
                    self.push_settings();
                }
                // (the Windows key over apps run as administrator: its helper set up the first time (Windows asks), then on or off)
                "admin_keys" => {
                    let on = m["on"] == true;
                    self.change_config(|c| c.admin_keys = on);
                    if on && !crate::adminkeys::installed() {
                        crate::adminkeys::install(self.msg_hwnd);
                    }
                    self.push_settings();
                }
                // (another version: an older one asked about first (it may not suit the plugins), a newer one just installed)
                "install_version" => {
                    let v = st("version");
                    let now = env!("CARGO_PKG_VERSION");
                    if crate::update::parts(&v) < crate::update::parts(now) {
                        let spec = json!({
                            "icon": "back", "tone": "amber", "careful": true,
                            "title": "Install an Older Version?",
                            "change": [now, v],
                            "text": "Newer plugins may stop working.",
                            "buttons": [{ "label": "Cancel" }, { "label": "Install", "action": "nadi_install", "args": { "version": v }, "tone": "amber" }],
                        });
                        self.ask(spec, crate::builtin::PLUGIN.into(), None);
                    } else {
                        crate::update::install_version(&v);
                    }
                }
                // (the Performance page open or shut, and over how long)
                "perf" => {
                    self.perf_on = m["on"] == true;
                    self.perf_range = m["range"].as_u64().unwrap_or(60).clamp(60, 3600) as usize;
                    if self.perf_on {
                        let plugins = self.pc.plugin_cost.lock().unwrap().clone();
                        self.push_perf(&plugins);
                    }
                }
                "autostart" => {
                    crate::autostart::set(m["on"].as_bool() == Some(true));
                    self.push_settings();
                }
                "set_on" => {
                    let _ = self.tx.send(Command::SetOn { id: st("id"), on: m["on"].as_bool().unwrap_or(false) });
                }
                "set_value" => {
                    let _ = self.tx.send(Command::SetValue { id: st("id"), key: st("key"), value: m["value"].clone() });
                }
                "action" => {
                    let _ = self.tx.send(Command::Action { plugin: st("id"), action: st("action"), args: json!({ "text": st("text") }) });
                }
                "config" => {
                    if let (Ok(Value::Object(mut base)), Some(patch)) = (serde_json::to_value(&self.config), m["patch"].as_object()) {
                        for (k, v) in patch {
                            base.insert(k.clone(), v.clone());
                        }
                        if let Ok(c) = serde_json::from_value::<Config>(Value::Object(base)) {
                            self.change_config(|x| *x = c);
                        }
                    }
                }
                "widget" => {
                    let k = st("key");
                    let (place, style) = (m["place"].as_str().map(str::to_string), m["style"].as_str().map(str::to_string));
                    self.change_config(|c| {
                        let w = c.widgets.entry(k).or_default();
                        if place.is_some() {
                            w.place = place;
                        }
                        if style.is_some() {
                            w.style = style;
                        }
                    });
                }
                "order" => {
                    let keys: Vec<String> = m["keys"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(str::to_string)).collect();
                    self.change_config(|c| {
                        for (i, k) in keys.iter().enumerate() {
                            c.widgets.entry(k.clone()).or_default().rank = Some(i as i32 * 10);
                        }
                    });
                }
                "repo_sync" => self.repo_job_sync(st("repo")),
                "repo_get" => {
                    let repo = st("repo");
                    let ids: Vec<String> = m["ids"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(str::to_string)).collect();
                    let keep_on = m["keep_on"].as_bool().unwrap_or(false);
                    let on: HashSet<String> = self.plugins.iter().filter(|p| p.on).map(|p| p.id.clone()).collect();
                    self.repo_busy.insert(repo.clone());
                    let (jobs, notify) = (self.jobs.clone(), self.msg_hwnd.0 as isize);
                    std::thread::spawn(move || {
                        let r = nadi_core::repo::get(&repo, ids, keep_on, &|id| on.contains(id));
                        jobs.lock().unwrap().push(Job::Got(repo, r));
                        unsafe {
                            let _ = PostMessageW(HWND(notify as *mut _), WM_JOB, WPARAM(0), LPARAM(0));
                        }
                    });
                }
                // (the official repository stays)
                "repo_remove" if !st("repo").eq_ignore_ascii_case("Llyme/nadi-plugins") => {
                    let _ = nadi_core::repo::remove_repo(&st("repo"));
                    let _ = self.tx.send(Command::Rescan);
                }
                "plugin_remove" => {
                    let id = st("id");
                    let _ = self.tx.send(Command::SetOn { id: id.clone(), on: false });
                    let _ = nadi_core::repo::remove_plugin(&id);
                    let _ = self.tx.send(Command::Rescan);
                }
                "open" => match m["what"].as_str() {
                    Some("plugins") => nadi_core::win::open_url(&nadi_core::paths::plugins_dir().unwrap_or_default().to_string_lossy()),
                    Some("data") => nadi_core::win::open_url(&nadi_core::paths::data_dir().unwrap_or_default().to_string_lossy()),
                    Some("leftovers") => nadi_core::win::open_url(&crate::cleanup::folder().to_string_lossy()),
                    Some("repo") => {
                        let _ = nadi_core::repo::open_page(&st("repo"));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if !quiet {
            self.push_settings();
        }
    }

    fn repo_job_sync(&mut self, repo: String) {
        if repo.is_empty() || self.repo_busy.contains(&repo) {
            return;
        }
        self.repo_busy.insert(repo.clone());
        let (jobs, notify) = (self.jobs.clone(), self.msg_hwnd.0 as isize);
        std::thread::spawn(move || {
            let r = nadi_core::repo::sync(&repo);
            jobs.lock().unwrap().push(Job::Synced(repo, r));
            unsafe {
                let _ = PostMessageW(HWND(notify as *mut _), WM_JOB, WPARAM(0), LPARAM(0));
            }
        });
        self.push_settings();
    }

    /// The settings' long jobs came back.
    pub fn jobs_done(&mut self) {
        let done: Vec<Job> = std::mem::take(&mut *self.jobs.lock().unwrap());
        for j in done {
            match j {
                Job::Icon(at, ident, Some(path)) => {
                    if let Some(kept) = crate::explorer::keep_picture(&path) {
                        self.edit_pin(&at, &ident, |_, icon| {
                            crate::explorer::drop_picture(icon);
                            *icon = kept;
                        });
                    }
                }
                Job::Icon(_, _, None) => {}
                Job::Synced(repo, r) => {
                    self.repo_busy.remove(&repo);
                    match r {
                        Ok(_) => {
                            self.repo_errors.remove(&repo);
                        }
                        Err(e) => {
                            self.repo_errors.insert(repo, e);
                        }
                    }
                }
                Job::Got(repo, r) => {
                    self.repo_busy.remove(&repo);
                    match r {
                        Ok(installed) => {
                            self.repo_errors.remove(&repo);
                            let _ = self.tx.send(Command::Rescan);
                            for (id, on) in installed.on {
                                let _ = self.tx.send(Command::SetOn { id, on });
                            }
                        }
                        Err(e) => {
                            self.repo_errors.insert(repo, e);
                        }
                    }
                }
            }
        }
        self.push_settings();
    }

    /// What the launcher's window hears.
    pub fn launcher_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let r = self.launcher.as_mut()?.message(msg, wp, lp);
        // (drawn again only for what it handled: the window is asked many things it has no answer to, its text among them)
        if r.is_some() || msg == WM_ACTIVATE {
            if let Some(l) = &mut self.launcher {
                l.render(&self.g, &self.pc);
            }
        }
        // (it asks before doing something (restart, shut down): in a modal, out of where it was pressed)
        // (a pin's name or picture to be made one's own)
        if let Some(req) = self.launcher.as_mut().and_then(|l| l.take_pin_req()) {
            self.act(crate::builtin::PLUGIN.into(), req, json!({}));
        }
        if let Some((spec, at)) = self.launcher.as_mut().and_then(|l| l.take_ask()) {
            self.ask(spec, crate::builtin::PLUGIN.into(), Some(at));
        }
        // (its pins changed: kept)
        if let Some(pins) = self.launcher.as_mut().and_then(|l| l.take_pins()) {
            self.change_config(|c| {
                c.launcher_pins = Some(pins.0);
                c.launcher_foot = Some(pins.1);
            });
        }
        // (an app picked to pin: on the dock, last)
        if let Some(a) = self.launcher.as_mut().and_then(|l| l.take_picked()) {
            self.change_config(|c| {
                if !c.pinned.iter().any(|p| p.id.eq_ignore_ascii_case(&a.id)) {
                    c.pinned.push(config::Pin { name: a.name.clone(), id: a.id.clone(), icon: String::new() });
                }
            });
            self.sync_builtins(true);
        }
        self.commit();
        self.tick_rate();
        r
    }

    pub fn input_hwnd(&self) -> Option<HWND> {
        self.input.as_ref().map(|i| i.hwnd)
    }

    /// What the box to type in heard: what was typed given to its plugin (at every change, for a live field; at Enter); gone at Esc
    /// or when the focus goes.
    pub fn input_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        use crate::inputbox::Said;
        let ib = self.input.as_mut()?;
        let said = ib.message(msg, wp, lp);
        let (key, plugin, action, live, text) = (ib.key.clone(), ib.plugin.clone(), ib.action.clone(), ib.live, ib.text.clone());
        // (Edit Mode's Find: the shelf narrows as it is typed; Esc empties it first, then (the box gone) ends Edit Mode)
        if key == edit::FIND_KEY {
            match said {
                Said::Changed | Said::Cleared => {
                    self.set_find(text);
                    let g = &self.g;
                    if let Some(ib) = &self.input {
                        ib.render(g, &self.pc);
                    }
                }
                Said::Done | Said::Gone => {
                    let esc = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000 != 0 };
                    if esc && !text.is_empty() {
                        // (emptied, and typed into again; this Esc does not end Edit Mode)
                        self.esc_down = true;
                        if let Some(ib) = &mut self.input {
                            ib.text.clear();
                        }
                        self.set_find(String::new());
                        self.open_find();
                    } else if let Some(ib) = &mut self.input {
                        ib.close();
                    }
                }
                Said::Nothing => {}
            }
            self.commit();
            return matches!(msg, WM_CHAR | WM_KEYDOWN).then_some(LRESULT(0));
        }
        let handled = matches!(msg, WM_CHAR | WM_KEYDOWN);
        match said {
            Said::Nothing => {}
            Said::Changed | Said::Cleared => {
                if live || matches!(said, Said::Cleared) {
                    self.act(plugin, action, json!({ "text": text }));
                }
                let g = &self.g;
                if let Some(ib) = &self.input {
                    ib.render(g, &self.pc);
                }
            }
            Said::Done | Said::Gone => {
                // (words of several lines are kept as they are, emptied too, however the box goes)
                let area = self.input.as_ref().is_some_and(|i| i.area || i.in_card());
                if area || (matches!(said, Said::Done) && !text.trim().is_empty()) || (matches!(said, Said::Done) && live) {
                    self.act(plugin, action, json!({ "text": text }));
                }
                if let Some(ib) = &mut self.input {
                    ib.close();
                    // (a password is not kept once given)
                    if ib.secret {
                        ib.text.clear();
                    }
                }
                for sc in &mut self.screens {
                    if let Some(it) = sc.items.get_mut(&key) {
                        it.pinned = false;
                    }
                }
            }
        }
        self.commit();
        handled.then_some(LRESULT(0))
    }

    pub fn modal_hwnd(&self) -> Option<HWND> {
        self.modal.as_ref().map(|m| m.hwnd)
    }

    /// Ask, in a modal (see `modal`): on the screen of `at` (physical px on the desktop), growing out of there; none: where the mouse
    /// is. What is pressed goes to `owner` as one of its actions.
    pub fn ask(&mut self, spec: Value, owner: String, at: Option<(i32, i32)>) {
        let point = at.unwrap_or_else(|| {
            let mut p = POINT::default();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
            }
            (p.x, p.y)
        });
        let si = self.screens.iter().position(|sc| {
            let m = sc.ov.monitor.rect;
            point.0 >= m.left && point.0 < m.right && point.1 >= m.top && point.1 < m.bottom
        });
        let Some(si) = si.or(if self.screens.is_empty() { None } else { Some(0) }) else { return };
        let (mon, s) = (self.screens[si].ov.monitor.rect, self.screens[si].s);
        let from = ((point.0 - mon.left) as f32 / s, (point.1 - mon.top) as f32 / s);
        let g = &self.g;
        if let Some(m) = &mut self.modal {
            m.ask(g, mon, s, from, spec, owner);
            m.render(g, &self.pc);
        }
        self.commit();
        self.tick_rate();
    }

    /// What the modal's window hears.
    pub fn modal_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let g = &self.g;
        let r = self.modal.as_mut()?.message(msg, wp, lp);
        if let Some(m) = &mut self.modal {
            m.render(g, &self.pc);
        }
        self.modal_answered();
        self.commit();
        self.tick_rate();
        r
    }

    /// The modal's answer, when there is one: to whoever asked, as one of its actions.
    fn modal_answered(&mut self) {
        if let Some((owner, action, args)) = self.modal.as_mut().and_then(|m| m.take_result()) {
            self.act(owner, action, args);
        }
    }

    pub fn launcher_hwnd(&self) -> Option<HWND> {
        self.launcher.as_ref().map(|l| l.hwnd)
    }

    /// One frame window for each monitor (the frame shows on the ones the config says: the one the mouse is on, the main one, or all).
    pub fn make_screens(&mut self) {
        for sc in self.screens.drain(..) {
            for it in sc.items.values() {
                for (_, th) in &it.thumbs {
                    crate::thumbs::unregister(*th);
                }
            }
            sc.ov.destroy();
        }
        // (what belonged to the old windows goes with them)
        self.fx.clear();
        self.ghost = None;
        self.pops.clear();
        self.pop_top = None;
        self.pop_drag = None;
        self.tear = None;
        self.pop_marks = None;
        self.pop_drops.clear();
        self.tear_neck = None;
        self.tear_pull = 0.0;
        self.tear_off = (0.0, 0.0);
        self.menu = None;
        self.edit = None;
        self.hover = None;
        self.press = None;
        self.drag = None;
        for m in overlay::monitors() {
            let Ok(ov) = Overlay::new(&self.g, m) else { continue };
            let s = m.scale * self.config.scale.clamp(0.5, 3.0);
            let (w, h) = ((m.rect.right - m.rect.left) as f32 / s, (m.rect.bottom - m.rect.top) as f32 / s);
            self.screens.push(Screen { ov, s, w, h, area: Bx { x: 0.0, y: 0.0, w, h }, bar: None, primary: m.primary, items: BTreeMap::new(), banner_piece: None, banner_glow: None, banner_slot: None, full: false, win_shown: true, maxed: false, liquid: None, liquid_dirty: true, liquid_shapes: Vec::new(), raised_at: None });
        }
        self.active = self.mouse_screen().or_else(|| self.screens.iter().position(|s| s.primary)).unwrap_or(0);
        self.areas();
        self.relayout();
    }

    /// Where the frame may go on each monitor: clear of the taskbar, on whichever edge it stands (all of it, when it is hidden).
    pub fn areas(&mut self) {
        // (hidden, or under the widgets (it stays, they stand at the screen's own edge over it): the whole screen is theirs)
        let under = self.config.taskbar == "under";
        let hide = self.config.taskbar == "hide" || under;
        let mons = overlay::monitors();
        for sc in &mut self.screens {
            let Some(m) = mons.iter().find(|m| m.handle == sc.ov.monitor.handle) else { continue };
            // (under the widgets: the edge it stands on, and how deep it is there)
            sc.bar = if under {
                let free = crate::taskbar::free_of(m);
                let s = sc.s;
                if free.bottom < m.rect.bottom {
                    Some((Edge::Bottom, (m.rect.bottom - free.bottom) as f32 / s))
                } else if free.top > m.rect.top {
                    Some((Edge::Top, (free.top - m.rect.top) as f32 / s))
                } else if free.left > m.rect.left {
                    Some((Edge::Left, (free.left - m.rect.left) as f32 / s))
                } else if free.right < m.rect.right {
                    Some((Edge::Right, (m.rect.right - free.right) as f32 / s))
                } else {
                    None
                }
            } else {
                None
            };
            // (kept: clear of the taskbar on whichever edge it stands, even one that hides itself, and of other bars that keep a part of the screen)
            let work = if hide {
                m.rect
            } else {
                let free = crate::taskbar::free_of(m);
                RECT { left: free.left.max(m.work.left), top: free.top.max(m.work.top), right: free.right.min(m.work.right), bottom: free.bottom.min(m.work.bottom) }
            };
            let s = sc.s;
            sc.area = Bx {
                x: (work.left - m.rect.left) as f32 / s,
                y: (work.top - m.rect.top) as f32 / s,
                w: (work.right - work.left) as f32 / s,
                h: (work.bottom - work.top) as f32 / s,
            };
        }
    }

    /// The screen the mouse is on.
    fn mouse_screen(&self) -> Option<usize> {
        unsafe {
            let mut pt = POINT::default();
            GetCursorPos(&mut pt).ok()?;
            let m = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            self.screens.iter().position(|s| s.ov.monitor.handle == m)
        }
    }

    /// Does the frame show on this screen?
    fn shows(&self, si: usize) -> bool {
        match self.config.monitors.as_str() {
            "all" => true,
            "primary" => self.screens[si].primary,
            _ => si == self.active,
        }
    }

    /// The screen banners drop on.
    fn banner_screen(&self) -> Option<usize> {
        if self.screens.is_empty() {
            return None;
        }
        // (its widget pinned to a monitor that is there: that one)
        if let Some(b) = &self.banner {
            let (hk, ..) = self.banner_home(&b.plugin, &b.say);
            if let Some(n) = self.config.widgets.get(&hk).and_then(|c| c.monitor.clone()) {
                if let Some(i) = self.screens.iter().position(|s| s.ov.monitor.name() == n) {
                    return Some(i);
                }
            }
        }
        if self.config.monitors == "all" || self.config.monitors == "primary" {
            return self.screens.iter().position(|s| s.primary).or(Some(0));
        }
        Some(self.active.min(self.screens.len() - 1))
    }

    /// Is a widget shown on this screen: where it is pinned (while that monitor is there), else where the frame is.
    fn belongs(&self, si: usize, k: &str) -> bool {
        match self.config.widgets.get(k).and_then(|c| c.monitor.clone()) {
            Some(n) if self.screens.iter().any(|s| s.ov.monitor.name() == n) => self.screens[si].ov.monitor.name() == n && (self.shows(si) || self.always_shown(k)),
            _ => self.shows(si),
        }
    }

    /// Idle (its plugin says nothing is going on), and so hidden (unless the person wants it shown anyway, or keeps it open).
    fn idle_hidden(&self, k: &str) -> bool {
        self.widgets.get(unpop(k)).is_some_and(|w| w.content["idle"] == true)
            && self.config.widgets.get(k).and_then(|c| c.show_idle) != Some(true)
            && self.config.widgets.get(k).and_then(|c| c.always_shown) != Some(true)
    }

    fn always_shown(&self, k: &str) -> bool {
        self.config.widgets.get(k).and_then(|c| c.always_shown) == Some(true)
    }

    /// Out over a full-screen app too.
    fn over_full(&self, k: &str) -> bool {
        self.config.widgets.get(k).and_then(|c| c.over_full) == Some(true)
    }

    fn always_open(&self, k: &str) -> bool {
        // (a popped-out window: open for as long as it is out)
        if is_pop(k) {
            return true;
        }
        self.config.widgets.get(k).and_then(|c| c.always_open) == Some(true)
    }

    fn click_to_open(&self) -> bool {
        self.config.open_on == "click"
    }

    /// Open one widget's panel and shut the others (opening on a click: one at a time).
    fn open_only(&mut self, si: usize, k: &str) {
        let kept: HashSet<String> = self.widgets.keys().filter(|k| self.always_open(k)).cloned().collect();
        let mut shut = Vec::new();
        for (s, sc) in self.screens.iter_mut().enumerate() {
            for (o, it) in sc.items.iter_mut() {
                if (s != si || o != k) && it.open.to == 1.0 && !kept.contains(o) && !is_pop(o) {
                    it.open.to = 0.0;
                    it.pinned = false;
                    shut.push((s, o.clone()));
                }
            }
        }
        for (s, o) in shut {
            self.set_open(s, &o, false);
        }
        if let Some(it) = self.screens[si].items.get_mut(k) {
            it.open.to = 1.0;
            it.pinned = true;
        }
        self.set_open(si, k, true);
    }

    /// Opening on a click: a click anywhere but on an open panel (or the widget it came from) shuts it. The frame's windows hear only
    /// clicks on its pieces, so the button is looked at as the frame ticks.
    fn click_outside(&mut self) {
        let down = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8001 != 0 };
        let was = std::mem::replace(&mut self.lbutton, down);
        if !self.click_to_open() || !down || was {
            return;
        }
        // (typing: a click in the box is the box's; one elsewhere puts the box away first, and shuts the panel)
        if let Some(ib) = self.input.as_ref().filter(|i| i.shown) {
            let mut pt = POINT::default();
            let mut r = RECT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = GetWindowRect(ib.hwnd, &mut r);
            }
            if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
                return;
            }
        }
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let mut over = false;
        let mut open = Vec::new();
        for (si, sc) in self.screens.iter().enumerate() {
            let m = sc.ov.monitor.rect;
            let (x, y) = ((pt.x - m.left) as f32 / sc.s, (pt.y - m.top) as f32 / sc.s);
            let on = pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom;
            for (k, it) in &sc.items {
                if on && self.shape_now(si, it).is_ok_and(|(_, b)| b.contains(x, y)) {
                    over = true;
                }
                if it.open.to == 1.0 && !self.always_open(k) {
                    open.push((si, k.clone()));
                }
            }
            if on && self.in_banner(si, x, y) {
                over = true;
            }
        }
        if over || open.is_empty() {
            return;
        }
        for (si, k) in open {
            if let Some(it) = self.screens[si].items.get_mut(&k) {
                it.open.to = 0.0;
                it.pinned = false;
            }
            self.set_open(si, &k, false);
        }
        self.relayout_with(Some(&HashSet::new()));
    }

    /// At rest, its icon alone? (The person's word, else its manifest's.)
    fn icon_alone(&self, k: &str) -> bool {
        if self.icon_always(k) {
            return true;
        }
        if let Some(v) = self.config.widgets.get(k).and_then(|c| c.icon_only) {
            return v;
        }
        let Some(w) = self.widgets.get(unpop(k)) else { return false };
        self.plugins.iter().find(|p| p.id == w.plugin).and_then(|p| p.widgets.iter().find(|x| x.id == w.id)).is_some_and(|x| x.compact)
    }

    /// Its icon alone, always: its manifest says so, or it has no words to show (its icon and a dot at most).
    fn icon_always(&self, k: &str) -> bool {
        let Some(w) = self.widgets.get(unpop(k)) else { return false };
        if self.plugins.iter().find(|p| p.id == w.plugin).and_then(|p| p.widgets.iter().find(|x| x.id == w.id)).is_some_and(|x| x.icon_only) {
            return true;
        }
        let pk = &w.content["peek"];
        let lead = ["icon", "image", "live", "ring", "pad", "signal"].iter().any(|f| !pk[*f].is_null() && pk[*f] != "");
        // (a picture of its own for the icon look: it has another look besides, its other picture)
        let words = ["text", "sub", "clock", "button", "viz", "bars", "vessel", "dock", "tray", "face", "cells", "desks", "roll", "level", "devs", "voice", "image_icon"].iter().any(|f| !pk[*f].is_null());
        lead && !words
    }

    /// Its glow now, if it has one: its colour, how strong (0..1), and whether it changes (it breathes, or fades). A widget asks for one
    /// (`glow`: a colour, or `{ color, style: "steady" | "breathe" | "flash" }`); an icon alone with something new glows amber.
    fn glow_of(&self, k: &str) -> Option<(crate::paint::Color, f32, bool)> {
        // (named by Edit Mode's Find: lit where it is)
        if self.found_by_find(k) {
            return Some((crate::draw::accent(), GLOW_FRESH, false));
        }
        let w = self.widgets.get(unpop(k))?;
        // (a bump: strong in the second colour as it comes out, fading as it goes back)
        if let Some(e) = self.bumped_at.get(k).map(|t| t.elapsed().as_secs_f32()).filter(|e| *e < 2.4) {
            let strength = GLOW_SETTLED + (GLOW_FRESH - GLOW_SETTLED) * (-e / 0.8).exp();
            return Some((crate::draw::accent2(), strength * (1.0 - ((e - 1.6).max(0.0) / 0.8)), true));
        }
        let g = &w.content["glow"];
        // (every glow in the theme's colour, whatever its plugin asks; one whose manifest names its own colour, in that, as its banners
        // are)
        let theme = self.plugin_accent(&w.plugin).unwrap_or_else(crate::draw::accent);
        let (c, style) = if g["from"].is_string() || g.is_object() {
            (if g["second"] == true { crate::draw::accent2() } else { theme }, g["style"].as_str().unwrap_or("steady"))
        } else if g.is_string() {
            (theme, "steady")
        } else if w.content["peek"]["dot"] == true && self.icon_alone(k) {
            (crate::draw::accent2(), "steady")
        } else {
            return None;
        };
        // (strong as it comes out, or as its glow begins, then settling: hidden and shown again, it plays again)
        let since = match self.shown_at.get(k) {
            Some(t) if t.elapsed() < w.glow_at.elapsed() => t.elapsed(),
            _ => w.glow_at.elapsed(),
        }
        .as_secs_f32();
        let fresh = GLOW_SETTLED + (GLOW_FRESH - GLOW_SETTLED) * (-since / GLOW_SETTLE_S).exp();
        let settling = since < GLOW_SETTLE_S * 4.0;
        match style {
            "breathe" => {
                let t = self.start.elapsed().as_secs_f32();
                Some((c, fresh * (0.55 + 0.45 * (0.5 + 0.5 * (t * std::f32::consts::TAU / 1.8).sin())), true))
            }
            "flash" => {
                let age = w.glow_at.elapsed().as_secs_f32();
                (age < 3.0).then(|| (c, GLOW_FRESH * (1.0 - age / 3.0).powf(1.5), true))
            }
            _ => Some((c, fresh, settling)),
        }
    }

    /// A widget's peek as it is drawn: its icon alone, when so.
    fn peek_of(&self, k: &str) -> Value {
        let Some(w) = self.widgets.get(unpop(k)) else { return Value::Null };
        let mut peek = if self.icon_alone(k) { peek::icon_only(&w.content["peek"]) } else { w.content["peek"].clone() };
        // (large: drawn that much bigger, with its button; small, its button is left out)
        if self.large(k) && peek["tray"].is_object() {
            peek["tray"]["large"] = json!(true);
        } else if self.large(k) && peek.is_object() && !peek["dock"].is_object() {
            peek["scale"] = json!((self.even_depth(k) / 32.0).clamp(1.0, 2.0));
        } else if peek.is_object() {
            if let Some(o) = peek.as_object_mut() {
                o.remove("button");
            }
        }
        // (a rolling number: what it was, and how far its roll has come)
        if let Some(v) = peek["roll"]["value"].as_f64() {
            let mut rolls = self.rolls.borrow_mut();
            let r = rolls.entry(k.to_string()).or_insert((v, v, Instant::now() - Duration::from_secs(5)));
            if (r.1 - v).abs() >= 0.5 {
                *r = (r.1, v, Instant::now());
            }
            peek["roll"]["from"] = json!(r.0);
            peek["roll"]["k"] = json!((r.2.elapsed().as_secs_f32() / ROLL_FOR).min(1.0));
        }
        if peek["dock"].is_object() {
            peek["dock"]["depth"] = json!(self.even_depth(k));
        }
        // (the app in front just changed: how far its coming has got, and which it was before)
        if peek["dock"].is_object() {
            if let Some((_, prev, at)) = &self.dock_front {
                let fk = (at.elapsed().as_secs_f32() / DOCK_FRONT_S).min(1.0);
                peek["dock"]["front_k"] = json!(fk);
                peek["dock"]["front_prev"] = json!(prev);
            }
        }
        // (the dock at its largest: the app in front on its card, with what it plays when it plays)
        if peek["dock"].is_object() && self.large(k) {
            peek["dock"]["large"] = json!(true);
            let sessions = self.pc.media_seen.lock().unwrap().clone();
            if let Some(items) = peek["dock"]["items"].as_array_mut() {
                for it in items.iter_mut().filter(|i| i["front"] == true) {
                    if let Some(m) = media_of(&sessions, it) {
                        it["media"] = m;
                    }
                }
            }
        }
        // (the dock: its band on the side away from the screen's edge, its apps' marks in it; where a click opens widgets, the band is
        // its grip too, lit while the mouse is on it)
        if peek["dock"].is_object() || peek["dock_front"].is_object() {
            let place = self.screens.iter().find_map(|sc| sc.items.get(k)).map(|it| it.slot.place);
            let at = match place {
                Some(Place::Edge(Edge::Bottom) | Place::Corner(Corner::BottomLeft | Corner::BottomRight)) => Some("top"),
                Some(Place::Edge(Edge::Top) | Place::Corner(Corner::TopLeft | Corner::TopRight)) => Some("bottom"),
                Some(Place::Edge(Edge::Left)) => Some("right"),
                Some(Place::Edge(Edge::Right)) => Some("left"),
                None => None,
            };
            if let (Some(at), true) = (at, peek["dock_front"].is_object()) {
                peek["dock_front"]["inner"] = json!(at);
            } else if let Some(at) = at {
                peek["dock"]["inner"] = json!(at);
                // (the grip: while the mouse is on the dock)
                peek["dock"]["grip"] = json!(self.click_to_open() && self.hover.as_ref().is_some_and(|(_, h)| h == k));
                if let Some(g) = self.grip_at {
                    peek["dock"]["grip_at"] = json!(g);
                }
            }
        }
        // (its colour from a picture: a cover's)
        if let Some(from) = peek["color_from"].as_str() {
            if let Some((r, g, b)) = crate::images::tone(&self.pc, from) {
                peek["color"] = json!(format!("#{:02x}{:02x}{:02x}", (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
            }
        }
        peek
    }

    /// Large? (The person's word, else its manifest's.)
    fn large(&self, k: &str) -> bool {
        match self.config.widgets.get(k).and_then(|c| c.size.as_deref()) {
            Some("large") => return true,
            Some("small") => return false,
            _ => {}
        }
        let Some(w) = self.widgets.get(unpop(k)) else { return false };
        self.plugins.iter().find(|p| p.id == w.plugin).and_then(|p| p.widgets.iter().find(|x| x.id == w.id)).is_some_and(|x| x.large)
    }

    /// In a corner, in its column (not its row)?
    fn stacked_down(&self, k: &str) -> bool {
        self.config.widgets.get(k).and_then(|c| c.stack.as_deref()) == Some("down")
    }

    /// The style of a widget: floating, or merged into its edge.
    fn floating(&self, k: &str) -> bool {
        self.config.widgets.get(k).and_then(|c| c.style.as_deref()) == Some("floating")
    }

    pub fn is_frame_window(&self, hwnd: HWND) -> Option<usize> {
        self.screens.iter().position(|s| s.ov.hwnd == hwnd)
    }

    fn painter<'a>(&'a self, s: f32) -> Painter<'a> {
        Painter { g: &self.g, dc: &self.g.dc, s, pc: &self.pc, t: self.start.elapsed().as_secs_f32(), viz: self.viz.now }
    }

    /// Where a widget goes, and in what order: the config's word, else its manifest's.
    fn place_of(&self, plugin: &str, widget: &str) -> Option<(Place, i32)> {
        let (spec_place, spec_rank) = if plugin == crate::builtin::PLUGIN {
            // (the frame's own widgets are widgets like any other: each has a place until it is moved or put away; the taskbar shown
            // or not changes nothing. The tray alone is put away until placed: placing it takes the tray from Explorer)
            match widget {
                crate::builtin::DOCK => ("bottom".to_string(), -1000),
                crate::builtin::APPS => ("bottom".to_string(), -1100),
                crate::builtin::TRAY => ("off".to_string(), 20),
                crate::sysw::PRIVACY => ("top".to_string(), 50),
                crate::sysw::DESKS => ("top".to_string(), -50),
                crate::sysw::DRIVES => ("bottom-right".to_string(), 8),
                crate::sysw::LANG => ("bottom-right".to_string(), 12),
                crate::net::NET => ("bottom-right".to_string(), 14),
                crate::sysw::DEVICES => ("bottom-right".to_string(), 13),
                crate::recycle::BIN => ("bottom-right".to_string(), 16),
                _ => ("bottom-right".to_string(), 0),
            }
        } else {
            let spec = self.plugins.iter().find(|p| p.id == plugin)?.widgets.iter().find(|w| w.id == widget)?.clone();
            (spec.place, spec.rank)
        };
        let cfg = self.config.widgets.get(&key(plugin, widget));
        let place = cfg.and_then(|c| c.place.clone()).unwrap_or(spec_place);
        if place == "off" {
            return None;
        }
        Some((Place::parse(&place)?, cfg.and_then(|c| c.rank).unwrap_or(spec_rank)))
    }

    // ------------------------------------------------------------------------------------------------------------ updates

    pub fn apply(&mut self, ups: Vec<Update>) {
        let mut layout = false;
        let mut redraw: HashSet<String> = HashSet::new();
        // (its peek changed shape: laid out again only if that changed its size)
        let mut reshaped: Vec<String> = Vec::new();
        let mut bump_now: Vec<String> = Vec::new();
        // (what each widget asks of its window, as it was and is now)
        let mut windows_asked: Vec<(String, Option<Value>, Value)> = Vec::new();
        for u in ups {
            let t_up = Instant::now();
            let what = match &u {
                Update::Plugins(_) => "apply plugins".to_string(),
                Update::Widget { plugin, .. } => format!("apply widget {plugin}"),
                _ => "apply other".to_string(),
            };
            match u {
                Update::Plugins(list) => {
                    self.plugins = list;
                    layout = true;
                    self.sync_faces();
                    self.push_settings();
                    // (a plugin turned on from the tray of Edit Mode: it leaves the tray)
                    self.render_edit_tray();
                }
                Update::Widget { plugin, widget, content } => {
                    let k = key(&plugin, &widget);
                    if content.is_null() {
                        if self.widgets.remove(&k).is_some() {
                            layout = true;
                        }
                        let _ = self.tx.send(Command::Open { plugin, widget, open: false });
                    } else {
                        let mut content = content;
                        let hosts = self.plugins.iter().find(|p| p.id == plugin).map(|p| p.net.clone()).unwrap_or_default();
                        own_pictures(&mut content, &plugin, &hosts);
                        let old_peek = self.widgets.get(unpop(&k)).map(|w| w.content["peek"].clone());
                        // (it asks to bump: `bump` set, and not as it was last)
                        let asked = content["bump"].clone();
                        if !asked.is_null() && asked != false && self.bump_asked.get(&k) != Some(&asked) {
                            bump_now.push(k.clone());
                        }
                        self.bump_asked.insert(k.clone(), asked);
                        // (the same shape of peek: no new layout, the piece is drawn again)
                        if old_peek.as_ref().map(|p| peek_shape(p)) != Some(peek_shape(&content["peek"])) {
                            if old_peek.is_some() {
                                reshaped.push(k.clone());
                            } else {
                                layout = true;
                            }
                        }
                        // (it went idle, or woke up: shown or hidden)
                        if self.widgets.get(unpop(&k)).map(|w| w.content["idle"] == true) != Some(content["idle"] == true) {
                            layout = true;
                        }
                        // (an open panel takes the room its content needs; one that went away shuts)
                        if self.screens.iter().any(|sc| sc.items.get(&k).is_some_and(|i| i.open.to == 1.0)) {
                            layout = true;
                            if !content["panel"].is_object() {
                                for sc in &mut self.screens {
                                    if let Some(it) = sc.items.get_mut(&k) {
                                        it.open.to = 0.0;
                                        it.pinned = false;
                                    }
                                }
                                let _ = self.tx.send(Command::Open { plugin: plugin.clone(), widget: widget.clone(), open: false });
                            }
                        }
                        // (its glow changed: a flash starts now)
                        let glow_at = match self.widgets.get(unpop(&k)) {
                            Some(w) if w.content["glow"] == content["glow"] => w.glow_at,
                            _ => Instant::now(),
                        };
                        windows_asked.push((k.clone(), self.widgets.get(unpop(&k)).map(|w| w.content["window"].clone()), content["window"].clone()));
                        self.widgets.insert(k.clone(), Widget { plugin, id: widget, content, glow_at });
                        redraw.insert(k);
                    }
                }
                Update::Say { plugin, say } => self.say(plugin, say),
                Update::Cancel { plugin, key } => self.cancel(&plugin, &key),
                Update::Modal { plugin, spec } => self.ask(spec, plugin, None),
                // (what a plugin adds to the launcher's search)
                Update::Apps { plugin, apps } => {
                    // (its pictures of its own folder, found)
                    let mut apps = Value::Array(apps);
                    let hosts = self.plugins.iter().find(|p| p.id == plugin).map(|p| p.net.clone()).unwrap_or_default();
                    own_pictures(&mut apps, &plugin, &hosts);
                    crate::apps::set_from_plugin(&plugin, apps.as_array().map(|a| a.as_slice()).unwrap_or(&[]));
                }
                // (how far things are on the dock's apps, as a plugin tells it)
                Update::Progress { plugin, items } => {
                    crate::builtin::set_progress(&plugin, items);
                    self.sync_builtins(true);
                }
                Update::Quiet { plugin, on } => {
                    if on {
                        self.quiet.insert(plugin);
                    } else {
                        self.quiet.remove(&plugin);
                    }
                }
            }
            self.prof_add(&what, t_up);
        }
        // (the dock's app in front changed: its coming forward starts now, before it is drawn)
        self.note_front();
        // (plugins that asked to bump; not as NADI starts)
        if self.start.elapsed() > Duration::from_secs(6) {
            for k in bump_now {
                self.bump(&k);
            }
        }
        let t_size = Instant::now();
        if !layout {
            let changed: Vec<String> = reshaped.into_iter().filter(|k| !self.same_size(k)).collect();
            // (only ones tucked into their edges changed size: laid out as one comes out)
            if !changed.is_empty() {
                if changed.iter().all(|k| self.tucked_everywhere(k)) {
                    self.layout_pending = true;
                } else {
                    layout = true;
                }
            }
        }
        self.prof_add("apply size check", t_size);
        for (k, old, new) in windows_asked {
            self.window_asked(&k, old, &new);
        }
        let t_lay = Instant::now();
        // (a window popped out whose panel changed: as big as its panel now, whatever else is laid out)
        let popped_changed = redraw.iter().any(|k| self.popped(k));
        if layout {
            // (every widget tucked into its edge: laid out when one comes out, not at each change)
            if self.all_tucked() {
                self.layout_pending = true;
                if popped_changed {
                    for si in 0..self.screens.len() {
                        self.relayout_pops(si, Some(&redraw));
                    }
                    self.commit();
                }
            } else {
                self.relayout_with(Some(&redraw));
            }
        } else {
            if popped_changed {
                for si in 0..self.screens.len() {
                    self.relayout_pops(si, Some(&redraw));
                }
            }
            for k in redraw {
                for i in 0..self.screens.len() {
                    if self.screens[i].items.contains_key(&k) {
                        self.render(i, &k);
                    }
                }
            }
            self.commit();
        }
        self.prof_add(if layout { "apply layout" } else { "apply draw" }, t_lay);
        self.tick_rate();
    }

    /// Measure every widget's peek, lay each screen out, and make, move or drop the pieces.
    pub fn relayout(&mut self) {
        self.relayout_with(None);
    }

    /// Is a widget as big as when it was last laid out, on every screen it is on (its words changed, its size did not)? Then it is only
    /// drawn again. On no screen (not placed, idle, elsewhere): nothing to lay out for it, so yes. Held, edited, open, or a banner
    /// out: not known, so no.
    fn same_size(&self, k: &str) -> bool {
        if self.drag.is_some() || self.edit.is_some() || self.banner.is_some() {
            return false;
        }
        let Some(w) = self.widgets.get(k) else { return true };
        // (not placed anywhere: nothing of it is laid out)
        let Some((place, _)) = self.place_of(&w.plugin, &w.id) else { return true };
        let down = matches!(place, Place::Corner(_)) && self.stacked_down(k);
        let look = drag_look(place, !down, down);
        for si in 0..self.screens.len() {
            let Some(it) = self.screens[si].items.get(k) else { continue };
            if it.open.to > 0.0 || it.open.now > 0.0 {
                return false;
            }
            let Some(&(w0, b0)) = self.sizes.get(&format!("{si}|{k}")) else { return false };
            let p = self.painter(self.screens[si].s);
            let (w1, b1) = self.sized(si, &p, k, look, matches!(place, Place::Corner(_)) && !down);
            if (w1 - w0).abs() > 0.5 || (b1 - b0).abs() > 0.5 {
                return false;
            }
        }
        true
    }

    /// Tell the plugins host which plugins have a widget to be seen (shown, coming or going, open, a window, out as a banner, or
    /// all of them while editing), when that changes. A plugin with no widgets is not told (it is always "seen").
    fn tell_seen(&mut self) {
        let editing = self.edit.is_some();
        let banner = self.banner.as_ref().map(|b| b.plugin.clone());
        let mut now: Vec<(String, bool)> = Vec::new();
        for p in &self.plugins {
            if !p.on || p.widgets.is_empty() {
                continue;
            }
            let seen = editing
                || banner.as_deref() == Some(p.id.as_str())
                || p.widgets.iter().any(|w| {
                    let k = key(&p.id, &w.id);
                    self.seen_anywhere(&k)
                });
            now.push((p.id.clone(), seen));
        }
        for (id, seen) in now {
            if self.told_seen.get(&id) != Some(&seen) {
                self.told_seen.insert(id.clone(), seen);
                let _ = self.tx.send(Command::Seen { plugin: id, seen });
            }
        }
    }

    /// Is it quiet: something full screen in front where banners come out (as the person set it), or a plugin asking for quiet (a
    /// game runs)? Then the small banners wait.
    fn is_quiet(&self) -> bool {
        !self.quiet.is_empty() || (self.config.quiet_full && self.banner_screen().is_some_and(|si| self.screens[si].full))
    }

    /// The dock's app in front: changed, its coming forward starts now (the one before it noted, to go back).
    fn note_front(&mut self) {
        let dk = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
        let now = self.widgets.get(&dk).and_then(|w| w.content["peek"]["dock"]["items"].as_array().and_then(|a| a.iter().find(|i| i["front"] == true)).and_then(|i| i["key"].as_str().map(str::to_string)));
        if let Some(now) = now.filter(|n| self.dock_front.as_ref().map(|f| &f.0) != Some(n)) {
            let prev = self.dock_front.take().map(|f| f.0);
            self.dock_front = Some((now, prev, Instant::now()));
        }
    }

    /// Is a widget tucked into its edge on every screen it is on (and on one at least)?
    fn tucked_everywhere(&self, k: &str) -> bool {
        let mut any = false;
        for sc in &self.screens {
            if let Some(it) = sc.items.get(k) {
                if !tucked(it, k) {
                    return false;
                }
                any = true;
            }
        }
        any
    }

    /// Is every widget tucked into its edge (and nothing held, edited or out as a banner)? None at all is not.
    fn all_tucked(&self) -> bool {
        self.banner.is_none()
            && self.edit.is_none()
            && self.drag.is_none()
            && self.menu.is_none()
            && !self.screens.is_empty()
            && self.screens.iter().all(|sc| !sc.items.is_empty() && sc.items.iter().all(|(k, it)| tucked(it, k)))
    }

    /// The same, drawing again only the pieces that are new, moved, or named in `dirty` (none named: all).
    fn relayout_with(&mut self, dirty: Option<&HashSet<String>>) {
        self.layout_pending = false;
        let m = Metrics::default();
        for si in 0..self.screens.len() {
            let sc_s = self.screens[si].s;
            let area = self.screens[si].area;
            let mut items: Vec<LayoutItem> = Vec::new();
            let mut measured: Vec<(String, f32)> = Vec::new();
            let mut sized_now: Vec<(String, f32, f32)> = Vec::new();
            // (a widget held: it goes, for now, where it would be dropped, and the others of that edge stand by where they are)
            let held = self.drag.as_ref().filter(|(ds, ..)| *ds == si).and_then(|(_, dk, _, _)| {
                let it = self.screens[si].items.get(dk)?;
                Some((dk.clone(), Spot { place: it.drag_place?, float: it.drag_float, down: it.drag_down, head: it.drag_head }))
            });
            let (mx, my) = self.drag_at.map_or((self.mouse.1, self.mouse.2), |(_, x, y)| (x, y));
            {
                let p = self.painter(sc_s);
                for (k, w) in &self.widgets {
                    // (the frame is elsewhere: only what is pinned to this monitor)
                    if !self.belongs(si, k) {
                        continue;
                    }
                    // (nothing going on, as its plugin says: hidden, unless it is to be shown anyway or is held)
                    if self.idle_hidden(k) && self.edit.is_none() && !held.as_ref().is_some_and(|(hk, _)| hk == k) {
                        continue;
                    }
                    let Some((mut place, mut rank)) = self.place_of(&w.plugin, &w.id) else { continue };
                    let mut down = matches!(place, Place::Corner(_)) && self.stacked_down(k);
                    if let Some((hk, spot)) = &held {
                        if hk == k {
                            place = spot.place;
                            down = spot.down;
                            rank = held_order(*spot, mx, my, area) as i32;
                        } else if place == spot.place {
                            if let Some(it) = self.screens[si].items.get(k) {
                                rank = order_in(place, it.slot.place, it.rest.now(), area) as i32;
                            }
                        }
                    }
                    // (in a corner's column: measured as on its side edge)
                    let look = drag_look(place, !down, down);
                    if !w.content["peek"].is_object() {
                        continue;
                    }
                    let open = self.screens[si].items.get(k).is_some_and(|i| i.open.to == 1.0);
                    let floating = match &held {
                        Some((hk, spot)) if hk == k => spot.float,
                        _ => self.floating(k),
                    };
                    // (a floating piece stands off the edge: it needs that much more room across)
                    let off = if floating { self.float_off() } else { 0.0 };
                    if open {
                        let (along, across, content_h) = self.panel_size(sc_s, area, k, look);
                        measured.push((k.clone(), content_h));
                        items.push(LayoutItem { key: k.clone(), place, rank, want: along, across: across + off, down, join: self.join_now(k) });
                    } else {
                        let (want, base) = self.sized(si, &p, k, look, matches!(place, Place::Corner(_)) && !down);
                        sized_now.push((k.clone(), want, base));
                        let _ = w;
                        let edge_depth = if look.horizontal() { m.depth_h } else { m.depth_v };
                        let across = if floating { base + off } else if (base - edge_depth).abs() > 0.5 { base } else { 0.0 };
                        // (its window out: a stump where it stands, as long as it is shut, but shallow, its dot on it; its icon alone,
                        // floating or not, too)
                        // (floating, it stays as deep as it was: its stump is drawn in the middle of that, where the widget was)
                        let want = if self.popped(k) && self.icon_alone(k) { STUMP } else { want };
                        let across = if self.popped(k) && floating { across } else if self.popped(k) { STUMP + if floating { off } else { 0.0 } } else { across };
                        items.push(LayoutItem { key: k.clone(), place, rank, want, across, down, join: self.join_now(k) });
                    }
                }
            }
            for (k, w, b) in sized_now {
                self.sizes.insert(format!("{si}|{k}"), (w, b));
            }
            // (a banner out on this screen: its plugin's widget becomes it, the banner's size, and the others make room)
            let mut banner_float = false;
            if let Some(b) = self.banner.as_ref().filter(|_| Some(si) == self.banner_screen()) {
                let bh = banner_h(&b.say);
                let (hk, place, rank, float, down) = self.banner_home(&b.plugin, &b.say);
                let (rank, down) = items.iter().find(|i| i.key == hk).map_or((rank, down), |i| (i.rank, i.down));
                items.retain(|i| i.key != hk);
                banner_float = float;
                let off = if float { self.float_off() } else { 0.0 };
                let side = matches!(place, Place::Edge(Edge::Left | Edge::Right)) || (down && matches!(place, Place::Corner(_)));
                let bw = self.banner_w(sc_s, &b.say);
                let (want, across) = if side { (bh, bw) } else { (bw, bh) };
                items.push(LayoutItem { key: BANNER_KEY.into(), place, rank, want, across: across + off, down, join: None });
            }
            // (joined to one not out here (hidden, idle): to the one that one is joined to, and so on, so a run joined together
            // stays together with one of it gone)
            let present: HashSet<String> = items.iter().map(|i| i.key.clone()).collect();
            for it in &mut items {
                let mut j = it.join.clone();
                for _ in 0..32 {
                    match &j {
                        Some(t) if !present.contains(t) => j = self.join_now(t),
                        _ => break,
                    }
                }
                it.join = j;
            }
            // (a corner's own one, floating: it stands off both its edges, so it wants that much more along its edge too)
            for c in [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight] {
                let head = items.iter().enumerate().filter(|(_, i)| i.place == Place::Corner(c) && !i.down).min_by(|a, b| a.1.rank.cmp(&b.1.rank).then(a.1.key.cmp(&b.1.key))).map(|(n, _)| n);
                if let Some(n) = head {
                    let k = items[n].key.clone();
                    let float = match &held {
                        Some((hk, spot)) if *hk == k => spot.float,
                        _ => self.floating(&k),
                    };
                    let open = self.screens[si].items.get(&k).is_some_and(|i| i.open.to == 1.0);
                    if float && !open {
                        items[n].want += self.float_off();
                    }
                }
            }
            let mut slots = layout::layout(&items, area.w, area.h, m, &[]);
            for sl in &mut slots {
                sl.x += area.x;
                sl.y += area.y;
            }
            self.screens[si].banner_slot = slots.iter().position(|s| s.key == BANNER_KEY).map(|i| {
                let sl = slots.remove(i);
                (sl.place, Self::closed_box(&sl), banner_float)
            });
            if let (Some((_, bx, _)), Some(b)) = (self.screens[si].banner_slot, self.banner.as_mut()) {
                // (the first time, or on another screen: there at once; else it springs there)
                match &mut b.home {
                    Some(h) if b.si == Some(si) => h.set(bx),
                    _ => {
                        let mut h = SpringBox::default();
                        h.set(bx);
                        h.x.now = bx.x;
                        h.y.now = bx.y;
                        h.w.now = bx.w;
                        h.h.now = bx.h;
                        b.home = Some(h);
                    }
                }
            }
            let keep: HashSet<String> = slots.iter().map(|s| s.key.clone()).collect();
            let gone: Vec<String> = self.screens[si].items.keys().filter(|k| !keep.contains(*k) && !is_pop(k)).cloned().collect();
            for k in gone {
                if let Some(it) = self.screens[si].items.remove(&k) {
                    self.screens[si].liquid_dirty = true;
                    let _ = self.screens[si].ov.remove(&it.piece);
                    if let Some(g) = &it.glow {
                        self.screens[si].ov.remove_glass(g);
                    }
                    for (_, th) in &it.thumbs {
                        crate::thumbs::unregister(*th);
                    }
                }
                if self.hover.as_ref().is_some_and(|(s, h)| *s == si && *h == k) {
                    self.hover = None;
                }
            }
            for slot in slots {
                let k = slot.key.clone();
                let floating = match &held {
                    Some((hk, spot)) if *hk == k => spot.float,
                    _ => self.floating(&k),
                };
                let exists = self.screens[si].items.contains_key(&k);
                let mut fresh = !exists;
                let held = self.drag.as_ref().is_some_and(|(ds, dk, _, _)| *ds == si && *dk == k);
                if exists {
                    let it = self.screens[si].items.get_mut(&k).unwrap();
                    if !held {
                        it.rest.set(Self::closed_box(&slot));
                    }
                    fresh = it.slot.dot != slot.dot || it.rest.moving() || it.floating != floating;
                    it.slot = slot;
                    it.floating = floating;
                } else {
                    let to = Self::closed_box(&slot);
                    // (it grows out of the edge from a dot where it goes)
                    let depth = if slot.place.horizontal() { m.depth_h } else { m.depth_v };
                    let seed = if slot.place.horizontal() {
                        Bx { x: to.x + to.w / 2.0 - (depth + 2.0 * m.fillet) / 2.0, y: to.y, w: depth + 2.0 * m.fillet, h: to.h }
                    } else {
                        Bx { x: to.x, y: to.y + to.h / 2.0 - (depth + 2.0 * m.fillet) / 2.0, w: to.w, h: depth + 2.0 * m.fillet }
                    };
                    let mut rest = SpringBox { x: Spring::at(seed.x), y: Spring::at(seed.y), w: Spring::at(seed.w), h: Spring::at(seed.h) };
                    rest.set(to);
                    let (pw, ph) = ((slot.w * sc_s * 1.15).ceil() as u32 + 4, (slot.h * sc_s * 1.15).ceil() as u32 + 4);
                    let Ok(piece) = self.screens[si].ov.piece(&self.g, pw, ph) else { continue };
                    self.screens[si].items.insert(
                        k.clone(),
                        Item {
                            piece,
                            glow: None,
                            slot,
                            open: Spring::default(),
                            rest,
                            content_h: 0.0,
                            scroll: 0.0,
                            hits: Vec::new(),
                            buttons: Vec::new(),
                            thumb_boxes: Vec::new(),
                            thumbs: Vec::new(),
                            sections_open: HashSet::new(),
                            sections_closed: HashSet::new(),
                            hover_since: None,
                            swell: Spring::at(0.0),
                            left_at: None,
                            alive: false,
                            pinned: false,
                            floating,
                            visible: 0.0,
                            scrolled_at: None,
                            drag_place: None,
                            drag_float: false,
                            drag_down: false,
                            drag_head: false,
                            shown: Spring::at(if self.config.reveal == "all" || self.config.reveal.is_empty() { 1.0 } else { 0.0 }),
                            unwanted_since: None,
                            wanted_since: None,
                        },
                    );
                }
                if let Some((_, h)) = measured.iter().find(|(x, _)| *x == k) {
                    let it = self.screens[si].items.get_mut(&k).unwrap();
                    it.content_h = *h;
                }
                if fresh || dirty.map_or(true, |d| d.contains(&k)) {
                    self.render(si, &k);
                }
            }
            // (the windows popped out on this screen: as big as their panels now)
            self.relayout_pops(si, dirty);
        }
        // (the banner drawn where its widget is now, on the screen it is on now)
        if self.banner.is_some() {
            self.render_banner();
        }
        // (the banner's piece goes on top of the widgets)
        self.hit_regions();
        self.commit();
        self.tick_rate();
    }

    pub fn commit(&mut self) {
        // (the liquid of each screen whose widgets changed: drawn again, with them)
        for si in 0..self.screens.len() {
            if self.screens[si].liquid_dirty {
                self.screens[si].liquid_dirty = false;
                self.render_liquid(si);
            }
        }
        unsafe {
            let _ = self.g.dcomp.Commit();
        }
    }

    /// How liquid (the blur's reach, logical px; none: off).
    fn liquid_px(&self) -> Option<f32> {
        liquid_px(self.config.liquid)
    }

    /// How far a floating widget stands off its edge (its liquid is its own: the edge's does not reach it, however strong).
    fn float_off(&self) -> f32 {
        FLOAT_OFF
    }

    /// The widget a widget is joined to (the one before it on its edge, while it is just before it).
    fn joined(&self, k: &str) -> Option<String> {
        self.config.widgets.get(k).and_then(|c| c.join.clone())
    }

    /// The widgets joined to `k`: moved away or hidden, it leaves them joined to nothing.
    fn joined_to(&self, k: &str) -> Vec<String> {
        self.config.widgets.iter().filter(|(_, w)| w.join.as_deref() == Some(k)).map(|(o, _)| o.clone()).collect()
    }

    /// Joined now: as it would be if the one held were let go here (the held one, and the one after it), else as kept.
    fn join_now(&self, k: &str) -> Option<String> {
        if let (Some((_, held, _, _)), Some((before, after))) = (&self.drag, &self.drag_join) {
            if held == k {
                return before.clone();
            }
            if let Some((ak, a)) = after {
                if ak == k {
                    return a.then(|| held.clone());
                }
            }
            // (joined to the one held, and not joined to it where it is held now: apart)
            if self.joined(k).as_deref() == Some(held.as_str()) {
                return None;
            }
        }
        self.joined(k)
    }

    /// Would the held one be joined to the one before it, and the one after it to it, let go at `spot`? (by their bodies, less the
    /// shoulders where each meets the edge; joined already, it takes a little more to part them)
    fn join_for(&self, ds: usize, k: &str, spot: Spot, x: f32, y: f32) -> (Option<String>, Option<(String, bool)>) {
        let place = spot.place;
        let a = self.screens[ds].area;
        // (in a corner: the corner's own, and those of the same row (or column) as where it would go)
        let in_corner = matches!(place, Place::Corner(_));
        let same_run = |o: &str, it: &Item| {
            if !in_corner {
                return true;
            }
            let head = matches!(it.slot.place, Place::Corner(_));
            let down = self.config.widgets.get(o).and_then(|c| c.stack.as_deref()) == Some("down");
            head || down == spot.down
        };
        let mut row: Vec<(f32, String)> = self.screens[ds]
            .items
            .iter()
            .filter(|(o, it)| o.as_str() != k && self.widgets.get(*o).and_then(|w| self.place_of(&w.plugin, &w.id)).is_some_and(|(p, _)| p == place) && same_run(o, it))
            .map(|(o, it)| (order_in(place, it.slot.place, it.rest.now(), a), o.clone()))
            .collect();
        row.push((held_order(spot, x, y, a), k.to_string()));
        row.sort_by(|a, b| a.0.total_cmp(&b.0));
        let Some(h) = self.screens[ds].items.get(k).map(|it| it.rest.now()) else { return (None, None) };
        let fl = Metrics::default().fillet;
        let look = drag_look(place, spot.head, spot.down);
        // (along the way it would stand: a corner's row across, its column down; the nearer the corner the earlier)
        let far = |c: Corner, b: &Bx| -> (f32, f32) {
            let left = matches!(c, Corner::TopLeft | Corner::BottomLeft);
            let top = matches!(c, Corner::TopLeft | Corner::TopRight);
            if look.horizontal() {
                if left { (b.x + fl, b.x + b.w - fl) } else { (a.x + a.w - (b.x + b.w) + fl, a.x + a.w - b.x - fl) }
            } else if top {
                (b.y + fl, b.y + b.h - fl)
            } else {
                (a.y + a.h - (b.y + b.h) + fl, a.y + a.h - b.y - fl)
            }
        };
        let along = |b: &Bx| match place {
            Place::Corner(c) => far(c, b),
            _ => if look.horizontal() { (b.x + fl, b.x + b.w - fl) } else { (b.y + fl, b.y + b.h - fl) },
        };
        let at = row.iter().position(|(_, key)| key == k).unwrap_or(0);
        let neighbour = |i: usize| row.get(i).and_then(|(_, key)| self.screens[ds].items.get(key).map(|it| (key.clone(), it.rest.now())));
        let (prev_before, prev_after) = match &self.drag_join {
            Some((b, a)) => (b.is_some(), a.as_ref().is_some_and(|(_, j)| *j)),
            None => (false, false),
        };
        // (held floating, its box has no shoulders: its body is all of it)
        let (a0, a1) = if spot.float { let (x0, x1) = along(&h); (x0 - fl, x1 + fl) } else { along(&h) };
        let before = at.checked_sub(1).and_then(neighbour).filter(|(_, b)| a0 - along(b).1 < JOIN_REACH + if prev_before { 8.0 } else { 0.0 }).map(|(key, _)| key);
        let after = neighbour(at + 1).map(|(key, b)| (key, along(&b).0 - a1 < JOIN_REACH + if prev_after { 8.0 } else { 0.0 }));
        // (the corner's own: nothing before it to join)
        if in_corner && spot.head {
            return (None, after.map(|(key, _)| (key, false)));
        }
        (before, after)
    }

    /// Is a widget drawn as part of the liquid now (out, shut), its own shape left to it?
    fn in_liquid(&self, it: &Item) -> bool {
        // (a stump, its window out: too small for the liquid, which would wear it away; it is its own shape)
        if self.popped(&it.slot.key) && !is_pop(&it.slot.key) {
            return false;
        }
        self.liquid_px().is_some() && it.open.now < 0.01 && it.open.to < 0.5 && (it.shown.now > 0.003 || it.shown.moving() || it.drag_place.is_some())
    }

    /// The liquid under a screen's widgets: each shut widget's shape (from its edge out, a merged one; a drop of its own, a
    /// floating one) and a strip along each edge they stand on, drawn together, blurred, and made sharp again where the blur is
    /// thick enough: shapes that are close run together, and each widget meets its edge in a smooth neck. A panel that is open
    /// draws its own shape.
    fn render_liquid(&mut self, si: usize) {
        let Some(px) = self.liquid_px() else {
            let sc = &mut self.screens[si];
            if let Some(p) = sc.liquid.take() {
                sc.ov.remove_glass(&p);
            }
            return;
        };
        let m = Metrics::default();
        let f = m.fillet;
        let reach = 40.0;
        // (each shape, logical: a box and how round)
        let mut shapes: Vec<(Bx, f32)> = Vec::new();
        // (the floating ones: a liquid of their own, so the edge's never draws them in; joined, they still run together)
        let mut drops: Vec<(Bx, f32)> = Vec::new();
        // (whether each shape's widget glows, in the same order; and the line round them: its colour and how strong)
        let mut shapes_lit: Vec<bool> = Vec::new();
        let mut drops_lit: Vec<bool> = Vec::new();
        let mut glow_line = [0.0f32; 4];
        let (sw, sh) = (self.screens[si].w, self.screens[si].h);
        for it in self.screens[si].items.values() {
            // (a panel open on its edge: in the liquid too (under its own shape), so it runs into the edge and into the widgets
            // beside it as a shut one does)
            let open_panel = it.open.now > 0.01 && !is_pop(&it.slot.key) && !self.popped(&it.slot.key) && !it.floating && it.drag_place.is_none() && it.shown.now > 0.003;
            if !self.in_liquid(it) && !open_panel {
                continue;
            }
            let Ok((_, b)) = self.shape_now(si, it) else { continue };
            let depth = if it.slot.place.horizontal() { m.depth_h } else { m.depth_v };
            let held = it.drag_place.is_some();
            let place = match it.drag_place {
                Some(p) => drag_look(p, it.drag_head, it.drag_down),
                None => it.slot.place,
            };
            let lit = match self.glow_of(&it.slot.key) {
                Some((c, st, _)) if !is_pop(&it.slot.key) => {
                    let a = ((st - 0.5) / 1.5).clamp(0.3, 1.0) * it.shown.now.clamp(0.0, 1.0);
                    if a > glow_line[3] {
                        glow_line = [c.0, c.1, c.2, a];
                    }
                    a > 0.01
                }
                _ => false,
            };
            if (held && it.drag_float) || (!held && it.floating) {
                let r = (b.w.min(b.h) / 2.0).min(depth / 2.0 + 2.0);
                // (as round as the setting says, as the merged ones are)
                let r = (r * crate::paint::roundness()).max(0.5);
                drops.push((b, r));
                drops_lit.push(lit);
                continue;
            }
            let r = if open_panel { RADIUS_OPEN.min(b.w.min(b.h) / 2.0) } else { (depth.min(if place.horizontal() { b.h } else { b.w }) * 0.44).min(b.w.min(b.h) / 2.0) };
            // (as round as the setting says; never quite square, or the liquid would read it as one with no corners at all)
            let r = (r * crate::paint::roundness()).max(0.5);
            // (coming out or going back in: as it goes, it sinks past its edge too, further than the neck reaches, so it melts into
            // the edge and nothing is left of it there when it is gone)
            let sink = (1.0 - it.shown.now.clamp(0.0, 1.0)) * (liquid_neck(px) * 1.5 + 4.0);
            let b = match place {
                Place::Edge(Edge::Top) | Place::Corner(Corner::TopLeft | Corner::TopRight) => Bx { y: b.y - sink, ..b },
                Place::Edge(Edge::Left) => Bx { x: b.x - sink, ..b },
                Place::Edge(Edge::Right) => Bx { x: b.x + sink, ..b },
                _ => Bx { y: b.y + sink, ..b },
            };
            // (merged: from past its edge out to its end, less its fillets: the liquid makes its neck)
            let body = match place {
                Place::Edge(Edge::Top) => Bx { x: b.x + f, y: b.y - reach, w: b.w - 2.0 * f, h: b.h + reach },
                Place::Edge(Edge::Bottom) => Bx { x: b.x + f, y: b.y, w: b.w - 2.0 * f, h: b.h + reach },
                Place::Edge(Edge::Left) => Bx { x: b.x - reach, y: b.y + f, w: b.w + reach, h: b.h - 2.0 * f },
                Place::Edge(Edge::Right) => Bx { x: b.x, y: b.y + f, w: b.w + reach, h: b.h - 2.0 * f },
                Place::Corner(c) => {
                    let l = matches!(c, Corner::TopLeft | Corner::BottomLeft);
                    let tp = matches!(c, Corner::TopLeft | Corner::TopRight);
                    Bx { x: if l { b.x - reach } else { b.x + f }, y: if tp { b.y - reach } else { b.y + f }, w: b.w - f + reach, h: b.h - f + reach }
                }
            };
            // (joined to the one beside it: its end there reaches into that one, square, so the two are one shape with no dip
            // where they meet)
            let body = match self.join_now(&it.slot.key).and_then(|pk| self.screens[si].items.get(&pk)).map(|o| o.rest.now()) {
                Some(o) if place.horizontal() => {
                    if o.x + o.w / 2.0 < body.x + body.w / 2.0 { Bx { x: body.x - r - f, w: body.w + r + f, ..body } } else { Bx { w: body.w + r + f, ..body } }
                }
                Some(o) => {
                    if o.y + o.h / 2.0 < body.y + body.h / 2.0 { Bx { y: body.y - r - f, h: body.h + r + f, ..body } } else { Bx { h: body.h + r + f, ..body } }
                }
                None => body,
            };
            shapes.push((body, r));
            shapes_lit.push(lit);
            // (its neck: the curve each end makes into its edge, as its own shape has. One in a corner's own place: none, so it
            // looks as square as it is)
            if let Place::Edge(e) = place {
                shapes.push((b, neck_code(e)));
                shapes_lit.push(lit);
            }
        }
        for (dsi, _, b, r) in &self.pop_drops {
            if *dsi == si {
                shapes.push((*b, *r));
            }
        }
        if let Some((nsi, neck)) = &self.tear_neck {
            if *nsi == si {
                shapes.extend(neck.iter().copied());
            }
        }
        let mut all = shapes.clone();
        all.push((Bx { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, -1.0));
        all.push((Bx { x: glow_line[0], y: glow_line[1], w: glow_line[2], h: (glow_line[3] * 40.0).round() / 40.0 }, -2.0));
        all.extend(shapes_lit.iter().chain(drops_lit.iter()).map(|l| (Bx { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, if *l { -3.0 } else { -4.0 })));
        all.extend(drops.iter().copied());
        if self.screens[si].liquid.is_some() && all == self.screens[si].liquid_shapes {
            return;
        }
        self.screens[si].liquid_shapes = all;
        // (where it is drawn: round what is on the screen, with room for the blur, or the necks)
        let neck = liquid_neck(px);
        let margin = (px * 3.0).max(neck + 4.0);
        let on: Vec<Bx> = shapes.iter().chain(drops.iter()).filter(|(b, r)| *r > 0.0 || b.w < sw || b.h < sh).map(|(b, _)| *b).collect();
        let s = self.screens[si].s;
        let sc_has_any = !on.is_empty();
        if !sc_has_any {
            let sc = &mut self.screens[si];
            if let Some(p) = sc.liquid.take() {
                sc.ov.remove_glass(&p);
            }
            return;
        }
        // (the edges the widgets stand on: the taskbar's when it is kept, not the screen's; nothing drawn past them, over the taskbar)
        let (top, bottom, left, right) = {
            let ar = self.screens[si].area;
            let bar = self.screens[si].bar;
            let edge = |e: Edge| bar.is_some_and(|(be, _)| be == e);
            (if edge(Edge::Top) { 0.0 } else { ar.y }, if edge(Edge::Bottom) { sh } else { ar.y + ar.h }, if edge(Edge::Left) { 0.0 } else { ar.x }, if edge(Edge::Right) { sw } else { ar.x + ar.w })
        };
        let x0 = on.iter().map(|b| b.x).fold(f32::MAX, f32::min).max(0.0) - margin;
        let y0 = on.iter().map(|b| b.y).fold(f32::MAX, f32::min).max(0.0) - margin;
        let x1 = on.iter().map(|b| b.x + b.w).fold(f32::MIN, f32::max).min(sw) + margin;
        let y1 = on.iter().map(|b| b.y + b.h).fold(f32::MIN, f32::max).min(sh) + margin;
        let (x0, y0) = (x0.max(left.max(0.0)), y0.max(top.max(0.0)));
        let (x1, y1) = (x1.min(right.min(sw)), y1.min(bottom.min(sh)));
        if x1 <= x0 || y1 <= y0 {
            let sc = &mut self.screens[si];
            if let Some(p) = sc.liquid.take() {
                sc.ov.remove_glass(&p);
            }
            return;
        }
        let need = (((x1 - x0) * s).ceil().max(1.0) as u32 + 2, ((y1 - y0) * s).ceil().max(1.0) as u32 + 2);
        let g = &self.g;
        let sc = &mut self.screens[si];
        if sc.liquid.is_none() {
            sc.liquid = sc.ov.glass_piece_top(g, need.0 + 64, need.1 + 64).ok();
        }
        let Some(lp) = sc.liquid.as_mut() else { return };
        if need.0 > lp.alloc.0 || need.1 > lp.alloc.1 {
            if let Ok(surface) = g.surface(need.0 + 64, need.1 + 64) {
                unsafe {
                    let _ = lp.visual.SetContent(&surface);
                }
                lp.surface = surface;
                lp.alloc = (need.0 + 64, need.1 + 64);
            }
        }
        let _ = lp.place((x0 * s).round() as i32, (y0 * s).round() as i32);
        let surface = lp.surface.clone();
        let (ox, oy) = ((x0 * s).round(), (y0 * s).round());
        // (on the graphics card, as a metaball: the merged ones and the edges they stand on as one field, the floating ones as another)
        if self.liquid_gpu.is_none() {
            self.liquid_gpu = Some(crate::liquidgpu::LiquidGpu::new(&self.g.d3d));
        }
        if let Some(gpu) = self.liquid_gpu.as_mut().and_then(|g| g.as_mut()) {
            let mut field: Vec<(Bx, f32)> = shapes.iter().filter(|(_, r)| neck_edge(*r).is_none()).copied().collect();
            let mut lit: Vec<bool> = shapes.iter().zip(shapes_lit.iter()).filter(|((_, r), _)| neck_edge(*r).is_none()).map(|(_, l)| *l).collect();
            lit.extend(drops_lit.iter().copied());
            // (each edge with merged ones on it: a slab just past it, as far along as they are, and a little more; the necks are
            // where they run into it)
            let mut spans: Vec<(Edge, f32, f32)> = Vec::new();
            for (b, r) in &shapes {
                let Some(e) = neck_edge(*r) else { continue };
                let (a0, a1) = if matches!(e, Edge::Top | Edge::Bottom) { (b.x, b.x + b.w) } else { (b.y, b.y + b.h) };
                match spans.iter_mut().find(|(x, _, _)| *x == e) {
                    Some(sp) => {
                        sp.1 = sp.1.min(a0);
                        sp.2 = sp.2.max(a1);
                    }
                    None => spans.push((e, a0, a1)),
                }
            }
            let slabs = spans.len();
            for (e, a0, a1) in spans {
                let (a0, a1) = (a0 - 3.0 * neck - 20.0, a1 + 3.0 * neck + 20.0);
                // (past the edge the widgets stand on)
                let slab = match e {
                    Edge::Top => Bx { x: a0, y: top - 43.0, w: a1 - a0, h: 40.0 },
                    Edge::Bottom => Bx { x: a0, y: bottom + 3.0, w: a1 - a0, h: 40.0 },
                    Edge::Left => Bx { x: left - 43.0, y: a0, w: 40.0, h: a1 - a0 },
                    Edge::Right => Bx { x: right + 3.0, y: a0, w: 40.0, h: a1 - a0 },
                };
                field.push((slab, 0.0));
            }
            let edges_at: Vec<(Bx, f32)> = field.split_off(field.len() - slabs);
            if let Some(tex) = gpu.render(&self.g.d3d, need.0, need.1, (ox / s, oy / s), s, neck, &field, &edges_at, &drops, &lit, glow_line) {
                let g = &self.g;
                let _ = g.draw_surface(&surface, |dc| crate::liquidgpu::draw_texture(dc, &tex, need.0, need.1));
                return;
            }
        }
        let g = &self.g;
        let merged = if shapes.is_empty() { None } else { liquid_list(&g.dc, &g.d2d_factory, &shapes, s, ox, oy, px) };
        let floating = if drops.is_empty() { None } else { liquid_list(&g.dc, &g.d2d_factory, &drops, s, ox, oy, px) };
        let _ = g.draw_surface(&surface, |dc| {
            for list in merged.iter().chain(floating.iter()) {
                draw_liquid(dc, list, px * s);
            }
            draw_necks(dc, &g.d2d_factory, &shapes, s, ox, oy, px);
        });
    }

    // ----------------------------------------------------------------------------------------------------- the geometry

    fn closed_box(slot: &Slot) -> Bx {
        Bx { x: slot.x, y: slot.y, w: slot.w, h: slot.h }
    }

    /// How big a widget's panel is, from its content: (along its edge, across it, the content's height). It is laid out with that size, so
    /// what is beside it on its edge moves aside.
    fn panel_size(&self, s: f32, area: Bx, k: &str, place: Place) -> (f32, f32, f32) {
        self.panel_size_as(s, area, k, place, true)
    }

    /// The same; `kept`: the size made by hand over it, when there is one (taller than its content: what fills it gets the rest).
    fn panel_size_as(&self, s: f32, area: Bx, k: &str, place: Place, kept: bool) -> (f32, f32, f32) {
        let Some(w) = self.widgets.get(unpop(k)) else { return (PANEL_W, 100.0, 0.0) };
        let panel = &w.content["panel"];
        let side = matches!(place, Place::Edge(Edge::Left | Edge::Right));
        // (a panel may ask to be narrower: the tray's grid of icons)
        let pw = if side { PANEL_W_SIDE } else { PANEL_W };
        // (the dock's windows: as wide as they need, up to most of the screen)
        // (a panel may ask to be wider too, within reason: a wiki's tables)
        let max_w = if w.content["peek"]["dock"].is_object() && !side { area.w * 0.92 } else if !side { (area.w * 0.5).max(pw).min(640.0) } else { pw };
        let pw = panel["width"].as_f64().map_or(pw, |w| (w as f32).clamp(120.0, max_w));
        let inner_w = pw - 2.0 * PAD;
        let empty = HashSet::new();
        let (open, closed) = self.screens.iter().find_map(|sc| sc.items.get(k)).map(|i| (&i.sections_open, &i.sections_closed)).unwrap_or((&empty, &empty));
        let p = self.painter(s);
        let mut c = Ctx { p: &p, draw: false, hits: Vec::new(), open, closed, thumbs: Vec::new() };
        let head = blocks::head(&mut c, panel, 0.0, 0.0, inner_w);
        let zoom = panel["zoom"].as_f64().unwrap_or(1.0).clamp(0.8, 1.6) as f32;
        let body = c.body_zoomed(panel["body"].as_array().map(|v| v.as_slice()).unwrap_or(&[]), 0.0, 0.0, inner_w, zoom);
        let content_h = head + if body > 0.0 { (if head > 0.0 { 14.0 } else { 0.0 }) + body } else { 0.0 };
        // (on its edge: room over its content for its handle; a window has its own strip)
        let tall = content_h + 2.0 * PAD + 6.0 + if is_pop(k) { 0.0 } else { HANDLE };
        // (a panel may ask to be no taller than so much: it scrolls past that)
        let tall = panel["height"].as_f64().map_or(tall, |h| tall.min((h as f32).max(80.0)));
        let (pw, ph) = if side { (pw, tall.min(area.h * 0.8).max(120.0)) } else { (pw, tall.min(area.h * 0.78).max(2.0 * PAD + 24.0)) };
        let (pw, ph, content_h) = match self.size_kept(k).filter(|_| kept) {
            Some((kw, kh)) => {
                let (mw, mh) = Self::size_max(area, k, place);
                let (kw, kh) = (kw.clamp(size::MIN_W, mw.max(size::MIN_W)), kh.clamp(size::MIN_H, mh.max(size::MIN_H)));
                let fill = (kh - tall).max(0.0);
                self.panel_fill.borrow_mut().insert(k.to_string(), fill);
                (kw, kh, content_h + fill)
            }
            None => {
                if kept {
                    self.panel_fill.borrow_mut().remove(k);
                }
                (pw, ph, content_h)
            }
        };
        if side {
            (ph, pw, content_h)
        } else {
            (pw, ph, content_h)
        }
    }

    /// The shape of an item now (physical px), and its box (logical, in the monitor).
    fn shape_now(&self, si: usize, it: &Item) -> windows::core::Result<(Shape, Bx)> {
        self.shape_with(si, it, true)
    }

    /// Its shape as it would be not swollen (what its contents are laid out in: the swelling moves none of them).
    fn shape_still(&self, si: usize, it: &Item) -> windows::core::Result<(Shape, Bx)> {
        self.shape_with(si, it, false)
    }

    fn shape_with(&self, si: usize, it: &Item, swell: bool) -> windows::core::Result<(Shape, Bx)> {
        let s = self.screens[si].s;
        let m = Metrics::default();
        let t = it.open.now.clamp(0.0, 1.0);
        let b = it.rest.now();
        let f = m.fillet;
        let depth = if it.slot.place.horizontal() { m.depth_h } else { m.depth_v };
        let fac = &self.g.d2d_factory;
        // (being pulled off its edge: a drop of its own, round all round)
        if self.tear_pull > 0.5 && self.tear.as_ref().is_some_and(|(ts, tk, ..)| *ts == si && *tk == it.slot.key) {
            let fb = pop::tear_box(it.slot.place, it.rest.now());
            return Ok((shape::floating(fac, fb.w * s, fb.h * s, RADIUS_OPEN * s)?, fb));
        }
        if is_pop(&it.slot.key) {
            let b = it.rest.now();
            return Ok((shape::floating(fac, b.w * s, b.h * s, RADIUS_OPEN * s)?, b));
        }
        if let Some(place) = it.drag_place.map(|p| drag_look(p, it.drag_head, it.drag_down)) {
            // (dragged: floating where it is held, or squished into the edge it is near)
            let b = Bx { w: b.w.max(2.0 * f + 4.0), h: b.h.max(8.0), ..b };
            if it.drag_float {
                let r = (if place.horizontal() { b.h } else { b.w }).min(depth) / 2.0;
                return Ok((shape::floating(fac, b.w * s, b.h * s, r * s)?, b));
            }
            let radius = depth.min(if place.horizontal() { b.h } else { b.w }) * 0.44;
            let shape = match place {
                Place::Edge(e @ (Edge::Top | Edge::Bottom)) => shape::notch(fac, e, (b.w - 2.0 * f) * s, b.h * s, radius * s, f * s)?,
                Place::Edge(e) => shape::notch(fac, e, (b.h - 2.0 * f) * s, b.w * s, radius * s, f * s)?,
                Place::Corner(c) => shape::corner(fac, c, (b.w - f) * s, (b.h - f) * s, radius * s, f * s)?,
            };
            return Ok((shape, b));
        }
        // (coming out: it grows out of its edge, out of it first and then along it, a little too far, and settles; going, it shrinks
        // back into it. While it grows it is round, like a drop the edge lets go; settled, it squares off)
        let g = it.shown.now.clamp(-0.1, 1.25).max(0.0);
        let across_k = g;
        let along_k = 0.5 + 0.5 * g;
        let b = if (g - 1.0).abs() > 0.001 {
            let grow = |len: f32, k: f32| (len * k).max(0.0);
            match it.slot.place {
                Place::Edge(Edge::Top) => { let (w, h) = (grow(b.w, along_k), grow(b.h, across_k)); Bx { x: b.x + (b.w - w) / 2.0, y: b.y, w, h } }
                Place::Edge(Edge::Bottom) => { let (w, h) = (grow(b.w, along_k), grow(b.h, across_k)); Bx { x: b.x + (b.w - w) / 2.0, y: b.y + b.h - h, w, h } }
                Place::Edge(Edge::Left) => { let (w, h) = (grow(b.w, across_k), grow(b.h, along_k)); Bx { x: b.x, y: b.y + (b.h - h) / 2.0, w, h } }
                Place::Edge(Edge::Right) => { let (w, h) = (grow(b.w, across_k), grow(b.h, along_k)); Bx { x: b.x + b.w - w, y: b.y + (b.h - h) / 2.0, w, h } }
                Place::Corner(c) => {
                    let (w, h) = (grow(b.w, along_k), grow(b.h, across_k));
                    let l = matches!(c, Corner::TopLeft | Corner::BottomLeft);
                    let tp = matches!(c, Corner::TopLeft | Corner::TopRight);
                    Bx { x: if l { b.x } else { b.x + b.w - w }, y: if tp { b.y } else { b.y + b.h - h }, w, h }
                }
            }
        } else {
            b
        };
        // (the mouse on it: swollen a little, out of its edge and along it; none of it once its panel is open)
        let sw = if swell { it.swell.now.clamp(-0.3, 1.4) * (1.0 - t) } else { 0.0 };
        let b = if sw.abs() > 0.001 && it.floating {
            // (floating: swollen in place, about its own middle, every way alike)
            let (out, along) = (SWELL_OUT * sw, SWELL_ALONG * sw);
            if it.slot.place.horizontal() || matches!(it.slot.place, Place::Corner(_)) {
                Bx { x: b.x - along / 2.0, y: b.y - out / 2.0, w: b.w + along, h: b.h + out }
            } else {
                Bx { x: b.x - out / 2.0, y: b.y - along / 2.0, w: b.w + out, h: b.h + along }
            }
        } else if sw.abs() > 0.001 {
            // (merged into its edge: swollen in place, along its edge only; it does not rise out of it)
            let (out, along) = (0.0, SWELL_ALONG * sw);
            match it.slot.place {
                Place::Edge(Edge::Top) => Bx { x: b.x - along / 2.0, w: b.w + along, h: b.h + out, ..b },
                Place::Edge(Edge::Bottom) => Bx { x: b.x - along / 2.0, y: b.y - out, w: b.w + along, h: b.h + out },
                Place::Edge(Edge::Left) => Bx { y: b.y - along / 2.0, w: b.w + out, h: b.h + along, ..b },
                Place::Edge(Edge::Right) => Bx { x: b.x - out, y: b.y - along / 2.0, w: b.w + out, h: b.h + along },
                Place::Corner(c) => {
                    let l = matches!(c, Corner::TopLeft | Corner::BottomLeft);
                    let tp = matches!(c, Corner::TopLeft | Corner::TopRight);
                    Bx { x: if l { b.x } else { b.x - along }, y: if tp { b.y } else { b.y - out }, w: b.w + along, h: b.h + out }
                }
            }
        } else {
            b
        };
        // (how round, while it grows: all of it at first, its own at rest)
        let drop = (1.0 - g).clamp(0.0, 1.0);
        if it.floating {
            // (off the edge: the box less its fillets along the edge, and the gap to the edge)
            let o = self.float_off();
            let fb = match it.slot.place {
                Place::Edge(Edge::Top) => Bx { x: b.x + f, y: b.y + o, w: b.w - 2.0 * f, h: b.h - o },
                Place::Edge(Edge::Bottom) => Bx { x: b.x + f, y: b.y, w: b.w - 2.0 * f, h: b.h - o },
                Place::Edge(Edge::Left) => Bx { x: b.x + o, y: b.y + f, w: b.w - o, h: b.h - 2.0 * f },
                Place::Edge(Edge::Right) => Bx { x: b.x, y: b.y + f, w: b.w - o, h: b.h - 2.0 * f },
                Place::Corner(c) => {
                    let (l, tp) = (matches!(c, Corner::TopLeft | Corner::BottomLeft), matches!(c, Corner::TopLeft | Corner::TopRight));
                    Bx { x: if l { b.x + o } else { b.x + f }, y: if tp { b.y + o } else { b.y + f }, w: b.w - o - f, h: b.h - o - f }
                }
            };
            // (gone: nothing left of it, not even a dot)
            let fb = Bx { w: fb.w.max(8.0 * g.min(1.0)).max(0.5), h: fb.h.max(8.0 * g.min(1.0)).max(0.5), ..fb };
            // (its window out: a shallow stump in the middle of where it floated)
            // (its icon alone: a square one)
            let fb = if t < 0.01 && !is_pop(&it.slot.key) && self.popped(&it.slot.key) {
                let square = self.icon_alone(&it.slot.key);
                let fb = if it.slot.place.horizontal() || matches!(it.slot.place, Place::Corner(_)) {
                    let h = STUMP.min(fb.h);
                    Bx { y: fb.y + (fb.h - h) / 2.0, h, ..fb }
                } else {
                    let w = STUMP.min(fb.w);
                    Bx { x: fb.x + (fb.w - w) / 2.0, w, ..fb }
                };
                if square {
                    let z = STUMP.min(fb.w).min(fb.h);
                    Bx { x: fb.x + (fb.w - z) / 2.0, y: fb.y + (fb.h - z) / 2.0, w: z, h: z }
                } else {
                    fb
                }
            } else {
                fb
            };
            let closed_r = (if it.slot.place.horizontal() { fb.h } else { fb.w }).min(depth) / 2.0;
            let r = closed_r + (RADIUS_OPEN - closed_r) * t;
            return Ok((shape::floating(fac, fb.w * s, fb.h * s, r * s)?, fb));
        }
        let own = depth.min((if it.slot.place.horizontal() { it.slot.h } else { it.slot.w }).max(1.0));
        let radius = (own * 0.44) + (RADIUS_OPEN - own * 0.44) * t;
        // (its window out, its icon alone: a square stump, as long along its edge as it is deep)
        let b = if t < 0.01 && !is_pop(&it.slot.key) && self.popped(&it.slot.key) && self.icon_alone(&it.slot.key) {
            match it.slot.place {
                Place::Edge(Edge::Top | Edge::Bottom) => {
                    let w = (b.h + 2.0 * f).min(b.w);
                    Bx { x: b.x + (b.w - w) / 2.0, w, ..b }
                }
                Place::Edge(_) => {
                    let h = (b.w + 2.0 * f).min(b.h);
                    Bx { y: b.y + (b.h - h) / 2.0, h, ..b }
                }
                Place::Corner(_) => b,
            }
        } else {
            b
        };
        // (long enough along its edge for its two fillets; as shallow as it is out of it, nothing left when it is gone)
        let b = if it.slot.place.horizontal() || matches!(it.slot.place, Place::Corner(_)) {
            Bx { w: b.w.max(2.0 * f + 4.0), h: b.h.max(4.0 * g.min(1.0)).max(0.5), ..b }
        } else {
            Bx { w: b.w.max(4.0 * g.min(1.0)).max(0.5), h: b.h.max(2.0 * f + 4.0), ..b }
        };
        let round = if it.slot.place.horizontal() { b.h.min((b.w - 2.0 * f) / 2.0) } else { b.w.min((b.h - 2.0 * f) / 2.0) };
        let radius = if drop > 0.0 { radius + (round.max(radius) - radius) * drop } else { radius };
        let b = bleed(it.slot.place, b, s);
        let shape = match it.slot.place {
            Place::Edge(e @ (Edge::Top | Edge::Bottom)) => shape::notch(fac, e, (b.w - 2.0 * f) * s, b.h * s, radius * s, f * s)?,
            Place::Edge(e) => shape::notch(fac, e, (b.h - 2.0 * f) * s, b.w * s, radius * s, f * s)?,
            Place::Corner(c) => shape::corner(fac, c, (b.w - f) * s, (b.h - f) * s, radius * s, f * s)?,
        };
        Ok((shape, b))
    }

    /// The shape of a widget at rest in a box (shut), merged into its place or floating off it.
    fn rest_shape(&self, s: f32, place: Place, b: Bx, floating: bool, round: Option<f32>) -> windows::core::Result<(Shape, Bx)> {
        let m = Metrics::default();
        let f = m.fillet;
        let depth = if place.horizontal() { m.depth_h } else { m.depth_v };
        let fac = &self.g.d2d_factory;
        if floating {
            let o = self.float_off();
            let fb = match place {
                Place::Edge(Edge::Top) => Bx { x: b.x + f, y: b.y + o, w: b.w - 2.0 * f, h: b.h - o },
                Place::Edge(Edge::Bottom) => Bx { x: b.x + f, y: b.y, w: b.w - 2.0 * f, h: b.h - o },
                Place::Edge(Edge::Left) => Bx { x: b.x + o, y: b.y + f, w: b.w - o, h: b.h - 2.0 * f },
                Place::Edge(Edge::Right) => Bx { x: b.x, y: b.y + f, w: b.w - o, h: b.h - 2.0 * f },
                Place::Corner(c) => {
                    let (l, tp) = (matches!(c, Corner::TopLeft | Corner::BottomLeft), matches!(c, Corner::TopLeft | Corner::TopRight));
                    Bx { x: if l { b.x + o } else { b.x + f }, y: if tp { b.y + o } else { b.y + f }, w: b.w - o - f, h: b.h - o - f }
                }
            };
            let fb = Bx { w: fb.w.max(8.0), h: fb.h.max(8.0), ..fb };
            let r = round.unwrap_or((if place.horizontal() { fb.h } else { fb.w }).min(depth) / 2.0);
            return Ok((shape::floating(fac, fb.w * s, fb.h * s, r * s)?, fb));
        }
        let radius = round.unwrap_or(depth.min(if place.horizontal() { b.h } else { b.w }) * 0.44);
        let b = Bx { w: b.w.max(2.0 * f + 4.0), h: b.h.max(4.0), ..b };
        let b = bleed(place, b, s);
        let shape = match place {
            Place::Edge(e @ (Edge::Top | Edge::Bottom)) => shape::notch(fac, e, (b.w - 2.0 * f) * s, b.h * s, radius * s, f * s)?,
            Place::Edge(e) => shape::notch(fac, e, (b.h - 2.0 * f) * s, b.w * s, radius * s, f * s)?,
            Place::Corner(c) => shape::corner(fac, c, (b.w - f) * s, (b.h - f) * s, radius * s, f * s)?,
        };
        Ok((shape, b))
    }

    /// While a widget is held: its shadow, springing to where it would go if let go now (made as it is picked up, gone as it is
    /// dropped). True when it changed.
    fn step_ghost(&mut self, dt: f32) -> bool {
        let target = self.drag.as_ref().and_then(|(si, k, _, _)| {
            let it = self.screens.get(*si)?.items.get(k)?;
            Some((*si, it.slot.clone(), it.drag_float))
        });
        let Some((si, slot, float)) = target else {
            if let Some(g) = self.ghost.take() {
                if let Some(sc) = self.screens.get(g.si) {
                    let _ = sc.ov.remove(&g.piece);
                }
                return true;
            }
            return false;
        };
        let to = Self::closed_box(&slot);
        if self.ghost.as_ref().is_some_and(|g| g.si != si) {
            if let Some(g) = self.ghost.take() {
                if let Some(sc) = self.screens.get(g.si) {
                    let _ = sc.ov.remove(&g.piece);
                }
            }
        }
        let s = self.screens[si].s;
        if self.ghost.is_none() {
            let (pw, ph) = ((to.w * s * 1.2).ceil() as u32 + 4, (to.h * s * 1.2).ceil() as u32 + 4);
            let Ok(piece) = self.screens[si].ov.piece_below(&self.g, pw, ph) else { return false };
            let rest = SpringBox { x: Spring::at(to.x), y: Spring::at(to.y), w: Spring::at(to.w), h: Spring::at(to.h) };
            self.ghost = Some(Ghost { si, piece, rest, drawn: None });
        }
        let mut g = self.ghost.take().unwrap();
        g.rest.set(to);
        g.rest.step(dt);
        let b = g.rest.now();
        let now = Some((b, slot.place, float));
        let changed = g.drawn != now;
        if changed {
            g.drawn = now;
            if let Ok((shape, sb)) = self.rest_shape(s, slot.place, b, float, None) {
                let need = (shape.w.ceil() as u32 + 2, shape.h.ceil() as u32 + 2);
                if need.0 > g.piece.alloc.0 || need.1 > g.piece.alloc.1 {
                    let grow = ((need.0 as f32 * 1.2) as u32 + 8, (need.1 as f32 * 1.2) as u32 + 8);
                    if let Ok(surface) = self.g.surface(grow.0, grow.1) {
                        unsafe {
                            let _ = g.piece.visual.SetContent(&surface);
                        }
                        g.piece.surface = surface;
                        g.piece.alloc = grow;
                    }
                }
                let _ = g.piece.place((sb.x * s).round() as i32, (sb.y * s).round() as i32);
                let surface = g.piece.surface.clone();
                let _ = self.g.draw_surface(&surface, |dc| {
                    // (no line round it: a dark glass, a little lit)
                    let _ = fill_shape(dc, &shape, crate::paint::Color(0.0, 0.0, 0.0, 0.35));
                    let _ = fill_shape(dc, &shape, crate::paint::Color(1.0, 1.0, 1.0, 0.12));
                });
            }
        }
        self.ghost = Some(g);
        changed
    }

    /// Draw one item as it is now: its shape, and its peek or its panel.
    fn render(&mut self, si: usize, k: &str) {
        self.drew = true;
        // (one tucked into its edge changes nothing of the liquid: it is not drawn again for it)
        let tucked = self.screens.get(si).and_then(|sc| sc.items.get(k)).is_some_and(|it| tucked(it, k));
        if let Some(sc) = self.screens.get_mut(si).filter(|_| !tucked) {
            sc.liquid_dirty = true;
        }
        let t0 = Instant::now();
        self.render_inner(si, k);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let e = self.prof.entry(format!("render {k}")).or_default();
        e.0 += 1;
        e.1 += ms;
        let e = self.render_cost.entry(k.to_string()).or_default();
        e.0 += 1;
        e.1 += ms;
    }

    fn render_inner(&mut self, si: usize, k: &str) {
        // (gone back into its edge, and staying there: nothing of it drawn, not even a sliver of its shape)
        if let Some(it) = self.screens[si].items.get(k) {
            if tucked(it, k) {
                // (its surface let go, for one of a pixel, empty: drawn again, as big as it needs, as it comes back out)
                if it.piece.alloc != (1, 1) {
                    if let Ok(tiny) = self.g.surface(1, 1) {
                        let _ = self.g.draw_surface(&tiny, |_| {});
                        let it = self.screens[si].items.get_mut(k).unwrap();
                        unsafe {
                            let _ = it.piece.visual.SetContent(&tiny);
                        }
                        it.piece.surface = tiny;
                        it.piece.alloc = (1, 1);
                    }
                }
                return;
            }
        }
        let Some(w) = self.widgets.get(unpop(k)) else { return };
        let mut content = w.content.clone();
        content["peek"] = self.peek_of(k);
        // (a subtitle's words come in as they are typed: what is new of it, a few letters a frame, quicker the more there is)
        let mut typing = false;
        if let Some(full) = content["peek"]["subtitle"]["text"].as_str().map(str::to_string) {
            let now = Instant::now();
            let e = self.typed.entry(k.to_string()).or_insert((String::new(), 0.0, now));
            if e.0 != full {
                let common = e.0.chars().zip(full.chars()).take_while(|(a, b)| a == b).count();
                e.1 = e.1.min(common as f32);
                e.0 = full.clone();
            }
            let total = full.chars().count() as f32;
            let dt = now.duration_since(e.2).as_secs_f32().min(0.1);
            e.2 = now;
            let rate = ((total - e.1) / 0.3).max(45.0);
            e.1 = (e.1 + rate * dt).min(total);
            typing = e.1 < total;
            let shown: String = full.chars().take(e.1 as usize).collect();
            content["peek"]["subtitle"]["text"] = json!(shown);
        }
        if content["peek"]["dock"].is_object() {
            self.dock_live(&mut content);
        }
        // (its window out: a stump, nothing on it but its dot)
        let stump = !is_pop(k) && self.popped(k);
        if stump {
            content["peek"] = json!({});
        }
        let s = self.screens[si].s;
        let (shape, b) = {
            let Some(it) = self.screens[si].items.get(k) else { return };
            match self.shape_now(si, it) {
                Ok(x) => x,
                Err(_) => return,
            }
        };
        // (swollen under the mouse: its ground swells, its contents stay where they were, laid out in its shape as it was)
        let still = self.screens[si].items.get(k).filter(|it| it.swell.now.abs() > 0.001 && it.open.now < 0.01).and_then(|it| self.shape_still(si, it).ok());
        // (its contents in the middle of its swollen shape: along its edge and out of it alike)
        let swell_off = still.as_ref().map_or((0.0, 0.0), |(_, sb)| ((b.w - sb.w) / 2.0, (b.h - sb.h) / 2.0));
        // (a hidden one's glow fades with it)
        let shown = self.screens[si].items.get(k).map_or(0.0, |i| i.shown.now.clamp(0.0, 1.0));
        let glow = self.glow_of(k).filter(|_| shown > 0.01 && !is_pop(k)).map(|(c, st, moves)| (c, st * shown, moves));
        // (glowing: over the ones beside it, so its border goes all the way round it where they join; once, as it begins)
        if glow.is_some() {
            if self.glow_raised.insert(k.to_string()) {
                if let Some(v) = self.screens[si].items.get(k).map(|it| it.piece.visual.clone()) {
                    self.screens[si].ov.to_front(&v);
                }
            }
        } else {
            self.glow_raised.remove(k);
        }
        // (merged into its edge: the liquid makes its neck there, so its border stops short of the edge, fillets and all)
        let liquid_line = self.liquid_gpu.as_ref().is_some_and(|g| g.is_some()) && self.screens[si].items.get(k).is_some_and(|it| self.in_liquid(it));
        // (in the liquid on the graphics card: the liquid draws the line, round its own edge)
        let glow_border = if liquid_line { None } else { glow };
        let border_edge = self.screens[si].items.get(k).and_then(|it| match it.slot.place {
            Place::Edge(e) if !it.floating && !is_pop(k) => Some(e),
            _ => None,
        });
        self.render_glow(si, k, &shape, b, glow);
        // (a surface big enough: it grows as the panel opens, and shrinks back when it is shut)
        let need = (shape.w.ceil() as u32 + 2, shape.h.ceil() as u32 + 2);
        {
            let sc = &mut self.screens[si];
            let it = sc.items.get_mut(k).unwrap();
            let shut = it.open.now == 0.0 && !it.open.moving() && !it.rest.moving();
            if need.0 > it.piece.alloc.0 || need.1 > it.piece.alloc.1 || (shut && (it.piece.alloc.0 > need.0 + 8 || it.piece.alloc.1 > need.1 + 8)) {
                let grow = if shut { need } else { ((need.0 as f32 * 1.12) as u32 + 8, (need.1 as f32 * 1.12) as u32 + 8) };
                if let Ok(surface) = self.g.surface(grow.0, grow.1) {
                    unsafe {
                        let _ = it.piece.visual.SetContent(&surface);
                    }
                    it.piece.surface = surface;
                    it.piece.alloc = grow;
                }
            }
            let _ = it.piece.place((b.x * s).round() as i32, (b.y * s).round() as i32);
        }
        let it = self.screens[si].items.get(k).unwrap();
        let t = it.open.now.clamp(0.0, 1.0);
        let mut slot = it.slot.clone();
        if let Some(place) = it.drag_place {
            // (held beside a corner, in its row or column: drawn as one of them)
            slot.place = drag_look(place, it.drag_head, it.drag_down);
            slot.w = b.w;
            slot.h = b.h;
            slot.dot = false;
        }
        let (open_set, closed_set, scroll, content_h) = (it.sections_open.clone(), it.sections_closed.clone(), it.scroll, it.content_h);
        // (scrolled back: is there more since?)
        if self.wheeled.contains_key(k) && self.seen_h.get(k).is_some_and(|h| content_h > h + 1.0) {
            content["\u{0}newer"] = json!(true);
        }
        let bar = self.scrollbar(si, k);
        // (in the liquid, its ground is the liquid's; but while it glows it paints its own as well (the same black): the liquid may lag
        // a piece that moves or comes out, and the glow under it would show through)
        if self.in_liquid(it) && glow.is_none() {
            content["\u{0}liquid"] = json!(true);
        }
        // (a window: its background drawn here, as solid as it is set)
        let pop_fill = is_pop(k).then(|| match self.pop_bg(k) {
            "see" => 0.55,
            "none" => 0.0,
            _ => 1.0,
        });
        if pop_fill.is_some() {
            content["\u{0}liquid"] = json!(true);
        }
        // (its ground: black, or the colour its widget asks for ("#rrggbb": a yellow note))
        let pop_ground = content["window"]["ground"].as_str().filter(|g| g.len() == 7 && g.starts_with('#') && g[1..].chars().all(|c| c.is_ascii_hexdigit())).map(crate::paint::Color::hex).unwrap_or(FACE);
        // (swollen: its ground drawn here, swollen; its contents over it, not)
        let swell_ground = still.is_some() && content["\u{0}liquid"] != true;
        if still.is_some() {
            content["\u{0}liquid"] = json!(true);
        }
        // (its window out: a dot in the middle of its stump)
        let dot = stump.then(|| {
            let bd = shape.body;
            // (on the pixel grid: one crisp dot, not one smeared over two rows)
            let snap = |v: f32| ((v * s).floor() + 0.5) / s;
            (snap(bd.x / s + bd.w / s / 2.0), snap(bd.y / s + bd.h / s / 2.0), 0.0, false)
        });
        let fade = ((it.shown.now - 0.8) / 0.2).clamp(0.0, 1.0);
        let floating = if it.drag_place.is_some() { it.drag_float } else { it.floating };
        // (the aura is taken out while the piece is drawn, and put back)
        let mut aura_own = std::mem::replace(&mut self.aura, crate::aura::Aura::new());
        let p = self.painter(s);
        let mut hits = Vec::new();
        let mut buttons = Vec::new();
        let mut alive = false;
        let mut boxes = Vec::new();
        let surface = it.piece.surface.clone();
        let is_eyes = content["peek"]["face"] == "eyes";
        let w_stick = content["panel"]["stick"] == "bottom";
        // (open on its edge: room for its handle over its content, the handle shown while the mouse is on it)
        let handle = if is_pop(k) { 0.0 } else { HANDLE };
        content["\u{0}top"] = json!(handle);
        let pulled = self.tear.as_ref().is_some_and(|(ts, tk, ..)| *ts == si && tk == k);
        if handle > 0.0 && (pulled || self.hover.as_ref().is_some_and(|(hs, hk)| *hs == si && hk == k)) {
            let since = self.screens[si].items.get(k).and_then(|it| it.hover_since).map_or(1.0, |t| t.elapsed().as_secs_f32());
            content["\u{0}handle"] = json!(if pulled { 1.0 } else { (since / 0.12).min(1.0) });
            content["\u{0}handle_w"] = json!(if pulled { (32.0 - self.tear_pull * 0.5).max(12.0) } else { 32.0 });
        }
        let held_here = self.hold.as_ref().filter(|h| h.si == si && h.key.as_deref() == Some(k)).map(|h| (h.rect, 0.0));
        let idle_faded = self.edit.is_some() && content["idle"] == true;
        let eyes_place = slot.place;
        let lit = self.aura_target().is_some_and(|(asi, ak)| asi == si && ak == k);
        let now = self.start.elapsed().as_secs_f32() * 1000.0;
        let audio = self.director.audio.clone();
        let (eyes, aura) = (&self.eyes, std::cell::RefCell::new(&mut aura_own));
        let mut aura_alive = false;
        // (a window: its panel under the strip along its top, which holds its grip and switch while the mouse is on it)
        let top = if is_pop(k) { POP_TOP } else { 0.0 };
        let inner = if top > 0.0 { shape::floating(&self.g.d2d_factory, shape.w, (shape.h - top * s).max(8.0), RADIUS_OPEN * s).ok() } else { None };
        // (a window going back: its content fading)
        let content_a = self.pops.get(k).map_or(1.0, |m| m.content_a);
        let marks = self.pops.get(k).map(|m| m.strip_a * m.content_a).filter(|a| top > 0.0 && *a > 0.01).map(|a| (pop::bg_index(self.pop_bg(k)), a));
        let vis_h = inner.as_ref().map_or(shape.body.h, |i| i.body.h);
        // (words too long for their room scroll: whether any did, from here)
        let _ = crate::draw::take_marquee();
        // (the mouse on its open panel: where, in the panel's own space, for its ghost buttons)
        let hover_at = (t > 0.5 && self.hover.as_ref().is_some_and(|(hs, hk)| *hs == si && hk == k) && self.mouse.0 == si).then(|| (self.mouse.1 - b.x, self.mouse.2 - b.y - top));
        let drawn = self.g.draw_surface(&surface, |dc| {
            let mut under = |p: &Painter, body: Rect| {
                if lit {
                    let c = (body.x + body.w / 2.0, body.y + body.h / 2.0);
                    aura_alive = aura.borrow_mut().paint(p, body.x, body.y, body.w, body.h, now, 1.0 / 30.0, &audio, c);
                }
            };
            let mut over = |p: &Painter, body: Rect| {
                if is_eyes {
                    // (on a side edge the face turns: one eye over the other, peeking in)
                    // (on a side edge the face turns: its head toward the edge, one eye over the other)
                    let turn = eyes_turn(eyes_place);
                    let sc = (if turn != 0.0 { body.w } else { body.h } / 32.0).clamp(0.8, 1.6);
                    eyes.paint(p, body.x + body.w / 2.0, body.y + body.h / 2.0, sc, turn, &audio, now);
                }
            };
            // (idle, shown only while editing: faded)
            let faded = idle_faded;
            if let Some(a) = pop_fill.filter(|a| *a > 0.0) {
                let _ = fill_shape(dc, &shape, pop_ground.alpha(a));
            }
            if swell_ground {
                let _ = fill_shape(dc, &shape, FACE);
            }
            // (a window that asks to be see-through (a map over a game): what is behind it shows through)
            let see = if is_pop(k) && content["see_through"] == true { 0.7 } else { 1.0 };
            let fill = self.panel_fill.borrow().get(k).copied().unwrap_or(0.0);
            let lit = self.lit_sides(si, k);
            with_opacity(dc, &self.g, if faded { 0.5 } else { 1.0 } * content_a * see, || {
                blocks::set_hover(hover_at);
                blocks::set_fill(fill);
                let mut base = windows::Foundation::Numerics::Matrix3x2::default();
                unsafe {
                    dc.GetTransform(&mut base);
                    if top > 0.0 || still.is_some() {
                        dc.SetTransform(&(windows::Foundation::Numerics::Matrix3x2::translation(swell_off.0 * s, (top + swell_off.1) * s) * base));
                    }
                }
                let laid = inner.as_ref().or(still.as_ref().map(|(sh, _)| sh)).unwrap_or(&shape);
                (hits, buttons, alive, boxes) = paint_piece(&p, &self.g, dc, laid, &slot, &content, t, &open_set, &closed_set, scroll, content_h, bar, fade, floating, &mut under, &mut over);
                unsafe {
                    dc.SetTransform(&base);
                }
                blocks::set_hover(None);
                blocks::set_fill(0.0);
            });
            // (a side the mouse is near, to resize it by: lit faintly)
            if lit != 0 && t > 0.9 {
                let bd = shape.body;
                let (x0, y0, w0, h0) = (bd.x / s, bd.y / s + top, bd.w / s, bd.h / s - top);
                let c = draw::accent().alpha(0.5);
                let r = RADIUS_OPEN;
                if lit & size::LEFT != 0 {
                    p.round(Rect::new(x0 + 1.0, y0 + r, 2.0, h0 - 2.0 * r), 1.0, c);
                }
                if lit & size::RIGHT != 0 {
                    p.round(Rect::new(x0 + w0 - 3.0, y0 + r, 2.0, h0 - 2.0 * r), 1.0, c);
                }
                if lit & size::TOP != 0 {
                    p.round(Rect::new(x0 + r, y0 + 1.0, w0 - 2.0 * r, 2.0), 1.0, c);
                }
                if lit & size::BOTTOM != 0 {
                    p.round(Rect::new(x0 + r, y0 + h0 - 3.0, w0 - 2.0 * r, 2.0), 1.0, c);
                }
            }
            if let Some((cx, cy, len, along)) = dot {
                if len > 0.0 {
                    let r = if along { Rect::new(cx - len / 2.0, cy - 1.5, len, 3.0) } else { Rect::new(cx - 1.5, cy - len / 2.0, 3.0, len) };
                    p.round(r, 1.5, draw::accent());
                } else {
                    p.circle(cx, cy, 2.5, draw::accent());
                }
            }
            if let Some((now, ma)) = marks {
                with_opacity(dc, &self.g, ma, || pop::paint_marks(&p, shape.w / s, now));
            }
            // (held: its ring filling round it)
            if let Some((r, frac)) = held_here {
                let red = crate::paint::Color(0.91, 0.42, 0.42, 1.0);
                let rr = r.w.min(r.h) / 2.0;
                p.ring(r.x + r.w / 2.0, r.y + r.h / 2.0, rr + 2.0, 2.5, frac, 0.0, red, crate::paint::Color(0.0, 0.0, 0.0, 0.0));
            }
            // (glowing: its own border bright, brightest as it comes out, then settling)
            if let Some((c, st, _)) = glow_border {
                let a = ((st - 0.5) / 1.5).clamp(0.3, 1.0) * fade.max(0.0);
                if let Ok(br) = crate::paint::brush(dc, c.alpha(a)) {
                    let band = Metrics::default().fillet * s + 2.0;
                    let (w, h) = (shape.w, shape.h);
                    let clip = border_edge.map(|e| {
                        let (l, t, r, b) = match e {
                            Edge::Top => (0.0, band, w, h),
                            Edge::Bottom => (0.0, 0.0, w, h - band),
                            Edge::Left => (band, 0.0, w, h),
                            Edge::Right => (0.0, 0.0, w - band, h),
                        };
                        windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: l, top: t, right: r, bottom: b }
                    });
                    unsafe {
                        if let Some(c) = &clip {
                            dc.PushAxisAlignedClip(c, windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
                        }
                        match border_edge {
                            // (merged into its edge: round its body only, from its edge out; its fillets are the liquid's neck, and a
                            // widget joined to it lies over its own; the side on the edge is left open by the clip)
                            Some(e) => {
                                let bd = shape.body;
                                let ext = band + 6.0;
                                let (x, y, w, h) = match e {
                                    Edge::Top => (bd.x, bd.y - ext, bd.w, bd.h + ext),
                                    Edge::Bottom => (bd.x, bd.y, bd.w, bd.h + ext),
                                    Edge::Left => (bd.x - ext, bd.y, bd.w + ext, bd.h),
                                    Edge::Right => (bd.x, bd.y, bd.w + ext, bd.h),
                                };
                                let half = 1.5 * s;
                                let dep = if matches!(e, Edge::Top | Edge::Bottom) { bd.h } else { bd.w };
                                let along = if matches!(e, Edge::Top | Edge::Bottom) { bd.w } else { bd.h };
                                let r = ((dep * 0.44).min(along / 2.0)) * crate::paint::roundness() - half;
                                let rr = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                                    rect: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: x + half, top: y + half, right: x + w - half, bottom: y + h - half },
                                    radiusX: r.max(0.0),
                                    radiusY: r.max(0.0),
                                };
                                dc.DrawRoundedRectangle(&rr, &br, 3.0 * s, None);
                            }
                            None => dc.DrawGeometry(&shape.geometry, &br, 3.0 * s, None),
                        }
                        if clip.is_some() {
                            dc.PopAxisAlignedClip();
                        }
                    }
                }
            }
        });
        drop(aura);
        self.aura = aura_own;
        if lit {
            self.aura_alive = aura_alive;
        }
        let marquee = crate::draw::take_marquee();
        let alive = alive || typing || marquee || is_eyes || (lit && aura_alive) || glow.is_some_and(|(_, _, moves)| moves);
        // (what can be pressed in a window's panel: under its strip)
        if top > 0.0 {
            for h in &mut hits {
                h.rect.y += top;
            }
            for (r, _) in &mut buttons {
                r.y += top;
            }
            for (_, r) in &mut boxes {
                r.y += top;
            }
        }
        // (swollen: its contents were laid out where they were before; what can be pressed, with them)
        if still.is_some() {
            let (dx, dy) = swell_off;
            for h in &mut hits {
                h.rect.x += dx;
                h.rect.y += dy;
            }
            for (r, _) in &mut buttons {
                r.x += dx;
                r.y += dy;
            }
            for (_, r) in &mut boxes {
                r.x += dx;
                r.y += dy;
            }
        }
        // (the piece's offset: hits are in the piece's own space, the box in the monitor's)
        let (s_now, mon) = (self.screens[si].s, self.screens[si].ov.monitor.rect);
        let it = self.screens[si].items.get_mut(k).unwrap();
        it.hits = hits;
        // (the stage's live picture: a box among the peek's, not a button)
        let (pictures, buttons): (Vec<(Rect, String)>, Vec<(Rect, String)>) = buttons.into_iter().partition(|(_, a)| a.starts_with("thumb:"));
        it.buttons = buttons;
        it.alive = alive;
        // (not drawn: its surface lost or broken (a graphics card switched, a display changed); it was left blank. A new surface, and
        // drawn again on the next tick, as hiding and showing it again would)
        if let Err(e) = &drawn {
            note_render_error(k, e);
            it.piece.alloc = (0, 0);
            it.alive = true;
        }
        // (being typed into: its box goes where its field is now)
        if let Some(ib) = self.input.as_mut().filter(|i| i.shown && i.key == k) {
            if let Some(h) = it.hits.iter().find(|h| matches!(&h.press, Press::Field(a, ..) if *a == ib.action)) {
                let (s, m) = (s_now, mon);
                let r = h.rect;
                ib.follow(RECT { left: m.left + ((b.x + r.x) * s).round() as i32, top: m.top + ((b.y + r.y) * s).round() as i32, right: m.left + ((b.x + r.x + r.w) * s).round() as i32, bottom: m.top + ((b.y + r.y + r.h) * s).round() as i32 });
            } else {
                // (its field gone from its panel (the panel changed, or shut): the box goes too, what was typed with it)
                ib.close();
                ib.text.clear();
                it.pinned = false;
            }
        }
        it.thumb_boxes = if t < 0.35 { pictures.into_iter().filter_map(|(r, a)| a[6..].parse::<isize>().ok().map(|h| (h, r))).collect() } else { boxes };
        it.visible = vis_h / s - 2.0 * PAD - handle;
        let stick = w_stick && !self.wheeled.contains_key(k);
        if stick {
            it.scroll = (it.content_h - it.visible).max(0.0);
        }
        it.scroll = it.scroll.clamp(0.0, (it.content_h - it.visible).max(0.0));
        self.place_thumbs(si, k, b, t);
    }

    /// A piece's glow: soft light out of its outline, on the glass under the frame's window (where the mouse goes through it); gone
    /// when it does not glow.
    /// How long since a widget's glow began, or since it last came out (its rings go out from it for the first few seconds).
    fn glow_age(&self, k: &str) -> f32 {
        let Some(w) = self.widgets.get(unpop(k)) else { return 99.0 };
        match self.shown_at.get(k) {
            Some(t) if t.elapsed() < w.glow_at.elapsed() => t.elapsed(),
            _ => w.glow_at.elapsed(),
        }
        .as_secs_f32()
    }

    fn render_glow(&mut self, si: usize, k: &str, shape: &crate::shape::Shape, b: Bx, glow: Option<(crate::paint::Color, f32, bool)>) {
        let age = self.glow_age(k);
        let s = self.screens[si].s;
        let glow = glow.filter(|_| !self.glass_hidden);
        let Some((c, strength, _)) = glow else {
            let sc = &mut self.screens[si];
            if let Some(g) = sc.items.get_mut(k).and_then(|it| it.glow.take()) {
                sc.ov.remove_glass(&g);
            }
            return;
        };
        let m = (GLOW * s).round();
        let need = (shape.w.ceil() as u32 + 2 + 2 * m as u32, shape.h.ceil() as u32 + 2 + 2 * m as u32);
        let sc = &mut self.screens[si];
        let Some(it) = sc.items.get_mut(k) else { return };
        if it.glow.is_none() {
            it.glow = sc.ov.glass_piece(&self.g, need.0 + 16, need.1 + 16).ok();
        }
        let Some(gp) = it.glow.as_mut() else { return };
        if need.0 > gp.alloc.0 || need.1 > gp.alloc.1 {
            let grow = (need.0 + 16, need.1 + 16);
            if let Ok(surface) = self.g.surface(grow.0, grow.1) {
                unsafe {
                    let _ = gp.visual.SetContent(&surface);
                }
                gp.surface = surface;
                gp.alloc = grow;
            }
        }
        paint_glow(&self.g, gp, s, shape, b, false, c, strength, age);
    }

    /// The live pictures of every open panel put again (a menu came over them, or went).
    fn refresh_thumbs(&mut self) {
        let mut todo = Vec::new();
        for (si, sc) in self.screens.iter().enumerate() {
            for (k, it) in &sc.items {
                if !it.thumbs.is_empty() {
                    if let Ok((_, b)) = self.shape_now(si, it) {
                        todo.push((si, k.clone(), b, it.open.now.clamp(0.0, 1.0)));
                    }
                }
            }
        }
        for (si, k, b, t) in todo {
            self.place_thumbs(si, &k, b, t);
        }
    }

    /// The live pictures of a panel's windows: made when it is open, put in their boxes, gone when it shuts.
    fn place_thumbs(&mut self, si: usize, k: &str, b: Bx, t: f32) {
        let s = self.screens[si].s;
        let hwnd = self.screens[si].ov.hwnd;
        // (a menu over them: the live pictures it covers are hidden while it is open, since Windows draws them over all else)
        let menu = self.menu.as_ref().filter(|m| m.si == si).map(|m| m.at);
        let Some(it) = self.screens[si].items.get_mut(k) else { return };
        // (open and still; or at rest, out and still, for a picture in its peek)
        let at_rest = t <= 0.01 && !it.open.moving() && it.shown.now > 0.99 && !it.shown.moving();
        let show = (t >= 0.97 || at_rest) && !it.open.moving() && !it.rest.moving();
        // (the ones no longer asked for go)
        let wanted: Vec<isize> = if show { it.thumb_boxes.iter().map(|(w, _)| *w).collect() } else { Vec::new() };
        it.thumbs.retain(|(w, th)| {
            let keep = wanted.contains(w);
            if !keep {
                crate::thumbs::unregister(*th);
            }
            keep
        });
        if !show {
            return;
        }
        for (w, r) in it.thumb_boxes.clone() {
            let th = match it.thumbs.iter().find(|(x, _)| *x == w) {
                Some((_, th)) => *th,
                None => match crate::thumbs::register(hwnd, w) {
                    Some(th) => {
                        it.thumbs.push((w, th));
                        th
                    }
                    None => continue,
                },
            };
            let px = RECT { left: ((b.x + r.x) * s) as i32, top: ((b.y + r.y) * s) as i32, right: ((b.x + r.x + r.w) * s) as i32, bottom: ((b.y + r.y + r.h) * s) as i32 };
            let covered = menu.is_some_and(|m| m.x < b.x + r.x + r.w && b.x + r.x < m.x + m.w && m.y < b.y + r.y + r.h && b.y + r.y < m.y + m.h);
            crate::thumbs::place(th, px, !covered);
        }
    }

    /// The mouse reaches the windows only over the pieces.
    fn hit_regions(&self) {
        let mut targets: Vec<(i32, i32, i32, i32, String)> = Vec::new();
        for (si, sc) in self.screens.iter().enumerate() {
            let s = sc.s;
            let mut boxes: Vec<RECT> = Vec::new();
            for (k, it) in sc.items.iter() {
                if it.shown.now < 0.05 && it.drag_place.is_none() {
                    continue;
                }
                if is_pop(k) && !self.pop_live(k) {
                    continue;
                }
                if let Ok((_, b)) = self.shape_now(si, it) {
                    // (one that takes files dropped on it)
                    if self.widgets.get(k).is_some_and(|w| w.content["drop"].is_string()) {
                        let m = sc.ov.monitor.rect;
                        targets.push((m.left + (b.x * s) as i32, m.top + (b.y * s) as i32, m.left + ((b.x + b.w) * s).ceil() as i32, m.top + ((b.y + b.h) * s).ceil() as i32, k.clone()));
                    }
                    // (being pulled: where it is drawn now too, and its neck, or the frame's window cuts it off)
                    let b = if self.tear_pull > 0.5 && self.tear.as_ref().is_some_and(|(ts, tk, ..)| *ts == si && tk == k) {
                        let (ox, oy) = self.tear_off;
                        let (x0, y0) = (b.x.min(b.x + ox) - 40.0, b.y.min(b.y + oy) - 40.0);
                        let (x1, y1) = ((b.x + b.w).max(b.x + b.w + ox) + 40.0, (b.y + b.h).max(b.y + b.h + oy) + 40.0);
                        Bx { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
                    } else {
                        b
                    };
                    boxes.push(RECT { left: (b.x * s) as i32, top: (b.y * s) as i32, right: ((b.x + b.w) * s).ceil() as i32, bottom: ((b.y + b.h) * s).ceil() as i32 });
                }
            }
            if self.edit.as_ref().is_some_and(|e| e.si == si) {
                let m = sc.ov.monitor.rect;
                boxes.push(RECT { left: 0, top: 0, right: m.right - m.left, bottom: m.bottom - m.top });
            }
            // (a window going back, as a drop: where it falls (the frame's window shows nothing outside where it takes the mouse))
            for (dsi, _, b, _) in &self.pop_drops {
                if *dsi == si {
                    boxes.push(RECT { left: ((b.x - 12.0) * s) as i32, top: ((b.y - 12.0) * s) as i32, right: ((b.x + b.w + 12.0) * s).ceil() as i32, bottom: ((b.y + b.h + 12.0) * s).ceil() as i32 });
                }
            }
            if let Some(m) = self.menu.as_ref().filter(|m| m.si == si) {
                let b = m.at;
                boxes.push(RECT { left: (b.x * s) as i32, top: (b.y * s) as i32, right: ((b.x + b.w) * s).ceil() as i32, bottom: ((b.y + b.h) * s).ceil() as i32 });
            }
            if let Some(g) = self.ghost.as_ref().filter(|g| g.si == si) {
                let b = g.rest.now();
                boxes.push(RECT { left: (b.x * s) as i32, top: (b.y * s) as i32, right: ((b.x + b.w) * s).ceil() as i32, bottom: ((b.y + b.h) * s).ceil() as i32 });
            }
            if let (Some(b), true) = (&self.banner, Some(si) == self.banner_screen()) {
                if let Some(a) = b.at {
                    boxes.push(RECT { left: (a.x * s) as i32, top: (a.y * s) as i32, right: ((a.x + a.w) * s).ceil() as i32, bottom: ((a.y + a.h) * s).ceil() as i32 });
                }
            }
            sc.ov.set_hit(&boxes);
        }
        crate::drop::set_targets(targets);
    }

    // ----------------------------------------------------------------------------------------------------------- banners

    fn say(&mut self, plugin: String, say: Value) {
        let priority = say["priority"].as_i64().unwrap_or(0);
        // (a game runs: what can wait, waits — it is let go)
        if self.is_quiet() && say["ambient"].as_bool() == Some(true) && priority < 90 && !self.config.quiet_banners.contains(&plugin) {
            return;
        }
        // (one with a key replaces the one of that key)
        let k = s_of(&say["pill"], "key").to_string();
        if !k.is_empty() {
            self.queue.retain(|(p, s)| !(p == &plugin && s_of(&s["pill"], "key") == k));
            if let Some(b) = &mut self.banner {
                if b.plugin == plugin && s_of(&b.say["pill"], "key") == k {
                    b.say = say;
                    b.since = Instant::now();
                    b.ms = banner_ms(&b.say);
                    self.render_banner();
                    self.commit();
                    return;
                }
            }
        }
        let mut say = say;
        say["\u{0}at"] = json!(nadi_core::paths::now_ms());
        self.queue.push((plugin, say));
        self.queue.sort_by_key(|(_, s)| -s["priority"].as_i64().unwrap_or(0));
        if self.queue.len() > 12 {
            self.queue.truncate(12);
        }
        // (what asks something of the person comes now)
        if let Some(b) = &mut self.banner {
            if priority >= 90 && b.say["priority"].as_i64().unwrap_or(0) < 90 {
                b.leaving = true;
                b.slide.to = 0.0;
            }
        }
        self.next_banner();
    }

    fn cancel(&mut self, plugin: &str, key: &str) {
        self.queue.retain(|(p, s)| !(p == plugin && s_of(&s["pill"], "key") == key));
        if let Some(b) = &mut self.banner {
            if b.plugin == plugin && s_of(&b.say["pill"], "key") == key {
                b.leaving = true;
                b.slide.to = 0.0;
            }
        }
        self.tick_rate();
    }

    /// Is the widget a banner would come out of open (on its edge, or as a window)? Its banners wait till it closes.
    fn owner_open(&self, plugin: &str, say: &Value) -> bool {
        let (hk, ..) = self.banner_home(plugin, say);
        if hk.is_empty() {
            return false;
        }
        let pk = pop_key(&hk);
        self.screens.iter().any(|sc| sc.items.iter().any(|(k, it)| (*k == hk || *k == pk) && it.open.to > 0.5))
    }

    fn next_banner(&mut self) {
        if self.banner.is_some() || self.queue.is_empty() || self.edit.is_some() {
            return;
        }
        let Some(si) = self.banner_screen() else { return };
        // (a live one that waited longer than it lasts: what it said is over)
        let now = nadi_core::paths::now_ms();
        self.queue.retain(|(_, s)| !(s["pill"]["live"] == true && now.saturating_sub(s["\u{0}at"].as_u64().unwrap_or(now)) > s["pill"]["ms"].as_u64().unwrap_or(4000)));
        // (the first whose widget is not open: the others wait)
        let Some(i) = (0..self.queue.len()).find(|&i| !self.owner_open(&self.queue[i].0, &self.queue[i].1)) else { return };
        let (plugin, mut say) = self.queue.remove(i);
        let hosts = self.plugins.iter().find(|p| p.id == plugin).map(|p| p.net.clone()).unwrap_or_default();
        own_pictures(&mut say, &plugin, &hosts);
        let ms = banner_ms(&say);
        self.typed.remove(LIVE_BANNER);
        self.banner = Some(Banner { plugin, say, since: Instant::now(), ms, slide: Spring { now: 0.0, to: 1.0, vel: 0.0 }, hits: Vec::new(), leaving: false, h: 0.0, at: None, home: None, si: None, typing: false, marquee: false, glowing: false, born: Instant::now() });
        self.relayout_with(Some(&HashSet::new()));
        let _ = si;
        self.render_banner();
        self.hit_regions();
        self.commit();
        self.tick_rate();
    }

    /// How long a banner is: as much as what it says needs (its words, its bars, its buttons), no more than BANNER_W.
    fn banner_w(&self, s: f32, say: &Value) -> f32 {
        let p = self.painter(s);
        let pill = &say["pill"];
        let rows = pill["rows"].as_array().map_or(0, |r| r.len());
        let bare = pill["bare"] == true && rows > 0;
        // (live: as wide as it goes, so it does not change as its words come)
        if pill["live"] == true {
            return BANNER_W;
        }
        let lead = if bare { 0.0 } else { 46.0 };
        let mut need: f32 = 0.0;
        if !bare {
            let title = if s_of(say, "title").is_empty() { s_of(say, "text") } else { s_of(say, "title") };
            let sub = if !s_of(pill, "sub").is_empty() { s_of(pill, "sub") } else if !s_of(say, "title").is_empty() { s_of(say, "text") } else { "" };
            let right = s_of(pill, "right");
            let rw = if right.is_empty() { 0.0 } else { p.measure(right, 11.5, Weight::Semibold, 120.0) + 8.0 };
            need = lead + p.measure(title, 14.0, Weight::Semibold, 320.0).max(p.measure(sub, 11.5, Weight::Regular, 320.0)) + rw + 4.0;
        }
        // (bars: a label, a bar long enough to read, a number)
        if rows > 0 {
            need = need.max(lead + 30.0 + 150.0 + 50.0);
        }
        if let Some(bts) = pill["buttons"].as_array() {
            let w: f32 = bts.iter().map(|bt| if s_of(bt, "icon").is_empty() { p.measure(s_of(bt, "label"), 11.5, Weight::Semibold, 160.0) + 32.0 } else { 42.0 }).sum();
            need = need.max(lead + w);
        }
        (need + 2.0 * PAD).clamp(200.0, BANNER_W)
    }

    fn render_banner(&mut self) {
        let Some(si) = self.banner_screen() else { return };
        // (on a screen of its own now: its piece there, and gone from the one it was on)
        let was = self.banner.as_ref().and_then(|b| b.si);
        if was != Some(si) {
            if let Some(sc) = was.and_then(|w| self.screens.get(w)) {
                sc.hide_banner();
            }
            if let Some(b) = &mut self.banner {
                b.si = Some(si);
            }
        }
        if self.screens[si].banner_piece.is_none() {
            let s = self.screens[si].s;
            if let Ok(p) = self.screens[si].ov.piece(&self.g, ((BANNER_W + 24.0) * s) as u32, (150.0 * s) as u32) {
                self.screens[si].banner_piece = Some(p);
            }
        }
        let Some(b) = &self.banner else { return };
        let s = self.screens[si].s;
        let say = b.say.clone();
        let pill = &say["pill"];
        let rows = pill["rows"].as_array().cloned().unwrap_or_default();
        let has_buttons = pill["buttons"].as_array().is_some_and(|a| !a.is_empty());
        let full_h = banner_h(&say);
        let t = b.slide.now.max(0.0);
        let m = Metrics::default();
        let f = m.fillet;
        // where it comes out: the Notifications widget's place (else the top, in the middle); it slides out of that edge
        let area = self.screens[si].area;
        let bw = self.banner_w(s, &say);
        let (place, full, floating) = self.screens[si].banner_slot.unwrap_or((Place::Edge(Edge::Top), Bx { x: area.x + area.w / 2.0 - (bw + 2.0 * f) / 2.0, y: area.y, w: bw + 2.0 * f, h: full_h }, false));
        // (where it is on its way to its widget's place)
        let full = b.home.map_or(full, |h| h.now());
        let hide = 1.0 - t.min(1.0);
        let (dx, dy) = match place {
            Place::Edge(Edge::Top) | Place::Corner(Corner::TopLeft | Corner::TopRight) => (0.0, -full.h * hide),
            Place::Edge(Edge::Bottom) | Place::Corner(Corner::BottomLeft | Corner::BottomRight) => (0.0, full.h * hide),
            Place::Edge(Edge::Left) => (-full.w * hide, 0.0),
            Place::Edge(Edge::Right) => (full.w * hide, 0.0),
        };
        let bx = Bx { x: full.x + dx, y: full.y + dy, ..full };
        let Ok((shape, sb)) = self.rest_shape(s, place, bx, floating, Some(BANNER_RADIUS)) else { return };
        // (what is in it: the shape less its fillets, from the corner of its piece)
        let inner = if floating {
            Rect::new(0.0, 0.0, sb.w, sb.h)
        } else {
            match place {
                Place::Edge(Edge::Top | Edge::Bottom) => Rect::new(f, 0.0, sb.w - 2.0 * f, sb.h),
                Place::Edge(_) => Rect::new(0.0, f, sb.w, sb.h - 2.0 * f),
                Place::Corner(Corner::TopLeft) => Rect::new(0.0, 0.0, sb.w - f, sb.h - f),
                Place::Corner(Corner::TopRight) => Rect::new(f, 0.0, sb.w - f, sb.h - f),
                Place::Corner(Corner::BottomLeft) => Rect::new(0.0, f, sb.w - f, sb.h - f),
                Place::Corner(Corner::BottomRight) => Rect::new(f, f, sb.w - f, sb.h - f),
            }
        };
        let h = inner.h;
        let t = t.min(1.0);
        let age = b.since.elapsed().as_secs_f32();
        let need = (shape.w.ceil() as u32 + 2, shape.h.ceil() as u32 + 2);
        let surface = {
            let Some(piece) = &mut self.screens[si].banner_piece else { return };
            if need.0 > piece.alloc.0 || need.1 > piece.alloc.1 {
                let grow = (need.0 + 16, need.1 + 16);
                if let Ok(surface) = self.g.surface(grow.0, grow.1) {
                    unsafe {
                        let _ = piece.visual.SetContent(&surface);
                    }
                    piece.surface = surface;
                    piece.alloc = grow;
                }
            }
            let _ = piece.place((sb.x * s).round() as i32, (sb.y * s).round() as i32);
            piece.surface.clone()
        };
        let mut hits: Vec<(Rect, Press)> = Vec::new();
        // (live: its words typed in as they come, a few letters a frame, quicker the more there is)
        let live = pill["live"] == true;
        let mut live_text = String::new();
        if live {
            let full = s_of(&say, "text").to_string();
            let now = Instant::now();
            let e = self.typed.entry(LIVE_BANNER.to_string()).or_insert((String::new(), 0.0, now));
            if e.0 != full {
                let common = e.0.chars().zip(full.chars()).take_while(|(a, b)| a == b).count();
                e.1 = e.1.min(common as f32);
                e.0 = full.clone();
            }
            let total = full.chars().count() as f32;
            let dt = now.duration_since(e.2).as_secs_f32().min(0.1);
            e.2 = now;
            e.1 = (e.1 + ((total - e.1) / 0.3).max(45.0) * dt).min(total);
            let typing = e.1 < total;
            live_text = full.chars().take(e.1 as usize).collect();
            if let Some(b) = self.banner.as_mut() {
                b.typing = typing;
            }
        }
        let p = self.painter(s);
        let _ = crate::draw::take_marquee();
        // (a banner of a plugin with its own colour: in that, not the theme's, which would say something else)
        let own = self.banner.as_ref().and_then(|b| self.plugin_accent(&b.plugin));
        let was_accent = crate::draw::set_accent_now(own);
        // (its border, bright as it comes out and settling, in its glow's colour; one that refuses the glow has none)
        let border = (say["glow"] != false && pill["glow"] != false).then(|| {
            let c = own.unwrap_or_else(crate::draw::accent);
            let e = self.banner.as_ref().map_or(99.0, |b| b.born.elapsed().as_secs_f32());
            let st = GLOW_SETTLED + (GLOW_FRESH - GLOW_SETTLED) * (-e / GLOW_SETTLE_S).exp();
            c.alpha(((st - 0.5) / 1.5).clamp(0.3, 1.0))
        });
        let _ = self.g.draw_surface(&surface, |dc| {
            let _ = fill_shape(dc, &shape, FACE);
            if let Some(c) = border {
                if let Ok(br) = crate::paint::brush(dc, c.alpha(c.3 * t.min(1.0))) {
                    unsafe {
                        dc.DrawGeometry(&shape.geometry, &br, 3.0 * s, None);
                    }
                }
            }
            if t < 0.5 {
                return;
            }
            let a = ((t - 0.5) / 0.5).min(1.0);
            with_opacity(dc, &self.g, a, || unsafe {
                let body = Rect::new(inner.x + PAD, inner.y, inner.w - 2.0 * PAD, h);
                dc.PushAxisAlignedClip(&p.px(Rect::new(0.0, 0.0, sb.w, sb.h)).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
                let cy = body.y + 30.0;
                let mut tx = body.x;
                // (bare: its rows alone, from the top; else its icon and its words, then the rows)
                let bare = pill["bare"] == true && !rows.is_empty();
                // (bare: its rows (and buttons) in the middle of it, as much room above as below)
                let block = (rows.len().max(1) - 1) as f32 * 18.0 + 12.0 + if has_buttons { 40.0 } else { 0.0 };
                let mut y = if bare { body.y + ((h - block) / 2.0).max(6.0) + 2.0 } else { cy + 26.0 };
                if live {
                    // (who: their animal on their ground, else its icon; their name over the newest of their words)
                    let mid = body.y + h / 2.0;
                    let avatar = s_of(pill, "avatar");
                    if !avatar.is_empty() {
                        p.circle(tx + 18.0, mid, 18.0, crate::draw::color_of(&pill["tint"], crate::paint::Color(0.149, 0.149, 0.173, 1.0)));
                        let aw = p.measure(avatar, 19.0, Weight::Regular, 60.0);
                        p.paragraph(avatar, tx + 18.0 - aw / 2.0 - 0.5, mid - 19.0 * 0.675, 19.0, aw + 6.0, 1, draw::TEXT, true);
                    } else {
                        let icon = if s_of(pill, "icon").is_empty() { "chat" } else { s_of(pill, "icon") };
                        p.circle(tx + 18.0, mid, 18.0, draw::FAINT);
                        p.icon(icon, tx + 9.0, mid - 9.0, 18.0, draw::TEXT);
                    }
                    tx += 48.0;
                    let tw = body.right() - tx;
                    let title = s_of(&say, "title");
                    let words = crate::peek::tail_fit(&p, &live_text, 13.0, tw, 2.0);
                    let wh = p.paragraph(&words, 0.0, 0.0, 13.0, tw, 2, draw::TEXT, false).max(13.0 * 1.35);
                    let th = if title.is_empty() { 0.0 } else { 18.0 };
                    let top = mid - (th + wh) / 2.0;
                    if !title.is_empty() {
                        p.text(title, tx, top + 8.0, 12.5, Weight::Semibold, tw, draw::DIM);
                    }
                    p.paragraph(&words, tx, top + th, 13.0, tw, 2, draw::TEXT, true);
                } else if !bare {
                    if pill["tile"].is_object() {
                        p.tile(&pill["tile"], tx, cy - 18.0, 36.0, s_of(pill, "icon"));
                        tx += 46.0;
                    } else {
                        let icon = if s_of(pill, "icon").is_empty() { "bell" } else { s_of(pill, "icon") };
                        p.circle(tx + 18.0, cy, 18.0, draw::FAINT);
                        p.icon(icon, tx + 9.0, cy - 9.0, 18.0, draw::tone_color(s_of(pill, "tone")).alpha(1.0));
                        tx += 46.0;
                    }
                    let right = s_of(pill, "right");
                    let rw = if right.is_empty() { 0.0 } else { p.measure(right, 11.5, Weight::Semibold, 120.0) + 8.0 };
                    let title = if s_of(&say, "title").is_empty() { s_of(&say, "text") } else { s_of(&say, "title") };
                    let sub = if !s_of(pill, "sub").is_empty() { s_of(pill, "sub") } else if !s_of(&say, "title").is_empty() { s_of(&say, "text") } else { "" };
                    if sub.is_empty() {
                        p.text(title, tx, cy, 14.0, Weight::Semibold, body.right() - tx - rw, draw::TEXT);
                    } else {
                        p.text(title, tx, cy - 8.0, 13.5, Weight::Semibold, body.right() - tx - rw, draw::TEXT);
                        p.text(sub, tx, cy + 9.0, 11.5, Weight::Regular, body.right() - tx - rw, draw::DIM);
                    }
                    if !right.is_empty() {
                        p.text_end(right, body.right(), cy, 11.5, Weight::Semibold, 120.0, draw::DIM);
                    }
                }
                // (bars that slide from what they were to what they are)
                let slide = ((age - 0.5) / 1.2).clamp(0.0, 1.0);
                for r in &rows {
                    let from = r["from"].as_f64().unwrap_or(r["value"].as_f64().unwrap_or(0.0)) as f32;
                    let to = r["value"].as_f64().unwrap_or(0.0) as f32;
                    let v = from + (to - from) * ease(slide);
                    p.text(s_of(r, "label"), tx, y + 4.0, 11.0, Weight::Semibold, 30.0, draw::DIM);
                    p.bar(Rect::new(tx + 30.0, y + 1.5, body.right() - tx - 80.0, 5.0), v, draw::accent());
                    p.text_end(s_of(r, "text"), body.right(), y + 4.0, 11.0, Weight::Semibold, 44.0, draw::TEXT);
                    y += 18.0;
                }
                if has_buttons {
                    let mut bx = tx;
                    for bt in pill["buttons"].as_array().cloned().unwrap_or_default() {
                        let label = s_of(&bt, "label");
                        let icon = s_of(&bt, "icon");
                        let tone = s_of(&bt, "tone");
                        let (bg, fg) = match tone {
                            "ok" => (draw::GREEN.alpha(0.22), draw::GREEN),
                            "no" => (draw::RED.alpha(0.2), draw::RED),
                            _ => (draw::FAINT, draw::TEXT),
                        };
                        let r = if !icon.is_empty() {
                            p.circle(bx + 16.0, y + 14.0, 16.0, bg);
                            p.icon(icon, bx + 8.0, y + 6.0, 16.0, fg);
                            Rect::new(bx, y - 2.0, 32.0, 32.0)
                        } else {
                            let w = p.measure(label, 11.5, Weight::Semibold, 160.0) + 22.0;
                            p.round(Rect::new(bx, y + 1.0, w, 26.0), 13.0, bg);
                            p.text(label, bx + 11.0, y + 14.0, 11.5, Weight::Semibold, 160.0, fg);
                            Rect::new(bx, y + 1.0, w, 26.0)
                        };
                        hits.push((r, Press::Action(s_of(&bt, "action").to_string(), Value::Null)));
                        bx += r.w + 10.0;
                    }
                }
                dc.PopAxisAlignedClip();
            });
        });
        crate::draw::set_accent_now(was_accent);
        let marquee = crate::draw::take_marquee();
        if let Some(b) = self.banner.as_mut() {
            b.marquee = marquee;
        }
        // (its glow, as a widget's as it comes out: strong, then settling, rings going out from it; a banner may refuse it)
        let glows = say["glow"] != false && pill["glow"] != false && !self.glass_hidden;
        let glow_age = self.banner.as_ref().map_or(99.0, |b| b.born.elapsed().as_secs_f32());
        if glows {
            let c = self.banner.as_ref().and_then(|b| self.plugin_accent(&b.plugin)).unwrap_or_else(crate::draw::accent);
            let strength = (GLOW_SETTLED + (GLOW_FRESH - GLOW_SETTLED) * (-glow_age / GLOW_SETTLE_S).exp()) * t;
            let m = (GLOW * s).round();
            let gneed = (shape.w.ceil() as u32 + 2 + 2 * m as u32, shape.h.ceil() as u32 + 2 + 2 * m as u32);
            let sc = &mut self.screens[si];
            if sc.banner_glow.is_none() {
                sc.banner_glow = sc.ov.glass_piece(&self.g, gneed.0 + 16, gneed.1 + 16).ok();
            }
            if let Some(gp) = sc.banner_glow.as_mut() {
                if gneed.0 > gp.alloc.0 || gneed.1 > gp.alloc.1 {
                    let grow = (gneed.0 + 16, gneed.1 + 16);
                    if let Ok(surface) = self.g.surface(grow.0, grow.1) {
                        unsafe {
                            let _ = gp.visual.SetContent(&surface);
                        }
                        gp.surface = surface;
                        gp.alloc = grow;
                    }
                }
                paint_glow(&self.g, gp, s, &shape, sb, true, c, strength, glow_age);
            }
        } else if let Some(gp) = &self.screens[si].banner_glow {
            unsafe {
                let _ = gp.visual.SetOffsetY2(-1000.0);
            }
        }
        let b = self.banner.as_mut().unwrap();
        b.hits = hits;
        b.h = full_h;
        b.at = Some(sb);
        b.glowing = glows && glow_age < GLOW_SETTLE_S * 4.0;
    }

    // ------------------------------------------------------------------------------------------------------------ time

    /// The graphics driver was reset (a GPU hang, a driver update): the drawing devices are gone. Make new ones, and everything drawn
    /// with them again. True when it happened.
    fn recover_device(&mut self) -> bool {
        let lost = unsafe { self.g.d3d.GetDeviceRemovedReason() }.is_err();
        if !lost {
            return false;
        }
        let Ok(g) = Gfx::new() else { return true };
        crate::icons::forget();
        crate::images::forget();
        self.close_settings();
        self.launcher = None;
        self.modal = None;
        self.banner = None;
        for sc in &self.screens {
            for it in sc.items.values() {
                for (_, th) in &it.thumbs {
                    crate::thumbs::unregister(*th);
                }
            }
        }
        // (the windows were drawn with the old devices: made again, with the new)
        let old = std::mem::replace(&mut self.g, g);
        self.make_screens();
        drop(old);
        self.launcher = crate::launcher::Launcher::new(&self.g).ok();
        self.modal = crate::modal::Modal::new(&self.g).ok();
        true
    }

    /// One tick of the clock: springs move, the hover opens and closes panels, banners come and go, what lives is drawn again.
    pub fn tick(&mut self) {
        let t0 = Instant::now();
        self.slid_settle();
        self.card_tick();
        self.focus_pending();
        self.tick_inner();
        let e = self.prof.entry("tick".into()).or_default();
        e.0 += 1;
        e.1 += t0.elapsed().as_secs_f64() * 1000.0;
        self.busy_ms += t0.elapsed().as_secs_f64() * 1000.0;
        // (once a second: what NADI costs, sampled; the Performance page told while it is open)
        if self.perf.due() {
            let plugins = self.pc.plugin_cost.lock().unwrap().clone();
            if self.perf.sample(self.busy_ms, &plugins, &self.render_cost) && self.perf_on {
                self.push_perf(&plugins);
            }
        }
        self.profile_on_request();
    }

    /// (for checking: while a file `profile.on` is in NADI's folder, what the frame spent its time on is written beside it, every
    /// ten seconds, as `profile.json`: each thing, how many times, how many ms, since the last write)
    fn profile_on_request(&mut self) {
        if self.prof_at.elapsed() < Duration::from_secs(10) {
            return;
        }
        let since = self.prof_at.elapsed().as_secs_f64();
        self.prof_at = Instant::now();
        let Some(dir) = nadi_core::paths::data_dir() else { return };
        if !dir.join("profile.on").exists() {
            self.prof.clear();
            return;
        }
        let mut rows: Vec<(&String, &(u32, f64))> = self.prof.iter().collect();
        rows.sort_by(|a, b| b.1 .1.total_cmp(&a.1 .1));
        let list: Vec<Value> = rows.iter().map(|(k, (n, ms))| json!({ "what": k, "times": n, "ms": (ms * 10.0).round() / 10.0, "ms_per_s": (ms / since * 100.0).round() / 100.0 })).collect();
        let tick_ms = self.timer_ms;
        let _ = std::fs::write(dir.join("profile.json"), serde_json::to_string_pretty(&json!({ "seconds": since.round(), "timer_ms": tick_ms, "hover": self.hover, "rows": list })).unwrap_or_default());
        self.prof.clear();
    }

    fn tick_inner(&mut self) {
        let dt = self.last.elapsed().as_secs_f32().min(0.05);
        self.last = Instant::now();
        let mut relayer = false;
        // a taskbar that hides itself slid out or away: the frame keeps clear of it
        let bars = crate::taskbar::rects();
        if bars != self.bars_at {
            self.bars_at = bars;
            if self.config.taskbar != "hide" {
                self.areas();
                self.relayout();
            }
        }
        // what is in front on each monitor, a few times a second (a window maximized, or full screen)
        if self.fronts_at.elapsed() >= Duration::from_millis(250) {
            self.fronts_at = Instant::now();
            let tq = Instant::now();
            self.full_screen(false);
            self.prof_add("full screen check", tq);
        }
        // the bars of the sound (and the analyzer is listened to while a widget shows them)
        let want_viz = self.widgets.iter().any(|(k, w)| shows_bars(&w.content["peek"]) || (w.content["peek"]["level"]["playing"] == true && self.large(k)));
        self.pc.want.shell_audio.store(want_viz || !self.faces.is_empty(), Ordering::Relaxed);
        if want_viz {
            self.viz.step(&self.pc, dt);
        }
        crate::draw::set_lift(if want_viz { self.viz.lift } else { 0.0 });
        // the faces
        let tf = Instant::now();
        let face_drawn = self.step_faces(dt);
        self.prof_add("faces step", tf);
        for (si, k) in &face_drawn {
            if self.screens.get(*si).is_some_and(|s| s.items.contains_key(k)) {
                self.render(*si, k);
            }
        }
        // a scrollbar, while it shows or fades
        if let Some((si, k)) = self.scrollbar_moves() {
            self.render(si, &k);
        }
        // the rolling numbers, while they roll
        let rolling: Vec<String> = self.rolls.borrow().iter().filter(|(_, r)| r.2.elapsed().as_secs_f32() < ROLL_FOR + 0.1).map(|(k, _)| k.clone()).collect();
        for k in rolling {
            for si in 0..self.screens.len() {
                if self.screens[si].items.contains_key(&k) {
                    self.render(si, &k);
                }
            }
        }
        // the PC's small things, when they changed
        if crate::images::web_arrived() {
            for si in 0..self.screens.len() {
                let keys: Vec<String> = self.screens[si].items.keys().cloned().collect();
                for k in keys {
                    self.render(si, &k);
                }
            }
        }
        // the Recycle Bin, when it changed
        if let Some(b) = crate::recycle::take() {
            self.bin = b;
            self.sync_bin();
        }
        // files being dragged (File Explorer's picture of what is dragged is out, the button held): the bin comes out to take them
        // (begun when File Explorer's picture of what is dragged shows; it lasts while the button is held: over the frame's windows
        // that picture is hidden, and the bin must not go then)
        let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 };
        let begun = held && unsafe { FindWindowW(windows::core::w!("SysDragImage"), None).is_ok_and(|h| !h.0.is_null() && IsWindowVisible(h).as_bool()) };
        let dragging = held && (self.files_dragging || begun || crate::drop::hover().is_some());
        if dragging != self.files_dragging {
            self.files_dragging = dragging;
            self.sync_bin();
        }
        let sys = crate::sysw::take();
        let net = crate::net::take().or_else(|| crate::net::status_changed().then(|| self.bi.net.clone()));
        let radios = crate::radios::now();
        if sys.is_some() || net.is_some() || radios != self.bi.radios {
            let warm = self.start.elapsed() > Duration::from_secs(6);
            if let Some(s) = sys {
                // (what is worth a glance: its widget bumps)
                if warm {
                    let o = &self.bi.sys;
                    let mut b: Vec<&str> = Vec::new();
                    if s.desk != o.desk || s.desks != o.desks {
                        self.bumped_at.remove(&key(crate::builtin::PLUGIN, crate::sysw::DESKS));
                        b.push(crate::sysw::DESKS);
                    }
                    if s.lang != o.lang {
                        b.push(crate::sysw::LANG);
                    }
                    if s.mic.iter().any(|a| !o.mic.contains(a)) || s.cam.iter().any(|a| !o.cam.contains(a)) {
                        b.push(crate::sysw::PRIVACY);
                    }
                    if s.drives.iter().any(|d| d.usb && !o.drives.iter().any(|x| x.letter == d.letter)) {
                        b.push(crate::sysw::DRIVES);
                    }
                    let lows = |g: &Value| g["controllers"].as_array().into_iter().chain(g["devices"].as_array()).flatten().filter(|d| d["level"].as_f64().is_some_and(|l| l <= 0.2)).count();
                    let ons = |g: &Value| g["controllers"].as_array().into_iter().chain(g["devices"].as_array()).flatten().count();
                    if lows(&s.gear) > lows(&o.gear) || ons(&s.gear) > ons(&o.gear) {
                        b.push(crate::sysw::DEVICES);
                    }
                    for id in b {
                        self.bump(&key(crate::builtin::PLUGIN, id));
                    }
                }
                self.bi.sys = s;
            }
            if let Some(n) = net {
                if warm {
                    let joined = |s: &crate::net::State| s.wifi.iter().find(|w| w.joined).map(|w| w.ssid.clone());
                    if n.online != self.bi.net.online || joined(&n) != joined(&self.bi.net) {
                        self.bump(&key(crate::builtin::PLUGIN, crate::net::NET));
                    }
                    let connected = |s: &crate::net::State| s.paired.iter().filter(|p| p.connected).count();
                    if connected(&n) > connected(&self.bi.net) {
                        self.bump(&key(crate::builtin::PLUGIN, crate::sysw::DEVICES));
                    }
                }
                self.bi.net = n;
                // (the radios asked again, with the network)
                crate::radios::refresh();
            }
            self.bi.radios = radios;
            let tq = Instant::now();
            self.sync_sys();
            self.prof_add("sys widgets", tq);
        }
        // how loud the microphone is, while a voice listening shows (Speech's bars)
        let voice = self.widgets.iter().any(|(k, w)| w.content["peek"]["voice"]["state"] == "listening" && self.seen_anywhere(k));
        if voice {
            let (_, mic) = crate::volume::peaks();
            let to = mic.clamp(0.0, 1.0);
            self.voice_now = if to > self.voice_now { to } else { self.voice_now + (to - self.voice_now) * (1.0 - (-dt * 8.0).exp()) };
            crate::peek::set_voice(self.voice_now);
        }
        // how fast the network moves, once a second while its panel is open
        let nk = key(crate::builtin::PLUGIN, crate::net::NET);
        let net_open = self.screens.iter().any(|sc| sc.items.get(&nk).is_some_and(|i| i.open.to == 1.0));
        if net_open {
            let due = self.net_octets.as_ref().is_none_or(|(t, ..)| t.elapsed() >= Duration::from_millis(1000));
            if due {
                if let Some((i, o)) = crate::net::octets() {
                    let now = Instant::now();
                    if let Some((t, pi, po)) = self.net_octets {
                        let dt = now.duration_since(t).as_secs_f64().max(0.2);
                        let sp = (i.saturating_sub(pi) as f64 / dt, o.saturating_sub(po) as f64 / dt);
                        let shown = |v: Option<(f64, f64)>| v.map(|(d, u)| (crate::net::rate_text(d), crate::net::rate_text(u)));
                        if shown(Some(sp)) != shown(self.net_speed) {
                            self.net_speed = Some(sp);
                            let content = crate::net::widget(&self.bi.net, self.bi.radios, self.net_speed, self.bi.net_ask.as_deref());
                            self.apply(vec![Update::Widget { plugin: crate::builtin::PLUGIN.into(), widget: crate::net::NET.into(), content }]);
                        }
                    }
                    self.net_octets = Some((now, i, o));
                }
            }
        } else if self.net_octets.is_some() {
            self.net_octets = None;
            self.net_speed = None;
        }
        // the Volume Mixer's meters: how loud the output, the microphone and each program are now, springing (up at once, down
        // slowly); only while it is seen
        let qk = key(crate::builtin::PLUGIN, crate::builtin::QUICK);
        if self.place_of(crate::builtin::PLUGIN, crate::builtin::QUICK).is_some() && self.seen_anywhere(&qk) {
            let (out, mic) = crate::volume::peaks();
            let mut raw: HashMap<String, f32> = self.bi.audio.peaks().into_iter().map(|(k, v)| (format!("app:{k}"), v)).collect();
            raw.insert("master".into(), out);
            raw.insert("mic".into(), mic);
            for (k, v) in self.meter_now.iter_mut() {
                let to = raw.get(k).copied().unwrap_or(0.0);
                *v = if to > *v { to } else { *v + (to - *v) * (1.0 - (-dt * 6.0).exp()) };
            }
            for (k, v) in raw {
                self.meter_now.entry(k).or_insert(v);
            }
            self.meter_now.retain(|_, v| *v > 0.002);
            crate::blocks::set_meters(self.meter_now.clone());
        } else if !self.meter_now.is_empty() {
            self.meter_now.clear();
            crate::blocks::set_meters(HashMap::new());
        }
        // the windows (for the dock), twice a second; its meters spring, its sound bars move; Win held, its numbers. Only while the
        // dock shows (hidden at its edge till the mouse comes: nothing is looked at or drawn; as it comes, it is looked at at once)
        let dock_seen = self.dock_placed() && self.seen_anywhere(&key(crate::builtin::PLUGIN, crate::builtin::DOCK));
        // (the dock at its largest, out: what plays is looked at, for its card)
        let card = dock_seen && self.large(&key(crate::builtin::PLUGIN, crate::builtin::DOCK));
        self.pc.want.shell_media.store(card, Ordering::Relaxed);
        // quick settings, placed and seen without the dock: its sound looked at with the windows
        if !dock_seen && self.place_of(crate::builtin::PLUGIN, crate::builtin::QUICK).is_some() && self.seen_anywhere(&key(crate::builtin::PLUGIN, crate::builtin::QUICK)) {
            self.sync_builtins(false);
        }
        if dock_seen && !self.dock_was_seen {
            self.sync_builtins(true);
        }
        self.dock_was_seen = dock_seen;
        // (the app in front changed: its card grows, its ground pops in; the dock drawn again (laid out, at its largest) as it does)
        let dk = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
        self.note_front();
        if dock_seen && self.dock_front.as_ref().is_some_and(|f| f.2.elapsed().as_secs_f32() < DOCK_FRONT_S + 0.05) {
            if self.large(&dk) {
                let one: HashSet<String> = [dk.clone()].into_iter().collect();
                self.relayout_with(Some(&one));
            } else {
                for si in 0..self.screens.len() {
                    if self.screens[si].items.contains_key(&dk) {
                        self.render(si, &dk);
                    }
                }
            }
            relayer = true;
        }
        if dock_seen {
            let held = crate::keyhook::win_held() && self.config.win_launcher();
            if held != self.bi.numbers {
                self.bi.numbers = held;
                self.sync_builtins(true);
            }
            let tq = Instant::now();
            self.sync_builtins(false);
            self.prof_add("windows scan", tq);
            let tq = Instant::now();
            self.step_dock(dt);
            self.prof_add("dock step", tq);
        }
        // the modal's spring (its countdown may answer)
        if let Some(m) = &mut self.modal {
            if m.step(&self.g, dt) {
                m.render(&self.g, &self.pc);
            }
        }
        self.modal_answered();
        // the launcher's spring
        if let Some(l) = &mut self.launcher {
            let changed = l.check();
            if l.step(dt) || changed {
                l.render(&self.g, &self.pc);
            }
        }
        // which widgets show (the setting: all, the edge the mouse comes to, the ones near it)
        if self.reveal(dt) {
            relayer = true;
        }
        // (which plugins have a widget that can be seen: the others are told, and what comes often comes to them seldom)
        self.tell_seen();
        // (one coming out after changes made while it, or all, were tucked away: laid out now, with them)
        let coming_out = self.screens.iter().any(|sc| sc.items.values().any(|it| (it.shown.to > 0.5 && it.shown.now < 0.98) || (it.open.to > 0.5 && it.open.now < 0.98)));
        if self.layout_pending && (coming_out || self.edit.is_some() || self.drag.is_some() || self.banner.is_some() || self.menu.is_some()) {
            self.relayout();
            relayer = true;
        }
        // a press held long enough picks its widget up
        if self.press.is_some() {
            self.pick_up();
        }
        // (held: a shadow where it would go)
        if self.step_ghost(dt) {
            relayer = true;
        }
        // a click elsewhere shuts what a click opened, and the menu
        // (Esc: pressed since the last tick; it was up then, and is down now)
        let down = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000 != 0 };
        self.esc = down && !std::mem::replace(&mut self.esc_down, down);
        self.menu_watch();
        self.edit_watch();
        self.step_pops(dt);
        // (an open panel the mouse just came onto: its handle fades in)
        if let Some((hs, hk)) = self.hover.clone() {
            if self.screens.get(hs).and_then(|sc| sc.items.get(&hk)).is_some_and(|it| !is_pop(&hk) && it.open.now > 0.5 && it.hover_since.is_some_and(|t| t.elapsed().as_secs_f32() < 0.16)) {
                self.render(hs, &hk);
                relayer = true;
            }
        }
        self.click_outside();
        if self.step_menu(dt) {
            relayer = true;
        }
        self.step_tip(dt);
        self.step_hold();
        // (the menu follows its widget as it moves; words in it that scroll, scroll)
        if let Some(m) = &self.menu {
            let moving = m.marquee || m.key.as_ref().and_then(|k| self.screens.get(m.si).and_then(|sc| sc.items.get(k))).is_some_and(|it| it.rest.moving() || it.shown.moving());
            if moving {
                self.render_menu();
                relayer = true;
            }
        }
        // hover: open what was held long enough; close what was left
        let mut relay = false;
        let mut greet = false;
        let hover = self.hover.clone();
        let by_click = self.click_to_open() || self.menu.is_some() || self.edit.is_some();
        let mut kept: HashSet<String> = self.widgets.keys().filter(|k| self.always_open(k) && self.widgets.get(*k).is_some_and(|w| w.content["panel"].is_object())).cloned().collect();
        // (one being typed into stays open, the mouse in its box or not)
        if let Some(ib) = self.input.as_ref().filter(|i| i.shown) {
            kept.insert(ib.key.clone());
        }
        for si in 0..self.screens.len() {
            let mut opened = Vec::new();
            for (k, it) in self.screens[si].items.iter_mut() {
                if kept.contains(k) && it.open.to != 1.0 && it.drag_place.is_none() && self.drag.is_none() {
                    it.open.to = 1.0;
                    opened.push(k.clone());
                }
            }
            for k in opened {
                self.set_open(si, &k, true);
                relay = true;
            }
        }
        for si in 0..self.screens.len() {
            let keys: Vec<String> = self.screens[si].items.keys().cloned().collect();
            for k in keys {
                if is_pop(&k) {
                    continue;
                }
                // (opening on a click, or always open: the mouse resting or leaving does nothing; but over a full-screen app, where a
                // click in it may not be seen, what a click opened shuts as the mouse leaves it)
                let full = self.screens[si].full;
                if kept.contains(&k) || (by_click && !full) {
                    continue;
                }
                let hovered = hover.as_ref().is_some_and(|(s, h)| *s == si && *h == k);
                // (its window out: its panel opens there, not here)
                let has_panel = self.widgets.get(unpop(&k)).is_some_and(|w| w.content["panel"].is_object()) && !self.popped(&k);
                // (being pulled off its edge: it stays open, the mouse off it or not)
                let pulled = self.tear.as_ref().is_some_and(|(ts, tk, ..)| *ts == si && *tk == k) || self.sizing.as_ref().is_some_and(|z| z.si == si && z.key == k);
                let it = self.screens[si].items.get_mut(&k).unwrap();
                if hovered {
                    it.left_at = None;
                    if !by_click && it.open.to == 0.0 && has_panel && it.hover_since.is_some_and(|t| t.elapsed() >= HOVER_OPEN) {
                        it.open.to = 1.0;
                        relayer = true;
                        relay = true;
                        greet = true;
                        self.set_open(si, &k, true);
                    }
                } else {
                    it.hover_since = None;
                    if it.open.to == 1.0 && (!it.pinned || by_click) && !pulled {
                        let left = *it.left_at.get_or_insert_with(Instant::now);
                        if left.elapsed() >= LEAVE_CLOSE {
                            it.open.to = 0.0;
                            relay = true;
                            self.set_open(si, &k, false);
                        }
                    }
                }
            }
        }
        if relay {
            self.relayout_with(Some(&HashSet::new()));
        }
        // (a panel opened: the eyes are happy to see you)
        if greet {
            self.eyes.greet(self.start.elapsed().as_secs_f32() * 1000.0);
        }
        // the frame follows the mouse to another monitor (not while a panel is open there)
        if self.config.monitors != "all" && self.config.monitors != "primary" {
            if let Some(m) = self.mouse_screen() {
                // (a panel kept open for good does not keep the frame here)
                let open_here = self.screens.get(self.active).is_some_and(|s| s.items.iter().any(|(k, i)| i.open.to == 1.0 && !self.always_open(k)));
                // (nor while editing: the widgets stay with the tray, on the screen it was begun on)
                if m != self.active && !open_here && self.banner.is_none() && self.edit.is_none() && self.drag.is_none() && self.pop_drag.is_none() && self.tear.is_none() {
                    self.active = m;
                    self.hover = None;
                    self.relayout();
                }
            }
        }
        // springs and what lives
        let hv = self.hover.clone();
        let calm = self.drag.is_none() && self.edit.is_none() && self.tear.is_none() && self.sizing.is_none() && self.menu.is_none();
        for si in 0..self.screens.len() {
            let full = self.screens[si].full;
            let keys: Vec<String> = self.screens[si].items.keys().cloned().collect();
            for k in keys {
                let it = self.screens[si].items.get_mut(&k).unwrap();
                let on = hv.as_ref().is_some_and(|(hs, hk)| *hs == si && *hk == k);
                let swell = on && calm && !full && !is_pop(&k) && it.open.to == 0.0 && it.drag_place.is_none() && it.shown.to >= 1.0;
                it.swell.to = if swell { 1.0 } else { 0.0 };
                let swelled = it.swell.step_with(dt, SWELL_STIFF, SWELL_DAMP);
                let opened = it.open.step(dt);
                let rested = it.rest.step(dt);
                let moved = opened || rested || swelled;
                let alive = it.alive;
                if moved {
                    relayer = true;
                }
                // (a face was drawn already this tick)
                let drawn = face_drawn.iter().any(|(fs, fk)| *fs == si && *fk == k);
                let hidden = it.shown.now <= 0.01 && !it.shown.moving();
                if !drawn && !hidden && (moved || alive || (it.open.now == 0.0 && it.piece.alloc.1 as f32 > (it.slot.h + 12.0) * self.screens[si].s)) {
                    self.render(si, &k);
                }
            }
        }
        // the banner: one waiting while its widget was open comes out when it closes; one out goes when its widget opens
        if self.banner.is_none() && !self.queue.is_empty() {
            self.next_banner();
        }
        if let Some((p, say)) = self.banner.as_ref().filter(|b| !b.leaving).map(|b| (b.plugin.clone(), b.say.clone())) {
            if self.owner_open(&p, &say) {
                if let Some(b) = self.banner.as_mut() {
                    b.leaving = true;
                    b.slide.to = 0.0;
                }
                // (it waits to come out again, whole, when its widget closes; a live one is over by then)
                if say["pill"]["live"] != true {
                    self.queue.insert(0, (p, say));
                }
            }
        }
        if let Some(b) = &mut self.banner {
            let homing = b.home.as_mut().is_some_and(|h| h.step(dt));
            let moved = b.slide.step(dt) | homing;
            if !b.leaving && b.since.elapsed() >= Duration::from_millis(b.ms) {
                b.leaving = true;
                b.slide.to = 0.0;
            }
            let rows = (b.say["pill"]["rows"].is_array() && b.since.elapsed().as_secs_f32() < 2.0) || b.typing || b.marquee || b.glowing;
            let done = b.leaving && b.slide.now == 0.0;
            if moved || rows {
                relayer = true;
                self.render_banner();
            }
            if done {
                self.banner = None;
                relayer = true;
                for sc in &self.screens {
                    sc.hide_banner();
                }
                self.next_banner();
                if self.banner.is_none() {
                    self.relayout_with(Some(&HashSet::new()));
                }
            }
        }
        // now and then: the clock's minute, the config, full screen
        if self.slow_at.elapsed() >= Duration::from_millis(1000) {
            self.slow_at = Instant::now();
            if self.recover_device() {
                return;
            }
            let (minute, _) = peek::clock_text(false);
            if minute != self.minute {
                self.minute = minute;
                let clocks: Vec<String> = self.widgets.iter().filter(|(_, w)| w.content["peek"]["clock"].is_object()).map(|(k, _)| k.clone()).collect();
                for si in 0..self.screens.len() {
                    for k in &clocks {
                        if self.screens[si].items.contains_key(k) {
                            self.render(si, k);
                        }
                    }
                }
            }
            // (the taskbar moved, or changed size: the frame keeps clear of it)
            let before: Vec<Bx> = self.screens.iter().map(|s| s.area).collect();
            self.areas();
            if self.screens.iter().map(|s| s.area).collect::<Vec<_>>() != before {
                self.relayout();
            }
            if config::changed_since(self.config_when) {
                let (c, w) = config::load();
                self.set_config(c);
                self.config_when = w;
            }
            // (the edge reach's band: gone a moment after the slider was last moved)
            if self.reach_shown.as_ref().is_some_and(|(_, _, t)| t.elapsed() >= Duration::from_millis(1500)) {
                self.show_reach(0.0);
            }
            // (pinned from File Explorer's menu)
            self.take_pins();
            // the taskbar: kept hidden while it is wanted hidden (Explorer shows it again now and then), unless it peeks for its icons
            if self.config.taskbar == "hide" {
                crate::taskbar::peek_done();
                if !crate::taskbar::hidden() && !crate::taskbar::peeking() {
                    crate::taskbar::hide(self.reserve());
                    self.areas();
                    self.relayout();
                }
            }
            let launcher_out = self.launcher.as_ref().is_some_and(|l| l.shown);
            self.full_screen(!launcher_out);
            self.fronts_at = Instant::now();
            if let Some(l) = &self.launcher {
                l.raise();
            }
            self.dump();
        }
        let tc = Instant::now();
        if relayer {
            self.hit_regions();
        }
        // (told to the compositor only when something was drawn or moved: else nothing changed for it)
        if self.drew || relayer || self.timer_ms <= 16 {
            self.commit();
            self.prof_add("commit", tc);
        }
        self.drew = false;
        self.tick_rate();
    }

    pub(crate) fn prof_add(&mut self, what: &str, since: Instant) {
        let e = self.prof.entry(what.to_string()).or_default();
        e.0 += 1;
        e.1 += since.elapsed().as_secs_f64() * 1000.0;
    }

    /// A new config: screens made again when their size or number changed, the taskbar hidden or given back, and the frame laid out.
    /// Does NADI hold the tray (so other apps' icons come to it)?
    /// NADI holds the tray while its Tray widget is placed.
    pub fn tray_held(&self) -> bool {
        self.place_of(crate::builtin::PLUGIN, crate::builtin::TRAY).is_some()
    }

    /// Hold the tray, or give it back, as the config says.
    fn sync_tray(&mut self) {
        if self.tray_held() && !crate::trayhost::running() {
            crate::trayhost::start(self.msg_hwnd);
            crate::spawn_watchdog();
        } else if !self.tray_held() && crate::trayhost::running() {
            crate::trayhost::stop();
        }
        self.sync_tray_widget();
    }

    /// The tray's icons changed: the Tray widget drawn again.
    pub fn tray_changed(&mut self) {
        self.sync_tray_widget();
    }

    /// The Tray widget, while NADI holds the tray (gone when it does not).
    fn sync_tray_widget(&mut self) {
        use crate::builtin::{PLUGIN, TRAY};
        let have = self.widgets.contains_key(&key(PLUGIN, TRAY));
        if self.tray_held() {
            let content = crate::builtin::tray(&self.config.tray_pinned, self.edit.is_some());
            self.apply(vec![Update::Widget { plugin: PLUGIN.into(), widget: TRAY.into(), content }]);
        } else if have {
            self.apply(vec![Update::Widget { plugin: PLUGIN.into(), widget: TRAY.into(), content: Value::Null }]);
        }
    }

    pub fn set_config(&mut self, c: Config) {
        let screens = c.scale != self.config.scale;
        // (File Explorer's menu: on or off, and "Unpin from" for what is pinned now)
        if c.explorer_menu || self.config.explorer_menu {
            let (l, d) = explorer_paths(&c);
            crate::explorer::set(c.explorer_menu, &l, &d);
        }
        // (how liquid may have changed: each screen's liquid drawn again)
        for sc in &mut self.screens {
            sc.liquid_shapes.clear();
            sc.liquid_dirty = true;
        }
        let taskbar = c.taskbar != self.config.taskbar;
        self.config = c;
        crate::draw::set_accent(&self.config.theme);
        crate::draw::set_accent2(&self.config.theme2);
        crate::paint::set_roundness(self.config.roundness);
        crate::keyhook::set_desk_keys(self.config.desk_keys);
        crate::keyhook::set_enabled(self.config.win_launcher());
        self.sync_tray();
        if taskbar && self.config.taskbar == "hide" {
            crate::spawn_watchdog();
        }
        if taskbar {
            if self.config.taskbar == "hide" {
                crate::taskbar::hide(self.reserve());
            } else {
                crate::taskbar::restore();
            }
        }
        if screens {
            self.make_screens();
        } else {
            if self.config.monitors == "follow" || self.config.monitors.is_empty() {
                self.active = self.mouse_screen().unwrap_or(self.active);
            }
            self.areas();
            self.sync_builtins(true);
            self.relayout();
        }
    }

    /// A file, a shortcut or a folder pinned from File Explorer: to the dock, or to the Apps launcher. A program (or a shortcut to one)
    /// is pinned as the app; anything else opens as the PC opens it.
    /// What File Explorer's menu left to pin (or unpin): done.
    pub fn take_pins(&mut self) {
        let left = crate::explorer::take();
        if left.is_empty() {
            return;
        }
        for (to, path) in left {
            self.pin_path(&to, &path);
        }
        // (the launcher open: its pins as they are now)
        let pinned: Vec<crate::apps::App> = self.config.pinned.iter().map(|p| crate::apps::App { name: p.name.clone(), id: p.id.clone() }).collect();
        let pins = self.config.launcher_pins.clone().unwrap_or_else(crate::launcher::default_pins);
        if let Some(l) = self.launcher.as_mut().filter(|l| l.shown) {
            l.set_pins(pinned, pins);
            l.render(&self.g, &self.pc);
            self.commit();
        }
    }

    /// Pin (`dock`, `apps`) or unpin (`undock`, `unapps`) what is at `path`: a program, or a shortcut to one, as that program; else
    /// the file or folder itself.
    fn pin_path(&mut self, to: &str, path: &str) {
        let p = std::path::Path::new(path);
        if !p.exists() {
            return;
        }
        let program = if path.to_lowercase().ends_with(".exe") { Some(path.to_string()) } else { crate::explorer::shortcut_program(path) };
        let name = match &program {
            Some(e) => nadi_core::sensors::exeinfo::lookup(e).name.unwrap_or_else(|| std::path::Path::new(e).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
            None if p.is_dir() || path.to_lowercase().ends_with(".lnk") || path.to_lowercase().ends_with(".url") => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            None => p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        };
        let path = path.to_string();
        let lnk = path.to_lowercase().ends_with(".lnk");
        // (a shortcut: the name of the one pinned)
        let name = if lnk { p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name) } else { name };
        // (unpinned: what it was pinned as, gone)
        if to == "undock" || to == "unapps" {
            let ids: Vec<String> = [program.clone(), Some(path.clone())].into_iter().flatten().chain(crate::explorer::sources().into_iter().filter(|(_, s, _)| s.eq_ignore_ascii_case(&path)).map(|(_, _, i)| i)).collect();
            let is = |id: &str| ids.iter().any(|i| i.eq_ignore_ascii_case(id));
            self.change_config(|c| {
                if to == "undock" {
                    c.pinned.retain(|x| !is(&x.id));
                } else if let Some(pins) = c.launcher_pins.as_mut() {
                    pins.retain(|x| !((x.kind == "app" || x.kind == "file") && is(&x.id)));
                }
            });
            return;
        }
        // (a shortcut pinned as itself, as NADI's own copy of it: it works on when the one pinned is deleted or moved; the one pinned
        // remembered, so File Explorer's menu offers to unpin it)
        let kept = if lnk { crate::explorer::keep_shortcut(&path) } else { None };
        if let Some(k) = &kept {
            crate::explorer::remember(if to == "dock" { "dock" } else { "apps" }, &path, k);
        }
        let lnk_id = kept.clone().unwrap_or(path.clone());
        self.change_config(|c| {
            if to == "dock" {
                // (a shortcut as itself: what it starts with kept, its own name)
                let id = if lnk { lnk_id.clone() } else { program.clone().unwrap_or(path.clone()) };
                let name = name.clone();
                if !c.pinned.iter().any(|x| x.id.eq_ignore_ascii_case(&id)) {
                    c.pinned.push(crate::config::Pin { name, id, icon: String::new() });
                }
            } else {
                let pins = c.launcher_pins.get_or_insert_with(crate::launcher::default_pins);
                // (a shortcut pinned as itself: what it starts with (its arguments, its folder) and its own name kept; a program, as
                // the app)
                let (kind, id, name) = match &program {
                    Some(e) if !lnk => ("app", e.clone(), name.clone()),
                    _ => ("file", if lnk { lnk_id.clone() } else { path.clone() }, name.clone()),
                };
                if !pins.iter().any(|x| x.kind == kind && x.id.eq_ignore_ascii_case(&id)) {
                    pins.push(crate::config::AppPin { kind: kind.into(), id, name, folder: String::new(), icon: String::new() });
                }
            }
        });
    }

    /// The edge reach's band on the screen (the one in front), as thick as it is being set: a soft frame of the theme's colour, its
    /// inner edge a line. 0: gone. It goes by itself a moment after the slider was last moved.
    fn show_reach(&mut self, px: f32) {
        if px <= 0.0 {
            if let Some((si, p, _)) = self.reach_shown.take() {
                if let Some(sc) = self.screens.get(si) {
                    sc.ov.remove_top(&p);
                }
            }
            self.commit();
            return;
        }
        let si = self.active.min(self.screens.len().saturating_sub(1));
        let Some(sc) = self.screens.get(si) else { return };
        let s = sc.s;
        let m = sc.ov.monitor.rect;
        let (pw, ph) = ((m.right - m.left) as u32, (m.bottom - m.top) as u32);
        if self.reach_shown.as_ref().map_or(true, |(rs, _, _)| *rs != si) {
            if let Some((os, p, _)) = self.reach_shown.take() {
                if let Some(o) = self.screens.get(os) {
                    o.ov.remove_top(&p);
                }
            }
            // (on the glass over the frame's window: seen across the screen, the mouse going through it)
            let Ok(p) = sc.ov.top_piece(&self.g, pw, ph) else { return };
            self.reach_shown = Some((si, p, Instant::now()));
        }
        let a = sc.area;
        let k = px.clamp(2.0, 64.0);
        let surface = {
            let Some((_, piece, at)) = self.reach_shown.as_mut() else { return };
            *at = Instant::now();
            piece.surface.clone()
        };
        let painter = self.painter(s);
        let c = crate::draw::accent();
        let _ = self.g.draw_surface(&surface, |_| {
            let band = c.alpha(0.22);
            painter.round(Rect::new(a.x, a.y, a.w, k), 0.0, band);
            painter.round(Rect::new(a.x, a.y + a.h - k, a.w, k), 0.0, band);
            painter.round(Rect::new(a.x, a.y + k, k, a.h - 2.0 * k), 0.0, band);
            painter.round(Rect::new(a.x + a.w - k, a.y + k, k, a.h - 2.0 * k), 0.0, band);
            painter.outline(Rect::new(a.x + k, a.y + k, a.w - 2.0 * k, a.h - 2.0 * k), 8.0, 2.0, c.alpha(0.9));
        });
        drop(painter);
        if let Some((_, piece, _)) = self.reach_shown.as_mut() {
            let _ = piece.place(0, 0);
        }
        self.commit();
    }

    /// The config, changed here (a menu, the settings) and kept.
    fn change_config(&mut self, f: impl FnOnce(&mut Config)) {
        let mut c = self.config.clone();
        f(&mut c);
        if c.pinned != self.config.pinned || c.launcher_pins != self.config.launcher_pins || c.launcher_foot != self.config.launcher_foot {
            crate::explorer::prune_kept(&kept_paths(&c));
        }
        config::save(&c);
        self.config_when = std::fs::metadata(config::path()).and_then(|m| m.modified()).ok();
        self.set_config(c);
        self.push_settings();
    }

    /// What the dock keeps free at the bottom of the screens while the taskbar is hidden.
    fn reserve(&self) -> f32 {
        if !self.floating(&key(crate::builtin::PLUGIN, crate::builtin::DOCK)) && self.place_of(crate::builtin::PLUGIN, crate::builtin::DOCK).map(|(p, _)| p) == Some(Place::Edge(Edge::Bottom)) {
            self.even_depth(&key(crate::builtin::PLUGIN, crate::builtin::DOCK))
        } else {
            0.0
        }
    }

    pub fn taskbar_wanted_hidden(&self) -> bool {
        self.config.taskbar == "hide"
    }

    /// (for checking: with `NADI_DUMP` set, what the frame shows is written there now and then, as `--preview` reads it)
    fn dump(&self) {
        let Some(path) = std::env::var_os("NADI_DUMP") else { return };
        let mut list = Vec::new();
        for sc in &self.screens {
            for (k, it) in &sc.items {
                let Some(w) = self.widgets.get(unpop(k)) else { continue };
                let place = self.place_of(&w.plugin, &w.id).map(|(p, _)| p.name()).unwrap_or("");
                let rb = it.rest.now();
                let rest_box = vec![rb.x, rb.y, rb.w, rb.h];
                let buttons: Vec<Value> = it.buttons.iter().map(|(r, a)| json!([r.x, r.y, r.w, r.h, a])).collect();
                list.push(json!({ "key": k, "monitor": sc.ov.monitor.name(), "place": place, "rank": self.place_of(&w.plugin, &w.id).map(|(_, r)| r).unwrap_or(0), "open": it.open.to == 1.0, "dot": it.slot.dot, "floating": it.floating, "shown": it.shown.now, "drag": it.drag_place.map(|p| p.name()), "drag_float": it.drag_float, "drag_down": it.drag_down, "drag_head": it.drag_head, "shape_box": self.shape_now(0, it).ok().map(|(_, b)| vec![b.x, b.y, b.w, b.h]), "in_liquid": self.in_liquid(it), "box": rest_box, "buttons": buttons, "content": w.content }));
            }
        }
        let banner = self.banner.as_ref().map(|b| json!({ "plugin": b.plugin, "say": b.say, "at": b.at.map(|a| [a.x, a.y, a.w, a.h]) }));
        let moving: Vec<String> = self.screens.iter().flat_map(|s| s.items.iter().filter(|(_, i)| i.open.moving() || i.rest.moving() || i.alive).map(|(k, _)| k.clone())).collect();
        let prof: serde_json::Map<String, Value> = self.prof.iter().map(|(k, (n, ms))| (k.clone(), json!([n, (ms * 10.0).round() / 10.0]))).collect();
        let out = json!({ "prof": prof, "settings_open": self.settings.is_some(), "settings_heard": crate::settings::HEARD.load(Ordering::Relaxed), "timer_ms": self.timer_ms, "edit": self.edit_dump(), "menu": self.menu.as_ref().map(|m| json!({ "key": m.key, "at": [m.at.x, m.at.y, m.at.w, m.at.h], "hits": m.hits.len() })), "tray_forwarded": [crate::trayhost::FORWARDED.load(Ordering::Relaxed), crate::trayhost::UNANSWERED.load(Ordering::Relaxed)], "tray": crate::trayhost::icons().iter().map(|i| json!({ "tip": i.tip, "pid": i.pid, "hidden": i.hidden, "icon": i.icon != 0, "version": i.version, "callback": i.callback })).collect::<Vec<_>>(), "ghost": self.ghost.as_ref().map(|g| { let b = g.rest.now(); [b.x, b.y, b.w, b.h] }), "screens": self.screens.iter().map(|s| json!({ "area": [s.area.x, s.area.y, s.area.w, s.area.h], "full": s.full, "maxed": s.maxed })).collect::<Vec<_>>(), "moving": moving, "widgets": list, "banner": banner, "queue": self.queue.len(), "plugins": self.plugins.iter().map(|p| json!({ "id": p.id, "on": p.on, "note": p.note, "problem": p.problem })).collect::<Vec<_>>() });
        let _ = nadi_core::paths::write_atomic(std::path::Path::new(&path), serde_json::to_string_pretty(&out).unwrap_or_default().as_bytes());
    }

    /// How often to tick: every frame while something moves, else four times a second.
    fn tick_rate(&mut self) {
        if let Some(l) = &self.launcher {
            l.keep_mouse_free();
        }
        let launcher_moves = self.launcher.as_ref().is_some_and(|l| l.moving()) || self.modal.as_ref().is_some_and(|m| m.moving());
        let banner_moves = launcher_moves || self.banner.as_ref().is_some_and(|b| b.slide.moving() || b.home.is_some_and(|h| h.moving()) || b.typing || b.marquee || b.glowing || (b.say["pill"]["rows"].is_array() && b.since.elapsed().as_secs_f32() < 2.0));
        let rolling = self.rolls.borrow().values().any(|r| r.2.elapsed().as_secs_f32() < ROLL_FOR + 0.1);
        let rolling = rolling || self.dock_front.as_ref().is_some_and(|f| f.2.elapsed().as_secs_f32() < DOCK_FRONT_S + 0.05);
        let moving = banner_moves || rolling || self.scrollbar_moves().is_some() || self.hold.is_some() || self.tip.as_ref().is_some_and(|t| t.grow.moving() || t.moving) || self.menu.as_ref().is_some_and(|m| m.grow.moving() || m.marquee) || self.ghost.as_ref().is_some_and(|g| g.rest.moving()) || self.screens.iter().any(|s| s.items.values().any(|i| i.open.moving() || i.rest.moving() || i.swell.moving())) || self.pops.values().any(|m| m.anim.is_some() || (m.strip_a > 0.0 && m.strip_a < 1.0)) || self.tear.is_some();
        let alive = self.files_dragging || self.screens.iter().any(|s| s.items.values().any(|i| (i.alive && i.shown.now > 0.01) || i.shown.moving())) || self.hover.is_some() || self.press.is_some() || self.drag.is_some();
        // (widgets that show as the mouse comes: the mouse is looked at often)
        let watching = self.menu.is_some() || self.edit.is_some() || !self.pops.is_empty() || (self.click_to_open() && self.screens.iter().any(|s| s.items.values().any(|i| i.open.to == 1.0))) || !(self.config.reveal.is_empty() || self.config.reveal == "all") || self.screens.iter().any(|s| s.full || (s.maxed && self.config.auto_hide == "maximized"));
        let ms = if moving { 16 } else if alive { 33 } else if watching { 50 } else { 250 };
        if ms != self.timer_ms {
            self.timer_ms = ms;
            unsafe {
                SetTimer(self.msg_hwnd, 1, ms, None);
            }
        }
    }

    fn set_open(&self, si: usize, k: &str, open: bool) {
        let _ = si;
        if let Some(w) = self.widgets.get(unpop(k)) {
            let _ = self.tx.send(Command::Open { plugin: w.plugin.clone(), widget: w.id.clone(), open });
        }
    }

    /// Is an app full screen in front on each monitor (a game, a video)? Over one, the frame is shown only while widgets are out (the
    /// mouse came to an edge), and brought to the top once, as it shows: kept on top in a loop, a window over a full-screen game makes
    /// the graphics driver switch how the game is shown again and again, and that can hang it. With "never", it stays shown.
    /// The top window of each monitor is looked at (focused or not); `raise`: the frame is put back on top where nothing is full screen.
    fn full_screen(&mut self, raise: bool) {
        // (another app's menu or popup in front, on top of everything: the frame stays under it, not put back over it)
        let menu = crate::winlist::other_on_top();
        // (Windows' own see-through panels open: the glows taken off the glass, they would show through; the glass itself stays, the
        // faces of the widgets at an edge are on it)
        let shell = crate::winlist::shell_panel_open();
        if shell != self.glass_hidden {
            self.glass_hidden = shell;
            if shell {
                for sc in &mut self.screens {
                    let gone: Vec<_> = sc.items.values_mut().filter_map(|it| it.glow.take()).collect();
                    for g in &gone {
                        sc.ov.remove_glass(g);
                    }
                    if let Some(gp) = &sc.banner_glow {
                        unsafe {
                            let _ = gp.visual.SetOffsetY2(-1000.0);
                        }
                    }
                }
            }
        }
        let raise = raise && !menu;
        let fronts = crate::winlist::fronts();
        self.keys_hidden = self.config.hold_key != "none" && crate::winlist::keys_hidden_in_front();
        {
            let never = self.config.auto_hide == "never";
            for sc in &mut self.screens {
                let front = fronts.iter().find(|(m, _)| *m == sc.ov.monitor.handle.0 as isize).map(|(_, f)| *f).unwrap_or_default();
                let full = front.full;
                sc.maxed = front.maxed;
                let became = full && !sc.full;
                sc.full = full;
                if !full {
                    let showing = !sc.win_shown;
                    if showing {
                        sc.win_shown = true;
                        sc.ov.show(true);
                    }
                    if raise || showing {
                        sc.ov.raise();
                    }
                } else if never && became {
                    // (always shown: on top of it, once)
                    if !sc.win_shown {
                        sc.win_shown = true;
                        sc.ov.show(true);
                    }
                    sc.ov.raise();
                    sc.raised_at = Some(Instant::now());
                }
                // (never put back on top over a full-screen app again and again: a game that keeps coming over the frame, raised over
                // each time, made the graphics driver switch how it was shown until it hung the PC)
            }
        }
        // (the box being typed in stays over them)
        if let Some(ib) = &self.input {
            ib.raise();
        }
    }

    // ----------------------------------------------------------------------------------------------------------- input

    /// The item under a point of a screen (logical px): an open one first.
    fn item_at(&self, si: usize, x: f32, y: f32) -> Option<String> {
        let sc = &self.screens[si];
        let mut best: Option<(f32, String)> = None;
        for (k, it) in &sc.items {
            if let Ok((_, b)) = self.shape_now(si, it) {
                if b.contains(x, y) && best.as_ref().map_or(true, |(o, _)| it.open.now > *o) {
                    best = Some((it.open.now, k.clone()));
                }
            }
        }
        best.map(|(_, k)| k)
    }

    fn in_banner(&self, si: usize, x: f32, y: f32) -> bool {
        let Some(b) = &self.banner else { return false };
        Some(si) == self.banner_screen() && b.at.is_some_and(|a| a.contains(x, y))
    }

    /// The left button went down: on a widget, it may be a press or the start of a drag.
    pub fn mouse_down(&mut self, si: usize, x: f32, y: f32) {
        // (on a free side of a panel: it is resized)
        if !self.menu_open() && self.size_press(si, x, y) {
            return;
        }
        // (on the menu: a press of it, never the start of a drag; a held row starts filling)
        if self.in_menu(si, x, y) {
            if let Some(i) = self.menu_hit(si, x, y) {
                let m = self.menu.as_ref().unwrap();
                if let (Some((plugin, items)), Some((r, _))) = (&m.items, m.hits.get(i)) {
                    if let Some(it) = items.get(i).filter(|it| it["hold"] == true) {
                        let spec = confirm_spec(it["icon"].as_str().unwrap_or("bang"), it["label"].as_str().unwrap_or(""), it["confirm"].as_str().unwrap_or(""), it["action"].as_str().unwrap_or(""));
                        self.hold = Some(Hold { si, key: None, row: Some(i), rect: *r, plugin: plugin.clone(), action: it["action"].as_str().unwrap_or("").to_string(), spec });
                        self.tick_rate();
                    }
                }
            }
            return;
        }
        // (a window's grip or switch; an open panel pressed where nothing is, to be pulled off its edge)
        if self.pop_press(si, x, y) {
            self.tick_rate();
            return;
        }
        self.tear_press(si, x, y);
        // (a held thing in an open panel starts filling)
        if let Some(k) = self.item_at(si, x, y) {
            let it = &self.screens[si].items[&k];
            if it.open.now > 0.6 {
                if let Ok((_, b)) = self.shape_now(si, it) {
                    let (lx, ly) = (x - b.x, y - b.y);
                    // (a picture to look at: dragged, it moves with the mouse)
                    let view = it.hits.iter().find(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).and_then(|h| match &h.press {
                        Press::View(id) => Some(id.clone()),
                        _ => None,
                    });
                    // (a desktop's card: held a moment, it is carried; pressed quickly, its action; dragged at once, the map moves)
                    let card = it.hits.iter().find(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).and_then(|h| match &h.press {
                        Press::Card(a, id) => Some((a.clone(), id.clone())),
                        _ => None,
                    });
                    if let Some((a, id)) = card {
                        self.card_press = Some((si, k.clone(), id, a, x, y, Instant::now(), false));
                        unsafe {
                            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.screens[si].ov.hwnd);
                        }
                        self.tick_rate();
                        return;
                    }
                    if let Some(id) = view {
                        self.pan = Some((si, k.clone(), id, x, y, false));
                        unsafe {
                            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.screens[si].ov.hwnd);
                        }
                        self.tick_rate();
                        return;
                    }
                    // (a slider, or a bar to seek on: its knob jumps here and follows the mouse until let go)
                    let seek = it.hits.iter().find(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).and_then(|h| match &h.press {
                        Press::Seek(a) => Some((a.clone(), h.rect)),
                        _ => None,
                    });
                    if let (Some((action, rect)), Some(w)) = (seek, self.widgets.get(unpop(&k))) {
                        let frac = slide_frac(rect, lx);
                        let live = blocks::slider_value(&action).is_some();
                        let plugin = w.plugin.clone();
                        self.slide = Some(Slide { si, key: k.clone(), plugin: plugin.clone(), action: action.clone(), rect, frac, told: Instant::now(), told_frac: frac, live });
                        if live {
                            self.act(plugin, action.clone(), json!({ "fraction": frac }));
                        }
                        blocks::set_sliding(Some((action, frac)));
                        unsafe {
                            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.screens[si].ov.hwnd);
                        }
                        self.render(si, &k);
                        self.commit();
                        self.tick_rate();
                        return;
                    }
                    let held = it.hits.iter().find(|h| h.hold && lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h);
                    if let (Some(h), Some(w)) = (held, self.widgets.get(unpop(&k))) {
                        if let Press::Action(a, _) = &h.press {
                            let label = match &h.tip {
                                Value::String(t) => t.trim_start_matches("Hold to ").to_string(),
                                _ => String::new(),
                            };
                            let spec = confirm_spec("bang", &label, "", a);
                            self.hold = Some(Hold { si, key: Some(k.clone()), row: None, rect: h.rect, plugin: w.plugin.clone(), action: a.clone(), spec });
                            self.tick_rate();
                            return;
                        }
                    }
                }
            }
        }
        if self.edit_press(si, x, y) {
            self.tick_rate();
            return;
        }
        if self.in_banner(si, x, y) {
            return;
        }
        self.press = self.item_at(si, x, y).filter(|k| !is_pop(k)).map(|k| (si, k, x, y, Instant::now()));
        self.mouse = (si, x, y);
        self.tick_rate();
    }

    /// Where a widget would go if it were let go at `x`, `y`: an edge or a corner, and whether it floats there (not near enough to
    /// squish into it). Near both edges of a corner, or over the one in it: the corner itself. Along an edge, over a corner's row or
    /// column, or a little past its end: one of that row or column.
    fn drop_place(&self, si: usize, x: f32, y: f32) -> Spot {
        let a = self.screens[si].area;
        let (dl, dr, dt, db) = (x - a.x, a.x + a.w - x, y - a.y, a.y + a.h - y);
        let held = self.drag.as_ref().map(|(_, k, _, _)| k.clone());
        let left = |c: Corner| matches!(c, Corner::TopLeft | Corner::BottomLeft);
        let top = |c: Corner| matches!(c, Corner::TopLeft | Corner::TopRight);
        let dx = |c: Corner| if left(c) { dl } else { dr };
        let dy = |c: Corner| if top(c) { dt } else { db };
        // what a corner holds now (not the one held): the box of the one in it, and how far its row and its column reach
        let holds = |c: Corner| -> (Option<Bx>, f32, f32) {
            let (mut piece, mut row, mut col) = (None, 0.0f32, 0.0f32);
            for (k, it) in &self.screens[si].items {
                if held.as_ref() == Some(k) || !self.widgets.get(unpop(k)).and_then(|w| self.place_of(&w.plugin, &w.id)).is_some_and(|(p, _)| p == Place::Corner(c)) {
                    continue;
                }
                let b = it.rest.now();
                let rx = if left(c) { b.x + b.w - a.x } else { a.x + a.w - b.x };
                let ry = if top(c) { b.y + b.h - a.y } else { a.y + a.h - b.y };
                match it.slot.place {
                    Place::Corner(_) => {
                        piece = Some(b);
                        row = row.max(rx);
                        col = col.max(ry);
                    }
                    Place::Edge(Edge::Left | Edge::Right) => col = col.max(ry),
                    _ => row = row.max(rx),
                }
            }
            (piece, row, col)
        };
        let corners = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];
        for c in corners {
            let (piece, _, _) = holds(c);
            let over = piece.is_some_and(|b| x >= b.x - 10.0 && x <= b.x + b.w + 10.0 && y >= b.y - 10.0 && y <= b.y + b.h + 10.0);
            // (an empty corner: near both its edges; one with a widget in it: over that one, so that its row and column can be reached)
            if over || (piece.is_none() && dx(c) < CORNER_ZONE && dy(c) < CORNER_ZONE) {
                let near = if over { 0.0 } else { dy(c).max(dx(c)).min(dy(c).min(dx(c)) + 40.0) };
                return Spot { place: Place::Corner(c), float: near > SQUISH, down: false, head: true };
            }
        }
        let m = dl.min(dr).min(dt).min(db);
        let edge = if m == dt {
            Edge::Top
        } else if m == db {
            Edge::Bottom
        } else if m == dl {
            Edge::Left
        } else {
            Edge::Right
        };
        let (c, down) = match edge {
            Edge::Top => (if dl < dr { Corner::TopLeft } else { Corner::TopRight }, false),
            Edge::Bottom => (if dl < dr { Corner::BottomLeft } else { Corner::BottomRight }, false),
            Edge::Left => (if dt < db { Corner::TopLeft } else { Corner::BottomLeft }, true),
            Edge::Right => (if dt < db { Corner::TopRight } else { Corner::BottomRight }, true),
        };
        let (piece, row, col) = holds(c);
        let (along, reach, near) = if down { (dy(c), col, dx(c)) } else { (dx(c), row, dy(c)) };
        if piece.is_some() && along < reach + 60.0 {
            return Spot { place: Place::Corner(c), float: near > SQUISH, down, head: false };
        }
        Spot { place: Place::Edge(edge), float: m > SQUISH, down: false, head: false }
    }

    /// The box a dragged widget takes: floating, round the mouse; squished, against its edge where the mouse is along it.
    fn drag_box(&self, si: usize, k: &str, x: f32, y: f32, gx: f32, gy: f32, spot: Spot) -> Bx {
        let float = spot.float;
        let m = Metrics::default();
        let a = self.screens[si].area;
        let p = self.painter(self.screens[si].s);
        // (in a corner's row or column, not the corner itself: along that edge, as one of them). Held, it is already the shape and
        // size it will have there: its length for that edge, as deep as it stands there (large, a square icon, the dock's depth)
        let place = drag_look(spot.place, spot.head, spot.down);
        let (along, depth) = self.sized(si, &p, k, place, matches!(place, Place::Corner(_)));
        let (w, h) = match place {
            Place::Edge(Edge::Left | Edge::Right) => (depth, along + 2.0 * m.fillet),
            Place::Corner(_) => (along + m.fillet, depth + m.fillet),
            _ => (along + 2.0 * m.fillet, depth),
        };
        if float {
            // (held where the mouse took it, kept inside the screen: its own size, no shoulders, a corner's as any other's)
            let (fw, fh) = if place.horizontal() { (along, depth) } else { (depth, along) };
            let gx = gx.clamp(0.0, fw);
            let gy = gy.clamp(0.0, fh);
            return Bx { x: (x - gx).clamp(a.x, a.x + a.w - fw), y: (y - gy).clamp(a.y, a.y + a.h - fh), w: fw, h: fh };
        }
        match place {
            Place::Edge(Edge::Top) => Bx { x: (x - w / 2.0).clamp(a.x, a.x + a.w - w), y: a.y, w, h },
            Place::Edge(Edge::Bottom) => Bx { x: (x - w / 2.0).clamp(a.x, a.x + a.w - w), y: a.y + a.h - h, w, h },
            Place::Edge(Edge::Left) => Bx { x: a.x, y: (y - h / 2.0).clamp(a.y, a.y + a.h - h), w, h },
            Place::Edge(Edge::Right) => Bx { x: a.x + a.w - w, y: (y - h / 2.0).clamp(a.y, a.y + a.h - h), w, h },
            Place::Corner(c) => {
                let x = if matches!(c, Corner::TopLeft | Corner::BottomLeft) { a.x } else { a.x + a.w - w };
                let y = if matches!(c, Corner::TopLeft | Corner::TopRight) { a.y } else { a.y + a.h - h };
                Bx { x, y, w, h }
            }
        }
    }

    /// How deep a widget stands at rest: the settings' depth for its size, or its plugin's own (its peek's `depth`, else its manifest's).
    fn even_depth(&self, k: &str) -> f32 {
        let large = self.large(k);
        let (small_d, large_d) = (self.config.depth_small.clamp(24.0, 48.0), self.config.depth_large.clamp(32.0, 64.0));
        let even = if large { large_d.max(small_d) } else { small_d };
        let pick = |v: &Value| -> Option<f32> {
            let n = if v.is_object() { v[if large { "large" } else { "small" }].as_f64() } else { v.as_f64() };
            n.map(|d| (d as f32).clamp(20.0, 120.0))
        };
        let Some(w) = self.widgets.get(unpop(k)) else { return even };
        if let Some(d) = pick(&w.content["peek"]["depth"]) {
            return d;
        }
        let spec = self.plugins.iter().find(|p| p.id == w.plugin).and_then(|p| p.widgets.iter().find(|s| s.names(&w.id)));
        spec.and_then(|s| pick(&s.depth)).unwrap_or(even)
    }

    /// A widget's size at a place: how long it is along that edge (its peek, measured for it; a square icon as long as it is deep;
    /// in a corner's own place at most so long), and how deep it stands out of it (the dock's own depth, large deeper).
    fn sized(&self, si: usize, p: &Painter, k: &str, look: Place, corner: bool) -> (f32, f32) {
        let m = Metrics::default();
        let Some(w) = self.widgets.get(unpop(k)) else { return (m.depth_h, m.depth_h) };
        let mut want = peek::measure(p, &self.peek_of(k), look.horizontal());
        if corner {
            want = want.min(m.corner_max);
        }
        // (every widget as deep as the others of its size, on every edge, the dock too: the settings' depth, or its plugin's own)
        let base = self.even_depth(k);
        // (the desktops' map: as deep as its rows need, every one shown)
        let base = if w.content["peek"]["desks"].is_object() && !self.icon_alone(k) { base.max(peek::desk_depth(&self.peek_of(k), look.horizontal()).round()) } else { base };
        // (on the edge the taskbar stands on, under the widgets: as deep as it, over all of it)
        let edge = match look {
            Place::Edge(e) => e,
            Place::Corner(Corner::TopLeft | Corner::TopRight) if look.horizontal() => Edge::Top,
            Place::Corner(Corner::BottomLeft | Corner::BottomRight) if look.horizontal() => Edge::Bottom,
            Place::Corner(Corner::TopLeft | Corner::BottomLeft) => Edge::Left,
            Place::Corner(_) => Edge::Right,
        };
        let base = match self.screens.get(si).and_then(|sc| sc.bar) {
            Some((e, d)) if e == edge && !self.floating(k) => base.max(d.round()),
            _ => base,
        };
        // (words in more than one line, a subtitle: as deep as they need)
        let base = match peek::depth_of(p, &self.peek_of(k), want) {
            Some(d) if look.horizontal() => d.max(base),
            _ => base,
        };
        // (on a side edge, with no words: as long as it is deep, a square, not a column)
        let pk = &w.content["peek"];
        let wordless = ["text", "sub", "clock", "bars", "vessel", "roll", "desks", "cells", "subtitle"].iter().all(|f| pk[*f].is_null() || pk[*f] == "");
        // (as deep as on a top or bottom edge, not the side's deeper strip: a square the same size on every edge)
        if !look.horizontal() && wordless && !(pk["dock"].is_object() || pk["tray"].is_object() || pk["face"].is_string() || pk["desks"].is_object()) {
            let sq = self.even_depth(k);
            return (sq, sq);
        }
        // (its icon alone: square, as long as it is deep; the same size on every edge, a side's too)
        if self.icon_alone(k) && !pk["face"].is_string() {
            let base = self.even_depth(k);
            return (base, base);
        }
        let want = match self.screens.get(si) {
            Some(sc) if pk["dock"].is_object() && !look.horizontal() => want.min(sc.area.h * 0.9),
            _ => want,
        };
        (want, base)
    }

    /// Held long enough without moving, as on a phone: the widget is picked up (it pops, and wobbles), and follows the mouse from here.
    fn pick_up(&mut self) {
        let Some((si, k, px, py, at)) = self.press.clone() else { return };
        if at.elapsed() < HOLD {
            return;
        }
        let Some(it) = self.screens.get_mut(si).and_then(|s| s.items.get_mut(&k)) else { return };
        let b = it.rest.now();
        it.open.to = 0.0;
        it.pinned = false;
        // (the pop)
        it.rest.w.vel += 160.0;
        it.rest.h.vel += 90.0;
        it.rest.x.vel -= 80.0;
        // (on top of the others while it is held)
        let visual = it.piece.visual.clone();
        self.screens[si].ov.to_front(&visual);
        self.drag = Some((si, k.clone(), px - b.x, py - b.y));
        if self.edit.is_none() {
            self.enter_edit(si);
        }
        self.drag_last = None;
        self.drag_at = None;
        self.press = None;
        self.hover = None;
        self.set_open(si, &k, false);
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.screens[si].ov.hwnd);
        }
        self.relayout_with(Some(&HashSet::new()));
    }

    /// The mouse moved with a widget picked up: it follows. (Moved before the hold: no drag, the press is let go.)
    fn drag_move(&mut self, si: usize, x: f32, y: f32) -> bool {
        self.mouse = (si, x, y);
        if let Some((ps, _, px, py, _)) = &self.press {
            if *ps != si || (x - px).hypot(y - py) > DRAG_START {
                self.press = None;
            }
        }
        let Some((ds, k, gx, gy)) = self.drag.clone() else { return false };
        if ds != si {
            return true;
        }
        let spot = self.drop_place(si, x, y);
        let b = self.drag_box(si, &k, x, y, gx, gy, spot);
        let join = self.join_for(si, &k, spot, x, y);
        let join_changed = self.drag_join.as_ref() != Some(&join);
        self.drag_join = Some(join);
        self.drag_at = Some((spot, x, y));
        if let Some(it) = self.screens[si].items.get_mut(&k) {
            it.drag_place = Some(spot.place);
            it.drag_float = spot.float;
            it.drag_down = spot.down;
            it.drag_head = spot.head;
            it.rest.set(b);
        }
        // (what is on that edge makes room as it would: laid out again when where it would go changed enough)
        let along = if drag_look(spot.place, spot.head, spot.down).horizontal() { x } else { y };
        let moved = join_changed
            || match self.drag_last {
                Some((p, a)) => p != spot || (a - along).abs() > 12.0,
                None => true,
            };
        if moved {
            self.drag_last = Some((spot, along));
            self.relayout_with(Some(&HashSet::new()));
        }
        self.dump();
        self.tick_rate();
        true
    }

    /// Let go: the widget goes where it was dropped — its edge or corner, merged or floating, among the others by where it is along the
    /// edge — and the config keeps it.
    fn drop(&mut self, si: usize, x: f32, y: f32) {
        let Some((ds, k, _, _)) = self.drag.take() else { return };
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
        }
        // (where the ghost was: let go off its screen, or before it moved, where the mouse is)
        let at = self.drag_at.take();
        // (while editing, let go over the tray: hidden)
        if si == ds && self.in_edit_tray(ds, x, y) {
            if let Some(it) = self.screens[ds].items.get_mut(&k) {
                it.drag_place = None;
                it.drag_down = false;
                it.drag_head = false;
                it.drag_float = false;
            }
            if let Some(e) = &mut self.edit {
                e.over_tray = false;
            }
            let loose = self.joined_to(&k);
            self.drag_join = None;
            self.change_config(|c| {
                let w = c.widgets.entry(k.clone()).or_default();
                w.place = Some("off".into());
                w.join = None;
                for l in &loose {
                    c.widgets.entry(l.clone()).or_default().join = None;
                }
            });
            self.render_edit_tray();
            return;
        }
        let (spot, x, y) = at.unwrap_or_else(|| (self.drop_place(ds, x, y), x, y));
        let (place, float) = (spot.place, spot.float);
        if let Some(it) = self.screens[ds].items.get_mut(&k) {
            it.drag_place = None;
            it.drag_down = false;
            it.drag_head = false;
            it.drag_float = false;
            it.floating = float;
        }
        // the order there: by where each is along it (in a corner: the one in it first, then by how far from it), the dropped one
        // where it was let go
        let a = self.screens[ds].area;
        let mut row: Vec<(f32, String)> = self.screens[ds]
            .items
            .iter()
            .filter(|(o, _)| **o != k && self.widgets.get(*o).and_then(|w| self.place_of(&w.plugin, &w.id)).is_some_and(|(p, _)| p == place))
            .map(|(o, it)| (order_in(place, it.slot.place, it.rest.now(), a), o.clone()))
            .collect();
        row.push((held_order(spot, x, y, a), k.clone()));
        row.sort_by(|a, b| a.0.total_cmp(&b.0));
        let name = place.name().to_string();
        // (let go against the one before it, or the one after it: joined to it, no gap between; else not: as it showed while held)
        let (join_before, after) = self.drag_join.take().unwrap_or_else(|| self.join_for(ds, &k, spot, x, y));
        let (after_key, join_after) = match after {
            Some((key, j)) => (Some(key), j),
            None => (None, false),
        };
        // (the one that was joined to it where it was: joined to nothing now)
        let loose = self.joined_to(&k);
        self.change_config(|c| {
            for l in &loose {
                c.widgets.entry(l.clone()).or_default().join = None;
            }
            for (i, (_, key)) in row.iter().enumerate() {
                let w = c.widgets.entry(key.clone()).or_default();
                w.rank = Some(i as i32 * 10);
                w.place = Some(name.clone());
                if *key == k {
                    w.style = Some(if float { "floating".into() } else { "merged".into() });
                    w.stack = if spot.down && !spot.head { Some("down".into()) } else { None };
                    w.join = join_before.clone();
                }
            }
            if let Some(ak) = &after_key {
                c.widgets.entry(ak.clone()).or_default().join = join_after.then(|| k.clone());
            }
        });
    }

    pub fn mouse_move(&mut self, si: usize, x: f32, y: f32) {
        // (the mouse onto one of the dock's grips, or off: its dot lit or not)
        let dock = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
        let g = self.grip_end(si, &dock, x, y);
        if g != self.grip_at {
            self.grip_at = g;
            if self.screens.get(si).is_some_and(|sc| sc.items.contains_key(&dock)) {
                self.render(si, &dock);
                self.commit();
            }
        }
        if self.size_move(si, x, y) {
            return;
        }
        if self.slide_move(si, x) {
            return;
        }
        if let Some((cs, ck, cid, _, x0, y0, _, carried)) = self.card_press.clone() {
            if cs == si {
                if carried {
                    if let Some(it) = self.screens[si].items.get(&ck) {
                        if let Ok((_, b)) = self.shape_now(si, it) {
                            let g = crate::blocks::desk_geom();
                            let (gx, gy) = g.map_or((70.0, 40.0), |g| (g.cell.0 * g.view.0 * 0.45, g.cell.1 * g.view.0 * 0.3));
                            crate::blocks::set_desk_carry(Some((cid, x - b.x, y - b.y, gx, gy)));
                        }
                    }
                    self.mouse = (si, x, y);
                    self.render(si, &ck);
                    self.commit();
                } else if (x - x0).abs() + (y - y0).abs() > 5.0 {
                    // (dragged before it was held long enough: the map moves instead)
                    self.card_press = None;
                    self.pan = Some((si, ck, crate::blocks::DESKMAP.to_string(), x0, y0, true));
                    crate::blocks::view_pan(crate::blocks::DESKMAP, x - x0, y - y0);
                    self.pan = Some((si, self.pan.as_ref().unwrap().1.clone(), crate::blocks::DESKMAP.to_string(), x, y, true));
                }
            }
            return;
        }
        if let Some((ps, k, id, px, py, moved)) = self.pan.clone() {
            if ps == si && (moved || (x - px).abs() + (y - py).abs() > 3.0) {
                crate::blocks::view_pan(&id, x - px, y - py);
                self.pan = Some((ps, k.clone(), id, x, y, true));
                self.render(si, &k);
                self.commit();
            }
            return;
        }
        if self.pop_move(si, x, y) {
            return;
        }
        if self.edit_move(si, x, y) {
            return;
        }
        if self.drag_move(si, x, y) {
            return;
        }
        // (over the menu: what is under the mouse lights up; the widgets under it are left alone)
        if self.menu_open() {
            let hit = self.menu_hit(si, x, y);
            if self.menu.as_ref().is_some_and(|m| m.hover != hit) {
                self.menu.as_mut().unwrap().hover = hit;
                self.render_menu();
                self.commit();
            }
            if self.in_menu(si, x, y) {
                unsafe {
                    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.screens[si].ov.hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                }
                return;
            }
        }
        if self.in_banner(si, x, y) {
            // (held under the mouse, a banner stays)
            if let Some(b) = &mut self.banner {
                if !b.leaving {
                    b.since = Instant::now();
                }
            }
            return;
        }
        let at = self.item_at(si, x, y);
        let now = at.as_ref().map(|k| (si, k.clone()));
        if now != self.hover {
            if let Some(k) = &at {
                if let Some(it) = self.screens[si].items.get_mut(k) {
                    it.hover_since = Some(Instant::now());
                    it.left_at = None;
                }
            }
            // (the one it left, pinned open by a press, shuts when the mouse comes back to another)
            let old = std::mem::replace(&mut self.hover, now);
            // (the dock's grip shows while the mouse is on it: drawn again as it comes or goes)
            let dock = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
            for (hs, hk) in old.iter().chain(self.hover.clone().iter()) {
                // (and a panel that scrolls: its scrollbar comes and goes with the mouse)
                // (and an open panel: its handle comes and goes with the mouse)
                let scrolls = self.screens.get(*hs).and_then(|sc| sc.items.get(hk)).is_some_and(|i| i.content_h > i.visible + 1.0 || i.open.now > 0.5);
                if (*hk == dock || scrolls) && self.screens.get(*hs).is_some_and(|sc| sc.items.contains_key(hk)) {
                    self.render(*hs, hk);
                }
            }
        }
        if let Some((hs, hk)) = self.hover.clone() {
            if let Some(it) = self.screens.get_mut(hs).and_then(|sc| sc.items.get_mut(&hk)) {
                if it.content_h > it.visible + 1.0 && it.open.now > 0.6 {
                    it.scrolled_at = Some(Instant::now());
                }
            }
            // (on an open panel: which of its things the mouse is on; another, or none: drawn again, its ghost buttons lit so)
            let under = self.screens.get(hs).and_then(|sc| sc.items.get(&hk)).filter(|it| it.open.now > 0.6).and_then(|it| {
                let (_, b) = self.shape_now(hs, it).ok()?;
                let (lx, ly) = (x - b.x, y - b.y);
                Some(it.hits.iter().position(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).map_or(usize::MAX, |i| i))
            });
            let now = under.map(|i| (hs, hk.clone(), i));
            if now != self.hover_hit {
                self.hover_hit = now;
                self.render(hs, &hk);
                self.commit();
            }
        }
        unsafe {
            let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.screens[si].ov.hwnd, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut t);
        }
        self.tick_rate();
    }

    /// The mouse moved with a slider held: its knob follows (its plugin told, at most every `SLIDE_EVERY`). True when one is held.
    fn slide_move(&mut self, si: usize, x: f32) -> bool {
        let Some(sl) = self.slide.as_ref() else { return false };
        if sl.si != si {
            return true;
        }
        let Some(it) = self.screens[si].items.get(&sl.key) else { return true };
        let Ok((_, b)) = self.shape_now(si, it) else { return true };
        let frac = slide_frac(sl.rect, x - b.x);
        if (frac - sl.frac).abs() < 1e-4 {
            return true;
        }
        let sl = self.slide.as_mut().unwrap();
        sl.frac = frac;
        let (key, action, plugin) = (sl.key.clone(), sl.action.clone(), sl.plugin.clone());
        if sl.live && sl.told.elapsed() >= SLIDE_EVERY {
            sl.told = Instant::now();
            sl.told_frac = frac;
            self.act(plugin, action.clone(), json!({ "fraction": frac }));
        }
        blocks::set_sliding(Some((action, frac)));
        self.render(si, &key);
        self.commit();
        true
    }

    /// Is a full-screen (or, as the setting says, maximized) app in front on this screen, the widgets out of its way?
    fn out_of_way(&self, si: usize) -> bool {
        self.screens.get(si).is_some_and(|sc| (self.config.auto_hide != "never" && sc.full) || (self.config.auto_hide == "maximized" && sc.maxed))
    }

    /// A box to type in, right over a field of a panel (`r`, in the panel at `b`); its widget stays open while it is.
    #[allow(clippy::too_many_arguments)]
    fn open_field(&mut self, si: usize, k: &str, b: Bx, r: Rect, plugin: String, a: String, live: bool, hint: String, value: String, secret: bool, look: Value) {
        let sc = &self.screens[si];
        let (s, m) = (sc.s, sc.ov.monitor.rect);
        // (words of several lines: room for more as they are written)
        let r = if look["area"] == true { Rect { h: r.h.max(140.0), ..r } } else { r };
        let at = RECT { left: m.left + ((b.x + r.x) * s).round() as i32, top: m.top + ((b.y + r.y) * s).round() as i32, right: m.left + ((b.x + r.x + r.w) * s).round() as i32, bottom: m.top + ((b.y + r.y + r.h) * s).round() as i32 };
        if self.input.is_none() {
            self.input = crate::inputbox::InputBox::new(&self.g).ok();
        }
        if let Some(it) = self.screens[si].items.get_mut(k) {
            it.pinned = true;
        }
        let g = &self.g;
        if let Some(ib) = &mut self.input {
            ib.open(g, at, s, k.to_string(), plugin, a, live, hint, value);
            ib.secret = secret;
            ib.set_look(&look);
            ib.render(g, &self.pc);
        }
        self.commit();
    }

    /// A field asked to be typed in at once (a network's password): opened as soon as its panel shows it.
    fn focus_pending(&mut self) {
        let Some((action, at)) = self.focus_field.clone() else { return };
        if at.elapsed() > Duration::from_millis(1500) {
            self.focus_field = None;
            return;
        }
        for si in 0..self.screens.len() {
            let found = self.screens[si].items.iter().find_map(|(k, it)| {
                if it.open.now < 0.9 {
                    return None;
                }
                it.hits.iter().find_map(|h| match &h.press {
                    Press::Field(a, live, hint, value, secret, look) if *a == action => Some((k.clone(), h.rect, *live, hint.clone(), value.clone(), *secret, look.clone())),
                    _ => None,
                })
            });
            if let Some((k, r, live, hint, value, secret, look)) = found {
                let Some(it) = self.screens[si].items.get(&k) else { return };
                let Ok((_, b)) = self.shape_now(si, it) else { return };
                let plugin = self.widgets.get(unpop(&k)).map(|w| w.plugin.clone()).unwrap_or_default();
                self.focus_field = None;
                self.open_field(si, &k, b, r, plugin, action, live, hint, value, secret, look);
                return;
            }
        }
    }

    /// Does a widget bump? As its menu says; else yes (the tray: no).
    fn bumps_on(&self, k: &str) -> bool {
        self.config.widgets.get(unpop(k)).and_then(|c| c.bump).unwrap_or(unpop(k) != key(crate::builtin::PLUGIN, crate::builtin::TRAY))
    }

    /// Something worth a glance happened to a widget: tucked in its edge, it comes out for a moment, glowing fresh, and goes back;
    /// already out, its glow plays again. Not more than once in half a minute; not while its plugin's banner says it; not over a
    /// full-screen app unless it may show there.
    pub(super) fn bump(&mut self, k: &str) {
        const HOLD: Duration = Duration::from_millis(1600);
        const EVERY: Duration = Duration::from_secs(30);
        if !self.bumps_on(k) || self.edit.is_some() {
            return;
        }
        if self.bumped_at.get(k).is_some_and(|t| t.elapsed() < EVERY) {
            return;
        }
        let Some(w) = self.widgets.get(unpop(k)) else { return };
        if self.banner.as_ref().is_some_and(|b| b.plugin == w.plugin) {
            return;
        }
        let Some(si) = self.screens.iter().position(|sc| sc.items.contains_key(k)) else { return };
        let full = self.screens[si].full;
        if full && !self.over_full(k) {
            return;
        }
        self.bumped_at.insert(k.to_string(), Instant::now());
        self.shown_at.insert(k.to_string(), Instant::now());
        let out = self.screens[si].items.get(k).is_some_and(|it| it.shown.to >= 1.0 && it.shown.now > 0.9);
        if !out {
            self.bumps.insert(k.to_string(), Instant::now() + HOLD);
        }
        self.render(si, k);
        self.tick_rate();
    }

    /// A desktop's card held: carried after a moment (the map shows its places); carried near the map's edge, the map moves that
    /// way, faster the nearer.
    fn card_tick(&mut self) {
        let Some((si, k, cid, _, x0, y0, at, carried)) = self.card_press.clone() else { return };
        if !carried {
            if at.elapsed() >= Duration::from_millis(350) && !cid.is_empty() {
                if let Some(cp) = self.card_press.as_mut() {
                    cp.7 = true;
                }
                if let Some(it) = self.screens.get(si).and_then(|sc| sc.items.get(&k)) {
                    if let Ok((_, b)) = self.shape_now(si, it) {
                        let g = crate::blocks::desk_geom();
                        let (gx, gy) = g.map_or((70.0, 40.0), |g| (g.cell.0 * g.view.0 * 0.45, g.cell.1 * g.view.0 * 0.3));
                        crate::blocks::set_desk_carry(Some((cid, x0 - b.x, y0 - b.y, gx, gy)));
                    }
                }
                self.render(si, &k);
                self.commit();
                self.tick_rate();
            }
            return;
        }
        let Some(g) = crate::blocks::desk_geom() else { return };
        let Some(it) = self.screens.get(si).and_then(|sc| sc.items.get(&k)) else { return };
        let Ok((_, b)) = self.shape_now(si, it) else { return };
        let (lx, ly) = (self.mouse.1 - b.x, self.mouse.2 - b.y);
        let r = g.rect;
        let push = |d: f32| if d < 40.0 { (40.0 - d.max(0.0)) / 40.0 } else { 0.0 };
        let dx = push(r.right() - lx) - push(lx - r.x);
        let dy = push(r.bottom() - ly) - push(ly - r.y);
        if dx != 0.0 || dy != 0.0 {
            crate::blocks::desk_scroll(dx * 9.0, dy * 9.0);
            self.render(si, &k);
            self.commit();
        }
    }

    /// The desktops' map, each desktop at its place: kept in the config by its id; a new one goes where it was asked to go, else
    /// the first free place to the right of the one shown.
    fn desk_places(&mut self) -> BTreeMap<String, (i32, i32)> {
        let mut places = self.config.desk_map.clone();
        let ids: Vec<String> = self.bi.sys.maps.iter().map(|m| m.id.clone()).filter(|i| !i.is_empty()).collect();
        let cur = ids.get(self.bi.sys.desk).and_then(|i| places.get(i).copied());
        let mut changed = false;
        for (i, id) in ids.iter().enumerate() {
            if places.contains_key(id) {
                continue;
            }
            let taken: Vec<(i32, i32)> = places.iter().filter(|(k, _)| ids.contains(k)).map(|(_, v)| *v).collect();
            let spot = match self.bi.desk_pending.take().filter(|p| !taken.contains(p)) {
                Some(p) => p,
                None => {
                    let (c, r) = cur.unwrap_or((i as i32 - 1, 0));
                    let mut c = c + 1;
                    while taken.contains(&(c, r)) {
                        c += 1;
                    }
                    (c, r)
                }
            };
            places.insert(id.clone(), spot);
            changed = true;
        }
        if changed {
            let p = places.clone();
            self.change_config(|c| c.desk_map = p);
        }
        places
    }

    /// Win + Ctrl + an arrow (`dir`: 0 left, 1 up, 2 right, 3 down): the nearest desktop that way on the map is shown (Windows' own
    /// steps, quick); the map shows in the middle of the screen as it goes. Nothing that way: the map nudges, and stays.
    pub fn desk_key(&mut self, dir: usize) {
        let places = self.desk_places();
        let ids: Vec<String> = self.bi.sys.maps.iter().map(|m| m.id.clone()).collect();
        let at = self.bi.sys.desk;
        let Some(cur) = ids.get(at).and_then(|i| places.get(i).copied()) else { return };
        let (dx, dy) = [(-1, 0), (0, -1), (1, 0), (0, 1)][dir.min(3)];
        let mut best: Option<(i32, usize, (i32, i32))> = None;
        for (i, id) in ids.iter().enumerate() {
            let Some((c, r)) = places.get(id).copied() else { continue };
            let (ac, ar) = (c - cur.0, r - cur.1);
            let ahead = ac * dx + ar * dy;
            if ahead <= 0 {
                continue;
            }
            let side = (ac * dy).abs() + (ar * dx).abs();
            let score = ahead + 2 * side;
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, i, (c, r)));
            }
        }
        if let Some((_, i, _)) = best {
            crate::sysw::switch_desk(at, i);
        }
        // (the Desktops widget comes out where it stands, as if the mouse had come to it, showing where it went)
        self.bumped_at.remove(&key(crate::builtin::PLUGIN, crate::sysw::DESKS));
        self.bump(&key(crate::builtin::PLUGIN, crate::sysw::DESKS));
        self.tick_rate();
    }

    /// A volume key pressed: the Volume Mixer shows the new level (bumps).
    pub fn volume_key(&mut self) {
        self.sync_builtins(true);
        self.bump(&key(crate::builtin::PLUGIN, crate::builtin::QUICK));
    }

    /// A slider let go a moment ago: its knob free again to show what its plugin says.
    fn slid_settle(&mut self) {
        let Some((si, k, at)) = self.slid.clone() else { return };
        if self.slide.is_none() && at.elapsed() > Duration::from_millis(700) {
            self.slid = None;
            blocks::set_sliding(None);
            if self.screens.get(si).is_some_and(|sc| sc.items.contains_key(&k)) {
                self.render(si, &k);
                self.commit();
            }
        }
    }

    pub fn mouse_leave(&mut self, si: usize) {
        if self.sizing.is_none() {
            if let Some((hs, hk, _)) = self.size_hover.take() {
                if self.screens.get(hs).is_some_and(|sc| sc.items.contains_key(&hk)) {
                    self.render(hs, &hk);
                }
            }
        }
        if self.hover.as_ref().is_some_and(|(s, _)| *s == si) {
            let old = self.hover.take();
            // (the dock's grip goes with the mouse)
            // (and a panel's scrollbar)
            if let Some((hs, hk)) = old.filter(|(s, h)| *h == key(crate::builtin::PLUGIN, crate::builtin::DOCK) || self.screens.get(*s).and_then(|sc| sc.items.get(h)).is_some_and(|i| i.content_h > i.visible + 1.0)) {
                if self.screens.get(hs).is_some_and(|sc| sc.items.contains_key(&hk)) {
                    self.render(hs, &hk);
                }
            }
        }
        for it in self.screens[si].items.values_mut() {
            it.pinned = false;
        }
        self.tick_rate();
    }

    pub fn click(&mut self, si: usize, x: f32, y: f32) {
        if self.size_release() {
            return;
        }
        // (a desktop's card let go: carried, it goes where it was dropped; else its action)
        if let Some((cs, ck, cid, a, _, _, _, carried)) = self.card_press.take() {
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            }
            let plugin = crate::builtin::PLUGIN.to_string();
            if carried {
                let cell = self.screens[cs].items.get(&ck).and_then(|it| self.shape_now(cs, it).ok()).and_then(|(_, b)| crate::blocks::desk_cell_at(x - b.x, y - b.y));
                crate::blocks::set_desk_carry(None);
                if let Some((c, r)) = cell {
                    self.act(plugin, format!("deskmove:{cid}:{c}:{r}"), json!({}));
                }
                if self.screens[cs].items.contains_key(&ck) {
                    self.render(cs, &ck);
                }
                self.commit();
            } else {
                self.act(plugin, a, json!({}));
            }
            let _ = si;
            self.press = None;
            return;
        }
        // (a picture let go: unmoved twice in a moment, it is fitted again)
        if let Some((ps, k, id, _, _, moved)) = self.pan.take() {
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            }
            if !moved {
                if self.view_click.as_ref().is_some_and(|(v, t)| *v == id && t.elapsed() < Duration::from_millis(450)) {
                    self.view_click = None;
                    crate::blocks::view_reset(&id);
                    self.render(ps, &k);
                    self.commit();
                } else {
                    self.view_click = Some((id, Instant::now()));
                }
            }
            self.press = None;
            return;
        }
        // (a slider let go: its plugin told where, its knob kept there until its plugin says)
        if self.slide.is_some() {
            self.slide_move(si, x);
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            }
            let sl = self.slide.take().unwrap();
            if !sl.live || (sl.told_frac - sl.frac).abs() > 1e-4 {
                self.act(sl.plugin.clone(), sl.action.clone(), json!({ "fraction": sl.frac }));
            }
            self.slid = Some((sl.si, sl.key.clone(), Instant::now()));
            self.render(sl.si, &sl.key);
            self.commit();
            self.press = None;
            return;
        }
        // (a thing that is asked about first, let go of on it: asked, in a modal, out of where it was pressed)
        if let Some(h) = self.hold.take() {
            match &h.key {
                Some(k) => self.render(h.si, k),
                None => self.close_menu(),
            }
            self.press = None;
            let m = self.screens[h.si].ov.monitor.rect;
            let s = self.screens[h.si].s;
            let (mx, my) = (self.mouse.1, self.mouse.2);
            self.ask(h.spec, h.plugin, Some((m.left + (mx * s) as i32, m.top + (my * s) as i32)));
            self.commit();
            return;
        }
        // (a window held by its grip, or pulled off its edge: put down)
        if self.pop_release() {
            return;
        }
        // (a menu open, while editing too: the click is its (on it, a row; off it, put away))
        if self.menu_open() && self.drag.is_none() && !self.edit.as_ref().is_some_and(|e| e.carry.is_some()) {
            self.menu_click(si, x, y);
            return;
        }
        if self.edit_click(si, x, y) {
            return;
        }
        if self.drag.is_some() {
            self.drop(si, x, y);
            return;
        }
        if self.menu_open() {
            self.menu_click(si, x, y);
            return;
        }
        // (a press that was let go after it moved is no click)
        // (a window is never picked up: its presses are not kept as one, and are clicks)
        if self.press.take().is_none() && self.item_at(si, x, y).is_some_and(|k| !is_pop(&k)) && !self.in_banner(si, x, y) {
            return;
        }
        if self.in_banner(si, x, y) {
            self.click_banner(si, x, y);
            return;
        }
        let Some(k) = self.item_at(si, x, y) else { return };
        let Some(w) = self.widgets.get(unpop(&k)) else { return };
        let plugin = w.plugin.clone();
        if w.content["peek"]["face"] == "eyes" {
            self.eyes.poke(self.start.elapsed().as_secs_f32() * 1000.0);
            return;
        }
        let it = self.screens[si].items.get(&k).unwrap();
        let (_, b) = match self.shape_now(si, it) {
            Ok(v) => v,
            Err(_) => return,
        };
        let (lx, ly) = (x - b.x, y - b.y);
        let panel_hit = it.hits.iter().any(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h);
        let row_hit = it.buttons.iter().any(|(r, a)| !a.is_empty() && lx >= r.x - 4.0 && lx < r.x + r.w + 4.0 && ly >= r.y - 6.0 && ly < r.y + r.h + 6.0);
        if it.open.now > 0.6 && (panel_hit || !row_hit) {
            let hit = it.hits.iter().find(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).cloned();
            match hit.map(|h| (h.press, h.rect)) {
                Some((Press::Action(a, _), _)) if a == BACK_TO_NOW => {
                    self.wheeled.remove(&k);
                    self.seen_h.remove(&k);
                    self.render(si, &k);
                    self.commit();
                }
                Some((Press::Action(a, _), _)) if a.is_empty() => {}
                Some((Press::Action(a, args), _)) => {
                    self.act(plugin, a, if args.is_null() { json!({}) } else { args });
                }
                Some((Press::Seek(a), r)) => {
                    let fr = ((lx - r.x) / r.w).clamp(0.0, 1.0);
                    self.act(plugin, a, json!({ "fraction": fr }));
                }
                Some((Press::Field(a, live, hint, value, secret, look), r)) => {
                    self.open_field(si, &k, b, r, plugin, a, live, hint, value, secret, look);
                }
                // (a picture: its own presses are its drags, from the press down)
                Some((Press::View(_), _)) => {}
                Some((Press::Card(a, _), _)) => {
                    self.act(plugin, a, json!({}));
                }
                Some((Press::Toggle(id), _)) => {
                    let it = self.screens[si].items.get_mut(&k).unwrap();
                    let open_now = it.sections_open.contains(&id);
                    // (the first is open until shut)
                    let first_open = !it.sections_closed.contains(&id) && !open_now;
                    if open_now {
                        it.sections_open.remove(&id);
                        it.sections_closed.insert(id);
                    } else if first_open && is_first_section(self.widgets.get(unpop(&k)), &id) {
                        it.sections_closed.insert(id);
                    } else {
                        it.sections_closed.remove(&id);
                        it.sections_open.insert(id);
                    }
                    // (the panel takes the room its sections need)
                    self.relayout_with(Some(&HashSet::from([k.clone()])));
                }
                None => {}
            }
            return;
        }
        // at rest: its button, else it opens (and stays open: pressed, not held); on the dock's grip, it opens
        let on_grip = self.on_grip(si, &k, x, y);
        for (r, a) in it.buttons.clone().into_iter().filter(|_| !on_grip) {
            if lx >= r.x - 4.0 && lx < r.x + r.w + 4.0 && ly >= r.y - 6.0 && ly < r.y + r.h + 6.0 && !a.is_empty() {
                self.act(plugin, a, json!({}));
                return;
            }
        }
        // (its window out: to the front, or back)
        if self.pop_widget_pressed(&k) {
            return;
        }
        // (a widget that is a button: pressed, it does its one thing)
        if let Some(a) = self.widgets.get(unpop(&k)).and_then(|w| w.content["press"].as_str().map(str::to_string)) {
            self.act(plugin, a, json!({}));
            return;
        }
        let has_panel = self.widgets.get(unpop(&k)).is_some_and(|w| w.content["panel"].is_object());
        if has_panel {
            self.open_only(si, &k);
            self.eyes.greet(self.start.elapsed().as_secs_f32() * 1000.0);
            self.relayout_with(Some(&HashSet::new()));
        }
    }

    fn click_banner(&mut self, si: usize, x: f32, y: f32) {
        let Some(b) = &self.banner else { return };
        let _ = si;
        let at = b.at.unwrap_or_default();
        let (lx, ly) = (x - at.x, y - at.y);
        let plugin = b.plugin.clone();
        let hit = b.hits.iter().find(|(r, _)| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h).map(|(_, p)| p.clone());
        let say = b.say.clone();
        if let Some(Press::Action(a, _)) = hit {
            if !a.is_empty() {
                self.act(plugin, a, json!({}));
            }
        } else {
            let press = s_of(&say["pill"], "press");
            let open = s_of(&say, "open");
            if !press.is_empty() {
                let _ = self.tx.send(Command::Action { plugin, action: press.to_string(), args: json!({}) });
            } else if !open.is_empty() {
                let _ = self.tx.send(Command::OpenUrl { plugin, url: open.to_string() });
            } else {
                // (nothing of its own to do: the widget it came out of opens, in its place)
                let (hk, ..) = self.banner_home(&plugin, &say);
                if !hk.is_empty() && self.widgets.get(&hk).is_some_and(|w| w.content["panel"].is_object()) {
                    self.banner = None;
                    for sc in &self.screens {
                        sc.hide_banner();
                    }
                    self.relayout_with(Some(&HashSet::new()));
                    if let Some(wsi) = (0..self.screens.len()).find(|&i| self.screens[i].items.contains_key(&hk)) {
                        self.open_only(wsi, &hk);
                        self.relayout_with(Some(&HashSet::new()));
                    }
                    self.commit();
                    self.tick_rate();
                    return;
                }
            }
        }
        if let Some(b) = &mut self.banner {
            b.leaving = true;
            b.slide.to = 0.0;
        }
        self.tick_rate();
    }

    /// The right button on a widget: its menu (merged or floating, where it goes, hidden), and the frame's own (monitors, taskbar).
    pub fn context_menu(&mut self, si: usize, x: f32, y: f32) {
        let k = self.item_at(si, x, y);
        // (on a tray icon: its app's own menu)
        if let Some(it) = k.as_ref().and_then(|k| self.screens[si].items.get(k)) {
            if let Ok((_, b)) = self.shape_now(si, it) {
                let (lx, ly) = (x - b.x, y - b.y);
                let inside = |r: &Rect| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h;
                let action = it.buttons.iter().find(|(r, a)| a.starts_with("tray:") && inside(r)).map(|(_, a)| a.clone()).or_else(|| {
                    it.hits.iter().find(|h| inside(&h.rect) && matches!(&h.press, Press::Action(a, _) if a.starts_with("tray:"))).and_then(|h| match &h.press {
                        Press::Action(a, _) => Some(a.clone()),
                        _ => None,
                    })
                });
                if let Some((owner, uid)) = action.as_deref().and_then(|a| a.strip_prefix("tray:")).and_then(|r| r.split_once(':')) {
                    let mut pt = POINT::default();
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    crate::trayhost::click(owner.parse().unwrap_or(0), uid.parse().unwrap_or(0), crate::trayhost::Click::Right, pt.x, pt.y);
                    return;
                }
            }
        }
        // (a second right-click puts the menu away)
        if self.menu_open() {
            self.close_menu();
            return;
        }
        // (on an app on the dock at rest: its own menu)
        if let Some(it) = k.as_ref().and_then(|k| self.screens[si].items.get(k)).filter(|it| it.open.now < 0.35) {
            if let Ok((_, b)) = self.shape_now(si, it) {
                let (lx, ly) = (x - b.x, y - b.y);
                let app = it.buttons.iter().find(|(r, a)| a.starts_with("app:") && lx >= r.x - 4.0 && lx < r.x + r.w + 4.0 && ly >= r.y - 6.0 && ly < r.y + r.h + 6.0).and_then(|(_, a)| a[4..].parse::<usize>().ok());
                let items = app.and_then(|i| k.as_ref().and_then(|k| self.widgets.get(unpop(k))).and_then(|w| w.content["peek"]["dock"]["items"][i]["menu"].as_array().cloned()));
                if let Some(items) = items {
                    self.open_items_menu(si, crate::builtin::PLUGIN.into(), items, x, y);
                    return;
                }
            }
        }
        // (on a thing in an open panel that has a menu of its own: that menu)
        if let Some(it) = k.as_ref().and_then(|k| self.screens[si].items.get(k)).filter(|it| it.open.now > 0.6) {
            if let Ok((_, b)) = self.shape_now(si, it) {
                let (lx, ly) = (x - b.x, y - b.y);
                let own = it.hits.iter().find(|h| h.menu.is_array() && lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).map(|h| h.menu.as_array().cloned().unwrap_or_default());
                if let (Some(items), Some(w)) = (own, k.as_ref().and_then(|k| self.widgets.get(unpop(k)))) {
                    let plugin = w.plugin.clone();
                    self.open_items_menu(si, plugin, items, x, y);
                    return;
                }
            }
        }
        self.open_menu(si, k, x, y);
    }

    // ------------------------------------------------------------------------------------------------------------- the menu

    /// Open a menu: a widget's (beside it), or the short one by a point (`k`: none).
    pub fn open_menu(&mut self, si: usize, k: Option<String>, x: f32, y: f32) {
        self.drop_menu();
        // (its widget's panel shuts while its menu is open)
        if let Some(k) = &k {
            let keep = self.always_open(k);
            let shut = self.screens[si].items.get_mut(k).is_some_and(|it| {
                let was = it.open.to == 1.0 && !keep;
                if was {
                    it.open.to = 0.0;
                    it.pinned = false;
                }
                was
            });
            if shut {
                self.set_open(si, k, false);
                self.relayout_with(Some(&HashSet::new()));
            }
        }
        let (w, h) = match &k {
            Some(_) => crate::menu::widget_size(),
            None => crate::menu::mini_size(),
        };
        let s = self.screens[si].s;
        let Ok(piece) = self.screens[si].ov.piece(&self.g, (w * s).ceil() as u32 + 4, (h * s).ceil() as u32 + 4) else { return };
        self.menu = Some(Menu { si, key: k, items: None, piece, at: Bx { x, y, w, h }, point: (x, y), hits: Vec::new(), names: Vec::new(), hover: None, marquee: false, grow: Spring { now: 0.0, to: 1.0, vel: 0.0 }, closing: false, pivot: (w / 2.0, 0.0) });
        self.render_menu();
        self.hit_regions();
        self.commit();
        self.tick_rate();
    }

    /// A thing's own menu (a right-click on a window in the dock), where the mouse is.
    fn open_items_menu(&mut self, si: usize, plugin: String, items: Vec<Value>, x: f32, y: f32) {
        self.drop_menu();
        let (w, h) = crate::menu::items_size(items.len());
        let s = self.screens[si].s;
        let Ok(piece) = self.screens[si].ov.piece(&self.g, (w * s).ceil() as u32 + 4, (h * s).ceil() as u32 + 4) else { return };
        self.menu = Some(Menu { si, key: None, items: Some((plugin, items)), piece, at: Bx { x, y, w, h }, point: (x, y), hits: Vec::new(), names: Vec::new(), hover: None, marquee: false, grow: Spring { now: 0.0, to: 1.0, vel: 0.0 }, closing: false, pivot: (w / 2.0, h) });
        self.render_menu();
        self.hit_regions();
        self.commit();
        self.tick_rate();
    }

    /// The short menu, where the mouse is (NADI's tray icon).
    pub fn open_menu_at_cursor(&mut self) {
        let Some(si) = self.mouse_screen() else { return };
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let sc = &self.screens[si];
        let m = sc.ov.monitor.rect;
        let (x, y) = ((pt.x - m.left) as f32 / sc.s, (pt.y - m.top) as f32 / sc.s);
        self.open_menu(si, None, x, y);
    }

    /// The menu goes: it shrinks back into its widget's side, then it is gone.
    fn close_menu(&mut self) {
        if let Some(m) = &mut self.menu {
            m.closing = true;
            m.grow.to = 0.0;
            m.hover = None;
            self.tick_rate();
        }
    }

    /// The menu gone at once.
    fn drop_menu(&mut self) {
        if let Some(m) = self.menu.take() {
            if let Some(sc) = self.screens.get(m.si) {
                let _ = sc.ov.remove(&m.piece);
            }
            self.refresh_thumbs();
            self.hit_regions();
            self.commit();
            self.tick_rate();
        }
    }

    /// A menu that is open (not going).
    fn menu_open(&self) -> bool {
        self.menu.as_ref().is_some_and(|m| !m.closing)
    }

    /// One frame of the menu's spring; true while it moves.
    fn step_menu(&mut self, dt: f32) -> bool {
        let Some(m) = &mut self.menu else { return false };
        let moved = m.grow.step(dt);
        if m.closing && m.grow.now <= 0.02 && !m.grow.moving() {
            self.drop_menu();
            return true;
        }
        if moved {
            self.render_menu();
        }
        moved
    }

    /// Where the menu goes: beside its widget, away from the widget's edge (or above the point it was asked at), kept on the screen.
    fn menu_box(&self, m: &Menu) -> Bx {
        let sc = &self.screens[m.si];
        let a = sc.area;
        let (w, h) = (m.at.w, m.at.h);
        let near = m.key.as_ref().and_then(|k| sc.items.get(k)).and_then(|it| self.shape_now(m.si, it).ok().map(|(_, b)| (it.slot.place, b)));
        let (x, y) = match near {
            Some((place, b)) => match place {
                Place::Edge(Edge::Left) => (b.x + b.w + 8.0, b.y + b.h / 2.0 - h / 2.0),
                Place::Edge(Edge::Right) => (b.x - w - 8.0, b.y + b.h / 2.0 - h / 2.0),
                Place::Edge(Edge::Bottom) | Place::Corner(Corner::BottomLeft | Corner::BottomRight) => (b.x + b.w / 2.0 - w / 2.0, b.y - h - 8.0),
                _ => (b.x + b.w / 2.0 - w / 2.0, b.y + b.h + 8.0),
            },
            None => (m.point.0 - w / 2.0, m.point.1 - h - 8.0),
        };
        Bx { x: x.clamp(a.x + 8.0, (a.x + a.w - w - 8.0).max(a.x + 8.0)), y: y.clamp(a.y + 8.0, (a.y + a.h - h - 8.0).max(a.y + 8.0)), w, h }
    }

    fn render_menu(&mut self) {
        let Some(m) = self.menu.as_ref() else { return };
        let b = self.menu_box(m);
        // (it grows from its side toward its widget: the side nearest the widget, else its foot)
        let pivot = match m.key.as_ref().and_then(|k| self.screens[m.si].items.get(k)).map(|it| it.slot.place) {
            Some(Place::Edge(Edge::Left)) => (0.0, b.h / 2.0),
            Some(Place::Edge(Edge::Right)) => (b.w, b.h / 2.0),
            Some(Place::Edge(Edge::Bottom) | Place::Corner(Corner::BottomLeft | Corner::BottomRight)) | None => (b.w / 2.0, b.h),
            _ => (b.w / 2.0, 0.0),
        };
        let g_now = m.grow.now;
        let si = m.si;
        let s = self.screens[si].s;
        let state = m.key.as_ref().map(|k| unpop(k)).map(|k| crate::menu::State {
            can_pop: self.widgets.get(k).is_some_and(|w| w.content["panel"].is_object()),
            popped: self.popped(k),
            floating: self.floating(k),
            place: self.widgets.get(unpop(k)).and_then(|w| self.place_of(&w.plugin, &w.id)).map(|(p, _)| p.name().to_string()).unwrap_or_default(),
            pinned: self.config.widgets.get(k).and_then(|c| c.monitor.clone()) == Some(self.screens[si].ov.monitor.name()),
            large: self.large(k),
            always_shown: self.config.widgets.get(k).and_then(|c| c.always_shown) == Some(true),
            over_full: self.over_full(k),
            always_open: self.always_open(k),
            hide_idle: self.config.widgets.get(k).and_then(|c| c.show_idle) != Some(true),
            bump: self.bumps_on(k),
            icon_only: if self.icon_always(k) { None } else { Some(self.icon_alone(k)) },
        });
        let hover = m.hover;
        let items = m.items.as_ref().map(|(_, i)| i.clone());
        let held = self.hold.as_ref().filter(|h| h.key.is_none()).and_then(|h| h.row.map(|r| (r, 0.0)));
        let Ok(shape) = shape::floating(&self.g.d2d_factory, b.w * s, b.h * s, crate::menu::RADIUS * s) else { return };
        let surface = m.piece.surface.clone();
        let p = self.painter(s);
        let mut hits = Vec::new();
        let mut names: Vec<&'static str> = Vec::new();
        let g = &self.g;
        let _ = crate::draw::take_marquee();
        let _ = self.g.draw_surface(&surface, |dc| unsafe {
            // (scaled about its pivot as it springs out, a little past whole, and faded in)
            let k = (0.55 + 0.45 * g_now).max(0.05);
            let mut base = windows::Foundation::Numerics::Matrix3x2::default();
            dc.GetTransform(&mut base);
            let (px, py) = (pivot.0 * s, pivot.1 * s);
            let scale = windows::Foundation::Numerics::Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: px * (1.0 - k), M32: py * (1.0 - k) };
            dc.SetTransform(&(scale * base));
            with_opacity(dc, g, g_now.clamp(0.0, 1.0), || {
                let _ = fill_shape(dc, &shape, FACE);
                hits = match (&items, &state) {
                    (Some(items), _) => crate::menu::draw_items(&p, items, hover, held),
                    (None, Some(st)) => crate::menu::draw_widget(&p, st, hover, &mut names),
                    (None, None) => crate::menu::draw_mini(&p, hover),
                };
            });
        });
        let m = self.menu.as_mut().unwrap();
        m.pivot = pivot;
        // (over every widget, those made after it too)
        self.screens[m.si].ov.to_front(&m.piece.visual);
        let _ = m.piece.place((b.x * s).round() as i32, (b.y * s).round() as i32);
        m.at = b;
        m.hits = hits;
        m.names = names;
        m.marquee = crate::draw::take_marquee();
        self.refresh_thumbs();
    }

    /// A held thing: drawn filling; full, it happens; the mouse gone off it, let go.
    fn step_hold(&mut self) {
        let Some(h) = &self.hold else { return };
        let (msi, mx, my) = self.mouse;
        // (the mouse left it: let go)
        let still = msi == h.si
            && match &h.key {
                Some(k) => self.screens.get(h.si).and_then(|sc| sc.items.get(k)).and_then(|it| self.shape_now(h.si, it).ok()).is_some_and(|(_, b)| {
                    let (lx, ly) = (mx - b.x, my - b.y);
                    lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h
                }),
                None => self.menu_hit(h.si, mx, my) == h.row,
            };
        // (nothing fills now: a press asks, in a modal)
        let done = false;
        let (key, si) = (h.key.clone(), h.si);
        if !still || done {
            let h = self.hold.take().unwrap();
            if done && still {
                if h.key.is_none() {
                    self.close_menu();
                }
                self.press = None;
                self.act(h.plugin, h.action, json!({}));
            }
        }
        match key {
            Some(k) => {
                if self.screens.get(si).is_some_and(|sc| sc.items.contains_key(&k)) {
                    self.render(si, &k);
                }
            }
            None => self.render_menu(),
        }
    }

    /// Is the mouse on the dock's grip (where a click opens widgets: its two ends, past its first and last icon)?
    fn on_grip(&self, si: usize, k: &str, x: f32, y: f32) -> bool {
        self.grip_end(si, k, x, y).is_some()
    }

    /// Which of the dock's grips the mouse is on: its first end (0) or its last (1).
    fn grip_end(&self, si: usize, k: &str, x: f32, y: f32) -> Option<u8> {
        if k != key(crate::builtin::PLUGIN, crate::builtin::DOCK) || !self.click_to_open() {
            return None;
        }
        let it = self.screens.get(si).and_then(|sc| sc.items.get(k))?;
        if it.open.now > 0.05 {
            return None;
        }
        let (_, b) = self.shape_now(si, it).ok()?;
        if x < b.x || x >= b.x + b.w || y < b.y || y >= b.y + b.h {
            return None;
        }
        // (its body, less its fillets into the edge)
        let f = if it.floating { 0.0 } else { Metrics::default().fillet };
        let end = crate::peek::DOCK_END + 2.0;
        let (v, lo, hi) = if it.slot.place.horizontal() { (x, b.x + f, b.x + b.w - f) } else { (y, b.y + f, b.y + b.h - f) };
        if v < lo + end {
            Some(0)
        } else if v >= hi - end {
            Some(1)
        } else {
            None
        }
    }

    /// The pointer over the dock's grips: a hand.
    pub fn grip_cursor(&self) -> Option<windows::core::PCWSTR> {
        self.grip_at.map(|_| windows::Win32::UI::WindowsAndMessaging::IDC_HAND)
    }

    /// What the mouse rests on that has a tip: (its screen, what it is, the tip, what it comes out of, the way out). A widget at rest
    /// has its own (where a click opens it: resting the mouse opens it otherwise); in an open panel, a thing that says what it is; on
    /// the dock and the tray at rest, each app and icon.
    fn tip_wanted(&self) -> Option<(usize, String, Value, Bx, Edge)> {
        // (in a widget's menu: the name of the picture under the mouse)
        if let Some(m) = self.menu.as_ref().filter(|m| !m.closing && m.grow.now > 0.9) {
            let i = m.hover?;
            let name = *m.names.get(i)?;
            let r = m.hits.get(i)?.0;
            let a = Bx { x: m.at.x + r.x, y: m.at.y + r.y, w: r.w, h: r.h };
            let out = if a.y > 70.0 { Edge::Top } else { Edge::Bottom };
            return Some((m.si, format!("\u{0}menu#{i}"), Value::String(name.to_string()), a, out));
        }
        if self.menu.is_some() || self.edit.is_some() || self.drag.is_some() || self.press.is_some() || self.launcher.as_ref().is_some_and(|l| l.shown) {
            return None;
        }
        let (hsi, k) = self.hover.clone()?;
        let (msi, mx, my) = self.mouse;
        if msi != hsi {
            return None;
        }
        // (a stump, its window out: nothing to say)
        if !is_pop(&k) && self.popped(&k) {
            return None;
        }
        let it = self.screens.get(hsi)?.items.get(&k)?;
        let (_, b) = self.shape_now(hsi, it).ok()?;
        let (lx, ly) = (mx - b.x, my - b.y);
        let inside = |r: &Rect| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h;
        let away = match it.slot.place {
            Place::Edge(Edge::Top) | Place::Corner(Corner::TopLeft | Corner::TopRight) => Edge::Bottom,
            Place::Edge(Edge::Left) => Edge::Right,
            Place::Edge(Edge::Right) => Edge::Left,
            _ => Edge::Top,
        };
        if it.open.now > 0.6 {
            let h = it.hits.iter().find(|h| inside(&h.rect) && !h.tip.is_null() && h.tip.as_str() != Some(""))?;
            let a = Bx { x: b.x + h.rect.x, y: b.y + h.rect.y, w: h.rect.w, h: h.rect.h };
            let out = if a.y > 70.0 { Edge::Top } else { Edge::Bottom };
            return Some((hsi, format!("{k}#{}:{}", h.rect.x as i32, h.rect.y as i32), h.tip.clone(), a, out));
        }
        if it.open.now > 0.05 || it.open.to > 0.0 {
            return None;
        }
        // (on the dock and the tray at rest: the app or the icon under the mouse)
        if let Some((r, a)) = it.buttons.iter().find(|(r, a)| !a.is_empty() && inside(r)) {
            // (an app on the dock with windows: a peek of them, their pictures and what the app uses)
            if let Some(app) = a.strip_prefix("app:").and_then(|i| i.parse::<usize>().ok()).and_then(|i| self.bi.dock_apps(&self.config).get(i).cloned()) {
                let tiles: Vec<Value> = self.widgets.get(unpop(&k)).map(|w| w.content["tiles"].as_array().cloned().unwrap_or_default()).unwrap_or_default().into_iter().filter(|t| t["key"].as_str() == Some(app.key.as_str())).collect();
                if !tiles.is_empty() {
                    // (its windows' pictures, big enough to read; four at most side by side)
                    let tiles: Vec<Value> = tiles.into_iter().take(4).collect();
                    let n = tiles.len() as f32;
                    let pic = if n <= 1.0 { 300.0 } else if n <= 2.0 { 250.0 } else if n <= 3.0 { 210.0 } else { 180.0 };
                    let win_w = pic + crate::peek::METERS + crate::peek::PROGRESS;
                    let width = 2.0 * 12.0 + n * win_w + (n - 1.0) * blocks::WIN_GAP;
                    let anchor = Bx { x: b.x + r.x, y: b.y + r.y, w: r.w, h: r.h };
                    return Some((hsi, format!("{k}@{a}"), json!({ "width": width, "body": [{ "kind": "windows", "items": tiles, "pic": pic }] }), anchor, away));
                }
            }
            let words = if let Some(i) = a.strip_prefix("app:").and_then(|i| i.parse::<usize>().ok()) {
                self.bi.dock_apps(&self.config).get(i).map(|x| x.name.clone())
            } else if a == "launcher" {
                Some("Apps".to_string())
            } else if let Some(rest) = a.strip_prefix("tray:") {
                let (owner, uid) = rest.split_once(':')?;
                crate::trayhost::icons().into_iter().find(|i| i.owner.to_string() == owner && i.uid.to_string() == uid).map(|i| i.tip.lines().next().unwrap_or("").to_string())
            } else {
                None
            };
            if let Some(w) = words.filter(|w| !w.is_empty()) {
                let anchor = Bx { x: b.x + r.x, y: b.y + r.y, w: r.w, h: r.h };
                return Some((hsi, format!("{k}@{a}"), Value::String(w), anchor, away));
            }
        }
        if !self.click_to_open() {
            return None;
        }
        let tip = &self.widgets.get(unpop(&k))?.content["tip"];
        if tip.is_null() || tip.as_str() == Some("") {
            return None;
        }
        Some((hsi, k.clone(), tip.clone(), b, away))
    }

    /// The tip: out after the mouse rests (quickly while another is out), springing; away as the mouse leaves.
    fn step_tip(&mut self, dt: f32) {
        let want = self.tip_wanted();
        let owner = want.as_ref().map(|w| w.1.clone());
        if self.tip_want.as_ref().map(|(o, _)| o) != owner.as_ref() {
            self.tip_want = owner.map(|o| (o, Instant::now()));
        }
        let warm = self.tip.as_ref().is_some_and(|t| t.grow.now > 0.3);
        let since = self.tip_want.as_ref().map(|(_, t)| t.elapsed());
        match want {
            Some((si, owner, content, anchor, out)) if since.is_some_and(|e| e >= if warm { TIP_NEXT } else { TIP_WAIT }) => {
                let same = self.tip.as_ref().is_some_and(|t| t.owner == owner && t.si == si);
                if same {
                    let t = self.tip.as_mut().unwrap();
                    t.content = content;
                    t.anchor = anchor;
                    t.out = out;
                    t.grow.to = 1.0;
                } else {
                    // (a new one: from small, on its screen's top glass)
                    let mut old = self.tip.take();
                    if let Some(t) = &mut old {
                        drop_tip_thumbs(t);
                    }
                    let piece = match old {
                        Some(t) if t.si == si => Some(t.piece),
                        Some(t) => {
                            self.screens[t.si].ov.remove_top(&t.piece);
                            None
                        }
                        None => None,
                    };
                    let piece = match piece {
                        Some(p) => p,
                        None => match self.screens[si].ov.top_piece(&self.g, 64, 64) {
                            Ok(p) => p,
                            Err(_) => return,
                        },
                    };
                    self.tip = Some(Tip { si, owner, content, anchor, out, piece, grow: Spring { now: 0.0, to: 1.0, vel: 0.0 }, drawn: None, thumbs: Vec::new(), moving: false });
                }
            }
            Some(_) => {}
            None => {
                if let Some(t) = &mut self.tip {
                    t.grow.to = 0.0;
                }
            }
        }
        let Some(t) = &mut self.tip else { return };
        let moved = t.grow.step(dt);
        // (going: its live pictures go first)
        if t.grow.to == 0.0 {
            drop_tip_thumbs(t);
        }
        if t.grow.to == 0.0 && t.grow.now <= 0.001 && !moved {
            let t = self.tip.take().unwrap();
            if let Some(sc) = self.screens.get(t.si) {
                sc.ov.remove_top(&t.piece);
            }
            return;
        }
        let changed = t.drawn.as_ref().map_or(true, |(c, a)| *c != t.content || *a != t.anchor);
        // (out, with windows in it and their pictures not yet put: drawn again, now with them)
        let wants_thumbs = t.grow.to == 1.0 && !moved && t.thumbs.is_empty() && t.content["body"].to_string().contains("\"hwnd\":");
        if moved || changed || wants_thumbs || t.moving {
            self.render_tip();
        }
    }

    fn render_tip(&mut self) {
        let Some(t) = &self.tip else { return };
        let si = t.si;
        let Some(sc) = self.screens.get(si) else { return };
        let s = sc.s;
        let (sw, sh) = (sc.w, sc.h);
        let empty = HashSet::new();
        const P: f32 = 12.0;
        // its size: words in a pill, or blocks like a panel's
        let p = self.painter(s);
        let (w, h) = if let Some(words) = t.content.as_str() {
            // (words: a line each, the first the strongest)
            let lines: Vec<&str> = words.lines().filter(|l| !l.is_empty()).collect();
            let lw = lines.iter().map(|l| p.measure(l, 12.0, Weight::Regular, 300.0)).fold(0.0f32, f32::max);
            (lw + 2.0 * P + 2.0, 30.0 + (lines.len().max(1) - 1) as f32 * 17.0)
        } else {
            let tw = t.content["width"].as_f64().map_or(TIP_W, |w| (w as f32).clamp(120.0, sw * 0.9));
            let inner = tw - 2.0 * P;
            let mut c = Ctx { p: &p, draw: false, hits: Vec::new(), open: &empty, closed: &empty, thumbs: Vec::new() };
            let head = blocks::head(&mut c, &t.content, 0.0, 0.0, inner);
            let body = c.body(t.content["body"].as_array().map(|v| v.as_slice()).unwrap_or(&[]), 0.0, 0.0, inner);
            (tw, head + if body > 0.0 { (if head > 0.0 { 10.0 } else { 0.0 }) + body } else { 0.0 } + 2.0 * P)
        };
        let a = t.anchor;
        let gap = 8.0;
        let (x, y) = match t.out {
            Edge::Bottom => (a.x + a.w / 2.0 - w / 2.0, a.y + a.h + gap),
            Edge::Top => (a.x + a.w / 2.0 - w / 2.0, a.y - gap - h),
            Edge::Right => (a.x + a.w + gap, a.y + a.h / 2.0 - h / 2.0),
            Edge::Left => (a.x - gap - w, a.y + a.h / 2.0 - h / 2.0),
        };
        let (x, y) = (x.clamp(6.0, (sw - w - 6.0).max(6.0)), y.clamp(6.0, (sh - h - 6.0).max(6.0)));
        // (it grows out of the point nearest what it comes out of)
        let (cx, cy) = (a.x + a.w / 2.0 - x, a.y + a.h / 2.0 - y);
        let pivot = match t.out {
            Edge::Bottom => (cx.clamp(0.0, w), 0.0),
            Edge::Top => (cx.clamp(0.0, w), h),
            Edge::Right => (0.0, cy.clamp(0.0, h)),
            Edge::Left => (w, cy.clamp(0.0, h)),
        };
        let g_now = t.grow.now;
        let settled = t.grow.to == 1.0 && !t.grow.moving();
        let content = t.content.clone();
        let Ok(shape) = shape::floating(&self.g.d2d_factory, w * s, h * s, (h / 2.0).min(16.0) * s) else { return };
        let need = ((w * s).ceil() as u32 + 4, (h * s).ceil() as u32 + 4);
        drop(p);
        let t = self.tip.as_mut().unwrap();
        let g = &self.g;
        if need.0 > t.piece.alloc.0 || need.1 > t.piece.alloc.1 {
            if let Ok(surface) = g.surface(need.0 + 32, need.1 + 32) {
                unsafe {
                    let _ = t.piece.visual.SetContent(&surface);
                }
                t.piece.surface = surface;
                t.piece.alloc = (need.0 + 32, need.1 + 32);
            }
        }
        let _ = t.piece.place((x * s).round() as i32, (y * s).round() as i32);
        let surface = t.piece.surface.clone();
        let p = self.painter(s);
        let g = &self.g;
        let mut boxes: Vec<(isize, Rect)> = Vec::new();
        let _ = g.draw_surface(&surface, |dc| unsafe {
            let k = (0.5 + 0.5 * g_now).max(0.05);
            let mut base = windows::Foundation::Numerics::Matrix3x2::default();
            dc.GetTransform(&mut base);
            let (px, py) = (pivot.0 * s, pivot.1 * s);
            dc.SetTransform(&(windows::Foundation::Numerics::Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: px * (1.0 - k), M32: py * (1.0 - k) } * base));
            with_opacity(dc, g, g_now.clamp(0.0, 1.0), || {
                let _ = fill_shape(dc, &shape, FACE);
                if let Some(words) = content.as_str() {
                    // (lines alike: no title)
                    for (i, l) in words.lines().filter(|l| !l.is_empty()).enumerate() {
                        p.text(l, P + 1.0, 15.0 + i as f32 * 17.0, 12.0, Weight::Regular, 300.0, draw::TEXT);
                    }
                } else {
                    let mut c = Ctx { p: &p, draw: true, hits: Vec::new(), open: &empty, closed: &empty, thumbs: Vec::new() };
                    let inner = w - 2.0 * P;
                    let head = blocks::head(&mut c, &content, P, P, inner);
                    c.body(content["body"].as_array().map(|v| v.as_slice()).unwrap_or(&[]), P, P + head + if head > 0.0 { 10.0 } else { 0.0 }, inner);
                    boxes = std::mem::take(&mut c.thumbs);
                }
            });
        });
        drop(p);
        let glass = self.screens[si].ov.top_hwnd();
        let t = self.tip.as_mut().unwrap();
        t.moving = crate::draw::take_marquee();
        t.drawn = Some((t.content.clone(), t.anchor));
        // (its windows' live pictures: once it stands still, in their boxes; while it springs, none)
        if settled && !boxes.is_empty() {
            t.thumbs.retain(|(wnd, th)| {
                let keep = boxes.iter().any(|(b, _)| b == wnd);
                if !keep {
                    crate::thumbs::unregister(*th);
                }
                keep
            });
            for (wnd, r) in boxes {
                let th = match t.thumbs.iter().find(|(x, _)| *x == wnd) {
                    Some((_, th)) => *th,
                    None => match crate::thumbs::register(glass, wnd) {
                        Some(th) => {
                            t.thumbs.push((wnd, th));
                            th
                        }
                        None => continue,
                    },
                };
                let px = RECT { left: ((x + r.x) * s) as i32, top: ((y + r.y) * s) as i32, right: ((x + r.x + r.w) * s) as i32, bottom: ((y + r.y + r.h) * s) as i32 };
                crate::thumbs::place(th, px, true);
            }
        } else {
            drop_tip_thumbs(t);
        }
    }

    /// The menu's press under a point (logical, in its screen), by its place in the list.
    fn menu_hit(&self, si: usize, x: f32, y: f32) -> Option<usize> {
        let m = self.menu.as_ref().filter(|m| !m.closing && m.si == si && m.at.contains(x, y))?;
        let (lx, ly) = (x - m.at.x, y - m.at.y);
        m.hits.iter().position(|(r, _)| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h)
    }

    fn in_menu(&self, si: usize, x: f32, y: f32) -> bool {
        self.menu.as_ref().is_some_and(|m| !m.closing && m.si == si && m.at.contains(x, y))
    }

    /// A click with the menu open: on it, what it says; elsewhere, it goes.
    fn menu_click(&mut self, si: usize, x: f32, y: f32) {
        if !self.in_menu(si, x, y) {
            self.close_menu();
            return;
        }
        let Some(i) = self.menu_hit(si, x, y) else { return };
        let m = self.menu.as_ref().unwrap();
        let act = m.hits[i].1.clone();
        let k = m.key.as_ref().map(|k| unpop(k).to_string());
        let set = |c: &mut Config, k: &str, f: &dyn Fn(&mut config::WidgetConfig)| f(c.widgets.entry(k.to_string()).or_default());
        use crate::menu::Act;
        if let Act::Item(i) = act {
            let Some((plugin, items)) = self.menu.as_ref().and_then(|m| m.items.clone()) else { return };
            let Some(it) = items.get(i) else { return };
            if it["hold"] == true {
                return;
            }
            let action = it["action"].as_str().unwrap_or("").to_string();
            self.close_menu();
            self.act(plugin, action, json!({}));
            return;
        }
        match (act, k) {
            (Act::Look(f, l, i), Some(k)) => {
                // (one only ever its icon keeps it)
                let fixed = self.icon_always(&k);
                self.change_config(|c| {
                    set(c, &k, &|w| {
                        w.style = Some(if f { "floating" } else { "merged" }.into());
                        w.size = Some(if l { "large" } else { "small" }.into());
                        if !fixed {
                            w.icon_only = Some(i);
                        }
                    })
                })
            }
            (Act::Move(p), Some(k)) => self.change_config(|c| {
                set(c, &k, &|w| {
                    w.place = Some(p.into());
                    w.stack = None;
                    w.rank = None;
                })
            }),
            (Act::Pin, Some(k)) => {
                let here = self.screens[si].ov.monitor.name();
                self.change_config(|c| set(c, &k, &|w| w.monitor = if w.monitor.as_deref() == Some(here.as_str()) { None } else { Some(here.clone()) }));
            }
            (Act::AlwaysShow, Some(k)) => self.change_config(|c| set(c, &k, &|w| w.always_shown = if w.always_shown == Some(true) { None } else { Some(true) })),
            (Act::OverFull, Some(k)) => self.change_config(|c| set(c, &k, &|w| w.over_full = if w.over_full == Some(true) { None } else { Some(true) })),
            (Act::HideIdle, Some(k)) => self.change_config(|c| set(c, &k, &|w| w.show_idle = if w.show_idle == Some(true) { None } else { Some(true) })),
            (Act::Bump, Some(k)) => {
                let on = self.bumps_on(&k);
                self.change_config(|c| set(c, &k, &|w| w.bump = Some(!on)));
            }
            (Act::AlwaysOpen, Some(k)) => {
                let was = self.always_open(&k);
                self.change_config(|c| set(c, &k, &|w| w.always_open = if w.always_open == Some(true) { None } else { Some(true) }));
                // (no longer kept open: it shuts now)
                if was {
                    if let Some(it) = self.screens[si].items.get_mut(&k) {
                        it.open.to = 0.0;
                        it.pinned = false;
                    }
                    self.set_open(si, &k, false);
                    self.relayout_with(Some(&HashSet::new()));
                }
            }
            (Act::Hide, Some(k)) => {
                self.close_menu();
                self.change_config(|c| set(c, &k, &|w| w.place = Some("off".into())));
                return;
            }
            (Act::Settings, k) => {
                // (a widget's: its plugin's own page; NADI's own widgets: the frame's)
                let goto = k.as_ref().and_then(|k| self.widgets.get(k.as_str())).map(|w| if w.plugin == crate::builtin::PLUGIN { "frame".to_string() } else { format!("plugin:{}", w.plugin) });
                self.close_menu();
                self.open_settings_at(goto);
                return;
            }
            (Act::PopOut, Some(k)) => {
                self.close_menu();
                if self.popped(&k) {
                    self.pop_in(&pop_key(&k));
                } else {
                    self.pop_out(si, &k, None);
                }
                return;
            }
            (Act::Edit, _) => {
                self.close_menu();
                self.enter_edit(si);
                return;
            }
            (Act::Exit, _) => {
                self.close_menu();
                unsafe { PostQuitMessage(0) };
                return;
            }
            _ => {}
        }
        // (the widget changed: the menu shows it, where the widget is now)
        if self.menu.is_some() {
            self.render_menu();
            self.hit_regions();
            self.commit();
        }
    }

    /// While the menu is open: a click (either button) anywhere off it, or Esc, puts it away.
    fn menu_watch(&mut self) {
        let Some(m) = self.menu.as_ref().filter(|m| !m.closing) else {
            self.rbutton = false;
            return;
        };
        let (si, at) = (m.si, m.at);
        let (l, r) = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0, GetAsyncKeyState(VK_RBUTTON.0 as i32) as u16 & 0x8000 != 0) };
        let esc = self.esc;
        let r_was = std::mem::replace(&mut self.rbutton, r);
        let pressed = (l && !self.lbutton) || (r && !r_was);
        if esc {
            self.close_menu();
            return;
        }
        if pressed {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            let sc = &self.screens[si];
            let mr = sc.ov.monitor.rect;
            let (x, y) = ((pt.x - mr.left) as f32 / sc.s, (pt.y - mr.top) as f32 / sc.s);
            let on = pt.x >= mr.left && pt.x < mr.right && pt.y >= mr.top && pt.y < mr.bottom;
            if !(on && at.contains(x, y)) {
                self.close_menu();
            }
        }
    }

    pub fn wheel(&mut self, si: usize, x: f32, y: f32, delta: f32) {
        let Some(k) = self.item_at(si, x, y) else { return };
        if k == key(crate::builtin::PLUGIN, crate::builtin::QUICK) && self.screens[si].items.get(&k).is_some_and(|i| i.open.now < 0.6) {
            self.bi.nudge_volume(delta > 0.0);
            self.sync_builtins(true);
            return;
        }
        // (a widget at rest that asks for the wheel: it is told, up or down (Now Playing: its players one after another))
        if let Some(w) = self.widgets.get(unpop(&k)).filter(|w| w.content["wheel"].is_string()) {
            if self.screens[si].items.get(&k).is_some_and(|i| i.open.now < 0.6) {
                let (plugin, a) = (w.plugin.clone(), w.content["wheel"].as_str().unwrap_or("").to_string());
                self.act(plugin, a, json!({ "up": delta > 0.0 }));
                return;
            }
        }
        // (the dock at rest, the wheel over an app: its windows one after another)
        if k == key(crate::builtin::PLUGIN, crate::builtin::DOCK) && self.screens[si].items.get(&k).is_some_and(|i| i.open.now < 0.6) {
            let it = &self.screens[si].items[&k];
            if let Ok((_, b)) = self.shape_now(si, it) {
                let (lx, ly) = (x - b.x, y - b.y);
                let app = it.buttons.iter().find(|(r, a)| a.starts_with("app:") && lx >= r.x - 4.0 && lx < r.x + r.w + 4.0 && ly >= r.y - 6.0 && ly < r.y + r.h + 6.0).map(|(_, a)| a[4..].to_string());
                if let Some(i) = app {
                    self.act(crate::builtin::PLUGIN.to_string(), format!("appwheel:{i}:{}", if delta > 0.0 { "up" } else { "down" }), json!({}));
                }
            }
            return;
        }
        // (open, over a picture (or anything on it, a card, a button): nearer or further, at the mouse)
        if let Some(it) = self.screens[si].items.get(&k).filter(|i| i.open.now >= 0.6) {
            if let Ok((_, b)) = self.shape_now(si, it) {
                let (lx, ly) = (x - b.x, y - b.y);
                let view = it.hits.iter().filter(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).find_map(|h| match &h.press {
                    Press::View(id) => Some(id.clone()),
                    _ => None,
                });
                if let Some(id) = view {
                    if crate::blocks::view_wheel(&id, lx, ly, delta > 0.0) {
                        self.render(si, &k);
                        self.commit();
                    }
                    return;
                }
            }
        }
        // (open, over a slider: it steps, 2% a notch)
        if let Some(it) = self.screens[si].items.get(&k).filter(|i| i.open.now >= 0.6) {
            if let Ok((_, b)) = self.shape_now(si, it) {
                let (lx, ly) = (x - b.x, y - b.y);
                let on = it.hits.iter().find(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h).and_then(|h| match &h.press {
                    Press::Seek(a) => blocks::slider_value(a).map(|v| (a.clone(), v)),
                    _ => None,
                });
                if let (Some((action, v)), Some(w)) = (on, self.widgets.get(unpop(&k))) {
                    let base = self.slid.as_ref().filter(|(_, sk, _)| *sk == k).and_then(|_| blocks::sliding_at(&action)).unwrap_or(v);
                    let to = (((base + if delta > 0.0 { 0.02 } else { -0.02 }) * 50.0).round() / 50.0).clamp(0.0, 1.0);
                    let plugin = w.plugin.clone();
                    self.act(plugin, action.clone(), json!({ "fraction": to }));
                    blocks::set_sliding(Some((action, to)));
                    self.slid = Some((si, k.clone(), Instant::now()));
                    self.render(si, &k);
                    self.commit();
                    return;
                }
            }
        }
        let it = self.screens[si].items.get_mut(&k).unwrap();
        if it.open.now < 0.6 {
            return;
        }
        it.scroll = (it.scroll - delta * 0.5).clamp(0.0, (it.content_h - it.visible).max(0.0));
        // (back at its end: it keeps to it again; else it stays where it is put)
        if it.scroll >= (it.content_h - it.visible).max(0.0) - 2.0 {
            self.wheeled.remove(&k);
            self.seen_h.remove(&k);
        } else {
            self.wheeled.insert(k.clone(), Instant::now());
            self.seen_h.insert(k.clone(), it.content_h);
        }
        it.scrolled_at = Some(Instant::now());
        self.render(si, &k);
        self.commit();
    }

    /// How much its scrollbar shows (0..1): only while the mouse is on it, a moment after it came there or scrolled, then fading.
    fn scrollbar(&self, si: usize, k: &str) -> f32 {
        let Some(it) = self.screens.get(si).and_then(|sc| sc.items.get(k)) else { return 0.0 };
        if !self.hover.as_ref().is_some_and(|(hs, hk)| *hs == si && hk == k) {
            return 0.0;
        }
        let age = [it.scrolled_at, it.hover_since].iter().flatten().map(|t| t.elapsed().as_secs_f32()).fold(f32::MAX, f32::min);
        (1.0 - (age - SCROLLBAR_FOR) / 0.3).clamp(0.0, 1.0)
    }

    /// Is a scrollbar showing or fading (it is drawn again till it is gone)?
    fn scrollbar_moves(&self) -> Option<(usize, String)> {
        let (si, k) = self.hover.clone()?;
        let it = self.screens.get(si)?.items.get(&k)?;
        let age = [it.scrolled_at, it.hover_since].iter().flatten().map(|t| t.elapsed().as_secs_f32()).fold(f32::MAX, f32::min);
        (it.content_h > it.visible + 1.0 && age < SCROLLBAR_FOR + 0.4).then_some((si, k))
    }

    pub fn screen_scale(&self, si: usize) -> f32 {
        self.screens[si].s
    }
}

/// Draw a piece: its shape, and its peek (fading as it opens, `t` 0..1) or its panel. What can be pressed in it, its button, and whether
/// something in it moves.
#[allow(clippy::too_many_arguments)]
pub fn paint_piece(p: &Painter, g: &Gfx, dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, shape: &Shape, slot: &Slot, content: &Value, t: f32, open_set: &HashSet<String>, closed_set: &HashSet<String>, scroll: f32, content_h: f32, bar: f32, fade: f32, floating: bool, under: &mut dyn FnMut(&Painter, Rect), over: &mut dyn FnMut(&Painter, Rect)) -> (Vec<Hit>, Vec<(Rect, String)>, bool, Vec<(isize, Rect)>) {
    let s = p.s;
    let mut hits = Vec::new();
    let mut button = Vec::new();
    let mut alive = false;
    let mut boxes = Vec::new();
        // (in the liquid, its shape is the liquid's: drawn under it, run into its edge and its neighbours)
        if content["\u{0}liquid"] != true {
            let _ = fill_shape(dc, &shape, FACE);
        }
        let body = Rect::new(shape.body.x / s, shape.body.y / s, shape.body.w / s, shape.body.h / s);
        under(p, body);
        let pk = &content["peek"];
        if t < 0.35 {
            // the peek, fading as it opens (and in, after its piece has grown out)
            let a = (1.0 - t / 0.35) * fade;
            with_opacity(dc, g, a, || {
                let peek_body = if floating { body } else { match slot.place {
                    // (a corner's peek lies along its horizontal edge, out of the curve)
                    Place::Corner(Corner::TopLeft) | Place::Corner(Corner::TopRight) => Rect::new(body.x, body.y, body.w, slot.h - Metrics::default().fillet),
                    Place::Corner(_) => Rect::new(body.x, body.bottom() - (slot.h - Metrics::default().fillet), body.w, slot.h - Metrics::default().fillet),
                    Place::Edge(Edge::Bottom) => Rect::new(body.x, body.bottom() - slot.h, body.w, slot.h),
                    Place::Edge(Edge::Right) => Rect::new(body.right() - slot.w, body.y, slot.w, body.h),
                    _ => body,
                } };
                // (while it grows: its contents where they will be at its full size, centred on it, kept inside it)
                let full = if slot.place.horizontal() { slot.w - 2.0 * Metrics::default().fillet } else { slot.h - 2.0 * Metrics::default().fillet };
                let growing = fade < 1.0 && (if slot.place.horizontal() { peek_body.w } else { peek_body.h }) < full - 0.5;
                let peek_body = if growing {
                    if slot.place.horizontal() {
                        Rect::new(peek_body.x + peek_body.w / 2.0 - full / 2.0, peek_body.y, full, peek_body.h)
                    } else {
                        Rect::new(peek_body.x, peek_body.y + peek_body.h / 2.0 - full / 2.0, peek_body.w, full)
                    }
                } else {
                    peek_body
                };
                if growing {
                    unsafe {
                        dc.PushAxisAlignedClip(&p.px(body).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
                    }
                }
                button = peek::draw(&p, pk, peek_body, slot.place.horizontal(), slot.dot);
                if growing {
                    unsafe {
                        dc.PopAxisAlignedClip();
                    }
                }
            });
            alive = (shows_bars(pk) || pk["level"]["playing"] == true) && p.viz.iter().any(|v| *v > 0.0)
                || pk["level"]["playing"] == true
                || matches!(pk["voice"]["state"].as_str(), Some("listening" | "review"))
                || matches!(pk["live"]["tone"].as_str(), Some("working" | "waiting" | "permission"))
                || [&pk["image"], &pk["live"]["image"]].iter().any(|v| v.as_str().is_some_and(crate::images::animated));
        }
        if t > 0.55 {
            let a = ((t - 0.55) / 0.45).min(1.0);
            let panel = &content["panel"];
            // (room over the content for its handle, the handle in it while the mouse is on it)
            let top = content["\u{0}top"].as_f64().unwrap_or(0.0) as f32;
            let inner = Rect::new(body.x + PAD, body.y + PAD + top, body.w - 2.0 * PAD, body.h - 2.0 * PAD - top);
            if let Some(ha) = content["\u{0}handle"].as_f64() {
                let cy = body.y + (PAD + top) / 2.0 + 1.0;
                let hw = content["\u{0}handle_w"].as_f64().unwrap_or(32.0) as f32;
                p.round(Rect::new(body.x + body.w / 2.0 - hw / 2.0, cy - 2.0, hw, 4.0), 2.0, draw::DIM.alpha(0.7 * a * ha as f32));
            }
            // (washed at its top in a picture's colours (a cover's), fading into the panel)
            if let Some((r, gr, bl)) = panel["wash"].as_str().and_then(|w| crate::images::tone(p.pc, w)) {
                let col = crate::paint::Color(r * 0.6, gr * 0.6, bl * 0.6, 0.65 * a);
                let _ = crate::paint::fill_shape_fade(dc, shape, col, shape.h * 0.55);
            }
            with_opacity(dc, g, a, || unsafe {
                dc.PushAxisAlignedClip(&p.px(inner).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
                let mut c = Ctx { p: &p, draw: true, hits: Vec::new(), open: &open_set, closed: &closed_set, thumbs: Vec::new() };
                let y0 = inner.y - scroll;
                let head = blocks::head(&mut c, panel, inner.x, y0, inner.w);
                let zoom = panel["zoom"].as_f64().unwrap_or(1.0).clamp(0.8, 1.6) as f32;
                c.body_zoomed(panel["body"].as_array().map(|v| v.as_slice()).unwrap_or(&[]), inner.x, y0 + head + if head > 0.0 { 14.0 } else { 0.0 }, inner.w, zoom);
                // (only what shows can be pressed)
                hits = c.hits.into_iter().filter(|h| h.rect.y + h.rect.h > inner.y && h.rect.y < inner.bottom()).collect();
                boxes = c.thumbs.into_iter().filter(|(_, r)| r.y >= inner.y - 1.0 && r.bottom() <= inner.bottom() + 1.0).collect();
                // (more above or below: that edge fades out, into the panel)
                if content_h > inner.h + 1.0 {
                    const FADE: f32 = 26.0;
                    const STEPS: usize = 13;
                    let up = (scroll / FADE).clamp(0.0, 1.0);
                    let down = ((content_h - inner.h - scroll) / FADE).clamp(0.0, 1.0);
                    let step = FADE / STEPS as f32;
                    for i in 0..STEPS {
                        let a = (1.0 - i as f32 / STEPS as f32).powf(1.6);
                        if up > 0.0 {
                            p.round(Rect::new(inner.x, inner.y + i as f32 * step, inner.w, step + 0.5), 0.0, draw::FACE.alpha(a * up));
                        }
                        if down > 0.0 {
                            p.round(Rect::new(inner.x, inner.bottom() - (i + 1) as f32 * step, inner.w, step + 0.5), 0.0, draw::FACE.alpha(a * down));
                        }
                    }
                }
                dc.PopAxisAlignedClip();
                // (one that keeps to its end, scrolled back: an arrow at its foot back to now, a red dot when there is more since)
                if panel["stick"] == "bottom" && content_h - inner.h - scroll > 24.0 {
                    let newer = content["\u{0}newer"] == true;
                    let pw = if newer { 52.0 } else { 40.0 };
                    let r = Rect::new(inner.x + inner.w / 2.0 - pw / 2.0, inner.bottom() - 36.0, pw, 28.0);
                    p.round(r, 14.0, crate::paint::Color(0.149, 0.149, 0.173, 1.0));
                    p.icon("arrow-down", r.x + 13.0, r.y + 7.0, 14.0, draw::TEXT);
                    if newer {
                        p.circle(r.x + 37.0, r.y + 14.0, 3.0, crate::paint::Color(0.91, 0.416, 0.416, 1.0));
                    }
                    hits.push(blocks::Hit { rect: r, press: Press::Action(BACK_TO_NOW.into(), Value::Null), tip: Value::Null, hold: false, menu: Value::Null });
                }
                // (more below: a thin line says so, while the mouse is on it and it was just scrolled or come to)
                if content_h > inner.h + 1.0 && bar > 0.01 {
                    let track = inner.h;
                    let part = (track / content_h).clamp(0.1, 1.0);
                    let at = scroll / (content_h - track).max(1.0);
                    p.round(Rect::new(inner.right() + 5.0, inner.y + (track - track * part) * at, 4.0, track * part), 2.0, crate::paint::Color(1.0, 1.0, 1.0, 0.4 * bar));
                }
            });
            alive = true;
        }
    if t < 0.35 {
        let body = Rect::new(shape.body.x / s, shape.body.y / s, shape.body.w / s, shape.body.h / s);
        over(p, body);
    }
    (hits, button, alive, boxes)
}

fn is_first_section(w: Option<&Widget>, id: &str) -> bool {
    w.and_then(|w| w.content["panel"]["body"].as_array().and_then(|b| b.iter().find(|x| x["kind"] == "section")).map(|x| x["id"] == id)).unwrap_or(false)
}

/// What decides a peek's length (its words and what leads it): a change of anything else is drawn in place.
fn peek_shape(p: &Value) -> String {
    if let Some(d) = p["dock"]["items"].as_array() {
        // (how many, and which have meters (or a stage))
        let marks: String = d.iter().map(|i| if i["front"] == true { 'F' } else if i["cpu"].is_null() { '.' } else if !i["progress"].is_null() { 'p' } else if i["sound"] == true { 's' } else { 'm' }).collect();
        return format!("dock{}{}", d.len(), marks);
    }
    if let Some(t) = p["tray"]["items"].as_array() {
        let words: String = t.iter().map(|i| i["news"].to_string()).collect::<Vec<_>>().join("|");
        return format!("tray{}+{}{}{}", t.len(), p["tray"]["more"].as_u64().unwrap_or(0), p["tray"]["large"] == true, words);
    }
    if let Some(n) = p["desks"]["n"].as_u64() {
        return format!("desks{n}");
    }
    if p["meters"].is_array() {
        return "meters".into();
    }
    if p["song"].is_object() {
        let s = &p["song"];
        return format!("song|{}|{}|{}|{}|{}", s["title"], s["artist"], s["alone"], s["stack"].as_array().map_or(0, |a| a.len()), s["left"].is_string());
    }
    if let Some(g) = p["graphs"]["items"].as_array() {
        return format!("graphs{}", g.iter().map(|i| i["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join("|"));
    }
    let lead = ["image", "icon", "live", "ring", "vessel", "bars", "clock", "button", "dot", "viz", "cells", "pad", "roll"].iter().filter(|k| !p[**k].is_null() && p[**k] != "").map(|k| *k).collect::<Vec<_>>().join(",");
    format!("{}|{}", lead, p["text"].as_str().unwrap_or(""))
}

/// Gone back into its edge, and staying there (not held, not open, not a window): nothing of it is drawn.
/// (for finding out: a widget that could not be drawn, a line in render.log; kept small)
fn note_render_error(k: &str, e: &windows::core::Error) {
    let Some(dir) = nadi_core::paths::data_dir() else { return };
    let f = dir.join("render.log");
    if std::fs::metadata(&f).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&f);
    }
    use std::io::Write;
    if let Ok(mut o) = std::fs::OpenOptions::new().create(true).append(true).open(&f) {
        let _ = writeln!(o, "{} {k}: {e}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

fn tucked(it: &Item, k: &str) -> bool {
    it.shown.now <= 0.005 && it.shown.to <= 0.0 && it.drag_place.is_none() && it.open.now < 0.01 && !is_pop(k)
}

/// What an app on the dock plays, as its card shows it (the session whose player is that app: by its program's name): its song,
/// who sings it, its cover, how far along. Nothing when it plays nothing.
fn media_of(sessions: &Value, it: &Value) -> Option<Value> {
    let key = it["key"].as_str().unwrap_or("").to_lowercase();
    let stem = key.rsplit(['\\', '/']).next().unwrap_or("").trim_end_matches(".exe").to_string();
    let name = it["name"].as_str().unwrap_or("").to_lowercase().replace(' ', "");
    let s = sessions.as_array()?.iter().find(|s| {
        let src = s["source"].as_str().unwrap_or("").to_lowercase();
        (!stem.is_empty() && src.contains(&stem)) || (name.len() > 2 && src.replace(' ', "").contains(&name))
    })?;
    let title = s["title"].as_str().unwrap_or("");
    if title.is_empty() {
        return None;
    }
    let dur = s["duration"].as_f64().unwrap_or(0.0);
    let art = s["art_id"].as_u64().filter(|a| *a != 0).map(|a| format!("media:{}:{}", s["source"].as_str().unwrap_or(""), a));
    Some(json!({
        "title": title, "artist": s["artist"], "art": art, "source": s["source"], "playing": s["playing"],
        "progress": if dur > 0.0 { json!((s["position"].as_f64().unwrap_or(0.0) / dur).clamp(0.0, 1.0)) } else { Value::Null },
    }))
}

/// how long the dock takes to bring the app in front forward (its card growing, its ground popping in)
const DOCK_FRONT_S: f32 = 0.35;

/// a glow, as it comes out (times today's strength), and settled; how long it takes to settle; its rings: for how long, how often
/// A glow drawn on its piece (on the glass under the frame): soft light out of a shape, and, while it is fresh, rings going out from it.
/// (`outline`: its rings follow its outline, grown, as a banner's; else they are pills round its box, as a widget's)
fn paint_glow(g: &Gfx, gp: &mut Piece, s: f32, shape: &Shape, b: Bx, outline: bool, c: crate::paint::Color, strength: f32, age: f32) {
    let m = (GLOW * s).round();
    let _ = gp.place((b.x * s).round() as i32 - m as i32, (b.y * s).round() as i32 - m as i32);
    let surface = gp.surface.clone();
    // (its shape, filled, blurred: soft light out of its outline; inside it, the piece covers it). The shape is drawn first, on
    // its own, into a list of drawing to blur.
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct2D::*;
    let dc = &g.dc;
    let list = unsafe {
        let Ok(list) = dc.CreateCommandList() else { return };
        dc.SetTarget(&list);
        dc.BeginDraw();
        dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2::translation(m, m));
        if let Ok(br) = crate::paint::brush(dc, c.alpha((0.55 * strength).min(1.0))) {
            dc.FillGeometry(&shape.geometry, &br, None);
        }
        let _ = dc.EndDraw(None, None);
        dc.SetTarget(None);
        let _ = list.Close();
        list
    };
    let _ = g.draw_surface(&surface, |dc| unsafe {
        let Ok(blur) = dc.CreateEffect(&CLSID_D2D1GaussianBlur) else { return };
        let Ok(img) = list.cast::<ID2D1Image>() else { return };
        blur.SetInput(0, &img, true);
        let _ = blur.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &(GLOW * s / 3.2).to_le_bytes());
        if let Ok(out) = blur.GetOutput() {
            dc.DrawImage(&out, None, None, D2D1_INTERPOLATION_MODE_LINEAR, windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE_SOURCE_OVER);
        }
        // (none of it inside its shape: a widget fading in or out, see-through, never shows its glow through it)
        if let Ok(clear) = crate::paint::brush(dc, crate::paint::Color(0.0, 0.0, 0.0, 0.0)) {
            dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2::translation(m, m));
            dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
            dc.FillGeometry(&shape.geometry, &clear, None);
            dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
            dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2::identity());
        }
        let _ = (age, outline, b);
    });
}

/// Does a peek show the bars of the sound (so the sound is listened to, and it is drawn again as they move)? Its `viz`, or a song
/// with a player playing.
fn shows_bars(peek: &Value) -> bool {
    peek["viz"].as_bool() == Some(true) || peek["song"]["playing"] == true || peek["song"]["busy"] == true
}

/// Where on a slider's (or a bar's) press area the mouse is, 0..1, along its track (the area less its ends).
fn slide_frac(r: Rect, lx: f32) -> f32 {
    let inset = blocks::SEEK_INSET.min(r.w / 4.0);
    ((lx - r.x - inset) / (r.w - 2.0 * inset).max(1.0)).clamp(0.0, 1.0)
}

/// how round a banner's corners are (logical px)
const BANNER_RADIUS: f32 = 20.0;

const GLOW_FRESH: f32 = 2.0;
const GLOW_SETTLED: f32 = 0.8;
const GLOW_SETTLE_S: f32 = 1.6;

/// where a live banner's words are typed in (in `typed`)
const LIVE_BANNER: &str = "\u{0}banner";

/// the arrow at the foot of a panel scrolled back: pressed, it goes back to its end
const BACK_TO_NOW: &str = "\u{0}now";

/// how long a panel's scrollbar stays after the mouse comes to it or scrolls it (seconds), before it fades
const SCROLLBAR_FOR: f32 = 1.2;

/// A program's three sound bars (0..1 each) for how loud it is now: quiet sound still stands up.
/// (a fourth: how far they are lifted, with how loud it is, as the bass lifts the sound's own)
fn sound_bars(loud: f32, t: f32) -> [f32; 4] {
    let (p, lift) = crate::draw::loud_bars(loud, t);
    [p[0], p[1], p[2], lift]
}

/// a dock meter's change smaller than this (of its whole) is not shown
const METER_STILL: f32 = 0.04;

/// How big the liquid's necks are (the round where shapes run together, logical px), for a blur of `px`.
/// Every path the pins name (what they are, their pictures): NADI's own copies among them are kept.
fn kept_paths(c: &Config) -> Vec<String> {
    let mut out: Vec<String> = c.pinned.iter().flat_map(|p| [p.id.clone(), p.icon.clone()]).collect();
    for l in [&c.launcher_pins, &c.launcher_foot].into_iter().flatten() {
        out.extend(l.iter().flat_map(|p| [p.id.clone(), p.icon.clone()]));
    }
    out.retain(|p| !p.is_empty());
    out
}

/// Shortcuts pinned before NADI kept its own copies: copied now (the pin then works on when the one pinned is gone). Changed?
fn keep_shortcuts(c: &mut Config) -> bool {
    let mut changed = false;
    let mut keep = |to: &str, id: &mut String| {
        if id.to_lowercase().ends_with(".lnk") && std::path::Path::new(id.as_str()).is_file() {
            if let Some(k) = crate::explorer::keep_shortcut(id).filter(|k| !k.eq_ignore_ascii_case(id)) {
                crate::explorer::remember(to, id, &k);
                *id = k;
                changed = true;
            }
        }
    };
    for p in &mut c.pinned {
        keep("dock", &mut p.id);
    }
    for p in c.launcher_pins.iter_mut().flatten().filter(|p| p.kind == "file") {
        keep("apps", &mut p.id);
    }
    changed
}

/// What File Explorer's menu offers to unpin: the paths pinned to the launcher, and to the dock (a program's, a file's, a folder's;
/// and the shortcuts pinned as their programs).
fn explorer_paths(c: &Config) -> (Vec<String>, Vec<String>) {
    let is_path = |id: &str| id.len() > 2 && id.as_bytes()[1] == b':';
    let sources = crate::explorer::sources();
    let with_sources = |to: &str, ids: Vec<String>| -> Vec<String> {
        let mut out = ids.clone();
        for (t, src, id) in &sources {
            if t == to && ids.iter().any(|i| i.eq_ignore_ascii_case(id)) && !out.iter().any(|o| o.eq_ignore_ascii_case(src)) {
                out.push(src.clone());
            }
        }
        out
    };
    let launcher: Vec<String> = c.launcher_pins.as_ref().map(|l| l.iter().filter(|p| (p.kind == "app" || p.kind == "file") && is_path(&p.id)).map(|p| p.id.clone()).collect()).unwrap_or_default();
    let dock: Vec<String> = c.pinned.iter().filter(|p| is_path(&p.id)).map(|p| p.id.clone()).collect();
    (with_sources("apps", launcher), with_sources("dock", dock))
}

pub fn liquid_neck(px: f32) -> f32 {
    (px * 1.6).max(2.0)
}

/// How liquid, from the setting (0 to 100): the blur's reach, logical px (0: none).
pub fn liquid_px(v: f32) -> Option<f32> {
    let v = v.clamp(0.0, 100.0);
    (v >= 1.0).then(|| 2.5 + v / 100.0 * 7.5)
}

/// In the liquid's shapes, a widget's neck on edge `e`: its box, and this in place of how round.
pub fn neck_code(e: Edge) -> f32 {
    -10.0 - match e {
        Edge::Top => 0.0,
        Edge::Bottom => 1.0,
        Edge::Left => 2.0,
        Edge::Right => 3.0,
    }
}

pub fn neck_edge(r: f32) -> Option<Edge> {
    match r {
        x if x == -10.0 => Some(Edge::Top),
        x if x == -11.0 => Some(Edge::Bottom),
        x if x == -12.0 => Some(Edge::Left),
        x if x == -13.0 => Some(Edge::Right),
        _ => None,
    }
}

/// The necks of the merged widgets among the liquid's shapes, drawn over the liquid as they are, not blurred: the blur would wear
/// their thin tips off, and they would stop short of the edge.
pub fn draw_necks(dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, fac: &windows::Win32::Graphics::Direct2D::ID2D1Factory1, shapes: &[(Bx, f32)], s: f32, ox: f32, oy: f32, px: f32) {
    let Ok(br) = crate::paint::brush(dc, FACE) else { return };
    for (b, r) in shapes {
        if let Some(e) = neck_edge(*r) {
            for g in necks(fac, *b, e, s, ox, oy, px) {
                unsafe {
                    dc.FillGeometry(&g, &br, None);
                }
            }
        }
    }
}

/// A merged widget's two necks (box `b`, logical, its shoulders in it): at each end, the hollow curve from its edge into its side,
/// as its own shape has it, filled out past the edge. (Its side as the liquid has it: the blur, made sharp again, leaves the body
/// a little wider, by about a fifth of `px`.)
fn necks(fac: &windows::Win32::Graphics::Direct2D::ID2D1Factory1, b: Bx, e: Edge, s: f32, ox: f32, oy: f32, px: f32) -> Vec<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry> {
    use windows::Win32::Graphics::Direct2D::Common::*;
    let f = Metrics::default().fillet;
    let len = if matches!(e, Edge::Top | Edge::Bottom) { b.w } else { b.h };
    // (coming out or going back in: as small as the widget is out of its edge, never curving past it)
    let across = if matches!(e, Edge::Top | Edge::Bottom) { b.h } else { b.w };
    // (the blur wears a thin body away faster than it shrinks: the necks go first, gone before it is 10 out)
    let k = ((across - 10.0) / 22.0).clamp(0.0, 1.0);
    let f = (f * k * k.sqrt()).min(across * 0.6).max(0.0);
    if f < 0.5 {
        return Vec::new();
    }
    // (u along the edge from the box's start, v out from the edge)
    let at = |u: f32, v: f32| {
        let (x, y) = match e {
            Edge::Top => (b.x + u, b.y + v),
            Edge::Bottom => (b.x + u, b.y + b.h - v),
            Edge::Left => (b.x + v, b.y + u),
            Edge::Right => (b.x + b.w - v, b.y + u),
        };
        D2D_POINT_2F { x: x * s - ox, y: y * s - oy }
    };
    let past = 40.0;
    let d = 0.21 * px;
    let mut out = Vec::new();
    for end in [false, true] {
        let u = |u: f32| if end { len - u } else { u };
        // (a little into the body, so no seam shows where they meet)
        let side = f - d;
        let mut pts = vec![at(u(-d), -past), at(u(side + 2.0), -past), at(u(side + 2.0), f)];
        // (the hollow: a quarter round about (-d, f), from the side at (side, f) down to the edge at (-d, 0))
        for i in 0..=16 {
            let t = i as f32 / 16.0 * std::f32::consts::FRAC_PI_2;
            pts.push(at(u(-d + f * t.cos()), f - f * t.sin()));
        }
        unsafe {
            let Ok(g) = fac.CreatePathGeometry() else { continue };
            let Ok(sink) = g.Open() else { continue };
            sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&pts[1..]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                out.push(g.into());
            }
        }
    }
    out
}

/// The liquid's shapes (logical boxes, how round), recorded to be blurred: drawn at `s` px a logical px, less `ox`, `oy`. Recorded
/// outside any drawing.
pub fn liquid_list(dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, _fac: &windows::Win32::Graphics::Direct2D::ID2D1Factory1, shapes: &[(Bx, f32)], s: f32, ox: f32, oy: f32, _px: f32) -> Option<windows::Win32::Graphics::Direct2D::ID2D1CommandList> {
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    unsafe {
        let list = dc.CreateCommandList().ok()?;
        dc.SetTarget(&list);
        dc.BeginDraw();
        dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2::identity());
        if let Ok(br) = crate::paint::brush(dc, FACE) {
            for (b, r) in shapes {
                let rect = D2D_RECT_F { left: b.x * s - ox, top: b.y * s - oy, right: (b.x + b.w) * s - ox, bottom: (b.y + b.h) * s - oy };
                if neck_edge(*r).is_some() {
                    continue;
                } else if *r > 0.0 {
                    dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: r * s, radiusY: r * s }, &br);
                } else {
                    dc.FillRectangle(&rect, &br);
                }
            }
        }
        let _ = dc.EndDraw(None, None);
        dc.SetTarget(None);
        let _ = list.Close();
        Some(list)
    }
}

/// The liquid drawn: its shapes blurred (`blur`: physical px), and made solid again where the blur is thick enough, so close ones
/// run together and every edge between solid and none is sharp.
pub fn draw_liquid(dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, list: &windows::Win32::Graphics::Direct2D::ID2D1CommandList, blur: f32) {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    unsafe {
        let Ok(blurfx) = dc.CreateEffect(&CLSID_D2D1GaussianBlur) else { return };
        let Ok(img) = list.cast::<ID2D1Image>() else { return };
        blurfx.SetInput(0, &img, true);
        let _ = blurfx.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &blur.to_le_bytes());
        let Ok(sharp) = dc.CreateEffect(&CLSID_D2D1ColorMatrix) else { return };
        let Ok(blurred) = blurfx.GetOutput() else { return };
        sharp.SetInput(0, &blurred, true);
        // (alpha 24 times over, less 10: thick blur solid, thin none, the edge between them sharp)
        let mut mx = [0f32; 20];
        mx[0] = 1.0;
        mx[5] = 1.0;
        mx[10] = 1.0;
        mx[15] = 24.0;
        mx[19] = -10.0;
        let bytes: Vec<u8> = mx.iter().flat_map(|v| v.to_le_bytes()).collect();
        let _ = sharp.SetValue(D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32, D2D1_PROPERTY_TYPE_MATRIX_5X4, &bytes);
        let _ = sharp.SetValue(D2D1_COLORMATRIX_PROP_CLAMP_OUTPUT.0 as u32, D2D1_PROPERTY_TYPE_BOOL, &1u32.to_le_bytes());
        if let Ok(out) = sharp.GetOutput() {
            dc.DrawImage(&out, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
        }
    }
}

/// let go with its body this near a neighbour's (logical px; standing apart, they are 26 apart): joined to it
const JOIN_REACH: f32 = 4.0;

/// how long a rolling number takes to roll to its next (seconds)
const ROLL_FOR: f32 = 0.5;

/// The layout's key for a banner standing in the Notifications widget's place.
const BANNER_KEY: &str = "\u{0}banner";

/// How tall a banner is (logical px): its line, its rows of bars, its buttons.
fn banner_h(say: &Value) -> f32 {
    let pill = &say["pill"];
    let rows = pill["rows"].as_array().map_or(0, |r| r.len());
    let has_buttons = pill["buttons"].as_array().is_some_and(|a| !a.is_empty());
    // (bare: its rows alone, no icon and no title)
    // (bare: its rows alone, as much room above them as below)
    if pill["bare"] == true && rows > 0 {
        return 40.0 + (rows - 1) as f32 * 18.0 + if has_buttons { 40.0 } else { 0.0 };
    }
    // (live: who, then two lines of what they say)
    if pill["live"] == true {
        return 78.0;
    }
    60.0 + rows as f32 * 18.0 + if has_buttons { 40.0 } else { 0.0 }
}

fn banner_ms(say: &Value) -> u64 {
    let pill = &say["pill"];
    let base = pill["ms"].as_u64().unwrap_or(4000).clamp(1500, 30000);
    if pill["buttons"].as_array().is_some_and(|a| !a.is_empty()) {
        base.max(10000)
    } else {
        base
    }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Draw with an opacity (a layer, only when it is not whole).
fn with_opacity(dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, _g: &Gfx, a: f32, f: impl FnOnce()) {
    use windows::Win32::Graphics::Direct2D::*;
    if a >= 0.999 {
        f();
        return;
    }
    if a <= 0.001 {
        return;
    }
    unsafe {
        let params = D2D1_LAYER_PARAMETERS1 {
            contentBounds: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
            geometricMask: std::mem::ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(),
            opacity: a,
            opacityBrush: std::mem::ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        dc.PushLayer(&params, None);
        f();
        dc.PopLayer();
    }
}

/// A point of a mouse message (client px), as logical px of the screen.
pub fn point_of(lp: LPARAM, s: f32) -> (f32, f32) {
    let x = (lp.0 & 0xffff) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
    (x / s, y / s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spring_overshoots_once_and_settles() {
        let mut s = Spring::at(0.0);
        s.to = 1.0;
        let (mut peak, mut frames) = (0f32, 0);
        while s.step(1.0 / 60.0) {
            peak = peak.max(s.now);
            frames += 1;
            assert!(frames < 120, "settles within two seconds");
        }
        assert!(peak > 1.02 && peak < 1.12, "a small overshoot: {peak}");
        assert_eq!((s.now, s.vel), (1.0, 0.0));
        // (the same at another frame rate)
        let mut t = Spring::at(0.0);
        t.to = 1.0;
        let mut peak2 = 0f32;
        while t.step(1.0 / 144.0) {
            peak2 = peak2.max(t.now);
        }
        assert!((peak - peak2).abs() < 0.01);
    }
}

/// The edges a place touches (a corner: two).
/// A plugin names a picture of its own folder "own:<file>": the frame makes that the file's whole path ("file:<path>"), so the picture
/// can be found. (A name that would leave the folder is dropped.)
fn own_pictures(v: &mut Value, plugin: &str, hosts: &[String]) {
    match v {
        Value::String(s) => {
            if let Some(f) = s.strip_prefix("own:") {
                let ok = !f.is_empty() && !f.contains(['/', '\\', ':']) && !f.contains("..");
                *s = match (ok, nadi_core::paths::plugins_dir()) {
                    (true, Some(d)) => format!("file:{}", d.join(plugin).join(f).display()),
                    _ => String::new(),
                };
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| own_pictures(x, plugin, hosts)),
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                // (a picture on the web, at a host the plugin may reach: the shell fetches it; another is none)
                if k == "image" {
                    if let Some(url) = x.as_str().filter(|u| u.starts_with("https://")) {
                        *x = if nadi_core::pluginrun::net_allows(hosts, url) { Value::String(format!("web:{url}")) } else { Value::String(String::new()) };
                        continue;
                    }
                }
                own_pictures(x, plugin, hosts);
            }
        }
        _ => {}
    }
}

/// How far the eyes' face is turned on a place: on the right edge its head is toward the right (a quarter turn clockwise), on the
/// left toward the left.
fn eyes_turn(place: Place) -> f32 {
    match place {
        Place::Edge(Edge::Right) => 90.0,
        Place::Edge(Edge::Left) => -90.0,
        _ => 0.0,
    }
}

/// A merged piece goes a little past its edge (off the screen, where nothing shows): its side on the edge then never ends on a part
/// of a pixel, which would leave a faint seam between it and the edge as it moves.
fn bleed(place: Place, b: Bx, s: f32) -> Bx {
    let e = 2.0 / s;
    let (top, bottom, left, right) = match place {
        Place::Edge(Edge::Top) => (true, false, false, false),
        Place::Edge(Edge::Bottom) => (false, true, false, false),
        Place::Edge(Edge::Left) => (false, false, true, false),
        Place::Edge(Edge::Right) => (false, false, false, true),
        Place::Corner(Corner::TopLeft) => (true, false, true, false),
        Place::Corner(Corner::TopRight) => (true, false, false, true),
        Place::Corner(Corner::BottomLeft) => (false, true, true, false),
        Place::Corner(Corner::BottomRight) => (false, true, false, true),
    };
    let mut b = b;
    if top {
        b.y -= e;
        b.h += e;
    }
    if bottom {
        b.h += e;
    }
    if left {
        b.x -= e;
        b.w += e;
    }
    if right {
        b.w += e;
    }
    b
}

/// How far a point is from a corner, along its row (or its column, `down`).
fn from_corner(c: Corner, down: bool, x: f32, y: f32, a: Bx) -> f32 {
    let left = matches!(c, Corner::TopLeft | Corner::BottomLeft);
    let top = matches!(c, Corner::TopLeft | Corner::TopRight);
    if down {
        if top { y - a.y } else { a.y + a.h - y }
    } else if left {
        x - a.x
    } else {
        a.x + a.w - x
    }
}

/// Where a widget stands in its place, for its order there (the lowest first). Along an edge: where along it. In a corner: the one
/// in the corner itself first, then by how far each is from the corner, along its row or its column.
fn order_in(place: Place, slot: Place, b: Bx, a: Bx) -> f32 {
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    match place {
        Place::Corner(_) if matches!(slot, Place::Corner(_)) => -500.0,
        Place::Corner(c) => from_corner(c, matches!(slot, Place::Edge(Edge::Left | Edge::Right)), cx, cy, a),
        p if p.horizontal() => cx,
        _ => cy,
    }
}

/// The order of the held widget where the mouse is (in a corner itself: before the one there).
fn held_order(spot: Spot, x: f32, y: f32, a: Bx) -> f32 {
    match spot.place {
        Place::Corner(_) if spot.head => -1000.0,
        Place::Corner(c) => from_corner(c, spot.down, x, y, a),
        p if p.horizontal() => x,
        _ => y,
    }
}

/// How a widget in a corner looks: the corner's own shape in the corner itself (`head`), else that of its row's edge, or of its
/// column's (`down`).
fn drag_look(place: Place, head: bool, down: bool) -> Place {
    match place {
        Place::Corner(c) if !head => {
            let left = matches!(c, Corner::TopLeft | Corner::BottomLeft);
            let top = matches!(c, Corner::TopLeft | Corner::TopRight);
            Place::Edge(if down {
                if left { Edge::Left } else { Edge::Right }
            } else if top {
                Edge::Top
            } else {
                Edge::Bottom
            })
        }
        p => p,
    }
}

fn edge_of(p: Place) -> Vec<Edge> {
    match p {
        Place::Edge(e) => vec![e],
        Place::Corner(Corner::TopLeft) => vec![Edge::Top, Edge::Left],
        Place::Corner(Corner::TopRight) => vec![Edge::Top, Edge::Right],
        Place::Corner(Corner::BottomLeft) => vec![Edge::Bottom, Edge::Left],
        Place::Corner(Corner::BottomRight) => vec![Edge::Bottom, Edge::Right],
    }
}


/// What is asked before something that cannot be undone is done: its pictogram, "Label?", a line of what happens, Cancel and the
/// thing itself, red.
fn confirm_spec(icon: &str, label: &str, text: &str, action: &str) -> Value {
    let label = if label.is_empty() { "Continue" } else { label };
    json!({ "icon": icon, "tone": "red", "title": format!("{label}?"), "text": text, "buttons": [{ "label": "Cancel" }, { "label": label, "action": action, "tone": "red" }] })
}
