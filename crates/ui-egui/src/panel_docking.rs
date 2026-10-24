//! Application-owned panel IDs and commands around the shared docking layout.

use craft_ui::docking::{Action, Floating, Layout, Node, Placement, Zone};
use serde_json::{Value, json};

use crate::VectorcraftApp;

#[derive(Clone, Default)]
struct LegacyDrag {
    panel: String,
    layout: Option<Layout<String>>,
    open_panel: Option<String>,
    floating: Vec<crate::state::FloatingPanels>,
    icons: std::collections::BTreeSet<String>,
    collapsed: std::collections::BTreeSet<String>,
}

#[derive(Clone)]
struct RootGroupDrag {
    panel: String,
    layout: Layout<String>,
    hidden: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
    collapsed: std::collections::BTreeSet<String>,
    open_panel: Option<String>,
    generation: u64,
}

fn cancel_legacy_drag(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let key = egui::Id::new("vectorcraft-panel-docking").with("legacy-origin");
    let previous = ctx.data_mut(|data| data.remove_temp::<LegacyDrag>(key));
    if let Some(previous) = previous {
        app.ui.open_panel = previous.open_panel;
        app.ui.floating_panels = previous.floating;
        app.ui.docking = previous.layout;
        app.ui.docking_icons = previous.icons;
        app.ui.docking_collapsed = previous.collapsed;
        craft_ui::docking::cancel_drag::<String>(ctx, egui::Id::new("vectorcraft-panel-docking"));
        true
    } else {
        false
    }
}

fn normalize(raw: &str) -> Option<&'static str> {
    crate::menus::normalize_panel(raw)
}

pub(crate) fn valid(layout: &Layout<String>) -> bool {
    layout.validate().is_ok() && layout.panels().iter().all(|id| normalize(id) == Some(id.as_str()))
}

fn legacy(app: &VectorcraftApp) -> Layout<String> {
    let panels: Vec<String> = crate::floating::docked_tabs(&app.ui).map(|tab| tab.info().0.to_string()).collect();
    let active = panels.iter().position(|id| id == app.ui.dock_tab.info().0).unwrap_or(0);
    let root = (!panels.is_empty()).then_some(Node::Tabs { panels, active });
    let mut layout = Layout { root, floating: Vec::new() };
    for group in &app.ui.floating_panels {
        let panels: Vec<String> = group.panels.iter().filter(|id| normalize(id).is_some() && !layout.contains(id)).cloned().collect();
        if !panels.is_empty() && group.pos.iter().all(|v| v.is_finite()) {
            let active = group.active.min(panels.len().saturating_sub(1));
            layout.floating.push(Floating {
                panels,
                active,
                rect: [group.pos[0], group.pos[1], group.width.unwrap_or(300.0).clamp(200.0, 720.0), 400.0],
            });
        }
    }
    layout
}

fn icon_origins(app: &VectorcraftApp) -> std::collections::BTreeSet<String> {
    let mut icons = app.ui.docking_icons.clone();
    if app.ui.docking.is_none() {
        // Legacy workspaces have no custom groups in the dock. Icon-panel floats therefore
        // retain their native rail destination when first converted to a shared layout.
        icons.extend(
            app.ui
                .floating_panels
                .iter()
                .flat_map(|group| group.panels.iter())
                .filter(|id| crate::state::ICON_PANELS.iter().any(|entry| entry.0 == id.as_str()))
                .cloned(),
        );
    }
    icons
}

fn first_root(layout: &Layout<String>, except: &str) -> Option<String> {
    let root = Layout { root: layout.root.clone(), floating: Vec::new() };
    root.panels().into_iter().find(|p| p.as_str() != except).cloned()
}

// Preserve the native dock's selected tab when another tab leaves. If the selected tab
// itself leaves, the remaining group's first tab is the native fallback.
fn float_fallback(layout: &Layout<String>, panel: &str) -> Option<String> {
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Tabs { panels, active } if panels.iter().any(|id| id == panel) => {
                return panels.get(*active).filter(|id| id.as_str() != panel).or_else(|| panels.iter().find(|id| id.as_str() != panel)).cloned();
            }
            Node::Split { first, second, .. } => pending.extend([second.as_ref(), first.as_ref()]),
            _ => {}
        }
    }
    None
}

fn restore_float_fallback(layout: &mut Layout<String>, panel: Option<String>) -> Result<(), String> {
    if let Some(panel) = panel.filter(|panel| Layout { root: layout.root.clone(), floating: Vec::new() }.contains(panel)) {
        layout.apply(Action::Activate { panel }).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn ensure(layout: &mut Layout<String>, panel: &str) -> Result<(), String> {
    let anchor = first_root(layout, panel);
    layout.apply(Action::Open { panel: panel.into(), anchor }).map_err(|e| e.to_string())
}

fn group_members(layout: &Layout<String>, panel: &str) -> Vec<String> {
    if let Some(group) = layout.floating.iter().find(|g| g.panels.iter().any(|id| id == panel)) {
        return group.panels.clone();
    }
    let mut visit: Vec<_> = layout.root.iter().collect();
    while let Some(node) = visit.pop() {
        match node {
            Node::Tabs { panels, .. } if panels.iter().any(|id| id == panel) => return panels.clone(),
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            _ => {}
        }
    }
    vec![panel.into()]
}

fn number(params: &Value, key: &str, default: f32) -> Result<f32, String> {
    match params.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|v| v.is_finite() && v.abs() <= 1_000_000.0)
            .map(|v| v as f32)
            .ok_or_else(|| format!("{key} must be a finite coordinate")),
    }
}

const LEGACY_REORDER_BAD_DEST: &str = "move requires an existing destination panel";

enum LegacyStripReorder {
    DockStrip,
    FloatingGroup(usize),
}

fn legacy_strip_reorder_target(
    app: &VectorcraftApp,
    before: &str,
    dock_strip: bool,
    panel_group: Option<usize>,
) -> Result<LegacyStripReorder, String> {
    if dock_strip {
        return Ok(LegacyStripReorder::DockStrip);
    }
    let gi = panel_group.ok_or(LEGACY_REORDER_BAD_DEST)?;
    if crate::floating::group_of(&app.ui, before) != Some(gi) {
        return Err(LEGACY_REORDER_BAD_DEST.into());
    }
    Ok(LegacyStripReorder::FloatingGroup(gi))
}

/// Legacy workspaces (no saved `docking` layout): reorder tabs in the dock strip or a floating
/// group without promoting to a shared layout.
fn legacy_same_strip_reorder(app: &mut VectorcraftApp, panel: &str, before: &str, target: LegacyStripReorder) -> Result<Value, String> {
    use crate::state::DockTab;
    match target {
        LegacyStripReorder::DockStrip => {
            let mut order: Vec<DockTab> = crate::floating::docked_tabs(&app.ui).collect();
            let from = order.iter().position(|t| t.info().0 == panel).ok_or(LEGACY_REORDER_BAD_DEST)?;
            let to = order.iter().position(|t| t.info().0 == before).ok_or(LEGACY_REORDER_BAD_DEST)?;
            if from != to {
                let item = order.remove(from);
                order.insert(to, item);
                crate::floating::set_dock_tab_order(&mut app.ui, order);
            }
        }
        LegacyStripReorder::FloatingGroup(gi) => {
            let g = app.ui.floating_panels.get_mut(gi).ok_or("invalid restored panel group")?;
            let from = g.panels.iter().position(|id| id == panel).ok_or(LEGACY_REORDER_BAD_DEST)?;
            let to = g.panels.iter().position(|id| id == before).ok_or(LEGACY_REORDER_BAD_DEST)?;
            crate::floating::reorder_group_tab_indices(g, from, to);
            if let Some(layout) = app.ui.docking.as_mut() {
                let panels = g.panels.clone();
                let active = g.active.min(panels.len().saturating_sub(1));
                if let Some(group) = layout.floating.iter_mut().find(|group| group.panels.iter().any(|id| id == panel)) {
                    group.panels = panels;
                    group.active = active;
                }
            }
        }
    }
    Ok(json!({ "panel": panel, "before": before }))
}

