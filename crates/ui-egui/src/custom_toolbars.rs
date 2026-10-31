//! Window › Toolbars › New Toolbar (#1759): toolbars the user makes and names. New Toolbar asks
//! for a name and makes a blank toolbar that floats on its own; the ••• button at its foot opens the
//! All Tools drawer beside it, and tools dragged from the drawer fill it (`edit_toolbar`, with the
//! `toolbar` param of its commands). Its title bar moves it, its × hides it and its double arrow
//! shows its tools in one column or two. Each toolbar is listed under Window › Toolbars, where its
//! item shows or hides it, and is kept with the workspace it was made in (`Workspace::toolbars`).
//!
//! Behavior from the public documentation:
//! <https://helpx.adobe.com/illustrator/using/tools.html> (Create and manage toolbars: New Toolbar
//! from Window › Toolbars or the drawer's flyout menu, a name and OK make a blank toolbar, its Edit
//! Toolbar button opens the drawer to add tools, and a toolbar stays in the workspace it is made in;
//! a toolbar moves by its title bar, whose double arrow toggles one column or two and whose close
//! button hides it).

use std::sync::atomic::Ordering;

use egui::{CornerRadius, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_tools::tool_info;

use crate::state::{CustomToolbar, Dialog, UiState};
use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, widgets};

/// The most toolbars a workspace keeps.
pub(crate) const MAX: usize = 64;
/// Width of a toolbar of one column of tool buttons, inside its border.
const WIDTH: f32 = 44.0;
/// Width a second column of tool buttons adds.
const BUTTON: f32 = 36.0;
/// Height of the title bar at a toolbar's top, which moves it.
const GRIP: f32 = 14.0;
/// Width of the × and the double arrow at the ends of the title bar.
const TITLE_BUTTON: f32 = 12.0;
/// Where the first toolbar made floats, right of the Tools panel; the next ones go lower.
const NEW_AT: [f32; 2] = [96.0, 120.0];

/// A toolbar's name without leading and trailing spaces, in lowercase. Names with the same key are
/// the same name: lookups, the check that a name is free, sanitizing and each toolbar's area
/// compare keys.
pub(crate) fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The area a toolbar is drawn in.
pub(crate) fn area_id(name: &str) -> egui::Id {
    egui::Id::unique(("custom-toolbar", key(name)))
}

/// The toolbar named `name`, whatever its case.
pub(crate) fn find<'a>(ui: &'a UiState, name: &str) -> Option<&'a CustomToolbar> {
    let k = key(name);
    ui.custom_toolbars.iter().find(|b| key(&b.name) == k)
}

pub(crate) fn find_mut<'a>(ui: &'a mut UiState, name: &str) -> Option<&'a mut CustomToolbar> {
    let k = key(name);
    ui.custom_toolbars.iter_mut().find(|b| key(&b.name) == k)
}

/// The `toolbar` param: the name of a toolbar made with New Toolbar, as it was given (none or
/// null: the Tools panel's toolbar).
pub(crate) fn target(ui: &UiState, p: &Value) -> Result<Option<String>, String> {
    match p.get("toolbar") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let name = v.as_str().ok_or("toolbar (a toolbar's name) must be a string")?;
            find(ui, name).map(|b| Some(b.name.clone())).ok_or_else(|| format!("no toolbar named `{name}`"))
        }
    }
}

/// The toolbar the open All Tools drawer edits: one made with New Toolbar, or none for the Tools
/// panel's toolbar (and while the drawer is closed).
pub(crate) fn drawer_target(ui: &UiState) -> Option<&str> {
    ui.drawer_toolbar.as_deref().filter(|_| ui.tools_drawer)
}

/// Make a blank toolbar named `name`, shown; returns its name.
pub(crate) fn create(ui: &mut UiState, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("toolbar name can't be empty".into());
    }
    if find(ui, name).is_some() {
        return Err(format!("there is already a toolbar named `{name}`"));
    }
    if ui.custom_toolbars.len() >= MAX {
        return Err(format!("a workspace keeps at most {MAX} toolbars"));
    }
    let n = ui.custom_toolbars.len() as f32;
    let pos = [NEW_AT[0] + 16.0 * n, NEW_AT[1] + 24.0 * n];
    ui.custom_toolbars.push(CustomToolbar {
        name: name.to_string(),
        slots: vec![],
        shown: true,
        pos,
        slot_tool: Default::default(),
        double: false,
        floating: vec![],
    });
    Ok(name.to_string())
}

/// Keep named toolbars, each name once whatever its case, of known tools, each once in its
/// toolbar, at a position that is a number, with one strip per group torn off them (a hand-edited
/// preferences file or workspace).
pub(crate) fn sanitize(bars: &mut Vec<CustomToolbar>) {
    let mut names = std::collections::BTreeSet::new();
    bars.retain_mut(|b| {
        b.name = b.name.trim().to_string();
        let mut seen = std::collections::BTreeSet::new();
        for s in &mut b.slots {
            s.retain(|id| tool_info(id).is_some() && seen.insert(id.clone()));
        }
        b.slots.retain(|s| !s.is_empty());
        crate::state::FloatingFlyout::sanitize(&mut b.floating);
        if !b.pos.iter().all(|v| v.is_finite()) {
            b.pos = NEW_AT;
        }
        !b.name.is_empty() && names.insert(key(&b.name))
    });
    bars.truncate(MAX);
}

