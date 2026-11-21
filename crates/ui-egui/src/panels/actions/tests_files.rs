use super::tests_support::*;
use super::*;
use vectorcraft_engine::Session;

#[test]
fn defaults_are_valid_commands() {
    for set in default_sets() {
        for a in set.actions {
            for (c, _) in a.steps {
                assert!(vectorcraft_engine::find_command(&c).is_some(), "{c}");
            }
        }
    }
}

/// The built-in Reflect Horizontal reflects a rotated object across its bounding box's own axis,
/// as the Reflect dialog does, so the box keeps its angle; a saved step without a basis, recorded
/// before Reflect followed the box, still reflects across the page's axis.
#[test]
fn the_built_in_reflect_follows_the_turned_box() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.run("object.rotate", json!({"angle": 30})).unwrap();
    let angle = |app: &VectorcraftApp| app.session.active().unwrap().doc.bbox_angle(&[id]);
    let reflect = default_sets()[0].actions.iter().position(|a| a.name == "Reflect Horizontal").unwrap();
    app.ui.action_sets = default_sets();
    play(&mut app, 0, reflect).unwrap();
    assert!((angle(&app) - 30.0).abs() < 1e-6, "{}", angle(&app));
    app.ui.action_sets = vec![ActionSet::new("Old", vec![Action::new("Reflect", vec![("object.reflect".into(), json!({"axis": "vertical"}))])])];
    play(&mut app, 0, 0).unwrap();
    assert!((angle(&app) + 30.0).abs() < 1e-6, "{}", angle(&app));
}

#[test]
fn excluded_steps_do_not_play_and_survive_saved_sets() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let mut action =
        Action::new("Move twice", vec![("object.move".into(), json!({"dx": 5, "dy": 0})), ("object.move".into(), json!({"dx": 20, "dy": 0}))]);
    action.excluded_steps.insert(1);
    assert_eq!(
        action.included_commands(),
        [json!({"command": "object.move", "params": {"dx": 5, "dy": 0}})],
        "normal and Batch playback use the same included commands"
    );
    app.ui.action_sets = vec![ActionSet::new("Test", vec![action])];
    let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let before = x(&app);
    play(&mut app, 0, 0).unwrap();
    assert!((x(&app) - before - 5.0).abs() < 1e-9, "the excluded second move did not run");
    let saved = files::write_set(&app.ui.action_sets[0]);
    assert!(saved.contains("\"excluded\": true"), "the set file keeps the exclusion");
    assert_eq!(serde_json::from_str::<Value>(&saved).unwrap()["version"], json!(2), "older builds must refuse excluded steps");
    let loaded = files::read_set(&saved, "Test").unwrap();
    assert_eq!(loaded.actions[0].excluded_steps, BTreeSet::from([1]));
    assert_eq!(loaded.actions[0].copy().excluded_steps, BTreeSet::from([1]));
    assert_eq!(serde_json::from_value::<Action>(serde_json::to_value(&loaded.actions[0]).unwrap()).unwrap().excluded_steps, BTreeSet::from([1]));
    app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "included": false})).unwrap();
    assert_eq!(play(&mut app, 0, 0).unwrap_err(), "the action has no included steps");
    assert_eq!(crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap_err(), "the action has no included steps");
}

/// Two sets with the same name: when the asked one is deleted elsewhere while the question is
/// open, OK does not delete the other one, which now has the asked index and name; when the
/// two trade places, OK deletes the asked one at its new index.
#[test]
fn ok_deletes_only_the_asked_one_of_two_sets_with_the_same_name() {
    use super::super::tests_appearance::click;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let x = |n: &str| ActionSet::new("X", vec![Action::new(n, vec![])]);
    app.ui.action_sets = vec![x("first"), x("second")];
    let ctx = egui::Context::default();
    click_set(&ctx, &mut app, "X", egui::Modifiers::NONE);
    assert_eq!(app.ui.action_set, Some(0));
    let delete = trash(&ctx, &mut app);
    click(&ctx, &mut app, delete, show);
    assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([0])));
    app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
    assert!(crate::dialogs::confirm(&mut app).is_err());
    assert_eq!(app.ui.action_sets.iter().map(|s| s.actions[0].name.as_str()).collect::<Vec<_>>(), ["second"], "OK deletes nothing");
    app.ui.action_sets = vec![x("first"), x("second")];
    app.ui.action_set = Some(0);
    app.ui.action_sets_selected = vec![0];
    let delete = trash(&ctx, &mut app);
    click(&ctx, &mut app, delete, show);
    assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([0])));
    app.ui.action_sets.swap(0, 1);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.ui.action_sets.iter().map(|s| s.actions[0].name.as_str()).collect::<Vec<_>>(), ["second"], "the asked set goes");
}

/// When the asked set is deleted while the question is open and a new set with the same name
/// takes its place, OK does not delete the new set.
#[test]
fn ok_does_not_delete_a_new_set_made_in_the_asked_sets_place() {
    use super::super::tests_appearance::click;
    let mut app = app_with_sets();
    let ctx = egui::Context::default();
    click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
    let delete = trash(&ctx, &mut app);
    click(&ctx, &mut app, delete, show);
    assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([2])));
    app.run(DELETE_SETS, json!({"sets": [2]})).unwrap();
    app.ui.action_sets.push(ActionSet::new("Other", vec![]));
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"]);
    assert!(crate::dialogs::confirm(&mut app).is_err());
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "the new Other stays");
}

/// Every set has its own id, and sets read back from the saved preferences get new ids that
/// no other set has.
#[test]
fn action_sets_get_ids_of_their_own_when_made_and_loaded() {
    let app = app_with_sets();
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert!(saved["action_sets"][0].get("id").is_none(), "ids are not saved");
    let restored = serde_json::from_value::<crate::state::UiState>(saved).unwrap();
    let mut ids: Vec<u64> = app.ui.action_sets.iter().chain(&restored.action_sets).map(|s| s.id).collect();
    assert_eq!(restored.action_sets.len(), 3);
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6, "six sets, six ids");
}

/// The current set is saved with the UI preferences: after a restart a recording goes into the
/// set clicked before it, not into Default Actions.
#[test]
fn a_recording_after_a_restart_goes_into_the_set_current_before_it() {
    use super::super::tests_appearance::{click, frame_events};
    let mut app = app_with_sets();
    let ctx = egui::Context::default();
    click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
    let saved = serde_json::to_value(&app.ui).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui = serde_json::from_value(saved).unwrap();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    assert_eq!(app.ui.action_set, Some(2), "Other is current after the restart");
    let ctx = egui::Context::default();
    frame_events(&ctx, &mut app, vec![], show);
    let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [stop, record, _, _, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
    click(&ctx, &mut app, record.center(), show);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    click(&ctx, &mut app, stop.center(), show);
    let counts = app.ui.action_sets.iter().map(|s| s.actions.len()).collect::<Vec<_>>();
    assert_eq!(counts, [6, 2, 1], "recorded into Other");
}

/// A saved current set that is not a set index, or is past the last set, leaves no set current;
/// the other preferences still load, and Begin Recording then records into the first set and
/// makes it current.
#[test]
fn a_saved_current_set_that_is_gone_falls_back_to_the_first_set() {
    use super::super::tests_appearance::{click, frame_events};
    let app = app_with_sets();
    let mut saved = serde_json::to_value(&app.ui).unwrap();
    saved["action_set"] = json!("Other");
    let restored = serde_json::from_value::<crate::state::UiState>(saved.clone()).unwrap();
    assert_eq!((restored.action_set, restored.action_sets.len()), (None, 3), "not an index: no set current, the sets load");
    saved["action_set"] = json!(9);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui = serde_json::from_value(saved).unwrap();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    frame_events(&ctx, &mut app, vec![], show);
    let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [stop, record, _, _, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
    click(&ctx, &mut app, record.center(), show);
    assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(0), vec![0]), "the set recorded into is current");
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    click(&ctx, &mut app, stop.center(), show);
    let counts = app.ui.action_sets.iter().map(|s| s.actions.len()).collect::<Vec<_>>();
    assert_eq!(counts, [7, 2, 0], "recorded into Default Actions");
}

