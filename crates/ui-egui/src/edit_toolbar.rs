//! Edit Toolbar (#1018): the All Tools drawer that the ••• button at the bottom of the toolbar
//! opens beside it, and the toolbar edits it makes. The drawer lists every tool under the Basic
//! toolbar's group headings (and headings for the tools only the Advanced toolbar shows); the tools
//! already in the toolbar are dimmed. While it is open the toolbar can be edited: a tool dragged
//! from the drawer onto the toolbar is added there (dropped between two tools, or onto one to join
//! its group), one dragged from the toolbar into the drawer is removed, and the toolbar's tools
//! drag to new places. Escape or a click outside closes it. Picking a tool in it selects the tool.
//!
//! Behaviour from the public documentation: <https://helpx.adobe.com/illustrator/using/tools.html>
//! (Edit Toolbar, the All Tools drawer; tools are added, removed or rearranged only while the drawer
//! is open, and the ones already in the toolbar show dimmed in it).
//!
//! Each toolbar (Basic, Advanced) keeps its own edits in `UiState::toolbar_edits`, saved with the
//! preferences; `toolbar.addTool`, `toolbar.removeTool` and `toolbar.reset` make them.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use egui::{DragAndDrop, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_tools::{TOOL_GROUPS, ToolInfo, tool_info};

use crate::state::{ToolbarCategory, ToolbarEdits};
use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, widgets};

/// A tool's row in the drawer: its icon, name and shortcut.
const ROW: f32 = 26.0;
const ICON: f32 = 18.0;
/// The drawer's width: one column of rows, framed in groups, and room for the scroll bar.
const WIDTH: f32 = 268.0;
/// Gap between the toolbar and the drawer, and the drawer's least distance from the window's edges.
const GAP: f32 = 4.0;
/// Size of the tool following the pointer while it is dragged.
const GHOST: f32 = 30.0;

/// A tool dragged between the drawer and the toolbar (egui's drag-and-drop payload).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToolDrag {
    pub tool: &'static str,
    /// Dragged from the toolbar (dropping it in the drawer removes it).
    pub from_toolbar: bool,
}

/// Where a tool dropped on a toolbar slot goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// Its own slot, before the slot (on the divider above it, or left of it in two columns).
    Before,
    /// Its own slot, after the slot.
    After,
    /// Into the slot's group (its flyout), last.
    Group,
}

impl Place {
    const ALL: [Self; 3] = [Self::Before, Self::After, Self::Group];

    /// The `place` param (omitted: before).
    fn parse(p: &Value) -> Result<Self, String> {
        let key = "place";
        match p.get(key) {
            None | Some(Value::Null) => Ok(Self::Before),
            Some(v) => Self::ALL
                .into_iter()
                .find(|place| v.as_str() == Some(place.name()))
                .ok_or_else(|| format!("`{key}` must be one of {}", Self::ALL.map(Self::name).join(", "))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::Group => "group",
        }
    }
}

// ---------- the model ----------

/// A toolbar as it ships: the Basic toolbar's categories, or the Advanced toolbar's tool groups.
fn shipped(advanced: bool) -> Vec<ToolbarCategory> {
    let strings = |s: &[&str]| s.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    if advanced {
        let slots = TOOL_GROUPS.iter().map(|g| g.iter().map(|t| t.id.to_string()).collect()).collect();
        vec![ToolbarCategory { name: None, slots }]
    } else {
        crate::toolbar::BASIC
            .iter()
            .map(|(cat, slots)| ToolbarCategory { name: Some(cat.to_string()), slots: slots.iter().map(|s| strings(s)).collect() })
            .collect()
    }
}

fn current(edits: &mut ToolbarEdits, advanced: bool) -> &mut Option<Vec<ToolbarCategory>> {
    if advanced { &mut edits.advanced } else { &mut edits.basic }
}

/// The Basic toolbar category named `name`.
fn category(name: &str) -> Option<&'static str> {
    crate::toolbar::BASIC.iter().map(|(c, _)| *c).find(|c| *c == name)
}

