//! ICC profiles through `craft-cms` (pure Rust): built-in RGB spaces and user-supplied `.icc`
//! files, and the profiles exports embed ([`builtin_rgb_bytes`], [`cmyk_profile`], [`gray_profile`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use craft_cms::{
    Builtin, Clut, ColorSpace, Curve, Intent as CmsIntent, Pcs, Profile, ProfileClass, Transform, builtin,
    pipeline::Stage,
    profile::{Lut, LutKind},
};

use super::{CmsError, Intent, ProfileKind, lab, profiles_generation};

fn cms_intent(i: Intent) -> CmsIntent {
    match i {
        Intent::Perceptual => CmsIntent::Perceptual,
        Intent::RelativeColorimetric => CmsIntent::RelativeColorimetric,
        Intent::Saturation => CmsIntent::Saturation,
        Intent::AbsoluteColorimetric => CmsIntent::AbsoluteColorimetric,
    }
}

const INTENTS: [Intent; 4] = [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation, Intent::AbsoluteColorimetric];

fn intent_index(i: Intent) -> usize {
    INTENTS.iter().position(|x| *x == i).unwrap_or(1)
}

type Xf = Option<Arc<Transform>>;

/// A direct transform's cache key: destination [`IccProfile::id`], [`profiles_generation`] it was
/// built in, intent index, black-point compensation.
type DirectKey = (u64, u64, usize, bool);

/// A parsed ICC profile plus lazily-built transforms to/from sRGB.
pub struct IccProfile {
    pub name: String,
    /// Unique to this instance: never reused, unlike its name or address.
    id: u64,
    pub kind: ProfileKind,
    /// The file it was loaded from (empty for profiles made in code).
    source: Vec<u8>,
    profile: Profile,
    srgb: Profile,
    to_srgb: [OnceLock<Xf>; 4],
    from_srgb: [OnceLock<Xf>; 4],
    /// Direct transforms into other profiles ([`Self::to_rgb_of`], [`Self::to_same_model_of`]),
    /// by destination, intent and black-point compensation.
    direct: Mutex<Vec<(DirectKey, Xf)>>,
}

impl std::fmt::Debug for IccProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IccProfile").field("name", &self.name).field("kind", &self.kind).finish()
    }
}

impl IccProfile {
    pub fn from_profile(name: Option<String>, profile: Profile) -> Result<Self, CmsError> {
        let kind = match profile.color_space {
            ColorSpace::Rgb => ProfileKind::Rgb,
            ColorSpace::Cmyk => ProfileKind::Cmyk,
            ColorSpace::Gray => ProfileKind::Gray,
            other => return Err(CmsError::Unsupported(format!("profile colour space {other:?} is not RGB, CMYK or Gray"))),
        };
        let desc = profile.description.trim_matches(char::from(0)).trim();
        let name = name.or_else(|| (!desc.is_empty()).then(|| desc.to_string())).unwrap_or_else(|| format!("Untitled {kind:?} profile"));
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Ok(Self {
            name,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            kind,
            source: Vec::new(),
            profile,
            srgb: builtin::srgb().clone(),
            to_srgb: Default::default(),
            from_srgb: Default::default(),
            direct: Mutex::new(Vec::new()),
        })
    }

    pub fn from_bytes(name: Option<String>, bytes: &[u8]) -> Result<Self, CmsError> {
        if !localized_text_fits(bytes) {
            return Err(CmsError::BadProfile(format!("its localized names take more than {} KB", MAX_LOCALIZED_TEXT >> 10)));
        }
        if let Some(tag) = repeated_tag(bytes) {
            return Err(CmsError::BadProfile(format!(
                "its tag table lists the `{}` tag more than once",
                String::from_utf8_lossy(&tag.to_be_bytes())
            )));
        }
        let p = Profile::parse(bytes).map_err(|e| CmsError::BadProfile(format!("{e:?}")))?;
        Ok(Self { source: bytes.to_vec(), ..Self::from_profile(name, p)? })
    }

    /// The profile as an ICC file: the bytes it was loaded from, else encoded.
    pub fn bytes(&self) -> Result<Vec<u8>, CmsError> {
        if self.source.is_empty() { encode(&self.profile) } else { Ok(self.source.clone()) }
    }

