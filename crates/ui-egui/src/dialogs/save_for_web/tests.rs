//! Save for Web (Legacy), driven through the hosted dialog and its pump as an agent drives it.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::dialog::{DialogId, StackDialog};
use craft::ui::patterns::form_dialogs::HostedSaveForWeb;
use craft::ui::shell::WindowDialogs;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;
use crate::dialogs::tests_export::{answer_saves, save_panel};
use crate::{Services, VectorcraftApp};

pub(super) type Log = Rc<RefCell<Vec<String>>>;

/// An app with a 60×40 pt document (an ellipse), whose save panel answers `<dir>/<suggested name>`; `web`
/// downloads instead (into the log). Its window context is the one [`frame`] runs.
pub(super) fn app(dir: &std::path::Path, web: bool) -> (VectorcraftApp, Log) {
    let log = Log::default();
    let l = log.clone();
    let mut services = Services::default();
    if web {
        services.download = Some(Box::new(move |name: &str, b: &[u8]| {
            l.borrow_mut().push(format!("download {name} {}", b.len()));
            Ok(())
        }));
    }
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    // The files go to disk, where the tests read them back.
    app.services.write = Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string())));
    let dir = dir.to_path_buf();
    save_panel(&mut app, move |p| Some(dir.join(p.file_name.as_deref().unwrap_or_default()).to_string_lossy().into_owned()));
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    app.run("paint.setFill", json!({"color": "#cc3300"})).unwrap();
    (app, log)
}

/// Two distinct export regions: 60×40 red and 32×24 blue, with no antialiased edges at the sample.
fn two_artboards(dir: &std::path::Path) -> VectorcraftApp {
    let (mut app, _) = app(dir, false);
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("artboard.new", json!({"x": 100, "y": 0, "width": 32, "height": 24})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 0, "width": 32, "height": 24})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app
}

pub(super) fn host(app: &VectorcraftApp) -> WindowDialogs {
    app.dialogs().expect("the app has a window")
}

fn is_open(app: &VectorcraftApp) -> bool {
    host(app).with(|h| h.get::<HostedSaveForWeb>().is_some())
}

pub(super) fn id(app: &VectorcraftApp) -> DialogId {
    host(app).with(|h| h.get::<HostedSaveForWeb>().map(StackDialog::id)).expect("Save for Web is open")
}

/// The open dialog's values, as automation lists them.
fn fields(app: &VectorcraftApp) -> Value {
    host(app).with(|h| h.get::<HostedSaveForWeb>().and_then(StackDialog::json_fields)).expect("the dialog lists its values")
}

/// Sets the field `key` as the user editing that control would.
pub(super) fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    let id = id(app);
    host(app).with(|h| h.set_json_field(&ctx, id, key, value)).unwrap();
}

