//! The Actions panel's edit commands. Each one checks its params and the app's limits, then makes
//! one [`ActionList`](craft::automation::actions::ActionList) call; the list keeps names unique and
//! the order of its sets.

use std::collections::BTreeSet;

use craft::automation::actions::{Action, ActionColor, ActionListError, ActionStep};
use craft::automation::ui::actions::{ActionSelection, ActionsWords, LibraryPrompt};
use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
use craft::ui::runtime::env::Env;
use craft::ui::runtime::words::CommonWords;
use serde_json::{Value, json};

use super::model;
use super::{FunctionKey, files, key_error};
use crate::VectorcraftApp;
use crate::dialogs::action_batch::BatchDialog;
use crate::dialogs::action_insert_menu_item::InsertMenuItemDialog;
use crate::dialogs::action_new::NewActionDialog;
use crate::dialogs::action_select_object::SelectObjectDialog;
use crate::dialogs::action_set_options::SetOptionsDialog;
use crate::dialogs::action_stop::RecordStopDialog;
use crate::dialogs::actions::{Target, open_or_say};

/// The key of the questions the panel asks before it deletes, clears or resets sets.
const QUESTION: &str = "actionsQuestion";

/// The questions of Delete, Clear Actions and Reset Actions on the dialog host, told apart by their
/// tag: the names of the sets to delete, or the [`LibraryPrompt`]. The wrapper keeps their answers
/// apart from the app's other prompts.
pub(crate) struct ActionsQuestion(pub(crate) StandardPrompt);

crate::dialogs::confirm::wrapped_dialog!(ActionsQuestion, StandardPrompt, QUESTION);

/// The place a step dialog holds: set index, action index in the set, and the step the new one goes
/// after (none: the action's end).
type Place = (usize, usize, Option<usize>);

/// The command that includes or excludes steps ([`include_steps`]).
pub const INCLUDE_STEPS: &str = "ui.action.stepsIncluded";
/// The command that renames an action ([`rename_action`]).
pub const RENAME_ACTION: &str = "ui.action.rename";
/// The command that makes an action and records into it ([`new_action`]).
pub const NEW_ACTION: &str = "ui.action.new";
/// The command that changes an action's name, function key and color ([`action_options`]).
pub const ACTION_OPTIONS: &str = "ui.action.options";
/// The command behind the panel menu's Button Mode ([`button_mode`]).
pub const BUTTON_MODE: &str = "ui.action.buttonMode";
/// The command that inserts a stop ([`insert_stop`]).
pub const INSERT_STOP: &str = "ui.action.insertStop";
/// The command that inserts a menu item's command as a step ([`insert_menu_item`]).
pub const INSERT_MENU_ITEM: &str = "ui.action.insertMenuItem";
/// The command that inserts a Select Path step ([`insert_select_path`]).
pub const INSERT_SELECT_PATH: &str = "ui.action.insertSelectPath";
/// The command id of a Select Path step, with {id, fingerprint} as its params.
pub const SELECT_PATH_STEP: &str = "select.storedPath";
/// The command a Select Object step runs, with {note, caseSensitive, wholeWord} as its params.
pub const SELECT_OBJECT_STEP: &str = "select.byNote";
/// The command that inserts a Select Object step ([`insert_select_object`]).
pub const INSERT_SELECT_OBJECT: &str = "ui.action.insertSelectObject";
/// The command that deletes one step of an action ([`delete_step`]).
pub const DELETE_STEP: &str = "ui.action.deleteStep";
/// The command that moves one step of an action ([`move_step`]).
pub const MOVE_STEP: &str = "ui.action.moveStep";
/// The command that deletes action sets ([`delete_sets`]).
pub const DELETE_SETS: &str = "ui.actionSet.delete";
/// The command that duplicates an action or an action set ([`duplicate`]).
pub const DUPLICATE: &str = "ui.actionSet.duplicate";
/// The command that puts the built-in action sets back ([`reset`]).
pub const RESET: &str = "ui.actionSet.reset";
/// The command that deletes every action set in the panel ([`clear`]).
pub const CLEAR: &str = "ui.actionSet.clear";

/// The longest note a Select Object step searches for, in bytes, as `select.byNote` takes it.
const MAX_SELECT_NOTE: usize = 1024;

fn index(p: &Value, key: &str) -> Option<usize> {
    p.get(key).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok())
}

/// The refusal text of a list edit; a taken name has the command surface's own words.
fn list_error(e: ActionListError) -> String {
    match e {
        ActionListError::NameTaken(_) => "an action with this name already exists".into(),
        e => e.to_string(),
    }
}

/// The name of the set at index `set` of [`ActionList::sets`](craft::automation::actions::ActionList::sets).
fn set_named(app: &VectorcraftApp, set: usize) -> Result<String, String> {
    model::set_name(&app.ui.actions, set).map(str::to_string).ok_or_else(|| "no such action set".into())
}

/// The `set`, `action` params as (set, index in the set, list index).
fn action_of(app: &VectorcraftApp, p: &Value) -> Result<(usize, usize, usize), String> {
    let (set, at) = index(p, "set").zip(index(p, "action")).ok_or("no such action")?;
    let flat = model::flat(&app.ui.actions, set, at).ok_or("no such action")?;
    Ok((set, at, flat))
}

/// Err when `edit`, made to a copy of the actions of `set`, would give the set a file that Load
/// Actions… refuses ([`model::edit_keeps_set_loadable`]); the limits run before the list changes.
fn keeps_loadable(app: &VectorcraftApp, set: &str, edit: impl FnOnce(&mut Vec<Action>)) -> Result<(), String> {
    let before = model::stored_actions(&app.ui.actions, set);
    let mut after = before.clone();
    edit(&mut after);
    model::edit_keeps_set_loadable(app, set, &before, &after)
}

