//! The window's panels, tool rails and the canvas as one workspace of the framework dock.
//!
//! `ui.dock_workspace` holds the layout and where closed and floated panels go back. Its default
//! ([`default_workspace`]) is the Tools rail docked at the window's start edge, the canvas, and a
//! dock at the end edge: the Properties · Layers · Libraries column, then the icon column (one
//! group per row of [`ICON_PANEL_GROUPS`]). Floating panels open in OS windows of their own where
//! the platform has them and inside the main window elsewhere; the framework decides.
//!
//! The dock area draws the workspace and applies the user's gestures to it ([`show`]). Commands
//! change it through [`apply`], which hands the change to the area when a body runs the command in
//! the area's pass and applies it at once otherwise. Tab and Shift+Tab hide the panels through the
//! window's shell ([`panels_shown`], [`show_panels`]).

use craft::behavior::dock::{
    Action, Column, DockEdge, DockPlacement, Floating, Group, Layout, MAX_PANELS, Node, PanelPlace, PanelState, Permissions, SplitSize, Target,
    Workspace, node_at,
};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use craft::behavior::{Axis, DockSide, IconId};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, BodyCx, PlainData, Show as _, Tracked};
use craft::ui::docking::{
    ColumnCards, DockCommands, DockConfig, DockEvent, DockGrid, DockPanels, DockScene, DockSizes, DockTo, DragUnit, FloatSpace, GroupAt, HidePanels,
    PanelKey, PanelSpec, PanelSpecs, actions as command_actions, move_actions, request_actions,
};
use craft::ui::menu::PanelRow;
use craft::ui::patterns::dock::DockArea;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::frame::{ChromeToggle, request_chrome_toggle};
use craft::ui::shell::chrome_state;
use craft::ui::tabs::PanelTabLook;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::i18n::LangExt as _;
use crate::state::{DockTab, ICON_PANEL_GROUPS, UiState};
use crate::toolbar::Owner;

/// The panel that holds the document area. It never moves, floats, closes or takes tabs.
pub(crate) const CANVAS: &str = "canvas";
/// The narrowest a panel other than the canvas is drawn, docked or floating.
pub(crate) const PANEL_MIN_WIDTH: f32 = 220.0;

// PLAN: the sheet deletes HEADLESS_AREA, but a command run before the dock first drew (headless, or before the first frame) still needs an area to float into.
/// The area a command floats into before the dock area ever drew.
const HEADLESS_AREA: egui::Rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1400.0, 900.0));

/// The share a split beside a dock records. A dock's width comes from its columns, so the split never reads it.
const BESIDE_DOCK: SplitSize = SplitSize::Ratio(0.5);

/// Docking state that lives for the session only.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Where the dock area floats panels, as it was last drawn.
    space: Option<FloatSpace<String>>,
    /// A panel a command showed: its floating window comes forward on the next frame.
    raise: Option<String>,
    /// The panel the user last clicked or pressed in (a tab, a panel body, a drop).
    focused: Option<String>,
    /// Where each dock column was placed last pass, named by a panel of its first group.
    columns: Vec<(String, egui::Rect)>,
    /// The OS windows the floating windows showed in last pass, each named by its first panel.
    windows: Vec<(String, egui::ViewportId)>,
    /// What the dock reads of the panels, kept up to date by [`refresh_specs`].
    specs: Tracked<Specs>,
    /// The tool rails of the layout, which Shift+Tab keeps shown, kept up to date by [`refresh_specs`].
    rails: Tracked<BTreeSet<PanelKey>>,
    /// What each tool rail of the layout reads across passes, by its panel.
    pub(crate) rail_models: HashMap<String, crate::toolbar::RailModel>,
    /// A panel just floated out of a tool rail, whose window takes over the held press on the next
    /// pass, with the pointer at this point inside it ([`continue_move`]).
    hand_over: Option<(String, egui::Vec2)>,
}

/// `ui.docking`: the layout in use, the hidden panels, the open flyout, the focused panel, the
/// Auto-Collapse preference, whether the panels show, the window edge whose panels are revealed
/// while they are hidden, and where the dock columns and floating windows were last drawn.
pub(crate) fn inspect(app: &VectorcraftApp, ctx: &egui::Context) -> Value {
    let workspace = &app.ui.dock_workspace;
    let layout = workspace.layout();
    let rect = |r: &egui::Rect| [r.left(), r.top(), r.width(), r.height()];
    let peek = chrome_state(ctx, egui::ViewportId::ROOT).revealed.and_then(peek::Edge::of_reveal).map(peek::Edge::id);
    let columns: Vec<Value> = app.dock_rt.columns.iter().map(|(panel, r)| json!({"panel": panel, "rect": rect(r)})).collect();
    let windows: Vec<Value> = app.dock_rt.windows.iter().map(|(panel, viewport)| json!({"panel": panel, "viewport": viewport.0.value()})).collect();
    json!({
        "layout": layout,
        "hidden": workspace.hidden().iter().map(|(panel, _)| panel).collect::<Vec<_>>(),
        "flyout": app.ui.open_panel,
        "focused": app.dock_rt.focused.as_ref().filter(|p| layout.contains(p)),
        "autoCollapse": app.session.prefs.auto_collapse_icon_panels,
        "panelsShown": panels_shown(app),
        "peek": peek,
        "columns": columns,
        "windows": windows,
    })
}

fn normalize(raw: &str) -> Option<&str> {
    crate::menus::normalize_panel(raw)
}

/// Is `id` a tool rail: the Tools panel, a toolbar made with New Toolbar, or a group torn off
/// either of them?
fn is_rail(id: &str) -> bool {
    id == crate::toolbar::TOOLS || id.starts_with("toolbar:") || id.starts_with("strip:")
}

/// Does the app know panel `id`: the canvas, a Window-menu panel, a tool rail or a library panel?
/// Rail and library keys are made at run time, so they are known by their form.
pub(crate) fn registered(id: &str) -> bool {
    id == CANVAS || is_rail(id) || crate::panels::library_panel::parse(id).is_some() || normalize(id) == Some(id)
}

/// The registered id `raw` names: a Window-menu panel by its id or English name in any case, or a
/// panel id made at run time.
fn panel_id(raw: &str) -> Option<String> {
    normalize(raw).map(str::to_owned).or_else(|| registered(raw).then(|| raw.to_owned()))
}

/// What may happen to panel `id`: the canvas is protected; a tool rail neither collapses to icons
/// nor shares its column; any other panel may do everything.
fn permissions(id: &str) -> Permissions {
    if id == CANVAS {
        Permissions::PROTECTED
    } else if is_rail(id) {
        Permissions::ALL.collapses(false).shares_column(false)
    } else {
        Permissions::ALL
    }
}

/// The rail panel `id` draws: the Tools panel, a toolbar of `ui.custom_toolbars`, or a group torn
/// off one of them.
fn rail_owner(ui: &UiState, id: &str) -> Option<Owner> {
    rail_owner_among(ui.custom_toolbars.iter().map(|bar| bar.id), id)
}

// PLAN: the strip key's form belongs to `toolbar::strip_id` (chrome-w10), which offers no parse; the owner is found by building each candidate's key with `Owner::panel`.
/// [`rail_owner`] among the toolbars with ids `toolbars`.
fn rail_owner_among(toolbars: impl Iterator<Item = u32>, id: &str) -> Option<Owner> {
    if !is_rail(id) {
        return None;
    }
    let first = id.rsplit(':').next().unwrap_or_default().to_owned();
    std::iter::once(Owner::Tools)
        .chain(toolbars.map(Owner::Toolbar))
        .flat_map(|parent| [Owner::Strip(Box::new(parent.clone()), first.clone()), parent])
        .find(|owner| owner.panel() == id)
}

