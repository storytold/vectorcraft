//! A font removed while the app runs: once the fonts are scanned again (`text.rescanFonts`), its
//! family is no longer listed, and type set in it, which had loaded it, is laid out in the fallback
//! font and listed as missing, until the font is installed again. One test in its own process, so
//! the process-wide font database and font folders start fresh, as when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_text::{FALLBACK_FAMILY, FontDb};

const FAMILY: &str = "Gonefont Sans";

/// `text.fonts`' status for [`FAMILY`] and the family that stands in for it.
fn status(s: &mut Session) -> (String, String) {
    let listed = s.execute("text.fonts", &json!({})).unwrap();
    let row = listed.as_array().unwrap().iter().find(|f| f["family"] == FAMILY).cloned().unwrap();
    (row["status"].as_str().unwrap().into(), row["resolved"]["family"].as_str().unwrap().into())
}

/// Whether `text.fontList` lists [`FAMILY`].
fn listed(s: &mut Session) -> bool {
    let r = s.execute("text.fontList", &json!({})).unwrap();
    r["families"].as_array().unwrap().iter().any(|f| f == FAMILY)
}

/// The families of the faces the type object `id` is laid out in.
fn laid_out_in(s: &Session, id: u64) -> Vec<String> {
    let st = s.doc().unwrap();
    let Some(NodeKind::Text(t)) = st.doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("no type object {id}") };
    let db = FontDb::global();
    let mut families: Vec<String> = vectorcraft_text::layout(db, t).glyphs.iter().map(|g| db.face_by_id(g.font_id).unwrap().family.clone()).collect();
    families.dedup();
    families
}

#[test]
fn type_in_a_removed_font_falls_back_after_a_rescan() {
    let root = vectorcraft_testkit::temp_dir("removed-font");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    vectorcraft_text::set_user_font_dirs(vec![root.clone()]);
    let file = root.join("Gonefont.otf");
    std::fs::write(&file, vectorcraft_testkit::fonts::renamed(FAMILY)).unwrap();

    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Gone", "font": FAMILY})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(status(&mut s), ("exact".into(), FAMILY.into()));
    assert!(listed(&mut s));
    assert_eq!(laid_out_in(&s, id), [FAMILY]);

    // Removed: after a rescan the type is laid out in the fallback font, without editing the document.
    std::fs::remove_file(&file).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let revision = s.doc().unwrap().revision;
    let r: Value = s.execute("text.rescanFonts", &json!({})).unwrap();
    assert!(r["faces"].is_u64(), "{r}");
    assert_eq!(status(&mut s), ("missing".into(), FALLBACK_FAMILY.into()));
    assert!(!listed(&mut s));
    assert!(s.execute("text.fontList", &json!({"family": FAMILY})).is_err());
    assert_eq!(laid_out_in(&s, id), [FALLBACK_FAMILY]);
    let missing = s.execute("text.missingFonts", &json!({})).unwrap();
    assert_eq!(missing["fonts"][0]["family"], FAMILY, "{missing}");
    let st = s.doc().unwrap();
    assert!(st.revision > revision && !st.is_dirty());

    // Installed again: the type is set in it again.
    std::fs::write(&file, vectorcraft_testkit::fonts::renamed(FAMILY)).unwrap();
    s.execute("text.rescanFonts", &json!({})).unwrap();
    assert_eq!(status(&mut s), ("exact".into(), FAMILY.into()));
    assert!(listed(&mut s));
    assert_eq!(laid_out_in(&s, id), [FAMILY]);
    let _ = std::fs::remove_dir_all(&root);
}
