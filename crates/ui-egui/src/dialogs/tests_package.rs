//! File → Package…: the dialog, saving first, the folder or the downloaded zip, Show Package.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::ui::dialog::{DialogId, HeldOk, dialog_id};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access::{Access, AutomationRoots};

use super::*;
use crate::i18n::LangExt as _;
use crate::tests_background::{Files, Gate, written};
use crate::{VectorcraftApp, background};

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

/// An app with a window, so its dialogs have a host.
fn windowed() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // A window context and a held file service.
    let _ = crate::tests::picks::scripted(&mut app);
    app
}

/// The id of the Package dialog.
fn package_id() -> DialogId {
    dialog_id("package")
}

/// Runs `f` on the open Package dialog.
fn with_package<R>(app: &VectorcraftApp, f: impl FnOnce(&mut HostedParamDialog) -> R) -> Option<R> {
    app.dialogs()?.with(|host| host.get_mut_by_id::<HostedParamDialog>(package_id()).map(f))
}

/// Sets the Package dialog's field `field`, as automation does.
fn set(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, package_id(), field, value))
}

/// Presses OK on the dialog `id`: it files its answer, the pump runs the app's OK and answers it, and a second OK
/// closes the dialog when the answer was accepted. Whether it closed.
fn press_ok_on(app: &mut VectorcraftApp, id: DialogId) -> bool {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let dialogs = app.dialogs().unwrap();
    dialogs.with(|host| host.confirm_by_id(&ctx, id));
    crate::dialogs::host::pump(app, &ctx);
    // A refused answer keeps the dialog open with its reason: another press would file the answer again.
    if matches!(dialogs.with(|host| host.held_state(id)), Some(HeldOk::Refused(_))) {
        return false;
    }
    dialogs.with(|host| host.confirm_by_id(&ctx, id))
}

/// [`press_ok_on`] the Package dialog.
fn press_ok(app: &mut VectorcraftApp) -> bool {
    press_ok_on(app, package_id())
}

fn held_state(app: &VectorcraftApp) -> Option<HeldOk> {
    app.dialogs().unwrap().with(|host| host.held_state(package_id()))
}

/// What OK does with the dialog's values, for a test that reads the reply (the dialog stays open).
fn reply(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = with_package(app, |d| d.fields().clone()).expect("the Package dialog is open");
    package::confirm(app, &fields)
}

/// The question the package asked (the one that is not the Package dialog), if one is open.
fn question_id(app: &VectorcraftApp) -> Option<DialogId> {
    let opened = app.dialogs()?.with(|host| host.open_dialogs());
    opened.into_iter().rev().find(|d| d.id != package_id()).map(|d| d.id)
}

/// Answers the question the package asked with OK, and runs what it asks for.
fn answer_question(app: &mut VectorcraftApp) {
    let id = question_id(app).expect("a question is open");
    press_ok_on(app, id);
}

/// Closes every open dialog, as Escape would.
fn close_all(app: &VectorcraftApp) {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    app.dialogs().unwrap().with(|host| {
        for d in host.open_dialogs().into_iter().rev() {
            host.cancel_by_id(&ctx, d.id);
        }
    });
}

