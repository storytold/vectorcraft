use super::tests_support::*;
use super::*;
use craft::ui::dialog::StackDialog;
use std::collections::BTreeSet;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::toast::{Level, Toast};

use crate::dialogs::action_stop::RecordStopDialog;

/// A recording keeps its steps when a canceled undo group drops journal entries made before the
/// recording began.
#[test]
fn a_recording_keeps_its_steps_when_a_canceled_undo_group_drops_earlier_entries() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let rect = |x: i32| json!({"x": x, "y": 10, "width": 20, "height": 20});
    for (id, p) in [
        ("file.new", json!({"width": 200, "height": 100})),
        ("file.new", json!({"width": 200, "height": 100})),
        ("document.activate", json!({"index": 0})),
        ("edit.undoGroup.begin", json!({})),
        ("shape.rectangle", rect(10)),
    ] {
        app.run(id, p).unwrap();
    }
    app.run(START_RECORDING, json!({})).unwrap();
    for (id, p) in [("document.activate", json!({"index": 1})), ("shape.rectangle", rect(50)), ("edit.undoGroup.end", json!({"cancel": true}))] {
        app.run(id, p).unwrap();
    }
    app.run(STOP_RECORDING, json!({})).unwrap();
    let steps = set_actions(&app, 0).last().map(|a| a.steps.iter().map(|s| s.command.clone()).collect::<Vec<_>>());
    assert_eq!(steps, Some(vec!["document.activate".to_string(), "shape.rectangle".into(), "edit.undoGroup.end".into()]));
}

/// A recording that New Action started goes into its action wherever the action is when Stop
/// is pressed, under the name the action has then, and is dropped when the action was deleted
/// meanwhile.
#[test]
fn a_new_actions_recording_follows_its_action() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run(NEW_ACTION, json!({"set": 2, "name": "Box"})).unwrap();
    app.run(DELETE_SETS, json!({"names": ["Mine"]})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Crate"})).unwrap();
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
    assert_eq!((action_names(&app, 1), set_actions(&app, 1)[0].steps.len()), (vec!["Crate".to_string()], 1), "the renamed action keeps the steps");
    app.run(NEW_ACTION, json!({"set": 1, "name": "Gone"})).unwrap();
    let gone = model::flat(&app.ui.actions, 1, 1).unwrap();
    app.ui.actions.delete_action(gone).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(action_names(&app, 1), ["Crate"], "the deleted action is not made again");
    assert!(app.ui.recording.is_none() && app.ui.recording_into.is_none());
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": false}), "nothing recording");
    app.run(NEW_ACTION, json!({"set": 1, "name": "Remote"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
    assert_eq!((action_names(&app, 1), set_actions(&app, 1)[1].steps.len()), (vec!["Crate".to_string(), "Remote".into()], 1));
    assert!(app.ui.recording.is_none());
}

#[test]
fn insert_menu_item_goes_into_the_recording() {
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap(), json!({"recording": true}));
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "file.save"})).unwrap_err(), "no menu item that can be inserted runs this command");
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = set_actions(&app, 0).pop().unwrap();
    assert_eq!(recorded.steps, [ActionStep::new("object.group", json!({}))]);
}

#[test]
fn insert_select_path_goes_into_the_recording() {
    let (mut app, _, [first, _]) = app_with_two_rectangles();
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(INSERT_SELECT_PATH, json!({})).unwrap(), json!({"recording": true}));
    assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(first)]);
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = set_actions(&app, 0).pop().unwrap();
    assert_eq!(recorded.steps.len(), 1);
    assert_eq!((recorded.steps[0].command.as_str(), &recorded.steps[0].params["id"]), (SELECT_PATH_STEP, &json!(first)));
}

