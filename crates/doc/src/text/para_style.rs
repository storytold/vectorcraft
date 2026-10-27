use super::char_style::CharStyle;
use super::tabs_lists::{ParaList, TabStop};
use super::*;

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

/// A paragraph's hyphenation options (Paragraph panel menu › Hyphenation…): which words automatic
/// hyphenation may break, where, and on how many lines in a row. Saved only when they differ from
/// the defaults: words longer than 5 letters, 3 letters on each side of the hyphen, no hyphen
/// limit, no hyphenation zone, and capitalized words hyphenated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HyphenationOptions {
    /// Words Longer Than: only words with more letters than this are hyphenated.
    pub words_longer_than: u32,
    /// After First: the fewest letters a word keeps before the hyphen.
    pub after_first: u32,
    /// Before Last: the fewest letters a word keeps after the hyphen.
    pub before_last: u32,
    /// Hyphen Limit: the most consecutive lines that automatic hyphenation may end with a hyphen;
    /// 0 sets no limit.
    pub hyphen_limit: u32,
    /// Hyphenation Zone (pt): with the Single-line Composer, a line whose text before the next
    /// word already ends within this distance of its right edge breaks before that word instead
    /// of hyphenating it. 0 allows every hyphenation.
    pub zone: f64,
    /// Hyphenate Capitalized Words: when it is off, words that start with a capital letter are not
    /// hyphenated. Words in capitals only are never hyphenated.
    pub capitalized: bool,
}

impl Default for HyphenationOptions {
    fn default() -> Self {
        Self { words_longer_than: 5, after_first: 3, before_last: 3, hyphen_limit: 0, zone: 0.0, capitalized: true }
    }
}

/// How a justified line holding a single word is set ([`JustificationOptions::single_word`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SingleWordJustify {
    /// Spread across the line, between its letters.
    #[default]
    FullJustify,
    AlignLeft,
    AlignCenter,
    AlignRight,
}

/// A paragraph's justification options (Paragraph panel menu › Justification…), in percent.
///
/// Each spacing has a Minimum, a Desired and a Maximum value (`[min, desired, max]`). Desired
/// applies to every line of the paragraph, justified or not: word spaces are that share of the
/// font's word space, each letter is followed by that share of a word space, and glyphs are that
/// wide. Justified lines may vary them between Minimum and Maximum to fill the measure. Saved only
/// when they differ from the defaults: word spacing 80/100/133, letter spacing 0/0/0, glyph
/// scaling 100/100/100, Auto Leading 120 and Full Justify.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JustificationOptions {
    /// Word Spacing, percent of the font's word space (0..1000).
    pub word_spacing: [f64; 3],
    /// Letter Spacing, percent of the font's word space added after each letter (-100..500).
    pub letter_spacing: [f64; 3],
    /// Glyph Scaling, percent of the glyphs' width (50..200).
    pub glyph_scaling: [f64; 3],
    /// Auto Leading: the leading of characters whose leading is Auto, in percent of their size
    /// (0..500).
    pub auto_leading: f64,
    /// Single Word Justification: how a justified line holding one word is set.
    pub single_word: SingleWordJustify,
}

impl JustificationOptions {
    /// Word Spacing's range (percent).
    pub const WORD_SPACING: (f64, f64) = (0.0, 1000.0);
    /// Letter Spacing's range (percent).
    pub const LETTER_SPACING: (f64, f64) = (-100.0, 500.0);
    /// Glyph Scaling's range (percent).
    pub const GLYPH_SCALING: (f64, f64) = (50.0, 200.0);
    /// Auto Leading's range (percent).
    pub const AUTO_LEADING: (f64, f64) = (0.0, 500.0);

    /// Are Desired word spacing, letter spacing and glyph scaling the defaults (nothing to change
    /// in a line's natural widths)?
    pub fn desired_is_default(&self) -> bool {
        let d = Self::default();
        self.word_spacing[1] == d.word_spacing[1] && self.letter_spacing[1] == d.letter_spacing[1] && self.glyph_scaling[1] == d.glyph_scaling[1]
    }

    /// Keeps options read from a file to what `text.setFormat` can set: a value that is not a
    /// finite number becomes its default, every value is clamped to its range, and a Minimum,
    /// Desired and Maximum out of order go back to their defaults together.
    pub fn keep_in_range(&mut self) {
        let d = Self::default();
        for (v, dv, (lo, hi)) in [
            (&mut self.word_spacing, d.word_spacing, Self::WORD_SPACING),
            (&mut self.letter_spacing, d.letter_spacing, Self::LETTER_SPACING),
            (&mut self.glyph_scaling, d.glyph_scaling, Self::GLYPH_SCALING),
        ] {
            for (x, dx) in v.iter_mut().zip(dv) {
                *x = if x.is_finite() { x.clamp(lo, hi) } else { dx };
            }
            if !(v[0] <= v[1] && v[1] <= v[2]) {
                *v = dv;
            }
        }
        let (lo, hi) = Self::AUTO_LEADING;
        self.auto_leading = if self.auto_leading.is_finite() { self.auto_leading.clamp(lo, hi) } else { d.auto_leading };
    }

    /// A paragraph's leading for characters of `size` whose leading is Auto.
    pub fn auto_leading_for(&self, size: f64) -> f64 {
        size * self.auto_leading / 100.0
    }
}

