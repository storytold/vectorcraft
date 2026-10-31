use super::char_style::{CharPosition, CharStyle};
use super::{Deserialize, Serialize};

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

/// The bullet of a bulleted list ([`ParaList::bullet`]): the Paragraph panel's Bullets presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Bullet {
    /// A filled dot (•).
    #[default]
    Dot,
    /// A dash (–).
    Dash,
    /// A hollow circle (◦).
    Circle,
    /// A filled square (▪).
    Square,
}

impl Bullet {
    /// The character the layout draws for the bullet.
    pub fn character(self) -> char {
        match self {
            Bullet::Dot => '\u{2022}',
            Bullet::Dash => '\u{2013}',
            Bullet::Circle => '\u{25E6}',
            Bullet::Square => '\u{25AA}',
        }
    }
}

/// How a numbered list writes its numbers ([`ParaList::number`]): the Paragraph panel's Numbered
/// List presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberStyle {
    /// 1, 2, 3…
    #[default]
    Arabic,
    /// A, B, C… Z, AA, BB…
    UpperLetters,
    /// a, b, c… z, aa, bb…
    LowerLetters,
    /// I, II, III, IV…
    UpperRoman,
    /// i, ii, iii, iv…
    LowerRoman,
    /// 01, 02… 99, 100…
    TwoDigits,
    /// 001, 002… 999, 1000…
    ThreeDigits,
}

/// Past this number, roman numerals and letters are written in Arabic digits.
const MAX_LETTERS_OR_ROMAN: u32 = 3999;

impl NumberStyle {
    /// Every style, in the order the Numbered List presets list them.
    pub const ALL: [NumberStyle; 7] = [
        NumberStyle::Arabic,
        NumberStyle::UpperLetters,
        NumberStyle::LowerLetters,
        NumberStyle::UpperRoman,
        NumberStyle::LowerRoman,
        NumberStyle::TwoDigits,
        NumberStyle::ThreeDigits,
    ];

    /// Number `n` (1 or more) written in this style, without a separator. Letters run A to Z, then
    /// AA to ZZ, then AAA and so on; roman numerals use the subtractive forms (IV, IX, XL, XC, CD,
    /// CM); both are written in Arabic digits past 3999. Two and three digits pad with zeros and
    /// keep every digit of a longer number.
    pub fn format(self, n: u32) -> String {
        let letters_or_roman =
            matches!(self, NumberStyle::UpperLetters | NumberStyle::LowerLetters | NumberStyle::UpperRoman | NumberStyle::LowerRoman);
        if n == 0 || (letters_or_roman && n > MAX_LETTERS_OR_ROMAN) {
            return n.to_string();
        }
        match self {
            NumberStyle::Arabic => n.to_string(),
            NumberStyle::TwoDigits => format!("{n:02}"),
            NumberStyle::ThreeDigits => format!("{n:03}"),
            NumberStyle::UpperLetters => letters(n),
            NumberStyle::LowerLetters => letters(n).to_lowercase(),
            NumberStyle::UpperRoman => roman(n),
            NumberStyle::LowerRoman => roman(n).to_lowercase(),
        }
    }
}

/// `n` (1 or more) in capital letters: A to Z, then each letter twice, three times and so on.
fn letters(n: u32) -> String {
    let n = n - 1;
    let letter = char::from(b'A' + (n % 26) as u8);
    std::iter::repeat_n(letter, (n / 26 + 1) as usize).collect()
}

/// `n` (1 to 3999) in capital roman numerals.
fn roman(mut n: u32) -> String {
    const DIGITS: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (value, digits) in DIGITS {
        while n >= value {
            s.push_str(digits);
            n -= value;
        }
    }
    s
}

/// Where a list's marker stands before the first line's text ([`ParaList::align`]): the Bullets
/// and Numbering dialog's Alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListAlign {
    /// The marker starts where the first line starts: at the left indent plus the first-line
    /// indent.
    #[default]
    Left,
    /// The marker ends where the first line's text starts, against the text.
    Right,
}

/// The list a paragraph is in (Paragraph panel › Bullets and Numbered List). The layout draws the
/// list's marker before the paragraph's first line, in the font, size and color of the paragraph's
/// first character without its baseline shift, rotation, superscript or subscript, tracking, manual
/// kerning, tate-chu-yoko, underline and strikethrough ([`Self::marker_style`]), and the text after
/// it starts at the next tab stop, the left indent counting as a stop. The marker stands at the
/// first line's start, or against the text with [`ListAlign::Right`]. A paragraph at a level past
/// the first ([`Self::level`]) is set [`LIST_LEVEL_INDENT`] further in for each level. The marker
/// is not a character of the text, and an empty paragraph has none. A saved list reads the fields
/// it leaves out as their defaults, and a build ignores the fields it doesn't know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ParaList {
    /// The bullet each paragraph of a bulleted list starts with. A numbered list keeps it for
    /// when the list is made bulleted again.
    pub bullet: Bullet,
    /// A numbered list's number style. A bulleted list has none, and the file saves it only for a
    /// numbered list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<NumberStyle>,
    /// Where the marker stands before the first line's text. The file saves it only when it is
    /// [`ListAlign::Right`].
    #[serde(skip_serializing_if = "crate::skip::is_default")]
    pub align: ListAlign,
    /// The paragraph's level in the list, 1 to [`LIST_LEVELS`] (Tab and Shift+Tab at the start of
    /// the paragraph, Bullets and Numbering…'s Level): each level counts its own numbers
    /// ([`Self::next_marker`]). A value outside that range is read as the nearest level
    /// ([`Self::depth`]). The file saves it only when it is not 1.
    #[serde(skip_serializing_if = "is_first_level")]
    pub level: u8,
}

