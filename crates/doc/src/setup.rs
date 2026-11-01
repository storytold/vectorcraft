//! Document Setup (File → Document Setup): the bleed around the artboards, view options
//! (transparency grid, simulated paper, images in outline mode, substitution highlights), output
//! options and the document's typographic defaults. Every field is defaulted, so documents saved
//! before Document Setup existed load unchanged.

use serde::{Deserialize, Serialize};
use vectorcraft_color::Color;
use vectorcraft_geom::Rect;

use crate::text::ScriptMetrics;

/// Transparency grid cell size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl GridSize {
    pub const ALL: [GridSize; 3] = [GridSize::Small, GridSize::Medium, GridSize::Large];
    pub fn id(self) -> &'static str {
        match self {
            GridSize::Small => "small",
            GridSize::Medium => "medium",
            GridSize::Large => "large",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            GridSize::Small => "Small",
            GridSize::Medium => "Medium",
            GridSize::Large => "Large",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id().eq_ignore_ascii_case(s))
    }
    /// Edge of one grid cell in screen points (the grid does not scale with the zoom).
    pub fn cell(self) -> f32 {
        match self {
            GridSize::Small => 4.0,
            GridSize::Medium => 8.0,
            GridSize::Large => 16.0,
        }
    }
}

const fn hex(v: u32) -> Color {
    Color::Rgb { r: ((v >> 16) & 0xff) as f32 / 255.0, g: ((v >> 8) & 0xff) as f32 / 255.0, b: (v & 0xff) as f32 / 255.0 }
}

/// Transparency grid colour presets (VectorCraft's own pairs). Any other pair is "Custom".
pub const GRID_COLOR_PRESETS: &[(&str, [Color; 2])] = &[
    ("Light", [hex(0xffffff), hex(0xcccccc)]),
    ("Medium", [hex(0xb3b3b3), hex(0x8c8c8c)]),
    ("Dark", [hex(0x666666), hex(0x4d4d4d)]),
    ("Red", [hex(0xffffff), hex(0xffc7c7)]),
    ("Orange", [hex(0xffffff), hex(0xffdcb8)]),
    ("Green", [hex(0xffffff), hex(0xc6efc6)]),
    ("Blue", [hex(0xffffff), hex(0xc4dcff)]),
    ("Purple", [hex(0xffffff), hex(0xe2c9ff)]),
];

/// How text is written to formats without live type (Document Setup → Type → Export).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportText {
    /// Keep text as text (editable).
    #[default]
    Editable,
    /// Write glyph outlines (exact appearance without the fonts).
    Appearance,
}

impl ExportText {
    pub fn id(self) -> &'static str {
        match self {
            ExportText::Editable => "editable",
            ExportText::Appearance => "appearance",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [ExportText::Editable, ExportText::Appearance].into_iter().find(|e| e.id().eq_ignore_ascii_case(s))
    }
}

/// Quote characters (Document Setup → Type): opening and closing double and single quotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quotes {
    pub double: [char; 2],
    pub single: [char; 2],
}

impl Default for Quotes {
    fn default() -> Self {
        Self::ENGLISH
    }
}

impl Quotes {
    pub const ENGLISH: Self = Self { double: ['“', '”'], single: ['‘', '’'] };
    /// Double-quote pairs offered by Document Setup.
    pub const DOUBLE_CHOICES: &[[char; 2]] = &[['“', '”'], ['„', '“'], ['„', '”'], ['”', '”'], ['«', '»'], ['»', '«'], ['「', '」'], ['"', '"']];
    /// Single-quote pairs offered by Document Setup.
    pub const SINGLE_CHOICES: &[[char; 2]] = &[['‘', '’'], ['‚', '‘'], ['‚', '’'], ['’', '’'], ['‹', '›'], ['›', '‹'], ['『', '』'], ['\'', '\'']];

    /// Is `c` one of these opening quotes? A typographic glyph that is also a closing quote does not
    /// count; a straight quote has no direction, so it still opens.
    fn is_open(&self, c: char) -> bool {
        (c == self.double[0] || c == self.single[0]) && (c.is_ascii() || (c != self.double[1] && c != self.single[1]))
    }