/// The workspace a fresh window starts with: the Tools rail one column wide at the start edge, the
/// canvas, and at the end edge the Properties · Layers · Libraries column and the icon column.
/// Each of `fronts` comes to the front of the group that holds it.
pub(crate) fn default_workspace(fronts: &[&str]) -> Workspace<String> {
    let mut placed: Vec<String> = Vec::new();
    let mut group = |ids: &[&str]| {
        let fresh: Vec<String> = ids.iter().filter(|id| registered(id) && !placed.iter().any(|p| p == *id)).map(|id| (*id).to_owned()).collect();
        placed.extend(fresh.iter().cloned());
        let active = fresh.iter().position(|p| fronts.contains(&p.as_str())).unwrap_or(0);
        (!fresh.is_empty()).then(|| Group::new(fresh, active))
    };
    let main: Vec<&str> = DockTab::ALL.iter().map(|tab| tab.info().0).collect();
    let mut columns: Vec<Column<String>> = group(main.as_slice()).into_iter().map(|g| Column::new(vec![g])).collect();
    let icons: Vec<Group<String>> = ICON_PANEL_GROUPS.iter().filter_map(|row| group(row)).collect();
    if !icons.is_empty() {
        columns.push(Column::new(icons).with_iconic(true));
    }
    let tools = Column::new(vec![Group::new(vec![crate::toolbar::TOOLS.to_owned()], 0)]).with_steps(1);
    let centre = Node::split(Axis::X, BESIDE_DOCK, Node::tabs(vec![CANVAS.to_owned()], 0), Node::dock(DockEdge::End, columns));
    let root = Node::split(Axis::X, BESIDE_DOCK, Node::dock(DockEdge::Start, vec![tools]), centre);
    Workspace::new(Layout::new(Some(root), Vec::new()))
}

/// Fits a loaded or switched-to workspace to the panels the app knows now ([`registered`]):
/// unknown panels go back to hidden, missing default panels come back, the canvas always shows.
pub(crate) fn reconcile(ws: &mut Arc<Workspace<String>>) {
    Arc::make_mut(ws).reconcile(|p: &String| registered(p), |p: &String| permissions(p), &default_workspace(&[]));
}

/// Changes the workspace by `actions`, all or none. While a dock pass runs on `ctx` (a panel body
/// ran the command) the area takes them and applies them when its pass ends, and this returns no
/// closed panels; otherwise they apply at once and this returns the panels they closed.
pub(crate) fn apply(app: &mut VectorcraftApp, ctx: Option<&egui::Context>, actions: Vec<Action<String>>) -> Result<Vec<String>, String> {
    let actions = match ctx {
        Some(ctx) => match request_actions(ctx, actions) {
            Ok(()) => return Ok(Vec::new()),
            Err(actions) => actions,
        },
        None => actions,
    };
    let closed = Arc::make_mut(&mut app.ui.dock_workspace).apply_all(actions, false, |p: &String| permissions(p)).map_err(|e| e.to_string())?;
    settle(app);
    Ok(closed)
}

/// Drops what no longer fits the layout after a change: F-key group memory of panels shown again,
/// a flyout of a panel no longer in a column collapsed to icons, a raise of a panel now closed and
/// the models of closed rails.
fn settle(app: &mut VectorcraftApp) {
    let ui = &mut app.ui;
    let ws = &ui.dock_workspace;
    for group in &mut ui.docking_hidden_groups {
        group.retain(|panel| ws.hidden().iter().any(|(hidden, _)| hidden == panel));
    }
    ui.docking_hidden_groups.retain(|group| !group.is_empty());
    if ui.open_panel.as_ref().is_some_and(|panel| !ws.layout().opens_in_flyout(panel)) {
        ui.open_panel = None;
    }
    let layout = app.ui.dock_workspace.layout();
    if app.dock_rt.raise.as_ref().is_some_and(|panel| !layout.contains(panel)) {
        app.dock_rt.raise = None;
    }
    app.dock_rt.rail_models.retain(|panel, _| layout.contains(panel));
}

/// Do the panels show: neither Tab nor Shift+Tab hides them in the main window, as its shell's
/// last pass left it, and Presentation Mode is off? With no window yet nothing hides them.
pub(crate) fn panels_shown(app: &VectorcraftApp) -> bool {
    let chrome = app.ui_ctx.as_ref().map(|ctx| chrome_state(ctx, egui::ViewportId::ROOT)).unwrap_or_default();
    !chrome.hidden && !chrome.panels_hidden && app.ui.screen_mode < 3
}

/// Whether Tab hides the main window's panels, toolbars and Control bar (Shift+Tab leaves the
/// toolbars).
pub(crate) fn tab_hidden(app: &VectorcraftApp) -> bool {
    app.ui_ctx.as_ref().is_some_and(|ctx| chrome_state(ctx, egui::ViewportId::ROOT).hidden)
}

/// Ends Tab and Shift+Tab hiding the panels in the main window; nothing while they show.
pub(crate) fn show_panels(app: &VectorcraftApp) {
    let Some(ctx) = &app.ui_ctx else { return };
    let chrome = chrome_state(ctx, egui::ViewportId::ROOT);
    if chrome.hidden || chrome.panels_hidden {
        request_chrome_toggle(ctx, egui::ViewportId::ROOT, ChromeToggle::ShowPanels);
    }
}

/// The layout in use.
pub(crate) fn layout_of(ui: &UiState) -> &Layout<String> {
    ui.dock_workspace.layout()
}

/// Every dock column of panels, from each dock's window edge inwards. Tool rail columns are left
/// out: no panel joins them and they never collapse to icons.
fn dock_columns(layout: &Layout<String>) -> Vec<&Column<String>> {
    let mut out = Vec::new();
    let mut visit: Vec<&Node<String>> = layout.root.iter().collect();
    let mut seen = 0usize;
    while let Some(node) = visit.pop() {
        seen = seen.saturating_add(1);
        if seen > 2 * MAX_PANELS {
            break;
        }
        match node {
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            Node::Dock { columns, .. } => out.extend(columns.iter().filter(|c| !c.panels().any(|p| is_rail(p)))),
            _ => {}
        }
    }
    out
}

/// The columns of the dock on `edge`, from the window edge inwards; none when that edge has no dock.
fn edge_columns(layout: &Layout<String>, edge: DockEdge) -> &[Column<String>] {
    let dock = layout.dock_path(edge).and_then(|path| node_at(layout.root.as_ref()?, &path));
    match dock {
        Some(Node::Dock { columns, .. }) => columns,
        _ => &[],
    }
}

/// A dock column's window edge and its index from that edge; `None` for a floating window's column.
pub(crate) type DockSlot = Option<(DockEdge, usize)>;

/// The column holding `panel`, with its [`DockSlot`].
pub(crate) fn column_of<'a>(layout: &'a Layout<String>, panel: &str) -> Option<(DockSlot, &'a Column<String>)> {
    let place = layout.place(&panel.to_owned())?;
    let column = layout.column(&place)?;
    let edge = match &place {
        PanelPlace::Dock { path, column, .. } => match layout.root.as_ref().and_then(|root| node_at(root, path)) {
            Some(Node::Dock { side, .. }) => Some((*side, *column)),
            _ => None,
        },
        _ => None,
    };
    Some((edge, column))
}

/// Is the dock's outer column of panels (Properties · Layers · Libraries) collapsed to icons?
pub(crate) fn dock_collapsed(ui: &UiState) -> bool {
    dock_columns(layout_of(ui)).first().is_some_and(|c| c.iconic)
}

/// A floating window's panels, first to last.
pub(crate) fn window_panels(window: &Floating<String>) -> Vec<String> {
    window.panels().cloned().collect()
}

/// The floating window holding `panel`.
pub(crate) fn floating_window<'a>(layout: &'a Layout<String>, panel: &str) -> Option<&'a Floating<String>> {
    layout.floating.iter().find(|w| w.contains(panel))
}

/// The top left a new floating window `height` tall takes beside the window floated last: a cascade
/// step from it, inside the float space. None while nothing floats.
pub(crate) fn cascaded_float_origin(app: &VectorcraftApp, height: f32) -> Option<egui::Pos2> {
    let last = layout_of(&app.ui).floating.last()?.rect;
    let space = command_space(app);
    let size = egui::vec2(space.default_size.x, height);
    Some(craft::behavior::placement::cascade(last, size, space.area, space.inset).min)
}

/// Is `panel` showing: in front of its group and not in a column collapsed to icons, or in the
/// open flyout of an iconic column? The Window menu checks these.
pub(crate) fn showing(ui: &UiState, panel: &str) -> bool {
    let layout = layout_of(ui);
    let id = panel.to_owned();
    if layout.opens_in_flyout(&id) { ui.open_panel.as_deref() == Some(panel) } else { layout.panel_state(&id) == PanelState::Front }
}

