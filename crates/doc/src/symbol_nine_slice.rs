//! Symbol Options › Enable Guides for 9-Slice Scaling: four guides that cut a symbol into a 3 × 3
//! grid so that the corners keep their size while the edges and the centre stretch
//! ([helpx Symbol Options](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html)).
//!
//! The guides are stored as fractions (0..1) of the symbol's natural size, measured from the left
//! and top edges and from the right and bottom edges. Stored under [`SYMBOL_NINE_SLICE`] keyed by
//! symbol name. When an instance is scaled with an axis-aligned scale+translate,
//! [`SymbolNineSlice::map_point`] remaps art so corners keep their natural size.

use serde_json::{Value, json};
use vectorcraft_geom::{Affine, PathData, Point, Rect};

use crate::node::{Node, NodeKind};

/// Document unknown key: `{ symbolName: [left, top, right, bottom] }`.
pub const SYMBOL_NINE_SLICE: &str = "symbolNineSlice";

/// The four 9-slice guides as fractions of the symbol's natural size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymbolNineSlice {
    /// Left guide, from the left edge.
    pub left: f64,
    /// Top guide, from the top edge.
    pub top: f64,
    /// Right guide, from the right edge.
    pub right: f64,
    /// Bottom guide, from the bottom edge.
    pub bottom: f64,
}

impl Default for SymbolNineSlice {
    /// The centre third of each axis is the stretchy cell.
    fn default() -> Self {
        let t = 1.0 / 3.0;
        Self { left: t, top: t, right: t, bottom: t }
    }
}

/// The nine rectangles of a 9-slice grid: `[row][col]` with row 0 at the top and col 0 at the left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NineSliceCells {
    pub cells: [[Rect; 3]; 3],
}

impl SymbolNineSlice {
    /// Guides from `[left, top, right, bottom]`; `None` unless each is finite, within 0..1, and the
    /// two on each axis leave a gap between them.
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Option<Self> {
        let ok = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        (ok(left) && ok(top) && ok(right) && ok(bottom) && left + right < 1.0 && top + bottom < 1.0).then_some(Self { left, top, right, bottom })
    }

    pub fn to_value(self) -> Value {
        json!([self.left, self.top, self.right, self.bottom])
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        let n = |i: usize| a.get(i).and_then(Value::as_f64);
        if a.len() != 4 {
            return None;
        }
        Self::new(n(0)?, n(1)?, n(2)?, n(3)?)
    }

    /// Vertical guide x positions inside `bounds` (left guide, then right guide).
    pub fn guide_xs(self, bounds: Rect) -> (f64, f64) {
        let w = bounds.width();
        (bounds.x0 + self.left * w, bounds.x1 - self.right * w)
    }

    /// Horizontal guide y positions inside `bounds` (top guide, then bottom guide).
    pub fn guide_ys(self, bounds: Rect) -> (f64, f64) {
        let h = bounds.height();
        (bounds.y0 + self.top * h, bounds.y1 - self.bottom * h)
    }

    /// The 3 × 3 cell rectangles of `bounds` cut by these guides.
    pub fn cells(self, bounds: Rect) -> NineSliceCells {
        let (x0, x1) = (bounds.x0, bounds.x1);
        let (y0, y1) = (bounds.y0, bounds.y1);
        let (xl, xr) = self.guide_xs(bounds);
        let (yt, yb) = self.guide_ys(bounds);
        let row = |ya: f64, yb: f64| [Rect::new(x0, ya, xl, yb), Rect::new(xl, ya, xr, yb), Rect::new(xr, ya, x1, yb)];
        NineSliceCells { cells: [row(y0, yt), row(yt, yb), row(yb, y1)] }
    }

