//! Saved preferences at the file boundary: docking data this version cannot read is dropped
//! without resetting anything else, and layouts saved before the canvas joined the panel layout
//! are converted.
//!
//! Those older files kept the dock as a tree of the right dock's panels only (`docking`, or the
//! tabbed Properties · Layers · Libraries group when there was none) and described the rest in
//! fields of their own: `floating_panels` (floating groups and sets, collapsed groups, sets
//! docked as columns), `docking_icons` (panels dropped on the icon column), `docking_collapsed`
//! (dock groups collapsed to their tabs), `dock_collapsed` and `dock_width`. [`from_legacy`]
//! turns them into one layout; the legacy fields are read, never written.

use std::collections::{BTreeMap, BTreeSet};

use craft_ui::docking::{Column, Floating, Group, Layout, Location, Node};
use serde_json::{Map, Value};

use crate::panel_docking::{self as docking, CANVAS, COLUMN_MAX, COLUMN_WIDTH, ICON_WIDTH, Placed};
use crate::state::{DockTab, FloatingPanels, ICON_PANEL_GROUPS, ICON_PANELS, UiState};
use crate::workspaces::Workspace;

/// Saved preferences, and whether reading them discarded anything.
pub struct ReadPreferences {
    pub ui: UiState,
    /// False when unreadable docking data, or a layout that could not be converted, was dropped:
    /// the caller keeps a copy of the file before saving over it.
    pub intact: bool,
    /// True when a layout saved before the canvas joined the panel layout was converted: the
    /// caller keeps a copy of the file as it was.
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

impl UiState {
    /// Read saved preferences. None when the file is not a preferences file at all; unreadable
    /// docking data alone does not reset the other settings or the saved workspaces.
    pub fn read_preferences(bytes: &[u8]) -> Option<ReadPreferences> {
        let mut saved: Value = serde_json::from_slice(bytes).ok()?;
        let fields = saved.as_object_mut()?;
        let mut intact = recover(fields, Keys::SNAKE);
        for key in ["custom_workspaces", "workspace_changes"] {
            if let Some(Value::Array(workspaces)) = fields.get_mut(key) {
                for workspace in workspaces.iter_mut().filter_map(Value::as_object_mut) {
                    intact &= recover(workspace, Keys::CAMEL);
                }
            }
        }
        let mut ui: UiState = serde_json::from_value(saved).ok()?;
        let outcome = migrate_ui(&mut ui);
        Some(ReadPreferences { ui, intact: intact && outcome.intact, converted: outcome.converted })
    }
}

/// The JSON keys of the docking fields: `UiState` is snake_case, [`Workspace`] camelCase.
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

/// The docking fields of a `UiState` or a [`Workspace`].
pub(crate) struct Fields<'a> {
    pub docking: &'a mut Option<Layout<String>>,
    pub hidden: &'a mut BTreeMap<String, Location<String>>,
    pub floating: &'a mut Vec<FloatingPanels>,
    pub icons: &'a mut BTreeSet<String>,
    pub collapsed: &'a mut BTreeSet<String>,
    pub dock_tab: DockTab,
    pub dock_tab_order: &'a [DockTab],
    pub dock_collapsed: bool,
    pub dock_width: Option<f32>,
}

impl<'a> Fields<'a> {
    pub(crate) fn of_ui(ui: &'a mut UiState) -> Self {
        Fields {
            docking: &mut ui.docking,
            hidden: &mut ui.docking_hidden,
            floating: &mut ui.floating_panels,
            icons: &mut ui.docking_icons,
            collapsed: &mut ui.docking_collapsed,
            dock_tab: ui.dock_tab,
            dock_tab_order: &ui.dock_tab_order,
            dock_collapsed: ui.dock_collapsed,
            dock_width: ui.dock_width,
        }
    }

