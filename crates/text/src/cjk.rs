//! CJK writing systems for the font menus and fallbacks: which of Simplified Chinese,
//! Traditional Chinese, Japanese and Korean a font serves, which one the interface is in, and the
//! order a menu lists names in.
//!
//! Han characters are shared between the four, but their preferred forms differ, so a font menu
//! lists the interface's own fonts first and sorts names the way that writing system sorts them:
//! GB2312's pinyin rows for Simplified Chinese, Big5's stroke order for Traditional Chinese,
//! JIS X 0208's gojūon order for Japanese and KS X 1001's for Korean. Sorting by code point is
//! not that order at all ("优设标题黑" first, "阿里巴巴" last), and a locale collation would need
//! ICU's data; the standard sets' own order is what Illustrator and Photoshop list these fonts in.

/// A CJK writing system with its own preferred fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CjkScript {
    Japanese,
    SimplifiedChinese,
    TraditionalChinese,
    Korean,
}

impl CjkScript {
    /// The ISO 15924 code the craft-fonts font manifest and OpenType script tags use.
    pub const fn iso_code(self) -> &'static str {
        match self {
            CjkScript::Japanese => "Jpan",
            CjkScript::SimplifiedChinese => "Hans",
            CjkScript::TraditionalChinese => "Hant",
            CjkScript::Korean => "Kore",
        }
    }

    /// The writing system a UI language code names (`zh-hans`, `zh-hant`, `ja`, `ko`), or `None`
    /// for every other language.
    pub fn of_language(code: &str) -> Option<CjkScript> {
        locale_script(Some(code))
    }
}

/// The writing systems a font serves, from what it says about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CjkScripts {
    pub hans: bool,
    pub hant: bool,
    pub jpan: bool,
    pub kore: bool,
}

impl CjkScripts {
    pub const NONE: Self = Self { hans: false, hant: false, jpan: false, kore: false };

    /// The scripts of a set, in [`CjkScript`] order.
    pub fn has(self, script: CjkScript) -> bool {
        match script {
            CjkScript::SimplifiedChinese => self.hans,
            CjkScript::TraditionalChinese => self.hant,
            CjkScript::Japanese => self.jpan,
            CjkScript::Korean => self.kore,
        }
    }

    pub fn any(self) -> bool {
        self.hans || self.hant || self.jpan || self.kore
    }

    /// Both sets' scripts together (a family of faces that serve different ones).
    pub fn or(self, other: Self) -> Self {
        Self {
            hans: self.hans || other.hans,
            hant: self.hant || other.hant,
            jpan: self.jpan || other.jpan,
            kore: self.kore || other.kore,
        }
    }

    pub fn set(&mut self, script: CjkScript) {
        match script {
            CjkScript::SimplifiedChinese => self.hans = true,
            CjkScript::TraditionalChinese => self.hant = true,
            CjkScript::Japanese => self.jpan = true,
            CjkScript::Korean => self.kore = true,
        }
    }
}

/// What kind of CJK character this is, for ordering names and for the scripts a font covers, or
/// `None` for everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CjkChar {
    /// Ideographs, CJK punctuation and full-width forms: font choice follows the locale.
    Han,
    Kana,
    Hangul,
    Bopomofo,
}

pub fn classify(c: char) -> Option<CjkChar> {
    let u = c as u32;
    Some(match u {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF | 0xFFA0..=0xFFDC => CjkChar::Hangul,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F | 0x1B000..=0x1B16F => CjkChar::Kana,
        0x3100..=0x312F | 0x31A0..=0x31BF => CjkChar::Bopomofo,
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3190..=0x319F | 0x31C0..=0x31EF | 0x3200..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF => CjkChar::Han,
        0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF64 | 0xFFE0..=0xFFEF | 0x20000..=0x3FFFF => CjkChar::Han,
        _ => return None,
    })
}

/// A locale tag reduced to what [`locale_script`] compares: lowercase, `_` as `-`, without an
/// encoding or modifier suffix.
fn normalized_locale(locale: Option<&str>) -> String {
    locale.unwrap_or("").split(['.', '@']).next().unwrap_or("").to_ascii_lowercase().replace('_', "-")
}

