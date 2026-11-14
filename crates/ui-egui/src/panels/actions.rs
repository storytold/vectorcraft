//! The Actions panel: record commands from the engine journal, play them back as one undo step.
//! Every recorded step is an ordinary command invocation, so actions are also plain JSON that
//! agents can author and replay (`command.batch`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use super::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, dim_label};
use crate::{VectorcraftApp, icons};

mod files;
pub(crate) use files::can_load;
pub use files::{ACTIONS_EXT, LOAD_SET, SAVE_SET, load_set, save_set};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    /// Tells the action apart from every other action in this run, also one with the same name:
    /// the recording New Action starts fills the action with this id wherever it is. It is not
    /// saved: each action gets a new id when it is made, copied or loaded.
    #[serde(skip, default = "next_action_id")]
    pub id: u64,
    pub name: String,
    pub steps: Vec<(String, Value)>,
    /// Step indexes omitted from playback. A sidecar keeps existing recorded command pairs and
    /// their params unchanged; newly recorded steps are included by default.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub excluded_steps: BTreeSet<usize>,
    /// The function key that plays the action when shortcuts are read, if one is assigned. A
    /// saved value that is not a function key loads as none.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_key")]
    pub key: Option<FunctionKey>,
    /// The action's Button Mode color, one of [`COLORS`], if one is assigned: the color of its
    /// button while the panel is in Button Mode ([`button_colors`]). A saved value that is not one
    /// of them loads as none.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_color")]
    pub color: Option<String>,
}

impl Action {
    /// An action with a new [`Action::id`], no function key and no color.
    pub fn new(name: impl Into<String>, steps: Vec<(String, Value)>) -> Self {
        Self { id: next_action_id(), name: name.into(), steps, excluded_steps: BTreeSet::new(), key: None, color: None }
    }

    /// A copy of the action with a new [`Action::id`] and no function key, so the key still plays
    /// only the original.
    pub fn copy(&self) -> Self {
        Self { id: next_action_id(), key: None, ..self.clone() }
    }

    fn all_steps_included(&self) -> bool {
        (0..self.steps.len()).all(|i| !self.excluded_steps.contains(&i))
    }

    /// Puts `step` at index `at`; the steps from there on keep whether they are excluded. An
    /// exclusion past the last step, which saved preferences can hold, goes.
    fn insert_step(&mut self, at: usize, step: (String, Value)) {
        let len = self.steps.len();
        self.steps.insert(at, step);
        self.excluded_steps = self.excluded_steps.iter().filter(|&&i| i < len).map(|&i| if i >= at { i + 1 } else { i }).collect();
    }

    /// Removes the step at index `at`; the steps after it keep whether they are excluded. An
    /// exclusion past the last step, which saved preferences can hold, goes.
    fn remove_step(&mut self, at: usize) {
        let len = self.steps.len();
        self.steps.remove(at);
        self.excluded_steps = self.excluded_steps.iter().filter(|&&i| i < len && i != at).map(|&i| if i > at { i - 1 } else { i }).collect();
    }

    /// The included steps as `command.batch` takes them. A recorded `command.batch` step (an action
    /// played, or a panel operation that runs as one batch, while recording) gives its own steps,
    /// since batches do not nest.
    pub(crate) fn included_commands(&self) -> Vec<Value> {
        self.steps
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.excluded_steps.contains(i))
            .flat_map(|(_, (command, params))| match params.get("commands").and_then(Value::as_array) {
                Some(steps) if command == "command.batch" => steps.clone(),
                _ => vec![json!({"command": command, "params": params})],
            })
            .collect()
    }
}

/// An action id no other action in this run has.
fn next_action_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The highest function key an action can play with: F1 to F15.
pub const FUNCTION_KEYS: u8 = 15;

/// A function key that plays an action: F1 to F15, alone or with Ctrl (Command on macOS), Shift,
/// or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionKey {
    /// 1 to [`FUNCTION_KEYS`]: F1 to F15.
    pub key: u8,
    /// Held with Ctrl, Command on macOS.
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub shift: bool,
}

impl FunctionKey {
    /// The key as shortcut text (`Cmd+Shift+F5`).
    fn shortcut(self) -> String {
        format!("{}{}F{}", if self.cmd { "Cmd+" } else { "" }, if self.shift { "Shift+" } else { "" }, self.key)
    }

    /// The chord the key is pressed as; none when `key` is not 1 to [`FUNCTION_KEYS`].
    pub fn chord(self) -> Option<craft_keymap::Chord> {
        (1..=FUNCTION_KEYS).contains(&self.key).then(|| crate::keymap::vc_chord(&self.shortcut())).flatten()
    }

    /// The key as the menus show shortcuts on this platform (`Ctrl+Shift+F5`, `⇧⌘F5`).
    pub fn text(self) -> String {
        crate::menus::pretty_shortcut(&self.shortcut())
    }

    /// The function key that params `p` give: `key` (1 to 15, F1 to F15; 0 or null for none), `cmd`
    /// and `shift`. `Ok(None)` for no key; an error for a `key` that is not one of them.
    pub fn from_params(p: &Value) -> Result<Option<Self>, String> {
        match p.get("key") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_u64() {
                Some(0) => Ok(None),
                Some(n) if n <= u64::from(FUNCTION_KEYS) => {
                    let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
                    Ok(Some(Self { key: n as u8, cmd: flag("cmd"), shift: flag("shift") }))
                }
                _ => Err("no such function key".into()),
            },
        }
    }
}

/// The Button Mode colors an action can have, as (value, label).
pub const COLORS: [(&str, &str); 5] = [("red", "Red"), ("yellow", "Yellow"), ("green", "Green"), ("blue", "Blue"), ("violet", "Violet")];

/// The fill of a Button Mode button in color `color`, one of [`COLORS`]: VectorCraft's own shades.
fn button_fill(color: &str) -> Option<egui::Color32> {
    let rgb = egui::Color32::from_rgb;
    Some(match color {
        "red" => rgb(0xd4, 0x3f, 0x3f),
        "yellow" => rgb(0xf0, 0xc8, 0x3c),
        "green" => rgb(0x3f, 0xa8, 0x5a),
        "blue" => rgb(0x3d, 0x7b, 0xd6),
        "violet" => rgb(0x8e, 0x5a, 0xd0),
        _ => return None,
    })
}

/// The relative luminance of `c`, as WCAG 2 defines it.
fn luminance(c: egui::Color32) -> f32 {
    let channel = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// The (fill, text) colors of the Button Mode button of an action with color `color`: a colored
/// fill with black or white text, whichever contrasts more with it, or the theme's ordinary button
/// for an action without a color.
fn button_colors(color: Option<&str>, t: &Tokens) -> (egui::Color32, egui::Color32) {
    match color.and_then(button_fill) {
        // Black text contrasts more than white above this luminance.
        Some(fill) if luminance(fill) > 0.179 => (fill, egui::Color32::BLACK),
        Some(fill) => (fill, egui::Color32::WHITE),
        None => (t.button, t.text_strong),
    }
}

/// Serde reader for [`Action::key`]: a value that is not a function key is none.
fn deserialize_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<FunctionKey>, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value::<FunctionKey>(v).ok().filter(|k| k.chord().is_some()))
}

/// Serde reader for [`Action::color`]: a value that is not one of [`COLORS`] is none.
fn deserialize_color<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(v.as_str().filter(|c| COLORS.iter().any(|(id, _)| id == c)).map(str::to_string))
}

/// The color that params `p` give: `color`, one of [`COLORS`], or null or `""` for none.
fn color_param(p: &Value) -> Result<Option<String>, String> {
    match p.get("color") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_str() {
            Some("") => Ok(None),
            Some(c) if COLORS.iter().any(|(id, _)| *id == c) => Ok(Some(c.to_string())),
            _ => Err("no such color".into()),
        },
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionSet {
    /// Tells the set apart from every other set in this run, even one with the same name. It is
    /// not saved: each set gets a new id when it is made or loaded.
    #[serde(skip, default = "next_set_id")]
    pub id: u64,
    pub name: String,
    pub actions: Vec<Action>,
}

impl ActionSet {
    /// A set with a new [`ActionSet::id`].
    pub fn new(name: impl Into<String>, actions: Vec<Action>) -> Self {
        Self { id: next_set_id(), name: name.into(), actions }
    }
}

/// A set id no other set in this run has.
fn next_set_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Serde reader for the saved current set ([`crate::UiState::action_set`]): a whole number is the
/// set's index, and any other value is no current set. An index past the last set is ignored where
/// the panel reads it.
pub fn deserialize_current_set<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(v.as_u64().and_then(|i| usize::try_from(i).ok()))
}

/// Built-in actions (our own, handy defaults).
pub fn default_sets() -> Vec<ActionSet> {
    let step = |c: &str, p: Value| (c.to_string(), p);
    vec![ActionSet::new(
        "Default Actions",
        vec![
            Action::new("Duplicate and Offset 10 pt", vec![step("object.move", json!({"dx": 10, "dy": 10, "copy": true}))]),
            Action::new(
                "Unite and Simplify",
                vec![step("object.pathfinder.unite", json!({})), step("object.path.simplify", json!({"tolerance": 0.5}))],
            ),
            Action::new("Center on Artboard", vec![step("object.align", json!({"horizontal": "center", "vertical": "center", "to": "artboard"}))]),
            Action::new("Outline Text", vec![step("type.createOutlines", json!({}))]),
            Action::new("Rotate 90° CW", vec![step("object.rotate", json!({"angle": -90}))]),
            Action::new("Reflect Horizontal", vec![step("object.reflect", json!({"axis": "vertical"}))]),
        ],
    )]
}

/// Plays an action at the speed [`UiState::action_playback`](crate::UiState::action_playback) sets.
/// Accelerated runs every included step at once as a single undoable batch → what `command.batch`
/// returns, {results}. Step by Step and Pause For run the first step now, each as its own undo step,
/// and leave the rest to [`tick_playback`] → {results: [the first step's result], pending: the steps
/// left}. While a paced playback is in progress, another action does not play.
///
/// An action with an included stop ([`STOP_STEP`]) plays up to the stop and waits there at any speed
/// ([`continue_playback`]). At Accelerated, the steps between two stops are one undo step. → {results:
/// [the results of the steps run], pending: the steps left, stops included, atStop: true when it waits}
///
/// A View menu step ([`is_ui_step`]) runs on its own at every speed, in its place among the steps, and
/// adds no undo step ([`run_steps`]). At Accelerated the steps before it and the steps after it are
/// then one undo step each.
pub fn play(app: &mut VectorcraftApp, set: usize, idx: usize) -> Result<Value, String> {
    let Some(a) = app.ui.action_sets.get(set).and_then(|s| s.actions.get(idx)).cloned() else { return Err("no such action".into()) };
    let commands = a.included_commands();
    if commands.is_empty() {
        return Err("the action has no included steps".into());
    }
    if app.ui.action_playing.is_some() {
        return Err("an action is already playing".into());
    }
    let options = app.ui.action_playback;
    if has_stop(&commands) {
        return play_with_stops(app, a.name, commands, options);
    }
    if options.speed == PlaybackSpeed::Accelerated {
        return Ok(json!({"results": run_steps(app, &a.name, commands)?}));
    }
    let mut steps: std::collections::VecDeque<Value> = commands.into();
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
fn play_step(app: &mut VectorcraftApp, label: &str, step: Value) -> Result<Value, String> {
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
/// its error: the batch it is in changes nothing, and the steps before that batch stay done.
fn run_steps(app: &mut VectorcraftApp, label: &str, steps: Vec<Value>) -> Result<Vec<Value>, String> {
    fn batch(app: &mut VectorcraftApp, label: &str, run: Vec<Value>, results: &mut Vec<Value>) -> Result<(), String> {
        if run.is_empty() {
            return Ok(());
        }
        let r = app.replay("command.batch", json!({"label": label, "commands": run}))?;
        results.extend(r.get("results").and_then(Value::as_array).cloned().unwrap_or_default());
        Ok(())
    }
    let mut results = Vec::with_capacity(steps.len());
    let mut run = vec![];
    for step in steps {
        if is_ui_step(&step) {
            batch(app, label, std::mem::take(&mut run), &mut results)?;
            results.push(play_ui_step(app, &step)?);
        } else {
            run.push(step);
        }
    }
    batch(app, label, run, &mut results)?;
    Ok(results)
}

/// Runs a View menu step as its menu item runs it, which adds no undo step → its result. While an
/// action records, the step goes into the recording too, as a stop does, unless it does not fit in
/// the action ([`recording_room`]): then the status bar shows the reason.
fn play_ui_step(app: &mut VectorcraftApp, step: &Value) -> Result<Value, String> {
    let command = step.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = step.get("params").filter(|p| !p.is_null()).cloned().unwrap_or_else(|| json!({}));
    let r = app.replay(&command, params.clone())?;
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
fn play_with_stops(app: &mut VectorcraftApp, label: String, commands: Vec<Value>, options: PlaybackOptions) -> Result<Value, String> {
    let mut steps: std::collections::VecDeque<Value> = commands.into();
    let run = next_run(&mut steps, options.speed);
    let ran = !run.is_empty();
    let results = if ran { run_steps(app, &label, run)? } else { vec![] };
    app.ui.action_playing = Some(Playing { label, steps, options, next_at: None, hold: false, doc: None, played_in: None, stop: None });
    after_run(app, ran);
    Ok(playback_result(app, results))
}

/// Takes the steps that run next as one undo step off the front of `steps`: at Accelerated all of
/// them up to the next stop, at the other speeds one. None when a stop comes next.
fn next_run(steps: &mut std::collections::VecDeque<Value>, speed: PlaybackSpeed) -> Vec<Value> {
    let mut run = vec![];
    while steps.front().is_some_and(|s| !is_stop(s)) {
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
    if (!ran || playing.options.speed == PlaybackSpeed::Accelerated) && playing.steps.front().is_some_and(is_stop) {
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
struct AtStop {
    message: String,
    allow_continue: bool,
    /// Whether the message box was shown. It waits for a dialog already open to close.
    shown: bool,
}

/// Takes the stop at the front of the playback's steps and waits there. A recording in progress
/// records the stop too, unless the stop does not fit in its action ([`recording_room`]): then the
/// status bar shows the reason.
fn hold_at_stop(app: &mut VectorcraftApp) {
    let Some(playing) = app.ui.action_playing.as_mut() else { return };
    let Some(step) = playing.steps.pop_front() else { return };
    let params = step.get("params").cloned().unwrap_or(Value::Null);
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
fn close_stop_message(app: &mut VectorcraftApp) {
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
    label: String,
    steps: std::collections::VecDeque<Value>,
    /// The options the action started playing with.
    options: PlaybackOptions,
    /// With Pause For, when the next step runs (seconds on the app's clock); none until the frame
    /// after each step, which starts the pause before the next one.
    next_at: Option<f64>,
    /// With Step by Step, set after each step until the window shows a frame drawn after it
    /// ([`playback_drawn`]): no step runs while it is set, so the window shows each step's result
    /// before the next one runs, also when Play in the panel ran after the canvas drew and while the
    /// window is minimized or covered.
    hold: bool,
    /// The [`uid`](vectorcraft_engine::DocState::uid) of the document that was active after the
    /// last step played, if any.
    doc: Option<u64>,
    /// The host iteration in which Play ran the first step, when that iteration's [`tick`] had
    /// already begun (a control request, or the UI pass after it): the pause before the second step
    /// starts in a later iteration.
    ///
    /// [`tick`]: VectorcraftApp::tick
    played_in: Option<u64>,
    /// The stop the playback waits at, if any: no step runs until [`continue_playback`] or
    /// [`stop`].
    stop: Option<AtStop>,
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
    if playing.steps.front().is_some_and(is_stop) {
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

/// The command behind the Actions panel's set, action and step checkmarks.
pub const INCLUDE_STEPS: &str = "ui.action.stepsIncluded";

/// `{set, action?, step?, included, others?}` includes or excludes all steps of a set, all steps
/// of one action, or one step. With `step` and `others: true`, leaves that step as it was and
/// applies `included` to the other steps of its action. Invalid indexes change nothing. So does an
/// exclusion that would give the set a file larger than [`files::MAX_SET_FILE`], the most Load
/// Actions… reads, counting the steps that the recording in progress adds to the set; it is an
/// error. A change that leaves the file as large or makes it smaller is never refused.
pub fn include_steps(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |key: &str| p.get(key).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    let set = index("set").ok_or("no such action set")?;
    let included = p.get("included").and_then(Value::as_bool).ok_or("included must be true or false")?;
    let others = match p.get("others") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err("others must be true or false".into()),
    };
    let action = if p.get("action").is_some() { Some(index("action").ok_or("no such action")?) } else { None };
    let step = if p.get("step").is_some() { Some(index("step").ok_or("no such action step")?) } else { None };
    if (step.is_some() && action.is_none()) || (others && step.is_none()) {
        return Err("no such action step".into());
    }
    let set_ref = app.ui.action_sets.get(set).ok_or("no such action set")?;
    let actions = match action {
        None => 0..set_ref.actions.len(),
        Some(ai) if ai < set_ref.actions.len() => ai..ai + 1,
        Some(_) => return Err("no such action".into()),
    };
    if let (Some(ai), Some(si)) = (action, step)
        && set_ref.actions.get(ai).is_none_or(|a| si >= a.steps.len())
    {
        return Err("no such action step".into());
    }
    // The exclusions each action gets, worked out before any of them changes.
    let every = |a: &Action| if included { BTreeSet::new() } else { (0..a.steps.len()).collect() };
    let changes: Vec<(usize, BTreeSet<usize>)> = actions
        .map(|ai| {
            let a = &set_ref.actions[ai];
            let mut excluded = match step {
                Some(_) if !others => a.excluded_steps.clone(),
                _ => every(a),
            };
            if let Some(si) = step {
                let keep_excluded = if others { a.excluded_steps.contains(&si) } else { !included };
                if keep_excluded {
                    excluded.insert(si);
                } else {
                    excluded.remove(&si);
                }
            }
            (ai, excluded)
        })
        .collect();
    // Each excluded step takes [`files::EXCLUDED_BYTES`] in the file; only exclusions past the
    // ones the set has now can make it too large.
    let (mut now, mut then) = (0, 0);
    for (ai, excluded) in &changes {
        let a = &set_ref.actions[*ai];
        now += a.excluded_steps.range(..a.steps.len()).count();
        then += excluded.range(..a.steps.len()).count();
    }
    if then > now && set_file_len(app, set) + (then - now) * files::EXCLUDED_BYTES > files::MAX_SET_FILE {
        return Err("the action set would be larger than 8 MiB".into());
    }
    let set_ref = app.ui.action_sets.get_mut(set).ok_or("no such action set")?;
    for (ai, excluded) in changes {
        if let Some(a) = set_ref.actions.get_mut(ai) {
            a.excluded_steps = excluded;
        }
    }
    Ok(json!({"set": set, "action": action, "step": step, "included": included}))
}

/// The command that renames an action ([`rename_action`]).
pub const RENAME_ACTION: &str = "ui.action.rename";

/// `ui.action.rename {set, action, name}`: renames the action at index `action` of set `set` (0 is
/// the top set) to `name` with spaces trimmed from both ends, and keeps its steps, function key and
/// color. An index past the last set or action, a name with nothing but spaces, or a name that would
/// give the set a file that Load Actions… refuses ([`edit_keeps_set_loadable`]) is an error, and
/// then nothing is renamed. → {name}
pub fn rename_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let (set, at) =
        index("set").zip(index("action")).filter(|&(s, a)| app.ui.action_sets.get(s).is_some_and(|s| a < s.actions.len())).ok_or("no such action")?;
    if name.is_empty() {
        return Err("name is empty".into());
    }
    let mut edited = app.ui.action_sets[set].clone();
    edited.actions[at].name = name.to_string();
    edit_keeps_set_loadable(app, Some(set), &edited)?;
    app.ui.action_sets[set] = edited;
    Ok(json!({ "name": name }))
}

/// The command that makes an action and records into it ([`new_action`]).
pub const NEW_ACTION: &str = "ui.action.new";

/// `ui.action.new {set, name, key?, cmd?, shift?, color?}`: adds an action named `name`
/// (spaces trimmed from both ends) after the last action of set `set` (0 is the top set) and starts
/// recording into it; Stop Playing/Recording ends the recording. `key` (1 to 15, F1 to F15; 0 or
/// null for none) with `cmd` (Ctrl, Command on macOS) and `shift` is the function key that plays
/// it, and `color` (one of [`COLORS`]; null or `""` for none) its Button Mode color. The action's
/// function key takes priority over command shortcuts. The set becomes current and the only selected set, and the new action is selected. While an
/// action is recording, and for an index past the last set, an empty name, a key or color that is
/// not one of them, or an action that would give the set a file that Load Actions… refuses
/// ([`edit_keeps_set_loadable`]: 1,000 actions, 8 MiB), nothing is added. The New Action dialog's Record runs this. → {set, action}
pub fn new_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    let len = app.ui.action_sets.len();
    let set = p.get("set").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).filter(|&s| s < len).ok_or("no such action set")?;
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    let key = FunctionKey::from_params(p)?;
    let color = color_param(p)?;
    if let Some(message) = key.and_then(key_error) {
        return Err(message);
    }
    let action = Action { key, color, ..Action::new(name.clone(), vec![]) };
    let id = action.id;
    let Some(mut edited) = app.ui.action_sets.get(set).cloned() else { return Err("no such action set".into()) };
    edited.actions.push(action);
    edit_keeps_set_loadable(app, Some(set), &edited)?;
    let index = edited.actions.len() - 1;
    app.ui.action_sets[set] = edited;
    start_recording(app, set, name, Some(id));
    app.ui.action_set = Some(set);
    app.ui.action_sets_selected = vec![set];
    if let Some(ctx) = app.ui_ctx.clone() {
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((set, index)));
    }
    Ok(json!({ "set": set, "action": index }))
}

/// The command that changes an action's name, function key and color ([`action_options`]).
pub const ACTION_OPTIONS: &str = "ui.action.options";

/// `ui.action.options {set, action, name?, key?, cmd?, shift?, color?}`: changes the
/// action at index `action` of set `set` (0 is the top set) and keeps its steps. `name` renames it
/// (spaces trimmed from both ends), `key` (1 to 15, F1 to F15; 0 or null for none) with `cmd`
/// (Ctrl, Command on macOS) and `shift` sets the function key that plays it, and `color` (one of
/// [`COLORS`]; null or `""` for none) its Button Mode color; a param left out keeps what the action
/// has, and `cmd` or `shift` alone changes that modifier of the action's key. The action's function
/// key takes priority over command shortcuts. An index past the last set or action, an empty name,
/// a key or color that is not one of them, or a change that would give the set a file that Load
/// Actions… refuses ([`edit_keeps_set_loadable`]) is an error, and then nothing changes. The panel
/// menu's Action Options… runs this for the selected action. → {name, key, color}
pub fn action_options(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let (set, at) = index("set").zip(index("action")).ok_or("no such action")?;
    let had = app.ui.action_sets.get(set).and_then(|s| s.actions.get(at)).map(|a| a.key).ok_or("no such action")?;
    let name = match p.get("name") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_str().map(str::trim).unwrap_or_default() {
            "" => return Err("name is empty".into()),
            n => Some(n.to_string()),
        },
    };
    // `cmd` and `shift` left out keep the modifiers of the action's key.
    let flag = |k: &str, kept: bool| p.get(k).and_then(Value::as_bool).unwrap_or(kept);
    let with_mods =
        |k: FunctionKey| FunctionKey { cmd: flag("cmd", had.is_some_and(|h| h.cmd)), shift: flag("shift", had.is_some_and(|h| h.shift)), ..k };
    let key = if p.get("key").is_some() {
        Some(FunctionKey::from_params(p)?.map(with_mods))
    } else if p.get("cmd").is_some() || p.get("shift").is_some() {
        had.map(|h| Some(with_mods(h)))
    } else {
        None
    };
    let color = if p.get("color").is_some() { Some(color_param(p)?) } else { None };
    if let Some(Some(k)) = key
        && had != Some(k)
        && let Some(message) = key_error(k)
    {
        return Err(message);
    }
    let Some(mut edited) = app.ui.action_sets.get(set).cloned() else { return Err("no such action".into()) };
    let Some(action) = edited.actions.get_mut(at) else { return Err("no such action".into()) };
    if let Some(name) = name {
        action.name = name;
    }
    if let Some(key) = key {
        action.key = key;
    }
    if let Some(color) = color {
        action.color = color;
    }
    let result = json!({ "name": action.name, "key": action.key, "color": action.color });
    edit_keeps_set_loadable(app, Some(set), &edited)?;
    app.ui.action_sets[set] = edited;
    Ok(result)
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

/// The command behind the panel menu's Button Mode ([`button_mode`]).
pub const BUTTON_MODE: &str = "ui.action.buttonMode";

/// `ui.action.buttonMode {enabled?}`: with `enabled: true` the Actions panel shows every action of
/// every set as a button in its color ([`Action::color`]) that plays the action when clicked, and
/// with `false` it shows its list of sets, actions and steps again; left out or null, it switches
/// to the other view. The view is kept with the UI preferences. Turning Button Mode on while an
/// action records is an error, and so is an `enabled` that is not true, false or null; then the
/// view does not change. → {enabled}
pub fn button_mode(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let enabled = crate::menus::opt_bool(p, "enabled")?.unwrap_or(!app.ui.action_button_mode);
    if enabled && !app.ui.action_button_mode && app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    app.ui.action_button_mode = enabled;
    Ok(json!({ "enabled": enabled }))
}

/// The command that ends a recording ([`stop`]).
pub const STOP_RECORDING: &str = "ui.action.stop";

/// The command that begins a plain recording ([`record`]).
pub const START_RECORDING: &str = "ui.action.record";

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

/// The command that inserts a stop ([`insert_stop`]).
pub const INSERT_STOP: &str = "ui.action.insertStop";

/// The panel state that holds the selected step as (the action's [`Action::id`], the step's index).
const SELECTED_STEP: &str = "actions-selected-step";

/// The panel state that holds the step being dragged as a [`DraggedStep`].
const DRAGGED_STEP: &str = "actions-dragged-step";

/// A step being dragged: the action's [`Action::id`], the step's index, and the action's steps and
/// excluded steps as they were when the drag started.
type DraggedStep = (u64, usize, std::sync::Arc<(Vec<(String, Value)>, BTreeSet<usize>)>);

/// `ui.action.insertStop {set?, action?, step?, message?, allowContinue?}`: inserts a stop. With `set`
/// and `action`, it goes into that action, after step `step`, else after its last step. Without
/// them, it goes into the recording in progress, after the steps recorded so far. `message` is
/// the text playback shows (default: none), and `allowContinue` offers Continue in the message box
/// (default: false). An index past the last set, action or step, a param of another type, params
/// that are not an object, or no action and no recording is an error, and then nothing changes. So
/// is a stop that would give the action more than 10,000 steps, or its set a file larger than 8 MiB,
/// the most Load Actions… reads. Record Stop's OK runs this. A stop in an action becomes the selected
/// step, and its action and set become the selected ones. → {set, action, step}, or {recording: true}
pub fn insert_stop(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if !p.is_object() {
        return Err("`params` must be an object".into());
    }
    let (message, allow_continue) = stop_params(p)?;
    insert(app, p, (STOP_STEP.to_string(), json!({"message": message, "allowContinue": allow_continue})))
}

/// The command that inserts a menu item's command as a step ([`insert_menu_item`]).
pub const INSERT_MENU_ITEM: &str = "ui.action.insertMenuItem";

/// `ui.action.insertMenuItem {set?, action?, step?, command, params?}`: inserts the command of a
/// menu item as a step, without running it. `command` and `params` (default: `{}`) are those of a
/// menu item that can be a step ([`crate::menus::insertable_items`]); the step plays as the item
/// runs. An Object › Align item's step holds the params that clicking the item sends now
/// ([`crate::panels::align::menu_align_params`]): with whole objects selected, the `to` of the key
/// object, the artboard or the selection, so a later Align To choice does not change the step, and a
/// step that aligns to the key object fails in playback when there is none then. A View menu item that the app runs itself
/// ([`crate::menus::plays_in_app`]) plays on its own, with no undo step ([`play`]). The place is as
/// [`insert_stop`] takes it, and the step is selected as a stop is. A command and params that no such
/// item has, a param of another type, params that are not an object, or a place [`insert_stop`]
/// refuses is an error, and then nothing changes. Insert Menu Item's OK runs this. → {set, action,
/// step}, or {recording: true}
pub fn insert_menu_item(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if !p.is_object() {
        return Err("`params` must be an object".into());
    }
    let command = p.get("command").and_then(Value::as_str).ok_or("command must be text")?;
    let params = match p.get("params") {
        None => json!({}),
        Some(v) if v.is_object() => v.clone(),
        Some(_) => return Err("`params` must be an object".into()),
    };
    let items = crate::menus::insertable_items(app.session.prefs.font_names_in_english);
    if !items.iter().any(|item| item.command == command && item.params == params) {
        return Err("no menu item that can be inserted runs this command".into());
    }
    let params = if command == "object.align" { super::align::menu_align_params(app, params) } else { params };
    insert(app, p, (command.to_string(), params))
}

/// The command that inserts a Select Path step ([`insert_select_path`]).
pub const INSERT_SELECT_PATH: &str = "ui.action.insertSelectPath";

/// The command id of a Select Path step in [`Action::steps`], with {id, fingerprint} as its params:
/// playback selects that path of the active document when it is still the path the step stored.
pub const SELECT_PATH_STEP: &str = "select.storedPath";

/// `ui.action.insertSelectPath {set?, action?, step?}`: inserts a step that selects the path
/// selected now, without changing the selection or the art. The selection must be one whole path or
/// compound path that can be selected ([`vectorcraft_engine::storable_selection`]). The step stores
/// the path's id and a fingerprint of it ([`vectorcraft_engine::path_fingerprint`]), and playback
/// selects that path only while it is still there unchanged. The place is as [`insert_stop`] takes
/// it, and the step is selected as a stop is. No document, a selection that is not such a path, a
/// param of another type, params that are not an object, or a place [`insert_stop`] refuses is an
/// error, and then nothing changes. The panel menu's Insert Select Path runs this.
/// → {set, action, step}, or {recording: true}
pub fn insert_select_path(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if !p.is_object() {
        return Err("`params` must be an object".into());
    }
    let st = app.session.active().ok_or("no document open")?;
    let params = vectorcraft_engine::stored_path_step(st)?;
    insert(app, p, (SELECT_PATH_STEP.to_string(), params))
}

/// The command a Select Object step runs, with {note, caseSensitive, wholeWord} as its params: it
/// selects the objects whose Attributes note matches.
pub const SELECT_OBJECT_STEP: &str = "select.byNote";

/// The longest note a Select Object step searches for, in bytes, as `select.byNote` takes it.
const MAX_SELECT_NOTE: usize = 1024;

/// The note, Case Sensitive and Whole Word in a Select Object step's params, the note without its
/// leading and trailing spaces. Case Sensitive and Whole Word may be left out. No note, an empty
/// note or one longer than 1,024 bytes, a value of another type, or params that are not an object
/// is an error.
pub(crate) fn select_object_params(p: &Value) -> Result<(String, bool, bool), String> {
    if !p.is_object() {
        return Err("`params` must be an object".into());
    }
    let key = "note";
    let note = match p.get(key) {
        None | Some(Value::Null) => "",
        Some(v) => v.as_str().ok_or_else(|| format!("`{key}` must be a string"))?,
    }
    .trim();
    if note.is_empty() {
        return Err("the note is empty".into());
    }
    if note.len() > MAX_SELECT_NOTE {
        return Err("the note is longer than 1,024 bytes".into());
    }
    let flag = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| format!("`{key}` must be true or false")),
    };
    Ok((note.to_string(), flag("caseSensitive")?, flag("wholeWord")?))
}

/// The command that inserts a Select Object step ([`insert_select_object`]).
pub const INSERT_SELECT_OBJECT: &str = "ui.action.insertSelectObject";

/// `ui.action.insertSelectObject {set?, action?, step?, note, caseSensitive?, wholeWord?}`: inserts
/// a Select Object step, without running it. Playback selects the objects whose Attributes note
/// matches `note` ([`SELECT_OBJECT_STEP`]); a playback that finds none stops there with an error.
/// The note and options are as [`select_object_params`] reads them. The place is as [`insert_stop`]
/// takes it, and the step is selected as a stop is. Bad params, or a place [`insert_stop`]
/// refuses, is an error, and then nothing changes. Set Selection's OK runs this. → {set, action,
/// step}, or {recording: true}
pub fn insert_select_object(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (note, case_sensitive, whole_word) = select_object_params(p)?;
    insert(app, p, (SELECT_OBJECT_STEP.to_string(), json!({"note": note, "caseSensitive": case_sensitive, "wholeWord": whole_word})))
}

/// The journal entry the recording in progress records from.
fn recording_start(app: &VectorcraftApp) -> Option<usize> {
    let (_, _, start) = app.ui.recording.as_ref()?;
    // A canceled undo group numbers the steps down when it drops entries made before them.
    Some(app.session.journal.held().unwrap_or(*start))
}

/// Where Stop Playing/Recording puts the steps of the recording in progress: the index of the set,
/// and of the action it records into ([`crate::UiState::recording_into`]), or None for the new
/// action Stop adds after the set's last action. None when there is no recording, no set to put
/// them in, or the action it records into was deleted, so that Stop keeps none of them.
fn recording_target(app: &VectorcraftApp) -> Option<(usize, Option<usize>)> {
    let (set, ..) = app.ui.recording.as_ref()?;
    let Some(id) = app.ui.recording_into else {
        app.ui.action_sets.get(*set)?;
        return Some((*set, None));
    };
    app.ui.action_sets.iter().enumerate().find_map(|(holder, s)| Some((holder, Some(s.actions.iter().position(|a| a.id == id)?))))
}

/// The set that Stop Playing/Recording leaves when the recording in progress has recorded `steps`,
/// and the index of the action they go into ([`recording_target`]).
fn set_with_recorded(app: &VectorcraftApp, steps: Vec<(String, Value)>) -> Option<(ActionSet, usize)> {
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

/// The length of the action set file of set `set`, with the steps that the recording in progress
/// adds to it ([`files::write_set`]).
fn set_file_len(app: &VectorcraftApp, set: usize) -> usize {
    if recording_target(app).is_some_and(|(holder, _)| holder == set)
        && let Some((s, _)) = recording_start(app).and_then(|start| set_with_recorded(app, app.session.journal_for_action(start)))
    {
        return files::write_set(&s).len();
    }
    app.ui.action_sets.get(set).map_or(0, |s| files::write_set(s).len())
}

/// The most that Load Actions… reads from an action set file ([`files::read_set`]), each with the
/// error an edit past it gives: the steps of one action, the actions of a set and the bytes of the
/// file.
const SET_LIMITS: [(usize, &str); 3] = [
    (files::MAX_ACTION_STEPS, "the action already has 10,000 steps"),
    (files::MAX_SET_ACTIONS, "the action set file holds more than 1,000 actions"),
    (files::MAX_SET_FILE, "the action set would be larger than 8 MiB"),
];

/// The sizes of `set` that [`SET_LIMITS`] bound, as the set at index `at` (None for a new set):
/// the most steps one action has, the actions and the length of its file. When the recording in
/// progress records into the set at `at` ([`recording_target`]), its steps count too.
fn set_sizes(app: &VectorcraftApp, at: Option<usize>, set: &ActionSet) -> [usize; 3] {
    let recorded = recording_target(app)
        .filter(|&(holder, _)| Some(holder) == at)
        .and_then(|_| Some((recording_start(app)?, app.ui.recording.as_ref()?.1.clone())));
    let with;
    let set = match recorded {
        Some((start, name)) => {
            let mut s = set.clone();
            let steps = app.session.journal_for_action(start);
            match app.ui.recording_into.and_then(|id| s.actions.iter().position(|a| a.id == id)) {
                Some(i) => put_recorded(&mut s.actions[i], steps, app.ui.recording_at),
                None => s.actions.push(Action::new(name, steps)),
            }
            with = s;
            &with
        }
        None => set,
    };
    [set.actions.iter().map(|a| a.steps.len()).max().unwrap_or(0), set.actions.len(), files::write_set(set).len()]
}

/// Err when `edited`, in place of the set at index `at` (None: added as a new set), would give an
/// action set file that Load Actions… refuses ([`SET_LIMITS`]), counting the steps that the
/// recording in progress adds to it. An edit that leaves the set no further past a limit than it
/// is now is not refused.
pub(crate) fn edit_keeps_set_loadable(app: &VectorcraftApp, at: Option<usize>, edited: &ActionSet) -> Result<(), String> {
    let after = set_sizes(app, at, edited);
    if after.iter().zip(SET_LIMITS).all(|(&size, (limit, _))| size <= limit) {
        return Ok(());
    }
    let before = at.and_then(|i| app.ui.action_sets.get(i)).map_or([0; 3], |s| set_sizes(app, at, s));
    for ((size, was), (limit, error)) in after.into_iter().zip(before).zip(SET_LIMITS) {
        if size > limit && size > was {
            return Err(error.into());
        }
    }
    Ok(())
}

/// The number of steps the action that the recording in progress records into has of its own: 0
/// for the new action Stop Playing/Recording adds.
fn recording_target_steps(app: &VectorcraftApp) -> usize {
    recording_target(app).and_then(|(holder, at)| app.ui.action_sets.get(holder)?.actions.get(at?)).map_or(0, |a| a.steps.len())
}

/// The bytes that the journal entries from `start` on add to an action set file after a step
/// ([`files::step_bytes`]), at least as many as the steps recorded from them add. Only the entries
/// journaled since the last count are counted, unless the journal rewrote entries since
/// ([`crate::UiState::recording_tail`]).
fn recorded_bytes(app: &mut VectorcraftApp, start: usize) -> usize {
    let journal = &app.session.journal;
    let (rewrites, len) = (journal.rewrites(), journal.len());
    let (from, counted) = match app.ui.recording_tail {
        Some((r, s, counted_len, bytes)) if r == rewrites && s == start && counted_len <= len => (counted_len, bytes),
        _ => (start, 0),
    };
    let bytes = counted + journal.since(from).iter().map(|(c, p)| files::step_bytes(c, p)).sum::<usize>();
    app.ui.recording_tail = Some((rewrites, start, len, bytes));
    bytes
}

/// Err when adding the step `(command, params)` to the recording in progress would give the action
/// that Stop Playing/Recording keeps more than [`files::MAX_ACTION_STEPS`] steps, or its set a file
/// larger than [`files::MAX_SET_FILE`], the most Load Actions… reads. The file without the steps
/// recorded so far is measured once ([`recording_base`]), and the recorded steps add
/// [`recorded_bytes`] to it. Only when that sum is past the limit is the whole file written out to
/// measure it.
fn recording_room(app: &mut VectorcraftApp, command: &str, params: &Value) -> Result<(), String> {
    let Some(start) = recording_start(app) else { return Err("no such action".into()) };
    if recording_target(app).is_none() {
        return Ok(());
    }
    let stored = recording_target_steps(app);
    if stored + app.session.journal.since(start).len() >= files::MAX_ACTION_STEPS {
        return Err("the action already has 10,000 steps".into());
    }
    let first = if stored == 0 { files::FIRST_STEP_BYTES } else { 0 };
    let added = recorded_bytes(app, start) + files::step_bytes(command, params) + first;
    if recording_base(app) + added <= files::MAX_SET_FILE {
        return Ok(());
    }
    let mut steps = app.session.journal_for_action(start);
    steps.push((command.to_string(), params.clone()));
    if set_with_recorded(app, steps).is_some_and(|(s, _)| files::write_set(&s).len() > files::MAX_SET_FILE) {
        return Err("the action set would be larger than 8 MiB".into());
    }
    Ok(())
}

/// The length of the action set file that Stop Playing/Recording would leave without the steps
/// recorded so far ([`crate::UiState::recording_base`]).
fn recording_base(app: &mut VectorcraftApp) -> usize {
    if let Some(base) = app.ui.recording_base {
        return base;
    }
    let base = set_with_recorded(app, vec![]).map_or(0, |(s, _)| files::write_set(&s).len());
    app.ui.recording_base = Some(base);
    base
}

/// Err when the steps recorded so far no longer fit in the action that Stop Playing/Recording gives
/// them ([`steps_that_fit`]): a command added values to its step after [`recording_room`] measured
/// it. The steps are written out to measure them only when [`recorded_bytes`] puts them past the
/// limit.
fn recorded_steps_fit(app: &mut VectorcraftApp) -> Result<(), String> {
    let Some(start) = recording_start(app) else { return Ok(()) };
    let stored = recording_target_steps(app);
    let recorded = app.session.journal.since(start).len();
    let first = if stored == 0 && recorded > 0 { files::FIRST_STEP_BYTES } else { 0 };
    if stored + recorded <= files::MAX_ACTION_STEPS && recording_base(app) + recorded_bytes(app, start) + first <= files::MAX_SET_FILE {
        return Ok(());
    }
    let steps = app.session.journal_for_action(start);
    match steps_that_fit(app, &steps) {
        (_, Some(limit)) => Err(limit.to_string()),
        (_, None) => Ok(()),
    }
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
struct Recorded {
    /// The set's action that the recording records into; None for the new action that the
    /// recording adds.
    into: Option<usize>,
    /// The new action's name.
    name: String,
    /// The steps recorded so far.
    steps: Vec<(String, Value)>,
}

/// What Stop Playing/Recording adds to set `set`; None when the recording in progress adds nothing
/// to it.
fn recorded_into(app: &VectorcraftApp, set: usize) -> Option<Recorded> {
    let (recording_set, name, start) = app.ui.recording.as_ref()?;
    let into = match app.ui.recording_into {
        Some(id) => Some(app.ui.action_sets.get(set)?.actions.iter().position(|a| a.id == id)?),
        None if *recording_set == set => None,
        None => return None,
    };
    let steps = app.session.journal_for_action(app.session.journal.held().unwrap_or(*start));
    Some(Recorded { into, name: name.clone(), steps })
}

/// Puts `step` where params `p` say ([`insert_stop`]) → what [`insert_stop`] returns. When the
/// recording in progress adds steps to the set, the limits count them as Stop Playing/Recording
/// would leave the set.
fn insert(app: &mut VectorcraftApp, p: &Value, step: (String, Value)) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    if p.get("set").is_none() && p.get("action").is_none() && p.get("step").is_none() {
        if app.ui.recording.is_none() {
            return Err("no such action".into());
        }
        recording_room(app, &step.0, &step.1)?;
        app.session.journal.push(step);
        return Ok(json!({"recording": true}));
    }
    let (set, at) = index("set").zip(index("action")).ok_or("no such action")?;
    let recorded = recorded_into(app, set);
    let pending = match &recorded {
        Some(r) if r.into == Some(at) => r.steps.len(),
        _ => 0,
    };
    let s = app.ui.action_sets.get_mut(set).filter(|s| at < s.actions.len()).ok_or("no such action")?;
    let action = &mut s.actions[at];
    let after = match p.get("step") {
        None => action.steps.len(),
        Some(_) => index("step").filter(|&s| s < action.steps.len()).ok_or("no such action step")? + 1,
    };
    if action.steps.len() + pending >= files::MAX_ACTION_STEPS {
        return Err("the action already has 10,000 steps".into());
    }
    let excluded = action.excluded_steps.clone();
    action.insert_step(after, step);
    let id = action.id;
    // The recording's steps stay after the steps that were before them.
    let slot = match (&recorded, app.ui.recording_at) {
        (Some(r), Some(slot)) if r.into == Some(at) && after <= slot => Some(slot + 1),
        (_, slot) => slot,
    };
    let file_len = match recorded {
        None => files::write_set(s).len(),
        Some(Recorded { into, name, steps }) => {
            let mut kept = s.clone();
            match into {
                Some(into) => put_recorded(&mut kept.actions[into], steps, slot),
                None => kept.actions.push(Action::new(name, steps)),
            }
            files::write_set(&kept).len()
        }
    };
    if file_len > files::MAX_SET_FILE {
        let action = &mut s.actions[at];
        action.steps.remove(after);
        action.excluded_steps = excluded;
        return Err("the action set would be larger than 8 MiB".into());
    }
    app.ui.recording_at = slot;
    app.ui.action_set = Some(set);
    app.ui.action_sets_selected = vec![set];
    if let Some(ctx) = app.ui_ctx.clone() {
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((set, at)));
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((id, after)));
        let mut expanded: BTreeSet<u64> = pstate(&ctx, "actions-expanded");
        expanded.insert(id);
        set_pstate(&ctx, "actions-expanded", expanded);
    }
    Ok(json!({"set": set, "action": at, "step": after}))
}

/// The selected step of the action `selected`, if it has one and the panel shows its steps.
fn selected_step(app: &VectorcraftApp, ctx: &egui::Context, selected: Option<(usize, usize)>) -> Option<usize> {
    let (s, a) = selected?;
    let action = app.ui.action_sets.get(s)?.actions.get(a)?;
    if !pstate::<BTreeSet<u64>>(ctx, "actions-expanded").contains(&action.id) {
        return None;
    }
    pstate::<Option<(u64, usize)>>(ctx, SELECTED_STEP).filter(|&(id, step)| id == action.id && step < action.steps.len()).map(|(_, step)| step)
}

/// The command that deletes one step of an action ([`delete_step`]).
pub const DELETE_STEP: &str = "ui.action.deleteStep";

/// `ui.action.deleteStep {set, action, step}`: deletes step `step`, a stop or a command, of the
/// action at index `action` of set `set`. The steps after it keep whether they are excluded, and
/// no step of that action stays selected. A Record Stop, Insert Menu Item or Set Selection dialog open on that step
/// closes, and one open on a later step of the action follows it up. A missing or invalid index is an error, and
/// then nothing changes. Undo does not bring the step back. Delete Selection runs this for the
/// selected step. → {set, action, step}
pub fn delete_step(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let number = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok());
    let index = |k: &str| p.get(k).and_then(number);
    let (set, at) = index("set").zip(index("action")).ok_or("no such action")?;
    let action = app.ui.action_sets.get_mut(set).and_then(|s| s.actions.get_mut(at)).ok_or("no such action")?;
    let step = index("step").filter(|&s| s < action.steps.len()).ok_or("no such action step")?;
    action.remove_step(step);
    let id = action.id;
    // The recording's steps stay after the steps that were before them.
    if app.ui.recording_into == Some(id)
        && let Some(slot) = app.ui.recording_at.filter(|&slot| step < slot)
    {
        app.ui.recording_at = Some(slot - 1);
    }
    if let Some(ctx) = app.ui_ctx.clone()
        && pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP).is_some_and(|(selected, _)| selected == id)
    {
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, None);
    }
    let held = app
        .ui
        .dialog
        .as_ref()
        .filter(|d| holds_step(d))
        .filter(|d| d.fields.get("set").and_then(number) == Some(set) && d.fields.get("action").and_then(number) == Some(at))
        .and_then(|d| d.fields.get("step").and_then(number));
    if held == Some(step) {
        app.ui.dialog = None;
    } else if let Some(later) = held.filter(|&s| s > step)
        && let Some(d) = app.ui.dialog.as_mut()
    {
        d.fields.insert("step".into(), json!(later - 1));
    }
    Ok(json!({"set": set, "action": at, "step": step}))
}