/// `ui.actionSet.replace` makes the file's set current and selected, clears the panel's selected
/// action and step, expanded actions and a dragged step, closes a dialog that holds a set index
/// and records no undo step. The same file again changes nothing and keeps the ids. A set with
/// no actions replaces the sets too, an `.aia` file reports its left-out steps, and a file with
/// an unknown command reports it, as Load Actions… does; a set with no name takes the file's.
#[test]
fn replace_actions_clears_what_named_the_old_sets_and_reads_aia_files() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let undo = undo_steps(&app);
    app.ui.action_set = Some(2);
    app.ui.action_sets_selected = vec![1, 2];
    let ctx = egui::Context::default();
    app.ui_ctx = Some(ctx.clone());
    let nudge = app.ui.action_sets[1].actions[1].id;
    set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((1, 1)));
    set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((nudge, 0)));
    set_pstate(&ctx, "actions-expanded", BTreeSet::from([nudge]));
    let dragged: DraggedStep = (nudge, 0, std::sync::Arc::new((vec![], BTreeSet::new())));
    set_pstate::<Option<DraggedStep>>(&ctx, DRAGGED_STEP, Some(dragged));
    crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();

    let text = files::write_set(&app.ui.action_sets[1]);
    let r = app.run(REPLACE_SET, json!({ "data": text })).unwrap();
    assert_eq!(r, json!({"changed": true, "set": 0, "name": "Mine", "actions": 2}));
    assert_eq!(set_names(&app), ["Mine"]);
    assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(0), vec![0]));
    assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), None);
    assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), None);
    assert!(pstate::<BTreeSet<u64>>(&ctx, "actions-expanded").is_empty());
    assert!(pstate::<Option<DraggedStep>>(&ctx, DRAGGED_STEP).is_none());
    assert!(app.ui.dialog.is_none(), "the Batch dialog of a replaced set closes");
    assert_eq!(undo_steps(&app), undo, "no undo step");

    let ids = (app.ui.action_sets[0].id, app.ui.action_sets[0].actions.iter().map(|a| a.id).collect::<Vec<_>>());
    assert_eq!(app.run(REPLACE_SET, json!({ "data": text })).unwrap()["changed"], false, "the same set, shown that way");
    assert_eq!((app.ui.action_sets[0].id, app.ui.action_sets[0].actions.iter().map(|a| a.id).collect()), ids, "the ids stay");

    let empty = json!({"format": "vcactions", "version": 1, "name": "Empty", "actions": []}).to_string();
    assert_eq!(app.run(REPLACE_SET, json!({ "data": empty })).unwrap(), json!({"changed": true, "set": 0, "name": "Empty", "actions": 0}));
    assert_eq!(set_names(&app), ["Empty"]);

    let aia = aia_text("Tidy Up", &[("Group Them", &[("group", true), ("Move", true)])]);
    let r = app.run(REPLACE_SET, json!({ "data": aia })).unwrap();
    let skipped = json!([{"action": "Group Them", "step": "Move"}]);
    assert_eq!(r, json!({"changed": true, "set": 0, "name": "Tidy Up", "actions": 1, "skipped": skipped, "leftOut": 1}));
    assert_eq!(set_names(&app), ["Tidy Up"]);
    assert_eq!(app.ui.action_sets[0].actions[0].steps, [("object.group".to_string(), json!({}))]);
    assert_eq!(app.ui.status, "Loaded “Tidy Up”: 1 step was left out");

    let files = with_files(&mut app, "/sets/unused.vcactions");
    let odd = json!({"format": "vcactions", "version": 1, "name": "", "actions": [{"name": "A", "steps": [{"command": "example.unknown"}]}]});
    files.borrow_mut().insert("/sets/Print Jobs.vcactions".into(), odd.to_string().into_bytes());
    let r = app.run(REPLACE_SET, json!({"path": "/sets/Print Jobs.vcactions"})).unwrap();
    assert_eq!((r["name"].clone(), r["unknownCount"].clone(), r["changed"].clone()), (json!("Print Jobs"), json!(1), json!(true)));
    assert_eq!(set_names(&app), ["Print Jobs"]);
}

/// `ui.actionSet.replace` refuses params that are not an object, another key, a `path` or `data`
/// that is not text, a file that can't be read or that Load Actions… refuses (an empty one too),
/// a canceled pick, and a replace while an action records or plays, waiting at a stop included,
/// and then changes no set and keeps the current set.
#[test]
fn replace_actions_refuses_bad_params_bad_files_and_a_running_action() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/missing.vcactions");
    files.borrow_mut().insert("/sets/empty.vcactions".into(), vec![]);
    app.ui.action_set = Some(1);
    let before = saved_sets(&app.ui.action_sets);
    let many = json!({"format": "vcactions", "version": 1, "name": "Many", "actions": vec![json!({"name": "A", "steps": []}); 1001]});
    let eof = "not an action set file (EOF while parsing a value at line 1 column 0)";
    for (p, error) in [
        (json!(1), "the params must be an object, not 1"),
        (json!([]), "the params must be an object, not []"),
        (json!({"set": 0}), "unknown parameter `set`; the command takes: path, data"),
        (json!({"path": 5}), "`path` must be a string"),
        (json!({"path": null}), "`path` must be a string"),
        (json!({"data": 42}), "`data` must be a string"),
        (json!({"data": ""}), eof),
        (json!({"data": "GIMP Palette"}), "not an action set file (expected value at line 1 column 1)"),
        (json!({"data": many.to_string()}), "the action set file holds more than 1,000 actions"),
        (json!({"data": " ".repeat(files::MAX_SET_FILE + 1)}), "the action set file is larger than 8 MiB"),
        (json!({"data": "/version\n"}), "not an action set file"),
        (json!({"path": "/sets/empty.vcactions"}), eof),
        (json!({"path": "/sets/nowhere.vcactions"}), "no such file"),
        (json!({}), "no such file"),
    ] {
        assert_eq!(app.run(REPLACE_SET, p.clone()).unwrap_err(), error, "{p}");
        assert_eq!(saved_sets(&app.ui.action_sets), before, "{p} changes nothing");
        assert_eq!(app.ui.action_set, Some(1));
    }
    app.services.pick_open = Some(Box::new(|_: &crate::FilePick| None));
    assert_eq!(app.run(REPLACE_SET, Value::Null).unwrap_err(), "cancelled");
    assert_eq!(saved_sets(&app.ui.action_sets), before, "a canceled pick changes nothing");

    let text = files::write_set(&app.ui.action_sets[1]);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(REPLACE_SET, json!({ "data": text })).unwrap_err(), "an action is already recording");
    assert_eq!(saved_sets(&app.ui.action_sets), before, "a recording keeps its set");
    app.run(STOP_RECORDING, json!({})).unwrap();

    let (mut app, _) = app_with_a_stop(true);
    let before = saved_sets(&app.ui.action_sets);
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    assert_eq!(app.run(REPLACE_SET, json!({ "data": text })).unwrap_err(), "an action is already playing");
    assert_eq!(saved_sets(&app.ui.action_sets), before, "the playing set stays");
}