/// One frame of the host and the pump; returns what the frame asked the platform to open.
fn frame_opened(app: &mut VectorcraftApp) -> Vec<String> {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    let host = host(app);
    let mut out = ctx.run_ui(Default::default(), |ui| {
        // The app's tick sends the URLs it was asked to open, as the hosts run it before the window draws.
        app.tick(ui.ctx());
        host.with(|h| h.run(ui));
        crate::dialogs::host::pump(app, ui.ctx());
    });
    out.textures_delta.clear();
    out.platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(o) => Some(o.url.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn frame(app: &mut VectorcraftApp) {
    frame_opened(app);
}

/// Enough frames for a request to be filed, answered and shown.
fn settle(app: &mut VectorcraftApp) {
    for _ in 0..4 {
        frame(app);
    }
}

/// The URLs the next frame asks the platform to open.
fn opened(app: &mut VectorcraftApp) -> Vec<String> {
    frame_opened(app)
}

/// Presses Save… (the dialog asks for a path, the scripted panel answers, the dialog closes and the
/// pump writes the files).
pub(super) fn confirm(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    let id = id(app);
    host(app).with(|h| h.confirm_by_id(&ctx, id));
    settle(app);
    answer_saves(app);
    settle(app);
}

pub(super) fn save(app: &mut VectorcraftApp) {
    confirm(app);
    assert!(!is_open(app), "Save closes the dialog");
}

/// Closes the dialog with Cancel.
fn close(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    let id = id(app);
    host(app).with(|h| h.cancel_by_id(&ctx, id));
    frame(app);
}

fn pixels(path: impl AsRef<std::path::Path>) -> image::RgbaImage {
    image::open(path).unwrap().to_rgba8()
}

#[test]
fn opens_on_the_artboard_tools_active_artboard_and_exports_it() {
    let mut outputs = Vec::new();
    for remembered in [false, true] {
        let dir = vectorcraft_testkit::temp_dir(if remembered { "sfw-active-remembered" } else { "sfw-active-default" });
        let mut app = two_artboards(&dir);
        if remembered {
            app.run("webExport.settings", json!({"artboard": 0, "quality": 70})).unwrap();
        }
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        app.run("tool.setOption", json!({"key": "active", "value": 1})).unwrap();
        assert_eq!(app.view().unwrap().artboard, 1);
        app.run("file.saveForWeb", json!({})).unwrap();
        let initial = fields(&app)["artboard"].clone();
        set(&mut app, "format", json!("png24"));
        frame(&mut app);
        save(&mut app);
        let image = pixels(dir.join("Untitled-1.png"));
        outputs.push((remembered, initial, image.dimensions(), image.get_pixel(8, 8).0));
    }
    assert_eq!(outputs, [(false, json!(1), (32, 24), [0, 0, 255, 255]), (true, json!(1), (32, 24), [0, 0, 255, 255]),]);
}

#[test]
fn opens_on_the_active_artboard_with_the_selection_tool_and_exports_it() {
    for (active, size, color) in [(1, (32, 24), [0, 0, 255, 255]), (0, (60, 40), [255, 0, 0, 255])] {
        let dir = vectorcraft_testkit::temp_dir(&format!("sfw-selection-active-{active}"));
        let mut app = two_artboards(&dir);
        app.run("webExport.settings", json!({"artboard": 1 - active, "format": "png24", "quality": 73})).unwrap();
        let saved = app.session.last_web_settings().to_json();
        app.run("tool.select", json!({"tool": "selection"})).unwrap();
        app.run("view.goToArtboard", json!({"index": active})).unwrap();
        assert_eq!(app.session.tool_id(), "selection");
        assert_eq!(app.view().unwrap().artboard, active);
        app.run("file.saveForWeb", json!({})).unwrap();
        assert_eq!(fields(&app)["artboard"], json!(active));
        assert_eq!(app.session.last_web_settings().to_json(), saved, "opening does not overwrite the remembered settings");
        frame(&mut app);
        save(&mut app);
        let image = pixels(dir.join("Untitled-1.png"));
        assert_eq!(image.dimensions(), size);
        assert_eq!(image.get_pixel(8, 8).0, color);
    }
}

#[test]
fn explicit_artboard_changes_update_the_preview_and_saved_pixels() {
    let dir = vectorcraft_testkit::temp_dir("sfw-explicit-artboard");
    let mut app = two_artboards(&dir);
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    app.run("tool.setOption", json!({"key": "active", "value": 1})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    for (index, size, color) in [(0, (60, 40), [255, 0, 0, 255]), (1, (32, 24), [0, 0, 255, 255])] {
        set(&mut app, "artboard", json!(index));
        settle(&mut app);
        let render = match &app.web_export.render {
            Some((_, Ok(r))) => r.clone(),
            other => panic!("the pane pictures are rendered again for artboard {index}: {:?}", other.as_ref().map(|(k, _)| k)),
        };
        assert_eq!((render.width, render.height), (size.0, size.1), "artboard {index}");
        assert!(render.rgba.as_chunks::<4>().0.iter().all(|pixel| *pixel == color));
        // Save through the same dialog: the file holds the pixels the panes were made from.
        save(&mut app);
        let image = pixels(dir.join("Untitled-1.png"));
        assert_eq!(image.dimensions(), size);
        assert_eq!(image.as_raw(), &render.rgba);
        if index == 0 {
            app.run("file.saveForWeb", json!({})).unwrap();
        }
    }
}

#[test]
fn opening_uses_the_active_artboard_in_a_multiselection_and_preserves_other_settings() {
    let dir = vectorcraft_testkit::temp_dir("sfw-settings-artboard");
    let mut app = two_artboards(&dir);
    app.run("webExport.settings", json!({"preset": "JPEG, quality 50", "artboard": 0, "quality": 73, "width": 96, "metadata": "all"})).unwrap();
    let saved = app.session.last_web_settings();
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    app.run("tool.setOption", json!({"key": "selected", "value": [1, 0]})).unwrap();
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(crate::panels::artboards::picked(&app, 2), [0, 1]);
    // The remembered settings as the dialog names them, on the active artboard.
    let expected = json!({"quality": 73, "width": 96, "metadata": "all", "artboard": 1});
    app.run("file.saveForWeb", json!({})).unwrap();
    for key in ["quality", "width", "metadata", "artboard"] {
        assert_eq!(fields(&app)[key], expected[key], "{key}");
    }
    assert_eq!(app.session.last_web_settings(), saved, "opening does not overwrite the remembered settings");
    close(&mut app);
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    assert_eq!(app.view().unwrap().artboard, 1);
    app.run("file.saveForWeb", json!({})).unwrap();
    for key in ["quality", "width", "metadata", "artboard"] {
        assert_eq!(fields(&app)[key], expected[key], "Selection tool keeps {key}");
    }
    assert_eq!(app.session.last_web_settings(), saved, "opening still does not overwrite the remembered settings");
}

#[test]
fn a_remembered_artboard_missing_from_another_document_falls_back_to_its_first() {
    let dir = vectorcraft_testkit::temp_dir("sfw-other-document");
    let mut app = two_artboards(&dir);
    app.run("webExport.settings", json!({"artboard": 1, "format": "png24"})).unwrap();
    app.run("file.new", json!({"width": 20, "height": 12})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 12})).unwrap();
    app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    assert_eq!(fields(&app)["artboard"], json!(0));
    frame(&mut app);
    save(&mut app);
    // The file the save wrote, named after the second document.
    let written: Vec<_> =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    assert_eq!(written.len(), 1, "{written:?}");
    let image = pixels(&written[0]);
    assert_eq!(image.dimensions(), (20, 12));
    assert_eq!(image.get_pixel(8, 8).0, [0, 255, 0, 255]);
}

#[test]
fn explicit_command_artboards_override_the_ui_active_artboard() {
    let dir = vectorcraft_testkit::temp_dir("sfw-api-artboard");
    let mut app = two_artboards(&dir);
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    for (active, explicit, size, color) in [(0, 1, (32, 24), [0, 0, 255, 255]), (1, 0, (60, 40), [255, 0, 0, 255])] {
        app.run("tool.setOption", json!({"key": "active", "value": active})).unwrap();
        assert_eq!(app.view().unwrap().artboard, active);
        for command in ["file.saveForWeb", "document.exportForWeb"] {
            let path = dir.join("explicit.png").to_string_lossy().replace('\\', "/");
            app.run(command, json!({"format": "png24", "artboard": explicit, "path": path})).unwrap();
            let image = image::open(&path).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), size, "{command}: UI active {active}, explicit {explicit}");
            assert_eq!(image.get_pixel(8, 8).0, color);
            assert!(!is_open(&app));
        }
    }
}

