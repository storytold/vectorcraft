use std::collections::BTreeSet;

use craft::automation::actions::{Action, ActionList, ActionListError, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use serde_json::{Value, json};

use super::model::{self, recorded_steps_fit, recording_room, steps_that_fit};
use super::playback::close_stop_message;
use crate::VectorcraftApp;

/// The command that begins a plain recording ([`record`]).
pub const START_RECORDING: &str = "ui.action.record";

/// The command that ends a recording ([`stop`]).
pub const STOP_RECORDING: &str = "ui.action.stop";

/// `ui.action.record {set?, action?, after?}`: begins recording an automatically named Action N in
/// `set`, or in the current set (the first if none is current). With no sets, makes Set 1. Stop
/// adds the action after the set's existing actions. With `action`, it records into the action at
/// that index of set `set` instead: Stop puts the recorded steps after step `after` of that action,
/// else after its last step, and the steps after them keep whether they are excluded. The Begin
/// Recording button passes the selected action and step. `action` without `set`, `after` without
/// `action`, an index past the last set, action or step, an explicit invalid set, an existing
/// recording, or a set whose new action would pass a limit of Load Actions…
/// ([`model::edit_keeps_set_loadable`]: 1,000 actions, 8 MiB) is an error and leaves the panel
/// unchanged. While it records, a command whose step does not fit in the action
/// ([`recording_room`]) does not run. → {set, name}, and with `action` {set, action, name}
pub fn record(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    if p.get("action").is_some() {
        return record_into(app, p);
    }
    if p.get("after").is_some() {
        return Err("no such action".into());
    }
    let set = match p.get("set") {
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < app.ui.actions.sets().len()).ok_or("no such action set")?,
        _ => {
            if app.ui.actions.sets().is_empty() {
                app.ui.actions.new_set("Set 1").map_err(|e| e.to_string())?;
            }
            let current = app.ui.action_set.as_deref().and_then(|name| app.ui.actions.sets().iter().position(|s| *s == name));
            current.unwrap_or(0)
        }
    };
    let set_name = model::set_name(&app.ui.actions, set).ok_or("no such action set")?.to_string();
    let members = model::stored_actions(&app.ui.actions, &set_name);
    // Names are unique in the whole list, so the number skips names other sets hold too.
    let mut number = members.len() + 1;
    let name = loop {
        let name = format!("Action {number}");
        if app.ui.actions.actions().iter().all(|action| action.name != name) {
            break name;
        }
        number += 1;
    };
    // Stop Playing/Recording adds the action even with no steps.
    let mut edited = members.clone();
    edited.push(Action { set: Some(set_name.clone()), ..Action::new(name.clone(), vec![]) });
    model::edit_keeps_set_loadable(app, &set_name, &members, &edited)?;
    app.ui.action_set = Some(set_name);
    app.ui.actions_panel.selected_sets.clear();
    start_recording(app, set, name.clone(), None);
    Ok(json!({ "set": set, "name": name }))
}

/// [`record`] with `action`: records into the action at index `action` of set `set`, after step
/// `after` or after its last step. The set becomes current and the only selected set, and the
/// action stays selected. → {set, action, name}
fn record_into(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let (set, at) = index("set").zip(index("action")).ok_or("no such action")?;
    let flat = model::flat(&app.ui.actions, set, at).ok_or("no such action")?;
    let action = app.ui.actions.action(flat).ok_or("no such action")?;
    let slot = match p.get("after") {
        None => None,
        Some(_) => Some(index("after").filter(|&s| s < action.steps.len()).ok_or("no such action step")? + 1),
    };
    let (name, set_name) = (action.name.clone(), action.set.clone());
    app.ui.action_set = set_name;
    app.ui.actions_panel.selected_sets.clear();
    start_recording(app, set, name.clone(), Some(name.clone()));
    app.ui.recording_at = slot;
    if !matches!(app.ui.actions_panel.selected, Some(ActionSelection::Action(i) | ActionSelection::Step(i, _)) if i == flat) {
        app.ui.actions_panel.selected = Some(ActionSelection::Action(flat));
    }
    Ok(json!({ "set": set, "action": at, "name": name }))
}

/// `ui.action.stop {}`: ends the recording in progress and keeps its steps, in the action New
/// Action made for it or the action [`record`] named, at the place it named, else in a new action
/// after the last action of its set, as the bottom bar's
/// Stop Playing/Recording does. It also ends a playback at Step by Step or Pause For, or one that
/// waits at a stop, whose message box closes: the steps already played stay, and the rest do not
/// run. With nothing recording or playing it changes nothing. Recorded steps past the limits of an
/// action set file are left out ([`stop_recording`]). → {stopped}
pub fn stop(app: &mut VectorcraftApp, _: &Value) -> Result<Value, String> {
    let playing = app.ui.action_playing.take().is_some();
    close_stop_message(app);
    let recording = app.ui.recording.is_some();
    stop_recording(app);
    Ok(json!({ "stopped": recording || playing }))
}