/// Makes the action at list index `flat` the selection (with its step), and its set the current and
/// only selected one, as the panel shows it.
fn select(app: &mut VectorcraftApp, flat: usize, step: Option<usize>) {
    let set = app.ui.actions.set_of(flat).map(str::to_string);
    let panel = &mut app.ui.actions_panel;
    panel.selected_sets = set.iter().cloned().collect();
    if step.is_some()
        && let Some(action) = app.ui.actions.action(flat)
    {
        panel.expanded.insert(action.name.clone());
    }
    panel.selected = Some(match step {
        Some(step) => ActionSelection::Step(flat, step),
        None => ActionSelection::Action(flat),
    });
    app.ui.action_set = set;
}

/// Renames the action at `flat`; a recording into it and the panel's expansion follow the name.
fn rename(app: &mut VectorcraftApp, flat: usize, name: &str) -> Result<(), String> {
    let old = app.ui.actions.action(flat).map(|a| a.name.clone()).ok_or("no such action")?;
    app.ui.actions.rename(flat, name, |_, _, _| {}).map_err(list_error)?;
    let new = app.ui.actions.action(flat).map_or_else(|| name.to_string(), |a| a.name.clone());
    if app.ui.recording_into.as_deref() == Some(old.as_str()) {
        app.ui.recording_into = Some(new.clone());
    }
    if app.ui.actions_panel.expanded.remove(&old) {
        app.ui.actions_panel.expanded.insert(new);
    }
    Ok(())
}

/// The color that params `p` give: `color`, the file name of an [`ActionColor`], or null or `""`
/// for none.
fn color_param(p: &Value) -> Result<ActionColor, String> {
    match p.get("color") {
        None | Some(Value::Null) => Ok(ActionColor::None),
        Some(v) => v.as_str().and_then(|c| ActionColor::ALL.into_iter().find(|color| color.name() == c)).ok_or_else(|| "no such color".into()),
    }
}

/// The optional per-action Align playback choice; null, like omission, leaves the default or kept value.
fn automatic_align_to_param(p: &Value) -> Result<Option<bool>, String> {
    match p.get("automaticAlignTo") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| "automaticAlignTo must be true or false".into()),
    }
}

/// Moves the Record Stop, Insert Menu Item and Set Selection dialogs that hold a step's place: `f`
/// gives the new place, or `None` to close the dialog. A dialog with no place inserts into the
/// recording, which no edit moves.
fn retarget_step_dialogs(app: &VectorcraftApp, f: impl Fn(Place) -> Option<Place>) {
    let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) else { return };
    // Whether the dialog stays open, with its place moved.
    let moved = |target: &mut Target| {
        let (Some(set), Some(action)) = (target.set, target.action) else { return true };
        let Some((set, action, step)) = f((set, action, target.step)) else { return false };
        *target = Target { set: Some(set), action: Some(action), step };
        true
    };
    macro_rules! follow {
        ($host:expr, $($d:ty),+) => {$(
            if $host.get_mut::<$d>().is_some_and(|d| !moved(&mut d.target)) {
                $host.cancel::<$d>(&ctx);
            }
        )+};
    }
    dialogs.with(|host| {
        follow!(host, RecordStopDialog, InsertMenuItemDialog, SelectObjectDialog);
    });
}

/// Moves every dialog that holds a set index: `f` gives the new index, or `None` to close it.
fn retarget_set_dialogs(app: &VectorcraftApp, f: impl Fn(usize) -> Option<usize>) {
    retarget_step_dialogs(app, |(set, action, step)| f(set).map(|set| (set, action, step)));
    let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) else { return };
    dialogs.with(|host| {
        if let Some(d) = host.get_mut::<BatchDialog>() {
            match f(d.set) {
                Some(set) => d.set = set,
                None => {
                    host.cancel::<BatchDialog>(&ctx);
                }
            }
        }
        if let Some(d) = host.get_mut::<NewActionDialog>() {
            match f(*d.set_mut()) {
                Some(set) => *d.set_mut() = set,
                None => {
                    host.cancel::<NewActionDialog>(&ctx);
                }
            }
        }
        if let Some(d) = host.get_mut::<SetOptionsDialog>()
            && let Some(held) = d.set
        {
            match f(held) {
                Some(set) => d.set = Some(set),
                None => {
                    host.cancel::<SetOptionsDialog>(&ctx);
                }
            }
        }
    });
}

/// Whether a dialog that holds a set index is open ([`retarget_set_dialogs`]).
fn set_dialog_open(app: &VectorcraftApp) -> bool {
    let placed = |target: Option<&Target>| target.is_some_and(|t| t.set.is_some());
    app.dialogs().is_some_and(|dialogs| {
        dialogs.with(|host| {
            placed(host.get::<RecordStopDialog>().map(|d| &d.target))
                || placed(host.get::<InsertMenuItemDialog>().map(|d| &d.target))
                || placed(host.get::<SelectObjectDialog>().map(|d| &d.target))
                || host.is_open::<BatchDialog>()
                || host.is_open::<NewActionDialog>()
                || host.get::<SetOptionsDialog>().is_some_and(|d| d.set.is_some())
        })
    })
}

/// Closes the dialogs of the deleted set `set` and moves those of later sets down.
fn close_set_dialogs(app: &VectorcraftApp, set: usize) {
    retarget_set_dialogs(app, |held| match held.cmp(&set) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(held - 1),
        std::cmp::Ordering::Less => Some(held),
    });
}

/// Moves the step dialogs of the steps from `step` on in the action at (`set`, `action`) by `delta`,
/// and closes the one on `step` itself when `delta` is negative (the step was deleted).
pub(super) fn follow_step_dialogs(app: &VectorcraftApp, set: usize, action: usize, step: usize, delta: isize) {
    retarget_step_dialogs(app, |(s, a, held)| match held {
        Some(held) if (s, a) == (set, action) && held >= step => {
            (held != step || delta >= 0).then(|| (s, a, Some(held.saturating_add_signed(delta))))
        }
        _ => Some((s, a, held)),
    });
}

