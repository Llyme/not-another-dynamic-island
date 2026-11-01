//! Panels made bigger or smaller by hand: near a free side of an open panel (or a popped-out window), the cursor becomes a resize
//! arrow and that side lights faintly; dragged, the panel follows (on its edge, evenly about its widget). Near the size the panel asks
//! for, it catches onto it; a dashed outline shows that size while dragging, and the size by the cursor. A double-click on a side
//! gives it back that size. The size is kept for each widget, on its edge and as a window apart. A window being moved snaps to the
//! screen's edges and middle lines and to the other windows, a thin line showing each snap (Shift: free).

use super::*;

/// how near a free side the mouse must be to take it (logical px, inside the panel)
const BAND: f32 = 6.0;
/// how near its own size a panel catches onto it, and a window onto a line
const CATCH: f32 = 12.0;
const SNAP: f32 = 10.0;
/// a window snapped to the screen's edge stands this far in from it, and this far from another window
const GAP: f32 = 12.0;
/// the smallest a panel may be made
pub(super) const MIN_W: f32 = 200.0;
pub(super) const MIN_H: f32 = 120.0;

pub(super) const LEFT: u8 = 1;
pub(super) const RIGHT: u8 = 2;
pub(super) const TOP: u8 = 4;
pub(super) const BOTTOM: u8 = 8;

/// A panel being resized: its screen and key, the sides held, where the mouse was, its box then (its panel's, less a window's strip),
/// the size it asks for, and its size now.
pub(super) struct Sizing {
    pub si: usize,
    pub key: String,
    pub sides: u8,
    from: (f32, f32),
    start: Bx,
    pref: (f32, f32),
    pub live: (f32, f32),
    /// a window's top left now (its left or top side held moves it)
    at: (f32, f32),
    /// its proportions: the height that goes with a width, `a + b × w` (a picture in it: the height its picture fills exactly)
    aspect: (f32, f32),
    /// caught onto its proportions now
    pub held_aspect: bool,
}

impl Frame {
    /// The panel's own body on the screen (logical): an open panel's, less its fillets; a window's, less its strip.
    fn panel_body(&self, si: usize, k: &str) -> Option<Bx> {
        let it = self.screens.get(si)?.items.get(k)?;
        let (shape, b) = self.shape_now(si, it).ok()?;
        let s = self.screens[si].s;
        let bd = shape.body;
        let body = Bx { x: b.x + bd.x / s, y: b.y + bd.y / s, w: bd.w / s, h: bd.h / s };
        if is_pop(k) {
            return Some(Bx { y: body.y + pop::POP_TOP, h: (body.h - pop::POP_TOP).max(1.0), ..body });
        }
        Some(body)
    }

    /// The sides of a panel that may be dragged: those away from its edge (or corner); a window's, all.
    fn free_sides(&self, si: usize, k: &str) -> u8 {
        if is_pop(k) {
            return LEFT | RIGHT | TOP | BOTTOM;
        }
        let Some(it) = self.screens.get(si).and_then(|sc| sc.items.get(k)) else { return 0 };
        match it.slot.place {
            Place::Edge(Edge::Top) => LEFT | RIGHT | BOTTOM,
            Place::Edge(Edge::Bottom) => LEFT | RIGHT | TOP,
            Place::Edge(Edge::Left) => TOP | BOTTOM | RIGHT,
            Place::Edge(Edge::Right) => TOP | BOTTOM | LEFT,
            Place::Corner(Corner::TopLeft) => RIGHT | BOTTOM,
            Place::Corner(Corner::TopRight) => LEFT | BOTTOM,
            Place::Corner(Corner::BottomLeft) => RIGHT | TOP,
            Place::Corner(Corner::BottomRight) => LEFT | TOP,
        }
    }

    /// The panel and the sides of it the mouse is near, if any (an open panel, settled; a window).
    fn sides_at(&self, si: usize, x: f32, y: f32) -> Option<(String, u8)> {
        if self.edit.is_some() || self.drag.is_some() {
            return None;
        }
        let sc = self.screens.get(si)?;
        // (a window on top first)
        let mut keys: Vec<&String> = sc.items.keys().collect();
        if let Some(top) = &self.pop_top {
            keys.sort_by_key(|k| *k != top);
        }
        for k in keys {
            let it = &sc.items[k];
            let open = if is_pop(k) { self.pop_live(k) } else { it.open.now > 0.95 && it.open.to == 1.0 && !self.popped(k) };
            if !open {
                continue;
            }
            let Some(b) = self.panel_body(si, k) else { continue };
            // (a window: its strip on top is its own)
            let top = if is_pop(k) { b.y - pop::POP_TOP } else { b.y };
            if x < b.x - 1.0 || x > b.x + b.w + 1.0 || y < top - 1.0 || y > b.y + b.h + 1.0 {
                continue;
            }
            let free = self.free_sides(si, k);
            let mut sides = 0;
            if x <= b.x + BAND {
                sides |= LEFT;
            }
            if x >= b.x + b.w - BAND {
                sides |= RIGHT;
            }
            if y <= top + BAND.min(4.0) {
                sides |= TOP;
            }
            if y >= b.y + b.h - BAND {
                sides |= BOTTOM;
            }
            let sides = sides & free;
            if sides != 0 {
                return Some((k.clone(), sides));
            }
            // (inside it: no other panel under it is looked at)
            return None;
        }
        None
    }

