//! The Free Transform widget (#598): while the Free Transform tool is active, a small floating box
//! at the canvas's top left with Constrain (as holding Shift: proportional scaling, moves and
//! rotations by 45°) over the tool's three modes, Free Transform, Perspective Distort and Free
//! Distort. It sets the tool's options through `tool.setOption` (`constrain`, `mode`), as agents
//! do; the modifier keys keep working while dragging.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The tool the widget belongs to.
const TOOL: &str = "freeTransform";
/// A button's side.
const BUTTON: f32 = 28.0;

/// What a button shows.
#[derive(Clone, Copy)]
enum Glyph {
    /// An icon of the set.
    Icon(&'static str),
    /// A box narrower at the top than at the bottom, its corners marked.
    Perspective,
    /// A box with corners pulled every which way, its corners marked.
    Distort,
}

/// The tool's modes as its `mode` option names them, with their buttons.
fn modes() -> [(&'static str, Glyph, &'static str); 3] {
    [
        ("free", Glyph::Icon("dc-free-transform"), tl!("Free Transform")),
        ("perspective", Glyph::Perspective, tl!("Perspective Distort")),
        ("distort", Glyph::Distort, tl!("Free Distort")),
    ]
}

/// Where the widget was last drawn (none while it isn't shown).
pub(crate) fn rect(ctx: &egui::Context) -> Option<Rect> {
    let id = egui::Id::unique(TOOL);
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// Show the widget in `canvas` (the canvas's area inside the rulers) while the Free Transform
/// tool is active.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &Ui, canvas: Rect) {
    if app.session.tool_id() != TOOL {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let opts = app.session.tool_options();
    let mode = opts["mode"].as_str().unwrap_or("free").to_string();
    let constrain = opts["constrain"].as_bool().unwrap_or(false);
    let mut set: Option<(&str, Value)> = None;
    egui::Area::new(egui::Id::unique(TOOL)).order(egui::Order::Middle).fixed_pos(canvas.min + vec2(10.0, 10.0)).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.button_border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(3))
            .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(60) })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // Constrain shapes only what the Free Transform mode does.
                if button(ui, Glyph::Icon("link"), tl!("Constrain"), constrain, mode == "free") {
                    set = Some(("constrain", json!(!constrain)));
                }
                let (line, _) = ui.allocate_exact_size(vec2(BUTTON, 3.0), Sense::hover());
                ui.painter().hline(line.x_range().shrink(4.0), line.center().y, Stroke::new(1.0, t.button_border));
                for (name, glyph, tip) in modes() {
                    if button(ui, glyph, tip, mode == name, true) && mode != name {
                        set = Some(("mode", json!(name)));
                    }
                }
            });
    });
    if let Some((key, value)) = set
        && let Err(e) = app.run("tool.setOption", json!({ "key": key, "value": value }))
    {
        app.status(e);
    }
}

/// A square button showing `glyph`, highlighted when `on`, greyed and inert unless `enabled`.
/// Returns whether it was clicked.
fn button(ui: &mut Ui, glyph: Glyph, tip: &str, on: bool, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if on && enabled {
        t.tool_active
    } else if enabled && resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), bg);
    let color = match (enabled, on) {
        (false, _) => t.text_dim,
        (true, true) => t.text,
        (true, false) => t.icon,
    };
    paint(ui, glyph, rect.shrink(6.0), color);
    resp.on_hover_text(tip).clicked() && enabled
}

/// Draw `glyph` in `r`.
fn paint(ui: &Ui, glyph: Glyph, r: Rect, color: Color32) {
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let quad: [Pos2; 4] = match glyph {
        Glyph::Icon(name) => return crate::icons::paint(ui, name, r, color),
        Glyph::Perspective => [at(0.3, 0.12), at(0.7, 0.12), at(0.95, 0.88), at(0.05, 0.88)],
        Glyph::Distort => [at(0.08, 0.2), at(0.78, 0.06), at(0.94, 0.9), at(0.2, 0.76)],
    };
    let (p, fill) = (ui.painter(), Tokens::get(ui.ctx()).panel);
    p.add(egui::Shape::closed_line(quad.to_vec(), Stroke::new(1.3, color)));
    for c in quad {
        let mark = Rect::from_center_size(c, vec2(3.5, 3.5));
        p.rect_filled(mark, 0.0, fill);
        p.rect_stroke(mark, 0.0, Stroke::new(1.0, color), StrokeKind::Middle);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_geom::Point;

    use super::*;
    use crate::canvas::Xf;

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
        ctx.run_ui(raw, |ui| crate::canvas::show(app, ui)).textures_delta.clear();
    }

    /// A press and release at `at`, then a frame to settle.
    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(app, ctx, vec![button(false)]);
        frame(app, ctx, vec![]);
    }

    fn options(app: &VectorcraftApp) -> (String, bool) {
        let o = app.session.tool_options();
        (o["mode"].as_str().unwrap_or_default().to_string(), o["constrain"].as_bool().unwrap_or_default())
    }

    /// The widget shows with the Free Transform tool only; its buttons set the mode and Constrain
    /// (inert in the distort modes); a corner dragged in Free Distort moves that corner alone.
    #[test]
    fn the_widget_sets_the_mode_and_a_corner_drag_then_distorts() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let ctx = egui::Context::default();
        app.select_tool("selection");
        frame(&mut app, &ctx, vec![]);
        assert!(rect(&ctx).is_none(), "not with the Selection tool");
        app.select_tool(TOOL);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let w = rect(&ctx).expect("the widget shows");
        let canvas = app.canvas_rect.unwrap();
        assert!(canvas.contains(w.min) && w.left() - canvas.left() < 20.0 && w.top() - canvas.top() < 20.0, "{w:?} in {canvas:?}");
        // Constrain, a rule, then Free Transform, Perspective Distort and Free Distort.
        let at = |i: usize| {
            let rule = if i > 0 { 3.0 + 2.0 } else { 0.0 };
            pos2(w.center().x, w.top() + 4.0 + BUTTON / 2.0 + i as f32 * (BUTTON + 2.0) + rule)
        };
        assert_eq!(options(&app), ("free".into(), false));
        click(&mut app, &ctx, at(0));
        assert_eq!(options(&app), ("free".into(), true), "Constrain on");
        click(&mut app, &ctx, at(3));
        assert_eq!(options(&app), ("distort".into(), true));
        click(&mut app, &ctx, at(0));
        assert_eq!(options(&app), ("distort".into(), true), "Constrain is inert in Free Distort");
        click(&mut app, &ctx, at(2));
        assert_eq!(options(&app).0, "perspective");
        click(&mut app, &ctx, at(3));
        // Free Distort: the top-left corner dragged alone.
        let xf = Xf::new(canvas, app.view().unwrap());
        let (from, to) = (xf.to_screen(Point::new(100.0, 100.0)), xf.to_screen(Point::new(80.0, 90.0)));
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + (to - from) / 2.0)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
        frame(&mut app, &ctx, vec![button(to, false)]);
        frame(&mut app, &ctx, vec![]);
        let st = app.session.active().unwrap();
        let node = st.doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let pts: Vec<Point> = node.path_data().unwrap().anchors().map(|(.., a)| a.p).collect();
        let near = |q: Point| pts.iter().any(|p| p.distance(q) < 0.5);
        assert!(
            near(Point::new(80.0, 90.0)) && near(Point::new(200.0, 100.0)) && near(Point::new(200.0, 160.0)) && near(Point::new(100.0, 160.0)),
            "{pts:?}"
        );
        assert_eq!(app.session.tool_id(), TOOL);
    }
}
