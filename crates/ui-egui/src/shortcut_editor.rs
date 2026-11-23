//! Edit → Keyboard Shortcuts: what a shortcut can be given ([`entries`]), the framework's
//! Keyboard Shortcuts dialog over the window's keymap ([`show`]), and the `shortcuts.*` commands.
//!
//! The user's sets live in [`crate::state::UiState::shortcut_sets`] (persisted through
//! [`crate::keymap::stored`]), over the built-in sets [`BUILTINS`]. The set in use reaches the key
//! stage as `VectorcraftApp::shortcut_overrides`, from which the runtime builds the window's keymap.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use craft::host::{Host, PickRequest, Picked};
use craft::keymap::{
    AssignError, Binding, CommandDef, CommandKind, ConflictKind, ForeignSet, ImportOptions, Keymap, Overrides, Policy, PressKind, Profiles, Trigger,
};
use craft::menu::shortcuts::{Area, DialogState, Format, HoldSetting, JSON_FORMAT, Options, Outcome, ShortcutsDialog, summary};
use craft::ui::runtime::commands;
use serde_json::{Value, json};

use crate::file_asks::{self, Owner};
use crate::keymap::{command_of, entry_key, stored, stored_key};
use crate::menus::Node;
use crate::shortcut_sheet::PageSize;
use crate::{VectorcraftApp, menus};

pub const PRESETS: &[&str] = &["VectorCraft Defaults", "Classic Defaults"];
pub const CUSTOM: &str = "Custom";
/// Set names earlier versions saved: (old name, current name).
pub const LEGACY_PRESETS: &[(&str, &str)] = &[("Illustrator Defaults", "Classic Defaults")]; // brand-ok: legacy preference value

/// The built-in sets, [`PRESETS`] by name. Both are the registry defaults.
pub static BUILTINS: [(&str, Overrides); 2] = [(PRESETS[0], Overrides::new()), (PRESETS[1], Overrides::new())];

/// The current name of a shortcut set: names earlier versions saved map to today's.
pub fn set_name(name: &str) -> &str {
    LEGACY_PRESETS.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new)
}

/// Registry default shortcut of a command (engine or UI command) on [`crate::keymap::platform`].
pub fn default_command_shortcut(id: &str) -> Option<&'static str> {
    default_command_shortcut_on(id, crate::keymap::platform())
}

/// Registry default shortcut of a command on platform `p`.
fn default_command_shortcut_on(id: &str, p: craft::keymap::Platform) -> Option<&'static str> {
    if let Some(c) = vectorcraft_engine::find_command(id) {
        return c.shortcut;
    }
    menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| ui_command_default(c.0, c.2, p)).filter(|s| !s.is_empty())
}

