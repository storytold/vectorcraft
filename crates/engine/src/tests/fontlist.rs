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
    assert_eq!(fams, *FontDb::global().menu_family_list());
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
