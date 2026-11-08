use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::Session;

/// A composite font named `name`: Source Sans 3 for the base, JetBrains Mono at half size for
/// Roman, Inter for numerals.
fn font(name: &str) -> Value {
    json!({"name": name, "parts": {
        "kanji": {"font": "Source Sans 3"},
        "roman": {"font": "JetBrains Mono", "size": 50, "baseline": 10},
        "numerals": {"font": "Inter", "hScale": 80},
    }})
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.composite_fonts.iter().map(|f| f.name.clone()).collect()
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn text(s: &Session, id: u64) -> vectorcraft_doc::TextObject {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

/// A scratch folder of the test's own.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-composite-fonts-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Composite Fonts' OK replaces the document's composite fonts as one undo step (none when nothing
/// changes); parts left out copy the Kanji part; they are saved with the document and undone with
/// it.
#[test]
fn composite_fonts_are_set_as_one_undo_step_and_saved() {
    let mut s = session();
    let before = steps(&s);
    let r = s.execute("type.compositeFonts.set", &json!({"fonts": [font(" CF Saved A "), font("CF Saved B")]})).unwrap();
    assert_eq!(r["fonts"][0]["name"], "CF Saved A");
    assert_eq!(names(&s), ["CF Saved A", "CF Saved B"]);
    assert_eq!(steps(&s), before + 1);
    let f = &s.doc().unwrap().doc.composite_fonts[0];
    assert_eq!((f.parts.kana.font.as_str(), f.parts.roman.size, f.parts.numerals.h_scale), ("Source Sans 3", 50.0, 80.0));
    // The same fonts again change nothing.
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Saved A"), font("CF Saved B")]})).unwrap();
    assert_eq!(steps(&s), before + 1);
    let listed = s.execute("type.compositeFonts.list", &json!({})).unwrap();
    assert_eq!(listed["fonts"].as_array().map(Vec::len), Some(2));
    // They travel with the file.
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(&s.doc().unwrap().doc)).unwrap();
    assert_eq!(*back.composite_fonts, *s.doc().unwrap().doc.composite_fonts);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(names(&s).is_empty());
}

/// Bad fonts are refused with an error and no undo step: a name that is empty, shared (in any
/// case) or an installed font's; a part set in a composite font; an unknown class or key; a value
/// out of range, of the wrong type, or on a class that can't take it; no Kanji part.
#[test]
fn bad_composite_fonts_are_refused() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Refused Base")]})).unwrap();
    let before = steps(&s);
    let with = |f: &dyn Fn(&mut Value)| {
        let mut v = font("CF Refused");
        f(&mut v);
        json!({"fonts": [v]})
    };
    for bad in [
        json!({}),
        json!({"fonts": "CF"}),
        json!({"fonts": [font("  ")]}),
        json!({"fonts": [font("CF Twice"), font("cf twice")]}),
        json!({"fonts": [font("Source Sans 3")]}),
        json!({"fonts": [font("Tab\tName")]}),
        with(&|v| v["parts"]["roman"]["font"] = json!("CF Refused Base")),
        with(&|v| v["parts"]["roman"]["font"] = json!("CF Refused")),
        with(&|v| v["parts"]["hangul"] = json!({"font": "Inter"})),
        with(&|v| v["parts"]["Roman"] = json!({"font": "Inter"})),
        with(&|v| v["parts"]["roman"]["width"] = json!(3)),
        with(&|v| v["parts"]["roman"]["size"] = json!(5)),
        with(&|v| v["parts"]["roman"]["size"] = json!("50")),
        with(&|v| v["parts"]["roman"]["baseline"] = json!(-120)),
        with(&|v| v["parts"]["kanji"]["hScale"] = json!(90)),
        with(&|v| v["parts"]["punctuation"] = json!({"font": "Inter", "vScale": 90})),
        with(&|v| v["parts"]["roman"]["scaleAtCenter"] = json!(true)),
        with(&|v| v["parts"]["kana"] = json!({"font": "Inter", "scaleAtCenter": "yes"})),
        with(&|v| v["parts"]["roman"]["font"] = json!("")),
        with(&|v| v["extra"] = json!(1)),
        with(&|v| {
            v["parts"].as_object_mut().unwrap().remove("kanji");
        }),
    ] {
        assert!(s.execute("type.compositeFonts.set", &bad).is_err(), "accepted {bad}");
        assert_eq!(names(&s), ["CF Refused Base"]);
        assert_eq!(steps(&s), before);
    }
    // Kana scaled at the center is allowed.
    let ok = with(&|v| v["parts"]["kana"] = json!({"font": "Inter", "vScale": 80, "scaleAtCenter": true}));
    s.execute("type.compositeFonts.set", &ok).unwrap();
    assert_eq!(names(&s), ["CF Refused"]);
}

