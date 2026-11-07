//! The frame drawn offscreen to a picture: the widgets of a file (or a sample of what the plugins draw), laid out on a screen, one of them
//! open. To see what the frame looks like without a window on the desktop.
//!
//! The file is a list of `{ "key": "plugin/widget", "place": "top", "rank": 10, "content": { peek, panel, tip }, "open": true }`.

use crate::draw::Painter;
use crate::frame::paint_piece;
use crate::gfx::{self, Gfx};
use crate::layout::{self, Item, Metrics, Place};
use crate::shape::{self, Corner, Edge};
use serde_json::{json, Value};
use std::collections::HashSet;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::E_FAIL;

fn sample() -> Value {
    json!([
        { "key": "now-playing/song", "place": "top", "rank": 40, "content": {
            "peek": { "icon": "music", "viz": true, "color": "#c23a7a", "text": "Midnight City", "button": { "icon": "pause", "action": "toggle" } },
            "panel": { "title": "Midnight City", "sub": "M83", "tile": { "icon": "music", "color": "#c23a7a" }, "body": [
                { "kind": "seek", "value": 0.36, "action": "seek", "position": 87, "duration": 243,
                  "before": [{ "icon": "back", "action": "back" }], "after": [{ "icon": "fwd", "action": "fwd" }] },
                { "kind": "controls", "items": [{ "icon": "prev", "action": "prev" }, { "icon": "pause", "action": "toggle" }, { "icon": "next", "action": "next" }] },
                { "kind": "choice", "label": "Speed", "action": "rate", "value": 1, "options": [{ "value": 0.75, "label": "0.75×" }, { "value": 1, "label": "1×" }, { "value": 1.5, "label": "1.5×" }, { "value": 2, "label": "2×" }] }
            ] } } },
        { "key": "claude/sessions", "place": "top", "rank": 39, "open": true, "content": {
            "peek": { "live": { "icon": "sparkle", "tone": "working" }, "color": "#d97757", "text": "Working" },
            "panel": { "title": "Claude", "sub": "2 sessions", "tile": { "icon": "sparkle", "color": "#d97757" }, "body": [
                { "kind": "section", "id": "a", "title": "Fix the thing", "sub": "Editing lib.rs", "live": { "icon": "pen", "tone": "working" },
                  "tile": { "text": "C", "color": "#d97757", "ring": 0.09, "ring_color": "#5ac88c" }, "body": [
                    { "kind": "chips", "items": [{ "icon": "branch", "text": "fix/oauth" }, { "icon": "shield", "text": "Plan", "tone": "hot" }, { "icon": "sparkle", "text": "Sonnet 5.5" }] },
                    { "kind": "strip", "text": "Working for 2m 14s", "side": "Thought 12s", "tone": "working" },
                    { "kind": "checklist", "title": "Plan", "items": [{ "state": "done", "text": "Read the sensor" }, { "state": "now", "text": "Show every session" }, { "state": "next", "text": "Tests" }] },
                    { "kind": "stats", "items": [{ "value": "2", "label": "Files" }, { "value": "12", "label": "Commands" }, { "value": "1", "label": "Agents" }, { "value": "$1.50", "label": "Cost" }] },
                    { "kind": "meter", "label": "Context", "part": 0.09, "text": "90k / 1.0M", "color": "#5ac88c" }
                ] },
                { "kind": "section", "id": "b", "title": "Plugins", "sub": "Finished", "live": { "icon": "check", "tone": "finished" }, "tile": { "text": "C", "color": "#d97757" }, "body": [
                    { "kind": "strip", "text": "Done in 6m 02s", "side": "3 min ago", "tone": "finished" } ] }
            ] } } },
        { "key": "system-meters/meters", "place": "right", "rank": 50, "content": {
            "peek": { "bars": [{ "part": 0.34, "color": "#5ac8e6" }, { "part": 0.61, "color": "#b47ee6" }, { "part": 0.22, "color": "#8adb6e" }], "label": "CPU 34%" },
            "panel": { "title": "System", "sub": "Busiest: RAM at 61%", "body": [] } } },
        { "key": "battery/battery", "place": "right", "rank": 30, "content": {
            "peek": { "vessel": { "level": 0.64, "charging": false }, "color": "#5ac88c", "text": "64%" },
            "panel": { "title": "Battery", "sub": "On battery", "body": [] } } },
        { "key": "ics-calendar/calendar", "place": "top-right", "rank": 10, "content": {
            "peek": { "clock": { "h24": false, "date": true }, "icon": "calendar", "text": "3:30 PM" },
            "panel": { "title": "Calendar", "body": [] } } },
        { "key": "example-weather/weather", "place": "top-left", "rank": 20, "content": {
            "peek": { "icon": "sun", "text": "28°" }, "panel": { "title": "28°C", "body": [] } } },
        { "key": "downloads/downloads", "place": "bottom", "rank": 40, "content": {
            "peek": { "icon": "download", "ring": { "part": 0.42 }, "color": "#4f8ff0", "text": "3.1 MB/s" }, "panel": { "title": "Downloads", "body": [] } } },
        { "key": "games/game", "place": "left", "rank": 30, "content": {
            "peek": { "icon": "gamepad", "color": "#3ea66f", "text": "144" }, "panel": { "title": "Game", "body": [] } } }
    ])
}