    fn channels(&self) -> usize {
        match self.kind {
            ProfileKind::Cmyk => 4,
            ProfileKind::Gray => 1,
            ProfileKind::Rgb => 3,
        }
    }

    fn to_xf(&self, intent: Intent) -> Xf {
        self.to_srgb[intent_index(intent)]
            .get_or_init(|| Transform::new(&self.profile, &self.srgb, cms_intent(intent), false).ok().map(Arc::new))
            .clone()
    }

    fn inverse_xf(&self, intent: Intent) -> Xf {
        self.from_srgb[intent_index(intent)]
            .get_or_init(|| Transform::new(&self.srgb, &self.profile, cms_intent(intent), false).ok().map(Arc::new))
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
        xf.eval_fast(v, &mut dst).ok()?;
        Some(dst.map(|x| x.clamp(0.0, 1.0)))
    }

    /// Device pixels `v` (RGB / CMYK / Gray, 0..1, the profile's channels a pixel, any number of
    /// pixels) → sRGB in `dst` (three values a pixel), converted with `intent` in one pass of the
    /// cached transform. `None` when `v` doesn't hold whole pixels, `dst` doesn't hold three values
    /// for each, or no transform can be built.
    pub fn to_srgb_pixels(&self, v: &[f32], dst: &mut [f32], intent: Intent) -> Option<()> {
        let n = self.channels();
        if !v.len().is_multiple_of(n) || dst.len() != v.len() / n * 3 {
            return None;
        }
        let xf = self.to_xf(intent)?;
        xf.convert_f32(v, n, dst, 3, false).ok()?;
        dst.iter_mut().for_each(|x| *x = x.clamp(0.0, 1.0));
        Some(())
    }

    /// sRGB → device values.
    pub fn from_srgb(&self, rgb: [f32; 3], intent: Intent) -> Option<Vec<f32>> {
        let xf = self.inverse_xf(intent)?;
        let mut dst = vec![0.0f32; self.channels()];
        xf.eval_fast(&rgb, &mut dst).ok()?;
        Some(dst.into_iter().map(|x| x.clamp(0.0, 1.0)).collect())
    }

    fn direct_xf(&self, dest: &IccProfile, intent: Intent, bpc: bool) -> Xf {
        let generation = profiles_generation();
        let key = (dest.id, generation, intent_index(intent), bpc);
        let mut cache = self.direct.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|(k, _)| k.1 == generation);
        if let Some((_, t)) = cache.iter().find(|(k, _)| *k == key) {
            return t.clone();
        }
        let t = Transform::new(&self.profile, &dest.profile, cms_intent(intent), bpc).ok().map(Arc::new);
        cache.push((key, t.clone()));
        t
    }

    /// Device values (RGB / CMYK / Gray, 0..1) → `dest`'s RGB, converted directly with `intent`:
    /// not through sRGB, so colours outside sRGB keep their place in a wider space. With `bpc`
    /// (and an intent other than absolute), this profile's black maps onto the destination's
    /// black (black-point compensation). `None` when `dest` isn't RGB or no transform can be built.
    pub fn to_rgb_of(&self, dest: &IccProfile, v: &[f32], intent: Intent, bpc: bool) -> Option<[f32; 3]> {
        if dest.kind != ProfileKind::Rgb {
            return None;
        }
        let xf = self.direct_xf(dest, intent, bpc && intent != Intent::AbsoluteColorimetric)?;
        let mut src = v.to_vec();
        src.resize(self.channels(), 0.0);
        let mut out = [0.0f32; 3];
        xf.eval_fast(&src, &mut out).ok()?;
        Some(out.map(|x| x.clamp(0.0, 1.0)))
    }

    /// Device values of this profile → device values of `dest`, a profile of the same model,
    /// converted with `intent` directly. With `bpc` (and an intent other than absolute), this
    /// profile's black maps onto the destination's black. `None` when `dest` is of another model or
    /// no transform can be built.
    pub fn to_same_model_of(&self, dest: &IccProfile, v: &[f32], intent: Intent, bpc: bool) -> Option<Vec<f32>> {
        if dest.kind != self.kind || v.len() != self.channels() {
            return None;
        }
        let xf = self.direct_xf(dest, intent, bpc && intent != Intent::AbsoluteColorimetric)?;
        let mut out = vec![0.0f32; dest.channels()];
        xf.eval_fast(v, &mut out).ok()?;
        Some(out.into_iter().map(|x| x.clamp(0.0, 1.0)).collect())
    }

    /// Whether transforms can be built for this profile (checked at registration).
    pub fn usable(&self) -> bool {
        self.to_xf(Intent::RelativeColorimetric).is_some() && self.inverse_xf(Intent::RelativeColorimetric).is_some()
    }
}

