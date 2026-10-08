//! The Tools panel. Default: Illustrator 2026's categorized single-column toolbar (Select, Shapes,
//! Draw, Modify, Type, Navigate, Color). Window → Toolbars → Advanced shows every tool group.
//! Bottom: fill/stroke proxy, colour/gradient/none, drawing modes, screen mode, Edit Toolbar.

use egui::{Color32, CornerRadius, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_tools::{TOOL_GROUPS, ToolInfo, tool_info};

use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, icons, widgets};

const PITCH: f32 = 30.0;
const WIDTH: f32 = 48.0;

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
        ],
    ),
    ("Type", &[&["areaType", "typeOnPath", "verticalType", "verticalAreaType", "verticalTypeOnPath"], &["type", "touchType"]]),
    ("Navigate", &[&["zoom"], &["hand", "printTiling"], &["rotateView"]]),
    ("Color", &[&["gradient", "mesh"], &["eyedropper", "measure"]]),
];

fn tip(t: &ToolInfo) -> String {
    match crate::shortcut_editor::tool_shortcut(t.id) {
        Some(s) => format!("{} ({})", tl!(t.label), s),
        None => tl!(t.label).to_string(),
    }
}

/// Slots of the current layout: (category label for the first slot of a category, tool ids).
fn slots(app: &VectorcraftApp) -> Vec<(Option<&'static str>, Vec<&'static str>)> {
    if app.ui.toolbar_advanced {
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
    let all = slots(app);
    let avail = ui.available_height();
    let labels = all.iter().filter(|s| s.0.is_some()).count() as f32 * 20.0;
    let need = all.len() as f32 * PITCH + labels + 190.0;
    let cols = if app.ui.toolbar_double || avail < need { 2 } else { 1 };
    let w = if cols == 2 { 76.0 } else { WIDTH };
    let shown = egui::Panel::left("toolbar")
        .resizable(false)
        .exact_size(w)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 4 }).stroke(Stroke::new(1.5, t.border)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            // Dock header with » and a grip strip.
            let (hdr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::click());
            ui.painter().rect_filled(hdr, 0.0, t.tab_strip);
            icons::paint(
                ui,
                if cols == 2 { "chevrons-left" } else { "chevrons-right" },
                egui::Rect::from_min_size(hdr.left_top() + vec2(3.0, 2.0), vec2(10.0, 10.0)),
                if hresp.hovered() { t.text_strong } else { t.text },
            );
            if hresp.on_hover_text(tl!("Toggle single/double column")).clicked() {
                app.ui.toolbar_double = !app.ui.toolbar_double;
            }
            let (grip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
            for k in 0..6 {
                ui.painter().line_segment(
                    [pos2(grip.center().x - 9.0, grip.top() + 1.5 + k as f32 * 0.6), pos2(grip.center().x + 9.0, grip.top() + 1.5 + k as f32 * 0.6)],
                    Stroke::new(0.4, t.text_disabled),
                );
            }
            // The tools and the controls under them scroll in a window too short for them.
            widgets::strip_scroll(ui, "toolbar", |ui| {
                let active = app.session.tool_id();
                // The tool button whose long press opened its flyout, until the next press.
                let held_id = egui::Id::new("toolbar-held");
                if ui.input(|inp| inp.pointer.any_pressed()) {
                    ui.data_mut(|d| d.remove::<egui::Id>(held_id));
                }
                let held: Option<egui::Id> = ui.data(|d| d.get_temp(held_id));
                let mut open_flyout: Option<(Vec<&'static str>, egui::Rect)> = None;
                let mut i = 0;
                while i < all.len() {
                    if let Some(cat) = all[i].0 {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                        // A long name is cut to its first four characters in the single column.
                        let cat = tl!(cat);
                        let label = if cols == 1 && cat.chars().count() > 6 {
                            format!("{}...", cat.chars().take(4).collect::<String>())
                        } else {
                            cat.to_string()
                        };
                        ui.painter().text(r.center() + vec2(0.0, 2.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), t.text);
                    }
                    // One row = `cols` slots (a category label always starts a new row).
                    let mut row = vec![i];
                    while row.len() < cols && i + row.len() < all.len() && all[i + row.len()].0.is_none() {
                        row.push(i + row.len());
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if cols == 1 {
                            ui.add_space((WIDTH - 36.0) / 2.0);
                        } else {
                            ui.add_space(2.0);
                        }
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
                            let well = egui::Rect::from_center_size(rect.center(), vec2(35.5, 27.5));
                            if is_active {
                                ui.painter().rect_filled(well, CornerRadius::same(1), t.tool_active);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(well, CornerRadius::same(1), t.hover);
                            }
                            let ir = egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
                            icons::paint(ui, icons::tool_icon(shown.icon), ir, if is_active { t.text_strong } else { t.icon });
                            if slot.len() > 1 {
                                let c = rect.center() + vec2(12.5, 10.0);
                                ui.painter().add(egui::Shape::convex_polygon(
                                    vec![c, c + vec2(-3.5, 0.0), c + vec2(0.0, -3.5)],
                                    t.icon,
                                    Stroke::NONE,
                                ));
                            }
                            let long_press = resp.is_pointer_button_down_on()
                                && ui.input(|inp| inp.pointer.press_start_time().is_some_and(|s| inp.time - s > 0.35));
                            let alt = ui.input(|inp| inp.modifiers.alt);
                            if (resp.secondary_clicked() || long_press) && slot.len() > 1 {
                                open_flyout = Some((slot.clone(), rect));
                                if long_press {
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
                            resp.on_hover_text(tip(shown));
                        }
                    });
                    i += row.len();
                }
                if let Some((slot, rect)) = open_flyout {
                    app.ui.flyout = Some(0);
                    ui.data_mut(|d| {
                        d.insert_temp(egui::Id::new("flyout-anchor"), rect);
                        d.insert_temp(egui::Id::new("flyout-tools"), slot.iter().map(|s| s.to_string()).collect::<Vec<String>>());
                    });
                }
                ui.add_space(8.0);
                bottom_controls(app, ui, &t);
            });
        });
    // Panels dragged out of the dock lock beside it.
    crate::floating::set_zone(ui.ctx(), crate::floating::Zone::Toolbar, shown.response.rect);
    flyout(app, ui.ctx());
}