impl Default for JustificationOptions {
    fn default() -> Self {
        Self {
            word_spacing: [80.0, 100.0, 133.0],
            letter_spacing: [0.0, 0.0, 0.0],
            glyph_scaling: [100.0, 100.0, 100.0],
            auto_leading: 120.0,
            single_word: SingleWordJustify::FullJustify,
        }
    }
}

/// Insert Kashidas (Paragraph panel): how long the kashidas are that lengthen the Arabic words of
/// a justified paragraph's lines. A kashida is a tatweel (U+0640) drawn between a word's last pair
/// of joined letters, at most one in each word. Short, Medium and Long kashidas are at most a quarter
/// of an em, half an em and an em long; a Stylistic kashida is a row of whole tatweels as the font
/// draws them, at most an em long.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kashida {
    /// No kashidas: justified lines widen their word spaces.
    #[default]
    None,
    Short,
    Medium,
    Long,
    Stylistic,
}

impl Kashida {
    /// The longest kashida, in ems of the letter that follows it in the text; 0 for None.
    pub fn max_em(self) -> f64 {
        match self {
            Kashida::None => 0.0,
            Kashida::Short => 0.25,
            Kashida::Medium => 0.5,
            Kashida::Long | Kashida::Stylistic => 1.0,
        }
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
    /// The limits automatic hyphenation keeps to (Paragraph panel menu › Hyphenation…).
    pub hyphenation: HyphenationOptions,
    /// Word and letter spacing, glyph scaling, Auto Leading and Single Word Justification
    /// (Paragraph panel menu › Justification…).
    pub justification: JustificationOptions,
    /// Line breaking: Single-line or Every-line Composer.
    pub composer: Composer,
    /// Tab stops (Tabs panel), sorted by position.
    pub tabs: Vec<TabStop>,
    /// Bullets (Paragraph panel): the list the paragraph is in; None for a paragraph that is not
    /// in a list.
    pub list: Option<ParaList>,
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
    /// Roman Hanging Punctuation (Paragraph panel menu): quotation marks, hyphens, full stops and
    /// commas starting or ending a line hang outside the margin, and asterisks, tildes, ellipses,
    /// dashes, colons and semicolons hang by half. East Asian punctuation hangs by burasagari.
    pub roman_hanging: bool,
    /// Insert Kashidas (Paragraph panel): justified lines lengthen their Arabic words with
    /// kashidas before they widen their word spaces.
    pub kashida: Kashida,
}

impl ParaStyle {
    /// Keeps the indents and the space before and after to -[`crate::MAX_COORD`] to
    /// [`crate::MAX_COORD`], and the tab stop positions to 0 to 100,000 pt, the range
    /// `text.tabs.set` takes. A value outside its range becomes the nearer end, and one that is
    /// not a number becomes 0.
    pub fn keep_spacing_in_range(&mut self) {
        let within = |v: f64, lo: f64, hi: f64| if v.is_nan() { 0.0 } else { v.clamp(lo, hi) };
        for v in [&mut self.left_indent, &mut self.right_indent, &mut self.first_line_indent, &mut self.space_before, &mut self.space_after] {
            *v = within(*v, -crate::MAX_COORD, crate::MAX_COORD);
        }
        for t in &mut self.tabs {
            t.position = within(t.position, 0.0, 100_000.0);
        }
    }

    /// The leading of characters styled `st` in this paragraph, as the layout sets it: their own
    /// leading, or for Auto leading the paragraph's Justification › Auto Leading share of their
    /// size (exactly [`CharStyle::effective_leading`] at the default 120%, or when the share is
    /// not a finite number).
    pub fn effective_leading(&self, st: &CharStyle) -> f64 {
        let auto = self.justification.auto_leading;
        if st.leading.is_some() || auto == JustificationOptions::default().auto_leading {
            return st.effective_leading();
        }
        let l = self.justification.auto_leading_for(st.size);
        if l.is_finite() { l } else { st.effective_leading() }
    }
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
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    hyphenation: HyphenationOptions,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    justification: JustificationOptions,
    #[serde(default, skip_serializing_if = "Composer::is_default")]
    composer: Composer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tabs: Vec<TabStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list: Option<ParaList>,
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
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    roman_hanging: bool,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    kashida: Kashida,
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
            hyphenation,
            justification,
            composer,
            tabs,
            list,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
            roman_hanging,
            kashida,
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
            hyphenation,
            justification,
            composer,
            tabs,
            list,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
            roman_hanging,
            kashida,
        }
    }
}

impl From<ParaStyleFile> for ParaStyle {
    fn from(f: ParaStyleFile) -> Self {
        let justify = if f.justify_auto { Justify::Auto } else { f.justify };
        let mut justification = f.justification;
        justification.keep_in_range();
        let ParaStyleFile {
            left_indent,
            right_indent,
            first_line_indent,
            space_before,
            space_after,
            hyphenate,
            hyphenation,
            composer,
            tabs,
            list,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
            roman_hanging,
            kashida,
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
            hyphenation,
            justification,
            composer,
            tabs,
            list,
            style_name,
            mojikumi,
            direction,
            leading_model,
            burasagari,
            kinsoku,
            roman_hanging,
            kashida,
        }
    }
}

/// A paragraph's base direction ([`ParaStyle::direction`]), or the direction characters are set
/// in ([`CharStyle::char_direction`]).
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
