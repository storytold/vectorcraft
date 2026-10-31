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
    /// (left, right) width factors at `t`: a smooth cubic between the points.
    pub fn at(&self, t: f64) -> (f64, f64) {
        Self::at_points(&self.points, t)
    }
    /// [`Self::at`] of a profile's points (a preset's or a saved profile's).
    ///
    /// The width runs as a cubic Hermite curve through the points, its slope at each one the
    /// weighted harmonic mean of the secants either side (Fritsch–Carlson). That is C1
    /// continuous and monotone between points, so it never overshoots them, a point eases into
    /// its neighbours instead of making a corner, and moving one reshapes the curve as far as its
    /// neighbours' neighbours (their slopes depend on it). The ends use the one-sided PCHIP
    /// slope, or the secant when there are only two points, so a two-point profile stays a
    /// straight taper. Two points at one `t` are still a step: the runs either side are
    /// independent and flat there.
    pub fn at_points(p: &[(f64, f64, f64)], t: f64) -> (f64, f64) {
        let (Some(first), Some(last)) = (p.first(), p.last()) else { return (1.0, 1.0) };
        if t <= first.0 {
            return (first.1, first.2);
        }
        if t >= last.0 {
            return (last.1, last.2);
        }
        // The segment ending at or after `t`: a step's zero-length segment is skipped, so `t`
        // exactly at a step still reads the width just before it (as `around` documents).
        let Some(i) = p.windows(2).position(|w| t <= w[1].0) else { return (last.1, last.2) };
        let (Some(&(t0, l0, r0)), Some(&(t1, l1, r1))) = (p.get(i), p.get(i + 1)) else { return (last.1, last.2) };
        let h = t1 - t0;
        if h <= 1e-12 {
            return (l1, r1);
        }
        let u = (t - t0) / h;
        let (ml0, mr0, ml1, mr1) = (slope(p, i, false), slope(p, i, true), slope(p, i + 1, false), slope(p, i + 1, true));
        (hermite(u, h, l0, l1, ml0, ml1), hermite(u, h, r0, r1, mr0, mr1))
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
        if let Some(i) = p.iter().position(|q| (q.0 - t).abs() <= 1e-7) {
            let j = i + p[i..].iter().take_while(|q| (q.0 - p[i].0).abs() <= 1e-9).count() - 1;
            if j > i {
                return ((p[i].1, p[i].2), (p[j].1, p[j].2));
            }
        }
        let v = self.at(t);
        (v, v)
    }
}

/// The one-sided three-point PCHIP slope at the start of a run, from the first two secants.
/// The caller uses the secant for a run of only two points; a sign change caps the slope, and a
/// slope that turns back on the first secant is flattened.
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

/// The slope of the smooth width curve at `p[i]` for one side: the weighted harmonic mean of the
/// secants either side. The profile's first and last points use the one-sided three-point PCHIP
/// slope instead (or the secant when their run has only two points), so a two-point profile stays
/// a straight taper. Either side of a step, and at a local extremum, the slope is flat.
fn slope(p: &[(f64, f64, f64)], i: usize, right: bool) -> f64 {
    let value = |k: usize| p.get(k).map_or(0.0, |q| if right { q.2 } else { q.1 });
    let Some(&(t, _, _)) = p.get(i) else { return 0.0 };
    if i == 0 {
        let Some(&(t1, _, _)) = p.get(1) else { return 0.0 };
        let h0 = t1 - t;
        if h0 <= 1e-12 {
            return 0.0;
        }
        let d0 = (value(1) - value(0)) / h0;
        let Some(&(t2, _, _)) = p.get(2) else { return d0 };
        let h1 = t2 - t1;
        if h1 <= 1e-12 {
            return d0;
        }
        let d1 = (value(2) - value(1)) / h1;
        return end_slope(h0, h1, d0, d1);
    }
    if i + 1 == p.len() {
        let Some(&(tm, _, _)) = i.checked_sub(1).and_then(|k| p.get(k)) else { return 0.0 };
        let h0 = t - tm;
        if h0 <= 1e-12 {
            return 0.0;
        }
        let d0 = (value(i) - value(i - 1)) / h0;
        let Some(&(tmm, _, _)) = i.checked_sub(2).and_then(|k| p.get(k)) else { return d0 };
        let h1 = tm - tmm;
        if h1 <= 1e-12 {
            return d0;
        }
        let d1 = (value(i - 1) - value(i - 2)) / h1;
        return end_slope(h0, h1, d0, d1);
    }
    let (Some(&(tm, _, _)), Some(&(tp, _, _))) = (i.checked_sub(1).and_then(|k| p.get(k)), p.get(i + 1)) else {
        return 0.0;
    };
    let (h0, h1) = (t - tm, tp - t);
    if h0 <= 1e-12 || h1 <= 1e-12 {
        return 0.0;
    }
    let (d0, d1) = ((value(i) - value(i - 1)) / h0, (value(i + 1) - value(i)) / h1);
    if d0 * d1 <= 0.0 {
        return 0.0;
    }
    let (a, b) = (2.0 * h1 + h0, h1 + 2.0 * h0);
    let m = (a + b) / (a / d0 + b / d1);
    // A file's widths are only checked to be finite: keep a slope that overflows flat.
    if m.is_finite() { m } else { 0.0 }
}

