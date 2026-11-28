//! The Actions panel: record commands from the engine journal, play them back as one undo step.
//! Every recorded step is an ordinary command invocation, so actions are also plain JSON that
//! agents can author and replay (`command.batch`).
//!
//! The panel is the framework's [`ActionsPanel`]. It reports events; this module runs the app's
//! commands for them and keeps the current set in step with the selection.

use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::{
    ActionMove, ActionPlace, ActionSelection, ActionsEvent, ActionsHost, ActionsPanel, ActionsPanelCommands, ShortcutChoice, StepCells,
};
use craft::keymap::CommandRef;
use craft::ui::contract::SweepStep;
use serde_json::{Value, json};

use crate::{UiState, VectorcraftApp};

pub(crate) mod edits;
pub(crate) mod files;
mod keys;
pub(crate) mod menu;
pub(crate) mod model;
pub(crate) mod playback;
pub(crate) use model::prefs;
mod recording;
#[cfg(test)]
pub(crate) mod tests_support;

#[cfg(test)]
mod tests_edits;
#[cfg(test)]
mod tests_files;
#[cfg(test)]
mod tests_keys;
#[cfg(test)]
mod tests_limits;
#[cfg(test)]
mod tests_menu;
#[cfg(test)]
mod tests_playback;
#[cfg(test)]
mod tests_recording;
#[cfg(test)]
mod tests_stops;

pub(crate) use edits::select_object_params;
pub use edits::{
    ACTION_OPTIONS, BUTTON_MODE, CLEAR, DELETE_SETS, DELETE_STEP, DUPLICATE, INCLUDE_STEPS, INSERT_MENU_ITEM, INSERT_SELECT_OBJECT,
    INSERT_SELECT_PATH, INSERT_STOP, MOVE_STEP, NEW_ACTION, RENAME_ACTION, RESET, SELECT_OBJECT_STEP, SELECT_PATH_STEP, action_options, button_mode,
    clear, delete_sets, delete_step, duplicate, include_steps, insert_menu_item, insert_select_object, insert_select_path, insert_stop, move_step,
    new_action, rename_action, reset,
};
pub use files::{ACTIONS_EXT, LOAD_SET, REPLACE_SET, SAVE_SET, load_set, replace_set, save_set};
pub use keys::{FUNCTION_KEYS, FunctionKey, function_keys, key_error};
pub use model::{automatic_align_to, default_list, default_sets, deserialize_current_set, members, set_automatic_align_to};
pub use playback::{
    CONTINUE_PLAYBACK, PAUSE_SECONDS, PLAY_ACTION, PLAY_AT, PLAYBACK_OPTIONS, PlaybackOptions, PlaybackSpeed, Playing, STOP_STEP, continue_playback,
    deserialize_playback, play, play_action, play_at, playback_drawn, playback_held, playback_options, playback_shown, tick_playback,
    waiting_at_stop,
};
pub(crate) use playback::{KnownCommands, add_script_actions, has_stop, is_ui_step, shown_text, step_place, stop_params, unknown_commands};
pub use recording::{START_RECORDING, STOP_RECORDING, record, stop};
pub(crate) use recording::{after_journaled_edit, after_recorded_command, before_recorded_command};

/// The error a canceled file dialog reports; it is not worth a status line.
const CANCELLED: &str = "cancelled";

/// What the framework panel asks of the app while it draws. It holds copies, never `app.ui`, so the
/// panel can borrow the list and its state for the same pass.
struct ActionHost {
    /// The list index of the action being recorded into.
    recording: Option<usize>,
    /// The steps the journal holds since that recording started.
    pending: Vec<ActionStep>,
    playing: bool,
}

impl ActionHost {
    fn of(app: &VectorcraftApp) -> Self {
        let recording = app.ui.recording_into.as_deref().and_then(|name| app.ui.actions.actions().iter().position(|a| a.name == name));
        let pending = recording
            .and_then(|_| recording::recording_start(app))
            .map(|start| model::steps_from(app.session.journal_for_action(start)))
            .unwrap_or_default();
        Self { recording, pending, playing: app.ui.action_playing.is_some() }
    }
}

impl ActionsHost for ActionHost {
    fn step_label(&self, step: &ActionStep) -> String {
        let first_line = |key: &str| step.params.get(key).and_then(Value::as_str).and_then(|m| m.lines().next()).unwrap_or_default();
        match step.command.as_str() {
            STOP_STEP => crate::i18n::fmt(tl!("Stop: {message}"), &[("message", first_line("message"))]),
            SELECT_PATH_STEP => tl!("Select Path").to_string(),
            SELECT_OBJECT_STEP => crate::i18n::fmt(tl!("Select Object: {note}"), &[("note", first_line("note"))]),
            id => vectorcraft_engine::find_command(id).map_or_else(|| id.to_string(), |c| crate::i18n::t_id(id, c.label).to_string()),
        }
    }

