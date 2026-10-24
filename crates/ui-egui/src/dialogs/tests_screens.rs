//! Export for Screens, drawn headlessly: the fields it opens on (defaults, then the document's last
//! settings, and the artboards the Artboard tool has selected), the artboard choice, Format
//! Settings, desktop folders, the export on the background thread and web downloads.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::tests_export::{frame, kind, set};
use super::*;
use crate::Services;
use crate::background;
use crate::tests_background::{Files, Gate, written};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with `artboards` 60×40 pt artboards and an ellipse on the first; `web` gives it a
/// download service (recorded in the log) instead of a file manager (whose reveals are recorded).
fn app(artboards: usize, web: bool) -> (VectorcraftApp, Log) {
    let log = Log::default();
    let l = log.clone();
    let mut services = Services::default();
    if web {
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            l.borrow_mut().push((name.to_string(), bytes.to_vec()));
            Ok(())
        }));
    } else {
        services.reveal = Some(Box::new(move |path: &str| {
            l.borrow_mut().push((path.to_string(), vec![]));
            Ok(())
        }));
        services.pick_folder = Some(Box::new(|| None));
    }
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": artboards})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, log)
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().expect("a dialog is open").fields.get(k).cloned().unwrap_or(Value::Null)
}

fn open(app: &mut VectorcraftApp) {
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!(kind(app), Some(export_for_screens::KIND));
}

fn temp_folder(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("vc-efs-{tag}-{}", std::process::id()))
}

#[test]
fn opens_on_defaults_and_draws_every_view() {
    let (mut app, _) = app(3, false);
    open(&mut app);
    assert_eq!((field(&app, "select"), field(&app, "range"), field(&app, "boards")), (json!("all"), json!("1-3"), json!([true, true, true])));
    assert_eq!(field(&app, "formats"), json!([{"format": "png", "scale": "1x", "suffix": ""}]));
    assert_eq!(field(&app, "folder"), json!(fileio::export_folder().unwrap_or_default()), "the Desktop, else home");
    assert_eq!((field(&app, "openLocation"), field(&app, "subfolders"), field(&app, "preset")), (json!(true), json!(false), json!("")));
    let settings = field(&app, "settings");
    for f in ["png", "png8", "jpg", "webp", "gif", "tiff", "svg", "pdf"] {
        assert!(settings[f].is_object(), "{f} has settings");
    }
    assert!(settings["jpg"].get("quality").is_none() && settings["jpg"].get("imageMap").is_none(), "rows choose quality; no maps");
    assert!(settings["svg"].get("images").is_none(), "no linked images");
    assert_eq!(settings["pdf"]["preset"], json!(fileio::pdf::DEFAULT_PRESET));
    frame(&mut app);
    // Assets, Full Document, each format's Format Settings: all draw.
    set(&mut app, "tab", json!("assets"));
    frame(&mut app);
    set(&mut app, "tab", json!("artboards"));
    set(&mut app, "select", json!("full"));
    for f in ["png", "png8", "jpg", "webp", "gif", "tiff", "svg", "pdf"] {
        set(&mut app, "__settings", json!(f));
        frame(&mut app);
        assert_eq!(field(&app, "__settings"), json!(f));
    }
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), "Export Artboard");
}

#[test]
fn checked_artboards_read_as_a_range() {
    let (mut app, _) = app(5, false);
    open(&mut app);
    set(&mut app, "boards", json!([true, false, true, true, false]));
    frame(&mut app);
    // Nothing clicked: the fields stay as set.
    assert_eq!(field(&app, "boards"), json!([true, false, true, true, false]));
    for (boards, text) in [
        (vec![true, true, true], "1-3"),
        (vec![false, true, false, true], "2, 4"),
        (vec![true, false, true, true, false, true], "1, 3-4, 6"),
        (vec![false], ""),
    ] {
        assert_eq!(export_for_screens::range_text(&boards), text);
    }
}

#[test]
fn opens_with_the_artboards_the_artboard_tool_selected_and_exports_only_them() {
    let (mut app, log) = app(4, true);
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let view = app.view_info();
    let click = |app: &mut VectorcraftApp, i: usize, shift: bool| {
        let mods = Mods { shift, ..Default::default() };
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(app, &PointerEvent { kind, pos: boards[i].center(), mods, pressure: 1.0 }, view);
        }
    };
    // A click on artboard 2 and a Shift-click on artboard 3 select those two. The dialog opens with
    // them checked, and Download writes only them.
    click(&mut app, 1, false);
    click(&mut app, 2, true);
    open(&mut app);
    assert_eq!(
        (field(&app, "select"), field(&app, "range"), field(&app, "boards")),
        (json!("range"), json!("2-3"), json!([false, true, true, false]))
    );
    frame(&mut app);
    assert_eq!(field(&app, "boards"), json!([false, true, true, false]), "drawing the dialog keeps them");
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["files"], json!(["Artboard-2.png", "Artboard-3.png"]));
    assert_eq!(log.borrow().len(), 1, "one zip");
    // With every artboard selected it opens on All.
    click(&mut app, 0, true);
    click(&mut app, 3, true);
    open(&mut app);
    assert_eq!((field(&app, "select"), field(&app, "range"), field(&app, "boards")), (json!("all"), json!("1-4"), json!([true, true, true, true])));
}