/// `{set, action?, step?, included, others?}` includes or excludes all steps of a set, all steps
/// of one action, or one step. With `step` and `others: true`, leaves that step as it was and
/// applies `included` to the other steps of its action. Invalid indexes change nothing. So does an
/// exclusion that would give the set a file larger than [`files::MAX_SET_FILE`], the most Load
/// Actions… reads, counting the steps that the recording in progress adds to the set; it is an
/// error. A change that leaves the file as large or makes it smaller is never refused.
pub fn include_steps(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let set = index(p, "set").ok_or("no such action set")?;
    let included = p.get("included").and_then(Value::as_bool).ok_or("included must be true or false")?;
    let others = match p.get("others") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err("others must be true or false".into()),
    };
    let action = if p.get("action").is_some() { Some(index(p, "action").ok_or("no such action")?) } else { None };
    let step = if p.get("step").is_some() { Some(index(p, "step").ok_or("no such action step")?) } else { None };
    if (step.is_some() && action.is_none()) || (others && step.is_none()) {
        return Err("no such action step".into());
    }
    let name = set_named(app, set)?;
    let members = model::members(&app.ui.actions, &name);
    let targets: Vec<usize> = match action {
        None => members.clone(),
        Some(at) => vec![*members.get(at).ok_or("no such action")?],
    };
    if let (Some(&flat), Some(si)) = (targets.first().filter(|_| action.is_some()), step)
        && si >= app.ui.actions.step_count(flat)
    {
        return Err("no such action step".into());
    }
    // The exclusions each action gets, worked out before any of them changes.
    let mut changes: Vec<(usize, usize, BTreeSet<usize>)> = Vec::new();
    let (mut now, mut then) = (0, 0);
    for &flat in &targets {
        let Some(a) = app.ui.actions.action(flat) else { continue };
        let count = a.steps.len();
        let every = || if included { BTreeSet::new() } else { (0..count).collect() };
        let mut excluded = match step {
            Some(_) if !others => a.disabled.clone(),
            _ => every(),
        };
        if let Some(si) = step {
            if if others { a.disabled.contains(&si) } else { !included } {
                excluded.insert(si);
            } else {
                excluded.remove(&si);
            }
        }
        now += a.disabled.range(..count).count();
        then += excluded.range(..count).count();
        changes.push((flat, count, excluded));
    }
    // Each excluded step takes [`files::EXCLUDED_BYTES`] in the file; only exclusions past the
    // ones the set has now can make it too large.
    if then > now {
        let [_, _, len] = model::set_sizes(app, &name, &model::stored_actions(&app.ui.actions, &name));
        if len + (then - now) * files::EXCLUDED_BYTES > files::MAX_SET_FILE {
            return Err("the action set would be larger than 8 MiB".into());
        }
    }
    for (flat, count, excluded) in changes {
        for i in 0..count {
            app.ui.actions.set_step(flat, i, !excluded.contains(&i)).map_err(list_error)?;
        }
    }
    Ok(json!({"set": set, "action": action, "step": step, "included": included}))
}

/// `ui.action.rename {set, action, name}`: renames the action at index `action` of set `set` (0 is
/// the top set) to `name` with spaces trimmed from both ends, and keeps its steps, function key and
/// color. An index past the last set or action, a name with nothing but spaces, a name another
/// action has, or a name that would give the set a file that Load Actions… refuses
/// ([`model::edit_keeps_set_loadable`]) is an error, and then nothing is renamed. → {name}
pub fn rename_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let (set, at, flat) = action_of(app, p)?;
    if name.is_empty() {
        return Err("name is empty".into());
    }
    keeps_loadable(app, &set_named(app, set)?, |actions| {
        if let Some(a) = actions.get_mut(at) {
            a.name = name.to_string();
        }
    })?;
    rename(app, flat, name)?;
    Ok(json!({ "name": name }))
}

/// `ui.action.new {set, name, key?, cmd?, shift?, color?, automaticAlignTo?}`: adds an action named `name`
/// (spaces trimmed from both ends, made unique when taken) after the last action of set `set` (0 is the top set)
/// and starts recording into it; Stop Playing/Recording ends the recording. `key` (1 to 15, F1 to F15; 0 or
/// null for none) with `cmd` (Ctrl, Command on macOS) and `shift` is the function key that plays
/// it, and `color` (the file name of an [`ActionColor`]; null or `""` for none) its Button Mode color. The action's
/// `automaticAlignTo` chooses Align's reference from the live selection at playback, false by default;
/// its recorded step params stay as they are. The function key takes priority over command shortcuts.
/// The set becomes current and the only selected set, and the new action is selected. While an
/// action is recording, and for an index past the last set, an empty name, a key or color that is
/// not one of them, or an action that would give the set a file that Load Actions… refuses
/// ([`model::edit_keeps_set_loadable`]: 1,000 actions, 8 MiB), nothing is added. The New Action dialog's Record runs this. → {set, action}
pub fn new_action(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    let set = index(p, "set").ok_or("no such action set")?;
    let set_name = set_named(app, set)?;
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    let key = FunctionKey::from_params(p)?;
    let color = color_param(p)?;
    let automatic_align_to = automatic_align_to_param(p)?.unwrap_or(false);
    if let Some(message) = key.and_then(key_error) {
        return Err(message);
    }
    let name = app.ui.actions.unique_name(name);
    let mut action = Action { set: Some(set_name.clone()), color, ..Action::new(name.clone(), vec![]) };
    model::set_function_key(&mut action, key);
    model::set_automatic_align_to(&mut action, automatic_align_to);
    keeps_loadable(app, &set_name, |actions| actions.push(action.clone()))?;
    let flat = app.ui.actions.push(action).map_err(list_error)?;
    super::recording::start_recording(app, set, name.clone(), Some(name));
    select(app, flat, None);
    let at = model::place(&app.ui.actions, flat).map_or(0, |(_, at)| at);
    Ok(json!({ "set": set, "action": at }))
}

