//! Colour management: working spaces, RGB ↔ CMYK ↔ Lab conversions with rendering intents,
//! gamut checks and soft-proof transforms.
//!
//! * **RGB working spaces** are ICC matrix/shaper profiles via [`moxcms`] (sRGB, Wide Gamut RGB
//!   (1998 primaries), Display P3, ProPhoto RGB) or user `.icc` files. Display output is sRGB;
//!   the desktop canvas converts it into the display's own profile last, in the app.
//! * **CMYK spaces:** [`GENERIC_CMYK`] is our own documented parametric press model
//!   ([`generic`]); [`FOGRA51_CMYK`] and [`FOGRA52_CMYK`] are the same model computed from
//!   Fogra's published characterization data ([`measured`]); [`DEVICE_CMYK`] is the old
//!   profile-free formula (kept for exact legacy numbers);
//!   any CMYK `.icc` the user loads goes through `moxcms` (LUT-based profiles).
//! * The **active settings** are process-wide: the working settings (Edit → Color Settings), with
//!   the profiles [`use_document_profiles`] last put in place of the working spaces;
//!   [`crate::Color::to_rgb`] uses them. [`with_active`] puts other settings in their place on one
//!   thread, as for a document drawn through its own profiles ([`for_document`]). Code that needs
//!   deterministic numbers builds its own [`Cms`] with [`Cms::new`].

pub mod generic;
pub mod icc;
pub mod lab;
pub mod measured;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

pub use generic::GenericCmyk;
pub use icc::IccProfile;
pub use lab::{Lab, delta_e76, delta_e2000};

use crate::Color;

pub const SRGB: &str = "sRGB IEC61966-2.1";
/// The wide-gamut RGB space with the 1998 primaries (γ 2.2, D65).
pub const WIDE_GAMUT_RGB: &str = "Wide Gamut RGB (1998 primaries)";
pub const DISPLAY_P3: &str = "Display P3";
pub const PROPHOTO_RGB: &str = "ProPhoto RGB";
/// Our parametric press model (see [`generic`]). Not an Adobe/ECI profile.
pub const GENERIC_CMYK: &str = "VectorCraft Generic CMYK (SWOP-like)";
/// VectorCraft's own press model computed from Fogra's FOGRA51 characterization data (offset on
/// premium coated paper, see [`measured`]).
pub const FOGRA51_CMYK: &str = "VectorCraft Coated CMYK (FOGRA51)";
/// VectorCraft's own press model computed from Fogra's FOGRA52 characterization data (offset on
/// wood-free uncoated paper, see [`measured`]).
pub const FOGRA52_CMYK: &str = "VectorCraft Uncoated CMYK (FOGRA52)";
/// Profile-free CMYK (`rgb = (1−c)(1−k)` …): uncalibrated, kept for legacy numbers.
pub const DEVICE_CMYK: &str = "Device CMYK (uncalibrated)";
/// The grey space greyscale exports are written in: sRGB's tone curve (a grey shows as on screen).
pub const GRAY: &str = "Gray (sRGB tone curve)";

/// Built-in RGB and CMYK working spaces, in menu order.
const BUILTIN_RGB: [&str; 4] = [SRGB, WIDE_GAMUT_RGB, DISPLAY_P3, PROPHOTO_RGB];
const BUILTIN_CMYK: [&str; 4] = [GENERIC_CMYK, FOGRA51_CMYK, FOGRA52_CMYK, DEVICE_CMYK];

/// Names earlier versions gave built-in profiles: (old name, current name). Settings, documents
/// and commands that still use an old name get the current profile.
pub const LEGACY_NAMES: &[(&str, &str)] = &[("Adobe RGB (1998) compatible", WIDE_GAMUT_RGB)]; // brand-ok: legacy alias

/// ΔE2000 above which a colour counts as out of the CMYK gamut (the gamut warning).
pub const GAMUT_THRESHOLD: f32 = 2.0;

/// ICC rendering intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Intent {
    Perceptual,
    #[default]
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

impl Intent {
    pub const ALL: [Intent; 4] = [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation, Intent::AbsoluteColorimetric];
    /// Parse `perceptual`, `relative`/`relativeColorimetric`, `saturation`, `absolute`/`absoluteColorimetric`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "perceptual" => Intent::Perceptual,
            "relative" | "relativecolorimetric" => Intent::RelativeColorimetric,
            "saturation" => Intent::Saturation,
            "absolute" | "absolutecolorimetric" => Intent::AbsoluteColorimetric,
            _ => return None,
        })
    }
    pub fn id(&self) -> &'static str {
        match self {
            Intent::Perceptual => "perceptual",
            Intent::RelativeColorimetric => "relative",
            Intent::Saturation => "saturation",
            Intent::AbsoluteColorimetric => "absolute",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Intent::Perceptual => "Perceptual",
            Intent::RelativeColorimetric => "Relative Colorimetric",
            Intent::Saturation => "Saturation",
            Intent::AbsoluteColorimetric => "Absolute Colorimetric",
        }
    }
}

/// A color management policy (Edit → Color Settings): what opening a document does with a profile
/// it is tagged with that is not the working space of its model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Policy {
    /// The document loses the profile; its color numbers stay as they are.
    Off,
    /// The document keeps the profile.
    #[default]
    PreserveEmbedded,
    /// The colors are converted from the profile into the working space, which keeps their
    /// appearance, and the document loses the profile.
    Convert,
    /// CMYK only: the document loses the profile; its color numbers stay as they are.
    PreserveNumbers,
}

impl Policy {
    /// The RGB policies, in menu order.
    pub const RGB: [Policy; 3] = [Policy::Off, Policy::PreserveEmbedded, Policy::Convert];
    /// The CMYK policies, in menu order.
    pub const CMYK: [Policy; 4] = [Policy::Off, Policy::PreserveEmbedded, Policy::Convert, Policy::PreserveNumbers];
    /// Parse an id of [`Policy::id`].
    pub fn parse(s: &str) -> Option<Self> {
        [Policy::Off, Policy::PreserveEmbedded, Policy::Convert, Policy::PreserveNumbers].into_iter().find(|p| p.id() == s)
    }
    pub fn id(&self) -> &'static str {
        match self {
            Policy::Off => "off",
            Policy::PreserveEmbedded => "preserveEmbedded",
            Policy::Convert => "convert",
            Policy::PreserveNumbers => "preserveNumbers",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Policy::Off => "Off",
            Policy::PreserveEmbedded => "Preserve Embedded Profiles",
            Policy::Convert => "Convert to Working Space",
            Policy::PreserveNumbers => "Preserve Numbers",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    Rgb,
    Cmyk,
    Gray,
}

/// A profile the user can pick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    pub kind: ProfileKind,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CmsError {
    #[error("unknown colour profile: {0}")]
    UnknownProfile(String),
    #[error("invalid ICC profile: {0}")]
    BadProfile(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("cannot read profile: {0}")]
    Io(String),
}

/// Edit → Color Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorSettings {
    /// RGB working space.
    pub rgb: String,
    /// CMYK working space.
    pub cmyk: String,
    /// Conversion intent (Convert to CMYK, document colour mode).
    pub intent: Intent,
    /// Black-point compensation.
    pub bpc: bool,
    /// The color management policy for RGB profiles (one of [`Policy::RGB`]). Settings saved
    /// without one take the default.
    #[serde(default)]
    pub rgb_policy: Policy,
    /// The color management policy for CMYK profiles. Settings saved without one take the default.
    #[serde(default)]
    pub cmyk_policy: Policy,
    /// Profile Mismatches: Ask When Opening. Off by default, as in settings saved without it.
    #[serde(default)]
    pub ask_mismatch_open: bool,
    /// Profile Mismatches: Ask When Pasting.
    #[serde(default)]
    pub ask_mismatch_paste: bool,
    /// Missing Profiles: Ask When Opening.
    #[serde(default)]
    pub ask_missing_open: bool,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            rgb: SRGB.into(),
            cmyk: GENERIC_CMYK.into(),
            intent: Intent::RelativeColorimetric,
            bpc: true,
            rgb_policy: Policy::default(),
            cmyk_policy: Policy::default(),
            ask_mismatch_open: false,
            ask_mismatch_paste: false,
            ask_missing_open: false,
        }
    }
}

