//! Drawing and path-editing tools (pencil, curvature, anchor tools, scissors, knife, line family…).
//!
//! Like every tool these only emit [`crate::Action`]s; the geometry work happens in the engine's
//! `path.*` commands (`crates/engine/src/cmd/draw2.rs`), so gestures replay through the journal.

mod anchor;
mod curvature;
mod family;
mod gesture;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, Point, SubPath, Vec2};

use crate::{Action, Tool, ToolContext};

pub use anchor::AnchorTool;
pub use curvature::{CurvatureTool, EndCurve, curvature_close, curvature_extend, curvature_insert, curvature_move, curvature_smooth};
pub use family::FamilyTool;
pub use gesture::GestureTool;

/// Colour of tool feedback (Illustrator's selection blue).
pub const FEEDBACK: [u8; 3] = [0x4f, 0x9d, 0xff];

/// Create a drawing tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "pencil" | "paintbrush" | "blobBrush" | "eraser" | "knife" | "smooth" | "pathEraser" | "join" => Box::new(GestureTool::new(id)),
        "curvature" => Box::new(CurvatureTool::default()),
        "addAnchor" | "deleteAnchor" | "anchorPoint" | "scissors" => Box::new(AnchorTool::new(id)),
        "arc" | "spiral" | "rectangularGrid" | "polarGrid" => Box::new(FamilyTool::new(id)),
        _ => return None,
    })
}

/// A handle's length as a fraction of its segment's chord where the handle runs along the chord.
/// It grows as the handle turns away from the chord ([`arc_handle`]).
pub(crate) const HANDLE_K: f64 = 1.0 / 3.0;

/// How long a handle leaving along `dir` should be for its segment, `chord`, to follow a circular
/// arc: `|chord| / (3 cos²(α/2))`, `α` the angle between the handle and the chord. A third of the
/// chord along it, up to two thirds at 90° (the cap: a handle turned further is treated as at 90°).
fn arc_handle(chord: Vec2, dir: Vec2) -> f64 {
    let (l, ld) = (chord.hypot(), dir.hypot());
    if !(l > 1e-9 && ld > 1e-12) {
        return 0.0;
    }
    // cos²(α/2) = (1 + cos α) / 2.
    let cos = (dir.dot(chord) / (ld * l)).clamp(0.0, 1.0);
    2.0 * HANDLE_K * l / (1.0 + cos)
}

/// The sine of half the angle between a smooth point's chords (sin 15°) below which its handles
/// shorten: a turn tighter than 30° is cut back toward a corner instead of swinging round the
/// circle through the three points, which for a narrow V is a near-whole circle.
pub(crate) const TIGHT_TURN: f64 = 0.258_819_045_102_520_8;

/// The handles `(in, out)` of a smooth point `p` between `prev` and `next`, so the curve through
/// the three follows the circle through them: both along that circle's tangent at `p`, the
/// direction of `(next − p)/|next − p|² − (prev − p)/|prev − p|²` (the shorter side weighs more),
/// each as long as [`arc_handle`] says for its own segment; in a turn tighter than 30°
/// ([`TIGHT_TURN`]) shorter, by the square of how far it is under, so a narrow V comes to a point.
/// Three points in a line give a straight tangent. None when there is no direction: a chord of no length, or a fold straight back on
/// itself. The point is then a corner.
pub(crate) fn smooth_handles(prev: Point, p: Point, next: Point) -> Option<(Point, Point)> {
    let (a, b) = (prev - p, next - p);
    let (la2, lb2) = (a.hypot2(), b.hypot2());
    if !(la2.is_finite() && lb2.is_finite()) || la2 <= 1e-18 || lb2 <= 1e-18 {
        return None;
    }
    // In a line with `p` not between its neighbours, the path folds straight back.
    if a.cross(b).abs() <= 1e-9 * la2.sqrt() * lb2.sqrt() && a.dot(b) > 0.0 {
        return None;
    }
    let t = b / lb2 - a / la2;
    let lt = t.hypot();
    if !lt.is_finite() || lt * la2.sqrt().min(lb2.sqrt()) <= 1e-9 {
        return None;
    }
    let u = t / lt;
    // sin(φ/2) = √((1 − cos φ) / 2), φ the angle at `p` between the chords.
    let cos = (a.dot(b) / (la2.sqrt() * lb2.sqrt())).clamp(-1.0, 1.0);
    let k = (((1.0 - cos) / 2.0).sqrt() / TIGHT_TURN).min(1.0).powi(2);
    let (h_in, h_out) = (p - u * (k * arc_handle(a, -u)), p + u * (k * arc_handle(b, u)));
    finite(h_in, h_out)
}