/// An app writing through `std::fs`, with a saved document holding a placed linked image.
fn saved(dir: &Folder) -> VectorcraftApp {
    let mut app = windowed();
    app.services.write = Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(300, 300, image::Rgba([1, 2, 3, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    std::fs::write(dir.path("pic.png"), &png).unwrap();
    app.run("file.place", json!({"path": dir.path("pic.png")})).unwrap();
    app.run("document.save", json!({"path": dir.path("card.vectorcraft")})).unwrap();
    app
}

#[test]
fn an_unsaved_document_is_offered_save_as_first() {
    let mut app = windowed();
    assert!(!crate::menus::enabled(&app, "ui.packageDialog"), "no document");
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("ui.packageDialog", json!({})).unwrap();
    let (_, _, command) = confirm::open_question(&app).unwrap();
    assert_eq!(command, "file.saveAs");
    assert!(with_package(&app, |_| ()).is_none(), "no Package dialog for a document never saved");
}

#[test]
fn package_writes_the_folder_then_offers_to_show_it() {
    let dir = Folder::new("folder");
    let mut app = saved(&dir);
    app.run("ui.packageDialog", json!({})).unwrap();
    let (name, copy_fonts, folder) = with_package(&app, |d| {
        (d.fields().text("name").map(str::to_string), d.fields().bool("copyFonts"), d.fields().text("folder").map(str::to_string))
    })
    .unwrap();
    assert_eq!((name.as_deref(), copy_fonts), (Some("card Folder"), Some(true)));
    assert_eq!(folder.as_deref().map(std::path::Path::new), Some(dir.0.as_path()), "next to the document");
    let out = dir.path("out");
    std::fs::create_dir_all(&out).unwrap();
    set(&app, "folder", json!(out)).unwrap();
    // Unsaved changes are saved first.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(press_ok(&mut app), "the accepted OK closes the dialog");
    assert!(!app.session.active().unwrap().is_dirty());
    let root = std::path::Path::new(&out).join("card Folder");
    assert!(root.join("card.vectorcraft").is_file() && root.join("Links/pic.png").is_file());
    // The offer to show the folder stays open after the Package dialog closes.
    let (_, detail, command) = confirm::open_question(&app).unwrap();
    assert_eq!(command, "file.showPackage");
    assert!(detail.contains(root.to_string_lossy().as_ref()), "{detail}");
    assert!(with_package(&app, |_| ()).is_none());
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let o = opened.clone();
    app.services.open_file = Some(Box::new(move |p: &str| {
        o.borrow_mut().push(p.to_string());
        Ok(())
    }));
    answer_question(&mut app);
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
    set(app, "folder", json!(out)).unwrap();
}

#[test]
fn package_opening_drawing_is_required_before_saving_or_packaging() {
    let dir = Folder::new("opening-drawing");
    let mut app = saved(&dir);
    let source = app.session.active().unwrap().uid;
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let destination = app.session.active().unwrap().uid;
    let destination_path = dir.path("destination.vectorcraft");
    app.run("document.save", json!({"path": destination_path})).unwrap();
    let saved_destination = std::fs::read(&destination_path).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap();

    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, source);
    let out = dir.path("out");
    package_dialog(&mut app, &out);
    app.run("document.activate", json!({"index": 1})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, destination);
    let accepted = press_ok(&mut app);
    assert!(app.session.active().unwrap().is_dirty(), "Package leaves the different drawing unsaved");
    assert_eq!(std::fs::read(&destination_path).unwrap(), saved_destination);
    let package = std::path::Path::new(&out).join("card Folder");
    assert!(!package.exists());
    assert!(!accepted && matches!(held_state(&app), Some(HeldOk::Refused(_))));

    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(press_ok(&mut app));
    assert!(package.join("card.vectorcraft").is_file());
    assert!(!package.join("destination.vectorcraft").exists());
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
    let at_once = reply(&mut app).unwrap();
    let want: Vec<(String, Vec<u8>)> = at_once["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| root.join(f.as_str().unwrap()))
        .map(|p| (p.to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
        .collect();
    assert_eq!(want.len(), 3, "the document, its linked file and the report: {at_once}");
    let (status, detail) = (app.ui.status.clone(), confirm::open_question(&app).unwrap().1);
    std::fs::remove_dir_all(&root).unwrap();
    close_all(&app);
    // With Background Export on, the dialog closes at once, and the same files are written later.
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": true}})).unwrap();
    package_dialog(&mut app, &out);
    assert!(press_ok(&mut app), "closed");
    assert!(with_package(&app, |_| ()).is_none());
    assert!(app.background.jobs.last().is_some_and(|job| job.label.starts_with("Packaging card Folder")), "the job is named for the folder");
    assert!(written(&files).is_empty(), "nothing written yet");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(*files.lock().unwrap(), want);
    assert_eq!(app.ui.status, status);
    let (_, later_detail, command) = confirm::open_question(&app).unwrap();
    assert_eq!((command.as_str(), later_detail), ("file.showPackage", detail));
}

#[test]
fn edits_after_ok_do_not_reach_the_package() {
    let dir = Folder::new("snapshot");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let nodes = app.session.active().unwrap().doc.node_count();
    package_dialog(&mut app, &dir.path("out"));
    assert_eq!(reply(&mut app).unwrap()["background"], true);
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    // A dialog opened meanwhile stays open; the status bar reports the package.
    app.run("ui.packageDialog", json!({})).unwrap();
    gate.open();
    background::wait_all(&mut app);
    assert!(with_package(&app, |_| ()).is_some(), "the dialog opened meanwhile stays");
    assert!(app.ui.status.starts_with("Packaged 3 files in "), "{}", app.ui.status);
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
    // package starts once the save is written.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    package_dialog(&mut app, &out);
    // A thread opens the gate after 10 s at the latest, so a confirm that waits for the save ends.
    let opener = gate.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(10));
        opener.open();
    });
    assert_eq!(reply(&mut app).unwrap()["background"], true);
    assert_eq!(app.background.jobs.len(), 1, "the save; the package waits for it");
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
    reply(&mut app).unwrap();
    release.join().unwrap();
    assert_eq!(written(&files), [doc]);
    assert!(app.background.jobs.is_empty() && !app.session.active().unwrap().is_dirty());
    assert!(app.ui.status.starts_with("Packaged 3 files in "), "the package's status came last: {}", app.ui.status);
}

/// A save before packaging that fails writes no package, with Background Export on or off.
#[test]
fn a_failed_save_before_packaging_writes_no_package() {
    let dir = Folder::new("failed-save");
    let (doc, out) = (dir.path("card.vectorcraft"), dir.path("out"));
    let root = std::path::Path::new(&out).join("card Folder");
    for background_export in [true, false] {
        let mut app = saved(&dir);
        let (files, refused) = (Files::default(), doc.clone());
        let recorded = files.clone();
        app.services.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| {
            if p == refused {
                return Err("Access is denied. (os error 5)".into());
            }
            recorded.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        }));
        app.session.execute("prefs.set", &json!({"values": {"backgroundExport": background_export}})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        package_dialog(&mut app, &out);
        let r = reply(&mut app);
        background::wait_all(&mut app);
        assert!(written(&files).is_empty(), "Background Export {background_export}: {:?}", written(&files));
        assert!(!root.exists(), "Background Export {background_export}: no package folder");
        assert!(app.session.active().unwrap().is_dirty(), "the save failed");
        match r {
            Ok(r) => assert!(background_export && r["background"] == true, "{r}"),
            Err(e) => assert!(!background_export && e.contains("Access is denied"), "{e}"),
        }
    }
}

/// A save before packaging that Cancel Save stops writes no package either: the document keeps
/// its unsaved changes, so the package would not hold it as saved.
#[test]
fn a_canceled_save_before_packaging_writes_no_package() {
    let dir = Folder::new("canceled-save");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let out = dir.path("out");
    // An export holds the worker, so the save waits behind it and Cancel comes before it starts.
    crate::io::export(&mut app, Some("svg"), Some(dir.path("first.svg")), &json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    package_dialog(&mut app, &out);
    assert_eq!(reply(&mut app).unwrap()["background"], true);
    assert_eq!(app.run("file.cancelSave", json!({})).unwrap()["cancelled"], 1);
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), [dir.path("first.svg")], "only the export");
    assert!(!std::path::Path::new(&out).join("card Folder").exists(), "no package folder");
    assert!(app.session.active().unwrap().is_dirty(), "the save was canceled");
}