/// Type set in a composite font names it and is not missing; each class is drawn in its part's
/// font; changing the font measures the type again, and taking it out leaves the type naming it,
/// shown as missing.
#[test]
fn type_set_in_a_composite_font_follows_it() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Type")]})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab12", "size": 20})).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.setStyle", &json!({"ids": [id], "font": "CF Type"})).unwrap();
    assert!(r.get("warning").is_none(), "{r}");
    let t = text(&s, id);
    assert_eq!(t.runs[0].style.font_family, "CF Type");
    let db = vectorcraft_text::FontDb::global();
    let face = |family: &str| db.face(family, "Regular").unwrap().id();
    let lay = vectorcraft_text::layout(db, &t);
    let fonts: Vec<u32> = lay.glyphs.iter().map(|g| g.font_id).collect();
    assert_eq!(fonts, [face("JetBrains Mono"), face("JetBrains Mono"), face("Inter"), face("Inter")]);
    let missing = |s: &mut Session| s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().filter(|f| f["missing"] == true).count();
    assert_eq!(missing(&mut s), 0);
    // Roman at full size makes the type wider, measured again in the same step.
    let wide = t.cached_bounds.unwrap().width();
    let mut full = font("CF Type");
    full["parts"]["roman"]["size"] = json!(100);
    let before = steps(&s);
    s.execute("type.compositeFonts.set", &json!({"fonts": [full]})).unwrap();
    assert_eq!(steps(&s), before + 1);
    assert!(text(&s, id).cached_bounds.unwrap().width() > wide + 1.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((text(&s, id).cached_bounds.unwrap().width() - wide).abs() < 1e-6);
    assert_eq!(db.composite_font("CF Type").unwrap().parts.roman.size, 50.0);
    // Taken out, the type keeps the name, now missing; undone, the font is back.
    s.execute("type.compositeFonts.set", &json!({"fonts": []})).unwrap();
    assert!(db.composite_font("CF Type").is_none());
    assert_eq!(text(&s, id).runs[0].style.font_family, "CF Type");
    assert_eq!(missing(&mut s), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(db.composite_font("CF Type").is_some());
    assert_eq!(missing(&mut s), 0);
}

/// Type in a symbol's art is measured again too when the composite font it names changes.
#[test]
fn type_in_symbol_art_follows_its_composite_font() {
    fn width(s: &Session) -> f64 {
        fn find(n: &vectorcraft_doc::Node) -> Option<f64> {
            if let NodeKind::Text(t) = &n.kind {
                return t.cached_bounds.map(|b| b.width());
            }
            n.children().into_iter().flatten().find_map(|c| find(c))
        }
        s.doc().unwrap().doc.symbols.iter().find_map(|sym| find(&sym.art)).unwrap()
    }
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Symbol")]})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab12", "size": 20, "font": "CF Symbol"})).unwrap()["id"].as_u64().unwrap();
    s.execute("symbol.new", &json!({"name": "CF Mark", "ids": [id]})).unwrap();
    let narrow = width(&s);
    let mut full = font("CF Symbol");
    full["parts"]["roman"]["size"] = json!(100);
    s.execute("type.compositeFonts.set", &json!({"fonts": [full]})).unwrap();
    assert!(width(&s) > narrow + 1.0);
}

/// The font database forgets a composite font once no open document defines it: after the step
/// that added it is undone, and after its document closes.
#[test]
fn composite_fonts_no_document_defines_are_forgotten() {
    let db = vectorcraft_text::FontDb::global();
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Forgotten Undo")]})).unwrap();
    assert!(db.composite_font("CF Forgotten Undo").is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(db.composite_font("CF Forgotten Undo").is_none());
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(db.composite_font("CF Forgotten Undo").is_some());
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Forgotten Close")]})).unwrap();
    s.execute("file.close", &json!({})).unwrap();
    assert!(db.composite_font("CF Forgotten Close").is_none());
    assert!(db.composite_font("CF Forgotten Undo").is_some());
}

/// SVG and legacy `.ai` write type in a composite font as its outlines, since a reader can't find
/// the font by its name.
#[test]
fn exports_outline_type_in_a_composite_font() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Export")]})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab1", "size": 20, "font": "CF Export"})).unwrap();
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(!svg.contains("CF Export") && !svg.contains("<text") && svg.contains("<path"), "{svg}");
    let ai = vectorcraft_eps::export_legacy_ai(&s.doc().unwrap().doc, &vectorcraft_eps::LegacyAiOptions::default()).unwrap();
    assert!(!String::from_utf8_lossy(&ai.bytes).contains("CF Export"));
}

