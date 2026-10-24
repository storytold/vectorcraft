//! Variable-width strokes (Width tool / width profiles).
//!
//! A stroke with a [`WidthProfile`] is drawn as a filled outline: each subpath is flattened, and
//! at every sample the left and right offsets `width/2 · factor(t)` (t = fraction of the
//! subpath's length) are placed along the normal. Corners of the path take the stroke's miter,
//! round or bevel join on their outer side (smooth points offset along the curve's normal), open
//! subpaths get butt, round or projecting caps (the round cap blends the two side widths), and
//! closed subpaths become two loops of opposite orientation filled with the non-zero rule. Dashes
//! are pieces of a longer subpath and read the profile where they sit along it ([`outline_spans`]).

use std::f64::consts::PI;

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, PathEl, PathSeg, Point, Vec2};
use vectorcraft_doc::{LineCap, LineJoin, StrokeLayer, WidthProfile};
use vectorcraft_pathops::fit_cubics;

use super::{kink, segments, subpaths, tangent, unit};

/// Filled outline of `bp` stroked with `width` scaled by `profile`, with the caps, joins and miter
/// limit of `st`, flattened to `tol`.
pub fn width_outline(bp: &BezPath, width: f64, profile: &WidthProfile, st: &StrokeLayer, tol: f64) -> BezPath {
    outline_spans(bp, &[], width, profile, st, tol)
}

/// [`width_outline`] of pieces of longer subpaths: subpath `i` of `bp` covers the fractions
/// `spans[i]` of the subpath it was cut from (see [`Dashed::spans`](super::Dashed::spans));
/// subpaths without a span are whole.
pub(super) fn outline_spans(bp: &BezPath, spans: &[(f64, f64)], width: f64, profile: &WidthProfile, st: &StrokeLayer, tol: f64) -> BezPath {
    let ctx = Ctx {
        half: width / 2.0,
        profile,
        cap: st.cap,
        join: st.join,
        miter_limit: st.miter_limit,
        tol: tol.max(1e-4),
        uniform: profile.points.first().is_some_and(|f| profile.points.iter().all(|p| p.1 == f.1 && p.2 == f.2)),
    };
    let mut out = BezPath::new();
    for (i, (r, closed)) in subpaths(bp).into_iter().enumerate() {
        let segs = segments(&bp.elements()[r]);
        let flat = flatten(&segs, closed, ctx.tol);
        if flat.pts.len() > 1 {
            ctx.subpath(&mut out, &flat, &segs, closed, spans.get(i).copied().unwrap_or((0.0, 1.0)));
        }
    }
    out
}

/// Left normal in y-down space (left of the direction of travel).
pub(super) fn left(t: Vec2) -> Vec2 {
    Vec2::new(t.y, -t.x)
}

/// A flattened subpath: its points, which of them are corners, and the source segment and
/// parameter range that drew each flattened piece (so a sample added between two points can read
/// that piece's tangent instead of the chord's direction).
pub(super) struct Flat {
    pub(super) pts: Vec<Point>,
    pub(super) corner: Vec<bool>,
    pub(super) seg_of: Vec<usize>,
    /// The source segment's parameter range for each flattened piece. Restricting the tangent
    /// search to this range keeps a sample on the branch that drew it when a cubic loops back
    /// near itself.
    pub(super) param: Vec<(f64, f64)>,
}

