//! `app.doScript(action, set, dialogs?)`: plays an action of the Actions panel inside the script's
//! run, as the VBScript reference's `DoScript` method of the Application object describes it
//! (<https://web.archive.org/web/20170621180804/https://wwwimages.adobe.com/content/dam/Adobe/en/devnet/illustrator/sdk/CC2015_3/Illustrator%20VBScript%20Reference.pdf>).
//!
//! The actions a run can play come with its request (`script.run`'s `actions`), each with the steps
//! that playback runs: the included steps, those of a recorded `command.batch` one by one. In the app
//! they are the Actions panel's.
//!
//! VectorCraft's own rules, which the reference does not state:
//! - The action and its set are matched by their exact names; a name that matches no action, or
//!   more than one, throws before any step runs.
//! - The call returns when every step has run, or when the run is asked to stop. Each step runs as `vc.run` runs a command: it joins
//!   the run's transaction and its one undo step for each changed document, it is counted as a host
//!   call, it reads and writes files only where the script may, and a step that fails is put back
//!   and throws an error that names the step and the action, while the steps before it stay done.
//! - A step a script may not run ([`super::refused_to_scripts`]), a step whose command the engine
//!   does not have (a View menu step, or one that plays an action), and a stop throw before any step
//!   runs, as does such a command among those an `action.batch` step plays. An action cannot play
//!   another action, so it cannot play itself.
//! - `dialogs` is `true` or `false` (default `false`); VectorCraft's steps open no dialogs, so it
//!   changes nothing.

use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{Data, HostError, HostMember};

use super::{Host, Link, command, current, refused_to_scripts};

/// The most actions a run's request may carry.
pub const MAX_ACTIONS: usize = 4096;
/// The most steps one action may have, after a `command.batch` step gives its own steps.
pub const MAX_ACTION_STEPS: usize = 10_000;
/// The most steps the actions of one request may have in all.
pub const MAX_ALL_STEPS: usize = 100_000;
/// The most bytes of the request's actions, as JSON.
pub const MAX_ACTIONS_BYTES: usize = 16 << 20;
/// The longest name of an action or a set, in bytes.
pub const MAX_NAME_BYTES: usize = 1024;
/// The longest command id of a step, in bytes.
const MAX_ID_BYTES: usize = 256;
/// The command id of a stop, which waits for the person in the Actions panel.
pub const STOP_STEP: &str = "action.stop";

/// An action a run can play: its set's name, its name and the steps playback runs.
#[derive(Clone, Debug)]
pub struct Playable {
    set: String,
    name: String,
    steps: Vec<Step>,
}

/// A step of a [`Playable`]: its place in the action as errors give it (1 for the first; a step of a
/// `command.batch` step has the batch's place and its own, as in `3.2`), its command, its params and
/// their size as JSON, and for a step of a `command.batch` step the `artboard` that batch names.
#[derive(Clone, Debug)]
struct Step {
    place: String,
    id: String,
    params: Value,
    bytes: u64,
    artboard: Option<u64>,
}