/// The commands of the toolbars made with New Toolbar; `None` for other ids.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.toolbar.new" => match p.get("name").filter(|v| !v.is_null()) {
            Some(name) => match name.as_str() {
                Some(name) => create(&mut app.ui, name).map(|name| json!({ "toolbar": name })),
                None => Err("name (a toolbar's name) must be a string".into()),
            },
            None => {
                let name = (1..).map(|i| format!("Toolbar {i}")).find(|n| find(&app.ui, n).is_none()).unwrap_or_default();
                app.ui.dialog = Some(Dialog::new(KIND, json!({ "name": name })));
                Ok(Value::Null)
            }
        },
        "window.toolbar.custom" => (|| {
            let show = crate::menus::opt_bool(p, "show")?;
            let name = p.get("name").and_then(Value::as_str).ok_or("name (a toolbar's name) is required")?;
            let bar = find_mut(&mut app.ui, name).ok_or_else(|| format!("no toolbar named `{name}`"))?;
            bar.shown = show.unwrap_or(!bar.shown);
            let (shown, name) = (bar.shown, bar.name.clone());
            // The drawer beside a toolbar that is hidden closes with it.
            if !shown && drawer_target(&app.ui) == Some(name.as_str()) {
                app.ui.tools_drawer = false;
                app.ui.drawer_toolbar = None;
            }
            Ok(json!(shown))
        })(),
        "window.toolbar.list" => Ok(json!(
            app.ui
                .custom_toolbars
                .iter()
                .map(|b| json!({"name": b.name, "shown": b.shown, "double": b.double, "slots": b.slots}))
                .collect::<Vec<_>>()
        )),
        _ => return None,
    })
}

/// New Toolbar's dialog.
pub(crate) const KIND: &str = "newToolbar";

/// OK in New Toolbar.
pub(crate) fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    let name = create(&mut app.ui, &d.str("name"))?;
    app.ui.dialog = None;
    Ok(json!({ "toolbar": name }))
}

