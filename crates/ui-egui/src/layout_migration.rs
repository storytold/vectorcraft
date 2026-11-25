//! Saved preferences at the file boundary: docking data this version cannot read is dropped
//! without resetting anything else, and layouts saved in an older shape become one
//! `Workspace<String>`.
//!
//! Older files kept the dock as a tree of the right dock's panels only (`docking`, or the
//! tabbed Properties · Layers · Libraries group when there was none) and described the rest in
//! fields of their own: `floating_panels` (floating groups and sets, collapsed groups, sets
//! docked as columns), `docking_icons` (panels dropped on the icon column), `docking_collapsed`
//! (dock groups collapsed to their tabs), `dock_collapsed` and `dock_width`. The Tools panel,
//! the custom toolbars and their torn-off strips lived outside the layout, in flags and lists of
//! their own. [`from_legacy`] turns all of them into one workspace; the legacy fields are read,
//! never written.

use std::collections::{BTreeMap, BTreeSet};

use craft::behavior::dock::{
    Column, ColumnLocation, DockEdge, DockPlacement, Floating, Group, GroupLocation, Layout, Location, MAX_DEPTH, MAX_PANELS, Node, SplitSize,
    Workspace,
};
use craft::behavior::flow::Axis;
use std::sync::Arc;
// PLAN: assumed path of the rail's slot memory (ui-panels/src/rails/tool/data.rs).
use craft::ui::rails::{LastUsed, SlotId};
use egui::{Rect, pos2, vec2};
use serde_json::{Map, Value};

use crate::panel_docking::{self, CANVAS};
use crate::state::{CustomToolbar, DockTab, FloatingFlyout, FloatingPanels, ICON_PANEL_GROUPS, ICON_PANELS, LegacyDock, UiState};
use crate::toolbar::{self, Owner};
use crate::workspaces::Workspaces;

/// Saved preferences, and whether reading them discarded anything.
pub struct ReadPreferences {
    pub ui: UiState,
    /// False when unreadable docking data, or a layout that could not be converted, was dropped:
    /// the caller keeps a copy of the file before saving over it.
    pub intact: bool,
    /// True when a layout saved in an older shape was converted: the caller keeps a copy of the
    /// file as it was.
    pub converted: bool,
}

/// Floating groups read from one legacy list; more are dropped.
const MAX_LEGACY_GROUPS: usize = 256;
/// Size of a floating window that saved none (older files saved only its position).
const FLOAT_WIDTH: f32 = 300.0;
const FLOAT_HEIGHT: f32 = 400.0;
const FLOAT_MIN: f32 = 200.0;
/// The tallest a converted floating set starts.
const STACK_MAX: f32 = 720.0;
/// Width of the Properties · Layers · Libraries column until the user resizes it.
const COLUMN_WIDTH: f32 = 310.0;
/// The widths the main column may be given (`dock_width`).
const COLUMN_MIN: f32 = 220.0;
const COLUMN_MAX: f32 = 720.0;
/// Width of a column collapsed to icons.
const ICON_WIDTH: f32 = 38.0;

impl UiState {
    /// Read saved preferences. None when the file is not a preferences file at all; unreadable
    /// docking data alone does not reset the other settings or the saved workspaces.
    pub fn read_preferences(bytes: &[u8]) -> Option<ReadPreferences> {
        let mut saved: Value = serde_json::from_slice(bytes).ok()?;
        let fields = saved.as_object_mut()?;
        help_bar_visibility(fields, "status_bar", "help_bar");
        // Before the shared dock, `dock` was whether Tab had hidden the panels; the key now holds the arrangement, which
        // the legacy fields below convert.
        if fields.get("dock").is_some_and(|dock| !dock.is_object()) {
            fields.remove("dock");
        }
        let mut intact = recover(fields, Keys::SNAKE);
        for key in ["custom_workspaces", "workspace_changes"] {
            if let Some(Value::Array(workspaces)) = fields.get_mut(key) {
                for workspace in workspaces.iter_mut().filter_map(Value::as_object_mut) {
                    help_bar_visibility(workspace, "statusBar", "helpBar");
                    intact &= recover(workspace, Keys::CAMEL);
                }
            }
        }
        // Only this function holds the raw map the Color panel's four old keys are read from.
        if !fields.contains_key("color_panel")
            && let Ok(panel) = serde_json::to_value(crate::panels::color::legacy_panel_state(fields))
        {
            fields.insert("color_panel".into(), panel);
        }
        let order: Vec<DockTab> = fields.get("dock_tab_order").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let mut records = Map::new();
        for key in ["workspace", "custom_workspaces", "workspace_changes"] {
            if let Some(value) = fields.remove(key) {
                records.insert(key.into(), value);
            }
        }
        let mut outcome = Outcome { intact, converted: false };
        if !records.is_empty() {
            // PLAN: assumed `Workspaces::from_legacy(&Value, &[DockTab]) -> (Workspaces, Outcome)` (chrome-w14, ch.P12).
            let (workspaces, converted) = Workspaces::from_legacy(&Value::Object(records), &order);
            outcome = outcome.and(converted);
            fields.insert("workspaces".into(), serde_json::to_value(&workspaces).ok()?);
        }
        let mut ui: UiState = serde_json::from_value(saved).ok()?;
        let outcome = outcome.and(migrate_ui(&mut ui));
        Some(ReadPreferences { ui, intact: outcome.intact, converted: outcome.converted })
    }
}