/// Can `panel` be seen: [`showing`] while the panels show ([`panels_shown`])? The Window menu's
/// check marks follow this.
pub(crate) fn visible(app: &VectorcraftApp, panel: &str) -> bool {
    panels_shown(app) && showing(&app.ui, panel)
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

/// The docking commands VectorCraft handles here: `window.panel` (the Window menu), its own
/// `window.panel.collapse`, `window.panel.cycleOptions`, `window.panels.peek` and
/// `window.collapseDock`, and the shared panel commands. Workspaces stay [`crate::workspaces`]'.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.panel" => show_panel(app, params),
        "window.collapseDock" => crate::menus::opt_bool(params, "collapsed").and_then(|c| collapse_dock(app, c)),
        "window.panels.peek" => peek::command(app, params),
        "window.panel.cycleOptions" => sizes::command(app, params),
        "window.panel.collapse" => {
            let minimized = match crate::menus::opt_bool(params, "collapsed") {
                Ok(c) => c,
                Err(e) => return Some(Err(e)),
            };
            let raw = params.get("panel").and_then(Value::as_str).unwrap_or_default();
            let Some(panel) = normalize(raw) else { return Some(Err(format!("unknown panel `{raw}`"))) };
            shared_command(app, "window.panel.minimize", &json!({"panel": panel, "minimized": minimized})).map(|mut value| {
                value["panel"] = json!(panel);
                value["collapsed"] = json!(group_minimized(layout_of(&app.ui), panel));
                value
            })
        }
        "ui.dock.reset" => app.run("window.workspace.reset", json!({})),
        "ui.dock" => ui_dock(app, params),
        "window.panel.float" | "window.panel.dock" => float_or_dock(app, id, params),
        "window.panel.layout" | "window.panel.move" | "window.panel.activate" | "window.panel.close" | "window.panel.minimize" => {
            shared_command(app, id, params)
        }
        _ => return None,
    })
}

/// `ui.dock`, the other shape of the docking commands: `{action}` is `window.panel.layout`, and
/// `{operation, …}` runs the `window.panel` command the operation names. `move` puts the panel in
/// `target`'s `zone` (center when none), as a tab before `before` when one is given; `float` and
/// `moveFloating` take `rect`, `[x, y, width, height]`; `setStackOpen` restores or minimizes.
fn ui_dock(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    if params.get("action").is_some() {
        return shared_command(app, "window.panel.layout", params);
    }
    let panel = params.get("panel").cloned().unwrap_or(Value::Null);
    let operation = params.get("operation").and_then(Value::as_str).unwrap_or_default();
    match operation {
        "open" | "activate" => shared_command(app, "window.panel.activate", &json!({"panel": panel})),
        "close" => shared_command(app, "window.panel.close", &json!({"panel": panel})),
        "move" => {
            let before = params.get("before");
            let zone = params.get("zone").and_then(Value::as_str);
            let zone = match (zone, before) {
                (None | Some("center" | "Center" | "tab"), Some(_)) => "tab",
                (zone, _) => zone.unwrap_or("center"),
            };
            let move_params = json!({"panel": panel, "anchor": params.get("target"), "zone": zone, "before": before});
            shared_command(app, "window.panel.move", &move_params)
        }
        "float" | "moveFloating" => {
            let [x, y, width, height]: [f32; 4] = serde_json::from_value(params.get("rect").cloned().unwrap_or_default())
                .map_err(|e| format!("rect must be [x, y, width, height]: {e}"))?;
            float_or_dock(app, "window.panel.float", &json!({"panel": panel, "x": x, "y": y, "width": width, "height": height}))
        }
        "setStackOpen" => {
            let open = params.get("open").and_then(Value::as_bool).ok_or("open must be true or false")?;
            shared_command(app, "window.panel.minimize", &json!({"panel": panel, "minimized": !open}))
        }
        "resizeStack" | "resizeSplit" => Err(format!("ui.dock `{operation}` is not available in VectorCraft")),
        other => Err(format!("unknown ui.dock operation `{other}`")),
    }
}

/// `window.panel {panel}`: show a panel. One in an icon column opens (or puts away) its flyout;
/// one in front of its floating window hides; one in a group comes to the front; one closed with a remembered place comes back there; a
/// library panel or a panel the dock has no place for floats. A flyout or floating panel is put
/// away only while it can be seen ([`visible`]): with the dock hidden by Tab or Shift+Tab, the
/// dock shows again with the panel where it was, and the toolbar and Control bar stay as they are.
/// Presentation Mode hides every panel, so there the command fails and changes nothing.
fn show_panel(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let raw = params.get("panel").and_then(Value::as_str).unwrap_or_default();
    let panel = normalize(raw).ok_or_else(|| format!("unknown panel `{raw}`"))?;
    if app.ui.screen_mode >= 3 {
        return Err("panels can't be shown in Presentation Mode".into());
    }
    place_panel(app, panel)
}

/// [`show_panel`] for a known panel, in any screen mode.
fn place_panel(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    let was_visible = visible(app, panel);
    show_panels(app);
    let id = panel.to_owned();
    let layout = layout_of(&app.ui);
    if layout.opens_in_flyout(&id) {
        app.ui.open_panel = if was_visible { None } else { Some(id) };
        return Ok(json!({"open": app.ui.open_panel}));
    }
    // A floating panel in front of its window hides again, as an icon panel's flyout does: its
    // shortcut shows and hides it.
    if was_visible && floating_window(layout, panel).is_some() {
        return shared_command(app, "window.panel.close", &json!({"panel": panel}));
    }
    let hidden = app.ui.dock_workspace.hidden().iter().any(|(p, _)| p == panel);
    if !layout.contains(&id) && !hidden && crate::panels::library_panel::parse(panel).is_some() {
        return crate::panels::library_panel::float(app, panel);
    }
    let value = if hidden { restore_hidden_group(app, panel)? } else { shared_command(app, "window.panel.activate", &json!({"panel": panel}))? };
    Ok(match floating_window(layout_of(&app.ui), panel) {
        Some(window) => json!({"floating": window_panels(window)}),
        None => value,
    })
}

/// Hides the group holding an active panel and records each tab's return location.
pub(crate) fn hide_group(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    let action = Action::close_group(panel.to_owned());
    let group = layout_of(&app.ui).closed_by(&action).map_err(|e| e.to_string())?;
    let ctx = app.ui_ctx.clone();
    let closed = apply(app, ctx.as_ref(), vec![action])?;
    if !group.is_empty() {
        app.ui.docking_hidden_groups.push(group);
    }
    Ok(result(app, closed))
}

/// A tab a panel shortcut hid with its group comes back alone when a command shows it: the
/// workspace would bring back the tabs closed with it, which only the shortcut restores
/// ([`restore_hidden_group`]).
fn show_alone(app: &mut VectorcraftApp, panel: &str) {
    let ws = &app.ui.dock_workspace;
    let grouped = app.ui.docking_hidden_groups.iter().any(|group| group.iter().any(|p| p == panel));
    if !grouped || !ws.hidden().iter().any(|(p, place)| p == panel && !place.closed_with.is_empty()) {
        return;
    }
    let hidden = ws.hidden().iter().map(|(p, place)| {
        let mut place = place.clone();
        if p == panel {
            place.closed_with.clear();
        }
        (p.clone(), place)
    });
    let next = Workspace::from_parts(ws.layout().clone(), hidden.collect(), ws.floated().to_vec());
    Arc::make_mut(&mut app.ui.dock_workspace).replace(next);
}

/// Reopens the tabs hidden together with `panel` and brings it to the front.
fn restore_hidden_group(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    let members =
        app.ui.docking_hidden_groups.iter().find(|group| group.iter().any(|id| id == panel)).cloned().unwrap_or_else(|| vec![panel.to_owned()]);
    let ws = &app.ui.dock_workspace;
    let mut actions: Vec<Action<String>> = members
        .into_iter()
        .filter(|member| ws.hidden().iter().any(|(id, _)| id == member))
        .map(|member| {
            let fallback = first_dock_panel(ws.layout(), &member);
            Action::open(member, fallback)
        })
        .collect();
    actions.push(Action::activate(panel.to_owned()));
    let value = run(app, actions, Some(panel.to_owned()))?;
    app.ui.docking_hidden_groups.retain(|group| !group.iter().any(|id| id == panel));
    Ok(value)
}

/// The tab in front of the group that holds `panel` in a dock column collapsed to icons.
fn flyout_tab(layout: &Layout<String>, panel: &str) -> Option<String> {
    dock_columns(layout)
        .into_iter()
        .filter(|column| column.iconic)
        .flat_map(|column| &column.groups)
        .find(|group| group.panels.iter().any(|p| p == panel))
        .and_then(|group| group.active_panel())
        .map(|front| front.to_string())
}

