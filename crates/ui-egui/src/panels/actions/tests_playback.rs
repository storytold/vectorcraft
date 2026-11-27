use super::tests_support::*;
use super::*;
use craft::automation::actions::{Action, ActionList, ActionStep};
use craft::quit::CloseAnswer;
use craft::ui::dialog::{DialogHost, UnsavedChangesDialog};
use vectorcraft_engine::Session;

/// The list index of the first action of the first set: the action the fixtures build and these tests edit.
pub(super) fn first_action(app: &VectorcraftApp) -> usize {
    model::flat(&app.ui.actions, 0, 0).unwrap()
}

/// One host frame of the app's window, shown or occluded: the runtime's frame input, then `tick` and a UI pass.
pub(super) fn host_frame(app: &mut VectorcraftApp, hidden: bool) {
    let ctx = app.ui_ctx.clone().unwrap();
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
    if hidden {
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().occluded = Some(true);
    }
    crate::testing::app_frame(app, &ctx, raw);
}

/// Runs `act` on the window's dialog host, as automation's `ui.dialog.*` does, then the frame's dialog pump, which
/// takes the answer. `act` returns whether the host took it.
pub(super) fn on_host(app: &mut VectorcraftApp, act: impl FnOnce(&mut DialogHost, &egui::Context) -> bool) {
    let ctx = app.ui_ctx.clone().unwrap();
    assert!(app.dialogs().unwrap().with(|host| act(host, &ctx)), "the dialog host took it");
    crate::dialogs::host::pump(app, &ctx);
}

/// An action recorded with a key its command no longer reads still plays, accelerated and one
/// step at a time, though the same command run by itself is refused.
#[test]
fn an_action_with_a_key_no_longer_read_still_plays() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let old = json!({"dx": 5, "dy": 0, "dyy": 1});
    assert!(app.run("object.move", old.clone()).is_err());
    app.ui.actions.replace(ActionList::new());
    push_set(&mut app, "Test", vec![action("Move", &[("object.move", old.clone()), ("object.move", old)])]);
    let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let before = x(&app);
    play(&mut app, 0, 0).unwrap();
    assert!((x(&app) - before - 10.0).abs() < 1e-9, "both moves ran");
    app.ui.action_playback.speed = PlaybackSpeed::Step;
    assert_eq!(play(&mut app, 0, 0).unwrap()["pending"], json!(1));
    assert!((x(&app) - before - 15.0).abs() < 1e-9, "the first step ran");
    assert!(!app.session.replaying());
}

#[test]
fn automatic_align_reaches_the_align_steps_of_a_recorded_batch_run() {
    let key = json!({"command": "object.align", "params": {"horizontal": "center", "to": "key", "artboard": 1}});
    let moved = json!({"command": "object.move", "params": {"dx": 5, "dy": 0}});
    let batch = json!({"commands": [key, moved], "label": "Center", "source": "datasets", "destination": "none"});
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui.actions.replace(ActionList::new());
    let mut aligned = action("Batch Center Auto", &[("action.batch", batch.clone())]);
    model::set_automatic_align_to(&mut aligned, true);
    push_set(&mut app, "Test", vec![action("Batch Center", &[("action.batch", batch.clone())]), aligned]);
    let actions = set_actions(&app, 0);
    assert_eq!(playback::included_commands(&actions[0]), [json!({"command": "action.batch", "params": batch})], "without the option");
    let auto = json!({"command": "object.align", "params": {"horizontal": "center", "to": "auto"}});
    let played = json!({"commands": [auto, moved], "label": "Center", "source": "datasets", "destination": "none"});
    assert_eq!(playback::included_commands(&actions[1]), [json!({"command": "action.batch", "params": played})]);
    assert_eq!(actions[1].steps[0].params, batch, "the stored Batch run keeps its recorded Align target");
}

