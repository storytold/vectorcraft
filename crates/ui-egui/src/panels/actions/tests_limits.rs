use std::collections::BTreeSet;

use craft::automation::actions::{ActionColor, ActionStep};

use super::recording::recording_start;
use super::tests_support::*;
use super::*;
use crate::dialogs::action_set_options::SetOptionsDialog;

/// An action stays one that Load Actions… reads back: a stop or a menu item that would give it
/// more than 10,000 steps, or its set's file more than 8 MiB, is refused, and the action keeps its
/// steps and exclusions as they were.
#[test]
fn inserted_steps_keep_an_action_set_that_load_actions_reads() {
    let (mut app, _) = app_with_three_moves();
    let full = vec![ActionStep::new("object.group", json!({})); files::MAX_ACTION_STEPS];
    edit_set(&mut app, 0, |actions| actions[0].steps = full);
    let steps = "the action already has 10,000 steps";
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).unwrap_err(), steps);
    assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap_err(), steps);
    assert_eq!(set_actions(&app, 0)[0].steps.len(), files::MAX_ACTION_STEPS);
    edit_set(&mut app, 0, |actions| {
        actions[0].steps.pop();
    });
    assert!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).is_ok(), "the 10,000th step goes in");
    let saved = set_file(&app, 0);
    assert_eq!(files::read_set(&saved, "Test").unwrap().1[0].steps.len(), files::MAX_ACTION_STEPS, "and loads again");

    let pad = |n: usize| vec![ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}))];
    edit_set(&mut app, 0, |actions| {
        actions[0].steps = pad(0);
        actions[0].disabled = BTreeSet::from([0, 7]);
    });
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    edit_set(&mut app, 0, |actions| actions[0].steps = pad(room - 8));
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "object.group"})).unwrap_err(), size);
    assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap_err(), size);
    let action = set_actions(&app, 0).remove(0);
    assert_eq!((action.steps.len(), &action.disabled), (1, &BTreeSet::from([0, 7])), "nothing changed");
    assert!(set_file(&app, 0).len() <= files::MAX_SET_FILE);
}

/// The limits hold for a step inserted into the recording in progress too: they count the steps
/// recorded so far and, for New Action's recording, the action's own steps and its set.
#[test]
fn steps_inserted_into_a_recording_keep_an_action_set_that_load_actions_reads() {
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap();
    for _ in 1..files::MAX_ACTION_STEPS - 1 {
        app.session.journal.push(("object.group".to_string(), json!({})));
    }
    let steps = "the action already has 10,000 steps";
    assert_eq!(app.run(INSERT_STOP, json!({})).unwrap(), json!({"recording": true}), "the 10,000th step goes in");
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap_err(), steps);
    assert_eq!(app.run(INSERT_STOP, json!({})).unwrap_err(), steps);
    app.run(STOP_RECORDING, json!({})).unwrap();
    let saved = set_file(&app, 0);
    assert_eq!(files::read_set(&saved, "Test").unwrap().1.last().unwrap().steps.len(), files::MAX_ACTION_STEPS, "and loads again");

    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room - 8)]);
    app.ui.recording_into = Some(set_actions(&app, 0)[0].name.clone());
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap_err(), size);
    assert_eq!(app.run(INSERT_STOP, json!({})).unwrap_err(), size);
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0)[0].steps.len(), 1, "nothing was recorded into the action");
    assert!(set_file(&app, 0).len() <= files::MAX_SET_FILE);
}

/// A step put by its set and action into the action that the recording in progress records into
/// counts the steps recorded so far, which Stop Playing/Recording adds to that action.
#[test]
fn steps_inserted_into_the_action_being_recorded_into_count_the_recorded_steps() {
    let (mut app, _) = app_with_three_moves();
    edit_set(&mut app, 0, |actions| actions.push(Action::new("Other", vec![])));
    let into = set_actions(&app, 0)[0].name.clone();
    let stored = set_actions(&app, 0)[0].steps.len();
    app.run(START_RECORDING, json!({})).unwrap();
    app.ui.recording_into = Some(into);
    for _ in stored..files::MAX_ACTION_STEPS {
        app.session.journal.push(("object.group".to_string(), json!({})));
    }
    let steps = "the action already has 10,000 steps";
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).unwrap_err(), steps);
    assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap_err(), steps);
    assert_eq!(set_actions(&app, 0)[0].steps.len(), stored, "nothing was inserted");
    assert!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 1, "command": "object.group"})).is_ok(), "another action takes the step");
    app.run(STOP_RECORDING, json!({})).unwrap();
    let saved = set_file(&app, 0);
    assert_eq!(files::read_set(&saved, "Test").unwrap().1[0].steps.len(), files::MAX_ACTION_STEPS, "and loads again");

    let (mut app, _) = app_with_three_moves();
    let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    let into = set_actions(&app, 0)[0].name.clone();
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    app.run(START_RECORDING, json!({})).unwrap();
    app.ui.recording_into = Some(into);
    app.session.journal.push(pad(room - 8));
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).unwrap_err(), size);
    assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap_err(), size);
    // Another action of the set counts them too, for a recording into an action and for a
    // recording that adds a new action.
    for into_first in [true, false] {
        let (mut app, _) = app_with_three_moves();
        edit_set(&mut app, 0, |actions| actions.push(Action::new("Other", vec![])));
        let first = set_actions(&app, 0)[0].name.clone();
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = into_first.then_some(first);
        app.session.journal.push(pad(room - 8));
        let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 1, "command": "object.group"}));
        assert_eq!(r.unwrap_err(), size, "recording into the first action: {into_first}");
        assert!(set_actions(&app, 0)[1].steps.is_empty(), "recording into the first action: {into_first}");
    }
}

