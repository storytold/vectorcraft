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

/// Blend panel Type and Ramp. Ramp is a fraction from 0 (even) to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendEase {
    #[serde(default)]
    pub kind: BlendEaseType,
    #[serde(default, deserialize_with = "blend_ramp")]
    pub ramp: f64,
}

impl BlendEase {
    pub fn is_linear(&self) -> bool {
        self.ramp <= 0.0
    }

    /// Only the default Type and Ramp can be omitted without losing a chosen Type at Ramp 0.
    pub fn is_default(&self) -> bool {
        self.kind == BlendEaseType::In && self.ramp == 0.0
    }

    /// A monotone polynomial distribution, blended with even spacing by Ramp. Public help
    /// describes the four distributions, not their numeric curves; these curves are original.
    pub fn apply(&self, u: f64) -> f64 {
        let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        let r = if self.ramp.is_finite() { self.ramp.clamp(0.0, 1.0) } else { 0.0 };
        let eased = match self.kind {
            BlendEaseType::In => u * u,
            BlendEaseType::Out => u * (2.0 - u),
            BlendEaseType::InOut => u * u * (3.0 - 2.0 * u),
            BlendEaseType::OutIn if u < 0.5 => 2.0 * u * (1.0 - u),
            BlendEaseType::OutIn => 0.5 + 2.0 * (u - 0.5) * (u - 0.5),
        };
        u + r * (eased - u)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendSpec {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
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
}

/// Blend Options set with no blend selected: what new blends start with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendDefaults {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
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
