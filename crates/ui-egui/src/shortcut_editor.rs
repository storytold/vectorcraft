//! Edit → Keyboard Shortcuts: user overrides of tool and command shortcuts, conflict detection,
//! presets, import/export, and the dialog.
//!
//! Overrides live in [`UiState::shortcut_overrides`] (persisted, [`crate::keymap::stored`]) keyed
//! by command id, `tool:<id>` or `panel:<id>`; an empty string removes a shortcut. A process-wide
//! mirror (synced every frame by [`sync`]) holds the keymap with them, so `menus::shortcut_of`, the
//! dispatcher, the native menu and tooltips consult them without threading the app state through.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use craft_keymap::{AssignError, Binding, ConflictKind, Keymap, Overrides, Policy, Trigger};
use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use crate::keymap::{command_of, effective_with, entry_key, entry_of_stored, keymap_with, stored, vc_chord, vc_format};

use crate::state::{Dialog, UiState};
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, menus, widgets};

pub const PRESETS: &[&str] = &["VectorCraft Defaults", "Classic Defaults"];
pub const CUSTOM: &str = "Custom";
/// Set names earlier versions saved: (old name, current name).
pub const LEGACY_PRESETS: &[(&str, &str)] = &[("Illustrator Defaults", "Classic Defaults")]; // brand-ok: legacy preference value

/// The current name of a shortcut set: names earlier versions saved map to today's.
pub fn set_name(name: &str) -> &str {
    LEGACY_PRESETS.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new)
}

/// Serde reader for a saved set name (UI preferences written by earlier versions).
pub fn deserialize_set_name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    <String as serde::Deserialize>::deserialize(d).map(|s| set_name(&s).to_string())
}

/// Bumped whenever overrides or the workspace list change (the native menu rebuilds on it).
pub static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Declares `fn $name() -> &'static RwLock<$t>`: a mirror of UI state for code without app access
/// (menus, the shortcut dispatcher). One per process in the app; one per thread in unit tests,
/// which run in parallel threads, each driving its own app whose every frame syncs the mirror: a
/// shared one would let a test's frames overwrite another test's state mid-assertion.
macro_rules! ui_mirror {
    ($(#[$meta:meta])* $vis:vis fn $name:ident() -> $t:ty) => {
        $(#[$meta])*
        $vis fn $name() -> &'static std::sync::RwLock<$t> {
            #[cfg(not(test))]
            {
                static S: std::sync::OnceLock<std::sync::RwLock<$t>> = std::sync::OnceLock::new();
                S.get_or_init(Default::default)
            }
            #[cfg(test)]
            {
                thread_local!(static S: &'static std::sync::RwLock<$t> = Box::leak(Box::default()));
                S.with(|s| *s)
            }
        }
    };
}
pub(crate) use ui_mirror;

/// The keymap with the user's overrides, and the effective text of every entry they change.
pub(crate) struct Mirror {
    pub keymap: Keymap,
    text: HashMap<String, Option<&'static str>>,
}

impl Mirror {
    fn new(overrides: Overrides) -> Mirror {
        let keymap = crate::keymap::template().clone().with_overrides(overrides);
        let text = keymap
            .overrides()
            .iter()
            .map(|(k, _)| {
                let key = entry_of_stored(k);
                let t = effective_with(&keymap, &key).map(|t| intern(&t));
                (key, t)
            })
            .collect();
        Mirror { keymap, text }
    }
}

ui_mirror!(fn store() -> Option<Arc<Mirror>>);

/// The mirror: the keymap in effect.
pub(crate) fn current() -> Arc<Mirror> {
    static DEFAULTS: [OnceLock<Arc<Mirror>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    store()
        .read()
        .ok()
        .and_then(|m| m.clone())
        .unwrap_or_else(|| crate::keymap::per_platform(&DEFAULTS, crate::keymap::platform(), || Arc::new(Mirror::new(Overrides::new()))).clone())
}

/// Leak-once interning for override strings (bounded by the chords a user ever assigns).
pub fn intern(s: &str) -> &'static str {
    static SET: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = SET.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = set.get(s) {
        return v;
    }
    let v: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(v);
    v
}

/// Mirror `ui.shortcut_overrides` into the global store (cheap when unchanged).
pub fn sync(ui: &UiState) {
    let same = store()
        .read()
        .map(|m| match &*m {
            Some(m) => *m.keymap.overrides() == ui.shortcut_overrides && m.keymap.platform() == crate::keymap::platform(),
            None => ui.shortcut_overrides.is_empty(),
        })
        .unwrap_or(false);
    if !same {
        if let Ok(mut m) = store().write() {
            *m = Some(Arc::new(Mirror::new(ui.shortcut_overrides.clone())));
        }
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    crate::workspaces::sync(ui);
}

/// Registry default shortcut of a command (engine or UI command).
pub fn default_command_shortcut(id: &str) -> Option<&'static str> {
    if let Some(c) = vectorcraft_engine::find_command(id) {
        return c.shortcut;
    }
    menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.2).filter(|s| !s.is_empty())
}

/// Window menu items that show a panel (`window.panel {panel}`) with a default shortcut: (panel
/// id, shortcut). Every panel can be given one in the editor (entry key `panel:<id>`).
pub const PANEL_SHORTCUTS: &[(&str, &str)] = &[
    ("brushes", "F5"),
    ("color", "F6"),
    ("layers", "F7"),
    ("colorGuide", "Shift+F3"),
    ("graphicStyles", "Shift+F5"),
    ("appearance", "Shift+F6"),
    ("align", "Shift+F7"),
    ("transform", "Shift+F8"),
    ("info", "Cmd+F8"),
    ("gradient", "Cmd+F9"),
    ("pathfinder", "Cmd+Shift+F9"),
    ("stroke", "Cmd+F10"),
    ("transparency", "Cmd+Shift+F10"),
    ("attributes", "Cmd+F11"),
    ("symbols", "Cmd+Shift+F11"),
    ("character", "Cmd+T"),
    ("paragraph", "Cmd+Alt+T"),
    ("tabs", "Cmd+Shift+T"),
    ("openType", "Cmd+Alt+Shift+T"),
];

/// Default shortcut of an entry key (`tool:<id>`, `panel:<id>` or a command id).
pub fn default_of(key: &str) -> Option<&'static str> {
    if let Some(t) = key.strip_prefix("tool:") {
        return vectorcraft_tools::tool_info(t).and_then(|t| t.shortcut);
    }
    if let Some(p) = key.strip_prefix("panel:") {
        return PANEL_SHORTCUTS.iter().find(|(id, _)| *id == p).map(|(_, sc)| *sc);
    }
    default_command_shortcut(key)
}

