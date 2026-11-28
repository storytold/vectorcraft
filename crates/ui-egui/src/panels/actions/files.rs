//! Action set files: the Actions panel menu's Save Actions…, Load Actions… and Replace Actions….
//!
//! An action set file (`.vcactions`) is JSON: `{"format": "vcactions", "version": 1 to 7, "name",
//! "actions": [{"name", "steps": [{"command", "params", "excluded"?}], "key"?, "color"?, "automaticAlignTo"?}]}`. Each step is a
//! command id and its params, as `command.batch` takes them, or a stop: the command `action.stop`
//! with `{"message", "allowContinue"}`. A Select Object step is the command `select.byNote` with
//! `{"note", "caseSensitive", "wholeWord"}`, and a Select Path step is the command `select.storedPath`
//! with `{"id", "fingerprint"}`. `key` is the action's function key
//! (`{"key": 1 to 15, "cmd", "shift"}`) and `color` its Button Mode color; an action without them,
//! or with a value that is not one, has none. `excluded: true` keeps a step but skips it during
//! playback. Sets with exclusions use version 2 so older builds refuse them rather than replay
//! excluded steps, sets with stops use version 3 so older builds refuse them rather than fail at a
//! stop, sets with Select Object steps use version 4, sets with Select Path steps use version 5 and
//! sets with View menu steps ([`crate::menus::plays_in_app`]), whose params are an object, use
//! version 6 so older builds refuse them rather than fail at that step. Other sets use version 1, and
//! earlier version 1 to 6 files still work. Automatic Align playback, whether the action enables it or
//! a stored step explicitly uses `object.align` with `to: "auto"`, requires version 7, including in
//! excluded steps, recorded batches and recorded Batch runs, so older builds cannot silently lose that choice.
//! Load Actions… also reads the action sets another app saves as text (`.aia`, through
//! `actionSet.read`), a Set Selection step as a Select Object step.

use craft::automation::actions::{Action, ActionColor, ActionStep, ActionsFile};
use craft::host::{PickKind, PickRequest, Picked}; // PLAN: assumed path of the host's picker types in the app's `craft` facade
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::keys::FunctionKey;
use super::model;
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner}; // PLAN: assumed home of fl.A3 and fl.A6

/// The command that saves an action set to a file ([`save_set`]).
pub const SAVE_SET: &str = "ui.actionSet.save";

/// The command that loads an action set from a file ([`load_set`]).
pub const LOAD_SET: &str = "ui.actionSet.load";

/// The command that replaces every action set with the one from a file ([`replace_set`]).
pub const REPLACE_SET: &str = "ui.actionSet.replace";

/// The extension of an action set file ([`write_set`]).
pub const ACTIONS_EXT: &str = "vcactions";

/// The largest action set file [`read_set`] reads.
pub(super) const MAX_SET_FILE: usize = 8 << 20;

/// The most actions [`read_set`] reads from one file.
pub(super) const MAX_SET_ACTIONS: usize = 1000;

/// The most steps [`read_set`] reads in one action.
pub(super) const MAX_ACTION_STEPS: usize = 10_000;

/// The most left-out steps that Load Actions… lists for an action set another app saved, and the
/// most steps with an unknown command it lists for an action set file.
const MAX_SKIPPED: usize = 1000;

/// An action set file: a header, the set's name and its actions.
#[derive(Serialize, Deserialize)]
struct SetFile {
    format: String,
    version: u32,
    name: String,
    actions: Vec<FileAction>,
}

/// An action in an action set file.
#[derive(Serialize, Deserialize)]
struct FileAction {
    name: String,
    steps: Vec<FileStep>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_key")]
    key: Option<FunctionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_color")]
    color: Option<String>,
    #[serde(default, rename = "automaticAlignTo", skip_serializing_if = "is_false")]
    automatic_align_to: bool,
}

/// A step of an action in an action set file: a command id and its params, as `command.batch`
/// takes them.
#[derive(Serialize, Deserialize)]
struct FileStep {
    command: String,
    #[serde(default)]
    params: Value,
    #[serde(default, skip_serializing_if = "is_false")]
    excluded: bool,
}

pub(super) fn is_false(value: &bool) -> bool {
    !value
}

/// Serde reader for a saved function key: a value that is not a function key is none.
fn deserialize_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<FunctionKey>, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(model::function_key_from(&v))
}

/// Serde reader for a saved Button Mode color: a value that is not one of the color names is none.
fn deserialize_color<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_str().filter(|c| ActionColor::from_name(c) != ActionColor::None).map(str::to_string))
}