#[test]
fn save_remembers_and_writes_and_done_only_remembers() {
    let dir = vectorcraft_testkit::temp_dir("sfw-save");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    // A preset field loads the preset; the keys set after it win.
    set(&mut app, "preset", json!("JPEG, quality 50"));
    frame(&mut app);
    assert_eq!(fields(&app)["quality"].as_f64(), Some(50.0));
    set(&mut app, "quality", json!(70));
    save(&mut app);
    let path = dir.join("Untitled-1.jpg");
    assert!(std::fs::read(&path).unwrap().starts_with(&[0xFF, 0xD8]));
    assert_eq!(app.session.last_web_settings().quality, 70, "remembered");
    // It reopens on them; Done remembers without writing.
    app.run("file.saveForWeb", json!({})).unwrap();
    assert_eq!(fields(&app)["quality"].as_f64(), Some(70.0));
    set(&mut app, "format", json!("png24"));
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    assert!(!is_open(&app));
    assert_eq!(app.session.last_web_settings().format, vectorcraft_engine::cmd::webexport::WebFormat::Png24);
    assert!(!dir.join("Untitled-1.png").exists());
    // Bad settings keep the dialog open, with nothing written and the reason in the status line.
    app.run("file.saveForWeb", json!({})).unwrap();
    app.web_export.table = json!({"locked": ["nope"]});
    app.status("");
    confirm(&mut app);
    assert!(is_open(&app));
    assert!(!app.ui.status.is_empty(), "the reason shows in the status line");
    assert!(!dir.join("Untitled-1.png").exists());
}

