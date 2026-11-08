//! Importer plug-ins in File › Open With and File › Open: listed by name under Open With, which
//! opens a file with the importer also when a built-in format reads its extension, and a file only
//! an importer reads is not read past the importer's input limit. Each test installs importers of
//! its own and removes them: plug-ins are process-wide, and an importer left installed would show
//! in other tests' Open dialog.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

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
    let r = app.run("file.openWith", json!({"importer": "plugin.test.ui.svgreader", "path": svg})).unwrap();
    assert_eq!(r["format"], "plugin.test.ui.svgreader");
    assert_eq!(artboard(&app), (10.0, 20.0, 110.0, 70.0), "Open With: the importer");
    assert_eq!(app.session.documents().len(), 2);
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
