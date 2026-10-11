//! Text objects (model only; layout and glyph outlines live in `vectorcraft-text`).

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Affine, PathData, Point, Rect, Vec2};

use crate::appearance::{Appearance, AppearanceItem, Dash, FillLayer, LineCap, LineJoin, StrokeLayer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Justify {
    /// Align to the start of each paragraph's direction ([`ParaStyle::direction`]): left for
    /// left-to-right paragraphs, right for right-to-left ones. New type's alignment.
    Auto,
    #[default]
    Left,
    Center,
    Right,
    JustifyLeft,
    JustifyCenter,
    JustifyRight,
    JustifyAll,
}

/// Character attributes (the Character panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharStyle {
    pub font_family: String,
    #[serde(default = "regular")]
    pub font_style: String,
    /// Which installed version of the font (its version string) when several versions of the
    /// family and style are installed and the text was set in one the family and style alone
    /// wouldn't pick (an imported file's type matched to it); None = the one they pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_version: Option<String>,
    /// Size in points.
    pub size: f64,
    /// Leading in points; None = Auto (120% of size).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leading: Option<f64>,
    /// Tracking in 1/1000 em.
    #[serde(default)]
    pub tracking: f64,
    /// Kerning: None = Auto (metrics), Some(v) = manual in 1/1000 em.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kerning: Option<f64>,
    #[serde(default)]
    pub baseline_shift: f64,
    #[serde(default = "hundred")]
    pub h_scale: f64,
    #[serde(default = "hundred")]
    pub v_scale: f64,
    #[serde(default)]
    pub rotation: f64,
    pub fill: Paint,
    #[serde(default)]
    pub stroke: Paint,
    #[serde(default)]
    pub stroke_width: f64,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub strikethrough: bool,
    #[serde(default)]
    pub all_caps: bool,
    /// OpenType features that differ from the defaults (OpenType panel), as tags: `"dlig"` turns
    /// a feature on, `"-liga"` off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// Character style (Character Styles panel) these attributes come from; None = Normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_name: Option<String>,
    /// Overprint Fill / Overprint Stroke of these characters (see [`crate::FillLayer::overprint`]).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub overprint_fill: bool,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub overprint_stroke: bool,
    /// Cap, join, miter limit and dashes of the character stroke (Stroke panel, with type
    /// selected); defaults as for object strokes.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub stroke_cap: LineCap,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub stroke_join: LineJoin,
    #[serde(default = "ten", skip_serializing_if = "is_ten")]
    pub stroke_miter_limit: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_dash: Option<Dash>,
    /// Superscript or subscript (Character panel).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub position: CharPosition,
    /// Small Caps (Character panel): lowercase letters drawn as capitals at this percentage of the
    /// size (Document Setup → Type → Small Caps); None = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_caps: Option<f64>,
    /// Character Alignment (Character panel menu): where characters smaller than the largest on
    /// their line line up with it.
    #[serde(default, rename = "charAlign", skip_serializing_if = "crate::skip::is_default")]
    pub char_align: CharAlign,
    /// Proportional Metrics (Character panel menu, East Asian options): full-width glyphs take the
    /// font's own proportional widths, OpenType `palt` (#966); in vertical type upright glyphs take
    /// its proportional heights, `vpal`.
    #[serde(default, rename = "proportionalMetrics", skip_serializing_if = "crate::skip::is_default")]
    pub proportional_metrics: bool,
}

/// Where a character smaller than the largest on its line lines up with it: on the Roman
/// baseline, at the top (right, in vertical type), centre or bottom (left) of the ideographic
/// em boxes, or at the top (right) or bottom (left) of the ideographic character faces (ICF, the
/// average box of the ideographs inside the em box: OpenType's `icft` and `icfb` baselines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CharAlign {
    #[default]
    RomanBaseline,
    EmBoxTop,
    EmBoxCenter,
    EmBoxBottom,
    IcfTop,
    IcfBottom,
}

/// Superscript or subscript proportions in percent of the font size (Document Setup → Type).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptMetrics {
    /// Glyph size.
    pub size: f64,
    /// Baseline offset: up for superscript, down for subscript.
    pub position: f64,
}

impl ScriptMetrics {
    pub const DEFAULT: Self = Self { size: 58.3, position: 33.3 };
}

impl Default for ScriptMetrics {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Character position (Character panel Superscript / Subscript). The proportions are the
/// document's when the position is applied, and follow later Document Setup changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CharPosition {
    #[default]
    Normal,
    Superscript(ScriptMetrics),
    Subscript(ScriptMetrics),
}

impl CharPosition {
    /// Glyph scale and baseline shift (points, positive = up) for text of `size` points.
    pub fn scale_shift(self, size: f64) -> (f64, f64) {
        match self {
            CharPosition::Normal => (1.0, 0.0),
            CharPosition::Superscript(m) => (m.size / 100.0, m.position / 100.0 * size),
            CharPosition::Subscript(m) => (m.size / 100.0, -m.position / 100.0 * size),
        }
    }
    /// `normal`, `superscript` or `subscript`.
    pub fn id(self) -> &'static str {
        match self {
            CharPosition::Normal => "normal",
            CharPosition::Superscript(_) => "superscript",
            CharPosition::Subscript(_) => "subscript",
        }
    }
}