/// Older layouts used status-bar visibility for both bottom bars. Preserve that choice until
/// the layout has an independent Help Bar setting.
fn help_bar_visibility(fields: &mut Map<String, Value>, status: &str, help: &str) {
    if !fields.contains_key(help) {
        let visible = fields.get(status).and_then(Value::as_bool).unwrap_or(true);
        fields.insert(help.into(), Value::Bool(visible));
    }
}

/// The JSON keys of the docking fields: `UiState` is snake_case, a workspace record camelCase.
struct Keys {
    hidden: &'static str,
    floating: &'static str,
    icons: &'static str,
    collapsed: &'static str,
}

impl Keys {
    const SNAKE: Keys = Keys { hidden: "docking_hidden", floating: "floating_panels", icons: "docking_icons", collapsed: "docking_collapsed" };
    const CAMEL: Keys = Keys { hidden: "dockingHidden", floating: "floatingPanels", icons: "dockingIcons", collapsed: "dockingCollapsed" };
}

/// Drop the docking data of one saved state that this version cannot read: the layout, each
/// hidden place and floating group that fails, and legacy panel sets of another type. True
/// when nothing was dropped.
fn recover(fields: &mut Map<String, Value>, keys: Keys) -> bool {
    fn readable<T: serde::de::DeserializeOwned>(value: &Value) -> bool {
        serde_json::from_value::<T>(value.clone()).is_ok()
    }
    let mut intact = true;
    if fields.get("docking").is_some_and(|v| !readable::<Option<Layout<String>>>(v)) {
        fields.remove("docking");
        intact = false;
    }
    match fields.get_mut(keys.hidden) {
        Some(Value::Object(entries)) => {
            let before = entries.len();
            entries.retain(|_, location| readable::<Location<String>>(location));
            intact &= entries.len() == before;
        }
        Some(Value::Null) => {
            fields.remove(keys.hidden);
        }
        None => {}
        Some(_) => {
            fields.remove(keys.hidden);
            intact = false;
        }
    }
    match fields.get_mut(keys.floating) {
        Some(Value::Array(groups)) => {
            let before = groups.len();
            groups.retain(readable::<FloatingPanels>);
            intact &= groups.len() == before;
        }
        Some(Value::Null) => {
            fields.remove(keys.floating);
        }
        None => {}
        Some(_) => {
            fields.remove(keys.floating);
            intact = false;
        }
    }
    for key in [keys.icons, keys.collapsed] {
        if fields.get(key).is_some_and(|v| !readable::<BTreeSet<String>>(v)) {
            fields.remove(key);
            intact = false;
        }
    }
    intact
}

/// Closed panels with the place each goes back to.
type Hidden = BTreeMap<String, Location<String>>;

/// What converting saved layouts did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Outcome {
    /// False when a layout was dropped.
    pub intact: bool,
    /// True when a legacy layout was converted (or dropped trying).
    pub converted: bool,
}

impl Outcome {
    pub(crate) fn and(self, other: Outcome) -> Outcome {
        Outcome { intact: self.intact && other.intact, converted: self.converted || other.converted }
    }
}

