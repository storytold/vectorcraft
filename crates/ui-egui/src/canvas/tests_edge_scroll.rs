//! A tool drag held near an edge of the canvas scrolls the view so the drag can go on past what
//! shows (#8551): a Selection tool move carries the art along, a drawing tool's shape grows, and
//! the view stops once the pointer leaves the edge. A click near the edge scrolls nothing.

use super::view::{EDGE_SCROLL_SPEED, EDGE_SCROLL_ZONE, edge_scroll, edge_scroll_pixels};
use super::*;
use vectorcraft_engine::Session;

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
    let ctx = egui::Context::default();
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

#[test]
fn edge_scroll_moves_the_content_away_from_the_edge_the_pointer_is_near() {
    let r = egui::Rect::from_min_size(pos2(100.0, 50.0), vec2(600.0, 400.0));
    let dt = 0.1;
    let full = EDGE_SCROLL_SPEED * dt;
    // Away from the edges, or off the canvas, nothing.
    assert_eq!(edge_scroll(r, r.center(), dt), None);
    assert_eq!(edge_scroll(r, pos2(r.right() + 5.0, r.center().y), dt), None, "over a panel");
    assert_eq!(edge_scroll(r, pos2(r.right() - EDGE_SCROLL_ZONE, r.center().y), dt), None, "at the zone's inner side");
    assert_eq!(edge_scroll(r, pos2(r.right() - 1.0, r.center().y), 0.0), None, "no time passed");
    assert_eq!(edge_scroll(r, pos2(f32::NAN, r.center().y), dt), None);
    // At the right edge the content moves left at full speed; half way into the zone, at half.
    let right = edge_scroll(r, pos2(r.right(), r.center().y), dt).unwrap();
    assert!((right.x + full).abs() < 1e-3 && right.y == 0.0, "{right:?}");
    let half = edge_scroll(r, pos2(r.right() - EDGE_SCROLL_ZONE / 2.0, r.center().y), dt).unwrap();
    assert!((half.x + full / 2.0).abs() < 1e-3, "{half:?}");
    // Near the left and top edges it moves right and down; in a corner, both ways.
    assert!(edge_scroll(r, pos2(r.left() + 1.0, r.center().y), dt).unwrap().x > 0.0);
    let corner = edge_scroll(r, pos2(r.left() + 1.0, r.top() + 1.0), dt).unwrap();
    assert!(corner.x > 0.0 && corner.y > 0.0, "{corner:?}");
    // In a corner both ways, but no faster than at one edge.
    let corner = edge_scroll(r, pos2(r.right() - 1.0, r.bottom() - 1.0), dt).unwrap();
    assert!(corner.x < 0.0 && corner.y < 0.0 && corner.length() <= full + 1e-3, "{corner:?}");
    assert!(edge_scroll(r, pos2(r.center().x, r.bottom() - 1.0), dt).unwrap().y < 0.0);
    // A stalled frame scrolls a tenth of a second's worth at most.
    assert_eq!(edge_scroll(r, pos2(r.right(), r.center().y), 5.0), Some(right));
}

/// The view moves by whole device pixels, keeping the rest for the press's next scroll; an axis
/// the scroll does not move keeps its remainder untouched rather than rounding it each frame.
#[test]
fn edge_scroll_pixels_keep_the_remainder_of_an_axis_that_does_not_scroll() {
    // 10.5 pixels: 11 now (rounded away from zero), half a pixel back owed.
    let (d, rest) = edge_scroll_pixels((0.0, 0.0), vec2(-10.5, 0.0), 1.0);
    assert_eq!(d, (-11.0, 0.0));
    assert_eq!(rest, (0.5, 0.0));
    // Along the top edge only: x keeps its half pixel, frame after frame, and does not move.
    let mut rest = rest;
    for _ in 0..4 {
        let (d, r) = edge_scroll_pixels(rest, vec2(0.0, 7.0), 1.0);
        assert_eq!(d, (0.0, 7.0));
        assert_eq!(r.0, 0.5, "x keeps its remainder");
        rest = r;
    }
    // Scrolling that way again, it counts: 0.5 + 1.5 = 2 pixels.
    assert_eq!(edge_scroll_pixels(rest, vec2(1.5, 0.0), 1.0).0, (2.0, 0.0));
    // At 2 device pixels a point.
    assert_eq!(edge_scroll_pixels((0.0, 0.0), vec2(0.25, -1.0), 2.0), ((1.0, -2.0), (-0.5, 0.0)));
}