/// Target colour model for [`Cms::convert`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Rgb,
    Cmyk,
    Gray,
    /// CIE L*a*b* (D50).
    Lab,
}

#[derive(Clone)]
enum CmykSpace {
    Generic(Arc<GenericCmyk>),
    Device,
    Icc(Arc<IccProfile>),
}

#[derive(Clone)]
enum RgbSpace {
    Srgb,
    Icc(Arc<IccProfile>),
}

fn generic() -> Arc<GenericCmyk> {
    static G: OnceLock<Arc<GenericCmyk>> = OnceLock::new();
    G.get_or_init(|| Arc::new(GenericCmyk::new(generic::swop_like()))).clone()
}

/// The working space of `condition`, computed from its data once and kept.
fn measured_space(condition: &measured::Condition) -> Result<Arc<GenericCmyk>, CmsError> {
    static SPACES: [OnceLock<Option<Arc<GenericCmyk>>>; measured::CONDITIONS.len()] = [const { OnceLock::new() }; measured::CONDITIONS.len()];
    let slot = measured::CONDITIONS.iter().position(|c| c.name == condition.name).and_then(|i| SPACES.get(i));
    slot.and_then(|s| s.get_or_init(|| measured::model(condition).map(Arc::new)).clone())
        .ok_or_else(|| CmsError::BadProfile(format!("{}: the {} data does not give a press model", condition.name, condition.reference)))
}

/// The registered printing condition a built-in CMYK working space characterizes: its reference
/// name, the `/OutputConditionIdentifier` of a PDF/X output intent, and the registry it belongs
/// to (by its current or legacy name). `None` for every other profile.
pub fn characterization(name: &str) -> Option<(&'static str, &'static str)> {
    measured::condition(canonical_name(name)).map(|c| (c.reference, measured::REGISTRY))
}

static USER: RwLock<Vec<Arc<IccProfile>>> = RwLock::new(Vec::new());
/// Counts the profiles [`register_icc`] has registered ([`profiles_generation`]).
static PROFILES_GENERATION: AtomicU64 = AtomicU64::new(0);

fn builtin_rgb_cache(name: &str) -> Option<Arc<IccProfile>> {
    static CACHE: Mutex<Vec<Arc<IccProfile>>> = Mutex::new(Vec::new());
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = c.iter().find(|p| p.name == name) {
        return Some(p.clone());
    }
    let p = Arc::new(icc::builtin_rgb(name)?);
    c.push(p.clone());
    Some(p)
}

fn user_profile(name: &str) -> Option<Arc<IccProfile>> {
    USER.read().unwrap_or_else(|e| e.into_inner()).iter().find(|p| p.name == name).cloned()
}

fn cmyk_space(name: &str) -> Result<CmykSpace, CmsError> {
    let canonical = canonical_name(name);
    if let Some(c) = measured::condition(canonical) {
        return measured_space(c).map(CmykSpace::Generic);
    }
    match canonical {
        GENERIC_CMYK => Ok(CmykSpace::Generic(generic())),
        DEVICE_CMYK => Ok(CmykSpace::Device),
        _ => match user_profile(name) {
            Some(p) if p.kind == ProfileKind::Cmyk => Ok(CmykSpace::Icc(p)),
            Some(_) => Err(CmsError::Unsupported(format!("{name} is not a CMYK profile"))),
            None => Err(CmsError::UnknownProfile(name.into())),
        },
    }
}

fn rgb_space(name: &str) -> Result<RgbSpace, CmsError> {
    if name == SRGB {
        return Ok(RgbSpace::Srgb);
    }
    if let Some(p) = builtin_rgb_cache(name) {
        return Ok(RgbSpace::Icc(p));
    }
    match user_profile(name) {
        Some(p) if p.kind == ProfileKind::Rgb => Ok(RgbSpace::Icc(p)),
        Some(_) => Err(CmsError::Unsupported(format!("{name} is not an RGB profile"))),
        None => Err(CmsError::UnknownProfile(name.into())),
    }
}

/// The current name of a profile: names earlier versions used resolve to today's name.
pub fn canonical_name(name: &str) -> &str {
    LEGACY_NAMES.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new)
}

/// An available profile by its current or legacy name.
pub fn profile(name: &str) -> Option<ProfileInfo> {
    let name = canonical_name(name);
    profiles().into_iter().find(|p| p.name == name)
}

/// Every profile available (built-in first, then user-loaded).
pub fn profiles() -> Vec<ProfileInfo> {
    let mut v: Vec<ProfileInfo> = BUILTIN_RGB
        .iter()
        .map(|n| ProfileInfo { name: (*n).into(), kind: ProfileKind::Rgb, builtin: true })
        .chain(BUILTIN_CMYK.iter().map(|n| ProfileInfo { name: (*n).into(), kind: ProfileKind::Cmyk, builtin: true }))
        .collect();
    for p in USER.read().unwrap_or_else(|e| e.into_inner()).iter() {
        v.push(ProfileInfo { name: p.name.clone(), kind: p.kind, builtin: false });
    }
    v
}

/// Register an ICC profile from bytes (named by its description unless `name` is given).
/// Re-registering a name replaces the old profile.
pub fn register_icc(bytes: &[u8], name: Option<String>) -> Result<ProfileInfo, CmsError> {
    let p = IccProfile::from_bytes(name, bytes)?;
    if p.kind == ProfileKind::Gray {
        return Err(CmsError::Unsupported("grayscale profiles can't be working spaces yet".into()));
    }
    let builtin = canonical_name(&p.name);
    if BUILTIN_RGB.contains(&builtin) || BUILTIN_CMYK.contains(&builtin) {
        return Err(CmsError::Unsupported(format!("{} clashes with a built-in profile name", p.name)));
    }
    if !p.usable() {
        return Err(CmsError::Unsupported(format!("{}: no usable transform to/from sRGB", p.name)));
    }
    let info = ProfileInfo { name: p.name.clone(), kind: p.kind, builtin: false };
    let mut u = USER.write().unwrap_or_else(|e| e.into_inner());
    u.retain(|x| x.name != info.name);
    u.push(Arc::new(p));
    drop(u);
    clear_proof_cache();
    PROFILES_GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(info)
}

