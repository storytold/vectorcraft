use super::tests_support::*;
use super::*;

/// The command that `chord` runs under `km`, pressed once or pressed twice.
#[cfg(test)]
fn command_on(km: &craft_keymap::Keymap, chord: &craft_keymap::Chord) -> Option<craft_keymap::CommandRef> {
    crate::keymap::lookup(km, chord).or_else(|| match km.lookup(chord, true, &craft_keymap::Active::NONE) {
        craft_keymap::Lookup::Run(c) => Some(c),
        craft_keymap::Lookup::Cycle(v) => v.into_iter().next(),
        _ => None,
    })
}

#[test]
fn shortcut_precedence_assignment_preserves_command_bindings() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Shift+F14");
    let before = app.ui.shortcut_overrides.clone();
    let set = app.ui.shortcut_set.clone();
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 14, "shift": true})).unwrap();
    assert_eq!(app.ui.shortcut_overrides, before);
    assert_eq!(app.ui.shortcut_set, set);
    let ctx = egui::Context::default();
    let event = egui::Event::Key { key: egui::Key::F14, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT };
    let mut output = ctx.run_ui(egui::RawInput { events: vec![event], ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "Nudge");
    assert_eq!(app.ui.shortcut_overrides, before);
    let undo = app.session.active().unwrap().history.undo.len();
    let repeated = egui::Event::Key { key: egui::Key::F14, physical_key: None, pressed: true, repeat: true, modifiers: egui::Modifiers::SHIFT };
    let mut output = ctx.run_ui(egui::RawInput { events: vec![repeated], ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    app.run(NEW_ACTION, json!({"set": 2, "name": "Another", "key": 14, "shift": true})).unwrap();
    assert_eq!(app.ui.shortcut_overrides, before);
    assert_eq!(app.ui.shortcut_set, set);
}

#[test]
fn shortcut_precedence_action_options_does_not_offer_replacement() {
    let mut app = app_with_sets();
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Shift+F14");
    crate::dialogs::action_set_options::open_action(&mut app, 1, 0);
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("key".into(), json!(14));
    d.fields.insert("shift".into(), json!(true));
    let texts = dialog_texts(&mut app);
    assert!(!texts.iter().any(|s| s == "Replace" || s.contains("already used")), "{texts:?}");
}

#[test]
fn shortcut_precedence_duplicate_keys_play_the_first_action_once() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let key = Some(FunctionKey { key: 15, cmd: false, shift: false });
    app.ui.action_sets[1].actions[0].key = key;
    app.ui.action_sets[1].actions[1].key = key;
    let before = app.session.active().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    let event = egui::Event::Key { key: egui::Key::F15, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let mut output = ctx.run_ui(egui::RawInput { events: vec![event], ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    output.textures_delta.clear();
    let history = &app.session.active().unwrap().history.undo;
    assert_eq!(history.len(), before + 1);
    assert_eq!(history.last().unwrap().label, "Nudge");
}

#[test]
fn shortcut_precedence_preferences_and_files_retain_action_keys() {
    let mut app = app_with_sets();
    let key = FunctionKey { key: 14, cmd: false, shift: true };
    app.ui.action_sets[1].actions[0].key = Some(key);
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Shift+F14");
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored = serde_json::from_value::<crate::UiState>(saved).unwrap().sanitized();
    assert_eq!(restored.action_sets[1].actions[0].key, Some(key));
    let _files = with_files(&mut app, "/sets/Mine.vcactions");
    app.run(SAVE_SET, json!({"set": 1, "path": "/sets/Mine.vcactions"})).unwrap();
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    assert_eq!(app.ui.action_sets.last().unwrap().actions[0].key, Some(key));
}

#[test]
fn shortcut_precedence_applies_while_the_type_tool_edits() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 50, "text": "Hi"})).unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(app.session.tool_wants_text());
    let key = FunctionKey { key: 15, cmd: true, shift: false };
    app.ui.action_sets[1].actions[0].key = Some(key);
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Cmd+F15");
    let before = app.session.active().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    let event = egui::Event::Key { key: egui::Key::F15, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    let mut output = ctx.run_ui(egui::RawInput { events: vec![event], ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    output.textures_delta.clear();
    let history = &app.session.active().unwrap().history.undo;
    assert_eq!(history.len(), before + 1);
    assert_eq!(history.last().unwrap().label, "Nudge");
}

#[test]
fn shortcut_precedence_command_edits_retain_action_keys() {
    let mut app = app_with_sets();
    let key = FunctionKey { key: 15, cmd: false, shift: false };
    app.ui.action_sets[1].actions[0].key = Some(key);
    app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": "F15"})).unwrap();
    assert_eq!(app.ui.action_sets[1].actions[0].key, Some(key));
    crate::shortcut_editor::open(&mut app);
    let mut overrides = app.ui.shortcut_overrides.clone();
    crate::keymap::set_override(&mut overrides, "edit.duplicate", "Shift+F14");
    app.ui.dialog.as_mut().unwrap().fields.insert("overrides".into(), crate::keymap::stored::to_value(&overrides));
    crate::shortcut_editor::confirm(&mut app).unwrap();
    assert_eq!(app.ui.action_sets[1].actions[0].key, Some(key));
}

/// Action function keys take priority over command shortcuts and keep both assignments.
#[test]
fn function_keys_play_their_action_and_preserve_other_assignments() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let p = |set: usize, action: usize, k: FunctionKey| json!({"set": set, "action": action, "key": k.key, "cmd": k.cmd, "shift": k.shift});
    let keys = |app: &VectorcraftApp| app.ui.action_sets[1].actions.iter().map(|a| a.key).collect::<Vec<_>>();
    let f15 = FunctionKey { key: 15, cmd: false, shift: false };
    app.run(ACTION_OPTIONS, p(1, 0, f15)).unwrap();
    assert_eq!(keys(&app), [Some(f15), None]);
    app.run(ACTION_OPTIONS, p(1, 1, f15)).unwrap();
    app.run(NEW_ACTION, json!({"set": 2, "name": "New", "key": 15})).unwrap();
    stop_recording(&mut app);
    assert_eq!(keys(&app), [Some(f15), Some(f15)]);
    assert_eq!(app.ui.action_sets[2].actions[0].key, Some(f15));
    let shift_f14 = FunctionKey { key: 14, cmd: false, shift: true };
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Shift+F14");
    app.run(ACTION_OPTIONS, p(1, 0, shift_f14)).unwrap();
    let km = |app: &VectorcraftApp| crate::keymap::keymap_with(&app.ui.shortcut_overrides);
    assert!(crate::keymap::lookup(&km(&app), &shift_f14.chord().unwrap()).is_some(), "the command keeps its shortcut");
    assert_eq!(keys(&app), [Some(shift_f14), Some(f15)]);
    // The keys play their actions.
    app.ui.action_sets[1].actions[1].steps = vec![("shape.rectangle".into(), json!({"x": 10, "y": 10, "width": 20, "height": 20}))];
    let ctx = egui::Context::default();
    let press = |app: &mut VectorcraftApp, key, modifiers| {
        let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    let last = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().map(|s| s.label.clone());
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = undo(&app);
    press(&mut app, egui::Key::F15, egui::Modifiers::NONE);
    assert_eq!((last(&app), undo(&app)), (Some("Nudge Again".to_string()), before + 1), "F15 plays Nudge Again");
    let id = app.session.active().unwrap().selection.objects.clone();
    let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.bounds_of(&id, false).unwrap().x0;
    let before = x(&app);
    press(&mut app, egui::Key::F14, egui::Modifiers::SHIFT);
    assert_eq!((last(&app), x(&app)), (Some("Nudge".to_string()), before + 1.0), "Shift+F14 plays Nudge");
    let before = undo(&app);
    press(&mut app, egui::Key::F14, egui::Modifiers::NONE);
    assert_eq!(undo(&app), before, "F14 without Shift plays nothing");
    app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": "F15"})).unwrap();
    assert_eq!(keys(&app), [Some(shift_f14), Some(f15)]);
    crate::shortcut_editor::open(&mut app);
    let mut ov = app.ui.shortcut_overrides.clone();
    crate::keymap::set_override(&mut ov, "edit.duplicate", "Shift+F14");
    app.ui.dialog.as_mut().unwrap().fields.insert("overrides".into(), crate::keymap::stored::to_value(&ov));
    crate::shortcut_editor::confirm(&mut app).unwrap();
    assert_eq!(keys(&app), [Some(shift_f14), Some(f15)]);
}

/// Off macOS, an action's Cmd function key and a command's Ctrl shortcut share the chord.
#[test]
fn an_action_keeps_a_commands_ctrl_shortcut_for_a_cmd_function_key() {
    if crate::keymap::platform().is_mac() {
        return;
    }
    let mut app = app_with_sets();
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Ctrl+F13");
    let cmd_f13 = FunctionKey { key: 13, cmd: true, shift: false };
    let km = |app: &VectorcraftApp| crate::keymap::keymap_with(&app.ui.shortcut_overrides);
    assert!(crate::keymap::lookup(&km(&app), &cmd_f13.chord().unwrap()).is_some(), "Ctrl+F13 runs Duplicate");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 13, "cmd": true, "shift": false})).unwrap();
    assert!(crate::keymap::lookup(&km(&app), &cmd_f13.chord().unwrap()).is_some(), "the command keeps its shortcut");
    assert_eq!(app.ui.action_sets[1].actions[0].key, Some(cmd_f13));
}

