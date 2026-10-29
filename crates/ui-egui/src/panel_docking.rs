//! The window's panels and the canvas as one craft-ui docking layout.
//!
//! The layout is a horizontal split of the protected `canvas` panel and a dock at the window's
//! right edge. The dock's columns, from the window edge inwards: the Properties · Layers ·
//! Libraries column, the icon column (one group per row of [`ICON_PANEL_GROUPS`]) and any columns
//! the user adds. Floating panels open in OS windows of their own on the desktop (feature
//! `native-windows`) and inside the main window on the web and on Wayland. A workspace that
//! was never customized has no saved layout (`UiState::docking` is `None`): its layout is built
//! from its flags (`dock_tab`, `dock_collapsed`, `dock_width`).
//!
//! craft-ui draws the layout and its gestures; every change goes through [`Transaction`], whether
//! a gesture, a menu item or a control command made it.

use std::collections::BTreeMap;

use craft_ui::docking::{
    Action, Column, DockCommand, DockEvent, DockOperation, FloatSpace, Floating, Group, HidePanels, Layout, Location, Node, Permissions, Placement,
    Side, Target, UiDock, Workspace as DockWorkspace,
};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::{DockTab, ICON_PANEL_GROUPS, UiState};

/// The id of the dock area that draws the panels.
pub(crate) fn area_id() -> egui::Id {
    egui::Id::unique("vectorcraft-panel-docking")
}

/// The dock area a headless app floats into, before it ever drew a frame.
const HEADLESS_AREA: egui::Rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1400.0, 900.0));

/// The panel that holds the document area. It never moves, floats, closes or takes tabs.
pub(crate) const CANVAS: &str = "canvas";
/// Width of the Properties · Layers · Libraries column until the user resizes it.
pub(crate) const COLUMN_WIDTH: f32 = 310.0;
/// The widths the main column may be given (`dock_width`).
pub(crate) const COLUMN_MIN: f32 = 220.0;
pub(crate) const COLUMN_MAX: f32 = 720.0;
/// Width of a column collapsed to icons.
pub(crate) const ICON_WIDTH: f32 = 38.0;

/// Docking state that lives for the session only.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Where the dock area floats panels, as it was last drawn.
    space: Option<FloatSpace<String>>,
    /// A panel a command showed: its floating window comes forward on the next frame.
    raise: Option<String>,
    /// The panel the user last clicked or pressed in (a tab, a panel body, a drop).
    focused: Option<String>,
}

/// Every docking control the main window drew last frame with its rect. Controls in floating
/// panels' own OS windows are not among them: egui keeps each window's widgets apart.
pub(crate) fn hit_rects(ctx: &egui::Context) -> Vec<(craft_ui::docking::Hit<String>, egui::Rect)> {
    craft_ui::docking::DockArea::<String>::hit_rects(ctx, area_id())
}

/// `ui.docking`: the layout in use, the open flyout, the focused panel and every docking
/// control the main window drew last frame (`hitRects`, rects in window points).
pub(crate) fn inspect(app: &VectorcraftApp, ctx: &egui::Context) -> Value {
    let layout = layout_of(&app.ui);
    let hits: Vec<Value> = hit_rects(ctx).iter().map(|(hit, r)| json!({"hit": hit, "rect": [r.left(), r.top(), r.width(), r.height()]})).collect();
    json!({
        "layout": layout,
        "customized": app.ui.docking.is_some(),
        "flyout": app.ui.open_panel,
        "focused": app.dock_rt.focused.as_ref().filter(|p| layout.contains(p)),
        "hidden": app.ui.docking_hidden.keys().collect::<Vec<_>>(),
        "autoCollapse": app.session.prefs.auto_collapse_icon_panels,
        "panelsShown": app.ui.dock && app.ui.screen_mode < 3,
        "hitRects": hits,
    })
}

fn normalize(raw: &str) -> Option<&'static str> {
    crate::menus::normalize_panel(raw)
}

fn registered(id: &str) -> bool {
    id == CANVAS || normalize(id) == Some(id)
}

/// The policy commands and the renderer apply: the canvas is protected.
fn model_permissions(id: &String) -> Permissions {
    if id == CANVAS { Permissions::PROTECTED } else { Permissions::default() }
}

/// Whether a saved layout can be used: valid, of registered panels only, with the canvas docked.
/// A layout from before the canvas joined it is not; its workspace falls back to the default.
pub(crate) fn valid(layout: &Layout<String>) -> bool {
    layout.validate().is_ok()
        && layout.panels().iter().all(|id| registered(id))
        && Layout { root: layout.root.clone(), floating: Vec::new() }.contains(&CANVAS.to_string())
}

/// Panels already given a place, so each appears once.
#[derive(Default)]
pub(crate) struct Placed(Vec<String>);

impl Placed {
    /// The registered id of `raw` if no other place holds it yet.
    pub(crate) fn take(&mut self, raw: &str) -> Option<String> {
        let id = normalize(raw)?;
        if self.0.iter().any(|p| p == id) {
            return None;
        }
        self.0.push(id.to_string());
        Some(id.to_string())
    }

    /// A group of the unplaced registered panels of `panels`.
    fn group(&mut self, panels: &[&str]) -> Option<Group<String>> {
        let kept: Vec<String> = panels.iter().filter_map(|p| self.take(p)).collect();
        (!kept.is_empty()).then(|| Group::new(kept, 0))
    }
}

/// The main column's tabs in their order: `dock_tab_order` first, then the tabs it leaves out.
pub(crate) fn main_tabs(order: &[DockTab]) -> Vec<DockTab> {
    let mut out: Vec<DockTab> = Vec::new();
    for tab in order.iter().copied().chain(DockTab::ALL) {
        if !out.contains(&tab) {
            out.push(tab);
        }
    }
    out
}

/// The main column's width for a saved `dock_width`.
pub(crate) fn main_width(width: Option<f32>) -> f32 {
    width.filter(|w| w.is_finite()).map_or(COLUMN_WIDTH, |w| w.clamp(COLUMN_MIN, COLUMN_MAX))
}

/// The canvas beside a dock of `columns` (from the window edge inwards) at the right edge.
pub(crate) fn with_canvas(columns: Vec<Column<String>>, floating: Vec<Floating<String>>) -> Layout<String> {
    let width: f32 = columns.iter().map(|c| if c.iconic { c.iconic_width.unwrap_or(ICON_WIDTH) } else { c.width }).sum();
    Layout {
        root: Some(Node::Split {
            axis: craft_ui::layout::SplitAxis::Horizontal,
            size: craft_ui::layout::SplitSize::FixedSecond(width),
            first: Box::new(Node::Tabs { panels: vec![CANVAS.to_string()], active: 0 }),
            second: Box::new(Node::Dock { side: Side::End, columns }),
        }),
        floating,
    }
}

