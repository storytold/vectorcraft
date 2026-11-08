//! Importer plug-ins (`"kind": "importer"`, docs/plugins.md › Importers) through the engine: the
//! formats they add to `document.formats` and `document.open`, which files they are given, the
//! document their objects become, and what happens when they fail, decline or are canceled.
//!
//! A test binary of its own: installed plug-ins are process-wide, and the unit tests that check the
//! exact list of formats would see these ones.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::{EngineError, Session};
use vectorcraft_plugins::wat::{self as tpl, Given};
use vectorcraft_plugins::{Limits, Plugin};

/// An importer's output: one red rectangle from (10, 20) to (110, 70) and a warning.
const RECT: &str = r#"{"objects":[{"type":"path","name":"Box","path":{"subpaths":[{"closed":true,"anchors":[{"p":[10,20]},{"p":[110,20]},{"p":[110,70]},{"p":[10,70]}]}]},"fills":[{"type":"solid","color":{"model":"rgb","r":1,"g":0,"b":0}}]}],"warnings":["2 layers merged"]}"#;

/// An importer's output: a horizontal line from (0, 5) to (50, 5).
const LINE: &str = r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,5]},{"p":[50,5]}]}]}}]}"#;

/// The `vc_run` body of an importer that never returns.
const SPINS: &str = "(loop $l (br $l)) (i64.const 0)";

/// Installs the plug-in written in WAT `src` → its `plugin.list` entry.
fn install(s: &mut Session, src: &str) -> Value {
    let wasm = wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}"));
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap()
}

/// An importer `id` reading `extensions` (a JSON list) whose objects are the file it is given.
fn echo(s: &mut Session, id: &str, extensions: &str) -> Value {
    install(s, &tpl::module(&tpl::importer(id, extensions, "{}"), tpl::ECHO, "", 1))
}

/// `document.open` of a file `name` holding `bytes`, with more params `extra`.
fn open(s: &mut Session, name: &str, bytes: &[u8], extra: Value) -> Result<Value, EngineError> {
    let mut p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("document.open", &p)
}

/// The active document's first artboard, as `(x0, y0, x1, y1)`.
fn artboard(s: &Session) -> (f64, f64, f64, f64) {
    let r = s.doc().unwrap().doc.artboards[0].rect;
    (r.x0, r.y0, r.x1, r.y1)
}

/// How many objects the active document's first layer holds.
fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers[0].children().unwrap().len()
}

/// The open documents' count, the active one's index, its undo and redo steps, whether it has
/// unsaved changes, and its content.
fn state(s: &mut Session) -> (usize, Option<usize>, usize, usize, bool, Value) {
    let content = s.execute("document.inspect", &json!({})).unwrap();
    let d = s.doc().unwrap();
    (s.documents().len(), s.active_index(), d.history.undo.len(), d.history.redo.len(), d.is_dirty(), content)
}