/// Convert the legacy docking fields of `ui` into its workspace, its Tools slot memory and its
/// custom toolbars' ids. The legacy fields are taken, so a second call finds nothing to do.
pub(crate) fn migrate_ui(ui: &mut UiState) -> Outcome {
    let legacy = std::mem::take(&mut ui.legacy);
    if !legacy.slot_tool.is_empty() {
        ui.tools_last_used = slot_memory(&legacy.slot_tool);
    }
    let (workspace, outcome) = migrate(&legacy, &mut ui.custom_toolbars);
    if let Some(workspace) = workspace {
        Arc::make_mut(&mut ui.dock_workspace).replace(workspace);
        panel_docking::reconcile(&mut ui.dock_workspace);
    }
    outcome
}

/// The old docking fields of one saved record (a workspace, or the changes kept for one) as a
/// workspace. `order` is the file's `dock_tab_order`: records keep none of their own. A record
/// with nothing to convert, or one that could not be converted, gets the default layout.
pub(crate) fn legacy_layout(record: &Value, order: &[DockTab]) -> (Workspace<String>, Outcome) {
    // PLAN: the contract row for ch.P3 lacks `order` and the outcome; the sheet's Writer note 9 adds both.
    let (mut legacy, readable) = match serde_json::from_value::<LegacyDock>(record.clone()) {
        Ok(legacy) => (legacy, true),
        Err(_) => (LegacyDock::default(), false),
    };
    legacy.dock_tab_order = order.to_vec();
    let mut toolbars: Vec<CustomToolbar> = record.get("toolbars").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let (workspace, outcome) = migrate(&legacy, &mut toolbars);
    // Workspaces::from_legacy (chrome-w14) stores the record's toolbars; it assigns the same ids and,
    // for each toolbar it converts, the memory `slot_memory` reads from `slot_tool`.
    (workspace.unwrap_or_else(|| panel_docking::default_workspace(&[])), Outcome { intact: outcome.intact && readable, ..outcome })
}

/// Give each toolbar without an id, or repeating an earlier one, the next number above the
/// largest id of its list, in list order.
pub(crate) fn assign_toolbar_ids(toolbars: &mut [CustomToolbar]) {
    let mut next = toolbars.iter().map(|t| t.id).max().unwrap_or(0);
    let mut seen = BTreeSet::new();
    for toolbar in toolbars.iter_mut() {
        if toolbar.id == 0 || !seen.insert(toolbar.id) {
            next = next.saturating_add(1);
            toolbar.id = next;
            seen.insert(next);
        }
    }
}

/// Does `f` hold anything the current shape keeps elsewhere? Flags at their defaults describe
/// the default layout and need no conversion.
fn has_data(f: &LegacyDock) -> bool {
    f.docking.is_some()
        || !f.floating_panels.is_empty()
        || !f.docking_icons.is_empty()
        || !f.docking_collapsed.is_empty()
        || f.dock_tab != DockTab::default()
        || f.dock_collapsed
        || f.dock_width.is_some()
        || f.toolbar_pos.is_some()
        || f.toolbar_right
        || f.tools_beside_canvas
        || f.toolbar_double
        || !f.floating_flyouts.is_empty()
}

/// Convert one saved state's legacy fields. None when there is nothing to convert: the state
/// keeps the layout it has. A conversion that fails falls back to the default layout and is not
/// intact. Toolbars without an id are legacy ones: they get ids and slot memory and are placed.
pub(crate) fn migrate(legacy: &LegacyDock, toolbars: &mut [CustomToolbar]) -> (Option<Workspace<String>>, Outcome) {
    let fresh = toolbars.iter().any(|t| t.id == 0);
    assign_toolbar_ids(toolbars);
    if fresh {
        for toolbar in toolbars.iter_mut() {
            toolbar.last_used = slot_memory(&toolbar.slot_tool);
        }
    }
    if !fresh && !has_data(legacy) {
        return (None, Outcome { intact: true, converted: false });
    }
    let placing: &[CustomToolbar] = if fresh { toolbars } else { &[] };
    let built = from_legacy(legacy, placing);
    let outcome = Outcome { intact: built.is_some(), converted: true };
    (Some(built.unwrap_or_else(|| panel_docking::default_workspace(&[]))), outcome)
}