/// A step as [`write_set`] writes it when it is included: [`FileStep`] without `excluded`.
#[derive(Serialize)]
struct IncludedStep<'a> {
    command: &'a str,
    params: &'a Value,
}

/// The bytes that excluding one step adds to the text [`write_set`] writes: a comma, a line
/// break, the indent and `"excluded": true`.
pub(super) const EXCLUDED_BYTES: usize = 28;

/// The bytes more that a step adds to the text [`write_set`] writes as an action's first step than
/// [`step_bytes`] counts: a line break on each side of the step and the indent of the closing
/// bracket, in place of a comma and a line break.
pub(super) const FIRST_STEP_BYTES: usize = 6;

/// The bytes that the included step `(command, params)` adds to the text [`write_set`] writes when
/// it goes after the last step of an action. As the action's first step it adds
/// [`FIRST_STEP_BYTES`] more.
pub(super) fn step_bytes(command: &str, params: &Value) -> usize {
    /// Counts the bytes and the line breaks written to it.
    #[derive(Default)]
    struct Count {
        bytes: usize,
        breaks: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes += buf.len();
            self.breaks += buf.iter().filter(|&&b| b == b'\n').count();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count::default();
    if serde_json::to_writer_pretty(&mut count, &IncludedStep { command, params }).is_err() {
        return MAX_SET_FILE + 1;
    }
    // In the file every line of the step is indented by 8 more spaces, and a comma and a line break
    // come before it.
    count.bytes + 8 * (count.breaks + 1) + 2
}

/// The params of the stops in a step: a stop's own, or those of the stops a `command.batch` step
/// holds, which playback runs as its own steps.
fn stops_in<'a>(command: &str, params: &'a Value) -> Vec<&'a Value> {
    steps_in(super::STOP_STEP, command, params)
}

/// The params of the Select Object steps in a step, as [`stops_in`] finds stops.
fn select_objects_in<'a>(command: &str, params: &'a Value) -> Vec<&'a Value> {
    steps_in(super::SELECT_OBJECT_STEP, command, params)
}

/// The params of the steps that run command `kind` in a step: the step's own, or those of the
/// steps a `command.batch` step holds, which playback runs as its own steps.
fn steps_in<'a>(kind: &str, command: &str, params: &'a Value) -> Vec<&'a Value> {
    if command == kind {
        return vec![params];
    }
    if command != "command.batch" {
        return vec![];
    }
    let steps = params.get("commands").and_then(Value::as_array).into_iter().flatten();
    steps.filter(|step| step.get("command").and_then(Value::as_str) == Some(kind)).map(|step| step.get("params").unwrap_or(&Value::Null)).collect()
}

/// The params of the Select Path steps in a step, as [`stops_in`] finds stops.
fn select_paths_in<'a>(command: &str, params: &'a Value) -> Vec<&'a Value> {
    steps_in(super::SELECT_PATH_STEP, command, params)
}

/// The params of the View menu steps ([`crate::menus::plays_in_app`]) in a step, as [`stops_in`]
/// finds stops.
fn ui_steps_in<'a>(command: &str, params: &'a Value) -> Vec<&'a Value> {
    if crate::menus::plays_in_app(command) {
        return vec![params];
    }
    if command != "command.batch" {
        return vec![];
    }
    let steps = params.get("commands").and_then(Value::as_array).into_iter().flatten();
    steps
        .filter(|step| step.get("command").and_then(Value::as_str).is_some_and(crate::menus::plays_in_app))
        .map(|step| step.get("params").unwrap_or(&Value::Null))
        .collect()
}

/// A folder preference whose value in a step running `command` with `params` names a folder
/// ([`vectorcraft_engine::cmd::prefscmds::names_a_folder`]): in a `prefs.set` step, given as
/// `{key, value}` or in `values`, and in a `command.batch` step, in any of its commands at any depth.
/// Every such value is found, also one under `values` that `{key, value}` replaces when the step runs,
/// because [`write_set`] writes the params as they are.
fn folder_pref_in(command: &str, params: &Value) -> Option<String> {
    use vectorcraft_engine::cmd::prefscmds::names_a_folder;
    match command {
        "prefs.set" => {
            let names = |key: &str, value: &Value| value.as_str().is_some_and(|v| names_a_folder(key, v));
            let pair = params.get("key").and_then(Value::as_str).filter(|key| params.get("value").is_some_and(|v| names(key, v)));
            let values = params.get("values").and_then(Value::as_object).into_iter().flatten();
            pair.map(str::to_string).or_else(|| values.filter(|(key, v)| names(key, v)).map(|(key, _)| key.clone()).next())
        }
        "command.batch" => {
            let mut steps = params.get("commands").and_then(Value::as_array).into_iter().flatten();
            steps.find_map(|step| {
                folder_pref_in(step.get("command").and_then(Value::as_str).unwrap_or_default(), step.get("params").unwrap_or(&Value::Null))
            })
        }
        _ => None,
    }
}

