//! Variable-width strokes (Width tool / width profiles).
//!
//! A stroke with a [`WidthProfile`] is drawn as a filled outline: each subpath is flattened, and
//! at every sample the left and right offsets `width/2 · factor(t)` (t = fraction of the
//! subpath's length) are placed along the normal. Corners of the path take the stroke's miter,
//! round or bevel join on their outer side (smooth points bend round), open subpaths get butt,
//! round or projecting caps (the round cap blends the two side widths), and closed subpaths become
//! two loops of opposite orientation filled with the non-zero rule. Dashes are pieces of a longer
//! subpath and read the profile where they sit along it ([`outline_spans`]).

use std::f64::consts::PI;

use kurbo::{BezPath, ParamCurve, PathEl, PathSeg, Point, Vec2};
use vectorcraft_doc::{LineCap, LineJoin, StrokeLayer, WidthProfile};

use super::{kink, segments, subpaths, unit};

/// Filled outline of `bp` stroked with `width` scaled by `profile`, with the caps, joins and miter
/// limit of `st`, flattened to `tol`.
pub fn width_outline(bp: &BezPath, width: f64, profile: &WidthProfile, st: &StrokeLayer, tol: f64) -> BezPath {
    outline_spans(bp, &[], width, profile, st, tol)
}

/// [`width_outline`] of pieces of longer subpaths: subpath `i` of `bp` covers the fractions
/// `spans[i]` of the subpath it was cut from (see [`Dashed::spans`](super::Dashed::spans));
/// subpaths without a span are whole.
pub(super) fn outline_spans(bp: &BezPath, spans: &[(f64, f64)], width: f64, profile: &WidthProfile, st: &StrokeLayer, tol: f64) -> BezPath {
    let ctx = Ctx { half: width / 2.0, profile, cap: st.cap, join: st.join, miter_limit: st.miter_limit, tol: tol.max(1e-4) };
    let mut out = BezPath::new();
    for (i, (r, closed)) in subpaths(bp).into_iter().enumerate() {
        let (pts, corner) = flatten(&segments(&bp.elements()[r]), closed, ctx.tol);
        if pts.len() > 1 {
            ctx.subpath(&mut out, &pts, &corner, closed, spans.get(i).copied().unwrap_or((0.0, 1.0)));
        }
    }
    out
}

/// Left normal in y-down space (left of the direction of travel).
pub(super) fn left(t: Vec2) -> Vec2 {
    Vec2::new(t.y, -t.x)
}

/// The flattened points of one subpath (consecutive duplicates removed; a closed one doesn't
/// repeat its start), each marked when it is a corner of the path.
pub(super) fn flatten(segs: &[PathSeg], closed: bool, tol: f64) -> (Vec<Point>, Vec<bool>) {
    let Some(s0) = segs.first() else { return (vec![], vec![]) };
    let (mut pts, mut corner) = (vec![s0.start()], vec![false]);
    // The first and the last segment that drew something.
    let (mut first, mut prev): (Option<&PathSeg>, Option<&PathSeg>) = (None, None);
    for seg in segs {
        let at = pts.len() - 1;
        let mut push = |p: Point| {
            if pts.last().is_none_or(|l| l.distance(p) > 1e-9) {
                pts.push(p);
                corner.push(false);
            }
        };
        match seg {
            PathSeg::Line(l) => push(l.p1),
            // Bounded per curve: a projection can blow a small curve up enough that kurbo sizes
            // an enormous buffer from it.
            _ => vectorcraft_geom::flatten([PathEl::MoveTo(seg.start()), seg.as_path_el()], tol, |el| {
                if let PathEl::LineTo(p) = el {
                    push(p);
                }
            }),
        }
        if pts.len() > at + 1 {
            match prev {
                Some(p) => corner[at] = kink(p, seg),
                None => first = Some(seg),
            }
            prev = Some(seg);
        }
    }
    if closed {
        if pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) <= 1e-9 {
            pts.pop();
            corner.pop();
        }
        if let (Some(a), Some(b)) = (prev, first) {
            corner[0] = kink(a, b);
        }
    }
    (pts, corner)
}