    /// A workspace's fields; workspaces keep no tab order of their own.
    pub(crate) fn of_workspace(w: &'a mut Workspace, dock_tab_order: &'a [DockTab]) -> Self {
        Fields {
            docking: &mut w.docking,
            hidden: &mut w.docking_hidden,
            floating: &mut w.floating_panels,
            icons: &mut w.docking_icons,
            collapsed: &mut w.docking_collapsed,
            dock_tab: w.dock_tab,
            dock_tab_order,
            dock_collapsed: w.dock_collapsed,
            dock_width: w.dock_width,
        }
    }
}

/// What converting saved layouts did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Outcome {
    /// False when a layout was dropped.
    pub intact: bool,
    /// True when a legacy layout was converted (or dropped trying).
    pub converted: bool,
}

impl Outcome {
    fn and(self, other: Outcome) -> Outcome {
        Outcome { intact: self.intact && other.intact, converted: self.converted || other.converted }
    }
}

/// Convert the layouts of `ui`, its saved workspaces and the changes kept for each workspace.
pub(crate) fn migrate_ui(ui: &mut UiState) -> Outcome {
    let order = ui.dock_tab_order.clone();
    let mut outcome = migrate(Fields::of_ui(ui));
    for w in ui.custom_workspaces.iter_mut().chain(ui.workspace_changes.iter_mut()) {
        outcome = outcome.and(migrate(Fields::of_workspace(w, &order)));
    }
    outcome
}

/// Convert one saved state's legacy layout in place and clear its legacy fields. A state with
/// no legacy data is left alone: without a layout, its flags still describe its default one.
/// Not intact when a layout was dropped: one this version cannot use, or a conversion that
/// failed (the state then falls back to its default layout).
pub(crate) fn migrate(f: Fields<'_>) -> Outcome {
    let current = f.docking.as_ref().is_some_and(has_canvas);
    let legacy = !current && (f.docking.is_some() || !f.floating.is_empty() || !f.icons.is_empty() || !f.collapsed.is_empty());
    let mut intact = true;
    if current {
        intact = f.docking.as_ref().is_some_and(docking::valid);
    } else if legacy {
        match from_legacy(&f) {
            Some((layout, hidden)) => {
                *f.docking = Some(layout);
                *f.hidden = hidden;
            }
            None => {
                *f.docking = None;
                f.hidden.clear();
                intact = false;
            }
        }
    }
    f.floating.clear();
    f.icons.clear();
    f.collapsed.clear();
    Outcome { intact, converted: legacy }
}