/// Whether the raw step stores automatic Align semantics, including in excluded recorded batches and
/// in the steps of a recorded Batch run (`action.batch`).
fn automatic_align_in(command: &str, params: &Value) -> bool {
    match command {
        "object.align" => params.get("to").and_then(Value::as_str) == Some("auto"),
        "command.batch" | "action.batch" => params.get("commands").and_then(Value::as_array).is_some_and(|steps| {
            steps.iter().any(|step| {
                automatic_align_in(step.get("command").and_then(Value::as_str).unwrap_or_default(), step.get("params").unwrap_or(&Value::Null))
            })
        }),
        _ => false,
    }
}

/// The set `name` with `actions` as the text of an action set file.
pub fn write_set(name: &str, actions: &[Action]) -> String {
    let file_actions = actions
        .iter()
        .map(|a| FileAction {
            name: a.name.clone(),
            steps: a
                .steps
                .iter()
                .enumerate()
                .map(|(i, s)| FileStep { command: s.command.clone(), params: s.params.clone(), excluded: a.disabled.contains(&i) })
                .collect(),
            key: model::function_key(a),
            color: (a.color != ActionColor::None).then(|| a.color.name().to_string()),
            automatic_align_to: model::automatic_align_to(a),
        })
        .collect();
    let steps = || actions.iter().flat_map(|a| &a.steps);
    let has_select_object = steps().any(|s| !select_objects_in(&s.command, &s.params).is_empty());
    let has_select_path = steps().any(|s| !select_paths_in(&s.command, &s.params).is_empty());
    let has_stop = steps().any(|s| !stops_in(&s.command, &s.params).is_empty());
    let has_ui_step = steps().any(|s| !ui_steps_in(&s.command, &s.params).is_empty());
    let has_automatic_align =
        actions.iter().any(|a| model::automatic_align_to(a) || a.steps.iter().any(|s| automatic_align_in(&s.command, &s.params)));
    let version = match (has_select_path, has_select_object, has_stop, actions.iter().any(|a| !a.disabled.is_empty())) {
        _ if has_automatic_align => 7,
        _ if has_ui_step => 6,
        (true, ..) => 5,
        (false, true, ..) => 4,
        (false, false, true, _) => 3,
        (false, false, false, true) => 2,
        (false, false, false, false) => 1,
    };
    let file = SetFile { format: ACTIONS_EXT.into(), version, name: name.to_string(), actions: file_actions };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// The set in the text of an action set file: its name (`name` when the file gives none) and its
/// actions, which are in no set yet: the caller places them.
pub fn read_set(text: &str, name: &str) -> Result<(String, Vec<Action>), String> {
    if text.len() > MAX_SET_FILE {
        return Err("the action set file is larger than 8 MiB".into());
    }
    let f: SetFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not an action set file ({e})"))?;
    if f.format != ACTIONS_EXT {
        return Err("not an action set file".into());
    }
    if !matches!(f.version, 1..=7) {
        return Err("the action set file needs a later version of VectorCraft".into());
    }
    if f.actions.len() > MAX_SET_ACTIONS {
        return Err("the action set file holds more than 1,000 actions".into());
    }
    if f.actions.iter().any(|a| a.steps.len() > MAX_ACTION_STEPS) {
        return Err("an action in the action set file has more than 10,000 steps".into());
    }
    let mut stops = f.actions.iter().flat_map(|a| &a.steps).flat_map(|s| stops_in(&s.command, &s.params));
    if stops.any(|p| super::stop_params(p).is_err()) {
        return Err("not an action set file".into());
    }
    let mut select_paths = f.actions.iter().flat_map(|a| &a.steps).flat_map(|s| select_paths_in(&s.command, &s.params));
    if select_paths.any(|p| vectorcraft_engine::stored_path_params(p).is_err()) {
        return Err("not an action set file".into());
    }
    let mut select_objects = f.actions.iter().flat_map(|a| &a.steps).flat_map(|s| select_objects_in(&s.command, &s.params));
    if select_objects.any(|p| super::select_object_params(p).is_err()) {
        return Err("not an action set file".into());
    }
    let mut ui_steps = f.actions.iter().flat_map(|a| &a.steps).flat_map(|s| ui_steps_in(&s.command, &s.params));
    if ui_steps.any(|p| !p.is_object()) {
        return Err("not an action set file".into());
    }
    let actions = f
        .actions
        .into_iter()
        .map(|a| {
            let disabled = a.steps.iter().enumerate().filter_map(|(i, step)| step.excluded.then_some(i)).collect();
            let steps = a.steps.into_iter().map(|s| ActionStep::new(s.command, s.params)).collect();
            let mut action =
                Action { disabled, color: a.color.as_deref().map(ActionColor::from_name).unwrap_or_default(), ..Action::new(a.name, steps) };
            model::set_function_key(&mut action, a.key);
            model::set_automatic_align_to(&mut action, a.automatic_align_to);
            action
        })
        .collect();
    let set_name = if f.name.trim().is_empty() { name.to_string() } else { f.name };
    Ok((set_name, actions))
}

/// `name` as a file name: the characters file systems refuse become `-`, and a Windows device name
/// before the first period (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`, also
/// with `¹`, `²` or `³`, in any case) gets `-` after it, on every platform.
pub(super) fn set_file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '-' } else { c }).collect();
    let stem = stem.trim_matches(['.', ' ']);
    if stem.is_empty() {
        return "Actions".into();
    }
    let (base, rest) = stem.split_at(stem.find('.').unwrap_or(stem.len()));
    let base = base.trim_end_matches(' ');
    let upper = base.to_ascii_uppercase();
    let numbered = |n: &str| n.chars().count() == 1 && n.chars().all(|c| matches!(c, '1'..='9' | '¹' | '²' | '³'));
    let device = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper.strip_prefix("COM").or_else(|| upper.strip_prefix("LPT")).is_some_and(numbered);
    if device { format!("{base}-{rest}") } else { stem.to_string() }
}

