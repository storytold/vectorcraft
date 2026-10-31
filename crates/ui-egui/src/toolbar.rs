//! The Tools panel. Default: Illustrator 2026's categorized single-column toolbar (Select, Shapes,
//! Draw, Modify, Type, Navigate, Color). Window → Toolbars → Advanced shows every tool group; the
//! double arrow at the top (or Window → Toolbars → Double Column, `window.toolbarColumns`) lays the
//! tools out in one or two columns.
//! Bottom: fill/stroke proxy, colour/gradient/none, drawing modes, screen mode, Edit Toolbar (its
//! All Tools drawer and the toolbar edits it makes: `edit_toolbar`).

use std::sync::Arc;

use egui::{Color32, CornerRadius, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_tools::{Mods, TOOL_GROUPS, ToolInfo, ToolKey, tool_info};

use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, icons, widgets};

const PITCH: f32 = 30.0;
const WIDTH: f32 = 48.0;
/// Seconds a press on a tool group's button is held before its flyout opens.
const LONG_PRESS: f64 = 0.35;
/// Width of the grab bar down a flyout's right side: dragging it tears the flyout off.
const BAR: f32 = 10.0;
/// Height of the × at the top of a floating flyout's bar.
const CLOSE: f32 = 14.0;
/// Points the pointer travels on a flyout's bar before the flyout tears off.
const TEAR: f32 = 3.0;
/// Seconds a floating flyout flashes when a press that would open its flyout raises it.
const FLASH: f64 = 0.4;
/// Size of a tool button in a floating flyout's strip.
const STRIP_BUTTON: egui::Vec2 = vec2(32.0, PITCH);

/// The Basic toolbar: (category, slots); each slot is a flyout group (first = default).
pub const BASIC: &[(&str, &[&[&str]])] = &[
    ("Select", &[&["selection"], &["directSelection", "groupSelection"], &["lasso", "magicWand"]]),
    (
        "Shapes",
        &[
            &["rectangle", "roundedRectangle", "star", "lineSegment", "arc", "spiral", "rectangularGrid", "polarGrid", "flare"],
            &["ellipse"],
            &["polygon"],
            &["shaper"],
        ],
    ),
    (
        "Draw",
        &[
            &["pencil", "smooth", "pathEraser", "join"],
            &["eraser", "scissors", "knife", "mirrorCut", "lineCut", "rectCut"],
            &["paintbrush", "blobBrush"],
            &["pen", "addAnchor", "deleteAnchor", "anchorPoint"],
            &["curvature"],
        ],
    ),
    (
        "Modify",
        &[
            &["width", "warp", "twirl", "pucker", "bloat", "scallop", "crystallize", "wrinkle"],
            &["rotate", "reflect", "scale", "shear", "reshape", "freeTransform", "puppetWarp"],
            &["shapeBuilder", "livePaintBucket", "livePaintSelection", "blend"],
            &["dimension"],
        ],
    ),
    ("Type", &[&["areaType", "typeOnPath", "verticalAreaType", "verticalTypeOnPath"], &["type", "verticalType", "touchType"]]),
    ("Navigate", &[&["zoom"], &["hand", "printTiling"], &["rotateView"]]),
    ("Color", &[&["gradient", "mesh"], &["eyedropper", "measure"]]),
];

pub(crate) fn tip(t: &ToolInfo) -> String {
    match crate::shortcut_editor::tool_shortcut(t.id) {
        Some(s) => format!("{} ({})", tl!(t.label), s),
        None => tl!(t.label).to_string(),
    }
}

/// Slots of the current layout, as Edit Toolbar left it: (category label for the first slot of a
/// category, tool ids).
pub(crate) fn slots(app: &VectorcraftApp) -> Vec<(Option<&'static str>, Vec<&'static str>)> {
    if let Some(edited) = crate::edit_toolbar::edited_slots(app) {
        edited
    } else if app.ui.toolbar_advanced {
        TOOL_GROUPS.iter().map(|g| (None, g.iter().map(|t| t.id).collect())).collect()
    } else {
        BASIC.iter().flat_map(|(cat, ss)| ss.iter().enumerate().map(move |(i, s)| (if i == 0 { Some(*cat) } else { None }, s.to_vec()))).collect()
    }
}

/// Remember the tool shown in the slot that contains `id`.
pub fn remember(app: &mut VectorcraftApp, id: &str) {
    for (_, s) in slots(app) {
        if s.contains(&id) {
            app.ui.slot_tool.insert(s[0].to_string(), id.to_string());
        }
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The user's choice alone: a window too short for the tools scrolls them.
    let cols = if app.ui.toolbar_double { 2 } else { 1 };
    // Two 36 pt buttons with 2 pt either side inside the border and separator line (at 76 they
    // overran the right border).
    let w = if cols == 2 { 80.0 } else { WIDTH };
    // Where the panel docks: along the window's left edge, under the bars.
    let edge = ui.available_rect_before_wrap();
    ui.ctx().data_mut(|d| d.insert_temp(crate::tools_float::tools_zone_id(), egui::Rect::from_min_size(edge.min, vec2(WIDTH, edge.height()))));
    match app.ui.toolbar_pos {
        None => {
            let margin = egui::Margin { left: 6, right: 0, top: 6, bottom: 6 };
            egui::Panel::left("toolbar").resizable(false).exact_size(w + 6.0).frame(egui::Frame::NONE).show(ui, |ui| {
                crate::theme::card(ui, &t, margin, |ui| body(app, ui, cols, None));
            });
        }
        Some(pos) => {
            // Floating, kept inside the window, its tools scrolling when they don't fit under it.
            let ctx = ui.ctx().clone();
            let id = egui::Id::unique("toolbar-floating");
            let screen = ctx.content_rect();
            // Kept on screen by its title bar and first tools: the rest scrolls.
            let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(w, 160.0), |r| vec2(r.width(), r.height().min(160.0)));
            let at = crate::tools_float::clamp(pos, size, screen);
            egui::Area::new(id).order(egui::Order::Middle).fixed_pos(at).show(&ctx, |ui| {
                flyout_frame(&t, false).inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
                    ui.set_width(w);
                    ui.set_max_height(screen.bottom() - at.y - 12.0);
                    body(app, ui, cols, Some(at));
                });
            });
        }
    }
    // Before the flyout: one torn off this frame floats from the next (its bar mustn't be in two
    // layers in one frame).
    floating(app, ui.ctx());
    crate::dialogs::dimension::floating(app, ui.ctx());
    flyout(app, ui.ctx());
    crate::edit_toolbar::drawer(app, ui.ctx());
}

