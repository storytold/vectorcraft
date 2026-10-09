//! Edit → Keyboard Shortcuts: user overrides of tool and command shortcuts, conflict detection,
//! presets, import/export, and the dialog.
//!
//! Overrides live in [`UiState::shortcut_overrides`] (persisted) keyed by command id or
//! `tool:<id>`; an empty string removes a shortcut. A process-wide mirror (synced every frame by
//! [`sync`]) lets `menus::shortcut_of`, the egui dispatcher, the native menu and tooltips consult
//! them without threading the app state through.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

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

ui_mirror!(fn store() -> BTreeMap<String, &'static str>);

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
        .map(|m| m.len() == ui.shortcut_overrides.len() && m.iter().all(|(k, v)| ui.shortcut_overrides.get(k).map(String::as_str) == Some(*v)))
        .unwrap_or(false);
    if !same {
        if let Ok(mut m) = store().write() {
            *m = ui.shortcut_overrides.iter().map(|(k, v)| (k.clone(), intern(v))).collect();
        }
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    crate::workspaces::sync(ui);
}

fn global_override(key: &str) -> Option<&'static str> {
    store().read().ok().and_then(|m| m.get(key).copied())
}

/// Registry default shortcut of a command (engine or UI command).
pub fn default_command_shortcut(id: &str) -> Option<&'static str> {
    if let Some(c) = vectorcraft_engine::find_command(id) {
        return c.shortcut;
    }
    menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.2).filter(|s| !s.is_empty())
}

/// Window menu items that show a panel (`window.panel {panel}`) with a default shortcut: (panel
/// id, shortcut). Every icon panel can be given one in the editor (entry key `panel:<id>`).
pub const PANEL_SHORTCUTS: &[(&str, &str)] = &[
    ("color", "F6"),
    ("colorGuide", "Shift+F3"),
    ("appearance", "Shift+F6"),
    ("graphicStyles", "Shift+F5"),
    ("stroke", "Cmd+F10"),
    ("gradient", "Cmd+F9"),
    ("transparency", "Cmd+Shift+F10"),
    ("attributes", "Cmd+F11"),
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
    match global_override(key) {
        Some("") => None,
        Some(s) => Some(s),
        None => default_of(key),
    }
}

/// Effective shortcut of `key` under an explicit override map: override > default; "" = none.
pub fn effective_in(overrides: &BTreeMap<String, String>, key: &str) -> Option<String> {
    match overrides.get(key) {
        Some(s) if s.is_empty() => None,
        Some(s) => Some(s.clone()),
        None => default_of(key).map(str::to_string),
    }
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
    let n = normalize(key)?;
    vectorcraft_tools::catalog::all_tools().find(|t| tool_shortcut(t.id).and_then(normalize).as_deref() == Some(n.as_str())).map(|t| t.id)
}

/// Command whose effective shortcut is exactly `key` (for single-key command shortcuts: X, D, /).
pub fn command_for_key(key: &str) -> Option<&'static str> {
    let n = normalize(key)?;
    let hit = |id: &'static str| command_shortcut(id).and_then(normalize).as_deref() == Some(n.as_str());
    vectorcraft_engine::command_specs().iter().map(|c| c.id).chain(menus::UI_COMMANDS.iter().map(|c| c.0)).find(|id| hit(id))
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
    crate::shortcuts::parse(s).map(|sc| format(&sc))
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
            if (c.menu.is_empty() && c.shortcut.is_none()) || !seen.insert(c.id) {
                continue;
            }
            v.push(Entry { key: c.id.into(), label: c.label.into(), group: c.menu.join(" › "), is_tool: false });
        }
        v.extend(crate::state::ICON_PANELS.iter().map(|(id, label, _)| Entry {
            key: format!("panel:{id}"),
            label: label.to_string(),
            group: "Window".into(),
            is_tool: false,
        }));
        for (id, label, sc, params) in menus::UI_COMMANDS {
            if !seen.insert(id) || (sc.is_empty() && !params.starts_with("{}") && !params.starts_with("{path")) {
                continue;
            }
            let group = id.split('.').next().map(|g| {
                let mut c = g.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            });
            v.push(Entry { key: id.to_string(), label: label.to_string(), group: group.unwrap_or_default(), is_tool: false });
        }
        v
    })
}

pub fn entry(key: &str) -> Option<&'static Entry> {
    entries().iter().find(|e| e.key == key)
}

/// Entries (other than `except`) whose effective shortcut equals `chord`.
pub fn conflicts_with(overrides: &BTreeMap<String, String>, chord: &str, except: &str) -> Vec<String> {
    let Some(n) = normalize(chord) else { return vec![] };
    entries()
        .iter()
        .filter(|e| e.key != except)
        .filter(|e| effective_in(overrides, &e.key).and_then(|s| normalize(&s)).as_deref() == Some(n.as_str()))
        .map(|e| e.key.clone())
        .collect()
}

