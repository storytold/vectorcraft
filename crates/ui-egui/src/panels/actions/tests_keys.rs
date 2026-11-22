use super::tests_support::*;
use super::*;
use craft::automation::actions::{ActionColor, ActionList};
use craft::keymap::{CommandRef, Trigger};
use craft::ui::runtime::commands;

use crate::tests_synthetic::key_frame;

/// What egui reports for Ctrl, Command on macOS, as egui-winit sets it on this platform.
fn command_key() -> egui::Modifiers {
    if crate::keymap::platform().is_mac() {
        egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND
    } else {
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND
    }
}

/// A press of `key` with `modifiers` held; `repeat` for the presses a key held down repeats.
fn key_press(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat, modifiers }
}

/// A press and release of `key` with `modifiers` held.
fn tap(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::ModifiersChanged(modifiers),
        key_press(key, modifiers, false),
        egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers },
        egui::Event::ModifiersChanged(egui::Modifiers::NONE),
    ]
}

/// A context whose first whole app frame has run, so the board holds the app's keymap.
fn ready(app: &mut VectorcraftApp) -> egui::Context {
    let ctx = crate::testing::ui_context();
    key_frame(app, &ctx, vec![]);
    ctx
}

/// Gives action `action` of set `set` the function key `key`, as Action Options… does.
fn give_key(app: &mut VectorcraftApp, set: usize, action: usize, key: FunctionKey) {
    app.run(ACTION_OPTIONS, json!({"set": set, "action": action, "key": key.key, "cmd": key.cmd, "shift": key.shift})).unwrap();
}

/// Gives Duplicate the shortcut `shortcut` in the active shortcut set.
// PLAN: shortcuts-w7's `shortcuts.set` must take a chord that only an action's function key holds without `force` (the base's `shortcut_editor.rs:1597-1647` did): an action key is the action's own option, not a binding to steal.
fn set_duplicate(app: &mut VectorcraftApp, shortcut: &str) {
    app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": shortcut})).unwrap();
}

/// Action `action` of set `set` in `list`.
fn action_at(list: &ActionList, set: usize, action: usize) -> &Action {
    model::flat(list, set, action).and_then(|i| list.action(i)).expect("the action exists")
}

/// The function keys of the actions of set `set` in `list`, in the set's order.
fn keys_of(list: &ActionList, set: usize) -> Vec<Option<FunctionKey>> {
    let name = model::set_name(list, set).expect("the set exists");
    model::members(list, name).into_iter().filter_map(|i| list.action(i)).map(model::function_key).collect()
}

/// Whether Duplicate keeps `trigger` (`Shift+F14`, `F15 F15`) in the keymap the app's key presses
/// go through. The binding is read, not looked up: a lookup of a chord an action also holds answers
/// the action first.
// PLAN: `command_on` (lookup of a chord pressed once or twice) is replaced by this read: with the Actions play rows in the keymap, a single-press lookup answers the action, so it could no longer tell whether Duplicate kept its binding.
fn duplicate_on(app: &mut VectorcraftApp, trigger: &str) -> bool {
    let ctx = ready(app);
    let platform = crate::keymap::platform();
    let want = Trigger::parse(trigger).expect("a trigger").effective(platform);
    let keymap = commands::keymap(&ctx).expect("a frame ran");
    keymap.bindings(&CommandRef::new("edit.duplicate")).iter().any(|b| b.trigger.effective(platform) == want)
}

#[test]
fn shortcut_precedence_assignment_preserves_command_bindings() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    set_duplicate(&mut app, "Shift+F14");
    let shortcuts = |app: &VectorcraftApp| {
        let builtins = &crate::shortcut_editor::BUILTINS;
        (app.ui.shortcut_sets.active_name(builtins).to_string(), app.ui.shortcut_sets.active_overrides(builtins).clone())
    };
    let before = shortcuts(&app);
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 14, "shift": true})).unwrap();
    assert_eq!(shortcuts(&app), before);
    let ctx = ready(&mut app);
    let shift = egui::Modifiers::SHIFT;
    key_frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(shift), key_press(egui::Key::F14, shift, false)]);
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "Nudge");
    assert_eq!(shortcuts(&app), before);
    let undo = undo_steps(&app);
    key_frame(&mut app, &ctx, vec![key_press(egui::Key::F14, shift, true)]);
    assert_eq!(undo_steps(&app), undo, "a repeat plays nothing");
    app.run(NEW_ACTION, json!({"set": 2, "name": "Another", "key": 14, "shift": true})).unwrap();
    assert_eq!(shortcuts(&app), before);
}

