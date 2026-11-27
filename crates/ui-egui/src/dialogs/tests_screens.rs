//! Export for Screens, driven through the dialog host headlessly: the fields it opens on (defaults,
//! then the document's last settings, and the artboards the Artboard tool has selected), the
//! artboard choice, Format Settings, desktop folders, the export on the background thread and web
//! downloads.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::PickKind;
use egui::accesskit::Role;
use serde_json::{Map, Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::export_for_screens;
use super::screen_formats::{self, PresetPick};
use super::tests_export::{fields, frame, kind, set, try_confirm};
use crate::tests_background::{Files, Gate, written};
use crate::{Services, VectorcraftApp, background};

pub(super) type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

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
    }
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": artboards})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, log)
}

/// The open dialog's fields.
fn values(app: &VectorcraftApp) -> Map<String, Value> {
    let Value::Object(fields) = fields(app) else { panic!("a dialog is open") };
    fields
}

/// The open dialog's value of `k`; null when it has none.
pub(super) fn field(app: &VectorcraftApp, k: &str) -> Value {
    values(app).get(k).cloned().unwrap_or(Value::Null)
}

fn open(app: &mut VectorcraftApp) {
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!(kind(app).as_deref(), Some(export_for_screens::KIND));
}

#[test]
fn the_first_artboard_is_clickable_on_open_in_a_short_web_viewport() {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

    for view in ["grid", "list"] {
        let (mut app, log) = app(2, true);
        app.run("artboard.rename", json!({"indices": [0], "name": "First audit board"})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(640.0, 500.0));
        let mut time = 0.0;
        let mut draw = |app: &mut VectorcraftApp, events| {
            time += 0.2;
            crate::testing::app_frame(app, &ctx, RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() })
        };
        draw(&mut app, vec![]);
        open(&mut app);
        set(&mut app, "view", json!(view));
        for _ in 0..5 {
            draw(&mut app, vec![]);
        }
        let out = draw(&mut app, vec![]);
        let nodes = out.platform_output.accesskit_update.unwrap().nodes;
        let bounds = nodes
            .iter()
            .find_map(|(_, node)| {
                if node.role() == Role::CheckBox && node.label().is_some_and(|label| label.contains("First audit board")) {
                    node.bounds()
                } else {
                    None
                }
            })
            .expect("the first artboard has a checkbox");
        let pos = pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);

        assert_eq!(field(&app, "boards"), json!([true, true]));
        draw(&mut app, vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            draw(&mut app, vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
        }
        draw(&mut app, vec![]);
        assert_eq!(field(&app, "boards"), json!([false, true]), "the first {view} checkbox should be clickable without scrolling");
        assert!(log.borrow().is_empty());
    }
}

/// Presses OK, which the dialog refuses or does not offer: it stays open.
fn stays_open(app: &mut VectorcraftApp) {
    let _ = try_confirm(app);
    assert_eq!(kind(app).as_deref(), Some(export_for_screens::KIND));
}

#[test]
fn a_queued_export_requires_fresh_confirmation_after_switching_documents() {
    use craft::ui::dialog::{HeldOk, dialog_id};

    let (mut app, log) = app(1, true);
    let source = app.session.active().unwrap().uid;
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("artboard.rename", json!({"indices": [0], "name": "Destination"})).unwrap();
    let destination = app.session.active().unwrap().uid;
    app.run("window.previousDocument", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, source);
    open(&mut app);

    let ctx = app.ui_ctx.clone().unwrap();
    let dialogs = app.dialogs().unwrap();
    let id = dialog_id(export_for_screens::KIND);
    dialogs.with(|host| {
        assert!(!host.confirm_by_id(&ctx, id));
        assert_eq!(host.held_state(id), Some(HeldOk::Waiting));
    });
    app.run("window.nextDocument", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, destination);
    export_for_screens::pump(&mut app, &ctx);

    assert!(log.borrow().is_empty(), "the earlier OK cannot download from the destination tab");
    assert!(matches!(dialogs.with(|host| host.held_state(id)), Some(HeldOk::Refused(_))));
    try_confirm(&mut app).unwrap();
    assert_eq!(log.borrow().iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["Destination.png"]);
}

fn temp_folder(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("vc-efs-{tag}-{}", std::process::id()))
}