/// All chords used by more than one entry: (chord, keys).
pub fn all_conflicts(overrides: &BTreeMap<String, String>) -> Vec<(String, Vec<String>)> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in entries() {
        if let Some(n) = effective_in(overrides, &e.key).and_then(|s| normalize(&s)) {
            by.entry(n).or_default().push(e.key.clone());
        }
    }
    by.into_iter().filter(|(_, v)| v.len() > 1).collect()
}

/// Assign `chord` (None = remove, "default" handled by callers via [`reset_one`]) to `key` in
/// `overrides`. Conflicting entries lose the chord when `force`, otherwise it's an error.
pub fn assign(overrides: &mut BTreeMap<String, String>, key: &str, chord: Option<&str>, force: bool) -> Result<Vec<String>, String> {
    if entry(key).is_none() && default_of(key).is_none() {
        return Err(format!("unknown tool or command `{key}`"));
    }
    let chord = match chord {
        None | Some("") => {
            set_or_clear(overrides, key, "");
            return Ok(vec![]);
        }
        Some(c) => normalize(c).ok_or_else(|| format!("can't parse shortcut `{c}`"))?,
    };
    if key.starts_with("tool:") {
        let sc = crate::shortcuts::parse(&chord).ok_or("bad shortcut")?;
        if sc.modifiers.command || sc.modifiers.alt || sc.modifiers.ctrl {
            return Err("tool shortcuts must be a single key (optionally with Shift)".into());
        }
    }
    let clashes = conflicts_with(overrides, &chord, key);
    if !clashes.is_empty() && !force {
        let names: Vec<String> = clashes.iter().map(|k| entry(k).map(|e| e.label.clone()).unwrap_or(k.clone())).collect();
        return Err(format!("{} is already used by {}", menus::pretty_shortcut(&chord), names.join(", ")));
    }
    for c in &clashes {
        set_or_clear(overrides, c, "");
    }
    set_or_clear(overrides, key, &chord);
    Ok(clashes)
}

/// Store `value` for `key`, dropping the override when it equals the default.
fn set_or_clear(overrides: &mut BTreeMap<String, String>, key: &str, value: &str) {
    let default = default_of(key).and_then(normalize).unwrap_or_default();
    let v = if value.is_empty() { String::new() } else { normalize(value).unwrap_or(value.to_string()) };
    if v == default {
        overrides.remove(key);
    } else {
        overrides.insert(key.to_string(), v);
    }
}

pub fn reset_one(overrides: &mut BTreeMap<String, String>, key: &str) {
    overrides.remove(key);
}

/// Overrides of a preset (both presets are the registry defaults for now).
pub fn preset(name: &str) -> Option<BTreeMap<String, String>> {
    PRESETS.contains(&set_name(name)).then(BTreeMap::new)
}

/// Export format: `{"format": "vectorcraft-shortcuts", "set": name, "overrides": {...}}`.
pub fn export_json(set: &str, overrides: &BTreeMap<String, String>) -> Value {
    json!({"format": "vectorcraft-shortcuts", "version": 1, "set": set, "overrides": overrides})
}

pub fn import_json(v: &Value) -> Result<(String, BTreeMap<String, String>), String> {
    let o = v.get("overrides").and_then(Value::as_object).ok_or("not a Vector W3K2 shortcut set (missing `overrides`)")?;
    let mut out = BTreeMap::new();
    for (k, v) in o {
        let s = v.as_str().ok_or_else(|| format!("shortcut for `{k}` must be a string"))?;
        if !s.is_empty() && normalize(s).is_none() {
            return Err(format!("can't parse shortcut `{s}` for `{k}`"));
        }
        out.insert(k.clone(), normalize(s).unwrap_or_default());
    }
    Ok((v.get("set").and_then(Value::as_str).map_or(CUSTOM, set_name).to_string(), out))
}

// ---------- UI commands ----------

pub fn open(app: &mut VectorcraftApp) {
    let ov = serde_json::to_value(&app.ui.shortcut_overrides).unwrap_or(json!({}));
    app.ui.dialog = Some(Dialog::new(
        "shortcuts",
        json!({"tab": "tools", "query": "", "set": app.ui.shortcut_set, "overrides": ov, "__recording": "", "__selected": "", "__message": "", "__conflict": ""}),
    ));
}

