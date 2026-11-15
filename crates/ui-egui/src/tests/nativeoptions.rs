//! VectorCraft Options after Save As: each artboard to a separate file (All or a range), Include
//! Linked Files, Embed ICC Profiles, Create PDF-Compatible File, Use Compression and Preview; the
//! web's Save As dialog shows them too.

use serde_json::{Value, json};

use crate::tests_svg::app;
use crate::{dialogs, theme};

/// One headless frame of the dialog layer.
fn frame(app: &mut crate::VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Add artboards B and C, a rectangle on each.
fn three_boards(app: &mut crate::VectorcraftApp) {
    for (x, name) in [(200, "B"), (400, "C")] {
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 100, "height": 80, "name": name})).unwrap();
        app.run("shape.rectangle", json!({"x": x + 10, "y": 10, "width": 20, "height": 20})).unwrap();
    }
}

#[test]
fn save_as_native_asks_for_its_options_and_writes_every_artboard() {
    let (mut app, written) = app("/tmp/doc.vectorcraft");
    three_boards(&mut app);
    let r = app.run("file.saveAs", Value::Null).unwrap();
    assert_eq!(r["pending"], "saveOptions");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("format").as_str(), d.bool("separateArtboards"), d.bool("embedProfiles"), d.bool("pdfCompatible")),
        ("vectorcraft", false, true, true)
    );
    assert!(written.borrow().is_empty(), "nothing is written before OK");
    frame(&mut app);
    assert!(app.ui.dialog.is_some(), "drawing keeps it open");
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("separateArtboards".into(), json!(true));
    d.fields.insert("all".into(), json!(false));
    d.fields.insert("range".into(), json!("9"));
    frame(&mut app);
    assert!(dialogs::confirm(&mut app).is_err(), "a bad range keeps the dialog open");
    assert!(app.ui.dialog.is_some());
    app.ui.dialog.as_mut().unwrap().fields.insert("range".into(), json!("2-3"));
    let r = dialogs::confirm(&mut app).unwrap();
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["/tmp/doc.vectorcraft", "/tmp/doc-B.vectorcraft", "/tmp/doc-C.vectorcraft"], "{r}");
    let st = app.session.active().unwrap();
    assert_eq!((st.path.as_deref(), st.is_dirty()), (Some("/tmp/doc.vectorcraft"), false));
    // Save writes them again with the same options, without asking.
    app.run("shape.ellipse", json!({"x": 50, "y": 40, "width": 10, "height": 10})).unwrap();
    app.run("file.save", Value::Null).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 6);
    // All: every artboard.
    app.run("file.saveAs", Value::Null).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    assert!(d.bool("separateArtboards"), "the options as last saved");
    d.fields.insert("all".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(written.borrow().len(), 10);
}

/// The paths written so far.
fn names(written: &crate::tests_svg::Written) -> Vec<String> {
    written.borrow().iter().map(|(p, _)| p.clone()).collect()
}

/// Save As with a file per artboard: a file there with an artboard file's name, which the save
/// panel did not ask about, is asked about before anything is written. Cancel writes nothing; OK
/// writes every file, and Save to the document's own file then writes them again without asking.
#[test]
fn save_as_asks_before_replacing_an_artboard_file_that_is_there() {
    let (mut app, written) = app("/tmp/doc.vectorcraft");
    three_boards(&mut app);
    let there = ["/tmp/doc-B.vectorcraft", "/tmp/doc-D.vectorcraft", "/tmp/doc-E.vectorcraft"];
    app.services.exists = Some(std::sync::Arc::new(move |p: &str| there.contains(&p)));
    let pick = |app: &mut crate::VectorcraftApp| {
        app.run("file.saveAs", Value::Null).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("separateArtboards".into(), json!(true));
        d.fields.insert("all".into(), json!(true));
        dialogs::confirm(app).unwrap()
    };
    let r = pick(&mut app);
    assert_eq!(r["pending"], dialogs::confirm::REPLACE_FILES, "{r}");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("message").as_str()), (dialogs::confirm::REPLACE_FILES, "“doc-B.vectorcraft” already exists. Replace it?"));
    assert_eq!(d.fields["files"], json!(["/tmp/doc-B.vectorcraft"]));
    assert!(written.borrow().is_empty(), "nothing is written while it asks");
    dialogs::cancel(&mut app);
    assert!(written.borrow().is_empty(), "Cancel writes nothing");
    assert!(app.session.active().unwrap().path.is_none());
    pick(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/tmp/doc.vectorcraft", "/tmp/doc-Artboard-1.vectorcraft", "/tmp/doc-B.vectorcraft", "/tmp/doc-C.vectorcraft"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.vectorcraft"));
    app.run("file.save", Value::Null).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 8);
    // A new artboard's file is not one the last save wrote: Save asks about it alone.
    app.run("artboard.new", json!({"x": 600, "y": 0, "width": 100, "height": 80, "name": "D"})).unwrap();
    let r = app.run("file.save", Value::Null).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::REPLACE_FILES, "{r}");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["files"], json!(["/tmp/doc-D.vectorcraft"]));
    assert_eq!(written.borrow().len(), 8);
    // An artboard added while it asks writes a file the question did not name: OK asks about it.
    app.run("artboard.new", json!({"x": 800, "y": 0, "width": 100, "height": 80, "name": "E"})).unwrap();
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["files"], json!(["/tmp/doc-E.vectorcraft"]));
    assert_eq!(written.borrow().len(), 8);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(written.borrow().len(), 14);
    assert!(!app.session.active().unwrap().is_dirty());
}

