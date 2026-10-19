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
    ui.ctx().data_mut(|d| d.insert_temp(crate::floating::tools_zone_id(), egui::Rect::from_min_size(edge.min, vec2(WIDTH, edge.height()))));
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
            let id = egui::Id::new("toolbar-floating");
            let screen = ctx.content_rect();
            // Kept on screen by its title bar and first tools: the rest scrolls.
            let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(w, 160.0), |r| vec2(r.width(), r.height().min(160.0)));
            let at = crate::floating::clamp(pos, size, screen);
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
    crate::floating::drag_tools(app, ui.ctx(), &hresp.union(gresp.clone()), bounds, floating);
    hresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Toggle single/double column")));
    if hresp.on_hover_text(tl!("Toggle single/double column")).clicked()
        // It only fails on bad params, which this never sends; show it all the same.
        && let Err(e) = app.run("window.toolbarColumns", json!({ "double": cols == 1 }))
    {
        app.ui.status = e;
    }
    let grip_tip = if floating.is_some() { tl!("Drag to the window's left edge to dock") } else { tl!("Drag to float the toolbar") };
    gresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), grip_tip));
    gresp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(grip_tip);
    // The tools and the controls under them scroll in a window too short for them.
    widgets::strip_scroll(ui, "toolbar", |ui| {
        let active = app.session.tool_id();
        // The tool button whose long press opened its flyout, until the next press.
        let held_id = egui::Id::new("toolbar-held");
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
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), is_active, tip(shown)));
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
                d.insert_temp(egui::Id::new("flyout-anchor"), rect);
                d.insert_temp(egui::Id::new("flyout-tools"), slot.iter().map(|s| s.to_string()).collect::<Vec<String>>());
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
/// Move dialog, and the Pencil, Paintbrush, Smooth, Blob Brush and Eraser tools' are their Tool
/// Options.
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
        "gradient" | "magicWand" if app.ui.open_panel.as_deref() == Some(tool) => Ok(json!({ "open": tool })),
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
            for (tip, cmd) in [("Color (,)", "paint.lastColor"), ("Gradient (.)", "paint.lastGradient"), ("None (/)", "paint.none")] {
                let (r, resp) = ui.allocate_exact_size(vec2(chip, chip), Sense::click());
                match cmd {
                    "paint.lastColor" => widgets::paint_chip(ui, r, &Paint::solid(app.session.last_solid)),
                    "paint.lastGradient" => widgets::gradient_chip(ui, r, &app.session.last_gradient),
                    _ => widgets::paint_chip(ui, r, &Paint::None),
                }
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(tip)));
                if resp.on_hover_text(tl!(tip)).clicked() {
                    clicked = Some(cmd);
                }
            }
            if let Some(cmd) = clicked {
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
    egui::Id::new(("tool-floating", key))
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
            let xr = ui.interact(x, egui::Id::new(("tool-flyout-close", key)), Sense::click());
            icons::paint(ui, "x", x.shrink2(vec2(1.0, 3.0)), if xr.hovered() { t.text_strong } else { t.text });
            xr.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Put back in the toolbar")));
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
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
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
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), is_active, tip(tool)));
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
    egui::Id::new(("tool-flyout-bar", key))
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
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), active == tool.id, tip(tool)));
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
    let tools: Vec<String> = ctx.data(|d| d.get_temp(egui::Id::new("flyout-tools"))).unwrap_or_default();
    if tools.is_empty() {
        app.ui.flyout = None;
        return;
    }
    let anchor: egui::Rect =
        ctx.data(|d| d.get_temp(egui::Id::new("flyout-anchor"))).unwrap_or(egui::Rect::from_min_size(pos2(40.0, 100.0), vec2(32.0, 32.0)));
    let t = Tokens::get(ctx);
    let active = app.session.tool_id();
    let mut input = FlyoutInput::default();
    let resp = egui::Area::new(egui::Id::new("tool-flyout")).order(egui::Order::Foreground).fixed_pos(anchor.right_top() + vec2(8.0, -1.0)).show(
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
        let pos = crate::floating::clamp(f.pos, size, screen);
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
pub(crate) mod tests {
    use egui::accesskit::{Role, Toggled};
    use egui::{Event, PointerButton, Pos2};
    use vectorcraft_engine::Session;

    use super::*;
    use crate::widgets::accessibility_tests::{role_and_state, tree};

    /// One headless frame of the toolbar; returns the tool buttons' rects, top to bottom.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<egui::Rect> {
        frame_in(app, ctx, time, events, 1200.0)
    }

    /// [`frame`] in a window `height` points tall.
    fn frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>, height: f32) -> Vec<egui::Rect> {
        let input = egui::RawInput {
            time: Some(time),
            events,
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, height))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
        widget_rects(ctx, vec2(36.0, PITCH - 1.0))
    }

    /// The rects of the last frame's widgets of `size`, top to bottom.
    pub(crate) fn widget_rects(ctx: &egui::Context, size: egui::Vec2) -> Vec<egui::Rect> {
        let mut r: Vec<egui::Rect> =
            ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size).map(|w| w.rect).collect());
        r.sort_by(|a, b| a.top().total_cmp(&b.top()));
        r
    }

    /// A mouse wheel turn over `at`, scrolling the content up by `dy` points.
    pub(crate) fn wheel(at: Pos2, dy: f32) -> Vec<Event> {
        let unit = egui::MouseWheelUnit::Point;
        vec![Event::PointerMoved(at), Event::MouseWheel { unit, delta: vec2(0.0, -dy), phase: egui::TouchPhase::Move, modifiers: Default::default() }]
    }

    #[test]
    fn the_toolbar_scrolls_in_a_short_window() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        // 500 pt: even two columns don't fit, so Edit Toolbar (the last button) starts below the window.
        let tools = frame_in(&mut app, &ctx, 0.0, vec![], 500.0);
        let edit_toolbar = || widget_rects(&ctx, vec2(26.0, 26.0)).last().copied().unwrap();
        assert!(edit_toolbar().bottom() > 500.0);
        frame_in(&mut app, &ctx, 0.1, wheel(tools[0].center(), 2000.0), 500.0);
        for k in 2..40 {
            frame_in(&mut app, &ctx, f64::from(k) * 0.1, vec![], 500.0);
        }
        assert!(edit_toolbar().bottom() <= 500.0, "scrolled into view: {:?}", edit_toolbar());
        // Back up to the top.
        frame_in(&mut app, &ctx, 4.0, wheel(tools[0].center(), -2000.0), 500.0);
        for k in 41..80 {
            frame_in(&mut app, &ctx, f64::from(k) * 0.1, vec![], 500.0);
        }
        assert_eq!(frame_in(&mut app, &ctx, 8.0, vec![], 500.0)[0], tools[0]);
    }

    /// The tool buttons sit in the middle of the panel, between its border and the separator
    /// line, docked and floating, in one or two columns (docked, they sat 2½ pt right of centre).
    #[test]
    fn the_tool_buttons_are_centred_in_the_panel() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        for (double, floating) in [(false, None), (true, None), (false, Some([100.0, 100.0])), (true, Some([100.0, 100.0]))] {
            app.run("window.toolbarColumns", json!({ "double": double })).unwrap();
            app.ui.toolbar_pos = floating;
            frame(&mut app, &ctx, 0.0, vec![]);
            let tools = frame(&mut app, &ctx, 0.1, vec![]);
            // The title bar spans the panel's inside.
            let hdr = ctx.viewport(|vp| {
                vp.prev_pass
                    .widgets
                    .layers()
                    .flat_map(|(_, w)| w.iter())
                    .filter(|w| w.rect.height() == 14.0 && w.rect.width() > 40.0)
                    .map(|w| w.rect)
                    .next()
            });
            let hdr = hdr.unwrap();
            let first = tools[0];
            let row_right = tools.iter().filter(|r| r.top() == first.top()).map(|r| r.right()).fold(first.right(), f32::max);
            assert_eq!(row_right - hdr.center().x, hdr.center().x - first.left(), "double {double}, floating {floating:?}: {first:?} in {hdr:?}");
            assert!(first.left() >= hdr.left() && row_right <= hdr.right(), "double {double}, floating {floating:?}: {first:?} in {hdr:?}");
        }
    }

    /// Screen readers name each tool button by its tool tip and say which tool is active, and name
    /// the title bar's column toggle and grip and the paint buttons under the tools.
    #[test]
    fn screen_readers_name_the_tool_buttons_and_the_active_tool() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.select_tool("pen");
        let tree = tree(|ui| show(&mut app, ui));
        let name = |id| tool_info(id).map(tip).unwrap_or_default();
        assert_eq!(role_and_state(&tree, &name("pen")), (Role::Button, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, &name("selection")), (Role::Button, Some(Toggled::False)));
        for label in ["Toggle single/double column", "Drag to float the toolbar", "Color (,)", "Gradient (.)", "None (/)"] {
            assert_eq!(role_and_state(&tree, label), (Role::Button, None), "{label}");
        }
    }

    /// #812: double-clicking the Artboard tool opens Artboard Options of the active artboard, a
    /// graph tool Graph Type for the selected graph, and the Magic Wand tool its panel.
    #[test]
    fn more_tools_open_their_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        // Without the Artboard tool, the window's artboard.
        assert_eq!(open_options(&mut app, "artboard").unwrap()["dialog"], "artboardOptions");
        let d = app.ui.dialog.take().unwrap();
        assert_eq!((d.f64("index", 9.0), d.f64("width", 0.0), d.str("name")), (0.0, 300.0, "Artboard 1".to_string()));
        // With it, its active artboard.
        app.run("artboard.new", json!({"x": 400, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        app.run("tool.setOption", json!({"key": "active", "value": 1})).unwrap();
        open_options(&mut app, "artboard").unwrap();
        let d = app.ui.dialog.take().unwrap();
        assert_eq!((d.f64("index", 9.0), d.f64("x", 0.0), d.f64("width", 0.0)), (1.0, 400.0, 100.0));
        // Graph Type needs a selected graph.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(open_options(&mut app, "pieGraph").is_err());
        assert!(app.ui.dialog.is_none());
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
        assert_eq!(open_options(&mut app, "pieGraph").unwrap()["dialog"], "command");
        let d = app.ui.dialog.take().unwrap();
        assert_eq!((d.str("__command"), d.str("type")), ("graph.setType".to_string(), "column".to_string()));
        // The Magic Wand panel opens, and stays open on a second double-click.
        open_options(&mut app, "magicWand").unwrap();
        assert_eq!(app.ui.open_panel.as_deref(), Some("magicWand"));
        assert_eq!(open_options(&mut app, "magicWand").unwrap()["open"], "magicWand");
        assert_eq!(app.ui.open_panel.as_deref(), Some("magicWand"));
    }

    /// A double-click at `at`, `time` seconds in (a second apart from the last).
    fn double_click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2) {
        let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, time, vec![Event::PointerMoved(at), b(true), b(false), b(true), b(false)]);
        frame(app, ctx, time + 0.1, vec![]);
    }

    #[test]
    fn double_clicking_a_tool_button_opens_its_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        let buttons = frame(&mut app, &ctx, 0.0, vec![]);
        // The Color category's slots close the list: Gradient, then Eyedropper.
        let (gradient, eyedropper) = (buttons[buttons.len() - 2], buttons[buttons.len() - 1]);
        double_click(&mut app, &ctx, 1.0, gradient.center());
        assert_eq!((app.session.tool_id(), app.ui.open_panel.as_deref()), ("gradient", Some("gradient")));
        // Again: the panel stays open.
        double_click(&mut app, &ctx, 2.0, gradient.center());
        assert_eq!(app.ui.open_panel.as_deref(), Some("gradient"));
        double_click(&mut app, &ctx, 3.0, eyedropper.center());
        let d = app.ui.dialog.clone().expect("Eyedropper Options");
        assert_eq!((app.session.tool_id(), d.kind.as_str()), ("eyedropper", crate::dialogs::eyedropper::KIND));
        // OK applies the options.
        app.ui.dialog.as_mut().unwrap().fields.insert("apply".into(), json!({"appearance": {"transparency": false}}));
        crate::dialogs::confirm(&mut app).unwrap();
        let o = app.session.prefs.eyedropper;
        assert!(o.pick_up.appearance.transparency && !o.apply.appearance.transparency && o.apply.appearance.fill.color);
        assert!(app.run("tool.options", json!({"tool": "lasso"})).is_err());
    }

    #[test]
    fn double_clicking_the_hand_zoom_and_transform_tools() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.canvas_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(660.0, 460.0)));
        app.run("view.setZoom", json!({"zoom": 333, "center": [10, 10]})).unwrap();
        // Hand: the artboard fits the window. Zoom: 100%.
        app.run("tool.options", json!({"tool": "hand"})).unwrap();
        let v = *app.view().unwrap();
        assert_eq!((v.center.x, v.center.y, v.zoom), (150.0, 100.0, 2.0));
        app.run("tool.options", json!({"tool": "zoom"})).unwrap();
        assert_eq!(app.view().unwrap().zoom, 1.0);
        // The transform tools open their dialogs, like Object › Transform: not with nothing selected.
        assert_eq!(app.run("tool.options", json!({"tool": "rotate"})), Err("nothing selected".into()));
        assert!(app.ui.dialog.is_none());
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
        for tool in ["rotate", "scale", "reflect", "shear"] {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some(tool.to_string()));
        }
        // OK in Rotate turns the selection about its centre.
        app.run("tool.options", json!({"tool": "rotate"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("angle".into(), json!(90));
        crate::dialogs::confirm(&mut app).unwrap();
        let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert!((b.x0 - 20.0).abs() < 1e-9 && (b.y0 - 0.0).abs() < 1e-9 && (b.width() - 20.0).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn double_clicking_a_selection_tool_opens_the_move_dialog() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        // The Selection tool's button is the first one.
        let selection = frame(&mut app, &ctx, 0.0, vec![])[0];
        // Nothing selected: Move is disabled, so nothing opens.
        double_click(&mut app, &ctx, 1.0, selection.center());
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.run("tool.options", json!({"tool": "directSelection"})), Err("nothing selected".into()));
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
        double_click(&mut app, &ctx, 2.0, selection.center());
        assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some("move".to_string()));
        // The Direct Selection and Group Selection tools open it too.
        for tool in ["directSelection", "groupSelection"] {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some("move".to_string()), "{tool}");
        }
        // OK moves the selection by what was typed.
        app.run("tool.options", json!({"tool": "selection"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("dx".into(), json!("25 pt"));
        crate::dialogs::confirm(&mut app).unwrap();
        let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0), (35.0, 10.0));
    }

    /// #1616: in every language a flyout is wide enough that no tool's name runs into its shortcut
    /// (the Spanish Live Paint names need more than the 250 pt the English ones fit in).
    #[test]
    fn a_flyout_is_as_wide_as_its_longest_translated_row() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for lang in crate::i18n::Lang::all() {
                for group in TOOL_GROUPS {
                    let rows: Vec<RowText> = group.iter().map(|tool| row_text(ui, lang, tool, Color32::WHITE)).collect();
                    let w = rows_width(rows.iter());
                    assert!(w >= 250.0, "{lang:?}");
                    for (tool, (name, sc)) in group.iter().zip(&rows) {
                        let name_end = NAME_X + name.size().x;
                        let shortcut_start = w - ROW_END - sc.as_ref().map_or(0.0, |sc| sc.size().x);
                        assert!(
                            name_end <= shortcut_start + 0.01,
                            "{lang:?} {}: the name ends at {name_end}, the shortcut starts at {shortcut_start}",
                            tool.id
                        );
                    }
                }
            }
            let es = crate::i18n::Lang::from_code("es").unwrap();
            let live_paint = TOOL_GROUPS.iter().find(|g| g.iter().any(|t| t.id == "livePaintSelection")).expect("the Live Paint group");
            let rows: Vec<RowText> = live_paint.iter().map(|tool| row_text(ui, es, tool, Color32::WHITE)).collect();
            assert!(rows_width(rows.iter()) > 250.0, "the Spanish Live Paint flyout is wider than the English one");
        });
        out.textures_delta.clear();
    }

    #[test]
    fn right_clicking_a_tool_group_opens_its_flyout() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        let buttons = frame(&mut app, &ctx, 0.0, vec![]);
        // Selection, Direct Selection, Lasso, then the Shapes group (Rectangle and its sub-tools).
        let at = buttons[3].center();
        let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), b(true)]);
        frame(&mut app, &ctx, 1.05, vec![b(false)]);
        frame(&mut app, &ctx, 1.1, vec![]);
        assert!(app.ui.flyout.is_some(), "the right-click opens the flyout and the same click doesn't close it");
        let menu = ctx.memory(|m| m.area_rect(egui::Id::new("tool-flyout"))).expect("the flyout is shown");
        assert_eq!(app.session.tool_id(), "selection", "a right-click only opens the flyout");
        // Its second row is Rounded Rectangle (30 pt rows).
        let row = egui::pos2(menu.left() + 60.0, menu.top() + 45.0);
        let p = |pressed| Event::PointerButton { pos: row, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 2.0, vec![Event::PointerMoved(row), p(true)]);
        frame(&mut app, &ctx, 2.05, vec![p(false)]);
        assert_eq!((app.session.tool_id(), app.ui.flyout), ("roundedRectangle", None));
        // A long press opens it too, and releasing the press (a click, being short of 0.8 s) leaves
        // it open without choosing the button's tool.
        let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 3.0, vec![Event::PointerMoved(at), p(true)]);
        frame(&mut app, &ctx, 3.5, vec![]);
        assert!(app.ui.flyout.is_some(), "a long press opens the flyout");
        frame(&mut app, &ctx, 3.6, vec![p(false)]);
        frame(&mut app, &ctx, 3.7, vec![]);
        assert!(app.ui.flyout.is_some(), "releasing the long press leaves the flyout open");
        // A click anywhere else closes it.
        let away = egui::pos2(300.0, 1000.0);
        let c = |pressed| Event::PointerButton { pos: away, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 4.0, vec![Event::PointerMoved(away), c(true)]);
        frame(&mut app, &ctx, 4.05, vec![c(false)]);
        assert_eq!((app.session.tool_id(), app.ui.flyout), ("roundedRectangle", None));
    }

    /// The Shapes group's button (Selection, Direct Selection, Lasso, then Rectangle).
    fn shapes_button(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::Rect {
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        frame(app, ctx, 0.0, vec![])[3]
    }

    #[test]
    fn a_held_press_asks_for_the_frame_that_opens_the_flyout() {
        // egui draws only on input: a still press must schedule the frame where it becomes long.
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let at = shapes_button(&mut app, &ctx).center();
        let p = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() };
        frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), p]);
        // The frames egui runs after the input settle; then nothing moves.
        frame(&mut app, &ctx, 1.02, vec![]);
        let input =
            egui::RawInput { time: Some(1.04), screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1200.0))), ..Default::default() };
        let out = ctx.run_ui(input, |ui| show(&mut app, ui));
        let delay = out.viewport_output.get(&egui::ViewportId::ROOT).unwrap().repaint_delay;
        assert!(delay <= std::time::Duration::from_secs_f64(LONG_PRESS), "repaint in {delay:?}");
        assert!(app.ui.flyout.is_none(), "not open before the press is long");
    }

    #[test]
    fn pressing_the_corner_triangle_opens_the_flyout_at_once() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let button = shapes_button(&mut app, &ctx);
        let at = button.right_bottom() - vec2(4.0, 4.0);
        let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), p(true)]);
        assert!(app.ui.flyout.is_some(), "the press on the triangle opens it");
        frame(&mut app, &ctx, 1.05, vec![p(false)]);
        frame(&mut app, &ctx, 1.1, vec![]);
        assert_eq!((app.session.tool_id(), app.ui.flyout.is_some()), ("selection", true), "the release neither closes it nor picks the tool");
        // A plain click in the middle of the button still just picks its tool.
        let mid = button.center();
        let c = |pressed| Event::PointerButton { pos: mid, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 2.0, vec![Event::PointerMoved(mid), c(true)]);
        frame(&mut app, &ctx, 2.05, vec![c(false)]);
        assert_eq!((app.session.tool_id(), app.ui.flyout), ("rectangle", None));
    }

    #[test]
    fn the_flyout_opens_on_the_right_press_even_when_its_tool_is_active() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let at = shapes_button(&mut app, &ctx).center();
        app.select_tool("rectangle");
        for (k, t0) in [1.0, 3.0, 5.0].into_iter().enumerate() {
            let r = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
            frame(&mut app, &ctx, t0, vec![Event::PointerMoved(at), r(true)]);
            assert!(app.ui.flyout.is_some(), "right press #{k} opens it before the release");
            frame(&mut app, &ctx, t0 + 0.05, vec![r(false)]);
            assert!(app.ui.flyout.is_some(), "right press #{k} leaves it open");
            // A click away closes it; then try again.
            let away = egui::pos2(300.0, 1000.0);
            let c = |pressed| Event::PointerButton { pos: away, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&mut app, &ctx, t0 + 1.0, vec![Event::PointerMoved(away), c(true)]);
            frame(&mut app, &ctx, t0 + 1.05, vec![c(false)]);
            assert!(app.ui.flyout.is_none());
        }
        // A long press on the active tool's button opens it too.
        let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 8.0, vec![Event::PointerMoved(at), p(true)]);
        frame(&mut app, &ctx, 8.0 + LONG_PRESS + 0.01, vec![]);
        assert!(app.ui.flyout.is_some(), "a long press on the active tool opens it");
        assert_eq!(app.session.tool_id(), "rectangle");
    }

    #[test]
    fn type_button_long_press_selects_vertical_type_in_both_layouts() {
        for advanced in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.ui.toolbar_advanced = advanced;
            let all = slots(&app);
            let index = all.iter().position(|(_, tools)| tools.contains(&"type")).unwrap();
            assert!(all[index].1.contains(&"verticalType"));
            for id in ["type", "verticalType", "areaType", "verticalAreaType", "typeOnPath", "verticalTypeOnPath"] {
                assert_eq!(all.iter().filter(|(_, tools)| tools.contains(&id)).count(), 1);
            }
            let ctx = egui::Context::default();
            let at = frame_in(&mut app, &ctx, 0.0, vec![], 2000.0)[index].center();
            let pointer = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame_in(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), pointer(at, true)], 2000.0);
            frame_in(&mut app, &ctx, 1.36, vec![], 2000.0);
            frame_in(&mut app, &ctx, 1.4, vec![pointer(at, false)], 2000.0);
            frame_in(&mut app, &ctx, 1.45, vec![], 2000.0);
            assert_eq!(app.session.tool_id(), "selection");
            let tools: Vec<String> = ctx.data(|d| d.get_temp(egui::Id::new("flyout-tools"))).unwrap();
            let row_index = tools.iter().position(|id| id == "verticalType").unwrap();
            let menu = ctx.memory(|m| m.area_rect(egui::Id::new("tool-flyout"))).unwrap();
            let row = egui::pos2(menu.left() + 60.0, menu.top() + 15.0 + row_index as f32 * 30.0);
            frame_in(&mut app, &ctx, 2.0, vec![Event::PointerMoved(row), pointer(row, true)], 2000.0);
            frame_in(&mut app, &ctx, 2.05, vec![pointer(row, false)], 2000.0);
            assert_eq!(app.session.tool_id(), "verticalType");
            assert_eq!(app.ui.flyout, None);
        }
    }

    /// A press at `from`, dragged through `to` (one frame each), then released.
    fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, from: Pos2, to: &[Pos2]) {
        let b = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, time, vec![Event::PointerMoved(from), b(from, true)]);
        for (k, p) in to.iter().enumerate() {
            frame(app, ctx, time + 0.05 * (k + 1) as f64, vec![Event::PointerMoved(*p)]);
        }
        let end = to.last().copied().unwrap_or(from);
        frame(app, ctx, time + 0.05 * (to.len() + 1) as f64, vec![b(end, false)]);
        frame(app, ctx, time + 0.05 * (to.len() + 2) as f64, vec![]);
    }

    /// A click at `at`.
    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2, button: PointerButton) {
        click_in(app, ctx, time, at, button, 1200.0);
    }

    /// [`click`] in a window `height` points tall; returns the tool buttons' rects after it.
    fn click_in(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2, button: PointerButton, height: f32) -> Vec<egui::Rect> {
        let b = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() };
        frame_in(app, ctx, time, vec![Event::PointerMoved(at), b(true)], height);
        frame_in(app, ctx, time + 0.05, vec![b(false)], height);
        frame_in(app, ctx, time + 0.1, vec![], height)
    }

    /// The Color, Gradient and None buttons grow with the toolbar, and in two columns the three
    /// drawing modes sit side by side, each setting its mode (#1129).
    #[test]
    fn the_paint_and_drawing_mode_buttons_fit_the_toolbar_width() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        frame_in(&mut app, &ctx, 0.0, vec![], 900.0);
        assert_eq!(widget_rects(&ctx, vec2(11.0, 11.0)).len(), 3, "one column: three 11 pt chips");
        app.ui.toolbar_double = true;
        frame_in(&mut app, &ctx, 1.0, vec![], 900.0);
        assert_eq!(widget_rects(&ctx, vec2(17.0, 17.0)).len(), 3, "two columns: three 17 pt chips");
        let modes = widget_rects(&ctx, vec2(22.0, 22.0));
        assert_eq!(modes.len(), 3, "the three drawing modes side by side");
        assert!(modes.iter().all(|r| (r.top() - modes[0].top()).abs() < 0.5), "{modes:?}");
        click_in(&mut app, &ctx, 2.0, modes[1].center(), PointerButton::Primary, 900.0);
        assert_eq!(app.session.draw_mode, vectorcraft_engine::DrawMode::Behind);
    }

    /// How many columns of tool buttons `rects` make.
    fn columns(rects: &[egui::Rect]) -> usize {
        let mut lefts: Vec<i32> = rects.iter().map(|r| r.left().round() as i32).collect();
        lefts.sort_unstable();
        lefts.dedup();
        lefts.len()
    }

    /// Rows of tool buttons by their tops: how many buttons each row holds, top to bottom.
    fn row_sizes(rects: &[egui::Rect]) -> Vec<usize> {
        let mut rows: Vec<(i32, usize)> = vec![];
        for r in rects {
            let top = r.top().round() as i32;
            match rows.last_mut() {
                Some((t, n)) if *t == top => *n += 1,
                _ => rows.push((top, 1)),
            }
        }
        rows.into_iter().map(|(_, n)| n).collect()
    }

    /// #1736: without group labels (as it ships), the tools run on from group to group: one column
    /// evenly spaced, with nothing between the groups; two columns with every row full but maybe
    /// the last. With the labels, each group starts its own row under its name.
    #[test]
    fn the_tools_run_on_without_group_labels() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        let one = frame_in(&mut app, &ctx, 0.0, vec![], 1600.0);
        assert_eq!(one.len(), slots(&app).len());
        let steps: Vec<f32> = one.windows(2).map(|w| w[1].top() - w[0].top()).collect();
        assert!(steps.iter().all(|d| (d - steps[0]).abs() < 0.5), "evenly spaced: {steps:?}");
        app.ui.toolbar_double = true;
        let rows = row_sizes(&frame_in(&mut app, &ctx, 0.5, vec![], 1600.0));
        let slots = slots(&app).len();
        assert_eq!(rows.iter().sum::<usize>(), slots);
        assert!(rows[..rows.len() - 1].iter().all(|n| *n == 2), "{rows:?}");
        assert_eq!(rows.len(), slots.div_ceil(2));
        app.run("prefs.set", json!({"key": "toolGroupLabels", "value": true})).unwrap();
        let labelled = row_sizes(&frame_in(&mut app, &ctx, 1.0, vec![], 1600.0));
        assert!(labelled.len() > rows.len() && labelled.contains(&1), "{labelled:?}");
    }

    #[test]
    fn the_double_arrow_toggles_one_and_two_columns_in_a_short_window() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        // 700 pt: too short for the single column, which scrolls instead of turning into two.
        assert_eq!(columns(&frame_in(&mut app, &ctx, 0.0, vec![], 700.0)), 1);
        let arrow = pos2(8.0, 7.0);
        assert_eq!(columns(&click_in(&mut app, &ctx, 1.0, arrow, PointerButton::Primary, 700.0)), 2);
        assert!(app.ui.toolbar_double && crate::menus::checked(&app, "window.toolbarColumns", &json!({})) == Some(true));
        assert_eq!(columns(&click_in(&mut app, &ctx, 2.0, arrow, PointerButton::Primary, 700.0)), 1);
        assert!(!app.ui.toolbar_double);
    }

    #[test]
    fn the_columns_command_toggles_or_sets_the_toolbar_columns() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        assert_eq!(app.run("window.toolbarColumns", json!({})).unwrap(), json!(true));
        assert_eq!(columns(&frame_in(&mut app, &ctx, 0.0, vec![], 700.0)), 2);
        assert_eq!(app.run("window.toolbarColumns", json!({"double": true})).unwrap(), json!(true));
        assert_eq!(app.run("window.toolbarColumns", json!({})).unwrap(), json!(false));
        assert_eq!(crate::menus::checked(&app, "window.toolbarColumns", &json!({})), Some(false));
        assert_eq!(columns(&frame_in(&mut app, &ctx, 1.0, vec![], 700.0)), 1);
        assert!(app.run("window.toolbarColumns", json!({"double": "yes"})).is_err());
        // Advanced (every tool group) is taller still and keeps the column the user chose too.
        app.run("window.toolbarAdvanced", json!({})).unwrap();
        assert_eq!(columns(&frame_in(&mut app, &ctx, 2.0, vec![], 700.0)), 1);
    }

    #[test]
    fn a_flyout_dragged_by_its_bar_floats_until_its_close_box_puts_it_back() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let at = shapes_button(&mut app, &ctx).center();
        click(&mut app, &ctx, 1.0, at, PointerButton::Secondary);
        let menu = ctx.memory(|m| m.area_rect(egui::Id::new("tool-flyout"))).expect("the flyout is shown");
        // Its grab bar runs down its right side: dragging it tears the flyout off.
        let bar = pos2(menu.right() - BAR / 2.0, menu.center().y);
        drag(&mut app, &ctx, 2.0, bar, &[bar + vec2(20.0, 0.0), bar + vec2(60.0, 40.0)]);
        assert_eq!(app.ui.flyout, None, "the flyout became the floating one");
        let [f] = app.ui.floating_flyouts.as_slice() else { panic!("one floating flyout: {:?}", app.ui.floating_flyouts) };
        assert_eq!(f.tools.first().map(String::as_str), Some("rectangle"));
        let float = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).expect("the floating flyout is shown");
        // A strip of its nine tools' buttons, its bar under the pointer.
        let drop = bar + vec2(60.0, 40.0);
        assert!(float.height() < 40.0 && float.width() > 9.0 * STRIP_BUTTON.x, "a one-row strip: {float:?}");
        assert!(
            (float.right() - 1.0 - BAR / 2.0 - drop.x).abs() < 1.0 && float.y_range().contains(drop.y),
            "its bar is under the pointer: {float:?}"
        );
        // The same bar moves it.
        let bar = pos2(float.right() - 1.0 - BAR / 2.0, float.bottom() - 6.0);
        drag(&mut app, &ctx, 3.0, bar, &[bar + vec2(0.0, 20.0), bar + vec2(-10.0, 100.0)]);
        let moved = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).unwrap();
        assert!((moved.min - (float.min + vec2(-10.0, 100.0))).length() < 2.0, "moved to {moved:?} from {float:?}");
        // Picking a tool in it leaves it open.
        click(&mut app, &ctx, 4.0, pos2(moved.left() + STRIP_BUTTON.x * 1.5, moved.center().y), PointerButton::Primary);
        assert_eq!((app.session.tool_id(), app.ui.floating_flyouts.len()), ("roundedRectangle", 1));
        // While it floats, the presses that open the group's flyout raise it instead.
        click(&mut app, &ctx, 5.0, at, PointerButton::Secondary);
        assert_eq!(app.ui.flyout, None, "no flyout beside the floating one");
        assert!(ctx.data(|d| d.get_temp::<f64>(floating_area("rectangle"))).is_some(), "the floating flyout was raised");
        let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 6.0, vec![Event::PointerMoved(at), p(true)]);
        frame(&mut app, &ctx, 6.0 + LONG_PRESS + 0.01, vec![]);
        frame(&mut app, &ctx, 6.5, vec![p(false)]);
        assert_eq!((app.ui.flyout, app.session.tool_id()), (None, "roundedRectangle"), "a long press neither opens the flyout nor picks the tool");
        // The × at the top of the bar puts it back: the group opens as a flyout again.
        click(&mut app, &ctx, 7.0, pos2(moved.right() - BAR / 2.0, moved.top() + CLOSE / 2.0), PointerButton::Primary);
        assert!(app.ui.floating_flyouts.is_empty());
        click(&mut app, &ctx, 8.0, at, PointerButton::Secondary);
        assert!(app.ui.flyout.is_some());
    }

    #[test]
    fn a_click_on_a_flyouts_bar_tears_it_off() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let at = shapes_button(&mut app, &ctx).center();
        click(&mut app, &ctx, 1.0, at, PointerButton::Secondary);
        let menu = ctx.memory(|m| m.area_rect(egui::Id::new("tool-flyout"))).unwrap();
        let bar = pos2(menu.right() - BAR / 2.0, menu.center().y);
        click(&mut app, &ctx, 2.0, bar, PointerButton::Primary);
        frame(&mut app, &ctx, 2.2, vec![]);
        assert_eq!(app.ui.flyout, None);
        assert_eq!(app.ui.floating_flyouts.iter().map(|f| f.tools[0].as_str()).collect::<Vec<_>>(), ["rectangle"]);
        let float = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).expect("the floating flyout is shown");
        assert!(float.contains(bar), "its bar is under the pointer: {float:?}");
        assert_eq!(app.session.tool_id(), "selection", "no tool picked");
    }

    #[test]
    fn releasing_a_long_press_over_the_flyouts_bar_tears_it_off() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let at = shapes_button(&mut app, &ctx).center();
        let b = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), b(at, true)]);
        frame(&mut app, &ctx, 1.0 + LONG_PRESS + 0.01, vec![]);
        frame(&mut app, &ctx, 1.45, vec![]);
        let menu = ctx.memory(|m| m.area_rect(egui::Id::new("tool-flyout"))).expect("the long press opens the flyout");
        let bar = pos2(menu.right() - BAR / 2.0, menu.center().y);
        frame(&mut app, &ctx, 1.5, vec![Event::PointerMoved(bar)]);
        frame(&mut app, &ctx, 1.55, vec![b(bar, false)]);
        frame(&mut app, &ctx, 1.6, vec![]);
        assert_eq!((app.ui.flyout, app.ui.floating_flyouts.len(), app.session.tool_id()), (None, 1, "selection"));
    }

    #[test]
    fn window_float_tools_floats_and_puts_back_a_group() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let check = |app: &VectorcraftApp| crate::menus::checked(app, "window.floatTools", &json!({"tool": "star"}));
        assert_eq!(app.run("window.floatTools", json!({"tool": "star"})).unwrap(), json!(true));
        let [f] = app.ui.floating_flyouts.as_slice() else { panic!("one strip: {:?}", app.ui.floating_flyouts) };
        assert_eq!(f.tools.first().map(String::as_str), Some("rectangle"), "the Basic toolbar's Shapes group");
        assert_eq!(check(&app), Some(true));
        // Already floating: stays where it is.
        app.ui.floating_flyouts[0].pos = [300.0, 400.0];
        assert_eq!(app.run("window.floatTools", json!({"tool": "rectangle", "floating": true})).unwrap(), json!(true));
        assert_eq!(app.ui.floating_flyouts.iter().map(|f| f.pos).collect::<Vec<_>>(), [[300.0, 400.0]]);
        // Omitted toggles it back.
        assert_eq!(app.run("window.floatTools", json!({"tool": "star"})).unwrap(), json!(false));
        assert!(app.ui.floating_flyouts.is_empty() && check(&app) == Some(false));
        assert_eq!(app.run("window.floatTools", json!({"tool": "pen", "floating": false})).unwrap(), json!(false));
        // A tool alone in its slot, an unknown tool, a bad flag.
        assert!(app.run("window.floatTools", json!({"tool": "ellipse"})).is_err());
        assert!(app.run("window.floatTools", json!({"tool": "nope"})).is_err());
        assert!(app.run("window.floatTools", json!({})).is_err());
        assert!(app.run("window.floatTools", json!({"tool": "pen", "floating": "yes"})).is_err());
        assert!(app.ui.floating_flyouts.is_empty());
    }

    #[test]
    fn workspaces_keep_floating_flyouts_and_reset_puts_them_back() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("window.floatTools", json!({"tool": "pen"})).unwrap();
        app.run("window.workspace.new", json!({"name": "Strips"})).unwrap();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.floating_flyouts.is_empty(), "the built-in workspaces float none");
        app.run("window.workspace", json!({"name": "Strips"})).unwrap();
        assert_eq!(app.ui.floating_flyouts.iter().map(|f| f.tools[0].as_str()).collect::<Vec<_>>(), ["pen"]);
        // Workspaces saved before floating strips load without any.
        let mut old = serde_json::to_value(crate::workspaces::Workspace::default()).unwrap();
        old.as_object_mut().unwrap().remove("floatingFlyouts");
        assert!(serde_json::from_value::<crate::workspaces::Workspace>(old).unwrap().floating_flyouts.is_empty());
    }

    #[test]
    fn loaded_floating_flyouts_keep_known_tools_one_strip_per_group() {
        let f = |tools: &[&str]| crate::state::FloatingFlyout { tools: tools.iter().map(|s| s.to_string()).collect(), pos: [0.0, 0.0] };
        let ui = crate::state::UiState {
            floating_flyouts: vec![f(&["pen", "nope", "addAnchor"]), f(&["pen", "addAnchor"]), f(&["nope"]), f(&[])],
            ..Default::default()
        };
        let ui = ui.sanitized();
        assert_eq!(ui.floating_flyouts.iter().map(|f| f.tools.join(",")).collect::<Vec<_>>(), ["pen,addAnchor"]);
    }

    #[test]
    fn floating_flyouts_survive_saved_preferences_and_stay_on_screen() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools: vec!["pen".into(), "addAnchor".into()], pos: [5000.0, 10.0] });
        let saved = serde_json::to_value(&app.ui).unwrap();
        let back: crate::state::UiState = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(back.floating_flyouts.len(), 1);
        // Preferences saved before floating flyouts load without any.
        let mut old = saved;
        old.as_object_mut().unwrap().remove("floating_flyouts");
        assert!(serde_json::from_value::<crate::state::UiState>(old).unwrap().floating_flyouts.is_empty());
        let ctx = egui::Context::default();
        shapes_button(&mut app, &ctx);
        frame(&mut app, &ctx, 1.0, vec![]);
        let r = ctx.memory(|m| m.area_rect(floating_area("pen"))).unwrap();
        assert!(r.right() <= 400.5 && r.left() >= 0.0 && r.top() >= 0.0, "on screen: {r:?}");
        // A position that isn't a number puts it near the toolbar.
        app.ui.floating_flyouts[0].pos = [f32::NAN, f32::INFINITY];
        frame(&mut app, &ctx, 2.0, vec![]);
        let r = ctx.memory(|m| m.area_rect(floating_area("pen"))).unwrap();
        assert!(r.min.x.is_finite() && r.min.y.is_finite() && r.left() >= 0.0, "on screen: {r:?}");
    }

    /// User Interface › Show Tool Group Labels (#663): off as it ships (#1736), the toolbar shows
    /// none of its groups' names and nothing between the groups; on, it names them.
    #[test]
    fn tool_group_labels_can_be_hidden() {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let draw = |app: &mut VectorcraftApp| {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1600.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
            let mut shown = vec![];
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
            shown
        };
        let without = draw(&mut app);
        assert!(!without.iter().any(|t| ["Select", "Shapes", "Draw", "Modify", "Type"].contains(&t.as_str())), "{without:?}");
        app.run("prefs.set", json!({"key": "toolGroupLabels", "value": true})).unwrap();
        let with = draw(&mut app);
        assert!(with.iter().any(|t| t == "Shapes") && with.iter().any(|t| t == "Draw"), "{with:?}");
    }

    /// Hover `at` for two seconds, coming from elsewhere: the texts painted meanwhile.
    fn hovered_texts(app: &mut VectorcraftApp, ctx: &egui::Context, t0: f64, at: Pos2) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut shown = vec![];
        for i in 0..20 {
            let events = match i {
                0 => vec![Event::PointerGone],
                1 => vec![Event::PointerMoved(at)],
                _ => vec![],
            };
            let time = t0 + f64::from(i) * 0.1;
            let raw = egui::RawInput {
                events,
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1200.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                crate::prefs_dialog::apply_runtime(app, ui.ctx());
                show(app, ui);
            });
            out.textures_delta.clear();
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        }
        shown
    }

    #[test]
    fn a_floating_flyouts_buttons_have_tool_tips_when_tool_tips_are_on() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let tools: Vec<String> = ["rectangle", "roundedRectangle", "star"].map(String::from).to_vec();
        app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [150.0, 600.0] });
        let ctx = egui::Context::default();
        shapes_button(&mut app, &ctx);
        frame(&mut app, &ctx, 1.0, vec![]);
        let strip = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).unwrap();
        let star = pos2(strip.left() + STRIP_BUTTON.x * 2.5, strip.center().y);
        let tip = tl!("Star Tool").to_string();
        assert!(hovered_texts(&mut app, &ctx, 10.0, star).contains(&tip), "on by default");
        app.session.prefs.show_tool_tips = false;
        assert!(!hovered_texts(&mut app, &ctx, 20.0, star).contains(&tip), "off: no tool tip");
        app.session.prefs.show_tool_tips = true;
        assert!(hovered_texts(&mut app, &ctx, 30.0, star).contains(&tip), "on again");
    }

    /// Screen readers name a flyout's rows and a floating strip's buttons by their tools' tool tips,
    /// with the active tool's pressed, and name the strip's close box and both tear-off bars.
    #[test]
    fn screen_readers_name_a_flyouts_rows_and_a_floating_strips_buttons() {
        let name = |id| tool_info(id).map(tip).unwrap_or_default();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let group: Vec<String> = ["rectangle", "roundedRectangle", "ellipse"].map(String::from).to_vec();
        let flyout = tree(|ui| {
            app.ui.flyout = Some(0);
            ui.data_mut(|d| d.insert_temp(egui::Id::new("flyout-tools"), group.clone()));
            show(&mut app, ui);
        });
        assert_eq!(role_and_state(&flyout, &name("roundedRectangle")), (Role::Button, Some(Toggled::False)));
        assert_eq!(role_and_state(&flyout, &name("ellipse")), (Role::Button, Some(Toggled::False)));
        assert_eq!(role_and_state(&flyout, "Drag to float as a toolbar"), (Role::Button, None));
        app.ui.flyout = None;
        app.select_tool("star");
        let tools: Vec<String> = ["polygon", "star"].map(String::from).to_vec();
        app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [150.0, 600.0] });
        let strip = tree(|ui| show(&mut app, ui));
        assert_eq!(role_and_state(&strip, &name("polygon")), (Role::Button, Some(Toggled::False)));
        assert_eq!(role_and_state(&strip, &name("star")), (Role::Button, Some(Toggled::True)));
        assert_eq!(role_and_state(&strip, "Put back in the toolbar"), (Role::Button, None));
        assert_eq!(role_and_state(&strip, "Drag to move"), (Role::Button, None));
    }
}
