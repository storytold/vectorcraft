//! Action set files: the Actions panel menu's Save Actions… and Load Actions….
//!
//! An action set file (`.vcactions`) is JSON: `{"format": "vcactions", "version": 1 to 6, "name",
//! "actions": [{"name", "steps": [{"command", "params", "excluded"?}], "key"?, "color"?}]}`. Each step is a
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
//! earlier version 1 to 5 files still work.
//! Load Actions… also reads the action sets another app saves as text (`.aia`, through
//! `actionSet.read`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Action, ActionSet};
use crate::VectorcraftApp;

/// The command that saves an action set to a file ([`save_set`]).
pub const SAVE_SET: &str = "ui.actionSet.save";

/// The command that loads an action set from a file ([`load_set`]).
pub const LOAD_SET: &str = "ui.actionSet.load";

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
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::deserialize_key")]
    key: Option<super::FunctionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::deserialize_color")]
    color: Option<String>,
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

fn is_false(value: &bool) -> bool {
    !value
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

/// `set` as the text of an action set file.
pub fn write_set(set: &ActionSet) -> String {
    let actions = set
        .actions
        .iter()
        .map(|a| FileAction {
            name: a.name.clone(),
            steps: a
                .steps
                .iter()
                .enumerate()
                .map(|(i, (command, params))| FileStep { command: command.clone(), params: params.clone(), excluded: a.excluded_steps.contains(&i) })
                .collect(),
            key: a.key,
            color: a.color.clone(),
        })
        .collect();
    let steps = || set.actions.iter().flat_map(|a| &a.steps);
    let has_select_object = steps().any(|(command, params)| !select_objects_in(command, params).is_empty());
    let has_select_path = steps().any(|(command, params)| !select_paths_in(command, params).is_empty());
    let has_stop = steps().any(|(command, params)| !stops_in(command, params).is_empty());
    let has_ui_step = steps().any(|(command, params)| !ui_steps_in(command, params).is_empty());
    let version = match (has_select_path, has_select_object, has_stop, set.actions.iter().any(|a| !a.excluded_steps.is_empty())) {
        _ if has_ui_step => 6,
        (true, ..) => 5,
        (false, true, ..) => 4,
        (false, false, true, _) => 3,
        (false, false, false, true) => 2,
        (false, false, false, false) => 1,
    };
    let file = SetFile { format: ACTIONS_EXT.into(), version, name: set.name.clone(), actions };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// The action set in the text of an action set file; a set without a name is called `name`.
pub fn read_set(text: &str, name: &str) -> Result<ActionSet, String> {
    if text.len() > MAX_SET_FILE {
        return Err("the action set file is larger than 8 MiB".into());
    }
    let f: SetFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not an action set file ({e})"))?;
    if f.format != ACTIONS_EXT {
        return Err("not an action set file".into());
    }
    if !matches!(f.version, 1..=6) {
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
            let excluded_steps = a.steps.iter().enumerate().filter_map(|(i, step)| step.excluded.then_some(i)).collect();
            Action { key: a.key, color: a.color, excluded_steps, ..Action::new(a.name, a.steps.into_iter().map(|s| (s.command, s.params)).collect()) }
        })
        .collect();
    let set_name = if f.name.trim().is_empty() { name.to_string() } else { f.name };
    Ok(ActionSet::new(set_name, actions))
}

/// `name`, else `name 2`, `name 3` … : the first that no set in `sets` has.
fn free_set_name(sets: &[ActionSet], name: &str) -> String {
    let taken = |n: &str| sets.iter().any(|s| s.name == n);
    if !taken(name) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} {n}")).find(|n| !taken(n)).unwrap_or_else(|| name.to_string())
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

/// `ui.actionSet.save {set?: index, id?, path?}`: writes the action set at `set` (default: the
/// current set) with its actions to an action set file at `path`, else a picked file named after
/// the set (the web downloads it) → `{path}`. `id` is the panel's own set id ([`ActionSet::id`]),
/// which the panel menu passes: the set with this id is saved wherever it is when the file dialog
/// answers, and `set` is not read. A set that [`read_set`] would refuse (larger than 8 MiB as a
/// file, more than 1,000 actions, an action with more than 10,000 steps) is not written.
pub fn save_set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let len = app.ui.action_sets.len();
    let set = match (p.get("id"), p.get("set")) {
        (Some(id), _) if !id.is_null() => {
            id.as_u64().and_then(|id| app.ui.action_sets.iter().position(|s| s.id == id)).ok_or("no such action set")?
        }
        (_, None | Some(Value::Null)) => app.ui.action_set.filter(|&s| s < len).ok_or("no action set to save")?,
        (_, Some(v)) => v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < len).ok_or("no such action set")?,
    };
    let Some(s) = app.ui.action_sets.get(set) else { return Err("no such action set".into()) };
    let text = write_set(s);
    // What Save Actions… writes, Load Actions… reads back.
    read_set(&text, "")?;
    let name = format!("{}.{ACTIONS_EXT}", set_file_stem(&s.name));
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    let path = crate::io::write_named(app, path, &name, text.as_bytes())?;
    app.status(format!("Saved {path}"));
    Ok(json!({ "path": path }))
}