fn ten() -> f64 {
    10.0
}
fn is_ten(v: &f64) -> bool {
    *v == 10.0
}
fn regular() -> String {
    "Regular".into()
}
fn hundred() -> f64 {
    100.0
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            font_family: "Source Sans 3".into(),
            font_style: "Regular".into(),
            font_version: None,
            size: 12.0,
            leading: None,
            tracking: 0.0,
            kerning: None,
            baseline_shift: 0.0,
            h_scale: 100.0,
            v_scale: 100.0,
            rotation: 0.0,
            fill: Paint::solid(Color::BLACK),
            stroke: Paint::None,
            stroke_width: 0.0,
            underline: false,
            strikethrough: false,
            all_caps: false,
            features: vec![],
            style_name: None,
            overprint_fill: false,
            overprint_stroke: false,
            stroke_cap: LineCap::Butt,
            stroke_join: LineJoin::Miter,
            stroke_miter_limit: 10.0,
            stroke_dash: None,
            position: CharPosition::Normal,
            char_align: CharAlign::RomanBaseline,
            proportional_metrics: false,
            small_caps: None,
        }
    }
}

impl CharStyle {
    pub fn effective_leading(&self) -> f64 {
        self.leading.unwrap_or(self.size * 1.2)
    }
    /// Do the characters draw a stroke (a paint and a positive weight)?
    pub fn has_stroke(&self) -> bool {
        !self.stroke.is_none() && self.stroke_width > 0.0
    }
    /// The character stroke as a stroke layer (paint, weight, cap, join, miter limit, dashes and
    /// overprint), so type strokes share the object strokes' geometry, rendering and export.
    pub fn stroke_layer(&self) -> StrokeLayer {
        StrokeLayer {
            overprint: self.overprint_stroke,
            cap: self.stroke_cap,
            join: self.stroke_join,
            miter_limit: self.stroke_miter_limit,
            dash: self.stroke_dash.clone(),
            ..StrokeLayer::new(self.stroke.clone(), self.stroke_width)
        }
    }
    /// Take the paint, weight, cap, join, miter limit and dashes of `st` as the character stroke
    /// (the options characters have no use for, such as alignment and arrowheads, are dropped).
    pub fn set_stroke_layer(&mut self, st: &StrokeLayer) {
        self.stroke = st.paint.clone();
        self.stroke_width = st.width;
        self.stroke_cap = st.cap;
        self.stroke_join = st.join;
        self.stroke_miter_limit = st.miter_limit;
        self.stroke_dash = st.dash.clone();
    }
    /// The character fill as a fill layer, overprinting as the characters do.
    fn fill_layer(&self) -> FillLayer {
        FillLayer { overprint: self.overprint_fill, ..FillLayer::new(self.fill.clone()) }
    }
    /// The characters' paint as an object appearance (their outlines'): the fill, plus the
    /// stroke when it draws one, overprinting as the characters do.
    pub fn appearance(&self) -> Appearance {
        let mut a = Appearance { items: vec![AppearanceItem::Fill(self.fill_layer())], ..Default::default() };
        if self.has_stroke() {
            a.items.push(AppearanceItem::Stroke(self.stroke_layer()));
        }
        a
    }
    /// The characters' paint as the basic fill and stroke rows (the stroke row even when it has
    /// no paint): the appearance the Eyedropper and graphic styles take from type.
    pub fn basic_appearance(&self) -> Appearance {
        Appearance { items: vec![AppearanceItem::Fill(self.fill_layer()), AppearanceItem::Stroke(self.stroke_layer())], ..Default::default() }
    }
}

/// Tab stop alignment (Tabs panel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabAlign {
    #[default]
    Left,
    Center,
    Right,
    /// Aligns on the first `align_on` character (a decimal point by default).
    Decimal,
}

/// A tab stop, measured from the left edge of the text (area type: the frame's left edge).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabStop {
    pub position: f64,
    #[serde(default)]
    pub align: TabAlign,
    /// Leader characters repeated across the tab's gap (e.g. ". ").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub leader: String,
    /// Decimal tabs align on this character.
    #[serde(default = "default_align_on")]
    pub align_on: char,
}

fn default_align_on() -> char {
    '.'
}

/// Distance between default tab stops when no explicit stop applies (½ inch).
pub const DEFAULT_TAB_INTERVAL: f64 = 36.0;

/// Paragraph composer (Paragraph panel menu): how lines are broken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Composer {
    /// Break each line as soon as it is full.
    SingleLine,
    /// Total fit over the whole paragraph (Illustrator's default): justified lines get even word
    /// spacing, ragged lines an even rag.
    #[default]
    EveryLine,
}