/// `ui.action.options {set, action, name?, key?, cmd?, shift?, color?, automaticAlignTo?}`: changes the
/// action at index `action` of set `set` (0 is the top set) and keeps its steps. `name` renames it
/// (spaces trimmed from both ends), `key` (1 to 15, F1 to F15; 0 or null for none) with `cmd`
/// (Ctrl, Command on macOS) and `shift` sets the function key that plays it, and `color` (the file
/// name of an [`ActionColor`]; null or `""` for none) its Button Mode color; a param left out keeps
/// what the action has, and `cmd` or `shift` alone changes that modifier of the action's key.
/// `automaticAlignTo` chooses Align's reference from the live selection at playback; false uses the
/// recorded targets, and omission or null keeps the choice. Stored steps stay unchanged. The
/// action's function key takes priority over command shortcuts. An index past the last set or
/// action, an empty name, a name another action has, a key or color that is not one of them, or a
/// change that would give the set a file that Load Actions… refuses
/// ([`model::edit_keeps_set_loadable`]) is an error, and then nothing changes. The panel menu's
/// Action Options… runs this for the selected action. → {name, key, color, automaticAlignTo}
pub fn action_options(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (set, at, flat) = action_of(app, p)?;
    let had = app.ui.actions.action(flat).and_then(model::function_key);
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
    let automatic_align_to = automatic_align_to_param(p)?;
    if let Some(Some(k)) = key
        && had != Some(k)
        && let Some(message) = key_error(k)
    {
        return Err(message);
    }
    // The action as the edit leaves it, measured in its set before anything changes.
    let mut edited = app.ui.actions.action(flat).cloned().ok_or("no such action")?;
    if let Some(name) = &name {
        edited.name = name.clone();
    }
    if let Some(color) = color {
        edited.color = color;
    }
    if let Some(key) = key {
        model::set_function_key(&mut edited, key);
    }
    if let Some(on) = automatic_align_to {
        model::set_automatic_align_to(&mut edited, on);
    }
    keeps_loadable(app, &set_named(app, set)?, |actions| {
        if let Some(a) = actions.get_mut(at) {
            *a = edited.clone();
        }
    })?;
    if let Some(name) = &name {
        // The only edit that can be refused goes first, so a refusal changes nothing.
        rename(app, flat, name)?;
    }
    app.ui.actions.set_color(flat, edited.color).map_err(list_error)?;
    app.ui.actions.set_options(flat, edited.options.clone()).map_err(list_error)?;
    Ok(json!({
        "name": app.ui.actions.action(flat).map_or(edited.name.as_str(), |a| a.name.as_str()),
        "key": model::function_key(&edited),
        "color": (edited.color != ActionColor::None).then_some(edited.color.name()),
        "automaticAlignTo": model::automatic_align_to(&edited),
    }))
}

/// `ui.action.buttonMode {enabled?}`: with `enabled: true` the Actions panel shows every action of
/// every set as a button in its color that plays the action when clicked, and
/// with `false` it shows its list of sets, actions and steps again; left out or null, it switches
/// to the other view. The view is kept with the UI preferences. Turning Button Mode on while an
/// action records is an error, and so is an `enabled` that is not true, false or null; then the
/// view does not change. → {enabled}
pub fn button_mode(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let on = app.ui.actions_panel.settings.button_mode;
    let enabled = crate::menus::opt_bool(p, "enabled")?.unwrap_or(!on);
    if enabled && !on && app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    app.ui.actions_panel.settings.button_mode = enabled;
    Ok(json!({ "enabled": enabled }))
}

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
    let (message, allow_continue) = super::stop_params(p)?;
    insert(app, p, ActionStep::new(super::STOP_STEP, json!({"message": message, "allowContinue": allow_continue})))
}

/// `ui.action.insertMenuItem {set?, action?, step?, command, params?}`: inserts the command of a
/// menu item as a step, without running it. `command` and `params` (default: `{}`) are those of a
/// menu item that can be a step ([`crate::menus::insertable_items`]); the step plays as the item
/// runs. An Object › Align item's step holds the params that clicking the item sends now
/// ([`crate::panels::align::menu_align_params`]), and an Effect › Pathfinder item's step holds the
/// Pathfinder Options in force now ([`crate::dialogs::pathfinder_options::menu_effect_params`]), so
/// a later change of either does not change the step. A View menu item that the app runs itself
/// ([`crate::menus::plays_in_app`]) plays on its own, with no undo step. The place is as
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
    let items = crate::menus::insertable_items(app.session.prefs.font_names_in_english, &crate::menus::Listed::of(app));
    if !items.iter().any(|item| item.command == command && item.params == params) {
        return Err("no menu item that can be inserted runs this command".into());
    }
    let params = match command {
        "object.align" => crate::panels::align::menu_align_params(app, params),
        "effect.apply" => crate::dialogs::pathfinder_options::menu_effect_params(app, params),
        _ => params,
    };
    insert(app, p, ActionStep::new(command, params))
}

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
    insert(app, p, ActionStep::new(SELECT_PATH_STEP, params))
}

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

/// `ui.action.insertSelectObject {set?, action?, step?, note, caseSensitive?, wholeWord?}`: inserts
/// a Select Object step, without running it. Playback selects the objects whose Attributes note
/// matches `note` ([`SELECT_OBJECT_STEP`]); a playback that finds none stops there with an error.
/// The note and options are as [`select_object_params`] reads them. The place is as [`insert_stop`]
/// takes it, and the step is selected as a stop is. Bad params, or a place [`insert_stop`]
/// refuses, is an error, and then nothing changes. Set Selection's OK runs this. → {set, action,
/// step}, or {recording: true}
pub fn insert_select_object(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (note, case_sensitive, whole_word) = select_object_params(p)?;
    insert(app, p, ActionStep::new(SELECT_OBJECT_STEP, json!({"note": note, "caseSensitive": case_sensitive, "wholeWord": whole_word})))
}

