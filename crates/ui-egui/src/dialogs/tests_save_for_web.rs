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

/// `ui.dialogReset` puts the remembered settings back (the defaults when none are remembered) on
/// the document's active artboard, drops a pending preset, the preset name being typed and the
/// selected color, keeps the view, zoom, speed and path, writes and remembers nothing, and leaves
/// the dialog open; settings changed after it are remembered as usual.
#[test]
fn reset_puts_the_remembered_settings_back() {
    let dir = vectorcraft_testkit::temp_dir("sfw-reset");
    let (mut app, _) = app(&dir, false);
    let defaults = vectorcraft_engine::cmd::webexport::WebSettings::default().to_json();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [
        ("format", json!("jpg")),
        ("quality", json!(20)),
        ("colorTable", json!({"locked": ["nope"]})),
        ("preset", json!("PNG-24")),
        ("__presetName", json!("Mine")),
        ("__color", json!("#cc3300")),
        ("__zoom", json!("200")),
        ("__kbps", json!(1024)),
        ("path", json!("elsewhere.png")),
    ] {
        set(&mut app, k, v);
    }
    app.run("ui.dialogReset", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().expect("Reset leaves the dialog open");
    for (k, v) in defaults.as_object().unwrap() {
        let want = if k == "artboard" { &json!(0) } else { v };
        assert_eq!(d.fields.get(k), Some(want), "{k}");
    }
    for k in ["preset", "__presetName", "__color"] {
        assert!(!d.fields.contains_key(k), "{k}");
    }
    assert_eq!((d.str("__zoom"), d.f64("__kbps", 0.0), d.str("path")), ("200".into(), 1024.0, "elsewhere.png".into()));
    assert_eq!(app.session.last_web_settings().to_json(), defaults, "Reset remembers nothing");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "Reset writes nothing");
    set(&mut app, "quality", json!(55));
    set(&mut app, "discard", json!(true));
    confirm(&mut app).unwrap();
    assert_eq!(app.session.last_web_settings().quality, 55);

    // Remembered on another document's second artboard: Reset keeps this document's active one.
    let mut app = two_artboards(&vectorcraft_testkit::temp_dir("sfw-reset-artboard"));
    app.run("view.goToArtboard", json!({"index": 1})).unwrap();
    app.run("webExport.settings", json!({"artboard": 1, "quality": 40})).unwrap();
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "quality", json!(90));
    set(&mut app, "artboard", json!(3));
    app.run("ui.dialogReset", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields["artboard"].clone(), d.f64("quality", 0.0)), (json!(0), 40.0));
}

/// A Done that failed on a bad setting leaves `discard` in the dialog; Reset drops it, so Save…
/// after Reset writes the file instead of only remembering the settings.
#[test]
fn reset_drops_a_done_that_failed() {
    let dir = vectorcraft_testkit::temp_dir("sfw-reset-done");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "colorTable", json!({"locked": ["nope"]}));
    set(&mut app, "discard", json!(true));
    assert!(confirm(&mut app).is_err());
    app.run("ui.dialogReset", json!({})).unwrap();
    assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("discard"));
    let r = confirm(&mut app).unwrap();
    let path = r["path"].as_str().expect("Save… writes a file");
    assert!(std::path::Path::new(path).exists(), "{path}");
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
    assert!(crate::menus::enabled(&app, "file.saveForWeb"));
    assert!(!crate::menus::enabled(&app, "file.saveForWeb.browser"), "the command palette and menus show it disabled on the web");
}