/// Show `panel` as `window.panel` does, but leave it be when it can be seen already ([`visible`]):
/// a command that opens a panel to work in (Edit › Color Settings…) never puts it away. A flyout
/// keeps the name of the icon that opened it after another of its tabs is clicked, so a panel whose
/// flyout is open with another tab in front comes to the front of it. In Presentation Mode the
/// panel comes to the front of its group and shows when the mode ends.
pub(crate) fn reveal_panel(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    if visible(app, panel) {
        if flyout_tab(layout_of(&app.ui), panel).is_some_and(|front| front != panel) {
            let ctx = app.ui_ctx.clone();
            apply(app, ctx.as_ref(), vec![Action::activate(panel.to_owned())])?;
        }
        return Ok(json!({"open": panel}));
    }
    // A tool rail (the Tools panel, a toolbar made with New Toolbar) comes back as a Window-menu panel does.
    let id = panel_id(panel).ok_or_else(|| format!("unknown panel `{panel}`"))?;
    place_panel(app, &id)
}

/// `window.collapseDock {collapsed?}`: collapse every dock column of panels to icons, or expand the
/// outer one; without `collapsed` it toggles the outer column.
pub(crate) fn collapse_dock(app: &mut VectorcraftApp, collapsed: Option<bool>) -> Result<Value, String> {
    let iconic = collapsed.unwrap_or(!dock_collapsed(&app.ui));
    show_panels(app);
    let columns = dock_columns(layout_of(&app.ui));
    let firsts = |c: &&Column<String>| c.groups.first().and_then(|g| g.panels.first()).cloned();
    let targets: Vec<String> = if iconic {
        columns.iter().filter(|c| !c.iconic).filter_map(firsts).collect()
    } else {
        columns.first().filter(|c| c.iconic).and_then(firsts).into_iter().collect()
    };
    let actions = targets.into_iter().map(|panel| Action::set_iconic(panel, iconic)).collect();
    let ctx = app.ui_ctx.clone();
    apply(app, ctx.as_ref(), actions)?;
    Ok(json!(iconic))
}

/// Where `window.panel.float` and `window.panel.dock` put a panel by the parameters VectorCraft
/// gave them before its panels moved to the framework's layout.
enum Legacy {
    /// `onto` (a tab of the group holding `anchor`, which floats unless the panel docks), `above`
    /// or `below` (a group of its own there, in that group's floating window or dock column).
    Join { anchor: String, placement: DockPlacement<String>, key: &'static str, floats: bool },
    /// `icon: true`: a group of its own at the end of the icon column (the innermost column of
    /// the end dock collapsed to icons), or a new icon column when the dock has none.
    Icons,
    /// `column: n` of `window.panel.dock`: a dock column of its own at place `n` of the end dock, 1
    /// next to the icon column, counting inwards; `column: true` with no destination: the
    /// innermost place.
    Column(usize),
}

/// The VectorCraft-only parameters of `window.panel.float` / `window.panel.dock`, taken out of
/// `fields` so the shared parser reads the rest.
#[derive(Default)]
struct Extra {
    /// Where the panel goes.
    legacy: Option<Legacy>,
    /// `collapsed`: minimize (true) or restore (false) its group afterwards.
    minimized: Option<bool>,
    /// `side` of `window.panel.dock`: a new innermost column of the dock on that window edge.
    side: Option<DockEdge>,
}

fn take_legacy(id: &str, fields: &mut serde_json::Map<String, Value>) -> Result<Extra, String> {
    let dock = id == "window.panel.dock";
    let minimized = match fields.remove("collapsed") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(c)) => Some(c),
        Some(_) => return Err("collapsed must be true or false".into()),
    };
    let mut joins = Vec::new();
    for (key, placement) in
        [("onto", DockPlacement::tab(None)), ("above", DockPlacement::group(false, None)), ("below", DockPlacement::group(true, None))]
    {
        if let Some(value) = fields.remove(key) {
            let anchor = value.as_str().and_then(panel_id).ok_or_else(|| format!("{key} must name a registered panel"))?;
            joins.push(Legacy::Join { anchor, placement, key, floats: !dock && key == "onto" });
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
        // `column: true`: the panel's whole dock column or floating window.
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
    // Saved actions and scripts name the window edge the old ways: left and right, Start and End.
    let side = match fields.remove("side").filter(|_| dock) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("start") => Some(DockEdge::Start),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("right") || s.eq_ignore_ascii_case("end") => Some(DockEdge::End),
        Some(Value::String(s)) => return Err(format!("side is left or right, not `{s}`")),
        Some(other) => return Err(format!("side is left or right, not `{other}`")),
    };
    Ok(Extra { legacy, minimized, side })
}

/// `window.panel.float` and `window.panel.dock`: the shared commands, with VectorCraft's own
/// parameters ([`Legacy`], `collapsed`, `side`) done in the same change.
fn float_or_dock(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    let mut params = if params.is_null() { json!({}) } else { params.clone() };
    let fields = params.as_object_mut().ok_or("params must be an object")?;
    let extra = take_legacy(id, fields)?;
    shared_command_with(app, id, &params, extra)
}

/// Is the group holding `panel` minimized to its title bar?
pub(crate) fn group_minimized(layout: &Layout<String>, panel: &str) -> bool {
    layout.place(&panel.to_owned()).and_then(|place| layout.group(&place).map(|g| g.minimized)).unwrap_or(false)
}

/// The panels of the group holding `panel` (an accordion entry is a group of one).
fn mates(layout: &Layout<String>, panel: &str) -> Vec<String> {
    let Some(place) = layout.place(&panel.to_owned()) else { return Vec::new() };
    match (layout.group(&place), layout.node_of(&place)) {
        (Some(group), _) => group.panels.clone(),
        (None, Some(Node::Tabs { panels, .. })) => panels.clone(),
        _ => vec![panel.to_owned()],
    }
}

