//! Helpers over the framework's [`ActionList`]: addressing by set order, the app's own action
//! options, the built-in set, and the limits an action set file keeps.

use craft::automation::actions::{Action, ActionList, ActionStep, ActionsFile};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use super::files;
use super::keys::FunctionKey;
// PLAN: assumed menus-w15 gives recording.rs `pub(super) fn recording_start(&VectorcraftApp) -> Option<usize>`,
// `pub(super) fn recording_target(&VectorcraftApp) -> Option<(String, Option<usize>)>` (the set's name and the
// list index of the action recorded into) and `pub(super) fn put_recorded(&mut Action, Vec<ActionStep>, Option<usize>)`.
use super::recording::{put_recorded, recording_start, recording_target};
use crate::VectorcraftApp;

/// The option holding an action's function key.
const KEY: &str = "key";

/// The option holding an action's Automatic Align To choice.
const AUTOMATIC_ALIGN_TO: &str = "automaticAlignTo";

/// The name of the set at position `set` of [`ActionList::sets`].
pub fn set_name(list: &ActionList, set: usize) -> Option<&str> {
    list.sets().get(set).copied()
}

/// The list indexes of the actions of `set`, in list order.
pub fn members(list: &ActionList, set: &str) -> Vec<usize> {
    list.actions().iter().enumerate().filter(|(_, a)| a.set.as_deref() == Some(set)).map(|(i, _)| i).collect()
}

/// The actions of `set`, in list order.
pub fn stored_actions(list: &ActionList, set: &str) -> Vec<Action> {
    members(list, set).into_iter().filter_map(|i| list.action(i)).cloned().collect()
}

/// The list index of the `action`-th action of the `set`-th set.
pub fn flat(list: &ActionList, set: usize, action: usize) -> Option<usize> {
    members(list, set_name(list, set)?).get(action).copied()
}

/// The (set, action) position of the action at list index `index`: the inverse of [`flat`]. None
/// for an action that is in no set.
pub fn place(list: &ActionList, index: usize) -> Option<(usize, usize)> {
    let name = list.set_of(index)?;
    let set = list.sets().iter().position(|s| *s == name)?;
    let action = members(list, name).iter().position(|&i| i == index)?;
    Some((set, action))
}

/// `name`, else `name 2`, `name 3` … : the first that no set has.
pub fn free_set_name(list: &ActionList, name: &str) -> String {
    let sets = list.sets();
    let taken = |n: &str| sets.contains(&n);
    if !taken(name) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} {n}")).find(|n| !taken(n)).unwrap_or_else(|| name.to_string())
}

/// The function key a saved value holds; a value that is not a function key is none.
pub fn function_key_from(value: &Value) -> Option<FunctionKey> {
    serde_json::from_value::<FunctionKey>(value.clone()).ok().filter(|k| k.chord().is_some())
}

/// The function key that plays the action.
pub fn function_key(action: &Action) -> Option<FunctionKey> {
    action.options.get(KEY).and_then(function_key_from)
}

/// Gives the action a function key, or takes it away.
pub fn set_function_key(action: &mut Action, key: Option<FunctionKey>) {
    match key.and_then(|k| serde_json::to_value(k).ok()) {
        Some(value) => {
            action.options.insert(KEY.into(), value);
        }
        None => {
            action.options.remove(KEY);
        }
    }
}

/// Whether Align takes its reference from the live selection when the action plays.
pub fn automatic_align_to(action: &Action) -> bool {
    action.options.get(AUTOMATIC_ALIGN_TO).and_then(Value::as_bool).unwrap_or(false)
}

/// Turns Automatic Align To on or off; off leaves no option, as an action saved without it.
pub fn set_automatic_align_to(action: &mut Action, on: bool) {
    if on {
        action.options.insert(AUTOMATIC_ALIGN_TO.into(), Value::Bool(true));
    } else {
        action.options.remove(AUTOMATIC_ALIGN_TO);
    }
}

/// The steps of journal entries.
pub fn steps_from(entries: Vec<(String, Value)>) -> Vec<ActionStep> {
    entries.into_iter().map(|(command, params)| ActionStep::new(command, params)).collect()
}

/// The saved current set ([`crate::UiState::action_set`]): a name is the current set, and any other
/// value, an index saved by an earlier version included, is no current set. A name no set has is
/// ignored where the panel reads it.
pub fn deserialize_current_set<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_str().map(str::to_string))
}

