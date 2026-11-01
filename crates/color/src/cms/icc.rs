//! ICC profiles through `moxcms` (pure Rust): built-in RGB spaces and user-supplied `.icc` files,
//! and the profiles exports embed ([`encode_builtin`]), written in code from the CMS.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use moxcms::{
    ColorProfile, DataColorSpace, Layout, LocalizableString, LutMultidimensionalType, LutStore, LutWarehouse, Matrix3d, ProfileClass, ProfileText,
    RenderingIntent, ToneReprCurve, TransformF32Executor, TransformOptions, Vector3d,
};

use super::{CmsError, Intent, ProfileKind, lab, profiles_generation};

fn moxcms_intent(i: Intent) -> RenderingIntent {
    match i {
        Intent::Perceptual => RenderingIntent::Perceptual,
        Intent::RelativeColorimetric => RenderingIntent::RelativeColorimetric,
        Intent::Saturation => RenderingIntent::Saturation,
        Intent::AbsoluteColorimetric => RenderingIntent::AbsoluteColorimetric,
    }
}

const INTENTS: [Intent; 4] = [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation, Intent::AbsoluteColorimetric];

fn intent_index(i: Intent) -> usize {
    INTENTS.iter().position(|x| *x == i).unwrap_or(1)
}

type Xf = Option<Arc<TransformF32Executor>>;

/// A direct transform's cache key: destination [`IccProfile::id`], [`profiles_generation`] it was
/// built in, intent index, black-point compensation. The id tells a reloaded profile from the old
/// one of the same name, which settings built before the reload still convert into; the
/// generation only lets the cache drop what reloads left behind.
type DirectKey = (u64, u64, usize, bool);

/// A parsed ICC profile plus lazily-built transforms to/from sRGB.
pub struct IccProfile {
    pub name: String,
    /// Unique to this instance: never reused, unlike its name or address.
    id: u64,
    pub kind: ProfileKind,
    /// The file it was loaded from (empty for profiles made in code).
    source: Vec<u8>,
    profile: ColorProfile,
    srgb: ColorProfile,
    to_srgb: [OnceLock<Xf>; 4],
    from_srgb: [OnceLock<Xf>; 4],
    /// Direct transforms into other RGB profiles ([`Self::to_rgb_of`]), by destination, intent
    /// and black-point compensation.
    direct: Mutex<Vec<(DirectKey, Option<Arc<Direct>>)>>,
}

/// A transform from one profile into an RGB profile without passing through sRGB: into the
/// destination's primaries with linear tone curves, where black-point compensation is a linear
/// remap, then onto its own curves.
struct Direct {
    to_linear: Arc<TransformF32Executor>,
    /// Linear → the destination's curves; `None` when the destination isn't a matrix profile
    /// (the first transform goes straight into it, without compensation).
    to_dest: Option<Arc<TransformF32Executor>>,
    /// The source black's luminance in the linear destination (0: no compensation).
    black: f32,
}

impl std::fmt::Debug for IccProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IccProfile").field("name", &self.name).field("kind", &self.kind).finish()
    }
}

fn text(t: &Option<ProfileText>) -> Option<String> {
    match t.as_ref()? {
        ProfileText::PlainString(s) => Some(s.clone()),
        ProfileText::Localizable(v) => v.iter().find(|l| l.language == "en").or(v.first()).map(|l| l.value.clone()),
        ProfileText::Description(d) => Some(if d.ascii_string.is_empty() { d.unicode_string.clone() } else { d.ascii_string.clone() }),
    }
    .map(|s| s.trim_matches(char::from(0)).trim().to_string())
    .filter(|s| !s.is_empty())
}

impl IccProfile {
    pub fn from_profile(name: Option<String>, profile: ColorProfile) -> Result<Self, CmsError> {
        let kind = match profile.color_space {
            DataColorSpace::Rgb => ProfileKind::Rgb,
            DataColorSpace::Cmyk => ProfileKind::Cmyk,
            DataColorSpace::Gray => ProfileKind::Gray,
            other => return Err(CmsError::Unsupported(format!("profile colour space {other:?} is not RGB, CMYK or Gray"))),
        };
        let name = name.or_else(|| text(&profile.description)).unwrap_or_else(|| format!("Untitled {kind:?} profile"));
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Ok(Self {
            name,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            kind,
            source: Vec::new(),
            profile,
            srgb: ColorProfile::new_srgb(),
            to_srgb: Default::default(),
            from_srgb: Default::default(),
            direct: Mutex::new(Vec::new()),
        })
    }