/// The flattened points of one subpath (consecutive duplicates removed; a closed one doesn't
/// repeat its start), each marked when it is a corner of the path.
pub(super) fn flatten(segs: &[PathSeg], closed: bool, tol: f64) -> Flat {
    let Some(s0) = segs.first() else { return Flat { pts: vec![], corner: vec![], seg_of: vec![], param: vec![] } };
    let mut flat = Flat { pts: vec![s0.start()], corner: vec![false], seg_of: vec![], param: vec![] };
    // The last segment that drew something, and the last parameter it reached.
    let mut prev: Option<&PathSeg> = None;
    let mut last_added: Option<(usize, f64)> = None;
    for (si, seg) in segs.iter().enumerate() {
        let at = flat.pts.len() - 1;
        let before = flat.pts.len();
        let mut push = |p: Point| {
            if flat.pts.last().is_none_or(|l| l.distance(p) > 1e-9) {
                flat.pts.push(p);
                flat.corner.push(false);
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
        if flat.pts.len() > before {
            let end = flat.pts.len() - 1;
            flat.seg_of.resize(end, si);
            for slot in flat.seg_of.iter_mut().take(end).skip(at) {
                *slot = si;
            }
            // Find each new point's source parameter, never going backwards. A point is on the
            // curve, so its nearest parameter is exact except at a self-intersection, where the
            // monotone search picks the branch that continues this flattened piece.
            let mut ts = Vec::with_capacity(end - at + 1);
            ts.push(0.0);
            let mut t = 0.0;
            for p in &flat.pts[at + 1..=end] {
                if t >= 1.0 - 1e-12 {
                    t = 1.0;
                } else {
                    let sub = seg.subsegment(t..1.0);
                    let nr = sub.nearest(*p, 1e-9);
                    t = (t + (1.0 - t) * nr.t).clamp(0.0, 1.0);
                }
                ts.push(t);
            }
            flat.param.extend(ts.windows(2).map(|w| (w[0], w[1])));
            last_added = ts.last().map(|t| (si, *t));
            if let Some(p) = prev {
                flat.corner[at] = kink(p, seg);
            }
            prev = Some(seg);
        }
    }
    if closed {
        if flat.pts.len() > 1 && flat.pts[0].distance(flat.pts[flat.pts.len() - 1]) <= 1e-9 {
            flat.pts.pop();
            flat.corner.pop();
        }
        if let (Some(a), Some(b)) = (segs.last(), segs.first()) {
            flat.corner[0] = kink(a, b);
        }
        // The closing piece comes from the last source segment even when its endpoint is the
        // already-present start point and did not add a flattened point. Keep a tangent (and its
        // whole parameter range) for it.
        flat.seg_of.truncate(flat.pts.len());
        flat.param.truncate(flat.pts.len());
        if flat.seg_of.len() < flat.pts.len() {
            let last = segs.len().saturating_sub(1);
            // A closing curve with interior flattened points has already reached a parameter
            // before its end (the duplicated start point was not added); the missing piece starts
            // where it left off, not at 0.
            let start = match last_added {
                Some((si, t)) if si == last => t,
                _ => 0.0,
            };
            flat.seg_of.resize(flat.pts.len(), last);
            flat.param.resize(flat.pts.len(), (start, 1.0));
        }
    }
    flat
}

/// Insert samples where the profile has width points and between them, so its smooth curve is
/// drawn as a smooth outline along straight runs too (the two points of a discontinuous point
/// share one sample). `span` maps the subpath onto the profile (see [`outline_spans`]).
fn densify(flat: &Flat, segs: &[PathSeg], closed: bool, profile: &WidthProfile, span: (f64, f64)) -> (Vec<Point>, Vec<bool>, Vec<Vec2>) {
    // Two samples per profile segment: the segment end plus its midpoint. The outline is fitted
    // with cubics through these points, so a smooth width curve needs far fewer samples than it
    // did when every pair of samples became a separate cubic. A profile with a strong bend gets
    // one more subdivision; a flat profile draws the same straight pieces as before.
    const SAMPLES: usize = 2;
    const MAX_SAMPLES: usize = 8;
    const BEND: f64 = 0.20;
    let total = length(&flat.pts, closed);
    let range = span.1 - span.0;
    if total <= 1e-12 || range <= 1e-12 {
        return split_at(flat, closed, &[], segs);
    }
    // Where the profile's points fall along this piece (a span past 1 wraps round a closed path).
    let mut ts: Vec<f64> = profile.points.iter().flat_map(|p| [p.0, p.0 + 1.0]).collect();
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    // A flat profile draws the same straight pieces as before: sample only its points.
    let constant = profile.points.first().is_some_and(|f| profile.points.iter().all(|p| (p.1 - f.1).abs() <= 1e-12 && (p.2 - f.2).abs() <= 1e-12));
    let cap = (4096 / ts.len().max(1)).max(1);
    // A curved path's flattened points already sample the width along the subpath. On a dense
    // piece the profile's own points are enough; a straight run has only its ends, so it needs
    // the extra samples. This keeps curved, profiled art close to a uniform stroke's size.
    let pieces = if closed { flat.pts.len() } else { flat.pts.len().saturating_sub(1) }.max(1);
    let spacing = total / pieces as f64;
    let mut marks = Vec::with_capacity(ts.len() * SAMPLES);
    for w in ts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let samples = if constant {
            1
        } else {
            // `ts` has the profile both at t and t + 1 for a dash crossing a closed seam; the
            // shifted copy has the same shape, so measure its bend on the unshifted segment.
            let start = a.rem_euclid(1.0);
            let end = start + (b - a);
            let want = adaptive_samples(profile, start, end, SAMPLES, MAX_SAMPLES, BEND).min(cap);
            let existing = (b - a) * total / spacing;
            if existing >= want as f64 { 1 } else { want }
        };
        for k in 0..samples {
            marks.push(a + (b - a) * k as f64 / samples as f64);
        }
    }
    if let Some(&last) = ts.last() {
        marks.push(last);
    }
    let marks: Vec<f64> = marks.into_iter().map(|t| (t - span.0) / range * total).collect();
    split_at(flat, closed, &marks, segs)
}

/// How many pieces one profile segment needs so a cubic fit through their ends keeps the width
/// curve. A straight taper is already straight; a visible bend gets progressively more samples.
fn adaptive_samples(profile: &WidthProfile, t0: f64, t1: f64, base: usize, max: usize, bend: f64) -> usize {
    let mut samples = base;
    let (l0, r0) = profile.at(t0);
    let (l1, r1) = profile.at(t1);
    while samples < max {
        let mut dev = 0.0_f64;
        for q in [0.25, 0.5, 0.75] {
            let (l, r) = profile.at(t0 + (t1 - t0) * q);
            let want = (l0 + (l1 - l0) * q, r0 + (r1 - r0) * q);
            dev = dev.max((l - want.0).abs()).max((r - want.1).abs());
        }
        if dev <= bend {
            break;
        }
        samples *= 2;
    }
    samples
}

/// Length of a flattened subpath (see [`flatten`]).
pub(super) fn length(pts: &[Point], closed: bool) -> f64 {
    let n = pts.len();
    let seg_count = if closed { n } else { n.saturating_sub(1) };
    (0..seg_count).map(|i| pts[i].distance(pts[(i + 1) % n])).sum()
}

/// A flattened subpath (see [`flatten`]) with points added `marks` along it (distances from its
/// start; those on a point or off the subpath are ignored).
pub(super) fn split_at(flat: &Flat, closed: bool, marks: &[f64], segs: &[PathSeg]) -> (Vec<Point>, Vec<bool>, Vec<Vec2>) {
    let Flat { pts, corner, seg_of, param } = flat;
    let n = pts.len();
    let seg_count = if closed { n } else { n.saturating_sub(1) };
    // The source's direction at a source parameter. The chord direction is the secant, not the
    // tangent: using it for points added inside a chord jogs the normal at every flatten vertex.
    let source_at = |si: usize, t: f64| -> Vec2 { segs.get(si).map_or(Vec2::new(1.0, 0.0), |seg| tangent(seg, t)) };
    // A point added inside a flattened piece searches only that piece's source parameter range:
    // a looping cubic can pass near itself, and a full-segment nearest would take the other
    // branch's tangent.
    let source_near = |p: Point, i: usize| -> Vec2 {
        let Some(si) = seg_of.get(i).copied() else { return Vec2::new(1.0, 0.0) };
        let Some(seg) = segs.get(si) else { return Vec2::new(1.0, 0.0) };
        let (t0, t1) = param.get(i).copied().unwrap_or((0.0, 1.0));
        if t1 - t0 > 1e-12 {
            let sub = seg.subsegment(t0..t1);
            tangent(seg, t0 + (t1 - t0) * sub.nearest(p, 1e-9).t)
        } else {
            tangent(seg, t0)
        }
    };
    // At a shared point, average the arriving and leaving directions unless it is a corner.
    let dir_at = |i: usize| -> Vec2 {
        let incoming = if i == 0 {
            if closed { seg_of.last().copied().zip(param.last().copied()) } else { None }
        } else {
            seg_of.get(i - 1).copied().zip(param.get(i - 1).copied())
        };
        let outgoing = seg_of.get(i).copied().zip(param.get(i).copied());
        let a = incoming.map(|(si, (_, t))| source_at(si, t));
        let b = outgoing.map(|(si, (t, _))| source_at(si, t));
        match (a, b) {
            (Some(a), Some(b)) if !corner[i] => {
                let sum = a + b;
                if sum.hypot() > 1e-9 { unit(sum) } else { b }
            }
            (_, Some(b)) => b,
            (Some(a), None) => a,
            (None, None) => Vec2::new(1.0, 0.0),
        }
    };
    let mut out = (Vec::with_capacity(n + marks.len()), Vec::with_capacity(n + marks.len()), Vec::with_capacity(n + marks.len()));
    let mut acc = 0.0;
    for i in 0..seg_count {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = a.distance(b);
        out.0.push(a);
        out.1.push(corner[i]);
        out.2.push(dir_at(i));
        let mut inner: Vec<f64> = marks.iter().copied().filter(|d| *d > acc + 1e-9 && *d < acc + len - 1e-9).collect();
        inner.sort_by(f64::total_cmp);
        inner.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
        for d in inner {
            let p = a.lerp(b, (d - acc) / len);
            out.0.push(p);
            out.1.push(false);
            out.2.push(source_near(p, i));
        }
        acc += len;
    }
    if !closed && n > 0 {
        out.0.push(pts[n - 1]);
        out.1.push(corner[n - 1]);
        out.2.push(dir_at(n - 1));
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
    /// The profile has one (left, right) width everywhere.
    uniform: bool,
}

/// A vertex of a flattened subpath: where it is, the directions in and out, the lengths of the
/// segments either side and whether it is a corner of the path (else a smooth point).
struct Vertex {
    p: Point,
    a: Vec2,
    b: Vec2,
    /// The smooth tangent: the source curve's direction at this sample, or the average of its
    /// arriving and leaving directions at a smooth join. A smooth point offsets along this
    /// normal, not the chord bisector; otherwise the chord normal inside each flattened piece and
    /// the bisector at each flatten vertex alternate and the edge is scalloped.
    n: Vec2,
    la: f64,
    lb: f64,
    corner: bool,
}

impl Ctx<'_> {
    fn subpath(&self, out: &mut BezPath, flat: &Flat, segs: &[PathSeg], closed: bool, span: (f64, f64)) {
        let (pts, corner, dirs) = densify(flat, segs, closed, self.profile, span);
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
            // A closed path joins its own start and end: a profile with different widths there
            // is a step, not a stretch across the seam.
            let (b, a) = if closed && span == (0.0, 1.0) && d <= 1e-12 {
                (self.profile.around(1.0).0, self.profile.around(0.0).1)
            } else {
                self.profile.around(if t > 1.0 { t - 1.0 } else { t })
            };
            let side = |(l, r): (f64, f64)| (self.half * l.max(0.0), self.half * r.max(0.0));
            (side(b), side(a))
        };
        let mut lpts: Vec<(Point, bool)> = Vec::with_capacity(n);
        let mut rpts: Vec<(Point, bool)> = Vec::with_capacity(n);
        // Only a butt cap makes the side end at a corner; square and round caps stay tangent to it.
        let end_sharp = matches!(self.cap, LineCap::Butt);
        for (i, &p) in pts.iter().enumerate() {
            let ((wl_b, wr_b), (wl_a, wr_a)) = width_at(cum[i]);
            if !closed && (i == 0 || i == n - 1) {
                let nm = left(dirs[i]);
                let (wl, wr) = if i == 0 { (wl_a, wr_a) } else { (wl_b, wr_b) };
                lpts.push((p + nm * wl, end_sharp));
                rpts.push((p - nm * wr, end_sharp));
                continue;
            }
            let ip = if i == 0 { seg_count - 1 } else { i - 1 };
            let v = Vertex { p, a: seg_dir[ip], b: seg_dir[i], n: dirs[i], la: cum[ip + 1] - cum[ip], lb: cum[i + 1] - cum[i], corner: corner[i] };
            // A step: the side goes straight out (or in) from the width before to the one after.
            // Both ends of that straight piece are corners: the run before it must not round
            // into the step.
            let (step_l, step_r) = (wl_a != wl_b, wr_a != wr_b);
            self.join(&mut lpts, &v, wl_b, 1.0, step_l);
            self.join(&mut rpts, &v, wr_b, -1.0, step_r);
            if step_l {
                self.join(&mut lpts, &v, wl_a, 1.0, true);
            }
            if step_r {
                self.join(&mut rpts, &v, wr_a, -1.0, true);
            }
        }
        if closed {
            polygon(out, lpts.into_iter(), self.tol);
            polygon(out, rpts.into_iter().rev(), self.tol);
            return;
        }
        let (t0, t1) = (dirs[0], dirs[n - 1]);
        let ((wl0, wr0), (wl1, wr1)) = (width_at(0.0).1, width_at(total).0);
        let mut ring = lpts;
        // End cap: from the left side around the end to the right side.
        cap_points(&mut ring, pts[n - 1], left(t1), t1, wl1, wr1, self.cap, self.tol);
        ring.extend(rpts.into_iter().rev());
        // Start cap: from the right side around the start back to the left side.
        cap_points(&mut ring, pts[0], -left(t0), -t0, wr0, wl0, self.cap, self.tol);
        polygon(out, ring.into_iter(), self.tol);
    }

    /// The offset points at `v` on one side (`side` 1: left, −1: right), `w` from the path. A
    /// smooth point follows the vertex normal, the offset of the curve itself; a corner takes the
    /// join on its outer side and the inner miter point (or a detour through the vertex when that
    /// would overshoot a segment) on its inner side.
    ///
    /// Each point says whether the outline is sharp there: a path corner, a miter tip, a width
    /// step or a butt cap end. Only those break the smooth fitting; a rapid width change makes
    /// the offset turn sharply, but it is not a corner and must not be fitted as one.
    fn join(&self, pts: &mut Vec<(Point, bool)>, v: &Vertex, w: f64, side: f64, step: bool) {
        let (na, nb) = (left(v.a) * side, left(v.b) * side);
        let mid = na + nb;
        // Cosine of half the turn; the miter point is `w / c` out along `mid`.
        let c = mid.hypot() / 2.0;
        if w <= 0.0 {
            pts.push((v.p, true));
            return;
        }
        let miter = if c > 1e-9 { Some(v.p + mid * (w / (2.0 * c * c))) } else { None };
        // The offset of the smooth path itself is at `w` along the path normal. The polyline
        // miter (`w / c`) is the offset of the flattened segments, not of the curve: at a tight
        // curve it spiked every sample outward and left a scalloped edge. The source curve's
        // tangent is the smooth normal; a corner still uses its two chords for the join.
        let smooth = v.p + left(v.n) * (w * side);
        if v.b.dot(na) <= 0.0 {
            // Outer side: the path turns away from it.
            if !v.corner {
                // A smooth point has no join: its offset is along the path normal.
                pts.push((smooth, step));
            } else {
                let excess = if c > 1e-9 { w * (1.0 / c - 1.0) } else { f64::INFINITY };
                match self.join {
                    _ if excess <= self.tol => pts.push((miter.unwrap_or(v.p), true)),
                    LineJoin::Miter if c > 1e-9 && 1.0 / c <= self.miter_limit => pts.push((miter.unwrap_or(v.p), true)),
                    LineJoin::Miter | LineJoin::Bevel => pts.extend([(v.p + na * w, true), (v.p + nb * w, true)]),
                    _ => self.arc(pts, v, na, nb, w, step),
                }
            }
        } else if c <= 1e-9 {
            // A reversal has no inner miter; go through the vertex. That is a notch in the
            // outline, so it stays sharp: fitting a curve through it would loop.
            pts.extend([(v.p + na * w, true), (v.p, true), (v.p + nb * w, true)]);
        } else if !v.corner {
            // A smooth point's inner offset follows its normal too. With a constant profile, once
            // the half-width reaches the local radius of curvature the offset crosses the centre of
            // curvature and comes back as a reversed contour, which the non-zero fill rules into a
            // hole. The stroke covers that centre, so cut through it instead: the cut is inside the
            // painted region and does not show. (With varying widths the crossing usually marks a
            // real caustic; a cut there would show as a slit, so leave it to the offset.)
            let turn = 2.0 * c.clamp(-1.0, 1.0).acos();
            let radius = if turn > 1e-9 { 0.5 * (v.la + v.lb) / turn } else { f64::INFINITY };
            if self.uniform && w >= radius {
                pts.push((smooth, true));
                pts.push((v.p, true));
                pts.push((smooth, true));
            } else {
                pts.push((smooth, step));
            }
        } else if w * (1.0 - c * c).sqrt() / c <= v.la.min(v.lb) {
            // A corner's inner offset is its miter while that stays inside both segments.
            pts.push((miter.unwrap_or(v.p), true));
        } else {
            // The inner offset of a corner cuts through the vertex; keep that notch sharp.
            pts.extend([(v.p + na * w, true), (v.p, true), (v.p + nb * w, true)]);
        }
    }

    /// A round join from `na` to `nb` (unit normals) at radius `w` around `v`; a full reversal
    /// rounds through the direction of travel. `sharp` marks a step at the join's ends.
    fn arc(&self, pts: &mut Vec<(Point, bool)>, v: &Vertex, na: Vec2, nb: Vec2, w: f64, sharp: bool) {
        let phi =
            if (na + nb).hypot() < 1e-9 { if Vec2::new(-na.y, na.x).dot(v.a) > 0.0 { PI } else { -PI } } else { na.cross(nb).atan2(na.dot(nb)) };
        let step = 2.0 * (1.0 - (self.tol / w).min(1.0)).acos();
        let steps = ((phi.abs() / step.max(1e-3)).ceil() as usize).clamp(1, 256);
        for k in 0..=steps {
            let (s, c) = (phi * k as f64 / steps as f64).sin_cos();
            pts.push((v.p + Vec2::new(na.x * c - na.y * s, na.x * s + na.y * c) * w, sharp));
        }
    }
}