/// Actions recorded with Align to Artboard and Paste in Place, and the default Center on
/// Artboard, play on the artboard active when they play, not on the one active when they were
/// recorded.
#[test]
fn actions_play_on_the_artboard_active_when_they_play() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let r = app.run("shape.rectangle", json!({"x": boards[1].x0 + 10.0, "y": boards[1].y0 + 15.0, "width": 20, "height": 20})).unwrap();
    let rect = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    let bounds = |app: &VectorcraftApp, ids: &[vectorcraft_doc::NodeId]| app.session.active().unwrap().doc.bounds_of(ids, false).unwrap();
    // Each recorded as the panel records an action, with artboard 0 active.
    let record = |app: &mut VectorcraftApp, name: &str, id: &str, params: Value| {
        let start = app.session.journal.len();
        app.run(id, params).unwrap();
        let steps = app.session.journal_for_action(start).into_iter().map(ActionStep::from).collect();
        let set = model::set_name(&app.ui.actions, 0).map(str::to_string);
        app.ui.actions.push(Action { set, ..Action::new(name, steps) }).unwrap();
        set_actions(app, 0).len() - 1
    };
    app.run("edit.copy", json!({})).unwrap();
    let align = record(&mut app, "Align Left", "object.align", json!({"horizontal": "left", "to": "artboard"}));
    assert_eq!(bounds(&app, &[rect]).x0, boards[0].x0, "aligned to artboard 0 while recording");
    let paste = record(&mut app, "Paste in Place", "edit.pasteInPlace", json!({}));
    app.view_mut().unwrap().artboard = 2;
    app.run("select.set", json!({"ids": [rect.0]})).unwrap();
    play(&mut app, 0, align).unwrap();
    assert_eq!(bounds(&app, &[rect]).x0, boards[2].x0, "aligned to artboard 2, active when the action plays");
    let ids: Vec<vectorcraft_doc::NodeId> = play(&mut app, 0, paste).unwrap()["results"][0]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| vectorcraft_doc::NodeId(v.as_u64().unwrap()))
        .collect();
    let b = bounds(&app, &ids);
    assert_eq!((b.x0, b.y0), (boards[2].x0 + 10.0, boards[2].y0 + 15.0), "pasted onto artboard 2, where it was on artboard 1");
    app.view_mut().unwrap().artboard = 1;
    app.run("select.set", json!({"ids": [rect.0]})).unwrap();
    let center = set_actions(&app, 0).iter().position(|a| a.name == "Center on Artboard").unwrap();
    play(&mut app, 0, center).unwrap();
    assert_eq!(bounds(&app, &[rect]).center(), boards[1].center(), "centered on artboard 1, the active one");
}

/// Actions that record Distribute Objects and Distribute Spacing with Align to Artboard play on
/// the artboard active when they play, not on the one active when they were recorded.
#[test]
fn distribute_actions_play_on_the_artboard_active_when_they_play() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    // The rectangles are 20, 40 and 20 pt wide, and they are on artboard 1.
    let ids: Vec<vectorcraft_doc::NodeId> = [(10.0, 20.0), (50.0, 40.0), (120.0, 20.0)]
        .into_iter()
        .map(|(x, w)| {
            let r = app.run("shape.rectangle", json!({"x": boards[1].x0 + x, "y": boards[1].y0 + 15.0, "width": w, "height": 20})).unwrap();
            vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())
        })
        .collect();
    app.run("select.set", json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    let lefts =
        |app: &VectorcraftApp| ids.iter().map(|id| app.session.active().unwrap().doc.bounds_of(&[*id], false).unwrap().x0).collect::<Vec<_>>();
    // Each step is recorded as the panel records an action, while artboard 0 is active.
    let record = |app: &mut VectorcraftApp, name: &str, id: &str, params: Value| {
        let start = app.session.journal.len();
        app.run(id, params).unwrap();
        let steps = app.session.journal_for_action(start).into_iter().map(ActionStep::from).collect();
        let set = model::set_name(&app.ui.actions, 0).map(str::to_string);
        app.ui.actions.push(Action { set, ..Action::new(name, steps) }).unwrap();
        set_actions(app, 0).len() - 1
    };
    let (x0, x1) = (boards[0].x0, boards[0].x1);
    let distribute = record(&mut app, "Distribute Left", "object.distribute", json!({"horizontal": "left", "to": "artboard"}));
    assert_eq!(lefts(&app), [x0, x0 + 90.0, x1 - 20.0], "distributed across artboard 0 while recording");
    let spacing = record(&mut app, "Distribute Spacing", "object.distributeSpacing", json!({"axis": "horizontal", "to": "artboard"}));
    assert_eq!(lefts(&app), [x0, x0 + 80.0, x1 - 20.0], "spaced across artboard 0 while recording");
    app.view_mut().unwrap().artboard = 2;
    let (x0, x1) = (boards[2].x0, boards[2].x1);
    play(&mut app, 0, distribute).unwrap();
    assert_eq!(lefts(&app), [x0, x0 + 90.0, x1 - 20.0], "distributed across artboard 2, active when the action plays");
    play(&mut app, 0, spacing).unwrap();
    assert_eq!(lefts(&app), [x0, x0 + 80.0, x1 - 20.0], "spaced across artboard 2, active when the action plays");
}

