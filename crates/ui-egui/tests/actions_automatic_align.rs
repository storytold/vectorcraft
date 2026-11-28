//! An action's opt-in Align To follows the selection at playback without rewriting its recording.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;

use craft::automation::actions::{Action, ActionList, ActionStep};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::panels::actions;
use vectorcraft_ui_egui::{UiState, VectorcraftApp};

static CASE: Mutex<()> = Mutex::new(());

struct CenterRecording {
    document: Document,
    a: NodeId,
    b: NodeId,
    ui: Value,
    raw_steps: Value,
}

fn bounds(app: &VectorcraftApp, id: NodeId) -> [f64; 4] {
    let b = app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    [b.x0, b.y0, b.x1, b.y1]
}

fn close(actual: [f64; 4], expected: [f64; 4]) -> bool {
    actual.into_iter().zip(expected).all(|(a, e)| (a - e).abs() < 1e-8)
}

/// Makes the action list one set, `name`, holding `members` in order.
fn one_set(app: &mut VectorcraftApp, name: &str, members: Vec<Action>) {
    let mut list = ActionList::new();
    list.new_set(name).unwrap();
    for action in members {
        list.push(Action { set: Some(name.to_string()), ..action }).unwrap();
    }
    app.ui.actions = list;
}

/// The list index of action `n` of the first set.
fn index(app: &VectorcraftApp, n: usize) -> usize {
    let list = &app.ui.actions;
    actions::members(list, list.sets()[0])[n]
}

/// Action `n` of the first set.
fn action(app: &VectorcraftApp, n: usize) -> &Action {
    app.ui.actions.action(index(app, n)).unwrap()
}

/// Gives the first action of the first set `params` for its step `step`.
fn set_params(app: &mut VectorcraftApp, step: usize, params: Value) {
    let i = index(app, 0);
    let command = app.ui.actions.action(i).unwrap().steps[step].command.clone();
    app.ui.actions.delete_step(i, step).unwrap();
    app.ui.actions.insert_step(i, step, ActionStep::new(command, params)).unwrap();
}

