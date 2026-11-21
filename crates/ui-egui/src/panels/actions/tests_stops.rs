use super::tests_support::*;
use super::*;

/// A step that fails after Continue at a stop is named by its place in the action, the stops
/// before it counted, the excluded one too.
#[test]
fn an_error_after_a_stop_names_the_step_in_the_action() {
    let (mut app, _) = app_with_three_moves();
    let stop = (STOP_STEP.to_string(), json!({"message": "", "allowContinue": true}));
    let action = &mut app.ui.action_sets[0].actions[0];
    action.steps.insert(1, stop.clone());
    action.steps.insert(3, stop);
    action.steps.push(("object.align".into(), json!({})));
    action.excluded_steps.insert(3);
    assert_eq!(play(&mut app, 0, 0).unwrap()["atStop"], json!(true));
    assert_names_step(&app.run(CONTINUE_PLAYBACK, json!({})).unwrap_err(), 6, "object.align");
    assert!(app.ui.action_playing.is_none(), "the playback ended");
}

/// A View menu step before a stop plays before the playback waits there, and the steps after the
/// stop play on Continue.
#[test]
fn an_inserted_view_item_plays_up_to_a_stop() {
    let (mut app, x) = app_with_a_view_step();
    app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 1})).unwrap();
    let r = play(&mut app, 0, 0).unwrap();
    assert_eq!(r["atStop"], json!(true), "{r}");
    assert!(app.outline_view());
    assert_eq!(x(&app), 1.0);
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!(x(&app), 7.0);
    assert!(app.outline_view(), "Outline did not play again");
}

/// At Accelerated, the steps up to a stop play as saved steps, as they do without a stop: a key
/// their command no longer reads, recorded by an older version, is taken.
#[test]
fn steps_before_a_stop_play_with_a_key_no_longer_read() {
    let (mut app, _) = app_with_three_moves();
    let old = json!({"x": 10, "y": 50, "width": 20, "height": 20, "cornerStyle": "round"});
    assert!(app.run("shape.rectangle", old.clone()).is_err());
    let stop = (STOP_STEP.to_string(), json!({"message": "", "allowContinue": false}));
    app.ui.action_sets[0].actions[0].steps = vec![("shape.rectangle".into(), old), stop];
    let shapes = app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let r = play(&mut app, 0, 0).unwrap();
    assert_eq!(r["atStop"], json!(true), "{r}");
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), shapes + 1);
    assert!(!app.session.replaying());
}

#[test]
fn accelerated_playback_holds_at_a_stop_and_play_plays_the_rest() {
    let (mut app, x) = app_with_a_stop(false);
    let undo = undo_steps(&app);
    let r = play(&mut app, 0, 0).unwrap();
    assert_eq!((r["pending"].clone(), r["atStop"].clone(), r["results"].as_array().map(Vec::len)), (json!(2), json!(true), Some(1)));
    assert_eq!(x(&app), 1.0, "the step before the stop runs");
    let d = app.ui.dialog.as_ref().expect("the stop shows its message");
    assert_eq!(
        (d.kind.as_str(), d.str("action"), d.str("message"), d.bool("allowContinue")),
        (crate::dialogs::action_stop::MESSAGE_KIND, "Three Moves".to_string(), "Paint it".to_string(), false)
    );
    for now in [0.0, 100.0, 1000.0] {
        assert_eq!(frame(&mut app, now), None);
        assert_eq!(x(&app), 1.0, "no later step runs while the action waits");
    }
    assert_eq!(play(&mut app, 0, 0).unwrap_err(), "an action is already playing");
    let r = app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!((r["pending"].clone(), r["atStop"].clone()), (json!(0), Value::Null));
    assert_eq!(x(&app), 7.0, "Play runs every step after the stop");
    assert!(app.ui.action_playing.is_none());
    assert_eq!(open_kind(&app), None, "the message box closes");
    assert_eq!(undo_steps(&app), undo + 2, "the steps on each side of the stop are one undo step each");
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap_err(), "no action is waiting at a stop");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(x(&app), 1.0, "one undo takes back the steps after the stop");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(x(&app), 0.0, "and one more the steps before it");
}

