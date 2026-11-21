use super::*;

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
/// ([`edit_keeps_set_loadable`]: 1,000 actions, 8 MiB) is an error and leaves the panel unchanged.
/// While it records, a command whose step does not fit in the action ([`recording_room`]) does not
/// run. → {set, name}, and with `action` {set, action, name}
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
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < app.ui.action_sets.len()).ok_or("no such action set")?,
        _ => {
            if app.ui.action_sets.is_empty() {
                app.ui.action_sets.push(ActionSet::new("Set 1", vec![]));
            }
            app.ui.action_set.filter(|&i| i < app.ui.action_sets.len()).unwrap_or(0)
        }
    };
    let mut number = app.ui.action_sets[set].actions.len() + 1;
    let name = loop {
        let name = format!("Action {number}");
        if app.ui.action_sets[set].actions.iter().all(|action| action.name != name) {
            break name;
        }
        number += 1;
    };
    // Stop Playing/Recording adds the action even with no steps.
    let mut edited = app.ui.action_sets[set].clone();
    edited.actions.push(Action::new(name.clone(), vec![]));
    edit_keeps_set_loadable(app, Some(set), &edited)?;
    app.ui.action_set = Some(set);
    app.ui.action_sets_selected = vec![set];
    start_recording(app, set, name.clone(), None);
    Ok(json!({ "set": set, "name": name }))
}

/// [`record`] with `action`: records into the action at index `action` of set `set`, after step
/// `after` or after its last step. The set becomes current and the only selected set, and the
/// action stays selected. → {set, action, name}
fn record_into(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let (set, at) =
        index("set").zip(index("action")).filter(|&(s, a)| app.ui.action_sets.get(s).is_some_and(|s| a < s.actions.len())).ok_or("no such action")?;
    let action = &app.ui.action_sets[set].actions[at];
    let slot = match p.get("after") {
        None => None,
        Some(_) => Some(index("after").filter(|&s| s < action.steps.len()).ok_or("no such action step")? + 1),
    };
    let (id, name) = (action.id, action.name.clone());
    app.ui.action_set = Some(set);
    app.ui.action_sets_selected = vec![set];
    start_recording(app, set, name.clone(), Some(id));
    app.ui.recording_at = slot;
    if let Some(ctx) = app.ui_ctx.clone() {
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((set, at)));
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

/// Where Stop Playing/Recording puts the steps of the recording in progress: the index of the set,
/// and of the action it records into ([`crate::UiState::recording_into`]), or None for the new
/// action Stop adds after the set's last action. None when there is no recording, no set to put
/// them in, or the action it records into was deleted, so that Stop keeps none of them.
pub(super) fn recording_target(app: &VectorcraftApp) -> Option<(usize, Option<usize>)> {
    let (set, ..) = app.ui.recording.as_ref()?;
    let Some(id) = app.ui.recording_into else {
        app.ui.action_sets.get(*set)?;
        return Some((*set, None));
    };
    app.ui.action_sets.iter().enumerate().find_map(|(holder, s)| Some((holder, Some(s.actions.iter().position(|a| a.id == id)?))))
}

/// The set that Stop Playing/Recording leaves when the recording in progress has recorded `steps`,
/// and the index of the action they go into ([`recording_target`]).
pub(super) fn set_with_recorded(app: &VectorcraftApp, steps: Vec<(String, Value)>) -> Option<(ActionSet, usize)> {
    let (holder, at) = recording_target(app)?;
    let mut s = app.ui.action_sets.get(holder)?.clone();
    let at = match at {
        Some(at) => at,
        None => {
            s.actions.push(Action::new(app.ui.recording.as_ref()?.1.clone(), vec![]));
            s.actions.len() - 1
        }
    };
    put_recorded(s.actions.get_mut(at)?, steps, app.ui.recording_at);
    Some((s, at))
}

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
    /// The set's action that the recording records into; None for the new action that the
    /// recording adds.
    pub(super) into: Option<usize>,
    /// The new action's name.
    pub(super) name: String,
    /// The steps recorded so far.
    pub(super) steps: Vec<(String, Value)>,
}

/// What Stop Playing/Recording adds to set `set`; None when the recording in progress adds nothing
/// to it.
pub(super) fn recorded_into(app: &VectorcraftApp, set: usize) -> Option<Recorded> {
    let (recording_set, name, start) = app.ui.recording.as_ref()?;
    let into = match app.ui.recording_into {
        Some(id) => Some(app.ui.action_sets.get(set)?.actions.iter().position(|a| a.id == id)?),
        None if *recording_set == set => None,
        None => return None,
    };
    let steps = app.session.journal_for_action(app.session.journal.held().unwrap_or(*start));
    Some(Recorded { into, name: name.clone(), steps })
}

/// Starts recording into set `set`: Stop Playing/Recording adds an action named `name` with the
/// steps, or, with `into`, gives them to the action with that id.
pub(super) fn start_recording(app: &mut VectorcraftApp, set: usize, name: String, into: Option<u64>) {
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

/// Puts the recorded `steps` into `action` before its step at index `at`, or after its last step
/// when `at` is None or past it, included in playback. The steps after them keep whether they are
/// excluded. An exclusion past the action's last step, which saved preferences can hold, goes.
pub(super) fn put_recorded(action: &mut Action, steps: Vec<(String, Value)>, at: Option<usize>) {
    let len = action.steps.len();
    let at = at.map_or(len, |at| at.min(len));
    let added = steps.len();
    action.excluded_steps = action.excluded_steps.iter().filter(|&&i| i < len).map(|&i| if i >= at { i + added } else { i }).collect();
    action.steps.splice(at..at, steps);
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
    match app.ui.recording_into.take() {
        Some(id) => {
            let found = app.ui.action_sets.iter().enumerate().find_map(|(s, set)| Some((s, set.actions.iter().position(|a| a.id == id)?)));
            if let Some((s, a)) = found {
                let action = &mut app.ui.action_sets[s].actions[a];
                let len = action.steps.len();
                let (from, added) = (at.map_or(len, |at| at.min(len)), steps.len());
                put_recorded(action, steps, at);
                follow_recorded(app, (s, a, id), from, added);
            }
        }
        None => {
            if let Some(s) = app.ui.action_sets.get_mut(set) {
                s.actions.push(Action::new(name, steps));
            }
        }
    }
}

/// After Stop Playing/Recording put `added` steps into the action at `place` (set, action, id)
/// before its step at index `from`: the selected step and a Record Stop, Insert Menu Item or Set
/// Selection dialog open on a later step of the action follow their step down.
fn follow_recorded(app: &mut VectorcraftApp, (set, at, id): (usize, usize, u64), from: usize, added: usize) {
    if added == 0 {
        return;
    }
    if let Some(ctx) = app.ui_ctx.clone()
        && let Some((step_id, step)) = pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP).filter(|&(step_id, step)| step_id == id && step >= from)
    {
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((step_id, step + added)));
    }
    let number = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok());
    if let Some(d) = app.ui.dialog.as_mut().filter(|d| holds_step(d))
        && d.fields.get("set").and_then(number) == Some(set)
        && d.fields.get("action").and_then(number) == Some(at)
        && let Some(step) = d.fields.get("step").and_then(number).filter(|&step| step >= from)
    {
        d.fields.insert("step".into(), json!(step + added));
    }
}