/// Record Center exactly once through New Action, the two actual Align commands, and Stop.
fn record_center(automatic: bool) -> CenterRecording {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "profile": null})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    let a = NodeId(app.run("shape.rectangle", json!({"x": 20, "y": 40, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let b = NodeId(app.run("shape.rectangle", json!({"x": 180, "y": 120, "width": 80, "height": 60})).unwrap()["id"].as_u64().unwrap());
    let document = (*app.session.active().unwrap().doc).clone();
    assert!(close(bounds(&app, a), [20.0, 40.0, 60.0, 60.0]));
    assert!(close(bounds(&app, b), [180.0, 120.0, 260.0, 180.0]));
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("select.key", json!({"id": b.0})).unwrap();
    one_set(&mut app, "Automatic Align", vec![]);
    let new_action = if automatic { json!({"set": 0, "name": "Center", "automaticAlignTo": true}) } else { json!({"set": 0, "name": "Center"}) };
    app.run(actions::NEW_ACTION, new_action).unwrap();
    app.run("object.align", json!({"horizontal": "center", "to": "key"})).unwrap();
    app.run("object.align", json!({"vertical": "center", "to": "key"})).unwrap();
    app.run(actions::STOP_RECORDING, json!({})).unwrap();
    let recorded = action(&app, 0);
    assert_eq!(recorded.steps.len(), 2);
    assert!(recorded.steps.iter().all(|s| s.command == "object.align" && s.params["to"] == "key"));
    assert_eq!(actions::automatic_align_to(recorded), automatic, "New Action sets the option it is given");
    let raw_steps = serde_json::to_value(&recorded.steps).unwrap();
    let ui = serde_json::to_value(&app.ui).unwrap();
    CenterRecording { document, a, b, ui, raw_steps }
}

fn playback_app(recording: &CenterRecording, only_a: bool, key: bool) -> VectorcraftApp {
    let mut session = Session::new();
    session.add_document(recording.document.clone(), None);
    let mut app = VectorcraftApp::new(session, Default::default());
    // Real UiState serialization/deserialization, not a separately authored Center action.
    app.ui = serde_json::from_slice::<UiState>(&serde_json::to_vec(&recording.ui).unwrap()).unwrap();
    let ids = if only_a { vec![recording.a.0] } else { vec![recording.a.0, recording.b.0] };
    app.run("select.set", json!({"ids": ids})).unwrap();
    if key {
        app.run("select.key", json!({"id": recording.b.0})).unwrap();
    } else {
        app.run("select.key", json!({})).unwrap();
    }
    app
}

#[derive(Debug)]
struct Observation {
    label: &'static str,
    result: Result<Value, String>,
    actual: [[f64; 4]; 2],
    expected: [[f64; 4]; 2],
    raw_unchanged: bool,
    selection_unchanged: bool,
    undo_steps: usize,
    undo_restored: bool,
}

#[test]
fn automatic_center_recording_resolves_all_three_playback_selections() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let recording = record_center(true);
    let mut observed = vec![];
    for (label, only_a, key, expected) in [
        ("one object / artboard", true, false, [[180.0, 140.0, 220.0, 160.0], [180.0, 120.0, 260.0, 180.0]]),
        ("two objects / B key", false, true, [[200.0, 140.0, 240.0, 160.0], [180.0, 120.0, 260.0, 180.0]]),
        ("two objects / selection", false, false, [[120.0, 100.0, 160.0, 120.0], [100.0, 80.0, 180.0, 140.0]]),
    ] {
        let mut app = playback_app(&recording, only_a, key);
        let selection = app.session.active().unwrap().selection.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let result = actions::play(&mut app, 0, 0);
        let actual = [bounds(&app, recording.a), bounds(&app, recording.b)];
        let raw_unchanged = raw_steps(&app) == recording.raw_steps;
        let selection_unchanged = app.session.active().unwrap().selection == selection;
        let undo_steps = app.session.active().unwrap().history.undo.len().saturating_sub(undo);
        let undo_restored = result.is_ok()
            && app.run("edit.undo", json!({})).is_ok()
            && close(bounds(&app, recording.a), [20.0, 40.0, 60.0, 60.0])
            && close(bounds(&app, recording.b), [180.0, 120.0, 260.0, 180.0]);
        observed.push(Observation { label, result, actual, expected, raw_unchanged, selection_unchanged, undo_steps, undo_restored });
    }
    // All three actual playbacks, including the legitimate B-key control, precede assertions.
    println!("automatic Center raw steps: {}", recording.raw_steps);
    println!("automatic Center playback observations: {observed:#?}");
    for o in observed {
        assert!(o.result.is_ok(), "{}: expected automatic reference selection, got {:?}", o.label, o.result);
        assert!(close(o.actual[0], o.expected[0]) && close(o.actual[1], o.expected[1]), "{}: {o:?}", o.label);
        assert!(o.raw_unchanged, "{}: playback must not rewrite the stored recording", o.label);
        assert!(o.selection_unchanged, "{}: selected IDs and key remain unchanged", o.label);
        assert_eq!(o.undo_steps, 1, "{}: Accelerated Center is one undo step", o.label);
        assert!(o.undo_restored, "{}: one Undo restores both original rectangles", o.label);
    }
}

#[test]
fn legacy_center_without_the_option_keeps_its_frozen_key_target() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let recording = record_center(false);
    assert!(recording.ui["action_sets"][0]["actions"][0].get("automaticAlignTo").is_none());
    let mut app = playback_app(&recording, true, false);
    let error = actions::play(&mut app, 0, 0).unwrap_err();
    assert!(error.to_lowercase().contains("key"), "{error}");
    assert!(close(bounds(&app, recording.a), [20.0, 40.0, 60.0, 60.0]));
    assert!(close(bounds(&app, recording.b), [180.0, 120.0, 260.0, 180.0]));
    assert_eq!(raw_steps(&app), recording.raw_steps);
}

fn raw_steps(app: &VectorcraftApp) -> Value {
    serde_json::to_value(&action(app, 0).steps).unwrap()
}

fn automatic_flag(app: &VectorcraftApp) -> bool {
    actions::automatic_align_to(action(app, 0))
}

#[test]
fn automatic_align_options_toggle_restores_the_original_recorded_targets() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let recording = record_center(true);
    let mut app = playback_app(&recording, false, false);
    app.run(actions::ACTION_OPTIONS, json!({"set": 0, "action": 0, "automaticAlignTo": false})).unwrap();
    assert!(!automatic_flag(&app));
    let error = actions::play(&mut app, 0, 0).unwrap_err();
    assert!(error.to_lowercase().contains("key"), "{error}");
    assert_eq!(raw_steps(&app), recording.raw_steps, "turning the option off does not replace recorded params");
    assert!(close(bounds(&app, recording.a), [20.0, 40.0, 60.0, 60.0]));
    app.run(actions::ACTION_OPTIONS, json!({"set": 0, "action": 0, "automaticAlignTo": true})).unwrap();
    assert!(automatic_flag(&app));
    let i = index(&app, 0);
    let copy = app.ui.actions.duplicate_action(i, |_, _, _| {}).unwrap();
    assert!(actions::automatic_align_to(app.ui.actions.action(copy).unwrap()), "copying retains the per-action choice");
    actions::play(&mut app, 0, 0).unwrap();
    assert!(close(bounds(&app, recording.a), [120.0, 100.0, 160.0, 120.0]));
    assert!(close(bounds(&app, recording.b), [100.0, 80.0, 180.0, 140.0]));
    assert_eq!(raw_steps(&app), recording.raw_steps);
}

