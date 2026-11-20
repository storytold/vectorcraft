//! The circles the Tangent Circle tool draws on two picked segments: the circle tangent to the
//! first segment at a point that also touches the second ([`circles_through`]), or the circles of a
//! radius tangent to the first segment that also touch the second ([`circles_of_radius`]). "The
//! second" is a list of segments (the picked one and those next to it). A circle touches them when
//! the nearest of their points to its center is on the circle, so they lie outside it, and it never
//! crosses the first segment.
//!
//! The roots are found by sampling a distance function along one parameter: samples where it is 0,
//! bisection where it changes sign, and a golden-section search where it comes down to 0 between
//! samples without changing sign.

use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest, Point, Vec2};

use super::Circle;

/// A circle on two picked segments: the circle, where it touches the first segment and where it
/// touches the second ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentCircle {
    pub circle: Circle,
    pub first: Point,
    pub second: Point,
}

/// Samples along the parameter the roots are looked for on.
const SAMPLES: usize = 512;
/// Bisection steps per root.
const BISECTIONS: usize = 64;
/// The largest radius [`circles_through`] looks at, as a multiple of the diagonal of the box around
/// the segments' control points.
const MAX_RADIUS: f64 = 1e4;
/// Below this share of the radius two points of a circle are one.
const SAME: f64 = 1e-7;
/// Below this share of the radius (or, while the radius is looked for, of the gap from the point to
/// the second segments) a circle's distance from the second segments counts as touching.
const CONTACT: f64 = 1e-9;
/// Below this (in units of the first segment's size) a length is rounding.
const ROUNDING: f64 = 1e-12;