    /// Map a scalar along one axis: corners keep source size when `dst` is large enough.
    pub fn map_axis(src: f64, dst: f64, a: f64, b: f64, t: f64) -> f64 {
        if !(src.is_finite() && dst.is_finite() && a.is_finite() && b.is_finite() && t.is_finite()) || src <= 0.0 {
            return if src > 0.0 && src.is_finite() && dst.is_finite() { t * (dst / src) } else { t };
        }
        let left = (a * src).clamp(0.0, src);
        let right = (b * src).clamp(0.0, src - left);
        let mid = src - left - right;
        let (left_d, mid_d, right_d) = if dst + f64::EPSILON >= left + right {
            (left, dst - left - right, right)
        } else if left + right <= f64::EPSILON {
            (0.0, dst, 0.0)
        } else {
            let scale = dst / (left + right);
            (left * scale, 0.0, right * scale)
        };
        let t = t.clamp(0.0, src);
        if t <= left {
            if left <= f64::EPSILON {
                0.0
            } else {
                t * (left_d / left)
            }
        } else if t >= src - right {
            if right <= f64::EPSILON {
                dst
            } else {
                left_d + mid_d + (t - (src - right)) * (right_d / right)
            }
        } else if mid <= f64::EPSILON {
            left_d
        } else {
            left_d + (t - left) * (mid_d / mid)
        }
    }

    /// Map `p` from `src` bounds into `dst` bounds with 9-slice scaling.
    pub fn map_point(self, src: Rect, dst: Rect, p: Point) -> Point {
        let x = Self::map_axis(src.width(), dst.width(), self.left, self.right, p.x - src.x0) + dst.x0;
        let y = Self::map_axis(src.height(), dst.height(), self.top, self.bottom, p.y - src.y0) + dst.y0;
        Point::new(x, y)
    }

    /// Remap every anchor and handle of `path` from `src` into `dst`.
    pub fn map_path(self, src: Rect, dst: Rect, path: &mut PathData) {
        for sp in &mut path.subpaths {
            for an in &mut sp.anchors {
                an.p = self.map_point(src, dst, an.p);
                an.h_in = self.map_point(src, dst, an.h_in);
                an.h_out = self.map_point(src, dst, an.h_out);
            }
        }
    }

    /// Remap path geometry under `node` from `src` into `dst`.
    pub fn remap_node(self, src: Rect, dst: Rect, node: &mut Node) {
        match &mut node.kind {
            NodeKind::Path { path, live, .. } => {
                self.map_path(src, dst, path);
                *live = None;
            }
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Intertwine { children }
            | NodeKind::Blend { children, .. }
            | NodeKind::Envelope { content: children, .. }
            | NodeKind::Repeat(crate::RepeatSpec { source: children, .. }) => {
                for c in children.iter_mut() {
                    self.remap_node(src, dst, std::sync::Arc::make_mut(c));
                }
            }
            _ => {
                if let Some(b) = node.geometric_bounds() {
                    let nb = Rect::from_points(self.map_point(src, dst, Point::new(b.x0, b.y0)), self.map_point(src, dst, Point::new(b.x1, b.y1)));
                    if b.width() > 1e-12 && b.height() > 1e-12 && nb.width().is_finite() && nb.height().is_finite() {
                        let a = Affine::translate((nb.x0, nb.y0))
                            * Affine::scale_non_uniform(nb.width() / b.width(), nb.height() / b.height())
                            * Affine::translate((-b.x0, -b.y0));
                        node.transform(a, false);
                    }
                }
            }
        }
    }
}