#[test]
fn shortcut_precedence_duplicate_keys_play_the_first_action_once() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let key = FunctionKey { key: 15, cmd: false, shift: false };
    give_key(&mut app, 1, 0, key);
    give_key(&mut app, 1, 1, key);
    let before = undo_steps(&app);
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, tap(egui::Key::F15, egui::Modifiers::NONE));
    let history = &app.session.active().unwrap().history.undo;
    assert_eq!(history.len(), before + 1);
    assert_eq!(history.last().unwrap().label, "Nudge");
}

#[test]
fn shortcut_precedence_preferences_and_files_retain_action_keys() {
    let mut app = app_with_sets();
    let key = FunctionKey { key: 14, cmd: false, shift: true };
    give_key(&mut app, 1, 0, key);
    set_duplicate(&mut app, "Shift+F14");
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored = serde_json::from_value::<crate::UiState>(saved).unwrap().sanitized();
    assert_eq!(keys_of(&restored.actions, 1)[0], Some(key));
    let _files = with_files(&mut app, "/sets/Mine.vcactions");
    app.run(SAVE_SET, json!({"set": 1, "path": "/sets/Mine.vcactions"})).unwrap();
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    assert_eq!(keys_of(&app.ui.actions, set_names(&app).len() - 1)[0], Some(key));
}

#[test]
fn shortcut_precedence_applies_while_the_type_tool_edits() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 50, "text": "Hi"})).unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(app.session.tool_wants_text());
    give_key(&mut app, 1, 0, FunctionKey { key: 15, cmd: true, shift: false });
    set_duplicate(&mut app, "Cmd+F15");
    let before = undo_steps(&app);
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, tap(egui::Key::F15, command_key()));
    let history = &app.session.active().unwrap().history.undo;
    assert_eq!(history.len(), before + 1);
    assert_eq!(history.last().unwrap().label, "Nudge");
}

#[test]
fn shortcut_precedence_command_edits_retain_action_keys() {
    let mut app = app_with_sets();
    let key = FunctionKey { key: 15, cmd: false, shift: false };
    give_key(&mut app, 1, 0, key);
    set_duplicate(&mut app, "F15");
    assert_eq!(keys_of(&app.ui.actions, 1)[0], Some(key));
    // PLAN: the Keyboard Shortcuts dialog is `app.shortcuts_dialog` (kb.C27), not on the dialog host; its OK stores the active set as `shortcuts.set` does (shortcuts-w7 sheet, `panels/actions.rs:5526-5530`), so the dialog's edit runs as `shortcuts.set`.
    set_duplicate(&mut app, "Shift+F14");
    assert_eq!(keys_of(&app.ui.actions, 1)[0], Some(key));
}

/// Action function keys take priority over command shortcuts and keep both assignments.
#[test]
fn function_keys_play_their_action_and_preserve_other_assignments() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let keys = |app: &VectorcraftApp| keys_of(&app.ui.actions, 1);
    let f15 = FunctionKey { key: 15, cmd: false, shift: false };
    give_key(&mut app, 1, 0, f15);
    assert_eq!(keys(&app), [Some(f15), None]);
    give_key(&mut app, 1, 1, f15);
    app.run(NEW_ACTION, json!({"set": 2, "name": "New", "key": 15})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(keys(&app), [Some(f15), Some(f15)]);
    assert_eq!(keys_of(&app.ui.actions, 2)[0], Some(f15));
    let shift_f14 = FunctionKey { key: 14, cmd: false, shift: true };
    set_duplicate(&mut app, "Shift+F14");
    give_key(&mut app, 1, 0, shift_f14);
    assert!(duplicate_on(&mut app, "Shift+F14"), "the command keeps its shortcut");
    assert_eq!(keys(&app), [Some(shift_f14), Some(f15)]);
    // The keys play their actions.
    let second = model::flat(&app.ui.actions, 1, 1).unwrap();
    let rect = ActionStep::new("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
    app.ui.actions.replace_step(second, 0, rect).unwrap();
    let ctx = ready(&mut app);
    let last = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().map(|s| s.label.clone());
    let before = undo_steps(&app);
    key_frame(&mut app, &ctx, tap(egui::Key::F15, egui::Modifiers::NONE));
    assert_eq!((last(&app), undo_steps(&app)), (Some("Nudge Again".to_string()), before + 1), "F15 plays Nudge Again");
    let id = app.session.active().unwrap().selection.objects.clone();
    let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.bounds_of(&id, false).unwrap().x0;
    let before = x(&app);
    key_frame(&mut app, &ctx, tap(egui::Key::F14, egui::Modifiers::SHIFT));
    assert_eq!((last(&app), x(&app)), (Some("Nudge".to_string()), before + 1.0), "Shift+F14 plays Nudge");
    let before = undo_steps(&app);
    key_frame(&mut app, &ctx, tap(egui::Key::F14, egui::Modifiers::NONE));
    assert_eq!(undo_steps(&app), before, "F14 without Shift plays nothing");
    set_duplicate(&mut app, "F15");
    assert_eq!(keys(&app), [Some(shift_f14), Some(f15)]);
    // The Keyboard Shortcuts dialog's OK stores the active set as `shortcuts.set` does.
    set_duplicate(&mut app, "Shift+F14");
    assert_eq!(keys(&app), [Some(shift_f14), Some(f15)]);
}

/// Off macOS, an action's Cmd function key and a command's Ctrl shortcut share the chord.
#[test]
fn an_action_keeps_a_commands_ctrl_shortcut_for_a_cmd_function_key() {
    if crate::keymap::platform().is_mac() {
        return;
    }
    let mut app = app_with_sets();
    set_duplicate(&mut app, "Ctrl+F13");
    let cmd_f13 = FunctionKey { key: 13, cmd: true, shift: false };
    assert!(duplicate_on(&mut app, "Cmd+F13"), "Ctrl+F13 runs Duplicate");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 13, "cmd": true, "shift": false})).unwrap();
    assert!(duplicate_on(&mut app, "Cmd+F13"), "the command keeps its shortcut");
    assert_eq!(keys_of(&app.ui.actions, 1)[0], Some(cmd_f13));
}

