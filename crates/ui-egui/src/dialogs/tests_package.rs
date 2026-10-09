//! File → Package…: the dialog, saving first, the folder or the downloaded zip, Show Package.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

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

#[test]
fn the_web_downloads_a_zip() {
    let dir = Folder::new("web");
    let mut app = saved(&dir);
    let got: Rc<RefCell<Vec<(String, usize)>>> = Rc::default();
    let g = got.clone();
    app.services.download = Some(Box::new(move |name: &str, b: &[u8]| g.borrow_mut().push((name.to_string(), b.len()))));
    app.run("ui.packageDialog", json!({})).unwrap();
    assert!(!frame(&mut app).contains("Location:"), "no folder to choose");
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
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
    app.services.download = Some(Box::new(|_: &str, _: &[u8]| {}));
    app.run("ui.packageDialog", json!({})).unwrap();
    confirm(&mut app).unwrap();
    assert!(app.ui.status.contains(gone), "{}", app.ui.status);
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
