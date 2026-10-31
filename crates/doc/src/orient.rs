//! Rotated bounding boxes. Every object keeps the angle of its own axes ([`Node::bbox_angle`])
//! through the transforms applied to it, so after a rotation its bounding box and handles stay
//! square to it: scaling by a side handle stretches it along its own width or height. Path
//! geometry is still baked; Object ▸ Transform ▸ Reset Bounding Box sets the angle back to 0.

use vectorcraft_geom::{Affine, Point, Rect, Vec2, normalize_deg, union_opt};

use crate::{Document, Node, NodeId, NodeKind, SymbolBoxes};

/// Angles closer than this (degrees) are the same.
const EPS_DEG: f64 = 1e-6;

/// A bounding box square to a frame turned by `angle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrientedBox {
    /// Counter-clockwise degrees in (-180, 180]; 0: square to the page.
    pub angle: f64,
    /// The box in the turned frame: [`OrientedBox::to_doc`] places it on the page.
    pub rect: Rect,
}

impl OrientedBox {
    /// A box square to the page.
    pub fn aligned(rect: Rect) -> Self {
        Self { angle: 0.0, rect }
    }
    /// The turned frame → the page (a rotation about the page origin).
    pub fn to_doc(&self) -> Affine {
        if self.angle == 0.0 { Affine::IDENTITY } else { Affine::rotate(-self.angle.to_radians()) }
    }
    /// A page point in the turned frame.
    pub fn to_local(&self, p: Point) -> Point {
        self.to_doc().inverse() * p
    }
    /// `a`, a transform of the turned frame, as a page transform.
    pub fn conjugate(&self, a: Affine) -> Affine {
        if self.angle == 0.0 { a } else { self.to_doc() * a * self.to_doc().inverse() }
    }
    pub fn center(&self) -> Point {
        self.to_doc() * self.rect.center()
    }
    /// One of the 9 reference points (row-major from the box's own top-left), on the page.
    pub fn reference_point(&self, index: usize) -> Point {
        self.to_doc() * vectorcraft_geom::reference_point(self.rect, index)
    }
    /// The corners on the page: top-left, top-right, bottom-right, bottom-left of the box itself.
    pub fn corners(&self) -> [Point; 4] {
        let r = self.rect;
        [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| self.to_doc() * Point::new(x, y))
    }
}

/// The angle (counter-clockwise degrees) of an object's axes after `a`: where `a` takes its x axis.
/// A mirroring `a` allows two boxes 180° apart: the one nearer the old angle (then the smaller
/// angle) is kept, so flipping an upright object keeps it upright.
pub fn transformed_angle(angle: f64, a: Affine) -> f64 {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    if (m0, m1, m2, m3) == (1.0, 0.0, 0.0, 1.0) {
        return angle;
    }
    let (s, c) = (-angle.to_radians()).sin_cos();
    let v = Vec2::new(m0 * c + m2 * s, m1 * c + m3 * s);
    // Degenerate (or non-finite) transforms leave the angle alone.
    let len = v.hypot();
    if !len.is_finite() || len <= 1e-12 {
        return angle;
    }
    let mut out = normalize_deg(-v.y.atan2(v.x).to_degrees());
    let dist = |x: f64| normalize_deg(x - angle).abs();
    if a.determinant() < 0.0 {
        let alt = normalize_deg(out + 180.0);
        let (d0, d1) = (dist(out), dist(alt));
        if d1 < d0 - EPS_DEG || ((d1 - d0).abs() <= EPS_DEG && alt.abs() < out.abs()) {
            out = alt;
        }
    }
    // Moves and scales along the object's own axes keep its angle exactly.
    if dist(out) <= EPS_DEG {
        angle
    } else if out.abs() <= EPS_DEG {
        0.0
    } else {
        out
    }
}

impl Node {
    /// Bounds of this object as `a` would place it (geometric, or `visual` with its strokes),
    /// without changing it, symbol instances measured by their art from `symbols`
    /// ([`Node::geometric_bounds_with`]).
    pub fn bounds_in(&self, symbols: &SymbolBoxes, a: Affine, visual: bool) -> Option<Rect> {
        match &self.kind {
            NodeKind::Group { clip: false, .. } => {
                let b = self.editing_children().iter().fold(None, |acc, c| union_opt(acc, c.bounds_in(symbols, a, visual)));
                // The group's own strokes paint around its members (as in `visual_bounds`).
                let o = if visual { self.appearance.outset() } else { 0.0 };
                b.map(|b| b.inflate(o, o))
            }
            NodeKind::Group { children, clip: true } => {
                let clip = children.first()?;
                let stroked = matches!(clip.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_));
                clip.bounds_in(symbols, a, visual && stroked)
            }
            _ => {
                let mut n = self.clone();
                n.transform(a, false);
                if visual { n.visual_bounds_with(symbols) } else { n.geometric_bounds_with(symbols) }
            }
        }
    }
}