/// The command that moves one step of an action ([`move_step`]).
pub const MOVE_STEP: &str = "ui.action.moveStep";

/// `ui.action.moveStep {set, action, step, toSet?, toAction?, before}`: moves step `step`, a stop or
/// a command, of the action at index `action` of set `set` into the action at index `toAction` of
/// set `toSet` (default: the same set and action), before the step at index `before` there, or
/// after its last step when `before` is its number of steps. `before` counts the steps as they are
/// before the move. The step keeps its params and whether it is excluded, and the other steps keep
/// theirs. A selected step stays selected: a moved selected step makes its new action the selected
/// one and its set the current one. A Record Stop, Insert Menu Item or Set Selection dialog open on
/// a step follows the step, and the steps a recording adds to either action stay after the steps
/// that were before them. A step put back just before or just after itself does not move. A missing
/// or invalid index, or a move that would give a set a file that Load Actions… refuses
/// ([`edit_keeps_set_loadable`]: 10,000 steps in an action, 8 MiB), is an error, and then nothing
/// changes. Undo does not move the step back. A drag of a step row in the panel runs this.
/// → {set, action, step, moved}: where the step is now
pub fn move_step(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let number = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok());
    let index = |k: &str| p.get(k).and_then(number);
    let (set, at) = index("set").zip(index("action")).ok_or("no such action")?;
    let source = app.ui.action_sets.get(set).and_then(|s| s.actions.get(at)).ok_or("no such action")?;
    let step = index("step").filter(|&s| s < source.steps.len()).ok_or("no such action step")?;
    let source_id = source.id;
    let to_set = match p.get("toSet") {
        None => set,
        Some(v) => number(v).filter(|&s| s < app.ui.action_sets.len()).ok_or("no such action set")?,
    };
    let to_action = match p.get("toAction") {
        None => at,
        Some(v) => number(v).ok_or("no such action")?,
    };
    let dest = app.ui.action_sets[to_set].actions.get(to_action).ok_or("no such action")?;
    let dest_id = dest.id;
    let before = index("before").filter(|&b| b <= dest.steps.len()).ok_or("no such action step")?;
    let same = (set, at) == (to_set, to_action);
    if same && (before == step || before == step + 1) {
        return Ok(json!({"set": set, "action": at, "step": step, "moved": false}));
    }
    // The step's index once it is in place, counted without it in its own action.
    let to = if same && before > step { before - 1 } else { before };
    let mut from_edit = app.ui.action_sets[set].clone();
    let moved = from_edit.actions[at].steps[step].clone();
    let excluded = from_edit.actions[at].excluded_steps.contains(&step);
    from_edit.actions[at].remove_step(step);
    let put = |a: &mut Action| {
        a.insert_step(to, moved);
        if excluded {
            a.excluded_steps.insert(to);
        }
    };
    let to_edit = if to_set == set {
        put(&mut from_edit.actions[to_action]);
        None
    } else {
        let mut edit = app.ui.action_sets[to_set].clone();
        put(&mut edit.actions[to_action]);
        Some(edit)
    };
    edit_keeps_set_loadable(app, Some(set), &from_edit)?;
    if let Some(edit) = &to_edit {
        edit_keeps_set_loadable(app, Some(to_set), edit)?;
    }
    app.ui.action_sets[set] = from_edit;
    if let Some(edit) = to_edit {
        app.ui.action_sets[to_set] = edit;
    }
    // A step index in `source_id` or `dest_id` as (the action's id, the step) after the move.
    let follow = |(id, s): (u64, usize)| -> (u64, usize) {
        if (id, s) == (source_id, step) {
            return (dest_id, to);
        }
        let s = if id == source_id && s > step { s - 1 } else { s };
        (id, if id == dest_id && s >= to { s + 1 } else { s })
    };
    // The recording's steps stay after the steps that were before them.
    if let Some(slot) = app.ui.recording_at {
        let slot = if app.ui.recording_into == Some(source_id) && step < slot { slot - 1 } else { slot };
        app.ui.recording_at = Some(if app.ui.recording_into == Some(dest_id) && to <= slot { slot + 1 } else { slot });
    }
    if let Some(ctx) = app.ui_ctx.clone()
        && let Some(selected) = pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP)
    {
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some(follow(selected)));
        if selected == (source_id, step) {
            set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((to_set, to_action)));
            let mut expanded: BTreeSet<u64> = pstate(&ctx, "actions-expanded");
            expanded.insert(dest_id);
            set_pstate(&ctx, "actions-expanded", expanded);
            app.ui.action_set = Some(to_set);
            app.ui.action_sets_selected = vec![to_set];
        }
    }
    // A Record Stop, Insert Menu Item or Set Selection dialog open on a step of either action follows it.
    let either = |s: usize, a: usize| (s, a) == (set, at) || (s, a) == (to_set, to_action);
    if let Some(d) = app.ui.dialog.as_mut().filter(|d| holds_step(d))
        && let (Some(ds), Some(da), Some(ds_step)) =
            (d.fields.get("set").and_then(number), d.fields.get("action").and_then(number), d.fields.get("step").and_then(number))
        && either(ds, da)
    {
        let id = if (ds, da) == (set, at) { source_id } else { dest_id };
        let (id, s) = follow((id, ds_step));
        let (ds, da) = if id == dest_id { (to_set, to_action) } else { (set, at) };
        d.fields.insert("set".into(), json!(ds));
        d.fields.insert("action".into(), json!(da));
        d.fields.insert("step".into(), json!(s));
    }
    Ok(json!({"set": to_set, "action": to_action, "step": to, "moved": true}))
}

/// The command that `chord` runs under `km`, pressed once or pressed twice.
#[cfg(test)]
fn command_on(km: &craft_keymap::Keymap, chord: &craft_keymap::Chord) -> Option<craft_keymap::CommandRef> {
    crate::keymap::lookup(km, chord).or_else(|| match km.lookup(chord, true, &craft_keymap::Active::NONE) {
        craft_keymap::Lookup::Run(c) => Some(c),
        craft_keymap::Lookup::Cycle(v) => v.into_iter().next(),
        _ => None,
    })
}

/// The function returns an error message if the function key is invalid or reserved on this platform.
pub fn key_error(key: FunctionKey) -> Option<String> {
    let Some(chord) = key.chord() else { return Some("no such function key".into()) };
    if crate::keymap::platform() == craft_keymap::Platform::Windows && (key.key == 1 || key.cmd && !key.shift && matches!(key.key, 4 | 6)) {
        return Some(format!("{} cannot be used on Windows", key.text()));
    }
    if crate::keymap::template().is_reserved(&chord) {
        return Some(format!("{} is reserved by the system", key.text()));
    }
    None
}

fn playable_chord(key: FunctionKey) -> Option<craft_keymap::Chord> {
    key_error(key).is_none().then(|| key.chord()).flatten()
}

/// Starts recording into set `set`: Stop Playing/Recording adds an action named `name` with the
/// steps, or, with `into`, gives them to the action with that id.
fn start_recording(app: &mut VectorcraftApp, set: usize, name: String, into: Option<u64>) {
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

/// How many of `steps`, from the first, Stop Playing/Recording can give the recording's action so
/// that its set stays one that Load Actions… reads ([`recording_room`]), and the error that names
/// the limit when that is fewer than all of them.
fn steps_that_fit(app: &VectorcraftApp, steps: &[(String, Value)]) -> (usize, Option<&'static str>) {
    let Some((s, _)) = set_with_recorded(app, vec![]) else { return (steps.len(), None) };
    let stored = recording_target_steps(app);
    let keep = steps.len().min(files::MAX_ACTION_STEPS.saturating_sub(stored));
    // Each step adds `step_bytes` to the file, and the action's first step a little more.
    let mut size = files::write_set(&s).len() + if stored == 0 { files::FIRST_STEP_BYTES } else { 0 };
    let fitting = steps[..keep]
        .iter()
        .take_while(|(c, p)| {
            size += files::step_bytes(c, p);
            size <= files::MAX_SET_FILE
        })
        .count();
    if fitting < keep {
        return (fitting, Some("the action set would be larger than 8 MiB"));
    }
    (keep, (keep < steps.len()).then_some("the action already has 10,000 steps"))
}

/// Puts the recorded `steps` into `action` before its step at index `at`, or after its last step
/// when `at` is None or past it, included in playback. The steps after them keep whether they are
/// excluded. An exclusion past the action's last step, which saved preferences can hold, goes.
fn put_recorded(action: &mut Action, steps: Vec<(String, Value)>, at: Option<usize>) {
    let len = action.steps.len();
    let at = at.map_or(len, |at| at.min(len));
    let added = steps.len();
    action.excluded_steps = action.excluded_steps.iter().filter(|&&i| i < len).map(|&i| if i >= at { i + added } else { i }).collect();
    action.steps.splice(at..at, steps);
}

/// Stop Playing/Recording: ends the recording and keeps its steps (see [`start_recording`]). An
/// action New Action made that was deleted while it recorded keeps nothing. Steps past the limits
/// of an action set file ([`steps_that_fit`]) are left out, and the status bar names the limit.
fn stop_recording(app: &mut VectorcraftApp) {
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

crate::shortcut_editor::ui_mirror!(fn function_key_chords() -> Vec<craft_keymap::Chord>);

/// Mirrors the actions' function keys for code without app access (cheap when unchanged).
pub fn sync_function_keys(ui: &crate::UiState) {
    let chords: Vec<craft_keymap::Chord> = ui.action_sets.iter().flat_map(|s| s.actions.iter().filter_map(|a| playable_chord(a.key?))).collect();
    if function_key_chords().read().is_ok_and(|v| *v != chords)
        && let Ok(mut v) = function_key_chords().write()
    {
        *v = chords;
    }
}

/// Whether `chord` plays an action, by the function keys [`sync_function_keys`] last mirrored.
pub(crate) fn plays_an_action(chord: &craft_keymap::Chord) -> bool {
    let platform = crate::keymap::platform();
    let chord = chord.effective(platform);
    function_key_chords().read().is_ok_and(|v| v.iter().any(|c| c.effective(platform) == chord))
}

/// Plays each action whose function key was pressed this frame, in the order of the presses and
/// before any command with the same shortcut, and takes those key presses. The frame's other
/// events stay for the command, panel and tool shortcuts.
pub fn play_function_key(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let keys: Vec<(craft_keymap::Chord, (usize, usize))> = app
        .ui
        .action_sets
        .iter()
        .enumerate()
        .flat_map(|(si, s)| s.actions.iter().enumerate().filter_map(move |(ai, a)| Some((playable_chord(a.key?)?, (si, ai)))))
        .collect();
    if keys.is_empty() {
        return;
    }
    let platform = crate::keymap::platform();
    let hits: Vec<(usize, usize)> = ctx.input_mut(|i| {
        let mut hits = Vec::new();
        i.events.retain(|e| {
            let hit = match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => craft_keymap::Key::from_egui(*key)
                    .map(|k| craft_keymap::Chord::new(craft_keymap::Mods::from_egui(modifiers, platform), k))
                    .and_then(|c| keys.iter().find(|(k, _)| *k == c).map(|(_, at)| *at)),
                _ => None,
            };
            if !matches!(e, egui::Event::Key { repeat: true, .. }) {
                hits.extend(hit);
            }
            hit.is_none()
        });
        hits
    });
    for (set, action) in hits {
        if let Err(e) = play(app, set, action) {
            app.status(e);
        }
    }
}

/// The command that deletes action sets ([`delete_sets`]).
pub const DELETE_SETS: &str = "ui.actionSet.delete";

/// The sets Delete Selection deletes: the selected sets ([`crate::state::UiState::action_sets_selected`])
/// when the current set is one of them, else the current set alone.
fn selected_sets(app: &VectorcraftApp) -> Vec<usize> {
    let len = app.ui.action_sets.len();
    let current = app.ui.action_set.filter(|&s| s < len);
    let sets: Vec<usize> = app.ui.action_sets_selected.iter().copied().filter(|&s| s < len).collect();
    if current.is_some_and(|c| sets.contains(&c)) { sets } else { current.into_iter().collect() }
}

/// Delete Selection, from the bottom bar and the panel menu: deletes the selected step
/// ([`DELETE_STEP`]), else the selected action, else asks whether to delete the selected sets
/// ([`DELETE_SETS`]).
fn delete_selection(app: &mut VectorcraftApp, ctx: &egui::Context) {
    const SELECTED: &str = "actions-selected";
    // A selected action counts only while its set is the current one, as the panel shows it.
    let selected = pstate::<Option<(usize, usize)>>(ctx, SELECTED).filter(|&(s, _)| app.ui.action_set == Some(s));
    if let Some(((set, action), step)) = selected.zip(selected_step(app, ctx, selected)) {
        if let Err(e) = app.run(DELETE_STEP, json!({"set": set, "action": action, "step": step})) {
            app.status(e);
        }
        return;
    }
    if let Some((si, ai)) = selected {
        if let Some(s) = app.ui.action_sets.get_mut(si)
            && ai < s.actions.len()
        {
            s.actions.remove(ai);
            set_pstate::<Option<(usize, usize)>>(ctx, SELECTED, None);
        }
        return;
    }
    let sets = selected_sets(app);
    let message = match sets[..] {
        [] => return,
        [s] => crate::i18n::fmt(
            tl!("Delete the action set “{name}” and its actions?"),
            &[("name", app.ui.action_sets.get(s).map_or("", |s| s.name.as_str()))],
        ),
        _ => tl!("Delete the selected action sets and their actions?").to_string(),
    };
    let ids: Vec<u64> = sets.iter().filter_map(|&s| app.ui.action_sets.get(s)).map(|s| s.id).collect();
    let params = json!({ "sets": sets, "ids": ids });
    crate::dialogs::confirm::ask(app, &message, "", DELETE_SETS, params);
}

/// `ui.actionSet.delete {sets?: [index], ids?: [id]}`: deletes the action sets at these
/// indexes, with their actions (default: the current set). `ids` holds the panel's own set ids
/// ([`ActionSet::id`]), which Delete Selection's question passes: the sets with these ids are
/// deleted wherever they are now, `sets` is not read, and when one of them no longer exists
/// nothing is deleted. A recording into a deleted set stops, and its steps are dropped; a
/// recording into another set goes on. An open Set Options, New Action or Batch dialog for a
/// deleted set closes. When the current set is deleted, no set is current afterwards.
pub fn delete_sets(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let len = app.ui.action_sets.len();
    let mut sets: Vec<usize> = match (p.get("ids"), p.get("sets")) {
        (Some(v), _) if !v.is_null() => v
            .as_array()
            .ok_or("the action set ids are not a list")?
            .iter()
            .map(|id| {
                id.as_u64().and_then(|id| app.ui.action_sets.iter().position(|s| s.id == id)).ok_or("the action sets changed; nothing was deleted")
            })
            .collect::<Result<_, _>>()?,
        (_, None | Some(Value::Null)) => app.ui.action_set.filter(|&s| s < len).into_iter().collect(),
        (_, Some(v)) => v
            .as_array()
            .ok_or("no such action set")?
            .iter()
            .map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < len).ok_or("no such action set"))
            .collect::<Result<_, _>>()?,
    };
    sets.sort_unstable();
    sets.dedup();
    if let Some((rec, ..)) = app.ui.recording.as_mut() {
        if sets.contains(rec) {
            app.ui.recording = None;
            app.ui.recording_into = None;
            app.ui.recording_at = None;
            app.session.journal.hold(None);
        } else {
            *rec -= sets.iter().filter(|&&s| s < *rec).count();
        }
    }
    // An open Set Options, New Action or Batch dialog holds a set index: it closes when its set is
    // deleted, and otherwise follows its set down.
    if let Some(d) = app.ui.dialog.as_mut().filter(|d| holds_set(d))
        && let Some(set) = d.fields.get("set").and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok())
    {
        if sets.contains(&set) {
            app.ui.dialog = None;
        } else {
            d.fields.insert("set".into(), json!(set - sets.iter().filter(|&&s| s < set).count()));
        }
    }
    // The panel's selected action follows its set down too, and is no longer selected when its set
    // is deleted, so a set made later at its old index does not find it selected.
    if let Some(ctx) = app.ui_ctx.clone()
        && let Some((set, action)) = pstate::<Option<(usize, usize)>>(&ctx, "actions-selected")
    {
        let moved = (!sets.contains(&set)).then(|| (set - sets.iter().filter(|&&s| s < set).count(), action));
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", moved);
    }
    for &s in sets.iter().rev() {
        app.ui.action_sets.remove(s);
    }
    if !sets.is_empty() {
        app.ui.action_set = app.ui.action_set.filter(|c| !sets.contains(c)).map(|c| c - sets.iter().filter(|&&s| s < c).count());
        app.ui.action_sets_selected.clear();
    }
    Ok(json!({ "deleted": sets.len() }))
}

/// Whether dialog `d` is a Set Options, New Action or Batch dialog that holds a set index in its
/// `set` field.
fn holds_set(d: &crate::state::Dialog) -> bool {
    d.kind == crate::dialogs::action_batch::KIND
        || d.kind == crate::dialogs::action_new::KIND
        || holds_step(d)
        || (d.kind == crate::dialogs::action_set_options::KIND && d.fields.contains_key("set"))
}

/// Whether dialog `d` is a Record Stop, Insert Menu Item or Set Selection dialog, which holds the
/// place of a new step in its `set`, `action` and `step` fields.
fn holds_step(d: &crate::state::Dialog) -> bool {
    d.kind == crate::dialogs::action_stop::RECORD_KIND
        || d.kind == crate::dialogs::action_insert_menu_item::KIND
        || d.kind == crate::dialogs::action_select_object::KIND
}

/// The command that duplicates an action or an action set ([`duplicate`]).
pub const DUPLICATE: &str = "ui.actionSet.duplicate";

/// The panel menu's Duplicate: copies the selected action, else the current set ([`DUPLICATE`]),
/// and selects the copy.
fn duplicate_selection(app: &mut VectorcraftApp, ctx: &egui::Context) {
    const SELECTED: &str = "actions-selected";
    let Some(set) = app.ui.action_set.filter(|&s| s < app.ui.action_sets.len()) else { return };
    // A selected action counts only while its set is the current one, as the panel shows it.
    let params = match pstate::<Option<(usize, usize)>>(ctx, SELECTED).filter(|&(s, _)| s == set) {
        Some((s, a)) => json!({ "set": s, "action": a }),
        None => json!({ "set": set }),
    };
    match app.run(DUPLICATE, params) {
        Ok(r) => {
            let copy = r["set"].as_u64().zip(r["action"].as_u64()).and_then(|(s, a)| Some((usize::try_from(s).ok()?, usize::try_from(a).ok()?)));
            set_pstate::<Option<(usize, usize)>>(ctx, SELECTED, copy);
        }
        Err(e) => app.status(e),
    }
}

/// `ui.actionSet.duplicate {set?: index, action?: index}`: copies the action at index `action`
/// of set `set` into the same set, right after the original; without `action`, copies set `set`
/// with all its actions to right after the original set (default: the current set). The copy is
/// named after the original with " copy" added, and a copied set gets an id of its own
/// ([`ActionSet::id`]). Copied actions have no function key, so a key plays only the original.
/// The copy is selected and its set is the current one. A recording and an open Set Options or
/// Batch dialog keep to their own set and action. A copy that would give its set a file that Load
/// Actions… refuses ([`edit_keeps_set_loadable`]) is an error, and nothing is copied.
/// → {set, action?}
pub fn duplicate(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    const SELECTED: &str = "actions-selected";
    let len = app.ui.action_sets.len();
    let index = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok());
    let set = match p.get("set") {
        None | Some(Value::Null) => app.ui.action_set.filter(|&s| s < len).ok_or("no action set is current")?,
        Some(v) => index(v).filter(|&s| s < len).ok_or("no such action set")?,
    };
    let Some(src) = app.ui.action_sets.get(set) else { return Err("no such action set".into()) };
    let action = match p.get("action") {
        None | Some(Value::Null) => None,
        Some(v) => Some(index(v).filter(|&a| a < src.actions.len()).ok_or("no such action")?),
    };
    let (copy, result) = if let Some(a) = action {
        let Some(mut copy) = src.actions.get(a).map(Action::copy) else { return Err("no such action".into()) };
        copy.name = format!("{} copy", copy.name);
        let mut edited = src.clone();
        edited.actions.insert(a + 1, copy);
        edit_keeps_set_loadable(app, Some(set), &edited)?;
        app.ui.action_sets[set] = edited;
        // A Batch, Record Stop, Insert Menu Item or Set Selection dialog open on a later action of the
        // set follows it down.
        if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == crate::dialogs::action_batch::KIND || holds_step(d))
            && d.fields.get("set").and_then(index) == Some(set)
            && let Some(da) = d.fields.get("action").and_then(index).filter(|&da| da > a)
        {
            d.fields.insert("action".into(), json!(da + 1));
        }
        (Some((set, a + 1)), json!({ "set": set, "action": a + 1 }))
    } else {
        let copy = ActionSet::new(format!("{} copy", src.name), src.actions.iter().map(Action::copy).collect());
        edit_keeps_set_loadable(app, None, &copy)?;
        app.ui.action_sets.insert(set + 1, copy);
        if let Some((rec, ..)) = app.ui.recording.as_mut()
            && *rec > set
        {
            *rec += 1;
        }
        if let Some(d) = app.ui.dialog.as_mut().filter(|d| holds_set(d))
            && let Some(ds) = d.fields.get("set").and_then(index).filter(|&ds| ds > set)
        {
            d.fields.insert("set".into(), json!(ds + 1));
        }
        (None, json!({ "set": set + 1 }))
    };
    let current = copy.map_or(set + 1, |(s, _)| s);
    app.ui.action_set = Some(current);
    app.ui.action_sets_selected = vec![current];
    if let Some(ctx) = app.ui_ctx.clone() {
        set_pstate::<Option<(usize, usize)>>(&ctx, SELECTED, copy);
    }
    Ok(result)
}