/// Same-strip tab reorder for `window.panel.move` and in-frame tab drags (`app.run` during egui deadlocks).
pub(crate) fn strip_tab_reorder(app: &mut VectorcraftApp, panel: &str, before: &str) -> Result<Value, String> {
    use crate::state::DockTab;
    let panel_group = crate::floating::group_of(&app.ui, panel);
    let dock_strip_reorder = DockTab::from_id(panel).is_some()
        && DockTab::from_id(before).is_some()
        && panel_group.is_none()
        && crate::floating::group_of(&app.ui, before).is_none();
    let target = legacy_strip_reorder_target(app, before, dock_strip_reorder, panel_group)?;
    legacy_same_strip_reorder(app, panel, before, target)
}

fn placement(params: &Value) -> Result<Placement<String>, String> {
    if let Some(before) = params.get("before") {
        let id = before.as_str().and_then(normalize).ok_or("before must name a panel")?;
        return Ok(Placement::Tab { before: Some(id.into()) });
    }
    let zone = match params.get("zone") {
        None => "tab",
        Some(value) => value.as_str().ok_or("zone must be a string")?,
    };
    Ok(match zone {
        "tab" => Placement::Tab { before: None },
        "center" => Placement::Split(Zone::Center),
        "left" => Placement::Split(Zone::Left),
        "right" => Placement::Split(Zone::Right),
        "top" => Placement::Split(Zone::Top),
        "bottom" => Placement::Split(Zone::Bottom),
        _ => return Err("zone must be tab, center, left, right, top or bottom".into()),
    })
}

pub(crate) fn command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.collapse" {
        return Some(collapse(app, params));
    }
    // Docking into the dock's tree or its icon column is the shared layout's, whatever set the
    // panel floats in. A dock column is the native sets', so it doesn't combine with either.
    let to_icons = params.get("icon").is_some_and(|icon| icon != &Value::Bool(false));
    let into_tree = id == "window.panel.dock" && (params.get("anchor").is_some() || to_icons);
    if into_tree && params.get("column").is_some() {
        return Some(Err("column docks a set as a dock column, with no anchor or icon".into()));
    }
    if matches!(id, "window.panel.float" | "window.panel.dock")
        && !into_tree
        && (params.get("above").is_some()
            || params.get("below").is_some()
            || params.get("column").is_some()
            || params.get("collapsed").is_some()
            || params
                .get("onto")
                .and_then(Value::as_str)
                .and_then(normalize)
                .and_then(|id| crate::floating::group_of(&app.ui, id))
                .and_then(|i| app.ui.floating_panels.get(i))
                .is_some_and(decorated)
            || params
                .get("panel")
                .and_then(Value::as_str)
                .and_then(normalize)
                .and_then(|id| crate::floating::group_of(&app.ui, id))
                .and_then(|i| app.ui.floating_panels.get(i))
                .is_some_and(decorated))
    {
        return Some(native_change(app, id, params));
    }
    if id == "window.panel.layout" {
        return Some(
            params
                .get("action")
                .ok_or_else(|| "action is required".to_string())
                .and_then(|value| serde_json::from_value::<Action<String>>(value.clone()).map_err(|error| error.to_string()))
                .and_then(|action| apply_action(app, action)),
        );
    }
    if params.get("panel").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("tools") || s.eq_ignore_ascii_case("toolbar")) {
        return None;
    }
    let operation = match id {
        "window.panel.move" => "move",
        "window.panel.float" => "float",
        "window.panel.dock" => "dock",
        "window.panel.activate" => "activate",
        "window.panel.close" => "close",
        "window.panel" if app.ui.docking.is_some() => "activate",
        _ => return None,
    };
    Some(change(app, operation, params).map(|value| {
        if id == "window.panel"
            && let Some(panel) = params.get("panel").and_then(Value::as_str).and_then(normalize)
            && let Some(group) = app.ui.floating_panels.iter().find(|group| group.panels.iter().any(|id| id == panel))
        {
            return json!({"floating":group.panels});
        }
        value
    }))
}

// UI output and automation share this atomic dispatcher. The application owns the registry
// and saved return locations; craft-ui owns tree validation and structural changes.
fn apply_action(app: &mut VectorcraftApp, action: Action<String>) -> Result<Value, String> {
    let registered = |id: &String| normalize(id) == Some(id.as_str());
    let known = match &action {
        Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } => {
            registered(panel)
                && registered(anchor)
                && match placement {
                    Placement::Tab { before: Some(id) } => registered(id),
                    _ => true,
                }
        }
        Action::Open { panel, anchor } => registered(panel) && anchor.as_ref().is_none_or(registered),
        Action::Float { panel, .. }
        | Action::Close { panel }
        | Action::Activate { panel }
        | Action::MoveFloating { panel, .. }
        | Action::SetStackOpen { panel, .. }
        | Action::ResizeStack { panel, .. } => registered(panel),
        Action::ResizeSplit { path, .. } => path.len() <= craft_ui::docking::MAX_DEPTH,
    };
    if !known {
        return Err("docking action contains an unknown panel or an excessive split path".into());
    }
    if app.ui.docking.as_ref().is_some_and(|layout| !valid(layout)) {
        return Err("saved panel layout is invalid; reset the workspace first".into());
    }
    let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
    let mut icons = icon_origins(app);
    let explicit_dock = match &action {
        Action::Move { panel, anchor, .. } | Action::OpenAt { panel, anchor, .. }
            if (Layout { root: layout.root.clone(), floating: Vec::new() }).contains(anchor) =>
        {
            Some(panel.clone())
        }
        _ => None,
    };
    let returning = match &action {
        Action::Close { panel } => layout.location(panel).ok().map(|location| (panel.clone(), location)),
        Action::Float { panel, .. } => {
            layout.location(panel).ok().filter(|location| location.floating.is_none()).map(|location| (panel.clone(), location))
        }
        _ => None,
    };
    let fallback = match &action {
        Action::Float { panel, .. } => float_fallback(&layout, panel),
        _ => None,
    };
    let activated = match &action {
        Action::Activate { panel } => Some(panel.clone()),
        _ => None,
    };
    // A panel moved, floated or closed leaves its group: that group isn't collapsed any more, and
    // the panel doesn't take the collapse along.
    let moved = match &action {
        Action::Move { panel, .. }
        | Action::OpenAt { panel, .. }
        | Action::Open { panel, .. }
        | Action::Float { panel, .. }
        | Action::Close { panel } => Some(panel.clone()),
        _ => None,
    };
    layout.apply(action).map_err(|error| error.to_string())?;
    restore_float_fallback(&mut layout, fallback)?;
    if let Some((panel, location)) = returning {
        app.ui.docking_hidden.insert(panel, location);
    }
    if let Some(panel) = explicit_dock {
        icons.remove(&panel);
    }
    app.ui.docking_icons = icons;
    app.ui.docking = Some(layout);
    if let Some(panel) = moved {
        app.ui.docking_collapsed.remove(&panel);
    }
    if let Some(panel) = activated {
        for group in &mut app.ui.floating_panels {
            if group.panels.contains(&panel) {
                group.collapsed = false;
            }
        }
    }
    sync_legacy(app);
    app.ui.open_panel = None;
    Ok(json!({"layout": app.ui.docking}))
}

/// The panels shown by the dock's groups that can collapse to their tabs: each group of the shared
/// layout's tree that is the whole tree or above or below another (one beside another can't be
/// shorter than it), or the tabbed group of the legacy dock.
fn dock_groups_shown(ui: &crate::state::UiState) -> Vec<String> {
    let Some(layout) = &ui.docking else {
        return crate::floating::shown_tab(ui).map(|tab| tab.info().0.to_string()).into_iter().collect();
    };
    let mut out = Vec::new();
    // Each node, and whether it may be shortened (the root, or in a top/bottom split).
    let mut pending: Vec<(&Node<String>, bool)> = layout.root.iter().map(|root| (root, true)).collect();
    while let Some((node, shortens)) = pending.pop() {
        match node {
            Node::Tabs { panels, active } if shortens => out.extend(panels.get(*active).cloned()),
            Node::Split { axis, first, second, .. } => {
                let vertical = *axis == craft_ui::layout::SplitAxis::Vertical;
                pending.extend([(second.as_ref(), vertical), (first.as_ref(), vertical)]);
            }
            _ => {}
        }
    }
    out
}