/// The params of `object.move` by 1 as the journal keeps them, which a recording records.
fn recorded_move(app: &mut VectorcraftApp) -> Value {
    app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap();
    let moved = app.session.journal.last().unwrap().1.clone();
    app.run("edit.undo", json!({})).unwrap();
    moved
}

/// An ordinary command recorded as the 10,001st step of an action does not run: the document,
/// the journal and the undo history stay as they were, the recording goes on, and Stop
/// Playing/Recording leaves a set that Load Actions… reads. This holds for Begin Recording's new
/// action and for New Action's recording into an action with steps of its own.
#[test]
fn recorded_commands_keep_an_action_at_10_000_steps() {
    for new_action in [false, true] {
        let (mut app, x) = app_with_three_moves();
        let moved = recorded_move(&mut app);
        let filler = ("object.group".to_string(), json!({}));
        if new_action {
            app.run(NEW_ACTION, json!({"set": 0, "name": "Long"})).unwrap();
            let steps = vec![ActionStep::new(filler.0.clone(), filler.1.clone()); files::MAX_ACTION_STEPS - 1];
            edit_set(&mut app, 0, |actions| actions[1].steps = steps);
        } else {
            app.run(START_RECORDING, json!({})).unwrap();
            for _ in 1..files::MAX_ACTION_STEPS {
                app.session.journal.push(filler.clone());
            }
        }
        app.run("object.move", moved.clone()).unwrap();
        assert_eq!(x(&app), 1.0, "the 10,000th step runs");
        let (journal, undo) = (app.session.journal.len(), undo_steps(&app));
        let steps = "the action already has 10,000 steps";
        assert_eq!(app.run("object.move", moved.clone()).unwrap_err(), steps, "new action: {new_action}");
        assert_eq!((x(&app), app.session.journal.len(), undo_steps(&app)), (1.0, journal, undo), "nothing ran");
        assert_eq!(app.ui.status, steps);
        assert!(app.ui.recording.is_some(), "the recording goes on");
        assert!(app.run("effect.list", json!({})).is_ok(), "a command that is not recorded still runs");
        let find = |select: bool| json!({"minWidth": 0, "select": select});
        assert_eq!(app.run("document.find", find(true)).unwrap_err(), steps, "Find Objects that selects is recorded");
        assert!(app.run("document.find", find(false)).is_ok());
        let batch = json!({"commands": [{"command": "object.move", "params": moved.clone()}]});
        assert_eq!(app.run("command.batch", batch).unwrap_err(), steps, "a batch is recorded");
        assert_eq!(x(&app), 1.0);
        assert_eq!(app.session.journal.len(), journal);
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(set_actions(&app, 0)[1].steps.len(), files::MAX_ACTION_STEPS);
        assert_eq!(set_actions(&app, 0)[1].steps.last(), Some(&ActionStep::new("object.move", moved)));
        let saved = set_file(&app, 0);
        assert_eq!(files::read_set(&saved, "Test").unwrap().1[1].steps.len(), files::MAX_ACTION_STEPS, "and loads again");
    }
}

/// An ordinary command whose step would give the set of the action it is recorded into a file
/// larger than 8 MiB does not run, and one that makes the file exactly 8 MiB does.
#[test]
fn recorded_commands_keep_an_action_set_file_within_8_mib() {
    let (mut app, x) = app_with_three_moves();
    let moved = recorded_move(&mut app);
    let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    let added = files::step_bytes("object.move", &moved);
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room - added)]);
    let into = set_actions(&app, 0)[0].name.clone();
    app.run(START_RECORDING, json!({})).unwrap();
    app.ui.recording_into = Some(into);
    app.run("object.move", moved.clone()).unwrap();
    assert_eq!(x(&app), 1.0, "the step that fills the file runs");
    let (journal, undo) = (app.session.journal.len(), undo_steps(&app));
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.run("object.move", moved).unwrap_err(), size);
    assert_eq!((x(&app), app.session.journal.len(), undo_steps(&app)), (1.0, journal, undo), "nothing ran");
    assert!(app.ui.recording.is_some(), "the recording goes on");
    app.ui.status.clear();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(app.ui.status, "", "Stop keeps every recorded step");
    let saved = set_file(&app, 0);
    assert_eq!(saved.len(), files::MAX_SET_FILE);
    assert_eq!(files::read_set(&saved, "Test").unwrap().1[0].steps.len(), 2, "and loads again");
}

/// A command whose step fits by its params but not with the values it adds to its step as it
/// runs (Scale's Scale Strokes & Effects and Scale Corners) keeps its edit, the status bar names
/// the limit, the next recorded command does not run, and Stop Playing/Recording leaves the step
/// out.
#[test]
fn a_recorded_command_whose_noted_values_do_not_fit_is_left_out() {
    let (mut app, x) = app_with_three_moves();
    let scaled = json!({"sx": 200});
    app.run("object.scale", scaled.clone()).unwrap();
    let noted = app.session.journal.last().unwrap().1.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert!(files::step_bytes("object.scale", &noted) > files::step_bytes("object.scale", &scaled), "Scale notes values: {noted}");
    let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    // Room for the step as given, not for the step as recorded.
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room - files::step_bytes("object.scale", &scaled))]);
    let into = set_actions(&app, 0)[0].name.clone();
    app.run(START_RECORDING, json!({})).unwrap();
    app.ui.recording_into = Some(into);
    let size = "the action set would be larger than 8 MiB";
    app.run("object.scale", scaled).unwrap();
    assert_eq!(app.ui.status, size);
    let scaled_x = x(&app);
    assert_ne!(scaled_x, 0.0, "the scale stayed");
    assert_eq!(app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap_err(), size, "no later command runs");
    assert_eq!(x(&app), scaled_x);
    assert!(app.ui.recording.is_some(), "the recording goes on");
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0)[0].steps.len(), 1, "the scale's step is left out");
    assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());
}

