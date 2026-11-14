//! Composite fonts: each class of character set in its part's font.

use crate::*;
use vectorcraft_doc::{CharStyle, CompositeFont, TextRun};

/// Base (Kanji) Source Sans 3; Roman in JetBrains Mono at half size, raised by a tenth of the size;
/// Numerals in Inter, scaled to 80% across; Kana scaled to 50% up from the center.
fn mixed(name: &str) -> CompositeFont {
    let mut f = CompositeFont::new(name, "Source Sans 3", "Regular");
    f.parts.roman.font = "JetBrains Mono".into();
    f.parts.roman.size = 50.0;
    f.parts.roman.baseline = 10.0;
    f.parts.numerals.font = "Inter".into();
    f.parts.numerals.h_scale = 80.0;
    f.parts.kana.v_scale = 50.0;
    f.parts.kana.scale_at_center = true;
    f
}

fn style(family: &str, size: f64) -> CharStyle {
    CharStyle { font_family: family.into(), size, ..CharStyle::default() }
}

fn face_id(db: &FontDb, family: &str) -> u32 {
    db.face(family, "Regular").unwrap().id()
}

/// Each run in a composite font splits where the class changes; a grapheme cluster stays with its
/// first character, runs in other fonts are kept, and each piece remembers its run.
#[test]
fn composite_runs_split_by_class() {
    let db = FontDb::with_font_dirs(vec![]);
    db.set_composite_font(&mixed("Mixed"));
    let mut t = TextObject::point(Point::ZERO, "Ae\u{301}12\u{304B}\u{3099}国", style("Mixed", 20.0));
    t.runs.insert(0, TextRun::new("x", style("Inter", 12.0)));
    let (split, from) = db.composite_split(&t).expect("split");
    let pieces: Vec<(&str, &str)> = split.runs.iter().map(|r| (r.text.as_str(), r.style.font_family.as_str())).collect();
    assert_eq!(
        pieces,
        [("x", "Inter"), ("Ae\u{301}", "JetBrains Mono"), ("12", "Inter"), ("\u{304B}\u{3099}", "Source Sans 3"), ("国", "Source Sans 3")]
    );
    assert_eq!(from, [0, 1, 1, 1, 1]);
    assert_eq!(split.plain_text(), t.plain_text());
    let roman = &split.runs[1].style;
    assert_eq!((roman.size, roman.baseline_shift, roman.h_scale, roman.v_scale), (10.0, 2.0, 100.0, 100.0));
    let numerals = &split.runs[2].style;
    assert_eq!((numerals.size, numerals.h_scale), (20.0, 80.0));
    // Kana at half height, raised by half the em box's center so the center stays put.
    let kana = &split.runs[3].style;
    let centre = db.face("Source Sans 3", "Regular").unwrap().ideographic_centre();
    assert_eq!(kana.v_scale, 50.0);
    assert!((kana.baseline_shift - 20.0 * centre * 0.5).abs() < 1e-9, "{}", kana.baseline_shift);
    assert_eq!(split.runs[4].style.baseline_shift, 0.0);
    // Text in no composite font is not split.
    assert!(db.composite_split(&TextObject::point(Point::ZERO, "Ab", style("Inter", 12.0))).is_none());
}

/// Laid out, each character is drawn in its class's font, sized by its part, and the glyphs still
/// name the run of the text they belong to.
#[test]
fn composite_type_draws_each_class_in_its_font() {
    let db = FontDb::with_font_dirs(vec![]);
    db.set_composite_font(&mixed("Mixed"));
    let mut t = TextObject::point(Point::ZERO, "x", style("Inter", 12.0));
    t.runs.push(TextRun::new("A1", style("Mixed", 20.0)));
    let l = layout(&db, &t);
    let fonts: Vec<(usize, u32)> = l.glyphs.iter().map(|g| (g.run, g.font_id)).collect();
    assert_eq!(fonts, [(0, face_id(&db, "Inter")), (1, face_id(&db, "JetBrains Mono")), (1, face_id(&db, "Inter"))]);
    // `A` is set as JetBrains Mono at 10 pt would set it.
    let alone = layout(&db, &TextObject::point(Point::ZERO, "A", style("JetBrains Mono", 10.0)));
    assert!((l.glyphs[1].advance - alone.glyphs[0].advance).abs() < 1e-9);
}

