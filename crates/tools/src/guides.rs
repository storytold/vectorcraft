//! Smart Guides: snapping to anchors, segment midpoints, object bounds (edges/centres) and
//! artboards, and (while drawing from a point) tangent to and perpendicular to curves and lines,
//! with the magenta construction lines and labels Illustrator users expect.

use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, Shape};
use vectorcraft_doc::hit::{HitOptions, hit_test};
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::{Overlay, ToolContext};

pub const MAGENTA: [u8; 3] = [0xff, 0x3d, 0xfc];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Anchor,
    Center,
    Edge,
    Artboard,
    Midpoint,
    Tangent,
    Perpendicular,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Anchor => "anchor",
            Kind::Center => "center",
            Kind::Edge => "path",
            Kind::Artboard => "artboard",
            Kind::Midpoint => "midpoint",
            Kind::Tangent => "tangent",
            Kind::Perpendicular => "perpendicular",
        }
    }
}

/// The filled magenta square marking the point the pointer snapped to.
fn snap_marker(p: Point) -> Overlay {
    Overlay::Anchor { p, color: MAGENTA, filled: true, size: 6.0 }
}

/// Segments kept for tangent and perpendicular snapping (the rest are skipped).
const MAX_SEGMENTS: usize = 20_000;

/// A path segment: the curve and whether it is straight.
#[derive(Clone, Copy, Debug)]
struct Segment {
    curve: CubicBez,
    line: bool,
    bounds: Rect,
}

/// Snap targets gathered from the document (excluding the objects being edited).
#[derive(Default)]
pub struct Targets {
    points: Vec<(Point, Kind)>,
    xs: Vec<(f64, Point, Kind)>,
    ys: Vec<(f64, Point, Kind)>,
    segments: Vec<Segment>,
}

/// Parameters `t` in 0..1 where `f` changes sign (sampled, then bisected), at most a few.
fn roots(f: impl Fn(f64) -> f64) -> Vec<f64> {
    const N: usize = 48;
    let mut out = vec![];
    let mut prev = (0.0, f(0.0));
    for i in 1..=N {
        let t = i as f64 / N as f64;
        let v = f(t);
        if !(v.is_finite() && prev.1.is_finite()) {
            prev = (t, v);
            continue;
        }
        if prev.1 == 0.0 {
            out.push(prev.0);
        } else if prev.1.signum() != v.signum() && v != 0.0 {
            let (mut a, mut b, mut fa) = (prev.0, t, prev.1);
            for _ in 0..40 {
                let m = 0.5 * (a + b);
                let fm = f(m);
                if fm.signum() == fa.signum() {
                    (a, fa) = (m, fm);
                } else {
                    b = m;
                }
            }
            out.push(0.5 * (a + b));
        }
        if out.len() >= 8 {
            break;
        }
        prev = (t, v);
    }
    if prev.1 == 0.0 {
        out.push(1.0);
    }
    out
}

impl Segment {
    /// Points of the segment where a line from `from` meets it at a right angle (perpendicular),
    /// or touches it (tangent; curves only).
    fn feet(&self, from: Point) -> Vec<(Point, Kind)> {
        let c = self.curve;
        let d = c.deriv();
        let at = |t: f64| (c.eval(t), d.eval(t).to_vec2());
        let mut out: Vec<(Point, Kind)> = roots(|t| {
            let (p, v) = at(t);
            (p - from).dot(v)
        })
        .into_iter()
        .map(|t| (c.eval(t), Kind::Perpendicular))
        .collect();
        if !self.line {
            out.extend(
                roots(|t| {
                    let (p, v) = at(t);
                    (p - from).cross(v)
                })
                .into_iter()
                .map(|t| (c.eval(t), Kind::Tangent)),
            );
        }
        // A foot on the starting point itself is no guide.
        out.retain(|(p, _)| p.distance(from) > 1e-6);
        out
    }
}

