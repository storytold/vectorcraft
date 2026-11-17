//! Shared command plug-ins: modules assembled from WebAssembly text at test time, bundled as
//! `.craftplugin` files and installed with `plugin.install {data}`.

use serde_json::{Value, json};

use super::craft_plugin;
use super::craft_plugin::bundle::Bundle;
use crate::Session;

/// A command module that makes `calls` (command id, params JSON) in order. When a call fails, the
/// run ends: with `stop_on_refusal`, it fails with the call's error (status 1); otherwise the error
/// is its output and it carries on. Then `tail` runs (`unreachable`, an endless loop…), and the
/// last reply is its output.
fn module(calls: &[(&str, &str)], stop_on_refusal: bool, tail: &str) -> Vec<u8> {
    let mut data = String::new();
    let mut body = String::new();
    for (i, (cmd, params)) in calls.iter().enumerate() {
        let (c, p) = (0x1000 + i * 0x100, 0x1080 + i * 0x100);
        let escaped = params.replace('"', "\\22");
        data.push_str(&format!("(data (i32.const {c}) \"{cmd}\") (data (i32.const {p}) \"{escaped}\")\n"));
        let on_error = if stop_on_refusal {
            "(call $set_output (i32.const 0x8000) (call $reply_read (i32.const 0x8000) (call $reply_len))) (return (i32.const 1))"
        } else {
            "(nop)"
        };
        body.push_str(&format!(
            "(if (i32.ne (call $call (i32.const {c}) (i32.const {}) (i32.const {p}) (i32.const {})) (i32.const 0)) (then {on_error}))\n",
            cmd.len(),
            params.len()
        ));
    }
    let wat = format!(
        r#"(module
  (import "craft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
  (import "craft" "reply_len" (func $reply_len (result i32)))
  (import "craft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
  (import "craft" "set_output" (func $set_output (param i32 i32)))
  (memory (export "memory") 2)
  {data}
  (func (export "craft_run") (param $n i32) (result i32)
    {body}
    {tail}
    (call $set_output (i32.const 0x8000) (call $reply_read (i32.const 0x8000) (call $reply_len)))
    (i32.const 0)))"#
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

/// The bundle of command plug-in `id` with `access` and `wasm`.
fn bundle(id: &str, access: &str, wasm: Vec<u8>) -> Vec<u8> {
    let manifest = format!(
        r#"{{"id": "{id}", "name": "Test {id}", "version": "1.0.0", "main": "t.wasm", "kind": "command",
            "apps": {{"vectorcraft": {{"api": 6}}}}, "permissions": {{"access": "{access}"}},
            "commands": [{{"id": "go", "label": "Go"}}]}}"#
    );
    Bundle { manifest, wasm, assets: vec![] }.encode()
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn install(s: &mut Session, bytes: &[u8]) -> Value {
    run(s, "plugin.install", json!({"data": vectorcraft_format::base64_encode(bytes), "name": "test.craftplugin"}))
}

/// A document with two rectangles and nothing selected.
fn document() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 200}));
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}));
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 50}));
    run(&mut s, "select.none", json!({}));
    s
}

/// The active document's layers and its undo steps.
fn state(s: &mut Session) -> (Value, usize) {
    let content = run(s, "document.inspect", json!({}))["layers"].take();
    (content, s.doc().unwrap().history.undo.len())
}