/// The New Toolbar dialog: the name, OK and Cancel.
pub(crate) fn show_dialog(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let (mut ok, mut cancel) = (false, false);
    crate::dialogs::modal::show(ctx, "New Toolbar", egui::Id::unique("dialog-new-toolbar"), -40.0, 22, |ui| {
        ui.set_width(320.0);
        crate::dialogs::modal::heading(ui, tl!("New Toolbar"));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim));
            let mut name = d.str("name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0)).changed() {
                d.fields.insert("name".into(), json!(name));
            }
        });
        ui.add_space(16.0);
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
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        ok = true;
    }
    app.ui.dialog = Some(d);
    if cancel {
        app.ui.dialog = None;
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

// ---------- the menu ----------

crate::shortcut_editor::ui_mirror!(fn names_store() -> Vec<&'static str>);

/// Mirror the toolbars' names for the (static) menu tree.
pub fn sync(ui: &UiState) {
    let same = names_store()
        .read()
        .map(|v| v.len() == ui.custom_toolbars.len() && v.iter().zip(&ui.custom_toolbars).all(|(a, b)| *a == b.name))
        .unwrap_or(true);
    if !same {
        if let Ok(mut v) = names_store().write() {
            *v = ui.custom_toolbars.iter().map(|b| crate::shortcut_editor::intern(&b.name)).collect();
        }
        crate::shortcut_editor::GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Window › Toolbars: Advanced and Double Column, the toolbars made with New Toolbar (each checked
/// while it shows), and New Toolbar….
pub fn menu_items() -> Vec<crate::menus::Item> {
    use crate::menus::Item;
    let c = |label: &'static str, id: &'static str| Item::Cmd(label, id, Value::Null);
    let mut v = vec![c("Advanced", "window.toolbarAdvanced"), c("Double Column", "window.toolbarColumns"), Item::Sep];
    let names = names_store().read().map(|v| v.clone()).unwrap_or_default();
    if !names.is_empty() {
        v.extend(names.into_iter().map(|n| Item::Cmd(n, "window.toolbar.custom", json!({ "name": n }))));
        v.push(Item::Sep);
    }
    v.push(c("New Toolbar…", "window.toolbar.new"));
    v
}

// ---------- the toolbars ----------

/// What a press on a toolbar did, applied once every toolbar is drawn.
enum Action {
    Select(&'static str),
    Options(&'static str),
    /// A group's flyout: its tools, its button, and the toolbar's name.
    Flyout(Vec<&'static str>, egui::Rect, String),
    EditToolbar(String),
    /// The title bar's ×.
    Hide(String),
    /// The title bar's double arrow: two columns (true) or one.
    Columns(String, bool),
}

/// The toolbars made with New Toolbar that show, each floating where its title bar was dragged: the
/// title bar's × hides it and its double arrow lays the tools out in one column or two; then its
/// tool buttons (a group's flyout opens as the Tools panel's do) and the ••• button that opens the
/// All Tools drawer beside it.
pub(crate) fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.ui.custom_toolbars.iter().all(|b| !b.shown) {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let active = app.session.tool_id();
    let editing = drawer_target(&app.ui).map(str::to_string);
    let held_id = egui::Id::unique("custom-toolbar-held");
    if ctx.input(|i| i.pointer.any_pressed()) {
        ctx.data_mut(|d| d.remove::<egui::Id>(held_id));
    }
    let held: Option<egui::Id> = ctx.data(|d| d.get_temp(held_id));
    let mut actions = vec![];
    // Where the drawer opens (the toolbar's bounds and its ••• button): beside the toolbar it
    // edits, or beside the one whose ••• button was just clicked.
    let (mut anchor, mut clicked_anchor) = (None, None);
    for k in 0..app.ui.custom_toolbars.len() {
        let Some(bar) = app.ui.custom_toolbars.get(k).filter(|b| b.shown).cloned() else { continue };
        let id = area_id(&bar.name);
        let cols = if bar.double { 2 } else { 1 };
        let width = WIDTH + BUTTON * (cols - 1) as f32;
        // Kept on screen by its title bar and first tools: the rest scrolls.
        let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(width, 60.0), |r| vec2(r.width(), r.height().min(160.0)));
        let pos = crate::tools_float::clamp(bar.pos, size, screen);
        let edit_this = editing.as_deref() == Some(bar.name.as_str());
        let mut moved = None;
        let mut drops = None;
        egui::Area::new(id).order(egui::Order::Middle).fixed_pos(pos).show(ctx, |ui| {
            crate::toolbar::flyout_frame(&t, false).inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
                ui.set_width(width);
                // The tools and the ••• button scroll when they don't fit in the window under the
                // title bar.
                ui.set_max_height(screen.bottom() - pos.y - 12.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let middle = ui.max_rect().center().x;
                // The title bar: × at its left end, the double arrow at its right end, and the grip
                // between them, whose drag moves the toolbar.
                let (title, _) = ui.allocate_exact_size(vec2(ui.available_width(), GRIP), Sense::hover());
                ui.painter().rect_filled(title, 0.0, t.tab_strip);
                let close = egui::Rect::from_min_size(title.min, vec2(TITLE_BUTTON, GRIP));
                let arrows = egui::Rect::from_min_size(pos2(title.right() - TITLE_BUTTON, title.top()), vec2(TITLE_BUTTON, GRIP));
                let grip = egui::Rect::from_min_max(pos2(close.right(), title.top()), pos2(arrows.left(), title.bottom()));
                let cresp = ui.interact(close, id.with("close"), Sense::click());
                icons::paint(ui, "x", close.shrink2(vec2(1.5, 2.5)), if cresp.hovered() { t.text_strong } else { t.text });
                cresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Close")));
                if cresp.on_hover_text(tl!("Close")).clicked() {
                    actions.push(Action::Hide(bar.name.clone()));
                }
                let aresp = ui.interact(arrows, id.with("columns"), Sense::click());
                let chevrons = if bar.double { "chevrons-left" } else { "chevrons-right" };
                icons::paint(ui, chevrons, arrows.shrink2(vec2(1.0, 2.0)), if aresp.hovered() { t.text_strong } else { t.text });
                aresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Toggle single/double column")));
                if aresp.on_hover_text(tl!("Toggle single/double column")).clicked() {
                    actions.push(Action::Columns(bar.name.clone(), !bar.double));
                }
                let gresp = ui.interact(grip, id.with("grip"), Sense::drag());
                for n in 0..3 {
                    let y = grip.center().y - 2.0 + n as f32 * 2.0;
                    let half = (grip.width() / 2.0 - 2.0).clamp(0.0, 8.0);
                    ui.painter().line_segment([pos2(grip.center().x - half, y), pos2(grip.center().x + half, y)], Stroke::new(0.5, t.text_disabled));
                }
                gresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &bar.name));
                // Held where the press took it, from the toolbar's corner, while it is dragged.
                let grab = id.with("grab");
                if gresp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    let (origin, at) = ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos()));
                    if let Some(at) = at {
                        let off = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(grab, || origin.unwrap_or(at) - pos));
                        moved = Some(at - off);
                    }
                } else {
                    ui.data_mut(|d| d.remove::<egui::Vec2>(grab));
                    gresp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(&bar.name);
                }
                // A gap keeps a press near the grip bar's lower edge on the bar.
                ui.add_space(4.0);
                let mut buttons: Vec<(usize, egui::Rect)> = vec![];
                let edit = widgets::strip_scroll(ui, &format!("custom-toolbar-{}", key(&bar.name)), |ui| {
                    let slots: Vec<(usize, &Vec<String>)> = bar.slots.iter().enumerate().collect();
                    let press = Press { bar: &bar, active, edit_this, held, held_id };
                    for row in slots.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.add_space((middle - BUTTON / 2.0 * row.len() as f32 - ui.cursor().min.x).max(0.0));
                            for &(s, slot) in row {
                                if let Some(rect) = slot_button(ui, &t, &press, slot, &mut actions)
                                    && edit_this
                                {
                                    buttons.push((s, rect));
                                }
                            }
                        });
                    }
                    ui.add_space(4.0);
                    // Edit Toolbar: the All Tools drawer opens beside this toolbar, by this button.
                    ui.vertical_centered(|ui| widgets::icon_button(ui, "ellipsis", "Edit Toolbar", edit_this, 26.0)).inner
                });
                if edit.clicked() {
                    actions.push(Action::EditToolbar(bar.name.clone()));
                }
                // The toolbar as drawn (inside its border): where tools drop, and where a click
                // leaves the drawer open; also on the click that opens the drawer here.
                let bounds = ui.min_rect();
                if edit.clicked() {
                    clicked_anchor = Some((bounds, edit.rect));
                } else if edit_this {
                    anchor = Some((bounds, edit.rect));
                }
                if edit_this {
                    drops = Some((buttons, bounds));
                }
            });
            if let Some((buttons, bounds)) = drops.take() {
                crate::edit_toolbar::toolbar_drop(app, ui, &buttons, cols, bounds, Some(&bar.name));
            }
        });
        if let Some(p) = moved.filter(|p| p.x.is_finite() && p.y.is_finite())
            && let Some(b) = app.ui.custom_toolbars.get_mut(k)
        {
            b.pos = [p.x, p.y];
        }
    }
    if let Some((bounds, button)) = clicked_anchor.or(anchor) {
        ctx.data_mut(|d| {
            d.insert_temp(crate::edit_toolbar::toolbar_id(), bounds);
            d.insert_temp(crate::edit_toolbar::button_id(), button);
        });
    }
    for a in actions {
        match a {
            Action::Select(id) => app.select_tool(id),
            Action::Options(id) => {
                app.select_tool(id);
                app.run("tool.options", json!({ "tool": id })).ok();
            }
            // A group that floats already (one strip per group) is raised instead, as the Tools
            // panel's buttons do.
            Action::Flyout(tools, _, name) if tools.first().is_some_and(|key| crate::toolbar::is_floating(app, Some(&name), key)) => {
                if let Some(key) = tools.first() {
                    crate::toolbar::raise_floating(ctx, Some(&name), key);
                }
            }
            Action::Flyout(tools, rect, name) => {
                app.ui.flyout = Some(0);
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::unique("flyout-anchor"), rect);
                    d.insert_temp(egui::Id::unique("flyout-tools"), tools.iter().map(|s| s.to_string()).collect::<Vec<String>>());
                    d.insert_temp(crate::toolbar::flyout_toolbar_id(), Some(name));
                });
            }
            Action::EditToolbar(name) => {
                if let Err(e) = app.run("window.editToolbar", json!({ "toolbar": name })) {
                    app.status(e);
                }
            }
            Action::Hide(name) => {
                if let Err(e) = app.run("window.toolbar.custom", json!({ "name": name, "show": false })) {
                    app.status(e);
                }
            }
            Action::Columns(name, double) => {
                if let Err(e) = app.run("window.toolbarColumns", json!({ "toolbar": name, "double": double })) {
                    app.status(e);
                }
            }
        }
    }
}