/// An action's function key leaves a command's double-press shortcut assigned.
#[test]
fn a_double_press_shortcut_keeps_the_function_key() {
    let mut app = app_with_sets();
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "F15 F15");
    let f15 = FunctionKey { key: 15, cmd: false, shift: false };
    let km = |app: &VectorcraftApp| crate::keymap::keymap_with(&app.ui.shortcut_overrides);
    assert!(command_on(&km(&app), &f15.chord().unwrap()).is_some(), "F15 pressed twice runs Duplicate");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 15})).unwrap();
    assert!(command_on(&km(&app), &f15.chord().unwrap()).is_some(), "the command keeps its shortcut");
    assert_eq!(app.ui.action_sets[1].actions[0].key, Some(f15));
}

/// The web build keeps an action's Ctrl or Cmd function key from the browser.
#[test]
fn an_action_key_runs_a_shortcut_for_the_web_build() {
    let mut app = app_with_sets();
    let press = || crate::shortcuts::runs_a_shortcut("F13", "F13", egui::Modifiers::COMMAND);
    sync_function_keys(&app.ui);
    assert!(!press(), "Ctrl+F13 runs nothing");
    app.ui.action_sets[1].actions[0].key = Some(FunctionKey { key: 13, cmd: true, shift: false });
    sync_function_keys(&app.ui);
    assert!(press(), "Ctrl+F13 plays Nudge");
}

