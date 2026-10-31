//! Popped out: an open widget's panel, taken off its edge (pulled away from it, or Pop Out in its menu), floats as a small window
//! anywhere on the screen: a wiki beside a game. It is the same widget (its plugin hears the same presses); the widget keeps its
//! place on the edge with a dot under it. A grip over the window (while the mouse is on it) moves it, and a three-way switch beside it
//! picks its background: solid, see-through, or none. See-through and none let clicks through to what is under it until the mouse
//! rests on it, or the hold key is held. Its close (top right, while the mouse is on it), or its widget pressed while it is in front, sends it back. Where
//! it stood is kept for each widget and monitor.

use super::*;
use windows::core::Interface;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::DirectComposition::{IDCompositionVisual, IDCompositionVisual3};

/// the key of a widget's window among the pieces of a screen (never a widget's own: those have no NUL)
const POP: &str = "\u{0}pop\u{0}";
/// the strip along a window's top that holds its grip and its switch (shown while the mouse is on it)
pub(super) const POP_TOP: f32 = 24.0;
const TOGGLE: f32 = 22.0;
/// resting this long on a see-through window makes it take clicks
const REST: Duration = Duration::from_millis(350);
/// the grip stays this long after the mouse left the window
const LINGER: Duration = Duration::from_millis(500);
/// pulled this far off its edge, an open panel comes away
const TEAR: f32 = 36.0;

pub(super) fn is_pop(k: &str) -> bool {
    k.starts_with(POP)
}

/// The widget a piece is: a window's widget, else the key itself.
pub(super) fn unpop(k: &str) -> &str {
    k.strip_prefix(POP).unwrap_or(k)
}

pub(super) fn pop_key(k: &str) -> String {
    format!("{POP}{k}")
}

/// A window's own state (its piece is an item of its screen).
pub(super) struct PopMeta {
    pub si: usize,
    /// the mouse on it (or its grip)
    pub hovered: bool,
    /// it takes clicks (solid; or rested on, or the hold key held)
    pub live: bool,
    pub rest_since: Option<Instant>,
    /// when the mouse left it (and its grip): the grip stays a moment, to be reached
    pub left_at: Option<Instant>,
    /// how it moves now (coming out, going back)
    pub anim: Option<PopAnim>,
    /// how much its top strip (grip, switch, close) shows, 0..1
    pub strip_a: f32,
    /// how much its content shows, 0..1 (going back: it fades first; the frame's visuals cannot be made see-through, so it is
    /// drawn so)
    pub content_a: f32,
}

/// How a window moves as it comes out or goes back.
pub(super) enum PopAnim {
    /// coming out: it grows to its size about a point in it (logical), from a little or a lot, fading in
    Grow { k: Spring, pivot: (f32, f32), fade: f32 },
    /// going back: its words fade, then it falls to its stump as a drop the edge takes back in (`to`: the stump's middle on its
    /// screen; none, or no liquid: it shrinks and fades)
    Home { t0: Instant, from: Bx, to: Option<(f32, f32)> },
}

/// coming back: its words fade (seconds), then it falls (seconds)
const HOME_FADE: f32 = 0.1;
const HOME_FALL: f32 = 0.28;

/// A piece's visual moved and scaled (physical px), and how opaque (where the device can).
fn visual_set(v: &IDCompositionVisual, m: Matrix3x2, opacity: f32) {
    unsafe {
        let _ = v.SetTransform2(&m);
        if let Ok(v3) = v.cast::<IDCompositionVisual3>() {
            let _ = v3.SetOpacity2(opacity.clamp(0.0, 1.0));
        }
    }
}

/// Scaled by `k` about a point (physical px), then moved by (dx, dy).
fn scale_about(k: f32, px: f32, py: f32, dx: f32, dy: f32) -> Matrix3x2 {
    Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: px * (1.0 - k) + dx, M32: py * (1.0 - k) + dy }
}

/// The grip and the switch over the window the mouse is on.
pub(super) struct PopMarks {
    pub si: usize,
    pub key: String,
}

/// Where a window's grip, its switch's three, and its close are (in the window, `w` wide): the close in the top right corner, the
/// switch beside it, the grip all the rest of its top strip.
pub(super) fn marks_rects(w: f32) -> (Rect, [Rect; 3], Rect) {
    let close = Rect::new(w - 12.0 - TOGGLE, 2.0, TOGGLE, POP_TOP - 2.0);
    let tx = close.x - 8.0 - 3.0 * TOGGLE;
    let toggles = [0, 1, 2].map(|i| Rect::new(tx + i as f32 * TOGGLE, 2.0, TOGGLE, POP_TOP - 2.0));
    (Rect::new(0.0, 0.0, tx - 4.0, POP_TOP), toggles, close)
}