/// Puts `step` where params `p` say ([`insert_stop`]) → what [`insert_stop`] returns. When the
/// recording in progress adds steps to the set, the limits count them as Stop Playing/Recording
/// would leave the set.
fn insert(app: &mut VectorcraftApp, p: &Value, step: ActionStep) -> Result<Value, String> {
    if p.get("set").is_none() && p.get("action").is_none() && p.get("step").is_none() {
        if app.ui.recording.is_none() {
            return Err("no such action".into());
        }
        model::recording_room(app, &step.command, &step.params)?;
        app.session.journal.push((step.command, step.params));
        return Ok(json!({"recording": true}));
    }
    let (set, at, flat) = action_of(app, p)?;
    let recorded = super::recording::recorded_into(app, set);
    let pending = recorded.as_ref().filter(|r| r.into == Some(at)).map_or(0, |r| r.steps.len());
    let count = app.ui.actions.step_count(flat);
    let after = match p.get("step") {
        None => count,
        Some(_) => index(p, "step").filter(|&s| s < count).ok_or("no such action step")? + 1,
    };
    if count + pending >= files::MAX_ACTION_STEPS {
        return Err("the action already has 10,000 steps".into());
    }
    keeps_loadable(app, &set_named(app, set)?, |actions| {
        if let Some(a) = actions.get_mut(at) {
            a.steps.insert(after, step.clone());
            a.remap_steps(|i| Some(if i >= after { i + 1 } else { i }));
        }
    })?;
    app.ui.actions.insert_step(flat, after, step).map_err(list_error)?;
    // The recording's steps stay after the steps that were before them.
    if let (Some(r), Some(slot)) = (&recorded, app.ui.recording_at)
        && r.into == Some(at)
        && after <= slot
    {
        app.ui.recording_at = Some(slot + 1);
    }
    select(app, flat, Some(after));
    Ok(json!({"set": set, "action": at, "step": after}))
}

/// `ui.action.deleteStep {set, action, step}`: deletes step `step`, a stop or a command, of the
/// action at index `action` of set `set`. The steps after it keep whether they are excluded, and
/// no step of that action stays selected. A Record Stop, Insert Menu Item or Set Selection dialog open on that step
/// closes, and one open on a later step of the action follows it up. A missing or invalid index is an error, and
/// then nothing changes. Undo does not bring the step back. Delete Selection runs this for the
/// selected step. → {set, action, step}
pub fn delete_step(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (set, at, flat) = action_of(app, p)?;
    let step = index(p, "step").filter(|&s| s < app.ui.actions.step_count(flat)).ok_or("no such action step")?;
    let name = app.ui.actions.action(flat).map(|a| a.name.clone()).unwrap_or_default();
    app.ui.actions.delete_step(flat, step).map_err(list_error)?;
    // The recording's steps stay after the steps that were before them.
    if app.ui.recording_into.as_deref() == Some(name.as_str())
        && let Some(slot) = app.ui.recording_at.filter(|&slot| step < slot)
    {
        app.ui.recording_at = Some(slot - 1);
    }
    if matches!(app.ui.actions_panel.selected, Some(ActionSelection::Step(a, _)) if a == flat) {
        app.ui.actions_panel.selected = Some(ActionSelection::Action(flat));
    }
    follow_step_dialogs(app, set, at, step, -1);
    Ok(json!({"set": set, "action": at, "step": step}))
}

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
/// ([`model::edit_keeps_set_loadable`]: 10,000 steps in an action, 8 MiB), is an error, and then nothing
/// changes. Undo does not move the step back. A drag of a step row in the panel runs this.
/// → {set, action, step, moved}: where the step is now
pub fn move_step(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (set, at, source) = action_of(app, p)?;
    let step = index(p, "step").filter(|&s| s < app.ui.actions.step_count(source)).ok_or("no such action step")?;
    let to_set = match p.get("toSet") {
        None => set,
        Some(_) => index(p, "toSet").filter(|&s| model::set_name(&app.ui.actions, s).is_some()).ok_or("no such action set")?,
    };
    let to_action = match p.get("toAction") {
        None => at,
        Some(_) => index(p, "toAction").ok_or("no such action")?,
    };
    let dest = model::flat(&app.ui.actions, to_set, to_action).ok_or("no such action")?;
    let before = index(p, "before").filter(|&b| b <= app.ui.actions.step_count(dest)).ok_or("no such action step")?;
    let same = source == dest;
    if same && (before == step || before == step + 1) {
        return Ok(json!({"set": set, "action": at, "step": step, "moved": false}));
    }
    // The step's index once it is in place, counted without it in its own action.
    let to = if same && before > step { before - 1 } else { before };
    // The limits run on the sets as the move leaves them.
    let moved = app.ui.actions.action(source).and_then(|a| a.steps.get(step)).cloned().ok_or("no such action step")?;
    let take = |actions: &mut Vec<Action>| {
        if let Some(a) = actions.get_mut(at) {
            a.steps.remove(step);
        }
    };
    let put = |actions: &mut Vec<Action>| {
        if let Some(a) = actions.get_mut(to_action) {
            a.steps.insert(to.min(a.steps.len()), moved.clone());
        }
    };
    let (from_name, to_name) = (set_named(app, set)?, set_named(app, to_set)?);
    if to_set == set {
        keeps_loadable(app, &from_name, |actions| {
            take(actions);
            put(actions);
        })?;
    } else {
        keeps_loadable(app, &to_name, put)?;
        keeps_loadable(app, &from_name, take)?;
    }
    let name_of = |app: &VectorcraftApp, flat: usize| app.ui.actions.action(flat).map(|a| a.name.clone());
    let (source_name, dest_name) = (name_of(app, source), name_of(app, dest));
    app.ui.actions.move_step(source, step, dest, to).map_err(list_error)?;
    // A step index in action `source` or `dest` as (the action's list index, the step) after the move.
    let follow = |(id, s): (usize, usize)| -> (usize, usize) {
        if (id, s) == (source, step) {
            return (dest, to);
        }
        let s = if id == source && s > step { s - 1 } else { s };
        (id, if id == dest && s >= to { s + 1 } else { s })
    };
    // The recording's steps stay after the steps that were before them.
    if let Some(slot) = app.ui.recording_at {
        let into = app.ui.recording_into.as_deref();
        let slot = if into.is_some() && into == source_name.as_deref() && step < slot { slot - 1 } else { slot };
        app.ui.recording_at = Some(if into.is_some() && into == dest_name.as_deref() && to <= slot { slot + 1 } else { slot });
    }
    if let Some(ActionSelection::Step(a, s)) = app.ui.actions_panel.selected.clone() {
        let (a2, s2) = follow((a, s));
        if (a, s) == (source, step) {
            select(app, a2, Some(s2));
        } else {
            app.ui.actions_panel.selected = Some(ActionSelection::Step(a2, s2));
        }
    }
    // A step dialog open on a step of either action follows it.
    retarget_step_dialogs(app, |(ds, da, held)| {
        let Some(held) = held.filter(|_| (ds, da) == (set, at) || (ds, da) == (to_set, to_action)) else { return Some((ds, da, held)) };
        let id = if (ds, da) == (set, at) { source } else { dest };
        let (id, s) = follow((id, held));
        let (ds, da) = if id == dest { (to_set, to_action) } else { (set, at) };
        Some((ds, da, Some(s)))
    });
    Ok(json!({"set": to_set, "action": to_action, "step": to, "moved": true}))
}