    /// A quote opens after nothing, whitespace, an opening bracket, a dash or another opening quote.
    /// `quote_opened` says whether `prev` is a quote this conversion just produced, and if so whether
    /// it opened; that settles glyphs which serve as both opening and closing quote.
    fn opens(&self, prev: Option<char>, quote_opened: Option<bool>) -> bool {
        prev.is_none_or(|p| p.is_whitespace() || matches!(p, '(' | '[' | '{' | '—' | '–') || quote_opened.unwrap_or_else(|| self.is_open(p)))
    }

    /// `text` with straight quotes (`"` and `'`) replaced by these quotes: opening or closing by
    /// context (an apostrophe is a closing single quote). `prev` is the character before `text`,
    /// and is updated to its last character, so text can be converted in pieces.
    pub fn apply(&self, text: &str, prev: &mut Option<char>) -> String {
        let mut out = String::with_capacity(text.len());
        let mut quote_opened = None;
        for c in text.chars() {
            quote_opened = match c {
                '"' | '\'' => Some(self.opens(*prev, quote_opened)),
                _ => None,
            };
            let r = match (c, quote_opened) {
                ('"', Some(open)) => self.double[usize::from(!open)],
                ('\'', Some(open)) => self.single[usize::from(!open)],
                (c, _) => c,
            };
            out.push(r);
            *prev = Some(r);
        }
        out
    }
}

/// Document and character languages: the name, the BCP 47 tag text is shaped in (so that fonts use
/// their language's forms, OpenType `locl`) and the quote characters each uses (choosing a
/// language in Document Setup also picks its quotes). The first is the default.
pub const LANGUAGES: &[(&str, &str, Quotes)] = &[
    ("English: USA", "en-US", Quotes::ENGLISH),
    ("English: UK", "en-GB", Quotes { double: ['“', '”'], single: ['‘', '’'] }),
    ("German", "de", Quotes { double: ['„', '“'], single: ['‚', '‘'] }),
    ("German: Swiss", "de-CH", Quotes { double: ['«', '»'], single: ['‹', '›'] }),
    ("French", "fr", Quotes { double: ['«', '»'], single: ['‹', '›'] }),
    ("Italian", "it", Quotes { double: ['«', '»'], single: ['‘', '’'] }),
    ("Spanish", "es", Quotes { double: ['«', '»'], single: ['‘', '’'] }),
    ("Catalan", "ca", Quotes { double: ['«', '»'], single: ['‘', '’'] }),
    ("Portuguese", "pt", Quotes { double: ['«', '»'], single: ['‘', '’'] }),
    ("Dutch", "nl", Quotes { double: ['“', '”'], single: ['‘', '’'] }),
    ("Danish", "da", Quotes { double: ['»', '«'], single: ['›', '‹'] }),
    ("Swedish", "sv", Quotes { double: ['”', '”'], single: ['’', '’'] }),
    ("Finnish", "fi", Quotes { double: ['”', '”'], single: ['’', '’'] }),
    ("Norwegian", "nb", Quotes { double: ['«', '»'], single: ['‘', '’'] }),
    ("Polish", "pl", Quotes { double: ['„', '”'], single: ['‚', '’'] }),
    ("Czech", "cs", Quotes { double: ['„', '“'], single: ['‚', '‘'] }),
    ("Russian", "ru", Quotes { double: ['«', '»'], single: ['„', '“'] }),
    ("Turkish", "tr", Quotes { double: ['“', '”'], single: ['‘', '’'] }),
    ("Japanese", "ja", Quotes { double: ['「', '」'], single: ['『', '』'] }),
    ("Chinese", "zh", Quotes { double: ['“', '”'], single: ['‘', '’'] }),
    ("Hindi", "hi", Quotes::ENGLISH),
    ("Marathi", "mr", Quotes::ENGLISH),
    ("Gujarati", "gu", Quotes::ENGLISH),
    ("Bengali", "bn", Quotes::ENGLISH),
    ("Punjabi", "pa", Quotes::ENGLISH),
    ("Tamil", "ta", Quotes::ENGLISH),
    ("Telugu", "te", Quotes::ENGLISH),
    ("Kannada", "kn", Quotes::ENGLISH),
    ("Malayalam", "ml", Quotes::ENGLISH),
    ("Odia", "or", Quotes::ENGLISH),
    ("Urdu", "ur", Quotes::ENGLISH),
];