const SELECT_ALL: (&str, &str) = ("select.all", "{}");
const MOVE: (&str, &str) = ("object.move", r#"{"dx":5,"dy":0}"#);

#[test]
fn a_write_plugin_runs_real_commands_as_one_undo_step() {
    let mut s = document();
    let before = state(&mut s);
    install(&mut s, &bundle("test.shared.mover", "write", module(&[SELECT_ALL, MOVE], true, "")));
    let listed = run(&mut s, "plugin.list", json!({}));
    let entry = listed["plugins"].as_array().unwrap().iter().find(|p| p["id"] == "test.shared.mover").unwrap();
    assert_eq!((entry["kind"].as_str(), entry["stored"].as_bool()), (Some("command"), Some(false)));

    let out = run(&mut s, "plugin.run", json!({"id": "test.shared.mover", "command": "go"}));
    assert!(out["calls"].as_u64().is_some_and(|n| n >= 2), "both commands were called: {out}");
    let after = state(&mut s);
    let xs: Vec<&Value> = after.0[0]["children"].as_array().unwrap().iter().map(|c| &c["bounds"]["x"]).collect();
    assert_eq!(xs, [&json!(105.0), &json!(15.0)]);
    assert_eq!(after.1, before.1 + 1, "one undo step");
    assert_eq!(s.doc().unwrap().history.undo.last().map(|u| u.label.as_str()), Some("Test test.shared.mover"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(state(&mut s), before, "one undo puts both calls back");
}

#[test]
fn a_read_plugin_that_edits_is_put_back_and_refused() {
    let mut s = document();
    let before = state(&mut s);
    install(&mut s, &bundle("test.shared.reader", "read", module(&[MOVE], false, "")));
    run(&mut s, "select.all", json!({}));
    let before_run = state(&mut s);
    let out = run(&mut s, "plugin.run", json!({"id": "test.shared.reader"}));
    let said = out["output"].as_str().unwrap_or_default();
    assert!(said.contains("may only read it"), "{out}");
    assert_eq!(state(&mut s).0, before.0, "the move was put back");
    assert_eq!(state(&mut s).1, before_run.1, "and left no undo step");
}

#[test]
fn a_refused_command_is_refused_before_it_runs() {
    let mut s = document();
    for (cmd, params) in [("file.save", "{}"), ("plugin.list", "{}"), ("edit.undo", "{}"), ("document.close", "{}")] {
        let id = format!("test.shared.refused{}", cmd.len());
        install(&mut s, &bundle(&id, "write", module(&[(cmd, params)], false, "")));
        let out = run(&mut s, "plugin.run", json!({"id": id}));
        assert!(out["output"].as_str().unwrap_or_default().contains("isn't available to plugins"), "{cmd}: {out}");
    }
    // A call whose params name a file is refused too, whatever the command.
    install(&mut s, &bundle("test.shared.path", "write", module(&[("object.move", r#"{"dx":1,"path":"/tmp/x"}"#)], false, "")));
    let out = run(&mut s, "plugin.run", json!({"id": "test.shared.path"}));
    assert!(out["output"].as_str().unwrap_or_default().contains("isn't available to plugins"), "{out}");
}

#[test]
fn a_plugin_that_traps_or_runs_out_of_fuel_leaves_the_document_as_it_was() {
    for (id, tail) in [("test.shared.trap", "(unreachable)"), ("test.shared.loop", "(loop $l (br $l))")] {
        let mut s = document();
        let before = state(&mut s);
        install(&mut s, &bundle(id, "write", module(&[SELECT_ALL, MOVE], true, tail)));
        let e = s.execute("plugin.run", &json!({"id": id})).unwrap_err().to_string();
        assert!(e.contains(&format!("Test {id}")), "{e}");
        assert_eq!(state(&mut s), before, "{id}: nothing changed");
        assert!(s.doc().unwrap().selection.is_empty(), "{id}: the selection is back too");
    }
}

#[test]
fn a_call_that_fails_fails_the_run_and_changes_nothing() {
    let mut s = document();
    let before = state(&mut s);
    install(&mut s, &bundle("test.shared.half", "write", module(&[SELECT_ALL, MOVE, ("object.move", r#"{"dx":"x"}"#)], true, "")));
    assert!(s.execute("plugin.run", &json!({"id": "test.shared.half"})).is_err());
    assert_eq!(state(&mut s), before);
}

#[test]
fn settings_are_bounded() {
    let mut all = std::collections::BTreeMap::new();
    super::set_setting(&mut all, "p", "k", Some("1")).unwrap();
    assert!(super::set_setting(&mut all, "p", &"k".repeat(129), Some("1")).is_err());
    let big = format!("\"{}\"", "x".repeat(super::MAX_SETTINGS_BYTES));
    assert!(super::set_setting(&mut all, "p", "big", Some(&big)).is_err());
    super::set_setting(&mut all, "p", "k", None).unwrap();
    assert!(all.is_empty());
}

#[test]
fn a_manifest_for_another_app_or_without_apps_is_refused() {
    let mut s = Session::new();
    let manifest = r#"{"id": "test.other", "name": "Other", "version": "1.0.0", "main": "t.wasm", "apps": {"figcraft": {"api": 6}}}"#;
    let bytes = Bundle { manifest: manifest.into(), wasm: module(&[], false, ""), assets: vec![] }.encode();
    let e = s.execute("plugin.install", &json!({"data": vectorcraft_format::base64_encode(&bytes)})).unwrap_err().to_string();
    assert!(e.contains("written for figcraft"), "{e}");
}

#[test]
fn install_review_list_and_remove_through_the_store() {
    let dir = std::env::temp_dir().join(format!("vc-craftplugin-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.attach_plugin_store(dir.clone());
    let bytes = bundle("test.shared.stored", "write", module(&[], false, ""));
    let data = vectorcraft_format::base64_encode(&bytes);

    // Review shows what the confirmation shows and installs nothing.
    let shown = run(&mut s, "plugin.install", json!({"data": data, "review": true}));
    let sha = craft_plugin::bundle::sha256_hex(&bytes);
    assert_eq!((shown["kind"].as_str(), shown["sha256"].as_str(), shown["stored"].as_bool()), (Some("command"), Some(sha.as_str()), Some(true)));
    assert!(s.command_plugins().get("test.shared.stored").is_none());
    let e = s.execute("plugin.install", &json!({"data": data, "sha256": "0".repeat(64)})).unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    run(&mut s, "plugin.install", json!({"data": data, "sha256": sha}));

    // The store holds it, beside VectorCraft's own modules, and a new session reads it back.
    let folder = dir.join(craft_plugin::store::folder_name("test.shared.stored"));
    assert!(folder.join("source.json").is_file());
    let mut again = Session::new();
    again.attach_plugin_store(dir.clone());
    let listed = run(&mut again, "plugin.list", json!({}));
    assert!(listed["plugins"].as_array().unwrap().iter().any(|p| p["id"] == "test.shared.stored" && p["stored"] == json!(true)), "{listed}");

    assert_eq!(run(&mut again, "plugin.remove", json!({"id": "test.shared.stored"}))["removed"], json!("test.shared.stored"));
    assert!(!folder.exists());
    assert!(again.command_plugins().list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn catalogs_are_checked_and_kept() {
    let mut s = Session::new();
    assert!(s.execute("plugin.catalog.add", &json!({"url": "http://example.com/index.json"})).is_err());
    let url = "https://raw.githubusercontent.com/example/plugins/main/index.json";
    assert_eq!(run(&mut s, "plugin.catalog.add", json!({"url": url}))["catalogs"], json!([url]));
    // Without a network client, listing says so rather than failing silently.
    let e = s.execute("plugin.catalog.list", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no network access"), "{e}");
    assert_eq!(run(&mut s, "plugin.catalog.add", json!({"url": url, "remove": true}))["catalogs"], json!([]));
    assert_eq!(run(&mut s, "plugin.update", json!({}))["plugins"], json!([]));
}

#[test]
fn the_shared_catalog_commands_are_not_plugin_commands() {
    assert!(!crate::cmd::plugin::is_command_id("plugin.catalog.list"));
    assert!(crate::cmd::plugin::is_command_id("plugin.some.cmd"));
    assert!(crate::find_command("plugin.catalog.add").is_some());
}

#[test]
fn the_policy_keeps_vectorcraft_refusals() {
    let p = super::policy();
    for refused in ["file.open", "document.close", "plugin.run", "edit.undo", "clipboard.copy", "prefs.set", "script.run"] {
        assert!(p.refuses(refused), "{refused}");
    }
    assert!(!p.refuses("object.move"));
}