/// Forget a profile [`register_icc`] registered (by its name); `false` when there is none of that
/// name (the built-in profiles can't be removed). Settings that name it no longer resolve, so a
/// caller checks [`active_settings`] first.
pub fn unregister_icc(name: &str) -> bool {
    let mut u = USER.write().unwrap_or_else(|e| e.into_inner());
    let before = u.len();
    u.retain(|x| x.name != name);
    let removed = u.len() != before;
    drop(u);
    if removed {
        clear_proof_cache();
        PROFILES_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    removed
}

/// A number that changes whenever [`register_icc`] registers a profile or [`unregister_icc`]
/// removes one: conversions through a profile looked up by name, such as those of a
/// [`ProofTarget::Cmyk`] proof, give the same colors as long as the number stays the same.
pub fn profiles_generation() -> u64 {
    PROFILES_GENERATION.load(Ordering::Relaxed)
}

/// Profile `name` as an ICC file to embed in an export: any RGB or CMYK profile of [`profiles`]
/// (by its current or legacy name), or [`GRAY`]. Built-in profiles are written in code from the
/// CMS: the RGB spaces from their primaries, the CMYK spaces as lookup tables sampled from their
/// model (both directions, perceptual and relative colorimetric, Lab PCS, with the model's paper
/// as the media white point; the colorimetric Lab → CMYK table without black-point
/// compensation, the perceptual and saturation ones with it); a profile the user
/// loaded is the file it came from. Cached.
pub fn icc_bytes(name: &str) -> Result<Arc<[u8]>, CmsError> {
    static CACHE: Mutex<Vec<(String, Arc<[u8]>)>> = Mutex::new(Vec::new());
    let name = canonical_name(name);
    if let Some(p) = user_profile(name) {
        return p.bytes().map(Arc::from);
    }
    if let Some((_, b)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(n, _)| n == name) {
        return Ok(b.clone());
    }
    let bytes: Arc<[u8]> = if name == GRAY {
        icc::encode(&icc::gray_profile(name))?
    } else if BUILTIN_CMYK.contains(&name) {
        // The perceptual and saturation tables map the shadows with black-point compensation; the
        // colorimetric table is the inverse of the CMYK → Lab table, without it.
        let mapped = Cms::new(&ColorSettings { cmyk: name.into(), bpc: true, ..ColorSettings::default() })?;
        let cms = Cms::new(&ColorSettings { cmyk: name.into(), bpc: false, ..ColorSettings::default() })?;
        let paper = match &cms.cmyk {
            CmykSpace::Generic(g) => g.paper_xyz(),
            CmykSpace::Device | CmykSpace::Icc(_) => lab::D50,
        };
        let copyright = match measured::condition(name) {
            Some(c) => format!("Computed by VectorCraft from the {} characterization data. Data source: Fogra, www.fogra.org", c.reference),
            None => icc::NO_COPYRIGHT.to_string(),
        };
        let to_lab = |c| cms.cmyk_to_lab(c);
        let shadows = |l| mapped.lab_to_cmyk(l, Intent::RelativeColorimetric);
        icc::encode(&icc::cmyk_profile(name, paper, &copyright, to_lab, shadows, |l| cms.lab_to_cmyk(l, Intent::RelativeColorimetric)))?
    } else {
        icc::builtin_rgb_bytes(name)?
    }
    .into();
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).push((name.to_string(), bytes.clone()));
    Ok(bytes)
}

/// Load a `.icc`/`.icm` file from disk (native only).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_icc_file(path: &std::path::Path) -> Result<ProfileInfo, CmsError> {
    let bytes = std::fs::read(path).map_err(|e| CmsError::Io(format!("{}: {e}", path.display())))?;
    register_icc(&bytes, None)
}

/// A colour-management context: resolved working spaces plus conversion settings.
#[derive(Clone)]
pub struct Cms {
    settings: ColorSettings,
    rgb: RgbSpace,
    cmyk: CmykSpace,
    /// The working settings, when a document's assigned profiles take the place of their spaces
    /// ([`use_document_profiles`]): a [`ProofTarget::WorkingCmyk`] proof simulates their CMYK space.
    working: Option<Arc<Cms>>,
}

impl std::fmt::Debug for Cms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cms").field("settings", &self.settings).finish()
    }
}

impl Default for Cms {
    fn default() -> Self {
        Self { settings: ColorSettings::default(), rgb: RgbSpace::Srgb, cmyk: CmykSpace::Generic(generic()), working: None }
    }
}

impl Cms {
    /// Profiles named by a legacy name are stored under their current name.
    pub fn new(settings: &ColorSettings) -> Result<Self, CmsError> {
        let settings = ColorSettings { rgb: canonical_name(&settings.rgb).into(), cmyk: canonical_name(&settings.cmyk).into(), ..settings.clone() };
        Ok(Self { rgb: rgb_space(&settings.rgb)?, cmyk: cmyk_space(&settings.cmyk)?, settings, working: None })
    }
    pub fn settings(&self) -> &ColorSettings {
        &self.settings
    }
    pub fn rgb_is_srgb(&self) -> bool {
        matches!(self.rgb, RgbSpace::Srgb)
    }

