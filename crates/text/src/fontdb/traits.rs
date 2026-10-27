use super::face::{MAX_AXES, Variation};
use super::names::{MAX_LEGACY_NAMES, all_names, localized, name, native_family_name, norm};
use super::scan::named_instances;
use super::*;

/// The names a face answers to besides its (English) family and style, normalized ([`norm`]):
/// its family in every language (typographic, else legacy: `ヒラギノ角ゴシック`), its style likewise
/// (`ミディアム`), its legacy family names (`Hiragino Sans W3`) with their legacy styles, paired per
/// language record (the Mac and Windows records of a face differ), its full names (name ID 4),
/// and its PostScript names (`HiraginoSans-W3`, as PDF and `.ai` files name fonts).
#[derive(Clone, Debug, Default)]
pub(super) struct FaceKeys {
    pub(super) families: Vec<String>,
    pub(super) styles: Vec<String>,
    pub(super) legacy: Vec<(String, String)>,
    pub(super) full_names: Vec<String>,
    pub(super) postscript: Vec<String>,
}

impl FaceKeys {
    pub(super) fn of(font: &skrifa::FontRef<'_>, family: &str, style: &str) -> Self {
        let (typo_family, typo_style) = (all_names(font, StringId::TYPOGRAPHIC_FAMILY_NAME), all_names(font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME));
        let (mut legacy_family, mut legacy_style) = (localized(font, StringId::FAMILY_NAME), localized(font, StringId::SUBFAMILY_NAME));
        // Records repeat (platforms, encodings) and are paired below: once each, and no more than a
        // font has any use for, or the pairs multiply.
        for names in [&mut legacy_family, &mut legacy_style] {
            names.sort();
            names.dedup();
            names.truncate(MAX_LEGACY_NAMES);
        }
        // Without typographic names, the legacy ones are the family's.
        let family_names: Vec<&str> = if typo_family.is_empty() {
            legacy_family.iter().map(|(_, f)| f.as_str()).collect()
        } else {
            typo_family.iter().map(String::as_str).collect()
        };
        let mut families: Vec<String> = std::iter::once(family).chain(family_names).map(norm).collect();
        // A legacy style only names the face beside its legacy family when the font has a
        // typographic style: every weight of `Hiragino Sans` is `Regular` in its own legacy family.
        let style_names: Vec<&str> = if typo_style.is_empty() {
            legacy_style.iter().map(|(_, s)| s.as_str()).collect()
        } else {
            typo_style.iter().map(String::as_str).collect()
        };
        let mut styles: Vec<String> = std::iter::once(style).chain(style_names).map(norm).collect();
        let mut legacy: Vec<(String, String)> = if typo_family.is_empty() {
            vec![]
        } else {
            legacy_family
                .iter()
                .flat_map(|(lf, f)| legacy_style.iter().filter(move |(ls, _)| ls == lf).map(move |(_, s)| (norm(f), norm(s))))
                .filter(|(f, s)| !f.is_empty() && !s.is_empty())
                .collect()
        };
        let mut postscript: Vec<String> = all_names(font, StringId::POSTSCRIPT_NAME).iter().map(|n| norm(n)).collect();
        let mut full_names: Vec<String> = all_names(font, StringId::FULL_NAME).iter().map(|n| norm(n)).collect();
        for v in [&mut families, &mut styles, &mut full_names, &mut postscript] {
            v.retain(|k| !k.is_empty());
            v.sort();
            v.dedup();
        }
        legacy.sort();
        legacy.dedup();
        FaceKeys { families, styles, legacy, full_names, postscript }
    }
}

/// A font's kind, for filtering font menus: the classification of its OS/2 table (IBM family
/// class, PANOSE) and, for CJK fonts whose tables seldom say, its names (明朝, ゴシック, 丸…).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontClass {
    /// Serif (Latin) or Mincho (CJK).
    Serif,
    /// Sans serif (Latin) or Gothic (CJK).
    Sans,
    /// Rounded sans / Maru Gothic.
    Rounded,
    /// Script, handwriting, brush (楷書・行書・教科書…).
    Script,
    Monospaced,
    /// Decorative, display, symbol and pictorial fonts.
    Decorative,
    #[default]
    Other,
}

impl FontClass {
    /// The kinds a font menu filters by (all but `Other`).
    pub const ALL: [FontClass; 6] =
        [FontClass::Serif, FontClass::Sans, FontClass::Rounded, FontClass::Script, FontClass::Monospaced, FontClass::Decorative];
}

/// What a font menu filters on: whether a family sets Japanese or Thai, its kind, and whether it
/// is a variable font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontTraits {
    pub japanese: bool,
    /// Its character map has Thai (ก and า).
    pub thai: bool,
    pub class: FontClass,
    /// The face has variation axes (an `fvar` table of at most 64 axes); for a family, one of
    /// its faces has.
    pub variable: bool,
}

