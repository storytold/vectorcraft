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
    assert_eq!((field(&app, "openLocation"), field(&app, "subfolders"), field(&app, "preset")), (json!(true), json!(true), json!("")));
    assert_eq!((field(&app, "subfolderBy"), field(&app, "pdfs"), field(&app, "view")), (json!("scale"), json!("multiple"), json!("grid")));
    assert_eq!((field(&app, "includeBleed"), field(&app, "includeArtboardFill")), (json!(false), json!(true)));
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
            crate::canvas::dispatch(app, &PointerEvent { kind, pos: boards[i].center(), mods, backquote: false, pressure: 1.0, pen: None }, view);
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
    // Create Sub-folders is on by default, a folder per scale.
    assert_eq!(r["files"], json!(["1x/Artboard-2.png", "1x/Artboard-3.png"]));
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
    assert_eq!(app.ui.status, format!("Exported 1 file to {folder}"));
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
    assert_eq!(r["files"], json!(["1x/Artboard-1.png", "2x/Artboard-1@2x.jpg", "1x/Artboard-2.png", "2x/Artboard-2@2x.jpg"]));
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

/// Include Artboard Fill: checked, the file shows the artboard's background color behind the art
/// (on transparency too); unchecked, it doesn't, and the dialog reopens unchecked.
#[test]
fn include_artboard_fill_reaches_the_files_and_is_remembered() {
    let (mut app, log) = app(1, true);
    app.run("artboard.setProps", json!({"index": 0, "color": "#ff0000"})).unwrap();
    for include in [true, false] {
        log.borrow_mut().clear();
        open(&mut app);
        assert_eq!(field(&app, "includeArtboardFill"), json!(true), "on until unchecked");
        set(&mut app, "includeArtboardFill", json!(include));
        set(&mut app, "settings", json!({"png": {"background": "transparent"}}));
        frame(&mut app);
        confirm(&mut app).unwrap();
        let px = image::load_from_memory(&log.borrow()[0].1).unwrap().to_rgba8().get_pixel(58, 38).0;
        assert_eq!(px, if include { [255, 0, 0, 255] } else { [0, 0, 0, 0] }, "include {include}");
        if include {
            continue;
        }
        open(&mut app);
        assert_eq!(field(&app, "includeArtboardFill"), json!(false), "remembered");
    }
}

#[test]
fn the_status_bar_says_when_an_image_could_not_be_decoded() {
    const NOTE: &str = "with 1 note: an image couldn't be decoded and is missing from the picture";
    // A PNG whose header reads but whose pixel data is cut short: the renderer draws nothing for it.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let place = |app: &mut VectorcraftApp| vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({"at": [10, 10]}));
    let rows = json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "jpg", "scale": "2x", "suffix": "@2x"}]);
    let dir = temp_folder("undecoded");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut desktop, log) = app(1, false);
    desktop.run("file.exportForScreens", json!({"folder": folder, "formats": rows})).unwrap();
    assert_eq!(desktop.ui.status, format!("Exported 2 files to {folder}"));
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

/// An export into a folder with several notes, from Export for Screens and from the Asset Export
/// panel: the status bar names the first, counts them, and keeps every one of them for its hover.
#[test]
fn an_export_into_a_folder_keeps_every_note_for_the_hover() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let dir = temp_folder("notes");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let rows = json!([{"format": "png", "scale": "1x", "suffix": ""}]);
    let (mut app, _) = app(1, false);
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({"at": [10, 10]}));
    app.run("text.create", json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let r = app.run("file.exportForScreens", json!({"folder": folder, "formats": rows})).unwrap();
    let notes = crate::io::notes_of(&r);
    assert_eq!(notes.len(), 2, "{r}");
    assert!(app.ui.status.starts_with(&format!("Saved {folder} with 2 notes: {}", notes[0])), "{}", app.ui.status);
    assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));
    app.ui.status_notes = None;
    app.run("select.all", json!({})).unwrap();
    app.run("assets.add", json!({"multiple": false})).unwrap();
    let r = crate::io::export_files(&mut app, "assets.export", json!({"folder": folder, "formats": rows})).unwrap();
    let notes = crate::io::notes_of(&r);
    assert_eq!(notes.len(), 2, "{r}");
    assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)), "assets.export");
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
    cpu: Option<ScreensCpu>,
}