/// Makes the set named `name`, empty, and selects it. A name that is empty or taken, or one that would
/// give the set a file Load Actions… refuses ([`model::name_keeps_set_loadable`]), is an error.
pub(crate) fn new_set(app: &mut VectorcraftApp, name: &str) -> Result<(), String> {
    let name = name.trim();
    model::name_keeps_set_loadable(app, None, name)?;
    app.ui.actions.new_set(name).map_err(list_error)?;
    app.ui.action_set = Some(name.to_string());
    app.ui.actions_panel.selected = Some(ActionSelection::Set(name.to_string()));
    app.ui.actions_panel.selected_sets = vec![name.to_string()];
    Ok(())
}

/// Renames the set `set` to `name`; a selection, a collapse, the current set and an open dialog keep
/// the set. A name that would give the set a file Load Actions… refuses, counting the steps the
/// recording in progress adds to it ([`model::name_keeps_set_loadable`]), is an error, and then
/// nothing is renamed.
pub(crate) fn rename_set(app: &mut VectorcraftApp, set: &str, name: &str) -> Result<(), String> {
    let name = name.trim();
    model::name_keeps_set_loadable(app, Some(set), name)?;
    app.ui.actions.rename_set(set, name).map_err(list_error)?;
    // A recording into the set measures its file again.
    app.ui.recording_base = None;
    let renamed = |s: &mut String| {
        if s == set {
            *s = name.to_string();
        }
    };
    let panel = &mut app.ui.actions_panel;
    if let Some(ActionSelection::Set(s)) = &mut panel.selected {
        renamed(s);
    }
    panel.selected_sets.iter_mut().for_each(renamed);
    panel.settings.collapsed_sets.iter_mut().for_each(renamed);
    if let Some(current) = &mut app.ui.action_set {
        renamed(current);
    }
    Ok(())
}

/// `ui.actionSet.delete {names?: [name]}`: deletes the action sets with these names, with their
/// actions (default: the current set). A name that is gone is an error ("the action sets changed;
/// nothing was deleted") and nothing is deleted. A recording into a deleted set stops, and its steps
/// are dropped; a recording into another set goes on. An open Set Options, Batch or step dialog for
/// a deleted set closes. When the current set is deleted, no set is current afterwards. → {deleted: n}
pub fn delete_sets(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let sets: Vec<String> = app.ui.actions.sets().into_iter().map(str::to_string).collect();
    let names: Vec<String> = match p.get("names") {
        None | Some(Value::Null) => app.ui.action_set.clone().filter(|current| sets.contains(current)).into_iter().collect(),
        Some(v) => v
            .as_array()
            .ok_or("the action set names are not a list")?
            .iter()
            .map(|n| n.as_str().map(str::to_string).filter(|n| sets.contains(n)).ok_or("the action sets changed; nothing was deleted"))
            .collect::<Result<_, _>>()?,
    };
    let mut at: Vec<usize> = names.iter().filter_map(|n| sets.iter().position(|s| s == n)).collect();
    at.sort_unstable();
    at.dedup();
    // The selected action is found again by name once the list has shifted.
    let selected = match &app.ui.actions_panel.selected {
        Some(ActionSelection::Action(i) | ActionSelection::Step(i, _)) => {
            app.ui.actions.action(*i).map(|a| (a.name.clone(), model::place(&app.ui.actions, *i)))
        }
        _ => None,
    };
    let selected_step = match app.ui.actions_panel.selected {
        Some(ActionSelection::Step(_, s)) => Some(s),
        _ => None,
    };
    if let Some((rec, ..)) = app.ui.recording.as_mut() {
        if at.contains(rec) {
            app.ui.recording = None;
            app.ui.recording_into = None;
            app.ui.recording_at = None;
            app.session.journal.hold(None);
        } else {
            *rec -= at.iter().filter(|&&s| s < *rec).count();
        }
    }
    for &s in at.iter().rev() {
        close_set_dialogs(app, s);
        if let Some(name) = sets.get(s) {
            app.ui.actions.delete_set(name).map_err(list_error)?;
        }
    }
    if !at.is_empty() {
        app.ui.action_set = app.ui.action_set.take().filter(|current| !names.contains(current));
        let panel = &mut app.ui.actions_panel;
        panel.selected_sets.clear();
        if matches!(&panel.selected, Some(ActionSelection::Set(s)) if names.contains(s)) {
            panel.selected = None;
        }
        // The panel's selected action follows the list down, and is no longer selected when its set is deleted.
        if let Some((name, place)) = selected {
            let kept = place.is_some_and(|(s, _)| !at.contains(&s));
            let found = kept.then(|| app.ui.actions.actions().iter().position(|a| a.name == name)).flatten();
            app.ui.actions_panel.selected = found.map(|i| match selected_step {
                Some(step) => ActionSelection::Step(i, step),
                None => ActionSelection::Action(i),
            });
        }
    }
    Ok(json!({ "deleted": at.len() }))
}