#[test]
fn opens_with_the_rows_selected_in_the_artboards_panel_else_on_the_saved_choice() {
    let (mut app, _) = app(4, true);
    // An export of the full document with a prefix and a JPEG row is the saved choice.
    app.run("file.exportForScreens", json!({"fullDocument": true, "prefix": "proof-", "formats": [{"format": "jpg", "quality": 80, "scale": "2x"}]}))
        .unwrap();
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 400.0)));
    let panel = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::panels::artboards::show(app, ui));
        out.textures_delta.clear();
        out
    };
    // With the Artboard tool in use, a click on Artboard 2's row and a Shift-click on Artboard 3's
    // select those two. The dialog opens with them checked instead of the saved Full Document.
    let rows = panel(&mut app, 0.0, vec![]);
    for (time, label, modifiers) in [(1.0, "Artboard 2", egui::Modifiers::NONE), (2.0, "Artboard 3", egui::Modifiers::SHIFT)] {
        let at = screen_text_at(&rows, label);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        panel(&mut app, time, vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(at), button(true), button(false)]);
    }
    assert_eq!(crate::panels::artboards::picked(&app, 4), [1, 2], "the panel highlights rows 2 and 3");
    open(&mut app);
    let picked = app.ui.dialog.take().expect("a dialog is open").fields;
    assert_eq!((&picked["select"], &picked["range"], &picked["boards"]), (&json!("range"), &json!("2-3"), &json!([false, true, true, false])));
    // With the Selection tool in use, the dialog opens on the saved choice.
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    open(&mut app);
    assert_eq!((field(&app, "select"), field(&app, "range"), field(&app, "boards")), (json!("full"), json!("1-4"), json!([true, true, true, true])));
    assert_eq!((field(&app, "prefix"), field(&app, "formats")[0]["format"].clone()), (json!("proof-"), json!("jpg")));
    // Only the artboard fields differ.
    let others = |fields: &serde_json::Map<String, Value>| {
        let mut f = fields.clone();
        for k in ["select", "range", "boards"] {
            f.remove(k);
        }
        f
    };
    assert_eq!(others(&picked), others(&app.ui.dialog.as_ref().expect("a dialog is open").fields));
}

#[test]
fn exports_into_the_folder_and_shows_it() {
    let (mut app, log) = app(2, false);
    let dir = temp_folder("desk");
    let folder = dir.to_string_lossy().replace('\\', "/");
    open(&mut app);
    set(&mut app, "folder", json!(folder));
    set(&mut app, "subfolders", json!(true));
    set(
        &mut app,
        "formats",
        json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "png", "scale": "120w", "suffix": "@120w"}, {"format": "svg", "scale": "1x", "suffix": ""}]),
    );
    // A bad range keeps the dialog open; so does no folder.
    set(&mut app, "select", json!("range"));
    set(&mut app, "range", json!("7"));
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    set(&mut app, "range", json!("2"));
    set(&mut app, "folder", json!(" "));
    assert!(confirm(&mut app).is_err());
    set(&mut app, "folder", json!(folder));
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, [format!("{folder}/1x/Artboard-2.png"), format!("{folder}/120w/Artboard-2@120w.png"), format!("{folder}/SVG/Artboard-2.svg")]);
    let png = image::load_from_memory(&std::fs::read(dir.join("120w").join("Artboard-2@120w.png")).unwrap()).unwrap();
    assert_eq!((png.width(), png.height()), (120, 80));
    assert_eq!(log.borrow().iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), [files[0]], "Open Location shows the first file");
    // It reopens on those settings.
    open(&mut app);
    assert_eq!((field(&app, "select"), field(&app, "range"), field(&app, "boards")), (json!("range"), json!("2"), json!([false, true])));
    assert_eq!(field(&app, "formats")[1]["scale"], json!("120w"));
    assert_eq!((field(&app, "subfolders"), field(&app, "folder")), (json!(true), json!(folder)));
    let _ = std::fs::remove_dir_all(dir);
}