/// The panel menu's Save Actions… writes the current set with its actions to a file named
/// after it, and Load Actions… adds the set it reads after the others, with the same actions
/// and steps; a set named like one in the panel gets the next free number, and the other sets
/// and the current set stay as they are.
#[test]
fn save_actions_and_load_actions_round_trip_a_set() {
    use super::super::tests_appearance::{click, frame_events};
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/Mine.vcactions");
    let ctx = egui::Context::default();
    let item = |ctx: &egui::Context, app: &mut VectorcraftApp, label: &str| {
        let texts = frame_events(ctx, app, vec![], menu);
        texts.iter().find(|(s, _)| s.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{label} in the menu"))
    };
    let save = item(&ctx, &mut app, "Save Actions…");
    click(&ctx, &mut app, save, menu);
    assert!(files.borrow().is_empty(), "no set is current: nothing to save");
    click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
    let save = item(&ctx, &mut app, "Save Actions…");
    click(&ctx, &mut app, save, menu);
    let text = String::from_utf8(files.borrow()["/sets/Mine.vcactions"].clone()).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!((v["format"].clone(), v["version"].clone(), v["name"].clone()), (json!("vcactions"), json!(1), json!("Mine")));
    assert_eq!(v["actions"][1], json!({"name": "Nudge Again", "steps": [{"command": "object.move", "params": {"dx": 1, "dy": 0}}]}));
    let load = item(&ctx, &mut app, "Load Actions…");
    click(&ctx, &mut app, load, menu);
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other", "Mine 2"], "added after the others, numbered");
    assert_eq!(app.ui.action_set, Some(1), "the current set stays");
    let steps = |s: &ActionSet| s.actions.iter().map(|a| (a.name.clone(), a.steps.clone())).collect::<Vec<_>>();
    assert_eq!(steps(&app.ui.action_sets[3]), steps(&app.ui.action_sets[1]), "the same actions and steps");
    let r = app.run(LOAD_SET, json!({"data": text})).unwrap();
    assert_eq!(r, json!({"set": 4, "name": "Mine 3", "actions": 2}));
    assert_ne!(app.ui.action_sets[3].id, app.ui.action_sets[4].id, "each loaded set has an id of its own");
}

/// `ui.actionSet.save` saves the set at `set`, else the current one, and `ui.actionSet.load` of
/// a set no other set is named like keeps its name; one with no name takes the file's.
#[test]
fn load_actions_adds_a_set_and_keeps_the_others() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/Other.vcactions");
    assert_eq!(app.run(SAVE_SET, json!({})).unwrap_err(), "no action set to save");
    assert_eq!(app.run(SAVE_SET, json!({"set": 3})).unwrap_err(), "no such action set");
    assert_eq!(app.run(SAVE_SET, json!({"set": 1, "path": "/sets/kept.vcactions"})).unwrap(), json!({"path": "/sets/kept.vcactions"}));
    let text = String::from_utf8(files.borrow()["/sets/kept.vcactions"].clone()).unwrap();
    app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
    assert_eq!(app.run(LOAD_SET, json!({"path": "/sets/kept.vcactions"})).unwrap(), json!({"set": 2, "name": "Mine", "actions": 2}));
    assert_eq!(set_names(&app), ["Default Actions", "Other", "Mine"]);
    let unnamed = text.replacen("\"name\": \"Mine\"", "\"name\": \"\"", 1);
    files.borrow_mut().insert("/sets/Print Jobs.vcactions".into(), unnamed.into_bytes());
    assert_eq!(app.run(LOAD_SET, json!({"path": "/sets/Print Jobs.vcactions"})).unwrap()["name"], json!("Print Jobs"));
    assert_eq!(app.run(SAVE_SET, json!({"set": 0, "path": "/sets/d.vcactions"})).unwrap(), json!({"path": "/sets/d.vcactions"}));
    assert_eq!(files::set_file_stem("a/b: c?."), "a-b- c-");
    for (name, stem) in
        [("CON", "CON-"), ("nul", "nul-"), ("Com1.print", "Com1-.print"), ("lpt²", "lpt²-"), ("COM10", "COM10"), ("Console", "Console")]
    {
        assert_eq!(files::set_file_stem(name), stem, "{name}");
    }
}

/// `ui.actionSet.save` refuses a set with a step that sets a folder preference to a folder, as
/// `{key, value}`, in `values` or in a batch step's commands at any depth, with an error that names
/// the action and the preference, and writes no file. An empty folder, a scratch disk choice and
/// other preferences save.
#[test]
fn save_actions_refuses_a_step_that_sets_a_folder_preference() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/x.vcactions");
    let step = |command: &str, params: Value| (command.to_string(), params);
    let nested = json!({"commands": [{"command": "command.batch", "params": {"commands": [
        {"command": "prefs.set", "params": {"values": {"fontsFolder": "fonts"}}},
    ]}}]});
    for (steps, key) in [
        (vec![step("prefs.set", json!({"key": "scratchPrimary", "value": "scratch"}))], "scratchPrimary"),
        (vec![step("prefs.set", json!({"values": {"uiAccent": "green", "templatesFolder": "/Users/me/Templates"}}))], "templatesFolder"),
        (
            vec![
                step("object.move", json!({"dx": 1, "dy": 0})),
                step(
                    "command.batch",
                    json!({"commands": [{"command": "prefs.set", "params": {"key": "scratchSecondary", "value": " /Volumes/Fast "}}]}),
                ),
            ],
            "scratchSecondary",
        ),
        (vec![step("command.batch", nested)], "fontsFolder"),
    ] {
        app.ui.action_sets[1] = ActionSet::new("Other", vec![Action::new("Scratch", steps)]);
        let e = app.run(SAVE_SET, json!({"set": 1, "path": "/sets/x.vcactions"})).unwrap_err();
        assert_eq!(e, format!("the action “Scratch” sets the folder preference {key}; delete that step to save the set"));
        assert!(files.borrow().is_empty(), "no file is written for {key}");
    }
    let kept = vec![
        step("prefs.set", json!({"values": {"scratchPrimary": "None", "scratchSecondary": " Startup ", "fontsFolder": "", "uiAccent": "green"}})),
        step("prefs.set", json!({"key": "pluginsFolder", "value": ""})),
        step("x.y", json!({"key": "fontsFolder", "value": "fonts"})),
    ];
    app.ui.action_sets[1] = ActionSet::new("Other", vec![Action::new("Scratch", kept)]);
    assert_eq!(app.run(SAVE_SET, json!({"set": 1, "path": "/sets/x.vcactions"})).unwrap(), json!({"path": "/sets/x.vcactions"}));
    assert!(files.borrow().contains_key("/sets/x.vcactions"));
}

/// A file that is not an action set file, or is too large, is refused with a message, and no
/// set is added.
#[test]
fn load_actions_refuses_a_file_that_is_not_an_action_set() {
    let mut app = app_with_sets();
    let before = set_names(&app);
    let many = json!({"format": "vcactions", "version": 1, "name": "Many", "actions": vec![json!({"name": "A", "steps": []}); 1001]});
    let steps = vec![json!({"command": "object.move"}); 10_001];
    let long = json!({"format": "vcactions", "version": 1, "name": "Long", "actions": [{"name": "A", "steps": steps}]});
    for (data, want) in [
        (json!("GIMP Palette"), "not an action set file (expected value at line 1 column 1)"),
        (json!("{\"format\": \"vcstyles\", \"version\": 1, \"name\": \"S\", \"actions\": []}"), "not an action set file"),
        (json!(42), "not an action set file"),
        (json!(many.to_string()), "the action set file holds more than 1,000 actions"),
        (json!(long.to_string()), "an action in the action set file has more than 10,000 steps"),
        (json!(" ".repeat(files::MAX_SET_FILE + 1)), "the action set file is larger than 8 MiB"),
        (
            json!("{\"format\": \"vcactions\", \"version\": 8, \"name\": \"S\", \"actions\": []}"),
            "the action set file needs a later version of VectorCraft",
        ),
    ] {
        assert_eq!(app.run(LOAD_SET, json!({ "data": data })).unwrap_err(), want);
    }
    with_files(&mut app, "/sets/missing.vcactions");
    assert_eq!(app.run(LOAD_SET, json!({})).unwrap_err(), "no such file");
    assert_eq!(set_names(&app), before, "no set added");
}