/// The primaries and tone curves of built-in RGB space `name`.
fn rgb_space(name: &str) -> Option<Profile> {
    Some(match name {
        super::WIDE_GAMUT_RGB => Builtin::AdobeRgbCompat.profile().clone(),
        super::DISPLAY_P3 => Builtin::DisplayP3.profile().clone(),
        super::PROPHOTO_RGB => Builtin::ProPhotoCompat.profile().clone(),
        super::SRGB => Builtin::Srgb.profile().clone(),
        _ => return None,
    })
}

/// Built-in RGB working spaces from `craft-cms`'s standard primaries.
pub fn builtin_rgb(name: &str) -> Option<IccProfile> {
    IccProfile::from_profile(Some(name.to_string()), rgb_space(name)?).ok()
}

/// Built-in RGB space `name` as an ICC file, under its name here.
pub(super) fn builtin_rgb_bytes(name: &str) -> Result<Vec<u8>, CmsError> {
    encode(&labelled(rgb_space(name).ok_or_else(|| CmsError::UnknownProfile(name.into()))?, name))
}

/// Encode `p` as an ICC file. The creation date is fixed so exports are reproducible.
pub(super) fn encode(p: &Profile) -> Result<Vec<u8>, CmsError> {
    let mut bytes = p.to_bytes().as_ref().clone();
    // Header bytes 24..36: year, month, day, hours, minutes, seconds (u16 each).
    if let Some(date) = bytes.get_mut(24..36) {
        date.copy_from_slice(&[2026u16, 1, 1, 0, 0, 0].map(u16::to_be_bytes).concat());
    }
    Ok(bytes)
}

/// Profiles made here carry their name and no copyright claim.
fn labelled(mut p: Profile, name: &str) -> Profile {
    p.description = name.to_string();
    p.copyright = "No copyright, use freely".to_string();
    p
}

/// Grid points per input channel of the CMYK → Lab table and of the Lab → CMYK table.
const A2B_GRID: usize = 9;
const B2A_GRID: usize = 17;
/// Grid points per Lab channel of the gamut table.
const GAMUT_GRID: usize = 9;

