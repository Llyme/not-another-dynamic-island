//! The menu of a widget (a right-click on it): a small panel of NADI's own beside it, about that widget only, all pictures (ghost
//! buttons): its look, where it goes (a map of the screen's edges and corners), its switches, Hide, Edit, Pop Out, and Settings. Elsewhere (a
//! banner, NADI's tray icon) a shorter one: Settings and Exit. What is about the whole frame lives in Settings.

use crate::draw::{self, Painter};
use crate::paint::Weight;
use crate::shape::Rect;

/// What a press in the menu does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// floating or merged, large or small, its icon alone or not
    Look(bool, bool, bool),
    Move(&'static str),
    Pin,
    AlwaysShow,
    OverFull,
    AlwaysOpen,
    HideIdle,
    Bump,
    /// the dock: the tray merged into it, or not
    MergeTray,
    Hide,
    Edit,
    Settings,
    /// its panel out as a window (out already: back)
    PopOut,
    Exit,
    /// a row of a thing's own menu (by its place)
    Item(usize),
}

/// How the widget stands now (what the menu shows).
pub struct State {
    pub floating: bool,
    pub large: bool,
    pub place: String,
    pub pinned: bool,
    pub always_shown: bool,
    pub over_full: bool,
    pub always_open: bool,
    pub hide_idle: bool,
    /// it bumps: comes out for a moment when something happens
    pub bump: bool,
    /// none: it is its icon alone always (no switch)
    pub icon_only: Option<bool>,
    /// it has a panel (it can be popped out)
    pub can_pop: bool,
    /// its panel is out as a window
    pub popped: bool,
    /// the dock: the tray merged into it (none: not the dock)
    pub merge_tray: Option<bool>,
}

pub const W: f32 = 240.0;
const P: f32 = 12.0;
/// the looks: switches of two (merged or floating, small or large, words or its icon alone)
const SEG: f32 = 30.0;
/// the map of the screen: its edges and corners
const MAP: f32 = 82.0;
/// the row of switches, and the foot
const TOG: f32 = 34.0;
const FOOT: f32 = 30.0;
const GAP: f32 = 8.0;
pub const RADIUS: f32 = 20.0;

pub fn widget_size() -> (f32, f32) {
    (W, P + SEG + GAP + MAP + GAP + TOG + GAP + 1.0 + GAP + FOOT + P)
}

pub const MINI_W: f32 = 170.0;
const MINI_ROW: f32 = 32.0;

pub fn mini_size() -> (f32, f32) {
    (MINI_W, 6.0 + 3.0 * MINI_ROW + 2.0 * 2.0 + 6.0)
}

/// A thing's own menu (`n` rows).
pub fn items_size(n: usize) -> (f32, f32) {
    (MINI_W, 6.0 + n as f32 * MINI_ROW + n.saturating_sub(1) as f32 * 2.0 + 6.0)
}

/// A thing's own menu: each row its icon and words (a dangerous one red); one held fills red from its start as it is held.
pub fn draw_items(p: &Painter, items: &[serde_json::Value], hover: Option<usize>, held: Option<(usize, f32)>) -> Vec<(Rect, Act)> {
    let red = crate::paint::Color(0.95, 0.63, 0.63, 1.0);
    let mut hits = Vec::new();
    let mut y = 6.0;
    for (i, it) in items.iter().enumerate() {
        let r = Rect::new(6.0, y, MINI_W - 12.0, MINI_ROW);
        if hover == Some(i) {
            p.round(r, 10.0, draw::FAINT);
        }
        if let Some((_, f)) = held.filter(|(h, _)| *h == i) {
            p.round(Rect::new(r.x, r.y, r.w * f.clamp(0.0, 1.0), r.h), 10.0, red.alpha(0.28));
        }
        // (on: its icon in the theme colour, a dot at its end)
        let on = it["on"] == true;
        let c = if it["tone"] == "no" { red } else { draw::TEXT };
        // (a picture of its own (a tray icon's), else its icon)
        let pic = it["image"].as_str().is_some_and(|i| p.picture(i, Rect::new(r.x + 12.0, r.y + MINI_ROW / 2.0 - 7.0, 14.0, 14.0), 3.0));
        if !pic {
            p.icon(it["icon"].as_str().unwrap_or("dot"), r.x + 12.0, r.y + MINI_ROW / 2.0 - 7.0, 14.0, if on { draw::accent() } else { c });
        }
        p.text(it["label"].as_str().unwrap_or(""), r.x + 36.0, r.y + MINI_ROW / 2.0, 12.0, Weight::Regular, r.w - 40.0 - if on { 14.0 } else { 0.0 }, c);
        if on {
            p.circle(r.right() - 12.0, r.y + MINI_ROW / 2.0, 2.5, draw::accent());
        }
        hits.push((r, Act::Item(i)));
        y += MINI_ROW + 2.0;
    }
    hits
}