/// A script that moves the selection twice.
const TWO_MOVES: &str = "vc.run('object.move', {dx: 1, dy: 0}); vc.run('object.move', {dx: 1, dy: 0});";

/// Runs `source` to its end: in one call (`frames` false, a run with nobody to ask), or a frame
/// at a time, as the app does.
fn run_script(app: &mut VectorcraftApp, source: &str, frames: bool) {
    app.session.script_frames = frames;
    app.run("script.run", json!({"source": source, "name": "t.js"})).unwrap();
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return;
        }
        crate::script_runs::poll(app);
    }
    panic!("the script did not end");
}

/// Begin Recording with `filler` steps already recorded.
fn recording_with(app: &mut VectorcraftApp, filler: usize) {
    app.run(START_RECORDING, json!({})).unwrap();
    for _ in 0..filler {
        app.session.journal.push(("object.group".to_string(), json!({})));
    }
}

/// A script whose commands take a recording past 10,000 steps keeps its edit and the whole
/// journal, the status bar names the limit as the run ends, the next recorded command does not
/// run, and Stop Playing/Recording keeps the 10,000 steps that fit. One that fills the action
/// exactly names no limit.
#[test]
fn script_run_checks_recording_step_limit_on_completion() {
    let steps = "the action already has 10,000 steps";
    for frames in [false, true] {
        let (mut app, x) = app_with_three_moves();
        recording_with(&mut app, files::MAX_ACTION_STEPS - 1);
        app.ui.status.clear();
        run_script(&mut app, TWO_MOVES, frames);
        assert_eq!(x(&app), 2.0, "frames {frames}: the script's edit stays");
        let start = recording_start(&app).unwrap();
        assert_eq!(app.session.journal.len() - start, files::MAX_ACTION_STEPS + 1, "frames {frames}: the journal keeps every step");
        assert_eq!(app.ui.status, steps, "frames {frames}: the status bar names the limit now");
        assert!(app.ui.recording.is_some(), "the recording goes on");
        assert_eq!(app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap_err(), steps, "no later command runs");
        assert_eq!(x(&app), 2.0);
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = set_actions(&app, 0).pop().unwrap();
        assert_eq!(recorded.steps.len(), files::MAX_ACTION_STEPS);
        assert_eq!(recorded.steps.last().map(|s| s.command.as_str()), Some("object.move"), "the first move fits");
        let saved = set_file(&app, 0);
        assert_eq!(files::read_set(&saved, "Test").unwrap().1[1].steps.len(), files::MAX_ACTION_STEPS, "and loads again");

        let (mut app, _) = app_with_three_moves();
        recording_with(&mut app, files::MAX_ACTION_STEPS - 2);
        app.ui.status.clear();
        run_script(&mut app, TWO_MOVES, frames);
        let ran = if frames { "Ran t.js" } else { "" };
        assert_eq!(app.ui.status, ran, "frames {frames}: 10,000 steps fit");
    }
}

/// A script whose commands give the recording's set a file larger than 8 MiB keeps its edit,
/// the status bar names the limit as the run ends, and Stop Playing/Recording keeps the steps
/// that fit in a set that Load Actions… reads. One that fills the file exactly names no limit.
#[test]
fn script_run_checks_recording_byte_limit_on_completion() {
    let size = "the action set would be larger than 8 MiB";
    for frames in [false, true] {
        for spare in [0, 1] {
            let (mut app, x) = app_with_three_moves();
            // The two steps as an action keeps them.
            let before = app.session.journal.len();
            run_script(&mut app, TWO_MOVES, false);
            let both: usize = app.session.journal_for_action(before).iter().map(|(c, p)| files::step_bytes(c, p)).sum();
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(x(&app), 0.0);
            let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
            edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
            let room = files::MAX_SET_FILE - set_file(&app, 0).len();
            // Room for both moves, or for one move and all but a byte of the other.
            edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room - both + 1 - spare)]);
            let into = set_actions(&app, 0)[0].name.clone();
            app.run(START_RECORDING, json!({})).unwrap();
            app.ui.recording_into = Some(into);
            app.ui.status.clear();
            run_script(&mut app, TWO_MOVES, frames);
            assert_eq!(x(&app), 2.0, "frames {frames}: the script's edit stays");
            let what = format!("frames {frames}, spare {spare}");
            if spare == 1 {
                assert_eq!(app.ui.status, if frames { "Ran t.js" } else { "" }, "{what}: the file is exactly 8 MiB");
                continue;
            }
            assert_eq!(app.ui.status, size, "{what}: the status bar names the limit now");
            assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), 2, "the journal keeps both moves");
            assert_eq!(app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap_err(), size, "no later command runs");
            app.run(STOP_RECORDING, json!({})).unwrap();
            assert_eq!(set_actions(&app, 0)[0].steps.len(), 2, "{what}: the first move fits");
            let saved = set_file(&app, 0);
            assert!(saved.len() <= files::MAX_SET_FILE);
            assert!(files::read_set(&saved, "Test").is_ok());
        }
    }
}

/// Presses, drags and releases the active tool from `from` to `to`, as the canvas hands its
/// results on ([`crate::canvas::apply_requests`]).
fn tool_drag(app: &mut VectorcraftApp, from: (f64, f64), to: (f64, f64)) {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let view = app.view_info();
    for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        let r = app.session.pointer(&PointerEvent::new(kind, x, y), view);
        crate::canvas::apply_requests(app, r);
    }
}

