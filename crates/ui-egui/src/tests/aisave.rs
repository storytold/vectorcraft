//! Save As .ai in the app: the options dialog, then a PDF-compatible file, which reopens as the
//! document and saves as .ai again (without asking); Save a Copy as .ai leaves the document's path
//! alone.

use serde_json::{Value, json};

use super::nativeoptions::answer;
use super::picks::open_dialogs;
use crate::tests_svg::{app, picked};

/// The fields of the options dialog the save opened.
fn options(app: &crate::VectorcraftApp) -> Value {
    open_dialogs(app).last().and_then(|d| d.fields.clone()).expect("the .ai options")
}

#[test]
fn save_as_ai_writes_a_pdf_that_reopens_editable() {
    let (mut app, written) = app("/tmp/art.ai");
    // Save As asks for the .ai options after the save panel (M4.38), then writes.
    let r = app.run("file.saveAs", Value::Null).unwrap();
    assert_eq!(r["pending"], "fileDialog", "{r}");
    picked(&mut app);
    let f = options(&app);
    assert_eq!((f["format"].clone(), f["pdfCompatible"].clone()), (json!("ai"), json!(true)));
    assert!(written.borrow().is_empty(), "nothing is written before OK");
    answer(&mut app, &[], true);
    assert!(open_dialogs(&app).is_empty());
    let (path, ai) = written.borrow()[0].clone();
    assert_eq!(path, "/tmp/art.ai");
    assert!(ai.starts_with(b"%PDF"));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/art.ai"));
    // Save writes .ai again; the document reopens from it.
    app.run("shape.ellipse", json!({"x": 50, "y": 40, "width": 20, "height": 20})).unwrap();
    app.run("file.save", Value::Null).unwrap();
    let saved = written.borrow()[1].clone();
    assert_eq!(saved.0, "/tmp/art.ai");
    let layers = app.session.doc().unwrap().doc.layers.clone();
    crate::io::open_bytes(&mut app, "/tmp/art.ai", &saved.1, Some("/tmp/art.ai".into())).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.layers, layers);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/art.ai"));
}

#[test]
fn save_a_copy_as_ai_keeps_the_path() {
    let (mut app, written) = app("/tmp/copy.ai");
    app.session.doc_mut().unwrap().path = Some("/tmp/doc.vectorcraft".into());
    // Save a Copy asks for the .ai options like Save As (#3141), then writes.
    let r = app.run("file.saveCopy", json!({"compression": {"compressText": false}})).unwrap();
    assert_eq!(r["pending"], "fileDialog", "{r}");
    picked(&mut app);
    assert_eq!(options(&app)["format"], "ai");
    answer(&mut app, &[], true);
    let w = written.borrow();
    assert_eq!(w[0].0, "/tmp/copy.ai");
    assert!(vectorcraft_pdf::editing(&w[0].1).is_some_and(|e| e.intact));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.vectorcraft"));
}

/// A document last saved as legacy version 7 .ai: Save a Copy as .ai asks for the options (Version
/// included) instead of silently writing another legacy file, and choosing PDF-compatible there
/// writes a PDF (#3141).
#[test]
fn save_a_copy_as_ai_asks_for_the_version() {
    let (mut app, written) = app("/tmp/copy.ai");
    app.run("file.saveAs", json!({"path": "/tmp/legacy.ai", "options": {"aiVersion": "7"}})).unwrap();
    written.borrow_mut().clear();
    let r = app.run("file.saveCopy", Value::Null).unwrap();
    assert_eq!(r["pending"], "fileDialog", "{r}");
    picked(&mut app);
    assert_eq!(options(&app)["format"], "ai");
    assert!(written.borrow().is_empty());
    answer(&mut app, &[("aiVersion", json!("pdf"))], true);
    assert!(written.borrow()[0].1.starts_with(b"%PDF"));
}