/// An action with an included step whose command is not known does not play at any speed: no
/// step runs, and the error names the step, a `command.batch` step's command by its place in
/// the batch too. An excluded step with an unknown command does not stop it.
#[test]
fn an_action_with_an_unknown_command_does_not_play() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let step = || ("object.move", json!({"dx": 5, "dy": 0}));
    let inner = json!({"commands": [{"command": "object.move", "params": {"dx": 5, "dy": 0}}, {"command": "example.unknown.inner"}]});
    app.ui.actions.replace(ActionList::new());
    push_set(
        &mut app,
        "Test",
        vec![action("Move", &[step(), ("example.unknown.finish", json!({})), step()]), action("Batch", &[step(), ("command.batch", inner)])],
    );
    let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let before = x(&app);
    for speed in [PlaybackSpeed::Accelerated, PlaybackSpeed::Step] {
        app.ui.action_playback.speed = speed;
        assert_eq!(play(&mut app, 0, 0).unwrap_err(), "step 2 of “Move” runs the unknown command `example.unknown.finish`");
        assert_eq!(play(&mut app, 0, 1).unwrap_err(), "step 2.2 of “Batch” runs the unknown command `example.unknown.inner`");
        assert_eq!(x(&app), before, "no step ran");
        assert!(app.ui.action_playing.is_none());
    }
    app.ui.action_playback.speed = PlaybackSpeed::Accelerated;
    let a = first_action(&app);
    app.ui.actions.set_step(a, 1, false).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert!((x(&app) - before - 10.0).abs() < 1e-9, "both included moves ran");
}

/// `ui.action.play` plays an action by its name; `set` narrows the search to the set it names. A
/// name no action has, a name no action in the named set has, and a `set` that is not a name are
/// refused, and then nothing plays.
#[test]
fn play_action_by_name_finds_the_action_and_the_set_narrows_it() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let far = action("Nudge Far", &[("object.move", json!({"dx": 5, "dy": 0}))]);
    app.ui.actions.push(Action { set: Some("Other".into()), ..far }).unwrap();
    let id = app.session.active().unwrap().selection.objects.clone();
    let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.bounds_of(&id, false).unwrap().x0;
    let before = x(&app);
    assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nope"})).unwrap_err(), "no such action");
    assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nudge", "set": "Other"})).unwrap_err(), "no such action", "Nudge is in Mine");
    assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nudge", "set": "Missing"})).unwrap_err(), "no such action");
    assert_eq!(app.run(PLAY_ACTION, json!({})).unwrap_err(), "no such action");
    assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nudge", "set": 0})).unwrap_err(), "no such action set");
    assert_eq!(x(&app), before, "nothing played");
    app.run(PLAY_ACTION, json!({"action": "Nudge Far", "set": "Other"})).unwrap();
    assert_eq!(x(&app), before + 5.0, "Other's Nudge Far played");
    app.run(PLAY_ACTION, json!({"action": "Nudge"})).unwrap();
    assert_eq!(x(&app), before + 6.0, "a name plays its action without a set");
}

/// Draws frames until the playback in progress ends (at most 20).
fn play_out(app: &mut VectorcraftApp) {
    for _ in 0..20 {
        if app.ui.action_playing.is_none() {
            return;
        }
        frame(app, 0.0);
    }
    panic!("the playback did not end");
}

#[test]
fn playback_options_sets_each_speed_and_refuses_bad_values() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.ui.action_playback, PlaybackOptions { speed: PlaybackSpeed::Accelerated, seconds: 1 }, "Accelerated is the default");
    assert_eq!(app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap(), json!({"speed": "step", "seconds": 1}));
    assert_eq!(app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 2})).unwrap(), json!({"speed": "pause", "seconds": 2}));
    assert_eq!(app.run(PLAYBACK_OPTIONS, json!({"speed": "accelerated"})).unwrap(), json!({"speed": "accelerated", "seconds": 2}), "seconds kept");
    assert_eq!(app.run(PLAYBACK_OPTIONS, json!({"seconds": 60})).unwrap(), json!({"speed": "accelerated", "seconds": 60}), "speed kept");
    let kept = app.ui.action_playback;
    for bad in [json!({"speed": "fast"}), json!({"speed": null}), json!({"speed": 1})] {
        assert_eq!(app.run(PLAYBACK_OPTIONS, bad.clone()).unwrap_err(), "speed must be accelerated, step or pause", "{bad}");
    }
    for bad in [json!(0), json!(61), json!(2.5), json!(-1), json!("2"), json!(null)] {
        let p = json!({"speed": "pause", "seconds": bad});
        assert_eq!(app.run(PLAYBACK_OPTIONS, p).unwrap_err(), "seconds must be a whole number from 1 to 60", "{bad}");
    }
    assert_eq!(app.ui.action_playback, kept, "a refused value changes nothing");
    // Kept with the UI preferences; a saved value that is not valid loads as the default.
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 5})).unwrap();
    let mut saved = serde_json::to_value(&app.ui).unwrap();
    let ui = serde_json::from_value::<crate::UiState>(saved.clone()).unwrap();
    assert_eq!(ui.action_playback, PlaybackOptions { speed: PlaybackSpeed::Pause, seconds: 5 });
    for bad in [json!({"speed": "pause", "seconds": 0}), json!({"speed": "slow", "seconds": 5}), json!("step")] {
        saved["action_playback"] = bad;
        assert_eq!(serde_json::from_value::<crate::UiState>(saved.clone()).unwrap().action_playback, PlaybackOptions::default());
    }
}

