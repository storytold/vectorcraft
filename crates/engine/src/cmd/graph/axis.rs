use super::*;

pub(super) const LABEL_SIZE: f64 = 9.0;

/// Most tick marks per division (Graph Type › Tick Marks), and most category tick marks a graph draws.
pub(super) const MAX_TICK_MARKS: usize = 20;

pub(super) const MAX_CATEGORY_TICKS: usize = 10_000;

/// Longest axis label prefix or suffix (Graph Type › Add Labels).
const MAX_AFFIX: usize = 64;

/// An axis label prefix or suffix: one line (no control characters or line separators), at most [`MAX_AFFIX`] long.
pub(super) fn one_line(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}')).take(MAX_AFFIX).collect()
}

/// A count parameter: a whole number, also as the float a dialog sends back (3.0), at most `max`.
pub(super) fn count_param(p: &Value, key: &str, max: u64) -> Option<usize> {
    let v = p.get(key)?;
    let n = v.as_u64().or_else(|| v.as_f64().filter(|x| x.is_finite() && *x >= 0.0).map(|x| x.round().min(max as f64) as u64))?;
    usize::try_from(n.min(max)).ok()
}

/// A "nice" axis: (min, max, step).
pub(super) fn nice_axis(lo: f64, hi: f64, ticks: usize) -> (f64, f64, f64) {
    let (lo, hi) = if (hi - lo).abs() < 1e-12 { (lo.min(0.0), lo.max(0.0) + 1.0) } else { (lo, hi) };
    let n = if ticks == 0 { 5 } else { ticks.clamp(1, 100) } as f64;
    let raw = (hi - lo) / n;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw - 1e-12).unwrap_or(10.0 * mag);
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

/// A value axis: with both `min` and `max` set (Graph Type › Tick Values › Override Calculated Values), exactly that
/// range split into `ticks` divisions (5 when automatic); otherwise a nice axis around the data, using whichever bound
/// was given.
pub(super) fn value_axis(ticks: usize, axis_min: Option<f64>, axis_max: Option<f64>, lo: f64, hi: f64) -> (f64, f64, f64) {
    match (axis_min, axis_max) {
        (Some(min), Some(max)) if max > min && (max - min).is_finite() => {
            let n = if ticks == 0 { 5 } else { ticks.clamp(1, 100) } as f64;
            (min, max, (max - min) / n)
        }
        _ => {
            // An override too wide for f64 (or NaN) gives a non-finite axis, which would reach `clamp` with NaN bounds
            // and panic: fall back to the data, then to 0..1.
            let finite = |(a, b, s): (f64, f64, f64)| a.is_finite() && b.is_finite() && s.is_finite() && b > a && s > 0.0;
            [nice_axis(axis_min.unwrap_or(lo), axis_max.unwrap_or(hi), ticks), nice_axis(lo, hi, ticks)]
                .into_iter()
                .find(|&a| finite(a))
                .unwrap_or((0.0, 1.0, 0.2))
        }
    }
}

/// The tick values from `lo` to `hi` every `step`, both ends included: counted, not accumulated, so the last tick
/// lands on `hi` and a step too small for the range (or not finite) can't loop forever. At most 1000 ticks.
pub(super) fn tick_values(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let n = ((hi - lo) / step).round();
    if !(lo.is_finite() && step.is_finite() && step > 0.0 && n.is_finite() && n >= 0.0) {
        return vec![];
    }
    (0..=(n as usize).min(1000)).map(|i| lo + i as f64 * step).collect()
}