impl ScreensFrame {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0, cpu: None }
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
        if let Some(cpu) = &mut self.cpu {
            cpu.update(&out);
        }
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
    // The existing 24 pt gear is right-aligned in the dialog's 370 pt column or the 236 pt panel.
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
    let gear = screen_format_gear(&out, if asset_panel { 236.0 } else { 370.0 });
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
    set(&mut app, "subfolders", json!(false));
    let mut h = ScreensFrame::new();
    h.choose(&mut app, "PNG", "TIFF");
    assert_eq!(field(&app, "formats"), json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    // Leave only the first of two boards checked.
    h.click_label(&mut app, "2  Artboard 2");
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

/// The existing export controls stay visible beside the filename pattern and counter controls.
#[test]
fn filename_pattern_controls_keep_the_existing_export_layout() {
    let (mut app, _) = app(3, false);
    open(&mut app);
    let text = crate::tests_labels::shapes_text(&ScreensFrame::new().settle(&mut app));
    for label in [
        "Artboards",
        "Assets",
        "Select:",
        "Full Document",
        "Include Bleed",
        "Include Artboard Fill",
        "Export to:",
        "Open Location after Export",
        "Create Sub-folders",
        "Scale",
        "Format",
        "Export PDFs As:",
        "Single File",
        "Multiple Files",
        "Formats:",
        "iOS",
        "Android",
        "+ Add Scale",
        "Clear Selection",
        "Prefix:",
        "File Name:",
        "Start at:",
        "Count:",
        "Per Item",
        "Selected: 3, Total number of files to export: 3",
    ] {
        assert!(text.lines().any(|l| l.trim() == label), "{label} missing:\n{text}");
    }
    for gone in ["Layers", "Selection", "Export Preset:", "Folders:"] {
        assert!(!text.lines().any(|l| l.trim() == gone), "{gone} shown:\n{text}");
    }
}

/// iOS and Android set the rows those platforms use (in sub-folders); Clear Selection unchecks
/// every artboard, so nothing is exported until one is checked again.
#[test]
fn ios_android_and_clear_selection() {
    let (mut app, _) = app(2, false);
    open(&mut app);
    set(&mut app, "subfolders", json!(false));
    let mut h = ScreensFrame::new();
    h.click_label(&mut app, "iOS");
    let scales: Vec<Value> = field(&app, "formats").as_array().unwrap().iter().map(|r| r["scale"].clone()).collect();
    assert_eq!((field(&app, "preset"), field(&app, "subfolders")), (json!("mobile"), json!(true)));
    assert_eq!(scales, [json!("1x"), json!("2x"), json!("3x")]);
    h.click_label(&mut app, "Android");
    assert_eq!((field(&app, "preset"), field(&app, "formats").as_array().unwrap().len()), (json!("density"), 6));
    h.click_label(&mut app, "Clear Selection");
    assert_eq!((field(&app, "boards"), field(&app, "select")), (json!([false, false]), json!("range")));
    let text = crate::tests_labels::shapes_text(&h.settle(&mut app));
    assert!(text.contains("Selected: 0, Total number of files to export: 0"), "{text}");
    set(&mut app, "folder", json!(temp_folder("cleared").to_string_lossy()));
    assert!(confirm(&mut app).is_err(), "nothing to export");
}

/// Create Sub-folders sorts the files into a folder per scale or per format; Export PDFs As ›
/// Single file writes the chosen artboards as one PDF named after the document.
#[test]
fn sub_folders_and_single_file_pdfs() {
    let (mut app, _) = app(3, false);
    let dir = temp_folder("subfolders");
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().to_string();
    let title = app.session.active().unwrap().doc.title.clone();
    let rows = json!([
        {"format": "png", "scale": "1x", "suffix": ""},
        {"format": "png", "scale": "2x", "suffix": "@2x"},
        {"format": "pdf", "scale": "1x", "suffix": ""}
    ]);
    let files = |app: &mut VectorcraftApp, extra: &[(&str, Value)]| -> Vec<String> {
        open(app);
        let base = [("folder", json!(folder)), ("select", json!("range")), ("range", json!("1-2")), ("formats", rows.clone())];
        for (k, v) in base.into_iter().chain(extra.iter().cloned()) {
            set(app, k, v);
        }
        frame(app);
        let r = confirm(app).unwrap();
        r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().strip_prefix(&format!("{folder}/")).unwrap().to_string()).collect()
    };
    let by_scale = files(&mut app, &[("subfolders", json!(true)), ("subfolderBy", json!("scale"))]);
    assert_eq!(&by_scale[..2], ["1x/Artboard-1.png", "2x/Artboard-1@2x.png"]);
    let by_format = files(&mut app, &[("subfolders", json!(true)), ("subfolderBy", json!("format")), ("pdfs", json!("single"))]);
    let pdf = format!("pdf/{title}.pdf");
    assert_eq!(by_format, ["png/Artboard-1.png", "png/Artboard-1@2x.png", pdf.as_str(), "png/Artboard-2.png", "png/Artboard-2@2x.png"]);
    let flat = files(&mut app, &[("subfolders", json!(false)), ("pdfs", json!("multiple"))]);
    assert_eq!(flat.len(), 6);
    assert!(flat.contains(&"Artboard-2.pdf".to_string()), "{flat:?}");
    // The status line counts the files Single file writes.
    open(&mut app);
    for (k, v) in [("select", json!("range")), ("range", json!("1-2")), ("formats", rows.clone()), ("pdfs", json!("single"))] {
        set(&mut app, k, v);
    }
    let text = crate::tests_labels::shapes_text(&ScreensFrame::new().settle(&mut app));
    assert!(text.contains("Selected: 2, Total number of files to export: 5"), "{text}");
    // A format preset sets the rows, their settings and the sub-folders, not a prefix it lacks.
    set(&mut app, "prefix", json!("x-"));
    let presets = field(&app, "__exportPresets");
    let logo = presets.as_array().unwrap().iter().find(|p| p["name"] == "Logo Package").unwrap().clone();
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &logo);
    assert_eq!(field(&app, "formats").as_array().unwrap().len(), 5);
    assert_eq!(
        (field(&app, "subfolders"), field(&app, "subfolderBy"), field(&app, "exportPreset")),
        (json!(true), json!("format"), json!("Logo Package"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save Preset keeps the edited naming controls with the rows, prefix and sub-folders, without
/// discarding an applied preset's other export options.
#[test]
fn filename_pattern_edits_are_saved_in_presets_without_losing_other_options() {
    use export_for_screens::PresetPick;
    let (mut app, _) = app(2, false);
    app.run(
        "document.exportPresets.save",
        json!({
            "name": "Repeat",
            "settings": {"formats": [{"format": "png"}], "name": "<name>-###", "counterStart": 7, "counterPer": "file", "antiAlias": "none", "prefix": "a-"}
        }),
    )
    .unwrap();
    open(&mut app);
    let preset = export_for_screens::export_presets(&mut app).as_array().unwrap().iter().find(|p| p["name"] == "Repeat").cloned().unwrap();
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &preset);
    assert_eq!((field(&app, "name"), field(&app, "counterStart"), field(&app, "counterPer")), (json!("<name>-###"), json!(7), json!("file")));
    set(&mut app, "name", json!("new-####"));
    set(&mut app, "counterStart", json!("9"));
    set(&mut app, "counterPer", json!("item"));
    set(&mut app, "prefix", json!("b-"));
    set(&mut app, "subfolders", json!(true));
    set(&mut app, "subfolderBy", json!("format"));
    let mut d = app.ui.dialog.take().unwrap();
    export_for_screens::preset_pick(&mut app, &mut d, PresetPick::Save("Repeat".into()));
    app.ui.dialog = Some(d);

    let saved = app.session.prefs.screen_export_presets.iter().find(|p| p.name == "Repeat").unwrap_or_else(|| panic!("{}", app.ui.status));
    let s = &saved.settings;
    assert_eq!((s.get("counterPer"), s.get("antiAlias"), s.get("name")), (Some(&json!("item")), Some(&json!("none")), Some(&json!("new-####"))));
    assert_eq!(s.get("counterStart"), Some(&json!(9)));
    assert_eq!((s.get("prefix"), s.get("folders")), (Some(&json!("b-")), Some(&json!("format"))));
    assert_eq!(field(&app, "exportPreset"), "Repeat");

    // Null names are absent to the real preset backend, so retain these valid dialog edits.
    let retained = (json!("new-####"), json!("b-"), json!("9"), json!("item"));
    assert_eq!((field(&app, "name"), field(&app, "prefix"), field(&app, "counterStart"), field(&app, "counterPer")), retained);
    let null_preset = app
        .run(
            "document.exportPresets.save",
            json!({
                "name": "Unspecified naming",
                "settings": {"formats": [{"format": "png"}], "name": null, "prefix": null, "counterStart": null, "counterPer": null}
            }),
        )
        .unwrap();
    for key in ["name", "prefix", "counterStart", "counterPer"] {
        assert_eq!(null_preset["settings"].get(key), Some(&Value::Null), "the saved preset retains {key}: null");
    }
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &null_preset);
    assert_eq!((field(&app, "name"), field(&app, "prefix"), field(&app, "counterStart"), field(&app, "counterPer")), retained);
    assert_eq!(field(&app, "exportPreset"), "Unspecified naming");
}

/// Save Preset… on an applied preset saves the dialog's Include Bleed and Include Artboard Fill.
#[test]
fn saving_an_applied_preset_keeps_its_bleed_and_fill() {
    use export_for_screens::PresetPick;
    let (mut app, _) = app(2, false);
    let settings = json!({"formats": [{"format": "png"}], "includeBleed": true, "includeArtboardFill": false});
    app.run("document.exportPresets.save", json!({"name": "Bleed", "settings": settings})).unwrap();
    open(&mut app);
    let preset = export_for_screens::export_presets(&mut app).as_array().unwrap().iter().find(|p| p["name"] == "Bleed").cloned().unwrap();
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &preset);
    let mut d = app.ui.dialog.take().unwrap();
    export_for_screens::preset_pick(&mut app, &mut d, PresetPick::Save("Bleed".into()));
    app.ui.dialog = Some(d);

    let saved = app.session.prefs.screen_export_presets.iter().find(|p| p.name == "Bleed").unwrap_or_else(|| panic!("{}", app.ui.status));
    let s = &saved.settings;
    assert_eq!((s.get("includeBleed"), s.get("includeArtboardFill")), (Some(&json!(true)), Some(&json!(false))));
}