/// The issue's case: zoomed in, a selected object dragged towards the panels on the right reaches
/// the canvas's edge. Held there, the view scrolls right and the object goes on with the pointer;
/// back away from the edge, the view stops.
#[test]
fn a_move_held_near_the_right_edge_scrolls_the_view_and_carries_the_art() {
    let (mut app, ctx, id) = zoomed_in("selection");
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let start = xf.to_screen(Point::new(100.0, 100.0));
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
    let rect = app.canvas_rect.unwrap();
    // Away from the selected square's centre point, where a press moves the square (#5477).
    let start = rect.center() - vec2(150.0, 100.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(10.0, 10.0))]);
    let edge = pos2(start.x + 10.0, rect.bottom() - 2.0);
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
        let now = Xf::new(rect, app.view().unwrap()).to_doc(edge).y;
        assert!((preview_bottom(&app) - now).abs() < 0.5, "the preview follows the pointer: {} vs {now}", preview_bottom(&app));
    }
    let c1 = center(&app);
    assert!(c1.y > c0.y && (c1.x - c0.x).abs() < 1e-9, "the view scrolls down: {c0:?} then {c1:?}");
    let xf = Xf::new(rect, app.view().unwrap());
    let bottom = xf.to_doc(edge).y;
    frame(&mut app, &ctx, vec![button(edge, false)]);
    let st = app.session.active().unwrap();
    let made = st.selection.objects.first().and_then(|id| st.doc.node(*id)).and_then(|n| n.geometric_bounds()).unwrap();
    assert!((made.y1 - bottom).abs() < 0.5, "the rectangle reaches the pointer in the scrolled view: {} vs {bottom}", made.y1);
}

/// A press near the edge that has not dragged is a click: the view stays.
#[test]
fn a_press_held_still_near_the_edge_does_not_scroll() {
    let (mut app, ctx, _) = zoomed_in("selection");
    let rect = app.canvas_rect.unwrap();
    let at = pos2(rect.right() - 4.0, rect.center().y);
    let c0 = center(&app);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), button(at, true)]);
    for _ in 0..4 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(center(&app), c0);
    frame(&mut app, &ctx, vec![button(at, false)]);
    assert_eq!(center(&app), c0);
}

/// The Hand tool's own drag is not a tool drag: held near the edge it pans with the pointer only.
#[test]
fn a_hand_drag_held_near_the_edge_does_not_scroll_on() {
    let (mut app, ctx, _) = zoomed_in("hand");
    let rect = app.canvas_rect.unwrap();
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

/// A press held still near the edge while the wheel scrolls the view is not a drag: the pointer
/// has not moved on screen, so the view does not go on scrolling after the wheel stops.
#[test]
fn a_wheel_scroll_under_a_still_press_near_the_edge_does_not_start_edge_scrolling() {
    let (mut app, ctx, _) = zoomed_in("selection");
    let rect = app.canvas_rect.unwrap();
    let at = pos2(rect.right() - 4.0, rect.center().y);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), button(at, true)]);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, 40.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    let c0 = center(&app);
    frame(&mut app, &ctx, vec![wheel]);
    let wheeled = center(&app);
    assert_ne!(wheeled, c0, "the wheel scrolls the view");
    for _ in 0..4 {
        frame(&mut app, &ctx, vec![]);
    }
    // The wheel's own scroll eases out along y over these frames; the right edge would scroll x.
    assert_eq!(center(&app).x, wheeled.x, "no edge scrolling");
    frame(&mut app, &ctx, vec![button(at, false)]);
}

/// The press is where the button went down, not where the frame's input left the pointer: a press
/// well inside the canvas and a move to the edge in the same input batch has dragged, so holding
/// the pointer still there scrolls.
#[test]
fn a_press_and_a_move_to_the_edge_in_one_frame_scroll_once_held() {
    let (mut app, ctx, _) = zoomed_in("selection");
    let rect = app.canvas_rect.unwrap();
    let start = pos2(rect.center().x, rect.bottom() - 30.0);
    let edge = pos2(start.x, rect.bottom() - 4.0);
    let c0 = center(&app);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true), egui::Event::PointerMoved(edge)]);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert!(center(&app).y > c0.y, "the view scrolls down: {:?} vs {c0:?}", center(&app));
    frame(&mut app, &ctx, vec![button(edge, false)]);
}

/// A move away from the press and back within one frame still dragged: the press held near the
/// edge then scrolls, though the pointer ends the frame where it was pressed.
#[test]
fn a_move_away_and_back_within_a_frame_still_arms_edge_scrolling() {
    let (mut app, ctx, _) = zoomed_in("selection");
    let rect = app.canvas_rect.unwrap();
    let at = pos2(rect.right() - 4.0, rect.center().y);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), button(at, true)]);
    let c0 = center(&app);
    let away = at - vec2(8.0, 0.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(away), egui::Event::PointerMoved(at + vec2(-0.5, 0.0))]);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert!(center(&app).x > c0.x, "the view scrolls right: {:?} vs {c0:?}", center(&app));
    frame(&mut app, &ctx, vec![button(at, false)]);
}