/// Whether the group of the shared layout's tree holding `panel` as a tab can be shortened to its
/// tabs (the whole tree, or above or below another group); `None` when `panel` isn't a tab of one
/// (it floats, or is in an accordion).
fn tab_group_shortens(layout: &Layout<String>, panel: &str) -> Option<bool> {
    let mut pending: Vec<(&Node<String>, bool)> = layout.root.iter().map(|root| (root, true)).collect();
    while let Some((node, shortens)) = pending.pop() {
        match node {
            Node::Tabs { panels, .. } if panels.iter().any(|id| id == panel) => return Some(shortens),
            Node::Split { axis, first, second, .. } => {
                let vertical = *axis == craft_ui::layout::SplitAxis::Vertical;
                pending.extend([(second.as_ref(), vertical), (first.as_ref(), vertical)]);
            }
            _ => {}
        }
    }
    None
}

/// Forget the collapsed groups that show another panel now (another tab clicked, or the panel
/// moved or closed): those are expanded.
pub(crate) fn prune_collapsed(ui: &mut crate::state::UiState) {
    if ui.docking_collapsed.is_empty() {
        return;
    }
    let shown = dock_groups_shown(ui);
    ui.docking_collapsed.retain(|id| shown.contains(id));
}

/// `window.panel.collapse`: show the dock group holding `panel` as its tabs only, or expanded.
fn collapse(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
    let collapsed = crate::menus::opt_bool(params, "collapsed")?;
    if crate::floating::group_of(&app.ui, panel).is_some() {
        return Err("panel floats; collapse its group with window.panel.float {panel, collapsed}".into());
    }
    match app.ui.docking.as_mut() {
        Some(layout) => {
            match tab_group_shortens(layout, panel) {
                None => return Err("panel is not in a dock group".into()),
                Some(false) => return Err("only a dock group above or below others collapses, not one beside another".into()),
                Some(true) => {}
            }
            layout.apply(Action::Activate { panel: panel.into() }).map_err(|e| e.to_string())?;
            sync_legacy(app);
        }
        None => {
            let tab = crate::state::DockTab::from_id(panel)
                .filter(|_| crate::floating::docked_tabs(&app.ui).any(|t| t.info().0 == panel))
                .ok_or("panel is not in a dock group")?;
            app.ui.dock_tab = tab;
        }
    }
    prune_collapsed(&mut app.ui);
    let now = collapsed.unwrap_or(!app.ui.docking_collapsed.contains(panel));
    if now {
        app.ui.docking_collapsed.insert(panel.into());
    } else {
        app.ui.docking_collapsed.remove(panel);
    }
    Ok(json!({"panel": panel, "collapsed": now}))
}

/// `node` as drawn with its collapsed groups (showing a panel in `collapsed`) as tall as their
/// tabs in a top/bottom split. The paths of the splits so sized go in `fixed`, the panels of
/// collapsed groups in `members`. Also returns the node's height when it is only collapsed groups.
fn collapse_node(
    node: &Node<String>,
    collapsed: &std::collections::BTreeSet<String>,
    (tab, gap): (f32, f32),
    path: &mut Vec<bool>,
    fixed: &mut Vec<Vec<bool>>,
    members: &mut std::collections::BTreeSet<String>,
) -> (Node<String>, Option<f32>) {
    use craft_ui::layout::{SplitAxis, SplitSize};
    match node {
        Node::Tabs { panels, active } if panels.get(*active).is_some_and(|id| collapsed.contains(id)) => {
            members.extend(panels.iter().cloned());
            (node.clone(), Some(tab))
        }
        Node::Split { axis, size, first, second } => {
            path.push(false);
            let (a, ha) = collapse_node(first, collapsed, (tab, gap), path, fixed, members);
            path.pop();
            path.push(true);
            let (b, hb) = collapse_node(second, collapsed, (tab, gap), path, fixed, members);
            path.pop();
            let (sized, height) = match (axis, ha, hb) {
                (SplitAxis::Vertical, Some(ha), Some(hb)) => (Some(SplitSize::FixedFirst(ha)), Some(ha + hb + gap)),
                (SplitAxis::Vertical, Some(ha), None) => (Some(SplitSize::FixedFirst(ha)), None),
                (SplitAxis::Vertical, None, Some(hb)) => (Some(SplitSize::FixedSecond(hb)), None),
                _ => (None, None),
            };
            if sized.is_some() {
                fixed.push(path.clone());
            }
            (Node::Split { axis: *axis, size: sized.unwrap_or(*size), first: Box::new(a), second: Box::new(b) }, height)
        }
        _ => (node.clone(), None),
    }
}