/// The default layout for these flags: the main column showing `tab` (collapsed to icons when
/// `collapsed`, `width` wide), then the icon column. A group collapsed to icons shows `front`
/// when it holds it (the open flyout).
fn default_from(tab: DockTab, order: &[DockTab], collapsed: bool, width: Option<f32>, front: Option<&str>) -> Layout<String> {
    let mut placed = Placed::default();
    let panels: Vec<String> = main_tabs(order).into_iter().filter_map(|t| placed.take(t.info().0)).collect();
    let active = panels.iter().position(|p| p == tab.info().0).unwrap_or(0);
    let mut columns = Vec::new();
    if !panels.is_empty() {
        columns.push(Column {
            groups: vec![Group::new(panels, active)],
            width: main_width(width),
            iconic: collapsed,
            iconic_width: Some(ICON_WIDTH),
        });
    }
    let groups: Vec<Group<String>> = ICON_PANEL_GROUPS.iter().filter_map(|g| placed.group(g)).collect();
    if !groups.is_empty() {
        columns.push(Column { groups, width: COLUMN_WIDTH, iconic: true, iconic_width: Some(ICON_WIDTH) });
    }
    for group in columns.iter_mut().filter(|c| c.iconic).flat_map(|c| c.groups.iter_mut()) {
        if let Some(k) = front.and_then(|front| group.panels.iter().position(|p| p == front)) {
            group.active = k;
        }
    }
    with_canvas(columns, Vec::new())
}

/// The layout of a workspace that has not been customized, from its flags.
pub(crate) fn default_layout(ui: &UiState) -> Layout<String> {
    default_from(ui.dock_tab, &ui.dock_tab_order, ui.dock_collapsed, ui.dock_width, ui.open_panel.as_deref())
}

/// The layout in use: the customized one, or the workspace's default.
pub(crate) fn layout_of(ui: &UiState) -> Layout<String> {
    ui.docking.clone().filter(valid).unwrap_or_else(|| default_layout(ui))
}

/// The flags whose default layout is `layout`, if there are any: a change that only picks another
/// tab of the main column, collapses or expands it or resizes it leaves a workspace uncustomized.
fn default_flags(ui: &UiState, layout: &Layout<String>) -> Option<(DockTab, bool, Option<f32>)> {
    let main = dock_columns(layout).first().copied()?;
    let group = main.groups.first()?;
    let tab = DockTab::from_id(group.panels.get(group.active)?)?;
    let width = if (main.width - main_width(ui.dock_width)).abs() < 0.01 { ui.dock_width } else { Some(main.width) };
    // The canvas split's size follows the dock's columns, whichever way a change kept it.
    let mut default = default_from(tab, &ui.dock_tab_order, main.iconic, width, None);
    if let (Some(Node::Split { size, .. }), Some(Node::Split { size: kept, .. })) = (default.root.as_mut(), layout.root.as_ref()) {
        *size = *kept;
    }
    // Which panel an icon group shows follows the open flyout.
    if let (Some(Node::Split { second, .. }), Some(Node::Split { second: kept, .. })) = (default.root.as_mut(), layout.root.as_ref())
        && let (Node::Dock { columns, .. }, Node::Dock { columns: kept, .. }) = (second.as_mut(), kept.as_ref())
    {
        for (column, kept) in columns.iter_mut().zip(kept).filter(|(c, _)| c.iconic) {
            for (group, kept) in column.groups.iter_mut().zip(&kept.groups) {
                group.active = kept.active;
            }
        }
    }
    (default == *layout).then_some((tab, main.iconic, width))
}

/// Every dock column, from each dock's window edge inwards.
fn dock_columns(layout: &Layout<String>) -> Vec<&Column<String>> {
    let mut out = Vec::new();
    let mut visit: Vec<&Node<String>> = layout.root.iter().collect();
    let mut seen = 0usize;
    while let Some(node) = visit.pop() {
        seen = seen.saturating_add(1);
        if seen > 2 * craft_ui::docking::MAX_PANELS {
            break;
        }
        match node {
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            Node::Dock { columns, .. } => out.extend(columns),
            _ => {}
        }
    }
    out
}

/// Is `panel` in a dock column collapsed to icons?
pub(crate) fn in_iconic_column(layout: &Layout<String>, panel: &str) -> bool {
    dock_columns(layout).iter().any(|c| c.iconic && c.groups.iter().any(|g| g.panels.iter().any(|p| p == panel)))
}

/// Is the dock's outer column (Properties · Layers · Libraries) collapsed to icons?
pub(crate) fn dock_collapsed(ui: &UiState) -> bool {
    match &ui.docking {
        Some(layout) if valid(layout) => dock_columns(layout).first().is_some_and(|c| c.iconic),
        _ => ui.dock_collapsed,
    }
}

/// A floating window's panels, first to last.
pub(crate) fn window_panels(window: &Floating<String>) -> Vec<String> {
    window.panels().cloned().collect()
}

/// The floating window holding `panel`.
fn floating_window<'a>(layout: &'a Layout<String>, panel: &str) -> Option<&'a Floating<String>> {
    layout.floating.iter().find(|w| w.contains(&panel.to_string()))
}

/// Does `panel` float?
pub(crate) fn floats(ui: &UiState, panel: &str) -> bool {
    floating_window(&layout_of(ui), panel).is_some()
}

/// Is `panel` showing: in front of its group and not in a column collapsed to icons, or in the
/// open flyout of an iconic column? The Window menu checks these.
pub(crate) fn showing(ui: &UiState, panel: &str) -> bool {
    let layout = layout_of(ui);
    let iconic = in_iconic_column(&layout, panel);
    (ui.open_panel.as_deref() == Some(panel) && iconic) || (!iconic && layout.panel_state(&panel.to_string()) == craft_ui::docking::PanelState::Front)
}

/// Where a panel with no other destination goes: a panel of the first expanded dock column,
/// else of any column.
fn first_dock_panel(layout: &Layout<String>, except: &str) -> Option<String> {
    let usable = |p: &&String| p.as_str() != except && p.as_str() != CANVAS;
    let columns = dock_columns(layout);
    let in_columns =
        |iconic: bool| columns.iter().filter(|c| c.iconic == iconic).flat_map(|c| &c.groups).flat_map(|g| &g.panels).find(usable).cloned();
    in_columns(false).or_else(|| in_columns(true))
}

fn apply(layout: &mut Layout<String>, action: Action<String>) -> Result<Vec<String>, String> {
    layout.apply_with_permissions(action, model_permissions).map_err(|e| e.to_string())
}

/// The docking commands VectorCraft handles here: `window.panel` (the Window menu), its own
/// `window.panel.collapse` and `window.collapseDock`, the Tools panel's float and dock
/// ([`crate::tools_float`]) and craft-ui's shared ids. Workspaces stay [`crate::workspaces`]'.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(result) = crate::tools_float::command(app, id, params) {
        return Some(result);
    }
    match id {
        "window.panel" => return Some(show_panel(app, params)),
        "window.collapseDock" => return Some(crate::menus::opt_bool(params, "collapsed").and_then(|c| collapse_dock(app, c))),
        "window.panel.collapse" => {
            let minimized = match crate::menus::opt_bool(params, "collapsed") {
                Ok(c) => c,
                Err(e) => return Some(Err(e)),
            };
            let raw = params.get("panel").and_then(Value::as_str).unwrap_or_default();
            let Some(panel) = normalize(raw) else { return Some(Err(format!("unknown panel `{raw}`"))) };
            return Some(shared_command(app, "window.panel.minimize", &json!({"panel": panel, "minimized": minimized})).map(|mut value| {
                let layout = layout_of(&app.ui);
                let window = floating_window(&layout, panel).is_some_and(|w| w.minimized);
                let group = layout.location(&panel.to_string()).ok().and_then(|l| l.group).is_some_and(|g| g.minimized);
                value["panel"] = json!(panel);
                value["collapsed"] = json!(window || group);
                value
            }));
        }
        "ui.dock.reset" => return Some(app.run("window.workspace.reset", json!({}))),
        "window.panel.float" | "window.panel.dock" => return Some(float_or_dock(app, id, params)),
        _ if id.starts_with("window.workspace") => return None,
        _ => {}
    }
    let id = DockCommand::<String>::IDS.iter().copied().find(|known| *known == id)?;
    Some(shared_command(app, id, params))
}