/// Insert samples where the profile has width points (so the piecewise-linear profile is exact
/// along straight runs; the two points of a discontinuous point share one sample). `span` maps the
/// subpath onto the profile (see [`outline_spans`]).
fn densify(pts: &[Point], corner: &[bool], closed: bool, profile: &WidthProfile, span: (f64, f64)) -> (Vec<Point>, Vec<bool>) {
    let total = length(pts, closed);
    let range = span.1 - span.0;
    if total <= 1e-12 || range <= 1e-12 {
        return (pts.to_vec(), corner.to_vec());
    }
    // Where the profile's points fall along this piece (a span past 1 wraps round a closed path).
    let marks: Vec<f64> = profile.points.iter().flat_map(|p| [p.0, p.0 + 1.0]).map(|t| (t - span.0) / range * total).collect();
    split_at(pts, corner, closed, &marks)
}

/// Length of a flattened subpath (see [`flatten`]).
pub(super) fn length(pts: &[Point], closed: bool) -> f64 {
    let n = pts.len();
    let seg_count = if closed { n } else { n.saturating_sub(1) };
    (0..seg_count).map(|i| pts[i].distance(pts[(i + 1) % n])).sum()
}

/// A flattened subpath (see [`flatten`]) with points added `marks` along it (distances from its
/// start; those on a point or off the subpath are ignored).
pub(super) fn split_at(pts: &[Point], corner: &[bool], closed: bool, marks: &[f64]) -> (Vec<Point>, Vec<bool>) {
    let n = pts.len();
    let seg_count = if closed { n } else { n.saturating_sub(1) };
    let mut out = (Vec::with_capacity(n + marks.len()), Vec::with_capacity(n + marks.len()));
    let mut acc = 0.0;
    for i in 0..seg_count {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = a.distance(b);
        out.0.push(a);
        out.1.push(corner[i]);
        let mut inner: Vec<f64> = marks.iter().copied().filter(|d| *d > acc + 1e-9 && *d < acc + len - 1e-9).collect();
        inner.sort_by(f64::total_cmp);
        inner.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
        for d in inner {
            out.0.push(a.lerp(b, (d - acc) / len));
            out.1.push(false);
        }
        acc += len;
    }
    if !closed && n > 0 {
        out.0.push(pts[n - 1]);
        out.1.push(corner[n - 1]);
    }
    out
}

/// What outlines one variable-width stroke.
struct Ctx<'a> {
    half: f64,
    profile: &'a WidthProfile,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
    tol: f64,
}

/// A vertex of a flattened subpath: where it is, the directions in and out, the lengths of the
/// segments either side and whether it is a corner of the path (else a smooth point).
struct Vertex {
    p: Point,
    a: Vec2,
    b: Vec2,
    la: f64,
    lb: f64,
    corner: bool,
}

