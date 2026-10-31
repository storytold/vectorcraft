//! File → Package…: the dialog, saving first, the folder or the downloaded zip, Show Package.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access::{Access, AutomationRoots};

use super::*;
use crate::background;
use crate::tests_background::{Files, Gate, written};

/// A fresh folder for one test (removed when dropped).
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-ui-package-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn frame(app: &mut VectorcraftApp) -> String {
    crate::tests_labels::painted_text(app, |app, ui| show(app, ui.ctx()))
}

/// An app writing through `std::fs`, with a saved document holding a placed linked image.
fn saved(dir: &Folder) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.services.write = Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let mut png = vec![];
    image::RgbaImage::from_pixel(300, 300, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.path("pic.png"), &png).unwrap();
    app.run("file.place", json!({"path": dir.path("pic.png")})).unwrap();
    app.run("document.save", json!({"path": dir.path("card.vectorcraft")})).unwrap();
    app
}

#[test]
fn an_unsaved_document_is_offered_save_as_first() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert!(!crate::menus::enabled(&app, "ui.packageDialog"), "no document");
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("ui.packageDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("__command")), (confirm::KIND, "file.saveAs".to_string()));
}

#[test]
fn package_writes_the_folder_then_offers_to_show_it() {
    let dir = Folder::new("folder");
    let mut app = saved(&dir);
    app.run("ui.packageDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("name"), d.bool("copyFonts")), ("package", "card Folder".to_string(), true));
    assert_eq!(std::path::Path::new(&d.str("folder")), dir.0.as_path(), "next to the document");
    let text = frame(&mut app);
    assert!(text.contains("Collect Links in a Separate Folder") && text.contains("Folder Name:"), "{text}");
    let out = dir.path("out");
    std::fs::create_dir_all(&out).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("folder".into(), json!(out));
    // Unsaved changes are saved first.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let r = confirm(&mut app).unwrap();
    assert!(!app.session.active().unwrap().is_dirty());
    let root = std::path::Path::new(&out).join("card Folder");
    assert!(root.join("card.vectorcraft").is_file() && root.join("Links/pic.png").is_file(), "{r}");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("__command")), (confirm::KIND, "file.showPackage".to_string()));
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let o = opened.clone();
    app.services.open_file = Some(Box::new(move |p: &str| {
        o.borrow_mut().push(p.to_string());
        Ok(())
    }));
    confirm(&mut app).unwrap();
    assert_eq!(opened.borrow().first().map(std::path::PathBuf::from), Some(root));
}