/// The current toolbar's slots as Edit Toolbar left them, laid out as `toolbar::slots` lays them
/// out (the category name on each category's first slot); `None` while it is as it ships.
pub(crate) fn edited_slots(app: &VectorcraftApp) -> Option<Vec<(Option<&'static str>, Vec<&'static str>)>> {
    let edits = &app.ui.toolbar_edits;
    let cats = if app.ui.toolbar_advanced { &edits.advanced } else { &edits.basic };
    let mut out = vec![];
    for c in cats.as_ref()? {
        let mut name = c.name.as_deref().and_then(category);
        for s in &c.slots {
            let tools: Vec<&'static str> = s.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect();
            if !tools.is_empty() {
                out.push((name.take(), tools));
            }
        }
    }
    Some(out)
}

/// Drop the empty slots, then the empty categories.
fn prune(cats: &mut Vec<ToolbarCategory>) {
    for c in cats.iter_mut() {
        c.slots.retain(|s| !s.is_empty());
    }
    cats.retain(|c| !c.slots.is_empty());
}

/// Keep known tools, each once, and known category names (a hand-edited preferences file).
pub(crate) fn sanitize(edits: &mut ToolbarEdits) {
    for cats in [&mut edits.basic, &mut edits.advanced].into_iter().flatten() {
        let mut seen = BTreeSet::new();
        for c in cats.iter_mut() {
            c.name = c.name.as_deref().and_then(category).map(str::to_string);
            for s in &mut c.slots {
                s.retain(|id| tool_info(id).is_some() && seen.insert(id.clone()));
            }
        }
        prune(cats);
    }
}

/// Change the current toolbar with `f`: an edit that gives back the toolbar as it ships forgets
/// the edits. Returns the toolbar's slots.
fn edit(app: &mut VectorcraftApp, f: impl FnOnce(&mut Vec<ToolbarCategory>) -> Result<(), String>) -> Result<Value, String> {
    let advanced = app.ui.toolbar_advanced;
    let slot = current(&mut app.ui.toolbar_edits, advanced);
    let mut cats = slot.clone().unwrap_or_else(|| shipped(advanced));
    f(&mut cats)?;
    prune(&mut cats);
    *slot = (cats != shipped(advanced)).then_some(cats);
    Ok(slots_json(app))
}

/// The current toolbar's slots, each a list of tool ids (`toolbar.*` commands return it).
fn slots_json(app: &VectorcraftApp) -> Value {
    json!(crate::toolbar::slots(app).into_iter().map(|(_, s)| s).collect::<Vec<_>>())
}

/// Put `tool` by the slot numbered `slot` (over the toolbar's slots, 0 = the first; none or null:
/// at the end), moving it if it is in the toolbar already. A tool appended to a toolbar left without
/// categories starts one (`named`: under its own heading).
fn add(cats: &mut Vec<ToolbarCategory>, tool: &str, slot: Option<&Value>, place: Place, named: bool) -> Result<(), String> {
    let at: Vec<(usize, usize)> = cats.iter().enumerate().flat_map(|(c, cat)| (0..cat.slots.len()).map(move |s| (c, s))).collect();
    let bad_slot = || {
        let (key, min, max, x) = ("slot", 0, at.len().saturating_sub(1), slot.unwrap_or(&Value::Null));
        format!("`{key}` must be a whole number between {min} and {max} (got {x})")
    };
    let target = match slot {
        None | Some(Value::Null) => None,
        Some(v) => Some(*v.as_u64().and_then(|k| usize::try_from(k).ok()).and_then(|k| at.get(k)).ok_or_else(bad_slot)?),
    };
    let holds =
        |cats: &[ToolbarCategory], (c, s): (usize, usize)| cats.get(c).and_then(|cat| cat.slots.get(s)).is_some_and(|s| s.iter().any(|t| t == tool));
    if place == Place::Group && target.is_some_and(|t| holds(cats, t)) {
        // Onto its own group: nothing to do.
        return Ok(());
    }
    // Out of where it was, leaving its slot in place (emptied ones go after) so `target` holds.
    for c in cats.iter_mut() {
        for s in &mut c.slots {
            s.retain(|t| t != tool);
        }
    }
    let new = vec![tool.to_string()];
    let Some((c, s)) = target else {
        match cats.last_mut() {
            Some(cat) => cat.slots.push(new),
            None => cats.push(ToolbarCategory { name: named.then(|| heading(tool)).and_then(category).map(str::to_string), slots: vec![new] }),
        }
        return Ok(());
    };
    let cat = cats.get_mut(c).ok_or_else(bad_slot)?;
    match place {
        Place::Group => cat.slots.get_mut(s).ok_or_else(bad_slot)?.push(tool.to_string()),
        Place::Before => cat.slots.insert(s.min(cat.slots.len()), new),
        Place::After => cat.slots.insert((s + 1).min(cat.slots.len()), new),
    }
    Ok(())
}