/// Opens a document through a new importer `id`, with an undo step and a redo step → its [`state`].
fn open_edited(s: &mut Session, id: &str) -> (usize, Option<usize>, usize, usize, bool, Value) {
    install(s, &tpl::returning(&tpl::importer(id, r#"["tsk"]"#, "{}"), RECT));
    open(s, "Kept.tsk", b"?", json!({"importer": format!("plugin.{id}")})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let before = state(s);
    assert!(before.2 > 0 && before.3 > 0, "{before:?}");
    before
}

/// A fresh folder for files this test writes.
fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-plugin-import-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path_of(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn an_importer_adds_a_read_only_format_while_it_is_installed() {
    let mut s = Session::new();
    let entry = echo(&mut s, "test.list", r#"["tsl","tsl2"]"#);
    assert_eq!((&entry["kind"], &entry["format"], &entry["extensions"]), (&json!("importer"), &json!("plugin.test.list"), &json!(["tsl", "tsl2"])));
    assert_eq!(entry["permissions"], json!(["import.read"]));
    let formats = s.execute("document.formats", &json!({})).unwrap();
    let listed = formats["formats"].as_array().unwrap().iter().find(|f| f["id"] == "plugin.test.list").expect("listed");
    assert_eq!((&listed["label"], &listed["extensions"]), (&json!("Test test.list"), &json!(["tsl", "tsl2"])));
    assert_eq!((&listed["read"], &listed["write"], &listed["plugin"]), (&json!(true), &json!(false), &json!("test.list")));
    assert!(formats["readable"].as_array().unwrap().contains(&json!("plugin.test.list")));
    assert!(!formats["writable"].as_array().unwrap().contains(&json!("plugin.test.list")));
    assert!(formats["openExtensions"].as_array().unwrap().contains(&json!("tsl2")));
    assert!(fileio::open_extensions().contains(&"tsl"));
    assert!(!fileio::export_extensions().contains(&"tsl"));
    assert_eq!(fileio::format(".TSL2").map(|f| f.id), Some("plugin.test.list"), "by any of its extensions, in any case");

    s.execute("plugin.remove", &json!({"id": "test.list"})).unwrap();
    let formats = s.execute("document.formats", &json!({})).unwrap();
    assert!(!formats["readable"].as_array().unwrap().contains(&json!("plugin.test.list")));
    assert!(!formats["openExtensions"].as_array().unwrap().contains(&json!("tsl")));
    assert!(fileio::format("tsl").is_none());
    assert!(open(&mut s, "a.tsl", RECT.as_bytes(), json!({})).is_err());
    assert!(s.documents().is_empty());
}

/// A file of an extension only the importer lists opens as a new document of editable art, its
/// artboard the art's bounds, with the importer's warnings; Save then asks for a name.
#[test]
fn a_file_an_importer_reads_opens_as_editable_art() {
    let dir = tmp_dir("open");
    let mut s = Session::new();
    echo(&mut s, "test.open", r#"["tso"]"#);
    let r = open(&mut s, "Shapes.tso", RECT.as_bytes(), json!({})).unwrap();
    assert_eq!((&r["format"], &r["warnings"], &r["restored"]), (&json!("plugin.test.open"), &json!(["2 layers merged"]), &json!(false)));
    assert_eq!(s.documents().len(), 1);
    assert_eq!(objects(&s), 1);
    assert_eq!(artboard(&s), (10.0, 20.0, 110.0, 70.0));
    // Editable: the rectangle moves like any path.
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();

    // A line's artboard is at least 1 pt high.
    open(&mut s, "Line.tso", LINE.as_bytes(), json!({})).unwrap();
    assert_eq!(artboard(&s), (0.0, 4.5, 50.0, 5.5));

    // Art at the edge of the canvas keeps its artboard within ±4,000,000 pt, so the document can
    // still be edited: a vertical line at x = 4,000,000 (its 1 pt moves inward) and a stroked line
    // along y = -4,000,000 (its stroke passes the edge).
    let edge = r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[4000000,0]},{"p":[4000000,50]}]}]}}]}"#;
    open(&mut s, "Edge.tso", edge.as_bytes(), json!({})).unwrap();
    assert_eq!(artboard(&s), (3_999_999.0, 0.0, 4_000_000.0, 50.0));
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": -5, "dy": 0})).unwrap();
    let stroked = r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,-4000000]},{"p":[50,-4000000]}]}]},"strokes":[{"paint":{"type":"solid","color":{"model":"rgb","r":0,"g":0,"b":0}},"width":10}]}]}"#;
    open(&mut s, "Stroked.tso", stroked.as_bytes(), json!({})).unwrap();
    let (x0, y0, x1, y1) = artboard(&s);
    assert!(x0 < 0.0 && x1 > 50.0 && y0 == -4_000_000.0 && y1 > y0 + 1.0, "{:?}", artboard(&s));
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 0, "dy": 5})).unwrap();

    // From a path: the document keeps no path, so Save asks for a name.
    let path = dir.join("Shapes.tso");
    std::fs::write(&path, RECT).unwrap();
    let r = s.execute("document.open", &json!({"path": path_of(&path)})).unwrap();
    assert_eq!(r["format"], "plugin.test.open");
    assert_eq!(s.doc().unwrap().path, None);
    assert!(s.doc().unwrap().doc.title.contains("Shapes"), "{}", s.doc().unwrap().doc.title);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A built-in format keeps its files: an importer that lists `svg` is given an `.svg` only when
