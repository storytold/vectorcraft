use craft::automation::actions::{Action, ActionList, ActionPlayer, ActionRunner, ActionStep, PlaybackBudget, StepOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::{self, recording_room};
use crate::VectorcraftApp;

/// The included steps of `action` as `command.batch` takes them. A recorded `command.batch` step
/// (an action played, or a panel operation that runs as one batch, while recording) gives its own
/// steps, since batches do not nest. With Automatic Align To ([`model::automatic_align_to`]), each
/// `object.align` copy, also one in a recorded Batch run (`action.batch`), gets `to: "auto"` and
/// loses any stored `artboard`; the stored steps keep their recorded params.
// PLAN: menus-w13's root `actions.rs` needs `pub(crate) use playback::included_commands` for menus-w20's `dialogs/action_batch.rs`.
pub(crate) fn included_commands(action: &Action) -> Vec<Value> {
    playback_steps(action).into_iter().map(|s| s.command).collect()
}

/// [`included_commands`], each with the index in the action's steps of the step it comes from: the
/// steps of a recorded `command.batch` share its index.
fn playback_steps(action: &Action) -> Vec<PlayStep> {
    let align = model::automatic_align_to(action);
    action
        .enabled_steps()
        .flat_map(|(row, step)| match step.params.get("commands").and_then(Value::as_array) {
            Some(steps) if step.command == "command.batch" => steps.iter().map(|c| PlayStep { row, command: c.clone() }).collect(),
            _ => vec![PlayStep { row, command: json!({"command": step.command, "params": step.params}) }],
        })
        .map(|mut play| {
            if align {
                automatic_align(&mut play.command);
                if play.command.get("command").and_then(Value::as_str) == Some("action.batch")
                    && let Some(Value::Array(steps)) = play.command.get_mut("params").and_then(|p| p.get_mut("commands"))
                {
                    steps.iter_mut().for_each(automatic_align);
                }
            }
            play
        })
        .collect()
}

/// For command `id` `script.run`: gives its `params` the panel's actions as `actions`, which the
/// script's `app.doScript` plays, unless the caller gave them: every action of every set, in panel
/// order, with its steps as playback runs them ([`playback_steps`]) and each step's place in the
/// action. An action that `actions` cannot hold (a step that is not a command, a name that is too
/// long, or one past the limits of [`vectorcraft_engine::script::actions`]) is left out.
pub(crate) fn add_script_actions(app: &VectorcraftApp, id: &str, params: &mut Value) {
    use vectorcraft_engine::script::actions::{MAX_ACTIONS, MAX_ACTIONS_BYTES, MAX_ALL_STEPS, Playable, parse};
    let Some(p) = params.as_object_mut().filter(|_| id == "script.run") else { return };
    if p.contains_key("actions") {
        return;
    }
    let list = &app.ui.actions;
    let (mut held, mut steps, mut bytes) = (Vec::new(), 0usize, 2usize);
    for set in list.sets() {
        for a in model::members(list, set).into_iter().filter_map(|i| list.action(i)) {
            if held.len() >= MAX_ACTIONS {
                break;
            }
            // The commands of a recorded `command.batch` step go back into one batch step, so that an
            // error names a failing command by the batch's place and its own (`3.2`).
            let mut played: Vec<Value> = Vec::new();
            for s in playback_steps(a) {
                let params = match s.command.get("params") {
                    None | Some(Value::Null) => json!({}),
                    Some(p) => p.clone(),
                };
                let one = json!({"command": s.command.get("command").cloned().unwrap_or(Value::Null), "params": params});
                let place = s.row + 1;
                if a.steps.get(s.row).is_none_or(|step| step.command != "command.batch") {
                    let mut one = one;
                    one["step"] = json!(place);
                    played.push(one);
                    continue;
                }
                match played.last_mut() {
                    Some(last) if last["command"] == "command.batch" && last["step"] == place => {
                        if let Some(Value::Array(inner)) = last.pointer_mut("/params/commands") {
                            inner.push(one);
                        }
                    }
                    _ => played.push(json!({"command": "command.batch", "params": {"commands": [one]}, "step": place})),
                }
            }
            let one = Value::Array(vec![json!({"set": set, "name": a.name, "steps": played})]);
            let Ok(parsed) = parse(Some(&one)) else { continue };
            let count = parsed.first().map_or(0, Playable::len);
            let size = serde_json::to_vec(&one).map_or(usize::MAX, |b| b.len());
            if steps + count > MAX_ALL_STEPS || bytes.saturating_add(size) > MAX_ACTIONS_BYTES {
                continue;
            }
            steps += count;
            bytes += size;
            if let Value::Array(mut one) = one {
                held.append(&mut one);
            }
        }
    }
    p.insert("actions".into(), Value::Array(held));
}

/// The steps of `action` whose command `known` does not hold, in order, as (step index, the index
/// of the command in a `command.batch` step, command id). A `command.batch` step's own commands are
/// checked one by one, as playback runs them ([`included_commands`]); a command of one with no
/// command id has the id "". With `included_only`, excluded steps are passed over.
pub(crate) fn unknown_commands<'a>(
    action: &'a Action,
    known: &'a KnownCommands,
    included_only: bool,
) -> impl Iterator<Item = (usize, Option<usize>, &'a str)> + 'a {
    action.steps.iter().enumerate().filter(move |(i, _)| !included_only || action.step_enabled(*i)).flat_map(
        move |(i, step)| -> Vec<(usize, Option<usize>, &'a str)> {
            match step.params.get("commands").and_then(Value::as_array) {
                Some(steps) if step.command == "command.batch" => steps
                    .iter()
                    .enumerate()
                    .map(|(j, inner)| (i, Some(j), inner.get("command").and_then(Value::as_str).unwrap_or_default()))
                    .filter(|(_, _, id)| !known.has(id))
                    .collect(),
                _ if known.has(&step.command) => vec![],
                _ => vec![(i, None, step.command.as_str())],
            }
        },
    )
}

/// The command ids a step of an action can run now: the built-in engine commands, the commands the
/// installed plug-ins add, the View menu commands the app plays itself ([`crate::menus::plays_in_app`])
/// and stops ([`STOP_STEP`]).
pub(crate) struct KnownCommands(std::collections::HashSet<&'static str>);

impl KnownCommands {
    /// The command ids known when it is called: a plug-in installed or removed later is not in it.
    pub(crate) fn now() -> Self {
        let engine = vectorcraft_engine::command_specs().iter().map(|c| c.id);
        let plugins = vectorcraft_engine::cmd::plugin::command_specs().into_iter().map(|c| c.id);
        Self(engine.chain(plugins).chain([STOP_STEP]).collect())
    }

    /// Whether a step can run command `id`.
    pub(crate) fn has(&self, id: &str) -> bool {
        self.0.contains(id) || crate::menus::plays_in_app(id)
    }
}

/// The one-based place of a step that [`unknown_commands`] gives: `3` for the third step, `3.2`
/// for the second command of a `command.batch` third step.
pub(crate) fn step_place(step: usize, in_batch: Option<usize>) -> String {
    match in_batch {
        Some(j) => format!("{}.{}", step + 1, j + 1),
        None => (step + 1).to_string(),
    }
}

/// `text` as a status message shows it: control characters become `�`, and text past 80
/// characters ends in `…`.
pub(crate) fn shown_text(text: &str) -> String {
    const MAX: usize = 80;
    let mut shown: String = text.chars().take(MAX).map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect();
    if text.chars().nth(MAX).is_some() {
        shown.push('…');
    }
    shown
}

/// A step that playback runs: a command as `command.batch` takes it, and the index in the action's
/// steps of the step it comes from, which an error names.
#[derive(Clone, Debug)]
pub(super) struct PlayStep {
    pub(super) row: usize,
    pub(super) command: Value,
}

impl PlayStep {
    /// The step as the playback's snapshot ([`Playing::list`]) holds it.
    fn into_step(self) -> ActionStep {
        let id = self.command.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
        let params = self.command.get("params").cloned().unwrap_or(Value::Null);
        ActionStep::new(id, params)
    }
}

/// Gives `step`, a command as `command.batch` takes it, `to: "auto"` and no `artboard` when it is
/// an `object.align` step ([`model::automatic_align_to`]); other steps stay as they are.
fn automatic_align(step: &mut Value) {
    if step.get("command").and_then(Value::as_str) != Some("object.align") {
        return;
    }
    let Some(step) = step.as_object_mut() else { return };
    let params = step.entry("params").or_insert_with(|| json!({}));
    if params.is_null() {
        *params = json!({});
    }
    if let Some(params) = params.as_object_mut() {
        params.insert("to".into(), json!("auto"));
        params.remove("artboard");
    }
}

/// Plays an action at the speed [`UiState::action_playback`](crate::UiState::action_playback) sets.
/// Accelerated runs every included step at once as a single undoable batch. Step by Step and Pause
/// For run the first step now, each as its own undo step, and leave the rest to [`tick_playback`].
/// While a paced playback is in progress, another action does not play. An action with an included
/// step whose command is not known now ([`KnownCommands`]), a plug-in's command after the plug-in
/// was removed for one, does not play: the error names the first such step and its command, and no
/// step runs. An excluded step is not checked.
///
/// An action with an included stop ([`STOP_STEP`]) plays up to the stop and waits there at any speed
/// ([`continue_playback`]). At Accelerated, the steps between two stops are one undo step.
///
/// A View menu step ([`is_ui_step`]) runs on its own at every speed, in its place among the steps, and
/// adds no undo step ([`play_ui_step`]). At Accelerated the steps before it and the steps after it are
/// then one undo step each.
///
/// At every speed, the error of a step that fails names the step by its place in the action, 1 for
/// the first, counting excluded steps and stops ([`step_error`]). → {results: the results of the
/// steps run, pending: the steps left, stops included, atStop: true when it waits at a stop}
pub fn play(app: &mut VectorcraftApp, set: usize, idx: usize) -> Result<Value, String> {
    let Some(a) = model::flat(&app.ui.actions, set, idx).and_then(|i| app.ui.actions.action(i)) else { return Err("no such action".into()) };
    let steps = playback_steps(a);
    if steps.is_empty() {
        return Err("the action has no included steps".into());
    }
    if app.ui.action_playing.is_some() {
        return Err("an action is already playing".into());
    }
    // A step whose command is not known now would fail only when playback reaches it, after the
    // steps before it changed the document.
    let known = KnownCommands::now();
    if let Some((step, in_batch, command)) = unknown_commands(a, &known, true).next() {
        let (step, action, command) = (step_place(step, in_batch), shown_text(&a.name), shown_text(command));
        return Err(format!("step {step} of “{action}” runs the unknown command `{command}`"));
    }
    let name = a.name.clone();
    let (rows, steps): (Vec<usize>, Vec<ActionStep>) = steps.into_iter().map(|s| (s.row, s.into_step())).unzip();
    let mut list = ActionList::new();
    list.push(Action::new(name, steps)).map_err(|e| e.to_string())?;
    let playing = Playing {
        list,
        rows,
        from: 0,
        budget: PlaybackBudget::new(),
        options: app.ui.action_playback,
        next_at: None,
        hold: false,
        shown_in: None,
        doc: None,
        played_in: pass_now(app),
        stop: None,
    };
    let results = advance(app, playing)?;
    Ok(playback_result(app, results))
}

/// Whether a step as [`included_commands`] gives it is a View menu command that the app runs
/// itself ([`crate::menus::plays_in_app`]), which `command.batch` does not run.
pub(crate) fn is_ui_step(step: &Value) -> bool {
    step.get("command").and_then(Value::as_str).is_some_and(crate::menus::plays_in_app)
}

/// The app's side of [`ActionPlayer::play`] for one call: it runs the snapshot's steps as saved
/// steps ([`VectorcraftApp::replay`]) at the playback's speed and keeps their results. It lives
/// for the call only; what goes on between calls is in [`Playing`].
struct Runner<'a> {
    app: &'a mut VectorcraftApp,
    speed: PlaybackSpeed,
    /// The place in the action of each step of the snapshot ([`Playing::rows`]).
    rows: &'a [usize],
    /// At Accelerated, the end of the steps the last batch ran: the player reaches them after the
    /// batch, and they answer `Done` without running again.
    ran_to: usize,
    results: Vec<Value>,
    /// The params of the stop the playback reached.
    stop: Option<Value>,
}

impl ActionRunner for Runner<'_> {
    /// A stop pauses. Otherwise a View menu step runs on its own, and any other step runs as one
    /// batch with, at Accelerated, the steps after it up to the next stop or View menu step. At
    /// Step by Step and Pause For each step waits for the next one's turn. The snapshot has no
    /// excluded step, so a run covers only steps the player would reach.
    fn run_step(&mut self, action: &Action, index: usize, step: &ActionStep, _: bool, _: &mut PlaybackBudget) -> StepOutcome {
        if step.command == STOP_STEP {
            self.stop = Some(step.params.clone());
            return StepOutcome::Paused;
        }
        if index < self.ran_to {
            return StepOutcome::Done(Value::Null);
        }
        // PLAN: the snapshot holds a recorded `command.batch`'s commands as steps of their own, so a stop nested in one holds (`a_stop_nested_in_a_recorded_batch_holds_too`); `rows` gives the player's index its place in the action for `step_error`.
        let ran = if crate::menus::plays_in_app(&step.command) {
            let row = self.rows.get(index).copied().unwrap_or(index);
            play_ui_step(self.app, row, step).map(|r| vec![r])
        } else {
            let end = match self.speed {
                PlaybackSpeed::Accelerated => action
                    .steps
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, s)| s.command == STOP_STEP || crate::menus::plays_in_app(&s.command))
                    .map_or(action.steps.len(), |(i, _)| i),
                PlaybackSpeed::Step | PlaybackSpeed::Pause => index + 1,
            };
            self.ran_to = end;
            run_batch(self.app, &action.name, self.rows.get(index..end).unwrap_or_default(), action.steps.get(index..end).unwrap_or_default())
        };
        match ran {
            Err(e) => StepOutcome::Failed(e),
            Ok(results) => {
                let first = results.first().cloned().unwrap_or(Value::Null);
                self.results.extend(results);
                match self.speed {
                    PlaybackSpeed::Accelerated => StepOutcome::Done(first),
                    PlaybackSpeed::Step | PlaybackSpeed::Pause => StepOutcome::Wait(first),
                }
            }
        }
    }
}