/// `window.panel {panel}`: show a panel. One in an icon column opens (or puts away) its flyout;
/// one in front of its floating window hides; one in a group comes to the front; one closed with a remembered place comes back there; a
/// library panel or a panel the dock has no place for floats.
fn show_panel(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let raw = params.get("panel").and_then(Value::as_str).unwrap_or_default();
    let panel = normalize(raw).ok_or_else(|| format!("unknown panel `{raw}`"))?;
    let layout = layout_of(&app.ui);
    app.ui.dock = true;
    if in_iconic_column(&layout, panel) {
        app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(panel) { None } else { Some(panel.into()) };
        return Ok(json!({"open": app.ui.open_panel}));
    }
    // A floating panel in front of its window hides again, as an icon panel's flyout does: its
    // shortcut shows and hides it.
    if floating_window(&layout, panel).is_some() && layout.panel_state(&panel.to_string()) == craft_ui::docking::PanelState::Front {
        return shared_command(app, "window.panel.close", &json!({"panel": panel}));
    }
    if !layout.contains(&panel.to_string()) && !app.ui.docking_hidden.contains_key(panel) && crate::panels::library_panel::parse(panel).is_some() {
        return crate::panels::library_panel::float(app, panel);
    }
    let value = shared_command(app, "window.panel.activate", &json!({"panel": panel}))?;
    let layout = layout_of(&app.ui);
    Ok(match floating_window(&layout, panel) {
        Some(window) => json!({"floating": window_panels(window)}),
        None => value,
    })
}

/// Show `panel` as `window.panel` does, but leave it be when it already shows ([`showing`]): a
/// command that opens a panel to work in (Edit › Color Settings…) never puts it away.
pub(crate) fn reveal_panel(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    if showing(&app.ui, panel) {
        app.ui.dock = true;
        return Ok(json!({"open": panel}));
    }
    show_panel(app, &json!({ "panel": panel }))
}

/// `window.collapseDock {collapsed?}`: collapse every dock column to icons, or expand the outer
/// one; without `collapsed` it toggles the outer column.
pub(crate) fn collapse_dock(app: &mut VectorcraftApp, collapsed: Option<bool>) -> Result<Value, String> {
    let iconic = collapsed.unwrap_or(!dock_collapsed(&app.ui));
    app.ui.dock = true;
    if app.ui.docking.is_none() {
        app.ui.dock_collapsed = iconic;
        // A main-column panel open as a flyout shows in the column instead.
        if !iconic && let Some(tab) = app.ui.open_panel.as_deref().and_then(DockTab::from_id) {
            app.ui.dock_tab = tab;
            app.ui.open_panel = None;
        }
        return Ok(json!(iconic));
    }
    let mut txn = Transaction::begin(app)?;
    let columns = dock_columns(&txn.workspace.layout);
    let firsts = |c: &&Column<String>| c.groups.first().and_then(|g| g.panels.first()).cloned();
    let targets: Vec<String> = if iconic {
        columns.iter().filter(|c| !c.iconic).filter_map(firsts).collect()
    } else {
        columns.first().filter(|c| c.iconic).and_then(firsts).into_iter().collect()
    };
    for panel in targets {
        txn.apply(Action::SetIconic { panel, iconic })?;
    }
    app.ui.dock_collapsed = iconic;
    txn.commit(app, Vec::new(), None);
    Ok(json!(iconic))
}

/// Where `window.panel.float` and `window.panel.dock` put a panel by the parameters VectorCraft
/// gave them before its panels moved to craft-ui's layout.
enum Legacy {
    /// `onto` (a tab of the group holding `anchor`, which floats unless the panel docks), `above`
    /// or `below` (a group of its own there, in that group's floating window or dock column).
    Join { anchor: String, placement: Placement<String>, key: &'static str, floats: bool },
    /// `icon: true`: a group of its own at the end of the icon column ([`icon_column`]), or a new
    /// icon column when the dock has none.
    Icons,
    /// `column: n` of `window.panel.dock`: a dock column of its own at place `n` of the dock, 1
    /// next to the icon column, counting inwards; `column: true` with no destination: the
    /// innermost place.
    Column(usize),
}

/// The VectorCraft-only parameters of `window.panel.float` / `window.panel.dock`, taken out of
/// `fields` so craft-ui parses the rest: where the panel goes, and `collapsed`.
fn take_legacy(id: &str, fields: &mut serde_json::Map<String, Value>) -> Result<(Option<Legacy>, Option<bool>), String> {
    let dock = id == "window.panel.dock";
    let minimized = match fields.remove("collapsed") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(c)) => Some(c),
        Some(_) => return Err("collapsed must be true or false".into()),
    };
    let mut joins = Vec::new();
    for (key, placement) in [
        ("onto", Placement::Tab { before: None }),
        ("above", Placement::Group { below: false, height: None }),
        ("below", Placement::Group { below: true, height: None }),
    ] {
        if let Some(value) = fields.remove(key) {
            let anchor = value.as_str().and_then(normalize).ok_or_else(|| format!("{key} must name a registered panel"))?;
            joins.push(Legacy::Join { anchor: anchor.to_owned(), placement, key, floats: !dock && key == "onto" });
        }
    }
    let icon = match fields.remove("icon") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) if dock => true,
        Some(Value::Bool(true)) => return Err("icon docks a panel in the icon column: use window.panel.dock".into()),
        Some(_) => return Err("icon must be true or false".into()),
    };
    let destination = ["anchor", "side", "zone", "before"].iter().any(|k| fields.get(*k).is_some_and(|v| !v.is_null()));
    let column = match fields.get("column") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        // craft-ui's `column: true`: the panel's whole dock column or floating window.
        Some(Value::Bool(true)) if !dock || destination => None,
        Some(Value::Bool(true)) => Some(usize::MAX),
        Some(value) => match value.as_u64().filter(|n| *n >= 1) {
            Some(n) if dock => Some(usize::try_from(n).unwrap_or(usize::MAX)),
            // On float a place means nothing: it floats the whole column, as `true` does.
            Some(_) => {
                fields.insert("column".into(), Value::Bool(true));
                None
            }
            None => return Err("column must be true or a number from 1".into()),
        },
    };
    if column.is_some() {
        fields.remove("column");
    }
    let mut legacy: Vec<Legacy> = joins;
    if icon {
        legacy.push(Legacy::Icons);
    }
    if let Some(n) = column {
        legacy.push(Legacy::Column(n));
    }
    if legacy.len() > 1 {
        return Err("give only one of onto, above, below, icon and a column place".into());
    }
    let legacy = legacy.pop();
    if legacy.is_some() && destination {
        return Err("onto, above, below, icon and a column place take no anchor, side, zone or before".into());
    }
    if legacy.is_some() && ["x", "y", "width", "height"].iter().any(|k| fields.contains_key(*k)) {
        return Err("onto, above, below, icon and a column place take no x, y, width or height".into());
    }
    if legacy.is_some() && fields.get("column") == Some(&Value::Bool(true)) {
        return Err("column: true floats the whole column and takes no onto, above or below".into());
    }
    Ok((legacy, minimized))
}