/// [`app`] on the desktop with Background Export's thread-safe writer, which records the files it
/// is given once `gate` opens.
fn app_with_worker(artboards: usize, gate: &Gate) -> (VectorcraftApp, Log, Files) {
    let (mut app, log) = app(artboards, false);
    let (files, g) = (Files::default(), gate.clone());
    let recorded = files.clone();
    app.services.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| {
        g.wait();
        recorded.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    (app, log, files)
}

fn background_export(app: &mut VectorcraftApp, on: bool) {
    app.session.execute("prefs.set", &json!({"values": {"backgroundExport": on}})).unwrap();
}

/// The files reply `r` names (its `files`, else the ZIP at its `path`) as they are on disk.
fn on_disk(r: &Value) -> Vec<(String, Vec<u8>)> {
    let paths: Vec<&str> = match r["path"].as_str() {
        Some(zip) => vec![zip],
        None => r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect(),
    };
    paths.into_iter().map(|p| (p.to_string(), std::fs::read(p).unwrap())).collect()
}

/// With Background Export on, OK closes the dialog with nothing written, and the background thread
/// writes the files OK writes at once with it off: the same names (`-2` included), sub-folders and
/// bytes, and the same status; Open Location shows the first file once it is written. The same
/// holds for one ZIP in the folder.
#[test]
fn the_files_are_written_in_the_background_as_at_once() {
    let rows = json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "png", "scale": "2x", "suffix": "@2x"}, {"format": "svg", "scale": "1x", "suffix": ""}]);
    let dir = temp_folder("background");
    let folder = dir.to_string_lossy().replace('\\', "/");
    for zip in [false, true] {
        let gate = Gate::default();
        let (mut app, log, files) = app_with_worker(2, &gate);
        app.run("artboard.setProps", json!({"index": 0, "name": "Icon"})).unwrap();
        app.run("artboard.setProps", json!({"index": 1, "name": "icon"})).unwrap();
        let export = |app: &mut VectorcraftApp| {
            if zip {
                let p = json!({"folder": folder, "zip": true, "folders": "rows", "name": "<name>", "formats": rows, "openLocation": true});
                return app.run("file.exportForScreens", p);
            }
            open(app);
            set(app, "folder", json!(folder));
            set(app, "folders", json!("rows"));
            set(app, "name", json!("<name>"));
            set(app, "formats", rows.clone());
            confirm(app)
        };
        background_export(&mut app, false);
        let r = export(&mut app).unwrap();
        let want = on_disk(&r);
        if !zip {
            assert_eq!(want.len(), 6, "{r}");
            assert_eq!(want[3].0, format!("{folder}/1x/icon-2.png"));
        }
        let (status, shown) = (app.ui.status.clone(), log.borrow().clone());
        assert_eq!(shown.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
        log.borrow_mut().clear();
        background_export(&mut app, true);
        let r = export(&mut app).unwrap();
        assert_eq!(r["background"], true, "{r}");
        assert!(app.ui.dialog.is_none(), "closed");
        assert_eq!(app.ui.status, format!("Exporting {}…", dir.file_name().unwrap().to_string_lossy()));
        assert!(written(&files).is_empty() && log.borrow().is_empty(), "nothing written yet");
        gate.open();
        background::wait_all(&mut app);
        assert_eq!(*files.lock().unwrap(), want, "zip: {zip}");
        assert_eq!((app.ui.status.clone(), log.borrow().clone()), (status, shown));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn the_asset_export_panel_writes_in_the_background_as_at_once() {
    let gate = Gate::default();
    let (mut app, _, files) = app_with_worker(1, &gate);
    app.run("select.all", json!({})).unwrap();
    app.run("assets.add", json!({"multiple": false})).unwrap();
    let dir = temp_folder("assets-background");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let p = json!({"folder": folder, "formats": [{"format": "png", "scale": "2x", "suffix": "@2x"}, {"format": "svg"}]});
    background_export(&mut app, false);
    let want = on_disk(&crate::io::export_files(&mut app, "assets.export", p.clone()).unwrap());
    assert_eq!(want.len(), 2);
    let status = app.ui.status.clone();
    std::fs::remove_dir_all(&dir).unwrap();
    background_export(&mut app, true);
    assert_eq!(crate::io::export_files(&mut app, "assets.export", p).unwrap()["background"], true);
    assert!(written(&files).is_empty(), "nothing written yet");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!((files.lock().unwrap().clone(), app.ui.status.clone()), (want, status));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edits_after_ok_do_not_reach_the_files() {
    let gate = Gate::default();
    let (mut app, _, files) = app_with_worker(1, &gate);
    let dir = temp_folder("snapshot");
    let p = json!({"folder": dir.to_string_lossy().replace('\\', "/"), "formats": [{"format": "svg"}]});
    background_export(&mut app, false);
    let want = on_disk(&app.run("file.exportForScreens", p.clone()).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
    background_export(&mut app, true);
    assert_eq!(app.run("file.exportForScreens", p).unwrap()["background"], true);
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(*files.lock().unwrap(), want, "the document as it was at OK");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cancel stops the export before its next file; the file being written when it came stays.
#[test]
fn cancel_stops_the_export_before_its_next_file() {
    let (gate, writing) = (Gate::default(), Gate::default());
    let (mut app, _) = app(3, false);
    let files = Files::default();
    let (f, g, w) = (files.clone(), gate.clone(), writing.clone());
    app.services.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| {
        w.open();
        g.wait();
        f.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    let dir = temp_folder("cancel");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let r = app.run("file.exportForScreens", json!({"folder": folder, "formats": [{"format": "svg"}]})).unwrap();
    assert_eq!(r["background"], true, "{r}");
    writing.wait();
    assert_eq!(app.run("file.cancelExport", json!({})).unwrap()["cancelled"], 1);
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), [format!("{folder}/Artboard-1.svg")]);
    assert_eq!(app.ui.status, crate::io::EXPORT_CANCELLED);
    let _ = std::fs::remove_dir_all(&dir);
}

/// With Background Export turned off, an export into the same folder is written after the one in
/// the background.
#[test]
fn an_export_made_at_once_waits_for_one_in_the_background() {
    let gate = Gate::default();
    let (mut app, _, files) = app_with_worker(1, &gate);
    let dir = temp_folder("twice");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let p = json!({"folder": folder, "formats": [{"format": "svg"}]});
    assert_eq!(app.run("file.exportForScreens", p.clone()).unwrap()["background"], true);
    background_export(&mut app, false);
    let opener = gate.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        opener.open();
    });
    let r = app.run("file.exportForScreens", p).unwrap();
    assert!(app.background.jobs.is_empty(), "the export in the background was written first");
    release.join().unwrap();
    let file = format!("{folder}/Artboard-1.svg");
    assert_eq!((written(&files), r["files"].clone()), (vec![file.clone()], json!([file])));
    assert_eq!(app.ui.status, format!("Exported 1 file(s) to {folder}"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without a folder (a script or an agent on the desktop) the files come back in the reply at once,
/// with Background Export on too.
#[test]
fn an_export_without_a_folder_replies_with_its_files() {
    let (mut app, _, files) = app_with_worker(1, &Gate::default());
    let r = app.run("file.exportForScreens", json!({"formats": [{"format": "svg"}]})).unwrap();
    assert!(r.get("background").is_none(), "{r}");
    assert_eq!(r["files"][0]["name"], "Artboard-1.svg");
    assert!(app.background.jobs.is_empty() && written(&files).is_empty());
}

/// The command's refusal keeps the dialog open and starts nothing, as at once.
#[test]
fn a_refused_export_keeps_the_dialog_open_with_background_export_on() {
    let (mut app, _, files) = app_with_worker(2, &Gate::default());
    open(&mut app);
    set(&mut app, "folder", json!(temp_folder("refused").to_string_lossy().replace('\\', "/")));
    set(&mut app, "formats", json!([{"format": "dxf", "scale": "1x", "suffix": ""}]));
    let e = confirm(&mut app).unwrap_err();
    assert!(e.contains("Export for Screens writes png"), "{e}");
    assert_eq!((kind(&app), app.ui.status.as_str()), (Some(export_for_screens::KIND), e.as_str()));
    assert!(app.background.jobs.is_empty() && written(&files).is_empty());
}

#[test]
fn the_web_downloads_one_zip_or_the_file() {
    let (mut app, log) = app(2, true);
    assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), "Download");
    open(&mut app);
    frame(&mut app);
    set(
        &mut app,
        "formats",
        json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "jpg", "quality": 80, "scale": "2x", "suffix": "@2x"}]),
    );
    set(&mut app, "settings", json!({"png": {"background": "black"}}));
    let r = confirm(&mut app).unwrap();
    assert!(r.get("dataBase64").is_some(), "four files: one zip");
    {
        let log = log.borrow();
        let [(name, zip)] = log.as_slice() else { panic!("one download: {:?}", log.iter().map(|(n, _)| n).collect::<Vec<_>>()) };
        assert_eq!(name, "Untitled-1.zip");
        assert_eq!(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap(), zip);
    }
    assert_eq!(r["files"], json!(["Artboard-1.png", "Artboard-1@2x.jpg", "Artboard-2.png", "Artboard-2@2x.jpg"]));
    // One file downloads as itself, with the format's settings.
    log.borrow_mut().clear();
    open(&mut app);
    assert_eq!(field(&app, "settings")["png"]["background"], json!("black"), "remembered");
    set(&mut app, "select", json!("range"));
    set(&mut app, "range", json!("1"));
    set(&mut app, "formats", json!([{"format": "png", "scale": "1x", "suffix": ""}]));
    confirm(&mut app).unwrap();
    let log = log.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "Artboard-1.png");
    assert_eq!(image::load_from_memory(&log[0].1).unwrap().to_rgba8().get_pixel(1, 1).0, [0, 0, 0, 255], "the PNG settings' background");
}

