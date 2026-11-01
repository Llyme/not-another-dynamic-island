//! Where each widget goes on a monitor. Widgets are put on an edge (top, bottom, left, right) or in a corner; on an edge they stand in a
//! row in the order of their rank, centred on the edge (or on both sides of a hole the edge keeps, such as the dock's), with a gap
//! between them, and never on a corner's piece. They never overlap: when an edge has more than it can hold, the last ones (by rank)
//! shrink to a dot, and when even dots do not fit, the last ones are not shown. All in logical px; the frame scales.

use crate::shape::{Corner, Edge};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    Edge(Edge),
    Corner(Corner),
}

impl Place {
    pub fn parse(s: &str) -> Option<Place> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "top" => Place::Edge(Edge::Top),
            "bottom" => Place::Edge(Edge::Bottom),
            "left" => Place::Edge(Edge::Left),
            "right" => Place::Edge(Edge::Right),
            "top-left" => Place::Corner(Corner::TopLeft),
            "top-right" => Place::Corner(Corner::TopRight),
            "bottom-left" => Place::Corner(Corner::BottomLeft),
            "bottom-right" => Place::Corner(Corner::BottomRight),
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Place::Edge(Edge::Top) => "top",
            Place::Edge(Edge::Bottom) => "bottom",
            Place::Edge(Edge::Left) => "left",
            Place::Edge(Edge::Right) => "right",
            Place::Corner(Corner::TopLeft) => "top-left",
            Place::Corner(Corner::TopRight) => "top-right",
            Place::Corner(Corner::BottomLeft) => "bottom-left",
            Place::Corner(Corner::BottomRight) => "bottom-right",
        }
    }

    /// along a horizontal edge (a corner's piece lies along the top or the bottom edge)
    pub fn horizontal(self) -> bool {
        !matches!(self, Place::Edge(Edge::Left | Edge::Right))
    }
}

/// How the frame is measured (logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// how far a piece comes out of a horizontal edge, and out of a vertical one
    pub depth_h: f32,
    pub depth_v: f32,
    /// the curve of a merged piece into its edge (each end)
    pub fillet: f32,
    /// between two pieces on an edge
    pub gap: f32,
    /// a corner's piece is at most this long (long enough for a large tray, or a dock, in a corner's row)
    pub corner_max: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics { depth_h: 32.0, depth_v: 40.0, fillet: 10.0, gap: 6.0, corner_max: 640.0 }
    }
}

/// A widget to place: how long it would like to be along its edge (its peek, measured), and its rank.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: String,
    pub place: Place,
    pub rank: i32,
    /// along the edge, the body alone (without the fillets)
    pub want: f32,
    /// how far it comes out of its edge (0: the edge's own depth; more for an open panel, which pushes what is beside it)
    pub across: f32,
    /// in a corner, not the first there: in the column down its side edge (else in the row along its top or bottom edge)
    pub down: bool,
    /// against this one (its key), no gap between, when it is the one just before it there (another between them, or it gone:
    /// apart as any other)
    pub join: Option<String>,
}

/// the gap between two joined ones: less than none, their shoulders (the fillets where each meets the edge) over each other, so
/// their bodies touch

/// Where a widget went: its box (with the fillets), in the monitor, and how long its body is along the edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub key: String,
    pub place: Place,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub along: f32,
    /// shrunk to a dot (its icon alone)
    pub dot: bool,
}