/// The remembered Transparency Dither, which the dialog does not show, reaches the file Save writes
/// and stays remembered after Done.
#[test]
fn save_keeps_the_remembered_transparency_dither() {
    let dir = vectorcraft_testkit::temp_dir("sfw-tdither");
    let (mut app, _) = app(&dir, false);
    let mut written = vec![];
    for method in ["none", "noise"] {
        app.run("webExport.settings", json!({"format": "gif", "transparency": true, "transparencyDither": method})).unwrap();
        app.run("file.saveForWeb", json!({})).unwrap();
        save(&mut app);
        let path = dir.join("Untitled-1.gif");
        written.push(std::fs::read(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(app.session.last_web_settings().transparency_dither.id(), method, "remembered");
        let expected = app.session.execute("document.exportForWeb", &json!({"format": "gif", "transparencyDither": method})).unwrap();
        assert_eq!(expected["files"][0]["dataBase64"].as_str().and_then(vectorcraft_format::base64_decode).as_ref(), written.last());
    }
    assert_ne!(written[0], written[1], "the partly transparent edge is dithered");
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    assert_eq!(app.session.last_web_settings().transparency_dither.id(), "noise");
}

#[test]
fn html_output_with_slices_saves_the_page_and_images() {
    let dir = vectorcraft_testkit::temp_dir("sfw-html");
    let (mut app, _) = app(&dir, false);
    app.run("object.slice.create", json!({"x": 0, "y": 0, "width": 30, "height": 20})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "html", json!(true));
    frame(&mut app);
    save(&mut app);
    let n = app.session.execute("slice.list", &json!({})).unwrap()["slices"].as_array().unwrap().len();
    assert!(std::fs::read_to_string(dir.join("Untitled-1.html")).unwrap().contains("images/Untitled-1_01.gif"));
    assert!(dir.join("images/Untitled-1_01.gif").exists());
    assert_eq!(std::fs::read_dir(dir.join("images")).unwrap().count(), n);
}

#[test]
fn the_web_downloads_and_has_no_browser_preview() {
    let dir = vectorcraft_testkit::temp_dir("sfw-web");
    let (mut app, log) = app(&dir, true);
    let r = app.run("file.saveForWeb", json!({"format": "png8", "output": "html"})).unwrap();
    assert_eq!(r["files"], json!(["Untitled-1.html", "images/Untitled-1.png"]));
    let log = log.borrow().clone();
    assert!(log[0].starts_with("download Untitled-1.html ") && log[1].starts_with("download Untitled-1.png "), "{log:?}");
    assert!(app.run("file.saveForWeb.browser", json!({})).is_err());
    assert!(crate::menus::enabled(&app, "file.saveForWeb"));
    assert!(!crate::menus::enabled(&app, "file.saveForWeb.browser"), "the command palette and menus show it disabled on the web");
}

#[test]
fn preview_in_browser_stays_in_its_folder_and_is_the_persons_to_open() {
    for (title, name) in
        [("Poster", "Poster"), ("../..", "Untitled"), ("..", "Untitled"), ("a/..", "Untitled"), ("..\\x", "Untitled"), (" ", "Untitled")]
    {
        assert_eq!(preview_folders::preview_name(title), name, "{title}");
    }
    let dir = vectorcraft_testkit::temp_dir("sfw-browser-automation");
    let (mut app, _) = app(&dir, false);
    app.ui.answering_control = true;
    assert!(app.run("file.saveForWeb.browser", json!({"format": "jpg"})).unwrap_err().contains("opens another app"));
    app.ui.answering_control = false;
    assert!(opened(&mut app).is_empty());
}

#[test]
fn preview_in_browser_opens_the_page() {
    let dir = vectorcraft_testkit::temp_dir("sfw-browser");
    let (mut app, _) = app(&dir, false);
    let r = app.run("file.saveForWeb.browser", json!({"format": "jpg"})).unwrap();
    let page = r["path"].as_str().unwrap().to_string();
    assert!(page.ends_with("/Untitled-1.html"));
    assert!(std::fs::read_to_string(&page).unwrap().contains("images/Untitled-1.jpg"));
    let urls = opened(&mut app);
    assert!(urls.last().is_some_and(|u| u.starts_with("file://") && u.ends_with("Untitled-1.html")), "{urls:?}");
}

/// A preview whose export fails opens nothing and removes the folder made for it.
#[test]
fn a_failed_browser_preview_removes_its_folder() {
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-failed");
    preview_folders::TEST_TMP.set(Some(tmp.clone()));
    let (mut app, _) = app(&tmp, false);
    assert!(app.run("file.saveForWeb.browser", json!({"format": "bogus"})).is_err());
    assert!(opened(&mut app).is_empty());
    let left: Vec<_> =
        std::fs::read_dir(tmp.join("VectorCraft Save for Web")).map(|d| d.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

/// Each preview has its own folder: a second one, from another app or the same one, leaves the
/// first page and its image as they were.
#[test]
fn a_later_browser_preview_leaves_an_earlier_one_alone() {
    let dir = vectorcraft_testkit::temp_dir("sfw-browser-two");
    let (mut first, _) = app(&dir, false);
    let (mut second, _) = app(&dir, false);
    let page = |r: serde_json::Value| r["path"].as_str().unwrap().to_string();
    let jpg = page(first.run("file.saveForWeb.browser", json!({"format": "jpg"})).unwrap());
    let gif = page(second.run("file.saveForWeb.browser", json!({"format": "gif"})).unwrap());
    let png = page(first.run("file.saveForWeb.browser", json!({"format": "png24"})).unwrap());
    assert!(jpg != gif && jpg != png && gif != png, "{jpg} {gif} {png}");
    let root = std::env::temp_dir().join("VectorCraft Save for Web").to_string_lossy().replace('\\', "/");
    for (page, image) in [(&jpg, "images/Untitled-1.jpg"), (&gif, "images/Untitled-1.gif"), (&png, "images/Untitled-1.png")] {
        assert!(page.starts_with(&root) && page.ends_with("/Untitled-1.html"), "{page}");
        assert!(std::fs::read_to_string(page).unwrap().contains(image), "{page}");
        let folder = std::path::Path::new(page).parent().unwrap();
        assert!(folder.join(image).exists(), "{page}");
    }
}

/// Making a preview's folder removes the preview folders in its root that are `keep` old or
/// older, and keeps younger ones.
#[test]
fn a_browser_preview_removes_old_previews() {
    let root = vectorcraft_testkit::temp_dir("sfw-browser-old").join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let earlier = preview_folders::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    std::fs::create_dir_all(format!("{earlier}/images")).unwrap();
    std::fs::write(format!("{earlier}/images/Untitled-1.gif"), b"GIF89a").unwrap();
    std::fs::write(root.join("Untitled-1.html"), b"<html>").unwrap();
    // The folder and the page are younger than an hour, so both stay.
    let kept = preview_folders::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    assert!(std::path::Path::new(&earlier).exists() && root.join("Untitled-1.html").exists());
    // With a `keep` of zero every entry is old enough: both preview folders go, and the page, which
    // no preview made, stays beside the new folder.
    let next = preview_folders::preview_folder(&root, std::time::Duration::ZERO).unwrap();
    assert!(!std::path::Path::new(&earlier).exists() && !std::path::Path::new(&kept).exists() && root.join("Untitled-1.html").exists());
    assert!(std::path::Path::new(&next).is_dir());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}

/// Old entries of the root that aren't preview folders stay: a file, folders whose names aren't
/// `<digits>-<digits>`, and a file with a preview folder's name.
#[test]
fn a_browser_preview_removes_only_preview_folders() {
    let root = vectorcraft_testkit::temp_dir("sfw-browser-filter").join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let earlier = preview_folders::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    let folders = ["notes", "12-", "-3", "12-3a", "a12-3", "12_3"];
    for name in folders {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    let files = ["Untitled-1.html", "12-3"];
    for name in files {
        std::fs::write(root.join(name), b"kept").unwrap();
    }
    let next = preview_folders::preview_folder(&root, std::time::Duration::ZERO).unwrap();
    assert!(!std::path::Path::new(&earlier).exists() && std::path::Path::new(&next).is_dir());
    for name in folders {
        assert!(root.join(name).is_dir(), "{name}");
    }
    for name in files {
        assert_eq!(std::fs::read(root.join(name)).unwrap(), b"kept", "{name}");
    }
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), folders.len() + files.len() + 1);
}

/// When `VectorCraft Save for Web` in the temporary folder is a file, the preview goes to a new
/// folder of its own in the temporary folder, and the file stays as it was.
#[test]
fn a_browser_preview_goes_beside_a_root_that_is_not_a_folder() {
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-file-root");
    let root = tmp.join("VectorCraft Save for Web");
    std::fs::write(&root, b"a file").unwrap();
    let keep = std::time::Duration::ZERO;
    let e = preview_folders::preview_folder(&root, keep).unwrap_err();
    assert!(e.ends_with(": it is not a folder"), "{e}");
    let dir = preview_folders::preview_folder_in(&tmp, keep).unwrap();
    let dir = std::path::Path::new(&dir);
    assert!(dir.is_dir() && dir.parent() == Some(tmp.as_path()), "{}", dir.display());
    assert!(dir.file_name().unwrap().to_string_lossy().starts_with("VectorCraft Save for Web "), "{}", dir.display());
    assert_eq!(std::fs::read(&root).unwrap(), b"a file");
}

/// The folders that previews made beside a root they couldn't use are removed once they are
/// `keep` old, both while the root still can't be used and once it can be used again; younger
/// ones and other entries stay.
#[test]
fn a_browser_preview_removes_old_folders_made_beside_the_root() {
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-beside-old");
    let root = tmp.join("VectorCraft Save for Web");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::write(&root, b"a file").unwrap();
    let hour = std::time::Duration::from_secs(3600);
    let first = preview_folders::preview_folder_in(&tmp, hour).unwrap();
    let second = preview_folders::preview_folder_in(&tmp, hour).unwrap();
    assert!(std::path::Path::new(&first).is_dir() && std::path::Path::new(&second).is_dir(), "{first} {second}");
    let others = ["VectorCraft Save for Web 7-7", "VectorCraft Save for Web 7-7.html", "VectorCraft Save for Web notes"];
    std::fs::write(tmp.join(others[0]), b"kept").unwrap();
    std::fs::write(tmp.join(others[1]), b"kept").unwrap();
    std::fs::create_dir_all(tmp.join(others[2])).unwrap();
    let third = preview_folders::preview_folder_in(&tmp, std::time::Duration::ZERO).unwrap();
    assert_eq!(std::path::Path::new(&third).parent(), Some(tmp.as_path()), "{third}");
    assert!(!std::path::Path::new(&first).exists() && !std::path::Path::new(&second).exists(), "{first} {second}");
    std::fs::remove_file(&root).unwrap();
    let dir = preview_folders::preview_folder_in(&tmp, std::time::Duration::ZERO).unwrap();
    assert_eq!(std::path::Path::new(&dir).parent(), Some(root.as_path()), "{dir}");
    assert!(!std::path::Path::new(&third).exists(), "{third}");
    for name in others {
        assert!(tmp.join(name).exists(), "{name}");
    }
}

/// The unix mode bits of `path`, not following a link.
#[cfg(unix)]
fn mode(path: impl AsRef<std::path::Path>) -> u32 {
    std::os::unix::fs::MetadataExt::mode(&std::fs::symlink_metadata(path).unwrap()) & 0o777
}

/// A root and a preview folder that a preview makes are open to their owner only (0o700).
#[cfg(unix)]
#[test]
fn a_browser_preview_makes_private_folders() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-private");
    let root = tmp.join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let dir = preview_folders::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    assert_eq!((mode(&root), mode(&dir)), (0o700, 0o700));
    // Beside a root it can't use, too.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
    let shared = tmp.join("VectorCraft Save for Web");
    let _ = std::fs::remove_dir_all(&shared);
    std::fs::rename(&root, &shared).unwrap();
    let own = preview_folders::preview_folder_in(&tmp, std::time::Duration::from_secs(3600)).unwrap();
    assert_eq!(std::path::Path::new(&own).parent(), Some(tmp.as_path()), "{own}");
    assert_eq!(mode(&own), 0o700);
}

/// A root that is a link is refused: nothing in the folder it points to is removed or added, and
/// the preview goes to a new folder of its own in the temporary folder.
#[cfg(unix)]
#[test]
fn a_browser_preview_refuses_a_linked_root() {
    use std::os::unix::fs::symlink;
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-linked-root");
    let elsewhere = tmp.join("elsewhere");
    std::fs::create_dir_all(elsewhere.join("5-1")).unwrap();
    let root = tmp.join("VectorCraft Save for Web");
    let _ = std::fs::remove_file(&root);
    symlink(&elsewhere, &root).unwrap();
    let keep = std::time::Duration::ZERO;
    let e = preview_folders::preview_folder(&root, keep).unwrap_err();
    assert!(e.ends_with(": it is a link"), "{e}");
    let dir = preview_folders::preview_folder_in(&tmp, keep).unwrap();
    assert_eq!(std::path::Path::new(&dir).parent(), Some(tmp.as_path()));
    let left: Vec<String> = std::fs::read_dir(&elsewhere).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, ["5-1"]);
}

/// A root that users other than its owner can write to is refused, and the old preview folder in
/// it stays.
#[cfg(unix)]
#[test]
fn a_browser_preview_refuses_a_root_others_can_write_to() {
    use std::os::unix::fs::PermissionsExt;
    let root = vectorcraft_testkit::temp_dir("sfw-browser-shared-root").join("previews");
    let keep = std::time::Duration::ZERO;
    for bits in [0o777, 0o770, 0o703] {
        let _ = std::fs::remove_dir_all(&root);
        let earlier = preview_folders::preview_folder(&root, keep).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(bits)).unwrap();
        let e = preview_folders::preview_folder(&root, keep).unwrap_err();
        assert!(e.ends_with(": other users can write to it"), "{bits:o}: {e}");
        assert!(std::path::Path::new(&earlier).is_dir(), "{bits:o}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "{bits:o}");
    }
}

/// An old link named like a preview folder stays, and so does the folder it points to, with what
/// it holds.
#[cfg(unix)]
#[test]
fn a_browser_preview_leaves_a_link_named_like_a_preview_folder() {
    use std::os::unix::fs::symlink;
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-link-entry");
    let root = tmp.join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let outside = tmp.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("page.html"), b"kept").unwrap();
    preview_folders::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    symlink(&outside, root.join("9-9")).unwrap();
    preview_folders::preview_folder(&root, std::time::Duration::ZERO).unwrap();
    assert!(std::fs::symlink_metadata(root.join("9-9")).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(outside.join("page.html")).unwrap(), b"kept");
}

