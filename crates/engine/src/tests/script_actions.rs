//! `app.doScript`: a script plays an action of `script.run`'s `actions`.

use serde_json::{Value, json};

use crate::*;

fn run_with(s: &mut Session, source: &str, actions: &Value) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js", "actions": actions}))
}

fn log_with(s: &mut Session, source: &str, actions: &Value) -> Vec<Value> {
    let r = run_with(s, source, actions).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The error message a `try` around `call` logs, or `"ok"`.
fn caught(s: &mut Session, call: &str, actions: &Value) -> String {
    let source = format!("try {{ {call}; vc.log('ok'); }} catch (e) {{ vc.log(e.name + ': ' + e.message); }}");
    log_with(s, &source, actions).first().and_then(Value::as_str).unwrap_or_default().to_string()
}

fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn objects(s: &Session) -> usize {
    s.active().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.active().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

fn rect(x: i64) -> Value {
    json!({"command": "shape.rectangle", "params": {"x": x, "y": 10, "width": 20, "height": 20}})
}

/// One set, `Set`, with the actions given as (name, steps).
fn set(actions: &[(&str, Vec<Value>)]) -> Value {
    Value::Array(actions.iter().map(|(name, steps)| json!({"set": "Set", "name": name, "steps": steps})).collect())
}

#[test]
fn do_script_plays_every_step_as_one_undo_step_with_the_script() {
    let mut s = doc();
    let before = undo_labels(&s);
    let actions = set(&[("Two", vec![rect(0), rect(50)])]);
    let out = log_with(&mut s, "vc.run('shape.rectangle', {x: 100, y: 10, width: 5, height: 5}); vc.log(app.doScript('Two', 'Set'));", &actions);
    assert_eq!(out, vec![Value::Null], "doScript returns undefined");
    assert_eq!(objects(&s), 3);
    let after = undo_labels(&s);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    assert_eq!(after.last().map(String::as_str), Some(script::STEP_LABEL));
    // The steps of a recorded `command.batch` run one by one, each counted as a host call.
    let batch = set(&[("Batch", vec![json!({"command": "command.batch", "params": {"commands": [rect(0), rect(10), rect(20)]}})])]);
    let r = run_with(&mut s, "app.doScript('Batch', 'Set', false);", &batch).unwrap();
    assert_eq!(objects(&s), 6);
    let r0 = run_with(&mut s, "1;", &batch).unwrap();
    assert!(r["hostCalls"].as_u64().unwrap() >= r0["hostCalls"].as_u64().unwrap() + 4, "{r}");
}

#[test]
fn a_later_throw_puts_back_what_the_action_did() {
    let mut s = doc();
    let before = (objects(&s), undo_labels(&s));
    let actions = set(&[("Two", vec![rect(0), rect(50)])]);
    let e = run_with(&mut s, "app.doScript('Two', 'Set'); throw new Error('late');", &actions).unwrap_err().to_string();
    assert!(e.contains("late"), "{e}");
    assert_eq!((objects(&s), undo_labels(&s)), before);
}

#[test]
fn a_failed_step_is_put_back_and_the_steps_before_it_stay() {
    let mut s = doc();
    let bad = json!({"command": "object.move", "params": {"ids": [999_999], "dx": 1, "dy": 1}});
    let actions = set(&[("Bad", vec![rect(0), bad, rect(50)])]);
    let m = caught(&mut s, "app.doScript('Bad', 'Set')", &actions);
    assert!(m.starts_with("Error: action step 2 of “Bad” (`object.move`) failed:"), "{m}");
    assert_eq!(objects(&s), 1, "the first step stays done and the third never runs");
}

#[test]
fn names_are_matched_exactly_and_must_be_one_action() {
    let mut s = doc();
    let actions = json!([
        {"set": "Set", "name": "One", "steps": [rect(0)]},
        {"set": "Set", "name": "Twice", "steps": [rect(0)]},
        {"set": "Set", "name": "Twice", "steps": [rect(0)]},
        {"set": "Set", "name": "Empty", "steps": []},
    ]);
    assert_eq!(caught(&mut s, "app.doScript('one', 'Set')", &actions), "Error: the set “Set” has no action “one”");
    assert_eq!(caught(&mut s, "app.doScript('One', 'set')", &actions), "Error: there is no action set “set”");
    assert_eq!(caught(&mut s, "app.doScript('Twice', 'Set')", &actions), "Error: the set “Set” has more than one action “Twice”");
    assert_eq!(caught(&mut s, "app.doScript('Empty', 'Set')", &actions), "Error: the action “Empty” has no included steps");
    assert_eq!(caught(&mut s, "app.doScript('One', 'Set')", &json!([])), "Error: there is no action set “Set”");
    assert_eq!(objects(&s), 0);
    assert_eq!(caught(&mut s, "app.doScript('One', 'Set', true)", &actions), "ok");
    assert_eq!(objects(&s), 1);
    assert_eq!(caught(&mut s, "app.loadAction(1)", &actions), "Error: Application.loadAction is not available yet");
    assert_eq!(caught(&mut s, "app.unloadAction('Set', '')", &actions), "Error: Application.unloadAction is not available yet");
}

#[test]
fn bad_arguments_throw_and_change_nothing() {
    let mut s = doc();
    let actions = set(&[("One", vec![rect(0)])]);
    for (call, want) in [
        ("app.doScript()", "TypeError"),
        ("app.doScript(1, 'Set')", "TypeError"),
        ("app.doScript('One', {})", "TypeError"),
        ("app.doScript('One', 'Set', 'yes')", "TypeError"),
        ("app.doScript('One', 'Set', 1)", "TypeError"),
        ("app.doScript('One', 'Set', false, 4)", "TypeError"),
        ("app.doScript('', 'Set')", "Error"),
        ("app.doScript(new Array(2000).join('x'), 'Set')", "RangeError"),
    ] {
        let m = caught(&mut s, call, &actions);
        assert!(m.starts_with(&format!("{want}:")), "{call}: {m}");
    }
    assert_eq!(objects(&s), 0);
}

#[test]
fn steps_a_script_may_not_run_stop_the_action_before_any_step() {
    let mut s = doc();
    s.prefs.show_external_script_warning = true;
    let steps = [
        (json!({"command": "script.run", "params": {"source": "1"}}), "runs `script.run`, which a script cannot run"),
        (json!({"command": "prefs.set", "params": {"key": "showExternalScriptWarning", "value": false}}), "runs `prefs.set`"),
        (json!({"command": "ui.action.play", "params": {"set": "Set", "action": "Loop"}}), "runs `ui.action.play`"),
        (json!({"command": "ui.runScript", "params": {"source": "1"}}), "runs `ui.runScript`"),
        (json!({"command": "view.zoomIn"}), "runs `view.zoomIn`"),
        (json!({"command": "action.stop", "params": {"message": "wait"}}), "is a stop, which a script cannot wait at"),
    ];
    for (bad, want) in steps {
        let actions = set(&[("Loop", vec![rect(0), bad.clone()])]);
        let m = caught(&mut s, "app.doScript('Loop', 'Set')", &actions);
        assert!(m.starts_with("Error: step 2 of “Loop” ") && m.contains(want), "{bad}: {m}");
        assert_eq!(objects(&s), 0, "{bad}: no step ran");
    }
    assert!(s.prefs.show_external_script_warning);
    // A refused command inside a recorded batch is refused too.
    let batch = set(&[("B", vec![json!({"command": "command.batch", "params": {"commands": [rect(0), {"command": "script.run", "params": {}}]}})])]);
    let m = caught(&mut s, "app.doScript('B', 'Set')", &batch);
    assert!(m.starts_with("Error: step 1.2 of “B” runs `script.run`"), "{m}");
    assert_eq!(objects(&s), 0);
    // So is one among the commands a Batch step plays on each file, before the steps ahead of it run.
    let files = json!({"command": "action.batch", "params": {"commands": [{"command": "select.all"}, {"command": "prefs.set", "params": {}}]}});
    let batch = set(&[("F", vec![rect(0), files])]);
    let m = caught(&mut s, "app.doScript('F', 'Set')", &batch);
    assert!(m.starts_with("Error: step 2.2 of “F” runs `prefs.set`, which a script cannot run"), "{m}");
    assert_eq!(objects(&s), 0);
}

#[test]
fn an_action_step_keeps_the_scripts_file_gate() {
    let mut s = doc();
    let path = std::env::temp_dir().join(format!("vc-do-script-{}.svg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let export = json!({"command": "document.export", "params": {"path": path.to_string_lossy(), "format": "svg"}});
    let actions = set(&[("Export", vec![export])]);
    let m = caught(&mut s, "app.doScript('Export', 'Set')", &actions);
    assert!(m.contains("Script file access denied"), "{m}");
    assert!(!path.exists());
}

#[test]
fn a_long_action_is_held_to_the_host_call_limit() {
    let mut s = doc();
    // Each step counts as a host call of its own, and its result as another, as for `vc.run`, so the
    // steps of one call cannot pass the limit.
    let calls = |s: &mut Session, steps: usize| {
        let actions = set(&[("One", vec![json!({"command": "select.none"}); steps])]);
        run_with(s, "app.doScript('One', 'Set');", &actions).unwrap()["hostCalls"].as_u64().unwrap()
    };
    let (one, many) = (calls(&mut s, 1), calls(&mut s, 1001));
    assert!(many >= one + 2000, "{one} {many}");
}

#[test]
fn bad_action_lists_are_refused_before_the_script_runs() {
    let mut s = doc();
    let step = || json!({"command": "select.none"});
    let cases = [
        (json!({}), "`actions` must be a list"),
        (json!([1]), "`actions[0]` must be an object"),
        (json!([{"set": "S", "name": 1, "steps": []}]), "`actions[0]`.name must be a string"),
        (json!([{"set": "S", "name": "A"}]), "`actions[0]`.steps must be a list"),
        (json!([{"set": "S", "name": "A", "steps": [], "key": 1}]), "unknown key `key`"),
        (json!([{"set": "S", "name": "A", "steps": [{"command": ""}]}]), "command must be a command id"),
        (json!([{"set": "S", "name": "A", "steps": [{"command": "select.none", "params": []}]}]), "params must be an object"),
        (json!([{"set": "S", "name": "A", "steps": [{"command": "select.none", "step": 0}]}]), "step must be a whole number"),
        (json!([{"set": "S", "name": "A", "steps": [{"command": "command.batch", "params": {}}]}]), "params.commands must be a list"),
        (
            json!([{"set": "S", "name": "A", "steps": [{"command": "command.batch", "params": {"commands": [{"command": "command.batch", "params": {"commands": []}}]}}]}]),
            "a command.batch inside a command.batch",
        ),
        (json!([{"set": "S", "name": "x".repeat(2000), "steps": []}]), "longer than 1024 bytes"),
        (Value::Array(vec![json!({"set": "S", "name": "A", "steps": []}); script::actions::MAX_ACTIONS + 1]), "more than 4096 actions"),
        (json!([{"set": "S", "name": "A", "steps": vec![step(); script::actions::MAX_ACTION_STEPS + 1]}]), "more than 10000 steps"),
    ];
    for (actions, want) in cases {
        let e = run_with(&mut s, "vc.run('shape.rectangle', {x: 0, y: 0, width: 1, height: 1});", &actions).unwrap_err().to_string();
        assert!(e.contains(want), "{want}: {e}");
    }
    let many: Vec<Value> = (0..11).map(|i| json!({"set": "S", "name": format!("A{i}"), "steps": vec![step(); 10_000]})).collect();
    let e = run_with(&mut s, "1;", &Value::Array(many)).unwrap_err().to_string();
    assert!(e.contains("more than 100000 steps in all"), "{e}");
    assert_eq!(objects(&s), 0, "no run started");
}

#[test]
fn a_kept_engine_plays_the_actions_of_the_run_that_calls_it() {
    let mut s = doc();
    let first = set(&[("One", vec![rect(0)])]);
    log_with(&mut s, "#targetengine keep\nvar play = function () { app.doScript('One', 'Set'); };", &first);
    assert_eq!(objects(&s), 0);
    // The next run has no actions: the function kept from the first run finds none.
    let call = "#targetengine keep\ntry { play(); vc.log('ok'); } catch (e) { vc.log(e.message); }";
    assert_eq!(log_with(&mut s, call, &json!([])), vec![json!("there is no action set “Set”")]);
    let two = set(&[("One", vec![rect(0), rect(30)])]);
    assert_eq!(log_with(&mut s, call, &two), vec![json!("ok")]);
    assert_eq!(objects(&s), 2);
}

/// Makes artboard `index` of the active document the view's active one, as the app does before each
/// command.
fn make_active(s: &mut Session, index: usize) {
    let id = s.active().unwrap().doc.artboards[index].id;
    s.doc_mut().unwrap().active_artboard = Some(id);
}

/// A step that acts on the active artboard, Align to Artboard in a recorded batch and Fit to Selected
/// Art, acts on the active artboard's index as the run started; after the script deletes that
/// artboard, the last one. A step that names its artboard keeps it.
#[test]
fn steps_act_on_the_artboard_active_as_they_run() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let boards = |s: &Session| s.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let was = boards(&s);
    make_active(&mut s, 2);
    let align = |to: Value| json!({"command": "object.align", "params": {"horizontal": "left", "to": "artboard", "artboard": to}});
    let mut first = align(json!(0));
    first["params"].as_object_mut().unwrap().remove("artboard");
    let actions = set(&[
        ("Align", vec![json!({"command": "command.batch", "params": {"commands": [first]}})]),
        ("Align 1", vec![align(json!(0))]),
        ("Fit", vec![json!({"command": "artboard.fitToSelection"})]),
    ]);
    let left = |s: &Session| s.active().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap().x0;
    log_with(&mut s, "vc.run('artboard.delete', {index: 2}); app.doScript('Align', 'Set'); app.doScript('Fit', 'Set');", &actions);
    let art = s.active().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap();
    assert_eq!(art.x0, was[1].x0, "aligned to the last artboard, where the deleted active one was");
    assert_eq!(boards(&s), [was[0], art], "Fit to Selected Art resizes that artboard and leaves the other");
    log_with(&mut s, "app.doScript('Align 1', 'Set');", &actions);
    assert_eq!(left(&s), was[0].x0, "a step that names artboard 0 keeps it");
}

/// A recorded `command.batch` step that names its `artboard` gives it to its steps that name none,
/// past the last artboard the last one, as `command.batch` does; a step that names its own keeps it.
#[test]
fn a_batch_steps_artboard_holds_for_its_steps_that_name_none() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let was: Vec<_> = s.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    make_active(&mut s, 1);
    let align = json!({"command": "object.align", "params": {"horizontal": "left", "to": "artboard"}});
    let mut own = align.clone();
    own["params"]["artboard"] = json!(1);
    let batch = |artboard: u64, step: &Value| json!({"command": "command.batch", "params": {"artboard": artboard, "commands": [step]}});
    let actions = set(&[("First", vec![batch(0, &align)]), ("Past", vec![batch(9, &align)]), ("Own", vec![batch(0, &own)])]);
    let left = |s: &Session| s.active().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap().x0;
    log_with(&mut s, "app.doScript('First', 'Set');", &actions);
    assert_eq!(left(&s), was[0].x0, "the batch's artboard 0");
    log_with(&mut s, "app.doScript('Past', 'Set');", &actions);
    assert_eq!(left(&s), was[2].x0, "past the last artboard, the last one");
    log_with(&mut s, "app.doScript('Own', 'Set');", &actions);
    assert_eq!(left(&s), was[1].x0, "a step that names artboard 1 keeps it");
}

/// The active artboard a step gets is the index it had as the run started: after a step deletes an
/// artboard before it, the artboard now at that index, as the view shows it.
#[test]
fn steps_keep_the_active_artboards_index() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let uid = s.active().unwrap().uid;
    let start = script::actions::Start { active: &[(uid, 1)], picks: None, batch: None };
    let add = |s: &Session| script::actions::on_active_artboard(s, &start, "object.align", &json!({}));
    assert_eq!(add(&s), json!({"artboard": 1}));
    s.execute("artboard.delete", &json!({"index": 0})).unwrap();
    assert_eq!(add(&s), json!({"artboard": 1}), "the artboard now at index 1");
    s.execute("artboard.delete", &json!({"index": 1})).unwrap();
    assert_eq!(add(&s), json!({"artboard": 0}), "past the last artboard, the last one");
}

/// A Fit to Artwork Bounds step that names no artboard fits the active one; one that names its
/// `indices` fits those.
#[test]
fn a_fit_to_artwork_bounds_step_fits_the_active_artboard_or_its_own() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
    let was: Vec<_> = s.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let mut art = Vec::new();
    for b in &was {
        let id = s.execute("shape.rectangle", &json!({"x": b.x0 + 10.0, "y": 10, "width": 20, "height": 30})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        art.push(s.active().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap());
    }
    make_active(&mut s, 1);
    let actions = set(&[
        ("Fit", vec![json!({"command": "artboard.fitToArt"})]),
        ("Fit first", vec![json!({"command": "artboard.fitToArt", "params": {"indices": [0]}})]),
    ]);
    let boards = |s: &Session| s.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    log_with(&mut s, "app.doScript('Fit', 'Set');", &actions);
    assert_eq!(boards(&s), [was[0], art[1]], "the active artboard fits its art");
    log_with(&mut s, "app.doScript('Fit first', 'Set');", &actions);
    assert_eq!(boards(&s), art, "the step's own indices");
}

/// While the Artboard tool has several artboards selected, a paste step gets no artboard, so that it
/// pastes on those, and a Fit to Artwork Bounds step gets those as its `indices`; the other steps
/// still get the active one.
#[test]
fn paste_and_fit_steps_keep_the_artboard_tools_selection() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    make_active(&mut s, 1);
    let start = script::actions::Start { active: &[], picks: None, batch: None };
    let add = |s: &Session, id: &str| script::actions::on_active_artboard(s, &start, id, &json!({}));
    assert_eq!(add(&s, "edit.pasteInPlace"), json!({"artboard": 1}));
    assert_eq!(add(&s, "artboard.fitToArt"), json!({"index": 1}));
    s.select_tool("artboard", crate::tooling::ViewInfo::default()).unwrap();
    s.set_tool_option("selected", &json!([0, 2]));
    for id in ["edit.pasteInPlace", "edit.pasteInFront", "edit.pasteInBack"] {
        assert_eq!(add(&s, id), json!({}), "{id}");
    }
    assert_eq!(add(&s, "artboard.fitToArt"), json!({"indices": [0, 2]}));
    assert_eq!(add(&s, "artboard.fitToSelection"), json!({"index": 1}));
    assert_eq!(add(&s, "object.align"), json!({"artboard": 1}));
    assert_eq!(add(&s, "shape.rectangle"), json!({}));
}

