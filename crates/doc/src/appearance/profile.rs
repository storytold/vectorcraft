use std::borrow::Cow;

use super::{Deserialize, Serialize};

/// Variable-width profile: (position 0..1 along the path, left width factor, right width factor),
/// in order along the path. Two points at the same position make a discontinuous point: the
/// first is the width just before it, the second the width just after it (a step).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WidthProfile {
    pub points: Vec<(f64, f64, f64)>,
}

/// A built-in width profile (the Stroke panel's Profile list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfilePreset {
    /// Stable id used by `stroke.set {profile}`.
    pub id: &'static str,
    /// Menu label.
    pub label: &'static str,
    /// (t, left, right) width points.
    pub points: &'static [(f64, f64, f64)],
}

impl WidthProfile {
    /// (left, right) width factors at `t`: a smooth curve through the points ([`Self::at_points`]).
    pub fn at(&self, t: f64) -> (f64, f64) {
        Self::at_points(&self.points, t)
    }
    /// [`Self::at`] of a profile's points (a preset's or a saved profile's).
    ///
    /// Each side runs through its points on a cubic Hermite curve, its slope at each one the
    /// weighted harmonic mean of the secants either side (Fritsch–Carlson). That is C1 continuous
    /// and monotone between points, so it never overshoots them, a point eases into its neighbours
    /// instead of making a corner, and moving one reshapes the curve as far as its neighbours'
    /// neighbours (their slopes depend on it). The ends of a run use the one-sided PCHIP slope
    /// from its first two secants, or the secant when it has only two points, so a two-point
    /// profile stays a straight taper and the lens is a parabola. Two points at one `t` are still
    /// a step: each run either side ends independently at it.
    pub fn at_points(p: &[(f64, f64, f64)], t: f64) -> (f64, f64) {
        if p.is_empty() {
            return (1.0, 1.0);
        }
        if t <= p[0].0 {
            return (p[0].1, p[0].2);
        }
        // (Positions go forward; found in log time, so a long profile read from a file is cheap to ask.)
        let i = p.partition_point(|q| q.0 < t);
        if let (Some(j), Some(b)) = (i.checked_sub(1), p.get(i))
            && let Some(a) = p.get(j)
        {
            let h = b.0 - a.0;
            let u = (t - a.0) / h.max(1e-9);
            let linear = (a.1 + (b.1 - a.1) * u, a.2 + (b.2 - a.2) * u);
            if h <= STEP {
                return linear;
            }
            let side = |v: fn(&(f64, f64, f64)) -> f64, straight: f64| {
                let y = hermite(v(a), v(b), tangent(p, j, v) * h, tangent(p, i, v) * h, u);
                // Widths read from a file may be huge: where the curve overflows, the straight line.
                if y.is_finite() { y } else { straight }
            };
            return (side(|q| q.1, linear.0), side(|q| q.2, linear.1));
        }
        p.last().map_or((1.0, 1.0), |l| (l.1, l.2))
    }
    /// The built-in profiles, in menu order. "uniform" is the plain stroke (no profile).
    pub const PRESETS: [ProfilePreset; 7] = [
        ProfilePreset { id: "uniform", label: "Uniform", points: &[(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] },
        ProfilePreset { id: "lens", label: "Lens", points: &[(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        ProfilePreset { id: "taperStart", label: "Taper Start", points: &[(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)] },
        ProfilePreset { id: "taperEnd", label: "Taper End", points: &[(0.0, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        // Full width at both ends, pinched to a quarter in the middle.
        ProfilePreset { id: "pinch", label: "Pinch", points: &[(0.0, 1.0, 1.0), (0.5, 0.25, 0.25), (1.0, 1.0, 1.0)] },
        // A round head a fifth of the way along, then a long taper to the end.
        ProfilePreset { id: "teardrop", label: "Teardrop", points: &[(0.0, 0.0, 0.0), (0.2, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        // Two swells between narrow necks.
        ProfilePreset { id: "wave", label: "Wave", points: &[(0.0, 0.3, 0.3), (0.25, 1.0, 1.0), (0.5, 0.3, 0.3), (0.75, 1.0, 1.0), (1.0, 0.3, 0.3)] },
    ];
    /// The widest the stroke gets, as a multiple of its weight, 1 for a profile without points.
    /// Each side's smooth curve stays between the widths of the two points of each span
    /// ([`Self::at_points`]), but the two sides needn't peak together, so a span counts its wider
    /// left and its wider right.
    pub fn widest(&self) -> f64 {
        let p = &self.points;
        if p.is_empty() {
            return 1.0;
        }
        let total = |l: f64, r: f64| (l.max(0.0) + r.max(0.0)) / 2.0;
        let spans = p.windows(2).map(|w| total(w[0].1.max(w[1].1), w[0].2.max(w[1].2)));
        p.iter().map(|&(_, l, r)| total(l, r)).chain(spans).fold(0.0, f64::max)
    }
    /// The built-in profile with this id.
    pub fn preset(id: &str) -> Option<Self> {
        Self::PRESETS.iter().find(|p| p.id == id).map(|p| Self { points: p.points.to_vec() })
    }
    /// The id of the built-in profile these points match, if any.
    pub fn preset_id(&self) -> Option<&'static str> {
        Self::PRESETS.iter().find(|p| p.points == self.points.as_slice()).map(|p| p.id)
    }
    /// The id of a stroke's profile: "uniform" without one, "custom" when it matches no preset.
    pub fn id_of(p: Option<&Self>) -> &'static str {
        p.map_or(Some("uniform"), Self::preset_id).unwrap_or("custom")
    }
    /// The lens profile (thin ends, full width in the middle).
    pub fn lens() -> Self {
        Self::preset("lens").unwrap_or_default()
    }
    pub fn taper_end() -> Self {
        Self::preset("taperEnd").unwrap_or_default()
    }
    pub fn taper_start() -> Self {
        Self::preset("taperStart").unwrap_or_default()
    }
    /// The (left, right) factors just before and just after `t`: they differ only at a
    /// discontinuous point (two points at `t`, within `1e-7`).
    pub fn around(&self, t: f64) -> ((f64, f64), (f64, f64)) {
        let p = &self.points;
        // The same distance test below, so a point exactly 1e-7 before `t` is never skipped by rounding.
        let i = p.partition_point(|q| q.0 < t && (q.0 - t).abs() > 1e-7);
        if let Some(first) = p.get(i).filter(|q| (q.0 - t).abs() <= 1e-7) {
            let j = i + p[i..].partition_point(|q| q.0 - first.0 <= 1e-9) - 1;
            if j > i {
                return ((first.1, first.2), (p[j].1, p[j].2));
            }
        }
        let v = self.at(t);
        (v, v)
    }
}

/// Pen pressure recorded along a stroke (the Paintbrush with a graphics tablet): (position 0..1
/// along the path, pressure 0..1) samples in order, positions measured as a [`WidthProfile`]'s
/// are. The pen's tilt is kept the same way ([`StrokeLayer::tilt`]). Calligraphic and Scatter
/// brushes with Pressure or Tilt variation read them; everything else ignores them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PressureProfile {
    pub points: Vec<(f64, f64)>,
}

impl PressureProfile {
    /// The pressure of a stroke drawn without one (a mouse, or a path the brush was applied to):
    /// half way, so a Pressure brush draws its own value there.
    pub const MID: f64 = 0.5;
    /// The most samples a stroke keeps (after simplifying them).
    pub const MAX_POINTS: usize = 512;
    /// How far (in pressure) simplifying may move the recorded curve.
    const TOLERANCE: f64 = 0.01;

    /// The pressure at `t` (0..1), linear between samples; [`Self::MID`] without any.
    pub fn at(&self, t: f64) -> f64 {
        let v = interpolated(&self.points, t).unwrap_or(Self::MID);
        if v.is_finite() { v.clamp(0.0, 1.0) } else { Self::MID }
    }

    /// A profile from (position, pressure) samples in order along the stroke: values are clamped
    /// into 0..1 (non-finite ones dropped), the curve is simplified to within 1 % and kept to
    /// [`Self::MAX_POINTS`]. `None` without a sample.
    pub fn from_samples(samples: impl IntoIterator<Item = (f64, f64)>) -> Option<Self> {
        let pts: Vec<(f64, f64)> =
            samples.into_iter().filter(|(t, p)| t.is_finite() && p.is_finite()).map(|(t, p)| (t.clamp(0.0, 1.0), p.clamp(0.0, 1.0))).collect();
        simplified(pts, Self::TOLERANCE).map(|points| Self { points })
    }

    /// The pressure of a path `old_len` long (`old`; None: drawn without pressure) continued at its
    /// end by a stroke `added_len` long (`added`). None when neither has pressure.
    pub fn extended(old: Option<&Self>, old_len: f64, added: Option<&Self>, added_len: f64) -> Option<Self> {
        match extended_points(old.map(|p| p.points.as_slice()), old_len, added.map(|p| p.points.as_slice()), added_len, Self::MID)? {
            Cow::Borrowed(points) => Some(Self { points: points.to_vec() }),
            Cow::Owned(points) => Self::from_samples(points),
        }
    }

    /// The pressure of the path reversed.
    pub fn reversed(&self) -> Self {
        Self { points: reversed_points(&self.points) }
    }

    /// A light–heavy–light stroke, for previews of pressure-sensitive brushes.
    pub fn sample() -> Self {
        Self { points: vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)] }
    }
}

/// A pen angle recorded along a stroke (the Paintbrush with a tablet that reads it): (position 0..1
/// along the path, degrees) samples in order, positions measured as a [`PressureProfile`]'s are.
/// Each sample is joined to the one before it the shorter way round, so the degrees can run past
/// ±180° and the angle read between two samples turns through the smaller angle between them.
/// Calligraphic and Scatter brushes with Bearing or Rotation variation read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AngleProfile {
    pub points: Vec<(f64, f64)>,
}

impl AngleProfile {
    /// How far (in degrees) simplifying may move the recorded curve.
    const TOLERANCE: f64 = 1.0;

    /// The angle at `t` (0..1), linear between samples, in degrees above −180 up to 180; 0
    /// without any samples.
    pub fn at(&self, t: f64) -> f64 {
        let v = interpolated(&self.points, t).unwrap_or(0.0);
        if v.is_finite() { vectorcraft_geom::normalize_deg(v) } else { 0.0 }
    }

    /// The angle at `t` (0..1) as [`Self::at`] reads it, but in the samples' own running degrees,
    /// so the whole turns the curve makes along the stroke are kept; 0 without any samples.
    pub fn unwrapped_at(&self, t: f64) -> f64 {
        let v = interpolated(&self.points, t).unwrap_or(0.0);
        if v.is_finite() { v } else { 0.0 }
    }

    /// A profile from (position, degrees) samples in order along the stroke: non-finite ones are
    /// dropped, each sample is joined to the one before it the shorter way round, and the curve is
    /// simplified to within 1° and kept to [`PressureProfile::MAX_POINTS`]. `None` without a sample.
    pub fn from_samples(samples: impl IntoIterator<Item = (f64, f64)>) -> Option<Self> {
        let mut prev: Option<f64> = None;
        let pts: Vec<(f64, f64)> = samples
            .into_iter()
            .filter(|(t, a)| t.is_finite() && a.is_finite())
            .map(|(t, a)| {
                let a = prev.map_or(vectorcraft_geom::normalize_deg(a), |p| p + vectorcraft_geom::normalize_deg(a - p));
                prev = Some(a);
                (t.clamp(0.0, 1.0), a)
            })
            .collect();
        simplified(pts, Self::TOLERANCE).map(|points| Self { points })
    }

    /// The angle of a path `old_len` long (`old`; None: drawn without it, 0°) continued at its end
    /// by a stroke `added_len` long (`added`). None when neither has one. Each part keeps the turns
    /// it recorded, more than half a turn between two samples included; the added part is moved by
    /// whole turns so that it starts the shorter way round from where the old part ends.
    pub fn extended(old: Option<&Self>, old_len: f64, added: Option<&Self>, added_len: f64) -> Option<Self> {
        let end = old.map_or(Some(0.0), |o| o.points.last().map(|p| p.1));
        let added = added.map(|a| {
            let shift = match (end, a.points.first()) {
                (Some(end), Some(&(_, start))) => end + vectorcraft_geom::normalize_deg(start - end) - start,
                _ => 0.0,
            };
            Self { points: a.points.iter().map(|&(t, v)| (t, v + shift)).collect() }
        });
        match extended_points(old.map(|p| p.points.as_slice()), old_len, added.as_ref().map(|p| p.points.as_slice()), added_len, 0.0)? {
            Cow::Borrowed(points) => Some(Self { points: points.to_vec() }),
            Cow::Owned(points) => {
                let pts = points.into_iter().filter(|(t, a)| t.is_finite() && a.is_finite()).map(|(t, a)| (t.clamp(0.0, 1.0), a)).collect();
                simplified(pts, Self::TOLERANCE).map(|points| Self { points })
            }
        }
    }

    /// The angle along the path reversed (the angles stay as they were on the page).
    pub fn reversed(&self) -> Self {
        Self { points: reversed_points(&self.points) }
    }

    /// One turn through every angle along the stroke, for previews of brushes that vary with one.
    pub fn sample() -> Self {
        Self { points: vec![(0.0, -179.0), (1.0, 179.0)] }
    }
}

/// How close two width points' positions are to make a step ([`WidthProfile::around`]).
const STEP: f64 = 1e-7;

/// The one-sided three-point PCHIP slope at the start of a run, from the first two secants.
/// A sign change caps the slope, and a slope that turns back on the first secant is flattened.
/// Runs of only two points use their secant instead (see [`tangent`]).
fn end_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
    let m = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
    if !m.is_finite() || m * d0 <= 0.0 {
        return 0.0;
    }
    if d0 * d1 < 0.0 {
        return if d0 > 0.0 { m.min(3.0 * d0) } else { m.max(3.0 * d0) };
    }
    m
}

/// The slope of one side (`v` reads it from a point) of width profile `p` at its point `j`, in
/// width per unit of position: Fritsch–Carlson's choice for a monotone cubic curve. It is 0 where
/// the point is wider or narrower than both neighbours, and the weighted harmonic mean of the
/// slopes of the spans on either side elsewhere (never more than three times either, so no span
/// overshoots its points). The ends of a run take the one-sided three-point PCHIP slope, or their
/// secant when the run has only two points; a step ends the run on each side, so the points either
/// side of it do not see across it.
fn tangent(p: &[(f64, f64, f64)], j: usize, v: fn(&(f64, f64, f64)) -> f64) -> f64 {
    let Some(q) = p.get(j) else { return 0.0 };
    let before = j.checked_sub(1).and_then(|k| p.get(k)).filter(|a| q.0 - a.0 > STEP);
    let after = p.get(j + 1).filter(|b| b.0 - q.0 > STEP);
    let slope = |a: &(f64, f64, f64), b: &(f64, f64, f64)| (v(b) - v(a)) / (b.0 - a.0);
    match (before, after) {
        (Some(a), Some(b)) => {
            let (d0, d1) = (slope(a, q), slope(q, b));
            if d0 * d1 <= 0.0 {
                return 0.0;
            }
            let (h0, h1) = (q.0 - a.0, b.0 - q.0);
            3.0 * (h0 + h1) / ((2.0 * h1 + h0) / d0 + (h1 + 2.0 * h0) / d1)
        }
        // The end of a run: its previous two points set a one-sided three-point slope, or its
        // secant when the run has only two points (the point before `a` is on the other side of a
        // step, or does not exist).
        (Some(a), None) => {
            let Some(prev) = j.checked_sub(2).and_then(|k| p.get(k)).filter(|z| a.0 - z.0 > STEP) else { return slope(a, q) };
            end_slope(q.0 - a.0, a.0 - prev.0, slope(a, q), slope(prev, a))
        }
        // The start of a run, symmetrically.
        (None, Some(b)) => {
            let Some(next) = p.get(j + 2).filter(|z| z.0 - b.0 > STEP) else { return slope(q, b) };
            end_slope(b.0 - q.0, next.0 - b.0, slope(q, b), slope(b, next))
        }
        (None, None) => 0.0,
    }
}

/// The cubic Hermite curve from `y0` to `y1` at `u` (0..1), with end slopes `m0` and `m1` per
/// whole span.
fn hermite(y0: f64, y1: f64, m0: f64, m1: f64, u: f64) -> f64 {
    let (u2, u3) = (u * u, u * u * u);
    (2.0 * u3 - 3.0 * u2 + 1.0) * y0 + (u3 - 2.0 * u2 + u) * m0 + (-2.0 * u3 + 3.0 * u2) * y1 + (u3 - u2) * m1
}

/// The value of samples `p` (position, value) at `t`, linear between them; `None` without any.
fn interpolated(p: &[(f64, f64)], t: f64) -> Option<f64> {
    let (first, last) = (p.first()?, p.last()?);
    if t <= first.0 {
        return Some(first.1);
    }
    // (Positions go forward; a profile read from a file is searched as if they do, in log time.)
    let i = p.partition_point(|q| q.0 < t);
    Some(match (i.checked_sub(1).and_then(|j| p.get(j)), p.get(i)) {
        (Some(a), Some(b)) => a.1 + (b.1 - a.1) * ((t - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0),
        _ => last.1,
    })
}

/// Samples (position, value) with positions in 0..1 and finite values, simplified to within `tol`
/// of the value and kept to [`PressureProfile::MAX_POINTS`]. `None` without a sample.
fn simplified(pts: Vec<(f64, f64)>, tol: f64) -> Option<Vec<(f64, f64)>> {
    if pts.is_empty() {
        return None;
    }
    // Simplifying is quadratic at worst: a huge stroke is thinned evenly first.
    let mut pts = thin(pts, 16 * PressureProfile::MAX_POINTS);
    // Positions only ever go forward.
    let mut prev = 0.0;
    for p in &mut pts {
        p.0 = p.0.max(prev);
        prev = p.0;
    }
    let keep = simplify_1d(&pts, tol);
    let pts = thin(pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect(), PressureProfile::MAX_POINTS);
    let round = |v: f64| (v * 1e4).round() / 1e4;
    Some(pts.into_iter().map(|(t, p)| (round(t), round(p))).collect())
}

/// The samples of a path `old_len` long (`old`) continued at its end by a stroke `added_len` long
/// (`added`), in order along the whole path, a side without samples (`None`) flat at `none`.
/// `None` when neither side has samples; the side that has them, as they are, when the two
/// lengths add up to no length.
fn extended_points<'a>(
    old: Option<&'a [(f64, f64)]>,
    old_len: f64,
    added: Option<&'a [(f64, f64)]>,
    added_len: f64,
    none: f64,
) -> Option<Cow<'a, [(f64, f64)]>> {
    if old.is_none() && added.is_none() {
        return None;
    }
    let (old_len, added_len) = (old_len.max(0.0), added_len.max(0.0));
    let total = old_len + added_len;
    if !total.is_finite() || total <= 1e-9 {
        return old.or(added).map(Cow::Borrowed);
    }
    let flat = [(0.0, none), (1.0, none)];
    let (old, added) = (old.unwrap_or(&flat), added.unwrap_or(&flat));
    let o = old_len / total;
    let old_pts = old.iter().map(|&(t, p)| (t * o, p));
    Some(Cow::Owned(old_pts.chain(added.iter().map(|&(u, p)| (o + u * (1.0 - o), p))).collect()))
}

/// Samples (position, value) of a path, for the path reversed.
fn reversed_points(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    points.iter().rev().map(|&(t, p)| (1.0 - t, p)).collect()
}

/// At most `max` of `pts`, evenly spread (the ends kept).
fn thin(pts: Vec<(f64, f64)>, max: usize) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n <= max || max < 2 {
        return pts;
    }
    (0..max).filter_map(|i| pts.get(i * (n - 1) / (max - 1)).copied()).collect()
}

/// Douglas–Peucker over a function sampled at `pts`: which samples to keep so the polyline through
/// them stays within `tol` (vertically) of every sample. The ends are always kept.
fn simplify_1d(pts: &[(f64, f64)], tol: f64) -> Vec<bool> {
    let mut keep = vec![false; pts.len()];
    let Some(last) = pts.len().checked_sub(1) else { return keep };
    for i in [0, last] {
        if let Some(k) = keep.get_mut(i) {
            *k = true;
        }
    }
    let mut stack = vec![(0, last)];
    while let Some((i, j)) = stack.pop() {
        let (Some(&a), Some(&b)) = (pts.get(i), pts.get(j)) else { continue };
        let line = |t: f64| if b.0 - a.0 > 1e-12 { a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0) } else { (a.1 + b.1) / 2.0 };
        let far = (i + 1..j).filter_map(|k| pts.get(k).map(|q| (k, (q.1 - line(q.0)).abs()))).max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((k, d)) = far
            && d > tol
        {
            if let Some(f) = keep.get_mut(k) {
                *f = true;
            }
            stack.push((i, k));
            stack.push((k, j));
        }
    }
    keep
}