#[test]
fn the_status_bar_says_when_an_image_could_not_be_decoded() {
    const NOTE: &str = "with 1 note(s): an image couldn't be decoded and is missing from the picture";
    // A PNG whose header reads but whose pixel data is cut short: the renderer draws nothing for it.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png.truncate(idat + 8);
    let place = |app: &mut VectorcraftApp| {
        app.run("file.place", json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [10, 10]})).unwrap()
    };
    let rows = json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "jpg", "scale": "2x", "suffix": "@2x"}]);
    let dir = temp_folder("undecoded");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut desktop, log) = app(1, false);
    desktop.run("file.exportForScreens", json!({"folder": folder, "formats": rows})).unwrap();
    assert_eq!(desktop.ui.status, format!("Exported 2 file(s) to {folder}"));
    place(&mut desktop);
    desktop.run("file.exportForScreens", json!({"folder": folder, "formats": rows, "openLocation": true})).unwrap();
    assert_eq!(desktop.ui.status, format!("Saved {folder} {NOTE}"));
    assert_eq!(log.borrow().len(), 1, "Open Location still shows the first file");
    let r = desktop.run("file.exportForScreens", json!({"folder": folder, "formats": rows, "zip": true})).unwrap();
    assert_eq!(desktop.ui.status, format!("Saved {} {NOTE}", r["path"].as_str().unwrap()));
    // The Asset Export panel's export goes the same way.
    desktop.run("select.all", json!({})).unwrap();
    desktop.run("assets.add", json!({"multiple": false})).unwrap();
    crate::io::export_files(&mut desktop, "assets.export", json!({"folder": folder, "formats": rows})).unwrap();
    assert_eq!(desktop.ui.status, format!("Saved {folder} {NOTE}"), "assets.export");
    let (mut web, _) = app(1, true);
    place(&mut web);
    web.run("file.exportForScreens", json!({"zip": true, "formats": rows})).unwrap();
    assert_eq!(web.ui.status, format!("Saved Untitled-1.zip {NOTE}"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_preset_export_reopens_on_its_rows() {
    let (mut app, _) = app(1, true);
    app.run("file.exportForScreens", json!({"preset": "density", "fullDocument": true})).unwrap();
    open(&mut app);
    assert_eq!(field(&app, "preset"), json!("density"));
    assert_eq!(field(&app, "select"), json!("full"));
    assert_eq!(field(&app, "subfolders"), json!(true));
    let rows = field(&app, "formats");
    assert_eq!(rows.as_array().unwrap().len(), 6);
    assert_eq!((rows[0]["scale"].clone(), rows[0]["folder"].clone()), (json!("0.75x"), json!("ldpi")));
    frame(&mut app);
    // Agents' numeric sizes show as text.
    app.run(
        "file.exportForScreens",
        json!({"formats": [{"format": "png", "width": 64}, {"format": "png", "scale": 3}, {"format": "png", "ppi": 144}]}),
    )
    .unwrap();
    open(&mut app);
    let scales: Vec<Value> = field(&app, "formats").as_array().unwrap().iter().map(|r| r["scale"].clone()).collect();
    assert_eq!(scales, [json!("64w"), json!("3x"), json!("144ppi")]);
}

/// Persistent frames keep dropdowns open between the pointer press and release.
struct ScreensFrame {
    ctx: egui::Context,
    time: f64,
}

impl ScreensFrame {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0 }
    }

    fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.1;
        let input = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 900.0))),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    fn settle(&mut self, app: &mut VectorcraftApp) -> egui::FullOutput {
        self.frame(app, vec![]);
        self.frame(app, vec![])
    }

    fn asset_panel(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.1;
        let input = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(236.0, 700.0))),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| crate::panels::show_icon_panel(app, ui, "assetExport"));
        out.textures_delta.clear();
        out
    }

    fn click(&mut self, app: &mut VectorcraftApp, at: egui::Pos2) -> egui::FullOutput {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        self.frame(app, vec![egui::Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
        self.settle(app)
    }

    fn click_label(&mut self, app: &mut VectorcraftApp, label: &str) -> egui::FullOutput {
        let out = self.settle(app);
        self.click(app, screen_text_at(&out, label))
    }

    fn choose(&mut self, app: &mut VectorcraftApp, current: &str, next: &str) {
        let out = self.click_label(app, current);
        self.click(app, screen_text_at(&out, next));
    }
}

fn screen_text_at(out: &egui::FullOutput, label: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text().trim() == label => Some(t.visual_bounding_rect().center()),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, label)),
            _ => None,
        }
    }
    out.shapes.iter().find_map(|s| find(&s.shape, label)).unwrap_or_else(|| panic!("missing {label}: {}", crate::tests_labels::shapes_text(out)))
}