/// Every action key pressed in one frame plays its action, and a command shortcut pressed in
/// the same frame still runs.
#[test]
fn action_keys_and_a_command_shortcut_in_one_frame_all_run() {
    let mut app = app_with_sets();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.ui.action_sets[1].actions[0].key = Some(FunctionKey { key: 15, cmd: false, shift: false });
    app.ui.action_sets[1].actions[1].key = Some(FunctionKey { key: 14, cmd: false, shift: false });
    app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": "F13"})).unwrap();
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.iter().map(|s| s.label.clone()).collect::<Vec<_>>();
    let before = undo(&app).len();
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let events = vec![key(egui::Key::F15), key(egui::Key::F14), key(egui::Key::F13)];
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    out.textures_delta.clear();
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
    let a = &restored.action_sets[1].actions[0];
    assert_eq!((a.key, a.color.as_deref()), (Some(k), Some("violet")));
    let mut odd = saved.clone();
    odd["action_sets"][1]["actions"][0]["key"] = json!("F13");
    odd["action_sets"][1]["actions"][0]["color"] = json!("pink");
    let restored = serde_json::from_value::<crate::state::UiState>(odd).unwrap();
    let a = &restored.action_sets[1].actions[0];
    assert_eq!((a.name.as_str(), a.key, a.color.clone()), ("Nudge", None, None), "values that are not one load as none");
    let files = with_files(&mut app, "/sets/Mine.vcactions");
    app.run(SAVE_SET, json!({"set": 1, "path": "/sets/Mine.vcactions"})).unwrap();
    let v: Value = serde_json::from_slice(&files.borrow()["/sets/Mine.vcactions"]).unwrap();
    assert_eq!(
        (v["version"].clone(), v["actions"][0]["key"].clone(), v["actions"][0]["color"].clone()),
        (json!(1), first["key"].clone(), json!("violet"))
    );
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    let loaded = &app.ui.action_sets[3].actions[0];
    assert_eq!((loaded.key, loaded.color.as_deref()), (Some(k), Some("violet")));
    app.run(DELETE_SETS, json!({"sets": [1, 3]})).unwrap();
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    assert_eq!(app.ui.action_sets[2].actions[0].key, Some(k), "a free key loads with its action");
    let plain = json!({"format": "vcactions", "version": 1, "name": "P", "actions": [{"name": "A", "steps": []}]});
    app.run(LOAD_SET, json!({"data": plain.to_string()})).unwrap();
    let a = &app.ui.action_sets[3].actions[0];
    assert_eq!((a.key, a.color.clone()), (None, None), "a file without them loads none");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Cmd+Shift+F13");
    app.run(DELETE_SETS, json!({"sets": [2]})).unwrap();
    app.run(LOAD_SET, json!({"path": "/sets/Mine.vcactions"})).unwrap();
    let a = &app.ui.action_sets.last().unwrap().actions[0];
    assert_eq!((a.key, a.color.as_deref()), (Some(k), Some("violet")));
}

