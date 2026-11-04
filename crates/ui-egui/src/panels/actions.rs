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
    /// The action's Button Mode color, one of [`COLORS`], if one is assigned. The panel does not
    /// show it. A saved value that is not one of them loads as none.
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

    fn set_steps_included(&mut self, included: bool) {
        self.excluded_steps.clear();
        if !included {
            self.excluded_steps.extend(0..self.steps.len());
        }
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
/// returns. Step by Step and Pause For run the first step now, each as its own undo step, and leave
/// the rest to [`tick_playback`] → {results: [the first step's result], pending: the steps left}.
/// While a paced playback is in progress, another action does not play.
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
    if options.speed == PlaybackSpeed::Accelerated {
        return app.run("command.batch", json!({"label": a.name, "commands": commands}));
    }
    let mut steps: std::collections::VecDeque<Value> = commands.into();
    let Some(first) = steps.pop_front() else { return Err("the action has no included steps".into()) };
    let result = play_step(app, &a.name, first)?;
    let pending = steps.len();
    if pending > 0 {
        let hold = options.speed == PlaybackSpeed::Step;
        let doc = active_doc(app);
        app.ui.action_playing = Some(Playing { label: a.name, steps, options, next_at: None, hold, doc });
        if let Some(ctx) = &app.ui_ctx {
            ctx.request_repaint();
        }
    }
    Ok(json!({"results": [result], "pending": pending}))
}