/// The Tools panel's contents, docked or floating at `floating`.
fn body(app: &mut VectorcraftApp, ui: &mut Ui, cols: usize, floating: Option<egui::Pos2>) {
    let t = Tokens::get(ui.ctx());
    let all = slots(app);
    let bounds = ui.max_rect();
    // The drawer opens beside it; a click in it leaves the drawer open.
    ui.data_mut(|d| d.insert_temp(crate::edit_toolbar::toolbar_id(), bounds));
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    // Title bar with » and a grip strip: a drag on either floats the panel or moves it.
    let (hdr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::click_and_drag());
    ui.painter().rect_filled(hdr, 0.0, t.tab_strip);
    icons::paint(
        ui,
        if cols == 2 { "chevrons-left" } else { "chevrons-right" },
        egui::Rect::from_min_size(hdr.left_top() + vec2(3.0, 2.0), vec2(10.0, 10.0)),
        if hresp.hovered() { t.text_strong } else { t.text },
    );
    let (grip, gresp) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::drag());
    for k in 0..6 {
        ui.painter().line_segment(
            [pos2(grip.center().x - 9.0, grip.top() + 1.5 + k as f32 * 0.6), pos2(grip.center().x + 9.0, grip.top() + 1.5 + k as f32 * 0.6)],
            Stroke::new(0.4, t.text_disabled),
        );
    }
    crate::tools_float::drag_tools(app, ui.ctx(), &hresp.union(gresp.clone()), bounds, floating);
    hresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Toggle single/double column")));
    if hresp.on_hover_text(tl!("Toggle single/double column")).clicked()
        // It only fails on bad params, which this never sends; show it all the same.
        && let Err(e) = app.run("window.toolbarColumns", json!({ "double": cols == 1 }))
    {
        app.ui.status = e;
    }
    let grip_tip = if floating.is_some() { tl!("Drag to the window's left edge to dock") } else { tl!("Drag to float the toolbar") };
    gresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), grip_tip));
    gresp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(grip_tip);
    // The tools and the controls under them scroll in a window too short for them.
    widgets::strip_scroll(ui, "toolbar", |ui| {
        let active = app.session.tool_id();
        // The tool button whose long press opened its flyout, until the next press.
        let held_id = egui::Id::unique("toolbar-held");
        if ui.input(|inp| inp.pointer.any_pressed()) {
            ui.data_mut(|d| d.remove::<egui::Id>(held_id));
        }
        let held: Option<egui::Id> = ui.data(|d| d.get_temp(held_id));
        // While Edit Toolbar's drawer is open, the tools drag (to new places or into the drawer)
        // instead of opening their flyouts.
        let editing = app.ui.tools_drawer;
        let mut buttons: Vec<(usize, egui::Rect)> = vec![];
        let mut open_flyout: Option<(Vec<&'static str>, egui::Rect)> = None;
        // A group torn off into a floating flyout is raised instead of opening its flyout.
        let mut raise: Option<&'static str> = None;
        let labels = app.session.prefs.tool_group_labels;
        let mut i = 0;
        while i < all.len() {
            if all[i].0.is_some() && !labels {
                // User Interface › Show Tool Group Labels off: the tools run on from group to group,
                // in one column or two (as the reference's Basic toolbar in two), with nothing
                // between the groups.
            } else if let Some(cat) = all[i].0 {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                // A long name is cut to its first four characters in the single column.
                let cat = tl!(cat);
                let label =
                    if cols == 1 && cat.chars().count() > 6 { format!("{}...", cat.chars().take(4).collect::<String>()) } else { cat.to_string() };
                ui.painter().text(r.center() + vec2(0.0, 2.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), t.text);
            }
            // One row = `cols` slots (a category label always starts a new row; without labels,
            // nothing does).
            let mut row = vec![i];
            while row.len() < cols && i + row.len() < all.len() && (all[i + row.len()].0.is_none() || !labels) {
                row.push(i + row.len());
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // Centred on the panel's inside (`bounds`: within its border and, docked, the
                // separator line egui reserves on its right), not on `WIDTH`, which sat them 2½ pt
                // right of centre docked; nor on this row, which the scroll area starts ½ pt in.
                ui.add_space((bounds.center().x - cols as f32 * 18.0 - ui.cursor().min.x).max(0.0));
                for &k in &row {
                    let slot = &all[k].1;
                    let shown_id = if slot.contains(&active) {
                        active.to_string()
                    } else {
                        app.ui.slot_tool.get(slot[0]).cloned().unwrap_or_else(|| slot[0].to_string())
                    };
                    let Some(shown) = tool_info(&shown_id).or_else(|| tool_info(slot[0])) else { continue };
                    let is_active = slot.contains(&active);
                    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, PITCH - 1.0), Sense::click_and_drag());
                    let well = egui::Rect::from_center_size(rect.center(), vec2(30.0, 27.5));
                    if is_active {
                        ui.painter().rect_filled(well, CornerRadius::same(6), t.tool_active);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(well, CornerRadius::same(6), t.hover);
                    }
                    let ir = egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
                    icons::paint(ui, icons::tool_icon(shown.icon), ir, if is_active { t.text_strong } else { t.icon });
                    if slot.len() > 1 {
                        let c = rect.center() + vec2(12.5, 10.0);
                        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-3.5, 0.0), c + vec2(0.0, -3.5)], t.icon, Stroke::NONE));
                    }
                    if editing {
                        buttons.push((k, rect));
                        resp.dnd_set_drag_payload(crate::edit_toolbar::ToolDrag { tool: shown.id, from_toolbar: true });
                    }
                    let press = if slot.len() > 1 && !editing { flyout_press(ui, &resp, rect) } else { None };
                    let alt = ui.input(|inp| inp.modifiers.alt);
                    if press.is_some() {
                        if is_floating(app, slot[0]) {
                            raise = Some(slot[0]);
                        } else {
                            open_flyout = Some((slot.clone(), rect));
                        }
                        if press == Some(FlyoutPress::Primary) {
                            ui.data_mut(|d| d.insert_temp(held_id, resp.id));
                        }
                    } else if resp.clicked() && held == Some(resp.id) {
                        // Releasing the long press that opened the flyout leaves it open.
                    } else if resp.clicked() && alt && slot.len() > 1 {
                        let idx = slot.iter().position(|x| *x == shown.id).unwrap_or(0);
                        app.select_tool(slot[(idx + 1) % slot.len()]);
                    } else if resp.double_clicked() {
                        app.select_tool(shown.id);
                        app.run("tool.options", json!({ "tool": shown.id })).ok();
                    } else if resp.clicked() {
                        app.select_tool(shown.id);
                    }
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), is_active, tip(shown)));
                    resp.on_hover_text(tip(shown));
                }
            });
            i += row.len();
        }
        if editing {
            crate::edit_toolbar::toolbar_drop(app, ui, &buttons, cols, bounds);
        }
        if let Some(key) = raise {
            raise_floating(ui.ctx(), key);
        }
        if let Some((slot, rect)) = open_flyout {
            app.ui.flyout = Some(0);
            ui.data_mut(|d| {
                d.insert_temp(egui::Id::unique("flyout-anchor"), rect);
                d.insert_temp(egui::Id::unique("flyout-tools"), slot.iter().map(|s| s.to_string()).collect::<Vec<String>>());
            });
        }
        ui.add_space(8.0);
        bottom_controls(app, ui, &t, cols);
    });
}