/// Runs `steps`, whose places in the action are `rows`, in order as one `command.batch` named
/// `label`, which is one undo step → their results. A step that fails ends the batch with its
/// error, which names the step's place in the action ([`step_error`]), and the batch changes
/// nothing.
fn run_batch(app: &mut VectorcraftApp, label: &str, rows: &[usize], steps: &[ActionStep]) -> Result<Vec<Value>, String> {
    let commands: Vec<Value> = steps.iter().map(|s| json!({"command": s.command, "params": s.params})).collect();
    let r = match app.replay("command.batch", json!({"label": label, "commands": commands})) {
        Ok(r) => r,
        Err(e) => {
            // The batch numbers its steps from 0 within this run of them, not within the action.
            if let Some((n, id, reason)) = batch_step_failure(&e)
                && let Some(&row) = rows.get(n)
            {
                return step_error(row, id, reason);
            }
            return Err(e);
        }
    };
    Ok(r.get("results").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// Plays `playing` on from its next step at its speed → the results of the steps that ran. The
/// playback stays in progress ([`crate::UiState::action_playing`]) while steps are left after a
/// Step by Step or Pause For step, and while it waits at a stop. It ends after its last step, and
/// at a step that fails, whose error it returns: the steps before it stay done.
fn advance(app: &mut VectorcraftApp, mut playing: Playing) -> Result<Vec<Value>, String> {
    let mut runner = Runner { app: &mut *app, speed: playing.options.speed, rows: &playing.rows, ran_to: 0, results: vec![], stop: None };
    let report = ActionPlayer::play(&playing.list, 0, playing.from, &mut runner, &mut playing.budget);
    let (results, stop) = (runner.results, runner.stop);
    if let Some(failure) = report.failed {
        return Err(failure.message);
    }
    let Some(at) = report.paused_at else { return Ok(results) };
    playing.from = at + 1;
    (playing.next_at, playing.shown_in) = (None, None);
    // A step of the action may open, close or switch documents itself.
    playing.doc = active_doc(app);
    match stop {
        Some(params) => hold_at_stop(app, playing, params),
        None if playing.pending() > 0 => {
            playing.hold = playing.options.speed == PlaybackSpeed::Step;
            app.ui.action_playing = Some(playing);
        }
        None => return Ok(results),
    }
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    Ok(results)
}

/// The step number, command id and reason in the error `command.batch` gives when one of its steps
/// fails (``batch step 2 (`object.move`) failed: …``); none for any other error, such as one about
/// the whole batch.
fn batch_step_failure(e: &str) -> Option<(usize, &str, &str)> {
    let rest = e.strip_prefix("batch step ")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let n = rest[..digits].parse().ok()?;
    let (id, reason) = rest[digits..].strip_prefix(" (`")?.split_once("`) failed: ")?;
    Some((n, id, reason))
}

/// The error of a played step with command `id` that failed with `reason`, naming the step by its
/// place in the action: 1 for the action's first step, counting excluded steps and stops. A step of
/// a recorded `command.batch` is named by the batch's place and its own command.
fn step_error<T>(row: usize, id: &str, reason: &str) -> Result<T, String> {
    Err(format!("action step {} (`{id}`) failed: {reason}", row + 1))
}

/// Runs a View menu step, at place `row` in the action, as its menu item runs it, which adds no
/// undo step → its result. While an action records, the step goes into the recording too, as a
/// stop does, unless it does not fit in the action ([`recording_room`]): then the status bar shows
/// the reason.
fn play_ui_step(app: &mut VectorcraftApp, row: usize, step: &ActionStep) -> Result<Value, String> {
    let params = if step.params.is_null() { json!({}) } else { step.params.clone() };
    let r = match app.replay(&step.command, params.clone()) {
        Ok(r) => r,
        Err(e) => return step_error(row, &step.command, &e),
    };
    if app.ui.recording.is_some() {
        match recording_room(app, &step.command, &params) {
            Ok(()) => app.session.journal.push((step.command.clone(), params)),
            Err(e) => app.status(e),
        }
    }
    Ok(r)
}

/// What [`play`] and [`continue_playback`] return.
fn playback_result(app: &VectorcraftApp, results: Vec<Value>) -> Value {
    let pending = app.ui.action_playing.as_ref().map_or(0, Playing::pending);
    let mut result = json!({"results": results, "pending": pending});
    if waiting_at_stop(app) {
        result["atStop"] = json!(true);
    }
    result
}

/// The command id of a stop in an action's steps, with {message, allowContinue} as its params. No
/// command runs under this id: playback holds where the step is.
pub const STOP_STEP: &str = "action.stop";

/// Whether a step as [`included_commands`] gives it is a stop.
fn is_stop(step: &Value) -> bool {
    step.get("command").and_then(Value::as_str) == Some(STOP_STEP)
}

/// Whether one of the steps as [`included_commands`] gives them is a stop.
pub(crate) fn has_stop(commands: &[Value]) -> bool {
    commands.iter().any(is_stop)
}

/// The message and Allow Continue setting in a stop's params; each may be left out. A saved stop
/// with no params reads as null here. A value of another type, or params that are neither null
/// nor an object, is an error.
pub(crate) fn stop_params(p: &Value) -> Result<(String, bool), String> {
    if !(p.is_null() || p.is_object()) {
        return Err("`params` must be an object".into());
    }
    let message = match p.get("message") {
        None => "",
        Some(v) => v.as_str().ok_or("message must be text")?,
    };
    let allow_continue = match p.get("allowContinue") {
        None => false,
        Some(v) => v.as_bool().ok_or("allowContinue must be true or false")?,
    };
    Ok((message.to_string(), allow_continue))
}

/// The stop a playback waits at. A saved value of the wrong type reads as left out.
#[derive(Clone, Debug)]
pub(super) struct AtStop {
    pub(super) message: String,
    pub(super) allow_continue: bool,
    /// Whether the message box was shown. It waits for a dialog already open to close.
    pub(super) shown: bool,
}

/// Waits at the stop with `params` that `playing` reached. A recording in progress records the
/// stop too, unless the stop does not fit in its action ([`recording_room`]): then the status bar
/// shows the reason.
fn hold_at_stop(app: &mut VectorcraftApp, mut playing: Playing, params: Value) {
    let message = params.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
    let allow_continue = params.get("allowContinue").and_then(Value::as_bool).unwrap_or(false);
    playing.stop = Some(AtStop { message, allow_continue, shown: false });
    playing.hold = false;
    app.ui.action_playing = Some(playing);
    if app.ui.recording.is_some() {
        match recording_room(app, STOP_STEP, &params) {
            Ok(()) => app.session.journal.push((STOP_STEP.to_string(), params)),
            Err(e) => app.status(e),
        }
    }
    show_stop_message(app);
}

/// Opens the message box of the stop the playback waits at, once, when no other dialog is open.
fn show_stop_message(app: &mut VectorcraftApp) {
    if app.any_dialog_open() {
        return;
    }
    let Some(playing) = app.ui.action_playing.as_mut() else { return };
    let label = playing.name().to_string();
    let Some(stop) = playing.stop.as_mut().filter(|s| !s.shown) else { return };
    stop.shown = true;
    let (message, allow_continue) = (stop.message.clone(), stop.allow_continue);
    crate::dialogs::action_stop::open_message(app, &label, &message, allow_continue);
}

/// Closes the message box of a stop, if it is open.
pub(super) fn close_stop_message(app: &mut VectorcraftApp) {
    // PLAN: assumed menus-w20's `pub(crate) fn close_message(app: &VectorcraftApp)` (decision: cancel then take the `StopMessage`).
    crate::dialogs::action_stop::close_message(app);
}

/// Whether the playback in progress waits at a stop.
pub fn waiting_at_stop(app: &VectorcraftApp) -> bool {
    app.ui.action_playing.as_ref().is_some_and(|p| p.stop.is_some())
}

/// The command that plays on from a stop ([`continue_playback`]).
pub const CONTINUE_PLAYBACK: &str = "ui.action.continue";

/// The window's current pass once its tick has begun, else `None`: a step played before the tick of a pass (or with
/// no window) lets the next tick start the pause.
fn pass_now(app: &VectorcraftApp) -> Option<u64> {
    app.ui_ctx.as_ref().map(egui::Context::cumulative_pass_nr).filter(|&pass| app.ticked_pass == Some(pass))
}

/// `ui.action.continue {}`: plays on from the stop the playback waits at, on the document active
/// now, and closes the stop's message box. The next step runs at once, or at Accelerated the steps
/// up to the next stop as one undo step; a failing step ends the playback with its error. With no
/// playback waiting at a stop it is an error. → {results, pending, atStop?}, as [`play`]
pub fn continue_playback(app: &mut VectorcraftApp, _: &Value) -> Result<Value, String> {
    let Some(mut playing) = app.ui.action_playing.take_if(|p| p.stop.is_some()) else {
        return Err("no action is waiting at a stop".into());
    };
    playing.stop = None;
    playing.played_in = pass_now(app);
    close_stop_message(app);
    let results = advance(app, playing)?;
    Ok(playback_result(app, results))
}

/// The speed actions play at (Actions panel menu › Playback Options…).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackSpeed {
    /// Every included step at once.
    #[default]
    Accelerated,
    /// One step at a time, with the window drawn after each step before the next step runs.
    Step,
    /// [`PlaybackOptions::seconds`] between steps.
    Pause,
}

/// The pause between steps that Pause For takes, in whole seconds: VectorCraft's own range.
pub const PAUSE_SECONDS: std::ops::RangeInclusive<u32> = 1..=60;

/// How actions play, kept with the UI preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackOptions {
    pub speed: PlaybackSpeed,
    /// The pause between steps with [`PlaybackSpeed::Pause`], within [`PAUSE_SECONDS`]. It is kept
    /// while another speed is chosen.
    pub seconds: u32,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self { speed: PlaybackSpeed::Accelerated, seconds: *PAUSE_SECONDS.start() }
    }
}