/// The panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    // Button Mode is off while an action records, so the Stop button the list shows stays in reach.
    let buttons = app.ui.action_button_mode;
    if widgets::menu_item(ui, tl!("Button Mode"), buttons || app.ui.recording.is_none(), buttons)
        && let Err(e) = app.run(BUTTON_MODE, json!({}))
    {
        app.status(e);
    }
    ui.separator();
    // A selected action counts only while its set is the current one, as the panel shows it: a set
    // deleted since leaves the stored pair pointing at another set's action. Button Mode shows no
    // selected action and no current set, so the items that act on them are off there.
    let selected = pstate::<Option<(usize, usize)>>(ui.ctx(), "actions-selected").filter(|&(s, _)| !buttons && app.ui.action_set == Some(s));
    let has = selected.and_then(|(s, a)| app.ui.action_sets.get(s).and_then(|set| set.actions.get(a))).is_some();
    if widgets::menu_item(ui, tl!("New Action…"), crate::dialogs::action_new::can_open(app), false)
        && let Err(e) = crate::dialogs::action_new::open(app)
    {
        app.status(e);
    }
    if widgets::menu_item(ui, tl!("New Set…"), true, false) {
        crate::dialogs::action_set_options::open_new(app);
    }
    let current = app.ui.action_set.filter(|&s| !buttons && s < app.ui.action_sets.len());
    if widgets::menu_item(ui, tl!("Duplicate"), current.is_some(), false) {
        duplicate_selection(app, ui.ctx());
    }
    if widgets::menu_item(ui, tl!("Delete"), has || (!buttons && !selected_sets(app).is_empty()), false) {
        delete_selection(app, ui.ctx());
    }
    if widgets::menu_item(ui, tl!("Set Options…"), current.is_some(), false)
        && let Some(s) = current
    {
        crate::dialogs::action_set_options::open(app, s);
    }
    if widgets::menu_item(ui, tl!("Batch…"), has, false)
        && let Some((s, a)) = selected
        && let Err(e) = crate::dialogs::action_batch::open(app, s, a)
    {
        app.status(e);
    }
    let waiting = waiting_at_stop(app);
    if widgets::menu_item(ui, tl!("Play Current Selection"), has || waiting, false) {
        let played = if waiting { app.run(CONTINUE_PLAYBACK, json!({})) } else { selected.map_or(Ok(Value::Null), |(s, a)| play(app, s, a)) };
        if let Err(e) = played {
            app.status(e);
        }
    }
    let recording = app.ui.recording.is_some();
    if widgets::menu_item(ui, tl!("Insert Stop…"), recording || has, false) {
        let step = selected_step(app, ui.ctx(), selected);
        crate::dialogs::action_stop::open_record(app, if recording { None } else { selected }, step);
    }
    if widgets::menu_item(ui, tl!("Insert Menu Item…"), recording || has, false) {
        let step = selected_step(app, ui.ctx(), selected);
        crate::dialogs::action_insert_menu_item::open(app, if recording { None } else { selected }, step);
    }
    let path = app.session.active().is_some_and(|st| vectorcraft_engine::storable_selection(st).is_ok());
    if widgets::menu_item(ui, tl!("Insert Select Path"), (recording || has) && path, false) {
        let place = match (recording, selected, selected_step(app, ui.ctx(), selected)) {
            (true, ..) | (false, None, _) => json!({}),
            (false, Some((s, a)), None) => json!({"set": s, "action": a}),
            (false, Some((s, a)), Some(step)) => json!({"set": s, "action": a, "step": step}),
        };
        if let Err(e) = app.run(INSERT_SELECT_PATH, place) {
            app.status(e);
        }
    }
    if widgets::menu_item(ui, tl!("Select Object…"), recording || has, false) {
        let step = selected_step(app, ui.ctx(), selected);
        crate::dialogs::action_select_object::open(app, if recording { None } else { selected }, step);
    }
    if widgets::menu_item(ui, tl!("Action Options…"), has, false)
        && let Some((s, a)) = selected
    {
        crate::dialogs::action_set_options::open_action(app, s, a);
    }
    if widgets::menu_item(ui, tl!("Playback Options…"), true, false) {
        crate::dialogs::action_playback::open(app);
    }
    ui.separator();
    // As commands, so a file dialog shown off the UI thread saves or loads when it answers.
    // Save Actions… names the current set by its id, so a file dialog that answers later saves that set.
    let save = json!({ "id": current.and_then(|s| app.ui.action_sets.get(s)).map(|s| s.id) });
    for (label, id, on, params) in
        [(tl!("Save Actions…"), SAVE_SET, current.is_some(), save), (tl!("Load Actions…"), LOAD_SET, can_load(app), json!({}))]
    {
        if widgets::menu_item(ui, label, on, false)
            && let Err(e) = app.run(id, params)
            && e != "cancelled"
        {
            app.status(e);
        }
    }
}

fn inclusion_box(
    ui: &mut egui::Ui,
    row: egui::Rect,
    x: f32,
    id: impl std::hash::Hash + std::fmt::Debug,
    included: bool,
    mixed: bool,
) -> (bool, egui::Rect) {
    let t = Tokens::get(ui.ctx());
    let hit = egui::Rect::from_center_size(egui::pos2(row.left() + x, row.center().y), egui::vec2(18.0, 18.0));
    let response = ui.interact(hit, ui.scope_id().with(id), egui::Sense::click());
    let box_rect = egui::Rect::from_center_size(hit.center(), egui::vec2(11.0, 11.0));
    ui.painter().rect_stroke(
        box_rect,
        1.0,
        egui::Stroke::new(1.0, if response.hovered() { t.text_strong } else { t.icon }),
        egui::StrokeKind::Inside,
    );
    if included || mixed {
        ui.painter().text(
            box_rect.center(),
            egui::Align2::CENTER_CENTER,
            if mixed { "−" } else { "✓" },
            egui::FontId::proportional(12.0),
            t.text_strong,
        );
    }
    (response.on_hover_text(if included { tl!("Exclude from playback") } else { tl!("Include in playback") }).clicked(), hit)
}

/// While a step is dragged over the list: where it goes as (set, action, `before` of [`MOVE_STEP`]),
/// shown by a line in `color` on the row under the pointer, or None off every action's rows
/// (`slots`, as [`show`] lists them). Near the list's top or bottom edge, the list scrolls.
fn drop_target(ui: &mut egui::Ui, slots: &[(egui::Rect, usize, usize, usize, usize)], color: egui::Color32) -> Option<(usize, usize, usize)> {
    let p = ui.ctx().pointer_interact_pos()?;
    let clip = ui.clip_rect();
    if !clip.contains(p) {
        return None;
    }
    let edge = 12.0;
    let scroll = if p.y < clip.top() + edge {
        6.0
    } else if p.y > clip.bottom() - edge {
        -6.0
    } else {
        0.0
    };
    if scroll != 0.0 && ui.input(|i| i.pointer.any_down()) {
        ui.scroll_with_delta(egui::vec2(0.0, scroll));
        ui.ctx().request_repaint();
    }
    let half_gap = ui.spacing().item_spacing.y / 2.0;
    let &(r, si, ai, upper, lower) = slots.iter().find(|(r, ..)| (r.top() - half_gap..r.bottom() + half_gap).contains(&p.y))?;
    let (before, y) = if upper != lower && p.y < r.center().y { (upper, r.top()) } else { (lower, r.bottom()) };
    ui.painter().line_segment([egui::pos2(r.left() + 42.0, y), egui::pos2(r.right(), y)], egui::Stroke::new(2.0, color));
    Some((si, ai, before))
}

enum InclusionChange {
    Set(usize),
    Action(usize, usize),
    Step(usize, usize, usize, bool),
}

/// Button Mode ([`button_mode`]): every action of every set, in set order and then action order,
/// as a button with its name in its color ([`button_colors`]), with no set names, steps or
/// checkmarks. A click on a button, or Enter or Space on the focused one, plays the action
/// ([`PLAY_AT`]). The footer shows only while an action plays or records: Stop Playing/Recording,
/// and Play Current Selection while a playback waits at a stop.
fn show_buttons(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let mut play_req = None;
    let list_height = widgets::panel_list_height(ui, 260.0, 0.0).min(260.0);
    egui::ScrollArea::vertical().id_salt("actions-buttons").max_height(list_height).min_scrolled_height(list_height.min(64.0)).show(ui, |ui| {
        for (si, set) in app.ui.action_sets.iter().enumerate() {
            for (ai, a) in set.actions.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &a.name));
                let (fill, text) = button_colors(a.color.as_deref(), &t);
                let button = r.shrink2(egui::vec2(2.0, 1.0));
                ui.painter().rect_filled(button, 3.0, fill);
                // The pointer over a button, or the keyboard focus on it, rings it in its text color.
                let (width, ring) = if resp.hovered() || resp.has_focus() { (2.0, text) } else { (1.0, t.button_border) };
                ui.painter().rect_stroke(button, 3.0, egui::Stroke::new(width, ring), egui::StrokeKind::Inside);
                ui.painter().with_clip_rect(button.shrink2(egui::vec2(6.0, 0.0))).text(
                    button.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &a.name,
                    egui::FontId::proportional(12.5),
                    text,
                );
                if resp.on_hover_text(&a.name).clicked() {
                    play_req = Some((si, ai));
                }
            }
        }
    });
    if app.ui.action_sets.iter().all(|s| s.actions.is_empty()) {
        dim_label(ui, tl!("No actions."));
    }
    let waiting = waiting_at_stop(app);
    if waiting {
        dim_label(ui, tl!("Press Play Current Selection in the Actions panel to go on."));
    }
    let recording = app.ui.recording.is_some();
    if recording || app.ui.action_playing.is_some() {
        widgets::bottom_bar(ui, |ui| {
            if widgets::icon_button(ui, "square", tl!("Stop Playing/Recording"), false, 24.0).clicked()
                && let Err(e) = app.run(STOP_RECORDING, json!({}))
            {
                app.status(e);
            }
            if waiting
                && widgets::icon_button(ui, "dc-actions", tl!("Play Current Selection"), false, 24.0).clicked()
                && let Err(e) = app.run(CONTINUE_PLAYBACK, json!({}))
            {
                app.status(e);
            }
            if recording {
                ui.label(egui::RichText::new(tl!("● Recording")).color(egui::Color32::from_rgb(0xe0, 0x30, 0x30)));
            }
        });
    }
    if let Some((si, ai)) = play_req
        && let Err(e) = app.run(PLAY_AT, json!({"set": si, "action": ai}))
    {
        app.status(e);
    }
}