/// How a press on a tool group's button opens its flyout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FlyoutPress {
    /// A right press (it opens at once, before the release).
    Secondary,
    /// A primary press on the corner triangle, or one held [`LONG_PRESS`] seconds: its release
    /// mustn't pick the button's tool.
    Primary,
}

/// Whether this frame's pointer state on a tool button opens its group's flyout. A primary press
/// still short of a long one asks for the frame where it becomes long: egui draws only on input,
/// so a still mouse would otherwise never get there.
fn flyout_press(ui: &Ui, resp: &egui::Response, rect: egui::Rect) -> Option<FlyoutPress> {
    if resp.hovered() && ui.input(|inp| inp.pointer.button_pressed(egui::PointerButton::Secondary)) {
        return Some(FlyoutPress::Secondary);
    }
    if !resp.is_pointer_button_down_on() || !ui.input(|inp| inp.pointer.primary_down()) {
        return None;
    }
    // The triangle's corner: the bottom-right third of the button.
    let corner = egui::Rect::from_min_max(rect.center() + vec2(6.0, 4.0), rect.max);
    let (origin, held) = ui.input(|inp| (inp.pointer.press_origin(), inp.pointer.press_start_time().map(|s| inp.time - s)));
    if origin.is_some_and(|p| corner.contains(p)) {
        return Some(FlyoutPress::Primary);
    }
    let held = held.unwrap_or(0.0);
    if held >= LONG_PRESS {
        return Some(FlyoutPress::Primary);
    }
    ui.ctx().request_repaint_after_secs((LONG_PRESS - held) as f32);
    None
}