/// A drag with the Selection tool or the Rectangle tool (each commits its preview) that takes a
/// recording past 10,000 steps keeps its edit and the journal, the status bar names the limit
/// as the tool ends, and Stop Playing/Recording keeps the 10,000 steps that fit. One that fills
/// the action exactly names no limit.
#[test]
fn tool_commit_checks_recording_step_limit() {
    let steps = "the action already has 10,000 steps";
    for tool in ["selection", "rectangle"] {
        for filler in [files::MAX_ACTION_STEPS, files::MAX_ACTION_STEPS - 1] {
            let (mut app, _) = app_with_three_moves();
            app.select_tool(tool);
            recording_with(&mut app, filler);
            app.ui.status.clear();
            let (journal, art) = (app.session.journal.len(), app.session.active().unwrap().doc.art_bounds());
            tool_drag(&mut app, (20.0, 20.0), (60.0, 50.0));
            assert_eq!(app.session.journal.len(), journal + 1, "{tool}: the tool journals one step");
            assert_ne!(app.session.active().unwrap().doc.art_bounds(), art, "{tool}: the edit stays");
            if filler < files::MAX_ACTION_STEPS {
                assert_eq!(app.ui.status, "", "{tool}: 10,000 steps fit");
                continue;
            }
            assert_eq!(app.ui.status, steps, "{tool}: the status bar names the limit now");
            assert!(app.ui.recording.is_some(), "the recording goes on");
            app.run(STOP_RECORDING, json!({})).unwrap();
            assert_eq!(set_actions(&app, 0).last().unwrap().steps.len(), files::MAX_ACTION_STEPS);
        }
    }
}

/// A tool's edit whose step gives the recording's set a file larger than 8 MiB keeps its edit,
/// the status bar names the limit as the tool ends, and Stop Playing/Recording leaves the step
/// out of a set that Load Actions… reads.
#[test]
fn tool_commit_checks_recording_byte_limit() {
    let size = "the action set would be larger than 8 MiB";
    for tool in ["selection", "rectangle"] {
        let (mut app, _) = app_with_three_moves();
        app.select_tool(tool);
        app.run(START_RECORDING, json!({})).unwrap();
        let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
        let room = files::MAX_SET_FILE - set_file(&app, 0).len();
        edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room)]);
        app.ui.recording_into = Some(set_actions(&app, 0)[0].name.clone());
        app.ui.status.clear();
        let art = app.session.active().unwrap().doc.art_bounds();
        tool_drag(&mut app, (20.0, 20.0), (60.0, 50.0));
        assert_ne!(app.session.active().unwrap().doc.art_bounds(), art, "{tool}: the edit stays");
        assert_eq!(app.ui.status, size, "{tool}: the status bar names the limit now");
        assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), 1, "{tool}: the journal keeps the step");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(set_actions(&app, 0)[0].steps.len(), 1, "{tool}: the step is left out");
        assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());
    }
}

/// Drags the Enclosing Lasso away from the selected rectangle, which runs `select.set` with no
/// objects, and checks that the tool journaled that one step and cleared the selection.
fn lasso_nothing(app: &mut VectorcraftApp) {
    app.select_tool(vectorcraft_tools::addons::enclosing_lasso::ID);
    assert_eq!(app.session.tool_id(), vectorcraft_tools::addons::enclosing_lasso::ID);
    assert!(!app.session.active().unwrap().selection.objects.is_empty());
    let journal = app.session.journal.len();
    tool_drag(app, (100.0, 80.0), (120.0, 80.0));
    assert_eq!(app.session.journal.len(), journal + 1, "the tool journals one step");
    assert_eq!(app.session.journal.last(), Some(&("select.set".to_string(), json!({"ids": []}))));
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "the edit stays");
}

/// A command a tool runs (the Enclosing Lasso's `select.set`) that takes a recording past
/// 10,000 steps keeps its edit and the journal, the status bar names the limit as the tool
/// ends, the next recorded command does not run, and Stop Playing/Recording keeps the 10,000
/// steps that fit. One that fills the action exactly names no limit.
#[test]
fn tool_exec_checks_recording_step_limit() {
    let steps = "the action already has 10,000 steps";
    for filler in [files::MAX_ACTION_STEPS, files::MAX_ACTION_STEPS - 1] {
        let (mut app, _) = app_with_three_moves();
        recording_with(&mut app, filler);
        app.ui.status.clear();
        lasso_nothing(&mut app);
        if filler < files::MAX_ACTION_STEPS {
            assert_eq!(app.ui.status, "", "10,000 steps fit");
            continue;
        }
        assert_eq!(app.ui.status, steps, "the status bar names the limit now");
        assert!(app.ui.recording.is_some(), "the recording goes on");
        let (journal, art) = (app.session.journal.len(), app.session.active().unwrap().doc.art_bounds());
        let r = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 20, "height": 20}));
        assert_eq!(r.unwrap_err(), steps, "no later command runs");
        assert_eq!(app.session.journal.len(), journal);
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), art, "no rectangle is drawn");
        assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), files::MAX_ACTION_STEPS + 1, "the journal keeps the step");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(set_actions(&app, 0).last().unwrap().steps.len(), files::MAX_ACTION_STEPS);
    }
}