#[test]
fn preview_in_browser_stays_in_its_folder_and_is_the_persons_to_open() {
    for (title, name) in
        [("Poster", "Poster"), ("../..", "Untitled"), ("..", "Untitled"), ("a/..", "Untitled"), ("..\\x", "Untitled"), (" ", "Untitled")]
    {
        assert_eq!(save_for_web::preview_name(title), name, "{title}");
    }
    let dir = vectorcraft_testkit::temp_dir("sfw-browser-automation");
    let (mut app, log) = app(&dir, false);
    app.ui.answering_control = true;
    assert!(app.run("file.saveForWeb.browser", json!({"format": "jpg"})).unwrap_err().contains("opens another app"));
    app.ui.answering_control = false;
    assert!(!log.borrow().iter().any(|l| l.starts_with("open ")), "{:?}", log.borrow());
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

/// A preview whose export fails opens nothing and removes the folder made for it.
#[test]
fn a_failed_browser_preview_removes_its_folder() {
    let tmp = vectorcraft_testkit::temp_dir("sfw-browser-failed");
    save_for_web::TEST_TMP.set(Some(tmp.clone()));
    let (mut app, log) = app(&tmp, false);
    assert!(app.run("file.saveForWeb.browser", json!({"format": "bogus"})).is_err());
    assert!(!log.borrow().iter().any(|l| l.starts_with("open ")), "{:?}", log.borrow());
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

/// A click selects one color, a Shift-click the range from the last one clicked, a Ctrl-click
/// (Command-click) adds or removes one; Delete Color deletes the whole selection in one edit,
/// takes it out of the other lists, fixes the palette and refuses to delete every color.
#[test]
fn colour_table_selection_and_delete_color() {
    let mut d = Dialog::new(save_for_web::KIND, json!({}));
    let order: Vec<String> = ["#000000", "#111111", "#222222", "#333333"].map(String::from).to_vec();
    let sel = |d: &Dialog| save_for_web::selection(d);
    assert!(sel(&d).is_empty());
    save_for_web::pick(&mut d, &order, "#111111", false, false);
    assert_eq!(sel(&d), ["#111111"]);
    save_for_web::pick(&mut d, &order, "#333333", true, false);
    assert_eq!((sel(&d), d.str("__color")), (vec!["#111111".to_string(), "#222222".into(), "#333333".into()], "#111111".into()));
    save_for_web::pick(&mut d, &order, "#222222", false, true);
    assert_eq!(sel(&d), ["#111111", "#333333"]);
    save_for_web::pick(&mut d, &order, "#000000", false, true);
    assert_eq!((sel(&d), d.str("__color")), (vec!["#111111".to_string(), "#333333".into(), "#000000".into()], "#000000".into()));
    save_for_web::pick(&mut d, &order, "#000000", false, true);
    assert_eq!((sel(&d), d.str("__color")), (vec!["#111111".to_string(), "#333333".into()], "#333333".into()));
    // A Shift-click back from the anchor.
    save_for_web::pick(&mut d, &order, "#222222", false, false);
    save_for_web::pick(&mut d, &order, "#000000", true, false);
    assert_eq!(sel(&d), ["#000000", "#111111", "#222222"]);
    // `__color` alone (an agent's selection) is the selection too.
    d.fields.remove("__colors");
    d.fields.insert("__color".into(), json!("#222222"));
    assert_eq!(sel(&d), ["#222222"]);

    save_for_web::toggle(&mut d, "locked", "#222222", true);
    save_for_web::toggle(&mut d, "locked", "#000000", true);
    save_for_web::toggle(&mut d, "webShift", "#333333", true);
    save_for_web::pick(&mut d, &order, "#333333", false, true);
    save_for_web::delete_colors(&mut d, &order).unwrap();
    assert_eq!(d.fields["colorTable"], json!({"locked": ["#000000"], "webShift": [], "customPalette": order, "deleted": ["#222222", "#333333"]}));
    assert!(!d.fields.contains_key("__colors") && !d.fields.contains_key("__color") && sel(&d).is_empty());
    // Nothing selected: nothing deleted.
    let before = d.fields.clone();
    save_for_web::delete_colors(&mut d, &order).unwrap();
    assert_eq!(d.fields, before);
    // The colors left can't all go: refused, nothing changed.
    save_for_web::pick(&mut d, &order, "#000000", false, false);
    save_for_web::pick(&mut d, &order, "#111111", false, true);
    let before = d.fields.clone();
    assert_eq!(save_for_web::delete_colors(&mut d, &order), Err(vectorcraft_engine::cmd::webexport::KEEP_ONE.to_string()));
    assert_eq!(d.fields, before);
    // Nor can the last color that is not mapped to transparent.
    save_for_web::toggle(&mut d, "transparent", "#111111", true);
    save_for_web::pick(&mut d, &order, "#000000", false, false);
    let before = d.fields.clone();
    assert_eq!(save_for_web::delete_colors(&mut d, &order), Err(vectorcraft_engine::cmd::webexport::KEEP_ONE.to_string()));
    assert_eq!(d.fields, before);
    // Restore Generated Table.
    save_for_web::restore_table(&mut d);
    assert_eq!((&d.fields["colorTable"]["deleted"], &d.fields["colorTable"]["customPalette"]), (&json!([]), &json!([])));
}

/// A selection written another way (`#00F`) is deleted as the table writes it, so deleting the
/// table's only color is refused with nothing changed; a spelling with alpha names no table color.
#[test]
fn delete_color_matches_a_selection_written_another_way() {
    let mut d = Dialog::new(save_for_web::KIND, json!({}));
    let shown: Vec<String> = ["#0000ff"].map(String::from).to_vec();
    d.fields.insert("__colors".into(), json!(["#00F", "#0000FF"]));
    assert_eq!(save_for_web::selected_in(&d, &shown), ["#0000ff"]);
    let before = d.fields.clone();
    assert_eq!(save_for_web::delete_colors(&mut d, &shown), Err(vectorcraft_engine::cmd::webexport::KEEP_ONE.to_string()));
    assert_eq!(d.fields, before);
    d.fields.insert("__colors".into(), json!(["#0000ff80"]));
    assert!(save_for_web::selected_in(&d, &shown).is_empty());
}

/// A Ctrl-click on a selected color takes out every selection entry that names it, however each is
/// written (`#00F`, `#0000ff`).
#[test]
fn command_click_deselects_every_spelling_of_a_color() {
    let order: Vec<String> = ["#0000ff", "#ff0000"].map(String::from).to_vec();
    let mut d = Dialog::new(save_for_web::KIND, json!({}));
    d.fields.insert("__colors".into(), json!(["#ff0000", "#00F", "#0000ff"]));
    save_for_web::pick(&mut d, &order, "#0000ff", false, true);
    assert_eq!(save_for_web::selection(&d), ["#ff0000"]);
    assert_eq!(d.fields["__color"], json!("#ff0000"));
}

/// Delete Color compares the table's lists as colors: with blue mapped to transparent as `#00f`,
/// deleting red (selected as `#f00`), the only other color, is refused with nothing changed, and a
/// deleted color written another way in `locked` (`blue`, ` #0000FF `) is taken out of the list.
#[test]
fn delete_color_compares_lists_written_another_way() {
    let shown: Vec<String> = ["#0000ff", "#ff0000"].map(String::from).to_vec();
    let mut d = Dialog::new(save_for_web::KIND, json!({"colorTable": {"transparent": ["#00f"]}}));
    d.fields.insert("__color".into(), json!("#f00"));
    let before = d.fields.clone();
    assert_eq!(save_for_web::delete_colors(&mut d, &shown), Err(vectorcraft_engine::cmd::webexport::KEEP_ONE.to_string()));
    assert_eq!(d.fields, before);
    let mut d = Dialog::new(save_for_web::KIND, json!({"colorTable": {"locked": ["blue", " #0000FF ", "#ff0000"]}}));
    d.fields.insert("__color".into(), json!("#00F"));
    save_for_web::delete_colors(&mut d, &shown).unwrap();
    assert_eq!(d.fields["colorTable"]["locked"], json!(["#ff0000"]));
    assert_eq!(d.fields["colorTable"]["deleted"], json!(["#0000ff"]));
}

/// Delete Color takes only selected colors the table shows: a selection the table no longer holds
/// (after a change of reduction or color count, or set through `ui.dialog.set`) deletes nothing,
/// fixes no palette and is cleared, and a selection partly in the table deletes only what it shows.
#[test]
fn delete_color_skips_selected_colors_the_table_does_not_show() {
    let mut d = Dialog::new(save_for_web::KIND, json!({}));
    let shown: Vec<String> = ["#000000", "#111111"].map(String::from).to_vec();
    let table = |d: &Dialog, key: &str| d.fields.get("colorTable").and_then(|t| t.get(key)).cloned();
    d.fields.insert("__colors".into(), json!(["#ABCDEF"]));
    d.fields.insert("__color".into(), json!("#abcdef"));
    assert!(save_for_web::selected_in(&d, &shown).is_empty());
    save_for_web::delete_colors(&mut d, &shown).unwrap();
    assert_eq!((table(&d, "customPalette"), table(&d, "deleted")), (None, None));
    assert!(!d.fields.contains_key("__colors") && !d.fields.contains_key("__color"));
    d.fields.insert("__colors".into(), json!(["#abcdef", "#111111"]));
    assert_eq!(save_for_web::selected_in(&d, &shown), ["#111111"]);
    save_for_web::delete_colors(&mut d, &shown).unwrap();
    assert_eq!((table(&d, "customPalette"), table(&d, "deleted")), (Some(json!(shown)), Some(json!(["#111111"]))));
}

/// A shift by hand replaces a web shift and selects the color; a shift to the color itself takes it
/// away; Unshift All Colors clears both kinds; Delete Color takes a deleted color's shift away.
#[test]
fn colour_table_shift_unshift_and_unshift_all() {
    let mut d = Dialog::new(save_for_web::KIND, json!({}));
    save_for_web::toggle(&mut d, "webShift", "#123456", true);
    save_for_web::shift_color(&mut d, "#123456", "#FF8800").unwrap();
    assert_eq!(d.fields["colorTable"], json!({"webShift": [], "shifted": {"#123456": "#ff8800"}}));
    assert_eq!(save_for_web::selection(&d), ["#123456"]);
    // Not a color: nothing changes.
    let before = d.fields.clone();
    save_for_web::shift_color(&mut d, "nope", "#000000").unwrap();
    save_for_web::shift_color(&mut d, "#123456", "nope").unwrap();
    assert_eq!(d.fields, before);
    save_for_web::shift_color(&mut d, "#ABCDEF", "#00ff00").unwrap();
    save_for_web::shift_color(&mut d, "#123456", "#123456").unwrap();
    assert_eq!(d.fields["colorTable"]["shifted"], json!({"#abcdef": "#00ff00"}));
    save_for_web::toggle(&mut d, "webShift", "#222222", true);
    save_for_web::unshift_all_colors(&mut d);
    assert_eq!((&d.fields["colorTable"]["shifted"], &d.fields["colorTable"]["webShift"]), (&json!({}), &json!([])));
    let order: Vec<String> = ["#000000", "#222222"].map(String::from).to_vec();
    save_for_web::shift_color(&mut d, "#222222", "#ffffff").unwrap();
    save_for_web::delete_colors(&mut d, &order).unwrap();
    assert_eq!((&d.fields["colorTable"]["shifted"], &d.fields["colorTable"]["deleted"]), (&json!({}), &json!(["#222222"])));
}

/// A key or list entry written another way (`#F00`, upper case) still names its color: a new shift
/// of that color replaces it, taking the shift away removes it, and a web shift of it is taken away.
#[test]
fn colour_table_shift_matches_a_color_written_another_way() {
    let mut d = Dialog::new(save_for_web::KIND, json!({"colorTable": {"shifted": {"#F00": "#00ff00", "nope": "#000000"}, "webShift": ["#0000FF"]}}));
    save_for_web::shift_color(&mut d, "#ff0000", "#123456").unwrap();
    assert_eq!(d.fields["colorTable"]["shifted"], json!({"#ff0000": "#123456", "nope": "#000000"}));
    save_for_web::shift_color(&mut d, "#0000ff", "#654321").unwrap();
    assert_eq!(d.fields["colorTable"]["webShift"], json!([]));
    save_for_web::unshift(&mut d, "#FF0000");
    assert_eq!(d.fields["colorTable"]["shifted"], json!({"#0000ff": "#654321", "nope": "#000000"}));
}

/// With 256 colors shifted, shifting another color is refused and changes nothing; a shifted color
/// can still be shifted again or have its shift taken away.
#[test]
fn colour_table_shift_holds_at_most_256_colors() {
    let full: serde_json::Map<String, serde_json::Value> = (0..256).map(|i| (format!("#{i:06x}"), json!("#ffffff"))).collect();
    let mut d = Dialog::new(save_for_web::KIND, json!({"colorTable": {"shifted": full}, "__color": "#000001", "__colors": ["#000001"]}));
    let before = d.fields.clone();
    let e = save_for_web::shift_color(&mut d, "#123456", "#000000").unwrap_err();
    assert_eq!((e.as_str(), &d.fields), ("colorTable.shifted: at most 256 colours", &before));
    save_for_web::shift_color(&mut d, "#000005", "#000000").unwrap();
    assert_eq!(
        (d.fields["colorTable"]["shifted"]["#000005"].as_str(), d.fields["colorTable"]["shifted"].as_object().map(|m| m.len())),
        (Some("#000000"), Some(256))
    );
    save_for_web::shift_color(&mut d, "#000005", "#000005").unwrap();
    save_for_web::shift_color(&mut d, "#123456", "#000000").unwrap();
    assert_eq!(d.fields["colorTable"]["shifted"]["#123456"], "#000000");
}

/// Entries the engine drops (a color shifted to itself, a deleted color) do not count toward the
/// 256 shifts, and a new shift writes the map without them.
#[test]
fn colour_table_shift_skips_entries_the_engine_drops() {
    let mut full: serde_json::Map<String, serde_json::Value> = (0..255).map(|i| (format!("#{i:06x}"), json!(format!("#{i:06x}")))).collect();
    full.insert("#abcdef".into(), json!("#000000"));
    let mut d = Dialog::new(save_for_web::KIND, json!({"colorTable": {"shifted": full, "deleted": ["#ABCDEF"]}}));
    save_for_web::shift_color(&mut d, "#123456", "#000000").unwrap();
    assert_eq!(d.fields["colorTable"]["shifted"], json!({"#123456": "#000000"}));
    // A map the engine refuses (a value that is not a color, two keys naming one color) is kept
    // whole for the engine to refuse.
    for shifted in [json!({"nope": "NOPE"}), json!({"#abcdef": "nope"}), json!({"#f00": "#f00", "#FF0000": "#00ff00"})] {
        let mut d = Dialog::new(save_for_web::KIND, json!({"colorTable": {"shifted": shifted.clone(), "deleted": ["#abcdef"]}}));
        save_for_web::shift_color(&mut d, "#123456", "#000000").unwrap();
        let mut want = shifted.as_object().cloned().unwrap_or_default();
        want.insert("#123456".into(), json!("#000000"));
        assert_eq!(d.fields["colorTable"]["shifted"], serde_json::Value::Object(want), "{shifted}");
    }
}

/// One frame of `app` in `ctx`, 50 ms after the last.
fn tick(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let time = ctx.input(|i| i.time) + 0.05;
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1000.0)));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() }, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Double-click the color table's swatch of the source color `source`.
fn double_click_swatch(app: &mut VectorcraftApp, ctx: &egui::Context, source: &str) {
    for _ in 0..3 {
        tick(app, ctx, vec![]);
    }
    let pos = ctx.read_response(save_for_web::swatch_id(source)).unwrap_or_else(|| panic!("no swatch for {source}")).rect.center();
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    tick(app, ctx, vec![egui::Event::PointerMoved(pos), button(true)]);
    tick(app, ctx, vec![button(false)]);
    tick(app, ctx, vec![button(true)]);
    tick(app, ctx, vec![button(false)]);
    tick(app, ctx, vec![]);
}