/// Remembered settings and a preset that hold keys the export no longer reads, in a row, in a
/// format's settings and inside an object option, and settings for a format it doesn't write, open
/// without them, so the dialog's export is not refused.
#[test]
fn saved_keys_no_longer_read_are_left_out_of_the_dialog_s_export() {
    let (mut app, _) = app(1, false);
    let stale = json!({
        "formats": [{"format": "png", "scale": "2x", "noSuchOption": 1}, {"format": "pdf", "marks": {"trim": true, "noSuchMark": 1}}],
        "settings": {"png": {"noSuchSetting": true}, "dxf": {}, "pdf": {"bleed": {"top": 1, "inner": 2}}}
    });
    let Value::Object(saved) = stale.clone() else { panic!("an object") };
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).export_settings = saved;
    let folder = temp_folder("stale");
    let check = |app: &VectorcraftApp| {
        let p = export_for_screens::params(app, app.ui.dialog.as_ref().unwrap()).unwrap();
        app.session.check_params("document.exportForScreens", &p).unwrap_or_else(|e| panic!("{e}: {p}"));
        p
    };
    open(&mut app);
    set(&mut app, "folder", json!(folder.to_string_lossy()));
    let p = check(&app);
    assert_eq!(p["formats"][0]["scale"], json!("2x"));
    assert_eq!(p["formats"][1]["marks"]["trim"], json!(true));
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &json!({"name": "Old", "settings": stale}));
    check(&app);
    let _ = std::fs::remove_dir_all(&folder);
}

/// A preset that puts each item in its own folder (`item` or `itemFormat`, which the dialog has no
/// control for) keeps that folder mode when it is exported or saved again from the dialog, until
/// the sub-folder controls change (in the dialog or by `ui.dialog.set`).
#[test]
fn an_applied_preset_keeps_its_item_folders() {
    use crate::control::{ControlRequest, Outcome};
    use export_for_screens::PresetPick;
    for mode in ["item", "itemFormat"] {
        let (mut app, _) = app(2, false);
        app.run("document.exportPresets.save", json!({"name": "Items", "settings": {"formats": [{"format": "png"}], "folders": mode}})).unwrap();
        open(&mut app);
        let preset = export_for_screens::export_presets(&mut app).as_array().unwrap().iter().find(|p| p["name"] == "Items").cloned().unwrap();
        export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &preset);
        set(&mut app, "folder", json!("out"));
        let p = export_for_screens::params(&app, app.ui.dialog.as_ref().unwrap()).unwrap();
        assert_eq!(p["folders"], json!(mode), "exported");
        let mut d = app.ui.dialog.take().unwrap();
        export_for_screens::preset_pick(&mut app, &mut d, PresetPick::Save("Items".into()));
        app.ui.dialog = Some(d);
        let saved = app.session.prefs.screen_export_presets.iter().find(|p| p.name == "Items").unwrap_or_else(|| panic!("{}", app.ui.status));
        assert_eq!(saved.settings.get("folders"), Some(&json!(mode)), "saved");

        // `ui.dialog.set` of the sub-folder controls, as a change in the dialog, ends the item folders.
        for (on, folders) in [(false, "none"), (true, "rows")] {
            let (req, _rx) = ControlRequest::new("ui.dialog.set", json!({"field": "subfolders", "value": on}));
            let Outcome::Done(r) = crate::control::handle(&mut app, &egui::Context::default(), &req) else { panic!("not done") };
            assert_eq!(r["ok"], true, "{r}");
            let p = export_for_screens::params(&app, app.ui.dialog.as_ref().unwrap()).unwrap();
            assert_eq!(p["folders"], json!(folders), "subfolders {on}");
        }
    }
}