/// A question asked when a Background Save or Export ends waits for a dialog open then to close.
#[test]
fn the_question_waits_for_an_open_dialog() {
    let (mut app, _) = app("/tmp/doc.vectorcraft");
    let uid = app.session.active().unwrap().uid;
    dialogs::confirm::tell(&mut app, "Open", "");
    dialogs::confirm::ask_replace_files(&mut app, uid, &["/tmp/doc-B.vectorcraft".into()], &[], "file.save", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("message")), Some("Open".to_string()));
    dialogs::cancel(&mut app);
    frame(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::confirm::REPLACE_FILES));
}

/// Export with a file per artboard asks the same way, naming every file that is there; OK writes
/// them.
#[test]
fn export_asks_before_replacing_artboard_files_that_are_there() {
    let (mut app, written) = app("/tmp/art.png");
    three_boards(&mut app);
    let there = ["/tmp/art-B.png", "/tmp/art-C.png"];
    app.services.exists = Some(std::sync::Arc::new(move |p: &str| there.contains(&p)));
    let r = crate::io::export(&mut app, Some("png"), Some("/tmp/art.png".into()), &json!({"useArtboards": true})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::REPLACE_FILES, "{r}");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("message").as_str(), d.str("detail").as_str(), d.str("__command").as_str()),
        ("“art-B.png” already exists. Replace it?", "art-B.png\nart-C.png", "file.exportAs")
    );
    assert!(written.borrow().is_empty());
    // Another document made active meanwhile: OK exports the one that asked.
    let uid = app.session.active().unwrap().uid;
    let question = app.ui.dialog.take();
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.ui.dialog = question;
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/tmp/art-Artboard-1.png", "/tmp/art-B.png", "/tmp/art-C.png"]);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uid));
}

#[test]
fn save_a_copy_and_save_dont_ask_and_options_reach_the_file() {
    let (mut app, written) = app("/tmp/plain.vectorcraft");
    // Save of an untitled document goes straight to the file, as before.
    app.run("file.save", Value::Null).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 1);
    app.run("file.saveAs", Value::Null).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("pdfCompatible".into(), json!(true));
    d.fields.insert("compress".into(), json!(false));
    dialogs::confirm(&mut app).unwrap();
    let bytes = written.borrow()[1].1.clone();
    assert!(vectorcraft_format::pdf_content(&bytes).is_some_and(|p| p.starts_with(b"%PDF")));
    assert!(!vectorcraft_format::is_compressed(&bytes));
}

#[test]
fn the_web_save_as_dialog_shows_the_native_options() {
    let written = crate::tests_svg::Written::default();
    let w = written.clone();
    let services = crate::Services {
        download: Some(Box::new(move |name: &str, b: &[u8]| {
            w.borrow_mut().push((name.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    three_boards(&mut app);
    app.run("file.saveAs", Value::Null).unwrap();
    frame(&mut app);
    let d = app.ui.dialog.as_mut().unwrap();
    assert!(d.bool("__pick") && d.fields.contains_key("separateArtboards"));
    d.fields.insert("separateArtboards".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["Untitled-1.vectorcraft", "Untitled-1-Artboard-1.vectorcraft", "Untitled-1-B.vectorcraft", "Untitled-1-C.vectorcraft"]);
}