/// The workspace older versions described with `f` and `toolbars`. The dock, from the window
/// edge inwards: the old dock tree's columns (its groups stacked top to bottom; a horizontal
/// split puts its parts side by side), the icon column, then the floating sets docked as columns;
/// a layout that already holds the canvas is kept as it is. The Tools panel and the toolbars are
/// then placed. Old split ratios are not kept. None when the result is not a valid workspace.
pub(crate) fn from_legacy(f: &LegacyDock, toolbars: &[CustomToolbar]) -> Option<Workspace<String>> {
    let (mut layout, hidden) = match f.docking.as_ref().filter(|l| has_canvas(l)) {
        Some(current) => (current.clone(), f.docking_hidden.clone()),
        None => pre_canvas(f),
    };
    let mut closed = add_tools(&mut layout, f, toolbars);
    let mut floated = Vec::new();
    // A hidden place applies to a closed panel, and to a floating one Dock Panel sends back
    // there; a floating panel whose way back was the icon column finds its icon group again.
    for (id, location) in hidden.into_iter().take(MAX_PANELS) {
        if layout.floating.iter().any(|w| w.contains(&id)) {
            if !f.docking_icons.contains(&id) {
                floated.push((id, location));
            }
        } else if !layout.contains(&id) {
            closed.push((id, location));
        }
    }
    // Older versions closed a panel by leaving it out of the layout: a default panel left out
    // stays closed, at its default place, instead of coming back when the workspace is fitted.
    let default = panel_docking::default_workspace(&[]);
    for id in default.layout().panels() {
        if closed.len() < MAX_PANELS
            && !layout.contains(id)
            && !closed.iter().any(|(p, _)| p == id)
            && let Ok(location) = default.layout().location(id)
        {
            closed.push((id.clone(), location));
        }
    }
    let workspace = Workspace::from_parts(layout, closed, floated);
    workspace.validate().ok()?;
    Some(workspace)
}