/// A preset whose `folders` is empty turns Create Sub-folders off whatever its `subfolders`, as
/// the export command puts every file in the folder itself; without `folders` it uses its
/// `subfolders`, and without either it keeps the dialog's sub-folders.
#[test]
fn a_preset_with_empty_folders_exports_into_the_folder() {
    let (mut app, _) = app(2, false);
    open(&mut app);
    let apply = |app: &mut VectorcraftApp, settings: Value| {
        export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &json!({"name": "Empty", "settings": settings}));
    };
    for settings in [json!({"folders": ""}), json!({"folders": "", "subfolders": true})] {
        set(&mut app, "subfolders", json!(true));
        apply(&mut app, settings.clone());
        assert_eq!(field(&app, "subfolders"), json!(false), "{settings}");
    }
    apply(&mut app, json!({"subfolders": true}));
    assert_eq!(field(&app, "subfolders"), json!(true));
    set(&mut app, "subfolderBy", json!("format"));
    apply(&mut app, json!({}));
    assert_eq!((field(&app, "subfolders"), field(&app, "subfolderBy")), (json!(true), json!("format")));
}

/// The Asset Export panel menu's Export Preset… writes a preset to a file and its Import Preset… adds
/// the presets of a file, which the open dialog's dropdown then lists, as it does after opening
/// the file (the web's Import).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_preset_and_import_preset_go_through_files() {
    let dir = temp_folder("preset-files");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pack.vcexportpresets").to_string_lossy().to_string();
    let (saved_to, picked) = (file.clone(), file.clone());
    let suggested = Rc::new(RefCell::new(vec![]));
    let shown = suggested.clone();
    let services = Services {
        pick_save: Some(Box::new(move |pick: &crate::FilePick| {
            shown.borrow_mut().push(pick.name.clone());
            Some(saved_to.clone())
        })),
        pick_open: Some(Box::new(move |_: &crate::FilePick| Some(picked.clone()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("document.exportPresets.save", json!({"name": "Pack", "settings": {"formats": [{"format": "svg"}], "folders": "format"}})).unwrap();
    open(&mut app);
    let names = |app: &VectorcraftApp| -> Vec<String> {
        field(app, "__exportPresets").as_array().unwrap().iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect()
    };
    assert_eq!(export_for_screens::export_preset(&mut app, "Pack").unwrap(), file);
    // The save panel suggests the preset's name, not the document's.
    assert_eq!(*suggested.borrow(), ["Pack.vcexportpresets"]);
    app.run("document.exportPresets.delete", json!({"name": "Pack"})).unwrap();
    frame(&mut app);
    assert_eq!(names(&app), ["Logo Package", "Print-Ready PDF"]);
    assert_eq!(export_for_screens::import_presets(&mut app).unwrap(), ["Pack"]);
    assert_eq!(app.ui.status, "Imported export presets: Pack");
    frame(&mut app);
    assert_eq!(names(&app), ["Logo Package", "Print-Ready PDF", "Pack"]);
    let pack = field(&app, "__exportPresets")[2].clone();
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &pack);
    assert_eq!((field(&app, "formats")[0]["format"].clone(), field(&app, "subfolderBy")), (json!("svg"), json!("format")));
    // The file opened again: a second copy, numbered, in the open dialog's dropdown.
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    frame(&mut app);
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    assert_eq!(names(&app), ["Logo Package", "Print-Ready PDF", "Pack", "Pack 2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Export Preset… suggests the preset's name as a file name, as Export Library… does a library's.
#[test]
fn an_exported_preset_file_is_named_after_the_preset() {
    let name = export_for_screens::preset_file_name;
    assert_eq!(name("Logo Package 2"), "Logo Package 2.vcexportpresets");
    assert_eq!(name("Print-Ready PDF"), "Print-Ready PDF.vcexportpresets");
    assert_eq!(name("Web/App: v1.2"), "Web-App- v1-2.vcexportpresets");
    assert_eq!(name("CON"), "CON-.vcexportpresets");
}

/// A preset saved with the mobile or density rows (as the Asset Export panel saves one) sets the
/// dialog's rows to them.
#[test]
fn a_mobile_preset_sets_the_rows() {
    let (mut app, _) = app(1, false);
    app.run("document.exportPresets.save", json!({"name": "Phones", "settings": {"preset": "mobile", "subfolders": true}})).unwrap();
    open(&mut app);
    let phones = export_for_screens::export_presets(&mut app).as_array().unwrap().iter().find(|p| p["name"] == "Phones").cloned().unwrap();
    set(&mut app, "subfolders", json!(false));
    set(&mut app, "subfolderBy", json!("format"));
    export_for_screens::apply_preset(app.ui.dialog.as_mut().unwrap(), &phones);
    assert_eq!(field(&app, "formats"), json!(fileio::screen_preset_rows("mobile").unwrap()));
    assert_eq!((field(&app, "preset"), field(&app, "subfolders"), field(&app, "subfolderBy")), (json!("mobile"), json!(true), json!("scale")));
}

/// `ui.dialog.set` of `exportPreset` applies the preset it names, as a pick in the ☰ menu does, so
/// the dialog holds the settings of the preset it names; a name that is no preset's is an error and
/// leaves the dialog as it was.
#[test]
fn setting_the_export_preset_field_applies_the_preset() {
    use crate::control::{ControlRequest, Outcome};
    let (mut app, _) = app(1, false);
    let ctx = egui::Context::default();
    open(&mut app);
    let set_preset = |app: &mut VectorcraftApp, name: &str| {
        let (req, _rx) = ControlRequest::new("ui.dialog.set", json!({"field": "exportPreset", "value": name}));
        let Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
        r
    };
    let r = set_preset(&mut app, "Logo Package");
    assert_eq!(r["ok"], true, "{r}");
    let rows: Vec<Value> = field(&app, "formats").as_array().unwrap().iter().map(|r| r["format"].clone()).collect();
    assert_eq!(rows, [json!("svg"), json!("png"), json!("png"), json!("pdf"), json!("pdf")]);
    assert_eq!(field(&app, "subfolderBy"), json!("format"));
    assert_eq!(r["result"]["fields"]["subfolderBy"], "format", "{r}");
    let r = set_preset(&mut app, "Never saved");
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(field(&app, "exportPreset"), "Logo Package");
    assert_eq!(field(&app, "subfolderBy"), "format");
    // A request that fails on a later field changes none of its fields.
    let fields = json!({"fields": [{"field": "subfolderBy", "value": "scale"}, {"field": "exportPreset", "value": "Never saved"}]});
    let (req, _rx) = ControlRequest::new("ui.dialog.set", fields);
    let Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!((field(&app, "exportPreset"), field(&app, "subfolderBy")), (json!("Logo Package"), json!("format")));
    // In one request the fields apply in order: a field after the preset goes over it.
    let fields = json!({"fields": [{"field": "exportPreset", "value": "Logo Package"}, {"field": "subfolderBy", "value": "scale"}]});
    let (req, _rx) = ControlRequest::new("ui.dialog.set", fields);
    let Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!((field(&app, "exportPreset"), field(&app, "subfolderBy")), (json!("Logo Package"), json!("scale")));
}

/// The Asset Export panel menu's Import Preset… and Export Preset… show their file dialogs off the UI
/// thread (as sheets on macOS) and read or write the file when the dialog answers.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn preset_files_ask_off_the_ui_thread() {
    use crate::panels::asset_export;
    use crate::picks::PickRequest;
    use export_for_screens::PresetPick;
    let mut services = Services { dialogs_are_sheets: true, ..Default::default() };
    let (shown, written) = crate::tests_picks::off_the_ui_thread(&mut services);
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("document.exportPresets.save", json!({"name": "Pack", "settings": {"formats": [{"format": "svg"}]}})).unwrap();
    let dir = temp_folder("preset-sheets");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pack.vcexportpresets");
    std::fs::write(&file, app.run("document.exportPresets.export", json!({"names": ["Pack"]})).unwrap()["data"].as_str().unwrap()).unwrap();
    let file = file.to_string_lossy().to_string();
    let names = |app: &VectorcraftApp| app.session.prefs.screen_export_presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    let poll = |app: &mut VectorcraftApp| crate::picks::poll(app, &egui::Context::default());
    let ctx = egui::Context::default();
    let doc = app.session.active().unwrap().uid;
    asset_export::preset(&mut app, &ctx, doc, PresetPick::Export("Pack".into()));
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Save(p)] if p.name == "Pack.vcexportpresets"), "{:?}", shown.requests());
    assert!(written.borrow().is_empty() && app.ui.status.is_empty(), "{}", app.ui.status);
    shown.answer(&["/presets/pack.vcexportpresets"]);
    poll(&mut app);
    assert_eq!(*written.borrow(), ["/presets/pack.vcexportpresets"], "{}", app.ui.status);
    asset_export::preset(&mut app, &ctx, doc, PresetPick::Import);
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]), "{:?}", shown.requests());
    assert_eq!(names(&app), ["Pack"]);
    shown.answer(&[&file]);
    poll(&mut app);
    assert_eq!(names(&app), ["Pack", "Pack 2"], "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The dialog's Export Preset row has no Import Preset… or Export Preset…: those picks change
/// nothing there.
#[test]
fn the_dialog_leaves_preset_files_to_the_panel() {
    use export_for_screens::PresetPick;
    let (mut app, _) = app(1, false);
    open(&mut app);
    let before = serde_json::to_value(&app.ui.dialog).unwrap();
    for pick in [PresetPick::Import, PresetPick::Export("Logo Package".into())] {
        let mut d = app.ui.dialog.take().unwrap();
        export_for_screens::preset_pick(&mut app, &mut d, pick);
        app.ui.dialog = Some(d);
    }
    assert_eq!(serde_json::to_value(&app.ui.dialog).unwrap(), before);
    assert_eq!(app.ui.status, "");
}

/// The dialog keeps its list of export presets from frame to frame, and builds it again when the
/// saved presets change.
#[test]
fn the_dialog_builds_the_preset_list_again_only_when_the_presets_change() {
    let (mut app, _) = app(1, false);
    open(&mut app);
    frame(&mut app);
    let mark = |app: &mut VectorcraftApp| app.ui.dialog.as_mut().unwrap().fields.get_mut("__exportPresets").unwrap()[0]["mark"] = json!(true);
    mark(&mut app);
    frame(&mut app);
    frame(&mut app);
    assert_eq!(field(&app, "__exportPresets")[0]["mark"], true, "the list was not built again");
    app.run("document.exportPresets.save", json!({"name": "Mine", "settings": {"formats": [{"format": "svg"}]}})).unwrap();
    frame(&mut app);
    let list = field(&app, "__exportPresets");
    assert!(list[0].get("mark").is_none() && list.as_array().unwrap().iter().any(|p| p["name"] == "Mine"), "{list}");
    // A saved preset changed under the same name is a change too.
    mark(&mut app);
    app.run("document.exportPresets.save", json!({"name": "Mine", "settings": {"formats": [{"format": "pdf"}]}})).unwrap();
    frame(&mut app);
    let list = field(&app, "__exportPresets");
    let mine = list.as_array().unwrap().iter().find(|p| p["name"] == "Mine").unwrap();
    assert_eq!(mine["settings"]["formats"][0]["format"], "pdf", "{list}");
}

/// Read the actual download through the existing bounded, CRC-checking ZIP reader. Its lookups
/// ignore ASCII case; the caller separately checks exact reported filename spelling. Four expected
/// paths and a four-entry limit leave no room for missing, duplicate or unrelated entries.
fn assert_filename_zip_download(log: &Log, index: usize, expected: &Value) {
    let expected = expected.as_array().unwrap();
    assert_eq!(expected.len(), 4);
    let log = log.borrow();
    let archive = fileio::zip::Archive::open(&log[index].1, expected.len()).unwrap();
    for name in expected {
        let name = name.as_str().unwrap();
        let bytes = archive.read(name, 1024 * 1024).unwrap().unwrap_or_else(|| panic!("missing ZIP entry {name}"));
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{name} is a PNG payload");
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
        assert!(image.width() > 0 && image.height() > 0, "{name} decodes to nonempty pixels");
    }
}

/// Actual text/popup input reaches exported filenames for both artboards and collected selection
/// assets. Cancel discards draft settings; blank names restore the existing item/prefix/suffix path.
#[test]
fn filename_pattern_input_exports_artboards_and_collected_assets_and_reopens() {
    let type_field = |h: &mut ScreensFrame, app: &mut VectorcraftApp, label: &str, offset: f32, value: &str| {
        let out = h.settle(app);
        let at = screen_text_at(&out, label) + egui::vec2(offset, 0.0);
        h.click(app, at);
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut events = vec![key(egui::Key::A, egui::Modifiers::COMMAND), key(egui::Key::Backspace, egui::Modifiers::NONE)];
        if !value.is_empty() {
            events.push(egui::Event::Text(value.into()));
        }
        h.frame(app, events);
    };
    for collected in [false, true] {
        let (mut app, log) = app(2, true);
        if collected {
            let mut rectangle = |x| app.run("shape.rectangle", json!({"x": x, "y": 5, "width": 8, "height": 6})).unwrap()["id"].clone();
            let ids = [rectangle(2), rectangle(20)];
            app.run("select.set", json!({"ids": ids})).unwrap();
        }
        let open_tab = |app: &mut VectorcraftApp| {
            if collected {
                app.run("file.exportSelection", json!({"multiple": true})).unwrap();
            } else {
                open(app);
            }
        };
        open_tab(&mut app);
        let saved_before = app.session.active().unwrap().doc.export_settings.clone();
        assert_eq!((field(&app, "name"), field(&app, "counterStart"), field(&app, "counterPer")), (json!(""), json!(1), json!("item")));
        let mut h = ScreensFrame::new();
        type_field(&mut h, &mut app, "File Name:", 100.0, "discard-####");
        assert_eq!(field(&app, "name"), "discard-####", "the actual field accepts typing");
        super::cancel(&mut app);
        assert!(log.borrow().is_empty());
        assert_eq!(app.session.active().unwrap().doc.export_settings, saved_before);
        open_tab(&mut app);
        assert_eq!(field(&app, "name"), "", "Cancel did not save the draft name");

        let formats = json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "png", "scale": "2x", "suffix": "@2x"}]);
        set(&mut app, "formats", formats.clone());
        set(&mut app, "prefix", json!("proof-"));
        type_field(&mut h, &mut app, "File Name:", 100.0, "batch-####");
        type_field(&mut h, &mut app, "Start at:", 65.0, "7");
        h.choose(&mut app, "Per Item", "Per File");
        assert_eq!((field(&app, "name"), field(&app, "counterStart"), field(&app, "counterPer")), (json!("batch-####"), json!("7"), json!("file")));
        let settings = field(&app, "settings");
        let result = confirm(&mut app).unwrap();
        let per_file = json!(["1x/proof-batch-0007.png", "2x/proof-batch-0008@2x.png", "1x/proof-batch-0009.png", "2x/proof-batch-0010@2x.png"]);
        assert_eq!(result["files"], per_file, "artboards/assets={collected}");
        assert_filename_zip_download(&log, 0, &per_file);
        assert_eq!(log.borrow().len(), 1, "the real export produced a download");
        assert!(log.borrow()[0].1.starts_with(b"PK"), "four encoded files are downloaded as a ZIP");
        assert!(app.ui.dialog.is_none());
        open_tab(&mut app);
        assert_eq!((field(&app, "name"), field(&app, "counterStart"), field(&app, "counterPer")), (json!("batch-####"), json!(7), json!("file")));
        assert_eq!((field(&app, "prefix"), field(&app, "formats"), field(&app, "settings")), (json!("proof-"), formats.clone(), settings));

        type_field(&mut h, &mut app, "File Name:", 100.0, "");
        h.choose(&mut app, "Per File", "Per Item");
        let result = confirm(&mut app).unwrap();
        let item = if collected { "Asset" } else { "Artboard" };
        let original_names = json!([
            format!("1x/proof-{item}-1.png"),
            format!("2x/proof-{item}-1@2x.png"),
            format!("1x/proof-{item}-2.png"),
            format!("2x/proof-{item}-2@2x.png")
        ]);
        assert_eq!(result["files"], original_names, "blank uses the original names and keeps prefix/suffix");
        assert_filename_zip_download(&log, 1, &original_names);
        assert_eq!(log.borrow().len(), 2);
        open_tab(&mut app);
        assert_eq!((field(&app, "name"), field(&app, "counterPer")), (json!(""), json!("item")));
        // Per Item must share a counter between an item's scales even when a nonblank
        // pattern uses it; clearing the pattern above alone would not exercise this setting.
        type_field(&mut h, &mut app, "File Name:", 100.0, "batch-####");
        let result = confirm(&mut app).unwrap();
        let per_item = json!(["1x/proof-batch-0007.png", "2x/proof-batch-0007@2x.png", "1x/proof-batch-0008.png", "2x/proof-batch-0008@2x.png"]);
        assert_eq!(result["files"], per_item, "each item's two scales share its counter: assets={collected}");
        assert_eq!(log.borrow().len(), 3);
        assert_filename_zip_download(&log, 2, &per_item);
        open_tab(&mut app);
        assert_eq!((field(&app, "name"), field(&app, "counterPer")), (json!("batch-####"), json!("item")));

        // This odd integer cannot survive a round-trip through f64. The actual filenames,
        // archive entries and reopened field all retain it, including the next item's counter.
        type_field(&mut h, &mut app, "Start at:", 65.0, "9007199254740993");
        let result = confirm(&mut app).unwrap();
        let exact = json!([
            "1x/proof-batch-9007199254740993.png",
            "2x/proof-batch-9007199254740993@2x.png",
            "1x/proof-batch-9007199254740994.png",
            "2x/proof-batch-9007199254740994@2x.png"
        ]);
        assert_eq!(result["files"], exact, "large counters stay exact in output names: assets={collected}");
        assert_eq!(log.borrow().len(), 4);
        assert_filename_zip_download(&log, 3, &exact);
        open_tab(&mut app);
        assert_eq!(field(&app, "counterStart"), json!(9007199254740993_u64), "reopened counter stays exact");
        let saved = app.session.active().unwrap().doc.export_settings.clone();
        for invalid in ["-1", "18446744073709551616"] {
            type_field(&mut h, &mut app, "Start at:", 65.0, invalid);
            assert!(confirm(&mut app).is_err(), "invalid counter keeps the dialog and writes nothing: {invalid}");
            assert_eq!(kind(&app), Some(export_for_screens::KIND));
            assert_eq!(log.borrow().len(), 4);
            assert_eq!(app.session.active().unwrap().doc.export_settings, saved);
        }
    }
}