/// The layout older versions described with `f`, and the hidden places that still apply. The
/// dock, from the window edge inwards: the old dock tree's columns (its groups stacked top to
/// bottom; a horizontal split puts its parts side by side), the icon column, then the floating
/// sets docked as columns. Old split ratios are not kept. None when the result is not a valid
/// layout.
pub(crate) fn from_legacy(f: &Fields<'_>) -> Option<(Layout<String>, Hidden)> {
    let old = f.docking.as_ref();
    let mut placed = Placed::default();
    let groups: Vec<&FloatingPanels> = f.floating.iter().take(MAX_LEGACY_GROUPS).collect();

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
                    None => docked.push((n, Column::new(groups.collect(), width))),
                }
            }
            None => {
                let at = finite_pos(g.pos);
                let all_minimized = kept.iter().all(|(group, _)| group.minimized);
                let height = kept.iter().map(|(_, h)| *h).sum::<f32>().min(STACK_MAX);
                let rect = [at[0], at[1], width, height];
                // A floating window's groups have no minimized state of their own: a set is
                // minimized when all its groups were collapsed, else every group shows. Their
                // heights become the set's split ratios.
                let window = match kept.as_slice() {
                    [(group, _)] => Some(Floating::single(group.panels.clone(), group.active, rect)),
                    _ => Floating::stack(kept.into_iter().map(|(group, height)| Group { height: Some(height), ..group }).collect(), rect),
                };
                if let Some(mut window) = window {
                    window.minimized = all_minimized;
                    floating.push(window);
                }
            }
        }
    }
    // Windows of the old layout that the floating groups did not list.
    for window in old.map(|l| l.floating.as_slice()).unwrap_or_default() {
        if let Some(root) = unplaced_tree(&window.root, 0, &mut placed) {
            floating.push(Floating { minimized: window.minimized, ..Floating::new(root, usable_rect(window.rect)) });
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
            let tabs: Vec<&str> = docking::main_tabs(f.dock_tab_order).into_iter().map(|t| t.info().0).filter(|id| !f.icons.contains(*id)).collect();
            let active = tabs.iter().position(|id| *id == f.dock_tab.info().0).unwrap_or(0);
            let tabs: Vec<String> = tabs.into_iter().map(String::from).collect();
            take_group(&mut placed, &tabs, active).map(|g| vec![vec![g]]).unwrap_or_default()
        }
    };
    let n = tree_columns.len().max(1) as f32;
    let tree_width = (docking::main_width(f.dock_width) / n).max(FLOAT_MIN.min(docking::main_width(f.dock_width)));
    let mut columns: Vec<Column<String>> = tree_columns
        .into_iter()
        .map(|mut groups| {
            for g in &mut groups {
                g.minimized |= g.panels.get(g.active).is_some_and(|p| f.collapsed.contains(p));
            }
            Column { groups, width: tree_width, iconic: f.dock_collapsed, iconic_width: Some(ICON_WIDTH) }
        })
        .collect();

    // Panels closed with a remembered place stay closed, except those the icon column showed
    // anyway (its rows, and the panels dropped on it): they take their place there.
    let in_rows = |id: &str| ICON_PANEL_GROUPS.iter().any(|row| row.contains(&id));
    let hidden: BTreeMap<String, Location<String>> =
        f.hidden.iter().take(craft_ui::docking::MAX_PANELS).map(|(id, location)| (id.clone(), location.clone())).collect();
    for id in hidden.keys().filter(|id| !in_rows(id) && !f.icons.contains(*id)) {
        let _ = placed.take(id);
    }

    // The icon column: the tabbed group's panels dropped on it, its rows of icon panels, then
    // the other panels dropped on it.
    let mut icon_groups = Vec::new();
    let tabs: Vec<String> = DockTab::ALL.iter().map(|t| t.info().0.to_string()).filter(|id| f.icons.contains(id)).collect();
    icon_groups.extend(take_group(&mut placed, &tabs, 0));
    for row in ICON_PANEL_GROUPS {
        let row: Vec<String> = row.iter().map(|id| id.to_string()).collect();
        icon_groups.extend(take_group(&mut placed, &row, 0));
    }
    let dropped: Vec<String> = ICON_PANELS.iter().map(|e| e.0).filter(|id| !in_rows(id) && f.icons.contains(*id)).map(String::from).collect();
    icon_groups.extend(take_group(&mut placed, &dropped, 0));
    if !icon_groups.is_empty() {
        columns.push(Column { groups: icon_groups, width: COLUMN_WIDTH, iconic: true, iconic_width: Some(ICON_WIDTH) });
    }
    docked.sort_by_key(|(n, _)| *n);
    columns.extend(docked.into_iter().map(|(_, column)| column));

    let layout = docking::with_canvas(columns, floating);
    if !docking::valid(&layout) {
        return None;
    }
    // A hidden place applies to a closed panel, and to a floating one Dock Panel sends back
    // there; a floating panel whose way back was the icon column finds its icon group again.
    let hidden = hidden
        .into_iter()
        .filter(|(id, _)| match layout.floating.iter().any(|w| w.contains(id)) {
            true => !f.icons.contains(id),
            false => !layout.contains(id),
        })
        .collect();
    Some((layout, hidden))
}

/// A group of the panels of `panels` that have no place yet, showing the panel at `active` when
/// it is kept.
fn take_group(placed: &mut Placed, panels: &[String], active: usize) -> Option<Group<String>> {
    let front = panels.get(active).cloned();
    let kept: Vec<String> = panels.iter().take(craft_ui::docking::MAX_PANELS).filter_map(|p| placed.take(p)).collect();
    let active = front.and_then(|front| kept.iter().position(|p| *p == front)).unwrap_or(0);
    (!kept.is_empty()).then(|| Group::new(kept, active))
}

