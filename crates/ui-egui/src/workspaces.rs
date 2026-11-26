//! Window → Workspace: saved panel/bar layouts (Essentials, Essentials Classic, Painting, …),
//! Reset, New Workspace…, Manage Workspaces…. A workspace is a panel layout kept by the
//! framework's [`WorkspaceSet`] plus a [`WorkspaceRecord`] of the bar flags the layout does not
//! hold; UI brightness and everything else is untouched when switching.
//!
//! A workspace remembers the changes made in it: switching to another one keeps the layout and
//! flags as they were left, switching back brings them back, and Reset <name> returns to the
//! workspace as saved. New Workspace… with the name of a user workspace saves the current layout
//! as it.
//!
//! Every `key` below is a workspace name (any case) or its id.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use craft::behavior::dock::{NamedWorkspace, Workspace as DockWorkspace, WorkspaceChrome, WorkspaceError, WorkspaceKeeps, WorkspaceSet};
use craft::keymap::CommandRef;
use craft::ui::dialog::{DialogResult, dialog_id};
use craft::ui::patterns::dock::{HostedNewWorkspace, NewWorkspace, WorkspaceSwitcher};
use craft::ui::patterns::form_dialogs::{ActionEnabled, HostedListManager, ListManagerAction, ListManagerChoice, ListManagerEntry};
use craft::ui::pickers::SelectLook;
use craft::ui::rails::LastUsed;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::layout_migration::Outcome;
use crate::state::{CustomToolbar, DockTab, UiState};

/// The most workspaces whose changes are remembered; the oldest go first.
const MAX_REMEMBERED: usize = 64;

/// What a workspace keeps beside its panel layout: the bars and their options. The layout itself
/// (panel places, Tools visibility, torn-off strips) lives in [`WorkspaceSet`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub control_bar: bool,
    /// The Control bar is docked to the bottom of the window (built-in workspaces, and workspaces
    /// saved without this field: the top).
    pub control_bar_bottom: bool,
    /// The Control bar floats at `control_bar_float_pos` (built-in workspaces, and workspaces saved
    /// without this field: docked at the edge `control_bar_bottom` names).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub control_bar_floating: bool,
    /// Where the Control bar floats, or last floated (built-in workspaces: none, the first position).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_bar_float_pos: Option<[f32; 2]>,
    /// The groups of the Control bar's options its panel menu has off (built-in workspaces, and
    /// workspaces saved without this field: none).
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty", deserialize_with = "crate::chrome::read_hidden_groups")]
    pub control_bar_hidden_groups: BTreeSet<String>,
    pub rulers: bool,
    /// Compact main Tools spacing (built-in and older workspaces: regular).
    pub toolbar_compact: bool,
    pub toolbar_advanced: bool,
    pub task_bar: bool,
    /// The Contextual Task Bar's actions and chips its Bar Actions has off (built-in workspaces, and
    /// workspaces saved without this field: none).
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty", deserialize_with = "crate::canvas::read_hidden_task_bar_actions")]
    pub task_bar_hidden_actions: BTreeSet<String>,
    pub status_bar: bool,
    pub help_bar: bool,
    /// Tabs closed together by a panel shortcut.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub docking_hidden_groups: Vec<Vec<String>>,
    /// The tool each flyout group of the main Tools shows (built-in workspaces: the first).
    pub tools_last_used: LastUsed<String>,
    /// The toolbars made with New Toolbar in this workspace (built-in workspaces: none).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub toolbars: Vec<CustomToolbar>,
}

impl Default for WorkspaceRecord {
    fn default() -> Self {
        Self {
            control_bar: false,
            control_bar_bottom: false,
            control_bar_floating: false,
            control_bar_float_pos: None,
            control_bar_hidden_groups: BTreeSet::new(),
            rulers: false,
            toolbar_compact: false,
            toolbar_advanced: false,
            task_bar: true,
            task_bar_hidden_actions: BTreeSet::new(),
            status_bar: true,
            help_bar: true,
            docking_hidden_groups: Vec::new(),
            tools_last_used: LastUsed::default(),
            toolbars: Vec::new(),
        }
    }
}

/// The workspaces of a [`UiState`]: the framework's set (layouts, current workspace) and the
/// [`WorkspaceRecord`] kept for each workspace beside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspaces {
    /// Built-in and saved workspaces with their layouts, and the current one.
    pub set: WorkspaceSet<String>,
    /// The record each saved workspace was saved with, by id; a built-in's comes from [`builtins`].
    pub records: BTreeMap<String, WorkspaceRecord>,
    /// The record each workspace was left with, by id, beside the layout `set.edited` keeps.
    pub edited: BTreeMap<String, WorkspaceRecord>,
    /// The ids in `edited`, the one left longest ago first.
    pub edit_order: Vec<String>,
}

impl Default for Workspaces {
    fn default() -> Self {
        let mut set = WorkspaceSet::new(builtin_set());
        set.current = ESSENTIALS_ID.into();
        Self { set, records: BTreeMap::new(), edited: BTreeMap::new(), edit_order: Vec::new() }
    }
}

impl Workspaces {
    /// Make a state read from a file consistent: the built-ins (code, not stored) are supplied
    /// again, only workspaces that exist keep records and remembered changes, no more than
    /// [`MAX_REMEMBERED`] are remembered, and an unknown current workspace becomes Essentials.
    pub fn sanitize(&mut self) {
        self.set.builtin = builtin_set();
        if self.set.find(&self.set.current).is_none() {
            self.set.current = ESSENTIALS_ID.into();
        }
        let set = &self.set;
        self.records.retain(|id, _| set.saved.iter().any(|w| w.id == *id));
        let mut seen = BTreeSet::new();
        self.edit_order.retain(|id| set.find(id).is_some() && seen.insert(id.clone()));
        let order = &self.edit_order;
        self.edited.retain(|id, _| order.contains(id));
        self.set.edited.retain(|id, _| order.contains(id));
        self.trim();
    }