/// A command a tool runs (the Enclosing Lasso's `select.set`) whose step gives the recording's
/// set a file larger than 8 MiB keeps its edit, the status bar names the limit as the tool
/// ends, and Stop Playing/Recording leaves the step out of a set that Load Actions… reads.
#[test]
fn tool_exec_checks_recording_byte_limit() {
    let size = "the action set would be larger than 8 MiB";
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room)]);
    app.ui.recording_into = Some(set_actions(&app, 0)[0].name.clone());
    app.ui.status.clear();
    lasso_nothing(&mut app);
    assert_eq!(app.ui.status, size, "the status bar names the limit now");
    assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), 1, "the journal keeps the step");
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0)[0].steps.len(), 1, "the step is left out");
    let saved = set_file(&app, 0);
    assert!(saved.len() <= files::MAX_SET_FILE);
    assert!(files::read_set(&saved, "Test").is_ok());
}

/// The place cursor's `file.place` that takes a recording past 10,000 steps keeps the placed
/// file and the journal, and the status bar names the limit once the file is placed: on the
/// release when the file is read at once, and on the frame that places it when it is read off
/// the UI thread. Stop Playing/Recording keeps the 10,000 steps that fit.
#[test]
fn place_cursor_exec_checks_recording_step_limit_on_completion() {
    let steps = "the action already has 10,000 steps";
    let raw =
        craft_image_codecs::Image::from_raw(8, 8, craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, vec![0; 8 * 8 * 3])
            .unwrap();
    let png = craft_image_codecs::encode(&raw, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
    let placed = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.layers.iter().flat_map(|l| l.children().into_iter().flatten()).filter(|n| n.name.as_deref() == Some("a.png")).count()
    };
    for background in [false, true] {
        let (mut app, _) = app_with_three_moves();
        app.placing.off_the_ui_thread = background;
        app.run("file.place.queue", json!({"files": [{"name": "a.png", "dataBase64": vectorcraft_format::base64_encode(&png)}]})).unwrap();
        assert_eq!(app.session.tool_id(), "place");
        recording_with(&mut app, files::MAX_ACTION_STEPS);
        app.ui.status.clear();
        let journal = app.session.journal.len();
        tool_drag(&mut app, (40.0, 40.0), (80.0, 80.0));
        if background {
            assert_eq!(app.session.journal.len(), journal, "the release places nothing yet");
            assert_eq!(placed(&app), 0);
            assert_ne!(app.ui.status, steps, "nothing is checked before the file is placed");
            // The read runs on the place lane; a frame after it ends places the file.
            let ctx = crate::testing::ui_context();
            crate::placing::wait(&mut app);
            craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
            ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx())).textures_delta.clear();
            assert!(app.placing.pending.is_empty(), "a frame after the read places the file");
        }
        assert_eq!(app.session.journal.len(), journal + 1, "background {background}: the place journals one step");
        assert_eq!(app.session.journal.last().map(|(c, _)| c.as_str()), Some("file.place"));
        assert_eq!(placed(&app), 1, "background {background}: the placed file stays");
        assert_eq!(app.ui.status, steps, "background {background}: the status bar names the limit once the file is placed");
        assert!(app.ui.recording.is_some(), "the recording goes on");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(set_actions(&app, 0).last().unwrap().steps.len(), files::MAX_ACTION_STEPS, "background {background}");
    }
}

/// A step that reaches the journal outside a command, as a panel's or a dialog's commit does, is
/// checked on the app's next frame. A journal rewrite that makes a checked step larger is
/// checked again, and a frame with no new step does not name the limit again.
#[test]
fn a_journal_rewrite_or_a_direct_commit_is_checked_on_the_next_frame() {
    let size = "the action set would be larger than 8 MiB";
    let (mut app, _) = app_with_three_moves();
    let small = ("object.move".to_string(), json!({"dx": 1, "dy": 0}));
    let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    let step = |(command, params): (String, Value)| ActionStep::new(command, params);
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![step(pad(0))]);
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![step(pad(room - files::step_bytes(&small.0, &small.1)))]);
    let into = set_actions(&app, 0)[0].name.clone();
    app.run(START_RECORDING, json!({})).unwrap();
    app.ui.recording_into = Some(into);
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp| {
        craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
    };
    app.session.journal.push(small);
    app.ui.status.clear();
    frame(&mut app);
    assert_eq!(app.ui.status, "", "a step that fills the file exactly");
    let start = recording_start(&app).unwrap();
    app.session.journal.replace(start, pad(1));
    frame(&mut app);
    assert_eq!(app.ui.status, size, "the rewritten step no longer fits");
    app.ui.status.clear();
    frame(&mut app);
    assert_eq!(app.ui.status, "", "named once");
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0)[0].steps.len(), 1, "the step is left out");
}

/// When the action New Action made for the recording in progress is deleted, Stop
/// Playing/Recording keeps none of its steps, so no limit stops a recorded command, even with
/// the set's file at 8 MiB.
#[test]
fn a_deleted_recording_action_does_not_limit_recorded_commands() {
    let (mut app, x) = app_with_three_moves();
    edit_set(&mut app, 0, |actions| actions.push(Action::new("Gone", vec![])));
    app.run(START_RECORDING, json!({})).unwrap();
    app.ui.recording_into = Some("Gone".into());
    edit_set(&mut app, 0, |actions| actions.retain(|a| a.name != "Gone"));
    let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(0)]);
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    edit_set(&mut app, 0, |actions| actions[0].steps = vec![pad(room)]);
    let before = x(&app);
    app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap();
    assert_ne!(x(&app), before, "the move ran");
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0).len(), 1, "no action is added");
    assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());
}

