//! Edit Mode: holding a widget (or Edit in its menu) puts the whole frame into it. Every widget comes out, the idle ones too (faded),
//! the screen dims a little, and a shelf in its middle holds what is not on screen: the hidden widgets, then the widgets of plugins
//! that are off (faded, a power mark on each), each a tile of its own peek, small, its name under it; with the dock out, a tile to
//! pin an app. Over it, Find: typed into at once (Esc empties it, then ends Edit Mode); the tiles narrow to the widgets it names
//! (apps are the launcher's to find). A widget carried back toward it: the shelf rings in the theme
//! colour and opens a slot. A widget is placed by dragging it from the tray to an edge, and hidden by dragging it into the tray. The dock's apps
//! are moved, pinned and unpinned by dragging them. A click on an empty spot, Esc, or a minute untouched ends it. While it lasts,
//! panels stay shut and banners wait.

use super::*;
use crate::paint::Color;

/// the shelf: four tiles a row, each so big, so far apart, in from its edges so far; the Find field over them; at most so many rows
const TILE: f32 = 72.0;
const TILE_GAP: f32 = 8.0;
const COLS: usize = 4;
const TRAY_P: f32 = 14.0;
const TRAY_W: f32 = TRAY_P * 2.0 + TILE * COLS as f32 + TILE_GAP * (COLS as f32 - 1.0);
const FIND_H: f32 = 30.0;
const ROWS: usize = 4;
const CHIP_H: f32 = 28.0;
/// the Find field's box to type in (its key among the boxes)
pub(super) const FIND_KEY: &str = "\u{0}edit-find";

/// A widget the tray holds.
#[derive(Clone, Debug)]
pub(super) struct Chip {
    pub key: String,
    pub label: String,
    /// its plugin is off (dropping it turns the plugin on)
    pub plugin_off: Option<String>,
}

/// What is carried by the mouse (not a widget on screen: those are dragged as always).
pub(super) enum Carried {
    Chip(Chip),
    App { key: String, name: String, image: String, pinned: bool },
    TrayIcon { key: String, image: String },
}

pub(super) struct Carry {
    pub what: Carried,
    pub piece: Piece,
    pub w: f32,
    pub h: f32,
}

pub(super) struct Edit {
    pub si: usize,
    /// the Find field, in the shelf; what is typed in it
    pub field: Option<Rect>,
    pub find: String,
    /// apps found (with the dock out), each a tile (pressed: pinned to the dock); the tile to pin one from the launcher
    pub apps: Vec<(Rect, crate::apps::App)>,
    pub add: Option<Rect>,
    pub touched: Instant,
    pub dim: Option<Piece>,
    pub tray: Option<Piece>,
    /// where the tray is (logical, in its screen)
    pub at: Bx,
    pub chips: Vec<(Rect, Chip)>,
    pub carry: Option<Carry>,
    /// something held over the tray (it lights up: let go there, and it is hidden)
    pub over_tray: bool,
    /// the edge something carried is nearest (its rail lit), or the corner it is in
    pub rail: Option<Edge>,
    pub corner: Option<Corner>,
}

/// NADI's own widgets: each its id, its name, and whether it can be placed.
const OWN: [(&str, &str, bool); 16] = [
    (crate::builtin::APPS, "Apps", true),
    (crate::builtin::DOCK, "Dock", true),
    (crate::builtin::TASKBAR, "Taskbar", true),
    (crate::builtin::QUICK, "Volume Mixer", true),
    (crate::builtin::TRAY, "Tray", true),
    (crate::sysw::PRIVACY, "Mic and Camera", true),
    (crate::sysw::DRIVES, "Drives", true),
    (crate::sysw::LANG, "Keyboard Language", true),
    (crate::sysw::DESKS, "Desktops", true),
    (crate::net::NET, "Network", true),
    (crate::sysw::DEVICES, "Bluetooth", true),
    (crate::screen::SCREEN, "Screen", true),
    (crate::screen::FOCUS, "Focus", true),
    (crate::screen::POWER, "Power Mode", true),
    (crate::builtin::NOTIFS, "Notifications", true),
    (crate::builtin::FLYOUT, "Quick Settings", true),
];