/// The one handle an end `p` of an open path gets so its segment, to the neighbour `q`, follows
/// the same circle as the curve at `q`: `q`'s tangent mirrored in the chord, so the end leaves the
/// chord at the angle the curve meets `q` at, and as long as [`arc_handle`] says. Where `q`'s own
/// handle on their segment falls short of the circle's (a tight turn at `q`), the end handle turns
/// toward the chord and shortens in that share, so the end segment straightens too. `forward` when
/// `p` comes before `q` on the path (its first anchor), else after it (its last). None when `q`
/// has no handle to take a tangent from (a corner): the segment stays straight.
pub(crate) fn end_handle(p: Point, q: &Anchor, forward: bool) -> Option<Point> {
    let t = if q.has_out() {
        q.h_out - q.p
    } else if q.has_in() {
        q.p - q.h_in
    } else {
        return None;
    };
    let (t, chord) = (if forward { t } else { -t }, q.p - p);
    let (lt, lc) = (t.hypot(), chord.hypot());
    if !(lt > 1e-12 && lc > 1e-9) {
        return None;
    }
    let (t, c) = (t / lt, chord / lc);
    let d = c * (2.0 * t.dot(c)) - t;
    let back = if forward { q.h_in } else { q.h_out } - q.p;
    let full = arc_handle(p - q.p, back);
    let share = if back.hypot() > 1e-9 && full > 1e-9 { (back.hypot() / full).min(1.0) } else { 1.0 };
    // Short of the circle, the end also turns toward its chord in that share.
    let d = c * (1.0 - share) + d * share;
    let d = d / d.hypot().max(1e-12);
    let h = p + d * (share * arc_handle(chord, d));
    (h.x.is_finite() && h.y.is_finite()).then_some(h)
}

/// `(a, b)` when every coordinate is finite.
fn finite(a: Point, b: Point) -> Option<(Point, Point)> {
    (a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()).then_some((a, b))
}

/// A spline through `points` (Catmull-Rom converted to cubic Béziers). Points flagged `true` are
/// corners (no handles); open ends have no handles either. Used by the freehand fitting (Pencil,
/// Paintbrush); the Curvature tool has its own rule, [`curvature_spline`].
pub fn catmull_rom(points: &[(Point, bool)], closed: bool) -> SubPath {
    let n = points.len();
    let mut anchors = Vec::with_capacity(n);
    for i in 0..n {
        let (p, corner) = points[i];
        let ends = !closed && (i == 0 || i + 1 == n);
        if corner || ends || n < 3 {
            anchors.push(Anchor::corner(p));
            continue;
        }
        let prev = points[(i + n - 1) % n].0;
        let next = points[(i + 1) % n].0;
        let t = (next - prev) / 6.0;
        if t.hypot() < 1e-9 {
            anchors.push(Anchor::corner(p));
            continue;
        }
        anchors.push(Anchor { p, h_in: p - t, h_out: p + t, kind: AnchorKind::Smooth });
    }
    SubPath::new(anchors, closed && n > 2)
}