/// The Artboard tool's selection as `script.run` starts holds for the steps `app.doScript` plays,
/// though the run gives the tool fresh state: a paste step pastes on those artboards and a Fit to
/// Artwork Bounds step fits them.
#[test]
fn a_run_keeps_the_artboard_tools_selection_for_its_steps() {
    // Three artboards with a rectangle on each, artboard 1 active and the Artboard tool holding 0 and 2.
    let setup = || {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
        let was: Vec<_> = s.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        let mut art = Vec::new();
        for b in &was {
            let id = s.execute("shape.rectangle", &json!({"x": b.x0 + 10.0, "y": 10, "width": 20, "height": 30})).unwrap()["id"].as_u64().unwrap();
            s.execute("paint.setStroke", &json!({"none": true})).unwrap();
            art.push(s.active().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap());
        }
        s.execute("edit.copy", &json!({})).unwrap();
        make_active(&mut s, 1);
        s.select_tool("artboard", crate::tooling::ViewInfo::default()).unwrap();
        s.set_tool_option("selected", &json!([0, 2]));
        (s, was, art)
    };
    let actions = set(&[("Paste", vec![json!({"command": "edit.pasteInPlace"})]), ("Fit", vec![json!({"command": "artboard.fitToArt"})])]);
    let (mut s, ..) = setup();
    let before = objects(&s);
    log_with(&mut s, "app.doScript('Paste', 'Set');", &actions);
    assert_eq!(objects(&s), before + 2, "one copy on each selected artboard");
    let (mut s, was, art) = setup();
    log_with(&mut s, "app.doScript('Fit', 'Set');", &actions);
    let boards: Vec<_> = s.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    assert_eq!(boards, [art[0], was[1], art[2]], "the selected artboards fit their art");
    // A kept artboard past the last one is left out; with fewer than two left, the active one applies.
    let (mut s, ..) = setup();
    s.reset_tool();
    let uid = s.active().unwrap().uid;
    let kept = (uid, vec![0, 2]);
    let start = script::actions::Start { active: &[(uid, 1)], picks: Some(&kept), batch: None };
    let add = |s: &Session, id: &str| script::actions::on_active_artboard(s, &start, id, &json!({}));
    assert_eq!(add(&s, "edit.pasteInBack"), json!({"artboards": [0, 2]}));
    assert_eq!(add(&s, "artboard.fitToSelection"), json!({"index": 1}));
    s.execute("artboard.delete", &json!({"index": 2})).unwrap();
    assert_eq!(add(&s, "artboard.fitToArt"), json!({"index": 1}));
}