/// A step whose command is not known loads as it is, excluded or not and in a `command.batch`
/// step too, from `data` and from `path`: the answer lists each one and the status bar names the
/// first and how many there are. A set whose commands are all known loads with neither.
#[test]
fn load_actions_reports_steps_with_unknown_commands() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/unused.vcactions");
    let batch = json!({"commands": [{"command": "object.move", "params": {"dx": 1, "dy": 0}}, {"command": "example.unknown.inner"}]});
    let export = json!([
        {"command": "object.move", "params": {"dx": 1, "dy": 0}},
        {"command": "view.zoomIn", "params": {}},
        {"command": STOP_STEP, "params": {"message": "", "allowContinue": false}},
        {"command": "example.unknown.finish", "params": {"x": [1, 2]}},
        {"command": "command.batch", "params": batch},
        {"command": "example.unknown.off", "params": {}, "excluded": true},
    ]);
    let tidy = json!({"name": "Tidy", "steps": [{"command": "object.move", "params": {"dx": 1, "dy": 0}}]});
    let file = json!({"format": "vcactions", "version": 6, "name": "Print", "actions": [tidy, {"name": "Export", "steps": export}]}).to_string();
    let want = json!([
        {"action": "Export", "actionIndex": 1, "step": 4, "stepIndex": 3, "command": "example.unknown.finish"},
        {"action": "Export", "actionIndex": 1, "step": 5, "stepIndex": 4, "batchStep": 2, "command": "example.unknown.inner"},
        {"action": "Export", "actionIndex": 1, "step": 6, "stepIndex": 5, "command": "example.unknown.off"},
    ]);
    let r = app.run(LOAD_SET, json!({"data": file})).unwrap();
    assert_eq!(r, json!({"set": 3, "name": "Print", "actions": 2, "unknownCommands": want, "unknownCount": 3}));
    assert_eq!(app.ui.status, "Loaded “Print”: 3 steps have unknown commands and were kept; the first: “Export”, step 4, `example.unknown.finish`");
    let loaded = &app.ui.action_sets[3].actions[1];
    assert_eq!(loaded.steps.len(), 6, "every step is kept");
    assert_eq!(loaded.steps[3], ("example.unknown.finish".to_string(), json!({"x": [1, 2]})), "with its params");
    assert_eq!(loaded.excluded_steps, BTreeSet::from([5]));
    files.borrow_mut().insert("/sets/Print.vcactions".into(), file.into_bytes());
    let r = app.run(LOAD_SET, json!({"path": "/sets/Print.vcactions"})).unwrap();
    assert_eq!((r["name"].clone(), r["unknownCommands"].clone(), r["unknownCount"].clone()), (json!("Print 2"), want, json!(3)));
    let saved = files::write_set(&app.ui.action_sets[4]);
    assert!(saved.contains("example.unknown.finish") && saved.contains("example.unknown.inner"), "Save Actions… writes them back");
    app.ui.status.clear();
    let known = files::write_set(&app.ui.action_sets[1]);
    assert_eq!(app.run(LOAD_SET, json!({"data": known})).unwrap(), json!({"set": 5, "name": "Mine 2", "actions": 2}));
    assert_eq!(app.ui.status, "", "no message for a set whose commands are all known");
    let empty = json!({"format": "vcactions", "version": 1, "name": "Empty", "actions": []}).to_string();
    assert_eq!(app.run(LOAD_SET, json!({"data": empty})).unwrap(), json!({"set": 6, "name": "Empty", "actions": 0}));
    assert_eq!(app.ui.status, "");
}

/// Load Actions… lists the first 1,000 steps with an unknown command and counts them all, and
/// the status bar shows an action name or command id with no control character and at most 80
/// characters, while the set and the answer keep them as they are. One step is named in the
/// singular.
#[test]
fn load_actions_bounds_what_it_reports_of_unknown_commands() {
    let mut app = app_with_sets();
    let steps = vec![json!({"command": "example.unknown"}); 1001];
    let many = json!({"format": "vcactions", "version": 1, "name": "Many", "actions": [{"name": "A", "steps": steps}]});
    let r = app.run(LOAD_SET, json!({"data": many.to_string()})).unwrap();
    assert_eq!((r["unknownCommands"].as_array().map(Vec::len), r["unknownCount"].clone()), (Some(1000), json!(1001)));
    assert_eq!(r["unknownCommands"][999], json!({"action": "A", "actionIndex": 0, "step": 1000, "stepIndex": 999, "command": "example.unknown"}));
    let (name, id) = ("Odd\u{1b}[31m\nname", format!("example.\r\n{}", "x".repeat(200)));
    let odd = json!({"format": "vcactions", "version": 1, "name": "Odd", "actions": [{"name": name, "steps": [{"command": id}]}]});
    let r = app.run(LOAD_SET, json!({"data": odd.to_string()})).unwrap();
    assert_eq!((r["unknownCommands"][0]["action"].clone(), r["unknownCommands"][0]["command"].clone()), (json!(name), json!(id)));
    assert_eq!(app.ui.action_sets.last().unwrap().actions[0].steps[0].0, id, "the set keeps the id");
    let shown_id = format!("example.\u{fffd}\u{fffd}{}…", "x".repeat(70));
    assert_eq!(
        app.ui.status,
        format!("Loaded “Odd”: 1 step has an unknown command and was kept: “Odd\u{fffd}[31m\u{fffd}name”, step 1, `{shown_id}`")
    );
}

/// Where the host can read part of a file, Load Actions… reads one byte past 8 MiB: a larger
/// file is refused without reading all of it, and a smaller one loads.
#[test]
fn load_actions_reads_a_file_no_further_than_one_byte_past_its_limit() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/big.vcactions");
    app.run(SAVE_SET, json!({"set": 1, "path": "/sets/small.vcactions"})).unwrap();
    files.borrow_mut().insert("/sets/big.vcactions".into(), vec![b' '; files::MAX_SET_FILE + 100]);
    app.services.read = Some(Box::new(|_: &str| Err("the whole file was read".to_string())));
    let asked = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let (f, a) = (files.clone(), asked.clone());
    app.services.read_head = Some(Box::new(move |path: &str, max: usize| {
        a.borrow_mut().push(max);
        let bytes = f.borrow().get(path).cloned().ok_or_else(|| "no such file".to_string())?;
        Ok(bytes.into_iter().take(max).collect())
    }));
    let before = set_names(&app);
    assert_eq!(app.run(LOAD_SET, json!({"path": "/sets/big.vcactions"})).unwrap_err(), "the action set file is larger than 8 MiB");
    assert_eq!(set_names(&app), before, "no set added");
    assert_eq!(app.run(LOAD_SET, json!({"path": "/sets/small.vcactions"})).unwrap()["name"], json!("Mine 2"));
    assert_eq!(*asked.borrow(), [files::MAX_SET_FILE + 1; 2], "each read stops one byte past the limit");
}