/// The color the open Color Picker shows.
fn picker_color(app: &VectorcraftApp) -> Option<String> {
    let d = app.ui.dialog.as_ref()?;
    assert_eq!(d.kind, "colorPicker", "the Color Picker is open");
    d.fields.get("color").and_then(vectorcraft_engine::cmd::color_value).map(|c| c.to_hex())
}

/// A double-click on a swatch opens the Color Picker on its color. Cancel changes nothing; OK
/// shifts the color in the table and the saved file. A double-click on a shifted swatch opens the
/// picker on the source color, and Web Shift replaces the shift by hand.
#[test]
fn double_clicking_a_swatch_shifts_its_color() {
    let dir = vectorcraft_testkit::temp_dir("sfw-shift-color");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let shifted = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.fields["colorTable"]["shifted"].clone());
    double_click_swatch(&mut app, &ctx, "#0000ff");
    assert_eq!(picker_color(&app).as_deref(), Some("#0000ff"));
    crate::dialogs::cancel(&mut app);
    tick(&mut app, &ctx, vec![]);
    assert_eq!((kind(&app), shifted(&app)), (Some(save_for_web::KIND), Some(json!({}))), "Cancel shifts nothing");
    double_click_swatch(&mut app, &ctx, "#0000ff");
    set(&mut app, "color", json!("#00ff00"));
    confirm(&mut app).unwrap();
    tick(&mut app, &ctx, vec![]);
    assert_eq!((kind(&app), shifted(&app)), (Some(save_for_web::KIND), Some(json!({"#0000ff": "#00ff00"}))), "OK shifts the color");
    // The shifted swatch opens on its source color.
    double_click_swatch(&mut app, &ctx, "#0000ff");
    assert_eq!(picker_color(&app).as_deref(), Some("#0000ff"));
    crate::dialogs::cancel(&mut app);
    // Web Shift replaces the shift by hand.
    click_text(&mut app, "Web Shift");
    let t = app.ui.dialog.as_ref().unwrap().fields["colorTable"].clone();
    assert_eq!((&t["shifted"], &t["webShift"]), (&json!({}), &json!(["#0000ff"])));
    // Shifted again, the color reaches the saved file and is remembered.
    double_click_swatch(&mut app, &ctx, "#0000ff");
    set(&mut app, "color", json!("#00ff00"));
    confirm(&mut app).unwrap();
    tick(&mut app, &ctx, vec![]);
    let r = confirm(&mut app).unwrap();
    let image = image::open(r["path"].as_str().unwrap()).unwrap().to_rgba8();
    assert_eq!((image.get_pixel(50, 20).0, image.get_pixel(20, 14).0), ([0, 255, 0, 255], [204, 51, 0, 255]));
    let remembered = app.session.last_web_settings().color_table;
    assert_eq!((remembered.shifted.get("#0000ff").map(String::as_str), remembered.web_shift.len()), (Some("#00ff00"), 0));
}

