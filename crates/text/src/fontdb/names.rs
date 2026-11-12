use super::{MetadataProvider, StringId};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct SysFallback {
    pub(super) enabled: bool,
    pub(super) misses: std::collections::HashSet<char>,
    /// Counts the scans of the font folders: a search that a scan overtook doesn't record its miss.
    pub(super) scans: u64,
    /// Characters that went on to read every cataloged font file (and found nothing, or a face).
    /// Each such search rereads the catalog, so only [`MAX_FILE_SEARCHES`] are made per scan.
    pub(super) file_searches: u32,
}

/// The most characters searched for in the font files themselves between two scans of the font
/// folders. Real text has a few uncovered characters; a document of thousands of distinct
/// private-use code points would reread the whole catalog for each.
pub(super) const MAX_FILE_SEARCHES: u32 = 1024;

/// Fonts made for Thai, tried first (when installed) for Thai the font in use lacks: Windows
/// (Leelawadee UI; Tahoma before Windows 8), macOS (Thonburi, Sukhumvit Set) and Linux (Noto,
/// TLWG).
pub const THAI_FAMILIES: &[&str] = &[
    "Leelawadee UI",
    "Leelawadee",
    "Tahoma",
    "Thonburi",
    "Sukhumvit Set",
    "Noto Sans Thai",
    "Noto Sans Thai UI",
    "Noto Sans Thai Looped",
    "Waree",
    "Loma",
    "Garuda",
];

/// The families tried first for character `c`, in a script whose text only fonts made for it set
/// well (Thai, Lao, Khmer, Myanmar, Devanagari); none for other characters.
pub(super) fn script_families(c: char) -> &'static [&'static str] {
    use unicode_script::{Script, UnicodeScript};
    match c.script() {
        Script::Thai => THAI_FAMILIES,
        Script::Lao => &["Lao UI", "Leelawadee UI", "Lao Sangam MN", "Noto Sans Lao", "Noto Sans Lao UI", "Phetsarath OT"],
        Script::Khmer => &["Khmer UI", "Khmer Sangam MN", "Noto Sans Khmer", "Noto Sans Khmer UI", "Khmer OS"],
        Script::Myanmar => &["Myanmar Text", "Myanmar Sangam MN", "Noto Sans Myanmar", "Noto Sans Myanmar UI", "Padauk"],
        Script::Devanagari => &[
            "Nirmala UI",
            "Kohinoor Devanagari",
            "Devanagari Sangam MN",
            "Noto Sans Devanagari",
            "Noto Sans Devanagari UI",
            "Lohit Devanagari",
            "Mangal",
        ],
        _ => &[],
    }
}

/// Families tried (when installed) for characters the loaded fonts lack: CJK, symbols, emoji.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const SYSTEM_FALLBACKS: &[&str] = &[
    "Helvetica Neue",
    "Arial",
    "Segoe UI",
    "Noto Sans",
    "DejaVu Sans",
    "PingFang SC",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Apple SD Gothic Neo",
    "Heiti SC",
    "STHeiti",
    "Microsoft YaHei",
    "Yu Gothic",
    "Malgun Gothic",
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "Arial Unicode MS",
    "Apple Symbols",
    "Segoe UI Symbol",
    "Noto Sans Symbols",
    "Noto Sans Symbols2",
    "Noto Emoji",
    "Segoe UI Emoji",
    "Apple Color Emoji",
    "Noto Color Emoji",
];

pub(super) fn name(font: &skrifa::FontRef<'_>, ids: &[StringId]) -> Option<String> {
    ids.iter().find_map(|id| font.localized_strings(*id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty()))
}

/// The most distinct legacy family (and style) names of a font used as keys.
pub(super) const MAX_LEGACY_NAMES: usize = 64;

/// Records can share their encoded bytes, so a small table names many long strings: stop at this
/// many records or decoded bytes (real fonts have a few dozen short names per id).
const MAX_NAME_RECORDS: usize = 1024;
const MAX_NAME_BYTES: usize = 1 << 20;

/// Every localized string of `id`, with its language.
pub(super) fn localized(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<(Option<String>, String)> {
    let mut bytes = 0usize;
    font.localized_strings(id)
        .take(MAX_NAME_RECORDS)
        .map(|s| (s.language().map(str::to_owned), s.to_string()))
        .take_while(|(_, s)| {
            bytes = bytes.saturating_add(s.len());
            bytes <= MAX_NAME_BYTES
        })
        .filter(|(_, s)| !s.is_empty())
        .collect()
}

/// Whether a name-table language tag is English (Illustrator's "Show Font Names in English").
fn is_english_lang(lang: Option<&str>) -> bool {
    lang.is_some_and(|l| {
        let l = l.to_ascii_lowercase();
        l == "en" || l == "eng" || l.starts_with("en-") || l.starts_with("en_")
    })
}

/// A non-English family label from the name table, for Preferences › Type › Show Font Names in
/// English (off). Prefers a record whose language isn't English; else any label that differs from
/// `english` and isn't plain ASCII. `None` when the font has no other name.
pub(super) fn pick_native_label(english: &str, pairs: &[(Option<String>, String)]) -> Option<String> {
    pairs
        .iter()
        .find(|(lang, s)| !s.eq_ignore_ascii_case(english) && !is_english_lang(lang.as_deref()))
        .or_else(|| pairs.iter().find(|(_, s)| !s.eq_ignore_ascii_case(english) && !s.is_ascii()))
        .map(|(_, s)| s.clone())
}

/// The family's native-language name from the font (typographic, else legacy), when it has one.
pub(super) fn native_family_name(font: &skrifa::FontRef<'_>, english: &str) -> Option<String> {
    pick_native_label(english, &localized(font, StringId::TYPOGRAPHIC_FAMILY_NAME))
        .or_else(|| pick_native_label(english, &localized(font, StringId::FAMILY_NAME)))
}

/// Every localized string of `id`.
pub(super) fn all_names(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<String> {
    localized(font, id).into_iter().map(|(_, s)| s).collect()
}

/// `s` lowercased, without anything but letters and digits.
pub(super) fn norm_chars(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase)
}

pub(super) fn norm(s: &str) -> String {
    norm_chars(s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Devanagari takes a font made for it, not whichever face covering it loaded first.
    #[test]
    fn devanagari_prefers_fonts_made_for_it() {
        assert_eq!(script_families('ह').first(), Some(&"Nirmala UI"));
        assert!(script_families('ह').contains(&"Noto Sans Devanagari"));
        assert!(script_families('a').is_empty());
    }

    #[test]
    fn a_name_table_of_many_records_is_cut_short() {
        let many: Vec<(u16, &str)> = (0..2000).map(|_| (1, "Short")).collect();
        let name = crate::test_fonts::name_table(&many).unwrap_or_default();
        let font = crate::fontdb::sfnt_of(&[(b"name", &name)]).unwrap_or_default();
        let font = skrifa::FontRef::new(&font).unwrap();
        assert_eq!(localized(&font, StringId::FAMILY_NAME).len(), MAX_NAME_RECORDS);
    }
}
