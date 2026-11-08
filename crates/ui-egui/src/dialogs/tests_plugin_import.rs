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

/// The extensions File › Open's filters leave out of importers' (`fileio::opens_as_other`) are the
/// ones File › Open installs, imports, checks or loads as something other than a document.
#[test]
fn importer_filters_leave_out_what_file_open_opens_as_something_else() {
    use vectorcraft_engine::cmd::{self, fileio};
    let lists: [&[&str]; 10] = [
        cmd::plugin::EXTS,
        cmd::shortcutset::EXTS,
        cmd::scripts::EXTS,
        cmd::flatten::PRESET_EXTS,
        cmd::pdfcmds::PRESET_EXTS,
        cmd::printpresets::PRESET_EXTS,
        cmd::perspgrid::PRESET_EXTS,
        fileio::SCREEN_EXPORT_PRESET_EXTS,
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