/// A package that an automation request confined to its roots asks for, started once the
/// document's background save is written, is still confined to those roots, though the save's
/// result arrives outside the request.
#[test]
fn a_package_started_after_its_save_keeps_the_automation_roots() {
    let (dir, elsewhere) = (Folder::new("scope"), Folder::new("scope-elsewhere"));
    let (doc, out) = (dir.path("card.vectorcraft"), elsewhere.path("out"));
    let root = std::path::Path::new(&out).join("card Folder");
    let mut app = saved(&dir);
    let files = Files::default();
    let recorded = files.clone();
    app.services.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| {
        vectorcraft_engine::file_access::check_write(p)?;
        recorded.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    let roots = AutomationRoots::new(&[dir.0.as_path()], &[dir.0.as_path()]).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    package_dialog(&mut app, &out);
    let r = vectorcraft_engine::file_access::confine(roots.as_ref(), || reply(&mut app)).unwrap();
    assert_eq!(r["background"], true, "{r}");
    // The save's result is applied outside the request, as the next frame does.
    background::wait_all(&mut app);
    assert_eq!(written(&files), [doc], "only the save, inside the roots");
    assert!(!root.exists(), "no package folder outside the roots");
    assert!(!app.session.active().unwrap().is_dirty(), "the save was written");
    assert!(app.ui.status.contains("failed"), "the package was refused: {}", app.ui.status);
}

#[test]
fn a_package_made_at_once_waits_for_one_in_the_background() {
    let dir = Folder::new("twice");
    let gate = Gate::default();
    let (mut app, files) = saved_with_worker(&dir, &gate);
    let out = dir.path("out");
    package_dialog(&mut app, &out);
    assert_eq!(reply(&mut app).unwrap()["background"], true);
    // With Background Export turned off, the same package made again is written after the one in
    // the background.
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": false}})).unwrap();
    package_dialog(&mut app, &out);
    let opener = gate.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        opener.open();
    });
    reply(&mut app).unwrap();
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
    assert!(set(&app, "folder", json!("elsewhere")).is_err(), "no folder to choose");
    assert!(press_ok(&mut app));
    assert!(with_package(&app, |_| ()).is_none());
    assert!(app.ui.status.contains("Packaged") && app.ui.status.contains("files"), "{}", app.ui.status);
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
    set(&app, "folder", json!(out)).unwrap();
    let r = reply(&mut app).unwrap();
    assert_eq!(r["skippedFonts"], json!([{"font": "No Such Font Family Regular", "reason": "not available on this computer"}]));
    let gone = "fonts not copied: No Such Font Family Regular";
    assert!(app.ui.status.contains(gone), "{}", app.ui.status);
    let (_, detail, _) = confirm::open_question(&app).unwrap();
    assert!(detail.contains(gone) && !detail.contains("Source Sans 3"), "{detail}");
    // The web's download says so too.
    close_all(&app);
    app.services.download = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
    app.run("ui.packageDialog", json!({})).unwrap();
    reply(&mut app).unwrap();
    assert!(app.ui.status.contains(gone), "{}", app.ui.status);
}

