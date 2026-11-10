//! A filter plug-in whose manifest's `commandAllowlist` names commands calls them with `vc_call`:
//! each call sees what the calls before it left, the calls and the filter's output are one undo
//! step, and a run that fails leaves the document as it was.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

/// The command imports and the strings the plug-ins pass: the ids "select.all" (0x2000, 10 bytes),
/// "object.move" (0x2010, 11), "no.such.cmd" (0x2020, 11), "effect.apply" (0x2030, 12) and
/// "select.globalEditOptions" (0x2040, 24), and the arguments `{"dx":5,"dy":0}` (0x2100, 15),
/// `{"dx":"x"}` (0x2200, 10), `{"effect":"plugin.test.call.fx"}` (0x2300, 32) and `{"size":true}`
/// (0x2400, 13).
const CALLS: &str = r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (import "env" "vc_call_result" (func $vc_call_result (param i32 i32) (result i32)))
  (data (i32.const 0x2000) "select.all")
  (data (i32.const 0x2010) "object.move")
  (data (i32.const 0x2020) "no.such.cmd")
  (data (i32.const 0x2030) "effect.apply")
  (data (i32.const 0x2040) "select.globalEditOptions")
  (data (i32.const 0x2100) "{\22dx\22:5,\22dy\22:0}")
  (data (i32.const 0x2200) "{\22dx\22:\22x\22}")
  (data (i32.const 0x2300) "{\22effect\22:\22plugin.test.call.fx\22}")
  (data (i32.const 0x2400) "{\22size\22:true}")"#;

/// Selects everything.
const SELECT_ALL: &str = "(drop (call $vc_call (i32.const 0x2000) (i32.const 10) (i32.const 0) (i32.const 0)))";
/// Moves the selection 5 to the right.
const MOVE: &str = "(drop (call $vc_call (i32.const 0x2010) (i32.const 11) (i32.const 0x2100) (i32.const 15)))";

const EMPTY: &str = r#"{"objects":[]}"#;

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// Installs filter `id` with `permissions` (a JSON list) and `allowlist` (a JSON list), whose
/// `vc_run` runs `first`, then returns `output` → what `plugin.install` returned, or why it refused.
fn install(s: &mut Session, id: &str, permissions: &str, allowlist: &str, first: &str, output: &str) -> Result<Value, String> {
    let manifest =
        format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{permissions},"commandAllowlist":{allowlist}}}"#);
    let wasm = wat::parse_str(tpl::with_imports(&manifest, CALLS, first, output)).unwrap();
    let grant: Value = serde_json::from_str(permissions).unwrap();
    let data = vectorcraft_format::base64_encode(&wasm);
    s.execute("plugin.install", &json!({"dataBase64": data, "grant": grant, "replace": true})).map_err(|e| e.to_string())
}

/// A filter `id` that may read and change the document and call `select.all` and `object.move`.
fn install_mover(s: &mut Session, id: &str, first: &str, output: &str) {
    install(s, id, r#"["doc.read","doc.write","commands"]"#, r#"["select.all","object.move"]"#, first, output).unwrap();
}

/// A document with two rectangles and nothing selected.
fn document() -> Session {
    let mut s = Session::new();
    s.allow_plugin_grants_from_requests();
    run(&mut s, "file.new", json!({"width": 400, "height": 200}));
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}));
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 50}));
    run(&mut s, "select.none", json!({}));
    s
}

/// The active document's layers, its undo and redo steps and how many objects are selected.
fn state(s: &mut Session) -> (Value, usize, usize, usize) {
    let content = run(s, "document.inspect", json!({}))["layers"].take();
    let d = s.doc().unwrap();
    (content, d.history.undo.len(), d.history.redo.len(), d.selection.len())
}

/// Runs filter `id` on nothing, so only its calls change the document.
fn run_plugin(s: &mut Session, id: &str) -> Result<Value, String> {
    s.execute("plugin.run", &json!({"id": id, "ids": []})).map_err(|e| e.to_string())
}