/// Take `tool` out of the toolbar (`group`: with the rest of its group); one that isn't in it
/// stays out.
fn remove(cats: &mut [ToolbarCategory], tool: &str, group: bool) {
    if let Some(slot) = cats.iter_mut().flat_map(|c| c.slots.iter_mut()).find(|s| s.iter().any(|t| t == tool)) {
        if group {
            slot.clear();
        } else {
            slot.retain(|t| t != tool);
        }
    }
}

/// A known tool's id from the `tool` param.
fn tool_param(p: &Value) -> Result<&'static str, String> {
    let tool = p.get("tool").and_then(Value::as_str).ok_or("tool (a tool id) is required")?;
    tool_info(tool).map(|t| t.id).ok_or_else(|| format!("unknown tool `{tool}`"))
}

/// The Edit Toolbar commands; `None` for other ids.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.editToolbar" => crate::menus::opt_bool(p, "open").map(|open| {
            let open = open.unwrap_or(!app.ui.tools_drawer);
            app.ui.tools_drawer = open;
            if open {
                app.ui.flyout = None;
            }
            json!(open)
        }),
        "toolbar.addTool" => (|| {
            let tool = tool_param(p)?;
            let place = Place::parse(p)?;
            let named = !app.ui.toolbar_advanced;
            edit(app, |cats| add(cats, tool, p.get("slot"), place, named))
        })(),
        "toolbar.removeTool" => (|| {
            let tool = tool_param(p)?;
            let group = crate::menus::opt_bool(p, "group")?.unwrap_or(false);
            edit(app, |cats| {
                remove(cats, tool, group);
                Ok(())
            })
        })(),
        "toolbar.show" => (|| {
            let on = crate::menus::opt_bool(p, "on")?;
            if let Some(name) = p.get("control").filter(|v| !v.is_null()) {
                let name = name.as_str().ok_or("control must be fillStroke, coloring, drawMode or screenMode")?;
                let flag = app
                    .ui
                    .toolbar_show
                    .get_mut(name)
                    .ok_or_else(|| format!("unknown control `{name}`: fillStroke, coloring, drawMode or screenMode"))?;
                *flag = on.unwrap_or(!*flag);
            }
            Ok(app.ui.toolbar_show.json())
        })(),
        "toolbar.reset" => {
            *current(&mut app.ui.toolbar_edits, app.ui.toolbar_advanced) = None;
            Ok(slots_json(app))
        }
        _ => return None,
    })
}

// ---------- dropping on the toolbar ----------

