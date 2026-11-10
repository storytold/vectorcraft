//! Curved outlines for the facets of a solid: a cap, the flat plane of Rotate and the edges of a
//! wall along the profile follow the profile's curves as cubics, while each facet's polygon stays
//! the shape that visibility and depth are worked out on.

use vectorcraft_geom::{BezPath, CubicBez, ParamCurve, PathData, Point};

use super::rings::{Ring, Span};

/// How far a fitted cubic may lie from the outline it follows, in points.
pub(super) const CURVE_TOLERANCE: f64 = 0.001;

/// How many times one piece of curve is halved at most to fit it. A piece that still misses the
/// tolerance keeps its sampled straight edges.
const MAX_DEPTH: u32 = 8;

/// Fitted cubics one solid's outlines hold at most. Past it, the remaining edges stay straight.
pub(super) const MAX_OUTLINE_CURVES: usize = 1 << 20;

/// The fitted cubics of one solid: how many it holds so far, against [`MAX_OUTLINE_CURVES`].
#[derive(Default)]
pub(super) struct Budget(usize);

/// The outline of a solid at one step, as a point of the document for a point of a profile's
/// curve: `at(span, t)` is where the outline lies at `t` on `span`'s curve. `tolerance` is how far
/// a fitted cubic may lie from it. With `map`, the outline is the curve moved point by point by
/// that map, and a piece of curve is tried first as its control points moved by it: exact when
/// `affine`, otherwise kept when it lies within the tolerance.
pub(super) struct Along<'a> {
    pub(super) at: &'a dyn Fn(&Span, f64) -> Point,
    pub(super) map: Option<&'a dyn Fn(Point) -> Point>,
    pub(super) affine: bool,
    pub(super) tolerance: f64,
}

/// `g` from `p` at `a` to `q` at `b` as cubics within `tolerance`, appended to `out`. False when it
/// does not fit within [`MAX_DEPTH`] halvings, or reaches a point that is not finite.
#[allow(clippy::too_many_arguments)]
fn fit(g: &dyn Fn(f64) -> Point, p: Point, q: Point, a: f64, b: f64, tolerance: f64, depth: u32, out: &mut Vec<CubicBez>) -> bool {
    // Second-order one-sided differences, taken inside the piece: a handle of zero length at an end
    // of the source curve turns its normal, and so an inset outline's tangent, at that end.
    let h = (b - a) * 1e-5;
    if h == 0.0 || !h.is_finite() {
        return false;
    }
    let (g0, g1, g2) = (g(a), g(a + h), g(a + 2.0 * h));
    let start = (g1 - g0) * 2.0 - (g2 - g0) * 0.5;
    let (g0, g1, g2) = (g(b), g(b - h), g(b - 2.0 * h));
    let end = (g0 - g1) * 2.0 - (g0 - g2) * 0.5;
    let (start, end) = (start / h, end / h);
    let curve = CubicBez::new(p, p + start * ((b - a) / 3.0), q - end * ((b - a) / 3.0), q);
    let points = [curve.p1, curve.p2];
    if points.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
        return false;
    }
    let mut error = 0.0f64;
    for u in [0.25, 0.5, 0.75] {
        let want = g(a + (b - a) * u);
        if !want.x.is_finite() || !want.y.is_finite() {
            return false;
        }
        error = error.max(curve.eval(u).distance(want));
    }
    if error <= tolerance {
        out.push(curve);
        return true;
    }
    if depth >= MAX_DEPTH {
        return false;
    }
    let mid = (a + b) / 2.0;
    let m = g(mid);
    if !m.x.is_finite() || !m.y.is_finite() {
        return false;
    }
    fit(g, p, m, a, mid, tolerance, depth + 1, out) && fit(g, m, q, mid, b, tolerance, depth + 1, out)
}