/// `script.run`'s `actions`: `[{set, name, steps: [{command, params?, step?}]}]`, where `step` is the
/// step's place in the action (1 for the first; default its place in the list), within the limits
/// above. A `command.batch` step gives its own steps, each at the batch's place and its own.
pub fn parse(v: Option<&Value>) -> Result<Vec<Playable>, String> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let Value::Array(list) = v else { return Err("`actions` must be a list".into()) };
    if list.len() > MAX_ACTIONS {
        return Err(format!("`actions` holds more than {MAX_ACTIONS} actions"));
    }
    if let Some(place) = super::too_deep(v) {
        return Err(format!("`actions{place}` is nested more than 64 levels deep"));
    }
    if serde_json::to_vec(v).map_or(true, |b| b.len() > MAX_ACTIONS_BYTES) {
        return Err("`actions` is larger than 16 MiB".into());
    }
    let mut all = 0usize;
    let mut out = Vec::with_capacity(list.len());
    for (i, a) in list.iter().enumerate() {
        let at = format!("`actions[{i}]`");
        let Value::Object(a) = a else { return Err(format!("{at} must be an object")) };
        only_keys(a, &["set", "name", "steps"], &at)?;
        let set = name_of(a, "set", &at)?;
        let name = name_of(a, "name", &at)?;
        let Some(Value::Array(given)) = a.get("steps") else { return Err(format!("{at}.steps must be a list")) };
        let mut steps = Vec::new();
        for (j, s) in given.iter().enumerate() {
            let at = format!("{at}.steps[{j}]");
            let Value::Object(s) = s else { return Err(format!("{at} must be an object")) };
            only_keys(s, &["command", "params", "step"], &at)?;
            let place = match s.get("step") {
                None => (j + 1).to_string(),
                Some(n) => match n.as_u64().filter(|n| (1..=1_000_000).contains(n)) {
                    Some(n) => n.to_string(),
                    None => return Err(format!("{at}.step must be a whole number from 1 to 1000000")),
                },
            };
            let (id, params) = command_of(s, &at)?;
            match params.get("commands") {
                Some(Value::Array(inner)) if id == "command.batch" => {
                    // The batch's `artboard`, for its steps that act on the active artboard, as
                    // `command.batch` reads it.
                    let artboard = params.get("artboard").and_then(Value::as_u64);
                    for (k, c) in inner.iter().enumerate() {
                        let at = format!("{at}.params.commands[{k}]");
                        let Value::Object(c) = c else { return Err(format!("{at} must be an object")) };
                        let (id, params) = command_of(c, &at)?;
                        if id == "command.batch" {
                            return Err(format!("{at} is a command.batch inside a command.batch"));
                        }
                        steps.push(step(format!("{place}.{}", k + 1), id, params, artboard));
                    }
                }
                _ if id == "command.batch" => return Err(format!("{at}.params.commands must be a list")),
                _ => steps.push(step(place, id, params, None)),
            }
            if steps.len() > MAX_ACTION_STEPS {
                return Err(format!("{at}: the action has more than {MAX_ACTION_STEPS} steps"));
            }
        }
        all += steps.len();
        if all > MAX_ALL_STEPS {
            return Err(format!("`actions` has more than {MAX_ALL_STEPS} steps in all"));
        }
        out.push(Playable { set, name, steps });
    }
    Ok(out)
}

impl Playable {
    /// The steps the action runs.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the action runs no step.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

fn only_keys(o: &Map<String, Value>, keys: &[&str], at: &str) -> Result<(), String> {
    match o.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("{at} has the unknown key `{}`", shown(k))),
        None => Ok(()),
    }
}

fn name_of(o: &Map<String, Value>, key: &str, at: &str) -> Result<String, String> {
    match o.get(key) {
        Some(Value::String(s)) if s.len() <= MAX_NAME_BYTES => Ok(s.clone()),
        Some(Value::String(_)) => Err(format!("{at}.{key} is longer than {MAX_NAME_BYTES} bytes")),
        _ => Err(format!("{at}.{key} must be a string")),
    }
}

/// A step's `command` and `params` (default `{}`).
fn command_of(o: &Map<String, Value>, at: &str) -> Result<(String, Value), String> {
    let id = match o.get("command") {
        Some(Value::String(id)) if !id.is_empty() && id.len() <= MAX_ID_BYTES => id.clone(),
        _ => return Err(format!("{at}.command must be a command id")),
    };
    let params = match o.get("params") {
        None => json!({}),
        Some(p @ Value::Object(_)) => p.clone(),
        Some(_) => return Err(format!("{at}.params must be an object")),
    };
    Ok((id, params))
}

fn step(place: String, id: String, params: Value, artboard: Option<u64>) -> Step {
    let bytes = serde_json::to_vec(&params).map_or(0, |b| b.len() as u64);
    Step { place, id, params, bytes, artboard }
}

/// `text` as an error shows it: at most 80 characters, with … after a cut, and control characters
/// as U+FFFD.
fn shown(text: &str) -> String {
    const MAX: usize = 80;
    let mut out: String = text.chars().take(MAX).map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect();
    if text.chars().nth(MAX).is_some() {
        out.push('…');
    }
    out
}