#[test]
fn accelerated_playback_plays_every_step_as_one_undo_step() {
    let (mut app, x) = app_with_three_moves();
    let before = undo_steps(&app);
    play(&mut app, 0, 0).unwrap();
    assert_eq!(x(&app), 7.0);
    assert_eq!(undo_steps(&app), before + 1);
    assert!(app.ui.action_playing.is_none());
    assert_eq!(tick_playback(&mut app, 0.0), None, "nothing waits for a frame");
}

#[test]
fn step_by_step_plays_one_step_per_frame_and_redraws_between_steps() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    let before = undo_steps(&app);
    assert_eq!(play(&mut app, 0, 0).unwrap()["pending"], json!(2));
    assert_eq!(x(&app), 1.0, "the first step runs at once");
    assert_eq!(frame(&mut app, 0.0), None, "the frame after Play draws the first step");
    assert_eq!(x(&app), 1.0, "and runs no step, even when Play ran after the canvas drew");
    assert_eq!(frame(&mut app, 0.0), Some(0.0), "the window draws again before the next step");
    assert_eq!(x(&app), 3.0, "one more step in this frame");
    assert_eq!(frame(&mut app, 0.0), None, "the last step ends the playback");
    assert_eq!(x(&app), 7.0);
    assert!(app.ui.action_playing.is_none());
    assert_eq!(frame(&mut app, 0.0), None);
    assert_eq!(x(&app), 7.0);
    assert_eq!(undo_steps(&app), before + 3, "each step is its own undo step");
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "Three Moves");
}

#[test]
fn step_by_step_runs_no_step_until_the_window_draws_the_one_before() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    play(&mut app, 0, 0).unwrap();
    // A hidden window runs logic passes only, which draw nothing.
    for _ in 0..3 {
        assert_eq!(tick_playback(&mut app, 0.0), None, "no step runs and no frame is asked for");
    }
    assert_eq!(x(&app), 1.0);
    playback_drawn(&mut app);
    assert_eq!(tick_playback(&mut app, 0.0), Some(0.0));
    assert_eq!(x(&app), 3.0, "the step after a drawn frame runs");
    for _ in 0..3 {
        assert_eq!(tick_playback(&mut app, 0.0), None);
    }
    assert_eq!(x(&app), 3.0, "the next step waits until the window draws this one");
    playback_drawn(&mut app);
    assert_eq!(tick_playback(&mut app, 0.0), None, "the last step ends the playback");
    assert_eq!(x(&app), 7.0);
}

#[test]
fn step_by_step_waits_while_the_window_is_hidden() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    // The window is up and shown before the action plays.
    host_frame(&mut app, false);
    host_frame(&mut app, false);
    play(&mut app, 0, 0).unwrap();
    for _ in 0..3 {
        host_frame(&mut app, true);
    }
    assert_eq!(x(&app), 1.0, "no step runs while the window is hidden, even in a UI pass");
    host_frame(&mut app, false);
    assert_eq!(x(&app), 1.0, "the shown frame draws the first step");
    host_frame(&mut app, true);
    assert_eq!(x(&app), 3.0, "the first step was shown, so the second runs");
    for _ in 0..3 {
        host_frame(&mut app, true);
    }
    assert_eq!(x(&app), 3.0, "the third step waits until the window shows the second");
    host_frame(&mut app, false);
    assert_eq!(x(&app), 3.0, "the shown frame draws the second step");
    host_frame(&mut app, false);
    assert_eq!(x(&app), 7.0);
    assert!(app.ui.action_playing.is_none());
}

#[test]
fn pause_for_waits_the_seconds_between_steps() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 2})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(x(&app), 1.0);
    assert_eq!(tick_playback(&mut app, 10.0), Some(2.0), "the pause starts in the frame after the first step");
    assert_eq!(tick_playback(&mut app, 11.5), Some(0.5));
    assert_eq!(x(&app), 1.0, "nothing runs before the pause has passed");
    assert_eq!(tick_playback(&mut app, 12.0), Some(0.0), "the next frame comes promptly");
    assert_eq!(x(&app), 3.0, "the second step runs after 2 s");
    assert_eq!(tick_playback(&mut app, 12.0), Some(2.0), "and starts the next pause");
    assert!(tick_playback(&mut app, 13.9).is_some_and(|wait| (wait - 0.1).abs() < 1e-9));
    assert_eq!(x(&app), 3.0);
    assert_eq!(tick_playback(&mut app, 14.0), None);
    assert_eq!(x(&app), 7.0, "the third step runs 2 s after the second");
    assert!(app.ui.action_playing.is_none());
}