/// Runs one step of a paced playback as its own undo step, named `label` → the step's result.
fn play_step(app: &mut VectorcraftApp, label: &str, step: Value) -> Result<Value, String> {
    let r = app.run("command.batch", json!({"label": label, "commands": [step]}))?;
    Ok(r.get("results").and_then(|r| r.get(0)).cloned().unwrap_or(Value::Null))
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
/// do not run. → the seconds until the window should draw again while playback goes on (none while
/// Step by Step waits for a UI pass, which asks for the next frame itself)
pub fn tick_playback(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    let doc = active_doc(app);
    let playing = app.ui.action_playing.as_mut()?;
    if playing.hold {
        return None;
    }
    let pause = f64::from(playing.options.seconds);
    if playing.options.speed == PlaybackSpeed::Pause {
        let due = *playing.next_at.get_or_insert(now + pause);
        if now < due {
            return Some(due - now);
        }
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
/// applies `included` to the other steps of its action. Invalid indexes change nothing.
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
    let set_ref = app.ui.action_sets.get_mut(set).ok_or("no such action set")?;
    match (action, step) {
        (None, None) => {
            for action in &mut set_ref.actions {
                action.set_steps_included(included);
            }
        }
        (Some(ai), None) => set_ref.actions.get_mut(ai).ok_or("no such action")?.set_steps_included(included),
        (Some(ai), Some(si)) => {
            let action = set_ref.actions.get_mut(ai).ok_or("no such action")?;
            if si >= action.steps.len() {
                return Err("no such action step".into());
            }
            if others {
                let was_excluded = action.excluded_steps.contains(&si);
                action.set_steps_included(included);
                if was_excluded {
                    action.excluded_steps.insert(si);
                } else {
                    action.excluded_steps.remove(&si);
                }
            } else if included {
                action.excluded_steps.remove(&si);
            } else {
                action.excluded_steps.insert(si);
            }
        }
        (None, Some(_)) => return Err("no such action step".into()),
    }
    Ok(json!({"set": set, "action": action, "step": step, "included": included}))
}

/// The command that renames an action ([`rename_action`]).
pub const RENAME_ACTION: &str = "ui.action.rename";

/// `ui.action.rename {set, action, name}`: renames the action at index `action` of set `set` (0 is
/// the top set) to `name` with spaces trimmed from both ends, and keeps its steps, function key and
/// color. An index past the last set or action, or a name with nothing but spaces, is an error, and
/// then nothing is renamed. → {name}
pub fn rename_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let index = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let action = index("set").zip(index("action")).and_then(|(s, a)| app.ui.action_sets.get_mut(s)?.actions.get_mut(a)).ok_or("no such action")?;
    if name.is_empty() {
        return Err("name is empty".into());
    }
    action.name = name.to_string();
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
/// not one of them, nothing is added. The New Action dialog's Record runs this. → {set, action}
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
    let Some(s) = app.ui.action_sets.get_mut(set) else { return Err("no such action set".into()) };
    s.actions.push(action);
    let index = s.actions.len() - 1;
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
/// or a key or color that is not one of them is an error, and then nothing changes. The panel
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
    let Some(action) = app.ui.action_sets.get_mut(set).and_then(|s| s.actions.get_mut(at)) else { return Err("no such action".into()) };
    if let Some(name) = name {
        action.name = name;
    }
    if let Some(key) = key {
        action.key = key;
    }
    if let Some(color) = color {
        action.color = color;
    }
    Ok(json!({ "name": action.name, "key": action.key, "color": action.color }))
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

/// The command that ends a recording ([`stop`]).
pub const STOP_RECORDING: &str = "ui.action.stop";

/// The command that begins a plain recording ([`record`]).
pub const START_RECORDING: &str = "ui.action.record";

/// `ui.action.record {set?}`: begins recording an automatically named Action N in `set`, or in
/// the current set (the first if none is current). With no sets, makes Set 1. Stop adds the action
/// after the set's existing actions. An explicit invalid set or an existing recording is an error
/// and leaves the panel unchanged. → {set, name}
pub fn record(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
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
    app.ui.action_set = Some(set);
    app.ui.action_sets_selected = vec![set];
    let mut number = app.ui.action_sets[set].actions.len() + 1;
    let name = loop {
        let name = format!("Action {number}");
        if app.ui.action_sets[set].actions.iter().all(|action| action.name != name) {
            break name;
        }
        number += 1;
    };
    start_recording(app, set, name.clone(), None);
    Ok(json!({ "set": set, "name": name }))
}

/// `ui.action.stop {}`: ends the recording in progress and keeps its steps, in the action New
/// Action made for it, else in a new action after the last action of its set, as the bottom bar's
/// Stop Playing/Recording does. It also ends a playback at Step by Step or Pause For: the steps
/// already played stay, and the rest do not run. With nothing recording or playing it changes
/// nothing. → {stopped}
pub fn stop(app: &mut VectorcraftApp, _: &Value) -> Result<Value, String> {
    let playing = app.ui.action_playing.take().is_some();
    let recording = app.ui.recording.is_some();
    stop_recording(app);
    Ok(json!({ "stopped": recording || playing }))
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
}

/// Stop Playing/Recording: ends the recording and keeps its steps (see [`start_recording`]). An
/// action New Action made that was deleted while it recorded keeps nothing.
fn stop_recording(app: &mut VectorcraftApp) {
    let Some((set, name, start)) = app.ui.recording.take() else { return };
    // A canceled undo group numbers the steps down when it drops entries made before them.
    let start = app.session.journal.held().unwrap_or(start);
    let steps = app.session.journal_for_action(start);
    app.session.journal.hold(None);
    match app.ui.recording_into.take() {
        Some(id) => {
            if let Some(a) = app.ui.action_sets.iter_mut().flat_map(|s| s.actions.iter_mut()).find(|a| a.id == id) {
                a.steps.extend(steps);
            }
        }
        None => {
            if let Some(s) = app.ui.action_sets.get_mut(set) {
                s.actions.push(Action::new(name, steps));
            }
        }
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

/// Delete Selection, from the bottom bar and the panel menu: deletes the selected action, else asks
/// whether to delete the selected sets ([`DELETE_SETS`]).
fn delete_selection(app: &mut VectorcraftApp, ctx: &egui::Context) {
    const SELECTED: &str = "actions-selected";
    // A selected action counts only while its set is the current one, as the panel shows it.
    if let Some((si, ai)) = pstate::<Option<(usize, usize)>>(ctx, SELECTED).filter(|&(s, _)| app.ui.action_set == Some(s)) {
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
        || (d.kind == crate::dialogs::action_set_options::KIND && d.fields.contains_key("set"))
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
/// Batch dialog keep to their own set and action. → {set, action?}
pub fn duplicate(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    const SELECTED: &str = "actions-selected";
    let len = app.ui.action_sets.len();
    let index = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok());
    let set = match p.get("set") {
        None | Some(Value::Null) => app.ui.action_set.filter(|&s| s < len).ok_or("no action set is current")?,
        Some(v) => index(v).filter(|&s| s < len).ok_or("no such action set")?,
    };
    let Some(src) = app.ui.action_sets.get_mut(set) else { return Err("no such action set".into()) };
    let action = match p.get("action") {
        None | Some(Value::Null) => None,
        Some(v) => Some(index(v).filter(|&a| a < src.actions.len()).ok_or("no such action")?),
    };
    let (copy, result) = if let Some(a) = action {
        let Some(mut copy) = src.actions.get(a).map(Action::copy) else { return Err("no such action".into()) };
        copy.name = format!("{} copy", copy.name);
        src.actions.insert(a + 1, copy);
        // A Batch dialog open on a later action of the set follows it down.
        if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == crate::dialogs::action_batch::KIND)
            && d.fields.get("set").and_then(index) == Some(set)
            && let Some(da) = d.fields.get("action").and_then(index).filter(|&da| da > a)
        {
            d.fields.insert("action".into(), json!(da + 1));
        }
        (Some((set, a + 1)), json!({ "set": set, "action": a + 1 }))
    } else {
        let copy = ActionSet::new(format!("{} copy", src.name), src.actions.iter().map(Action::copy).collect());
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
    // A selected action counts only while its set is the current one, as the panel shows it: a set
    // deleted since leaves the stored pair pointing at another set's action.
    let selected = pstate::<Option<(usize, usize)>>(ui.ctx(), "actions-selected").filter(|&(s, _)| app.ui.action_set == Some(s));
    let has = selected.and_then(|(s, a)| app.ui.action_sets.get(s).and_then(|set| set.actions.get(a))).is_some();
    if widgets::menu_item(ui, tl!("New Action…"), crate::dialogs::action_new::can_open(app), false)
        && let Err(e) = crate::dialogs::action_new::open(app)
    {
        app.status(e);
    }
    if widgets::menu_item(ui, tl!("New Set…"), true, false) {
        crate::dialogs::action_set_options::open_new(app);
    }
    let current = app.ui.action_set.filter(|&s| s < app.ui.action_sets.len());
    if widgets::menu_item(ui, tl!("Duplicate"), current.is_some(), false) {
        duplicate_selection(app, ui.ctx());
    }
    if widgets::menu_item(ui, tl!("Delete"), has || !selected_sets(app).is_empty(), false) {
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
    if widgets::menu_item(ui, tl!("Play Current Selection"), has, false)
        && let Some((s, a)) = selected
        && let Err(e) = play(app, s, a)
    {
        app.status(e);
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

enum InclusionChange {
    Set(usize),
    Action(usize, usize),
    Step(usize, usize, usize, bool),
}

pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
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
    let mut current = app.ui.action_set;
    let mut sets_selected = selected_sets(app);
    let shift = ui.input(|i| i.modifiers.shift);
    let alt = ui.input(|i| i.modifiers.alt);
    let mut expanded: BTreeSet<u64> = pstate(ui.ctx(), "actions-expanded");
    let mut inclusion_changes = Vec::new();
    let mut play_req = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
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
                    current = Some(si);
                    sets_selected = vec![si];
                }
                if resp.double_clicked() && !resp.interact_pointer_pos().is_some_and(|p| check_rect.contains(p) || disclosure.contains(p)) {
                    play_req = Some((si, ai));
                }
                if open {
                    for (step, (command, params)) in a.steps.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, 0.0, t.hover);
                        }
                        let (clicked, _) = inclusion_box(ui, r, 55.0, ("step-inclusion", a.id, step), !a.excluded_steps.contains(&step), false);
                        if clicked {
                            inclusion_changes.push(InclusionChange::Step(si, ai, step, alt));
                        }
                        ui.painter().text(
                            r.left_center() + egui::vec2(70.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            command,
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
    });
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
    app.ui.action_set = current;
    app.ui.action_sets_selected = sets_selected;
    widgets::divider(ui);
    // Bottom bar: stop, record, play, new set, new action, delete.
    ui.horizontal(|ui| {
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
            && let Err(e) = app.run(START_RECORDING, json!({}))
        {
            app.status(e);
        }
        if widgets::icon_button(ui, "dc-actions", tl!("Play Current Selection"), false, 24.0).clicked()
            && let Some(s) = selected
        {
            play_req = Some(s);
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
    if app.ui.action_sets.is_empty() {
        dim_label(ui, tl!("No actions. Press ● to record."));
    }
}

#[cfg(test)]
mod tests {
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
                json!("{\"format\": \"vcactions\", \"version\": 3, \"name\": \"S\", \"actions\": []}"),
                "the action set file needs a later version of VectorCraft",
            ),
        ] {
            assert_eq!(app.run(LOAD_SET, json!({ "data": data })).unwrap_err(), want);
        }
        with_files(&mut app, "/sets/missing.vcactions");
        assert_eq!(app.run(LOAD_SET, json!({})).unwrap_err(), "no such file");
        assert_eq!(set_names(&app), before, "no set added");
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
}