/// `importer` names it, and a file with a built-in format's content opens with the built-in reader
/// whatever its extension.
#[test]
fn built_in_formats_keep_their_files() {
    let mut s = Session::new();
    install(&mut s, &tpl::returning(&tpl::importer("test.svgtoo", r#"["tsv","svg"]"#, "{}"), RECT));
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="40"><rect width="10" height="10"/></svg>"#;
    assert_eq!(open(&mut s, "a.svg", svg, json!({})).unwrap()["format"], "svg");
    // A damaged SVG reports the SVG reader's error; the importer isn't tried instead.
    match open(&mut s, "b.svg", b"<svg not closed", json!({})) {
        Ok(r) => assert_eq!(r["format"], "svg"),
        Err(e) => assert!(!e.to_string().contains("importer"), "{e}"),
    }
    // Named, the importer opens an .svg.
    let r = open(&mut s, "c.svg", svg, json!({"importer": "plugin.test.svgtoo"})).unwrap();
    assert_eq!(r["format"], "plugin.test.svgtoo");
    // An SVG named as the importer's file is still an SVG.
    assert_eq!(open(&mut s, "d.tsv", svg, json!({})).unwrap()["format"], "svg");
    assert_eq!(fileio::format("svg").map(|f| f.id), Some("svg"));
    assert_eq!(fileio::open_extensions().iter().filter(|e| **e == "svg").count(), 1);
}

/// File › Open lists an importer's extensions only where it would open the file with the importer:
/// not those a built-in format reads, and not those it opens as a plug-in, a shortcut set, a
/// script, presets or a library. An importer with none of its own gets no filter.
#[test]
fn file_open_lists_only_the_extensions_it_opens_with_an_importer() {
    let mut s = Session::new();
    install(&mut s, &tpl::returning(&tpl::importer("test.claimed", r#"["tcl","ase","wasm","kys","jsx","vcstyles","svg"]"#, "{}"), RECT));
    install(&mut s, &tpl::returning(&tpl::importer("test.claimedonly", r#"["ase","gpl"]"#, "{}"), RECT));
    for ext in ["ase", "gpl", "wasm", "kys", "jsx", "vcstyles"] {
        assert!(fileio::opens_as_other(ext), "{ext}");
        assert!(!fileio::open_extensions().contains(&ext), "All readable files: {ext}");
    }
    assert!(!fileio::opens_as_other("tcl") && !fileio::opens_as_other("svg"));
    assert!(fileio::open_extensions().contains(&"tcl"));
    let filters: Vec<_> = fileio::open_filters().collect();
    assert!(filters.contains(&("Test test.claimed", &["tcl"][..])), "{filters:?}");
    assert!(!filters.iter().any(|(label, _)| *label == "Test test.claimedonly"), "{filters:?}");
    assert!(filters.contains(&("Composite fonts", vectorcraft_engine::cmd::compositefonts::EXTS)), "{filters:?}");
    s.execute("plugin.remove", &json!({"id": "test.claimed"})).unwrap();
    s.execute("plugin.remove", &json!({"id": "test.claimedonly"})).unwrap();
}

/// Several importers list one extension: the one whose id sorts first opens the file, and
/// `importer` picks another (by its format id or its id).
#[test]
fn the_first_importer_by_id_opens_a_shared_extension() {
    let mut s = Session::new();
    install(&mut s, &tpl::returning(&tpl::importer("test.shared.b", r#"["tsh"]"#, "{}"), LINE));
    install(&mut s, &tpl::returning(&tpl::importer("test.shared.a", r#"["tsh"]"#, "{}"), RECT));
    assert_eq!(open(&mut s, "x.tsh", b"?", json!({})).unwrap()["format"], "plugin.test.shared.a");
    assert_eq!(open(&mut s, "x.tsh", b"?", json!({"importer": "plugin.test.shared.b"})).unwrap()["format"], "plugin.test.shared.b");
    assert_eq!(open(&mut s, "x.tsh", b"?", json!({"importer": "test.shared.b"})).unwrap()["format"], "plugin.test.shared.b");
}

#[test]
fn importer_names_that_do_not_fit_are_errors() {
    let mut s = Session::new();
    echo(&mut s, "test.names", r#"["tsn"]"#);
    install(&mut s, &tpl::returning(&tpl::manifest("test.names.filter", "filter", "{}"), r#"{"objects":[]}"#));
    install(&mut s, &tpl::returning(&tpl::exporter("test.names.exporter", r#"["tsx"]"#, "{}"), r#"{"dataBase64":""}"#));
    let cases = [
        (json!({"importer": "plugin.test.names"}), "x.png", "opens .tsn files, not `x.png`"),
        (json!({"importer": "plugin.test.names.filter"}), "x.tsn", "is a plug-in of kind `filter`, not an importer"),
        (json!({"importer": "plugin.test.names.exporter"}), "x.tsn", "is a plug-in of kind `exporter`, not an importer"),
        (json!({"importer": "plugin.test.nothere"}), "x.tsn", "test.nothere"),
        (json!({"importer": 3}), "x.tsn", "`importer` must be a plug-in format id"),
    ];
    for (extra, name, why) in cases {
        let e = open(&mut s, name, RECT.as_bytes(), extra.clone()).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{extra}: {e}");
        assert!(e.to_string().contains(why), "{extra}: {e}");
    }
    assert!(s.documents().is_empty());
    // Without an importer, a file nothing reads stays unsupported.
    assert!(open(&mut s, "x.tsunknown", RECT.as_bytes(), json!({})).is_err());
    // An importer is not a filter.
    s.execute("file.new", &json!({})).unwrap();
    let e = s.execute("plugin.run", &json!({"id": "test.names"})).unwrap_err();
    assert!(e.to_string().contains("document.open"), "{e}");
}

/// The importer is given its parameters, checked against its manifest, and `_context` with the
/// file's extension and name.
#[test]
fn the_importer_is_given_its_parameters_and_the_file_s_name() {
    let mut s = Session::new();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    let needles = [r#""mode":"import""#, r#""extension":"tsc""#, r#""name":"Shapes""#, r#""density":10.0"#];
    for (i, needle) in needles.into_iter().enumerate() {
        // RECT when the parameters hold the needle; the file, which isn't JSON, otherwise.
        let wasm = wat::parse_str(tpl::when_given(&tpl::importer("test.params", r#"["tsc"]"#, params), Given::Params, needle, RECT)).unwrap();
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "replace": i > 0})).unwrap();
        let r = open(&mut s, "Shapes.tsc", b"not json", json!({"params": {"density": 12}}));
        assert_eq!(r.map(|r| r["format"].clone()).map_err(|e| e.to_string()), Ok(json!("plugin.test.params")), "{needle}");
    }
    let before = s.documents().len();
    let e = open(&mut s, "Shapes.tsc", b"", json!({"params": {"density": "lots"}})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
    assert_eq!(s.documents().len(), before);
}

#[test]
fn failures_open_nothing() {
    let mut s = Session::new();
    let far = r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[9000000,0]}]}]}}]}"#;
    let bad_warnings = LINE.replace("]}]}}]}", r#"]}]}}],"warnings":"no"}"#);
    let cases = [
        ("test.fail.declines", r#"{"error":"Not a drawing."}"#, "Not a drawing."),
        ("test.fail.notjson", "not json", "JSON"),
        ("test.fail.empty", r#"{"objects":[]}"#, "the importer returned no objects"),
        ("test.fail.key", r#"{"objects":[],"layers":[]}"#, "an importer returns `objects` and `warnings`, not `layers`"),
        ("test.fail.id", r#"{"objects":[{"id":3}]}"#, "is not the id of an input object"),
        ("test.fail.far", far, "4000000"),
        ("test.fail.warnings", bad_warnings.as_str(), "`warnings` must be a list of strings"),
    ];
    for (id, output, why) in cases {
        install(&mut s, &tpl::returning(&tpl::importer(id, r#"["tsf"]"#, "{}"), output));
        let e = open(&mut s, "x.tsf", b"?", json!({"importer": format!("plugin.{id}")})).unwrap_err();
        assert!(e.to_string().contains(why), "{id}: {e}");
        assert!(e.to_string().contains("x.tsf"), "{id}: names the file: {e}");
    }
    install(&mut s, &tpl::module(&tpl::importer("test.fail.traps", r#"["tsf"]"#, "{}"), "(unreachable)", "", 1));
    assert!(open(&mut s, "x.tsf", b"?", json!({"importer": "plugin.test.fail.traps"})).is_err());
    assert!(s.documents().is_empty(), "nothing opened");

    // A document that is already open keeps its tab, content and undo and redo steps.
    install(&mut s, &tpl::module(&tpl::importer("test.fail.spins", r#"["tsf"]"#, "{}"), SPINS, "", 1));
    let before = open_edited(&mut s, "test.fail.opens");
    for id in cases.iter().map(|c| c.0).chain(["test.fail.traps"]) {
        assert!(open(&mut s, "x.tsf", b"?", json!({"importer": format!("plugin.{id}")})).is_err(), "{id}");
        assert_eq!(state(&mut s), before, "{id}");
    }
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || open(&mut s, "x.tsf", b"?", json!({"importer": "plugin.test.fail.spins"})));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!(state(&mut s), before, "canceled");
}

/// Cancel stops an importer between two slices of its budget, and nothing opens: here one that
/// never returns, run with a token canceled beforehand.
#[test]
fn a_canceled_open_stops_the_importer() {
    let mut s = Session::new();
    install(&mut s, &tpl::module(&tpl::importer("test.spins", r#"["tss"]"#, "{}"), SPINS, "", 1));
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || open(&mut s, "x.tss", b"?", json!({})));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert!(s.documents().is_empty());
}

/// An importer that never returns, held to a small import budget, ends at its instruction limit
/// or at its time limit (0 ms, which has passed at the first check between two slices of the
/// budget). The open reports the limit, opens no document, and leaves an open document's content
/// and undo and redo steps as they were.
#[test]
fn an_importer_past_its_budget_opens_nothing() {
    let cases = [
        ("test.budget.fuel", Limits { export_fuel: 1_000_000, ..Limits::default() }, "instruction budget"),
        ("test.budget.time", Limits { export_fuel: 1_000_000_000, export_wall_time_ms: 0, ..Limits::default() }, "time budget"),
    ];
    for (id, limits, _) in &cases {
        let wasm = wat::parse_str(tpl::module(&tpl::importer(id, r#"["tbg"]"#, "{}"), SPINS, "", 1)).unwrap();
        vectorcraft_plugins::registry::install(Plugin::load(&wasm, limits.clone()).unwrap());
    }
    let mut s = Session::new();
    for (id, _, why) in &cases {
        let e = open(&mut s, "x.tbg", b"?", json!({"importer": format!("plugin.{id}")})).unwrap_err();
        assert!(e.to_string().contains(why) && e.to_string().contains("x.tbg"), "{id}: {e}");
        assert!(s.documents().is_empty(), "{id}");
    }
    let before = open_edited(&mut s, "test.budget.opens");
    for (id, _, why) in &cases {
        let e = open(&mut s, "x.tbg", b"?", json!({"importer": format!("plugin.{id}")})).unwrap_err();
        assert!(e.to_string().contains(why), "{id}: {e}");
        assert_eq!(state(&mut s), before, "{id}");
    }
    for (id, _, _) in &cases {
        vectorcraft_plugins::registry::remove(id);
    }
}

/// A file past the importer's input limit is not read past it, and doesn't open: an open document
/// keeps its content and its undo and redo steps. Given as data, it is refused the same way.
#[test]
fn a_file_past_the_input_limit_is_refused() {
    let dir = tmp_dir("large");
    let mut s = Session::new();
    echo(&mut s, "test.large", r#"["tsb"]"#);
    let path = dir.join("big.tsb");
    let max = vectorcraft_plugins::import::MAX_INPUT_BYTES;
    std::fs::File::create(&path).unwrap().set_len(max as u64 + 1).unwrap();
    let e = s.execute("document.open", &json!({"path": path_of(&path)})).unwrap_err();
    assert!(e.to_string().contains("larger than"), "{e}");
    assert!(e.to_string().contains("big.tsb"), "{e}");
    assert!(s.documents().is_empty());

    let before = open_edited(&mut s, "test.large.opens");
    let e = s.execute("document.open", &json!({"path": path_of(&path)})).unwrap_err();
    assert!(e.to_string().contains("larger than"), "{e}");
    assert_eq!(state(&mut s), before, "by path");
    let e = open(&mut s, "big.tsb", &vec![0; max + 1], json!({})).unwrap_err();
    assert!(e.to_string().contains("larger than"), "{e}");
    assert_eq!(state(&mut s), before, "as data");
    // The size is checked before the text is decoded: this text is not base64 at all.
    let not_base64 = "!".repeat((max + 1) / 3 * 4 + 8);
    for extra in [json!({}), json!({"importer": "plugin.test.large"})] {
        let mut p = json!({"name": "big.tsb", "dataBase64": not_base64});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let e = s.execute("document.open", &p).unwrap_err();
        assert!(e.to_string().contains("larger than"), "{e}");
        assert_eq!(state(&mut s), before, "before decoding");
    }
    // White space is not part of the file's size.
    let spaced = format!("QUJD{}", " ".repeat(max / 3 * 4 + 8));
    if let Err(e) = s.execute("document.open", &json!({"name": "small.tsb", "dataBase64": spaced})) {
        assert!(!e.to_string().contains("larger than"), "{e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file past the importer's input limit whose start shows a built-in format opens as that format,
/// whatever its extension; named, the importer's limit applies to it.
#[test]
fn a_built_in_file_past_the_input_limit_still_opens() {
    let dir = tmp_dir("largebuiltin");
    let mut s = Session::new();
    echo(&mut s, "test.largepng", r#"["tsp"]"#);
    let path = dir.join("big.tsp");
    let mut png = std::io::Cursor::new(vec![]);
    image::RgbaImage::new(2, 2).write_to(&mut png, image::ImageFormat::Png).unwrap();
    std::fs::write(&path, png.get_ref()).unwrap();
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(vectorcraft_plugins::import::MAX_INPUT_BYTES as u64 + 1).unwrap();
    let e = s.execute("document.open", &json!({"path": path_of(&path), "importer": "plugin.test.largepng"})).unwrap_err();
    assert!(e.to_string().contains("larger than"), "{e}");
    assert!(s.documents().is_empty());
    let r = s.execute("document.open", &json!({"path": path_of(&path)})).unwrap();
    assert_eq!(r["format"], "png");
    let mut data = png.into_inner();
    data.resize((vectorcraft_plugins::import::MAX_INPUT_BYTES + 3) / 3 * 3, 0);
    let encoded = json!({"name": "big.tsp", "dataBase64": vectorcraft_format::base64_encode(&data)});
    assert_eq!(s.execute("document.open", &encoded).unwrap()["format"], "png");
    let _ = std::fs::remove_dir_all(&dir);
}
