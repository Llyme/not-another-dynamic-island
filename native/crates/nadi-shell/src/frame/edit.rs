//! Edit Mode: holding a widget (or Edit in its menu) puts the whole frame into it. Every widget comes out, the idle ones too (faded),
//! the screen dims a little, and a tray in its middle holds what is not on screen: the hidden widgets, and the widgets of plugins
//! that are off. A widget is placed by dragging it from the tray to an edge, and hidden by dragging it into the tray. The dock's apps
//! are moved, pinned and unpinned by dragging them. A click on an empty spot, Esc, or a minute untouched ends it. While it lasts,
//! panels stay shut and banners wait.

use super::*;

const TRAY_W: f32 = 300.0;
const TRAY_P: f32 = 14.0;
const CHIP_H: f32 = 28.0;

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
    /// the field to find an app to pin to the dock (while the dock is on), in the tray
    pub field: Option<Rect>,
    pub touched: Instant,
    pub dim: Option<Piece>,
    pub tray: Option<Piece>,
    /// where the tray is (logical, in its screen)
    pub at: Bx,
    pub chips: Vec<(Rect, Chip)>,
    pub carry: Option<Carry>,
    /// something held over the tray (it lights up: let go there, and it is hidden)
    pub over_tray: bool,
}

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
        if let Some(d) = &dim {
            let surface = d.surface.clone();
            let _ = self.g.draw_surface(&surface, |dc| unsafe {
                if let Ok(b) = crate::paint::brush(dc, crate::paint::Color(0.02, 0.02, 0.04, 0.35)) {
                    dc.FillRectangle(&windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: 0.0, top: 0.0, right: (m.right - m.left) as f32, bottom: (m.bottom - m.top) as f32 }, &b);
                }
            });
        }
        let s = self.screens[si].s;
        let tray = self.screens[si].ov.piece(&self.g, (TRAY_W * s) as u32 + 4, (480.0 * s) as u32).ok();
        self.edit = Some(Edit { si, field: None, touched: Instant::now(), dim, tray, at: Bx::default(), chips: Vec::new(), carry: None, over_tray: false });
        self.render_edit_tray();
        // (the Tray widget shows every icon while editing)
        self.sync_tray_widget();
        self.relayout_with(Some(&HashSet::new()));
        self.tick_rate();
    }

    pub(super) fn leave_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
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
        let own = [
            (crate::builtin::APPS, "Apps", true),
            (crate::builtin::DOCK, "Dock", true),
            (crate::builtin::QUICK, "Volume Mixer", true),
            (crate::builtin::TRAY, "Tray", true),
            (crate::sysw::PRIVACY, "Mic and Camera", true),
            (crate::sysw::DRIVES, "Drives", true),
            (crate::sysw::LANG, "Keyboard Language", true),
            (crate::sysw::DESKS, "Desktops", true),
            (crate::net::NET, "Network", true),
            (crate::sysw::DEVICES, "Bluetooth", true),
        ];
        for (id, label, there) in own {
            let k = key(crate::builtin::PLUGIN, id);
            if there && self.place_of(crate::builtin::PLUGIN, id).is_none() {
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
    pub(super) fn render_edit_tray(&mut self) {
        let Some(e) = &self.edit else { return };
        let si = e.si;
        let s = self.screens[si].s;
        let p = self.painter(s);
        let chips = self.edit_chips();
        // where each chip goes
        let mut boxes = Vec::new();
        // (with the dock: a field first, to find an app to pin)
        let dock_on = self.dock_placed();
        let field = dock_on.then(|| Rect::new(TRAY_P, TRAY_P, TRAY_W - 2.0 * TRAY_P, 34.0));
        let (mut x, mut y) = (TRAY_P, if dock_on { TRAY_P + 34.0 + 10.0 } else { TRAY_P });
        for c in &chips {
            let w = p.measure(&c.label, 11.5, Weight::Semibold, 200.0) + 24.0;
            if x + w > TRAY_W - TRAY_P && x > TRAY_P {
                x = TRAY_P;
                y += CHIP_H + 8.0;
            }
            boxes.push(Rect::new(x, y, w, CHIP_H));
            x += w + 8.0;
        }
        let (w, h) = if chips.is_empty() && field.is_none() { (120.0, 56.0) } else if chips.is_empty() { (TRAY_W, TRAY_P + 34.0 + TRAY_P) } else { (TRAY_W, y + CHIP_H + TRAY_P) };
        let a = self.screens[si].area;
        let at = Bx { x: a.x + a.w / 2.0 - w / 2.0, y: a.y + a.h / 2.0 - h / 2.0, w, h };
        let over = e.over_tray;
        let Ok(shape) = shape::floating(&self.g.d2d_factory, w * s, h * s, 20.0 * s) else { return };
        let Some(piece) = &e.tray else { return };
        let surface = piece.surface.clone();
        let _ = self.g.draw_surface(&surface, |dc| {
            let _ = fill_shape(dc, &shape, FACE);
            if over {
                let _ = fill_shape(dc, &shape, draw::FAINT);
            }
            if let Some(f) = field {
                p.round(f, 12.0, draw::FAINT);
                p.icon("search", f.x + 12.0, f.y + 10.0, 14.0, draw::DIM);
                p.text("Apps", f.x + 34.0, f.y + f.h / 2.0, 12.0, Weight::Regular, f.w - 40.0, draw::DIM);
            }
            for (c, r) in chips.iter().zip(&boxes) {
                p.round(*r, CHIP_H / 2.0, if c.plugin_off.is_some() { draw::FAINT } else { draw::SOFT });
                p.text(&c.label, r.x + 12.0, r.y + CHIP_H / 2.0, 11.5, Weight::Semibold, r.w - 20.0, if c.plugin_off.is_some() { draw::DIM } else { draw::TEXT });
            }
        });
        let e = self.edit.as_mut().unwrap();
        e.field = field;
        if let Some(piece) = &mut e.tray {
            let _ = piece.place((at.x * s).round() as i32, (at.y * s).round() as i32);
        }
        e.at = at;
        e.chips = boxes.into_iter().zip(chips).collect();
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
            // (the field: the launcher, to pick an app to pin)
            if e.field.is_some_and(|f| lx >= f.x && lx < f.x + f.w && ly >= f.y && ly < f.y + f.h) {
                self.open_launcher(true);
                return true;
            }
            if let Some((_, c)) = e.chips.iter().find(|(r, _)| lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h).cloned() {
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
        let dock = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
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
            self.render_carry(si, x, y);
            return true;
        }
        if self.drag.is_some() {
            let over = self.in_edit_tray(si, x, y);
            if self.edit.as_ref().is_some_and(|e| e.over_tray != over) {
                self.edit.as_mut().unwrap().over_tray = over;
                self.render_edit_tray();
            }
        }
        false
    }

    /// The button let go while editing. True: handled here.
    pub(super) fn edit_click(&mut self, si: usize, x: f32, y: f32) -> bool {
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
        let dock = key(crate::builtin::PLUGIN, crate::builtin::DOCK);
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
                let pin = had.unwrap_or(config::Pin { name: name.clone(), id: app.clone() });
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