#[test]
fn step_by_step_and_pause_for_hold_at_a_stop() {
    for (speed, seconds) in [("step", 1), ("pause", 2)] {
        let (mut app, x) = app_with_a_stop(false);
        app.run(PLAYBACK_OPTIONS, json!({"speed": speed, "seconds": seconds})).unwrap();
        let r = play(&mut app, 0, 0).unwrap();
        assert_eq!((r["pending"].clone(), r["atStop"].clone()), (json!(3), Value::Null), "{speed}");
        assert_eq!(open_kind(&app), None, "{speed}: the stop comes after the step before it is shown");
        let mut now = 0.0;
        while !waiting_at_stop(&app) {
            assert!(now < 100.0, "{speed}: the playback reaches the stop");
            frame(&mut app, now);
            now += 1.0;
        }
        assert_eq!(open_kind(&app), Some(crate::dialogs::action_stop::MESSAGE_KIND), "{speed}");
        for _ in 0..5 {
            assert_eq!(frame(&mut app, now + 1000.0), None, "{speed}");
            assert_eq!(x(&app), 1.0, "{speed}: no later step runs while the action waits");
        }
        let r = app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!(r["pending"], json!(1), "{speed}: Play runs the next step, and the last stays for its turn");
        assert_eq!(x(&app), 3.0, "{speed}");
        for _ in 0..20 {
            if app.ui.action_playing.is_none() {
                break;
            }
            frame(&mut app, now);
            now += 10.0;
        }
        assert_eq!(x(&app), 7.0, "{speed}: the paced playback goes on to the end");
        assert!(app.ui.action_playing.is_none(), "{speed}");
    }
}

#[test]
fn a_paced_playback_holds_at_a_stop_when_another_document_became_active() {
    let (mut app, _) = app_with_a_stop(false);
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(tick_playback(&mut app, 0.0), Some(1.0));
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.run("shape.rectangle", json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    assert_eq!(tick_playback(&mut app, 1.0), None);
    assert!(waiting_at_stop(&app), "the stop holds the playback, as it does at Accelerated");
    assert_ne!(app.ui.status, "the action stopped playing: its document is no longer active");
    let r = app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!(r["pending"], json!(1), "Continue runs the next step in the document active now");
}

#[test]
fn allow_continue_offers_continue_and_stop_closes_the_box_and_keeps_waiting() {
    let (mut app, x) = app_with_a_stop(true);
    play(&mut app, 0, 0).unwrap();
    assert!(dialog_texts(&mut app).iter().any(|s| s == "Continue"), "Allow Continue shows Continue");
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(x(&app), 7.0, "Continue plays the rest");
    assert!(app.ui.action_playing.is_none());
    assert_eq!(open_kind(&app), None);

    let (mut app, x) = app_with_a_stop(false);
    play(&mut app, 0, 0).unwrap();
    let texts = dialog_texts(&mut app);
    assert!(!texts.iter().any(|s| s == "Continue"), "without Allow Continue there is no Continue: {texts:?}");
    assert!(texts.iter().any(|s| s == "Paint it") && texts.iter().any(|s| s == "Stop"), "{texts:?}");
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(open_kind(&app), None, "the box's only button closes it");
    assert!(waiting_at_stop(&app), "the action waits for the manual task");
    assert_eq!(x(&app), 1.0);
    frame(&mut app, 100.0);
    assert_eq!(open_kind(&app), None, "the box does not open again");
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!(x(&app), 7.0);

    let (mut app, x) = app_with_a_stop(true);
    play(&mut app, 0, 0).unwrap();
    crate::dialogs::cancel(&mut app);
    assert!(waiting_at_stop(&app), "Stop closes the box and the action waits");
    assert_eq!(x(&app), 1.0);
}

#[test]
fn stop_at_a_stop_ends_the_playback_and_keeps_the_steps_run() {
    let (mut app, x) = app_with_a_stop(false);
    let undo = undo_steps(&app);
    play(&mut app, 0, 0).unwrap();
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
    assert!(app.ui.action_playing.is_none());
    assert_eq!(open_kind(&app), None, "the message box closes");
    assert_eq!(frame(&mut app, 100.0), None);
    assert_eq!(x(&app), 1.0, "the steps after the stop do not run; the one played stays");
    assert_eq!(undo_steps(&app), undo + 1);
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap_err(), "no action is waiting at a stop");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(x(&app), 0.0, "the step played undoes");
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app), "the action plays again after Stop");
}