/// The icon column among `columns`: the iconic column holding the most panels of the icon groups.
fn icon_column(columns: &[Column<String>]) -> Option<usize> {
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

/// The action that moves `unit` to `to`.
fn unit_action(unit: &DragUnit<String>, to: Target<String>) -> Action<String> {
    match unit {
        DragUnit::Group(panel) => Action::move_group(panel.clone(), to),
        DragUnit::Column(panel) => Action::move_column(panel.clone(), to),
        DragUnit::Window(panel) => Action::move_window(panel.clone(), to),
        _ => Action::move_panel(unit.lead().clone(), to),
    }
}

impl Legacy {
    /// The actions that put `panel` (with `group`, its whole group) where `self` says, on the
    /// layout with the panel shown; none when it is there already.
    fn actions(&self, layout: &Layout<String>, panel: &str, group: bool) -> Result<Vec<Action<String>>, String> {
        let id = panel.to_owned();
        let mates = mates(layout, panel);
        let unit = if group { DragUnit::Group(id.clone()) } else { DragUnit::Panel(id.clone()) };
        match self {
            Self::Join { anchor, placement, key, floats } => {
                if !layout.contains(anchor) {
                    return Err(format!("{key} must name a panel that shows"));
                }
                let tab = matches!(placement, DockPlacement::Tab { .. });
                if anchor == panel || ((group || tab) && mates.contains(anchor)) {
                    return Err(format!("{key} must name a panel in another group"));
                }
                // Float Panel stacks onto floating groups only; Dock Panel's `onto` is its `anchor`.
                if *floats && floating_window(layout, anchor).is_none() {
                    return Err(format!("{key} must name a floating panel"));
                }
                Ok(vec![unit_action(&unit, Target::at(anchor.clone(), placement.clone()))])
            }
            Self::Icons => {
                let columns = edge_columns(layout, DockEdge::End);
                let moving = if group { mates } else { vec![id.clone()] };
                if let Some(column) = icon_column(columns).and_then(|k| columns.get(k)) {
                    if column.panels().any(|p| p == panel) {
                        return Ok(Vec::new());
                    }
                    if let Some(anchor) = column.groups.iter().rev().flat_map(|g| &g.panels).find(|p| !moving.contains(p)) {
                        return Ok(vec![unit_action(&unit, Target::at(anchor.clone(), DockPlacement::group(true, None)))]);
                    }
                }
                // No icon column: a new one inside the outer column, collapsed to icons.
                let to = match columns.iter().flat_map(|c| c.panels()).find(|p| !moving.contains(p)) {
                    Some(anchor) => Target::at(anchor.clone(), DockPlacement::column(true)),
                    None => Target::new_dock(DockEdge::End),
                };
                Ok(vec![unit_action(&unit, to), Action::set_iconic(id, true)])
            }
            Self::Column(place) => {
                let columns = edge_columns(layout, DockEdge::End);
                let window = floating_window(layout, panel);
                // Its group, or alone in its group anyway: the group, or the whole floating
                // window it floats in (a stack of groups docks as one column).
                let whole = group || mates.len() == 1;
                let moving = match window {
                    Some(window) if whole => window_panels(window),
                    _ if whole => mates,
                    _ => vec![id.clone()],
                };
                // The dock as it is without what moves: each column that keeps a panel, with one
                // of them as the anchor.
                let staying: Vec<(usize, &String)> =
                    columns.iter().enumerate().filter_map(|(k, c)| c.panels().find(|p| !moving.contains(p)).map(|p| (k, p))).collect();
                // Place 1 is next to the icon column, inwards; without one, next to the outer column.
                let base = match icon_column(columns) {
                    Some(icons) => staying.iter().filter(|(k, _)| *k <= icons).count(),
                    None => staying.len().min(1),
                };
                let at = base.saturating_add(place.saturating_sub(1)).min(staying.len());
                // What moves is a whole column at that place already.
                let alone = columns.iter().position(|c| {
                    let panels: Vec<&String> = c.panels().collect();
                    panels.len() == moving.len() && panels.iter().all(|p| moving.contains(p))
                });
                if alone.is_some_and(|k| staying.iter().filter(|(j, _)| *j < k).count() == at) {
                    return Ok(Vec::new());
                }
                let to = match staying.get(at) {
                    Some((_, anchor)) => Target::at((*anchor).clone(), DockPlacement::column(false)),
                    None => {
                        let (_, anchor) = staying.last().ok_or("the window has no dock")?;
                        Target::at((*anchor).clone(), DockPlacement::column(true))
                    }
                };
                // Taken out of an icon column, the new column keeps that column's icons: it opens expanded.
                let expand = layout.opens_in_flyout(&id).then(|| Action::set_iconic(id.clone(), false));
                let unit = match window {
                    Some(_) if whole => DragUnit::Window(id),
                    _ if whole => DragUnit::Group(id),
                    _ => DragUnit::Panel(id),
                };
                Ok(std::iter::once(unit_action(&unit, to)).chain(expand).collect())
            }
        }
    }
}

/// Command params with VectorCraft's panel names resolved (a panel id or English name, in any
/// case, or a panel id made at run time).
fn resolve_panels(params: &Value) -> Result<Value, String> {
    let mut params = if params.is_null() { json!({}) } else { params.clone() };
    let fields = params.as_object_mut().ok_or("params must be an object")?;
    for key in ["panel", "anchor", "target", "before"] {
        let Some(value) = fields.get_mut(key) else { continue };
        if key == "before" && value.is_null() {
            continue;
        }
        let id = value.as_str().and_then(panel_id).ok_or_else(|| format!("{key} must name a registered panel"))?;
        *value = Value::String(id);
    }
    Ok(params)
}

/// The params of the shared panel commands, as JSON gives them.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Params {
    /// The panel the command acts on.
    panel: Option<String>,
    /// The panel whose group a move or dock goes to.
    anchor: Option<String>,
    /// Where beside `anchor`: tab, center, left, right, top, bottom, above or below.
    zone: Option<String>,
    /// The tab a move into a group goes before.
    before: Option<String>,
    /// A serialized placement beside `anchor`, in place of `zone` and `before`.
    placement: Option<DockPlacement<String>>,
    /// The serialized action of `window.panel.layout`.
    action: Option<Action<String>>,
    /// The panel's whole group.
    group: bool,
    /// The panel's whole dock column or floating window.
    column: bool,
    /// `window.panel.minimize`: minimized (true) or restored; none toggles.
    minimized: Option<bool>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
}

/// A shared panel command (`window.panel.layout`, `.move`, `.activate`, `.close`, `.minimize`,
/// `.float`, `.dock`) on the workspace, all or nothing. VectorCraft resolves panel names, shows a
/// hidden or new panel before moving, floating or docking it, and returns a floated panel to its
/// dock place on Dock Panel.
fn shared_command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    shared_command_with(app, id, params, Extra::default())
}

/// [`shared_command`], with the panel put where `extra.legacy` says instead of where the command
/// puts it, and its group then minimized or restored (`extra.minimized`).
fn shared_command_with(app: &mut VectorcraftApp, id: &str, params: &Value, extra: Extra) -> Result<Value, String> {
    let p: Params = serde_json::from_value(resolve_panels(params)?).map_err(|e| e.to_string())?;
    if id == "window.panel.layout" {
        let action = p.action.ok_or("action is required")?;
        let reveal = match &action {
            Action::Activate { panel, .. } | Action::Open { panel, .. } | Action::OpenAt { panel, .. } => Some(panel.clone()),
            _ => None,
        };
        return run(app, vec![action], reveal);
    }
    let panel = p.panel.clone().ok_or("panel is required")?;
    if !matches!(id, "window.panel.close" | "window.panel.minimize") {
        show_alone(app, &panel);
    }
    let space = command_space(app);
    let ws: &Workspace<String> = &app.ui.dock_workspace;
    let shown = ws.layout().contains(&panel);
    let mut actions = match id {
        _ if shown => Vec::new(),
        "window.panel.close" | "window.panel.minimize" => Vec::new(),
        // Only Dock and Move put a panel with no place in the dock; showing it (`window.panel.activate`) floats it.
        _ => show_actions(ws, &space, &panel, matches!(id, "window.panel.dock" | "window.panel.move")),
    };
    // The command's own actions read the layout with the panel shown.
    let trial = if actions.is_empty() {
        None
    } else {
        let mut trial = ws.clone();
        trial.apply_all(actions.clone(), false, |p: &String| permissions(p)).map_err(|e| e.to_string())?;
        Some(trial)
    };
    let current = trial.as_ref().unwrap_or(ws);
    let layout = current.layout();
    let rights = |p: &String| permissions(p);
    let scene = DockScene::new(layout, &space, false, &rights).floated(current.floated());
    let unit = match (p.column, floating_window(layout, &panel)) {
        (true, Some(_)) => DragUnit::Window(panel.clone()),
        (true, None) => DragUnit::Column(panel.clone()),
        _ if p.group => DragUnit::Group(panel.clone()),
        _ => DragUnit::Panel(panel.clone()),
    };
    let dock_error = |e: craft::behavior::dock::DockError| e.to_string();
    let mut more = match (id, &extra.legacy) {
        ("window.panel.float" | "window.panel.dock", Some(legacy)) => legacy.actions(layout, &panel, p.group)?,
        (_, Some(_)) => return Err("onto, above, below, icon and a column place belong to window.panel.float and window.panel.dock".into()),
        // Showing it was the whole command.
        ("window.panel.activate", _) if !shown => Vec::new(),
        ("window.panel.activate", _) => vec![Action::activate(panel.clone())],
        ("window.panel.close", _) => vec![Action::close(panel.clone())],
        ("window.panel.minimize", _) => {
            vec![Action::set_minimized(panel.clone(), p.minimized.unwrap_or(!group_minimized(layout, &panel)))]
        }
        ("window.panel.move", _) => {
            let anchor = p.anchor.clone().ok_or("anchor is required")?;
            moved_to(&scene, &unit, anchor, &p)?
        }
        ("window.panel.float", _) => float_actions(app.ui_ctx.as_ref(), &scene, layout, &space, &unit, &p)?,
        ("window.panel.dock", _) => match (extra.side, p.anchor.clone()) {
            (Some(_), Some(_)) => return Err("give anchor or side, not both".into()),
            (Some(edge), None) => move_actions(&scene, &unit, DockTo::Dock(edge)).map_err(dock_error)?,
            (None, Some(anchor)) => moved_to(&scene, &unit, anchor, &p)?,
            (None, None) if p.zone.is_some() => {
                return Err("zone needs an anchor: dock beside a panel with anchor, or in a new dock column with side (Start or End)".into());
            }
            // Showing it was the whole command.
            (None, None) if !shown => Vec::new(),
            (None, None) if p.group => move_actions(&scene, &unit, DockTo::Docked).map_err(dock_error)?,
            (None, None) => command_actions(&scene, DockCommands::DockPanel, &panel).map_err(dock_error)?,
        },
        _ => return Err(format!("unknown docking command `{id}`")),
    };
    if let Some(minimized) = extra.minimized {
        more.push(Action::set_minimized(panel.clone(), minimized));
    }
    actions.extend(more);
    let reveal = (!matches!(id, "window.panel.close" | "window.panel.minimize")).then_some(panel);
    run(app, actions, reveal)
}