/// Load Actions… reads an action set another app saved as text: its actions come in with the
/// steps with no recorded values whose label is a VectorCraft command's that takes no values and
/// has no dialog, a step that is off stays excluded from playback, the other steps are left out
/// and listed, and the status bar says how many. An action plays the steps that are on.
#[test]
fn load_actions_reads_another_apps_action_set() {
    let mut app = app_with_sets();
    let text =
        aia_text("Tidy Up", &[("Group Them", &[("group", true), ("Move", true), ("Ungroup", false)]), ("Nothing", &[]), ("", &[("Ungroup", true)])]);
    let r = app.run(LOAD_SET, json!({ "data": text })).unwrap();
    let skipped = json!([{"action": "Group Them", "step": "Move"}]);
    assert_eq!(r, json!({"set": 3, "name": "Tidy Up", "actions": 3, "skipped": skipped, "leftOut": 1}));
    let set = &app.ui.action_sets[3];
    assert_eq!(set.actions[0].name, "Group Them");
    assert_eq!(set.actions[0].steps, [("object.group".to_string(), json!({})), ("object.ungroup".to_string(), json!({}))]);
    assert_eq!(set.actions[0].excluded_steps, [1].into());
    assert!(set.actions[1].steps.is_empty());
    assert_eq!(set.actions[2].name, "Action 3", "an action with no name");
    assert_eq!(app.ui.status, "Loaded “Tidy Up”: 1 step was left out");
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for x in [0, 50] {
        app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 2);
    play(&mut app, 3, 0).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1, "the action grouped the rectangles and did not ungroup them");
}

/// A step recorded with values is left out even when a command has its label: a step that set an
/// opacity does not select by opacity. A step whose values name a menu command, in two or three
/// values, is left out too. The status bar counts the steps left out.
#[test]
fn recorded_values_keep_a_step_out() {
    let hex = |t: &str| format!("[ {} {} ]", t.len(), t.bytes().map(|b| format!("{b:02x}")).collect::<String>());
    let text = |t: &str| format!("/type (ustring) /value {}", hex(t));
    let number = || "/type (real) /value 60.0".to_string();
    let menu = |shown: &str| vec![text("some_command"), text(shown), number()];
    let steps = [
        ("Opacity", vec![number()]),
        ("Stroke Weight", vec![number()]),
        ("A Menu Step", menu("Group")),
        ("A Menu Step", vec![text("some_command"), text("Ungroup")]),
        ("Rename", vec![text("OldLayer"), text("Group")]),
        ("Group", vec![number(), text("Group"), number()]),
        ("Group", vec![]),
    ];
    let events: String = steps
        .iter()
        .enumerate()
        .map(|(i, (label, values))| {
            let params: String = values.iter().enumerate().map(|(n, v)| format!("/parameter-{} {{ {v} }} ", n + 1)).collect();
            format!("/event-{} {{ /localizedName {} /parameterCount {} {params} }} ", i + 1, hex(label), values.len())
        })
        .collect();
    let set = format!("/version 3 /name {} /action-1 {{ /name {} {events} }}", hex("Values"), hex("A"));
    let mut app = app_with_sets();
    let r = app.run(LOAD_SET, json!({ "data": set })).unwrap();
    assert_eq!(app.ui.action_sets[3].actions[0].steps, [("object.group".to_string(), json!({}))]);
    assert_eq!(r["leftOut"], 6);
    let left_out: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|s| s["step"].as_str().unwrap()).collect();
    assert_eq!(left_out, ["Opacity", "Stroke Weight", "A Menu Step", "A Menu Step", "Rename", "Group"]);
    assert_eq!(app.ui.status, "Loaded “Values”: 6 steps were left out");
}

/// A step with no label, or a blank one, is left out and listed by its internal name, even when
/// that name is a command's label.
#[test]
fn a_step_with_no_label_is_left_out() {
    let set = "/version 3 /name [ 4 4e6f6e65 ] /action-1 { /name [ 1 41 ] \
                   /event-1 { /internalName (group) /parameterCount 0 } \
                   /event-2 { /internalName (Ungroup) /localizedName [ 1 20 ] /parameterCount 0 } \
                   /event-3 { /internalName (x) /localizedName [ 5 47726f7570 ] /parameterCount 0 } }";
    let mut app = app_with_sets();
    let r = app.run(LOAD_SET, json!({ "data": set })).unwrap();
    assert_eq!(app.ui.action_sets[3].actions[0].steps, [("object.group".to_string(), json!({}))]);
    assert_eq!(r["skipped"], json!([{"action": "A", "step": "group"}, {"action": "A", "step": "Ungroup"}]));
    assert_eq!(r["leftOut"], 2);
}