/// [`saved`] with a thread-safe writer, the background thread's, that records the files it is
/// given once `gate` opens.
fn saved_with_worker(dir: &Folder, gate: &Gate) -> (VectorcraftApp, Files) {
    let mut app = saved(dir);
    let (files, g) = (Files::default(), gate.clone());
    let recorded = files.clone();
    app.services.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| {
        g.wait();
        recorded.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    (app, files)
}

/// Open the Package dialog for a package in `out`.
fn package_dialog(app: &mut VectorcraftApp, out: &str) {
    app.run("ui.packageDialog", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("folder".into(), json!(out));
}

#[test]
fn package_is_written_in_the_background_as_it_is_at_once() {
    let dir = Folder::new("background");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let out = dir.path("out");
    let root = std::path::Path::new(&out).join("card Folder");
    // With Background Export off, the package is written at once through the UI thread's writer.
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": false}})).unwrap();
    package_dialog(&mut app, &out);
    let at_once = confirm(&mut app).unwrap();
    let want: Vec<(String, Vec<u8>)> = at_once["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| root.join(f.as_str().unwrap()))
        .map(|p| (p.to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
        .collect();
    assert_eq!(want.len(), 3, "the document, its linked file and the report: {at_once}");
    let (status, detail) = (app.ui.status.clone(), app.ui.dialog.as_ref().unwrap().str("detail"));
    std::fs::remove_dir_all(&root).unwrap();
    app.ui.dialog = None;
    // With Background Export on, the dialog closes at once, and the same files are written later.
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": true}})).unwrap();
    package_dialog(&mut app, &out);
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["background"], true, "{r}");
    assert!(app.ui.dialog.is_none(), "closed");
    assert_eq!(app.ui.status, "Packaging card Folder…");
    assert!(written(&files).is_empty(), "nothing written yet");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(*files.lock().unwrap(), want);
    assert_eq!(app.ui.status, status);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("__command"), d.str("detail")), ("file.showPackage".to_string(), detail));
}

#[test]
fn edits_after_ok_do_not_reach_the_package() {
    let dir = Folder::new("snapshot");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let nodes = app.session.active().unwrap().doc.node_count();
    package_dialog(&mut app, &dir.path("out"));
    assert_eq!(confirm(&mut app).unwrap()["background"], true);
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    // A dialog opened meanwhile stays open; the status bar reports the package.
    app.run("ui.packageDialog", json!({})).unwrap();
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("package"));
    assert!(app.ui.status.starts_with("Packaged 3 file(s) in "), "{}", app.ui.status);
    let files = files.lock().unwrap();
    let (_, bytes) = files.iter().find(|(p, _)| p.ends_with("card.vectorcraft")).unwrap();
    assert_eq!(vectorcraft_format::load(bytes).unwrap().node_count(), nodes, "the document as it was at OK");
    assert!(files.iter().any(|(p, _)| p.ends_with("pic.png")), "its linked file");
}

#[test]
fn a_dirty_documents_save_is_written_before_its_package() {
    let dir = Folder::new("dirty");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let (doc, out) = (dir.path("card.vectorcraft"), dir.path("out"));
    // With Background Export on, the save and the package both run in the background, and the
    // worker writes the save first.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    package_dialog(&mut app, &out);
    // A thread opens the gate after 10 s at the latest, so a confirm that waits for the save ends.
    let opener = gate.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(10));
        opener.open();
    });
    assert_eq!(confirm(&mut app).unwrap()["background"], true);
    assert_eq!(app.background.jobs.len(), 2, "the save and the package");
    gate.open();
    background::wait_all(&mut app);
    let paths = written(&files);
    assert_eq!(paths.first(), Some(&doc), "{paths:?}");
    assert_eq!(paths.len(), 4, "{paths:?}");
    assert!(!app.session.active().unwrap().is_dirty());
    // With Background Export off, the package is made at once and waits for the background save.
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": false}})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    package_dialog(&mut app, &out);
    let opener = gate.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        opener.open();
    });
    confirm(&mut app).unwrap();
    release.join().unwrap();
    assert_eq!(written(&files), [doc]);
    assert!(app.background.jobs.is_empty() && !app.session.active().unwrap().is_dirty());
    assert!(app.ui.status.starts_with("Packaged 3 file(s) in "), "the package's status came last: {}", app.ui.status);
}

#[test]
fn a_package_made_at_once_waits_for_one_in_the_background() {
    let dir = Folder::new("twice");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let out = dir.path("out");
    package_dialog(&mut app, &out);
    assert_eq!(confirm(&mut app).unwrap()["background"], true);
    // With Background Export turned off, the same package made again is written after the one in
    // the background.
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": false}})).unwrap();
    package_dialog(&mut app, &out);
    let opener = gate.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        opener.open();
    });
    confirm(&mut app).unwrap();
    assert!(app.background.jobs.is_empty(), "the package in the background was written first");
    release.join().unwrap();
    assert_eq!(written(&files).len(), 3);
    assert!(std::path::Path::new(&out).join("card Folder/card.vectorcraft").is_file());
}