/// The layout of a file from before the canvas joined the panel layout, and its hidden places.
fn pre_canvas(f: &LegacyDock) -> (Layout<String>, Hidden) {
    let old = f.docking.as_ref();
    let mut placed = Placed::default();
    let groups: Vec<&FloatingPanels> = f.floating_panels.iter().take(MAX_LEGACY_GROUPS).collect();

    // Floating groups and sets first: the old dock tree never held a floating panel.
    let mut floating = Vec::new();
    let mut docked: Vec<(u32, Column<String>)> = Vec::new();
    let mut sets: Vec<u32> = Vec::new();
    for (k, g) in groups.iter().enumerate() {
        if let Some(set) = g.column {
            if sets.contains(&set) {
                continue;
            }
            sets.push(set);
        }
        let members: Vec<&FloatingPanels> = match g.column {
            Some(set) => groups.iter().skip(k).filter(|o| o.column == Some(set)).copied().collect(),
            None => vec![*g],
        };
        let kept: Vec<(Group<String>, f32)> = members
            .iter()
            .filter_map(|m| {
                let mut group = take_group(&mut placed, &m.panels, m.active)?;
                group.minimized = m.collapsed;
                let height = old_height(old, &group.panels);
                Some((group, height))
            })
            .collect();
        if kept.is_empty() {
            continue;
        }
        let width = g.width.filter(|w| w.is_finite()).map_or(FLOAT_WIDTH, |w| w.clamp(FLOAT_MIN, COLUMN_MAX));
        match g.docked {
            Some(n) => {
                let groups = kept.into_iter().map(|(group, _)| group);
                match docked.iter_mut().find(|(m, _)| *m == n) {
                    Some((_, column)) => column.groups.extend(groups),
                    None => docked.push((n, column_of(groups.collect(), width, false))),
                }
            }
            None => {
                let at = finite_pos(g.pos);
                // A floating window's groups have no minimized state of their own: a set is
                // minimized when all its groups were collapsed, else every group shows. A set
                // keeps its groups' heights.
                let all_minimized = kept.iter().all(|(group, _)| group.minimized);
                let height = kept.iter().map(|(_, h)| *h).sum::<f32>().min(STACK_MAX);
                let sized = kept.len() > 1;
                let groups = kept
                    .into_iter()
                    .map(|(mut group, height)| {
                        group.minimized = false;
                        group.height = sized.then_some(height);
                        group
                    })
                    .collect();
                let rect = Rect::from_min_size(pos2(at[0], at[1]), vec2(width, height));
                if let Some(mut window) = Floating::stack(floating.len() as u64, groups, rect) {
                    window.column.iconic = all_minimized;
                    floating.push(window);
                }
            }
        }
    }
    // Windows of the old layout that the floating groups did not list.
    for window in old.map(|l| l.floating.as_slice()).unwrap_or_default() {
        let groups: Vec<Group<String>> = window
            .groups()
            .iter()
            .take(MAX_PANELS)
            .filter_map(|g| {
                let mut kept = take_group(&mut placed, &g.panels, g.active)?;
                kept.minimized = g.minimized;
                kept.height = g.height;
                Some(kept)
            })
            .collect();
        if let Some(mut converted) = Floating::stack(floating.len() as u64, groups, usable_rect(window.rect)) {
            converted.column.iconic = window.column.iconic;
            floating.push(converted);
        }
    }

    // The dock tree, or the tabbed group of a file from before there was one.
    let tree_columns: Vec<Vec<Group<String>>> = match old {
        Some(layout) => {
            let mut reading = layout.root.as_ref().map(|root| tree_columns(root, 0, &mut placed)).unwrap_or_default();
            reading.reverse();
            reading
        }
        None => {
            let tabs: Vec<&str> = main_tabs(&f.dock_tab_order).into_iter().map(|t| t.info().0).filter(|id| !f.docking_icons.contains(*id)).collect();
            let active = tabs.iter().position(|id| *id == f.dock_tab.info().0).unwrap_or(0);
            let tabs: Vec<String> = tabs.into_iter().map(String::from).collect();
            take_group(&mut placed, &tabs, active).map(|g| vec![vec![g]]).unwrap_or_default()
        }
    };
    let n = tree_columns.len().max(1) as f32;
    let main = main_width(f.dock_width);
    let tree_width = (main / n).max(FLOAT_MIN.min(main));
    let mut columns: Vec<Column<String>> = tree_columns
        .into_iter()
        .map(|mut groups| {
            for g in &mut groups {
                g.minimized |= g.panels.get(g.active).is_some_and(|p| f.docking_collapsed.contains(p));
            }
            column_of(groups, tree_width, f.dock_collapsed)
        })
        .collect();

    // Panels closed with a remembered place stay closed, except those the icon column showed
    // anyway (its rows, and the panels dropped on it): they take their place there.
    let in_rows = |id: &str| ICON_PANEL_GROUPS.iter().any(|row| row.contains(&id));
    let hidden: Hidden = f.docking_hidden.iter().take(MAX_PANELS).map(|(id, location)| (id.clone(), location.clone())).collect();
    for id in hidden.keys().filter(|id| !in_rows(id) && !f.docking_icons.contains(*id)) {
        let _ = placed.take(id);
    }

    // The icon column: the tabbed group's panels dropped on it, its rows of icon panels, then
    // the other panels dropped on it.
    let mut icon_groups = Vec::new();
    let tabs: Vec<String> = DockTab::ALL.iter().map(|t| t.info().0.to_string()).filter(|id| f.docking_icons.contains(id)).collect();
    icon_groups.extend(take_group(&mut placed, &tabs, 0));
    for row in ICON_PANEL_GROUPS {
        let row: Vec<String> = row.iter().map(|id| id.to_string()).collect();
        icon_groups.extend(take_group(&mut placed, &row, 0));
    }
    let dropped: Vec<String> = ICON_PANELS.iter().map(|e| e.0).filter(|id| !in_rows(id) && f.docking_icons.contains(*id)).map(String::from).collect();
    icon_groups.extend(take_group(&mut placed, &dropped, 0));
    if !icon_groups.is_empty() {
        columns.push(column_of(icon_groups, COLUMN_WIDTH, true));
    }
    docked.sort_by_key(|(n, _)| *n);
    columns.extend(docked.into_iter().map(|(_, column)| column));
    (with_canvas(columns, floating), hidden)
}