fn dialog_overrides(d: &Dialog) -> BTreeMap<String, String> {
    d.fields.get("overrides").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
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
            let ov = &app.ui.shortcut_overrides;
            Ok(Value::Array(
                entries()
                    .iter()
                    .filter(|e| q.is_empty() || e.label.to_lowercase().contains(&q) || e.key.to_lowercase().contains(&q))
                    .map(|e| json!({"id": e.key, "label": e.label, "group": e.group, "shortcut": effective_in(ov, &e.key), "default": default_of(&e.key), "overridden": ov.contains_key(&e.key)}))
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
            let path = s("path").or_else(|| app.services.pick_save.as_mut().and_then(|f| f(&crate::FilePick::named("VectorCraft Shortcuts.json"))));
            match path {
                Some(path) => match app.services.write.as_mut() {
                    Some(w) => w(&path, &bytes).map(|_| json!({"path": path})),
                    None => Err("no file writer".into()),
                },
                None => match app.services.download.as_mut() {
                    Some(dl) => {
                        dl("VectorCraft Shortcuts.json", &bytes);
                        Ok(Value::Null)
                    }
                    None => Ok(v),
                },
            }
        }
        "shortcuts.import" => {
            let data = if let Some(v) = p.get("data") {
                Ok(v.clone())
            } else {
                let path = s("path").or_else(|| app.services.pick_open.as_mut().and_then(|f| f(&crate::FilePick::default())));
                match (path, app.services.read.as_ref()) {
                    (Some(path), Some(r)) => r(&path).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())),
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
                    d.fields.insert("overrides".into(), serde_json::to_value(&app.ui.shortcut_overrides).unwrap_or(json!({})));
                    d.fields.insert("set".into(), json!(app.ui.shortcut_set));
                }
                json!({"overrides": n})
            })
        }
        _ => return None,
    };
    Some(r)
}

/// While recording, Escape cancels the recording instead of closing the dialog.
pub fn is_recording(app: &VectorcraftApp) -> bool {
    app.ui.dialog.as_ref().is_some_and(|d| d.kind == "shortcuts" && !d.str("__recording").is_empty())
}