/// `ui.actionSet.duplicate {set?: index, action?: index}`: copies the action at index `action`
/// of set `set` into the same set, right after the original; without `action`, copies set `set`
/// with all its actions to right after the original set (default: the current set). The copy is
/// named after the original with " copy" added. The copy is selected and its set is the current one.
/// A recording and an open Set Options or Batch dialog keep to their own set and action. A copy that
/// would give its set a file that Load Actions… refuses ([`model::edit_keeps_set_loadable`]) is an
/// error, and nothing is copied. → {set, action?}
pub fn duplicate(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let set = match p.get("set") {
        None | Some(Value::Null) => app
            .ui
            .action_set
            .as_deref()
            .and_then(|current| app.ui.actions.sets().iter().position(|s| *s == current))
            .ok_or("no action set is current")?,
        Some(_) => index(p, "set").filter(|&s| model::set_name(&app.ui.actions, s).is_some()).ok_or("no such action set")?,
    };
    let name = set_named(app, set)?;
    let members = model::members(&app.ui.actions, &name);
    let action = match p.get("action") {
        None | Some(Value::Null) => None,
        Some(_) => Some(index(p, "action").filter(|&a| a < members.len()).ok_or("no such action")?),
    };
    if let Some(at) = action {
        let flat = *members.get(at).ok_or("no such action")?;
        keeps_loadable(app, &name, |actions| {
            if let Some(copy) = actions.get(at).cloned() {
                actions.insert(at + 1, copy);
            }
        })?;
        // The steps hold engine commands only, so nothing in them points at an action.
        let copy = app.ui.actions.duplicate_action(flat, |_, _, _| {}).map_err(list_error)?;
        drop_function_keys(app, &[copy])?;
        // A Record Stop, Insert Menu Item or Set Selection dialog open on a later action of the set follows it down.
        retarget_step_dialogs(app, |(s, a, step)| Some((s, if s == set && a > at { a + 1 } else { a }, step)));
        retarget_batch_actions(app, |(s, a)| (s, if s == set && a > at { a + 1 } else { a }));
        select(app, copy, None);
        return Ok(json!({ "set": set, "action": at + 1 }));
    }
    // The copy is a new set, measured as Load Actions… measures one, under the names it gets.
    let mut trial = app.ui.actions.clone();
    let named = trial.duplicate_set(&name, |_, _, _| {}).map_err(list_error)?;
    model::edit_keeps_set_loadable(app, &named, &[], &model::stored_actions(&trial, &named))?;
    let copy = app.ui.actions.duplicate_set(&name, |_, _, _| {}).map_err(list_error)?;
    let at = app.ui.actions.sets().iter().position(|s| *s == copy).ok_or("no such action set")?;
    drop_function_keys(app, &model::members(&app.ui.actions, &copy))?;
    if let Some((rec, ..)) = app.ui.recording.as_mut()
        && *rec >= at
    {
        *rec += 1;
    }
    retarget_set_dialogs(app, |held| Some(if held >= at { held + 1 } else { held }));
    app.ui.action_set = Some(copy.clone());
    let panel = &mut app.ui.actions_panel;
    panel.selected = Some(ActionSelection::Set(copy.clone()));
    panel.selected_sets = vec![copy];
    Ok(json!({ "set": at }))
}

/// Takes the function keys from the actions at `copies`, so a key still plays only the original.
fn drop_function_keys(app: &mut VectorcraftApp, copies: &[usize]) -> Result<(), String> {
    for &index in copies {
        let mut action = app.ui.actions.action(index).cloned().ok_or("no such action")?;
        model::set_function_key(&mut action, None);
        app.ui.actions.set_options(index, action.options).map_err(list_error)?;
    }
    Ok(())
}

/// Moves the Batch dialog's action: `f` gives the new (set, action).
fn retarget_batch_actions(app: &VectorcraftApp, f: impl Fn((usize, usize)) -> (usize, usize)) {
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| {
            if let Some(d) = host.get_mut::<BatchDialog>() {
                (d.set, d.action) = f((d.set, d.action));
            }
        });
    }
}

/// Ends the panel's per-list state after the whole list changed: nothing is selected or expanded,
/// and no dialog that holds a set stays open.
fn clear_panel(app: &mut VectorcraftApp) {
    let panel = &mut app.ui.actions_panel;
    panel.selected = None;
    panel.selected_sets.clear();
    panel.expanded.clear();
    retarget_set_dialogs(app, |_| None);
}

/// After Reset Actions or Replace Actions… put new sets in place of every set: the first becomes
/// current and selected, and the panel clears what named the old sets ([`clear_panel`]).
pub(super) fn replace_sets(app: &mut VectorcraftApp) {
    clear_panel(app);
    let first = app.ui.actions.sets().first().map(|s| s.to_string());
    app.ui.actions_panel.selected = first.clone().map(ActionSelection::Set);
    app.ui.action_set = first;
}

/// Refuses a whole-list command while an action records or plays (also while it waits at a stop).
fn require_idle(app: &VectorcraftApp) -> Result<(), String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    if app.ui.action_playing.is_some() {
        return Err("an action is already playing".into());
    }
    Ok(())
}

/// The params of a whole-list command as an object with at most the key `only`, or null for none.
fn object_params<'a>(p: &'a Value, only: &str) -> Result<Option<&'a serde_json::Map<String, Value>>, String> {
    match p {
        Value::Null => Ok(None),
        Value::Object(m) => match m.keys().find(|k| *k != only) {
            Some(key) => Err(format!("unknown parameter `{key}`; the command takes: {only}")),
            None => Ok(Some(m)),
        },
        got => Err(format!("the params must be an object, not {got}")),
    }
}