    /// Working RGB → display sRGB.
    pub fn rgb_to_srgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        match &self.rgb {
            RgbSpace::Srgb => rgb,
            RgbSpace::Icc(p) => p.to_srgb(&rgb, Intent::RelativeColorimetric).unwrap_or(rgb),
        }
    }
    /// Display sRGB → working RGB.
    pub fn srgb_to_rgb(&self, srgb: [f32; 3]) -> [f32; 3] {
        match &self.rgb {
            RgbSpace::Srgb => srgb,
            RgbSpace::Icc(p) => p.from_srgb(srgb, Intent::RelativeColorimetric).map(|v| [v[0], v[1], v[2]]).unwrap_or(srgb),
        }
    }

    fn cmyk_to_srgb_in(space: &CmykSpace, cmyk: [f32; 4], absolute: bool, bpc: bool) -> [f32; 3] {
        let cmyk = cmyk.map(|v| v.clamp(0.0, 1.0));
        match space {
            CmykSpace::Generic(g) => g.to_srgb(cmyk, absolute, bpc),
            CmykSpace::Device => naive_cmyk_to_rgb(cmyk),
            CmykSpace::Icc(p) => {
                // With black-point compensation the profile's black maps onto sRGB black, as
                // `cmyk_to_srgb` shows the working space.
                if bpc
                    && !absolute
                    && let Some(srgb) = builtin_rgb_cache(SRGB)
                    && let Some(v) = p.to_rgb_of(&srgb, &cmyk, Intent::RelativeColorimetric, true)
                {
                    return v;
                }
                let i = if absolute { Intent::AbsoluteColorimetric } else { Intent::RelativeColorimetric };
                p.to_srgb(&cmyk, i).unwrap_or_else(|| naive_cmyk_to_rgb(cmyk))
            }
        }
    }

    fn srgb_to_cmyk_in(space: &CmykSpace, srgb: [f32; 3], intent: Intent, bpc: bool) -> [f32; 4] {
        match space {
            CmykSpace::Generic(g) => g.from_srgb(srgb, intent, bpc),
            CmykSpace::Device => naive_rgb_to_cmyk(srgb),
            CmykSpace::Icc(p) => p.from_srgb(srgb, intent).map(|v| [v[0], v[1], v[2], v[3]]).unwrap_or_else(|| naive_rgb_to_cmyk(srgb)),
        }
    }

    /// CMYK in the working space → display sRGB (`absolute` simulates the paper colour). An ICC
    /// CMYK profile converts as [`Self::cmyk_to_rgb`] does into sRGB (with the settings'
    /// black-point compensation), so what shows is what an RGB export writes.
    pub fn cmyk_to_srgb(&self, cmyk: [f32; 4], absolute: bool) -> [f32; 3] {
        if let (false, CmykSpace::Icc(p)) = (absolute, &self.cmyk)
            && let Some(srgb) = builtin_rgb_cache(SRGB)
            && let Some(v) = p.to_rgb_of(&srgb, &cmyk.map(|v| v.clamp(0.0, 1.0)), Intent::RelativeColorimetric, self.settings.bpc)
        {
            return v;
        }
        Self::cmyk_to_srgb_in(&self.cmyk, cmyk, absolute, self.settings.bpc)
    }
    /// CMYK in the working space → working RGB with `intent`. An ICC CMYK profile converts
    /// directly into the RGB profile: not through display sRGB, which would clip colours a wider
    /// RGB space holds (Wide Gamut RGB keeps CMYK cyan that sRGB can't show), and with the
    /// settings' black-point compensation. The built-in CMYK spaces go through sRGB as before
    /// (Generic CMYK compensates in its own model); with the absolute colorimetric intent the
    /// paper colour shows.
    pub fn cmyk_to_rgb(&self, cmyk: [f32; 4], intent: Intent) -> [f32; 3] {
        let cmyk = cmyk.map(|v| v.clamp(0.0, 1.0));
        let src = match &self.cmyk {
            CmykSpace::Icc(p) => Some(p.clone()),
            CmykSpace::Generic(_) | CmykSpace::Device => None,
        };
        let dest = match &self.rgb {
            RgbSpace::Icc(p) => Some(p.clone()),
            RgbSpace::Srgb => builtin_rgb_cache(SRGB),
        };
        src.zip(dest)
            .and_then(|(s, d)| s.to_rgb_of(&d, &cmyk, intent, self.settings.bpc))
            .unwrap_or_else(|| self.srgb_to_rgb(self.cmyk_to_srgb(cmyk, intent == Intent::AbsoluteColorimetric)))
    }
    /// Display sRGB → working CMYK with `intent`.
    pub fn srgb_to_cmyk(&self, srgb: [f32; 3], intent: Intent) -> [f32; 4] {
        Self::srgb_to_cmyk_in(&self.cmyk, srgb, intent, self.settings.bpc)
    }
    /// Media-relative Lab of a working-space CMYK colour.
    pub fn cmyk_to_lab(&self, cmyk: [f32; 4]) -> Lab {
        match &self.cmyk {
            CmykSpace::Generic(g) => g.to_lab_rel(cmyk.map(|v| v.clamp(0.0, 1.0))),
            _ => lab::srgb_to_lab(self.cmyk_to_srgb(cmyk, false)),
        }
    }
    /// Lab of working-space CMYK as it displays: [`Self::cmyk_to_lab`] plus, for the built-in
    /// Generic CMYK, the black-point compensation of [`Self::cmyk_to_srgb`] (its press black shows
    /// as black). Unclipped, for interpolating tables.
    pub fn cmyk_display_lab(&self, cmyk: [f32; 4]) -> Lab {
        match &self.cmyk {
            CmykSpace::Generic(g) => lab::xyz_to_lab(g.display_xyz(cmyk.map(|v| v.clamp(0.0, 1.0)), false, self.settings.bpc)),
            _ => self.cmyk_to_lab(cmyk),
        }
    }
    /// Lab → working CMYK with `intent`.
    pub fn lab_to_cmyk(&self, l: Lab, intent: Intent) -> [f32; 4] {
        match &self.cmyk {
            CmykSpace::Generic(g) => g.from_lab(l, intent, self.settings.bpc),
            _ => self.srgb_to_cmyk(lab::lab_to_srgb(l), intent),
        }
    }

    /// Display sRGB of any colour.
    pub fn display_rgb(&self, c: &Color) -> [f32; 3] {
        match *c {
            Color::Rgb { r, g, b } => self.rgb_to_srgb([r, g, b]),
            Color::Cmyk { c, m, y, k } => self.cmyk_to_srgb([c, m, y, k], false),
            Color::Gray { k } => [1.0 - k.clamp(0.0, 1.0); 3],
            Color::Lab { l, a, b } => lab::lab_to_srgb(Lab::new(l, a, b)),
        }
    }

    /// CIE Lab (D50) of any colour.
    pub fn lab(&self, c: &Color) -> Lab {
        match *c {
            Color::Cmyk { c, m, y, k } => self.cmyk_display_lab([c, m, y, k]),
            Color::Lab { l, a, b } => Lab::new(l, a, b),
            _ => lab::srgb_to_lab(self.display_rgb(c)),
        }
    }

    /// Ink values of any colour in the working CMYK space (CMYK passes through; grey is K only;
    /// Lab separates from its own values).
    pub fn to_cmyk(&self, c: &Color, intent: Intent) -> [f32; 4] {
        match *c {
            Color::Cmyk { c, m, y, k } => [c, m, y, k],
            Color::Gray { k } => [0.0, 0.0, 0.0, k],
            Color::Rgb { .. } => self.srgb_to_cmyk(self.display_rgb(c), intent),
            Color::Lab { l, a, b } => self.lab_to_cmyk(Lab::new(l, a, b), intent),
        }
    }

    /// Convert `c` into `model` (a colour already in `model` is returned unchanged).
    pub fn convert(&self, c: &Color, model: Model, intent: Intent) -> Color {
        match (c, model) {
            (Color::Rgb { .. }, Model::Rgb)
            | (Color::Cmyk { .. }, Model::Cmyk)
            | (Color::Gray { .. }, Model::Gray)
            | (Color::Lab { .. }, Model::Lab) => *c,
            (_, Model::Lab) => {
                let Lab { l, a, b } = self.lab(c);
                Color::Lab { l, a, b }
            }
            (_, Model::Cmyk) => {
                let [c, m, y, k] = self.to_cmyk(c, intent);
                Color::Cmyk { c, m, y, k }
            }
            (Color::Cmyk { c, m, y, k }, Model::Rgb) => {
                let [r, g, b] = self.cmyk_to_rgb([*c, *m, *y, *k], intent);
                Color::Rgb { r, g, b }
            }
            (_, Model::Rgb) => {
                let [r, g, b] = self.srgb_to_rgb(self.display_rgb(c));
                Color::Rgb { r, g, b }
            }
            (_, Model::Gray) => {
                // Neutral with the same L* (grey ink percentage is 1 − lightness on screen).
                let l = self.lab(c).l;
                let v = lab::xyz_to_srgb(lab::lab_to_xyz(Lab::new(l, 0.0, 0.0)))[1];
                Color::Gray { k: (1.0 - v).clamp(0.0, 1.0) }
            }
        }
    }

    /// Colour from Lab (in the working RGB space).
    pub fn from_lab(&self, l: Lab) -> Color {
        let [r, g, b] = self.srgb_to_rgb(lab::lab_to_srgb(l));
        Color::Rgb { r, g, b }
    }

    /// An RGB or CMYK colour of this context's space as the colour of the same model in `dest`'s
    /// space that looks the same, converted with `intent` (Edit → Color Settings › Convert to
    /// Working Space). RGB converts directly from profile to profile, with this context's
    /// black-point compensation. CMYK converts between two loaded profiles through ProPhoto RGB
    /// ([`IccProfile::to_same_model_of`]), and through Lab when either space is built in: the Lab it
    /// displays as ([`Self::cmyk_display_lab`]), or for Absolute Colorimetric its absolute Lab. Grey
    /// and Lab colours stay as they are.
    pub fn convert_into(&self, dest: &Cms, c: &Color, intent: Intent) -> Color {
        match *c {
            Color::Rgb { r, g, b } => {
                let icc = |s: &RgbSpace| match s {
                    RgbSpace::Icc(p) => Some(p.clone()),
                    RgbSpace::Srgb => builtin_rgb_cache(SRGB),
                };
                let [r, g, b] = icc(&self.rgb)
                    .zip(icc(&dest.rgb))
                    .and_then(|(s, d)| s.to_rgb_of(&d, &[r, g, b], intent, self.settings.bpc))
                    .unwrap_or_else(|| dest.srgb_to_rgb(self.rgb_to_srgb([r, g, b])));
                Color::Rgb { r, g, b }
            }
            Color::Cmyk { c, m, y, k } => {
                let inks = [c, m, y, k].map(|v| v.clamp(0.0, 1.0));
                let direct = match (&self.cmyk, &dest.cmyk) {
                    (CmykSpace::Icc(s), CmykSpace::Icc(d)) => s.to_same_model_of(d, &inks, intent, self.settings.bpc),
                    _ => None,
                };
                // Absolute colorimetry starts from the inks' absolute Lab, paper color included and
                // without black-point compensation; the other intents from the Lab they display as.
                let source_lab = || match (&self.cmyk, intent) {
                    (CmykSpace::Generic(g), Intent::AbsoluteColorimetric) => g.to_lab_abs(inks),
                    (_, Intent::AbsoluteColorimetric) => lab::srgb_to_lab(self.cmyk_to_srgb(inks, true)),
                    _ => self.cmyk_display_lab(inks),
                };
                let [c, m, y, k] = match direct.as_deref() {
                    Some(&[c, m, y, k]) => [c, m, y, k],
                    _ => dest.lab_to_cmyk(source_lab(), intent),
                };
                Color::Cmyk { c, m, y, k }
            }
            Color::Gray { .. } | Color::Lab { .. } => *c,
        }
    }

    /// How far (ΔE2000) the best CMYK reproduction of `c` is from `c` (0 for CMYK/grey colours).
    pub fn gamut_error(&self, c: &Color) -> f32 {
        match c {
            Color::Rgb { .. } | Color::Lab { .. } => {
                let src = self.lab(c);
                match &self.cmyk {
                    CmykSpace::Generic(g) => g.gamut_error(src, self.settings.bpc),
                    CmykSpace::Device => 0.0,
                    CmykSpace::Icc(_) => {
                        let back = self.cmyk_to_srgb(self.to_cmyk(c, Intent::RelativeColorimetric), false);
                        delta_e2000(src, lab::srgb_to_lab(back))
                    }
                }
            }
            Color::Cmyk { .. } | Color::Gray { .. } => 0.0,
        }
    }

    /// Gamut warning: can this color be printed in this context's CMYK space?
    pub fn out_of_gamut(&self, c: &Color) -> bool {
        self.gamut_error(c) > GAMUT_THRESHOLD
    }

    /// Soft-proof one display sRGB colour.
    pub fn proof_srgb(&self, srgb: [f32; 3], proof: &ProofSetup) -> [f32; 3] {
        let srgb = srgb.map(|v| v.clamp(0.0, 1.0));
        match &proof.target {
            ProofTarget::Srgb | ProofTarget::MonitorRgb => srgb,
            ProofTarget::LegacyMacRgb => srgb.map(|v| lab::linear_to_srgb(v.powf(1.8))),
            ProofTarget::Protanopia => simulate_cvd(srgb, &PROTANOPIA),
            ProofTarget::Deuteranopia => simulate_cvd(srgb, &DEUTERANOPIA),
            ProofTarget::Tritanopia => TRITANOPIA.simulate(srgb),
            ProofTarget::WorkingCmyk => {
                let working = self.working_cms();
                let cmyk = if proof.preserve_numbers { self.preserved_cmyk(srgb) } else { working.srgb_to_cmyk(srgb, proof.intent) };
                self.proof_cmyk_to_srgb(cmyk, proof)
            }
            ProofTarget::Cmyk(name) => match cmyk_space(name) {
                Ok(sp) => {
                    let cmyk = if proof.preserve_numbers {
                        self.preserved_cmyk(srgb)
                    } else {
                        Self::srgb_to_cmyk_in(&sp, srgb, proof.intent, self.settings.bpc)
                    };
                    self.proof_cmyk_to_srgb(cmyk, proof)
                }
                Err(_) => srgb,
            },
            ProofTarget::Rgb(name) => match rgb_space(canonical_name(name)) {
                Ok(RgbSpace::Srgb) if !proof.preserve_numbers => srgb,
                Ok(RgbSpace::Srgb) => self.srgb_to_rgb(srgb).map(|v| v.clamp(0.0, 1.0)),
                Ok(RgbSpace::Icc(p)) => {
                    // The device's own numbers for the color: the document's RGB numbers unchanged,
                    // or the color converted into the device's space with the proof's intent.
                    let numbers = if proof.preserve_numbers {
                        self.srgb_to_rgb(srgb)
                    } else {
                        p.from_srgb(srgb, proof.intent).and_then(|v| Some([*v.first()?, *v.get(1)?, *v.get(2)?])).unwrap_or(srgb)
                    };
                    p.to_srgb(&numbers, Intent::RelativeColorimetric).unwrap_or(srgb)
                }
                Err(_) => srgb,
            },
        }
    }

    /// Display sRGB of color `c` proofed with Preserve Numbers on a CMYK or RGB device: its own
    /// numbers go to the device unchanged. A CMYK device takes a CMYK color's inks as they are, a
    /// gray as K only and another color's inks in this context's CMYK space (relative
    /// colorimetric); an RGB device takes an RGB color's numbers as they are and another color's
    /// numbers in this context's RGB space. Other targets proof the color's display sRGB
    /// ([`Self::proof_srgb`]).
    pub fn proof_color(&self, c: &Color, proof: &ProofSetup) -> [f32; 3] {
        if !proof.preserve_numbers {
            return self.proof_srgb(self.display_rgb(c), proof);
        }
        match &proof.target {
            // A device whose profile is not loaded leaves the color as it is, as for a pixel.
            ProofTarget::Cmyk(name) if cmyk_space(name).is_err() => self.display_rgb(c),
            ProofTarget::WorkingCmyk | ProofTarget::Cmyk(_) => self.proof_cmyk_to_srgb(self.to_cmyk(c, Intent::RelativeColorimetric), proof),
            ProofTarget::Rgb(name) => {
                // A CMYK color converts straight into the RGB space, so a wide space keeps it.
                let numbers = match self.convert(c, Model::Rgb, Intent::RelativeColorimetric) {
                    Color::Rgb { r, g, b } => [r, g, b],
                    other => self.srgb_to_rgb(self.display_rgb(&other)),
                }
                .map(|v| v.clamp(0.0, 1.0));
                match rgb_space(canonical_name(name)) {
                    Ok(RgbSpace::Srgb) => numbers,
                    Ok(RgbSpace::Icc(p)) => p.to_srgb(&numbers, Intent::RelativeColorimetric).unwrap_or(numbers),
                    Err(_) => self.display_rgb(c),
                }
            }
            _ => self.proof_srgb(self.display_rgb(c), proof),
        }
    }

    /// The CMYK numbers display sRGB `srgb` has in this context's CMYK space (relative
    /// colorimetric), which Preserve CMYK Numbers sends to the simulated device unchanged: for
    /// pixels whose own numbers are not known (an RGB image's pixels, art drawn over other art).
    /// A color whose numbers are known proofs through [`Self::proof_color`].
    fn preserved_cmyk(&self, srgb: [f32; 3]) -> [f32; 4] {
        self.srgb_to_cmyk(srgb, Intent::RelativeColorimetric)
    }

    /// Display sRGB of CMYK inks in the proof's CMYK space (this context's CMYK space for RGB
    /// targets). Simulate Paper Color shows the paper's color and the press black (absolute
    /// colorimetric); else Simulate Black Ink shows the press black as the dark gray it prints
    /// (media-relative, without black-point compensation).
    pub fn proof_cmyk_to_srgb(&self, cmyk: [f32; 4], proof: &ProofSetup) -> [f32; 3] {
        let black = proof.simulate_black && !proof.simulate_paper;
        match &proof.target {
            ProofTarget::Cmyk(name) => match cmyk_space(name) {
                Ok(sp) if black => Self::black_ink_srgb(&sp, cmyk),
                Ok(sp) => Self::cmyk_to_srgb_in(&sp, cmyk, proof.simulate_paper, self.settings.bpc),
                Err(_) => self.cmyk_to_srgb(cmyk, proof.simulate_paper),
            },
            ProofTarget::WorkingCmyk if black => Self::black_ink_srgb(&self.working_cms().cmyk, cmyk),
            ProofTarget::WorkingCmyk => self.working_cms().cmyk_to_srgb(cmyk, proof.simulate_paper),
            _ => self.cmyk_to_srgb(cmyk, proof.simulate_paper),
        }
    }

    /// Display sRGB of `cmyk` in `space`, media-relative and without black-point compensation, so
    /// the darkest ink the space prints shows as the gray it is rather than as black.
    fn black_ink_srgb(space: &CmykSpace, cmyk: [f32; 4]) -> [f32; 3] {
        let cmyk = cmyk.map(|v| v.clamp(0.0, 1.0));
        if let CmykSpace::Icc(p) = space
            && let Some(srgb) = builtin_rgb_cache(SRGB)
            && let Some(v) = p.to_rgb_of(&srgb, &cmyk, Intent::RelativeColorimetric, false)
        {
            return v;
        }
        Self::cmyk_to_srgb_in(space, cmyk, false, false)
    }

    /// The context of the working settings: the one [`use_document_profiles`] put a document's
    /// profiles in place of, else this one.
    fn working_cms(&self) -> &Cms {
        self.working.as_deref().unwrap_or(self)
    }

    /// A cached 3-D lookup table implementing [`Cms::proof_srgb`] for 8-bit pixels.
    pub fn proof_lut(&self, proof: &ProofSetup) -> Arc<ProofLut> {
        let working = self.working.as_ref().map(|w| &w.settings);
        let key = format!(
            "{:?}|{working:?}|{:?}|{:?}|{}|{}|{}",
            self.settings, proof.target, proof.intent, proof.simulate_paper, proof.preserve_numbers, proof.simulate_black
        );
        let mut cache = PROOF_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, l)) = cache.iter().find(|(k, _)| *k == key) {
            return l.clone();
        }
        let lut = Arc::new(ProofLut::build(|rgb| self.proof_srgb(rgb, proof)));
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((key, lut.clone()));
        lut
    }
}

