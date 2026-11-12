//! A repeating or reflecting gradient as one that pads, for formats whose gradients only pad
//! past their ends: the stops repeated (every other run mirrored, for Reflect) as many times as
//! the painted area needs, and the vector stretched over those runs.

use kurbo::{Point, Rect, Vec2};

use super::{Gradient, GradientGeom, GradientKind, GradientSpread, GradientStop};

/// Most runs of the stops [`Gradient::padded_over`] writes.
pub const MAX_SPREAD_RUNS: usize = 256;

/// Most stops [`Gradient::padded_over`] writes, all runs together.
pub const MAX_SPREAD_STOPS: usize = 16384;

/// The farthest level from its center a radial or diamond gradient's runs are written out to by
/// [`Gradient::padded_over`] (its stops' offsets, single-precision fractions of it, stay apart),
/// nearer for stops closer together than a run ([`MIN_OFFSET_GAP`]).
pub const MAX_SPREAD_LEVEL: f64 = 65536.0;

/// The least difference between two stops' offsets [`Gradient::padded_over`] writes as
/// different: sixteen times single precision's step just below 1.
const MIN_OFFSET_GAP: f64 = 1e-6;

/// A repeating or reflecting gradient written as one that pads ([`Gradient::padded_over`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Padded {
    /// The gradient with its stops repeated, padding past its ends.
    pub gradient: Gradient,
    /// Where those runs lie.
    pub geom: GradientGeom,
    /// Where along `geom` (0 to 1) the first run written starts: 0 for a linear gradient, the
    /// first run's level over the last one's outer level for a radial or diamond one.
    pub first: f64,
    /// How many runs are written, evenly spaced from [`Self::first`] to 1.
    pub runs: usize,
    /// The area needed more runs than [`MAX_SPREAD_RUNS`] (or than [`MAX_SPREAD_STOPS`] allows),
    /// or runs past [`MAX_SPREAD_LEVEL`]: the gradient pads past the runs written.
    pub capped: bool,
}

impl Gradient {
    /// A linear, radial or diamond gradient that repeats or reflects, placed at `geom`, as a
    /// gradient that pads and paints `area` the same way: its stops repeated, each run's
    /// placement [`GradientGeom::period`] of `geom`, as many runs as the area reaches (a radial or
    /// diamond from its first level, 0, the first stop's color inside the area's innermost run).
    /// `None` for a gradient that pads, a conic or
    /// freeform one, one without stops, and a placement whose levels at the corners are not
    /// finite.
    pub fn padded_over(&self, geom: &GradientGeom, area: Rect) -> Option<Padded> {
        self.padded_within(geom, area, MAX_SPREAD_STOPS)
    }

    /// [`Self::padded_over`] with at most `max_stops` stops (and at least one run).
    pub fn padded_within(&self, geom: &GradientGeom, area: Rect, max_stops: usize) -> Option<Padded> {
        let kind = self.kind;
        if !self.is_spread() || matches!(kind, GradientKind::Conic | GradientKind::Freeform) || self.stops.is_empty() {
            return None;
        }
        let period = geom.period(kind, self.repeat_count());
        let corners = [Point::new(area.x0, area.y0), Point::new(area.x1, area.y0), Point::new(area.x1, area.y1), Point::new(area.x0, area.y1)];
        let ts = corners.map(|p| period.param_at(kind, p));
        if ts.iter().any(|t| !t.is_finite()) {
            return None;
        }
        let centered = kind.is_centered();
        // The level's range over the area. A linear gradient's extremes lie at corners. Radial and
        // diamond levels are nested convex rings from level 0: the largest at a corner, the
        // smallest 0 when the area holds the rings' center, else on its edge.
        let lo = if centered { inner_level(&period, kind, area).floor() } else { ts.iter().copied().fold(f64::INFINITY, f64::min).floor() };
        let hi = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil().max(lo + 1.0);
        let one = whole_run(&self.stops);
        let most = MAX_SPREAD_RUNS.min(max_stops.min(MAX_SPREAD_STOPS) / one.len().max(1)).max(1);
        let capped = hi - lo > most as f64;
        // Capped, the runs kept are the area's innermost ones (radial and diamond), or (linear)
        // those around the vector's start where the area reaches it, else the area's runs nearest
        // to it.
        let lo = match capped {
            true if !centered => (-((most / 2) as f64)).clamp(lo, hi - most as f64),
            _ => lo,
        };
        let n = (hi - lo).min(most as f64).max(1.0);
        // A radial or diamond writes its runs among the levels from 0, which its stops' offsets
        // (single-precision fractions of the outermost level) tell apart only so far out, the
        // nearer the closer its stops lie: past that, the runs end there and the gradient pads.
        let gap = one.windows(2).map(|w| f64::from(w[1].offset - w[0].offset)).filter(|d| *d > 0.0).fold(1.0, f64::min);
        // A whole level, so the runs keep their places (and Reflect its mirrored ones).
        let farthest = MAX_SPREAD_LEVEL.min(gap / MIN_OFFSET_GAP).floor().max(n);
        let far = centered && lo + n > farthest;
        let (lo, capped) = if far { ((farthest - n).max(0.0), true) } else { (lo, capped) };
        // `lo` is a whole number, possibly past what an integer holds: its parity in floating point.
        let first_odd = lo.rem_euclid(2.0) >= 1.0;
        let (stops, geom) = if centered {
            // The levels from 0, the runs from level `lo` (the first stop's color inside it).
            (self.runs(&one, first_odd, lo, n as usize, lo + n), period.levels_scaled(kind, lo + n))
        } else {
            let u = period.end - period.start;
            (
                self.runs(&one, first_odd, 0.0, n as usize, n),
                GradientGeom { start: period.start + u * lo, end: period.start + u * (lo + n), ..period },
            )
        };
        let gradient = Gradient { stops, spread: GradientSpread::Pad, repeats: 1, ..self.clone() };
        let first = if centered { lo / (lo + n) } else { 0.0 };
        Some(Padded { gradient, geom, first, runs: n as usize, capped })
    }

