use super::para_style::ParaDirection;
use super::{Appearance, AppearanceItem, Color, Dash, Deserialize, FillLayer, LineCap, LineJoin, Paint, Serialize, StrokeAlign, StrokeLayer};

/// Character attributes (the Character panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharStyle {
    pub font_family: String,
    #[serde(default = "regular")]
    pub font_style: String,
    /// Internal layout selection from a validated file-font binding, never read from a file.
    #[serde(skip)]
    pub file_font_id: Option<u32>,
    /// Which installed version of the font (its version string) when several versions of the
    /// family and style are installed and the text was set in one the family and style alone
    /// wouldn't pick (an imported file's type matched to it); None = the one they pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_version: Option<String>,
    /// Variation axis settings of a variable font (Character panel › Variable Font), by axis tag
    /// (`wght`, `wdth`), in the axis's own units. They replace the named instance's setting of
    /// each axis they name; the other axes keep the style's settings. Empty: the style as it is.
    #[serde(default, rename = "fontAxes", skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub font_axes: std::collections::BTreeMap<String, f64>,
    /// Size in points.
    pub size: f64,
    /// Leading in points; None = Auto (120% of size).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leading: Option<f64>,
    /// Tracking in 1/1000 em; in horizontal type, of the em as [`Self::h_scale`] draws it.
    #[serde(default)]
    pub tracking: f64,
    /// Manual pair spacing in 1/1000 em, as [`Self::tracking`]; None uses the automatic mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kerning: Option<f64>,
    /// Space before the character in 1/1000 em, as [`Self::kerning`], when it starts a line by
    /// itself: the first character of the text, of a paragraph, or after a forced line break. The
    /// Type tool's caret before such a character kerns with it (#5111), since there is no character
    /// before it whose [`Self::kerning`] could space it. Elsewhere it has no effect.
    #[serde(default, rename = "kerningBefore", skip_serializing_if = "crate::skip::is_default")]
    pub kerning_before: f64,
    /// Insert Aki Left (Character panel, East Asian options): space added before the character in
    /// reading order, in 1/1000 em as [`Self::tracking`], 0 to 1000. Before is on the left in
    /// left-to-right type, on the right in right-to-left type and above in vertical type. A
    /// cluster takes the value of its first character, and full justification leaves it as it is.
    /// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
    #[serde(default, rename = "akiLeft", skip_serializing_if = "crate::skip::is_default")]
    pub aki_left: f64,
    /// Insert Aki Right: space added after the character in reading order, as [`Self::aki_left`].
    #[serde(default, rename = "akiRight", skip_serializing_if = "crate::skip::is_default")]
    pub aki_right: f64,
    /// Use glyph shapes for automatic pair spacing; false uses font metrics.
    #[serde(default, rename = "opticalKerning", skip_serializing_if = "crate::skip::is_default")]
    pub optical_kerning: bool,
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
    /// a feature on, `"-liga"` off, and `"salt=2"` turns a feature with alternates on at the font's
    /// alternate number 2 (the value the shaper gets; a plain `"salt"` is value 1). Runs that
    /// differ only in this list are shaped together, each with its own features.
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
    /// Align Stroke of the character stroke (Stroke panel, with type selected): centred on the
    /// glyph outlines, inside or outside them, as for object strokes; the type stays live.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub stroke_align: StrokeAlign,
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
    /// Character Direction (Character panel menu, Middle Eastern options): these characters are
    /// set left to right or right to left inside their paragraph, each stretch of them a Unicode
    /// bidi embedding (UAX #9 LRE or RLE); None = Default, the direction the bidi algorithm gives
    /// them.
    #[serde(default, rename = "charDirection", skip_serializing_if = "Option::is_none")]
    pub char_direction: Option<ParaDirection>,
    /// Character panel menu › Kashidas: whether these characters may receive a kashida when
    /// their paragraph is justified with Insert Kashidas. On by default, including old documents.
    #[serde(default = "crate::yes", skip_serializing_if = "crate::skip::is_true")]
    pub kashidas: bool,
    /// Proportional Metrics (Character panel menu, East Asian options): full-width glyphs take the
    /// font's own proportional widths, OpenType `palt` (#966); in vertical type upright glyphs take
    /// its proportional heights, `vpal`.
    #[serde(default, rename = "proportionalMetrics", skip_serializing_if = "crate::skip::is_default")]
    pub proportional_metrics: bool,
    /// Tate-chu-yoko (Character panel menu, East Asian options): in vertical type these characters
    /// stand side by side across one em of the column. Horizontal type keeps the mark and does not
    /// move them. Two or three half-width digits do this without the mark.
    /// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
    #[serde(default, rename = "tateChuYoko", skip_serializing_if = "crate::skip::is_default")]
    pub tate_chu_yoko: bool,
    /// Tate-chu-yoko Settings › Up/Down (Character panel menu, East Asian options), in points: a
    /// block of marked characters in vertical type moves up by a positive value and down by a
    /// negative one. The block takes the value of its first character; automatic digit blocks and
    /// horizontal type do not move. Saved only when not 0.
    /// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
    #[serde(default, rename = "tcyUpDown", skip_serializing_if = "crate::skip::is_default")]
    pub tcy_up_down: f64,
    /// Tate-chu-yoko Settings › Left/Right, in points: a block of marked characters in vertical type
    /// moves right by a positive value and left by a negative one, as [`Self::tcy_up_down`] does.
    #[serde(default, rename = "tcyLeftRight", skip_serializing_if = "crate::skip::is_default")]
    pub tcy_left_right: f64,
    /// Standard Vertical Roman Alignment (Character panel menu, East Asian options): in vertical
    /// type the half-width characters among these (Latin letters, digits and other characters that
    /// would lie on their side) stand upright one below the other instead of being rotated
    /// individually. Spaces and tate-chu-yoko blocks do not change, and horizontal type keeps the
    /// mark and does not change.
    /// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
    #[serde(default, rename = "standardVerticalRoman", skip_serializing_if = "crate::skip::is_default")]
    pub standard_vertical_roman: bool,
    /// Warichu (Character panel menu, East Asian options): these characters are set at
    /// [`Self::warichu_scale`] percent of their size and stacked on [`Self::warichu_lines`] lines
    /// inside their line, one above the other in horizontal type and side by side in vertical type.
    /// A stretch of marked characters is one block that takes the settings of its first character
    /// and stays whole on one line. Type on a path sets them at their own size on the path.
    /// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub warichu: bool,
    /// Warichu Settings › Lines: how many lines a warichu block is stacked on, within
    /// [`WARICHU_LINES`]. Saved only when not 2.
    #[serde(default = "two", rename = "warichuLines", skip_serializing_if = "is_two")]
    pub warichu_lines: u8,
    /// Warichu Settings › Line Gap, in points from 0 to [`MAX_WARICHU_GAP`]: the space between the
    /// lines of a warichu block. Saved only when not 0.
    #[serde(default, rename = "warichuGap", skip_serializing_if = "crate::skip::is_default")]
    pub warichu_gap: f64,
    /// Warichu Settings › Scale: the size of warichu characters as a percentage of their size,
    /// within [`WARICHU_SCALE`]. Saved only when not 50.
    #[serde(default = "fifty", rename = "warichuScale", skip_serializing_if = "is_fifty")]
    pub warichu_scale: f64,
    /// Warichu Settings › Alignment: how the lines of a warichu block line up along the block.
    #[serde(default, rename = "warichuAlign", skip_serializing_if = "crate::skip::is_default")]
    pub warichu_align: WarichuAlign,
    /// Digits (Character panel menu, Middle Eastern options): the numeral system the digits of
    /// these characters are drawn in, whatever digits were typed.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub digits: Digits,
    /// Font height reference (Character panel menu › Show Font Height Options): the height of the
    /// font that [`Self::size`] measures, the em box by default.
    #[serde(default, rename = "fontHeightReference", skip_serializing_if = "crate::skip::is_default")]
    pub font_height_reference: FontHeightReference,
    /// No Break (Character panel menu): these characters are not broken across lines, including
    /// at a space or a hyphenation point inside the run. A run longer than the line stays together
    /// and can overflow it.
    /// <https://helpx.adobe.com/illustrator/using/hyphenation-line-breaks.html>
    #[serde(default, rename = "noBreak", skip_serializing_if = "crate::skip::is_default")]
    pub no_break: bool,
    /// Language (Character panel): the BCP 47 tag of one of [`crate::setup::LANGUAGES`] these
    /// characters are shaped in, so that the font uses that language's forms (OpenType `locl`).
    /// `None`: the default language, English: USA, shaped without a language, which is what text
    /// saved without one reads as. A tag the list doesn't have, from a file, is kept and has no
    /// hyphenation. New type takes the document's language (Document Setup), else the default
    /// language of Preferences › Hyphenation. Saved only when set. Each run hyphenates in its own
    /// language (`vectorcraft_text::hyphen`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Another glyph of the font for the run's first character (Glyphs panel › Show › Alternates
    /// for Current Selection). The text keeps the character; layout draws the chosen glyph while
    /// the run starts with that character and is set in the face the choice names, and the
    /// default glyph otherwise. Saved only when set.
    #[serde(default, rename = "glyphAlternate", skip_serializing_if = "Option::is_none")]
    pub glyph_alternate: Option<GlyphChoice>,
}