/// A document exported without a session (whose composite fonts the font database doesn't hold),
/// or while another document defines the name differently, still has its type in a composite font
/// written as outlines, drawn in its own definition's parts: SVG
/// writes the outlines the same type set in the part's font writes, PDF embeds the part's font, and
/// legacy `.ai` doesn't name the composite font.
#[test]
fn exports_draw_composite_fonts_of_a_document_without_a_session() {
    let mut s = session();
    let mono = json!({"name": "CF Standalone", "parts": {"kanji": {"font": "JetBrains Mono"}}});
    s.execute("type.compositeFonts.set", &json!({"fonts": [mono]})).unwrap();
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab1", "size": 20, "font": "CF Standalone"})).unwrap()["id"].as_u64().unwrap();
    let doc = (*s.doc().unwrap().doc).clone();
    // The font database forgets the font; the copy of the document still defines it.
    s.execute("type.compositeFonts.set", &json!({"fonts": []})).unwrap();
    let db = vectorcraft_text::FontDb::global();
    assert!(db.composite_font("CF Standalone").is_none());
    let paths = |svg: &str| svg.split(" d=\"").skip(1).filter_map(|p| p.split('"').next().map(str::to_string)).collect::<Vec<_>>();
    let svg = vectorcraft_svg::export(&doc, &vectorcraft_svg::ExportOptions::default());
    assert!(!svg.contains("CF Standalone") && !svg.contains("<text"), "{svg}");
    s.execute("text.setStyle", &json!({"ids": [id], "font": "JetBrains Mono"})).unwrap();
    let plain = vectorcraft_svg::export(&s.doc().unwrap().doc, &vectorcraft_svg::ExportOptions { outline_text: true, ..Default::default() });
    assert!(!paths(&svg).is_empty());
    assert_eq!(paths(&svg), paths(&plain));
    let pdf = vectorcraft_pdf::export(&doc, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("JetBrainsMono"));
    let ai = vectorcraft_eps::export_legacy_ai(&doc, &vectorcraft_eps::LegacyAiOptions::default()).unwrap();
    assert!(!String::from_utf8_lossy(&ai.bytes).contains("CF Standalone"));
    // The fonts lent for the exports are forgotten again.
    assert!(db.composite_font("CF Standalone").is_none());
    // With the front document defining the name differently, the exported document's own
    // definition draws its type, and the front document's is put back.
    let inter = json!({"name": "CF Standalone", "parts": {"kanji": {"font": "Inter"}}});
    s.execute("type.compositeFonts.set", &json!({"fonts": [inter]})).unwrap();
    let svg = vectorcraft_svg::export(&doc, &vectorcraft_svg::ExportOptions::default());
    assert_eq!(paths(&svg), paths(&plain));
    assert_eq!(db.composite_font("CF Standalone").unwrap().parts.kanji.font, "Inter");
}