impl FontTraits {
    /// From a face's OS/2 and `fvar` tables (when it has them) and names.
    fn of(font: &skrifa::FontRef<'_>, keys: &FaceKeys) -> Self {
        use skrifa::raw::TableProvider;
        let os2 = font.os2().ok();
        let names: Vec<&str> = keys.families.iter().chain(&keys.postscript).map(String::as_str).collect();
        let named = |words: &[&str]| names.iter().any(|n| words.iter().any(|w| n.contains(w)));
        let has_japanese_name = keys.families.iter().any(|k| k.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x4E00..=0x9FFF)));
        let (class_id, panose, ranges2, codepages) = match &os2 {
            Some(o) => (o.s_family_class(), o.panose_10().to_vec(), o.ul_unicode_range_2(), o.ul_code_page_range_1().unwrap_or(0)),
            None => (0, vec![], 0, 0),
        };
        // Hiragana (bit 49) or Katakana (bit 50) in the Unicode ranges, or the JIS code page.
        let japanese = ranges2 & (0b11 << 17) != 0 || codepages & (1 << 17) != 0 || has_japanese_name;
        let thai = ['\u{0E01}', '\u{0E32}'].iter().all(|c| font.charmap().map(*c).is_some());
        let family_type = panose.first().copied().unwrap_or(0);
        let serif_style = panose.get(1).copied().unwrap_or(0);
        let proportion = panose.get(3).copied().unwrap_or(0);
        let ibm = (class_id >> 8) as u8;
        let class = if named(&["丸", "maru", "rounded", "round"]) || japanese && named(&["jun"]) {
            FontClass::Rounded
        } else if named(&[
            "楷書",
            "行書",
            "草書",
            "隷書",
            "教科書",
            "毛筆",
            "筆",
            "kaisho",
            "gyosho",
            "sosho",
            "reisho",
            "kyokasho",
            "brush",
            "script",
            "hand",
            "kaiti",
            "xingkai",
            "libian",
        ]) {
            FontClass::Script
        } else if named(&["mono", "code", "courier", "menlo", "consol"]) {
            FontClass::Monospaced
        } else if named(&[
            "明朝",
            "mincho",
            "ryumin",
            "hiramin",
            "yumin",
            "kozmin",
            "serif",
            "songti",
            "simsun",
            "stsong",
            "fangsong",
            "myungjo",
            "times",
            "georgia",
            "garamond",
            "baskerville",
            "bodoni",
            "caslon",
            "palatino",
            "didot",
            "century",
            "cambria",
            "minion",
            "charter",
            "hoefler",
        ]) && !named(&["sansserif", "sans"])
        {
            FontClass::Serif
        } else if named(&[
            "ゴシック",
            "gothic",
            "goth",
            "sans",
            "kakugo",
            "kaku",
            "heiti",
            "helvetica",
            "arial",
            "grotesk",
            "grotesque",
            "futura",
            "gill",
            "verdana",
            "tahoma",
            "frutiger",
            "avenir",
            "geneva",
            "lucidagrande",
            "inter",
            "roboto",
            "optima",
            "pingfang",
        ]) {
            FontClass::Sans
        } else if family_type == 2 && proportion == 9 {
            FontClass::Monospaced
        } else if family_type == 3 || ibm == 10 {
            FontClass::Script
        } else if family_type == 4 || family_type == 5 || ibm == 9 || ibm == 12 {
            FontClass::Decorative
        } else if matches!(ibm, 1..=5 | 7) || family_type == 2 && (2..=10).contains(&serif_style) {
            FontClass::Serif
        } else if ibm == 8 || family_type == 2 && (11..=13).contains(&serif_style) {
            FontClass::Sans
        } else {
            FontClass::Other
        };
        let variable = (1..=MAX_AXES).contains(&font.axes().len());
        FontTraits { japanese, thai, class, variable }
    }
}

/// What a name other than a family's own one stands for ([`FontDb::canonical`]).
#[derive(Clone, Debug)]
pub(super) struct Alias {
    /// The family's own name.
    pub(super) family: String,
    /// The face the name names, for legacy family names, full names and PostScript names.
    pub(super) style: Option<String>,
    /// A legacy family name's legacy style (`Regular` beside `Hiragino Sans W3`).
    pub(super) paired: Option<String>,
}