impl Frame {
    pub(super) fn enter_edit(&mut self, si: usize) {
        if self.edit.is_some() || si >= self.screens.len() {
            return;
        }
        self.close_menu();
        // panels shut (those kept open stay)
        let mut shut = Vec::new();
        for (s, sc) in self.screens.iter_mut().enumerate() {
            for (k, it) in sc.items.iter_mut() {
                if it.open.to == 1.0 && self.config.widgets.get(k).and_then(|c| c.always_open) != Some(true) && !is_pop(k) {
                    it.open.to = 0.0;
                    it.pinned = false;
                    shut.push((s, k.clone()));
                }
            }
        }
        for (s, k) in shut {
            self.set_open(s, &k, false);
        }
        // the dimmed screen, under the widgets
        let m = self.screens[si].ov.monitor.rect;
        let dim = self.screens[si].ov.piece_below(&self.g, (m.right - m.left) as u32, (m.bottom - m.top) as u32).ok();
        let s = self.screens[si].s;
        let tray = self.screens[si].ov.piece(&self.g, (TRAY_W * s) as u32 + 4, (480.0 * s) as u32).ok();
        self.edit = Some(Edit { si, field: None, find: String::new(), apps: Vec::new(), add: None, touched: Instant::now(), dim, tray, at: Bx::default(), chips: Vec::new(), carry: None, over_tray: false, rail: None, corner: None });
        self.render_edit_dim();
        self.render_edit_tray();
        // (Find typed into at once)
        self.open_find();
        // (the Tray widget shows every icon while editing)
        self.sync_tray_widget();
        self.relayout_with(Some(&HashSet::new()));
        self.tick_rate();
    }