fn change(app: &mut VectorcraftApp, operation: &str, params: &Value) -> Result<Value, String> {
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
    if operation == "move" && params.get("before").is_some() {
        let before = params.get("before").and_then(Value::as_str).and_then(normalize).ok_or("before must name a panel")?;
        use crate::state::DockTab;
        let panel_group = crate::floating::group_of(&app.ui, panel);
        let dock_strip_reorder = DockTab::from_id(panel).is_some()
            && DockTab::from_id(before).is_some()
            && panel_group.is_none()
            && crate::floating::group_of(&app.ui, before).is_none();
        if app.ui.docking.is_none() || panel_group.is_some() || dock_strip_reorder {
            return strip_tab_reorder(app, panel, before);
        }
    }
    if app.ui.docking.as_ref().is_some_and(|layout| !valid(layout)) {
        return Err("saved panel layout is invalid; reset the workspace first".into());
    }
    let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
    if params.get("x").is_some() != params.get("y").is_some() {
        return Err("x and y must be provided together".into());
    }
    let whole_group = match params.get("group") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("group must be true or false".into()),
    };
    let to_icons = match params.get("icon") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("icon must be true or false".into()),
    };
    if to_icons && (operation != "dock" || params.get("anchor").or_else(|| params.get("onto")).is_some()) {
        return Err("icon only docks a panel in the icon column, with no anchor".into());
    }
    let members = if whole_group { group_members(&layout, panel) } else { vec![panel.to_string()] };
    let mut hidden: std::collections::BTreeMap<_, _> =
        app.ui.docking_hidden.iter().filter(|(id, _)| normalize(id).is_some()).take(64).map(|(id, at)| (id.clone(), at.clone())).collect();
    let mut icons = icon_origins(app);
    if to_icons {
        // Any panel the icon column can show lives there once dropped on it, the tabbed group's
        // too; the others (library panels) dock as they would without `icon`.
        icons.extend(members.iter().filter(|id| crate::state::rail_panel(id).is_some()).cloned());
    }
    let icon_panel = crate::state::ICON_PANELS.iter().any(|entry| entry.0 == panel) || icons.contains(panel);
    if icon_panel && !layout.contains(&panel.to_string()) {
        icons.insert(panel.to_string());
    }
    if operation == "activate" && icon_panel && !layout.contains(&panel.to_string()) {
        app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(panel) { None } else { Some(panel.into()) };
        return Ok(json!({"panel": panel, "open": app.ui.open_panel}));
    }
    if operation == "dock" && params.get("anchor").or_else(|| params.get("onto")).is_none() {
        let mut candidate = layout.clone();
        let mut open = None;
        for member in &members {
            if candidate.contains(member) {
                candidate.apply(Action::Close { panel: member.clone() }).map_err(|error| error.to_string())?;
            }
        }
        // Remove the complete group before restoring any saved neighbor. Otherwise a
        // still-floating group member can accidentally receive the first restored tab.
        for member in &members {
            if icons.contains(member) {
                open = Some(member.clone());
            } else if let Some(location) = hidden.get(member).filter(|location| {
                location.floating.is_none()
                    && location.anchor.as_ref().is_none_or(|anchor| Layout { root: candidate.root.clone(), floating: Vec::new() }.contains(anchor))
            }) {
                if candidate.restore(member.clone(), location).is_err() {
                    // A removed neighboring group uses the normal dock as its documented fallback.
                    ensure(&mut candidate, member)?;
                }
            } else {
                ensure(&mut candidate, member)?;
            }
        }
        if candidate.contains(&panel.to_string()) {
            candidate.apply(Action::Activate { panel: panel.to_string() }).map_err(|error| error.to_string())?;
        } else if icons.contains(panel) {
            open = Some(panel.to_string());
        }
        if !valid(&candidate) {
            return Err("invalid restored panel group".into());
        }
        app.ui.docking = Some(candidate);
        app.ui.docking_icons = icons;
        for member in &members {
            hidden.remove(member);
        }
        app.ui.docking_hidden = hidden;
        sync_legacy(app);
        app.ui.open_panel = open;
        return Ok(json!({"panel": panel, "floating": false, "layout": app.ui.docking}));
    }
    let mut restored = false;
    if operation == "dock"
        && params.get("anchor").or_else(|| params.get("onto")).is_none()
        && let Some(location) = hidden.get(panel).filter(|location| {
            location.floating.is_none()
                && location.anchor.as_ref().is_none_or(|anchor| Layout { root: layout.root.clone(), floating: Vec::new() }.contains(anchor))
        })
    {
        let mut candidate = layout.clone();
        if candidate.contains(&panel.to_string()) {
            candidate.apply(Action::Close { panel: panel.into() }).map_err(|e| e.to_string())?;
        }
        if candidate.restore(panel.into(), location).is_ok() {
            layout = candidate;
            restored = true;
            hidden.remove(panel);
        }
    }
    match operation {
        "dock" if restored => {}
        "float" => {
            let old = layout
                .floating
                .iter()
                .find(|group| group.panels.iter().any(|id| id == panel) && group.panels.len() == members.len())
                .map(|group| group.rect)
                .unwrap_or([400.0 + 24.0 * layout.floating.len().min(12) as f32, 140.0 + 24.0 * layout.floating.len().min(12) as f32, 300.0, 400.0]);
            let rect =
                [number(params, "x", old[0])?, number(params, "y", old[1])?, number(params, "width", old[2])?, number(params, "height", old[3])?];
            let fallback = float_fallback(&layout, panel).or_else(|| first_root(&layout, panel));
            for member in &members {
                if let Ok(location) = layout.location(member)
                    && location.floating.is_none()
                {
                    hidden.insert(member.clone(), location);
                }
            }
            ensure(&mut layout, panel)?;
            if let Ok(location) = layout.location(&panel.to_string())
                && location.floating.is_none()
            {
                hidden.insert(panel.into(), location);
            }
            if let Some(onto) = params.get("onto") {
                let onto = onto.as_str().and_then(normalize).ok_or("onto must name a floating panel")?;
                if members.iter().any(|id| id == onto)
                    || layout.floating.iter().any(|group| group.panels.iter().any(|id| id == panel) && group.panels.iter().any(|id| id == onto))
                    || !layout.floating.iter().any(|group| group.panels.iter().any(|id| id == onto))
                {
                    return Err("onto must name a panel floating in another group".into());
                }
                for member in &members {
                    layout
                        .apply(Action::Move { panel: member.clone(), anchor: onto.into(), placement: Placement::Tab { before: None } })
                        .map_err(|e| e.to_string())?;
                }
            } else {
                let lead = members.first().ok_or("panel group is empty")?;
                layout.apply(Action::Float { panel: lead.clone(), rect }).map_err(|e| e.to_string())?;
                for member in members.iter().filter(|id| *id != lead) {
                    layout
                        .apply(Action::Move { panel: member.clone(), anchor: lead.clone(), placement: Placement::Tab { before: None } })
                        .map_err(|e| e.to_string())?;
                }
            }
            layout.apply(Action::Activate { panel: panel.into() }).map_err(|e| e.to_string())?;
            restore_float_fallback(&mut layout, fallback)?;
        }
        "move" | "dock" => {
            if operation == "move" && params.get("anchor").or_else(|| params.get("onto")).is_none() {
                return Err("move requires a destination panel".into());
            }
            let anchor = match params.get("anchor").or_else(|| params.get("onto")) {
                Some(v) => Some(v.as_str().and_then(normalize).ok_or("anchor must name a registered panel")?.to_string()),
                None => first_root(&layout, panel),
            };
            let placement = placement(params)?;
            if let Some(anchor) = anchor {
                ensure(&mut layout, panel)?;
                layout.apply(Action::Move { panel: panel.into(), anchor, placement }).map_err(|e| e.to_string())?;
            } else if operation == "dock" {
                if layout.contains(&panel.to_string()) {
                    layout.apply(Action::Close { panel: panel.into() }).map_err(|e| e.to_string())?;
                }
                layout.apply(Action::Open { panel: panel.into(), anchor: None }).map_err(|e| e.to_string())?;
            } else {
                return Err("move requires an existing destination panel".into());
            }
        }
        "close" => {
            hidden.insert(panel.into(), layout.location(&panel.to_string()).map_err(|e| e.to_string())?);
            layout.apply(Action::Close { panel: panel.into() }).map_err(|e| e.to_string())?;
        }
        _ => {
            if !layout.contains(&panel.to_string())
                && let Some(location) = hidden.get(panel)
            {
                // A missing neighbor is allowed: the normal visible dock becomes the fallback.
                let _ = layout.restore(panel.into(), location);
            }
            ensure(&mut layout, panel)?;
            hidden.remove(panel);
        }
    }
    if operation == "dock" && whole_group {
        for member in members.iter().filter(|id| id.as_str() != panel) {
            layout
                .apply(Action::Move { panel: member.clone(), anchor: panel.into(), placement: Placement::Tab { before: None } })
                .map_err(|e| e.to_string())?;
        }
        layout.apply(Action::Activate { panel: panel.into() }).map_err(|e| e.to_string())?;
    }
    let group = layout.floating.iter().find(|group| group.panels.iter().any(|id| id == panel));
    let result = json!({"panel": panel, "floating": group.is_some(), "group": group.map(|g| &g.panels), "pos": group.map(|g| [g.rect[0], g.rect[1]]), "layout": layout});
    if matches!(operation, "move" | "dock") && (Layout { root: layout.root.clone(), floating: Vec::new() }).contains(&panel.to_string()) {
        icons.remove(panel);
    }
    app.ui.docking = Some(layout);
    app.ui.docking_hidden = hidden;
    app.ui.docking_icons = icons;
    if operation != "activate" {
        app.ui.docking_collapsed.remove(panel);
        for member in &members {
            app.ui.docking_collapsed.remove(member);
        }
    }
    if operation == "activate" {
        for group in &mut app.ui.floating_panels {
            if group.panels.iter().any(|id| id == panel) {
                group.collapsed = false;
            }
        }
    }
    sync_legacy(app);
    app.ui.open_panel = None;
    app.ui.dock = true;
    if matches!(operation, "activate" | "move" | "dock") {
        app.ui.dock_collapsed = false;
    }
    Ok(result)
}

struct PanelContent<'a> {
    app: &'a mut VectorcraftApp,
    commands: Vec<(&'static str, Value)>,
    /// The panels of groups collapsed to their tabs: no body.
    shut: std::collections::BTreeSet<String>,
}

impl craft_ui::docking::DockContent<String> for PanelContent<'_> {
    fn body(&mut self, ui: &mut egui::Ui, panel: &String) {
        if self.shut.contains(panel) {
            return;
        }
        let t = crate::theme::Tokens::get(ui.ctx());
        let back = ui.max_rect();
        if back.is_positive() {
            ui.painter().rect_filled(back, 0.0, t.panel);
        }
        let width = ui.available_width();
        let height = (ui.available_height() - 20.0).max(0.0);
        if crate::state::DockTab::from_id(panel).is_some() {
            crate::dock::panel_body(self.app, ui, panel, width, height);
        } else {
            // The body is as wide as the scroll area leaves it beside its scroll bar.
            crate::widgets::panel_scroll(ui, egui::ScrollArea::vertical().id_salt(("shared-panel-body", panel)).auto_shrink([false, false]), |ui| {
                let width = ui.available_width();
                crate::dock::panel_body(self.app, ui, panel, width, height)
            });
        }
    }

    fn panel_menu(&mut self, ui: &mut egui::Ui, panel: &String) {
        crate::panels::panel_menu_items(self.app, ui, panel);
    }

    fn group_header_width(&self, _panels: &[String], _active: &String, floating: bool) -> f32 {
        if floating { 48.0 } else { 80.0 }
    }

    fn group_header(&mut self, ui: &mut egui::Ui, _panels: &[String], active: &String, floating: bool) {
        let rect = ui.max_rect();
        let menu = egui::Rect::from_center_size(rect.right_center() - egui::vec2(12.0, 0.0), egui::vec2(16.0, 16.0));
        crate::panels::panel_menu(self.app, ui, active, menu);
        let grip = egui::Rect::from_min_max(rect.min, egui::pos2(menu.left() - 4.0, rect.bottom()));
        if floating {
            let response = ui.put(grip, egui::Button::new("×")).on_hover_text(tl!("Dock Panel"));
            if response.clicked() {
                self.commands.push(("window.panel.dock", json!({"panel": active, "group": true})));
            }
        } else {
            // Click as well as drag, so a double-click here collapses the group
            // (`tab_area_double_clicked`) rather than floating it on the first press.
            let response = ui.interact(grip, ui.id().with("dock-group-bar"), egui::Sense::click_and_drag());
            if response.drag_started()
                && let Some(pos) = response.interact_pointer_pos()
            {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("vectorcraft-root-group-drag"),
                        RootGroupDrag {
                            panel: active.clone(),
                            layout: self.app.ui.docking.clone().unwrap_or_else(|| legacy(self.app)),
                            hidden: self.app.ui.docking_hidden.clone(),
                            collapsed: self.app.ui.docking_collapsed.clone(),
                            open_panel: self.app.ui.open_panel.clone(),
                            generation: self.app.ui.docking_generation,
                        },
                    )
                });
                self.commands.push(("window.panel.float", json!({"panel": active, "group": true, "x": pos.x, "y": pos.y})));
            }
            response.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to float the panel group"));
        }
    }
}

