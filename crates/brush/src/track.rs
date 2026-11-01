//! Arc-length parameterised polylines ("tracks") that brushes lay art along.
//!
//! A track is one flattened subpath. `frame(s)` gives the point and unit tangent at arc length
//! `s`; tangents are blended across the gentle vertices of flattened curves (so warped art bends
//! smoothly) but stay discontinuous at real corners (so art folds there, like Illustrator).

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

/// Left-hand normal in y-down document space (for a path running +x it points +y).
pub fn normal(t: Vec2) -> Vec2 {
    Vec2::new(-t.y, t.x)
}

impl Track {
    fn new(mut pts: Vec<Point>, closed: bool) -> Option<Track> {
        pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
        if closed && pts.len() > 2 && pts[0].distance(pts[pts.len() - 1]) < 1e-9 {
            pts.pop();
        }
        if pts.len() < 2 {
            return None;
        }
        let closed = closed && pts.len() > 2;
        if closed {
            pts.push(pts[0]);
        }
        let nseg = pts.len() - 1;
        let dirs: Vec<Vec2> = (0..nseg).map(|i| unit(pts[i + 1] - pts[i])).collect();
        let mut cum = vec![0.0];
        for i in 0..nseg {
            cum.push(cum[i] + pts[i].distance(pts[i + 1]));
        }
        let mut tangents = Vec::with_capacity(nseg + 1);
        let mut corner = Vec::with_capacity(nseg + 1);
        let cos_limit = CORNER_DEG.to_radians().cos();
        for i in 0..=nseg {
            let incoming = if i > 0 {
                Some(dirs[i - 1])
            } else if closed {
                Some(dirs[nseg - 1])
            } else {
                None
            };
            let outgoing = if i < nseg {
                Some(dirs[i])
            } else if closed {
                Some(dirs[0])
            } else {
                None
            };
            match (incoming, outgoing) {
                (Some(a), Some(b)) => {
                    corner.push(a.dot(b) < cos_limit);
                    tangents.push(unit(a + b));
                }
                (a, b) => {
                    // An end of an open track (both `None` only for a track without segments).
                    corner.push(false);
                    tangents.push(a.or(b).unwrap_or(Vec2::new(1.0, 0.0)));
                }
            }
        }
        Some(Track { verts: pts, cum, closed, dirs, tangents, corner })
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
            let t = self.dirs[0];
            return (self.verts[0] + t * s, t);
        }
        if s >= l && !self.closed {
            let t = self.dirs[n - 1];
            return (self.verts[n] + t * (s - l), t);
        }
        // Binary search the segment.
        let i = match self.cum.binary_search_by(|c| c.partial_cmp(&s).unwrap_or(std::cmp::Ordering::Less)) {
            Ok(i) => i.min(n - 1),
            Err(i) => i.saturating_sub(1).min(n - 1),
        };
        let seg = self.cum[i + 1] - self.cum[i];
        let u = if seg > 1e-12 { ((s - self.cum[i]) / seg).clamp(0.0, 1.0) } else { 0.0 };
        let p = self.verts[i].lerp(self.verts[i + 1], u);
        let t0 = if self.corner[i] { self.dirs[i] } else { self.tangents[i] };
        let t1 = if self.corner[i + 1] { self.dirs[i] } else { self.tangents[i + 1] };
        (p, unit(t0 + (t1 - t0) * u))
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
        // Past an open track's ends `frame` extrapolates along the end segment's direction, so
        // those segments are part of the span too.
        let ends = [(!self.closed && lo < 0.0).then_some(0), (!self.closed && hi > l).then(|| n - 1)];
        let touched = (0..n).filter(|&i| {
            let (a, b) = (self.cum.get(i), self.cum.get(i + 1));
            matches!((a, b), (Some(a), Some(b)) if *a < hi + 1e-9 && *b > lo - 1e-9)
        });
        let mut dir: Option<Vec2> = None;
        let mut any = false;
        for i in touched.chain(ends.into_iter().flatten()) {
            let (Some(&d), Some(&t0), Some(&t1), Some(&c0), Some(&c1)) =
                (self.dirs.get(i), self.tangents.get(i), self.tangents.get(i + 1), self.corner.get(i), self.corner.get(i + 1))
            else {
                return false;
            };
            if dir.is_some_and(|first: Vec2| !aligned(first, d)) {
                return false;
            }
            dir = Some(d);
            any = true;
            // `frame` blends the two vertex tangents across the segment unless the vertex is a
            // corner, where it uses the segment's own direction. So a neighbour outside the span
            // can still bend the tangent inside it, and checking the directions alone would miss
            // that: segments d, d, e leave vertex 2's tangent at unit(d + e).
            if !(c0 || aligned(t0, d)) || !(c1 || aligned(t1, d)) {
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
    let mut out = vec![];
    let mut cur: Vec<Point> = vec![];
    let flush = |cur: &mut Vec<Point>, closed: bool, out: &mut Vec<Track>| {
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
    kurbo::flatten(bp.iter(), tol.max(1e-4), |el| {
        if els.len() < MAX_FLATTENED {
            els.push(el);
        }
    });
    for el in els {
        match el {
            PathEl::MoveTo(p) => {
                flush(&mut cur, false, &mut out);
                cur.push(p);
            }
            PathEl::LineTo(p) => cur.push(p),
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