/// The members of `app` this module adds.
pub(super) fn app_members(link: &Link) -> Vec<super::dom::Described> {
    let l = link.clone();
    let play = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16 + a.iter().map(super::data_size).sum::<u64>())?;
        do_script(&h, &a)
    });
    let info = super::dom::returns("undefined", &[("action", "string"), ("set", "string"), ("dialogs", "boolean")]);
    let mut members = vec![("doScript".to_string(), HostMember::Method(play), info)];
    members.extend(["loadAction", "unloadAction"].map(|name| super::dom::later(link, "Application", name)));
    members
}

fn arg(a: &[Data], i: usize) -> &Data {
    a.get(i).unwrap_or(&Data::Undefined)
}

/// `app.doScript(action, set, dialogs?)`, its arguments after `this`.
fn do_script(h: &Host, a: &[Data]) -> std::result::Result<Data, HostError> {
    if a.len() > 4 {
        return Err(HostError::Type("doScript takes an action's name, its set's name and dialogs".into()));
    }
    let (Data::String(action), Data::String(set)) = (arg(a, 1), arg(a, 2)) else {
        return Err(HostError::Type("doScript needs the action's name and its set's name, as strings".into()));
    };
    if action.is_empty() || set.is_empty() {
        return Err(HostError::Error("doScript needs the action's name and its set's name".into()));
    }
    if action.len() > MAX_NAME_BYTES || set.len() > MAX_NAME_BYTES {
        return Err(HostError::Range(format!("an action's or a set's name is at most {MAX_NAME_BYTES} bytes")));
    }
    if !matches!(arg(a, 3), Data::Undefined | Data::Bool(_)) {
        return Err(HostError::Type("doScript's dialogs must be true or false".into()));
    }
    let found: Vec<&Playable> = h.actions.iter().filter(|p| p.set == *set && p.name == *action).take(2).collect();
    let p = match found.as_slice() {
        [p] => *p,
        [] if !h.actions.iter().any(|p| p.set == *set) => return Err(HostError::Error(format!("there is no action set “{}”", shown(set)))),
        [] => return Err(HostError::Error(format!("the set “{}” has no action “{}”", shown(set), shown(action)))),
        _ => return Err(HostError::Error(format!("the set “{}” has more than one action “{}”", shown(set), shown(action)))),
    };
    if p.steps.is_empty() {
        return Err(HostError::Error(format!("the action “{}” has no included steps", shown(&p.name))));
    }
    // Every step is checked before the first runs, so a step that cannot run leaves the documents as
    // they were.
    for s in &p.steps {
        if let Some((place, id)) = refused_step(s) {
            let name = shown(&p.name);
            return Err(HostError::Error(if id == STOP_STEP {
                format!("step {place} of “{name}” is a stop, which a script cannot wait at")
            } else {
                format!("step {place} of “{name}” runs `{}`, which a script cannot run", shown(&id))
            }));
        }
    }
    for s in &p.steps {
        // A run asked to stop runs no further step; the realm ends it at its next instruction and
        // puts the documents back.
        if h.session.borrow().script_stop.is_set() {
            break;
        }
        h.charge_call(16 + s.id.len() as u64 + s.bytes)?;
        // Saved steps run as they run in the Actions panel: keys their commands no longer read are
        // passed over.
        let start = Start { active: &h.active_artboards, picks: h.artboard_picks.as_ref(), batch: s.artboard };
        let params = on_active_artboard(&h.session.borrow(), &start, &s.id, &s.params);
        let outer = h.session.borrow_mut().set_replaying(true);
        let r = command(h, &s.id, &params);
        h.session.borrow_mut().set_replaying(outer);
        if let Err(e) = r {
            let named = |m: String| format!("action step {} of “{}” (`{}`) failed: {m}", s.place, shown(&p.name), shown(&s.id));
            return Err(match e {
                HostError::Error(m) | HostError::Type(m) | HostError::Range(m) | HostError::Syntax(m) => HostError::Error(named(m)),
                other => other,
            });
        }
    }
    Ok(Data::Undefined)
}