/// `ui.dialog.set` accepts `__editColor`, and the next frame opens the Color Picker on that color,
/// whose OK shifts it.
#[test]
fn dialog_set_edit_color_opens_the_color_picker() {
    let dir = vectorcraft_testkit::temp_dir("sfw-edit-color-set");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("gif"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (req, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"field": "__editColor", "value": "#0000ff"}));
    let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], true, "{r}");
    tick(&mut app, &ctx, vec![]);
    assert_eq!(picker_color(&app).as_deref(), Some("#0000ff"));
    set(&mut app, "color", json!("#00ff00"));
    confirm(&mut app).unwrap();
    tick(&mut app, &ctx, vec![]);
    let shifted = app.ui.dialog.as_ref().map(|d| d.fields["colorTable"]["shifted"].clone());
    assert_eq!((kind(&app), shifted), (Some(save_for_web::KIND), Some(json!({"#0000ff": "#00ff00"}))));
}

/// Done right after the Color Picker's OK, before the dialog draws again, remembers the shift.
#[test]
fn done_right_after_the_color_picker_remembers_the_shift() {
    let dir = vectorcraft_testkit::temp_dir("sfw-shift-done");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("gif"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set(&mut app, "__editColor", json!("#0000ff"));
    tick(&mut app, &ctx, vec![]);
    assert_eq!(picker_color(&app).as_deref(), Some("#0000ff"));
    set(&mut app, "color", json!("#00ff00"));
    confirm(&mut app).unwrap();
    set(&mut app, "discard", json!(true));
    confirm(&mut app).unwrap();
    let remembered = app.session.last_web_settings().color_table;
    assert_eq!(remembered.shifted.get("#0000ff").map(String::as_str), Some("#00ff00"));
}

/// Lock, Transparent and Web Shift are off while the selected color (`__color`) is one the table
/// no longer shows, so a click adds it to no list; with a color the table shows, Lock locks it.
#[test]
fn table_buttons_skip_a_selected_color_the_table_does_not_show() {
    let dir = vectorcraft_testkit::temp_dir("sfw-stale-anchor");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    let table = |app: &VectorcraftApp| app.ui.dialog.as_ref().and_then(|d| d.fields.get("colorTable").cloned()).unwrap_or_default();
    let has = |app: &VectorcraftApp, list: &str, c: &str| table(app)[list].as_array().is_some_and(|l| l.iter().any(|v| v == c));
    set(&mut app, "__color", json!("#abcdef"));
    for label in ["Lock", "Transparent", "Web Shift"] {
        click_text(&mut app, label);
    }
    for list in ["locked", "transparent", "webShift"] {
        assert!(!has(&app, list, "#abcdef"), "{list}: {}", table(&app));
    }
    set(&mut app, "__color", json!("#0000ff"));
    click_text(&mut app, "Lock");
    assert!(has(&app, "locked", "#0000ff"), "{}", table(&app));
}

/// A list entry written another way (`#00F`) names the selected color: the buttons show Unlock,
/// Unshift and Restore, and a click takes the color out of the list.
#[test]
fn table_buttons_match_a_color_written_another_way() {
    let dir = vectorcraft_testkit::temp_dir("sfw-short-hex");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    let table = |app: &VectorcraftApp| app.ui.dialog.as_ref().and_then(|d| d.fields.get("colorTable").cloned()).unwrap_or_default();
    for (list, label) in [("locked", "Unlock"), ("webShift", "Unshift"), ("transparent", "Restore")] {
        set(&mut app, "colorTable", json!({ list: ["#00F"] }));
        set(&mut app, "__color", json!("#0000ff"));
        click_text(&mut app, label);
        assert_eq!(table(&app)[list], json!([]), "{list}");
    }
    // Selected as `#00F`, a click adds the color as the table writes it.
    set(&mut app, "colorTable", json!({}));
    set(&mut app, "__color", json!("#00F"));
    click_text(&mut app, "Lock");
    assert_eq!(table(&app)["locked"], json!(["#0000ff"]));
    // A spelling with alpha names no table color: the buttons stay off and nothing is added.
    set(&mut app, "colorTable", json!({}));
    set(&mut app, "__color", json!("#0000ff80"));
    click_text(&mut app, "Lock");
    assert_eq!(table(&app).get("locked"), None, "{}", table(&app));
}

/// Click the dialog's text `label` with the pointer (frames a quarter second apart).
fn click_text(app: &mut VectorcraftApp, label: &str) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let time = ctx.input(|i| i.time) + 0.25;
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1000.0)));
        let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        fn walk(s: &egui::Shape, v: &mut Vec<(String, egui::Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    };
    frame(app, vec![]);
    let texts = frame(app, vec![]);
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, vec![egui::Event::PointerMoved(at)]);
    frame(app, vec![press(true)]);
    frame(app, vec![press(false)]);
}