    fn shortcut(&self, action: &Action) -> Option<ShortcutChoice> {
        model::function_key(action).map(|k| ShortcutChoice::new(k.key, k.shift, k.cmd))
    }

    fn recording(&self) -> Option<usize> {
        self.recording
    }

    fn pending_steps(&self, action: usize) -> &[ActionStep] {
        if self.recording == Some(action) { &self.pending } else { &[] }
    }

    fn playing(&self) -> bool {
        self.playing
    }
}

/// Draws the panel and runs what the user did in it.
pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let host = ActionHost::of(app);
    let enabled = !app.file_dialog_open();
    let UiState { actions_panel, actions, .. } = &mut app.ui;
    let output = ActionsPanel::new(actions_panel, actions).host(&host).step_cells(StepCells::Include).enabled(enabled).show(ui);
    // The current set is where a recording goes and what Set Options renames: the set of the
    // selected row, and unchanged when nothing is selected.
    let current = match &app.ui.actions_panel.selected {
        Some(ActionSelection::Set(name)) => Some(name.clone()),
        Some(ActionSelection::Action(i) | ActionSelection::Step(i, _)) => app.ui.actions.set_of(*i).map(str::to_string),
        _ => None,
    };
    if current.is_some() {
        app.ui.action_set = current;
    }
    for event in output.events {
        apply(app, event);
    }
}

/// Runs one event of the panel; a refusal shows on the status line.
fn apply(app: &mut VectorcraftApp, event: ActionsEvent) {
    let result = match event {
        ActionsEvent::Play { action, .. } => play_selected(app, action),
        ActionsEvent::BeginRecording { into, .. } => begin_recording(app, into),
        ActionsEvent::StopRecording => run(app, STOP_RECORDING, json!({})),
        ActionsEvent::PlaySet(_) => {
            app.status(tl!("Select an action to play"));
            Ok(())
        }
        ActionsEvent::NewAction => command(app, ActionsPanelCommands::NewAction),
        ActionsEvent::NewSet => command(app, ActionsPanelCommands::NewSet),
        ActionsEvent::SetOptions(set) => {
            app.ui.action_set = Some(set);
            command(app, ActionsPanelCommands::SetOptions)
        }
        ActionsEvent::InsertStop => command(app, ActionsPanelCommands::InsertStop),
        ActionsEvent::InsertMenuItem => command(app, ActionsPanelCommands::InsertMenuItem),
        ActionsEvent::InsertPath => command(app, ActionsPanelCommands::InsertPath),
        ActionsEvent::PlaybackOptions => command(app, ActionsPanelCommands::PlaybackOptions),
        ActionsEvent::Batch(action) => {
            app.ui.actions_panel.selected = Some(ActionSelection::Action(action));
            command(app, ActionsPanelCommands::Batch)
        }
        ActionsEvent::Load => command(app, ActionsPanelCommands::Load),
        ActionsEvent::ClearAll => command(app, ActionsPanelCommands::ClearAll),
        ActionsEvent::Reset => command(app, ActionsPanelCommands::Reset),
        ActionsEvent::Replace => command(app, ActionsPanelCommands::Replace),
        ActionsEvent::Save { set: None, .. } => command(app, ActionsPanelCommands::Save),
        ActionsEvent::Save { set: Some(name), .. } => run(app, SAVE_SET, json!({ "name": name })),
        ActionsEvent::Delete(selection) => delete(app, selection),
        ActionsEvent::DeleteSets(names) => {
            edits::ask_delete(app, names);
            Ok(())
        }
        ActionsEvent::Rename { action, text, .. } => action_command(app, RENAME_ACTION, action, json!({ "name": text })),
        ActionsEvent::SetColor { action, color, .. } => action_command(app, ACTION_OPTIONS, action, json!({ "color": color.name() })),
        ActionsEvent::AssignShortcut { action, choice, .. } => {
            let key = choice.map_or_else(|| json!({ "key": 0 }), |c| json!({ "key": c.key, "cmd": c.command, "shift": c.shift }));
            action_command(app, ACTION_OPTIONS, action, key)
        }
        ActionsEvent::Duplicate(selection) => duplicate_row(app, selection),
        ActionsEvent::Reorder(mv) => reorder(app, mv),
        ActionsEvent::Toggle(step) => toggle(app, step),
        // Record Again and Allow Tool Recording are not offered by VectorCraft: the panel emits them
        // only from rows its menu leaves out.
        _ => Ok(()),
    };
    if let Err(e) = result
        && e != CANCELLED
    {
        app.status(e);
    }
}