/// A Set Selection step written from scratch, as `/event-N` text: its label (none when empty),
/// whether it is on, and its recorded values, each the text inside a `/parameter-N` block.
fn set_selection_event(n: usize, label: &str, on: bool, values: &[String]) -> String {
    let hex = |t: &[u8]| format!("[ {} {} ]", t.len(), t.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let label = if label.is_empty() { String::new() } else { format!("/localizedName {} ", hex(label.as_bytes())) };
    let params: String = values.iter().enumerate().map(|(i, v)| format!("/parameter-{} {{ {v} }} ", i + 1)).collect();
    format!(
        "/event-{n} {{ /useRulersIn1stQuadrant 0 /internalName (adobe_setSelection) {label}/isOn {} /isOff 0 /parameterCount {} {params}}} ",
        u8::from(on),
        values.len()
    )
}

/// The recorded values of a Set Selection step: the note's bytes, then Whole Word and Case
/// Sensitive.
fn set_selection_values(note: &[u8], whole_word: u8, case_sensitive: u8) -> Vec<String> {
    let hex = |t: &[u8]| format!("[ {} {} ]", t.len(), t.iter().map(|b| format!("{b:02x}")).collect::<String>());
    vec![
        format!("/key 1952807028 /showInPalette 4294967295 /type (ustring) /value {}", hex(note)),
        format!("/key 2003792484 /showInPalette 4294967295 /type (boolean) /value {whole_word}"),
        format!("/key 1667330917 /showInPalette 4294967295 /type (boolean) /value {case_sensitive}"),
    ]
}

/// Load Actions… reads an `.aia` Set Selection step, found by its internal name whatever its
/// label, as a Select Object step with its note (trimmed), Case Sensitive and Whole Word, in any
/// order; a step that is off stays excluded. The set saves to VectorCraft's own file and loads
/// back the same, and playing it selects the objects with that note.
#[test]
fn load_actions_reads_set_selection_steps_as_select_object() {
    let mut reordered = set_selection_values("Größe ✓".as_bytes(), 1, 0);
    reordered.reverse();
    let events = [
        set_selection_event(1, "Set Selection", true, &set_selection_values(b" Badge front ", 0, 1)),
        set_selection_event(2, "Auswahl festlegen", true, &reordered),
        set_selection_event(3, "", false, &set_selection_values(b"Off", 1, 1)),
    ]
    .concat();
    let text = format!("/version 3 /name [ 5 4e6f746573 ] /actionCount 1 /action-1 {{ /name [ 1 41 ] /eventCount 3 {events}}}");
    let mut app = app_with_sets();
    let r = app.run(LOAD_SET, json!({ "data": text })).unwrap();
    assert_eq!((r["leftOut"].clone(), r["skipped"].clone()), (json!(0), json!([])));
    let step = |note: &str, case_sensitive: bool, whole_word: bool| {
        (SELECT_OBJECT_STEP.to_string(), json!({"note": note, "caseSensitive": case_sensitive, "wholeWord": whole_word}))
    };
    let expected = [step("Badge front", true, false), step("Größe ✓", false, true), step("Off", true, true)];
    assert_eq!(app.ui.action_sets[3].actions[0].steps, expected);
    assert_eq!(app.ui.action_sets[3].actions[0].excluded_steps, [2].into());

    let saved = files::write_set(&app.ui.action_sets[3]);
    assert!(saved.contains("\"version\": 4"), "{saved}");
    app.run(LOAD_SET, json!({ "data": saved })).unwrap();
    assert_eq!(app.ui.action_sets[4].actions[0].steps, expected);
    assert_eq!(app.ui.action_sets[4].actions[0].excluded_steps, [2].into());

    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut ids = vec![];
    for x in [0, 50] {
        ids.push(app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    }
    app.run("attributes.set", json!({"ids": [ids[1]], "note": "Badge front"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.ui.action_sets[3].actions[0].excluded_steps = [1, 2].into();
    play(&mut app, 3, 0).unwrap();
    let selected: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    assert_eq!(selected, [ids[1]], "the step selected the object with the note");
}

/// A Set Selection step whose recorded values are not exactly the note (UTF-8 text, not empty
/// once trimmed, at most 1,024 bytes) and the two options (each 0 or 1), with the right types and
/// no other value, is left out, listed by its label (else its internal name) and counted; no step
/// comes in with a missing option filled in. A value whose byte count is wrong refuses the file.
#[test]
fn set_selection_steps_with_bad_values_are_left_out() {
    let good = || set_selection_values(b"Badge", 0, 0);
    let with = |i: usize, value: &str| {
        let mut v = good();
        v[i] = value.to_string();
        v
    };
    let mut extra = good();
    extra.push("/key 1 /type (boolean) /value 0".into());
    let mut duplicate = good();
    duplicate[0] = good()[2].clone();
    let bad: Vec<Vec<String>> = vec![
        good()[1..].to_vec(),
        vec![good()[0].clone(), good()[1].clone()],
        extra,
        duplicate,
        with(0, "/key 1952807028 /type (ustring) /value [ 2 c328 ]"),
        with(0, "/key 1952807028 /type (ustring) /value [ 3 202020 ]"),
        with(0, &format!("/key 1952807028 /type (ustring) /value [ 1025 {} ]", "61".repeat(1025))),
        with(0, "/key 1952807028 /type (text) /value [ 5 4261646765 ]"),
        with(0, "/key 1952807028 /type (ustring) /value 5"),
        with(1, "/key 2003792484 /type (boolean) /value 2"),
        with(1, "/key 2003792484 /type (integer) /value 1"),
        with(2, "/key 1667330917 /type (boolean) /value 1.0"),
        with(2, "/key 1667330917 /type (boolean)"),
        with(2, "/key 1 /type (boolean) /value 1"),
    ];
    let events: String = bad.iter().enumerate().map(|(i, values)| set_selection_event(i + 1, "Set Selection", true, values)).collect();
    let events = events + &set_selection_event(bad.len() + 1, "", true, &[]);
    let text = format!("/version 3 /name [ 3 426164 ] /action-1 {{ /name [ 1 41 ] {events}}}");
    let mut app = app_with_sets();
    let r = app.run(LOAD_SET, json!({ "data": text })).unwrap();
    assert!(app.ui.action_sets[3].actions[0].steps.is_empty(), "{:?}", app.ui.action_sets[3].actions[0].steps);
    assert_eq!(r["leftOut"], bad.len() + 1);
    let names: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|s| s["step"].as_str().unwrap()).collect();
    let mut expected = vec!["Set Selection"; bad.len()];
    expected.push("adobe_setSelection");
    assert_eq!(names, expected);
    assert_eq!(app.ui.status, format!("Loaded “Bad”: {} steps were left out", bad.len() + 1));

    let wrong_count = set_selection_event(1, "Set Selection", true, &with(0, "/key 1952807028 /type (ustring) /value [ 6 4261646765 ]"));
    let text = format!("/version 3 /action-1 {{ {wrong_count}}}");
    assert_eq!(app.run(LOAD_SET, json!({ "data": text })).unwrap_err(), "the action set file can't be read in full");
    assert_eq!(app.ui.action_sets.len(), 4, "no set added");
}

/// Load Actions… picks action set files of both kinds, and a set another app saved without a
/// name takes the file's name, or `Actions` when there is no file; one named like a set in the
/// panel gets the next free number. A file that isn't UTF-8 still reads. A set past the reader's
/// caps, one whose text ends inside a block, one whose counts differ from what follows and one
/// with a damaged name are refused, and text that isn't an action set is not read.
#[test]
fn load_actions_names_another_apps_set_after_its_file() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/Mine.aia");
    let mut bytes = aia_text("", &[("One", &[("Ungroup", true)])]).into_bytes();
    bytes.extend(b"/note (\xFF)\n");
    files.borrow_mut().insert("/sets/Mine.aia".into(), bytes);
    let r = app.run(LOAD_SET, json!({})).unwrap();
    assert_eq!((r["name"].clone(), r["leftOut"].clone()), (json!("Mine 2"), json!(0)));
    assert_eq!(app.ui.action_sets[3].actions[0].steps, [("object.ungroup".to_string(), json!({}))]);
    let actions = vec![("A", [("Group", true)].as_slice()); 1001];
    let r = app.run(LOAD_SET, json!({ "data": aia_text("Big", &actions) }));
    assert_eq!(r.unwrap_err(), "the action set file can't be read in full");
    let cut = aia_text("Cut", &[("A", &[("Group", true)])]);
    let cut = cut.trim_end().strip_suffix('}').unwrap().to_string();
    assert_eq!(app.run(LOAD_SET, json!({ "data": cut })).unwrap_err(), "the action set file can't be read in full");
    let counts = aia_text("Counts", &[("A", &[]), ("B", &[])]).replace("/actionCount 2", "/actionCount 1");
    assert_eq!(app.run(LOAD_SET, json!({ "data": counts })).unwrap_err(), "the action set file can't be read in full");
    for damaged in ["/version 3 /action-1 { /name [ 3 41 ] }", "/version 3 /name [ 3 41 ]"] {
        assert_eq!(app.run(LOAD_SET, json!({ "data": damaged })).unwrap_err(), "the action set file can't be read in full");
    }
    for not_a_set in ["/version three", "/versionX 3"] {
        assert_eq!(app.run(LOAD_SET, json!({ "data": not_a_set })).unwrap_err(), "not an action set file");
    }
    assert_eq!(app.ui.action_sets.len(), 4, "no set added");
    let r = app.run(LOAD_SET, json!({ "data": aia_text("", &[]) })).unwrap();
    assert_eq!(r["name"], "Actions");
}

/// A step stands for a command only when no other menu command has its label, and the command is
/// recordable, runs with no values, has no dialog and sits directly in a top menu. The item of a
/// submenu stands for nothing: its label alone does not say which command it is.
#[test]
fn steps_stand_for_commands_that_run_with_no_values() {
    assert_eq!(files::step_command("Group"), Some("object.group"));
    assert_eq!(files::step_command(" GROUP... "), Some("object.group"));
    for label in ["Move", "Move…", "", "No Such Command"] {
        assert_eq!(files::step_command(label), None, "{label}");
    }
    let submenu_items = ["Opacity", "Stroke Weight", "Fill Color", "Blending Mode", "Grid", "Radial", "Create", "Delete All"];
    let matched: Vec<&str> = submenu_items.into_iter().filter(|label| files::step_command(label).is_some()).collect();
    assert!(matched.is_empty(), "{matched:?}");
    // The menu bar's Crop Image item runs another command, which opens the crop tool.
    let not_on_the_menu_bar = ["Crop Image", "Expand Gradient Mesh", "Duplicate"];
    let matched: Vec<&str> = not_on_the_menu_bar.into_iter().filter(|label| files::step_command(label).is_some()).collect();
    assert!(matched.is_empty(), "{matched:?}");
}

/// A step does not stand for a command that closes, reverts or writes documents, for Paste, for
/// Undo or Redo, or for an add-on's command: such a step is left out, and nothing is closed on
/// playback.
#[test]
fn steps_do_not_stand_for_file_commands_paste_undo_or_add_ons() {
    let labels = ["Close", "Close All", "Revert", "Export for Screens", "Paste", "Undo", "Redo", "Close Path", "Save a Timestamped Copy"];
    let matched: Vec<&str> = labels.into_iter().filter(|label| files::step_command(label).is_some()).collect();
    assert!(matched.is_empty(), "{matched:?}");
    let mut app = app_with_sets();
    let r = app.run(LOAD_SET, json!({ "data": aia_text("Closes", &[("A", &[("Close", true), ("Group", true)])]) })).unwrap();
    assert_eq!(r["leftOut"], 1);
    assert_eq!(app.ui.action_sets[3].actions[0].steps, [("object.group".to_string(), json!({}))]);
}

/// A set loaded from another app's file saves to VectorCraft's own file with its excluded steps,
/// and loads back the same.
#[test]
fn another_apps_set_saves_and_loads_back_with_its_excluded_steps() {
    let mut app = app_with_sets();
    let text = aia_text("Tidy", &[("A", &[("Group", true), ("Ungroup", false)])]);
    app.run(LOAD_SET, json!({ "data": text })).unwrap();
    let saved = files::write_set(&app.ui.action_sets[3]);
    let r = app.run(LOAD_SET, json!({ "data": saved })).unwrap();
    assert_eq!(r, json!({"set": 4, "name": "Tidy 2", "actions": 1}));
    assert_eq!(app.ui.action_sets[4].actions[0].excluded_steps, [1].into());
}

/// Save Actions… does not write a set that Load Actions… would refuse: more than 1,000 actions.
#[test]
fn save_actions_does_not_write_a_set_load_actions_refuses() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/none.vcactions");
    app.ui.action_sets[2].actions = vec![Action::new("A", vec![]); 1001];
    let r = app.run(SAVE_SET, json!({"set": 2, "path": "/sets/big.vcactions"}));
    assert_eq!(r.unwrap_err(), "the action set file holds more than 1,000 actions");
    assert!(files.borrow().is_empty(), "nothing written");
}

/// `ui.actionSet.save` with the set id the panel menu passes saves that set after another set
/// took its index and the current set changed, as when its file dialog answers later; an id
/// that no set has saves nothing, and `set` is not read.
#[test]
fn save_actions_saves_the_set_it_was_chosen_for() {
    let mut app = app_with_sets();
    let files = with_files(&mut app, "/sets/none.vcactions");
    let id = app.ui.action_sets[2].id;
    app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
    app.ui.action_sets.push(ActionSet::new("Third", vec![]));
    app.ui.action_set = Some(2);
    app.run(SAVE_SET, json!({"id": id, "path": "/sets/x.vcactions"})).unwrap();
    let v: Value = serde_json::from_slice(&files.borrow()["/sets/x.vcactions"]).unwrap();
    assert_eq!(v["name"], json!("Other"), "the set chosen, not the current one");
    assert_eq!(app.run(SAVE_SET, json!({"id": 0, "set": 0, "path": "/sets/y.vcactions"})).unwrap_err(), "no such action set");
}

/// A saved `automaticAlignTo` that is not true or false loads as false, and the rest of the
/// preferences still load.
#[test]
fn loaded_preferences_read_a_damaged_automatic_align_to_as_off() {
    let app = app_with_sets();
    let mut saved = serde_json::to_value(&app.ui).unwrap();
    saved["action_sets"][0]["actions"][0]["automaticAlignTo"] = json!(true);
    saved["action_sets"][0]["actions"][1]["automaticAlignTo"] = Value::Null;
    saved["action_sets"][0]["actions"][2]["automaticAlignTo"] = json!("yes");
    let ui = serde_json::from_value::<crate::UiState>(saved).unwrap();
    let on = ui.action_sets[0].actions.iter().map(|a| a.automatic_align_to).collect::<Vec<_>>();
    assert_eq!(on[..3], [true, false, false]);
    assert_eq!(ui.action_sets.len(), app.ui.action_sets.len());
}

/// An action set with a View menu step saves as version 6 and loads back with the step; one whose
/// View menu step has params that are not an object is not an action set file. A View menu step
/// in a recorded batch counts too, since playback runs it as its own step.
#[test]
fn a_saved_view_step_survives_as_version_6() {
    let (app, _) = app_with_a_view_step();
    let saved = files::write_set(&app.ui.action_sets[0]);
    let file: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(file["version"], json!(6), "older builds must refuse a View menu step");
    let loaded = files::read_set(&saved, "Test").unwrap();
    assert_eq!(loaded.actions[0].steps, app.ui.action_sets[0].actions[0].steps);
    for params in [json!(5), json!([1]), json!(null)] {
        let step = json!({"command": "view.zoomIn", "params": params});
        for step in [step.clone(), json!({"command": "command.batch", "params": {"commands": [step]}})] {
            let bad = json!({"format": "vcactions", "version": 6, "name": "Bad", "actions": [{"name": "A", "steps": [step]}]});
            assert_eq!(files::read_set(&bad.to_string(), "Test").unwrap_err(), "not an action set file", "{step}");
        }
    }
    let nested = json!({"commands": [{"command": "view.outline", "params": {}}]});
    let set = ActionSet::new("Nested", vec![Action::new("A", vec![("command.batch".into(), nested)])]);
    assert_eq!(serde_json::from_str::<Value>(&files::write_set(&set)).unwrap()["version"], json!(6), "a step in a recorded batch counts");
}

#[test]
fn a_saved_select_path_step_survives_and_a_damaged_one_is_refused() {
    let (mut app, _, _) = app_with_two_rectangles();
    app.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    let saved = files::write_set(&app.ui.action_sets[0]);
    assert_eq!(serde_json::from_str::<Value>(&saved).unwrap()["version"], json!(5), "older builds must refuse a Select Path step");
    let loaded = files::read_set(&saved, "Test").unwrap();
    assert_eq!(loaded.actions[0].steps, app.ui.action_sets[0].actions[0].steps);
    let step = app.ui.action_sets[0].actions[0].steps[1].clone();
    let nested = json!({"commands": [{"command": SELECT_PATH_STEP, "params": step.1}]});
    let set = ActionSet::new("Nested", vec![Action::new("A", vec![("command.batch".into(), nested)])]);
    assert_eq!(serde_json::from_str::<Value>(&files::write_set(&set)).unwrap()["version"], json!(5), "a step in a recorded batch counts");
    let mut both = app.ui.action_sets[0].clone();
    both.actions[0].steps.push((SELECT_OBJECT_STEP.into(), json!({"note": "a", "caseSensitive": false, "wholeWord": false})));
    assert_eq!(serde_json::from_str::<Value>(&files::write_set(&both)).unwrap()["version"], json!(5), "with a Select Object step too");
    for params in [json!({"id": 3}), json!({"id": -1, "fingerprint": step.1["fingerprint"]}), json!({"id": 3, "fingerprint": "v1:xyz"}), json!(42)] {
        let bad = json!({"command": SELECT_PATH_STEP, "params": params});
        for s in [bad.clone(), json!({"command": "command.batch", "params": {"commands": [bad]}})] {
            let file = json!({"format": "vcactions", "version": 5, "name": "Bad", "actions": [{"name": "A", "steps": [s]}]});
            assert_eq!(files::read_set(&file.to_string(), "Test").unwrap_err(), "not an action set file", "{s}");
        }
    }
}

/// `files::step_bytes` is what a step adds to the file after another step, and
/// `files::FIRST_STEP_BYTES` less than it adds as an action's first step, with quotes,
/// backslashes, control characters, text outside ASCII, nested params, a stop that changes the
/// file's version and empty params. Excluding a step adds `files::EXCLUDED_BYTES`.
#[test]
fn step_bytes_count_what_a_step_adds_to_an_action_set_file() {
    let steps = [
        ("object.move", json!({"dx": 1, "dy": 0})),
        ("type.set", json!({"text": "say \"hi\" \\ back\tslash\nline \u{1} é 日本 🎨 </script>"})),
        ("command.batch", json!({"label": "B", "commands": [{"command": "object.move", "params": {"dx": 1, "n": {"a": [1, 2.5, {"b": null}]}}}]})),
        (STOP_STEP, json!({"message": "Paint it", "allowContinue": false})),
        ("object.group", json!({})),
        ("object.group", json!([])),
        ("object.group", Value::Null),
    ];
    for (command, params) in steps {
        for first in [true, false] {
            let before = if first { vec![] } else { vec![("object.group".to_string(), json!({}))] };
            let mut set = ActionSet::new("Set", vec![Action::new("A", before), Action::new("B", vec![("object.group".into(), json!({}))])]);
            let len = files::write_set(&set).len();
            set.actions[0].steps.push((command.to_string(), params.clone()));
            let added = files::write_set(&set).len() - len;
            let counted = files::step_bytes(command, &params);
            let first_bytes = if first { files::FIRST_STEP_BYTES } else { 0 };
            assert_eq!(counted + first_bytes, added, "{command} {params}, first: {first}");
        }
    }
    let mut set = ActionSet::new("Set", vec![Action::new("A", vec![("object.group".into(), json!({"n": "日本"})); 3])]);
    let len = files::write_set(&set).len();
    set.actions[0].excluded_steps = BTreeSet::from([0, 2]);
    assert_eq!(files::write_set(&set).len(), len + 2 * files::EXCLUDED_BYTES);
}

/// A step inserted from the menu saves and loads as the command and params it holds, with its
/// place and exclusion.
#[test]
fn an_inserted_menu_item_saves_and_loads() {
    let (mut app, _) = app_with_three_moves();
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 1, "command": "type.changeCase", "params": {"case": "title"}})).unwrap();
    app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([2]);
    let loaded = files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").unwrap();
    let action = &loaded.actions[0];
    assert_eq!(action.steps, app.ui.action_sets[0].actions[0].steps);
    assert_eq!(action.steps[2], ("type.changeCase".to_string(), json!({"case": "title"})));
    assert_eq!(action.excluded_steps, BTreeSet::from([2]));
}