impl Alias {
    /// The aliases of a face `family` / `style` with `keys`, by normalized name.
    pub(super) fn of(family: &str, style: &str, keys: &FaceKeys) -> Vec<(String, Alias)> {
        #[cfg(test)]
        ALIAS_LISTS.with(|c| c.set(c.get() + 1));
        let mut v: Vec<(String, Alias)> =
            keys.families.iter().map(|k| (k.clone(), Alias { family: family.to_string(), style: None, paired: None })).collect();
        v.extend(
            keys.legacy
                .iter()
                .map(|(f, s)| (f.clone(), Alias { family: family.to_string(), style: Some(style.to_string()), paired: Some(s.clone()) })),
        );
        v.extend(
            keys.postscript
                .iter()
                .chain(&keys.full_names)
                .map(|k| (k.clone(), Alias { family: family.to_string(), style: Some(style.to_string()), paired: None })),
        );
        v
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: counts the faces whose aliases ([`Alias::of`]) were listed on this thread.
    static ALIAS_LISTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Tests: returns how many faces' aliases were listed on this thread.
#[cfg(test)]
pub(crate) fn alias_lists() -> u64 {
    ALIAS_LISTS.with(std::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    /// Tests: run on this thread each time it publishes faces to a database, to look the database
    /// up at exactly that point (as another thread could).
    static ON_FACES_PUBLISHED: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

/// Tests: run `hook` each time this thread publishes faces to a database.
#[cfg(test)]
pub(crate) fn on_faces_published(hook: Option<Box<dyn FnMut()>>) {
    ON_FACES_PUBLISHED.with(|h| *h.borrow_mut() = hook);
}

#[cfg(test)]
pub(super) fn faces_published() {
    // Taken out while it runs, so a hook that adds faces itself doesn't run again.
    if let Some(mut hook) = ON_FACES_PUBLISHED.with(|h| h.borrow_mut().take()) {
        hook();
        ON_FACES_PUBLISHED.with(|h| {
            h.borrow_mut().get_or_insert(hook);
        });
    }
}

/// How a requested family and style resolved ([`FontDb::resolve`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontMatch {
    /// The face of that family and style (by any of its names, in any language).
    Exact,
    /// The family is there but not the style: its closest style stands in.
    Style,
    /// The family is unknown: the fallback family stands in.
    Missing,
}

impl FontMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            FontMatch::Exact => "exact",
            FontMatch::Style => "substitute",
            FontMatch::Missing => "missing",
        }
    }
}

/// A face's (family, style) names.
pub(super) fn face_names(f: &skrifa::FontRef<'_>) -> Option<(String, String)> {
    let family = name(f, &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME])?;
    let style = name(f, &[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]).unwrap_or_else(|| "Regular".into());
    Some((family, style))
}

/// One style a font file offers: one of its faces, or a named instance of a variable font
/// (Figtree's `Figtree[wght].ttf` holds Light, the default, and Regular to Black as instances).
#[derive(Clone, Debug)]
pub(crate) struct FaceStyle {
    pub(super) family: String,
    /// See [`FontFace::native_family`].
    pub(super) native_family: Option<String>,
    pub(super) style: String,
    pub(super) keys: FaceKeys,
    /// A named instance's axis settings (user units); empty for the face itself.
    pub(super) variations: Vec<Variation>,
    /// A named instance's weight (its `wght` setting); `None` for the face itself (its OS/2 table
    /// says) or an instance of a font without a weight axis.
    pub(super) weight: Option<f32>,
    /// A named instance is italic (by its `ital` or `slnt` setting, or its name).
    pub(super) italic: bool,
    /// Its kind, for font menus' filters (the face's, shared by its named instances).
    pub(super) traits: FontTraits,
}

/// The styles of a face: the face itself, then its named instances when it is a variable font.
pub(crate) fn face_styles(f: &skrifa::FontRef<'_>) -> Vec<FaceStyle> {
    let Some((family, style)) = face_names(f) else { return vec![] };
    let native_family = native_family_name(f, &family);
    let mut keys = FaceKeys::of(f, &family, &style);
    let traits = FontTraits::of(f, &keys);
    let (mut instances, renamed) = named_instances(f, &family, &style, &mut keys, native_family.clone());
    for i in &mut instances {
        i.traits = traits;
    }
    // A variable font's default face renders at its `wght` default, whatever its name table and
    // OS/2 table say (Noto Sans Hebrew: named `Light`, weight class 300, default instance Regular
    // at 400): weigh it so, and name it by its default instance when that is another weight.
    let weight = f
        .axes()
        .iter()
        .find(|a| a.tag().to_be_bytes() == *b"wght")
        .map(|a| a.default_value())
        .filter(|w| w.is_finite())
        .map(|w| w.clamp(1.0, 1000.0));
    let style = renamed.unwrap_or(style);
    let face = FaceStyle { family, native_family, style, keys, variations: vec![], weight, italic: false, traits };
    std::iter::once(face).chain(instances).collect()
}