// CPU-only images of production dialog meshes. Unknown content refuses the capture.
#[derive(Default)]
struct ScreensCpu {
    textures: std::collections::HashMap<egui::TextureId, egui::ColorImage>,
    free_after_paint: Vec<egui::TextureId>,
}

impl ScreensCpu {
    fn update(&mut self, out: &egui::FullOutput) {
        // Previous frees can retire now; this frame must still paint its own freed textures.
        for id in self.free_after_paint.drain(..) {
            self.textures.remove(&id);
        }
        for (id, patches) in &out.textures_delta.set {
            assert!(matches!(id, egui::TextureId::Managed(_)), "unavailable user texture");
            for delta in patches {
                assert_eq!(delta.options, egui::TextureOptions::LINEAR, "unsupported sampler");
                let egui::ImageData::Color(image) = &delta.image;
                assert!(image.width() > 0 && image.height() > 0);
                assert_eq!(image.pixels.len(), image.width().checked_mul(image.height()).unwrap());
                if let Some([x, y]) = delta.pos {
                    let dst = self.textures.get_mut(id).expect("patch before allocation");
                    assert!(x.checked_add(image.width()).unwrap() <= dst.width());
                    assert!(y.checked_add(image.height()).unwrap() <= dst.height());
                    let width = dst.width();
                    for row in 0..image.height() {
                        let at = (y + row) * width + x;
                        dst.pixels[at..at + image.width()].copy_from_slice(&image.pixels[row * image.width()..(row + 1) * image.width()]);
                    }
                } else {
                    self.textures.insert(*id, (**image).clone());
                }
            }
        }
        self.free_after_paint = out.textures_delta.free.iter().copied().collect();
    }