/// A glyph chosen for one character ([`CharStyle::glyph_alternate`]): the glyph id and the face it
/// belongs to, named by its PostScript name and version string, so that a glyph id is never used
/// in another font or another version of the font.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GlyphChoice {
    /// The character the glyph is drawn for.
    pub ch: char,
    /// The glyph id in the face.
    pub gid: u32,
    /// The face's PostScript name (`SourceSans3-Regular`).
    pub font: String,
    /// The face's version string (`Version 3.052`); empty when it has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
}

/// What a character's font size measures ([`CharStyle::font_height_reference`]): the em square
/// of the font, the height of its capitals or of its lowercase letters (the cap height and x
/// height of its OS/2 table), or its ideographic character face (ICF, OpenType's `icfb` and
/// `icft` baselines). Layout sets the characters at the em size at which the chosen height equals
/// the font size; with a font that has no such value, the size measures the em box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontHeightReference {
    #[default]
    EmBox,
    CapHeight,
    XHeight,
    IcfBox,
}

/// The numeral system digits are drawn in ([`CharStyle::digits`]): as typed, or every digit of
/// the Western, Arabic-Indic and Eastern Arabic-Indic sets drawn in one of them or in the digits
/// of an Indic script. The text keeps the digits that were typed; only the glyphs change (like
/// All Caps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Digits {
    /// Each digit as typed.
    #[default]
    Default,
    /// Western digits 0–9 (the digits Arabic typography calls Arabic).
    Arabic,
    /// Arabic-Indic digits ٠–٩ (U+0660–U+0669), used in Arabic.
    Hindi,
    /// Eastern Arabic-Indic digits ۰–۹ (U+06F0–U+06F9), used in Persian and Urdu.
    Farsi,
    /// Devanagari digits ०–९ (U+0966–U+096F), used in Hindi, Marathi, Nepali and Sanskrit.
    Devanagari,
    /// Bengali digits ০–৯ (U+09E6–U+09EF), used in Bengali and Assamese.
    Bengali,
    /// Gurmukhi digits ੦–੯ (U+0A66–U+0A6F), used in Punjabi.
    Gurmukhi,
    /// Gujarati digits ૦–૯ (U+0AE6–U+0AEF).
    Gujarati,
    /// Odia digits ୦–୯ (U+0B66–U+0B6F).
    Odia,
    /// Tamil digits ௦–௯ (U+0BE6–U+0BEF).
    Tamil,
    /// Telugu digits ౦–౯ (U+0C66–U+0C6F).
    Telugu,
    /// Kannada digits ೦–೯ (U+0CE6–U+0CEF).
    Kannada,
    /// Malayalam digits ൦–൯ (U+0D66–U+0D6F).
    Malayalam,
}