/// A cubic Hermite from `y0` at `u = 0` to `y1` at `u = 1` over `h` (in `t`), with `m0`/`m1` the
/// slopes at its ends (width factor per `t`).
fn hermite(u: f64, h: f64, y0: f64, y1: f64, m0: f64, m1: f64) -> f64 {
    let (u2, u3) = (u * u, u * u * u);
    (2.0 * u3 - 3.0 * u2 + 1.0) * y0 + (u3 - 2.0 * u2 + u) * h * m0 + (3.0 * u2 - 2.0 * u3) * y1 + (u3 - u2) * h * m1
}

/// Pen pressure recorded along a stroke (the Paintbrush with a graphics tablet): (position 0..1
/// along the path, pressure 0..1) samples in order, positions measured as a [`WidthProfile`]'s
/// are. Calligraphic brushes with Pressure variation read it; everything else ignores it.
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
        let p = &self.points;
        let (Some(first), Some(last)) = (p.first(), p.last()) else { return Self::MID };
        // (Positions go forward; a profile read from a file is searched as if they do, in log time.)
        let v = if t <= first.0 {
            first.1
        } else {
            let i = p.partition_point(|q| q.0 < t);
            match (i.checked_sub(1).and_then(|j| p.get(j)), p.get(i)) {
                (Some(a), Some(b)) => a.1 + (b.1 - a.1) * ((t - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0),
                _ => last.1,
            }
        };
        if v.is_finite() { v.clamp(0.0, 1.0) } else { Self::MID }
    }

    /// A profile from (position, pressure) samples in order along the stroke: values are clamped
    /// into 0..1 (non-finite ones dropped), the curve is simplified to within 1 % and kept to
    /// [`Self::MAX_POINTS`]. `None` without a sample.
    pub fn from_samples(samples: impl IntoIterator<Item = (f64, f64)>) -> Option<Self> {
        let pts: Vec<(f64, f64)> =
            samples.into_iter().filter(|(t, p)| t.is_finite() && p.is_finite()).map(|(t, p)| (t.clamp(0.0, 1.0), p.clamp(0.0, 1.0))).collect();
        if pts.is_empty() {
            return None;
        }
        // Simplifying is quadratic at worst: a huge stroke is thinned evenly first.
        let mut pts = thin(pts, 16 * Self::MAX_POINTS);
        // Positions only ever go forward.
        let mut prev = 0.0;
        for p in &mut pts {
            p.0 = p.0.max(prev);
            prev = p.0;
        }
        let keep = simplify_1d(&pts, Self::TOLERANCE);
        let pts = thin(pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect(), Self::MAX_POINTS);
        let round = |v: f64| (v * 1e4).round() / 1e4;
        Some(Self { points: pts.into_iter().map(|(t, p)| (round(t), round(p))).collect() })
    }

    /// The pressure of a path `old_len` long (`old`; None: drawn without pressure) continued at its
    /// end by a stroke `added_len` long (`added`). None when neither has pressure.
    pub fn extended(old: Option<&Self>, old_len: f64, added: Option<&Self>, added_len: f64) -> Option<Self> {
        if old.is_none() && added.is_none() {
            return None;
        }
        let (old_len, added_len) = (old_len.max(0.0), added_len.max(0.0));
        let total = old_len + added_len;
        if !total.is_finite() || total <= 1e-9 {
            return old.or(added).cloned();
        }
        let flat = Self { points: vec![(0.0, Self::MID), (1.0, Self::MID)] };
        let (old, added) = (old.unwrap_or(&flat), added.unwrap_or(&flat));
        let o = old_len / total;
        let old_pts = old.points.iter().map(|&(t, p)| (t * o, p));
        Self::from_samples(old_pts.chain(added.points.iter().map(|&(u, p)| (o + u * (1.0 - o), p))))
    }

    /// The pressure of the path reversed.
    pub fn reversed(&self) -> Self {
        Self { points: self.points.iter().rev().map(|&(t, p)| (1.0 - t, p)).collect() }
    }

    /// A light–heavy–light stroke, for previews of pressure-sensitive brushes.
    pub fn sample() -> Self {
        Self { points: vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)] }
    }
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