/// Cap points between `c + u·r0` and `c − u·r1`, bulging towards `v`.
#[allow(clippy::too_many_arguments)]
fn cap_points(ring: &mut Vec<(Point, bool)>, c: Point, u: Vec2, v: Vec2, r0: f64, r1: f64, cap: LineCap, tol: f64) {
    match cap {
        LineCap::Butt => {}
        LineCap::Square => {
            let h = (r0 + r1) / 2.0;
            ring.push((c + u * r0 + v * h, true));
            ring.push((c - u * r1 + v * h, true));
        }
        LineCap::Round => {
            // Enough steps that the cap's facets stay within `tol`, as a round join does.
            let r = r0.max(r1).max(1e-9);
            let step = 2.0 * (1.0 - (tol / r).min(1.0)).acos();
            let steps = ((PI / step.max(1e-6)).ceil() as usize).clamp(8, 256);
            for k in 1..steps {
                let a = PI * k as f64 / steps as f64;
                let r = r0 + (r1 - r0) * (k as f64 / steps as f64);
                ring.push((c + u * (a.cos() * r) + v * (a.sin() * r), false));
            }
        }
    }
}

/// Append `pts` as a closed contour (a width outline's ring or loop), through a smooth run's
/// points as cubic curves. Sampled as line segments, a curved, wide stroke shows the facets of
/// its offset whenever it is magnified; the cubics keep it smooth. A run whose points are
/// collinear stays a line, and the points marked sharp — caps, miter joins, width steps and path
/// corners — stay sharp. A rapid width change turns the offset sharply without being a corner:
/// marking corners here rather than measuring turns keeps that turn smooth.
fn polygon(out: &mut BezPath, pts: impl Iterator<Item = (Point, bool)>, tol: f64) {
    let mut ring: Vec<(Point, bool)> = Vec::new();
    for (p, sharp) in pts {
        if !p.x.is_finite() || !p.y.is_finite() {
            return;
        }
        if let Some(last) = ring.last_mut()
            && last.0.distance(p) <= 1e-9
        {
            last.1 |= sharp;
            continue;
        }
        ring.push((p, sharp));
    }
    if ring.len() > 1
        && ring[0].0.distance(ring[ring.len() - 1].0) <= 1e-9
        && let Some(last) = ring.pop()
    {
        ring[0].1 |= last.1;
    }
    let n = ring.len();
    if n == 0 {
        return;
    }
    if n < 3 {
        out.move_to(ring[0].0);
        if n == 2 {
            out.line_to(ring[1].0);
        }
        out.close_path();
        return;
    }
    // Start at a sharp point when there is one, so no run has to wrap: `smooth_run` starts from
    // whichever point is current, and starting anywhere else would bridge over the skipped arc.
    match ring.iter().position(|&(_, sharp)| sharp) {
        None => {
            // A smooth closed loop: fit across the seam, so it doesn't start with a kink.
            let pts: Vec<Point> = ring.iter().map(|&(p, _)| p).collect();
            out.move_to(pts[0]);
            smooth_run(out, &pts, true, tol);
        }
        Some(first) => {
            out.move_to(ring[first].0);
            // From each sharp point to the next; the sharp points themselves stay sharp.
            let mut i = first;
            loop {
                let mut j = (i + 1) % n;
                while !ring[j].1 {
                    j = (j + 1) % n;
                }
                // A contour with a single sharp point comes back to it: the run is the whole
                // loop, fitted non-cyclically so that point stays sharp.
                let steps = if j == i { n } else { (j + n - i) % n };
                if steps == 1 {
                    out.line_to(ring[j].0);
                } else {
                    let run: Vec<Point> = (0..=steps).map(|k| ring[(i + k) % n].0).collect();
                    smooth_run(out, &run, false, tol);
                }
                if j == first {
                    break;
                }
                i = j;
            }
        }
    }
    out.close_path();
}