/// Destination bounds of `src` under `xf` when `xf` is only an axis-aligned scale and translate.
pub fn axis_aligned_dst(src: Rect, xf: Affine) -> Option<Rect> {
    let [a, b, c, d, _, _] = xf.as_coeffs();
    if !a.is_finite() || !b.is_finite() || !c.is_finite() || !d.is_finite() {
        return None;
    }
    if b.abs() > 1e-9 || c.abs() > 1e-9 {
        return None;
    }
    let dst = xf.transform_rect_bbox(src);
    ([dst.x0, dst.y0, dst.x1, dst.y1].iter().all(|v| v.is_finite()) && dst.width().abs() > 1e-12 && dst.height().abs() > 1e-12).then_some(dst)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use vectorcraft_geom::SubPath;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    fn approx_pt(a: Point, b: Point) {
        approx(a.x, b.x);
        approx(a.y, b.y);
    }

    #[test]
    fn cells_split_a_square_into_nine_with_default_thirds() {
        let g = SymbolNineSlice::default();
        let b = Rect::new(0.0, 0.0, 90.0, 90.0);
        let c = g.cells(b);
        assert_eq!(c.cells[0][0], Rect::new(0.0, 0.0, 30.0, 30.0));
        assert_eq!(c.cells[0][1], Rect::new(30.0, 0.0, 60.0, 30.0));
        assert_eq!(c.cells[0][2], Rect::new(60.0, 0.0, 90.0, 30.0));
        assert_eq!(c.cells[1][1], Rect::new(30.0, 30.0, 60.0, 60.0));
        assert_eq!(c.cells[2][2], Rect::new(60.0, 60.0, 90.0, 90.0));
        assert_eq!(g.guide_xs(b), (30.0, 60.0));
        assert_eq!(g.guide_ys(b), (30.0, 60.0));
    }

    #[test]
    fn map_axis_keeps_corners_and_stretches_the_middle() {
        approx(SymbolNineSlice::map_axis(90.0, 180.0, 1.0 / 3.0, 1.0 / 3.0, 0.0), 0.0);
        approx(SymbolNineSlice::map_axis(90.0, 180.0, 1.0 / 3.0, 1.0 / 3.0, 30.0), 30.0);
        approx(SymbolNineSlice::map_axis(90.0, 180.0, 1.0 / 3.0, 1.0 / 3.0, 60.0), 150.0);
        approx(SymbolNineSlice::map_axis(90.0, 180.0, 1.0 / 3.0, 1.0 / 3.0, 90.0), 180.0);
        approx(SymbolNineSlice::map_axis(90.0, 180.0, 1.0 / 3.0, 1.0 / 3.0, 45.0), 90.0);
    }

    #[test]
    fn map_point_keeps_corner_offsets_when_widening() {
        let g = SymbolNineSlice::default();
        let src = Rect::new(0.0, 0.0, 90.0, 90.0);
        let dst = Rect::new(0.0, 0.0, 180.0, 90.0);
        approx_pt(g.map_point(src, dst, Point::new(10.0, 10.0)), Point::new(10.0, 10.0));
        approx_pt(g.map_point(src, dst, Point::new(80.0, 10.0)), Point::new(170.0, 10.0));
        approx_pt(g.map_point(src, dst, Point::new(45.0, 45.0)), Point::new(90.0, 45.0));
    }

    #[test]
    fn axis_aligned_dst_accepts_scale_translate_and_rejects_rotation() {
        let src = Rect::new(0.0, 0.0, 20.0, 20.0);
        let ok = Affine::translate((5.0, 7.0)) * Affine::scale_non_uniform(2.0, 3.0);
        let dst = axis_aligned_dst(src, ok).unwrap();
        approx(dst.x0, 5.0);
        approx(dst.y0, 7.0);
        approx(dst.x1, 45.0);
        approx(dst.y1, 67.0);
        assert!(axis_aligned_dst(src, Affine::rotate(0.3)).is_none());
    }

    #[test]
    fn map_path_keeps_corner_square_size_when_widening() {
        let g = SymbolNineSlice::default();
        let src = Rect::new(0.0, 0.0, 90.0, 90.0);
        let dst = Rect::new(0.0, 0.0, 180.0, 90.0);
        let mut path = PathData::single(SubPath::polyline(
            &[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
            true,
        ));
        g.map_path(src, dst, &mut path);
        let b = path.bounds().unwrap();
        approx(b.width(), 10.0);
        approx(b.height(), 10.0);
        approx(b.x0, 0.0);
        approx(b.y0, 0.0);
    }
}