#[test]
fn stops_first_last_and_in_a_row_each_wait() {
    let (mut app, x) = app_with_three_moves();
    let stop = |message: &str| (STOP_STEP.to_string(), json!({"message": message, "allowContinue": false}));
    let moved = ("object.move".to_string(), json!({"dx": 1, "dy": 0}));
    app.ui.action_sets[0].actions[0].steps = vec![stop("first"), stop("second"), moved, stop("last")];
    let message = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.str("message"));
    assert_eq!(play(&mut app, 0, 0).unwrap()["atStop"], json!(true));
    assert_eq!((message(&app).as_deref(), x(&app)), (Some("first"), 0.0));
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap()["atStop"], json!(true));
    assert_eq!((message(&app).as_deref(), x(&app)), (Some("second"), 0.0));
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap()["atStop"], json!(true));
    assert_eq!((message(&app).as_deref(), x(&app)), (Some("last"), 1.0));
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap(), json!({"results": [], "pending": 0}));
    assert!(app.ui.action_playing.is_none(), "after the last stop the playback ends");
    assert_eq!(message(&app), None);
}

#[test]
fn a_stop_with_an_empty_message_still_waits() {
    let (mut app, x) = app_with_three_moves();
    app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    assert_eq!(x(&app), 1.0);
    assert_eq!(open_kind(&app), Some(crate::dialogs::action_stop::MESSAGE_KIND));
}

#[test]
fn a_stop_nested_in_a_recorded_batch_holds_too() {
    let (mut app, x) = app_with_three_moves();
    let moved = json!({"command": "object.move", "params": {"dx": 1, "dy": 0}});
    let stop = json!({"command": STOP_STEP, "params": {"message": "Inner", "allowContinue": false}});
    app.ui.action_sets[0].actions[0].steps = vec![("command.batch".into(), json!({"commands": [moved, stop, moved]}))];
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    assert_eq!(x(&app), 1.0);
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!(x(&app), 2.0);
}

#[test]
fn a_stop_reached_while_recording_is_recorded() {
    let (mut app, _) = app_with_a_stop(false);
    app.run(START_RECORDING, json!({})).unwrap();
    play(&mut app, 0, 0).unwrap();
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = app.ui.action_sets[0].actions.last().unwrap();
    let ids: Vec<&str> = recorded.steps.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["command.batch", STOP_STEP, "command.batch"]);
    assert_eq!(recorded.steps[1].1, json!({"message": "Paint it", "allowContinue": false}));
}

#[test]
fn the_message_waits_for_a_dialog_already_open() {
    let (mut app, _) = app_with_a_stop(false);
    crate::dialogs::action_playback::open(&mut app);
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    frame(&mut app, 0.0);
    assert_eq!(open_kind(&app), Some(crate::dialogs::action_playback::KIND), "the open dialog stays");
    crate::dialogs::cancel(&mut app);
    frame(&mut app, 0.0);
    assert_eq!(open_kind(&app), Some(crate::dialogs::action_stop::MESSAGE_KIND), "the message shows once it closes");
}

#[test]
fn play_on_from_a_stop_uses_the_document_active_then() {
    let (mut app, x) = app_with_a_stop(false);
    play(&mut app, 0, 0).unwrap();
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!(x(&app), 7.0);
    let (mut app, _) = app_with_a_stop(false);
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    play(&mut app, 0, 0).unwrap();
    frame(&mut app, 0.0);
    frame(&mut app, 0.0);
    assert!(waiting_at_stop(&app));
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert!(app.ui.action_playing.is_some(), "a document opened at the stop does not end the playback");
    let bounds = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert_eq!(bounds.x0, 2.0, "the next step runs on the document active now");
}