/// `ui.actionSet.save {name?, set?: index, path?}`: writes the action set named `name`, else the one
/// at `set` (an index into the panel's sets; default: the current set), with its actions to an
/// action set file at `path`, else a picked file named after the set (the web downloads it) →
/// `{path}`. The panel menu passes `name`: the set is addressed by its name, which survives the
/// wait for the file dialog. A set that [`read_set`] would refuse (larger than 8 MiB as a
/// file, more than 1,000 actions, an action with more than 10,000 steps) is not written, nor is a set
/// with a step that sets a folder preference to a folder ([`folder_pref_in`]), which names a folder
/// on this computer: the error names the action and the preference.
pub fn save_set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let sets = app.ui.actions.sets();
    let set: String = match (p.get("name"), p.get("set")) {
        (Some(name), _) if !name.is_null() => name.as_str().filter(|n| sets.contains(n)).ok_or("no such action set")?.to_string(),
        (_, None | Some(Value::Null)) => app.ui.action_set.clone().filter(|s| sets.contains(&s.as_str())).ok_or("no action set to save")?,
        (_, Some(v)) => v.as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| sets.get(i)).ok_or("no such action set")?.to_string(),
    };
    let actions = model::stored_actions(&app.ui.actions, &set);
    let folder = actions.iter().find_map(|a| a.steps.iter().find_map(|s| folder_pref_in(&s.command, &s.params)).map(|key| (&a.name, key)));
    if let Some((action, key)) = folder {
        return Err(format!("the action “{}” sets the folder preference {}; delete that step to save the set", action, key));
    }
    let text = write_set(&set, &actions);
    // What Save Actions… writes, Load Actions… reads back.
    read_set(&text, "")?;
    let name = format!("{}.{ACTIONS_EXT}", set_file_stem(&set));
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    crate::io::write_named(app, path, &name, text.into_bytes(), |app, path| {
        app.status(format!("Saved {path}"));
        Ok(json!({ "path": path }))
    })
}

/// Whether Load Actions… can ask for a file: not where there is no open dialog (the web).
pub(crate) fn can_load(app: &VectorcraftApp) -> bool {
    file_asks::can(app, PickKind::Open)
}

/// The bytes start with `/version`, as the action sets another app saves as text (`.aia`) do.
fn is_aia(bytes: &[u8]) -> bool {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes).trim_ascii_start().starts_with(b"/version")
}

/// `label` in any case, without a final `…` or `...`.
fn bare_label(label: &str) -> String {
    label.trim().trim_end_matches("...").trim_end_matches('…').trim().to_lowercase()
}

/// Whether a step of another app's action set may stand for command `id`. `command.batch` runs a
/// step without the app's handling of it, so a file or document command would close, revert or write
/// documents without asking, and a Paste would skip the view's center and the swatch conflict
/// question. Undo and Redo would act on the action's own undo step, and an add-on command with a
/// label a file also uses is not what the file recorded.
fn may_stand_for(id: &str) -> bool {
    !(id.starts_with("file.")
        || id.starts_with("document.")
        || id.starts_with("addon.")
        || id.starts_with("edit.paste")
        || matches!(id, "edit.undo" | "edit.redo"))
}