/// The CJK script a language tag asks for (`zh-Hans`, `zh-Hant`, `zh-CN`, `zh-TW`, `ja-JP`,
/// `ko-KR`), or `None` for a non-CJK (or unknown) language.
pub fn locale_script(locale: Option<&str>) -> Option<CjkScript> {
    use CjkScript::*;
    let tag = normalized_locale(locale);
    let mut parts = tag.split('-');
    let lang = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    match lang {
        "ja" => Some(Japanese),
        "ko" => Some(Korean),
        "zh" | "yue" => {
            let hant = rest.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"));
            let hans = rest.iter().any(|p| matches!(*p, "hans" | "cn" | "sg"));
            if (hant || lang == "yue") && !hans { Some(TraditionalChinese) } else { Some(SimplifiedChinese) }
        }
        _ => None,
    }
}

/// The generated tables (`build.rs`): each writing system's standard set, its characters sorted
/// by code point and the rank each has in the set's own order.
mod tables {
    include!(concat!(env!("OUT_DIR"), "/collation.rs"));
}

/// One writing system's characters and their order.
struct Set {
    chars: &'static [char],
    ranks: &'static [u16],
}

impl Set {
    /// The character's rank in the set's order; `None` for a character the set doesn't have.
    fn rank(&self, c: char) -> Option<u32> {
        self.chars.binary_search(&c).ok().and_then(|i| self.ranks.get(i)).map(|r| u32::from(*r))
    }
}

fn set(script: CjkScript) -> Set {
    match script {
        CjkScript::SimplifiedChinese => Set { chars: &tables::HANS_CHARS, ranks: &tables::HANS_RANKS },
        CjkScript::TraditionalChinese => Set { chars: &tables::HANT_CHARS, ranks: &tables::HANT_RANKS },
        CjkScript::Japanese => Set { chars: &tables::JPAN_CHARS, ranks: &tables::JPAN_RANKS },
        CjkScript::Korean => Set { chars: &tables::KORE_CHARS, ranks: &tables::KORE_RANKS },
    }
}

/// ASCII first (letters, digits, spaces and punctuation alike, in code point order), then the
/// script's own characters in its set's order, then the other scripts' characters, then
/// everything else, each group by code point.
const SET: u32 = 1 << 24;
const OTHER_SCRIPT: u32 = 2 << 24;
const OTHER: u32 = 3 << 24;

/// The sort key of a menu label for `script`: compare keys as sequences, then the English family
/// names, and the rows come out in menu order.
pub fn menu_sort_key(label: &str, script: Option<CjkScript>) -> Vec<u32> {
    let set = script.map(set);
    label.chars().map(|c| rank_of(c, set.as_ref())).collect()
}

fn rank_of(c: char, set: Option<&Set>) -> u32 {
    let c = fold(c);
    // All ASCII, a space or a comma included: those sort with the letters, so "Open Sans" reads
    // before "OpenSymbol" and "Aa 厚底" before "Aa厚底" instead of after every Han character.
    if c.is_ascii() {
        return u32::from(c);
    }
    if let Some(rank) = set.and_then(|s| s.rank(c)) {
        return SET | rank;
    }
    if classify(c).is_some() { OTHER_SCRIPT | u32::from(c) } else { OTHER | u32::from(c) }
}