    /// The gradient as it runs from the start of its vector (0) to its end (1), as one that
    /// pads: a gradient that repeats or reflects with its stops repeated [`Self::repeat_count`]
    /// times between 0 and 1 (every other run mirrored, for Reflect), for what only shows that
    /// stretch (a gradient along or across a stroke, a swatch's preview). Others as they are.
    pub fn unrolled(&self) -> std::borrow::Cow<'_, Gradient> {
        self.unrolled_within(MAX_SPREAD_STOPS)
    }

    /// [`Self::unrolled`] with at most `max_stops` stops (and at least one run): past the runs
    /// that fit, the last one pads.
    pub fn unrolled_within(&self, max_stops: usize) -> std::borrow::Cow<'_, Gradient> {
        use std::borrow::Cow;
        if !self.is_spread() {
            return Cow::Borrowed(self);
        }
        // Without stops there is nothing to repeat: it pads all the same.
        if self.stops.is_empty() {
            return Cow::Owned(Gradient { spread: GradientSpread::Pad, repeats: 1, ..self.clone() });
        }
        let one = whole_run(&self.stops);
        let n = self.repeat_count() as usize;
        let kept = n.min(max_stops.min(MAX_SPREAD_STOPS) / one.len().max(1)).max(1);
        let stops = self.runs(&one, false, 0.0, kept, n as f64);
        Cow::Owned(Gradient { stops, spread: GradientSpread::Pad, repeats: 1, ..self.clone() })
    }

    /// One cycle of a gradient that repeats or reflects, which repeated end to end paints it, as
    /// a gradient that pads: one run of the stops (Repeat), or a run and its mirror (Reflect),
    /// between 0 and 1, and how many runs that is (1 or 2). `None` for a gradient that pads and
    /// one without stops.
    pub fn cycle(&self) -> Option<(Gradient, u32)> {
        if !self.is_spread() || self.stops.is_empty() {
            return None;
        }
        let one = whole_run(&self.stops);
        let runs = if self.spread == GradientSpread::Reflect { 2 } else { 1 };
        let stops = self.runs(&one, false, 0.0, runs, runs as f64);
        Some((Gradient { stops, spread: GradientSpread::Pad, repeats: 1, ..self.clone() }, runs as u32))
    }

    /// `n` runs of `one` (one run of the stops, [`whole_run`]) on a line of `of` runs from 0 to 1,
    /// the first of them `start` runs along it and an odd-numbered run when `first_odd` (with
    /// Reflect, the odd-numbered runs are mirrored).
    fn runs(&self, one: &[GradientStop], first_odd: bool, start: f64, n: usize, of: f64) -> Vec<GradientStop> {
        let mirrored_run = (self.spread == GradientSpread::Reflect).then(|| mirror(one));
        let mut stops = Vec::with_capacity(one.len() * n);
        for j in 0..n {
            let run = match &mirrored_run {
                Some(m) if (j % 2 == 1) != first_odd => m.as_slice(),
                _ => one,
            };
            stops.extend(run.iter().map(|s| GradientStop { offset: ((start + j as f64 + f64::from(s.offset)) / of) as f32, ..s.clone() }));
        }
        stops
    }
}