/// A Custom table whose preview fails (only a color mapped to transparent left) still offers
/// Restore Generated Table, which brings the preview back.
#[test]
fn restore_generated_table_shows_when_the_preview_fails() {
    let dir = vectorcraft_testkit::temp_dir("sfw-restore-table");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    let table = json!({"deleted": ["#cc3300"], "customPalette": ["#cc3300", "#0000ff"], "transparent": ["#0000ff"]});
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none")), ("colorTable", table)] {
        set(&mut app, k, v);
    }
    let p = json!({"format": "gif", "antiAlias": "none", "colorTable": app.ui.dialog.as_ref().unwrap().fields["colorTable"].clone()});
    assert!(app.session.execute("document.exportForWeb.preview", &p).is_err(), "the preview fails");
    click_text(&mut app, "Restore Generated Table");
    let t = &app.ui.dialog.as_ref().unwrap().fields["colorTable"];
    assert_eq!((&t["deleted"], &t["customPalette"]), (&json!([]), &json!([])));
}

/// The saved file lacks the deleted color, whose pixels take the color left; the edit is
/// remembered, reopens with the dialog and stays through a format change.
#[test]
fn delete_color_reaches_the_saved_file_and_is_remembered() {
    let dir = vectorcraft_testkit::temp_dir("sfw-delete-color");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    frame(&mut app);
    set(&mut app, "__colors", json!(["#cc3300"]));
    save_for_web::delete_colors(app.ui.dialog.as_mut().unwrap(), &["#cc3300".into(), "#0000ff".into()]).unwrap();
    frame(&mut app);
    for format in ["jpg", "gif"] {
        set(&mut app, "format", json!(format));
        frame(&mut app);
    }
    let r = confirm(&mut app).unwrap();
    let image = image::open(r["path"].as_str().unwrap()).unwrap().to_rgba8();
    assert_eq!((image.get_pixel(20, 14).0, image.get_pixel(50, 20).0), ([0, 0, 255, 255], [0, 0, 255, 255]));
    assert_eq!(app.session.last_web_settings().color_table.deleted, ["#cc3300"]);
    app.run("file.saveForWeb", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["colorTable"]["deleted"], json!(["#cc3300"]));
    frame(&mut app);
}

