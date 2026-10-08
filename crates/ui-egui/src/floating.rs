//! Panels taken out of the dock. Dragging a panel's heading (a dock tab, a popped-out icon panel)
//! tears it off: it floats wherever it is dropped, locks to the toolbar when dropped beside it (a
//! column next to the toolbar, the canvas making room), and goes back in the dock when dropped on
//! the dock or closed. The lock button locks and unlocks without dragging. Places are kept with
//! the UI preferences (`UiState::floating_panels`).

use egui::{CornerRadius, Pos2, Rect, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::state::{DockTab, FloatingPanel, ICON_PANELS, PanelPlace};
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, icons, panels};

/// The dock's tabbed panels, by id.
pub const DOCK_TABS: [(&str, &str, DockTab); 3] =
    [("properties", "Properties", DockTab::Properties), ("layers", "Layers", DockTab::Layers), ("libraries", "Libraries", DockTab::Libraries)];

/// Height of a panel's heading strip.
pub const HEADING: f32 = 26.0;
/// How far right of the toolbar a dropped panel still locks to it.
const SNAP: f32 = 48.0;
/// Gap between panels locked in the toolbar column.
const GAP: f32 = 4.0;

/// Whether `id` is a panel that can leave the dock.
pub fn is_panel(id: &str) -> bool {
    label(id).is_some()
}

/// The panel's English name.
pub fn label(id: &str) -> Option<&'static str> {
    DOCK_TABS.iter().find(|t| t.0 == id).map(|t| t.1).or_else(|| ICON_PANELS.iter().find(|p| p.0 == id).map(|p| p.1))
}

fn dock_tab(id: &str) -> Option<DockTab> {
    DOCK_TABS.iter().find(|t| t.0 == id).map(|t| t.2)
}

/// A panel's width: the dock tabs keep the dock's width, icon panels their pop-out width.
pub fn width(id: &str) -> f32 {
    if dock_tab(id).is_some() { 300.0 } else { 256.0 }
}

/// Where the panel is, if it is out of the dock.
pub fn place(app: &VectorcraftApp, id: &str) -> Option<PanelPlace> {
    app.ui.floating_panels.iter().find(|p| p.id == id).map(|p| p.place)
}

/// The dock tabs still in the dock, in order.
pub fn docked_tabs(app: &VectorcraftApp) -> Vec<(&'static str, &'static str, DockTab)> {
    DOCK_TABS.iter().copied().filter(|t| place(app, t.0).is_none()).collect()
}

/// Parts of the window a dragged panel can be dropped on, recorded as they are laid out.
#[derive(Clone, Copy, Debug)]
pub enum Zone {
    Toolbar,
    Column,
    Dock,
}

#[derive(Clone, Copy, Debug, Default)]
struct Zones {
    toolbar: Option<Rect>,
    column: Option<Rect>,
    dock: Option<Rect>,
}

fn zones_id() -> egui::Id {
    egui::Id::new("panel-zones")
}

fn zones(ctx: &egui::Context) -> Zones {
    ctx.data(|d| d.get_temp::<Zones>(zones_id())).unwrap_or_default()
}

/// Forget last frame's zones (a hidden toolbar or dock is no drop target).
pub fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(zones_id(), Zones::default()));
}

pub fn set_zone(ctx: &egui::Context, zone: Zone, rect: Rect) {
    let mut z = zones(ctx);
    let slot = match zone {
        Zone::Toolbar => &mut z.toolbar,
        Zone::Column => &mut z.column,
        Zone::Dock => &mut z.dock,
    };
    *slot = Some(slot.map_or(rect, |r| r.union(rect)));
    ctx.data_mut(|d| d.insert_temp(zones_id(), z));
}

/// The id of a panel heading's drag handle: the same wherever the panel is drawn, so a drag that
/// starts in the dock carries on while the panel floats.
pub fn handle_id(id: &str) -> egui::Id {
    egui::Id::new(("panel-heading", id))
}

fn drag_id() -> egui::Id {
    egui::Id::new("panel-drag")
}

/// The panel being dragged, where the pointer holds it (relative to its top-left corner) and the
/// pass the drag started in.
fn dragging(ctx: &egui::Context) -> Option<(String, egui::Vec2, u64)> {
    ctx.data(|d| d.get_temp::<(String, egui::Vec2, u64)>(drag_id()))
}