/// Reads saved [`PlaybackOptions`]: a value that is not a valid one loads as the default.
pub fn deserialize_playback<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PlaybackOptions, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value::<PlaybackOptions>(v).ok().filter(|o| PAUSE_SECONDS.contains(&o.seconds)).unwrap_or_default())
}

/// A playback in progress, between the steps it runs: one that waits for its next step at Step by
/// Step or Pause For, or at a stop.
#[derive(Clone, Debug)]
pub struct Playing {
    /// The action as it plays, alone in a list: its included steps as [`included_commands`] gives
    /// them (Automatic Align applied, a recorded `command.batch`'s commands one by one), under the
    /// action's name, which names each undo step. Edits to the panel's list while it plays do not
    /// change it.
    list: ActionList,
    /// The place in the action of each step of [`Playing::list`], which an error names.
    rows: Vec<usize>,
    /// The index in [`Playing::list`] of the next step to play.
    from: usize,
    /// The steps the playback has run, counted against the player's limit.
    budget: PlaybackBudget,
    /// The options the action started playing with.
    pub(super) options: PlaybackOptions,
    /// With Pause For, when the next step runs (seconds on the app's clock); none until the frame
    /// after each step, which starts the pause before the next one.
    next_at: Option<f64>,
    /// With Step by Step, set after each step until the window shows a frame drawn after it
    /// ([`playback_drawn`]): no step runs while it is set, so the window shows each step's result
    /// before the next one runs, also when Play in the panel ran after the canvas drew and while the
    /// window is minimized or covered.
    pub(super) hold: bool,
    /// With Step by Step, the frame whose pass showed the last step ([`playback_shown`]): the hold
    /// ends in a later frame, not in a pass egui runs again within that frame.
    shown_in: Option<u64>,
    /// The [`uid`](vectorcraft_engine::DocState::uid) of the document that was active after the
    /// last step played, if any.
    pub(super) doc: Option<u64>,
    /// The pass in which Play or Continue ran a step ([`pass_now`]): the pause before the next step
    /// starts in a later pass.
    played_in: Option<u64>,
    /// The stop the playback waits at, if any: no step runs until [`continue_playback`] or
    /// [`stop`](super::stop).
    pub(super) stop: Option<AtStop>,
}