    pub fn from_bytes(name: Option<String>, bytes: &[u8]) -> Result<Self, CmsError> {
        if !localized_text_fits(bytes) {
            return Err(CmsError::BadProfile(format!("its localized names take more than {} KB", MAX_LOCALIZED_TEXT >> 10)));
        }
        let p = ColorProfile::new_from_slice(bytes).map_err(|e| CmsError::BadProfile(format!("{e:?}")))?;
        Ok(Self { source: bytes.to_vec(), ..Self::from_profile(name, p)? })
    }

    /// The profile as an ICC file: the bytes it was loaded from, else encoded.
    pub fn bytes(&self) -> Result<Vec<u8>, CmsError> {
        if self.source.is_empty() { encode(&self.profile) } else { Ok(self.source.clone()) }
    }

    fn layout(&self) -> Layout {
        match self.kind {
            ProfileKind::Cmyk => Layout::Rgba, // 4 channels
            ProfileKind::Gray => Layout::Gray,
            ProfileKind::Rgb => Layout::Rgb,
        }
    }

    fn channels(&self) -> usize {
        match self.kind {
            ProfileKind::Cmyk => 4,
            ProfileKind::Gray => 1,
            ProfileKind::Rgb => 3,
        }
    }

    fn opts(intent: Intent) -> TransformOptions {
        TransformOptions { rendering_intent: moxcms_intent(intent), ..Default::default() }
    }

    fn to_xf(&self, intent: Intent) -> Xf {
        self.to_srgb[intent_index(intent)]
            .get_or_init(|| self.profile.create_transform_f32(self.layout(), &self.srgb, Layout::Rgb, Self::opts(intent)).ok())
            .clone()
    }

    fn inverse_xf(&self, intent: Intent) -> Xf {
        self.from_srgb[intent_index(intent)]
            .get_or_init(|| self.srgb.create_transform_f32(Layout::Rgb, &self.profile, self.layout(), Self::opts(intent)).ok())
            .clone()
    }

    /// Device values (RGB / CMYK / Gray, 0..1) → sRGB.
    pub fn to_srgb(&self, v: &[f32], intent: Intent) -> Option<[f32; 3]> {
        let mut src = [0.0f32; 4];
        let channels = self.channels();
        src.get_mut(..channels)?.iter_mut().zip(v.iter()).for_each(|(dst, value)| *dst = *value);
        self.to_srgb_into(src.get(..channels)?, intent)
    }

    /// Convert an exact-width device pixel using the cached transform and stack storage.
    pub fn to_srgb_into(&self, v: &[f32], intent: Intent) -> Option<[f32; 3]> {
        if v.len() != self.channels() {
            return None;
        }
        let xf = self.to_xf(intent)?;
        let mut dst = [0.0f32; 3];
        xf.transform(v, &mut dst).ok()?;
        Some(dst.map(|x| x.clamp(0.0, 1.0)))
    }

    /// sRGB → device values.
    pub fn from_srgb(&self, rgb: [f32; 3], intent: Intent) -> Option<Vec<f32>> {
        let xf = self.inverse_xf(intent)?;
        let mut dst = vec![0.0f32; self.channels()];
        xf.transform(&rgb, &mut dst).ok()?;
        Some(dst.into_iter().map(|x| x.clamp(0.0, 1.0)).collect())
    }

    /// Device values (RGB / CMYK / Gray, 0..1) → `dest`'s RGB, converted directly with `intent`:
    /// not through sRGB, so colours outside sRGB keep their place in a wider space. With `bpc`
    /// (and an intent other than absolute), this profile's black maps onto the destination's
    /// black (black-point compensation, the darkest ink the profile prints becoming RGB 0 rather
    /// than a dark grey). `None` when `dest` isn't RGB or no transform can be built.
    pub fn to_rgb_of(&self, dest: &IccProfile, v: &[f32], intent: Intent, bpc: bool) -> Option<[f32; 3]> {
        let t = self.direct_xf(dest, intent, bpc && intent != Intent::AbsoluteColorimetric)?;
        let mut src = v.to_vec();
        src.resize(self.channels(), 0.0);
        let mut lin = [0.0f32; 3];
        t.to_linear.transform(&src, &mut lin).ok()?;
        let Some(to_dest) = &t.to_dest else { return Some(lin.map(|x| x.clamp(0.0, 1.0))) };
        if t.black > 0.0 {
            lin = lin.map(|x| (x - t.black) / (1.0 - t.black));
        }
        let lin = lin.map(|x| x.clamp(0.0, 1.0));
        let mut out = [0.0f32; 3];
        to_dest.transform(&lin, &mut out).ok()?;
        Some(out.map(|x| x.clamp(0.0, 1.0)))
    }