/// Effective shortcut of an entry key, honouring the user's overrides.
fn effective(key: &str) -> Option<&'static str> {
    let m = current();
    match m.text.get(key) {
        Some(t) => *t,
        None => default_of(key),
    }
}

/// Effective shortcut of `key` under explicit overrides: override > default.
pub fn effective_in(overrides: &Overrides, key: &str) -> Option<String> {
    effective_with(&keymap_with(overrides), key)
}

/// Effective shortcut of a command, honouring the user's overrides.
pub fn command_shortcut(id: &str) -> Option<&'static str> {
    effective(id)
}

/// Effective shortcut of a tool.
pub fn tool_shortcut(tool: &str) -> Option<&'static str> {
    effective(&format!("tool:{tool}"))
}

/// Effective shortcut of the Window menu item showing `panel` (`window.panel {panel}`).
pub fn panel_shortcut(panel: &str) -> Option<&'static str> {
    effective(&format!("panel:{panel}"))
}

/// Tool whose (effective) shortcut is `key` ("V", "Shift+M").
pub fn tool_for_key(key: &str) -> Option<&'static str> {
    let c = crate::keymap::lookup(&current().keymap, &vc_chord(key)?)?;
    c.id.strip_prefix("tool:").and_then(vectorcraft_tools::tool_info).map(|t| t.id)
}

/// Command (not a tool or a panel) whose effective shortcut is exactly `key`.
pub fn command_for_key(key: &str) -> Option<&'static str> {
    let c = crate::keymap::lookup(&current().keymap, &vc_chord(key)?)?;
    vectorcraft_engine::command_specs()
        .iter()
        .map(|c| c.id)
        .chain(menus::UI_COMMANDS.iter().map(|c| c.0))
        .find(|id| *id == c.id && c.params.is_null())
}

// ---------- chords ----------

/// Canonical text of a chord: `Cmd+Ctrl+Alt+Shift+Key`.
pub fn format(sc: &KeyboardShortcut) -> String {
    let m = sc.modifiers;
    let mut s = String::new();
    if m.command || m.mac_cmd {
        s.push_str("Cmd+");
    }
    if m.ctrl && (cfg!(target_os = "macos") || !m.command) {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    let k = sc.logical_key;
    let name = match k {
        // A shifted punctuation key is recorded as the key, not the character it types.
        Key::CloseCurlyBracket if m.shift => "]",
        Key::OpenCurlyBracket if m.shift => "[",
        Key::Questionmark if m.shift => "/",
        Key::Colon if m.shift => ";",
        Key::Pipe if m.shift => "\\",
        Key::CloseBracket => "]",
        Key::OpenBracket => "[",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Equals | Key::Plus => "=",
        Key::Minus => "-",
        Key::Backtick => "~",
        Key::Comma => "Comma",
        Key::Period => "Period",
        k => k.name(),
    };
    s.push_str(name);
    s
}

/// Canonical form of a shortcut string (None if it doesn't parse).
pub fn normalize(s: &str) -> Option<String> {
    vc_chord(s).map(|c| vc_format(&c))
}

/// A recorded key press → shortcut text (None for keys that can't be shortcuts).
pub fn chord_from_event(key: Key, m: Modifiers) -> Option<String> {
    if matches!(key, Key::Escape) {
        return None;
    }
    let s = format(&KeyboardShortcut::new(m, key));
    crate::shortcuts::parse(&s).map(|_| s)
}

// ---------- entries & conflicts ----------

#[derive(Clone, Debug)]
pub struct Entry {
    /// `tool:<id>` or a command id.
    pub key: String,
    pub label: String,
    /// Menu path ("Object › Arrange") or "Tools".
    pub group: String,
    pub is_tool: bool,
}

/// Every tool and menu command the editor lists.
pub fn entries() -> &'static [Entry] {
    static E: OnceLock<Vec<Entry>> = OnceLock::new();
    E.get_or_init(|| {
        let mut v: Vec<Entry> = vectorcraft_tools::catalog::all_tools()
            .map(|t| Entry { key: format!("tool:{}", t.id), label: t.label.to_string(), group: "Tools".into(), is_tool: true })
            .collect();
        let mut seen = HashSet::new();
        for c in vectorcraft_engine::command_specs() {
            // Commands outside the menus (panel menu items) are
            // listed when a key alone can run them; queries and aliases never are.
            let listed = if c.menu.is_empty() {
                c.shortcut.is_some() || (c.journal && !vectorcraft_engine::cmd::is_alias(c.id) && runs_without_params(c.params))
            } else {
                true
            };
            if !listed || !seen.insert(c.id) {
                continue;
            }
            let group = if c.menu.is_empty() { id_group(c.id) } else { c.menu.join(" › ") };
            v.push(Entry { key: c.id.into(), label: c.label.into(), group, is_tool: false });
        }
        v.extend(crate::state::all_panels().map(|(id, label)| Entry {
            key: format!("panel:{id}"),
            label: label.to_string(),
            group: "Window".into(),
            is_tool: false,
        }));
        for (id, label, sc, params) in menus::UI_COMMANDS {
            if !seen.insert(id) || (sc.is_empty() && !params.starts_with("{}") && !params.starts_with("{path")) {
                continue;
            }
            v.push(Entry { key: id.to_string(), label: label.to_string(), group: id_group(id), is_tool: false });
        }
        v
    })
}