#[test]
fn pause_for_starts_each_pause_in_the_frame_after_the_step() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 2})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(tick_playback(&mut app, 0.0), Some(2.0));
    assert!(tick_playback(&mut app, 2.0).is_some());
    assert_eq!(x(&app), 3.0, "the second step runs");
    // The next frame comes later than the pause, for example after a slow step.
    assert_eq!(tick_playback(&mut app, 9.0), Some(2.0), "the pause starts in this frame");
    assert_eq!(x(&app), 3.0, "no step runs without a pause before it");
    assert_eq!(tick_playback(&mut app, 11.0), None);
    assert_eq!(x(&app), 7.0, "the third step runs 2 s after the frame that started the pause");
}

#[test]
fn stop_ends_a_paced_playback_and_another_action_waits_for_it() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(play(&mut app, 0, 0).unwrap_err(), "an action is already playing");
    assert_eq!(app.run(PLAY_ACTION, json!({"action": "Three Moves"})).unwrap_err(), "an action is already playing");
    assert_eq!(x(&app), 1.0);
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
    assert_eq!(tick_playback(&mut app, 100.0), None);
    assert_eq!(x(&app), 1.0, "the steps after Stop do not run; the one played stays");
    assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": false}));
    play(&mut app, 0, 0).unwrap();
    assert_eq!(x(&app), 2.0, "an action plays again after Stop");
}

#[test]
fn control_requests_come_before_the_step_that_is_due() {
    // One host iteration's `tick` at `now`.
    let tick = |app: &mut VectorcraftApp, now: f64| {
        let ctx = app.ui_ctx.clone().unwrap();
        app.now_override = Some(now);
        craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
    };
    // The same, with `method` and `params` queued on the control channel → its reply.
    let iteration =
        |app: &mut VectorcraftApp, tx: &std::sync::mpsc::Sender<craft::automation::ControlRequest>, now: f64, method: &str, params: Value| {
            let (req, reply) = craft::automation::ControlRequest::new(method, params);
            tx.send(req).unwrap();
            tick(app, now);
            reply.try_recv().unwrap()
        };
    // Stop: the step that is due does not run.
    let (app, x) = app_with_three_moves();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = app.with_control(rx);
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(tick_playback(&mut app, 0.0), Some(1.0));
    let r = iteration(&mut app, &tx, 5.0, "engine.execute", json!({"command": STOP_RECORDING}));
    assert_eq!(r["result"], json!({"stopped": true}), "{r}");
    assert!(app.ui.action_playing.is_none());
    assert_eq!(x(&app), 1.0, "the step that was due did not run");
    // A switch to another document: the playback ends, and the step does not run in the first one.
    let (app, x) = app_with_three_moves();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = app.with_control(rx);
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(tick_playback(&mut app, 0.0), Some(1.0));
    let r = iteration(&mut app, &tx, 5.0, "engine.execute", json!({"command": "file.new", "params": {"width": 50, "height": 50}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.ui.action_playing.is_none(), "the playback ends in the same iteration");
    assert_eq!(app.ui.status, "the action stopped playing: its document is no longer active");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(x(&app), 1.0, "the step that was due did not run in the first document");
    // Play: the pause after the first step starts in the next iteration.
    let (app, x) = app_with_three_moves();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = app.with_control(rx);
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    let r = iteration(&mut app, &tx, 0.0, "engine.execute", json!({"command": PLAY_ACTION, "params": {"action": "Three Moves"}}));
    assert_eq!(r["result"]["pending"], json!(2), "{r}");
    assert_eq!(x(&app), 1.0);
    tick(&mut app, 0.5);
    tick(&mut app, 1.0);
    assert_eq!(x(&app), 1.0, "the pause started at 0.5");
    tick(&mut app, 1.5);
    assert_eq!(x(&app), 3.0);
}

#[test]
fn an_action_recorded_while_another_plays_plays_its_steps() {
    let (mut app, x) = app_with_three_moves();
    // Accelerated: the recording gets the played action as one batch step.
    app.run(START_RECORDING, json!({"set": 0})).unwrap();
    play(&mut app, 0, 0).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(set_actions(&app, 0)[1].steps.iter().map(|s| s.command.as_str()).collect::<Vec<_>>(), ["command.batch"]);
    assert_eq!(x(&app), 7.0);
    play(&mut app, 0, 1).unwrap();
    assert_eq!(x(&app), 14.0, "the batch step plays its moves");
    // Step by Step: recording begins after the first step, and gets each later step as a batch.
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    play(&mut app, 0, 0).unwrap();
    app.run(START_RECORDING, json!({"set": 0})).unwrap();
    play_out(&mut app);
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(x(&app), 21.0);
    assert_eq!(set_actions(&app, 0)[2].steps.len(), 2);
    play(&mut app, 0, 2).unwrap();
    play_out(&mut app);
    assert_eq!(x(&app), 27.0, "paced: the recorded batches play the second and third moves");
    app.run(PLAYBACK_OPTIONS, json!({"speed": "accelerated"})).unwrap();
    play(&mut app, 0, 2).unwrap();
    assert_eq!(x(&app), 33.0, "accelerated: the recorded batches play inside the action's batch");
}

#[test]
fn a_paced_playback_ends_when_another_document_becomes_active() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(tick_playback(&mut app, 0.0), Some(1.0));
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    assert_eq!(tick_playback(&mut app, 0.5), Some(0.5), "nothing ends before the next step is due");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(tick_playback(&mut app, 1.0), Some(0.0));
    assert_eq!(x(&app), 3.0, "the document is active again when the step is due, so it runs");
    app.run("document.activate", json!({"index": 1})).unwrap();
    let other = app.session.active().unwrap().doc.clone();
    assert_eq!(tick_playback(&mut app, 5.0), Some(1.0), "the next pause starts");
    assert_eq!(tick_playback(&mut app, 6.0), None, "the playback ends");
    assert!(app.ui.action_playing.is_none());
    assert_eq!(app.ui.status, "the action stopped playing: its document is no longer active");
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &other), "no step ran in the other document");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(tick_playback(&mut app, 10.0), None);
    assert_eq!(x(&app), 3.0, "the rest of the steps do not run in the first document either");
    // A step that makes another document active itself does not end the playback.
    let a = first_action(&app);
    app.ui.actions.insert_step(a, 0, ActionStep::new("document.activate", json!({"index": 0}))).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    app.ui.status.clear();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(frame(&mut app, 0.0), None);
    play_out(&mut app);
    assert_ne!(app.ui.status, "the action stopped playing: its document is no longer active");
    assert_eq!(x(&app), 10.0, "the moves after the step ran in the document it made active");
}

