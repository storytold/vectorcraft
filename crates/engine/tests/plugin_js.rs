//! JavaScript filter plug-ins through the engine: a `.vcplugin` manifest and the `.js` file it
//! names install with `plugin.install {path}`, run with `plugin.run` as one undo step, and a run
//! that fails or exhausts its budget leaves the document as it was.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_engine::Session;
use vectorcraft_testkit::fixtures::{exec, rect, select, session};

/// Paints every input object's first fill blue.
const BLUE: &str = "function run(input, params) {\n\
    return {objects: input.objects.map(function (o) {\n\
        return {id: o.id, fills: [{type: 'solid', color: {model: 'rgb', r: 0, g: 0, b: 1}}]};\n\
    })};\n\
}\n";

/// An empty folder for one test, with anything an earlier run left there removed.
fn folder(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-plugin-js-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Writes the manifest of filter `id`, whose code is `tint.js`, with `extra` keys, and `script` as
/// that file → the manifest's path.
fn package(dir: &Path, id: &str, extra: Value, script: Option<&str>) -> String {
    let mut m = json!({"id": id, "name": "Tint", "kind": "filter", "runtime": "js", "script": "tint.js"});
    if let (Some(m), Value::Object(extra)) = (m.as_object_mut(), extra) {
        m.extend(extra);
    }
    let path = dir.join(format!("{id}.vcplugin"));
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    if let Some(script) = script {
        std::fs::write(dir.join("tint.js"), script).unwrap();
    }
    path.display().to_string()
}

/// The active document's layers and its undo steps.
fn state(s: &mut Session) -> (Value, usize) {
    let layers = exec(s, "document.inspect", json!({}))["layers"].take();
    (layers, s.doc().unwrap().history.undo.len())
}

fn fill(s: &Session, id: vectorcraft_doc::NodeId) -> Option<Color> {
    s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color()
}

#[test]
fn a_javascript_filter_changes_a_fill_as_one_undo_step() {
    let dir = folder("blue");
    let mut s = session();
    let path = package(&dir, "test.js.engine.blue", json!({}), Some(BLUE));
    let info = exec(&mut s, "plugin.install", json!({"path": path, "replace": true}));
    assert_eq!((info["runtime"].as_str(), info["script"].as_str(), info["kind"].as_str()), (Some("js"), Some("tint.js"), Some("filter")));
    let id = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    exec(&mut s, "paint.setFill", json!({"ids": [id.0], "color": "#ff0000"}));
    select(&mut s, &[id]);
    let before = state(&mut s);
    let r = exec(&mut s, "plugin.run", json!({"id": "test.js.engine.blue"}));
    assert_eq!(r["ids"], json!([id.0]));
    assert_eq!(fill(&s, id).map(|c| c.to_rgb_uncalibrated()), Some([0.0, 0.0, 1.0]));
    assert_eq!(state(&mut s).1, before.1 + 1, "one undo step");
    exec(&mut s, "edit.undo", json!({}));
    assert_eq!(state(&mut s), before);
    exec(&mut s, "plugin.remove", json!({"id": "test.js.engine.blue"}));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// An endless loop ends with the plug-in's instruction budget, the error names the plug-in, and the
/// document and its history stay as they were.
#[test]
fn an_endless_loop_stops_on_its_budget_and_changes_nothing() {
    let dir = folder("loop");
    let mut s = session();
    let path = package(&dir, "test.js.engine.loop", json!({}), Some("function run(input) {\n  while (true) {}\n}\n"));
    exec(&mut s, "plugin.install", json!({"path": path, "replace": true}));
    let id = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    select(&mut s, &[id]);
    let before = state(&mut s);
    let e = s.execute("plugin.run", &json!({"id": "test.js.engine.loop"})).unwrap_err().to_string();
    assert!(e.contains("instruction budget") && e.contains("test.js.engine.loop, tint.js line 2"), "{e}");
    assert_eq!(state(&mut s), before);
    exec(&mut s, "plugin.remove", json!({"id": "test.js.engine.loop"}));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A filter that reads `File` throws a `ReferenceError`: the realm has none of the script runner's
/// file access. Nothing changes.
#[test]
fn a_javascript_filter_has_no_file_access() {
    let dir = folder("file");
    let mut s = session();
    let path = package(&dir, "test.js.engine.file", json!({}), Some("function run(input) {\n  return new File('/etc/passwd').read();\n}\n"));
    exec(&mut s, "plugin.install", json!({"path": path, "replace": true}));
    let id = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    select(&mut s, &[id]);
    let before = state(&mut s);
    let e = s.execute("plugin.run", &json!({"id": "test.js.engine.file"})).unwrap_err().to_string();
    assert!(e.contains("ReferenceError") && e.contains("File") && e.contains("test.js.engine.file"), "{e}");
    assert_eq!(state(&mut s), before);
    exec(&mut s, "plugin.remove", json!({"id": "test.js.engine.file"}));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A manifest with `"runtime": "js"` and no `script` is refused at install, the error names the
/// key, and nothing is installed.
#[test]
fn a_manifest_without_its_script_is_refused_at_install() {
    let dir = folder("noscript");
    let mut s = session();
    let path = package(&dir, "test.js.engine.noscript", json!({"script": null}), None);
    let mut m: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m.as_object_mut().unwrap().remove("script");
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    for cmd in ["plugin.inspect", "plugin.install"] {
        let e = s.execute(cmd, &json!({"path": path})).unwrap_err().to_string();
        assert!(e.contains("`runtime` \"js\" needs `script`"), "{cmd}: {e}");
    }
    assert!(vectorcraft_plugins::registry::get("test.js.engine.noscript").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}