/// Draw a window's grip and switch (`now`: which background is on) in its top strip.
pub(super) fn paint_marks(p: &Painter, w: f32, now: usize) {
    let c = POP_TOP / 2.0 + 2.0;
    // (the grip: a short bar in the middle, to take hold of)
    p.round(Rect::new(w / 2.0 - 18.0, c - 2.0, 36.0, 4.0), 2.0, draw::DIM.alpha(0.8));
    let (_, toggles, close) = marks_rects(w);
    // (close: back into its widget)
    p.icon("x", close.x + close.w / 2.0 - 6.0, c - 6.0, 12.0, draw::TEXT.alpha(0.85));
    for (i, t) in toggles.iter().enumerate() {
        let cx = t.x + t.w / 2.0;
        if i == now {
            p.circle(cx, c, 9.0, draw::accent().alpha(0.85));
        }
        let r = Rect::new(cx - 5.0, c - 5.0, 10.0, 10.0);
        match i {
            0 => p.round(r, 3.0, draw::TEXT),
            1 => {
                p.round(r, 3.0, draw::TEXT.alpha(0.35));
                p.round(Rect::new(r.x, r.y, r.w / 2.0, r.h), 2.0, draw::TEXT);
            }
            _ => {
                // (none: its four corners alone)
                for (dx, dy) in [(0.0, 0.0), (6.5, 0.0), (0.0, 6.5), (6.5, 6.5)] {
                    p.round(Rect::new(r.x + dx, r.y + dy, 3.5, 3.5), 1.0, draw::TEXT);
                }
            }
        }
    }
}

pub(super) fn bg_index(bg: &str) -> usize {
    BGS.iter().position(|b| *b == bg).unwrap_or(0)
}

const BGS: [&str; 3] = ["solid", "see", "none"];

