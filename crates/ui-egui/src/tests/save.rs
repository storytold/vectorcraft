//! The File menu around saving, in the app: Revert's confirmation, recent files, Show in Folder,
//! the single Document Color Mode entry and documents reopening at their saved view.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{Services, VectorcraftApp, dialogs, io, menus, palette, theme};

/// An app that reads and writes real files.
fn app() -> VectorcraftApp {
    let services = Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    VectorcraftApp::new(Session::new(), services)
}

/// A fresh folder for one test.
fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-ui-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rect(app: &mut VectorcraftApp) {
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.node_count()
}

/// One headless frame of the dialog layer with Enter pressed (OK).
fn press_ok(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn revert_asks_then_reloads_in_the_same_tab_keeping_every_zoom() {
    let d = dir("revert");
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    rect(&mut app);
    app.run("file.saveAs", json!({"path": path})).unwrap();
    assert!(!menus::enabled(&app, "file.revert"), "nothing changed since the save");
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.views[0].zoom = 2.0;
    app.views[1].zoom = 4.0;
    app.session.set_active(0);
    rect(&mut app);
    assert!(menus::enabled(&app, "file.revert"));
    let r = app.run("file.revert", json!({})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    assert_eq!(objects(&app), 3, "nothing happens before OK");
    press_ok(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!((objects(&app), app.session.active_index(), app.session.documents().len()), (2, Some(0), 2));
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!((app.views[0].zoom, app.views[1].zoom), (2.0, 4.0));
    // Cancel leaves the changes.
    rect(&mut app);
    app.run("file.revert", json!({})).unwrap();
    app.ui.dialog = None;
    assert_eq!(objects(&app), 3);
    // An untitled document can't revert.
    app.session.set_active(1);
    rect(&mut app);
    assert!(app.run("file.revert", json!({})).is_err() && app.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(d);
}

/// With the Artboard tool, Revert keeps the active artboard, and the tool starts on it.
#[test]
fn revert_with_the_artboard_tool_keeps_the_active_artboard() {
    let d = dir("revert-artboard");
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 2);
    rect(&mut app);
    app.run("file.revert", json!({"confirmed": true})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (2, json!([2])));
    let _ = std::fs::remove_dir_all(d);
}

/// With the Artboard tool, a `command.batch` with a Revert step keeps the active artboard, and the
/// tool starts on it, as Revert on its own does. When a later step pastes an artboard, the pasted
/// one becomes active, as Paste on its own makes it.
#[test]
fn a_batch_that_reverts_with_the_artboard_tool_keeps_the_active_artboard() {
    let d = dir("revert-batch");
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    let saved = objects(&app);
    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 2);
    rect(&mut app);
    let revert = json!({"command": "file.revert", "params": {"confirmed": true}});
    app.run("command.batch", json!({"commands": [revert]})).unwrap();
    assert_eq!(objects(&app), saved, "the batch reverted");
    let active = |app: &VectorcraftApp| (app.view().unwrap().artboard, app.session.tool_options()["selected"].clone());
    assert_eq!(active(&app), (2, json!([2])));
    app.run("edit.copy", json!({})).unwrap();
    rect(&mut app);
    app.run("command.batch", json!({"commands": [revert, {"command": "edit.paste", "params": {}}]})).unwrap();
    assert_eq!(active(&app), (3, json!([3])), "the pasted Artboard 4");
    let _ = std::fs::remove_dir_all(d);
}

/// With the Artboard tool, the Actions panel menu's Batch (`action.batch`) keeps the active
/// artboard, and the tool starts on it: over a folder, whose files it opens and closes, and over
/// data sets with a Revert step.
#[test]
fn batch_with_the_artboard_tool_keeps_the_active_artboard() {
    let d = dir("batch-artboard");
    let src = d.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let mut other = app();
    other.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    other.run("file.saveAs", json!({"path": src.join("b.vectorcraft").to_string_lossy()})).unwrap();
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    app.run("dataset.new", json!({"name": "a"})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    let saved = objects(&app);
    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 2);
    let active = |app: &VectorcraftApp| (app.view().unwrap().artboard, app.session.tool_options()["selected"].clone());
    let unlock = json!([{"command": "object.unlockAll", "params": {}}]);
    let r = app.run("action.batch", json!({"folder": src.to_string_lossy(), "destination": "saveAndClose", "commands": unlock})).unwrap();
    assert_eq!((r["count"].clone(), r["failed"].clone(), app.session.documents().len()), (json!(1), json!(0), 1), "{r}");
    assert_eq!(active(&app), (2, json!([2])), "over a folder");
    rect(&mut app);
    let revert = json!([{"command": "file.revert", "params": {"confirmed": true}}]);
    let r = app.run("action.batch", json!({"source": "datasets", "commands": revert})).unwrap();
    assert_eq!((r["count"].clone(), r["failed"].clone(), objects(&app)), (json!(1), json!(0), saved), "{r}");
    assert_eq!(active(&app), (2, json!([2])), "over data sets, reverting");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn new_from_template_takes_bytes_from_agents() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    let bytes = app.session.execute("document.serialize", &json!({"format": "template"})).unwrap();
    let r = app.run("file.newFromTemplate", json!({"name": "flyer.vctemplate", "dataBase64": bytes["dataBase64"]})).unwrap();
    assert!(r["title"].as_str().is_some_and(|t| t.starts_with("Untitled-")), "{r}");
    assert_eq!((app.session.documents().len(), app.views.len()), (2, 2));
    assert_eq!(app.session.active().unwrap().path, None);
}

#[test]
fn recent_files_show_as_many_as_the_preference_keeps() {
    let mut app = app();
    for i in 0..35 {
        io::note_recent(&mut app, &format!("/art/{i}.svg"));
    }
    let listed = |app: &VectorcraftApp| {
        menus::menu_entries(app).iter().filter(|e| e.command.as_deref().is_some_and(|c| c.starts_with("file.openRecent"))).count()
    };
    assert_eq!((io::recent_files(&app).len(), listed(&app)), (20, 20), "the default");
    assert_eq!(menus::dynamic_label(&app, "file.openRecent1", ""), "34.svg");
    app.session.prefs.recent_files_count = 30;
    assert_eq!((io::recent_files(&app).len(), listed(&app)), (30, 30));
    assert!(menus::enabled(&app, "file.openRecent30"));
    app.session.prefs.recent_files_count = 0;
    assert_eq!((io::recent_files(&app).len(), listed(&app)), (0, 0), "0 hides the list");
    assert!(menus::shown_state(&app, "file.openRecent1", &json!(null)).is_none(), "the slots leave the menus");
    assert!(app.run("file.openRecent1", json!({})).is_err());
    app.session.prefs.recent_files_count = 5;
    assert_eq!(io::recent_files(&app)[0], "/art/34.svg", "hiding them didn't forget them");
}

#[test]
fn show_in_folder_needs_a_saved_document_and_a_file_manager() {
    let shown = Rc::new(RefCell::new(vec![]));
    let s = shown.clone();
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    assert!(!menus::enabled(&app, "file.reveal"));
    app.session.doc_mut().unwrap().path = Some("/art/a.vectorcraft".into());
    assert!(!menus::enabled(&app, "file.reveal"), "no file manager (web)");
    app.services.reveal = Some(Box::new(move |p: &str| {
        s.borrow_mut().push(p.to_string());
        Ok(())
    }));
    assert!(menus::enabled(&app, "file.reveal"));
    assert_eq!(app.run("file.reveal", json!({})).unwrap()["path"], "/art/a.vectorcraft");
    assert_eq!(*shown.borrow(), ["/art/a.vectorcraft"]);
}

#[test]
fn the_palette_lists_document_color_mode_once() {
    let items = palette::items();
    let ids: Vec<&str> = items.iter().map(|(_, id, _)| id.as_str()).filter(|id| id.to_ascii_lowercase().contains("documentcolormode")).collect();
    assert_eq!(ids, ["file.documentColorMode"]);
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    let entries = menus::menu_entries(&app);
    let modes: Vec<&str> =
        entries.iter().filter(|e| e.path.last().is_some_and(|p| p == "Document Color Mode")).filter_map(|e| e.command.as_deref()).collect();
    assert_eq!(modes, ["file.documentColorMode", "file.documentColorMode"], "CMYK and RGB run the one command");
}

#[test]
fn a_document_reopens_at_its_saved_view() {
    let d = dir("view");
    let path = d.join("v.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    {
        let v = app.view_mut().unwrap();
        (v.zoom, v.center, v.rotation, v.fitted) = (3.0, Point::new(70.0, 40.0), 30.0, true);
    }
    app.run("file.saveAs", json!({"path": path})).unwrap();
    // Moving around afterwards isn't an edit.
    app.view_mut().unwrap().zoom = 0.5;
    assert!(!app.session.active().unwrap().is_dirty());
    io::open_path(&mut app, &path).unwrap();
    let v = *app.view().unwrap();
    assert_eq!((v.zoom, v.center, v.rotation, v.fitted), (3.0, Point::new(70.0, 40.0), 30.0, true));
    // A document without a saved view is fitted on first display.
    app.run("file.new", json!({})).unwrap();
    assert!(!app.view().unwrap().fitted);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn replacing_a_file_that_opening_left_things_out_of_asks_first() {
    let d = dir("lossy");
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    rect(&mut app);
    app.run("file.saveAs", json!({"path": path})).unwrap();
    // As if reading the file had left out text it can't read yet.
    let st = app.session.doc_mut().unwrap();
    st.imported_from = Some(path.clone());
    st.import_losses = vec!["hidden type".into()];
    let saved = std::fs::read(&path).unwrap();
    rect(&mut app);
    let r = app.run("file.save", json!({})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    let dialog = app.ui.dialog.as_ref().unwrap();
    assert!(dialog.str("detail").contains("“hidden type”"), "{:?}", dialog.fields);
    assert_eq!(std::fs::read(&path).unwrap(), saved, "nothing is written before OK");
    // Export As over it asks too; Cancel leaves the file.
    app.ui.dialog = None;
    assert_eq!(app.run("file.exportAs", json!({"path": path, "format": "svg"})).unwrap()["pending"], dialogs::confirm::KIND);
    app.ui.dialog = None;
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    // Another name doesn't ask.
    let copy = d.join("b.svg").to_string_lossy().to_string();
    assert!(app.run("file.exportAs", json!({"path": copy, "format": "svg"})).unwrap().get("pending").is_none());
    crate::background::wait_all(&mut app);
    assert!(app.ui.dialog.is_none());
    // OK replaces it.
    app.run("file.save", json!({})).unwrap();
    press_ok(&mut app);
    crate::background::wait_all(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_ne!(std::fs::read(&path).unwrap(), saved);
    assert!(!app.session.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn native_saves_stream_into_the_file() {
    let d = dir("stream");
    let streamed = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let log = streamed.clone();
    let mut app = app();
    app.services.stream_shared = Some(std::sync::Arc::new(move |p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
        log.lock().unwrap().push(p.to_string());
        let mut f = std::fs::File::create(p).map_err(|e| e.to_string())?;
        fill(&mut f).map_err(|e| e.to_string())
    }));
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    rect(&mut app);
    let native = d.join("a.vectorcraft").to_string_lossy().to_string();
    let r = app.run("file.saveAs", json!({"path": native, "compress": true})).unwrap();
    let bytes = std::fs::read(&native).unwrap();
    assert_eq!(r["bytes"], bytes.len(), "{r}");
    assert!(vectorcraft_format::is_compressed(&bytes));
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().node_count(), objects(&app));
    assert!(!app.session.doc().unwrap().is_dirty(), "the file is the document's own");
    assert_eq!(*streamed.lock().unwrap(), [native]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn file_save_writes_opened_eps_back_without_another_save_as_dialog() {
    let d = dir("eps-original-format");
    let path = d.join("Artwork.eps").to_string_lossy().to_string();
    std::fs::write(&path, b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 100\n0 0 1 setrgbcolor 30 20 40 30 rectfill\nshowpage\n%%EOF\n").unwrap();
    let mut app = app();
    app.run("document.open", json!({"path":path})).unwrap();
    rect(&mut app);
    app.run("file.save", json!({})).unwrap();
    crate::background::wait_all(&mut app);
    assert!(app.ui.dialog.is_none(), "ordinary EPS Save does not require another format dialog");
    let bytes = std::fs::read(&path).unwrap();
    assert!(vectorcraft_eps::sections(&bytes).unwrap().0.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    assert!(vectorcraft_eps::native(&bytes).is_some());
    assert_eq!(app.session.active().unwrap().format, "eps");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    std::fs::remove_dir_all(d).unwrap();
}
