//! Arc-length parameterised polylines ("tracks") that brushes lay art along.
//!
//! A track is one flattened subpath. `frame(s)` gives the point and unit tangent at arc length
//! `s`. Along a flattened curve the frame runs on a smooth curve through the vertices, so warped
//! art bends smoothly; along a line of the path it stays on the line. At real corners the tangent
//! stays discontinuous, so art folds there.

use vectorcraft_geom::{BezPath, PathEl, Point, Vec2};

/// Turn angle (degrees) above which a vertex is a corner.
pub const CORNER_DEG: f64 = 30.0;

#[derive(Clone, Debug)]
pub struct Track {
    /// Vertices; closed tracks repeat the first vertex at the end.
    pub verts: Vec<Point>,
    /// Cumulative arc length at each vertex.
    pub cum: Vec<f64>,
    pub closed: bool,
    /// Unit direction of each segment.
    pub dirs: Vec<Vec2>,
    /// Smoothed unit tangent at each vertex.
    pub tangents: Vec<Vec2>,
    /// Is the vertex a corner (turn > [`CORNER_DEG`])? Open ends are never corners.
    pub corner: Vec<bool>,
    /// Does the segment come from flattening a curve (rather than from a line of the path)?
    pub curved: Vec<bool>,
    /// How far (radians) a curved segment's end tangents may turn from its direction for `frame`
    /// to follow a curve through them; 0 for a line.
    turn: Vec<f64>,
}

fn unit(v: Vec2) -> Vec2 {
    let l = v.hypot();
    if l < 1e-12 { Vec2::new(1.0, 0.0) } else { v / l }
}

/// Whether two unit vectors point the same way, to within the slack a unit vector carries after
/// a few multiplications (a bend of even a thousandth of a degree fails this).
fn aligned(a: Vec2, b: Vec2) -> bool {
    a.dot(b) > 1.0 - 1e-12
}

/// How far a vertex tangent may turn from a chord of a flattened curve, as a multiple of the
/// flattening tolerance over the chord's length. A circular arc needs 4; the rest is slack for
/// flattening that lands a little inside its tolerance.
const TURN_PER_TOL: f64 = 6.0;

/// `t` turned by the smallest angle that brings it within `limit` radians of each `dir`. When no
/// angle satisfies every limit, `t` turns to the middle of the gap between them.
fn held_within(t: Vec2, limits: [Option<(Vec2, f64)>; 2]) -> Vec2 {
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    for (d, limit) in limits.into_iter().flatten() {
        let to = t.cross(d).atan2(t.dot(d));
        lo = lo.max(to - limit);
        hi = hi.min(to + limit);
    }
    let turn = if lo <= hi { 0f64.max(lo).min(hi) } else { (lo + hi) / 2.0 };
    if turn == 0.0 || !turn.is_finite() {
        return t;
    }
    let (sin, cos) = turn.sin_cos();
    Vec2::new(t.x * cos - t.y * sin, t.x * sin + t.y * cos)
}

/// Left-hand normal in y-down document space (for a path running +x it points +y).
pub fn normal(t: Vec2) -> Vec2 {
    Vec2::new(-t.y, t.x)
}

