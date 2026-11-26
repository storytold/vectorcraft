//! Contrast between two colors as WCAG 2 measures it (<https://www.w3.org/TR/WCAG22/#dfn-contrast-ratio>):
//! the relative luminance of display sRGB with the 0.04045 threshold, the ratio
//! (L1 + 0.05) / (L2 + 0.05) from 1 to 21, and the minimum ratios of success criteria 1.4.3
//! (Contrast (Minimum), AA), 1.4.6 (Contrast (Enhanced), AAA) and 1.4.11 (Non-text Contrast, AA).
//! [`closest_passing`] finds the color of the same hue nearest in lightness that meets a minimum.

/// What the content in front is, which sets the ratio it needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    /// Text below the large-scale size: AA 4.5:1, AAA 7:1.
    #[default]
    NormalText,
    /// Large-scale text (at least 18 pt, or 14 pt bold): AA 3:1, AAA 4.5:1.
    LargeText,
    /// User interface components and graphical objects: AA 3:1, no AAA level.
    Graphics,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::NormalText, Category::LargeText, Category::Graphics];

    /// The id commands and settings use.
    pub fn id(self) -> &'static str {
        match self {
            Category::NormalText => "normalText",
            Category::LargeText => "largeText",
            Category::Graphics => "graphics",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == s)
    }

    /// The English name shown in menus.
    pub fn label(self) -> &'static str {
        match self {
            Category::NormalText => "Normal Text",
            Category::LargeText => "Large Text",
            Category::Graphics => "Graphics",
        }
    }

    /// The minimum ratio for AA, and for AAA where the category has that level.
    pub fn minimums(self) -> (f64, Option<f64>) {
        match self {
            Category::NormalText => (4.5, Some(7.0)),
            Category::LargeText => (3.0, Some(4.5)),
            Category::Graphics => (3.0, None),
        }
    }

    /// Whether `ratio` meets AA, and AAA where the category has that level. The unrounded ratio
    /// is compared, so 4.499:1 fails 4.5:1.
    pub fn passes(self, ratio: f64) -> (bool, Option<bool>) {
        let (aa, aaa) = self.minimums();
        (ratio >= aa, aaa.map(|m| ratio >= m))
    }
}