    fn png(&self, ctx: &egui::Context, mut out: egui::FullOutput) -> image::RgbaImage {
        assert!(!ctx.has_pending_images(), "unfinished production images");
        let ppp = out.pixels_per_point;
        assert!(ppp.is_finite() && ppp > 0.0);
        let viewport = ctx.content_rect();
        assert_eq!(viewport.min, egui::Pos2::ZERO);
        let size = ((viewport.width() * ppp).ceil() as usize, (viewport.height() * ppp).ceil() as usize);
        assert!(size.0 > 0 && size.1 > 0 && size.0 <= 4096 && size.1 <= 4096);
        let mut pixels = vec![[0.0f32; 4]; size.0.checked_mul(size.1).unwrap()];
        let primitives = ctx.tessellate(std::mem::take(&mut out.shapes), ppp);
        let mut thumbnails = 0;
        let cross = |a: (i64, i64), b: (i64, i64), c: (i64, i64)| {
            i128::from(b.0 - a.0) * i128::from(c.1 - a.1) - i128::from(b.1 - a.1) * i128::from(c.0 - a.0)
        };
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else { panic!("unsupported paint callback") };
            assert!(matches!(mesh.texture_id, egui::TextureId::Managed(_)), "unavailable user texture");
            let atlas = self.textures.get(&mesh.texture_id).expect("missing referenced texture");
            if mesh.texture_id != egui::TextureId::Managed(0) && atlas.width() >= 40 && atlas.height() >= 40 {
                thumbnails += 1;
            }
            assert_eq!(mesh.indices.len() % 3, 0, "incomplete triangle");
            for v in &mesh.vertices {
                assert!(v.pos.x.is_finite() && v.pos.y.is_finite() && v.uv.x.is_finite() && v.uv.y.is_finite());
                assert!((v.pos.x * ppp).abs() < 1_000_000.0 && (v.pos.y * ppp).abs() < 1_000_000.0);
            }
            assert!(
                !primitive.clip_rect.min.x.is_nan()
                    && !primitive.clip_rect.min.y.is_nan()
                    && !primitive.clip_rect.max.x.is_nan()
                    && !primitive.clip_rect.max.y.is_nan()
            );
            let clip = primitive.clip_rect.intersect(viewport);
            assert!(!clip.min.x.is_nan() && !clip.min.y.is_nan() && !clip.max.x.is_nan() && !clip.max.y.is_nan());
            for [i, j, k] in mesh.indices.as_chunks::<3>().0 {
                let mut v = [
                    *mesh.vertices.get(*i as usize).expect("triangle index"),
                    *mesh.vertices.get(*j as usize).expect("triangle index"),
                    *mesh.vertices.get(*k as usize).expect("triangle index"),
                ];
                let at = |v: &egui::epaint::Vertex| ((v.pos.x * ppp * 256.0).round() as i64, (v.pos.y * ppp * 256.0).round() as i64);
                let area = cross(at(&v[0]), at(&v[1]), at(&v[2]));
                if area == 0 {
                    continue;
                }
                if area < 0 {
                    v.swap(1, 2);
                }
                let area = area.abs() as f32;
                let [a, b, c] = [at(&v[0]), at(&v[1]), at(&v[2])];
                let range = |lo: i64, hi: i64, clip_lo: f32, clip_hi: f32, n: usize| {
                    let lo = (lo as f32 / 256.0).max(clip_lo * ppp).floor().max(0.0) as usize;
                    let hi = ((hi as f32 / 256.0).min(clip_hi * ppp).ceil().max(0.0) as usize).min(n);
                    lo..hi
                };
                let upper_left = |p: (i64, i64), q: (i64, i64)| q.1 < p.1 || (q.1 == p.1 && q.0 > p.0);
                for y in range(a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1), clip.min.y, clip.max.y, size.1) {
                    for x in range(a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0), clip.min.x, clip.max.x, size.0) {
                        if !clip.contains(egui::pos2((x as f32 + 0.5) / ppp, (y as f32 + 0.5) / ppp)) {
                            continue;
                        }
                        let q = (x as i64 * 256 + 128, y as i64 * 256 + 128);
                        let (w0, w1, w2) = (cross(b, c, q), cross(c, a, q), cross(a, b, q));
                        let inside = |w: i128, p, q| w > 0 || (w == 0 && upper_left(p, q));
                        if !(inside(w0, b, c) && inside(w1, c, a) && inside(w2, a, b)) {
                            continue;
                        }
                        let (w0, w1, w2) = (w0 as f32 / area, w1 as f32 / area, w2 as f32 / area);
                        let uv = (w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x, w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y);
                        let texel = screens_cpu_sample(atlas, uv);
                        let channels = [v[0].color.to_array(), v[1].color.to_array(), v[2].color.to_array()];
                        let src: [f32; 4] = std::array::from_fn(|i| {
                            (w0 * f32::from(channels[0][i]) + w1 * f32::from(channels[1][i]) + w2 * f32::from(channels[2][i])) * texel[i] / 255.0
                        });
                        let dst = &mut pixels[y * size.0 + x];
                        let keep = 1.0 - src[3] / 255.0;
                        for i in 0..4 {
                            dst[i] = src[i] + dst[i] * keep;
                        }
                    }
                }
            }
        }
        assert!(thumbnails > 0, "no production thumbnail was painted");
        let mut png = image::RgbaImage::new(size.0 as u32, size.1 as u32);
        for (dst, src) in png.pixels_mut().zip(pixels) {
            let c = src.map(|v| v.round().clamp(0.0, 255.0) as u8);
            *dst = image::Rgba(egui::Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3]).to_srgba_unmultiplied());
        }
        png
    }
}

