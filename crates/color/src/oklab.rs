//! OKLCH, the polar form of Oklab, as CSS Color 4 defines it
//! (<https://www.w3.org/TR/css-color-4/#specifying-oklab-oklch>): lightness L from 0 to 1, chroma
//! C (CSS reads 100% as 0.4) and hue H in degrees from the positive `a` axis. The conversions take
//! and give gamma-encoded sRGB through the Oklab matrices of [`crate::libraries`].

use crate::libraries::{oklab_to_srgb, srgb_to_oklab};

/// The chroma CSS reads as 100%.
pub const CHROMA_FULL: f64 = 0.4;

/// Chroma at or below which the hue is powerless (CSS Color 4's ε); [`to_oklch`] gives hue 0 there.
pub const POWERLESS_CHROMA: f64 = 0.000_004;

/// How far outside 0..=1 an sRGB component may be and still count as inside the gamut.
const GAMUT_SLACK: f64 = 1e-4;

/// OKLCH `[l, c, h°]` of sRGB `rgb` (gamma encoded). Values outside 0..=1 are converted as they
/// are. The hue is in 0..360, and 0 while the chroma is powerless.
pub fn to_oklch(rgb: [f32; 3]) -> [f64; 3] {
    let [l, a, b] = srgb_to_oklab(rgb);
    let c = a.hypot(b);
    if c <= POWERLESS_CHROMA {
        return [l, 0.0, 0.0];
    }
    [l, c, b.atan2(a).to_degrees().rem_euclid(360.0)]
}

/// sRGB (gamma encoded) of OKLCH `(l, c, h°)`, unclamped.
pub fn oklch_to_srgb(l: f64, c: f64, h: f64) -> [f64; 3] {
    let (sin, cos) = h.to_radians().sin_cos();
    oklab_to_srgb(l, c * cos, c * sin)
}

/// Whether sRGB `rgb` lies inside the sRGB gamut.
pub fn in_srgb(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|v| (-GAMUT_SLACK..=1.0 + GAMUT_SLACK).contains(v))
}

/// The chroma past which no hue at any lightness is inside sRGB.
const CHROMA_SEARCH: f64 = 0.5;

/// The largest chroma sRGB holds at lightness `l` and hue `h°`: 0 at or outside black and white.
pub fn max_chroma(l: f64, h: f64) -> f64 {
    fitting_chroma(l, h, CHROMA_SEARCH)
}