/// The derivative of the run at each point, in points per point of chord length. Each coordinate
/// is a shape-preserving cubic Hermite (Fritsch–Carlson): between two points it stays within their
/// coordinate range, so a cubic through a fast width pinch cannot bulge past the pinch and cross
/// the other side, and it has a flat slope at a local extremum. A Catmull–Rom tangent (the secant
/// across the neighbours) does neither: it overshoots a V and, with the uneven sample spacing of a
/// flattened path, leaves a scalloped edge.
fn tangents(run: &[Point], cyclic: bool) -> Vec<Vec2> {
    let n = run.len();
    let segs = if cyclic { n } else { n.saturating_sub(1) };
    let mut h = vec![0.0; segs];
    for (i, hi) in h.iter_mut().enumerate() {
        *hi = run[i].distance(run[(i + 1) % n]);
    }
    let slope = |d0: f64, d1: f64, h0: f64, h1: f64| {
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (w0, w1) = (2.0 * h1 + h0, h1 + 2.0 * h0);
        let m = (w0 + w1) / (w0 / d0 + w1 / d1);
        if m.is_finite() { m } else { 0.0 }
    };
    // The one-sided slope of a run's end uses its two end secants, instead of only the last one.
    // A coarse sample then still leaves the end of a profile curve with the right slope.
    let end_slope = |d0: f64, d1: f64, h0: f64, h1: f64| {
        if d0 * d1 <= 0.0 || h0 + h1 <= 1e-12 {
            return 0.0;
        }
        let m = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
        if m.is_finite() && m * d0 > 0.0 {
            let cap = 3.0 * d0.abs();
            m.clamp(-cap, cap)
        } else {
            0.0
        }
    };
    let delta = |i: usize| -> Vec2 {
        let hi = h[i % segs];
        if hi > 1e-12 { (run[(i + 1) % n] - run[i % n]) / hi } else { Vec2::ZERO }
    };
    let mut out = vec![Vec2::ZERO; n];
    for (i, m) in out.iter_mut().enumerate() {
        let (d0, d1) = (delta((i + n - 1) % n), delta(i % segs));
        let (h0, h1) = (h[(i + n - 1) % segs], h[i % segs]);
        if h0 <= 1e-12 || h1 <= 1e-12 {
            *m = if h1 > 1e-12 { d1 } else { d0 };
        } else if cyclic || (i > 0 && i + 1 < n) {
            *m = Vec2::new(slope(d0.x, d1.x, h0, h1), slope(d0.y, d1.y, h0, h1));
        } else if i == 0 {
            *m = if n >= 3 {
                let (a, b) = (delta(0), delta(1));
                Vec2::new(end_slope(a.x, b.x, h[0], h[1]), end_slope(a.y, b.y, h[0], h[1]))
            } else {
                d1
            };
        } else {
            *m = if n >= 3 {
                let (a, b) = (delta(n - 2), delta(n - 3));
                Vec2::new(end_slope(a.x, b.x, h[n - 2], h[n - 3]), end_slope(a.y, b.y, h[n - 2], h[n - 3]))
            } else {
                d0
            };
        }
        // A shape-preserving slope is flat in a coordinate at that coordinate's local extremum.
        // Where both are extrema at one sample (a loop or a pinch in the sampled edge) the flat
        // tangent would make a line and a kink; the secant across the sample keeps the curve C1
        // there, and the extremum itself is what trims the loop later.
        if m.hypot() <= 1e-12 {
            let prev = if cyclic || i > 0 { run[(i + n - 1) % n] } else { run[i] };
            let next = if cyclic || i + 1 < n { run[(i + 1) % n] } else { run[i] };
            let sec = next - prev;
            if sec.hypot() > 1e-12 {
                *m = sec.normalize();
            }
        }
    }
    out
}

