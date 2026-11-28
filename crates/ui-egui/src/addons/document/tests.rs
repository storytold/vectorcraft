use std::path::PathBuf;

use serde_json::json;
use vectorcraft_engine::Session;

use super::{BATCH_TRACE, EXPORT_ALL, SAVE_COPY};
use crate::tests::picks::{poll, scripted};
use crate::{VectorcraftApp, menus};
use craft::host::PickKind;

/// Whether the newest pick asked for is a folder.
fn asked_for_folder(picker: &crate::tests::picks::Picker) -> bool {
    picker.requests().last().map(|r| r.kind) == Some(PickKind::Folder)
}

/// A new, empty folder of its own for test `name`.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-ui-document-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// Click the Document Tools item `label`.
fn click(app: &mut VectorcraftApp, label: &str) {
    let entries = menus::menu_entries(app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Document Tools"] && e.label == label).expect(label);
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &id, p);
}

fn files(dir: &PathBuf) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()
}

#[test]
fn save_a_timestamped_copy_writes_beside_the_document() {
    let dir = folder("beside");
    let mut app = app();
    app.run("document.save", json!({"path": dir.join("card.vectorcraft")})).unwrap();
    click(&mut app, "Save a Timestamped Copy");
    let names = files(&dir);
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("card_20") && n.ends_with(".vectorcraft")), "{names:?}");
    assert!(app.ui.status.starts_with("Saved "), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `.ai` timestamped copy, and a `.ai` copy Export All Open Documents writes, name the view's
/// active artboard as their active one.
#[test]
fn ai_copies_name_the_views_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": 3})).unwrap();
    app.sync_views();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let opts = vectorcraft_engine::cmd::fileio::LoadOptions { native: false, ..Default::default() };
    for (i, (cmd, artboard)) in [(SAVE_COPY, 2), (EXPORT_ALL, 1)].into_iter().enumerate() {
        let dir = folder(&format!("active-artboard-{i}"));
        if let Some(v) = app.view_mut() {
            v.artboard = artboard;
        }
        app.run(cmd, json!({"folder": dir, "format": "ai"})).unwrap();
        let names = files(&dir);
        assert_eq!(names.len(), 1, "{cmd}: {names:?}");
        let bytes = std::fs::read(dir.join(&names[0])).unwrap();
        let l = vectorcraft_engine::cmd::fileio::load_with("a.ai", &bytes, &opts).unwrap();
        assert!(l.editing_data && !l.restored, "{cmd}: {:?}", l.warnings);
        assert_eq!(l.doc.opened_artboard.and_then(|id| l.doc.artboards.iter().position(|a| a.id == id)), Some(artboard), "{cmd}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Export All Open Documents as `.ai` names each document's own active artboard, also for a
/// document whose artboard changed in its view after the last command and that is not active.
#[test]
fn export_all_names_each_documents_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for name in ["first", "second"] {
        app.run("file.new", json!({"width": 60, "height": 40, "artboards": 3, "name": name})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    }
    app.sync_views();
    // Choosing a tab and an artboard runs no command.
    assert!(app.session.set_active(0));
    if let Some(v) = app.view_mut() {
        v.artboard = 2;
    }
    assert!(app.session.set_active(1));
    if let Some(v) = app.view_mut() {
        v.artboard = 1;
    }
    let dir = folder("export-all-active-artboards");
    let r = app.run(EXPORT_ALL, json!({"folder": dir, "format": "ai"})).unwrap();
    let opts = vectorcraft_engine::cmd::fileio::LoadOptions { native: false, ..Default::default() };
    let written = r["files"].as_array().unwrap();
    assert_eq!(written.len(), 2, "{r}");
    for (f, artboard) in written.iter().zip([2, 1]) {
        let bytes = std::fs::read(f["path"].as_str().unwrap()).unwrap();
        let l = vectorcraft_engine::cmd::fileio::load_with("a.ai", &bytes, &opts).unwrap();
        assert!(l.editing_data && !l.restored, "{:?}", l.warnings);
        assert_eq!(l.doc.opened_artboard.and_then(|id| l.doc.artboards.iter().position(|a| a.id == id)), Some(artboard), "{}", f["title"]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unsaved_document_asks_for_a_folder() {
    let dir = folder("picked");
    let mut app = app();
    let picker = scripted(&mut app);
    let ctx = crate::testing::ui_context();
    click(&mut app, "Save a Timestamped Copy");
    assert!(asked_for_folder(&picker));
    picker.answer(&[dir.to_str().unwrap()]);
    poll(&mut app, &ctx);
    assert_eq!(files(&dir).len(), 1);
    assert!(app.session.doc().unwrap().path.is_none(), "the document stays unsaved");
    // No folder picked: nothing written.
    let asked = picker.requests().len();
    let r = app.run(SAVE_COPY, json!({})).unwrap();
    assert_eq!(r["pending"], "fileDialog");
    assert_eq!(picker.requests().len(), asked + 1);
    picker.cancel();
    poll(&mut app, &ctx);
    assert_eq!(files(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_symbols_from_layers_and_convert_layers_to_symbols() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    app.run("layer.new", json!({"name": "Badges"})).unwrap();
    app.run("shape.ellipse", json!({"x": 100, "y": 10, "width": 40, "height": 40})).unwrap();
    let symbols = |app: &VectorcraftApp| app.session.doc().unwrap().doc.symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let before = symbols(&app).len();
    click(&mut app, "New Symbols from Layers");
    assert_eq!(symbols(&app).len(), before + 2);
    assert!(symbols(&app).contains(&"Badges".to_string()));
    app.run("edit.undo", json!({})).unwrap();
    click(&mut app, "Convert Layers to Symbols");
    let doc = &app.session.doc().unwrap().doc;
    assert!(
        doc.layers
            .iter()
            .all(|l| matches!(l.children().unwrap()[..], [ref one] if matches!(one.kind, vectorcraft_doc::NodeKind::SymbolInstance { .. })))
    );
}

/// #4920: Export All Open Documents as PDF asks for a folder and writes one PDF per open document,
/// named after each, without replacing a file there, and leaves every document as it was.
#[test]
fn export_all_open_documents_writes_each_to_the_folder_picked() {
    let dir = folder("export-all");
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("document.save", json!({"path": dir.join("card.vectorcraft")})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 100, "name": "card"})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let untitled = app.session.doc().unwrap().doc.title.clone();
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join(format!("{untitled}.pdf")), b"keep").unwrap();
    let picker = scripted(&mut app);
    let ctx = crate::testing::ui_context();
    let active = app.session.active_index();
    click(&mut app, "Export All Open Documents as PDF");
    assert!(asked_for_folder(&picker));
    picker.answer(&[out.to_str().unwrap()]);
    poll(&mut app, &ctx);
    let mut names = files(&out);
    names.sort();
    let mut want = vec!["card.pdf".to_string(), "card-2.pdf".into(), format!("{untitled}.pdf"), format!("{untitled}-2.pdf")];
    want.sort();
    assert_eq!(names, want, "one per document, the taken names numbered");
    assert_eq!(std::fs::read(out.join(format!("{untitled}.pdf"))).unwrap(), b"keep", "no file is replaced");
    assert!(std::fs::read(out.join("card.pdf")).unwrap().starts_with(b"%PDF"));
    assert_eq!(app.session.active_index(), active, "the active document stays active");
    assert!(
        app.session.documents().iter().all(|d| d.path.as_ref().is_none_or(|p| p.ends_with("card.vectorcraft"))),
        "no document takes a copy's path"
    );
    assert!(app.ui.status.starts_with("Exported 3 documents"), "{}", app.ui.status);
    // Cancelled: nothing written.
    let asked = picker.requests().len();
    let r = app.run(EXPORT_ALL, json!({})).unwrap();
    assert_eq!(r["pending"], "fileDialog");
    assert_eq!(picker.requests().len(), asked + 1);
    picker.cancel();
    poll(&mut app, &ctx);
    assert_eq!(files(&out).len(), 4);
    // The engine command refuses a missing folder and an unknown format.
    assert!(app.session.execute(super::EXPORT_ALL, &json!({})).is_err());
    assert!(app.session.execute(super::EXPORT_ALL, &json!({"folder": out, "format": "doc"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// #4921: Batch Trace Folder from Object › Plug-ins › Document Tools asks for the folder of images,
/// then the destination, traces each image there and reports the count (a status the catalogs
/// translate); cancelling either does nothing.
#[test]
fn batch_trace_folder_asks_for_both_folders() {
    let dir = folder("batch-trace");
    let (src, out) = (dir.join("in"), dir.join("out"));
    std::fs::create_dir_all(&src).unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 255])).save(src.join("a.png")).unwrap();
    std::fs::write(src.join("b.png"), b"not a png").unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let picker = scripted(&mut app);
    let ctx = crate::testing::ui_context();
    click(&mut app, "Batch Trace Folder");
    // The destination is asked only after the source is answered.
    assert_eq!(picker.requests().len(), 1);
    assert!(asked_for_folder(&picker));
    picker.answer(&[src.to_str().unwrap()]);
    poll(&mut app, &ctx);
    assert_eq!(picker.requests().len(), 2);
    assert!(asked_for_folder(&picker));
    picker.answer(&[out.to_str().unwrap()]);
    poll(&mut app, &ctx);
    assert!(out.join("a.vectorcraft").exists(), "{}", app.ui.status);
    assert!(!out.join("b.vectorcraft").exists());
    assert!(app.session.documents().is_empty(), "the images' documents are closed");
    let asked = picker.requests().len();
    let r = app.run(BATCH_TRACE, json!({})).unwrap();
    assert_eq!(r["pending"], "fileDialog");
    assert_eq!(picker.requests().len(), asked + 1);
    picker.cancel();
    poll(&mut app, &ctx);
    assert_eq!(picker.requests().len(), asked + 1, "declining the source asks nothing more");
    let _ = std::fs::remove_dir_all(&dir);
}
