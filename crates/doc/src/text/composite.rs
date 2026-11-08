//! Type › Composite Fonts…: a font made of other fonts, one for each class of character (the
//! Kanji font is the base; kana, punctuation, symbols, Roman letters and numerals each take a font
//! of their own, sized, raised and scaled against it). Type set in a composite font names it as its
//! family; the layout sets each character in its class's font.
//!
//! Behavior reference (public documentation):
//! <https://help.adobe.com/archive/en_US/illustrator/cs3/illustrator_cs3_help.pdf#page=347>.
//! Which characters fall in which class, the ranges and the defaults are VectorCraft's own.

use serde::{Deserialize, Serialize};

/// A class of characters a composite font gives a font of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharClass {
    /// The base font: ideographs, full-width letters and digits, and every character no other
    /// class takes.
    Kanji,
    /// Hiragana and katakana, full-width and half-width.
    Kana,
    /// East Asian and full-width punctuation, and typographic quotes and dashes.
    Punctuation,
    /// Arrows, math, shapes, enclosed and other symbols.
    Symbols,
    /// Half-width Roman letters, their punctuation and spaces.
    Roman,
    /// The half-width digits 0 to 9.
    Numerals,
}

impl CharClass {
    /// Every class, in the order the Composite Fonts dialog lists them.
    pub const ALL: [CharClass; 6] = [Self::Kanji, Self::Kana, Self::Punctuation, Self::Symbols, Self::Roman, Self::Numerals];

    /// The name commands and files use (`kanji`, `kana`, …).
    pub fn key(self) -> &'static str {
        match self {
            Self::Kanji => "kanji",
            Self::Kana => "kana",
            Self::Punctuation => "punctuation",
            Self::Symbols => "symbols",
            Self::Roman => "roman",
            Self::Numerals => "numerals",
        }
    }

    /// The class named `key` ([`Self::key`], any case).
    pub fn named(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key().eq_ignore_ascii_case(key))
    }

    /// Whether the class's characters can be scaled across and up (Kana, Roman and Numerals).
    pub fn scales(self) -> bool {
        matches!(self, Self::Kana | Self::Roman | Self::Numerals)
    }

    /// The class of character `c`.
    pub fn of(c: char) -> Self {
        let u = c as u32;
        match u {
            0x30..=0x39 => Self::Numerals,
            // Basic Latin, Latin-1, Latin Extended, IPA, spacing and combining marks, and Latin
            // Extended Additional.
            0x00..=0x036F | 0x1E00..=0x1EFF => Self::Roman,
            // The other Latin blocks: combining marks extended, phonetic extensions and their
            // supplement, combining marks supplement, Latin Extended-C, -D, -E, -F and -G, and the
            // Latin ligatures of the alphabetic presentation forms.
            0x1AB0..=0x1AFF
            | 0x1D00..=0x1DFF
            | 0x2C60..=0x2C7F
            | 0xA720..=0xA7FF
            | 0xAB30..=0xAB6F
            | 0xFB00..=0xFB06
            | 0x10780..=0x107BF
            | 0x1DF00..=0x1DFFF => Self::Roman,
            // ・ and ゠ sit in the katakana block but are punctuation.
            0x30A0 | 0x30FB => Self::Punctuation,
            // Hiragana, katakana, katakana phonetic extensions, half-width katakana (with its
            // prolonged sound and voicing marks) and the kana supplements.
            0x3041..=0x309F | 0x30A1..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F | 0x1B000..=0x1B16F => Self::Kana,
            // CJK symbols: 〄 〒 〓 〠 〶; ideographic space, 々 〆 〇 and the like stay with the base.
            0x3004 | 0x3012 | 0x3013 | 0x3020 | 0x3036 => Self::Symbols,
            0x3000 | 0x3005..=0x3007 | 0x3021..=0x302F | 0x3031..=0x3035 | 0x3038..=0x303C => Self::Kanji,
            0x3001..=0x303F => Self::Punctuation,
            // ※ is a symbol; the rest of General Punctuation (dashes, quotes, leaders) punctuation.
            0x203B => Self::Symbols,
            0x2010..=0x205E => Self::Punctuation,
            // Letterlike symbols, number forms, arrows, math, technical, enclosed alphanumerics, box
            // drawing, shapes, miscellaneous symbols and dingbats; enclosed CJK and CJK
            // compatibility; full-width signs.
            0x2100..=0x2BFF | 0x3200..=0x33FF | 0xFFE0..=0xFFEE => Self::Symbols,
            // Full-width forms of the ASCII signs.
            0xFF03..=0xFF06 | 0xFF0A | 0xFF0B | 0xFF1C..=0xFF1E | 0xFF20 | 0xFF3C | 0xFF3E | 0xFF5C | 0xFF5E => Self::Symbols,
            // Full-width forms of the other ASCII punctuation, and the half-width CJK punctuation.
            0xFF01..=0xFF0F | 0xFF1A..=0xFF1F | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65 => Self::Punctuation,
            _ => Self::Kanji,
        }
    }
}