/// `ui.actionSet.reset {mode?: "replace" | "append"}`: Reset Actions. `replace` (the default)
/// replaces every set in the panel with a new copy of the built-in sets ([`model::default_sets`]),
/// makes the first one current and selected, clears the panel's selected action and step and its
/// expanded actions, and closes an open Set Options, Batch or step dialog that names a set. When the
/// panel already shows the built-in sets that way, nothing changes. `append` adds a new copy of the
/// built-in sets after the sets there are, a name already in use getting the next free number
/// (`Default Actions 2`), and keeps the current set and the selection. Omitted params (`null`) count as
/// `{}`. Params of another type, another key, or another `mode` is an error, and so is a reset while
/// an action records or plays (also while it waits at a stop); then nothing changes. No undo step is
/// recorded. → {changed, mode, set: the index of the first built-in set}
pub fn reset(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let append = match object_params(p, "mode")?.and_then(|m| m.get("mode")) {
        None => false,
        Some(v) => match v.as_str() {
            Some("replace") => false,
            Some("append") => true,
            _ => return Err("mode must be replace or append".into()),
        },
    };
    require_idle(app)?;
    if append {
        let first = app.ui.actions.sets().len();
        // The list joins a set to one of the same name: the copy gets a free name instead.
        let mut defaults = model::default_sets();
        for action in &mut defaults.actions {
            action.set = action.set.as_deref().map(|set| model::free_set_name(&app.ui.actions, set));
        }
        app.ui.actions.reset(defaults, true, |_, _| {});
        return Ok(json!({ "changed": true, "mode": "append", "set": first }));
    }
    let defaults = model::default_list();
    let first = defaults.sets().first().map(|s| s.to_string());
    let panel = &app.ui.actions_panel;
    let shown = app.ui.action_set == first
        && panel.selected == first.clone().map(ActionSelection::Set)
        && panel.selected_sets.is_empty()
        && panel.expanded.is_empty()
        && !set_dialog_open(app);
    if shown && app.ui.actions.actions() == defaults.actions() && app.ui.actions.sets() == defaults.sets() {
        return Ok(json!({ "changed": false, "mode": "replace", "set": 0 }));
    }
    app.ui.actions.reset(model::default_sets(), false, |_, _| {});
    replace_sets(app);
    Ok(json!({ "changed": true, "mode": "replace", "set": 0 }))
}

/// `ui.actionSet.clear {names?: [name]}`: Clear Actions. Deletes every set in the panel with its
/// actions, the built-in Default Actions set included, so no set is current or selected; the
/// panel's selected action and step and its expanded actions are cleared, and an open Set Options,
/// Batch or step dialog that names a set closes. `names` holds the panel's set names in panel order,
/// which the menu's question passes: when the sets are no longer exactly those, nothing is deleted.
/// With no sets, nothing changes. Omitted params (`null`) count as `{}`. Params of another type,
/// another key, `names` that is not a list of names, or a clear while an action records or plays
/// (also while it waits at a stop) is an error, and then nothing changes. No undo step is recorded.
/// → {cleared: the number of sets deleted}
pub fn clear(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let names = match object_params(p, "names")?.and_then(|m| m.get("names")) {
        None => None,
        Some(v) => Some(
            v.as_array()
                .ok_or("the action set names are not a list")?
                .iter()
                .map(|n| n.as_str().ok_or("the action set names are not a list"))
                .collect::<Result<Vec<&str>, _>>()?,
        ),
    };
    require_idle(app)?;
    if names.is_some_and(|names| names != app.ui.actions.sets()) {
        return Err("the action sets changed; nothing was deleted".into());
    }
    let cleared = app.ui.actions.sets().len();
    if cleared == 0 {
        return Ok(json!({ "cleared": 0 }));
    }
    app.ui.actions.clear_all();
    app.ui.action_set = None;
    clear_panel(app);
    Ok(json!({ "cleared": cleared }))
}

/// Opens the question of deleting the sets `names`; the answer runs [`DELETE_SETS`] ([`take_answers`]).
pub(crate) fn ask_delete(app: &mut VectorcraftApp, names: Vec<String>) {
    let prompt = StandardPrompt::confirm_delete(names.iter().take(4), names.len()).tag(names);
    open_or_say(app, ActionsQuestion(prompt));
}

/// Opens Clear Actions' question; OK runs [`CLEAR`].
pub(crate) fn ask_clear(app: &mut VectorcraftApp) {
    ask_library(app, &LibraryPrompt::ClearAll);
}

/// Opens Reset Actions' question; OK runs [`RESET`] replacing, Append runs it appending.
pub(crate) fn ask_reset(app: &mut VectorcraftApp) {
    ask_library(app, &LibraryPrompt::Reset);
}

/// Opens the panel's question `question`, in the interface language.
fn ask_library(app: &mut VectorcraftApp, question: &LibraryPrompt) {
    match app.ui_ctx.as_ref().map(Env::of_root) {
        Some(env) => {
            let prompt = question.prompt(&env.words::<ActionsWords>(), &env.words::<CommonWords>());
            open_or_say(app, ActionsQuestion(prompt));
        }
        None => app.status("no window"),
    }
}

/// Runs the command an answered question confirms. Called once per frame by the dialogs pump; a second
/// pass in the frame finds no answer, as taking removes it. A cancel changes nothing.
pub(crate) fn take_answers(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(DialogResult::Ok(answer)) = dialogs.with(|host| host.take::<ActionsQuestion>()) else { return };
    let (id, params) = match (answer.tag::<LibraryPrompt>(), answer.tag::<Vec<String>>(), answer.choice) {
        (_, _, PromptChoice::Cancel) => return,
        (Some(LibraryPrompt::ClearAll), ..) => (CLEAR, json!({ "names": app.ui.actions.sets() })),
        (Some(LibraryPrompt::Reset), _, PromptChoice::Alternate) => (RESET, json!({ "mode": "append" })),
        (Some(LibraryPrompt::Reset), ..) => (RESET, json!({ "mode": "replace" })),
        (_, Some(names), _) => (DELETE_SETS, json!({ "names": names })),
        // The Replace question is asked and answered by the file commands.
        _ => return,
    };
    if let Err(e) = app.run(id, params) {
        app.status(e);
    }
}