impl Composer {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Paragraph attributes (the Paragraph panel).
///
/// Saved through [`ParaStyleFile`], which keeps files with text openable by builds from before
/// [`Justify::Auto`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "ParaStyleFile", into = "ParaStyleFile")]
pub struct ParaStyle {
    pub justify: Justify,
    pub left_indent: f64,
    pub right_indent: f64,
    pub first_line_indent: f64,
    pub space_before: f64,
    pub space_after: f64,
    pub hyphenate: bool,
    /// Line breaking: Single-line or Every-line Composer.
    pub composer: Composer,
    /// Tab stops (Tabs panel), sorted by position.
    pub tabs: Vec<TabStop>,
    /// Paragraph style (Paragraph Styles panel) these attributes come from; None = Normal.
    pub style_name: Option<String>,
    /// Japanese composition: the spacing of punctuation (Paragraph panel › Mojikumi). Type made
    /// with the Type tools and `text.create` takes [`Mojikumi::LineEndHalf`]; documents from before
    /// it and imported text (already set) keep [`Mojikumi::None`].
    pub mojikumi: Mojikumi,
    /// Paragraph direction (Paragraph panel): the base direction of each paragraph for
    /// bidirectional text (UAX #9). None: from each paragraph's first strong character (Hebrew or
    /// Arabic: right to left).
    pub direction: Option<ParaDirection>,
    /// How leading is measured (Paragraph panel menu): from baseline to baseline, or from the top
    /// of one line's ideographic em box to the next.
    pub leading_model: LeadingModel,
    /// Hanging punctuation (Paragraph panel menu › Burasagari): a comma or full stop ending a line
    /// may stand outside the frame. New type takes [`Burasagari::Standard`]; documents from before
    /// it and imported text keep [`Burasagari::None`].
    pub burasagari: Burasagari,
    /// Kinsoku (Paragraph panel › Kinsoku Set): which Japanese characters may not start or end a
    /// line.
    pub kinsoku: Kinsoku,
}

/// [`ParaStyle`] as saved. [`Justify::Auto`] is written as the alignment it has in the paragraph
/// direction (`Right` for right to left, else `Left`) with `justify_auto` set: builds without Auto
/// read the alignment and ignore the flag, and builds with it read Auto back.
#[derive(Serialize, Deserialize)]
struct ParaStyleFile {
    #[serde(default)]
    justify: Justify,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    justify_auto: bool,
    #[serde(default)]
    left_indent: f64,
    #[serde(default)]
    right_indent: f64,
    #[serde(default)]
    first_line_indent: f64,
    #[serde(default)]
    space_before: f64,
    #[serde(default)]
    space_after: f64,
    #[serde(default)]
    hyphenate: bool,
    #[serde(default, skip_serializing_if = "Composer::is_default")]
    composer: Composer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tabs: Vec<TabStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style_name: Option<String>,
    #[serde(default, skip_serializing_if = "Mojikumi::is_none")]
    mojikumi: Mojikumi,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    direction: Option<ParaDirection>,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    leading_model: LeadingModel,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    burasagari: Burasagari,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    kinsoku: Kinsoku,
}

impl From<ParaStyle> for ParaStyleFile {
    fn from(p: ParaStyle) -> Self {
        let justify_auto = p.justify == Justify::Auto;
        let justify = match p.justify {
            Justify::Auto if p.direction == Some(ParaDirection::RightToLeft) => Justify::Right,
            Justify::Auto => Justify::Left,
            j => j,
        };
        let ParaStyle {
            left_indent,
            right_indent,
            first_line_indent,
            space_before,
            space_after,
            hyphenate,
            composer,
            tabs,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
            ..
        } = p;
        Self {
            justify,
            justify_auto,
            left_indent,
            right_indent,
            first_line_indent,
            space_before,
            space_after,
            hyphenate,
            composer,
            tabs,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
        }
    }
}

impl From<ParaStyleFile> for ParaStyle {
    fn from(f: ParaStyleFile) -> Self {
        let justify = if f.justify_auto { Justify::Auto } else { f.justify };
        let ParaStyleFile {
            left_indent,
            right_indent,
            first_line_indent,
            space_before,
            space_after,
            hyphenate,
            composer,
            tabs,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
            ..
        } = f;
        Self {
            justify,
            left_indent,
            right_indent,
            first_line_indent,
            space_before,
            space_after,
            hyphenate,
            composer,
            tabs,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
        }
    }
}

/// A paragraph's base direction ([`ParaStyle::direction`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParaDirection {
    LeftToRight,
    RightToLeft,
}

/// How a paragraph's leading is measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LeadingModel {
    /// From one line's baseline to the next's: a line's leading is the space above it, and area
    /// type's first baseline follows Area Type Options › First Baseline.
    #[default]
    RomanBaseline,
    /// From the top of one line's ideographic em box (the right side, in vertical type) to the
    /// next's: a line's leading is the space below it, and area type's first line touches the top
    /// of the frame. Japanese layout's usual model.
    EmBoxTop,
}

/// How Japanese punctuation is spaced (JLREQ 3.1). Full-width punctuation is half a glyph and half
/// a space: an opening bracket's space before it, a closing bracket's, a comma's or a full stop's
/// after it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mojikumi {
    /// Every character takes its full advance.
    #[default]
    None,
    /// Consecutive punctuation shares one half-em space (JLREQ 3.1.4), and a closing bracket,
    /// comma or full stop ending a line is set half width.
    LineEndHalf,
}

impl Mojikumi {
    pub fn is_none(&self) -> bool {
        *self == Mojikumi::None
    }
}

/// Which Japanese characters may not start or end a line (kinsoku shori).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kinsoku {
    /// No kinsoku: a line may break between any two characters that allow a break.
    None,
    /// Closing brackets, commas, full stops, iteration marks, the prolonged sound mark, small
    /// kana and the like don't start a line; opening brackets don't end one.
    #[default]
    Hard,
    /// As Hard, except that 々, the prolonged sound mark ー and small kana may start a line
    /// (JLREQ's level 3 line-breaking rules, Appendix C.3).
    Soft,
}

/// Hanging punctuation (burasagari): an East Asian comma or full stop ending a line (、。，．､｡)
/// stands outside the line's measure, in the space its punctuation spacing gives it (half width
/// with Line-end Punctuation Half Width). Closing brackets and Latin punctuation don't hang. Shown
/// as None / Regular / Force.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Burasagari {
    /// A comma or full stop that doesn't fit goes to the next line with the character before it.
    #[default]
    None,
    /// A comma or full stop that doesn't fit hangs outside the line; one that fits stays inside.
    Standard,
    /// A comma or full stop ending a line always hangs, and the rest of the line fills the measure.
    Forced,
}

