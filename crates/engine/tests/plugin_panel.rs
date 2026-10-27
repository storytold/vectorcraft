//! Panel plug-ins (`"kind": "panel"`) through the engine: `plugin.panel.event` hands a panel its
//! event, what it kept, its values and the selection, keeps what it returns, and runs its commands
//! as one undo step, or none of them when one is refused or fails.
//!
//! A test binary of its own: installed plug-ins are process-wide.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::{EngineError, Session};
use vectorcraft_plugins::wat::{self as tpl, Given};

/// A document with one rectangle, selected.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s
}

/// The paths of the active document (each has its anchors).
fn paths(s: &Session) -> usize {
    String::from_utf8(vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap().matches("\"anchors\"").count()
}

/// Installs the plug-in written in WAT `src`.
fn install(s: &mut Session, src: &str) {
    let wasm = wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}"));
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
}

/// A panel `id` that answers every event with `output`.
fn answering(s: &mut Session, id: &str, output: &Value) {
    install(s, &tpl::returning(&tpl::manifest(id, "panel", "{}"), &output.to_string()));
}

/// A panel `id` that answers `{"panel": {}}` only when what it is given (`given`) holds `needle`.
fn expecting(s: &mut Session, id: &str, given: Given, needle: &str) {
    install(s, &tpl::when_given(&tpl::manifest(id, "panel", "{}"), given, needle, r#"{"panel":{}}"#));
}

fn event(s: &mut Session, id: &str, event: Value, values: Value) -> Result<Value, EngineError> {
    s.execute("plugin.panel.event", &json!({"id": id, "event": event, "values": values}))
}

fn click(s: &mut Session, id: &str) -> Result<Value, EngineError> {
    event(s, id, json!({"type": "click", "target": "add"}), Value::Null)
}

fn rectangle(x: i32) -> Value {
    json!({"command": "shape.rectangle", "params": {"x": x, "y": 100, "width": 20, "height": 20}})
}

/// The panel's answer comes back as it is, and its commands run as one undo step.
#[test]
fn a_panel_answers_with_its_panel_and_commands_that_run_as_one_step() {
    let mut s = session();
    let panel = json!({"title": "Shapes", "items": [{"type": "button", "id": "add", "label": "Add"}]});
    answering(&mut s, "test.shapes", &json!({"panel": panel, "commands": [rectangle(150), rectangle(200)]}));
    let before = paths(&s);
    let r = click(&mut s, "test.shapes").unwrap();
    assert_eq!((&r["plugin"], &r["panel"]), (&json!("test.shapes"), &panel));
    assert_eq!(r["results"].as_array().map(Vec::len), Some(2), "{r}");
    assert_eq!(paths(&s), before + 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paths(&s), before, "one undo step takes both back");
    // The batch is journaled, not the event: replaying the journal needs no plug-in.
    assert!(s.journal.iter().any(|(id, _)| id == "command.batch"));
    assert!(!s.journal.iter().any(|(id, _)| id == "plugin.panel.event"));
}

/// The panel is given its event, the values of its fields, the selected paths and its `_context`;
/// a test panel that answers only when it finds a text in what it is given shows each.
#[test]
fn a_panel_is_given_its_event_values_and_selection() {
    let mut s = session();
    for (given, needle) in [
        (Given::Input, r#""type":"click""#),
        (Given::Input, r#""target":"add""#),
        (Given::Input, r#""width":40"#),
        (Given::Input, r#""type":"path""#),
        (Given::Params, r#""mode":"panel""#),
    ] {
        expecting(&mut s, "test.given", given, needle);
        let r = event(&mut s, "test.given", json!({"type": "click", "target": "add"}), json!({"width": 40}));
        assert!(r.is_ok(), "{needle}: {r:?}");
    }
    // Without it, the panel hands its input back, which is no answer.
    expecting(&mut s, "test.given", Given::Input, "not there");
    let e = click(&mut s, "test.given").unwrap_err();
    assert!(e.to_string().contains("panel"), "{e}");
}

/// What a panel keeps comes back with its next event; `null` forgets it, and so does removing the
/// plug-in.
#[test]
fn a_panel_keeps_its_state_between_events() {
    let mut s = session();
    answering(&mut s, "test.state", &json!({"panel": {}, "state": {"count": 1}}));
    click(&mut s, "test.state").unwrap();
    expecting(&mut s, "test.state", Given::Input, r#""state":{"count":1}"#);
    click(&mut s, "test.state").unwrap();
    // No `state` keeps it.
    click(&mut s, "test.state").unwrap();
    answering(&mut s, "test.state", &json!({"panel": {}, "state": null}));
    click(&mut s, "test.state").unwrap();
    expecting(&mut s, "test.state", Given::Input, r#""state":null"#);
    click(&mut s, "test.state").unwrap();

    answering(&mut s, "test.state", &json!({"panel": {}, "state": {"count": 2}}));
    click(&mut s, "test.state").unwrap();
    s.execute("plugin.remove", &json!({"id": "test.state"})).unwrap();
    expecting(&mut s, "test.state", Given::Input, r#""state":null"#);
    click(&mut s, "test.state").unwrap();
}

/// One command a panel may not run refuses them all before any runs, and the panel keeps what it
/// kept.
#[test]
fn a_refused_command_runs_none() {
    let mut s = session();
    for refused in ["file.save", "edit.undo", "plugin.remove", "prefs.set", "stroke.widthProfile.reset", "object.pathfinder.options", "command.batch"]
    {
        let commands = json!([rectangle(150), {"command": refused}]);
        answering(&mut s, "test.refused", &json!({"panel": {}, "state": {"ran": true}, "commands": commands}));
        let before = paths(&s);
        let e = click(&mut s, "test.refused").unwrap_err();
        assert!(e.to_string().contains(&format!("may not run `{refused}`")), "{refused}: {e}");
        assert_eq!(paths(&s), before, "{refused}");
    }
    expecting(&mut s, "test.refused", Given::Input, r#""state":null"#);
    click(&mut s, "test.refused").unwrap();
}

/// A command that fails rolls back the ones before it, and the panel keeps what it kept.
#[test]
fn a_failing_command_rolls_the_event_back() {
    let mut s = session();
    let bad = json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": "wide", "height": 20}});
    answering(&mut s, "test.fails", &json!({"panel": {}, "state": {"ran": true}, "commands": [rectangle(150), bad]}));
    let before = paths(&s);
    let undo = s.doc().unwrap().history.undo.len();
    let e = click(&mut s, "test.fails").unwrap_err();
    assert!(e.to_string().contains("batch step 1"), "{e}");
    assert_eq!(paths(&s), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step is left behind");
    expecting(&mut s, "test.fails", Given::Input, r#""state":null"#);
    click(&mut s, "test.fails").unwrap();
}

/// Without a document a panel still answers; commands then have nothing to run in.
#[test]
fn without_a_document_a_panel_answers_but_runs_nothing() {
    let mut s = Session::new();
    answering(&mut s, "test.nodoc", &json!({"panel": {"items": []}}));
    let r = event(&mut s, "test.nodoc", json!({"type": "open"}), Value::Null).unwrap();
    assert_eq!(r["panel"], json!({"items": []}));
    answering(&mut s, "test.nodoc", &json!({"panel": {}, "commands": [rectangle(0)]}));
    assert!(click(&mut s, "test.nodoc").is_err());
}

/// A panel is no filter, a filter no panel, and an event must say what happened.
#[test]
fn panels_and_events_are_checked() {
    let mut s = session();
    answering(&mut s, "test.kind", &json!({"panel": {}}));
    let e = s.execute("plugin.run", &json!({"id": "test.kind"})).unwrap_err();
    assert!(e.to_string().contains("plugin.panel.event"), "{e}");
    install(&mut s, &tpl::returning(&tpl::manifest("test.filter", "filter", "{}"), r#"{"objects":[]}"#));
    let e = click(&mut s, "test.filter").unwrap_err();
    assert!(e.to_string().contains("not a panel"), "{e}");
    for bad in [json!(null), json!({"type": "drag"}), json!({"type": "click"})] {
        let e = event(&mut s, "test.kind", bad.clone(), Value::Null).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{bad}: {e}");
    }
    let e = event(&mut s, "test.kind", json!({"type": "open"}), json!([1])).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
    // A decline is the plug-in's message.
    answering(&mut s, "test.kind", &json!({"error": "Select a path first."}));
    let e = click(&mut s, "test.kind").unwrap_err();
    assert!(e.to_string().contains("Select a path first."), "{e}");
}