#[test]
fn closing_the_playing_document_ends_a_paced_playback() {
    let stopped = "the action stopped playing: its document is no longer active";
    // File › Close with `p`, answering Don't Save for the modified document.
    let close = |app: &mut VectorcraftApp, p: Value| {
        assert_eq!(app.run("file.close", p).unwrap(), json!({"pending": crate::unsaved::KIND}));
        on_host(app, |host, ctx| {
            host.get_mut::<UnsavedChangesDialog>().unwrap().set_answer(CloseAnswer::DontSave);
            host.confirm::<UnsavedChangesDialog>(ctx)
        });
    };
    // Step by Step, the only document closed: no document is active when the next step is due.
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(x(&app), 1.0);
    close(&mut app, json!({}));
    assert!(app.session.active().is_none());
    assert_eq!(frame(&mut app, 0.0), None, "the frame after Play draws");
    assert_eq!(frame(&mut app, 0.0), None, "the next step is due, and the playback ends");
    assert!(app.ui.action_playing.is_none());
    assert_eq!(app.ui.status, stopped);
    // Pause For, another document left active: no later step runs in it.
    let (mut app, x) = app_with_three_moves();
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    let other = app.session.active().unwrap().doc.clone();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(x(&app), 1.0);
    assert_eq!(tick_playback(&mut app, 0.0), Some(1.0));
    close(&mut app, json!({"index": 0}));
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &other));
    assert_eq!(tick_playback(&mut app, 1.0), None, "the playback ends when the next step is due");
    assert!(app.ui.action_playing.is_none());
    assert_eq!(app.ui.status, stopped);
    assert_eq!(tick_playback(&mut app, 10.0), None);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &other), "no step ran in the document left open");
}

#[test]
fn a_failing_step_ends_a_paced_playback_with_its_error() {
    let (mut app, x) = app_with_three_moves();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    // A Select Object step that finds no object fails as it plays.
    let a = first_action(&app);
    let select = ActionStep::new(SELECT_OBJECT_STEP, json!({"note": "missing", "caseSensitive": false, "wholeWord": false}));
    app.ui.actions.replace_step(a, 1, select).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert_eq!(frame(&mut app, 0.0), None, "the frame after Play draws the first step");
    assert_eq!(frame(&mut app, 0.0), None);
    assert!(app.ui.action_playing.is_none());
    assert!(!app.ui.status.is_empty(), "the step's error shows");
    assert_eq!(frame(&mut app, 0.0), None);
    assert_eq!(x(&app), 1.0, "the step after the failing one does not run");
}

#[test]
fn an_inserted_menu_item_plays_step_by_step() {
    let (mut app, x) = app_with_three_moves();
    let a = first_action(&app);
    // Only the first move stays.
    for step in [2, 1] {
        app.ui.actions.delete_step(a, step).unwrap();
    }
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "object.group"})).unwrap();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    let rect = app.session.active().unwrap().selection.objects.first().copied();
    let before = undo_steps(&app);
    let r = play(&mut app, 0, 0).unwrap();
    assert_eq!(r["pending"], json!(1), "the move ran, the group waits: {r}");
    play_out(&mut app);
    assert_eq!(undo_steps(&app), before + 2, "each step its own undo step");
    assert_eq!(x(&app), 1.0);
    let selected = app.session.active().unwrap().selection.objects.first().copied();
    assert!(selected.is_some() && selected != rect, "the group step ran and selected the new group");
}