/// The command that puts `panel` (with its group when `whole_group`) where a pointer drop on
/// `target` puts it.
fn drop_command(panel: &str, target: crate::floating::Drop, whole_group: bool) -> (&'static str, Value) {
    use crate::floating::Drop;
    match target {
        Drop::Dock => ("window.panel.dock", json!({"panel": panel, "group": whole_group})),
        Drop::Stack(onto) => ("window.panel.float", json!({"panel": panel, "group": whole_group, "onto": onto})),
        Drop::Below(onto) => ("window.panel.float", json!({"panel": panel, "group": whole_group, "below": onto})),
        Drop::Above(onto) => ("window.panel.float", json!({"panel": panel, "group": whole_group, "above": onto})),
        Drop::Column(slot) => ("window.panel.dock", json!({"panel": panel, "group": whole_group, "column": slot})),
        Drop::Root(anchor, zone) => ("window.panel.dock", json!({"panel": panel, "group": whole_group, "anchor": anchor, "zone": zone.id()})),
        Drop::Icons => ("window.panel.dock", json!({"panel": panel, "group": whole_group, "icon": true})),
    }
}

/// The groups of the shared dock's tree as drawn in `content` (the rect its tree was laid out in),
/// from where its tabs were drawn: a group's tabs start at its top-left corner, and the groups of
/// a split meet where the second one's tabs start.
fn root_groups(ctx: &egui::Context, layout: &Layout<String>, content: egui::Rect, tab_height: f32) -> Vec<crate::floating::RootGroup> {
    let area = egui::Id::new("vectorcraft-panel-docking");
    let tab = |panel: &String| ctx.read_response(area.with(("tab", panel))).map(|response| response.rect).filter(|r| r.is_positive());
    /// The top-left corner of a node's first group, from where its tabs were drawn.
    fn corner(node: &Node<String>, tab: &dyn Fn(&String) -> Option<egui::Rect>) -> Option<egui::Pos2> {
        match node {
            Node::Tabs { panels, .. } => panels.iter().filter_map(tab).map(|r| r.min).reduce(|a, b| a.min(b)),
            Node::Stack { entries } => entries.first().and_then(|entry| tab(&entry.panel)).map(|r| r.min),
            Node::Split { first, .. } => corner(first, tab),
        }
    }
    let mut out = Vec::new();
    let mut pending: Vec<(&Node<String>, egui::Rect)> = layout.root.iter().map(|root| (root, content)).collect();
    while let Some((node, rect)) = pending.pop() {
        if out.len() >= craft_ui::docking::MAX_PANELS || !rect.is_positive() {
            break;
        }
        match node {
            Node::Tabs { panels, active } => {
                let ids: Vec<&'static str> = panels.iter().filter_map(|id| normalize(id)).collect();
                let Some(shown) = panels.get(*active).and_then(|id| normalize(id)).or_else(|| ids.first().copied()) else { continue };
                let strip = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), (rect.top() + tab_height).min(rect.bottom())));
                out.push(crate::floating::RootGroup { panels: ids, shown, rect, strip });
            }
            Node::Stack { entries } => {
                let ids: Vec<&'static str> = entries.iter().filter_map(|entry| normalize(&entry.panel)).collect();
                let Some(shown) = ids.first().copied() else { continue };
                let strip = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), (rect.top() + tab_height).min(rect.bottom())));
                out.push(crate::floating::RootGroup { panels: ids, shown, rect, strip });
            }
            Node::Split { axis, first, second, .. } => {
                let Some(at) = corner(second, &tab) else { continue };
                let (a, b) = match axis {
                    craft_ui::layout::SplitAxis::Vertical => {
                        let y = at.y.clamp(rect.top(), rect.bottom());
                        (egui::Rect::from_x_y_ranges(rect.x_range(), rect.top()..=y), egui::Rect::from_x_y_ranges(rect.x_range(), y..=rect.bottom()))
                    }
                    craft_ui::layout::SplitAxis::Horizontal => {
                        let x = at.x.clamp(rect.left(), rect.right());
                        (egui::Rect::from_x_y_ranges(rect.left()..=x, rect.y_range()), egui::Rect::from_x_y_ranges(x..=rect.right(), rect.y_range()))
                    }
                };
                pending.push((second, b));
                pending.push((first, a));
            }
        }
    }
    out
}

/// The panel shown by the dock group whose strip was double-clicked this frame right of its tabs
/// and left of its panel menu, from which Illustrator also collapses the group
/// (<https://helpx.adobe.com/illustrator/using/customizing-workspace.html>). Most of it has no
/// widget of its own, so the press is read from the input; `layer` is the dock's, so a press on a
/// floating group or a menu over the strip doesn't count. Accordions, and groups beside another
/// (which don't collapse), are left out.
fn tab_area_double_clicked(
    ctx: &egui::Context,
    layer: egui::LayerId,
    layout: &Layout<String>,
    groups: &[crate::floating::RootGroup],
) -> Option<&'static str> {
    let at = ctx.input(|i| if i.pointer.button_double_clicked(egui::PointerButton::Primary) { i.pointer.interact_pos() } else { None })?;
    // No layer: the background, where the dock panel is.
    if ctx.layer_id_at(at).is_some_and(|l| l != layer) {
        return None;
    }
    let group = groups.iter().find(|g| g.strip.contains(at))?;
    if tab_group_shortens(layout, group.shown) != Some(true) {
        return None;
    }
    let area = egui::Id::new("vectorcraft-panel-docking");
    let right = |id: egui::Id| ctx.read_response(id).map(|r| r.rect.right());
    let tabs = group
        .panels
        .iter()
        .filter_map(|p| right(area.with(("tab", &p.to_string()))))
        .chain(right(area.with(("overflow", &group.shown.to_string()))))
        .fold(group.strip.left(), f32::max);
    // The panel menu is 16 wide, 12 in from the right, with 4 to spare (`group_header`).
    (at.x >= tabs && at.x < group.strip.right() - 24.0).then_some(group.shown)
}

pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) -> bool {
    let root_key = egui::Id::new("vectorcraft-root-group-drag");
    if let Some(drag) = ui.ctx().data(|data| data.get_temp::<RootGroupDrag>(root_key)) {
        if drag.generation != app.ui.docking_generation {
            ui.ctx().data_mut(|data| data.remove::<RootGroupDrag>(root_key));
        } else if ui.input(|input| !input.focused || input.key_pressed(egui::Key::Escape)) {
            app.ui.docking = Some(drag.layout);
            app.ui.docking_hidden = drag.hidden;
            app.ui.docking_collapsed = drag.collapsed;
            app.ui.open_panel = drag.open_panel;
            sync_legacy(app);
            ui.ctx().data_mut(|data| data.remove::<RootGroupDrag>(root_key));
        } else if let Some(pos) = ui.input(|input| input.pointer.interact_pos()) {
            if ui.input(|input| input.pointer.primary_down()) {
                if let Err(error) = app.run("window.panel.float", json!({"panel":drag.panel, "group":true, "x":pos.x, "y":pos.y})) {
                    app.ui.status = error;
                }
                if let Some(panel) = normalize(&drag.panel)
                    && let Some((target, rect)) = crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos)
                {
                    crate::floating::preview(ui.ctx(), target, rect);
                }
            } else {
                if let Some(panel) = normalize(&drag.panel)
                    && let Some((target, _)) = crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos)
                {
                    let (id, params) = drop_command(panel, target, true);
                    if let Err(error) = app.run(id, params) {
                        app.ui.status = error;
                    }
                }
                ui.ctx().data_mut(|data| data.remove::<RootGroupDrag>(root_key));
            }
        }
    }
    let area = egui::Id::new("vectorcraft-panel-docking");
    let changed_workspace = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<u64>(area.with("generation"));
        data.insert_temp(area.with("generation"), app.ui.docking_generation);
        previous.is_some_and(|generation| generation != app.ui.docking_generation)
    });
    if changed_workspace {
        craft_ui::docking::cancel_drag::<String>(ui.ctx(), area);
        ui.ctx().data_mut(|data| data.remove::<LegacyDrag>(area.with("legacy-origin")));
    }

    let cancelled =
        ui.input(|input| !input.focused || input.key_pressed(egui::Key::Escape) || (!input.pointer.primary_down() && !input.pointer.any_released()));
    if cancelled && cancel_legacy_drag(app, ui.ctx()) && app.ui.docking.is_none() {
        return false;
    }
    let Some(layout) = app.ui.docking.as_ref() else { return false };
    if !valid(layout) {
        app.ui.docking = None;
        app.ui.status = "Invalid panel layout; restored the default dock".into();
        return false;
    }
    let layout = layout.clone();
    let t = crate::theme::Tokens::get(ui.ctx());
    let collapsed = app.ui.dock_collapsed;
    let mut display = if collapsed { Layout { root: None, floating: layout.floating.clone() } } else { layout.clone() };
    display
        .floating
        .retain(|group| !app.ui.floating_panels.iter().any(|native| decorated(native) && native.panels.iter().any(|id| group.panels.contains(id))));
    // Library panels are titled by their libraries' names.
    let library_labels: Vec<(String, String)> = layout
        .panels()
        .into_iter()
        .filter_map(|panel| crate::panels::library_panel::tab_label(app, panel).map(|label| (panel.clone(), label)))
        .collect();
    let library_label = |panel: &str| library_labels.iter().find(|(p, _)| p == panel).map(|(_, label)| label.clone());
    // Floating groups lie on the canvas, not on the dock's card: reserve a fill under each one
    // before the dock draws it, sized once it is placed, so the canvas doesn't show through (#2421).
    let fills: Vec<_> = display
        .floating
        .iter()
        .filter_map(|group| {
            let first = group.panels.first()?;
            let layer = egui::LayerId::new(egui::Order::Middle, area.with(("floating", first)));
            let [x, y, w, h] = group.rect;
            let layout_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
            let fill = ui.ctx().layer_painter(layer).add(egui::Shape::Noop);
            Some((layer, layout_rect, fill))
        })
        .collect();
    crate::dock::apply_pending_dock_width(app, ui.ctx(), crate::dock::SHARED_DOCK_PANEL, crate::dock::clamp_shared_dock_width);
    let shared_dock_default = crate::dock::shared_dock_default_width(&app.ui);
    // Splits sized to a collapsed group's tabs: their dividers don't resize the saved layout.
    let mut fixed: Vec<Vec<bool>> = Vec::new();
    let mut draw = |ui: &mut egui::Ui| {
        if layout.root.is_some() && crate::dock::collapse_header(ui, collapsed) {
            let _ = app.run("window.collapseDock", json!({"collapsed":!collapsed}));
        }
        if collapsed {
            let root = Layout { root: layout.root.clone(), floating: Vec::new() };
            for panel in root.panels() {
                let info = crate::state::DockTab::from_id(panel)
                    .map(|tab| tab.info())
                    .or_else(|| crate::state::ICON_PANELS.iter().find(|entry| entry.0 == panel).copied());
                let label = info.map(|(_, label, _)| tl!(label).to_string()).or_else(|| library_label(panel));
                if let Some(label) = label
                    && crate::widgets::icon_button(
                        ui,
                        info.map_or("library", |(_, _, icon)| icon),
                        &label,
                        app.ui.open_panel.as_ref() == Some(panel),
                        30.0,
                    )
                    .clicked()
                {
                    app.ui.open_panel = if app.ui.open_panel.as_ref() == Some(panel) { None } else { Some(panel.clone()) };
                }
            }
        }
        let mut style = craft_ui::docking::DockStyle::from_ui(ui);
        style.tab_height = 33.0;
        // The dock is a card (`theme::card`): its panes and tab strip lie on the card's colour
        // (square fills would cover its rounded corners). Floating groups get the fills below.
        style.background = egui::Color32::TRANSPARENT;
        style.tab_background = egui::Color32::TRANSPARENT;
        style.active_background = t.button;
        style.text = t.text_strong;
        style.inactive_text = t.text_dim;
        style.border = egui::Stroke::new(1.0, t.border);
        style.accent = t.accent;
        style.float_label = tl!("Float Panel").to_string();
        style.close_label = tl!("Close Panel").to_string();
        style.move_label = tl!("Group with").to_string();
        style.panels_label = tl!("Panels").to_string();
        style.resize_label = tl!("Resize panels").to_string();
        style.resize_window_label = tl!("Resize panel window").to_string();
        // Panels set their own minimum height, so a collapsed group can be as short as its tabs.
        style.min_pane = 0.0;
        let mut tree = ui.available_rect_before_wrap();
        let mut shut = std::collections::BTreeSet::new();
        let mut drawn = display.clone();
        // The height of the tree when it is only collapsed groups: drawn that tall, so no group
        // takes the rest of the dock as a blank pane.
        let mut only_tabs = None;
        if let Some(root) = &display.root {
            let (node, height) =
                collapse_node(root, &app.ui.docking_collapsed, (style.tab_height, style.gap), &mut Vec::new(), &mut fixed, &mut shut);
            drawn.root = Some(node);
            only_tabs = height.filter(|h| h.is_finite() && *h > 0.0);
        }
        let short = shut.clone();
        let mut content = PanelContent { app, commands: Vec::new(), shut };
        let mut dock = |ui: &mut egui::Ui| {
            craft_ui::docking::DockArea::new(area).show_customized(
                ui,
                &drawn,
                &style,
                |id| {
                    crate::state::all_panels()
                        .find(|p| p.0 == id)
                        .map(|p| tl!(p.1).to_string())
                        .or_else(|| library_label(id))
                        .unwrap_or_else(|| id.clone())
                },
                |_| craft_ui::docking::Permissions::default(),
                |id| craft_ui::docking::PanelLimits { min: egui::vec2(220.0, if short.contains(id) { 0.0 } else { 80.0 }), ..Default::default() },
                &mut content,
            )
        };
        let output = match only_tabs {
            Some(height) => {
                let size = egui::vec2(ui.available_width(), height.min(ui.available_height()));
                ui.allocate_ui(size, |ui| {
                    ui.set_max_height(size.y);
                    tree = ui.available_rect_before_wrap();
                    dock(ui)
                })
                .inner
            }
            None => dock(ui),
        };
        // A double-click on a tab of the dock's tree collapses its group to its tabs, or expands it.
        // A group beside another doesn't collapse, so its tabs ignore it rather than report an error.
        if let Some(root) = &drawn.root {
            let tabs = Layout { root: Some(root.clone()), floating: Vec::new() };
            if let Some(panel) =
                tabs.panels().into_iter().find(|p| ui.ctx().read_response(area.with(("tab", *p))).is_some_and(|r| r.double_clicked()))
                && tab_group_shortens(&tabs, panel) == Some(true)
            {
                content.commands.push(("window.panel.collapse", json!({"panel": panel})));
            }
        }
        let groups = if collapsed { Vec::new() } else { root_groups(ui.ctx(), &display, tree, style.tab_height) };
        if let Some(panel) = tab_area_double_clicked(ui.ctx(), ui.layer_id(), &display, &groups) {
            content.commands.push(("window.panel.collapse", json!({"panel": panel})));
        }
        ui.ctx().data_mut(|data| data.insert_temp(crate::floating::root_groups_id(), groups));
        for (id, params) in content.commands {
            if let Err(error) = app.run(id, params) {
                app.ui.status = error;
            }
        }
        output
    };
    let output = if layout.root.is_some() {
        let panel = egui::Panel::right(crate::dock::SHARED_DOCK_PANEL)
            .default_size(shared_dock_default)
            .size_range(220.0..=720.0)
            .resizable(true)
            .frame(egui::Frame::NONE);
        let panel = if collapsed { panel.exact_size(38.0).resizable(false) } else { panel };
        let shown = panel.show(ui, |ui| crate::theme::card(ui, &t, egui::Margin { left: 0, right: 6, top: 6, bottom: 6 }, &mut draw));
        ui.ctx().data_mut(|data| data.insert_temp(crate::floating::dock_rect_id(), shown.response.rect));
        crate::dock::note_dock_width(app, ui.ctx(), crate::dock::SHARED_DOCK_PANEL);
        shown.inner
    } else {
        draw(ui)
    };
    if let Some(panel) = output.group_drag.as_deref().and_then(normalize)
        && let Some(pos) = ui.input(|input| input.pointer.interact_pos())
        && let Some((target, rect)) = crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos)
    {
        crate::floating::preview(ui.ctx(), target, rect);
    }
    // A tab dragged over the icon column becomes an icon there: light the column up (the shared
    // dock shows the other drops).
    if ui.input(|input| input.pointer.primary_down())
        && let Some(pos) = ui.input(|input| input.pointer.interact_pos())
        && let Some(panel) = dragged_tab(ui.ctx(), &layout).and_then(|panel| normalize(&panel))
        && let Some((target @ crate::floating::Drop::Icons, rect)) =
            crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos)
    {
        crate::floating::preview(ui.ctx(), target, rect);
    }
    if let Some(error) = output.error {
        app.ui.status = error.to_string();
    }
    // Tab and icon tears finish as model actions; title moves finish as group_drop.
    // Native sets are absent from the shared render tree, so their pointer target must win
    // over a float proposal or a root target drawn behind the native window.
    let tab_drop = ui.input(|input| input.pointer.primary_released().then(|| input.pointer.interact_pos()).flatten()).and_then(|pos| {
        output.actions.iter().find_map(|action| {
            let panel = match action {
                Action::Float { panel, .. } | Action::Move { panel, .. } => normalize(panel)?,
                _ => return None,
            };
            crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos)
                .filter(|(target, _)| !matches!(target, crate::floating::Drop::Dock | crate::floating::Drop::Root(..)))
                .map(|(target, _)| (panel, target))
        })
    });
    if ui.input(|input| input.pointer.any_released()) {
        let native_drop = output.group_drop.as_ref().is_some_and(|(panel, pos)| {
            normalize(panel).and_then(|id| crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(id), *pos)).is_some()
        });
        if !output.actions.iter().any(|action| matches!(action, Action::Move { .. } | Action::Float { .. }))
            && !native_drop
            && tab_drop.is_none()
            && cancel_legacy_drag(app, ui.ctx())
        {
            return app.ui.docking.is_some();
        }
        craft_ui::docking::cancel_drag::<String>(ui.ctx(), egui::Id::new("vectorcraft-panel-docking"));
        ui.ctx().data_mut(|data| data.remove::<LegacyDrag>(egui::Id::new("vectorcraft-panel-docking").with("legacy-origin")));
    }
    for action in output.actions {
        if matches!(&action, Action::ResizeSplit { path, .. } if fixed.contains(path)) {
            continue;
        }
        if tab_drop
            .is_some_and(|(panel, _)| matches!(&action, Action::Float { panel: moved, .. } | Action::Move { panel: moved, .. } if moved == panel))
        {
            continue;
        }
        if let Err(error) = apply_action(app, action) {
            app.ui.status = error;
        }
    }
    let group_drop = output.group_drop.and_then(|(panel, pos)| {
        let panel = normalize(&panel)?;
        crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos).map(|(target, _)| (panel, target, true))
    });
    if let Some((panel, target, whole_group)) = group_drop.or_else(|| tab_drop.map(|(panel, target)| (panel, target, false))) {
        let (command, params) = drop_command(panel, target, whole_group);
        // Shared and native windows have already chosen their render ownership for this
        // frame. Switch representations only after both renderers finish, so a panel's
        // widgets never appear in two layers in the same frame.
        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("vectorcraft-native-panel-drop"), (command, params)));
    }
    for group in &display.floating {
        if let Some(first) = group.panels.first()
            && let Some(rect) = ui.ctx().memory(|memory| memory.area_rect(area.with(("floating", first))))
        {
            ui.ctx().data_mut(|data| data.insert_temp(crate::floating::group_rect_id(first), rect));
        }
    }
    for (layer, layout_rect, fill) in fills {
        let rect = ui.ctx().memory(|memory| memory.area_rect(layer.id)).filter(|rect| rect.is_positive()).unwrap_or(layout_rect);
        if rect.is_positive() {
            ui.ctx().layer_painter(layer).set(fill, egui::Shape::rect_filled(rect, 0.0, t.panel));
        }
    }
    icon_rail(app, ui);
    crate::floating::docked_columns(app, ui);
    true
}