/// The group of a command no menu holds: the first part of its id, capitalised (`edit.x` → "Edit").
fn id_group(id: &str) -> String {
    let mut c = id.split('.').next().unwrap_or_default().chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Does a command's params doc (`{a?, b?} …`) say it runs with no params, as a shortcut runs it?
fn runs_without_params(params: &str) -> bool {
    params
        .strip_prefix('{')
        .and_then(|r| r.split_once('}'))
        .is_some_and(|(fields, _)| fields.split(',').all(|f| f.trim().is_empty() || f.trim_end().ends_with('?')))
}

pub fn entry(key: &str) -> Option<&'static Entry> {
    entries().iter().find(|e| e.key == key)
}

/// Entries (other than `except`) whose effective shortcut equals `chord`.
pub fn conflicts_with(overrides: &Overrides, chord: &str, except: &str) -> Vec<String> {
    let Some(c) = vc_chord(chord) else { return vec![] };
    let km = keymap_with(overrides);
    let p = km.platform();
    let want = Trigger::single(c).effective(p);
    entries()
        .iter()
        .filter(|e| e.key != except)
        .filter(|e| km.bindings(&command_of(&e.key)).iter().any(|b| b.trigger.effective(p) == want))
        .map(|e| e.key.clone())
        .collect()
}

/// All chords used by more than one entry: (chord, keys).
pub fn all_conflicts(overrides: &Overrides) -> Vec<(String, Vec<String>)> {
    conflicts_in(&keymap_with(overrides))
}

/// [`all_conflicts`] of a keymap.
pub fn conflicts_in(km: &Keymap) -> Vec<(String, Vec<String>)> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in km.conflicts().into_iter().filter(|c| c.kind == ConflictKind::Clash) {
        let keys = by.entry(crate::keymap::binding_text(&Binding::new(c.chord))).or_default();
        for k in [entry_key(&c.a.0), entry_key(&c.b.0)] {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    by.into_iter().collect()
}

/// Assign `chord` (None = remove, "default" handled by callers via [`reset_one`]) to `key` in
/// `overrides`. Conflicting entries lose the chord when `force`, otherwise it's an error.
pub fn assign(overrides: &mut Overrides, key: &str, chord: Option<&str>, force: bool) -> Result<Vec<String>, String> {
    let mut km = keymap_with(overrides);
    let removed = assign_in(&mut km, key, chord, force)?;
    *overrides = km.overrides().clone();
    Ok(removed)
}

/// [`assign`] on a keymap.
pub fn assign_in(km: &mut Keymap, key: &str, chord: Option<&str>, force: bool) -> Result<Vec<String>, String> {
    let command = command_of(key);
    if km.bindable(&command).is_none() {
        return Err(format!("unknown tool or command `{key}`"));
    }
    let c = match chord {
        None | Some("") => {
            // Fails only when the overrides hold the most entries they can.
            if !km.clear(&command) {
                return Err("too many changed shortcuts".into());
            }
            return Ok(vec![]);
        }
        Some(c) => c,
    };
    let chord = vc_chord(c).ok_or_else(|| format!("can't parse shortcut `{c}`"))?;
    if key.starts_with("tool:") && (chord.mods.cmd || chord.mods.ctrl || chord.mods.alt) {
        return Err("tool shortcuts must be a single key (optionally with Shift)".into());
    }
    let policy = if force { Policy::Steal } else { Policy::Refuse };
    match km.set_bindings(&command, vec![Binding::new(chord)], policy) {
        Ok(done) => {
            let mut removed: Vec<String> = vec![];
            for h in done.taken_from {
                let k = entry_key(&h.command);
                if !removed.contains(&k) {
                    removed.push(k);
                }
            }
            Ok(removed)
        }
        Err(AssignError::Taken(holders)) => {
            let names: Vec<String> = holders
                .iter()
                .map(|h| {
                    let k = entry_key(&h.command);
                    entry(&k).map_or(k, |e| e.label.clone())
                })
                .collect();
            Err(format!("{} is already used by {}", menus::pretty_shortcut(&vc_format(&chord)), names.join(", ")))
        }
        Err(AssignError::Reserved(_)) => Err(format!("{} is reserved by the system", menus::pretty_shortcut(&vc_format(&chord)))),
        Err(e) => Err(e.to_string()),
    }
}

pub fn reset_one(overrides: &mut Overrides, key: &str) {
    overrides.remove(&crate::keymap::stored_key(key));
}

/// Overrides of a preset (both presets are the registry defaults for now).
pub fn preset(name: &str) -> Option<Overrides> {
    PRESETS.contains(&set_name(name)).then(Overrides::new)
}

/// Export format: `{"format": "vectorcraft-shortcuts", "set": name, "overrides": {...}}`.
pub fn export_json(set: &str, overrides: &Overrides) -> Value {
    json!({"format": "vectorcraft-shortcuts", "version": 1, "set": set, "overrides": stored::to_value(overrides)})
}

pub fn import_json(v: &Value) -> Result<(String, Overrides), String> {
    let o = v.get("overrides").and_then(Value::as_object).ok_or("not a VectorCraft shortcut set (missing `overrides`)")?;
    let mut out = Overrides::new();
    for (k, v) in o {
        let s = v.as_str().ok_or_else(|| format!("shortcut for `{k}` must be a string"))?;
        if !s.is_empty() && normalize(s).is_none() {
            return Err(format!("can't parse shortcut `{s}` for `{k}`"));
        }
        crate::keymap::set_override(&mut out, k, &normalize(s).unwrap_or_default());
    }
    Ok((v.get("set").and_then(Value::as_str).map_or(CUSTOM, set_name).to_string(), out))
}

// ---------- UI commands ----------

pub fn open(app: &mut VectorcraftApp) {
    let ov = stored::to_value(&app.ui.shortcut_overrides);
    app.ui.dialog = Some(Dialog::new(
        "shortcuts",
        json!({"tab": "tools", "query": "", "set": app.ui.shortcut_set, "overrides": ov, "__recording": "", "__selected": "", "__message": "", "__conflict": ""}),
    ));
}

fn dialog_overrides(d: &Dialog) -> Overrides {
    d.fields.get("overrides").map(|v| stored::from_value(v.clone())).unwrap_or_default()
}

/// OK: commit the working set.
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    app.ui.shortcut_overrides = dialog_overrides(&d);
    app.ui.shortcut_set = d.str("set");
    app.ui.dialog = None;
    sync(&app.ui);
    Ok(json!({"overrides": app.ui.shortcut_overrides.len()}))
}