/// A stop that playback reaches while a full action records is not recorded: the status bar shows
/// the reason, and the playback still waits at the stop.
#[test]
fn a_stop_reached_while_recording_a_full_action_is_left_out() {
    let (mut app, _) = app_with_a_stop(false);
    app.run(START_RECORDING, json!({})).unwrap();
    for _ in 1..files::MAX_ACTION_STEPS {
        app.session.journal.push(("object.group".to_string(), json!({})));
    }
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    assert_eq!(app.ui.status, "the action already has 10,000 steps");
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = set_actions(&app, 0).pop().unwrap();
    assert_eq!(recorded.steps.len(), files::MAX_ACTION_STEPS);
    assert_eq!(recorded.steps.last().map(|s| s.command.as_str()), Some("command.batch"));
}

/// Steps that reach the journal without a command's check are left out by Stop Playing/Recording
/// past either limit, and the status bar names the limit.
#[test]
fn stop_leaves_out_recorded_steps_past_the_limits() {
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    for _ in 0..files::MAX_ACTION_STEPS + 5 {
        app.session.journal.push(("object.group".to_string(), json!({})));
    }
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(app.ui.status, "the action already has 10,000 steps");
    assert_eq!(set_actions(&app, 0).last().unwrap().steps.len(), files::MAX_ACTION_STEPS);

    let (mut app, _) = app_with_three_moves();
    let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    app.run(START_RECORDING, json!({})).unwrap();
    app.session.journal.push(pad(files::MAX_SET_FILE / 2));
    app.session.journal.push(pad(files::MAX_SET_FILE / 2));
    app.session.journal.push(pad(0));
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(app.ui.status, "the action set would be larger than 8 MiB");
    assert_eq!(set_actions(&app, 0).last().unwrap().steps.len(), 1);
    assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());
}

/// An exclusion that would give a set a file larger than 8 MiB is refused for a set, an action,
/// a step and the other steps of an action, and leaves every exclusion as it was. One that
/// fills the file exactly, one that changes nothing and an inclusion still work, also in a set
/// that is already larger than 8 MiB.
#[test]
fn excluding_steps_keeps_an_action_set_file_within_8_mib() {
    let (mut app, _) = app_with_three_moves();
    let pad = |n: usize| ActionStep::new("object.move", json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
    edit_set(&mut app, 0, |actions| actions.push(Action::new("Pad", vec![pad(0)])));
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    // Room for two more excluded steps.
    edit_set(&mut app, 0, |actions| actions[1].steps = vec![pad(room - 2 * files::EXCLUDED_BYTES)]);
    let size = "the action set would be larger than 8 MiB";
    let include = |app: &mut VectorcraftApp, p: Value| app.run(INCLUDE_STEPS, p);
    let excluded = |app: &VectorcraftApp| set_actions(app, 0).into_iter().map(|a| a.disabled).collect::<Vec<_>>();
    let none = vec![BTreeSet::new(), BTreeSet::new()];
    assert_eq!(include(&mut app, json!({"set": 0, "included": false})).unwrap_err(), size);
    assert_eq!(include(&mut app, json!({"set": 0, "action": 0, "included": false})).unwrap_err(), size);
    assert_eq!(excluded(&app), none, "nothing changed");
    // Two steps fill the file to 8 MiB.
    include(&mut app, json!({"set": 0, "action": 0, "step": 0, "included": false, "others": true})).unwrap();
    assert_eq!(excluded(&app), [BTreeSet::from([1, 2]), BTreeSet::new()]);
    assert_eq!(set_file(&app, 0).len(), files::MAX_SET_FILE);
    assert_eq!(include(&mut app, json!({"set": 0, "action": 0, "step": 0, "included": false})).unwrap_err(), size);
    assert_eq!(include(&mut app, json!({"set": 0, "action": 1, "step": 0, "included": false})).unwrap_err(), size);
    assert!(include(&mut app, json!({"set": 0, "action": 0, "step": 1, "included": false})).is_ok(), "no change");
    include(&mut app, json!({"set": 0, "action": 0, "step": 1, "included": true})).unwrap();
    include(&mut app, json!({"set": 0, "action": 0, "step": 0, "included": false})).unwrap();
    assert_eq!(excluded(&app), [BTreeSet::from([0, 2]), BTreeSet::new()]);
    assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());

    // A set already larger than 8 MiB can still include its steps.
    edit_set(&mut app, 0, |actions| actions[1].steps[0] = pad(room));
    assert_eq!(include(&mut app, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap_err(), size);
    include(&mut app, json!({"set": 0, "included": true})).unwrap();
    assert_eq!(excluded(&app), none);

    // The steps a recording adds to the set count too.
    let (mut app, _) = app_with_three_moves();
    let room = files::MAX_SET_FILE - set_file(&app, 0).len();
    app.run(START_RECORDING, json!({})).unwrap();
    let padded = pad(room - files::step_bytes("object.move", &pad(0).params));
    app.session.journal.push((padded.command, padded.params));
    assert_eq!(include(&mut app, json!({"set": 0, "included": false})).unwrap_err(), size);
    assert!(set_actions(&app, 0)[0].disabled.is_empty());
}

/// Adds an action to set `set` whose step makes the set's file `short` bytes short of 8 MiB.
fn pad_set_to(app: &mut VectorcraftApp, set: usize, short: usize) {
    let pad = |n: usize| ActionStep::new("object.move", json!({"pad": "x".repeat(n)}));
    edit_set(app, set, |actions| actions.push(Action::new("Pad", vec![pad(0)])));
    let room = files::MAX_SET_FILE - set_file(app, set).len();
    edit_set(app, set, |actions| actions.last_mut().unwrap().steps[0] = pad(room - short));
    assert_eq!(set_file(app, set).len(), files::MAX_SET_FILE - short);
}

/// A recorded batch that edits another document, and whose values make its step too large only
/// as it runs, keeps its edits in both documents: the status bar names the limit, the next
/// recorded command does not run, and Stop Playing/Recording leaves the batch's step out.
#[test]
fn a_recorded_batch_that_edits_another_document_is_left_out_when_it_grows() {
    // Three moves' document, a move as recorded there, and a second document that is active.
    let two_documents = || {
        let (mut app, x) = app_with_three_moves();
        let moved = recorded_move(&mut app);
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        (app, x, moved)
    };
    let batch = json!({"commands": [
        {"command": "document.activate", "params": {"index": 0}},
        {"command": "object.scale", "params": {"sx": 200}},
    ]});
    let (mut app, _, _) = two_documents();
    app.run("command.batch", batch.clone()).unwrap();
    let noted = app.session.journal.last().unwrap().1.clone();
    // The batch as the app runs it (with the active artboard) before the scale notes its values.
    let mut sent = noted.clone();
    sent["commands"][1]["params"] = json!({"sx": 200});
    let given = files::step_bytes("command.batch", &sent);
    assert!(files::step_bytes("command.batch", &noted) > given, "the scale notes values: {noted}");

    let (mut app, x, moved) = two_documents();
    pad_set_to(&mut app, 0, given);
    let into = set_actions(&app, 0).pop().unwrap().name;
    app.run(START_RECORDING, json!({"set": 0})).unwrap();
    app.ui.recording_into = Some(into);
    app.run("command.batch", batch).unwrap();
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.ui.status, size);
    let scaled = x(&app);
    assert_ne!(scaled, 0.0, "the scale stayed");
    assert_eq!(app.run("object.move", moved).unwrap_err(), size, "no later command runs");
    assert_eq!(x(&app), scaled);
    let pad = set_actions(&app, 0).last().unwrap().steps.len();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0).last().unwrap().steps.len(), pad, "the batch's step is left out");
    assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());
}