// ---------- dialog ----------

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let mut ov = dialog_overrides(&d);
    let mut ok = false;
    let mut cancel = false;
    let recording = d.str("__recording");
    // Record a chord (before widgets see the keys).
    if !recording.is_empty() {
        let ev = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = ev {
            ctx.input_mut(|i| i.events.clear());
            if key == Key::Escape {
                d.fields.insert("__recording".into(), json!(""));
            } else if let Some(chord) = chord_from_event(key, m) {
                record(&mut d, &mut ov, &recording, &chord);
            }
        }
    }
    egui::Area::new(egui::Id::new("modal-dim")).order(egui::Order::Middle).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
        ui.allocate_rect(ctx.content_rect(), egui::Sense::click());
    });
    egui::Window::new(tl!("Keyboard Shortcuts"))
        .id(egui::Id::new("dialog-shortcuts"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() + egui::vec2(0.0, -20.0))
        .constrain(true)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(20)))
        .show(ctx, |ui| {
            ui.set_width(640.0);
            ui.label(egui::RichText::new(tl!("Keyboard Shortcuts")).font(theme::semibold(16.0)).color(t.text));
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
                    ov = p;
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
                    ov.clear();
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
                    for e in entries().iter().filter(|e| e.is_tool == tools) {
                        let sc = effective_in(&ov, &e.key);
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
                            let changed = ov.contains_key(&e.key);
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
                        let _ = assign(&mut ov, &sel, None, true);
                        d.fields.insert("set".into(), json!(CUSTOM));
                    }
                    if ui.button(tl!("Use Default")).clicked() {
                        reset_one(&mut ov, &sel);
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
            let n = all_conflicts(&ov).len();
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
    d.fields.insert("overrides".into(), serde_json::to_value(&ov).unwrap_or(json!({})));
    let import = d.fields.remove("__import").is_some();
    let export = d.fields.remove("__export").is_some();
    app.ui.dialog = Some(d);
    if import {
        // Import replaces the working set (committed on OK like other edits).
        let saved = (app.ui.shortcut_overrides.clone(), app.ui.shortcut_set.clone());
        match run_command(app, "shortcuts.import", &json!({})) {
            Some(Ok(_)) => {
                let (o, s) = (app.ui.shortcut_overrides.clone(), app.ui.shortcut_set.clone());
                (app.ui.shortcut_overrides, app.ui.shortcut_set) = saved;
                sync(&app.ui);
                if let Some(d) = app.ui.dialog.as_mut() {
                    d.fields.insert("overrides".into(), serde_json::to_value(&o).unwrap_or(json!({})));
                    d.fields.insert("set".into(), json!(s));
                    d.fields.insert("__message".into(), json!(tl!("Imported. Press OK to keep the imported set.")));
                }
            }
            Some(Err(e)) if e != "cancelled" => app.status(e),
            _ => {}
        }
    }
    if export {
        let (ov, set) = app.ui.dialog.as_ref().map(|d| (dialog_overrides(d), d.str("set"))).unwrap_or_default();
        let saved = std::mem::replace(&mut app.ui.shortcut_overrides, ov);
        let saved_set = std::mem::replace(&mut app.ui.shortcut_set, set);
        if let Some(Err(e)) = run_command(app, "shortcuts.export", &json!({})) {
            app.status(e);
        }
        app.ui.shortcut_overrides = saved;
        app.ui.shortcut_set = saved_set;
    }
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

/// Apply a recorded chord to `key` in the working set, warning about (and resolving) conflicts.
fn record(d: &mut Dialog, ov: &mut BTreeMap<String, String>, key: &str, chord: &str) {
    d.fields.insert("__recording".into(), json!(""));
    match assign(ov, key, Some(chord), true) {
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
    }

    #[test]
    fn override_beats_default_and_empty_removes() {
        let mut ov = BTreeMap::new();
        assert_eq!(effective_in(&ov, "edit.preferences").as_deref(), Some("Cmd+K"));
        ov.insert("edit.preferences".to_string(), "Cmd+Alt+Shift+9".to_string());
        assert_eq!(effective_in(&ov, "edit.preferences").as_deref(), Some("Cmd+Alt+Shift+9"));
        ov.insert("edit.preferences".to_string(), String::new());
        assert_eq!(effective_in(&ov, "edit.preferences"), None);
        assert_eq!(effective_in(&BTreeMap::new(), "tool:pen").as_deref(), Some("P"));
    }

    #[test]
    fn global_override_reaches_shortcut_of_and_tools() {
        let mut ui = UiState::default();
        ui.shortcut_overrides.insert("test.nonexistent".into(), "Cmd+Alt+Shift+F9".into());
        ui.shortcut_overrides.insert("tool:measure".into(), "Shift+9".into());
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
        ui.shortcut_overrides.insert("tool:measure".into(), "Shift+8".into());
        crate::workspaces::save_as(&mut ui, "Mine").unwrap();
        sync(&ui);
        // What each frame of a test running in parallel does with its default state.
        std::thread::spawn(|| sync(&UiState::default())).join().unwrap();
        assert_eq!(tool_shortcut("measure"), Some("Shift+8"));
        assert!(crate::workspaces::menu_items().iter().any(|i| matches!(i, menus::Item::Cmd("Mine", ..))));
    }

    #[test]
    fn assign_detects_conflicts() {
        let mut ov = BTreeMap::new();
        // ⌘K is Preferences.
        let err = assign(&mut ov, "file.revert", Some("Cmd+K"), false).unwrap_err();
        assert!(err.contains("Preferences"), "{err}");
        assert!(ov.is_empty());
        assert_eq!(conflicts_with(&ov, "Cmd+K", "file.revert"), vec!["edit.preferences".to_string()]);
    }

    #[test]
    fn forced_assign_moves_the_chord() {
        let mut ov = BTreeMap::new();
        let removed = assign(&mut ov, "file.revert", Some("Cmd+K"), true).unwrap();
        assert_eq!(removed, vec!["edit.preferences".to_string()]);
        assert_eq!(effective_in(&ov, "file.revert").as_deref(), Some("Cmd+K"));
        assert_eq!(effective_in(&ov, "edit.preferences"), None);
        assert!(conflicts_with(&ov, "Cmd+K", "file.revert").is_empty());
    }

    #[test]
    fn assigning_the_default_drops_the_override() {
        let mut ov = BTreeMap::new();
        assign(&mut ov, "tool:pen", Some("Shift+9"), false).unwrap();
        assert_eq!(ov.len(), 1);
        assign(&mut ov, "tool:pen", Some("P"), false).unwrap();
        assert!(ov.is_empty());
    }

    #[test]
    fn tool_shortcuts_must_be_single_keys() {
        let mut ov = BTreeMap::new();
        assert!(assign(&mut ov, "tool:pen", Some("Cmd+Alt+Shift+F9"), true).is_err());
        assert!(assign(&mut ov, "tool:bogus", Some("Q"), true).is_err());
    }

    #[test]
    fn export_import_round_trip() {
        let mut ov = BTreeMap::new();
        ov.insert("tool:pen".to_string(), "Shift+9".to_string());
        ov.insert("edit.preferences".to_string(), String::new());
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
            assert_eq!(preset(p), Some(BTreeMap::new()));
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
}
