use super::para_style::FirstBaseline;
use super::*;

/// Area Type Options "Align" (vertical): where the lines of each row/column sit in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerticalAlign {
    /// Lines start at the top of the cell (the default).
    #[default]
    Top,
    /// The block of lines is centred in the cell.
    Center,
    /// The last line's descent touches the cell bottom.
    Bottom,
    /// The first line stays at the top, the last one moves to the bottom and the space left over
    /// is shared equally between the lines (no paragraph spacing limit).
    Justify,
}

impl VerticalAlign {
    pub const ALL: [VerticalAlign; 4] = [VerticalAlign::Top, VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify];
    pub fn id(self) -> &'static str {
        match self {
            VerticalAlign::Top => "top",
            VerticalAlign::Center => "center",
            VerticalAlign::Bottom => "bottom",
            VerticalAlign::Justify => "justify",
        }
    }
}

/// Text Wrap Options of a wrap object (Object → Text Wrap).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextWrap {
    /// Distance kept between the text and the object, in points (Illustrator's default 6 pt).
    pub offset: f64,
    /// Invert Wrap: text flows inside the object instead of around it.
    pub invert: bool,
}

impl Default for TextWrap {
    fn default() -> Self {
        Self { offset: 6.0, invert: false }
    }
}

/// A resolved wrap shape on an area text object: the outline of a wrap object in text space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WrapShape {
    pub path: PathData,
    #[serde(flatten)]
    pub wrap: TextWrap,
}

/// Type on a Path effect: how each glyph is oriented on the path (Type → Type on a Path).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathEffect {
    /// Glyphs rotate with the path (the default).
    #[default]
    Rainbow,
    /// Vertical edges stay vertical; the baseline follows the path.
    Skew,
    /// Horizontal edges stay horizontal; vertical edges are perpendicular to the path.
    #[serde(rename = "3dRibbon")]
    Ribbon3d,
    /// No rotation: the left end of each glyph's baseline sits on the path.
    StairStep,
    /// The baseline centre sits on the path and glyphs point away from the path's centre.
    Gravity,
}

impl PathEffect {
    pub const ALL: [PathEffect; 5] = [PathEffect::Rainbow, PathEffect::Skew, PathEffect::Ribbon3d, PathEffect::StairStep, PathEffect::Gravity];
    pub fn id(self) -> &'static str {
        match self {
            PathEffect::Rainbow => "rainbow",
            PathEffect::Skew => "skew",
            PathEffect::Ribbon3d => "3dRibbon",
            PathEffect::StairStep => "stairStep",
            PathEffect::Gravity => "gravity",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        let k = s.to_ascii_lowercase().replace([' ', '-', '_'], "");
        Self::ALL.into_iter().find(|e| e.id().to_ascii_lowercase() == k || (k == "ribbon3d" && *e == PathEffect::Ribbon3d))
    }
}

/// Type on a Path Options › Align to Path: which height of the type runs along the path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathAlign {
    /// The font's top edge: the type hangs below the path.
    Ascender,
    /// The font's bottom edge: the type stands above the path.
    Descender,
    /// Halfway between the ascender and the descender.
    Center,
    /// The baseline (the default).
    #[default]
    Baseline,
}

impl PathAlign {
    pub const ALL: [PathAlign; 4] = [PathAlign::Ascender, PathAlign::Descender, PathAlign::Center, PathAlign::Baseline];
    pub fn id(self) -> &'static str {
        match self {
            PathAlign::Ascender => "ascender",
            PathAlign::Descender => "descender",
            PathAlign::Center => "center",
            PathAlign::Baseline => "baseline",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(s.trim()))
    }
}

/// Area Type Options: rows and columns, gutters, inset and first baseline of area type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AreaOptions {
    #[serde(deserialize_with = "de_rows_or_columns")]
    pub rows: usize,
    #[serde(deserialize_with = "de_rows_or_columns")]
    pub columns: usize,
    /// Gutter between rows/columns in points.
    pub gutter: f64,
    /// Inset from the frame edges in points.
    pub inset: f64,
    pub first_baseline: FirstBaseline,
    /// Minimum first-baseline offset in points.
    pub first_baseline_min: f64,
    /// Vertical alignment of the lines in each row/column.
    pub vertical_align: VerticalAlign,
    /// How the frame and its text fit each other (Auto Size, Shrink Text to Fit).
    #[serde(skip_serializing_if = "crate::skip::is_default")]
    pub fit: AreaFit,
}