fn run(app: &mut VectorcraftApp, id: &str, params: Value) -> Result<(), String> {
    app.run(id, params).map(drop)
}

/// A panel-menu command, which acts on the panel's selection; the menu shows its own refusals.
fn command(app: &mut VectorcraftApp, command: ActionsPanelCommands) -> Result<(), String> {
    menu::run(app, &CommandRef::from(command).id, &Value::Null).unwrap_or(Ok(Value::Null)).map(drop)
}

/// The (set, action) indexes the commands take for the action at list index `action`.
fn located(list: &craft::automation::actions::ActionList, action: usize) -> Result<(usize, usize), String> {
    model::place(list, action).ok_or_else(|| "no such action".to_string())
}

/// Runs `id` for the action at list index `action` with `fields` added to its params.
fn action_command(app: &mut VectorcraftApp, id: &str, action: usize, fields: Value) -> Result<(), String> {
    let (set, at) = located(&app.ui.actions, action)?;
    let mut params = json!({ "set": set, "action": at });
    if let (Some(params), Value::Object(fields)) = (params.as_object_mut(), fields) {
        params.extend(fields);
    }
    run(app, id, params)
}

/// Play on from a stop the playback waits at, else play the action.
fn play_selected(app: &mut VectorcraftApp, action: usize) -> Result<(), String> {
    if waiting_at_stop(app) {
        return run(app, CONTINUE_PLAYBACK, json!({}));
    }
    let (set, at) = located(&app.ui.actions, action)?;
    run(app, PLAY_AT, json!({ "set": set, "action": at }))
}

/// Begin Recording records into the selected action, after its selected step; with no action it
/// records a new one.
fn begin_recording(app: &mut VectorcraftApp, into: Option<usize>) -> Result<(), String> {
    let Some(action) = into else { return run(app, START_RECORDING, json!({})) };
    let (set, at) = located(&app.ui.actions, action)?;
    let mut params = json!({ "set": set, "action": at });
    if let Some(ActionSelection::Step(selected, step)) = app.ui.actions_panel.selected
        && selected == action
    {
        params["after"] = json!(step);
    }
    run(app, START_RECORDING, params)
}

fn delete(app: &mut VectorcraftApp, selection: ActionSelection) -> Result<(), String> {
    match selection {
        ActionSelection::Set(name) => {
            edits::ask_delete(app, vec![name]);
            Ok(())
        }
        ActionSelection::Action(action) => {
            app.ui.actions.delete_action(action).map_err(|e| e.to_string())?;
            app.ui.actions_panel.selected = None;
            Ok(())
        }
        ActionSelection::Step(action, step) => {
            let (set, at) = located(&app.ui.actions, action)?;
            run(app, DELETE_STEP, json!({ "set": set, "action": at, "step": step }))
        }
        _ => Ok(()),
    }
}

/// Delete Selection, from the panel menu: the selected row, else the selected sets, else the
/// current set.
pub(crate) fn delete_selection(app: &mut VectorcraftApp) {
    let panel = &app.ui.actions_panel;
    let event = match (&panel.selected, panel.selected_sets.as_slice(), &app.ui.action_set) {
        (Some(selection), ..) => ActionsEvent::Delete(selection.clone()),
        (None, [], Some(current)) => ActionsEvent::Delete(ActionSelection::Set(current.clone())),
        (None, sets, _) if !sets.is_empty() => ActionsEvent::DeleteSets(sets.to_vec()),
        _ => return,
    };
    apply(app, event);
}

/// Duplicate, from the panel menu: the selected row, else the current set.
pub(crate) fn duplicate_selection(app: &mut VectorcraftApp) {
    let selection = app.ui.actions_panel.selected.clone().or_else(|| app.ui.action_set.clone().map(ActionSelection::Set));
    if let Some(selection) = selection {
        apply(app, ActionsEvent::Duplicate(selection));
    }
}

/// Copies the row and selects the copy.
fn duplicate_row(app: &mut VectorcraftApp, selection: ActionSelection) -> Result<(), String> {
    let params = match selection {
        ActionSelection::Action(action) => {
            let (set, at) = located(&app.ui.actions, action)?;
            json!({ "set": set, "action": at })
        }
        ActionSelection::Set(name) => {
            let set = app.ui.actions.sets().iter().position(|s| *s == name).ok_or("no such action set")?;
            json!({ "set": set })
        }
        ActionSelection::Step(action, step) => {
            return app.ui.actions.duplicate_step(action, step).map_err(|e| e.to_string());
        }
        _ => return Ok(()),
    };
    let copy = app.run(DUPLICATE, params)?;
    let index = |key: &str| copy[key].as_u64().and_then(|i| usize::try_from(i).ok());
    let Some(set) = index("set").and_then(|i| model::set_name(&app.ui.actions, i)).map(str::to_string) else { return Ok(()) };
    app.ui.actions_panel.selected = match index("action") {
        Some(action) => model::flat(&app.ui.actions, index("set").unwrap_or_default(), action).map(ActionSelection::Action),
        None => Some(ActionSelection::Set(set.clone())),
    };
    app.ui.action_set = Some(set);
    Ok(())
}

