#![allow(clippy::unwrap_used, clippy::expect_used)]

use craft::automation::actions::Action;
use craft::automation::ui::actions::{ActionSelection, ActionsPanelCommands as C};
use craft_keymap::CommandRef;
use craft_ui::runtime::commands::CommandStates;
use serde_json::json;
use vectorcraft_engine::Session;

use super::edits::DELETE_SETS;
use super::menu::{menu_states, run};
use crate::VectorcraftApp;
use crate::dialogs::action_insert_menu_item::InsertMenuItemDialog;
use crate::dialogs::action_select_object::SelectObjectDialog;
use crate::dialogs::action_stop::RecordStopDialog;

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Default::default())
}

/// An app with sets "S1" and "S2" holding one action each, the second action current and selected.
fn with_sets() -> VectorcraftApp {
    let mut app = app();
    for set in ["S1", "S2"] {
        app.ui.actions.new_action(&format!("A in {set}"), Some(set.to_string())).unwrap();
    }
    app.ui.action_set = Some(1);
    app.ui.actions_panel.selected = Some(ActionSelection::Action(1));
    app
}

fn enabled(app: &VectorcraftApp, command: C) -> bool {
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    states.get(&CommandRef::from(command)).is_some_and(|s| s.is_enabled())
}

#[test]
fn the_panel_menu_states_follow_selection_recording_and_playback() {
    let mut empty = app();
    for (command, want) in [
        (C::SetOptions, false),
        (C::Batch, false),
        (C::Play, false),
        (C::Duplicate, false),
        (C::ClearAll, false),
        (C::InsertStop, false),
        (C::InsertPath, false),
        (C::Reset, true),
        (C::NewSet, true),
    ] {
        assert_eq!(enabled(&empty, command), want, "no sets: {command:?}");
    }

    let mut app = with_sets();
    for (command, want) in [(C::Batch, true), (C::Play, true), (C::InsertStop, true), (C::Duplicate, true), (C::Replace, true), (C::ClearAll, true)] {
        assert_eq!(enabled(&app, command), want, "an action selected: {command:?}");
    }
    app.ui.recording = Some((1, "New".into(), 0));
    for command in [C::Replace, C::ClearAll, C::Reset] {
        assert!(!enabled(&app, command), "recording: {command:?}");
    }
    app.ui.actions_panel.selected = None;
    assert!(enabled(&app, C::InsertStop), "a recording takes a stop with nothing selected");
    app.ui.recording = None;
    assert!(!enabled(&app, C::InsertStop), "nothing to put a stop into");

    // Deleting the selected action's set leaves nothing for the action rows to act on.
    let mut app = with_sets();
    app.run(DELETE_SETS, json!({ "names": ["S2"] })).unwrap();
    app.ui.action_set = None;
    for command in [C::Batch, C::Duplicate, C::Play] {
        assert!(!enabled(&app, command), "set deleted: {command:?}");
    }
    empty.ui.actions.push(Action::new("Loose", vec![])).unwrap();
}

#[test]
fn menu_insert_rows_open_their_dialog_on_the_selected_step_the_action_end_or_the_recording() {
    // PLAN: assumed the app has a dialog host over `ui_ctx` once a context is set.
    let place = |app: &VectorcraftApp| {
        app.dialogs().unwrap().with(|host| {
            let stop = host.get::<RecordStopDialog>().map(|d| (d.set, d.action, Some(d.step)));
            let item = host.get::<InsertMenuItemDialog>().map(|d| (d.set, d.action, Some(d.step)));
            let object = host.get::<SelectObjectDialog>().map(|d| (d.set, d.action, Some(d.step)));
            (stop, item, object)
        })
    };
    for id in ["actionsPanel.insertStop", "actionsPanel.insertMenuItem", "ui.action.selectObjectDialog"] {
        let mut app = with_sets();
        app.ui_ctx = Some(egui::Context::default());
        app.ui.actions.insert_step(1, 0, craft::automation::actions::ActionStep::new("a.b", json!({}))).unwrap();
        app.ui.actions_panel.expanded.insert("A in S2".into());

        app.ui.actions_panel.selected = Some(ActionSelection::Action(1));
        run(&mut app, id, &json!({})).unwrap().unwrap();
        let (stop, item, object) = place(&app);
        let opened = stop.or(item).or(object).unwrap_or_else(|| panic!("{id} opens a dialog"));
        assert_eq!((opened.0, opened.1), (1, 0), "{id}: the end of the selected action");
        assert_eq!(opened.2, None, "{id}: no step selected");

        app.ui.actions_panel.selected = Some(ActionSelection::Step(1, 0));
        run(&mut app, id, &json!({})).unwrap().unwrap();
        let (stop, item, object) = place(&app);
        let opened = stop.or(item).or(object).unwrap();
        assert_eq!(opened.2, Some(0), "{id}: after the selected step");

        app.ui.recording = Some((1, "New".into(), 0));
        run(&mut app, id, &json!({})).unwrap().unwrap();
        let (stop, item, object) = place(&app);
        let opened = stop.or(item).or(object).unwrap();
        assert_eq!(opened.2, None, "{id}: a recording takes the step at its end");
    }
}
