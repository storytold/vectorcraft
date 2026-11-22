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
    /// The overrides this was built from, as the UI state holds them.
    source: Overrides,
    /// The plug-in revision of the entries the keymap holds ([`Snapshot::revision`]).
    revision: u64,
}

impl Mirror {
    fn new(overrides: Overrides) -> Mirror {
        let snapshot = snapshot();
        let keymap = crate::keymap::keymap_in(&snapshot, &overrides);
        let text = overrides
            .iter()
            .map(|(k, _)| {
                let key = entry_of_stored(k);
                let t = effective_with(&keymap, &key).map(|t| intern(&t));
                (key, t)
            })
            .collect();
        Mirror { keymap, text, source: overrides, revision: snapshot.revision }
    }

    /// Was this built from `overrides` for the platform and the plug-ins in effect now?
    fn is_for(&self, overrides: &Overrides) -> bool {
        self.source == *overrides && self.revision == menus::plugin_revision() && self.keymap.platform() == crate::keymap::platform()
    }
}

ui_mirror!(fn store() -> Option<Arc<Mirror>>);

/// The mirror: the keymap in effect. One the plug-ins installed or removed since it was built
/// left behind is built again, with the overrides it was built from.
pub(crate) fn current() -> Arc<Mirror> {
    static DEFAULT: Mutex<Option<Arc<Mirror>>> = Mutex::new(None);
    if let Some(m) = store().read().ok().and_then(|m| m.clone()) {
        if m.is_for(&m.source) {
            return m;
        }
        let fresh = Arc::new(Mirror::new(m.source.clone()));
        if let Ok(mut s) = store().write()
            && s.as_ref().is_some_and(|s| Arc::ptr_eq(s, &m))
        {
            *s = Some(fresh.clone());
        }
        return fresh;
    }
    let none = Overrides::new();
    let cached = DEFAULT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(m) = cached.filter(|m| m.is_for(&none)) {
        return m;
    }
    let fresh = Arc::new(Mirror::new(none));
    *DEFAULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(fresh.clone());
    fresh
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
            Some(m) => m.is_for(&ui.shortcut_overrides),
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
    crate::custom_toolbars::sync(ui);
}

/// Registry default shortcut of a command (engine or UI command) on [`crate::keymap::platform`].
pub fn default_command_shortcut(id: &str) -> Option<&'static str> {
    default_command_shortcut_on(id, crate::keymap::platform())
}

/// Registry default shortcut of a command on platform `p`.
fn default_command_shortcut_on(id: &str, p: craft_keymap::Platform) -> Option<&'static str> {
    if let Some(c) = vectorcraft_engine::find_command(id) {
        return c.shortcut;
    }
    menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| ui_command_default(c.0, c.2, p)).filter(|s| !s.is_empty())
}

/// The default shortcut `sc` of UI command `id` on platform `p` (empty for none). Select Last
/// Selection Tool has none on macOS, which takes Cmd+` to switch between an app's windows.
pub(crate) fn ui_command_default(id: &str, sc: &'static str, p: craft_keymap::Platform) -> &'static str {
    if id == "tool.selectLastSelection" && p.is_mac() { "" } else { sc }
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

/// Default shortcut of an entry key (`tool:<id>`, `panel:<id>` or a command id) on
/// [`crate::keymap::platform`].
pub fn default_of(key: &str) -> Option<&'static str> {
    default_on(key, crate::keymap::platform())
}

/// Default shortcut of an entry key on platform `p`.
pub(crate) fn default_on(key: &str, p: craft_keymap::Platform) -> Option<&'static str> {
    if let Some(t) = key.strip_prefix("tool:") {
        return vectorcraft_tools::tool_info(t).and_then(|t| t.shortcut);
    }
    if let Some(panel) = key.strip_prefix("panel:") {
        return PANEL_SHORTCUTS.iter().find(|(id, _)| *id == panel).map(|(_, sc)| *sc);
    }
    default_command_shortcut_on(key, p)
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
        .chain(vectorcraft_engine::cmd::plugin::command_specs())
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

ui_mirror!(fn backtick_store() -> Option<String>);

/// Notes what the backtick key (left of 1) typed in `events`: the last one-character text that
/// [`crate::shortcuts::typing_presses`] pairs with a press of that key without modifiers, whether
/// the press comes before its text (egui-winit) or after it (eframe's web build with no text field
/// focused). egui reports that key as [`Key::Backtick`] on layouts where it types a character egui
/// has no key for, such as `"` on Turkish Q.
pub(crate) fn note_backtick_text(events: &[egui::Event]) {
    let typed_by = crate::shortcuts::typing_presses(events);
    let typed = events.iter().zip(typed_by).rev().find_map(|(e, by)| match (e, by.and_then(|k| events.get(k))) {
        (egui::Event::Text(t), Some(egui::Event::Key { key: Key::Backtick, modifiers, .. }))
            if modifiers.is_none() && t.chars().count() == 1 && !t.chars().any(char::is_whitespace) =>
        {
            Some(t.clone())
        }
        _ => None,
    });
    if let Some(t) = typed
        && let Ok(mut store) = backtick_store().write()
        && store.as_deref() != Some(t.as_str())
    {
        *store = Some(t);
    }
}

/// The character the backtick key types on the layout in use, once a press of it has been seen,
/// when it is neither `` ` `` nor `~`.
pub(crate) fn backtick_text() -> Option<String> {
    backtick_store().read().ok().and_then(|t| t.clone()).filter(|t| t != "`" && t != "~")
}

/// `text` as the editor shows it: an unshifted backtick key shows the character it types on the
/// layout in use (`"` on Turkish Q), so it reads as the key the user pressed.
fn on_layout(text: String, typed: Option<&str>) -> String {
    let Some(typed) = typed else { return text };
    match vc_chord(&text) {
        Some(c) if c.key == craft_keymap::Key::Backtick && !c.mods.shift => match text.strip_suffix('~') {
            Some(mods) => format!("{mods}{typed}"),
            None => text,
        },
        _ => text,
    }
}

/// `text` with the backtick key shown as the character it types on the layout in use
/// ([`backtick_text`]), as the editor shows it.
pub(crate) fn as_typed(text: &str) -> String {
    on_layout(text.to_string(), backtick_text().as_deref())
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
    /// Keyboard Shortcuts adds the key to the entry's name: another entry has the same group and
    /// label ([`distinct_names`]).
    pub shows_key: bool,
}

/// The editor's entries, the Effect menu's effect items and Object › Plug-ins' filter items while
/// the installed plug-ins are those of one [`menus::plugin_revision`].
pub(crate) struct Snapshot {
    /// The plug-in revision read before the menus were walked.
    pub revision: u64,
    pub effects: Arc<[EffectItem]>,
    pub filters: Arc<[FilterItem]>,
    pub entries: Arc<[Entry]>,
}

/// The entries for the plug-ins installed now: rebuilt when a plug-in is installed or removed, so
/// a live-effect or filter plug-in installed later has its row, and one removed has none.
pub(crate) fn snapshot() -> Arc<Snapshot> {
    static LAST: Mutex<Option<Arc<Snapshot>>> = Mutex::new(None);
    let mut revision = menus::plugin_revision();
    let last = LAST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // A test's walk hook runs even when a snapshot of this revision is already stored.
    #[cfg(test)]
    let hooked = {
        let f = tests::WHILE_WALKING.take();
        let hooked = f.is_some();
        tests::WHILE_WALKING.set(f);
        hooked
    };
    #[cfg(not(test))]
    let hooked = false;
    if let Some(s) = last.filter(|s| s.revision == revision && !hooked) {
        return s;
    }
    // When a plug-in is installed or removed during the walk, the entries may not match
    // `revision`, and the keymap cached for it would lack rows or hold extra ones. The menus are
    // walked again at the new revision, and a snapshot that still doesn't match is not cached.
    let mut s = build_snapshot(revision);
    for _ in 0..8 {
        let now = menus::plugin_revision();
        if now == revision {
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            // A newer snapshot another thread stored meanwhile stays.
            if last.as_ref().is_none_or(|l| l.revision <= revision) {
                *last = Some(s.clone());
            }
            return s;
        }
        revision = now;
        s = build_snapshot(revision);
    }
    s
}

/// The snapshot of the menus as they are now, under plug-in revision `revision`.
fn build_snapshot(revision: u64) -> Arc<Snapshot> {
    // Built without the lock held: the Effect menu is walked anew.
    #[cfg(test)]
    if let Some(f) = tests::WHILE_WALKING.take() {
        f();
    }
    let effects: Arc<[EffectItem]> = build_effect_items().into();
    let filters: Arc<[FilterItem]> = build_filter_items().into();
    let (head, tail) = fixed_entries();
    let mut entries: Vec<Entry> = head
        .iter()
        .cloned()
        .chain(effects.iter().map(|e| Entry {
            key: format!("effect:{}", e.effect),
            label: e.label.clone(),
            group: e.group.clone(),
            is_tool: false,
            shows_key: false,
        }))
        .chain(filters.iter().map(|f| Entry {
            key: format!("plugin:{}", f.id),
            label: f.label.clone(),
            group: f.group.clone(),
            is_tool: false,
            shows_key: false,
        }))
        .chain(vectorcraft_engine::cmd::plugin::command_specs().into_iter().map(|c| Entry {
            key: c.id.into(),
            label: c.label.into(),
            group: c.menu.join(" › "),
            is_tool: false,
            shows_key: false,
        }))
        .chain(tail.iter().cloned())
        .collect();
    distinct_names(&mut entries);
    Arc::new(Snapshot { revision, effects, filters, entries: entries.into() })
}

/// Every tool, command and panel the editor can give a shortcut; [`listed`] says which it lists.
pub fn entries() -> Arc<[Entry]> {
    snapshot().entries.clone()
}

/// The entries no plug-in changes: (tools, commands and panels; UI commands). The Effect menu's
/// items, Object › Plug-ins' filters and the plug-in commands go between them.
fn fixed_entries() -> &'static (Vec<Entry>, Vec<Entry>) {
    static E: OnceLock<(Vec<Entry>, Vec<Entry>)> = OnceLock::new();
    E.get_or_init(|| {
        let mut v: Vec<Entry> = vectorcraft_tools::catalog::all_tools()
            .map(|t| Entry { key: format!("tool:{}", t.id), label: t.label.to_string(), group: "Tools".into(), is_tool: true, shows_key: false })
            .collect();
        let mut seen = HashSet::new();
        // The commands listed only because an item of the menu bar runs them as a key press would
        // (Set Corners…).
        let mut by_the_menu_bar = HashSet::new();
        for c in vectorcraft_engine::command_specs() {
            let own = listed_on_its_own(c);
            if !(own || a_menu_bar_item_runs(c.id, c.params)) || !seen.insert(c.id) {
                continue;
            }
            if !own {
                by_the_menu_bar.insert(c.id);
            }
            let group = if c.menu.is_empty() { id_group(c.id) } else { c.menu.join(" › ") };
            v.push(Entry { key: c.id.into(), label: c.label.into(), group, is_tool: false, shows_key: false });
        }
        v.extend(crate::state::all_panels().map(|(id, label)| Entry {
            key: format!("panel:{id}"),
            label: label.to_string(),
            group: "Window".into(),
            is_tool: false,
            shows_key: false,
        }));
        // Each effect of the Effect menu comes next, by its own key (#3296): see [`snapshot`].
        let mut tail = vec![];
        for (id, label, sc, params) in menus::UI_COMMANDS {
            let own = ui_listed_on_its_own(id, sc, params);
            if !seen.insert(id) || !(own || a_menu_bar_item_runs(id, params)) {
                continue;
            }
            if !own {
                by_the_menu_bar.insert(id);
            }
            tail.push(Entry { key: id.to_string(), label: label.to_string(), group: id_group(id), is_tool: false, shows_key: false });
        }
        // A command listed only because an item of the menu bar runs it can have the group and
        // label of another entry. When that entry is an engine command that no item of the menu bar
        // runs, the command the item runs keeps the name and the other entry is shown in the group
        // of its id: File › Export Selection… runs `file.exportSelection`, as the menu item does,
        // and `document.exportSelection` shows as Document › Export Selection…. Otherwise the
        // command is left out: Window › Learn stays the Learn panel's entry. Entries that still
        // have the same name, plug-ins included, are given names of their own by
        // [`distinct_names`].
        let name = |e: &Entry| (e.group.clone(), e.label.clone());
        let new_names: HashSet<_> = v.iter().chain(&tail).filter(|e| by_the_menu_bar.contains(e.key.as_str())).map(name).collect();
        for e in v.iter_mut().chain(tail.iter_mut()) {
            if new_names.contains(&name(e))
                && !by_the_menu_bar.contains(e.key.as_str())
                && vectorcraft_engine::find_command(&e.key).is_some()
                && !menu_bar_items().contains_key(e.key.as_str())
            {
                e.group = id_group(&e.key);
            }
        }
        let names: HashSet<_> = v.iter().chain(&tail).filter(|e| !by_the_menu_bar.contains(e.key.as_str())).map(name).collect();
        let keep = |e: &Entry| !by_the_menu_bar.contains(e.key.as_str()) || !names.contains(&name(e));
        v.retain(keep);
        tail.retain(keep);
        (v, tail)
    })
}

/// One Effect menu item a shortcut can be given (#3296): the effect's id, the command its item runs
/// (`effect.dialog`, or `effect.apply` for an effect without options), its label and its menu path.
#[derive(Clone, Debug)]
pub struct EffectItem {
    pub effect: String,
    pub command: &'static str,
    pub label: String,
    pub group: String,
}

/// The Effect menu's effect items, in menu order, each once, with the live-effect plug-ins
/// installed now.
pub fn effect_items() -> Arc<[EffectItem]> {
    snapshot().effects.clone()
}

