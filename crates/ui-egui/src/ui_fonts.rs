//! The app's interface font stack: the craft faces it embeds (Japanese, Georgian, Devanagari,
//! Gujarati), JetBrains Mono for monospace, and the framework's loader for every other script.
//! The stack is installed once by the shell (`embed::install`).

use std::sync::Arc;

use craft::ui::fonts::{Family, I18nScripts, SEMIBOLD, UiFonts};

/// The interface fonts configuration. Characters no face covers are looked up in the installed
/// system fonts on native; the web build has the faces its server offers instead.
pub fn ui_fonts() -> UiFonts<'static> {
    let fonts =
        UiFonts::new(&I18nScripts).craft_faces(vectorcraft_text::CRAFT_FONTS).family(Family::monospace()).family(Family::named(SEMIBOLD).weight(600));
    #[cfg(not(target_arch = "wasm32"))]
    {
        fonts.on_demand(Arc::new(craft::ui::fonts::SystemCandidates))
    }
    #[cfg(target_arch = "wasm32")]
    {
        // The fetcher (`BrowserFetcher`) lives in the web app and is added by the web shell's install.
        fonts.on_demand(Arc::new(craft::ui::fonts::NoCandidates)).served_faces(vectorcraft_text::SERVED_FONTS)
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::fonts::{FontRef, Sample, UiFonts, assert_covers, missing_glyphs};

    use super::*;

    const HINDI: &str = "हिन्दी क्षि त्रि क्रि प्रिंट फ़ाइल लेयर RGB 123";
    const JAPANESE: &str = "日本語の文字";

    fn embeds(script: &str) -> bool {
        vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.has_script(script))
    }

    #[test]
    fn hindi_ui_covers_devanagari_from_the_craft_faces() {
        if !embeds("Deva") {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
            return;
        }
        assert_covers(&ui_fonts().stack(), [Sample::new("hi", "hindi", HINDI)]);
    }

    #[test]
    fn japanese_renders_with_the_craft_fonts_without_installed_fonts() {
        if !embeds("Jpan") {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
            return;
        }
        assert_covers(&ui_fonts().stack(), [Sample::new("ja", "japanese", JAPANESE)]);
    }

    #[test]
    fn the_ui_works_without_the_craft_fonts() {
        let stack = UiFonts::new(&I18nScripts).stack();
        let family = FontRef::proportional();
        assert!(missing_glyphs(&stack, &family, [Sample::new("en", "latin", "Layer 1")], false).is_empty());
        assert!(!missing_glyphs(&stack, &family, [Sample::new("ja", "japanese", JAPANESE)], false).is_empty());
    }
}
