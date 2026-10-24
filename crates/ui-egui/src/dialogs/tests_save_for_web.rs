//! Save for Web (Legacy), drawn headlessly and driven as an agent drives it.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use super::tests_export::{frame, kind, set};
use super::*;
use crate::Services;

type Log = Rc<RefCell<Vec<String>>>;

/// An app with a 60×40 pt document (an ellipse), whose save panel answers inside `dir`; `web`
/// downloads instead (into the log).
fn app(dir: &std::path::Path, web: bool) -> (VectorcraftApp, Log) {
    let log = Log::default();
    let (l1, l2) = (log.clone(), log.clone());
    let dir = dir.to_string_lossy().replace('\\', "/");
    let mut services = Services {
        pick_save: Some(Box::new(move |p: &crate::FilePick| Some(format!("{dir}/{}", p.name)))),
        open_url: Some(Box::new(move |u: &str| l1.borrow_mut().push(format!("open {u}")))),
        ..Default::default()
    };
    if web {
        services.download = Some(Box::new(move |name: &str, b: &[u8]| {
            l2.borrow_mut().push(format!("download {name} {}", b.len()));
            Ok(())
        }));
    }
    let mut app = VectorcraftApp::new(Session::new(), services);
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
        let initial = app.ui.dialog.as_ref().unwrap().fields["artboard"].clone();
        set(&mut app, "format", json!("png24"));
        frame(&mut app);
        let result = confirm(&mut app).unwrap();
        let image = image::open(result["path"].as_str().unwrap()).unwrap().to_rgba8();
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
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["artboard"], json!(active));
        assert_eq!(app.session.last_web_settings().to_json(), saved, "opening does not overwrite the remembered settings");
        frame(&mut app);
        let result = confirm(&mut app).unwrap();
        let image = image::open(result["path"].as_str().unwrap()).unwrap().to_rgba8();
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
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    for (index, size, color) in [(0, [60, 40], [255, 0, 0, 255]), (1, [32, 24], [0, 0, 255, 255])] {
        set(&mut app, "artboard", json!(index));
        let mut out = ctx.run_ui(Default::default(), |ui| show(&mut app, ui.ctx()));
        let previews: Vec<_> = out
            .textures_delta
            .set
            .values()
            .flatten()
            .filter(|delta| delta.image.size() == size)
            .map(|delta| {
                let egui::ImageData::Color(image) = &delta.image;
                image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(previews.len(), 2, "original and optimized, artboard {index}");
        assert!(previews.iter().all(|p| p.as_chunks::<4>().0.iter().all(|pixel| *pixel == color)));
        out.textures_delta.clear();
        // Save through the same dialog while keeping the egui context (and its cache) alive.
        let result = confirm(&mut app).unwrap();
        let image = image::open(result["path"].as_str().unwrap()).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (size[0] as u32, size[1] as u32));
        assert!(previews.iter().all(|p| *p == image.as_raw().as_slice()));
        if index == 0 {
            app.run("file.saveForWeb", json!({})).unwrap();
        }
    }
}

#[test]
fn choosing_the_second_artboard_in_the_dropdown_exports_its_pixels() {
    use egui_kittest::kittest::{By, Queryable};
    let dir = vectorcraft_testkit::temp_dir("sfw-dropdown-artboard");
    let mut app = two_artboards(&dir);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 1200.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);
    h.get(By::new().role(egui::accesskit::Role::ComboBox).value("Artboard 1")).click();
    h.run();
    h.get_by_label("Artboard 2").click();
    h.run();
    assert_eq!(h.state().ui.dialog.as_ref().unwrap().fields["artboard"], json!(1));
    let result = confirm(h.state_mut()).unwrap();
    let image = image::open(result["path"].as_str().unwrap()).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (32, 24));
    assert_eq!(image.get_pixel(8, 8).0, [0, 0, 255, 255]);
}

