//! Fonts from the optional craft-fonts build input (<https://github.com/storytold/craft-fonts>).
//!
//! Built with `CRAFT_FONTS_DIR=<craft-fonts checkout>`, [`CRAFT_FONTS`] holds every font in its
//! manifest (Japanese UI and document fonts, Arabic fonts and Noto Sans Thai); built without it,
//! it is empty and that text falls back to the installed system fonts. Release builds always set it. See `build.rs`.

/// A font from the optional craft-fonts build input (empty unless built with `CRAFT_FONTS_DIR`).
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    /// ISO 15924 scripts the font is for, e.g. `"Jpan"`.
    pub scripts: &'static [&'static str],
    pub bytes: &'static [u8],
}

/// A craft-fonts face the web (wasm32) build fetches from beside the wasm instead of embedding,
/// listed in `crates/text/web-fonts.txt` (empty unless built with `CRAFT_FONTS_DIR`). Fetched by
/// `apps/vectorcraft-web/src/fonts.rs`; copied into the site by `cargo xtask web-fonts`.
pub struct WebFont {
    pub family: &'static str,
    pub style: &'static str,
    /// Fetched before the app starts (a fallback the first frame needs).
    pub startup: bool,
    /// Fetched only when the interface needs a character of one of [`WebFont::scripts`] (see
    /// [`demand_fonts`]). With neither this nor `startup`, fetched in the background.
    pub demand: bool,
    /// ISO 15924 scripts the face is for (its craft-fonts manifest row), e.g. `"Hang"`.
    pub scripts: &'static [&'static str],
    /// Site-relative URL, `fonts/<first 16 hex digits of the SHA-256>/<file name>`.
    pub url: &'static str,
    /// Subresource Integrity value, `sha256-<base64>`; the browser rejects other bytes.
    pub integrity: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

impl CraftFont {
    /// Is the font for Japanese text?
    pub fn is_japanese(&self) -> bool {
        self.scripts.contains(&"Jpan")
    }
    /// Is the font for Arabic text?
    pub fn is_arabic(&self) -> bool {
        self.scripts.contains(&"Arab")
    }
    /// Is the font for Thai text?
    pub fn is_thai(&self) -> bool {
        self.scripts.contains(&"Thai")
    }
    /// Is the font for Georgian text?
    pub fn is_georgian(&self) -> bool {
        self.scripts.contains(&"Geor")
    }
    /// A serif (Mincho) family, for document text.
    pub fn is_mincho(&self) -> bool {
        self.family.contains("Mincho")
    }
    /// A sans (Gothic) family, for UI text.
    pub fn is_gothic(&self) -> bool {
        self.family.contains("Gothic")
    }
    /// A bold face.
    pub fn is_bold(&self) -> bool {
        crate::style_weight(self.style) >= 600.0
    }
}

/// The craft-fonts faces for Japanese text, best for document text first: Mincho (serif) families,
/// then the others, regular weights before bold. Empty when built without craft-fonts.
pub fn japanese_document_fonts() -> Vec<&'static CraftFont> {
    japanese_document_fonts_from(CRAFT_FONTS)
}

fn japanese_document_fonts_from(fonts: &[CraftFont]) -> Vec<&CraftFont> {
    let mut v: Vec<_> = fonts.iter().filter(|f| f.is_japanese()).collect();
    v.sort_by_key(|f| (!f.is_mincho(), f.is_bold()));
    v
}

/// The craft-fonts faces the document font database loads: the Japanese ones in
/// [`japanese_document_fonts`] order, then the Arabic ones in manifest order (Noto Sans Arabic
/// first: the fallback for Arabic text; the other families are picked by name), then the Thai ones
/// (Noto Sans Thai: the fallback for Thai text where no Thai font is installed, and on the web),
/// then the Georgian ones (Noto Sans Georgian, the same for Georgian). Remaining faces follow in manifest order, so every explicitly supplied family can be selected.
/// Empty when built without craft-fonts.
pub fn document_fonts() -> Vec<&'static CraftFont> {
    document_fonts_from(CRAFT_FONTS)
}

