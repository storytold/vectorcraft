//! VectorCraft Options after Save As: each artboard to a separate file (All or a range), Include
//! Linked Files, Embed ICC Profiles, Create PDF-Compatible File, Use Compression and Preview; the
//! web's Save As dialog shows them too.

use craft::ui::dialog::OpenDialog;
use serde_json::{Value, json};

use crate::dialogs::confirm::REPLACE_FILES;
use crate::tests_svg::app;
use crate::{VectorcraftApp, dialogs};

/// Gives `app` a window to host dialogs in.
pub(super) fn host(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.get_or_insert_with(egui::Context::default).clone();
    crate::testing::install(&ctx);
}

/// One frame of the host (which opens a queued question), then the pump; first the answers of the save panels, as a
/// frame's tick takes them.
pub(super) fn pass(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    crate::tests_svg::picked(app);
    let Some(dialogs) = app.dialogs() else { return };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs.with(|host| host.run(ui)));
    out.textures_delta.clear();
    dialogs::host::pump(app, &ctx);
}

/// The open dialogs, the top one last.
fn open_dialogs(app: &VectorcraftApp) -> Vec<OpenDialog> {
    app.dialogs().map(|d| d.with(|host| host.open_dialogs())).unwrap_or_default()
}

/// Sets `fields` on the top dialog and answers it: OK when `ok`, else Cancel.
pub(super) fn answer(app: &mut VectorcraftApp, fields: &[(&str, Value)], ok: bool) {
    pass(app);
    let ctx = app.ui_ctx.clone().unwrap();
    for (field, value) in fields {
        app.dialogs().unwrap().with(|host| {
            let id = host.open_dialogs().last().unwrap().id;
            host.set_json_field(&ctx, id, field, value.clone()).unwrap_or_else(|e| panic!("{field}: {e}"));
        });
        // A row this one enables (All after Save each artboard) takes input once the dialog showed it.
        pass(app);
    }
    app.dialogs().unwrap().with(|host| {
        let id = host.open_dialogs().last().unwrap().id;
        if ok {
            host.confirm_by_id(&ctx, id);
        } else {
            host.cancel_by_id(&ctx, id);
        }
    });
    // OK is held: the pump accepts the options, and the next frame closes the dialog and goes on with the save.
    pass(app);
    pass(app);
}

/// The files the replace question open now names.
pub(super) fn replace_files(app: &VectorcraftApp) -> Option<Vec<String>> {
    dialogs::confirm::open_replace_files(app)
}

/// Add artboards B and C, a rectangle on each.
fn three_boards(app: &mut VectorcraftApp) {
    for (x, name) in [(200, "B"), (400, "C")] {
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 100, "height": 80, "name": name})).unwrap();
        app.run("shape.rectangle", json!({"x": x + 10, "y": 10, "width": 20, "height": 20})).unwrap();
    }
}