#[test]
fn opening_uses_the_active_artboard_in_a_multiselection_and_preserves_other_settings() {
    let dir = vectorcraft_testkit::temp_dir("sfw-settings-artboard");
    let mut app = two_artboards(&dir);
    app.run("webExport.settings", json!({"preset": "JPEG, quality 50", "artboard": 0, "quality": 73, "width": 96, "metadata": "all"})).unwrap();
    let saved = app.session.last_web_settings().to_json();
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    app.run("tool.setOption", json!({"key": "selected", "value": [1, 0]})).unwrap();
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(crate::panels::artboards::picked(&app, 2), [0, 1]);
    app.run("file.saveForWeb", json!({})).unwrap();
    let fields = &app.ui.dialog.as_ref().unwrap().fields;
    let mut expected = saved.clone();
    expected["artboard"] = json!(1);
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&fields[key], value, "{key}");
    }
    assert_eq!(app.session.last_web_settings().to_json(), saved, "opening does not overwrite the remembered settings");
    app.ui.dialog = None;
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    assert_eq!(app.view().unwrap().artboard, 1);
    app.run("file.saveForWeb", json!({})).unwrap();
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&app.ui.dialog.as_ref().unwrap().fields[key], value, "Selection tool keeps {key}");
    }
    assert_eq!(app.session.last_web_settings().to_json(), saved, "opening still does not overwrite the remembered settings");
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
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["artboard"], json!(0));
    frame(&mut app);
    let result = confirm(&mut app).unwrap();
    let image = image::open(result["path"].as_str().unwrap()).unwrap().to_rgba8();
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
            assert!(app.ui.dialog.is_none());
        }
    }
}

#[test]
fn every_view_and_format_draws() {
    let dir = vectorcraft_testkit::temp_dir("sfw-views");
    let (mut app, _) = app(&dir, false);
    assert_eq!(crate::menus::shortcut_of("file.saveForWeb"), Some("Cmd+Alt+Shift+S"));
    assert!(crate::menus::enabled(&app, "file.saveForWeb"));
    assert_eq!((spec(save_for_web::KIND).ok_label.unwrap())(&app), "Save…");
    app.run("file.saveForWeb", json!({})).unwrap();
    assert_eq!(kind(&app), Some(save_for_web::KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("format"), d.str("__view"), d.f64("colors", 0.0)), ("gif".into(), "2up".into(), 128.0));
    for view in ["original", "optimized", "2up", "4up"] {
        set(&mut app, "__view", json!(view));
        for format in ["gif", "jpg", "png8", "png24"] {
            set(&mut app, "format", json!(format));
            frame(&mut app);
            assert_eq!(kind(&app), Some(save_for_web::KIND), "{view} {format}");
        }
    }
    // Fit, other speeds, a colour selected in the table, a bad value (an error in the panes).
    set(&mut app, "format", json!("gif"));
    set(&mut app, "__zoom", json!("fit"));
    set(&mut app, "__kbps", json!(1024));
    set(&mut app, "__color", json!("#cc3300"));
    frame(&mut app);
    set(&mut app, "percent", json!(0));
    frame(&mut app);
    assert_eq!(kind(&app), Some(save_for_web::KIND));
}

#[test]
fn save_remembers_and_writes_and_done_only_remembers() {
    let dir = vectorcraft_testkit::temp_dir("sfw-save");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    // A preset field loads the preset; the keys set after it win.
    set(&mut app, "preset", json!("JPEG, quality 50"));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("format"), d.f64("quality", 0.0)), ("jpg".into(), 50.0));
    assert!(!d.fields.contains_key("preset"));
    set(&mut app, "quality", json!(70));
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let path = dir.join("Untitled-1.jpg");
    assert_eq!(r["path"], json!(path.to_string_lossy().replace('\\', "/")));
    assert!(std::fs::read(&path).unwrap().starts_with(&[0xFF, 0xD8]));
    assert_eq!(app.session.last_web_settings().quality, 70, "remembered");
    // It reopens on them; Done remembers without writing.
    app.run("file.saveForWeb", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("quality", 0.0), 70.0);
    set(&mut app, "format", json!("png24"));
    set(&mut app, "discard", json!(true));
    assert_eq!(confirm(&mut app).unwrap()["remembered"], true);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.last_web_settings().format, vectorcraft_engine::cmd::webexport::WebFormat::Png24);
    assert!(!dir.join("Untitled-1.png").exists());
    // Bad settings keep the dialog open, with nothing written.
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "colorTable", json!({"locked": ["nope"]}));
    assert!(confirm(&mut app).is_err());
    assert!(!dir.join("Untitled-1.png").exists());
}

