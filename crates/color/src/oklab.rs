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

/// The largest chroma sRGB holds at lightness `l` and hue `h°`: 0 at or outside black and white.
pub fn max_chroma(l: f64, h: f64) -> f64 {
    largest_fitting_chroma(l, h, f64::INFINITY)
}

/// The primary and secondary colors of sRGB: the corners of its cube next to black.
const PRIMARIES: [[f32; 3]; 6] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 0.0]];

/// The largest chroma no more than `limit` that sRGB holds at lightness `l` and hue `h°`: 0 at or
/// outside black and white.
fn largest_fitting_chroma(l: f64, h: f64, limit: f64) -> f64 {
    if !(l > 0.0 && l < 1.0) || !h.is_finite() {
        return 0.0;
    }
    let fits = |c: f64| in_srgb(oklch_to_srgb(l, c, h));
    // The edge of the chromas from 0 that sRGB holds without a break.
    let (mut lo, mut hi) = (0.0, 0.5);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) { lo = mid } else { hi = mid }
    }
    // Past that edge the ray can come back into sRGB near a line from black to a primary or
    // secondary color. Such a line is straight in Oklab, so it keeps that color's hue and its
    // chroma grows with its lightness. At the lightness and hue of blue, for example, the ray leaves
    // sRGB at a chroma of about 0.266 and meets blue itself at about 0.313; at L 0.44 and a hue
    // 0.01° past blue's, a second stretch runs from about 0.3042 to 0.3048. Each such stretch ends
    // at or just below the line's chroma, so the search walks down from there in small steps to the
    // first chroma that fits and then finds the stretch's end between it and the step above.
    let past = PRIMARIES.iter().filter_map(|rgb| {
        let [pl, a, b] = srgb_to_oklab(*rgb);
        let edge = l / pl * a.hypot(b);
        let off = (h - b.atan2(a).to_degrees()).rem_euclid(360.0);
        if l > pl || edge <= lo || off.min(360.0 - off) > NEAR_PRIMARY_HUE {
            return None;
        }
        let start = edge.min(limit);
        let mut c = start;
        while c > lo && start - c <= PAST_WALK {
            if fits(c) {
                let (mut inside, mut outside) = (c, (c + PAST_STEP).min(start));
                if outside > inside && !fits(outside) {
                    for _ in 0..20 {
                        let mid = (inside + outside) / 2.0;
                        if fits(mid) { inside = mid } else { outside = mid }
                    }
                } else {
                    inside = outside;
                }
                return Some(inside);
            }
            c -= PAST_STEP;
        }
        None
    });
    past.fold(lo.min(limit), f64::max)
}

/// How near a primary or secondary color's hue, in degrees, the ray must pass for
/// [`largest_fitting_chroma`] to look past the first edge.
const NEAR_PRIMARY_HUE: f64 = 2.0;
/// The step of that walk.
const PAST_STEP: f64 = 2e-4;
/// How far below the line's chroma that walk goes.
const PAST_WALK: f64 = 0.06;

/// The sRGB color (gamma encoded, 0..=1) of OKLCH `(l, c, h°)`, and whether its chroma was lowered:
/// a color outside sRGB keeps its lightness and hue and takes the largest chroma below it that
/// fits. Lightness is clamped to 0..=1 and chroma to 0 or more.
pub fn from_oklch(l: f64, c: f64, h: f64) -> ([f32; 3], bool) {
    let l = if l.is_nan() { 0.0 } else { l.clamp(0.0, 1.0) };
    let c = if c.is_nan() { 0.0 } else { c.max(0.0) };
    let h = if h.is_finite() { h } else { 0.0 };
    let exact = oklch_to_srgb(l, c, h);
    let (rgb, lowered) = if in_srgb(exact) { (exact, false) } else { (oklch_to_srgb(l, largest_fitting_chroma(l, h, c), h), true) };
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

    /// At the lightness and hue of blue the ray leaves sRGB and comes back at blue itself, so the
    /// largest chroma is blue's, and a chroma past it gives blue.
    #[test]
    fn the_largest_chroma_at_blues_hue_is_blues() {
        let [l, c, h] = to_oklch([0.0, 0.0, 1.0]);
        let max = max_chroma(l, h);
        assert!((max - c).abs() < 1e-4 && in_srgb(oklch_to_srgb(l, max, h)), "{max} {c}");
        let (rgb, lowered) = from_oklch(l, CHROMA_FULL, h);
        assert_eq!((eight(rgb), lowered), ([0, 0, 255], true));
        // A chroma between the two stretches takes the end of the first one, inside sRGB.
        let (rgb, lowered) = from_oklch(l, 0.29, h);
        let [_, gc, _] = to_oklch(rgb);
        assert!(lowered && gc < 0.27 && in_srgb(oklch_to_srgb(l, gc, h)), "{gc}");
        // A hue just past blue's: a narrow second stretch ends just below the line from black to blue.
        let (l, h) = (0.44, 264.06);
        let max = max_chroma(l, h);
        assert!(max > 0.304 && max < 0.305 && in_srgb(oklch_to_srgb(l, max, h)), "{max}");
        let [_, got, _] = to_oklch(from_oklch(l, 0.35, h).0);
        assert!(got > 0.303, "{got}");
        // A darker blue on the line from black to blue.
        let [l, c, h] = to_oklch([0.0, 0.0, 0.25]);
        assert!((max_chroma(l, h) - c).abs() < 1e-4, "{} {c}", max_chroma(l, h));
    }

    /// sRGB channels below 0 go through the transfer curve with its sign turned, as CSS Color 4's
    /// extended sRGB does, in both directions.
    #[test]
    fn negative_channels_use_the_signed_transfer_curve() {
        let linear = -((0.5_f64 + 0.055) / 1.055).powf(2.4);
        let [l, c, _] = to_oklch([-0.5; 3]);
        assert!((l - linear.cbrt()).abs() < 1e-4 && c <= POWERLESS_CHROMA, "{l} {c}");
        for v in oklch_to_srgb(linear.cbrt(), 0.0, 0.0) {
            assert!((v + 0.5).abs() < 1e-4, "{v}");
        }
        // Small values near 0 keep the straight part of the curve.
        let [l, _, _] = to_oklch([-0.01; 3]);
        assert!((l - (-0.01_f64 / 12.92).cbrt()).abs() < 1e-4, "{l}");
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
