//! Widget groups: one widget on the edge that holds several. At rest it shows one of them as it would alone; the mouse on it, the
//! others' pictograms slide out beside it, each pressed opening that widget's own panel. Made in Edit Mode by holding a widget over
//! another; undone from the group's menu.

use super::*;

impl Frame {
    /// The group a widget is in (its id), if any.
    pub(super) fn group_of(&self, k: &str) -> Option<String> {
        let k = unpop(k);
        self.config.groups.iter().find(|(_, g)| g.members.iter().any(|m| m == k)).map(|(id, _)| id.clone())
    }

    /// A group's widget it shows now (its key), for a group's key (none: not a group).
    pub(super) fn shown_member(&self, k: &str) -> Option<String> {
        let w = self.widgets.get(unpop(k))?;
        w.content["group"].is_object().then(|| ())?;
        w.via.as_ref().map(|(p, id)| key(p, id))
    }

    /// The widget of a group a point is on: one of its pictograms beside it, else the one it shows (none: not a group).
    pub(super) fn group_member_at(&self, si: usize, k: &str, x: f32, y: f32) -> Option<String> {
        let shown = self.shown_member(k)?;
        let it = self.screens.get(si)?.items.get(k)?;
        let (_, b) = self.shape_now(si, it).ok()?;
        let (lx, ly) = (x - b.x, y - b.y);
        let pressed = it.buttons.iter().find(|(r, a)| a.starts_with("\u{0}group:") && lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h).map(|(_, a)| a.clone());
        let Some((gid, i)) = pressed.as_deref().and_then(|a| a.strip_prefix("\u{0}group:")).and_then(|r| r.split_once(':')) else { return Some(shown) };
        let i: usize = i.parse().ok()?;
        self.config.groups.get(gid).and_then(|g| g.members.iter().filter(|m| self.widgets.contains_key(*m)).nth(i).cloned()).or(Some(shown))
    }

    /// The pictogram of a group's (beside the one it shows) under a point: its widget, and its box (logical, in its screen).
    pub(super) fn fan_at(&self, si: usize, k: &str, x: f32, y: f32) -> Option<(String, Rect)> {
        let it = self.screens.get(si)?.items.get(k)?;
        let (_, b) = self.shape_now(si, it).ok()?;
        let (lx, ly) = (x - b.x, y - b.y);
        let (r, a) = it.buttons.iter().find(|(r, a)| a.starts_with("\u{0}group:") && lx >= r.x && lx < r.x + r.w && ly >= r.y && ly < r.y + r.h)?;
        let (gid, i) = a.strip_prefix("\u{0}group:")?.split_once(':')?;
        let i: usize = i.parse().ok()?;
        let m = self.config.groups.get(gid)?.members.iter().filter(|m| self.widgets.contains_key(*m)).nth(i)?.clone();
        Some((m, Rect::new(r.x + b.x, r.y + b.y, r.w, r.h)))
    }