/// The actions that show `panel`, which is not in the layout: back where it was when it is hidden,
/// else beside the first of its [`ICON_PANEL_GROUPS`] row that is placed, else where the dock
/// opens a panel when the command `docks` it, else floating.
fn show_actions(ws: &Workspace<String>, space: &FloatSpace<String>, panel: &str, docks: bool) -> Vec<Action<String>> {
    let id = panel.to_owned();
    let layout = ws.layout();
    if ws.hidden().iter().any(|(p, _)| p == panel) {
        return vec![Action::open(id.clone(), first_dock_panel(layout, panel)), Action::activate(id)];
    }
    let mate = ICON_PANEL_GROUPS
        .iter()
        .find(|row| row.contains(&panel))
        .and_then(|row| row.iter().find(|p| **p != panel && layout.contains(&(**p).to_owned())));
    match (mate, docks) {
        (Some(mate), _) => vec![Action::open_at(id, (*mate).to_owned(), DockPlacement::tab(None))],
        (None, true) => vec![Action::open(id, first_dock_panel(layout, panel))],
        (None, false) => {
            let rect = space.place(space.size_of(&DragUnit::Panel(id.clone()), None));
            vec![Action::open(id.clone(), None), Action::float(id, rect)]
        }
    }
}

/// The actions that move `unit` beside `anchor`: at `placement` when the params give one, else
/// in the zone they name (a tab before `before` when they name none).
fn moved_to(scene: &DockScene<'_, String>, unit: &DragUnit<String>, anchor: String, p: &Params) -> Result<Vec<Action<String>>, String> {
    if let Some(placement) = &p.placement {
        return Ok(vec![unit_action(unit, Target::at(anchor, placement.clone()))]);
    }
    let side = match p.zone.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("tab") => None,
        Some("center") => Some(DockSide::Center),
        Some("left") => Some(DockSide::Start),
        Some("right") => Some(DockSide::End),
        Some("top" | "above") => Some(DockSide::Top),
        Some("bottom" | "below") => Some(DockSide::Bottom),
        Some(other) => return Err(format!("zone is tab, center, left, right, top, bottom, above or below, not `{other}`")),
    };
    let to = match side {
        Some(side) => DockTo::beside(anchor, side),
        None => DockTo::tabs(anchor, p.before.clone()),
    };
    move_actions(scene, unit, to).map_err(|e| e.to_string())
}

/// The actions that float `unit`: where a drag of it would float it when the params give no
/// geometry, else at `x`, `y`, `width` and `height`, each defaulting to the rect of the window it
/// already floats alone in, or to where the float space puts it. A whole window keeps its rect
/// unless given one. A window in an OS window of its own (on `ctx`) is where that window is.
fn float_actions(
    ctx: Option<&egui::Context>,
    scene: &DockScene<'_, String>,
    layout: &Layout<String>,
    space: &FloatSpace<String>,
    unit: &DragUnit<String>,
    p: &Params,
) -> Result<Vec<Action<String>>, String> {
    let panel = unit.lead();
    let geometry = [p.x, p.y, p.width, p.height];
    if geometry.iter().flatten().any(|v| !v.is_finite()) {
        return Err("x, y, width and height must be finite".into());
    }
    if [p.width, p.height].iter().flatten().any(|v| *v <= 0.0) {
        return Err("width and height must be positive".into());
    }
    let framework = match unit {
        DragUnit::Panel(_) => Some(DockCommands::FloatPanel),
        DragUnit::Group(_) => Some(DockCommands::FloatGroup),
        _ => None,
    };
    let window = floating_window(layout, panel);
    // A panel or group floated out of an icon column opens expanded; the dock floats a dragged icon collapsed.
    let expand = (framework.is_some() && window.is_none() && layout.opens_in_flyout(panel)).then(|| Action::set_iconic(panel.clone(), false));
    if let Some(command) = framework.filter(|_| geometry.iter().all(Option::is_none)) {
        let mut actions = command_actions(scene, command, panel).map_err(|e| e.to_string())?;
        actions.extend(expand);
        return Ok(actions);
    }
    let whole = match unit {
        DragUnit::Panel(_) => window.is_some_and(|w| w.panels().count() == 1),
        DragUnit::Group(_) => window.is_some_and(|w| w.groups().len() == 1),
        _ => window.is_some(),
    };
    let kept = window.filter(|_| whole).map(|w| shown_rect(ctx, w));
    let base = kept.unwrap_or_else(|| space.place(space.size_of(unit, None)));
    let min = egui::pos2(p.x.unwrap_or(base.min.x), p.y.unwrap_or(base.min.y));
    let rect = egui::Rect::from_min_size(min, egui::vec2(p.width.unwrap_or(base.width()), p.height.unwrap_or(base.height())));
    let float = match (kept, unit) {
        (Some(_), _) => Action::move_floating(panel.clone(), rect),
        (None, DragUnit::Panel(_)) => Action::float(panel.clone(), rect),
        (None, _) => unit_action(unit, Target::float(rect)),
    };
    let native = window.filter(|_| kept.is_some()).and_then(|w| native_moved(ctx, w, rect));
    Ok(std::iter::once(float).chain(native).chain(expand).collect())
}

/// The record of where the OS window of `window` (on `ctx`) is, moved to `rect` in the main window's points: an open OS
/// window stays where the dock recorded it, so a command that moves the window moves the record with it.
fn native_moved(ctx: Option<&egui::Context>, window: &Floating<String>, rect: egui::Rect) -> Option<Action<String>> {
    let mut native = window.native.clone()?;
    let main = craft::ui::runtime::windows::WindowHost::of(ctx?).facts(egui::ViewportId::ROOT)?;
    let at = rect.min + main.inner.min.to_vec2();
    native.normal.pos = at.into();
    native.normal.size = rect.size().into();
    native.position_px = Some([(at.x * native.scale).round() as i32, (at.y * native.scale).round() as i32]);
    Some(Action::set_native(window.panels().next()?.clone(), Some(native)))
}

/// Where floating `window` is, in the main window's points: where the OS put its own window when it has one (on `ctx`),
/// else its rect in the main window.
fn shown_rect(ctx: Option<&egui::Context>, window: &Floating<String>) -> egui::Rect {
    let native = window.native.as_ref().and_then(|n| Some(egui::Rect::from_min_size(n.position()?.into(), n.normal.size.into())));
    native.zip(ctx).and_then(|(rect, ctx)| craft::ui::docking::native::in_app(ctx, rect)).unwrap_or(window.rect)
}

/// Applies `actions` ([`apply`]) and reveals `reveal`, the panel the command showed: the panels
/// show, and its flyout opens or its floating window comes forward.
fn run(app: &mut VectorcraftApp, actions: Vec<Action<String>>, reveal: Option<String>) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone();
    let closed = apply(app, ctx.as_ref(), actions)?;
    if let Some(panel) = reveal.filter(|panel| layout_of(&app.ui).contains(panel)) {
        show_panels(app);
        if layout_of(&app.ui).opens_in_flyout(&panel) {
            app.ui.open_panel = Some(panel);
        } else {
            app.dock_rt.raise = Some(panel);
        }
    }
    Ok(result(app, closed))
}

/// A docking command's answer: the layout, the panels it closed and the floating panels.
fn result(app: &VectorcraftApp, closed: Vec<String>) -> Value {
    let layout = layout_of(&app.ui);
    let floating: Vec<&String> = layout.floating.iter().flat_map(|w| w.panels()).collect();
    json!({"layout": layout, "closed": closed, "floating": floating})
}

/// Where a command floats a panel, group or column it gives no rect: inside the dock area as last
/// drawn, else inside [`HEADLESS_AREA`] at the theme's float size.
fn command_space(app: &VectorcraftApp) -> FloatSpace<String> {
    if let Some(space) = &app.dock_rt.space {
        return space.clone();
    }
    let sizes = match &app.ui_ctx {
        Some(ctx) => DockSizes::of(ctx),
        None => DockSizes::of(&egui::Context::default()),
    };
    FloatSpace::new(HEADLESS_AREA, &sizes)
}