fn screens_cpu_sample(atlas: &egui::ColorImage, uv: (f32, f32)) -> [f32; 4] {
    let (w, h) = (atlas.width(), atlas.height());
    let (x, y) = (uv.0.clamp(0.0, 1.0) * w as f32 - 0.5, uv.1.clamp(0.0, 1.0) * h as f32 - 0.5);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let texel =
        |dx: f32, dy: f32| atlas[(((x.floor() + dx).max(0.0) as usize).min(w - 1), ((y.floor() + dy).max(0.0) as usize).min(h - 1))].to_array();
    let (a, b, c, d) = (texel(0.0, 0.0), texel(1.0, 0.0), texel(0.0, 1.0), texel(1.0, 1.0));
    std::array::from_fn(|i| {
        let top = f32::from(a[i]) * (1.0 - fx) + f32::from(b[i]) * fx;
        let bottom = f32::from(c[i]) * (1.0 - fx) + f32::from(d[i]) * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

/// A headless CPU image of the actual production dialog, not an OS/GPU screenshot.
#[test]
fn filename_pattern_cpu_images_cover_artboards_and_assets() {
    let folder = temp_folder("filename-cpu-images");
    std::fs::create_dir_all(&folder).unwrap();
    for collected in [false, true] {
        let (mut app, _) = app(2, false);
        if collected {
            let ids: Vec<Value> = [2, 20]
                .into_iter()
                .map(|x| app.run("shape.rectangle", json!({"x": x, "y": 5, "width": 8, "height": 6})).unwrap()["id"].clone())
                .collect();
            app.run("select.set", json!({"ids": ids})).unwrap();
            app.run("file.exportSelection", json!({"multiple": true})).unwrap();
        } else {
            open(&mut app);
        }
        set(&mut app, "folder", json!(folder.to_string_lossy()));
        let mut h = ScreensFrame::new();
        crate::theme::apply(&h.ctx, crate::theme::Brightness::MediumDark);
        egui_extras::install_image_loaders(&h.ctx);
        h.cpu = Some(ScreensCpu::default());
        // The production dialog's embedded icon sources, never replacement art.
        for icon in ["layout-grid", "dc-list-view", "folder-open", "menu", "settings", "x"] {
            let poll = crate::icons::source(icon).load(&h.ctx, egui::TextureOptions::LINEAR, egui::load::SizeHint::Height(24)).unwrap();
            assert!(poll.is_ready(), "unfinished icon {icon}");
        }
        for (label, offset, value) in [("File Name:", 100.0, "batch-####"), ("Start at:", 65.0, "7")] {
            let out = h.settle(&mut app);
            let at = screen_text_at(&out, label) + egui::vec2(offset, 0.0);
            h.click(&mut app, at);
            let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            h.frame(
                &mut app,
                vec![key(egui::Key::A, egui::Modifiers::COMMAND), key(egui::Key::Backspace, egui::Modifiers::NONE), egui::Event::Text(value.into())],
            );
        }
        h.choose(&mut app, "Per Item", "Per File");
        assert_eq!((field(&app, "name"), field(&app, "counterStart"), field(&app, "counterPer")), (json!("batch-####"), json!("7"), json!("file")));
        let out = h.settle(&mut app);
        let text = crate::tests_labels::shapes_text(&out);
        for label in ["File Name:", "Start at:", "Count:", "Per File", "batch-####"] {
            assert!(text.contains(label), "missing {label}");
        }
        let png = h.cpu.as_ref().unwrap().png(&h.ctx, out);
        let path = folder.join(if collected { "assets.png" } else { "artboards.png" });
        png.save_with_format(&path, image::ImageFormat::Png).unwrap();
        println!("6720 CPU DIALOG PNG: {}", path.display());
    }
}