/// First-directory entries of the big-endian TIFF chosen below: tag, type, count, inline value.
fn big_tiff_tags(bytes: &[u8]) -> Vec<(u16, u16, u32, u16)> {
    assert_eq!(&bytes[..4], b"MM\0*");
    let short = |at| u16::from_be_bytes(bytes[at..at + 2].try_into().unwrap());
    let long = |at| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let dir = long(4) as usize;
    (0..usize::from(short(dir)))
        .map(|i| {
            let at = dir + 2 + 12 * i;
            (short(at), short(at + 2), long(at + 4), short(at + 8))
        })
        .collect()
}

/// Locate the gear from the painted Formats label and its production column width.
fn screen_format_gear(out: &egui::FullOutput, width: f32) -> egui::Pos2 {
    fn label(shape: &egui::Shape) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text().trim() == "Formats:" => Some(t.visual_bounding_rect()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(label),
            _ => None,
        }
    }
    let label = out.shapes.iter().find_map(|s| label(&s.shape)).expect("painted Formats row");
    // The existing 24 pt gear is right-aligned in the 350 pt dialog or 236 pt panel.
    egui::pos2(label.left() + width - 12.0, label.center().y)
}

fn saved_tiff_alias_gear(alias: &str, asset_panel: bool) {
    let (mut app, log) = app(1, true);
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
    if asset_panel {
        app.run("assets.add", json!({"name": "Badge"})).unwrap();
    }
    let rows = json!([{"format": alias, "scale": "2x", "suffix": "@2x", "antiAlias": "none"}]);
    app.run(
        "assets.settings.set",
        json!({
            "formats": rows.clone(),
            "settings": {
                "tiff": {"colorModel": "rgb", "byteOrder": "little", "lzw": false, "embedIcc": false, "antiAlias": "type"},
                "png": {"interlaced": true}
            }
        }),
    )
    .unwrap();
    let saved_before = app.session.active().unwrap().doc.export_settings.clone();
    if !asset_panel {
        open(&mut app);
    }
    let mut h = ScreensFrame::new();
    let draw = |h: &mut ScreensFrame, app: &mut VectorcraftApp, events| {
        if asset_panel { h.asset_panel(app, events) } else { h.frame(app, events) }
    };
    draw(&mut h, &mut app, vec![]);
    let out = draw(&mut h, &mut app, vec![]);
    let gear = screen_format_gear(&out, if asset_panel { 236.0 } else { 350.0 });
    draw(&mut h, &mut app, vec![egui::Event::PointerMoved(gear)]);
    for pressed in [true, false] {
        draw(
            &mut h,
            &mut app,
            vec![egui::Event::PointerButton { pos: gear, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }],
        );
    }
    assert_eq!(field(&app, "__settings"), "tiff", "gear for {alias}, asset panel={asset_panel}");
    assert_eq!(field(&app, "tab"), if asset_panel { "assets" } else { "artboards" });
    let out = h.settle(&mut app);
    for label in ["Format Settings", "TIFF", "Color Model:", "RGB", "Byte Order:", "Little-endian (II)", "LZW Compression", "Embed ICC Profile"] {
        screen_text_at(&out, label);
    }
    assert_eq!(field(&app, "formats"), rows, "opening the gear preserves the saved spelling and row options");
    assert_eq!(app.session.active().unwrap().doc.export_settings, saved_before, "opening settings does not save them");
    let mut expected = field(&app, "settings");
    assert_eq!(expected["tiff"]["colorModel"], "rgb", "saved RGB overrides the CMYK document default");
    assert_eq!(expected["tiff"]["lzw"], false);
    assert_eq!(expected["tiff"]["embedIcc"], false);
    assert_eq!(expected["tiff"]["antiAlias"], "type");
    assert_eq!(expected["png"]["interlaced"], true);
    h.choose(&mut app, "Little-endian (II)", "Big-endian (MM)");
    expected["tiff"]["byteOrder"] = json!("big");
    assert_eq!(field(&app, "settings"), expected, "only the chosen TIFF option changes");
    h.click_label(&mut app, "Done");
    assert_eq!(field(&app, "formats"), rows);
    h.click_label(&mut app, "Download");
    assert!(app.ui.dialog.is_none(), "{}", app.ui.status);
    {
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF download for {alias}") };
        assert_eq!(name, if asset_panel { "Badge@2x.tif" } else { "Artboard-1@2x.tif" });
        let tags = big_tiff_tags(bytes); // The gear's edited canonical settings reach the aliased row.
        assert!(tags.contains(&(259, 3, 1, 1)), "saved uncompressed option survives");
        assert!(tags.contains(&(262, 3, 1, 2)), "saved RGB survives");
        assert!(!tags.iter().any(|t| t.0 == 34675), "saved profile-off option survives");
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Tiff).unwrap();
        assert_eq!((img.width(), img.height()), if asset_panel { (60, 40) } else { (120, 80) });
    }
    let saved = app.run("document.exportSettings", json!({})).unwrap()["settings"].clone();
    assert_eq!(saved["formats"], rows, "export preserves the alias, scale, suffix and row override");
    assert_eq!(saved["settings"], expected, "canonical TIFF edits and other format settings persist");
    open(&mut app);
    assert_eq!(field(&app, "formats"), rows);
    assert_eq!(field(&app, "settings"), expected);
}