impl Ctx<'_> {
    fn subpath(&self, out: &mut BezPath, pts: &[Point], corner: &[bool], closed: bool, span: (f64, f64)) {
        let (pts, corner) = densify(pts, corner, closed, self.profile, span);
        let n = pts.len();
        let seg_count = if closed { n } else { n - 1 };
        let seg_dir: Vec<Vec2> = (0..seg_count).map(|i| unit(pts[(i + 1) % n] - pts[i])).collect();
        let mut cum = vec![0.0; n + 1];
        for i in 0..seg_count {
            cum[i + 1] = cum[i] + pts[i].distance(pts[(i + 1) % n]);
        }
        let total = cum[seg_count];
        if total <= 1e-12 {
            return;
        }
        // Left and right offsets just before and just after `d` along this piece (they differ
        // at a discontinuous point).
        let width_at = |d: f64| {
            let t = span.0 + (span.1 - span.0) * d / total;
            let (b, a) = self.profile.around(if t > 1.0 { t - 1.0 } else { t });
            let side = |(l, r): (f64, f64)| (self.half * l.max(0.0), self.half * r.max(0.0));
            (side(b), side(a))
        };
        let mut lpts = Vec::with_capacity(n);
        let mut rpts = Vec::with_capacity(n);
        for (i, &p) in pts.iter().enumerate() {
            let (before, after) = width_at(cum[i]);
            if !closed && (i == 0 || i == n - 1) {
                let nm = left(seg_dir[i.min(seg_count - 1)]);
                let (wl, wr) = if i == 0 { after } else { before };
                lpts.push(p + nm * wl);
                rpts.push(p - nm * wr);
                continue;
            }
            let ip = if i == 0 { seg_count - 1 } else { i - 1 };
            let v = Vertex { p, a: seg_dir[ip], b: seg_dir[i], la: cum[ip + 1] - cum[ip], lb: cum[i + 1] - cum[i], corner: corner[i] };
            // A step: the side goes straight out (or in) from the width before to the one after.
            for (k, (wl, wr)) in [before, after].into_iter().enumerate() {
                if k == 0 || (wl, wr) != before {
                    self.join(&mut lpts, &v, wl, 1.0);
                    self.join(&mut rpts, &v, wr, -1.0);
                }
            }
        }
        if closed {
            polygon(out, lpts.into_iter());
            polygon(out, rpts.into_iter().rev());
            return;
        }
        let (t0, t1) = (seg_dir[0], seg_dir[seg_count - 1]);
        let ((wl0, wr0), (wl1, wr1)) = (width_at(0.0).1, width_at(total).0);
        let mut ring = lpts;
        // End cap: from the left side around the end to the right side.
        cap_points(&mut ring, pts[n - 1], left(t1), t1, wl1, wr1, self.cap);
        ring.extend(rpts.into_iter().rev());
        // Start cap: from the right side around the start back to the left side.
        cap_points(&mut ring, pts[0], -left(t0), -t0, wr0, wl0, self.cap);
        polygon(out, ring.into_iter());
    }

    /// The offset points at `v` on one side (`side` 1: left, −1: right), `w` from the path: the
    /// join on the outer side of a turn, the inner miter point (or a detour through the vertex
    /// when that would overshoot a segment) on the inner side.
    fn join(&self, pts: &mut Vec<Point>, v: &Vertex, w: f64, side: f64) {
        let (na, nb) = (left(v.a) * side, left(v.b) * side);
        let mid = na + nb;
        // Cosine of half the turn; the miter point is `w / c` out along `mid`.
        let c = mid.hypot() / 2.0;
        let miter = |pts: &mut Vec<Point>| pts.push(v.p + mid * (w / (2.0 * c * c)));
        if w <= 0.0 {
            pts.push(v.p);
        } else if v.b.dot(na) <= 0.0 {
            // Outer side: the path turns away from it.
            let excess = if c > 1e-9 { w * (1.0 / c - 1.0) } else { f64::INFINITY };
            match self.join {
                _ if excess <= self.tol => miter(pts),
                LineJoin::Miter if v.corner && 1.0 / c <= self.miter_limit => miter(pts),
                LineJoin::Miter | LineJoin::Bevel if v.corner => pts.extend([v.p + na * w, v.p + nb * w]),
                _ => self.arc(pts, v, na, nb, w),
            }
        } else if c > 1e-9 && w * (1.0 - c * c).sqrt() / c <= v.la.min(v.lb) {
            miter(pts);
        } else {
            pts.extend([v.p + na * w, v.p, v.p + nb * w]);
        }
    }

    /// A round join from `na` to `nb` (unit normals) at radius `w` around `v`; a full reversal
    /// rounds through the direction of travel.
    fn arc(&self, pts: &mut Vec<Point>, v: &Vertex, na: Vec2, nb: Vec2, w: f64) {
        let phi =
            if (na + nb).hypot() < 1e-9 { if Vec2::new(-na.y, na.x).dot(v.a) > 0.0 { PI } else { -PI } } else { na.cross(nb).atan2(na.dot(nb)) };
        let step = 2.0 * (1.0 - (self.tol / w).min(1.0)).acos();
        let steps = ((phi.abs() / step.max(1e-3)).ceil() as usize).clamp(1, 256);
        for k in 0..=steps {
            let (s, c) = (phi * k as f64 / steps as f64).sin_cos();
            pts.push(v.p + Vec2::new(na.x * c - na.y * s, na.x * s + na.y * c) * w);
        }
    }
}

/// Cap points between `c + u·r0` and `c − u·r1`, bulging towards `v`.
fn cap_points(ring: &mut Vec<Point>, c: Point, u: Vec2, v: Vec2, r0: f64, r1: f64, cap: LineCap) {
    match cap {
        LineCap::Butt => {}
        LineCap::Square => {
            let h = (r0 + r1) / 2.0;
            ring.push(c + u * r0 + v * h);
            ring.push(c - u * r1 + v * h);
        }
        LineCap::Round => {
            let steps = 16;
            for k in 1..steps {
                let a = std::f64::consts::PI * k as f64 / steps as f64;
                let r = r0 + (r1 - r0) * (k as f64 / steps as f64);
                ring.push(c + u * (a.cos() * r) + v * (a.sin() * r));
            }
        }
    }
}