    fn direct_xf(&self, dest: &IccProfile, intent: Intent, bpc: bool) -> Option<Arc<Direct>> {
        if dest.kind != ProfileKind::Rgb {
            return None;
        }
        let generation = profiles_generation();
        let key = (dest.id, generation, intent_index(intent), bpc);
        let mut cache = self.direct.lock().unwrap_or_else(|e| e.into_inner());
        // an older generation's entries can't match again: drop them so reloads don't pile up
        cache.retain(|(k, _)| k.1 == generation);
        if let Some((_, t)) = cache.iter().find(|(k, _)| *k == key) {
            return t.clone();
        }
        let t = self.build_direct(dest, intent, bpc).map(Arc::new);
        cache.push((key, t.clone()));
        t
    }

    fn build_direct(&self, dest: &IccProfile, intent: Intent, bpc: bool) -> Option<Direct> {
        let d = &dest.profile;
        let matrix = d.red_trc.is_some() && d.green_trc.is_some() && d.blue_trc.is_some();
        if !matrix {
            let xf = self.profile.create_transform_f32(self.layout(), d, Layout::Rgb, Self::opts(intent)).ok()?;
            return Some(Direct { to_linear: xf, to_dest: None, black: 0.0 });
        }
        let mut linear = d.clone();
        for trc in [&mut linear.red_trc, &mut linear.green_trc, &mut linear.blue_trc] {
            *trc = Some(ToneReprCurve::Parametric(vec![1.0]));
        }
        // A coding-independent code point names the transfer curve outright (moxcms's sRGB has
        // one) and would override the linear curves.
        linear.cicp = None;
        let to_linear = self.profile.create_transform_f32(self.layout(), &linear, Layout::Rgb, Self::opts(intent)).ok()?;
        let to_dest = linear.create_transform_f32(Layout::Rgb, d, Layout::Rgb, Self::opts(Intent::RelativeColorimetric)).ok()?;
        // The source black: the darkest colour the profile reproduces, found as lcms does for an
        // output profile (black sent into the profile and read back), made neutral.
        let black = if bpc && self.kind == ProfileKind::Cmyk {
            let inks = self.from_srgb([0.0; 3], Intent::RelativeColorimetric)?;
            let mut lin = [0.0f32; 3];
            to_linear.transform(&inks, &mut lin).ok()?;
            let y = |c: &moxcms::Xyzd| c.y as f32;
            let lum = lin[0] * y(&d.red_colorant) + lin[1] * y(&d.green_colorant) + lin[2] * y(&d.blue_colorant);
            lum.clamp(0.0, 0.5)
        } else {
            0.0
        };
        Some(Direct { to_linear, to_dest: Some(to_dest), black })
    }

    /// Whether transforms can be built for this profile (checked at registration).
    pub fn usable(&self) -> bool {
        self.to_xf(Intent::RelativeColorimetric).is_some() && self.inverse_xf(Intent::RelativeColorimetric).is_some()
    }
}

/// The primaries and tone curves of built-in RGB space `name`.
fn rgb_space(name: &str) -> Option<ColorProfile> {
    Some(match name {
        super::WIDE_GAMUT_RGB => ColorProfile::new_adobe_rgb(),
        super::DISPLAY_P3 => ColorProfile::new_display_p3(),
        super::PROPHOTO_RGB => ColorProfile::new_pro_photo_rgb(),
        super::SRGB => ColorProfile::new_srgb(),
        _ => return None,
    })
}

/// Built-in RGB working spaces from `moxcms`'s standard primaries.
pub fn builtin_rgb(name: &str) -> Option<IccProfile> {
    IccProfile::from_profile(Some(name.to_string()), rgb_space(name)?).ok()
}

/// Built-in RGB space `name` as an ICC file, under its name here.
pub(super) fn builtin_rgb_bytes(name: &str) -> Result<Vec<u8>, CmsError> {
    encode(&labelled(rgb_space(name).ok_or_else(|| CmsError::UnknownProfile(name.into()))?, name))
}

/// Encode `p` as an ICC file. The creation date is fixed so exports are reproducible.
pub(super) fn encode(p: &ColorProfile) -> Result<Vec<u8>, CmsError> {
    let mut bytes = aligned(p.encode().map_err(|e| CmsError::BadProfile(format!("{e:?}")))?);
    // Header bytes 24..36: year, month, day, hours, minutes, seconds (u16 each).
    if let Some(date) = bytes.get_mut(24..36) {
        date.copy_from_slice(&[2026u16, 1, 1, 0, 0, 0].map(u16::to_be_bytes).concat());
    }
    Ok(bytes)
}