/// A root and a preview folder that another user owns are refused and left alone. Only root can
/// give a folder to another user, so the test skips itself for other users.
#[cfg(unix)]
#[test]
fn a_browser_preview_leaves_what_another_user_owns() {
    let root = vectorcraft_testkit::temp_dir("sfw-browser-other-user").join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let keep = std::time::Duration::ZERO;
    let theirs = preview_folders::preview_folder(&root, keep).unwrap();
    let other = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&root).unwrap()) + 1;
    if std::os::unix::fs::chown(&theirs, Some(other), None).is_err() {
        eprintln!("skipped: only root can give a folder to another user");
        return;
    }
    preview_folders::preview_folder(&root, keep).unwrap();
    assert!(std::path::Path::new(&theirs).is_dir());
    std::os::unix::fs::chown(&root, Some(other), None).unwrap();
    let e = preview_folders::preview_folder(&root, keep).unwrap_err();
    assert!(e.ends_with(": it belongs to another user"), "{e}");
}

/// A folder that an earlier process with the same id left, younger than `keep`, is not reused:
/// the preview makes a new, empty one.
#[test]
fn a_browser_preview_skips_a_folder_an_earlier_process_left() {
    let root = vectorcraft_testkit::temp_dir("sfw-browser-reused").join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let keep = std::time::Duration::from_secs(3600);
    let earlier = preview_folders::preview_folder(&root, keep).unwrap();
    let count: u64 = earlier.rsplit('-').next().unwrap().parse().unwrap();
    // The next 200 names, as an earlier process with the same id would have left them.
    for n in count + 1..=count + 200 {
        let left = root.join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(left.join("images")).unwrap();
        std::fs::write(left.join("Untitled-1.html"), b"<html>").unwrap();
    }
    let next = preview_folders::preview_folder(&root, keep).unwrap();
    assert_ne!(next, earlier);
    assert_eq!(std::fs::read_dir(&next).unwrap().count(), 0, "{next}");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 202);
}