/// Whether Load Actions… can ask for a file: not where there is no open dialog (the web).
pub(crate) fn can_load(app: &VectorcraftApp) -> bool {
    crate::picks::can(app, &crate::picks::PickRequest::Open(crate::FilePick::default()))
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

/// The command a step of another app's action set stands for ([`step_commands`]).
#[cfg(test)]
pub(super) fn step_command(label: &str) -> Option<&'static str> {
    step_commands().get(&bare_label(label)).copied().flatten()
}

/// What Load Actions… made of an action set another app saved as text.
struct AiaLoad {
    set: ActionSet,
    /// The first [`MAX_SKIPPED`] steps left out: `[{action, step}]`.
    skipped: Vec<Value>,
    /// How many steps were left out.
    left_out: usize,
}

/// The set of `read`, `actionSet.read`'s answer, named `name` when the file gives none. A step comes
/// in as the command it stands for ([`step_name`], [`step_commands`]), with no params, and excluded
/// from playback when it is off in the file. An action with no name is called `Action N`.
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
                let Some(id) = commands.get(&bare_label(&name)).copied().flatten().filter(|_| can_stand) else {
                    left_out += 1;
                    if skipped.len() < MAX_SKIPPED {
                        skipped.push(json!({ "action": action, "step": name }));
                    }
                    continue;
                };
                if s.get("on").and_then(Value::as_bool) == Some(false) {
                    excluded_steps.insert(steps.len());
                }
                steps.push((id.to_string(), json!({})));
            }
            Action { excluded_steps, ..Action::new(action, steps) }
        })
        .collect();
    let set = text(read, "set");
    AiaLoad { set: ActionSet::new(if set.trim().is_empty() { name.to_string() } else { set }, actions), skipped, left_out }
}