    pub(super) fn leave_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        if let Some(ib) = self.input.as_mut().filter(|i| i.key == FIND_KEY) {
            ib.close();
        }
        if let Some(sc) = self.screens.get(e.si) {
            for p in [e.dim, e.tray, e.carry.map(|c| c.piece)].into_iter().flatten() {
                let _ = sc.ov.remove(&p);
            }
        }
        self.sync_tray_widget();
        self.relayout();
        self.next_banner();
        self.tick_rate();
    }

    /// The widgets that are not on screen: the hidden ones of the plugins that are on, then the widgets of the plugins that are off.
    fn edit_chips(&self) -> Vec<Chip> {
        let mut out = Vec::new();
        for p in self.plugins.iter().filter(|p| p.on) {
            for w in &p.widgets {
                let k = key(&p.id, &w.id);
                if self.config.widgets.get(&k).and_then(|c| c.place.as_deref()) == Some("off") {
                    out.push(Chip { key: k, label: w.name.clone(), plugin_off: None });
                }
            }
        }
        // (NADI's own, every one of them (the tray too: placing it is what makes NADI hold the tray))
        for (id, label, there) in OWN {
            let k = key(crate::builtin::PLUGIN, id);
            // (in a group: placed, with it)
            if there && self.place_of(crate::builtin::PLUGIN, id).is_none() && self.group_of(&k).is_none() {
                out.push(Chip { key: k, label: label.into(), plugin_off: None });
            }
        }
        for p in self.plugins.iter().filter(|p| !p.on && p.problem.is_none()) {
            for w in &p.widgets {
                out.push(Chip { key: key(&p.id, &w.id), label: w.name.clone(), plugin_off: Some(p.id.clone()) });
            }
        }
        out
    }

    /// Draw the tray: a chip for each widget not on screen (as many to a row as fit). Empty, it is a small place to drop one.
    /// The dimmed screen, and a thin rail along each edge (where widgets can go); the one something carried is nearest, lit.
    pub(super) fn render_edit_dim(&mut self) {
        let Some(e) = &self.edit else { return };
        let Some(d) = &e.dim else { return };
        let (si, lit, corner) = (e.si, e.rail, e.corner);
        let s = self.screens[si].s;
        let p = self.painter(s);
        let m = self.screens[si].ov.monitor.rect;
        let (w, h) = ((m.right - m.left) as f32 / s, (m.bottom - m.top) as f32 / s);
        let a = self.screens[si].area;
        let surface = d.surface.clone();
        let _ = self.g.draw_surface(&surface, |dc| {
            unsafe {
                dc.Clear(Some(&windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            }
            p.round(Rect::new(0.0, 0.0, w, h), 0.0, Color(0.02, 0.02, 0.04, 0.35));
            let t = 3.0;
            let inset = 6.0;
            for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                let r = match e {
                    Edge::Top => Rect::new(a.x + 16.0, a.y + inset, a.w - 32.0, t),
                    Edge::Bottom => Rect::new(a.x + 16.0, a.y + a.h - inset - t, a.w - 32.0, t),
                    Edge::Left => Rect::new(a.x + inset, a.y + 16.0, t, a.h - 32.0),
                    Edge::Right => Rect::new(a.x + a.w - inset - t, a.y + 16.0, t, a.h - 32.0),
                };
                if lit == Some(e) && corner.is_none() {
                    p.round(Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0), 4.5, draw::accent().alpha(0.25));
                    p.round(r, 1.5, draw::accent());
                } else {
                    p.round(r, 1.5, Color(1.0, 1.0, 1.0, 0.09));
                }
            }
            for c in [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight] {
                let (cx, cy) = match c {
                    Corner::TopLeft => (a.x + 12.0, a.y + 12.0),
                    Corner::TopRight => (a.x + a.w - 12.0, a.y + 12.0),
                    Corner::BottomLeft => (a.x + 12.0, a.y + a.h - 12.0),
                    Corner::BottomRight => (a.x + a.w - 12.0, a.y + a.h - 12.0),
                };
                if corner == Some(c) {
                    p.circle(cx, cy, 11.0, draw::accent().alpha(0.25));
                    p.circle(cx, cy, 6.0, draw::accent());
                } else {
                    p.circle(cx, cy, 5.0, Color(1.0, 1.0, 1.0, 0.12));
                }
            }
        });
        self.commit();
    }

    /// Something carried at (x, y): the rail of the edge it is nearest lights (none, over the shelf).
    fn light_rail(&mut self, si: usize, x: f32, y: f32) {
        let a = self.screens[si].area;
        let over = self.in_edit_tray(si, x, y);
        let d = [(Edge::Top, y - a.y), (Edge::Bottom, a.y + a.h - y), (Edge::Left, x - a.x), (Edge::Right, a.x + a.w - x)];
        let near = d.iter().copied().fold((Edge::Bottom, f32::MAX), |b, c| if c.1 < b.1 { c } else { b }).0;
        let want = (!over).then_some(near);
        // (in a corner: near two edges at once)
        let reach = 140.0;
        let corner = (!over).then(|| match (d[0].1 < reach, d[1].1 < reach, d[2].1 < reach, d[3].1 < reach) {
            (true, _, true, _) => Some(Corner::TopLeft),
            (true, _, _, true) => Some(Corner::TopRight),
            (_, true, true, _) => Some(Corner::BottomLeft),
            (_, true, _, true) => Some(Corner::BottomRight),
            _ => None,
        }).flatten();
        if self.edit.as_ref().is_some_and(|e| e.rail != want || e.corner != corner) {
            let e = self.edit.as_mut().unwrap();
            e.rail = want;
            e.corner = corner;
            self.render_edit_dim();
        }
    }

    /// The box to type in over Find (live: the shelf narrows as it is typed).
    pub(super) fn open_find(&mut self) {
        let Some(e) = &self.edit else { return };
        let (si, at, f, find) = (e.si, e.at, e.field, e.find.clone());
        let Some(f) = f else { return };
        let sc = &self.screens[si];
        let (s, m) = (sc.s, sc.ov.monitor.rect);
        let r = RECT { left: m.left + ((at.x + f.x) * s).round() as i32, top: m.top + ((at.y + f.y) * s).round() as i32, right: m.left + ((at.x + f.x + f.w) * s).round() as i32, bottom: m.top + ((at.y + f.y + f.h) * s).round() as i32 };
        if self.input.is_none() {
            self.input = crate::inputbox::InputBox::new(&self.g).ok();
        }
        let g = &self.g;
        if let Some(ib) = &mut self.input {
            ib.open(g, r, s, FIND_KEY.into(), String::new(), "find".into(), true, "Find".into(), find);
            ib.set_look(&Value::Null);
            ib.render(g, &self.pc);
        }
        self.commit();
    }

    /// What was typed in Find: the shelf narrows to it, and the widgets on screen it names glow where they are.
    pub(super) fn set_find(&mut self, text: String) {
        if let Some(e) = &mut self.edit {
            e.find = text;
            e.touched = Instant::now();
        }
        self.render_edit_tray();
        let on: Vec<(usize, String)> = self.screens.iter().enumerate().flat_map(|(si, sc)| sc.items.keys().filter(|k| !is_pop(k)).map(move |k| (si, k.clone()))).collect();
        for (si, k) in on {
            self.render(si, &k);
        }
        self.commit();
    }

    /// A widget's name, as the shelf would show it.
    pub(super) fn widget_label(&self, k: &str) -> String {
        let Some((pl, id)) = k.split_once('/') else { return String::new() };
        if pl == crate::builtin::PLUGIN {
            if let Some((_, label, _)) = OWN.iter().find(|(o, ..)| *o == id) {
                return label.to_string();
            }
        }
        if let Some(n) = self.plugins.iter().find(|p| p.id == pl).and_then(|p| p.widgets.iter().find(|w| w.id == id).map(|w| if w.name.is_empty() { p.name.clone() } else { w.name.clone() })) {
            return n;
        }
        self.edit_chips().into_iter().find(|c| c.key == k).map(|c| c.label).unwrap_or_else(|| id.to_string())
    }

    /// Is this widget on screen named by what is typed in Find?
    pub(super) fn found_by_find(&self, k: &str) -> bool {
        let Some(e) = &self.edit else { return false };
        let f = e.find.trim().to_lowercase();
        !f.is_empty() && self.widget_label(unpop(k)).to_lowercase().contains(&f)
    }

    pub(super) fn render_edit_tray(&mut self) {
        let Some(e) = &self.edit else { return };
        let si = e.si;
        let s = self.screens[si].s;
        let p = self.painter(s);
        let find = e.find.trim().to_lowercase();
        let names = |c: &Chip| -> bool {
            find.is_empty() || c.label.to_lowercase().contains(&find) || c.plugin_off.as_deref().unwrap_or(c.key.split('/').next().unwrap_or("")).to_lowercase().contains(&find)
        };
        let chips: Vec<Chip> = self.edit_chips().into_iter().filter(|c| names(c)).collect();
        // (with the dock out: a tile to pin an app, from the launcher. Find names widgets alone: apps are the launcher's to find)
        let dock_on = self.dock_placed();
        let apps: Vec<crate::apps::App> = Vec::new();
        let add_tile = dock_on && find.is_empty();
        let field = Rect::new(TRAY_P, TRAY_P, TRAY_W - 2.0 * TRAY_P, FIND_H);
        let top = TRAY_P + FIND_H + 12.0;
        let n = chips.len() + apps.len() + usize::from(add_tile) + usize::from(e.over_tray);
        let slot = |i: usize| Rect::new(TRAY_P + (i % COLS) as f32 * (TILE + TILE_GAP), top + (i / COLS) as f32 * (TILE + TILE_GAP), TILE, TILE);
        let rows = n.div_ceil(COLS).clamp(1, ROWS);
        let (w, h) = (TRAY_W, top + rows as f32 * (TILE + TILE_GAP) - TILE_GAP + TRAY_P);
        let a = self.screens[si].area;
        let at = Bx { x: a.x + a.w / 2.0 - w / 2.0, y: a.y + a.h / 2.0 - h / 2.0, w, h };
        let over = e.over_tray;
        let Ok(shape) = shape::floating(&self.g.d2d_factory, w * s, h * s, 22.0 * s) else { return };
        let Some(piece) = &e.tray else { return };
        let surface = piece.surface.clone();
        let fits = |i: usize| i < ROWS * COLS;
        // (each tile's look: its widget's peek, its icon alone, small)
        let looks: Vec<Value> = chips.iter().map(|c| self.widgets.get(&c.key).map(|w| peek::icon_only(&w.content["peek"])).unwrap_or(Value::Null)).collect();
        let mut chip_boxes = Vec::new();
        let mut app_boxes = Vec::new();
        let mut add_box = None;
        let _ = self.g.draw_surface(&surface, |dc| {
            let _ = fill_shape(dc, &shape, FACE);
            // (a widget carried here: the shelf rings in the theme colour)
            if over {
                p.outline(Rect::new(1.0, 1.0, w - 2.0, h - 2.0), 21.0, 1.5, draw::accent());
            }
            p.round(field, FIND_H / 2.0, Color(0.08, 0.08, 0.1, 1.0));
            p.icon("search", field.x + 12.0, field.y + FIND_H / 2.0 - 6.5, 13.0, draw::DIM);
            p.text(if e.find.is_empty() { "Find" } else { &e.find }, field.x + 32.0, field.y + FIND_H / 2.0, 12.0, Weight::Regular, field.w - 44.0, if e.find.is_empty() { draw::DIM } else { draw::TEXT });
            let mut i = 0;
            let tile = |r: Rect, faded: bool| {
                p.round(r, 14.0, if faded { Color(0.07, 0.07, 0.085, 1.0) } else { Color(0.08, 0.082, 0.098, 1.0) });
            };
            for (c, look) in chips.iter().zip(&looks) {
                if !fits(i) {
                    break;
                }
                let r = slot(i);
                let off = c.plugin_off.is_some();
                tile(r, off);
                // (its peek, small, in a little black piece of its own)
                let mini = Rect::new(r.x + r.w / 2.0 - 15.0, r.y + 12.0, 30.0, 26.0);
                p.round(mini, 9.0, FACE);
                p.outline(mini, 9.0, 1.0, Color(1.0, 1.0, 1.0, 0.08));
                let a = if off { 0.5 } else { 1.0 };
                draw::faded(dc, a, || {
                    if look.is_object() {
                        let _ = peek::draw(&p, look, Rect::new(mini.x + 3.0, mini.y + 3.0, mini.w - 6.0, mini.h - 6.0), true, false);
                    } else {
                        p.icon(if off { "plug" } else { "grid" }, mini.x + 8.0, mini.y + 6.0, 14.0, draw::DIM);
                    }
                    let lw = p.measure(&c.label, 10.0, Weight::Regular, r.w - 8.0);
                    p.text(&c.label, r.x + r.w / 2.0 - lw / 2.0, r.y + 54.0, 10.0, Weight::Regular, r.w - 8.0, draw::DIM);
                });
                // (its plugin off: a power mark in its corner)
                if off {
                    p.round(Rect::new(r.right() - 20.0, r.y + 6.0, 14.0, 14.0), 7.0, Color(0.15, 0.15, 0.18, 1.0));
                    p.icon("power", r.right() - 18.0, r.y + 8.0, 10.0, draw::DIM);
                }
                chip_boxes.push(r);
                i += 1;
            }
            for app in &apps {
                if !fits(i) {
                    break;
                }
                let r = slot(i);
                tile(r, false);
                let img = Rect::new(r.x + r.w / 2.0 - 13.0, r.y + 12.0, 26.0, 26.0);
                if !p.picture(&crate::launcher::app_image(&app.id), img, 6.0) {
                    p.round(img, 6.0, draw::SOFT);
                }
                let lw = p.measure(&app.name, 10.0, Weight::Regular, r.w - 8.0);
                p.text(&app.name, r.x + r.w / 2.0 - lw / 2.0, r.y + 54.0, 10.0, Weight::Regular, r.w - 8.0, draw::DIM);
                app_boxes.push(r);
                i += 1;
            }
            if add_tile && fits(i) {
                let r = slot(i);
                p.dashed(r, 14.0, 1.2, Color(1.0, 1.0, 1.0, 0.14));
                p.icon("plus", r.x + r.w / 2.0 - 8.0, r.y + 18.0, 16.0, draw::DIM);
                let lw = p.measure("App", 10.0, Weight::Regular, r.w);
                p.text("App", r.x + r.w / 2.0 - lw / 2.0, r.y + 54.0, 10.0, Weight::Regular, r.w, draw::DIM);
                add_box = Some(r);
                i += 1;
            }
            // (where a widget carried here lands)
            if over && fits(i) {
                let r = slot(i);
                p.round(r, 14.0, draw::accent().alpha(0.08));
                p.dashed(r, 14.0, 1.5, draw::accent());
            }
            // (nothing found: a magnifier, crossed out)
            if i == 0 && !e.find.is_empty() {
                let cy = top + TILE / 2.0;
                p.icon("search", w / 2.0 - 10.0, cy - 10.0, 20.0, draw::DIM);
                p.line(&[(w / 2.0 - 13.0, cy + 13.0), (w / 2.0 + 13.0, cy - 13.0)], 1.6, draw::DIM);
            }
        });
        let e = self.edit.as_mut().unwrap();
        e.field = Some(field);
        if let Some(piece) = &mut e.tray {
            let _ = piece.place((at.x * s).round() as i32, (at.y * s).round() as i32);
        }
        e.at = at;
        e.chips = chip_boxes.into_iter().zip(chips).collect();
        e.apps = app_boxes.into_iter().zip(apps).collect();
        e.add = add_box;
        // (the shelf is centred, so it moves as Find narrows it: the box typed in, with its field)
        let m = self.screens[si].ov.monitor.rect;
        let px = |v: f32| (v * s).round() as i32;
        let r = RECT { left: m.left + px(at.x + field.x), top: m.top + px(at.y + field.y), right: m.left + px(at.x + field.x + field.w), bottom: m.top + px(at.y + field.y + field.h) };
        if let Some(ib) = self.input.as_mut().filter(|i| i.shown && i.key == FIND_KEY) {
            ib.follow(r);
        }
        self.hit_regions();
        self.commit();
    }

    /// (for checking: what Edit Mode shows)
    pub(super) fn edit_dump(&self) -> Value {
        match &self.edit {
            Some(e) => json!({ "screen": e.si, "tray": [e.at.x, e.at.y, e.at.w, e.at.h], "chips": e.chips.iter().map(|(r, c)| json!({ "key": c.key, "label": c.label, "off": c.plugin_off.is_some(), "at": [r.x, r.y, r.w, r.h] })).collect::<Vec<_>>(), "carry": e.carry.is_some() }),
            None => Value::Null,
        }
    }

    /// Over the tray, or near it (the middle of the screen: let go there, a widget is hidden).
    pub(super) fn in_edit_tray(&self, si: usize, x: f32, y: f32) -> bool {
        const NEAR: f32 = 80.0;
        self.edit.as_ref().is_some_and(|e| e.si == si && x >= e.at.x - NEAR && x <= e.at.x + e.at.w + NEAR && y >= e.at.y - NEAR && y <= e.at.y + e.at.h + NEAR)
    }

    /// A press while editing: on the tray (a chip is carried off), on a dock's app (carried), on a widget (picked up at once). True:
    /// handled here.
    pub(super) fn edit_press(&mut self, si: usize, x: f32, y: f32) -> bool {
        let Some(e) = &mut self.edit else { return false };
        e.touched = Instant::now();
        if e.si == si && e.at.contains(x, y) {
            let (lx, ly) = (x - e.at.x, y - e.at.y);
            let inside = |r: &Rect| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h;
            // (Find: typed into)
            if e.field.is_some_and(|f| inside(&f)) {
                self.open_find();
                return true;
            }
            // (the tile to pin an app: the launcher, to pick one)
            if e.add.is_some_and(|r| inside(&r)) {
                self.open_launcher(true);
                return true;
            }
            // (an app found: pinned to the dock, last)
            if let Some((_, a)) = e.apps.iter().find(|(r, _)| inside(r)).cloned() {
                self.change_config(|c| {
                    if !c.pinned.iter().any(|p| p.id.eq_ignore_ascii_case(&a.id)) {
                        c.pinned.push(config::Pin { name: a.name.clone(), id: a.id.clone(), icon: String::new() });
                    }
                });
                self.sync_builtins(true);
                self.render_edit_tray();
                return true;
            }
            if let Some((_, c)) = e.chips.iter().find(|(r, _)| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h).cloned() {
                // (a widget whose plugin is on: lifted out as itself, carried as any widget on screen is)
                if c.plugin_off.is_none() && self.widgets.contains_key(&c.key) && self.lift_out(si, &c.key, x, y) {
                    return true;
                }
                let s = self.screens[si].s;
                let w = self.painter(s).measure(&c.label, 11.5, Weight::Semibold, 200.0) + 24.0;
                self.start_carry(si, Carried::Chip(c), w, CHIP_H, x, y);
            }
            return true;
        }
        // a tray icon, in the Tray widget
        let tray = key(crate::builtin::PLUGIN, crate::builtin::TRAY);
        if self.item_at(si, x, y).as_deref() == Some(tray.as_str()) {
            if let Some(it) = self.screens[si].items.get(&tray) {
                if let Ok((_, b)) = self.shape_now(si, it) {
                    let (lx, ly) = (x - b.x, y - b.y);
                    let hit = it.buttons.iter().find(|(r, _)| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h).map(|(_, a)| a.clone());
                    if let Some((owner, uid)) = hit.as_deref().and_then(|a| a.strip_prefix("tray:")).and_then(|r| r.split_once(':')) {
                        let (owner, uid) = (owner.parse::<isize>().unwrap_or(0), uid.parse::<u32>().unwrap_or(0));
                        if let Some(i) = crate::builtin::tray_icons().into_iter().find(|i| i.owner == owner && i.uid == uid) {
                            let what = Carried::TrayIcon { key: crate::builtin::tray_key(&i), image: format!("trayicon:{}", i.icon) };
                            self.start_carry(si, what, 26.0, 26.0, x, y);
                            return true;
                        }
                    }
                }
            }
        }
        // a dock's app
        let dock = self.dock_in_use();
        if self.item_at(si, x, y).as_deref() == Some(dock.as_str()) {
            if let Some(it) = self.screens[si].items.get(&dock) {
                if let Ok((_, b)) = self.shape_now(si, it) {
                    let (lx, ly) = (x - b.x, y - b.y);
                    let hit = it.buttons.iter().find(|(r, _)| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h).map(|(_, a)| a.clone());
                    if let Some(i) = hit.as_deref().and_then(|a| a.strip_prefix("app:")).and_then(|i| i.parse::<usize>().ok()) {
                        if let Some(a) = self.bi.dock_apps(&self.config).get(i) {
                            let what = Carried::App { key: a.key.clone(), name: a.name.clone(), image: a.image.clone(), pinned: a.pinned };
                            self.start_carry(si, what, 34.0, 34.0, x, y);
                            return true;
                        }
                    }
                }
            }
        }
        // a widget: picked up at once (no holding while editing)
        if let Some(k) = self.item_at(si, x, y) {
            self.press = Some((si, k, x, y, Instant::now() - HOLD));
            self.mouse = (si, x, y);
            return true;
        }
        true
    }

    /// A hidden widget lifted out of the shelf: put on the edge nearest the mouse, last there, and picked up at once, its middle under
    /// the mouse (so the widgets of the edge it is carried to part for it, and it goes back into the shelf as any widget does).
    fn lift_out(&mut self, si: usize, k: &str, x: f32, y: f32) -> bool {
        let spot = self.drop_place(si, x, y);
        let name = spot.place.name().to_string();
        let key = k.to_string();
        self.change_config(|c| {
            let w = c.widgets.entry(key).or_default();
            w.place = Some(name);
            w.rank = Some(100_000);
        });
        self.relayout();
        let Some(b) = self.screens.get(si).and_then(|sc| sc.items.get(k)).map(|it| it.rest.now()) else { return false };
        self.press = Some((si, k.to_string(), b.x + b.w / 2.0, b.y + b.h / 2.0, Instant::now() - HOLD));
        self.mouse = (si, x, y);
        self.pick_up();
        self.render_edit_tray();
        true
    }

    /// A widget carried over the shelf: drawn smaller (as small as a tile), so it is plain where it will go; away from it, its size.
    fn shrink_over_shelf(&mut self, si: usize, over: bool) {
        use windows::Foundation::Numerics::Matrix3x2;
        let Some((_, k, _, _)) = self.drag.clone() else { return };
        let Some(it) = self.screens.get(si).and_then(|sc| sc.items.get(&k)) else { return };
        let s = self.screens[si].s;
        let b = it.rest.now();
        let z = if over { (TILE / b.w.max(b.h)).clamp(0.4, 1.0) } else { 1.0 };
        let (px, py) = (b.w * s / 2.0, b.h * s / 2.0);
        unsafe {
            let _ = it.piece.visual.SetTransform2(&Matrix3x2 { M11: z, M12: 0.0, M21: 0.0, M22: z, M31: px * (1.0 - z), M32: py * (1.0 - z) });
        }
        self.commit();
    }

    fn start_carry(&mut self, si: usize, what: Carried, w: f32, h: f32, x: f32, y: f32) {
        let s = self.screens[si].s;
        let Ok(piece) = self.screens[si].ov.piece(&self.g, (w * s) as u32 + 4, (h * s) as u32 + 4) else { return };
        if let Some(e) = &mut self.edit {
            e.carry = Some(Carry { what, piece, w, h });
        }
        self.render_carry(si, x, y);
        unsafe {
            let _ = SetCapture(self.screens[si].ov.hwnd);
        }
    }

    pub(super) fn render_carry(&mut self, si: usize, x: f32, y: f32) {
        let Some(e) = &self.edit else { return };
        let Some(c) = &e.carry else { return };
        let s = self.screens[si].s;
        let p = self.painter(s);
        let (w, h) = (c.w, c.h);
        let surface = c.piece.surface.clone();
        let label = match &c.what {
            Carried::Chip(ch) => Some(ch.label.clone()),
            Carried::App { .. } | Carried::TrayIcon { .. } => None,
        };
        let image = match &c.what {
            Carried::App { image, .. } | Carried::TrayIcon { image, .. } => image.clone(),
            _ => String::new(),
        };
        let _ = self.g.draw_surface(&surface, |_| {
            match &label {
                Some(l) => {
                    p.round(Rect::new(0.0, 0.0, w, h), h / 2.0, draw::FACE);
                    p.round(Rect::new(0.0, 0.0, w, h), h / 2.0, draw::SOFT);
                    p.text(l, 12.0, h / 2.0, 11.5, Weight::Semibold, w - 20.0, draw::TEXT);
                }
                None => {
                    if !p.picture(&image, Rect::new(2.0, 2.0, w - 4.0, h - 4.0), 8.0) {
                        p.round(Rect::new(2.0, 2.0, w - 4.0, h - 4.0), 9.0, draw::SOFT);
                    }
                }
            }
        });
        let over = self.in_edit_tray(si, x, y);
        let e = self.edit.as_mut().unwrap();
        if let Some(c) = &mut e.carry {
            let _ = c.piece.place(((x - w / 2.0) * s).round() as i32, ((y - h / 2.0) * s).round() as i32);
        }
        if e.over_tray != over {
            e.over_tray = over;
            self.render_edit_tray();
        }
        self.commit();
    }

    /// The mouse moved while editing: what is carried follows it, and a widget held over the tray lights it. True: handled here.
    pub(super) fn edit_move(&mut self, si: usize, x: f32, y: f32) -> bool {
        let Some(e) = &mut self.edit else { return false };
        e.touched = Instant::now();
        if e.carry.is_some() {
            if !matches!(e.carry.as_ref().map(|c| &c.what), Some(Carried::App { .. } | Carried::TrayIcon { .. })) {
                self.light_rail(si, x, y);
            }
            self.render_carry(si, x, y);
            return true;
        }
        if self.drag.is_some() {
            self.light_rail(si, x, y);
            let over = self.in_edit_tray(si, x, y);
            if self.edit.as_ref().is_some_and(|e| e.over_tray != over) {
                self.edit.as_mut().unwrap().over_tray = over;
                self.render_edit_tray();
                self.shrink_over_shelf(si, over);
            }
        }
        false
    }

    /// The button let go while editing. True: handled here.
    pub(super) fn edit_click(&mut self, si: usize, x: f32, y: f32) -> bool {
        // (let go: whatever was carried at its own size again)
        if self.edit.as_ref().is_some_and(|e| e.over_tray) && self.drag.is_some() {
            self.shrink_over_shelf(si, false);
        }
        // (let go: no rail lit)
        if self.edit.as_ref().is_some_and(|e| e.rail.is_some() || e.corner.is_some()) {
            let e = self.edit.as_mut().unwrap();
            e.rail = None;
            e.corner = None;
            self.render_edit_dim();
        }
        let Some(e) = &self.edit else { return false };
        // what was carried: put down
        if e.carry.is_some() {
            self.drop_carry(si, x, y);
            return true;
        }
        if self.drag.is_some() {
            return false;
        }
        if e.si == si && e.at.contains(x, y) {
            return true;
        }
        // on a widget: nothing (it is moved by dragging); on the dimmed screen: done
        if self.item_at(si, x, y).is_none() {
            self.leave_edit();
        }
        true
    }

    fn drop_carry(&mut self, si: usize, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let Some(e) = &mut self.edit else { return };
        let Some(c) = e.carry.take() else { return };
        e.over_tray = false;
        let _ = self.screens[e.si].ov.remove(&c.piece);
        let in_tray = self.in_edit_tray(si, x, y);
        match c.what {
            Carried::Chip(chip) => {
                if !in_tray {
                    // placed where it was let go, among the others there
                    let spot = self.drop_place(si, x, y);
                    let a = self.screens[si].area;
                    let mut row: Vec<(f32, String)> = self.screens[si]
                        .items
                        .iter()
                        .filter(|(o, _)| self.widgets.get(*o).and_then(|w| self.place_of(&w.plugin, &w.id)).is_some_and(|(p, _)| p == spot.place))
                        .map(|(o, it)| (order_in(spot.place, it.slot.place, it.rest.now(), a), o.clone()))
                        .collect();
                    row.push((held_order(spot, x, y, a), chip.key.clone()));
                    row.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let name = spot.place.name().to_string();
                    let k = chip.key.clone();
                    self.change_config(|c| {
                        for (i, (_, key)) in row.iter().enumerate() {
                            let w = c.widgets.entry(key.clone()).or_default();
                            w.rank = Some(i as i32 * 10);
                            w.place = Some(name.clone());
                            if *key == k {
                                w.style = Some(if spot.float { "floating".into() } else { "merged".into() });
                                w.stack = if spot.down && !spot.head { Some("down".into()) } else { None };
                                w.join = None;
                            }
                        }
                    });
                    if let Some(id) = chip.plugin_off {
                        let _ = self.tx.send(Command::SetOn { id, on: true });
                    }
                }
            }
            Carried::App { key: app, name, pinned, .. } => self.drop_app(si, x, y, &app, &name, pinned),
            Carried::TrayIcon { key: k, .. } => self.drop_tray_icon(si, x, y, &k),
        }
        self.render_edit_tray();
        self.hit_regions();
        self.commit();
    }

    /// A dock's app let go: on the dock, it is pinned where it was let go (among the pinned ones); off it, unpinned.
    fn drop_app(&mut self, si: usize, x: f32, y: f32, app: &str, name: &str, pinned: bool) {
        let dock = self.dock_in_use();
        let Some(it) = self.screens[si].items.get(&dock) else { return };
        let Ok((_, b)) = self.shape_now(si, it) else { return };
        let near = x >= b.x - 40.0 && x <= b.x + b.w + 40.0 && y >= b.y - 50.0 && y <= b.y + b.h + 50.0;
        let apps = self.bi.dock_apps(&self.config);
        // where along the dock: before the first pinned app whose middle is past the mouse
        let lx = x - b.x;
        let mut at = 0usize;
        for (r, a) in &it.buttons {
            let Some(i) = a.strip_prefix("app:").and_then(|i| i.parse::<usize>().ok()) else { continue };
            let Some(d) = apps.get(i) else { continue };
            if d.pinned && d.key != app && r.x + r.w / 2.0 < lx {
                at += 1;
            }
        }
        let (app, name) = (app.to_string(), name.to_string());
        self.change_config(|c| {
            let had = c.pinned.iter().position(|p| p.id.to_lowercase() == app).map(|i| c.pinned.remove(i));
            // (on the dock: pinned there; off it: unpinned, and one that was not pinned stays as it was)
            if near {
                let pin = had.unwrap_or(config::Pin { name: name.clone(), id: app.clone(), icon: String::new() });
                let at = at.min(c.pinned.len());
                c.pinned.insert(at, pin);
            }
            let _ = pinned;
        });
        self.sync_builtins(true);
    }

    /// A tray icon let go: on the Tray widget, it joins its row where it was let go; off it, it leaves the row.
    fn drop_tray_icon(&mut self, si: usize, x: f32, y: f32, icon: &str) {
        let tray = key(crate::builtin::PLUGIN, crate::builtin::TRAY);
        let Some(it) = self.screens[si].items.get(&tray) else { return };
        let Ok((_, b)) = self.shape_now(si, it) else { return };
        let near = x >= b.x - 30.0 && x <= b.x + b.w + 30.0 && y >= b.y - 40.0 && y <= b.y + b.h + 40.0;
        let list = crate::builtin::tray_icons();
        let row = crate::builtin::tray_row(&list, &self.config.tray_pinned);
        // (where along the row: before the first of its icons whose middle is past the mouse)
        let lx = x - b.x;
        let mut at = 0usize;
        for (n, (r, _)) in it.buttons.iter().enumerate().take(row.len()) {
            if row.get(n).is_some_and(|k| k != icon) && r.x + r.w / 2.0 < lx {
                at += 1;
            }
        }
        let icon = icon.to_string();
        self.change_config(|c| {
            let mut r: Vec<String> = row.iter().filter(|k| **k != icon).cloned().collect();
            if near {
                r.insert(at.min(r.len()), icon.clone());
            }
            c.tray_pinned = r;
        });
        self.sync_tray_widget();
    }

    /// While editing: Esc, or a minute untouched, ends it.
    pub(super) fn edit_watch(&mut self) {
        let Some(e) = &self.edit else { return };
        if self.esc || (e.touched.elapsed() > Duration::from_secs(60) && self.drag.is_none() && e.carry.is_none()) {
            self.leave_edit();
        }
    }
}