pub fn preview(out: &std::path::Path, file: Option<&std::path::Path>) -> windows::core::Result<()> {
    let list: Value = match file {
        Some(f) => serde_json::from_str(&std::fs::read_to_string(f).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?,
        None => sample(),
    };
    let g = Gfx::new()?;
    let pc = nadi_core::pc::Pc::new();
    let s = 1.25f32;
    let (pw, ph) = (1920u32, 1080u32);
    let (w, h) = (pw as f32 / s, ph as f32 / s);
    let m = Metrics::default();
    // (a dump of the frame holds its widgets under "widgets")
    let list = if list["widgets"].is_array() { list["widgets"].clone() } else { list };
    let floats: Vec<String> = list.as_array().into_iter().flatten().filter(|x| x["floating"] == true).filter_map(|x| x["key"].as_str().map(str::to_string)).collect();
    let items: Vec<(Item, Value, bool)> = list
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|x| {
            let place = Place::parse(x["place"].as_str()?)?;
            Some((Item { key: x["key"].as_str()?.to_string(), place, rank: x["rank"].as_i64().unwrap_or(50) as i32, want: 0.0, across: 0.0, down: false, join: x["join"].as_str().map(String::from) }, x["content"].clone(), x["open"].as_bool() == Some(true)))
        })
        .collect();
    let empty = HashSet::new();
    // (NADI_LIQUID: 0 to 100)
    let liquid: Option<f32> = std::env::var("NADI_LIQUID").ok().and_then(|v| v.parse::<f32>().ok()).and_then(crate::frame::liquid_px);
    let float_off = crate::frame::FLOAT_OFF;
    // (laid out before drawing: the liquid is recorded from where they stand)
    let slots = {
        let p = Painter { g: &g, dc: &g.dc, s, pc: &pc, t: 0.4, viz: [0.6, 0.95, 0.45, 0.5, 0.8, 0.4] };
        let mut its: Vec<Item> = Vec::new();
        for (it, content, _) in &items {
            let mut it = it.clone();
            it.want = crate::peek::measure(&p, &content["peek"], it.place.horizontal());
            if let Some(d) = crate::peek::depth_of(&p, &content["peek"], it.want).filter(|_| it.place.horizontal()) {
                it.across = d;
            }
            if content["peek"]["dock"].is_object() {
                it.across = if content["peek"]["dock"]["large"] == true { 112.0 } else { 56.0 };
            }
            if floats.contains(&it.key) {
                it.across = (if it.place.horizontal() { m.depth_h } else { m.depth_v }) + float_off;
            }
            its.push(it);
        }
        layout::layout(&its, w, h, m, &[])
    };
    let liquid_list = liquid.and_then(|_| {
        let f = m.fillet;
        let reach = 40.0;
        let mut shapes: Vec<(crate::frame::Bx, f32)> = Vec::new();
        for sl in &slots {
            let Some((_, _, open)) = items.iter().find(|(i, _, _)| i.key == sl.key) else { continue };
            if *open {
                continue;
            }
            let depth = if sl.place.horizontal() { m.depth_h } else { m.depth_v };
            if floats.contains(&sl.key) {
                let o = float_off;
                let b = match sl.place {
                    Place::Edge(Edge::Top) => crate::frame::Bx { x: sl.x + f, y: sl.y + o, w: sl.w - 2.0 * f, h: depth },
                    Place::Edge(Edge::Bottom) => crate::frame::Bx { x: sl.x + f, y: sl.y + sl.h - o - depth, w: sl.w - 2.0 * f, h: depth },
                    Place::Edge(Edge::Left) => crate::frame::Bx { x: sl.x + o, y: sl.y + f, w: depth, h: sl.h - 2.0 * f },
                    Place::Edge(Edge::Right) => crate::frame::Bx { x: sl.x + sl.w - o - depth, y: sl.y + f, w: depth, h: sl.h - 2.0 * f },
                    _ => crate::frame::Bx { x: sl.x + o, y: sl.y + o, w: sl.w - o - f, h: sl.h - o - f },
                };
                shapes.push((b, b.w.min(b.h) / 2.0));
                continue;
            }
            let r = depth * 0.44;
            let body = match sl.place {
                Place::Edge(Edge::Top) => crate::frame::Bx { x: sl.x + f, y: sl.y - reach, w: sl.w - 2.0 * f, h: sl.h + reach },
                Place::Edge(Edge::Bottom) => crate::frame::Bx { x: sl.x + f, y: sl.y, w: sl.w - 2.0 * f, h: sl.h + reach },
                Place::Edge(Edge::Left) => crate::frame::Bx { x: sl.x - reach, y: sl.y + f, w: sl.w + reach, h: sl.h - 2.0 * f },
                Place::Edge(Edge::Right) => crate::frame::Bx { x: sl.x, y: sl.y + f, w: sl.w + reach, h: sl.h - 2.0 * f },
                Place::Corner(c) => {
                    let l = matches!(c, Corner::TopLeft | Corner::BottomLeft);
                    let tp = matches!(c, Corner::TopLeft | Corner::TopRight);
                    crate::frame::Bx { x: if l { sl.x - reach } else { sl.x + f }, y: if tp { sl.y - reach } else { sl.y + f }, w: sl.w - f + reach, h: sl.h - f + reach }
                }
            };
            shapes.push((body, r));
            if let Place::Edge(e) = sl.place {
                shapes.push((crate::frame::Bx { x: sl.x, y: sl.y, w: sl.w, h: sl.h }, crate::frame::neck_code(e)));
            }
        }
        let list = crate::frame::liquid_list(&g.dc, &g.d2d_factory, &shapes, s, 0.0, 0.0, liquid.unwrap_or(6.0))?;
        Some((list, shapes))
    });
    // (the liquid on the graphics card, as the frame draws it; NADI_LIQUID_OLD: the blur)
    let gpu_tex = liquid.filter(|_| std::env::var("NADI_LIQUID_OLD").is_err()).and_then(|r| {
        let (_, shapes) = liquid_list.as_ref()?;
        let neck = crate::frame::liquid_neck(r);
        let mut field: Vec<(crate::frame::Bx, f32)> = shapes.iter().filter(|(_, x)| crate::frame::neck_edge(*x).is_none()).copied().collect();
        let mut spans: Vec<(Edge, f32, f32)> = Vec::new();
        for (b, x) in shapes {
            let Some(e) = crate::frame::neck_edge(*x) else { continue };
            let (a0, a1) = if matches!(e, Edge::Top | Edge::Bottom) { (b.x, b.x + b.w) } else { (b.y, b.y + b.h) };
            match spans.iter_mut().find(|(y, _, _)| *y == e) {
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
            field.push((match e {
                Edge::Top => crate::frame::Bx { x: a0, y: -40.0, w: a1 - a0, h: 40.0 },
                Edge::Bottom => crate::frame::Bx { x: a0, y: h, w: a1 - a0, h: 40.0 },
                Edge::Left => crate::frame::Bx { x: -40.0, y: a0, w: 40.0, h: a1 - a0 },
                Edge::Right => crate::frame::Bx { x: w, y: a0, w: 40.0, h: a1 - a0 },
            }, 0.0));
        }
        let mut gpu = crate::liquidgpu::LiquidGpu::new(&g.d3d)?;
        let edges_at = field.split_off(field.len() - slabs);
        // (NADI_PREVIEW_GLOW=n: the n-th merged shape glows, its line drawn round it)
        let lit_n = std::env::var("NADI_PREVIEW_GLOW").ok().and_then(|v| v.parse::<usize>().ok());
        let lit: Vec<bool> = (0..field.len()).map(|i| Some(i) == lit_n).collect();
        gpu.render(&g.d3d, pw, ph, (0.0, 0.0), s, neck, &field, &edges_at, &[], &lit, if lit_n.is_some() { [0.35, 0.78, 0.55, 1.0] } else { [0.0; 4] })
    });
    let px = g.snapshot(pw, ph, [0.17, 0.20, 0.30, 1.0], |dc| {
        let p = Painter { g: &g, dc, s, pc: &pc, t: 0.4, viz: [0.6, 0.95, 0.45, 0.5, 0.8, 0.4] };
        if let Some(tex) = &gpu_tex {
            crate::liquidgpu::draw_texture(dc, tex, pw, ph);
        } else if let (Some((list, liquid_shapes)), Some(r)) = (&liquid_list, liquid) {
            crate::frame::draw_liquid(dc, list, r * s);
            crate::frame::draw_necks(dc, &g.d2d_factory, &liquid_shapes, s, 0.0, 0.0, r);
        }
        let mut its: Vec<Item> = Vec::new();
        for (it, content, _) in &items {
            let mut it = it.clone();
            it.want = crate::peek::measure(&p, &content["peek"], it.place.horizontal());
            if let Some(d) = crate::peek::depth_of(&p, &content["peek"], it.want).filter(|_| it.place.horizontal()) {
                it.across = d;
            }
            // (large: deeper, as the frame makes it)
            let k = content["peek"]["scale"].as_f64().unwrap_or(1.0) as f32;
            if content["peek"]["dock"].is_object() {
                it.across = if content["peek"]["dock"]["large"] == true { 112.0 } else { 56.0 };
            }
            if k > 1.01 {
                it.across = ((if it.place.horizontal() { m.depth_h } else { m.depth_v }) * k).round();
            }
            if floats.contains(&it.key) {
                it.across = (if it.place.horizontal() { m.depth_h } else { m.depth_v }) + float_off;
            }
            its.push(it);
        }
        let _ = its;
        // (the open one last: it is on top)
        let mut order: Vec<&layout::Slot> = slots.iter().collect();
        order.sort_by_key(|sl| items.iter().any(|(i, _, o)| *o && i.key == sl.key));
        for sl in order {
            let Some((_, content, open)) = items.iter().find(|(i, _, _)| i.key == sl.key) else { continue };
            let f = m.fillet;
            let depth = if sl.place.horizontal() { m.depth_h } else { m.depth_v };
            let depth = (depth * content["peek"]["scale"].as_f64().unwrap_or(1.0) as f32).round();
            let depth = if content["peek"]["dock"].is_object() { if content["peek"]["dock"]["large"] == true { 112.0 } else { 56.0 } } else { depth };
            let fac = &g.d2d_factory;
            let (shape, x, y, _panel_h, content_h) = if *open && content["panel"].is_object() {
                // (measured as the frame measures it)
                // (its width as it asks, as the frame allows it; its body as big as it asks)
                let pw = content["panel"]["width"].as_f64().map_or(360.0, |v| (v as f32).clamp(120.0, (w * 0.5).max(360.0).min(640.0)));
                let zoom = content["panel"]["zoom"].as_f64().unwrap_or(1.0).clamp(0.8, 1.6) as f32;
                let inner_w = pw - 32.0;
                let mut c = crate::blocks::Ctx { p: &p, draw: false, hits: Vec::new(), open: &empty, closed: &empty, thumbs: Vec::new() };
                let head = crate::blocks::head(&mut c, &content["panel"], 0.0, 0.0, inner_w);
                let body = c.body_zoomed(content["panel"]["body"].as_array().map(|v| v.as_slice()).unwrap_or(&[]), 0.0, 0.0, inner_w, zoom);
                let ch = head + if body > 0.0 { (if head > 0.0 { 14.0 } else { 0.0 }) + body } else { 0.0 };
                let across = (ch + 32.0 + 6.0).min(h * 0.78);
                let bw = pw + 2.0 * f;
                let x = (sl.x + sl.w / 2.0 - bw / 2.0).clamp(0.0, w - bw);
                (shape::notch(fac, Edge::Top, pw * s, across * s, 24.0 * s, f * s), x, 0.0, across, ch)
            } else {
                let sh = match sl.place {
                    Place::Edge(e @ (Edge::Top | Edge::Bottom)) => shape::notch(fac, e, sl.along * s, sl.h.max(depth) * s, depth * 0.44 * s, f * s),
                    Place::Edge(e) => shape::notch(fac, e, sl.along * s, depth * s, depth * 0.44 * s, f * s),
                    Place::Corner(c) => shape::corner(fac, c, sl.along * s, depth * s, depth * 0.44 * s, f * s),
                };
                (sh, sl.x, sl.y, 0.0, 0.0)
            };
            let Ok(mut shape) = shape else { continue };
            let (mut x, mut y) = (x, y);
            let floating = floats.contains(&sl.key) && !*open;
            if floating {
                // (as the frame draws one: off its edge, its own round shape)
                let o = float_off;
                let (fx, fy, fw, fh) = match sl.place {
                    Place::Edge(Edge::Top) => (sl.x + f, sl.y + o, sl.w - 2.0 * f, depth),
                    Place::Edge(Edge::Bottom) => (sl.x + f, sl.y, sl.w - 2.0 * f, depth),
                    Place::Edge(Edge::Left) => (sl.x + o, sl.y + f, depth, sl.h - 2.0 * f),
                    Place::Edge(Edge::Right) => (sl.x, sl.y + f, depth, sl.h - 2.0 * f),
                    _ => (sl.x + o, sl.y + o, sl.w - o - f, sl.h - o - f),
                };
                let r = (if sl.place.horizontal() { fh } else { fw }).min(depth) / 2.0;
                if let Ok(sh) = shape::floating(fac, fw * s, fh * s, r * s) {
                    shape = sh;
                    x = fx;
                    y = fy;
                }
            }
            unsafe { dc.SetTransform(&Matrix3x2::translation(x * s, y * s)) };
            let t = if *open { 1.0 } else { 0.0 };
            // (in the liquid: its shape is the liquid's)
            let mut content = content.clone();
            if liquid.is_some() && !*open {
                content["\u{0}liquid"] = serde_json::json!(true);
            }
            let content = &content;
            let _ = paint_piece(&p, &g, dc, &shape, sl, content, t, &empty, &empty, 0.0, content_h, 1.0, 1.0, floating, &mut |_, _| {}, &mut |p: &Painter, body: crate::shape::Rect| {
                if content["peek"]["face"] == "eyes" {
                    let turn = match sl.place { Place::Edge(Edge::Right) => 90.0, Place::Edge(Edge::Left) => -90.0, _ => 0.0 };
                    let sc = (if turn != 0.0 { body.w } else { body.h } / 32.0).clamp(0.8, 1.6);
                    crate::eyes::Eyes::new().paint(p, body.x + body.w / 2.0, body.y + body.h / 2.0, sc, turn, &crate::director::Audio::default(), 0.0);
                }
            });
            let _ = Corner::TopLeft;
        }
        unsafe { dc.SetTransform(&Matrix3x2::identity()) };
    })?;
    gfx::save_png(out, pw, ph, &px).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
    Ok(())
}