impl Default for AreaOptions {
    fn default() -> Self {
        Self {
            rows: 1,
            columns: 1,
            gutter: 18.0,
            inset: 0.0,
            first_baseline: FirstBaseline::Ascent,
            first_baseline_min: 0.0,
            vertical_align: VerticalAlign::Top,
            fit: AreaFit::None,
        }
    }
}

impl AreaOptions {
    /// The most rows, and the most columns, area type can have.
    pub const MAX_ROWS_COLUMNS: usize = 100;
}

/// [`AreaOptions::rows`] or [`AreaOptions::columns`], read as a `u64` and clamped to
/// 1..=[`AreaOptions::MAX_ROWS_COLUMNS`].
fn de_rows_or_columns<'de, D: serde::Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    let n = usize::try_from(u64::deserialize(d)?).unwrap_or(usize::MAX);
    Ok(n.clamp(1, AreaOptions::MAX_ROWS_COLUMNS))
}

/// How area type and its frame fit each other. Serialized as `"none"`, `"autoHeight"` or
/// `{"shrinkText": {"minPercent": 50}}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AreaFit {
    /// The frame keeps its size; text that doesn't fit overflows.
    #[default]
    None,
    /// The frame's height follows the text (Illustrator's Auto Size): resolved by the engine after
    /// each edit, for rectangular frames of horizontal type in one row.
    AutoHeight,
    /// Text that overflows is scaled down (size, leading and baseline shift; not paragraph
    /// spacing) by the largest factor down to `min_percent` % that makes it fit, at layout time.
    ShrinkText {
        #[serde(rename = "minPercent", default = "AreaFit::default_min_percent")]
        min_percent: f64,
    },
}

impl AreaFit {
    /// The smallest Shrink Text to Fit percentage allowed.
    pub const MIN_PERCENT: f64 = 10.0;
    /// The Shrink Text to Fit percentage new settings start with.
    pub const DEFAULT_MIN_PERCENT: f64 = 50.0;
    fn default_min_percent() -> f64 {
        Self::DEFAULT_MIN_PERCENT
    }
    /// Its id: `none`, `autoHeight` or `shrinkText`.
    pub fn id(self) -> &'static str {
        match self {
            AreaFit::None => "none",
            AreaFit::AutoHeight => "autoHeight",
            AreaFit::ShrinkText { .. } => "shrinkText",
        }
    }
    /// The fit an id names (case, spaces, dashes and underscores ignored), with `min_percent`
    /// for Shrink Text (clamped to 10..100; a non-finite value gives the default).
    pub fn parse(id: &str, min_percent: Option<f64>) -> Option<Self> {
        match id.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "none" | "off" => Some(AreaFit::None),
            "autoheight" | "autosize" => Some(AreaFit::AutoHeight),
            "shrinktext" | "shrinktexttofit" | "shrink" => {
                Some(AreaFit::ShrinkText { min_percent: Self::clamp_percent(min_percent.unwrap_or(Self::DEFAULT_MIN_PERCENT)) })
            }
            _ => None,
        }
    }
    /// A Shrink Text minimum percentage within 10..100 (the default when not finite).
    pub fn clamp_percent(p: f64) -> f64 {
        if p.is_finite() { p.clamp(Self::MIN_PERCENT, 100.0) } else { Self::DEFAULT_MIN_PERCENT }
    }
    /// Shrink Text's minimum scale factor (0.1..1), if this is Shrink Text.
    pub fn min_scale(self) -> Option<f64> {
        match self {
            AreaFit::ShrinkText { min_percent } => Some(Self::clamp_percent(min_percent) / 100.0),
            _ => None,
        }
    }
}
