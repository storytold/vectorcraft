//! Importer plug-ins in File › Open With and File › Open: listed by name under Open With, which
//! opens a file with the importer also when a built-in format reads its extension, and a file only
//! an importer reads is not read past the importer's input limit. Each test installs importers of
//! its own and removes them: plug-ins are process-wide, and an importer left installed would show
//! in other tests' Open dialog.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat::{self as tpl, Given};

use super::tests_export::{cancel, fields, frame, kind, set, try_confirm};
use super::*;
use crate::Services;
use crate::VectorcraftApp;
use crate::tests::picks::{poll, scripted};

/// Sends `app.open` with `params` through the control channel and returns the reply.
fn control(app: &mut VectorcraftApp, params: serde_json::Value) -> serde_json::Value {
    let (req, _rx) = craft::automation::ControlRequest::new("app.open", params);
    let craft::automation::Reply::Done(r) = crate::control::handle(app, &crate::testing::ui_context(), &req) else { panic!("not done") };
    r
}

/// The extensions of the filter named `name` in the last request `requests` holds.
fn filter_exts(requests: &[craft::host::PickRequest], name: &str) -> Option<Vec<String>> {
    requests.last()?.filters.iter().find(|f| f.name == name).map(|f| f.extensions.clone())
}

/// An importer's output: one rectangle from (10, 20) to (110, 70).
const RECT: &str =
    r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":true,"anchors":[{"p":[10,20]},{"p":[110,20]},{"p":[110,70]},{"p":[10,70]}]}]}}]}"#;

/// An app that reads files from disk, refusing to read one past the importers' input limit.
fn app() -> VectorcraftApp {
    // A window, where the dialogs open.
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
    app.services.read = Some(Box::new(|p: &str| match std::fs::metadata(p) {
        Ok(m) if m.len() > vectorcraft_plugins::import::MAX_INPUT_BYTES as u64 => Err("read past the limit".into()),
        _ => std::fs::read(p).map_err(|e| e.to_string()),
    }));
    app
}

fn install(app: &mut VectorcraftApp, src: &str) {
    let wasm = wat::parse_str(src).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
}

/// A fresh folder for files this test writes.
fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-ui-plugin-import-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The active document's first artboard, as `(x0, y0, x1, y1)`.
fn artboard(app: &VectorcraftApp) -> (f64, f64, f64, f64) {
    let r = app.session.doc().unwrap().doc.artboards[0].rect;
    (r.x0, r.y0, r.x1, r.y1)
}