impl Digits {
    /// Every numeral system, in menu order.
    pub const ALL: [Digits; 13] = [
        Digits::Default,
        Digits::Arabic,
        Digits::Hindi,
        Digits::Farsi,
        Digits::Devanagari,
        Digits::Bengali,
        Digits::Gurmukhi,
        Digits::Gujarati,
        Digits::Odia,
        Digits::Tamil,
        Digits::Telugu,
        Digits::Kannada,
        Digits::Malayalam,
    ];

    /// The name files and commands give this numeral system (as it is saved).
    pub fn id(self) -> &'static str {
        match self {
            Digits::Default => "default",
            Digits::Arabic => "arabic",
            Digits::Hindi => "hindi",
            Digits::Farsi => "farsi",
            Digits::Devanagari => "devanagari",
            Digits::Bengali => "bengali",
            Digits::Gurmukhi => "gurmukhi",
            Digits::Gujarati => "gujarati",
            Digits::Odia => "odia",
            Digits::Tamil => "tamil",
            Digits::Telugu => "telugu",
            Digits::Kannada => "kannada",
            Digits::Malayalam => "malayalam",
        }
    }

    /// The numeral system [`Digits::id`] names; `None` for any other name.
    pub fn from_id(id: &str) -> Option<Digits> {
        Digits::ALL.into_iter().find(|d| d.id() == id)
    }

    /// The digit `c` is drawn as in this numeral system: the same digit in the chosen set, or `c`
    /// itself when `c` isn't a digit of the Western, Arabic-Indic or Eastern Arabic-Indic set.
    pub fn map(self, c: char) -> char {
        let value = match c {
            '0'..='9' => c as u32 - '0' as u32,
            '\u{660}'..='\u{669}' => c as u32 - 0x660,
            '\u{6F0}'..='\u{6F9}' => c as u32 - 0x6F0,
            _ => return c,
        };
        let base = match self {
            Digits::Default => return c,
            Digits::Arabic => '0' as u32,
            Digits::Hindi => 0x660,
            Digits::Farsi => 0x6F0,
            Digits::Devanagari => 0x966,
            Digits::Bengali => 0x9E6,
            Digits::Gurmukhi => 0xA66,
            Digits::Gujarati => 0xAE6,
            Digits::Odia => 0xB66,
            Digits::Tamil => 0xBE6,
            Digits::Telugu => 0xC66,
            Digits::Kannada => 0xCE6,
            Digits::Malayalam => 0xD66,
        };
        char::from_u32(base + value).unwrap_or(c)
    }
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
    /// Returns these proportions with the size kept to 1 to 500% and the position kept to -500 to
    /// 500%, the ranges `document.setup` takes for `superscript` and `subscript`. A value outside
    /// its range becomes the nearer end.
    pub fn in_range(self) -> Self {
        Self { size: self.size.clamp(1.0, 500.0), position: self.position.clamp(-500.0, 500.0) }
    }
}