static PROOF_CACHE: Mutex<Vec<(String, Arc<ProofLut>)>> = Mutex::new(Vec::new());

fn clear_proof_cache() {
    PROOF_CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

pub(crate) fn naive_cmyk_to_rgb([c, m, y, k]: [f32; 4]) -> [f32; 3] {
    [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
}

pub(crate) fn naive_rgb_to_cmyk([r, g, b]: [f32; 3]) -> [f32; 4] {
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [(1.0 - r - k) / (1.0 - k), (1.0 - g - k) / (1.0 - k), (1.0 - b - k) / (1.0 - k), k]
}

// Machado, Oliveira & Fernandes (2009) dichromacy simulation matrices (severity 1.0, linear RGB).
const PROTANOPIA: [[f32; 3]; 3] = [[0.152_286, 1.052_583, -0.204_868], [0.114_503, 0.786_281, 0.099_216], [-0.003_882, -0.048_116, 1.051_998]];
const DEUTERANOPIA: [[f32; 3]; 3] = [[0.367_322, 0.860_646, -0.227_968], [0.280_085, 0.672_501, 0.047_413], [-0.011_820, 0.042_940, 0.968_881]];

fn simulate_cvd(srgb: [f32; 3], m: &[[f32; 3]; 3]) -> [f32; 3] {
    lab::mat_mul(m, lab::srgb_linear(srgb)).map(|v| lab::linear_to_srgb(v.clamp(0.0, 1.0)))
}

// Tritanopia: Brettel, Viénot & Mollon (1997), "Computerized simulation of color appearance for
// dichromats", JOSA A 14(10), 2647-2655, https://doi.org/10.1364/JOSAA.14.002647. A color keeps its
// L and M cone responses, and its S response is replaced so that it lies on one of two half-planes in
// LMS space. Both half-planes contain the neutral axis (black to the display's white, so grays keep
// their value); one passes through the 485 nm spectral color and the other through the 660 nm one. A
// color uses the half-plane on its own side of the plane through the neutral axis and the S axis.
// The matrices are computed at compile time from these inputs:
// - linear sRGB → CIE XYZ: IEC 61966-2-1 (D65);
// - CIE XYZ → LMS: Smith & Pokorny (1975), as printed in https://daltonlens.org/understanding-cvd-simulation/;
// - the 485 nm and 660 nm colors: the Judd-Vos color matching functions,
//   http://www.cvrl.org/database/data/cmfs/ciexyzjv.csv.
const LINEAR_SRGB_TO_XYZ: [[f64; 3]; 3] = [[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]];
const XYZ_TO_LMS: [[f64; 3]; 3] = [[0.155_14, 0.543_12, -0.032_86], [-0.155_14, 0.456_84, 0.032_86], [0.0, 0.0, 0.016_08]];
const XYZ_485_NM: [f64; 3] = [0.056_985, 0.169_87, 0.586_4];
const XYZ_660_NM: [f64; 3] = [0.161_61, 0.061, 0.000_011_906];
const TRITANOPIA: Dichromat = Dichromat::tritanope();

/// A dichromat simulation with two half-planes, as matrices on linear sRGB. `near` applies to colors
/// whose dot product with `side` is 0 or more, and `far` applies to the others.
struct Dichromat {
    side: [f32; 3],
    near: [[f32; 3]; 3],
    far: [[f32; 3]; 3],
}

impl Dichromat {
    const fn tritanope() -> Self {
        let to_lms = mat3_mul(&XYZ_TO_LMS, &LINEAR_SRGB_TO_XYZ);
        let to_rgb = mat3_inverse(&to_lms);
        let white = mat3_apply(&to_lms, [1.0; 3]);
        let a485 = mat3_apply(&XYZ_TO_LMS, XYZ_485_NM);
        let a660 = mat3_apply(&XYZ_TO_LMS, XYZ_660_NM);
        let mut side = cross(white, [0.0, 0.0, 1.0]);
        if dot(side, a485) < 0.0 {
            side = [-side[0], -side[1], -side[2]];
        }
        // A color on the 485 nm side lands on that half-plane.
        let near = mat3_mul(&to_rgb, &mat3_mul(&keep_lm_on_plane(cross(white, a485)), &to_lms));
        let far = mat3_mul(&to_rgb, &mat3_mul(&keep_lm_on_plane(cross(white, a660)), &to_lms));
        // `side · lms(rgb)` is `(to_lmsᵀ · side) · rgb`.
        let t = &to_lms;
        let side_rgb = [
            t[0][0] * side[0] + t[1][0] * side[1] + t[2][0] * side[2],
            t[0][1] * side[0] + t[1][1] * side[1] + t[2][1] * side[2],
            t[0][2] * side[0] + t[1][2] * side[1] + t[2][2] * side[2],
        ];
        Dichromat { side: vec3_f32(side_rgb), near: mat3_f32(&near), far: mat3_f32(&far) }
    }

    fn simulate(&self, srgb: [f32; 3]) -> [f32; 3] {
        let rgb = lab::srgb_linear(srgb);
        let s = self.side;
        let m = if s[0] * rgb[0] + s[1] * rgb[1] + s[2] * rgb[2] >= 0.0 { &self.near } else { &self.far };
        lab::mat_mul(m, rgb).map(|v| lab::linear_to_srgb(v.clamp(0.0, 1.0)))
    }
}

/// LMS → LMS: keeps L and M and sets S so the result lies in the plane with normal `n`.
const fn keep_lm_on_plane(n: [f64; 3]) -> [[f64; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-n[0] / n[2], -n[1] / n[2], 0.0]]
}

const fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

const fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

const fn mat3_apply(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

const fn mat3_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    let mut i = 0;
    while i < 3 {
        out[i] = [dot(a[i], [b[0][0], b[1][0], b[2][0]]), dot(a[i], [b[0][1], b[1][1], b[2][1]]), dot(a[i], [b[0][2], b[1][2], b[2][2]])];
        i += 1;
    }
    out
}

const fn mat3_inverse(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c0 = cross(m[1], m[2]);
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    let det = dot(m[0], c0);
    [[c0[0] / det, c1[0] / det, c2[0] / det], [c0[1] / det, c1[1] / det, c2[1] / det], [c0[2] / det, c1[2] / det, c2[2] / det]]
}

const fn vec3_f32(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

const fn mat3_f32(m: &[[f64; 3]; 3]) -> [[f32; 3]; 3] {
    [vec3_f32(m[0]), vec3_f32(m[1]), vec3_f32(m[2])]
}

/// View → Proof Setup target.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "profile", rename_all = "camelCase")]
pub enum ProofTarget {
    /// Working CMYK.
    #[default]
    WorkingCmyk,
    /// A specific CMYK profile (Customize…).
    Cmyk(String),
    /// A specific RGB profile (Customize…): the color as that RGB device shows it.
    Rgb(String),
    /// Legacy Macintosh RGB (Gamma 1.8).
    LegacyMacRgb,
    /// Internet Standard RGB (sRGB).
    Srgb,
    /// Monitor RGB (no simulation).
    MonitorRgb,
    /// Colour blindness — protanopia type.
    Protanopia,
    /// Colour blindness — deuteranopia type.
    Deuteranopia,
    /// Color blindness, tritanopia type.
    Tritanopia,
}

impl ProofTarget {
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(p) = s.strip_prefix("cmyk:") {
            return Some(ProofTarget::Cmyk(p.to_string()));
        }
        if let Some(p) = s.strip_prefix("rgb:") {
            return Some(ProofTarget::Rgb(p.to_string()));
        }
        Some(match s {
            "workingCmyk" | "cmyk" => ProofTarget::WorkingCmyk,
            "legacyMacRgb" | "macRgb" => ProofTarget::LegacyMacRgb,
            "srgb" | "sRGB" => ProofTarget::Srgb,
            "monitorRgb" | "monitor" => ProofTarget::MonitorRgb,
            "protanopia" => ProofTarget::Protanopia,
            "deuteranopia" => ProofTarget::Deuteranopia,
            "tritanopia" => ProofTarget::Tritanopia,
            _ => return None,
        })
    }
    pub fn id(&self) -> String {
        match self {
            ProofTarget::WorkingCmyk => "workingCmyk".into(),
            ProofTarget::Cmyk(p) => format!("cmyk:{p}"),
            ProofTarget::Rgb(p) => format!("rgb:{p}"),
            ProofTarget::LegacyMacRgb => "legacyMacRgb".into(),
            ProofTarget::Srgb => "srgb".into(),
            ProofTarget::MonitorRgb => "monitorRgb".into(),
            ProofTarget::Protanopia => "protanopia".into(),
            ProofTarget::Deuteranopia => "deuteranopia".into(),
            ProofTarget::Tritanopia => "tritanopia".into(),
        }
    }
    /// Whether the target simulates a CMYK press (separations make sense).
    pub fn is_cmyk(&self) -> bool {
        matches!(self, ProofTarget::WorkingCmyk | ProofTarget::Cmyk(_))
    }
}

/// View → Proof Setup / Proof Colors / Separations Preview state for rendering.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofSetup {
    pub target: ProofTarget,
    pub intent: Intent,
    /// Simulate Paper Color (absolute colorimetric display of the press).
    #[serde(default)]
    pub simulate_paper: bool,
    /// Preserve CMYK Numbers (a CMYK device) or Preserve RGB Numbers (an RGB device): the colors'
    /// numbers go to the device unchanged instead of being converted into its space with
    /// `intent`.
    #[serde(default)]
    pub preserve_numbers: bool,
    /// Simulate Black Ink: the press black shows as the dark gray it prints (a CMYK device; implied
    /// by `simulate_paper`).
    #[serde(default)]
    pub simulate_black: bool,
    /// Separations Preview: names of the visible plates ("Cyan", "Magenta", "Yellow", "Black",
    /// spot swatch names). `None` = normal composite proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separations: Option<Vec<String>>,
}