/// `shortcuts.*` UI commands. `None` = not ours.
pub fn run_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "edit.keyboardShortcuts" => {
            open(app);
            Ok(Value::Null)
        }
        "shortcuts.set" => {
            let key = s("id").unwrap_or_default();
            let mut ov = app.ui.shortcut_overrides.clone();
            let r = if p.get("shortcut").is_some_and(Value::is_null) {
                reset_one(&mut ov, &key);
                Ok(vec![])
            } else {
                assign(&mut ov, &key, s("shortcut").as_deref(), p.get("force").and_then(Value::as_bool).unwrap_or(false))
            };
            r.map(|removed| {
                app.ui.shortcut_overrides = ov;
                app.ui.shortcut_set = CUSTOM.into();
                sync(&app.ui);
                json!({"id": key, "shortcut": effective_in(&app.ui.shortcut_overrides, &key), "removedFrom": removed})
            })
        }
        "shortcuts.list" => {
            let q = s("query").unwrap_or_default().to_lowercase();
            let km = keymap_with(&app.ui.shortcut_overrides);
            let ov = km.overrides();
            Ok(Value::Array(
                entries()
                    .iter()
                    .filter(|e| q.is_empty() || e.label.to_lowercase().contains(&q) || e.key.to_lowercase().contains(&q))
                    .map(|e| json!({"id": e.key, "label": e.label, "group": e.group, "shortcut": effective_with(&km, &e.key), "default": default_of(&e.key), "overridden": crate::keymap::has_override(ov, &e.key)}))
                    .collect(),
            ))
        }
        "shortcuts.conflicts" => {
            Ok(json!(all_conflicts(&app.ui.shortcut_overrides).into_iter().map(|(c, k)| json!({"shortcut": c, "ids": k})).collect::<Vec<_>>()))
        }
        "shortcuts.reset" => {
            app.ui.shortcut_overrides.clear();
            app.ui.shortcut_set = PRESETS[0].into();
            sync(&app.ui);
            Ok(Value::Null)
        }
        "shortcuts.preset" => match s("name").as_deref().map(set_name).and_then(|n| preset(n).map(|p| (n.to_string(), p))) {
            Some((n, p)) => {
                app.ui.shortcut_overrides = p;
                app.ui.shortcut_set = n;
                sync(&app.ui);
                Ok(Value::Null)
            }
            None => Err(format!("unknown preset (one of {})", PRESETS.join(", "))),
        },
        "shortcuts.export" => {
            let v = export_json(&app.ui.shortcut_set, &app.ui.shortcut_overrides);
            let bytes = serde_json::to_vec_pretty(&v).unwrap_or_default();
            let pick = crate::FilePick {
                filters: vec![crate::FilePick::filter("Shortcut sets", &["json"])],
                ..crate::FilePick::named("VectorCraft Shortcuts.json")
            };
            let path = s("path").or_else(|| crate::picks::save(app, &pick));
            match path {
                Some(path) => match app.services.write.as_mut() {
                    Some(w) => w(&path, &bytes).map(|_| json!({"path": path})),
                    None => Err("no file writer".into()),
                },
                // Its save dialog is open: the file is written when it answers.
                None if app.file_dialog_open() => Err("cancelled".into()),
                None => match app.services.download.as_mut() {
                    Some(dl) => dl("VectorCraft Shortcuts.json", &bytes).map(|()| Value::Null),
                    None => Ok(v),
                },
            }
        }
        "shortcuts.import" => {
            let data = if let Some(v) = p.get("data") {
                Ok(v.clone())
            } else {
                let pick = crate::FilePick {
                    filters: vec![crate::FilePick::filter("Shortcut sets", crate::dialogs::shortcut_import::IMPORT_EXTS)],
                    ..Default::default()
                };
                let path = s("path").or_else(|| crate::picks::open(app, &pick));
                // A `.kys` file is read by Import Shortcut Set, up to its reader's cap.
                if let Some(path) = path.as_deref().filter(|p| crate::dialogs::shortcut_import::is_kys(p)) {
                    return Some(crate::dialogs::shortcut_import::command(app, &json!({ "path": path })));
                }
                match (path, app.services.read.as_ref()) {
                    (Some(path), Some(r)) => match r(&path) {
                        // A set another app saved as text opens Import Shortcut Set.
                        Ok(b) if crate::dialogs::shortcut_import::is_text_set(&path, &b) => {
                            return Some(crate::dialogs::shortcut_import::open(app, &path, &b));
                        }
                        b => b.and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())),
                    },
                    (None, _) => Err("cancelled".into()),
                    (_, None) => Err("no file reader".into()),
                }
            };
            data.and_then(|v| import_json(&v)).map(|(set, ov)| {
                let n = ov.len();
                app.ui.shortcut_overrides = ov;
                app.ui.shortcut_set = set;
                sync(&app.ui);
                if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == "shortcuts") {
                    d.fields.insert("overrides".into(), stored::to_value(&app.ui.shortcut_overrides));
                    d.fields.insert("set".into(), json!(app.ui.shortcut_set));
                }
                json!({"overrides": n})
            })
        }
        _ => return None,
    };
    Some(r)
}