/// Where a tool dropped at `at` goes, given the toolbar's slot buttons (slot index, rect) in a
/// toolbar `cols` columns wide: onto a button's middle joins its group, onto its edge (top or
/// bottom; left or right in two columns) goes beside it; elsewhere it goes beside the nearest
/// button. `(None, _)`: at the end (no buttons).
pub(crate) fn drop_target(slots: &[(usize, Rect)], at: Pos2, cols: usize) -> (Option<usize>, Place) {
    let Some(&(k, r)) = slots.iter().min_by(|a, b| a.1.distance_sq_to_pos(at).total_cmp(&b.1.distance_sq_to_pos(at))) else {
        return (None, Place::Before);
    };
    if r.contains(at) {
        let f = if cols == 1 { (at.y - r.top()) / r.height().max(1.0) } else { (at.x - r.left()) / r.width().max(1.0) };
        let place = if f < 0.3 {
            Place::Before
        } else if f > 0.7 {
            Place::After
        } else {
            Place::Group
        };
        return (Some(k), place);
    }
    let before = at.y < r.top() || (at.y <= r.bottom() && at.x < r.center().x);
    (Some(k), if before { Place::Before } else { Place::After })
}

/// While a tool is dragged over the toolbar (`bounds`): mark where it would go, and on the release
/// put it there (`toolbar.addTool`).
pub(crate) fn toolbar_drop(app: &mut VectorcraftApp, ui: &Ui, slots: &[(usize, Rect)], cols: usize, bounds: Rect) {
    let Some(drag) = DragAndDrop::payload::<ToolDrag>(ui.ctx()) else { return };
    let Some(at) = ui.ctx().pointer_interact_pos().filter(|p| bounds.contains(*p)) else { return };
    let t = Tokens::get(ui.ctx());
    let (slot, place) = drop_target(slots, at, cols);
    if let Some(r) = slot.and_then(|k| slots.iter().find(|s| s.0 == k)).map(|s| s.1) {
        let line = Stroke::new(2.0, t.accent);
        match (place, cols == 1) {
            (Place::Group, _) => {
                ui.painter().rect_stroke(r.shrink(1.0), 2.0, line, StrokeKind::Inside);
            }
            (Place::Before, true) => {
                ui.painter().hline(r.x_range(), r.top(), line);
            }
            (Place::After, true) => {
                ui.painter().hline(r.x_range(), r.bottom(), line);
            }
            (Place::Before, false) => {
                ui.painter().vline(r.left(), r.y_range(), line);
            }
            (Place::After, false) => {
                ui.painter().vline(r.right(), r.y_range(), line);
            }
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        DragAndDrop::clear_payload(ui.ctx());
        let p = json!({ "tool": drag.tool, "slot": slot, "place": place.name() });
        if let Err(e) = app.run("toolbar.addTool", p) {
            app.ui.status = e;
        }
    }
}

// ---------- the drawer ----------

/// Where the toolbar leaves its ••• button's rect and its own for the drawer, each frame.
pub(crate) fn button_id() -> egui::Id {
    egui::Id::unique("edit-toolbar-button")
}

/// Stable id of the floating edit toolbar.
pub(crate) fn toolbar_id() -> egui::Id {
    egui::Id::unique("edit-toolbar-toolbar")
}

/// The drawer's area.
pub(crate) fn drawer_id() -> egui::Id {
    egui::Id::unique("tools-drawer")
}

/// The heading a tool is listed under: its Basic toolbar group, else one for the tools only the
/// Advanced toolbar shows.
fn heading(id: &str) -> &'static str {
    if let Some((cat, _)) = crate::toolbar::BASIC.iter().find(|(_, slots)| slots.iter().any(|s| s.contains(&id))) {
        return cat;
    }
    match id {
        _ if id.starts_with("symbol") => "Symbols",
        _ if id.ends_with("Graph") => "Graph",
        _ if id.starts_with("perspective") => "Perspective",
        "artboard" => "Artboards",
        "slice" | "sliceSelection" => "Slices",
        "objectsOnPath" => "Modify",
        _ => "More",
    }
}

/// The headings in order, each with its tools in the toolbar's order.
fn sections() -> &'static [(&'static str, Vec<&'static ToolInfo>)] {
    static SECTIONS: OnceLock<Vec<(&'static str, Vec<&'static ToolInfo>)>> = OnceLock::new();
    SECTIONS.get_or_init(|| {
        let mut out: Vec<(&'static str, Vec<&'static ToolInfo>)> = crate::toolbar::BASIC.iter().map(|(cat, _)| (*cat, vec![])).collect();
        for tool in TOOL_GROUPS.iter().flat_map(|g| g.iter()) {
            let h = heading(tool.id);
            match out.iter_mut().find(|(c, _)| *c == h) {
                Some((_, tools)) => tools.push(tool),
                None => out.push((h, vec![tool])),
            }
        }
        out.retain(|(_, tools)| !tools.is_empty());
        out
    })
}

