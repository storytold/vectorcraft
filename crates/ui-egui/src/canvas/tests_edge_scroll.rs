//! A tool drag held near an edge of the canvas scrolls the view so the drag can go on past what
//! shows (#8551): a Selection tool move carries the art along, a drawing tool's shape grows, and
//! the view stops once the pointer leaves the edge. The Hand tool's drag pans and scrolls no further.

use super::*;
use egui::{Pos2, pos2, vec2};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

/// One headless canvas frame on an 800 × 600 window.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// A 400 × 300 document with a 100 pt square at (50, 50), zoomed in to 4× about the square's
/// centre, with `tool` selected; the frame drawn once.
fn zoomed_in(tool: &str) -> (VectorcraftApp, egui::Context, vectorcraft_doc::NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.select_tool(tool);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let v = app.view_mut().unwrap();
    v.zoom = 4.0;
    v.center = Point::new(100.0, 100.0);
    frame(&mut app, &ctx, vec![]);
    (app, ctx, vectorcraft_doc::NodeId(id))
}

fn center(app: &VectorcraftApp) -> Point {
    app.view().unwrap().center
}

/// The issue's case: zoomed in, a selected object dragged towards the panels on the right reaches
/// the canvas's edge. Held there, the view scrolls right and the object goes on with the pointer;
/// back away from the edge, the view stops.
#[test]
fn a_move_held_near_the_right_edge_scrolls_the_view_and_carries_the_art() {
    let (mut app, ctx, id) = zoomed_in("selection");
    let view = app.canvas_view().unwrap();
    let rect = view.rect;
    let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let start = view.to_screen(Point::new(100.0, 100.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(20.0, 0.0))]);
    let edge = pos2(rect.right() - 4.0, start.y);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
    let (c0, x0) = (center(&app), left(&app));
    // Held still at the edge, frame after frame the view scrolls right and the art keeps up.
    let mut last = (c0, x0);
    for _ in 0..5 {
        frame(&mut app, &ctx, vec![]);
        let now = (center(&app), left(&app));
        assert!(now.0.x > last.0.x, "the view scrolls right: {:?} then {:?}", last.0, now.0);
        assert!((now.0.y - c0.y).abs() < 1e-9, "and only right");
        assert!(now.1 > last.1, "the art moves with the pointer: {} then {}", last.1, now.1);
        last = now;
    }
    // The art stays under the pointer: it moved as far as the view (give or take the move's
    // rounding to the pixel grid).
    assert!(((last.1 - x0) - (last.0.x - c0.x)).abs() < 1.0, "art {} view {}", last.1 - x0, last.0.x - c0.x);
    assert!(app.session.active().unwrap().interaction.is_some(), "still the same move");
    // Back away from the edge the view stops.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(rect.center())]);
    let stopped = center(&app);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(center(&app), stopped, "the view stops away from the edge");
    // The release ends the move where the art is.
    let moved = left(&app);
    frame(&mut app, &ctx, vec![button(rect.center(), false)]);
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none());
    assert!((left(&app) - moved).abs() < 1e-6);
    assert!(moved > 50.0 + (last.0.x - c0.x), "the art went past where the edge showed at first");
}

/// A drawing tool's drag scrolls too: a rectangle dragged to the bottom edge goes on growing.
#[test]
fn a_rectangle_drag_held_near_the_bottom_edge_keeps_growing() {
    let (mut app, ctx, _) = zoomed_in("rectangle");
    let rect = app.canvas_view().unwrap().rect;
    // Away from the selected square's centre point, where a press moves the square (#5477).
    let start = rect.center() - vec2(150.0, 100.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(10.0, 10.0))]);
    // Just past the bottom edge: the canvas scrolls for a drag that goes past its edge.
    let edge = pos2(start.x + 10.0, rect.bottom() + 2.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
    let c0 = center(&app);
    // The bottom of the rectangle drawn so far: the preview in the document.
    let preview_bottom = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let mut bottom = f64::MIN;
        st.doc.walk(|n| {
            if let Some(b) = n.geometric_bounds() {
                bottom = bottom.max(b.y1);
            }
        });
        bottom
    };
    for _ in 0..4 {
        frame(&mut app, &ctx, vec![]);
        // Each held frame the preview reaches the pointer in the view as it is now.
        let now = app.canvas_view().unwrap().to_doc(edge).y;
        assert!((preview_bottom(&app) - now).abs() < 0.5, "the preview follows the pointer: {} vs {now}", preview_bottom(&app));
    }
    let c1 = center(&app);
    assert!(c1.y > c0.y && (c1.x - c0.x).abs() < 1e-9, "the view scrolls down: {c0:?} then {c1:?}");
    let bottom = app.canvas_view().unwrap().to_doc(edge).y;
    frame(&mut app, &ctx, vec![button(edge, false)]);
    let st = app.session.active().unwrap();
    let made = st.selection.objects.first().and_then(|id| st.doc.node(*id)).and_then(|n| n.geometric_bounds()).unwrap();
    assert!((made.y1 - bottom).abs() < 0.5, "the rectangle reaches the pointer in the scrolled view: {} vs {bottom}", made.y1);
}

/// The Hand tool's own drag is not a tool drag: held near the edge it pans with the pointer only.
#[test]
fn a_hand_drag_held_near_the_edge_does_not_scroll_on() {
    let (mut app, ctx, _) = zoomed_in("hand");
    let rect = app.canvas_view().unwrap().rect;
    let start = rect.center();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    let edge = pos2(rect.right() - 4.0, start.y);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
    let c0 = center(&app);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(center(&app), c0);
    frame(&mut app, &ctx, vec![button(edge, false)]);
}
