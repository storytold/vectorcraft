use super::*;

impl Action {
    /// The included steps as `command.batch` takes them. A recorded `command.batch` step (an action
    /// played, or a panel operation that runs as one batch, while recording) gives its own steps,
    /// since batches do not nest. With [`Action::automatic_align_to`], each `object.align` copy, also
    /// one in a recorded Batch run (`action.batch`), gets `to: "auto"` and loses any stored
    /// `artboard`; the stored steps keep their recorded params.
    pub(crate) fn included_commands(&self) -> Vec<Value> {
        self.playback_steps().into_iter().map(|s| s.command).collect()
    }

    /// [`Action::included_commands`], each with the index in [`Action::steps`] of the step it comes
    /// from: the steps of a recorded `command.batch` share its index.
    fn playback_steps(&self) -> Vec<PlayStep> {
        self.steps
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.excluded_steps.contains(i))
            .flat_map(|(row, (command, params))| match params.get("commands").and_then(Value::as_array) {
                Some(steps) if command == "command.batch" => steps.iter().map(|c| PlayStep { row, command: c.clone() }).collect(),
                _ => vec![PlayStep { row, command: json!({"command": command, "params": params}) }],
            })
            .map(|mut play| {
                if self.automatic_align_to {
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

    /// The steps whose command `known` does not hold, in order, as (step index, the index of the
    /// command in a `command.batch` step, command id). A `command.batch` step's own commands are
    /// checked one by one, as playback runs them ([`Action::included_commands`]); a command of one
    /// with no command id has the id "". With `included_only`, excluded steps are passed over.
    pub(crate) fn unknown_commands<'a>(
        &'a self,
        known: &'a KnownCommands,
        included_only: bool,
    ) -> impl Iterator<Item = (usize, Option<usize>, &'a str)> + 'a {
        self.steps.iter().enumerate().filter(move |(i, _)| !(included_only && self.excluded_steps.contains(i))).flat_map(
            |(i, (command, params))| -> Vec<(usize, Option<usize>, &'a str)> {
                match params.get("commands").and_then(Value::as_array) {
                    Some(steps) if command == "command.batch" => steps
                        .iter()
                        .enumerate()
                        .map(|(j, step)| (i, Some(j), step.get("command").and_then(Value::as_str).unwrap_or_default()))
                        .filter(|(_, _, id)| !known.has(id))
                        .collect(),
                    _ if known.has(command) => vec![],
                    _ => vec![(i, None, command.as_str())],
                }
            },
        )
    }
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

/// The one-based place of a step that [`Action::unknown_commands`] gives: `3` for the third step,
/// `3.2` for the second command of a `command.batch` third step.
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

/// A step that playback runs: a command as `command.batch` takes it, and the index in
/// [`Action::steps`] of the step it comes from, which an error names.
#[derive(Clone, Debug)]
pub(super) struct PlayStep {
    pub(super) row: usize,
    pub(super) command: Value,
}

/// Gives `step`, a command as `command.batch` takes it, `to: "auto"` and no `artboard` when it is
/// an `object.align` step ([`Action::automatic_align_to`]); other steps stay as they are.
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
/// Accelerated runs every included step at once as a single undoable batch → what `command.batch`
/// returns, {results}. Step by Step and Pause For run the first step now, each as its own undo step,
/// and leave the rest to [`tick_playback`] → {results: [the first step's result], pending: the steps
/// left}. While a paced playback is in progress, another action does not play. An action with an
/// included step whose command is not known now ([`KnownCommands`]), a plug-in's command after the
/// plug-in was removed for one, does not play: the error names the first such step and its command,
/// and no step runs. An excluded step is not checked.
///
/// An action with an included stop ([`STOP_STEP`]) plays up to the stop and waits there at any speed
/// ([`continue_playback`]). At Accelerated, the steps between two stops are one undo step. → {results:
/// [the results of the steps run], pending: the steps left, stops included, atStop: true when it waits}
///
/// A View menu step ([`is_ui_step`]) runs on its own at every speed, in its place among the steps, and
/// adds no undo step ([`run_steps`]). At Accelerated the steps before it and the steps after it are
/// then one undo step each.
///
/// At every speed, the error of a step that fails names the step by its place in the action, 1 for
/// the first, counting excluded steps and stops ([`step_error`]).
pub fn play(app: &mut VectorcraftApp, set: usize, idx: usize) -> Result<Value, String> {
    let Some(a) = app.ui.action_sets.get(set).and_then(|s| s.actions.get(idx)).cloned() else { return Err("no such action".into()) };
    let commands = a.playback_steps();
    if commands.is_empty() {
        return Err("the action has no included steps".into());
    }
    if app.ui.action_playing.is_some() {
        return Err("an action is already playing".into());
    }
    // A step whose command is not known now would fail only when playback reaches it, after the
    // steps before it changed the document.
    let known = KnownCommands::now();
    if let Some((step, in_batch, command)) = a.unknown_commands(&known, true).next() {
        let (step, action, command) = (step_place(step, in_batch), shown_text(&a.name), shown_text(command));
        return Err(format!("step {step} of “{action}” runs the unknown command `{command}`"));
    }
    let options = app.ui.action_playback;
    if commands.iter().any(|s| is_stop(&s.command)) {
        return play_with_stops(app, a.name, commands, options);
    }
    if options.speed == PlaybackSpeed::Accelerated {
        return Ok(json!({"results": run_steps(app, &a.name, commands)?}));
    }
    let mut steps: std::collections::VecDeque<PlayStep> = commands.into();
    let Some(first) = steps.pop_front() else { return Err("the action has no included steps".into()) };
    let result = play_step(app, &a.name, first)?;
    let pending = steps.len();
    if pending > 0 {
        let hold = options.speed == PlaybackSpeed::Step;
        let doc = active_doc(app);
        let played_in = app.tick_iteration.filter(|&t| t == app.host_iteration);
        app.ui.action_playing = Some(Playing { label: a.name, steps, options, next_at: None, hold, doc, played_in, stop: None });
        if let Some(ctx) = &app.ui_ctx {
            ctx.request_repaint();
        }
    }
    Ok(json!({"results": [result], "pending": pending}))
}

/// Runs one step of a paced playback as its own undo step, named `label`, or a View menu step with
/// no undo step ([`run_steps`]) → the step's result.
fn play_step(app: &mut VectorcraftApp, label: &str, step: PlayStep) -> Result<Value, String> {
    Ok(run_steps(app, label, vec![step])?.into_iter().next().unwrap_or(Value::Null))
}

/// Whether a step as [`Action::included_commands`] gives it is a View menu command that the app runs
/// itself ([`crate::menus::plays_in_app`]), which `command.batch` does not run.
pub(crate) fn is_ui_step(step: &Value) -> bool {
    step.get("command").and_then(Value::as_str).is_some_and(crate::menus::plays_in_app)
}

/// Runs `steps` in order, as saved steps ([`VectorcraftApp::replay`]) → their results. The steps
/// between two View menu steps ([`is_ui_step`]) run as one `command.batch` named `label`, which is one
/// undo step; a View menu step runs on its own ([`play_ui_step`]). A step that fails ends the run with
/// its error: the batch it is in changes nothing, and the steps before that batch stay done. The error
/// of a step names the step's place in the action ([`step_error`]).
fn run_steps(app: &mut VectorcraftApp, label: &str, steps: Vec<PlayStep>) -> Result<Vec<Value>, String> {
    fn batch(app: &mut VectorcraftApp, label: &str, run: Vec<PlayStep>, results: &mut Vec<Value>) -> Result<(), String> {
        if run.is_empty() {
            return Ok(());
        }
        let (rows, run): (Vec<usize>, Vec<Value>) = run.into_iter().map(|s| (s.row, s.command)).unzip();
        let r = match app.replay("command.batch", json!({"label": label, "commands": run})) {
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
        results.extend(r.get("results").and_then(Value::as_array).cloned().unwrap_or_default());
        Ok(())
    }
    let mut results = Vec::with_capacity(steps.len());
    let mut run = vec![];
    for step in steps {
        if is_ui_step(&step.command) {
            batch(app, label, std::mem::take(&mut run), &mut results)?;
            results.push(play_ui_step(app, &step)?);
        } else {
            run.push(step);
        }
    }
    batch(app, label, run, &mut results)?;
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

/// Runs a View menu step as its menu item runs it, which adds no undo step → its result. While an
/// action records, the step goes into the recording too, as a stop does, unless it does not fit in
/// the action ([`recording_room`]): then the status bar shows the reason.
fn play_ui_step(app: &mut VectorcraftApp, step: &PlayStep) -> Result<Value, String> {
    let command = step.command.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = step.command.get("params").filter(|p| !p.is_null()).cloned().unwrap_or_else(|| json!({}));
    let r = match app.replay(&command, params.clone()) {
        Ok(r) => r,
        Err(e) => return step_error(step.row, &command, &e),
    };
    if app.ui.recording.is_some() {
        match recording_room(app, &command, &params) {
            Ok(()) => app.session.journal.push((command, params)),
            Err(e) => app.status(e),
        }
    }
    Ok(r)
}

/// [`play`] for an action with a stop: runs what comes before the first stop, or at Step by Step and
/// Pause For the first step, and leaves the rest to [`tick_playback`] and [`continue_playback`].
fn play_with_stops(app: &mut VectorcraftApp, label: String, commands: Vec<PlayStep>, options: PlaybackOptions) -> Result<Value, String> {
    let mut steps: std::collections::VecDeque<PlayStep> = commands.into();
    let run = next_run(&mut steps, options.speed);
    let ran = !run.is_empty();
    let results = if ran { run_steps(app, &label, run)? } else { vec![] };
    app.ui.action_playing = Some(Playing { label, steps, options, next_at: None, hold: false, doc: None, played_in: None, stop: None });
    after_run(app, ran);
    Ok(playback_result(app, results))
}

/// Takes the steps that run next as one undo step off the front of `steps`: at Accelerated all of
/// them up to the next stop, at the other speeds one. None when a stop comes next.
fn next_run(steps: &mut std::collections::VecDeque<PlayStep>, speed: PlaybackSpeed) -> Vec<PlayStep> {
    let mut run = vec![];
    while steps.front().is_some_and(|s| !is_stop(&s.command)) {
        run.extend(steps.pop_front());
        if speed != PlaybackSpeed::Accelerated {
            break;
        }
    }
    run
}

/// Sets up the playback in progress after steps ran (`ran`), or none did: the next pause and frame
/// start over, and a stop that comes next at Accelerated, or with no step run before it, holds now.
fn after_run(app: &mut VectorcraftApp, ran: bool) {
    let doc = active_doc(app);
    let played_in = app.tick_iteration.filter(|&t| t == app.host_iteration);
    let Some(playing) = app.ui.action_playing.as_mut() else { return };
    playing.doc = doc;
    playing.played_in = played_in;
    playing.next_at = None;
    playing.hold = ran && playing.options.speed == PlaybackSpeed::Step;
    if (!ran || playing.options.speed == PlaybackSpeed::Accelerated) && playing.steps.front().is_some_and(|s| is_stop(&s.command)) {
        hold_at_stop(app);
    }
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
}

/// What [`play`] and [`continue_playback`] return.
fn playback_result(app: &VectorcraftApp, results: Vec<Value>) -> Value {
    let pending = app.ui.action_playing.as_ref().map_or(0, |p| p.steps.len());
    let mut result = json!({"results": results, "pending": pending});
    if waiting_at_stop(app) {
        result["atStop"] = json!(true);
    }
    result
}

/// The command id of a stop in [`Action::steps`], with {message, allowContinue} as its params. No
/// command runs under this id: playback holds where the step is.
pub const STOP_STEP: &str = "action.stop";

/// Whether a step as [`Action::included_commands`] gives it is a stop.
fn is_stop(step: &Value) -> bool {
    step.get("command").and_then(Value::as_str) == Some(STOP_STEP)
}

/// Whether one of the steps as [`Action::included_commands`] gives them is a stop.
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

/// Takes the stop at the front of the playback's steps and waits there. A recording in progress
/// records the stop too, unless the stop does not fit in its action ([`recording_room`]): then the
/// status bar shows the reason.
fn hold_at_stop(app: &mut VectorcraftApp) {
    let Some(playing) = app.ui.action_playing.as_mut() else { return };
    let Some(step) = playing.steps.pop_front() else { return };
    let params = step.command.get("params").cloned().unwrap_or(Value::Null);
    let message = params.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
    let allow_continue = params.get("allowContinue").and_then(Value::as_bool).unwrap_or(false);
    playing.stop = Some(AtStop { message, allow_continue, shown: false });
    playing.hold = false;
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
    if app.ui.dialog.is_some() {
        return;
    }
    let Some(playing) = app.ui.action_playing.as_mut() else { return };
    let label = playing.label.clone();
    let Some(stop) = playing.stop.as_mut().filter(|s| !s.shown) else { return };
    stop.shown = true;
    let (message, allow_continue) = (stop.message.clone(), stop.allow_continue);
    crate::dialogs::action_stop::open_message(app, &label, &message, allow_continue);
}

/// Closes the message box of a stop, if it is open.
pub(super) fn close_stop_message(app: &mut VectorcraftApp) {
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == crate::dialogs::action_stop::MESSAGE_KIND) {
        app.ui.dialog = None;
    }
}

/// Whether the playback in progress waits at a stop.
pub fn waiting_at_stop(app: &VectorcraftApp) -> bool {
    app.ui.action_playing.as_ref().is_some_and(|p| p.stop.is_some())
}

/// The command that plays on from a stop ([`continue_playback`]).
pub const CONTINUE_PLAYBACK: &str = "ui.action.continue";

/// `ui.action.continue {}`: plays on from the stop the playback waits at, on the document active
/// now, and closes the stop's message box. The next step runs at once, or at Accelerated the steps
/// up to the next stop as one undo step; a failing step ends the playback with its error. With no
/// playback waiting at a stop it is an error. → {results, pending, atStop?}, as [`play`]
pub fn continue_playback(app: &mut VectorcraftApp, _: &Value) -> Result<Value, String> {
    let Some(playing) = app.ui.action_playing.as_mut().filter(|p| p.stop.is_some()) else {
        return Err("no action is waiting at a stop".into());
    };
    playing.stop = None;
    let run = next_run(&mut playing.steps, playing.options.speed);
    let label = playing.label.clone();
    if playing.steps.is_empty() {
        app.ui.action_playing = None;
    }
    close_stop_message(app);
    let ran = !run.is_empty();
    let results = if ran { run_steps(app, &label, run).inspect_err(|_| app.ui.action_playing = None)? } else { vec![] };
    after_run(app, ran);
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

/// A paced playback in progress: the steps still to run.
#[derive(Clone, Debug)]
pub struct Playing {
    /// The action's name, which names each step's undo step.
    pub(super) label: String,
    pub(super) steps: std::collections::VecDeque<PlayStep>,
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
    /// The [`uid`](vectorcraft_engine::DocState::uid) of the document that was active after the
    /// last step played, if any.
    pub(super) doc: Option<u64>,
    /// The host iteration in which Play ran the first step, when that iteration's [`tick`] had
    /// already begun (a control request, or the UI pass after it): the pause before the second step
    /// starts in a later iteration.
    ///
    /// [`tick`]: VectorcraftApp::tick
    played_in: Option<u64>,
    /// The stop the playback waits at, if any: no step runs until [`continue_playback`] or
    /// [`stop`].
    pub(super) stop: Option<AtStop>,
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
/// do not run, except that a stop is held whichever document is active. → the seconds until the window should draw again while playback goes on (none while
/// Step by Step waits for a UI pass, which asks for the next frame itself)
pub fn tick_playback(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    let doc = active_doc(app);
    let playing = app.ui.action_playing.as_mut()?;
    if playing.stop.is_some() {
        show_stop_message(app);
        return None;
    }
    if playing.hold {
        return None;
    }
    let pause = f64::from(playing.options.seconds);
    if playing.options.speed == PlaybackSpeed::Pause {
        if playing.next_at.is_none() && playing.played_in == Some(app.host_iteration) {
            return Some(0.0);
        }
        let due = *playing.next_at.get_or_insert(now + pause);
        if now < due {
            return Some(due - now);
        }
    }
    if playing.steps.front().is_some_and(|s| is_stop(&s.command)) {
        hold_at_stop(app);
        return None;
    }
    if playing.doc != doc {
        app.ui.action_playing = None;
        app.status("the action stopped playing: its document is no longer active");
        return None;
    }
    let Some(step) = playing.steps.pop_front() else {
        app.ui.action_playing = None;
        return None;
    };
    let label = playing.label.clone();
    if playing.steps.is_empty() {
        app.ui.action_playing = None;
    }
    if let Err(e) = play_step(app, &label, step) {
        app.ui.action_playing = None;
        app.status(e);
        return None;
    }
    let doc = active_doc(app);
    let playing = app.ui.action_playing.as_mut()?;
    // A step of the action may open, close or switch documents itself.
    playing.doc = doc;
    // With Pause For, the next frame starts the next pause.
    playing.next_at = None;
    playing.hold = playing.options.speed == PlaybackSpeed::Step;
    Some(0.0)
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
/// as Play Current Selection does ([`play`]). `set` names the set to look in (default: every set); a
/// `set` that is not a string is an error. A name that no action has is an error, and so is a name
/// that more than one action in the sets looked in has: then nothing plays. → what [`play`] returns:
/// at Accelerated, what `command.batch` returns; at Step by Step and Pause For, {results, pending}
pub fn play_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let name = p.get("action").and_then(Value::as_str).ok_or("no such action")?;
    let in_set = match p.get("set") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or("no such action set")?),
    };
    let found: Vec<(usize, usize)> = app
        .ui
        .action_sets
        .iter()
        .enumerate()
        .filter(|(_, s)| in_set.is_none_or(|n| s.name == n))
        .flat_map(|(si, s)| s.actions.iter().enumerate().filter(|(_, a)| a.name == name).map(move |(ai, _)| (si, ai)))
        .collect();
    match found[..] {
        [] => Err("no such action".into()),
        [(s, a)] => play(app, s, a),
        _ => Err(format!("more than one action is named “{}”", name)),
    }
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