/// Open a tool's options (`tool.options`, a double-click on its button): the Gradient tool's are
/// the Gradient panel, the Eyedropper's the Eyedropper Options dialog, a Liquify tool's its Tool
/// Options dialog, the Blend tool's Blend
/// Options; the Print Tiling tool's resets the print tiling. As in the reference app, the Hand
/// tool's fits the artboard in the window, the Zoom tool's shows it at 100%, the Rotate, Scale,
/// Reflect and Shear tools' are their Object › Transform dialogs, the selection tools' are the
/// Move dialog, and the Pencil, Paintbrush, Smooth, Blob Brush, Eraser and Shape Builder tools' are
/// their Tool Options, as are the Line Segment, Arc, Rectangular Grid and Polar Grid tools'
/// ([`crate::dialogs::shapes::open_options`]). The Type on a Path and Vertical Type on a Path
/// tools' are Type on a Path Options for the selected type on a path.
pub fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<serde_json::Value, String> {
    match tool {
        "dimension" => crate::dialogs::dimension::open(app),
        "hand" => app.run("view.fitArtboard", json!({})),
        "zoom" => app.run("view.actualSize", json!({})),
        "rotate" | "scale" | "reflect" | "shear" | "selection" | "directSelection" | "groupSelection" => {
            let dialog = if vectorcraft_tools::catalog::is_selection_tool(tool) { "move" } else { tool };
            let id = format!("object.{dialog}");
            if let Some(c) = vectorcraft_engine::find_command(&id) {
                (c.enabled)(&app.session)?;
            }
            crate::menus::invoke(app, &id, json!({}));
            Ok(json!({ "dialog": dialog }))
        }
        // The Gradient and Magic Wand tools: their panels.
        "gradient" | "magicWand" if crate::panel_docking::showing(&app.ui, tool) => Ok(json!({ "open": tool })),
        "gradient" | "magicWand" => app.run("window.panel", json!({ "panel": tool })),
        "artboard" => crate::dialogs::artboard_options::open(app),
        "eyedropper" => {
            crate::dialogs::eyedropper::open(app);
            Ok(json!({ "dialog": crate::dialogs::eyedropper::KIND }))
        }
        "blend" => {
            crate::dialogs::blend_options::open(app)?;
            Ok(json!({ "dialog": crate::dialogs::blend_options::KIND }))
        }
        "perspectiveGrid" => {
            crate::dialogs::perspective_options::open(app);
            Ok(json!({ "dialog": crate::dialogs::perspective_options::KIND }))
        }
        "shapeBuilder" => Ok(crate::dialogs::shape_builder_options::open(app)),
        // The Line Segment, Arc and grid tools: their Tool Options, the options of their next
        // drag and click.
        _ if crate::dialogs::shapes::OPTION_TOOLS.contains(&tool) => crate::dialogs::shapes::open_options(app, tool),
        // The Flare tool: its options, which draw the next flare (OK draws none).
        "flare" => Ok(crate::dialogs::flare_options::open(app, None)),
        // A double click on the Print Tiling tool puts the pages back where the placement puts them.
        "printTiling" => app.run("print.tiling.set", json!({ "reset": true })),
        // The Liquify tools: their Tool Options (the Global Brush Dimensions and the tool's own).
        _ if vectorcraft_tools::settings::LIQUIFY.contains(&tool) => crate::dialogs::liquify::open(app, tool),
        // The freehand tools: their Tool Options (Fidelity, fill, the tolerances, the brush size).
        _ if crate::dialogs::freehand::TOOLS.contains(&tool) => crate::dialogs::freehand::open(app, tool),
        // The Live Paint Selection tool: its Tool Options (what a click selects, the highlight).
        "livePaintSelection" => crate::dialogs::live_paint_selection::open(app),
        // The Symbolism tools: the brush they share.
        _ if vectorcraft_tools::settings::SYMBOLISM.contains(&tool) => Ok(crate::dialogs::symbolism_options::open(app, tool)),
        // The graph tools: Graph Type for the selected graph.
        _ if vectorcraft_tools::extra::is_graph_tool(tool) => crate::menus::graph_dialog(app, "graph.setType"),
        // The Type on a Path tools: Type on a Path Options for the selected type on a path, as
        // Type › Type on a Path › Type on a Path Options… opens it.
        "typeOnPath" | "verticalTypeOnPath" => {
            let id = "type.pathOptions";
            if let Some(c) = vectorcraft_engine::find_command(id) {
                (c.enabled)(&app.session)?;
            }
            app.session.execute(id, &json!({})).map_err(|e| e.to_string())?;
            crate::menus::invoke(app, id, json!({}));
            Ok(json!({ "dialog": "command" }))
        }
        // The add-ons' tools.
        _ if crate::addons::pathtools::has_options(tool) => crate::addons::pathtools::open_options(app, tool),
        _ if vectorcraft_tools::tool_info(tool).is_none() => Err(format!("unknown tool `{tool}`")),
        _ => Err(format!("the {tool} tool has no options")),
    }
}

/// The active tool's options in the Control bar: Mirror & Cut's axis and the side it keeps, Puppet
/// Warp's mesh and pins (set through `tool.setOption`). The Artboard tool's options are drawn by
/// [`crate::panels::artboards::control_bar`].
pub fn control_bar_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.tool_id() == "dimension" {
        return crate::dialogs::dimension::controls(app, ui);
    }
    /// (value, label) of each choice.
    type Choices = &'static [(&'static str, &'static str)];
    const MIRROR: [(&str, &str, Choices); 2] = [
        ("axis", "Axis:", &[("free", "Free"), ("vertical", "Vertical"), ("horizontal", "Horizontal")]),
        ("keep", "Keep:", &[("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom")]),
    ];
    if app.session.tool_id() == "puppetWarp" {
        return puppet_warp_options(app, ui);
    }
    if app.session.tool_id() == vectorcraft_tools::cropimage::ID {
        return crop_options(app, ui);
    }
    if app.session.tool_id() != "mirrorCut" {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let opts = app.session.tool_options();
    for (key, label, choices) in MIRROR {
        ui.label(egui::RichText::new(tl!(label)).size(12.0).color(t.text));
        let cur = opts[key].as_str().unwrap_or_default();
        let shown = choices.iter().find(|(v, _)| *v == cur).map_or(cur, |(_, l)| *l);
        let labels: Vec<&str> = choices.iter().map(|(_, l)| *l).collect();
        if let Some((value, _)) = widgets::dropdown(ui, ("cb-tool", key), shown, &labels, 96.0).and_then(|i| choices.get(i)) {
            app.run("tool.setOption", json!({ "key": key, "value": value })).ok();
        }
    }
    ui.separator();
}