#[test]
fn calls_see_each_others_changes_and_make_one_undo_step() {
    let mut s = document();
    let before = state(&mut s);
    install_mover(&mut s, "test.call.mover", &[SELECT_ALL, MOVE].concat(), EMPTY);
    run_plugin(&mut s, "test.call.mover").unwrap();
    let after = state(&mut s);
    // `object.move` moved what `select.all` selected: both rectangles.
    let xs: Vec<&Value> = after.0[0]["children"].as_array().unwrap().iter().map(|c| &c["bounds"]["x"]).collect();
    assert_eq!(xs, [&json!(105.0), &json!(15.0)]);
    assert_eq!(after.1, before.1 + 1, "one undo step");
    assert_eq!(s.doc().unwrap().history.undo.last().map(|u| u.label.as_str()), Some("Test test.call.mover"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(state(&mut s).0, before.0);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(state(&mut s).0, after.0);
    // Without `select.all` nothing is selected, so `object.move` has nothing to move.
    run(&mut s, "select.none", json!({}));
    install_mover(&mut s, "test.call.mover", MOVE, EMPTY);
    assert!(run_plugin(&mut s, "test.call.mover").is_err());
    run(&mut s, "plugin.remove", json!({"id": "test.call.mover"}));
}

#[test]
fn a_run_that_fails_after_its_calls_changes_nothing() {
    let mut s = document();
    let before = state(&mut s);
    // A trap, an output that isn't JSON, and a command that fails, each after the move.
    let bad_move = "(drop (call $vc_call (i32.const 0x2010) (i32.const 11) (i32.const 0x2200) (i32.const 10)))";
    for (first, output) in
        [([SELECT_ALL, MOVE, "unreachable"].concat(), EMPTY), ([SELECT_ALL, MOVE].concat(), "nope"), ([SELECT_ALL, MOVE, bad_move].concat(), EMPTY)]
    {
        install_mover(&mut s, "test.call.fails", &first, output);
        assert!(run_plugin(&mut s, "test.call.fails").is_err(), "{first} {output}");
        assert_eq!(state(&mut s), before, "{first} {output}");
    }
    run(&mut s, "plugin.remove", json!({"id": "test.call.fails"}));
    // Session state a call changed outside the document goes back too: Global Edit's options.
    let options = "(drop (call $vc_call (i32.const 0x2040) (i32.const 24) (i32.const 0x2400) (i32.const 13))) unreachable";
    let allowlist = r#"["select.globalEditOptions"]"#;
    install(&mut s, "test.call.options", r#"["doc.read","commands"]"#, allowlist, options, EMPTY).unwrap();
    assert!(run_plugin(&mut s, "test.call.options").is_err());
    assert_eq!(run(&mut s, "select.globalEditOptions", json!({}))["size"], json!(false));
    run(&mut s, "plugin.remove", json!({"id": "test.call.options"}));
}

/// A command that changes state a failed run can't put back (the preferences, a tool's options, a
/// drag's state) is refused at install, by name.
#[test]
fn install_refuses_reviewed_denials() {
    let mut s = document();
    for refused in ["object.liquify", "object.pathfinder.options", "addon.pen.nextSegment"] {
        let allowlist = format!(r#"["select.all","{refused}"]"#);
        let e = install(&mut s, "test.call.reviewed", r#"["doc.read","commands"]"#, &allowlist, "", EMPTY).unwrap_err();
        assert!(e.contains(refused) && e.contains("may not call"), "{e}");
    }
}

#[test]
fn install_refuses_a_command_a_plugin_may_not_call_and_reports_unknown_ones() {
    let mut s = document();
    let first = "(drop (call $vc_call (i32.const 0x2020) (i32.const 11) (i32.const 0) (i32.const 0)))";
    for refused in ["document.save", "edit.undo", "plugin.run", "script.run"] {
        let allowlist = format!(r#"["select.all","{refused}"]"#);
        let e = install(&mut s, "test.call.refused", r#"["doc.read","commands"]"#, &allowlist, "", EMPTY).unwrap_err();
        assert!(e.contains(refused) && e.contains("may not call"), "{e}");
    }
    let r = install(&mut s, "test.call.unknown", r#"["doc.read","commands"]"#, r#"["select.all","no.such.cmd"]"#, first, EMPTY).unwrap();
    assert_eq!(r["unknownCommands"], json!(["no.such.cmd"]));
    let e = run_plugin(&mut s, "test.call.unknown").unwrap_err();
    assert!(e.contains("no.such.cmd"), "{e}");
    run(&mut s, "plugin.remove", json!({"id": "test.call.unknown"}));
}

#[test]
fn a_call_that_names_a_plugin_is_refused() {
    let mut s = document();
    let before = state(&mut s);
    let first = format!("{SELECT_ALL} (drop (call $vc_call (i32.const 0x2030) (i32.const 12) (i32.const 0x2300) (i32.const 32)))");
    install(&mut s, "test.call.fx", r#"["doc.read","doc.write","commands"]"#, r#"["select.all","effect.apply"]"#, &first, EMPTY).unwrap();
    let e = run_plugin(&mut s, "test.call.fx").unwrap_err();
    assert!(e.contains("may not call"), "{e}");
    assert_eq!(state(&mut s), before);
    run(&mut s, "plugin.remove", json!({"id": "test.call.fx"}));
}

#[test]
fn without_doc_write_a_call_that_edits_is_refused() {
    let mut s = document();
    let before = state(&mut s);
    let allowlist = r#"["select.all","object.move"]"#;
    install(&mut s, "test.call.reader", r#"["doc.read","commands"]"#, allowlist, &[SELECT_ALL, MOVE].concat(), EMPTY).unwrap();
    let e = run_plugin(&mut s, "test.call.reader").unwrap_err();
    assert!(e.contains("may not call"), "{e}");
    assert_eq!(state(&mut s), before);
    // Selecting changes no art, so it runs.
    install(&mut s, "test.call.reader", r#"["doc.read","commands"]"#, allowlist, SELECT_ALL, EMPTY).unwrap();
    run_plugin(&mut s, "test.call.reader").unwrap();
    run(&mut s, "plugin.remove", json!({"id": "test.call.reader"}));
}