    /// Forget the changes of the oldest workspaces beyond the cap.
    fn trim(&mut self) {
        while self.edit_order.len() > MAX_REMEMBERED {
            let oldest = self.edit_order.remove(0);
            self.edited.remove(&oldest);
            self.set.edited.remove(&oldest);
        }
    }

    /// Forget the record `id` was left with.
    fn forget(&mut self, id: &str) {
        self.edited.remove(id);
        self.edit_order.retain(|x| x != id);
    }

    /// Remember `record` as how `id` was left, as the newest.
    fn remember(&mut self, id: &str, record: WorkspaceRecord) {
        self.edited.insert(id.to_string(), record);
        self.edit_order.retain(|x| x != id);
        self.edit_order.push(id.to_string());
        self.trim();
    }

    /// Read the workspaces of a file saved before [`WorkspaceSet`]: `saved` holds the keys
    /// `workspace` (the current name), `custom_workspaces` and `workspace_changes`. A saved name
    /// that is a built-in's is refused and reported; a remembered layout whose name no workspace
    /// has is dropped, and one per workspace is kept whatever the case.
    pub(crate) fn from_legacy(saved: &Value, order: &[DockTab]) -> (Workspaces, Outcome) {
        let mut out = Workspaces::default();
        let mut outcome = Outcome { intact: true, converted: false };
        let list = |key: &str| saved.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let name_of = |v: &Value| v.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let read = |v: &Value, outcome: &mut Outcome| -> (WorkspaceRecord, Option<DockWorkspace<String>>) {
            let mut record = serde_json::from_value::<WorkspaceRecord>(v.clone()).unwrap_or_else(|_| {
                outcome.intact = false;
                WorkspaceRecord::default()
            });
            crate::layout_migration::assign_toolbar_ids(&mut record.toolbars);
            let (layout, converted) = crate::layout_migration::legacy_layout(v, order);
            *outcome = outcome.and(converted);
            // A record with nothing to convert keeps no layout of its own.
            (record, converted.converted.then_some(layout))
        };
        for v in list("custom_workspaces") {
            let (record, layout) = read(v, &mut outcome);
            let layout = layout.unwrap_or_else(|| crate::panel_docking::default_workspace(&[]));
            match out.set.save(&name_of(v), layout, chrome_of(&record), WorkspaceKeeps::NONE) {
                Ok(id) => {
                    out.records.insert(id, record);
                }
                Err(_) => outcome.intact = false,
            }
        }
        for v in list("workspace_changes") {
            let Some((id, own)) = resolve(&out.set, &name_of(v)).map(|w| (w.id.clone(), w.layout.clone())) else { continue };
            if out.edited.contains_key(&id) {
                continue;
            }
            let (record, layout) = read(v, &mut outcome);
            out.set.edited.insert(id.clone(), layout.unwrap_or(own));
            out.remember(&id, record);
        }
        let current = saved.get("workspace").and_then(Value::as_str).and_then(|n| resolve(&out.set, n)).map(|w| w.id.clone());
        out.set.current = current.unwrap_or_else(|| ESSENTIALS_ID.into());
        (out, outcome)
    }
}

pub const ESSENTIALS: &str = "Essentials";
/// Touch workspace (Window → Workspace → Touch; Preferences › Devices › Enable Touch Workspace).
pub const TOUCH: &str = "Touch";
/// The id of [`ESSENTIALS`].
const ESSENTIALS_ID: &str = "essentials";

/// A built-in workspace: its name, the panels it brings to the front of their groups, the number
/// of columns its Tools panel lays the tools out in, and its record.
pub struct Builtin {
    pub name: &'static str,
    pub fronts: &'static [&'static str],
    pub tools_steps: u16,
    pub record: WorkspaceRecord,
}

impl Builtin {
    /// The workspace with its Tools panel in two columns (Window › Toolbars › Double Column on).
    fn double_tools(mut self) -> Self {
        self.tools_steps = 2;
        self
    }
}

/// The id of a built-in workspace: its lower-case name with dashes.
fn builtin_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

/// Built-in workspaces, in menu order (Essentials first).
pub fn builtins() -> Vec<Builtin> {
    let w = |name: &'static str, fronts: &'static [&'static str], f: &dyn Fn(&mut WorkspaceRecord)| {
        let mut record = WorkspaceRecord::default();
        f(&mut record);
        Builtin { name, fronts, tools_steps: 1, record }
    };
    vec![
        w(ESSENTIALS, &[], &|_| {}),
        w("Essentials Classic", &["color"], &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
            x.task_bar = false;
        }),
        w("Automation", &["layers", "actions"], &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
        }),
        w("Layout", &["layers", "align"], &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
        }),
        w("Painting", &["brushes"], &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
        })
        .double_tools(),
        w("Printing and Proofing", &["info"], &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
        }),
        w("Tracing", &["layers"], &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
            x.task_bar = false;
        }),
        w("Typography", &["character"], &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
        }),
        w("Web", &["symbols"], &|x| {
            x.control_bar = true;
            x.rulers = true;
            x.toolbar_advanced = true;
        }),
        w(TOUCH, &["align"], &|x| {
            x.control_bar = true;
            x.toolbar_advanced = true;
            x.task_bar = true;
        }),
    ]
}

/// The built-ins as the framework's named workspaces.
fn builtin_set() -> Vec<NamedWorkspace<String>> {
    builtins()
        .into_iter()
        .map(|b| NamedWorkspace::new(builtin_id(b.name), b.name, crate::panel_docking::builtin_workspace(b.fronts, b.tools_steps)))
        .collect()
}

/// The chrome the set stores beside a layout, from `record`. Never read back: the record is the truth.
fn chrome_of(record: &WorkspaceRecord) -> WorkspaceChrome {
    let mut chrome = WorkspaceChrome::default();
    chrome.options_bar = record.control_bar;
    chrome.status_bar = record.status_bar;
    chrome
}