/// Crop Image's box: its centre (X, Y) and size (W, H) in the general unit, set through the tool's
/// `rect` option, then Apply (Enter) and Cancel (Escape).
fn crop_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let set = app.session.tool_options()["rect"].as_array().and_then(|v| match v.iter().filter_map(Value::as_f64).collect::<Vec<_>>()[..] {
        [x, y, w, h] => Some(vectorcraft_geom::Rect::new(x, y, x + w, y + h)),
        _ => None,
    });
    let Some((_, _, r)) = app.session.active().and_then(|st| vectorcraft_tools::cropimage::crop_box(&st.doc, &st.selection, set)) else { return };
    let t = Tokens::get(ui.ctx());
    let units = app.session.general_unit();
    let c = r.center();
    for (k, lbl, v) in [("x", "X:", c.x), ("y", "Y:", c.y), ("width", "W:", r.width()), ("height", "H:", r.height())] {
        widgets::field_label(ui, egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim));
        if let Some(nv) = widgets::num_field(ui, ("cb-crop", k), Some(v), units, 80.0) {
            let (mut c, mut w, mut h) = (c, r.width(), r.height());
            match k {
                "x" => c.x = nv,
                "y" => c.y = nv,
                "width" => w = nv.max(0.0),
                _ => h = nv.max(0.0),
            }
            let rect = [c.x - w / 2.0, c.y - h / 2.0, w, h];
            app.run("tool.setOption", json!({ "key": "rect", "value": rect })).ok();
        }
    }
    ui.separator();
    for (label, key) in [(tl!("Apply"), ToolKey::Enter), (tl!("Cancel"), ToolKey::Escape)] {
        if widgets::flat_button(ui, label, 64.0).clicked() {
            let view = app.view_info();
            let r = app.session.tool_key(key, Mods::default(), view);
            crate::canvas::apply_requests(app, r);
        }
    }
    ui.separator();
}

/// The Fill/Stroke proxy, Color/Gradient/None, the drawing modes, screen mode and Edit Toolbar, as
/// large as the toolbar's `cols` (one or two) leave room for (#1129): in two columns the proxy and
/// buttons grow and the three drawing modes show side by side.
/// The All Tools drawer's Show section hides any of them but Edit Toolbar.
fn bottom_controls(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens, cols: usize) {
    let two = cols == 2;
    let show = app.ui.toolbar_show;
    if show.fill_stroke {
        ui.vertical_centered(|ui| crate::panels::proxy(app, ui, if two { 54.0 } else { 36.0 }));
        ui.add_space(5.0);
    }
    // Color (the last solid colour), Gradient (the last gradient) and None, as commands.
    let (chip, gap) = if two { (17.0, 4.0) } else { (11.0, 2.0) };
    if show.coloring {
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 3.0 * chip - 2.0 * gap) / 2.0);
            ui.spacing_mut().item_spacing.x = gap;
            let mut clicked = None;
            for (tip, cmd) in [("Color (, or <)", "paint.lastColor"), ("Gradient (. or >)", "paint.lastGradient"), ("None (/)", "paint.none")] {
                let (r, resp) = ui.allocate_exact_size(vec2(chip, chip), Sense::click());
                match cmd {
                    "paint.lastColor" => widgets::paint_chip(ui, r, &Paint::solid(app.session.last_solid)),
                    "paint.lastGradient" => widgets::gradient_chip(ui, r, &app.session.last_gradient),
                    _ => widgets::paint_chip(ui, r, &Paint::None),
                }
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(tip)));
                if resp.on_hover_text(tl!(tip)).clicked() {
                    clicked = Some(cmd);
                }
            }
            // Cmd/Ctrl-click on Gradient resets the gradient to White, Black.
            if clicked == Some("paint.lastGradient") && ui.input(|i| i.modifiers.command) {
                crate::panels::gradient::reset_to_linear_white_black(app);
            } else if let Some(cmd) = clicked {
                app.run(cmd, json!({})).ok();
            }
        });
        ui.add_space(6.0);
    }
    let modes = [
        (vectorcraft_engine::DrawMode::Normal, "dc-draw-normal", "Draw Normal", "normal"),
        (vectorcraft_engine::DrawMode::Behind, "dc-draw-behind", "Draw Behind", "behind"),
        (vectorcraft_engine::DrawMode::Inside, "dc-draw-inside", "Draw Inside", "inside"),
    ];
    if two && show.draw_mode {
        // Side by side, each one a click away.
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 3.0 * 22.0) / 2.0);
            for (mode, icon, name, id) in modes {
                if widgets::icon_button(ui, icon, &format!("{} (Shift+D)", tl!(name)), app.session.draw_mode == mode, 22.0).clicked() {
                    app.run("view.drawMode", json!({ "mode": id })).ok();
                }
            }
        });
    }
    ui.vertical_centered(|ui| {
        // One column: the current mode's button, which cycles them.
        if !two
            && show.draw_mode
            && let Some((mode, icon, name, _)) = modes.into_iter().find(|m| m.0 == app.session.draw_mode)
            && widgets::icon_button(ui, icon, &format!("{} (Shift+D)", tl!(name)), mode != vectorcraft_engine::DrawMode::Normal, 26.0).clicked()
        {
            app.run("view.drawMode", json!({})).ok();
        }
        if show.screen_mode && widgets::icon_button(ui, "dc-screen-mode", "Change Screen Mode (F)", false, 26.0).clicked() {
            app.run("view.screenMode", json!({})).ok();
        }
        // Edit Toolbar: the All Tools drawer opens beside the toolbar, by this button.
        let edit = widgets::icon_button(ui, "ellipsis", "Edit Toolbar", app.ui.tools_drawer, 26.0);
        ui.data_mut(|d| d.insert_temp(crate::edit_toolbar::button_id(), edit.rect));
        if edit.clicked() {
            app.run("window.editToolbar", json!({})).ok();
        }
    });
    let _ = t;
}