/// Append a run of points as a fitted chain of cubics (or one line when they are collinear).
///
/// A cubic through every flattened point, as this used to emit, keeps the outline smooth but puts
/// three coordinates per point into the file; a small, thin stroke then exports several times the
/// path data of the same stroke without a profile. Fit a chain within the outline tolerance
/// instead: the result is still G1 continuous and stays within `tol` of the sampled edge, but a
/// smooth side is a few cubics rather than one per flatten step. Profile bends that need finer
/// detail produce more cubics; straight runs stay lines.
fn smooth_run(out: &mut BezPath, run: &[Point], cyclic: bool, tol: f64) {
    let n = run.len();
    if n < 2 {
        return;
    }
    if !cyclic && is_straight(run, tol) {
        out.line_to(run[n - 1]);
        return;
    }
    let tans = tangents(run, cyclic);
    let tol = tol.max(1e-6);
    // A short run is one or two profile features, not flattened path detail. Fitting it loosely
    // enough to save path data can let a handle cross the narrow side of a V; keep the
    // shape-preserving per-point cubics there.
    let segs = if cyclic { n } else { n - 1 };
    if n <= 3 {
        for i in 0..segs {
            let j = (i + 1) % n;
            let h = run[i].distance(run[j]);
            let (m0, m1) = (tans[i] * (h / 3.0), tans[j] * (h / 3.0));
            if m0.hypot() <= 1e-12 || m1.hypot() <= 1e-12 {
                out.line_to(run[j]);
            } else {
                out.curve_to(run[i] + m0, run[j] - m1, run[j]);
            }
        }
        return;
    }
    let unit_or = |v: Vec2, fallback: Vec2| {
        let len = v.hypot();
        if len > 1e-12 {
            v / len
        } else {
            let len = fallback.hypot();
            if len > 1e-12 { fallback / len } else { Vec2::new(1.0, 0.0) }
        }
    };
    let t0 = unit_or(tans[0], run[1] - run[0]);
    let (pts, t0, t1) = if cyclic {
        // Fit the seam too: the fit starts and arrives with the same tangent, so closing the
        // contour adds no kink.
        let mut pts = Vec::with_capacity(n + 1);
        pts.extend_from_slice(run);
        pts.push(run[0]);
        (pts, t0, t0)
    } else {
        (run.to_vec(), t0, unit_or(tans[n - 1], run[n - 1] - run[n - 2]))
    };
    // Fit tighter than the flattening tolerance: the fit's error is the whole outline's error,
    // and a least-squares fit may sit on one side of the sampled edge, which would otherwise
    // shift a profile's area visibly.
    let cubics = fit_cubics(&pts, t0, t1, tol * 0.1);
    if cubics.is_empty() {
        out.line_to(run[n - 1]);
        return;
    }
    for c in cubics {
        out.curve_to(c.p1, c.p2, c.p3);
    }
}