/// The quotes of a [`LANGUAGES`] entry (case-insensitive).
pub fn language_quotes(language: &str) -> Option<Quotes> {
    LANGUAGES.iter().find(|(l, _, _)| l.eq_ignore_ascii_case(language)).map(|(_, _, q)| *q)
}

/// The [`LANGUAGES`] entry named or tagged `language` (case-insensitive) as a character's
/// language ([`crate::CharStyle::language`]) stores it: its tag, or `None` for the default (the
/// first entry); `Err` for a language not in the list.
pub fn char_language(language: &str) -> Result<Option<&'static str>, String> {
    let i = LANGUAGES
        .iter()
        .position(|(n, t, _)| n.eq_ignore_ascii_case(language) || t.eq_ignore_ascii_case(language))
        .ok_or_else(|| format!("unknown language `{language}` (one of {})", LANGUAGES.iter().map(|(n, _, _)| *n).collect::<Vec<_>>().join(", ")))?;
    Ok(LANGUAGES.get(i).filter(|_| i > 0).map(|(_, t, _)| *t))
}

/// The name of the language a character's stored `language` tag stands for: the default's for
/// none, `None` for a tag not in [`LANGUAGES`] (from a file).
pub fn char_language_name(language: Option<&str>) -> Option<&'static str> {
    match language {
        None => LANGUAGES.first().map(|(n, _, _)| *n),
        Some(tag) => LANGUAGES.iter().find(|(_, t, _)| t.eq_ignore_ascii_case(tag)).map(|(n, _, _)| *n),
    }
}

/// The transparency flattener preset Document Setup uses unless another is chosen.
pub const DEFAULT_FLATTENER: &str = "Medium Resolution";

/// Background Contents (New Document): what shows behind the art on the artboards, and what
/// raster exports put there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Background {
    #[default]
    Transparent,
    /// A white artboard background (not an object): raster exports are opaque white there.
    White,
}

impl Background {
    pub const ALL: [Background; 2] = [Background::Transparent, Background::White];
    pub fn id(self) -> &'static str {
        match self {
            Background::Transparent => "transparent",
            Background::White => "white",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id().eq_ignore_ascii_case(s))
    }
}

/// Largest bleed on any side, in points (1 inch).
pub const MAX_BLEED: f64 = 72.0;
/// Largest superscript or subscript size, and the farthest its baseline moves either way, in
/// percent of the font size.
pub const MAX_SCRIPT: f64 = 500.0;
/// Largest synthesized small capitals size, in percent of the font size.
pub const MAX_SMALL_CAPS: f64 = 200.0;

/// Returns `bleed` with each side kept to 0–[`MAX_BLEED`] points; a side that isn't finite becomes 0.
pub fn bleed_in_range(bleed: [f64; 4]) -> [f64; 4] {
    bleed.map(|b| if b.is_finite() { b.clamp(0.0, MAX_BLEED) } else { 0.0 })
}