#[test]
fn tiff_saved_alias_gear_opens_and_edits_settings_in_screens() {
    for alias in ["tiff", "tif", "TIFF", ".tif"] {
        saved_tiff_alias_gear(alias, false);
    }
}

#[test]
fn tiff_saved_alias_gear_opens_and_edits_settings_in_asset_export() {
    for alias in ["tiff", "tif", "TIFF", ".tif"] {
        saved_tiff_alias_gear(alias, true);
    }
}

#[test]
fn tiff_dropdown_exports_scaled_artboards_with_options_and_reopens() {
    let (mut app, log) = app(2, true);
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("paint.setFill", json!({"color": "#c03020"})).unwrap();
    open(&mut app);
    // Begin with an existing PNG row; TIFF must be chosen from the actual popup.
    set(&mut app, "formats", json!([{"format": "png", "scale": "2x", "suffix": "@2x"}]));
    set(&mut app, "prefix", json!("proof-"));
    let mut h = ScreensFrame::new();
    h.choose(&mut app, "PNG", "TIFF");
    assert_eq!(field(&app, "formats"), json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    // Leave only the first of two boards checked.
    h.click_label(&mut app, "Artboard 2");
    assert_eq!(field(&app, "boards"), json!([true, false]));
    assert_eq!(field(&app, "range"), json!("1"));

    // Exercise the settings dropdown independently of the row's format choice.
    set(&mut app, "__settings", json!("png"));
    h.choose(&mut app, "PNG", "TIFF");
    assert_eq!(field(&app, "__settings"), json!("tiff"));
    let settings = field(&app, "settings");
    assert_eq!(settings["tiff"]["colorModel"], "rgb");
    assert_eq!(settings["tiff"]["byteOrder"], "little");
    assert_eq!(settings["tiff"]["lzw"], true);
    assert_eq!(settings["tiff"]["embedIcc"], true);
    h.choose(&mut app, "Little-endian (II)", "Big-endian (MM)");
    h.click_label(&mut app, "LZW Compression");
    h.click_label(&mut app, "Embed ICC Profile");
    let settings = field(&app, "settings");
    assert_eq!(settings["tiff"]["byteOrder"], "big");
    assert_eq!(settings["tiff"]["lzw"], false);
    assert_eq!(settings["tiff"]["embedIcc"], false);
    h.click_label(&mut app, "Done");
    h.click_label(&mut app, "Download");
    assert!(app.ui.dialog.is_none(), "{}", app.ui.status);

    {
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF download") };
        assert_eq!(name, "proof-Artboard-1@2x.tif");
        let tags = big_tiff_tags(bytes);
        assert!(tags.contains(&(259, 3, 1, 1)), "uncompressed as selected");
        assert!(tags.contains(&(262, 3, 1, 2)), "RGB");
        assert!(!tags.iter().any(|t| t.0 == 34675), "ICC profile disabled");
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Tiff).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (120, 80));
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "transparent board corner");
        let inside = img.get_pixel(40, 28).0;
        assert!(inside[0] > inside[1] && inside[3] == 255, "ellipse at the scaled position: {inside:?}");
    }

    open(&mut app);
    assert_eq!(field(&app, "formats"), json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    assert_eq!(field(&app, "boards"), json!([true, false]));
    assert_eq!(field(&app, "prefix"), "proof-");
    assert_eq!(field(&app, "settings")["tiff"], settings["tiff"]);
    // Existing vector and raster choices remain selectable.
    h.choose(&mut app, "TIFF", "SVG");
    assert_eq!(field(&app, "formats")[0]["format"], "svg");
    h.choose(&mut app, "SVG", "PNG");
    assert_eq!(field(&app, "formats")[0]["format"], "png");
}

