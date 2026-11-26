//! The File menu around saving, in the app: Revert's confirmation, Show in Folder and documents
//! reopening at their saved view.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use super::picks::{self, answer_dialog, answer_replace, cancel_dialog, open_dialogs, scripted};
use crate::dialogs::confirm;
use crate::{Services, VectorcraftApp, io, menus};

/// An app that reads and writes real files.
fn app() -> VectorcraftApp {
    let services = Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    crate::testing::windowed(VectorcraftApp::new(Session::new(), services))
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

/// Press OK (or Cancel) on the dialog or question open on top.
fn answer_top(app: &mut VectorcraftApp, ok: bool) {
    let id = open_dialogs(app).pop().map(|d| d.id).expect("a dialog is open");
    if ok { answer_dialog(app, id, &[], true) } else { cancel_dialog(app, id) }
}

fn poll(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    picks::poll(app, &ctx);
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
    assert_eq!(r["pending"], confirm::KIND);
    assert!(confirm::open_question(&app).is_some());
    assert_eq!(objects(&app), 3, "nothing happens before OK");
    answer_top(&mut app, true);
    assert!(!app.any_dialog_open());
    assert_eq!((objects(&app), app.session.active_index(), app.session.documents().len()), (2, Some(0), 2));
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!((app.views[0].zoom, app.views[1].zoom), (2.0, 4.0));
    // Cancel leaves the changes.
    rect(&mut app);
    app.run("file.revert", json!({})).unwrap();
    answer_top(&mut app, false);
    assert_eq!(objects(&app), 3);
    // An untitled document can't revert.
    app.session.set_active(1);
    rect(&mut app);
    assert!(app.run("file.revert", json!({})).is_err() && !app.any_dialog_open());
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
    // Reopened: an open file would only come to the front (#3455).
    app.run("file.close", json!({})).unwrap();
    io::open_path(&mut app, &path).unwrap();
    let v = *app.view().unwrap();
    assert_eq!((v.zoom, v.center, v.rotation, v.fitted), (3.0, Point::new(70.0, 40.0), 30.0, true));
    // A document without a saved view is fitted on first display.
    app.run("file.new", json!({})).unwrap();
    assert!(!app.view().unwrap().fitted);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_saved_ai_file_reopens_on_the_artboard_active_when_saved() {
    let d = dir("ai-artboard");
    let path = d.join("a.ai").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    app.sync_views();
    app.view_mut().unwrap().artboard = 1;
    app.run("file.saveAs", json!({"path": path})).unwrap();
    app.run("file.close", json!({})).unwrap();
    let r = io::open_path(&mut app, &path).unwrap();
    assert_eq!(r["restored"], json!(true), "{r}");
    assert_eq!(app.view().map(|v| v.artboard), Some(1), "{r}");
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
    assert_eq!(r["pending"], confirm::KIND);
    let (_, detail, _) = confirm::open_question(&app).expect("Save asks first");
    assert!(detail.contains("“hidden type”"), "{detail}");
    assert_eq!(std::fs::read(&path).unwrap(), saved, "nothing is written before OK");
    // Export As over it asks too; Cancel leaves the file.
    answer_top(&mut app, false);
    assert_eq!(app.run("file.exportAs", json!({"path": path, "format": "svg"})).unwrap()["pending"], confirm::KIND);
    answer_top(&mut app, false);
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    // So does a PDF written by Save PDF's OK, and the question stays open when Save PDF closes.
    assert_eq!(app.run("file.export.pdf", json!({"path": path})).unwrap()["pending"], confirm::KIND);
    answer_top(&mut app, false);
    for artboards in [true, false] {
        app.run("ui.savePdfDialog", json!({"path": path, "useArtboards": artboards})).unwrap();
        answer_top(&mut app, true);
        assert!(confirm::open_question(&app).is_some(), "useArtboards {artboards}");
        answer_top(&mut app, false);
    }
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    // Also when File › Save as PDF… leaves the file to the save panel and the panel picks the PDF
    // the document was opened from.
    let pdf = d.join("a.pdf").to_string_lossy().to_string();
    std::fs::write(&pdf, b"%PDF-1.7 as opened").unwrap();
    app.session.doc_mut().unwrap().imported_from = Some(pdf.clone());
    let picker = scripted(&mut app);
    for artboards in [true, false] {
        app.run("ui.savePdfDialog", json!({"useArtboards": artboards})).unwrap();
        answer_top(&mut app, true);
        picker.answer_saved(&pdf, None);
        poll(&mut app);
        let asked = confirm::open_question(&app);
        assert!(asked.is_some_and(|(_, detail, _)| detail.contains("“hidden type”")), "picked, useArtboards {artboards}");
        answer_top(&mut app, false);
    }
    assert_eq!(std::fs::read(&pdf).unwrap(), b"%PDF-1.7 as opened");
    app.session.doc_mut().unwrap().imported_from = Some(path.clone());
    // Another name doesn't ask.
    let copy = d.join("b.svg").to_string_lossy().to_string();
    assert!(app.run("file.exportAs", json!({"path": copy, "format": "svg"})).unwrap().get("pending").is_none());
    crate::background::wait_all(&mut app);
    assert!(!app.any_dialog_open());
    // OK replaces it.
    app.run("file.save", json!({})).unwrap();
    answer_top(&mut app, true);
    crate::background::wait_all(&mut app);
    assert!(!app.any_dialog_open());
    assert_ne!(std::fs::read(&path).unwrap(), saved);
    assert!(!app.session.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_save_does_not_replace_the_file_read_from_through_a_file_written_beside_the_path() {
    let d = dir("lossy-beside");
    let path = d.join("Art.eps").to_string_lossy().to_string();
    let mut app = app();
    checks_files_there(&mut app);
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    rect(&mut app);
    let options = json!({"useArtboards": true, "previewFormat": "none", "thumbnails": false});
    let _ = app.run("file.saveAs", json!({"path": path, "options": options}));
    crate::background::wait_all(&mut app);
    let siblings = artboard_files(&d);
    assert_eq!(siblings.len(), 2, "{siblings:?}");
    let sibling = siblings[0].clone();
    let saved = std::fs::read(&sibling).unwrap();
    std::fs::remove_file(&path).unwrap();
    let st = app.session.doc_mut().unwrap();
    st.imported_from = Some(sibling.clone());
    st.import_losses = vec!["hidden type".into()];
    rect(&mut app);
    let _ = app.run("file.saveAs", json!({"path": path, "options": options}));
    crate::background::wait_all(&mut app);
    // The document's last save wrote the files beside the path, so the save does not ask whether
    // to replace them: it is refused.
    assert!(confirm::open_replace_files(&app).is_none());
    assert_eq!(std::fs::read(&sibling).unwrap(), saved);
    assert!(!std::path::Path::new(&path).exists(), "no file of the set is written when one is refused");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn file_save_does_not_replace_the_file_read_from_through_a_file_written_beside_the_path() {
    let d = dir("lossy-beside-save");
    let path = d.join("Art.eps").to_string_lossy().to_string();
    let mut app = app();
    checks_files_there(&mut app);
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    rect(&mut app);
    let options = json!({"useArtboards": true, "previewFormat": "none", "thumbnails": false});
    let _ = app.run("file.saveAs", json!({"path": path, "options": options}));
    crate::background::wait_all(&mut app);
    let siblings = artboard_files(&d);
    assert_eq!(siblings.len(), 2, "{siblings:?}");
    let sibling = siblings[0].clone();
    let saved = std::fs::read(&sibling).unwrap();
    std::fs::remove_file(&path).unwrap();
    let st = app.session.doc_mut().unwrap();
    st.imported_from = Some(sibling.clone());
    st.import_losses = vec!["hidden type".into()];
    rect(&mut app);
    // File > Save to the document's own file reuses its options, so it writes the artboard files
    // again; the one the document was read from is refused, as on Save As.
    let _ = app.run("file.save", json!({}));
    crate::background::wait_all(&mut app);
    assert!(confirm::open_replace_files(&app).is_none());
    assert_eq!(std::fs::read(&sibling).unwrap(), saved);
    assert!(!std::path::Path::new(&path).exists(), "no file of the set is written when one is refused");
    assert!(app.session.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(d);
}

/// The app checks for files that are there, as the desktop app does.
fn checks_files_there(app: &mut VectorcraftApp) {
    app.services.exists = Some(std::sync::Arc::new(|p: &str| std::path::Path::new(p).exists()));
}

/// The files in `d` that an EPS save of `Art.eps` with Use Artboards wrote beside it, sorted.
fn artboard_files(d: &std::path::Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Art_")))
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    files.sort();
    files
}

#[test]
fn an_eps_save_with_use_artboards_asks_before_replacing_an_artboard_file_that_is_there() {
    let d = dir("eps-artboards-there");
    let path = d.join("Art.eps").to_string_lossy().to_string();
    let options = json!({"useArtboards": true, "previewFormat": "none", "thumbnails": false});
    let save_as = |app: &mut VectorcraftApp| {
        let r = app.run("file.saveAs", json!({"path": path, "options": options}));
        crate::background::wait_all(app);
        r
    };
    // Another document's save leaves files with the names of the artboard files.
    let mut other = app();
    other.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let _ = save_as(&mut other);
    let there = artboard_files(&d);
    assert_eq!(there.len(), 2, "{there:?}");
    std::fs::remove_file(&path).unwrap();
    for f in &there {
        std::fs::write(f, b"other").unwrap();
    }
    let mut app = app();
    checks_files_there(&mut app);
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    rect(&mut app);
    assert_eq!(save_as(&mut app).unwrap()["pending"], confirm::REPLACE_FILES);
    assert_eq!(confirm::open_replace_files(&app), Some(there.clone()));
    assert!(!std::path::Path::new(&path).exists(), "nothing is written while it asks");
    assert!(there.iter().all(|f| std::fs::read(f).unwrap() == b"other"));
    // Cancel writes nothing.
    answer_replace(&mut app, false);
    assert!(!std::path::Path::new(&path).exists());
    assert!(there.iter().all(|f| std::fs::read(f).unwrap() == b"other"));
    // OK writes every file.
    let _ = save_as(&mut app);
    answer_replace(&mut app, true);
    crate::background::wait_all(&mut app);
    assert!(std::path::Path::new(&path).exists());
    assert!(there.iter().all(|f| std::fs::read(f).unwrap().starts_with(b"%!PS")));
    // The document's next save writes them again without asking.
    rect(&mut app);
    let _ = app.run("file.save", json!({}));
    crate::background::wait_all(&mut app);
    assert!(!app.any_dialog_open());
    assert!(!app.session.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(d);
}

/// A background save that automation confined to a folder started asks about the files it would
/// replace once its result arrives, outside the request; OK still runs the save again under that
/// folder and as automation.
#[test]
fn a_background_save_asks_about_replacing_files_under_the_access_of_the_request_that_started_it() {
    let d = dir("eps-artboards-confined");
    let path = d.join("Art.eps").to_string_lossy().to_string();
    let options = json!({"useArtboards": true, "previewFormat": "none", "thumbnails": false});
    let mut other = app();
    other.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let _ = other.run("file.saveAs", json!({"path": path, "options": options}));
    let there = artboard_files(&d);
    assert_eq!(there.len(), 2, "{there:?}");
    std::fs::remove_file(&path).unwrap();
    let mut app = app();
    checks_files_there(&mut app);
    app.services.write_shared = Some(std::sync::Arc::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    rect(&mut app);
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[d.as_path()], &[d.as_path()]).unwrap();
    let r = vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        crate::file_asks::as_automated(&mut app, true, |app| app.run("file.saveAs", json!({"path": path, "options": options})))
    })
    .unwrap();
    assert_eq!(r["background"], true, "{r}");
    // The save's result is applied outside the request, as the next frame does.
    crate::background::wait_all(&mut app);
    assert_eq!(confirm::open_replace_files(&app), Some(there.clone()));
    assert_eq!(confirm::open_replace_files_access(&app), Some((true, true)), "(confined, as automation)");
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
    assert!(!app.any_dialog_open(), "ordinary EPS Save does not require another format dialog");
    let bytes = std::fs::read(&path).unwrap();
    assert!(vectorcraft_eps::sections(&bytes).unwrap().0.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    assert!(vectorcraft_eps::native(&bytes).is_some());
    assert_eq!(app.session.active().unwrap().format, "eps");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn revert_confirmation_never_discards_another_drawings_changes() {
    let d = dir("revert-drawing-switch");
    let mut app = app();
    for name in ["first.vectorcraft", "second.vectorcraft"] {
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let path = d.join(name).to_string_lossy().to_string();
        app.run("file.saveAs", json!({"path": path})).unwrap();
        rect(&mut app);
    }
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("file.revert", json!({})).unwrap();
    assert!(confirm::open_question(&app).unwrap().0.contains("first"));
    app.run("document.activate", json!({"index": 1})).unwrap();
    let count = objects(&app);
    answer_top(&mut app, true);
    let result = (objects(&app), app.session.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(d);
    assert_eq!(result, (count, true), "the second drawing must retain its unsaved edits");
}