    /// The resize arrow for the sides the mouse is near (or holds), if any.
    pub(crate) fn sizing_cursor(&self) -> Option<windows::core::PCWSTR> {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let sides = self.sizing.as_ref().map(|z| z.sides).or(self.size_hover.as_ref().map(|h| h.2))?;
        let h = sides & (LEFT | RIGHT) != 0;
        let v = sides & (TOP | BOTTOM) != 0;
        Some(if h && v {
            if sides == LEFT | TOP || sides == RIGHT | BOTTOM { IDC_SIZENWSE } else { IDC_SIZENESW }
        } else if h {
            IDC_SIZEWE
        } else {
            IDC_SIZENS
        })
    }

    /// The mouse moved: the side under it lights (or goes out), a panel being resized follows it. True: taken (resizing).
    pub(super) fn size_move(&mut self, si: usize, x: f32, y: f32) -> bool {
        if self.sizing.is_some() {
            self.size_drag(si, x, y);
            return true;
        }
        let now = self.sides_at(si, x, y).map(|(k, s)| (si, k, s));
        if now != self.size_hover {
            let was = self.size_hover.take();
            self.size_hover = now.clone();
            for (s, k, _) in [was, now].into_iter().flatten() {
                if self.screens.get(s).is_some_and(|sc| sc.items.contains_key(&k)) {
                    self.render(s, &k);
                }
            }
            self.set_cursor();
            self.commit();
        }
        false
    }

    fn set_cursor(&self) {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let c = self.sizing_cursor().unwrap_or(IDC_ARROW);
        unsafe {
            if let Ok(h) = LoadCursorW(None, c) {
                SetCursor(h);
            }
        }
    }