/// The diagonal of the box around the control points of `first` and `second`; none when it is 0 or
/// not finite.
fn scale(first: &CubicBez, second: &[CubicBez]) -> Option<f64> {
    let pts = std::iter::once(first).chain(second).flat_map(|c| [c.p0, c.p1, c.p2, c.p3]);
    let (mut lo, mut hi) = (Point::new(f64::INFINITY, f64::INFINITY), Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    for p in pts {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        lo = Point::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Point::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let d = (hi - lo).hypot();
    (d.is_finite() && d > 0.0).then_some(d)
}

/// The nearest point to `q` on `segs` and its distance.
fn nearest(q: Point, segs: &[CubicBez]) -> Option<(f64, Point)> {
    segs.iter()
        .map(|c| {
            let n = c.nearest(q, 1e-9);
            (n.distance_sq.sqrt(), c.eval(n.t))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The unit normal of `c` at `t` (its tangent turned a quarter turn). Where the derivative is 0 (a
/// handle lying on its anchor), the direction the curve leaves in: toward the far handle, or the
/// other end. None where the segment does not move by more than rounding in units of `scale`, its
/// size.
fn normal(c: &CubicBez, t: f64, scale: f64) -> Option<Vec2> {
    let tiny = ROUNDING * scale;
    let d = c.deriv().eval(t).to_vec2();
    let d = if d.hypot() > tiny {
        d
    } else if t < 0.5 {
        c.p2 - c.p0
    } else {
        c.p3 - c.p1
    };
    let d = if d.hypot() > tiny { d } else { c.p3 - c.p0 };
    let len = d.hypot();
    (len.is_finite() && len > tiny).then(|| Vec2::new(-d.y / len, d.x / len))
}

/// The parameters where `f` is 0, within `zero`, in order: every sample where it is (a stretch where
/// it stays 0 gives each of its samples), each place between two samples where it changes sign
/// (found by bisection), and, where `f` comes toward 0 between samples and goes back without a
/// sample on the other side, the place it comes nearest (a golden-section search): a touch when it
/// reaches 0 there, and the two places on either side, by bisection, when it passes 0.
fn roots(samples: &[f64], f: impl Fn(f64) -> Option<f64>, zero: f64) -> Vec<f64> {
    let mut out = vec![];
    let values: Vec<Option<f64>> = samples.iter().map(|s| f(*s)).collect();
    let is_zero = |v: f64| v.abs() <= zero;
    for (s, v) in samples.iter().zip(&values) {
        if v.is_some_and(is_zero) {
            out.push(*s);
        }
    }
    for (k, w) in values.windows(3).enumerate() {
        let (Some(a), Some(b), Some(c)) = (w[0], w[1], w[2]) else { continue };
        let (Some(&lo), Some(&hi)) = (samples.get(k), samples.get(k + 2)) else { continue };
        let same_side = (a > 0.0) == (b > 0.0) && (b > 0.0) == (c > 0.0);
        // A low point, or one of two equal low samples about a touch half way between them.
        let low = b.abs() <= a.abs() && b.abs() <= c.abs() && (b.abs() < a.abs() || b.abs() < c.abs());
        if !same_side || is_zero(a) || is_zero(b) || is_zero(c) || !low {
            continue;
        }
        out.extend(near_zero(lo, hi, b > 0.0, &f, zero));
    }
    // At either end the lowest sample has no sample beyond it: the stretch to the next one.
    for (end, next) in [(0, 1), (values.len().saturating_sub(1), values.len().saturating_sub(2))] {
        let (Some(Some(a)), Some(Some(b))) = (values.get(end), values.get(next)) else { continue };
        let (Some(&s), Some(&t)) = (samples.get(end), samples.get(next)) else { continue };
        if (a > &0.0) == (b > &0.0) && !is_zero(*a) && !is_zero(*b) && a.abs() < b.abs() {
            out.extend(near_zero(s.min(t), s.max(t), *a > 0.0, &f, zero));
        }
    }
    for (k, w) in values.windows(2).enumerate() {
        let (Some(a), Some(b)) = (w[0], w[1]) else { continue };
        let (Some(&lo), Some(&hi)) = (samples.get(k), samples.get(k + 1)) else { continue };
        if (a > 0.0) == (b > 0.0) || is_zero(a) || is_zero(b) {
            continue;
        }
        out.extend(bisect(lo, hi, &f));
    }
    out.sort_by(f64::total_cmp);
    out
}

/// Where `f`, above 0 (`positive`) or below at the samples `lo` and `hi`, comes nearest 0 between
/// them: that place when it reaches 0 there, the two places on either side where it passes 0, or
/// none.
fn near_zero(lo: f64, hi: f64, positive: bool, f: &impl Fn(f64) -> Option<f64>, zero: f64) -> Vec<f64> {
    // Toward 0 is down for a positive `f`, up for a negative one.
    let toward = if positive { 1.0 } else { -1.0 };
    let Some(m) = smallest(lo, hi, |s| f(s).map(|v| v * toward)) else { return vec![] };
    let Some(fm) = f(m) else { return vec![] };
    if fm.abs() <= zero {
        vec![m]
    } else if fm * toward < 0.0 {
        bisect(lo, m, f).into_iter().chain(bisect(m, hi, f)).collect()
    } else {
        vec![]
    }
}

/// Where `f` changes sign between `lo` and `hi`, by bisection; none when it does not.
fn bisect(mut lo: f64, mut hi: f64, f: impl Fn(f64) -> Option<f64>) -> Option<f64> {
    let (a, b) = (f(lo)?, f(hi)?);
    if (a > 0.0) == (b > 0.0) {
        return None;
    }
    let positive_low = a > 0.0;
    for _ in 0..BISECTIONS {
        let mid = 0.5 * (lo + hi);
        match f(mid) {
            Some(v) if (v > 0.0) == positive_low => lo = mid,
            Some(_) => hi = mid,
            None => break,
        }
    }
    Some(0.5 * (lo + hi))
}

/// Where `g` is smallest between `lo` and `hi`, by a golden-section search; none where `g` has no
/// value.
fn smallest(mut lo: f64, mut hi: f64, g: impl Fn(f64) -> Option<f64>) -> Option<f64> {
    const INV_PHI: f64 = 0.618_033_988_749_894_9;
    let (mut a, mut b) = (hi - INV_PHI * (hi - lo), lo + INV_PHI * (hi - lo));
    let (mut ga, mut gb) = (g(a)?, g(b)?);
    for _ in 0..BISECTIONS {
        if ga < gb {
            hi = b;
            (b, gb) = (a, ga);
            a = hi - INV_PHI * (hi - lo);
            ga = g(a)?;
        } else {
            lo = a;
            (a, ga) = (b, gb);
            b = lo + INV_PHI * (hi - lo);
            gb = g(b)?;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Where the circle about `center` of radius `r` touches `second`: the nearest of their points, when
/// it is on the circle (within a millionth of the radius, plus rounding in units of `scale`). None
/// for a root where the distance only jumps past the radius, as it does where the first segment
/// turns back on itself and its normal flips.
fn touching(center: Point, r: f64, second: &[CubicBez], scale: f64) -> Option<Point> {
    nearest(center, second).filter(|(d, _)| (d - r).abs() <= 1e-6 * r + ROUNDING * scale).map(|(_, p)| p)
}

/// `found` without circles that cross `first`, that touch the two at one point, or that repeat an
/// earlier one (the same center, touching `first` at the same point).
fn keep(found: Vec<SegmentCircle>, first: &CubicBez) -> Vec<SegmentCircle> {
    let own = scale(first, &[]).unwrap_or(0.0);
    let mut out: Vec<SegmentCircle> = vec![];
    for c in found {
        let r = c.circle.radius;
        let crosses = nearest(c.circle.center, std::slice::from_ref(first)).is_none_or(|(d, _)| d < r * (1.0 - 1e-6));
        let one_point = (c.first - c.second).hypot() <= SAME * r;
        // The roots come in order along their parameter, so a repeat follows its first. Circles of
        // one large radius at places tried close together along `first` are not repeats.
        let repeat =
            out.last().is_some_and(|o| (o.circle.center - c.circle.center).hypot() <= SAME * r * 10.0 && (o.first - c.first).hypot() <= SAME * own);
        if !(crosses || one_point || repeat) && r.is_finite() && r > 0.0 {
            out.push(c);
        }
    }
    out
}

/// The circles tangent to `first` at parameter `t` (passing through that point, their centers on
/// its normal there) that touch `second`. An empty list when `first` has no direction at `t`, when
/// the point is on `second`, or when no circle touches it; at most two in practice, one on each side.
pub fn circles_through(first: CubicBez, t: f64, second: &[CubicBez]) -> Vec<SegmentCircle> {
    if second.is_empty() || !t.is_finite() {
        return vec![];
    }
    let t = t.clamp(0.0, 1.0);
    let (Some(scale), Some(own)) = (scale(&first, second), scale(&first, &[])) else { return vec![] };
    let Some(n) = normal(&first, t, own) else { return vec![] };
    let p = first.eval(t);
    // On the second segments (a corner the two share), up to rounding in units of the first
    // segment's size: no circle.
    let on = ROUNDING * own;
    let Some((gap, _)) = nearest(p, second).filter(|(d, _)| *d > on) else { return vec![] };
    // Radii spaced evenly in their logarithm, from half the gap from the point to the second
    // segments (no smaller circle through the point reaches them) to far past the segments.
    let (lo, hi) = (0.5 * gap * (1.0 - 1e-9), MAX_RADIUS * scale.max(gap));
    let samples: Vec<f64> = (0..=SAMPLES).map(|k| lo * (hi / lo).powf(k as f64 / SAMPLES as f64)).collect();
    let mut found = vec![];
    for side in [1.0, -1.0] {
        let center = |r: f64| p + n * (side * r);
        for r in roots(&samples, |r| nearest(center(r), second).map(|(d, _)| d - r), CONTACT * gap) {
            if let Some(touch) = touching(center(r), r, second, own) {
                found.push(SegmentCircle { circle: Circle { center: center(r), radius: r }, first: p, second: touch });
            }
        }
    }
    keep(found, &first)
}

/// The circles of `radius` tangent to `first` (their centers `radius` from it along its normal)
/// that touch `second`. An empty list when the radius is not above 0 or none is found. Where the
/// segments run twice the radius apart along a stretch, every circle along it touches both, and one
/// is listed for each of the 513 places along `first` that are tried there.
pub fn circles_of_radius(first: CubicBez, second: &[CubicBez], radius: f64) -> Vec<SegmentCircle> {
    if second.is_empty() || !radius.is_finite() || radius <= 0.0 {
        return vec![];
    }
    let (Some(_), Some(own)) = (scale(&first, second), scale(&first, &[])) else { return vec![] };
    let samples: Vec<f64> = (0..=SAMPLES).map(|k| k as f64 / SAMPLES as f64).collect();
    let mut found = vec![];
    for side in [1.0, -1.0] {
        let center = |t: f64| normal(&first, t, own).map(|n| first.eval(t) + n * (side * radius));
        for t in roots(&samples, |t| center(t).and_then(|c| nearest(c, second)).map(|(d, _)| d - radius), CONTACT * radius) {
            if let Some(c) = center(t)
                && let Some(touch) = touching(c, radius, second, own)
            {
                found.push(SegmentCircle { circle: Circle { center: c, radius }, first: first.eval(t), second: touch });
            }
        }
    }
    keep(found, &first)
}