/// [`load_set`] of an action set another app saved as text, which `actionSet.read` reads with
/// `params`.
fn load_aia(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    // Text that starts `/version` and no number is not an action set either.
    let read = app.run("actionSet.read", params).map_err(|_| "not an action set file")?;
    // A set read only in part would play actions that differ from the file's.
    if read.get("truncated").and_then(Value::as_bool).unwrap_or(false) {
        return Err("the action set file can't be read in full".into());
    }
    let AiaLoad { mut set, skipped, left_out } = aia_set(&read, "Actions");
    set.name = free_set_name(&app.ui.action_sets, &set.name);
    // The set must save as a file that Load Actions… reads, which can be larger than the file read.
    super::edit_keeps_set_loadable(app, None, &set)?;
    let (name, actions) = (set.name.clone(), set.actions.len());
    app.ui.action_sets.push(set);
    if left_out > 0 {
        app.status(crate::i18n::msg_n(
            left_out as u64,
            "Loaded “{name}”: {n} step was left out",
            "Loaded “{name}”: {n} steps were left out",
            &[("name", &name)],
        ));
    }
    Ok(json!({
        "set": app.ui.action_sets.len() - 1, "name": name, "actions": actions,
        "skipped": skipped, "leftOut": left_out,
    }))
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
/// with no params; a step that is off in the file is excluded from playback. A set with no name is
/// called after the file, else `Actions`. The other steps are left out and listed, and the status
/// bar says how many → `{set, name, actions, skipped: [{action, step}] (at most 1,000), leftOut: n}`.
/// Such a file that `actionSet.read` marks `truncated` is refused, and so is one whose set Save
/// Actions… would write larger than 8 MiB; then no set is added.
pub fn load_set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (text, stem) = match p.get("data") {
        Some(Value::String(data)) => {
            // Checked before the copy: `read_set` checks it again.
            if data.len() > MAX_SET_FILE {
                return Err("the action set file is larger than 8 MiB".into());
            }
            if is_aia(data.as_bytes()) {
                return load_aia(app, json!({ "data": data }));
            }
            (data.clone(), String::new())
        }
        Some(_) => return Err("not an action set file".into()),
        None => {
            let path = match p.get("path").and_then(Value::as_str) {
                Some(path) => path.to_string(),
                None => {
                    let exts = [&[ACTIONS_EXT], vectorcraft_engine::cmd::actionset::EXTS].concat();
                    let pick = crate::FilePick { filters: vec![crate::FilePick::filter("Action sets", &exts)], ..Default::default() };
                    crate::picks::open(app, &pick).ok_or("cancelled")?
                }
            };
            // One byte past the limit tells a larger file apart without reading all of it.
            let bytes = match (&app.services.read_head, &app.services.read) {
                (Some(read_head), _) => read_head(&path, MAX_SET_FILE + 1)?,
                (None, Some(read)) => read(&path)?,
                (None, None) => return Err("no file reader".into()),
            };
            if bytes.len() > MAX_SET_FILE {
                return Err("the action set file is larger than 8 MiB".into());
            }
            if is_aia(&bytes) {
                return load_aia(app, json!({ "name": path, "dataBase64": vectorcraft_format::base64_encode(&bytes) }));
            }
            let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (String::from_utf8(bytes).map_err(|_| "not an action set file")?, stem)
        }
    };
    let mut set = read_set(&text, if stem.is_empty() { "Actions" } else { &stem })?;
    set.name = free_set_name(&app.ui.action_sets, &set.name);
    // A number added to the name can make a file at the limit larger than it.
    super::edit_keeps_set_loadable(app, None, &set)?;
    let (unknown, unknown_count, first) = unknown_commands(&set);
    let (name, actions) = (set.name.clone(), set.actions.len());
    app.ui.action_sets.push(set);
    let mut answer = json!({ "set": app.ui.action_sets.len() - 1, "name": name, "actions": actions });
    if let Some((action, step, command)) = first {
        app.status(crate::i18n::msg_n(
            unknown_count as u64,
            "Loaded “{name}”: {n} step has an unknown command and was kept: “{action}”, step {step}, `{command}`",
            "Loaded “{name}”: {n} steps have unknown commands and were kept; the first: “{action}”, step {step}, `{command}`",
            &[
                ("name", &super::shown_text(&name)),
                ("action", &super::shown_text(&action)),
                ("step", &step),
                ("command", &super::shown_text(&command)),
            ],
        ));
        answer["unknownCommands"] = json!(unknown);
        answer["unknownCount"] = json!(unknown_count);
    }
    Ok(answer)
}

/// The steps of `set` whose command is not known now ([`super::KnownCommands`]), excluded steps
/// included: the first [`MAX_SKIPPED`] as `[{action, actionIndex, step, stepIndex, batchStep?,
/// command}]` (`step` and `batchStep` count from 1, the indexes from 0), how many there are, and the
/// first one's action name, place ([`super::step_place`]) and command.
fn unknown_commands(set: &ActionSet) -> (Vec<Value>, usize, Option<(String, String, String)>) {
    let known = super::KnownCommands::now();
    let (mut listed, mut count, mut first) = (vec![], 0usize, None);
    for (a, action) in set.actions.iter().enumerate() {
        for (step, in_batch, command) in action.unknown_commands(&known, false) {
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
