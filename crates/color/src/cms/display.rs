//! The canvas's last step before the screen: the sRGB the canvas draws, converted into a display's
//! ICC profile (Preferences › Performance › Use Monitor Profile for Canvas).
//!
//! The display profile is a destination only. Document colors, proofs, exports and the web build
//! keep their sRGB output; a soft proof is applied first, then this conversion.
//!
//! [`DisplayTransform::from_icc`] accepts an RGB display-class (`mntr`) profile of at most
//! [`MAX_PROFILE_BYTES`] whose declared length fits the data and from which a transform can be built.
//! The conversion is relative colorimetric, sampled once into a [`LUT_N`]³ table, so drawing a frame
//! costs a table lookup per pixel and never builds a transform.

use std::hash::{Hash, Hasher};

use super::{Intent, ProfileKind, icc::IccProfile};

/// The largest display profile read: VectorCraft's own limit. Display profiles are a few kilobytes
/// (matrix/shaper) to a few hundred (table-based).
pub const MAX_PROFILE_BYTES: usize = 1 << 20;

/// Grid points per channel of the conversion table.
const LUT_N: usize = 33;

/// Why a display profile can't be used. The canvas then stays sRGB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// More bytes than [`MAX_PROFILE_BYTES`].
    TooLarge(usize),
    /// No ICC header (`acsp` signature) or a header cut short.
    NotIcc,
    /// The header's length is shorter than a header or longer than the data.
    BadLength { declared: usize, len: usize },
    /// The profile class isn't a display's (`mntr`): an input, output or other profile.
    NotDisplay(String),
    /// The profile's data color space isn't RGB.
    NotRgb(String),
    /// The profile doesn't parse.
    Unreadable(String),
    /// No conversion from sRGB into the profile can be built.
    NoTransform,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "the profile is {n} bytes, more than the {MAX_PROFILE_BYTES} a display profile may have"),
            Self::NotIcc => f.write_str("the file is not an ICC profile"),
            Self::BadLength { declared, len } => write!(f, "the profile's header gives a length of {declared} bytes for {len} bytes of data"),
            Self::NotDisplay(class) => write!(f, "the profile's class is `{class}`, not a display profile (`mntr`)"),
            Self::NotRgb(space) => write!(f, "the profile's color space is `{space}`, not RGB"),
            Self::Unreadable(e) => write!(f, "the profile can't be read: {e}"),
            Self::NoTransform => f.write_str("no conversion into the profile can be built"),
        }
    }
}

/// Four header bytes as text, for an error message.
fn signature(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_end().to_string()
}

/// sRGB → a display profile, as a table.
pub struct DisplayTransform {
    name: String,
    fingerprint: u64,
    /// `LUT_N`³ display values, red slowest; `None` when the profile shows sRGB unchanged (every
    /// grid point within half an 8-bit step of itself).
    table: Option<Vec<[f32; 3]>>,
}

impl std::fmt::Debug for DisplayTransform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayTransform").field("name", &self.name).field("identity", &self.is_identity()).finish()
    }
}

impl DisplayTransform {
    /// Checks `bytes` as a display profile, as [`Self::from_icc`] does, without building the
    /// conversion table.
    pub fn check(bytes: &[u8]) -> Result<(), ProfileError> {
        Self::parse(bytes).map(|_| ())
    }

    /// Checks `bytes` as a display profile and builds the conversion into it.
    pub fn from_icc(bytes: &[u8]) -> Result<Self, ProfileError> {
        let (profile, bytes) = Self::parse(bytes)?;
        let to_display =
            |rgb: [f32; 3]| profile.from_srgb(rgb, Intent::RelativeColorimetric).and_then(|v| Some([*v.first()?, *v.get(1)?, *v.get(2)?]));
        let s = (LUT_N - 1) as f32;
        let mut table = Vec::with_capacity(LUT_N.pow(3));
        for r in 0..LUT_N {
            for g in 0..LUT_N {
                for b in 0..LUT_N {
                    let rgb = [r as f32 / s, g as f32 / s, b as f32 / s];
                    table.push(to_display(rgb).ok_or(ProfileError::NoTransform)?);
                }
            }
        }
        let grid = |i: usize| [i / (LUT_N * LUT_N), i / LUT_N % LUT_N, i % LUT_N].map(|v| v as f32 / s);
        let identity = table.iter().enumerate().all(|(i, v)| v.iter().zip(grid(i)).all(|(a, b)| (a - b).abs() <= 0.5 / 255.0));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        Ok(Self { name: profile.name.clone(), fingerprint: h.finish(), table: (!identity).then_some(table) })
    }