/// A file font and a composite font can share one text object without losing either face or the
/// original run indices when the composite run splits.
#[test]
fn file_fonts_and_composite_fonts_draw_together() {
    use vectorcraft_doc::{FileFont, FileFonts};

    let db = FontDb::with_font_dirs(vec![]);
    db.set_composite_font(&mixed("Mixed"));
    let info = FileFontInfo { family: "Carried Test".into(), style: "Regular".into(), ..Default::default() };
    let glyph = FileGlyph { ch: 'x', outline: Rect::new(0.0, 0.0, 500.0, 700.0).to_path(0.1), advance: 600.0 };
    let blob = vectorcraft_doc::ImageBlob::new(vectorcraft_doc::FILE_FONT_MIME, build_file_font(&info, &[glyph]).unwrap());
    let mut t = TextObject::point(Point::ZERO, "x", style("Carried Test", 12.0));
    t.runs.push(TextRun::new("A1", style("Mixed", 20.0)));
    t.runs.push(TextRun::new("x", style("Carried Test", 12.0)));
    t.file_fonts = Some(Box::new(FileFonts::new(
        vec![FileFont { family: "Carried Test".into(), style: "Regular".into(), key: "carried-test".into() }],
        &t.plain_text(),
    )));
    assert!(db.bind_file_fonts(&mut t, |key| (key == "carried-test").then_some(&blob)));
    let carried = db.file_faces(&t)[0].as_ref().unwrap().id();
    let l = layout(&db, &t);
    let fonts: Vec<(usize, u32)> = l.glyphs.iter().map(|g| (g.run, g.font_id)).collect();
    assert_eq!(fonts, [(0, carried), (1, face_id(&db, "JetBrains Mono")), (1, face_id(&db, "Inter")), (2, carried)]);
    assert_eq!(t.runs[0].style.font_family, "Carried Test", "layout does not rename the source runs");
    // Editing the text drops the file faces, but not the composite font's class-specific faces.
    t.runs[0].text.push('x');
    let edited = layout(&db, &t);
    assert!(edited.glyphs.iter().all(|g| !db.is_file_font(g.font_id)));
    let composite: Vec<u32> = edited.glyphs.iter().filter(|g| g.run == 1).map(|g| g.font_id).collect();
    assert_eq!(composite, [face_id(&db, "JetBrains Mono"), face_id(&db, "Inter")]);
}

/// A ruby reading over type in a composite font is set in the parts its own characters fall in.
#[test]
fn ruby_over_composite_type_is_set_in_its_own_parts() {
    let db = FontDb::with_font_dirs(vec![]);
    db.set_composite_font(&mixed("Ruby Parts"));
    let mut t = TextObject::point(Point::ZERO, "\u{56FD}", style("Ruby Parts", 20.0));
    t.ruby = vec![vectorcraft_doc::text::RubySpan::new(0, 3, "A")];
    let l = layout(&db, &t);
    let fonts: Vec<u32> = l.ruby.iter().map(|g| g.font_id).collect();
    assert_eq!(fonts, [face_id(&db, "JetBrains Mono")]);
    assert_eq!(l.ruby[0].run, 0);
    // Over a Roman base (a 50% part), the reading is half the 20 pt type, scaled once by its part:
    // 5 pt, not the base's 10 pt halved twice.
    let mut t = TextObject::point(Point::ZERO, "A", style("Ruby Parts", 20.0));
    t.ruby = vec![vectorcraft_doc::text::RubySpan::new(0, 1, "a")];
    let l = layout(&db, &t);
    let alone = layout(&db, &TextObject::point(Point::ZERO, "a", style("JetBrains Mono", 5.0)));
    assert_eq!(l.ruby.len(), 1);
    // The width each glyph is drawn at, in text space.
    let width = |g: &crate::PositionedGlyph| {
        use kurbo::Shape;
        g.outline.bounding_box().width()
    };
    let (reading, expected) = (width(&l.ruby[0]), width(&alone.glyphs[0]));
    assert!(expected > 0.0 && (reading - expected).abs() < 1e-6, "{reading} vs {expected}");
    // It sits above the base as drawn (10 pt), not above the 20 pt the text sets it in.
    let rise = l.glyphs[0].origin.y - l.ruby[0].origin.y;
    assert!(rise > 8.0 && rise < 15.0, "the reading is {rise} pt above the base's baseline");
}

