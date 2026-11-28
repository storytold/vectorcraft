//! Contrast between two colors as WCAG 2 measures it (<https://www.w3.org/TR/WCAG22/#dfn-contrast-ratio>):
//! the relative luminance of display sRGB with the 0.04045 threshold, the ratio
//! (L1 + 0.05) / (L2 + 0.05) from 1 to 21, and the minimum ratios of success criteria 1.4.3
//! (Contrast (Minimum), AA), 1.4.6 (Contrast (Enhanced), AAA) and 1.4.11 (Non-text Contrast, AA).

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

    #[test]
    fn half_opaque_black_over_white_is_mid_gray() {
        assert_eq!(over([0.0; 3], 0.5, [1.0; 3]), [0.5; 3]);
        assert_eq!(over([0.2; 3], f32::NAN, [1.0; 3]), [0.2; 3]);
        assert_eq!(over([0.2; 3], 2.0, [1.0; 3]), [0.2; 3]);
    }
}