/// Lay the widgets out on a monitor `width` × `height`; `holes` keeps a stretch in the middle of an edge free (its length).
pub fn layout(items: &[Item], width: f32, height: f32, m: Metrics, holes: &[(Edge, f32)]) -> Vec<Slot> {
    let mut out = Vec::new();
    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.key.cmp(&b.key)));
    // the corners: the first by rank in the corner itself; the others in a row beside it along its horizontal edge, or in a column
    // under (or over) it along its side edge, outward (while there is room: then on along that edge with the rest, at its end
    // nearest that corner)
    // (each corner: how far its row and its column reach, and the last in each)
    let mut taken: Vec<(Corner, f32, String)> = Vec::new();
    let mut tall: Vec<(Corner, f32, String)> = Vec::new();
    let mut spill: Vec<Item> = Vec::new();
    for it in sorted.iter().filter(|i| matches!(i.place, Place::Corner(_))) {
        let Place::Corner(c) = it.place else { continue };
        let left = matches!(c, Corner::TopLeft | Corner::BottomLeft);
        let top = matches!(c, Corner::TopLeft | Corner::TopRight);
        let edge = if top { Edge::Top } else { Edge::Bottom };
        let side = if left { Edge::Left } else { Edge::Right };
        if it.down && taken.iter().any(|(t, _, _)| *t == c) {
            let col = tall.iter_mut().find(|(t, _, _)| *t == c).unwrap();
            let along = it.want.max(m.depth_v);
            let (bw, bh) = (it.across.max(m.depth_v), along + 2.0 * m.fillet);
            let gap = if it.join.as_deref() == Some(col.2.as_str()) { -2.0 * m.fillet } else { m.gap };
            if col.1 + gap + bh > height * 0.45 {
                spill.push(Item { key: it.key.clone(), place: Place::Edge(side), rank: if top { i32::MIN + spill.len() as i32 } else { i32::MAX - spill.len() as i32 }, want: it.want, across: it.across, down: false, join: None });
                continue;
            }
            let x = if left { 0.0 } else { width - bw };
            let y = if top { col.1 + gap } else { height - col.1 - gap - bh };
            col.1 += gap + bh;
            col.2 = it.key.clone();
            out.push(Slot { key: it.key.clone(), place: Place::Edge(side), x, y, w: bw, h: bh, along, dot: false });
            continue;
        }
        if let Some(run) = taken.iter_mut().find(|(t, _, _)| *t == c) {
            let along = it.want.max(m.depth_h);
            let (bw, bh) = (along + 2.0 * m.fillet, it.across.max(m.depth_h));
            let gap = if it.join.as_deref() == Some(run.2.as_str()) { -2.0 * m.fillet } else { m.gap };
            if run.1 + gap + bw > width * 0.45 {
                spill.push(Item { key: it.key.clone(), place: Place::Edge(edge), rank: if left { i32::MIN + spill.len() as i32 } else { i32::MAX - spill.len() as i32 }, want: it.want, across: it.across, down: false, join: None });
                continue;
            }
            let x = if left { run.1 + gap } else { width - run.1 - gap - bw };
            let y = if top { 0.0 } else { height - bh };
            run.1 += gap + bw;
            run.2 = it.key.clone();
            out.push(Slot { key: it.key.clone(), place: Place::Edge(edge), x, y, w: bw, h: bh, along, dot: false });
            continue;
        }
        let w = it.want.max(m.depth_h);
        let (bw, bh) = (w + m.fillet, it.across.max(m.depth_h) + m.fillet);
        let x = if left { 0.0 } else { width - bw };
        let y = if top { 0.0 } else { height - bh };
        taken.push((c, bw, it.key.clone()));
        tall.push((c, bh, it.key.clone()));
        out.push(Slot { key: it.key.clone(), place: it.place, x, y, w: bw, h: bh, along: w, dot: false });
    }
    let corner = |c: Corner| taken.iter().find(|(t, _, _)| *t == c).map_or(0.0, |(_, w, _)| *w);
    let corner_h = |c: Corner| tall.iter().find(|(t, _, _)| *t == c).map_or(0.0, |(_, h, _)| *h);
    // (the edge with the biggest open panel first: what it holds stands where it wants, and the others keep clear of it)
    let reach = |e: Edge| sorted.iter().filter(|i| i.place == Place::Edge(e)).map(|i| i.across).fold(0.0f32, f32::max);
    let mut edges = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right];
    edges.sort_by(|a, b| reach(*b).total_cmp(&reach(*a)));
    for edge in edges {
        let mut row: Vec<Item> = sorted.iter().filter(|i| i.place == Place::Edge(edge)).map(|i| (*i).clone()).collect();
        row.extend(spill.iter().filter(|i| i.place == Place::Edge(edge)).cloned());
        row.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.key.cmp(&b.key)));
        if row.is_empty() {
            continue;
        }
        let horizontal = matches!(edge, Edge::Top | Edge::Bottom);
        let (len, depth) = if horizontal { (width, m.depth_h) } else { (height, m.depth_v) };
        // the stretch it may use: past the corners' pieces (a vertical edge keeps clear of the top and bottom pieces' depth)
        let (from, to) = match edge {
            Edge::Top => (corner(Corner::TopLeft) + m.gap, width - corner(Corner::TopRight) - m.gap),
            Edge::Bottom => (corner(Corner::BottomLeft) + m.gap, width - corner(Corner::BottomRight) - m.gap),
            Edge::Left => ((m.depth_h + m.fillet).max(corner_h(Corner::TopLeft)) + m.gap, height - (m.depth_h + m.fillet).max(corner_h(Corner::BottomLeft)) - m.gap),
            Edge::Right => ((m.depth_h + m.fillet).max(corner_h(Corner::TopRight)) + m.gap, height - (m.depth_h + m.fillet).max(corner_h(Corner::BottomRight)) - m.gap),
        };
        // what stands on this edge's strip already (another edge's open panel): what is on the near half keeps the row after it, what
        // is on the far half before it
        let thick = row.iter().map(|i| i.across.max(depth)).fold(depth, f32::max);
        let (mut from, mut to) = (from, to);
        for o in out.iter().filter(|o| o.place != Place::Edge(edge)) {
            let (on_strip, a0, a1) = match edge {
                Edge::Top => (o.y < thick, o.x, o.x + o.w),
                Edge::Bottom => (o.y + o.h > height - thick, o.x, o.x + o.w),
                Edge::Left => (o.x < thick, o.y, o.y + o.h),
                Edge::Right => (o.x + o.w > width - thick, o.y, o.y + o.h),
            };
            if !on_strip {
                continue;
            }
            if (a0 + a1) / 2.0 < len / 2.0 {
                from = from.max(a1 + m.gap);
            } else {
                to = to.min(a0 - m.gap);
            }
        }
        let hole = holes.iter().find(|(e, _)| *e == edge).map_or(0.0, |(_, l)| *l);
        let foot = |along: f32| along + 2.0 * m.fillet;
        // what each takes: its want, or a dot. Too many: the last (by rank) shrink to dots, then go; an open panel last of all (the
        // others give way to it), and then it narrows to what there is
        // (at least as long as the edge is deep; one that says how deep it is (a square icon, floating or not), as long as it asks)
        let mut wants: Vec<f32> = row.iter().map(|i| if i.across > 0.0 { i.want.max(1.0) } else { i.want.max(depth) }).collect();
        let mut dots = vec![false; row.len()];
        let mut on = vec![true; row.len()];
        let open: Vec<bool> = row.iter().map(|i| i.across > depth + 0.5).collect();
        // (the gap before each that is out: none before the first, a joined one's none (joined to the one out just before it),
        // else the edge's)
        let join_gap = -2.0 * m.fillet;
        let joined = |i: usize, prev: Option<usize>| prev.is_some_and(|p| row[i].join.as_deref() == Some(row[p].key.as_str()));
        let total = |w: &[f32], on: &[bool]| {
            let mut sum = 0.0;
            let mut prev: Option<usize> = None;
            for i in 0..w.len() {
                if !on[i] {
                    continue;
                }
                if prev.is_some() {
                    sum += if joined(i, prev) { join_gap } else { m.gap };
                }
                prev = Some(i);
                sum += foot(w[i]);
            }
            sum
        };
        let room = (to - from - if hole > 0.0 { hole + 2.0 * m.gap } else { 0.0 }).max(0.0);
        for k in (0..row.len()).rev() {
            if total(&wants, &on) <= room {
                break;
            }
            if !open[k] {
                wants[k] = depth;
                dots[k] = true;
            }
        }
        for k in (0..row.len()).rev() {
            if total(&wants, &on) <= room {
                break;
            }
            if !open[k] {
                on[k] = false;
            }
        }
        for k in (0..row.len()).rev() {
            let over = total(&wants, &on) - room;
            if over <= 0.0 {
                break;
            }
            if open[k] && on[k] {
                let less = wants[k] - over;
                if less >= depth * 2.0 {
                    wants[k] = less;
                } else {
                    on[k] = false;
                }
            }
        }
        let kept: Vec<usize> = (0..row.len()).filter(|i| on[*i]).collect();
        let mut put = |i: usize, start: f32| {
            let along = wants[i];
            // (as deep as it asks, when it asks: an open panel deeper, a square icon on a side edge less deep)
            let d = if dots[i] { depth } else if row[i].across > 0.0 { row[i].across } else { depth };
            let (x, y, w, h) = match edge {
                Edge::Top => (start, 0.0, foot(along), d),
                Edge::Bottom => (start, height - d, foot(along), d),
                Edge::Left => (0.0, start, d, foot(along)),
                Edge::Right => (width - d, start, d, foot(along)),
            };
            out.push(Slot { key: row[i].key.clone(), place: Place::Edge(edge), x, y, w, h, along, dot: dots[i] });
        };
        if hole > 0.0 {
            // on both sides of the hole: the first half by rank on the near side, outward from it
            let mid = len / 2.0;
            let (mut near, mut far) = (mid - hole / 2.0 - m.gap, mid + hole / 2.0 + m.gap);
            for (j, &i) in kept.iter().enumerate() {
                let f = foot(wants[i]);
                if j % 2 == 0 && near - f >= from {
                    near -= f;
                    put(i, near);
                    near -= m.gap;
                } else if far + f <= to {
                    put(i, far);
                    far += f + m.gap;
                } else if near - f >= from {
                    near -= f;
                    put(i, near);
                    near -= m.gap;
                }
            }
        } else {
            // a row, centred on the edge, kept inside what it may use
            let t = total(&wants, &on);
            let mut start = (len / 2.0 - t / 2.0).clamp(from, (to - t).max(from));
            for (j, &i) in kept.iter().enumerate() {
                if j > 0 {
                    start += if joined(i, Some(kept[j - 1])) { join_gap } else { m.gap };
                }
                put(i, start);
                start += foot(wants[i]);
            }
        }
    }
    out
}