/// Open a tool's options (`tool.options`, a double-click on its button): the Gradient tool's are
/// the Gradient panel, the Eyedropper's the Eyedropper Options dialog, a Liquify tool's its Tool
/// Options dialog, the Blend tool's Blend
/// Options; the Print Tiling tool's resets the print tiling.
pub fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<serde_json::Value, String> {
    match tool {
        "gradient" if app.ui.open_panel.as_deref() == Some("gradient") => Ok(json!({ "open": "gradient" })),
        "gradient" => app.run("window.panel", json!({ "panel": "gradient" })),
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
        // A double click on the Print Tiling tool puts the pages back where the placement puts them.
        "printTiling" => app.run("print.tiling.set", json!({ "reset": true })),
        // The Liquify tools: their Tool Options (the Global Brush Dimensions and the tool's own).
        _ if vectorcraft_tools::settings::LIQUIFY.contains(&tool) => crate::dialogs::liquify::open(app, tool),
        _ if vectorcraft_tools::tool_info(tool).is_none() => Err(format!("unknown tool `{tool}`")),
        _ => Err(format!("the {tool} tool has no options")),
    }
}

/// The active tool's options in the Control bar: Mirror & Cut's axis and the side it keeps, Puppet
/// Warp's mesh and pins (set through `tool.setOption`).
pub fn control_bar_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    /// (value, label) of each choice.
    type Choices = &'static [(&'static str, &'static str)];
    const MIRROR: [(&str, &str, Choices); 2] = [
        ("axis", "Axis:", &[("free", "Free"), ("vertical", "Vertical"), ("horizontal", "Horizontal")]),
        ("keep", "Keep:", &[("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom")]),
    ];
    if app.session.tool_id() == "puppetWarp" {
        return puppet_warp_options(app, ui);
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

fn bottom_controls(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens) {
    ui.vertical_centered(|ui| crate::panels::proxy(app, ui, 36.0));
    ui.add_space(5.0);
    // Color (the last solid colour), Gradient (the last gradient) and None, as commands.
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 27.0) / 2.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut clicked = None;
        for (tip, cmd) in [("Color (,)", "paint.lastColor"), ("Gradient (.)", "paint.lastGradient"), ("None (/)", "paint.none")] {
            let (r, resp) = ui.allocate_exact_size(vec2(7.5, 7.5), Sense::click());
            match cmd {
                "paint.lastColor" => widgets::paint_chip(ui, r, &Paint::solid(app.session.last_solid)),
                "paint.lastGradient" => widgets::gradient_chip(ui, r, &app.session.last_gradient.gradient),
                _ => widgets::paint_chip(ui, r, &Paint::None),
            }
            if resp.on_hover_text(tl!(tip)).clicked() {
                clicked = Some(cmd);
            }
        }
        if let Some(cmd) = clicked {
            app.run(cmd, json!({})).ok();
        }
    });
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        let modes = ["dc-draw-normal", "dc-draw-behind", "dc-draw-inside"];
        let names = ["Draw Normal", "Draw Behind", "Draw Inside"];
        let m = match app.session.draw_mode {
            vectorcraft_engine::DrawMode::Normal => 0,
            vectorcraft_engine::DrawMode::Behind => 1,
            vectorcraft_engine::DrawMode::Inside => 2,
        };
        if widgets::icon_button(ui, modes[m], &format!("{} (Shift+D)", tl!(names[m])), m != 0, 26.0).clicked() {
            app.run("view.drawMode", json!({})).ok();
        }
        if widgets::icon_button(ui, "dc-screen-mode", "Change Screen Mode (F)", false, 26.0).clicked() {
            app.run("view.screenMode", json!({})).ok();
        }
        if widgets::icon_button(ui, "ellipsis", "Edit Toolbar", false, 26.0).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::new("allTools", json!({})));
        }
    });
    let _ = t;
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
    let mut chosen = None;
    let resp = egui::Area::new(egui::Id::new("tool-flyout")).order(egui::Order::Foreground).fixed_pos(anchor.right_top() + vec2(8.0, -1.0)).show(
        ctx,
        |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .stroke(Stroke::new(1.0, t.input_border))
                .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(90) })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let w = 250.0;
                    for id in &tools {
                        let Some(tool) = tool_info(id) else { continue };
                        let active = app.session.tool_id() == tool.id;
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, 0.0, t.hover);
                        }
                        if active {
                            ui.painter().rect_filled(egui::Rect::from_center_size(r.left_center() + vec2(12.0, 0.0), vec2(5.0, 5.0)), 0.0, t.text);
                        }
                        let ir = egui::Rect::from_min_size(r.min + vec2(24.0, 6.0), vec2(18.0, 18.0));
                        icons::paint(ui, icons::tool_icon(tool.icon), ir, t.icon);
                        let color = if active { t.flyout_active } else { t.text_strong };
                        ui.painter().text(
                            r.left_center() + vec2(52.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            tl!(tool.label),
                            egui::FontId::proportional(13.0),
                            color,
                        );
                        if let Some(sc) = crate::shortcut_editor::tool_shortcut(tool.id) {
                            ui.painter().text(
                                r.right_center() - vec2(18.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                format!("({sc})"),
                                egui::FontId::proportional(13.0),
                                color,
                            );
                        }
                        if resp.clicked() {
                            chosen = Some(tool.id);
                        }
                    }
                });
        },
    );
    if let Some(id) = chosen {
        app.select_tool(id);
    } else if resp.response.clicked_elsewhere() && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| anchor.contains(p))) {
        // A click on the flyout's own tool button (the right-click that opened it, or the end of a
        // long press) leaves it to that button.
        app.ui.flyout = None;
    }
    let _ = theme::semibold;
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
    use egui::{Event, PointerButton, Pos2};
    use vectorcraft_engine::Session;

    use super::*;

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
        assert!(app.run("tool.options", json!({"tool": "zoom"})).is_err());
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
}