/// Returns a small caps size kept to 1 to 200%, the range `document.setup` takes for
/// `smallCapsSize`. A size outside it becomes the nearer end.
fn small_caps_in_range(size: f64) -> f64 {
    size.clamp(1.0, 200.0)
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
    /// Returns this position with its proportions kept by [`ScriptMetrics::in_range`].
    pub fn in_range(self) -> Self {
        match self {
            CharPosition::Normal => CharPosition::Normal,
            CharPosition::Superscript(m) => CharPosition::Superscript(m.in_range()),
            CharPosition::Subscript(m) => CharPosition::Subscript(m.in_range()),
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

/// The largest Insert Aki Left or Right ([`CharStyle::aki_left`]), in 1/1000 em: one em.
pub const MAX_AKI: f64 = 1000.0;

/// The number of lines a warichu block may be stacked on ([`CharStyle::warichu_lines`]).
pub const WARICHU_LINES: std::ops::RangeInclusive<u8> = 1..=6;

/// The largest Warichu Settings Line Gap ([`CharStyle::warichu_gap`]), in points.
pub const MAX_WARICHU_GAP: f64 = 100.0;

/// The Warichu Settings Scale ([`CharStyle::warichu_scale`]) range, in percent of the size.
pub const WARICHU_SCALE: std::ops::RangeInclusive<f64> = 10.0..=100.0;

fn two() -> u8 {
    2
}

fn is_two(v: &u8) -> bool {
    *v == 2
}

fn fifty() -> f64 {
    50.0
}

fn is_fifty(v: &f64) -> bool {
    *v == 50.0
}

/// How the lines of a warichu block line up along the block ([`CharStyle::warichu_align`]):
/// each line set flush left (top, in vertical type), centered or flush right (bottom), or spread
/// to the block's length with the last line set flush left, centered or flush right, or spread as
/// well. Auto is the default and sets the lines as [`WarichuAlign::JustifyLeft`] does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarichuAlign {
    #[default]
    Auto,
    Left,
    Center,
    Right,
    JustifyLeft,
    JustifyCenter,
    JustifyRight,
    JustifyAll,
}

impl WarichuAlign {
    /// Every alignment, in the order Warichu Settings lists them.
    pub const ALL: [WarichuAlign; 8] = [
        WarichuAlign::Auto,
        WarichuAlign::Left,
        WarichuAlign::Center,
        WarichuAlign::Right,
        WarichuAlign::JustifyLeft,
        WarichuAlign::JustifyCenter,
        WarichuAlign::JustifyRight,
        WarichuAlign::JustifyAll,
    ];

    /// The name files and commands give this alignment (as it is saved).
    pub fn id(self) -> &'static str {
        match self {
            WarichuAlign::Auto => "auto",
            WarichuAlign::Left => "left",
            WarichuAlign::Center => "center",
            WarichuAlign::Right => "right",
            WarichuAlign::JustifyLeft => "justifyLeft",
            WarichuAlign::JustifyCenter => "justifyCenter",
            WarichuAlign::JustifyRight => "justifyRight",
            WarichuAlign::JustifyAll => "justifyAll",
        }
    }

    /// The alignment [`WarichuAlign::id`] names; `None` for any other name.
    pub fn from_id(id: &str) -> Option<WarichuAlign> {
        WarichuAlign::ALL.into_iter().find(|a| a.id() == id)
    }
}

pub(super) fn yes() -> bool {
    true
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            font_family: "Source Sans 3".into(),
            font_style: "Regular".into(),
            file_font_id: None,
            font_version: None,
            font_axes: std::collections::BTreeMap::new(),
            size: 12.0,
            leading: None,
            tracking: 0.0,
            kerning: None,
            kerning_before: 0.0,
            aki_left: 0.0,
            aki_right: 0.0,
            optical_kerning: false,
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
            stroke_align: StrokeAlign::Center,
            position: CharPosition::Normal,
            char_align: CharAlign::RomanBaseline,
            char_direction: None,
            kashidas: true,
            proportional_metrics: false,
            tate_chu_yoko: false,
            tcy_up_down: 0.0,
            tcy_left_right: 0.0,
            standard_vertical_roman: false,
            warichu: false,
            warichu_lines: 2,
            warichu_gap: 0.0,
            warichu_scale: 50.0,
            warichu_align: WarichuAlign::Auto,
            small_caps: None,
            digits: Digits::Default,
            font_height_reference: FontHeightReference::EmBox,
            no_break: false,
            language: None,
            glyph_alternate: None,
        }
    }
}