#[test]
fn save_as_native_asks_for_its_options_and_writes_every_artboard() {
    let (mut app, written) = app("/tmp/doc.vectorcraft");
    host(&mut app);
    three_boards(&mut app);
    let r = app.run("file.saveAs", Value::Null).unwrap();
    assert_eq!(r["pending"], "fileDialog");
    pass(&mut app);
    let open = open_dialogs(&app);
    assert_eq!(open.len(), 1);
    let f = open[0].fields.clone().unwrap();
    assert_eq!(
        (&f["format"], &f["separateArtboards"], &f["embedProfiles"], &f["pdfCompatible"]),
        (&json!("vectorcraft"), &json!(false), &json!(true), &json!(true))
    );
    assert!(written.borrow().is_empty(), "nothing is written before OK");
    answer(&mut app, &[("separateArtboards", json!(true)), ("all", json!("false")), ("range", json!("9"))], true);
    assert_eq!(open_dialogs(&app).len(), 1, "a bad range keeps the dialog open");
    assert!(written.borrow().is_empty());
    answer(&mut app, &[("range", json!("2-3"))], true);
    assert_eq!(names(&written), ["/tmp/doc.vectorcraft", "/tmp/doc-B.vectorcraft", "/tmp/doc-C.vectorcraft"]);
    let st = app.session.active().unwrap();
    assert_eq!((st.path.as_deref(), st.is_dirty()), (Some("/tmp/doc.vectorcraft"), false));
    // Save writes them again with the same options, without asking.
    app.run("shape.ellipse", json!({"x": 50, "y": 40, "width": 10, "height": 10})).unwrap();
    app.run("file.save", Value::Null).unwrap();
    assert!(open_dialogs(&app).is_empty());
    assert_eq!(written.borrow().len(), 6);
    // All: every artboard.
    app.run("file.saveAs", Value::Null).unwrap();
    pass(&mut app);
    assert_eq!(open_dialogs(&app)[0].fields.as_ref().unwrap()["separateArtboards"], json!(true), "the options as last saved");
    answer(&mut app, &[("all", json!("true"))], true);
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
    host(&mut app);
    three_boards(&mut app);
    let there = ["/tmp/doc-B.vectorcraft", "/tmp/doc-D.vectorcraft", "/tmp/doc-E.vectorcraft"];
    app.services.exists = Some(std::sync::Arc::new(move |p: &str| there.contains(&p)));
    let pick = |app: &mut VectorcraftApp| {
        app.run("file.saveAs", Value::Null).unwrap();
        answer(app, &[("separateArtboards", json!(true)), ("all", json!("true"))], true);
    };
    pick(&mut app);
    assert_eq!(replace_files(&app), Some(vec!["/tmp/doc-B.vectorcraft".to_string()]));
    assert!(written.borrow().is_empty(), "nothing is written while it asks");
    answer(&mut app, &[], false);
    assert!(written.borrow().is_empty(), "Cancel writes nothing");
    assert!(app.session.active().unwrap().path.is_none());
    pick(&mut app);
    answer(&mut app, &[], true);
    assert_eq!(names(&written), ["/tmp/doc.vectorcraft", "/tmp/doc-Artboard-1.vectorcraft", "/tmp/doc-B.vectorcraft", "/tmp/doc-C.vectorcraft"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.vectorcraft"));
    app.run("file.save", Value::Null).unwrap();
    assert!(open_dialogs(&app).is_empty());
    assert_eq!(written.borrow().len(), 8);
    // A new artboard's file is not one the last save wrote: Save asks about it alone.
    app.run("artboard.new", json!({"x": 600, "y": 0, "width": 100, "height": 80, "name": "D"})).unwrap();
    let r = app.run("file.save", Value::Null).unwrap();
    assert_eq!(r["pending"], REPLACE_FILES, "{r}");
    pass(&mut app);
    assert_eq!(replace_files(&app), Some(vec!["/tmp/doc-D.vectorcraft".to_string()]));
    assert_eq!(written.borrow().len(), 8);
    // An artboard added while it asks writes a file the question did not name: OK asks about it.
    app.run("artboard.new", json!({"x": 800, "y": 0, "width": 100, "height": 80, "name": "E"})).unwrap();
    answer(&mut app, &[], true);
    assert_eq!(replace_files(&app), Some(vec!["/tmp/doc-E.vectorcraft".to_string()]));
    assert_eq!(written.borrow().len(), 8);
    answer(&mut app, &[], true);
    assert_eq!(written.borrow().len(), 14);
    assert!(!app.session.active().unwrap().is_dirty());
}

/// A question asked when a Background Save or Export ends waits for a dialog open then to close.
#[test]
fn the_question_waits_for_an_open_dialog() {
    let (mut app, _) = app("/tmp/doc.vectorcraft");
    host(&mut app);
    let uid = app.session.active().unwrap().uid;
    dialogs::confirm::tell(&mut app, "Open", "");
    dialogs::confirm::ask_replace_files(&mut app, uid, &["/tmp/doc-B.vectorcraft".into()], &[], "file.save", json!({}));
    pass(&mut app);
    let keys: Vec<_> = open_dialogs(&app).into_iter().map(|o| o.key).collect();
    assert_eq!(keys, [Some(dialogs::confirm::MESSAGE.to_string())]);
    assert_eq!(replace_files(&app), None);
    answer(&mut app, &[], false);
    assert_eq!(replace_files(&app), Some(vec!["/tmp/doc-B.vectorcraft".to_string()]));
}

/// Export with a file per artboard asks the same way, naming every file that is there; OK writes
/// them.
#[test]
fn export_asks_before_replacing_artboard_files_that_are_there() {
    let (mut app, written) = app("/tmp/art.png");
    host(&mut app);
    three_boards(&mut app);
    let there = ["/tmp/art-B.png", "/tmp/art-C.png"];
    app.services.exists = Some(std::sync::Arc::new(move |p: &str| there.contains(&p)));
    let r = crate::io::export(&mut app, Some("png"), Some("/tmp/art.png".into()), &json!({"useArtboards": true})).unwrap();
    assert_eq!(r["pending"], REPLACE_FILES, "{r}");
    pass(&mut app);
    assert_eq!(replace_files(&app), Some(vec!["/tmp/art-B.png".to_string(), "/tmp/art-C.png".to_string()]));
    assert!(written.borrow().is_empty());
    // Another document made active meanwhile: OK exports the one that asked.
    let uid = app.session.active().unwrap().uid;
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    answer(&mut app, &[], true);
    assert_eq!(names(&written), ["/tmp/art-Artboard-1.png", "/tmp/art-B.png", "/tmp/art-C.png"]);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uid));
}

#[test]
fn save_a_copy_and_save_dont_ask_and_options_reach_the_file() {
    let (mut app, written) = app("/tmp/plain.vectorcraft");
    host(&mut app);
    // Save of an untitled document goes straight to the file, as before.
    app.run("file.save", Value::Null).unwrap();
    assert!(open_dialogs(&app).is_empty());
    assert_eq!(written.borrow().len(), 1);
    app.run("file.saveAs", Value::Null).unwrap();
    answer(&mut app, &[("pdfCompatible", json!(true)), ("compress", json!(false))], true);
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
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
    host(&mut app);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    three_boards(&mut app);
    app.run("file.saveAs", Value::Null).unwrap();
    pass(&mut app);
    let f = open_dialogs(&app)[0].fields.clone().unwrap();
    assert!(f.get("__pick").is_some() && f.get("separateArtboards").is_some());
    answer(&mut app, &[("separateArtboards", json!(true))], true);
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["Untitled-1.vectorcraft", "Untitled-1-Artboard-1.vectorcraft", "Untitled-1-B.vectorcraft", "Untitled-1-C.vectorcraft"]);
}