fn document_fonts_from(fonts: &[CraftFont]) -> Vec<&CraftFont> {
    let mut v = japanese_document_fonts_from(fonts);
    v.extend(fonts.iter().filter(|f| f.is_arabic() && !f.is_japanese()));
    v.extend(fonts.iter().filter(|f| f.is_thai() && !f.is_japanese() && !f.is_arabic()));
    v.extend(fonts.iter().filter(|f| f.is_georgian() && !f.is_japanese() && !f.is_arabic() && !f.is_thai()));
    v.extend(fonts.iter().filter(|f| !f.is_japanese() && !f.is_arabic() && !f.is_thai() && !f.is_georgian()));
    v
}

/// The craft-fonts faces for Georgian UI text (Noto Sans Georgian), in manifest order. The UI fonts
/// have no Georgian letters. Empty when built without craft-fonts, and on the web, which fetches
/// the face at startup (`web-fonts.txt`) instead of embedding it.
pub fn georgian_ui_fonts() -> Vec<&'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.is_georgian() && !f.is_japanese()).collect()
}

/// The craft-fonts faces for Japanese text, best for UI text first: Gothic (sans) families, then
/// the others; `bold` puts bold faces before regular ones. Empty when built without craft-fonts.
pub fn japanese_ui_fonts(bold: bool) -> Vec<&'static CraftFont> {
    let mut v: Vec<&'static CraftFont> = CRAFT_FONTS.iter().filter(|f| f.is_japanese()).collect();
    v.sort_by_key(|f| (!f.is_gothic(), f.is_bold() != bold));
    v
}

/// The script of `c` that a `demand` web face is fetched for: `"Hang"` for Hangul (jamo,
/// compatibility jamo, syllables and the halfwidth forms) and `"Deva"` for Devanagari (with its
/// extended blocks and the Vedic extensions). `None` for every other character.
pub fn demand_script(c: char) -> Option<&'static str> {
    match c as u32 {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xFFA0..=0xFFDC => Some("Hang"),
        0x0900..=0x097F | 0x1CD0..=0x1CFF | 0xA8E0..=0xA8FF | 0x11B00..=0x11B5F => Some("Deva"),
        _ => None,
    }
}