/// At Accelerated a View menu step plays in its place: the steps before it are one undo step,
/// the View step adds none, and the steps after it are one more.
#[test]
fn an_inserted_view_item_plays_in_its_place_at_accelerated() {
    let (mut app, x) = app_with_a_view_step();
    let before = undo_steps(&app);
    let r = play(&mut app, 0, 0).unwrap();
    assert_eq!(r["results"].as_array().map(Vec::len), Some(4), "{r}");
    assert_eq!(r["results"][1], json!(true), "Outline turned on: {r}");
    assert!(app.outline_view());
    assert_eq!(x(&app), 7.0);
    assert_eq!(undo_steps(&app), before + 2, "the moves on each side of Outline are one undo step each");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(x(&app), 1.0, "Undo takes back the moves after Outline");
    assert!(app.outline_view(), "and leaves the view as it is");
}

/// A step that fails after a View menu step ends the playback: the steps before it stay done and
/// the batch it is in changes nothing.
#[test]
fn a_failing_step_after_a_view_item_keeps_what_played_before_it() {
    let (mut app, x) = app_with_a_view_step();
    let a = first_action(&app);
    app.ui.actions.insert_step(a, 3, ActionStep::new("object.align", json!({}))).unwrap();
    assert!(play(&mut app, 0, 0).is_err());
    assert!(app.outline_view(), "Outline played");
    assert_eq!(x(&app), 1.0, "the move before Outline stays, and the move of 2 in the failing batch is taken back");
}

/// A step that fails after a View menu step at Accelerated is named by its place in the action,
/// not by its place in the batch of the steps after the View step.
#[test]
fn an_error_after_a_view_item_names_the_step_in_the_action() {
    let (mut app, _) = app_with_a_view_step();
    let a = first_action(&app);
    app.ui.actions.insert_step(a, 3, ActionStep::new("object.align", json!({}))).unwrap();
    assert_names_step(&play(&mut app, 0, 0).unwrap_err(), 4, "object.align");
}

/// A View menu step that fails is named by its place in the action, excluded steps counted.
#[test]
fn a_failing_view_item_names_the_step_in_the_action() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let mut outline = action("Outline", &[("object.move", json!({"dx": 1, "dy": 0})), ("view.outline", json!({}))]);
    outline.disabled.insert(0);
    app.ui.actions.replace(ActionList::new());
    push_set(&mut app, "Test", vec![outline]);
    assert_names_step(&play(&mut app, 0, 0).unwrap_err(), 2, "view.outline");
}

/// An excluded step before the failing one counts at Accelerated, as the panel lists it.
#[test]
fn an_error_counts_excluded_steps() {
    let (mut app, x) = app_with_three_moves();
    let a = first_action(&app);
    app.ui.actions.insert_step(a, app.ui.actions.step_count(a), ActionStep::new("object.align", json!({}))).unwrap();
    app.ui.actions.set_step(a, 0, false).unwrap();
    assert_names_step(&play(&mut app, 0, 0).unwrap_err(), 4, "object.align");
    assert_eq!(x(&app), 0.0, "the failing batch changes nothing");
}

/// A step of a recorded `command.batch` that fails is named by the batch's place in the action
/// and its own command.
#[test]
fn an_error_in_a_recorded_batch_names_the_batch_step() {
    let (mut app, _) = app_with_three_moves();
    let batch = json!({"commands": [{"command": "object.move", "params": {"dx": 2, "dy": 0}}, {"command": "object.align", "params": {}}]});
    let a = first_action(&app);
    app.ui.actions.insert_step(a, 1, ActionStep::new("command.batch", batch)).unwrap();
    assert_names_step(&play(&mut app, 0, 0).unwrap_err(), 2, "object.align");
}

/// An error about the whole batch, which names no step, is kept as it is.
#[test]
fn an_error_of_the_whole_batch_names_no_step() {
    let (mut app, _) = app_with_three_moves();
    let nested = json!({"commands": [{"command": "command.batch", "params": {"commands": []}}]});
    let a = first_action(&app);
    app.ui.actions.insert_step(a, app.ui.actions.step_count(a), ActionStep::new("command.batch", nested)).unwrap();
    let e = play(&mut app, 0, 0).unwrap_err();
    assert!(!e.starts_with("action step"), "{e}");
    assert!(e.contains("batches cannot nest"), "{e}");
}

/// At Step by Step and Pause For, a later step that fails is named by its place in the action,
/// in the status bar, and the playback ends.
#[test]
fn a_paced_error_names_the_step_in_the_action() {
    for speed in ["step", "pause"] {
        let (mut app, x) = app_with_three_moves();
        let a = first_action(&app);
        app.ui.actions.insert_step(a, 2, ActionStep::new("object.align", json!({}))).unwrap();
        app.run(PLAYBACK_OPTIONS, json!({"speed": speed})).unwrap();
        play(&mut app, 0, 0).unwrap();
        let mut now = 0.0;
        for _ in 0..40 {
            if app.ui.action_playing.is_none() {
                break;
            }
            now += 2.0;
            frame(&mut app, now);
        }
        assert!(app.ui.action_playing.is_none(), "{speed}: the playback ended");
        assert_names_step(&app.ui.status, 3, "object.align");
        assert_eq!(x(&app), 3.0, "{speed}: the moves before the failing step stay, the move after it does not run");
    }
}