/// The name of a step of `actionSet.read`'s answer, and whether the step can stand for a command. A
/// step goes by its label. Only a step with a label and no recorded values can stand for a command;
/// a step recorded with values, a recorded menu item included, is left out. A step with no label is
/// listed by its name.
fn step_name(step: &Value) -> (String, bool) {
    let label = step.get("label").and_then(Value::as_str);
    let params = step.get("params").and_then(Value::as_u64).unwrap_or_default();
    match label {
        Some(label) => (label.to_string(), params == 0),
        None => (step.get("name").and_then(Value::as_str).unwrap_or_default().to_string(), false),
    }
}

/// The commands a step of another app's action set can stand for, by label, in English and in the
/// interface language ([`bare_label`]). A label stands for a command when no other menu command has
/// it, and the command is recordable, is run by a menu bar item with no params
/// ([`runs_with_no_values`](crate::shortcut_editor::runs_with_no_values)), has no `…` in its label,
/// sits directly in a top menu and is one [`may_stand_for`] allows. The
/// label of a submenu's item does not say which command it is: Select › Same › Opacity is "Opacity".
fn step_commands() -> std::collections::HashMap<String, Option<&'static str>> {
    let mut by_label = std::collections::HashMap::new();
    for c in vectorcraft_engine::command_specs().iter().filter(|c| !c.menu.is_empty()) {
        let can_stand =
            may_stand_for(c.id) && c.journal && c.menu.len() == 1 && !c.label.contains('…') && crate::shortcut_editor::runs_with_no_values(c);
        let mut labels = vec![bare_label(c.label), bare_label(crate::i18n::t_id(c.id, c.label))];
        labels.dedup();
        for label in labels.into_iter().filter(|l| !l.is_empty()) {
            by_label
                .entry(label)
                .and_modify(|id: &mut Option<&'static str>| *id = id.filter(|id| can_stand && *id == c.id))
                .or_insert(can_stand.then_some(c.id));
        }
    }
    by_label
}

/// The `/internalName` of a step of another app's action set that selects objects by their note: the
/// Set Selection step.
const SET_SELECTION: &str = "adobe_setSelection"; // brand-ok: the step's internal name in the file format

/// The `/key` of a Set Selection step's note (`text`), of its Whole Word option (`word`) and of its
/// Case Sensitive option (`case`): four characters read as a big-endian number.
const NOTE_KEY: i64 = 0x7465_7874;
const WHOLE_WORD_KEY: i64 = 0x776f_7264;
const CASE_SENSITIVE_KEY: i64 = 0x6361_7365;

/// The Select Object step ([`super::SELECT_OBJECT_STEP`]) a Set Selection step of `actionSet.read`'s
/// answer stands for. The step must have exactly three recorded values, all kept: the note, of type
/// `ustring` and UTF-8, and Whole Word and Case Sensitive, each of type `boolean` and 0 or 1. The note
/// is as [`super::select_object_params`] takes it (trimmed, not empty, at most 1,024 bytes). Anything
/// else stands for nothing.
fn select_object_step(step: &Value) -> Option<ActionStep> {
    let values = step.get("values").and_then(Value::as_array)?;
    if values.len() != 3 || step.get("params").and_then(Value::as_u64) != Some(3) {
        return None;
    }
    let value = |key: i64, kind: &str| {
        let mut found = values.iter().filter(|v| v.get("key").and_then(Value::as_i64) == Some(key));
        let v = found.next().filter(|v| v.get("type").and_then(Value::as_str) == Some(kind))?;
        if found.next().is_some() {
            return None;
        }
        v.get("value")
    };
    let flag = |key: i64| match value(key, "boolean")?.as_i64()? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    let note = value(NOTE_KEY, "ustring")?.as_str()?;
    let params = json!({"note": note, "caseSensitive": flag(CASE_SENSITIVE_KEY)?, "wholeWord": flag(WHOLE_WORD_KEY)?});
    let (note, case_sensitive, whole_word) = super::select_object_params(&params).ok()?;
    Some(ActionStep::new(super::SELECT_OBJECT_STEP, json!({"note": note, "caseSensitive": case_sensitive, "wholeWord": whole_word})))
}

/// The command a step of another app's action set stands for ([`step_commands`]).
#[cfg(test)]
pub(super) fn step_command(label: &str) -> Option<&'static str> {
    step_commands().get(&bare_label(label)).copied().flatten()
}