#[test]
fn html_output_with_slices_saves_the_page_and_images() {
    let dir = vectorcraft_testkit::temp_dir("sfw-html");
    let (mut app, _) = app(&dir, false);
    app.run("object.slice.create", json!({"x": 0, "y": 0, "width": 30, "height": 20})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().bool("__hasSlices"));
    set(&mut app, "output", json!("html"));
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    let n = app.session.execute("slice.list", &json!({})).unwrap()["slices"].as_array().unwrap().len();
    assert_eq!(r["files"].as_array().unwrap().len(), n + 1);
    assert!(std::fs::read_to_string(dir.join("Untitled-1.html")).unwrap().contains("images/Untitled-1_01.gif"));
    assert!(dir.join("images/Untitled-1_01.gif").exists());
}

#[test]
fn the_web_downloads_and_has_no_browser_preview() {
    let dir = vectorcraft_testkit::temp_dir("sfw-web");
    let (mut app, log) = app(&dir, true);
    assert_eq!((spec(save_for_web::KIND).ok_label.unwrap())(&app), "Download");
    let r = app.run("file.saveForWeb", json!({"format": "png8", "output": "html"})).unwrap();
    assert_eq!(r["files"], json!(["Untitled-1.html", "images/Untitled-1.png"]));
    let log = log.borrow().clone();
    assert!(log[0].starts_with("download Untitled-1.html ") && log[1].starts_with("download Untitled-1.png "), "{log:?}");
    assert!(app.run("file.saveForWeb.browser", json!({})).is_err());
}

#[test]
fn preview_in_browser_opens_the_page() {
    let dir = vectorcraft_testkit::temp_dir("sfw-browser");
    let (mut app, log) = app(&dir, false);
    let r = app.run("file.saveForWeb.browser", json!({"format": "jpg"})).unwrap();
    let page = r["path"].as_str().unwrap().to_string();
    assert!(page.ends_with("/Untitled-1.html"));
    assert!(std::fs::read_to_string(&page).unwrap().contains("images/Untitled-1.jpg"));
    let log = log.borrow();
    assert!(log.last().unwrap().starts_with("open file://") && log.last().unwrap().ends_with("Untitled-1.html"), "{log:?}");
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
    let earlier = save_for_web::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    std::fs::create_dir_all(format!("{earlier}/images")).unwrap();
    std::fs::write(format!("{earlier}/images/Untitled-1.gif"), b"GIF89a").unwrap();
    std::fs::write(root.join("Untitled-1.html"), b"<html>").unwrap();
    // The folder and the page are younger than an hour, so both stay.
    let kept = save_for_web::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    assert!(std::path::Path::new(&earlier).exists() && root.join("Untitled-1.html").exists());
    // With a `keep` of zero every entry is old enough: both preview folders go, and the page, which
    // no preview made, stays beside the new folder.
    let next = save_for_web::preview_folder(&root, std::time::Duration::ZERO).unwrap();
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
    let earlier = save_for_web::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    let folders = ["notes", "12-", "-3", "12-3a", "a12-3", "12_3"];
    for name in folders {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    let files = ["Untitled-1.html", "12-3"];
    for name in files {
        std::fs::write(root.join(name), b"kept").unwrap();
    }
    let next = save_for_web::preview_folder(&root, std::time::Duration::ZERO).unwrap();
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
    let e = save_for_web::preview_folder(&root, keep).unwrap_err();
    assert!(e.ends_with(": it is not a folder"), "{e}");
    let dir = save_for_web::preview_folder_in(&tmp, keep).unwrap();
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
    let first = save_for_web::preview_folder_in(&tmp, hour).unwrap();
    let second = save_for_web::preview_folder_in(&tmp, hour).unwrap();
    assert!(std::path::Path::new(&first).is_dir() && std::path::Path::new(&second).is_dir(), "{first} {second}");
    let others = ["VectorCraft Save for Web 7-7", "VectorCraft Save for Web 7-7.html", "VectorCraft Save for Web notes"];
    std::fs::write(tmp.join(others[0]), b"kept").unwrap();
    std::fs::write(tmp.join(others[1]), b"kept").unwrap();
    std::fs::create_dir_all(tmp.join(others[2])).unwrap();
    let third = save_for_web::preview_folder_in(&tmp, std::time::Duration::ZERO).unwrap();
    assert_eq!(std::path::Path::new(&third).parent(), Some(tmp.as_path()), "{third}");
    assert!(!std::path::Path::new(&first).exists() && !std::path::Path::new(&second).exists(), "{first} {second}");
    std::fs::remove_file(&root).unwrap();
    let dir = save_for_web::preview_folder_in(&tmp, std::time::Duration::ZERO).unwrap();
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
    let dir = save_for_web::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    assert_eq!((mode(&root), mode(&dir)), (0o700, 0o700));
    // Beside a root it can't use, too.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
    let shared = tmp.join("VectorCraft Save for Web");
    let _ = std::fs::remove_dir_all(&shared);
    std::fs::rename(&root, &shared).unwrap();
    let own = save_for_web::preview_folder_in(&tmp, std::time::Duration::from_secs(3600)).unwrap();
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
    let e = save_for_web::preview_folder(&root, keep).unwrap_err();
    assert!(e.ends_with(": it is a link"), "{e}");
    let dir = save_for_web::preview_folder_in(&tmp, keep).unwrap();
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
        let earlier = save_for_web::preview_folder(&root, keep).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(bits)).unwrap();
        let e = save_for_web::preview_folder(&root, keep).unwrap_err();
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
    save_for_web::preview_folder(&root, std::time::Duration::from_secs(3600)).unwrap();
    symlink(&outside, root.join("9-9")).unwrap();
    save_for_web::preview_folder(&root, std::time::Duration::ZERO).unwrap();
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
    let theirs = save_for_web::preview_folder(&root, keep).unwrap();
    let other = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&root).unwrap()) + 1;
    if std::os::unix::fs::chown(&theirs, Some(other), None).is_err() {
        eprintln!("skipped: only root can give a folder to another user");
        return;
    }
    save_for_web::preview_folder(&root, keep).unwrap();
    assert!(std::path::Path::new(&theirs).is_dir());
    std::os::unix::fs::chown(&root, Some(other), None).unwrap();
    let e = save_for_web::preview_folder(&root, keep).unwrap_err();
    assert!(e.ends_with(": it belongs to another user"), "{e}");
}