    /// A group's pictogram dragged: past the middle of another's, it takes that one's place (the others move up). Whether one is
    /// being dragged.
    pub(super) fn fan_move(&mut self, si: usize, x: f32, y: f32) -> bool {
        let Some((fs, gk, m, px, py, moved)) = self.fan_drag.clone() else { return false };
        if fs != si {
            return moved;
        }
        if !moved {
            if (x - px).hypot(y - py) <= DRAG_START {
                return false;
            }
            self.fan_drag = Some((fs, gk.clone(), m.clone(), px, py, true));
            self.press = None;
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.screens[si].ov.hwnd);
            }
        }
        self.mouse = (si, x, y);
        let Some(id) = self.group_of(&m) else { return true };
        let Some((other, r)) = self.fan_at(si, &gk, x, y).filter(|(o, _)| *o != m) else { return true };
        let members = self.config.groups.get(&id).map(|g| g.members.clone()).unwrap_or_default();
        let (Some(from), Some(to)) = (members.iter().position(|k| *k == m), members.iter().position(|k| *k == other)) else { return true };
        // (along the edge: across the middle of the other, in the way it is dragged; so that a wide one and a narrow one never swap
        // back and forth)
        let horizontal = (r.x + r.w / 2.0 - px).abs() >= (r.y + r.h / 2.0 - py).abs();
        let (at, mid) = if horizontal { (x, r.x + r.w / 2.0) } else { (y, r.y + r.h / 2.0) };
        if (to > from && at < mid) || (to < from && at > mid) {
            return true;
        }
        self.reorder_group(&id, from, to);
        true
    }

    /// One of a group's widgets moved from one place in its order to another; the one shown, and the one opened, kept.
    fn reorder_group(&mut self, id: &str, from: usize, to: usize) {
        let Some(g) = self.config.groups.get(id).cloned() else { return };
        let alive: Vec<String> = g.members.iter().filter(|k| self.widgets.contains_key(*k)).cloned().collect();
        let shown = alive.get(g.shown.min(alive.len().saturating_sub(1))).cloned();
        let opened = self.group_opened.get(id).and_then(|i| alive.get(*i)).cloned();
        let mut members = g.members.clone();
        let k = members.remove(from);
        members.insert(to, k);
        let alive: Vec<String> = members.iter().filter(|k| self.widgets.contains_key(*k)).cloned().collect();
        let place = |k: &Option<String>| k.as_ref().and_then(|k| alive.iter().position(|a| a == k));
        let (new_shown, new_opened) = (place(&shown).unwrap_or(0), place(&opened));
        let id_s = id.to_string();
        self.change_config(|c| {
            if let Some(gr) = c.groups.get_mut(&id_s) {
                gr.members = members;
                gr.shown = new_shown;
            }
        });
        if let Some(o) = new_opened {
            self.group_opened.insert(id_s, o);
        }
        let changed = self.refresh_groups(None);
        self.relayout_with(Some(&changed.into_iter().collect()));
        self.commit();
    }

    /// One of a group's widgets taken out of it: beside it on the edge (the group of one left undone).
    pub(super) fn take_out(&mut self, gk: &str, member: &str) {
        let Some(id) = self.widgets.get(gk).and_then(|w| w.content["group"]["id"].as_str().map(str::to_string)) else { return };
        let Some(g) = self.config.groups.get(&id).cloned() else { return };
        if g.members.len() <= 2 {
            self.ungroup(gk);
            return;
        }
        let (gk, member) = (gk.to_string(), member.to_string());
        self.change_config(|c| {
            let wc = c.widgets.get(&gk).cloned().unwrap_or_default();
            let w = c.widgets.entry(member.clone()).or_default();
            w.place = wc.place.clone();
            w.style = wc.style.clone();
            w.stack = wc.stack.clone();
            w.monitor = wc.monitor.clone();
            w.rank = Some(wc.rank.unwrap_or(0) + 1);
            w.join = None;
            if let Some(gr) = c.groups.get_mut(&id) {
                gr.members.retain(|m| *m != member);
                gr.shown = gr.shown.min(gr.members.len().saturating_sub(1));
            }
        });
        self.group_opened.remove(&id);
        self.refresh_groups(None);
        self.relayout();
    }

    /// A group's key among the widgets.
    pub(super) fn group_key(id: &str) -> String {
        key(crate::builtin::PLUGIN, &format!("group:{id}"))
    }

    /// Can this widget be in a group? NADI's docks, its tray and its launcher's button stand alone.
    fn groupable(k: &str) -> bool {
        let k = unpop(k);
        let alone = [crate::builtin::DOCK, crate::builtin::TASKBAR, crate::builtin::TRAY, crate::builtin::APPS].iter().any(|w| key(crate::builtin::PLUGIN, w) == k);
        !alone && !k.is_empty()
    }

    /// The groups made again from their widgets (those with a widget in `changed`; none: every one), and ones whose widgets are all
    /// gone taken away. The keys of the groups that changed.
    pub(super) fn refresh_groups(&mut self, changed: Option<&HashSet<String>>) -> Vec<String> {
        let mut out = Vec::new();
        let groups: Vec<(String, crate::config::Group)> = self.config.groups.iter().map(|(k, g)| (k.clone(), g.clone())).collect();
        for (id, g) in &groups {
            let gk = Self::group_key(id);
            let touched = changed.is_none_or(|c| g.members.iter().any(|m| c.contains(m))) || !self.widgets.contains_key(&gk);
            if !touched {
                continue;
            }
            let members: Vec<&String> = g.members.iter().filter(|m| self.widgets.contains_key(*m)).collect();
            if members.is_empty() {
                if self.widgets.remove(&gk).is_some() {
                    out.push(gk);
                }
                continue;
            }
            // (the one shown: one opened from its pictogram while its panel is open; else one that wants attention (something that
            // asks for you, or just bumped); else the one chosen)
            let wants = |m: &String| {
                let c = &self.widgets[m].content;
                matches!(c["peek"]["live"]["tone"].as_str(), Some("waiting" | "permission")) || self.group_attention.contains_key(m.as_str())
            };
            let chosen = g.shown.min(members.len() - 1);
            let attention = if wants(members[chosen]) { None } else { members.iter().position(|m| wants(m)) };
            let shown = self.group_opened.get(id).copied().or(attention).unwrap_or(chosen).min(members.len() - 1);
            let w = &self.widgets[members[shown]];
            let mut content = w.content.clone();
            let pics: Vec<Value> = members
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let c = &self.widgets[*m].content;
                    let icon = c["peek"]["icon"].as_str().or(c["peek"]["live"]["icon"].as_str()).or(c["panel"]["tile"]["icon"].as_str()).unwrap_or("dot");
                    json!({ "key": m, "icon": icon, "image": c["peek"]["image"], "action": format!("\u{0}group:{id}:{i}"), "tip": self.widget_label(m) })
                })
                .collect();
            content["group"] = json!({ "id": id, "members": pics, "shown": shown });
            content["idle"] = json!(members.iter().all(|m| self.widgets[*m].content["idle"] == true));
            let via = Some((w.plugin.clone(), w.id.clone()));
            let same = self.widgets.get(&gk).is_some_and(|x| x.content == content && x.via == via);
            if !same {
                let glow_at = self.widgets.get(&gk).map_or_else(Instant::now, |x| x.glow_at);
                self.widgets.insert(gk.clone(), Widget { plugin: crate::builtin::PLUGIN.into(), id: format!("group:{id}"), content, glow_at, via });
                out.push(gk);
            }
        }
        // (a group taken away: its widget too)
        let gone: Vec<String> = self.widgets.keys().filter(|k| k.starts_with(&key(crate::builtin::PLUGIN, "group:")) && !groups.iter().any(|(id, _)| Self::group_key(id) == **k)).cloned().collect();
        for k in gone {
            self.widgets.remove(&k);
            out.push(k);
        }
        out
    }

    /// The widget under the mouse that the one dragged could be grouped with (none: none, or one that stands alone).
    pub(super) fn merge_target(&self, si: usize, dragged: &str, x: f32, y: f32) -> Option<String> {
        if self.edit.is_none() || !(Self::groupable(dragged) || self.widgets.get(dragged).is_some_and(|w| w.content["group"].is_object())) {
            return None;
        }
        let sc = self.screens.get(si)?;
        sc.items
            .iter()
            .filter(|(k, _)| k.as_str() != dragged && !is_pop(k) && (Self::groupable(k) || self.widgets.get(*k).is_some_and(|w| w.content["group"].is_object())))
            .find(|(_, it)| self.shape_now(si, it).is_ok_and(|(_, b)| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h))
            .map(|(k, _)| k.clone())
    }

    /// How far the hold over a widget has filled (0 to 1), while one is held over it.
    pub(super) fn merge_fill(&self, k: &str) -> Option<f32> {
        let (_, t, at) = self.merge_over.as_ref().filter(|(_, t, _)| t == k)?;
        let _ = t;
        Some((at.elapsed().as_secs_f32() / MERGE_HOLD).min(1.0))
    }

    /// The widgets in a widget: a group's, else itself.
    fn members_in(&self, k: &str) -> (Vec<String>, Option<String>) {
        match self.widgets.get(k).and_then(|w| w.content["group"]["id"].as_str().map(str::to_string)) {
            Some(id) => (self.config.groups.get(&id).map(|g| g.members.clone()).unwrap_or_default(), Some(id)),
            None => (vec![k.to_string()], None),
        }
    }

    /// The widget dragged (`dragged`) let go on another (`target`), held there long enough: they are a group (the target's place, the
    /// target first). A group dropped on, or dropped: its widgets go into the one group.
    pub(super) fn merge(&mut self, target: &str, dragged: &str) {
        let (t_members, t_group) = self.members_in(target);
        let (d_members, d_group) = self.members_in(dragged);
        let id = t_group.clone().unwrap_or_else(|| {
            let mut n = 1;
            while self.config.groups.contains_key(&format!("g{n}")) {
                n += 1;
            }
            format!("g{n}")
        });
        let gk = Self::group_key(&id);
        let (t, d) = (target.to_string(), dragged.to_string());
        self.change_config(|c| {
            let mut members = t_members.clone();
            for m in &d_members {
                if !members.contains(m) {
                    members.push(m.clone());
                }
            }
            // (a new group stands where the target stood, as it did; what was joined to the target, joined to the group)
            if t_group.is_none() {
                let mut wc = c.widgets.get(&t).cloned().unwrap_or_default();
                wc.pop_at = None;
                wc.popped_on = None;
                c.widgets.insert(gk.clone(), wc);
                for w in c.widgets.values_mut() {
                    if w.join.as_deref() == Some(t.as_str()) {
                        w.join = Some(gk.clone());
                    }
                }
            }
            // (anything joined to the one dragged: joined to nothing now)
            for w in c.widgets.values_mut() {
                if w.join.as_deref() == Some(d.as_str()) {
                    w.join = None;
                }
            }
            if let Some(dg) = &d_group {
                c.groups.remove(dg);
                c.widgets.remove(&Self::group_key(dg));
            }
            let shown = c.groups.get(&id).map_or(0, |g| g.shown);
            c.groups.insert(id.clone(), crate::config::Group { members, shown });
        });
        self.refresh_groups(None);
        self.relayout();
    }

    /// A group undone: its widgets back on the edge where it stood, side by side, in its order.
    pub(super) fn ungroup(&mut self, gk: &str) {
        let Some(id) = self.widgets.get(gk).and_then(|w| w.content["group"]["id"].as_str().map(str::to_string)) else { return };
        let Some(g) = self.config.groups.get(&id).cloned() else { return };
        let gk = gk.to_string();
        self.change_config(|c| {
            let wc = c.widgets.get(&gk).cloned().unwrap_or_default();
            let rank = wc.rank.unwrap_or(0);
            for (i, m) in g.members.iter().enumerate() {
                let w = c.widgets.entry(m.clone()).or_default();
                w.place = wc.place.clone();
                w.style = wc.style.clone();
                w.stack = wc.stack.clone();
                w.monitor = wc.monitor.clone();
                w.rank = Some(rank + i as i32);
                w.join = None;
            }
            for w in c.widgets.values_mut() {
                if w.join.as_deref() == Some(gk.as_str()) {
                    w.join = None;
                }
            }
            c.groups.remove(&id);
            c.widgets.remove(&gk);
        });
        self.group_opened.remove(&id);
        self.refresh_groups(None);
        self.relayout();
    }
}