/// What Load Actions… made of an action set another app saved as text.
struct AiaLoad {
    name: String,
    actions: Vec<Action>,
    /// The first [`MAX_SKIPPED`] steps left out: `[{action, step}]`.
    skipped: Vec<Value>,
    /// How many steps were left out.
    left_out: usize,
}

/// The set of `read`, `actionSet.read`'s answer, named `name` when the file gives none. A step comes
/// in as the command it stands for ([`step_name`], [`step_commands`]), with no params, or a Set
/// Selection step as a Select Object step ([`select_object_step`]), and excluded from playback when
/// it is off in the file. An action with no name is called `Action N`.
fn aia_set(read: &Value, name: &str) -> AiaLoad {
    let (mut skipped, mut left_out) = (vec![], 0usize);
    let commands = step_commands();
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let actions = read
        .get("actions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let action = Some(text(a, "name")).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Action {}", i + 1));
            let (mut steps, mut excluded_steps) = (vec![], std::collections::BTreeSet::new());
            for s in a.get("steps").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
                let (name, can_stand) = step_name(s);
                let step = if s.get("name").and_then(Value::as_str) == Some(SET_SELECTION) {
                    select_object_step(s)
                } else {
                    commands.get(&bare_label(&name)).copied().flatten().filter(|_| can_stand).map(|id| ActionStep::new(id, json!({})))
                };
                let Some(step) = step else {
                    left_out += 1;
                    if skipped.len() < MAX_SKIPPED {
                        skipped.push(json!({ "action": action, "step": name }));
                    }
                    continue;
                };
                if s.get("on").and_then(Value::as_bool) == Some(false) {
                    excluded_steps.insert(steps.len());
                }
                steps.push(step);
            }
            Action { disabled: excluded_steps, ..Action::new(action, steps) }
        })
        .collect();
    let set = text(read, "set");
    AiaLoad { name: if set.trim().is_empty() { name.to_string() } else { set }, actions, skipped, left_out }
}

/// The steps left out of an action set another app saved as text: the first [`MAX_SKIPPED`] as
/// `[{action, step}]`, and how many there are. None for an action set file.
type LeftOut = Option<(Vec<Value>, usize)>;

/// A set a file holds, with its left-out steps.
struct FileRead {
    name: String,
    actions: Vec<Action>,
    aia: LeftOut,
}

/// The set of an action set another app saved as text, which `actionSet.read` reads with `params`,
/// with its left-out steps.
fn read_aia(app: &mut VectorcraftApp, params: Value) -> Result<FileRead, String> {
    // Text that starts `/version` and no number is not an action set either.
    let read = app.run("actionSet.read", params).map_err(|_| "not an action set file")?;
    // A set read only in part would play actions that differ from the file's.
    if read.get("truncated").and_then(Value::as_bool).unwrap_or(false) {
        return Err("the action set file can't be read in full".into());
    }
    let AiaLoad { name, actions, skipped, left_out } = aia_set(&read, "Actions");
    Ok(FileRead { name, actions, aia: Some((skipped, left_out)) })
}

/// The set of the file's text `data`.
fn read_data(app: &mut VectorcraftApp, data: &str) -> Result<FileRead, String> {
    // Checked before the copy: `read_set` checks it again.
    if data.len() > MAX_SET_FILE {
        return Err("the action set file is larger than 8 MiB".into());
    }
    if is_aia(data.as_bytes()) {
        return read_aia(app, json!({ "data": data }));
    }
    let (name, actions) = read_set(data, "Actions")?;
    Ok(FileRead { name, actions, aia: None })
}

/// The set of the file at `path`.
fn read_path(app: &mut VectorcraftApp, path: &str) -> Result<FileRead, String> {
    // One byte past the limit tells a larger file apart without reading all of it.
    let bytes = match (&app.services.read_head, &app.services.read) {
        (Some(read_head), _) => read_head(path, MAX_SET_FILE + 1)?,
        (None, Some(read)) => read(path)?,
        (None, None) => return Err("no file reader".into()),
    };
    read_bytes(app, path, &bytes)
}

/// A desktop or browser file's contents, with its name used only as the unnamed set's fallback.
fn read_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<FileRead, String> {
    if bytes.len() > MAX_SET_FILE {
        return Err("the action set file is larger than 8 MiB".into());
    }
    if is_aia(bytes) {
        return read_aia(app, json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) }));
    }
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = std::str::from_utf8(bytes).map_err(|_| "not an action set file")?;
    let (name, actions) = read_set(text, if stem.is_empty() { "Actions" } else { &stem })?;
    Ok(FileRead { name, actions, aia: None })
}