/// Built-in actions (our own, handy defaults).
pub fn default_sets() -> ActionsFile {
    let set = "Default Actions";
    let action = |name: &str, steps: Vec<(&str, Value)>| Action {
        set: Some(set.to_string()),
        ..Action::new(name, steps.into_iter().map(|(command, params)| ActionStep::new(command, params)).collect())
    };
    ActionsFile::new(
        vec![
            action("Duplicate and Offset 10 pt", vec![("object.move", json!({"dx": 10, "dy": 10, "copy": true}))]),
            action("Unite and Simplify", vec![("object.pathfinder.unite", json!({})), ("object.path.simplify", json!({"tolerance": 0.5}))]),
            action("Center on Artboard", vec![("object.align", json!({"horizontal": "center", "vertical": "center", "to": "artboard"}))]),
            action("Outline Text", vec![("type.createOutlines", json!({}))]),
            action("Rotate 90° CW", vec![("object.rotate", json!({"angle": -90}))]),
            action("Reflect Horizontal", vec![("object.reflect", json!({"axis": "vertical", "basis": "box"}))]),
        ],
        vec![],
    )
}

/// The list a new profile starts with.
pub fn default_list() -> ActionList {
    ActionList::from_file(default_sets())
}

/// The most that Load Actions… reads from an action set file ([`files::read_set`]), each with the
/// error an edit past it gives: the steps of one action, the actions of a set and the bytes of the
/// file.
const SET_LIMITS: [(usize, &str); 3] = [
    (files::MAX_ACTION_STEPS, "the action already has 10,000 steps"),
    (files::MAX_SET_ACTIONS, "the action set file holds more than 1,000 actions"),
    (files::MAX_SET_FILE, "the action set would be larger than 8 MiB"),
];

/// The steps the journal holds from entry `start` on.
fn recorded_steps(app: &VectorcraftApp, start: usize) -> Vec<ActionStep> {
    steps_from(app.session.journal_for_action(start))
}

/// `actions`, the members of `set`, as Stop Playing/Recording leaves them when the recording in
/// progress records `steps` into `set`, with the index of the action they go into. None when the
/// recording goes to another set, or to none.
fn set_with_recorded(app: &VectorcraftApp, set: &str, actions: &[Action], steps: Vec<ActionStep>) -> Option<(Vec<Action>, usize)> {
    let (holder, _) = recording_target(app)?;
    if holder != set {
        return None;
    }
    let mut out = actions.to_vec();
    let at = match app.ui.recording_into.as_deref().and_then(|name| out.iter().position(|a| a.name == name)) {
        Some(at) => at,
        None => {
            out.push(Action::new(app.ui.recording.as_ref()?.1.clone(), vec![]));
            out.len() - 1
        }
    };
    put_recorded(out.get_mut(at)?, steps, app.ui.recording_at);
    Some((out, at))
}

/// The sizes of the set `set` with members `actions` that [`SET_LIMITS`] bound: the most steps one
/// action has, the actions and the length of the file. When the recording in progress records into
/// the set, its steps count too.
pub(super) fn set_sizes(app: &VectorcraftApp, set: &str, actions: &[Action]) -> [usize; 3] {
    let recorded = with_recording(app, set, actions);
    let actions = recorded.as_deref().unwrap_or(actions);
    [actions.iter().map(|a| a.steps.len()).max().unwrap_or(0), actions.len(), files::write_set(set, actions).len()]
}

/// `actions`, the members of `set`, with the steps recorded so far when the recording in progress
/// records into the set; None when it records into another set, or nothing records.
fn with_recording(app: &VectorcraftApp, set: &str, actions: &[Action]) -> Option<Vec<Action>> {
    let start = recording_start(app)?;
    set_with_recorded(app, set, actions, recorded_steps(app, start)).map(|(with, _)| with)
}

/// Err when naming the set `set` (None: a new, empty set) `name` would give it an action set file
/// larger than Load Actions… reads, counting the steps the recording in progress adds to it. A
/// name that leaves the file as large as it is, or makes it smaller, is not refused.
pub(crate) fn name_keeps_set_loadable(app: &VectorcraftApp, set: Option<&str>, name: &str) -> Result<(), String> {
    let (actions, was) = match set {
        Some(set) => {
            let stored = stored_actions(&app.ui.actions, set);
            let actions = with_recording(app, set, &stored).unwrap_or(stored);
            let was = files::write_set(set, &actions).len();
            (actions, was)
        }
        None => (Vec::new(), 0),
    };
    let size = files::write_set(name, &actions).len();
    if size > files::MAX_SET_FILE && size > was {
        return Err("the action set would be larger than 8 MiB".into());
    }
    Ok(())
}