impl Track {
    /// `raw` pairs each vertex with the tolerance the segment ending at it was flattened with
    /// when it comes from a curve, and `None` when it is a line of the path.
    fn new(raw: Vec<(Point, Option<f64>)>, closed: bool) -> Option<Track> {
        let mut pts: Vec<Point> = Vec::with_capacity(raw.len());
        // `ends_curve[k]`: the flattening tolerance of the segment ending at vertex `k`, when it
        // comes from a curve.
        let mut ends_curve: Vec<Option<f64>> = Vec::with_capacity(raw.len());
        for (p, c) in raw {
            // A duplicate vertex adds no segment, so it is dropped with its tolerance.
            if pts.last().is_some_and(|q| q.distance(p) < 1e-9) {
                continue;
            }
            pts.push(p);
            ends_curve.push(c);
        }
        let mut closing_curve = None;
        if closed && pts.len() > 2 && pts[0].distance(pts[pts.len() - 1]) < 1e-9 {
            pts.pop();
            closing_curve = ends_curve.pop().flatten();
        }
        if pts.len() < 2 {
            return None;
        }
        let closed = closed && pts.len() > 2;
        if closed {
            pts.push(pts[0]);
            ends_curve.push(closing_curve);
        }
        let nseg = pts.len() - 1;
        let curve_tol: Vec<Option<f64>> = (0..nseg).map(|i| ends_curve.get(i + 1).copied().flatten()).collect();
        let curved: Vec<bool> = curve_tol.iter().map(Option::is_some).collect();
        let dirs: Vec<Vec2> = (0..nseg).map(|i| unit(pts[i + 1] - pts[i])).collect();
        let mut cum = vec![0.0];
        for i in 0..nseg {
            cum.push(cum[i] + pts[i].distance(pts[i + 1]));
        }
        let mut tangents = Vec::with_capacity(nseg + 1);
        let mut corner = Vec::with_capacity(nseg + 1);
        let cos_limit = CORNER_DEG.to_radians().cos();
        let seg = |i: usize| (dirs[i], cum[i + 1] - cum[i], curved[i]);
        for i in 0..=nseg {
            let incoming = if i > 0 {
                Some(seg(i - 1))
            } else if closed {
                Some(seg(nseg - 1))
            } else {
                None
            };
            let outgoing = if i < nseg {
                Some(seg(i))
            } else if closed {
                Some(seg(0))
            } else {
                None
            };
            match (incoming, outgoing) {
                (Some((a, la, ca)), Some((b, lb, cb))) => {
                    corner.push(a.dot(b) < cos_limit);
                    tangents.push(match (ca, cb) {
                        // Between two chords of a flattened curve: each direction weighted by the
                        // length of the other chord, which is the tangent of the circle through
                        // the vertex and its two neighbors.
                        (true, true) => unit(a * lb + b * la),
                        // Where a line of the path meets a curve, the line's direction.
                        (false, true) => a,
                        (true, false) => b,
                        (false, false) => unit(a + b),
                    });
                }
                (a, b) => {
                    // An end of an open track (both `None` only for a track without segments).
                    corner.push(false);
                    tangents.push(a.or(b).map_or(Vec2::new(1.0, 0.0), |(d, _, _)| d));
                }
            }
        }
        // An open track's end on a curve continues the bend of its last chord: the tangent there
        // is the neighboring vertex's tangent mirrored about the chord, as on the circle through
        // the last three vertices.
        if !closed && nseg >= 2 {
            for (end, inner, d) in [(0, 1, dirs[0]), (nseg, nseg - 1, dirs[nseg - 1])] {
                let (s_end, s_inner) = if end == 0 { (0, 1) } else { (nseg - 1, nseg - 2) };
                if !corner[inner] && curved[s_end] && curved[s_inner] {
                    let t = tangents[inner];
                    tangents[end] = unit(d * (2.0 * d.dot(t)) - t);
                }
            }
        }
        // A curve lies within its flattening tolerance `tol` of each of its chords, so its tangent
        // at a vertex turns only a little from a chord: at most about `4 * tol / len` on a
        // circular arc. Next to a long chord the estimates above can turn much further (a
        // straight curve element beside a tight bend), and `frame` would then bow off the chord.
        // Between curved chords, each vertex tangent is held within `TURN_PER_TOL * tol / len` of
        // both chords. Where a line meets a curve the tangent stays the line's direction, and a
        // curved chord whose end tangents turn further than that from it keeps to the chord.
        let turn: Vec<f64> = (0..nseg)
            .map(|j| match curve_tol.get(j) {
                Some(Some(tol)) => (TURN_PER_TOL * tol / (cum[j + 1] - cum[j]).max(1e-12)).min(1.0).asin(),
                _ => 0.0,
            })
            .collect();
        for (i, t) in tangents.iter_mut().enumerate() {
            let incoming = if i > 0 { Some(i - 1) } else { closed.then_some(nseg - 1) };
            let outgoing = if i < nseg { Some(i) } else { closed.then_some(0) };
            let sides = [incoming, outgoing];
            if corner[i] || sides.iter().flatten().any(|&j| !curved[j]) {
                continue;
            }
            *t = held_within(*t, sides.map(|j| j.map(|j| (dirs[j], turn[j]))));
        }
        Some(Track { verts: pts, cum, closed, dirs, tangents, corner, curved, turn })
    }