/// Reads the set of the action set file at `p`'s `path` (else a picked one), or of the file's text
/// `data`, as [`load_set`] reads it, and hands it to `then`, at once or when the file dialog
/// answers. Nothing in `app` changes but its status before `then` runs, and an error leaves the
/// sets as they are.
fn read_file(
    app: &mut VectorcraftApp,
    p: &Value,
    then: impl FnOnce(&mut VectorcraftApp, FileRead) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    match p.get("data") {
        Some(Value::String(data)) => {
            let read = read_data(app, data)?;
            return then(app, read);
        }
        Some(_) => return Err("not an action set file".into()),
        None => {}
    }
    if let Some(path) = p.get("path").and_then(Value::as_str) {
        let read = read_path(app, path)?;
        return then(app, read);
    }
    let exts = [&[ACTIONS_EXT], vectorcraft_engine::cmd::actionset::EXTS].concat();
    let request = PickRequest::open("actions").filter("Action sets", &exts);
    Ok(file_asks::ask(app, request, Owner::Free, move |app, picked| {
        let read = match picked {
            Picked::Contents(files) => {
                let (name, bytes) = files.into_iter().next().ok_or("cancelled")?;
                read_bytes(app, &name, &bytes)?
            }
            other => {
                let path = other.first_path().ok_or("cancelled")?;
                read_path(app, &path.to_string_lossy())?
            }
        };
        then(app, read)
    }))
}

/// The file `name` with `actions` as an [`ActionsFile`]: the actions go in the set, which is kept
/// by name when it has none.
fn file_of(name: &str, actions: Vec<Action>) -> ActionsFile {
    let sets = if actions.is_empty() { vec![name.to_string()] } else { vec![] };
    let actions = actions.into_iter().map(|a| Action { set: Some(name.to_string()), ..a }).collect();
    ActionsFile::new(actions, sets)
}

/// The answer of [`load_set`] and [`replace_set`] for the set `name` with `actions`, which goes at
/// index `at`, with its left-out steps `aia` ([`read_file`]); the status bar names what was left out
/// or kept.
fn loaded(app: &mut VectorcraftApp, name: &str, actions: &[Action], at: usize, aia: LeftOut) -> Value {
    let count = actions.len();
    if let Some((skipped, left_out)) = aia {
        if left_out > 0 {
            app.status(crate::i18n::msg_n(
                left_out as u64,
                "Loaded “{name}”: {n} step was left out",
                "Loaded “{name}”: {n} steps were left out",
                &[("name", name)],
            ));
        }
        return json!({ "set": at, "name": name, "actions": count, "skipped": skipped, "leftOut": left_out });
    }
    let (unknown, unknown_count, first) = unknown_commands(actions);
    let mut answer = json!({ "set": at, "name": name, "actions": count });
    if let Some((action, step, command)) = first {
        app.status(crate::i18n::msg_n(
            unknown_count as u64,
            "Loaded “{name}”: {n} step has an unknown command and was kept: “{action}”, step {step}, `{command}`",
            "Loaded “{name}”: {n} steps have unknown commands and were kept; the first: “{action}”, step {step}, `{command}`",
            &[
                ("name", &super::shown_text(name)),
                ("action", &super::shown_text(&action)),
                ("step", &step),
                ("command", &super::shown_text(&command)),
            ],
        ));
        answer["unknownCommands"] = json!(unknown);
        answer["unknownCount"] = json!(unknown_count);
    }
    answer
}

/// `ui.actionSet.load {path? | data?}`: reads the action set file at `path` (else a picked one), or
/// the file's text `data`, and adds its set after the others, with its actions. A set with the
/// same name as one in the panel gets the next free number (`Print 2`). Loaded actions keep their
/// function keys. The current set and the other sets stay as they are →
/// `{set: index, name, actions: n}`. Where the host can read part of a file (the desktop), a file
/// larger than 8 MiB is refused after its first 8 MiB and one byte are read. A set that Save
/// Actions… would write as a file Load Actions… refuses (larger than 8 MiB, as a file with less
/// white space or a name that gets a number can give) is refused too, and no set is added.
///
/// A step whose command is not known now ([`super::KnownCommands`]), excluded or not, and a command
/// of a `command.batch` step that is not, still loads as it is. The answer then also lists them,
/// and the status bar names the first and how many there are → `{…, unknownCommands: [{action,
/// actionIndex, step, stepIndex, batchStep?, command}] (at most 1,000), unknownCount: n}`.
///
/// An action set another app saved as text (`.aia`, read with `actionSet.read`) comes in with its
/// actions and the steps that stand for a VectorCraft command ([`step_name`], [`step_commands`]),
/// with no params, and its Set Selection steps as Select Object steps ([`select_object_step`]); a
/// step that is off in the file is excluded from playback. A set with no name is
/// called after the file, else `Actions`. The other steps are left out and listed, and the status
/// bar says how many → `{set, name, actions, skipped: [{action, step}] (at most 1,000), leftOut: n}`.
/// Such a file that `actionSet.read` marks `truncated` is refused, and so is one whose set Save
/// Actions… would write larger than 8 MiB; then no set is added.
pub fn load_set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    read_file(app, p, |app, read| {
        let name = model::free_set_name(&app.ui.actions, &read.name);
        // A number added to the name can make a file at the limit larger than it, and the set of an
        // `.aia` file can save larger than the file read.
        model::edit_keeps_set_loadable(app, &name, &[], &read.actions)?;
        let answer = loaded(app, &name, &read.actions, app.ui.actions.sets().len(), read.aia);
        app.ui.actions.merge_file(file_of(&name, read.actions), false, |_, _| {});
        Ok(answer)
    })
}