/// At Step by Step and Pause For a View menu step is one step of the playback and adds no undo
/// step.
#[test]
fn an_inserted_view_item_plays_step_by_step_and_with_pauses() {
    for speed in ["step", "pause"] {
        let (mut app, x) = app_with_a_view_step();
        app.run(PLAYBACK_OPTIONS, json!({"speed": speed})).unwrap();
        let before = undo_steps(&app);
        let r = play(&mut app, 0, 0).unwrap();
        assert_eq!(r["pending"], json!(3), "{speed}: {r}");
        assert!(!app.outline_view(), "{speed}: Outline waits its turn");
        let mut now = 0.0;
        for _ in 0..40 {
            if app.ui.action_playing.is_none() {
                break;
            }
            now += 2.0;
            frame(&mut app, now);
        }
        assert!(app.ui.action_playing.is_none(), "{speed}: the playback ended");
        assert!(app.outline_view(), "{speed}");
        assert_eq!(x(&app), 7.0, "{speed}");
        assert_eq!(undo_steps(&app), before + 3, "{speed}: each move its own undo step, Outline none");
    }
}

/// Playing an action with a View menu step while another records keeps the View step in the
/// recording, between the batches of the steps around it.
#[test]
fn a_view_step_played_while_recording_is_recorded() {
    let (mut app, _) = app_with_a_view_step();
    app.run(START_RECORDING, json!({})).unwrap();
    play(&mut app, 0, 0).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    let recorded = set_actions(&app, 0).pop().unwrap().steps;
    let ids: Vec<&str> = recorded.iter().map(|s| s.command.as_str()).collect();
    assert_eq!(ids, ["command.batch", "view.outline", "command.batch"]);
    assert_eq!(recorded[1].params, json!({}));
}

#[test]
fn a_select_path_step_plays_step_by_step_and_a_changed_path_ends_the_playback() {
    let (mut app, x, [_, second]) = app_with_two_rectangles();
    let a = first_action(&app);
    // Only the first move stays.
    for step in [2, 1] {
        app.ui.actions.delete_step(a, step).unwrap();
    }
    app.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0})).unwrap();
    app.ui.actions.insert_step(a, app.ui.actions.step_count(a), ActionStep::new("object.move", json!({"dx": 4, "dy": 0}))).unwrap();
    app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
    let before = undo_steps(&app);
    app.run("select.set", json!({"ids": [second]})).unwrap();
    assert_eq!(undo_steps(&app), before, "a selection change alone is no undo step");
    play(&mut app, 0, 0).unwrap();
    play_out(&mut app);
    assert_eq!(left_of(&app, second), 111.0, "the first move moved the path selected when the action started");
    assert_eq!(x(&app), 4.0, "the step selected the stored path, and the last move moved it");
    assert_eq!(undo_steps(&app), before + 2, "each move its own undo step, and the selection change none, as select.set adds none");

    // The stored path has moved since: the step ends the playback, and the move after it does not run.
    app.run("select.set", json!({"ids": [second]})).unwrap();
    play(&mut app, 0, 0).unwrap();
    play_out(&mut app);
    assert!(app.ui.status.contains("is not the stored path"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(second)], "the selection stays as it was");
    assert_eq!((left_of(&app, second), x(&app)), (112.0, 4.0), "the step before it ran, the one after it did not");
}

/// A playback whose Select Object step finds no object stops there with the step's error, at
/// every speed, and runs none of the steps after it.
#[test]
fn a_select_object_step_that_finds_nothing_stops_the_playback() {
    for speed in ["accelerated", "step"] {
        let (mut app, x, noted) = app_with_a_noted_rect();
        app.run(PLAYBACK_OPTIONS, json!({"speed": speed})).unwrap();
        let a = first_action(&app);
        app.ui.actions.insert_step(a, 0, ActionStep::new(SELECT_OBJECT_STEP, json!({"note": "missing"}))).unwrap();
        let selection = app.session.active().unwrap().selection.clone();
        let e = play(&mut app, 0, 0).unwrap_err();
        assert!(e.contains("no object that can be selected has a note matching “missing”"), "{speed}: {e}");
        play_out(&mut app);
        assert_eq!((x(&app), noted(&app)), (0.0, 0.0), "{speed}: nothing after it runs");
        assert_eq!(app.session.active().unwrap().selection, selection, "{speed}: the selection stays");
        assert!(app.ui.action_playing.is_none(), "{speed}");
    }
}