#[test]
fn optimize_to_file_size_takes_the_settings_that_fit() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("jpg"));
    set(&mut app, "quality", json!(100));
    frame(&mut app);
    let full = app.session.execute("document.exportForWeb.preview", &json!({"format": "jpg", "quality": 100})).unwrap()["bytes"].as_u64().unwrap();
    let field = |app: &VectorcraftApp, k: &str| app.ui.dialog.as_ref().unwrap().fields.get(k).cloned();
    // A size of 0 is refused: the settings and the rows stay.
    set(&mut app, "__optimizeK", json!(0));
    set(&mut app, "__optimizeStart", json!("current"));
    set(&mut app, "__optimizeGo", json!(true));
    frame(&mut app);
    assert_eq!(field(&app, "quality"), Some(json!(100)));
    assert_eq!(field(&app, "__optimizeK"), Some(json!(0)));
    assert_eq!((field(&app, "__optimized"), field(&app, "__optimizeGo")), (None, None));
    // OK takes the settings found and closes the rows.
    set(&mut app, "__optimizeK", json!(full as f64 * 0.8 / 1024.0));
    set(&mut app, "__optimizeGo", json!(true));
    frame(&mut app);
    let found = field(&app, "__optimized").unwrap();
    assert_eq!(found["achieved"], json!(true), "{found}");
    let quality = field(&app, "quality").unwrap().as_u64().unwrap();
    assert!(quality < 100, "{found}");
    assert_eq!(field(&app, "__optimizeK"), None);
    // The result holds for the settings found on the art it measured.
    let settings = app.session.web_settings(&found["settings"]).unwrap();
    let shown = |app: &VectorcraftApp| {
        save_for_web::optimized_result(app.ui.dialog.as_ref().unwrap(), Some(&settings), &save_for_web::art_key(app)).is_some()
    };
    assert!(shown(&app));
    // The result line draws, and Save writes the size found.
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    let written = std::fs::read(r["path"].as_str().unwrap()).unwrap();
    assert_eq!(written.len() as u64, found["bytes"].as_u64().unwrap());
    assert!(written.len() as u64 <= found["targetBytes"].as_u64().unwrap());
    assert_eq!(app.session.last_web_settings().quality as u64, quality);
    // A change to the art drops the result.
    app.run("file.saveForWeb", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("__optimized".into(), found.clone());
    assert!(shown(&app));
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    assert!(!shown(&app));
    // So does a change to the selection, which decides the slices Save writes with `slices: selected`,
    // also one that leaves the revision as it is (a click on the canvas).
    let mut current = found.clone();
    current["art"] = json!(save_for_web::art_key(&app));
    app.ui.dialog.as_mut().unwrap().fields.insert("__optimized".into(), current);
    assert!(shown(&app));
    let st = app.session.active_mut().unwrap();
    assert!(!st.selection.objects.is_empty());
    st.selection.objects.clear();
    assert!(!shown(&app));
    // And a change to the selected slices alone (the Slice Selection tool).
    let mut current = found.clone();
    current["art"] = json!(save_for_web::art_key(&app));
    app.ui.dialog.as_mut().unwrap().fields.insert("__optimized".into(), current);
    assert!(shown(&app));
    let st = app.session.active_mut().unwrap();
    let any = st.doc.layers[0].id;
    st.selection.slices.push(any);
    assert!(!shown(&app));
}