/// `icc` with every tagged element starting on a 4-byte boundary, padded with zeros, as ICC.1
/// §7.1.2 requires (moxcms packs them, so a 62-byte `desc` left the next tag unaligned, #1584).
/// Tags that share data keep sharing it. A profile whose tag table doesn't read is returned as it is.
fn aligned(icc: Vec<u8>) -> Vec<u8> {
    const TABLE: usize = 128;
    let be = |b: &[u8], at: usize| b.get(at..at + 4).map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]) as usize);
    let Some(n) = be(&icc, TABLE).filter(|n| *n <= 1000) else { return icc };
    let data_start = TABLE + 4 + 12 * n;
    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let e = TABLE + 4 + 12 * i;
        let (Some(off), Some(size)) = (be(&icc, e + 4), be(&icc, e + 8)) else { return icc };
        if off < data_start || off.checked_add(size).is_none_or(|end| end > icc.len()) {
            return icc;
        }
        entries.push((off, size));
    }
    if entries.iter().all(|(off, _)| off.is_multiple_of(4)) && icc.len().is_multiple_of(4) {
        return icc;
    }
    let mut out = icc.get(..data_start).map(<[u8]>::to_vec).unwrap_or_default();
    let mut moved: Vec<((usize, usize), usize)> = Vec::new();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| entries[i].0);
    for i in order {
        let (off, size) = entries[i];
        let at = match moved.iter().find(|(k, _)| *k == (off, size)) {
            Some((_, at)) => *at,
            None => {
                out.resize(out.len().next_multiple_of(4), 0);
                let at = out.len();
                out.extend_from_slice(&icc[off..off + size]);
                moved.push(((off, size), at));
                at
            }
        };
        let e = TABLE + 4 + 12 * i + 4;
        out[e..e + 4].copy_from_slice(&(at as u32).to_be_bytes());
    }
    out.resize(out.len().next_multiple_of(4), 0);
    let len = out.len() as u32;
    out[0..4].copy_from_slice(&len.to_be_bytes());
    // The profile ID (an MD5 of the old bytes) no longer matches: zero is "not computed".
    if let Some(id) = out.get_mut(84..100) {
        id.fill(0);
    }
    out
}

fn text_tag(s: &str) -> Option<ProfileText> {
    Some(ProfileText::Localizable(vec![LocalizableString::new("en".into(), "US".into(), s.into())]))
}

/// Profiles made here carry their name and no copyright claim.
fn labelled(mut p: ColorProfile, name: &str) -> ColorProfile {
    p.description = text_tag(name);
    p.copyright = text_tag("No copyright, use freely");
    p
}

/// Grid points per input channel of the CMYK → Lab table and of the Lab → CMYK table.
const A2B_GRID: usize = 9;
const B2A_GRID: usize = 17;
/// Grid points per Lab channel of the gamut table.
const GAMUT_GRID: usize = 9;