/// Area Type Options "First Baseline" offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FirstBaseline {
    /// The tallest glyph ascent touches the frame top (Illustrator's default).
    #[default]
    Ascent,
    CapHeight,
    XHeight,
    /// The first line's leading.
    Leading,
    /// Exactly `first_baseline_min` below the top.
    Fixed,
}

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
    pub rows: usize,
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

/// A named character or paragraph style: the attributes it sets (a subset of [`CharStyle`] or
/// [`ParaStyle`] fields, by their serialized names). Text using it records the name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyleDef {
    pub name: String,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
}

/// A run of text sharing one character style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub style: CharStyle,
    /// An inline graphic: the run is one [`INLINE_CHAR`] drawn as a document symbol that flows
    /// with the text like a glyph (InDesign-style inline anchored object). Its `style` still sets
    /// the size and tracking; fills and strokes don't apply to the art.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<InlineArt>,
}

impl TextRun {
    /// A plain text run.
    pub fn new(text: impl Into<String>, style: CharStyle) -> Self {
        Self { text: text.into(), style, inline: None }
    }
    /// An inline graphic run showing `art`.
    pub fn inline(art: InlineArt, style: CharStyle) -> Self {
        Self { text: INLINE_CHAR.to_string(), style, inline: Some(art) }
    }
}

/// The object replacement character: the plain text of an inline graphic run.
pub const INLINE_CHAR: char = '\u{FFFC}';

/// Largest inline graphic scale (× the run's size) accepted from input.
pub const INLINE_MAX_SCALE: f64 = 100.0;

/// A document symbol placed inline in text ([`TextRun::inline`]).
///
/// Placement: the art is scaled uniformly so its height (visual bounds) is `scale` × the run's
/// font size, its left edge at the pen position, and its vertical centre on the middle of the
/// run's cap height, raised by `baseline_shift` (points, positive = up). Centred on the cap
/// height the art lines up with capitals and figures (like a mana symbol in rules text) and, at
/// the default scale, stays inside the font's ascent and descent so the leading doesn't change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InlineArt {
    /// Name of the document symbol ([`crate::Symbol::name`]).
    pub symbol: String,
    /// Art height as a multiple of the run's font size.
    #[serde(default = "one")]
    pub scale: f64,
    /// Extra raise in points (positive = up).
    #[serde(default)]
    pub baseline_shift: f64,
    /// The symbol art's visual bounds at its natural size, resolved from the document's symbols by
    /// [`crate::Document::resolve_inline_art`] (not saved). `None`: not resolved or missing; the
    /// layout then reserves a one-em square and nothing is drawn.
    #[serde(skip)]
    pub bounds: Option<Rect>,
}

fn one() -> f64 {
    1.0
}