impl Playing {
    /// The action's name.
    fn name(&self) -> &str {
        self.list.action(0).map_or("", |a| a.name.as_str())
    }

    /// The steps still to play, stops included.
    fn pending(&self) -> usize {
        self.list.step_count(0).saturating_sub(self.from)
    }

    /// Whether the next step is a stop.
    fn stop_next(&self) -> bool {
        self.list.action(0).and_then(|a| a.steps.get(self.from)).is_some_and(|s| s.command == STOP_STEP)
    }
}

fn active_doc(app: &VectorcraftApp) -> Option<u64> {
    app.session.active().map(|d| d.uid)
}

/// Runs the next step of the paced playback in progress when it is due at `now` (seconds on the
/// app's clock): with Step by Step once the window has shown a frame drawn after the step before
/// ([`playback_drawn`]), so it shows each step before the next one runs; with Pause For once
/// the pause has passed, which starts in the frame after the step before, so a slow step or a late
/// frame does not shorten it. A step that fails ends the playback and shows its error. When a step
/// is due and the active document is not the one that was active after the last step (another
/// document was made active, or that one was closed), the playback ends with a status and the rest
/// do not run, except that a stop is held whichever document is active. → the seconds until the
/// window should draw again while playback goes on (none while Step by Step waits for a UI pass,
/// which asks for the next frame itself)
pub fn tick_playback(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    let doc = active_doc(app);
    let pass = pass_now(app);
    let frame = app.ui_ctx.as_ref().map(egui::Context::cumulative_frame_nr);
    let playing = app.ui.action_playing.as_mut()?;
    if playing.hold && playing.shown_in.is_some_and(|shown| frame > Some(shown)) {
        playing.hold = false;
    }
    if playing.stop.is_some() {
        show_stop_message(app);
        return None;
    }
    if playing.hold {
        return None;
    }
    let pause = f64::from(playing.options.seconds);
    if playing.options.speed == PlaybackSpeed::Pause {
        if playing.next_at.is_none() && playing.played_in.is_some_and(|played| pass == Some(played)) {
            return Some(0.0);
        }
        let due = *playing.next_at.get_or_insert(now + pause);
        if now < due {
            return Some(due - now);
        }
    }
    if !playing.stop_next() && playing.doc != doc {
        app.ui.action_playing = None;
        app.status("the action stopped playing: its document is no longer active");
        return None;
    }
    let playing = app.ui.action_playing.take()?;
    if let Err(e) = advance(app, playing) {
        app.status(e);
        return None;
    }
    // A step that waits for its turn asks for the frame that starts it; a stop waits for the user.
    app.ui.action_playing.as_ref().filter(|p| p.stop.is_none()).map(|_| 0.0)
}