impl Frame {
    /// How a widget's window is backed: "solid", "see" (see-through) or "none".
    pub(super) fn pop_bg(&self, k: &str) -> &'static str {
        match self.config.widgets.get(unpop(k)).and_then(|c| c.pop_bg.as_deref()) {
            Some("see") => "see",
            Some("none") => "none",
            _ => "solid",
        }
    }

    pub(super) fn popped(&self, k: &str) -> bool {
        self.pops.contains_key(&pop_key(unpop(k)))
    }

    /// Pop a widget's panel out (`at`: where its window goes, logical, on screen `si`; none: where it was last time, else the middle).
    pub(super) fn pop_out(&mut self, si: usize, k: &str, at: Option<(f32, f32)>) {
        let k = unpop(k).to_string();
        let pk = pop_key(&k);
        if self.pops.contains_key(&pk) {
            self.pop_front(&pk);
            return;
        }
        if !self.widgets.get(&k).is_some_and(|w| w.content["panel"].is_object()) {
            return;
        }
        // (being typed into on its edge: the box put away, or it keeps the panel there open)
        if let Some(ib) = self.input.as_mut().filter(|i| i.key == k) {
            ib.close();
        }
        // (its panel on the edge shuts: the window is it now)
        for (s, sc) in self.screens.iter_mut().enumerate() {
            if let Some(it) = sc.items.get_mut(&k) {
                if it.open.to == 1.0 {
                    it.open.to = 0.0;
                    it.pinned = false;
                    let _ = s;
                }
            }
        }
        self.set_open(si, &k, false);
        // (its widget's middle, before it shrinks to its stump: from the menu, the window grows out of it)
        let from_widget = self.screens[si].items.get(&k).and_then(|it| self.shape_now(si, it).ok()).map(|(_, b)| (b.x + b.w / 2.0, b.y + b.h / 2.0));
        let pulled = at.is_some();
        let s = self.screens[si].s;
        let area = self.screens[si].area;
        let (pw, ph, ch) = self.panel_size(s, area, &pk, Place::Edge(Edge::Top));
        let mon = self.screens[si].ov.monitor.name();
        let kept = self.config.widgets.get(&k).and_then(|c| c.pop_at.as_ref()).and_then(|m| m.get(&mon).copied());
        let (x, y) = at.or(kept).unwrap_or((area.x + area.w / 2.0 - pw / 2.0, area.y + area.h / 2.0 - ph / 2.0));
        let b = clamp_in(Bx { x, y, w: pw, h: ph + POP_TOP }, area);
        let Ok(piece) = self.screens[si].ov.piece(&self.g, (pw * s * 1.15).ceil() as u32 + 8, ((ph + POP_TOP) * s * 1.15).ceil() as u32 + 8) else { return };
        self.screens[si].ov.to_front(&piece.visual);
        let slot = Slot { key: pk.clone(), place: Place::Edge(Edge::Top), x: b.x, y: b.y, w: b.w, h: b.h, along: b.w, dot: false };
        let rest = SpringBox { x: Spring::at(b.x), y: Spring::at(b.y), w: Spring::at(b.w), h: Spring::at(b.h) };
        self.screens[si].items.insert(
            pk.clone(),
            Item {
                piece,
                glow: None,
                slot,
                open: Spring::at(1.0),
                rest,
                content_h: ch,
                scroll: 0.0,
                hits: Vec::new(),
                buttons: Vec::new(),
                thumb_boxes: Vec::new(),
                thumbs: Vec::new(),
                sections_open: HashSet::new(),
                sections_closed: HashSet::new(),
                hover_since: None,
                left_at: None,
                alive: false,
                pinned: true,
                floating: true,
                visible: 0.0,
                scrolled_at: None,
                drag_place: None,
                drag_float: false,
                drag_down: false,
                drag_head: false,
                shown: Spring::at(1.0),
                unwanted_since: None,
                wanted_since: None,
            },
        );
        let solid = self.pop_bg(&k) == "solid";
        // (pulled off: it springs to its size from a little less, about its middle; from the menu: out of its widget)
        let anim = if pulled {
            PopAnim::Grow { k: Spring { now: 0.94, to: 1.0, vel: 0.0 }, pivot: (b.w / 2.0, b.h / 2.0), fade: 1.0 }
        } else {
            let (px, py) = from_widget.map_or((b.w / 2.0, b.h / 2.0), |(wx, wy)| (wx - b.x, wy - b.y));
            PopAnim::Grow { k: Spring { now: 0.06, to: 1.0, vel: 0.0 }, pivot: (px, py), fade: 0.0 }
        };
        self.pops.insert(pk.clone(), PopMeta { si, hovered: false, live: solid, rest_since: None, left_at: None, anim: Some(anim), strip_a: 0.0, content_a: 1.0 });
        self.apply_anim(&pk);
        self.pop_layer(&pk);
        self.pop_top = Some(pk.clone());
        self.set_open(si, &pk, true);
        // (its widget on the edge: drawn again, with its dot, its panel shut)
        self.relayout_with(Some(&HashSet::from([k.clone(), pk.clone()])));
    }

    /// A window goes back into its widget: its words fade, and it falls back to its stump as a drop.
    pub(super) fn pop_in(&mut self, pk: &str) {
        let Some(si) = self.pops.get(pk).filter(|m| !matches!(m.anim, Some(PopAnim::Home { .. }))).map(|m| m.si) else { return };
        let Some(from) = self.screens[si].items.get(pk).map(|it| it.rest.now()) else { return };
        let k = unpop(pk);
        let to = self.screens[si].items.get(k).and_then(|it| self.shape_now(si, it).ok()).map(|(_, b)| (b.x + b.w / 2.0, b.y + b.h / 2.0));
        let meta = self.pops.get_mut(pk).unwrap();
        meta.anim = Some(PopAnim::Home { t0: Instant::now(), from, to });
        // (going: it takes no clicks)
        meta.live = false;
        if self.pop_marks.as_ref().is_some_and(|m| m.key == pk) {
            self.pop_marks = None;
        }
        self.pop_layer(pk);
        self.hit_regions();
        self.tick_rate();
    }

    /// A window gone back at once.
    fn pop_in_now(&mut self, pk: &str) {
        self.pop_drops.retain(|(_, k, _, _)| k != pk);
        if let Some(si) = self.pop_remove(pk) {
            self.screens[si].liquid_dirty = true;
            let k = unpop(pk).to_string();
            self.relayout_with(Some(&HashSet::from([k])));
        }
    }

    /// One frame of each window's movement (coming out, going back) and of its top strip fading. True: something moves.
    pub(super) fn step_pop_anims(&mut self, dt: f32) -> bool {
        let mut moving = false;
        let mut done: Vec<String> = Vec::new();
        let keys: Vec<String> = self.pops.keys().cloned().collect();
        let liquid = self.liquid_px().is_some();
        for pk in keys {
            let Some(meta) = self.pops.get_mut(&pk) else { continue };
            let si = meta.si;
            // (its top strip: in or out over 120 ms)
            let want = if self.pop_marks.as_ref().is_some_and(|m| m.key == pk) { 1.0 } else { 0.0 };
            if (meta.strip_a - want).abs() > 0.001 {
                meta.strip_a = if want > meta.strip_a { (meta.strip_a + dt / 0.12).min(1.0) } else { (meta.strip_a - dt / 0.12).max(0.0) };
                moving = true;
                self.render(si, &pk);
            }
            let Some(meta) = self.pops.get_mut(&pk) else { continue };
            match &mut meta.anim {
                Some(PopAnim::Grow { k, fade, .. }) => {
                    // (a touch over, then it settles)
                    k.step_with(dt, 520.0, 24.0);
                    *fade = (*fade + dt / 0.1).min(1.0);
                    if !k.moving() && *fade >= 1.0 {
                        meta.anim = None;
                    }
                    moving = true;
                    self.apply_anim(&pk);
                }
                Some(PopAnim::Home { t0, from, to }) => {
                    let e = t0.elapsed().as_secs_f32();
                    let (from, to) = (*from, *to);
                    moving = true;
                    match to.filter(|_| liquid) {
                        Some((tx, ty)) => {
                            // (its words fade over it; under it, the same shape in the liquid, falling to its stump and taken in)
                            let u = ((e - HOME_FADE) / HOME_FALL).clamp(0.0, 1.0);
                            let q = u * u;
                            let end = 14.0;
                            let (cx, cy) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
                            let (w, h) = (from.w + (end - from.w) * q, from.h + (end - from.h) * q);
                            let (x, y) = (cx + (tx - cx) * q - w / 2.0, cy + (ty - cy) * q - h / 2.0);
                            let r = (RADIUS_OPEN + (end / 2.0 - RADIUS_OPEN) * q).min(w.min(h) / 2.0);
                            self.pop_drops.retain(|(_, k, _, _)| *k != pk);
                            self.pop_drops.push((si, pk.clone(), Bx { x, y, w, h }, r));
                            self.screens[si].liquid_dirty = true;
                            self.fade_content(si, &pk, 1.0 - e / HOME_FADE);
                            // (faded: the window's piece is put away, and the drop under it is all there is)
                            if e >= HOME_FADE {
                                if let Some(v) = self.screens[si].items.get(&pk).map(|it| it.piece.visual.clone()) {
                                    visual_set(&v, scale_about(0.0001, 0.0, 0.0, 0.0, 0.0), 0.0);
                                }
                            }
                            if e >= HOME_FADE + HOME_FALL {
                                done.push(pk.clone());
                            }
                        }
                        None => {
                            // (no liquid, or its stump elsewhere: it shrinks toward its stump and fades)
                            let u = (e / 0.26).clamp(0.0, 1.0);
                            let q = u * u;
                            self.fade_content(si, &pk, 1.0 - e / HOME_FADE);
                            let s = self.screens[si].s;
                            let (cx, cy) = (from.w / 2.0, from.h / 2.0);
                            let (dx, dy) = to.map_or((0.0, 0.0), |(tx, ty)| ((tx - from.x - cx) * q, (ty - from.y - cy) * q));
                            if let Some(v) = self.screens[si].items.get(&pk).map(|it| it.piece.visual.clone()) {
                                visual_set(&v, scale_about(1.0 - 0.9 * q, cx * s, cy * s, dx * s, dy * s), 1.0 - u);
                            }
                            if u >= 1.0 {
                                done.push(pk.clone());
                            }
                        }
                    }
                }
                None => {}
            }
        }
        for pk in done {
            self.pop_in_now(&pk);
        }
        // (the drops falling: the frame's window shows them where they are)
        if !self.pop_drops.is_empty() || moving {
            self.hit_regions();
        }
        if moving {
            self.commit();
        }
        moving
    }

    /// A window's content drawn as faded as `a`.
    fn fade_content(&mut self, si: usize, pk: &str, a: f32) {
        let a = a.clamp(0.0, 1.0);
        let Some(meta) = self.pops.get_mut(pk) else { return };
        if (meta.content_a - a).abs() > 0.02 || (a == 0.0 && meta.content_a > 0.0) {
            meta.content_a = a;
            self.render(si, pk);
        }
    }

    /// A window's visual as it is in its coming out.
    fn apply_anim(&self, pk: &str) {
        let Some(meta) = self.pops.get(pk) else { return };
        let Some(v) = self.screens[meta.si].items.get(pk).map(|it| it.piece.visual.clone()) else { return };
        let s = self.screens[meta.si].s;
        match &meta.anim {
            Some(PopAnim::Grow { k, pivot, fade }) => visual_set(&v, scale_about(k.now.max(0.01), pivot.0 * s, pivot.1 * s, 0.0, 0.0), *fade),
            None => visual_set(&v, Matrix3x2::identity(), 1.0),
            _ => {}
        }
    }

    /// Pulling an open panel off its edge, not far enough yet: it follows the mouse a little (`off`, logical, any way), a drop on a
    /// neck of liquid from where it was; (0, 0): as it was.
    fn tear_stretch(&mut self, si: usize, k: &str, off: (f32, f32)) {
        let (ox, oy) = off;
        let d = ox.hypot(oy);
        let was = self.tear_pull;
        self.tear_pull = d;
        self.tear_off = off;
        let Some(it) = self.screens[si].items.get(k) else { return };
        let v = it.piece.visual.clone();
        let place = it.slot.place;
        // (pulled: it is a drop of its own, round all round; let go: as it was)
        if (d > 0.5) != (was > 0.5) {
            self.render(si, k);
        }
        let Some(it) = self.screens[si].items.get(k) else { return };
        let fb = tear_box(place, it.rest.now());
        let s = self.screens[si].s;
        visual_set(&v, scale_about(1.0, 0.0, 0.0, ox * s, oy * s), 1.0);
        self.tear_neck = (d > 0.5).then(|| {
            // (where it meets its edge, the middle of that side: a small drop stays there; one between; and the panel's own)
            let (ax, ay) = match edge_of(place).first() {
                Some(Edge::Top) => (fb.x + fb.w / 2.0, fb.y),
                Some(Edge::Left) => (fb.x, fb.y + fb.h / 2.0),
                Some(Edge::Right) => (fb.x + fb.w, fb.y + fb.h / 2.0),
                _ => (fb.x + fb.w / 2.0, fb.y + fb.h),
            };
            let at = |k: f32, cx: f32, cy: f32| {
                let (w, h) = (fb.w * k, fb.h * k);
                (Bx { x: cx - w / 2.0, y: cy - h / 2.0, w, h }, (RADIUS_OPEN * k).min(w.min(h) / 2.0))
            };
            let (pcx, pcy) = (fb.x + fb.w / 2.0 + ox, fb.y + fb.h / 2.0 + oy);
            let (mcx, mcy) = ((ax + pcx) / 2.0, (ay + pcy) / 2.0);
            let thin = (1.0 - d / TEAR).clamp(0.25, 1.0);
            let anchor = at(0.42 * thin, ax, ay);
            let mid = at(0.5 * thin, mcx, mcy);
            let panel = (Bx { x: fb.x + ox, y: fb.y + oy, w: fb.w, h: fb.h }, RADIUS_OPEN);
            (si, vec![anchor, mid, panel])
        });
        self.screens[si].liquid_dirty = true;
        // (its handle narrows as it is pulled)
        self.render(si, k);
        self.hit_regions();
        self.commit();
    }

    /// A window taken away (its screen, or none: there was none), the layout left as it is.
    fn pop_remove(&mut self, pk: &str) -> Option<usize> {
        let meta = self.pops.remove(pk)?;
        let si = meta.si;
        if let Some(it) = self.screens.get_mut(si).and_then(|sc| sc.items.remove(pk)) {
            let _ = self.screens[si].ov.remove(&it.piece);
            // (the live pictures of windows in it (the dock's) go with it)
            for (_, th) in &it.thumbs {
                crate::thumbs::unregister(*th);
            }
        }
        if self.pop_marks.as_ref().is_some_and(|m| m.key == pk) {
            self.drop_marks();
        }
        if self.pop_top.as_deref() == Some(pk) {
            self.pop_top = None;
        }
        if self.hover.as_ref().is_some_and(|(_, h)| h == pk) {
            self.hover = None;
        }
        if self.pop_drag.as_ref().is_some_and(|(_, k, ..)| k == pk) {
            self.pop_drag = None;
        }
        self.set_open(si, unpop(pk), false);
        Some(si)
    }

    /// A window over the others.
    pub(super) fn pop_front(&mut self, pk: &str) {
        if !self.pops.contains_key(pk) {
            return;
        }
        self.pop_top = Some(pk.to_string());
        self.pop_layer(pk);
        self.commit();
    }

    /// Each window as the layout runs: as big as its panel now (its spot kept, inside the screen), drawn again when its widget changed.
    pub(super) fn relayout_pops(&mut self, si: usize, dirty: Option<&HashSet<String>>) {
        let keys: Vec<String> = self.pops.iter().filter(|(_, m)| m.si == si).map(|(k, _)| k.clone()).collect();
        for pk in keys {
            let k = unpop(&pk).to_string();
            // (its widget gone, or its panel: it goes back)
            if !self.widgets.get(&k).is_some_and(|w| w.content["panel"].is_object()) {
                self.pop_remove(&pk);
                if self.screens[si].items.contains_key(&k) {
                    self.render(si, &k);
                }
                continue;
            }
            let s = self.screens[si].s;
            let area = self.screens[si].area;
            let (pw, ph, ch) = self.panel_size(s, area, &pk, Place::Edge(Edge::Top));
            let Some(it) = self.screens[si].items.get_mut(&pk) else { continue };
            let b = clamp_in(Bx { x: it.slot.x, y: it.slot.y, w: pw, h: ph + POP_TOP }, area);
            let changed = (it.slot.w - b.w).abs() > 0.5 || (it.slot.h - b.h).abs() > 0.5 || (it.slot.x - b.x).abs() > 0.5 || (it.slot.y - b.y).abs() > 0.5;
            it.slot.x = b.x;
            it.slot.y = b.y;
            it.slot.w = b.w;
            it.slot.h = b.h;
            it.slot.along = b.w;
            it.rest = SpringBox { x: Spring::at(b.x), y: Spring::at(b.y), w: Spring::at(b.w), h: Spring::at(b.h) };
            it.content_h = ch;
            it.open = Spring::at(1.0);
            it.shown = Spring::at(1.0);
            if changed || dirty.map_or(true, |d| d.contains(&pk) || d.contains(&k)) {
                self.render(si, &pk);
            }
        }
    }

    /// The widget pressed on its edge while its window is out: the window to the front; already in front, back it goes.
    pub(super) fn pop_widget_pressed(&mut self, k: &str) -> bool {
        let pk = pop_key(unpop(k));
        if !self.pops.contains_key(&pk) {
            return false;
        }
        if self.pop_top.as_deref() == Some(pk.as_str()) {
            self.pop_in(&pk);
        } else {
            self.pop_front(&pk);
        }
        true
    }

    /// The left button down on a window's grip or switch: the grip picks it up; the switch picks the next background. True: taken.
    pub(super) fn pop_press(&mut self, si: usize, x: f32, y: f32) -> bool {
        // (on a window: it comes to the front)
        if let Some(k) = self.item_at(si, x, y).filter(|k| is_pop(k)) {
            if self.pop_top.as_deref() != Some(k.as_str()) {
                self.pop_front(&k);
            }
        }
        let Some(pk) = self.pop_marks.as_ref().filter(|m| m.si == si).map(|m| m.key.clone()) else { return false };
        let Some(wb) = self.screens[si].items.get(&pk).map(|it| it.rest.now()) else { return false };
        let (lx, ly) = (x - wb.x, y - wb.y);
        let inside = |r: &Rect| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h;
        let (grip, toggles, close) = marks_rects(wb.w);
        if !inside(&Rect::new(0.0, 0.0, wb.w, POP_TOP)) {
            return false;
        }
        if inside(&close) {
            self.pop_in(&pk);
            return true;
        }
        if inside(&grip) {
            let Some(b) = self.screens[si].items.get(&pk).map(|it| it.rest.now()) else { return true };
            self.pop_drag = Some((si, pk, x - b.x, y - b.y));
            unsafe {
                let _ = SetCapture(self.screens[si].ov.hwnd);
            }
            self.tick_rate();
            return true;
        }
        if let Some(i) = toggles.iter().position(|t| inside(t)) {
            let k = unpop(&pk).to_string();
            let bg = BGS[i];
            self.change_config(|c| c.widgets.entry(k.clone()).or_default().pop_bg = if bg == "solid" { None } else { Some(bg.into()) });
            if let Some(meta) = self.pops.get_mut(&pk) {
                meta.live = true;
            }
            self.pop_layer(&pk);
            self.render(si, &pk);
            self.hit_regions();
            self.commit();
        }
        true
    }

    /// An open panel pressed where nothing in it is pressed: it may be pulled off its edge.
    pub(super) fn tear_press(&mut self, si: usize, x: f32, y: f32) {
        self.tear = None;
        let Some(k) = self.item_at(si, x, y).filter(|k| !is_pop(k)) else { return };
        let Some(it) = self.screens[si].items.get(&k) else { return };
        if it.open.now < 0.9 || it.drag_place.is_some() || self.edit.is_some() {
            return;
        }
        let Ok((_, b)) = self.shape_now(si, it) else { return };
        let (lx, ly) = (x - b.x, y - b.y);
        if it.hits.iter().any(|h| lx >= h.rect.x && lx < h.rect.x + h.rect.w && ly >= h.rect.y && ly < h.rect.y + h.rect.h) {
            return;
        }
        self.tear = Some((si, k, x, y));
        // (the mouse is held here: pulled away, it leaves the panel at once, and is still heard)
        unsafe {
            let _ = SetCapture(self.screens[si].ov.hwnd);
        }
    }

    /// The mouse moved: a window held by its grip follows; a panel pulled far enough off its edge comes away. True: taken.
    pub(super) fn pop_move(&mut self, si: usize, x: f32, y: f32) -> bool {
        if let Some((ds, pk, gx, gy)) = self.pop_drag.clone() {
            // (where the mouse is on the desktop: on another monitor, the window goes over to it)
            let (mx, my) = {
                let sc = &self.screens[si];
                let m = sc.ov.monitor.rect;
                (m.left as f32 + x * sc.s, m.top as f32 + y * sc.s)
            };
            let to = self
                .screens
                .iter()
                .position(|sc| {
                    let m = sc.ov.monitor.rect;
                    mx >= m.left as f32 && mx < m.right as f32 && my >= m.top as f32 && my < m.bottom as f32
                })
                .unwrap_or(ds);
            if to != ds {
                self.pop_move_to(&pk, ds, to);
            }
            let si = to;
            let (x, y) = {
                let sc = &self.screens[si];
                let m = sc.ov.monitor.rect;
                ((mx - m.left as f32) / sc.s, (my - m.top as f32) / sc.s)
            };
            let area = self.screens[si].area;
            let s = self.screens[si].s;
            let mut moved = None;
            if let Some(it) = self.screens[si].items.get_mut(&pk) {
                let b = clamp_in(Bx { x: x - gx, y: y - gy, w: it.slot.w, h: it.slot.h }, area);
                it.slot.x = b.x;
                it.slot.y = b.y;
                it.rest = SpringBox { x: Spring::at(b.x), y: Spring::at(b.y), w: Spring::at(b.w), h: Spring::at(b.h) };
                let _ = it.piece.place((b.x * s).round() as i32, (b.y * s).round() as i32);
                moved = Some(b);
            }
            // (live pictures of windows in it (the dock's) go with it)
            if let Some(b) = moved {
                self.place_thumbs(si, &pk, b, 1.0);
            }
            self.hit_regions();
            self.commit();
            return true;
        }
        let Some((ts, k, x0, y0)) = self.tear.clone() else { return false };
        if ts != si {
            return false;
        }
        let Some(it) = self.screens[si].items.get(&k) else {
            self.tear = None;
            return false;
        };
        // (pulled any way)
        let (dx, dy) = (x - x0, y - y0);
        let away = dx.hypot(dy);
        let _ = it;
        if away < TEAR {
            // (not far enough yet: it stretches after the mouse)
            let off = (dx * 0.55, dy * 0.55);
            if (off.0 - self.tear_off.0).abs() + (off.1 - self.tear_off.1).abs() > 0.5 {
                self.tear_stretch(si, &k, off);
            }
            // (being pulled: no press to pick it up with)
            if away > 4.0 {
                self.press = None;
                return true;
            }
            return false;
        }
        let Ok((_, b)) = self.shape_now(si, it) else { return false };
        self.tear = None;
        self.press = None;
        // (it snaps off: the panel back as it was, and shut; the window springs out where it was pulled to)
        self.tear_stretch(si, &k, (0.0, 0.0));
        let (nx, ny) = (b.x + (x - x0), b.y + (y - y0));
        self.pop_out(si, &k, Some((nx, ny)));
        let pk = pop_key(&k);
        if let Some(nb) = self.screens[si].items.get(&pk).map(|it| it.rest.now()) {
            self.pop_drag = Some((si, pk, x - nb.x, y - nb.y));
            unsafe {
                let _ = SetCapture(self.screens[si].ov.hwnd);
            }
        }
        self.tick_rate();
        true
    }

    /// The button let go: a window held is put down (near its widget's edge: back it goes; else it stays there, and is kept there).
    /// True: taken.
    pub(super) fn pop_release(&mut self) -> bool {
        if let Some((ts, tk, ..)) = self.tear.take() {
            unsafe {
                let _ = ReleaseCapture();
            }
            if self.tear_pull > 0.0 {
                self.tear_stretch(ts, &tk, (0.0, 0.0));
            }
        }
        let Some((si, pk, ..)) = self.pop_drag.take() else { return false };
        unsafe {
            let _ = ReleaseCapture();
        }
        let Some(b) = self.screens[si].items.get(&pk).map(|it| it.rest.now()) else { return true };
        let k = unpop(&pk).to_string();
        let mon = self.screens[si].ov.monitor.name();
        self.change_config(|c| {
            c.widgets.entry(k.clone()).or_default().pop_at.get_or_insert_with(BTreeMap::new).insert(mon.clone(), (b.x, b.y));
        });
        self.tick_rate();
        true
    }

    /// As the frame ticks: which window the mouse is on (its grip shows), and which take clicks (see-through ones: rested on, or the
    /// hold key held).
    pub(super) fn step_pops(&mut self, dt: f32) {
        if self.pops.is_empty() {
            return;
        }
        self.step_pop_anims(dt);
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let key_held = if self.keys_hidden {
            crate::winlist::cursor_showing()
        } else {
            unsafe {
                match self.config.hold_key.as_str() {
                    "alt" => GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0,
                    "ctrl" => GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0,
                    _ => false,
                }
            }
        };
        let mut regions = false;
        let mut marks: Option<(usize, String)> = self.pop_marks.as_ref().map(|m| (m.si, m.key.clone()));
        // (the one on top first: the mouse is on that one)
        let mut keys: Vec<String> = self.pops.keys().cloned().collect();
        if let Some(top) = &self.pop_top {
            keys.sort_by_key(|k| k != top);
        }
        let mut on: Option<String> = None;
        for pk in &keys {
            let Some(si) = self.pops.get(pk).map(|m| m.si) else { continue };
            let sc = &self.screens[si];
            let m = sc.ov.monitor.rect;
            let (x, y) = ((pt.x - m.left) as f32 / sc.s, (pt.y - m.top) as f32 / sc.s);
            let Some(b) = sc.items.get(pk).map(|it| it.rest.now()) else { continue };
            let inside = on.is_none() && b.contains(x, y);
            let solid = self.pop_bg(pk) == "solid";
            let dragging = self.pop_drag.as_ref().is_some_and(|(_, k, ..)| k == pk);
            let meta = self.pops.get_mut(pk).unwrap();
            if inside {
                meta.left_at = None;
            } else if meta.hovered && meta.left_at.is_none() {
                meta.left_at = Some(Instant::now());
            }
            // (left a moment ago: still as if on it)
            let lingering = meta.left_at.is_some_and(|t| t.elapsed() < LINGER);
            if inside || (on.is_none() && lingering && meta.hovered) {
                on = Some(pk.clone());
            }
            meta.hovered = inside || dragging || lingering;
            if inside {
                meta.rest_since.get_or_insert_with(Instant::now);
            } else if !lingering {
                meta.rest_since = None;
            }
            let going = matches!(meta.anim, Some(PopAnim::Home { .. }));
            let live = !going && (solid || dragging || key_held || meta.rest_since.is_some_and(|t| t.elapsed() >= REST));
            if live != meta.live {
                meta.live = live;
                regions = true;
                self.pop_layer(pk);
            }
        }
        // (the grip and the switch: over the window the mouse is on, while it takes clicks)
        let want = on.filter(|pk| self.pops.get(pk).is_some_and(|m| m.live)).map(|pk| (self.pops[&pk].si, pk));
        let want = want.or_else(|| self.pop_drag.as_ref().map(|(si, k, ..)| (*si, k.clone())));
        if want != marks {
            marks = want;
            match &marks {
                Some((si, pk)) => self.show_marks(*si, pk),
                None => self.drop_marks(),
            }
            regions = true;
        }
        if regions {
            self.hit_regions();
            self.commit();
        }
    }

    fn drop_marks(&mut self) {
        self.pop_marks = None;
    }

    fn show_marks(&mut self, si: usize, pk: &str) {
        self.pop_marks = Some(PopMarks { si, key: pk.to_string() });
    }

    /// A window on top, where it is seen whether it takes clicks or not: on the frame's window while it does, on the glass over it
    /// while the mouse goes through it.
    fn pop_layer(&mut self, pk: &str) {
        let Some(m) = self.pops.get(pk) else { return };
        let (si, live) = (m.si, m.live);
        let Some(v) = self.screens[si].items.get(pk).map(|it| it.piece.visual.clone()) else { return };
        if live {
            self.screens[si].ov.to_front(&v);
        } else {
            self.screens[si].ov.to_glass_front(&v);
        }
    }

    /// A window taken over to another monitor's frame, as it is dragged there.
    fn pop_move_to(&mut self, pk: &str, from: usize, to: usize) {
        let Some(mut it) = self.screens[from].items.remove(pk) else { return };
        let _ = self.screens[from].ov.remove(&it.piece);
        for (_, th) in &it.thumbs {
            crate::thumbs::unregister(*th);
        }
        it.thumbs.clear();
        self.screens[to].ov.adopt(&it.piece.visual);
        self.screens[to].items.insert(pk.to_string(), it);
        if let Some(m) = self.pops.get_mut(pk) {
            m.si = to;
        }
        if let Some(m) = self.pop_marks.as_mut().filter(|m| m.key == pk) {
            m.si = to;
        }
        if let Some(d) = self.pop_drag.as_mut().filter(|d| d.1 == pk) {
            d.0 = to;
        }
        if self.hover.as_ref().is_some_and(|(_, h)| h == pk) {
            self.hover = None;
        }
        self.pop_layer(pk);
        // (as big as its panel is on that monitor, drawn at its scale)
        self.relayout_pops(to, Some(&HashSet::from([pk.to_string()])));
        self.hit_regions();
    }

    /// Is a window taking clicks (its box in the frame's window's region)?
    pub(super) fn pop_live(&self, pk: &str) -> bool {
        self.pops.get(pk).map_or(true, |m| m.live)
    }
}

/// A box kept inside an area (as much of it as fits).
fn clamp_in(b: Bx, a: Bx) -> Bx {
    let w = b.w.min(a.w);
    let h = b.h.min(a.h);
    Bx { x: b.x.clamp(a.x, a.x + a.w - w), y: b.y.clamp(a.y, a.y + a.h - h), w, h }
}

/// An open panel's box as a drop of its own (being pulled off its edge): its box less its shoulders along its edge.
pub(super) fn tear_box(place: Place, b: Bx) -> Bx {
    let f = Metrics::default().fillet;
    match place {
        Place::Edge(Edge::Top | Edge::Bottom) => Bx { x: b.x + f, w: (b.w - 2.0 * f).max(8.0), ..b },
        Place::Edge(Edge::Left | Edge::Right) => Bx { y: b.y + f, h: (b.h - 2.0 * f).max(8.0), ..b },
        Place::Corner(_) => b,
    }
}