/// The journal entry the recording in progress records from.
pub(super) fn recording_start(app: &VectorcraftApp) -> Option<usize> {
    let (_, _, start) = app.ui.recording.as_ref()?;
    // A canceled undo group numbers the steps down when it drops entries made before them.
    Some(app.session.journal.held().unwrap_or(*start))
}

/// Where Stop Playing/Recording puts the steps of the recording in progress: the name of the set,
/// and the list index of the action it records into ([`crate::UiState::recording_into`]), or None
/// for the new action Stop adds after the set's last action. None when there is no recording, no
/// set to put them in, or the action it records into is gone, so that Stop keeps none of them.
pub(super) fn recording_target(app: &VectorcraftApp) -> Option<(String, Option<usize>)> {
    let (set, ..) = app.ui.recording.as_ref()?;
    let list = &app.ui.actions;
    let Some(into) = app.ui.recording_into.as_deref() else {
        return Some((model::set_name(list, *set)?.to_string(), None));
    };
    let at = list.actions().iter().position(|a| a.name == into)?;
    Some((list.set_of(at)?.to_string(), Some(at)))
}
// PLAN: the sheet keeps `set_with_recorded` here; menus-w13's model.rs holds its one copy (`model::set_with_recorded`), which the limits read.

/// Before the engine command `id` runs with `params`: Err when the recording in progress would keep
/// it as a step that does not fit in its action ([`recording_room`]). A command is kept as a step
/// when it is journaled, a batch (`command.batch`, which journals itself) too, and Find Objects
/// when it selects what it finds. Undo and Redo are steps too, and are refused when they do not
/// fit.
pub(crate) fn before_recorded_command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<(), String> {
    let journaled = vectorcraft_engine::find_command(id).is_some_and(|c| c.journal)
        || id == "command.batch"
        || (id == "document.find" && params.get("select").and_then(Value::as_bool) == Some(true));
    if app.ui.recording.is_none() || !journaled || app.session.active().is_some_and(|d| d.interaction.is_some()) {
        return Ok(());
    }
    recording_room(app, id, params)
}

/// After an engine command has run with result `r`: when the steps it journaled, the values a
/// command added to its step as it ran or the commands a script ran (`script.run`), make the steps
/// no longer fit ([`after_journaled_edit`]), the command's edit and its result stay, the status bar
/// names the limit, no later recorded command runs, and Stop Playing/Recording leaves the steps
/// out ([`steps_that_fit`]) → the command's result.
pub(crate) fn after_recorded_command(app: &mut VectorcraftApp, r: Result<Value, String>) -> Result<Value, String> {
    if r.is_ok() {
        after_journaled_edit(app);
    }
    r
}

/// After an edit that journals without [`before_recorded_command`]'s check (a script run's
/// commands, a tool's or a panel's edit, a dialog's commit): when the journal changed since the
/// last check and the steps that the recording in progress keeps no longer fit in its action
/// ([`recorded_steps_fit`]), the edit and the journal stay, the status bar names the limit, no
/// later recorded command runs, and Stop Playing/Recording keeps the steps that fit
/// ([`steps_that_fit`]) → whether the status bar names a limit.
pub(crate) fn after_journaled_edit(app: &mut VectorcraftApp) -> bool {
    if app.ui.recording.is_none() {
        return false;
    }
    let journal = (app.session.journal.rewrites(), app.session.journal.len());
    if app.ui.recording_checked == Some(journal) {
        return false;
    }
    app.ui.recording_checked = Some(journal);
    match recorded_steps_fit(app) {
        Ok(()) => false,
        Err(e) => {
            app.status(e);
            true
        }
    }
}

/// What Stop Playing/Recording adds to a set ([`recorded_into`]).
pub(super) struct Recorded {
    /// The index in the set of the action that the recording records into; None for the new
    /// action that the recording adds.
    pub(super) into: Option<usize>,
    /// The steps recorded so far.
    pub(super) steps: Vec<(String, Value)>,
}

/// What Stop Playing/Recording adds to the set at index `set`; None when the recording in progress
/// adds nothing to it.
pub(super) fn recorded_into(app: &VectorcraftApp, set: usize) -> Option<Recorded> {
    let (recording_set, _, _) = app.ui.recording.as_ref()?;
    let list = &app.ui.actions;
    let into = match app.ui.recording_into.as_deref() {
        Some(into) => {
            let members = model::members(list, model::set_name(list, set)?);
            Some(members.iter().position(|&i| list.action(i).is_some_and(|a| a.name == into))?)
        }
        None if *recording_set == set => None,
        None => return None,
    };
    let steps = app.session.journal_for_action(recording_start(app)?);
    Some(Recorded { into, steps })
}