/// Replace Font and Replace Missing Fonts set type in a composite font by the font's own name,
/// not its Kanji part's, and keep the type's style.
#[test]
fn replacing_a_font_with_a_composite_font_names_it() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Replace")]})).unwrap();
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab12", "size": 20, "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "cf replace"}})).unwrap();
    assert_eq!(r["runs"], 1, "{r}");
    assert!(r.get("warning").is_none(), "{r}");
    let t = text(&s, id);
    assert_eq!((t.runs[0].style.font_family.as_str(), t.runs[0].style.font_style.as_str()), ("CF Replace", "Regular"));
    let gone = s
        .execute("text.create", &json!({"x": 10, "y": 90, "text": "Lost", "size": 20, "font": "No Such Font Family", "style": "Bold"}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let r = s.execute("addon.cleanup.replaceFonts", &json!({"font": "CF Replace"})).unwrap();
    assert_eq!((r["runs"].as_u64(), r["font"].as_str()), (Some(1), Some("CF Replace")), "{r}");
    let t = text(&s, gone);
    assert_eq!((t.runs[0].style.font_family.as_str(), t.runs[0].style.font_style.as_str()), ("CF Replace", "Bold"));
}

/// When a part's font is missing, though the Kanji part's is installed, text.missingFonts names
/// that part's font for type with characters in that part, text.fonts shows the composite font as
/// missing (resolved in the fallback font that draws that part), and setting type in it warns;
/// missingGlyphs counts the characters that the font of their part lacks. Each run is checked on
/// its own, as layout splits it.
#[test]
fn font_inventory_checks_the_part_of_each_character() {
    let mut s = session();
    let mut f = font("CF Inventory");
    f["parts"]["kanji"]["font"] = json!("Source Serif 4");
    f["parts"]["roman"]["font"] = json!("No Such Part Font");
    s.execute("type.compositeFonts.set", &json!({"fonts": [f]})).unwrap();
    // Numerals are drawn in Inter: nothing is missing.
    let r = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "12", "size": 20, "font": "CF Inventory"})).unwrap();
    assert!(r.get("warning").is_none(), "{r}");
    assert_eq!(s.execute("text.missingFonts", &json!({})).unwrap()["count"], 0);
    // Roman letters are drawn in the missing font.
    let r = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Ab", "size": 20, "font": "CF Inventory"})).unwrap();
    assert!(r["warning"].as_str().is_some_and(|w| w.contains("No Such Part Font")), "{r}");
    let missing = s.execute("text.missingFonts", &json!({})).unwrap();
    let names: Vec<&str> = missing["fonts"].as_array().unwrap().iter().filter_map(|f| f["family"].as_str()).collect();
    assert_eq!(names, ["No Such Part Font"], "{missing}");
    let fonts = s.execute("text.fonts", &json!({})).unwrap();
    let row = fonts.as_array().unwrap().iter().find(|r| r["family"] == "CF Inventory").cloned().unwrap();
    assert_eq!((&row["missing"], &row["status"]), (&json!(true), &json!("missing")), "{row}");
    assert_eq!(row["resolved"]["family"], vectorcraft_text::FALLBACK_FAMILY, "{row}");
    // A run that starts with a combining mark after a run ending in kana: the mark is Roman.
    let boundary = json!({"name": "CF Boundary", "parts": {"kanji": {"font": "Source Sans 3"}, "roman": {"font": "No Such Part Font"}}});
    s.execute("type.compositeFonts.set", &json!({"fonts": [boundary]})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 170, "text": "\u{304B}\u{301}", "size": 20, "font": "CF Boundary"})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let row =
        |s: &mut Session| s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().find(|r| r["family"] == "CF Boundary").cloned();
    assert_eq!(row(&mut s).unwrap()["status"], "exact");
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 5, "size": 30})).unwrap();
    assert_eq!(text(&s, id).runs.len(), 2);
    assert_eq!(row(&mut s).unwrap()["status"], "missing");
    // A symbol the Kanji part's font draws but the Symbols part's font lacks.
    let db = vectorcraft_text::FontDb::global();
    let (mono, sans) = (db.face("JetBrains Mono", "Regular").unwrap(), db.face("Source Sans 3", "Regular").unwrap());
    let c = ('\u{2190}'..='\u{2BFF}').find(|c| mono.covers(*c) && !sans.covers(*c)).expect("a symbol only JetBrains Mono draws");
    let glyphs = json!({"name": "CF Glyphs", "parts": {"kanji": {"font": "JetBrains Mono"}, "symbols": {"font": "Source Sans 3"}}});
    s.execute("type.compositeFonts.set", &json!({"fonts": [glyphs]})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 130, "text": c.to_string(), "size": 20, "font": "CF Glyphs"})).unwrap();
    let fonts = s.execute("text.fonts", &json!({})).unwrap();
    let row = fonts.as_array().unwrap().iter().find(|r| r["family"] == "CF Glyphs").cloned().unwrap();
    assert_eq!(row["missingGlyphs"], 1, "{row}");
}

/// The Glyphs panel's alternates of a character in a composite font come from the font of the
/// part its grapheme cluster falls in.
#[test]
fn glyph_alternates_of_composite_type_come_from_the_characters_part() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Glyph Part")]})).unwrap();
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab12", "size": 20, "font": "CF Glyph Part"})).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.glyphAlternates", &json!({"id": id, "start": 2})).unwrap();
    assert_eq!(r["font"], "Inter", "{r}");
    let r = s.execute("text.glyphAlternates", &json!({"id": id, "start": 0})).unwrap();
    assert_eq!(r["font"], "JetBrains Mono", "{r}");
    // A combining mark is drawn in the part of the character it follows.
    let id = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "1\u{301}", "size": 20, "font": "CF Glyph Part"})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let r = s.execute("text.glyphAlternates", &json!({"id": id, "start": 1})).unwrap();
    assert_eq!(r["font"], "Inter", "{r}");
}

