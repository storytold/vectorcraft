use super::tests_support::*;
use super::*;
use vectorcraft_engine::Session;

/// A recording keeps its steps when a canceled undo group drops journal entries made before the
/// recording began.
#[test]
fn a_recording_keeps_its_steps_when_a_canceled_undo_group_drops_earlier_entries() {
    use super::super::tests_appearance::{click, frame_events};
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
    let ctx = egui::Context::default();
    frame_events(&ctx, &mut app, vec![], show);
    // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, new action,
    // delete.
    let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [stop, record, _, _, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
    click(&ctx, &mut app, record.center(), show);
    for (id, p) in [("document.activate", json!({"index": 1})), ("shape.rectangle", rect(50)), ("edit.undoGroup.end", json!({"cancel": true}))] {
        app.run(id, p).unwrap();
    }
    click(&ctx, &mut app, stop.center(), show);
    let steps = app.ui.action_sets[0].actions.last().map(|a| a.steps.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
    assert_eq!(steps, Some(vec!["document.activate".to_string(), "shape.rectangle".into(), "edit.undoGroup.end".into()]));
}

/// A recording that New Action started goes into its action wherever the action is when Stop
/// is pressed, and is dropped when the action was deleted meanwhile.
#[test]
fn a_new_actions_recording_follows_its_action() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    app.run(NEW_ACTION, json!({"set": 2, "name": "Box"})).unwrap();
    app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let (stop, _) = stop_and_new_action(&ctx, &mut app);
    super::super::tests_appearance::click(&ctx, &mut app, stop, show);
    assert_eq!((action_names(&app, 1), app.ui.action_sets[1].actions[0].steps.len()), (vec!["Box".to_string()], 1));
    app.run(NEW_ACTION, json!({"set": 1, "name": "Gone"})).unwrap();
    app.ui.action_sets[1].actions.pop();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let (stop, _) = stop_and_new_action(&ctx, &mut app);
    super::super::tests_appearance::click(&ctx, &mut app, stop, show);
    assert_eq!(action_names(&app, 1), ["Box"], "the deleted action is not made again");
    assert!(app.ui.recording.is_none() && app.ui.recording_into.is_none());
    // `ui.action.stop` ends a recording as the button does.
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": false}), "nothing recording");
    app.run(NEW_ACTION, json!({"set": 1, "name": "Remote"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
    assert_eq!((action_names(&app, 1), app.ui.action_sets[1].actions[1].steps.len()), (vec!["Box".to_string(), "Remote".into()], 1));
    assert!(app.ui.recording.is_none());
}

#[test]
fn insert_menu_item_goes_into_the_recording() {
    let (mut app, _) = app_with_three_moves();
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap(), json!({"recording": true}));
    assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "file.save"})).unwrap_err(), "no menu item that can be inserted runs this command");
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = app.ui.action_sets[0].actions.last().unwrap();
    assert_eq!(recorded.steps, [("object.group".to_string(), json!({}))]);
}

#[test]
fn insert_select_path_goes_into_the_recording() {
    let (mut app, _, [first, _]) = app_with_two_rectangles();
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(INSERT_SELECT_PATH, json!({})).unwrap(), json!({"recording": true}));
    assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(first)]);
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = app.ui.action_sets[0].actions.last().unwrap();
    assert_eq!(recorded.steps.len(), 1);
    assert_eq!((recorded.steps[0].0.as_str(), &recorded.steps[0].1["id"]), (SELECT_PATH_STEP, &json!(first)));
}

/// The bottom bar's Stop and Begin Recording buttons, as drawn last.
fn stop_and_record(ctx: &egui::Context, app: &mut VectorcraftApp) -> (egui::Pos2, egui::Pos2) {
    super::super::tests_appearance::frame_events(ctx, app, vec![], show);
    let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [stop, record, ..] = bar[..] else { panic!("the bottom bar: {bar:?}") };
    (stop.center(), record.center())
}

#[test]
fn begin_recording_selected_action_adds_steps_without_new_action() {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    let (mut app, _) = app_with_three_moves();
    app.ui.action_set = Some(0);
    let ctx = egui::Context::default();
    app.ui_ctx = Some(ctx.clone());
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
    let (stop, record) = stop_and_record(&ctx, &mut app);
    click(&ctx, &mut app, record, show);
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    click(&ctx, &mut app, stop, show);
    assert_eq!(app.ui.action_sets[0].actions.len(), 1, "no new action");
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2", "object.move 4", "object.move 8"], "added at the end");

    // With a step selected, the recorded steps go after it.
    click_first_step(&ctx, &mut app);
    // The steps shown move the bottom bar down.
    let (stop, record) = stop_and_record(&ctx, &mut app);
    click(&ctx, &mut app, record, show);
    app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
    click(&ctx, &mut app, stop, show);
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 16", "object.move 2", "object.move 4", "object.move 8"]);

    // With no action selected, Begin Recording still adds a new action.
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "▾ Moves").center(), show);
    let (stop, record) = stop_and_record(&ctx, &mut app);
    click(&ctx, &mut app, record, show);
    app.run("object.move", json!({"dx": 32, "dy": 0})).unwrap();
    click(&ctx, &mut app, stop, show);
    let names: Vec<&str> = app.ui.action_sets[0].actions.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Three Moves", "Action 2"]);
    assert_eq!(step_names(&app, 0, 1), ["object.move 32"]);
}

#[test]
fn begin_recording_after_selected_step_preserves_order_and_exclusions() {
    let (mut app, _) = app_with_three_moves();
    app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 2]);
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
    assert_eq!(app.ui.action_sets[0].actions.len(), 1, "no new action");
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 8", "object.move 16", "object.move 2", "object.move 4"]);
    assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([0, 4]), "the steps keep whether they are excluded");
}

#[test]
fn steps_recorded_into_the_middle_of_an_action_keep_the_selected_step_and_dialog_on_their_steps() {
    let (mut app, _) = app_with_three_moves();
    let ctx = egui::Context::default();
    app.ui_ctx = Some(ctx.clone());
    let id = app.ui.action_sets[0].actions[0].id;
    app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 0})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
    set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((id, 2)));
    crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(1));
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 8", "object.move 16", "object.move 2", "object.move 4"]);
    assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), Some((id, 4)), "the move by 4 stays selected");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["step"], json!(3), "Record Stop stays after the move by 2");
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
    let recorded = app.ui.action_sets[0].actions.last().unwrap();
    let ids: Vec<&str> = recorded.steps.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["object.move", STOP_STEP, "object.move"]);
    assert_eq!(recorded.steps[1].1, json!({"message": "Now", "allowContinue": true}));
}