/// `window.panel.float` and `window.panel.dock`: craft-ui's commands, with VectorCraft's own
/// parameters ([`Legacy`], `collapsed`) done in the same transaction.
fn float_or_dock(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    let mut params = if params.is_null() { json!({}) } else { params.clone() };
    let fields = params.as_object_mut().ok_or("params must be an object")?;
    let (legacy, minimized) = take_legacy(id, fields)?;
    shared_command_with(app, id, &params, legacy, minimized)
}

/// The panels of the group holding `panel` (an accordion entry is a group of one).
fn group_of(layout: &Layout<String>, panel: &String) -> Vec<String> {
    let mut visit: Vec<&Node<String>> = layout.root.iter().chain(layout.floating.iter().map(|w| &w.root)).collect();
    let mut seen = 0usize;
    while let Some(node) = visit.pop() {
        seen = seen.saturating_add(1);
        if seen > 2 * craft_ui::docking::MAX_PANELS {
            break;
        }
        match node {
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            Node::Tabs { panels, .. } if panels.contains(panel) => return panels.clone(),
            Node::Stack { entries } if entries.iter().any(|e| &e.panel == panel) => return vec![panel.clone()],
            Node::Dock { columns, .. } => {
                if let Some(group) = columns.iter().flat_map(|c| &c.groups).find(|g| g.panels.contains(panel)) {
                    return group.panels.clone();
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

/// The columns of the dock at the window's end edge, from the edge inwards.
fn end_columns(layout: &Layout<String>) -> Vec<&Column<String>> {
    let mut visit: Vec<&Node<String>> = layout.root.iter().collect();
    let mut seen = 0usize;
    while let Some(node) = visit.pop() {
        seen = seen.saturating_add(1);
        if seen > 2 * craft_ui::docking::MAX_PANELS {
            break;
        }
        match node {
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            Node::Dock { side: Side::End, columns } => return columns.iter().collect(),
            _ => {}
        }
    }
    Vec::new()
}

/// The icon column among a dock's `columns`: the column collapsed to icons holding the most
/// panels of [`ICON_PANEL_GROUPS`] rows, the innermost on a tie. Columns carry no mark of their
/// own, and the main column may be collapsed to icons too (or, in a converted layout, several
/// columns from an old dock); what it holds tells them apart.
fn icon_column(columns: &[&Column<String>]) -> Option<usize> {
    let in_rows = |p: &&String| ICON_PANEL_GROUPS.iter().any(|row| row.contains(&p.as_str()));
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.iconic)
        .map(|(k, c)| (c.groups.iter().flat_map(|g| &g.panels).filter(in_rows).count(), k))
        .filter(|(count, _)| *count > 0)
        .max()
        .map(|(_, k)| k)
}

impl Legacy {
    /// The action that puts `panel` (with `group`, its whole group) where `self` says, on the
    /// layout with the panel shown; none when it is there already.
    fn actions(&self, layout: &Layout<String>, panel: &String, group: bool) -> Result<Vec<Action<String>>, String> {
        let mates = group_of(layout, panel);
        let unit = |to: Target<String>| {
            if group { Action::MoveGroup { panel: panel.clone(), to } } else { Action::MovePanel { panel: panel.clone(), to } }
        };
        match self {
            Self::Join { anchor, placement, key, floats } => {
                if !layout.contains(anchor) {
                    return Err(format!("{key} must name a panel that shows"));
                }
                let tab = matches!(placement, Placement::Tab { .. });
                if anchor == panel || ((group || tab) && mates.contains(anchor)) {
                    return Err(format!("{key} must name a panel in another group"));
                }
                // Float Panel stacks onto floating groups only; Dock Panel's `onto` is its `anchor`.
                if *floats && floating_window(layout, anchor).is_none() {
                    return Err(format!("{key} must name a floating panel"));
                }
                Ok(vec![unit(Target::At { anchor: anchor.clone(), placement: placement.clone() })])
            }
            Self::Icons => {
                let columns = end_columns(layout);
                let moving = if group { mates } else { vec![panel.clone()] };
                if let Some(column) = icon_column(&columns).and_then(|k| columns.get(k)) {
                    if column.groups.iter().any(|g| g.panels.contains(panel)) {
                        return Ok(Vec::new());
                    }
                    if let Some(anchor) = column.groups.iter().rev().flat_map(|g| &g.panels).find(|p| !moving.contains(p)) {
                        return Ok(vec![unit(Target::At { anchor: anchor.clone(), placement: Placement::Group { below: true, height: None } })]);
                    }
                }
                // No icon column: a new one inside the outer column, collapsed to icons.
                let to = match columns.iter().flat_map(|c| &c.groups).flat_map(|g| &g.panels).find(|p| !moving.contains(p)) {
                    Some(anchor) => Target::At { anchor: anchor.clone(), placement: Placement::Column { after: true } },
                    None => Target::NewDock { side: Side::End },
                };
                Ok(vec![unit(to), Action::SetIconic { panel: panel.clone(), iconic: true }])
            }
            Self::Column(place) => {
                let columns = end_columns(layout);
                let window = floating_window(layout, panel);
                // Its group, or alone in its group anyway: the group, or the whole floating
                // window it floats in (a stack of groups docks as one column).
                let whole = group || mates.len() == 1;
                let moving = match window {
                    Some(window) if whole => window_panels(window),
                    _ if whole => mates,
                    _ => vec![panel.clone()],
                };
                // The dock as it is without what moves: each column that keeps a panel, with one
                // of them as the anchor.
                let staying: Vec<(usize, &String)> = columns
                    .iter()
                    .enumerate()
                    .filter_map(|(k, c)| c.groups.iter().flat_map(|g| &g.panels).find(|p| !moving.contains(p)).map(|p| (k, p)))
                    .collect();
                // Place 1 is next to the icon column, inwards; without one, next to the outer column.
                let base = match icon_column(&columns) {
                    Some(icons) => staying.iter().filter(|(k, _)| *k <= icons).count(),
                    None => staying.len().min(1),
                };
                let at = base.saturating_add(place.saturating_sub(1)).min(staying.len());
                // What moves is a whole column at that place already.
                let alone = columns.iter().position(|c| {
                    let panels: Vec<&String> = c.groups.iter().flat_map(|g| &g.panels).collect();
                    panels.len() == moving.len() && panels.iter().all(|p| moving.contains(p))
                });
                if alone.is_some_and(|k| staying.iter().filter(|(j, _)| *j < k).count() == at) {
                    return Ok(Vec::new());
                }
                let to = match staying.get(at) {
                    Some((_, anchor)) => Target::At { anchor: (*anchor).clone(), placement: Placement::Column { after: false } },
                    None => {
                        let (_, anchor) = staying.last().ok_or("the window has no dock")?;
                        Target::At { anchor: (*anchor).clone(), placement: Placement::Column { after: true } }
                    }
                };
                Ok(vec![match window {
                    Some(_) if whole => Action::MoveWindow { panel: panel.clone(), to },
                    _ if whole => Action::MoveGroup { panel: panel.clone(), to },
                    _ => Action::MovePanel { panel: panel.clone(), to },
                }])
            }
        }
    }
}

/// Command params with VectorCraft's panel names resolved (a panel id or English name, in any
/// case).
fn resolve_panels(params: &Value) -> Result<Value, String> {
    let mut params = if params.is_null() { json!({}) } else { params.clone() };
    let fields = params.as_object_mut().ok_or("params must be an object")?;
    for key in ["panel", "anchor", "target", "before"] {
        let Some(value) = fields.get_mut(key) else { continue };
        if key == "before" && value.is_null() {
            continue;
        }
        let id = value.as_str().and_then(normalize).ok_or_else(|| format!("{key} must name a registered panel"))?;
        *value = Value::String(id.to_owned());
    }
    Ok(params)
}

/// A shared docking command (`window.panel.*`, `ui.dock`) on the layout in use, all or nothing.
/// craft-ui's `DockCommand` parses it and makes the layout actions; VectorCraft resolves panel
/// names, shows a hidden panel before moving, floating or docking it, floats a panel the layout
/// has no place for, and returns a floated panel to its dock place on Dock Panel.
fn shared_command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    shared_command_with(app, id, params, None, None)
}

/// [`shared_command`], with the panel put where `legacy` says instead of where the command puts
/// it, and its group or floating window then minimized or restored (`minimized`).
fn shared_command_with(app: &mut VectorcraftApp, id: &str, params: &Value, legacy: Option<Legacy>, minimized: Option<bool>) -> Result<Value, String> {
    let mut params = resolve_panels(params)?;
    if let Some(fields) = params.as_object_mut() {
        fields.insert("command".into(), Value::String(id.to_owned()));
    }
    let command: DockCommand<String> = serde_json::from_value(params).map_err(|e| e.to_string())?;
    match &command {
        DockCommand::Workspace { name } => return app.run("window.workspace", json!({"name": name})),
        DockCommand::NewWorkspace { name } => return app.run("window.workspace.new", json!({"name": name})),
        DockCommand::DeleteWorkspace { name } => return app.run("window.workspace.delete", json!({"name": name})),
        DockCommand::ResetWorkspace {} => return app.run("window.workspace.reset", json!({})),
        DockCommand::Lock { .. } => return Err("Lock Workspace is not available in VectorCraft".into()),
        DockCommand::Window { .. } => return Err("Move Panel to New Window is not available: floating panels stay in the main window".into()),
        _ => {}
    }
    // VectorCraft's docks are columns beside the canvas, so a dock at a window edge is a `side`,
    // not a frame on the outer edge (craft-ui's `Placement::Edge`).
    if let DockCommand::Dock { anchor: None, side: None, zone: Some(_), .. } = &command {
        return Err("zone needs an anchor: dock beside a panel with anchor, or in a new dock column with side (Start or End)".into());
    }
    let space = float_space(app);
    let mut txn = Transaction::begin(app)?;
    let shows = match &command {
        DockCommand::Activate { panel }
        | DockCommand::Move { panel, .. }
        | DockCommand::Float { panel, .. }
        | DockCommand::Dock { panel, .. }
        | DockCommand::UiDock(UiDock::Operation(DockOperation::Open { panel } | DockOperation::Activate { panel })) => Some(panel.clone()),
        _ => None,
    };
    let mut shown = false;
    if let Some(panel) = shows.as_ref().filter(|p| !txn.workspace.layout.contains(p)) {
        if txn.workspace.hidden.iter().any(|(p, _)| p == panel) {
            // Its recorded place, else beside the main column's panels when that place is gone.
            let fallback = first_dock_panel(&txn.workspace.layout, panel);
            txn.workspace.set_visible(panel.clone(), true, fallback, model_permissions).map_err(|e| e.to_string())?;
        } else {
            let docks = matches!(command, DockCommand::Dock { .. } | DockCommand::Move { .. });
            txn.place_new(panel, &space, docks)?;
        }
        shown = true;
    }
    let returned = match &command {
        DockCommand::Dock { panel, anchor: None, side: None, group, column: false, .. } if !shown && legacy.is_none() => {
            txn.dock_group_back(panel)? || (!group && txn.dock_back(panel)?)
        }
        _ => false,
    };
    let mut actions = match (&command, &legacy) {
        (DockCommand::Float { panel, group, .. } | DockCommand::Dock { panel, group, .. }, Some(legacy)) => {
            legacy.actions(&txn.workspace.layout, panel, *group)?
        }
        (_, Some(_)) => return Err("onto, above, below, icon and a column place belong to window.panel.float and window.panel.dock".into()),
        (command, None) => match command {
            // Showing it was the whole command (craft-ui's `DockCommand::apply` does the same).
            DockCommand::Dock { anchor: None, side: None, .. } if shown || returned => Vec::new(),
            DockCommand::Activate { .. } if shown => Vec::new(),
            _ => command.to_actions(&txn.workspace, false, &space).map_err(|e| e.to_string())?,
        },
    };
    if let (Some(minimized), Some(panel)) = (minimized, shows.as_ref()) {
        actions.push(Action::SetMinimized { panel: panel.clone(), minimized });
    }
    let reveal = match &command {
        DockCommand::Layout { action } | DockCommand::UiDock(UiDock::Action { action }) => match action {
            Action::Activate { panel } | Action::Open { panel, .. } | Action::OpenAt { panel, .. } => Some(panel.clone()),
            _ => None,
        },
        DockCommand::UiDock(UiDock::Operation(
            DockOperation::Float { panel, .. } | DockOperation::MoveFloating { panel, .. } | DockOperation::Move { panel, .. },
        )) => Some(panel.clone()),
        _ => shows,
    };
    let mut closed = Vec::new();
    for action in actions {
        closed.extend(txn.apply(action)?);
    }
    Ok(txn.commit(app, closed, reveal))
}

/// One action from the renderer: the user's own gesture.
fn gesture(app: &mut VectorcraftApp, action: Action<String>) -> Result<Value, String> {
    let mut txn = Transaction::begin(app)?;
    let closed = txn.apply(action)?;
    Ok(txn.commit(app, closed, None))
}

/// Where a command floats a panel, group or column it gives no rect: inside the dock area as
/// last drawn, else (headless, before any frame) inside [`HEADLESS_AREA`]. Seeded with every
/// panel's own size, so a panel the area never drew floats at that size too.
fn float_space(app: &VectorcraftApp) -> FloatSpace<String> {
    let space = app.dock_rt.space.clone().unwrap_or_else(|| FloatSpace::new(HEADLESS_AREA));
    crate::state::all_panels().fold(space, |space, (id, _)| space.with_panel_size(id.to_owned(), floating_size(id)))
}

/// Float `panel` (open or not) as a window of its own with its top-left corner at `at`, `width`
/// wide: the Tabs panel over the text it sets.
pub(crate) fn place_floating(app: &mut VectorcraftApp, panel: &str, at: egui::Pos2, width: f32) -> Result<(), String> {
    let panel = normalize(panel).ok_or("unknown panel")?.to_string();
    if !(at.x.is_finite() && at.y.is_finite() && width.is_finite()) {
        return Err("panel position must be finite".into());
    }
    let mut txn = Transaction::begin(app)?;
    if !txn.workspace.layout.contains(&panel) {
        txn.float_new(&panel, &FloatSpace::new(HEADLESS_AREA))?;
    }
    let height = floating_window(&txn.workspace.layout, &panel).map_or(floating_size(&panel).y, |w| w.rect[3]);
    let rect = [at.x, at.y, width.max(1.0), height];
    let whole = floating_window(&txn.workspace.layout, &panel).is_some_and(|w| w.panels().count() == 1);
    let action = if whole { Action::MoveFloating { panel, rect } } else { Action::Float { panel, rect } };
    txn.apply(action)?;
    txn.commit(app, Vec::new(), None);
    Ok(())
}

/// A change to the layout in use, committed only when every step succeeded.
struct Transaction {
    /// The layout, and the closed panels with where each one goes back.
    workspace: DockWorkspace<String>,
    /// Where each floating panel was docked before it floated, for Dock Panel.
    returns: BTreeMap<String, Location<String>>,
}

impl Transaction {
    fn begin(app: &VectorcraftApp) -> Result<Self, String> {
        if app.ui.docking.as_ref().is_some_and(|layout| !valid(layout)) {
            return Err("saved panel layout is invalid; reset the workspace first".into());
        }
        let mut workspace = DockWorkspace::new(layout_of(&app.ui));
        let mut returns = BTreeMap::new();
        for (panel, location) in app.ui.docking_hidden.iter().filter(|(id, _)| registered(id)).take(craft_ui::docking::MAX_PANELS) {
            if !workspace.layout.contains(panel) {
                workspace.hidden.push((panel.clone(), location.clone()));
            } else if workspace.layout.floating.iter().any(|w| w.contains(panel)) {
                returns.insert(panel.clone(), location.clone());
            }
        }
        Ok(Self { workspace, returns })
    }

    /// Show `panel`, which has no recorded place: in its icon group when a panel of that
    /// [`ICON_PANEL_GROUPS`] row is in an icon column, else beside the main column's panels when
    /// it `docks`, else floating.
    fn place_new(&mut self, panel: &String, space: &FloatSpace<String>, docks: bool) -> Result<(), String> {
        let layout = &self.workspace.layout;
        let mate = ICON_PANEL_GROUPS
            .iter()
            .find(|row| row.contains(&panel.as_str()))
            .and_then(|row| row.iter().find(|p| **p != panel.as_str() && in_iconic_column(layout, p)));
        let action = match (mate, docks) {
            (Some(mate), _) => Action::OpenAt { panel: panel.clone(), anchor: (*mate).to_string(), placement: Placement::Tab { before: None } },
            (None, true) => Action::Open { panel: panel.clone(), anchor: first_dock_panel(layout, panel) },
            (None, false) => return self.float_new(panel, space),
        };
        self.apply(action).map(|_| ())
    }

    /// Add `panel`, which is in no group, as a floating window of its own where `space` floats
    /// a panel, each further window 24 points lower right.
    fn float_new(&mut self, panel: &String, space: &FloatSpace<String>) -> Result<(), String> {
        let size = floating_size(panel);
        let step = 24.0 * self.workspace.layout.floating.len().min(12) as f32;
        let at = space.area.min + egui::vec2(24.0 + step, 24.0 + step);
        let mut layout = self.workspace.layout.clone();
        layout.floating.push(Floating::single(vec![panel.clone()], 0, [at.x, at.y, size.x, size.y]));
        layout.validate().map_err(|e| e.to_string())?;
        self.workspace.layout = layout;
        self.workspace.hidden.retain(|(p, _)| p != panel);
        Ok(())
    }

    /// Dock Panel for a floating panel that floated from the dock: back to that place, while
    /// the panel it was beside is still docked. True when it went back.
    fn dock_back(&mut self, panel: &String) -> Result<bool, String> {
        let layout = &self.workspace.layout;
        if !layout.floating.iter().any(|w| w.contains(panel)) {
            return Ok(false);
        }
        let docked = Layout { root: layout.root.clone(), floating: Vec::new() };
        let Some(location) = self
            .returns
            .get(panel)
            .filter(|location| location.floating.is_none() && location.anchor.as_ref().is_none_or(|anchor| docked.contains(anchor)))
        else {
            return Ok(false);
        };
        let mut candidate = layout.clone();
        apply(&mut candidate, Action::Close { panel: panel.clone() })?;
        if candidate.restore(panel.clone(), location).is_err() {
            return Ok(false);
        }
        self.workspace.layout = candidate;
        self.returns.remove(panel);
        Ok(true)
    }

    /// Dock Panel for a panel of a floating group that floated from the dock as one: the whole
    /// group back to that place, beside the same neighbour while it is still docked. True when it
    /// went back.
    fn dock_group_back(&mut self, panel: &String) -> Result<bool, String> {
        let layout = &self.workspace.layout;
        let Some(window) = layout.floating.iter().find(|w| w.contains(panel)) else {
            return Ok(false);
        };
        let members = window_panels(window);
        let Some(location) = self.returns.get(panel).cloned() else {
            return Ok(false);
        };
        // A group of one docks back as a panel. Every member keeps the same group place only
        // while the group is the one that floated.
        if members.len() < 2 || !members.iter().all(|p| self.returns.get(p) == Some(&location)) {
            return Ok(false);
        }
        let docked = Layout { root: layout.root.clone(), floating: Vec::new() };
        let Some(anchor) = location.anchor.clone().filter(|anchor| docked.contains(anchor)) else {
            return Ok(false);
        };
        if !matches!(location.placement, Placement::Group { .. } | Placement::Column { .. }) {
            return Ok(false);
        }
        let mut candidate = layout.clone();
        let to = Target::At { anchor, placement: location.placement.clone() };
        if apply(&mut candidate, Action::MoveGroup { panel: panel.clone(), to }).is_err() {
            return Ok(false);
        }
        if let Some(saved) = &location.group {
            apply(&mut candidate, Action::ResizeGroup { panel: panel.clone(), height: saved.height })?;
            apply(&mut candidate, Action::SetMinimized { panel: panel.clone(), minimized: saved.minimized })?;
        }
        self.workspace.layout = candidate;
        for member in &members {
            self.returns.remove(member);
        }
        Ok(true)
    }

    /// One action from a command or the renderer, for registered panels only. Floating a
    /// docked panel, or its whole group, remembers its place for Dock Panel.
    fn apply(&mut self, action: Action<String>) -> Result<Vec<String>, String> {
        if !known(&action) {
            return Err("docking action contains an unknown panel or an excessive split path".into());
        }
        if let Action::Float { panel, .. } = &action
            && let Ok(location) = self.workspace.layout.location(panel)
            && location.floating.is_none()
        {
            self.returns.insert(panel.clone(), location);
        }
        if let Action::MoveGroup { panel, to: Target::Float { .. } } = &action
            && let Some((members, location)) = docked_group_place(&self.workspace.layout, panel)
        {
            for member in members {
                self.returns.insert(member, location.clone());
            }
        }
        self.workspace.apply(action, model_permissions).map_err(|e| e.to_string())
    }

    /// Store the result. `reveal` is the panel the command showed: its flyout opens, or its
    /// floating window comes forward on the next frame.
    fn commit(self, app: &mut VectorcraftApp, closed: Vec<String>, reveal: Option<String>) -> Value {
        let Self { workspace, returns } = self;
        let layout = workspace.layout;
        let mut hidden: BTreeMap<String, Location<String>> = workspace.hidden.into_iter().collect();
        for (panel, location) in returns {
            if layout.floating.iter().any(|w| w.contains(&panel)) {
                hidden.entry(panel).or_insert(location);
            }
        }
        let reveal = reveal.filter(|panel| layout.contains(panel));
        let flyout = reveal.as_ref().filter(|panel| in_iconic_column(&layout, panel)).cloned();
        match (app.ui.docking.is_none() && hidden.is_empty()).then(|| default_flags(&app.ui, &layout)).flatten() {
            Some((tab, collapsed, width)) => {
                app.ui.dock_tab = tab;
                app.ui.dock_collapsed = collapsed;
                app.ui.dock_width = width;
            }
            None => {
                // `dock_tab` names the main column's front panel in a customized layout too.
                if let Some(tab) = dock_columns(&layout)
                    .first()
                    .and_then(|c| c.groups.first())
                    .and_then(|g| g.panels.get(g.active))
                    .and_then(|p| DockTab::from_id(p))
                {
                    app.ui.dock_tab = tab;
                }
                app.ui.docking = Some(layout.clone());
                app.ui.docking_hidden = hidden;
            }
        }
        for slot in [&mut app.ui.open_panel, &mut app.dock_rt.raise] {
            if slot.as_ref().is_some_and(|panel| closed.contains(panel)) {
                *slot = None;
            }
        }
        if let Some(panel) = reveal {
            app.ui.dock = true;
            match flyout {
                Some(panel) => app.ui.open_panel = Some(panel),
                None => app.dock_rt.raise = Some(panel),
            }
        }
        // A flyout opens only from an iconic column.
        if app.ui.open_panel.as_deref().is_some_and(|panel| !in_iconic_column(&layout, panel)) {
            app.ui.open_panel = None;
        }
        let floating: Vec<String> = layout.floating.iter().flat_map(|w| w.panels().cloned()).collect();
        json!({"layout": layout, "closed": closed, "floating": floating})
    }
}

/// The panels of the docked group holding `panel`, and that group's place as a whole: beside
/// the group or column next to it, with its group and column state. None when `panel` is not
/// in a dock.
fn docked_group_place(layout: &Layout<String>, panel: &String) -> Option<(Vec<String>, Location<String>)> {
    let members = dock_columns(layout).into_iter().flat_map(|c| &c.groups).find(|g| g.panels.contains(panel))?.panels.clone();
    // The place of the group's last panel left in it is the group's own place.
    let mut alone = layout.clone();
    for other in members.iter().filter(|p| *p != panel) {
        apply(&mut alone, Action::Close { panel: other.clone() }).ok()?;
    }
    let location = alone.location(panel).ok().filter(|l| l.floating.is_none())?;
    Some((members, location))
}

/// Whether every panel an action names is registered (and a split path is not excessive).
/// The application owns the registry; craft-ui owns tree validation and structural changes.
fn known(action: &Action<String>) -> bool {
    let registered = |id: &String| self::registered(id);
    let tab = |placement: &Placement<String>| match placement {
        Placement::Tab { before: Some(id) } => registered(id),
        _ => true,
    };
    match action {
        Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } => {
            registered(panel) && registered(anchor) && tab(placement)
        }
        Action::Open { panel, anchor } => registered(panel) && anchor.as_ref().is_none_or(registered),
        Action::Float { panel, .. }
        | Action::Close { panel }
        | Action::Activate { panel }
        | Action::MoveFloating { panel, .. }
        | Action::SetStackOpen { panel, .. }
        | Action::ResizeStack { panel, .. } => registered(panel),
        Action::ResizeSplit { path, .. } => path.len() <= craft_ui::docking::MAX_DEPTH,
        Action::ResizeWindowSplit { panel, path, .. } => registered(panel) && path.len() <= craft_ui::docking::MAX_DEPTH,
        Action::MovePanel { panel, to } | Action::MoveGroup { panel, to } | Action::MoveColumn { panel, to } | Action::MoveWindow { panel, to } => {
            registered(panel)
                && match to {
                    Target::At { anchor, placement } => registered(anchor) && tab(placement),
                    _ => true,
                }
        }
        Action::CloseGroup { panel }
        | Action::CloseWindow { panel }
        | Action::CloseOthers { panel }
        | Action::SetMinimized { panel, .. }
        | Action::SetIconic { panel, .. }
        | Action::ResizeColumn { panel, .. }
        | Action::ResizeGroup { panel, .. }
        | Action::ResizeIconic { panel, .. }
        | Action::SetNative { panel, .. }
        | Action::SetMonitor { panel, .. } => registered(panel),
        // Actions newer than this list are rejected until VectorCraft checks them.
        _ => false,
    }
}

/// Draw the canvas and the panels in the space left by the bars and the Tools panel, and apply
/// what the user did to them.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let area = area_id();
    let ctx = ui.ctx().clone();
    let changed_workspace = ctx.data_mut(|data| {
        let previous = data.get_temp::<u64>(area.with("generation"));
        data.insert_temp(area.with("generation"), app.ui.docking_generation);
        previous.is_some_and(|generation| generation != app.ui.docking_generation)
    });
    if changed_workspace {
        craft_ui::docking::cancel_drag::<String>(&ctx, area);
    }
    if app.ui.docking.as_ref().is_some_and(|layout| !valid(layout)) {
        app.ui.docking = None;
        app.status(tl!("Invalid panel layout; restored the default dock").to_string());
    }
    let mut layout = layout_of(&app.ui);
    // `open_panel` is the open flyout: a command or a workspace that sets it opens that flyout,
    // showing that panel of its group.
    let flyout = craft_ui::docking::flyout::<String>(&ctx, area);
    if flyout != app.ui.open_panel {
        match app.ui.open_panel.clone() {
            Some(panel) if in_iconic_column(&layout, &panel) => {
                if app.ui.docking.is_some() && layout.panel_state(&panel) != craft_ui::docking::PanelState::Front {
                    if let Err(error) = gesture(app, Action::Activate { panel: panel.clone() }) {
                        app.ui.status = error;
                    }
                    layout = layout_of(&app.ui);
                }
                craft_ui::docking::open_flyout(&ctx, area, panel);
            }
            Some(_) => {
                app.ui.open_panel = None;
                craft_ui::docking::close_flyout::<String>(&ctx, area);
            }
            None => craft_ui::docking::close_flyout::<String>(&ctx, area),
        }
    }
    let modal = modal_open(app, &ctx);
    let mut blocked = Vec::new();
    if modal || egui::Popup::is_any_open(&ctx) {
        blocked.push(ctx.content_rect());
    }
    let style = dock_style(ui);
    let mut dock = craft_ui::docking::DockArea::new(area)
        .blocked_rects(&blocked)
        .input_blocked(modal)
        .auto_collapse_iconic(app.session.prefs.auto_collapse_icon_panels);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(hook) = app.services.native_input_hook.clone() {
        dock = dock.native_input_hook(move |_, raw| hook(raw));
    }
    // Tab and Shift+Tab (`window.dock`, `window.panels`) and the full screen modes hide the panels;
    // the canvas stays.
    if !app.ui.dock || app.ui.screen_mode >= 3 {
        dock = dock.hide_panels(HidePanels::All);
    }
    if let Some(panel) = app.dock_rt.raise.take() {
        dock = dock.raise_panel(panel);
    }
    let mut output = dock.show_content(ui, &layout, &style, &mut PanelContent { app: &mut *app });
    app.dock_rt.space = craft_ui::docking::float_space(&ctx, area);
    if let Some(panel) = output.focused.take() {
        app.dock_rt.focused = Some(panel);
    }
    for event in std::mem::take(&mut output.events) {
        match event {
            DockEvent::ToggleAutoCollapse => app.session.prefs.auto_collapse_icon_panels = !app.session.prefs.auto_collapse_icon_panels,
            DockEvent::FlyoutChanged { panel } => app.ui.open_panel = panel,
            _ => {}
        }
    }
    if let Some(error) = output.error {
        app.ui.status = error.to_string();
    }
    for action in output.actions {
        if let Err(error) = gesture(app, action) {
            app.ui.status = error;
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    for window in output.window_files {
        // The keyboard's Shift as the main window last saw it: the window system sends a file
        // drag no key events.
        let shift = ctx.input(|i| i.modifiers.shift);
        app.open_dropped_files(window.dropped, None, shift);
    }
    forward_keys(app, &ctx, output.forwarded_events, output.forwarded_modifiers);
}

/// Whether a modal is open: a dialog, About, the command palette, a native file dialog, or a
/// window behind the modal backdrop last frame (export progress, plug-in runs). Floating panels
/// in OS windows of their own take no input meanwhile, as the main window behind it takes none.
pub(crate) fn modal_open(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    app.ui.dialog.is_some()
        || app.ui.about
        || app.ui.palette_open
        || app.file_dialog_open()
        || ctx.memory(|m| m.areas().is_visible(&crate::dialogs::modal::backdrop()))
}

/// Keys, text and clipboard events that a floating panel's own OS window left over run the main
/// window's shortcuts, tool keys and clipboard on its next frame ([`VectorcraftApp::raw_input_hook`]).
/// craft-ui forwards nothing while a field in that window has the keyboard. A field of the main
/// window loses it, as a click elsewhere in one window would take it.
fn forward_keys(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, modifiers: Option<egui::Modifiers>) {
    if events.is_empty() {
        return;
    }
    // Only key releases come while a modal is open; its field keeps the keyboard.
    if events.iter().any(|e| !matches!(e, egui::Event::Key { pressed: false, .. })) {
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    app.forwarded.extend(events);
    app.forwarded_modifiers = modifiers.or(app.forwarded_modifiers);
    ctx.request_repaint();
}

/// VectorCraft's look and wording of craft-ui's docking.
fn dock_style(ui: &egui::Ui) -> craft_ui::docking::DockStyle {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut style = craft_ui::docking::DockStyle::from_ui(ui);
    style.tab_height = 33.0;
    style.background = t.panel;
    style.tab_background = t.tab_strip;
    style.active_background = t.button;
    style.text = t.text_strong;
    style.inactive_text = t.text_dim;
    style.border = egui::Stroke::new(1.0, t.border);
    style.accent = t.accent;
    style.blocked_dim = t.panel.gamma_multiply(0.6);
    // New docks at the docked area's free sides but no full-height column from the window's left
    // or right edge; a drop on no target (the canvas) floats.
    style.side_edge_zones = false;
    style.new_dock_zones = true;
    style.float_on_empty_drop = true;
    style.localized(|key| DOCK_LABELS.iter().find(|(k, _)| *k == key).map(|(_, english)| tl!(english).to_owned()))
}

/// VectorCraft's wording of craft-ui's docking labels, translated through `tl!`.
const DOCK_LABELS: &[(&str, &str)] = &[
    ("dock.float", "Float Panel"),
    ("dock.close", "Close Panel"),
    ("dock.movePanelTo", "Group with"),
    ("dock.panels", "Panels"),
    ("dock.resize", "Resize panels"),
    ("dock.resizeWindow", "Resize panel window"),
    ("dock.collapseToIcons", "Collapse to Icons"),
    ("dock.expandPanels", "Expand Panels"),
    ("dock.collapseColumn", "Collapse to Icons"),
    ("dock.expandColumn", "Expand Panels"),
];

/// The size a panel floats at (tab strip included), whatever the size of the dock it left.
fn floating_size(panel: &str) -> egui::Vec2 {
    match panel {
        "properties" | "layers" | "libraries" => egui::vec2(300.0, 420.0),
        _ => egui::vec2(260.0, 360.0),
    }
}

/// The panels' content, labels and icons.
struct PanelContent<'a> {
    app: &'a mut VectorcraftApp,
}

impl craft_ui::docking::DockContent<String> for PanelContent<'_> {
    fn body(&mut self, ui: &mut egui::Ui, id: &String) {
        if id == CANVAS {
            self.app.canvas_area(ui);
            return;
        }
        if ui.ctx().viewport_id() != egui::ViewportId::ROOT {
            crate::canvas::release_art_in_window(self.app, ui.ctx());
        }
        let t = crate::theme::Tokens::get(ui.ctx());
        let back = ui.max_rect();
        if back.is_positive() {
            ui.painter().rect_filled(back, 0.0, t.panel);
        }
        let width = ui.available_width();
        let height = (ui.available_height() - 20.0).max(0.0);
        if DockTab::from_id(id).is_some() {
            crate::dock::panel_body(self.app, ui, id, width, height);
        } else {
            // Shrinking to the content's height lets craft-ui fit a float or flyout to it.
            crate::widgets::panel_scroll(ui, egui::ScrollArea::vertical().id_salt(("shared-panel-body", id)).auto_shrink([false, true]), |ui| {
                let width = ui.available_width();
                crate::dock::panel_body(self.app, ui, id, width, height)
            });
        }
    }

    fn logical_label(&self, id: &String) -> String {
        if id == CANVAS {
            return tl!("Canvas").to_string();
        }
        crate::panels::library_panel::tab_label(self.app, id)
            .or_else(|| crate::state::all_panels().find(|p| p.0 == id).map(|p| tl!(p.1).to_string()))
            .unwrap_or_else(|| id.clone())
    }

    fn permissions(&self, id: &String) -> Permissions {
        model_permissions(id)
    }

    fn limits(&self, id: &String) -> craft_ui::docking::PanelLimits {
        let min = if id == CANVAS { egui::vec2(200.0, 120.0) } else { egui::vec2(220.0, 80.0) };
        craft_ui::docking::PanelLimits { min, ..Default::default() }
    }

    fn floating_size(&self, id: &String) -> Option<egui::Vec2> {
        Some(floating_size(id))
    }

    fn paint_icon(&mut self, painter: &egui::Painter, rect: egui::Rect, id: &String, color: egui::Color32) -> bool {
        let icon = crate::state::rail_panel(id).map(|p| p.2).or_else(|| crate::panels::library_panel::parse(id).map(|_| "library"));
        let Some(icon) = icon else { return false };
        let image = egui::Image::new(crate::icons::source(icon));
        match image.load_for_size(painter.ctx(), rect.size()) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                painter.image(texture.id, rect, uv, color);
                true
            }
            _ => false,
        }
    }

    fn panel_menu(&mut self, ui: &mut egui::Ui, id: &String) {
        crate::panels::panel_menu_items(self.app, ui, id);
    }

    fn group_header_width(&self, _panels: &[String], _active: &String, _floating: bool, _strip_width: f32) -> f32 {
        28.0
    }

    fn group_header(&mut self, ui: &mut egui::Ui, _panels: &[String], active: &String, _floating: bool) {
        let rect = egui::Rect::from_center_size(ui.max_rect().center(), egui::vec2(16.0, 16.0));
        crate::panels::panel_menu(self.app, ui, active, rect);
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests;