/// The panel whose tab (of the shared dock, or of the legacy dock or icon column on the drag that
/// began the shared layout) is being dragged.
fn dragged_tab(ctx: &egui::Context, layout: &Layout<String>) -> Option<String> {
    let area = egui::Id::new("vectorcraft-panel-docking");
    if let Some(drag) = ctx.data(|data| data.get_temp::<LegacyDrag>(area.with("legacy-origin"))) {
        return Some(drag.panel);
    }
    let dragged = ctx.dragged_id()?;
    layout.panels().into_iter().find(|panel| area.with(("tab", *panel)) == dragged).cloned()
}

fn icon_rail(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let column = egui::Panel::right("shared-panel-icons")
        .exact_size(38.0)
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            crate::theme::card(ui, &t, egui::Margin { left: 3, right: 3, top: 6, bottom: 6 }, |ui| {
                crate::widgets::strip_scroll(ui, "shared-panel-icons-scroll", |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    // The tabbed group's panels dropped on the column head it, then the icon panels,
                    // then the other panels dropped on it (those the column has no place for).
                    let tabs: Vec<_> = crate::state::DockTab::ALL
                        .into_iter()
                        .map(crate::state::DockTab::info)
                        .filter(|info| app.ui.docking_icons.contains(info.0))
                        .collect();
                    let grouped = |id: &str| crate::state::ICON_PANEL_GROUPS.iter().any(|group| group.contains(&id));
                    let dropped: Vec<_> = crate::state::ICON_PANELS
                        .iter()
                        .filter(|entry| !grouped(entry.0) && app.ui.docking_icons.contains(entry.0))
                        .copied()
                        .collect();
                    for (id, label, icon) in tabs
                        .into_iter()
                        .chain(
                            crate::state::ICON_PANEL_GROUPS
                                .iter()
                                .flat_map(|group| group.iter())
                                .filter_map(|id| crate::state::ICON_PANELS.iter().find(|entry| entry.0 == *id).copied()),
                        )
                        .chain(dropped)
                    {
                        if app.ui.docking.as_ref().is_some_and(|layout| layout.contains(&id.to_string())) {
                            continue;
                        }
                        let (_, rect) = ui.allocate_space(egui::vec2(30.0, 30.0));
                        let response = ui.interact(rect, ui.id().with(("panel-icon", id)), egui::Sense::click_and_drag());
                        let open = app.ui.open_panel.as_deref() == Some(id);
                        let response = crate::widgets::paint_icon_button(ui, response, icon, tl!(label), open);
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), open, tl!(label)));
                        if response.clicked() {
                            app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(id) { None } else { Some(id.into()) };
                        }
                        legacy_tab(app, ui, id, &response, false);
                    }
                });
            })
        })
        .response
        .rect;
    ui.ctx().data_mut(|data| {
        let dock = if app.ui.docking.as_ref().is_some_and(|layout| layout.root.is_some()) {
            data.get_temp::<egui::Rect>(crate::floating::dock_rect_id()).unwrap_or(column)
        } else {
            column
        };
        data.insert_temp(crate::floating::dock_rect_id(), dock.union(column));
        data.insert_temp(crate::floating::icons_rect_id(), column);
        data.insert_temp(egui::Id::new("dock-icon-column-left"), column.left());
    });
}