    pub fn len(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() <= 1e-9
    }
    pub fn segments(&self) -> usize {
        self.dirs.len()
    }

    /// Incoming and outgoing unit directions at vertex `i`.
    pub fn in_out(&self, i: usize) -> (Vec2, Vec2) {
        let n = self.segments();
        let inc = if i > 0 {
            self.dirs[i - 1]
        } else if self.closed {
            self.dirs[n - 1]
        } else {
            self.dirs[0]
        };
        let out = if i < n {
            self.dirs[i]
        } else if self.closed {
            self.dirs[0]
        } else {
            self.dirs[n - 1]
        };
        (inc, out)
    }

    /// Arc-length positions of the corner vertices (closed tracks list vertex 0 once).
    pub fn corner_positions(&self) -> Vec<(usize, f64)> {
        let last = if self.closed { self.segments() } else { self.segments() + 1 };
        (0..last).filter(|i| self.corner[*i]).map(|i| (i, self.cum[i])).collect()
    }

    /// Point and unit tangent at arc length `s`. Open tracks extrapolate beyond their ends along
    /// the end tangents; closed tracks wrap.
    pub fn frame(&self, s: f64) -> (Point, Vec2) {
        let l = self.len();
        let n = self.segments();
        let s = if self.closed && l > 0.0 { s.rem_euclid(l) } else { s };
        if s <= 0.0 && !self.closed {
            let t = self.tangents[0];
            return (self.verts[0] + t * s, t);
        }
        if s >= l && !self.closed {
            let t = self.tangents[n];
            return (self.verts[n] + t * (s - l), t);
        }
        // Binary search the segment.
        let i = match self.cum.binary_search_by(|c| c.partial_cmp(&s).unwrap_or(std::cmp::Ordering::Less)) {
            Ok(i) => i.min(n - 1),
            Err(i) => i.saturating_sub(1).min(n - 1),
        };
        let seg = self.cum[i + 1] - self.cum[i];
        let u = if seg > 1e-12 { ((s - self.cum[i]) / seg).clamp(0.0, 1.0) } else { 0.0 };
        self.frame_in(i, u)
    }

    /// Segment `i`'s direction and the tangents [`Self::frame`] uses at its two ends: the vertex
    /// tangents, or the segment's direction at a corner.
    fn end_tangents(&self, i: usize) -> Option<(Vec2, Vec2, Vec2)> {
        let (&d, &c0, &c1, &t0, &t1) =
            (self.dirs.get(i)?, self.corner.get(i)?, self.corner.get(i + 1)?, self.tangents.get(i)?, self.tangents.get(i + 1)?);
        Some((d, if c0 { d } else { t0 }, if c1 { d } else { t1 }))
    }