/// Draw action sets, steps and playback controls, keeping the list above a pinned footer, or in
/// Button Mode the actions as buttons ([`show_buttons`]).
pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    if app.ui.action_button_mode {
        return show_buttons(app, ui);
    }
    let t = Tokens::get(ui.ctx());
    // The selected action as (set, action). Read as the type it is stored as: egui keeps a
    // value per type, so reading the bare pair never found it and no action stayed selected (#1629).
    const SELECTED: &str = "actions-selected";
    let mut selected: Option<(usize, usize)> = pstate(ui.ctx(), SELECTED);
    // A set made with New Set, or whose name was clicked, is current with none of its actions
    // selected.
    if selected.is_some_and(|(s, _)| app.ui.action_set != Some(s)) {
        selected = None;
    }
    // The selected step as (the action's id, the step), of the selected action only.
    let mut step_selected =
        selected_step(app, ui.ctx(), selected).zip(selected).and_then(|(step, (s, a))| Some((app.ui.action_sets[s].actions.get(a)?.id, step)));
    let mut current = app.ui.action_set;
    let mut sets_selected = selected_sets(app);
    let shift = ui.input(|i| i.modifiers.shift);
    let alt = ui.input(|i| i.modifiers.alt);
    let mut expanded: BTreeSet<u64> = pstate(ui.ctx(), "actions-expanded");
    let mut inclusion_changes = Vec::new();
    let mut play_req = None;
    // The step being dragged, from the frame its drag started. The drag ends without a move when a
    // command since then changed its action's steps or which of them are excluded, or deleted the
    // action.
    let dragging = pstate::<Option<DraggedStep>>(ui.ctx(), DRAGGED_STEP).filter(|(id, _, was)| {
        let action = app.ui.action_sets.iter().flat_map(|s| &s.actions).find(|a| a.id == *id);
        action.is_some_and(|a| (&a.steps, &a.excluded_steps) == (&was.0, &was.1))
    });
    let mut drag_started = None;
    let mut drag_stopped = false;
    // The rows a dragged step can be dropped on: (row, set, action, where it goes on the row's upper
    // half, where on its lower half), as `before` of [`MOVE_STEP`].
    let mut slots: Vec<(egui::Rect, usize, usize, usize, usize)> = Vec::new();
    let mut drop = None;
    let list_height = widgets::panel_list_height(ui, 260.0, 0.0).min(260.0);
    egui::ScrollArea::vertical().max_height(list_height).min_scrolled_height(list_height.min(64.0)).show(ui, |ui| {
        for (si, set) in app.ui.action_sets.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
            if sets_selected.contains(&si) && selected.is_none() {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover);
            }
            let step_count: usize = set.actions.iter().map(|a| a.steps.len()).sum();
            let excluded_count: usize = set.actions.iter().map(|a| (0..a.steps.len()).filter(|i| a.excluded_steps.contains(i)).count()).sum();
            let (check_clicked, check_rect) =
                inclusion_box(ui, r, 10.0, ("set-inclusion", set.id), excluded_count == 0, excluded_count > 0 && excluded_count < step_count);
            if check_clicked && step_count > 0 {
                inclusion_changes.push(InclusionChange::Set(si));
            }
            ui.painter().text(
                r.left_center() + egui::vec2(22.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("▾ {}", set.name),
                egui::FontId::proportional(12.5),
                t.text,
            );
            if resp.clicked() && !resp.interact_pointer_pos().is_some_and(|p| check_rect.contains(p)) {
                // Shift-click selects the sets from the current one to the one clicked; the current
                // set stays as it was.
                match current {
                    Some(c) if shift && sets_selected.contains(&c) => sets_selected = (c.min(si)..=c.max(si)).collect(),
                    _ => {
                        current = Some(si);
                        sets_selected = vec![si];
                    }
                }
                selected = None;
                step_selected = None;
            }
            for (ai, a) in set.actions.iter().enumerate() {
                let sel = selected == Some((si, ai));
                let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
                if sel {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let excluded = (0..a.steps.len()).filter(|i| a.excluded_steps.contains(i)).count();
                let (check_clicked, check_rect) =
                    inclusion_box(ui, r, 10.0, ("action-inclusion", a.id), excluded == 0, excluded > 0 && excluded < a.steps.len());
                if check_clicked && !a.steps.is_empty() {
                    inclusion_changes.push(InclusionChange::Action(si, ai));
                }
                let disclosure = egui::Rect::from_center_size(egui::pos2(r.left() + 29.0, r.center().y), egui::vec2(18.0, 18.0));
                let disclosure_resp = ui.interact(disclosure, ui.scope_id().with(("action-disclosure", a.id)), egui::Sense::click());
                let open = expanded.contains(&a.id);
                icons::paint(
                    ui,
                    if open { "chevron-down" } else { "chevron-right" },
                    disclosure,
                    if disclosure_resp.hovered() { t.text_strong } else { t.icon },
                );
                if disclosure_resp.clicked() && !expanded.insert(a.id) {
                    expanded.remove(&a.id);
                }
                ui.painter().text(
                    r.left_center() + egui::vec2(42.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &a.name,
                    egui::FontId::proportional(12.5),
                    t.text_strong,
                );
                ui.painter().text(
                    r.right_center() - egui::vec2(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    crate::i18n::tn(a.steps.len() as u64, "{n} step", "{n} steps"),
                    egui::FontId::proportional(11.0),
                    t.text_dim,
                );
                if resp.clicked() && !resp.interact_pointer_pos().is_some_and(|p| check_rect.contains(p) || disclosure.contains(p)) {
                    selected = Some((si, ai));
                    step_selected = None;
                    current = Some(si);
                    sets_selected = vec![si];
                }
                if resp.double_clicked() && !resp.interact_pointer_pos().is_some_and(|p| check_rect.contains(p) || disclosure.contains(p)) {
                    play_req = Some((si, ai));
                }
                // A step dropped on an action's row goes first when the action shows its steps, else
                // after its last step.
                let first = if open && !a.steps.is_empty() { 0 } else { a.steps.len() };
                slots.push((r, si, ai, first, first));
                if open {
                    for (step, (command, params)) in a.steps.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click_and_drag());
                        slots.push((r, si, ai, step, step + 1));
                        if resp.drag_started() {
                            drag_started = Some((a.id, step, std::sync::Arc::new((a.steps.clone(), a.excluded_steps.clone()))));
                        }
                        drag_stopped |= resp.drag_stopped();
                        if step_selected == Some((a.id, step)) {
                            ui.painter().rect_filled(r, 0.0, t.row_selected);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 0.0, t.hover);
                        }
                        let (clicked, check_rect) =
                            inclusion_box(ui, r, 55.0, ("step-inclusion", a.id, step), !a.excluded_steps.contains(&step), false);
                        if clicked {
                            inclusion_changes.push(InclusionChange::Step(si, ai, step, alt));
                        }
                        if resp.clicked() && !resp.interact_pointer_pos().is_some_and(|p| check_rect.contains(p)) {
                            selected = Some((si, ai));
                            step_selected = Some((a.id, step));
                            current = Some(si);
                            sets_selected = vec![si];
                        }
                        // A stop's row shows its message, a Select Path step's its name and a Select Object step's its note; other
                        // steps show their command.
                        let shown = if command == STOP_STEP {
                            let message = params.get("message").and_then(Value::as_str).and_then(|m| m.lines().next()).unwrap_or_default();
                            crate::i18n::fmt(tl!("Stop: {message}"), &[("message", message)])
                        } else if command == SELECT_PATH_STEP {
                            tl!("Select Path").to_string()
                        } else if command == SELECT_OBJECT_STEP {
                            let note = params.get("note").and_then(Value::as_str).and_then(|m| m.lines().next()).unwrap_or_default();
                            crate::i18n::fmt(tl!("Select Object: {note}"), &[("note", note)])
                        } else {
                            command.clone()
                        };
                        ui.painter().text(
                            r.left_center() + egui::vec2(70.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            shown,
                            egui::FontId::proportional(11.0),
                            t.text_dim,
                        );
                        resp.on_hover_ui(|ui| {
                            ui.label(format!("{command}\n{params}"));
                        });
                    }
                }
            }
        }
        if dragging.is_some() {
            drop = drop_target(ui, &slots, t.accent);
        }
    });
    // A released drag moves the step once, to where the line showed.
    let dragged = dragging.as_ref().map(|(id, step, _)| (*id, *step));
    let move_req = drag_stopped.then_some(dragged.zip(drop)).flatten().and_then(|((id, step), (si, ai, before))| {
        let (fs, fa) = app.ui.action_sets.iter().enumerate().find_map(|(fs, s)| Some((fs, s.actions.iter().position(|a| a.id == id)?)))?;
        Some(json!({"set": fs, "action": fa, "step": step, "toSet": si, "toAction": ai, "before": before}))
    });
    let still_dragging = !drag_stopped && ui.input(|i| i.pointer.any_down());
    set_pstate(ui.ctx(), DRAGGED_STEP, drag_started.or(dragging.filter(|_| still_dragging)));
    if waiting_at_stop(app) {
        dim_label(ui, tl!("Press Play Current Selection in the Actions panel to go on."));
    }
    for change in inclusion_changes {
        let params = match change {
            InclusionChange::Set(si) => {
                json!({"set": si, "included": app.ui.action_sets.get(si).is_some_and(|s| s.actions.iter().any(|a| !a.all_steps_included()))})
            }
            InclusionChange::Action(si, ai) => {
                json!({"set": si, "action": ai, "included": app.ui.action_sets.get(si).and_then(|s| s.actions.get(ai)).is_some_and(|a| !a.all_steps_included())})
            }
            InclusionChange::Step(si, ai, step, alt) => json!({
                "set": si,
                "action": ai,
                "step": step,
                "others": alt,
                "included": app.ui.action_sets.get(si).and_then(|s| s.actions.get(ai)).is_some_and(|a| a.excluded_steps.contains(&step)),
            }),
        };
        if let Err(error) = app.run(INCLUDE_STEPS, params) {
            app.status(error);
        }
    }
    set_pstate(ui.ctx(), "actions-expanded", expanded);
    set_pstate(ui.ctx(), SELECTED, selected);
    set_pstate(ui.ctx(), SELECTED_STEP, step_selected);
    app.ui.action_set = current;
    app.ui.action_sets_selected = sets_selected;
    if let Some(params) = move_req
        && let Err(e) = app.run(MOVE_STEP, params)
    {
        app.status(e);
    }
    // Begin Recording records into the selected action, after its selected step.
    let record_params = match selected.and_then(|(s, a)| Some((s, a, app.ui.action_sets.get(s)?.actions.get(a)?.id))) {
        Some((s, a, id)) => match step_selected.filter(|&(step_id, _)| step_id == id) {
            Some((_, step)) => json!({"set": s, "action": a, "after": step}),
            None => json!({"set": s, "action": a}),
        },
        None => json!({}),
    };
    if app.ui.action_sets.is_empty() {
        dim_label(ui, tl!("No actions. Press ● to record."));
    }
    // Bottom bar: stop, record, play, new set, new action, delete.
    widgets::bottom_bar(ui, |ui| {
        let recording = app.ui.recording.is_some();
        if widgets::icon_button(ui, "square", tl!("Stop Playing/Recording"), false, 24.0).clicked()
            && let Err(e) = app.run(STOP_RECORDING, json!({}))
        {
            app.status(e);
        }
        let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
        ui.painter().circle_filled(r.center(), 6.0, if recording { egui::Color32::from_rgb(0xe0, 0x30, 0x30) } else { t.icon });
        if resp.on_hover_text(tl!("Begin Recording")).clicked()
            && !recording
            && let Err(e) = app.run(START_RECORDING, record_params.clone())
        {
            app.status(e);
        }
        if widgets::icon_button(ui, "dc-actions", tl!("Play Current Selection"), false, 24.0).clicked() {
            if waiting_at_stop(app) {
                if let Err(e) = app.run(CONTINUE_PLAYBACK, json!({})) {
                    app.status(e);
                }
            } else if let Some(s) = selected {
                play_req = Some(s);
            }
        }
        if widgets::icon_button(ui, "dc-folder", tl!("Create New Set"), false, 24.0).clicked() {
            crate::dialogs::action_set_options::open_new(app);
        }
        if widgets::icon_button(ui, "dc-new-item", tl!("Create New Action"), false, 24.0).clicked()
            && let Err(e) = crate::dialogs::action_new::open(app)
        {
            app.status(e);
        }
        if widgets::icon_button(ui, "trash-2", tl!("Delete Selection"), false, 24.0).clicked() {
            delete_selection(app, ui.ctx());
        }
        if recording {
            ui.label(egui::RichText::new(tl!("● Recording")).color(egui::Color32::from_rgb(0xe0, 0x30, 0x30)));
        }
    });
    if let Some((si, ai)) = play_req
        && let Err(e) = play(app, si, ai)
    {
        app.status(e);
    }
    let _ = icons::exists;
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// A short sized panel clips the list above its footer and can scroll to select its final row.
    #[test]
    fn a_short_actions_panel_keeps_its_last_row_reachable_above_the_footer() {
        use egui::{Event, PointerButton, Rect, UiBuilder, UiStackInfo, pos2, vec2};
        for height in [60.0, 120.0] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            let ctx = egui::Context::default();
            let mut time = 0.0;
            let mut frame = |app: &mut VectorcraftApp, events| {
                time += 0.1;
                let input = egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, height))),
                    events,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let info = UiStackInfo::default().with_tag_value(widgets::PANEL_FOOTER, Some(height));
                    ui.scope_builder(UiBuilder::new().ui_stack_info(info), |ui| show(app, ui));
                });
                output.textures_delta.clear();
                output.shapes
            };
            let last_row = |shapes: &[egui::epaint::ClippedShape]| {
                shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "Reflect Horizontal" => Some((text.visual_bounding_rect(), shape.clip_rect)),
                        _ => None,
                    })
                    .expect("the final action is drawn")
            };
            for _ in 0..3 {
                let shapes = frame(&mut app, vec![]);
                assert!(last_row(&shapes).1.bottom() <= height - 31.0, "height={height}: the list overlaps its footer");
            }
            for _ in 0..12 {
                frame(&mut app, crate::toolbar::tests::wheel(pos2(100.0, 12.0), 1000.0));
            }
            let shapes = frame(&mut app, vec![]);
            let (row, clip) = last_row(&shapes);
            assert!(clip.contains_rect(row), "height={height}: the final row {row:?} is outside {clip:?}");
            let at = row.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
                frame(&mut app, events);
            }
            assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((0, 5)));
        }
    }

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

    /// An action recorded with a key its command no longer reads still plays, accelerated and one
    /// step at a time, though the same command run by itself is refused.
    #[test]
    fn an_action_with_a_key_no_longer_read_still_plays() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = vectorcraft_doc::NodeId(
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap(),
        );
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        let old = json!({"dx": 5, "dy": 0, "dyy": 1});
        assert!(app.run("object.move", old.clone()).is_err());
        app.ui.action_sets =
            vec![ActionSet::new("Test", vec![Action::new("Move", vec![("object.move".into(), old.clone()), ("object.move".into(), old)])])];
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
    fn excluded_steps_do_not_play_and_survive_saved_sets() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = vectorcraft_doc::NodeId(
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap(),
        );
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

    #[test]
    fn inclusion_controls_expand_and_toggle_steps_without_selecting_the_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let action = Action::new(
            "Mixed",
            vec![
                ("object.move".into(), json!({"dx": 1})),
                ("object.rotate".into(), json!({"angle": 1})),
                ("object.reflect".into(), json!({"axis": "vertical"})),
            ],
        );
        app.ui.action_sets = vec![ActionSet::new("Example", vec![action])];
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        assert!(!texts.iter().any(|(s, _)| s == "object.rotate"), "steps start collapsed");
        let label = text_rect(&texts, "Mixed");
        click(&ctx, &mut app, egui::pos2(label.left() - 13.0, label.center().y), show);
        let texts = frame_events(&ctx, &mut app, vec![], show);
        let step = text_rect(&texts, "object.rotate");
        click(&ctx, &mut app, egui::pos2(step.left() - 15.0, step.center().y), show);
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([1]));
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), None, "the checkbox does not select the action");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        let label = text_rect(&texts, "Mixed");
        click(&ctx, &mut app, egui::pos2(label.left() - 32.0, label.center().y), show);
        assert!(app.ui.action_sets[0].actions[0].excluded_steps.is_empty(), "mixed action check includes all steps");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        let label = text_rect(&texts, "▾ Example");
        click(&ctx, &mut app, egui::pos2(label.left() - 12.0, label.center().y), show);
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([0, 1, 2]), "set check excludes all steps");
    }

    #[test]
    fn alt_click_keeps_the_chosen_step_and_toggles_the_others() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let action = Action::new("Three", vec![("a".into(), json!({})), ("b".into(), json!({})), ("c".into(), json!({}))]);
        app.ui.action_sets = vec![ActionSet::new("Example", vec![action])];
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": false, "others": true})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([0, 2]));
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 0, "included": true, "others": true})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([0]));
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 0, "included": true})).unwrap();
        assert!(app.ui.action_sets[0].actions[0].excluded_steps.is_empty());
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 0, "included": true, "others": true})).unwrap();
        assert!(app.ui.action_sets[0].actions[0].excluded_steps.is_empty(), "including the other steps preserves the clicked included step");
        for invalid in [
            json!({"set": 0, "action": 0, "step": 3, "included": false}),
            json!({"set": 0, "action": 3, "included": false}),
            json!({"set": 2, "included": false}),
            json!({"set": 0, "step": 1, "included": false}),
            json!({"set": 0, "included": false, "others": true}),
            json!({"set": 0, "included": "false"}),
        ] {
            assert!(app.run(INCLUDE_STEPS, invalid.clone()).is_err(), "{invalid}");
            assert!(app.ui.action_sets[0].actions[0].excluded_steps.is_empty(), "invalid params changed playback: {invalid}");
        }
    }

    #[test]
    fn begin_recording_command_matches_the_panel_button() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.ui.action_sets.clear();
        app.ui.action_set = None;
        assert_eq!(app.run(START_RECORDING, json!({})).unwrap(), json!({"set": 0, "name": "Action 1"}));
        assert_eq!((app.ui.action_sets[0].name.as_str(), app.ui.action_set), ("Set 1", Some(0)));
        assert_eq!(app.run(START_RECORDING, json!({})).unwrap_err(), "an action is already recording");
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
        assert_eq!((app.ui.action_sets[0].actions[0].name.as_str(), app.ui.action_sets[0].actions[0].steps.len()), ("Action 1", 1));

        app.ui.action_sets.push(ActionSet::new("Other", vec![]));
        assert_eq!(app.run(START_RECORDING, json!({"set": 99})).unwrap_err(), "no such action set");
        assert_eq!(app.run(START_RECORDING, json!({"set": null})).unwrap_err(), "no such action set");
        assert!(app.ui.recording.is_none());
        assert_eq!(app.run(START_RECORDING, json!({"set": 1})).unwrap(), json!({"set": 1, "name": "Action 1"}));
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.as_slice()), (Some(1), &[1][..]));
        assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
    }

    /// #1629: a clicked action stays selected, so Play Current Selection plays it and Delete
    /// Selection deletes it.
    #[test]
    fn a_clicked_action_stays_selected_for_play_and_delete() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Reflect Horizontal").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((0, 5)), "selected on the frames after the click");
        // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, new action,
        // delete.
        let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [_, _, play, _, _, delete] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        click(&ctx, &mut app, text_rect(&texts, "Duplicate and Offset 10 pt").center(), show);
        click(&ctx, &mut app, play.center(), show);
        assert_eq!(app.session.active().unwrap().selection.len(), 1);
        assert_ne!(app.session.active().unwrap().selection.objects, [id], "played: the copy is selected");
        let names = |app: &VectorcraftApp| app.ui.action_sets[0].actions.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
        let before = names(&app);
        click(&ctx, &mut app, delete.center(), show);
        assert_eq!(names(&app), before[1..], "the selected action is deleted");
        click(&ctx, &mut app, delete.center(), show);
        assert_eq!(names(&app), before[1..], "nothing selected after a delete: nothing more goes");
    }

    /// The fills of the rectangles drawn in one headless frame of the panel.
    fn drawn_fills(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<egui::Color32> {
        fn collect(s: &egui::Shape, out: &mut Vec<egui::Color32>) {
            match s {
                egui::Shape::Rect(r) => out.push(r.fill),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        output.textures_delta.clear();
        let mut fills = vec![];
        output.shapes.iter().for_each(|c| collect(&c.shape, &mut fills));
        fills
    }

    /// The panel menu's Button Mode switches the panel to buttons and back, with a check mark while
    /// it is on; `ui.action.buttonMode` does the same, and the view is kept with the UI preferences.
    /// Turning it on while an action records is refused, so Stop stays in reach.
    #[test]
    fn button_mode_switches_the_view_and_is_kept_with_the_preferences() {
        use super::super::tests_appearance::{click, frame_events};
        let mut app = app_with_sets();
        assert!(!app.ui.action_button_mode, "the list is the default");
        let ctx = egui::Context::default();
        let item = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            texts.into_iter().find(|(s, _)| s.trim_start_matches('✓').trim() == "Button Mode").expect("Button Mode in the menu")
        };
        let (label, at) = item(&ctx, &mut app);
        assert_eq!(label, "   Button Mode", "no check mark while off");
        click(&ctx, &mut app, at.center(), menu);
        assert!(app.ui.action_button_mode, "the menu item turns it on");
        assert_eq!(item(&ctx, &mut app).0, "✓ Button Mode", "a check mark while on");
        let at = item(&ctx, &mut app).1;
        click(&ctx, &mut app, at.center(), menu);
        assert!(!app.ui.action_button_mode, "choosing it again returns to the list");

        assert_eq!(app.run(BUTTON_MODE, json!({})).unwrap(), json!({"enabled": true}), "left out: the other view");
        assert_eq!(app.run(BUTTON_MODE, json!({"enabled": true})).unwrap(), json!({"enabled": true}));
        assert_eq!(app.run(BUTTON_MODE, json!({"enabled": "no"})).unwrap_err(), "`enabled` must be true or false");
        assert!(app.ui.action_button_mode, "a bad value keeps the view");
        let saved = serde_json::to_value(&app.ui).unwrap();
        assert_eq!(saved["action_button_mode"], json!(true));
        assert!(serde_json::from_value::<crate::state::UiState>(saved.clone()).unwrap().action_button_mode, "kept after a restart");
        let mut older = saved;
        older.as_object_mut().unwrap().remove("action_button_mode");
        assert!(!serde_json::from_value::<crate::state::UiState>(older).unwrap().action_button_mode, "older preferences show the list");

        assert_eq!(app.run(BUTTON_MODE, json!({"enabled": null})).unwrap(), json!({"enabled": false}));
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run(START_RECORDING, json!({})).unwrap();
        assert_eq!(app.run(BUTTON_MODE, json!({})).unwrap_err(), "an action is already recording");
        let at = item(&ctx, &mut app).1;
        click(&ctx, &mut app, at.center(), menu);
        assert!(!app.ui.action_button_mode, "the menu item is off while an action records");
        assert_eq!(app.run(BUTTON_MODE, json!({"enabled": false})).unwrap(), json!({"enabled": false}), "turning it off is never refused");
        app.run(STOP_RECORDING, json!({})).unwrap();
    }

    /// Button Mode shows every action of every set by its name, in set order and then action order,
    /// without set names, step counts or steps, each in its Action Options color; an action without
    /// a color is an ordinary button. The menu items for a selected action or the current set are
    /// off, since the view shows neither.
    #[test]
    fn button_mode_shows_every_action_as_a_button_in_its_color() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "color": "red"})).unwrap();
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "color": "yellow"})).unwrap();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((1, 0)), "selected in the list");
        app.run(BUTTON_MODE, json!({"enabled": true})).unwrap();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        let names: Vec<&str> = texts.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "Duplicate and Offset 10 pt",
                "Unite and Simplify",
                "Center on Artboard",
                "Outline Text",
                "Rotate 90° CW",
                "Reflect Horizontal",
                "Nudge",
                "Nudge Again"
            ],
            "only the actions' names, in set and action order"
        );
        let tops: Vec<f32> = texts.iter().map(|(_, r)| r.top()).collect();
        assert!(tops.windows(2).all(|w| w[0] < w[1]), "one button under the other: {tops:?}");
        let t = Tokens::get(&ctx);
        let fills = drawn_fills(&ctx, &mut app);
        for color in ["red", "yellow"] {
            let fill = button_fill(color).unwrap();
            assert_eq!(fills.iter().filter(|f| **f == fill).count(), 1, "{color}: {fills:?}");
        }
        assert_eq!(fills.iter().filter(|f| **f == t.button).count(), 6, "the six actions without a color: {fills:?}");
        for (color, _) in COLORS {
            let (fill, text) = button_colors(Some(color), &t);
            let (hi, lo) = (luminance(fill).max(luminance(text)), luminance(fill).min(luminance(text)));
            assert!((hi + 0.05) / (lo + 0.05) >= 4.5, "{color}: its name is legible");
        }
        assert_eq!(button_colors(None, &t), (t.button, t.text_strong));
        let menu_texts = frame_events(&ctx, &mut app, vec![], menu);
        click(&ctx, &mut app, text_rect(&menu_texts, "   Delete").center(), menu);
        click(&ctx, &mut app, text_rect(&menu_texts, "   Action Options…").center(), menu);
        assert!(app.ui.dialog.is_none(), "Delete and Action Options… are off in Button Mode");
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"]);
        assert_eq!(action_names(&app, 1), ["Nudge", "Nudge Again"], "the action selected in the list stays");
    }

    /// A click on a button plays that action at once, as one undo step at Accelerated, without its
    /// excluded steps, which stay excluded; `ui.action.playAt` refuses an index that names no action.
    #[test]
    fn a_click_on_a_button_plays_its_action_without_its_excluded_steps() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let (mut app, x) = app_with_three_moves();
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap();
        app.run(BUTTON_MODE, json!({"enabled": true})).unwrap();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        let undo = undo_steps(&app);
        click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
        assert_eq!((x(&app), undo_steps(&app)), (5.0, undo + 1), "1 and 4 pt as one undo step; the 2 pt step is skipped");
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([1]), "the exclusion stays");
        for (p, want) in [
            (json!({"set": 0, "action": 1}), "no such action"),
            (json!({"set": 1, "action": 0}), "no such action"),
            (json!({"set": "0", "action": 0}), "no such action"),
            (json!({"set": 0}), "no such action"),
        ] {
            assert_eq!(app.run(PLAY_AT, p.clone()).unwrap_err(), want, "{p}");
        }
        assert_eq!(x(&app), 5.0, "nothing more played");
        app.run(PLAY_AT, json!({"set": 0, "action": 0})).unwrap();
        assert_eq!(x(&app), 10.0);
    }

    /// Button Mode shows no bottom bar while nothing plays or records. While an action plays step by
    /// step its Stop Playing/Recording button ends the playback, and while one waits at a stop its
    /// Play Current Selection button plays on.
    #[test]
    fn button_mode_shows_stop_and_continue_only_while_an_action_plays() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let bar = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            frame_events(ctx, app, vec![], show);
            let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
            bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
            // A button can be listed more than once in a pass.
            bar.dedup();
            bar
        };
        let (mut app, x) = app_with_three_moves();
        app.run(BUTTON_MODE, json!({"enabled": true})).unwrap();
        app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
        let ctx = egui::Context::default();
        assert_eq!(bar(&ctx, &mut app), [], "no bottom bar while idle");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
        assert_eq!(x(&app), 1.0, "the first step played");
        let [stop] = bar(&ctx, &mut app)[..] else { panic!("only Stop while it plays") };
        click(&ctx, &mut app, stop.center(), show);
        assert!(app.ui.action_playing.is_none(), "Stop ends the playback");
        assert_eq!(x(&app), 1.0);

        let (mut app, x) = app_with_a_stop(true);
        app.run(BUTTON_MODE, json!({"enabled": true})).unwrap();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
        assert!(waiting_at_stop(&app));
        assert_eq!(x(&app), 1.0, "played up to the stop");
        let [_, play] = bar(&ctx, &mut app)[..] else { panic!("Stop and Play Current Selection at a stop") };
        click(&ctx, &mut app, play.center(), show);
        assert!(app.ui.action_playing.is_none());
        assert_eq!(x(&app), 7.0, "Play Current Selection plays on");
    }

    /// #3712: Create New Set asks for a name (Set 1 when free), the new set is the current one, so a
    /// recording goes into it, and a click on another set's name makes that one current. The sets
    /// are saved with the UI preferences.
    #[test]
    fn create_new_set_names_a_set_that_recordings_go_into() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, new action,
        // delete. A new set or action moves the bar down.
        let bar = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            frame_events(ctx, app, vec![], show);
            let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
            bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
            let [stop, record, _, new_set, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
            (stop, record, new_set)
        };
        let (_, _, new_set) = bar(&ctx, &mut app);
        click(&ctx, &mut app, new_set.center(), show);
        let d = app.ui.dialog.as_ref().expect("New Set asks for a name");
        assert_eq!((d.kind.as_str(), d.str("name")), ("actionSetOptions", "Set 1".to_string()));
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Print"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let sets = |app: &VectorcraftApp| app.ui.action_sets.iter().map(|s| (s.name.clone(), s.actions.len())).collect::<Vec<_>>();
        assert_eq!(sets(&app), [("Default Actions".to_string(), 6), ("Print".into(), 0)]);
        let rect = json!({"x": 10, "y": 10, "width": 20, "height": 20});
        let (stop, record, _) = bar(&ctx, &mut app);
        click(&ctx, &mut app, record.center(), show);
        app.run("shape.rectangle", rect.clone()).unwrap();
        click(&ctx, &mut app, stop.center(), show);
        assert_eq!(sets(&app), [("Default Actions".to_string(), 6), ("Print".into(), 1)], "recorded into the new set");
        assert_eq!(app.ui.action_sets[1].actions[0].name, "Action 1", "numbered within its set");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "▾ Default Actions").center(), show);
        let (stop, record, _) = bar(&ctx, &mut app);
        click(&ctx, &mut app, record.center(), show);
        app.run("shape.rectangle", rect).unwrap();
        click(&ctx, &mut app, stop.center(), show);
        assert_eq!(sets(&app), [("Default Actions".to_string(), 7), ("Print".into(), 1)], "recorded into the set clicked");
        let saved = serde_json::to_value(&app.ui).unwrap();
        let restored = serde_json::from_value::<crate::state::UiState>(saved).unwrap();
        assert_eq!(restored.action_sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Default Actions", "Print"]);
    }

    /// #3712: the panel menu's Set Options… renames the current set and keeps its actions.
    #[test]
    fn set_options_renames_the_current_set() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.action_sets.push(ActionSet::new("Mine", vec![]));
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "▾ Mine").center(), show);
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Set Options…").map(|(_, r)| *r).expect("Set Options… in the menu");
        click(&ctx, &mut app, item.center(), menu);
        let d = app.ui.dialog.as_ref().expect("Set Options opens");
        assert_eq!((d.kind.as_str(), d.str("name")), ("actionSetOptions", "Mine".to_string()));
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Web"));
        crate::dialogs::confirm(&mut app).unwrap();
        let names = app.ui.action_sets.iter().map(|s| (s.name.as_str(), s.actions.len())).collect::<Vec<_>>();
        assert_eq!(names, [("Default Actions", 6), ("Web", 0)], "the clicked set is renamed, the other kept");
    }

    /// #1629: the panel menu's Action Options… is off with no action selected, and with an action
    /// selected it asks for a new name: OK renames that action and keeps its steps, Cancel keeps
    /// the name.
    #[test]
    fn action_options_renames_the_selected_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let action_options = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            let item = texts.iter().find(|(s, _)| s.trim() == "Action Options…").map(|(_, r)| *r).expect("Action Options… in the menu");
            click(ctx, app, item.center(), menu);
        };
        action_options(&ctx, &mut app);
        assert!(app.ui.dialog.is_none(), "no action selected: Action Options… is off");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge Again").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        action_options(&ctx, &mut app);
        let d = app.ui.dialog.as_ref().expect("Action Options opens");
        assert_eq!((d.kind.as_str(), d.str("name")), (crate::dialogs::action_set_options::KIND, "Nudge Again".to_string()));
        crate::dialogs::cancel(&mut app);
        assert_eq!(app.ui.action_sets[1].actions[1].name, "Nudge Again", "Cancel keeps the name");
        action_options(&ctx, &mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("  Nudge Right  "));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let names = app.ui.action_sets[1].actions.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Nudge", "Nudge Right"], "the selected action is renamed, trimmed; the other is kept");
        assert_eq!(app.ui.action_sets[1].actions[1].steps, [("object.move".to_string(), json!({"dx": 1, "dy": 0}))], "the steps stay");
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "no set is renamed");
    }

    /// `ui.action.rename` renames the action at its set and action indexes and refuses a missing
    /// index or a name with nothing but spaces, renaming nothing.
    #[test]
    fn rename_action_renames_one_action_and_refuses_bad_params() {
        let mut app = app_with_sets();
        let names = |app: &VectorcraftApp| app.ui.action_sets[1].actions.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
        assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Push"})).unwrap(), json!({"name": "Push"}));
        assert_eq!(names(&app), ["Push", "Nudge Again"]);
        for p in [
            json!({"set": 1, "action": 2, "name": "X"}),
            json!({"set": 3, "action": 0, "name": "X"}),
            json!({"set": 1, "name": "X"}),
            json!({"action": 0, "name": "X"}),
            json!({"set": 4_294_967_297u64, "action": 0, "name": "X"}),
        ] {
            assert_eq!(app.run(RENAME_ACTION, p.clone()).unwrap_err(), "no such action", "{p}");
        }
        assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "   "})).unwrap_err(), "name is empty");
        assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0})).unwrap_err(), "name is empty");
        assert_eq!(names(&app), ["Push", "Nudge Again"], "a refused rename renames nothing");
    }

    /// OK in Action Options or Set Options whose `set` or `action` field `ui.dialog.set` changed to
    /// something that is not an index is an error: it renames no set or action, adds no set, and
    /// keeps the dialog open.
    #[test]
    fn options_with_a_field_that_is_not_an_index_change_nothing() {
        use crate::dialogs::action_set_options::KIND;
        let mut app = app_with_sets();
        for fields in [
            json!({"set": 1, "action": null, "name": "X"}),
            json!({"set": null, "action": 0, "name": "X"}),
            json!({"set": 1, "action": 9, "name": "X"}),
            json!({"set": null, "name": "X"}),
            json!({"set": 9, "name": "X"}),
        ] {
            app.ui.dialog = Some(crate::state::Dialog::new(KIND, fields.clone()));
            assert!(crate::dialogs::confirm(&mut app).is_err(), "{fields}");
            assert!(app.ui.dialog.is_some(), "the dialog stays open: {fields}");
            assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "{fields}");
            let names = app.ui.action_sets[1].actions.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
            assert_eq!(names, ["Nudge", "Nudge Again"], "{fields}");
        }
    }

    /// After `ui.actionSet.delete` removes a set above the selected action's set, the panel menu's
    /// Action Options… is off: it does not open for the action that moved to the stored position.
    #[test]
    fn action_options_is_off_after_a_set_above_the_selection_is_deleted() {
        use super::super::tests_appearance::{click, frame_events};
        let mut app = app_with_sets();
        let action = |name: &str| Action::new(name, vec![]);
        app.ui.action_sets[2].actions.push(action("Spin"));
        app.ui.action_sets.push(ActionSet::new("Last", vec![action("Wrong")]));
        let ctx = egui::Context::default();
        app.ui.action_set = Some(2);
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((2, 0)));
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert_eq!(app.ui.action_set, Some(1), "Other is still current");
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Action Options…").map(|(_, r)| *r).expect("Action Options… in the menu");
        click(&ctx, &mut app, item.center(), menu);
        assert!(app.ui.dialog.is_none(), "Action Options… does not open for Last's action");
    }

    /// `ui.actionSet.delete` moves the panel's selected action down with its set, and clears it
    /// when its set is deleted: a new set made at that index, and the action recorded into it, are
    /// not selected, so the panel menu's Action Options… stays off.
    #[test]
    fn deleting_sets_moves_or_clears_the_selected_action() {
        use super::super::tests_appearance::{click, frame_events};
        let selected = |ctx: &egui::Context| pstate::<Option<(usize, usize)>>(ctx, "actions-selected");
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        app.ui.action_set = Some(1);
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((1, 1)));
        app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
        assert_eq!(selected(&ctx), Some((0, 1)), "Nudge Again is still selected, at its set's new index");
        app.ui.action_sets.push(ActionSet::new("Last", vec![]));
        app.ui.action_set = Some(2);
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((2, 0)));
        app.ui.action_sets[2].actions.push(Action::new("Spin", vec![]));
        app.run(DELETE_SETS, json!({"sets": [2]})).unwrap();
        assert_eq!(selected(&ctx), None, "the deleted set's action is no longer selected");
        crate::dialogs::action_set_options::open_new(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.ui.action_set, Some(2), "the new set is current, at the deleted set's index");
        app.ui.action_sets[2].actions.push(Action::new("Recorded", vec![]));
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Action Options…").map(|(_, r)| *r).expect("Action Options… in the menu");
        click(&ctx, &mut app, item.center(), menu);
        assert!(app.ui.dialog.is_none(), "Action Options… does not open for the new set's action");
    }

    /// An app with the default set and the sets Mine (two actions) and Other.
    fn app_with_sets() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let action = |name: &str| Action::new(name, vec![("object.move".into(), json!({"dx": 1, "dy": 0}))]);
        app.ui.action_sets.push(ActionSet::new("Mine", vec![action("Nudge"), action("Nudge Again")]));
        app.ui.action_sets.push(ActionSet::new("Other", vec![]));
        app
    }

    fn set_names(app: &VectorcraftApp) -> Vec<String> {
        app.ui.action_sets.iter().map(|s| s.name.clone()).collect()
    }

    /// The bottom bar's Delete Selection button (stop, record, play, new set, new action, delete).
    fn trash(ctx: &egui::Context, app: &mut VectorcraftApp) -> egui::Pos2 {
        super::super::tests_appearance::frame_events(ctx, app, vec![], show);
        let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [_, _, _, _, _, delete] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        delete.center()
    }

    /// A click on the name of set `name` with `modifiers` held.
    fn click_set(ctx: &egui::Context, app: &mut VectorcraftApp, name: &str, modifiers: egui::Modifiers) {
        use super::super::tests_appearance::{frame_events, text_rect};
        let pos = text_rect(&frame_events(ctx, app, vec![], show), &format!("▾ {name}")).center();
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
            frame_events(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), e], show);
        }
    }

    /// The question Delete Selection asks about sets, with the sets OK deletes.
    fn asked(app: &VectorcraftApp) -> Option<(String, Value)> {
        let d = app.ui.dialog.as_ref()?;
        (d.kind == crate::dialogs::confirm::KIND && d.str("__command") == DELETE_SETS)
            .then(|| (d.str("message"), d.fields["__params"]["sets"].clone()))
    }

    /// Delete Selection with a set selected asks first: Cancel keeps every set, OK deletes the
    /// selected set with its actions and keeps the others.
    #[test]
    fn delete_selection_asks_and_then_deletes_the_selected_set() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        let (message, sets) = asked(&app).expect("Delete Selection asks before deleting a set");
        assert_eq!((message.as_str(), sets), ("Delete the action set “Mine” and its actions?", json!([1])));
        crate::dialogs::cancel(&mut app);
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "Cancel deletes nothing");
        click(&ctx, &mut app, delete, show);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Other"], "OK deletes the selected set and its actions");
        assert_eq!(app.ui.action_set, None, "no set is current after the current one is deleted");
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert!(app.ui.dialog.is_none(), "nothing selected: nothing to ask about");
    }

    /// The panel menu's Delete asks the same question about the selected set, and deletes a
    /// selected action at once, as the bottom bar's button does.
    #[test]
    fn the_panel_menus_delete_deletes_the_selected_set_or_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let delete = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            let item = texts.iter().find(|(s, _)| s.trim() == "Delete").map(|(_, r)| *r).expect("Delete in the menu");
            click(ctx, app, item.center(), menu);
        };
        click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
        delete(&ctx, &mut app);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([2])));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Mine"]);
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        delete(&ctx, &mut app);
        assert!(app.ui.dialog.is_none(), "an action is deleted without a question");
        let actions = app.ui.action_sets[1].actions.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
        assert_eq!((set_names(&app).len(), actions), (2, vec!["Nudge Again"]), "the action goes, its set stays");
    }

    /// After New Set makes another set current, the panel menu's Delete asks about that set and
    /// leaves the action selected before it.
    #[test]
    fn delete_after_new_set_targets_the_new_set() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        crate::dialogs::action_set_options::open_new(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Delete").map(|(_, r)| *r).expect("Delete in the menu");
        click(&ctx, &mut app, item.center(), menu);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([3])), "the new set, not the action");
        assert_eq!(app.ui.action_sets[1].actions.len(), 2);
    }

    /// Shift-click on a set's name selects the sets from the current one to it, and one question
    /// deletes them all; the set left out stays.
    #[test]
    fn shift_click_selects_sets_that_one_question_deletes() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        click_set(&ctx, &mut app, "Other", egui::Modifiers::SHIFT);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(1), vec![1, 2]), "the current set stays");
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app), Some(("Delete the selected action sets and their actions?".to_string(), json!([1, 2]))));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions"]);
        // A plain click selects one set again.
        app.ui.action_sets.extend([ActionSet::new("A", vec![]), ActionSet::new("B", vec![])]);
        click_set(&ctx, &mut app, "A", egui::Modifiers::NONE);
        click_set(&ctx, &mut app, "Default Actions", egui::Modifiers::SHIFT);
        click_set(&ctx, &mut app, "B", egui::Modifiers::NONE);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(2), vec![2]));
    }

    /// The question holds the ids of the sets it asks about: when a set before the asked one is
    /// deleted while it is open, OK deletes the asked set at its new index, not the set that moved
    /// into the asked index.
    #[test]
    fn ok_deletes_the_asked_set_after_an_earlier_set_is_deleted_under_the_question() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        app.ui.action_sets.push(ActionSet::new("Third", vec![]));
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([2])));
        let id = app.ui.action_sets[2].id;
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["__params"]["ids"].clone()), Some(json!([id])));
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Other", "Third"]);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Third"], "OK deletes Other, the asked set");
        let p = json!({"sets": [0], "ids": [id]});
        assert_eq!(app.run(DELETE_SETS, p).unwrap_err(), "the action sets changed; nothing was deleted");
        assert_eq!(set_names(&app), ["Default Actions", "Third"], "an id that is gone deletes nothing, sets is not read");
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

    /// Shift-click on another set's name while an action is selected selects the sets from the
    /// current one to it and clears the action.
    #[test]
    fn shift_click_on_a_set_with_an_action_selected_selects_the_range() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((1, 0)));
        click_set(&ctx, &mut app, "Other", egui::Modifiers::SHIFT);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(1), vec![1, 2]));
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), None, "the action is no longer selected");
    }

    /// New Action in the current set of a Shift-selected range leaves that set as the only selected
    /// one, so Delete Selection does not ask about the rest of the range afterwards.
    #[test]
    fn new_action_in_the_current_set_clears_the_selected_range() {
        let mut app = app_with_sets();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        click_set(&ctx, &mut app, "Other", egui::Modifiers::SHIFT);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(1), vec![1, 2]));
        app.run(NEW_ACTION, json!({"set": 1, "name": "Fresh"})).unwrap();
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(1), vec![1]));
        assert_eq!(selected_sets(&app), [1]);
    }

    /// The default set can be deleted, and so can the last set: the panel is then empty, no set is
    /// current, and New Set makes a set again.
    #[test]
    fn deleting_the_default_and_the_last_set_leaves_an_empty_panel() {
        use super::super::tests_appearance::click;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Default Actions", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.action_sets.is_empty());
        assert_eq!(app.ui.action_set, None);
        crate::dialogs::action_set_options::open_new(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((set_names(&app), app.ui.action_set), (vec!["Set 1".to_string()], Some(0)), "New Set works on an empty panel");
    }

    /// `ui.actionSet.delete` keeps a recording into a later set going in the same set, stops one
    /// into a deleted set, and refuses an index past the last set.
    #[test]
    fn deleting_sets_keeps_a_recording_in_its_set() {
        let mut app = app_with_sets();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.ui.action_set = Some(2);
        let start = app.session.journal.len();
        app.session.journal.hold(Some(start));
        app.ui.recording = Some((2, "Action 1".into(), start));
        assert_eq!(app.run(DELETE_SETS, json!({"sets": [1]})).unwrap(), json!({"deleted": 1}));
        assert_eq!(set_names(&app), ["Default Actions", "Other"]);
        assert_eq!((app.ui.recording.as_ref().map(|r| r.0), app.ui.action_set), (Some(1), Some(1)), "still recording into Other");
        assert_eq!(app.run(DELETE_SETS, json!({"sets": [5]})).unwrap_err(), "no such action set");
        assert_eq!(app.run(DELETE_SETS, json!({"sets": [4_294_967_296u64]})).unwrap_err(), "no such action set", "above u32::MAX");
        assert_eq!(set_names(&app), ["Default Actions", "Other"], "a refused index deletes nothing");
        assert_eq!(app.run(DELETE_SETS, json!({})).unwrap(), json!({"deleted": 1}), "the current set by default");
        assert_eq!(set_names(&app), ["Default Actions"]);
        assert!(app.ui.recording.is_none(), "the recording into the deleted set stops");
        assert_eq!(app.session.journal.held(), None);
    }

    /// `ui.actionSet.delete` while Set Options is open on a later set: OK renames that set, not
    /// the one that moves into its old index.
    #[test]
    fn set_options_renames_its_own_set_after_an_earlier_set_is_deleted() {
        let mut app = app_with_sets();
        crate::dialogs::action_set_options::open(&mut app, 2);
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        app.ui.dialog.as_mut().expect("Set Options stays open").fields.insert("name".into(), json!("Web"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Web"], "the set that was Other is renamed");
    }

    /// `ui.actionSet.delete` closes a Batch dialog open on an action of a deleted set, and keeps
    /// one open on a later set pointing at that set.
    #[test]
    fn deleting_a_set_closes_its_batch_dialog() {
        let mut app = app_with_sets();
        crate::dialogs::action_batch::open(&mut app, 1, 0).unwrap();
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert!(app.ui.dialog.is_none(), "the Batch dialog of the deleted set closes");
        let mut app = app_with_sets();
        crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();
        app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
        let d = app.ui.dialog.as_ref().expect("the Batch dialog of a kept set stays open");
        assert_eq!((d.fields["set"].clone(), d.fields["action"].clone()), (json!(0), json!(1)), "it follows Mine to index 0");
    }

    /// A click on the panel menu's Duplicate.
    fn menu_duplicate(ctx: &egui::Context, app: &mut VectorcraftApp) {
        use super::super::tests_appearance::{click, frame_events};
        let texts = frame_events(ctx, app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Duplicate").map(|(_, r)| *r).expect("Duplicate in the menu");
        click(ctx, app, item.center(), menu);
    }

    fn action_names(app: &VectorcraftApp, set: usize) -> Vec<String> {
        app.ui.action_sets[set].actions.iter().map(|a| a.name.clone()).collect()
    }

    /// The panel menu's Duplicate copies the selected action to right after it in its set, named
    /// with " copy" added and with steps of its own, and selects the copy.
    #[test]
    fn the_panel_menus_duplicate_copies_the_selected_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        menu_duplicate(&ctx, &mut app);
        assert_eq!(action_names(&app, 1), ["Nudge", "Nudge copy", "Nudge Again"], "the copy goes right after the original");
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "no set is added");
        assert_eq!(app.ui.action_sets[1].actions[1].steps, app.ui.action_sets[1].actions[0].steps);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((1, 1)), "the copy is selected");
        frame_events(&ctx, &mut app, vec![], show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((1, 1)), "and stays selected in the panel");
        app.ui.action_sets[1].actions[1].steps[0].1 = json!({"dx": 5, "dy": 0});
        assert_eq!(app.ui.action_sets[1].actions[0].steps[0].1, json!({"dx": 1, "dy": 0}), "the original's steps are its own");
    }

    /// The panel menu's Duplicate with a set selected copies the set and all its actions to right
    /// after it, with an id of its own, makes the copy current, and the copy is saved with the sets.
    #[test]
    fn the_panel_menus_duplicate_copies_the_current_set() {
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        menu_duplicate(&ctx, &mut app);
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Mine copy", "Other"]);
        assert_eq!(action_names(&app, 2), ["Nudge", "Nudge Again"], "every action is copied");
        assert_ne!(app.ui.action_sets[2].id, app.ui.action_sets[1].id, "the copy has an id of its own");
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(2), vec![2]), "the copy is current");
        app.ui.action_sets[2].actions[0].steps.clear();
        assert_eq!(app.ui.action_sets[1].actions[0].steps.len(), 1, "the original's steps are its own");
        let saved = serde_json::to_value(&app.ui).unwrap();
        let restored = serde_json::from_value::<crate::state::UiState>(saved).unwrap();
        assert_eq!(restored.action_sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Default Actions", "Mine", "Mine copy", "Other"]);
    }

    /// The panel menu's Duplicate does nothing while no action and no set is selected.
    #[test]
    fn the_panel_menus_duplicate_needs_a_selected_action_or_set() {
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        menu_duplicate(&ctx, &mut app);
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"]);
        assert_eq!(app.ui.action_sets.iter().map(|s| s.actions.len()).collect::<Vec<_>>(), [6, 2, 0]);
        assert_eq!(app.ui.action_set, None);
    }

    /// `ui.actionSet.duplicate` copies an action or a set by index, the current set by default,
    /// refuses indexes past the last set or action, and keeps a recording and an open Batch dialog
    /// on their own set and action.
    #[test]
    fn the_duplicate_command_copies_by_index_and_keeps_recordings_and_dialogs_in_place() {
        let mut app = app_with_sets();
        assert_eq!(app.run(DUPLICATE, json!({})).unwrap_err(), "no action set is current");
        assert_eq!(app.run(DUPLICATE, json!({"set": 3})).unwrap_err(), "no such action set");
        assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 2})).unwrap_err(), "no such action");
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "a refused index copies nothing");
        crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();
        assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 0})).unwrap(), json!({"set": 1, "action": 1}));
        assert_eq!(action_names(&app, 1), ["Nudge", "Nudge copy", "Nudge Again"]);
        let d = app.ui.dialog.as_ref().expect("the Batch dialog stays open");
        assert_eq!((d.fields["set"].clone(), d.fields["action"].clone()), (json!(1), json!(2)), "it follows Nudge Again");
        app.ui.dialog = None;
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let start = app.session.journal.len();
        app.session.journal.hold(Some(start));
        app.ui.recording = Some((2, "Action 1".into(), start));
        crate::dialogs::action_set_options::open(&mut app, 2);
        app.ui.action_set = Some(0);
        assert_eq!(app.run(DUPLICATE, json!({})).unwrap(), json!({"set": 1}), "the current set by default");
        assert_eq!(set_names(&app), ["Default Actions", "Default Actions copy", "Mine", "Other"]);
        assert_eq!(app.ui.recording.as_ref().map(|r| r.0), Some(3), "still recording into Other");
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["set"].clone()), Some(json!(3)), "Set Options still on Other");
    }

    /// A recording keeps its steps when a canceled undo group drops journal entries made before the
    /// recording began.
    #[test]
    fn a_recording_keeps_its_steps_when_a_canceled_undo_group_drops_earlier_entries() {
        use super::super::tests_appearance::{click, frame_events};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let rect = |x: i32| json!({"x": x, "y": 10, "width": 20, "height": 20});
        for (id, p) in [
            ("file.new", json!({"width": 200, "height": 100})),
            ("file.new", json!({"width": 200, "height": 100})),
            ("document.activate", json!({"index": 0})),
            ("edit.undoGroup.begin", json!({})),
            ("shape.rectangle", rect(10)),
        ] {
            app.run(id, p).unwrap();
        }
        let ctx = egui::Context::default();
        frame_events(&ctx, &mut app, vec![], show);
        // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, new action,
        // delete.
        let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [stop, record, _, _, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        click(&ctx, &mut app, record.center(), show);
        for (id, p) in [("document.activate", json!({"index": 1})), ("shape.rectangle", rect(50)), ("edit.undoGroup.end", json!({"cancel": true}))] {
            app.run(id, p).unwrap();
        }
        click(&ctx, &mut app, stop.center(), show);
        let steps = app.ui.action_sets[0].actions.last().map(|a| a.steps.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
        assert_eq!(steps, Some(vec!["document.activate".to_string(), "shape.rectangle".into(), "edit.undoGroup.end".into()]));
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
            let steps = app.session.journal_for_action(start);
            app.ui.action_sets[0].actions.push(Action::new(name, steps));
            app.ui.action_sets[0].actions.len() - 1
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
        let center = app.ui.action_sets[0].actions.iter().position(|a| a.name == "Center on Artboard").unwrap();
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
            let steps = app.session.journal_for_action(start);
            app.ui.action_sets[0].actions.push(Action::new(name, steps));
            app.ui.action_sets[0].actions.len() - 1
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

    /// Files in memory for `app`: what it writes, and what it reads back. Save dialogs pick
    /// `/sets/` and the suggested name, open dialogs pick `open`.
    fn with_files(app: &mut VectorcraftApp, open: &str) -> std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<String, Vec<u8>>>> {
        let files = std::rc::Rc::new(std::cell::RefCell::new(std::collections::BTreeMap::<String, Vec<u8>>::new()));
        let (w, r) = (files.clone(), files.clone());
        app.services.write = Some(Box::new(move |path: &str, bytes: &[u8]| {
            w.borrow_mut().insert(path.to_string(), bytes.to_vec());
            Ok(())
        }));
        app.services.read = Some(Box::new(move |path: &str| r.borrow().get(path).cloned().ok_or_else(|| "no such file".to_string())));
        app.services.pick_save = Some(Box::new(|pick: &crate::FilePick| Some(format!("/sets/{}", pick.name))));
        let open = open.to_string();
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(open.clone())));
        files
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
                json!("{\"format\": \"vcactions\", \"version\": 7, \"name\": \"S\", \"actions\": []}"),
                "the action set file needs a later version of VectorCraft",
            ),
        ] {
            assert_eq!(app.run(LOAD_SET, json!({ "data": data })).unwrap_err(), want);
        }
        with_files(&mut app, "/sets/missing.vcactions");
        assert_eq!(app.run(LOAD_SET, json!({})).unwrap_err(), "no such file");
        assert_eq!(set_names(&app), before, "no set added");
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

    /// An action set as another app saves it as text, written from scratch: the set `name` (none
    /// when empty) and actions of steps, each a label and whether it is on, with no recorded values.
    fn aia_text(name: &str, actions: &[(&str, &[(&str, bool)])]) -> String {
        let hex = |t: &str| format!("[ {} {} ]", t.len(), t.bytes().map(|b| format!("{b:02x}")).collect::<String>());
        let mut out = String::from("/version 3\n");
        if !name.is_empty() {
            out += &format!("/name {}\n", hex(name));
        }
        out += &format!("/isOpen 1\n/actionCount {}\n", actions.len());
        for (i, (action, steps)) in actions.iter().enumerate() {
            out += &format!("/action-{} {{\n\t/name {}\n\t/eventCount {}\n", i + 1, hex(action), steps.len());
            for (j, (label, on)) in steps.iter().enumerate() {
                out += &format!(
                    "\t/event-{} {{\n\t\t/internalName (sample_step_{j})\n\t\t/localizedName {}\n\t\t/isOn {}\n\t\t/parameterCount 0\n\t}}\n",
                    j + 1,
                    hex(label),
                    u8::from(*on)
                );
            }
            out += "}\n";
        }
        out
    }

    /// Load Actions… reads an action set another app saved as text: its actions come in with the
    /// steps with no recorded values whose label is a VectorCraft command's that takes no values and
    /// has no dialog, a step that is off stays excluded from playback, the other steps are left out
    /// and listed, and the status bar says how many. An action plays the steps that are on.
    #[test]
    fn load_actions_reads_another_apps_action_set() {
        let mut app = app_with_sets();
        let text = aia_text(
            "Tidy Up",
            &[("Group Them", &[("group", true), ("Move", true), ("Ungroup", false)]), ("Nothing", &[]), ("", &[("Ungroup", true)])],
        );
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

    /// The bottom bar's stop and Create New Action buttons (stop, record, play, new set, new
    /// action, delete).
    fn stop_and_new_action(ctx: &egui::Context, app: &mut VectorcraftApp) -> (egui::Pos2, egui::Pos2) {
        super::super::tests_appearance::frame_events(ctx, app, vec![], show);
        let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [stop, _, _, _, new_action, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        (stop.center(), new_action.center())
    }

    /// The texts the dialog layer draws once the open dialog has settled.
    fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut texts = vec![];
        for _ in 0..3 {
            texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], |app, ui| crate::dialogs::show(app, ui.ctx()));
        }
        texts.into_iter().map(|(t, _)| t).collect()
    }

    /// The Create New Action button and the panel menu's New Action… both open New Action with a
    /// suggested name in the current set, no function key and no color; Cancel adds nothing and
    /// starts no recording. Record with another set chosen adds the action there, selects it and
    /// records into it, and Stop keeps exactly the steps recorded in that action; Record and Stop
    /// with nothing done in between keep an empty action.
    #[test]
    fn new_action_asks_for_a_name_and_set_and_records_into_the_new_action() {
        use super::super::tests_appearance::{click, frame_events};
        let mut app = app_with_sets();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        let (_, new_action) = stop_and_new_action(&ctx, &mut app);
        click(&ctx, &mut app, new_action, show);
        let d = app.ui.dialog.as_ref().expect("Create New Action opens New Action");
        assert_eq!(d.kind, crate::dialogs::action_new::KIND);
        let fields = (d.fields["set"].clone(), d.str("name"), d.fields["key"].clone(), d.str("color"));
        assert_eq!(fields, (json!(1), "Action 3".to_string(), json!(0), String::new()), "the current set, a free name, no key or color");
        crate::dialogs::cancel(&mut app);
        let counts = |app: &VectorcraftApp| app.ui.action_sets.iter().map(|s| s.actions.len()).collect::<Vec<_>>();
        assert_eq!(counts(&app), [6, 2, 0], "Cancel adds nothing");
        assert!(app.ui.recording.is_none(), "and records nothing");
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "New Action…").map(|(_, r)| *r).expect("New Action… in the menu");
        click(&ctx, &mut app, item.center(), menu);
        let d = app.ui.dialog.as_mut().expect("New Action… opens New Action");
        assert_eq!(d.kind, crate::dialogs::action_new::KIND);
        d.fields.insert("set".into(), json!(2));
        d.fields.insert("name".into(), json!("  Box  "));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(action_names(&app, 2), ["Box"], "added to the set chosen, with the name trimmed");
        let selected = pstate::<Option<(usize, usize)>>(&ctx, "actions-selected");
        assert_eq!((app.ui.action_set, selected), (Some(2), Some((2, 0))), "its set is current and it is selected");
        assert!(app.ui.recording.is_some(), "Record starts recording");
        assert!(frame_events(&ctx, &mut app, vec![], show).iter().any(|(t, _)| t == "● Recording"), "the panel shows the recording");
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let (stop, _) = stop_and_new_action(&ctx, &mut app);
        click(&ctx, &mut app, stop, show);
        assert!(app.ui.recording.is_none());
        let steps: Vec<String> = app.ui.action_sets[2].actions[0].steps.iter().map(|(c, _)| c.clone()).collect();
        assert_eq!(steps, ["shape.rectangle"], "the recorded steps, in the new action");
        assert_eq!(counts(&app), [6, 2, 1], "Stop adds no other action");
        app.run(NEW_ACTION, json!({"set": 2, "name": "Empty"})).unwrap();
        let (stop, _) = stop_and_new_action(&ctx, &mut app);
        click(&ctx, &mut app, stop, show);
        assert_eq!(action_names(&app, 2), ["Box", "Empty"]);
        assert!(app.ui.action_sets[2].actions[1].steps.is_empty(), "nothing recorded: an empty action");
    }

    /// New Action shows Name, Action Set, Function Key with Ctrl (Command on macOS) and Shift, Color and Record.
    #[test]
    fn new_action_shows_its_fields() {
        let mut app = app_with_sets();
        crate::dialogs::action_new::open(&mut app).unwrap();
        let texts = dialog_texts(&mut app);
        for label in ["New Action", "Name:", "Action Set:", "Default Actions", "Function Key:", "Shift", "Color:", "None", "Record", "Cancel"] {
            assert!(texts.iter().any(|t| t == label), "{label}: {texts:?}");
        }
        assert!(!texts.iter().any(|t| t == "Replace"));
        crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Shift+F14");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("key".into(), json!(14));
        d.fields.insert("shift".into(), json!(true));
        let texts = dialog_texts(&mut app);
        assert!(!texts.iter().any(|s| s == "Replace" || s.contains("already used")), "{texts:?}");
    }

    /// `ui.action.new` refuses an index past the last set, an empty name, a key or color that is
    /// not one, and a second recording, and then adds nothing. New Action does not open without a
    /// set or while recording, follows its set when an earlier set is deleted, and closes when its
    /// set is deleted. On Windows F1, Ctrl+F4 and Ctrl+F6 are refused.
    #[test]
    fn new_action_refuses_bad_params_and_adds_nothing() {
        let mut app = app_with_sets();
        let counts = |app: &VectorcraftApp| app.ui.action_sets.iter().map(|s| s.actions.len()).collect::<Vec<_>>();
        for (p, want) in [
            (json!({"set": 3, "name": "A"}), "no such action set"),
            (json!({"name": "A"}), "no such action set"),
            (json!({"set": 1, "name": "  "}), "name is empty"),
            (json!({"set": 1}), "name is empty"),
            (json!({"set": 1, "name": "A", "key": 16}), "no such function key"),
            (json!({"set": 1, "name": "A", "key": "F5"}), "no such function key"),
            (json!({"set": 1, "name": "A", "color": "pink"}), "no such color"),
        ] {
            assert_eq!(app.run(NEW_ACTION, p.clone()).unwrap_err(), want, "{p}");
        }
        if crate::keymap::platform() == craft_keymap::Platform::Windows {
            for (key, cmd) in [(1, false), (1, true), (4, true), (6, true)] {
                let want = format!("{} cannot be used on Windows", FunctionKey { key, cmd, shift: false }.text());
                assert_eq!(app.run(NEW_ACTION, json!({"set": 1, "name": "A", "key": key, "cmd": cmd})).unwrap_err(), want);
            }
        }
        assert_eq!(counts(&app), [6, 2, 0], "nothing added");
        assert!(app.ui.recording.is_none(), "nothing recording");
        // The dialog follows its set down, and closes with it.
        crate::dialogs::action_new::open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("set".into(), json!(2));
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["set"].clone()), Some(json!(1)), "New Action follows Other");
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert!(app.ui.dialog.is_none(), "New Action closes with its set");
        crate::dialogs::action_new::open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("set".into(), json!(9));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some(), "a set that is gone: the dialog stays open");
        app.ui.dialog = None;
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run(NEW_ACTION, json!({"set": 0, "name": "A"})).unwrap();
        assert_eq!(app.run(NEW_ACTION, json!({"set": 0, "name": "B"})).unwrap_err(), "an action is already recording");
        assert_eq!(crate::dialogs::action_new::open(&mut app).unwrap_err(), "an action is already recording");
        assert!(!crate::dialogs::action_new::can_open(&app));
        assert_eq!(counts(&app), [7], "one action added");
        app.ui.action_sets.clear();
        app.ui.recording = None;
        assert_eq!(crate::dialogs::action_new::open(&mut app).unwrap_err(), "no such action set");
        assert!(app.ui.dialog.is_none());
    }

    /// A recording that New Action started goes into its action wherever the action is when Stop
    /// is pressed, and is dropped when the action was deleted meanwhile.
    #[test]
    fn a_new_actions_recording_follows_its_action() {
        let mut app = app_with_sets();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        app.run(NEW_ACTION, json!({"set": 2, "name": "Box"})).unwrap();
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let (stop, _) = stop_and_new_action(&ctx, &mut app);
        super::super::tests_appearance::click(&ctx, &mut app, stop, show);
        assert_eq!((action_names(&app, 1), app.ui.action_sets[1].actions[0].steps.len()), (vec!["Box".to_string()], 1));
        app.run(NEW_ACTION, json!({"set": 1, "name": "Gone"})).unwrap();
        app.ui.action_sets[1].actions.pop();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let (stop, _) = stop_and_new_action(&ctx, &mut app);
        super::super::tests_appearance::click(&ctx, &mut app, stop, show);
        assert_eq!(action_names(&app, 1), ["Box"], "the deleted action is not made again");
        assert!(app.ui.recording.is_none() && app.ui.recording_into.is_none());
        // `ui.action.stop` ends a recording as the button does.
        assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": false}), "nothing recording");
        app.run(NEW_ACTION, json!({"set": 1, "name": "Remote"})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
        assert_eq!((action_names(&app, 1), app.ui.action_sets[1].actions[1].steps.len()), (vec!["Box".to_string(), "Remote".into()], 1));
        assert!(app.ui.recording.is_none());
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
        let mut output =
            ctx.run_ui(egui::RawInput { events: vec![repeated], ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
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

    /// `ui.action.play` plays an action by its name, in the set named when one is, and refuses a
    /// name that more than one action has, or that none has.
    #[test]
    fn play_action_by_name_refuses_a_name_more_than_one_action_has() {
        let mut app = app_with_sets();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.ui.action_sets[2].actions.push(Action::new("Nudge", vec![("object.move".into(), json!({"dx": 5, "dy": 0}))]));
        let id = app.session.active().unwrap().selection.objects.clone();
        let x = |app: &VectorcraftApp| app.session.active().unwrap().doc.bounds_of(&id, false).unwrap().x0;
        let before = x(&app);
        assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nudge"})).unwrap_err(), "more than one action is named “Nudge”");
        assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nope"})).unwrap_err(), "no such action");
        assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nudge", "set": "Missing"})).unwrap_err(), "no such action");
        assert_eq!(app.run(PLAY_ACTION, json!({})).unwrap_err(), "no such action");
        assert_eq!(app.run(PLAY_ACTION, json!({"action": "Nudge Again", "set": 0})).unwrap_err(), "no such action set");
        assert_eq!(x(&app), before, "nothing played");
        app.run(PLAY_ACTION, json!({"action": "Nudge", "set": "Other"})).unwrap();
        assert_eq!(x(&app), before + 5.0, "Other's Nudge played");
        app.run(PLAY_ACTION, json!({"action": "Nudge Again"})).unwrap();
        assert_eq!(x(&app), before + 6.0, "a name one action has plays without a set");
    }

    /// Action Options shows the action's function key and color; OK sets them and keeps the name's
    /// steps, Cancel keeps them as they were. A rename keeps them, and a copy has no key.
    #[test]
    fn action_options_sets_the_function_key_and_color_and_keeps_the_steps() {
        use crate::dialogs::action_set_options::open_action;
        let mut app = app_with_sets();
        open_action(&mut app, 1, 1);
        let texts = dialog_texts(&mut app);
        for label in ["Action Options", "Name:", "Function Key:", "Shift", "Color:"] {
            assert!(texts.iter().any(|t| t == label), "{label}: {texts:?}");
        }
        let set = |app: &mut VectorcraftApp| {
            let d = app.ui.dialog.as_mut().expect("Action Options is open");
            assert_eq!((d.fields["key"].clone(), d.bool("shift"), d.str("color")), (json!(0), false, String::new()));
            d.fields.insert("key".into(), json!(15));
            d.fields.insert("shift".into(), json!(true));
            d.fields.insert("color".into(), json!("green"));
        };
        set(&mut app);
        crate::dialogs::cancel(&mut app);
        let a = |app: &VectorcraftApp, i: usize| app.ui.action_sets[1].actions[i].clone();
        assert_eq!((a(&app, 1).key, a(&app, 1).color), (None, None), "Cancel keeps them");
        open_action(&mut app, 1, 1);
        set(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        let shift_f15 = FunctionKey { key: 15, cmd: false, shift: true };
        assert_eq!((a(&app, 1).name, a(&app, 1).key, a(&app, 1).color), ("Nudge Again".to_string(), Some(shift_f15), Some("green".to_string())));
        assert_eq!(a(&app, 1).steps, [("object.move".to_string(), json!({"dx": 1, "dy": 0}))], "the steps stay");
        open_action(&mut app, 1, 1);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.fields["key"].clone(), d.bool("shift"), d.str("color")), (json!(15), true, "green".to_string()), "shown when it opens again");
        assert!(dialog_texts(&mut app).iter().any(|t| t == "Green"));
        app.ui.dialog = None;
        app.run(RENAME_ACTION, json!({"set": 1, "action": 1, "name": "Push"})).unwrap();
        assert_eq!((a(&app, 1).key, a(&app, 1).color.as_deref()), (Some(shift_f15), Some("green")), "a rename keeps them");
        app.run(DUPLICATE, json!({"set": 1, "action": 1})).unwrap();
        assert_eq!(
            (a(&app, 2).name.as_str(), a(&app, 2).key, a(&app, 2).color.as_deref()),
            ("Push copy", None, Some("green")),
            "the copy has no key"
        );
        assert_ne!(a(&app, 2).id, a(&app, 1).id);
        assert_eq!(a(&app, 1).key, Some(shift_f15), "the original keeps it");
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "key": 0, "color": null})).unwrap();
        assert_eq!((a(&app, 1).name.as_str(), a(&app, 1).key, a(&app, 1).color.clone()), ("Push", None, None), "cleared; the name stays");
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "key": 15, "cmd": true, "shift": true})).unwrap();
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "key": 14})).unwrap();
        assert_eq!(a(&app, 1).key, Some(FunctionKey { key: 14, cmd: true, shift: true }), "a new key keeps the modifiers left out");
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "shift": false})).unwrap();
        assert_eq!(a(&app, 1).key, Some(FunctionKey { key: 14, cmd: true, shift: false }), "shift alone changes Shift");
        app.run(ACTION_OPTIONS, json!({"set": 1, "action": 2, "shift": true})).unwrap();
        assert_eq!(a(&app, 2).key, None, "a modifier alone gives no key");
        for (p, want) in [
            (json!({"set": 1, "action": 9}), "no such action"),
            (json!({"set": 1, "action": 1, "name": " "}), "name is empty"),
            (json!({"set": 1, "action": 1, "color": "pink"}), "no such color"),
            (json!({"set": 1, "action": 1, "key": 99}), "no such function key"),
        ] {
            assert_eq!(app.run(ACTION_OPTIONS, p.clone()).unwrap_err(), want, "{p}");
        }
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

    /// A document with one rectangle selected and the set "Moves" with one action that moves it
    /// right three times, by 1, 2 and 4 pt → (the app, the rectangle's left edge as it moves).
    fn app_with_three_moves() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = vectorcraft_doc::NodeId(
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap(),
        );
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        let step = |dx: u32| ("object.move".to_string(), json!({"dx": dx, "dy": 0}));
        app.ui.action_sets = vec![ActionSet::new("Moves", vec![Action::new("Three Moves", vec![step(1), step(2), step(4)])])];
        let x = move |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - 10.0;
        (app, x)
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// One frame of a shown window at `now`: the logic pass, then a UI pass that draws → what the
    /// logic pass's [`tick_playback`] returns.
    fn frame(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
        let wait = tick_playback(app, now);
        playback_drawn(app);
        wait
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
        assert_eq!(
            app.run(PLAYBACK_OPTIONS, json!({"speed": "accelerated"})).unwrap(),
            json!({"speed": "accelerated", "seconds": 2}),
            "seconds kept"
        );
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
    fn the_playback_options_dialog_sets_the_speed_and_pause() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run(PLAYBACK_OPTIONS, json!({"speed": "step", "seconds": 3})).unwrap();
        crate::dialogs::action_playback::open(&mut app);
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.kind.as_str(), d.str("speed"), d.fields["seconds"].clone()), ("actionPlayback", "step".to_string(), json!(3)));
        d.fields.insert("speed".into(), json!("pause"));
        d.fields.insert("seconds".into(), json!(4));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.action_playback, PlaybackOptions { speed: PlaybackSpeed::Pause, seconds: 4 });
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
        let ctx = egui::Context::default();
        // One host iteration: `tick`, then a UI pass, in a shown or an occluded window.
        let frame = |app: &mut VectorcraftApp, hidden: bool| {
            let mut raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
            if hidden {
                raw.viewports.entry(egui::ViewportId::ROOT).or_default().occluded = Some(true);
            }
            app.begin_host_iteration();
            let mut out = ctx.run_ui(raw, |ui| {
                app.tick(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        };
        // Fonts install on the first pass and are ready on the second.
        frame(&mut app, false);
        frame(&mut app, false);
        play(&mut app, 0, 0).unwrap();
        for _ in 0..3 {
            frame(&mut app, true);
        }
        assert_eq!(x(&app), 1.0, "no step runs while the window is hidden, even in a UI pass");
        frame(&mut app, false);
        assert_eq!(x(&app), 1.0, "the shown frame draws the first step");
        frame(&mut app, true);
        assert_eq!(x(&app), 3.0, "the first step was shown, so the second runs");
        for _ in 0..3 {
            frame(&mut app, true);
        }
        assert_eq!(x(&app), 3.0, "the third step waits until the window shows the second");
        frame(&mut app, false);
        assert_eq!(x(&app), 3.0, "the shown frame draws the second step");
        frame(&mut app, false);
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
        let ctx = egui::Context::default();
        // One host iteration's `tick` at `now`.
        let tick = |app: &mut VectorcraftApp, now: f64| {
            app.now_override = Some(now);
            app.begin_host_iteration();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
            out.textures_delta.clear();
        };
        // The same, with `method` and `params` queued on the control channel → its reply.
        let iteration =
            |app: &mut VectorcraftApp, tx: &std::sync::mpsc::Sender<crate::control::ControlRequest>, now: f64, method: &str, params: Value| {
                let (req, reply) = crate::control::ControlRequest::new(method, params);
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
        assert_eq!(app.ui.action_sets[0].actions[1].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["command.batch"]);
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
        assert_eq!(app.ui.action_sets[0].actions[2].steps.len(), 2);
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
        app.ui.action_sets[0].actions[0].steps.insert(0, ("document.activate".into(), json!({"index": 0})));
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
            app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
            crate::unsaved::confirm(app).unwrap();
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
        app.ui.action_sets[0].actions[0].steps[1] = ("no.such.command".into(), json!({}));
        play(&mut app, 0, 0).unwrap();
        assert_eq!(frame(&mut app, 0.0), None, "the frame after Play draws the first step");
        assert_eq!(frame(&mut app, 0.0), None);
        assert!(app.ui.action_playing.is_none());
        assert!(!app.ui.status.is_empty(), "the step's error shows");
        assert_eq!(frame(&mut app, 0.0), None);
        assert_eq!(x(&app), 1.0, "the step after the failing one does not run");
    }

    /// [`app_with_three_moves`] with a stop after the first move.
    fn app_with_a_stop(allow_continue: bool) -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
        let (mut app, x) = app_with_three_moves();
        let params = json!({"set": 0, "action": 0, "step": 0, "message": "Paint it", "allowContinue": allow_continue});
        assert_eq!(app.run(INSERT_STOP, params).unwrap(), json!({"set": 0, "action": 0, "step": 1}));
        (app, x)
    }

    fn open_kind(app: &VectorcraftApp) -> Option<&str> {
        app.ui.dialog.as_ref().map(|d| d.kind.as_str())
    }

    #[test]
    fn insert_stop_goes_after_the_given_step_or_at_the_end_and_keeps_exclusions() {
        let (mut app, _) = app_with_three_moves();
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 2]);
        app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0, "message": "After the first"})).unwrap();
        app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap();
        let action = &app.ui.action_sets[0].actions[0];
        let ids: Vec<&str> = action.steps.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(ids, ["object.move", STOP_STEP, "object.move", "object.move", STOP_STEP]);
        assert_eq!(action.steps[1].1, json!({"message": "After the first", "allowContinue": false}));
        assert_eq!(action.steps[4].1, json!({"message": "", "allowContinue": false}), "the message and Allow Continue are optional");
        assert_eq!(action.excluded_steps, BTreeSet::from([0, 3]), "the step excluded after the insertion point moved with it");

        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([1, usize::MAX]);
        app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([2]), "an exclusion past the last step is dropped");
    }

    #[test]
    fn insert_stop_refuses_bad_params_and_changes_nothing() {
        let (mut app, _) = app_with_three_moves();
        for (params, want) in [
            (json!({"message": "m"}), "no such action"),
            (json!({"set": 0}), "no such action"),
            (json!({"action": 0}), "no such action"),
            (json!({"step": 0}), "no such action"),
            (json!({"set": 1, "action": 0}), "no such action"),
            (json!({"set": 0, "action": 1}), "no such action"),
            (json!({"set": "0", "action": 0}), "no such action"),
            (json!({"set": 0, "action": null}), "no such action"),
            (json!({"set": 0, "action": 0, "step": 3}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": -1}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": null}), "no such action step"),
            (json!({"set": 0, "action": 0, "message": 5}), "message must be text"),
            (json!({"set": 0, "action": 0, "message": null}), "message must be text"),
            (json!({"set": 0, "action": 0, "allowContinue": "yes"}), "allowContinue must be true or false"),
            (json!(5), "`params` must be an object"),
            (json!([1]), "`params` must be an object"),
            (json!("message"), "`params` must be an object"),
            (json!(null), "`params` must be an object"),
        ] {
            assert_eq!(app.run(INSERT_STOP, params.clone()).unwrap_err(), want, "{params}");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "{params}");
        }
        app.run(START_RECORDING, json!({})).unwrap();
        assert_eq!(app.run(INSERT_STOP, json!({"message": 5})).unwrap_err(), "message must be text");
        for params in [json!(5), json!([1]), json!(true), json!(null)] {
            assert_eq!(app.run(INSERT_STOP, params.clone()).unwrap_err(), "`params` must be an object", "{params}");
        }
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert!(app.ui.action_sets[0].actions.last().unwrap().steps.is_empty(), "the refused stop is not recorded");
    }

    #[test]
    fn insert_menu_item_goes_after_the_given_step_or_at_the_end_runs_nothing_and_plays_as_the_menu_runs_it() {
        let (mut app, x) = app_with_three_moves();
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 2]);
        let doc = app.session.active().unwrap().doc.clone();
        let journal = app.session.journal.len();
        let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "object.group"})).unwrap();
        assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
        let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "type.changeCase", "params": {"case": "upper"}})).unwrap();
        assert_eq!(r, json!({"set": 0, "action": 0, "step": 4}));
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "inserting runs nothing");
        assert_eq!(app.session.journal.len(), journal, "and records nothing");
        let action = &app.ui.action_sets[0].actions[0];
        let ids: Vec<&str> = action.steps.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(ids, ["object.move", "object.group", "object.move", "object.move", "type.changeCase"]);
        assert_eq!((&action.steps[1].1, &action.steps[4].1), (&json!({}), &json!({"case": "upper"})));
        assert_eq!(action.excluded_steps, BTreeSet::from([0, 3]), "the step excluded after the insertion point moved with it");

        app.ui.action_sets[0].actions[0].excluded_steps.insert(4);
        let before = undo_steps(&app);
        let r = play(&mut app, 0, 0).unwrap();
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 2, "the group and the included move play, and the excluded change of case does not: {r}");
        assert!(results[0]["id"].is_u64(), "the group step made a group: {r}");
        assert_eq!(undo_steps(&app), before + 1, "the action plays as one undo step");
        assert_eq!(x(&app), 2.0, "the move of 2 moved the group");
    }

    #[test]
    fn an_inserted_menu_item_plays_step_by_step() {
        let (mut app, x) = app_with_three_moves();
        app.ui.action_sets[0].actions[0].steps.truncate(1);
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

    /// [`app_with_three_moves`] with View › Outline inserted after the first move: the moves of 1,
    /// Outline, then the moves of 2 and 4.
    fn app_with_a_view_step() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
        let (mut app, x) = app_with_three_moves();
        let doc = app.session.active().unwrap().doc.clone();
        let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "view.outline"})).unwrap();
        assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
        assert!(!app.outline_view(), "inserting runs nothing");
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
        assert_eq!(app.ui.action_sets[0].actions[0].steps[1], ("view.outline".to_string(), json!({})));
        (app, x)
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
        app.ui.action_sets[0].actions[0].steps.insert(3, ("object.align".into(), json!({})));
        assert!(play(&mut app, 0, 0).is_err());
        assert!(app.outline_view(), "Outline played");
        assert_eq!(x(&app), 1.0, "the move before Outline stays, and the move of 2 in the failing batch is taken back");
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

    /// A View menu step before a stop plays before the playback waits there, and the steps after the
    /// stop play on Continue.
    #[test]
    fn an_inserted_view_item_plays_up_to_a_stop() {
        let (mut app, x) = app_with_a_view_step();
        app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 1})).unwrap();
        let r = play(&mut app, 0, 0).unwrap();
        assert_eq!(r["atStop"], json!(true), "{r}");
        assert!(app.outline_view());
        assert_eq!(x(&app), 1.0);
        app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!(x(&app), 7.0);
        assert!(app.outline_view(), "Outline did not play again");
    }

    /// Playing an action with a View menu step while another records keeps the View step in the
    /// recording, between the batches of the steps around it.
    #[test]
    fn a_view_step_played_while_recording_is_recorded() {
        let (mut app, _) = app_with_a_view_step();
        app.run(START_RECORDING, json!({})).unwrap();
        play(&mut app, 0, 0).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = &app.ui.action_sets[0].actions.last().unwrap().steps;
        let ids: Vec<&str> = recorded.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(ids, ["command.batch", "view.outline", "command.batch"]);
        assert_eq!(recorded[1].1, json!({}));
    }

    /// Batch… refuses an action with an included View menu step, which acts on a document window,
    /// and plays one whose View menu step is excluded.
    #[test]
    fn batch_refuses_an_action_with_a_view_step() {
        let (mut app, _) = app_with_a_view_step();
        assert_eq!(crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap_err(), "Batch cannot play an action with a View menu step");
        assert!(app.ui.dialog.is_none());
        app.ui.action_sets[0].actions[0].excluded_steps.insert(1);
        crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap();
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

    /// At Accelerated, the steps up to a stop play as saved steps, as they do without a stop: a key
    /// their command no longer reads, recorded by an older version, is taken.
    #[test]
    fn steps_before_a_stop_play_with_a_key_no_longer_read() {
        let (mut app, _) = app_with_three_moves();
        let old = json!({"x": 10, "y": 50, "width": 20, "height": 20, "cornerStyle": "round"});
        assert!(app.run("shape.rectangle", old.clone()).is_err());
        let stop = (STOP_STEP.to_string(), json!({"message": "", "allowContinue": false}));
        app.ui.action_sets[0].actions[0].steps = vec![("shape.rectangle".into(), old), stop];
        let shapes = app.session.active().unwrap().doc.layers[0].children().unwrap().len();
        let r = play(&mut app, 0, 0).unwrap();
        assert_eq!(r["atStop"], json!(true), "{r}");
        assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), shapes + 1);
        assert!(!app.session.replaying());
    }

    #[test]
    fn insert_menu_item_refuses_what_no_menu_item_runs_and_changes_nothing() {
        let (mut app, _) = app_with_three_moves();
        let place = |extra: Value| {
            let mut p = json!({"set": 0, "action": 0});
            if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
                p.extend(extra.clone());
            }
            p
        };
        let none = "no menu item that can be inserted runs this command";
        for (params, want) in [
            (place(json!({})), "command must be text"),
            (place(json!({"command": 5})), "command must be text"),
            (place(json!({"command": "no.such.command"})), none),
            // Undo, a File menu item, an item that asks for values first, the app's own commands and
            // an action's own command.
            (place(json!({"command": "edit.undo"})), none),
            (place(json!({"command": "file.save"})), none),
            (place(json!({"command": "object.move"})), none),
            (place(json!({"command": "object.move", "params": {"dx": 1, "dy": 0}})), none),
            (place(json!({"command": INSERT_MENU_ITEM})), none),
            (place(json!({"command": "edit.paste"})), none),
            // The app's own commands outside the View menu, a saved-view slot and the View menu's New
            // View…, which asks for a name first.
            (place(json!({"command": "window.panel", "params": {"panel": "layers", "toggle": true}})), none),
            (place(json!({"command": "view.goto1"})), none),
            (place(json!({"command": "view.saved.new"})), none),
            // Align To is the step's own: only the menu item's params are taken.
            (place(json!({"command": "object.align", "params": {"horizontal": "left", "to": "selection"}})), none),
            // Image Trace, whose menu items trace off the UI thread with progress and Cancel.
            (place(json!({"command": "imageTrace.make", "params": {"preset": "Default"}})), none),
            (place(json!({"command": "imageTrace.makeAndExpand", "params": {"preset": "Default"}})), none),
            // Pattern's Make and Edit Pattern, whose menu items open the Pattern Options panel.
            (place(json!({"command": "object.pattern.make"})), none),
            (place(json!({"command": "object.pattern.edit"})), none),
            // Params that no menu item gives its command.
            (place(json!({"command": "type.changeCase"})), none),
            (place(json!({"command": "type.changeCase", "params": {"case": "middle"}})), none),
            (place(json!({"command": "object.group", "params": {"ids": [1]}})), none),
            (place(json!({"command": "object.group", "params": [1]})), "`params` must be an object"),
            (place(json!({"command": "object.group", "params": null})), "`params` must be an object"),
            (place(json!({"command": "object.group", "step": 3})), "no such action step"),
            (place(json!({"command": "object.group", "step": null})), "no such action step"),
            (json!({"command": "object.group"}), "no such action"),
            (json!({"command": "object.group", "set": 0}), "no such action"),
            (json!({"command": "object.group", "set": 0, "action": 1}), "no such action"),
            (json!({"command": "object.group", "set": 1, "action": 0}), "no such action"),
            (json!(5), "`params` must be an object"),
            (json!(null), "`params` must be an object"),
        ] {
            assert_eq!(app.run(INSERT_MENU_ITEM, params.clone()).unwrap_err(), want, "{params}");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "{params}");
        }
    }

    /// A listed menu item runs from the menu what its step runs in playback: the click opens no
    /// dialog, and it journals the item's own command with the params its step holds. An Object ›
    /// Align item's step holds the Align To choice that the click sends, and the app adds the active
    /// artboard to both.
    #[test]
    fn every_listed_menu_item_runs_from_the_menu_what_its_step_holds() {
        for item in crate::menus::insertable_items(false) {
            let (mut app, _) = app_with_three_moves();
            app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": item.command, "params": item.params})).unwrap();
            let held = app.ui.action_sets[0].actions[0].steps[3].1.clone();
            let start = app.session.journal.len();
            crate::menus::invoke(&mut app, item.command, item.params.clone());
            assert!(app.ui.dialog.is_none(), "{item:?} opens a dialog");
            let ran = app.session.journal_for_action(start);
            let without_artboard = |p: &Value| {
                let mut p = p.clone();
                if let Some(o) = p.as_object_mut() {
                    o.remove("artboard");
                }
                p
            };
            assert!(ran.iter().all(|(c, p)| c != item.command || without_artboard(p) == held), "{item:?} ran {ran:?}, its step holds {held}");
        }
    }

    /// Object › Align items can be inserted, and the step keeps the Align To that the click sends at
    /// insertion: a later Align To choice does not change it.
    #[test]
    fn an_inserted_align_item_keeps_the_align_to_of_its_insertion() {
        let (mut app, _, ids) = app_with_two_rectangles();
        app.run("select.set", json!({ "ids": ids })).unwrap();
        app.ui.align_to = super::super::align::AlignTo::Artboard;
        app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
        app.ui.align_to = super::super::align::AlignTo::Selection;
        app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
        let steps = &app.ui.action_sets[0].actions[0].steps;
        assert_eq!(steps[3], ("object.align".to_string(), json!({"horizontal": "left", "to": "artboard"})));
        assert_eq!(steps[4], ("object.align".to_string(), json!({"horizontal": "left", "to": "selection"})));
    }

    /// With anchor points selected, an Object › Align item sends its params unchanged, and the points
    /// align to their own bounds: its step holds the same params, without the Align To choice.
    #[test]
    fn an_inserted_align_item_with_anchors_selected_keeps_the_menu_params() {
        let (mut app, _, [first, _]) = app_with_two_rectangles();
        app.ui.align_to = super::super::align::AlignTo::Artboard;
        app.run("select.anchors", json!({"id": first, "anchors": [[0, 0], [0, 1]]})).unwrap();
        assert!(!app.session.active().unwrap().selection.anchors.is_empty());
        app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps[3], ("object.align".to_string(), json!({"horizontal": "left"})));
    }

    /// A step that aligns to the key object fails in playback when the selection has no key then:
    /// it does not align to the selection or the artboard instead, and the steps after it do not run.
    #[test]
    fn an_align_step_to_the_key_object_fails_without_a_key() {
        let (mut app, _, [first, second]) = app_with_two_rectangles();
        app.run("select.set", json!({"ids": [first, second]})).unwrap();
        app.run("select.key", json!({"id": second})).unwrap();
        app.ui.action_sets[0].actions[0].steps.clear();
        app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps[0].1["to"], json!("key"));
        app.ui.action_sets[0].actions[0].steps.push(("object.move".into(), json!({"dx": 5, "dy": 0})));
        app.run("select.set", json!({"ids": [first, second]})).unwrap();
        let before = (left_of(&app, first), left_of(&app, second));
        assert!(play(&mut app, 0, 0).unwrap_err().contains("no key object"));
        assert_eq!((left_of(&app, first), left_of(&app, second)), before, "nothing moved");
    }

    #[test]
    fn insert_menu_item_goes_into_the_recording() {
        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({})).unwrap();
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap(), json!({"recording": true}));
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "file.save"})).unwrap_err(), "no menu item that can be inserted runs this command");
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        assert_eq!(recorded.steps, [("object.group".to_string(), json!({}))]);
    }

    /// [`app_with_three_moves`] with a second rectangle, 100 pt to the right, and the first one
    /// selected again → (the app, the left edge of the first one as it moves, the ids of both).
    fn app_with_two_rectangles() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64, [u64; 2]) {
        let (mut app, x) = app_with_three_moves();
        let first = app.session.active().unwrap().selection.objects[0].0;
        let second = app.run("shape.rectangle", json!({"x": 110, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [first]})).unwrap();
        (app, x, [first, second])
    }

    fn left_of(app: &VectorcraftApp, id: u64) -> f64 {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0
    }

    #[test]
    fn insert_select_path_stores_the_selected_path_changes_nothing_and_plays_by_selecting_it() {
        let (mut app, x, [first, second]) = app_with_two_rectangles();
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 2]);
        let doc = app.session.active().unwrap().doc.clone();
        let journal = app.session.journal.len();
        let undo = undo_steps(&app);
        let r = app.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "inserting edits nothing");
        assert_eq!(app.session.journal.len(), journal, "and records nothing");
        assert_eq!(undo_steps(&app), undo, "and adds no undo step");
        let action = &app.ui.action_sets[0].actions[0];
        assert_eq!(action.steps[1].0, SELECT_PATH_STEP);
        assert_eq!(action.steps[1].1["id"], json!(first));
        assert!(action.steps[1].1["fingerprint"].as_str().is_some_and(|f| f.starts_with("v1:")), "{:?}", action.steps[1]);
        assert_eq!(action.excluded_steps, BTreeSet::from([0, 3]), "the step excluded after the insertion point moved with it");
        assert!(!action.excluded_steps.contains(&1), "the new step is included");

        app.run("select.set", json!({"ids": [second]})).unwrap();
        let before = undo_steps(&app);
        play(&mut app, 0, 0).unwrap();
        assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(first)], "the step selected the stored path");
        assert_eq!(x(&app), 2.0, "the move after it moved the stored path");
        assert_eq!(left_of(&app, second), 110.0, "and not the path selected before");
        assert_eq!(undo_steps(&app), before + 1, "the action plays as one undo step");
    }

    #[test]
    fn a_select_path_step_plays_step_by_step_and_a_changed_path_ends_the_playback() {
        let (mut app, x, [_, second]) = app_with_two_rectangles();
        app.ui.action_sets[0].actions[0].steps.truncate(1);
        app.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0})).unwrap();
        app.ui.action_sets[0].actions[0].steps.push(("object.move".into(), json!({"dx": 4, "dy": 0})));
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

    #[test]
    fn insert_select_path_refuses_what_is_not_one_whole_path_and_changes_nothing() {
        let (mut app, _, [first, second]) = app_with_two_rectangles();
        let place = json!({"set": 0, "action": 0});
        let refused = |app: &mut VectorcraftApp, params: Value, want: &str| {
            let selection = app.session.active().map(|st| st.selection.objects.clone());
            assert_eq!(app.run(INSERT_SELECT_PATH, params.clone()).unwrap_err(), want, "{params}");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "{params}");
            assert_eq!(app.session.active().map(|st| st.selection.objects.clone()), selection, "{params}");
        };
        app.run("select.set", json!({"ids": [first, second]})).unwrap();
        refused(&mut app, place.clone(), "select only one path");
        app.run("select.none", json!({})).unwrap();
        refused(&mut app, place.clone(), "select a path");
        app.run("select.anchors", json!({"id": first, "anchors": [[0, 0]], "mode": "set"})).unwrap();
        refused(&mut app, place.clone(), "select one whole path or compound path");
        app.run("select.set", json!({"ids": [first, second]})).unwrap();
        let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [group]})).unwrap();
        refused(&mut app, place.clone(), "select one whole path or compound path");
        app.run("select.set", json!({"ids": [first]})).unwrap();
        for (params, want) in [
            (json!({}), "no such action"),
            (json!({"set": 0}), "no such action"),
            (json!({"set": 1, "action": 0}), "no such action"),
            (json!({"set": 0, "action": 1}), "no such action"),
            (json!({"set": "0", "action": 0}), "no such action"),
            (json!({"set": 0, "action": 0, "step": 3}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": null}), "no such action step"),
            (json!(5), "`params` must be an object"),
            (json!(null), "`params` must be an object"),
        ] {
            refused(&mut app, params, want);
        }
        let mut empty = app_with_sets();
        assert_eq!(empty.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0})).unwrap_err(), "no document open");
    }

    #[test]
    fn insert_select_path_goes_into_the_recording() {
        let (mut app, _, [first, _]) = app_with_two_rectangles();
        app.run(START_RECORDING, json!({})).unwrap();
        assert_eq!(app.run(INSERT_SELECT_PATH, json!({})).unwrap(), json!({"recording": true}));
        assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(first)]);
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        assert_eq!(recorded.steps.len(), 1);
        assert_eq!((recorded.steps[0].0.as_str(), &recorded.steps[0].1["id"]), (SELECT_PATH_STEP, &json!(first)));
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
        for params in
            [json!({"id": 3}), json!({"id": -1, "fingerprint": step.1["fingerprint"]}), json!({"id": 3, "fingerprint": "v1:xyz"}), json!(42)]
        {
            let bad = json!({"command": SELECT_PATH_STEP, "params": params});
            for s in [bad.clone(), json!({"command": "command.batch", "params": {"commands": [bad]}})] {
                let file = json!({"format": "vcactions", "version": 5, "name": "Bad", "actions": [{"name": "A", "steps": [s]}]});
                assert_eq!(files::read_set(&file.to_string(), "Test").unwrap_err(), "not an action set file", "{s}");
            }
        }
    }

    #[test]
    fn insert_select_path_in_the_menu_needs_a_path_and_shows_its_row() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let (mut app, _, [first, _]) = app_with_two_rectangles();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let menu_click = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            let item = texts.iter().find(|(s, _)| s.trim() == "Insert Select Path").map(|(_, r)| *r).expect("Insert Select Path in the menu");
            click(ctx, app, item.center(), menu);
        };
        menu_click(&ctx, &mut app);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "no action selected and nothing recording: the item is off");

        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
        app.run("select.none", json!({})).unwrap();
        menu_click(&ctx, &mut app);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "no path selected: the item is off");

        app.run("select.set", json!({"ids": [first]})).unwrap();
        menu_click(&ctx, &mut app);
        let steps = &app.ui.action_sets[0].actions[0].steps;
        assert_eq!((steps.len(), steps[3].0.as_str()), (4, SELECT_PATH_STEP), "the action selected: the step goes at its end");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        assert!(texts.iter().any(|(s, _)| s == "Select Path"), "the step's row names it: {texts:?}");
    }

    /// An action stays one that Load Actions… reads back: a stop or a menu item that would give it
    /// more than 10,000 steps, or its set's file more than 8 MiB, is refused, and the action keeps its
    /// steps and exclusions as they were.
    #[test]
    fn inserted_steps_keep_an_action_set_that_load_actions_reads() {
        let (mut app, _) = app_with_three_moves();
        let full = vec![("object.group".to_string(), json!({})); files::MAX_ACTION_STEPS];
        app.ui.action_sets[0].actions[0].steps = full;
        let steps = "the action already has 10,000 steps";
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).unwrap_err(), steps);
        assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap_err(), steps);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), files::MAX_ACTION_STEPS);
        app.ui.action_sets[0].actions[0].steps.pop();
        assert!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).is_ok(), "the 10,000th step goes in");
        let saved = files::write_set(&app.ui.action_sets[0]);
        assert_eq!(files::read_set(&saved, "Test").unwrap().actions[0].steps.len(), files::MAX_ACTION_STEPS, "and loads again");

        let pad = |n: usize| vec![("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}))];
        app.ui.action_sets[0].actions[0].steps = pad(0);
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 7]);
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.ui.action_sets[0].actions[0].steps = pad(room - 8);
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "object.group"})).unwrap_err(), size);
        assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap_err(), size);
        let action = &app.ui.action_sets[0].actions[0];
        assert_eq!((action.steps.len(), &action.excluded_steps), (1, &BTreeSet::from([0, 7])), "nothing changed");
        assert!(files::write_set(&app.ui.action_sets[0]).len() <= files::MAX_SET_FILE);
    }

    /// The limits hold for a step inserted into the recording in progress too: they count the steps
    /// recorded so far and, for New Action's recording, the action's own steps and its set.
    #[test]
    fn steps_inserted_into_a_recording_keep_an_action_set_that_load_actions_reads() {
        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({})).unwrap();
        app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap();
        for _ in 1..files::MAX_ACTION_STEPS - 1 {
            app.session.journal.push(("object.group".to_string(), json!({})));
        }
        let steps = "the action already has 10,000 steps";
        assert_eq!(app.run(INSERT_STOP, json!({})).unwrap(), json!({"recording": true}), "the 10,000th step goes in");
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap_err(), steps);
        assert_eq!(app.run(INSERT_STOP, json!({})).unwrap_err(), steps);
        app.run(STOP_RECORDING, json!({})).unwrap();
        let saved = files::write_set(&app.ui.action_sets[0]);
        assert_eq!(files::read_set(&saved, "Test").unwrap().actions.last().unwrap().steps.len(), files::MAX_ACTION_STEPS, "and loads again");

        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({})).unwrap();
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.ui.action_sets[0].actions[0].steps = vec![pad(room - 8)];
        let into = app.ui.action_sets[0].actions[0].id;
        app.ui.recording_into = Some(into);
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"command": "object.group"})).unwrap_err(), size);
        assert_eq!(app.run(INSERT_STOP, json!({})).unwrap_err(), size);
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 1, "nothing was recorded into the action");
        assert!(files::write_set(&app.ui.action_sets[0]).len() <= files::MAX_SET_FILE);
    }

    /// A step put by its set and action into the action that the recording in progress records into
    /// counts the steps recorded so far, which Stop Playing/Recording adds to that action.
    #[test]
    fn steps_inserted_into_the_action_being_recorded_into_count_the_recorded_steps() {
        let (mut app, _) = app_with_three_moves();
        app.ui.action_sets[0].actions.push(Action::new("Other", vec![]));
        let into = app.ui.action_sets[0].actions[0].id;
        let stored = app.ui.action_sets[0].actions[0].steps.len();
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = Some(into);
        for _ in stored..files::MAX_ACTION_STEPS {
            app.session.journal.push(("object.group".to_string(), json!({})));
        }
        let steps = "the action already has 10,000 steps";
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).unwrap_err(), steps);
        assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap_err(), steps);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), stored, "nothing was inserted");
        assert!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 1, "command": "object.group"})).is_ok(), "another action takes the step");
        app.run(STOP_RECORDING, json!({})).unwrap();
        let saved = files::write_set(&app.ui.action_sets[0]);
        assert_eq!(files::read_set(&saved, "Test").unwrap().actions[0].steps.len(), files::MAX_ACTION_STEPS, "and loads again");

        let (mut app, _) = app_with_three_moves();
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        let into = app.ui.action_sets[0].actions[0].id;
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = Some(into);
        app.session.journal.push(pad(room - 8));
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.group"})).unwrap_err(), size);
        assert_eq!(app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap_err(), size);
        // Another action of the set counts them too, for a recording into an action and for a
        // recording that adds a new action.
        for into_first in [true, false] {
            let (mut app, _) = app_with_three_moves();
            app.ui.action_sets[0].actions.push(Action::new("Other", vec![]));
            let first = app.ui.action_sets[0].actions[0].id;
            app.run(START_RECORDING, json!({})).unwrap();
            app.ui.recording_into = into_first.then_some(first);
            app.session.journal.push(pad(room - 8));
            let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 1, "command": "object.group"}));
            assert_eq!(r.unwrap_err(), size, "recording into the first action: {into_first}");
            assert!(app.ui.action_sets[0].actions[1].steps.is_empty(), "recording into the first action: {into_first}");
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
            (
                "command.batch",
                json!({"label": "B", "commands": [{"command": "object.move", "params": {"dx": 1, "n": {"a": [1, 2.5, {"b": null}]}}}]}),
            ),
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

    /// The params of `object.move` by 1 as the journal keeps them, which a recording records.
    fn recorded_move(app: &mut VectorcraftApp) -> Value {
        app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap();
        let moved = app.session.journal.last().unwrap().1.clone();
        app.run("edit.undo", json!({})).unwrap();
        moved
    }

    /// An ordinary command recorded as the 10,001st step of an action does not run: the document,
    /// the journal and the undo history stay as they were, the recording goes on, and Stop
    /// Playing/Recording leaves a set that Load Actions… reads. This holds for Begin Recording's new
    /// action and for New Action's recording into an action with steps of its own.
    #[test]
    fn recorded_commands_keep_an_action_at_10_000_steps() {
        for new_action in [false, true] {
            let (mut app, x) = app_with_three_moves();
            let moved = recorded_move(&mut app);
            let filler = ("object.group".to_string(), json!({}));
            if new_action {
                app.run(NEW_ACTION, json!({"set": 0, "name": "Long"})).unwrap();
                app.ui.action_sets[0].actions[1].steps = vec![filler; files::MAX_ACTION_STEPS - 1];
            } else {
                app.run(START_RECORDING, json!({})).unwrap();
                for _ in 1..files::MAX_ACTION_STEPS {
                    app.session.journal.push(filler.clone());
                }
            }
            app.run("object.move", moved.clone()).unwrap();
            assert_eq!(x(&app), 1.0, "the 10,000th step runs");
            let (journal, undo) = (app.session.journal.len(), undo_steps(&app));
            let steps = "the action already has 10,000 steps";
            assert_eq!(app.run("object.move", moved.clone()).unwrap_err(), steps, "new action: {new_action}");
            assert_eq!((x(&app), app.session.journal.len(), undo_steps(&app)), (1.0, journal, undo), "nothing ran");
            assert_eq!(app.ui.status, steps);
            assert!(app.ui.recording.is_some(), "the recording goes on");
            assert!(app.run("effect.list", json!({})).is_ok(), "a command that is not recorded still runs");
            let find = |select: bool| json!({"minWidth": 0, "select": select});
            assert_eq!(app.run("document.find", find(true)).unwrap_err(), steps, "Find Objects that selects is recorded");
            assert!(app.run("document.find", find(false)).is_ok());
            let batch = json!({"commands": [{"command": "object.move", "params": moved.clone()}]});
            assert_eq!(app.run("command.batch", batch).unwrap_err(), steps, "a batch is recorded");
            assert_eq!(x(&app), 1.0);
            assert_eq!(app.session.journal.len(), journal);
            app.run(STOP_RECORDING, json!({})).unwrap();
            assert_eq!(app.ui.action_sets[0].actions[1].steps.len(), files::MAX_ACTION_STEPS);
            assert_eq!(app.ui.action_sets[0].actions[1].steps.last(), Some(&("object.move".to_string(), moved)));
            let saved = files::write_set(&app.ui.action_sets[0]);
            assert_eq!(files::read_set(&saved, "Test").unwrap().actions[1].steps.len(), files::MAX_ACTION_STEPS, "and loads again");
        }
    }

    /// An ordinary command whose step would give the set of the action it is recorded into a file
    /// larger than 8 MiB does not run, and one that makes the file exactly 8 MiB does.
    #[test]
    fn recorded_commands_keep_an_action_set_file_within_8_mib() {
        let (mut app, x) = app_with_three_moves();
        let moved = recorded_move(&mut app);
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        let added = files::step_bytes("object.move", &moved);
        app.ui.action_sets[0].actions[0].steps = vec![pad(room - added)];
        let into = app.ui.action_sets[0].actions[0].id;
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = Some(into);
        app.run("object.move", moved.clone()).unwrap();
        assert_eq!(x(&app), 1.0, "the step that fills the file runs");
        let (journal, undo) = (app.session.journal.len(), undo_steps(&app));
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.run("object.move", moved).unwrap_err(), size);
        assert_eq!((x(&app), app.session.journal.len(), undo_steps(&app)), (1.0, journal, undo), "nothing ran");
        assert!(app.ui.recording.is_some(), "the recording goes on");
        app.ui.status.clear();
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.status, "", "Stop keeps every recorded step");
        let saved = files::write_set(&app.ui.action_sets[0]);
        assert_eq!(saved.len(), files::MAX_SET_FILE);
        assert_eq!(files::read_set(&saved, "Test").unwrap().actions[0].steps.len(), 2, "and loads again");
    }

    /// A command whose step fits by its params but not with the values it adds to its step as it
    /// runs (Scale's Scale Strokes & Effects and Scale Corners) keeps its edit, the status bar names
    /// the limit, the next recorded command does not run, and Stop Playing/Recording leaves the step
    /// out.
    #[test]
    fn a_recorded_command_whose_noted_values_do_not_fit_is_left_out() {
        let (mut app, x) = app_with_three_moves();
        let scaled = json!({"sx": 200});
        app.run("object.scale", scaled.clone()).unwrap();
        let noted = app.session.journal.last().unwrap().1.clone();
        app.run("edit.undo", json!({})).unwrap();
        assert!(files::step_bytes("object.scale", &noted) > files::step_bytes("object.scale", &scaled), "Scale notes values: {noted}");
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        // Room for the step as given, not for the step as recorded.
        app.ui.action_sets[0].actions[0].steps = vec![pad(room - files::step_bytes("object.scale", &scaled))];
        let into = app.ui.action_sets[0].actions[0].id;
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = Some(into);
        let size = "the action set would be larger than 8 MiB";
        app.run("object.scale", scaled).unwrap();
        assert_eq!(app.ui.status, size);
        let scaled_x = x(&app);
        assert_ne!(scaled_x, 0.0, "the scale stayed");
        assert_eq!(app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap_err(), size, "no later command runs");
        assert_eq!(x(&app), scaled_x);
        assert!(app.ui.recording.is_some(), "the recording goes on");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 1, "the scale's step is left out");
        assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());
    }

    /// A script that moves the selection twice.
    const TWO_MOVES: &str = "vc.run('object.move', {dx: 1, dy: 0}); vc.run('object.move', {dx: 1, dy: 0});";

    /// Runs `source` to its end: in one call (`frames` false, a run with nobody to ask), or a frame
    /// at a time, as the app does.
    fn run_script(app: &mut VectorcraftApp, source: &str, frames: bool) {
        app.session.script_frames = frames;
        app.run("script.run", json!({"source": source, "name": "t.js"})).unwrap();
        for _ in 0..10_000 {
            if !app.session.script_running() {
                return;
            }
            crate::script_runs::poll(app);
        }
        panic!("the script did not end");
    }

    /// Begin Recording with `filler` steps already recorded.
    fn recording_with(app: &mut VectorcraftApp, filler: usize) {
        app.run(START_RECORDING, json!({})).unwrap();
        for _ in 0..filler {
            app.session.journal.push(("object.group".to_string(), json!({})));
        }
    }

    /// A script whose commands take a recording past 10,000 steps keeps its edit and the whole
    /// journal, the status bar names the limit as the run ends, the next recorded command does not
    /// run, and Stop Playing/Recording keeps the 10,000 steps that fit. One that fills the action
    /// exactly names no limit.
    #[test]
    fn script_run_checks_recording_step_limit_on_completion() {
        let steps = "the action already has 10,000 steps";
        for frames in [false, true] {
            let (mut app, x) = app_with_three_moves();
            recording_with(&mut app, files::MAX_ACTION_STEPS - 1);
            app.ui.status.clear();
            run_script(&mut app, TWO_MOVES, frames);
            assert_eq!(x(&app), 2.0, "frames {frames}: the script's edit stays");
            let start = recording_start(&app).unwrap();
            assert_eq!(app.session.journal.len() - start, files::MAX_ACTION_STEPS + 1, "frames {frames}: the journal keeps every step");
            assert_eq!(app.ui.status, steps, "frames {frames}: the status bar names the limit now");
            assert!(app.ui.recording.is_some(), "the recording goes on");
            assert_eq!(app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap_err(), steps, "no later command runs");
            assert_eq!(x(&app), 2.0);
            app.run(STOP_RECORDING, json!({})).unwrap();
            let recorded = app.ui.action_sets[0].actions.last().unwrap();
            assert_eq!(recorded.steps.len(), files::MAX_ACTION_STEPS);
            assert_eq!(recorded.steps.last().map(|(c, _)| c.as_str()), Some("object.move"), "the first move fits");
            let saved = files::write_set(&app.ui.action_sets[0]);
            assert_eq!(files::read_set(&saved, "Test").unwrap().actions[1].steps.len(), files::MAX_ACTION_STEPS, "and loads again");

            let (mut app, _) = app_with_three_moves();
            recording_with(&mut app, files::MAX_ACTION_STEPS - 2);
            app.ui.status.clear();
            run_script(&mut app, TWO_MOVES, frames);
            let ran = if frames { "Ran t.js" } else { "" };
            assert_eq!(app.ui.status, ran, "frames {frames}: 10,000 steps fit");
        }
    }

    /// A script whose commands give the recording's set a file larger than 8 MiB keeps its edit,
    /// the status bar names the limit as the run ends, and Stop Playing/Recording keeps the steps
    /// that fit in a set that Load Actions… reads. One that fills the file exactly names no limit.
    #[test]
    fn script_run_checks_recording_byte_limit_on_completion() {
        let size = "the action set would be larger than 8 MiB";
        for frames in [false, true] {
            for spare in [0, 1] {
                let (mut app, x) = app_with_three_moves();
                // The two steps as an action keeps them.
                let before = app.session.journal.len();
                run_script(&mut app, TWO_MOVES, false);
                let both: usize = app.session.journal_for_action(before).iter().map(|(c, p)| files::step_bytes(c, p)).sum();
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(x(&app), 0.0);
                let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
                app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
                let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
                // Room for both moves, or for one move and all but a byte of the other.
                app.ui.action_sets[0].actions[0].steps = vec![pad(room - both + 1 - spare)];
                let into = app.ui.action_sets[0].actions[0].id;
                app.run(START_RECORDING, json!({})).unwrap();
                app.ui.recording_into = Some(into);
                app.ui.status.clear();
                run_script(&mut app, TWO_MOVES, frames);
                assert_eq!(x(&app), 2.0, "frames {frames}: the script's edit stays");
                let what = format!("frames {frames}, spare {spare}");
                if spare == 1 {
                    assert_eq!(app.ui.status, if frames { "Ran t.js" } else { "" }, "{what}: the file is exactly 8 MiB");
                    continue;
                }
                assert_eq!(app.ui.status, size, "{what}: the status bar names the limit now");
                assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), 2, "the journal keeps both moves");
                assert_eq!(app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap_err(), size, "no later command runs");
                app.run(STOP_RECORDING, json!({})).unwrap();
                assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 2, "{what}: the first move fits");
                let saved = files::write_set(&app.ui.action_sets[0]);
                assert!(saved.len() <= files::MAX_SET_FILE);
                assert!(files::read_set(&saved, "Test").is_ok());
            }
        }
    }

    /// Presses, drags and releases the active tool from `from` to `to`, as the canvas hands its
    /// results on ([`crate::canvas::apply_requests`]).
    fn tool_drag(app: &mut VectorcraftApp, from: (f64, f64), to: (f64, f64)) {
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let view = app.view_info();
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            let r = app.session.pointer(&PointerEvent::new(kind, x, y), view);
            crate::canvas::apply_requests(app, r);
        }
    }

    /// A drag with the Selection tool or the Rectangle tool (each commits its preview) that takes a
    /// recording past 10,000 steps keeps its edit and the journal, the status bar names the limit
    /// as the tool ends, and Stop Playing/Recording keeps the 10,000 steps that fit. One that fills
    /// the action exactly names no limit.
    #[test]
    fn tool_commit_checks_recording_step_limit() {
        let steps = "the action already has 10,000 steps";
        for tool in ["selection", "rectangle"] {
            for filler in [files::MAX_ACTION_STEPS, files::MAX_ACTION_STEPS - 1] {
                let (mut app, _) = app_with_three_moves();
                app.select_tool(tool);
                recording_with(&mut app, filler);
                app.ui.status.clear();
                let (journal, art) = (app.session.journal.len(), app.session.active().unwrap().doc.art_bounds());
                tool_drag(&mut app, (20.0, 20.0), (60.0, 50.0));
                assert_eq!(app.session.journal.len(), journal + 1, "{tool}: the tool journals one step");
                assert_ne!(app.session.active().unwrap().doc.art_bounds(), art, "{tool}: the edit stays");
                if filler < files::MAX_ACTION_STEPS {
                    assert_eq!(app.ui.status, "", "{tool}: 10,000 steps fit");
                    continue;
                }
                assert_eq!(app.ui.status, steps, "{tool}: the status bar names the limit now");
                assert!(app.ui.recording.is_some(), "the recording goes on");
                app.run(STOP_RECORDING, json!({})).unwrap();
                assert_eq!(app.ui.action_sets[0].actions.last().unwrap().steps.len(), files::MAX_ACTION_STEPS);
            }
        }
    }

    /// A tool's edit whose step gives the recording's set a file larger than 8 MiB keeps its edit,
    /// the status bar names the limit as the tool ends, and Stop Playing/Recording leaves the step
    /// out of a set that Load Actions… reads.
    #[test]
    fn tool_commit_checks_recording_byte_limit() {
        let size = "the action set would be larger than 8 MiB";
        for tool in ["selection", "rectangle"] {
            let (mut app, _) = app_with_three_moves();
            app.select_tool(tool);
            app.run(START_RECORDING, json!({})).unwrap();
            let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
            app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
            let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
            app.ui.action_sets[0].actions[0].steps = vec![pad(room)];
            app.ui.recording_into = Some(app.ui.action_sets[0].actions[0].id);
            app.ui.status.clear();
            let art = app.session.active().unwrap().doc.art_bounds();
            tool_drag(&mut app, (20.0, 20.0), (60.0, 50.0));
            assert_ne!(app.session.active().unwrap().doc.art_bounds(), art, "{tool}: the edit stays");
            assert_eq!(app.ui.status, size, "{tool}: the status bar names the limit now");
            assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), 1, "{tool}: the journal keeps the step");
            app.run(STOP_RECORDING, json!({})).unwrap();
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 1, "{tool}: the step is left out");
            assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());
        }
    }

    /// Drags the Enclosing Lasso away from the selected rectangle, which runs `select.set` with no
    /// objects, and checks that the tool journaled that one step and cleared the selection.
    fn lasso_nothing(app: &mut VectorcraftApp) {
        app.select_tool(vectorcraft_tools::addons::enclosing_lasso::ID);
        assert_eq!(app.session.tool_id(), vectorcraft_tools::addons::enclosing_lasso::ID);
        assert!(!app.session.active().unwrap().selection.objects.is_empty());
        let journal = app.session.journal.len();
        tool_drag(app, (100.0, 80.0), (120.0, 80.0));
        assert_eq!(app.session.journal.len(), journal + 1, "the tool journals one step");
        assert_eq!(app.session.journal.last(), Some(&("select.set".to_string(), json!({"ids": []}))));
        assert!(app.session.active().unwrap().selection.objects.is_empty(), "the edit stays");
    }

    /// A command a tool runs (the Enclosing Lasso's `select.set`) that takes a recording past
    /// 10,000 steps keeps its edit and the journal, the status bar names the limit as the tool
    /// ends, the next recorded command does not run, and Stop Playing/Recording keeps the 10,000
    /// steps that fit. One that fills the action exactly names no limit.
    #[test]
    fn tool_exec_checks_recording_step_limit() {
        let steps = "the action already has 10,000 steps";
        for filler in [files::MAX_ACTION_STEPS, files::MAX_ACTION_STEPS - 1] {
            let (mut app, _) = app_with_three_moves();
            recording_with(&mut app, filler);
            app.ui.status.clear();
            lasso_nothing(&mut app);
            if filler < files::MAX_ACTION_STEPS {
                assert_eq!(app.ui.status, "", "10,000 steps fit");
                continue;
            }
            assert_eq!(app.ui.status, steps, "the status bar names the limit now");
            assert!(app.ui.recording.is_some(), "the recording goes on");
            let (journal, art) = (app.session.journal.len(), app.session.active().unwrap().doc.art_bounds());
            let r = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 20, "height": 20}));
            assert_eq!(r.unwrap_err(), steps, "no later command runs");
            assert_eq!(app.session.journal.len(), journal);
            assert_eq!(app.session.active().unwrap().doc.art_bounds(), art, "no rectangle is drawn");
            assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), files::MAX_ACTION_STEPS + 1, "the journal keeps the step");
            app.run(STOP_RECORDING, json!({})).unwrap();
            assert_eq!(app.ui.action_sets[0].actions.last().unwrap().steps.len(), files::MAX_ACTION_STEPS);
        }
    }

    /// A command a tool runs (the Enclosing Lasso's `select.set`) whose step gives the recording's
    /// set a file larger than 8 MiB keeps its edit, the status bar names the limit as the tool
    /// ends, and Stop Playing/Recording leaves the step out of a set that Load Actions… reads.
    #[test]
    fn tool_exec_checks_recording_byte_limit() {
        let size = "the action set would be larger than 8 MiB";
        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({})).unwrap();
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.ui.action_sets[0].actions[0].steps = vec![pad(room)];
        app.ui.recording_into = Some(app.ui.action_sets[0].actions[0].id);
        app.ui.status.clear();
        lasso_nothing(&mut app);
        assert_eq!(app.ui.status, size, "the status bar names the limit now");
        assert_eq!(app.session.journal.since(recording_start(&app).unwrap()).len(), 1, "the journal keeps the step");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 1, "the step is left out");
        let saved = files::write_set(&app.ui.action_sets[0]);
        assert!(saved.len() <= files::MAX_SET_FILE);
        assert!(files::read_set(&saved, "Test").is_ok());
    }

    /// The place cursor's `file.place` that takes a recording past 10,000 steps keeps the placed
    /// file and the journal, and the status bar names the limit once the file is placed: on the
    /// release when the file is read at once, and on the frame that places it when it is read off
    /// the UI thread. Stop Playing/Recording keeps the 10,000 steps that fit.
    #[test]
    fn place_cursor_exec_checks_recording_step_limit_on_completion() {
        let steps = "the action already has 10,000 steps";
        let mut png = vec![];
        image::RgbImage::new(8, 8).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let placed = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            st.doc.layers.iter().flat_map(|l| l.children().into_iter().flatten()).filter(|n| n.name.as_deref() == Some("a.png")).count()
        };
        for background in [false, true] {
            let (mut app, _) = app_with_three_moves();
            app.placing.off_the_ui_thread = background;
            app.run("file.place.queue", json!({"files": [{"name": "a.png", "dataBase64": vectorcraft_format::base64_encode(&png)}]})).unwrap();
            assert_eq!(app.session.tool_id(), "place");
            recording_with(&mut app, files::MAX_ACTION_STEPS);
            app.ui.status.clear();
            let journal = app.session.journal.len();
            tool_drag(&mut app, (40.0, 40.0), (80.0, 80.0));
            if background {
                assert_eq!(app.session.journal.len(), journal, "the release places nothing yet");
                assert_eq!(placed(&app), 0);
                assert_ne!(app.ui.status, steps, "nothing is checked before the file is placed");
                app.placing.wait_for_reads();
                let ctx = egui::Context::default();
                app.begin_host_iteration();
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
                out.textures_delta.clear();
                assert!(app.placing.pending.is_empty(), "the frame after the read places the file");
            }
            assert_eq!(app.session.journal.len(), journal + 1, "background {background}: the place journals one step");
            assert_eq!(app.session.journal.last().map(|(c, _)| c.as_str()), Some("file.place"));
            assert_eq!(placed(&app), 1, "background {background}: the placed file stays");
            assert_eq!(app.ui.status, steps, "background {background}: the status bar names the limit once the file is placed");
            assert!(app.ui.recording.is_some(), "the recording goes on");
            app.run(STOP_RECORDING, json!({})).unwrap();
            assert_eq!(app.ui.action_sets[0].actions.last().unwrap().steps.len(), files::MAX_ACTION_STEPS, "background {background}");
        }
    }

    /// A step that reaches the journal outside a command, as a panel's or a dialog's commit does, is
    /// checked on the app's next frame. A journal rewrite that makes a checked step larger is
    /// checked again, and a frame with no new step does not name the limit again.
    #[test]
    fn a_journal_rewrite_or_a_direct_commit_is_checked_on_the_next_frame() {
        let size = "the action set would be larger than 8 MiB";
        let (mut app, _) = app_with_three_moves();
        let small = ("object.move".to_string(), json!({"dx": 1, "dy": 0}));
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.ui.action_sets[0].actions[0].steps = vec![pad(room - files::step_bytes(&small.0, &small.1))];
        let into = app.ui.action_sets[0].actions[0].id;
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = Some(into);
        let ctx = egui::Context::default();
        let frame = |app: &mut VectorcraftApp| {
            app.begin_host_iteration();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
            out.textures_delta.clear();
        };
        app.session.journal.push(small);
        app.ui.status.clear();
        frame(&mut app);
        assert_eq!(app.ui.status, "", "a step that fills the file exactly");
        let start = recording_start(&app).unwrap();
        app.session.journal.replace(start, pad(1));
        frame(&mut app);
        assert_eq!(app.ui.status, size, "the rewritten step no longer fits");
        app.ui.status.clear();
        frame(&mut app);
        assert_eq!(app.ui.status, "", "named once");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 1, "the step is left out");
    }

    /// When the action New Action made for the recording in progress is deleted, Stop
    /// Playing/Recording keeps none of its steps, so no limit stops a recorded command, even with
    /// the set's file at 8 MiB.
    #[test]
    fn a_deleted_recording_action_does_not_limit_recorded_commands() {
        let (mut app, x) = app_with_three_moves();
        let gone = Action::new("Gone", vec![]);
        let id = gone.id;
        app.ui.action_sets[0].actions.push(gone);
        app.run(START_RECORDING, json!({})).unwrap();
        app.ui.recording_into = Some(id);
        app.ui.action_sets[0].actions.retain(|a| a.id != id);
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions[0].steps = vec![pad(0)];
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.ui.action_sets[0].actions[0].steps = vec![pad(room)];
        let before = x(&app);
        app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap();
        assert_ne!(x(&app), before, "the move ran");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions.len(), 1, "no action is added");
        assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());
    }

    /// A stop that playback reaches while a full action records is not recorded: the status bar shows
    /// the reason, and the playback still waits at the stop.
    #[test]
    fn a_stop_reached_while_recording_a_full_action_is_left_out() {
        let (mut app, _) = app_with_a_stop(false);
        app.run(START_RECORDING, json!({})).unwrap();
        for _ in 1..files::MAX_ACTION_STEPS {
            app.session.journal.push(("object.group".to_string(), json!({})));
        }
        play(&mut app, 0, 0).unwrap();
        assert!(waiting_at_stop(&app));
        assert_eq!(app.ui.status, "the action already has 10,000 steps");
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        assert_eq!(recorded.steps.len(), files::MAX_ACTION_STEPS);
        assert_eq!(recorded.steps.last().map(|(c, _)| c.as_str()), Some("command.batch"));
    }

    /// Steps that reach the journal without a command's check are left out by Stop Playing/Recording
    /// past either limit, and the status bar names the limit.
    #[test]
    fn stop_leaves_out_recorded_steps_past_the_limits() {
        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({})).unwrap();
        for _ in 0..files::MAX_ACTION_STEPS + 5 {
            app.session.journal.push(("object.group".to_string(), json!({})));
        }
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.status, "the action already has 10,000 steps");
        assert_eq!(app.ui.action_sets[0].actions.last().unwrap().steps.len(), files::MAX_ACTION_STEPS);

        let (mut app, _) = app_with_three_moves();
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.run(START_RECORDING, json!({})).unwrap();
        app.session.journal.push(pad(files::MAX_SET_FILE / 2));
        app.session.journal.push(pad(files::MAX_SET_FILE / 2));
        app.session.journal.push(pad(0));
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.status, "the action set would be larger than 8 MiB");
        assert_eq!(app.ui.action_sets[0].actions.last().unwrap().steps.len(), 1);
        assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());
    }

    /// An exclusion that would give a set a file larger than 8 MiB is refused for a set, an action,
    /// a step and the other steps of an action, and leaves every exclusion as it was. One that
    /// fills the file exactly, one that changes nothing and an inclusion still work, also in a set
    /// that is already larger than 8 MiB.
    #[test]
    fn excluding_steps_keeps_an_action_set_file_within_8_mib() {
        let (mut app, _) = app_with_three_moves();
        let pad = |n: usize| ("object.move".to_string(), json!({"dx": 1, "dy": 0, "pad": "x".repeat(n)}));
        app.ui.action_sets[0].actions.push(Action::new("Pad", vec![pad(0)]));
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        // Room for two more excluded steps.
        app.ui.action_sets[0].actions[1].steps = vec![pad(room - 2 * files::EXCLUDED_BYTES)];
        let size = "the action set would be larger than 8 MiB";
        let include = |app: &mut VectorcraftApp, p: Value| app.run(INCLUDE_STEPS, p);
        let excluded = |app: &VectorcraftApp| app.ui.action_sets[0].actions.iter().map(|a| a.excluded_steps.clone()).collect::<Vec<_>>();
        let none = vec![BTreeSet::new(), BTreeSet::new()];
        assert_eq!(include(&mut app, json!({"set": 0, "included": false})).unwrap_err(), size);
        assert_eq!(include(&mut app, json!({"set": 0, "action": 0, "included": false})).unwrap_err(), size);
        assert_eq!(excluded(&app), none, "nothing changed");
        // Two steps fill the file to 8 MiB.
        include(&mut app, json!({"set": 0, "action": 0, "step": 0, "included": false, "others": true})).unwrap();
        assert_eq!(excluded(&app), [BTreeSet::from([1, 2]), BTreeSet::new()]);
        assert_eq!(files::write_set(&app.ui.action_sets[0]).len(), files::MAX_SET_FILE);
        assert_eq!(include(&mut app, json!({"set": 0, "action": 0, "step": 0, "included": false})).unwrap_err(), size);
        assert_eq!(include(&mut app, json!({"set": 0, "action": 1, "step": 0, "included": false})).unwrap_err(), size);
        assert!(include(&mut app, json!({"set": 0, "action": 0, "step": 1, "included": false})).is_ok(), "no change");
        include(&mut app, json!({"set": 0, "action": 0, "step": 1, "included": true})).unwrap();
        include(&mut app, json!({"set": 0, "action": 0, "step": 0, "included": false})).unwrap();
        assert_eq!(excluded(&app), [BTreeSet::from([0, 2]), BTreeSet::new()]);
        assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());

        // A set already larger than 8 MiB can still include its steps.
        app.ui.action_sets[0].actions[1].steps[0] = pad(room);
        assert_eq!(include(&mut app, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap_err(), size);
        include(&mut app, json!({"set": 0, "included": true})).unwrap();
        assert_eq!(excluded(&app), none);

        // The steps a recording adds to the set count too.
        let (mut app, _) = app_with_three_moves();
        let room = files::MAX_SET_FILE - files::write_set(&app.ui.action_sets[0]).len();
        app.run(START_RECORDING, json!({})).unwrap();
        app.session.journal.push(pad(room - files::step_bytes("object.move", &pad(0).1)));
        assert_eq!(include(&mut app, json!({"set": 0, "included": false})).unwrap_err(), size);
        assert!(app.ui.action_sets[0].actions[0].excluded_steps.is_empty());
    }

    /// Adds an action to set `set` whose step makes the set's file `short` bytes short of 8 MiB.
    fn pad_set_to(app: &mut VectorcraftApp, set: usize, short: usize) {
        let pad = |n: usize| ("object.move".to_string(), json!({"pad": "x".repeat(n)}));
        let s = &mut app.ui.action_sets[set];
        s.actions.push(Action::new("Pad", vec![pad(0)]));
        let room = files::MAX_SET_FILE - files::write_set(s).len();
        s.actions.last_mut().unwrap().steps[0] = pad(room - short);
        assert_eq!(files::write_set(s).len(), files::MAX_SET_FILE - short);
    }

    /// `n` actions with no steps, named A0, A1 …
    fn empty_actions(n: usize) -> Vec<Action> {
        (0..n).map(|i| Action::new(format!("A{i}"), vec![])).collect()
    }

    /// A recorded batch that edits another document, and whose values make its step too large only
    /// as it runs, keeps its edits in both documents: the status bar names the limit, the next
    /// recorded command does not run, and Stop Playing/Recording leaves the batch's step out.
    #[test]
    fn a_recorded_batch_that_edits_another_document_is_left_out_when_it_grows() {
        // Three moves' document, a move as recorded there, and a second document that is active.
        let two_documents = || {
            let (mut app, x) = app_with_three_moves();
            let moved = recorded_move(&mut app);
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            (app, x, moved)
        };
        let batch = json!({"commands": [
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "object.scale", "params": {"sx": 200}},
        ]});
        let (mut app, _, _) = two_documents();
        app.run("command.batch", batch.clone()).unwrap();
        let noted = app.session.journal.last().unwrap().1.clone();
        // The batch as the app runs it (with the active artboard) before the scale notes its values.
        let mut sent = noted.clone();
        sent["commands"][1]["params"] = json!({"sx": 200});
        let given = files::step_bytes("command.batch", &sent);
        assert!(files::step_bytes("command.batch", &noted) > given, "the scale notes values: {noted}");

        let (mut app, x, moved) = two_documents();
        pad_set_to(&mut app, 0, given);
        let into = app.ui.action_sets[0].actions.last().unwrap().id;
        app.run(START_RECORDING, json!({"set": 0})).unwrap();
        app.ui.recording_into = Some(into);
        app.run("command.batch", batch).unwrap();
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.ui.status, size);
        let scaled = x(&app);
        assert_ne!(scaled, 0.0, "the scale stayed");
        assert_eq!(app.run("object.move", moved).unwrap_err(), size, "no later command runs");
        assert_eq!(x(&app), scaled);
        let pad = app.ui.action_sets[0].actions.last().unwrap().steps.len();
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions.last().unwrap().steps.len(), pad, "the batch's step is left out");
        assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());
    }

    /// Set Options… renaming the set that an action records into has the next recorded command
    /// measure the set's file again: a step that fit before the longer name does not run after it.
    #[test]
    fn a_set_renamed_while_it_records_is_measured_again() {
        let (mut app, x) = app_with_three_moves();
        let moved = recorded_move(&mut app);
        let added = files::step_bytes("object.move", &moved);
        pad_set_to(&mut app, 0, 2 * added + 3);
        let into = app.ui.action_sets[0].actions.last().unwrap().id;
        app.run(START_RECORDING, json!({"set": 0})).unwrap();
        app.ui.recording_into = Some(into);
        app.run("object.move", moved.clone()).unwrap();
        crate::dialogs::action_set_options::open(&mut app, 0);
        let longer = format!("{} 1234", app.ui.action_sets[0].name);
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(longer));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.ui.action_sets[0].name, longer);
        assert_eq!(app.run("object.move", moved).unwrap_err(), "the action set would be larger than 8 MiB");
        assert_eq!(x(&app), 1.0, "only the first move ran");
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert!(files::read_set(&files::write_set(&app.ui.action_sets[0]), "Test").is_ok());
    }

    /// Steps recorded into an action are included in playback, also when the action holds an
    /// exclusion past its last step, as saved preferences can; the set's file stays within 8 MiB.
    #[test]
    fn steps_recorded_after_an_exclusion_past_the_last_step_are_included() {
        let (mut app, _) = app_with_three_moves();
        let moved = recorded_move(&mut app);
        let added = files::step_bytes("object.move", &moved);
        pad_set_to(&mut app, 0, added);
        let pad = app.ui.action_sets[0].actions.last_mut().unwrap();
        pad.excluded_steps.insert(1);
        let into = pad.id;
        app.run(START_RECORDING, json!({"set": 0})).unwrap();
        app.ui.recording_into = Some(into);
        app.run("object.move", moved).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        let pad = app.ui.action_sets[0].actions.last().unwrap();
        assert_eq!((pad.steps.len(), pad.excluded_steps.len()), (2, 0), "the recorded step is included");
        assert_eq!(files::write_set(&app.ui.action_sets[0]).len(), files::MAX_SET_FILE);
    }

    /// Duplicate copies no action or set when the copy would give a file larger than 8 MiB or a set
    /// of more than 1,000 actions, and still copies one that fits.
    #[test]
    fn duplicate_keeps_action_set_files_within_their_limits() {
        let mut app = app_with_sets();
        pad_set_to(&mut app, 1, 0);
        let size = "the action set would be larger than 8 MiB";
        let names = set_names(&app);
        assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 0})).unwrap_err(), size);
        assert_eq!(app.run(DUPLICATE, json!({"set": 1})).unwrap_err(), size, "the copy's name is longer");
        assert_eq!((set_names(&app), app.ui.action_sets[1].actions.len()), (names, 3), "nothing was copied");
        app.ui.action_sets[2].actions = empty_actions(1000);
        let many = "the action set file holds more than 1,000 actions";
        assert_eq!(app.run(DUPLICATE, json!({"set": 2, "action": 0})).unwrap_err(), many);
        assert_eq!(app.ui.action_sets[2].actions.len(), 1000);
        app.ui.action_sets[2].actions.pop();
        app.run(DUPLICATE, json!({"set": 2, "action": 0})).unwrap();
        app.run(DUPLICATE, json!({"set": 2})).unwrap();
        assert_eq!((app.ui.action_sets[2].actions.len(), app.ui.action_sets[3].actions.len()), (1000, 1000));
    }

    /// New Action and Begin Recording add no action and start no recording when the action would
    /// give its set more than 1,000 actions or a file larger than 8 MiB, and New Action adds one that
    /// fits.
    #[test]
    fn new_action_keeps_an_action_set_file_within_its_limits() {
        let mut app = app_with_sets();
        app.ui.action_sets[2].actions = empty_actions(1000);
        let many = "the action set file holds more than 1,000 actions";
        assert_eq!(app.run(NEW_ACTION, json!({"set": 2, "name": "More"})).unwrap_err(), many);
        assert_eq!(app.run(START_RECORDING, json!({"set": 2})).unwrap_err(), many, "Begin Recording too");
        pad_set_to(&mut app, 1, 200);
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.run(NEW_ACTION, json!({"set": 1, "name": "x".repeat(300)})).unwrap_err(), size);
        assert!(app.ui.recording.is_none(), "no recording started");
        assert_eq!((app.ui.action_sets[1].actions.len(), app.ui.action_sets[2].actions.len()), (3, 1000));
        app.run(NEW_ACTION, json!({"set": 1, "name": "More"})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[1].actions.len(), 4);
        assert!(files::write_set(&app.ui.action_sets[1]).len() <= files::MAX_SET_FILE);
    }

    /// Renaming an action, Action Options and Set Options change nothing when the change would give
    /// the set a file larger than 8 MiB, with the steps a recording adds to the set; a change that
    /// fills the file exactly, or that makes a set already past 8 MiB smaller, is made. New Set adds
    /// no set whose file would be larger than 8 MiB.
    #[test]
    fn renaming_keeps_an_action_set_file_within_8_mib() {
        let mut app = app_with_sets();
        pad_set_to(&mut app, 1, 2);
        let size = "the action set would be larger than 8 MiB";
        assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudge Far"})).unwrap_err(), size);
        assert_eq!(app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "color": "red"})).unwrap_err(), size);
        assert_eq!(app.run(ACTION_OPTIONS, json!({"set": 1, "action": 0, "key": 2})).unwrap_err(), size);
        let nudge = &app.ui.action_sets[1].actions[0];
        assert_eq!((nudge.name.as_str(), nudge.key, nudge.color.as_deref()), ("Nudge", None, None), "nothing changed");
        crate::dialogs::action_set_options::open(&mut app, 1);
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Mine and More"));
        assert_eq!(crate::dialogs::confirm(&mut app).unwrap_err(), size);
        assert!(app.ui.dialog.is_some(), "Set Options stays open");
        assert_eq!(app.ui.action_sets[1].name, "Mine");
        app.ui.dialog = None;
        app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudge++"})).unwrap();
        assert_eq!(files::write_set(&app.ui.action_sets[1]).len(), files::MAX_SET_FILE, "the file is exactly 8 MiB");
        app.ui.action_sets[1].actions[1].name = "x".repeat(100);
        app.run(RENAME_ACTION, json!({"set": 1, "action": 1, "name": "Nudge Again"})).unwrap();
        assert_eq!(app.ui.action_sets[1].actions[1].name, "Nudge Again", "a set past 8 MiB gets smaller");

        crate::dialogs::action_set_options::open_new(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("x".repeat(files::MAX_SET_FILE)));
        assert_eq!(crate::dialogs::confirm(&mut app).unwrap_err(), size);
        assert_eq!(app.ui.action_sets.len(), 3, "no set was added");
        app.ui.dialog = None;

        // The steps a recording adds to the set count too.
        let mut app = app_with_sets();
        pad_set_to(&mut app, 1, 1000);
        app.run(START_RECORDING, json!({"set": 1})).unwrap();
        app.session.journal.push(("object.move".to_string(), json!({"pad": "x".repeat(1000)})));
        assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudge+"})).unwrap_err(), size);
        app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Nudg"})).unwrap();
    }

    /// Load Actions… adds no set that Save Actions… would write as a file larger than 8 MiB: a file
    /// with less white space, a file whose set name gets a number, and another app's action set whose
    /// names take more bytes in the file.
    #[test]
    fn load_actions_refuses_a_set_that_would_save_larger_than_8_mib() {
        let mut app = app_with_sets();
        let size = "the action set would be larger than 8 MiB";
        let wide = ActionSet::new("Wide", vec![Action::new("Zeros", vec![("object.move".into(), json!({"p": vec![0; 600_000]}))])]);
        let saved = files::write_set(&wide);
        let packed = serde_json::to_string(&serde_json::from_str::<Value>(&saved).unwrap()).unwrap();
        assert!(packed.len() <= files::MAX_SET_FILE && saved.len() > files::MAX_SET_FILE);
        assert_eq!(app.run(LOAD_SET, json!({ "data": packed })).unwrap_err(), size);

        let mut full = app_with_sets();
        full.ui.action_sets[1].actions.clear();
        pad_set_to(&mut full, 1, 0);
        let mine = files::write_set(&full.ui.action_sets[1]);
        assert_eq!(app.run(LOAD_SET, json!({ "data": mine })).unwrap_err(), size, "Mine 2 is longer");
        let yours = mine.replacen("\"Mine\"", "\"Your\"", 1);
        assert_eq!(app.run(LOAD_SET, json!({ "data": yours })).unwrap()["name"], "Your");

        let name = "\u{1}".repeat(1024);
        let steps = [("Group", true); 40];
        let text = aia_text("Names", &vec![(name.as_str(), steps.as_slice()); 1000]);
        assert!(text.len() <= files::MAX_SET_FILE);
        assert_eq!(app.run(LOAD_SET, json!({ "data": text })).unwrap_err(), size);
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other", "Your"]);
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
    fn delete_step_removes_one_step_and_moves_the_exclusions_up() {
        let (mut app, _) = app_with_a_stop(false);
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 1, 3, usize::MAX]);
        assert_eq!(app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 1})).unwrap(), json!({"set": 0, "action": 0, "step": 1}));
        let action = &app.ui.action_sets[0].actions[0];
        let ids: Vec<&str> = action.steps.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(ids, ["object.move", "object.move", "object.move"]);
        assert_eq!(action.excluded_steps, BTreeSet::from([0, 2]), "the deleted step's exclusion goes and a later one moves up");
    }

    #[test]
    fn delete_step_refuses_bad_params_and_changes_nothing() {
        let (mut app, _) = app_with_three_moves();
        for (params, want) in [
            (json!({}), "no such action"),
            (json!(5), "no such action"),
            (json!([1]), "no such action"),
            (json!({"set": 1, "action": 0, "step": 0}), "no such action"),
            (json!({"set": 0, "action": 1, "step": 0}), "no such action"),
            (json!({"set": "0", "action": 0, "step": 0}), "no such action"),
            (json!({"set": 0, "action": 0}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": 3}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": -1}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": null}), "no such action step"),
        ] {
            assert_eq!(app.run(DELETE_STEP, params.clone()).unwrap_err(), want, "{params}");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "{params}");
        }
    }

    #[test]
    fn delete_step_keeps_an_open_record_stop_on_its_place() {
        let (mut app, _) = app_with_three_moves();
        crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(2));
        app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["step"], json!(1), "a deleted step above moves it up");
        app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 1})).unwrap();
        assert_eq!(open_kind(&app), None, "deleting its step closes it");
    }

    /// The steps of action `action` in set `set`: each command, with its `dx` when it has one.
    fn step_names(app: &VectorcraftApp, set: usize, action: usize) -> Vec<String> {
        app.ui.action_sets[set].actions[action]
            .steps
            .iter()
            .map(|(c, p)| match p.get("dx") {
                Some(dx) => format!("{c} {dx}"),
                None => c.clone(),
            })
            .collect()
    }

    /// The bottom bar's Stop and Begin Recording buttons, as drawn last.
    fn stop_and_record(ctx: &egui::Context, app: &mut VectorcraftApp) -> (egui::Pos2, egui::Pos2) {
        super::super::tests_appearance::frame_events(ctx, app, vec![], show);
        let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [stop, record, ..] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        (stop.center(), record.center())
    }

    #[test]
    fn begin_recording_selected_action_adds_steps_without_new_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
        let (stop, record) = stop_and_record(&ctx, &mut app);
        click(&ctx, &mut app, record, show);
        app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
        click(&ctx, &mut app, stop, show);
        assert_eq!(app.ui.action_sets[0].actions.len(), 1, "no new action");
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2", "object.move 4", "object.move 8"], "added at the end");

        // With a step selected, the recorded steps go after it.
        click_first_step(&ctx, &mut app);
        // The steps shown move the bottom bar down.
        let (stop, record) = stop_and_record(&ctx, &mut app);
        click(&ctx, &mut app, record, show);
        app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
        click(&ctx, &mut app, stop, show);
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 16", "object.move 2", "object.move 4", "object.move 8"]);

        // With no action selected, Begin Recording still adds a new action.
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "▾ Moves").center(), show);
        let (stop, record) = stop_and_record(&ctx, &mut app);
        click(&ctx, &mut app, record, show);
        app.run("object.move", json!({"dx": 32, "dy": 0})).unwrap();
        click(&ctx, &mut app, stop, show);
        let names: Vec<&str> = app.ui.action_sets[0].actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Three Moves", "Action 2"]);
        assert_eq!(step_names(&app, 0, 1), ["object.move 32"]);
    }

    #[test]
    fn begin_recording_after_selected_step_preserves_order_and_exclusions() {
        let (mut app, _) = app_with_three_moves();
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([0, 2]);
        for (params, want) in [
            (json!({"action": 0}), "no such action"),
            (json!({"set": 0, "after": 0}), "no such action"),
            (json!({"set": 1, "action": 0}), "no such action"),
            (json!({"set": 0, "action": 1}), "no such action"),
            (json!({"set": 0, "action": "0"}), "no such action"),
            (json!({"set": 0, "action": 0, "after": 3}), "no such action step"),
            (json!({"set": 0, "action": 0, "after": null}), "no such action step"),
            (json!({"set": 0, "action": 0, "after": -1}), "no such action step"),
        ] {
            assert_eq!(app.run(START_RECORDING, params.clone()).unwrap_err(), want, "{params}");
            assert!(app.ui.recording.is_none(), "{params}");
        }
        assert_eq!(
            app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 0})).unwrap(),
            json!({"set": 0, "action": 0, "name": "Three Moves"})
        );
        app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
        app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions.len(), 1, "no new action");
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 8", "object.move 16", "object.move 2", "object.move 4"]);
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([0, 4]), "the steps keep whether they are excluded");
    }

    #[test]
    fn steps_recorded_into_the_middle_of_an_action_keep_the_selected_step_and_dialog_on_their_steps() {
        let (mut app, _) = app_with_three_moves();
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let id = app.ui.action_sets[0].actions[0].id;
        app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 0})).unwrap();
        app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
        app.run("object.move", json!({"dx": 16, "dy": 0})).unwrap();
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((id, 2)));
        crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(1));
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 8", "object.move 16", "object.move 2", "object.move 4"]);
        assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), Some((id, 4)), "the move by 4 stays selected");
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["step"], json!(3), "Record Stop stays after the move by 2");
    }

    #[test]
    fn step_edits_while_recording_into_an_action_keep_the_recording_in_place() {
        let (mut app, _) = app_with_three_moves();
        // The recording goes between the moves by 2 and 4.
        app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 1})).unwrap();
        app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
        app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 2, "before": 0})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(step_names(&app, 0, 0), ["object.move 4", "object.move 2", STOP_STEP, "object.move 8"]);

        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({"set": 0, "action": 0, "after": 1})).unwrap();
        app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
        app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        assert_eq!(step_names(&app, 0, 0), ["object.move 2", "object.move 8", "object.move 4"], "a step deleted before the place");
    }

    #[test]
    fn move_step_within_action_updates_order_exclusions_and_selection() {
        let (mut app, _) = app_with_three_moves();
        app.ui.action_sets[0].actions[0].steps.push(("object.move".into(), json!({"dx": 8, "dy": 0})));
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([1]);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let id = app.ui.action_sets[0].actions[0].id;
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((id, 3)));
        let selected_step = |ctx: &egui::Context| pstate::<Option<(u64, usize)>>(ctx, SELECTED_STEP);
        assert_eq!(
            app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 1, "before": 4})).unwrap(),
            json!({"set": 0, "action": 0, "step": 3, "moved": true})
        );
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 4", "object.move 8", "object.move 2"]);
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([3]), "the moved step stays excluded");
        assert_eq!(selected_step(&ctx), Some((id, 2)), "the selected step stays selected");
        assert_eq!(
            app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 2, "before": 0})).unwrap(),
            json!({"set": 0, "action": 0, "step": 0, "moved": true})
        );
        assert_eq!(step_names(&app, 0, 0), ["object.move 8", "object.move 1", "object.move 4", "object.move 2"]);
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([3]));
        assert_eq!(selected_step(&ctx), Some((id, 0)), "the moved selected step is selected where it went");
        for before in [1, 2] {
            assert_eq!(
                app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 1, "before": before})).unwrap(),
                json!({"set": 0, "action": 0, "step": 1, "moved": false})
            );
        }
        assert_eq!(step_names(&app, 0, 0), ["object.move 8", "object.move 1", "object.move 4", "object.move 2"], "a drop on itself moves nothing");
        crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(3));
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 3, "before": 0})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["step"], json!(0), "an open Record Stop follows its step");
    }

    #[test]
    fn move_step_between_actions_and_sets_keeps_file_roundtrip() {
        let (mut app, _) = app_with_three_moves();
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([2]);
        let rotate = ("object.rotate".to_string(), json!({"angle": 5}));
        app.ui.action_sets.push(ActionSet::new("Other", vec![Action::new("Empty", vec![]), Action::new("One", vec![rotate])]));
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let (id, empty) = (app.ui.action_sets[0].actions[0].id, app.ui.action_sets[1].actions[0].id);
        set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((0, 0)));
        set_pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP, Some((id, 2)));
        assert_eq!(
            app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 2, "toSet": 1, "toAction": 0, "before": 0})).unwrap(),
            json!({"set": 1, "action": 0, "step": 0, "moved": true})
        );
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2"]);
        assert!(app.ui.action_sets[0].actions[0].excluded_steps.is_empty());
        assert_eq!(step_names(&app, 1, 0), ["object.move 4"], "an empty action takes a step");
        assert_eq!(app.ui.action_sets[1].actions[0].excluded_steps, BTreeSet::from([0]), "the moved step stays excluded");
        assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), Some((empty, 0)));
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((1, 0)));
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.as_slice()), (Some(1), &[1][..]));
        assert!(pstate::<BTreeSet<u64>>(&ctx, "actions-expanded").contains(&empty), "the step's new action shows its steps");
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 0, "toSet": 1, "toAction": 1, "before": 0})).unwrap();
        assert_eq!(step_names(&app, 1, 1), ["object.move 1", "object.rotate"]);
        assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), Some((empty, 0)), "an unmoved selected step stays");
        let loaded = files::read_set(&files::write_set(&app.ui.action_sets[1]), "Other").unwrap();
        for (a, b) in loaded.actions.iter().zip(&app.ui.action_sets[1].actions) {
            assert_eq!((&a.name, &a.steps, &a.excluded_steps), (&b.name, &b.steps, &b.excluded_steps));
        }
    }

    #[test]
    fn move_step_rejects_invalid_and_oversize_destinations_without_changes() {
        let (mut app, _) = app_with_three_moves();
        let full = Action::new("Full", vec![("object.move".to_string(), json!({"dx": 0})); files::MAX_ACTION_STEPS]);
        let big = |text: String| ActionSet::new("Big", vec![Action::new("Big", vec![("document.note".to_string(), json!({ "s": text }))])]);
        let room = files::MAX_SET_FILE - files::write_set(&big(String::new())).len() - 4;
        app.ui.action_sets.push(ActionSet::new("Full", vec![full]));
        app.ui.action_sets.push(big("x".repeat(room)));
        assert!(files::write_set(&app.ui.action_sets[2]).len() <= files::MAX_SET_FILE);
        let before: Vec<String> = app.ui.action_sets.iter().map(files::write_set).collect();
        for (params, want) in [
            (json!({}), "no such action"),
            (json!({"set": 0, "action": 1, "step": 0, "before": 0}), "no such action"),
            (json!({"set": 3, "action": 0, "step": 0, "before": 0}), "no such action"),
            (json!({"set": 0, "action": 0, "step": 3, "before": 0}), "no such action step"),
            (json!({"set": 0, "action": 0, "before": 0}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": 0}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": 0, "before": 4}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": 0, "before": "1"}), "no such action step"),
            (json!({"set": 0, "action": 0, "step": 0, "toSet": 3, "before": 0}), "no such action set"),
            (json!({"set": 0, "action": 0, "step": 0, "toSet": null, "before": 0}), "no such action set"),
            (json!({"set": 0, "action": 0, "step": 0, "toSet": 1, "toAction": 1, "before": 0}), "no such action"),
            (json!({"set": 0, "action": 0, "step": 0, "toSet": 1, "toAction": 0, "before": 0}), "the action already has 10,000 steps"),
            (json!({"set": 0, "action": 0, "step": 0, "toSet": 2, "toAction": 0, "before": 1}), "the action set would be larger than 8 MiB"),
        ] {
            assert_eq!(app.run(MOVE_STEP, params.clone()).unwrap_err(), want, "{params}");
            assert_eq!(app.ui.action_sets.iter().map(files::write_set).collect::<Vec<_>>(), before, "{params}");
        }
    }

    #[test]
    fn drag_step_shows_insertion_line_and_moves_once() {
        use egui::{Event, PointerButton};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        set_pstate(&ctx, "actions-expanded", BTreeSet::from([app.ui.action_sets[0].actions[0].id]));
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
        let mut rows: Vec<egui::Rect> = texts.iter().filter(|(s, _)| s == "object.move").map(|(_, r)| *r).collect();
        rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
        let (from, to) = (rows[0].center(), egui::pos2(rows[0].center().x, rows[2].center().y + 4.0));
        let accent = Tokens::get(&ctx).accent;
        // Whether a frame with `events` draws the insertion line.
        let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            fn line(s: &egui::Shape, accent: egui::Color32) -> bool {
                match s {
                    egui::Shape::LineSegment { stroke, .. } => stroke.width == 2.0 && stroke.color == accent,
                    egui::Shape::Vec(v) => v.iter().any(|s| line(s, accent)),
                    _ => false,
                }
            }
            out.shapes.iter().any(|c| line(&c.shape, accent))
        };
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        assert!(!frame(&mut app, vec![Event::PointerMoved(from)]), "no line before a drag");
        frame(&mut app, vec![button(from, true)]);
        frame(&mut app, vec![Event::PointerMoved(from + egui::vec2(0.0, 12.0))]);
        assert!(frame(&mut app, vec![Event::PointerMoved(to)]), "the line shows where the step goes");
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2", "object.move 4"], "nothing moves while dragging");
        frame(&mut app, vec![button(to, false)]);
        assert_eq!(step_names(&app, 0, 0), ["object.move 2", "object.move 4", "object.move 1"], "the release moves the step");
        for _ in 0..3 {
            assert!(!frame(&mut app, vec![]), "no line after the drop");
        }
        assert_eq!(step_names(&app, 0, 0), ["object.move 2", "object.move 4", "object.move 1"], "the step moves once");
    }

    #[test]
    fn drag_step_released_off_the_rows_leaves_it_in_place_without_an_undo_step() {
        use egui::{Event, PointerButton};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        set_pstate(&ctx, "actions-expanded", BTreeSet::from([app.ui.action_sets[0].actions[0].id]));
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
        let mut rows: Vec<egui::Rect> = texts.iter().filter(|(s, _)| s == "object.move").map(|(_, r)| *r).collect();
        rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
        let from = rows[0].center();
        let over = egui::pos2(from.x, rows[2].center().y + 4.0);
        let off = egui::pos2(from.x, rows[2].bottom() + 2_000.0);
        let undo = undo_steps(&app);
        let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![button(from, true)]);
        frame(&mut app, vec![Event::PointerMoved(from + egui::vec2(0.0, 12.0))]);
        frame(&mut app, vec![Event::PointerMoved(over)]);
        frame(&mut app, vec![Event::PointerMoved(off)]);
        frame(&mut app, vec![button(off, false)]);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2", "object.move 4"], "the step stays where it was");
        assert_eq!(undo_steps(&app), undo, "no undo step");
        assert_eq!(pstate::<Option<DraggedStep>>(&ctx, DRAGGED_STEP), None, "the drag ends");
    }

    #[test]
    fn drag_step_ends_without_a_move_when_a_command_changes_its_action() {
        use egui::{Event, PointerButton};
        let (mut app, _) = app_with_three_moves();
        let rotate = |angle: u32| ("object.rotate".to_string(), json!({"angle": angle}));
        app.ui.action_sets[0].actions.push(Action::new("Turns", vec![rotate(5), rotate(6)]));
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        set_pstate(&ctx, "actions-expanded", BTreeSet::from([app.ui.action_sets[0].actions[0].id]));
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
        let mut rows: Vec<egui::Rect> = texts.iter().filter(|(s, _)| s == "object.move").map(|(_, r)| *r).collect();
        rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
        let (from, to) = (rows[0].center(), egui::pos2(rows[0].center().x, rows[1].center().y + 4.0));
        let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let start_drag = |app: &mut VectorcraftApp| {
            frame(app, vec![button(from, true)]);
            frame(app, vec![Event::PointerMoved(from + egui::vec2(0.0, 12.0))]);
        };
        // A command on another action leaves the drag as it is.
        start_drag(&mut app);
        app.run(DELETE_STEP, json!({"set": 0, "action": 1, "step": 0})).unwrap();
        frame(&mut app, vec![Event::PointerMoved(to)]);
        frame(&mut app, vec![button(to, false)]);
        assert_eq!(step_names(&app, 0, 0), ["object.move 2", "object.move 1", "object.move 4"], "the dragged step moves");
        // A command that deletes the dragged step's action's first step puts another step at the
        // dragged index.
        start_drag(&mut app);
        app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        frame(&mut app, vec![Event::PointerMoved(to)]);
        assert_eq!(pstate::<Option<DraggedStep>>(&ctx, DRAGGED_STEP), None, "the drag ends");
        frame(&mut app, vec![button(to, false)]);
        assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 4"], "no other step moves");
        // Two equal steps, one excluded, that change places put the other one at the dragged index.
        let same = ("object.move".to_string(), json!({"dx": 1, "dy": 0}));
        app.ui.action_sets[0].actions[0].steps = vec![same.clone(), same];
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([1]);
        start_drag(&mut app);
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 1, "before": 0})).unwrap();
        frame(&mut app, vec![Event::PointerMoved(to)]);
        frame(&mut app, vec![button(to, false)]);
        assert_eq!(app.ui.action_sets[0].actions[0].excluded_steps, BTreeSet::from([0]), "the excluded step stays first");
    }

    #[test]
    fn step_and_set_edits_keep_an_open_insert_menu_item_on_its_place() {
        let (mut app, _) = app_with_three_moves();
        app.ui.action_sets[0].actions.insert(0, Action::new("First", vec![]));
        crate::dialogs::action_insert_menu_item::open(&mut app, Some((0, 1)), Some(2));
        app.run(DUPLICATE, json!({"set": 0, "action": 0})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["action"], json!(2), "a copied action above moves it down");
        app.run(DELETE_STEP, json!({"set": 0, "action": 2, "step": 0})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["step"], json!(1), "a deleted step above moves it up");
        app.run(DELETE_STEP, json!({"set": 0, "action": 2, "step": 1})).unwrap();
        assert_eq!(open_kind(&app), None, "deleting its step closes it");
        crate::dialogs::action_insert_menu_item::open(&mut app, Some((0, 0)), None);
        app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
        assert_eq!(open_kind(&app), None, "deleting its set closes it");
    }

    /// Selects the first step of "Three Moves" by clicking its row, with the action open.
    fn click_first_step(ctx: &egui::Context, app: &mut VectorcraftApp) {
        use super::super::tests_appearance::{click, frame_events};
        set_pstate(ctx, "actions-expanded", BTreeSet::from([app.ui.action_sets[0].actions[0].id]));
        let texts = frame_events(ctx, app, vec![], show);
        let first = texts.iter().filter(|(s, _)| s == "object.move").map(|(_, r)| *r).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
        click(ctx, app, first.center(), show);
    }

    #[test]
    fn delete_selection_on_a_selected_step_deletes_that_step_and_keeps_the_action() {
        use super::super::tests_appearance::click;
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        app.ui.action_sets[0].actions[0].excluded_steps = BTreeSet::from([2]);
        click_first_step(&ctx, &mut app);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        let action = &app.ui.action_sets[0].actions[0];
        assert_eq!(action.name, "Three Moves", "the action stays");
        assert_eq!(action.steps.len(), 2, "one step goes");
        assert_eq!(action.excluded_steps, BTreeSet::from([1]));
        assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), None, "no step stays selected");
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((0, 0)), "the action stays selected");

        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert!(app.ui.action_sets[0].actions.is_empty(), "with no step selected, the selected action goes");
    }

    #[test]
    fn the_panel_menus_delete_deletes_the_selected_step_and_a_stop_inserted_there() {
        use super::super::tests_appearance::{click, frame_events};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        click_first_step(&ctx, &mut app);
        menu_insert_stop(&ctx, &mut app, "Paint it", false);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 4);
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Delete").map(|(_, r)| *r).expect("Delete in the menu");
        click(&ctx, &mut app, item.center(), menu);
        let action = &app.ui.action_sets[0].actions[0];
        assert!(action.steps.iter().all(|(c, _)| c == "object.move"), "the new stop was selected, and it goes");
        assert_eq!(action.steps.len(), 3);
    }

    #[test]
    fn a_stop_inserted_into_another_action_selects_it_and_delete_selection_deletes_the_stop() {
        use super::super::tests_appearance::click;
        let (mut app, _) = app_with_three_moves();
        let step = ("object.move".to_string(), json!({"dx": 8, "dy": 0}));
        app.ui.action_sets[0].actions.push(Action::new("Same Set", vec![step.clone()]));
        app.ui.action_sets.push(ActionSet::new("More", vec![Action::new("Other Set", vec![step])]));
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        for (set, at) in [(0, 1), (1, 0)] {
            app.ui.action_set = Some(0);
            app.ui.action_sets_selected = vec![0];
            set_pstate::<Option<(usize, usize)>>(&ctx, "actions-selected", Some((0, 0)));
            app.run(INSERT_STOP, json!({"set": set, "action": at, "message": "Paint it"})).unwrap();
            assert_eq!(app.ui.action_set, Some(set));
            assert_eq!(app.ui.action_sets_selected, vec![set]);
            let selected = pstate::<Option<(usize, usize)>>(&ctx, "actions-selected");
            assert_eq!(selected, Some((set, at)), "the stop's action is the selected one");
            assert_eq!(selected_step(&app, &ctx, selected), Some(1), "the new stop is the selected step");
            let delete = trash(&ctx, &mut app);
            click(&ctx, &mut app, delete, show);
            let action = &app.ui.action_sets[set].actions[at];
            assert!(action.steps.iter().all(|(c, _)| c == "object.move"), "the stop goes");
            assert_eq!(action.steps.len(), 1, "the command stays");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "the action selected before keeps its steps");
        }
        assert_eq!(app.ui.action_sets[0].actions.len(), 2);
        assert_eq!(app.ui.action_sets[1].actions.len(), 1);
    }

    #[test]
    fn a_step_in_a_closed_action_is_not_the_selection() {
        use super::super::tests_appearance::click;
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        click_first_step(&ctx, &mut app);
        set_pstate(&ctx, "actions-expanded", BTreeSet::<u64>::new());
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert!(app.ui.action_sets[0].actions.is_empty(), "the step's row is hidden and the action's row is the one highlighted");
    }

    #[test]
    fn a_stop_inserted_while_recording_goes_at_the_recording_position() {
        let (mut app, _) = app_with_three_moves();
        app.run(START_RECORDING, json!({})).unwrap();
        app.run("object.move", json!({"dx": 1, "dy": 0})).unwrap();
        assert_eq!(app.run(INSERT_STOP, json!({"message": "Now", "allowContinue": true})).unwrap(), json!({"recording": true}));
        app.run("object.move", json!({"dx": 2, "dy": 0})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        let ids: Vec<&str> = recorded.steps.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(ids, ["object.move", STOP_STEP, "object.move"]);
        assert_eq!(recorded.steps[1].1, json!({"message": "Now", "allowContinue": true}));
    }

    #[test]
    fn accelerated_playback_holds_at_a_stop_and_play_plays_the_rest() {
        let (mut app, x) = app_with_a_stop(false);
        let undo = undo_steps(&app);
        let r = play(&mut app, 0, 0).unwrap();
        assert_eq!((r["pending"].clone(), r["atStop"].clone(), r["results"].as_array().map(Vec::len)), (json!(2), json!(true), Some(1)));
        assert_eq!(x(&app), 1.0, "the step before the stop runs");
        let d = app.ui.dialog.as_ref().expect("the stop shows its message");
        assert_eq!(
            (d.kind.as_str(), d.str("action"), d.str("message"), d.bool("allowContinue")),
            (crate::dialogs::action_stop::MESSAGE_KIND, "Three Moves".to_string(), "Paint it".to_string(), false)
        );
        for now in [0.0, 100.0, 1000.0] {
            assert_eq!(frame(&mut app, now), None);
            assert_eq!(x(&app), 1.0, "no later step runs while the action waits");
        }
        assert_eq!(play(&mut app, 0, 0).unwrap_err(), "an action is already playing");
        let r = app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!((r["pending"].clone(), r["atStop"].clone()), (json!(0), Value::Null));
        assert_eq!(x(&app), 7.0, "Play runs every step after the stop");
        assert!(app.ui.action_playing.is_none());
        assert_eq!(open_kind(&app), None, "the message box closes");
        assert_eq!(undo_steps(&app), undo + 2, "the steps on each side of the stop are one undo step each");
        assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap_err(), "no action is waiting at a stop");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(x(&app), 1.0, "one undo takes back the steps after the stop");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(x(&app), 0.0, "and one more the steps before it");
    }

    #[test]
    fn step_by_step_and_pause_for_hold_at_a_stop() {
        for (speed, seconds) in [("step", 1), ("pause", 2)] {
            let (mut app, x) = app_with_a_stop(false);
            app.run(PLAYBACK_OPTIONS, json!({"speed": speed, "seconds": seconds})).unwrap();
            let r = play(&mut app, 0, 0).unwrap();
            assert_eq!((r["pending"].clone(), r["atStop"].clone()), (json!(3), Value::Null), "{speed}");
            assert_eq!(open_kind(&app), None, "{speed}: the stop comes after the step before it is shown");
            let mut now = 0.0;
            while !waiting_at_stop(&app) {
                assert!(now < 100.0, "{speed}: the playback reaches the stop");
                frame(&mut app, now);
                now += 1.0;
            }
            assert_eq!(open_kind(&app), Some(crate::dialogs::action_stop::MESSAGE_KIND), "{speed}");
            for _ in 0..5 {
                assert_eq!(frame(&mut app, now + 1000.0), None, "{speed}");
                assert_eq!(x(&app), 1.0, "{speed}: no later step runs while the action waits");
            }
            let r = app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
            assert_eq!(r["pending"], json!(1), "{speed}: Play runs the next step, and the last stays for its turn");
            assert_eq!(x(&app), 3.0, "{speed}");
            for _ in 0..20 {
                if app.ui.action_playing.is_none() {
                    break;
                }
                frame(&mut app, now);
                now += 10.0;
            }
            assert_eq!(x(&app), 7.0, "{speed}: the paced playback goes on to the end");
            assert!(app.ui.action_playing.is_none(), "{speed}");
        }
    }

    #[test]
    fn a_paced_playback_holds_at_a_stop_when_another_document_became_active() {
        let (mut app, _) = app_with_a_stop(false);
        app.run(PLAYBACK_OPTIONS, json!({"speed": "pause", "seconds": 1})).unwrap();
        play(&mut app, 0, 0).unwrap();
        assert_eq!(tick_playback(&mut app, 0.0), Some(1.0));
        app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
        app.run("shape.rectangle", json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
        assert_eq!(tick_playback(&mut app, 1.0), None);
        assert!(waiting_at_stop(&app), "the stop holds the playback, as it does at Accelerated");
        assert_ne!(app.ui.status, "the action stopped playing: its document is no longer active");
        let r = app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!(r["pending"], json!(1), "Continue runs the next step in the document active now");
    }

    #[test]
    fn allow_continue_offers_continue_and_stop_closes_the_box_and_keeps_waiting() {
        let (mut app, x) = app_with_a_stop(true);
        play(&mut app, 0, 0).unwrap();
        assert!(dialog_texts(&mut app).iter().any(|s| s == "Continue"), "Allow Continue shows Continue");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(x(&app), 7.0, "Continue plays the rest");
        assert!(app.ui.action_playing.is_none());
        assert_eq!(open_kind(&app), None);

        let (mut app, x) = app_with_a_stop(false);
        play(&mut app, 0, 0).unwrap();
        let texts = dialog_texts(&mut app);
        assert!(!texts.iter().any(|s| s == "Continue"), "without Allow Continue there is no Continue: {texts:?}");
        assert!(texts.iter().any(|s| s == "Paint it") && texts.iter().any(|s| s == "Stop"), "{texts:?}");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(open_kind(&app), None, "the box's only button closes it");
        assert!(waiting_at_stop(&app), "the action waits for the manual task");
        assert_eq!(x(&app), 1.0);
        frame(&mut app, 100.0);
        assert_eq!(open_kind(&app), None, "the box does not open again");
        app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!(x(&app), 7.0);

        let (mut app, x) = app_with_a_stop(true);
        play(&mut app, 0, 0).unwrap();
        crate::dialogs::cancel(&mut app);
        assert!(waiting_at_stop(&app), "Stop closes the box and the action waits");
        assert_eq!(x(&app), 1.0);
    }

    #[test]
    fn play_in_the_panel_plays_on_from_a_stop_with_nothing_selected() {
        use super::super::tests_appearance::{click, frame_events};
        let (mut app, x) = app_with_a_stop(false);
        play(&mut app, 0, 0).unwrap();
        crate::dialogs::cancel(&mut app);
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        assert!(texts.iter().any(|(s, _)| s.contains("Play Current Selection")), "the panel says how to go on: {texts:?}");
        let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [_, _, play_button, _, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        click(&ctx, &mut app, play_button.center(), show);
        assert_eq!(x(&app), 7.0);
        assert!(app.ui.action_playing.is_none());

        let (mut app, x) = app_with_a_stop(false);
        play(&mut app, 0, 0).unwrap();
        crate::dialogs::cancel(&mut app);
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Play Current Selection").map(|(_, r)| *r).expect("the item in the menu");
        click(&ctx, &mut app, item.center(), menu);
        assert_eq!(x(&app), 7.0, "the panel menu's item plays on too, with nothing selected");
    }

    #[test]
    fn stop_at_a_stop_ends_the_playback_and_keeps_the_steps_run() {
        let (mut app, x) = app_with_a_stop(false);
        let undo = undo_steps(&app);
        play(&mut app, 0, 0).unwrap();
        assert_eq!(app.run(STOP_RECORDING, json!({})).unwrap(), json!({"stopped": true}));
        assert!(app.ui.action_playing.is_none());
        assert_eq!(open_kind(&app), None, "the message box closes");
        assert_eq!(frame(&mut app, 100.0), None);
        assert_eq!(x(&app), 1.0, "the steps after the stop do not run; the one played stays");
        assert_eq!(undo_steps(&app), undo + 1);
        assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap_err(), "no action is waiting at a stop");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(x(&app), 0.0, "the step played undoes");
        play(&mut app, 0, 0).unwrap();
        assert!(waiting_at_stop(&app), "the action plays again after Stop");
    }

    #[test]
    fn stops_first_last_and_in_a_row_each_wait() {
        let (mut app, x) = app_with_three_moves();
        let stop = |message: &str| (STOP_STEP.to_string(), json!({"message": message, "allowContinue": false}));
        let moved = ("object.move".to_string(), json!({"dx": 1, "dy": 0}));
        app.ui.action_sets[0].actions[0].steps = vec![stop("first"), stop("second"), moved, stop("last")];
        let message = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.str("message"));
        assert_eq!(play(&mut app, 0, 0).unwrap()["atStop"], json!(true));
        assert_eq!((message(&app).as_deref(), x(&app)), (Some("first"), 0.0));
        assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap()["atStop"], json!(true));
        assert_eq!((message(&app).as_deref(), x(&app)), (Some("second"), 0.0));
        assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap()["atStop"], json!(true));
        assert_eq!((message(&app).as_deref(), x(&app)), (Some("last"), 1.0));
        assert_eq!(app.run(CONTINUE_PLAYBACK, json!({})).unwrap(), json!({"results": [], "pending": 0}));
        assert!(app.ui.action_playing.is_none(), "after the last stop the playback ends");
        assert_eq!(message(&app), None);
    }

    #[test]
    fn a_stop_with_an_empty_message_still_waits() {
        let (mut app, x) = app_with_three_moves();
        app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
        play(&mut app, 0, 0).unwrap();
        assert!(waiting_at_stop(&app));
        assert_eq!(x(&app), 1.0);
        assert_eq!(open_kind(&app), Some(crate::dialogs::action_stop::MESSAGE_KIND));
    }

    #[test]
    fn an_excluded_stop_does_not_hold_and_batch_refuses_an_included_one() {
        let (mut app, x) = app_with_a_stop(false);
        assert_eq!(crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap_err(), "Batch cannot play an action with a stop");
        assert_eq!(open_kind(&app), None);
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap();
        crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap();
        let d = app.ui.dialog.take().unwrap();
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": true})).unwrap();
        app.ui.dialog = Some(d);
        assert_eq!(crate::dialogs::confirm(&mut app).unwrap_err(), "Batch cannot play an action with a stop", "a stop included since it opened");
        app.ui.dialog = None;
        app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap();
        play(&mut app, 0, 0).unwrap();
        assert!(app.ui.action_playing.is_none());
        assert_eq!(x(&app), 7.0, "the excluded stop does not hold the playback");
    }

    #[test]
    fn a_stop_nested_in_a_recorded_batch_holds_too() {
        let (mut app, x) = app_with_three_moves();
        let moved = json!({"command": "object.move", "params": {"dx": 1, "dy": 0}});
        let stop = json!({"command": STOP_STEP, "params": {"message": "Inner", "allowContinue": false}});
        app.ui.action_sets[0].actions[0].steps = vec![("command.batch".into(), json!({"commands": [moved, stop, moved]}))];
        play(&mut app, 0, 0).unwrap();
        assert!(waiting_at_stop(&app));
        assert_eq!(x(&app), 1.0);
        app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!(x(&app), 2.0);
    }

    #[test]
    fn a_stop_reached_while_recording_is_recorded() {
        let (mut app, _) = app_with_a_stop(false);
        app.run(START_RECORDING, json!({})).unwrap();
        play(&mut app, 0, 0).unwrap();
        app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        let ids: Vec<&str> = recorded.steps.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(ids, ["command.batch", STOP_STEP, "command.batch"]);
        assert_eq!(recorded.steps[1].1, json!({"message": "Paint it", "allowContinue": false}));
    }

    #[test]
    fn the_message_waits_for_a_dialog_already_open() {
        let (mut app, _) = app_with_a_stop(false);
        crate::dialogs::action_playback::open(&mut app);
        play(&mut app, 0, 0).unwrap();
        assert!(waiting_at_stop(&app));
        frame(&mut app, 0.0);
        assert_eq!(open_kind(&app), Some(crate::dialogs::action_playback::KIND), "the open dialog stays");
        crate::dialogs::cancel(&mut app);
        frame(&mut app, 0.0);
        assert_eq!(open_kind(&app), Some(crate::dialogs::action_stop::MESSAGE_KIND), "the message shows once it closes");
    }

    #[test]
    fn play_on_from_a_stop_uses_the_document_active_then() {
        let (mut app, x) = app_with_a_stop(false);
        play(&mut app, 0, 0).unwrap();
        app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert_eq!(x(&app), 7.0);
        let (mut app, _) = app_with_a_stop(false);
        app.run(PLAYBACK_OPTIONS, json!({"speed": "step"})).unwrap();
        play(&mut app, 0, 0).unwrap();
        frame(&mut app, 0.0);
        frame(&mut app, 0.0);
        assert!(waiting_at_stop(&app));
        app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run(CONTINUE_PLAYBACK, json!({})).unwrap();
        assert!(app.ui.action_playing.is_some(), "a document opened at the stop does not end the playback");
        let bounds = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert_eq!(bounds.x0, 2.0, "the next step runs on the document active now");
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
        let later = json!({"format": "vcactions", "version": 7, "name": "New", "actions": []});
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

    /// The panel menu's Insert Stop…, with Record Stop's fields as given, and OK.
    fn menu_insert_stop(ctx: &egui::Context, app: &mut VectorcraftApp, message: &str, allow_continue: bool) {
        use super::super::tests_appearance::{click, frame_events};
        let texts = frame_events(ctx, app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Insert Stop…").map(|(_, r)| *r).expect("Insert Stop… in the menu");
        click(ctx, app, item.center(), menu);
        let d = app.ui.dialog.as_mut().expect("Insert Stop… opens Record Stop");
        assert_eq!(d.kind, crate::dialogs::action_stop::RECORD_KIND);
        d.fields.insert("message".into(), json!(message));
        d.fields.insert("allowContinue".into(), json!(allow_continue));
        crate::dialogs::confirm(app).unwrap();
        assert!(app.ui.dialog.is_none(), "OK closes Record Stop");
    }

    #[test]
    fn insert_stop_in_the_menu_goes_after_the_selected_step_or_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let texts = frame_events(&ctx, &mut app, vec![], show);
        let label = text_rect(&texts, "Three Moves");
        click(&ctx, &mut app, label.center(), show);
        menu_insert_stop(&ctx, &mut app, "At the end", false);
        let ids = |app: &VectorcraftApp| app.ui.action_sets[0].actions[0].steps.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&app).iter().position(|c| c == STOP_STEP), Some(3), "the action selected: the stop goes at its end");
        assert!(pstate::<BTreeSet<u64>>(&ctx, "actions-expanded").contains(&app.ui.action_sets[0].actions[0].id), "the action opens to show it");

        let texts = frame_events(&ctx, &mut app, vec![], show);
        assert!(texts.iter().any(|(s, _)| s == "Stop: At the end"), "the step's row shows the message: {texts:?}");
        let first = texts.iter().filter(|(s, _)| s == "object.move").map(|(_, r)| *r).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
        click(&ctx, &mut app, first.center(), show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((0, 0)), "a click on a step selects its action");
        menu_insert_stop(&ctx, &mut app, "After the first", true);
        assert_eq!(app.ui.action_sets[0].actions[0].steps[1].1, json!({"message": "After the first", "allowContinue": true}));

        menu_insert_stop(&ctx, &mut app, "Next", false);
        assert_eq!(app.ui.action_sets[0].actions[0].steps[2].1["message"], json!("Next"), "the new stop was the selected step");

        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Three Moves").center(), show);
        menu_insert_stop(&ctx, &mut app, "End again", false);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.last().unwrap().1["message"], json!("End again"), "a click on the action clears the step");
    }

    #[test]
    fn insert_stop_in_the_menu_goes_into_the_recording_and_needs_something_to_go_into() {
        use super::super::tests_appearance::{click, frame_events};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Insert Stop…").map(|(_, r)| *r).expect("Insert Stop… in the menu");
        click(&ctx, &mut app, item.center(), menu);
        assert!(app.ui.dialog.is_none(), "nothing selected and nothing recording: the item is off");

        app.run(START_RECORDING, json!({})).unwrap();
        menu_insert_stop(&ctx, &mut app, "While recording", false);
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        assert_eq!(recorded.steps, [(STOP_STEP.to_string(), json!({"message": "While recording", "allowContinue": false}))]);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3);
    }

    /// The panel menu's Insert Menu Item… after the selected step: Find narrows the list, a click on
    /// a row picks the item and runs nothing, Cancel inserts nothing, and OK inserts the pick after
    /// the step. With nothing selected and nothing recording, the item is off.
    #[test]
    fn insert_menu_item_in_the_menu_picks_from_the_list_and_inserts_on_ok() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let (mut app, _) = app_with_three_moves();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let open = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            let item = texts.iter().find(|(s, _)| s.trim() == "Insert Menu Item…").map(|(_, r)| *r).expect("Insert Menu Item… in the menu");
            click(ctx, app, item.center(), menu);
        };
        open(&ctx, &mut app);
        assert_eq!(open_kind(&app), None, "nothing selected and nothing recording: the item is off");

        click_first_step(&ctx, &mut app);
        open(&ctx, &mut app);
        let d = app.ui.dialog.as_ref().expect("Insert Menu Item… opens its dialog");
        assert_eq!(d.kind, crate::dialogs::action_insert_menu_item::KIND);
        assert_eq!((&d.fields["set"], &d.fields["action"], &d.fields["step"]), (&json!(0), &json!(0), &json!(0)));
        if let Some(d) = app.ui.dialog.as_mut() {
            d.fields.insert("find".into(), json!("object.ungroup"));
        }
        let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::dialogs::show(app, ui.ctx());
        let mut texts = vec![];
        for _ in 0..3 {
            texts = frame_events(&ctx, &mut app, vec![], draw);
        }
        assert!(!texts.iter().any(|(s, _)| s == "Object › Group"), "Find narrows the list: {texts:?}");
        let doc = app.session.active().unwrap().doc.clone();
        click(&ctx, &mut app, text_rect(&texts, "Object › Ungroup").center(), draw);
        let d = app.ui.dialog.as_ref().expect("a pick keeps the dialog open");
        assert_eq!((d.str("command"), &d.fields["params"]), ("object.ungroup".to_string(), &json!({})));
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "a pick runs nothing");

        crate::dialogs::cancel(&mut app);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "Cancel inserts nothing");

        open(&ctx, &mut app);
        if let Some(d) = app.ui.dialog.as_mut() {
            d.fields.insert("command".into(), json!("object.ungroup"));
        }
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(open_kind(&app), None, "OK closes the dialog");
        let ids: Vec<String> = app.ui.action_sets[0].actions[0].steps.iter().map(|(c, _)| c.clone()).collect();
        assert_eq!(ids, ["object.move", "object.ungroup", "object.move", "object.move"], "the pick goes after the selected step");
        assert_eq!(pstate::<Option<(u64, usize)>>(&ctx, SELECTED_STEP), Some((app.ui.action_sets[0].actions[0].id, 1)), "and is selected");
    }

    #[test]
    fn set_edits_keep_an_open_record_stop_on_its_action() {
        let mut app = app_with_sets();
        crate::dialogs::action_stop::open_record(&mut app, Some((1, 0)), None);
        app.run(DUPLICATE, json!({"set": 0})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["set"], json!(2), "a copied set above moves it down");
        app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["set"], json!(1), "a deleted set above moves it up");
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert_eq!(open_kind(&app), None, "deleting its set closes it");
    }

    /// [`app_with_three_moves`] with a second rectangle whose Attributes note is "Badge front" →
    /// (the app, the first rectangle's x offset, the second rectangle's x offset).
    #[allow(clippy::type_complexity)]
    fn app_with_a_noted_rect() -> (VectorcraftApp, Box<dyn Fn(&VectorcraftApp) -> f64>, Box<dyn Fn(&VectorcraftApp) -> f64>) {
        let (mut app, x) = app_with_three_moves();
        let first = app.session.active().unwrap().selection.objects.clone();
        let id = vectorcraft_doc::NodeId(
            app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap(),
        );
        app.run("attributes.set", json!({"ids": [id.0], "note": "Badge front"})).unwrap();
        app.run("select.set", json!({"ids": first.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
        let noted = move |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - 100.0;
        (app, Box::new(x), Box::new(noted))
    }

    /// A Select Object step goes after the given step, or at the action's end, runs nothing when it
    /// goes in, and plays as `select.byNote`: the steps after it act on the objects it selects.
    #[test]
    fn insert_select_object_goes_after_the_given_step_and_plays_as_a_selection_by_note() {
        let (mut app, x, noted) = app_with_a_noted_rect();
        let selection = app.session.active().unwrap().selection.clone();
        let r = app.run(INSERT_SELECT_OBJECT, json!({"set": 0, "action": 0, "step": 0, "note": "  badge "})).unwrap();
        assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
        assert_eq!(app.session.active().unwrap().selection, selection, "inserting runs nothing");
        let action = &app.ui.action_sets[0].actions[0];
        assert_eq!(action.steps[1], (SELECT_OBJECT_STEP.to_string(), json!({"note": "badge", "caseSensitive": false, "wholeWord": false})));
        app.run(INSERT_SELECT_OBJECT, json!({"set": 0, "action": 0, "note": "Badge", "caseSensitive": true, "wholeWord": true})).unwrap();
        assert_eq!(app.ui.action_sets[0].actions[0].steps[4].1, json!({"note": "Badge", "caseSensitive": true, "wholeWord": true}));
        play(&mut app, 0, 0).unwrap();
        assert_eq!((x(&app), noted(&app)), (1.0, 6.0), "the moves after the step move the object with the note");
        let ids = &app.session.active().unwrap().selection.objects;
        assert_eq!(ids.len(), 1);
    }

    /// A playback whose Select Object step finds no object stops there with the step's error, at
    /// every speed, and runs none of the steps after it.
    #[test]
    fn a_select_object_step_that_finds_nothing_stops_the_playback() {
        for speed in ["accelerated", "step"] {
            let (mut app, x, noted) = app_with_a_noted_rect();
            app.run(PLAYBACK_OPTIONS, json!({"speed": speed})).unwrap();
            app.ui.action_sets[0].actions[0].insert_step(0, (SELECT_OBJECT_STEP.into(), json!({"note": "missing"})));
            let selection = app.session.active().unwrap().selection.clone();
            let e = play(&mut app, 0, 0).unwrap_err();
            assert!(e.contains("no object that can be selected has a note matching “missing”"), "{speed}: {e}");
            play_out(&mut app);
            assert_eq!((x(&app), noted(&app)), (0.0, 0.0), "{speed}: nothing after it runs");
            assert_eq!(app.session.active().unwrap().selection, selection, "{speed}: the selection stays");
            assert!(app.ui.action_playing.is_none(), "{speed}");
        }
    }

    #[test]
    fn insert_select_object_refuses_bad_params_and_changes_nothing() {
        let (mut app, _, _) = app_with_a_noted_rect();
        let place = |extra: Value| {
            let mut p = json!({"set": 0, "action": 0});
            if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
                p.extend(extra.clone());
            }
            p
        };
        for (params, want) in [
            (place(json!({})), "the note is empty"),
            (place(json!({"note": " \t "})), "the note is empty"),
            (place(json!({"note": 5})), "`note` must be a string"),
            (place(json!({"note": "x".repeat(1025)})), "the note is longer than 1,024 bytes"),
            (place(json!({"note": "x", "caseSensitive": "yes"})), "`caseSensitive` must be true or false"),
            (place(json!({"note": "x", "wholeWord": 0})), "`wholeWord` must be true or false"),
            (place(json!({"note": "x", "action": 5})), "no such action"),
            (place(json!({"note": "x", "step": 9})), "no such action step"),
            (json!({"note": "x"}), "no such action"),
            (json!(["note"]), "`params` must be an object"),
        ] {
            assert_eq!(app.run(INSERT_SELECT_OBJECT, params.clone()).unwrap_err(), want, "{params}");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "{params}");
        }
        assert!(app.run(INSERT_SELECT_OBJECT, place(json!({"note": "x".repeat(1024)}))).is_ok(), "1,024 bytes goes in");
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

    /// The panel menu's Select Object… opens Set Selection on the selected step's place, with an
    /// empty note and both options off; Cancel inserts nothing, and OK inserts the step after the
    /// selected step, whose row shows the note. While recording, the step goes into the recording.
    /// With nothing selected and nothing recording, the item is off.
    #[test]
    fn select_object_in_the_menu_opens_set_selection_and_inserts_on_ok() {
        use super::super::tests_appearance::{click, frame_events};
        let (mut app, _, _) = app_with_a_noted_rect();
        app.ui.action_set = Some(0);
        let ctx = egui::Context::default();
        app.ui_ctx = Some(ctx.clone());
        let open = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            let item = texts.iter().find(|(s, _)| s.trim() == "Select Object…").map(|(_, r)| *r).expect("Select Object… in the menu");
            click(ctx, app, item.center(), menu);
        };
        open(&ctx, &mut app);
        assert_eq!(open_kind(&app), None, "nothing selected and nothing recording: the item is off");

        click_first_step(&ctx, &mut app);
        open(&ctx, &mut app);
        let d = app.ui.dialog.as_ref().expect("Select Object… opens Set Selection");
        assert_eq!(d.kind, crate::dialogs::action_select_object::KIND);
        assert_eq!((&d.fields["set"], &d.fields["action"], &d.fields["step"]), (&json!(0), &json!(0), &json!(0)));
        assert_eq!((d.str("note"), d.bool("caseSensitive"), d.bool("wholeWord")), (String::new(), false, false));
        crate::dialogs::cancel(&mut app);
        assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "Cancel inserts nothing");

        open(&ctx, &mut app);
        // A field of another type (set through ui.dialog.set) is an error, and the dialog stays.
        for (key, value) in [("note", json!(5)), ("caseSensitive", json!("yes")), ("wholeWord", json!(1))] {
            if let Some(d) = app.ui.dialog.as_mut() {
                d.fields.insert("note".into(), json!("Badge"));
                d.fields.insert("caseSensitive".into(), json!(false));
                d.fields.insert("wholeWord".into(), json!(false));
                d.fields.insert(key.into(), value.clone());
            }
            assert!(crate::dialogs::confirm(&mut app).is_err(), "`{key}`: {value} is an error");
            assert_eq!(open_kind(&app), Some(crate::dialogs::action_select_object::KIND), "`{key}`: the dialog stays open");
            assert_eq!(app.ui.action_sets[0].actions[0].steps.len(), 3, "`{key}`: nothing is inserted");
        }
        if let Some(d) = app.ui.dialog.as_mut() {
            d.fields.insert("note".into(), json!("Badge"));
            d.fields.insert("caseSensitive".into(), json!(true));
            d.fields.insert("wholeWord".into(), json!(false));
        }
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(open_kind(&app), None, "OK closes Set Selection");
        let action = &app.ui.action_sets[0].actions[0];
        assert_eq!(action.steps[1], (SELECT_OBJECT_STEP.to_string(), json!({"note": "Badge", "caseSensitive": true, "wholeWord": false})));
        let texts = frame_events(&ctx, &mut app, vec![], show);
        assert!(texts.iter().any(|(s, _)| s == "Select Object: Badge"), "the step's row shows the note: {texts:?}");

        app.run(START_RECORDING, json!({})).unwrap();
        open(&ctx, &mut app);
        assert!(!app.ui.dialog.as_ref().is_some_and(|d| d.fields.contains_key("set")), "while recording it goes into the recording");
        if let Some(d) = app.ui.dialog.as_mut() {
            d.fields.insert("note".into(), json!("While recording"));
        }
        crate::dialogs::confirm(&mut app).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        let recorded = app.ui.action_sets[0].actions.last().unwrap();
        assert_eq!(
            recorded.steps,
            [(SELECT_OBJECT_STEP.to_string(), json!({"note": "While recording", "caseSensitive": false, "wholeWord": false}))]
        );
    }
}
