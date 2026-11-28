//! Automatic Plane Positioning (Perspective Grid Options): the active plane takes its height or
//! depth from an anchor point of art in perspective, or from a gridline intersection. With the
//! Perspective Selection tool, Shift held over one, with no button down, moves the active plane
//! through that point for now and hides the other planes ([`PerspectiveGrid::inferred`]). Making
//! another plane the active one brings that plane through the same point. The plane goes back to
//! its place once an object is drawn on it or attached to it.
//!
//! The point is kept in the grid's frame: x, y and z in points along [`Axis::X`], [`Axis::Y`] and
//! [`Axis::Z`] from the origin, so a plane through it lies at the point's coordinate along the
//! plane's normal.

use serde_json::json;
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::Point;

use super::widget::PlaneTargets;
use super::{Axis, DOC_KEY, PerspectiveGrid, Plane, attachment};
use crate::{Action, ToolContext};

/// The pointer is over an anchor point or a gridline intersection within this many screen pixels.
pub const HIT_PX: f64 = 6.0;

/// What Automatic Plane Positioning took its point from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InferSource {
    Anchor,
    Gridline,
}

impl InferSource {
    pub fn id(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Gridline => "gridline",
        }
    }
}

impl Axis {
    /// Where the axis' coordinate is in a point of the grid's frame.
    fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

/// Call `f` with each shown object in perspective that isn't inside another one, its plane and
/// its depth: its own record, else the one grid `g` stored before attachments were kept on the
/// objects ([`PerspectiveGrid::attachment_of`]).
fn each_attached<'a>(g: &PerspectiveGrid, n: &'a Node, f: &mut dyn FnMut(&'a Node, Plane, f64)) {
    if !n.visible {
        return;
    }
    match attachment(n).or_else(|| g.attached_plane(n.id).map(|p| (p, 0.0))) {
        Some((pl, depth)) => f(n, pl, depth),
        None => n.children().into_iter().flatten().for_each(|c| each_attached(g, c, f)),
    }
}

/// Call `f` with `n` and each node inside it, leaving out hidden nodes and what they hold.
fn each_shown<'a>(n: &'a Node, f: &mut dyn FnMut(&'a Node)) {
    if !n.visible {
        return;
    }
    f(n);
    n.children().into_iter().flatten().for_each(|c| each_shown(c, f));
}

impl PerspectiveGrid {
    /// Where Automatic Plane Positioning holds `plane`: the active plane only.
    pub(super) fn inferred_offset(&self, plane: Plane) -> Option<f64> {
        let p = self.inferred?;
        if plane != self.plane {
            return None;
        }
        let [.., n] = self.axes(plane)?;
        Some(p[n.index()])
    }

    /// The planes that show: all three, or only the active one while Automatic Plane Positioning
    /// holds it.
    pub fn shown_planes(&self) -> Vec<Plane> {
        match self.inferred {
            Some(_) if self.plane != Plane::None => vec![self.plane],
            _ => Plane::ALL.to_vec(),
        }
    }

    /// The point of the grid's frame at plane coordinates `q` on `plane` at `depth`.
    fn frame_point(&self, plane: Plane, depth: f64, q: Point) -> Option<[f64; 3]> {
        let [u, v, n] = self.axes(plane)?;
        let mut out = [0.0; 3];
        out[u.index()] = q.x;
        out[v.index()] = q.y;
        out[n.index()] = depth;
        Some(out)
    }

    /// The anchor point of shown art in perspective nearest `p` within `tol`: (distance, frame point).
    fn anchor_near(&self, doc: &Document, p: Point, tol: f64) -> Option<(f64, [f64; 3])> {
        let mut best: Option<(f64, [f64; 3])> = None;
        let mut visit = |n: &Node, plane: Plane, depth: f64| {
            // The plane's inverse map, worked out only once an anchor is within reach.
            let mut hi = None;
            each_shown(n, &mut |c| {
                let NodeKind::Path { path, .. } = &c.kind else { return };
                for a in path.subpaths.iter().flat_map(|sp| sp.anchors.iter()) {
                    let d = a.p.distance(p);
                    if !(d <= tol && best.is_none_or(|b| d < b.0)) {
                        continue;
                    }
                    let inv = hi.get_or_insert_with(|| self.homography_at(plane, depth).and_then(|h| h.inverse()));
                    if let Some(at) = inv.as_ref().and_then(|m| m.apply(a.p)).and_then(|q| self.frame_point(plane, depth, q)) {
                        best = Some((d, at));
                    }
                }
            });
        };
        for l in &doc.layers {
            each_attached(self, l, &mut visit);
        }
        best
    }