/// The files under `dir`, as paths below it with `/`, sorted.
pub(super) fn files_under(dir: &std::path::Path) -> Vec<String> {
    fn walk(at: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(at).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The names of the entries of ZIP archive `zip`, in order: each local file header holds its name
/// 30 bytes in, its compressed size at byte 18 and the lengths of the name and the extra field at
/// bytes 26 and 28.
pub(super) fn zip_names(zip: &[u8]) -> Vec<String> {
    const HEADER: [u8; 4] = [b'P', b'K', 3, 4];
    let mut names = vec![];
    let mut at = 0;
    while zip.get(at..at + 4) == Some(&HEADER[..]) {
        let short = |i: usize| usize::from(u16::from_le_bytes([zip[at + i], zip[at + i + 1]]));
        let size = u32::from_le_bytes([zip[at + 18], zip[at + 19], zip[at + 20], zip[at + 21]]) as usize;
        let (name, extra) = (short(26), short(28));
        names.push(String::from_utf8_lossy(&zip[at + 30..at + 30 + name]).into_owned());
        at += 30 + name + extra + size;
    }
    names
}

/// The saved or built-in format preset `name`.
fn preset(app: &mut VectorcraftApp, name: &str) -> Value {
    export_for_screens::export_presets(app).as_array().unwrap().iter().find(|p| p["name"] == name).cloned().unwrap()
}

/// The dialog's own request, with a row of every format its Format menu offers and every format's
/// settings as it opens, gets no note about an option a file doesn't read.
#[test]
fn the_dialogs_own_rows_and_settings_get_no_unused_option_notes() {
    let (mut app, _) = app(1, true);
    open(&mut app);
    let rows: Vec<Value> = [
        json!({"format": "png"}),
        json!({"format": "png8"}),
        json!({"format": "jpg", "quality": 100}),
        json!({"format": "webp"}),
        json!({"format": "gif"}),
        json!({"format": "tiff"}),
        json!({"format": "svg"}),
        json!({"format": "pdf"}),
        json!({"format": "pdf", "marks": {"trim": true}, "suffix": "-print"}),
        json!({"format": "eps"}),
        json!({"format": "ai"}),
        json!({"format": "vectorcraft"}),
    ]
    .into_iter()
    .map(|mut r| {
        r["scale"] = json!("1x");
        r
    })
    .collect();
    set(&mut app, "formats", json!(rows));
    let p = export_for_screens::params(&app, &values(&app)).unwrap();
    let r = app.session.execute("document.exportForScreens", &p).unwrap();
    let unused: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("was not used")).collect();
    assert!(unused.is_empty(), "{unused:?}");
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

/// The rect the frame `shapes` paint `texture` over; `None` when none paints it.
fn painted(shapes: &[egui::epaint::ClippedShape], texture: egui::TextureId) -> Option<egui::Rect> {
    fn walk(shape: &egui::Shape, texture: egui::TextureId, out: &mut Option<egui::Rect>) {
        match shape {
            egui::Shape::Vec(all) => all.iter().for_each(|s| walk(s, texture, out)),
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                let at = mesh.calc_bounds();
                *out = Some(out.map_or(at, |r| r.union(at)));
            }
            _ => {}
        }
    }
    let mut out = None;
    shapes.iter().for_each(|clipped| walk(&clipped.shape, texture, &mut out));
    out
}

/// The grid's tiles and the list's thumbnails are square; an artboard of another shape keeps its
/// proportions in them.
#[test]
fn an_artboards_picture_keeps_its_proportions() {
    let (mut app, _) = app(1, false);
    open(&mut app);
    for view in ["grid", "list"] {
        set(&mut app, "view", json!(view));
        frame(&mut app);
        let ctx = app.ui_ctx.clone().expect("a test app has a window");
        let out = crate::testing::part_frame(&mut app, &ctx, Default::default(), |app, ui| app.ui(ui));
        let tex = app.canvas.artboard_thumbs.as_ref().and_then(|(_, thumbs)| thumbs.first().cloned().flatten()).expect("the artboard has a picture");
        let drawn = painted(&out.shapes, tex.id()).expect("the artboard's picture is drawn");
        let (picture, shown) = (tex.size_vec2(), drawn.size());
        assert!(picture.x > picture.y, "the 60×40 pt artboard is wider than high");
        assert!((shown.x / shown.y - picture.x / picture.y).abs() < 0.02, "{view}: a {picture:?} picture is drawn {shown:?}");
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
    try_confirm(&mut app).unwrap();
    // Create Sub-folders is on by default, a folder per scale.
    {
        let log = log.borrow();
        let [(_, zip)] = log.as_slice() else { panic!("one zip") };
        assert_eq!(zip_names(zip), ["1x/Artboard-2.png", "1x/Artboard-3.png"]);
    }
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
    let ctx = crate::testing::ui_context();
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
        // The frame after the click, as the hosts draw it: the list reports a Shift-click's range there.
        panel(&mut app, time + 0.1, vec![]);
    }
    assert_eq!(crate::panels::artboards::picked(&app, 4), [1, 2], "the panel highlights rows 2 and 3");
    open(&mut app);
    let picked = values(&app);
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
    assert_eq!(others(&picked), others(&values(&app)));
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
    stays_open(&mut app);
    set(&mut app, "range", json!("2"));
    set(&mut app, "folder", json!(" "));
    assert!(try_confirm(&mut app).is_err());
    set(&mut app, "folder", json!(folder));
    try_confirm(&mut app).unwrap();
    assert_eq!(kind(&app), None);
    assert_eq!(files_under(&dir), ["120w/Artboard-2@120w.png", "1x/Artboard-2.png", "SVG/Artboard-2.svg"]);
    let png = craft_image_codecs::decode(&std::fs::read(dir.join("120w").join("Artboard-2@120w.png")).unwrap())
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap();
    assert_eq!((png.width(), png.height()), (120, 80));
    assert_eq!(
        log.borrow().iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        [format!("{folder}/1x/Artboard-2.png")],
        "Open Location shows the first file"
    );
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
        // The zip through the command, the files through the dialog's OK, whose files are the ones in the folder.
        let export = |app: &mut VectorcraftApp| -> Option<Value> {
            if zip {
                let p = json!({"folder": folder, "zip": true, "folders": "rows", "name": "<name>", "formats": rows, "openLocation": true});
                return Some(app.run("file.exportForScreens", p).unwrap());
            }
            open(app);
            set(app, "folder", json!(folder));
            set(app, "subfolders", json!(true));
            set(app, "formats", rows.clone());
            try_confirm(app).unwrap();
            assert_eq!(kind(app), None, "closed");
            None
        };
        let in_folder = || -> Vec<(String, Vec<u8>)> {
            files_under(&dir).into_iter().map(|f| (format!("{folder}/{f}"), std::fs::read(dir.join(&f)).unwrap())).collect()
        };
        background_export(&mut app, false);
        let want = export(&mut app).map_or_else(in_folder, |r| on_disk(&r));
        if !zip {
            assert_eq!(want.len(), 6);
            assert!(want.iter().any(|(f, _)| *f == format!("{folder}/1x/icon-2.png")), "a name taken gets -2");
        }
        let (status, shown) = (app.ui.status.clone(), log.borrow().clone());
        assert_eq!(shown.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
        log.borrow_mut().clear();
        background_export(&mut app, true);
        if let Some(r) = export(&mut app) {
            assert_eq!(r["background"], true, "{r}");
        }
        assert_eq!(app.ui.status, format!("Exporting {}…", dir.file_name().unwrap().to_string_lossy()));
        assert!(written(&files).is_empty() && log.borrow().is_empty(), "nothing written yet");
        gate.open();
        background::wait_all(&mut app);
        let mut got = files.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, want, "zip: {zip}");
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

/// An export into the folder its export preset keeps, with no folder of its own, is written in the
/// background as one into a folder it names, and the status bar names that folder.
#[test]
fn an_export_into_a_presets_folder_writes_in_the_background() {
    let gate = Gate::default();
    let (mut app, _, files) = app_with_worker(1, &gate);
    let dir = temp_folder("preset-folder");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let settings = json!({"formats": [{"format": "svg"}], "folder": folder});
    app.run("document.exportPresets.save", json!({"name": "Kept", "settings": settings})).unwrap();
    let p = json!({"exportPreset": "Kept"});
    background_export(&mut app, false);
    let want = on_disk(&app.run("file.exportForScreens", p.clone()).unwrap());
    assert_eq!(want.len(), 1);
    let status = app.ui.status.clone();
    assert_eq!(status, format!("Exported 1 file to {folder}"));
    std::fs::remove_dir_all(&dir).unwrap();
    background_export(&mut app, true);
    assert_eq!(app.run("file.exportForScreens", p).unwrap()["background"], true);
    assert!(written(&files).is_empty(), "nothing written yet");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!((files.lock().unwrap().clone(), app.ui.status.clone()), (want, status));
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
    let e = try_confirm(&mut app).unwrap_err();
    assert!(e.contains("Export for Screens writes png"), "{e}");
    assert_eq!((kind(&app).as_deref(), app.ui.status.as_str()), (Some(export_for_screens::KIND), e.as_str()));
    assert!(app.background.jobs.is_empty() && written(&files).is_empty());
}

#[test]
fn the_web_downloads_one_zip_or_the_file() {
    let (mut app, log) = app(2, true);
    open(&mut app);
    frame(&mut app);
    set(
        &mut app,
        "formats",
        json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "jpg", "quality": 80, "scale": "2x", "suffix": "@2x"}]),
    );
    set(&mut app, "settings", json!({"png": {"background": "black"}}));
    try_confirm(&mut app).unwrap();
    {
        // Four files: one zip.
        let log = log.borrow();
        let [(name, zip)] = log.as_slice() else { panic!("one download: {:?}", log.iter().map(|(n, _)| n).collect::<Vec<_>>()) };
        assert_eq!(name, "Untitled-1.zip");
        assert_eq!(zip_names(zip), ["1x/Artboard-1.png", "2x/Artboard-1@2x.jpg", "1x/Artboard-2.png", "2x/Artboard-2@2x.jpg"]);
    }
    // One file downloads as itself, with the format's settings.
    log.borrow_mut().clear();
    open(&mut app);
    assert_eq!(field(&app, "settings")["png"]["background"], json!("black"), "remembered");
    set(&mut app, "select", json!("range"));
    set(&mut app, "range", json!("1"));
    set(&mut app, "formats", json!([{"format": "png", "scale": "1x", "suffix": ""}]));
    try_confirm(&mut app).unwrap();
    let log = log.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "Artboard-1.png");
    assert_eq!(
        craft_image_codecs::decode(&log[0].1)
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .unwrap()
            .to_rgba8()
            .get_pixel(1, 1)
            .0,
        [0, 0, 0, 255],
        "the PNG settings' background"
    );
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
        try_confirm(&mut app).unwrap();
        let px = craft_image_codecs::decode(&log.borrow()[0].1)
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .unwrap()
            .to_rgba8()
            .get_pixel(58, 38)
            .0;
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

