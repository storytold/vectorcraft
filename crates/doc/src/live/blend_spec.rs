use super::*;

// =====================================================================================
// Data
// =====================================================================================

/// Blend Options → Spacing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendSpacing {
    /// Step count chosen from the colour difference (or the distance when colours match).
    #[default]
    SmoothColor,
    /// Specified number of steps between each pair of key objects (1–1000).
    Steps(u32),
    /// Specified distance (pt) between the steps.
    Distance(f64),
}

/// Blend Options → Orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendOrientation {
    #[default]
    AlignToPage,
    AlignToPath,
}

/// Blend panel → Spine: when the blend follows and shows its spine.
///
/// `Auto` follows a replaced spine, or a straight one when the steps align to the path; `Always`
/// always follows and shows one (so easing works on a straight blend too); `Never` leaves the
/// spine alone: no distribution along it, no spine or easing handles on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendSpineMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl BlendSpineMode {
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    pub fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

/// Distribution of blend steps, as offered by the Blend panel's Type menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendEaseType {
    #[default]
    In,
    Out,
    InOut,
    OutIn,
}

impl BlendEaseType {
    pub fn id(self) -> &'static str {
        match self {
            Self::In => "easeIn",
            Self::Out => "easeOut",
            Self::InOut => "easeInOut",
            Self::OutIn => "easeOutIn",
        }
    }
}

/// Blend panel Type and Ramp, with the midpoint the 50% step sits at. Ramp is a fraction from 0
/// (even) to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendEase {
    #[serde(default)]
    pub kind: BlendEaseType,
    #[serde(default, deserialize_with = "blend_ramp")]
    pub ramp: f64,
    /// Where the blend's halfway step sits along a pair of keys (0.05-0.95, 0.5 at the middle):
    /// the on-canvas Ease knob's place.
    #[serde(default = "half", deserialize_with = "blend_midpoint", skip_serializing_if = "midpoint_is_half")]
    pub midpoint: f64,
}

/// The midpoint every blend starts with.
fn half() -> f64 {
    0.5
}

fn midpoint_is_half(m: &f64) -> bool {
    (*m - 0.5).abs() < 1e-9
}

fn blend_midpoint<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|n| n.is_finite()).map_or(0.5, |n| n.clamp(0.05, 0.95)))
}

impl Default for BlendEase {
    fn default() -> Self {
        Self { kind: BlendEaseType::In, ramp: 0.0, midpoint: 0.5 }
    }
}

impl BlendEase {
    pub fn is_linear(&self) -> bool {
        self.ramp <= 0.0
    }

    /// Only the default Type, Ramp and midpoint can be omitted without losing a chosen Type at
    /// Ramp 0.
    pub fn is_default(&self) -> bool {
        self.kind == BlendEaseType::In && self.ramp == 0.0 && self.midpoint == 0.5
    }

    /// A monotone polynomial distribution on each side of the midpoint, blended with even spacing
    /// by Ramp, so `apply(0.5)` is the midpoint (where the Ease knob sits) and the ends stay put.
    /// Public help describes the four distributions, not their numeric curves; these curves are
    /// original.
    pub fn apply(&self, u: f64) -> f64 {
        let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        let r = if self.ramp.is_finite() { self.ramp.clamp(0.0, 1.0) } else { 0.0 };
        let m = if self.midpoint.is_finite() { self.midpoint.clamp(0.05, 0.95) } else { 0.5 };
        let half = |v: f64| {
            let eased = match self.kind {
                BlendEaseType::In => v * v,
                BlendEaseType::Out => v * (2.0 - v),
                BlendEaseType::InOut => v * v * (3.0 - 2.0 * v),
                BlendEaseType::OutIn if v < 0.5 => 2.0 * v * (1.0 - v),
                BlendEaseType::OutIn => 0.5 + 2.0 * (v - 0.5) * (v - 0.5),
            };
            v + r * (eased - v)
        };
        if u <= 0.5 { m * half(u / 0.5) } else { m + (1.0 - m) * half((u - 0.5) / 0.5) }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendSpec {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
    /// When the blend follows and shows its spine (Auto, Always, Never).
    #[serde(default, rename = "spineMode", skip_serializing_if = "BlendSpineMode::is_auto")]
    pub spine_mode: BlendSpineMode,
    /// Acceleration of the steps' shapes and positions along the spine between two keys.
    #[serde(default, skip_serializing_if = "BlendEase::is_default")]
    pub ease: BlendEase,
    /// Independent color acceleration. -100..100, with 0 preserving the even color ramp.
    #[serde(default, rename = "colorShift", skip_serializing_if = "is_zero", deserialize_with = "blend_color_shift")]
    pub color_shift: f64,
    /// Replaced spine. `None` = the straight lines between the key objects' centres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spine: Option<PathData>,
    /// The anchor of the spine's first subpath each key object sits on (a spine edited on the
    /// canvas: moving a key moves its anchor and the other way round). Empty: the keys spread
    /// evenly along the spine by arc length (Replace Spine).
    #[serde(default, rename = "keyAnchors", skip_serializing_if = "Vec::is_empty")]
    pub key_anchors: Vec<u32>,
    /// Per key object, the anchor of its first subpath the blend starts from (Blend tool clicks
    /// on anchor points); `None`: chosen so closed shapes don't twist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<Option<u32>>,
}

impl BlendSpec {
    /// The start anchor of key `i` (see [`Self::starts`]).
    pub fn start(&self, i: usize) -> Option<usize> {
        self.starts.get(i).copied().flatten().map(|a| a as usize)
    }

    /// Whether the blend shows its spine and the Pen can edit it: a spine always exists (a stored
    /// one or the lines between the keys), and only Spine → Never hides it.
    pub fn spine_visible(&self) -> bool {
        self.spine_mode != BlendSpineMode::Never
    }

    /// Whether the blend runs its steps along the spine: with `Always` the straight lines between
    /// the keys count as one; with `Auto` only a replaced spine or Align to Path does. The easing
    /// (Type/Ramp) moves the steps between two keys in every mode; this only decides the path.
    pub fn spine_followed(&self) -> bool {
        match self.spine_mode {
            BlendSpineMode::Auto => self.spine.is_some() || self.orientation == BlendOrientation::AlignToPath,
            BlendSpineMode::Always => true,
            BlendSpineMode::Never => false,
        }
    }
}

/// Blend Options set with no blend selected: what new blends start with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendDefaults {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
    #[serde(default, rename = "spineMode")]
    pub spine_mode: BlendSpineMode,
}

/// Reading a native file never fails over one bad blend number: out-of-range values clamp,
/// and anything that is not a finite number (NaN, null, text) becomes the default. The
/// commands still refuse such input (`ease_param`).
fn blend_number<'de, D: serde::Deserializer<'de>>(d: D, min: f64, max: f64) -> Result<f64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|n| n.is_finite()).map_or(0.0, |n| n.clamp(min, max)))
}

pub(super) fn blend_ramp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    blend_number(d, 0.0, 1.0)
}

pub(super) fn blend_color_shift<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    blend_number(d, -100.0, 100.0)
}

/// Whether an optional stored color shift has its unchanged default.
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl BlendSpec {
    /// Shift the color transition independently of shape placement. The exponential curve is
    /// original; the documented behavior specifies the direction, not numeric sample values.
    pub fn color_position(&self, u: f64) -> f64 {
        let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        let shift = if self.color_shift.is_finite() { self.color_shift.clamp(-100.0, 100.0) } else { 0.0 };
        if shift == 0.0 { u } else { u.powf(2.0f64.powf(shift / 50.0)) }
    }
}