/// The default shortcut `sc` of UI command `id` on platform `p` (empty for none). Select Last
/// Selection Tool has none on macOS, which takes Cmd+` to switch between an app's windows.
pub(crate) fn ui_command_default(id: &str, sc: &'static str, p: craft::keymap::Platform) -> &'static str {
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
pub(crate) fn default_on(key: &str, p: craft::keymap::Platform) -> Option<&'static str> {
    if let Some(t) = key.strip_prefix("tool:") {
        return vectorcraft_tools::tool_info(t).and_then(|t| t.shortcut);
    }
    if let Some(panel) = key.strip_prefix("panel:") {
        return PANEL_SHORTCUTS.iter().find(|(id, _)| *id == panel).map(|(_, sc)| *sc);
    }
    default_command_shortcut_on(key, p)
}

// ---------- entries ----------

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
    fn walk(items: &[Node], path: &mut Vec<String>, seen: &mut HashSet<String>, out: &mut Vec<EffectItem>) {
        for node in items {
            match node {
                Node::Item(it) => {
                    let command = match it.command.id.as_str() {
                        "effect.dialog" => "effect.dialog",
                        "effect.apply" => "effect.apply",
                        _ => continue,
                    };
                    if let Some(effect) = it.command.params.get("effect").and_then(serde_json::Value::as_str)
                        && seen.insert(effect.to_string())
                    {
                        out.push(EffectItem { effect: effect.to_string(), command, label: it.label.clone(), group: path.join(" › ") });
                    }
                }
                Node::Submenu { label, children, .. } => {
                    path.push(label.clone());
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
    fn walk(items: &[Node], path: &mut Vec<String>, seen: &mut HashSet<String>, out: &mut Vec<FilterItem>) {
        for node in items {
            match node {
                Node::Item(it) if it.command.id == "plugin.dialog" => {
                    if let Some(id) = it.command.params.get("id").and_then(serde_json::Value::as_str)
                        && seen.insert(id.to_string())
                    {
                        out.push(FilterItem { id: id.to_string(), label: it.label.clone(), group: path.join(" › ") });
                    }
                }
                Node::Submenu { label, children, .. } => {
                    path.push(label.clone());
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
fn menu_bar_paths() -> &'static HashMap<String, Vec<(String, Value)>> {
    fn walk(items: &[Node], path: &mut Vec<String>, out: &mut HashMap<String, Vec<(String, Value)>>) {
        for node in items {
            match node {
                Node::Item(it) if on_the_bar(&it.command.id, &it.label, &it.command.params) => {
                    out.entry(it.command.id.clone()).or_default().push((path.join(" › "), it.command.params.clone()));
                }
                Node::Submenu { label, children, .. } => {
                    path.push(label.clone());
                    walk(children, path, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    static PATHS: OnceLock<HashMap<String, Vec<(String, Value)>>> = OnceLock::new();
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
/// the user has given it a shortcut (a hold alone is none).
fn listed(overrides: &Overrides, e: &Entry) -> bool {
    !needs_a_value(&e.key)
        || overrides.get(&stored_key(&e.key)).is_some_and(|texts| texts.iter().any(|t| Trigger::parse(t).is_ok_and(|t| !t.is_hold())))
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
fn menu_bar_items() -> &'static HashMap<String, Vec<(String, Value)>> {
    static ITEMS: OnceLock<HashMap<String, Vec<(String, Value)>>> = OnceLock::new();
    ITEMS.get_or_init(|| bar_items(&menus::menu_tree()))
}

/// The (label, params) of each item of the menus `tree`, by command id. Items that list an
/// installed plug-in ([`menus::lists_plugin`]) or show a name that is user data
/// ([`menus::shows_a_name`]: a custom toolbar, a script, a recent file, a saved selection) are left
/// out: [`menu_bar_items`] is built once, and such items come and go after that.
fn bar_items(tree: &[(&'static str, Vec<Node>)]) -> HashMap<String, Vec<(String, Value)>> {
    fn walk(items: &[Node], out: &mut HashMap<String, Vec<(String, Value)>>) {
        for node in items {
            match node {
                Node::Item(it) if on_the_bar(&it.command.id, &it.label, &it.command.params) => {
                    out.entry(it.command.id.clone()).or_default().push((it.label.clone(), it.command.params.clone()));
                }
                Node::Submenu { children, .. } => walk(children, out),
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

/// Reads a set in the `vectorcraft-shortcuts` form earlier versions wrote: its name (an earlier
/// name maps to today's; none is [`CUSTOM`]) and its overrides, with its hold shortcuts.
pub fn import_json(v: &Value) -> Result<(String, Overrides), String> {
    let o = v.get("overrides").and_then(Value::as_object).ok_or("not a VectorCraft shortcut set (missing `overrides`)")?;
    for (k, v) in o {
        let s = v.as_str().ok_or_else(|| format!("shortcut for `{k}` must be a string"))?;
        if !s.is_empty() && stored::parse_vc(s).is_none() {
            return Err(format!("can't parse shortcut `{s}` for `{k}`"));
        }
    }
    let mut out = stored::from_value(Value::Object(o.clone()));
    stored::add_holds(&mut out, &v["holds"]);
    Ok((v.get("set").and_then(Value::as_str).map_or(CUSTOM, set_name).to_string(), out))
}

/// `v` as craft-keymap JSON, which [`Profiles::import`] reads: a set in the form earlier versions
/// wrote ([`import_json`]) is written in it, under its name, or [`CUSTOM`] for a built-in set's.
fn keymap_json(v: &Value) -> Result<String, String> {
    if v.get("overrides").is_none() || v.get("format").and_then(Value::as_str) == Some(JSON_FORMAT) {
        return Ok(v.to_string());
    }
    let (name, overrides) = import_json(v)?;
    let mut sets = Profiles::default();
    let name =
        sets.save_as(&name, overrides.clone(), &BUILTINS).or_else(|_| sets.save_as(CUSTOM, overrides, &BUILTINS)).map_err(|e| e.to_string())?;
    sets.export(&name, &BUILTINS).map_err(|e| e.to_string())
}

/// Export Text's file formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFormat {
    /// The dialog's summary as a text file ([`summary::text`]).
    Text,
    /// The printable shortcut sheet as a PDF ([`crate::shortcut_sheet::sheet_pdf`]).
    Sheet,
    /// A CSV file of [`export_sections`] ([`crate::shortcut_sheet::csv`]).
    Csv,
}

impl TextFormat {
    pub const ALL: [Self; 3] = [Self::Text, Self::Sheet, Self::Csv];

    /// The format in the dialog's Export and in a save panel's file types.
    const fn format(self) -> Format {
        match self {
            Self::Text => Format::new("text", "Text", &["txt"]),
            Self::Sheet => Format::new("sheet", "Shortcut sheet", &["pdf"]),
            Self::Csv => Format::new("csv", "CSV", &["csv"]),
        }
    }

    /// The `format` param's value.
    pub fn name(self) -> &'static str {
        self.format().id
    }

    fn extension(self) -> &'static str {
        self.format().extensions.first().copied().unwrap_or_default()
    }

    /// `request` with this format among its file types.
    fn filter(self, request: PickRequest) -> PickRequest {
        let f = self.format();
        request.filter(f.label, f.extensions)
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

/// What a row shows for entry `key` in `keymap`: its shortcuts, joined as the dialog joins them,
/// then its hold shortcut while holds act.
fn shown_bindings(keymap: &Keymap, key: &str) -> String {
    let p = keymap.platform();
    let bindings = keymap.bindings(&command_of(key));
    let singles = bindings.iter().filter(|b| !b.trigger.is_hold()).map(|b| b.trigger.display(p));
    let holds = bindings.iter().filter(|b| b.trigger.is_hold() && keymap.holds()).map(|b| format!("{} {}", tl!("Hold"), b.trigger.chord.display(p)));
    singles.chain(holds).collect::<Vec<_>>().join(" · ")
}

/// Export Text's rows in sections, for the shortcut sheet and the CSV file: Tools, then one section
/// per top-level menu in the order the dialog first lists a command of it, with the names and
/// shortcuts of `keymap` as the dialog shows them, each name without its top-level menu. Every row
/// the dialog lists is there, with an empty shortcut when it has none.
pub fn export_sections(keymap: &Keymap) -> Vec<crate::shortcut_sheet::Section> {
    use crate::shortcut_sheet::{Row, Section};
    let snapshot = snapshot();
    let mut tools = Section { title: tl!("Tools").to_string(), rows: vec![] };
    let mut menus: Vec<Section> = vec![];
    for e in snapshot.entries.iter().filter(|e| listed(keymap.overrides(), e)) {
        let shortcut = shown_bindings(keymap, &e.key);
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

/// The file `format` writes for the set `name` of `keymap`: the sheet on `page`, the text in the
/// words of `ui`.
fn export_text_file(format: TextFormat, keymap: &Keymap, name: &str, page: PageSize, host: &Arc<Host>) -> Result<Vec<u8>, String> {
    let title = format!("{} · {name}", tl!("Keyboard Shortcuts"));
    match format {
        TextFormat::Text => Ok(summary::text_for(keymap, host, &title).into_bytes()),
        TextFormat::Sheet => crate::shortcut_sheet::sheet_pdf(&title, &export_sections(keymap), page),
        TextFormat::Csv => Ok(crate::shortcut_sheet::csv(&export_sections(keymap), ["Group", "Command", "Shortcut"]).into_bytes()),
    }
}

/// The sheet's paper when `pageSize` is left out: A4 when the current document prints on A4 (its
/// print settings, or, with none saved, a first artboard of A4 size either way up), else Letter.
fn default_sheet_page(app: &VectorcraftApp) -> PageSize {
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

// ---------- the dialog ----------

/// The purpose of the file dialogs of shortcut sets: they share a remembered folder.
pub(crate) const SHORTCUT_SETS: &str = "shortcut-sets";

/// Keyboard Shortcuts while it is open (`VectorcraftApp::shortcuts_dialog`).
pub(crate) struct ShortcutsDialogState {
    pub state: DialogState,
    /// The keymap the dialog edits: a copy of the window's, taken by the first [`show`].
    pub keymap: Option<Keymap>,
    pub hold: HoldSetting,
    /// A set another app saved as text, for the review the next [`show`] opens.
    pub pending_review: Option<(ForeignSet, ImportOptions)>,
    /// A set as craft-keymap JSON that Import… read, which the next [`show`] loads.
    pub pending_import: Option<String>,
    /// The set under review, for the answers the review reports.
    pub reviewed: Option<ForeignSet>,
}

/// Opens Keyboard Shortcuts on the Tools tab, with the Hold Shortcuts preferences.
pub(crate) fn open(app: &mut VectorcraftApp) {
    let (min, max) = vectorcraft_engine::KEY_HOLD_DELAY_RANGE;
    let prefs = &app.session.prefs;
    let hold = HoldSetting::new(prefs.hold_shortcuts, prefs.key_hold_delay(), min..=max);
    app.shortcuts_dialog = Some(ShortcutsDialogState {
        state: DialogState::new().with_tab("tools"),
        keymap: None,
        hold,
        pending_review: None,
        pending_import: None,
        reviewed: None,
    });
}

fn is_tool(def: &CommandDef) -> bool {
    matches!(def.kind, CommandKind::Tool { .. })
}

/// The app's rule for the keys a command takes: a tool's shortcut is one key, alone or with Shift.
fn accept(def: &CommandDef, binding: &Binding) -> Result<(), String> {
    let t = binding.trigger;
    let one_key = matches!(t.press, PressKind::Single) && !(t.chord.mods.cmd || t.chord.mods.ctrl || t.chord.mods.alt);
    if is_tool(def) && !t.is_hold() && !one_key {
        Err(tl!("tool shortcuts must be a single key (optionally with Shift)").to_string())
    } else {
        Ok(())
    }
}

/// How the dialog behaves: Tools and Menu Commands as tabs, the tools' rule ([`accept`]), the sets
/// another app saves as text, and Export Text's formats.
fn options() -> Options {
    Options::default().with(|o| {
        o.area_tabs = true;
        o.areas = vec![Area::new("tools", "Tools", is_tool), Area::new("menu", MENU_COMMANDS, |def| !is_tool(def))];
        o.summary = false;
        o.accept = Some(Box::new(accept));
        o.import_formats = vec![Format::new("kys", "Shortcut set", &["kys"]), Format::new("json", "VectorCraft shortcut set", &["json"])];
        o.export_formats = TextFormat::ALL.map(TextFormat::format).to_vec();
    })
}

/// The dialog for one call, over the user's sets.
fn dialog<'a>(keymap: &'a mut Keymap, sets: &'a mut Profiles, hold: &'a mut HoldSetting, options: &'a Options) -> ShortcutsDialog<'a> {
    ShortcutsDialog::new(keymap).sets(sets, &BUILTINS).holds(hold).options(options)
}

/// Keyboard Shortcuts for this pass, while it is open: the review or the set waiting for it, the
/// dialog, and what it reports.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let Some(mut current) = app.shortcuts_dialog.take() else { return };
    if current.keymap.is_none() {
        current.keymap = commands::keymap(ui.ctx()).map(|window| (*window).clone());
    }
    let ShortcutsDialogState { state, keymap: Some(keymap), hold, pending_review, pending_import, reviewed } = &mut current else {
        // The window's keymap comes with its first frame: the dialog shows from the next.
        app.shortcuts_dialog = Some(current);
        return;
    };
    let options = options();
    let sets = &mut app.ui.shortcut_sets;
    if let Some((set, import)) = pending_review.take() {
        *reviewed = Some(set.clone());
        dialog(keymap, sets, hold, &options).review_import(ui, state, set, import);
    }
    if let Some(text) = pending_import.take()
        && let Err(e) = dialog(keymap, sets, hold, &options).import_json(ui, state, &text)
    {
        state.show_error(e.to_string());
    }
    let outcome = dialog(keymap, sets, hold, &options).show(ui, state).outcome;
    // An edit, and the OK after it, hand the dialog's keymap to the key stage.
    let edited = matches!(outcome, Some(Outcome::Ok | Outcome::Changed)).then(|| Arc::new(keymap.overrides().clone()));
    let setting = hold.clone();
    let reviewed = if matches!(outcome, Some(Outcome::Remembered { .. })) { reviewed.take() } else { None };
    if state.is_open() {
        app.shortcuts_dialog = Some(current);
    }
    match outcome {
        Some(Outcome::Ok) => {
            if let Err(e) = save_hold_setting(app, &setting) {
                app.status(e);
            }
        }
        Some(Outcome::Import { .. }) => {
            file_asks::ask(app, crate::dialogs::shortcut_import::open_request(), Owner::Free, |app, picked| {
                with_picked_set(app, picked, import_into_dialog)
            });
        }
        Some(Outcome::Export { name, json, .. }) => {
            let written = crate::io::save::write_named(app, None, &format!("{name}.json"), json.into_bytes(), |_, path| Ok(json!({ "path": path })));
            report(app, written);
        }
        Some(Outcome::ExportAs { format, name, keymap, .. }) => {
            let format = TextFormat::ALL.into_iter().find(|f| f.name() == format).unwrap_or(TextFormat::Text);
            let page = default_sheet_page(app);
            let written = export_text_file(format, &keymap, &name, page, &app.host)
                .and_then(|bytes| crate::io::save::write_named(app, None, &format.file_name(), bytes, |_, path| Ok(json!({ "path": path }))));
            report(app, written);
        }
        Some(Outcome::Remembered { targets, codes, .. }) => {
            if let Some(set) = reviewed {
                crate::dialogs::shortcut_import::remember(app, &set, &targets, &codes);
            }
        }
        _ => {}
    }
    if let Some(overrides) = edited {
        app.shortcut_overrides = overrides;
    }
}

/// Saves the dialog's Hold Shortcuts setting as preferences, as OK in Preferences saves them.
fn save_hold_setting(app: &mut VectorcraftApp, setting: &HoldSetting) -> Result<(), String> {
    let prefs = &app.session.prefs;
    if setting.on != prefs.hold_shortcuts || setting.delay != prefs.key_hold_delay() {
        app.run("prefs.set", json!({"values": {"holdShortcuts": setting.on, "keyHoldDelay": setting.delay}}))?;
        crate::prefs_dialog::snapshot(app);
        app.prefs_changed = true;
    }
    Ok(())
}

/// An error of the dialog's own file step, under its table, or on the status line once it has
/// closed. A cancelled file dialog is none.
fn report(app: &mut VectorcraftApp, result: Result<Value, String>) {
    let Err(e) = result else { return };
    if e == "cancelled" {
        return;
    }
    match app.shortcuts_dialog.as_mut() {
        Some(open) => open.state.show_error(e),
        None => app.status(e),
    }
}

type SetReader = fn(&mut VectorcraftApp, &str, &[u8]) -> Result<Value, String>;

/// Runs `then` on the file an Open answer chose: as the browser read it, or read from its path. A
/// `.kys` file is read by Import Shortcut Set, up to its reader's cap.
fn with_picked_set(app: &mut VectorcraftApp, picked: Picked, then: SetReader) -> Result<Value, String> {
    match picked {
        Picked::Contents(files) => {
            let (name, bytes) = files.into_iter().next().ok_or("cancelled")?;
            then(app, &name, &bytes)
        }
        picked => with_set_at(app, &picked.first_path().ok_or("cancelled")?.to_string_lossy(), then),
    }
}

/// Runs `then` on the file at `path`, as [`with_picked_set`] does.
fn with_set_at(app: &mut VectorcraftApp, path: &str, then: SetReader) -> Result<Value, String> {
    if crate::dialogs::shortcut_import::is_kys(path) {
        return crate::dialogs::shortcut_import::command(app, &json!({ "path": path }));
    }
    let read = app.services.read.as_ref().ok_or("no file reader")?;
    let bytes = read(path)?;
    then(app, path, &bytes)
}

/// The dialog's Import…: a set another app saved as text opens its review; a JSON set waits for
/// the dialog's next pass, which loads it.
fn import_into_dialog(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    if app.shortcuts_dialog.is_none() {
        return Err("Keyboard Shortcuts closed before the set was read".into());
    }
    if crate::dialogs::shortcut_import::is_text_set(name, bytes) {
        return crate::dialogs::shortcut_import::open(app, name, bytes);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let text = keymap_json(&v)?;
    if let Some(open) = app.shortcuts_dialog.as_mut() {
        open.pending_import = Some(text);
    }
    Ok(Value::Null)
}

// ---------- commands ----------

/// The string `p` gives for `key`, or `None` when `p` leaves it out or gives null. Any other
/// value, such as `5`, is an error: "`{key}` must be a string".
fn text_param(p: &Value, key: &str) -> Result<Option<String>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// The commands that change the sets wait while Keyboard Shortcuts is open: it edits them.
fn dialog_closed(app: &VectorcraftApp) -> Result<(), String> {
    if app.shortcuts_dialog.is_some() { Err("Keyboard Shortcuts is open: change the shortcuts there".into()) } else { Ok(()) }
}

/// The window's keymap with the set in use, for a command to read or change. The window builds it
/// in its first frame.
fn current_keymap(app: &VectorcraftApp) -> Result<Keymap, String> {
    let window = app.ui_ctx.as_ref().and_then(commands::keymap).ok_or("the window has not drawn a frame")?;
    let mut keymap = (*window).clone();
    keymap.set_overrides((*app.shortcut_overrides).clone());
    keymap.set_holds(app.session.prefs.hold_shortcuts);
    Ok(keymap)
}

/// [`current_keymap`] with its hold shortcuts counted whether Hold Shortcuts is on or not: the
/// editor lists and sets them either way, and the preference decides only whether they act.
fn editing_keymap(app: &VectorcraftApp) -> Result<Keymap, String> {
    let mut keymap = current_keymap(app)?;
    keymap.set_holds(true);
    Ok(keymap)
}

/// Saves `keymap`'s overrides to the set in use (an edit of a built-in set becomes a set of the
/// user's, named after [`CUSTOM`]) and hands them to the key stage.
fn keep(app: &mut VectorcraftApp, keymap: &Keymap) -> Result<(), String> {
    app.ui.shortcut_sets.store(keymap.overrides().clone(), &BUILTINS, CUSTOM).map_err(|e| e.to_string())?;
    app.shortcut_overrides = Arc::new(keymap.overrides().clone());
    Ok(())
}

/// Hands the set in use to the key stage, each Effect menu item's shortcut under the command its
/// item runs now.
pub(crate) fn use_active_set(app: &mut VectorcraftApp) {
    let active = app.ui.shortcut_sets.active_overrides(&BUILTINS);
    app.shortcut_overrides = Arc::new(crate::keymap::effect_keys_in(active, &snapshot().effects));
}

/// A chord in VectorCraft's spelling.
fn parse_chord(text: &str) -> Result<craft::keymap::Chord, String> {
    stored::parse_vc(text).ok_or_else(|| format!("can't parse shortcut `{text}`"))
}

/// A shortcut in VectorCraft's spelling: a chord `X`, a double press `X X` or a hold `Hold X` (kb.C13b).
fn parse_trigger(text: &str) -> Result<Trigger, String> {
    let text = text.trim();
    if let Some(chord) = text.strip_prefix("Hold ") {
        return Ok(Trigger::hold(parse_chord(chord)?));
    }
    match text.split_once(' ') {
        Some((first, second)) if first == second.trim() => Ok(Trigger::double(parse_chord(first)?)),
        _ => Ok(Trigger::single(parse_chord(text)?)),
    }
}

/// Entry `key`'s shortcut and hold shortcut in `keymap`, in the crate's spelling.
fn row_bindings(keymap: &Keymap, key: &str) -> (Option<String>, Option<String>) {
    let bindings = keymap.bindings(&command_of(key));
    let shortcut = bindings.iter().find(|b| !b.trigger.is_hold()).map(|b| b.trigger.to_string());
    let hold = bindings.iter().find(|b| b.trigger.is_hold()).map(|b| b.trigger.chord.to_string());
    (shortcut, hold)
}

/// Why the keymap refused a binding, naming the entries that hold it as the dialog names them.
fn refusal(e: AssignError, keymap: &Keymap) -> String {
    let p = keymap.platform();
    match e {
        AssignError::Taken(holders) => {
            let names: Vec<String> = holders
                .iter()
                .map(|h| {
                    let k = entry_key(&h.command);
                    entry(&k).map_or(k, |e| english_name(&e))
                })
                .collect();
            let trigger = holders.first().map(|h| h.binding.trigger.display(p)).unwrap_or_default();
            format!("{trigger} is already used by {}", names.join(", "))
        }
        AssignError::Reserved(t) => format!("{} is reserved by the system", t.display(p)),
        e => e.to_string(),
    }
}

/// `shortcuts.set {id, shortcut?, hold?, force?}`: gives entry `id` a shortcut (null: its default;
/// empty: none) and a hold shortcut (null or empty: none). Without `hold`, a missing `shortcut`
/// removes the shortcut, as it always has; with `hold` alone, the shortcut is left as it is. Both
/// are checked before either is kept.
fn set(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = text_param(p, "id")?.unwrap_or_default();
    let shortcut = text_param(p, "shortcut")?;
    let force = menus::opt_bool(p, "force")?.unwrap_or(false);
    let hold = text_param(p, "hold")?;
    dialog_closed(app)?;
    let mut keymap = editing_keymap(app)?;
    let command = command_of(&key);
    let def = keymap.bindable(&command).cloned().ok_or_else(|| format!("unknown tool or command `{key}`"))?;
    let (mut singles, mut holds): (Vec<Binding>, Vec<Binding>) = keymap.bindings(&command).into_iter().partition(|b| !b.trigger.is_hold());
    let has_hold = p.get("hold").is_some();
    if p.get("shortcut").is_some() || !has_hold {
        singles = match (p.get("shortcut"), shortcut.as_deref()) {
            (Some(Value::Null), _) => keymap.default_bindings(&command).into_iter().filter(|b| !b.trigger.is_hold()).collect(),
            (_, None | Some("")) => vec![],
            (_, Some(text)) => vec![Binding::new(parse_trigger(text)?)],
        };
    }
    if has_hold {
        holds = match hold.as_deref() {
            None | Some("") => vec![],
            Some(text) => vec![Binding::new(Trigger::hold(parse_chord(text)?))],
        };
    }
    // An entry the editor lists only with a shortcut takes a hold shortcut only with one.
    if singles.is_empty() && needs_a_value(&key) {
        if hold.as_deref().is_some_and(|h| !h.is_empty()) {
            return Err(format!("`{key}` needs a shortcut before it can have a hold shortcut"));
        }
        holds.clear();
    }
    for b in singles.iter().chain(&holds) {
        accept(&def, b)?;
    }
    let policy = if force { Policy::Steal } else { Policy::Refuse };
    let bindings: Vec<Binding> = singles.into_iter().chain(holds).collect();
    let done = match keymap.set_bindings(&command, bindings.clone(), policy) {
        // An action's function key is the action's own option, not a binding to take: the command
        // gets the chord beside it, and the action's key keeps priority.
        Err(AssignError::Taken(holders)) if holders.iter().all(|h| h.command.id == crate::panels::actions::PLAY_ACTION) => {
            keymap.set_bindings(&command, bindings, Policy::Keep)
        }
        r => r,
    }
    .map_err(|e| refusal(e, &keymap))?;
    let mut removed: Vec<String> = vec![];
    for h in &done.taken_from {
        let k = entry_key(&h.command);
        if !removed.contains(&k) {
            removed.push(k);
        }
    }
    // An entry listed only with a shortcut loses its hold shortcut with its last shortcut.
    for k in &removed {
        let other = command_of(k);
        if needs_a_value(k) && keymap.bindings(&other).iter().all(|b| b.trigger.is_hold()) && !keymap.clear(&other) {
            return Err(AssignError::Full.to_string());
        }
    }
    give_back_function_keys(&mut keymap);
    keep(app, &keymap)?;
    let (shortcut, hold) = row_bindings(&keymap, &key);
    Ok(json!({"id": key, "shortcut": shortcut, "hold": hold, "removedFrom": removed}))
}

/// Off macOS, Cut, Copy and Paste keep only their primary shortcut while another command has their
/// function key ([`stored::legacy`] pins them so); each takes its key back, with its defaults, once
/// no other command has it.
fn give_back_function_keys(keymap: &mut Keymap) {
    if keymap.platform().is_mac() {
        return;
    }
    for (edit, primary, alias) in crate::keymap::EDIT_FUNCTION_KEYS {
        let (Ok(primary), Ok(alias)) = (parse_chord(primary), parse_chord(alias)) else { continue };
        let command = command_of(edit);
        let pinned = keymap.overrides().get(&stored_key(edit)).is_some_and(|texts| {
            let primary = Binding::new(primary);
            matches!(texts, [text] if Binding::parse(text).is_ok_and(|b| b == primary))
        });
        // A component's own F2 (a preset row's Rename) acts only where that component has the keys.
        let app_holder = |c: &craft::keymap::CommandRef| keymap.bindable(c).is_none_or(|d| d.scope.is_app());
        if pinned && keymap.holders(&alias).iter().filter(|c| app_holder(c)).all(|c| *c == command) {
            keymap.reset(&command);
        }
    }
}

/// `shortcuts.list {query?}`: the entries the editor lists, with their shortcuts.
fn list(app: &VectorcraftApp, p: &Value) -> Result<Value, String> {
    let q = text_param(p, "query")?.unwrap_or_default().to_lowercase();
    let keymap = editing_keymap(app)?;
    let snapshot = snapshot();
    let rows = snapshot
        .entries
        .iter()
        .filter(|e| listed(keymap.overrides(), e))
        .filter(|e| q.is_empty() || e.label.to_lowercase().contains(&q) || e.key.to_lowercase().contains(&q))
        .map(|e| {
            let (shortcut, hold) = row_bindings(&keymap, &e.key);
            let overridden = keymap.overrides().get(&stored_key(&e.key)).is_some();
            json!({"id": e.key, "label": e.label, "group": e.group, "shortcut": shortcut, "hold": hold, "default": default_of(&e.key), "overridden": overridden})
        })
        .collect();
    Ok(Value::Array(rows))
}

/// `shortcuts.conflicts`: each shortcut more than one entry has, hold shortcuts as `Hold X`.
fn conflicts(app: &VectorcraftApp) -> Result<Value, String> {
    let keymap = editing_keymap(app)?;
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // The framework's Select All and Delete run the app's own (`shortcuts::engine_id`): one command, no clash.
    let same = |a: &str, b: &str| crate::shortcuts::engine_id(a) == Some(b) || crate::shortcuts::engine_id(b) == Some(a);
    for c in keymap.conflicts().into_iter().filter(|c| c.kind == ConflictKind::Clash && !same(&c.a.0.id, &c.b.0.id)) {
        let ids = by.entry(c.chord.to_string()).or_default();
        for k in [entry_key(&c.a.0), entry_key(&c.b.0)] {
            if !ids.contains(&k) {
                ids.push(k);
            }
        }
    }
    Ok(Value::Array(by.into_iter().map(|(c, k)| json!({"shortcut": c, "ids": k})).collect()))
}

/// Makes the set `name` the one in use.
fn select_set(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    dialog_closed(app)?;
    app.ui.shortcut_sets.select(name, &BUILTINS).map_err(|e| e.to_string())?;
    use_active_set(app);
    Ok(Value::Null)
}

/// `shortcuts.preset {name}`: a built-in set becomes the one in use.
fn preset(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let name = text_param(p, "name")?;
    match name.as_deref().map(set_name).filter(|n| PRESETS.contains(n)) {
        Some(n) => select_set(app, n),
        None => Err(format!("unknown preset (one of {})", PRESETS.join(", "))),
    }
}

/// `shortcuts.export {path?}`: the set in use as craft-keymap JSON, written to `path` or a picked
/// file.
fn export(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let path = text_param(p, "path")?;
    let sets = &app.ui.shortcut_sets;
    let json = sets.export(sets.active_name(&BUILTINS), &BUILTINS).map_err(|e| e.to_string())?;
    let filters = file_asks::filters(&[("Shortcut sets", &["json"])]);
    crate::io::save::write_named_as(app, path, "VectorCraft Shortcuts.json", filters, json.into_bytes(), |_, path| Ok(json!({ "path": path })))
}

/// `shortcuts.exportText {path?, format?, pageSize?}`: the set in use as text, a shortcut sheet or
/// CSV. A given path is written as text unless `format` says otherwise; a save dialog offers every
/// format (text first) unless `format` names one, and its answer picks the format. With no file
/// dialog, writer or download, the file comes back: text under `text`, the sheet as base64 under
/// `pdf`.
fn export_text(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let path = text_param(p, "path")?;
    let format = text_param(p, "format")?
        .map(|f| {
            TextFormat::ALL
                .into_iter()
                .find(|t| t.name() == f)
                .ok_or_else(|| format!("`format` must be one of {}", TextFormat::ALL.map(TextFormat::name).join(", ")))
        })
        .transpose()?;
    let page = match text_param(p, "pageSize")? {
        None => default_sheet_page(app),
        Some(n) => PageSize::parse(&n).ok_or_else(|| format!("`pageSize` must be one of {}", PageSize::ALL.map(PageSize::name).join(", ")))?,
    };
    let keymap = current_keymap(app)?;
    let name = app.ui.shortcut_sets.active_name(&BUILTINS).to_string();
    if let Some(path) = path {
        return write_text_file(app, &keymap, &name, format.unwrap_or(TextFormat::Text), page, Some(path));
    }
    let shown = format.unwrap_or(TextFormat::Text);
    if !crate::io::is_web(app) {
        let offered: Vec<TextFormat> = format.map_or_else(|| TextFormat::ALL.to_vec(), |f| vec![f]);
        let request = offered.iter().fold(PickRequest::save(SHORTCUT_SETS).file_name(shown.file_name()), |r, f| f.filter(r));
        let (km, set) = (keymap.clone(), name.clone());
        let asked = file_asks::ask(app, request, Owner::Free, move |app, picked| {
            let Picked::Saved { path, file_type } = picked else { return Err("cancelled".into()) };
            let answer = path.to_string_lossy().into_owned();
            let typed = file_type.and_then(|i| offered.get(i)).copied();
            let picked = format.unwrap_or_else(|| TextFormat::picked(&answer, typed.map(TextFormat::extension)));
            // The format that `format` names, or that the dialog reports as the type picked, gives
            // the name its extension. A file already at a path changed that way, which the dialog
            // did not check, is replaced only after asking.
            let path = if format.is_some() || typed.is_some() { TextFormat::with_extension(&answer, picked) } else { answer.clone() };
            if path != answer && std::path::Path::new(&path).exists() {
                let to = path.clone();
                return Ok(file_asks::ask_replace(app, &path, &answer, move |app| write_text_file(app, &km, &set, picked, page, Some(to))));
            }
            write_text_file(app, &km, &set, picked, page, Some(path))
        });
        if !asked.is_null() {
            return Ok(asked);
        }
    }
    write_text_file(app, &keymap, &name, shown, page, None)
}

/// Writes Export Text's file to `path`; with none, downloads it, or returns it.
fn write_text_file(
    app: &mut VectorcraftApp,
    keymap: &Keymap,
    name: &str,
    format: TextFormat,
    page: PageSize,
    path: Option<String>,
) -> Result<Value, String> {
    let bytes = export_text_file(format, keymap, name, page, &app.host)?;
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

/// `shortcuts.import {path? | data?}`: a set in craft-keymap JSON or in the form earlier versions
/// wrote becomes the set in use; a set another app saved as text opens its review.
fn import(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    dialog_closed(app)?;
    if let Some(v) = p.get("data") {
        return import_set(app, v);
    }
    match text_param(p, "path")? {
        Some(path) => with_set_at(app, &path, import_bytes),
        None => Ok(file_asks::ask(app, crate::dialogs::shortcut_import::open_request(), Owner::Free, |app, picked| {
            with_picked_set(app, picked, import_bytes)
        })),
    }
}

/// [`import`] of the file `name` holding `bytes`.
fn import_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    if crate::dialogs::shortcut_import::is_text_set(name, bytes) {
        return crate::dialogs::shortcut_import::open(app, name, bytes);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    import_set(app, &v)
}

/// [`import`] of the set `v`.
fn import_set(app: &mut VectorcraftApp, v: &Value) -> Result<Value, String> {
    let name = app.ui.shortcut_sets.import(&keymap_json(v)?, &BUILTINS).map_err(|e| e.to_string())?;
    use_active_set(app);
    Ok(json!({"set": name, "overrides": app.shortcut_overrides.len()}))
}

/// `shortcuts.*` UI commands. `None` = not ours.
pub fn run_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "edit.keyboardShortcuts" => {
            if app.shortcuts_dialog.is_none() {
                open(app);
            }
            Ok(Value::Null)
        }
        "shortcuts.set" => set(app, p),
        "shortcuts.list" => list(app, p),
        "shortcuts.conflicts" => conflicts(app),
        "shortcuts.reset" => select_set(app, PRESETS[0]),
        "shortcuts.preset" => preset(app, p),
        "shortcuts.export" => export(app, p),
        "shortcuts.exportText" => export_text(app, p),
        "shortcuts.import" => import(app, p),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::LangExt as _;
    use craft::keymap::Scopes;

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

    /// An app whose window has drawn a frame, so the `shortcuts.*` commands have its keymap.
    fn app() -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    /// The keymap of the entries with `overrides`, on this platform.
    fn keymap_with(overrides: Overrides) -> Keymap {
        let p = crate::keymap::platform();
        Keymap::new(crate::keymap::catalog_defs(p, &snapshot()), Scopes::new(), p).with_overrides(overrides)
    }

    /// Overrides of entry keys, in VectorCraft's spelling.
    fn overrides(entries: Value) -> Overrides {
        stored::from_value(entries)
    }

    /// The `shortcuts.list` row of entry `id`.
    fn row(app: &mut VectorcraftApp, id: &str) -> Option<Value> {
        let rows = with_steady_plugins(|| app.run("shortcuts.list", json!({ "query": id })).unwrap());
        rows.as_array().and_then(|rows| rows.iter().find(|r| r["id"] == id).cloned())
    }

    /// A chord in VectorCraft's spelling, as the commands report it.
    fn spelled(text: &str) -> String {
        parse_chord(text).unwrap().to_string()
    }

    /// The files a test's writer gets: (path, bytes).
    type Files = std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>>;

    /// Gives `app` a writer that keeps what it writes; a path under `/missing/` can't be written.
    fn writer(app: &mut VectorcraftApp) -> Files {
        let written: Files = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |path: &str, b: &[u8]| {
            if path.starts_with("/missing/") {
                return Err(format!("can't write {path}"));
            }
            w.borrow_mut().push((path.to_string(), b.to_vec()));
            Ok(())
        }));
        written
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
            // Another caller builds the snapshot and catalog of the revision read before the walk.
            crate::keymap::catalog_defs(crate::keymap::platform(), &snapshot());
            install(during);
        })));
        let s = snapshot();
        let keys: Vec<&str> = s.entries.iter().map(|e| e.key.as_str()).collect();
        let catalog = crate::keymap::catalog_defs(crate::keymap::platform(), &s).len();
        vectorcraft_plugins::registry::remove(before);
        vectorcraft_plugins::registry::remove(during);
        assert!(keys.contains(&"plugin:test.ui.walk-during"), "the walk lists the plug-in");
        assert_eq!(catalog, s.entries.len(), "the catalog of the snapshot's revision has every entry");
    }

    /// A saved override that no chord can be (a modifier's own key) keeps the default; an empty
    /// one leaves the entry without a shortcut.
    #[test]
    fn saved_modifier_only_overrides_give_the_default_back() {
        // The Selection tool has a default key to give back; the Group Selection tool production's test used has none.
        let km = keymap_with(overrides(json!({"tool:selection": "Shift+ShiftLeft", "edit.preferences": ""})));
        let group = command_of("tool:selection");
        assert!(!km.default_bindings(&group).is_empty());
        assert_eq!(km.bindings(&group), km.default_bindings(&group));
        assert!(!km.problems().is_empty(), "the text that is no chord is reported");
        assert!(km.bindings(&command_of("edit.preferences")).is_empty());
    }

    /// A tool takes one key, alone or with Shift; other commands take any chord.
    #[test]
    fn tool_shortcuts_must_be_single_keys() {
        let p = crate::keymap::platform();
        let defs = crate::keymap::catalog_defs(p, &snapshot());
        let def = |key: &str| defs.iter().find(|d| d.command == command_of(key)).cloned().unwrap_or_else(|| panic!("no row {key}"));
        let accepts = |def: &CommandDef, text: &str| accept(def, &Binding::parse(text).unwrap()).is_ok();
        let (pen, prefs) = (def("tool:pen"), def("edit.preferences"));
        for text in ["Cmd+K", "Shift+Alt+K", "K K"] {
            assert!(!accepts(&pen, text), "{text}");
            assert!(accepts(&prefs, text), "{text}");
        }
        for text in ["K", "Shift+K"] {
            assert!(accepts(&pen, text), "{text}");
        }
    }

    /// `shortcuts.export` writes the set in use as craft-keymap JSON, which `shortcuts.import`
    /// reads back to the same shortcuts; a set in the form earlier versions wrote imports too, and a
    /// file in neither form changes nothing.
    #[test]
    fn export_import_round_trip() {
        let (mut app, _ctx) = app();
        let written = writer(&mut app);
        app.run("shortcuts.set", json!({"id": "tool:pen", "shortcut": "Shift+9"})).unwrap();
        app.run("shortcuts.set", json!({"id": "edit.preferences", "shortcut": ""})).unwrap();
        app.run("shortcuts.export", json!({"path": "/sets/Mine.json"})).unwrap();
        let file: Value = serde_json::from_slice(&written.borrow()[0].1).unwrap();
        assert_eq!(file["format"], JSON_FORMAT);
        app.run("shortcuts.reset", json!({})).unwrap();
        assert_ne!(row(&mut app, "tool:pen").unwrap()["shortcut"], spelled("Shift+9"));
        app.run("shortcuts.import", json!({ "data": file })).unwrap();
        assert_eq!(row(&mut app, "tool:pen").unwrap()["shortcut"], spelled("Shift+9"));
        assert_eq!(row(&mut app, "edit.preferences").unwrap()["shortcut"], Value::Null);
        app.run("shortcuts.reset", json!({})).unwrap();
        let old = json!({"format": "vectorcraft-shortcuts", "version": 1, "set": "Studio", "overrides": {"tool:pen": "Shift+8"}});
        app.run("shortcuts.import", json!({ "data": old })).unwrap();
        assert_eq!(app.ui.shortcut_sets.active(), "Studio");
        assert_eq!(row(&mut app, "tool:pen").unwrap()["shortcut"], spelled("Shift+8"));
        let sets = app.ui.shortcut_sets.clone();
        assert!(app.run("shortcuts.import", json!({"data": {"overrides": {"x": "Cmd+Nope+Q"}}})).is_err());
        assert!(app.run("shortcuts.import", json!({"data": {}})).is_err());
        assert_eq!(app.ui.shortcut_sets, sets);
    }

    /// `shortcuts.exportText` writes the set in use with its name, and a path that can't be
    /// written or isn't a string gives an error and writes nothing.
    #[test]
    fn export_text_command_writes_the_sets_name_and_rejects_a_bad_path() {
        let (mut app, _ctx) = app();
        let written = writer(&mut app);
        let old = json!({"format": "vectorcraft-shortcuts", "version": 1, "set": "Studio", "overrides": {}});
        app.run("shortcuts.import", json!({ "data": old })).unwrap();
        assert_eq!(app.run("shortcuts.exportText", json!({"path": "/sets/Studio.txt"})), Ok(json!({"path": "/sets/Studio.txt"})));
        assert_eq!(
            app.run("shortcuts.exportText", json!({"path": "/missing/dir/Studio.txt"})),
            Err("can't write /missing/dir/Studio.txt".to_string())
        );
        assert_eq!(app.run("shortcuts.exportText", json!({"path": 5})), Err("`path` must be a string".to_string()));
        let files = written.borrow();
        let [(path, text)] = files.as_slice() else { panic!("{} files", files.len()) };
        assert_eq!(path, "/sets/Studio.txt");
        let text = String::from_utf8_lossy(text);
        assert!(text.lines().next().is_some_and(|title| title.contains("Studio")), "{text}");
    }

    /// The sheet's sections: Tools first, then each top-level menu once, in the order the dialog
    /// first lists a command of it, holding every row the dialog lists with its shortcut, the
    /// command named without its top-level menu.
    #[test]
    fn export_sections_hold_every_listed_row_once_by_top_level_menu() {
        let (sections, listed_rows, tops) = with_steady_plugins(|| {
            let km = keymap_with(overrides(json!({"edit.preferences": "Ctrl+Alt+Shift+9"})));
            let listed: Vec<Entry> = entries().iter().filter(|e| listed(km.overrides(), e)).cloned().collect();
            let mut tops: Vec<String> = vec![];
            for e in listed.iter().filter(|e| !e.is_tool) {
                let top = e.group.split(" › ").next().filter(|t| !t.is_empty()).unwrap_or(MENU_COMMANDS).to_string();
                if !tops.contains(&top) {
                    tops.push(top);
                }
            }
            (export_sections(&km), listed.len(), tops)
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
        let pen = shown_name(&entry("tool:pen").expect("the Pen tool"));
        assert!(sections[0].rows.iter().any(|r| r.name == pen && !r.shortcut.is_empty()), "the Pen tool's row with its key");
    }

    /// `format` picks the file: text (the default) or the PDF sheet. A given path is written as
    /// text whatever its extension, and a bad `format` or `pageSize` is an error that writes nothing.
    #[test]
    fn export_text_command_writes_text_or_a_sheet() {
        let (mut app, _ctx) = app();
        let written = writer(&mut app);
        let files = with_steady_plugins(|| {
            written.borrow_mut().clear();
            app.run("shortcuts.exportText", json!({"path": "/a/plain.txt"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/named.txt", "format": "text"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/as-text.pdf"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/sheet.pdf", "format": "sheet", "pageSize": "a4"})).unwrap();
            app.run("shortcuts.exportText", json!({"path": "/a/sheet.txt", "format": "sheet"})).unwrap();
            written.borrow().clone()
        });
        let [plain, named, as_text, sheet, sheet_as_named] = files.as_slice() else { panic!("{} files", files.len()) };
        for f in [named, as_text] {
            assert_eq!(f.1, plain.1, "{}", f.0);
        }
        assert!(!plain.1.starts_with(b"%PDF-"), "the text is no PDF");
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
        let (mut app, _ctx) = app();
        let (returned, files, sections) = with_steady_plugins(|| {
            app.services.write = None;
            let returned = app.run("shortcuts.exportText", json!({"format": "csv"}));
            let written = writer(&mut app);
            app.run("shortcuts.exportText", json!({"path": "/a/list.csv", "format": "csv"})).unwrap();
            let files = written.borrow().clone();
            (returned, files, export_sections(&current_keymap(&app).unwrap()))
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
        let (mut app, _ctx) = app();
        let (text, sheet) =
            with_steady_plugins(|| (app.run("shortcuts.exportText", json!({})), app.run("shortcuts.exportText", json!({"format": "sheet"}))));
        let text = text.unwrap();
        assert!(text["text"].as_str().is_some_and(|t| !t.is_empty()), "{text}");
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

    /// A saved F2 on another command leaves Cut on its primary shortcut, with no conflict.
    #[test]
    fn saved_function_key_assignment_does_not_change_cut_in_the_editor() {
        if crate::keymap::platform().is_mac() {
            return;
        }
        let (mut app, _ctx) = app();
        app.ui.shortcut_sets = stored::legacy(&json!({"tool:pen": "F2"}), None, &Value::Null);
        use_active_set(&mut app);
        assert_eq!(row(&mut app, "edit.cut").unwrap()["shortcut"], spelled("Cmd+X"));
        assert_eq!(row(&mut app, "tool:pen").unwrap()["shortcut"], spelled("F2"));
        assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]));
    }

    /// Cut, Copy and Paste take F2, F3 and F4 back, with their defaults, once the command that had
    /// the key lets it go: one that took it with `force`, and one a saved set gave it to.
    #[test]
    fn reset_stolen_edit_function_keys_restores_their_defaults() {
        if crate::keymap::platform().is_mac() {
            return;
        }
        for (alias, edit) in [("F2", "edit.cut"), ("F3", "edit.copy"), ("F4", "edit.paste")] {
            let (mut app, _ctx) = app();
            let r = app.run("shortcuts.set", json!({"id": "tool:pen", "shortcut": alias, "force": true})).unwrap();
            assert_eq!(r["removedFrom"], json!([edit]), "{alias}");
            app.run("shortcuts.set", json!({"id": "tool:pen", "shortcut": null})).unwrap();
            let r = row(&mut app, edit).unwrap();
            assert_eq!((r["overridden"].as_bool(), r["shortcut"].clone()), (Some(false), json!(r["default"].as_str().map(spelled))), "{alias}");
            let km = current_keymap(&app).unwrap();
            assert_eq!(km.holder(&parse_chord(alias).unwrap()), Some(command_of(edit)), "{alias} is {edit}'s again");

            app.ui.shortcut_sets = stored::legacy(&json!({"edit.preferences": alias}), None, &Value::Null);
            use_active_set(&mut app);
            app.run("shortcuts.set", json!({"id": "edit.preferences", "shortcut": "Cmd+Alt+Shift+F12"})).unwrap();
            let km = current_keymap(&app).unwrap();
            assert_eq!(km.holder(&parse_chord(alias).unwrap()), Some(command_of(edit)), "saved {alias} is {edit}'s again");
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
        let (mut app, _ctx) = app();
        app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": "Cmd+Alt+Shift+F8"})).unwrap();
        assert_eq!(row(&mut app, "edit.duplicate").unwrap()["shortcut"], spelled("Cmd+Alt+Shift+F8"));
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
        let (mut app, _ctx) = app();
        app.run("shortcuts.set", json!({"id": "symbol.breakLink", "shortcut": "Cmd+Alt+Shift+F9", "force": true})).unwrap();
        assert_eq!(row(&mut app, "symbol.breakLink").unwrap()["shortcut"], spelled("Cmd+Alt+Shift+F9"));
    }

    /// One pass of Keyboard Shortcuts in the window of `ctx`, which `show` takes the keymap from.
    fn dialog_pass(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
    }

    /// An open dialog lists a live-effect plug-in installed while it is open, with the shortcut its
    /// set gives it, and drops the row while the plug-in is removed, keeping that shortcut in the
    /// set.
    #[test]
    fn an_open_dialog_follows_live_effect_plugins() {
        use vectorcraft_plugins::wat as tpl;
        let (plugin, key) = ("test.ui.openfx", "effect:plugin.test.ui.openfx");
        let wasm = wat::parse_str(tpl::module(&tpl::manifest(plugin, "effect", r#"{"wiggle":{"type":"bool"}}"#), tpl::ECHO, "", 1)).unwrap();
        let (mut app, ctx) = app();
        let saved = overrides(json!({ key: "Cmd+Alt+Shift+F5" }));
        app.ui.shortcut_sets.store(saved, &BUILTINS, CUSTOM).unwrap();
        use_active_set(&mut app);
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        open(&mut app);
        dialog_pass(&mut app, &ctx);
        let lists = |app: &VectorcraftApp| {
            let keymap = app.shortcuts_dialog.as_ref().and_then(|d| d.keymap.as_ref()).expect("the dialog's keymap");
            keymap.bindable(&command_of(key)).is_some().then(|| shown_bindings(keymap, key))
        };
        assert_eq!(lists(&app), None);
        let shortcut = parse_chord("Cmd+Alt+Shift+F5").unwrap().display(crate::keymap::platform());
        vectorcraft_plugins::registry::install_bytes(&wasm).unwrap();
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        dialog_pass(&mut app, &ctx);
        assert_eq!(lists(&app), Some(shortcut.clone()));
        vectorcraft_plugins::registry::remove(plugin);
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        dialog_pass(&mut app, &ctx);
        assert_eq!(lists(&app), None);
        assert!(app.ui.shortcut_sets.active_overrides(&BUILTINS).get(&stored_key(key)).is_some(), "the set keeps the shortcut");
        vectorcraft_plugins::registry::install_bytes(&wasm).unwrap();
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        dialog_pass(&mut app, &ctx);
        assert_eq!(lists(&app), Some(shortcut));
        vectorcraft_plugins::registry::remove(plugin);
    }

    /// A menu command whose params need a value (Units needs `{units}`) is not listed while it has
    /// no shortcut. A shortcut the user gave it keeps it listed, where it can be changed or cleared.
    #[test]
    fn commands_that_need_a_value_are_listed_only_with_the_users_shortcut() {
        let (mut app, _ctx) = app();
        let list = |app: &mut VectorcraftApp| app.run("shortcuts.list", json!({"query": "document.setUnits"})).unwrap();
        assert_eq!(list(&mut app), json!([]));
        app.run("shortcuts.set", json!({"id": "document.setUnits", "shortcut": "Cmd+Alt+Shift+F12", "force": true})).unwrap();
        let rows = list(&mut app);
        let e = &rows[0];
        assert_eq!(
            (e["id"].as_str(), e["shortcut"].clone(), e["overridden"].as_bool()),
            (Some("document.setUnits"), json!(spelled("Cmd+Alt+Shift+F12")), Some(true))
        );
        // An empty shortcut gives it no key, so it is not listed.
        app.run("shortcuts.set", json!({"id": "document.setUnits", "shortcut": ""})).unwrap();
        assert_eq!(list(&mut app), json!([]));
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
        let saved = overrides(json!({"transparency.set": "Cmd+Alt+Shift+F10"}));
        assert!(entry("transparency.set").is_some_and(|e| listed(&saved, &e)));
        let (mut app, _ctx) = app();
        for id in ["transparency.set", "transparency.setOpacityMask"] {
            let list = app.run("shortcuts.list", json!({ "query": id })).unwrap();
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
                menus::sub("Open With", vec![menus::cp("Spins…", "file.openWith", json!({"importer": "plugin.test.spins"}))]),
                menus::cp("Filter…", "plugin.dialog", json!({"id": "test.filter"})),
                menus::cp("Mine", "window.toolbar.custom", json!({"name": "Mine"})),
                menus::named("tidy.js", crate::scripts_menu::COMMAND, json!({"path": "/scripts/tidy.js"})),
                menus::cp("Clean Up…", "object.path.cleanUp", json!({"strayPoints": true})),
            ],
        )];
        let items = bar_items(&tree);
        assert!(!items.contains_key("file.openWith"), "{items:?}");
        assert!(!items.contains_key("plugin.dialog"), "{items:?}");
        assert!(!items.contains_key("window.toolbar.custom"), "{items:?}");
        assert!(!items.contains_key(crate::scripts_menu::COMMAND), "{items:?}");
        assert_eq!(items.get("object.path.cleanUp"), Some(&vec![("Clean Up…".to_string(), json!({"strayPoints": true}))]));
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
        let (mut app, _ctx) = app();
        app.run("shortcuts.set", json!({"id": "object.isolate", "shortcut": "Cmd+Alt+Shift+F11", "force": true})).unwrap();
        assert_eq!(row(&mut app, "object.isolate").unwrap()["shortcut"], spelled("Cmd+Alt+Shift+F11"));
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
            .map(|(id, _)| id.as_str())
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
        let (mut app, _ctx) = app();
        let written = writer(&mut app);
        for id in
            [crate::addons::pathtools::set_corners::COMMAND, "perspective.grid.lock", crate::dialogs::custom_dictionary::COMMAND, "ui.recolorDialog"]
        {
            app.run("shortcuts.set", json!({"id": id, "shortcut": "Cmd+Alt+Shift+F11", "force": true})).unwrap();
            app.run("shortcuts.export", json!({"path": "/sets/Mine.json"})).unwrap();
            let file: Value = serde_json::from_slice(&written.borrow_mut().pop().unwrap().1).unwrap();
            app.run("shortcuts.reset", json!({})).unwrap();
            app.run("shortcuts.import", json!({ "data": file })).unwrap();
            assert_eq!(row(&mut app, id).unwrap()["shortcut"], spelled("Cmd+Alt+Shift+F11"), "{id}");
        }
    }

    /// No two entries of Keyboard Shortcuts have the same English name. A command that an item of
    /// the menu bar runs takes that item's menu path (Window › Swatch Libraries › Other Library…), a
    /// command whose id extends another command's id takes that command's label (Character Style
    /// Options › Reset Panel), and the other entries that would share a name show their key, except
    /// the one an item of the menu bar runs: File › New… is the New Document dialog's entry, and
    /// `file.new` is File › New… (file.new). An entry whose name no other entry has keeps its group.
    /// A conflict in `shortcuts.set` names the entry that has the shortcut by its whole name.
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
        let e = entry("text.fitHeadline").unwrap();
        assert!(shown_name(&e).ends_with("Fit Headline"), "{}", shown_name(&e));
        let (mut app, _ctx) = app();
        let set = |app: &mut VectorcraftApp, id: &str, shortcut: &str| app.run("shortcuts.set", json!({"id": id, "shortcut": shortcut}));
        set(&mut app, "window.swatchLibrary.other", "Cmd+Alt+Shift+F11").unwrap();
        let err = set(&mut app, "window.brushLibrary.other", "Cmd+Alt+Shift+F11").unwrap_err();
        assert!(err.ends_with("is already used by Window › Swatch Libraries › Other Library…"), "{err}");
        set(&mut app, "type.fitHeadline", "Cmd+Alt+Shift+F10").unwrap();
        let err = set(&mut app, "text.fitHeadline", "Cmd+Alt+Shift+F10").unwrap_err();
        assert!(err.ends_with("is already used by Type › Fit Headline (type.fitHeadline)"), "{err}");
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
        let (mut app, _ctx) = app();
        let sets = app.ui.shortcut_sets.clone();
        for (id, p, err) in [
            ("shortcuts.set", json!({"id": 5, "shortcut": "Cmd+Alt+Shift+F11"}), "`id` must be a string"),
            ("shortcuts.set", json!({"id": "object.isolate", "shortcut": 7}), "`shortcut` must be a string"),
            ("shortcuts.set", json!({"id": "object.isolate", "shortcut": "Cmd+Alt+Shift+F11", "force": "yes"}), "`force` must be true or false"),
            ("shortcuts.list", json!({"query": 5}), "`query` must be a string"),
            ("shortcuts.preset", json!({"name": true}), "`name` must be a string"),
        ] {
            assert_eq!(app.run(id, p.clone()), Err(err.to_string()), "{id} {p}");
            assert_eq!(app.ui.shortcut_sets, sets, "{id} {p}");
        }
        let (all, null_query) =
            with_steady_plugins(|| (app.run("shortcuts.list", json!({})).unwrap(), app.run("shortcuts.list", json!({"query": null}))));
        assert_eq!(null_query, Ok(all));
        let r = app.run("shortcuts.set", json!({"id": "object.isolate", "shortcut": "Cmd+Alt+Shift+F11", "force": null})).unwrap();
        assert_eq!(r["shortcut"], spelled("Cmd+Alt+Shift+F11"));
        assert!(app.run("shortcuts.preset", json!({"name": null})).unwrap_err().starts_with("unknown preset"));
    }

    /// `shortcuts.set {hold}` writes a `Hold X` binding beside the entry's shortcut, which it leaves
    /// alone; `shortcuts.list` reports both; Use Default (`shortcut: null`) and a new shortcut keep
    /// the hold, and an empty or null hold removes it.
    #[test]
    fn a_hold_shortcut_is_kept_apart_from_the_shortcut() {
        let (mut app, _ctx) = app();
        let before = row(&mut app, "paint.swap").unwrap()["shortcut"].clone();
        let r = app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "x"})).unwrap();
        assert_eq!((r["hold"].clone(), r["shortcut"].clone()), (json!(spelled("X")), before.clone()));
        let saved = app.ui.shortcut_sets.active_overrides(&BUILTINS).get("paint.swap").map(<[String]>::to_vec).unwrap_or_default();
        assert!(saved.iter().any(|t| Trigger::parse(t).is_ok_and(|t| t.is_hold())), "{saved:?}");
        let hold = |app: &mut VectorcraftApp| row(app, "paint.swap").unwrap()["hold"].clone();
        assert_eq!(hold(&mut app), json!(spelled("X")));
        app.run("shortcuts.set", json!({"id": "paint.swap", "shortcut": null})).unwrap();
        assert_eq!(
            (hold(&mut app), row(&mut app, "paint.swap").unwrap()["shortcut"].clone()),
            (json!(spelled("X")), before),
            "Use Default keeps the hold"
        );
        app.run("shortcuts.set", json!({"id": "paint.swap", "shortcut": "Cmd+Alt+Shift+F9"})).unwrap();
        assert_eq!(hold(&mut app), json!(spelled("X")), "a new shortcut keeps the hold");
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": ""})).unwrap();
        assert_eq!(hold(&mut app), Value::Null, "an empty hold removes it");
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+1"})).unwrap();
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": null})).unwrap();
        assert_eq!(hold(&mut app), Value::Null, "a null hold removes it");
    }

    /// A hold shortcut is a key that types a character, alone or with Shift; other keys, a clash
    /// without `force` and a hold that is not a string are refused and change nothing.
    #[test]
    fn hold_shortcuts_refuse_keys_that_cannot_wait() {
        let (mut app, _ctx) = app();
        let sets = app.ui.shortcut_sets.clone();
        for bad in ["Space", "Shift+Space", "~", "Shift+~", "Cmd+X", "Alt+X", "Ctrl+X", "Left", "F5", "Enter", "Delete"] {
            let r = app.run("shortcuts.set", json!({"id": "paint.swap", "hold": bad}));
            assert!(r.is_err(), "{bad}: {r:?}");
        }
        assert_eq!(app.run("shortcuts.set", json!({"id": "paint.swap", "hold": 5})), Err("`hold` must be a string".to_string()));
        assert_eq!(app.ui.shortcut_sets, sets);
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+X"})).unwrap();
        let sets = app.ui.shortcut_sets.clone();
        assert!(app.run("shortcuts.set", json!({"id": "select.none", "hold": "Shift+X"})).is_err(), "a clash without force");
        // A refused hold leaves the shortcut given with it unchanged too.
        let r = app.run("shortcuts.set", json!({"id": "select.none", "shortcut": "Cmd+Alt+Shift+F9", "hold": "Shift+X"}));
        assert!(r.is_err());
        assert_eq!(app.ui.shortcut_sets, sets);
        let r = app.run("shortcuts.set", json!({"id": "select.none", "hold": "Shift+X", "force": true})).unwrap();
        assert_eq!(r["removedFrom"], json!(["paint.swap"]));
        assert_eq!(row(&mut app, "paint.swap").unwrap()["hold"], Value::Null);
    }

    /// A command the editor lists only with a shortcut (Transparency needs a value) takes a hold
    /// shortcut only with one, and loses its hold, as it stops being listed, when the shortcut is
    /// taken away: by clearing it or by giving it to another command.
    #[test]
    fn a_hold_shortcut_needs_a_listed_entry() {
        let (mut app, _ctx) = app();
        assert!(needs_a_value("transparency.set"));
        let sets = app.ui.shortcut_sets.clone();
        let r = app.run("shortcuts.set", json!({"id": "transparency.set", "hold": "Q"}));
        assert!(r.is_err(), "{r:?}");
        assert_eq!(app.ui.shortcut_sets, sets);
        app.run("shortcuts.set", json!({"id": "transparency.set", "shortcut": "Cmd+Alt+Shift+F9", "hold": "Q", "force": true})).unwrap();
        assert_eq!(row(&mut app, "transparency.set").unwrap()["hold"], spelled("Q"));
        app.run("shortcuts.set", json!({"id": "transparency.set", "shortcut": ""})).unwrap();
        assert!(row(&mut app, "transparency.set").is_none(), "no longer listed");
        // The key is free for another command, with no conflict left behind.
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Q"})).unwrap();
        assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]));
        app.run("shortcuts.set", json!({"id": "transparency.set", "shortcut": "Cmd+Alt+Shift+F9", "hold": "Shift+Q"})).unwrap();
        app.run("shortcuts.set", json!({"id": "edit.duplicate", "shortcut": "Cmd+Alt+Shift+F9", "force": true})).unwrap();
        let km = current_keymap(&app).unwrap();
        assert!(km.bindings(&command_of("transparency.set")).is_empty(), "the hold went with the shortcut");
        assert_eq!(row(&mut app, "paint.swap").unwrap()["hold"], spelled("Q"));
    }

    /// Hold shortcuts survive both files: an old file's `holds` import as `Hold X` bindings, and
    /// the export writes them back.
    #[test]
    fn hold_shortcuts_export_and_import() {
        let (mut app, _ctx) = app();
        let written = writer(&mut app);
        let old = json!({"format": "vectorcraft-shortcuts", "version": 1, "set": "Studio", "overrides": {}, "holds": {"paint.swap": "X"}});
        app.run("shortcuts.import", json!({ "data": old })).unwrap();
        assert_eq!(row(&mut app, "paint.swap").unwrap()["hold"], spelled("X"));
        app.run("shortcuts.export", json!({"path": "/sets/Studio.json"})).unwrap();
        let file: Value = serde_json::from_slice(&written.borrow()[0].1).unwrap();
        app.run("shortcuts.reset", json!({})).unwrap();
        assert_eq!(row(&mut app, "paint.swap").unwrap()["hold"], Value::Null);
        app.run("shortcuts.import", json!({ "data": file })).unwrap();
        assert_eq!(row(&mut app, "paint.swap").unwrap()["hold"], spelled("X"));
    }

    /// Export Text…'s shortcut sheet and CSV file list a hold shortcut only while Hold Shortcuts is
    /// on, after the row's shortcuts.
    #[test]
    fn the_sheet_and_csv_list_hold_shortcuts_while_they_act() {
        let (mut app, _ctx) = app();
        let written = writer(&mut app);
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
        let hold = format!("{} {}", tl!("Hold"), parse_chord("X").unwrap().display(crate::keymap::platform()));
        let files = with_steady_plugins(|| {
            written.borrow_mut().clear();
            for on in [false, false, true] {
                app.run("prefs.set", json!({"key": "holdShortcuts", "value": on})).unwrap();
                app.run("shortcuts.exportText", json!({"path": "/a.csv", "format": "csv"})).unwrap();
                app.run("shortcuts.exportText", json!({"path": "/a.pdf", "format": "sheet"})).unwrap();
            }
            app.run("prefs.set", json!({"key": "holdShortcuts", "value": false})).unwrap();
            written.borrow().clone()
        });
        let [(_, csv_off), (_, sheet_off), _, (_, sheet_off_again), (_, csv_on), (_, sheet_on)] = files.as_slice() else {
            panic!("{} files", files.len())
        };
        let (csv_off, csv_on) = (String::from_utf8_lossy(csv_off), String::from_utf8_lossy(csv_on));
        assert!(!csv_off.contains(&hold), "off, the CSV file lists no hold");
        assert!(csv_on.contains(&hold), "on, the CSV file lists it");
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
        assert_ne!(unstamped(sheet_off), unstamped(sheet_on), "on, the sheet lists the hold");
    }

    /// A command that needs a value is listed in the editor once the user gives it a key, but the
    /// command search still leaves it out: a pick gives it no value.
    #[test]
    fn a_command_given_a_key_that_needs_a_value_is_no_pick() {
        let saved = overrides(json!({"document.setUnits": "Cmd+Alt+Shift+F12"}));
        let entry = entries().iter().find(|e| e.key == "document.setUnits").cloned().expect("the editor has a row for it");
        assert!(!listed(&Overrides::default(), &entry) && listed(&saved, &entry));
        assert!(needs_a_value("document.setUnits"));
        assert!(crate::palette::items().iter().all(|(_, id)| id != "document.setUnits"));
    }
}