#[test]
fn automatic_align_observes_selection_changed_by_an_earlier_accelerated_step() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let recording = record_center(true);
    let mut app = playback_app(&recording, false, true);
    let i = index(&app, 0);
    app.ui.actions.insert_step(i, 0, ActionStep::new("select.set", json!({"ids": [recording.a.0]}))).unwrap();
    let raw = raw_steps(&app);
    let undo = app.session.active().unwrap().history.undo.len();
    actions::play(&mut app, 0, 0).unwrap();
    assert!(close(bounds(&app, recording.a), [180.0, 140.0, 220.0, 160.0]), "the new one-object selection uses the artboard, not the former key");
    assert!(close(bounds(&app, recording.b), [180.0, 120.0, 260.0, 180.0]));
    assert_eq!(raw_steps(&app), raw);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert!(close(bounds(&app, recording.a), [20.0, 40.0, 60.0, 60.0]));
}

#[test]
fn automatic_align_step_speed_and_stop_continue_share_the_compiled_choice() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let recording = record_center(true);
    let mut app = playback_app(&recording, true, false);
    let i = index(&app, 0);
    app.ui.actions.insert_step(i, 1, ActionStep::new(actions::STOP_STEP, json!({"message": "Choose B as key", "allowContinue": true}))).unwrap();
    let raw = raw_steps(&app);
    app.run(actions::PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    actions::play(&mut app, 0, 0).unwrap();
    assert!(close(bounds(&app, recording.a), [180.0, 40.0, 220.0, 60.0]), "only horizontal Center ran before the Step wait");
    assert!(actions::playback_held(&app));
    actions::playback_drawn(&mut app);
    actions::tick_playback(&mut app, 1.0);
    assert!(actions::waiting_at_stop(&app));
    // B must differ from the artboard center so a stale artboard reference fails this check.
    app.run("select.set", json!({"ids": [recording.b.0]})).unwrap();
    app.run("object.move", json!({"dx": 0, "dy": 40})).unwrap();
    app.run("select.set", json!({"ids": [recording.a.0, recording.b.0]})).unwrap();
    app.run("select.key", json!({"id": recording.b.0})).unwrap();
    app.run(actions::CONTINUE_PLAYBACK, json!({})).unwrap();
    assert!(app.ui.action_playing.is_none());
    assert!(
        close(bounds(&app, recording.a), [180.0, 180.0, 220.0, 200.0]),
        "continued vertical Center resolves B at y=190, not the artboard at y=150"
    );
    assert!(close(bounds(&app, recording.b), [180.0, 160.0, 260.0, 220.0]), "the current key stays stationary during Continue");
    assert_eq!(raw_steps(&app), raw);
}

// These are the actual host I/O callbacks used by public Save/Load Actions commands.
// The stored bytes are observable without writing user files or exposing private file helpers.
type ActionFiles = std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<String, Vec<u8>>>>;