/// solid greys (nothing see-through: what is under the menu never shows in it)
const LIT: crate::paint::Color = crate::paint::Color(0.15, 0.15, 0.17, 1.0);
const TRACK: crate::paint::Color = crate::paint::Color(0.09, 0.09, 0.1, 1.0);
const CHOSEN: crate::paint::Color = crate::paint::Color(0.22, 0.22, 0.25, 1.0);
const LINE: crate::paint::Color = crate::paint::Color(0.26, 0.26, 0.29, 1.0);
const MARK: crate::paint::Color = crate::paint::Color(0.3, 0.3, 0.33, 1.0);
const MARK_LIT: crate::paint::Color = crate::paint::Color(0.62, 0.62, 0.66, 1.0);
const RULE: crate::paint::Color = crate::paint::Color(0.15, 0.15, 0.17, 1.0);

/// A rounded box's outline, `width` thick.
fn outline(p: &Painter, r: Rect, rad: f32, width: f32, c: crate::paint::Color) {
    let h = width / 2.0;
    let (x0, y0, x1, y1) = (r.x + h, r.y + h, r.x + r.w - h, r.y + r.h - h);
    let rad = (rad - h).max(0.0);
    let mut pts = Vec::new();
    for (cx, cy, a0) in [(x1 - rad, y0 + rad, -90.0f32), (x1 - rad, y1 - rad, 0.0), (x0 + rad, y1 - rad, 90.0), (x0 + rad, y0 + rad, 180.0)] {
        for i in 0..=6 {
            let a = (a0 + i as f32 * 15.0).to_radians();
            pts.push((cx + rad * a.cos(), cy + rad * a.sin()));
        }
    }
    pts.push(pts[0]);
    p.line(&pts, width, c);
}

/// A ghost button: nothing at rest; under the mouse, a faint ground; on, a small dot under it in the theme colour.
fn ghost(p: &Painter, r: Rect, lit: bool, on: bool) {
    if lit {
        p.round(r, 9.0, LIT);
    }
    if on {
        p.circle(r.x + r.w / 2.0, r.y + r.h - 2.5, 1.8, draw::accent());
    }
}

/// The little pictures of a look: merged into its edge, floating off it, small, large, words, its icon alone.
fn look_glyph(p: &Painter, kind: &str, cx: f32, cy: f32, c: crate::paint::Color) {
    match kind {
        "merged" => {
            p.round(Rect::new(cx - 9.0, cy - 6.0, 18.0, 2.0), 1.0, c);
            p.round(Rect::new(cx - 6.0, cy - 6.0, 12.0, 10.0), 3.5, c);
        }
        "floating" => {
            p.round(Rect::new(cx - 9.0, cy - 7.0, 18.0, 2.0), 1.0, c);
            p.round(Rect::new(cx - 6.0, cy - 2.0, 12.0, 7.0), 3.5, c);
        }
        "small" => p.round(Rect::new(cx - 5.0, cy - 3.0, 10.0, 6.0), 3.0, c),
        "large" => p.round(Rect::new(cx - 8.0, cy - 5.0, 16.0, 10.0), 4.0, c),
        "words" => {
            p.circle(cx - 6.0, cy, 2.4, c);
            p.round(Rect::new(cx - 2.0, cy - 3.5, 9.0, 2.0), 1.0, c);
            p.round(Rect::new(cx - 2.0, cy + 1.5, 6.0, 2.0), 1.0, c);
        }
        _ => p.circle(cx, cy, 4.0, c),
    }
}