#[test]
fn the_web_downloads_a_zip() {
    let dir = Folder::new("web");
    let mut app = saved(&dir);
    let got: Rc<RefCell<Vec<(String, usize)>>> = Rc::default();
    let g = got.clone();
    app.services.download = Some(Box::new(move |name: &str, b: &[u8]| {
        g.borrow_mut().push((name.to_string(), b.len()));
        Ok(())
    }));
    app.run("ui.packageDialog", json!({})).unwrap();
    assert!(!frame(&mut app).contains("Location:"), "no folder to choose");
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(app.ui.status.contains("Packaged") && app.ui.status.contains("file(s)"), "{}", app.ui.status);
    let got = got.borrow();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "card Folder.zip");
    assert!(got[0].1 > 100);
}

#[test]
fn fonts_left_out_of_the_package_are_named() {
    let dir = Folder::new("fonts");
    let mut app = saved(&dir);
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Kept"})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 80, "text": "Gone", "font": "No Such Font Family"})).unwrap();
    app.run("ui.packageDialog", json!({})).unwrap();
    let out = dir.path("out");
    std::fs::create_dir_all(&out).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("folder".into(), json!(out));
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["skippedFonts"], json!([{"font": "No Such Font Family Regular", "reason": "not available on this computer"}]));
    let gone = "fonts not copied: No Such Font Family Regular";
    assert!(app.ui.status.contains(gone), "{}", app.ui.status);
    let detail = app.ui.dialog.as_ref().unwrap().str("detail");
    assert!(detail.contains(gone) && !detail.contains("Source Sans 3"), "{detail}");
    // The web's download says so too.
    app.ui.dialog = None;
    app.services.download = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
    app.run("ui.packageDialog", json!({})).unwrap();
    confirm(&mut app).unwrap();
    assert!(app.ui.status.contains(gone), "{}", app.ui.status);
}

#[test]
fn package_status_without_notes_has_no_extra_spaces() {
    let dir = Folder::new("spaces");
    let mut app = saved(&dir);
    app.run("ui.packageDialog", json!({})).unwrap();
    let out = dir.path("out");
    std::fs::create_dir_all(&out).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("folder".into(), json!(out));
    confirm(&mut app).unwrap();
    assert!(!app.ui.status.ends_with(' '), "{}", app.ui.status);
    let detail = app.ui.dialog.as_ref().unwrap().str("detail");
    assert!(!detail.contains(" ."), "{detail}");
}

/// A file the writer refuses is named once in the status bar: a refusal from the automation-root
/// check already names the path, and an error without it gets the path in front.
#[test]
fn a_refused_file_is_named_once() {
    let dir = Folder::new("refused");
    let mut app = saved(&dir);
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": false}})).unwrap();
    let (out, elsewhere, other) = (dir.path("out"), dir.path("elsewhere"), dir.path("other"));
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let doc = std::path::Path::new(&out).join("card Folder").join("card.vectorcraft").to_string_lossy().into_owned();
    let (elsewhere, other) = (std::path::Path::new(&elsewhere), std::path::Path::new(&other));
    for write_roots in [vec![elsewhere], vec![elsewhere, other]] {
        let roots = AutomationRoots::new(&[], &write_roots).unwrap().unwrap();
        let refused = roots.check(&doc, Access::Write).unwrap_err();
        assert!(refused.ends_with(&doc), "{refused}");
        app.services.write = Some(Box::new(move |p: &str, _: &[u8]| roots.check(p, Access::Write)));
        package_dialog(&mut app, &out);
        assert_eq!(confirm(&mut app), Err(refused.clone()));
        assert_eq!(app.ui.status, refused);
    }
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("Access is denied. (os error 5)".into())));
    package_dialog(&mut app, &out);
    assert!(confirm(&mut app).is_err());
    assert_eq!(app.ui.status, format!("{doc}: Access is denied. (os error 5)"));
}

#[test]
fn package_without_a_location_says_so() {
    let dir = Folder::new("empty");
    let mut app = saved(&dir);
    app.run("ui.packageDialog", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("folder".into(), json!(" "));
    assert!(confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some(), "stays open");
}