fn host_files(app: &mut VectorcraftApp) -> ActionFiles {
    let files = ActionFiles::default();
    let writes = files.clone();
    app.services.write = Some(Box::new(move |path, bytes| {
        writes.borrow_mut().insert(path.to_string(), bytes.to_vec());
        Ok(())
    }));
    let reads = files.clone();
    app.services.read = Some(Box::new(move |path| reads.borrow().get(path).cloned().ok_or_else(|| format!("no saved action file: {path}"))));
    files
}

fn save_file(app: &mut VectorcraftApp, files: &ActionFiles, path: &str) -> Value {
    app.run(actions::SAVE_SET, json!({"set": 0, "path": path})).unwrap();
    serde_json::from_slice(files.borrow().get(path).unwrap()).unwrap()
}

fn load_only_file(app: &mut VectorcraftApp, path: &str) {
    app.ui.actions = ActionList::new();
    let result = app.run(actions::LOAD_SET, json!({"path": path})).unwrap();
    assert_eq!(result["set"], 0);
}

#[test]
fn automatic_align_files_preserve_enabled_and_raw_recorded_excluded_semantics() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let recording = record_center(true);
    let mut app = playback_app(&recording, false, false);
    let files = host_files(&mut app);
    let enabled = save_file(&mut app, &files, "enabled.vcactions");
    assert_eq!(enabled["version"], 7);
    assert_eq!(enabled["actions"][0]["automaticAlignTo"], true);
    load_only_file(&mut app, "enabled.vcactions");
    assert!(automatic_flag(&app), "public file loading retains the enabled choice");
    assert_eq!(raw_steps(&app), recording.raw_steps);

    // The destination stays default-false; its automatic meaning comes only from recorded raw steps.
    app.run(actions::NEW_ACTION, json!({"set": 0, "name": "Recorded Automatic Center"})).unwrap();
    actions::play(&mut app, 0, 0).unwrap();
    app.run(actions::STOP_RECORDING, json!({})).unwrap();
    let destination = action(&app, 1).clone();
    assert!(!actions::automatic_align_to(&destination));
    assert_eq!(destination.steps.len(), 1);
    assert_eq!(destination.steps[0].command, "command.batch");
    let nested = destination.steps[0].params["commands"].as_array().unwrap();
    assert_eq!(nested.len(), 2);
    assert!(nested.iter().all(|s| s["command"] == "object.align" && s["params"]["to"] == "auto"));
    let raw = serde_json::to_value(&destination.steps).unwrap();
    one_set(&mut app, "Recorded Default-false", vec![destination]);
    let recorded = save_file(&mut app, &files, "recorded.vcactions");
    assert_eq!(recorded["version"], 7, "raw automatic batches require version 7 even when no action flag is true");
    assert_ne!(recorded["actions"][0]["automaticAlignTo"], true);
    load_only_file(&mut app, "recorded.vcactions");
    assert!(!automatic_flag(&app));
    assert_eq!(raw_steps(&app), raw);
    let imported = action(&app, 0).clone();
    let mut fresh = playback_app(&recording, false, false);
    one_set(&mut fresh, "Imported", vec![imported]);
    actions::play(&mut fresh, 0, 0).unwrap();
    assert!(close(bounds(&fresh, recording.a), [120.0, 100.0, 160.0, 120.0]));
    assert!(close(bounds(&fresh, recording.b), [100.0, 80.0, 180.0, 140.0]));

    let i = index(&app, 0);
    app.ui.actions.set_step(i, 0, false).unwrap();
    let excluded = save_file(&mut app, &files, "excluded.vcactions");
    assert_eq!(excluded["version"], 7, "excluding a raw automatic batch does not make its stored semantics compatible with older readers");
    load_only_file(&mut app, "excluded.vcactions");
    assert!(!automatic_flag(&app));
    assert_eq!(raw_steps(&app), raw);
    assert!(action(&app, 0).disabled.contains(&0));

    // Nested raw batches may be stored/excluded even though command.batch cannot execute nested batches.
    // The format requirement follows their retained command semantics, not an executable flattening.
    let recorded_batch = action(&app, 0).steps[0].params.clone();
    set_params(&mut app, 0, json!({"commands": [{"command": "command.batch", "params": recorded_batch}]}));
    let i = index(&app, 0);
    app.ui.actions.set_step(i, 0, false).unwrap();
    let deeper = raw_steps(&app);
    assert_eq!(save_file(&mut app, &files, "nested-excluded.vcactions")["version"], 7);
    load_only_file(&mut app, "nested-excluded.vcactions");
    assert_eq!(raw_steps(&app), deeper);
    assert!(action(&app, 0).disabled.contains(&0));
}