/// `ui.actionSet.replace {path? | data?}`: Replace Actions…. Reads an action set file as
/// [`load_set`] does, from `path`, a picked file or the file's text `data`, with the same formats,
/// limits and errors, and replaces every set in the panel with the file's set, which keeps its
/// name. The set becomes current and selected, and the panel clears what named the old sets as
/// [`super::reset`] does. When the panel already holds only a set that saves alike, shown that
/// way, nothing changes. Omitted params (`null`) count as `{}`. Params
/// of another type, another key or a `path` or `data` that is not a string is an error, and so is a replace
/// while an action records or plays (also while it waits at a stop). On any error, a canceled
/// pick included, nothing changes. No undo step is recorded → `{changed, set: 0, name, actions}`,
/// with what [`load_set`] reports of left-out steps and unknown commands.
pub fn replace_set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    match p {
        Value::Null => {}
        Value::Object(m) => {
            if let Some(key) = m.keys().find(|k| !matches!(k.as_str(), "path" | "data")) {
                let keys = "path, data";
                return Err(format!("unknown parameter `{key}`; the command takes: {keys}"));
            }
            if let Some(key) = ["path", "data"].into_iter().find(|k| m.get(*k).is_some_and(|v| !v.is_string())) {
                return Err(format!("`{key}` must be a string"));
            }
        }
        got => return Err(format!("the params must be an object, not {got}")),
    }
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    if app.ui.action_playing.is_some() {
        return Err("an action is already playing".into());
    }
    read_file(app, p, |app, read| {
        model::edit_keeps_set_loadable(app, &read.name, &[], &read.actions)?;
        let mut answer = loaded(app, &read.name, &read.actions, 0, read.aia);
        let before = (app.ui.actions.actions().to_vec(), app.ui.actions.sets().into_iter().map(str::to_string).collect::<Vec<_>>());
        app.ui.actions.replace_all(file_of(&read.name, read.actions), |_, _| {});
        let after = (app.ui.actions.actions().to_vec(), app.ui.actions.sets().into_iter().map(str::to_string).collect::<Vec<_>>());
        let changed = before != after;
        if changed {
            super::edits::replace_sets(app); // PLAN: assumed menus-w18 `pub(super) fn replace_sets(&mut VectorcraftApp)`: the panel follow-up (selection, current set, dialogs)
        }
        answer["changed"] = json!(changed);
        Ok(answer)
    })
}

/// The steps of `actions` whose command is not known now ([`super::KnownCommands`]), excluded steps
/// included: the first [`MAX_SKIPPED`] as `[{action, actionIndex, step, stepIndex, batchStep?,
/// command}]` (`step` and `batchStep` count from 1, the indexes from 0), how many there are, and the
/// first one's action name, place ([`super::step_place`]) and command.
fn unknown_commands(actions: &[Action]) -> (Vec<Value>, usize, Option<(String, String, String)>) {
    let known = super::KnownCommands::now();
    let (mut listed, mut count, mut first) = (vec![], 0usize, None);
    for (a, action) in actions.iter().enumerate() {
        for (step, in_batch, command) in super::unknown_commands(action, &known, false) {
            count += 1;
            first.get_or_insert_with(|| (action.name.clone(), super::step_place(step, in_batch), command.to_string()));
            if listed.len() < MAX_SKIPPED {
                let mut entry = json!({ "action": action.name, "actionIndex": a, "step": step + 1, "stepIndex": step, "command": command });
                if let Some(j) = in_batch {
                    entry["batchStep"] = json!(j + 1);
                }
                listed.push(entry);
            }
        }
    }
    (listed, count, first)
}