/// The relative luminance of display sRGB `srgb` (each channel 0 to 1), from 0 for black to 1
/// for white.
pub fn relative_luminance(srgb: [f32; 3]) -> f64 {
    let lin = |v: f32| {
        let v = f64::from(v.clamp(0.0, 1.0));
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(srgb[0]) + 0.7152 * lin(srgb[1]) + 0.0722 * lin(srgb[2])
}

/// The contrast ratio of display sRGB colors `a` and `b`, from 1 to 21, whichever is lighter.
pub fn contrast_ratio(a: [f32; 3], b: [f32; 3]) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Display sRGB `fg` drawn at `opacity` (0 to 1) over the opaque `bg`, mixed per channel as the
/// canvas mixes a translucent color.
pub fn over(fg: [f32; 3], opacity: f32, bg: [f32; 3]) -> [f32; 3] {
    let a = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
    std::array::from_fn(|i| fg[i] * a + bg[i] * (1.0 - a))
}

/// `ratio` as text with two decimals, cut off rather than rounded, so a ratio just under a
/// minimum never reads as the minimum: 4.4963 is "4.49:1".
pub fn ratio_text(ratio: f64) -> String {
    let r = if ratio.is_finite() { ratio.clamp(1.0, 21.0) } else { 1.0 };
    format!("{:.2}:1", (r * 100.0).floor() / 100.0)
}

/// The lightnesses [`closest_passing`] tests on each side before it narrows down the first that
/// passes.
const LIGHTNESS_SAMPLES: usize = 16;
/// The halvings [`closest_passing`] makes of the lightness step, which leave it within 1e-9.
const LIGHTNESS_STEPS: usize = 32;
/// The golden-section steps [`closest_passing`] takes to find the highest ratio between samples,
/// which leave it within 1e-7 of the step.
const PEAK_STEPS: usize = 32;
/// Where between `lo` and `hi` `f` is highest, by golden-section search, which finds the top of a
/// single peak between them. `f` is called 34 times.
pub fn peak_between(mut lo: f64, mut hi: f64, f: &dyn Fn(f64) -> f64) -> f64 {
    let g = (5f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (hi - g * (hi - lo), lo + g * (hi - lo));
    let (mut fa, mut fb) = (f(a), f(b));
    for _ in 0..PEAK_STEPS {
        if fa < fb {
            (lo, a, fa) = (a, b, fb);
            b = lo + g * (hi - lo);
            fb = f(b);
        } else {
            (hi, b, fb) = (b, a, fa);
            a = hi - g * (hi - lo);
            fa = f(a);
        }
    }
    if fa > fb { a } else { b }
}

/// The display sRGB color closest to `srgb` whose `ratio` is at least `minimum`: the same OKLCH hue
/// (<https://bottosson.github.io/posts/oklab/>) with the lightness raised or lowered, whichever
/// takes the smaller step, and the chroma lowered where sRGB cannot show it, to the largest chroma
/// below it that fits ([`crate::oklab::from_oklch`]). Each side is tested at
/// 16 even steps up to white or black, and the step before the first that passes is narrowed down.
/// When no step passes, the highest ratio between the neighbors of the highest step is looked for,
/// so a color that passes only in a narrow band between white and black (as under a Difference
/// blend) is found too. `srgb` itself when it passes, and None when nothing passes. The work is
/// bounded: at most 167 colors are tested. The answer is the nearest passing color when the colors
/// that pass on each side form one stretch up to white or black, as for a plain contrast ratio.
/// When `ratio` lets colors pass in more than one stretch, a stretch that lies between two tested
/// steps before the first passing one is missed, and a farther color can be returned.
pub fn closest_passing(srgb: [f32; 3], minimum: f64, ratio: &dyn Fn([f32; 3]) -> f64) -> Option<[f32; 3]> {
    let passes = |c: [f32; 3]| ratio(c) >= minimum;
    let srgb = srgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    if passes(srgb) {
        return Some(srgb);
    }
    let [l, c, h] = crate::oklab::to_oklch(srgb);
    let mut best: Option<(f64, [f32; 3])> = None;
    for end in [1.0, 0.0] {
        let at = |t: f64| crate::oklab::from_oklch(l + (end - l) * t, c, h).0;
        let step = |k: usize| k as f64 / LIGHTNESS_SAMPLES as f64;
        let ratios: Vec<f64> = (1..=LIGHTNESS_SAMPLES).map(|k| ratio(at(step(k)))).collect();
        // `lo` fails and `hi` passes, the step before the first sample that passes.
        let found = match ratios.iter().position(|r| *r >= minimum) {
            Some(i) => Some((step(i), step(i + 1))),
            None => {
                // Every sample fails: look for a higher ratio around the highest one.
                let top = ratios.iter().enumerate().fold((0, f64::NEG_INFINITY), |m, (i, r)| if *r > m.1 { (i, *r) } else { m }).0;
                let (lo, hi) = (step(top), step((top + 2).min(LIGHTNESS_SAMPLES)));
                let peak = peak_between(lo, hi, &|t| ratio(at(t)));
                passes(at(peak)).then_some((lo, peak))
            }
        };
        let Some((mut lo, mut hi)) = found else { continue };
        for _ in 0..LIGHTNESS_STEPS {
            let mid = (lo + hi) / 2.0;
            if passes(at(mid)) { hi = mid } else { lo = mid }
        }
        let step = (end - l).abs() * hi;
        if best.is_none_or(|(d, _)| step < d) {
            best = Some((step, at(hi)));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> [f32; 3] {
        let v = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).map_or(0.0, |b| f32::from(b) / 255.0);
        [v(0), v(2), v(4)]
    }

    #[test]
    fn black_on_white_is_21_and_a_color_on_itself_is_1() {
        assert!((contrast_ratio(hex("000000"), hex("ffffff")) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(hex("ffffff"), hex("000000")) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(hex("355f78"), hex("355f78")) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ratios_near_a_minimum_are_compared_unrounded_and_cut_off_in_text() {
        let r = contrast_ratio(hex("fdc96e"), hex("355f78"));
        assert!((r - 4.4963).abs() < 1e-3, "{r}");
        assert_eq!(ratio_text(r), "4.49:1");
        assert_eq!(Category::NormalText.passes(r), (false, Some(false)));
        assert_eq!(Category::LargeText.passes(r), (true, Some(false)));
        let gray = contrast_ratio(hex("767676"), hex("ffffff"));
        assert!((gray - 4.542).abs() < 1e-3, "{gray}");
        assert_eq!(ratio_text(gray), "4.54:1");
        assert!(Category::NormalText.passes(gray).0);
        let lighter = contrast_ratio(hex("777777"), hex("ffffff"));
        assert!((lighter - 4.478).abs() < 1e-3, "{lighter}");
        assert_eq!(ratio_text(lighter), "4.47:1");
        assert!(!Category::NormalText.passes(lighter).0);
        assert_eq!(ratio_text(21.0), "21.00:1");
        assert_eq!(ratio_text(f64::NAN), "1.00:1");
    }

    #[test]
    fn graphics_have_no_aaa_level() {
        assert_eq!(Category::Graphics.passes(3.0), (true, None));
        assert_eq!(Category::Graphics.passes(2.99), (false, None));
        assert_eq!(Category::LargeText.passes(4.5), (true, Some(true)));
    }

    #[test]
    fn category_ids_parse_back() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.id()), Some(c));
        }
        assert_eq!(Category::parse("NormalText"), None);
    }

    fn hue(srgb: [f32; 3]) -> f64 {
        let [_, a, b] = crate::libraries::srgb_to_oklab(srgb);
        b.atan2(a).to_degrees()
    }

    #[test]
    fn the_closest_passing_color_keeps_the_hue_and_just_passes() {
        let (fg, bg) = (hex("fdc96e"), hex("355f78"));
        let fix = closest_passing(fg, 4.5, &|c| contrast_ratio(c, bg)).unwrap();
        let r = contrast_ratio(fix, bg);
        assert!((4.5..4.501).contains(&r), "{r}");
        assert!((hue(fix) - hue(fg)).abs() < 1.0, "{} {}", hue(fix), hue(fg));
        // A color lighter than the background gets lighter.
        assert!(relative_luminance(fix) > relative_luminance(fg));
    }

    #[test]
    fn the_closest_passing_gray_on_white_is_a_darker_gray() {
        let white = [1.0; 3];
        let fix = closest_passing(hex("777777"), 4.5, &|c| contrast_ratio(c, white)).unwrap();
        let r = contrast_ratio(fix, white);
        assert!((4.5..4.501).contains(&r), "{r}");
        assert!((fix[0] - fix[1]).abs() < 1e-4 && (fix[1] - fix[2]).abs() < 1e-4, "{fix:?}");
    }

    #[test]
    fn a_color_that_passes_only_between_white_and_black_is_found() {
        // White drawn with Difference over mid gray: white and black both draw mid gray (1:1), and
        // mid gray draws black, 5.3:1.
        let gray = [0.5; 3];
        let difference = |c: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| (c[i] - gray[i]).abs()) };
        let fix = closest_passing([1.0; 3], 4.5, &|c| contrast_ratio(difference(c), gray)).unwrap();
        let r = contrast_ratio(difference(fix), gray);
        assert!((4.5..4.51).contains(&r), "{fix:?}: {r}");
        assert!(fix[0] > 0.5, "the nearest side of mid gray from white: {fix:?}");
    }

    #[test]
    fn a_narrow_band_between_the_samples_is_found() {
        // Over #5A5A5A only colors within a few hundredths of it draw dark enough with Difference
        // for 3:1 (black reaches 3.04:1), and no even step from white lands there.
        let gray = hex("5a5a5a");
        let difference = |c: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| (c[i] - gray[i]).abs()) };
        let ratio = |c: [f32; 3]| contrast_ratio(difference(c), gray);
        let fix = closest_passing([1.0; 3], 3.0, &ratio).unwrap();
        let r = ratio(fix);
        assert!((3.0..3.01).contains(&r), "{fix:?}: {r}");
    }

    /// Just below blue's lightness the chromas sRGB holds at blue's hue come in two stretches, so a
    /// blue that needs a little more contrast keeps nearly all of its chroma.
    #[test]
    fn a_blue_that_darkens_a_little_keeps_its_chroma() {
        let gray = hex("b8b8b8");
        let ratio = |c: [f32; 3]| contrast_ratio(c, gray);
        let blue = [0.0, 0.0, 1.0];
        assert!(ratio(blue) < 4.5, "{}", ratio(blue));
        let fix = closest_passing(blue, 4.5, &ratio).unwrap();
        let ([_, c, h], [_, blue_c, blue_h]) = (crate::oklab::to_oklch(fix), crate::oklab::to_oklch(blue));
        assert!(ratio(fix) >= 4.5 && (h - blue_h).abs() < 1.0, "{fix:?}");
        assert!(c > blue_c - 0.02, "{fix:?}: chroma {c}, blue's {blue_c}");
    }

    #[test]
    fn a_passing_color_is_its_own_closest_and_nothing_passing_is_none() {
        let white = [1.0; 3];
        assert_eq!(closest_passing([0.0; 3], 7.0, &|c| contrast_ratio(c, white)), Some([0.0; 3]));
        assert_eq!(closest_passing(hex("808080"), 4.5, &|_| 1.0), None);
        // Mid gray: AAA for normal text needs 7:1, which neither white nor black reaches.
        let mid = hex("777777");
        assert_eq!(closest_passing(mid, 7.0, &|c| contrast_ratio(c, mid)), None);
        assert_eq!(closest_passing([f32::NAN; 3], 4.5, &|c| contrast_ratio(c, white)).map(|c| c.map(|v| v.is_finite())), Some([true; 3]));
    }

    #[test]
    fn half_opaque_black_over_white_is_mid_gray() {
        assert_eq!(over([0.0; 3], 0.5, [1.0; 3]), [0.5; 3]);
        assert_eq!(over([0.2; 3], f32::NAN, [1.0; 3]), [0.2; 3]);
        assert_eq!(over([0.2; 3], 2.0, [1.0; 3]), [0.2; 3]);
    }
}