/// Preferences › Devices › Enable Touch Workspace (#394): on, a touch screen switches to the Touch
/// workspace; off, or when no touch screen is reported, the workspace before that switch is
/// restored if Touch was entered automatically. Each switch keeps the layout it leaves and brings
/// back the one it returns to as it was left, as Window › Workspace does.
pub fn sync_touch_workspace(app: &mut VectorcraftApp, touch_screen: bool) {
    let auto = app.session.prefs.touch_workspace && touch_screen;
    if auto {
        if !is_current(&app.ui, TOUCH) {
            if app.ui.touch_workspace_restore.is_none() {
                app.ui.touch_workspace_restore = Some(current_name(&app.ui).to_string());
            }
            // Touch is built in, so the switch finds it.
            let _ = switch(app, TOUCH);
        }
    } else if app.ui.touch_workspace_restore.is_some() && is_current(&app.ui, TOUCH) {
        let back = app.ui.touch_workspace_restore.take().filter(|n| find(&app.ui, n).is_some()).unwrap_or_else(|| ESSENTIALS.to_string());
        // `back` names a workspace that exists (or Essentials), so the switch finds it.
        let _ = switch(app, &back);
    } else if !auto {
        app.ui.touch_workspace_restore = None;
    }
}

/// The workspace `key` names, built-ins first.
fn resolve<'a>(set: &'a WorkspaceSet<String>, key: &str) -> Option<&'a NamedWorkspace<String>> {
    let key = key.trim().to_lowercase();
    set.builtin.iter().chain(&set.saved).find(|w| w.id.to_lowercase() == key || w.name.to_lowercase() == key)
}

/// The workspace `key` names.
pub fn find<'a>(ui: &'a UiState, key: &str) -> Option<&'a NamedWorkspace<String>> {
    resolve(&ui.workspaces.set, key)
}

/// Whether `key` names a built-in workspace.
pub fn is_builtin(ui: &UiState, key: &str) -> bool {
    let key = key.trim().to_lowercase();
    ui.workspaces.set.builtin.iter().any(|w| w.id.to_lowercase() == key || w.name.to_lowercase() == key)
}

/// Whether `key` names the current workspace.
pub fn is_current(ui: &UiState, key: &str) -> bool {
    find(ui, key).is_some_and(|w| w.id == ui.workspaces.set.current)
}

/// The current workspace's name.
pub fn current_name(ui: &UiState) -> &str {
    ui.workspaces.set.current().map_or(ESSENTIALS, |w| w.name.as_str())
}

/// Every workspace name: built-ins then user workspaces.
pub fn names(ui: &UiState) -> Vec<String> {
    ui.workspaces.set.builtin.iter().chain(&ui.workspaces.set.saved).map(|w| w.name.clone()).collect()
}

/// Snapshot the bar flags of `ui` as a workspace record.
pub fn record(ui: &UiState) -> WorkspaceRecord {
    WorkspaceRecord {
        control_bar: ui.control_bar,
        control_bar_bottom: ui.control_bar_bottom,
        control_bar_floating: ui.control_bar_floating,
        control_bar_float_pos: ui.control_bar_float_pos,
        control_bar_hidden_groups: ui.control_bar_hidden_groups.clone(),
        rulers: ui.view.rulers,
        toolbar_compact: ui.toolbar_compact,
        toolbar_advanced: ui.toolbar_advanced,
        task_bar: ui.task_bar,
        task_bar_hidden_actions: ui.task_bar_hidden_actions.clone(),
        status_bar: ui.status_bar,
        help_bar: ui.help_bar,
        docking_hidden_groups: ui.docking_hidden_groups.clone(),
        tools_last_used: ui.tools_last_used.clone(),
        toolbars: ui.custom_toolbars.clone(),
    }
}

/// Apply a record's flags (brightness, prefs, shortcuts and the panel layout untouched).
fn apply_record(ui: &mut UiState, w: &WorkspaceRecord) {
    ui.control_bar = w.control_bar;
    ui.control_bar_bottom = w.control_bar_bottom;
    ui.control_bar_floating = w.control_bar_floating;
    // A floating workspace floats the bar where it was saved (none: the first position); a docked
    // one without a floating position keeps the one the bar last floated at.
    if w.control_bar_floating || w.control_bar_float_pos.is_some() {
        ui.control_bar_float_pos = w.control_bar_float_pos;
        crate::state::sanitize_toolbar_pos(&mut ui.control_bar_float_pos);
    }
    ui.control_bar_hidden_groups = w.control_bar_hidden_groups.clone();
    ui.view.rulers = w.rulers;
    ui.toolbar_compact = w.toolbar_compact;
    ui.toolbar_advanced = w.toolbar_advanced;
    ui.task_bar = w.task_bar;
    ui.task_bar_hidden_actions = w.task_bar_hidden_actions.clone();
    ui.status_bar = w.status_bar;
    ui.help_bar = w.help_bar;
    ui.docking_hidden_groups = w.docking_hidden_groups.clone();
    ui.tools_last_used = w.tools_last_used.clone();
    ui.custom_toolbars = w.toolbars.clone();
    crate::custom_toolbars::sanitize(&mut ui.custom_toolbars);
    // The drawer edits a toolbar of the workspace left: it closes.
    if ui.drawer_toolbar.take().is_some() {
        ui.tools_drawer = false;
    }
}

/// The record `id` was saved with (a built-in's, or the user's).
fn saved_record(w: &Workspaces, id: &str) -> WorkspaceRecord {
    w.records.get(id).cloned().or_else(|| builtins().into_iter().find(|b| builtin_id(b.name) == id).map(|b| b.record)).unwrap_or_default()
}

/// The text the user reads for a refused name.
fn refusal(e: WorkspaceError, name: &str) -> String {
    match e {
        WorkspaceError::EmptyName => "workspace name can't be empty".into(),
        WorkspaceError::NameTaken => format!("`{name}` is a built-in workspace; choose another name"),
        WorkspaceError::CurrentCannotBeDeleted => "the current workspace can't be deleted".into(),
        WorkspaceError::UnknownId => format!("no workspace `{name}`"),
        _ => format!("`{name}` is not an available name"),
    }
}