/// Set Options… renaming the set that an action records into has the next recorded command
/// measure the set's file again: a step that fit before the longer name does not run after it.
#[test]
fn a_set_renamed_while_it_records_is_measured_again() {
    let (mut app, x) = app_with_three_moves();
    let moved = recorded_move(&mut app);
    let added = files::step_bytes("object.move", &moved);
    pad_set_to(&mut app, 0, 2 * added + 3);
    let into = set_actions(&app, 0).pop().unwrap().name;
    app.run(START_RECORDING, json!({"set": 0})).unwrap();
    app.ui.recording_into = Some(into);
    app.run("object.move", moved.clone()).unwrap();
    crate::dialogs::action_set_options::open(&mut app, 0);
    let longer = format!("{} 1234", set_names(&app)[0]);
    set_dialog::<SetOptionsDialog>(&mut app, "name", json!(longer));
    confirm_dialog::<SetOptionsDialog>(&mut app).unwrap();
    assert_eq!(set_names(&app)[0], longer);
    assert_eq!(app.run("object.move", moved).unwrap_err(), "the action set would be larger than 8 MiB");
    assert_eq!(x(&app), 1.0, "only the first move ran");
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert!(files::read_set(&set_file(&app, 0), "Test").is_ok());
}

/// Steps recorded into an action are included in playback, also when the action holds an
/// exclusion past its last step, as saved preferences can; the set's file stays within 8 MiB.
#[test]
fn steps_recorded_after_an_exclusion_past_the_last_step_are_included() {
    let (mut app, _) = app_with_three_moves();
    let moved = recorded_move(&mut app);
    let added = files::step_bytes("object.move", &moved);
    pad_set_to(&mut app, 0, added);
    edit_set(&mut app, 0, |actions| {
        actions.last_mut().unwrap().disabled.insert(1);
    });
    let into = set_actions(&app, 0).pop().unwrap().name;
    app.run(START_RECORDING, json!({"set": 0})).unwrap();
    app.ui.recording_into = Some(into);
    app.run("object.move", moved).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    let pad = set_actions(&app, 0).pop().unwrap();
    assert_eq!((pad.steps.len(), pad.disabled.len()), (2, 0), "the recorded step is included");
    assert_eq!(set_file(&app, 0).len(), files::MAX_SET_FILE);
}

/// Duplicate copies no action or set when the copy would give a file larger than 8 MiB or a set
/// of more than 1,000 actions, and still copies one that fits.
#[test]
fn duplicate_keeps_action_set_files_within_their_limits() {
    let mut app = app_with_sets();
    pad_set_to(&mut app, 1, 0);
    let size = "the action set would be larger than 8 MiB";
    let names = set_names(&app);
    assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 0})).unwrap_err(), size);
    assert_eq!(app.run(DUPLICATE, json!({"set": 1})).unwrap_err(), size, "the copy's name is longer");
    assert_eq!((set_names(&app), counts(&app)[1]), (names, 3), "nothing was copied");
    edit_set(&mut app, 2, |actions| *actions = empty_actions(1000));
    let many = "the action set file holds more than 1,000 actions";
    assert_eq!(app.run(DUPLICATE, json!({"set": 2, "action": 0})).unwrap_err(), many);
    assert_eq!(counts(&app)[2], 1000);
    edit_set(&mut app, 2, |actions| {
        actions.pop();
    });
    app.run(DUPLICATE, json!({"set": 2, "action": 0})).unwrap();
    app.run(DUPLICATE, json!({"set": 2})).unwrap();
    assert_eq!((counts(&app)[2], counts(&app)[3]), (1000, 1000));
}