/// What a toolbar's tool buttons need from the frame: the toolbar, the active tool, whether the
/// drawer edits the toolbar, and the button whose long press opened its flyout.
struct Press<'a> {
    bar: &'a CustomToolbar,
    active: &'static str,
    edit_this: bool,
    held: Option<egui::Id>,
    held_id: egui::Id,
}

/// One slot's tool button, as the Tools panel's: it shows the tool chosen last from its group, a
/// press selects it, and a long or right press opens the group's flyout; while the drawer edits the
/// toolbar, it drags instead. Returns where it is drawn.
fn slot_button(ui: &mut egui::Ui, t: &Tokens, p: &Press, slot: &[String], actions: &mut Vec<Action>) -> Option<egui::Rect> {
    let tools: Vec<&'static str> = slot.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
    let &first = tools.first()?;
    let is_active = tools.contains(&p.active);
    let shown_id = if is_active { p.active } else { p.bar.slot_tool.get(first).and_then(|id| tool_info(id)).map_or(first, |t| t.id) };
    let shown = tool_info(shown_id).filter(|t| tools.contains(&t.id)).or_else(|| tool_info(first))?;
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, crate::toolbar::PITCH - 1.0), Sense::click_and_drag());
    let well = egui::Rect::from_center_size(rect.center(), vec2(30.0, 27.5));
    if is_active {
        ui.painter().rect_filled(well, CornerRadius::same(6), t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(well, CornerRadius::same(6), t.hover);
    }
    let ir = egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
    icons::paint(ui, icons::tool_icon(shown.icon), ir, if is_active { t.text_strong } else { t.icon });
    if tools.len() > 1 {
        let c = rect.center() + vec2(12.5, 10.0);
        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-3.5, 0.0), c + vec2(0.0, -3.5)], t.icon, Stroke::NONE));
    }
    if p.edit_this {
        resp.dnd_set_drag_payload(crate::edit_toolbar::ToolDrag { tool: shown.id, from_toolbar: true });
    }
    let press = if tools.len() > 1 && !p.edit_this { crate::toolbar::flyout_press(ui, &resp, rect) } else { None };
    let alt = ui.input(|i| i.modifiers.alt);
    if press.is_some() {
        actions.push(Action::Flyout(tools.clone(), rect, p.bar.name.clone()));
        if press == Some(crate::toolbar::FlyoutPress::Primary) {
            ui.data_mut(|d| d.insert_temp(p.held_id, resp.id));
        }
    } else if resp.clicked() && p.held == Some(resp.id) {
        // Releasing the long press that opened the flyout leaves it open.
    } else if resp.clicked() && alt && tools.len() > 1 {
        let i = tools.iter().position(|x| *x == shown.id).unwrap_or(0);
        actions.push(Action::Select(tools[(i + 1) % tools.len()]));
    } else if resp.double_clicked() {
        actions.push(Action::Options(shown.id));
    } else if resp.clicked() {
        actions.push(Action::Select(shown.id));
    }
    let tip = crate::toolbar::tip(shown);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), is_active, &tip));
    resp.on_hover_text(tip);
    Some(rect)
}