/// A heading of the drawer and the flyout groups listed under it.
type Heading = (&'static str, Vec<&'static [ToolInfo]>);

/// The headings in order, each with the flyout groups listed under it (a group under the heading
/// most of its tools have, the first tool's on a tie), each in its tools' order: what the drawer
/// frames together.
fn groups() -> &'static [Heading] {
    static GROUPS: OnceLock<Vec<Heading>> = OnceLock::new();
    GROUPS.get_or_init(|| {
        let mut out: Vec<Heading> = sections().iter().map(|(cat, _)| (*cat, vec![])).collect();
        for g in TOOL_GROUPS {
            let Some(first) = g.first() else { continue };
            let count = |h: &str| g.iter().filter(|t| heading(t.id) == h).count();
            let h = g.iter().map(|t| heading(t.id)).fold(heading(first.id), |best, h| if count(h) > count(best) { h } else { best });
            match out.iter_mut().find(|(c, _)| *c == h) {
                Some((_, gs)) => gs.push(g),
                None => out.push((h, vec![g])),
            }
        }
        out.retain(|(_, gs)| !gs.is_empty());
        out
    })
}

/// The All Tools drawer beside the toolbar, its top by the ••• button (higher when that would put
/// it past the window's bottom), while `tools_drawer` is on.
pub(crate) fn drawer(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !app.ui.tools_drawer {
        return;
    }
    let (Some(button), Some(bar)) = ctx.data(|d| (d.get_temp::<Rect>(button_id()), d.get_temp::<Rect>(toolbar_id()))) else { return };
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let id = drawer_id();
    let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(WIDTH, 200.0), |r| r.size());
    let x = (bar.right() + GAP).min(screen.right() - size.x - GAP).max(screen.left());
    // Never above the toolbar's top: higher up are the window's title bar (and on macOS its
    // buttons) and the bars under it.
    let top = bar.top().max(screen.top() + GAP);
    let y = button.top().min(screen.bottom() - size.y - GAP).max(top);
    let in_toolbar: BTreeSet<&str> = crate::toolbar::slots(app).into_iter().flat_map(|(_, s)| s).collect();
    let active = app.session.tool_id();
    let dragged = DragAndDrop::payload::<ToolDrag>(ctx).map(|d| *d);
    let pointer = ctx.pointer_interact_pos();
    let over = |r: Rect| pointer.is_some_and(|p| r.contains(p));
    let removing = dragged.is_some_and(|d| d.from_toolbar) && ctx.memory(|m| m.area_rect(id)).is_some_and(over);
    let mut picked = None;
    let mut reset = false;
    let mut advanced = None;
    let mut show_toggle = None;
    let resp = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).show(ctx, |ui| {
        crate::toolbar::flyout_frame(&t, removing).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            let width = WIDTH.min(screen.width() - 2.0 * GAP - 16.0).max(160.0);
            ui.set_width(width);
            // All Tools, and the drawer's flyout menu: Reset, and the Basic or Advanced toolbar.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("All Tools")).size(13.0).color(t.text_strong));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, menu) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    icons::paint(ui, "menu", r.shrink(1.0), if menu.hovered() { t.text_strong } else { t.text_dim });
                    menu.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Drawer menu")));
                    let menu = menu.on_hover_text(tl!("Drawer menu"));
                    egui::Popup::menu(&menu).id(menu_id()).show(|ui| {
                        ui.set_min_width(160.0);
                        if widgets::menu_item(ui, "Reset", true, false) {
                            reset = true;
                        }
                        ui.separator();
                        for (label, adv) in [("Basic", false), ("Advanced", true)] {
                            if widgets::menu_item(ui, label, true, app.ui.toolbar_advanced == adv) {
                                advanced = Some(adv);
                            }
                        }
                    });
                });
            });
            ui.add_space(4.0);
            // As tall as its tools, up to the window's height (it scrolls then); the Area's size
            // from the last frame mustn't cap it.
            let room = (screen.bottom() - top - GAP - 104.0).max(120.0);
            egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let inner = ui.available_width() - 10.0;
                for (cat, gs) in groups() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(format!("{}:", tl!(cat))).size(12.0).color(t.text_strong));
                    ui.add_space(4.0);
                    for g in gs {
                        // A flyout group: its tools framed together, as one toolbar button holds them.
                        egui::Frame::NONE.stroke(Stroke::new(1.0, t.divider)).corner_radius(2.0).show(ui, |ui| {
                            ui.set_width(inner);
                            for tool in g.iter() {
                                if row(ui, &t, tool, tool.id == active, in_toolbar.contains(tool.id)) {
                                    picked = Some(tool.id);
                                }
                            }
                        });
                        ui.add_space(5.0);
                    }
                }
            });
            // Show: the controls under the toolbar's tools.
            ui.add_space(4.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Show:")).size(12.0).color(t.text));
                ui.add_space(4.0);
                for (name, label, icon) in crate::state::ToolbarShow::CONTROLS {
                    if show_button(ui, &t, name, label, icon, app.ui.toolbar_show.get(name)) {
                        show_toggle = Some(name);
                    }
                }
            });
        });
    });
    let rect = resp.response.rect;
    // A tool dragged from the toolbar and dropped in the drawer leaves the toolbar.
    if let Some(d) = dragged.filter(|d| d.from_toolbar && over(rect))
        && ctx.input(|i| i.pointer.any_released())
    {
        DragAndDrop::clear_payload(ctx);
        if let Err(e) = app.run("toolbar.removeTool", json!({ "tool": d.tool })) {
            app.ui.status = e;
        }
    }
    if reset && let Err(e) = app.run("toolbar.reset", json!({})) {
        app.ui.status = e;
    }
    // Basic or Advanced: the menu item of the other toolbar switches to it.
    if advanced.is_some_and(|a| a != app.ui.toolbar_advanced)
        && let Err(e) = app.run("window.toolbarAdvanced", json!({}))
    {
        app.ui.status = e;
    }
    if let Some(name) = show_toggle
        && let Err(e) = app.run("toolbar.show", json!({ "control": name }))
    {
        app.ui.status = e;
    }
    if let Some(id) = picked {
        app.select_tool(id);
        app.ui.tools_drawer = false;
    } else if resp.response.clicked_elsewhere() && !pointer.is_some_and(|p| bar.contains(p)) && !egui::Popup::is_id_open(ctx, menu_id()) {
        // A click in the toolbar (picking a tool, or the ••• button that toggles the drawer) leaves
        // it open.
        app.ui.tools_drawer = false;
    }
    ghost(ctx, &t);
}