/// Frames of the app's own window (`app.ui_ctx`, where its dialogs are) with the accessibility
/// tree, input events and a clock; each frame runs the dialog layer's pump after it. The window is
/// 1280 by 900 unless `sized` gives another size.
#[derive(Default)]
pub(super) struct Window {
    time: f64,
    size: Option<egui::Vec2>,
}

impl Window {
    /// A window of `size` instead of the default 1280 by 900.
    fn sized(size: egui::Vec2) -> Self {
        Self { size: Some(size), ..Self::default() }
    }

    fn run(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        let ctx = app.ui_ctx.clone().expect("a test app has a window");
        ctx.enable_accesskit();
        self.time += 0.1;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size.unwrap_or(egui::vec2(1280.0, 900.0)));
        let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
        // The window's frame with the app's frame input, as the hosts run it: the controls' commands reach the app.
        let out = crate::testing::part_frame(app, &ctx, input, |app, ui| app.ui(ui));
        super::host::pump(app, &ctx);
        out
    }

    /// The names the accessibility tree gives what the window shows, after the frame that lays the
    /// dialogs out.
    pub(super) fn names(&mut self, app: &mut VectorcraftApp) -> Vec<String> {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        nodes.iter().filter_map(|(_, n)| n.label().map(str::to_string)).collect()
    }

    /// The center of the control of `role` named `label`.
    fn at(&mut self, app: &mut VectorcraftApp, label: &str, role: Role) -> egui::Pos2 {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let (_, n) =
            nodes.iter().find(|(_, n)| n.label() == Some(label) && n.role() == role).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
        let b = n.bounds().expect("bounds");
        egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5)
    }

    /// Clicks the control of `role` named `label`.
    fn press(&mut self, app: &mut VectorcraftApp, label: &str, role: Role) {
        let at = self.at(app, label, role);
        for pressed in [true, false] {
            let button = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            self.run(app, vec![egui::Event::PointerMoved(at), button]);
        }
        self.run(app, vec![]);
    }

    /// Presses and releases `key`.
    fn key(&mut self, app: &mut VectorcraftApp, key: egui::Key) {
        for pressed in [true, false] {
            self.run(app, vec![egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }]);
        }
        self.run(app, vec![]);
    }

    /// Types `text` where the focus is.
    fn type_text(&mut self, app: &mut VectorcraftApp, text: &str) {
        self.run(app, vec![egui::Event::Text(text.into())]);
    }
}