    /// Point and unit tangent at the fraction `u` (0 to 1) of segment `i`, as [`Self::frame`]
    /// gives them. `i` must be below [`Self::segments`].
    pub fn frame_in(&self, i: usize, u: f64) -> (Point, Vec2) {
        let (Some(&p0), Some(&p1), Some((d, t0, t1))) = (self.verts.get(i), self.verts.get(i + 1), self.end_tangents(i)) else {
            return (self.verts.last().copied().unwrap_or(Point::ZERO), Vec2::new(1.0, 0.0));
        };
        let seg = self.cum.get(i + 1).zip(self.cum.get(i)).map_or(0.0, |(b, a)| b - a);
        if aligned(t0, d) && aligned(t1, d) {
            return (p0.lerp(p1, u), d);
        }
        // The tangents may turn this far from a curved chord, a little beyond it for rounding.
        let within = self.turn.get(i).map_or(-1.0, |a| a.sin() + 1e-9);
        let bends = |t: Vec2| t.dot(d) <= 0.0 || t.cross(d).abs() > within;
        if !self.curved.get(i).copied().unwrap_or(false) || bends(t0) || bends(t1) {
            // A line of the path, or a curved chord at a sharper turn than the curve can make
            // there: the point stays on it, and the tangent turns across it toward the vertex
            // tangents.
            return (p0.lerp(p1, u), unit(t0 + (t1 - t0) * u));
        }
        // On a chord of a flattened curve the frame follows the cubic through both vertices with
        // their tangents, at the chord's length per unit of `u`. Its position and tangent then
        // meet those of the next segment at each vertex.
        let (u2, u3) = (u * u, u * u * u);
        let (m0, m1) = (t0 * seg, t1 * seg);
        let p = p0 + (p1 - p0) * (3.0 * u2 - 2.0 * u3) + m0 * (u3 - 2.0 * u2 + u) + m1 * (u3 - u2);
        let dp = (p1 - p0) * (6.0 * u - 6.0 * u2) + m0 * (3.0 * u2 - 4.0 * u + 1.0) + m1 * (3.0 * u2 - 2.0 * u);
        (p, unit(dp))
    }

    /// The point at arc length `s`, offset `d` along the left normal.
    pub fn map(&self, s: f64, d: f64) -> Point {
        let (p, t) = self.frame(s);
        p + normal(t) * d
    }

    /// Whether [`Self::frame`] holds one constant tangent over arc lengths `s0..s1`, so mapping
    /// through it is an affine of the art and nothing more.
    ///
    /// Read off the track, not sampled. A bend in the span is two segments with different
    /// directions, however short the bent part is, so this cannot miss one; a sampled check can,
    /// because a wiggle narrower than the sample spacing passes between the samples (#3926).
    ///
    /// Fails closed: a span that needs reasoning this does not do (a closed track's seam, a
    /// non-finite bound, a track with no segments) is reported as not straight, and the caller
    /// bends it the long way.
    pub fn is_straight(&self, s0: f64, s1: f64) -> bool {
        // Checked before the min/max, which quietly drop a NaN rather than propagate it.
        let n = self.segments();
        if !(s0.is_finite() && s1.is_finite()) || n == 0 {
            return false;
        }
        let (lo, hi) = (s0.min(s1), s0.max(s1));
        let l = self.len();
        // A closed track wraps, so a span reaching past either end covers segments twice.
        if self.closed && (lo < -1e-9 || hi > l + 1e-9) {
            return false;
        }
        // Past an open track's ends `frame` extrapolates along the end tangent, which is checked
        // against the end segment's direction below, so those segments are part of the span too.
        let ends = [(!self.closed && lo < 0.0).then_some(0), (!self.closed && hi > l).then(|| n - 1)];
        let touched = (0..n).filter(|&i| {
            let (a, b) = (self.cum.get(i), self.cum.get(i + 1));
            matches!((a, b), (Some(a), Some(b)) if *a < hi + 1e-9 && *b > lo - 1e-9)
        });
        let mut dir: Option<Vec2> = None;
        let mut any = false;
        for i in touched.chain(ends.into_iter().flatten()) {
            let Some((d, t0, t1)) = self.end_tangents(i) else {
                return false;
            };
            if dir.is_some_and(|first: Vec2| !aligned(first, d)) {
                return false;
            }
            dir = Some(d);
            any = true;
            // `frame` blends the two end tangents across a line, and follows a curve through them
            // on a curved chord. So a neighbour outside the span can still bend the tangent
            // inside it, and checking the directions alone would miss that: segments d, d, e
            // leave vertex 2's tangent at unit(d + e).
            if !aligned(t0, d) || !aligned(t1, d) {
                return false;
            }
        }
        any
    }
}

/// The most flattened elements of a path kept.
const MAX_FLATTENED: usize = 4_000_000;