/// Walks the Effect menu for [`effect_items`].
fn build_effect_items() -> Vec<EffectItem> {
    fn walk(items: &[menus::Item], path: &mut Vec<String>, seen: &mut HashSet<String>, out: &mut Vec<EffectItem>) {
        for it in items {
            match it {
                menus::Item::Cmd(label, id @ ("effect.dialog" | "effect.apply"), p) => {
                    if let Some(effect) = p.get("effect").and_then(serde_json::Value::as_str)
                        && seen.insert(effect.to_string())
                    {
                        out.push(EffectItem { effect: effect.to_string(), command: id, label: label.to_string(), group: path.join(" › ") });
                    }
                }
                menus::Item::Sub(name, children) => {
                    path.push(name.to_string());
                    walk(children, path, seen, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (name, items) in menus::menu_tree() {
        if name == "Effect" {
            walk(&items, &mut vec![name.to_string()], &mut seen, &mut out);
        }
    }
    out
}

/// Effective shortcut of the Effect menu item of `effect` (`effect:<id>`).
pub fn effect_shortcut(effect: &str) -> Option<&'static str> {
    effective(&format!("effect:{effect}"))
}

/// One filter plug-in's Object › Plug-ins item (`plugin.dialog {id}`) a shortcut can be given: the
/// plug-in's id, its label (the plug-in's name, with an ellipsis when it opens a dialog) and its
/// menu path.
#[derive(Clone, Debug)]
pub struct FilterItem {
    pub id: String,
    pub label: String,
    pub group: String,
}

/// Object › Plug-ins' filter items, in menu order, each once, with the filter plug-ins installed
/// now.
pub fn filter_items() -> Arc<[FilterItem]> {
    snapshot().filters.clone()
}

/// Walks the Object menu for [`filter_items`].
fn build_filter_items() -> Vec<FilterItem> {
    fn walk(items: &[menus::Item], path: &mut Vec<String>, seen: &mut HashSet<String>, out: &mut Vec<FilterItem>) {
        for it in items {
            match it {
                menus::Item::Cmd(label, "plugin.dialog", p) => {
                    if let Some(id) = p.get("id").and_then(serde_json::Value::as_str)
                        && seen.insert(id.to_string())
                    {
                        out.push(FilterItem { id: id.to_string(), label: label.to_string(), group: path.join(" › ") });
                    }
                }
                menus::Item::Sub(name, children) => {
                    path.push(name.to_string());
                    walk(children, path, seen, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (name, items) in menus::menu_tree() {
        if name == "Object" {
            walk(&items, &mut vec![name.to_string()], &mut seen, &mut out);
        }
    }
    out
}

/// Effective shortcut of filter plug-in `id`'s Object › Plug-ins item (`plugin:<id>`).
pub fn filter_shortcut(id: &str) -> Option<&'static str> {
    effective(&format!("plugin:{id}"))
}

/// Entry `e`'s label in the UI language. A plug-in's own name is not translated.
pub(crate) fn shown_label(e: &Entry) -> &str {
    let plugin = e.key.starts_with("plugin:") || e.key.strip_prefix("effect:").and_then(vectorcraft_plugins::effect::plugin_id).is_some();
    if plugin { &e.label } else { tl!(&e.label) }
}

/// A menu path in the UI language, one segment at a time.
pub(crate) fn shown_group(group: &str) -> String {
    group.split(" › ").map(|s| tl!(s)).collect::<Vec<_>>().join(" › ")
}

/// How Keyboard Shortcuts names entry `e`: a tool by its label, the others after their menu, and
/// an entry that [`Entry::shows_key`] with its key after the name.
pub(crate) fn shown_name(e: &Entry) -> String {
    name_of(e, &shown_group(&e.group), shown_label(e))
}

/// [`shown_name`] in English, for a status or error message, which is translated when shown.
fn english_name(e: &Entry) -> String {
    name_of(e, &e.group, &e.label)
}

/// Entry `e`'s name with its group and label as `group` and `label`.
fn name_of(e: &Entry, group: &str, label: &str) -> String {
    let name = if e.is_tool || e.group.is_empty() { label.to_string() } else { format!("{group} › {label}") };
    if e.shows_key { format!("{name} ({})", e.key) } else { name }
}

/// The group of a command no menu holds: the first part of its id, capitalised (`edit.x` → "Edit").
fn id_group(id: &str) -> String {
    let mut c = id.split('.').next().unwrap_or_default().chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Gives entries that have the same group and label names of their own, and leaves the others as
/// they are. It runs on the whole snapshot, plug-ins included, after [`fixed_entries`] has shown an
/// engine command in the group of its id or left out a command listed only because an item of the
/// menu bar runs it:
/// 1. A command that an item of the menu bar runs, as a key press would run it, is shown under
///    that item's menu path ([`menu_bar_path`]): the four Other Library… commands go under Window ›
///    Swatch Libraries, Brush Libraries, Graphic Style Libraries and Symbol Libraries.
/// 2. A command whose id is another command's id with one more part is shown under that command's
///    label: Reset Panel goes under Character Style Options and Paragraph Style Options.
/// 3. Entries that still have the same name show their key after it ([`Entry::shows_key`]), except
///    the one entry an item of the menu bar runs: File › New… stays the New Document dialog's
///    entry, and `file.new` shows as File › New… (file.new). An entry whose name is then the name
///    another entry has with its key (a plug-in named "Blur (plugin:a)") shows its key too.
fn distinct_names(entries: &mut [Entry]) {
    // The entries of each name that more than one entry has.
    fn shared(entries: &[Entry]) -> Vec<Vec<usize>> {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            by_name.entry(english_name(e)).or_default().push(i);
        }
        by_name.into_values().filter(|list| list.len() > 1).collect()
    }
    for i in shared(entries).into_iter().flatten() {
        let Some(e) = entries.get_mut(i).filter(|e| !e.is_tool) else { continue };
        if let Some(path) = menu_bar_path(&e.key) {
            e.group = path.to_string();
        } else if let Some((parent, _)) = e.key.rsplit_once('.').and_then(|(parent, _)| command_doc(parent)) {
            e.group = parent.trim_end_matches('…').to_string();
        }
    }
    // A pass gives every entry of a shared name but one its key, so a name is shared again only by
    // an entry whose label ends with another entry's key; one pass per entry is enough.
    for _ in 0..entries.len() {
        let lists = shared(entries);
        if lists.is_empty() {
            break;
        }
        for list in lists {
            let plain: Vec<usize> = list.iter().copied().filter(|&i| entries.get(i).is_some_and(|e| !e.shows_key)).collect();
            let on_the_menu_bar: Vec<usize> =
                plain.iter().copied().filter(|&i| entries.get(i).is_some_and(|e| !e.is_tool && menu_bar_path(&e.key).is_some())).collect();
            // The entry an item of the menu bar runs keeps the plain name, unless another entry
            // already shows that name with its key.
            let keeps = on_the_menu_bar.first().copied().filter(|_| on_the_menu_bar.len() == 1 && plain.len() == list.len());
            for i in plain {
                if let Some(e) = entries.get_mut(i)
                    && keeps != Some(i)
                {
                    e.shows_key = true;
                }
            }
        }
    }
}

/// The (label, params doc) of engine or UI command `id`.
fn command_doc(id: &str) -> Option<(&'static str, &'static str)> {
    vectorcraft_engine::find_command(id).map(|c| (c.label, c.params)).or_else(|| menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| (c.1, c.3)))
}

/// The (menu path, params) of each item of the menu bar that [`menu_bar_items`] reads, by command
/// id: "Window › Swatch Libraries" for Window › Swatch Libraries › Other Library….
fn menu_bar_paths() -> &'static HashMap<&'static str, Vec<(String, Value)>> {
    fn walk(items: &[menus::Item], path: &mut Vec<String>, out: &mut HashMap<&'static str, Vec<(String, Value)>>) {
        for it in items {
            match it {
                menus::Item::Cmd(label, id, p) if on_the_bar(id, label, p) => out.entry(*id).or_default().push((path.join(" › "), p.clone())),
                menus::Item::Sub(name, children) => {
                    path.push(name.to_string());
                    walk(children, path, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    static PATHS: OnceLock<HashMap<&'static str, Vec<(String, Value)>>> = OnceLock::new();
    PATHS.get_or_init(|| {
        let mut out = HashMap::new();
        for (name, items) in menus::menu_tree() {
            walk(&items, &mut vec![name.to_string()], &mut out);
        }
        out
    })
}

/// The menu path of the item of the menu bar that runs command `id` as a key press runs it
/// ([`a_menu_bar_item_runs`]): the first item that runs it with no params, or else the first item,
/// whose params all its items give.
pub(crate) fn menu_bar_path(id: &str) -> Option<&'static str> {
    let (_, params) = command_doc(id)?;
    let items = menu_bar_paths().get(id).filter(|_| a_menu_bar_item_runs(id, params))?;
    chosen_path(items)
}

/// The menu path [`menu_bar_path`] takes from a command's (menu path, params) items: the first
/// item with no params, or else the first item.
fn chosen_path(items: &[(String, Value)]) -> Option<&str> {
    items.iter().find(|(_, p)| is_bare(p)).or(items.first()).map(|(path, _)| path.as_str())
}

/// Does a command's params doc (`{a?, b?: bool} …`) say it runs with no params, as a shortcut runs
/// it? It does when it starts with `{` and [`names_a_required_field`] finds no field a call must give.
fn runs_without_params(params: &str) -> bool {
    params.starts_with('{') && !names_a_required_field(params)
}

/// Does the editor list `e` under `overrides`? An entry that [`needs_a_value`] is listed only while
/// the user has given it a shortcut.
fn listed(overrides: &Overrides, e: &Entry) -> bool {
    !needs_a_value(&e.key) || crate::keymap::stored_texts(overrides, &e.key).is_some_and(|t| !t.is_empty())
}

/// Commands whose params are all optional that do nothing, or fail, when a key press runs them
/// with none, and that no item of the menu bar runs: Transparency and Opacity Mask Options need
/// a value to set, Align and Distribute a direction, Delete Swatch and Move Swatch a swatch name,
/// and the library, data set, text style and SVG filter loaders a file, which their menu items ask
/// for. The commands after them have no menu path: the setters of panels and dialogs (Fill, Stroke
/// Options, Character, Set Bounds), the exports and saves that give the file back without a `path`
/// (Save Document gives the file back when the document has no file yet; File › Save takes a key
/// instead), the undo group's begin and end, and the commands that need a point, a value or a name
/// (Create Dimension, Edit Freeform Point, Select by Color, Move Brush, Paintbrush, Map Art, Quick Pen). New
/// Symbol is left out because the Symbols panel runs it through the Symbol Options dialog.
const NEEDS_A_PARAM: [&str; 77] = [
    "transparency.set",
    "transparency.setOpacityMask",
    "stroke.set",
    "layer.setProps",
    "object.align",
    "object.distribute",
    "swatch.delete",
    "swatch.move",
    "swatch.spotOptions",
    "edit.colors.adjustBalance",
    "swatch.library.load",
    "graphicStyle.loadLibrary",
    "symbol.loadLibrary",
    "brush.loadLibrary",
    "dataset.import",
    "variable.loadLibrary",
    "charStyle.load",
    "paraStyle.load",
    "textStyle.loadAll",
    "svgFilter.import",
    "addon.pathtools.setAnchor",
    "addon.shapes.configure",
    "appearance.addEffect",
    "artboard.align",
    "artboard.distribute",
    "assets.export",
    "assets.export3D",
    "assets.settings.set",
    "attributes.set",
    "brush.freehand",
    "brush.move",
    "css.export",
    "dimension.create",
    "document.export",
    "document.exportDxf",
    "document.exportEps",
    "document.exportForWeb",
    "document.exportPdf",
    "document.save",
    "document.setup",
    "edit.undoGroup.begin",
    "edit.undoGroup.end",
    "eyedropper.setOptions",
    "file.package",
    "library.createGroup",
    "links.placementOptions",
    "links.unembed",
    "livePaint.gapOptions",
    "object.blend.createSpine",
    "object.blend.setEasing",
    "object.intertwine.arrange",
    "object.objectsOnPath.options",
    "object.puppetWarp.pins",
    "object.setBounds",
    "object.setLiveShape",
    "object.setOverprint",
    "object.setProps",
    "object.slice.select",
    "paint.freeform.setPoint",
    "paint.setFill",
    "paint.setGradientGeom",
    "paint.setStroke",
    "path.quickPen",
    "perspective.grid.define",
    "perspective.grid.set",
    "perspective.plane.move",
    "print.setup",
    "select.color",
    "select.globalEditOptions",
    "stroke.setAdvanced",
    "symbol.new",
    "symbol.options",
    "text.setFormat",
    "text.setRangeStyle",
    "text.setStyle",
    "threeD.mapArt",
    "type.ruby",
];

/// The (label, params) of each menu bar item, by command id.
fn menu_bar_items() -> &'static HashMap<&'static str, Vec<(&'static str, Value)>> {
    static ITEMS: OnceLock<HashMap<&'static str, Vec<(&'static str, Value)>>> = OnceLock::new();
    ITEMS.get_or_init(|| bar_items(&menus::menu_tree()))
}

/// The (label, params) of each item of the menus `tree`, by command id. Items that list an
/// installed plug-in ([`menus::lists_plugin`]) or show a name that is user data
/// ([`menus::shows_a_name`]: a custom toolbar, a script, a recent file, a saved selection) are left
/// out: [`menu_bar_items`] is built once, and such items come and go after that.
fn bar_items(tree: &[(&'static str, Vec<menus::Item>)]) -> HashMap<&'static str, Vec<(&'static str, Value)>> {
    fn walk(items: &[menus::Item], out: &mut HashMap<&'static str, Vec<(&'static str, Value)>>) {
        for it in items {
            match it {
                menus::Item::Cmd(label, id, p) if on_the_bar(id, label, p) => out.entry(*id).or_default().push((*label, p.clone())),
                menus::Item::Sub(_, children) => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for (_, items) in tree {
        walk(items, &mut out);
    }
    out
}

/// Is the menu item with `label` that runs command `id` with params `p` one [`bar_items`] reads: it
/// lists no installed plug-in and shows no name that is user data?
fn on_the_bar(id: &str, label: &str, p: &Value) -> bool {
    !menus::lists_plugin(id, p) && !menus::shows_a_name(id, label)
}

fn is_bare(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

/// What a shortcut without params for command `id` runs when no menu bar item runs `id` without
/// params and every item that runs it gives the same params: what a click on that item runs
/// ([`menus::click_target`]), so Print Ink Fixes runs with its menu item's params and Clean Up…
/// opens its dialog.
pub(crate) fn menu_target(id: &str) -> Option<(String, Value)> {
    let items = menu_bar_items().get(id)?;
    let (label, p) = items.first().filter(|(_, p)| !is_bare(p))?;
    items.iter().all(|(_, q)| q == p).then(|| menus::click_target(label, id, p))
}

/// What a command search pick, or a shortcut without params, of command `id` runs now: `Ok` with
/// what a click on its menu bar item runs ([`menu_target`]), or `None` when no such item gives it
/// params. While that item is dimmed ([`menus::enabled`]), no dialog opens for it: `Err` holds the
/// status line ([`dimmed`]), as for Remove Background… with no image selected.
pub(crate) fn menu_target_now(app: &VectorcraftApp, id: &str) -> Option<Result<(String, Value), String>> {
    let target = menu_target(id)?;
    Some(match dimmed(app, id, id) {
        None => Ok(target),
        Some(why) => Err(why),
    })
}

/// What [`menu_target_now`] gives for command `id` while its menu bar item is dimmed, or `None`
/// while the item is not dimmed: the status line, which calls the command `name` (its id on the
/// status line), with the engine's reason when it gives one. The app's own rules
/// ([`menus::enabled`]) dim UI commands, and some commands the engine would run, without giving a
/// reason. The engine's check runs at most once.
pub(crate) fn dimmed(app: &VectorcraftApp, id: &str, name: &str) -> Option<String> {
    let why = match menus::why_off(app, id)? {
        Some(why) => Some(why),
        // Dimmed before the engine was asked (no Appearance subject, an input method composing).
        None => vectorcraft_engine::find_command(id).and_then(|c| (c.enabled)(&app.session).err()),
    };
    Some(match why {
        Some(why) => vectorcraft_engine::EngineError::Disabled(name.to_string(), why).to_string(),
        None => format!("command `{name}` is not available right now"),
    })
}

/// Is `key` a command with no default shortcut that a key press can't run: no item of the menu
/// bar runs it without params or with one set of params ([`menu_target`]), it opens no dialog when
/// run without them (as the Variables panel's commands do), and it is a query, whose answer a key
/// press drops (Select All Unused), a command of [`NEEDS_A_PARAM`], or a menu command whose params
/// doc names a field a call must give (File › Document Setup › Units, `{units}`)?
pub(crate) fn needs_a_value(key: &str) -> bool {
    static KEYS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    KEYS.get_or_init(|| {
        vectorcraft_engine::command_specs()
            .iter()
            .filter(|c| {
                c.shortcut.is_none()
                    && !a_menu_bar_item_runs(c.id, c.params)
                    && !crate::dialogs::variables::opens(c.id)
                    && (!c.journal || NEEDS_A_PARAM.contains(&c.id) || (!c.menu.is_empty() && names_a_required_field(c.params)))
            })
            .map(|c| c.id)
            .collect()
    })
    .contains(key)
}

/// Does an item of the menu bar run command `id`, whose params doc is `params`, as a key press runs
/// it: with no params, or with the one set of params all its items give ([`menu_target`])? A menu
/// item that runs the command itself with params of its own counts only when the params doc names
/// no required field: Edit › Find Next gives `find` empty, which it rejects.
fn a_menu_bar_item_runs(id: &str, params: &str) -> bool {
    menu_bar_items().get(id).is_some_and(|ps| ps.iter().any(|(_, p)| is_bare(p)))
        || menu_target(id).is_some_and(|(target, _)| target != id || !names_a_required_field(params))
}

/// Is engine command `c` listed in Edit › Keyboard Shortcuts whether or not an item of the menu bar
/// runs it? A command with a menu path of its own is. One without (a panel menu item) is when a key
/// alone can run it: it has a default key, or it is an undoable command, not an alias, whose params
/// doc names no field a call must give ([`runs_without_params`]).
fn listed_on_its_own(c: &vectorcraft_engine::CommandSpec) -> bool {
    !c.menu.is_empty() || c.shortcut.is_some() || (c.journal && !vectorcraft_engine::cmd::is_alias(c.id) && runs_without_params(c.params))
}

/// Is UI command `id` (default key `sc`, params doc `params`) listed in Edit › Keyboard Shortcuts
/// whether or not an item of the menu bar runs it? It is when it has a default key or a params doc
/// of `{}` or `{path…}`. Rename has no default key, and a key press names the selection: it needs
/// no `id`. Color Picker… has no default key either, and a key press opens it for the proxy in
/// front.
fn ui_listed_on_its_own(id: &str, sc: &str, params: &str) -> bool {
    !sc.is_empty() || params.starts_with("{}") || params.starts_with("{path") || id == "ui.rename" || id == "ui.colorPicker"
}

/// Does a menu bar item run the command with no params, so that it does what its label says and opens
/// no dialog? A command no menu bar item runs (the Crop Image item runs `ui.cropImage`), a menu item
/// that gives it params, a [`NEEDS_A_PARAM`] command and a field a call must give each rule it out.
pub(crate) fn runs_with_no_values(c: &vectorcraft_engine::CommandSpec) -> bool {
    menu_bar_items().contains_key(c.id)
        && menu_target(c.id).is_none()
        && !NEEDS_A_PARAM.contains(&c.id)
        && !crate::dialogs::variables::opens(c.id)
        && !names_a_required_field(c.params)
}

/// Does a params doc name a field a call must give? Its fields are the top-level parts of its
/// first `{…}`. A field is optional when it is empty, starts with `…`, gives a default (`rows=10`)
/// or each of its names ends with `?` (`id?|ids?: [id…]`); `{index | indices}` names a required
/// field. A doc that does not start with `{` names none.
fn names_a_required_field(params: &str) -> bool {
    let required = |f: &str| {
        let f = f.trim();
        let end = f.find([':', '=', '(']).unwrap_or(f.len());
        !(f.is_empty() || f.starts_with('…') || f[end..].starts_with('=') || f[..end].split('|').all(|n| n.trim().ends_with('?')))
    };
    let Some(rest) = params.strip_prefix('{') else { return false };
    let mut start = 0;
    for (i, ch, depth) in outside_quotes(rest) {
        match ch {
            '}' if depth == 0 => return required(&rest[start..i]),
            ',' if depth == 0 => {
                if required(&rest[start..i]) {
                    return true;
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    false
}

/// Each character of `text` that is not a double quote or inside a pair of them, with its byte
/// offset and the number of `{`, `[` and `(` groups open before it. A closing `}`, `]` or `)` comes
/// with a count that includes the group it closes, and one with no group open counts none.
pub(crate) fn outside_quotes(text: &str) -> impl Iterator<Item = (usize, char, usize)> + '_ {
    outside_marks(text, &['"'])
}

/// [`outside_quotes`], with a span between backticks skipped as well, as the engine's params object
/// reads it: inside a span opened by a double quote or a backtick, only the same mark closes it.
pub(crate) fn outside_quotes_and_backticks(text: &str) -> impl Iterator<Item = (usize, char, usize)> + '_ {
    outside_marks(text, &['"', '`'])
}

/// Each character of `text` that is not one of `marks` or inside a span a mark opens and the same
/// mark closes, as [`outside_quotes`] counts them.
fn outside_marks<'a>(text: &'a str, marks: &'static [char]) -> impl Iterator<Item = (usize, char, usize)> + 'a {
    let (mut depth, mut quote) = (0usize, None::<char>);
    text.char_indices().filter_map(move |(i, ch)| {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            return None;
        }
        if marks.contains(&ch) {
            quote = Some(ch);
            return None;
        }
        let before = depth;
        match ch {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        Some((i, ch, before))
    })
}

pub fn entry(key: &str) -> Option<Entry> {
    entries().iter().find(|e| e.key == key).cloned()
}

/// Entries (other than `except`) whose effective shortcut equals `chord`.
pub fn conflicts_with(overrides: &Overrides, chord: &str, except: &str) -> Vec<String> {
    let Some(c) = vc_chord(chord) else { return vec![] };
    let snapshot = snapshot();
    let km = crate::keymap::keymap_in(&snapshot, overrides);
    let p = km.platform();
    let want = Trigger::single(c).effective(p);
    snapshot
        .entries
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
    *overrides = saved_overrides(km.overrides(), overrides);
    Ok(removed)
}

/// The overrides to save from a working keymap's `current` overrides. An entry for Cut, Copy or
/// Paste that `source`, the set before the change, did not hold is left out when it is the entry
/// [`crate::keymap::keymap_with`] makes from the other entries of `source` or of `current`.
pub(crate) fn saved_overrides(current: &Overrides, source: &Overrides) -> Overrides {
    let mut saved = current.clone();
    for (key, _, _) in crate::keymap::EDIT_FUNCTION_KEYS {
        let Some(now) = current.get(key) else { continue };
        if source.get(key).is_some() {
            continue;
        }
        let made = |from: &Overrides| {
            let mut rest = from.clone();
            rest.remove(key);
            crate::keymap::edit_alias_overrides(&rest, crate::keymap::platform()).get(key).is_some_and(|t| t == now)
        };
        if made(current) || made(source) {
            saved.remove(key);
        }
    }
    saved
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
                    entry(&k).map_or(k, |e| english_name(&e))
                })
                .collect();
            Err(format!("{} is already used by {}", menus::pretty_shortcut(&vc_format(&chord)), names.join(", ")))
        }
        Err(AssignError::Reserved(_)) => Err(format!("{} is reserved by the system", menus::pretty_shortcut(&vc_format(&chord)))),
        Err(e) => Err(e.to_string()),
    }
}

pub fn reset_one(overrides: &mut Overrides, key: &str) {
    crate::keymap::remove_override(overrides, key);
}

/// Hold shortcuts: entry key → chord text ([`UiState::shortcut_holds`]).
pub type Holds = BTreeMap<String, String>;

/// The chord of hold shortcut text `text`, or why it can't be one. A hold shortcut is a key that
/// types a character, alone or with Shift. Space and the backtick key are refused: the canvas reads
/// them while they are held.
pub fn hold_chord(text: &str) -> Result<craft_keymap::Chord, String> {
    let chord = vc_chord(text).ok_or_else(|| format!("can't parse shortcut `{text}`"))?;
    let shown = menus::pretty_shortcut(&vc_format(&chord));
    if chord.mods.cmd || chord.mods.ctrl || chord.mods.alt {
        return Err(format!("a hold shortcut is a key that types a character, alone or with Shift ({shown} has Cmd, Ctrl or Alt)"));
    }
    if chord.key == craft_keymap::Key::Backtick || !crate::shortcuts::types_text(chord.key) {
        return Err(format!("{shown} can't be a hold shortcut: use a letter, a digit or a punctuation key"));
    }
    Ok(chord)
}

/// The hold shortcut of `key` in `holds`, in VectorCraft's chord spelling, when it is usable.
pub fn hold_of(holds: &Holds, key: &str) -> Option<String> {
    holds.get(key).and_then(|t| hold_chord(t).ok()).map(|c| vc_format(&c))
}

/// The entry whose usable hold shortcut is `chord`: the first in `holds`' order when several are.
/// An entry no longer listed (a removed plug-in's, or one that [`needs_a_value`] whose shortcut
/// was taken away) keeps its text but does not hold.
pub(crate) fn hold_for<'a>(holds: &'a Holds, chord: &craft_keymap::Chord) -> Option<&'a str> {
    let mirror = current();
    holds
        .iter()
        .find(|(k, t)| hold_chord(t).is_ok_and(|c| c == *chord) && entry(k).is_some_and(|e| listed(mirror.keymap.overrides(), &e)))
        .map(|(k, _)| k.as_str())
}

/// Removes the hold shortcuts of the entries the editor does not list under `overrides`: one that
/// [`needs_a_value`] loses its hold with its shortcut. A removed plug-in's entry keeps its hold.
fn drop_unlisted_holds(holds: &mut Holds, overrides: &Overrides) {
    holds.retain(|k, _| entry(k).is_none_or(|e| listed(overrides, &e)));
}

/// Sets (`Some`) or removes (`None` or `""`) the hold shortcut of `key`. Another entry that holds
/// the same key loses it when `force`; otherwise that is an error. Returns the entries that lost
/// it. Nothing changes on an error.
pub fn assign_hold(holds: &mut Holds, key: &str, text: Option<&str>, force: bool) -> Result<Vec<String>, String> {
    if entry(key).is_none() {
        return Err(format!("unknown tool or command `{key}`"));
    }
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        holds.remove(key);
        return Ok(vec![]);
    };
    let chord = hold_chord(text)?;
    let taken: Vec<String> = holds.iter().filter(|(k, t)| *k != key && vc_chord(t) == Some(chord)).map(|(k, _)| k.clone()).collect();
    if !taken.is_empty() && !force {
        let names: Vec<String> = taken.iter().map(|k| entry(k).map_or(k.clone(), |e| english_name(&e))).collect();
        return Err(format!("Hold {} is already used by {}", menus::pretty_shortcut(&vc_format(&chord)), names.join(", ")));
    }
    for k in &taken {
        holds.remove(k);
    }
    holds.insert(key.to_string(), vc_format(&chord));
    Ok(taken)
}

/// Hold shortcuts used by more than one entry: (chord text, entry keys).
pub fn hold_conflicts(holds: &Holds) -> Vec<(String, Vec<String>)> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, t) in holds {
        if let Ok(c) = hold_chord(t) {
            by.entry(vc_format(&c)).or_default().push(k.clone());
        }
    }
    by.into_iter().filter(|(_, keys)| keys.len() > 1).collect()
}

/// The hold shortcuts of an exported set's `holds` member: none when it is missing. Every value
/// must be a usable hold shortcut; otherwise the error names the first entry that is not.
pub fn import_holds(v: &Value) -> Result<Holds, String> {
    let Some(h) = v.get("holds") else { return Ok(Holds::new()) };
    let h = h.as_object().ok_or("`holds` must be an object")?;
    let mut out = Holds::new();
    for (k, v) in h {
        let s = v.as_str().ok_or_else(|| format!("hold shortcut for `{k}` must be a string"))?;
        let c = hold_chord(s).map_err(|e| format!("hold shortcut for `{k}`: {e}"))?;
        out.insert(k.clone(), vc_format(&c));
    }
    Ok(out)
}

/// The hold shortcuts of the Keyboard Shortcuts dialog's working set.
fn dialog_holds(d: &Dialog) -> Holds {
    d.fields.get("holds").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// Text shown for all active chords in a dialog row, with the backtick key shown as `typed`
/// ([`backtick_text`]).
fn dialog_shortcuts_with(km: &Keymap, key: &str, typed: Option<&str>) -> Option<String> {
    let shown = |s: String| menus::pretty_shortcut(&on_layout(s, typed));
    let bindings = km.bindings(&command_of(key));
    if bindings.len() > 1 {
        let primary = effective_with(km, key).map(shown)?;
        let additional = bindings.iter().skip(1).map(|b| shown(crate::keymap::binding_text(b)));
        return Some(std::iter::once(primary).chain(additional).collect::<Vec<_>>().join(" · "));
    }
    effective_with(km, key).map(shown)
}

/// Overrides of a preset (both presets are the registry defaults for now).
pub fn preset(name: &str) -> Option<Overrides> {
    PRESETS.contains(&set_name(name)).then(Overrides::new)
}

/// Export format: `{"format": "vectorcraft-shortcuts", "set": name, "overrides": {...}}`.
pub fn export_json(set: &str, overrides: &Overrides) -> Value {
    export_json_with(set, overrides, &Holds::new())
}

/// [`export_json`] with the set's hold shortcuts, written as `holds` when there are any. The
/// version stays 1: earlier versions read `overrides` only, so they import the file without them.
pub fn export_json_with(set: &str, overrides: &Overrides, holds: &Holds) -> Value {
    let mut v = json!({"format": "vectorcraft-shortcuts", "version": 1, "set": set, "overrides": stored::to_value(overrides)});
    if !holds.is_empty() {
        v["holds"] = json!(holds);
    }
    v
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

/// Export Text: the set's name, then the dialog's Tools and Menu Commands lists in its order, one
/// row per line with the name and the shortcuts as the dialog shows them, separated by a tab. A
/// row with no shortcut ends after the tab.
pub fn export_text(set: &str, overrides: &Overrides) -> String {
    export_text_with(set, overrides, &Holds::new())
}

/// [`export_text`] with the set's hold shortcuts: a row with one gets a third column, `Hold X`.
pub fn export_text_with(set: &str, overrides: &Overrides, holds: &Holds) -> String {
    let snapshot = snapshot();
    let km = crate::keymap::keymap_in(&snapshot, overrides);
    let backtick = backtick_text();
    let shown_set = shown_set_name(set);
    let mut out = format!("{}\n{} {shown_set}\n", tl!("Keyboard Shortcuts"), tl!("Set:"));
    for (tools, heading) in [(true, "Tools"), (false, "Menu Commands")] {
        out += &format!("\n{}\n", tl!(heading));
        for e in snapshot.entries.iter().filter(|e| e.is_tool == tools && listed(km.overrides(), e)) {
            let name = shown_name(e);
            let sc = dialog_shortcuts_with(&km, &e.key, backtick.as_deref()).unwrap_or_default();
            match hold_of(holds, &e.key) {
                Some(h) => out += &format!("{name}\t{sc}\t{} {}\n", tl!("Hold"), menus::pretty_shortcut(&h)),
                None => out += &format!("{name}\t{sc}\n"),
            }
        }
    }
    out
}

/// A set's name as the dialog shows it: the presets and Custom in the UI language.
fn shown_set_name(set: &str) -> &str {
    if PRESETS.contains(&set) || set == CUSTOM { tl!(set) } else { set }
}

/// Export Text's file formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFormat {
    /// [`export_text`]'s text file.
    Text,
    /// The printable shortcut sheet as a PDF ([`crate::shortcut_sheet::sheet_pdf`]).
    Sheet,
    /// A CSV file of [`export_sections`] ([`crate::shortcut_sheet::csv`]).
    Csv,
}

impl TextFormat {
    pub const ALL: [Self; 3] = [Self::Text, Self::Sheet, Self::Csv];

    /// The `format` param's value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Sheet => "sheet",
            Self::Csv => "csv",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Text => "txt",
            Self::Sheet => "pdf",
            Self::Csv => "csv",
        }
    }

    /// The save dialog's file type.
    fn filter(self) -> crate::FileFilter {
        let label = match self {
            Self::Text => "Text",
            Self::Sheet => "Shortcut sheet",
            Self::Csv => "CSV",
        };
        crate::FilePick::filter(label, &[self.extension()])
    }

    fn file_name(self) -> String {
        format!("VectorCraft Shortcuts.{}", self.extension())
    }

    /// `path` with `format`'s extension: kept when it has it in any letter case, the extension of
    /// another of these formats replaced, and the extension added to any other path.
    fn with_extension(path: &str, format: Self) -> String {
        let ext = vectorcraft_engine::cmd::fileio::extension(path);
        if ext.eq_ignore_ascii_case(format.extension()) {
            path.to_string()
        } else if Self::ALL.iter().any(|f| f.extension().eq_ignore_ascii_case(&ext)) {
            std::path::Path::new(path).with_extension(format.extension()).to_string_lossy().into_owned()
        } else {
            format!("{path}.{}", format.extension())
        }
    }

    /// The format a save dialog's answer picked: the file type it reports, else the extension of
    /// `path`; text for any other.
    fn picked(path: &str, file_type: Option<&str>) -> Self {
        let ext = file_type.map_or_else(|| vectorcraft_engine::cmd::fileio::extension(path), str::to_string);
        Self::ALL.into_iter().find(|f| f.extension().eq_ignore_ascii_case(ext.trim_start_matches('.'))).unwrap_or(Self::Text)
    }
}

/// The section name for menu commands no menu holds, as Export Text's heading has it.
const MENU_COMMANDS: &str = "Menu Commands";

/// Export Text's rows in sections, for the shortcut sheet and the CSV file: Tools, then one section
/// per top-level menu in the order the dialog first lists a command of it, with the names and
/// shortcuts as the dialog shows them, each name without its top-level menu. Every row the dialog
/// lists is there, with an empty shortcut when it has none. A row with a hold shortcut in `holds`
/// ends its shortcuts with `Hold X`, joined as the dialog joins them.
pub fn export_sections(overrides: &Overrides, holds: &Holds) -> Vec<crate::shortcut_sheet::Section> {
    use crate::shortcut_sheet::{Row, Section};
    let snapshot = snapshot();
    let km = crate::keymap::keymap_in(&snapshot, overrides);
    let backtick = backtick_text();
    let mut tools = Section { title: tl!("Tools").to_string(), rows: vec![] };
    let mut menus: Vec<Section> = vec![];
    for e in snapshot.entries.iter().filter(|e| listed(km.overrides(), e)) {
        let shortcut = dialog_shortcuts_with(&km, &e.key, backtick.as_deref()).unwrap_or_default();
        let shortcut = match hold_of(holds, &e.key) {
            Some(h) if shortcut.is_empty() => format!("{} {}", tl!("Hold"), menus::pretty_shortcut(&h)),
            Some(h) => format!("{shortcut} · {} {}", tl!("Hold"), menus::pretty_shortcut(&h)),
            None => shortcut,
        };
        if e.is_tool {
            tools.rows.push(Row { name: shown_name(e), shortcut });
            continue;
        }
        let group = shown_group(&e.group);
        let (top, rest) = group.split_once(" › ").unwrap_or((group.as_str(), ""));
        let top = if top.is_empty() { tl!(MENU_COMMANDS) } else { top };
        let label = shown_label(e);
        let inner = if rest.is_empty() { label.to_string() } else { format!("{rest} › {label}") };
        let name = if e.shows_key { format!("{inner} ({})", e.key) } else { inner };
        match menus.iter_mut().find(|s| s.title == top) {
            Some(s) => s.rows.push(Row { name, shortcut }),
            None => menus.push(Section { title: top.to_string(), rows: vec![Row { name, shortcut }] }),
        }
    }
    std::iter::once(tools).chain(menus).collect()
}

/// The file `format` writes for `set` with `overrides`: the sheet on `page`.
fn export_text_file(
    format: TextFormat,
    set: &str,
    overrides: &Overrides,
    holds: &Holds,
    page: crate::shortcut_sheet::PageSize,
) -> Result<Vec<u8>, String> {
    match format {
        TextFormat::Text => Ok(export_text_with(set, overrides, holds).into_bytes()),
        TextFormat::Sheet => {
            let title = format!("{} · {}", tl!("Keyboard Shortcuts"), shown_set_name(set));
            crate::shortcut_sheet::sheet_pdf(&title, &export_sections(overrides, holds), page)
        }
        TextFormat::Csv => Ok(crate::shortcut_sheet::csv(&export_sections(overrides, holds), ["Group", "Command", "Shortcut"]).into_bytes()),
    }
}

/// The sheet's paper when `pageSize` is left out: A4 when the current document prints on A4 (its
/// print settings, or, with none saved, a first artboard of A4 size either way up), else Letter.
fn default_sheet_page(app: &VectorcraftApp) -> crate::shortcut_sheet::PageSize {
    use crate::shortcut_sheet::PageSize;
    let Some(st) = app.session.active() else { return PageSize::Letter };
    let a4 = match st.doc.print_setup.clone().and_then(|v| serde_json::from_value::<vectorcraft_pdf::PrintSettings>(v).ok()) {
        Some(settings) => settings.media == vectorcraft_pdf::Media::A4,
        None => st.doc.artboards.first().is_some_and(|a| {
            let (w, h) = PageSize::A4.landscape();
            let (long, short) = (a.rect.width().max(a.rect.height()), a.rect.width().min(a.rect.height()));
            (long - w).abs() < 1.0 && (short - h).abs() < 1.0
        }),
    };
    if a4 { PageSize::A4 } else { PageSize::Letter }
}

// ---------- UI commands ----------

pub fn open(app: &mut VectorcraftApp) {
    let ov = stored::to_value(&app.ui.shortcut_overrides);
    app.ui.dialog = Some(Dialog::new(
        "shortcuts",
        json!({
            "tab": "tools",
            "query": "",
            "set": app.ui.shortcut_set,
            "overrides": ov,
            "holds": app.ui.shortcut_holds,
            "holdShortcuts": app.session.prefs.hold_shortcuts,
            "keyHoldDelay": app.session.prefs.key_hold_delay(),
            "__holdInfo": false,
            "__recording": "",
            "__recordingHold": false,
            "__selected": "",
            "__message": "",
            "__conflict": ""
        }),
    ));
}

/// Hold Shortcuts in the Keyboard Shortcuts dialog's working copy.
fn dialog_holds_on(d: &Dialog) -> bool {
    d.bool("holdShortcuts")
}

/// The Hold Shortcut Delay in the Keyboard Shortcuts dialog's working copy, in its range.
fn dialog_hold_delay(d: &Dialog) -> f64 {
    let (min, max) = vectorcraft_engine::KEY_HOLD_DELAY_RANGE;
    d.fields.get("keyHoldDelay").and_then(Value::as_f64).filter(|x| x.is_finite()).map_or(vectorcraft_engine::KEY_HOLD_DELAY, |x| x.clamp(min, max))
}

fn dialog_overrides(d: &Dialog) -> Overrides {
    d.fields.get("overrides").map(|v| stored::from_value(v.clone())).unwrap_or_default()
}

/// OK: commit the working set.
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    let (on, delay) = (dialog_holds_on(&d), dialog_hold_delay(&d));
    let prefs = &app.session.prefs;
    if d.kind == "shortcuts" && (on != prefs.hold_shortcuts || delay != prefs.key_hold_delay()) {
        app.run("prefs.set", json!({"values": {"holdShortcuts": on, "keyHoldDelay": delay}})).map_err(|e| e.to_string())?;
        // Saved as OK in Preferences saves them.
        crate::prefs_dialog::snapshot(app);
        app.prefs_changed = true;
    }
    app.ui.shortcut_overrides = dialog_overrides(&d);
    app.ui.shortcut_holds = dialog_holds(&d);
    app.ui.shortcut_set = d.str("set");
    app.ui.dialog = None;
    sync(&app.ui);
    Ok(json!({"overrides": app.ui.shortcut_overrides.len()}))
}

/// The string `p` gives for `key`, or `None` when `p` leaves it out or gives null. Any other
/// value, such as `5`, is an error: "`{key}` must be a string".
fn text_param(p: &Value, key: &str) -> Result<Option<String>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// `shortcuts.*` UI commands. `None` = not ours.
pub fn run_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| text_param(p, k);
    let r = match id {
        "edit.keyboardShortcuts" => {
            open(app);
            Ok(Value::Null)
        }
        "shortcuts.set" => {
            let (key, shortcut, force) = match (s("id"), s("shortcut"), crate::menus::opt_bool(p, "force")) {
                (Ok(key), Ok(shortcut), Ok(force)) => (key.unwrap_or_default(), shortcut, force.unwrap_or(false)),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return Some(Err(e)),
            };
            let hold = match s("hold") {
                Ok(hold) => hold,
                Err(e) => return Some(Err(e)),
            };
            // Without `hold`, a missing `shortcut` removes the shortcut, as it always has; with
            // `hold` alone, the shortcut is left as it is. Both are checked before either is kept.
            let has_hold = p.get("hold").is_some();
            let has_shortcut = p.get("shortcut").is_some() || !has_hold;
            let mut ov = app.ui.shortcut_overrides.clone();
            let mut holds = app.ui.shortcut_holds.clone();
            let r = if !has_shortcut {
                Ok(vec![])
            } else if p.get("shortcut").is_some_and(Value::is_null) {
                reset_one(&mut ov, &key);
                Ok(vec![])
            } else {
                assign(&mut ov, &key, shortcut.as_deref(), force)
            };
            let r = r.and_then(|mut removed| {
                // An entry the editor lists only with a shortcut takes a hold shortcut only with one.
                if hold.as_deref().is_some_and(|h| !h.is_empty()) && entry(&key).is_some_and(|e| !listed(&ov, &e)) {
                    return Err(format!("`{key}` needs a shortcut before it can have a hold shortcut"));
                }
                if has_hold {
                    for k in assign_hold(&mut holds, &key, hold.as_deref(), force)? {
                        if !removed.contains(&k) {
                            removed.push(k);
                        }
                    }
                }
                drop_unlisted_holds(&mut holds, &ov);
                Ok(removed)
            });
            r.map(|removed| {
                app.ui.shortcut_overrides = ov;
                app.ui.shortcut_holds = holds;
                app.ui.shortcut_set = CUSTOM.into();
                sync(&app.ui);
                json!({
                    "id": key,
                    "shortcut": effective_in(&app.ui.shortcut_overrides, &key),
                    "hold": hold_of(&app.ui.shortcut_holds, &key),
                    "removedFrom": removed
                })
            })
        }
        "shortcuts.list" => {
            let q = match s("query") {
                Ok(q) => q.unwrap_or_default().to_lowercase(),
                Err(e) => return Some(Err(e)),
            };
            let snapshot = snapshot();
            let km = crate::keymap::keymap_in(&snapshot, &app.ui.shortcut_overrides);
            let ov = &crate::keymap::effect_keys_in(&app.ui.shortcut_overrides, &snapshot.effects);
            Ok(Value::Array(
                snapshot
                    .entries
                    .iter()
                    .filter(|e| listed(ov, e))
                    .filter(|e| q.is_empty() || e.label.to_lowercase().contains(&q) || e.key.to_lowercase().contains(&q))
                    .map(|e| json!({"id": e.key, "label": e.label, "group": e.group, "shortcut": effective_with(&km, &e.key), "hold": hold_of(&app.ui.shortcut_holds, &e.key), "default": default_of(&e.key), "overridden": crate::keymap::has_override(ov, &e.key)}))
                    .collect(),
            ))
        }
        "shortcuts.conflicts" => {
            let holds = hold_conflicts(&app.ui.shortcut_holds).into_iter().map(|(c, k)| (format!("Hold {c}"), k));
            Ok(json!(
                all_conflicts(&app.ui.shortcut_overrides).into_iter().chain(holds).map(|(c, k)| json!({"shortcut": c, "ids": k})).collect::<Vec<_>>()
            ))
        }
        "shortcuts.reset" => {
            app.ui.shortcut_overrides.clear();
            app.ui.shortcut_holds.clear();
            app.ui.shortcut_set = PRESETS[0].into();
            sync(&app.ui);
            Ok(Value::Null)
        }
        "shortcuts.preset" => match s("name").map(|n| n.as_deref().map(set_name).and_then(|n| preset(n).map(|p| (n.to_string(), p)))) {
            Ok(Some((n, p))) => {
                app.ui.shortcut_overrides = p;
                // The presets have no hold shortcuts.
                app.ui.shortcut_holds.clear();
                app.ui.shortcut_set = n;
                sync(&app.ui);
                Ok(Value::Null)
            }
            Ok(None) => Err(format!("unknown preset (one of {})", PRESETS.join(", "))),
            Err(e) => Err(e),
        },
        "shortcuts.export" => {
            let path = match s("path") {
                Ok(path) => path,
                Err(e) => return Some(Err(e)),
            };
            let v = export_json_with(&app.ui.shortcut_set, &app.ui.shortcut_overrides, &app.ui.shortcut_holds);
            let bytes = serde_json::to_vec_pretty(&v).unwrap_or_default();
            let pick = crate::FilePick {
                filters: vec![crate::FilePick::filter("Shortcut sets", &["json"])],
                ..crate::FilePick::named("VectorCraft Shortcuts.json")
            };
            let path = path.or_else(|| crate::picks::save(app, &pick));
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
        "shortcuts.exportText" => {
            let path = match p.get("path") {
                None | Some(Value::Null) => None,
                Some(Value::String(path)) => Some(path.clone()),
                Some(_) => {
                    let key = "path";
                    return Some(Err(format!("`{key}` must be a string")));
                }
            };
            let format = match s("format") {
                Ok(None) => None,
                Ok(Some(f)) => match TextFormat::ALL.into_iter().find(|t| t.name() == f) {
                    Some(t) => Some(t),
                    None => {
                        let key = "format";
                        return Some(Err(format!("`{key}` must be one of {}", TextFormat::ALL.map(TextFormat::name).join(", "))));
                    }
                },
                Err(e) => return Some(Err(e)),
            };
            let page = match s("pageSize") {
                Ok(None) => default_sheet_page(app),
                Ok(Some(n)) => match crate::shortcut_sheet::PageSize::parse(&n) {
                    Some(page) => page,
                    None => {
                        let key = "pageSize";
                        return Some(Err(format!(
                            "`{key}` must be one of {}",
                            crate::shortcut_sheet::PageSize::ALL.map(crate::shortcut_sheet::PageSize::name).join(", ")
                        )));
                    }
                },
                Err(e) => return Some(Err(e)),
            };
            // A given path is written as text unless `format` says otherwise; a save dialog offers
            // every format (text first) unless `format` names one, and its answer picks the format.
            let (path, format) = match path {
                Some(path) => (Some(path), format.unwrap_or(TextFormat::Text)),
                None => {
                    let shown = format.unwrap_or(TextFormat::Text);
                    let filters = format.map_or_else(|| TextFormat::ALL.map(TextFormat::filter).to_vec(), |f| vec![f.filter()]);
                    let pick = crate::FilePick { filters, ..crate::FilePick::named(&shown.file_name()) };
                    match crate::picks::save_with_type(app, &pick) {
                        Some((answer, file_type)) => {
                            let picked = format.unwrap_or_else(|| TextFormat::picked(&answer, file_type.as_deref()));
                            // The format that `format` names, or that a dialog reports as the type
                            // picked, gives the name its extension. A file already at a path
                            // changed that way, which the dialog did not check, is replaced only
                            // after asking.
                            let typed = format.is_some() || file_type.is_some();
                            let path = if typed { TextFormat::with_extension(&answer, picked) } else { answer.clone() };
                            let request = crate::picks::PickRequest::Save(pick.clone());
                            if path != answer && crate::picks::ask_to_replace(app, request, &answer, file_type.as_deref(), &path) {
                                return Some(Err("cancelled".into()));
                            }
                            (Some(path), picked)
                        }
                        None => (None, shown),
                    }
                }
            };
            // Its save dialog is open: the file is made and written when it answers.
            if path.is_none() && app.file_dialog_open() {
                return Some(Err("cancelled".into()));
            }
            // The hold shortcuts are listed while they act.
            let holds = if app.session.prefs.hold_shortcuts { app.ui.shortcut_holds.clone() } else { Holds::new() };
            let bytes = match export_text_file(format, &app.ui.shortcut_set, &app.ui.shortcut_overrides, &holds, page) {
                Ok(bytes) => bytes,
                Err(e) => return Some(Err(e)),
            };
            match path {
                Some(path) => match app.services.write.as_mut() {
                    Some(w) => w(&path, &bytes).map(|_| json!({"path": path})),
                    None => Err("no file writer".into()),
                },
                None => match app.services.download.as_mut() {
                    Some(dl) => dl(&format.file_name(), &bytes).map(|()| Value::Null),
                    None if format == TextFormat::Sheet => Ok(json!({"pdf": vectorcraft_svg::base64_encode(&bytes)})),
                    None => Ok(json!({"text": String::from_utf8_lossy(&bytes)})),
                },
            }
        }
        "shortcuts.import" => {
            let data = if let Some(v) = p.get("data") {
                Ok(v.clone())
            } else {
                let path = match s("path") {
                    Ok(path) => path,
                    Err(e) => return Some(Err(e)),
                };
                let pick = crate::FilePick {
                    filters: vec![crate::FilePick::filter("Shortcut sets", crate::dialogs::shortcut_import::IMPORT_EXTS)],
                    ..Default::default()
                };
                let path = path.or_else(|| crate::picks::open(app, &pick));
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
            data.and_then(|v| Ok((import_json(&v)?, import_holds(&v)?))).map(|((set, ov), mut holds)| {
                let n = ov.len();
                drop_unlisted_holds(&mut holds, &ov);
                app.ui.shortcut_overrides = ov;
                app.ui.shortcut_holds = holds;
                app.ui.shortcut_set = set;
                sync(&app.ui);
                if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == "shortcuts") {
                    d.fields.insert("overrides".into(), stored::to_value(&app.ui.shortcut_overrides));
                    d.fields.insert("holds".into(), json!(app.ui.shortcut_holds));
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

/// The dialog's working set as a keymap, with the entries it lists, each entry's shortcut text (in
/// their order) and the number of conflicts, kept between frames while the set and the installed
/// plug-ins are unchanged.
struct DialogView {
    /// The dialog's `overrides` field this was built from.
    source: Value,
    /// The [`backtick_text`] the texts show.
    backtick: Option<String>,
    /// The plug-in revision of `entries`.
    revision: u64,
    entries: Arc<[Entry]>,
    keymap: Keymap,
    texts: Vec<Option<String>>,
    conflicts: usize,
}

impl DialogView {
    fn new(overrides: Overrides) -> DialogView {
        let snapshot = snapshot();
        let keymap = crate::keymap::keymap_in(&snapshot, &overrides);
        let backtick = backtick_text();
        let texts = snapshot.entries.iter().map(|e| dialog_shortcuts_with(&keymap, &e.key, backtick.as_deref())).collect();
        let conflicts = conflicts_in(&keymap).len();
        DialogView {
            source: stored::to_value(&crate::keymap::effect_keys_in(&overrides, &snapshot.effects)),
            backtick,
            revision: snapshot.revision,
            entries: snapshot.entries.clone(),
            keymap,
            texts,
            conflicts,
        }
    }

    /// The view of `d`'s working set, from the cache unless the set, the backtick key's character
    /// or the installed plug-ins changed. The dialog's `overrides` field is left in its stored form.
    fn of(ctx: &egui::Context, d: &mut Dialog) -> Arc<DialogView> {
        let id = egui::Id::unique("dialog-shortcuts-view");
        let source = d.fields.get("overrides").unwrap_or(&Value::Null);
        let backtick = backtick_text();
        let revision = menus::plugin_revision();
        if let Some(v) =
            ctx.data(|m| m.get_temp::<Arc<DialogView>>(id)).filter(|v| v.source == *source && v.backtick == backtick && v.revision == revision)
        {
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
    ctx.input(|i| note_backtick_text(&i.events));
    // The working set, as a keymap: copied only by an edit this frame.
    let view = DialogView::of(ctx, &mut d);
    let mut km = Cow::Borrowed(&view.keymap);
    let mut ok = false;
    let mut cancel = false;
    drop_hidden(&mut d, km.overrides());
    let saved = dialog_overrides(&d);
    let mut holds = dialog_holds(&d);
    let recording = d.str("__recording");
    // Which column records: the Hold column, or the Shortcut column.
    let recording_hold = !recording.is_empty() && d.bool("__recordingHold");
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
                if recording_hold {
                    record_hold(&mut d, &mut holds, &recording, &chord);
                } else {
                    record(&mut d, km.to_mut(), &recording, &chord);
                }
            }
        }
    }
    crate::dialogs::modal::show(ctx, tl!("Keyboard Shortcuts"), egui::Id::unique("dialog-shortcuts"), -20.0, 20, |ui| {
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
                // The presets have no hold shortcuts.
                holds.clear();
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
            if ui.button(tl!("Export Text…")).clicked() {
                d.fields.insert("__exportText".into(), json!(true));
            }
            if ui.button(tl!("Reset to Defaults")).clicked() {
                km.to_mut().reset_all();
                holds.clear();
                d.fields.insert("set".into(), json!(PRESETS[0]));
                d.fields.insert("__message".into(), json!(tl!("All shortcuts reset to defaults.")));
            }
        });
        ui.add_space(6.0);
        hold_row(ui, &t, &mut d);
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
                if ui.add(egui::TextEdit::singleline(&mut q).align(egui::Align2::LEFT_CENTER).hint_text(tl!("Search")).desired_width(200.0)).changed()
                {
                    d.fields.insert("query".into(), json!(q));
                }
            });
        });
        ui.add_space(6.0);
        let tools = d.str("tab") != "menu";
        let q = d.str("query").to_lowercase();
        let selected = d.str("__selected");
        let rec = d.str("__recording");
        let rec_hold = d.bool("__recordingHold");
        let holds_on = dialog_holds_on(&d);
        let scroll_to = d.fields.remove("__scrollTo").and_then(|v| v.as_str().map(str::to_string));
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let name_width = if holds_on { 340.0 } else { 360.0 };
                ui.allocate_ui_with_layout(egui::vec2(name_width, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(name_width);
                    ui.label(egui::RichText::new(if tools { tl!("Tool") } else { tl!("Command") }).color(t.text_dim).size(11.0));
                });
                if holds_on {
                    ui.allocate_ui_with_layout(egui::vec2(150.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(150.0);
                        ui.label(egui::RichText::new(tl!("Shortcut")).color(t.text_dim).size(11.0));
                    });
                    ui.label(egui::RichText::new(tl!("Hold")).color(t.text_dim).size(11.0));
                } else {
                    ui.label(egui::RichText::new(tl!("Shortcut")).color(t.text_dim).size(11.0));
                }
            });
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
                for (i, e) in view.entries.iter().enumerate().filter(|(_, e)| e.is_tool == tools && listed(km.overrides(), e)) {
                    let sc = match &km {
                        Cow::Borrowed(_) => view.texts.get(i).cloned().flatten(),
                        Cow::Owned(km) => dialog_shortcuts_with(km, &e.key, view.backtick.as_deref()),
                    };
                    let hold_text = if holds_on { hold_of(&holds, &e.key).map(|h| menus::pretty_shortcut(&h)) } else { None };
                    if !q.is_empty() && !row_matches(e, &q, sc.as_deref()) && !hold_text.is_some_and(|h| h.to_lowercase().contains(&q)) {
                        continue;
                    }
                    let row = ui.horizontal(|ui| {
                        let name = shown_name(e);
                        let name_width = if holds_on { 340.0 } else { 360.0 };
                        let r = ui
                            .allocate_ui_with_layout(egui::vec2(name_width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.set_min_width(name_width);
                                ui.add(egui::Button::selectable(selected == e.key, egui::RichText::new(name).color(t.text)).truncate())
                            })
                            .inner;
                        if r.clicked() {
                            d.fields.insert("__selected".into(), json!(e.key));
                        }
                        let recording_here = rec == e.key && !rec_hold;
                        let label = if recording_here { tl!("Press keys…").to_string() } else { sc.unwrap_or_else(|| "—".into()) };
                        let changed = crate::keymap::has_override(&saved, &e.key);
                        let txt = egui::RichText::new(label).color(if changed { t.accent } else { t.text });
                        let b = ui
                            .add_sized([150.0, 20.0], egui::Button::new(txt).selected(recording_here))
                            .on_hover_text(tl!("Click, then press the new shortcut (Esc cancels)"));
                        if b.clicked() {
                            d.fields.insert("__recording".into(), json!(e.key));
                            d.fields.insert("__recordingHold".into(), json!(false));
                            d.fields.insert("__selected".into(), json!(e.key));
                        }
                        if !holds_on {
                            return;
                        }
                        let holding_here = rec == e.key && rec_hold;
                        let hold = hold_of(&holds, &e.key);
                        let label = if holding_here {
                            tl!("Press keys…").to_string()
                        } else {
                            hold.as_deref().map_or_else(|| "—".into(), menus::pretty_shortcut)
                        };
                        let h = ui.add_sized([90.0, 20.0], egui::Button::new(egui::RichText::new(label).color(t.text)).selected(holding_here)).on_hover_text(
                            tl!("Click, then press a key to hold for this (Esc cancels). Holding the key runs this; a quick press runs the key's own shortcut."),
                        );
                        if h.clicked() {
                            d.fields.insert("__recording".into(), json!(e.key));
                            d.fields.insert("__recordingHold".into(), json!(true));
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
                if dialog_holds_on(&d) && ui.add_enabled(holds.contains_key(&sel), egui::Button::new(tl!("Clear Hold"))).clicked() {
                    holds.remove(&sel);
                    d.fields.insert("set".into(), json!(CUSTOM));
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
        } + if dialog_holds_on(&d) { hold_conflicts(&holds).len() } else { 0 };
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
        d.fields.insert("overrides".into(), stored::to_value(&saved_overrides(km.overrides(), &saved)));
    }
    drop_unlisted_holds(&mut holds, km.overrides());
    d.fields.insert("holds".into(), json!(holds));
    if d.str("__recording").is_empty() {
        d.fields.insert("__recordingHold".into(), json!(false));
    }
    let import = d.fields.remove("__import").is_some();
    let export = d.fields.remove("__export").is_some();
    let export_text = d.fields.remove("__exportText").is_some();
    app.ui.dialog = Some(d);
    // Their file dialogs, shown off the UI thread, import into or export from the dialog as it is
    // when they answer.
    if import {
        crate::picks::button(app, import_into_dialog);
    }
    if export {
        crate::picks::button(app, export_from_dialog);
    }
    if export_text {
        crate::picks::button(app, export_text_from_dialog);
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
    let saved = (app.ui.shortcut_overrides.clone(), app.ui.shortcut_holds.clone(), app.ui.shortcut_set.clone());
    match run_command(app, "shortcuts.import", &json!({})) {
        Some(Ok(_)) => {
            let (o, h, s) = (app.ui.shortcut_overrides.clone(), app.ui.shortcut_holds.clone(), app.ui.shortcut_set.clone());
            (app.ui.shortcut_overrides, app.ui.shortcut_holds, app.ui.shortcut_set) = saved;
            sync(&app.ui);
            if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == "shortcuts") {
                d.fields.insert("overrides".into(), stored::to_value(&o));
                d.fields.insert("holds".into(), json!(h));
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
    export_dialog_set(app, "shortcuts.export");
}

/// The dialog's Export Text…: its working set, as edited so far, as a text file.
fn export_text_from_dialog(app: &mut VectorcraftApp) {
    export_dialog_set(app, "shortcuts.exportText");
}

/// Runs `command` on the dialog's working set and set name.
fn export_dialog_set(app: &mut VectorcraftApp, command: &str) {
    if !shortcuts_dialog(app) {
        app.status(crate::picks::dialog_gone());
        return;
    }
    let (ov, holds, set, on) =
        app.ui.dialog.as_ref().map(|d| (dialog_overrides(d), dialog_holds(d), d.str("set"), dialog_holds_on(d))).unwrap_or_default();
    let saved = std::mem::replace(&mut app.ui.shortcut_overrides, ov);
    let saved_holds = std::mem::replace(&mut app.ui.shortcut_holds, holds);
    let saved_set = std::mem::replace(&mut app.ui.shortcut_set, set);
    // Export Text… lists the hold shortcuts while the dialog's Hold Shortcuts is on.
    let saved_on = std::mem::replace(&mut app.session.prefs.hold_shortcuts, on);
    if let Some(Err(e)) = run_command(app, command, &json!({})) {
        app.status(e);
    }
    app.session.prefs.hold_shortcuts = saved_on;
    app.ui.shortcut_overrides = saved;
    app.ui.shortcut_holds = saved_holds;
    app.ui.shortcut_set = saved_set;
}

/// A row that left the list (Clear, Use Default, Reset to Defaults or another set took the
/// shortcut from a command that needs a value) is no longer recorded, selected or offered by Go to
/// Conflict, so the next key press does not give the hidden command a shortcut.
fn drop_hidden(d: &mut Dialog, overrides: &Overrides) {
    for field in ["__recording", "__selected", "__conflict"] {
        let key = d.str(field);
        if !key.is_empty() && entry(&key).is_none_or(|e| !listed(overrides, &e)) {
            d.fields.insert(field.into(), json!(""));
        }
    }
}

/// Does the search text `q` (lowercase) find entry `e`, whose shortcut text is `sc`? It does when
/// `e`'s label or group, in English or as shown, the key it shows ([`Entry::shows_key`]) or its
/// shortcut contains `q`.
fn row_matches(e: &Entry, q: &str, sc: Option<&str>) -> bool {
    e.label.to_lowercase().contains(q)
        || shown_label(e).to_lowercase().contains(q)
        || e.group.to_lowercase().contains(q)
        || shown_group(&e.group).to_lowercase().contains(q)
        || (e.shows_key && e.key.to_lowercase().contains(q))
        || sc.is_some_and(|s| s.to_lowercase().contains(q))
}

/// Apply a recorded chord to `key` in the working set, warning about (and resolving) conflicts.
fn record(d: &mut Dialog, km: &mut Keymap, key: &str, chord: &str) {
    d.fields.insert("__recording".into(), json!(""));
    match assign_in(km, key, Some(chord), true) {
        Ok(removed) => {
            d.fields.insert("set".into(), json!(CUSTOM));
            if let Some(first) = removed.first() {
                let names: Vec<String> = removed.iter().map(|k| entry(k).map(|e| shown_name(&e)).unwrap_or(k.clone())).collect();
                d.fields.insert(
                    "__message".into(),
                    json!(crate::i18n::fmt(
                        tl!("⚠ {chord} was already used by {names} — it has been removed there."),
                        &[
                            ("chord", &menus::pretty_shortcut(&on_layout(chord.to_string(), backtick_text().as_deref()))),
                            ("names", &names.join(", "))
                        ]
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

/// The Hold Shortcuts row of the Keyboard Shortcuts dialog: the checkbox that turns hold shortcuts
/// on (and shows the Hold column), a button that shows how they work, and, while on, the Hold
/// Shortcut Delay.
fn hold_row(ui: &mut egui::Ui, t: &Tokens, d: &mut Dialog) {
    ui.horizontal(|ui| {
        let mut on = dialog_holds_on(d);
        if ui.checkbox(&mut on, egui::RichText::new(tl!("Hold Shortcuts")).color(t.text)).changed() {
            d.fields.insert("holdShortcuts".into(), json!(on));
            if !on && d.bool("__recordingHold") {
                d.fields.insert("__recording".into(), json!(""));
                d.fields.insert("__recordingHold".into(), json!(false));
            }
        }
        let info = d.bool("__holdInfo");
        if ui
            .add(egui::Button::new(egui::RichText::new("?").color(t.text_dim)).selected(info).small())
            .on_hover_text(tl!("How hold shortcuts work"))
            .clicked()
        {
            d.fields.insert("__holdInfo".into(), json!(!info));
        }
        if on {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tl!("Hold Shortcut Delay")).color(t.text_dim));
            let (min, max) = vectorcraft_engine::KEY_HOLD_DELAY_RANGE;
            let mut delay = dialog_hold_delay(d);
            if ui.add(egui::DragValue::new(&mut delay).range(min..=max).speed(0.01).fixed_decimals(2).suffix(" s")).changed() {
                d.fields.insert("keyHoldDelay".into(), json!(delay));
            }
        }
    });
    if d.bool("__holdInfo") {
        egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                hold_illustration(ui, t);
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(tl!(
                            "A quick press of a key runs its shortcut. Holding the key for the delay runs the tool or command in its Hold column instead. A hold shortcut is a letter, digit or punctuation key, alone or with Shift."
                        ))
                        .color(t.text)
                        .size(11.5),
                    )
                    .wrap(),
                );
            });
        });
    }
}

/// A key and two presses of it drawn in code: a short press (Tap) ends before the delay, and a long
/// one (Hold) reaches it.
fn hold_illustration(ui: &mut egui::Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 52.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let key = egui::Rect::from_min_size(rect.min + egui::vec2(2.0, 12.0), egui::vec2(28.0, 28.0));
    p.rect(key, 4.0, t.input, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    p.text(key.center(), egui::Align2::CENTER_CENTER, "X", egui::FontId::proportional(13.0), t.text);
    let start = key.right() + 10.0;
    let delay_x = start + 70.0;
    let rows = [(tl!("Tap"), 40.0, key.top() + 6.0), (tl!("Hold"), 100.0, key.bottom() - 6.0)];
    for (label, len, y) in rows {
        let color = if start + len >= delay_x { t.accent } else { t.text_dim };
        p.line_segment([egui::pos2(start, y), egui::pos2(start + len, y)], egui::Stroke::new(4.0, color));
        p.text(egui::pos2(start + len + 4.0, y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(10.5), t.text);
    }
    // The delay.
    let mut y = rect.top() + 4.0;
    while y < rect.bottom() - 4.0 {
        p.line_segment([egui::pos2(delay_x, y), egui::pos2(delay_x, (y + 3.0).min(rect.bottom() - 4.0))], egui::Stroke::new(1.0, t.text_dim));
        y += 6.0;
    }
}

/// Apply a recorded key as `key`'s hold shortcut in the working set. Another entry that held the
/// key loses it, with the same warning as a shortcut; a key that can't hold says why.
fn record_hold(d: &mut Dialog, holds: &mut Holds, key: &str, chord: &str) {
    d.fields.insert("__recording".into(), json!(""));
    d.fields.insert("__recordingHold".into(), json!(false));
    match assign_hold(holds, key, Some(chord), true) {
        Ok(removed) => {
            d.fields.insert("set".into(), json!(CUSTOM));
            let names: Vec<String> = removed.iter().map(|k| entry(k).map(|e| shown_name(&e)).unwrap_or(k.clone())).collect();
            let message = if names.is_empty() {
                String::new()
            } else {
                crate::i18n::fmt(
                    tl!("⚠ {chord} was already used by {names} — it has been removed there."),
                    &[("chord", &menus::pretty_shortcut(chord)), ("names", &names.join(", "))],
                )
            };
            d.fields.insert("__message".into(), json!(message));
            d.fields.insert("__conflict".into(), json!(""));
        }
        Err(e) => {
            d.fields.insert("__message".into(), json!(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` again until no plug-in was installed or removed while it ran, and returns what it
    /// returned. Other tests install and remove plug-ins at any time, and each changes the Effect
    /// menu's rows.
    fn with_steady_plugins<T>(mut f: impl FnMut() -> T) -> T {
        for _ in 0..100 {
            let revision = menus::plugin_revision();
            let r = f();
            if menus::plugin_revision() == revision {
                return r;
            }
        }
        panic!("the installed plug-ins kept changing");
    }

    thread_local! {
        /// Run by [`snapshot`] on this thread once, after it reads the plug-in revision and before
        /// it walks the menus.
        pub(super) static WHILE_WALKING: std::cell::Cell<Option<Box<dyn FnOnce()>>> = const { std::cell::Cell::new(None) };
    }

    /// A plug-in installed while the menus are walked doesn't leave a snapshot under the revision
    /// read before the walk: the keymap built for that revision would then lack the plug-in's row.
    #[test]
    fn a_plugin_installed_while_the_menus_are_walked_gets_a_new_snapshot() {
        use vectorcraft_plugins::wat as tpl;
        let install = |id: &str| {
            let manifest = format!(r#"{{"id":"{id}","name":"{id}","version":"1","kind":"filter","params":{{}}}}"#);
            vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
        };
        let (before, during) = ("test.ui.walk-before", "test.ui.walk-during");
        install(before);
        // Stores the snapshot of this revision, as another test thread may already have done.
        snapshot();
        WHILE_WALKING.set(Some(Box::new(move || {
            // Another caller builds the snapshot and keymap of the revision read before the walk.
            crate::keymap::template_for(&snapshot());
            install(during);
        })));
        let s = snapshot();
        let keys: Vec<&str> = s.entries.iter().map(|e| e.key.as_str()).collect();
        let catalog = crate::keymap::template_for(&s).catalog().len();
        vectorcraft_plugins::registry::remove(before);
        vectorcraft_plugins::registry::remove(during);
        assert!(keys.contains(&"plugin:test.ui.walk-during"), "the walk lists the plug-in");
        assert_eq!(catalog, s.entries.len(), "the keymap of the snapshot's revision has every entry");
    }

    #[test]
    fn format_parse_round_trip_for_every_default() {
        for e in entries().iter() {
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
    fn clearing_a_row_that_leaves_the_list_stops_its_recording() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        crate::keymap::set_override(&mut app.ui.shortcut_overrides, "document.setUnits", "Cmd+Alt+Shift+F12");
        crate::keymap::set_override(&mut app.ui.shortcut_overrides, "edit.duplicate", "Cmd+Alt+Shift+F11");
        open(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        let start = keymap_with(&dialog_overrides(&d));
        let set = |d: &mut Dialog, key: &str| {
            d.fields.insert("__recording".into(), json!(key));
            d.fields.insert("__selected".into(), json!(key));
        };
        let state = |d: &Dialog| (d.str("__recording"), d.str("__selected"));
        // Clear, and Use Default, take the row out of the list: its recording stops.
        let mut km = start.clone();
        set(&mut d, "document.setUnits");
        assign_in(&mut km, "document.setUnits", None, true).unwrap();
        drop_hidden(&mut d, km.overrides());
        assert_eq!(state(&d), (String::new(), String::new()));
        let mut km = start.clone();
        set(&mut d, "document.setUnits");
        km.reset(&command_of("document.setUnits"));
        drop_hidden(&mut d, km.overrides());
        assert_eq!(state(&d), (String::new(), String::new()));
        // A row that stays listed keeps its recording, and so does a tool's.
        let mut km = start.clone();
        set(&mut d, "edit.duplicate");
        assign_in(&mut km, "edit.duplicate", None, true).unwrap();
        drop_hidden(&mut d, km.overrides());
        assert_eq!(state(&d), ("edit.duplicate".to_string(), "edit.duplicate".to_string()));
        set(&mut d, "tool:groupSelection");
        drop_hidden(&mut d, km.overrides());
        assert_eq!(state(&d), ("tool:groupSelection".to_string(), "tool:groupSelection".to_string()));
        // A chord taken from a row that then leaves the list offers no Go to Conflict for it.
        let mut km = start.clone();
        record(&mut d, &mut km, "edit.duplicate", "Cmd+Alt+Shift+F12");
        assert_eq!(d.str("__conflict"), "document.setUnits");
        drop_hidden(&mut d, km.overrides());
        assert_eq!(d.str("__conflict"), "");
        // A conflict with a row that stays listed is kept.
        let mut km = start.clone();
        record(&mut d, &mut km, "document.setUnits", "Cmd+Alt+Shift+F11");
        drop_hidden(&mut d, km.overrides());
        assert_eq!(d.str("__conflict"), "edit.duplicate");
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

    /// On Turkish Q the key left of 1 types `"`, which egui reports as the backtick key: the
    /// recorded shortcut is that key, saved as `~` as before, and its row shows `"`. On a US
    /// layout the row still shows `~`.
    #[test]
    fn the_backtick_key_shows_what_it_types() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        open(&mut app);
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let press = |key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let text = |t: &str| egui::Event::Text(t.into());
        let mut record_with = |events: Vec<egui::Event>| {
            app.ui.dialog.as_mut().unwrap().fields.insert("__recording".into(), json!("edit.clear"));
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            let d = app.ui.dialog.clone().unwrap();
            let view = DialogView::new(dialog_overrides(&d));
            let at = view.entries.iter().position(|e| e.key == "edit.clear").unwrap();
            let shown = view.texts[at].clone();
            (crate::keymap::override_text(&dialog_overrides(&d), "edit.clear"), shown)
        };
        assert_eq!(record_with(vec![press(Key::Backtick), text("\"")]), (Some("~".to_string()), Some("\"".to_string())));
        assert_eq!(record_with(vec![press(Key::Backtick), text("`")]), (Some("~".to_string()), Some("~".to_string())));
        // eframe's web build with no text field focused sends the text before its key.
        assert_eq!(record_with(vec![text("\""), press(Key::Backtick)]), (Some("~".to_string()), Some("\"".to_string())));
        // A text right after another key's press is that key's.
        note_backtick_text(&[press(Key::Num2), text("'"), press(Key::Backtick)]);
        assert_eq!(backtick_text().as_deref(), Some("\""));
        note_backtick_text(&[text("`"), press(Key::Backtick)]);
        assert_eq!(backtick_text(), None);
        // Several keys typed in one frame on the web: each text goes with the press after it.
        let web = [text("a"), press(Key::A), text("\""), press(Key::Backtick), text("b"), press(Key::B)];
        note_backtick_text(&web);
        assert_eq!(backtick_text().as_deref(), Some("\""));
        note_backtick_text(&[text("`"), press(Key::Backtick)]);
        note_backtick_text(&[press(Key::A), text("a"), press(Key::Backtick), text("\""), press(Key::B), text("b")]);
        assert_eq!(backtick_text().as_deref(), Some("\""));
        // Only the unshifted key types that character.
        assert_eq!(on_layout("Shift+~".into(), Some("\"")), "Shift+~");
        assert_eq!(on_layout("Cmd+~".into(), Some("\"")), "Cmd+\"");
        assert_eq!(on_layout("Cmd+K".into(), Some("\"")), "Cmd+K");
    }

    #[test]
    fn menus_and_tooltips_show_the_backtick_key_as_it_types() {
        let mut ui = UiState::default();
        // Not Cmd+~: macOS reserves it, so the override would leave Edit › Clear on Delete there.
        crate::keymap::set_override(&mut ui.shortcut_overrides, "edit.clear", "Alt+~");
        crate::keymap::set_override(&mut ui.shortcut_overrides, "tool:measure", "~");
        sync(&ui);
        let measure = vectorcraft_tools::tool_info("measure").unwrap();
        let press = |key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let clear = || menus::shown_shortcut(menus::shortcut_of("edit.clear").unwrap());
        note_backtick_text(&[press(Key::Backtick), egui::Event::Text("`".into())]);
        assert_eq!(clear(), menus::pretty_shortcut("Alt+~"));
        assert!(crate::toolbar::tip(measure).ends_with("(~)"));
        note_backtick_text(&[press(Key::Backtick), egui::Event::Text("\"".into())]);
        assert_eq!(clear(), menus::pretty_shortcut("Alt+\""));
        assert!(crate::toolbar::tip(measure).ends_with("(\")"), "{}", crate::toolbar::tip(measure));
        assert_eq!(menus::shown_shortcut("Shift+~"), menus::pretty_shortcut("Shift+~"));
        sync(&UiState::default());
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

    /// An Export Text line: the row's name as the dialog shows it, a tab and `shortcut`.
    fn text_row(key: &str, shortcut: &str) -> String {
        let e = entry(key).unwrap_or_else(|| panic!("no entry {key}"));
        format!("{}\t{shortcut}", english_name(&e))
    }

    /// Export Text writes the set's name, then the Tools and the Menu Commands rows in the dialog's
    /// order, each with its shortcut as the dialog shows it.
    #[test]
    fn export_text_lists_the_dialogs_rows_with_their_shortcuts() {
        let mut ov = Overrides::new();
        crate::keymap::set_override(&mut ov, "edit.preferences", "Ctrl+Alt+Shift+9");
        let (text, km, tool_rows, menu_rows) = with_steady_plugins(|| {
            let text = export_text("VectorCraft Defaults", &ov);
            let km = keymap_with(&ov);
            let names = |tools: bool| -> Vec<String> {
                entries().iter().filter(|e| e.is_tool == tools && listed(km.overrides(), e)).map(|e| format!("{}\t", english_name(e))).collect()
            };
            let rows = (names(true), names(false));
            (text, km, rows.0, rows.1)
        });
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[..4], ["Keyboard Shortcuts", "Set: VectorCraft Defaults", "", "Tools"]);
        let menu_at = lines.iter().position(|l| *l == "Menu Commands").expect("a Menu Commands section");
        assert_eq!(lines[menu_at - 1], "", "a blank line before Menu Commands");
        let names_of = |rows: &[&str]| -> Vec<String> { rows.iter().map(|l| format!("{}\t", l.split('\t').next().unwrap_or_default())).collect() };
        assert_eq!(names_of(&lines[4..menu_at - 1]), tool_rows, "the Tools rows, in the dialog's order");
        assert_eq!(names_of(&lines[menu_at + 1..]), menu_rows, "the Menu Commands rows, in the dialog's order");
        let pen = menus::pretty_shortcut(default_of("tool:pen").expect("the Pen tool has a default key"));
        assert!(lines[4..menu_at].contains(&text_row("tool:pen", &pen).as_str()), "{text}");
        let prefs = menus::pretty_shortcut(&effective_with(&km, "edit.preferences").expect("the shortcut set"));
        assert!(prefs.ends_with('9'), "{prefs}");
        assert!(lines[menu_at..].contains(&text_row("edit.preferences", &prefs).as_str()), "{text}");
        assert!(text.ends_with('\n'));
    }

    /// A command without a shortcut is listed with nothing after the tab: one without a default key
    /// and one whose shortcut the set clears.
    #[test]
    fn export_text_lists_a_command_without_a_shortcut_with_an_empty_shortcut() {
        let mut ov = Overrides::new();
        crate::keymap::set_override(&mut ov, "edit.preferences", "");
        assert!(default_of("align.horizontalLeft").is_none());
        let text = export_text(CUSTOM, &ov);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "Set: Custom");
        for key in ["align.horizontalLeft", "edit.preferences"] {
            assert!(lines.contains(&text_row(key, "").as_str()), "{key}: {text}");
        }
    }

    /// `shortcuts.exportText` writes the set's name as the set was named, and a path that can't be
    /// written or isn't a string gives an error and writes nothing.
    #[test]
    fn export_text_command_writes_the_sets_name_and_rejects_a_bad_path() {
        type Files = std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>>;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let written: Files = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |path: &str, b: &[u8]| {
            if path.starts_with("/missing/") {
                return Err(format!("can't write {path}"));
            }
            w.borrow_mut().push((path.to_string(), String::from_utf8(b.to_vec()).map_err(|e| e.to_string())?));
            Ok(())
        }));
        app.run("shortcuts.import", json!({"data": export_json("Studio", &Overrides::new())})).unwrap();
        let expected = with_steady_plugins(|| {
            written.borrow_mut().clear();
            assert_eq!(app.run("shortcuts.exportText", json!({"path": "/sets/Studio.txt"})), Ok(json!({"path": "/sets/Studio.txt"})));
            export_text("Studio", &Overrides::new())
        });
        assert_eq!(
            app.run("shortcuts.exportText", json!({"path": "/missing/dir/Studio.txt"})),
            Err("can't write /missing/dir/Studio.txt".to_string())
        );
        assert_eq!(app.run("shortcuts.exportText", json!({"path": 5})), Err("`path` must be a string".to_string()));
        let files = written.borrow();
        let [(path, text)] = files.as_slice() else { panic!("{files:?}") };
        assert_eq!(path, "/sets/Studio.txt");
        assert!(text.starts_with("Keyboard Shortcuts\nSet: Studio\n\nTools\n"), "{text}");
        assert_eq!(*text, expected);
    }

    /// The sheet's sections: Tools first, then each top-level menu once, in the
    /// order the dialog first lists a command of it, holding every row the dialog lists with its
    /// shortcut, the command named without its top-level menu.
    #[test]
    fn export_sections_hold_every_listed_row_once_by_top_level_menu() {
        let mut ov = Overrides::new();
        crate::keymap::set_override(&mut ov, "edit.preferences", "Ctrl+Alt+Shift+9");
        let (sections, listed_rows, tops) = with_steady_plugins(|| {
            let km = keymap_with(&ov);
            let listed: Vec<Entry> = entries().iter().filter(|e| listed(km.overrides(), e)).cloned().collect();
            let mut tops: Vec<String> = vec![];
            for e in listed.iter().filter(|e| !e.is_tool) {
                let top = e.group.split(" › ").next().filter(|t| !t.is_empty()).unwrap_or(MENU_COMMANDS).to_string();
                if !tops.contains(&top) {
                    tops.push(top);
                }
            }
            (export_sections(&ov, &Holds::new()), listed.len(), tops)
        });
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles[0], "Tools");
        assert_eq!(titles[1..], tops.iter().map(String::as_str).collect::<Vec<_>>()[..], "one section per top-level menu");
        for menu in ["File", "Edit", "Object", "Type", "Select", "Effect", "View", "Window", "Help"] {
            assert!(titles.contains(&menu), "no {menu} section: {titles:?}");
        }
        assert_eq!(sections.iter().map(|s| s.rows.len()).sum::<usize>(), listed_rows, "every listed row once");
        let prefs = entry("edit.preferences").expect("the Preferences row");
        let top = prefs.group.split(" › ").next().unwrap_or_default().to_string();
        let name = shown_name(&prefs).strip_prefix(&format!("{top} › ")).map(str::to_string).expect("Preferences is in a menu");
        let section = sections.iter().find(|s| s.title == top).expect("its menu's section");
        assert!(section.rows.iter().any(|r| r.name == name && r.shortcut.ends_with('9')), "{name} in {:?}", section.rows);
        let pen = sections[0].rows.iter().find(|r| r.shortcut == menus::pretty_shortcut(default_of("tool:pen").unwrap_or_default()));
        assert!(pen.is_some(), "the Pen tool's row");
    }

    /// `format` picks the file: text (the default, the same bytes as before) or the PDF sheet. A
    /// given path is written as text whatever its extension, and a bad `format` or `pageSize` is an
    /// error that writes nothing.
    #[test]
    fn export_text_command_writes_text_or_a_sheet() {
        type Files = std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>>;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let written: Files = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |path: &str, b: &[u8]| {
            w.borrow_mut().push((path.to_string(), b.to_vec()));
            Ok(())
        }));
        let (files, text) = with_steady_plugins(|| {
            written.borrow_mut().clear();
            app.run("shortcuts.exportText", json!({"path": "/a/plain.txt"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/named.txt", "format": "text"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/as-text.pdf"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/sheet.pdf", "format": "sheet", "pageSize": "a4"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/sheet.txt", "format": "sheet"})).unwrap();
            (written.borrow().clone(), export_text(&app.ui.shortcut_set, &app.ui.shortcut_overrides))
        });
        let [plain, named, as_text, sheet, sheet_as_named] = files.as_slice() else { panic!("{} files", files.len()) };
        for f in [plain, named, as_text] {
            assert_eq!(f.1, text.as_bytes(), "{}", f.0);
        }
        assert!(sheet.1.starts_with(b"%PDF-"), "{}", sheet.0);
        assert_eq!(sheet_as_named.0, "/a/sheet.txt", "a path is written as given");
        assert!(sheet_as_named.1.starts_with(b"%PDF-"));
        written.borrow_mut().clear();
        let mut err = |p: Value| app.run("shortcuts.exportText", p);
        assert_eq!(err(json!({"path": "/a/x", "format": "xlsx"})), Err("`format` must be one of text, sheet, csv".to_string()));
        assert_eq!(err(json!({"path": "/a/x", "format": "Sheet"})), Err("`format` must be one of text, sheet, csv".to_string()));
        assert_eq!(err(json!({"path": "/a/x", "format": 3})), Err("`format` must be a string".to_string()));
        assert_eq!(err(json!({"path": "/a/x", "pageSize": "legal"})), Err("`pageSize` must be one of letter, a4".to_string()));
        assert_eq!(err(json!({"path": "/a/x", "pageSize": "A4"})), Err("`pageSize` must be one of letter, a4".to_string()));
        assert!(written.borrow().is_empty());
    }

    /// `format: "csv"` writes the CSV file: the `Group,Command,Shortcut` header, then a record per
    /// row the dialog lists, every group and row of [`export_sections`] once. Without a file it
    /// comes back under `text`.
    #[test]
    fn export_text_command_writes_a_csv_file() {
        type Files = std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>>;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let written: Files = Default::default();
        let (returned, files, sections) = with_steady_plugins(|| {
            app.services.write = None;
            written.borrow_mut().clear();
            let returned = app.run("shortcuts.exportText", json!({"format": "csv"}));
            let w = written.clone();
            app.services.write = Some(Box::new(move |path: &str, b: &[u8]| {
                w.borrow_mut().push((path.to_string(), b.to_vec()));
                Ok(())
            }));
            app.run("shortcuts.exportText", json!({"path": "/a/list.csv", "format": "csv"})).unwrap();
            (returned, written.borrow().clone(), export_sections(&app.ui.shortcut_overrides, &Holds::new()))
        });
        let [(path, bytes)] = files.as_slice() else { panic!("{} files", files.len()) };
        assert_eq!(path, "/a/list.csv");
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert_eq!(returned.unwrap()["text"].as_str(), Some(text.as_str()), "the same file without a writer");
        assert_eq!(text, crate::shortcut_sheet::csv(&sections, ["Group", "Command", "Shortcut"]));
        assert!(text.starts_with("\u{feff}Group,Command,Shortcut\r\nTools,"), "{text}");
        let records = text.matches("\r\n").count();
        assert_eq!(records, 1 + sections.iter().map(|s| s.rows.len()).sum::<usize>(), "a header and a record per row");
        for s in &sections {
            assert!(text.contains(&format!("\r\n{},", s.title)), "no {} group", s.title);
        }
    }

    /// With no `pageSize` the sheet is on A4 when the current document prints on A4: its print
    /// settings say so, or, with none saved, its first artboard is A4 either way up; else Letter.
    #[test]
    fn the_sheet_is_on_a4_when_the_document_prints_on_a4() {
        use crate::shortcut_sheet::PageSize;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(default_sheet_page(&app), PageSize::Letter, "no document");
        app.session.add_document(vectorcraft_doc::Document::new(612.0, 792.0), None);
        assert_eq!(default_sheet_page(&app), PageSize::Letter, "a Letter artboard");
        app.session.add_document(vectorcraft_doc::Document::new(595.28, 841.89), None);
        assert_eq!(default_sheet_page(&app), PageSize::A4, "an A4 artboard");
        let printing = |w, h, media| {
            let mut doc = vectorcraft_doc::Document::new(w, h);
            doc.print_setup = serde_json::to_value(vectorcraft_pdf::PrintSettings { media, ..Default::default() }).ok();
            doc
        };
        app.session.add_document(printing(595.28, 841.89, vectorcraft_pdf::Media::Letter), None);
        assert_eq!(default_sheet_page(&app), PageSize::Letter, "an A4 artboard printing on Letter");
        app.session.add_document(printing(612.0, 792.0, vectorcraft_pdf::Media::A4), None);
        assert_eq!(default_sheet_page(&app), PageSize::A4, "printing on A4");
    }

    /// Without a path, a file writer or a download, `shortcuts.exportText` returns the file: the
    /// text file under `text` and the sheet as base64 under `pdf`.
    #[test]
    fn export_text_without_a_file_returns_it() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let (text, sheet) =
            with_steady_plugins(|| (app.run("shortcuts.exportText", json!({})), app.run("shortcuts.exportText", json!({"format": "sheet"}))));
        let text = text.unwrap();
        assert!(text["text"].as_str().unwrap_or_default().starts_with("Keyboard Shortcuts\n"), "{text}");
        let sheet = sheet.unwrap();
        // "%PDF-" in base64.
        assert!(sheet["pdf"].as_str().unwrap_or_default().starts_with("JVBERi0"), "{}", sheet.to_string().len());
    }

    /// The save dialog's answer picks the format: the file type it reports, else the extension
    /// typed; text for any other.
    #[test]
    fn the_save_dialogs_answer_picks_the_export_text_format() {
        assert_eq!(TextFormat::picked("/a/sheet.PDF", None), TextFormat::Sheet);
        assert_eq!(TextFormat::picked("/a/list", Some("pdf")), TextFormat::Sheet);
        assert_eq!(TextFormat::picked("/a/list.pdf", Some("txt")), TextFormat::Text);
        assert_eq!(TextFormat::picked("/a/list.txt", None), TextFormat::Text);
        assert_eq!(TextFormat::picked("/a/list", None), TextFormat::Text);
        assert_eq!(TextFormat::picked("/a/list.json", None), TextFormat::Text);
        assert_eq!(TextFormat::with_extension("/a/list", TextFormat::Sheet), "/a/list.pdf");
        assert_eq!(TextFormat::with_extension("/a/list.txt", TextFormat::Sheet), "/a/list.pdf");
        assert_eq!(TextFormat::with_extension("/a/list", TextFormat::Text), "/a/list.txt");
        assert_eq!(TextFormat::picked("/a/list.CSV", None), TextFormat::Csv);
        assert_eq!(TextFormat::picked("/a/list", Some("csv")), TextFormat::Csv);
        assert_eq!(TextFormat::with_extension("/a/list.pdf", TextFormat::Csv), "/a/list.csv");
        assert_eq!(TextFormat::with_extension("/a/list.PDF", TextFormat::Sheet), "/a/list.PDF");
        assert_eq!(TextFormat::with_extension("/a/v1.2", TextFormat::Sheet), "/a/v1.2.pdf");
    }

    /// The dialog's Import… leaves an action its function key when the imported set gives the key
    /// to a command, through both Cancel and OK.
    #[test]
    fn importing_into_the_dialog_preserves_action_keys_on_ok() {
        use crate::panels::actions::{Action, ActionSet, FunctionKey};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let f15 = FunctionKey { key: 15, cmd: false, shift: false };
        let mut nudge = Action::new("Nudge", vec![]);
        nudge.key = Some(f15);
        app.ui.action_sets.push(ActionSet::new("Mine", vec![nudge]));
        let mut ov = Overrides::new();
        crate::keymap::set_override(&mut ov, "edit.duplicate", "F15");
        let bytes = serde_json::to_vec(&export_json("Imported", &ov)).unwrap();
        app.services.pick_open = Some(Box::new(|_: &crate::FilePick| Some("/sets/Imported.json".to_string())));
        app.services.read = Some(Box::new(move |_: &str| Ok(bytes.clone())));
        let key = |app: &VectorcraftApp| app.ui.action_sets.last().and_then(|s| s.actions.first()).and_then(|a| a.key);
        open(&mut app);
        import_into_dialog(&mut app);
        assert_eq!(key(&app), Some(f15), "the import alone keeps the action's key");
        assert!(app.ui.shortcut_overrides.is_empty(), "the import is only in the dialog");
        app.ui.dialog = None;
        assert_eq!(key(&app), Some(f15), "Cancel keeps the action's key");
        open(&mut app);
        import_into_dialog(&mut app);
        assert_eq!(key(&app), Some(f15));
        confirm(&mut app).unwrap();
        assert_eq!(key(&app), Some(f15), "OK keeps the action's function key");
    }

    #[test]
    fn presets_are_the_defaults() {
        for p in PRESETS {
            assert_eq!(preset(p), Some(Overrides::new()));
        }
        assert!(preset("Nope").is_none());
    }

    #[test]
    fn paint_dialog_rows_show_both_default_keys() {
        let view = DialogView::new(Overrides::new());
        for (key, plain, shifted) in [("paint.lastColor", ",", "Shift+Comma"), ("paint.lastGradient", ".", "Shift+Period")] {
            let at = view.entries.iter().position(|e| e.key == key).unwrap();
            let shown = view.texts[at].as_deref().unwrap();
            let shifted = menus::pretty_shortcut(shifted);
            assert!(shown.contains(plain) && shown.contains(&shifted), "{key}: {shown}");
        }
        assert_eq!(view.conflicts, 0);
    }

    #[test]
    fn edit_dialog_rows_show_primary_and_function_keys() {
        for p in [craft_keymap::Platform::Windows, craft_keymap::Platform::Linux, craft_keymap::Platform::Mac] {
            let km = crate::keymap::keymap_for(p, &snapshot());
            for (key, primary, alternate) in [("edit.cut", "Cmd+X", "F2"), ("edit.copy", "Cmd+C", "F3"), ("edit.paste", "Cmd+V", "F4")] {
                let shown = dialog_shortcuts_with(&km, key, None).unwrap();
                let primary = menus::pretty_shortcut(primary);
                assert!(shown.contains(&primary), "{p:?} {key}: {shown}");
                assert_eq!(shown.contains(alternate), !p.is_mac(), "{p:?} {key}: {shown}");
            }
        }
        for (key, primary) in [("edit.cut", "Cmd+X"), ("edit.copy", "Cmd+C"), ("edit.paste", "Cmd+V")] {
            let shown = menus::item_shortcut(key, &Value::Null).map(menus::pretty_shortcut);
            assert_eq!(shown, Some(menus::pretty_shortcut(primary)), "{key} menu shortcut");
        }
    }

    #[test]
    fn saved_function_key_assignment_does_not_change_cut_in_the_editor() {
        if crate::keymap::platform().is_mac() {
            return;
        }
        let mut saved = Overrides::new();
        crate::keymap::set_override(&mut saved, "tool:pen", "F2");
        let mirror = Mirror::new(saved.clone());
        assert!(!mirror.text.contains_key("edit.cut"));
        assert!(conflicts_in(&mirror.keymap).is_empty());

        let view = DialogView::new(saved.clone());
        assert_eq!(view.source, stored::to_value(&saved));
        assert_eq!(view.conflicts, 0);
        assert_eq!(dialog_shortcuts_with(&view.keymap, "edit.cut", None), Some(menus::pretty_shortcut("Cmd+X")));

        let mut other = saved.clone();
        assign(&mut other, "edit.preferences", Some("Cmd+Alt+Shift+F12"), false).unwrap();
        assert!(!crate::keymap::has_override(&other, "edit.cut"));
        let (set, imported) = import_json(&export_json("Mine", &other)).unwrap();
        assert_eq!(set, "Mine");
        assert_eq!(imported, other);
        assert!(!crate::keymap::has_override(&imported, "edit.cut"));

        let mut km = view.keymap.clone();
        assign_in(&mut km, "edit.preferences", Some("Cmd+Alt+Shift+F12"), false).unwrap();
        assert_eq!(saved_overrides(km.overrides(), &saved), other);
    }

    #[test]
    fn reset_stolen_edit_function_keys_restores_their_defaults() {
        if crate::keymap::platform().is_mac() {
            return;
        }
        for (alias, edit) in [("F2", "edit.cut"), ("F3", "edit.copy"), ("F4", "edit.paste")] {
            let mut saved = Overrides::new();
            assert_eq!(assign(&mut saved, "tool:pen", Some(alias), true).unwrap(), vec![edit]);
            assert!(!crate::keymap::has_override(&saved, edit), "{alias} saved edit override");
            reset_one(&mut saved, "tool:pen");
            assert_eq!(crate::keymap::lookup(&keymap_with(&saved), &crate::keymap::vc_chord(alias).unwrap()), Some(command_of(edit)));

            let mut d = Dialog::new("shortcuts", json!({"overrides": {}}));
            let source = dialog_overrides(&d);
            let mut km = keymap_with(&source);
            record(&mut d, &mut km, "tool:pen", alias);
            d.fields.insert("overrides".into(), stored::to_value(&saved_overrides(km.overrides(), &source)));
            let mut dialog_saved = dialog_overrides(&d);
            assert!(!crate::keymap::has_override(&dialog_saved, edit), "{alias} dialog edit override");
            reset_one(&mut dialog_saved, "tool:pen");
            assert_eq!(crate::keymap::lookup(&keymap_with(&dialog_saved), &crate::keymap::vc_chord(alias).unwrap()), Some(command_of(edit)));

            let mut changed = Overrides::new();
            crate::keymap::set_override(&mut changed, edit, "Ctrl+Alt+Shift+F12");
            assign(&mut changed, "tool:pen", Some(alias), true).unwrap();
            assert_eq!(crate::keymap::override_text(&changed, edit).as_deref(), Some("Ctrl+Alt+Shift+F12"));
        }
    }

    #[test]
    fn reassigning_a_saved_function_key_gives_it_back_to_its_edit_command() {
        if crate::keymap::platform().is_mac() {
            return;
        }
        for (alias, edit) in [("F2", "edit.cut"), ("F3", "edit.copy"), ("F4", "edit.paste")] {
            let mut saved = Overrides::new();
            crate::keymap::set_override(&mut saved, "edit.preferences", alias);
            assign(&mut saved, "edit.preferences", Some("Cmd+Alt+Shift+F12"), false).unwrap();
            assert!(!crate::keymap::has_override(&saved, edit), "{alias} saved edit override");
            assert_eq!(crate::keymap::lookup(&keymap_with(&saved), &crate::keymap::vc_chord(alias).unwrap()), Some(command_of(edit)));

            let mut source = Overrides::new();
            crate::keymap::set_override(&mut source, "edit.preferences", alias);
            let mut d = Dialog::new("shortcuts", json!({"overrides": stored::to_value(&source)}));
            let mut km = keymap_with(&source);
            record(&mut d, &mut km, "edit.preferences", "Cmd+Alt+Shift+F12");
            let dialog_saved = saved_overrides(km.overrides(), &source);
            assert!(!crate::keymap::has_override(&dialog_saved, edit), "{alias} dialog edit override");
            assert_eq!(crate::keymap::lookup(&keymap_with(&dialog_saved), &crate::keymap::vc_chord(alias).unwrap()), Some(command_of(edit)));
        }
    }

    #[test]
    fn assigning_an_edit_function_key_elsewhere_reports_the_conflict() {
        if crate::keymap::platform().is_mac() {
            return;
        }
        for (alias, edit) in [("F2", "edit.cut"), ("F3", "edit.copy"), ("F4", "edit.paste")] {
            let label = entry(edit).map(|e| english_name(&e)).unwrap();
            assert_eq!(conflicts_with(&Overrides::new(), alias, "tool:pen"), vec![edit.to_string()], "{alias}");
            let mut ov = Overrides::new();
            let err = assign(&mut ov, "tool:pen", Some(alias), false).unwrap_err();
            assert!(err.ends_with(&format!("is already used by {label}")), "{alias}: {err}");
            assert!(!crate::keymap::has_override(&ov, "tool:pen"), "{alias}: a refused assignment saves nothing");

            let mut d = Dialog::new("shortcuts", json!({"overrides": {}}));
            let mut km = keymap_with(&Overrides::new());
            record(&mut d, &mut km, "tool:pen", alias);
            assert_eq!(d.str("__conflict"), edit, "{alias}");
            assert!(d.str("__message").contains(&label), "{alias}: {}", d.str("__message"));
            assert_eq!(crate::keymap::lookup(&km, &crate::keymap::vc_chord(alias).unwrap()), Some(command_of("tool:pen")), "{alias}");
        }
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
        assert_eq!(entry("edit.duplicate").as_ref().map(|e| e.group.as_str()), Some("Edit"));
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
        // The Symbols panel runs New Symbol through Symbol Options, so it waits for a key the user gives it.
        assert!(entry("symbol.new").is_some_and(|e| !listed(&Overrides::new(), &e)));
        for (doc, bare) in [
            ("{}", true),
            ("{dx?, dy?} x", true),
            ("{id?|ids?, clip?}", true),
            ("{name, newName?}", false),
            ("{a?: {b}}", true),
            ("{ids?|id?, on?: bool} turn it on or off", true),
            ("{ids?: [id…] (the selection), anchors?: [index…]}", true),
            ("{format?: a|b, size?: 1|3|5}", true),
            ("{artboard?: index=0}", true),
            ("{mode: a|b, on?: bool}", false),
            ("same as", false),
        ] {
            assert_eq!(runs_without_params(doc), bare, "{doc}");
        }
        // Panel menu commands whose params doc gives each optional field a type are listed too.
        let none = Overrides::new();
        for id in [
            "type.fractionalWidths",
            "type.systemLayout",
            "object.setPixelAligned",
            "links.embed",
            "links.update",
            "object.slice.delete",
            "artboard.switchOrientation",
            "appearance.targetContents",
            "paint.invert",
            "artboard.distributeSpacing",
        ] {
            assert!(entry(id).is_some_and(|e| listed(&none, &e)), "{id}");
        }
        let mut ov = Overrides::new();
        assert!(assign(&mut ov, "symbol.breakLink", Some("Cmd+Alt+Shift+F9"), true).is_ok());
        assert_eq!(effective_in(&ov, "symbol.breakLink").as_deref(), Some("Cmd+Alt+Shift+F9"));
    }

    /// The Menu Commands rows the dialog draws for `query`.
    fn menu_rows(app: &mut VectorcraftApp, query: &str) -> Vec<String> {
        open(app);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("tab".into(), json!("menu"));
        d.fields.insert("query".into(), json!(query));
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        // The modal draws nothing in its first frame, which only sizes it.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        app.ui.dialog = None;
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().contains(" › ") => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }

    /// An open dialog lists a live-effect plug-in installed while it is open, with the shortcut its
    /// working set gives it, and drops the row while the plug-in is removed, keeping that shortcut
    /// in the working set.
    #[test]
    fn an_open_dialog_follows_live_effect_plugins() {
        use vectorcraft_plugins::wat as tpl;
        let (plugin, key) = ("test.ui.openfx", "effect:plugin.test.ui.openfx");
        let wasm = wat::parse_str(tpl::module(&tpl::manifest(plugin, "effect", r#"{"wiggle":{"type":"bool"}}"#), tpl::ECHO, "", 1)).unwrap();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        open(&mut app);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("tab".into(), json!("menu"));
        d.fields.insert("query".into(), json!("test.ui.openfx"));
        let mut working = Overrides::new();
        crate::keymap::set_override(&mut working, key, "Cmd+Alt+Shift+F5");
        d.fields.insert("overrides".into(), stored::to_value(&working));
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let texts = |app: &mut VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let row = "Effect › Plug-ins › Test test.ui.openfx…";
        let shortcut = menus::pretty_shortcut("Cmd+Alt+Shift+F5");
        // The modal draws nothing in its first frame, which only sizes it.
        texts(&mut app);
        assert!(!texts(&mut app).iter().any(|t| t == row));
        vectorcraft_plugins::registry::install_bytes(&wasm).unwrap();
        let shown = texts(&mut app);
        assert!(shown.iter().any(|t| t == row) && shown.contains(&shortcut), "{shown:?}");
        vectorcraft_plugins::registry::remove(plugin);
        assert!(!texts(&mut app).iter().any(|t| t == row));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(crate::keymap::override_text(&dialog_overrides(d), key).as_deref(), Some("Cmd+Alt+Shift+F5"));
        vectorcraft_plugins::registry::install_bytes(&wasm).unwrap();
        let shown = texts(&mut app);
        assert!(shown.iter().any(|t| t == row) && shown.contains(&shortcut), "{shown:?}");
        vectorcraft_plugins::registry::remove(plugin);
    }

    /// A menu command whose params need a value (Units needs `{units}`) is not listed while it has
    /// no shortcut. A shortcut the user gave it keeps it listed, where it can be changed or cleared.
    #[test]
    fn commands_that_need_a_value_are_listed_only_with_the_users_shortcut() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let list = run_command(&mut app, "shortcuts.list", &json!({"query": "document.setUnits"})).unwrap().unwrap();
        assert_eq!(list, json!([]));
        assert!(!menu_rows(&mut app, "units").iter().any(|r| r.ends_with(" › Units")));
        // A key saved for it before stays listed.
        crate::keymap::set_override(&mut app.ui.shortcut_overrides, "document.setUnits", "Cmd+Alt+Shift+F12");
        let list = run_command(&mut app, "shortcuts.list", &json!({"query": "document.setUnits"})).unwrap().unwrap();
        let e = &list[0];
        assert_eq!(
            (e["id"].as_str(), e["shortcut"].as_str(), e["overridden"].as_bool()),
            (Some("document.setUnits"), Some("Cmd+Alt+Shift+F12"), Some(true))
        );
        assert!(menu_rows(&mut app, "units").iter().any(|r| r == "File › Document Setup › Units"));
        // An empty saved shortcut (as an imported set can hold) gives it no key, so it is not listed.
        crate::keymap::set_override(&mut app.ui.shortcut_overrides, "document.setUnits", "");
        let list = run_command(&mut app, "shortcuts.list", &json!({"query": "document.setUnits"})).unwrap().unwrap();
        assert_eq!(list, json!([]));
        // Every entry with a default shortcut is listed, and so are commands a key press runs: one
        // whose params are all optional, one the menu bar runs without params (Rotate… opens its
        // dialog) and one that opens a dialog for its values (New Variable).
        let none = Overrides::new();
        let all = entries();
        let dropped: Vec<&str> = all.iter().filter(|e| default_of(&e.key).is_some() && !listed(&none, e)).map(|e| e.key.as_str()).collect();
        assert!(dropped.is_empty(), "{dropped:?}");
        for id in ["edit.duplicate", "object.pathfinder.unite", "object.rotate", "variable.define", "object.distributeSpacing"] {
            assert!(entry(id).is_some_and(|e| listed(&none, &e)), "{id}");
        }
        for (doc, required) in [
            ("{}", false),
            ("{units: \"Points\"|\"Picas\"} the units", true),
            ("{ids?: [id…] (selected first), dx?, dy?}", false),
            ("{name? (trimmed, as `to`), attrs?: {…}}", false),
            ("{columns=10, rows=10 (whole numbers)}", false),
            ("{index | indices: [index…]}", true),
            ("{index?: n | indices?: [n…]}", false),
            ("{…as document.save} {path?}", false),
            ("{find, replace?: \"\"}", true),
            ("same as file.save", false),
        ] {
            assert_eq!(names_a_required_field(doc), required, "{doc}");
        }
    }

    /// Commands whose params are all optional but that a key press can't run are not listed while
    /// they have no shortcut: Transparency and Opacity Mask Options set nothing without a value,
    /// Align needs a direction, Other Library… a file, and a query's answer (Select All Unused)
    /// is dropped. A key saved for one before keeps it listed, where it can be cleared.
    #[test]
    fn commands_a_key_press_cannot_run_are_not_listed() {
        let none = Overrides::new();
        // Edit › Find Next's menu item gives an empty `find`, which it rejects.
        let others = ["swatch.unused", "graphicStyle.unused", "document.info", "view.proofSetup", "magicWand.set", "edit.findNext"];
        for id in NEEDS_A_PARAM.iter().chain(&others) {
            let e = entry(id).unwrap_or_else(|| panic!("{id} has an entry"));
            assert!(!listed(&none, &e), "{id}");
            assert_eq!(default_of(id), None, "{id}");
        }
        let mut ov = Overrides::new();
        crate::keymap::set_override(&mut ov, "transparency.set", "Cmd+Alt+Shift+F10");
        assert!(entry("transparency.set").is_some_and(|e| listed(&ov, &e)));
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        for id in ["transparency.set", "transparency.setOpacityMask"] {
            let list = run_command(&mut app, "shortcuts.list", &json!({ "query": id })).unwrap().unwrap();
            assert_eq!(list, json!([]), "{id}");
        }
        // Commands that work without params stay listed: panel menu commands that act on the
        // selection, toggles, and a menu item that runs with params of its own (Clean Up…).
        for id in [
            "transparency.releaseOpacityMask",
            "transparency.toggleNewMasksClipping",
            "object.path.cleanUp",
            "addon.color.printFixes",
            "edit.findReplace",
        ] {
            assert!(entry(id).is_some_and(|e| listed(&none, &e)), "{id}");
        }
        assert_eq!(menu_target("object.path.cleanUp").map(|(id, _)| id).as_deref(), Some("ui.paramDialog"));
        assert_eq!(
            menu_target("addon.color.printFixes").map(|(id, p)| (id, p["command"].clone(), p["params"]["inkLimit"].clone())),
            Some(("ui.paramDialog".into(), json!("addon.color.printFixes"), json!(300)))
        );
        assert_eq!(menu_target("edit.findReplace").map(|(id, _)| id).as_deref(), Some("ui.paramDialog"));
        assert_eq!(menu_target("object.align"), None, "seven items, each with its own params");
        assert_eq!(menu_target("edit.undo"), None, "runs without params");
    }

    /// The menu bar items a shortcut or a command search pick reads leave out items that list an
    /// installed plug-in (a File › Open With importer, an Object › Plug-ins filter) or show a name
    /// that is user data (a custom toolbar, a script of the Scripts folder), which come and go after
    /// the items are read; the other items stay.
    #[test]
    fn menu_bar_items_leave_out_installed_plug_ins_and_user_data() {
        let tree = vec![(
            "File",
            vec![
                menus::Item::Sub("Open With", vec![menus::Item::Cmd("Spins…", "file.openWith", json!({"importer": "plugin.test.spins"}))]),
                menus::Item::Cmd("Filter…", "plugin.dialog", json!({"id": "test.filter"})),
                menus::Item::Cmd("Mine", "window.toolbar.custom", json!({"name": "Mine"})),
                menus::Item::Cmd("tidy.js", crate::scripts_menu::COMMAND, json!({"path": "/scripts/tidy.js"})),
                menus::Item::Cmd("Clean Up…", "object.path.cleanUp", json!({"strayPoints": true})),
            ],
        )];
        let items = bar_items(&tree);
        assert!(!items.contains_key("file.openWith"), "{items:?}");
        assert!(!items.contains_key("plugin.dialog"), "{items:?}");
        assert!(!items.contains_key("window.toolbar.custom"), "{items:?}");
        assert!(!items.contains_key(crate::scripts_menu::COMMAND), "{items:?}");
        assert_eq!(items.get("object.path.cleanUp"), Some(&vec![("Clean Up…", json!({"strayPoints": true}))]));
    }

    /// While a menu item is dimmed, a pick or a shortcut gets a status line and no target: with the
    /// engine's reason when it gives one, and without one when the app's own rules dim the item (an
    /// Appearance command with nothing targeted, a UI command).
    #[test]
    fn a_dimmed_item_gives_a_status_line_with_or_without_a_reason() {
        use vectorcraft_testkit::fixtures::Fixture;
        let none = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        assert!(!menus::enabled(&none, "edit.findReplace"));
        let why = "command `edit.findReplace` is not available right now: no document open";
        assert_eq!(dimmed(&none, "edit.findReplace", "edit.findReplace").as_deref(), Some(why));
        // A name to show takes the id's place.
        let shown = "command `Edit › Find and Replace…` is not available right now: no document open";
        assert_eq!(dimmed(&none, "edit.findReplace", "Edit › Find and Replace…").as_deref(), Some(shown));
        let empty = VectorcraftApp::new(Fixture::Empty.session(), crate::Services::default());
        for id in ["appearance.addFill", "ui.cropImage"] {
            assert!(!menus::enabled(&empty, id), "{id}");
            assert!(vectorcraft_engine::find_command(id).is_none_or(|c| (c.enabled)(&empty.session).is_ok()), "{id}: the engine gives no reason");
            assert_eq!(dimmed(&empty, id, id), Some(format!("command `{id}` is not available right now")));
        }
        assert!(menus::enabled(&empty, "edit.findReplace"));
        assert_eq!(dimmed(&empty, "edit.findReplace", "edit.findReplace"), None, "not dimmed");
    }

    /// Every command the editor lists, run as a key press runs it on a document with one or with
    /// several objects selected, reports no missing param.
    #[test]
    fn listed_commands_run_from_a_key_without_a_missing_param() {
        use vectorcraft_testkit::fixtures::Fixture;
        // These need a state the fixtures lack: text being edited, a pattern being edited, an
        // object linked to a graphic style, a plug-ins folder.
        let context = ["text.insertInline", "pattern.options", "graphicStyle.redefine", "plugin.reload"];
        let none = Overrides::new();
        let mut failures = vec![];
        for e in entries().iter().filter(|e| !e.is_tool && !e.key.starts_with("panel:") && listed(&none, e)) {
            if ["file.", "app.", "help."].iter().any(|p| e.key.starts_with(p)) || context.contains(&e.key.as_str()) {
                continue;
            }
            for fx in [Fixture::Single, Fixture::Multi] {
                let mut app = VectorcraftApp::new(fx.session(), Default::default());
                app.ui.status.clear();
                let (id, p) = menu_target(&e.key).unwrap_or_else(|| (e.key.clone(), json!({})));
                menus::invoke(&mut app, &id, p);
                let s = &app.ui.status;
                if s.starts_with("invalid parameters for") && (s.contains(": give ") || s.contains(": missing ")) {
                    failures.push(format!("{} [{fx:?}]: {s}", e.key));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
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

    /// Every engine or UI command that an item of the menu bar runs as a key press runs it (without
    /// params, or with the one set of params all its items give) is listed in Edit › Keyboard
    /// Shortcuts, also when the command has no menu path of its own: Set Corners…, whose params doc
    /// names a field a call must give, the Perspective Grid toggles, Edit Custom Dictionary…, a
    /// query, and Recolor Artwork…, a UI command. Such a command is listed in the group of its id,
    /// next to the commands of its family that were listed before. A shortcut given to one survives
    /// a saved set's round trip. Find Next is left out: its menu item gives `find` empty, which the
    /// command rejects. When another entry has the same group and label, the command the menu item
    /// runs keeps the name if no menu item runs the other: File › Export Selection… is
    /// `file.exportSelection`, and `document.exportSelection` moves to Document. Otherwise the
    /// command is left out: Window › Learn is the Learn panel's entry, not `window.learn`.
    #[test]
    fn every_command_the_menu_bar_runs_can_be_given_a_shortcut() {
        let none = Overrides::new();
        let mut missing: Vec<&str> = menu_bar_items()
            .iter()
            .filter(|(id, items)| items.iter().any(|(_, p)| is_bare(p)) || menu_target(id).is_some())
            .map(|(id, _)| *id)
            .filter(|id| *id != "edit.findNext" && *id != "window.learn")
            .filter(|id| vectorcraft_engine::find_command(id).is_some() || menus::UI_COMMANDS.iter().any(|c| c.0 == *id))
            .filter(|id| !entry(id).is_some_and(|e| listed(&none, &e)))
            .collect();
        missing.sort_unstable();
        assert!(missing.is_empty(), "{missing:?}");
        assert!(entry("window.learn").is_none(), "Window › Learn is the Learn panel's entry");
        let shown = |id: &str| entry(id).map(|e| (e.group, e.label));
        assert_eq!(shown("file.exportSelection"), Some(("File".into(), "Export Selection…".into())));
        assert_eq!(shown("document.exportSelection"), Some(("Document".into(), "Export Selection…".into())));
        // No command listed only because an item of the menu bar runs it shares its group and label
        // with another entry.
        let own = |k: &str| {
            vectorcraft_engine::command_specs()
                .iter()
                .find(|c| c.id == k)
                .map(listed_on_its_own)
                .or_else(|| menus::UI_COMMANDS.iter().find(|c| c.0 == k).map(|(id, _, sc, p)| ui_listed_on_its_own(id, sc, p)))
                .unwrap_or(true)
        };
        let (v, tail) = fixed_entries();
        let mut names: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
        for e in v.iter().chain(tail) {
            names.entry((e.group.as_str(), e.label.as_str())).or_default().push(e.key.as_str());
        }
        let shared: Vec<_> = names.iter().filter(|(_, keys)| keys.len() > 1 && keys.iter().any(|k| !own(k))).collect();
        assert!(shared.is_empty(), "{shared:?}");
        for (id, group) in [
            (crate::addons::pathtools::set_corners::COMMAND, "Addon"),
            ("perspective.grid.snap", "Perspective"),
            (crate::dialogs::custom_dictionary::COMMAND, "Text"),
            ("ui.recolorDialog", "Ui"),
        ] {
            let e = entry(id).unwrap_or_else(|| panic!("{id} is listed"));
            assert_eq!(e.group, group, "{id}");
        }
        for id in
            [crate::addons::pathtools::set_corners::COMMAND, "perspective.grid.lock", crate::dialogs::custom_dictionary::COMMAND, "ui.recolorDialog"]
        {
            let mut ov = Overrides::new();
            assign(&mut ov, id, Some("Cmd+Alt+Shift+F11"), false).unwrap();
            let (_, back) = import_json(&export_json("Mine", &ov)).unwrap();
            assert_eq!(effective_in(&back, id).as_deref(), Some("Cmd+Alt+Shift+F11"), "{id}");
        }
    }

    /// No two entries of Keyboard Shortcuts have the same English name. A command that an item of
    /// the menu bar runs takes that item's menu path (Window › Swatch Libraries › Other Library…), a
    /// command whose id extends another command's id takes that command's label (Character Style
    /// Options › Reset Panel), and the other entries that would share a name show their key, except
    /// the one an item of the menu bar runs: File › New… is the New Document dialog's entry, and
    /// `file.new` is File › New… (file.new). An entry whose name no other entry has keeps its group.
    /// The search finds an entry by the key it shows, and a conflict, in `shortcuts.set` and in the
    /// dialog, names the entry that has the shortcut by its whole name.
    #[test]
    fn no_two_entries_have_the_same_name() {
        let all = entries();
        let mut by_name: HashMap<String, Vec<&str>> = HashMap::new();
        for e in all.iter() {
            by_name.entry(english_name(e)).or_default().push(e.key.as_str());
        }
        let shared: Vec<_> = by_name.into_iter().filter(|(_, keys)| keys.len() > 1).collect();
        assert!(shared.is_empty(), "{shared:?}");
        let shown = |id: &str| entry(id).map(|e| (e.group, e.label, e.shows_key));
        for (id, group) in [
            ("window.brushLibrary.other", "Window › Brush Libraries"),
            ("window.graphicStyleLibrary.other", "Window › Graphic Style Libraries"),
            ("window.swatchLibrary.other", "Window › Swatch Libraries"),
            ("window.symbolLibrary.other", "Window › Symbol Libraries"),
        ] {
            assert_eq!(shown(id), Some((group.into(), "Other Library…".into(), false)), "{id}");
        }
        assert_eq!(shown("ui.charStyleOptions.resetPanel"), Some(("Character Style Options".into(), "Reset Panel".into(), false)));
        assert_eq!(shown("ui.paraStyleOptions.resetPanel"), Some(("Paragraph Style Options".into(), "Reset Panel".into(), false)));
        assert_eq!(shown("file.newDialog"), Some(("File".into(), "New…".into(), false)));
        assert_eq!(shown("file.new"), Some(("File".into(), "New…".into(), true)));
        assert_eq!(shown("text.fitHeadline"), Some(("Type".into(), "Fit Headline".into(), false)));
        assert_eq!(shown("type.fitHeadline"), Some(("Type".into(), "Fit Headline".into(), true)));
        assert_eq!(shown("text.convertListToText"), Some(("Type".into(), "Convert to Text".into(), false)));
        assert_eq!(shown("type.convertListToText"), Some(("Type › Bullets and Numbering".into(), "Convert to Text".into(), false)));
        for id in ["file.exportInBackground", "file.openInBackground", "file.saveInBackground"] {
            assert_eq!(shown(id), Some(("File".into(), "Continue in Background".into(), true)), "{id}");
        }
        assert_eq!(shown("file.cancelExport"), Some(("File".into(), "Cancel Export".into(), false)));
        assert_eq!(shown("file.cancelSave"), Some(("File".into(), "Cancel Save".into(), false)));
        let e = entry("type.fitHeadline").unwrap();
        assert!(shown_name(&e).ends_with("Fit Headline (type.fitHeadline)"), "{}", shown_name(&e));
        assert!(row_matches(&e, "type.fitheadline", None));
        let e = entry("text.fitHeadline").unwrap();
        assert!(shown_name(&e).ends_with("Fit Headline"), "{}", shown_name(&e));
        assert!(!row_matches(&e, "text.fitheadline", None));
        let mut ov = Overrides::new();
        assign(&mut ov, "window.swatchLibrary.other", Some("Cmd+Alt+Shift+F11"), false).unwrap();
        let err = assign(&mut ov, "window.brushLibrary.other", Some("Cmd+Alt+Shift+F11"), false).unwrap_err();
        assert!(err.ends_with("is already used by Window › Swatch Libraries › Other Library…"), "{err}");
        assign(&mut ov, "type.fitHeadline", Some("Cmd+Alt+Shift+F10"), false).unwrap();
        let err = assign(&mut ov, "text.fitHeadline", Some("Cmd+Alt+Shift+F10"), false).unwrap_err();
        assert!(err.ends_with("is already used by Type › Fit Headline (type.fitHeadline)"), "{err}");
        let mut d = Dialog::new("shortcuts", json!({}));
        let mut km = keymap_with(&Overrides::new());
        record(&mut d, &mut km, "type.fitHeadline", "Cmd+Alt+Shift+F10");
        record(&mut d, &mut km, "text.fitHeadline", "Cmd+Alt+Shift+F10");
        assert!(d.str("__message").contains("was already used by Type › Fit Headline (type.fitHeadline) "), "{}", d.str("__message"));
    }

    /// No two entries of Keyboard Shortcuts have the same name in any UI language: each catalog
    /// translates two different English names two different ways.
    #[test]
    fn no_two_entries_have_the_same_name_in_any_language() {
        let all = entries();
        let mut shared = vec![];
        let previous = crate::i18n::current();
        for lang in crate::i18n::Lang::all() {
            crate::i18n::set_current(lang);
            let mut by_name: HashMap<String, Vec<&str>> = HashMap::new();
            for e in all.iter() {
                by_name.entry(shown_name(e)).or_default().push(e.key.as_str());
            }
            shared.extend(by_name.into_iter().filter(|(_, keys)| keys.len() > 1).map(|(name, keys)| (lang.code(), name, keys)));
        }
        crate::i18n::set_current(previous);
        shared.sort();
        assert!(shared.is_empty(), "{} names: {shared:#?}", shared.len());
    }

    /// Two filter plug-ins with the same name in the same menu show their ids, and a third whose
    /// name is one of those names with the id stays apart from it. A filter with that name in
    /// another menu, or with a label that differs by its "…", shows no id.
    #[test]
    fn a_plugin_named_like_another_row_with_its_key_stays_apart() {
        let row = |key: &str, label: &str, group: &str| Entry {
            key: key.into(),
            label: label.into(),
            group: group.into(),
            is_tool: false,
            shows_key: false,
        };
        let menu = "Object › Plug-ins";
        let mut rows = vec![
            row("plugin:a", "Foo", menu),
            row("plugin:b", "Foo", menu),
            row("plugin:c", "Foo (plugin:a)", menu),
            row("plugin:d", "Bar", menu),
            row("plugin:e", "Foo", "Object › Plug-ins › Other"),
            row("plugin:f", "Foo…", menu),
        ];
        distinct_names(&mut rows);
        let names: Vec<String> = rows.iter().map(english_name).collect();
        assert_eq!(
            names,
            [
                "Object › Plug-ins › Foo (plugin:a)",
                "Object › Plug-ins › Foo (plugin:b)",
                "Object › Plug-ins › Foo (plugin:a) (plugin:c)",
                "Object › Plug-ins › Bar",
                "Object › Plug-ins › Other › Foo",
                "Object › Plug-ins › Foo…"
            ]
        );
    }

    /// A command with items in more than one menu is shown under the path of its first item with
    /// no params, or else of its first item: Recolor Artwork… under Edit › Edit Colors, not Edit ›
    /// Edit Colors › Recolor with Preset.
    #[test]
    fn a_command_on_several_items_takes_the_path_of_its_item_with_no_params() {
        let item = |path: &str, p: Value| (path.to_string(), p);
        let items = [item("A › B", json!({"x": 1})), item("A › C", Value::Null), item("A › D", json!({}))];
        assert_eq!(chosen_path(&items), Some("A › C"));
        assert_eq!(chosen_path(&[item("A › B", json!({"x": 1})), item("A › C", json!({"x": 2}))]), Some("A › B"));
        assert_eq!(chosen_path(&[]), None);
        assert_eq!(menu_bar_path("ui.recolorDialog"), Some("Edit › Edit Colors"));
    }

    /// `shortcuts.set`, `shortcuts.list` and `shortcuts.preset` reject a supplied param of the wrong
    /// type with an error naming it, and change nothing; an omitted or null param keeps its default.
    #[test]
    fn shortcut_commands_reject_a_param_of_the_wrong_type() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let set = app.ui.shortcut_set.clone();
        for (id, p, err) in [
            ("shortcuts.set", json!({"id": 5, "shortcut": "Cmd+Alt+Shift+F11"}), "`id` must be a string"),
            ("shortcuts.set", json!({"id": "object.isolate", "shortcut": 7}), "`shortcut` must be a string"),
            ("shortcuts.set", json!({"id": "object.isolate", "shortcut": "Cmd+Alt+Shift+F11", "force": "yes"}), "`force` must be true or false"),
            ("shortcuts.list", json!({"query": 5}), "`query` must be a string"),
            ("shortcuts.preset", json!({"name": true}), "`name` must be a string"),
        ] {
            assert_eq!(app.run(id, p.clone()), Err(err.to_string()), "{id} {p}");
            assert!(app.ui.shortcut_overrides.is_empty(), "{id} {p}");
            assert_eq!(app.ui.shortcut_set, set, "{id} {p}");
        }
        let (all, null_query) =
            with_steady_plugins(|| (app.run("shortcuts.list", json!({})).unwrap(), app.run("shortcuts.list", json!({"query": null}))));
        assert_eq!(null_query, Ok(all));
        let r = app.run("shortcuts.set", json!({"id": "object.isolate", "shortcut": "Cmd+Alt+Shift+F11", "force": null})).unwrap();
        assert_eq!(r["shortcut"], "Cmd+Alt+Shift+F11");
        assert!(app.run("shortcuts.preset", json!({"name": null})).unwrap_err().starts_with("unknown preset"));
    }

    /// `shortcuts.set {hold}` gives an entry a hold shortcut and leaves its shortcut, its default
    /// and its overridden state alone; `shortcuts.list` shows it; Clear (`shortcut: null`) keeps it.
    #[test]
    fn a_hold_shortcut_is_kept_apart_from_the_shortcut() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let before = effective_in(&app.ui.shortcut_overrides, "paint.swap");
        let r = app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "x"})).unwrap();
        assert_eq!((r["hold"].as_str(), r["shortcut"].as_str().map(str::to_string)), (Some("X"), before.clone()));
        assert!(app.ui.shortcut_overrides.is_empty(), "no shortcut changed");
        assert_eq!(app.ui.shortcut_set, CUSTOM);
        let rows = with_steady_plugins(|| app.run("shortcuts.list", json!({"query": "paint.swap"})).unwrap());
        let row = rows.as_array().unwrap().iter().find(|r| r["id"] == "paint.swap").unwrap().clone();
        assert_eq!((row["hold"].as_str(), row["overridden"].as_bool()), (Some("X"), Some(false)));
        app.run("shortcuts.set", json!({"id": "paint.swap", "shortcut": null})).unwrap();
        assert_eq!(hold_of(&app.ui.shortcut_holds, "paint.swap").as_deref(), Some("X"), "Use Default keeps the hold");
        app.run("shortcuts.set", json!({"id": "paint.swap", "shortcut": "Cmd+Alt+Shift+F9"})).unwrap();
        assert_eq!(hold_of(&app.ui.shortcut_holds, "paint.swap").as_deref(), Some("X"), "a new shortcut keeps the hold");
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": ""})).unwrap();
        assert!(app.ui.shortcut_holds.is_empty(), "an empty hold removes it");
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+1"})).unwrap();
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": null})).unwrap();
        assert!(app.ui.shortcut_holds.is_empty(), "a null hold removes it");
    }

    /// A hold shortcut is a key that types a character, alone or with Shift; other keys, a clash
    /// without `force` and a hold that is not a string are refused and change nothing.
    #[test]
    fn hold_shortcuts_refuse_keys_that_cannot_wait() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        for bad in ["Space", "Shift+Space", "~", "Shift+~", "Cmd+X", "Alt+X", "Ctrl+X", "Left", "F5", "Enter", "Delete"] {
            let r = app.run("shortcuts.set", json!({"id": "paint.swap", "hold": bad}));
            assert!(r.is_err(), "{bad}: {r:?}");
        }
        assert_eq!(app.run("shortcuts.set", json!({"id": "paint.swap", "hold": 5})), Err("`hold` must be a string".to_string()));
        assert!(app.ui.shortcut_holds.is_empty());
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+X"})).unwrap();
        let clash = app.run("shortcuts.set", json!({"id": "select.none", "hold": "Shift+X"})).unwrap_err();
        assert!(clash.starts_with("Hold "), "{clash}");
        // A refused hold leaves the shortcut given with it unchanged too.
        let r = app.run("shortcuts.set", json!({"id": "select.none", "shortcut": "Cmd+Alt+Shift+F9", "hold": "Shift+X"}));
        assert!(r.is_err());
        assert!(app.ui.shortcut_overrides.is_empty());
        let r = app.run("shortcuts.set", json!({"id": "select.none", "hold": "Shift+X", "force": true})).unwrap();
        assert_eq!(r["removedFrom"], json!(["paint.swap"]));
        assert_eq!(app.ui.shortcut_holds.keys().collect::<Vec<_>>(), ["select.none"]);
    }

    /// A command the editor lists only with a shortcut (Transparency needs a value) takes a hold
    /// shortcut only with one, and loses its hold, as it stops being listed, when the shortcut is
    /// taken away: by clearing it or by giving it to another command.
    #[test]
    fn a_hold_shortcut_needs_a_listed_entry() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert!(needs_a_value("transparency.set"));
        let r = app.run("shortcuts.set", json!({"id": "transparency.set", "hold": "Q"}));
        assert!(r.is_err(), "{r:?}");
        assert!(app.ui.shortcut_holds.is_empty() && app.ui.shortcut_overrides.is_empty());
        let q = vc_chord("Q").unwrap();
        app.run("shortcuts.set", json!({"id": "transparency.set", "shortcut": "Cmd+Alt+Shift+F9", "hold": "Q"})).unwrap();
        assert_eq!(hold_for(&app.ui.shortcut_holds, &q), Some("transparency.set"));
        let rows = with_steady_plugins(|| app.run("shortcuts.list", json!({"query": "transparency.set"})).unwrap());
        assert!(rows.as_array().unwrap().iter().any(|r| r["id"] == "transparency.set" && r["hold"] == "Q"), "{rows}");
        app.run("shortcuts.set", json!({"id": "transparency.set", "shortcut": ""})).unwrap();
        assert_eq!(hold_for(&app.ui.shortcut_holds, &q), None);
        assert!(app.ui.shortcut_holds.is_empty(), "{:?}", app.ui.shortcut_holds);
        // The key is free for another command, with no conflict left behind.
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Q"})).unwrap();
        assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]));
        app.run("shortcuts.set", json!({"id": "transparency.set", "shortcut": "Cmd+Alt+Shift+F9", "hold": "Shift+Q"})).unwrap();
        app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": "Cmd+Alt+Shift+F9", "force": true})).unwrap();
        assert_eq!(app.ui.shortcut_holds.keys().collect::<Vec<_>>(), ["paint.swap"]);
    }

    /// Reset All and a preset clear the hold shortcuts; two entries holding one key are a conflict.
    #[test]
    fn hold_shortcut_conflicts_reset_and_presets() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.ui.shortcut_holds.insert("paint.swap".into(), "X".into());
        app.ui.shortcut_holds.insert("select.none".into(), "X".into());
        let c = app.run("shortcuts.conflicts", json!({})).unwrap();
        assert!(c.as_array().unwrap().iter().any(|c| c["shortcut"] == "Hold X"), "{c}");
        app.run("shortcuts.reset", json!({})).unwrap();
        assert!(app.ui.shortcut_holds.is_empty());
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
        app.run("shortcuts.preset", json!({"name": PRESETS[0]})).unwrap();
        assert!(app.ui.shortcut_holds.is_empty());
    }

    /// Export… writes the hold shortcuts as `holds` (version 1) only when there are any; Import…
    /// reads them, a file without them has none, and a bad one fails and applies nothing.
    #[test]
    fn hold_shortcuts_export_and_import() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let none = export_json("Studio", &Overrides::new());
        assert!(none.get("holds").is_none());
        let holds: Holds = [("paint.swap".to_string(), "X".to_string())].into();
        let v = export_json_with("Studio", &Overrides::new(), &holds);
        assert_eq!((v["version"].as_u64(), v["holds"].clone()), (Some(1), json!({"paint.swap": "X"})));
        app.run("shortcuts.import", json!({"data": v})).unwrap();
        assert_eq!(app.ui.shortcut_holds, holds);
        for bad in [json!("X"), json!({"paint.swap": 5}), json!({"paint.swap": "Space"})] {
            let mut v = export_json("Other", &Overrides::new());
            v["holds"] = bad.clone();
            assert!(app.run("shortcuts.import", json!({"data": v})).is_err(), "{bad}");
            assert_eq!((app.ui.shortcut_holds.clone(), app.ui.shortcut_set.as_str()), (holds.clone(), "Studio"), "{bad}");
        }
        app.run("shortcuts.import", json!({"data": none})).unwrap();
        assert!(app.ui.shortcut_holds.is_empty(), "an import replaces the whole set");
    }

    /// Export Text… gives a row with a hold shortcut a third column while Hold Shortcuts is on.
    #[test]
    fn export_text_lists_hold_shortcuts_while_they_act() {
        type Files = std::rc::Rc<std::cell::RefCell<Vec<String>>>;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let written: Files = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().push(String::from_utf8(b.to_vec()).map_err(|e| e.to_string())?);
            Ok(())
        }));
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
        let row = |text: &str| text.lines().find(|l| l.starts_with(&english_name(&entry("paint.swap").unwrap()))).map(str::to_string);
        with_steady_plugins(|| {
            written.borrow_mut().clear();
            app.run("shortcuts.exportText", json!({"path": "/a.txt"})).unwrap();
            app.run("prefs.set", json!({"key": "holdShortcuts", "value": true})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/b.txt"})).unwrap();
            app.run("prefs.set", json!({"key": "holdShortcuts", "value": false})).unwrap();
        });
        let files = written.borrow();
        let (off, on) = (row(&files[0]).unwrap(), row(&files[1]).unwrap());
        assert_eq!(off.split('\t').count(), 2, "{off}");
        assert_eq!(on.split('\t').nth(2), Some(format!("Hold {}", menus::pretty_shortcut("X")).as_str()), "{on}");
    }

    /// Export Text…'s shortcut sheet and CSV file also list a hold shortcut while Hold Shortcuts is
    /// on, after the row's shortcuts.
    #[test]
    fn the_sheet_and_csv_list_hold_shortcuts_while_they_act() {
        type Files = std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let written: Files = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().push(b.to_vec());
            Ok(())
        }));
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
        let hold = format!("Hold {}", menus::pretty_shortcut("X"));
        let sections = with_steady_plugins(|| {
            written.borrow_mut().clear();
            for on in [false, false, true] {
                app.run("prefs.set", json!({"key": "holdShortcuts", "value": on})).unwrap();
                app.run("shortcuts.exportText", json!({"path": "/a.csv", "format": "csv"})).unwrap();
                app.run("shortcuts.exportText", json!({"path": "/a.pdf", "format": "sheet"})).unwrap();
            }
            app.run("prefs.set", json!({"key": "holdShortcuts", "value": false})).unwrap();
            export_sections(&app.ui.shortcut_overrides, &app.ui.shortcut_holds)
        });
        let files = written.borrow();
        let [csv_off, sheet_off, _, sheet_off_again, csv_on, sheet_on] = files.as_slice() else { panic!("{} files", files.len()) };
        let swap = sections.iter().flat_map(|s| &s.rows).filter(|r| r.shortcut.ends_with(&hold)).collect::<Vec<_>>();
        assert_eq!(swap.len(), 1, "one row has the hold: {swap:?}");
        let (csv_off, csv_on) = (String::from_utf8_lossy(csv_off), String::from_utf8_lossy(csv_on));
        assert!(!csv_off.contains(&hold), "off, the CSV file lists no hold");
        assert!(csv_on.contains(&swap[0].shortcut), "on, the CSV file lists it: {}", swap[0].shortcut);
        // The writer stamps each file with the clock: its dates and its ID (fixed width), which differ
        // between two exports a second apart. The rest of the file is the same each time.
        let unstamped = |pdf: &[u8]| {
            let mut b: Vec<u8> = pdf.iter().map(|&c| if c.is_ascii_digit() { b'0' } else { c }).collect();
            for key in [&b"InstanceID>"[..], b"DocumentID>", b"/ID["] {
                let mut at = 0;
                while let Some(i) = b[at..].windows(key.len()).position(|w| w == key) {
                    let start = at + i + key.len();
                    let end = b[start..].iter().position(|&c| c == b'<' || c == b']').map_or(b.len(), |e| start + e);
                    b[start..end].fill(b'?');
                    at = end;
                }
            }
            b
        };
        assert_eq!(unstamped(sheet_off), unstamped(sheet_off_again), "the sheet is the same each time");
        assert_ne!(sheet_off, sheet_on, "on, the sheet lists the hold");
    }

    /// The text the Keyboard Shortcuts dialog draws in its second frame.
    fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        // The modal draws nothing in its first frame, which only sizes it.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }

    /// Hold Shortcuts is off by default: the dialog shows its checkbox and no Hold column. Turned
    /// on, the dialog shows the column and the delay, and OK turns hold shortcuts on and asks the
    /// host to write the preferences file; Cancel keeps them off.
    #[test]
    fn the_hold_shortcuts_checkbox_shows_the_hold_column() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert!(!app.session.prefs.hold_shortcuts);
        open(&mut app);
        let off = dialog_texts(&mut app);
        assert!(off.iter().any(|t| t == "Hold Shortcuts"), "{off:?}");
        assert!(!off.iter().any(|t| t == "Hold" || t == "Hold Shortcut Delay"), "{off:?}");
        app.ui.dialog.as_mut().unwrap().fields.insert("holdShortcuts".into(), json!(true));
        let on = dialog_texts(&mut app);
        assert!(on.iter().any(|t| t == "Hold") && on.iter().any(|t| t == "Hold Shortcut Delay"), "{on:?}");
        app.ui.dialog = None;
        assert!(!app.session.prefs.hold_shortcuts, "Cancel keeps them off");
        assert!(!app.take_prefs_changed(), "nothing to write after Cancel");
        open(&mut app);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("holdShortcuts".into(), json!(true));
        d.fields.insert("keyHoldDelay".into(), json!(0.5));
        confirm(&mut app).unwrap();
        assert!(app.session.prefs.hold_shortcuts);
        assert_eq!(app.session.prefs.key_hold_delay, 0.5);
        assert!(app.take_prefs_changed(), "OK asks the host to write the preferences file");
        assert_eq!(app.ui.engine_prefs["holdShortcuts"], json!(true));
        assert_eq!(app.ui.engine_prefs["keyHoldDelay"], json!(0.5));
    }

    /// The info button shows how hold shortcuts work.
    #[test]
    fn the_info_button_explains_hold_shortcuts() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        open(&mut app);
        assert!(!dialog_texts(&mut app).iter().any(|t| t.starts_with("A quick press")));
        app.ui.dialog.as_mut().unwrap().fields.insert("__holdInfo".into(), json!(true));
        let texts = dialog_texts(&mut app);
        assert!(texts.iter().any(|t| t.starts_with("A quick press")) && texts.iter().any(|t| t == "Tap"), "{texts:?}");
    }

    /// While Hold Shortcuts is on, the search finds a row by its hold shortcut as the Hold column
    /// shows it; while it is off, the hold shortcut finds nothing.
    #[test]
    fn the_search_finds_a_row_by_its_hold_shortcut() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+;"})).unwrap();
        let hold = menus::pretty_shortcut("Shift+;").to_lowercase();
        let name = shown_name(&entry("paint.swap").unwrap());
        assert!(!name.to_lowercase().contains(&hold), "{name}");
        open(&mut app);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("tab".into(), json!("menu"));
        d.fields.insert("query".into(), json!(hold));
        assert!(!dialog_texts(&mut app).contains(&name), "off, the hold shortcut finds nothing");
        app.ui.dialog.as_mut().unwrap().fields.insert("holdShortcuts".into(), json!(true));
        let texts = dialog_texts(&mut app);
        assert!(texts.contains(&name), "{name} in {texts:?}");
    }

    /// A command that needs a value is listed in the editor once the user gives it a key, but the
    /// command search still leaves it out: a pick gives it no value.
    #[test]
    fn a_command_given_a_key_that_needs_a_value_is_no_pick() {
        let mut overrides = Overrides::default();
        crate::keymap::set_override(&mut overrides, "document.setUnits", "Cmd+Alt+Shift+F12");
        let entry = entries().iter().find(|e| e.key == "document.setUnits").cloned().expect("the editor has a row for it");
        assert!(!listed(&Overrides::default(), &entry) && listed(&overrides, &entry));
        assert!(needs_a_value("document.setUnits"));
        assert!(crate::palette::items().iter().all(|(_, id, _)| id != "document.setUnits"));
    }
}
