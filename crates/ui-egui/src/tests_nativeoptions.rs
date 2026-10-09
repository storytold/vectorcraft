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
        ("vectorcraft", false, true, false)
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
        download: Some(Box::new(move |name: &str, b: &[u8]| w.borrow_mut().push((name.to_string(), b.to_vec())))),
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
    assert_eq!(names, ["Untitled-1.ai", "Untitled-1-Artboard-1.ai", "Untitled-1-B.ai", "Untitled-1-C.ai"]);
}