/// Draw the widget's menu (from its top-left corner, logical px); the boxes that can be pressed, and what each does. All pictures,
/// each a ghost button; `names`: each one's name, in the same order (the tip of the one under the mouse).
pub fn draw_widget(p: &Painter, st: &State, hover: Option<usize>, names: &mut Vec<&'static str>) -> Vec<(Rect, Act)> {
    let mut hits: Vec<(Rect, Act)> = Vec::new();
    names.clear();
    let x = P;
    let w = W - 2.0 * P;
    let lit = |hits: &Vec<(Rect, Act)>| hover == Some(hits.len());
    let ink = |on: bool| if on { draw::accent() } else { draw::TEXT };
    // its look: switches of two (words or its icon alone only for one that can be either)
    let mut y = P;
    let icon_now = st.icon_only.unwrap_or(true);
    let mut groups: Vec<[(&str, &'static str, bool, Act); 2]> = vec![
        [("merged", "Merged", !st.floating, Act::Look(false, st.large, icon_now)), ("floating", "Floating", st.floating, Act::Look(true, st.large, icon_now))],
        [("small", "Small", !st.large, Act::Look(st.floating, false, icon_now)), ("large", "Large", st.large, Act::Look(st.floating, true, icon_now))],
    ];
    if st.icon_only.is_some() {
        groups.push([("words", "Words", !icon_now, Act::Look(st.floating, st.large, false)), ("icon", "Icon Only", icon_now, Act::Look(st.floating, st.large, true))]);
    }
    let n = groups.len() as f32;
    let gw = (w - (n - 1.0) * 6.0) / n;
    for (g, pair) in groups.into_iter().enumerate() {
        // (a toggle group: its track; the one chosen on a pill)
        p.round(Rect::new(x + g as f32 * (gw + 6.0), y, gw, SEG), 10.0, TRACK);
        for (i, (kind, name, on, act)) in pair.into_iter().enumerate() {
            let r = Rect::new(x + g as f32 * (gw + 6.0) + i as f32 * gw / 2.0, y, gw / 2.0, SEG);
            let pill = Rect::new(r.x + 3.0, r.y + 3.0, r.w - 6.0, r.h - 6.0);
            if on {
                p.round(pill, 7.5, CHOSEN);
            } else if lit(&hits) {
                p.round(pill, 7.5, LIT);
            }
            look_glyph(p, kind, r.x + r.w / 2.0, r.y + r.h / 2.0, if on { draw::TEXT } else { draw::DIM });
            hits.push((r, act));
            names.push(name);
        }
    }
    // where it goes: the screen, a bar on each edge and a bracket in each corner; where it is now in colour
    y += SEG + GAP;
    let m = Rect::new(x + 4.0, y, w - 8.0, MAP);
    outline(p, m, 9.0, 1.5, LINE);
    let (cx, cy, r, b) = (m.x + m.w / 2.0, m.y + m.h / 2.0, m.x + m.w, m.y + m.h);
    let places: [(&'static str, &'static str, Vec<Rect>, Rect); 8] = [
        ("top-left", "Top Left", vec![Rect::new(m.x + 6.0, m.y + 6.0, 18.0, 4.0), Rect::new(m.x + 6.0, m.y + 6.0, 4.0, 12.0)], Rect::new(m.x, m.y, 44.0, 30.0)),
        ("top", "Top", vec![Rect::new(cx - 20.0, m.y + 6.0, 40.0, 4.0)], Rect::new(cx - 32.0, m.y, 64.0, 26.0)),
        ("top-right", "Top Right", vec![Rect::new(r - 24.0, m.y + 6.0, 18.0, 4.0), Rect::new(r - 10.0, m.y + 6.0, 4.0, 12.0)], Rect::new(r - 44.0, m.y, 44.0, 30.0)),
        ("left", "Left", vec![Rect::new(m.x + 6.0, cy - 13.0, 4.0, 26.0)], Rect::new(m.x, cy - 18.0, 34.0, 36.0)),
        ("right", "Right", vec![Rect::new(r - 10.0, cy - 13.0, 4.0, 26.0)], Rect::new(r - 34.0, cy - 18.0, 34.0, 36.0)),
        ("bottom-left", "Bottom Left", vec![Rect::new(m.x + 6.0, b - 10.0, 18.0, 4.0), Rect::new(m.x + 6.0, b - 18.0, 4.0, 12.0)], Rect::new(m.x, b - 30.0, 44.0, 30.0)),
        ("bottom", "Bottom", vec![Rect::new(cx - 20.0, b - 10.0, 40.0, 4.0)], Rect::new(cx - 32.0, b - 26.0, 64.0, 26.0)),
        ("bottom-right", "Bottom Right", vec![Rect::new(r - 24.0, b - 10.0, 18.0, 4.0), Rect::new(r - 10.0, b - 18.0, 4.0, 12.0)], Rect::new(r - 44.0, b - 30.0, 44.0, 30.0)),
    ];
    for (place, name, marks, hit) in places {
        let here = st.place == place;
        let c = if here { draw::accent() } else if lit(&hits) { MARK_LIT } else { MARK };
        for mk in marks {
            p.round(mk, 2.0, c);
        }
        hits.push((hit, Act::Move(place)));
        names.push(name);
    }
    // the switches: a row of pictures (one that has no panel: no Always Open)
    y += MAP + GAP;
    let mut toggles: Vec<(&str, &'static str, bool, Act)> = vec![
        ("desktop", "Pin to This Monitor", st.pinned, Act::Pin),
        ("eye", "Always Show", st.always_shown, Act::AlwaysShow),
        ("maximize", "Show Over Full Screen", st.over_full, Act::OverFull),
    ];
    if st.can_pop {
        toggles.push(("window-new", "Always Open", st.always_open, Act::AlwaysOpen));
    }
    toggles.push(("moon", "Hide When Idle", st.hide_idle, Act::HideIdle));
    toggles.push(("sparkle", "Bump When Something Happens", st.bump, Act::Bump));
    if let Some(on) = st.merge_tray {
        toggles.push(("merge", "Merge Tray", on, Act::MergeTray));
    }
    let n = toggles.len() as f32;
    let step = (w - TOG) / (n - 1.0).max(1.0);
    for (i, (icon, name, on, act)) in toggles.into_iter().enumerate() {
        let r = Rect::new(x + i as f32 * step, y, TOG, TOG);
        ghost(p, r, lit(&hits), on);
        p.icon(icon, r.x + (TOG - 16.0) / 2.0, r.y + (TOG - 16.0) / 2.0 - 1.0, 16.0, ink(on));
        hits.push((r, act));
        names.push(name);
    }
    // the foot: hide and edit; pop out and settings; between them, the name of what is under the mouse
    y += TOG + GAP;
    p.round(Rect::new(x, y, w, 1.0), 0.5, RULE);
    y += 1.0 + GAP;
    let mut foot: Vec<(Rect, &str, &'static str, bool, Act)> = vec![
        (Rect::new(x, y, 34.0, FOOT), "eyeoff", "Hide", false, Act::Hide),
        (Rect::new(x + 38.0, y, 34.0, FOOT), "grid", "Edit Widgets", false, Act::Edit),
    ];
    if st.can_pop {
        foot.push((Rect::new(x + w - 72.0, y, 34.0, FOOT), "external", if st.popped { "Put Back" } else { "Pop Out" }, st.popped, Act::PopOut));
    }
    foot.push((Rect::new(x + w - 34.0, y, 34.0, FOOT), "gear", "Settings", false, Act::Settings));
    for (r, icon, name, on, act) in foot {
        ghost(p, r, lit(&hits), on);
        p.icon(icon, r.x + 9.0, r.y + 7.0, 16.0, ink(on));
        hits.push((r, act));
        names.push(name);
    }
    hits
}

/// Draw the short menu (a banner's, the tray icon's); what can be pressed.
pub fn draw_mini(p: &Painter, hover: Option<usize>) -> Vec<(Rect, Act)> {
    let mut hits = Vec::new();
    let mut y = 6.0;
    for (i, (icon, label, act)) in [("grid", "Edit Widgets", Act::Edit), ("gear", "Settings", Act::Settings), ("power", "Exit", Act::Exit)].into_iter().enumerate() {
        let r = Rect::new(6.0, y, MINI_W - 12.0, MINI_ROW);
        if hover == Some(i) {
            p.round(r, 10.0, draw::FAINT);
        }
        p.icon(icon, r.x + 12.0, r.y + MINI_ROW / 2.0 - 7.0, 14.0, draw::TEXT);
        p.text(label, r.x + 36.0, r.y + MINI_ROW / 2.0, 12.0, Weight::Regular, r.w - 40.0, draw::TEXT);
        hits.push((r, act));
        y += MINI_ROW + 2.0;
    }
    hits
}

/// (for checking) A widget's menu and the short one, drawn offscreen side by side.
pub fn preview(g: &crate::gfx::Gfx, pc: &std::sync::Arc<nadi_core::pc::Pc>) -> windows::core::Result<(u32, u32, Vec<u8>)> {
    let s = 1.25;
    let (w, h) = widget_size();
    let (mw, mh) = mini_size();
    let p = Painter { g, dc: &g.dc, s, pc, t: 0.0, viz: [0.0; 3] };
    let big = crate::shape::floating(&g.d2d_factory, w * s, h * s, RADIUS * s)?;
    let small = crate::shape::floating(&g.d2d_factory, mw * s, mh * s, RADIUS * s)?;
    let st = State { floating: false, large: true, place: "top".into(), pinned: false, always_shown: true, over_full: false, always_open: false, hide_idle: true, bump: true, icon_only: Some(false), can_pop: true, popped: false, merge_tray: None };
    let (tw, th) = (((w + mw + 60.0) * s) as u32, ((h + 40.0) * s) as u32);
    let px = g.snapshot(tw, th, [0.17, 0.2, 0.3, 1.0], |dc| unsafe {
        dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2::translation(20.0 * s, 20.0 * s));
        let _ = crate::paint::fill_shape(dc, &big, draw::FACE);
        draw_widget(&p, &st, Some(9), &mut Vec::new());
        dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2::translation((w + 40.0) * s, 20.0 * s));
        let _ = crate::paint::fill_shape(dc, &small, draw::FACE);
        draw_mini(&p, Some(0));
    })?;
    Ok((tw, th, px))
}