#[test]
fn a_saved_stop_survives_and_older_files_still_load() {
    let (mut app, _) = app_with_a_stop(true);
    let saved = files::write_set(&app.ui.action_sets[0]);
    let file: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(file["version"], json!(3), "older builds must refuse a stop");
    let steps = file["actions"][0]["steps"].clone();
    assert_eq!(steps[1], json!({"command": "action.stop", "params": {"message": "Paint it", "allowContinue": true}}));
    let loaded = files::read_set(&saved, "Test").unwrap();
    assert_eq!(loaded.actions[0].steps, app.ui.action_sets[0].actions[0].steps);
    let kept: Action = serde_json::from_value(serde_json::to_value(&loaded.actions[0]).unwrap()).unwrap();
    assert_eq!(kept.steps[1].1, json!({"message": "Paint it", "allowContinue": true}), "the preferences keep it too");
    app.ui.action_sets[0].actions[0].steps.remove(1);
    let without = files::write_set(&app.ui.action_sets[0]);
    assert_eq!(serde_json::from_str::<Value>(&without).unwrap()["version"], json!(1), "a set with no stop stays version 1");
    for version in [1, 2] {
        let old =
            json!({"format": "vcactions", "version": version, "name": "Old", "actions": [{"name": "A", "steps": [{"command": "object.move"}]}]});
        assert_eq!(files::read_set(&old.to_string(), "Test").unwrap().actions[0].steps.len(), 1, "version {version}");
    }
    let later = json!({"format": "vcactions", "version": 8, "name": "New", "actions": []});
    assert_eq!(files::read_set(&later.to_string(), "Test").unwrap_err(), "the action set file needs a later version of VectorCraft");
    for params in [json!({"message": 5}), json!({"allowContinue": "yes"}), json!(42)] {
        let stop = json!({"command": "action.stop", "params": params});
        for step in [stop.clone(), json!({"command": "command.batch", "params": {"commands": [stop]}})] {
            let bad = json!({"format": "vcactions", "version": 3, "name": "Bad", "actions": [{"name": "A", "steps": [step]}]});
            assert_eq!(files::read_set(&bad.to_string(), "Test").unwrap_err(), "not an action set file", "{step}");
        }
    }
    let bare = json!({"format": "vcactions", "version": 3, "name": "Bare", "actions": [{"name": "A", "steps": [{"command": "action.stop"}]}]});
    assert!(files::read_set(&bare.to_string(), "Test").is_ok(), "a stop may leave its params out");
    let nested = json!({"commands": [{"command": "action.stop", "params": {"message": "Inner"}}]});
    let set = ActionSet::new("Nested", vec![Action::new("A", vec![("command.batch".into(), nested)])]);
    assert_eq!(serde_json::from_str::<Value>(&files::write_set(&set)).unwrap()["version"], json!(3), "a stop in a recorded batch counts");
}