/// A folder that an earlier process with the same id left, younger than `keep`, is not reused:
/// the preview makes a new, empty one.
#[test]
fn a_browser_preview_skips_a_folder_an_earlier_process_left() {
    let root = vectorcraft_testkit::temp_dir("sfw-browser-reused").join("previews");
    let _ = std::fs::remove_dir_all(&root);
    let keep = std::time::Duration::from_secs(3600);
    let earlier = save_for_web::preview_folder(&root, keep).unwrap();
    let count: u64 = earlier.rsplit('-').next().unwrap().parse().unwrap();
    // The next 200 names, as an earlier process with the same id would have left them.
    for n in count + 1..=count + 200 {
        let left = root.join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(left.join("images")).unwrap();
        std::fs::write(left.join("Untitled-1.html"), b"<html>").unwrap();
    }
    let next = save_for_web::preview_folder(&root, keep).unwrap();
    assert_ne!(next, earlier);
    assert_eq!(std::fs::read_dir(&next).unwrap().count(), 0, "{next}");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 202);
}

#[test]
fn saving_says_in_the_status_bar_when_an_image_could_not_be_decoded() {
    const NOTE: &str = "with 1 note(s): an image couldn't be decoded and is missing from the picture";
    // A PNG whose header reads but whose pixel data is cut short: the renderer draws nothing for it.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png.truncate(idat + 8);
    let place = |app: &mut VectorcraftApp| {
        app.run("file.place", json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [30, 20]})).unwrap()
    };
    let dir = vectorcraft_testkit::temp_dir("sfw-undecoded");
    let (mut desktop, _) = app(&dir, false);
    desktop.run("file.saveForWeb", json!({"format": "png24"})).unwrap();
    assert!(!desktop.ui.status.contains("note"), "{}", desktop.ui.status);
    place(&mut desktop);
    for p in [json!({"format": "png24"}), json!({"format": "jpg", "output": "html"})] {
        desktop.status("");
        desktop.run("file.saveForWeb", p.clone()).unwrap();
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

#[test]
fn colour_table_buttons_edit_the_lists() {
    let mut d = Dialog::new(save_for_web::KIND, json!({}));
    save_for_web::toggle(&mut d, "locked", "#cc3300", true);
    save_for_web::toggle(&mut d, "webShift", "#123456", true);
    save_for_web::toggle(&mut d, "locked", "#CC3300", true);
    assert_eq!(d.fields["colorTable"], json!({"locked": ["#cc3300"], "webShift": ["#123456"]}));
    save_for_web::toggle(&mut d, "locked", "#cc3300", false);
    save_for_web::set_table(&mut d, "sort", json!("hue"));
    assert_eq!(d.fields["colorTable"], json!({"locked": [], "webShift": ["#123456"], "sort": "hue"}));
}