// PLAN: the local `stop_and_record` (and menus-w14's `stop_and_new_action` in `tests_support.rs`) lost their last callers when Record and Stop moved to `ui.action.record`/`ui.action.stop`; this file drops `stop_and_record`, menus-w14 owns the other.
#[test]
fn begin_recording_selected_action_adds_steps_without_new_action() {
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({"set": 0, "action": 0})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0).len(), 1, "no new action");
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2", "object.move 4", "object.move 8"], "added at the end");

    // With a step selected, the recorded steps go after it.
    app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 0})).unwrap();
    app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 16", "object.move 2", "object.move 4", "object.move 8"]);

    // With no action selected, Begin Recording still adds a new action.
    app.run(START_RECORDING, json!({"set": 0})).unwrap();
    app.run("object.move", json!({"dx": 32, "dy": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(action_names(&app, 0), ["Three Moves", "Action 2"]);
    assert_eq!(step_names(&app, 0, 1), ["object.move 32"]);
}

#[test]
fn begin_recording_after_selected_step_preserves_order_and_exclusions() {
    let (mut app, _) = app_with_three_moves();
    let at = model::flat(&app.ui.actions, 0, 0).unwrap();
    for step in [0, 2] {
        app.ui.actions.set_step(at, step, false).unwrap();
    }
    for (params, want) in [
        (json!({"action": 0}), "no such action"),
        (json!({"set": 0, "after": 0}), "no such action"),
        (json!({"set": 1, "action": 0}), "no such action"),
        (json!({"set": 0, "action": 1}), "no such action"),
        (json!({"set": 0, "action": "0"}), "no such action"),
        (json!({"set": 0, "action": 0, "after": 3}), "no such action step"),
        (json!({"set": 0, "action": 0, "after": null}), "no such action step"),
        (json!({"set": 0, "action": 0, "after": -1}), "no such action step"),
    ] {
        assert_eq!(app.run(START_RECORDING, params.clone()).unwrap_err(), want, "{params}");
        assert!(app.ui.recording.is_none(), "{params}");
    }
    assert_eq!(app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 0})).unwrap(), json!({"set": 0, "action": 0, "name": "Three Moves"}));
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0).len(), 1, "no new action");
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 8", "object.move 16", "object.move 2", "object.move 4"]);
    assert_eq!(set_actions(&app, 0)[0].disabled, BTreeSet::from([0, 4]), "the steps keep whether they are excluded");
}

#[test]
fn steps_recorded_into_the_middle_of_an_action_keep_the_selected_step_and_dialog_on_their_steps() {
    let (mut app, _) = app_with_three_moves();
    let at = model::flat(&app.ui.actions, 0, 0).unwrap();
    app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 0})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
    app.ui.actions_panel.selected = Some(ActionSelection::Step(at, 2));
    crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(1));
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 8", "object.move 16", "object.move 2", "object.move 4"]);
    assert_eq!(app.ui.actions_panel.selected, Some(ActionSelection::Step(at, 4)), "the move by 4 stays selected");
    // PLAN: assumed menus-w20's `RecordStopDialog::json_fields` names the stop's place `step`, as the base dialog's field did.
    let fields = app.dialogs().and_then(|dialogs| dialogs.with(|host| host.get::<RecordStopDialog>().and_then(|d| d.json_fields())));
    assert_eq!(fields.map(|f| f["step"].clone()), Some(json!(3)), "Record Stop stays after the move by 2");
}

#[test]
fn step_edits_while_recording_into_an_action_keep_the_recording_in_place() {
    let (mut app, _) = app_with_three_moves();
    // The recording goes between the moves by 2 and 4.
    app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 1})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 2, "before": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(step_names(&app, 0, 0), ["object.move 4", "object.move 2", STOP_STEP, "object.move 8"]);

    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 1})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(step_names(&app, 0, 0), ["object.move 2", "object.move 8", "object.move 4"], "a step deleted before the place");
}

#[test]
fn a_stop_inserted_while_recording_goes_at_the_recording_position() {
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap();
    assert_eq!(app.run(INSERT_STOP, json!({"message": "Now", "allowContinue": true})).unwrap(), json!({"recording": true}));
    app.run("object.move", json!({"dx": 2, "dy": 0})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = set_actions(&app, 0).pop().unwrap();
    let ids: Vec<&str> = recorded.steps.iter().map(|s| s.command.as_str()).collect();
    assert_eq!(ids, ["object.move", STOP_STEP, "object.move"]);
    assert_eq!(recorded.steps[1].params, json!({"message": "Now", "allowContinue": true}));
}

/// A message step recorded into an action shows the message again when the action plays, and
/// adds no undo step.
#[test]
fn an_action_records_a_message_and_shows_it_again_when_played() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run(START_RECORDING, json!({})).unwrap();
    app.run("app.toast", json!({"message": "Step done", "level": "success"})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    // The recording is the set's last action.
    let actions = set_actions(&app, 0);
    let last = actions.len() - 1;
    assert_eq!(actions[last].steps, [ActionStep::new("app.toast", json!({"message": "Step done", "level": "success"}))]);
    app.session.take_toasts();
    let undo = undo_steps(&app);
    play(&mut app, 0, last).unwrap();
    assert_eq!(app.session.take_toasts(), vec![Toast::new("Step done", Level::Success)]);
    assert_eq!(undo_steps(&app), undo, "a message adds no undo step");
}
