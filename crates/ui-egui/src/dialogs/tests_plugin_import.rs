//! Importer plug-ins in File › Open With and File › Open: listed by name under Open With, which
//! opens a file with the importer also when a built-in format reads its extension, and a file only
//! an importer reads is not read past the importer's input limit. Each test installs importers of
//! its own and removes them: plug-ins are process-wide, and an importer left installed would show
//! in other tests' Open dialog.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat::{self as tpl, Given};

use super::tests_export::{frame, kind, set};
use super::*;
use crate::Services;
use crate::menus::Item;

/// An importer's output: one rectangle from (10, 20) to (110, 70).
const RECT: &str =
    r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":true,"anchors":[{"p":[10,20]},{"p":[110,20]},{"p":[110,70]},{"p":[10,70]}]}]}}]}"#;

/// An app that reads files from disk, refusing to read one past the importers' input limit.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
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
    let listed = menu.iter().any(|i| match i {
        Item::Cmd(label, "file.openWith", p) => *label == "Test test.ui.svgreader…" && p["importer"] == "plugin.test.ui.svgreader",
        _ => false,
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
    let (req, _rx) = crate::control::ControlRequest::new("app.open", json!({"path": svg, "importer": "plugin.test.ui.svgreader"}));
    let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &egui::Context::default(), &req) else { panic!("not done") };
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
    assert_eq!(kind(&app), Some(plugin::IMPORT_KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("__label"), d.fields.get("density")), ("Test test.ui.importparams".to_string(), Some(&json!(4.0))));
    assert!(app.session.documents().is_empty(), "nothing opened before Open");
    frame(&mut app);
    set(&mut app, "density", json!(7));
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(artboard(&app), (10.0, 20.0, 110.0, 70.0));
    assert_eq!(app.ui.recent_files, std::slice::from_ref(&file));

    // Only a file from the picker goes off the UI thread after the dialog: a caller naming `path`
    // gets the document.
    app.run("file.openWith", json!({"importer": "plugin.test.ui.importparams", "path": file})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__open"]["picked"], false);
    app.ui.dialog = None;
    let picked = file.clone();
    app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
    app.run("file.openWith", json!({"importer": "plugin.test.ui.importparams"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__open"]["picked"], true);
    app.ui.dialog = None;

    let r = app.run("file.openWith", json!({"importer": "plugin.test.ui.importparams", "path": file, "params": {"density": 7}})).unwrap();
    assert_eq!(r["format"], "plugin.test.ui.importparams");
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.documents().len(), 2);

    // `app.open` (MCP's open_file) passes `params` on, and asks nothing without them.
    let control = |app: &mut VectorcraftApp, p: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new("app.open", p);
        let crate::control::Outcome::Done(r) = crate::control::handle(app, &egui::Context::default(), &req) else { panic!("not done") };
        r
    };
    let r = control(&mut app, json!({"path": file, "importer": "plugin.test.ui.importparams", "params": {"density": 7}}));
    assert_eq!(r["result"]["format"], "plugin.test.ui.importparams", "{r}");
    assert_eq!(app.session.documents().len(), 3);
    let r = control(&mut app, json!({"path": file, "importer": "plugin.test.ui.importparams"}));
    assert!(r.get("error").is_some(), "the defaults (density 4) don't open the file: {r}");
    assert_ne!(kind(&app), Some(plugin::IMPORT_KIND));
    assert_eq!(app.session.documents().len(), 3);

    // Without `importer`, `params` go to the importer File › Open picks for the file.
    let other = dir.join("b.tup");
    std::fs::write(&other, "not json").unwrap();
    let other = other.to_string_lossy().into_owned();
    let r = control(&mut app, json!({"path": other, "params": {"density": 7}}));
    assert_eq!(r["result"]["format"], "plugin.test.ui.importparams", "{r}");
    assert_eq!(app.session.documents().len(), 4);
    app.run("plugin.remove", json!({"id": "test.ui.importparams"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// File › Open's dialog lists an importer's files; the open dialogs whose files only the built-in
/// readers read (New from Template, Load Styles, the library panels' Other Library…) don't.
#[test]
fn only_file_open_s_dialog_lists_an_importer_s_files() {
    let mut app = app();
    install(&mut app, &tpl::module(&tpl::importer("test.ui.filters", r#"["tuf"]"#, "{}"), tpl::ECHO, "", 1));
    let asked: std::rc::Rc<std::cell::RefCell<Vec<crate::FilePick>>> = Default::default();
    let seen = asked.clone();
    app.services.pick_open = Some(Box::new(move |pick: &crate::FilePick| {
        seen.borrow_mut().push(pick.clone());
        None
    }));
    let lists = |filters: &[(String, Vec<String>)]| filters.iter().any(|(_, exts)| exts.iter().any(|e| e == "tuf"));
    let shown = |app: &mut VectorcraftApp, cmd: &str| {
        let _ = app.run(cmd, json!({}));
        let pick = asked.borrow_mut().pop().unwrap_or_else(|| panic!("{cmd} showed no open dialog"));
        lists(&pick.filters)
    };
    assert!(shown(&mut app, "file.open"), "File › Open");
    for cmd in ["file.newFromTemplate", "ui.loadCharStyles", "ui.loadParaStyles", "ui.loadAllStyles"] {
        assert!(!shown(&mut app, cmd), "{cmd}");
    }
    for kind in ["swatches", "graphicStyles", "symbols"] {
        let filters = crate::FilePick::filters(crate::panels::library_panel::library_pick_filters(kind));
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
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("format"), "png");
    app.run("plugin.remove", json!({"id": "test.ui.readonly"})).unwrap();
    app.run("plugin.remove", json!({"id": "test.ui.writes"})).unwrap();
}

/// A file only an importer reads opens wherever it is dropped, also when an exporter lists its
/// extension; a file File › Place reads is still placed on the canvas.
#[test]
fn a_dropped_file_only_an_importer_reads_opens() {
    use crate::place::DropTarget;
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)));
    let on_canvas = Some(egui::pos2(400.0, 300.0));
    let placed = |app: &VectorcraftApp, name, pos| matches!(app.drop_target(name, pos, false), DropTarget::Place(_));
    assert!(placed(&app, "a.tud", on_canvas), "no importer lists the extension yet");

    install(&mut app, &tpl::module(&tpl::importer("test.ui.dropped", r#"["tud","tus","png"]"#, "{}"), tpl::ECHO, "", 1));
    install(&mut app, &tpl::module(&tpl::exporter("test.ui.dropshare", r#"["tus"]"#, "{}"), tpl::ECHO, "", 1));
    for name in ["a.tud", "A.TUD", "a.tus"] {
        assert_eq!(app.drop_target(name, on_canvas, false), DropTarget::Open, "{name} on the canvas");
        assert_eq!(app.drop_target(name, None, false), DropTarget::Open, "{name} without a position");
    }
    assert!(placed(&app, "a.png", on_canvas), "File › Place reads the extension");
    assert!(placed(&app, "a.png", None));

    app.run("plugin.remove", json!({"id": "test.ui.dropped"})).unwrap();
    app.run("plugin.remove", json!({"id": "test.ui.dropshare"})).unwrap();
}

/// A file only an importer that places its files reads, dropped on the canvas, is placed there as
/// one group by that importer, as one undo step; dropped where the platform gives no position, it
/// opens as its own document, as a vector file does.
#[test]
fn a_dropped_file_a_placing_importer_reads_is_placed_on_the_canvas() {
    use crate::place::{DropTarget, drop_files};
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)));
    let on_canvas = Some(egui::pos2(400.0, 300.0));
    let manifest = tpl::importer("test.ui.dropplace", r#"["tdp"]"#, "{}").replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    assert!(manifest.contains(r#""place":true"#), "{manifest}");
    install(&mut app, &tpl::returning(&manifest, RECT));

    let target = app.drop_target("Drawing.tdp", on_canvas, false);
    assert!(matches!(target, DropTarget::Place(_)), "{target:?}");
    assert_eq!(app.drop_target("Drawing.tdp", None, false), DropTarget::Open);
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
        assert!(app.ui.dialog.is_none(), "{name}: no import options dialog");
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
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::place::KIND));
    let steps = app.session.active().unwrap().history.undo.len();
    let r = crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(r["format"], "plugin.test.ui.placepaths", "{r} {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);

    app.run("file.place", given(&paths)).unwrap();
    crate::dialogs::confirm(&mut app).unwrap();
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
    let fields = |app: &mut VectorcraftApp, p: serde_json::Value| {
        app.run("file.place", p).unwrap();
        let d = app.ui.dialog.take().expect("the Place dialog");
        (d.fields["__documents"].clone(), d.fields["__others"].clone())
    };
    assert_eq!(fields(&mut app, json!({"paths": [path]})), (json!(true), json!(false)));
    assert_eq!(fields(&mut app, json!({"paths": [path], "importer": "plugin.test.ui.placepdf"})), (json!(false), json!(true)));
    app.run("plugin.remove", json!({"id": "test.ui.placepdf"})).unwrap();
}

/// On the web, `file.place` with `importer` and `params` and no file starts the browser's pick, and
/// the Place dialog's OK places the picked file with that importer and those parameters. Each
/// pick's files keep that pick's importer, also when another pick started before they arrived, and
/// a pick with another importer waits for its own dialog.
#[test]
fn a_web_pick_places_with_the_importer_and_params_given() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    let manifest = tpl::importer("test.ui.webpick", r#"["svg"]"#, params).replace(r#""kind":"importer""#, r#""kind":"importer","place":true"#);
    install(&mut app, &tpl::when_given(&manifest, Given::Params, r#""density":7.0"#, RECT));
    let inbox = crate::place::PlaceInbox::default();
    app.services.place_inbox = Some(inbox.clone());
    let started: std::rc::Rc<std::cell::RefCell<Vec<serde_json::Value>>> = Default::default();
    let starts = started.clone();
    app.services.place_async = Some(Box::new(move |with| starts.borrow_mut().push(with)));

    app.run("file.place", json!({"importer": "plugin.test.ui.webpick", "params": {"density": 7}, "showImportOptions": false})).unwrap();
    app.run("file.place", json!({})).unwrap();
    let arrive = |name: &str, with: serde_json::Value| {
        inbox.lock().unwrap().push(crate::place::PlaceArrival { name: name.into(), bytes: b"?".to_vec(), drop: None, with })
    };
    arrive("a.svg", started.borrow()[0].clone());
    arrive("b.svg", started.borrow()[1].clone());
    crate::place::drain(&mut app);
    let files = |app: &VectorcraftApp| app.ui.dialog.as_ref().filter(|d| d.kind == crate::dialogs::place::KIND).map(|d| d.fields["files"].clone());
    assert_eq!(files(&app), Some(json!([{"name": "a.svg", "importer": "plugin.test.ui.webpick", "params": {"density": 7}}])));
    let steps = app.session.active().unwrap().history.undo.len();
    let r = crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(r["format"], "plugin.test.ui.webpick", "{r} {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);
    crate::place::drain(&mut app);
    assert_eq!(files(&app), Some(json!([{"name": "b.svg"}])));

    app.run("plugin.remove", json!({"id": "test.ui.webpick"})).unwrap();
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
    let asked: std::rc::Rc<std::cell::RefCell<Option<crate::FilePick>>> = Default::default();
    let seen = asked.clone();
    app.services.pick_open = Some(Box::new(move |pick: &crate::FilePick| {
        *seen.borrow_mut() = Some(pick.clone());
        None
    }));
    let _ = app.run("file.open", json!({}));
    let pick = asked.borrow_mut().take().expect("File › Open showed its dialog");
    let own = pick.filters.iter().find(|(label, _)| label == "Test test.ui.claimed").map(|(_, exts)| exts.clone());
    assert_eq!(own, Some(vec!["tuo".to_string()]), "{:?}", pick.filters);
    assert!(!pick.filters[0].1.iter().any(|e| e == "ase"), "All readable files: {:?}", pick.filters[0]);
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