/// With two documents defining one name differently, the type is drawn in the front document's.
#[test]
fn the_front_document_defines_a_shared_name() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Front")]})).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let mut other = font("CF Front");
    other["parts"]["roman"]["size"] = json!(75);
    s.execute("type.compositeFonts.set", &json!({"fonts": [other]})).unwrap();
    let db = vectorcraft_text::FontDb::global();
    assert_eq!(db.composite_font("CF Front").unwrap().parts.roman.size, 75.0);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(db.composite_font("CF Front").unwrap().parts.roman.size, 50.0);
}

/// A composite font only a document in the back defines is not available in the front one, where
/// type naming it shows as missing; a click on the back document's tab brings it back.
#[test]
fn a_font_only_a_back_document_defines_is_missing_in_front() {
    let db = vectorcraft_text::FontDb::global();
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Back Only")]})).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert!(db.composite_font("CF Back Only").is_none());
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab", "size": 20, "font": "CF Back Only"})).unwrap();
    let fonts = s.execute("text.fonts", &json!({})).unwrap();
    assert!(fonts.as_array().unwrap().iter().any(|f| f["missing"] == true), "{fonts}");
    assert!(s.set_active(0));
    assert!(db.composite_font("CF Back Only").is_some());
}

/// A newly opened document measures a missing font without using the previous tab's composite
/// font of the same name.
#[test]
fn opened_document_measures_missing_composite_font_as_missing() {
    let mut s = session();
    s.execute("type.compositeFonts.set", &json!({"fonts": [font("CF Previous Tab")]})).unwrap();
    let id =
        s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab", "size": 20, "font": "CF Previous Tab"})).unwrap()["id"].as_u64().unwrap();
    let mut doc = (*s.doc().unwrap().doc).clone();
    doc.composite_fonts.clear();
    if let NodeKind::Text(t) = &mut doc.node_mut(NodeId(id)).unwrap().kind {
        t.cached_bounds = None;
    }
    s.add_document(doc, None);
    assert!(s.set_active(0));
    assert!(s.set_active(1));
    let actual = text(&s, id);
    let expected = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &actual).bounds;
    assert_eq!(actual.cached_bounds, Some(expected));
}

/// A loaded composite font cannot replace an installed family used by existing type.
#[test]
fn loaded_composite_font_cannot_take_an_installed_family_name() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ab", "size": 20, "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    let mut doc = (*s.doc().unwrap().doc).clone();
    doc.composite_fonts.push(vectorcraft_doc::CompositeFont::new("Inter", "Source Sans 3", "Regular"));
    if let NodeKind::Text(t) = &mut doc.node_mut(NodeId(id)).unwrap().kind {
        t.cached_bounds = None;
    }
    s.add_document(doc, None);
    assert!(names(&s).is_empty());
    assert!(vectorcraft_text::FontDb::global().composite_font("Inter").is_none());
    let actual = text(&s, id);
    let expected = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &actual).bounds;
    assert_eq!(actual.cached_bounds, Some(expected));
}

/// A file's composite fonts that Composite Fonts… would refuse (a value out of range, a name used
/// twice) are left out when the file opens.
#[test]
fn unusable_composite_fonts_in_a_file_are_left_out() {
    let db = vectorcraft_text::FontDb::global();
    let mut s = session();
    let mut doc = (*s.doc().unwrap().doc).clone();
    let mut wild = vectorcraft_doc::CompositeFont::new("CF Wild", "Source Sans 3", "Regular");
    wild.parts.roman.size = 5000.0;
    let good = vectorcraft_doc::CompositeFont::new("CF Kept", "Source Sans 3", "Regular");
    let twice = vectorcraft_doc::CompositeFont::new("cf kept", "Inter", "Regular");
    *doc.composite_fonts = vec![wild, good, twice];
    s.add_document(doc, None);
    assert_eq!(names(&s), ["CF Kept"]);
    s.execute("document.activate", &json!({"index": 1})).unwrap();
    assert!(db.composite_font("CF Wild").is_none());
    assert_eq!(db.composite_font("CF Kept").unwrap().parts.roman.font, "Source Sans 3");
}

