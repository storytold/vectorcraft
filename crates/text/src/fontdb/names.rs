use std::sync::Arc;

use super::{MetadataProvider, StringId};

use crate::cjk::{self, CjkScript};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct SysFallback {
    pub(super) enabled: bool,
    pub(super) misses: std::collections::HashSet<char>,
    /// Counts the scans of the font folders: a search that a scan overtook doesn't record its miss.
    pub(super) scans: u64,
}

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
/// well (Thai, Lao, Khmer, Myanmar); none for other characters.
pub(super) fn script_families(c: char) -> &'static [&'static str] {
    use unicode_script::{Script, UnicodeScript};
    match c.script() {
        Script::Thai => THAI_FAMILIES,
        Script::Lao => &["Lao UI", "Leelawadee UI", "Lao Sangam MN", "Noto Sans Lao", "Noto Sans Lao UI", "Phetsarath OT"],
        Script::Khmer => &["Khmer UI", "Khmer Sangam MN", "Noto Sans Khmer", "Noto Sans Khmer UI", "Khmer OS"],
        Script::Myanmar => &["Myanmar Text", "Myanmar Sangam MN", "Noto Sans Myanmar", "Noto Sans Myanmar UI", "Padauk"],
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

/// Every localized string of `id`, with its language.
pub(super) fn localized(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<(Option<String>, String)> {
    font.localized_strings(id).map(|s| (s.language().map(str::to_owned), s.to_string())).filter(|(_, s)| !s.is_empty()).collect()
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

/// The most name records one name ID is read from, and the most text they may hold. A font can
/// point many records at one large string, or fill the table with ones that name nothing, so the
/// blob's size alone bounds neither the decoding work nor the memory (CWE-770); a menu lists a
/// handful of languages and no family name is longer than a sentence.
pub(super) const MAX_NAME_RECORDS: usize = 64;
const MAX_NAME_BYTES: usize = 8 << 10;

/// Every family name a face writes, in every language: its typographic family (ID 16) records and
/// its family (ID 1) ones, plus the names an old font keeps only in a legacy CJK encoding. What a
/// font menu labels the family by (in the interface's writing system), searches it by and reads
/// its writing systems from. Bounded: the name table is untrusted input.
pub(super) fn localized_family(font: &skrifa::FontRef<'_>) -> Arc<[(String, String)]> {
    let mut names = records_of(font, StringId::TYPOGRAPHIC_FAMILY_NAME);
    let mut seen: Vec<String> = names.iter().map(|(_, name)| name.clone()).collect();
    // The legacy family name is searched too: an extended family's faces can carry different ID 1
    // names ("HYQiHei-45S" beside "HYQiHei"), and old fonts name themselves only there.
    names.extend(records_of(font, StringId::FAMILY_NAME).into_iter().filter(|(_, name)| {
        if seen.contains(name) {
            return false;
        }
        seen.push(name.clone());
        true
    }));
    names.truncate(MAX_NAME_RECORDS);
    Arc::from(names)
}

/// The name `names` gives the family in `script`'s language: the label a font menu shows for a
/// font of that writing system.
pub(super) fn name_for_script(names: &[(String, String)], script: CjkScript) -> Option<String> {
    names.iter().find(|(lang, name)| !name.trim().is_empty() && cjk::locale_script(Some(lang)) == Some(script)).map(|(_, name)| name.clone())
}

/// A face's name records of one ID, without blanks or duplicates: the ones the font stack reads,
/// then names kept only in a legacy CJK encoding ([`legacy_records`]). Bounded by
/// [`MAX_NAME_RECORDS`] records examined — blank ones count, so a font whose records mostly name
/// nothing cannot make the menu walk the whole table — and by [`MAX_NAME_BYTES`] of text.
fn records_of(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<(String, String)> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut bytes = 0usize;
    let mut examined = 0usize;
    for record in font.localized_strings(id) {
        if examined >= MAX_NAME_RECORDS {
            break;
        }
        examined += 1;
        let language = record.language().unwrap_or("").to_string();
        let name = record.to_string();
        if name.trim().is_empty() {
            continue;
        }
        if bytes + name.len() > MAX_NAME_BYTES {
            break;
        }
        bytes += name.len();
        names.push((language, name));
    }
    // The legacy records get what the Unicode ones left of both budgets.
    names.extend(legacy_records(font, id, MAX_NAME_BYTES.saturating_sub(bytes), MAX_NAME_RECORDS.saturating_sub(examined)));
    let mut seen: Vec<(String, String)> = Vec::with_capacity(names.len());
    names.retain(|name| {
        if seen.contains(name) {
            return false;
        }
        seen.push(name.clone());
        true
    });
    names
}

/// Name records the font stack can't read, from fonts that keep a localized name only in a legacy
/// CJK encoding: Mac platform records (Japanese Shift-JIS, Traditional Chinese Big5, Korean
/// EUC-KR, Simplified Chinese GB2312) and Windows ones (encodings 2-5). Decoded here so an old
/// Chinese, Japanese or Korean font still lists its own name; the tag comes from the encoding,
/// which is what these records are written in. Nothing on the web, which has no system fonts and
/// must stay under its size gate.
#[cfg(not(target_arch = "wasm32"))]
fn legacy_records(font: &skrifa::FontRef<'_>, id: StringId, bytes_left: usize, records_left: usize) -> Vec<(String, String)> {
    /// The encoding a name record may use besides Unicode, and the language its text is in.
    fn codec(platform: u16, encoding: u16) -> Option<(&'static encoding_rs::Encoding, &'static str)> {
        use encoding_rs::{BIG5, EUC_KR, GBK, SHIFT_JIS};
        match (platform, encoding) {
            (1, 1) | (3, 2) => Some((SHIFT_JIS, "ja")),
            (1, 2) | (3, 4) => Some((BIG5, "zh-Hant")),
            (1, 3) | (3, 5) => Some((EUC_KR, "ko")),
            (1, 25) | (3, 3) => Some((GBK, "zh-Hans")),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let Some(table) = font.table_data(super::Tag::new(b"name")) else {
        return out;
    };
    let bytes = table.as_bytes();
    let be16 = |at: usize| -> Option<u16> { Some(u16::from_be_bytes(<[u8; 2]>::try_from(bytes.get(at..at.checked_add(2)?)?).ok()?)) };
    let (Some(count), Some(strings)) = (be16(2), be16(4).map(usize::from)) else {
        return out;
    };
    let mut budget = bytes_left;
    let mut examined = 0usize;
    for i in 0..usize::from(count) {
        if examined >= records_left {
            break;
        }
        let record = 6 + i * 12;
        let (Some(platform), Some(encoding), Some(name), Some(length), Some(offset)) =
            (be16(record), be16(record + 2), be16(record + 6), be16(record + 8), be16(record + 10))
        else {
            continue;
        };
        if name != id.to_u16() {
            continue;
        }
        let Some((codec, language)) = codec(platform, encoding) else {
            continue;
        };
        // The allowance pays for decoding, so only a record this reader can decode uses it up.
        examined += 1;
        let Some(start) = strings.checked_add(usize::from(offset)) else {
            continue;
        };
        let Some(raw) = start.checked_add(usize::from(length)).and_then(|end| bytes.get(start..end)) else {
            continue;
        };
        let (text, _, had_errors) = codec.decode(raw);
        if had_errors || text.trim().is_empty() {
            continue;
        }
        let text = text.into_owned();
        if text.len() > budget {
            break;
        }
        budget -= text.len();
        out.push((language.to_string(), text));
    }
    out
}

#[cfg(target_arch = "wasm32")]
fn legacy_records(_font: &skrifa::FontRef<'_>, _id: StringId, _bytes_left: usize, _records_left: usize) -> Vec<(String, String)> {
    Vec::new()
}