/// A tab of the dock's tabbed group (`closable`) or an icon of the icon column: its context menu
/// floats, groups or (a tab) closes its panel, and a drag moves it in the shared layout.
pub(crate) fn legacy_tab(app: &mut VectorcraftApp, ui: &mut egui::Ui, panel: &str, response: &egui::Response, closable: bool) {
    legacy_tab_in_strip(app, ui, panel, response, closable, None);
}

/// [`legacy_tab`] for a tab of the tab strip `strip`, where a drag reorders the tabs.
pub(crate) fn legacy_tab_in_strip(
    app: &mut VectorcraftApp,
    ui: &mut egui::Ui,
    panel: &str,
    response: &egui::Response,
    closable: bool,
    strip: Option<egui::Rect>,
) {
    let Some(panel) = normalize(panel) else { return };
    response.context_menu(|ui| {
        if ui.button(tl!("Float Panel")).clicked() {
            if let Err(error) = app.run("window.panel.float", json!({"panel": panel})) {
                app.ui.status = error;
            }
            ui.close();
        }
        ui.menu_button(tl!("Group with"), |ui| {
            for (anchor, label) in crate::state::all_panels() {
                if anchor != panel && ui.button(tl!(label)).clicked() {
                    // A closed icon panel is made visible before becoming the destination.
                    let result = app
                        .run("window.panel.activate", json!({"panel": anchor}))
                        .and_then(|_| app.run("window.panel.move", json!({"panel":panel,"anchor":anchor})));
                    if let Err(error) = result {
                        app.ui.status = error;
                    }
                    ui.close();
                }
            }
        });
        if closable && ui.button(tl!("Close Panel")).clicked() {
            if let Err(error) = app.run("window.panel.close", json!({"panel": panel})) {
                app.ui.status = error;
            }
            ui.close();
        }
    });
    let inside_strip = strip.is_some_and(|strip| ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| strip.expand(4.0).contains(p)));
    let area = egui::Id::new("vectorcraft-panel-docking");
    let begun = ui.ctx().data(|d| d.get_temp::<LegacyDrag>(area.with("legacy-origin")).is_some());
    let begin = if strip.is_some() { !begun && (response.drag_started() || (!inside_strip && response.dragged())) } else { response.drag_started() };
    if begin && let Some(id) = normalize(panel) {
        let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
        let bounds = ui.max_rect();
        let source = egui::Rect::from_min_size(
            egui::pos2(bounds.left(), response.rect.top()),
            egui::vec2(bounds.width().clamp(240.0, 600.0), bounds.height().clamp(200.0, 600.0)),
        );
        if ensure(&mut layout, id).is_ok()
            && craft_ui::docking::begin_drag(ui.ctx(), egui::Id::new("vectorcraft-panel-docking"), id.to_string(), source)
        {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("vectorcraft-panel-docking").with("legacy-origin"),
                    LegacyDrag {
                        panel: id.to_string(),
                        layout: app.ui.docking.clone(),
                        open_panel: app.ui.open_panel.clone(),
                        floating: app.ui.floating_panels.clone(),
                        icons: app.ui.docking_icons.clone(),
                        collapsed: app.ui.docking_collapsed.clone(),
                    },
                )
            });
            if !app.ui.docking.as_ref().unwrap_or(&legacy(app)).contains(&id.to_string())
                && crate::state::ICON_PANELS.iter().any(|entry| entry.0 == id)
            {
                app.ui.docking_icons.insert(id.into());
            }
            app.ui.docking = Some(layout);
            app.ui.open_panel = None;
        }
    }
}

// Keep the legacy inspection schema useful; shared layout remains the rendering authority.
pub(crate) fn decorated(group: &crate::state::FloatingPanels) -> bool {
    group.column.is_some() || group.docked.is_some() || group.collapsed
}

/// Keep the native set metadata alongside the validated shared ownership tree. Return locations
/// and root splits survive the mirror update; failed reconciliation restores the whole UI state.
pub(crate) fn reconcile_native(app: &mut VectorcraftApp, before: crate::state::UiState) -> Result<(), String> {
    if before.docking.is_none() {
        return Ok(());
    }
    let desired = app.ui.floating_panels.clone();
    let result = (|| {
        let prior = before.docking.as_ref().ok_or("missing panel layout")?;
        app.ui.docking = Some(prior.clone());
        for group in &prior.floating {
            for panel in &group.panels {
                if !desired.iter().any(|g| g.panels.contains(panel)) {
                    change(app, "dock", &json!({"panel":panel}))?;
                }
            }
        }
        let mut layout = app.ui.docking.clone().ok_or("missing panel layout")?;
        for group in &desired {
            for panel in &group.panels {
                if (Layout { root: layout.root.clone(), floating: vec![] }).contains(panel) {
                    if let Ok(location) = layout.location(panel) {
                        app.ui.docking_hidden.insert(panel.clone(), location);
                    }
                    layout.apply(Action::Close { panel: panel.clone() }).map_err(|e| e.to_string())?;
                }
            }
        }
        layout.floating = desired
            .iter()
            .map(|group| {
                let height = prior.floating.iter().find(|g| g.panels.iter().any(|id| group.panels.contains(id))).map_or(400.0, |g| g.rect[3]);
                Floating {
                    panels: group.panels.clone(),
                    active: group.active,
                    rect: [group.pos[0], group.pos[1], group.width.unwrap_or(300.0), height],
                }
            })
            .collect();
        if !valid(&layout) {
            return Err("invalid native panel set".into());
        }
        app.ui.docking = Some(layout);
        app.ui.floating_panels = desired;
        Ok(())
    })();
    if result.is_err() {
        app.ui = before;
    }
    result
}

fn native_change(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    let before = app.ui.clone();
    let result = (|| {
        let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
        if crate::floating::group_of(&app.ui, panel).is_none() {
            let mut initial = params.clone();
            if let Some(object) = initial.as_object_mut() {
                for key in ["above", "below", "column", "collapsed", "onto"] {
                    object.remove(key);
                }
            }
            change(app, "float", &initial)?;
        }
        let baseline = app.ui.clone();
        let value = crate::floating::command(app, id, params).ok_or("unsupported native docking operation")??;
        reconcile_native(app, baseline)?;
        Ok(value)
    })();
    if result.is_err() {
        app.ui = before;
    }
    result
}

fn sync_legacy(app: &mut VectorcraftApp) {
    if let Some(layout) = &app.ui.docking {
        let old = app.ui.floating_panels.clone();
        let mut pending: Vec<_> = layout.root.iter().collect();
        while let Some(node) = pending.pop() {
            match node {
                Node::Tabs { panels, active } => {
                    if let Some(tab) = panels.get(*active).and_then(|id| crate::state::DockTab::from_id(id)) {
                        app.ui.dock_tab = tab;
                    }
                }
                Node::Split { first, second, .. } => pending.extend([second.as_ref(), first.as_ref()]),
                _ => {}
            }
        }
        app.ui.floating_panels = layout
            .floating
            .iter()
            .map(|group| crate::state::FloatingPanels {
                panels: group.panels.clone(),
                active: group.active,
                pos: [group.rect[0], group.rect[1]],
                width: Some(group.rect[2]),
                column: old.iter().find(|native| group.panels.first().is_some_and(|first| native.panels.contains(first))).and_then(|g| g.column),
                collapsed: old
                    .iter()
                    .find(|native| group.panels.first().is_some_and(|first| native.panels.contains(first)))
                    .is_some_and(|g| g.collapsed),
                docked: old.iter().find(|native| group.panels.first().is_some_and(|first| native.panels.contains(first))).and_then(|g| g.docked),
            })
            .collect();
    }
}

#[cfg(test)]
#[path = "panel_docking_tests.rs"]
mod tests;

/// Commit a pointer drop after shared and native panel windows have finished rendering.
pub(crate) fn finish_native_drop(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if let Some((id, params)) =
        ctx.data_mut(|data| data.remove_temp::<(&'static str, serde_json::Value)>(egui::Id::new("vectorcraft-native-panel-drop")))
        && let Some(result) = command(app, id, &params)
        && let Err(error) = result
    {
        app.ui.status = error;
    }
}