/// Whether a Step by Step playback waits for a frame that shows its last step ([`playback_drawn`]).
pub fn playback_held(app: &VectorcraftApp) -> bool {
    app.ui.action_playing.as_ref().is_some_and(|p| p.hold)
}

/// Called after a UI pass that the window shows, when [`playback_held`] was true as the pass began:
/// the Step by Step playback runs its next step in the frame after this one, which it asks for.
pub fn playback_drawn(app: &mut VectorcraftApp) {
    if let Some(playing) = app.ui.action_playing.as_mut().filter(|p| p.hold) {
        playing.hold = false;
        if let Some(ctx) = &app.ui_ctx {
            ctx.request_repaint();
        }
    }
}

/// Called after a UI pass that the window shows, when [`playback_held`] was true as the pass began:
/// the Step by Step playback runs its next step in the next frame ([`playback_drawn`] there), which
/// it asks for. A pass egui runs again in this frame runs no step.
pub fn playback_shown(app: &mut VectorcraftApp, frame: u64) {
    if let Some(playing) = app.ui.action_playing.as_mut().filter(|p| p.hold) {
        playing.shown_in = Some(frame);
        if let Some(ctx) = &app.ui_ctx {
            ctx.request_repaint();
        }
    }
}

/// The command behind Playback Options… ([`playback_options`]).
pub const PLAYBACK_OPTIONS: &str = "ui.action.playbackOptions";