/// The place and command of the first command step `s` would run that a script may not run: the
/// step itself, or one of the `commands` an `action.batch` step plays on each file (placed as the
/// batch's place and its own, as in `3.2`). A stop, a command a script may not run and a command the
/// engine does not have are refused.
fn refused_step(s: &Step) -> Option<(String, String)> {
    let refused = |id: &str| id == STOP_STEP || refused_to_scripts(id) || crate::find_command(id).is_none();
    if refused(&s.id) {
        return Some((s.place.clone(), s.id.clone()));
    }
    if s.id != "action.batch" {
        return None;
    }
    let commands = s.params.get("commands").and_then(Value::as_array)?;
    commands.iter().enumerate().find_map(|(k, c)| {
        let id = c.get("command").and_then(Value::as_str).unwrap_or("");
        refused(id).then(|| (format!("{}.{}", s.place, k + 1), id.to_owned()))
    })
}

/// `params` of step `id`, with the active artboard of the active document added as a
/// `command.batch` given that artboard adds it when the Actions panel plays the step: `artboard` for
/// the commands that act on the active artboard, `index` for Fit to Artwork Bounds and Fit to
/// Selected Art. The active artboard is the index `start` gives for the document (its index as the
/// run started, which the view keeps), past the last artboard the last one, as the view shows it
/// after a step deletes artboards; a document the run opened has its own ([`super::dom`]'s). While
/// the Artboard tool has several artboards selected, a Fit to Artwork Bounds step gets those as its
/// `indices` instead, and a paste step gets none (it pastes on them). The run gives the tool fresh
/// state, which drops its selection, so while the tool is still the Artboard tool and has none, the
/// selection [`Start::picks`] holds for the document as the run started applies: a Fit to Artwork
/// Bounds step gets those as its `indices` and a paste step as its `artboards`, the ones past the
/// last artboard left out, and only while two or more remain. A step of a recorded `command.batch`
/// that names its `artboard` ([`Start::batch`]) gets that artboard instead of the active one, past
/// the last artboard the last one, and none in a document without artboards, as `command.batch`
/// gives it. A paste step in a document without artboards, or one that names `artboards`, and a Fit
/// to Artwork Bounds step that names `indices`, get none. A key the step gives is kept.
pub(crate) fn on_active_artboard(s: &crate::Session, start: &Start<'_>, id: &str, params: &Value) -> Value {
    let mut params = params.clone();
    let Some(st) = s.active() else { return params };
    let n = st.doc.artboards.len();
    let active = match (start.batch, start.active.iter().find(|(uid, _)| *uid == st.uid)) {
        (Some(_), _) if n == 0 => return params,
        (Some(a), _) => usize::try_from(a).map_or(n - 1, |a| a.min(n - 1)),
        (None, Some((_, i))) => (*i).min(n.saturating_sub(1)),
        (None, None) => super::dom::active_artboard(st),
    };
    let paste = matches!(id, "edit.pasteInPlace" | "edit.pasteInFront" | "edit.pasteInBack");
    let fit_art = id == "artboard.fitToArt";
    let key = if crate::ON_ACTIVE_ARTBOARD.contains(&id) {
        "artboard"
    } else if crate::FIT_ACTIVE_ARTBOARD.contains(&id) {
        "index"
    } else {
        return params;
    };
    let Some(p) = params.as_object_mut() else { return params };
    if p.contains_key(key) || (fit_art && p.contains_key("indices")) || (paste && (n == 0 || p.contains_key("artboards"))) {
        return params;
    }
    let picks = if paste || fit_art { crate::cmd::artboard_tool_picks(s) } else { Vec::new() };
    let kept: Vec<usize> = match start.picks {
        Some((uid, list)) if picks.is_empty() && (paste || fit_art) && *uid == st.uid && s.tool_id() == "artboard" => {
            list.iter().copied().filter(|i| *i < n).collect()
        }
        _ => Vec::new(),
    };
    if kept.len() >= 2 {
        p.insert((if fit_art { "indices" } else { "artboards" }).into(), json!(kept));
    } else if picks.is_empty() {
        p.insert(key.into(), json!(active));
    } else if fit_art {
        p.insert("indices".into(), json!(picks));
    }
    params
}

/// What [`on_active_artboard`] reads besides the session: the index of each open document's active
/// artboard as the run started, by document uid; the Artboard tool's selection as the run started,
/// with its document's uid; and the `artboard` a step's recorded `command.batch` names.
pub(crate) struct Start<'a> {
    pub active: &'a [(u64, usize)],
    pub picks: Option<&'a (u64, Vec<usize>)>,
    pub batch: Option<u64>,
}