/// New Action and Begin Recording add no action and start no recording when the action would
/// give its set more than 1,000 actions or a file larger than 8 MiB, and New Action adds one that
/// fits.
#[test]
fn new_action_keeps_an_action_set_file_within_its_limits() {
    let mut app = app_with_sets();
    edit_set(&mut app, 2, |actions| *actions = empty_actions(1000));
    let many = "the action set file holds more than 1,000 actions";
    assert_eq!(app.run(NEW_ACTION, json!({"set": 2, "name": "More"})).unwrap_err(), many);
    assert_eq!(app.run(START_RECORDING, json!({"set": 2})).unwrap_err(), many, "Begin Recording too");
    pad_set_to(&mut app, 1, 200);
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.run(NEW_ACTION, json!({"set": 1, "name": "x".repeat(300)})).unwrap_err(), size);
    assert!(app.ui.recording.is_none(), "no recording started");
    assert_eq!((counts(&app)[1], counts(&app)[2]), (3, 1000));
    app.run(NEW_ACTION, json!({"set": 1, "name": "More"})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(counts(&app)[1], 4);
    assert!(set_file(&app, 1).len() <= files::MAX_SET_FILE);
}

/// Renaming an action, Action Options and Set Options change nothing when the change would give
/// the set a file larger than 8 MiB, with the steps a recording adds to the set; a change that
/// fills the file exactly, or that makes a set already past 8 MiB smaller, is made. New Set adds
/// no set whose file would be larger than 8 MiB. A dialog whose OK is refused stays open with the
/// reason.
#[test]
fn renaming_keeps_an_action_set_file_within_8_mib() {
    let mut app = app_with_sets();
    pad_set_to(&mut app, 1, 2);
    let size = "the action set would be larger than 8 MiB";
    assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudge Far"})).unwrap_err(), size);
    assert_eq!(app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "color": "red"})).unwrap_err(), size);
    assert_eq!(app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 2})).unwrap_err(), size);
    let nudge = set_actions(&app, 1).remove(0);
    let unchanged = (nudge.name.as_str(), model::function_key(&nudge).is_none(), nudge.color);
    assert_eq!(unchanged, ("Nudge", true, ActionColor::None), "nothing changed");
    crate::dialogs::action_set_options::open(&mut app, 1);
    set_dialog::<SetOptionsDialog>(&mut app, "name", json!("Mine and More"));
    assert_eq!(confirm_dialog::<SetOptionsDialog>(&mut app).unwrap_err(), size);
    assert!(is_open::<SetOptionsDialog>(&app), "Set Options stays open");
    assert_eq!(set_names(&app)[1], "Mine");
    cancel_dialog::<SetOptionsDialog>(&mut app);
    app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudge++"})).unwrap();
    assert_eq!(set_file(&app, 1).len(), files::MAX_SET_FILE, "the file is exactly 8 MiB");
    edit_set(&mut app, 1, |actions| actions[1].name = "x".repeat(100));
    app.run(RENAME_ACTION, json!({"set": 1, "action": 1, "name": "Nudge Again"})).unwrap();
    assert_eq!(set_actions(&app, 1)[1].name, "Nudge Again", "a set past 8 MiB gets smaller");

    crate::dialogs::action_set_options::open_new(&mut app);
    set_dialog::<SetOptionsDialog>(&mut app, "name", json!("x".repeat(files::MAX_SET_FILE)));
    assert_eq!(confirm_dialog::<SetOptionsDialog>(&mut app).unwrap_err(), size);
    assert!(is_open::<SetOptionsDialog>(&app), "New Set stays open");
    assert_eq!(set_names(&app).len(), 3, "no set was added");
    cancel_dialog::<SetOptionsDialog>(&mut app);

    // The steps a recording adds to the set count too.
    let mut app = app_with_sets();
    pad_set_to(&mut app, 1, 1000);
    app.run(START_RECORDING, json!({"set": 1})).unwrap();
    app.session.journal.push(("object.move".to_string(), json!({"pad": "x".repeat(1000)})));
    assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudge+"})).unwrap_err(), size);
    app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudg"})).unwrap();
}

/// Load Actions… adds no set that Save Actions… would write as a file larger than 8 MiB: a file
/// with less white space, a file whose set name gets a number, and another app's action set whose
/// names take more bytes in the file.
#[test]
fn load_actions_refuses_a_set_that_would_save_larger_than_8_mib() {
    let mut app = app_with_sets();
    let size = "the action set would be larger than 8 MiB";
    let wide = [action("Zeros", &[("object.move", json!({"p": vec![0; 600_000]}))])];
    let saved = files::write_set("Wide", &wide);
    let packed = serde_json::to_string(&serde_json::from_str::<Value>(&saved).unwrap()).unwrap();
    assert!(packed.len() <= files::MAX_SET_FILE && saved.len() > files::MAX_SET_FILE);
    assert_eq!(app.run(LOAD_SET, json!({ "data": packed })).unwrap_err(), size);

    let mut full = app_with_sets();
    edit_set(&mut full, 1, Vec::clear);
    pad_set_to(&mut full, 1, 0);
    let mine = set_file(&full, 1);
    assert_eq!(app.run(LOAD_SET, json!({ "data": mine })).unwrap_err(), size, "Mine 2 is longer");
    let yours = mine.replacen("\"Mine\"", "\"Your\"", 1);
    assert_eq!(app.run(LOAD_SET, json!({ "data": yours })).unwrap()["name"], "Your");

    let name = "\u{1}".repeat(1024);
    let steps = [("Group", true); 40];
    let text = aia_text("Names", &vec![(name.as_str(), steps.as_slice()); 1000]);
    assert!(text.len() <= files::MAX_SET_FILE);
    assert_eq!(app.run(LOAD_SET, json!({ "data": text })).unwrap_err(), size);
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other", "Your"]);
}