/// The font a composite font sets one class of characters in, and how.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositePart {
    /// The font family and style.
    pub font: String,
    #[serde(default = "regular")]
    pub style: String,
    /// The size, in percent of the type's size.
    #[serde(default = "hundred")]
    pub size: f64,
    /// How far the characters are raised from the baseline, in percent of the type's size
    /// (negative lowers them).
    #[serde(default)]
    pub baseline: f64,
    /// Horizontal and vertical scale, in percent (Kana, Roman and Numerals only).
    #[serde(default = "hundred")]
    pub h_scale: f64,
    #[serde(default = "hundred")]
    pub v_scale: f64,
    /// Kana only: the characters are scaled from the center of the em box rather than from the
    /// baseline.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scale_at_center: bool,
}

fn hundred() -> f64 {
    100.0
}

fn regular() -> String {
    "Regular".into()
}

impl CompositePart {
    /// The part a new composite font gives every class: `font` in `style` at full size, on the
    /// baseline, unscaled.
    pub fn new(font: impl Into<String>, style: impl Into<String>) -> Self {
        Self { font: font.into(), style: style.into(), size: 100.0, baseline: 0.0, h_scale: 100.0, v_scale: 100.0, scale_at_center: false }
    }
}

/// The six parts of a composite font, one per [`CharClass`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompositeParts {
    pub kanji: CompositePart,
    pub kana: CompositePart,
    pub punctuation: CompositePart,
    pub symbols: CompositePart,
    pub roman: CompositePart,
    pub numerals: CompositePart,
}

impl CompositeParts {
    /// Every class set in `part`.
    pub fn all(part: CompositePart) -> Self {
        Self { kanji: part.clone(), kana: part.clone(), punctuation: part.clone(), symbols: part.clone(), roman: part.clone(), numerals: part }
    }

    pub fn get(&self, class: CharClass) -> &CompositePart {
        match class {
            CharClass::Kanji => &self.kanji,
            CharClass::Kana => &self.kana,
            CharClass::Punctuation => &self.punctuation,
            CharClass::Symbols => &self.symbols,
            CharClass::Roman => &self.roman,
            CharClass::Numerals => &self.numerals,
        }
    }

    pub fn get_mut(&mut self, class: CharClass) -> &mut CompositePart {
        match class {
            CharClass::Kanji => &mut self.kanji,
            CharClass::Kana => &mut self.kana,
            CharClass::Punctuation => &mut self.punctuation,
            CharClass::Symbols => &mut self.symbols,
            CharClass::Roman => &mut self.roman,
            CharClass::Numerals => &mut self.numerals,
        }
    }
}

/// A composite font (Type › Composite Fonts…), saved with the document. Its name is the family
/// that type set in it names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompositeFont {
    pub name: String,
    pub parts: CompositeParts,
}