impl Targets {
    pub fn collect(doc: &Document, exclude: &[NodeId], visible: Option<Rect>) -> Self {
        let mut t = Targets::default();
        let excluded = |id: NodeId| exclude.iter().any(|e| doc.ancestry(id).is_some_and(|a| a.contains(e)));
        let add_rect = |t: &mut Targets, r: Rect, kind: Kind| {
            let c = r.center();
            for p in [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)] {
                t.xs.push((p.x, p, kind));
                t.ys.push((p.y, p, kind));
            }
            t.points.push((c, Kind::Center));
            t.xs.push((c.x, c, Kind::Center));
            t.ys.push((c.y, c, Kind::Center));
        };
        for ab in &doc.artboards {
            add_rect(&mut t, ab.rect, Kind::Artboard);
        }
        let mut budget = 20_000usize;
        doc.walk(|n| {
            if budget == 0 || n.is_container() || !n.visible || excluded(n.id) {
                return;
            }
            let Some(b) = n.geometric_bounds() else { return };
            if let Some(v) = visible
                && b.intersect(v).area() <= 0.0
                && !v.contains(b.center())
            {
                return;
            }
            if let NodeKind::Path { path, .. } = &n.kind {
                for (_, _, a) in path.anchors() {
                    t.points.push((a.p, Kind::Anchor));
                    budget = budget.saturating_sub(1);
                }
                for sp in &path.subpaths {
                    for i in 0..sp.segment_count() {
                        let curve = sp.segment(i);
                        let line = sp.segment_is_line(i);
                        // Midpoints of straight segments (where Illustrator shows them).
                        if line {
                            t.points.push((curve.p0.midpoint(curve.p3), Kind::Midpoint));
                        }
                        if t.segments.len() < MAX_SEGMENTS {
                            t.segments.push(Segment { curve, line, bounds: curve.bounding_box() });
                        }
                    }
                }
            }
            add_rect(&mut t, b, Kind::Edge);
        });
        t
    }

    /// Anchors and centres of the visible leaves under `roots` (the roots included) whose bounds
    /// reach into `near`.
    fn points_near(doc: &Document, roots: &[NodeId], near: Rect) -> Self {
        let mut t = Targets::default();
        for n in roots.iter().filter_map(|id| doc.node(*id)) {
            n.walk(&mut |c| {
                if c.is_container() || !c.visible {
                    return;
                }
                let Some(b) = c.geometric_bounds() else { return };
                if b.x0 > near.x1 || b.x1 < near.x0 || b.y0 > near.y1 || b.y1 < near.y0 {
                    return;
                }
                if let NodeKind::Path { path, .. } = &c.kind {
                    t.points.extend(path.anchors().map(|(_, _, a)| (a.p, Kind::Anchor)));
                }
                t.points.push((b.center(), Kind::Center));
            });
        }
        t
    }

    /// Snap a single point. Returns the snapped point and guide overlays.
    pub fn snap_point(&self, p: Point, tol: f64) -> (Point, Vec<Overlay>) {
        self.snap_point_from(p, tol, None)
    }

    /// [`Self::snap_point`] for a point drawn from `from` (the other end of a line, the previous
    /// anchor): also snaps where a line from `from` is tangent to a curve or perpendicular to a
    /// line or curve near `p`.
    pub fn snap_point_from(&self, p: Point, tol: f64, from: Option<Point>) -> (Point, Vec<Overlay>) {
        if let Some((q, k)) = self.points.iter().filter(|(q, _)| q.distance(p) <= tol).min_by(|a, b| a.0.distance(p).total_cmp(&b.0.distance(p))) {
            return (*q, vec![Overlay::Label { p: *q, text: k.label().into(), color: MAGENTA }, snap_marker(*q)]);
        }
        if let Some(from) = from {
            let near = Rect::new(p.x - tol, p.y - tol, p.x + tol, p.y + tol);
            let best = self
                .segments
                .iter()
                .filter(|s| s.bounds.inflate(tol, tol).intersect(near).area() > 0.0 || s.bounds.inflate(tol, tol).contains(p))
                .flat_map(|s| s.feet(from))
                .filter(|(q, _)| q.distance(p) <= tol)
                .min_by(|a, b| a.0.distance(p).total_cmp(&b.0.distance(p)));
            if let Some((q, k)) = best {
                return (
                    q,
                    vec![
                        Overlay::Line { a: from, b: q, color: MAGENTA, dashed: true },
                        Overlay::Label { p: q, text: k.label().into(), color: MAGENTA },
                        snap_marker(q),
                    ],
                );
            }
        }
        let mut out = p;
        let mut ov = vec![];
        if let Some((x, from, _)) =
            self.xs.iter().filter(|(x, _, _)| (x - p.x).abs() <= tol).min_by(|a, b| (a.0 - p.x).abs().total_cmp(&(b.0 - p.x).abs()))
        {
            out.x = *x;
            ov.push(Overlay::Line { a: *from, b: Point::new(*x, p.y), color: MAGENTA, dashed: false });
        }
        if let Some((y, from, _)) =
            self.ys.iter().filter(|(y, _, _)| (y - p.y).abs() <= tol).min_by(|a, b| (a.0 - p.y).abs().total_cmp(&(b.0 - p.y).abs()))
        {
            out.y = *y;
            ov.push(Overlay::Line { a: *from, b: Point::new(p.x, *y), color: MAGENTA, dashed: false });
        }
        if !ov.is_empty() {
            ov.push(Overlay::Label { p: out, text: "align".into(), color: MAGENTA });
            ov.push(snap_marker(out));
        }
        (out, ov)
    }

    /// Snap a moving rectangle (selection bounds after a move by `d`): tries its corners/edges/centre.
    pub fn snap_rect(&self, r: Rect, tol: f64) -> (Vec2, Vec<Overlay>) {
        let c = r.center();
        let xs = [r.x0, c.x, r.x1];
        let ys = [r.y0, c.y, r.y1];
        let best_x = xs
            .iter()
            .flat_map(|x| self.xs.iter().map(move |(t, from, _)| (t - x, *from, *x)))
            .filter(|(d, _, _)| d.abs() <= tol)
            .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        let best_y = ys
            .iter()
            .flat_map(|y| self.ys.iter().map(move |(t, from, _)| (t - y, *from, *y)))
            .filter(|(d, _, _)| d.abs() <= tol)
            .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        let mut d = Vec2::ZERO;
        let mut ov = vec![];
        if let Some((dx, from, x)) = best_x {
            d.x = dx;
            let x = x + dx;
            let (y0, y1) = (from.y.min(r.y0), from.y.max(r.y1));
            ov.push(Overlay::Line { a: Point::new(x, y0), b: Point::new(x, y1), color: MAGENTA, dashed: false });
        }
        if let Some((dy, from, y)) = best_y {
            d.y = dy;
            let y = y + dy;
            let (x0, x1) = (from.x.min(r.x0), from.x.max(r.x1));
            ov.push(Overlay::Line { a: Point::new(x0, y), b: Point::new(x1, y), color: MAGENTA, dashed: false });
        }
        (d, ov)
    }
}