/// Whether `id` started moving in this pass: it was drawn where it was (its heading's widget id
/// can't appear in two places in one pass), and shows up in its new place next pass.
fn just_torn_off(ctx: &egui::Context, id: &str) -> bool {
    dragging(ctx).is_some_and(|d| d.0 == id && d.2 == ctx.cumulative_pass_nr())
}

/// Start dragging panel `id` (from wherever it is), its heading grabbed at `grab` from its
/// top-left corner.
pub fn start_drag(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str, grab: egui::Vec2) {
    let Some(at) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    let pos = at - grab;
    float(app, id, PanelPlace::Free, Some(pos));
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(drag_id(), (id.to_string(), grab, pass)));
    ctx.request_repaint();
}

/// Take `id` out of the dock (or move it) to `place`; a free panel goes to `pos` when given.
fn float(app: &mut VectorcraftApp, id: &str, place: PanelPlace, pos: Option<Pos2>) {
    if app.ui.open_panel.as_deref() == Some(id) {
        app.ui.open_panel = None;
    }
    let i = match app.ui.floating_panels.iter().position(|p| p.id == id) {
        Some(i) => i,
        None => {
            app.ui.floating_panels.push(FloatingPanel { id: id.to_string(), ..Default::default() });
            app.ui.floating_panels.len() - 1
        }
    };
    if let Some(p) = app.ui.floating_panels.get_mut(i) {
        if let Some(pos) = pos {
            p.pos = [pos.x, pos.y];
        }
        p.place = place;
    }
}

/// Put `id` back in the dock (a dock tab becomes the active tab).
fn dock(app: &mut VectorcraftApp, id: &str) {
    app.ui.floating_panels.retain(|p| p.id != id);
    if let Some(tab) = dock_tab(id) {
        app.ui.dock_tab = tab;
    }
}

/// Lock `id` to the toolbar, below the panels already there whose tops are above `y` (all of
/// them without `y`).
fn lock(app: &mut VectorcraftApp, id: &str, y: Option<f32>) {
    float(app, id, PanelPlace::Toolbar, None);
    let Some(i) = app.ui.floating_panels.iter().position(|p| p.id == id) else { return };
    let p = app.ui.floating_panels.remove(i);
    let at = y
        .and_then(|y| app.ui.floating_panels.iter().position(|q| q.place == PanelPlace::Toolbar && q.pos[1] > y))
        .unwrap_or(app.ui.floating_panels.len());
    app.ui.floating_panels.insert(at.min(app.ui.floating_panels.len()), p);
}

/// The drop target under the pointer while panel `id` is dragged.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Dock,
    Toolbar,
    Free,
}

fn target(ctx: &egui::Context, at: Pos2) -> Target {
    let z = zones(ctx);
    if z.dock.is_some_and(|d| at.x >= d.left()) {
        return Target::Dock;
    }
    let edge = z.column.or(z.toolbar).map(|r| r.right());
    match (z.toolbar.or(z.column), edge) {
        (Some(tb), Some(edge)) if at.x <= edge + SNAP && at.x >= tb.left() => Target::Toolbar,
        _ => Target::Free,
    }
}

/// Follow the pointer with the dragged panel, and settle it where the button is released.
fn track_drag(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some((id, grab, _)) = dragging(ctx) else { return };
    let (at, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
    let Some(at) = at else { return };
    if down {
        if let Some(p) = app.ui.floating_panels.iter_mut().find(|p| p.id == id) {
            p.pos = [at.x - grab.x, at.y - grab.y];
            p.place = PanelPlace::Free;
        }
        let (t, z) = (Tokens::get(ctx), zones(ctx));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("panel-drop")));
        match target(ctx, at) {
            Target::Dock => {
                if let Some(d) = z.dock {
                    painter.rect_filled(d, 0.0, t.accent.gamma_multiply(0.18));
                    painter.rect_stroke(d.shrink(1.0), 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
            }
            Target::Toolbar => {
                if let Some(tb) = z.column.or(z.toolbar) {
                    let bar = Rect::from_min_max(egui::pos2(tb.right(), tb.top()), egui::pos2(tb.right() + 4.0, tb.bottom()));
                    painter.rect_filled(bar, 0.0, t.accent);
                }
            }
            Target::Free => {}
        }
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        return;
    }
    ctx.data_mut(|d| d.remove::<(String, egui::Vec2, u64)>(drag_id()));
    match target(ctx, at) {
        Target::Dock => dock(app, &id),
        Target::Toolbar => lock(app, &id, Some(at.y)),
        Target::Free => {}
    }
}

/// What a panel heading's buttons asked for.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    None,
    Close,
    ToggleLock,
}

