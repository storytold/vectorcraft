//! Action set files: the Actions panel menu's Save Actions… and Load Actions….
//!
//! An action set file (`.vcactions`) is JSON: `{"format": "vcactions", "version": 1, "name",
//! "actions": [{"name", "steps": [{"command", "params"}]}]}`. Each step is a command id and its
//! params, as `command.batch` takes them.

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
const MAX_SET_ACTIONS: usize = 1000;

/// The most steps [`read_set`] reads in one action.
const MAX_ACTION_STEPS: usize = 10_000;

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
}

/// A step of an action in an action set file: a command id and its params, as `command.batch`
/// takes them.
#[derive(Serialize, Deserialize)]
struct FileStep {
    command: String,
    #[serde(default)]
    params: Value,
}

/// `set` as the text of an action set file.
pub fn write_set(set: &ActionSet) -> String {
    let actions = set
        .actions
        .iter()
        .map(|a| FileAction {
            name: a.name.clone(),
            steps: a.steps.iter().map(|(command, params)| FileStep { command: command.clone(), params: params.clone() }).collect(),
        })
        .collect();
    let file = SetFile { format: ACTIONS_EXT.into(), version: 1, name: set.name.clone(), actions };
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
    if f.version != 1 {
        return Err("the action set file needs a later version of VectorCraft".into());
    }
    if f.actions.len() > MAX_SET_ACTIONS {
        return Err("the action set file holds more than 1,000 actions".into());
    }
    if f.actions.iter().any(|a| a.steps.len() > MAX_ACTION_STEPS) {
        return Err("an action in the action set file has more than 10,000 steps".into());
    }
    let actions =
        f.actions.into_iter().map(|a| Action { name: a.name, steps: a.steps.into_iter().map(|s| (s.command, s.params)).collect() }).collect();
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

/// `ui.actionSet.load {path? | data?}`: reads the action set file at `path` (else a picked one), or
/// the file's text `data`, and adds its set after the others, with its actions. A set with the
/// same name as one in the panel gets the next free number (`Print 2`). The current set and the
/// other sets stay as they are → `{set: index, name, actions: n}`.
pub fn load_set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (text, stem) = match p.get("data") {
        Some(Value::String(data)) => {
            // Checked before the copy: `read_set` checks it again.
            if data.len() > MAX_SET_FILE {
                return Err("the action set file is larger than 8 MiB".into());
            }
            (data.clone(), String::new())
        }
        Some(_) => return Err("not an action set file".into()),
        None => {
            let path = match p.get("path").and_then(Value::as_str) {
                Some(path) => path.to_string(),
                None => {
                    let pick = crate::FilePick { filters: vec![crate::FilePick::filter("Action sets", &[ACTIONS_EXT])], ..Default::default() };
                    crate::picks::open(app, &pick).ok_or("cancelled")?
                }
            };
            let bytes = app.services.read.as_ref().ok_or("no file reader")?(&path)?;
            if bytes.len() > MAX_SET_FILE {
                return Err("the action set file is larger than 8 MiB".into());
            }
            let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (String::from_utf8(bytes).map_err(|_| "not an action set file")?, stem)
        }
    };
    let mut set = read_set(&text, if stem.is_empty() { "Actions" } else { &stem })?;
    set.name = free_set_name(&app.ui.action_sets, &set.name);
    let (name, actions) = (set.name.clone(), set.actions.len());
    app.ui.action_sets.push(set);
    Ok(json!({ "set": app.ui.action_sets.len() - 1, "name": name, "actions": actions }))
}