/// The largest chroma from 0 to `top` (at most [`CHROMA_SEARCH`]) that sRGB holds at lightness `l`
/// and hue `h°`: 0 at or outside black and white.
///
/// Along a hue at a given lightness each linear sRGB component is a cubic in the chroma, so sRGB can
/// hold more than one run of chroma: near blue a run of low chroma and one ending at the blue
/// primary. A chroma enters or leaves the gamut only where a component's cubic reaches an edge of
/// [`in_srgb`], so the search tests the chromas between those points from the top down and refines
/// the highest run that fits.
fn fitting_chroma(l: f64, h: f64, top: f64) -> f64 {
    if !(l > 0.0 && l < 1.0) || !h.is_finite() || top.is_nan() || top <= 0.0 {
        return 0.0;
    }
    let top = top.min(CHROMA_SEARCH);
    let fits = |c: f64| in_srgb(oklch_to_srgb(l, c, h));
    if fits(top) {
        return top;
    }
    let decode = |v: f64| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let linear = |c: f64| oklch_to_srgb(l, c, h).map(decode);
    // The edges of `in_srgb` in linear light.
    let edges = [decode(-GAMUT_SLACK), decode(1.0 + GAMUT_SLACK)];
    let samples = [0.0, 1.0, 2.0, 3.0].map(linear);
    let mut cuts = vec![0.0, top];
    for i in 0..3 {
        // The cubic's coefficients from its values at chroma 0, 1, 2 and 3.
        let [p0, p1, p2, p3] = samples.map(|s| s[i]);
        let d3 = (p3 - 3.0 * p2 + 3.0 * p1 - p0) / 6.0;
        let d2 = (p2 - 2.0 * p1 + p0) / 2.0 - 3.0 * d3;
        let d1 = p1 - p0 - d2 - d3;
        let cubic = |c: f64| ((d3 * c + d2) * c + d1) * c + p0;
        // The cubic is monotonic between its turning points, so it reaches each edge at most once
        // between two of them.
        let mut ends = vec![0.0, top];
        let (a, b) = (3.0 * d3, 2.0 * d2);
        if a != 0.0 {
            let disc = b * b - 4.0 * a * d1;
            if disc >= 0.0 {
                let q = disc.sqrt();
                ends.extend([(-b - q) / (2.0 * a), (-b + q) / (2.0 * a)]);
            }
        } else if b != 0.0 {
            ends.push(-d1 / b);
        }
        ends.retain(|x| (0.0..=top).contains(x));
        ends.sort_by(f64::total_cmp);
        cuts.extend_from_slice(&ends);
        for w in ends.windows(2) {
            for edge in edges {
                let (mut lo, mut hi) = (w[0], w[1]);
                let below = cubic(lo) < edge;
                if below == (cubic(hi) < edge) {
                    continue;
                }
                for _ in 0..60 {
                    let mid = (lo + hi) / 2.0;
                    if (cubic(mid) < edge) == below { lo = mid } else { hi = mid }
                }
                cuts.push((lo + hi) / 2.0);
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut tests = Vec::with_capacity(cuts.len() * 2);
    for w in cuts.windows(2) {
        tests.extend([w[0], (w[0] + w[1]) / 2.0]);
    }
    tests.push(top);
    // `top` does not fit, so the highest test that fits has a test above it that does not.
    for w in tests.windows(2).rev() {
        if fits(w[0]) {
            let (mut lo, mut hi) = (w[0], w[1]);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if fits(mid) { lo = mid } else { hi = mid }
            }
            return lo;
        }
    }
    0.0
}

/// The sRGB color (gamma encoded, 0..=1) of OKLCH `(l, c, h°)`, and whether its chroma was lowered:
/// a color outside sRGB keeps its lightness and hue and takes the largest chroma under `c` that fits.
/// Lightness is clamped to 0..=1 and chroma to 0 or more.
pub fn from_oklch(l: f64, c: f64, h: f64) -> ([f32; 3], bool) {
    let l = if l.is_nan() { 0.0 } else { l.clamp(0.0, 1.0) };
    let c = if c.is_nan() { 0.0 } else { c.max(0.0) };
    let h = if h.is_finite() { h } else { 0.0 };
    let exact = oklch_to_srgb(l, c, h);
    let (rgb, lowered) = if in_srgb(exact) { (exact, false) } else { (oklch_to_srgb(l, fitting_chroma(l, h, c), h), true) };
    (rgb.map(|v| v.clamp(0.0, 1.0) as f32), lowered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eight(rgb: [f32; 3]) -> [u8; 3] {
        rgb.map(|v| (v * 255.0).round() as u8)
    }

    #[test]
    fn red_has_the_oklch_of_the_oklab_matrices_and_converts_back() {
        let [l, c, h] = to_oklch([1.0, 0.0, 0.0]);
        assert!((l - 0.6280).abs() < 5e-5 && (c - 0.2577).abs() < 5e-5 && (h - 29.23).abs() < 5e-3, "{l} {c} {h}");
        let (rgb, lowered) = from_oklch(l, c, h);
        assert_eq!((eight(rgb), lowered), ([255, 0, 0], false));
    }

    /// CSS Color 4's examples convert to the hue its oklab() and oklch() forms give.
    #[test]
    fn the_polar_form_matches_the_css_examples() {
        for ([l, a, b], [cl, cc, ch]) in
            [([0.40101, 0.1147, 0.0453], [0.40101, 0.12332, 21.555]), ([0.65125, -0.0320, 0.1274], [0.65125, 0.13138, 104.097])]
        {
            let rgb = oklab_to_srgb(l, a, b).map(|v| v as f32);
            let [ol, oc, oh] = to_oklch(rgb);
            assert!((ol - cl).abs() < 1e-4 && (oc - cc).abs() < 1e-4 && (oh - ch).abs() < 0.05, "{ol} {oc} {oh}");
        }
    }

    #[test]
    fn the_216_web_colors_round_trip() {
        let steps = [0u8, 0x33, 0x66, 0x99, 0xcc, 0xff];
        for r in steps {
            for g in steps {
                for b in steps {
                    let rgb = [r, g, b].map(|v| f32::from(v) / 255.0);
                    let [l, c, h] = to_oklch(rgb);
                    assert_eq!(eight(from_oklch(l, c, h).0), [r, g, b], "{l} {c} {h}");
                }
            }
        }
    }

    #[test]
    fn grays_have_no_chroma_and_hue_zero() {
        for v in [0.0, 0.5, 1.0] {
            let [_, c, h] = to_oklch([v, v, v]);
            assert!(c <= POWERLESS_CHROMA && h == 0.0, "{c} {h}");
        }
    }

    /// A chroma past the gamut gives a color inside it with the same lightness and hue, and says so.
    #[test]
    fn a_chroma_past_the_gamut_is_lowered_keeping_lightness_and_hue() {
        let (l, h) = (0.7, 150.0);
        let max = max_chroma(l, h);
        assert!(max > 0.05 && max < CHROMA_FULL, "{max}");
        assert!(in_srgb(oklch_to_srgb(l, max, h)) && !in_srgb(oklch_to_srgb(l, max + 1e-3, h)));
        let (rgb, lowered) = from_oklch(l, CHROMA_FULL, h);
        assert!(lowered);
        let [ol, oc, oh] = to_oklch(rgb);
        assert!((ol - l).abs() < 2e-3 && (oh - h).abs() < 1.0 && (oc - max).abs() < 2e-3, "{ol} {oc} {oh}");
        assert!(!from_oklch(l, max * 0.5, h).1);
    }

    /// At blue's lightness and hue sRGB holds a run of low chroma and a run ending at the blue
    /// primary, with chromas outside sRGB between them: the largest chroma is the primary's.
    #[test]
    fn the_largest_chroma_of_blue_is_the_primarys() {
        let [l, c, h] = to_oklch([0.0, 0.0, 1.0]);
        assert!(!in_srgb(oklch_to_srgb(l, 0.29, h)), "the runs have a gap");
        let max = max_chroma(l, h);
        assert!(max >= 0.3132 && max >= c - 1e-6 && in_srgb(oklch_to_srgb(l, max, h)), "{max} {c}");
        assert_eq!(eight(from_oklch(l, CHROMA_FULL, h).0), [0, 0, 255]);
    }

    /// A dark blue whose runs are far apart: the largest chroma is the top of the upper run, and a
    /// chroma in the gap is lowered to the top of the run under it, keeping lightness and hue.
    #[test]
    fn a_chroma_between_two_runs_is_lowered_to_the_run_under_it() {
        let (l, h) = (0.14, 264.0);
        let max = max_chroma(l, h);
        assert!(max > 0.097 && in_srgb(oklch_to_srgb(l, max, h)) && !in_srgb(oklch_to_srgb(l, max + 1e-3, h)), "{max}");
        assert!(!in_srgb(oklch_to_srgb(l, 0.09, h)), "0.09 is in the gap");
        let (rgb, lowered) = from_oklch(l, 0.09, h);
        let [ol, oc, oh] = to_oklch(rgb);
        assert!(lowered && (ol - l).abs() < 1e-3 && (oh - h).abs() < 0.5 && (oc - 0.0825).abs() < 1e-3, "{ol} {oc} {oh}");
    }

    /// Every lightness and hue on a grid: the chroma found fits, and no chroma above it does.
    #[test]
    fn no_chroma_above_the_largest_fits() {
        for li in 1..50 {
            let l = f64::from(li) / 50.0;
            for hi in 0..120 {
                let h = f64::from(hi) * 3.0;
                let max = max_chroma(l, h);
                assert!(in_srgb(oklch_to_srgb(l, max, h)), "{l} {h} {max}");
                for step in 1..=200 {
                    let c = max + f64::from(step) * 0.0005;
                    assert!(!in_srgb(oklch_to_srgb(l, c, h)), "{l} {h}: {max} but {c} fits");
                }
            }
        }
    }

    #[test]
    fn black_white_and_bad_numbers_hold_no_chroma() {
        assert_eq!(max_chroma(0.0, 30.0), 0.0);
        assert_eq!(max_chroma(1.0, 30.0), 0.0);
        assert_eq!(max_chroma(f64::NAN, 30.0), 0.0);
        assert_eq!(max_chroma(0.5, f64::INFINITY), 0.0);
        assert_eq!(eight(from_oklch(f64::NAN, f64::NAN, f64::NAN).0), [0, 0, 0]);
        assert_eq!(eight(from_oklch(2.0, -1.0, 0.0).0), [255, 255, 255]);
    }
}