/// Enter while the Optimize to File Size rows are open runs the search and writes nothing; once the
/// search has closed the rows, Enter saves with the settings it found.
#[test]
fn enter_in_the_optimize_rows_runs_the_search_and_saves_nothing() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize-enter");
    let (mut app, _) = app(&dir, false);
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("jpg"));
    set(&mut app, "quality", json!(100));
    frame(&mut app);
    let full = app.session.execute("document.exportForWeb.preview", &json!({"format": "jpg", "quality": 100})).unwrap()["bytes"].as_u64().unwrap();
    set(&mut app, "__optimizeK", json!(full as f64 * 0.8 / 1024.0));
    set(&mut app, "__optimizeStart", json!("current"));
    let files = |dir: &std::path::Path| std::fs::read_dir(dir).map_or(0, |r| r.count());
    let enter_with = |app: &mut VectorcraftApp, modifiers: egui::Modifiers| {
        let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    let enter = |app: &mut VectorcraftApp| enter_with(app, egui::Modifiers::NONE);
    // Ctrl+Enter, which the size field does not commit, neither searches nor saves.
    enter_with(&mut app, egui::Modifiers::CTRL);
    frame(&mut app);
    assert_eq!(kind(&app), Some(save_for_web::KIND), "Ctrl+Enter in the rows does not save");
    assert!(app.ui.dialog.as_ref().unwrap().fields.contains_key("__optimizeK"), "the rows stay open");
    assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("__optimized"));
    assert_eq!(files(&dir), 0);
    enter(&mut app);
    assert_eq!(kind(&app), Some(save_for_web::KIND), "Enter in the rows does not save");
    assert_eq!(files(&dir), 0);
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    let found = d.fields.get("__optimized").cloned().unwrap();
    assert_eq!(found["achieved"], json!(true), "{found}");
    assert!(!d.fields.contains_key("__optimizeK"), "the search closed the rows");
    enter(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!(files(&dir), 1);
}