#[test]
fn saving_says_in_the_status_bar_when_an_image_could_not_be_decoded() {
    const NOTE: &str = "with 1 note: an image couldn't be decoded and is missing from the picture";
    // A PNG whose header reads but whose pixel data is cut short: the renderer draws nothing for it.
    let mut png = Vec::new();
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))).into();
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
    let place = |app: &mut VectorcraftApp| vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({"at": [30, 20]}));
    let dir = vectorcraft_testkit::temp_dir("sfw-undecoded");
    let (mut desktop, _) = app(&dir, false);
    desktop.run("file.saveForWeb", json!({"format": "png24"})).unwrap();
    // The save panel answers on the next poll.
    answer_saves(&mut desktop);
    assert!(!desktop.ui.status.contains("note"), "{}", desktop.ui.status);
    place(&mut desktop);
    for p in [json!({"format": "png24"}), json!({"format": "jpg", "output": "html"})] {
        desktop.status("");
        desktop.run("file.saveForWeb", p.clone()).unwrap();
        answer_saves(&mut desktop);
        assert!(desktop.ui.status.starts_with("Saved ") && desktop.ui.status.ends_with(NOTE), "{p}: {}", desktop.ui.status);
    }
    desktop.status("");
    desktop.run("file.saveForWeb.browser", json!({"format": "gif"})).unwrap();
    assert!(desktop.ui.status.ends_with(NOTE), "Preview in Browser: {}", desktop.ui.status);
    let (mut web, _) = app(&dir, true);
    place(&mut web);
    web.run("file.saveForWeb", json!({"format": "png8"})).unwrap();
    assert!(web.ui.status.ends_with(NOTE), "download: {}", web.ui.status);
}

