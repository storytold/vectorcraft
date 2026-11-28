//! Fonts from the optional craft-fonts build input (<https://github.com/storytold/craft-fonts>).
//!
//! Built with `CRAFT_FONTS_DIR=<craft-fonts checkout>`, [`CRAFT_FONTS`] holds every font in its
//! manifest (Japanese UI and document fonts, Arabic fonts and Noto Sans Thai); built without it,
//! it is empty and that text falls back to the installed system fonts. Release builds always set it. See `build.rs`.

pub use craft_fonts_build::EmbeddedFace;
#[cfg(target_arch = "wasm32")]
pub use craft_fonts_build::ServedFace;

/// The craft-fonts faces embedded in the build (empty unless built with `CRAFT_FONTS_DIR`).
pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// The craft-fonts faces the web build's interface font stack fetches from beside the wasm when its
/// text needs them: the `startup` and `demand` lines of `crates/text/web-fonts.txt` (empty unless
/// built with `CRAFT_FONTS_DIR`).
#[cfg(target_arch = "wasm32")]
pub static SERVED_FONTS: &[ServedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts_served.rs"));

/// A craft-fonts face the web (wasm32) build fetches from beside the wasm instead of embedding,
/// listed in `crates/text/web-fonts.txt` (empty unless built with `CRAFT_FONTS_DIR`). Fetched into
/// the document font database by `apps/vectorcraft-web/src/fonts.rs`; copied into the site by
/// `cargo xtask web-fonts`.
pub struct WebFont {
    pub family: &'static str,
    pub style: &'static str,
    /// Fetched before the app starts (a fallback the first frame needs).
    pub startup: bool,
    /// Fetched only by the interface font stack, when its text needs one of the face's scripts
    /// ([`SERVED_FONTS`]). With neither this nor `startup`, fetched in the background.
    pub demand: bool,
    /// ISO 15924 scripts the face is for (its craft-fonts manifest row), e.g. `"Hang"`.
    pub scripts: &'static [&'static str],
    /// Site-relative URL, `fonts/<first 16 hex digits of the SHA-256>/<file name>`.
    pub url: &'static str,
    /// Subresource Integrity value, `sha256-<base64>`; the browser rejects other bytes.
    pub integrity: &'static str,
}

/// The faces of `web-fonts.txt` (empty unless built with `CRAFT_FONTS_DIR`).
pub static WEB_FONTS: &[WebFont] = include!(concat!(env!("OUT_DIR"), "/web_fonts.rs"));

/// A serif (Mincho) family, for document text.
fn is_mincho(f: &EmbeddedFace) -> bool {
    f.family.contains("Mincho")
}

/// A sans (Gothic) family, for UI text.
fn is_gothic(f: &EmbeddedFace) -> bool {
    f.family.contains("Gothic")
}

/// A bold face.
fn is_bold(f: &EmbeddedFace) -> bool {
    f.weight() >= 600
}

/// The craft-fonts faces for Japanese text, best for document text first: Mincho (serif) families,
/// then the others, regular weights before bold. Empty when built without craft-fonts.
pub fn japanese_document_fonts() -> Vec<&'static EmbeddedFace> {
    japanese_document_fonts_from(CRAFT_FONTS)
}

fn japanese_document_fonts_from(fonts: &[EmbeddedFace]) -> Vec<&EmbeddedFace> {
    let mut v: Vec<_> = fonts.iter().filter(|f| f.has_script("Jpan")).collect();
    v.sort_by_key(|f| (!is_mincho(f), is_bold(f)));
    v
}

/// The craft-fonts faces the document font database loads: the Japanese ones in
/// [`japanese_document_fonts`] order, then the Arabic ones in manifest order (Noto Sans Arabic
/// first: the fallback for Arabic text; the other families are picked by name), then the Thai ones
/// (Noto Sans Thai: the fallback for Thai text where no Thai font is installed, and on the web),
/// then the Georgian ones (Noto Sans Georgian, the same for Georgian). Remaining faces follow in manifest order, so every explicitly supplied family can be selected.
/// Empty when built without craft-fonts.
pub fn document_fonts() -> Vec<&'static EmbeddedFace> {
    document_fonts_from(CRAFT_FONTS)
}

fn document_fonts_from(fonts: &[EmbeddedFace]) -> Vec<&EmbeddedFace> {
    let mut v = japanese_document_fonts_from(fonts);
    let rest = |f: &&EmbeddedFace, earlier: &[&str]| !f.has_script("Jpan") && !earlier.iter().any(|s| f.has_script(s));
    v.extend(fonts.iter().filter(|f| f.has_script("Arab") && rest(f, &[])));
    v.extend(fonts.iter().filter(|f| f.has_script("Thai") && rest(f, &["Arab"])));
    v.extend(fonts.iter().filter(|f| f.has_script("Geor") && rest(f, &["Arab", "Thai"])));
    v.extend(fonts.iter().filter(|f| rest(f, &["Arab", "Thai", "Geor"])));
    v
}

/// The craft-fonts faces for Japanese text, best for UI text first: Gothic (sans) families, then
/// the others; `bold` puts bold faces before regular ones. Empty when built without craft-fonts.
pub fn japanese_ui_fonts(bold: bool) -> Vec<&'static EmbeddedFace> {
    let mut v: Vec<&'static EmbeddedFace> = CRAFT_FONTS.iter().filter(|f| f.has_script("Jpan")).collect();
    v.sort_by_key(|f| (!is_gothic(f), is_bold(f) != bold));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_fonts_keep_script_priority_and_include_every_face_once() {
        let font = |family, style, scripts| EmbeddedFace { family, style, scripts, index: 0, bytes: &[], role: None };
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
        let expected: Vec<_> =
            ["NanumGothic", "Noto Sans Devanagari", "Noto Sans Hebrew"].into_iter().filter(|n| CRAFT_FONTS.iter().any(|f| f.family == *n)).collect();
        assert_eq!(demand, expected);
        assert!(WEB_FONTS.iter().all(|w| !(w.startup && w.demand)));
    }
}