/// Switch to workspace `key` as it was left (its changes since it was saved), else as saved.
pub fn switch(app: &mut VectorcraftApp, key: &str) -> Result<String, String> {
    let ui = &mut app.ui;
    let (id, name) =
        find(ui, key).map(|w| (w.id.clone(), w.name.clone())).ok_or_else(|| format!("unknown workspace (one of {})", names(ui).join(", ")))?;
    let current = ui.workspaces.set.current.clone();
    let left = record(ui);
    ui.workspaces.remember(&current, left);
    // Keeps the live layout under the old id and replaces it with the target's.
    ui.workspaces.set.select(&id, Arc::make_mut(&mut ui.dock_workspace)).map_err(|e| refusal(e, key))?;
    let w = ui.workspaces.edited.get(&id).cloned().unwrap_or_else(|| saved_record(&ui.workspaces, &id));
    apply_record(ui, &w);
    crate::panel_docking::reconcile(&mut ui.dock_workspace);
    crate::panel_docking::show_panels(app);
    Ok(name)
}

/// Reset the current workspace to the way it was saved, dropping its changes.
pub fn reset(app: &mut VectorcraftApp) -> String {
    let ui = &mut app.ui;
    let id = ui.workspaces.set.current.clone();
    // The current workspace is always one the set has (`sanitize`); an unknown one has nothing to reset.
    let _ = ui.workspaces.set.reset(Arc::make_mut(&mut ui.dock_workspace));
    ui.workspaces.forget(&id);
    let w = saved_record(&ui.workspaces, &id);
    // The toolbars made with New Toolbar stay in the workspace they were made in.
    let toolbars = std::mem::take(&mut ui.custom_toolbars);
    apply_record(ui, &w);
    ui.custom_toolbars = toolbars;
    crate::panel_docking::reconcile(&mut ui.dock_workspace);
    crate::panel_docking::show_panels(app);
    current_name(&app.ui).to_string()
}

/// Keep the toolbars made with New Toolbar in the current workspace's changes, and nothing else of
/// its layout that its changes do not already hold.
fn keep_toolbars(ui: &mut UiState) {
    let id = ui.workspaces.set.current.clone();
    let toolbars = ui.custom_toolbars.clone();
    let w = &mut ui.workspaces;
    if let Some(left) = w.edited.get_mut(&id) {
        left.toolbars = toolbars;
        return;
    }
    let saved = saved_record(w, &id);
    if saved.toolbars != toolbars {
        w.remember(&id, WorkspaceRecord { toolbars, ..saved });
    }
}

/// Save the current layout as `name` (replacing a user workspace of that name) and make it current.
pub fn save_as(ui: &mut UiState, name: &str) -> Result<(), String> {
    let (name, existing) =
        ui.workspaces.set.check_name(name).map(|(n, e)| (n.to_owned(), e.map(str::to_owned))).map_err(|e| refusal(e, name.trim()))?;
    // The workspace left keeps the toolbars made in it, which the one saved starts with a copy of.
    if existing.as_deref() != Some(ui.workspaces.set.current.as_str()) {
        keep_toolbars(ui);
    }
    let record = record(ui);
    let chrome = chrome_of(&record);
    let layout = DockWorkspace::clone(&ui.dock_workspace);
    let id = ui.workspaces.set.save(&name, layout, chrome, WorkspaceKeeps::NONE).map_err(|e| refusal(e, &name))?;
    ui.workspaces.forget(&id);
    ui.workspaces.records.insert(id, record);
    Ok(())
}

/// Delete the saved workspace `key`; built-ins and the current workspace stay.
pub fn delete(ui: &mut UiState, key: &str) -> Result<(), String> {
    let Some(id) = find(ui, key).map(|w| w.id.clone()) else { return Err(format!("no workspace `{key}`")) };
    if is_builtin(ui, &id) {
        return Err(format!("`{key}` is built in and can't be deleted"));
    }
    ui.workspaces.set.delete(&id).map_err(|e| refusal(e, key))?;
    ui.workspaces.records.remove(&id);
    ui.workspaces.forget(&id);
    Ok(())
}

/// Rename the saved workspace `from` to `to`; its id, changes and layout stay.
pub fn rename(ui: &mut UiState, from: &str, to: &str) -> Result<(), String> {
    let id = find(ui, from).filter(|w| !is_builtin(ui, &w.id)).map(|w| w.id.clone()).ok_or_else(|| format!("no user workspace `{from}`"))?;
    let unavailable = || format!("`{}` is not an available name", to.trim());
    let (to, existing) = ui.workspaces.set.check_name(to).map(|(n, e)| (n.to_owned(), e.map(str::to_owned))).map_err(|_| unavailable())?;
    if existing.is_some_and(|other| other != id) {
        return Err(unavailable());
    }
    if let Some(w) = ui.workspaces.set.saved.iter_mut().find(|w| w.id == id) {
        w.name = to;
    }
    Ok(())
}

// ---------- the menu ----------

/// Window → Workspace submenu items: the built-in workspaces, the saved ones (`saved`) and the
/// workspace commands.
pub fn menu_items(saved: &[String]) -> Vec<crate::menus::Node> {
    use crate::menus::{Node, Sep, c, named};
    let mut v: Vec<Node> = builtins().into_iter().map(|b| named(b.name, "window.workspace", json!({"name": b.name}))).collect();
    if !saved.is_empty() {
        v.push(Sep);
        v.extend(saved.iter().map(|n| named(n.as_str(), "window.workspace", json!({"name": n}))));
    }
    v.push(Sep);
    v.push(c("Reset Essentials", "window.workspace.reset"));
    v.push(c("New Workspace…", "window.workspace.new"));
    v.push(c("Manage Workspaces…", "window.workspace.manage"));
    v
}