/// Place the Tools panel, the torn-off strips and the custom toolbars of `toolbars`. Returns the
/// toolbars that were not shown, each with the place it goes back to. Toolbars go in before
/// Tools, so Tools takes the outermost start column unless it is kept beside the canvas.
fn add_tools(layout: &mut Layout<String>, f: &LegacyDock, toolbars: &[CustomToolbar]) -> Vec<(String, Location<String>)> {
    let steps = |double: bool| if double { 2 } else { 1 };
    let mut placed = Placed::default();
    let mut hidden = Vec::new();
    let mut docked = Vec::new();
    for toolbar in toolbars.iter().take(MAX_LEGACY_GROUPS) {
        let owner = Owner::Toolbar(toolbar.id);
        let Some(key) = placed.claim(owner.panel()) else { continue };
        let steps = steps(toolbar.double);
        if !toolbar.shown {
            hidden.push((key, hidden_place(toolbar, steps)));
            continue;
        }
        match toolbar.dock {
            Some(order) => docked.push((order, toolbar.dock_right, steps_column(key, steps))),
            None => float_steps(layout, key, toolbar.pos, steps),
        }
        for strip in &toolbar.floating {
            torn_off(layout, &mut placed, &owner, strip);
        }
    }
    // Docked toolbars stack from the window edge in their old order: each goes outermost.
    docked.sort_by_key(|(order, ..)| std::cmp::Reverse(*order));
    for (_, right, column) in docked {
        dock_column(layout, if right { DockEdge::End } else { DockEdge::Start }, false, column);
    }

    if let Some(key) = placed.claim(toolbar::TOOLS.to_string()) {
        let steps = steps(f.toolbar_double);
        match f.toolbar_pos.filter(|p| p.iter().all(|v| v.is_finite())) {
            Some(pos) => float_steps(layout, key, pos, steps),
            None => {
                let (side, inner) = if f.toolbar_right { (DockEdge::End, true) } else { (DockEdge::Start, f.tools_beside_canvas) };
                dock_column(layout, side, inner, steps_column(key, steps));
            }
        }
    }
    for strip in f.floating_flyouts.iter().take(MAX_LEGACY_GROUPS) {
        torn_off(layout, &mut placed, &Owner::Tools, strip);
    }
    hidden
}

/// A strip torn off `owner`: a floating window of one group, as `toolbar::float_group` makes.
fn torn_off(layout: &mut Layout<String>, placed: &mut Placed, owner: &Owner, strip: &FloatingFlyout) {
    // PLAN: chrome-w10 (ch.P4) is asked for one shared builder of this window; this one matches `float_group`.
    // PLAN: assumed `toolbar::strip_id(&Owner, &str) -> String`.
    let Some(first) = strip.tools.first() else { return };
    if let Some(key) = placed.claim(toolbar::strip_id(owner, first)) {
        float_steps(layout, key, strip.pos, 1);
    }
}

/// A column of one panel that lays its content in `steps` columns (the Tools panel, a toolbar,
/// a strip).
fn steps_column(panel: String, steps: u16) -> Column<String> {
    Column::new(vec![Group::new(vec![panel], 0)]).with_steps(steps)
}

/// Floats a [`steps_column`] of `panel` at `pos` in a window of its own; the renderer sizes it
/// from its panel.
fn float_steps(layout: &mut Layout<String>, panel: String, pos: [f32; 2], steps: u16) {
    let at = finite_pos(pos);
    let id = layout.fresh_window_id();
    layout.floating.push(Floating::new(id, steps_column(panel, steps), Rect::from_min_size(pos2(at[0], at[1]), vec2(ICON_WIDTH, ICON_WIDTH))));
}

/// Where a toolbar that is not shown goes back to: its dock edge, else where it floated.
fn hidden_place(toolbar: &CustomToolbar, steps: u16) -> Location<String> {
    if toolbar.dock.is_some() {
        let edge = if toolbar.dock_right { DockEdge::End } else { DockEdge::Start };
        // With no anchor, the column's edge is where the toolbar goes back: the outermost column there.
        let mut location = Location::new(None, DockPlacement::column(false));
        location.group = Some(GroupLocation::new(false, None, Some(ColumnLocation::new(Some(edge), None, false, None).with_steps(Some(steps)))));
        location
    } else {
        let at = finite_pos(toolbar.pos);
        let mut location = Location::new(None, DockPlacement::tab(None));
        location.floating = Some(Rect::from_min_size(pos2(at[0], at[1]), vec2(ICON_WIDTH, ICON_WIDTH)));
        location
    }
}