/// Flatten `bp` into tracks (one per subpath) with `tol` accuracy.
pub fn tracks(bp: &BezPath, tol: f64) -> Vec<Track> {
    let tol = tol.max(1e-4);
    let mut out = vec![];
    let mut cur: Vec<(Point, Option<f64>)> = vec![];
    let flush = |cur: &mut Vec<(Point, Option<f64>)>, closed: bool, out: &mut Vec<Track>| {
        if let Some(t) = Track::new(std::mem::take(cur), closed) {
            out.push(t);
        }
    };
    // Far outside any drawing a curve flattens to millions of points; past a few million, the
    // rest are not kept.
    let b = bp.control_box();
    let extent = b.x0.abs().max(b.x1.abs()).max(b.y0.abs()).max(b.y1.abs());
    // NaN coordinates must be rejected too, so keep the negation of the `<=` check.
    if extent.is_nan() || extent > 1e9 {
        return out;
    }
    let mut els = vec![];
    // When the element being flattened is a curve, the tolerance `flatten` uses for it: `flatten`
    // pulls each element before it reports that element's lines, and raises the tolerance of a
    // long curve as `vectorcraft_geom::flatten` describes.
    let in_curve = std::cell::Cell::new(None);
    let (last, start) = (std::cell::Cell::new(Point::ZERO), std::cell::Cell::new(Point::ZERO));
    let source = bp.iter().inspect(|el| {
        let from = last.get();
        let (curve, to) = match *el {
            PathEl::MoveTo(p) => {
                start.set(p);
                (None, p)
            }
            PathEl::LineTo(p) => (None, p),
            PathEl::QuadTo(a, p) => (Some(from.distance(a) + a.distance(p)), p),
            PathEl::CurveTo(a, b, p) => (Some(from.distance(a) + a.distance(b) + b.distance(p)), p),
            PathEl::ClosePath => (None, start.get()),
        };
        last.set(to);
        let per = vectorcraft_geom::MAX_FLATTEN_PER_CURVE * vectorcraft_geom::MAX_FLATTEN_PER_CURVE;
        in_curve.set(curve.map(|length| if length.is_finite() { tol.max(length / per) } else { f64::INFINITY }));
    });
    // Guarded: kurbo works out every subdivision of a curve even when the callback drops them.
    vectorcraft_geom::flatten(source, tol, |el| {
        if els.len() < MAX_FLATTENED {
            els.push((el, in_curve.get()));
        }
    });
    for (el, from_curve) in els {
        match el {
            PathEl::MoveTo(p) => {
                flush(&mut cur, false, &mut out);
                cur.push((p, None));
            }
            PathEl::LineTo(p) => cur.push((p, from_curve)),
            PathEl::ClosePath => flush(&mut cur, true, &mut out),
            _ => {}
        }
    }
    flush(&mut cur, false, &mut out);
    out
}

/// Deterministic pseudo-random numbers (SplitMix64), seeded from the path so a brushed stroke
/// looks the same on every frame and in exports.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in [a, b] (a == b returns a without consuming randomness differently).
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        let u = self.unit();
        if (b - a).abs() < 1e-12 { a } else { a + (b - a) * u }
    }
}