/// Export writes the document's composite fonts (all, or those named) to a file; Import reads it:
/// with apply false it only returns them, otherwise they join the document's, replacing a font of
/// the same name, as one undo step.
#[test]
fn composite_fonts_export_and_import() {
    let dir = folder("io");
    let path = dir.join("Mixed fonts (日本).vccomposite");
    let path = path.to_str().unwrap();
    let mut s = session();
    assert!(s.execute("type.compositeFonts.export", &json!({"path": path})).is_err(), "nothing to export");
    let mut slash = font("CF IO a/b: \"quoted\" 日本");
    slash["parts"]["kana"] = json!({"font": "Inter", "vScale": 80, "scaleAtCenter": true});
    s.execute("type.compositeFonts.set", &json!({"fonts": [slash, font("CF IO Other")]})).unwrap();
    assert!(s.execute("type.compositeFonts.export", &json!({"path": path, "names": ["CF IO Nope"]})).is_err());
    let r = s
        .execute("type.compositeFonts.export", &json!({"path": path, "names": ["cf io a/b: \"quoted\" 日本", "CF IO a/b: \"quoted\" 日本"]}))
        .unwrap();
    assert_eq!(r["count"], 1);
    let saved = s.doc().unwrap().doc.composite_fonts[0].clone();

    let mut t = session();
    let mut mine = font("CF IO a/b: \"quoted\" 日本");
    mine["parts"]["roman"]["size"] = json!(90);
    t.execute("type.compositeFonts.set", &json!({"fonts": [mine, font("CF IO Kept")]})).unwrap();
    let before = steps(&t);
    let read = t.execute("type.compositeFonts.import", &json!({"path": path, "apply": false})).unwrap();
    assert_eq!(read["fonts"].as_array().map(Vec::len), Some(1));
    assert_eq!(steps(&t), before);
    t.execute("type.compositeFonts.import", &json!({"path": path})).unwrap();
    assert_eq!(steps(&t), before + 1);
    assert_eq!(names(&t), ["CF IO a/b: \"quoted\" 日本", "CF IO Kept"]);
    assert_eq!(t.doc().unwrap().doc.composite_fonts[0], saved);
    // The same file again changes nothing.
    t.execute("type.compositeFonts.import", &json!({"path": path})).unwrap();
    assert_eq!(steps(&t), before + 1);

    // Files that aren't composite fonts, or are too large, are refused.
    for (file, bytes) in [
        ("bad.vccomposite", b"not json".to_vec()),
        ("empty.vccomposite", br#"{"compositeFonts": []}"#.to_vec()),
        ("twice.vccomposite", format!(r#"{{"compositeFonts": [{}, {}]}}"#, font("CF Dup"), font("cf dup")).into_bytes()),
        ("large.vccomposite", vec![b' '; (4 << 20) + 1]),
    ] {
        let p = dir.join(file);
        std::fs::write(&p, bytes).unwrap();
        assert!(t.execute("type.compositeFonts.import", &json!({"path": p.to_str().unwrap()})).is_err(), "{file}");
        assert_eq!(steps(&t), before + 1);
    }
    assert!(t.execute("type.compositeFonts.import", &json!({"path": path, "apply": "no"})).is_err());
    assert!(t.execute("type.compositeFonts.import", &json!({"path": ""})).is_err());
    assert!(t.execute("type.compositeFonts.import", &json!({"path": path, "data": "{}"})).is_err());
    assert!(t.execute("type.compositeFonts.import", &json!({"dataBase64": "%%%"})).is_err());

    // Without a path, Export gives the file's text, which Import takes as data.
    let data = s.execute("type.compositeFonts.export", &json!({"names": ["CF IO Other"]})).unwrap();
    assert_eq!(data["count"], 1);
    let mut u = session();
    let r = u.execute("type.compositeFonts.import", &json!({"data": data["data"]})).unwrap();
    assert_eq!(r["imported"], json!(["CF IO Other"]));
    assert_eq!(names(&u), ["CF IO Other"]);
    // Fonts given are written in place of the document's; bad ones are refused.
    let given = s.execute("type.compositeFonts.export", &json!({"fonts": [font("CF IO Given")]})).unwrap();
    assert!(given["data"].as_str().unwrap().contains("CF IO Given"));
    assert!(s.execute("type.compositeFonts.export", &json!({"fonts": [font("")]})).is_err());
    assert!(s.execute("type.compositeFonts.export", &json!({"fonts": []})).is_err());
    assert!(s.execute("type.compositeFonts.export", &json!({"path": " "})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
