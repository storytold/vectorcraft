//! Exporter plug-ins in File › Export › Export As: listed after the built-in formats, written
//! whole (Use Artboards is not theirs to apply), and their parameters asked for once the file is
//! picked. Each test installs exporters of its own and removes them: plug-ins are process-wide, and
//! a format left installed would show in other tests' Export As.

use serde_json::{Value, json};
use vectorcraft_plugins::wat::{self as tpl, Echo};

use super::tests_export::{Written, answer_saves, app, app_saving, fields, frame, kind, names, set, try_confirm, try_set};
use super::*;
use crate::VectorcraftApp;

/// The id of the hosted dialog on top.
pub(super) fn top(app: &VectorcraftApp) -> Option<egui::Id> {
    app.dialogs()?.with(|host| host.open_dialogs().last().map(|d| d.id))
}

fn install(app: &mut VectorcraftApp, src: &str) {
    let wasm = wat::parse_str(src).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
}

/// Export As lists an exporter after the built-in formats and writes its file of the whole
/// document (Use Artboards greyed out), with the exporter's warnings in the status bar.
#[test]
fn export_as_lists_exporters_and_writes_their_file_whole() {
    let (mut app, written) = app(2);
    install(&mut app, &tpl::returning(&tpl::exporter("test.ui.whole", r#"["tuw"]"#, "{}"), r#"{"dataBase64":"SGk=","warnings":["a note"]}"#));
    // After the built-in formats, with the other exporters installed.
    let listed: Vec<&str> = export_as::formats().iter().map(|f| f.id).collect();
    let first = listed.iter().position(|id| id.starts_with("plugin.")).expect("an exporter listed");
    let added = listed.get(first..).unwrap_or_default();
    assert!(added.iter().all(|id| id.starts_with("plugin.")) && added.contains(&"plugin.test.ui.whole"), "{listed:?}");
    // It isn't a filter: Object › Plug-ins' command says where it is.
    let e = app.run("plugin.dialog", json!({"id": "test.ui.whole"})).unwrap_err();
    assert!(e.contains("is an exporter (File › Export › Export As…)"), "{e}");

    app.run("file.exportAs", json!({"format": "plugin.test.ui.whole"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(fields(&app)["format"], "plugin.test.ui.whole");
    // Use Artboards is greyed out and doesn't apply: one file of the whole document.
    assert!(try_set(&mut app, "useArtboards", json!(true)).is_err(), "Use Artboards is greyed out");
    frame(&mut app);
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none(), "an exporter without parameters writes at once");
    assert_eq!(names(&written), ["/out/Untitled-1.tuw"]);
    assert_eq!(written.borrow()[0].1, b"Hi");
    assert!(app.ui.status.contains("a note"), "{}", app.ui.status);
    app.run("plugin.remove", json!({"id": "test.ui.whole"})).unwrap();
}

/// An exporter with parameters asks for them once the file is picked, and writes with them.
#[test]
fn an_exporter_with_parameters_asks_for_them_once_the_file_is_picked() {
    let (mut app, written) = app(1);
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    install(&mut app, &tpl::echo_exporter(&tpl::exporter("test.ui.params", r#"["tup"]"#, params), Echo::Params));
    // Asked for by its extension, as agents may.
    app.run("file.exportAs", json!({"format": "tup"})).unwrap();
    assert_eq!(fields(&app)["format"], "plugin.test.ui.params");
    try_confirm(&mut app).unwrap();
    assert_eq!(kind(&app).as_deref(), Some(plugin::EXPORT_KIND));
    assert_eq!(fields(&app)["density"], json!(4.0));
    assert!(written.borrow().is_empty(), "nothing written before Export");
    frame(&mut app);
    set(&mut app, "density", json!(7));
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none());
    assert_eq!(names(&written), ["/out/Untitled-1.tup"]);
    let sent: Value = serde_json::from_slice(&written.borrow()[0].1).unwrap();
    assert_eq!(sent, json!({"density": 7.0, "_context": {"mode": "export", "extension": "tup", "name": "Untitled-1"}}));
    app.run("plugin.remove", json!({"id": "test.ui.params"})).unwrap();
}

/// [`app`] with a second drawing that has no ellipse, the first active and saved in `dir`, so it
/// closes without asking and can be reverted.
fn two_drawings(dir: &vectorcraft_testkit::TempDir) -> (VectorcraftApp, Written) {
    let (mut app, written) = app(1);
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
    app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
    app.run("file.saveAs", json!({"path": dir.join("first.vectorcraft")})).unwrap();
    let w = written.clone();
    app.services.write = Some(Box::new(move |p: &str, b: &[u8]| {
        w.borrow_mut().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    (app, written)
}

/// Installs exporter `id` (extension `tus`), which has parameters and writes the document it is
/// handed.
fn install_source_exporter(app: &mut VectorcraftApp, id: &str) {
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    install(app, &tpl::echo_exporter(&tpl::exporter(id, r#"["tus"]"#, params), Echo::Input));
}

/// Export As with exporter `id`, up to its parameters dialog, opened on the active drawing.
fn open_source_exporter(app: &mut VectorcraftApp, id: &str) {
    app.run("file.exportAs", json!({"format": format!("plugin.{id}")})).unwrap();
    try_confirm(app).unwrap();
    assert_eq!(kind(app).as_deref(), Some(plugin::EXPORT_KIND));
    frame(app);
}

/// The exporter's files.
fn exported(written: &Written) -> Vec<(String, Vec<u8>)> {
    written.borrow().iter().filter(|(p, _)| p.ends_with(".tus")).cloned().collect()
}

/// Export writes the drawing the dialog opened on, made active again, not the one active when
/// Export is pressed.
#[test]
fn an_exporter_writes_the_drawing_its_dialog_opened_on() {
    let dir = vectorcraft_testkit::TempDir::new("plugin-export-opening-drawing");
    let (mut app, written) = two_drawings(&dir);
    let id = "test.ui.opening";
    install_source_exporter(&mut app, id);
    open_source_exporter(&mut app, id);
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app);
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none());
    assert_eq!(app.session.active_index(), Some(0), "the opening drawing is active again");
    // The second drawing, exported from its own dialog, is another file.
    app.run("document.activate", json!({"index": 1})).unwrap();
    open_source_exporter(&mut app, id);
    try_confirm(&mut app).unwrap();
    let files = exported(&written);
    assert_eq!(files.len(), 2, "{:?}", files.iter().map(|(p, _)| p).collect::<Vec<_>>());
    assert_ne!(files[0].1, files[1].1, "the first export wrote the first drawing, not the second");
    app.run("plugin.remove", json!({"id": id})).unwrap();
}

/// A drawing reverted or closed while its exporter's dialog was open is not exported, and the
/// status bar says why; the drawing active then is not exported either.
#[test]
fn an_exporter_writes_nothing_once_its_drawing_is_reverted_or_closed() {
    for close in [false, true] {
        let dir = vectorcraft_testkit::TempDir::new("plugin-export-reverted-drawing");
        let (mut app, written) = two_drawings(&dir);
        let id = if close { "test.ui.closed" } else { "test.ui.reverted" };
        install_source_exporter(&mut app, id);
        open_source_exporter(&mut app, id);
        if close {
            app.run("document.activate", json!({"index": 1})).unwrap();
            app.run("file.close", json!({"index": 0})).unwrap();
            assert_eq!(app.session.documents().len(), 1, "the saved drawing closes without asking");
        } else {
            app.run("shape.rectangle", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
            app.run("file.revert", json!({"confirmed": true})).unwrap();
        }
        frame(&mut app);
        try_confirm(&mut app).unwrap();
        assert!(kind(&app).is_none());
        assert!(exported(&written).is_empty(), "nothing exported");
        let said = if close { "closed" } else { "reverted" };
        assert!(app.ui.status.contains(said), "{}", app.ui.status);
        app.run("plugin.remove", json!({"id": id})).unwrap();
    }
}

/// A plug-in that declines keeps nothing written and says why.
#[test]
fn a_declining_exporter_writes_nothing() {
    let (mut app, written) = app(1);
    install(&mut app, &tpl::returning(&tpl::exporter("test.ui.declines", r#"["tud"]"#, "{}"), r#"{"error":"Nothing to plot."}"#));
    app.run("file.exportAs", json!({"format": "plugin.test.ui.declines"})).unwrap();
    let r = try_confirm(&mut app);
    assert!(written.borrow().is_empty());
    let said = r.err().unwrap_or_default() + &app.ui.status;
    assert!(said.contains("Nothing to plot."), "{said}");
    app.run("plugin.remove", json!({"id": "test.ui.declines"})).unwrap();
}

/// Export options that aren't an object are none: the exporter is still told the file it writes,
/// by its name and extension (here the second it lists), not the document's name and its first.
#[test]
fn an_exporter_is_told_its_file_whatever_the_options_are() {
    let (mut app, written) = app_saving(1, |_| "/out/plotter-job.tnb".into());
    install(&mut app, &tpl::echo_exporter(&tpl::exporter("test.ui.nulloptions", r#"["tna","tnb"]"#, "{}"), Echo::Params));
    crate::io::export(&mut app, Some("plugin.test.ui.nulloptions"), None, &Value::Null).unwrap();
    answer_saves(&mut app);
    assert_eq!(names(&written), ["/out/plotter-job.tnb"]);
    let sent: Value = serde_json::from_slice(&written.borrow()[0].1).unwrap();
    assert_eq!(sent, json!({"_context": {"mode": "export", "extension": "tnb", "name": "plotter-job"}}));
    app.run("plugin.remove", json!({"id": "test.ui.nulloptions"})).unwrap();
}