/// Do two slots overlap?
#[cfg(test)]
pub fn overlap(a: &Slot, b: &Slot) -> bool {
    a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01 && a.y < b.y + b.h - 0.01 && b.y < a.y + a.h - 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, place: &str, rank: i32, want: f32) -> Item {
        Item { key: key.into(), place: Place::parse(place).unwrap(), rank, want, across: 0.0, down: false, join: None }
    }

    fn none_overlap(s: &[Slot]) {
        for (i, a) in s.iter().enumerate() {
            for b in &s[i + 1..] {
                assert!(!overlap(a, b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn widgets_on_an_edge_stand_in_a_centred_row_by_rank() {
        let s = layout(&[item("b", "top", 2, 100.0), item("a", "top", 1, 200.0)], 1920.0, 1080.0, Metrics::default(), &[]);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].key.as_str(), s[1].key.as_str()), ("a", "b"));
        // (200 + 20, a gap of 6, 100 + 20: 346 long, centred)
        assert_eq!(s[0].x, 960.0 - 173.0);
        assert_eq!(s[1].x, s[0].x + 220.0 + 6.0);
        assert_eq!((s[0].y, s[0].h), (0.0, 32.0));
        none_overlap(&s);
    }

    #[test]
    fn too_many_shrink_to_dots_then_the_last_go() {
        let items: Vec<Item> = (0..8).map(|i| item(&format!("w{i}"), "left", i, 200.0)).collect();
        let s = layout(&items, 1920.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s);
        assert!(s.iter().any(|x| x.dot), "some are dots");
        assert!(!s[0].dot, "the first keeps its size");
        assert!(s.iter().all(|x| x.y >= 32.0 && x.y + x.h <= 1080.0 - 32.0), "clear of the top and the bottom");
        let tiny: Vec<Item> = (0..60).map(|i| item(&format!("w{i}"), "left", i, 200.0)).collect();
        let s = layout(&tiny, 1920.0, 1080.0, Metrics::default(), &[]);
        assert!(s.len() < 60, "the last are not shown");
        none_overlap(&s);
    }

    #[test]
    fn a_corner_holds_several_in_a_row_and_the_edge_keeps_clear_of_them() {
        let items = vec![item("c", "top-left", 1, 180.0), item("d", "top-left", 2, 90.0), item("e", "top-right", 1, 100.0), item("f", "top-right", 2, 100.0), item("t", "top", 1, 1700.0)];
        let s = layout(&items, 1920.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s);
        let c = s.iter().find(|x| x.key == "c").unwrap();
        assert_eq!((c.x, c.y, c.w), (0.0, 0.0, 190.0));
        // the second in that corner stands beside the first, along the top edge
        let d = s.iter().find(|x| x.key == "d").unwrap();
        assert_eq!(d.place, Place::Edge(Edge::Top));
        assert_eq!(d.x, 196.0);
        // (and in a right corner, to its left)
        let (e, f) = (s.iter().find(|x| x.key == "e").unwrap(), s.iter().find(|x| x.key == "f").unwrap());
        assert_eq!(e.x + e.w, 1920.0);
        assert_eq!(f.x + f.w, e.x - 6.0);
        // the edge's own row keeps clear of both runs
        let t = s.iter().find(|x| x.key == "t").unwrap();
        assert!(t.x >= d.x + d.w + 6.0 - 0.01 && t.x + t.w <= f.x - 6.0 + 0.01);
    }

    #[test]
    fn a_corner_holds_a_row_and_a_column_at_once() {
        let down = |k: &str, rank: i32| Item { down: true, ..item(k, "top-left", rank, 80.0) };
        let items = vec![item("c", "top-left", 1, 180.0), item("r", "top-left", 2, 90.0), down("d1", 3), down("d2", 4), item("l", "left", 1, 100.0)];
        let s = layout(&items, 1920.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s);
        let get = |k: &str| s.iter().find(|x| x.key == k).unwrap().clone();
        let (c, r, d1, d2, l) = (get("c"), get("r"), get("d1"), get("d2"), get("l"));
        assert_eq!(r.place, Place::Edge(Edge::Top));
        assert_eq!(r.x, c.x + c.w + 6.0);
        // (the column: under the corner's piece, against the left edge, in rank order)
        assert_eq!((d1.place, d1.x), (Place::Edge(Edge::Left), 0.0));
        assert_eq!(d1.y, c.y + c.h + 6.0);
        assert_eq!(d2.y, d1.y + d1.h + 6.0);
        // the left edge's own row keeps clear of the column
        assert!(l.y >= d2.y + d2.h + 6.0 - 0.01);
        // (in a bottom-right corner: over its piece, against the right edge)
        let items = vec![item("p", "bottom-right", 1, 100.0), Item { down: true, ..item("q", "bottom-right", 2, 80.0) }];
        let s = layout(&items, 1920.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s);
        let (p, q) = (s.iter().find(|x| x.key == "p").unwrap(), s.iter().find(|x| x.key == "q").unwrap());
        assert_eq!(q.x + q.w, 1920.0);
        assert_eq!(q.y + q.h, p.y - 6.0);
    }

    #[test]
    fn on_a_small_screen_the_others_give_way_to_an_open_panel() {
        // (the last by rank is open: the others shrink to dots and go before it does)
        let mut items = vec![item("a", "top", 1, 300.0), item("b", "top", 2, 300.0), item("c", "top", 3, 500.0)];
        items[2].across = 400.0;
        let s = layout(&items, 900.0, 600.0, Metrics::default(), &[]);
        let c = s.iter().find(|x| x.key == "c").expect("the open one stays");
        assert_eq!(c.along, 500.0);
        assert!(s.iter().filter(|x| x.key != "c").all(|x| x.dot), "the others are dots or gone: {s:?}");
        none_overlap(&s);
        // (narrower still: they go, and it narrows to what there is)
        let s = layout(&items, 420.0, 600.0, Metrics::default(), &[]);
        assert_eq!(s.len(), 1);
        assert!(s[0].key == "c" && s[0].along < 500.0);
    }

    #[test]
    fn an_open_panel_pushes_its_neighbours_aside() {
        let shut = vec![item("a", "top", 1, 100.0), item("b", "top", 2, 100.0), item("c", "top", 3, 100.0)];
        let s0 = layout(&shut, 1920.0, 1080.0, Metrics::default(), &[]);
        let mut open = shut.clone();
        open[1].want = 360.0;
        open[1].across = 300.0;
        let s1 = layout(&open, 1920.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s1);
        assert!(s1[0].x < s0[0].x && s1[2].x > s0[2].x, "pushed aside");
        assert_eq!((s1[1].w, s1[1].h), (380.0, 300.0));
        // (a tall corner panel pushes the side edge down)
        let c = vec![Item { across: 400.0, ..item("k", "top-left", 1, 360.0) }, item("l", "left", 1, 100.0)];
        let s2 = layout(&c, 1920.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s2);
        assert!(s2.iter().find(|x| x.key == "l").unwrap().y >= 410.0);
    }

    #[test]
    fn an_open_panel_pushes_the_widgets_of_other_edges_too() {
        // (a tall panel on the top edge, near its left end, and widgets on the left edge: they move down past it)
        let mut items = vec![item("t", "top", 1, 360.0), item("l1", "left", 1, 100.0), item("l2", "left", 2, 100.0)];
        items[0].across = 700.0;
        let s = layout(&items, 400.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s);
        let t = s.iter().find(|x| x.key == "t").unwrap();
        for k in ["l1", "l2"] {
            let l = s.iter().find(|x| x.key == k).unwrap();
            assert!(l.y >= t.y + t.h, "{k} is below the panel");
        }
        // (an open side panel, wide: the top edge keeps clear of it)
        let mut items = vec![item("l", "left", 1, 900.0), item("a", "top", 1, 300.0), item("b", "top", 2, 300.0)];
        items[0].across = 600.0;
        let s = layout(&items, 1200.0, 1080.0, Metrics::default(), &[]);
        none_overlap(&s);
    }

    #[test]
    fn a_hole_is_kept_free_and_widgets_stand_on_both_sides() {
        let items = vec![item("a", "bottom", 1, 100.0), item("b", "bottom", 2, 100.0), item("c", "bottom", 3, 100.0)];
        let s = layout(&items, 1920.0, 1080.0, Metrics::default(), &[(Edge::Bottom, 600.0)]);
        none_overlap(&s);
        for x in &s {
            assert!(x.x + x.w <= 660.0 - 6.0 + 0.01 || x.x >= 1260.0 + 6.0 - 0.01, "{x:?} in the hole");
            assert_eq!(x.y, 1080.0 - 32.0);
        }
        assert!(s[0].x < 960.0 && s[1].x > 960.0);
    }
}