/// A panel's heading: its name on a tab (the drag handle), the panel menu, lock and close.
fn heading(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, w: f32, locked: bool) -> Action {
    let t = Tokens::get(ui.ctx());
    let name = label(id).unwrap_or(id);
    let (strip, _) = ui.allocate_exact_size(vec2(w, HEADING), Sense::hover());
    let round = if locked { CornerRadius::ZERO } else { CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 } };
    ui.painter().rect_filled(strip, round, t.panel_darker);
    let galley = ui.painter().layout_no_wrap(tl!(name).to_string(), theme::semibold(12.0), t.text);
    let tab = Rect::from_min_size(strip.min, vec2(galley.size().x + 24.0, HEADING));
    ui.painter().rect_filled(tab, if locked { CornerRadius::ZERO } else { CornerRadius { nw: 4, ne: 0, sw: 0, se: 0 } }, t.panel);
    ui.painter().galley(tab.left_center() + vec2(12.0, -galley.size().y / 2.0), galley, t.text);
    // The whole strip drags (the buttons, drawn after, take their own clicks).
    let grip = ui.interact(strip, handle_id(id), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Grab);
    if grip.drag_started() {
        let grab = ui.input(|i| i.pointer.press_origin()).map_or(vec2(20.0, 12.0), |p| p - strip.min);
        start_drag(app, ui.ctx(), id, grab);
    }
    let mut action = Action::None;
    let button = |ui: &mut Ui, x: f32, icon: &str, tip: &str| {
        let r = Rect::from_center_size(egui::pos2(strip.right() - x, strip.center().y), vec2(14.0, 14.0));
        let resp = ui.interact(r, ui.id().with(("panel-button", id, icon)), Sense::click());
        icons::paint(ui, icon, r, if resp.hovered() { t.text_strong } else { t.text_dim });
        resp.on_hover_text(tip).clicked()
    };
    if button(ui, 13.0, "x", tl!("Close (back to the dock)")) {
        action = Action::Close;
    }
    let (icon, tip) = if locked { ("lock", tl!("Unlock from the toolbar")) } else { ("lock-open", tl!("Lock to the toolbar")) };
    if button(ui, 33.0, icon, tip) {
        action = Action::ToggleLock;
    }
    panels::panel_menu(app, ui, id, Rect::from_center_size(egui::pos2(strip.right() - 54.0, strip.center().y), vec2(16.0, 16.0)));
    action
}

/// A panel's contents, `max_h` points tall at most (scrolling inside when taller).
fn body(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, w: f32, max_h: f32) {
    egui::Frame::NONE.inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        let inner = w - 20.0;
        ui.set_width(inner);
        match id {
            // The Layers panel fills the height it is given (its list scrolls).
            "layers" => {
                let h = max_h.min(420.0);
                ui.allocate_ui_with_layout(vec2(inner, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_height(h);
                    ui.set_max_height(h);
                    panels::layers::show(app, ui);
                });
            }
            "libraries" => panels::libraries(app, ui),
            _ => {
                egui::ScrollArea::vertical().id_salt(("panel-body", id)).max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    if id == "properties" {
                        panels::properties::show(app, ui);
                    } else {
                        panels::show_icon_panel(app, ui, id);
                    }
                });
            }
        }
    });
}

fn act(app: &mut VectorcraftApp, id: &str, action: Action, unlocked_at: Pos2) {
    match action {
        Action::None => {}
        Action::Close => dock(app, id),
        Action::ToggleLock if place(app, id) == Some(PanelPlace::Toolbar) => float(app, id, PanelPlace::Free, Some(unlocked_at)),
        Action::ToggleLock => lock(app, id, None),
    }
}

fn shown(app: &VectorcraftApp) -> bool {
    app.ui.dock && app.ui.screen_mode < 3
}