    /// The left button down: on a free side, resizing starts (twice in a moment: the size it asks for again). True: taken.
    pub(super) fn size_press(&mut self, si: usize, x: f32, y: f32) -> bool {
        let Some((k, sides)) = self.sides_at(si, x, y) else { return false };
        if self.size_click.as_ref().is_some_and(|(c, t)| *c == k && t.elapsed() < Duration::from_millis(400)) {
            self.size_click = None;
            self.size_reset(si, &k);
            return true;
        }
        self.size_click = Some((k.clone(), Instant::now()));
        self.mouse = (si, x, y);
        let Some(body) = self.panel_body(si, &k) else { return false };
        let (s, area) = (self.screens[si].s, self.screens[si].area);
        let pref = self.panel_pref(s, area, &k);
        let at = self.screens[si].items.get(&k).map_or((body.x, body.y), |it| (it.slot.x, it.slot.y));
        let aspect = self.aspect_of(&k, pref);
        self.sizing = Some(Sizing { si, key: k.clone(), sides, from: (x, y), start: body, pref, live: (body.w, body.h), at, aspect, held_aspect: false });
        if let Some(it) = self.screens[si].items.get_mut(&k) {
            it.left_at = None;
        }
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.screens[si].ov.hwnd);
        }
        self.press = None;
        self.set_cursor();
        self.draw_guides();
        self.tick_rate();
        true
    }

    /// The size a panel asks for (its width and height; a window's less its strip).
    pub(super) fn panel_pref(&self, s: f32, area: Bx, k: &str) -> (f32, f32) {
        let place = if is_pop(k) { Place::Edge(Edge::Top) } else { self.screens.iter().find_map(|sc| sc.items.get(k)).map_or(Place::Edge(Edge::Top), |it| it.slot.place) };
        let (along, across, _) = self.panel_size_as(s, area, k, place, false);
        if matches!(place, Place::Edge(Edge::Left | Edge::Right)) {
            (across, along)
        } else {
            (along, across)
        }
    }

    /// A panel's proportions, as the height that goes with a width (`a + b × w`): with a picture to look at in it, the height at which
    /// the picture fills it exactly, no bars beside it or over it; else the proportions of the size it asks for.
    fn aspect_of(&self, k: &str, pref: (f32, f32)) -> (f32, f32) {
        let plain = (0.0, if pref.0 > 0.0 { pref.1 / pref.0 } else { 0.0 });
        let Some(w) = self.widgets.get(unpop(k)) else { return plain };
        let Some(view) = w.content["panel"]["body"].as_array().and_then(|b| b.iter().find(|x| x["kind"] == "view")) else { return plain };
        let img = view["image"].as_str().unwrap_or("");
        let Some((iw, ih)) = self.painter(1.0).picture_size(img) else { return plain };
        let b = ih / iw;
        let max_h = view["max_h"].as_f64().map_or(420.0, |m| m as f32);
        // (what else is in it, at the size it asks for: all but its picture)
        let view_h = ((pref.0 - 2.0 * PAD) * b).clamp(120.0, max_h);
        let chrome = pref.1 - view_h;
        (chrome - 2.0 * PAD * b, b)
    }

    /// The size kept for a panel by hand, or being dragged to now.
    pub(super) fn size_kept(&self, k: &str) -> Option<(f32, f32)> {
        if let Some(z) = self.sizing.as_ref().filter(|z| z.key == k) {
            return Some(z.live);
        }
        let c = self.config.widgets.get(unpop(k))?;
        if is_pop(k) {
            c.pop_size
        } else {
            c.panel_size
        }
    }

    /// The largest a panel may be made, on its screen (a window; on a side edge, never past the screen's middle).
    pub(super) fn size_max(area: Bx, k: &str, place: Place) -> (f32, f32) {
        if is_pop(k) {
            return (area.w * 0.9, area.h * 0.9 - pop::POP_TOP);
        }
        match place {
            Place::Edge(Edge::Left | Edge::Right) => (area.w * 0.5, area.h * 0.9),
            _ => (area.w * 0.9, area.h * 0.78),
        }
    }

    fn size_drag(&mut self, si: usize, x: f32, y: f32) {
        self.mouse = (si, x, y);
        let Some(z) = self.sizing.as_ref() else { return };
        if z.si != si {
            return;
        }
        let k = z.key.clone();
        let (dx, dy) = (x - z.from.0, y - z.from.1);
        let place = self.screens[si].items.get(&k).map_or(Place::Edge(Edge::Top), |it| it.slot.place);
        let pop = is_pop(&k);
        // (on an edge, it grows evenly about its widget along the edge)
        let even_x = !pop && matches!(place, Place::Edge(Edge::Top | Edge::Bottom));
        let even_y = !pop && matches!(place, Place::Edge(Edge::Left | Edge::Right));
        let (mut w, mut h) = (z.start.w, z.start.h);
        if z.sides & LEFT != 0 {
            w -= if even_x { 2.0 * dx } else { dx };
        }
        if z.sides & RIGHT != 0 {
            w += if even_x { 2.0 * dx } else { dx };
        }
        if z.sides & TOP != 0 {
            h -= if even_y { 2.0 * dy } else { dy };
        }
        if z.sides & BOTTOM != 0 {
            h += if even_y { 2.0 * dy } else { dy };
        }
        // (near its own proportions: caught onto them; the side held moves to where they are kept)
        let (a, b) = z.aspect;
        let mut held_aspect = false;
        let (horiz, vert) = (z.sides & (LEFT | RIGHT) != 0, z.sides & (TOP | BOTTOM) != 0);
        if b > 0.0 {
            if vert {
                let want = a + b * w;
                if (h - want).abs() < CATCH {
                    h = want;
                    held_aspect = true;
                }
            } else if horiz {
                let want = (h - a) / b;
                if (w - want).abs() < CATCH {
                    w = want;
                    held_aspect = true;
                }
            }
        }
        // (near its own size: caught onto it, first of all)
        if (w - z.pref.0).abs() < CATCH {
            w = z.pref.0;
            held_aspect = false;
        }
        if (h - z.pref.1).abs() < CATCH {
            h = z.pref.1;
            held_aspect = false;
        }
        let area = self.screens[si].area;
        let (mw, mh) = Self::size_max(area, &k, place);
        let w = w.clamp(MIN_W, mw.max(MIN_W));
        let h = h.clamp(MIN_H, mh.max(MIN_H));
        // (a window held by its left or top side: its other side stays where it is)
        let mut at = z.at;
        if pop {
            if z.sides & LEFT != 0 {
                at.0 = z.start.x + z.start.w - w;
            }
            if z.sides & TOP != 0 {
                at.1 = z.start.y - pop::POP_TOP + z.start.h - h;
            }
        }
        if (w, h) == z.live && at == z.at && held_aspect == z.held_aspect {
            return;
        }
        let z = self.sizing.as_mut().unwrap();
        z.live = (w, h);
        z.at = at;
        z.held_aspect = held_aspect;
        if pop {
            if let Some(it) = self.screens[si].items.get_mut(&k) {
                it.slot.x = at.0;
                it.slot.y = at.1;
            }
        }
        self.relayout_with(Some(&HashSet::from([k.clone()])));
        self.draw_guides();
        self.commit();
    }

    /// The button let go while resizing: the size kept (the one it asks for: none kept). True: taken.
    pub(super) fn size_release(&mut self) -> bool {
        let Some(z) = self.sizing.take() else { return false };
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
        }
        let own = (z.live.0 - z.pref.0).abs() < 0.5 && (z.live.1 - z.pref.1).abs() < 0.5;
        let keep = if own { None } else { Some((z.live.0.round(), z.live.1.round())) };
        let k = unpop(&z.key).to_string();
        let pop = is_pop(&z.key);
        let at = z.at;
        let mon = self.screens[z.si].ov.monitor.name();
        self.change_config(|c| {
            let w = c.widgets.entry(k.clone()).or_default();
            if pop {
                w.pop_size = keep;
                w.pop_at.get_or_insert_with(BTreeMap::new).insert(mon.clone(), at);
            } else {
                w.panel_size = keep;
            }
        });
        self.drop_guides();
        self.relayout_with(Some(&HashSet::from([z.key.clone()])));
        self.set_cursor();
        self.commit();
        true
    }

    /// A double-click on a side: the size it asks for again, springing there.
    fn size_reset(&mut self, si: usize, k: &str) {
        let pop = is_pop(k);
        let wk = unpop(k).to_string();
        self.change_config(|c| {
            let w = c.widgets.entry(wk.clone()).or_default();
            if pop {
                w.pop_size = None;
            } else {
                w.panel_size = None;
            }
        });
        if let Some(it) = self.screens[si].items.get_mut(k) {
            // (a little past it, and back)
            it.rest.w.vel += 120.0;
            it.rest.h.vel += 80.0;
        }
        self.relayout_with(Some(&HashSet::from([k.to_string()])));
        self.commit();
    }

    /// The sides of a panel lit now: those the mouse is near, or holds.
    pub(super) fn lit_sides(&self, si: usize, k: &str) -> u8 {
        if let Some(z) = self.sizing.as_ref().filter(|z| z.si == si && z.key == k) {
            return z.sides;
        }
        self.size_hover.as_ref().filter(|(s, hk, _)| *s == si && hk == k).map_or(0, |h| h.2)
    }

    /// While resizing: the size it asks for, dashed, and the size by the cursor; while a window is moved: the lines it snapped to.
    /// Drawn on the glass over the frame (the mouse goes through it).
    pub(super) fn draw_guides(&mut self) {
        let (si, outline, chip) = if let Some(z) = &self.sizing {
            let Some(body) = self.panel_body(z.si, &z.key) else { return };
            let place = self.screens[z.si].items.get(&z.key).map_or(Place::Edge(Edge::Top), |it| it.slot.place);
            let (pw, ph) = z.pref;
            // (where it would stand at that size: grown from where it holds)
            let (x, y) = if is_pop(&z.key) {
                (body.x, body.y)
            } else {
                match place {
                    Place::Edge(Edge::Top) => (body.x + body.w / 2.0 - pw / 2.0, body.y),
                    Place::Edge(Edge::Bottom) => (body.x + body.w / 2.0 - pw / 2.0, body.y + body.h - ph),
                    Place::Edge(Edge::Left) => (body.x, body.y + body.h / 2.0 - ph / 2.0),
                    Place::Edge(Edge::Right) => (body.x + body.w - pw, body.y + body.h / 2.0 - ph / 2.0),
                    Place::Corner(Corner::TopLeft) => (body.x, body.y),
                    Place::Corner(Corner::TopRight) => (body.x + body.w - pw, body.y),
                    Place::Corner(Corner::BottomLeft) => (body.x, body.y + body.h - ph),
                    Place::Corner(Corner::BottomRight) => (body.x + body.w - pw, body.y + body.h - ph),
                }
            };
            let mouse = self.mouse;
            (z.si, Some(Rect::new(x, y, pw, ph)), Some((format!("{} × {}", z.live.0.round() as i32, z.live.1.round() as i32), mouse.1 + 16.0, mouse.2 + 18.0, z.held_aspect)))
        } else if let Some((si, _, _, _)) = self.pop_drag.as_ref().filter(|_| !self.snap_lines.is_empty()) {
            (*si, None, None)
        } else {
            self.drop_guides();
            return;
        };
        let (s, area) = (self.screens[si].s, self.screens[si].area);
        let (pw, ph) = ((area.x + area.w) * s, (area.y + area.h) * s);
        if self.guides.as_ref().is_some_and(|(gs, _)| *gs != si) {
            self.drop_guides();
        }
        if self.guides.is_none() {
            let Ok(p) = self.screens[si].ov.top_piece(&self.g, pw.ceil() as u32 + 2, ph.ceil() as u32 + 2) else { return };
            self.guides = Some((si, p));
        }
        let lines = self.snap_lines.clone();
        let p = self.painter(s);
        let Some((_, piece)) = self.guides.as_ref() else { return };
        let surface = piece.surface.clone();
        let _ = self.g.draw_surface(&surface, |_| {
            let acc = crate::draw::accent();
            if let Some(r) = outline {
                p.dashed(r, RADIUS_OPEN, 1.5, acc.alpha(0.85));
            }
            for (vertical, at) in &lines {
                let r = if *vertical { Rect::new(*at - 0.75, area.y, 1.5, area.h) } else { Rect::new(area.x, *at - 0.75, area.w, 1.5) };
                p.round(r, 0.75, acc.alpha(0.85));
            }
            if let Some((t, cx, cy, held)) = &chip {
                let tw = p.measure(t, 11.0, crate::paint::Weight::Semibold, 120.0);
                p.round(Rect::new(*cx, *cy - 11.0, tw + 16.0, 22.0), 11.0, crate::paint::Color(0.0, 0.0, 0.0, 0.8));
                // (caught onto its proportions: in the theme's colour)
                p.text(t, *cx + 8.0, *cy, 11.0, crate::paint::Weight::Semibold, 120.0, if *held { acc } else { crate::draw::DIM });
            }
        });
    }

    pub(super) fn drop_guides(&mut self) {
        if let Some((si, p)) = self.guides.take() {
            if let Some(sc) = self.screens.get(si) {
                sc.ov.remove_top(&p);
            }
        }
        self.snap_lines.clear();
    }

    /// A window being moved to `b`: snapped to the screen's edges and middle lines and to the other windows near it (Shift held:
    /// left free); the lines it snapped to kept, to be drawn.
    pub(super) fn snap_window(&mut self, si: usize, pk: &str, b: Bx) -> Bx {
        let shift = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(windows::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT.0 as i32) as u16 & 0x8000 != 0 };
        let mut lines = Vec::new();
        let mut out = b;
        if !shift {
            let a = self.screens[si].area;
            let others: Vec<Bx> = self.screens[si].items.iter().filter(|(k, _)| is_pop(k) && k.as_str() != pk).map(|(_, it)| it.rest.now()).collect();
            // (each way: where its near side, its far side or its middle may go, and the line that shows it)
            let mut xs: Vec<(f32, f32)> = vec![(a.x + GAP, a.x + GAP), (a.x + a.w - GAP - b.w, a.x + a.w - GAP), (a.x + a.w / 2.0 - b.w / 2.0, a.x + a.w / 2.0)];
            let mut ys: Vec<(f32, f32)> = vec![(a.y + GAP, a.y + GAP), (a.y + a.h - GAP - b.h, a.y + a.h - GAP), (a.y + a.h / 2.0 - b.h / 2.0, a.y + a.h / 2.0)];
            for o in &others {
                xs.extend([(o.x, o.x), (o.x + o.w - b.w, o.x + o.w), (o.x + o.w + GAP, o.x + o.w + GAP / 2.0), (o.x - GAP - b.w, o.x - GAP / 2.0)]);
                ys.extend([(o.y, o.y), (o.y + o.h - b.h, o.y + o.h), (o.y + o.h + GAP, o.y + o.h + GAP / 2.0), (o.y - GAP - b.h, o.y - GAP / 2.0)]);
            }
            let best = |v: f32, list: &[(f32, f32)]| list.iter().filter(|(to, _)| (to - v).abs() < SNAP).min_by(|p, q| (p.0 - v).abs().total_cmp(&(q.0 - v).abs())).copied();
            if let Some((to, line)) = best(b.x, &xs) {
                out.x = to;
                lines.push((true, line));
            }
            if let Some((to, line)) = best(b.y, &ys) {
                out.y = to;
                lines.push((false, line));
            }
        }
        if lines != self.snap_lines {
            self.snap_lines = lines;
            if self.snap_lines.is_empty() {
                self.drop_guides();
            } else {
                self.draw_guides();
            }
        }
        out
    }
}