/// The slot remembered for each of the old `slot_tool` entries (slot's first tool to the tool
/// shown); an id that no longer names a tool is dropped.
fn slot_memory(slot_tool: &BTreeMap<String, String>) -> LastUsed<String> {
    let mut memory = LastUsed::default();
    for (first, tool) in slot_tool.iter().take(MAX_LEGACY_GROUPS) {
        if let Some(slot) = slot_of(first).filter(|_| vectorcraft_tools::tool_info(tool).is_some()) {
            memory.set(slot, tool.clone());
        }
    }
    memory
}

/// A slot's id: the index of its first tool among all tools.
fn slot_of(first: &str) -> Option<SlotId> {
    vectorcraft_tools::catalog::all_tools().position(|t| t.id == first).and_then(|i| SlotId::try_from(i).ok())
}

/// The canvas beside a dock of `columns` (from the window edge inwards) at the end edge.
fn with_canvas(columns: Vec<Column<String>>, floating: Vec<Floating<String>>) -> Layout<String> {
    let width: f32 = columns.iter().map(extent).sum();
    let root = Node::split(Axis::X, SplitSize::FixedSecond(width), Node::tabs(vec![CANVAS.to_string()], 0), Node::dock(DockEdge::End, columns));
    Layout::new(Some(root), floating)
}

/// The room a column takes beside the canvas.
fn extent(column: &Column<String>) -> f32 {
    match column.steps {
        // The renderer sizes a steps column from its panel; a step is about an icon wide.
        Some(steps) => ICON_WIDTH * f32::from(steps),
        None if column.iconic => column.iconic_width.unwrap_or(ICON_WIDTH),
        None => column.width.unwrap_or(COLUMN_WIDTH),
    }
}

/// Put `column` in the dock at `side`, the innermost column when `inner`, else the outermost,
/// making the dock when the layout has none.
fn dock_column(layout: &mut Layout<String>, side: DockEdge, inner: bool, column: Column<String>) {
    let width = extent(&column);
    let mut column = Some(column);
    if let Some(root) = layout.root.as_mut() {
        insert_column(root, side, inner, width, &mut column, 0);
    }
    let Some(column) = column else { return };
    let dock = Node::dock(side, vec![column]);
    layout.root = Some(match (side, layout.root.take()) {
        (_, None) => dock,
        (DockEdge::Start, Some(rest)) => Node::split(Axis::X, SplitSize::FixedFirst(width), dock, rest),
        (_, Some(rest)) => Node::split(Axis::X, SplitSize::FixedSecond(width), rest, dock),
    });
}

/// Put `column` in the dock at `side` found along the horizontal splits below `node`, growing the
/// fixed size of the split that holds it. `column` is left `Some` when no such dock exists.
fn insert_column(node: &mut Node<String>, side: DockEdge, inner: bool, width: f32, column: &mut Option<Column<String>>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match node {
        Node::Dock { side: at, columns, .. } if *at == side => {
            if let Some(column) = column.take() {
                if inner {
                    columns.push(column);
                } else {
                    columns.insert(0, column);
                }
            }
        }
        Node::Split { size, first, second, .. } => {
            let waiting = column.is_some();
            insert_column(first, side, inner, width, column, depth.saturating_add(1));
            if waiting && column.is_none() {
                if let SplitSize::FixedFirst(room) = size {
                    *room += width;
                }
                return;
            }
            insert_column(second, side, inner, width, column, depth.saturating_add(1));
            if waiting
                && column.is_none()
                && let SplitSize::FixedSecond(room) = size
            {
                *room += width;
            }
        }
        _ => {}
    }
}

/// A dock column of `groups`, `width` wide when expanded.
fn column_of(groups: Vec<Group<String>>, width: f32, iconic: bool) -> Column<String> {
    let mut column = Column::new(groups).with_width(width).with_iconic(iconic);
    column.iconic_width = Some(ICON_WIDTH);
    column
}

/// Panels already given a place, so each appears once.
#[derive(Default)]
struct Placed(Vec<String>);

impl Placed {
    /// The registered id of `raw` if no other place holds it yet.
    fn take(&mut self, raw: &str) -> Option<String> {
        self.claim(crate::menus::normalize_panel(raw)?.to_string())
    }