/// The column of panels locked to the toolbar (call right after the toolbar, before the canvas).
pub fn toolbar_column(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ids: Vec<String> = app.ui.floating_panels.iter().filter(|p| p.place == PanelPlace::Toolbar).map(|p| p.id.clone()).collect();
    if ids.is_empty() || !shown(app) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let w = ids.iter().map(|id| width(id)).fold(0.0, f32::max);
    let screen_h = ui.ctx().content_rect().height();
    let mut actions = vec![];
    let resp = egui::Panel::left("panel_column")
        .resizable(false)
        .exact_size(w)
        .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.5, t.border)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().id_salt("panel-column").auto_shrink([false, false]).show(ui, |ui| {
                for (k, id) in ids.iter().enumerate() {
                    if k > 0 {
                        let (r, _) = ui.allocate_exact_size(vec2(w, GAP), Sense::hover());
                        ui.painter().rect_filled(r, 0.0, t.border);
                    }
                    let top = ui.cursor().min;
                    let action = heading(app, ui, id, w, true);
                    actions.push((id.clone(), action));
                    body(app, ui, id, w, (screen_h * 0.6).max(160.0));
                    if let Some(p) = app.ui.floating_panels.iter_mut().find(|p| &p.id == id && p.place == PanelPlace::Toolbar) {
                        p.pos = [top.x, top.y];
                    }
                }
            });
        });
    let rect = resp.response.rect;
    set_zone(ui.ctx(), Zone::Column, rect);
    for (id, action) in actions {
        let at = egui::pos2(rect.right() + 24.0, place_y(app, &id).unwrap_or(rect.top() + 40.0));
        act(app, &id, action, at);
    }
}

fn place_y(app: &VectorcraftApp, id: &str) -> Option<f32> {
    app.ui.floating_panels.iter().find(|p| p.id == id).map(|p| p.pos[1])
}

/// The free-floating panels, over the canvas (call after the canvas, before dialogs).
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    track_drag(app, ctx);
    // A panel opened from the dock that is already floating: bring it to the front instead.
    if let Some(open) = app.ui.open_panel.clone()
        && place(app, &open).is_some()
    {
        app.ui.open_panel = None;
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, area_id(&open)));
    }
    if !shown(app) {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let free: Vec<FloatingPanel> =
        app.ui.floating_panels.iter().filter(|p| p.place == PanelPlace::Free && !just_torn_off(ctx, &p.id)).cloned().collect();
    for p in free {
        let w = width(&p.id);
        // Keep at least the heading reachable when the window shrinks.
        let x = p.pos[0].clamp(screen.left() - w + 80.0, (screen.right() - 80.0).max(screen.left()));
        let y = p.pos[1].clamp(screen.top(), (screen.bottom() - HEADING).max(screen.top()));
        let max_h = (screen.bottom() - y - HEADING - 40.0).clamp(160.0, 640.0);
        let mut action = Action::None;
        egui::Area::new(area_id(&p.id)).order(egui::Order::Middle).current_pos(egui::pos2(x, y)).movable(false).constrain(false).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(t.panel).corner_radius(CornerRadius::same(4)).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(w);
                action = heading(app, ui, &p.id, w, false);
                ui.spacing_mut().item_spacing = ctx.global_style().spacing.item_spacing;
                body(app, ui, &p.id, w, max_h);
            });
        });
        if let Some(q) = app.ui.floating_panels.iter_mut().find(|q| q.id == p.id && q.place == PanelPlace::Free)
            && dragging(ctx).is_none_or(|d| d.0 != p.id)
        {
            q.pos = [x, y];
        }
        act(app, &p.id, action, egui::pos2(x, y));
    }
}

/// The egui area of a free panel.
pub fn area_id(id: &str) -> egui::Id {
    egui::Id::new(("floating-panel", id))
}

/// `window.panelPlace {panel, place: free|toolbar|dock, x?, y?}`.
pub fn run_place(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("panel").and_then(Value::as_str).unwrap_or("");
    if !is_panel(id) {
        return Err(format!("unknown panel `{id}`"));
    }
    let coord = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.clamp(-1e5, 1e5) as f32);
    match p.get("place").and_then(Value::as_str).unwrap_or("free") {
        "free" => {
            let cur = app.ui.floating_panels.iter().find(|q| q.id == id).map_or([200.0, 120.0], |q| q.pos);
            let at = egui::pos2(coord("x").unwrap_or(cur[0]), coord("y").unwrap_or(cur[1]));
            float(app, id, PanelPlace::Free, Some(at));
        }
        "toolbar" => lock(app, id, coord("y")),
        "dock" => dock(app, id),
        other => return Err(format!("unknown place `{other}` (free, toolbar or dock)")),
    }
    app.ui.dock = true;
    Ok(
        json!({ "panel": id, "place": match place(app, id) { None => "dock", Some(PanelPlace::Free) => "free", Some(PanelPlace::Toolbar) => "toolbar" } }),
    )
}
/// Where the toolbar was laid out last frame (none while hidden).
pub fn toolbar_zone(ctx: &egui::Context) -> Option<Rect> {
    zones(ctx).toolbar
}