/// A width profile saved to the Profile list under a name (Add to Profiles), kept with the
/// preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedProfile {
    pub name: String,
    pub profile: WidthProfile,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Widths and steps are found by search, at the ends, between points and at a step.
    #[test]
    fn width_profile_lookups_by_search() {
        let p = WidthProfile { points: vec![(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (0.5, 0.5, 0.25), (1.0, 0.0, 0.0)] };
        assert_eq!(p.at(-1.0), (0.0, 0.0));
        assert_eq!(p.at(0.25), (0.5, 0.5));
        assert_eq!(p.at(0.75), (0.25, 0.125));
        assert_eq!(p.at(2.0), (0.0, 0.0));
        // A step: the widths just before and just after it.
        assert_eq!(p.around(0.5), ((1.0, 1.0), (0.5, 0.25)));
        assert_eq!(p.around(0.25), ((0.5, 0.5), (0.5, 0.5)));
        // Many points at one position are one step, found without a pass over them all.
        let many = WidthProfile { points: (0..1000).map(|i| (0.5, i as f64, i as f64)).collect() };
        assert_eq!(many.around(0.5), ((0.0, 0.0), (999.0, 999.0)));
        // 0.4999999 == 0.5 - 1e-7, yet its distance from 0.5 rounds above 1e-7: it is not part of the step.
        let near = WidthProfile { points: vec![(0.4999999, 2.0, 2.0), (0.5, 1.0, 1.0), (0.5, 0.5, 0.25)] };
        assert_eq!(near.around(0.5), ((1.0, 1.0), (0.5, 0.25)));
    }

    /// The slope of one side of `p` just before and just after `t`.
    fn slopes(p: &WidthProfile, t: f64) -> (f64, f64) {
        let e = 1e-6;
        ((p.at(t).0 - p.at(t - e).0) / e, (p.at(t + e).0 - p.at(t).0) / e)
    }

    /// #5714: the width flows through a width point instead of turning a corner there.
    #[test]
    fn widths_run_smoothly_through_width_points() {
        // A swell in the middle: flat at its widest, and a quarter of the way along above the
        // straight ramp from 1 to 3 (2) because the end eases out of the first point.
        let swell = WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 3.0, 2.0), (1.0, 1.0, 1.0)] };
        let (before, after) = slopes(&swell, 0.5);
        assert!(before.abs() < 1e-4 && after.abs() < 1e-4, "{before} {after}");
        let (l, r) = swell.at(0.25);
        assert!((l - 2.5).abs() < 1e-12 && (r - 1.75).abs() < 1e-12, "{l} {r}");
        // Rising on both sides of a point: the slope is the same either side of it.
        let rise = WidthProfile { points: vec![(0.0, 0.0, 0.0), (0.3, 1.0, 1.0), (1.0, 4.0, 4.0)] };
        let (before, after) = slopes(&rise, 0.3);
        assert!(before > 0.0 && (before - after).abs() < 1e-3 * before, "{before} {after}");
        // Points on one line stay on it.
        let straight = WidthProfile { points: vec![(0.0, 0.0, 1.0), (0.25, 0.5, 1.0), (1.0, 2.0, 1.0)] };
        for t in [0.1, 0.25, 0.6, 0.9] {
            let (l, r) = straight.at(t);
            assert!((l - 2.0 * t).abs() < 1e-12 && (r - 1.0).abs() < 1e-12, "{t}: {l} {r}");
        }
    }

    /// Neither side of a profile goes wider or narrower than the two points of the span it is in.
    #[test]
    fn widths_never_pass_the_points_either_side() {
        for preset in WidthProfile::PRESETS {
            let p = WidthProfile { points: preset.points.to_vec() };
            for w in p.points.windows(2) {
                let (lo, hi) = (w[0].1.min(w[1].1), w[0].1.max(w[1].1));
                for k in 0..=100 {
                    let t = w[0].0 + (w[1].0 - w[0].0) * k as f64 / 100.0;
                    let l = p.at(t).0;
                    assert!(l >= lo - 1e-12 && l <= hi + 1e-12, "{}: {l} at {t} outside {lo}..{hi}", preset.id);
                }
            }
        }
        // Uneven spacing and a steep jump next to a gentle rise.
        let p = WidthProfile { points: vec![(0.0, 0.0, 0.0), (0.05, 1.0, 1.0), (0.9, 1.1, 1.1), (1.0, 5.0, 5.0)] };
        for k in 0..=1000 {
            let t = k as f64 / 1000.0;
            let l = p.at(t).0;
            let span = p.points.windows(2).find(|w| t >= w[0].0 && t <= w[1].0).map(|w| (w[0].1.min(w[1].1), w[0].1.max(w[1].1)));
            let (lo, hi) = span.unwrap();
            assert!(l >= lo - 1e-12 && l <= hi + 1e-12, "{l} at {t}");
        }
    }

    /// A step still breaks the width sharply, and the curve on either side ends at it.
    #[test]
    fn a_step_ends_the_curve_on_each_side() {
        let p = WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 3.0, 3.0), (0.5, 1.0, 1.0), (1.0, 2.0, 2.0)] };
        assert_eq!(p.around(0.5), ((3.0, 3.0), (1.0, 1.0)));
        // Two points between ends: each side of the step is a straight ramp.
        assert!((p.at(0.25).0 - 2.0).abs() < 1e-12 && (p.at(0.75).0 - 1.5).abs() < 1e-12);
    }

    /// `widest()` is at least the stroke's width anywhere along the profile, even where the sides peak apart.
    #[test]
    fn widest_covers_every_width_along_the_profile() {
        let profiles = [
            vec![(0.0, 1.0, 0.0), (0.5, 0.2, 1.0), (1.0, 1.0, 0.0)],
            vec![(0.0, 0.0, 1.0), (0.3, 1.0, 0.0), (0.6, 0.0, 1.0), (1.0, 1.0, 1.0)],
            vec![(0.0, 0.3, 0.3), (0.25, 1.0, 1.0), (0.5, 0.3, 0.3), (0.75, 1.0, 1.0), (1.0, 0.3, 0.3)],
        ];
        for points in profiles {
            let p = WidthProfile { points };
            let most = (0..=1000).map(|k| p.at(k as f64 / 1000.0)).map(|(l, r)| (l.max(0.0) + r.max(0.0)) / 2.0).fold(0.0, f64::max);
            assert!(p.widest() >= most - 1e-12, "{} < {most}: {:?}", p.widest(), p.points);
        }
        assert_eq!(WidthProfile::lens().widest(), 1.0);
        assert_eq!(WidthProfile::default().widest(), 1.0);
    }

    /// Widths read from a file may be absurd: the curve stays finite or falls back to the line.
    #[test]
    fn huge_widths_stay_finite() {
        let p = WidthProfile { points: vec![(0.0, 0.0, 0.0), (0.5, f64::MAX, 1.0), (1.0, -f64::MAX, 0.0)] };
        for k in 0..=10 {
            let (l, r) = p.at(k as f64 / 10.0);
            assert!(!l.is_nan() && r.is_finite(), "{l} {r}");
        }
    }
}