    /// `key` if no other place holds it yet.
    fn claim(&mut self, key: String) -> Option<String> {
        if self.0.contains(&key) {
            return None;
        }
        self.0.push(key.clone());
        Some(key)
    }
}

/// The main column's tabs in their order: `dock_tab_order` first, then the tabs it leaves out.
fn main_tabs(order: &[DockTab]) -> Vec<DockTab> {
    let mut out: Vec<DockTab> = Vec::new();
    for tab in order.iter().copied().chain(DockTab::ALL) {
        if !out.contains(&tab) {
            out.push(tab);
        }
    }
    out
}

/// The main column's width for a saved `dock_width`.
fn main_width(width: Option<f32>) -> f32 {
    width.filter(|w| w.is_finite()).map_or(COLUMN_WIDTH, |w| w.clamp(COLUMN_MIN, COLUMN_MAX))
}

/// A group of the panels of `panels` that have no place yet, showing the panel at `active` when
/// it is kept.
fn take_group(placed: &mut Placed, panels: &[String], active: usize) -> Option<Group<String>> {
    let front = panels.get(active).cloned();
    let kept: Vec<String> = panels.iter().take(MAX_PANELS).filter_map(|p| placed.take(p)).collect();
    let active = front.and_then(|front| kept.iter().position(|p| *p == front)).unwrap_or(0);
    (!kept.is_empty()).then(|| Group::new(kept, active))
}

/// The height the old layout gave the floating window of `panels`, else the default.
fn old_height(old: Option<&Layout<String>>, panels: &[String]) -> f32 {
    old.and_then(|l| l.floating.iter().find(|w| panels.iter().any(|p| w.contains(p))))
        .map(|w| w.rect.height())
        .filter(|h| h.is_finite() && *h >= 40.0)
        .map_or(FLOAT_HEIGHT, |h| h.min(STACK_MAX))
}

/// A saved floating window rect with a usable position and size.
fn usable_rect(rect: Rect) -> Rect {
    let at = finite_pos([rect.min.x, rect.min.y]);
    let width = Some(rect.width()).filter(|w| w.is_finite() && *w > 0.0).map_or(FLOAT_WIDTH, |w| w.clamp(FLOAT_MIN, COLUMN_MAX));
    let height = Some(rect.height()).filter(|h| h.is_finite() && *h >= 40.0).map_or(FLOAT_HEIGHT, |h| h.min(STACK_MAX));
    Rect::from_min_size(pos2(at[0], at[1]), vec2(width, height))
}

/// A saved floating position, or a place near the window's corner when it is not a number.
fn finite_pos(pos: [f32; 2]) -> [f32; 2] {
    if pos.iter().all(|v| v.is_finite()) { pos.map(|v| v.clamp(-100_000.0, 100_000.0)) } else { [100.0, 100.0] }
}

/// The groups of an old dock tree's node, column by column in reading order. A vertical split
/// stacks its parts in one column; a horizontal split puts them side by side.
fn tree_columns(node: &Node<String>, depth: usize, placed: &mut Placed) -> Vec<Vec<Group<String>>> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let column = |groups: Vec<Group<String>>| if groups.is_empty() { Vec::new() } else { vec![groups] };
    match node {
        Node::Tabs { panels, active, .. } => column(take_group(placed, panels, *active).into_iter().collect()),
        Node::Stack { entries, .. } => column(
            entries
                .iter()
                .take(MAX_PANELS)
                .filter_map(|e| {
                    let id = placed.take(&e.panel)?;
                    let mut group = Group::new(vec![id], 0);
                    group.minimized = !e.open;
                    Some(group)
                })
                .collect(),
        ),
        Node::Split { axis, first, second, .. } => {
            let mut parts = tree_columns(first, depth.saturating_add(1), placed);
            parts.extend(tree_columns(second, depth.saturating_add(1), placed));
            match axis {
                Axis::X => parts,
                _ => column(parts.into_iter().flatten().collect()),
            }
        }
        _ => Vec::new(),
    }
}

/// Does a saved layout hold the canvas (one saved by this version)?
pub(crate) fn has_canvas(layout: &Layout<String>) -> bool {
    layout.panels().iter().any(|p| p.as_str() == CANVAS)
}

#[cfg(test)]
mod tests;