fn polygon(out: &mut BezPath, mut pts: impl Iterator<Item = Point>) {
    let Some(first) = pts.next() else { return };
    out.move_to(first);
    for p in pts {
        out.line_to(p);
    }
    out.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn line() -> BezPath {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((100.0, 0.0));
        b
    }

    fn area(b: &BezPath) -> f64 {
        b.area().abs()
    }

    fn capped(cap: LineCap) -> StrokeLayer {
        let mut st = StrokeLayer::new(vectorcraft_color::Paint::None, 10.0);
        st.cap = cap;
        st
    }

    #[test]
    fn uniform_profile_matches_a_plain_stroke() {
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
        assert!((area(&width_outline(&line(), 10.0, &p, &capped(LineCap::Butt), 0.01)) - 1000.0).abs() < 1e-6);
        let round = area(&width_outline(&line(), 10.0, &p, &capped(LineCap::Round), 0.01));
        let want = 1000.0 + std::f64::consts::PI * 25.0;
        assert!((round - want).abs() / want < 0.01, "{round} vs {want}");
        assert!((area(&width_outline(&line(), 10.0, &p, &capped(LineCap::Square), 0.01)) - 1100.0).abs() < 1e-6);
    }

    #[test]
    fn lens_profile_is_a_diamond_and_one_sided_profiles_stay_on_their_side() {
        let a = area(&width_outline(&line(), 10.0, &WidthProfile::lens(), &capped(LineCap::Butt), 0.01));
        assert!((a - 500.0).abs() < 1e-6, "{a}");
        let one = WidthProfile { points: vec![(0.0, 1.0, 0.0), (1.0, 1.0, 0.0)] };
        let o = width_outline(&line(), 10.0, &one, &capped(LineCap::Butt), 0.01);
        assert!((area(&o) - 500.0).abs() < 1e-6);
        let bb = o.bounding_box();
        // Travelling +x, the left side is up (negative y).
        assert!(bb.y0 < -4.99 && bb.y1 <= 1e-9, "{bb:?}");
    }

    #[test]
    fn a_discontinuous_point_steps_the_width() {
        // 5 pt either side for the first half, 10 pt for the second: a step at the middle.
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 2.0, 2.0)] };
        let o = width_outline(&line(), 10.0, &p, &capped(LineCap::Butt), 0.01);
        assert!((area(&o) - 1500.0).abs() < 1e-6, "{}", area(&o));
        // No ramp: just before the middle the outline is still 5 pt from the line.
        assert!(!o.contains(kurbo::Point::new(49.0, -6.0)) && o.contains(kurbo::Point::new(51.0, -6.0)));
        // A step at a corner of a polyline renders too.
        let mut l = line();
        l.line_to((100.0, 100.0));
        let a = area(&width_outline(&l, 10.0, &p, &capped(LineCap::Butt), 0.01));
        assert!(a.is_finite() && (a - 3000.0).abs() < 150.0, "{a}");
    }

    #[test]
    fn closed_paths_make_a_ring() {
        let mut sq = BezPath::new();
        sq.move_to((0.0, 0.0));
        sq.line_to((100.0, 0.0));
        sq.line_to((100.0, 100.0));
        sq.line_to((0.0, 100.0));
        sq.close_path();
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
        let a = area(&width_outline(&sq, 10.0, &p, &capped(LineCap::Butt), 0.01));
        assert!((a - (110.0 * 110.0 - 90.0 * 90.0)).abs() < 1e-6, "{a}");
    }

    #[test]
    fn curves_are_offset_by_the_profile() {
        // A circle of radius 50 with a uniform 10 pt stroke: ring area 2π·50·10.
        let c = kurbo::Circle::new((0.0, 0.0), 50.0).to_path(1e-4);
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
        let a = area(&width_outline(&c, 10.0, &p, &capped(LineCap::Butt), 0.001));
        let want = 2.0 * std::f64::consts::PI * 50.0 * 10.0;
        assert!((a - want).abs() / want < 0.01, "{a} vs {want}");
    }
}