/// While recording (here or in Import Shortcut Set), Escape cancels the recording instead of closing
/// the dialog.
pub fn is_recording(app: &VectorcraftApp) -> bool {
    app.ui
        .dialog
        .as_ref()
        .is_some_and(|d| (d.kind == "shortcuts" || d.kind == crate::dialogs::shortcut_import::KIND) && !d.str("__recording").is_empty())
}

// ---------- dialog ----------

/// The dialog's working set as a keymap, with each entry's shortcut text (in [`entries`] order)
/// and the number of conflicts, kept between frames while the set is unchanged.
struct DialogView {
    /// The dialog's `overrides` field this was built from.
    source: Value,
    keymap: Keymap,
    texts: Vec<Option<String>>,
    conflicts: usize,
}

impl DialogView {
    fn new(overrides: Overrides) -> DialogView {
        let keymap = keymap_with(&overrides);
        let texts = entries().iter().map(|e| effective_with(&keymap, &e.key)).collect();
        let conflicts = conflicts_in(&keymap).len();
        DialogView { source: stored::to_value(keymap.overrides()), keymap, texts, conflicts }
    }

    /// The view of `d`'s working set, from the cache unless the set changed. The dialog's
    /// `overrides` field is left in its stored form.
    fn of(ctx: &egui::Context, d: &mut Dialog) -> Arc<DialogView> {
        let id = egui::Id::new("dialog-shortcuts-view");
        let source = d.fields.get("overrides").unwrap_or(&Value::Null);
        if let Some(v) = ctx.data(|m| m.get_temp::<Arc<DialogView>>(id)).filter(|v| v.source == *source) {
            return v;
        }
        let v = Arc::new(DialogView::new(dialog_overrides(d)));
        d.fields.insert("overrides".into(), v.source.clone());
        ctx.data_mut(|m| m.insert_temp(id, v.clone()));
        v
    }
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    // The working set, as a keymap: copied only by an edit this frame.
    let view = DialogView::of(ctx, &mut d);
    let mut km = Cow::Borrowed(&view.keymap);
    let mut ok = false;
    let mut cancel = false;
    let recording = d.str("__recording");
    // Record a chord (before widgets see the keys). Modifiers held down are part of the next key's
    // chord, not a chord of their own.
    if !recording.is_empty() {
        let ev = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } if !crate::shortcuts::is_modifier(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = ev {
            ctx.input_mut(|i| i.events.clear());
            if key == Key::Escape {
                d.fields.insert("__recording".into(), json!(""));
            } else if let Some(chord) = chord_from_event(key, m) {
                record(&mut d, km.to_mut(), &recording, &chord);
            }
        }
    }
    crate::dialogs::modal::show(ctx, tl!("Keyboard Shortcuts"), egui::Id::new("dialog-shortcuts"), -20.0, 20, |ui| {
        ui.set_width(640.0);
        crate::dialogs::modal::heading(ui, tl!("Keyboard Shortcuts"));
        ui.add_space(10.0);
        // Set row.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Set:")).color(t.text_dim));
            let cur = d.str("set");
            let mut opts: Vec<&str> = PRESETS.to_vec();
            if !PRESETS.contains(&cur.as_str()) {
                opts.push(CUSTOM);
            }
            // The sets listed are ours (translated); a set read from an imported file shows
            // its name as it is.
            if let Some(&name) = crate::dialogs::mixed_dropdown(ui, "kbset", &cur, &opts, 200.0, |_| true).and_then(|i| opts.get(i))
                && let Some(p) = preset(name)
            {
                km.to_mut().set_overrides(p);
                d.fields.insert("set".into(), json!(name));
                d.fields.insert("__message".into(), json!(""));
            }
            ui.add_space(12.0);
            if ui.button(tl!("Import…")).clicked() {
                d.fields.insert("__import".into(), json!(true));
            }
            if ui.button(tl!("Export…")).clicked() {
                d.fields.insert("__export".into(), json!(true));
            }
            if ui.button(tl!("Reset to Defaults")).clicked() {
                km.to_mut().reset_all();
                d.fields.insert("set".into(), json!(PRESETS[0]));
                d.fields.insert("__message".into(), json!(tl!("All shortcuts reset to defaults.")));
            }
        });
        ui.add_space(8.0);
        // Tabs + search.
        ui.horizontal(|ui| {
            let tab = d.str("tab");
            for (id, label) in [("tools", "Tools"), ("menu", "Menu Commands")] {
                if ui.selectable_label(tab == id, egui::RichText::new(tl!(label)).font(theme::semibold(12.5))).clicked() {
                    d.fields.insert("tab".into(), json!(id));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut q = d.str("query");
                if ui.add(egui::TextEdit::singleline(&mut q).hint_text(tl!("Search")).desired_width(200.0)).changed() {
                    d.fields.insert("query".into(), json!(q));
                }
            });
        });
        ui.add_space(6.0);
        let tools = d.str("tab") != "menu";
        let q = d.str("query").to_lowercase();
        let selected = d.str("__selected");
        let rec = d.str("__recording");
        let scroll_to = d.fields.remove("__scrollTo").and_then(|v| v.as_str().map(str::to_string));
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(360.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(360.0);
                    ui.label(egui::RichText::new(if tools { tl!("Tool") } else { tl!("Command") }).color(t.text_dim).size(11.0));
                });
                ui.label(egui::RichText::new(tl!("Shortcut")).color(t.text_dim).size(11.0));
            });
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
                for (i, e) in entries().iter().enumerate().filter(|(_, e)| e.is_tool == tools) {
                    let sc = match &km {
                        Cow::Borrowed(_) => view.texts.get(i).cloned().flatten(),
                        Cow::Owned(km) => effective_with(km, &e.key),
                    };
                    let matches = e.label.to_lowercase().contains(&q)
                        || tl!(&e.label).to_lowercase().contains(&q)
                        || e.group.to_lowercase().contains(&q)
                        || tl!(&e.group).to_lowercase().contains(&q)
                        || sc.as_deref().is_some_and(|s| s.to_lowercase().contains(&q));
                    if !q.is_empty() && !matches {
                        continue;
                    }
                    let row = ui.horizontal(|ui| {
                        let name = if e.is_tool || e.group.is_empty() {
                            tl!(&e.label).to_string()
                        } else {
                            format!("{} › {}", tl!(&e.group), tl!(&e.label))
                        };
                        let r = ui
                            .allocate_ui_with_layout(egui::vec2(360.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.set_min_width(360.0);
                                ui.add(egui::Button::selectable(selected == e.key, egui::RichText::new(name).color(t.text)).truncate())
                            })
                            .inner;
                        if r.clicked() {
                            d.fields.insert("__selected".into(), json!(e.key));
                        }
                        let label = if rec == e.key {
                            tl!("Press keys…").to_string()
                        } else {
                            sc.as_deref().map(menus::pretty_shortcut).unwrap_or_else(|| "—".into())
                        };
                        let changed = crate::keymap::has_override(km.overrides(), &e.key);
                        let txt = egui::RichText::new(label).color(if changed { t.accent } else { t.text });
                        let b = ui
                            .add_sized([150.0, 20.0], egui::Button::new(txt).selected(rec == e.key))
                            .on_hover_text(tl!("Click, then press the new shortcut (Esc cancels)"));
                        if b.clicked() {
                            d.fields.insert("__recording".into(), json!(e.key));
                            d.fields.insert("__selected".into(), json!(e.key));
                        }
                    });
                    if scroll_to.as_deref() == Some(e.key.as_str()) {
                        row.response.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
        });
        ui.add_space(6.0);
        // Selected-row actions + message.
        ui.horizontal(|ui| {
            let sel = d.str("__selected");
            ui.add_enabled_ui(!sel.is_empty(), |ui| {
                if ui.button(tl!("Clear")).on_hover_text(tl!("Remove the shortcut")).clicked() {
                    if let Err(e) = assign_in(km.to_mut(), &sel, None, true) {
                        d.fields.insert("__message".into(), json!(e));
                    }
                    d.fields.insert("set".into(), json!(CUSTOM));
                }
                if ui.button(tl!("Use Default")).clicked() {
                    km.to_mut().reset(&command_of(&sel));
                }
            });
            let conflict = d.str("__conflict");
            if !conflict.is_empty() && ui.button(tl!("Go to Conflict")).clicked() {
                let tab = if conflict.starts_with("tool:") { "tools" } else { "menu" };
                d.fields.insert("tab".into(), json!(tab));
                d.fields.insert("query".into(), json!(""));
                d.fields.insert("__selected".into(), json!(conflict));
                d.fields.insert("__scrollTo".into(), json!(conflict));
                d.fields.insert("__conflict".into(), json!(""));
            }
        });
        let msg = d.str("__message");
        if !msg.is_empty() {
            ui.label(egui::RichText::new(msg).color(t.accent_strong).size(11.5));
        }
        let n = match &km {
            Cow::Borrowed(_) => view.conflicts,
            Cow::Owned(km) => conflicts_in(km).len(),
        };
        if n > 0 {
            ui.label(
                egui::RichText::new(crate::i18n::tn(
                    n as u64,
                    "{n} shortcut is assigned more than once",
                    "{n} shortcuts are assigned more than once",
                ))
                .color(t.text_dim)
                .size(11.0),
            );
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::primary_button(ui, tl!("OK")).clicked() {
                ok = true;
            }
            ui.add_space(8.0);
            if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if let Cow::Owned(km) = &km {
        d.fields.insert("overrides".into(), stored::to_value(km.overrides()));
    }
    let import = d.fields.remove("__import").is_some();
    let export = d.fields.remove("__export").is_some();
    app.ui.dialog = Some(d);
    // Their file dialogs, shown off the UI thread, import into or export from the dialog as it is
    // when they answer.
    if import {
        crate::picks::button(app, import_into_dialog);
    }
    if export {
        crate::picks::button(app, export_from_dialog);
    }
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

/// The Keyboard Shortcuts dialog, while it is open.
fn shortcuts_dialog(app: &VectorcraftApp) -> bool {
    app.ui.dialog.as_ref().is_some_and(|d| d.kind == "shortcuts")
}

/// The dialog's Import…: the imported set replaces its working set (committed on OK like other
/// edits).
fn import_into_dialog(app: &mut VectorcraftApp) {
    if !shortcuts_dialog(app) {
        app.status(crate::picks::dialog_gone());
        return;
    }
    let saved = (app.ui.shortcut_overrides.clone(), app.ui.shortcut_set.clone());
    match run_command(app, "shortcuts.import", &json!({})) {
        Some(Ok(_)) => {
            let (o, s) = (app.ui.shortcut_overrides.clone(), app.ui.shortcut_set.clone());
            (app.ui.shortcut_overrides, app.ui.shortcut_set) = saved;
            sync(&app.ui);
            if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == "shortcuts") {
                d.fields.insert("overrides".into(), stored::to_value(&o));
                d.fields.insert("set".into(), json!(s));
                d.fields.insert("__message".into(), json!(tl!("Imported. Press OK to keep the imported set.")));
            }
        }
        Some(Err(e)) if e != "cancelled" => app.status(e),
        _ => {}
    }
}