/// The bytes of the `document.exportForWeb.preview` encode of `params`.
fn encoded_bytes(app: &mut VectorcraftApp, params: Value) -> u64 {
    app.session.execute("document.exportForWeb.preview", &params).unwrap()["bytes"].as_u64().unwrap()
}

#[test]
fn optimize_to_file_size_takes_the_settings_that_fit() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("jpeg"));
    set(&mut app, "quality", json!(100));
    settle(&mut app);
    let full = encoded_bytes(&mut app, json!({"format": "jpg", "quality": 100}));
    set(&mut app, "optimizeOpen", json!(true));
    settle(&mut app);
    // A size of 0 is refused: the settings and the rows stay.
    let start = fields(&app)["optimizeK"].as_f64();
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    let dialog = id(&app);
    assert!(host(&app).with(|h| h.set_json_field(&ctx, dialog, "optimizeK", json!(0))).is_err());
    set(&mut app, "optimizeStart", json!("current"));
    settle(&mut app);
    assert_eq!(fields(&app)["quality"].as_f64(), Some(100.0));
    assert_eq!(fields(&app)["optimizeK"].as_f64(), start);
    assert!(app.web_export.optimized.is_none());
    // OK takes the settings found and closes the rows.
    set(&mut app, "optimizeK", json!(full as f64 * 0.8 / 1024.0));
    set(&mut app, "optimizeGo", json!(true));
    settle(&mut app);
    let found = app.web_export.optimized.clone().expect("the search ran");
    assert_eq!(found["achieved"], json!(true), "{found}");
    let quality = fields(&app)["quality"].as_u64().unwrap();
    assert!(quality < 100, "{found}");
    assert!(fields(&app).get("optimizeK").is_none(), "the rows closed");
    assert!(optimized_result(&app.web_export.optimized, &art_key(&app)).is_some());
    // Save writes the size found.
    save(&mut app);
    let written = std::fs::read(dir.join("Untitled-1.jpg")).unwrap();
    assert_eq!(written.len() as u64, found["bytes"].as_u64().unwrap());
    assert!(written.len() as u64 <= found["targetBytes"].as_u64().unwrap());
    assert_eq!(app.session.last_web_settings().quality as u64, quality);
    // The result drops when the art it measured changes: a new shape, a cleared selection (which
    // decides the slices Save writes, and leaves the revision as it is) or a selected slice.
    let plant = |app: &mut VectorcraftApp| {
        let mut current = found.clone();
        current["art"] = json!(art_key(app));
        app.web_export.optimized = Some(current);
        frame(app);
        assert!(app.web_export.optimized.is_some(), "the result holds for the art it measured");
    };
    app.run("file.saveForWeb", json!({})).unwrap();
    plant(&mut app);
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    frame(&mut app);
    assert!(app.web_export.optimized.is_none(), "a new shape drops it");
    plant(&mut app);
    let st = app.session.active_mut().unwrap();
    assert!(!st.selection.objects.is_empty());
    st.selection.objects.clear();
    frame(&mut app);
    assert!(app.web_export.optimized.is_none(), "a cleared selection drops it");
    plant(&mut app);
    let st = app.session.active_mut().unwrap();
    let any = st.doc.layers[0].id;
    st.selection.slices.push(any);
    frame(&mut app);
    assert!(app.web_export.optimized.is_none(), "a selected slice drops it");
}

