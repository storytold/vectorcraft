//! The `bundled-fonts` feature turned off, as by an embedder that supplies its own fonts at run
//! time: a database starts without the bundled fonts, text in no font lays out with no glyphs
//! instead of failing, and the fonts added later are used. Run with
//! `cargo test -p vectorcraft-text --no-default-features --test without_bundled_fonts`.
#![cfg(not(feature = "bundled-fonts"))]
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use kurbo::Point;
use vectorcraft_doc::CharStyle;
use vectorcraft_text::{CRAFT_FONTS, FALLBACK_FAMILY, FontDb, TextObject, layout};

#[test]
fn fonts_come_from_the_embedder() {
    // No system fonts, as on the web.
    let db = FontDb::with_font_dirs(vec![]);
    db.set_system_fallback(false);
    let families = db.families();
    for bundled in ["Source Sans 3", "Source Serif 4", "Inter", "JetBrains Mono"] {
        assert!(!families.iter().any(|f| f == bundled), "{bundled} is bundled: {families:?}");
    }
    let t = TextObject::point(Point::ZERO, "Hello", CharStyle { size: 24.0, ..CharStyle::default() });
    if CRAFT_FONTS.is_empty() {
        assert!(families.is_empty(), "{families:?}");
        assert!(db.face(FALLBACK_FAMILY, "Regular").is_none());
        assert!(layout(&db, &t).glyphs.is_empty(), "no font, no glyphs");
    }
    // The embedder adds its fonts: text lays out in them, and a family the database lacks (the
    // default style's Source Sans 3 too) falls back to them.
    let font = |file: &str| std::fs::read(format!("{}/../../assets/fonts/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert_eq!(db.add_font(font("Inter-Regular.ttf")), 1);
    assert!(db.families().iter().any(|f| f == "Inter"));
    if CRAFT_FONTS.is_empty() {
        assert_eq!(db.face("No Such Font", "Regular").unwrap().family, "Inter");
    }
    let l = layout(&db, &t);
    assert_eq!(l.glyphs.len(), 5);
    assert!(l.glyphs.iter().all(|g| g.advance > 0.0));
    // With Source Sans 3 added, the usual fallback is back.
    assert_eq!(db.add_font(font("SourceSans3-Regular.ttf")), 1);
    assert_eq!(db.face("No Such Font", "Regular").unwrap().family, FALLBACK_FAMILY);
}