/// Lent composite fonts are available while a guard lends them, in place of a font of the same name
/// the database holds, which the database keeps. A font named like an installed family is not
/// lent, and of two fonts of one name the first is.
#[test]
fn lent_composite_fonts_last_as_long_as_a_guard() {
    let db = FontDb::with_font_dirs(vec![]);
    db.set_composite_font(&mixed("Held"));
    let mut held = mixed("Held");
    held.parts.roman.size = 75.0;
    let fonts = [mixed("Lent"), held, CompositeFont::new("Inter", "Source Sans 3", "Regular")];
    let first = db.lend_composite_fonts(&fonts);
    assert!(db.composite_font("lent").is_some());
    assert_eq!(db.composite_font("Held").unwrap().parts.roman.size, 75.0);
    assert!(db.composite_font("Inter").is_none());
    assert_eq!(db.composite_font_names(), ["Held"], "the database's own fonts stay");
    let second = db.lend_composite_fonts(&fonts);
    drop(first);
    assert!(db.composite_font("Lent").is_some());
    assert_eq!(db.composite_font("Held").unwrap().parts.roman.size, 75.0);
    drop(second);
    assert!(db.composite_font("Lent").is_none());
    assert_eq!(db.composite_font("Held").unwrap().parts.roman.size, 50.0);
    // A font set in the database while one of that name is lent doesn't change the one in use, and
    // is used once the guard drops.
    let lent = db.lend_composite_fonts(&fonts[..1]);
    let mut replaced = mixed("Lent");
    replaced.parts.roman.size = 60.0;
    db.set_composite_font(&replaced);
    assert_eq!(db.composite_font("Lent").unwrap().parts.roman.size, 50.0);
    drop(lent);
    assert_eq!(db.composite_font("Lent").unwrap().parts.roman.size, 60.0);
    let mut other = mixed("LENT");
    other.parts.roman.size = 70.0;
    let lent = db.lend_composite_fonts(&[mixed("Lent"), other]);
    assert_eq!(db.composite_font("lent").unwrap().parts.roman.size, 50.0);
    drop(lent);
}

/// A composite font `name` whose Roman part is set at `size` percent.
fn roman_at(name: &str, size: f64) -> CompositeFont {
    let mut f = mixed(name);
    f.parts.roman.size = size;
    f
}

/// The advance of `A` set at `size` points in `family`.
fn advance(db: &FontDb, family: &str, size: f64) -> f64 {
    layout(db, &TextObject::point(Point::ZERO, "A", style(family, size))).glyphs[0].advance
}

/// Two documents that define one name differently each lay out in their own definition, also when
/// one's guard lends inside the other's (a document placed in another): the inner definition is
/// used until its guard drops, then the outer one again, also after a panic inside the inner one.
/// The generation tells the layouts apart, and lending changes nothing the database holds.
#[test]
fn nested_loans_of_one_name_use_each_documents_definition() {
    let db = FontDb::with_font_dirs(vec![]);
    let (a, b) = (roman_at("CF Shared", 50.0), roman_at("CF Shared", 75.0));
    db.set_composite_font(&a);
    let (half, three_quarters) = (advance(&db, "JetBrains Mono", 10.0), advance(&db, "JetBrains Mono", 15.0));
    // Lending looks the name up among the installed families, which catalogs them the first time.
    assert!(!db.has_installed_family("CF Shared"));
    let plain = db.generation();
    let outer = db.lend_composite_fonts(std::slice::from_ref(&a));
    assert_eq!(db.generation(), plain, "the database's own definition");
    assert!((advance(&db, "CF Shared", 20.0) - half).abs() < 1e-9);
    {
        let _inner = db.lend_composite_fonts(std::slice::from_ref(&b));
        assert_eq!(db.composite_font("cf shared").unwrap().parts.roman.size, 75.0);
        assert!((advance(&db, "CF Shared", 20.0) - three_quarters).abs() < 1e-9);
        assert_ne!(db.generation(), plain, "another definition lays out under another generation");
        let elsewhere =
            std::thread::scope(|s| s.spawn(|| (db.generation(), db.composite_font("CF Shared").unwrap().parts.roman.size)).join().unwrap());
        assert_eq!(elsewhere, (plain, 50.0), "other threads keep the database's");
    }
    assert!((advance(&db, "CF Shared", 20.0) - half).abs() < 1e-9);
    assert_eq!(db.generation(), plain);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _inner = db.lend_composite_fonts(std::slice::from_ref(&b));
        panic!("a render that fails");
    }));
    assert!(caught.is_err());
    assert_eq!(db.composite_font("CF Shared").unwrap().parts.roman.size, 50.0);
    drop(outer);
    let next = db.lend_composite_fonts(std::slice::from_ref(&b));
    assert_eq!(db.composite_font("CF Shared").unwrap().parts.roman.size, 75.0);
    drop(next);
    assert_eq!(db.composite_font("CF Shared").unwrap().parts.roman.size, 50.0);
    assert_eq!(db.generation(), plain);
}