/// The height of `panel`'s window while it floats, else the height [`place_floating`] gives a new one.
pub(crate) fn floating_height(app: &VectorcraftApp, panel: &str) -> f32 {
    match floating_window(layout_of(&app.ui), panel) {
        Some(window) => window.rect.height(),
        None => command_space(app).size_of(&DragUnit::Panel(panel.to_owned()), None).y,
    }
}

/// Float `panel` (open or not) as a window of its own with its top-left corner at `pos`, `width`
/// wide (the float space's width when `width` is not positive) and [`floating_height`] high: the
/// Tabs panel over the text it sets, a new toolbar. A panel alone in its window moves there. A
/// panel whose spec lays its content in a grid keeps whole grid steps; the dock sizes its window.
pub(crate) fn place_floating(app: &mut VectorcraftApp, panel: &str, pos: egui::Pos2, width: f32) -> Result<(), String> {
    let id = panel_id(panel).ok_or("unknown panel")?;
    if !(pos.x.is_finite() && pos.y.is_finite() && width.is_finite()) {
        return Err("panel position must be finite".into());
    }
    let width = if width > 0.0 { width } else { command_space(app).size_of(&DragUnit::Panel(id.clone()), None).x };
    let rect = egui::Rect::from_min_size(pos, egui::vec2(width, floating_height(app, &id)));
    let layout = layout_of(&app.ui);
    let mut actions = Vec::new();
    if !layout.contains(&id) {
        actions.push(Action::open(id.clone(), None));
    }
    // A rail's window is a stepped column, which the dock fits to the rail's grid; it keeps the steps it had.
    let steps = is_rail(&id).then(|| column_of(layout, &id).and_then(|(_, column)| column.steps).unwrap_or(1));
    actions.push(match floating_window(layout, &id) {
        Some(window) if window.panels().count() == 1 => Action::move_floating(id.clone(), rect),
        _ => Action::float(id.clone(), rect),
    });
    if let Some(steps) = steps {
        actions.push(Action::set_column_steps(id, steps));
    }
    let ctx = app.ui_ctx.clone();
    apply(app, ctx.as_ref(), actions).map(drop)
}

/// What the dock reads of the app's panels besides their fixed names and icons: the language the
/// names are in, the names of the toolbars made with New Toolbar, the titles of the library
/// panels in the layout and the grids of its tool rails. Plain data, edited only when one of them
/// changes ([`refresh_specs`]), so the dock reads no spec in a pass where none changed.
#[derive(Clone, Debug, Default, PartialEq, PlainData)]
pub(crate) struct Specs {
    /// The UI language's code.
    lang: &'static str,
    /// Each toolbar made with New Toolbar: its id and name.
    toolbars: Vec<(u32, String)>,
    /// Each library panel of the layout: its id and tab text.
    libraries: Vec<(String, String)>,
    /// Each tool rail of the layout: its id and the grid its column steps in ([`crate::toolbar::dock_grid`]).
    #[plain_data(assume = "a dock grid is plain numbers")]
    grids: Vec<(String, DockGrid)>,
}

impl Specs {
    /// The specs as the app shows them now.
    fn of(app: &VectorcraftApp) -> Specs {
        let libraries = layout_of(&app.ui)
            .panels()
            .into_iter()
            .filter_map(|panel| crate::panels::library_panel::tab_label(app, panel).map(|label| (panel.clone(), label)))
            .collect();
        let grids = layout_of(&app.ui)
            .panels()
            .into_iter()
            .filter_map(|panel| rail_owner(&app.ui, panel).map(|owner| (panel.clone(), crate::toolbar::dock_grid(app, &owner))))
            .collect();
        Specs {
            lang: crate::i18n::current().code(),
            toolbars: app.ui.custom_toolbars.iter().map(|bar| (bar.id, bar.name.clone())).collect(),
            libraries,
            grids,
        }
    }

    /// The tab text of `panel`.
    fn label(&self, panel: &str) -> String {
        if panel == CANVAS {
            return tl!("Canvas").to_string();
        }
        let mut owner = rail_owner_among(self.toolbars.iter().map(|(id, _)| *id), panel);
        // A strip is named for the toolbar it was torn off.
        while let Some(Owner::Strip(parent, _)) = owner {
            owner = Some(*parent);
        }
        match owner {
            Some(Owner::Toolbar(id)) => self.toolbars.iter().find(|(bar, _)| *bar == id).map_or_else(|| panel.to_owned(), |(_, name)| name.clone()),
            Some(_) => tl!("Tools").to_string(),
            None => self
                .libraries
                .iter()
                .find(|(library, _)| library == panel)
                .map(|(_, label)| label.clone())
                .or_else(|| crate::state::all_panels().find(|p| p.0 == panel).map(|p| tl!(p.1).to_string()))
                .unwrap_or_else(|| panel.to_owned()),
        }
    }
}

impl PanelSpecs<String> for Specs {
    fn spec(&self, panel: &String) -> PanelSpec {
        let spec = PanelSpec::new(self.label(panel));
        if panel == CANVAS {
            return spec.canvas();
        }
        let spec = match self.grids.iter().find(|(rail, _)| rail == panel) {
            Some((_, grid)) => spec.grid(*grid),
            None => spec,
        };
        let icon = crate::state::rail_panel(panel).map(|p| p.2).or_else(|| crate::panels::library_panel::parse(panel).map(|_| "library"));
        match icon {
            Some(icon) => spec.icon(IconSpec::svg(IconId::new(icon))),
            None => spec,
        }
    }

    fn permissions(&self, panel: &String) -> Permissions {
        permissions(panel)
    }
}

/// Brings the dock's spec model and rail set up to date: each edited, which moves its revision,
/// only when it changed.
fn refresh_specs(app: &mut VectorcraftApp) {
    let now = Specs::of(app);
    if *app.dock_rt.specs != now {
        app.dock_rt.specs.edit(|specs| *specs = now);
    }
    let rails: BTreeSet<PanelKey> = layout_of(&app.ui).panels().into_iter().filter(|p| is_rail(p)).map(PanelKey::of).collect();
    if *app.dock_rt.rails != rails {
        app.dock_rt.rails.edit(|kept| *kept = rails);
    }
}

/// The app's panels as the dock shows them: their bodies, and the canvas and tool rails it hands
/// back to the app to draw.
pub(crate) struct Panels<'a> {
    /// The app whose panels these are.
    app: &'a mut VectorcraftApp,
}

impl DockPanels<String> for Panels<'_> {
    fn body(&mut self, panel: &String, ui: &mut egui::Ui, _cx: &mut BodyCx<'_>) {
        crate::dock::panel_body(self.app, ui, panel);
        // A floating panel's OS window runs a pass of its own, whose leftover keys the main window's pass never sees.
        if ui.ctx().viewport_id() != egui::ViewportId::ROOT {
            crate::shortcuts::palette_window_keys(self.app, ui.ctx());
        }
    }

    fn group_view(&mut self, panels: &[String], _active: usize, ui: &mut egui::Ui, at: GroupAt, cx: &mut BodyCx<'_>) -> Option<f32> {
        if panels.iter().any(|p| p == CANVAS) {
            return Some(match at {
                GroupAt::Measure(_) => 0.0,
                GroupAt::Show(rect) => {
                    self.app.canvas_area(ui);
                    rect.height()
                }
            });
        }
        // A rail shares its column with nothing, so its group is the rail alone. Its view draws
        // the Edit Toolbar drawer while the drawer targets it.
        let owner = panels.iter().find_map(|p| rail_owner(&self.app.ui, p))?;
        Some(crate::toolbar::rail_view(self.app, ui, owner, at, cx))
    }

    fn group_header(&mut self, panels: &[String], active: usize, ui: &mut egui::Ui, _cx: &mut BodyCx<'_>) {
        let Some(panel) = panels.get(active).filter(|p| sizes::has_sizes(p)) else { return };
        IconButton::command(IconId::new("dc-cycle-sizes"), CommandRef::with_params("window.panel.cycleOptions", json!({"panel": panel}))).show(ui);
    }

    fn panel_rows(&mut self, panel: &String) -> Vec<PanelRow> {
        // A toolbar docks and floats by dragging: its menu has only the dock's own rows.
        if matches!(rail_owner(&self.app.ui, panel), Some(Owner::Toolbar(_))) {
            return Vec::new();
        }
        crate::panels::panel_rows(self.app, panel)
    }
}