/// Process plate names in order.
pub const PROCESS_PLATES: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];

const LUT_N: usize = 17;

/// A 17³ RGB → RGB lookup table with trilinear interpolation.
pub struct ProofLut {
    data: Vec<[f32; 3]>,
}

impl ProofLut {
    pub fn build(f: impl Fn([f32; 3]) -> [f32; 3]) -> Self {
        let n = LUT_N;
        let mut data = Vec::with_capacity(n * n * n);
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    let s = (n - 1) as f32;
                    data.push(f([r as f32 / s, g as f32 / s, b as f32 / s]));
                }
            }
        }
        Self { data }
    }
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = LUT_N;
        let s = (n - 1) as f32;
        let p = rgb.map(|v| v.clamp(0.0, 1.0) * s);
        let i = p.map(|v| (v.floor() as usize).min(n - 2));
        let t = [p[0] - i[0] as f32, p[1] - i[1] as f32, p[2] - i[2] as f32];
        let at = |r: usize, g: usize, b: usize| self.data[(r * n + g) * n + b];
        let mut out = [0.0f32; 3];
        for (dr, wr) in [(0, 1.0 - t[0]), (1, t[0])] {
            for (dg, wg) in [(0, 1.0 - t[1]), (1, t[1])] {
                for (db, wb) in [(0, 1.0 - t[2]), (1, t[2])] {
                    let w = wr * wg * wb;
                    if w == 0.0 {
                        continue;
                    }
                    let v = at(i[0] + dr, i[1] + dg, i[2] + db);
                    for c in 0..3 {
                        out[c] += w * v[c];
                    }
                }
            }
        }
        out
    }
    pub fn apply8(&self, rgb: [u8; 3]) -> [u8; 3] {
        self.apply(rgb.map(|v| v as f32 / 255.0)).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

/// A 17⁴ CMYK → four values lookup table with quadrilinear interpolation: ink amounts to a
/// colour (XYZ, RGB, or CMYK in another space).
pub struct CmykLut {
    data: Vec<[f32; 4]>,
}

impl CmykLut {
    pub fn build(f: impl Fn([f32; 4]) -> [f32; 4]) -> Self {
        let s = (LUT_N - 1) as f32;
        let mut data = Vec::with_capacity(LUT_N.pow(4));
        for c in 0..LUT_N {
            for m in 0..LUT_N {
                for y in 0..LUT_N {
                    for k in 0..LUT_N {
                        data.push(f([c, m, y, k].map(|i| i as f32 / s)));
                    }
                }
            }
        }
        Self { data }
    }

    pub fn apply(&self, inks: [f32; 4]) -> [f32; 4] {
        const STRIDE: [usize; 4] = [LUT_N * LUT_N * LUT_N, LUT_N * LUT_N, LUT_N, 1];
        let p = inks.map(|v| v.clamp(0.0, 1.0) * (LUT_N - 1) as f32);
        let i = p.map(|v| (v as usize).min(LUT_N - 2));
        let base: usize = (0..4).map(|d| i[d] * STRIDE[d]).sum();
        let mut out = [0.0f32; 4];
        for corner in 0..16 {
            let (mut w, mut at) = (1.0f32, base);
            for d in 0..4 {
                let t = p[d] - i[d] as f32;
                if corner >> d & 1 == 1 {
                    w *= t;
                    at += STRIDE[d];
                } else {
                    w *= 1.0 - t;
                }
            }
            if w > 0.0 {
                for (o, v) in out.iter_mut().zip(self.data[at]) {
                    *o += w * v;
                }
            }
        }
        out
    }

    /// [`Self::apply`] to 8-bit ink amounts, for 8-bit values.
    pub fn apply8(&self, inks: [u8; 4]) -> [u8; 4] {
        self.apply(inks.map(|v| v as f32 / 255.0)).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

// ---------- the active (process-wide) settings ----------

static ACTIVE: RwLock<Option<Arc<Cms>>> = RwLock::new(None);
static WORKING: RwLock<Option<Arc<Cms>>> = RwLock::new(None);
static RGB_IS_SRGB: AtomicBool = AtomicBool::new(true);
static CMYK_IS_DEVICE: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The settings [`with_active`] put in place of the active settings on this thread.
    static SCOPED: RefCell<Option<Arc<Cms>>> = const { RefCell::new(None) };
    /// [`SCOPED`]'s flags for the fast paths: [`SCOPED_SET`] while it is set, [`SCOPED_SRGB`] when
    /// its RGB space is sRGB, and [`SCOPED_DEVICE`] when its CMYK space is device CMYK.
    static SCOPED_FLAGS: Cell<u8> = const { Cell::new(0) };
}
const SCOPED_SET: u8 = 1;
const SCOPED_SRGB: u8 = 2;
const SCOPED_DEVICE: u8 = 4;

/// The active color-management context: the working settings, with the profiles
/// [`use_document_profiles`] last put in place of the working spaces. Inside [`with_active`] on
/// this thread: the settings it was given.
pub fn active() -> Arc<Cms> {
    if SCOPED_FLAGS.with(Cell::get) & SCOPED_SET != 0
        && let Some(c) = SCOPED.with(|s| s.borrow().clone())
    {
        return c;
    }
    installed()
}

/// The active settings of the process, as [`use_document_profiles`] or [`set_active`] last put them
/// in place.
fn installed() -> Arc<Cms> {
    if let Some(c) = ACTIVE.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return c.clone();
    }
    let mut w = ACTIVE.write().unwrap_or_else(|e| e.into_inner());
    w.get_or_insert_with(|| Arc::new(Cms::default())).clone()
}

pub fn active_settings() -> ColorSettings {
    active().settings.clone()
}

/// The color-management context of Edit → Color Settings (the working spaces, intent and black
/// point compensation), as [`set_active`] last set them, whatever profiles a document is assigned.
pub fn working() -> Arc<Cms> {
    if let Some(c) = WORKING.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return c.clone();
    }
    let mut w = WORKING.write().unwrap_or_else(|e| e.into_inner());
    w.get_or_insert_with(|| Arc::new(Cms::default())).clone()
}