/// Snap `p` for a drawing tool when smart guides (or grid snapping) are on.
pub fn snap_draw(cx: &ToolContext, p: Point, exclude: &[NodeId]) -> (Point, Vec<Overlay>) {
    snap_draw_from(cx, p, exclude, None)
}

/// [`snap_draw`] for a point drawn from `from` (a line's start, the pen's previous anchor): smart
/// guides also snap tangent and perpendicular to the paths near `p`.
pub fn snap_draw_from(cx: &ToolContext, p: Point, exclude: &[NodeId], from: Option<Point>) -> (Point, Vec<Overlay>) {
    if cx.snap_to_pixel {
        return (Point::new(p.x.round(), p.y.round()), vec![]);
    }
    if cx.snap_to_grid {
        let s = cx.doc.grid.spacing / cx.doc.grid.subdivisions.max(1) as f64;
        return (vectorcraft_geom::snap::snap_point_to_grid(p, s), vec![]);
    }
    if !cx.smart_guides {
        return (p, vec![]);
    }
    Targets::collect(cx.doc, exclude, None).snap_point_from(p, cx.tol(5.0), from)
}

/// Snap a picked point (a transform tool's reference point) to the nearest anchor or centre of the
/// selection or of the object under the pointer, when Snap to Point or Smart Guides is on.
pub fn snap_pick(cx: &ToolContext, p: Point) -> (Point, Vec<Overlay>) {
    if !(cx.snap_to_point || cx.smart_guides) {
        return (p, vec![]);
    }
    let tol = cx.tol(5.0);
    let mut roots = cx.selection.objects.clone();
    roots.extend(hit_test(cx.doc, p, HitOptions { tol, ..cx.hit_options() }).map(|h| h.leaf));
    let near = Rect::new(p.x - tol, p.y - tol, p.x + tol, p.y + tol);
    Targets::points_near(cx.doc, &roots, near).snap_point(p, tol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn snaps_to_anchor_and_alignment() {
        let (d, _) = doc_with_rect();
        let t = Targets::collect(&d, &[], None);
        let (p, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
        assert_eq!(p, Point::new(100.0, 100.0));
        assert!(matches!(&ov[0], Overlay::Label { text, .. } if text == "anchor"));
        let (p, _) = t.snap_point(Point::new(301.0, 199.0), 4.0);
        assert_eq!(p.y, 200.0);
    }

    #[test]
    fn snaps_to_the_midpoint_of_a_straight_segment() {
        let (d, _) = doc_with_rect();
        let t = Targets::collect(&d, &[], None);
        // The rectangle's top edge runs from (100,100) to (200,100).
        let (p, ov) = t.snap_point(Point::new(151.0, 102.0), 4.0);
        assert_eq!(p, Point::new(150.0, 100.0));
        assert!(matches!(&ov[0], Overlay::Label { text, .. } if text == "midpoint"));
    }

    #[test]
    fn snaps_perpendicular_to_a_line_from_the_start_point() {
        let (d, _) = doc_with_rect();
        let t = Targets::collect(&d, &[], None);
        // From (130, 40) straight down meets the top edge (y = 100) at a right angle at x = 130.
        let (p, ov) = t.snap_point_from(Point::new(132.5, 101.0), 4.0, Some(Point::new(130.0, 40.0)));
        assert!((p.x - 130.0).abs() < 1e-6 && (p.y - 100.0).abs() < 1e-6, "{p:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "perpendicular")));
    }

    #[test]
    fn snaps_tangent_to_a_circle() {
        let r = 50.0;
        let c = Point::new(0.0, 0.0);
        // A circle as four cubic arcs.
        let k = 0.552_284_749_8 * r;
        let anchors = vec![
            vectorcraft_geom::Anchor::with_handles(Point::new(r, 0.0), Point::new(r, -k), Point::new(r, k)),
            vectorcraft_geom::Anchor::with_handles(Point::new(0.0, r), Point::new(k, r), Point::new(-k, r)),
            vectorcraft_geom::Anchor::with_handles(Point::new(-r, 0.0), Point::new(-r, k), Point::new(-r, -k)),
            vectorcraft_geom::Anchor::with_handles(Point::new(0.0, -r), Point::new(-k, -r), Point::new(k, -r)),
        ];
        let sp = vectorcraft_geom::SubPath::new(anchors, true);
        let mut t = Targets::default();
        for i in 0..sp.segment_count() {
            let curve = sp.segment(i);
            t.segments.push(Segment { curve, line: false, bounds: curve.bounding_box() });
        }
        // From (100, 0), the tangent points are at angle ±60° (cos = r / d = 0.5).
        let from = Point::new(100.0, 0.0);
        let expect = Point::new(r * 0.5, r * (3f64).sqrt() / 2.0);
        let (p, ov) = t.snap_point_from(expect + Vec2::new(2.0, -1.0), 4.0, Some(from));
        assert!(p.distance(expect) < 0.05, "{p:?} vs {expect:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "tangent")));
        let _ = c;
    }

    #[test]
    fn rect_snap_aligns_edges() {
        let (d, id) = doc_with_rect();
        let t = Targets::collect(&d, &[id], None);
        // Artboard is 0..500; a rect at x0=2 snaps to the artboard's left edge.
        let (dv, ov) = t.snap_rect(Rect::new(2.0, 50.0, 52.0, 90.0), 4.0);
        assert_eq!(dv.x, -2.0);
        assert!(!ov.is_empty());
    }
}