#[test]
fn optimize_to_file_size_starts_at_the_size_of_the_slices_files() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize-slices");
    let (mut app, _) = app(&dir, false);
    app.run("object.slice.create", json!({"x": 0, "y": 0, "width": 30, "height": 20})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    settle(&mut app);
    let slices = app.session.execute("webExport.optimizeToFileSize", &json!({"format": "png24", "desiredK": 1048576})).unwrap();
    let whole = encoded_bytes(&mut app, json!({"format": "png24"}));
    let b = slices["bytes"].as_u64().unwrap();
    assert_ne!(b, whole, "{slices}");
    set(&mut app, "optimizeOpen", json!(true));
    settle(&mut app);
    let f = fields(&app);
    // The size asked for holds the files measured, and is not the whole image's.
    let bytes = f["optimizeK"].as_f64().unwrap() * 1024.0;
    assert!(bytes >= b as f64 && bytes < b as f64 + 2.0, "{bytes} for {b}");
    assert!((bytes - whole as f64).abs() > 1.0);
    assert_eq!(f["optimizeUse"], json!("allSlices"));
}

#[test]
fn optimize_to_file_size_starts_at_the_size_of_the_art_as_it_is_now() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize-revision");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    settle(&mut app);
    let before = encoded_bytes(&mut app, json!({"format": "png24"}));
    // The art changes after the panes were drawn and before the button's click is handled.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#3366ff"})).unwrap();
    let after = encoded_bytes(&mut app, json!({"format": "png24"}));
    assert_ne!(before, after);
    set(&mut app, "optimizeOpen", json!(true));
    settle(&mut app);
    let bytes = fields(&app)["optimizeK"].as_f64().unwrap() * 1024.0;
    assert!(bytes >= after as f64 && bytes < after as f64 + 2.0, "{bytes} for {after}");
}