/// Where `window.floatTools` puts a strip (the next one a row lower), right of the toolbar; it's
/// kept on screen and moved by its bar.
const FLOAT_AT: [f32; 2] = [90.0, 100.0];

/// Float the toolbar group (of the current layout) holding `tool` as a strip of tool buttons, put
/// it back in the toolbar, or toggle (`floating` omitted), as a flyout's tear-off bar and the
/// strip's × do (`window.floatTools`). Returns the new state.
pub fn float_group(app: &mut VectorcraftApp, tool: &str, floating: Option<bool>) -> Result<bool, String> {
    let Some((_, group)) = slots(app).into_iter().find(|(_, s)| s.len() > 1 && s.contains(&tool)) else {
        return Err(format!("{tool} isn't in a toolbar group of several tools"));
    };
    let key = group.first().copied().unwrap_or_default();
    let was = is_floating(app, key);
    let on = floating.unwrap_or(!was);
    if on && !was {
        let n = app.ui.floating_flyouts.len() as f32;
        let tools = group.iter().map(|id| id.to_string()).collect();
        app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [FLOAT_AT[0], FLOAT_AT[1] + n * (PITCH + 12.0)] });
        app.ui.flyout = None;
    } else if !on {
        app.ui.floating_flyouts.retain(|f| f.tools.first().is_none_or(|k| k != key));
    }
    Ok(on)
}

/// Whether the group whose first tool is `key` floats as its own panel.
fn is_floating(app: &VectorcraftApp, key: &str) -> bool {
    app.ui.floating_flyouts.iter().any(|f| f.tools.first().is_some_and(|k| k == key))
}

/// The area of the floating flyout of the group whose first tool is `key`.
fn floating_area(key: &str) -> egui::Id {
    egui::Id::unique(("tool-floating", key))
}

/// Bring a floating flyout to the front and flash its border.
fn raise_floating(ctx: &egui::Context, key: &str) {
    let id = floating_area(key);
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(id, now));
}

/// A flyout's frame: drop shadow and border (the accent while it flashes).
pub(crate) fn flyout_frame(t: &Tokens, flash: bool) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.panel)
        .stroke(if flash { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, t.input_border) })
        .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(90) })
}

/// What a press on a flyout did.
#[derive(Default)]
struct FlyoutInput {
    /// The tool row clicked.
    chosen: Option<&'static str>,
    /// The grab bar's response (its drag tears off or moves the flyout).
    bar: Option<egui::Response>,
    /// A floating flyout's × was clicked.
    close: bool,
}

/// A flyout's tool rows (a floating flyout's strip of tool buttons) with the grab bar down their
/// right side; a floating flyout's bar has a × at its top. The bar's id is the group's, so a drag
/// that tears a flyout off carries on moving the floating flyout it becomes.
fn flyout_body(ui: &mut Ui, t: &Tokens, active: &str, tools: &[String], floating: bool) -> FlyoutInput {
    let mut out = FlyoutInput::default();
    let key = tools.first().map_or("", String::as_str);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.horizontal(|ui| {
        let rows = if floating { ui.horizontal(|ui| tool_strip(ui, t, active, tools)) } else { ui.vertical(|ui| tool_rows(ui, t, active, tools)) };
        out.chosen = rows.inner;
        let (bar, _) = ui.allocate_exact_size(vec2(BAR, rows.response.rect.height()), Sense::hover());
        ui.painter().rect_filled(bar, 0.0, t.tab_strip);
        let grip = if floating {
            let x = egui::Rect::from_min_size(bar.min, vec2(BAR, CLOSE));
            let xr = ui.interact(x, egui::Id::unique(("tool-flyout-close", key)), Sense::click());
            icons::paint(ui, "x", x.shrink2(vec2(1.0, 3.0)), if xr.hovered() { t.text_strong } else { t.text });
            xr.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Put back in the toolbar")));
            out.close = xr.on_hover_text(tl!("Put back in the toolbar")).clicked();
            egui::Rect::from_min_max(bar.min + vec2(0.0, CLOSE), bar.max)
        } else {
            bar
        };
        let resp = ui.interact(grip, bar_id(key), Sense::drag());
        let half = (grip.height() / 2.0 - 3.0).clamp(0.0, 9.0);
        for k in 0..3 {
            let x = grip.center().x - 2.0 + k as f32 * 2.0;
            ui.painter().line_segment([pos2(x, grip.center().y - half), pos2(x, grip.center().y + half)], Stroke::new(0.6, t.text_disabled));
        }
        let tip = if floating { tl!("Drag to move") } else { tl!("Drag to float as a toolbar") };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
        let resp = if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            resp
        } else {
            resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tip)
        };
        out.bar = Some(resp);
    });
    out
}