const LEGACY_V1: &str = r#"{
  "format": "vcactions",
  "version": 1,
  "name": "Legacy Align",
  "actions": [
    {
      "name": "Frozen Center",
      "steps": [
        {
          "command": "object.align",
          "params": {
            "horizontal": "center",
            "to": "key"
          }
        }
      ]
    }
  ]
}"#;

const LEGACY_V6: &str = r#"{
  "format": "vcactions",
  "version": 6,
  "name": "Legacy View",
  "actions": [
    {
      "name": "Zoom",
      "steps": [
        {
          "command": "view.zoomIn",
          "params": {}
        }
      ]
    }
  ]
}"#;

#[test]
fn automatic_align_legacy_files_keep_their_old_bytes_and_versions() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let files = host_files(&mut app);
    for (path, wire, version) in [("legacy-v1.vcactions", LEGACY_V1, 1), ("legacy-v6.vcactions", LEGACY_V6, 6)] {
        files.borrow_mut().insert(path.into(), wire.as_bytes().to_vec());
        load_only_file(&mut app, path);
        assert!(!automatic_flag(&app));
        let saved = save_file(&mut app, &files, "legacy-resaved.vcactions");
        assert_eq!(saved["version"], version);
        assert_eq!(files.borrow().get("legacy-resaved.vcactions").unwrap(), wire.as_bytes(), "false-only legacy serialization is byte-equivalent");
    }
    let future = LEGACY_V6.replace("\"version\": 6", "\"version\": 8");
    assert!(app.run(actions::LOAD_SET, json!({"data": future})).unwrap_err().contains("later version"));
}

#[test]
fn automatic_align_ignores_an_artboard_the_step_stores() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "artboards": 2, "profile": null})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    let a = NodeId(app.run("shape.rectangle", json!({"x": 20, "y": 40, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let center = json!({"horizontal": "center", "vertical": "center", "to": "artboard"});
    app.run("object.align", center.clone()).unwrap();
    let current = bounds(&app, a);
    app.run("edit.undo", json!({})).unwrap();
    let mut stored = center;
    stored["artboard"] = json!(1);
    app.run("object.align", stored.clone()).unwrap();
    let second = bounds(&app, a);
    app.run("edit.undo", json!({})).unwrap();
    assert!(!close(current, second), "the two artboards center the rectangle in different places");

    let mut center_action = Action::new("Center", vec![ActionStep::new("object.align", stored.clone())]);
    actions::set_automatic_align_to(&mut center_action, true);
    one_set(&mut app, "Automatic Align", vec![center_action]);
    actions::play(&mut app, 0, 0).unwrap();
    assert!(close(bounds(&app, a), current), "automatic playback uses the current artboard, not the stored one");
    assert_eq!(action(&app, 0).steps[0].params, stored, "the stored step keeps its artboard");
}

#[test]
fn automatic_align_inside_a_recorded_batch_run_needs_version_7() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let files = host_files(&mut app);
    let align = json!({"command": "object.align", "params": {"horizontal": "center", "to": "auto"}});
    let batch = json!({"commands": [align], "label": "Center", "source": "datasets", "destination": "none"});
    one_set(&mut app, "Recorded Batch", vec![Action::new("Batch Center", vec![ActionStep::new("action.batch", batch.clone())])]);
    let saved = save_file(&mut app, &files, "batch.vcactions");
    assert_eq!(saved["version"], 7, "a Batch run's automatic Align step needs a reader that knows `to: \"auto\"`");
    assert_ne!(saved["actions"][0]["automaticAlignTo"], true);
    load_only_file(&mut app, "batch.vcactions");
    assert_eq!(action(&app, 0).steps[0].params, batch);
}