impl Document {
    /// The angle the bounding box of `ids` stands at: the `bbox_angle` they share, else 0.
    pub fn bbox_angle(&self, ids: &[NodeId]) -> f64 {
        let mut angles = ids.iter().filter_map(|id| self.node(*id)).map(|n| n.bbox_angle);
        let Some(first) = angles.next() else { return 0.0 };
        if angles.all(|a| normalize_deg(a - first).abs() <= EPS_DEG) { first } else { 0.0 }
    }
    /// The bounding box of `ids` square to their shared angle ([`Document::bbox_angle`]):
    /// geometric bounds, or `visual` with their strokes, symbol instances measured by their art
    /// ([`Document::bounds_of`]).
    pub fn oriented_bounds(&self, ids: &[NodeId], visual: bool) -> Option<OrientedBox> {
        let angle = self.bbox_angle(ids);
        if angle == 0.0 {
            return self.bounds_of(ids, visual).map(OrientedBox::aligned);
        }
        let to_local = OrientedBox { angle, rect: Rect::ZERO }.to_doc().inverse();
        let symbols = self.symbol_boxes();
        let rect = ids.iter().filter_map(|id| self.node(*id)).fold(None, |acc, n| union_opt(acc, n.bounds_in(&symbols, to_local, visual)))?;
        Some(OrientedBox { angle, rect })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Appearance;
    use vectorcraft_geom::shapes;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn rot(deg: f64) -> Affine {
        Affine::rotate(-deg.to_radians())
    }

    #[test]
    fn rotations_add_and_moves_keep_the_angle() {
        assert!(close(transformed_angle(0.0, rot(45.0)), 45.0));
        assert!(close(transformed_angle(170.0, rot(20.0)), -170.0));
        assert_eq!(transformed_angle(30.0, Affine::translate((5.0, 7.0))), 30.0);
        assert_eq!(transformed_angle(45.0, rot(-45.0)), 0.0);
    }

    #[test]
    fn local_scales_keep_the_angle_exactly() {
        let b = OrientedBox { angle: 45.0, rect: Rect::ZERO };
        assert_eq!(transformed_angle(45.0, b.conjugate(Affine::scale_non_uniform(3.0, 0.5))), 45.0);
        // A flip along the object's own axis keeps the box too.
        let b = OrientedBox { angle: 135.0, rect: Rect::ZERO };
        assert_eq!(transformed_angle(135.0, b.conjugate(Affine::scale_non_uniform(-1.0, 1.0))), 135.0);
    }

    #[test]
    fn page_flips_mirror_the_angle() {
        let flip_h = Affine::scale_non_uniform(-1.0, 1.0);
        let flip_v = Affine::scale_non_uniform(1.0, -1.0);
        assert_eq!(transformed_angle(0.0, flip_h), 0.0);
        assert_eq!(transformed_angle(0.0, flip_v), 0.0);
        assert!(close(transformed_angle(30.0, flip_h), -30.0));
        assert!(close(transformed_angle(30.0, flip_v), -30.0));
        // Degenerate transforms leave it alone.
        assert_eq!(transformed_angle(30.0, Affine::scale(0.0)), 30.0);
    }

    #[test]
    fn oriented_bounds_hug_a_rotated_rectangle() {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), Appearance::default_art())).unwrap();
        let c = Point::new(50.0, 25.0);
        d.node_mut(id).unwrap().transform(Affine::translate(c.to_vec2()) * rot(30.0) * Affine::translate(-c.to_vec2()), false);
        let b = d.oriented_bounds(&[id], false).unwrap();
        assert!(close(b.angle, 30.0));
        assert!(close(b.rect.width(), 100.0) && close(b.rect.height(), 50.0));
        assert!(b.center().distance(c) < 1e-9);
        // The page box is bigger.
        assert!(d.bounds_of(&[id], false).unwrap().width() > 100.0);
        // Grouped: the group has no angle of its own, the bounds are square to the page.
        let g = d.alloc_id();
        d.insert(Some(l), 1, Node::group(g, vec![])).unwrap();
        d.move_node(id, Some(g), 0).unwrap();
        assert_eq!(d.oriented_bounds(&[g], false).unwrap().angle, 0.0);
    }

    #[test]
    fn oriented_bounds_hug_a_rotated_symbol_instances_art() {
        let mut d = Document::new(500.0, 500.0);
        let bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 10.0)), Appearance::default_art());
        d.symbols.push(crate::Symbol { name: "Bar".into(), art: std::sync::Arc::new(bar) });
        let (l, id) = (d.layers[0].id, d.alloc_id());
        d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Bar".into(), xf: Affine::translate((100.0, 100.0)) })).unwrap();
        let c = Point::new(120.0, 105.0);
        d.node_mut(id).unwrap().transform(Affine::translate(c.to_vec2()) * rot(30.0) * Affine::translate(-c.to_vec2()), false);
        let b = d.oriented_bounds(&[id], false).unwrap();
        assert!(close(b.angle, 30.0));
        assert!(close(b.rect.width(), 40.0) && close(b.rect.height(), 10.0), "the bar, not the 20 × 20 square: {b:?}");
        assert!(b.center().distance(c) < 1e-9);
    }
}
