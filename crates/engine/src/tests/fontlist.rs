//! The fonts available (`text.fontList`), the installed ones listed without asking for them (#36),
//! and Refresh Font List (`text.rescanFonts`).

use serde_json::{Value, json};
use vectorcraft_text::{FontDb, system_font_dirs};

use crate::*;

fn names(v: &Value) -> Vec<String> {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn the_font_list_names_every_available_family_with_its_styles() {
    let mut s = Session::new();
    let fams = names(&s.execute("text.fontList", &json!({})).unwrap()["families"]);
    // Other tests running at the same time add and remove composite fonts, which the lists show
    // first; the installed families are compared.
    let db = FontDb::global();
    let installed_only = |list: &[String]| list.iter().filter(|f| db.has_installed_family(f)).cloned().collect::<Vec<_>>();
    assert_eq!(installed_only(&fams), installed_only(&db.menu_family_list()));
    for f in ["Source Sans 3", "Source Serif 4", "Inter", "JetBrains Mono"] {
        assert!(fams.iter().any(|x| x == f), "{f}");
    }
    // Every installed family, though nothing asked for system fonts.
    let installed = FontDb::with_font_dirs(system_font_dirs());
    installed.load_system_fonts();
    let missing: Vec<String> = installed.menu_family_list().iter().filter(|f| !fams.contains(f)).cloned().collect();
    assert!(missing.is_empty(), "{missing:?}");
    let r = s.execute("text.fontList", &json!({"family": "source sans 3"})).unwrap();
    assert_eq!(r["family"], "Source Sans 3");
    let styles = names(&r["styles"]);
    assert!(["Regular", "Semibold", "Bold", "Italic"].iter().all(|x| styles.iter().any(|s| s == x)), "{styles:?}");
    assert!(s.execute("text.fontList", &json!({"family": "No Such Font"})).is_err());
}

/// Show Similar Fonts (`text.similarFonts`): every listed family, the asked one first, in the
/// order the font menus show; bad params are refused.
#[test]
fn similar_fonts_lists_every_family_by_likeness() {
    let mut s = Session::new();
    let db = FontDb::global();
    let r = s.execute("text.similarFonts", &json!({"family": "source sans 3"})).unwrap();
    assert_eq!(r["family"], "Source Sans 3");
    let order = names(&r["families"]);
    assert_eq!(order.first().map(String::as_str), Some("Source Sans 3"));
    let mut sorted = order.clone();
    sorted.sort();
    let mut listed = db.menu_family_list().to_vec();
    listed.sort();
    // Other tests running at the same time add and remove composite fonts; the installed families are compared.
    let installed_only = |list: &[String]| list.iter().filter(|f| db.has_installed_family(f)).cloned().collect::<Vec<_>>();
    assert_eq!(installed_only(&sorted), installed_only(&listed), "every family once");
    // The two answers are compared whole when the available fonts did not change while both were
    // made (the same `FontDb::generation`), and by their installed families when they changed
    // during every try.
    let mut answers = (vec![], vec![], false);
    for _ in 0..20 {
        let generation = db.generation();
        let all = names(&s.execute("text.similarFonts", &json!({"family": "Source Sans 3"})).unwrap()["families"]);
        let two = names(&s.execute("text.similarFonts", &json!({"family": "Source Sans 3", "limit": 2})).unwrap()["families"]);
        let unchanged = db.generation() == generation;
        answers = (all, two, unchanged);
        if unchanged {
            break;
        }
    }
    let (all, two, unchanged) = answers;
    assert_eq!(two.len(), 2);
    if unchanged {
        assert_eq!(two, all[..2], "the first families, in order");
    } else {
        let (two, all) = (installed_only(&two), installed_only(&all));
        assert_eq!(two[..], all[..two.len()], "the first installed families, in order");
    }
    for (params, error) in [
        (json!({}), "`family` is required"),
        (json!({"family": " "}), "`family` is required"),
        (json!({"family": 3}), "`family` must be a string"),
        (json!({"family": "No Such Font"}), "font family `No Such Font` is not available"),
        (json!({"family": "Inter", "limit": 0}), "`limit` must be a whole number from 1 to 100000"),
        (json!({"family": "Inter", "limit": 2.5}), "`limit` must be a whole number from 1 to 100000"),
        (json!({"family": "Inter", "limit": "2"}), "`limit` must be a whole number from 1 to 100000"),
    ] {
        let e = s.execute("text.similarFonts", &params).unwrap_err().to_string();
        assert!(e.contains(error), "{params}: {e}");
    }
}

#[test]
fn refreshing_the_font_list_redraws_type_without_editing_the_document() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font"})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let revision = s.doc().unwrap().revision;
    let r = s.execute("text.rescanFonts", &json!({})).unwrap();
    assert_eq!(r["families"], json!(FontDb::global().menu_family_list().len()));
    assert!(r["faces"].is_u64());
    let d = s.doc().unwrap();
    assert!(d.revision > revision, "the canvas redraws");
    assert!(!d.is_dirty(), "nothing to save");
}

/// `text.glyphCategories` lists the kinds of glyph a font's OpenType features give, with a
/// stylistic set's name, and refuses a font that isn't available and params that aren't strings.
#[test]
fn glyph_categories_list_the_glyphs_of_each_kind_of_feature() {
    use vectorcraft_text::test_fonts::{CATEGORIES_FAMILY, categories_font, source_sans_gid};
    FontDb::global().add_font(categories_font().expect("the test font builds"));
    let mut s = Session::new();
    let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
    let mut figures = vec![g('7'), g('8')];
    figures.sort_unstable();
    let r = s.execute("text.glyphCategories", &json!({"family": CATEGORIES_FAMILY})).unwrap();
    assert_eq!(r["family"], CATEGORIES_FAMILY);
    assert_eq!(r["style"], "Regular");
    assert_eq!(
        r["categories"],
        json!([
            {"id": "ligatures", "glyphs": [g('L')]},
            {"id": "ss02", "name": "Round Dots", "glyphs": [g('G')]},
            {"id": "smallCaps", "glyphs": [g('B')]},
            {"id": "figures", "glyphs": figures},
        ])
    );
    assert_eq!(s.execute("text.glyphCategories", &json!({"family": CATEGORIES_FAMILY, "style": "regular"})).unwrap(), r);
    for (p, says) in [
        (json!({}), "missing `family`"),
        (json!({"family": 3}), "`family` must be a string"),
        (json!({"family": "No Such Font"}), "is not available"),
        (json!({"family": CATEGORIES_FAMILY, "style": "Black"}), "is not available"),
        (json!({"family": CATEGORIES_FAMILY, "style": true}), "`style` must be a string"),
    ] {
        let e = s.execute("text.glyphCategories", &p).unwrap_err().to_string();
        assert!(e.contains(says), "{p}: {e}");
    }
}

/// Refreshing the font list while a background save writes the file leaves the document saved
/// once the file is written.
#[test]
fn refreshing_the_font_list_during_a_save_leaves_the_document_saved() {
    use crate::cmd::fileio::{SaveMode, save_job, save_plan};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font"})).unwrap();
    let uid = s.doc().unwrap().uid;
    let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": "/x/type.vectorcraft", "modified": 777})).unwrap();
    let job = save_job(&mut s, plan).unwrap();
    s.execute("text.rescanFonts", &json!({})).unwrap();
    job.write(|_, _| Ok(())).unwrap();
    job.complete(&mut s, uid);
    let d = s.doc().unwrap();
    assert_eq!(d.doc.metadata.modified, Some(777));
    assert!(!d.is_dirty());
}