/// Saved function keys survive loading; reserved keys cannot play actions on this platform.
#[test]
fn loaded_preferences_keep_function_keys_and_skip_reserved_keys_at_playback() {
    let app = app_with_sets();
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
    let mut overrides = app.ui.shortcut_overrides.clone();
    crate::keymap::set_override(&mut overrides, "edit.duplicate", "Shift+F12");
    saved["shortcut_overrides"] = crate::keymap::stored::to_value(&overrides);
    let ui = serde_json::from_value::<crate::UiState>(saved).unwrap().sanitized();
    let keys = |set: usize| ui.action_sets[set].actions.iter().map(|a| a.key).collect::<Vec<_>>();
    let f13 = FunctionKey { key: 13, cmd: false, shift: false };
    assert_eq!(keys(1), [Some(f13), Some(f13)]);
    let windows = crate::keymap::platform() == craft_keymap::Platform::Windows;
    let f1 = FunctionKey { key: 1, cmd: false, shift: false };
    let ctrl_f4 = FunctionKey { key: 4, cmd: true, shift: false };
    let shift_f12 = FunctionKey { key: 12, cmd: false, shift: true };
    assert_eq!(keys(0)[..3], [Some(f1), Some(shift_f12), Some(ctrl_f4)]);
    assert_eq!(playable_chord(f1).is_some(), !windows);
    assert_eq!(playable_chord(ctrl_f4).is_some(), !windows);
    assert!(
        crate::keymap::lookup(&crate::keymap::keymap_with(&ui.shortcut_overrides), &shift_f12.chord().unwrap()).is_some(),
        "the command keeps its shortcut"
    );
}