/// The Example line names the first file the export writes for the same settings: the item's file
/// name, a prefix with characters Windows does not allow, a range, a Windows device name as an item
/// folder, the rows' folders and sizes, Create Sub-folders, a vector row's size suffix, the counter
/// typed as text, and the Layers and Selection tabs.
#[test]
fn the_example_names_the_first_file_the_export_writes() {
    let (mut app, _) = app(3, false);
    app.run("artboard.setProps", json!({"index": 2, "name": "CON"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let dir = temp_folder("example");
    let folder = dir.to_string_lossy().to_string();
    let png = json!([{"format": "png", "scale": "1x", "suffix": ""}]);
    let cases: Vec<Vec<(&str, Value)>> = vec![
        vec![],
        vec![("prefix", json!("a?b"))],
        vec![("select", json!("range")), ("range", json!("2-3"))],
        vec![("select", json!("range")), ("range", json!("3")), ("folders", json!("item"))],
        vec![("folders", json!("rows")), ("formats", json!([{"format": "png", "scale": "144ppi", "suffix": ""}]))],
        vec![("subfolders", json!(true))],
        vec![("folders", json!("rows")), ("formats", json!([{"format": "png", "scale": "2x", "suffix": "@2x", "folder": "retina"}]))],
        vec![("formats", json!([{"format": "svg", "scale": "2x", "suffix": "@2x"}]))],
        vec![("name", json!("<scale>-####")), ("counterStart", json!("7")), ("formats", json!([{"format": "png", "scale": "144ppi", "suffix": ""}]))],
        vec![("tab", json!("layers"))],
        vec![("tab", json!("selection")), ("selectionMode", json!("each"))],
    ];
    let mut wrong = vec![];
    for case in cases {
        let _ = std::fs::remove_dir_all(&dir);
        open(&mut app);
        let defaults = [
            ("folder", json!(folder)),
            ("tab", json!("artboards")),
            ("select", json!("all")),
            ("prefix", json!("")),
            ("name", json!("")),
            ("counterStart", json!(1)),
            ("folders", json!("none")),
            ("subfolders", json!(false)),
            ("formats", png.clone()),
        ];
        for (k, v) in defaults.into_iter().chain(case.iter().cloned()) {
            set(&mut app, k, v);
        }
        frame(&mut app);
        let example = export_for_screens::example_name(&app, app.ui.dialog.as_ref().unwrap());
        let r = confirm(&mut app).unwrap();
        let first = r["files"][0].as_str().and_then(|f| f.strip_prefix(&format!("{folder}/"))).unwrap_or_default().to_string();
        if example != first {
            wrong.push(format!("{case:?}: example {example:?}, written {first:?}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The Example line shows before a folder is chosen and is left out while the settings choose
/// nothing to export (the Assets tab with no asset checked); Start at shows the counter the export
/// uses when it was set as text.
#[test]
fn the_example_needs_no_folder_and_start_at_shows_the_counter_used() {
    let (mut app, _) = app(1, false);
    open(&mut app);
    for (k, v) in [("folder", json!("")), ("name", json!("####")), ("counterStart", json!("7"))] {
        set(&mut app, k, v);
    }
    let mut h = ScreensFrame::new();
    let text = crate::tests_labels::shapes_text(&h.settle(&mut app));
    assert!(text.lines().any(|l| l == "Example: 0007.png"), "{text}");
    assert!(text.lines().any(|l| l.trim() == "7"), "{text}");
    set(&mut app, "tab", json!("assets"));
    let text = crate::tests_labels::shapes_text(&h.settle(&mut app));
    assert!(!text.contains("Example:"), "{text}");
}

/// Batch exporting (#1935): a file name pattern and a folder per format, the Layers and Selection
/// tabs, and export presets applied and saved from the dialog.
#[test]
fn batch_naming_folders_layers_selection_and_presets() {
    let (mut app, _) = app(3, false);
    let dir = temp_folder("batch");
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().to_string();
    open(&mut app);
    assert_eq!(field(&app, "folders"), json!("none"));
    // Every tab draws; the OK button says what is exported.
    for (tab, ok) in [("layers", "Export Layer"), ("selection", "Export Selection"), ("assets", "Export Asset"), ("artboards", "Export Artboard")] {
        set(&mut app, "tab", json!(tab));
        frame(&mut app);
        assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), ok);
    }
    // Artboards 1–2 as SVG and print PDF, named by pattern, a folder per format.
    set(&mut app, "folder", json!(folder));
    set(&mut app, "select", json!("range"));
    set(&mut app, "range", json!("1-2"));
    set(&mut app, "name", json!("ACME-<name>-##"));
    set(&mut app, "folders", json!("format"));
    set(
        &mut app,
        "formats",
        json!([{"format": "svg", "scale": "1x", "suffix": ""}, {"format": "pdf", "scale": "1x", "suffix": "-print", "marks": {"trim": true}}]),
    );
    frame(&mut app);
    {
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(export_for_screens::example_name(&app, d), "svg/ACME-Artboard 1-01.svg");
    }
    let r = confirm(&mut app).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    assert_eq!(
        files,
        [
            format!("{folder}/svg/ACME-Artboard 1-01.svg"),
            format!("{folder}/pdf/ACME-Artboard 1-01-print.pdf"),
            format!("{folder}/svg/ACME-Artboard 2-02.svg"),
            format!("{folder}/pdf/ACME-Artboard 2-02-print.pdf")
        ]
    );
    // It reopens on the pattern and the folders.
    open(&mut app);
    assert_eq!((field(&app, "name"), field(&app, "folders")), (json!("ACME-<name>-##"), json!("format")));

    // The Layers tab: one file of a layer's art.
    let layer = app.session.active().unwrap().doc.layers[0].id.0;
    set(&mut app, "folder", json!(folder));
    set(&mut app, "tab", json!("layers"));
    set(&mut app, "layers", json!([layer]));
    set(&mut app, "name", json!(""));
    set(&mut app, "folders", json!("none"));
    set(&mut app, "formats", json!([{"format": "png", "scale": "1x", "suffix": ""}]));
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["files"][0], json!(format!("{folder}/Layer-1.png")));

    // The Selection tab: nothing selected keeps the dialog open; a file per selected object.
    open(&mut app);
    app.run("select.none", json!({})).unwrap();
    set(&mut app, "folder", json!(folder));
    set(&mut app, "tab", json!("selection"));
    frame(&mut app);
    assert!(confirm(&mut app).is_err());
    app.run("select.all", json!({})).unwrap();
    set(&mut app, "selectionMode", json!("each"));
    set(&mut app, "name", json!("piece-#"));
    set(&mut app, "formats", json!([{"format": "svg", "scale": "1x", "suffix": ""}]));
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["files"][0], json!(format!("{folder}/piece-1.svg")));

    // Presets: the Logo Package sets the formats, naming and folders; the dialog saves its own.
    open(&mut app);
    let presets = field(&app, "__exportPresets");
    let logo = presets.as_array().unwrap().iter().find(|p| p["name"] == "Logo Package").unwrap().clone();
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &logo);
    assert_eq!(field(&app, "formats").as_array().unwrap().len(), 5);
    assert_eq!(
        (field(&app, "folders"), field(&app, "name"), field(&app, "exportPreset")),
        (json!("format"), json!("<filename>-<name>"), json!("Logo Package"))
    );
    frame(&mut app);
    set(&mut app, "folder", json!(folder));
    set(&mut app, "__savePreset", json!("Client Pack"));
    frame(&mut app);
    let saved = app
        .run("document.exportPresets.save", json!({"name": "Client Pack", "settings": {"formats": field(&app, "formats"), "folders": "item"}}))
        .unwrap();
    assert_eq!(saved["name"], "Client Pack");
    let list = export_for_screens::export_presets(&mut app);
    assert!(list.as_array().unwrap().iter().any(|p| p["name"] == "Client Pack" && p["builtIn"] == false));
    let _ = std::fs::remove_dir_all(&dir);
}