/// The size a part can take, in percent of the type's size.
pub const COMPOSITE_SIZE: std::ops::RangeInclusive<f64> = 10.0..=400.0;
/// How far a part can raise or lower its characters, in percent of the type's size.
pub const COMPOSITE_BASELINE: std::ops::RangeInclusive<f64> = -100.0..=100.0;
/// The horizontal and vertical scale a part can take, in percent.
pub const COMPOSITE_SCALE: std::ops::RangeInclusive<f64> = 10.0..=400.0;
/// The longest composite font name, in characters.
pub const COMPOSITE_NAME_MAX: usize = 255;
/// The most composite fonts a document or a file keeps.
pub const COMPOSITE_FONTS_MAX: usize = 256;

impl CompositeFont {
    /// A composite font named `name` setting every class in `font` and `style`.
    pub fn new(name: impl Into<String>, font: impl Into<String>, style: impl Into<String>) -> Self {
        Self { name: name.into(), parts: CompositeParts::all(CompositePart::new(font, style)) }
    }

    /// Do `a` and `b` name the same composite font: equal in any case?
    pub fn same_name(a: &str, b: &str) -> bool {
        a == b || a.to_lowercase() == b.to_lowercase()
    }

    /// The font of `fonts` (a document's composite fonts) named `name`, in any case.
    pub fn named<'a>(fonts: &'a [CompositeFont], name: &str) -> Option<&'a CompositeFont> {
        fonts.iter().find(|f| Self::same_name(&f.name, name))
    }

    /// `name` trimmed, when it can name a composite font: not empty, at most
    /// [`COMPOSITE_NAME_MAX`] characters and without control characters.
    pub fn clean_name(name: &str) -> Option<String> {
        let name = name.trim();
        let ok = !name.is_empty() && name.chars().count() <= COMPOSITE_NAME_MAX && !name.chars().any(char::is_control);
        ok.then(|| name.to_string())
    }

    /// Why this composite font can't be used, if it can't: a name [`Self::clean_name`] refuses, a
    /// part without a font, a value out of its range, or a scale or Scale at Center on a class
    /// that has none.
    pub fn check(&self) -> Result<(), String> {
        if Self::clean_name(&self.name).as_deref() != Some(self.name.as_str()) {
            return Err(format!("`{}` can't name a composite font", self.name));
        }
        for class in CharClass::ALL {
            let p = self.parts.get(class);
            let k = class.key();
            if p.font.trim().is_empty() {
                return Err(format!("{k}: give a font"));
            }
            let within = |v: f64, r: &std::ops::RangeInclusive<f64>| v.is_finite() && r.contains(&v);
            if !within(p.size, &COMPOSITE_SIZE) {
                return Err(format!("{k}: size must be {} to {}%", COMPOSITE_SIZE.start(), COMPOSITE_SIZE.end()));
            }
            if !within(p.baseline, &COMPOSITE_BASELINE) {
                return Err(format!("{k}: baseline must be {} to {}%", COMPOSITE_BASELINE.start(), COMPOSITE_BASELINE.end()));
            }
            for v in [p.h_scale, p.v_scale] {
                if !within(v, &COMPOSITE_SCALE) {
                    return Err(format!("{k}: scale must be {} to {}%", COMPOSITE_SCALE.start(), COMPOSITE_SCALE.end()));
                }
                if v != 100.0 && !class.scales() {
                    return Err(format!("{k}: only kana, roman and numerals can be scaled"));
                }
            }
            if p.scale_at_center && class != CharClass::Kana {
                return Err(format!("{k}: only kana can be scaled at the center"));
            }
        }
        Ok(())
    }

    /// `fonts` without the ones that can't be used: those [`Self::check`] refuses, a later font
    /// sharing an earlier one's name (in any case), a part set in one of the fonts, and those past
    /// [`COMPOSITE_FONTS_MAX`]. `None` when every font can be used.
    pub fn usable(fonts: &[CompositeFont]) -> Option<Vec<CompositeFont>> {
        let mut names = std::collections::HashSet::new();
        let mut kept: Vec<CompositeFont> =
            fonts.iter().filter(|f| f.check().is_ok() && names.insert(f.name.to_lowercase())).take(COMPOSITE_FONTS_MAX).cloned().collect();
        kept.retain(|f| CharClass::ALL.iter().all(|&c| !names.contains(&f.parts.get(c).font.to_lowercase())));
        (kept.len() != fonts.len()).then_some(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each class takes its characters: digits and Latin letters are half-width numerals and
    /// Roman, kana both widths, and the punctuation, symbols and full-width forms their own.
    #[test]
    fn characters_fall_in_their_classes() {
        for (s, class) in [
            ("漢字国一龍〇々", CharClass::Kanji),
            ("ＡＺａ０９　", CharClass::Kanji),
            ("あをアンーｱﾝｰ", CharClass::Kana),
            ("、。「」『』【】〜・！？（）：", CharClass::Punctuation),
            ("“”‘’—…", CharClass::Punctuation),
            ("※→★○〒＋＝＄％①㈱", CharClass::Symbols),
            ("AZaz é!? ()", CharClass::Roman),
            // Latin Extended-C, -D and -E, phonetic extensions and a ligature.
            ("\u{2C60}\u{A7A0}\u{AB30}\u{1D00}\u{FB01}", CharClass::Roman),
            ("0123456789", CharClass::Numerals),
        ] {
            for c in s.chars() {
                assert_eq!(CharClass::of(c), class, "{c:?} (U+{:04X})", c as u32);
            }
        }
    }

    /// A composite font is read and written by its fields' names, and a part's omitted values
    /// take the defaults.
    #[test]
    fn composite_fonts_read_back_with_defaults() {
        let mut f = CompositeFont::new("Mixed", "Source Sans 3", "Regular");
        f.parts.kana.v_scale = 90.0;
        f.parts.kana.scale_at_center = true;
        f.parts.roman.baseline = -5.0;
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["parts"]["kana"]["vScale"], 90.0);
        assert_eq!(json["parts"]["kana"]["scaleAtCenter"], true);
        assert!(json["parts"]["roman"].get("scaleAtCenter").is_none());
        assert_eq!(serde_json::from_value::<CompositeFont>(json).unwrap(), f);
        let part: CompositePart = serde_json::from_value(serde_json::json!({"font": "Inter"})).unwrap();
        assert_eq!(part, CompositePart::new("Inter", "Regular"));
    }

    /// Names are trimmed; values out of their ranges, and scales on classes that don't scale, are
    /// refused.
    #[test]
    fn composite_fonts_check_their_values() {
        assert_eq!(CompositeFont::clean_name("  Mixed  ").as_deref(), Some("Mixed"));
        for bad in ["", "   ", "a\tb", &"x".repeat(COMPOSITE_NAME_MAX + 1)] {
            assert_eq!(CompositeFont::clean_name(bad), None, "{bad:?}");
        }
        let ok = CompositeFont::new("Mixed", "Inter", "Regular");
        assert_eq!(ok.check(), Ok(()));
        let with = |f: fn(&mut CompositeParts)| {
            let mut c = ok.clone();
            f(&mut c.parts);
            c.check()
        };
        assert!(with(|p| p.roman.size = 9.0).is_err());
        assert!(with(|p| p.roman.size = f64::NAN).is_err());
        assert!(with(|p| p.kana.baseline = 101.0).is_err());
        assert!(with(|p| p.kana.h_scale = 401.0).is_err());
        assert!(with(|p| p.kanji.v_scale = 90.0).is_err());
        assert!(with(|p| p.symbols.h_scale = 90.0).is_err());
        assert!(with(|p| p.roman.scale_at_center = true).is_err());
        assert!(with(|p| p.numerals.font = " ".into()).is_err());
        assert_eq!(with(|p| p.kana.scale_at_center = true), Ok(()));
        assert_eq!(with(|p| p.numerals.h_scale = 50.0), Ok(()));
        let mut spaced = ok.clone();
        spaced.name = " Mixed".into();
        assert!(spaced.check().is_err());
    }
}
