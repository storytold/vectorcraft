//! Panel plug-ins (`"kind": "panel"`) through the engine: `plugin.panel.event` hands a panel its
//! event, what it kept, its values and the selection, keeps what it returns, and runs its commands
//! as one undo step, or none of them when one is refused or fails.
//!
//! A test binary of its own: installed plug-ins are process-wide.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::{EngineError, Session};
use vectorcraft_plugins::panel::MAX_FILE;
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

/// A file picked in the panel comes with its name and its bytes, as plain base64, never its folder;
/// bytes that aren't base64, or more than a panel takes, are refused before the plug-in runs.
#[test]
fn a_panel_is_given_the_file_picked() {
    let mut s = session();
    let csv = vectorcraft_format::base64_encode(b"name,Title\nOne,Alpha\n");
    let picked = |data: &str| json!({"type": "file", "target": "pick", "name": "data.csv", "dataBase64": data});
    let bytes = format!(r#""dataBase64":"{csv}""#);
    for needle in [r#""type":"file""#, r#""target":"pick""#, r#""name":"data.csv""#, &bytes] {
        expecting(&mut s, "test.file", Given::Input, needle);
        assert!(event(&mut s, "test.file", picked(&csv), Value::Null).is_ok(), "{needle}");
    }
    // White space in the base64 is left out before the plug-in is given it.
    let spaced = format!("{}\n{}", &csv[..8], &csv[8..]);
    assert!(event(&mut s, "test.file", picked(&spaced), Value::Null).is_ok());
    let big = vectorcraft_format::base64_encode(&vec![0; MAX_FILE + 1]);
    for (data, why) in [("not base64!", "not base64"), (big.as_str(), "larger than")] {
        let e = event(&mut s, "test.file", picked(data), Value::Null).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }) && e.to_string().contains(why), "{e}");
    }
    let e = event(&mut s, "test.file", json!({"type": "file", "target": "pick", "name": "/tmp/data.csv", "dataBase64": csv}), Value::Null);
    assert!(e.unwrap_err().to_string().contains("without its folder"));
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
    for refused in [
        "file.save",
        "edit.undo",
        "plugin.remove",
        "prefs.set",
        "stroke.widthProfile.reset",
        "object.pathfinder.options",
        "text.findFontFiles",
        "text.addFontFiles",
        "text.rescanFonts",
        "text.missingFonts",
        "command.batch",
    ] {
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

/// A panel sets type through a variable and steps through data sets it hands in as CSV, all as one
/// undo step.
#[test]
fn a_panel_steps_through_data_sets_it_hands_in() {
    let mut s = session();
    let commands = json!([
        {"command": "text.create", "params": {"x": 20, "y": 150, "text": "Title"}},
        {"command": "variable.define", "params": {"name": "Title", "kind": "text"}},
        {"command": "variable.bind", "params": {"variable": "Title"}},
        {"command": "dataset.import", "params": {"csv": "name,Title\nOne,Alpha\nTwo,Beta\n"}},
        {"command": "dataset.next"},
        {"command": "dataset.next"},
    ]);
    answering(&mut s, "test.data", &json!({"panel": {}, "commands": commands}));
    let r = click(&mut s, "test.data").unwrap();
    let id = r["results"][0]["id"].clone();
    assert_eq!(s.execute("text.getRange", &json!({"id": id})).unwrap()["text"], "Beta", "{r}");
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["active"], "Two");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.execute("text.getRange", &json!({"id": id})).is_err(), "one undo step takes the type back");
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["variables"], json!([]));
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"], json!([]));
}

/// A 1×1 PNG.
const DOT_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A panel's commands touch no file: one that reads or writes a path fails, and with it the batch,
/// writing nothing; a data set it applies leaves an image variable's image as it was, not reading
/// the file it names. Outside a panel the same commands read and write those files.
#[test]
fn a_panel_reads_and_writes_no_file() {
    let dir = std::env::temp_dir().join(format!("vc-panel-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    std::fs::write(path("data.csv"), "name,Title\nOne,Alpha\n").unwrap();
    std::fs::write(path("library.xml"), "<variableSets/>").unwrap();
    std::fs::write(path("other.png"), vectorcraft_format::base64_decode(DOT_PNG).unwrap()).unwrap();
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "Row"})).unwrap();
    for (command, params, refused) in [
        ("dataset.import", json!({"path": path("data.csv")}), "read authority is absent"),
        ("variable.loadLibrary", json!({"path": path("library.xml")}), "read authority is absent"),
        ("dataset.export", json!({"path": path("out.csv")}), "write authority is absent"),
    ] {
        answering(&mut s, "test.files", &json!({"panel": {}, "commands": [rectangle(150), {"command": command, "params": params}]}));
        let before = paths(&s);
        let e = click(&mut s, "test.files").unwrap_err();
        assert!(e.to_string().contains(refused), "{command}: {e}");
        assert_eq!(paths(&s), before, "{command}");
    }
    assert!(!dir.join("out.csv").exists());
    s.execute("dataset.export", &json!({"path": path("out.csv")})).unwrap();
    assert!(dir.join("out.csv").exists());

    s.execute("file.place", &json!({"name": "dot.png", "dataBase64": DOT_PNG})).unwrap();
    s.execute("variable.define", &json!({"name": "Picture", "kind": "image"})).unwrap();
    assert_eq!(s.execute("variable.bind", &json!({"variable": "Picture"})).unwrap()["bound"], 1);
    let commands = json!([
        {"command": "dataset.new", "params": {"name": "Other", "values": {"Picture": path("other.png")}}},
        {"command": "dataset.select", "params": {"name": "Other"}},
    ]);
    answering(&mut s, "test.files", &json!({"panel": {}, "commands": commands}));
    let r = click(&mut s, "test.files").unwrap();
    assert_eq!((&r["results"][1]["applied"], &r["results"][1]["skipped"]), (&json!(0), &json!(1)), "{r}");
    let r = s.execute("dataset.select", &json!({"name": "Other"})).unwrap();
    assert_eq!((&r["applied"], &r["skipped"]), (&json!(1), &json!(0)), "{r}");
    let _ = std::fs::remove_dir_all(dir);
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