/// The smallest level of radial or diamond gradient `geom` over `area`, less a run (for rounding):
/// 0 when the area holds the rings' center (a radial's focal point, a diamond's center), else the
/// smallest along its sides, worked out in the gradient's unit frame from the sides' ends rather
/// than from samples along them, so a long side does not hide a run near it. 0 for a placement
/// without a frame.
fn inner_level(geom: &GradientGeom, kind: GradientKind, area: Rect) -> f64 {
    let center = if kind == GradientKind::Radial { geom.focal_point() } else { geom.start };
    if area.contains(center) {
        return 0.0;
    }
    let frame = if kind == GradientKind::Diamond { geom.diamond_frame() } else { geom.unit_frame() };
    let Some(to_unit) = frame.map(|m| m.inverse()) else { return 0.0 };
    let focal = geom.focal.filter(|_| kind == GradientKind::Radial).map(|f| (to_unit * f).to_vec2());
    let corners = [Point::new(area.x0, area.y0), Point::new(area.x1, area.y0), Point::new(area.x1, area.y1), Point::new(area.x0, area.y1)]
        .map(|p| (to_unit * p).to_vec2());
    let mut least = f64::INFINITY;
    for (&a, &b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let side = if kind == GradientKind::Diamond { diamond_least(a, b) } else { radial_least(a, b, focal) };
        least = least.min(side);
    }
    if least.is_finite() { (least - 1.0).max(0.0) } else { 0.0 }
}

/// The smallest diamond level, |x| + |y| in its unit frame, on the segment from `a` to `b`: the
/// level is linear between the points where the segment crosses an axis, so the smallest is at
/// an end or at one of those crossings.
fn diamond_least(a: Vec2, b: Vec2) -> f64 {
    let d = b - a;
    let crossings = [(d.x != 0.0).then(|| -a.x / d.x), (d.y != 0.0).then(|| -a.y / d.y)];
    [Some(0.0), Some(1.0), crossings[0], crossings[1]]
        .into_iter()
        .flatten()
        .filter(|u| (0.0..=1.0).contains(u))
        .map(|u| {
            let p = a + d * u;
            p.x.abs() + p.y.abs()
        })
        .fold(f64::INFINITY, f64::min)
}

/// Halvings at most in [`radial_least`]: enough to bring any finite span below a quarter level.
const RADIAL_HALVINGS: usize = 1100;

/// The smallest radial level on the segment from `a` to `b` in the unit frame, at most a quarter
/// level under it: the distance to the center for a centered radial. An off-center radial's level
/// `t` is the circle of radius `t` about `f`(1 − `t`), `f` its focal point; the segment's distance
/// from that center less `t` falls as `t` grows (the center moves |`f`| < 1 per level), and the
/// smallest level is where it reaches 0, found by halving. 0 for a focal point not inside the
/// extent ellipse.
fn radial_least(a: Vec2, b: Vec2, focal: Option<Vec2>) -> f64 {
    let Some(f) = focal else { return distance_to_segment(a, b, Vec2::ZERO) };
    let reach = f.hypot();
    if reach.is_nan() || reach >= 1.0 {
        return 0.0;
    }
    let past = |t: f64| distance_to_segment(a, b, f * (1.0 - t)) <= t;
    // At this level the circle reaches the segment's nearest point to the ellipse's center.
    let mut hi = (distance_to_segment(a, b, Vec2::ZERO) / (1.0 - reach)).max(1.0);
    let mut lo = 0.0;
    if !hi.is_finite() || past(lo) {
        return 0.0;
    }
    for _ in 0..RADIAL_HALVINGS {
        let mid = lo + (hi - lo) / 2.0;
        if hi - lo <= 0.25 || mid <= lo || mid >= hi {
            break;
        }
        if past(mid) { hi = mid } else { lo = mid }
    }
    lo
}

/// How far `c` is from the segment from `a` to `b`.
fn distance_to_segment(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    let d = b - a;
    let along = (c - a).dot(d) / d.hypot2();
    let u = if along.is_finite() { along.clamp(0.0, 1.0) } else { 0.0 };
    (a + d * u - c).hypot()
}

/// The stops of one run, from offset 0 to 1: the end colors held out to the ends.
fn whole_run(stops: &[GradientStop]) -> Vec<GradientStop> {
    let mut out = stops.to_vec();
    if let Some(first) = out.first().filter(|s| s.offset > 0.0).cloned() {
        out.insert(0, GradientStop { offset: 0.0, ..first });
    }
    if let Some(last) = out.last_mut() {
        // The last stop's midpoint leads nowhere; the copy at 1 takes over.
        if last.offset < 1.0 {
            let end = GradientStop { offset: 1.0, ..last.clone() };
            out.push(end);
        }
    }
    out
}

/// A run mirrored: the stops in reverse, each midpoint measured from the other side.
fn mirror(run: &[GradientStop]) -> Vec<GradientStop> {
    let mut g = Gradient::new(GradientKind::Linear, run.to_vec());
    g.reverse();
    g.stops
}

#[cfg(test)]
mod tests;