/// A package made while the interface is in another language keeps its status line in English,
/// note included, so the status bar can show it in the language of the moment. The dialog shown
/// at once is in the interface language.
#[test]
fn a_package_status_is_stored_in_english() {
    let dir = Folder::new("english");
    let mut app = saved(&dir);
    app.run("text.create", json!({"x": 10, "y": 80, "text": "Gone", "font": "No Such Font Family"})).unwrap();
    app.run("ui.packageDialog", json!({})).unwrap();
    let out = dir.path("out");
    std::fs::create_dir_all(&out).unwrap();
    set(&app, "folder", json!(out)).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::from_code("cs").unwrap());
    let r = reply(&mut app);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    r.unwrap();
    let status = &app.ui.status;
    assert!(status.starts_with("Packaged "), "{status}");
    assert!(status.contains(&format!(" (fonts not copied: No Such Font Family Regular) in {out}")), "{status}");
    // The prompt is in the interface language: its note is not the English one, and it still names the font.
    let (_, detail, _) = confirm::open_question(&app).unwrap();
    assert!(!detail.contains("(fonts not copied: No Such Font Family Regular)"), "{detail}");
    assert!(detail.contains("No Such Font Family Regular"), "{detail}");
}

/// A long list of fonts left out is cut short in the desktop's status line, so the status bar can
/// still translate the line. The dialog names every font.
#[test]
fn a_long_list_of_fonts_left_out_is_cut_short_in_the_status() {
    let fonts = (0..200).map(|i| format!("Family {i} Regular")).collect::<Vec<_>>().join(", ");
    assert_eq!(package::status_fonts("Mono, Sans"), "Mono, Sans", "a short list is kept whole");
    let cut = package::status_fonts(&fonts);
    assert!(cut.ends_with(", …") && cut.len() <= package::STATUS_FONTS + ", …".len() && cut.len() < fonts.len(), "{cut}");
    let note = package::package_status_note(0, &cut);
    assert!(note.starts_with("(fonts not copied: Family 0 Regular, ") && note.ends_with(", …)"), "{note}");
    assert!(package::package_dialog_note(0, &fonts).contains("Family 199 Regular"));
}

/// A package written in the background while another dialog is open asks nothing, so its
/// status line names every font left out, however long the list.
#[test]
fn a_background_package_behind_another_dialog_names_every_font_in_the_status() {
    let dir = Folder::new("fonts-behind");
    let gate = Gate::default();
    let (mut app, _files) = saved_with_worker(&dir, &gate);
    for i in 0..30 {
        app.run("text.create", json!({"x": 10, "y": 40 + i, "text": "Gone", "font": format!("No Such Font Family {i}")})).unwrap();
    }
    package_dialog(&mut app, &dir.path("out"));
    assert!(press_ok(&mut app));
    // File Info stands in for the other dialog: the Package dialog's key holds one dialog.
    app.run("ui.fileInfoDialog", json!({})).unwrap();
    gate.open();
    background::wait_all(&mut app);
    assert!(app.any_dialog_open(), "the other dialog stays");
    assert!(confirm::open_question(&app).is_none(), "nothing is asked behind it");
    let status = &app.ui.status;
    assert!(status.contains("No Such Font Family 0 Regular") && status.contains("No Such Font Family 29 Regular"), "{status}");
    assert!(!status.contains('…'), "{status}");
}