/// A Show section toggle: lit while its control shows under the toolbar's tools. Fill and Stroke
/// and Coloring are drawn as small copies of those controls, the modes with their buttons' icons.
fn show_button(ui: &mut Ui, t: &Tokens, name: &str, label: &str, icon: &str, on: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, tl!(label)));
    let p = ui.painter();
    if on {
        p.rect_filled(r, 3.0, t.tool_active);
    } else if resp.hovered() {
        p.rect_filled(r, 3.0, t.hover);
    }
    let ink = if on { t.text_strong } else { t.icon };
    let c = r.center();
    match name {
        "fillStroke" => {
            // The proxy: a filled square in front of a stroked one.
            let back = Rect::from_center_size(c + vec2(3.0, 3.0), vec2(10.0, 10.0));
            p.rect_stroke(back, 0.0, Stroke::new(2.0, ink), StrokeKind::Inside);
            p.rect_filled(Rect::from_center_size(c - vec2(3.0, 3.0), vec2(10.0, 10.0)), 0.0, ink);
        }
        "coloring" => {
            // Color, Gradient and None.
            for (k, x) in [-6.5f32, 0.0, 6.5].into_iter().enumerate() {
                let chip = Rect::from_center_size(c + vec2(x, 0.0), vec2(5.5, 9.0));
                match k {
                    0 => {
                        p.rect_filled(chip, 0.0, ink);
                    }
                    1 => {
                        p.rect_filled(chip, 0.0, ink.gamma_multiply(0.5));
                    }
                    _ => {
                        p.rect_stroke(chip, 0.0, Stroke::new(1.0, ink), StrokeKind::Inside);
                        p.line_segment([chip.left_bottom(), chip.right_top()], Stroke::new(1.0, ink));
                    }
                }
            }
        }
        _ => icons::paint(ui, icon, r.shrink(5.0), ink),
    }
    resp.on_hover_text(tl!(label)).clicked()
}

