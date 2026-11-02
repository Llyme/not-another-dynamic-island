//! Grips: the bar a popped-out window is held by, and the handle over a panel open on its edge. Each is a pill that springs out of a
//! dot as it comes, widens and brightens under the mouse, turns the theme colour and glows while held, and stretches a little as it is
//! dragged; it shrinks back to a dot as it goes. Its springs live here, one set for each widget that shows a grip.

use super::*;

#[derive(Default)]
pub(super) struct GripAnim {
    /// coming (0 none, 1 shown; it springs a little past)
    show: Spring,
    /// the mouse on it
    over: Spring,
    /// held
    held: Spring,
    /// how far it stretches, dragged (px)
    stretch: Spring,
    /// where its window was a step ago (to tell how fast it moves)
    last: Option<(f32, f32)>,
}

impl GripAnim {
    fn moving(&self) -> bool {
        self.show.moving() || self.over.moving() || self.held.moving() || self.stretch.moving()
    }

    /// (show, over, held, stretch) as drawn now
    pub(super) fn now(&self) -> [f32; 4] {
        [self.show.now, self.over.now, self.held.now, self.stretch.now]
    }

    pub(super) fn shown(&self) -> bool {
        self.show.now > 0.01
    }
}

impl Frame {
    /// Where each grip is going: shown, under the mouse, held; dragged, how far it stretches.
    pub(super) fn grip_targets(&mut self) {
        let (msi, mx, my) = self.mouse;
        let mut wants: Vec<(String, f32, f32, f32, Option<(f32, f32)>, f32)> = Vec::new();
        for (si, sc) in self.screens.iter().enumerate() {
            for (k, it) in &sc.items {
                if is_pop(k) {
                    let b = it.rest.now();
                    let held = self.pop_drag.as_ref().is_some_and(|(_, dk, ..)| dk == k);
                    let shown = held || self.pop_marks.as_ref().is_some_and(|m| &m.key == k);
                    let over = msi == si && mx >= b.x && mx < b.x + b.w && my >= b.y && my < b.y + pop::POP_TOP;
                    wants.push((k.clone(), shown as u8 as f32, (shown && over) as u8 as f32, held as u8 as f32, Some((b.x, b.y)), 0.0));
                } else if it.open.now > 0.5 {
                    let pulled = self.tear.as_ref().is_some_and(|(ts, tk, ..)| *ts == si && tk == k);
                    let hovered = self.hover.as_ref().is_some_and(|(hs, hk)| *hs == si && hk == k);
                    let shown = pulled || hovered;
                    let over = shown && self.shape_now(si, it).is_ok_and(|(_, b)| msi == si && mx >= b.x && mx < b.x + b.w && my >= b.y && my < b.y + PAD + HANDLE);
                    let stretch = if pulled { (self.tear_pull * 0.4).min(30.0) } else { 0.0 };
                    wants.push((k.clone(), shown as u8 as f32, over as u8 as f32, pulled as u8 as f32, None, stretch));
                }
            }
        }
        let seen: HashSet<String> = wants.iter().map(|w| w.0.clone()).collect();
        for (k, show, over, held, _, stretch) in wants {
            if show == 0.0 && !self.grips.contains_key(&k) {
                continue;
            }
            let g = self.grips.entry(k).or_default();
            g.show.to = show;
            g.over.to = over;
            g.held.to = held;
            if stretch > 0.0 || held == 0.0 {
                g.stretch.to = stretch;
            }
        }
        // (gone with its widget: its grip too)
        self.grips.retain(|k, _| seen.contains(k));
    }

    /// One step of every grip's springs; each that moved is drawn again. True: one moves.
    pub(super) fn step_grips(&mut self, dt: f32) -> bool {
        self.grip_targets();
        let mut moved: Vec<String> = Vec::new();
        let places: HashMap<String, (f32, f32)> = self.screens.iter().flat_map(|sc| sc.items.iter().filter(|(k, _)| is_pop(k)).map(|(k, it)| (k.clone(), (it.rest.now().x, it.rest.now().y)))).collect();
        for (k, g) in self.grips.iter_mut() {
            // (a window dragged: its grip stretches as fast as it goes, at most so far; slowing, it draws back)
            if let Some(&(x, y)) = places.get(k) {
                if g.held.to > 0.5 && dt > 0.0 {
                    if let Some((lx, ly)) = g.last {
                        let speed = ((x - lx).powi(2) + (y - ly).powi(2)).sqrt() / dt;
                        g.stretch.to = (speed * 0.025).min(36.0);
                    }
                } else {
                    g.stretch.to = g.stretch.to.min(if g.held.to > 0.5 { 36.0 } else { 0.0 });
                }
                g.last = Some((x, y));
            }
            if !g.moving() {
                continue;
            }
            // (coming: a soft spring, a little past its size; the mouse and the hand: quick)
            g.show.step_with(dt, 420.0, 18.0);
            g.over.step_with(dt, 900.0, 55.0);
            g.held.step_with(dt, 900.0, 55.0);
            g.stretch.step_with(dt, 500.0, 30.0);
            moved.push(k.clone());
        }
        // (shrunk away: forgotten)
        self.grips.retain(|_, g| g.show.to > 0.0 || g.moving());
        for k in &moved {
            if let Some(si) = (0..self.screens.len()).find(|&i| self.screens[i].items.contains_key(k)) {
                self.render(si, k);
            }
        }
        !moved.is_empty()
    }

    pub(super) fn grips_moving(&self) -> bool {
        self.grips.values().any(|g| g.moving())
    }
}