/// A spline through `points` by the Curvature tool's rule (not a uniform Catmull-Rom: see
/// [`catmull_rom`], which the freehand fitting keeps): each stretch of three points follows the
/// circle through them. Every smooth point takes [`smooth_handles`] from its neighbours (a corner,
/// or one with no direction, gets none); with three or more points the ends of an open path get
/// [`end_handle`] into the curve unless flagged `true`, and a closed path has no ends. Points flagged `true` are corners; two points are a straight line. The
/// drawing, its preview and the engine's `path.curvature` all build the path with this.
pub fn curvature_spline(points: &[(Point, bool)], closed: bool) -> SubPath {
    let n = points.len();
    let mut anchors: Vec<Anchor> = Vec::with_capacity(n);
    for (i, &(p, corner)) in points.iter().enumerate() {
        let open_end = !closed && (i == 0 || i + 1 == n);
        let neighbours = (n >= 3 && !corner && !open_end)
            .then(|| (points.get((i + n - 1) % n), points.get((i + 1) % n)))
            .and_then(|(prev, next)| Some((prev?.0, next?.0)));
        anchors.push(match neighbours.and_then(|(prev, next)| smooth_handles(prev, p, next)) {
            Some((h_in, h_out)) => Anchor { p, h_in, h_out, kind: AnchorKind::Smooth },
            None => Anchor::corner(p),
        });
    }
    if n >= 3 && !closed {
        // The end handles mirror the neighbours' tangents, which are in place by now.
        let (first, last) = (points.first().is_some_and(|f| !f.1), points.last().is_some_and(|l| !l.1));
        let head = match (anchors.first(), anchors.get(1)) {
            (Some(a), Some(b)) if first => end_handle(a.p, b, true),
            _ => None,
        };
        let tail = match (anchors.last(), n.checked_sub(2).and_then(|j| anchors.get(j))) {
            (Some(a), Some(b)) if last => end_handle(a.p, b, false),
            _ => None,
        };
        if let (Some(h), Some(a)) = (head, anchors.first_mut()) {
            a.h_out = h;
        }
        if let (Some(h), Some(a)) = (tail, anchors.last_mut()) {
            a.h_in = h;
        }
    }
    SubPath::new(anchors, closed && n > 2)
}

/// `[[x, y], …]` for command params.
pub fn points_json(pts: &[Point]) -> Value {
    Value::Array(pts.iter().map(|p| json!([p.x, p.y])).collect())
}

/// A polyline preview path.
pub fn polyline(pts: &[Point]) -> BezPath {
    let mut bp = BezPath::new();
    if let Some(f) = pts.first() {
        bp.move_to(*f);
        for p in &pts[1..] {
            bp.line_to(*p);
        }
    }
    bp
}

/// Editable path nodes, selected ones first, then the rest top-most first.
fn candidate_paths(cx: &ToolContext) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = cx.selection.objects.iter().copied().filter(|id| cx.doc.node(*id).is_some_and(|n| n.path_data().is_some())).collect();
    let mut rest = vec![];
    cx.doc.walk(|n| {
        if let NodeKind::Path { guide: false, .. } = &n.kind {
            rest.push(n.id);
        }
    });
    rest.reverse();
    for id in rest {
        if !v.contains(&id) {
            v.push(id);
        }
    }
    let editable = cx.doc.editable();
    v.retain(|id| editable(*id));
    v
}

/// The selected paths the Pen and the Curvature tool edit (not guides, nor locked or hidden ones).
pub(crate) fn editable_paths<'a>(cx: &'a ToolContext) -> impl Iterator<Item = NodeId> + 'a {
    let editable = cx.doc.editable();
    cx.selection
        .objects
        .iter()
        .copied()
        .filter(move |&id| editable(id) && cx.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Path { guide: false, .. })))
}