/// An action's function key leaves a command's double-press shortcut assigned.
#[test]
fn a_double_press_shortcut_keeps_the_function_key() {
    let mut app = app_with_sets();
    set_duplicate(&mut app, "F15 F15");
    let f15 = FunctionKey { key: 15, cmd: false, shift: false };
    assert!(duplicate_on(&mut app, "F15 F15"), "F15 pressed twice runs Duplicate");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 15})).unwrap();
    assert!(duplicate_on(&mut app, "F15 F15"), "the command keeps its shortcut");
    assert_eq!(keys_of(&app.ui.actions, 1)[0], Some(f15));
}

/// The web build keeps an action's Ctrl or Cmd function key from the browser: the board names the
/// action's play command as the holder of the chord once the action has the key.
#[test]
fn an_action_key_runs_a_shortcut_for_the_web_build() {
    let mut app = app_with_sets();
    let chord = FunctionKey { key: 13, cmd: true, shift: false }.chord().unwrap();
    let ctx = ready(&mut app);
    assert_eq!(commands::holder(&ctx, &chord), None, "Ctrl+F13 runs nothing");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 13, "cmd": true})).unwrap();
    key_frame(&mut app, &ctx, vec![]);
    // PLAN: assumed menus-w5's Actions play row is `PLAY_ACTION` with params `{action: <name>}` (conflicts B6, shortcuts change 9).
    let holder = commands::holder(&ctx, &chord).map(|c| (c.id, c.params["action"].clone()));
    assert_eq!(holder, Some((PLAY_ACTION.to_string(), json!("Nudge"))), "Ctrl+F13 plays Nudge");
}

/// Every action key pressed in one frame plays its action, and a command shortcut pressed in
/// the same frame still runs.
#[test]
fn action_keys_and_a_command_shortcut_in_one_frame_all_run() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    give_key(&mut app, 1, 0, FunctionKey { key: 15, cmd: false, shift: false });
    give_key(&mut app, 1, 1, FunctionKey { key: 14, cmd: false, shift: false });
    set_duplicate(&mut app, "F13");
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.iter().map(|s| s.label.clone()).collect::<Vec<_>>();
    let before = undo(&app).len();
    let events = [egui::Key::F15, egui::Key::F14, egui::Key::F13].into_iter().flat_map(|key| tap(key, egui::Modifiers::NONE)).collect();
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, events);
    assert_eq!(undo(&app)[before..], ["Nudge", "Nudge Again", "Duplicate"], "both actions played, then Duplicate ran");
}