/// The settings of [`working`].
pub fn working_settings() -> ColorSettings {
    working().settings.clone()
}

/// Replace the working settings (Edit → Color Settings) and make them the active settings (fails,
/// leaving both unchanged, if a profile is unknown or the RGB policy is Preserve Numbers).
pub fn set_active(settings: &ColorSettings) -> Result<(), CmsError> {
    if !Policy::RGB.contains(&settings.rgb_policy) {
        return Err(CmsError::Unsupported(format!("the RGB policy can't be {}", settings.rgb_policy.id())));
    }
    let cms = Arc::new(Cms::new(settings)?);
    *WORKING.write().unwrap_or_else(|e| e.into_inner()) = Some(cms.clone());
    install(cms);
    Ok(())
}

/// Make the working settings the active settings, with the profiles a document is assigned
/// (`None`: none) in place of the working spaces. An assigned profile that is not available, or
/// is not of its model, leaves the working space. The working settings stay as they are. True when
/// the active settings changed.
pub fn use_document_profiles(rgb: Option<&str>, cmyk: Option<&str>) -> bool {
    let working = working();
    let st = document_settings(&working.settings, rgb, cmyk);
    if st == installed().settings {
        return false;
    }
    if st == working.settings {
        install(working);
        return true;
    }
    // The working settings were checked when they were set; a working profile removed since then
    // leaves the active settings as they are.
    let Ok(mut cms) = Cms::new(&st) else { return false };
    cms.working = Some(working);
    install(Arc::new(cms));
    true
}