/// A stable seed for a path (FNV-1a over its coordinates' bits, quantised to 1/64 pt).
pub fn seed_of(bp: &BezPath, salt: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for b in salt.bytes() {
        eat(b as u64);
    }
    for el in bp.elements() {
        let pts: Vec<Point> = match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
            PathEl::QuadTo(a, b) => vec![a, b],
            PathEl::CurveTo(a, b, c) => vec![a, b, c],
            PathEl::ClosePath => vec![],
        };
        for p in pts {
            eat((p.x * 64.0).round() as i64 as u64);
            eat((p.y * 64.0).round() as i64 as u64);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight run is straight; a bend in the span is not, however short it is. This is the
    /// property a sampled check cannot offer, so it is worth pinning on its own (#3926 review).
    #[test]
    fn straightness_is_read_off_the_track_not_sampled() {
        let line = |pts: &[(f64, f64)], closed: bool| {
            let mut bp = BezPath::new();
            bp.move_to(pts[0]);
            for p in &pts[1..] {
                bp.line_to(*p);
            }
            if closed {
                bp.close_path();
            }
            tracks(&bp, 0.1).remove(0)
        };

        let straight = line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)], false);
        assert!(straight.is_straight(0.0, 200.0), "two collinear segments are one straight run");
        // Open tracks extrapolate along the end directions, which is still the same line.
        assert!(straight.is_straight(-50.0, 250.0), "and so is the extrapolation past both ends");

        // A detour of a fifth of a point, 0.1% of the run: far below any sampling grid this code
        // would choose, and still a bend.
        let wiggle = line(&[(0.0, 0.0), (99.9, 0.0), (100.0, 0.2), (100.1, 0.0), (200.0, 0.0)], false);
        assert!(!wiggle.is_straight(0.0, 200.0), "a 0.2 pt detour is a bend");
        assert!(!wiggle.is_straight(99.0, 101.0), "and is caught when the span is only around it");
        assert!(wiggle.is_straight(0.0, 50.0), "while a span before it is straight");

        // `frame` blends vertex tangents, so the segment before a bend is not straight all the way
        // to the vertex: the tangent there is already turning. Pinning this because checking the
        // segment directions alone would call that span straight and be wrong.
        let kink = line(&[(0.0, 0.0), (100.0, 0.0), (160.0, 60.0)], false);
        assert!(!kink.is_straight(0.0, 100.0), "the last segment's tangent is already turning at its far vertex");
        assert!(kink.is_straight(0.0, 50.0), "a span clear of the turning vertex is straight");

        // Fails closed where it does not reason: a closed track's seam, and nonsense bounds.
        let ring = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
        assert!(!ring.is_straight(-10.0, 10.0), "a span across a closed track's seam is refused");
        assert!(ring.is_straight(10.0, 90.0), "a span inside one of its sides is straight");
        assert!(!straight.is_straight(f64::NAN, 10.0), "a non-finite bound is refused");
    }
    #[test]
    fn straight_track_frames() {
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        bp.line_to((100.0, 0.0));
        let t = &tracks(&bp, 0.1)[0];
        assert_eq!(t.len(), 100.0);
        let (p, d) = t.frame(25.0);
        assert_eq!(p, Point::new(25.0, 0.0));
        assert_eq!(d, Vec2::new(1.0, 0.0));
        assert_eq!(t.map(50.0, 5.0), Point::new(50.0, 5.0));
        assert_eq!(t.map(110.0, 0.0), Point::new(110.0, 0.0));
    }

    /// Between two lines that meet at a gentle turn, the frame stays on the lines: only the chords
    /// of a flattened curve are smoothed. Where a line meets a curve at a gentle turn, the frame
    /// stays on the line with the line's direction, and its tangent does not jump at the join.
    #[test]
    fn a_polylines_frame_stays_on_its_lines() {
        // A line, then a line or a nearly straight curve at a 20° turn, below the corner angle, so
        // the vertex is not a corner.
        let turn = 20f64.to_radians();
        let (c, s) = (100.0 * turn.cos(), 100.0 * turn.sin());
        for curve in [false, true] {
            let mut bp = BezPath::new();
            bp.move_to((0.0, 0.0));
            bp.line_to((100.0, 0.0));
            if curve {
                bp.curve_to((100.0 + c / 3.0, s / 3.0 + 0.5), (100.0 + 2.0 * c / 3.0, 2.0 * s / 3.0 + 0.5), (100.0 + c, s));
            } else {
                bp.line_to((100.0 + c, s));
            }
            let t = &tracks(&bp, 0.05)[0];
            for at in [10.0, 30.0, 50.0, 70.0, 90.0] {
                let (p, tg) = t.frame(at);
                assert!(p.y.abs() < 1e-9 && (p.x - at).abs() < 1e-9, "curve {curve}: at {at} the frame left the first line: {p:?}");
                if curve {
                    assert!(tg.y.abs() < 1e-12, "at {at} the line's tangent turned toward the curve: {tg:?}");
                }
            }
            if curve {
                // The curve's first chord keeps to the chord rather than bending toward the line.
                let (a, b) = (t.cum[1], t.cum[2]);
                let (p, q) = (t.verts[1], t.verts[2]);
                let mid = t.frame((a + b) / 2.0).0;
                assert!((mid - p.midpoint(q)).hypot() < 1e-9, "the first curve chord bows: {mid:?}");
            }
            let (before, after) = (t.frame(100.0 - 1e-7).1, t.frame(100.0 + 1e-7).1);
            assert!(before.dot(after) > 1.0 - 1e-9, "curve {curve}: the tangent jumps at the join: {before:?} {after:?}");
        }
    }

    /// A curve of no length between two lines leaves them lines.
    #[test]
    fn a_curve_of_no_length_leaves_a_line_a_line() {
        let turn = 20f64.to_radians();
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        bp.line_to((100.0, 0.0));
        bp.curve_to((100.0, 0.0), (100.0, 0.0), (100.0, 0.0));
        bp.line_to((100.0 + 100.0 * turn.cos(), 100.0 * turn.sin()));
        let t = &tracks(&bp, 0.05)[0];
        assert!(t.curved.iter().all(|c| !c), "no segment is a curve: {:?}", t.curved);
        for at in [10.0, 50.0, 90.0] {
            let (p, _) = t.frame(at);
            assert!(p.y.abs() < 1e-9 && (p.x - at).abs() < 1e-9, "at {at} the frame left the first line: {p:?}");
        }
    }

    /// Where a line joins a curve smoothly, the line keeps its direction up to the join.
    #[test]
    fn a_line_into_a_smooth_curve_keeps_its_direction() {
        // A line along y = 0 into a quarter circle of radius 50 that starts along +x.
        let r = 50.0;
        let mut bp = BezPath::new();
        bp.move_to((-100.0, 0.0));
        bp.line_to((0.0, 0.0));
        bp.curve_to((r * K, 0.0), (r, r - r * K), (r, r));
        let t = &tracks(&bp, 0.05)[0];
        for s in [10.0, 50.0, 90.0, 99.0] {
            let (p, tg) = t.frame(s);
            assert!(p.y.abs() < 1e-9 && tg.y.abs() < 1e-9, "at {s} the line's frame turned: {p:?} {tg:?}");
        }
    }

    /// The handle length of a cubic quarter circle.
    const K: f64 = 0.552_284_749_8;

    /// A curve element whose handles lie on its chord is a line drawn as a curve. Next to a tight
    /// bend, at either end of a track, the frame along it stays within the flattening tolerance
    /// of the line.
    #[test]
    fn a_straight_curve_element_beside_a_tight_bend_stays_on_its_line() {
        let tol = 0.05;
        for bend_first in [false, true] {
            // A 200 pt line along y = 0 drawn as a cubic, joined smoothly to a quarter circle of
            // radius 5 that turns toward +y.
            let mut bp = BezPath::new();
            if bend_first {
                bp.move_to((-5.0, -5.0));
                bp.curve_to((-5.0, -5.0 + 5.0 * K), (-5.0 * K, 0.0), (0.0, 0.0));
                bp.curve_to((50.0, 0.0), (150.0, 0.0), (200.0, 0.0));
            } else {
                bp.move_to((0.0, 0.0));
                bp.curve_to((50.0, 0.0), (150.0, 0.0), (200.0, 0.0));
                bp.curve_to((200.0 + 5.0 * K, 0.0), (205.0, 5.0 - 5.0 * K), (205.0, 5.0));
            }
            let t = &tracks(&bp, tol)[0];
            // The bend lies outside 0 <= x <= 200, so every frame point in that range is on the
            // line's stretch of the track.
            let on_line: Vec<(Point, Vec2)> =
                (0..=2000).map(|k| t.frame(t.len() * k as f64 / 2000.0)).filter(|(p, _)| (0.0..=200.0).contains(&p.x)).collect();
            let worst = on_line.iter().map(|(p, _)| p.y.abs()).fold(0.0, f64::max);
            assert!(worst <= 2.0 * tol, "bend first {bend_first}: the frame strays {worst:.4} pt off the line");
            // Along the line the tangent turns by no more than the line's flattening allows.
            let lean = on_line.iter().map(|(_, tg)| tg.y.abs()).fold(0.0, f64::max);
            assert!(lean <= 0.01, "bend first {bend_first}: the tangent turns {lean:.4} off the line");
        }
    }

    /// A curve long enough that flattening raises its tolerance still has its frame follow the
    /// curve between the vertices, not the chords.
    #[test]
    fn a_long_curve_flattened_coarser_is_still_followed() {
        // A line along y = 0 into a cubic quarter circle of radius 5,000,000 that starts along +x.
        let r = 5.0e6;
        let curve = kurbo::CubicBez::new((0.0, 0.0), (r * K, 0.0), (r, r - r * K), (r, r));
        let mut bp = BezPath::new();
        bp.move_to((-100.0, 0.0));
        bp.line_to((0.0, 0.0));
        bp.curve_to(curve.p1, curve.p2, curve.p3);
        let t = &tracks(&bp, 0.05)[0];
        let worst = (1..400)
            .map(|k| t.frame(100.0 + (t.len() - 100.0) * k as f64 / 400.0).0)
            .map(|p| kurbo::ParamCurveNearest::nearest(&curve, p, 1e-9).distance_sq.sqrt())
            .fold(0.0, f64::max);
        assert!(worst < 0.05, "the frame strays {worst:.4} pt from the curve");
    }

    /// A closed curve is smooth across its seam: the frame stays on the circle and at right angles
    /// to its radius all the way round, the start included.
    #[test]
    fn a_closed_curves_frame_is_smooth_across_its_seam() {
        let (r, tol) = (50.0, 0.05);
        let mut bp = BezPath::new();
        bp.move_to((r, 0.0));
        bp.curve_to((r, r * K), (r * K, r), (0.0, r));
        bp.curve_to((-r * K, r), (-r, r * K), (-r, 0.0));
        bp.curve_to((-r, -r * K), (-r * K, -r), (0.0, -r));
        bp.curve_to((r * K, -r), (r, -r * K), (r, 0.0));
        bp.close_path();
        let t = &tracks(&bp, tol)[0];
        assert!(t.closed && t.curved.iter().all(|c| *c), "every chord of the circle is a chord of a curve");
        for k in -20..=20 {
            let (p, tg) = t.frame(k as f64 * 0.37);
            let radial = p.to_vec2() / p.to_vec2().hypot();
            assert!((p.to_vec2().hypot() - r).abs() <= tol, "at {k} the frame is off the circle: {p:?}");
            assert!(tg.dot(radial).abs() < 0.5f64.to_radians().sin(), "at {k} the tangent leans off the circle: {tg:?}");
        }
    }

    /// A closed path whose last side is the implicit line back to the start keeps that line
    /// straight.
    #[test]
    fn an_implicit_closing_line_is_not_a_curve() {
        let r = 50.0;
        let mut bp = BezPath::new();
        bp.move_to((r, 0.0));
        bp.curve_to((r, r * K), (r * K, r), (0.0, r));
        bp.curve_to((-r * K, r), (-r, r * K), (-r, 0.0));
        bp.close_path();
        let t = &tracks(&bp, 0.05)[0];
        assert!(t.closed);
        assert_eq!(t.curved.last(), Some(&false), "the closing line is a line");
        let n = t.segments();
        for u in [0.1, 0.5, 0.9] {
            let (p, _) = t.frame(t.cum[n - 1] + (t.cum[n] - t.cum[n - 1]) * u);
            assert!(p.y.abs() < 1e-9, "the frame leaves the closing line: {p:?}");
        }
    }

    #[test]
    fn closed_square_has_four_corners() {
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        bp.line_to((10.0, 0.0));
        bp.line_to((10.0, 10.0));
        bp.line_to((0.0, 10.0));
        bp.close_path();
        let t = &tracks(&bp, 0.1)[0];
        assert!(t.closed);
        assert_eq!(t.len(), 40.0);
        assert_eq!(t.corner_positions().len(), 4);
        assert_eq!(t.frame(45.0).0, Point::new(5.0, 0.0));
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let v = Rng::new(1).range(2.0, 3.0);
        assert!((2.0..=3.0).contains(&v));
    }
}