/// Applies a drag or a key move of a row.
fn reorder(app: &mut VectorcraftApp, mv: ActionMove) -> Result<(), String> {
    match (mv.row, mv.place) {
        (ActionSelection::Action(from), ActionPlace::Action { index, set: Some(set), .. }) => {
            let moved = app.ui.actions.action(from).cloned().ok_or("no such action")?;
            let before = model::stored_actions(&app.ui.actions, &set);
            let mut after = before.clone();
            if app.ui.actions.set_of(from) != Some(set.as_str()) {
                after.push(moved);
            }
            model::edit_keeps_set_loadable(app, &set, &before, &after)?;
            app.ui.actions.move_action(from, index).map_err(|e| e.to_string())?;
            app.ui.actions.assign_set(index, Some(set)).map_err(|e| e.to_string())
        }
        (ActionSelection::Step(action, step), ActionPlace::Step { action: into, index, .. }) => {
            let (set, at) = located(&app.ui.actions, action)?;
            let (to_set, to_action) = located(&app.ui.actions, into)?;
            // `index` counts the steps after the step left its place; `before` counts them before.
            let before = if action == into && index > step { index + 1 } else { index };
            run(app, MOVE_STEP, json!({ "set": set, "action": at, "step": step, "toSet": to_set, "toAction": to_action, "before": before }))
        }
        // Every VectorCraft action lives in a set (the saved shape has no loose actions).
        _ => Ok(()),
    }
}

/// Applies one step of an include-cell press or sweep.
fn toggle(app: &mut VectorcraftApp, step: SweepStep<ActionSelection>) -> Result<(), String> {
    // PLAN: an `UndoShowOnly` of a canceled Alt sweep cannot restore the other cells exactly: the
    // list keeps no snapshot of them, so it reapplies the cell's value to them.
    let (edit, others) = match step {
        SweepStep::Flip(edit) | SweepStep::Restore(edit) => (edit, false),
        SweepStep::ShowOnly(edit) | SweepStep::UndoShowOnly(edit) => (edit, true),
        _ => return Ok(()),
    };
    let list = &app.ui.actions;
    let params = match &edit.key {
        ActionSelection::Set(name) => {
            let set = list.sets().iter().position(|s| *s == name.as_str()).ok_or("no such action set")?;
            json!({ "set": set, "included": edit.on })
        }
        ActionSelection::Action(action) => {
            let (set, at) = located(list, *action)?;
            json!({ "set": set, "action": at, "included": edit.on })
        }
        ActionSelection::Step(action, step) => {
            let (set, at) = located(list, *action)?;
            json!({ "set": set, "action": at, "step": step, "included": edit.on, "others": others })
        }
        _ => return Ok(()),
    };
    run(app, INCLUDE_STEPS, params)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::{BUTTON_MODE, START_RECORDING, STOP_RECORDING};
    use crate::{UiState, VectorcraftApp};

    #[test]
    fn button_mode_switches_the_view_and_is_kept_with_the_preferences() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(!app.ui.actions_panel.settings.button_mode);
        assert_eq!(app.run(BUTTON_MODE, json!({})).unwrap(), json!({"enabled": true}));
        assert!(app.ui.actions_panel.settings.button_mode);
        // The saved key stays a bool, as earlier versions wrote it.
        let saved = serde_json::to_value(&app.ui).unwrap();
        assert_eq!(saved["action_button_mode"], json!(true));
        let read: UiState = serde_json::from_value(saved.clone()).unwrap();
        assert!(read.actions_panel.settings.button_mode);
        // Preferences saved before the key existed read as off.
        let mut older = saved;
        older.as_object_mut().unwrap().remove("action_button_mode");
        assert!(!serde_json::from_value::<UiState>(older).unwrap().actions_panel.settings.button_mode);
        // Turning it on is refused while an action records, and the view stays.
        app.run(BUTTON_MODE, json!({"enabled": false})).unwrap();
        app.run(START_RECORDING, json!({})).unwrap();
        assert!(app.run(BUTTON_MODE, json!({"enabled": true})).is_err());
        assert!(!app.ui.actions_panel.settings.button_mode);
        app.run(STOP_RECORDING, json!({})).unwrap();
    }
}