/// One row per tool: icon, name and shortcut, with a dot by the active tool. Returns the tool clicked.
fn tool_rows(ui: &mut Ui, t: &Tokens, active: &str, tools: &[String]) -> Option<&'static str> {
    let mut chosen = None;
    let lang = crate::i18n::current();
    let color = |tool: &ToolInfo| if active == tool.id { t.flyout_active } else { t.text_strong };
    let rows: Vec<(&ToolInfo, RowText)> =
        tools.iter().filter_map(|id| tool_info(id)).map(|tool| (tool, row_text(ui, lang, tool, color(tool)))).collect();
    let w = rows_width(rows.iter().map(|(_, text)| text));
    for (tool, (name, shortcut)) in rows {
        let is_active = active == tool.id;
        let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover);
        }
        if is_active {
            ui.painter().rect_filled(egui::Rect::from_center_size(r.left_center() + vec2(12.0, 0.0), vec2(5.0, 5.0)), 0.0, t.text);
        }
        let ir = egui::Rect::from_min_size(r.min + vec2(24.0, 6.0), vec2(18.0, 18.0));
        icons::paint(ui, icons::tool_icon(tool.icon), ir, t.icon);
        let color = color(tool);
        ui.painter().galley(pos2(r.left() + NAME_X, r.center().y - name.size().y / 2.0), name, color);
        if let Some(sc) = shortcut {
            ui.painter().galley(pos2(r.right() - ROW_END - sc.size().x, r.center().y - sc.size().y / 2.0), sc, color);
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), is_active, tip(tool)));
        if resp.clicked() {
            chosen = Some(tool.id);
        }
    }
    chosen
}

/// A flyout row's name and its shortcut, laid out.
type RowText = (Arc<egui::Galley>, Option<Arc<egui::Galley>>);

/// Where a flyout row's name starts, the least gap between it and the shortcut, and the room right
/// of the shortcut.
const NAME_X: f32 = 52.0;
const NAME_GAP: f32 = 14.0;
const ROW_END: f32 = 18.0;

/// The name of `tool` in `lang` and its shortcut in brackets, as a flyout row draws them.
fn row_text(ui: &Ui, lang: crate::i18n::Lang, tool: &ToolInfo, color: Color32) -> RowText {
    let lay = |s: String| ui.painter().layout_no_wrap(s, egui::FontId::proportional(13.0), color);
    (lay(crate::i18n::tr(lang, tool.label).to_string()), crate::shortcut_editor::tool_shortcut(tool.id).map(|sc| lay(format!("({sc})"))))
}

/// The width of a flyout's rows: 250 pt, or what its longest name needs to stay clear of its
/// shortcut (translated names are far longer than the English ones, #1616).
fn rows_width<'a>(rows: impl Iterator<Item = &'a RowText>) -> f32 {
    rows.map(|(name, sc)| NAME_X + name.size().x + sc.as_ref().map_or(0.0, |sc| NAME_GAP + sc.size().x) + ROW_END).fold(250.0, f32::max)
}

/// The grab bar of the group whose first tool is `key` (one id, flyout or floating, so a drag
/// that tears the flyout off carries on moving it).
fn bar_id(key: &str) -> egui::Id {
    egui::Id::unique(("tool-flyout-bar", key))
}

/// Where the pointer holds a floating flyout's bar, from the flyout's corner, while it's dragged.
fn grab_id(key: &str) -> egui::Id {
    bar_id(key).with("grab")
}

/// One icon button per tool, the active one lit, with its name and shortcut on hover. Returns the
/// tool clicked.
fn tool_strip(ui: &mut Ui, t: &Tokens, active: &str, tools: &[String]) -> Option<&'static str> {
    let mut chosen = None;
    for tool in tools.iter().filter_map(|id| tool_info(id)) {
        let (r, resp) = ui.allocate_exact_size(STRIP_BUTTON, Sense::click());
        let well = r.shrink(1.0);
        if active == tool.id {
            ui.painter().rect_filled(well, CornerRadius::same(6), t.tool_active);
        } else if resp.hovered() {
            ui.painter().rect_filled(well, CornerRadius::same(6), t.hover);
        }
        let ir = egui::Rect::from_center_size(r.center(), vec2(18.0, 18.0));
        icons::paint(ui, icons::tool_icon(tool.icon), ir, if active == tool.id { t.text_strong } else { t.icon });
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active == tool.id, tip(tool)));
        if resp.on_hover_text(tip(tool)).clicked() {
            chosen = Some(tool.id);
        }
    }
    chosen
}

/// The width of a floating flyout's strip of `tools`, its bar included.
fn strip_width(tools: &[String]) -> f32 {
    tools.iter().filter(|id| tool_info(id).is_some()).count() as f32 * STRIP_BUTTON.x + BAR
}

/// How far the pointer has moved since the press that's down, if one is.
fn press_travel(ctx: &egui::Context) -> Option<egui::Vec2> {
    ctx.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos()).map(|(a, b)| b - a))
}