/// The paths whose anchors show with the Pen: the selected ones, and those inside selected groups
/// and compound paths (not guides, nor locked or hidden ones). Pen + Alt converts their anchors.
pub(crate) fn shown_paths(cx: &ToolContext) -> Vec<NodeId> {
    fn inside(n: &vectorcraft_doc::Node, v: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Path { guide: false, .. } => v.push(n.id),
            // A Shape Builder group's first child is its shaper, not art.
            NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => {
                for c in children.iter().skip(usize::from(n.extras().shaper.is_some())) {
                    inside(c, v);
                }
            }
            _ => {}
        }
    }
    let mut v = vec![];
    for n in cx.selection.objects.iter().filter_map(|id| cx.doc.node(*id)) {
        inside(n, &mut v);
    }
    let editable = cx.doc.editable();
    v.retain(|id| editable(*id));
    v
}

/// Nearest anchor within `tol`: (path, subpath, anchor).
pub(crate) fn hit_anchor(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize)> {
    anchor_in(cx, candidate_paths(cx), p, tol)
}

/// Nearest anchor within `tol` on the first of `ids` that has one: (path, subpath, anchor).
pub(crate) fn anchor_in(cx: &ToolContext, ids: impl IntoIterator<Item = NodeId>, p: Point, tol: f64) -> Option<(NodeId, usize, usize)> {
    let mut best: Option<((NodeId, usize, usize), f64)> = None;
    for id in ids {
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { continue };
        for (si, ai, a) in pd.anchors() {
            let d = a.p.distance(p);
            if d <= tol && best.is_none_or(|b| d < b.1 - 1e-9) {
                best = Some(((id, si, ai), d));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|b| b.0)
}

/// Nearest point on a path segment within `tol`: (path, subpath, segment, t).
pub(crate) fn hit_segment(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, f64)> {
    segment_in(cx, candidate_paths(cx), p, tol)
}

/// Nearest point within `tol` on a segment of the first of `ids` that passes there: (path,
/// subpath, segment, t).
pub(crate) fn segment_in(cx: &ToolContext, ids: impl IntoIterator<Item = NodeId>, p: Point, tol: f64) -> Option<(NodeId, usize, usize, f64)> {
    for id in ids {
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { continue };
        if let Some((si, seg, t, _, d)) = pd.nearest(p)
            && d <= tol
        {
            return Some((id, si, seg, t));
        }
    }
    None
}

/// Add an anchor where [`hit_segment`] or [`segment_in`] found a segment.
pub(crate) fn insert_anchor((id, si, seg, t): (NodeId, usize, usize, f64)) -> Action {
    Action::Exec("path.insertAnchor".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))
}

/// Delete the anchor [`hit_anchor`] or [`anchor_in`] found.
pub(crate) fn remove_anchor((id, si, ai): (NodeId, usize, usize)) -> Action {
    Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::ParamCurve;

    #[test]
    fn catmull_rom_passes_through_points() {
        let pts = [(Point::new(0.0, 0.0), false), (Point::new(50.0, 50.0), false), (Point::new(100.0, 0.0), false)];
        let sp = catmull_rom(&pts, false);
        assert_eq!(sp.anchors.len(), 3);
        assert_eq!(sp.anchors[1].p, Point::new(50.0, 50.0));
        assert_eq!(sp.anchors[1].kind, AnchorKind::Smooth);
        // The middle tangent is horizontal (parallel to first → last).
        assert!((sp.anchors[1].h_out.y - 50.0).abs() < 1e-9);
        assert!(!sp.anchors[0].has_out());
    }

    /// The freehand fitting (Pencil, Paintbrush) keeps its rule: a symmetric handle of a sixth of
    /// the chord between the neighbours, corners and open ends without handles.
    #[test]
    fn catmull_rom_keeps_the_freehand_rule() {
        let pts = [(pt(0.0, 0.0), false), (pt(10.0, 0.0), false), (pt(200.0, 100.0), false), (pt(150.0, 160.0), true), (pt(0.0, 200.0), false)];
        let sp = catmull_rom(&pts, false);
        let t = (pts[2].0 - pts[0].0) / 6.0;
        assert_eq!((sp.anchors[1].h_in, sp.anchors[1].h_out), (pts[1].0 - t, pts[1].0 + t));
        let t = (pts[3].0 - pts[1].0) / 6.0;
        assert_eq!((sp.anchors[2].h_in, sp.anchors[2].h_out), (pts[2].0 - t, pts[2].0 + t));
        for i in [0, 3, 4] {
            assert_eq!(sp.anchors[i], Anchor::corner(pts[i].0), "anchor {i}");
        }
        // Closed: every smooth point wraps round.
        let t = (pts[1].0 - pts[4].0) / 6.0;
        let closed = catmull_rom(&pts, true);
        assert_eq!((closed.anchors[0].h_in, closed.anchors[0].h_out), (pts[0].0 - t, pts[0].0 + t));
    }

    #[test]
    fn three_points_curve_through_all_of_them() {
        let pts = [(pt(0.0, 0.0), false), (pt(50.0, 50.0), false), (pt(100.0, 0.0), false)];
        let sp = curvature_spline(&pts, false);
        assert_eq!(sp.anchors.len(), 3);
        assert_eq!((sp.anchors[1].p, sp.anchors[1].kind), (pt(50.0, 50.0), AnchorKind::Smooth));
        // The middle tangent is horizontal.
        assert!((sp.anchors[1].h_out.y - 50.0).abs() < 1e-9);
        // Both ends get one handle into the curve, none on the outer side.
        let (first, last) = (sp.anchors[0], sp.anchors[2]);
        assert!(first.has_out() && !first.has_in() && last.has_in() && !last.has_out());
        assert!(!sp.segment_is_line(0) && !sp.segment_is_line(1));
    }

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn smooth(points: &[(f64, f64)]) -> Vec<(Point, bool)> {
        points.iter().map(|&(x, y)| (pt(x, y), false)).collect()
    }

    fn finite(sp: &SubPath) -> bool {
        sp.anchors.iter().all(|a| [a.p, a.h_in, a.h_out].iter().all(|q| q.x.is_finite() && q.y.is_finite()))
    }

    /// The point at `deg` degrees on the circle of radius `r` round `c`.
    fn on_circle(c: Point, r: f64, deg: f64) -> Point {
        let a = deg.to_radians();
        Point::new(c.x + r * a.cos(), c.y + r * a.sin())
    }

    /// The largest relative error of the curve's distance from `c` against `r`, sampled.
    fn off_circle(sp: &SubPath, c: Point, r: f64) -> f64 {
        (0..sp.segment_count())
            .flat_map(|i| (0..=50).map(move |k| (i, f64::from(k) / 50.0)))
            .map(|(i, t)| (sp.segment(i).eval(t).distance(c) - r).abs() / r)
            .fold(0.0, f64::max)
    }

    /// The angle between two directions, in degrees.
    fn angle(a: Vec2, b: Vec2) -> f64 {
        a.cross(b).abs().atan2(a.dot(b)).to_degrees()
    }

    /// Three points on a circle give a curve that stays on it, the ends included, however they
    /// are spaced along it (a J or a U as much as an even arc).
    #[test]
    fn three_points_on_a_circle_follow_it() {
        let c = pt(300.0, 300.0);
        for degs in [[210.0, 270.0, 330.0], [180.0, 230.0, 340.0], [150.0, 270.0, 30.0], [100.0, 180.0, 280.0]] {
            let pts: Vec<(Point, bool)> = degs.iter().map(|&d| (on_circle(c, 100.0, d), false)).collect();
            let sp = curvature_spline(&pts, false);
            let e = off_circle(&sp, c, 100.0);
            assert!(e < 0.01, "{degs:?}: {:.3}% off the circle", e * 100.0);
        }
    }

    /// Three points: both ends get a handle, leaving the chord at the angle the curve meets the
    /// middle point at, measured from the same chord.
    #[test]
    fn the_ends_leave_their_chord_as_the_curve_meets_the_middle() {
        let pts = smooth(&[(100.0, 100.0), (130.0, 400.0), (420.0, 430.0)]);
        let sp = curvature_spline(&pts, false);
        let (first, mid, last) = (sp.anchors[0], sp.anchors[1], sp.anchors[2]);
        assert!(first.has_out() && last.has_in());
        let t = mid.h_out - mid.p;
        let c = mid.p - first.p;
        assert!((angle(first.h_out - first.p, c) - angle(t, c)).abs() < 1e-9);
        let c = last.p - mid.p;
        assert!((angle(last.p - last.h_in, c) - angle(t, c)).abs() < 1e-9);
    }

    /// The middle tangent is the circle's: for points symmetric about a vertical axis it is
    /// horizontal, and the handle on the shorter side is the shorter.
    #[test]
    fn the_tangent_is_the_circles() {
        let sp = curvature_spline(&smooth(&[(0.0, 0.0), (50.0, 80.0), (100.0, 0.0)]), false);
        let m = sp.anchors[1];
        assert!((m.h_out.y - m.p.y).abs() < 1e-9 && (m.h_in.y - m.p.y).abs() < 1e-9);
        assert!(m.h_out.x > m.p.x && m.h_in.x < m.p.x);
        let (prev, p, next) = (pt(0.0, 0.0), pt(10.0, 0.0), pt(200.0, 100.0));
        let (h_in, h_out) = smooth_handles(prev, p, next).unwrap();
        assert!(p.distance(h_in) < p.distance(h_out));
        assert!((h_in - p).cross(h_out - p).abs() < 1e-9 && (h_in - p).dot(h_out - p) < 0.0, "in line, on either side");
        // A handle along its chord is a third of it; turned 90° from it, two thirds.
        assert!((arc_handle(Vec2::new(30.0, 0.0), Vec2::new(1.0, 0.0)) - 10.0).abs() < 1e-12);
        assert!((arc_handle(Vec2::new(30.0, 0.0), Vec2::new(0.0, 1.0)) - 20.0).abs() < 1e-12);
        assert!((arc_handle(Vec2::new(30.0, 0.0), Vec2::new(-1.0, 0.1)) - 20.0).abs() < 1e-12, "capped at 90°");
    }

    /// A narrow V comes to a point at its foot instead of swinging round the circle through its
    /// three points, ends included; a turn of 30° or more keeps the circle's handles.
    #[test]
    fn a_narrow_v_comes_to_a_point() {
        let sp = curvature_spline(&smooth(&[(0.0, 0.0), (35.0, 800.0), (70.0, 10.0)]), false);
        for seg in 0..2 {
            for k in 0..=40 {
                let x = sp.segment(seg).eval(f64::from(k) / 40.0).x;
                assert!((-3.0..=73.0).contains(&x), "segment {seg} reaches x = {x}");
            }
        }
        let m = sp.anchors[1];
        assert!(m.p.distance(m.h_out) < 0.1 * m.p.distance(pt(70.0, 10.0)));
        // A U whose chords meet at about 45° still follows its circle.
        let c = pt(300.0, 300.0);
        let pts: Vec<(Point, bool)> = [225.0, 90.0, 315.0].iter().map(|&d| (on_circle(c, 100.0, d), false)).collect();
        assert!(off_circle(&curvature_spline(&pts, false), c, 100.0) < 0.01);
    }

    /// Two line segments `(a, b)` and `(c, d)` cross.
    fn crosses(a: Point, b: Point, c: Point, d: Point) -> bool {
        let side = |p: Point, q: Point, r: Point| (q - p).cross(r - p);
        let (d1, d2, d3, d4) = (side(c, d, a), side(c, d, b), side(a, b, c), side(a, b, d));
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    }

    /// Very uneven spacing: no NaN, and the curve never crosses itself.
    #[test]
    fn very_uneven_spacing_stays_finite_and_simple() {
        for pts in [
            vec![(0.0, 0.0), (1.0, 0.0), (1000.0, 500.0), (1001.0, 520.0), (1500.0, 0.0)],
            vec![(0.0, 0.0), (800.0, 10.0), (802.0, 400.0), (0.5, 410.0)],
            vec![(0.0, 0.0), (0.001, 0.001), (300.0, 0.0), (300.0, 900.0), (299.0, 901.0)],
        ] {
            let sp = curvature_spline(&smooth(&pts), false);
            assert!(finite(&sp), "{pts:?}");
            let poly: Vec<Point> =
                (0..sp.segment_count()).flat_map(|i| (0..30).map(move |k| (i, f64::from(k) / 30.0))).map(|(i, t)| sp.segment(i).eval(t)).collect();
            for i in 0..poly.len() - 1 {
                for j in i + 2..poly.len() - 1 {
                    assert!(!crosses(poly[i], poly[i + 1], poly[j], poly[j + 1]), "{pts:?} crosses itself near {:?}", poly[i]);
                }
            }
        }
    }

    #[test]
    fn two_points_are_a_line_and_corners_have_no_handles() {
        let sp = curvature_spline(&smooth(&[(0.0, 0.0), (80.0, 40.0)]), false);
        assert!(sp.anchors.iter().all(|a| !a.has_in() && !a.has_out()) && sp.segment_is_line(0));
        // An Alt-clicked middle point is a corner; its neighbours keep their end handle only where
        // the neighbour has a handle on that segment, so both segments here stay straight.
        let pts = [(pt(0.0, 0.0), false), (pt(50.0, 50.0), true), (pt(100.0, 0.0), false)];
        let sp = curvature_spline(&pts, false);
        assert!(!sp.anchors[1].has_in() && !sp.anchors[1].has_out());
        assert!(sp.segment_is_line(0) && sp.segment_is_line(1));
        // An Alt-clicked end stays a corner while the other end curves in.
        let pts = [(pt(0.0, 0.0), true), (pt(50.0, 50.0), false), (pt(100.0, 0.0), false), (pt(150.0, 50.0), false)];
        let sp = curvature_spline(&pts, false);
        assert!(!sp.anchors[0].has_out() && sp.anchors[3].has_in());
    }

    #[test]
    fn a_closed_path_is_smooth_all_round() {
        let pts = smooth(&[(0.0, 0.0), (100.0, 10.0), (120.0, 90.0), (10.0, 100.0)]);
        let sp = curvature_spline(&pts, true);
        assert!(sp.closed);
        for a in &sp.anchors {
            assert_eq!(a.kind, AnchorKind::Smooth);
            assert!((a.h_in - a.p).cross(a.h_out - a.p).abs() < 1e-9 * a.p.distance(a.h_out).max(1.0) * a.p.distance(a.h_in).max(1.0));
        }
    }

    #[test]
    fn degenerate_points_become_corners_without_nan() {
        // A repeated point, and a fold straight back on itself.
        let dup = curvature_spline(&smooth(&[(0.0, 0.0), (50.0, 0.0), (50.0, 0.0), (100.0, 40.0)]), false);
        let fold = curvature_spline(&smooth(&[(0.0, 0.0), (100.0, 0.0), (0.0, 0.0)]), false);
        let same = curvature_spline(&smooth(&[(5.0, 5.0), (5.0, 5.0), (5.0, 5.0)]), true);
        for sp in [&dup, &fold, &same] {
            assert!(finite(sp));
        }
        assert_eq!(fold.anchors[1].kind, AnchorKind::Corner);
        assert!(!fold.anchors[1].has_in() && !fold.anchors[1].has_out());
        assert!(smooth_handles(pt(0.0, 0.0), pt(0.0, 0.0), pt(1.0, 0.0)).is_none());
    }

    #[test]
    fn all_tools_exist() {
        for id in [
            "pencil",
            "paintbrush",
            "blobBrush",
            "eraser",
            "knife",
            "smooth",
            "pathEraser",
            "join",
            "curvature",
            "addAnchor",
            "deleteAnchor",
            "anchorPoint",
            "scissors",
            "arc",
            "spiral",
            "rectangularGrid",
            "polarGrid",
        ] {
            assert_eq!(create(id).unwrap().id(), id);
        }
        assert!(create("selection").is_none());
    }
}