/// File → Document Setup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocSetup {
    /// Bleed beyond every artboard edge in points: top, bottom, left, right.
    pub bleed: [f64; 4],
    /// Show Images in Outline Mode.
    pub outline_images: bool,
    /// The Highlight Substituted Fonts value older builds kept in Document Setup. Files keep it as
    /// they were saved; the screen follows Preferences › Type › Highlight Substituted Fonts.
    pub highlight_substituted_fonts: bool,
    /// Highlight glyphs the font lacks (drawn from a fallback font).
    pub highlight_substituted_glyphs: bool,
    /// Transparency grid cell size.
    pub grid_size: GridSize,
    /// Transparency grid colours; the first is also the simulated paper colour.
    pub grid_colors: [Color; 2],
    /// Simulate Colored Paper: artboards show the paper colour and art is multiplied onto it.
    pub simulate_paper: bool,
    /// Transparency flattener preset for output that can't keep transparency (None =
    /// [`DEFAULT_FLATTENER`]).
    pub flattener_preset: Option<String>,
    /// Discard White Overprint in Output.
    pub discard_white_overprint: bool,
    /// Document language (one of [`LANGUAGES`]).
    pub language: String,
    pub quotes: Quotes,
    /// Use Typographer's Quotes: typed straight quotes become [`DocSetup::quotes`].
    pub typographers_quotes: bool,
    pub superscript: ScriptMetrics,
    pub subscript: ScriptMetrics,
    /// Synthesized small capitals, percent of the font size.
    pub small_caps_size: f64,
    pub export_text: ExportText,
    /// Background Contents chosen in New Document.
    pub background: Background,
    /// Character panel menu › System Layout: every text object advances by whole points. Off by
    /// default and for documents from before it, and saved only when on.
    /// <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/turn-fractional-character-widths-off-or-on.html>
    #[serde(default, rename = "systemLayout", skip_serializing_if = "crate::skip::is_default")]
    pub system_layout: bool,
}

impl Default for DocSetup {
    fn default() -> Self {
        Self {
            bleed: [0.0; 4],
            outline_images: false,
            highlight_substituted_fonts: true,
            highlight_substituted_glyphs: false,
            grid_size: GridSize::Medium,
            grid_colors: GRID_COLOR_PRESETS[0].1,
            simulate_paper: false,
            flattener_preset: None,
            discard_white_overprint: true,
            language: LANGUAGES[0].0.to_string(),
            quotes: Quotes::ENGLISH,
            typographers_quotes: true,
            superscript: ScriptMetrics::DEFAULT,
            subscript: ScriptMetrics::DEFAULT,
            small_caps_size: 70.0,
            export_text: ExportText::Editable,
            background: Background::Transparent,
            system_layout: false,
        }
    }
}