/// Two threads that lend different definitions of one name at once each lay out in their own.
#[test]
fn threads_lending_one_name_at_once_use_their_own_definitions() {
    let db = FontDb::with_font_dirs(vec![]);
    let (half, three_quarters) = (advance(&db, "JetBrains Mono", 10.0), advance(&db, "JetBrains Mono", 15.0));
    let (both_lent, both_read) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
    let lend = |size: f64| {
        let _lent = db.lend_composite_fonts(&[roman_at("CF Threads", size)]);
        both_lent.wait();
        let read = (advance(&db, "CF Threads", 20.0), db.generation());
        both_read.wait();
        read
    };
    let ((a, ga), (b, gb)) = std::thread::scope(|s| {
        let a = s.spawn(|| lend(50.0));
        let b = s.spawn(|| lend(75.0));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert!((a - half).abs() < 1e-9, "{a} vs {half}");
    assert!((b - three_quarters).abs() < 1e-9, "{b} vs {three_quarters}");
    assert_ne!(ga, gb);
    assert!(db.composite_font("CF Threads").is_none());
}

/// The fonts a composite font sets text in are those of the parts its characters fall in, and
/// coverage is checked against each character's part.
#[test]
fn composite_fonts_for_text_follow_each_characters_part() {
    let db = FontDb::with_font_dirs(vec![]);
    let mut f = mixed("Parts");
    f.parts.kana.font = "No Such Kana Font".into();
    db.set_composite_font(&f);
    let fonts = |text: &str| db.composite_fonts_for("parts", text).unwrap();
    let pair = |family: &str| (family.to_string(), "Regular".to_string());
    assert_eq!(fonts("A1"), [pair("JetBrains Mono"), pair("Inter")]);
    assert_eq!(fonts(""), [pair("Source Sans 3")]);
    assert!(db.composite_fonts_for("Inter", "A").is_none());
    let exact = db.composite_coverage("Parts", &["A1"]).unwrap();
    assert_eq!((exact.status, exact.lacking.len()), (FontMatch::Exact, 0));
    let missing = db.composite_coverage("Parts", &["A\u{304B}"]).unwrap();
    assert_eq!(missing.status, FontMatch::Missing);
    assert_eq!(missing.face.map(|f| f.family.clone()).as_deref(), Some(FALLBACK_FAMILY));
    // Each run is split on its own: a combining mark that starts a run is a Roman character.
    f.parts.kana.font = "Source Sans 3".into();
    f.parts.roman.font = "No Such Roman Font".into();
    db.set_composite_font(&f);
    assert_eq!(db.composite_coverage("Parts", &["\u{304B}\u{301}"]).unwrap().status, FontMatch::Exact);
    assert_eq!(db.composite_coverage("Parts", &["\u{304B}", "\u{301}"]).unwrap().status, FontMatch::Missing);
    assert_eq!(db.stored_family("parts", "Regular").as_deref(), Some("Parts"));
    assert_eq!(db.stored_family("No Such Kana Font", "Regular"), None);
}

/// A composite font resolves to its base font as an exact match, lists first in the font menus,
/// and is gone from them once removed.
#[test]
fn composite_fonts_resolve_to_their_base_and_list_first() {
    let db = FontDb::with_font_dirs(vec![]);
    let generation = db.generation();
    db.set_composite_font(&mixed("Zeta Mixed"));
    assert_ne!(db.generation(), generation);
    let (face, how) = db.resolve("zeta mixed", "Regular").unwrap();
    assert_eq!((face.family.as_str(), how), ("Source Sans 3", FontMatch::Exact));
    assert!(db.has_family("Zeta Mixed"));
    assert_eq!(db.menu_family_list().first().map(String::as_str), Some("Zeta Mixed"));
    // Registering the same definition again changes nothing.
    let generation = db.generation();
    db.set_composite_font(&mixed("Zeta Mixed"));
    assert_eq!(db.generation(), generation);
    // A base naming a composite font falls back rather than looping.
    let mut odd = mixed("Odd");
    odd.parts.kanji.font = "Zeta Mixed".into();
    db.set_composite_font(&odd);
    assert_eq!(db.face("Odd", "Regular").unwrap().family, FALLBACK_FAMILY);
    assert!(db.remove_composite_font("ZETA MIXED"));
    assert!(!db.remove_composite_font("Zeta Mixed"));
    assert!(!db.menu_family_list().iter().any(|f| f == "Zeta Mixed"));
    assert!(!db.has_family("Zeta Mixed"));
}