/// A saved Select Object step loads back as it was, in a version 4 file that older builds refuse;
/// a file whose Select Object step has bad params is refused, also inside a recorded batch.
#[test]
fn a_saved_select_object_step_survives_and_bad_ones_are_refused() {
    let (mut app, _, _) = app_with_a_noted_rect();
    app.run(INSERT_SELECT_OBJECT, json!({"set": 0, "action": 0, "note": "Badge", "wholeWord": true})).unwrap();
    let saved = files::write_set(&app.ui.action_sets[0]);
    let file: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(file["version"], json!(4), "older builds must refuse a Select Object step");
    assert_eq!(
        file["actions"][0]["steps"][3],
        json!({"command": "select.byNote", "params": {"note": "Badge", "caseSensitive": false, "wholeWord": true}})
    );
    let loaded = files::read_set(&saved, "Test").unwrap();
    assert_eq!(loaded.actions[0].steps, app.ui.action_sets[0].actions[0].steps);
    app.ui.action_sets[0].actions[0].steps.push(("action.stop".into(), json!({"message": ""})));
    let file: Value = serde_json::from_str(&files::write_set(&app.ui.action_sets[0])).unwrap();
    assert_eq!(file["version"], json!(4), "with a stop too");
    for params in [json!({}), json!({"note": "  "}), json!({"note": 5}), json!({"note": "a", "caseSensitive": 1}), json!(null), json!(42)] {
        let step = json!({"command": "select.byNote", "params": params});
        for step in [step.clone(), json!({"command": "command.batch", "params": {"commands": [step]}})] {
            let bad = json!({"format": "vcactions", "version": 4, "name": "Bad", "actions": [{"name": "A", "steps": [step]}]});
            assert_eq!(files::read_set(&bad.to_string(), "Test").unwrap_err(), "not an action set file", "{step}");
        }
    }
    let nested = json!({"commands": [{"command": "select.byNote", "params": {"note": "Inner"}}]});
    let set = ActionSet::new("Nested", vec![Action::new("A", vec![("command.batch".into(), nested)])]);
    assert_eq!(serde_json::from_str::<Value>(&files::write_set(&set)).unwrap()["version"], json!(4), "a step in a recorded batch counts");
}