/// The cubics from `p` to `q` along the piece of `span`'s curve from `t0` to `t1` at `step`, or
/// `None` when they do not fit, or would take the budget past [`MAX_OUTLINE_CURVES`].
///
/// `p` and `q` are the face's own corners, which a bevel's miter can move off the curve's offset
/// where the curve meets another edge at an angle. The difference is eased out from `p` by `fade.0`
/// and into `q` from `fade.1` (the ends of the first and last sampled edge), as the face's polygon
/// does over those edges, so the curves meet the face's corners.
#[allow(clippy::too_many_arguments)]
fn piece(step: &Along, span: &Span, t0: f64, t1: f64, fade: (f64, f64), p: Point, q: Point, budget: &mut Budget) -> Option<Vec<CubicBez>> {
    if let Some(map) = step.map {
        let c = span.curve.subsegment(t0..t1);
        let c = CubicBez::new(p, map(c.p1), map(c.p2), q);
        let finite = [c.p1, c.p2].iter().all(|q| q.x.is_finite() && q.y.is_finite());
        let fits = || [0.25, 0.5, 0.75].into_iter().all(|u| c.eval(u).distance((step.at)(span, t0 + (t1 - t0) * u)) <= step.tolerance);
        if finite && (step.affine || fits()) {
            budget.0 = budget.0.saturating_add(1);
            return (budget.0 <= MAX_OUTLINE_CURVES).then(|| vec![c]);
        }
    }
    let (dp, dq) = (p - (step.at)(span, t0), q - (step.at)(span, t1));
    // 1 at the end, 0 from `to` on, with no slope at either.
    let ease = |t: f64, from: f64, to: f64| {
        let x = if to == from { 1.0 } else { ((t - from) / (to - from)).clamp(0.0, 1.0) };
        1.0 - x * x * (3.0 - 2.0 * x)
    };
    let g = |t: f64| (step.at)(span, t) + dp * ease(t, t0, fade.0) + dq * ease(t, t1, fade.1);
    let mut out = vec![];
    if !fit(&g, p, q, t0, t1, step.tolerance, 0, &mut out) {
        return None;
    }
    budget.0 = budget.0.saturating_add(out.len());
    (budget.0 <= MAX_OUTLINE_CURVES).then_some(out)
}

/// Append contour `points` (closed), whose edges follow `ring`'s, to `path`: each run of edges along
/// one piece of curve as cubics, and the rest straight. False when no edge is curved.
fn contour(path: &mut BezPath, ring: &Ring, points: &[Point], step: &Along, budget: &mut Budget) -> bool {
    let n = points.len();
    let Some(&first) = points.first() else { return false };
    if ring.spans.len() != n {
        return false;
    }
    path.move_to(first);
    let mut curved = false;
    let mut i = 0;
    while i < n {
        let end = |k: usize| points.get((k + 1) % n).copied().unwrap_or(first);
        let Some(span) = ring.spans.get(i).copied().flatten() else {
            if i + 1 < n {
                path.line_to(end(i));
            }
            i += 1;
            continue;
        };
        // The run of edges that follow this curve one after the other.
        let (mut j, mut t1, mut last) = (i, span.t1, span.t0);
        while let Some(Some(next)) = ring.spans.get(j + 1)
            && next.curve == span.curve
            && next.t0 == t1
        {
            (j, t1, last) = (j + 1, next.t1, next.t0);
        }
        let (p, q) = (points.get(i).copied().unwrap_or(first), end(j));
        match piece(step, &span, span.t0, t1, (span.t1, last), p, q, budget) {
            Some(curves) => {
                curved = true;
                for c in curves {
                    path.curve_to(c.p1, c.p2, c.p3);
                }
            }
            // The edge back to the first point is drawn by `close_path`.
            None => (i..=j).filter(|k| k + 1 < n).for_each(|k| path.line_to(end(k))),
        }
        i = j + 1;
    }
    path.close_path();
    curved
}

/// The outline of a face made of `rings` at one `step` (a cap, or Rotate's plane), projected as
/// `points` (the face's contours, one for each ring and as long). `None` when no edge is curved.
pub(super) fn face(rings: &[&Ring], points: &[Vec<Point>], step: &Along, budget: &mut Budget) -> Option<PathData> {
    if rings.len() != points.len() || rings.iter().all(|r| r.spans.iter().all(Option::is_none)) {
        return None;
    }
    let mut path = BezPath::new();
    let mut curved = false;
    for (ring, points) in rings.iter().zip(points) {
        curved |= contour(&mut path, ring, points, step, budget);
    }
    curved.then(|| PathData::from_bezpath(&path))
}

/// The outline of a wall's quad `quad` (`a`, `a'`, `b'`, `b` projected: the edge at `near` from
/// `a` to `a'`, then the edge at `far` back from `b'` to `b`), whose edges along the profile follow
/// `span`. An edge without its outline stays straight. `None` when neither edge is curved.
pub(super) fn wall(span: &Span, quad: &[Point], near: Option<&Along>, far: Option<&Along>, budget: &mut Budget) -> Option<PathData> {
    let [a, a2, b2, b] = <[Point; 4]>::try_from(quad).ok()?;
    let front = near.and_then(|near| piece(near, span, span.t0, span.t1, (span.t1, span.t0), a, a2, budget));
    let back = far.and_then(|far| piece(far, span, span.t1, span.t0, (span.t0, span.t1), b2, b, budget));
    if front.is_none() && back.is_none() {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to(a);
    match front {
        Some(curves) => curves.iter().for_each(|c| path.curve_to(c.p1, c.p2, c.p3)),
        None => path.line_to(a2),
    }
    path.line_to(b2);
    match back {
        Some(curves) => curves.iter().for_each(|c| path.curve_to(c.p1, c.p2, c.p3)),
        None => path.line_to(b),
    }
    path.close_path();
    Some(PathData::from_bezpath(&path))
}