    /// The gridline intersection of a shown plane nearest `p` within `tol`: (distance, frame point).
    fn intersection_near(&self, p: Point, tol: f64) -> Option<(f64, [f64; 3])> {
        let mut best: Option<(f64, [f64; 3])> = None;
        for plane in self.shown_planes() {
            let depth = self.offset(plane);
            let Some(h) = self.homography_at(plane, depth) else { continue };
            let Some(hi) = h.inverse() else { continue };
            let Some(((su, nu), (sv, nv))) = self.line_steps(plane) else { continue };
            // On each drawn line, the intersections either side of the point of the line nearest
            // the pointer on the page. The projection keeps the order of points along a line, so
            // the nearest intersection on that line is one of those two, however skewed the cells.
            let mut candidates = Vec::with_capacity(2 * (nu + nv + 2));
            let mut along = |line: &dyn Fn(f64) -> Point, step: f64, last: usize, coord: fn(Point) -> f64| {
                let (Some(a), Some(b)) = (h.apply(line(0.0)), h.apply(line(last as f64 * step))) else { return };
                let ab = b - a;
                let len2 = ab.x * ab.x + ab.y * ab.y;
                let t = if len2 > 0.0 { (((p - a).x * ab.x + (p - a).y * ab.y) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let Some(at) = hi.apply(a + ab * t).map(coord).filter(|v| v.is_finite()) else { return };
                let k = (at / step).floor().clamp(0.0, last as f64);
                for k in [k, (k + 1.0).min(last as f64)] {
                    candidates.push(line(k * step));
                }
            };
            for i in 0..=nu {
                let u = i as f64 * su;
                along(&|v| Point::new(u, v), sv, nv, |q| q.y);
            }
            for i in 0..=nv {
                let v = i as f64 * sv;
                along(&|u| Point::new(u, v), su, nu, |q| q.x);
            }
            for q in candidates {
                let Some(d) = h.apply(q).map(|at| at.distance(p)) else { continue };
                if d <= tol
                    && best.is_none_or(|b| d < b.0)
                    && let Some(at) = self.frame_point(plane, depth, q)
                {
                    best = Some((d, at));
                }
            }
        }
        best
    }

    /// What Automatic Plane Positioning finds within `tol` of page point `p`: the nearest anchor
    /// point of shown art in perspective, else the nearest gridline intersection of a shown plane,
    /// as a point of the grid's frame. `None` while the grid is hidden or has no active plane.
    pub fn infer_at(&self, doc: &Document, p: Point, tol: f64, targets: PlaneTargets) -> Option<(InferSource, [f64; 3])> {
        if !self.visible || self.plane == Plane::None || !(tol.is_finite() && p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        let anchor = if targets.anchors { self.anchor_near(doc, p, tol) } else { None };
        let found = match anchor {
            Some((_, at)) => Some((InferSource::Anchor, at)),
            None if targets.gridlines => self.intersection_near(p, tol).map(|(_, at)| (InferSource::Gridline, at)),
            None => None,
        };
        found.filter(|(_, at)| at.iter().all(|v| v.is_finite()))
    }

    /// Put a plane that Automatic Plane Positioning holds in the grid stored in `doc` back in its
    /// place. Returns whether one was held.
    pub fn end_positioning(doc: &mut Document) -> bool {
        if !Self::positioning(doc) {
            return false;
        }
        doc.unknown.get_mut(DOC_KEY).and_then(|v| v.as_object_mut()).is_some_and(|o| o.remove("inferred").is_some())
    }

    /// Drop the point Automatic Plane Positioning holds when the grid no longer shows, has no
    /// active plane, or would put the active plane behind the viewer through it.
    pub fn keep_positioning_valid(&mut self) {
        if self.inferred.is_some() && (!self.visible || self.plane == Plane::None || self.validate().is_err()) {
            self.inferred = None;
        }
    }

    /// Whether Automatic Plane Positioning holds a plane of the grid stored in `doc`.
    pub fn positioning(doc: &Document) -> bool {
        doc.unknown.get(DOC_KEY).and_then(|v| v.get("inferred")).is_some_and(|v| !v.is_null())
    }
}

/// The Perspective Selection tool's step for Shift held over `p` with no button down: the command
/// that moves the active plane through the anchor point or gridline intersection there, unless
/// the plane is held through that point already.
pub(super) fn hover_action(cx: &ToolContext, g: &PerspectiveGrid, p: Point) -> Option<Action> {
    let tol = cx.tol(HIT_PX);
    let (_, at) = g.infer_at(cx.doc, p, tol, cx.plane_targets)?;
    if g.inferred.is_some_and(|held| held.iter().zip(&at).all(|(a, b)| (a - b).abs() <= 1e-9)) {
        return None;
    }
    Some(Action::Exec("perspective.plane.infer".into(), json!({"at": [p.x, p.y], "tolerance": tol})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{Mods, PointerEvent, PointerKind, Tool, ToolKey};
    use vectorcraft_doc::Selection;
    use vectorcraft_geom::Rect;

    fn grid() -> PerspectiveGrid {
        PerspectiveGrid::preset(2, Rect::new(0.0, 0.0, 800.0, 600.0))
    }

    #[test]
    fn a_gridline_intersection_gives_the_frame_point_the_planes_pass_through() {
        let mut g = grid();
        let (d, cell) = (Document::new(800.0, 600.0), g.cell);
        // Three cells along and two up the left wall (two-point: u recedes along Z, v is Y).
        let p = g.to_page(Plane::Left, Point::new(3.0 * cell, 2.0 * cell)).unwrap();
        let (source, at) = g.infer_at(&d, p, 1.0, PlaneTargets::ALL).unwrap();
        assert_eq!(source, InferSource::Gridline);
        for (got, want) in at.iter().zip([0.0, 2.0 * cell, 3.0 * cell]) {
            assert!((got - want).abs() < 1e-6, "{at:?}");
        }
        // Held, the active plane lies at the point's coordinate along its normal, the other planes
        // hide, and their own places stay.
        g.inferred = Some(at);
        g.plane = Plane::Ground;
        assert!((g.offset(Plane::Ground) - 2.0 * cell).abs() < 1e-6);
        assert_eq!((g.placed_offset(Plane::Ground), g.offset(Plane::Left)), (0.0, 0.0));
        assert_eq!(g.shown_planes(), vec![Plane::Ground]);
        assert_eq!(g.plane_widgets().len(), 1);
        // The plane drawn there passes through the point the pointer was over.
        let back = g.to_page(Plane::Ground, Point::new(0.0, 3.0 * cell)).unwrap();
        assert!((back - p).hypot() < 1e-6, "{back:?} {p:?}");
        // Another plane made active passes through it too.
        g.plane = Plane::Right;
        assert!((g.offset(Plane::Right) - 3.0 * cell).abs() < 1e-6);
        assert_eq!(g.offset(Plane::Ground), 0.0);
        // With no active plane nothing is held.
        g.plane = Plane::None;
        assert_eq!(g.shown_planes(), Plane::ALL.to_vec());
    }

    #[test]
    fn the_pointer_must_be_within_reach_of_a_target_that_is_on() {
        // Only the right wall shows, in its own place: the other planes' lines are out of the way.
        let g = PerspectiveGrid { plane: Plane::Right, inferred: Some([0.0; 3]), ..grid() };
        let (d, cell) = (Document::new(800.0, 600.0), g.cell);
        let on = g.to_page(Plane::Right, Point::new(2.0 * cell, cell)).unwrap();
        let mid = g.to_page(Plane::Right, Point::new(2.5 * cell, 1.5 * cell)).unwrap();
        assert!(g.infer_at(&d, on + vectorcraft_geom::Vec2::new(2.0, 0.0), 3.0, PlaneTargets::ALL).is_some());
        assert!(g.infer_at(&d, mid, 3.0, PlaneTargets::ALL).is_none(), "the middle of a cell is no intersection");
        assert!(g.infer_at(&d, on, 3.0, PlaneTargets { anchors: true, gridlines: false }).is_none());
        assert!(g.infer_at(&d, on, f64::NAN, PlaneTargets::ALL).is_none());
        assert!(g.infer_at(&d, Point::new(f64::INFINITY, 0.0), 3.0, PlaneTargets::ALL).is_none());
        // Hidden, or without an active plane, the grid gives no point.
        assert!(PerspectiveGrid { visible: false, ..g.clone() }.infer_at(&d, on, 3.0, PlaneTargets::ALL).is_none());
        assert!(PerspectiveGrid { plane: Plane::None, ..g.clone() }.infer_at(&d, on, 3.0, PlaneTargets::ALL).is_none());
        // Beyond the plane's last line there is no intersection.
        let past = g.to_page(Plane::Right, Point::new(g.right_extent() + 5.0 * cell, cell)).unwrap();
        assert!(g.infer_at(&d, past, 3.0, PlaneTargets::ALL).is_none());
    }

    #[test]
    fn a_held_plane_still_checks_its_own_place() {
        // Held at x = 0, the left wall's own place must stay in front of the viewer too: it goes
        // back there once positioning ends.
        let g = PerspectiveGrid { plane: Plane::Left, inferred: Some([0.0; 3]), ..grid() };
        assert!(g.merged(&json!({"leftOffset": -800})).is_err());
        assert!(PerspectiveGrid { inferred: None, ..g.clone() }.merged(&json!({"leftOffset": -800})).is_err());
        let moved = g.merged(&json!({"leftOffset": 40})).unwrap();
        assert_eq!((moved.placed_offset(Plane::Left), moved.offset(Plane::Left), moved.inferred), (40.0, 0.0, Some([0.0; 3])));
    }

    #[test]
    fn the_nearest_intersection_on_the_page_is_found_on_a_skewed_plane() {
        // The left vanishing point next to the origin skews the left wall's cells hard: the
        // intersection nearest on the page lies many cells from the pointer's inverse.
        let g = PerspectiveGrid { vp_left: 399.0, plane: Plane::Left, inferred: Some([0.0; 3]), ..grid() };
        let d = Document::new(800.0, 600.0);
        let gridlines = PlaneTargets { anchors: false, gridlines: true };
        let p = Point::new(400.0, 219.761194);
        let want = Point::new(10.0 * g.cell, 10.0 * g.cell);
        assert!((g.to_page(Plane::Left, want).unwrap() - p).hypot() < 0.5);
        let (_, at) = g.infer_at(&d, p, 6.0, gridlines).unwrap();
        for (got, want) in at.iter().zip([0.0, want.y, want.x]) {
            assert!((got - want).abs() < 1e-6, "{at:?}");
        }
    }

    #[test]
    fn a_hidden_path_inside_art_in_perspective_has_no_anchor_to_take() {
        let g = PerspectiveGrid { plane: Plane::Right, ..grid() };
        let mut d = Document::new(800.0, 600.0);
        g.store(&mut d);
        // A group on the right wall holding one small square, a corner in the middle of a cell.
        let at = g.to_page(Plane::Right, Point::new(2.5 * g.cell, 1.5 * g.cell)).unwrap();
        let (layer, group, square) = (d.layers[0].id, d.alloc_id(), d.alloc_id());
        let mut path = Node::path(square, vectorcraft_geom::shapes::rectangle(Rect::new(at.x, at.y, at.x + 2.0, at.y + 2.0)), Default::default());
        path.visible = false;
        let mut art = Node::group(group, vec![std::sync::Arc::new(path)]);
        super::super::set_attachment(&mut art, Plane::Right, 0.0);
        d.insert(Some(layer), 0, art).unwrap();
        let anchors = PlaneTargets { anchors: true, gridlines: false };
        assert!(g.infer_at(&d, at, 0.5, anchors).is_none(), "the square is hidden");
        d.node_mut(square).unwrap().visible = true;
        let (source, point) = g.infer_at(&d, at, 0.5, anchors).unwrap();
        assert_eq!(source, InferSource::Anchor);
        for (got, want) in point.iter().zip([2.5 * g.cell, 1.5 * g.cell, 0.0]) {
            assert!((got - want).abs() < 1e-6, "{point:?}");
        }
        // A grid stored before attachments were kept on the objects lists them itself.
        let mut d = Document::new(800.0, 600.0);
        let (layer, square) = (d.layers[0].id, d.alloc_id());
        d.insert(
            Some(layer),
            0,
            Node::path(square, vectorcraft_geom::shapes::rectangle(Rect::new(at.x, at.y, at.x + 2.0, at.y + 2.0)), Default::default()),
        )
        .unwrap();
        assert!(g.infer_at(&d, at, 0.5, anchors).is_none(), "not in perspective");
        let mut legacy = g.clone();
        legacy.attached.insert(square.0.to_string(), Plane::Right);
        assert_eq!(legacy.infer_at(&d, at, 0.5, anchors).map(|f| f.0), Some(InferSource::Anchor));
    }

    #[test]
    fn the_stored_point_ends_with_end_positioning() {
        let mut d = Document::new(800.0, 600.0);
        let mut g = grid();
        g.store(&mut d);
        assert!(!PerspectiveGrid::positioning(&d));
        assert!(!PerspectiveGrid::end_positioning(&mut d));
        g.inferred = Some([1.0, 2.0, 3.0]);
        g.store(&mut d);
        assert!(PerspectiveGrid::positioning(&d));
        assert_eq!(PerspectiveGrid::current(&d).inferred, Some([1.0, 2.0, 3.0]));
        // `perspective.grid.set` leaves the point as it is.
        assert_eq!(g.merged(&json!({"inferred": [9, 9, 9], "cell": 25})).unwrap().inferred, Some([1.0, 2.0, 3.0]));
        assert!(PerspectiveGrid { inferred: Some([f64::NAN, 0.0, 0.0]), ..g.clone() }.validate().is_err());
        assert!(PerspectiveGrid::end_positioning(&mut d));
        assert_eq!(PerspectiveGrid::current(&d), PerspectiveGrid { inferred: None, ..g });
    }

    #[test]
    fn shift_over_an_intersection_asks_for_the_plane_once() {
        let mut d = Document::new(800.0, 600.0);
        let mut g = grid();
        g.store(&mut d);
        let (s, p) = (Selection::default(), paint());
        let at = g.to_page(Plane::Left, Point::new(2.0 * g.cell, 3.0 * g.cell)).unwrap();
        let shift = Mods { shift: true, ..Default::default() };
        let hover = |d: &Document, mods: Mods| {
            let c = cx(d, &s, &p);
            let mut t = super::super::PerspectiveSelectionTool::default();
            t.pointer(&c, &PointerEvent { mods, ..PointerEvent::new(PointerKind::Move, at.x, at.y) })
        };
        assert!(hover(&d, Mods::default()).is_empty(), "without Shift the pointer only hovers");
        let acts = hover(&d, shift);
        let [Action::Exec(cmd, v)] = acts.as_slice() else { panic!("{acts:?}") };
        assert_eq!((cmd.as_str(), &v["at"]), ("perspective.plane.infer", &json!([at.x, at.y])));
        // Held through that point already, the hover asks for nothing more.
        g.inferred = g.infer_at(&d, at, 1.0, PlaneTargets::ALL).map(|f| f.1);
        g.store(&mut d);
        assert!(hover(&d, shift).is_empty());
        // Escape puts the plane back.
        let c = cx(&d, &s, &p);
        let mut t = super::super::PerspectiveSelectionTool::default();
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Exec("perspective.plane.infer".into(), json!({"clear": true}))]);
    }
}
