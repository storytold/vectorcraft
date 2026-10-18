//! Application-owned panel IDs and commands around the shared docking layout.

use craft_ui::docking::{Action, Floating, Layout, Node, Placement, Zone};
use serde_json::{Value, json};

use crate::VectorcraftApp;

#[derive(Clone, Default)]
struct LegacyDrag {
    layout: Option<Layout<String>>,
    open_panel: Option<String>,
    floating: Vec<crate::state::FloatingPanels>,
    icons: std::collections::BTreeSet<String>,
}

#[derive(Clone)]
struct RootGroupDrag {
    panel: String,
    layout: Layout<String>,
    hidden: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
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
    if matches!(id, "window.panel.float" | "window.panel.dock")
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

fn change(app: &mut VectorcraftApp, operation: &str, params: &Value) -> Result<Value, String> {
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
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
    let members = if whole_group { group_members(&layout, panel) } else { vec![panel.to_string()] };
    let mut hidden: std::collections::BTreeMap<_, _> =
        app.ui.docking_hidden.iter().filter(|(id, _)| normalize(id).is_some()).take(64).map(|(id, at)| (id.clone(), at.clone())).collect();
    let mut icons = icon_origins(app);
    let icon_panel = crate::state::ICON_PANELS.iter().any(|entry| entry.0 == panel);
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
}

impl craft_ui::docking::DockContent<String> for PanelContent<'_> {
    fn body(&mut self, ui: &mut egui::Ui, panel: &String) {
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
            egui::ScrollArea::vertical()
                .id_salt(("shared-panel-body", panel))
                .auto_shrink([false, false])
                .show(ui, |ui| crate::dock::panel_body(self.app, ui, panel, width, height));
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
            let response = ui.interact(grip, ui.id().with("dock-group-bar"), egui::Sense::drag());
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

pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) -> bool {
    let root_key = egui::Id::new("vectorcraft-root-group-drag");
    if let Some(drag) = ui.ctx().data(|data| data.get_temp::<RootGroupDrag>(root_key)) {
        if drag.generation != app.ui.docking_generation {
            ui.ctx().data_mut(|data| data.remove::<RootGroupDrag>(root_key));
        } else if ui.input(|input| !input.focused || input.key_pressed(egui::Key::Escape)) {
            app.ui.docking = Some(drag.layout);
            app.ui.docking_hidden = drag.hidden;
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
                    let (id, params) = match target {
                        crate::floating::Drop::Dock => ("window.panel.dock", json!({"panel":panel,"group":true})),
                        crate::floating::Drop::Stack(onto) => ("window.panel.float", json!({"panel":panel,"group":true,"onto":onto})),
                        crate::floating::Drop::Below(onto) => ("window.panel.float", json!({"panel":panel,"group":true,"below":onto})),
                        crate::floating::Drop::Above(onto) => ("window.panel.float", json!({"panel":panel,"group":true,"above":onto})),
                        crate::floating::Drop::Column(slot) => ("window.panel.dock", json!({"panel":panel,"group":true,"column":slot})),
                    };
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
        let mut content = PanelContent { app, commands: Vec::new() };
        let output = craft_ui::docking::DockArea::new(area).show_customized(
            ui,
            &display,
            &style,
            |id| {
                crate::state::all_panels()
                    .find(|p| p.0 == id)
                    .map(|p| tl!(p.1).to_string())
                    .or_else(|| library_label(id))
                    .unwrap_or_else(|| id.clone())
            },
            |_| craft_ui::docking::Permissions::default(),
            |_| craft_ui::docking::PanelLimits { min: egui::vec2(220.0, 80.0), ..Default::default() },
            &mut content,
        );
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
                .filter(|(target, _)| !matches!(target, crate::floating::Drop::Dock))
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
        ui.ctx().data_mut(|data| data.remove::<LegacyDrag>(egui::Id::new("vectorcraft-panel-docking").with("legacy-origin")));
    }
    for action in output.actions {
        if tab_drop
            .is_some_and(|(panel, _)| matches!(&action, Action::Float { panel: moved, .. } | Action::Move { panel: moved, .. } if moved == panel))
        {
            continue;
        }
        if let Err(error) = app.run("window.panel.layout", json!({"action": action})) {
            app.ui.status = error;
        }
    }
    let group_drop = output.group_drop.and_then(|(panel, pos)| {
        let panel = normalize(&panel)?;
        crate::floating::drop_target(app, ui.ctx(), crate::floating::Moving::Panel(panel), pos).map(|(target, _)| (panel, target, true))
    });
    if let Some((panel, target, whole_group)) = group_drop.or_else(|| tab_drop.map(|(panel, target)| (panel, target, false))) {
        let (command, params) = match target {
            crate::floating::Drop::Dock => ("window.panel.dock", json!({"panel": panel, "group": whole_group})),
            crate::floating::Drop::Stack(onto) => ("window.panel.float", json!({"panel": panel, "group": whole_group, "onto": onto})),
            crate::floating::Drop::Below(onto) => ("window.panel.float", json!({"panel":panel,"group":whole_group,"below":onto})),
            crate::floating::Drop::Above(onto) => ("window.panel.float", json!({"panel":panel,"group":whole_group,"above":onto})),
            crate::floating::Drop::Column(slot) => ("window.panel.dock", json!({"panel":panel,"group":whole_group,"column":slot})),
        };
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

fn icon_rail(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let column = egui::Panel::right("shared-panel-icons")
        .exact_size(38.0)
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            crate::theme::card(ui, &t, egui::Margin { left: 3, right: 3, top: 6, bottom: 6 }, |ui| {
                egui::ScrollArea::vertical().id_salt("shared-panel-icons-scroll").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for &(id, label, icon) in crate::state::ICON_PANEL_GROUPS
                        .iter()
                        .flat_map(|group| group.iter())
                        .filter_map(|id| crate::state::ICON_PANELS.iter().find(|entry| entry.0 == *id))
                    {
                        if app.ui.docking.as_ref().is_some_and(|layout| layout.contains(&id.to_string())) {
                            continue;
                        }
                        let (_, rect) = ui.allocate_space(egui::vec2(30.0, 30.0));
                        let response = ui.interact(rect, ui.id().with(("panel-icon", id)), egui::Sense::click_and_drag());
                        let response = crate::widgets::paint_icon_button(ui, response, icon, tl!(label), app.ui.open_panel.as_deref() == Some(id));
                        if response.clicked() {
                            app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(id) { None } else { Some(id.into()) };
                        }
                        legacy_tab(app, ui, id, &response);
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

pub(crate) fn legacy_tab(app: &mut VectorcraftApp, ui: &mut egui::Ui, panel: &str, response: &egui::Response) {
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
    });
    if response.drag_started()
        && let Some(id) = normalize(panel)
    {
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
                        layout: app.ui.docking.clone(),
                        open_panel: app.ui.open_panel.clone(),
                        floating: app.ui.floating_panels.clone(),
                        icons: app.ui.docking_icons.clone(),
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
    if let Some((command, params)) =
        ctx.data_mut(|data| data.remove_temp::<(&'static str, serde_json::Value)>(egui::Id::new("vectorcraft-native-panel-drop")))
        && let Err(error) = app.run(command, params)
    {
        app.ui.status = error;
    }
}