/// A CMYK output profile sampled from a conversion: `to_lab` (CMYK → media-relative Lab) in a
/// 9⁴ table, and Lab → CMYK in 17³ tables, Lab PCS (ICC v4 encoding): `mapped` (gamut-mapped,
/// with black-point compensation) for the perceptual and saturation intents, `colorimetric` (the
/// colorimetric inverse of `to_lab`) for the colorimetric intents and the gamut tag.
pub(super) fn cmyk_profile(
    name: &str,
    paper: [f32; 3],
    to_lab: impl Fn([f32; 4]) -> lab::Lab,
    mapped: impl Fn(lab::Lab) -> [f32; 4],
    colorimetric: impl Fn(lab::Lab) -> [f32; 4],
) -> Result<Profile, CmsError> {
    let unit = |i: usize, n: usize| i as f32 / (n - 1) as f32;
    let mut a2b_clut = Vec::with_capacity(A2B_GRID.pow(4) * 3);
    for i in 0..A2B_GRID.pow(4) {
        let cmyk = [3, 2, 1, 0].map(|d| unit(i / A2B_GRID.pow(d) % A2B_GRID, A2B_GRID));
        let l = to_lab(cmyk);
        a2b_clut.extend([l.l / 100.0, (l.a + 128.0) / 255.0, (l.b + 128.0) / 255.0]);
    }
    let b2a_of = |from_lab: &dyn Fn(lab::Lab) -> [f32; 4]| {
        let mut clut = Vec::with_capacity(B2A_GRID.pow(3) * 4);
        for i in 0..B2A_GRID.pow(3) {
            let [l, a, b] = [2, 1, 0].map(|d| unit(i / B2A_GRID.pow(d) % B2A_GRID, B2A_GRID));
            clut.extend(from_lab(lab::Lab::new(l * 100.0, a * 255.0 - 128.0, b * 255.0 - 128.0)));
        }
        clut
    };
    let (b2a_mapped_clut, b2a_clut) = (b2a_of(&mapped), b2a_of(&colorimetric));
    let mut gamut_clut = Vec::with_capacity(GAMUT_GRID.pow(3));
    for i in 0..GAMUT_GRID.pow(3) {
        let [l, a, b] = [2, 1, 0].map(|d| unit(i / GAMUT_GRID.pow(d) % GAMUT_GRID, GAMUT_GRID));
        let want = lab::Lab::new(l * 100.0, a * 255.0 - 128.0, b * 255.0 - 128.0);
        gamut_clut.push(if lab::delta_e2000(want, to_lab(colorimetric(want))) <= 3.0 { 0.0 } else { 1.0 });
    }
    let table = |inputs: usize, outputs: usize, grid: usize, clut: Vec<f32>| -> Result<Lut, CmsError> {
        let clut = Clut::new(vec![grid; inputs], outputs, clut).map_err(|e| CmsError::BadProfile(format!("{e:?}")))?;
        Ok(Lut { kind: LutKind::Ab, inputs, outputs, stages: vec![Stage::Clut(clut)] })
    };
    let a2b = table(4, 3, A2B_GRID, a2b_clut)?;
    let b2a_mapped = table(3, 4, B2A_GRID, b2a_mapped_clut)?;
    let b2a = table(3, 4, B2A_GRID, b2a_clut)?;
    let _gamut = table(3, 1, GAMUT_GRID, gamut_clut)?;
    let mut p = Profile::gray([f64::from(paper[0]), f64::from(paper[1]), f64::from(paper[2])], Curve::Gamma(1.0))
        .unwrap_or_else(|_| Builtin::Srgb.profile().clone());
    p.class = ProfileClass::Output;
    p.color_space = ColorSpace::Cmyk;
    p.pcs = Pcs::Lab;
    p.rendering_intent = CmsIntent::RelativeColorimetric;
    p.white_point = [f64::from(paper[0]), f64::from(paper[1]), f64::from(paper[2])];
    p.matrix = None;
    p.trc = None;
    p.gray_trc = None;
    p.a2b = [Some(a2b.clone()), Some(a2b.clone()), Some(a2b)];
    p.b2a = [Some(b2a_mapped.clone()), Some(b2a), Some(b2a_mapped)];
    Ok(labelled(p, name))
}

/// The grey space of greyscale exports: sRGB's tone curve, D50 white.
pub(super) fn gray_profile(name: &str) -> Profile {
    labelled(Builtin::SGray.profile().clone(), name)
}

/// The first tag signature the tag table of ICC `bytes` lists twice, which the ICC
/// specification forbids.
pub(crate) fn repeated_tag(bytes: &[u8]) -> Option<u32> {
    let be32 = |at: usize| bytes.get(at..at.checked_add(4)?).map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
    let tags = be32(128)? as usize;
    let mut seen = std::collections::HashSet::new();
    for i in 0..tags {
        let sig = be32(i.checked_mul(12)?.checked_add(132)?)?;
        if !seen.insert(sig) {
            return Some(sig);
        }
    }
    None
}

/// Most bytes of UTF-16 the localized names (`mluc` tags) of a profile may take together.
const MAX_LOCALIZED_TEXT: usize = 1 << 20;

/// Whether the localized-name records of the `mluc` tags in ICC `bytes` take at most
/// [`MAX_LOCALIZED_TEXT`] together.
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
            let Some(length) = r.checked_mul(12).and_then(|h| be32(tag, h + 16 + 4)) else { break };
            total = total.saturating_add(length).saturating_add(12);
            if total > MAX_LOCALIZED_TEXT {
                return false;
            }
        }
    }
    true
}