/// Open With lists an importer by name; it opens an `.svg` with the importer, where File › Open
/// keeps the built-in SVG reader.
#[test]
fn open_with_lists_importers_and_opens_a_file_with_one() {
    let dir = tmp_dir("with");
    let mut app = app();
    install(&mut app, &tpl::returning(&tpl::importer("test.ui.svgreader", r#"["svg","tui"]"#, "{}"), RECT));
    let menu = plugin::open_with_menu();
    let listed = menu.iter().any(|n| {
        matches!(n, craft::menu::Node::Item(item) if item.command.id == "file.openWith"
            && item.label == "Test test.ui.svgreader…"
            && item.command.params["importer"] == "plugin.test.ui.svgreader")
    });
    assert!(listed, "{menu:?}");

    let svg = dir.join("a.svg");
    std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="40"><rect width="10" height="10"/></svg>"#).unwrap();
    let svg = svg.to_string_lossy().into_owned();
    crate::io::open_path(&mut app, &svg).unwrap();
    assert_eq!(artboard(&app), (0.0, 0.0, 50.0, 40.0), "File › Open: the built-in SVG reader");
    app.ui.recent_files.clear();
    let r = app.run("file.openWith", json!({"importer": "plugin.test.ui.svgreader", "path": svg})).unwrap();
    assert_eq!(r["format"], "plugin.test.ui.svgreader");
    assert_eq!(artboard(&app), (10.0, 20.0, 110.0, 70.0), "Open With: the importer");
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.ui.recent_files, std::slice::from_ref(&svg), "Open With adds the file to Open Recent");
    // `app.open {importer}` (MCP's open_file) opens as Open With does.
    let r = control(&mut app, json!({"path": svg, "importer": "plugin.test.ui.svgreader"}));
    assert_eq!(r["result"]["format"], "plugin.test.ui.svgreader", "{r}");
    assert_eq!(app.session.documents().len(), 3);

    // An importer is not a filter: Object › Plug-ins' command says where it is.
    let e = app.run("plugin.dialog", json!({"id": "test.ui.svgreader"})).unwrap_err();
    assert!(e.contains("is an importer (File › Open With)"), "{e}");
    let e = app.run("file.openWith", json!({"path": svg})).unwrap_err();
    assert!(e.contains("missing `importer`"), "{e}");
    app.run("plugin.remove", json!({"id": "test.ui.svgreader"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open With asks for an importer's parameters, at their defaults, once the file is picked, and
/// opens the file with the values set; given `params`, it asks nothing, and `app.open` without
/// them runs the importer on their defaults.
#[test]
fn open_with_asks_for_an_importer_s_parameters_and_passes_them_on() {
    let dir = tmp_dir("params");
    let mut app = app();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    // RECT when the parameters hold density 7; the file, which isn't JSON, otherwise.
    install(&mut app, &tpl::when_given(&tpl::importer("test.ui.importparams", r#"["tup"]"#, params), Given::Params, r#""density":7.0"#, RECT));
    let file = dir.join("a.tup");
    std::fs::write(&file, "not json").unwrap();
    let file = file.to_string_lossy().into_owned();

    app.run("file.openWith", json!({"importer": "plugin.test.ui.importparams", "path": file})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some(plugin::IMPORT_KIND));
    assert!(!app.plugin_dialog.as_ref().is_some_and(plugin::PluginDialog::opens_picked), "a named file opens in the pump");
    assert_eq!(fields(&app)["density"], json!(4.0));
    assert!(app.session.documents().is_empty(), "nothing opened before Open");
    frame(&mut app);
    set(&mut app, "density", json!(7));
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none());
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(artboard(&app), (10.0, 20.0, 110.0, 70.0));
    assert_eq!(app.ui.recent_files, std::slice::from_ref(&file));

    // A caller naming `path` gets the document in the pump; a file from the picker is marked to go through
    // `opening`, off the UI thread where the app can. Tests open at once (`opening::available` is off).
    let picker = scripted(&mut app);
    app.run("file.openWith", json!({"importer": "plugin.test.ui.importparams"})).unwrap();
    picker.answer(&[&file]);
    poll(&mut app, &crate::testing::ui_context());
    assert_eq!(kind(&app).as_deref(), Some(plugin::IMPORT_KIND));
    assert!(app.plugin_dialog.as_ref().is_some_and(plugin::PluginDialog::opens_picked), "a picked file goes through `opening`");
    frame(&mut app);
    set(&mut app, "density", json!(7));
    try_confirm(&mut app).unwrap();
    assert_eq!(app.session.documents().len(), 2);

    let r = app.run("file.openWith", json!({"importer": "plugin.test.ui.importparams", "path": file, "params": {"density": 7}})).unwrap();
    assert_eq!(r["format"], "plugin.test.ui.importparams");
    assert!(kind(&app).is_none());
    assert_eq!(app.session.documents().len(), 3);

    // `app.open` (MCP's open_file) passes `params` on, and asks nothing without them.
    let r = control(&mut app, json!({"path": file, "importer": "plugin.test.ui.importparams", "params": {"density": 7}}));
    assert_eq!(r["result"]["format"], "plugin.test.ui.importparams", "{r}");
    assert_eq!(app.session.documents().len(), 4);
    let r = control(&mut app, json!({"path": file, "importer": "plugin.test.ui.importparams"}));
    assert!(r.get("error").is_some(), "the defaults (density 4) don't open the file: {r}");
    assert_ne!(kind(&app).as_deref(), Some(plugin::IMPORT_KIND));
    assert_eq!(app.session.documents().len(), 4);

    // Without `importer`, `params` go to the importer File › Open picks for the file.
    let other = dir.join("b.tup");
    std::fs::write(&other, "not json").unwrap();
    let other = other.to_string_lossy().into_owned();
    let r = control(&mut app, json!({"path": other, "params": {"density": 7}}));
    assert_eq!(r["result"]["format"], "plugin.test.ui.importparams", "{r}");
    assert_eq!(app.session.documents().len(), 5);
    app.run("plugin.remove", json!({"id": "test.ui.importparams"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// File › Open's dialog lists an importer's files; the open dialogs whose files only the built-in
/// readers read (New from Template, Load Styles, the library panels' Other Library…) don't.
#[test]
fn only_file_open_s_dialog_lists_an_importer_s_files() {
    let mut app = app();
    install(&mut app, &tpl::module(&tpl::importer("test.ui.filters", r#"["tuf"]"#, "{}"), tpl::ECHO, "", 1));
    let picker = scripted(&mut app);
    let lists = |filters: &[(String, Vec<String>)]| filters.iter().any(|(_, exts)| exts.iter().any(|e| e == "tuf"));
    let shown = |app: &mut VectorcraftApp, cmd: &str| {
        let before = picker.requests().len();
        let _ = app.run(cmd, json!({}));
        let requests = picker.requests();
        assert!(requests.len() > before, "{cmd} showed no open dialog");
        let filters: Vec<(String, Vec<String>)> =
            requests.last().map(|r| r.filters.iter().map(|f| (f.name.clone(), f.extensions.clone())).collect()).unwrap_or_default();
        lists(&filters)
    };
    assert!(shown(&mut app, "file.open"), "File › Open");
    for cmd in ["file.newFromTemplate", "ui.loadCharStyles", "ui.loadParaStyles", "ui.loadAllStyles"] {
        assert!(!shown(&mut app, cmd), "{cmd}");
    }
    for kind in ["swatches", "graphicStyles", "symbols"] {
        let filters: Vec<(String, Vec<String>)> = crate::panels::library_panel::library_pick_filters(kind)
            .into_iter()
            .map(|(name, exts)| (name.to_owned(), exts.iter().map(|e| (*e).to_owned()).collect()))
            .collect();
        assert!(!lists(&filters), "{kind}");
    }
    app.run("plugin.remove", json!({"id": "test.ui.filters"})).unwrap();
}

/// Export As lists an installed exporter and not an installed importer, whose format can only be
/// read, and it does not preset the importer's format.
#[test]
fn export_as_lists_exporters_and_not_importers() {
    let mut app = app();
    install(&mut app, &tpl::module(&tpl::importer("test.ui.readonly", r#"["tur"]"#, "{}"), tpl::ECHO, "", 1));
    install(&mut app, &tpl::module(&tpl::exporter("test.ui.writes", r#"["tuw"]"#, "{}"), tpl::ECHO, "", 1));
    let listed: Vec<&str> = export_as::formats().iter().map(|f| f.id).collect();
    assert!(listed.contains(&"plugin.test.ui.writes"), "{listed:?}");
    assert!(!listed.contains(&"plugin.test.ui.readonly"), "{listed:?}");
    app.run("file.exportAs", json!({"format": "plugin.test.ui.readonly"})).unwrap();
    assert_eq!(fields(&app)["format"], "png");
    app.run("plugin.remove", json!({"id": "test.ui.readonly"})).unwrap();
    app.run("plugin.remove", json!({"id": "test.ui.writes"})).unwrap();
}

/// A file only an importer reads opens wherever it is dropped, also when an exporter lists its
/// extension; a file File › Place reads is still placed on the canvas.
#[test]
fn a_dropped_file_only_an_importer_reads_opens() {
    use crate::place::{DropSpot, DropTarget};
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let on_canvas = DropSpot::Canvas(vectorcraft_geom::Point::new(400.0, 300.0));
    let placed = |app: &VectorcraftApp, name, spot| matches!(app.drop_target(name, spot, false), DropTarget::Place(_));
    assert!(placed(&app, "a.tud", on_canvas), "no importer lists the extension yet");

    install(&mut app, &tpl::module(&tpl::importer("test.ui.dropped", r#"["tud","tus","png"]"#, "{}"), tpl::ECHO, "", 1));
    install(&mut app, &tpl::module(&tpl::exporter("test.ui.dropshare", r#"["tus"]"#, "{}"), tpl::ECHO, "", 1));
    for name in ["a.tud", "A.TUD", "a.tus"] {
        assert_eq!(app.drop_target(name, on_canvas, false), DropTarget::Open, "{name} on the canvas");
        assert_eq!(app.drop_target(name, DropSpot::Unknown, false), DropTarget::Open, "{name} without a position");
    }
    assert!(placed(&app, "a.png", on_canvas), "File › Place reads the extension");
    assert!(placed(&app, "a.png", DropSpot::Unknown));

    app.run("plugin.remove", json!({"id": "test.ui.dropped"})).unwrap();
    app.run("plugin.remove", json!({"id": "test.ui.dropshare"})).unwrap();
}

/// A file only an importer that places its files reads, dropped on the canvas, is placed there as
/// one group by that importer, as one undo step; dropped where the platform gives no position, it
/// opens as its own document, as a vector file does.
#[test]
fn a_dropped_file_a_placing_importer_reads_is_placed_on_the_canvas() {
    use crate::place::{DropSpot, DropTarget, drop_files};
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let on_canvas = DropSpot::Canvas(vectorcraft_geom::Point::new(400.0, 300.0));
    let manifest = tpl::importer("test.ui.dropplace", r#"["tdp"]"#, "{}").replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    assert!(manifest.contains(r#""place":true"#), "{manifest}");
    install(&mut app, &tpl::returning(&manifest, RECT));

    let target = app.drop_target("Drawing.tdp", on_canvas, false);
    assert!(matches!(target, DropTarget::Place(_)), "{target:?}");
    assert_eq!(app.drop_target("Drawing.tdp", DropSpot::Unknown, false), DropTarget::Open);
    let steps = app.session.active().unwrap().history.undo.len();
    drop_files(&mut app, vec![(target, ("Drawing.tdp".into(), None, b"?".to_vec()))]);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), steps + 1, "one undo step");
    let [id] = st.selection.objects[..] else { panic!("one placed object: {:?}", st.selection.objects) };
    let n = st.doc.node(id).unwrap();
    assert!(matches!(n.kind, vectorcraft_doc::NodeKind::Group { .. }) && n.name.as_deref() == Some("Drawing.tdp"), "{n:?}");

    app.run("plugin.remove", json!({"id": "test.ui.dropplace"})).unwrap();
}

/// A text file or DXF drawing placed with the importer `importer` names goes to that importer,
/// without the text or DXF import options File › Place asks for those formats.
#[test]
fn a_named_placing_importer_skips_built_in_import_options() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let manifest = tpl::importer("test.ui.namedplace", r#"["txt","dxf"]"#, "{}").replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    install(&mut app, &tpl::returning(&manifest, RECT));
    for name in ["Notes.txt", "Plan.dxf"] {
        let steps = app.session.active().unwrap().history.undo.len();
        let p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(b"0\nSECTION\n"), "importer": "plugin.test.ui.namedplace"});
        let r = app.run("file.place", p).unwrap();
        assert!(kind(&app).is_none(), "{name}: no import options dialog");
        assert_eq!(r["format"], "plugin.test.ui.namedplace", "{name}: {r}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "{name}");
    }
    app.run("plugin.remove", json!({"id": "test.ui.namedplace"})).unwrap();
}

/// `file.place` with `paths`, `importer` and `params` opens the Place dialog, and its OK places the
/// files with that importer and those parameters: one file at once, several through the place
/// cursor. The files are SVG files by their extension, which the built-in reader can't read.
#[test]
fn the_place_dialog_places_paths_with_the_importer_and_params_given() {
    let dir = tmp_dir("placepaths");
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    let manifest = tpl::importer("test.ui.placepaths", r#"["svg"]"#, params).replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    install(&mut app, &tpl::when_given(&manifest, Given::Params, r#""density":7.0"#, RECT));
    let paths: Vec<String> = ["a.svg", "b.svg"]
        .iter()
        .map(|n| {
            let p = dir.join(n);
            std::fs::write(&p, b"?").unwrap();
            p.to_string_lossy().into_owned()
        })
        .collect();
    let given =
        |paths: &[String]| json!({"paths": paths, "importer": "plugin.test.ui.placepaths", "params": {"density": 7}, "showImportOptions": false});

    app.run("file.place", given(&paths[..1])).unwrap();
    assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::place::KIND));
    let steps = app.session.active().unwrap().history.undo.len();
    try_confirm(&mut app).unwrap();
    assert_eq!(placed_group(&app).as_deref(), Some("a.svg"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);

    app.run("file.place", given(&paths)).unwrap();
    try_confirm(&mut app).unwrap();
    assert_eq!(app.session.tool_options()["count"], 2, "{}", app.ui.status);

    app.run("plugin.remove", json!({"id": "test.ui.placepaths"})).unwrap();
}

/// A PDF file placed with an importer plug-in named is not read to tell whether Link would place it
/// as a document: the Place dialog counts it as a file the importer embeds. The file here doesn't
/// exist, which counts as a document when its start is read.
#[test]
fn the_place_dialog_does_not_read_a_file_a_named_importer_places() {
    let dir = tmp_dir("placenamedpdf");
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let manifest = tpl::importer("test.ui.placepdf", r#"["pdf"]"#, "{}").replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    install(&mut app, &tpl::returning(&manifest, RECT));
    let path = dir.join("missing.pdf").to_string_lossy().into_owned();
    let counted = |app: &mut VectorcraftApp, p: serde_json::Value| {
        app.run("file.place", p).unwrap();
        assert_eq!(kind(app).as_deref(), Some(crate::dialogs::place::KIND));
        let f = fields(app);
        cancel(app);
        (f["__documents"].clone(), f["__others"].clone())
    };
    assert_eq!(counted(&mut app, json!({"paths": [path]})), (json!(true), json!(false)));
    assert_eq!(counted(&mut app, json!({"paths": [path], "importer": "plugin.test.ui.placepdf"})), (json!(false), json!(true)));
    app.run("plugin.remove", json!({"id": "test.ui.placepdf"})).unwrap();
}

/// `file.place` with `importer` and `params` and no file asks for the files, and the Place dialog's
/// OK places the picked files with that importer and those parameters: files the picker gives by
/// their contents (the web) and by their paths. A pick without them places with neither, also while
/// a pick with them is open.
#[test]
fn a_pick_places_with_the_importer_and_params_given() {
    let dir = tmp_dir("placepick");
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    let manifest = tpl::importer("test.ui.placepick", r#"["svg"]"#, params).replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    install(&mut app, &tpl::when_given(&manifest, Given::Params, r#""density":7.0"#, RECT));
    let picker = scripted(&mut app);
    let ctx = crate::testing::ui_context();
    let with = json!({"importer": "plugin.test.ui.placepick", "params": {"density": 7}, "showImportOptions": false});
    let picked_files = |app: &VectorcraftApp| {
        let mut files = fields(app)["files"].clone();
        for file in files.as_array_mut().unwrap() {
            let source = file.as_object_mut().unwrap().remove("__pickedSource").expect("browser files keep their source identity");
            let token = source.as_str().unwrap();
            assert!(app.place.picked.iter().any(|(key, bytes)| key == token && bytes == b"?"));
        }
        files
    };

    // The web: the picker answers with the files' contents.
    app.run("file.place", with.clone()).unwrap();
    picker.answer_contents(&[("a.svg", b"?")]);
    poll(&mut app, &ctx);
    assert_eq!(picked_files(&app), json!([{"name": "a.svg", "importer": "plugin.test.ui.placepick", "params": {"density": 7}}]));
    let steps = app.session.active().unwrap().history.undo.len();
    try_confirm(&mut app).unwrap();
    assert_eq!(placed_group(&app).as_deref(), Some("a.svg"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);

    // The desktop: the picker answers with paths.
    let path = dir.join("b.svg");
    std::fs::write(&path, b"?").unwrap();
    let path = path.to_string_lossy().into_owned();
    app.run("file.place", with.clone()).unwrap();
    picker.answer(&[&path]);
    poll(&mut app, &ctx);
    assert_eq!(fields(&app)["files"], json!([{"path": path, "importer": "plugin.test.ui.placepick", "params": {"density": 7}}]));
    try_confirm(&mut app).unwrap();
    assert_eq!(placed_group(&app).as_deref(), Some("b.svg"), "{}", app.ui.status);

    app.run("file.place", json!({})).unwrap();
    picker.answer_contents(&[("c.svg", b"?")]);
    poll(&mut app, &ctx);
    assert_eq!(picked_files(&app), json!([{"name": "c.svg"}]));
    cancel(&mut app);

    // Two picks open at once: each one's files keep that pick's importer and parameters, also when
    // the later pick is answered first.
    app.run("file.place", with).unwrap();
    app.run("file.place", json!({})).unwrap();
    picker.answer_contents(&[("e.svg", b"?")]);
    poll(&mut app, &ctx);
    assert_eq!(picked_files(&app), json!([{"name": "e.svg"}]));
    cancel(&mut app);
    picker.answer_contents(&[("d.svg", b"?")]);
    poll(&mut app, &ctx);
    assert_eq!(picked_files(&app), json!([{"name": "d.svg", "importer": "plugin.test.ui.placepick", "params": {"density": 7}}]));
    try_confirm(&mut app).unwrap();
    assert_eq!(placed_group(&app).as_deref(), Some("d.svg"), "{}", app.ui.status);

    app.run("plugin.remove", json!({"id": "test.ui.placepick"})).unwrap();
}

/// The name of the one selected object when it is a group: what an importer places.
fn placed_group(app: &VectorcraftApp) -> Option<String> {
    let st = app.session.active()?;
    let [id] = st.selection.objects[..] else { return None };
    let n = st.doc.node(id)?;
    matches!(n.kind, vectorcraft_doc::NodeKind::Group { .. }).then(|| n.name.clone()).flatten()
}

/// The extensions File › Open's filters leave out of importers' (`fileio::opens_as_other`) are the
/// ones File › Open installs, imports, checks or loads as something other than a document.
#[test]
fn importer_filters_leave_out_what_file_open_opens_as_something_else() {
    use vectorcraft_engine::cmd::{self, fileio};
    let lists: [&[&str]; 11] = [
        cmd::plugin::EXTS,
        cmd::shortcutset::EXTS,
        cmd::scripts::EXTS,
        cmd::flatten::PRESET_EXTS,
        cmd::pdfcmds::PRESET_EXTS,
        cmd::printpresets::PRESET_EXTS,
        cmd::perspgrid::PRESET_EXTS,
        fileio::SCREEN_EXPORT_PRESET_EXTS,
        cmd::compositefonts::EXTS,
        cmd::library::LIBRARY_EXTS,
        cmd::swatchlib::LIBRARY_EXTS,
    ];
    for ext in lists.iter().flat_map(|l| l.iter().copied()).chain([vectorcraft_doc::style_libs::STYLES_EXT]) {
        assert!(crate::io::opens_as(&format!("a.{ext}")).is_some(), "{ext}");
        assert!(fileio::opens_as_other(ext), "{ext}");
    }
    for ext in ["svg", "tcl", "pdf"] {
        assert!(crate::io::opens_as(&format!("a.{ext}")).is_none() && !fileio::opens_as_other(ext), "{ext}");
    }
    // File › Open's dialog lists an importer's `.tuo` but not its `.ase` (a swatch library there).
    let mut app = app();
    install(&mut app, &tpl::module(&tpl::importer("test.ui.claimed", r#"["tuo","ase"]"#, "{}"), tpl::ECHO, "", 1));
    let picker = scripted(&mut app);
    let _ = app.run("file.open", json!({}));
    let requests = picker.requests();
    let own = filter_exts(&requests, "Test test.ui.claimed");
    assert_eq!(own, Some(vec!["tuo".to_string()]), "{:?}", requests.last().map(|r| &r.filters));
    let all = requests.last().and_then(|r| r.filters.first()).expect("File › Open lists file types");
    assert!(!all.extensions.iter().any(|e| e == "ase"), "All readable files: {all:?}");
    app.run("plugin.remove", json!({"id": "test.ui.claimed"})).unwrap();
}

/// File › Open reads a file of an extension only an importer lists, and refuses one past the
/// importer's input limit before reading it, as Open With does.
#[test]
fn file_open_refuses_an_importer_s_file_past_the_limit_before_reading_it() {
    let dir = tmp_dir("limit");
    let mut app = app();
    install(&mut app, &tpl::module(&tpl::importer("test.ui.big", r#"["tub"]"#, "{}"), tpl::ECHO, "", 1));
    let small = dir.join("small.tub");
    std::fs::write(&small, RECT).unwrap();
    crate::io::open_path(&mut app, &small.to_string_lossy()).unwrap();
    assert_eq!(artboard(&app), (10.0, 20.0, 110.0, 70.0));

    let big = dir.join("big.tub");
    std::fs::File::create(&big).unwrap().set_len(vectorcraft_plugins::import::MAX_INPUT_BYTES as u64 + 1).unwrap();
    let big = big.to_string_lossy().into_owned();
    let e = crate::io::open_path(&mut app, &big).unwrap_err();
    assert!(e.contains("larger than"), "{e}");
    let e = app.run("file.openWith", json!({"importer": "plugin.test.ui.big", "path": big})).unwrap_err();
    assert!(e.contains("larger than"), "{e}");
    assert_eq!(app.session.documents().len(), 1);
    app.run("plugin.remove", json!({"id": "test.ui.big"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