/// The indexes in `fonts` of the `demand` faces for the scripts of `chars` (by
/// [`demand_script`]), each once, in `fonts` order.
pub fn demand_fonts(fonts: &[WebFont], chars: impl IntoIterator<Item = char>) -> Vec<usize> {
    let mut scripts: Vec<&str> = Vec::new();
    for s in chars.into_iter().filter_map(demand_script) {
        if !scripts.contains(&s) {
            scripts.push(s);
        }
    }
    fonts.iter().enumerate().filter(|(_, f)| f.demand && f.scripts.iter().any(|s| scripts.contains(s))).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web_font(family: &'static str, startup: bool, demand: bool, scripts: &'static [&'static str]) -> WebFont {
        WebFont { family, style: "Regular", startup, demand, scripts, url: "", integrity: "" }
    }

    #[test]
    fn demand_scripts_of_hangul_and_devanagari() {
        for c in ['한', '국', 'ㄱ', 'ᄀ', 'ﾡ'] {
            assert_eq!(demand_script(c), Some("Hang"), "{c}");
        }
        for c in ['ह', 'ि', '्', '़', '।', '\u{A8F2}'] {
            assert_eq!(demand_script(c), Some("Deva"), "{c}");
        }
        for c in ['A', 'é', 'ا', 'ก', 'ა', '日', 'ロ', '中'] {
            assert_eq!(demand_script(c), None, "{c}");
        }
    }

    #[test]
    fn demand_fonts_follow_the_scripts_painted() {
        let fonts = [
            web_font("Arabic", true, false, &["Arab"]),
            web_font("Picked", false, false, &["Hang"]),
            web_font("Korean", false, true, &["Hang", "Kore"]),
            web_font("Devanagari", false, true, &["Deva"]),
        ];
        assert_eq!(demand_fonts(&fonts, "한국어".chars()), [2], "Korean fetches only the Korean face");
        assert_eq!(demand_fonts(&fonts, "हिन्दी".chars()), [3], "Hindi fetches only the Devanagari face");
        assert_eq!(demand_fonts(&fonts, "हिन्दी 한국어 Layer".chars()), [2, 3], "each once, in list order");
        assert!(demand_fonts(&fonts, "English العربية 日本語".chars()).is_empty(), "other scripts fetch nothing");
        assert!(demand_fonts(&[], "한국어".chars()).is_empty(), "built without craft-fonts: nothing to fetch");
    }

    #[test]
    fn document_fonts_keep_script_priority_and_include_every_face_once() {
        let font = |family, style, scripts| CraftFont { family, style, scripts, bytes: &[] };
        let fonts = [
            font("Chinese", "Regular", &["Hans"]),
            font("Thai", "Regular", &["Thai", "Arab"]),
            font("Gothic", "Regular", &["Jpan"]),
            font("Mincho", "Bold", &["Jpan"]),
            font("Arabic", "Regular", &["Arab"]),
            font("Mincho", "Regular", &["Jpan", "Hans"]),
            font("Thai only", "Regular", &["Thai"]),
            font("Georgian", "Regular", &["Geor"]),
            font("Latin", "Regular", &["Latn"]),
        ];
        let ordered: Vec<_> = document_fonts_from(&fonts).into_iter().map(|f| (f.family, f.style)).collect();
        assert_eq!(
            ordered,
            [
                ("Mincho", "Regular"),
                ("Mincho", "Bold"),
                ("Gothic", "Regular"),
                ("Thai", "Regular"),
                ("Arabic", "Regular"),
                ("Thai only", "Regular"),
                ("Georgian", "Regular"),
                ("Chinese", "Regular"),
                ("Latin", "Regular"),
            ]
        );
        assert!(document_fonts_from(&[]).is_empty());
    }

    #[test]
    fn web_fonts_follow_the_list() {
        if CRAFT_FONTS.is_empty() {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        }
        // The listed faces this craft-fonts checkout has (an older one lacks some; build.rs leaves
        // those out with a warning).
        let listed: Vec<_> = crate::web_fonts_build::parse_list(include_str!("../web-fonts.txt"))
            .unwrap()
            .into_iter()
            .filter(|l| CRAFT_FONTS.iter().any(|f| f.family == l.family && f.style == l.style))
            .collect();
        assert_eq!(WEB_FONTS.len(), listed.len());
        for (l, w) in listed.iter().zip(WEB_FONTS) {
            assert_eq!((l.family, l.style, l.startup, l.demand), (w.family, w.style, w.startup, w.demand));
            let face = CRAFT_FONTS.iter().find(|f| f.family == w.family && f.style == w.style);
            assert_eq!(face.map(|f| f.scripts), Some(w.scripts), "{} {}: the manifest's scripts", w.family, w.style);
            let (dir, name) = w.url.strip_prefix("fonts/").and_then(|r| r.split_once('/')).unwrap();
            assert!(dir.len() == 16 && dir.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{}", w.url);
            assert!(name.ends_with(".ttf") || name.ends_with(".otf"), "{}", w.url);
            assert!(w.integrity.starts_with("sha256-") && w.integrity.len() == "sha256-".len() + 44, "{}", w.integrity);
        }
        // The fallbacks the first frame may need: Noto Sans Arabic, Noto Sans Thai and Noto Sans
        // Georgian, where the checkout has them.
        let startup: Vec<_> = WEB_FONTS.iter().filter(|w| w.startup).map(|w| w.family).collect();
        let expected: Vec<_> =
            ["Noto Sans Arabic", "Noto Sans Thai", "Noto Sans Georgian"].into_iter().filter(|n| CRAFT_FONTS.iter().any(|f| f.family == *n)).collect();
        assert_eq!(startup, expected);
        // Korean and Devanagari faces are fetched only on demand, never at startup or in the
        // background queue every session runs.
        let demand: Vec<_> = WEB_FONTS.iter().filter(|w| w.demand).map(|w| w.family).collect();
        let expected: Vec<_> = ["NanumGothic", "Noto Sans Devanagari"].into_iter().filter(|n| CRAFT_FONTS.iter().any(|f| f.family == *n)).collect();
        assert_eq!(demand, expected);
        assert!(WEB_FONTS.iter().all(|w| !(w.startup && w.demand)));
    }
}