/// The working settings `working` with the profiles a document is assigned (`None`: none) in place
/// of the working spaces, as [`use_document_profiles`] takes them.
fn document_settings(working: &ColorSettings, rgb: Option<&str>, cmyk: Option<&str>) -> ColorSettings {
    let mut st = working.clone();
    if let Some(p) = rgb.and_then(profile).filter(|p| p.kind == ProfileKind::Rgb) {
        st.rgb = p.name;
    }
    if let Some(p) = cmyk.and_then(profile).filter(|p| p.kind == ProfileKind::Cmyk) {
        st.cmyk = p.name;
    }
    st
}

/// How many contexts [`for_document`] keeps.
const DOCUMENT_CONTEXTS: usize = 8;

/// The color-management context a document assigned the profiles `rgb` and `cmyk` (`None`: none)
/// draws through: the one [`use_document_profiles`] would make active for it. A context that is the
/// active or the working one is that one; the others are kept for the same working settings and
/// loaded profiles, so the same document gets the same context again. A profile that cannot be
/// loaded gives the working settings.
pub fn for_document(rgb: Option<&str>, cmyk: Option<&str>) -> Arc<Cms> {
    static KEPT: Mutex<Vec<(u64, Arc<Cms>)>> = Mutex::new(Vec::new());
    let working = working();
    let st = document_settings(&working.settings, rgb, cmyk);
    let installed = installed();
    if st == installed.settings {
        return installed;
    }
    if st == working.settings {
        return working;
    }
    let generation = profiles_generation();
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|(g, c)| *g == generation && c.working.as_ref().is_some_and(|w| Arc::ptr_eq(w, &working)));
    if let Some((_, c)) = kept.iter().find(|(_, c)| c.settings == st) {
        return c.clone();
    }
    let Ok(mut cms) = Cms::new(&st) else { return working };
    cms.working = Some(working);
    let cms = Arc::new(cms);
    if kept.len() >= DOCUMENT_CONTEXTS {
        kept.remove(0);
    }
    kept.push((generation, cms.clone()));
    cms
}

/// Run `f` with `cms` as the active settings on this thread ([`active`], [`crate::Color::to_rgb`]
/// and the conversions that read them), as when drawing a document other than the front one
/// through its own profiles. Other threads, and this thread after `f`, keep the settings they had.
pub fn with_active<R>(cms: Arc<Cms>, f: impl FnOnce() -> R) -> R {
    /// Puts back the settings this thread had before, when `f` returns or unwinds.
    struct Restore(Option<Arc<Cms>>, u8);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPED.with(|s| *s.borrow_mut() = self.0.take());
            SCOPED_FLAGS.with(|c| c.set(self.1));
        }
    }
    let flags = SCOPED_SET | if cms.rgb_is_srgb() { SCOPED_SRGB } else { 0 } | if matches!(cms.cmyk, CmykSpace::Device) { SCOPED_DEVICE } else { 0 };
    let before = SCOPED.with(|s| s.borrow_mut().replace(cms));
    let _restore = Restore(before, SCOPED_FLAGS.with(|c| c.replace(flags)));
    f()
}

fn install(cms: Arc<Cms>) {
    RGB_IS_SRGB.store(cms.rgb_is_srgb(), Ordering::Relaxed);
    CMYK_IS_DEVICE.store(matches!(cms.cmyk, CmykSpace::Device), Ordering::Relaxed);
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(cms);
}

/// Fast path for [`Color::to_rgb`]: RGB colours need no conversion while the working space is sRGB.
pub(crate) fn rgb_is_srgb() -> bool {
    match SCOPED_FLAGS.with(Cell::get) {
        f if f & SCOPED_SET != 0 => f & SCOPED_SRGB != 0,
        _ => RGB_IS_SRGB.load(Ordering::Relaxed),
    }
}
pub(crate) fn cmyk_is_device() -> bool {
    match SCOPED_FLAGS.with(Cell::get) {
        f if f & SCOPED_SET != 0 => f & SCOPED_DEVICE != 0,
        _ => CMYK_IS_DEVICE.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_icc;
#[cfg(test)]
mod tests_measured;
