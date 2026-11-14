//! Fonts added to the session from their bytes (`text.addFonts`), as the web build gets the fonts it
//! doesn't bundle: listed, found exactly and drawn, without editing the document; a face loaded
//! already is kept, a file that is no font is skipped, and a malformed call adds nothing. One test
//! in its own process, so the process-wide font database starts fresh, as when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_text::{FALLBACK_FAMILY, FontDb};

const FAMILY: &str = "Pagefont Sans";

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

/// `text.addFonts` with one file of `bytes`, named `name`.
fn add(s: &mut Session, name: &str, bytes: &[u8]) -> Value {
    s.execute("text.addFonts", &json!({"fonts": [{"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}]})).unwrap()
}

#[test]
fn a_font_added_from_its_bytes_is_listed_found_and_drawn() {
    let font = vectorcraft_testkit::fonts::renamed(FAMILY);
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Page", "font": FAMILY})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(status(&mut s), ("missing".into(), FALLBACK_FAMILY.into()));
    assert!(!listed(&mut s));
    s.doc_mut().unwrap().mark_saved();
    let revision = s.doc().unwrap().revision;

    // Refused before anything is added: no files, too many, no bytes, bytes that aren't base64, a
    // name that isn't text, and more than 256 MB in all.
    let file = json!({"dataBase64": vectorcraft_format::base64_encode(&font)});
    let too_large = "A".repeat((256 << 20) / 3 * 4 + 8);
    let refused = [
        json!({}),
        json!({"fonts": []}),
        json!({"fonts": file.clone()}),
        json!({"fonts": vec![file.clone(); 101]}),
        json!({"fonts": [{"name": "a.ttf"}]}),
        json!({"fonts": [file.clone(), {"dataBase64": "not base64!"}]}),
        json!({"fonts": [{"dataBase64": "AAAA", "name": 7}]}),
        json!({"fonts": [file, {"dataBase64": too_large}]}),
    ];
    for (i, p) in refused.iter().enumerate() {
        let r = s.execute("text.addFonts", p);
        assert!(r.as_ref().is_err_and(|e| matches!(e, vectorcraft_engine::EngineError::BadParams { .. })), "case {i}: {r:?}");
    }
    assert!(!listed(&mut s), "a refused call adds nothing");

    // Added: listed, found exactly and drawn in it, without editing the document.
    let r = add(&mut s, "Pagefont.ttf", &font);
    assert_eq!((&r["added"], &r["kept"], &r["skipped"]), (&json!([{"family": FAMILY, "style": "Regular"}]), &json!([]), &json!([])), "{r}");
    assert_eq!(r["families"], json!(FontDb::global().menu_family_list().len()));
    assert_eq!(status(&mut s), ("exact".into(), FAMILY.into()));
    assert!(listed(&mut s));
    assert_eq!(s.execute("text.fontList", &json!({"family": FAMILY})).unwrap()["styles"], json!(["Regular"]));
    assert_eq!(laid_out_in(&s, id), [FAMILY]);
    assert_eq!(s.execute("text.missingFonts", &json!({})).unwrap()["count"], 0);
    let st = s.doc().unwrap();
    assert!(st.revision > revision && !st.is_dirty(), "the canvas redraws; nothing to save");

    // Again: kept as it is. A file that is no font, or a web font, is skipped and says why.
    let mut woff2 = b"wOF2".to_vec();
    woff2.extend_from_slice(&[0; 44]);
    let r = s
        .execute(
            "text.addFonts",
            &json!({"fonts": [
                {"name": "Pagefont.ttf", "dataBase64": vectorcraft_format::base64_encode(&font)},
                {"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(b"not a font")},
                {"dataBase64": vectorcraft_format::base64_encode(&woff2)},
            ]}),
        )
        .unwrap();
    assert_eq!((&r["added"], &r["kept"]), (&json!([]), &json!([{"family": FAMILY, "style": "Regular"}])), "{r}");
    assert_eq!(r["skipped"][0], json!({"index": 1, "name": "notes.txt", "reason": "not a font file"}), "{r}");
    assert_eq!(r["skipped"][1]["index"], 2, "{r}");
    assert!(r["skipped"][1].get("name").is_none() && r["skipped"][1]["reason"].as_str().unwrap().contains("WOFF"), "{r}");
    assert_eq!(status(&mut s), ("exact".into(), FAMILY.into()));
}