/// The OS window the floating window holding `panel` showed in last pass.
fn window_viewport(app: &VectorcraftApp, panel: &str) -> Option<egui::ViewportId> {
    let first = floating_window(layout_of(&app.ui), panel)?.panels().next()?;
    app.dock_rt.windows.iter().find(|(p, _)| p == first).map(|(_, viewport)| *viewport)
}

/// Whether the OS window of a floating panel lies at `pos`, a point of the main window: a palette
/// window stays in front of the main window.
pub(crate) fn os_window_at(app: &VectorcraftApp, ctx: &egui::Context, pos: egui::Pos2) -> bool {
    if app.dock_rt.windows.is_empty() {
        return false;
    }
    let origin = ctx.input(|i| i.raw.viewports.get(&i.raw.viewport_id).and_then(|v| v.inner_rect).map(|r| r.min.to_vec2()));
    let Some(origin) = origin else { return false };
    let under = craft::ui::runtime::windows::WindowHost::of(ctx).viewports_at(pos + origin);
    under.iter().any(|viewport| app.dock_rt.windows.iter().any(|(_, shown)| shown == viewport))
}

/// Brings the OS window of the floating window holding `panel` forward, as it was last drawn.
fn raise(app: &VectorcraftApp, ctx: &egui::Context, panel: &str) {
    if let Some(viewport) = window_viewport(app, panel) {
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
    }
}

/// The floating window holding `panel` takes over the held primary press on the next pass, with the
/// pointer at `grab` inside it: a panel floated out of a tool rail mid-drag follows the pointer and
/// docks where the release lands.
pub(crate) fn continue_move(app: &mut VectorcraftApp, panel: &str, grab: egui::Vec2) {
    app.dock_rt.hand_over = Some((panel.to_owned(), grab));
}

/// Draw the canvas, the tool rails and the panels below the bars, and apply what the user did to
/// them. Tab and Shift+Tab hide the panels through the window's shell; Shift+Tab keeps the rails.
/// Presentation Mode hides every panel and rail; the canvas stays. `keeps_tools`: the shell around the dock hides the
/// panels and keeps the tools (Shift+Tab), as its `ShellCx` says; [`chrome_state`] is for code outside its pass.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, keeps_tools: bool) {
    let ctx = ui.ctx().clone();
    if let Some(panel) = app.dock_rt.raise.take() {
        raise(app, &ctx, &panel);
    }
    refresh_specs(app);
    // The area shares the workspace's `Arc`, so panel bodies that read the layout in the pass
    // (`visible`, `showing`) see it; their changes go through `apply`.
    let mut workspace = Arc::clone(&app.ui.dock_workspace);
    let revision = workspace.revision();
    let flyout_before = app.ui.open_panel.clone();
    let presenting = app.ui.screen_mode >= 3;
    // Shift+Tab, from its key, a menu or automation, closes a panel popped out of the icon column.
    let mut flyout = flyout_before.clone().filter(|_| !keeps_tools);
    let auto_collapse = app.session.prefs.auto_collapse_icon_panels;
    // The specs and rails leave the app for the pass, which lends the app to the panel bodies.
    let specs = std::mem::take(&mut app.dock_rt.specs);
    let rails = std::mem::take(&mut app.dock_rt.rails);
    // The rails are what Shift+Tab keeps; while the panels show, the area hides none.
    let hide = if presenting {
        HidePanels::All
    } else if keeps_tools {
        HidePanels::Except(&rails)
    } else {
        HidePanels::None
    };
    let hand_over = app.dock_rt.hand_over.take();
    let output = {
        let mut panels = Panels { app: &mut *app };
        let mut area = DockArea::new(
            "Dock",
            Bind::bound(&mut workspace),
            &specs,
            &mut panels,
            DockConfig::default()
                .panel_menu_always(true)
                .panel_menu_icon(IconId::new("menu"))
                .floating_rounded_body(true)
                .tab_look(PanelTabLook::Block)
                .column_cards(ColumnCards::Stepped)
                .flush_docks(true),
        )
        .hide(hide)
        .flyout(Bind::bound(&mut flyout));
        if let Some((panel, grab)) = hand_over {
            area = area.continue_move((panel, grab));
        }
        if auto_collapse { area.auto_collapse().show(ui) } else { area.show(ui) }
    };
    app.dock_rt.specs = specs;
    app.dock_rt.rails = rails;
    // A workspace a command switched to or changed in this pass (`Workspace::replace`) stays; the
    // pass's edits were to the one it replaced.
    if app.ui.dock_workspace.revision() == revision {
        app.ui.dock_workspace = workspace;
    }
    // A command of this pass that opened or closed a flyout wins over the area's own change.
    if app.ui.open_panel == flyout_before {
        app.ui.open_panel = flyout;
    }
    if output.error.is_some() {
        Arc::make_mut(&mut app.ui.dock_workspace).replace(default_workspace(&[]));
        app.status(tl!("Invalid panel layout; restored the default dock").to_string());
    }
    if !output.requested.is_empty()
        && let Err(error) = apply(app, None, output.requested)
    {
        app.status(error);
    }
    for event in &output.events {
        if matches!(event, DockEvent::ToggleAutoCollapse) {
            app.session.prefs.auto_collapse_icon_panels = !app.session.prefs.auto_collapse_icon_panels;
        }
        // PLAN: `DockEvent::PanelCommand` (Move Panel To…, Move Group To…) needs a destination picker over `Layout::move_targets`; no sheet row builds it.
    }
    if !output.window_files.is_empty() {
        // Shift as the frame holds it: the window system sends a file drag no key events, and a second pass of the
        // frame reads the same state.
        let shift = ctx.input(|i| i.modifiers.shift);
        // The panel each drop landed on is not a target here: every dropped file opens, as before.
        let files = output.window_files.into_iter().flat_map(|drop| drop.files).collect();
        crate::place::os_drop(app, files, crate::place::DropSpot::Unknown, shift);
    }
    if output.float_space.is_some() {
        app.dock_rt.space = output.float_space;
    }
    if let Some(panel) = output.focused {
        app.dock_rt.focused = Some(panel);
    }
    app.dock_rt.columns = output.columns;
    app.dock_rt.windows = output.windows;
    settle(app);
}

/// Is a modal window open in this pass: a modal dialog, Preferences, Keyboard Shortcuts or a file
/// dialog? A popup (a menu, a dropdown, a slider strip) is no window, so it does not close itself;
/// nor is the open palette's own box, so a modal layer of the last pass counts only while the
/// palette is closed.
pub(crate) fn modal_window_up(app: &VectorcraftApp) -> bool {
    use craft::ui::runtime::overlay::{OpenFilter, any_open};
    (!app.ui.palette_open && app.ui_ctx.as_ref().is_some_and(|ctx| any_open(ctx, OpenFilter::Modal)))
        || app.dialogs().is_some_and(|d| d.with(|h| h.shows_modal()))
        || crate::prefs_dialog::is_open(app)
        || app.shortcuts_dialog.is_some()
        || app.file_dialog_open()
}

/// The key of whether a modal window was up in the last pass ([`close_overlays_for_modal`]).
fn modal_window_up_id() -> egui::Id {
    egui::Id::unique("modal-window-up")
}

/// While a modal window is up ([`modal_window_up`]), keeps the command palette closed, so a palette
/// opened over it from the macOS menu bar or by an automation command does not take the keyboard
/// from the window. In the pass the window opens, also closes the All Tools drawer and every open
/// popup (a tool group's flyout among them), whatever opened the window: a press in the window, a
/// pick in the macOS menu bar, an automation command or a run that ends in a dialog. Popups the
/// window itself opens later stay open. Called after every window of the pass has drawn.
pub(crate) fn close_overlays_for_modal(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let up = modal_window_up(app);
    let was = ctx.data_mut(|d| std::mem::replace(d.get_temp_mut_or_default::<bool>(modal_window_up_id()), up));
    if up {
        app.ui.palette_open = false;
    }
    if up && !was {
        app.ui.tools_drawer = false;
        craft::ui::runtime::overlay::close_popups(ctx);
    }
}

mod peek;
mod sizes;

#[cfg(test)]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests;