/// `ui.action.playbackOptions {speed?, seconds?}`: sets how actions play. `speed` is
/// `accelerated` (every step at once, the default), `step` (one step at a time, the window drawn
/// after each) or `pause` (`seconds` between steps); `seconds` is a whole number in
/// [`PAUSE_SECONDS`]. A param left out keeps its value. Any other value is an error, and then
/// nothing changes. An action already playing keeps its speed. → {speed, seconds}
pub fn playback_options(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let current = app.ui.action_playback;
    let speed = match p.get("speed") {
        None => current.speed,
        Some(v) => match v.as_str() {
            Some("accelerated") => PlaybackSpeed::Accelerated,
            Some("step") => PlaybackSpeed::Step,
            Some("pause") => PlaybackSpeed::Pause,
            _ => return Err("speed must be accelerated, step or pause".into()),
        },
    };
    let seconds = match p.get("seconds") {
        None => current.seconds,
        Some(v) => v
            .as_u64()
            .and_then(|s| u32::try_from(s).ok())
            .filter(|s| PAUSE_SECONDS.contains(s))
            .ok_or("seconds must be a whole number from 1 to 60")?,
    };
    app.ui.action_playback = PlaybackOptions { speed, seconds };
    Ok(json!({"speed": speed, "seconds": seconds}))
}