/// The app bar's workspace switcher. Its list rows are the commands Reset, New Workspace… and
/// Manage Workspaces…; picking a workspace runs `window.workspace`.
pub fn switcher(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let rows = ["window.workspace.reset", "window.workspace.new", "window.workspace.manage"].map(CommandRef::new).to_vec();
    let shown = WorkspaceSwitcher::new(&app.ui.workspaces.set).commands(rows).look(SelectLook::Quiet).show(ui);
    if let Some(id) = shown.switched {
        crate::menus::invoke(app, "window.workspace", json!({"name": id}));
    }
}

// ---------- dialogs ----------

/// A built-in workspace in Manage Workspaces: listed, never renamed or deleted.
const BUILTIN: u32 = 0;
/// A user workspace in Manage Workspaces.
const SAVED: u32 = 1;
const NEW: &str = "new";
const DELETE: &str = "delete";
/// The key Manage Workspaces opens with, which names it on the dialog host.
const MANAGE: &str = "window.workspace.manage";

/// The rows of Manage Workspaces: built-ins (dim, fixed) then the user's.
fn manage_entries(ui: &UiState) -> Vec<ListManagerEntry> {
    let set = &ui.workspaces.set;
    let row = |w: &NamedWorkspace<String>, kind: u32| {
        ListManagerEntry::new(w.id.clone(), crate::panels::label_or_name(&w.name, kind == BUILTIN).to_string()).kind(kind)
    };
    set.builtin.iter().map(|w| row(w, BUILTIN)).chain(set.saved.iter().map(|w| row(w, SAVED))).collect()
}

/// Open Manage Workspaces; the dialog is left as it is when it is already open.
fn open_manage(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let actions = vec![
        ListManagerAction::new("ok", tl!("OK"), ActionEnabled::Always),
        ListManagerAction::new(NEW, tl!("New"), ActionEnabled::Always).keeps_open(),
        ListManagerAction::new(DELETE, tl!("Delete"), ActionEnabled::Always).kind(SAVED),
    ];
    let dialog = HostedListManager::new(MANAGE, tl!("Manage Workspaces"), tl!("Name:"), actions).entries(manage_entries(&app.ui)).renames(SAVED);
    dialogs.with(|host| {
        if host.get_by_id::<HostedListManager>(dialog_id(MANAGE)).is_none() {
            host.open(dialog);
        }
    });
}

/// Open New Workspace, offering a free "Workspace N"; it is left as it is when already open.
fn open_new(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let name = (1..).map(|i| format!("Workspace {i}")).find(|n| find(&app.ui, n).is_none()).unwrap_or_default();
    let mut value = NewWorkspace::default();
    value.name = name;
    let dialog = HostedNewWorkspace::new(&app.ui.workspaces.set).offered(WorkspaceKeeps::NONE).value(value);
    // One already open keeps its input.
    let _ = dialogs.with(|host| host.open_unless_open(dialog).ok());
}

/// Run what the workspace dialogs answered since the last pass, and keep Manage Workspaces' rows
/// current. Called once per frame by the dialog host's `pump`.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(answer) = dialogs.with(|host| host.take::<HostedNewWorkspace<String>>()).and_then(DialogResult::ok)
        && let Err(e) = save_as(&mut app.ui, &answer.name)
    {
        app.status(e);
    }
    let id = dialog_id(MANAGE);
    let entries = manage_entries(&app.ui);
    // A choice that kept Manage Workspaces open waits on it; the one that closed it is its answer.
    let choices = dialogs.with(|host| {
        let kept = host.get_mut_by_id::<HostedListManager>(id).and_then(|manager| {
            manager.set_entries(entries);
            manager.take_choice()
        });
        [kept, host.take_by_id::<HostedListManager>(id).and_then(DialogResult::ok)]
    });
    for choice in choices.into_iter().flatten() {
        if let Err(e) = run_choice(&mut app.ui, &choice) {
            app.status(e);
        }
    }
}

/// Does what a Manage Workspaces button asked: rename, save the layout as a new workspace, or delete.
fn run_choice(ui: &mut UiState, choice: &ListManagerChoice) -> Result<(), String> {
    let entry = choice.entry.as_deref().unwrap_or_default();
    if choice.is_rename() {
        return rename(ui, entry, &choice.name);
    }
    match choice.action.as_str() {
        NEW => save_as(ui, &choice.name),
        DELETE => delete(ui, entry),
        _ => Ok(()),
    }
}

// ---------- commands ----------

/// The saved workspaces as `window.workspace.list` reports them: the record's fields and the name.
fn listed(ui: &UiState) -> Vec<Value> {
    ui.workspaces
        .set
        .saved
        .iter()
        .map(|w| {
            let mut v = serde_json::to_value(saved_record(&ui.workspaces, &w.id)).unwrap_or(Value::Null);
            if let Some(fields) = v.as_object_mut() {
                fields.insert("name".into(), json!(w.name));
            }
            v
        })
        .collect()
}

pub fn run_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "window.workspace" => switch(app, &s("name").unwrap_or_default()).map(|name| json!({"workspace": name})),
        "window.workspace.reset" => Ok(json!({"workspace": reset(app)})),
        "window.workspace.new" => match s("name") {
            Some(n) => save_as(&mut app.ui, &n).map(|_| json!({"workspace": n})),
            None => {
                open_new(app);
                Ok(Value::Null)
            }
        },
        "window.workspace.manage" => {
            open_manage(app);
            Ok(Value::Null)
        }
        "window.workspace.delete" => delete(&mut app.ui, &s("name").unwrap_or_default()).map(|_| Value::Null),
        "window.workspace.rename" => rename(&mut app.ui, &s("name").unwrap_or_default(), &s("to").unwrap_or_default()).map(|_| Value::Null),
        "window.workspace.list" => Ok(json!({"current": current_name(&app.ui), "workspaces": names(&app.ui), "custom": listed(&app.ui)})),
        _ => return None,
    };
    Some(r)
}