/// The height the old layout gave the floating window of `panels`, else the default.
fn old_height(old: Option<&Layout<String>>, panels: &[String]) -> f32 {
    old.and_then(|l| l.floating.iter().find(|w| panels.iter().any(|p| w.contains(p))))
        .map(|w| w.rect[3])
        .filter(|h| h.is_finite() && *h >= 40.0)
        .map_or(FLOAT_HEIGHT, |h| h.min(STACK_MAX))
}

/// A saved floating window rect with a usable position and size.
fn usable_rect(rect: [f32; 4]) -> [f32; 4] {
    let [x, y, width, height] = rect;
    let at = finite_pos([x, y]);
    let width = Some(width).filter(|w| w.is_finite() && *w > 0.0).map_or(FLOAT_WIDTH, |w| w.clamp(FLOAT_MIN, COLUMN_MAX));
    let height = Some(height).filter(|h| h.is_finite() && *h >= 40.0).map_or(FLOAT_HEIGHT, |h| h.min(STACK_MAX));
    [at[0], at[1], width, height]
}

/// A saved floating position, or a place near the window's corner when it is not a number.
fn finite_pos(pos: [f32; 2]) -> [f32; 2] {
    if pos.iter().all(|v| v.is_finite()) { pos.map(|v| v.clamp(-100_000.0, 100_000.0)) } else { [100.0, 100.0] }
}

/// The groups of an old dock tree's node, column by column in reading order. A vertical split
/// stacks its parts in one column; a horizontal split puts them side by side.
fn tree_columns(node: &Node<String>, depth: usize, placed: &mut Placed) -> Vec<Vec<Group<String>>> {
    if depth > craft_ui::docking::MAX_DEPTH {
        return Vec::new();
    }
    let column = |groups: Vec<Group<String>>| if groups.is_empty() { Vec::new() } else { vec![groups] };
    match node {
        Node::Tabs { panels, active } => column(take_group(placed, panels, *active).into_iter().collect()),
        Node::Stack { entries } => column(
            entries
                .iter()
                .take(craft_ui::docking::MAX_PANELS)
                .filter_map(|e| placed.take(&e.panel).map(|id| Group { minimized: !e.open, ..Group::new(vec![id], 0) }))
                .collect(),
        ),
        Node::Split { axis, first, second, .. } => {
            let mut parts = tree_columns(first, depth.saturating_add(1), placed);
            parts.extend(tree_columns(second, depth.saturating_add(1), placed));
            match axis {
                craft_ui::layout::SplitAxis::Horizontal => parts,
                _ => column(parts.into_iter().flatten().collect()),
            }
        }
        _ => Vec::new(),
    }
}

/// A floating window's tree with only the panels not placed yet; a split that loses one side
/// becomes its other side. None when no panel is left.
fn unplaced_tree(node: &Node<String>, depth: usize, placed: &mut Placed) -> Option<Node<String>> {
    if depth > craft_ui::docking::MAX_DEPTH {
        return None;
    }
    match node {
        Node::Tabs { panels, active } => take_group(placed, panels, *active).map(|g| Node::Tabs { panels: g.panels, active: g.active }),
        Node::Stack { entries } => {
            let entries: Vec<_> = entries
                .iter()
                .take(craft_ui::docking::MAX_PANELS)
                .filter_map(|e| placed.take(&e.panel).map(|panel| craft_ui::docking::StackEntry { panel, ..e.clone() }))
                .collect();
            (!entries.is_empty()).then_some(Node::Stack { entries })
        }
        Node::Split { axis, size, first, second } => {
            let first = unplaced_tree(first, depth.saturating_add(1), placed);
            let second = unplaced_tree(second, depth.saturating_add(1), placed);
            match (first, second) {
                (Some(first), Some(second)) => Some(Node::Split { axis: *axis, size: *size, first: Box::new(first), second: Box::new(second) }),
                (one, other) => one.or(other),
            }
        }
        _ => None,
    }
}

/// Does a saved layout hold the canvas (one saved by this version)?
pub(crate) fn has_canvas(layout: &Layout<String>) -> bool {
    layout.panels().iter().any(|p| p.as_str() == CANVAS)
}

#[cfg(test)]
mod tests;
