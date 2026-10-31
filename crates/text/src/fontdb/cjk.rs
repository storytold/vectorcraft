//! Which script's Han ideographs the interface asked for.
//!
//! Simplified and Traditional Chinese share nearly all of their code points and differ only in the
//! glyph forms, and Japanese kanji are the same code points again, so coverage cannot tell them
//! apart: the interface language has to, and a Japanese interface's kanji must not come from a
//! Chinese face.

/// Which script's Han ideographs the interface asked for (see [`FontDb::set_cjk_preference`]).
///
/// [`FontDb::set_cjk_preference`]: super::db::FontDb::set_cjk_preference
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CjkPreference {
    /// The language names no Han script (`en`, `ko`, `uk`...). The lookup keeps the order it had
    /// before the choice existed: the loaded faces first, then the general `SYSTEM_FALLBACKS` list.
    #[default]
    Unspecified,
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
}

impl CjkPreference {
    /// The preference for a resolved interface language code (`zh-hans`, `zh-hant`, `zh-TW`, `ja`,
    /// `en`...). A bare `zh` is Simplified, as in the interface's own language resolution; a language
    /// that names no Han script has no preference, so a document opened in it draws exactly as it
    /// did before.
    pub fn for_language(code: &str) -> Self {
        let tag = code.trim().to_ascii_lowercase();
        if tag.starts_with("zh") {
            if asks_traditional_chinese(&tag) { Self::TraditionalChinese } else { Self::SimplifiedChinese }
        } else if tag.starts_with("ja") {
            Self::Japanese
        } else {
            Self::Unspecified
        }
    }

    /// The Han families to load for this preference, empty when the language named no Han script.
    pub(super) fn families(self) -> &'static [&'static str] {
        match self {
            Self::Unspecified => &[],
            Self::SimplifiedChinese => HAN_SIMPLIFIED_FAMILIES,
            Self::TraditionalChinese => HAN_TRADITIONAL_FAMILIES,
            Self::Japanese => HAN_JAPANESE_FAMILIES,
        }
    }
}

/// Han ideograph families for Simplified Chinese, in the order the platforms' own fallback tables
/// prefer them (macOS, Windows, then the Linux/Noto names). Missing ones cost one lookup each.
pub(super) const HAN_SIMPLIFIED_FAMILIES: &[&str] = &[
    "PingFang SC",
    "Heiti SC",
    "STHeiti",
    "Microsoft YaHei",
    "SimSun",
    "Hiragino Sans GB",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
];
/// The same for Traditional Chinese. One list per script, because a Simplified face covers the
/// Traditional glyphs' code points and would otherwise win (`东` against `東`'s neighbours); the
/// faces here are the Taiwanese and Hong Kong ones, not a Simplified face as a last resort.
pub(super) const HAN_TRADITIONAL_FAMILIES: &[&str] =
    &["PingFang TC", "Heiti TC", "Hiragino Sans CNS", "Microsoft JhengHei", "MingLiU", "Noto Sans CJK TC", "Noto Sans TC", "Source Han Sans TC"];
/// Japanese faces for kanji, so a Japanese interface keeps the Japanese forms.
pub(super) const HAN_JAPANESE_FAMILIES: &[&str] = &[
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Yu Gothic",
    "Meiryo",
    "MS Gothic",
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Source Han Sans JP",
    "IPAGothic",
    "TakaoGothic",
];

/// Whether a CJK tag asks for Traditional Chinese: a `Hant` script, or a Taiwan, Hong Kong or Macao
/// region (`zh-TW`, `zh_HK`, `zh_MO.UTF-8`). Anything else Chinese - including a bare `zh` - is
/// Simplified.
fn asks_traditional_chinese(tag: &str) -> bool {
    tag.split(['-', '_', '.']).any(|p| matches!(p, "hant" | "tw" | "hk" | "mo"))
}

/// Whether `c` belongs to the group the CJK preference faces are drawn for: Han ideographs, kana and
/// bopomofo (plus the compatibility and extension blocks). Not Hangul, and not the Latin, Greek or
/// Cyrillic ranges those faces also cover - a preference that loads a CJK face for one of those
/// would answer the character before the script-aware list gets to (#1424 review).
pub(crate) fn is_han_script(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF | 0xFF65..=0xFF9F)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The language code picks the preference, and a language with no Han script gets none - which is
    /// what keeps the fallback order it had before the preference existed.
    #[test]
    fn the_interface_language_picks_the_han_script_or_none() {
        for tag in ["zh-hant", "zh-Hant-TW", "zh_TW", "zh_HK", "zh_MO.UTF-8"] {
            assert_eq!(CjkPreference::for_language(tag), CjkPreference::TraditionalChinese, "{tag}");
        }
        for tag in ["zh-hans", "zh-Hans-CN", "zh_CN.UTF-8", "zh", "zh-SG"] {
            assert_eq!(CjkPreference::for_language(tag), CjkPreference::SimplifiedChinese, "{tag}");
        }
        assert_eq!(CjkPreference::for_language("ja-JP"), CjkPreference::Japanese);
        for tag in ["en-US", "de", "ko", ""] {
            assert_eq!(CjkPreference::for_language(tag), CjkPreference::Unspecified, "{tag}");
            assert!(CjkPreference::for_language(tag).families().is_empty(), "{tag} must name no Han family");
        }
    }

    #[test]
    fn the_preference_applies_to_the_han_script_group_only() {
        // The faces a preference names draw Han, kana and bopomofo - and, as a side effect, Latin,
        // Greek and Cyrillic. Only the first group may be resolved through the preference; the rest
        // keep the general list, which knows which face draws their script.
        for c in ['東', '文', '骨', 'あ', 'ア', 'ㄅ', 'ｶ', '\u{20000}', '\u{30000}', '\u{3ffff}'] {
            assert!(is_han_script(c), "{c} is in the Han-script group");
        }
        for c in ['Ж', 'A', 'ß', 'α', '한', 'ก', 'প', '0', ' ', '\u{ff64}', '\u{40000}'] {
            assert!(!is_han_script(c), "{c} must not be resolved through a Han preference");
        }
    }

    /// One list per script, with no Simplified face in the Traditional or the Japanese one: a
    /// Simplified face covers the same code points and would win the lookup.
    #[test]
    fn the_lists_do_not_borrow_from_the_other_scripts() {
        for fam in HAN_TRADITIONAL_FAMILIES.iter().chain(HAN_JAPANESE_FAMILIES) {
            assert!(!HAN_SIMPLIFIED_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(fam)), "{fam}");
        }
        for fam in HAN_JAPANESE_FAMILIES {
            assert!(!HAN_TRADITIONAL_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(fam)), "{fam}");
        }
    }
}