    /// The parsed profile and its bytes up to its declared length, when `bytes` is an RGB display
    /// profile from which a conversion can be built.
    fn parse(bytes: &[u8]) -> Result<(IccProfile, &[u8]), ProfileError> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::TooLarge(bytes.len()));
        }
        if bytes.len() < 132 || bytes.get(36..40) != Some(b"acsp") {
            return Err(ProfileError::NotIcc);
        }
        let declared = bytes.get(0..4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
        if declared < 132 || declared > bytes.len() {
            return Err(ProfileError::BadLength { declared, len: bytes.len() });
        }
        let bytes = &bytes[..declared];
        if &bytes[12..16] != b"mntr" {
            return Err(ProfileError::NotDisplay(signature(&bytes[12..16])));
        }
        if &bytes[16..20] != b"RGB " {
            return Err(ProfileError::NotRgb(signature(&bytes[16..20])));
        }
        let profile = IccProfile::from_bytes(None, bytes).map_err(|e| ProfileError::Unreadable(e.to_string()))?;
        if profile.kind != ProfileKind::Rgb {
            return Err(ProfileError::NotRgb(format!("{:?}", profile.kind)));
        }
        let to_display =
            |rgb: [f32; 3]| profile.from_srgb(rgb, Intent::RelativeColorimetric).and_then(|v| Some([*v.first()?, *v.get(1)?, *v.get(2)?]));
        if to_display([0.5; 3]).is_none() {
            return Err(ProfileError::NoTransform);
        }
        Ok((profile, bytes))
    }

    /// The profile's description.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A hash of the profile's bytes, which tells two profiles with the same name apart.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// The profile shows sRGB unchanged, so nothing is converted.
    pub fn is_identity(&self) -> bool {
        self.table.is_none()
    }

    /// One sRGB color (0..1) in the display profile.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let Some(table) = &self.table else { return rgb.map(|v| v.clamp(0.0, 1.0)) };
        let n = LUT_N;
        let s = (n - 1) as f32;
        let p = rgb.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) * s });
        let i = p.map(|v| (v.floor() as usize).min(n - 2));
        let t = [p[0] - i[0] as f32, p[1] - i[1] as f32, p[2] - i[2] as f32];
        let mut out = [0.0f32; 3];
        for (dr, wr) in [(0, 1.0 - t[0]), (1, t[0])] {
            for (dg, wg) in [(0, 1.0 - t[1]), (1, t[1])] {
                for (db, wb) in [(0, 1.0 - t[2]), (1, t[2])] {
                    let w = wr * wg * wb;
                    if w == 0.0 {
                        continue;
                    }
                    let v = table[((i[0] + dr) * n + i[1] + dg) * n + i[2] + db];
                    for c in 0..3 {
                        out[c] += w * v[c];
                    }
                }
            }
        }
        out.map(|v| v.clamp(0.0, 1.0))
    }

    /// One 8-bit sRGB color in the display profile.
    pub fn apply8(&self, rgb: [u8; 3]) -> [u8; 3] {
        if self.table.is_none() {
            return rgb;
        }
        self.apply(rgb.map(|v| f32::from(v) / 255.0)).map(|v| (v * 255.0).round() as u8)
    }

    /// Premultiplied RGBA8 pixels, converted in place. Alpha is kept, and fully transparent
    /// pixels are left as they are.
    pub fn apply_premultiplied(&self, pixels: &mut [u8]) {
        if self.table.is_none() {
            return;
        }
        let mut last_in = [0u8; 4];
        let mut last_out = [0u8; 4];
        for px in pixels.as_chunks_mut::<4>().0 {
            let a = px[3];
            if a == 0 {
                continue;
            }
            if *px == last_in {
                *px = last_out;
                continue;
            }
            last_in = *px;
            let un = |c: u8| if a == 255 { c } else { ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 };
            let out = self.apply8([un(px[0]), un(px[1]), un(px[2])]);
            for (c, o) in px.iter_mut().zip(out) {
                *c = if a == 255 { o } else { ((u32::from(o) * u32::from(a) + 127) / 255) as u8 };
            }
            last_out = *px;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cms::{DISPLAY_P3, GENERIC_CMYK, SRGB, icc_bytes};

    /// `name`'s ICC file, marked as a display profile.
    fn display(name: &str) -> Vec<u8> {
        let mut b = icc_bytes(name).unwrap().to_vec();
        b[12..16].copy_from_slice(b"mntr");
        b
    }

    #[test]
    fn a_wide_gamut_display_shows_srgb_red_less_saturated_and_srgb_is_left_alone() {
        let p3 = DisplayTransform::from_icc(&display(DISPLAY_P3)).unwrap();
        assert!(!p3.is_identity());
        assert_eq!(p3.name(), DISPLAY_P3);
        // sRGB red is inside Display P3: its display values have green and blue above 0 and less red.
        let [r, g, b] = p3.apply8([255, 0, 0]);
        assert!(r < 245 && g > 40 && b > 10, "{:?}", [r, g, b]);
        // White and black stay where they are (relative colorimetric).
        assert_eq!(p3.apply8([255; 3]), [255; 3]);
        assert_eq!(p3.apply8([0; 3]), [0; 3]);
        let srgb = DisplayTransform::from_icc(&display(SRGB)).unwrap();
        assert!(srgb.is_identity());
        assert_eq!(srgb.apply8([255, 0, 0]), [255, 0, 0]);
        assert_ne!(srgb.fingerprint(), p3.fingerprint());
    }

    #[test]
    fn the_table_matches_the_profile_between_its_grid_points() {
        let bytes = display(DISPLAY_P3);
        let p3 = DisplayTransform::from_icc(&bytes).unwrap();
        let profile = IccProfile::from_bytes(None, &bytes).unwrap();
        for rgb in [[0.1, 0.7, 0.33], [0.9, 0.05, 0.6], [0.47, 0.47, 0.47], [0.0, 0.2, 1.0]] {
            let want = profile.from_srgb(rgb, Intent::RelativeColorimetric).unwrap();
            let got = p3.apply(rgb);
            for c in 0..3 {
                assert!((got[c] - want[c]).abs() < 1.0 / 255.0, "{rgb:?}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn premultiplied_pixels_keep_their_alpha_and_transparent_ones_stay_empty() {
        let p3 = DisplayTransform::from_icc(&display(DISPLAY_P3)).unwrap();
        let mut px = vec![255, 0, 0, 255, 128, 0, 0, 128, 0, 0, 0, 0, 255, 0, 0, 255];
        p3.apply_premultiplied(&mut px);
        let opaque = p3.apply8([255, 0, 0]);
        assert_eq!(&px[0..4], &[opaque[0], opaque[1], opaque[2], 255]);
        assert_eq!(&px[12..16], &px[0..4], "a repeated pixel converts the same");
        assert_eq!(px[7], 128);
        // Half-covered red: the same color at half coverage, within rounding.
        for c in 0..3 {
            assert!((i32::from(px[4 + c]) - i32::from(opaque[c]) / 2).abs() <= 1, "{:?}", &px[4..8]);
        }
        assert_eq!(&px[8..12], &[0, 0, 0, 0]);
    }

    #[test]
    fn profiles_that_are_not_rgb_display_profiles_are_refused() {
        let cmyk = icc_bytes(GENERIC_CMYK).unwrap().to_vec();
        assert!(matches!(DisplayTransform::from_icc(&cmyk), Err(ProfileError::NotDisplay(_))));
        let mut cmyk_display = cmyk.clone();
        cmyk_display[12..16].copy_from_slice(b"mntr");
        assert!(matches!(DisplayTransform::from_icc(&cmyk_display), Err(ProfileError::NotRgb(s)) if s == "CMYK"));
        let mut input = display(DISPLAY_P3);
        input[12..16].copy_from_slice(b"scnr");
        assert_eq!(DisplayTransform::from_icc(&input).unwrap_err(), ProfileError::NotDisplay("scnr".into()));
    }

    #[test]
    fn malformed_short_and_oversized_data_is_refused() {
        assert_eq!(DisplayTransform::from_icc(&[]).unwrap_err(), ProfileError::NotIcc);
        assert_eq!(DisplayTransform::from_icc(&[0; 132]).unwrap_err(), ProfileError::NotIcc);
        let good = display(DISPLAY_P3);
        assert_eq!(DisplayTransform::from_icc(&good[..100]).unwrap_err(), ProfileError::NotIcc);
        let mut long = good.clone();
        long[0..4].copy_from_slice(&(good.len() as u32 + 1).to_be_bytes());
        assert_eq!(DisplayTransform::from_icc(&long).unwrap_err(), ProfileError::BadLength { declared: good.len() + 1, len: good.len() });
        let mut short = good.clone();
        short[0..4].copy_from_slice(&64u32.to_be_bytes());
        assert!(matches!(DisplayTransform::from_icc(&short), Err(ProfileError::BadLength { declared: 64, .. })));
        // Trailing bytes past the declared length are not part of the profile.
        let mut padded = good.clone();
        padded.extend([0xAB; 16]);
        let a = DisplayTransform::from_icc(&padded).unwrap();
        assert_eq!(a.fingerprint(), DisplayTransform::from_icc(&good).unwrap().fingerprint());
        let mut garbage_tags = good.clone();
        for b in &mut garbage_tags[128..] {
            *b = 0xFF;
        }
        assert!(matches!(DisplayTransform::from_icc(&garbage_tags), Err(ProfileError::Unreadable(_) | ProfileError::NoTransform)));
        let huge = vec![0u8; MAX_PROFILE_BYTES + 1];
        assert_eq!(DisplayTransform::from_icc(&huge).unwrap_err(), ProfileError::TooLarge(MAX_PROFILE_BYTES + 1));
    }

    #[test]
    fn check_accepts_and_refuses_what_from_icc_does() {
        assert_eq!(DisplayTransform::check(&display(DISPLAY_P3)), Ok(()));
        assert_eq!(DisplayTransform::check(&display(SRGB)), Ok(()));
        let cmyk = icc_bytes(GENERIC_CMYK).unwrap().to_vec();
        assert_eq!(DisplayTransform::check(&cmyk), Err(DisplayTransform::from_icc(&cmyk).unwrap_err()));
        assert_eq!(DisplayTransform::check(&[1, 2, 3]), Err(ProfileError::NotIcc));
    }

    #[test]
    fn nan_and_out_of_range_values_stay_in_range() {
        let p3 = DisplayTransform::from_icc(&display(DISPLAY_P3)).unwrap();
        for v in p3.apply([f32::NAN, -1.0, 9.0]) {
            assert!((0.0..=1.0).contains(&v));
        }
    }
}