impl InlineArt {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self { symbol: symbol.into(), scale: 1.0, baseline_shift: 0.0, bounds: None }
    }
    /// `scale`, made finite and positive (untrusted input).
    pub fn safe_scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 { self.scale.min(INLINE_MAX_SCALE) } else { 1.0 }
    }
    /// `baseline_shift`, made finite (untrusted input).
    pub fn safe_shift(&self) -> f64 {
        if self.baseline_shift.is_finite() { self.baseline_shift.clamp(-1e5, 1e5) } else { 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TextKind {
    /// Point type: anchored at the baseline origin of the first line.
    Point,
    /// Area type flowed inside `frame` (document coordinates, untransformed by `xf`).
    Area { frame: PathData },
    /// Type on a path, flowing from its start bracket `start` to its end bracket `end` (0..1 of the
    /// path length; no `end`: the end of the path, or once round a closed path). See
    /// [`TextKind::path_span`].
    OnPath {
        path: PathData,
        start: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
    },
}

impl TextKind {
    /// Type on a path's span: from its start to its end bracket, as fractions of its path's length.
    /// Round a closed path the end may be past 1 (the span runs on past the path's start), a full
    /// turn without an end or with the end at the start. None for other type.
    pub fn path_span(&self) -> Option<(f64, f64)> {
        let TextKind::OnPath { path, start, end } = self else { return None };
        let s = fraction(*start);
        Some(if path.is_closed() {
            let run = end.map_or(0.0, |e| (fraction(e) - s).rem_euclid(1.0));
            (s, s + if run < 1e-9 { 1.0 } else { run })
        } else {
            (s, end.map_or(1.0, |e| fraction(e).max(s)))
        })
    }
}

/// `x` as a fraction of a path's length (0..1; 0 when not finite).
fn fraction(x: f64) -> f64 {
    if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
}

/// `path` run the other way, each point as far from its new start as it was from its old end
/// (the subpaths in reverse order, a closed one keeping its first anchor first).
fn reverse_from_end(path: &mut PathData) {
    path.subpaths.reverse();
    for sp in &mut path.subpaths {
        sp.reverse();
        if sp.closed && !sp.anchors.is_empty() {
            sp.anchors.rotate_right(1);
        }
    }
}

/// A text object. `runs` split into paragraphs at `\n`.
///
/// Paragraph attributes: `para` is paragraph 0's (and, while `paras` is empty, every
/// paragraph's). Invariant (kept by [`TextObject::normalize_paras`]): `paras` is either empty —
/// every paragraph uses `para` — or holds one style per paragraph with `paras[0] == para` and at
/// least two different styles. Readers that only know `para` see the first paragraph's style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextObject {
    /// Top-to-bottom, right-to-left writing. Defaults to horizontal for old documents.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub vertical: bool,
    pub kind: TextKind,
    /// Maps text space (origin = first baseline start for point type) to the document.
    pub xf: Affine,
    pub runs: Vec<TextRun>,
    #[serde(default)]
    pub para: ParaStyle,
    /// Per-paragraph attributes (see the type's docs); empty = every paragraph uses `para`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paras: Vec<ParaStyle>,
    /// Area Type Options (area type only).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub area: AreaOptions,
    /// Type on a Path effect (type on a path only).
    #[serde(default, rename = "pathEffect", skip_serializing_if = "crate::skip::is_default")]
    pub path_effect: PathEffect,
    /// Type on a Path Options › Align to Path (type on a path only).
    #[serde(default, rename = "pathAlign", skip_serializing_if = "crate::skip::is_default")]
    pub path_align: PathAlign,
    /// Type on a Path Options › Spacing in points (type on a path only): glyphs are spaced as if
    /// set this far above the path, which closes them up round the outside of a curve and opens
    /// them up round the inside.
    #[serde(default, rename = "pathSpacing", skip_serializing_if = "crate::skip::is_default")]
    pub path_spacing: f64,
    /// Wrap objects above this area type, resolved by the engine after each edit (text space).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wrap: Vec<WrapShape>,
    /// Cached layout bounds in text space, filled in by the layout engine (not serialized).
    #[serde(skip)]
    pub cached_bounds: Option<Rect>,
    /// Cached baselines in text space, start to end, one per line that holds characters (a
    /// vertical column's centre line), filled in with `cached_bounds` (not serialized).
    #[serde(skip)]
    pub cached_baselines: Vec<(Point, Point)>,
}

/// Conversion from stored character attributes to document-space type controls.
/// Point size follows the transformed em-height axis; horizontal scale is the ratio of
/// the two transformed em axes. Rotation, reflection and shear remain in the affine.
#[derive(Clone, Copy, Debug)]
pub struct TextStyleScale {
    pub points: f64,
    pub horizontal: f64,
}

impl TextObject {
    /// None for collapsed or unrepresentable transforms, whose type dimensions cannot
    /// be edited by compensating the stored attributes.
    pub fn style_scale(&self) -> Option<TextStyleScale> {
        let [a, b, c, d, e, f] = self.xf.as_coeffs();
        if ![a, b, c, d, e, f].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        let determinant = (a / sx) * (d / sy) - (b / sx) * (c / sy);
        let horizontal = sx / sy;
        if !sy.is_finite() || sy <= 0.0 || !horizontal.is_finite() || horizontal <= 0.0 || !determinant.is_finite() || determinant.abs() <= 1e-12 {
            return None;
        }
        // Avoid a pure rotation turning otherwise identical fields into mixed values.
        let unit = |v: f64| if (v - 1.0).abs() <= 1e-12 { 1.0 } else { v };
        Some(TextStyleScale { points: unit(sy), horizontal: unit(horizontal) })
    }

    /// Character attributes as presented by controls (stored runs stay local for layout
    /// and rich-text clipboard operations). Explicit vertical character scale stays separate.
    pub fn effective_char_style(&self, style: &CharStyle) -> Option<CharStyle> {
        let scale = self.style_scale()?;
        let mut style = style.clone();
        style.size *= scale.points;
        style.leading = style.leading.map(|v| v * scale.points);
        style.baseline_shift *= scale.points;
        style.h_scale *= scale.horizontal;
        if ![style.size, style.leading.unwrap_or(0.0), style.baseline_shift, style.h_scale].iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(style)
    }

    pub fn point(origin: Point, text: &str, style: CharStyle) -> Self {
        Self {
            vertical: false,
            kind: TextKind::Point,
            xf: Affine::translate(origin.to_vec2()),
            runs: vec![TextRun::new(text, style)],
            para: ParaStyle::default(),
            paras: Vec::new(),
            area: AreaOptions::default(),
            path_effect: PathEffect::default(),
            path_align: PathAlign::default(),
            path_spacing: 0.0,
            wrap: Vec::new(),
            cached_bounds: None,
            cached_baselines: Vec::new(),
        }
    }
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
    /// Number of paragraphs (`\n`-separated; empty text has one).
    pub fn paragraph_count(&self) -> usize {
        1 + self.runs.iter().map(|r| r.text.bytes().filter(|&b| b == b'\n').count()).sum::<usize>()
    }
    /// Attributes of paragraph `i` (the last paragraph's past the end).
    pub fn para_at(&self, i: usize) -> &ParaStyle {
        self.paras.get(i).or(self.paras.last()).unwrap_or(&self.para)
    }
    /// Every paragraph's attributes, one per paragraph.
    pub fn paragraph_styles(&self) -> Vec<ParaStyle> {
        (0..self.paragraph_count()).map(|i| self.para_at(i).clone()).collect()
    }
    /// Set every paragraph's attributes from `styles` (one per paragraph; a short list repeats its
    /// last entry, a long one is cut).
    pub fn set_paragraph_styles(&mut self, styles: Vec<ParaStyle>) {
        if let Some(first) = styles.first() {
            self.para = first.clone();
        }
        self.paras = styles;
        self.normalize_paras();
    }
    /// The same attributes for every paragraph.
    pub fn set_all_paras(&mut self, style: ParaStyle) {
        self.para = style;
        self.paras.clear();
    }
    /// Re-establish the `paras` invariant: one entry per paragraph (cut, or extended with the
    /// last), `paras[0] == para`, and empty when every paragraph is alike.
    pub fn normalize_paras(&mut self) {
        if self.paras.is_empty() {
            return;
        }
        let n = self.paragraph_count();
        let last = self.paras.last().cloned().unwrap_or_default();
        self.paras.resize(n, last);
        if let Some(first) = self.paras.first_mut() {
            first.clone_from(&self.para);
        }
        if self.paras.iter().all(|p| *p == self.para) {
            self.paras.clear();
        }
    }
    /// Indices of the paragraphs that byte range `start..end` of the plain text touches (a caret
    /// touches its paragraph).
    pub fn paragraphs_in(&self, start: usize, end: usize) -> std::ops::Range<usize> {
        let text = self.plain_text();
        let (a, b) = (start.min(end).min(text.len()), start.max(end).min(text.len()));
        let index = |byte: usize| text.as_bytes().get(..byte).map_or(0, |s| s.iter().filter(|&&c| c == b'\n').count());
        index(a)..index(b) + 1
    }
    /// Apply `f` to the attributes of paragraphs `range` (indices; None = every paragraph).
    pub fn edit_paras(&mut self, range: Option<std::ops::Range<usize>>, mut f: impl FnMut(&mut ParaStyle)) {
        match range {
            None => {
                f(&mut self.para);
                for p in &mut self.paras {
                    f(p);
                }
            }
            Some(r) => {
                let mut v = self.paragraph_styles();
                for p in v.iter_mut().take(r.end).skip(r.start) {
                    f(p);
                }
                self.set_paragraph_styles(v);
            }
        }
        self.normalize_paras();
    }
    /// Every stored paragraph style (`para` and the per-paragraph ones), for scans and renames.
    pub fn para_styles_mut(&mut self) -> impl Iterator<Item = &mut ParaStyle> {
        std::iter::once(&mut self.para).chain(self.paras.iter_mut())
    }
    /// Every stored paragraph style (`para` and the per-paragraph ones).
    pub fn para_styles(&self) -> impl Iterator<Item = &ParaStyle> {
        std::iter::once(&self.para).chain(self.paras.iter())
    }
    /// Update the paragraph styles for replacing bytes `start..end` of the current plain text by
    /// `insert` (call before changing the runs). The paragraph the range starts in keeps its
    /// style (merging paragraphs: the first one's wins); paragraphs the insertion starts take that
    /// style too (Return continues the paragraph it splits); later paragraphs keep theirs.
    pub fn splice_paras(&mut self, start: usize, end: usize, insert: &str) {
        if self.paras.is_empty() {
            return;
        }
        let r = self.paragraphs_in(start, end);
        let v = self.paragraph_styles();
        let Some(keep) = v.get(r.start).cloned() else { return };
        let added = insert.bytes().filter(|&b| b == b'\n').count();
        let mut out: Vec<ParaStyle> = v.iter().take(r.start + 1).cloned().collect();
        out.extend(std::iter::repeat_n(keep, added));
        out.extend(v.iter().skip(r.end).cloned());
        self.paras = out;
        if let Some(first) = self.paras.first() {
            self.para = first.clone();
        }
    }
    pub fn first_style(&self) -> CharStyle {
        self.runs.first().map(|r| r.style.clone()).unwrap_or_default()
    }
    /// Approximate bounds when no layout cache is available (0.55 em average advance).
    pub fn estimate_bounds(&self) -> Rect {
        let st = self.first_style();
        let text = self.plain_text();
        let lines: Vec<&str> = text.split('\n').collect();
        let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * st.size * 0.55;
        let lead = st.effective_leading();
        Rect::new(0.0, -st.size * 0.8, w.max(1.0), -st.size * 0.8 + lead * lines.len().max(1) as f64)
    }
    pub fn bounds(&self) -> Option<Rect> {
        match &self.kind {
            TextKind::Area { frame } => frame.bounds().map(|b| self.xf.transform_rect_bbox(b)),
            TextKind::OnPath { path, .. } => path.bounds().map(|b| self.xf.transform_rect_bbox(b)),
            TextKind::Point => Some(self.xf.transform_rect_bbox(self.cached_bounds.unwrap_or_else(|| self.estimate_bounds()))),
        }
    }
    pub fn transform(&mut self, a: Affine) {
        self.xf = a * self.xf;
    }
    /// Type on a path's path in document space; None for other type.
    pub fn type_path(&self) -> Option<PathData> {
        match &self.kind {
            TextKind::OnPath { path, .. } => Some(path.transformed(self.xf)),
            _ => None,
        }
    }
    /// Flip type on a path to the other side of its path (Type on a Path Options › Flip, or its
    /// centre bracket dragged across the path): the path runs the other way and the brackets swap
    /// ends, so the type keeps its stretch of the path. False, changing nothing, for other type.
    pub fn flip_on_path(&mut self) -> bool {
        let Some((s, e)) = self.kind.path_span() else { return false };
        let TextKind::OnPath { path, start, end } = &mut self.kind else { return false };
        reverse_from_end(path);
        if path.is_closed() {
            *start = (1.0 - e).rem_euclid(1.0);
            *end = (e - s < 1.0 - 1e-9).then(|| (1.0 - s).rem_euclid(1.0));
        } else {
            *start = 1.0 - e;
            *end = (s > 1e-9).then_some(1.0 - s);
        }
        true
    }
    /// Area type's frame (the type area) in document space; None for other type.
    pub fn area_frame(&self) -> Option<PathData> {
        match &self.kind {
            TextKind::Area { frame } => Some(frame.transformed(self.xf)),
            _ => None,
        }
    }
    /// Reshape area type's frame by `a`, a document-space transform, keeping the type at its size
    /// (a bounding-box resize: the text reflows, it isn't scaled). False, changing nothing, for
    /// other type, a degenerate `xf` or a frame that would not be finite.
    pub fn transform_area(&mut self, a: Affine) -> bool {
        let xf = self.xf;
        self.reshape_area(|frame, to_text| {
            frame.transform(to_text * a * xf);
            true
        })
    }
    /// Move anchors `refs` (subpath, anchor) of area type's frame by `d` in document space, with
    /// their handles (a Direct Selection drag of a frame corner or edge). False, changing
    /// nothing, when an anchor doesn't exist, or as [`Self::transform_area`].
    pub fn move_area_anchors(&mut self, refs: &[(usize, usize)], d: Vec2) -> bool {
        self.reshape_area(|frame, to_text| {
            // A distance: only the linear part of the document → text map applies.
            let local = to_text * d.to_point() - to_text * Point::ZERO;
            refs.iter().all(|&(si, ai)| frame.anchor_mut(si, ai).map(|a| a.translate(local)).is_some())
        })
    }
    /// Edit a copy of the area frame with `f` (given the document → text space map) and keep it
    /// when `f` succeeds and the result is finite.
    fn reshape_area(&mut self, f: impl FnOnce(&mut PathData, Affine) -> bool) -> bool {
        let det = self.xf.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return false;
        }
        let to_text = self.xf.inverse();
        let TextKind::Area { frame } = &mut self.kind else { return false };
        let mut next = frame.clone();
        if !f(&mut next, to_text) || !next.anchors().all(|(_, _, a)| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return false;
        }
        *frame = next;
        true
    }
    /// Scale the character strokes' weights and dashes by `s` (they are drawn in text space, so
    /// `1 / scale` keeps their weight through a transform that scales the type).
    pub fn scale_char_strokes(&mut self, s: f64) {
        for r in &mut self.runs {
            r.style.stroke_width *= s;
            if let Some(d) = &mut r.style.stroke_dash {
                d.scale(s);
            }
        }
    }
    /// Layout bounds in text space (the layout cache, else the estimate): the box an unplaced run
    /// gradient fits, as the renderers lay it out.
    pub fn local_bounds(&self) -> Rect {
        self.cached_bounds.unwrap_or_else(|| self.estimate_bounds())
    }
    /// The baselines in text space (the layout cache): for point type with none cached, the first
    /// one across [`Self::local_bounds`] (down the first column's centre line for vertical type).
    pub fn baselines(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        let first = (self.cached_baselines.is_empty() && matches!(self.kind, TextKind::Point)).then(|| {
            let b = self.local_bounds();
            if self.vertical { (Point::new(0.0, b.y0), Point::new(0.0, b.y1)) } else { (Point::new(b.x0, 0.0), Point::new(b.x1, 0.0)) }
        });
        self.cached_baselines.iter().copied().chain(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_controls_reject_collapsed_and_nonfinite_transforms() {
        let mut t = TextObject::point(Point::ZERO, "Text", CharStyle::default());
        for matrix in [[0.0; 6], [1.0, 1.0, 2.0, 2.0, 0.0, 0.0], [f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, f64::NAN, 0.0]] {
            t.xf = Affine::new(matrix);
            assert!(t.effective_char_style(&t.first_style()).is_none());
        }
        t.xf = Affine::rotate(0.83);
        assert_eq!(t.effective_char_style(&t.first_style()).unwrap().size, 12.0);
        t.xf = Affine::scale(1e308);
        assert!(t.effective_char_style(&t.first_style()).is_none(), "overflowing point size");
    }

    #[test]
    fn point_text_basics() {
        let t = TextObject::point(Point::new(10.0, 20.0), "Hello", CharStyle::default());
        assert_eq!(t.plain_text(), "Hello");
        let b = t.bounds().unwrap();
        assert!(b.x0 >= 10.0 - 1e-9 && b.y1 > 20.0);
        assert_eq!(CharStyle::default().effective_leading(), 14.399999999999999);
    }

    fn justified(j: Justify) -> ParaStyle {
        ParaStyle { justify: j, ..ParaStyle::default() }
    }

    #[test]
    fn paragraph_styles_split_merge_and_normalize() {
        let mut t = TextObject::point(Point::ZERO, "one\ntwo\nthree", CharStyle::default());
        assert_eq!((t.paragraph_count(), t.paras.len()), (3, 0), "one style for all: nothing stored");
        assert_eq!(t.paragraphs_in(0, 0), 0..1);
        assert_eq!(t.paragraphs_in(4, 4), 1..2);
        assert_eq!(t.paragraphs_in(2, 9), 0..3);
        assert_eq!(t.paragraphs_in(99, 99), 2..3, "clamped to the text");
        t.edit_paras(Some(1..2), |p| p.justify = Justify::Center);
        let js = |t: &TextObject| t.paragraph_styles().iter().map(|p| p.justify).collect::<Vec<_>>();
        assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Left]);
        // Return inside "two": the new paragraph continues its style.
        t.splice_paras(5, 5, "\n");
        t.runs[0].text.insert(5, '\n');
        assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Center, Justify::Left]);
        // Deleting the break between "one" and "t": the first paragraph's style wins.
        t.splice_paras(3, 4, "");
        t.runs[0].text.remove(3);
        assert_eq!(t.plain_text(), "onet\nwo\nthree");
        assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Left]);
        // An edit that skips the splice is repaired by normalizing (the last style continues).
        t.runs[0].text.push_str("\nfour");
        t.normalize_paras();
        assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Left, Justify::Left]);
        // `para` is paragraph 0's; all alike collapses to `para` alone.
        t.edit_paras(Some(0..1), |p| p.justify = Justify::Right);
        assert_eq!(t.para.justify, Justify::Right);
        t.edit_paras(None, |p| p.justify = Justify::Right);
        assert!(t.paras.is_empty() && t.para.justify == Justify::Right);
        // A short list repeats its last entry, a long one is cut.
        t.set_paragraph_styles(vec![justified(Justify::Left), justified(Justify::Center)]);
        assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Center, Justify::Center]);
        t.set_paragraph_styles(vec![justified(Justify::Center); 9]);
        assert!(t.paras.is_empty() && t.para.justify == Justify::Center);
        // Old documents (no `paras`) load; new ones keep paragraph 0 in `para`.
        t.edit_paras(Some(3..4), |p| p.space_before = 4.0);
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["para"]["justify"], json["paras"][0]["justify"]);
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("paras");
        let back: TextObject = serde_json::from_value(old).unwrap();
        assert!(back.paras.is_empty() && back.para_at(3).justify == Justify::Center);
        let back: TextObject = serde_json::from_value(json).unwrap();
        assert_eq!(back.para_at(3).space_before, 4.0);
    }

    /// 120 × 40 area type at (40, 40), its text drawn at twice its size.
    fn area_type() -> TextObject {
        let mut t = TextObject::point(Point::ZERO, "Some text", CharStyle::default());
        t.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 20.0)) };
        t.xf = Affine::translate((40.0, 40.0)) * Affine::scale(2.0);
        t
    }

    #[test]
    fn area_resizes_its_frame_and_keeps_its_type() {
        let mut t = area_type();
        assert_eq!(t.bounds(), Some(Rect::new(40.0, 40.0, 160.0, 80.0)));
        // Bottom-right handle to (200, 140), about the top-left corner.
        let o = Affine::translate((40.0, 40.0));
        assert!(t.transform_area(o * Affine::scale_non_uniform(160.0 / 120.0, 100.0 / 40.0) * o.inverse()));
        assert_eq!(t.xf, Affine::translate((40.0, 40.0)) * Affine::scale(2.0), "the type keeps its size");
        let b = t.bounds().unwrap();
        assert!((b.x1 - 200.0).abs() < 1e-9 && (b.y1 - 140.0).abs() < 1e-9 && b.x0 == 40.0, "{b:?}");
        assert_eq!(t.area_frame().unwrap().bounds(), Some(b));
        // Point type has no area; a degenerate transform or a non-finite frame changes nothing.
        let mut p = TextObject::point(Point::ZERO, "x", CharStyle::default());
        assert!(!p.transform_area(Affine::scale(2.0)) && p.area_frame().is_none());
        let before = t.clone();
        assert!(!t.transform_area(Affine::scale(f64::INFINITY)));
        t.xf = Affine::scale(0.0);
        assert!(!t.transform_area(Affine::scale(2.0)));
        t.xf = before.xf;
        assert_eq!(t, before);
    }

    #[test]
    fn area_anchors_move_in_document_space() {
        let mut t = area_type();
        // The bottom-right corner (anchor 2) 20 pt right: 10 pt in text space at 2×.
        assert!(t.move_area_anchors(&[(0, 2)], Vec2::new(20.0, 0.0)));
        let TextKind::Area { frame } = &t.kind else { panic!() };
        assert_eq!(frame.subpaths[0].anchors[2].p, Point::new(70.0, 20.0));
        assert_eq!(frame.subpaths[0].anchors[1].p, Point::new(60.0, 0.0), "the others stay");
        // A missing anchor changes nothing.
        let before = t.clone();
        assert!(!t.move_area_anchors(&[(0, 1), (0, 9)], Vec2::new(5.0, 5.0)));
        assert!(!t.move_area_anchors(&[(3, 0)], Vec2::new(5.0, 5.0)));
        assert_eq!(t, before);
    }

    /// Builds from before [`Justify::Auto`] read new type's alignment as Left or Right (and ignore
    /// `justify_auto`); this build reads Auto back.
    #[test]
    fn auto_alignment_saves_readable_by_older_builds() {
        #[derive(Deserialize, Debug, PartialEq)]
        enum OldJustify {
            Left,
            Center,
            Right,
            JustifyLeft,
            JustifyCenter,
            JustifyRight,
            JustifyAll,
        }
        #[derive(Deserialize)]
        struct OldPara {
            justify: OldJustify,
        }
        #[derive(Deserialize)]
        struct OldText {
            para: OldPara,
        }
        for (direction, physical) in
            [(Some(ParaDirection::RightToLeft), OldJustify::Right), (Some(ParaDirection::LeftToRight), OldJustify::Left), (None, OldJustify::Left)]
        {
            let mut t = TextObject::point(Point::ZERO, "שלום", CharStyle::default());
            t.para = ParaStyle { justify: Justify::Auto, direction, ..Default::default() };
            let json = serde_json::to_string(&t).unwrap();
            let old: OldText = serde_json::from_str(&json).unwrap();
            assert_eq!(old.para.justify, physical, "{json}");
            assert_eq!(serde_json::from_str::<TextObject>(&json).unwrap(), t);
        }
        // Other alignments save as before, without the flag.
        let p = ParaStyle { justify: Justify::Right, ..Default::default() };
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("justify_auto"), "{json}");
        assert_eq!(serde_json::from_str::<ParaStyle>(&json).unwrap(), p);
        // Files from before the flag keep their alignment.
        assert_eq!(serde_json::from_str::<ParaStyle>(r#"{"justify":"Center"}"#).unwrap().justify, Justify::Center);
    }
}