/// An Enter in the Optimize to File Size rows runs no search when a field of the dialog, one drawn
/// after the rows included, refused the value typed in it on that press, or when a control drawn
/// after the rows (an open dropdown) took the press.
#[test]
fn enter_in_the_optimize_rows_runs_nothing_when_a_later_control_refused_or_took_it() {
    // `before` stands for the controls drawn after the rows, which act on the press first.
    let go = |before: fn(&egui::Context)| {
        let mut d = Dialog::new(save_for_web::KIND, json!({"__optimizeK": 10}));
        let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| {
            before(ui.ctx());
            save_for_web::optimize_on_enter(ui.ctx(), &mut d);
        });
        out.textures_delta.clear();
        assert!(d.fields.contains_key("__optimizeK"), "the rows stay open");
        d.fields.contains_key("__optimizeGo")
    };
    assert!(go(|_| {}), "a plain Enter runs the search");
    assert!(!go(crate::widgets::reject_dialog_commit), "a refused value runs nothing");
    assert!(!go(|ctx| ctx.input_mut(|i| _ = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))), "a taken press runs nothing");
}

#[test]
fn optimize_to_file_size_starts_at_the_size_of_the_slices_files() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize-slices");
    let (mut app, _) = app(&dir, false);
    app.run("object.slice.create", json!({"x": 0, "y": 0, "width": 30, "height": 20})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    frame(&mut app);
    let slices = app.session.execute("webExport.optimizeToFileSize", &json!({"format": "png24", "desiredK": 1048576})).unwrap();
    let whole = app.session.execute("document.exportForWeb.preview", &json!({"format": "png24"})).unwrap()["bytes"].as_u64().unwrap();
    let k = |b: u64| json!(save_for_web::size_k(b as usize));
    assert_ne!(k(slices["bytes"].as_u64().unwrap()), k(whole), "{slices}");
    // The size asked for holds the files measured.
    let b = slices["bytes"].as_u64().unwrap();
    assert!((save_for_web::size_k(b as usize) * 1024.0).floor() as u64 >= b);
    set(&mut app, "__optimizeOpen", json!(true));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields.get("__optimizeK"), Some(&k(slices["bytes"].as_u64().unwrap())));
    assert_eq!((d.fields.get("__optimizeUse"), d.fields.get("__optimizeOpen")), (Some(&json!("allSlices")), None));
}

#[test]
fn optimize_to_file_size_starts_at_the_size_of_the_art_as_it_is_now() {
    let dir = vectorcraft_testkit::temp_dir("sfw-optimize-revision");
    let (mut app, _) = app(&dir, false);
    // One context for every frame, so the preview cache of the first frame is still there.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp| ctx.run_ui(Default::default(), |ui| show(app, ui.ctx())).textures_delta.clear();
    app.run("file.saveForWeb", json!({})).unwrap();
    set(&mut app, "format", json!("png24"));
    frame(&mut app);
    let bytes = |app: &mut VectorcraftApp| {
        app.session.execute("document.exportForWeb.preview", &json!({"format": "png24"})).unwrap()["bytes"].as_u64().unwrap()
    };
    let before = bytes(&mut app);
    // The art changes after the preview drew and before the button's click is handled.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#3366ff"})).unwrap();
    let after = bytes(&mut app);
    assert_ne!(save_for_web::size_k(before as usize), save_for_web::size_k(after as usize));
    set(&mut app, "__optimizeOpen", json!(true));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields.get("__optimizeK"), Some(&json!(save_for_web::size_k(after as usize))));
}

#[test]
fn size_in_k_holds_sizes_past_4_mib_on_every_target() {
    assert_eq!(save_for_web::size_k(5 * 1024 * 1024), 5120.0);
    assert_eq!(save_for_web::size_k(0), 0.001);
    assert_eq!(save_for_web::size_k(1), 0.001);
    // The largest size a target can hold still rounds up to a size that holds it.
    assert!(save_for_web::size_k(usize::MAX) * 1024.0 >= usize::MAX as f64 * 0.999);
}