#[cfg(test)]
mod tests {
    use craft::behavior::dock::{Action, Permissions};

    use super::*;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default())
    }

    /// Whether `panel` is the one its group shows.
    fn front(ui: &UiState, panel: &str) -> bool {
        let layout = crate::panel_docking::layout_of(ui);
        crate::panel_docking::column_of(layout, panel)
            .is_some_and(|(_, column)| column.groups.iter().any(|g| g.panels.get(g.active).is_some_and(|p| p == panel)))
    }

    fn id_of(ui: &UiState, name: &str) -> String {
        find(ui, name).unwrap().id.clone()
    }

    /// What a workspace applies and what it leaves alone: a switch does not touch UI brightness.
    #[test]
    fn apply_classic_then_reset_to_essentials_keeps_brightness() {
        let mut app = app();
        app.ui.brightness = crate::theme::Brightness::Light;
        switch(&mut app, "Essentials Classic").unwrap();
        assert!(app.ui.control_bar && app.ui.view.rulers && app.ui.toolbar_advanced);
        assert!(front(&app.ui, "color"));
        assert_eq!(current_name(&app.ui), "Essentials Classic");
        switch(&mut app, "essentials").unwrap();
        assert!(!app.ui.control_bar && !app.ui.view.rulers);
        assert_eq!(app.ui.brightness, crate::theme::Brightness::Light);
    }

    #[test]
    fn save_apply_round_trip() {
        let mut app = app();
        app.ui.control_bar = true;
        for panel in ["layers", "stroke"] {
            Arc::make_mut(&mut app.ui.dock_workspace).apply(Action::activate(panel.to_string()), false, |_: &String| Permissions::ALL).unwrap();
        }
        let layout = crate::panel_docking::layout_of(&app.ui).clone();
        save_as(&mut app.ui, "Mine").unwrap();
        switch(&mut app, ESSENTIALS).unwrap();
        assert!(!app.ui.control_bar);
        switch(&mut app, "Mine").unwrap();
        assert!(app.ui.control_bar);
        assert_eq!(crate::panel_docking::layout_of(&app.ui), &layout);
        // Survives the UiState JSON round trip.
        let back: UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        let back = back.sanitized();
        assert_eq!(back.workspaces, app.ui.workspaces);
        assert_eq!(current_name(&back), "Mine");
    }

    /// With 64 layouts remembered, the oldest goes and the one just left is kept.
    #[test]
    fn a_full_list_of_remembered_layouts_drops_the_oldest_not_the_newest() {
        let mut app = app();
        let names: Vec<String> = (0..70).map(|i| format!("W{i}")).collect();
        for name in &names {
            save_as(&mut app.ui, name).unwrap();
        }
        for name in &names {
            switch(&mut app, name).unwrap();
            app.ui.control_bar = true;
        }
        switch(&mut app, ESSENTIALS).unwrap();
        assert_eq!(app.ui.workspaces.edit_order.len(), 64);
        assert_eq!(app.ui.workspaces.set.edited.len(), 64);
        let oldest = id_of(&app.ui, "W0");
        assert!(!app.ui.workspaces.edited.contains_key(&oldest) && !app.ui.workspaces.set.edited.contains_key(&oldest), "the oldest went");
        switch(&mut app, "W69").unwrap();
        assert!(app.ui.control_bar, "the layout just left is kept");
    }

    #[test]
    fn all_requested_workspaces_exist() {
        let ui = UiState::default();
        for n in ["Essentials", "Essentials Classic", "Painting", "Typography", "Layout", "Tracing", "Web", "Automation", TOUCH] {
            assert!(find(&ui, n).is_some(), "{n}");
        }
    }

    /// Painting lays the Tools panel out in two columns and Window › Toolbars › Double Column reads
    /// checked; every other built-in workspace has one column. Switching back to Essentials brings
    /// back its own column count, as it was left, and Reset Painting gives two columns again.
    #[test]
    fn painting_shows_tools_in_two_columns_and_essentials_keeps_its_own() {
        let steps = |app: &VectorcraftApp| {
            crate::panel_docking::column_of(crate::panel_docking::layout_of(&app.ui), crate::toolbar::TOOLS).and_then(|(_, column)| column.steps)
        };
        let double = |app: &VectorcraftApp| crate::menus::checked(app, "window.toolbarColumns", &json!({}));
        let mut app = app();
        assert_eq!((steps(&app), double(&app)), (Some(1), Some(false)), "Essentials starts with one column");
        for b in builtins() {
            switch(&mut app, b.name).unwrap();
            let painting = b.name == "Painting";
            assert_eq!(steps(&app), Some(if painting { 2 } else { 1 }), "{}", b.name);
            assert_eq!(double(&app), Some(painting), "Double Column in {}", b.name);
        }
        switch(&mut app, ESSENTIALS).unwrap();
        assert_eq!((steps(&app), double(&app)), (Some(1), Some(false)), "Essentials has one column again");
        app.run("window.toolbarColumns", json!({"double": true})).unwrap();
        switch(&mut app, "Painting").unwrap();
        app.run("window.toolbarColumns", json!({"double": false})).unwrap();
        switch(&mut app, ESSENTIALS).unwrap();
        assert_eq!(double(&app), Some(true), "Essentials keeps the two columns it was left with");
        switch(&mut app, "Painting").unwrap();
        assert_eq!(double(&app), Some(false), "Painting keeps the one column it was left with");
        reset(&mut app);
        assert_eq!((steps(&app), double(&app)), (Some(2), Some(true)), "Reset Painting gives two columns again");
    }

    /// A file saved before `WorkspaceSet` names workspaces in any case: the layout remembered under
    /// another spelling belongs to the saved workspace, once; a name no workspace has is dropped,
    /// and a saved name equal to a built-in's is refused and reported.
    #[test]
    fn legacy_remembered_layouts_map_to_their_workspace_in_any_case_once() {
        let file = json!({
            "workspace": "mine",
            "custom_workspaces": [{"name": "Mine"}, {"name": "painting"}],
            "workspace_changes": [{"name": "MINE", "controlBar": true}, {"name": "mine", "controlBar": false}, {"name": "Gone"}],
        });
        let (w, outcome) = Workspaces::from_legacy(&file, &[]);
        assert!(!outcome.intact, "the built-in's name was refused");
        assert_eq!(w.set.saved.len(), 1);
        let id = w.set.saved[0].id.clone();
        assert_eq!(w.edit_order, std::slice::from_ref(&id));
        assert!(w.edited[&id].control_bar, "the first spelling's layout is the one kept");
        assert_eq!(w.set.current, id);
        assert!(w.records.contains_key(&id));
    }

    #[test]
    fn touch_workspace_preference_switches_and_restores() {
        let mut app = app();
        switch(&mut app, "Typography").unwrap();
        app.session.prefs.touch_workspace = true;
        sync_touch_workspace(&mut app, true);
        assert!(is_current(&app.ui, TOUCH));
        assert_eq!(app.ui.touch_workspace_restore.as_deref(), Some("Typography"));
        assert!(app.ui.control_bar && front(&app.ui, "align"));
        app.session.prefs.touch_workspace = false;
        sync_touch_workspace(&mut app, true);
        assert!(is_current(&app.ui, "Typography"));
        assert!(app.ui.touch_workspace_restore.is_none());
    }

    /// A workspace keeps the edge the Control bar is docked to, and one saved without it docks the
    /// bar at the top.
    #[test]
    fn saved_workspaces_restore_the_control_bar_edge() {
        let mut app = app();
        app.ui.control_bar = true;
        app.ui.control_bar_bottom = true;
        save_as(&mut app.ui, "Bottom").unwrap();
        switch(&mut app, ESSENTIALS).unwrap();
        assert!(!app.ui.control_bar_bottom, "Essentials docks the bar at the top");
        switch(&mut app, "Bottom").unwrap();
        assert!(app.ui.control_bar_bottom, "the saved workspace docks it at the bottom");
        let id = id_of(&app.ui, "Bottom");
        let back: UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(back.control_bar_bottom && back.workspaces.records[&id].control_bar_bottom, "both survive the preferences' JSON");
        let mut old = serde_json::to_value(&back.workspaces.records[&id]).unwrap();
        assert!(old.as_object_mut().unwrap().remove("controlBarBottom").is_some());
        let old: WorkspaceRecord = serde_json::from_value(old).unwrap();
        apply_record(&mut app.ui, &old);
        assert!(app.ui.control_bar && !app.ui.control_bar_bottom, "an older workspace docks the bar at the top");
    }

    /// A workspace keeps a floating Control bar and where it floats, through the preferences' JSON
    /// too; switching to a docked workspace docks it and keeps that position for the next float.
    /// Reset floats the bar where the workspace was saved floating. A workspace or preferences saved
    /// before floating keep their top or bottom edge, and a position that is not a number is
    /// dropped.
    #[test]
    fn saved_workspaces_restore_floating_control_bar_position() {
        let mut app = app();
        app.ui.control_bar = true;
        app.ui.control_bar_floating = true;
        app.ui.control_bar_float_pos = Some([220.0, 140.0]);
        save_as(&mut app.ui, "Floating").unwrap();
        switch(&mut app, ESSENTIALS).unwrap();
        assert!(!app.ui.control_bar_floating && !app.ui.control_bar_bottom, "Essentials docks the bar at the top");
        assert_eq!(app.ui.control_bar_float_pos, Some([220.0, 140.0]), "the last floating position is kept while docked");
        switch(&mut app, "Floating").unwrap();
        assert!(app.ui.control_bar_floating, "the saved workspace floats the bar again");
        assert_eq!(app.ui.control_bar_float_pos, Some([220.0, 140.0]));
        let id = id_of(&app.ui, "Floating");
        let back: UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(back.control_bar_floating && back.workspaces.records[&id].control_bar_floating, "both survive the preferences' JSON");
        assert_eq!(back.workspaces.records[&id].control_bar_float_pos, Some([220.0, 140.0]));
        // Saved before the bar floated: the edge it was docked to, top or bottom.
        for bottom in [false, true] {
            let old: WorkspaceRecord = serde_json::from_value(json!({"controlBar": true, "controlBarBottom": bottom})).unwrap();
            apply_record(&mut app.ui, &old);
            assert!(
                !app.ui.control_bar_floating && app.ui.control_bar_bottom == bottom,
                "an older workspace docks the bar at its edge, bottom: {bottom}"
            );
            let old: UiState = serde_json::from_value(json!({"control_bar": true, "control_bar_bottom": bottom})).unwrap();
            assert!(!old.control_bar_floating && old.control_bar_bottom == bottom, "older preferences dock it at their edge, bottom: {bottom}");
        }
        // Saved floating at the first position: Reset after a move floats it there again.
        let mut app = self::app();
        app.ui.control_bar = true;
        app.ui.control_bar_floating = true;
        save_as(&mut app.ui, "First").unwrap();
        app.ui.control_bar_float_pos = Some([300.0, 90.0]);
        reset(&mut app);
        assert!(app.ui.control_bar_floating, "Reset floats the bar");
        assert_eq!(app.ui.control_bar_float_pos, None, "at the first position, as the workspace was saved");
        let odd: UiState = serde_json::from_value(json!({"control_bar_floating": true, "control_bar_float_pos": [1e39, 5.0]})).unwrap();
        assert_eq!(odd.sanitized().control_bar_float_pos, None, "a position out of range is dropped");
    }

    /// A workspace keeps the groups of Control bar options its panel menu has off: switching
    /// brings each workspace's back, Reset turns a built-in workspace's on again, and they survive
    /// the preferences' JSON. A workspace or preferences saved without them, or with groups the bar
    /// does not have, show every group, or the ones they name that it has.
    #[test]
    fn saved_workspaces_restore_the_control_bar_groups() {
        let hidden = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>();
        let mut app = app();
        app.ui.control_bar = true;
        app.ui.control_bar_hidden_groups = hidden(&["opacity", "transform"]);
        save_as(&mut app.ui, "Lean").unwrap();
        switch(&mut app, ESSENTIALS).unwrap();
        assert!(app.ui.control_bar_hidden_groups.is_empty(), "Essentials shows every group");
        app.ui.control_bar_hidden_groups = hidden(&["snapping"]);
        switch(&mut app, "Lean").unwrap();
        assert_eq!(app.ui.control_bar_hidden_groups, hidden(&["opacity", "transform"]), "the saved workspace's groups");
        switch(&mut app, ESSENTIALS).unwrap();
        assert_eq!(app.ui.control_bar_hidden_groups, hidden(&["snapping"]), "Essentials as it was left");
        reset(&mut app);
        assert!(app.ui.control_bar_hidden_groups.is_empty(), "Reset Essentials shows every group again");
        switch(&mut app, "Lean").unwrap();
        let id = id_of(&app.ui, "Lean");
        let json = serde_json::to_value(&app.ui).unwrap();
        assert_eq!(json["workspaces"]["records"][id.as_str()]["controlBarHiddenGroups"], json!(["opacity", "transform"]));
        let back: UiState = serde_json::from_value(json).unwrap();
        assert_eq!(back.control_bar_hidden_groups, hidden(&["opacity", "transform"]), "kept in the preferences");
        assert_eq!(back.workspaces.records[&id].control_bar_hidden_groups, hidden(&["opacity", "transform"]), "kept with the workspace");
        let mut old = serde_json::to_value(&back.workspaces.records[&id]).unwrap();
        assert!(old.as_object_mut().unwrap().remove("controlBarHiddenGroups").is_some());
        let old: WorkspaceRecord = serde_json::from_value(old).unwrap();
        apply_record(&mut app.ui, &old);
        assert!(app.ui.control_bar_hidden_groups.is_empty(), "an older workspace shows every group");
        let mut prefs = serde_json::to_value(&back).unwrap();
        prefs["control_bar_hidden_groups"] = json!(["opacity", "stroke", 3, "paragraph"]);
        prefs["workspaces"]["records"][id.as_str()]["controlBarHiddenGroups"] = json!("opacity");
        let read: UiState = serde_json::from_value(prefs.clone()).unwrap();
        assert_eq!(read.control_bar_hidden_groups, hidden(&["opacity", "paragraph"]), "groups the bar does not have are dropped");
        assert!(read.workspaces.records[&id].control_bar_hidden_groups.is_empty(), "a value that is not a list shows every group");
        prefs.as_object_mut().unwrap().remove("control_bar_hidden_groups");
        let read: UiState = serde_json::from_value(prefs).unwrap();
        assert!(read.control_bar_hidden_groups.is_empty(), "older preferences show every group");
    }

    /// A workspace keeps the Contextual Task Bar's actions its Bar Actions has off: switching
    /// brings each workspace's back, Reset turns a built-in workspace's on again, and they survive
    /// the preferences' JSON. A workspace saved without them, or with ids the bar does not have,
    /// shows every action, or the ones it names that the bar has.
    #[test]
    fn saved_workspaces_restore_the_task_bar_actions() {
        let hidden = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>();
        let mut app = app();
        app.ui.task_bar_hidden_actions = hidden(&["duplicate", "fill"]);
        save_as(&mut app.ui, "Lean").unwrap();
        switch(&mut app, ESSENTIALS).unwrap();
        assert!(app.ui.task_bar_hidden_actions.is_empty(), "Essentials shows every action");
        app.ui.task_bar_hidden_actions = hidden(&["simplify"]);
        switch(&mut app, "Lean").unwrap();
        assert_eq!(app.ui.task_bar_hidden_actions, hidden(&["duplicate", "fill"]), "the saved workspace's actions");
        switch(&mut app, ESSENTIALS).unwrap();
        assert_eq!(app.ui.task_bar_hidden_actions, hidden(&["simplify"]), "Essentials as it was left");
        reset(&mut app);
        assert!(app.ui.task_bar_hidden_actions.is_empty(), "Reset Essentials shows every action again");
        switch(&mut app, "Lean").unwrap();
        let id = id_of(&app.ui, "Lean");
        let json = serde_json::to_value(&app.ui).unwrap();
        assert_eq!(json["workspaces"]["records"][id.as_str()]["taskBarHiddenActions"], json!(["duplicate", "fill"]));
        let back: UiState = serde_json::from_value(json).unwrap();
        assert_eq!(back.task_bar_hidden_actions, hidden(&["duplicate", "fill"]), "kept in the preferences");
        assert_eq!(back.workspaces.records[&id].task_bar_hidden_actions, hidden(&["duplicate", "fill"]), "kept with the workspace");
        let mut old = serde_json::to_value(&back.workspaces.records[&id]).unwrap();
        assert!(old.as_object_mut().unwrap().remove("taskBarHiddenActions").is_some());
        let old: WorkspaceRecord = serde_json::from_value(old).unwrap();
        apply_record(&mut app.ui, &old);
        assert!(app.ui.task_bar_hidden_actions.is_empty(), "an older workspace shows every action");
        let mut prefs = serde_json::to_value(&back).unwrap();
        prefs["workspaces"]["records"][id.as_str()]["taskBarHiddenActions"] = json!(["fill", "teleport", 3]);
        let read: UiState = serde_json::from_value(prefs.clone()).unwrap();
        assert_eq!(read.workspaces.records[&id].task_bar_hidden_actions, hidden(&["fill"]), "ids the bar does not have are dropped");
        prefs["workspaces"]["records"][id.as_str()]["taskBarHiddenActions"] = json!("fill");
        let read: UiState = serde_json::from_value(prefs).unwrap();
        assert!(read.workspaces.records[&id].task_bar_hidden_actions.is_empty(), "a value that is not a list shows every action");
    }
}