/// Are the run's points within the flattening tolerance of its chord?
fn is_straight(run: &[Point], tol: f64) -> bool {
    let (a, b) = (run[0], run[run.len() - 1]);
    let d = b - a;
    let l = d.hypot();
    if l <= 1e-12 {
        return false;
    }
    run.iter().all(|p| (d.cross(*p - a) / l).abs() <= tol.max(1e-6) * 0.25)
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

    /// The lens eases up to full width and back down (a parabolic bump averaging two thirds of
    /// the width); a one-sided profile keeps its side at zero and stays on the other.
    #[test]
    fn lens_profile_is_a_smooth_bump_and_one_sided_profiles_stay_on_their_side() {
        let a = area(&width_outline(&line(), 10.0, &WidthProfile::lens(), &capped(LineCap::Butt), 0.01));
        assert!((a - 2000.0 / 3.0).abs() < 0.1, "{a}");
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
    fn a_closed_path_steps_where_its_profile_ends_differ() {
        let mut sq = BezPath::new();
        sq.move_to((0.0, 0.0));
        sq.line_to((100.0, 0.0));
        sq.line_to((100.0, 100.0));
        sq.line_to((0.0, 100.0));
        sq.close_path();
        // 5 pt at the seam's start, 10 pt at its end: the ring must step between them, not
        // stretch the last width point round the corner.
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 2.0, 2.0)] };
        let o = width_outline(&sq, 10.0, &p, &capped(LineCap::Butt), 0.01);
        let at = |p: Point, x: f64, y: f64| (p.x - x).abs() < 1e-9 && (p.y - y).abs() < 1e-9;
        let step = o.elements().windows(2).any(|w| match (w[0], w[1]) {
            (PathEl::MoveTo(a), PathEl::LineTo(b)) | (PathEl::LineTo(a), PathEl::LineTo(b)) => {
                at(a, -10.0, -10.0) && at(b, -5.0, -5.0) || at(a, -5.0, -5.0) && at(b, -10.0, -10.0)
            }
            _ => false,
        });
        assert!(step, "{o:?}");
    }

    #[test]
    fn added_samples_follow_the_curve_not_the_flattened_chord() {
        // A cubic with a tight S: its chords are far from its tangents. A sample inserted inside a
        // chord must take the curve's direction there; the chord's direction is what made a wide
        // profile scallop at every flatten vertex.
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((0.0, 120.0), (120.0, -120.0), (120.0, 0.0));
        let segs = segments(b.elements());
        let flat = flatten(&segs, false, 2.0);
        assert!(flat.pts.len() >= 3);
        let mut marks = vec![];
        let mut acc = 0.0;
        for i in 0..flat.pts.len() - 1 {
            let len = flat.pts[i].distance(flat.pts[i + 1]);
            marks.push(acc + len / 2.0);
            acc += len;
        }
        let (all, _, dirs) = split_at(&flat, false, &marks, &segs);
        assert!(all.len() > flat.pts.len());
        let seg = segs[0];
        for (p, d) in all.iter().zip(&dirs) {
            let want = tangent(&seg, seg.nearest(*p, 1e-9).t);
            assert!(d.dot(want) > 0.999, "{d:?} vs {want:?} at {p:?}");
        }
    }

    #[test]
    fn a_wiggly_profile_on_a_curve_fits_without_kinks() {
        // A rapid width change turns the offset sharply; that is not a path corner, so the
        // outline must stay tangent-continuous through it (the old turn-angle test did not).
        let c = kurbo::Circle::new((0.0, 0.0), 50.0).to_path(1e-3);
        let p = WidthProfile {
            points: vec![(0.0, 1.0, 1.0), (0.15, 3.0, 0.4), (0.3, 0.2, 2.8), (0.5, 2.5, 0.3), (0.7, 0.4, 2.2), (0.85, 2.0, 0.5), (1.0, 1.0, 1.0)],
        };
        let o = width_outline(&c, 10.0, &p, &capped(LineCap::Butt), 0.01);
        let mut cursor = None;
        let mut prev: Option<Vec2> = None;
        let mut kinks = 0;
        let mut check = |a: Vec2, b: Vec2| {
            let (la, lb) = (a.hypot(), b.hypot());
            if la > 1e-9 && lb > 1e-9 && (a.dot(b) <= 0.0 || a.cross(b).abs() / (la * lb) > 1e-6) {
                kinks += 1;
            }
        };
        for el in o.elements() {
            match el {
                PathEl::MoveTo(p) => {
                    cursor = Some(*p);
                    prev = None;
                }
                PathEl::LineTo(p) => {
                    if let Some(q) = cursor {
                        let d = *p - q;
                        if let Some(a) = prev {
                            check(a, d);
                        }
                        prev = Some(d);
                    }
                    cursor = Some(*p);
                }
                PathEl::CurveTo(a, b, p) => {
                    if let Some(q) = cursor {
                        let d = if a.distance(q) > 1e-9 { *a - q } else { *b - q };
                        if let Some(prev) = prev {
                            check(prev, d);
                        }
                        prev = Some(if p.distance(*b) > 1e-9 { *p - *b } else { *p - *a });
                    }
                    cursor = Some(*p);
                }
                PathEl::QuadTo(_, _) | PathEl::ClosePath => {}
            }
        }
        assert_eq!(kinks, 0, "{o:?}");
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

    #[test]
    fn a_uniform_circle_narrower_than_its_stroke_has_no_hole() {
        // Half-width 15 on a radius-5 circle: the inner offset crosses the centre. The constant
        // stroke still covers the centre, so the outline must not leave a hole there.
        let c = kurbo::Circle::new((0.0, 0.0), 5.0).to_path(1e-3);
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
        let o = width_outline(&c, 30.0, &p, &capped(LineCap::Butt), 0.01);
        assert!(o.contains(Point::new(0.0, 0.0)), "{o:?}");
    }

    #[test]
    fn a_quick_width_change_does_not_overshoot_its_samples() {
        // Width falls to a point and comes back, unevenly on either side: the fitted edge must not
        // dip past the point (that crossed the other edge and left a scalloped neck).
        let mut run = vec![Point::new(0.0, 0.0), Point::new(10.0, -1.0), Point::new(20.0, -0.3)];
        let (mut a, mut b) = (BezPath::new(), BezPath::new());
        a.move_to(run[0]);
        b.move_to(run[0]);
        smooth_run(&mut a, &run, false, 0.0);
        // Reverse the same points: the fit must stay inside the V from either direction too.
        run.reverse();
        smooth_run(&mut b, &run, false, 0.0);
        for path in [a, b] {
            let mut start = Point::ZERO;
            for el in path.elements() {
                match el {
                    PathEl::MoveTo(p) => start = *p,
                    PathEl::CurveTo(c1, c2, p) => {
                        for c in [c1, c2] {
                            assert!(c.y >= -1.0 - 1e-9, "{c:?} in {path:?}");
                            assert!(c.y <= start.y.max(p.y) + 1e-9, "{c:?} in {path:?}");
                        }
                        start = *p;
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn added_samples_stay_on_their_flattened_branch() {
        // This cubic loops back near itself. At a coarse tolerance a chord midpoint can be closer
        // to the other branch; a full-segment nearest would then take that branch's tangent and
        // spike the offset. Each inserted sample must use the parameter range of its own piece.
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((150.0, 100.0), (-50.0, 100.0), (100.0, 0.0));
        let segs = segments(b.elements());
        let seg = segs[0];
        let flat = flatten(&segs, false, 2.0);
        let mut acc = 0.0;
        let mut divergent = 0;
        for i in 0..flat.pts.len() - 1 {
            let len = flat.pts[i].distance(flat.pts[i + 1]);
            if len <= 1e-12 {
                continue;
            }
            let (t0, t1) = flat.param[i];
            let sub = seg.subsegment(t0..t1);
            for k in 1..16 {
                let (out, _, dirs) = split_at(&flat, false, &[acc + len * k as f64 / 16.0], &segs);
                assert!(out.len() >= i + 2, "no sample inserted in piece {i}");
                let p = out[i + 1];
                let want = tangent(&seg, t0 + (t1 - t0) * sub.nearest(p, 1e-9).t);
                assert!(dirs[i + 1].dot(want) > 0.999, "piece {i}, k {k}: {:?} vs {want:?}", dirs[i + 1]);
                let full = tangent(&seg, seg.nearest(p, 1e-9).t);
                if full.dot(want) < 0.9 {
                    divergent += 1;
                }
            }
            acc += len;
        }
        assert!(divergent > 0, "the fixture did not cross the other branch");
    }

    #[test]
    fn a_coarse_wide_circle_keeps_its_offset() {
        // A wide stroke flattened coarsely: offsetting each flattened segment by its miter
        // (`w / cos(θ/2)`) scalloped the circle outward at every sample. Smooth points follow the
        // curve's normal instead, so the ring is the annulus between radii 10 and 30.
        let c = kurbo::Circle::new((0.0, 0.0), 20.0).to_path(0.5);
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
        let a = area(&width_outline(&c, 20.0, &p, &capped(LineCap::Butt), 0.5));
        let want = std::f64::consts::PI * (30.0 * 30.0 - 10.0 * 10.0);
        assert!((a - want).abs() / want < 0.01, "{a} vs {want}");
    }

    #[test]
    fn profiled_outlines_stay_compact() {
        // A smooth profile is fitted with cubics, so it must not turn every profile sample into
        // exported path data. Compare it with the same curved path's uniform outline.
        let circle = kurbo::Circle::new((0.0, 0.0), 5.0).to_path(0.01);
        let count = |points| {
            let p = WidthProfile { points };
            width_outline(&circle, 5.0, &p, &capped(LineCap::Butt), 0.01).elements().len()
        };
        let uniform = count(vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)]);
        let lens = count(WidthProfile::lens().points);
        let wave = count(vec![(0.0, 0.3, 0.3), (0.25, 1.0, 1.0), (0.5, 0.3, 0.3), (0.75, 1.0, 1.0), (1.0, 0.3, 0.3)]);
        assert!(lens <= uniform * 3 / 2, "lens {lens} vs uniform {uniform}");
        assert!(wave <= uniform * 3 / 2, "wave {wave} vs uniform {uniform}");
    }
}