/// The command that plays an action by its name ([`play_action`]).
pub const PLAY_ACTION: &str = "ui.action.play";

/// `ui.action.play {action, set?}`: plays the action named `action` at the Playback Options speed,
/// as Play Current Selection does ([`play`]). Names are unique in the list; `set` names the set the
/// action must be in (default: any set), and a `set` that is not a string is an error. A name that
/// no action has, or none in `set`, is an error, and then nothing plays. → what [`play`] returns
pub fn play_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let name = p.get("action").and_then(Value::as_str).ok_or("no such action")?;
    let in_set = match p.get("set") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or("no such action set")?),
    };
    let list = &app.ui.actions;
    let index =
        list.actions().iter().position(|a| a.name == name && in_set.is_none_or(|set| a.set.as_deref() == Some(set))).ok_or("no such action")?;
    let (set, at) = model::place(list, index).ok_or("no such action")?;
    play(app, set, at)
}

/// The command that plays an action by its place in the panel ([`play_at`]).
pub const PLAY_AT: &str = "ui.action.playAt";

/// `ui.action.playAt {set, action}`: plays the action at index `action` of set `set` (0 is the top
/// set) at the Playback Options speed, as [`play`] does; a click on an action's button in Button
/// Mode runs this. A missing index, or one that is not a whole number or is past the last set or
/// action, is an error, and then nothing plays. → what [`play`] returns
pub fn play_at(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let (set, at) = index("set").zip(index("action")).ok_or("no such action")?;
    play(app, set, at)
}