impl DocSetup {
    /// `artboard` grown by the bleed.
    pub fn bleed_rect(&self, artboard: Rect) -> Rect {
        crate::marks::outset(artboard, self.bleed)
    }
    pub fn has_bleed(&self) -> bool {
        self.bleed.iter().any(|b| *b > 0.0)
    }
    /// The simulated paper colour.
    pub fn paper(&self) -> Color {
        self.grid_colors[0]
    }
    /// The name of the grid colour preset in use, or "Custom".
    pub fn grid_colors_name(&self) -> &'static str {
        GRID_COLOR_PRESETS.iter().find(|(_, c)| *c == self.grid_colors).map_or("Custom", |(n, _)| n)
    }
    /// The flattener preset in use.
    pub fn flattener(&self) -> &str {
        self.flattener_preset.as_deref().unwrap_or(DEFAULT_FLATTENER)
    }
    /// Moves the bleed and the type proportions into the ranges Document Setup takes, each to the
    /// nearer end: the bleed by [`bleed_in_range`], superscript and subscript sizes to
    /// 1–[`MAX_SCRIPT`] and positions to ±[`MAX_SCRIPT`], the small caps size to
    /// 1–[`MAX_SMALL_CAPS`].
    pub fn keep_in_range(&mut self) {
        self.bleed = bleed_in_range(self.bleed);
        for m in [&mut self.superscript, &mut self.subscript] {
            // `clamp` passes NaN through, so a non-finite value falls back to the default first.
            if !m.size.is_finite() {
                m.size = ScriptMetrics::DEFAULT.size;
            }
            if !m.position.is_finite() {
                m.position = ScriptMetrics::DEFAULT.position;
            }
            m.size = m.size.clamp(1.0, MAX_SCRIPT);
            m.position = m.position.clamp(-MAX_SCRIPT, MAX_SCRIPT);
        }
        if !self.small_caps_size.is_finite() {
            self.small_caps_size = DocSetup::default().small_caps_size;
        }
        self.small_caps_size = self.small_caps_size.clamp(1.0, MAX_SMALL_CAPS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Characters store a language of the list by its tag, the default as none.
    #[test]
    fn char_languages_are_stored_by_tag() {
        assert_eq!(char_language("Turkish"), Ok(Some("tr")));
        assert_eq!(char_language("MR"), Ok(Some("mr")));
        assert_eq!(char_language("english: usa"), Ok(None));
        assert_eq!(char_language("en-US"), Ok(None));
        assert!(char_language("Klingon").is_err() && char_language("").is_err());
        assert_eq!(
            (char_language_name(None), char_language_name(Some("gu")), char_language_name(Some("x-unknown"))),
            (Some("English: USA"), Some("Gujarati"), None)
        );
        // Every entry has a tag of its own, and a name of its own.
        for (i, (n, t, _)) in LANGUAGES.iter().enumerate() {
            assert!(LANGUAGES.iter().skip(i + 1).all(|(n2, t2, _)| n2 != n && t2 != t), "{n} {t}");
        }
    }

    #[test]
    fn quotes_open_and_close_by_context() {
        let mut prev = None;
        assert_eq!(Quotes::ENGLISH.apply("\"Hi,\" it's 'x'", &mut prev), "“Hi,” it’s ‘x’");
        let german = language_quotes("german").unwrap();
        let mut prev = Some('(');
        assert_eq!(german.apply("\"a\"", &mut prev), "„a“");
        assert_eq!(prev, Some('“'));
        // Converting in pieces keeps the context.
        let mut prev = None;
        let a = Quotes::ENGLISH.apply("\"", &mut prev);
        let b = Quotes::ENGLISH.apply("\"", &mut prev);
        assert_eq!((a.as_str(), b.as_str()), ("“", "“"));
    }

    #[test]
    fn closing_quote_equal_to_other_kinds_opening_quote_still_closes() {
        let q = Quotes { double: ['”', '”'], single: ['‘', '’'] };
        let mut prev = None;
        assert_eq!(q.apply("'\"a\"'", &mut prev), "‘”a”’");
        let q = Quotes { double: ['"', '"'], single: ['‘', '’'] };
        let mut prev = None;
        assert_eq!(q.apply("'\"a\"'", &mut prev), "‘\"a\"’");
        let mut prev = None;
        assert_eq!(q.apply("\"'a'\"", &mut prev), "\"‘a’\"");
    }

    #[test]
    fn bleed_grows_the_artboard() {
        let s = DocSetup { bleed: [9.0, 9.0, 4.0, 2.0], ..Default::default() };
        assert_eq!(s.bleed_rect(Rect::new(0.0, 0.0, 100.0, 50.0)), Rect::new(-4.0, -9.0, 102.0, 59.0));
        assert!(s.has_bleed() && !DocSetup::default().has_bleed());
    }

    #[test]
    fn keep_in_range_replaces_non_finite_type_values() {
        let mut s = DocSetup::default();
        s.superscript.size = f64::NAN;
        s.superscript.position = f64::INFINITY;
        s.subscript.size = f64::NEG_INFINITY;
        s.subscript.position = f64::NAN;
        s.small_caps_size = f64::NAN;
        s.keep_in_range();
        for v in [s.superscript.size, s.superscript.position, s.subscript.size, s.subscript.position, s.small_caps_size] {
            assert!(v.is_finite(), "{v}");
        }
    }

    #[test]
    fn presets_and_defaults() {
        let d = DocSetup::default();
        assert_eq!(d.grid_colors_name(), "Light");
        assert_eq!(d.flattener(), "Medium Resolution");
        assert_eq!(Background::parse("WHITE"), Some(Background::White));
        assert_eq!(GridSize::parse("LARGE"), Some(GridSize::Large));
        assert_eq!(ExportText::parse("appearance"), Some(ExportText::Appearance));
        // A partial setup fills in the rest.
        let s: DocSetup = serde_json::from_str(r#"{"bleed":[1,2,3,4]}"#).unwrap();
        assert_eq!(s, DocSetup { bleed: [1.0, 2.0, 3.0, 4.0], ..DocSetup::default() });
    }
}