/// Starts recording into the set at index `set`: Stop Playing/Recording adds an action named
/// `name` with the steps, or, with `into`, gives them to the action of that name.
pub(super) fn start_recording(app: &mut VectorcraftApp, set: usize, name: String, into: Option<String>) {
    let start = app.session.journal.len();
    // Keep every step from here on until recording stops, however long it runs.
    app.session.journal.hold(Some(start));
    app.ui.recording = Some((set, name, start));
    app.ui.recording_into = into;
    app.ui.recording_at = None;
    app.ui.recording_base = None;
    app.ui.recording_tail = None;
    app.ui.recording_checked = Some((app.session.journal.rewrites(), start));
}

/// Where Stop Playing/Recording puts recorded steps in `action`: the index of the first one,
/// before its step at index `at`, or after its last step when `at` is None or past it; and the
/// exclusions the action keeps, all but those past its last step, which saved preferences can hold.
fn recorded_place(action: &Action, at: Option<usize>) -> (usize, BTreeSet<usize>) {
    let len = action.steps.len();
    (at.map_or(len, |at| at.min(len)), action.disabled.iter().copied().filter(|&i| i < len).collect())
}

/// Puts the recorded `steps` into `action` at [`recorded_place`], included in playback. The steps
/// after them keep whether they are excluded, as [`ActionList::insert_step`] keeps them.
pub(super) fn put_recorded(action: &mut Action, steps: Vec<ActionStep>, at: Option<usize>) {
    let (slot, kept) = recorded_place(action, at);
    let added = steps.len();
    action.disabled = kept;
    action.steps.splice(slot..slot, steps);
    action.remap_steps(|i| Some(if i >= slot { i + added } else { i }));
}

/// [`put_recorded`] on the action at list index `flat` of `list` → the index of the first step
/// put in.
fn put_recorded_in(list: &mut ActionList, flat: usize, steps: Vec<ActionStep>, at: Option<usize>) -> Result<usize, ActionListError> {
    let action = list.action(flat).ok_or(ActionListError::NoSuchAction(flat))?;
    let (slot, kept) = recorded_place(action, at);
    // The list edits exclusions only within the steps, so a stale one goes by setting them all again.
    if kept != action.disabled {
        list.set_all(flat, true)?;
        for step in kept {
            list.set_step(flat, step, false)?;
        }
    }
    // Each step goes in before the ones put in after it, so they keep their recorded order.
    for step in steps.into_iter().rev() {
        list.insert_step(flat, slot, step)?;
    }
    Ok(slot)
}

/// Stop Playing/Recording: ends the recording and keeps its steps (see [`start_recording`]). An
/// action New Action made that was deleted while it recorded keeps nothing. Steps past the limits
/// of an action set file ([`steps_that_fit`]) are left out, and the status bar names the limit.
pub(super) fn stop_recording(app: &mut VectorcraftApp) {
    let Some(start) = recording_start(app) else { return };
    let mut steps = app.session.journal_for_action(start);
    let (keep, limit) = steps_that_fit(app, &steps);
    steps.truncate(keep);
    if let Some(limit) = limit {
        app.status(limit);
    }
    let Some((set, name, _)) = app.ui.recording.take() else { return };
    app.ui.recording_base = None;
    app.ui.recording_tail = None;
    app.ui.recording_checked = None;
    app.session.journal.hold(None);
    let at = app.ui.recording_at.take();
    let steps = model::steps_from(steps);
    let kept = match app.ui.recording_into.take() {
        Some(into) => {
            let Some(flat) = app.ui.actions.actions().iter().position(|a| a.name == into) else { return };
            let added = steps.len();
            put_recorded_in(&mut app.ui.actions, flat, steps, at).map(|from| follow_recorded(app, flat, from, added))
        }
        None => {
            let Some(set_name) = model::set_name(&app.ui.actions, set).map(str::to_string) else { return };
            // Another action may have taken the name while the recording ran.
            let name = app.ui.actions.unique_name(&name);
            app.ui.actions.push(Action { set: Some(set_name), ..Action::new(name, steps) }).map(drop)
        }
    };
    if let Err(e) = kept {
        app.status(e.to_string());
    }
}

/// After Stop Playing/Recording put `added` steps into the action at list index `flat` before its
/// step at index `from`: the selected step and a Record Stop, Insert Menu Item or Set Selection
/// dialog open on a later step of the action follow their step down.
fn follow_recorded(app: &mut VectorcraftApp, flat: usize, from: usize, added: usize) {
    if added == 0 {
        return;
    }
    if let Some(ActionSelection::Step(action, step)) = &mut app.ui.actions_panel.selected
        && *action == flat
        && *step >= from
    {
        *step += added;
    }
    if let Some((set, at)) = model::place(&app.ui.actions, flat) {
        super::edits::follow_step_dialogs(app, set, at, from, isize::try_from(added).unwrap_or(isize::MAX));
    }
}