/// A CMYK output profile sampled from a conversion: `to_lab` (CMYK → media-relative Lab) in a
/// 9⁴ table, `from_lab` (Lab → CMYK, gamut-mapped) in a 17³ table, Lab PCS (ICC v4 encoding).
/// It carries every tag ICC.1 requires of an output profile: the tables for all three intents
/// and the gamut tag (Windows rejects a CMYK profile without them, so Photos couldn't show a CMYK
/// TIFF that embedded one, #2707).
pub(super) fn cmyk_profile(name: &str, to_lab: impl Fn([f32; 4]) -> lab::Lab, from_lab: impl Fn(lab::Lab) -> [f32; 4]) -> ColorProfile {
    let unit = |i: usize, n: usize| i as f32 / (n - 1) as f32;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    let mut a2b = Vec::with_capacity(A2B_GRID.pow(4) * 3);
    for i in 0..A2B_GRID.pow(4) {
        let cmyk = [3, 2, 1, 0].map(|d| unit(i / A2B_GRID.pow(d) % A2B_GRID, A2B_GRID));
        let l = to_lab(cmyk);
        a2b.extend([l.l / 100.0, (l.a + 128.0) / 255.0, (l.b + 128.0) / 255.0].map(q));
    }
    let mut b2a = Vec::with_capacity(B2A_GRID.pow(3) * 4);
    for i in 0..B2A_GRID.pow(3) {
        let [l, a, b] = [2, 1, 0].map(|d| unit(i / B2A_GRID.pow(d) % B2A_GRID, B2A_GRID));
        b2a.extend(from_lab(lab::Lab::new(l * 100.0, a * 255.0 - 128.0, b * 255.0 - 128.0)).map(q));
    }
    // Gamut: 0 where a colour separates and comes back within a ΔE of 3, else out of gamut.
    let mut gamut = Vec::with_capacity(GAMUT_GRID.pow(3));
    for i in 0..GAMUT_GRID.pow(3) {
        let [l, a, b] = [2, 1, 0].map(|d| unit(i / GAMUT_GRID.pow(d) % GAMUT_GRID, GAMUT_GRID));
        let want = lab::Lab::new(l * 100.0, a * 255.0 - 128.0, b * 255.0 - 128.0);
        gamut.push(if lab::delta_e2000(want, to_lab(from_lab(want))) <= 3.0 { 0 } else { u16::MAX });
    }
    let identity = |n: usize| vec![ToneReprCurve::Parametric(vec![1.0]); n];
    let table = |inputs: usize, outputs: usize, grid: usize, clut: Vec<u16>| {
        let mut grid_points = [0u8; 16];
        grid_points[..inputs].fill(grid as u8);
        LutWarehouse::Multidimensional(LutMultidimensionalType {
            num_input_channels: inputs as u8,
            num_output_channels: outputs as u8,
            grid_points,
            clut: Some(LutStore::Store16(clut)),
            // A curves sit on the device side (the gamut tag's is its one output), B curves on the
            // PCS side.
            a_curves: identity(inputs + outputs - 3),
            b_curves: identity(3),
            m_curves: vec![],
            matrix: Matrix3d::IDENTITY,
            bias: Vector3d::default(),
        })
    };
    let a2b = table(4, 3, A2B_GRID, a2b);
    let b2a = table(3, 4, B2A_GRID, b2a);
    let gamut = table(3, 1, GAMUT_GRID, gamut);
    let mut p = ColorProfile::default();
    p.profile_class = ProfileClass::OutputDevice;
    p.color_space = DataColorSpace::Cmyk;
    p.pcs = DataColorSpace::Lab;
    p.rendering_intent = RenderingIntent::RelativeColorimetric;
    p.white_point = moxcms::WHITE_POINT_D50.to_xyzd();
    p.media_white_point = Some(p.white_point);
    p.lut_a_to_b_perceptual = Some(a2b.clone());
    p.lut_a_to_b_saturation = Some(a2b.clone());
    p.lut_a_to_b_colorimetric = Some(a2b);
    p.lut_b_to_a_perceptual = Some(b2a.clone());
    p.lut_b_to_a_saturation = Some(b2a.clone());
    p.lut_b_to_a_colorimetric = Some(b2a);
    p.gamut = Some(gamut);
    labelled(p, name)
}

/// The grey space of greyscale exports: sRGB's tone curve, D50 white.
pub(super) fn gray_profile(name: &str) -> ColorProfile {
    let mut p = ColorProfile::new_gray_with_gamma(2.2);
    p.gray_trc = ColorProfile::new_srgb().red_trc;
    labelled(p, name)
}

/// Most bytes of UTF-16 the localized names (`mluc` tags) of a profile may take together.
const MAX_LOCALIZED_TEXT: usize = 1 << 20;

/// Whether the localized-name records of the `mluc` tags in ICC `bytes` take at most
/// [`MAX_LOCALIZED_TEXT`] together. moxcms copies each record into a string of its own, and records
/// may all point at the same bytes, so a profile under a megabyte could make it allocate
/// gigabytes. A table it can't read is left to moxcms to refuse.
pub(crate) fn localized_text_fits(bytes: &[u8]) -> bool {
    let be32 = |b: &[u8], at: usize| b.get(at..at.checked_add(4)?).map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]) as usize);
    let Some(tags) = be32(bytes, 128) else { return true };
    let mut total = 0usize;
    for i in 0..tags {
        let Some(entry) = i.checked_mul(12).and_then(|e| e.checked_add(132)) else { return false };
        let (Some(offset), Some(size)) = (be32(bytes, entry + 4), be32(bytes, entry + 8)) else { break };
        let Some(tag) = offset.checked_add(size).and_then(|end| bytes.get(offset..end)) else { continue };
        if !tag.starts_with(b"mluc") {
            continue;
        }
        let records = be32(tag, 8).unwrap_or(0);
        for r in 0..records {
            // Each record's header: language, country, length, offset.
            let Some(length) = r.checked_mul(12).and_then(|h| be32(tag, h + 16 + 4)) else { break };
            // Each record counts its 12-byte header too: records of length 0 are not free, so
            // many tags pointing at one `mluc` can't make this loop quadratic.
            total = total.saturating_add(length).saturating_add(12);
            if total > MAX_LOCALIZED_TEXT {
                return false;
            }
        }
    }
    true
}
