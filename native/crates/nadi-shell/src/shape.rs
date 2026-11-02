//! The shapes of the frame. A piece is merged into an edge (a notch: flush with the edge, rounded on the side that faces the screen, its
//! corners curving into the edge), into a corner (flush with two edges, rounded on its inner corner, curving into both), or floating
//! (its own rounded shape). Each is built once in a canonical place (an edge piece on the top edge, a corner piece in the top-left) and
//! turned to where it goes; the body (where its content is drawn, unturned) is given in the piece's own coordinates.

use windows::core::Result;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// A shape, built: its outline (to fill), how big the box round it is (with the curves into the edge), and where its content goes.
pub struct Shape {
    pub geometry: ID2D1Geometry,
    pub w: f32,
    pub h: f32,
    pub body: Rect,
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

fn arc(sink: &ID2D1GeometrySink, to: D2D_POINT_2F, r: f32, clockwise: bool) {
    unsafe {
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: to,
            size: D2D_SIZE_F { width: r, height: r },
            rotationAngle: 0.0,
            sweepDirection: if clockwise { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
    }
}

fn path(factory: &ID2D1Factory1, build: impl FnOnce(&ID2D1GeometrySink)) -> Result<ID2D1PathGeometry1> {
    unsafe {
        let g = factory.CreatePathGeometry()?;
        let sink = g.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        build(&sink);
        sink.Close()?;
        Ok(g)
    }
}

fn turned(factory: &ID2D1Factory1, g: &ID2D1PathGeometry1, m: Matrix3x2) -> Result<ID2D1Geometry> {
    unsafe { Ok(factory.CreateTransformedGeometry(g, &m)?.into()) }
}

/// A piece merged into an edge: `along` is its length along the edge, `across` how far it comes out; `radius` rounds the side facing the
/// screen, `fillet` is the curve into the edge (the box is `along + 2 × fillet` long).
pub fn notch(factory: &ID2D1Factory1, edge: Edge, along: f32, across: f32, radius: f32, fillet: f32) -> Result<Shape> {
    let f = fillet.max(0.0);
    // (its corners as round as the setting says; its fillets into the edge as they are: the liquid's)
    let r = (radius * crate::paint::roundness()).min(along / 2.0).min((across - f).max(0.0)).max(0.0);
    let (len, dep) = (along + 2.0 * f, across);
    // the canonical piece, on the top edge, traversed down its left side, along its bottom, up its right side
    let g = path(factory, |s| unsafe {
        s.BeginFigure(pt(0.0, 0.0), D2D1_FIGURE_BEGIN_FILLED);
        arc(s, pt(f, f), f, true);
        s.AddLine(pt(f, dep - r));
        arc(s, pt(f + r, dep), r, false);
        s.AddLine(pt(f + along - r, dep));
        arc(s, pt(f + along, dep - r), r, false);
        s.AddLine(pt(f + along, f));
        arc(s, pt(len, 0.0), f, true);
        s.EndFigure(D2D1_FIGURE_END_CLOSED);
    })?;
    // turned to its edge: (u along the edge, v away from it)
    let (m, w, h, body) = match edge {
        Edge::Top => (Matrix3x2::identity(), len, dep, Rect::new(f, 0.0, along, dep)),
        Edge::Bottom => (Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: -1.0, M31: 0.0, M32: dep }, len, dep, Rect::new(f, 0.0, along, dep)),
        Edge::Left => (Matrix3x2 { M11: 0.0, M12: 1.0, M21: 1.0, M22: 0.0, M31: 0.0, M32: 0.0 }, dep, len, Rect::new(0.0, f, dep, along)),
        Edge::Right => (Matrix3x2 { M11: 0.0, M12: 1.0, M21: -1.0, M22: 0.0, M31: dep, M32: 0.0 }, dep, len, Rect::new(0.0, f, dep, along)),
    };
    Ok(Shape { geometry: turned(factory, &g, m)?, w, h, body })
}

/// A piece in a corner: `w` × `h`, rounded by `radius` on the corner that faces the screen, curving into both edges by `fillet` (the box is
/// `w + fillet` × `h + fillet`).
pub fn corner(factory: &ID2D1Factory1, which: Corner, w: f32, h: f32, radius: f32, fillet: f32) -> Result<Shape> {
    let f = fillet.max(0.0);
    let r = (radius * crate::paint::roundness()).min(w).min(h).max(0.0);
    // the canonical piece, in the top-left corner, traversed along the top edge, down its right side, along its bottom, up the left edge
    let g = path(factory, |s| unsafe {
        s.BeginFigure(pt(0.0, 0.0), D2D1_FIGURE_BEGIN_FILLED);
        s.AddLine(pt(w + f, 0.0));
        arc(s, pt(w, f), f, false);
        s.AddLine(pt(w, h - r));
        arc(s, pt(w - r, h), r, true);
        s.AddLine(pt(f, h));
        arc(s, pt(0.0, h + f), f, false);
        s.EndFigure(D2D1_FIGURE_END_CLOSED);
    })?;
    let (bw, bh) = (w + f, h + f);
    let (m, body) = match which {
        Corner::TopLeft => (Matrix3x2::identity(), Rect::new(0.0, 0.0, w, h)),
        Corner::TopRight => (Matrix3x2 { M11: -1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: bw, M32: 0.0 }, Rect::new(f, 0.0, w, h)),
        Corner::BottomLeft => (Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: -1.0, M31: 0.0, M32: bh }, Rect::new(0.0, f, w, h)),
        Corner::BottomRight => (Matrix3x2 { M11: -1.0, M12: 0.0, M21: 0.0, M22: -1.0, M31: bw, M32: bh }, Rect::new(f, f, w, h)),
    };
    Ok(Shape { geometry: turned(factory, &g, m)?, w: bw, h: bh, body })
}

/// A floating piece: its own rounded shape, `w` × `h`.
pub fn floating(factory: &ID2D1Factory1, w: f32, h: f32, radius: f32) -> Result<Shape> {
    let r = crate::paint::rounded(radius.min(w / 2.0).min(h / 2.0), w, h);
    let g = unsafe { factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: h }, radiusX: r, radiusY: r })? };
    Ok(Shape { geometry: g.into(), w, h, body: Rect::new(0.0, 0.0, w, h) })
}