impl CharStyle {
    pub fn effective_leading(&self) -> f64 {
        self.leading.unwrap_or(self.size * 1.2)
    }
    /// Keeps the size, leading, baseline shift and horizontal and vertical scales in range: a size
    /// or scale of 0 or less becomes the default new type takes (12 pt, 100%), a negative leading
    /// becomes Auto, and a value past [`crate::MAX_COORD`] (a baseline shift past it either way)
    /// becomes the nearer end. A leading of 0, which legacy `.ai` import stores, stays, and a
    /// baseline shift that is not a number becomes 0. The Tate-chu-yoko Up/Down and Left/Right
    /// offsets are kept as the baseline shift is. Insert Aki Left and Right are kept from 0 to
    /// [`MAX_AKI`], and one that is not a number becomes 0. The Warichu Settings Lines and Scale
    /// are kept to [`WARICHU_LINES`] and [`WARICHU_SCALE`]; a gap or scale that is not a number becomes its default. The gap is kept to 0 to
    /// [`crate::MAX_COORD`], as the baseline shift is, since a scaled type object holds it in its
    /// own units.
    pub fn keep_dimensions_in_range(&mut self) {
        let default = Self::default();
        let within = |v: f64, or: f64| if v > 0.0 { v.min(crate::MAX_COORD) } else { or };
        self.size = within(self.size, default.size);
        self.h_scale = within(self.h_scale, default.h_scale);
        self.v_scale = within(self.v_scale, default.v_scale);
        self.leading = self.leading.filter(|l| *l >= 0.0).map(|l| l.min(crate::MAX_COORD));
        let shift = |v: f64| if v.is_nan() { 0.0 } else { v.clamp(-crate::MAX_COORD, crate::MAX_COORD) };
        self.baseline_shift = shift(self.baseline_shift);
        self.tcy_up_down = shift(self.tcy_up_down);
        self.tcy_left_right = shift(self.tcy_left_right);
        let aki = |v: f64| if v.is_nan() { 0.0 } else { v.clamp(0.0, MAX_AKI) };
        self.aki_left = aki(self.aki_left);
        self.aki_right = aki(self.aki_right);
        self.warichu_lines = self.warichu_lines.clamp(*WARICHU_LINES.start(), *WARICHU_LINES.end());
        self.warichu_gap = if self.warichu_gap.is_nan() { default.warichu_gap } else { self.warichu_gap.clamp(0.0, crate::MAX_COORD) };
        self.warichu_scale =
            if self.warichu_scale.is_nan() { default.warichu_scale } else { self.warichu_scale.clamp(*WARICHU_SCALE.start(), *WARICHU_SCALE.end()) };
    }
    /// Do the characters draw a stroke (a paint and a positive weight)?
    pub fn has_stroke(&self) -> bool {
        !self.stroke.is_none() && self.stroke_width > 0.0
    }
    /// The character stroke as a stroke layer (paint, weight, cap, join, miter limit, dashes,
    /// alignment and overprint), so type strokes share the object strokes' geometry, rendering
    /// and export.
    pub fn stroke_layer(&self) -> StrokeLayer {
        StrokeLayer {
            overprint: self.overprint_stroke,
            cap: self.stroke_cap,
            join: self.stroke_join,
            miter_limit: self.stroke_miter_limit,
            dash: self.stroke_dash.clone(),
            align: self.stroke_align,
            ..StrokeLayer::new(self.stroke.clone(), self.stroke_width)
        }
    }
    /// Take the paint, weight, cap, join, miter limit, dashes and alignment of `st` as the
    /// character stroke (the options characters have no use for, such as arrowheads and width
    /// profiles, are dropped).
    pub fn set_stroke_layer(&mut self, st: &StrokeLayer) {
        self.stroke = st.paint.clone();
        self.stroke_width = st.width;
        self.stroke_cap = st.cap;
        self.stroke_join = st.join;
        self.stroke_miter_limit = st.miter_limit;
        self.stroke_dash = st.dash.clone();
        self.stroke_align = st.align;
    }
    /// How far the character stroke paints past the glyph outlines (text space): half the weight
    /// centred, the whole weight outside, nothing inside (miter spikes aside).
    pub fn stroke_outset(&self) -> f64 {
        if !self.has_stroke() {
            return 0.0;
        }
        match self.stroke_align {
            StrokeAlign::Center => self.stroke_width / 2.0,
            StrokeAlign::Outside => self.stroke_width,
            StrokeAlign::Inside => 0.0,
        }
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
    /// Keeps the superscript or subscript proportions in range with [`CharPosition::in_range`] and
    /// keeps the small caps size to 1 to 200%, the range `document.setup` takes for `smallCapsSize`.
    pub fn keep_scripts_in_range(&mut self) {
        self.position = self.position.in_range();
        self.small_caps = self.small_caps.map(small_caps_in_range);
    }
}

impl crate::Document {
    /// Keeps the superscript, subscript and small caps proportions to the ranges `document.setup`
    /// takes ([`CharStyle::keep_scripts_in_range`]) in the type of the layers, symbols, graph
    /// designs, pattern swatches and the pattern definition that pattern editing's Cancel puts
    /// back (opacity-mask art included), in graphs' label styles and in the character styles. It
    /// recurses through the art: call it on a document no deeper than [`crate::depth::MAX_DEPTH`].
    pub fn keep_scripts_in_range(&mut self) {
        let mut fix = |n: &mut crate::Node| {
            if let crate::NodeKind::Text(t) = &mut n.kind {
                for r in &mut t.runs {
                    r.style.keep_scripts_in_range();
                }
            }
            if let Some(g) = n.extras_if_any().and_then(|e| e.graph.as_deref_mut()) {
                let l = &mut g.label_styles;
                for st in [&mut l.values, &mut l.categories, &mut l.legend].into_iter().flatten() {
                    st.keep_scripts_in_range();
                }
            }
        };
        let roots = self
            .layers
            .iter_mut()
            .chain(self.symbols.iter_mut().map(|s| &mut s.art))
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()));
        for root in roots {
            crate::placed_document::each_mut(std::sync::Arc::make_mut(root), &mut fix);
        }
        // Character styles store the attributes they set by their serialized names.
        for def in self.char_styles.iter_mut() {
            if let Some(v) = def.attrs.get_mut("position")
                && let Ok(p) = serde_json::from_value::<CharPosition>(v.clone())
                && p.in_range() != p
            {
                *v = serde_json::to_value(p.in_range()).unwrap_or(serde_json::Value::Null);
            }
            if let Some(v) = def.attrs.get_mut("small_caps")
                && let Some(size) = v.as_f64()
                && small_caps_in_range(size) != size
            {
                *v = small_caps_in_range(size).into();
            }
        }
    }
}