fn flyout(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.ui.flyout.is_none() {
        return;
    }
    let tools: Vec<String> = ctx.data(|d| d.get_temp(egui::Id::unique("flyout-tools"))).unwrap_or_default();
    if tools.is_empty() {
        app.ui.flyout = None;
        return;
    }
    let anchor: egui::Rect =
        ctx.data(|d| d.get_temp(egui::Id::unique("flyout-anchor"))).unwrap_or(egui::Rect::from_min_size(pos2(40.0, 100.0), vec2(32.0, 32.0)));
    let t = Tokens::get(ctx);
    let active = app.session.tool_id();
    let mut input = FlyoutInput::default();
    let resp = egui::Area::new(egui::Id::unique("tool-flyout")).order(egui::Order::Foreground).fixed_pos(anchor.right_top() + vec2(8.0, -1.0)).show(
        ctx,
        |ui| {
            flyout_frame(&t, false).show(ui, |ui| input = flyout_body(ui, &t, active, &tools, false));
        },
    );
    // Dragging the bar tears the flyout off, and so does releasing a press over it (a click, or the
    // end of the long press that opened the flyout): it floats where the pointer is.
    let at = ctx.input(|i| i.pointer.interact_pos());
    let released_on_bar = input.bar.as_ref().is_some_and(|b| ctx.input(|i| i.pointer.primary_released()) && at.is_some_and(|p| b.rect.contains(p)));
    let dragged_off = input.bar.as_ref().is_some_and(egui::Response::dragged) && press_travel(ctx).is_some_and(|d| d.length() > TEAR);
    if released_on_bar || dragged_off {
        // The strip is far narrower than the flyout: it floats with its bar under the pointer.
        let at = at.unwrap_or(resp.response.rect.right_center());
        // (Past the frame's 1 pt border.)
        let grab = vec2(1.0 + strip_width(&tools) - BAR / 2.0, 1.0 + CLOSE + 4.0);
        let p = at - grab;
        if let Some(key) = tools.first() {
            ctx.data_mut(|m| m.insert_temp(grab_id(key), grab));
        }
        app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [p.x, p.y] });
        app.ui.flyout = None;
        return;
    }
    if let Some(id) = input.chosen {
        app.select_tool(id);
    } else if resp.response.clicked_elsewhere() && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| anchor.contains(p))) {
        // A click on the flyout's own tool button (the right-click that opened it, or the end of a
        // long press) leaves it to that button.
        app.ui.flyout = None;
    }
    let _ = theme::semibold;
}

/// The flyouts torn off the toolbar, each floating where its bar was dragged; its × puts it back.
fn floating(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.ui.floating_flyouts.is_empty() {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let now = ctx.input(|i| i.time);
    let active = app.session.tool_id();
    let mut chosen = None;
    let mut closed = None;
    for (k, f) in app.ui.floating_flyouts.iter_mut().enumerate() {
        let Some(key) = f.tools.first() else { continue };
        let id = floating_area(key);
        let since = ctx.data(|d| d.get_temp::<f64>(id)).map(|t0| now - t0);
        let flash = since.is_some_and(|s| (0.0..FLASH).contains(&s));
        if let Some(s) = since.filter(|_| flash) {
            ctx.request_repaint_after_secs((FLASH - s) as f32);
        }
        // Kept on screen (a smaller window, or a position saved on a bigger one).
        let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(strip_width(&f.tools), PITCH), |r| r.size());
        let pos = crate::tools_float::clamp(f.pos, size, screen);
        let mut input = FlyoutInput::default();
        egui::Area::new(id).order(egui::Order::Middle).fixed_pos(pos).show(ctx, |ui| {
            flyout_frame(&t, flash).show(ui, |ui| input = flyout_body(ui, &t, active, &f.tools, true));
        });
        // Dragging the bar keeps the flyout where the pointer holds it (by an offset rather than
        // each frame's motion, which a just torn-off flyout's first, unseen frame would drop).
        let grab = grab_id(key);
        if ctx.is_being_dragged(bar_id(key))
            && let Some(at) = ctx.input(|i| i.pointer.interact_pos())
        {
            let off = ctx.data_mut(|m| *m.get_temp_mut_or_insert_with(grab, || at - pos));
            let p = at - off;
            if p.x.is_finite() && p.y.is_finite() {
                f.pos = [p.x, p.y];
            }
        } else if !ctx.input(|i| i.pointer.any_down()) {
            ctx.data_mut(|m| m.remove::<egui::Vec2>(grab));
        }
        if input.close {
            closed = Some(k);
        }
        chosen = chosen.or(input.chosen);
    }
    if let Some(k) = closed.filter(|k| *k < app.ui.floating_flyouts.len()) {
        app.ui.floating_flyouts.remove(k);
    }
    if let Some(id) = chosen {
        app.select_tool(id);
    }
}

/// The Puppet Warp tool's Control bar: Expand (how far the mesh reaches past the art), Show Mesh
/// and Select All Pins.
fn puppet_warp_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let opts = app.session.tool_options();
    ui.label(egui::RichText::new(tl!("Expand:")).size(12.0).color(t.text));
    let unit = app.session.general_unit();
    if let Some(v) = widgets::num_field(ui, ("cb-tool", "expand"), opts["expand"].as_f64(), unit, 64.0) {
        app.run("tool.setOption", json!({ "key": "expand", "value": v })).ok();
    }
    let show = opts["showMesh"].as_bool().unwrap_or(true);
    if widgets::check(ui, "Show Mesh", show, true) {
        app.run("tool.setOption", json!({ "key": "showMesh", "value": !show })).ok();
    }
    if widgets::flat_button(ui, "Select All Pins", 104.0).clicked() {
        app.run("tool.setOption", json!({ "key": "selectAllPins", "value": true })).ok();
    }
    ui.separator();
}

#[cfg(test)]
pub(crate) mod tests;