/// Function keys and colors are saved with the UI preferences and in action set files; older
/// preferences and files without them, or with values that are not one, load none.
#[test]
fn function_keys_and_colors_are_kept_with_the_preferences_and_in_set_files() {
    let mut app = app_with_sets();
    let k = FunctionKey { key: 13, cmd: true, shift: true };
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 13, "cmd": true, "shift": true, "color": "violet"})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    let first = &saved["action_sets"][1]["actions"][0];
    assert_eq!((first["key"].clone(), first["color"].clone()), (json!({"key": 13, "cmd": true, "shift": true}), json!("violet")));
    assert!(saved["action_sets"][1]["actions"][1].get("key").is_none(), "no key: nothing written");
    let restored = serde_json::from_value::<crate::state::UiState>(saved.clone()).unwrap();
    let a = action_at(&restored.actions, 1, 0);
    assert_eq!((model::function_key(a), a.color), (Some(k), ActionColor::Violet));
    let mut odd = saved.clone();
    odd["action_sets"][1]["actions"][0]["key"] = json!("F13");
    odd["action_sets"][1]["actions"][0]["color"] = json!("pink");
    let restored = serde_json::from_value::<crate::state::UiState>(odd).unwrap();
    let a = action_at(&restored.actions, 1, 0);
    assert_eq!((a.name.as_str(), model::function_key(a), a.color), ("Nudge", None, ActionColor::None), "values that are not one load as none");
    let files = with_files(&mut app, "/sets/Mine.vcactions");
    app.run(SAVE_SET, json!({"set": 1, "path": "/sets/Mine.vcactions"})).unwrap();
    let v: Value = serde_json::from_slice(&files.borrow()["/sets/Mine.vcactions"]).unwrap();
    assert_eq!(
        (v["version"].clone(), v["actions"][0]["key"].clone(), v["actions"][0]["color"].clone()),
        (json!(1), first["key"].clone(), json!("violet"))
    );
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    let loaded = action_at(&app.ui.actions, 3, 0);
    assert_eq!((model::function_key(loaded), loaded.color), (Some(k), ActionColor::Violet));
    let names = set_names(&app);
    app.run(DELETE_SETS, json!({"names": [names[1], names[3]]})).unwrap();
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    assert_eq!(keys_of(&app.ui.actions, 2)[0], Some(k), "a free key loads with its action");
    let plain = json!({"format": "vcactions", "version": 1, "name": "P", "actions": [{"name": "A", "steps": []}]});
    app.run(LOAD_SET, json!({"data": plain.to_string()})).unwrap();
    let a = action_at(&app.ui.actions, 3, 0);
    assert_eq!((model::function_key(a), a.color), (None, ActionColor::None), "a file without them loads none");
    set_duplicate(&mut app, "Cmd+Shift+F13");
    let names = set_names(&app);
    app.run(DELETE_SETS, json!({"names": [names[2]]})).unwrap();
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    let a = action_at(&app.ui.actions, set_names(&app).len() - 1, 0);
    assert_eq!((model::function_key(a), a.color), (Some(k), ActionColor::Violet));
}

/// Saved function keys survive loading; reserved keys cannot play actions on this platform.
#[test]
fn loaded_preferences_keep_function_keys_and_skip_reserved_keys_at_playback() {
    let mut app = app_with_sets();
    set_duplicate(&mut app, "Shift+F12");
    let mut saved = serde_json::to_value(&app.ui).unwrap();
    let key = |k: u8, cmd: bool, shift: bool| json!({"key": k, "cmd": cmd, "shift": shift});
    for (set, action, k) in [
        (1, 0, key(13, false, false)),
        (1, 1, key(13, false, false)),
        (0, 0, key(1, false, false)),
        (0, 1, key(12, false, true)),
        (0, 2, key(4, true, false)),
    ] {
        saved["action_sets"][set]["actions"][action]["key"] = k;
    }
    let ui = serde_json::from_value::<crate::UiState>(saved).unwrap().sanitized();
    let set_keys = |set: usize| keys_of(&ui.actions, set);
    let f13 = FunctionKey { key: 13, cmd: false, shift: false };
    assert_eq!(set_keys(1), [Some(f13), Some(f13)]);
    let windows = crate::keymap::platform() == craft::keymap::Platform::Windows;
    let f1 = FunctionKey { key: 1, cmd: false, shift: false };
    let ctrl_f4 = FunctionKey { key: 4, cmd: true, shift: false };
    let shift_f12 = FunctionKey { key: 12, cmd: false, shift: true };
    assert_eq!(set_keys(0)[..3], [Some(f1), Some(shift_f12), Some(ctrl_f4)]);
    assert_eq!(super::keys::playable_chord(f1).is_some(), !windows);
    assert_eq!(super::keys::playable_chord(ctrl_f4).is_some(), !windows);
    let builtins = &crate::shortcut_editor::BUILTINS;
    let duplicate = ui.shortcut_sets.active_overrides(builtins).get("edit.duplicate");
    let want = Trigger::parse("Shift+F12").expect("a trigger");
    assert!(duplicate.is_some_and(|texts| texts.iter().any(|t| Trigger::parse(t).ok() == Some(want))), "the command keeps its shortcut");
}