impl Default for ParaList {
    /// A bulleted list with a dot, its marker aligned left, at level 1.
    fn default() -> Self {
        Self { bullet: Bullet::default(), number: None, align: ListAlign::default(), level: 1 }
    }
}

/// Is `level` the first list level, which the file does not save?
fn is_first_level(level: &u8) -> bool {
    *level == 1
}

/// How many levels a list has (Bullets and Numbering…'s Level: 1 to 9).
pub const LIST_LEVELS: usize = 9;

/// How much further in (pt) the layout sets a list paragraph for each level past the first: its
/// left indent counts that much more, and the marker and the text move with it.
pub const LIST_LEVEL_INDENT: f64 = 18.0;

/// The numbered paragraphs a story's list has counted at each level, level 1 first
/// ([`ParaList::next_marker`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListCounts(pub [u32; LIST_LEVELS]);

impl ListCounts {
    /// The counts with `first` at level 1 and `deeper` at levels 2 on, a missing level at 0, as a
    /// type object saves them ([`TextObject::list_count`], [`TextObject::list_level_counts`]).
    pub fn from_parts(first: u32, deeper: &[u32]) -> Self {
        let mut counts = [0; LIST_LEVELS];
        counts[0] = first;
        for (c, &d) in counts[1..].iter_mut().zip(deeper) {
            *c = d;
        }
        Self(counts)
    }

    /// The count at level 1, and the counts at levels 2 on without the zeros that end them.
    pub fn parts(&self) -> (u32, Vec<u32>) {
        let deeper = &self.0[1..];
        let used = deeper.iter().rposition(|&c| c != 0).map_or(0, |i| i + 1);
        (self.0[0], deeper[..used].to_vec())
    }
}

impl ParaList {
    /// A bulleted list with `bullet`.
    pub fn bulleted(bullet: Bullet) -> Self {
        Self { bullet, ..Self::default() }
    }

    /// A numbered list with `style`.
    pub fn numbered(style: NumberStyle) -> Self {
        Self { number: Some(style), ..Self::default() }
    }

    /// This list with its marker at `align`.
    pub fn aligned(self, align: ListAlign) -> Self {
        Self { align, ..self }
    }

    /// This list at level `level` (1 to [`LIST_LEVELS`]).
    pub fn at_level(self, level: u8) -> Self {
        Self { level, ..self }
    }

    /// The list's level, 1 to [`LIST_LEVELS`]: [`Self::level`], a value outside the range taken
    /// as the nearest level.
    pub fn depth(&self) -> usize {
        usize::from(self.level).clamp(1, LIST_LEVELS)
    }

    /// How much further in (pt) than its left indent the layout sets a paragraph of this list:
    /// [`LIST_LEVEL_INDENT`] for each level past the first.
    pub fn level_indent(&self) -> f64 {
        (self.depth() - 1) as f64 * LIST_LEVEL_INDENT
    }

    /// The characters of the marker the layout draws before the paragraph: the bullet, or for a
    /// numbered list number `n` in its style followed by a period.
    pub fn marker(&self, n: u32) -> String {
        match self.number {
            Some(style) => format!("{}.", style.format(n)),
            None => self.bullet.character().to_string(),
        }
    }

    /// The marker of the next paragraph of a story, whose list is `list` (None: in no list) and
    /// which holds no text when `empty`, with `counts` the numbered paragraphs counted before it at
    /// each level. A paragraph in no list sets every count back to 0 and has no marker; an empty
    /// paragraph has none and leaves the counts as they are. A paragraph with text sets the counts
    /// of the levels deeper than its own back to 0, so a deeper paragraph after it counts from 1
    /// again; a bulleted one has its bullet and leaves the count of its level as it is, and a
    /// numbered one adds one to the count of its level and is numbered with it. A paragraph back
    /// at a shallower level then goes on with that level's count.
    pub fn next_marker(list: Option<ParaList>, empty: bool, counts: &mut ListCounts) -> Option<String> {
        let Some(list) = list else {
            *counts = ListCounts::default();
            return None;
        };
        if empty {
            return None;
        }
        let level = list.depth() - 1;
        for deeper in &mut counts.0[level + 1..] {
            *deeper = 0;
        }
        let count = &mut counts.0[level];
        if list.number.is_some() {
            *count = count.saturating_add(1);
        }
        Some(list.marker(*count))
    }

    /// The style the marker is set in: `first` (the style of the paragraph's first character)
    /// without its baseline shift, rotation, superscript or subscript, tracking, manual kerning,
    /// tate-chu-yoko, underline and strikethrough.
    pub fn marker_style(first: &CharStyle) -> CharStyle {
        CharStyle {
            baseline_shift: 0.0,
            rotation: 0.0,
            position: CharPosition::Normal,
            tracking: 0.0,
            kerning: None,
            tate_chu_yoko: false,
            underline: false,
            strikethrough: false,
            ..first.clone()
        }
    }

    /// The list that the Type tool starts when `typed` and a space are typed at the start of a
    /// paragraph: "1." or "1)" starts a numbered list in 1, 2, 3, "A." or "A)" starts one in A, B,
    /// C, and "*", "#", ">" or "+" starts a bulleted list with the filled dot. Any other text starts
    /// none.
    pub fn typed(typed: &str) -> Option<ParaList> {
        match typed {
            "1." | "1)" => Some(Self::numbered(NumberStyle::Arabic)),
            "A." | "A)" => Some(Self::numbered(NumberStyle::UpperLetters)),
            "*" | "#" | ">" | "+" => Some(Self::bulleted(Bullet::Dot)),
            _ => None,
        }
    }
}

/// The left and first-line indents (pt) a list gives a paragraph whose indents are both 0: the
/// marker hangs in the first line's 18 pt before the text.
pub const LIST_HANGING_INDENT: f64 = 18.0;
