use std::sync::OnceLock;

use serde_json::{Value, json};
use vectorcraft_text::FontDb;

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

/// The bundled fonts alone: other tests add copies of them to the global database, and a copy
/// ties with its original, so the ranking is read against a database of its own.
fn bundled() -> &'static FontDb {
    static DB: OnceLock<FontDb> = OnceLock::new();
    DB.get_or_init(|| FontDb::with_font_dirs(vec![]))
}

/// `type.retypeFonts` with `params`, comparing the fonts of `db`.
fn query(s: &mut Session, params: Value, db: &FontDb) -> Value {
    super::retype_fonts_in(s, &params, db).unwrap()
}

/// Point text, outlined: the id of the outlines' group.
fn outlined(s: &mut Session, p: Value) -> u64 {
    let id = s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap()
}

fn names(r: &Value) -> Vec<(String, String, u64)> {
    r["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["family"].as_str().unwrap().to_string(), f["style"].as_str().unwrap().to_string(), f["matched"].as_u64().unwrap()))
        .collect()
}

#[test]
fn matching_fonts_rank_by_how_many_shapes_each_matches() {
    let mut s = session();
    // Seven shapes in Source Sans 3 Bold, three in Source Serif 4.
    let a = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Ranking", "size": 30, "font": "Source Sans 3", "style": "Bold"}));
    let b = outlined(&mut s, json!({"x": 40, "y": 200, "text": "Wxy", "size": 30, "font": "Source Serif 4", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let r = query(&mut s, json!({}), bundled());
    assert_eq!(r["shapes"], json!(10), "{r}");
    let fonts = names(&r);
    assert_eq!(fonts.first(), Some(&("Source Sans 3".to_string(), "Bold".to_string(), 7)), "{r}");
    let serif = fonts.iter().position(|f| f.0 == "Source Serif 4" && f.1 == "Regular").expect("the serif is listed");
    assert_eq!(fonts[serif].2, 3, "{r}");
    // Each font once, and no font matches more shapes than one listed before it.
    for (i, f) in fonts.iter().enumerate() {
        assert!(!fonts[..i].iter().any(|g| g.0 == f.0 && g.1 == f.1), "{f:?} is listed twice: {r}");
        assert!(fonts[..i].iter().all(|g| g.2 >= f.2), "{f:?} ranks after a font that matches fewer shapes: {r}");
    }
    assert_eq!(r["truncated"], json!(false));
    // The command: a query, so the document stays as it was, and there is no undo step for it.
    let r = s.execute("type.retypeFonts", &json!({})).unwrap();
    assert_eq!(r["shapes"], json!(10), "{r}");
    assert!(s.doc().unwrap().doc == before, "nothing changes");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc != before, "undo takes back the last outlining, not the query");
}

#[test]
fn count_lists_the_first_fonts_and_more_says_others_match() {
    let mut s = session();
    let a = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Aa", "size": 30, "font": "Source Sans 3", "style": "Regular"}));
    let b = outlined(&mut s, json!({"x": 40, "y": 200, "text": "B", "size": 30, "font": "Source Serif 4", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let all = query(&mut s, json!({"count": 96}), bundled());
    assert!(names(&all).len() >= 2, "{all}");
    assert_eq!(all["more"], json!(false), "{all}");
    let one = query(&mut s, json!({"count": 1}), bundled());
    assert_eq!(names(&one), vec![("Source Sans 3".to_string(), "Regular".to_string(), 2)], "{one}");
    assert_eq!(one["more"], json!(true), "{one}");
    assert_eq!(query(&mut s, json!({"count": null}), bundled()), query(&mut s, json!({}), bundled()));
}

#[test]
fn a_count_that_is_not_a_whole_number_from_1_to_96_is_an_error() {
    let mut s = session();
    let a = outlined(&mut s, json!({"x": 40, "y": 100, "text": "A", "size": 30}));
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    for bad in [json!(0), json!(97), json!(-1), json!(2.5), json!("10"), json!(true)] {
        let e = s.execute("type.retypeFonts", &json!({"count": bad})).unwrap_err().to_string();
        assert!(e.contains("`count` must be a whole number from 1 to 96"), "{bad}: {e}");
    }
}

#[test]
fn shapes_that_are_no_glyph_list_no_font() {
    let mut s = session();
    let rect = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [rect]})).unwrap();
    let r = query(&mut s, json!({}), bundled());
    assert_eq!(r, json!({"fonts": [], "shapes": 1, "more": false, "truncated": false, "installed": {"read": 0, "skipped": 0}}));
    // Live type alone holds no shape to compare.
    let t = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "live"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    assert!(s.execute("type.retypeFonts", &json!({})).is_err());
}

#[test]
fn a_font_that_matches_only_combining_marks_is_not_listed() {
    let mut s = session();
    // A lone combining mark (a drawn dot, bar or ring is the same shape): type.retype keeps it.
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "\u{312}", "size": 40, "font": "Source Sans 3", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = query(&mut s, json!({}), bundled());
    assert_eq!(r, json!({"fonts": [], "shapes": 1, "more": false, "truncated": false, "installed": {"read": 0, "skipped": 0}}));
    // After a base, the mark counts with it.
    let g = outlined(&mut s, json!({"x": 40, "y": 200, "text": "x\u{312}", "size": 40, "font": "Source Sans 3", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = query(&mut s, json!({}), bundled());
    assert!(names(&r).contains(&("Source Sans 3".into(), "Regular".into(), 2)), "{r}");
}

#[test]
fn shapes_past_the_limits_are_reported_as_left_out() {
    let mut s = session();
    let top = outlined(&mut s, json!({"x": 40, "y": 100, "text": "A", "size": 40}));
    // B, nested deeper than Retype looks.
    let mut deep = outlined(&mut s, json!({"x": 40, "y": 200, "text": "B", "size": 40}));
    for _ in 0..20 {
        s.execute("select.set", &json!({"ids": [deep]})).unwrap();
        deep = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    }
    s.execute("select.set", &json!({"ids": [top, deep]})).unwrap();
    let r = query(&mut s, json!({}), bundled());
    assert_eq!((r["shapes"].clone(), r["truncated"].clone()), (json!(1), json!(true)), "{r}");
}

/// Two versions of one font: choosing the font by name sets the version the name gives, so only
/// that version is compared, and a font whose other version alone matches is not listed.
#[test]
fn of_two_versions_only_the_one_the_name_gives_is_listed() {
    use vectorcraft_text::test_fonts::named_copy;
    const FAMILY: &str = "Versiontest Sans";
    // A database of its own, so that no other test sees these fonts.
    let db = FontDb::with_font_dirs(vec![]);
    // The first version loaded (Source Sans 3's glyphs) is the one the name gives.
    db.add_font(named_copy(false, (FAMILY, "Regular", "Version 1.000", "VersiontestSans-Regular")).unwrap());
    db.add_font(named_copy(true, (FAMILY, "Regular", "Version 2.000", "VersiontestSans-Regular")).unwrap());
    assert_eq!(db.face(FAMILY, "Regular").unwrap().version, "Version 1.000");
    let mut s = session();
    // Live type in the family puts the font among those compared first. Added last, after the
    // bundled fonts and however many faces craft-fonts supplies, it can fall past the fonts
    // Retype compares (`MAX_FACES`).
    s.execute("text.create", &json!({"x": 40, "y": 300, "text": "live", "size": 40, "font": FAMILY, "style": "Regular"})).unwrap();
    let inter = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Kg", "size": 40, "font": "Inter", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [inter]})).unwrap();
    let r = query(&mut s, json!({"count": 96}), &db);
    let fonts = names(&r);
    assert!(fonts.iter().any(|f| f.0 == "Inter"), "{r}");
    assert!(!fonts.iter().any(|f| f.0 == FAMILY), "version 2 matches, but choosing the font sets version 1: {r}");
    let source = outlined(&mut s, json!({"x": 40, "y": 200, "text": "Kg", "size": 40, "font": "Source Sans 3", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [source]})).unwrap();
    let r = query(&mut s, json!({"count": 96}), &db);
    assert_eq!(names(&r).iter().filter(|f| f.0 == FAMILY).count(), 1, "version 1 matches and is listed once: {r}");
}

#[test]
fn installed_fonts_not_loaded_are_ranked_too_unless_left_out() {
    use vectorcraft_text::test_fonts::named_copy;
    let dir = vectorcraft_testkit::temp_dir("retype-fonts-installed");
    let file = dir.join("RetypeListed-Regular.ttf");
    std::fs::write(&file, named_copy(true, ("Retype Listed", "Regular", "Version 1.000", "RetypeListed-Regular")).unwrap()).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let mut s = session();
    let inter = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Kgw", "size": 40, "font": "Inter", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [inter]})).unwrap();
    // Left out: the installed copy is not read, and stays unloaded.
    let r = query(&mut s, json!({"count": 96, "installed": false}), &db);
    assert!(!names(&r).iter().any(|f| f.0 == "Retype Listed"), "{r}");
    assert_eq!(r["installed"], json!({"read": 0, "skipped": 0}), "{r}");
    assert!(db.unloaded_installed_faces().iter().any(|f| f.family == "Retype Listed"));
    // Compared: the copy of Inter matches every shape, as Inter does, and is loaded.
    let r = query(&mut s, json!({"count": 96}), &db);
    let fonts = names(&r);
    assert!(fonts.contains(&("Retype Listed".to_string(), "Regular".to_string(), 3)), "{r}");
    assert!(fonts.contains(&("Inter".to_string(), "Regular".to_string(), 3)), "{r}");
    assert_eq!(r["installed"], json!({"read": 1, "skipped": 0}), "{r}");
    assert!(!db.unloaded_installed_faces().iter().any(|f| f.family == "Retype Listed"));
}

#[test]
fn installed_fonts_fill_the_list_up_to_96_fonts_in_all() {
    use vectorcraft_text::test_fonts::named_copy;
    let copy = |family: &str| {
        let postscript = format!("{}-Regular", family.replace(' ', ""));
        (named_copy(false, (family, "Regular", "Version 1.000", &postscript)).unwrap(), postscript)
    };
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Rg", "size": 40, "font": "Source Sans 3", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    // Loaded copies that match fill most of the list.
    let loaded = FontDb::with_font_dirs(vec![]);
    for i in 0..super::MAX_COUNT {
        loaded.add_font(copy(&format!("Fullcopy {i:02}")).0);
    }
    let listed = names(&query(&mut s, json!({"count": 96, "installed": false}), &loaded)).len();
    assert!(listed < super::MAX_COUNT, "{listed}");
    // Installed copies, two more than the places left.
    let dir = vectorcraft_testkit::temp_dir("retype-fonts-installed-full");
    for i in 0..super::MAX_COUNT - listed + 2 {
        let (data, postscript) = copy(&format!("Installed {i:02}"));
        std::fs::write(dir.join(format!("{postscript}.ttf")), data).unwrap();
    }
    let db = FontDb::with_font_dirs(vec![dir]);
    for i in 0..super::MAX_COUNT {
        db.add_font(copy(&format!("Fullcopy {i:02}")).0);
    }
    let r = query(&mut s, json!({"count": 96}), &db);
    assert_eq!(names(&r).len(), super::MAX_COUNT, "the installed copies fill the places left: {r}");
    assert_eq!(r["more"], json!(false), "no font past the list is found: {r}");
    assert_eq!(r["installed"]["skipped"], json!(2), "{r}");
    assert_eq!(db.unloaded_installed_faces().len(), 2, "the two past the list stay unloaded");
}

#[test]
fn shapes_that_stay_outlines_still_name_their_font() {
    // A glyph with paint type cannot take (two fills) is not retyped, but it is still that font's glyph.
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "M", "size": 40, "font": "Source Serif 4", "style": "Regular"}));
    let glyph = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(g)).unwrap().children().unwrap()[0].id.0;
    s.execute("select.set", &json!({"ids": [glyph]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let r = query(&mut s, json!({}), bundled());
    assert_eq!(names(&r).first().map(|f| (f.0.as_str(), f.1.as_str())), Some(("Source Serif 4", "Regular")), "{r}");
}