/// The drawer's flyout menu.
fn menu_id() -> egui::Id {
    egui::Id::unique("tools-drawer-menu")
}

/// A tool's row in the drawer: icon, name and shortcut, dimmed when the tool is in the toolbar
/// already (it can't be dragged in again). Returns whether it was clicked.
fn row(ui: &mut Ui, t: &Tokens, tool: &'static ToolInfo, on: bool, dimmed: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), if dimmed { Sense::click() } else { Sense::click_and_drag() });
    if !dimmed {
        resp.dnd_set_drag_payload(ToolDrag { tool: tool.id, from_toolbar: false });
    }
    let fill = rect.shrink(1.0);
    if on {
        ui.painter().rect_filled(fill, 2.0, t.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(fill, 2.0, t.hover);
    }
    // The active tool's row is lit, dimmed or not.
    let color = if on {
        t.text_strong
    } else if dimmed {
        t.text_disabled
    } else {
        t.text
    };
    let icon = Rect::from_center_size(rect.left_center() + vec2(8.0 + ICON / 2.0, 0.0), vec2(ICON, ICON));
    icons::paint(ui, icons::tool_icon(tool.icon), icon, if dimmed || on { color } else { t.icon });
    let painter = ui.painter().with_clip_rect(rect);
    let shortcut = crate::shortcut_editor::tool_shortcut(tool.id);
    let keys_w = shortcut.as_ref().map_or(0.0, |k| {
        let keys = if on {
            t.text_strong
        } else if dimmed {
            t.text_disabled
        } else {
            t.text_dim
        };
        let g = painter.layout_no_wrap(k.to_string(), egui::FontId::proportional(11.5), keys);
        let w = g.size().x;
        painter.galley(pos2(rect.right() - 10.0 - w, rect.center().y - g.size().y / 2.0), g, keys);
        w + 12.0
    });
    let text_at = pos2(icon.right() + 10.0, rect.center().y);
    painter.with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.right() - 10.0 - keys_w, rect.bottom()))).text(
        text_at,
        egui::Align2::LEFT_CENTER,
        tl!(tool.label),
        egui::FontId::proportional(12.5),
        color,
    );
    resp.on_hover_text(crate::toolbar::tip(tool)).clicked()
}

/// The tool being dragged, by the pointer (below and right of it, clear of the drop marker).
fn ghost(ctx: &egui::Context, t: &Tokens) {
    let Some(d) = DragAndDrop::payload::<ToolDrag>(ctx) else { return };
    let (Some(at), Some(tool)) = (ctx.pointer_interact_pos(), tool_info(d.tool)) else { return };
    let r = Rect::from_min_size(at + vec2(8.0, 8.0), vec2(GHOST, GHOST));
    egui::Area::new(egui::Id::unique("tool-drag-ghost")).order(egui::Order::Tooltip).fixed_pos(r.min).interactable(false).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(r.size(), Sense::hover());
        ui.painter().rect(r, 3.0, t.panel, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        icons::paint(ui, icons::tool_icon(tool.icon), Rect::from_center_size(r.center(), vec2(18.0, 18.0)), t.text_strong);
    });
}

#[cfg(test)]
mod tests;
