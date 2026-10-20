//! Exporter plug-ins (`"kind": "exporter"`, docs/plugins.md › Exporters) through the engine: the
//! formats they add to `document.formats` and `document.export`, the document and parameters they
//! are given, and what is written when they succeed, decline, misbehave or are cancelled.
//!
//! A test binary of its own: installed plug-ins are process-wide, and the unit tests that check the
//! exact list of formats would see these ones.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::{EngineError, Session};
use vectorcraft_plugins::wat::{self as tpl, Echo};

/// A document with one rectangle.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s
}

/// Installs the plug-in written in WAT `src` → its `plugin.list` entry.
fn install(s: &mut Session, src: &str) -> Value {
    let wasm = wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}"));
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap()
}

/// The file a `document.export` without a path gives back.
fn exported(r: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A fresh folder for files this test writes.
fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-plugin-export-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path_of(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn an_exporter_adds_a_write_only_format_while_it_is_installed() {
    let mut s = session();
    let entry = install(&mut s, &tpl::returning(&tpl::exporter("test.list", r#"["tsl","tsl2"]"#, "{}"), r#"{"dataBase64":""}"#));
    assert_eq!((&entry["kind"], &entry["format"], &entry["extensions"]), (&json!("exporter"), &json!("plugin.test.list"), &json!(["tsl", "tsl2"])));
    let formats = s.execute("document.formats", &json!({})).unwrap();
    let listed = formats["formats"].as_array().unwrap().iter().find(|f| f["id"] == "plugin.test.list").expect("listed");
    assert_eq!((&listed["label"], &listed["extensions"]), (&json!("Test test.list"), &json!(["tsl", "tsl2"])));
    assert_eq!((&listed["read"], &listed["write"], &listed["plugin"]), (&json!(false), &json!(true), &json!("test.list")));
    assert!(formats["writable"].as_array().unwrap().contains(&json!("plugin.test.list")));
    assert!(!formats["readable"].as_array().unwrap().contains(&json!("plugin.test.list")));
    assert!(fileio::export_extensions().contains(&"tsl"));
    assert_eq!(fileio::format(".TSL2").map(|f| f.id), Some("plugin.test.list"), "by any of its extensions, in any case");
    assert_eq!(fileio::format("plugin.test.list").map(|f| f.label), Some("Test test.list"));

    s.execute("plugin.remove", &json!({"id": "test.list"})).unwrap();
    let formats = s.execute("document.formats", &json!({})).unwrap();
    assert!(!formats["writable"].as_array().unwrap().contains(&json!("plugin.test.list")));
    assert!(fileio::format("tsl").is_none());
    let e = s.execute("document.export", &json!({"format": "plugin.test.list"})).unwrap_err();
    assert!(e.to_string().contains("unknown format"), "{e}");
}

/// The exporter is given the document as a native file's JSON: what `document.save` writes before
/// the data of large images, which stays behind (compact, where a save of a small document is
/// indented).
#[test]
fn the_exporter_is_given_the_native_file_s_json() {
    let mut s = session();
    install(&mut s, &tpl::echo_exporter(&tpl::exporter("test.input", r#"["tsi"]"#, "{}"), Echo::Input));
    // Noise, so the PNG is large enough to go after the JSON in a native file.
    let mut seed = 1_u32;
    let noise = image::RgbaImage::from_fn(64, 64, |_, _| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let [r, g, b, _] = seed.to_be_bytes();
        image::Rgba([r, g, b, 255])
    });
    let mut png = vec![];
    noise.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    assert!(png.len() >= vectorcraft_format::INLINE_MAX, "{} bytes", png.len());
    s.execute("file.place", &json!({"name": "noise.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();

    let r = s.execute("document.export", &json!({"format": "plugin.test.input"})).unwrap();
    assert_eq!(r["format"], "plugin.test.input");
    let given = exported(&r);
    let saved = exported(&s.execute("document.save", &json!({})).unwrap());
    assert!(saved.len() > given.len() + png.len(), "the image data follows the JSON in the saved file");
    let doc: Value = serde_json::from_slice(&given).unwrap();
    let saved_json: Value = serde_json::from_slice(vectorcraft_format::json_part(&saved)).unwrap();
    assert_eq!(doc, saved_json, "the saved file's JSON");
    assert!(!given.contains(&b'\n'), "without indentation");
    assert_eq!((&doc["format"], &doc["version"]), (&json!("vectorcraft"), &json!(vectorcraft_format::VERSION)));
    let images = doc["images"].as_object().unwrap();
    let image = images.values().next().unwrap();
    assert_eq!(images.len(), 1);
    assert!(image.get("blob").is_some() && image.get("data").is_none(), "{image}");
}

#[test]
fn the_exporter_is_given_its_parameters_and_the_file_it_writes() {
    let dir = tmp_dir("params");
    let mut s = session();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4},"mode":{"type":"choice","options":["fast","fine"],"default":"fast"}}"#;
    install(&mut s, &tpl::echo_exporter(&tpl::exporter("test.params", r#"["tsp","tsq"]"#, params), Echo::Params));
    let sent = |r: &Value| -> Value { serde_json::from_slice(&exported(r)).unwrap() };

    // The defaults, and without a path its first extension and the document's name.
    let r = s.execute("document.export", &json!({"format": "plugin.test.params"})).unwrap();
    let got = sent(&r);
    assert_eq!((&got["density"], &got["mode"]), (&json!(4.0), &json!("fast")));
    assert_eq!((&got["_context"]["mode"], &got["_context"]["extension"]), (&json!("export"), &json!("tsp")));
    let title = s.doc().unwrap().doc.title.clone();
    assert_eq!(got["_context"]["name"], json!(title));

    // Given ones, clamped as a filter's are, and the path's extension when the exporter lists it.
    let path = dir.join("Poster.tsq");
    let r = s.execute("document.export", &json!({"path": path_of(&path), "params": {"density": 12, "mode": "fine"}})).unwrap();
    assert_eq!(r["format"], "plugin.test.params");
    let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written, json!({"density": 10.0, "mode": "fine", "_context": {"mode": "export", "extension": "tsq", "name": "Poster"}}));

    // Bad parameters fail before the plug-in runs, and nothing is written.
    let bad = dir.join("Bad.tsp");
    let e = s.execute("document.export", &json!({"path": path_of(&bad), "params": {"mode": "slow"}})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
    assert!(!bad.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Only the selected objects, with `selectedOnly`, as for the built-in formats.
#[test]
fn selected_only_hands_over_the_selection() {
    let mut s = session();
    install(&mut s, &tpl::echo_exporter(&tpl::exporter("test.selection", r#"["tse"]"#, "{}"), Echo::Input));
    let ellipse = s.execute("shape.ellipse", &json!({"x": 200, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [ellipse]})).unwrap();
    let count = |r: &Value| String::from_utf8(exported(r)).unwrap().matches("\"anchors\"").count();
    let all = count(&s.execute("document.export", &json!({"format": "plugin.test.selection"})).unwrap());
    let selected = count(&s.execute("document.export", &json!({"format": "plugin.test.selection", "selectedOnly": true})).unwrap());
    assert!(selected >= 1 && selected < all, "{selected} of {all} paths");
}

/// A built-in format keeps its extension: an exporter that lists `svg` too writes it only when
/// asked for by its format id.
#[test]
fn built_in_formats_keep_their_extensions() {
    let dir = tmp_dir("builtin");
    let mut s = session();
    install(&mut s, &tpl::echo_exporter(&tpl::exporter("test.svgtoo", r#"["tsv","svg"]"#, "{}"), Echo::Params));
    assert_eq!(fileio::format("svg").map(|f| f.id), Some("svg"));
    let svg = dir.join("a.svg");
    s.execute("document.export", &json!({"path": path_of(&svg)})).unwrap();
    assert!(String::from_utf8(std::fs::read(&svg).unwrap()).unwrap().contains("<svg"), "the built-in SVG");
    s.execute("document.export", &json!({"path": path_of(&svg), "format": "plugin.test.svgtoo"})).unwrap();
    let sent: Value = serde_json::from_slice(&std::fs::read(&svg).unwrap()).unwrap();
    assert_eq!(sent["_context"]["extension"], "svg", "an extension it lists, asked for by its format id");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn declines_and_bad_output_write_nothing_and_warnings_come_back() {
    let dir = tmp_dir("output");
    let mut s = session();
    let cases = [
        ("test.declines", r#"{"error":"Nothing to plot."}"#, "Nothing to plot."),
        ("test.badbase64", r#"{"dataBase64":"@@@"}"#, "`dataBase64` is not valid base64"),
        ("test.nodata", r#"{"warnings":[]}"#, "dataBase64"),
    ];
    for (id, output, why) in cases {
        install(&mut s, &tpl::returning(&tpl::exporter(id, r#"["tsd"]"#, "{}"), output));
        let path = dir.join(format!("{id}.tsd"));
        let e = s.execute("document.export", &json!({"path": path_of(&path), "format": format!("plugin.{id}")})).unwrap_err();
        assert!(e.to_string().contains(why), "{id}: {e}");
        assert!(!path.exists(), "{id}: nothing written");
    }
    install(
        &mut s,
        &tpl::returning(&tpl::exporter("test.warns", r#"["tsw"]"#, "{}"), r#"{"dataBase64":"SGk=","warnings":["3 colours over the limit"]}"#),
    );
    let path = dir.join("w.tsw");
    let r = s.execute("document.export", &json!({"path": path_of(&path)})).unwrap();
    assert_eq!((r["bytes"].as_u64(), &r["warnings"]), (Some(2), &json!(["3 colours over the limit"])));
    assert_eq!(std::fs::read(&path).unwrap(), b"Hi");
    // An exporter is not a filter.
    let e = s.execute("plugin.run", &json!({"id": "test.warns"})).unwrap_err();
    assert!(e.to_string().contains("document.export"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Background Export's Cancel stops an exporter between two slices of its budget: here one that
/// never returns, cancelled from another thread once it is running.
#[test]
fn a_cancelled_export_stops_the_exporter() {
    let mut s = session();
    install(&mut s, &tpl::module(&tpl::exporter("test.spins", r#"["tss"]"#, "{}"), "(loop $l (br $l)) (i64.const 0)", "", 1));
    let token = vectorcraft_doc::cancel::CancelToken::new();
    let cancel = token.clone();
    let wait = Duration::from_millis(300);
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(wait);
        cancel.cancel();
    });
    let t = Instant::now();
    let r = vectorcraft_doc::cancel::scope(&token, None, || s.execute("document.export", &json!({"format": "plugin.test.spins"})));
    let took = t.elapsed();
    canceller.join().unwrap();
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert!(took >= wait && took < Duration::from_secs(10), "stopped while running, not before: {took:?}");
}