/// A package status written in English, for one file or several, with or without a folder and a
/// note, is translated by every catalog when the status bar shows it after a language switch, in
/// the form the language's plural rule picks for the count, and a folder whose name has a space or
/// parentheses keeps its whole name, next to one note or two.
#[test]
fn an_english_package_status_is_translated_after_a_language_switch() {
    use crate::i18n::{Lang, message};
    let folders = [None, Some("/out/card Folder"), Some("/out/card (draft) v2")];
    let notes = ["", "(fonts not copied: Mono)", "(1 linked file not found) (fonts not copied: Mono)"];
    let mut statuses = vec![];
    for n in [1, 3] {
        for folder in folders {
            for note in notes {
                statuses.push((package::package_status(n, folder, note), folder, note));
            }
        }
    }
    let english: Vec<&str> = statuses.iter().map(|(s, ..)| s.as_str()).collect();
    assert!(english.contains(&"Packaged 1 file in /out/card Folder"), "{english:?}");
    assert!(english.contains(&"Packaged 1 file (fonts not copied: Mono) in /out/card (draft) v2"), "{english:?}");
    assert!(english.contains(&"Packaged 3 files"), "{english:?}");
    for lang in Lang::all().filter(|l| l.code() != "en") {
        for (s, folder, note) in &statuses {
            let shown = message(lang, s);
            assert!(!shown.starts_with("Packaged"), "{}: {shown}", lang.code());
            // The folder stays whole in the translated line, and the note's font is still named.
            assert!(folder.is_none_or(|f| shown.contains(f)), "{}: {shown}", lang.code());
            assert_eq!(note.contains("Mono"), shown.contains("Mono"), "{}: {shown}", lang.code());
            // The notes are translated too.
            assert!(!shown.contains("linked file") && !shown.contains("not copied"), "{}: {shown}", lang.code());
        }
    }
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
        assert_eq!(reply(&mut app), Err(refused.clone()));
        assert_eq!(app.ui.status, refused);
    }
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("Access is denied. (os error 5)".into())));
    package_dialog(&mut app, &out);
    assert!(reply(&mut app).is_err());
    assert_eq!(app.ui.status, format!("{doc}: Access is denied. (os error 5)"));
}

#[test]
fn package_without_a_location_says_so() {
    let dir = Folder::new("empty");
    let mut app = saved(&dir);
    app.run("ui.packageDialog", json!({})).unwrap();
    set(&app, "folder", json!(" ")).unwrap();
    assert!(!press_ok(&mut app));
    assert!(matches!(held_state(&app), Some(HeldOk::Refused(_))), "OK is refused with its reason");
    assert!(with_package(&app, |_| ()).is_some(), "stays open");
}

#[test]
fn package_save_first_does_not_save_a_different_drawing() {
    let mut app = windowed();
    let picker = crate::tests::picks::scripted(&mut app);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("ui.packageDialog", json!({})).unwrap();
    assert_eq!(confirm::open_question(&app).unwrap().2, "file.saveAs");

    app.run("document.activate", json!({"index": 1})).unwrap();
    answer_question(&mut app);
    let ctx = app.ui_ctx.clone().unwrap();
    crate::tests::picks::poll(&mut app, &ctx);

    assert!(picker.requests().is_empty(), "Package in drawing A must never start Save As for drawing B");
    assert_eq!(app.ui.status, "This dialog was opened in a different document.");

    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("ui.packageDialog", json!({})).unwrap();
    answer_question(&mut app);
    crate::tests::picks::poll(&mut app, &ctx);
    assert_eq!(picker.requests().len(), 1, "approval in the opening drawing starts Save As");
}

#[test]
fn held_package_approval_does_not_write_output_after_revert() {
    let dir = Folder::new("package-revert");
    let mut app = saved(&dir);
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": false, "backgroundSave": false}})).unwrap();
    let before = std::fs::read(dir.path("card.vectorcraft")).unwrap();
    let out = dir.path("out");
    let output = std::path::Path::new(&out).join("card Folder");
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap();
    package_dialog(&mut app, &out);
    let ctx = app.ui_ctx.clone().unwrap();
    let host = app.dialogs().unwrap();
    assert!(!host.with(|h| h.confirm_by_id(&ctx, package_id())));
    assert_eq!(held_state(&app), Some(HeldOk::Waiting));

    app.run("file.revert", json!({"confirmed": true})).unwrap();
    crate::dialogs::host::pump(&mut app, &ctx);
    assert!(!output.exists(), "the old approval writes no output");
    assert_eq!(std::fs::read(dir.path("card.vectorcraft")).unwrap(), before);
    assert!(matches!(held_state(&app), Some(HeldOk::Refused(_))));

    close_all(&app);
    package_dialog(&mut app, &out);
    assert!(press_ok(&mut app));
    assert!(output.join("card.vectorcraft").exists());
}