/// Err when `after`, in place of `before` as the members of `set`, would give an action set file
/// that Load Actions… refuses ([`SET_LIMITS`]), counting the steps that the recording in progress
/// adds to it. An edit that leaves the set no further past a limit than it is now is not refused.
pub(crate) fn edit_keeps_set_loadable(app: &VectorcraftApp, set: &str, before: &[Action], after: &[Action]) -> Result<(), String> {
    let after = set_sizes(app, set, after);
    if after.iter().zip(SET_LIMITS).all(|(&size, (limit, _))| size <= limit) {
        return Ok(());
    }
    let before = set_sizes(app, set, before);
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
    recording_target(app).and_then(|(_, at)| app.ui.actions.action(at?)).map_or(0, |a| a.steps.len())
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

/// The length of the action set file that Stop Playing/Recording would leave without the steps
/// recorded so far ([`crate::UiState::recording_base`]).
fn recording_base(app: &mut VectorcraftApp) -> usize {
    if let Some(base) = app.ui.recording_base {
        return base;
    }
    let base = recording_target(app)
        .and_then(|(holder, _)| {
            let stored = stored_actions(&app.ui.actions, &holder);
            let (with, _) = set_with_recorded(app, &holder, &stored, vec![])?;
            Some(files::write_set(&holder, &with).len())
        })
        .unwrap_or(0);
    app.ui.recording_base = Some(base);
    base
}

/// Err when adding the step `(command, params)` to the recording in progress would give the action
/// that Stop Playing/Recording keeps more than [`files::MAX_ACTION_STEPS`] steps, or its set a file
/// larger than [`files::MAX_SET_FILE`], the most Load Actions… reads. The file without the steps
/// recorded so far is measured once ([`recording_base`]), and the recorded steps add
/// [`recorded_bytes`] to it. Only when that sum is past the limit is the whole file written out to
/// measure it.
pub(super) fn recording_room(app: &mut VectorcraftApp, command: &str, params: &Value) -> Result<(), String> {
    let Some(start) = recording_start(app) else { return Err("no such action".into()) };
    let Some((holder, _)) = recording_target(app) else { return Ok(()) };
    let stored = recording_target_steps(app);
    if stored + app.session.journal.since(start).len() >= files::MAX_ACTION_STEPS {
        return Err("the action already has 10,000 steps".into());
    }
    let first = if stored == 0 { files::FIRST_STEP_BYTES } else { 0 };
    let added = recorded_bytes(app, start) + files::step_bytes(command, params) + first;
    if recording_base(app) + added <= files::MAX_SET_FILE {
        return Ok(());
    }
    let mut steps = recorded_steps(app, start);
    steps.push(ActionStep::new(command, params.clone()));
    let stored = stored_actions(&app.ui.actions, &holder);
    if set_with_recorded(app, &holder, &stored, steps).is_some_and(|(with, _)| files::write_set(&holder, &with).len() > files::MAX_SET_FILE) {
        return Err("the action set would be larger than 8 MiB".into());
    }
    Ok(())
}

/// Err when the steps recorded so far no longer fit in the action that Stop Playing/Recording gives
/// them ([`steps_that_fit`]): a command added values to its step after [`recording_room`] measured
/// it. The steps are written out to measure them only when [`recorded_bytes`] puts them past the
/// limit.
pub(super) fn recorded_steps_fit(app: &mut VectorcraftApp) -> Result<(), String> {
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

/// The saved shapes of the action list and of Button Mode: the shapes earlier versions wrote, so old
/// and new builds read each other's preferences.
pub mod prefs {
    use craft::automation::actions::{Action, ActionColor, ActionList, ActionStep};
    use craft::automation::ui::actions::ActionsPanelState;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::{Map, Value, json};

    use super::{automatic_align_to, free_set_name, function_key, function_key_from, members, set_automatic_align_to, set_function_key};

    fn action_value(action: &Action) -> Value {
        let mut out = Map::new();
        out.insert("name".into(), json!(action.name));
        if automatic_align_to(action) {
            out.insert("automaticAlignTo".into(), json!(true));
        }
        let steps: Vec<Value> = action.steps.iter().map(|s| json!([s.command, s.params])).collect();
        out.insert("steps".into(), Value::Array(steps));
        if !action.disabled.is_empty() {
            out.insert("excluded_steps".into(), json!(action.disabled));
        }
        if let Some(key) = function_key(action).and_then(|k| serde_json::to_value(k).ok()) {
            out.insert("key".into(), key);
        }
        if action.color != ActionColor::None {
            out.insert("color".into(), json!(action.color.name()));
        }
        Value::Object(out)
    }

    /// Writes the list as `[{name, actions: [..]}]` in [`ActionList::sets`] order, empty sets included.
    pub fn serialize<S: Serializer>(list: &ActionList, s: S) -> Result<S::Ok, S::Error> {
        let sets: Vec<Value> = list
            .sets()
            .into_iter()
            .map(|set| {
                let actions: Vec<Value> = members(list, set).into_iter().filter_map(|i| list.action(i)).map(action_value).collect();
                json!({"name": set, "actions": actions})
            })
            .collect();
        Value::Array(sets).serialize(s)
    }

    fn step_of(value: &Value) -> Option<ActionStep> {
        let pair = value.as_array()?;
        Some(ActionStep::new(pair.first()?.as_str()?, pair.get(1).cloned().unwrap_or(Value::Null)))
    }

    /// The action a saved value holds; damaged options read as none, none and false.
    fn action_of(value: &Value, set: &str) -> Option<Action> {
        let steps = value.get("steps").and_then(Value::as_array).map(|a| a.iter().filter_map(step_of).collect()).unwrap_or_default();
        let mut action = Action::new(value.get("name")?.as_str()?, steps);
        action.set = Some(set.to_string());
        action.disabled =
            value.get("excluded_steps").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).map(|n| n as usize).collect();
        action.color = value.get("color").and_then(Value::as_str).map_or(ActionColor::None, ActionColor::from_name);
        set_function_key(&mut action, value.get("key").and_then(function_key_from));
        set_automatic_align_to(&mut action, value.get("automaticAlignTo").and_then(Value::as_bool).unwrap_or(false));
        Some(action)
    }

    /// Reads the list in order: a repeated set name takes a free one, a repeated action name too.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ActionList, D::Error> {
        let saved = Value::deserialize(d)?;
        let mut list = ActionList::new();
        for entry in saved.as_array().into_iter().flatten() {
            let Some(name) = entry.get("name").and_then(Value::as_str) else { continue };
            let set = free_set_name(&list, name.trim());
            let saved_actions = entry.get("actions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            // A refused (empty) set name leaves the set out; there is nothing else to do with it.
            if saved_actions.is_empty() {
                let _ = list.new_set(&set);
            }
            for value in saved_actions {
                let Some(mut action) = action_of(value, &set) else { continue };
                action.name = list.unique_name(&action.name);
                // A refused action (empty name) is left out for the same reason.
                let _ = list.push(action);
            }
        }
        Ok(list)
    }

    /// Button Mode alone is saved, as the bool it always was.
    pub mod button_mode {
        use super::{ActionsPanelState, Deserialize, Deserializer, Serializer, Value};

        /// Writes `settings.button_mode`.
        pub fn serialize<S: Serializer>(panel: &ActionsPanelState, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bool(panel.settings.button_mode)
        }

        /// Reads a bool; a missing or other value is off.
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ActionsPanelState, D::Error> {
            let on = Value::deserialize(d)?.as_bool().unwrap_or(false);
            let mut panel = ActionsPanelState::default();
            panel.settings.button_mode = on;
            Ok(panel)
        }
    }
}

/// How many of `steps`, from the first, Stop Playing/Recording can give the recording's action so
/// that its set stays one that Load Actions… reads ([`recording_room`]), and the error that names
/// the limit when that is fewer than all of them.
pub(super) fn steps_that_fit(app: &VectorcraftApp, steps: &[(String, Value)]) -> (usize, Option<&'static str>) {
    let Some((holder, _)) = recording_target(app) else { return (steps.len(), None) };
    let stored_set = stored_actions(&app.ui.actions, &holder);
    let Some((with, _)) = set_with_recorded(app, &holder, &stored_set, vec![]) else { return (steps.len(), None) };
    let stored = recording_target_steps(app);
    let keep = steps.len().min(files::MAX_ACTION_STEPS.saturating_sub(stored));
    // Each step adds `step_bytes` to the file, and the action's first step a little more.
    let mut size = files::write_set(&holder, &with).len() + if stored == 0 { files::FIRST_STEP_BYTES } else { 0 };
    let fitting = steps
        .iter()
        .take(keep)
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
