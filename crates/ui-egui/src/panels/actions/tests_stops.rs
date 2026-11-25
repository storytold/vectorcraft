use super::tests_playback::{first_action, host_frame, on_host};
use super::tests_support::*;
use super::*;
use crate::dialogs::action_playback::PlaybackDialog;
use crate::dialogs::action_stop::StopMessage;
use craft::automation::actions::ActionStep;
use craft::ui::dialog::StackDialog;

/// Makes `steps` the steps of the action [`app_with_three_moves`] builds.
fn set_steps(app: &mut VectorcraftApp, steps: &[(&str, Value)]) {
    let a = first_action(app);
    for step in (0..app.ui.actions.step_count(a)).rev() {
        app.ui.actions.delete_step(a, step).unwrap();
    }
    for (at, (command, params)) in steps.iter().enumerate() {
        app.ui.actions.insert_step(a, at, ActionStep::new(*command, params.clone())).unwrap();
    }
}

/// Whether a stop's message box is open.
fn message_open(app: &VectorcraftApp) -> bool {
    app.dialogs().unwrap().with(|host| host.is_open::<StopMessage>())
}

/// Sets the answer of the stop's message box, as automation's `ui.dialog.set` does: `"proceed"` is Continue and
/// `"cancel"` is Stop. Refused for an answer the box has no button for.
fn choose(app: &VectorcraftApp, choice: &str) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().unwrap();
    app.dialogs().unwrap().with(|host| -> Result<(), String> {
        let id = host.get::<StopMessage>().map(|stop| stop.id()).ok_or("no stop message is open")?;
        host.set_json_field(&ctx, id, "choice", json!(choice))
    })
}

/// A step that fails after Continue at a stop is named by its place in the action, the stops
/// before it counted, the excluded one too.
#[test]
fn an_error_after_a_stop_names_the_step_in_the_action() {
    let (mut app, _) = app_with_three_moves();
    let stop = ActionStep::new(STOP_STEP, json!({"message": "", "allowContinue": true}));
    let a = first_action(&app);
    app.ui.actions.insert_step(a, 1, stop.clone()).unwrap();
    app.ui.actions.insert_step(a, 3, stop).unwrap();
    app.ui.actions.insert_step(a, app.ui.actions.step_count(a), ActionStep::new("object.align", json!({}))).unwrap();
    app.ui.actions.set_step(a, 3, false).unwrap();
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
    let stop = (STOP_STEP, json!({"message": "", "allowContinue": false}));
    set_steps(&mut app, &[("shape.rectangle", old), stop]);
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
    assert!(message_open(&app), "the stop shows its message");
    assert!(choose(&app, "proceed").is_err(), "without Allow Continue the message offers no Continue");
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
        assert_eq!(open_kind(&app).as_deref(), Some(crate::dialogs::action_stop::MESSAGE_KIND), "{speed}");
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
    assert_eq!(choose(&app, "proceed"), Ok(()), "Allow Continue offers Continue");
    on_host(&mut app, |host, ctx| host.confirm::<StopMessage>(ctx));
    assert_eq!(x(&app), 7.0, "Continue plays the rest");
    assert!(app.ui.action_playing.is_none());
    assert!(!message_open(&app));

    let (mut app, x) = app_with_a_stop(false);
    play(&mut app, 0, 0).unwrap();
    assert!(choose(&app, "proceed").is_err(), "without Allow Continue there is no Continue");
    assert_eq!(choose(&app, "cancel"), Ok(()), "Stop is offered");
    on_host(&mut app, |host, ctx| host.confirm::<StopMessage>(ctx));
    assert!(!message_open(&app), "the box's only button closes it");
    assert!(waiting_at_stop(&app), "the action waits for the manual task");
    assert_eq!(x(&app), 1.0);
    frame(&mut app, 100.0);
    assert!(!message_open(&app), "the box does not open again");
    app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
    assert_eq!(x(&app), 7.0);

    let (mut app, x) = app_with_a_stop(true);
    play(&mut app, 0, 0).unwrap();
    on_host(&mut app, |host, ctx| host.cancel::<StopMessage>(ctx));
    assert!(!message_open(&app));
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
    let stop = |message: &str| (STOP_STEP, json!({"message": message, "allowContinue": false}));
    let moved = ("object.move", json!({"dx": 1, "dy": 0}));
    set_steps(&mut app, &[stop("first"), stop("second"), moved, stop("last")]);
    // Which open of the stop's message box shows: the host numbers every open, so each stop's own box has a new one.
    let shown = |app: &VectorcraftApp| {
        app.dialogs()?.with(|host| {
            let id = host.get::<StopMessage>()?.id();
            host.open_serial(id)
        })
    };
    assert_eq!(play(&mut app, 0, 0).unwrap()["atStop"], json!(true));
    let first = shown(&app);
    assert_eq!((first.is_some(), x(&app)), (true, 0.0));
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap()["atStop"], json!(true));
    let second = shown(&app);
    assert_eq!((second.is_some() && second != first, x(&app)), (true, 0.0), "the second stop shows its own message");
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap()["atStop"], json!(true));
    let last = shown(&app);
    assert_eq!((last.is_some() && last != second, x(&app)), (true, 1.0), "the last stop shows its own message");
    assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap(), json!({"results": [], "pending": 0}));
    assert!(app.ui.action_playing.is_none(), "after the last stop the playback ends");
    assert_eq!(shown(&app), None);
}

#[test]
fn a_stop_with_an_empty_message_still_waits() {
    let (mut app, x) = app_with_three_moves();
    app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    assert_eq!(x(&app), 1.0);
    assert_eq!(open_kind(&app).as_deref(), Some(crate::dialogs::action_stop::MESSAGE_KIND));
}

#[test]
fn a_stop_nested_in_a_recorded_batch_holds_too() {
    let (mut app, x) = app_with_three_moves();
    let moved = json!({"command": "object.move", "params": {"dx": 1, "dy": 0}});
    let stop = json!({"command": STOP_STEP, "params": {"message": "Inner", "allowContinue": false}});
    set_steps(&mut app, &[("command.batch", json!({"commands": [moved, stop, moved]}))]);
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
    let recorded = set_actions(&app, 0).pop().unwrap();
    let ids: Vec<&str> = recorded.steps.iter().map(|s| s.command.as_str()).collect();
    assert_eq!(ids, ["command.batch", STOP_STEP, "command.batch"]);
    assert_eq!(recorded.steps[1].params, json!({"message": "Paint it", "allowContinue": false}));
}

#[test]
fn the_message_waits_for_a_dialog_already_open() {
    let (mut app, _) = app_with_a_stop(false);
    let playback_open = |app: &VectorcraftApp| app.dialogs().unwrap().with(|host| host.is_open::<PlaybackDialog>());
    crate::dialogs::action_playback::open(&mut app);
    // The window shows the dialog before the action plays: a dialog is open while the window shows its layer.
    host_frame(&mut app, false);
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    host_frame(&mut app, false);
    assert!(playback_open(&app), "the open dialog stays");
    assert!(!message_open(&app), "the message waits for it");
    on_host(&mut app, |host, ctx| host.cancel::<PlaybackDialog>(ctx));
    // The frame after the close still began with the dialog's layer shown; the next one shows the message.
    host_frame(&mut app, false);
    host_frame(&mut app, false);
    assert!(!playback_open(&app));
    assert!(message_open(&app), "the message shows once it closes");
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