/// The form of a character the menu sorts by: full-width ASCII forms are the letters they stand
/// for ("ＭＳ ゴシック" sorts with "MS"), katakana fold onto the hiragana they share the gojūon
/// order with ("ヒカリ" among "ひかり"), and capitals onto lower case.
fn fold(c: char) -> char {
    let c = if ('\u{FF01}'..='\u{FF5E}').contains(&c) { char::from_u32(c as u32 - 0xFEE0).unwrap_or(c) } else { c };
    let c = if ('\u{30A1}'..='\u{30F6}').contains(&c) { char::from_u32(c as u32 - 0x60).unwrap_or(c) } else { c };
    c.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(label: &str, script: CjkScript) -> Vec<u32> {
        menu_sort_key(label, Some(script))
    }

    /// The pinyin order Photoshop and Illustrator list Chinese fonts in: "阿里巴巴" (ba) before
    /// "阿里健康" (jian), which code point order reverses.
    #[test]
    fn simplified_chinese_sorts_by_pinyin() {
        let mut names = ["未来荧黑", "阿里巴巴普惠体", "阿里健康体", "寒蝉德黑体", "优设标题黑", "站酷高端黑"];
        names.sort_by_cached_key(|n| key(n, CjkScript::SimplifiedChinese));
        assert_eq!(names, ["阿里巴巴普惠体", "阿里健康体", "寒蝉德黑体", "未来荧黑", "优设标题黑", "站酷高端黑"]);
    }

    /// Japanese sorts in gojūon order, with katakana folded onto hiragana; a name that starts
    /// with Latin letters still leads, and full-width letters are the letters they stand for.
    #[test]
    fn japanese_sorts_by_gojuon() {
        let mut names = ["游ゴシック", "ヒカリ角ゴ", "ＭＳ ゴシック", "あおぞら"];
        names.sort_by_cached_key(|n| key(n, CjkScript::Japanese));
        assert_eq!(names, ["ＭＳ ゴシック", "あおぞら", "ヒカリ角ゴ", "游ゴシック"]);
    }

    /// Latin names come first, whatever the script, and compare letter by letter.
    #[test]
    fn latin_names_lead_and_compare_letter_by_letter() {
        let mut names = ["阿里巴巴普惠体", "Aa新华霜剑体", "Aa剑豪体", "Aa厚底黑"];
        names.sort_by_cached_key(|n| key(n, CjkScript::SimplifiedChinese));
        assert_eq!(names, ["Aa厚底黑", "Aa剑豪体", "Aa新华霜剑体", "阿里巴巴普惠体"]);
        let folded = |a: &str, b: &str| key(a, CjkScript::SimplifiedChinese) < key(b, CjkScript::SimplifiedChinese);
        assert_eq!(key("adobe", CjkScript::SimplifiedChinese), key("Adobe", CjkScript::SimplifiedChinese), "case is ignored");
        assert!(folded("ＡＣＭＥ", "adobe"), "full-width forms are the same letters: acme < adobe");
        assert!(folded("07NikumaruFont", "Arial"), "digits before letters");
        assert!(folded("Open Sans", "OpenSymbol"), "a space sorts before a letter, as in a dictionary");
        assert!(folded("Aa 厚底", "Aa厚底"), "and before a Han character, so the Latin name still leads");
    }

    /// A character the writing system's set doesn't have sorts after its own characters, by
    /// code point, and a label with no script at all still has a stable key.
    #[test]
    fn uncovered_characters_sort_by_code_point_after_the_script() {
        let mut names = ["漢字", "Arial", "字"];
        names.sort_by_cached_key(|n| key(n, CjkScript::SimplifiedChinese));
        assert_eq!(names, ["Arial", "字", "漢字"], "漢 is not in GB2312, 字 is");
        assert!(key("漢", CjkScript::SimplifiedChinese) > key("字", CjkScript::SimplifiedChinese));
        assert_eq!(menu_sort_key("字", None), vec![OTHER_SCRIPT | u32::from('字')], "no script: still a stable key");
    }

    /// Every writing system has a table with its own characters.
    #[test]
    fn every_writing_system_has_its_characters() {
        for (script, c) in [
            (CjkScript::SimplifiedChinese, '这'),
            (CjkScript::SimplifiedChinese, '说'),
            (CjkScript::TraditionalChinese, '們'),
            (CjkScript::TraditionalChinese, '體'),
            (CjkScript::Japanese, 'あ'),
            (CjkScript::Japanese, 'ゴ'),
            (CjkScript::Korean, '가'),
            (CjkScript::Korean, '한'),
        ] {
            assert!(set(script).rank(c).is_some(), "{c} is in {}'s set", script.iso_code());
        }
    }

    /// A language code names its writing system: Simplified and Traditional Chinese by their
    /// subtags, Japanese and Korean by their language.
    #[test]
    fn language_codes_name_their_writing_system() {
        use CjkScript::*;
        for (code, script) in [
            ("zh-hans", SimplifiedChinese),
            ("zh-cn", SimplifiedChinese),
            ("zh_CN", SimplifiedChinese),
            ("zh-hant", TraditionalChinese),
            ("zh-tw", TraditionalChinese),
            ("zh-HK", TraditionalChinese),
            ("ja", Japanese),
            ("ja-JP", Japanese),
            ("ko", Korean),
            ("ko-KR", Korean),
        ] {
            assert_eq!(CjkScript::of_language(code), Some(script), "{code}");
        }
        assert_eq!(CjkScript::of_language("zh"), Some(SimplifiedChinese), "Chinese without a subtag is Simplified");
        for code in ["en", "de", "fr", "pt-br", ""] {
            assert_eq!(CjkScript::of_language(code), None, "{code} is not CJK");
        }
    }
}
