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

/// The list a paragraph is in (Paragraph panel › Bullets and Numbered List). The layout draws the
/// list's marker before the paragraph's first line, in the font, size and color of the paragraph's
/// first character without its baseline shift, rotation, superscript or subscript, tracking, manual
/// kerning, tate-chu-yoko, underline and strikethrough ([`Self::marker_style`]), and the text after
/// it starts at the next tab stop, the left indent counting as a stop. The marker is not a
/// character of the text, and an empty paragraph has none. A saved list reads the fields it leaves
/// out as their defaults, and a build ignores the fields it doesn't know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ParaList {
    /// The bullet each paragraph of a bulleted list starts with. A numbered list keeps it for
    /// when the list is made bulleted again.
    pub bullet: Bullet,
    /// A numbered list's number style. A bulleted list has none, and the file saves it only for a
    /// numbered list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<NumberStyle>,
}

impl ParaList {
    /// A bulleted list with `bullet`.
    pub fn bulleted(bullet: Bullet) -> Self {
        Self { bullet, number: None }
    }

    /// A numbered list with `style`.
    pub fn numbered(style: NumberStyle) -> Self {
        Self { bullet: Bullet::default(), number: Some(style) }
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
    /// which holds no text when `empty`, with `count` the numbered paragraphs counted before it.
    /// A paragraph in no list sets the count back to 0 and has no marker; an empty paragraph has
    /// none and leaves the count as it is; a bulleted paragraph has its bullet and leaves the
    /// count as it is; a numbered paragraph adds one to the count and is numbered with it.
    pub fn next_marker(list: Option<ParaList>, empty: bool, count: &mut u32) -> Option<String> {
        let Some(list) = list else {
            *count = 0;
            return None;
        };
        if empty {
            return None;
        }
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
            optical_kerning: first.optical_kerning,
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