/// The dialog's Export…: its working set, as edited so far.
fn export_from_dialog(app: &mut VectorcraftApp) {
    if !shortcuts_dialog(app) {
        app.status(crate::picks::dialog_gone());
        return;
    }
    let (ov, set) = app.ui.dialog.as_ref().map(|d| (dialog_overrides(d), d.str("set"))).unwrap_or_default();
    let saved = std::mem::replace(&mut app.ui.shortcut_overrides, ov);
    let saved_set = std::mem::replace(&mut app.ui.shortcut_set, set);
    if let Some(Err(e)) = run_command(app, "shortcuts.export", &json!({})) {
        app.status(e);
    }
    app.ui.shortcut_overrides = saved;
    app.ui.shortcut_set = saved_set;
}

/// Apply a recorded chord to `key` in the working set, warning about (and resolving) conflicts.
fn record(d: &mut Dialog, km: &mut Keymap, key: &str, chord: &str) {
    d.fields.insert("__recording".into(), json!(""));
    match assign_in(km, key, Some(chord), true) {
        Ok(removed) => {
            d.fields.insert("set".into(), json!(CUSTOM));
            if let Some(first) = removed.first() {
                let names: Vec<String> = removed.iter().map(|k| entry(k).map(|e| e.label.clone()).unwrap_or(k.clone())).collect();
                d.fields.insert(
                    "__message".into(),
                    json!(crate::i18n::fmt(
                        tl!("⚠ {chord} was already used by {names} — it has been removed there."),
                        &[("chord", &menus::pretty_shortcut(chord)), ("names", &names.join(", "))]
                    )),
                );
                d.fields.insert("__conflict".into(), json!(first));
            } else {
                d.fields.insert("__message".into(), json!(""));
                d.fields.insert("__conflict".into(), json!(""));
            }
        }
        Err(e) => {
            d.fields.insert("__message".into(), json!(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_round_trip_for_every_default() {
        for e in entries() {
            if let Some(s) = default_of(&e.key)
                && let Some(sc) = crate::shortcuts::parse(s)
            {
                let f = format(&sc);
                assert_eq!(crate::shortcuts::parse(&f), Some(sc), "{} `{s}` → `{f}`", e.key);
                assert_eq!(normalize(&f).as_deref(), Some(f.as_str()));
            }
        }
    }

    #[test]
    fn normalize_is_order_insensitive() {
        assert_eq!(normalize("Shift+Cmd+Alt+K"), normalize("Cmd+Alt+Shift+K"));
        assert_eq!(normalize("Cmd+Shift+]").as_deref(), Some("Cmd+Shift+]"));
        assert_eq!(normalize("Cmd+="), Some("Cmd+=".into()));
        assert!(normalize("Hyper+K").is_none());
    }

    #[test]
    fn chord_from_recorded_event() {
        assert_eq!(chord_from_event(Key::K, Modifiers::COMMAND | Modifiers::SHIFT).as_deref(), Some("Cmd+Shift+K"));
        assert_eq!(chord_from_event(Key::P, Modifiers::NONE).as_deref(), Some("P"));
        assert_eq!(chord_from_event(Key::Escape, Modifiers::NONE), None);
        // A modifier's own key press is no chord (#487).
        assert_eq!(chord_from_event(Key::ShiftLeft, Modifiers::SHIFT), None);
        assert!(normalize("ShiftLeft").is_none() && normalize("Cmd+ControlRight").is_none());
    }

    #[test]
    fn a_modifier_pressed_while_recording_waits_for_the_key() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        open(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("__recording".into(), json!("tool:groupSelection"));
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut press = |key, modifiers| {
            let events = vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            app.ui.dialog.clone().unwrap()
        };
        // egui reports Shift's own key press first (#487): the recording goes on.
        assert_eq!(press(Key::ShiftLeft, Modifiers::SHIFT).str("__recording"), "tool:groupSelection");
        let d = press(Key::A, Modifiers::SHIFT);
        assert_eq!(d.str("__recording"), "");
        assert_eq!(crate::keymap::override_text(&dialog_overrides(&d), "tool:groupSelection").as_deref(), Some("Shift+A"));
    }

    #[test]
    fn saved_modifier_only_overrides_give_the_default_back() {
        let mut ui = UiState::default();
        crate::keymap::set_override(&mut ui.shortcut_overrides, "tool:groupSelection", "Shift+ShiftLeft");
        crate::keymap::set_override(&mut ui.shortcut_overrides, "edit.preferences", "");
        let ui = ui.sanitized();
        assert!(!crate::keymap::has_override(&ui.shortcut_overrides, "tool:groupSelection"));
        assert_eq!(crate::keymap::override_text(&ui.shortcut_overrides, "edit.preferences").as_deref(), Some(""));
    }

    #[test]
    fn override_beats_default_and_empty_removes() {
        let mut ov = Overrides::new();
        assert_eq!(effective_in(&ov, "edit.preferences").as_deref(), Some("Cmd+K"));
        crate::keymap::set_override(&mut ov, "edit.preferences", "Cmd+Alt+Shift+9");
        assert_eq!(effective_in(&ov, "edit.preferences").as_deref(), Some("Cmd+Alt+Shift+9"));
        crate::keymap::set_override(&mut ov, "edit.preferences", "");
        assert_eq!(effective_in(&ov, "edit.preferences"), None);
        assert_eq!(effective_in(&Overrides::new(), "tool:pen").as_deref(), Some("P"));
    }

    #[test]
    fn global_override_reaches_shortcut_of_and_tools() {
        let mut ui = UiState::default();
        crate::keymap::set_override(&mut ui.shortcut_overrides, "test.nonexistent", "Cmd+Alt+Shift+F9");
        crate::keymap::set_override(&mut ui.shortcut_overrides, "tool:measure", "Shift+9");
        let g = GENERATION.load(Ordering::Relaxed);
        sync(&ui);
        assert!(GENERATION.load(Ordering::Relaxed) > g);
        assert_eq!(menus::shortcut_of("test.nonexistent"), Some("Cmd+Alt+Shift+F9"));
        assert_eq!(tool_shortcut("measure"), Some("Shift+9"));
        assert_eq!(tool_for_key("Shift+9"), Some("measure"));
        sync(&UiState::default());
        assert_eq!(menus::shortcut_of("test.nonexistent"), None);
    }

    #[test]
    fn another_tests_frames_leave_this_tests_mirror_alone() {
        let mut ui = UiState::default();
        crate::keymap::set_override(&mut ui.shortcut_overrides, "tool:measure", "Shift+8");
        crate::workspaces::save_as(&mut ui, "Mine").unwrap();
        sync(&ui);
        // What each frame of a test running in parallel does with its default state.
        std::thread::spawn(|| sync(&UiState::default())).join().unwrap();
        assert_eq!(tool_shortcut("measure"), Some("Shift+8"));
        assert!(crate::workspaces::menu_items().iter().any(|i| matches!(i, menus::Item::Cmd("Mine", ..))));
    }

    #[test]
    fn assign_detects_conflicts() {
        let mut ov = Overrides::new();
        // ⌘K is Preferences.
        let err = assign(&mut ov, "file.revert", Some("Cmd+K"), false).unwrap_err();
        assert!(err.contains("Preferences"), "{err}");
        assert!(ov.is_empty());
        assert_eq!(conflicts_with(&ov, "Cmd+K", "file.revert"), vec!["edit.preferences".to_string()]);
    }

    #[test]
    fn forced_assign_moves_the_chord() {
        let mut ov = Overrides::new();
        let removed = assign(&mut ov, "file.revert", Some("Cmd+K"), true).unwrap();
        assert_eq!(removed, vec!["edit.preferences".to_string()]);
        assert_eq!(effective_in(&ov, "file.revert").as_deref(), Some("Cmd+K"));
        assert_eq!(effective_in(&ov, "edit.preferences"), None);
        assert!(conflicts_with(&ov, "Cmd+K", "file.revert").is_empty());
    }

    #[test]
    fn assigning_the_default_drops_the_override() {
        let mut ov = Overrides::new();
        assign(&mut ov, "tool:pen", Some("Shift+9"), false).unwrap();
        assert_eq!(ov.len(), 1);
        assign(&mut ov, "tool:pen", Some("P"), false).unwrap();
        assert!(ov.is_empty());
    }

    #[test]
    fn tool_shortcuts_must_be_single_keys() {
        let mut ov = Overrides::new();
        assert!(assign(&mut ov, "tool:pen", Some("Cmd+Alt+Shift+F9"), true).is_err());
        assert!(assign(&mut ov, "tool:bogus", Some("Q"), true).is_err());
    }

    #[test]
    fn export_import_round_trip() {
        let mut ov = Overrides::new();
        crate::keymap::set_override(&mut ov, "tool:pen", "Shift+9");
        crate::keymap::set_override(&mut ov, "edit.preferences", "");
        let v = export_json("Mine", &ov);
        let (set, back) = import_json(&v).unwrap();
        assert_eq!(set, "Mine");
        assert_eq!(back, ov);
        assert!(import_json(&json!({"overrides": {"x": "Cmd+Nope+Q"}})).is_err());
        assert!(import_json(&json!({})).is_err());
    }

    #[test]
    fn presets_are_the_defaults() {
        for p in PRESETS {
            assert_eq!(preset(p), Some(Overrides::new()));
        }
        assert!(preset("Nope").is_none());
    }

    #[test]
    fn entries_cover_tools_and_menu_commands() {
        assert!(entries().iter().any(|e| e.key == "tool:selection"));
        assert!(entries().iter().any(|e| e.key == "edit.preferences"));
        assert!(entries().iter().any(|e| e.key == "edit.undo"));
        let mut seen = HashSet::new();
        assert!(entries().iter().all(|e| seen.insert(e.key.clone())), "duplicate entries");
    }

    #[test]
    fn duplicate_can_be_given_a_shortcut() {
        // Duplicate has no default chord (#1539): it must still be listed so one can be set.
        assert_eq!(entry("edit.duplicate").map(|e| e.group.as_str()), Some("Edit"));
        let mut ov = Overrides::new();
        assign(&mut ov, "edit.duplicate", Some("Cmd+Alt+Shift+F8"), false).unwrap();
        assert_eq!(effective_in(&ov, "edit.duplicate").as_deref(), Some("Cmd+Alt+Shift+F8"));
    }

    /// #1539: a command outside the menus that runs without params (a panel menu's Break Link to
    /// Symbol) can be given a shortcut; one that needs params, a query or an alias can't.
    #[test]
    fn menuless_commands_that_run_bare_are_listed() {
        let e = entry("symbol.breakLink").expect("listed");
        assert_eq!((e.label.as_str(), e.group.as_str()), ("Break Link to Symbol", "Symbol"));
        for id in ["brush.duplicate", "path.convertAnchors", "document.json", "prefs.list"] {
            assert!(entry(id).is_none(), "{id}");
        }
        for (doc, bare) in
            [("{}", true), ("{dx?, dy?} x", true), ("{id?|ids?, clip?}", true), ("{name, newName?}", false), ("{a?: {b}}", false), ("same as", false)]
        {
            assert_eq!(runs_without_params(doc), bare, "{doc}");
        }
        let mut ov = Overrides::new();
        assert!(assign(&mut ov, "symbol.breakLink", Some("Cmd+Alt+Shift+F9"), true).is_ok());
        assert_eq!(effective_in(&ov, "symbol.breakLink").as_deref(), Some("Cmd+Alt+Shift+F9"));
    }

    /// #2991: Enter Isolation Mode has no default key but runs on the selection, so it belongs in
    /// Edit › Keyboard Shortcuts.
    #[test]
    fn enter_isolation_mode_command_is_listed_and_assignable() {
        let e = entry("object.isolate").expect("listed");
        assert_eq!((e.label.as_str(), e.group.as_str(), e.is_tool), ("Enter Isolation Mode", "Object", false));
        assert_eq!(default_of("object.isolate"), None);
        let mut ov = Overrides::new();
        assign(&mut ov, "object.isolate", Some("Cmd+Alt+Shift+F11"), false).unwrap();
        assert_eq!(effective_in(&ov, "object.isolate").as_deref(), Some("Cmd+Alt+Shift+F11"));
    }
}