/// The gear opens Format Settings on the format of the saved first row, whatever its spelling, and
/// edits there keep the saved rows and reach the file.
fn saved_tiff_alias_gear(alias: &str, asset_panel: bool) {
    let (mut app, log) = app(1, true);
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
    let asset = asset_panel.then(|| app.run("assets.add", json!({"name": "Badge"})).unwrap()["assets"][0].as_u64().unwrap());
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
    let mut w = Window::default();
    match asset {
        // The Asset Export panel's gear opens the dialog on its Assets tab with the rows' Format Settings.
        Some(id) => {
            export_for_screens::open_assets(&mut app, Some(&[id]));
            set(&mut app, "__settings", json!(screen_formats::settings_format(rows.as_array().unwrap())));
        }
        None => {
            open(&mut app);
            w.press(&mut app, "Format Settings", Role::Button);
        }
    }
    assert_eq!(field(&app, "__settings"), "tiff", "gear for {alias}, asset panel={asset_panel}");
    assert_eq!(field(&app, "tab"), if asset_panel { "assets" } else { "artboards" });
    assert_eq!(field(&app, "formats"), rows, "opening the gear preserves the saved spelling and row options");
    assert_eq!(app.session.active().unwrap().doc.export_settings, saved_before, "opening settings does not save them");
    let mut expected = field(&app, "settings");
    assert_eq!(expected["tiff"]["colorModel"], "rgb", "saved RGB overrides the CMYK document default");
    assert_eq!(expected["tiff"]["lzw"], false);
    assert_eq!(expected["tiff"]["embedIcc"], false);
    assert_eq!(expected["tiff"]["antiAlias"], "type");
    assert_eq!(expected["png"]["interlaced"], true);
    expected["tiff"]["byteOrder"] = json!("big");
    set(&mut app, "settings", expected.clone());
    set(&mut app, "__settings", json!(""));
    assert_eq!(field(&app, "formats"), rows);
    try_confirm(&mut app).unwrap();
    assert_eq!(kind(&app), None, "{}", app.ui.status);
    {
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF download for {alias}") };
        assert_eq!(name, if asset_panel { "Badge@2x.tif" } else { "Artboard-1@2x.tif" });
        let tags = big_tiff_tags(bytes); // The edited canonical settings reach the aliased row.
        assert!(tags.contains(&(259, 3, 1, 1)), "saved uncompressed option survives");
        assert!(tags.contains(&(262, 3, 1, 2)), "saved RGB survives");
        assert!(!tags.iter().any(|t| t.0 == 34675), "saved profile-off option survives");
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
    let mut w = Window::default();
    w.names(&mut app);
    open(&mut app);
    set(&mut app, "formats", json!([{"format": "png", "scale": "2x", "suffix": "@2x"}]));
    set(&mut app, "prefix", json!("proof-"));
    set(&mut app, "subfolders", json!(false));
    // TIFF is chosen from the row's format list.
    w.press(&mut app, "Format", Role::ComboBox);
    w.press(&mut app, "TIFF", Role::ListBoxOption);
    assert_eq!(field(&app, "formats"), json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    // Only the first of two boards stays checked (a tile's check box is named for what it selects).
    w.press(&mut app, "Select 2  Artboard 2", Role::CheckBox);
    assert_eq!((field(&app, "boards"), field(&app, "range")), (json!([true, false]), json!("1")));
    set(&mut app, "__settings", json!("tiff"));
    let mut settings = field(&app, "settings");
    assert_eq!((settings["tiff"]["byteOrder"].clone(), settings["tiff"]["lzw"].clone()), (json!("little"), json!(true)));
    settings["tiff"]["byteOrder"] = json!("big");
    settings["tiff"]["lzw"] = json!(false);
    settings["tiff"]["embedIcc"] = json!(false);
    set(&mut app, "settings", settings.clone());
    set(&mut app, "__settings", json!(""));
    w.press(&mut app, "Download", Role::Button);
    assert_eq!(kind(&app), None, "{}", app.ui.status);
    {
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF download") };
        assert_eq!(name, "proof-Artboard-1@2x.tif");
        let tags = big_tiff_tags(bytes);
        assert!(tags.contains(&(259, 3, 1, 1)), "uncompressed as selected");
        assert!(!tags.iter().any(|t| t.0 == 34675), "ICC profile disabled");
    }
    open(&mut app);
    assert_eq!(field(&app, "formats"), json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    assert_eq!((field(&app, "boards"), field(&app, "prefix")), (json!([true, false]), json!("proof-")));
    assert_eq!(field(&app, "settings")["tiff"], settings["tiff"]);
}

/// iOS and Android, and a saved preset made of the mobile rows, set the rows those platforms use, in
/// sub-folders by scale.
#[test]
fn a_mobile_button_or_preset_sets_the_rows() {
    let mobile = json!(fileio::screen_preset_rows("mobile").unwrap());
    let density = json!(fileio::screen_preset_rows("density").unwrap());
    for (how, preset, rows) in [("iOS", "mobile", &mobile), ("Android", "density", &density), ("Phones", "mobile", &mobile)] {
        let (mut app, _) = app(1, false);
        app.run("document.exportPresets.save", json!({"name": "Phones", "settings": {"preset": "mobile", "subfolders": true}})).unwrap();
        let mut w = Window::default();
        w.names(&mut app);
        open(&mut app);
        set(&mut app, "subfolders", json!(false));
        set(&mut app, "subfolderBy", json!("format"));
        if how == "Phones" {
            set(&mut app, "exportPreset", json!("Phones"));
        } else {
            w.press(&mut app, how, Role::Button);
        }
        assert_eq!(&field(&app, "formats"), rows, "{how}");
        assert_eq!(
            (field(&app, "preset"), field(&app, "subfolders"), field(&app, "subfolderBy")),
            (json!(preset), json!(true), json!("scale")),
            "{how}"
        );
    }
}

/// Clear Selection unchecks every artboard, so nothing is exported until one is checked again.
#[test]
fn clear_selection_leaves_nothing_to_export() {
    let (mut app, _) = app(2, false);
    let mut w = Window::default();
    w.names(&mut app);
    open(&mut app);
    w.press(&mut app, "Clear Selection", Role::Button);
    assert_eq!((field(&app, "boards"), field(&app, "select")), (json!([false, false]), json!("range")));
    set(&mut app, "folder", json!(temp_folder("cleared").to_string_lossy()));
    stays_open(&mut app);
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
        let _ = std::fs::remove_dir_all(&dir);
        open(app);
        let base = [("folder", json!(folder)), ("select", json!("range")), ("range", json!("1-2")), ("formats", rows.clone())];
        for (k, v) in base.into_iter().chain(extra.iter().cloned()) {
            set(app, k, v);
        }
        frame(app);
        try_confirm(app).unwrap();
        files_under(&dir)
    };
    let by_scale = files(&mut app, &[("subfolders", json!(true)), ("subfolderBy", json!("scale"))]);
    assert!(by_scale.contains(&"1x/Artboard-1.png".to_string()) && by_scale.contains(&"2x/Artboard-1@2x.png".to_string()), "{by_scale:?}");
    let by_format = files(&mut app, &[("subfolders", json!(true)), ("subfolderBy", json!("format")), ("pdfs", json!("single"))]);
    let pdf = format!("pdf/{title}.pdf");
    assert_eq!(by_format, [pdf.as_str(), "png/Artboard-1.png", "png/Artboard-1@2x.png", "png/Artboard-2.png", "png/Artboard-2@2x.png"]);
    let flat = files(&mut app, &[("subfolders", json!(false)), ("pdfs", json!("multiple"))]);
    assert_eq!(flat.len(), 6);
    assert!(flat.contains(&"Artboard-2.pdf".to_string()), "{flat:?}");
    // The status line counts the files Single file writes.
    open(&mut app);
    for (k, v) in [("select", json!("range")), ("range", json!("1-2")), ("formats", rows.clone()), ("pdfs", json!("single"))] {
        set(&mut app, k, v);
    }
    let names = Window::default().names(&mut app);
    assert!(names.iter().any(|n| n.contains("Selected: 2, Total number of files to export: 5")), "{names:?}");
    // A format preset sets the rows, their settings and the sub-folders, not a prefix it lacks.
    let mut f = values(&app);
    f.insert("prefix".into(), json!("x-"));
    screen_formats::apply_preset(&mut f, &preset(&mut app, "Logo Package"));
    assert_eq!(f["formats"].as_array().unwrap().len(), 5);
    assert_eq!(
        (&f["subfolders"], &f["subfolderBy"], &f["exportPreset"], &f["prefix"]),
        (&json!(true), &json!("format"), &json!("Logo Package"), &json!("x-"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save Preset… on an applied preset saves the dialog's rows, settings, prefix and sub-folders and
/// keeps the preset's keys the dialog does not show (a file name pattern, other export settings).
#[test]
fn saving_an_applied_preset_keeps_the_keys_the_dialog_does_not_show() {
    let (mut app, _) = app(2, false);
    app.run(
        "document.exportPresets.save",
        json!({
            "name": "Repeat",
            "settings": {"formats": [{"format": "png"}], "name": "<name>-###", "counterPer": "file", "antiAlias": "none", "prefix": "a-"}
        }),
    )
    .unwrap();
    open(&mut app);
    set(&mut app, "exportPreset", json!("Repeat"));
    set(&mut app, "prefix", json!("b-"));
    set(&mut app, "subfolders", json!(true));
    set(&mut app, "subfolderBy", json!("format"));
    let mut f = values(&app);
    screen_formats::preset_pick(&mut app, &mut f, PresetPick::Save("Repeat".into()));
    let saved = app.session.prefs.screen_export_presets.iter().find(|p| p.name == "Repeat").unwrap_or_else(|| panic!("{}", app.ui.status));
    let s = &saved.settings;
    assert_eq!((s.get("counterPer"), s.get("antiAlias"), s.get("name")), (Some(&json!("file")), Some(&json!("none")), Some(&json!("<name>-###"))));
    assert_eq!((s.get("prefix"), s.get("folders")), (Some(&json!("b-")), Some(&json!("format"))));
    assert_eq!(f["exportPreset"], "Repeat");
}

/// Save Preset… on an applied preset saves the dialog's Include Bleed and Include Artboard Fill.
#[test]
fn saving_an_applied_preset_keeps_its_bleed_and_fill() {
    let (mut app, _) = app(2, false);
    let settings = json!({"formats": [{"format": "png"}], "includeBleed": true, "includeArtboardFill": false});
    app.run("document.exportPresets.save", json!({"name": "Bleed", "settings": settings})).unwrap();
    open(&mut app);
    set(&mut app, "exportPreset", json!("Bleed"));
    let mut f = values(&app);
    screen_formats::preset_pick(&mut app, &mut f, PresetPick::Save("Bleed".into()));
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
    let check = |app: &VectorcraftApp, fields: &Map<String, Value>| {
        let p = export_for_screens::params(app, fields).unwrap();
        app.session.check_params("document.exportForScreens", &p).unwrap_or_else(|e| panic!("{e}: {p}"));
        p
    };
    open(&mut app);
    set(&mut app, "folder", json!(folder.to_string_lossy()));
    let mut f = values(&app);
    let p = check(&app, &f);
    assert_eq!(p["formats"][0]["scale"], json!("2x"));
    assert_eq!(p["formats"][1]["marks"]["trim"], json!(true));
    screen_formats::apply_preset(&mut f, &json!({"name": "Old", "settings": stale}));
    check(&app, &f);
    let _ = std::fs::remove_dir_all(&folder);
}

/// A preset that puts each item in its own folder (`item` or `itemFormat`, which the dialog has no
/// control for) keeps that folder mode when it is exported or saved again from the dialog, until
/// the sub-folder controls change (in the dialog or by `ui.dialog.set`).
#[test]
fn an_applied_preset_keeps_its_item_folders() {
    use craft::automation::{ControlRequest, Reply};
    for mode in ["item", "itemFormat"] {
        let (mut app, _) = app(2, false);
        app.run("document.exportPresets.save", json!({"name": "Items", "settings": {"formats": [{"format": "png"}], "folders": mode}})).unwrap();
        open(&mut app);
        let mut f = values(&app);
        screen_formats::apply_preset(&mut f, &preset(&mut app, "Items"));
        f.insert("folder".into(), json!("out"));
        let p = export_for_screens::params(&app, &f).unwrap();
        assert_eq!(p["folders"], json!(mode), "exported");
        screen_formats::preset_pick(&mut app, &mut f, PresetPick::Save("Items".into()));
        let saved = app.session.prefs.screen_export_presets.iter().find(|p| p.name == "Items").unwrap_or_else(|| panic!("{}", app.ui.status));
        assert_eq!(saved.settings.get("folders"), Some(&json!(mode)), "saved");

        // `ui.dialog.set` of the sub-folder controls, as a change in the dialog, ends the item folders:
        // the files go where the controls say, not into a folder per item.
        for (on, first) in [(false, "Artboard-1.png"), (true, "1x/Artboard-1.png")] {
            let dir = temp_folder(&format!("items-{mode}-{on}"));
            let _ = std::fs::remove_dir_all(&dir);
            open(&mut app);
            set(&mut app, "exportPreset", json!("Items"));
            set(&mut app, "folder", json!(dir.to_string_lossy()));
            let (req, _rx) = ControlRequest::new("ui.dialog.set", json!({"field": "subfolders", "value": on}));
            let Reply::Done(r) = crate::control::handle(&mut app, &crate::testing::ui_context(), &req) else { panic!("not done") };
            assert_eq!(r["ok"], true, "{r}");
            try_confirm(&mut app).unwrap();
            assert_eq!(files_under(&dir).first().map(String::as_str), Some(first), "subfolders {on}");
            let _ = std::fs::remove_dir_all(&dir);
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
    let mut f = values(&app);
    let apply = |f: &mut Map<String, Value>, settings: Value| screen_formats::apply_preset(f, &json!({"name": "Empty", "settings": settings}));
    for settings in [json!({"folders": ""}), json!({"folders": "", "subfolders": true})] {
        f.insert("subfolders".into(), json!(true));
        apply(&mut f, settings.clone());
        assert_eq!(f["subfolders"], json!(false), "{settings}");
    }
    apply(&mut f, json!({"subfolders": true}));
    assert_eq!(f["subfolders"], json!(true));
    f.insert("subfolderBy".into(), json!("format"));
    apply(&mut f, json!({}));
    assert_eq!((&f["subfolders"], &f["subfolderBy"]), (&json!(true), &json!("format")));
}

/// The Asset Export panel menu's Export Preset… writes a preset to a file and its Import Preset… adds
/// the presets of a file, which the open dialog then offers, as it does after opening the file (the
/// web's Import).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_preset_and_import_preset_go_through_files() {
    let dir = temp_folder("preset-files");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pack.vcexportpresets").to_string_lossy().to_string();
    let services = Services { write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    let picker = crate::tests_picks::scripted(&mut app);
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let ctx = app.ui_ctx.clone().expect("a window");
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("document.exportPresets.save", json!({"name": "Pack", "settings": {"formats": [{"format": "svg"}], "folders": "format"}})).unwrap();
    open(&mut app);
    let names = |app: &mut VectorcraftApp| -> Vec<String> {
        export_for_screens::export_presets(app).as_array().unwrap().iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect()
    };
    let exported = Rc::new(RefCell::new(None));
    let to = exported.clone();
    screen_formats::export_preset(&mut app, "Pack", move |_, r| *to.borrow_mut() = Some(r));
    // The save panel suggests the preset's name, not the document's.
    assert_eq!(picker.requests().last().and_then(|p| p.file_name.clone()).as_deref(), Some("Pack.vcexportpresets"));
    picker.answer_saved(&file, None);
    crate::tests_picks::poll(&mut app, &ctx);
    assert!(std::fs::metadata(&file).is_ok(), "{:?}", exported.borrow());
    app.run("document.exportPresets.delete", json!({"name": "Pack"})).unwrap();
    assert_eq!(names(&mut app), ["Logo Package", "Print-Ready PDF"]);
    let imported = Rc::new(RefCell::new(None));
    let to = imported.clone();
    screen_formats::import_presets(&mut app, move |_, r| *to.borrow_mut() = Some(r));
    picker.answer(&[&file]);
    crate::tests_picks::poll(&mut app, &ctx);
    assert_eq!(imported.borrow_mut().take(), Some(Ok(vec!["Pack".to_string()])));
    assert_eq!(app.ui.status, "Imported export presets: Pack");
    frame(&mut app);
    assert_eq!(names(&mut app), ["Logo Package", "Print-Ready PDF", "Pack"]);
    set(&mut app, "exportPreset", json!("Pack"));
    assert_eq!((field(&app, "formats")[0]["format"].clone(), field(&app, "subfolderBy")), (json!("svg"), json!("format")));
    // The file opened again: a second copy, numbered, which the open dialog offers.
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    frame(&mut app);
    assert_eq!(kind(&app).as_deref(), Some(export_for_screens::KIND));
    assert_eq!(names(&mut app), ["Logo Package", "Print-Ready PDF", "Pack", "Pack 2"]);
    set(&mut app, "exportPreset", json!("Pack 2"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Export Preset… suggests the preset's name as a file name, as Export Library… does a library's.
#[test]
fn an_exported_preset_file_is_named_after_the_preset() {
    let name = screen_formats::preset_file_name;
    assert_eq!(name("Logo Package 2"), "Logo Package 2.vcexportpresets");
    assert_eq!(name("Print-Ready PDF"), "Print-Ready PDF.vcexportpresets");
    assert_eq!(name("Web/App: v1.2"), "Web-App- v1-2.vcexportpresets");
    assert_eq!(name("CON"), "CON-.vcexportpresets");
}

/// `ui.dialog.set` of `exportPreset` applies the preset it names, as a pick in the ☰ menu does, so
/// the dialog holds the settings of the preset it names; a name that is no preset's is an error and
/// leaves the dialog as it was.
#[test]
fn setting_the_export_preset_field_applies_the_preset() {
    use craft::automation::{ControlRequest, Reply};
    let (mut app, _) = app(1, false);
    let ctx = crate::testing::ui_context();
    open(&mut app);
    let set_preset = |app: &mut VectorcraftApp, name: &str| {
        let (req, _rx) = ControlRequest::new("ui.dialog.set", json!({"field": "exportPreset", "value": name}));
        let Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
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
    let Reply::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!((field(&app, "exportPreset"), field(&app, "subfolderBy")), (json!("Logo Package"), json!("format")));
    // In one request the fields apply in order: a field after the preset goes over it.
    let fields = json!({"fields": [{"field": "exportPreset", "value": "Logo Package"}, {"field": "subfolderBy", "value": "scale"}]});
    let (req, _rx) = ControlRequest::new("ui.dialog.set", fields);
    let Reply::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!((field(&app, "exportPreset"), field(&app, "subfolderBy")), (json!("Logo Package"), json!("scale")));
}

/// The Asset Export panel menu's Import Preset… and Export Preset… show their file dialogs off the UI
/// thread (as sheets on macOS) and read or write the file when the dialog answers.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn preset_files_ask_off_the_ui_thread() {
    use crate::panels::asset_export;
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let picker = crate::tests_picks::scripted_sheets(&mut app, true);
    let ctx = app.ui_ctx.clone().expect("a window");
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("document.exportPresets.save", json!({"name": "Pack", "settings": {"formats": [{"format": "svg"}]}})).unwrap();
    let dir = temp_folder("preset-sheets");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pack.vcexportpresets");
    std::fs::write(&file, app.run("document.exportPresets.export", json!({"names": ["Pack"]})).unwrap()["data"].as_str().unwrap()).unwrap();
    let file = file.to_string_lossy().to_string();
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let names = |app: &VectorcraftApp| app.session.prefs.screen_export_presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    let doc = app.session.active().unwrap().uid;
    asset_export::preset(&mut app, doc, PresetPick::Export("Pack".into()));
    let asked = picker.requests();
    assert!(matches!(asked.as_slice(), [p] if p.kind == PickKind::Save && p.file_name.as_deref() == Some("Pack.vcexportpresets")), "{asked:?}");
    assert!(picker.written().is_empty() && app.ui.status.is_empty(), "{}", app.ui.status);
    picker.answer_saved("/presets/pack.vcexportpresets", None);
    crate::tests_picks::poll(&mut app, &ctx);
    assert_eq!(picker.written(), ["/presets/pack.vcexportpresets"], "{}", app.ui.status);
    asset_export::preset(&mut app, doc, PresetPick::Import);
    assert!(picker.requests().last().is_some_and(|p| p.kind == PickKind::Open), "{:?}", picker.requests());
    assert_eq!(names(&app), ["Pack"]);
    picker.answer(&[&file]);
    crate::tests_picks::poll(&mut app, &ctx);
    assert_eq!(names(&app), ["Pack", "Pack 2"], "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Starts renaming the artboard whose tile is named `tile` (F2 on it) and types `text`.
fn type_artboard_name(w: &mut Window, app: &mut VectorcraftApp, tile: &str, text: &str) {
    w.press(app, tile, Role::ListItem);
    w.key(app, egui::Key::F2);
    w.type_text(app, text);
}

/// A name committed in an artboard tile renames the artboard (`artboard.rename`, undoable) and the
/// dialog stays open on the same checks; Escape cancels only the name.
#[test]
fn renaming_an_artboard_in_export_for_screens_renames_it() {
    for view in ["grid", "list"] {
        let (mut app, log) = app(2, true);
        let mut w = Window::default();
        w.names(&mut app);
        open(&mut app);
        set(&mut app, "view", json!(view));
        let old = app.session.active().unwrap().doc.artboards[0].name.clone();
        type_artboard_name(&mut w, &mut app, &format!("1  {old}"), "Renamed");
        let before = field(&app, "boards");
        w.key(&mut app, egui::Key::Enter);
        assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Renamed", "{view}");
        assert_eq!(kind(&app).as_deref(), Some(export_for_screens::KIND), "{view}");
        assert_eq!(field(&app, "boards"), before, "{view}: renaming must not change the checked artboards");
        assert!(log.borrow().is_empty(), "{view}: renaming must not export");
        type_artboard_name(&mut w, &mut app, "1  Renamed", "Discard");
        w.key(&mut app, egui::Key::Escape);
        assert_eq!(kind(&app).as_deref(), Some(export_for_screens::KIND), "{view}");
        assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Renamed", "{view}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.artboards[0].name, old, "{view}");
    }
}

/// A name typed while another document becomes active (a command from the control channel or MCP)
/// renames no artboard of the active document.
#[test]
fn an_artboard_name_stays_with_its_document() {
    let (mut app, log) = app(2, true);
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": 2})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    let mut w = Window::default();
    w.names(&mut app);
    open(&mut app);
    let names = |app: &VectorcraftApp, i: usize| app.session.documents()[i].doc.artboards.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    type_artboard_name(&mut w, &mut app, "2  Artboard 2", "Elsewhere");
    app.run("document.activate", json!({"index": 1})).unwrap();
    w.key(&mut app, egui::Key::Enter);
    assert_eq!(names(&app, 0), ["Artboard 1", "Artboard 2"]);
    assert_eq!(names(&app, 1), ["Artboard 1", "Artboard 2"]);
    assert!(log.borrow().is_empty(), "renaming must not export");
}

/// In a window of the default size and in one of the app's least size, every control of the dialog stays between its
/// sides, the format rows stay in the right column, and in the default window the Formats heading's buttons end at
/// the column's right edge.
#[test]
fn the_format_rows_fit_the_right_column_at_the_default_and_least_window_sizes() {
    let rows = json!([
        {"scale": "1x", "suffix": "", "format": "png"},
        {"scale": "2x", "suffix": "@2x-long-suffix", "format": "jpg", "quality": 100},
        {"scale": "100w", "format": "pdf"},
    ]);
    // The default window, then the least size the desktop app's window takes.
    for (size, default) in [(egui::vec2(1280.0, 900.0), true), (egui::vec2(800.0, 500.0), false)] {
        let (mut app, _) = app(2, false);
        let mut w = Window::sized(size);
        w.run(&mut app, vec![]);
        open(&mut app);
        set(&mut app, "formats", rows.clone());
        w.run(&mut app, vec![]);
        let out = w.run(&mut app, vec![]);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let rect = |n: &egui::accesskit::Node| {
            n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
        };
        let find = |label: &str, role: Role| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label) && n.role() == role)
                .and_then(|(_, n)| rect(n))
                .unwrap_or_else(|| panic!("no {role:?} {label:?}"))
        };
        let Some((dialog_id, dialog)) = nodes.iter().find(|(_, n)| n.role() == Role::Dialog) else { panic!("no dialog at {size:?}") };
        let dialog = rect(dialog).expect("the dialog's bounds");
        // The Export to: section spans the right column.
        let column = find("Export to:", Role::Group);
        let by_id: std::collections::HashMap<_, _> = nodes.iter().map(|(id, n)| (*id, n)).collect();
        let mut under = vec![*dialog_id];
        while let Some(id) = under.pop() {
            let Some(n) = by_id.get(&id) else { continue };
            under.extend(n.children().iter().copied());
            if let Some(r) = rect(n) {
                assert!(
                    r.left() >= dialog.left() - 0.5 && r.right() <= dialog.right() + 0.5,
                    "{:?} {:?} at {r:?} is outside the dialog at {dialog:?} in a {size:?} window",
                    n.role(),
                    n.label()
                );
            }
        }
        // Each control with how many of it the three rows show.
        let format_rows = [
            ("Formats:", Role::Label, 1),
            ("iOS", Role::Button, 1),
            ("Android", Role::Button, 1),
            ("Format Presets", Role::Button, 1),
            ("Format Settings", Role::Button, 1),
            ("Scale", Role::ComboBox, 3),
            ("Suffix", Role::TextInput, 3),
            ("Format", Role::ComboBox, 3),
            ("Remove", Role::Button, 3),
            ("+ Add Scale", Role::Button, 1),
        ];
        for (label, role, count) in format_rows {
            let shown: Vec<_> = nodes.iter().filter(|(_, n)| n.label() == Some(label) && n.role() == role).collect();
            assert_eq!(shown.len(), count, "{role:?} {label:?} in a {size:?} window");
            for (_, n) in shown {
                let r = rect(n).expect("bounds");
                assert!(
                    r.left() >= column.left() - 0.5 && r.right() <= column.right() + 0.5,
                    "{label} at {r:?} is outside the column at {column:?} in a {size:?} window"
                );
            }
        }
        if default {
            let gear = find("Format Settings", Role::Button);
            assert!((gear.right() - column.right()).abs() <= 0.5, "the gear at {gear:?} ends at the column's edge {column:?}");
        }
    }
}