/// Remember `id` as the tool shown by each slot of a toolbar made with New Toolbar that holds it,
/// however it was chosen (`toolbar::remember` calls it).
pub(crate) fn remember(ui: &mut UiState, id: &str) {
    for bar in &mut ui.custom_toolbars {
        let firsts: Vec<String> = bar.slots.iter().filter(|s| s.len() > 1 && s.iter().any(|t| t == id)).filter_map(|s| s.first().cloned()).collect();
        for first in firsts {
            bar.slot_tool.insert(first, id.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::menus::Item;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Default::default())
    }

    /// The items of Window › Toolbars: (label, command id, params).
    fn toolbars_menu(app: &VectorcraftApp) -> Vec<(&'static str, &'static str, Value)> {
        crate::shortcut_editor::sync(&app.ui);
        let window = crate::menus::menu_tree().into_iter().find(|(m, _)| *m == "Window").map(|(_, items)| items).unwrap();
        let Some(Item::Sub(_, items)) = window.into_iter().find(|i| matches!(i, Item::Sub("Toolbars", _))) else { panic!("no Toolbars submenu") };
        items.into_iter().filter_map(|i| if let Item::Cmd(l, id, p) = i { Some((l, id, p)) } else { None }).collect()
    }

    /// New Toolbar named "Mine" is listed under Window › Toolbars, checked while it shows; its item
    /// hides and shows it. Without a name, a dialog asks for one.
    #[test]
    fn a_new_toolbar_is_listed_under_window_toolbars() {
        let mut app = app();
        assert_eq!(toolbars_menu(&app).last().map(|(l, id, _)| (*l, *id)), Some(("New Toolbar…", "window.toolbar.new")));
        assert_eq!(app.run("window.toolbar.new", json!({"name": " Mine "})).unwrap(), json!({"toolbar": "Mine"}));
        let menu = toolbars_menu(&app);
        let item = menu.iter().find(|(l, ..)| *l == "Mine").expect("Mine is listed");
        assert_eq!((item.1, &item.2), ("window.toolbar.custom", &json!({"name": "Mine"})));
        assert_eq!(crate::menus::checked(&app, item.1, &item.2), Some(true));
        assert_eq!(crate::menus::display_label(&app, item.1, item.0), "Mine", "listed by its own name");
        // A blank toolbar, floating, shown.
        let bar = find(&app.ui, "mine").unwrap();
        assert!(bar.slots.is_empty() && bar.shown);
        // Its item hides it and shows it again.
        assert_eq!(app.run("window.toolbar.custom", json!({"name": "Mine"})).unwrap(), json!(false));
        assert_eq!(crate::menus::checked(&app, item.1, &item.2), Some(false));
        assert_eq!(app.run("window.toolbar.custom", json!({"name": "MINE", "show": true})).unwrap(), json!(true));
        // Names: not empty, not taken whatever the case.
        assert!(app.run("window.toolbar.new", json!({"name": "  "})).unwrap_err().contains("empty"));
        assert!(app.run("window.toolbar.new", json!({"name": "mine"})).unwrap_err().contains("already"));
        // Whatever the case outside ASCII too: one name, one toolbar.
        app.run("window.toolbar.new", json!({"name": "É"})).unwrap();
        assert!(app.run("window.toolbar.new", json!({"name": "é"})).unwrap_err().contains("already"));
        assert!(app.run("window.toolbar.custom", json!({"name": "é", "show": true})).is_ok());
        app.ui.custom_toolbars.retain(|b| b.name != "É");
        assert!(app.run("window.toolbar.custom", json!({"name": "Nope"})).unwrap_err().contains("Nope"));
        // A name is looked up without its leading and trailing spaces, as it was made.
        assert_eq!(app.run("window.toolbar.custom", json!({"name": " Mine ", "show": true})).unwrap(), json!(true));
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine "})).is_ok());
        assert!(app.run("toolbar.removeTool", json!({"tool": "pen", "toolbar": "Mine"})).is_ok());
        // A name that is not a string is reported as one.
        assert!(app.run("window.toolbar.new", json!({"name": 5})).unwrap_err().contains("must be a string"));
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": 5})).unwrap_err().contains("must be a string"));
        // Without a name: the dialog, its OK makes the toolbar.
        assert_eq!(app.run("window.toolbar.new", json!({})).unwrap(), Value::Null);
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.str("name").as_str()), (KIND, "Toolbar 1"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let names: Vec<&str> = toolbars_menu(&app).into_iter().filter(|(_, id, _)| *id == "window.toolbar.custom").map(|(l, ..)| l).collect();
        assert_eq!(names, ["Mine", "Toolbar 1"]);
        let list = app.run("window.toolbar.list", json!({})).unwrap();
        assert_eq!(
            list,
            json!([{"name": "Mine", "shown": true, "double": false, "slots": []}, {"name": "Toolbar 1", "shown": true, "double": false, "slots": []}])
        );
    }

    /// The New Toolbar dialog: a name typed in it and Enter make the toolbar; a name that is taken is
    /// reported in the status line and the dialog stays open; OK makes it once the name is free, and
    /// Cancel closes the dialog without a toolbar.
    #[test]
    fn the_new_toolbar_dialog_takes_the_name_typed() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| show_dialog(app, ui.ctx()));
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(
                    |c| if let egui::Shape::Text(t) = &c.shape { Some((t.galley.text().to_string(), t.visual_bounding_rect())) } else { None },
                )
                .collect::<Vec<_>>()
        };
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let button =
            |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let at =
            |texts: &[(String, egui::Rect)], s: &str| texts.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("no {s} in the dialog")).1.center();
        // Typed into the Name field, then Enter. The dialog is sized in its first frame and drawn
        // from the second.
        app.run("window.toolbar.new", json!({})).unwrap();
        frame(&mut app, vec![]);
        let texts = frame(&mut app, vec![]);
        let field = at(&texts, "Toolbar 1");
        frame(&mut app, vec![egui::Event::PointerMoved(field), button(field, true)]);
        frame(&mut app, vec![button(field, false)]);
        frame(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Mine".into())]);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("name")).as_deref(), Some("Mine"));
        frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(app.ui.dialog.is_none(), "Enter makes the toolbar");
        assert!(find(&app.ui, "Mine").is_some());
        // A name that is taken: the status line says so, and the dialog stays open.
        app.run("window.toolbar.new", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("MINE"));
        frame(&mut app, vec![]);
        let texts = frame(&mut app, vec![]);
        let ok = at(&texts, "OK");
        frame(&mut app, vec![egui::Event::PointerMoved(ok), button(ok, true)]);
        frame(&mut app, vec![button(ok, false)]);
        assert!(app.ui.status.contains("already"), "{}", app.ui.status);
        assert!(app.ui.dialog.is_some(), "the dialog stays open");
        // OK with a free name makes it.
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Other"));
        frame(&mut app, vec![egui::Event::PointerMoved(ok), button(ok, true)]);
        frame(&mut app, vec![button(ok, false)]);
        assert!(app.ui.dialog.is_none() && find(&app.ui, "Other").is_some());
        // Cancel makes none.
        app.run("window.toolbar.new", json!({})).unwrap();
        frame(&mut app, vec![]);
        let texts = frame(&mut app, vec![]);
        let cancel = at(&texts, "Cancel");
        frame(&mut app, vec![egui::Event::PointerMoved(cancel), button(cancel, true)]);
        frame(&mut app, vec![button(cancel, false)]);
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Mine", "Other"]);
    }

    /// Switching workspaces while the drawer edits a toolbar of the workspace left closes the
    /// drawer, so it names no toolbar that the new workspace lacks.
    #[test]
    fn switching_workspaces_closes_the_drawer_on_a_toolbar_left() {
        let mut app = app();
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("window.editToolbar", json!({"toolbar": "Mine", "open": true})).unwrap();
        assert_eq!(drawer_target(&app.ui), Some("Mine"));
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(!app.ui.tools_drawer && app.ui.drawer_toolbar.is_none());
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).is_err(), "Mine is not in Essentials");
    }

    /// Tools added to a toolbar made with New Toolbar leave the Tools panel's toolbar as it was, and
    /// the toolbar comes back with them after switching workspaces and after a restart.
    #[test]
    fn tools_added_to_a_new_toolbar_survive_a_workspace_reload() {
        let mut app = app();
        let shipped = crate::toolbar::slots(&app);
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        let r = app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "mine", "slot": 0, "place": "group"})).unwrap();
        assert_eq!(r, json!([["pen", "curvature"]]));
        let r = app.run("toolbar.addTool", json!({"tool": "artboard", "toolbar": "Mine", "slot": 0})).unwrap();
        assert_eq!(r, json!([["artboard"], ["pen", "curvature"]]));
        assert_eq!(crate::toolbar::slots(&app), shipped, "the Tools panel's toolbar is untouched");
        assert!(app.ui.toolbar_edits.basic.is_none());
        assert!(app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Nope"})).is_err());
        let r = app.run("toolbar.removeTool", json!({"tool": "artboard", "toolbar": "Mine"})).unwrap();
        assert_eq!(r, json!([["pen", "curvature"]]));
        let mine = find(&app.ui, "Mine").cloned().unwrap();
        // Another workspace has its own toolbars (none); coming back brings Mine back.
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.custom_toolbars.is_empty());
        assert!(toolbars_menu(&app).iter().all(|(l, ..)| *l != "Mine"));
        app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        assert_eq!(find(&app.ui, "Mine"), Some(&mine));
        // Saved under a workspace and in the preferences, it loads again.
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let mut ui = prefs.sanitized();
        assert_eq!(find(&ui, "Mine"), Some(&mine));
        crate::workspaces::switch(&mut ui, "Essentials").unwrap();
        crate::workspaces::switch(&mut ui, "Drawing").unwrap();
        assert_eq!(find(&ui, "Mine"), Some(&mine));
        // Reset makes it blank again.
        assert_eq!(app.run("toolbar.reset", json!({"toolbar": "Mine"})).unwrap(), json!([]));
        // Workspaces saved before New Toolbar load without toolbars.
        let mut old = serde_json::to_value(crate::workspaces::capture(&app.ui, "Old")).unwrap();
        old.as_object_mut().unwrap().remove("toolbars");
        assert!(serde_json::from_value::<crate::workspaces::Workspace>(old).unwrap().toolbars.is_empty());
    }

    /// One frame of the whole window, `height` points tall.
    fn window_frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, height: f32) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, height));
        let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
        ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        })
        .textures_delta
        .clear();
    }

    fn drawn(ctx: &egui::Context, id: egui::Id, order: egui::Order) -> bool {
        ctx.memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(order, id)))
    }

    /// Window › Tools hides the Tools panel alone: a toolbar made with New Toolbar still shows, and
    /// the drawer opens beside it. Tab hides it with the other panels, and a second Tab shows it.
    #[test]
    fn a_new_toolbar_shows_without_the_tools_panel_until_tab() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp| {
            for _ in 0..3 {
                time += 0.05;
                window_frame(app, &ctx, time, 800.0);
            }
        };
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle));
        app.run("window.toolbar", json!({})).unwrap();
        assert!(!app.ui.toolbar);
        app.run("window.editToolbar", json!({"toolbar": "Mine"})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle), "shown without the Tools panel");
        assert!(drawn(&ctx, crate::edit_toolbar::drawer_id(), egui::Order::Foreground), "the drawer beside it");
        app.run("window.editToolbar", json!({"open": false})).unwrap();
        // A group torn off this toolbar shows too; one torn off the Tools panel stays hidden with it.
        let strip = |tools: &[&str]| crate::state::FloatingFlyout { tools: tools.iter().map(|s| s.to_string()).collect(), pos: [400.0, 300.0] };
        find_mut(&mut app.ui, "Mine").unwrap().floating = vec![strip(&["pen", "curvature"])];
        app.ui.floating_flyouts = vec![strip(&["rectangle", "star"])];
        frames(&mut app);
        assert!(drawn(&ctx, egui::Id::unique(("tool-floating", "pen")).with(key("Mine")), egui::Order::Middle));
        assert!(!drawn(&ctx, egui::Id::unique(("tool-floating", "rectangle")), egui::Order::Middle));
        // Hidden with the toolbar it was torn off, and shown again with it.
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": false})).unwrap();
        frames(&mut app);
        assert!(!drawn(&ctx, egui::Id::unique(("tool-floating", "pen")).with(key("Mine")), egui::Order::Middle));
        app.run("window.toolbar.custom", json!({"name": "Mine", "show": true})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, egui::Id::unique(("tool-floating", "pen")).with(key("Mine")), egui::Order::Middle));
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle), "Tab hides it");
        app.run("window.dock", json!({})).unwrap();
        frames(&mut app);
        assert!(drawn(&ctx, area_id("Mine"), egui::Order::Middle), "a second Tab shows it");
    }

    /// A group's button shows the tool chosen last from it however it was chosen (its flyout, a
    /// shortcut), and the Tools panel's buttons keep their own.
    #[test]
    fn a_group_shows_the_tool_chosen_last() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
        app.select_tool("curvature");
        app.select_tool("selection");
        assert_eq!(find(&app.ui, "Mine").unwrap().slot_tool.get("pen").map(String::as_str), Some("curvature"));
        assert_ne!(app.ui.slot_tool.get("pen").map(String::as_str), Some("curvature"), "the Tools panel's Pen group holds no Curvature tool");
        // Kept with the toolbar.
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(find(&prefs.sanitized(), "Mine").unwrap().slot_tool.get("pen").map(String::as_str), Some("curvature"));
    }

    /// A group of a toolbar made with New Toolbar floats on its own, apart from a Tools panel group
    /// with the same first tool: both strips show, and each toolbar finds only its own.
    #[test]
    fn groups_float_apart_from_the_tools_panels() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "curvature", "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
        app.run("window.floatTools", json!({"tool": "pen", "floating": true})).unwrap();
        assert!(crate::toolbar::is_floating(&app, None, "pen"));
        assert!(!crate::toolbar::is_floating(&app, Some("Mine"), "pen"), "the Tools panel's Pen group floats, not Mine's");
        assert_eq!(app.run("window.floatTools", json!({"tool": "curvature", "toolbar": "mine"})).unwrap(), json!(true));
        assert!(crate::toolbar::is_floating(&app, Some("Mine"), "pen"));
        assert_eq!(crate::menus::checked(&app, "window.floatTools", &json!({"tool": "curvature", "toolbar": "Mine"})), Some(true));
        assert!(app.run("window.floatTools", json!({"tool": "selection", "toolbar": "Mine"})).is_err(), "not in a group of Mine");
        assert!(app.run("window.floatTools", json!({"tool": "pen", "toolbar": "Nope"})).is_err());
        // Both kept by the preferences, Mine's with Mine: a build without New Toolbar reads the
        // Tools panel's strips alone. Both are drawn.
        let saved = serde_json::to_value(&app.ui).unwrap();
        let tools_panel = &app.ui.floating_flyouts[0].tools;
        assert_eq!(saved["floating_flyouts"], json!([{"tools": tools_panel, "pos": [90.0, 100.0]}]));
        let prefs = serde_json::from_value::<UiState>(saved).unwrap().sanitized();
        assert_eq!(prefs.floating_flyouts.len(), 1);
        assert_eq!(find(&prefs, "Mine").unwrap().floating.iter().map(|f| f.tools.clone()).collect::<Vec<_>>(), [["pen", "curvature"]]);
        for k in 0..3 {
            window_frame(&mut app, &ctx, f64::from(k) * 0.05, 800.0);
        }
        let main = egui::Id::unique(("tool-floating", "pen"));
        assert!(drawn(&ctx, main, egui::Order::Middle) && drawn(&ctx, main.with(key("Mine")), egui::Order::Middle));
        // Putting the Tools panel's group back leaves Mine's floating.
        app.run("window.floatTools", json!({"tool": "pen", "floating": false})).unwrap();
        assert!(app.ui.floating_flyouts.is_empty() && find(&app.ui, "Mine").unwrap().floating.len() == 1);
        assert_eq!(crate::menus::checked(&app, "window.floatTools", &json!({"tool": "pen"})), Some(false));
        // And Mine's goes back by its command too.
        assert_eq!(app.run("window.floatTools", json!({"tool": "pen", "toolbar": "Mine"})).unwrap(), json!(false));
        assert!(find(&app.ui, "Mine").unwrap().floating.is_empty());
    }

    /// A toolbar with more tools than the window's height holds stays in the window: its tools
    /// scroll.
    #[test]
    fn a_tall_toolbar_scrolls_inside_the_window() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Tall"})).unwrap();
        for tool in vectorcraft_tools::TOOL_GROUPS.iter().flat_map(|g| g.iter()).take(30) {
            app.run("toolbar.addTool", json!({"tool": tool.id, "toolbar": "Tall"})).unwrap();
        }
        assert_eq!(find(&app.ui, "Tall").unwrap().slots.len(), 30);
        for k in 0..4 {
            window_frame(&mut app, &ctx, f64::from(k) * 0.05, 500.0);
        }
        let r = ctx.memory(|m| m.area_rect(area_id("Tall"))).unwrap();
        assert!(r.bottom() <= 500.0 && r.top() >= 0.0, "{r:?}");
    }

    /// Preferences › Devices › Enable Touch Workspace switches to Touch and back as Window ›
    /// Workspace does: the toolbars made in the workspace left come back with it.
    #[test]
    fn the_touch_workspace_brings_the_toolbars_back() {
        let mut app = app();
        app.run("window.workspace", json!({"name": "Typography"})).unwrap();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.session.prefs.touch_workspace = true;
        crate::workspaces::sync_touch_workspace(&mut app, true);
        assert_eq!(app.ui.workspace, crate::workspaces::TOUCH);
        assert!(app.ui.custom_toolbars.is_empty());
        app.session.prefs.touch_workspace = false;
        crate::workspaces::sync_touch_workspace(&mut app, true);
        assert_eq!(app.ui.workspace, "Typography");
        assert!(find(&app.ui, "Mine").is_some(), "back with Typography");
    }

    /// Reset Workspace keeps the toolbars made in the workspace, in a built-in one too, and New
    /// Workspace leaves the workspace it starts from with its toolbars: the new one starts with a
    /// copy, and switching back brings the first workspace's own back.
    #[test]
    fn toolbars_stay_in_their_workspace_through_reset_and_new_workspace() {
        let mut app = app();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        app.run("toolbar.addTool", json!({"tool": "pen", "toolbar": "Mine"})).unwrap();
        app.run("window.toolbar", json!({})).unwrap();
        app.run("window.workspace.reset", json!({})).unwrap();
        assert!(app.ui.toolbar, "the layout is reset");
        assert_eq!(find(&app.ui, "Mine").map(|b| b.slots.clone()), Some(vec![vec!["pen".to_string()]]), "Mine stays in Essentials");
        let mine = find(&app.ui, "Mine").cloned().unwrap();
        // The layout changes go into the new workspace; Essentials keeps only its toolbars.
        app.run("window.toolbar", json!({})).unwrap();
        app.run("window.workspace.new", json!({"name": "Drawing"})).unwrap();
        assert_eq!(find(&app.ui, "Mine"), Some(&mine), "Drawing starts with a copy");
        app.run("window.toolbar.new", json!({"name": "Only in Drawing"})).unwrap();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.toolbar, "Essentials as saved");
        assert_eq!(app.ui.custom_toolbars, std::slice::from_ref(&mine), "with its own toolbars");
        app.run("window.workspace", json!({"name": "Drawing"})).unwrap();
        assert_eq!(app.ui.custom_toolbars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Mine", "Only in Drawing"]);
    }

    /// The title bar's × hides the toolbar, and its double arrow lays its tools out in two columns
    /// and back in one (`window.toolbarColumns` with `toolbar`).
    #[test]
    fn the_title_bar_hides_the_toolbar_and_toggles_two_columns() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        app.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
        for tool in ["pen", "rectangle", "selection"] {
            app.run("toolbar.addTool", json!({"tool": tool, "toolbar": "Mine"})).unwrap();
        }
        let mut time = 0.0;
        let mut frames = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            for e in std::iter::once(events).chain(std::iter::repeat_with(Vec::new)).take(3) {
                time += 0.05;
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
                let raw = egui::RawInput { time: Some(time), screen_rect: Some(screen), events: e, ..Default::default() };
                ctx.run_ui(raw, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                })
                .textures_delta
                .clear();
            }
        };
        let click = |p: egui::Pos2| {
            let button =
                |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            vec![egui::Event::PointerMoved(p), button(true), button(false)]
        };
        frames(&mut app, vec![]);
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        let one = r.width();
        // The double arrow at the right end of the title bar.
        frames(&mut app, click(pos2(r.right() - 1.0 - TITLE_BUTTON / 2.0, r.top() + 1.0 + GRIP / 2.0)));
        assert!(find(&app.ui, "Mine").unwrap().double);
        assert_eq!(crate::menus::checked(&app, "window.toolbarColumns", &json!({"toolbar": "Mine"})), Some(true));
        assert!(!app.ui.toolbar_double, "the Tools panel keeps one column");
        frames(&mut app, vec![]);
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        assert!((r.width() - one - BUTTON).abs() < 0.5, "{one} then {}", r.width());
        assert_eq!(app.run("window.toolbarColumns", json!({"toolbar": "mine"})).unwrap(), json!(false));
        assert!(app.run("window.toolbarColumns", json!({"toolbar": "Nope"})).is_err());
        // The × at the left end hides it.
        frames(&mut app, vec![]);
        let r = ctx.memory(|m| m.area_rect(area_id("Mine"))).unwrap();
        frames(&mut app, click(pos2(r.left() + 1.0 + TITLE_BUTTON / 2.0, r.top() + 1.0 + GRIP / 2.0)));
        assert!(!find(&app.ui, "Mine").unwrap().shown);
        assert!(!drawn(&ctx, area_id("Mine"), egui::Order::Middle));
    }

    /// A hand-edited file: unnamed toolbars and repeated names go, unknown tools go, each tool once.
    #[test]
    fn toolbars_load_sanitized() {
        let bar = |name: &str, slots: &[&[&str]]| CustomToolbar {
            name: name.into(),
            slots: slots.iter().map(|s| s.iter().map(|id| id.to_string()).collect()).collect(),
            shown: true,
            pos: [f32::NAN, 0.0],
            slot_tool: Default::default(),
            double: false,
            floating: ["pen", "nope", "pen"]
                .iter()
                .map(|id| crate::state::FloatingFlyout { tools: vec![id.to_string(), "curvature".into()], pos: [0.0, 0.0] })
                .collect(),
        };
        let mut bars = vec![bar("A", &[&["pen", "nope"], &["pen"], &["nope"]]), bar("a", &[]), bar(" ", &[]), bar("B", &[])];
        sanitize(&mut bars);
        assert_eq!(bars.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(bars[0].slots, [["pen"]]);
        assert_eq!(bars[0].pos, NEW_AT);
        // One strip per group, of known tools.
        assert_eq!(bars[0].floating.iter().map(|f| f.tools.join(",")).collect::<Vec<_>>(), ["pen,curvature", "curvature"]);
    }
}
