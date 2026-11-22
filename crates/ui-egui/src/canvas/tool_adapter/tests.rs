use craft::behavior::{CanvasView, PointerSample};
use craft::ui::canvas::{CanvasSpace, ToolContext, ToolPointer};
use vectorcraft_engine::Session;
use vectorcraft_tools::PointerKind;

use super::*;
use crate::VectorcraftApp;

fn context(view: CanvasView) -> ToolContext<'static> {
    ToolContext::new(view.rect, Some(CanvasSpace::new(view, 1.0)))
}

fn view() -> CanvasView {
    CanvasView::new(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0)), Point::new(100.0, 100.0), 2.0)
}

/// The event carries the document point under the pointer and its pressure, whatever its kind.
#[test]
fn a_pointer_event_carries_the_document_point_and_pressure() {
    let view = view();
    let cx = context(view);
    let at = egui::pos2(250.0, 175.0);
    let pointer = ToolPointer::new(at, craft::behavior::PointerButton::Primary, 0.0).sample(PointerSample::new(0.5));
    for kind in [PointerKind::Down, PointerKind::Drag, PointerKind::Up, PointerKind::Move, PointerKind::DoubleClick, PointerKind::TripleClick] {
        let ev = pointer_event(kind, &pointer, &cx);
        assert_eq!(ev.kind, kind);
        assert_eq!(ev.pos, view.to_doc(at));
        assert_eq!(ev.pressure, 0.5);
        assert_eq!(ev.pen, None, "a mouse has no pose");
    }
}

/// A pick that consumed a press is not a click of the run: pick, then a single, then a double.
#[test]
fn a_consumed_pick_does_not_count_toward_the_click_run() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui.trace_pick_clicks = 1;
    assert_eq!(run_clicks(&mut app, 2), 1);
    assert_eq!(run_clicks(&mut app, 3), 2);
    assert_eq!(run_clicks(&mut app, 1), 1, "a lone press starts a new run");
    assert_eq!(app.ui.trace_pick_clicks, 0);
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::canvas::show(app, ui));
    out.textures_delta.clear();
}

/// With the Live Paint Bucket and the pointer still, an Alt-only pass changes the cursor and back.
#[test]
fn bucket_modifier_only_hover_updates_the_visible_cursor() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("livePaintBucket");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let at = app.canvas_view().unwrap().to_screen(Point::new(200.0, 150.0));
    let shown = |app: &mut VectorcraftApp, mods: egui::Modifiers| {
        frame(app, &ctx, vec![egui::Event::ModifiersChanged(mods)]);
        craft::ui::runtime::cursor::current(&ctx)
    };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    let plain = craft::ui::runtime::cursor::current(&ctx);
    let alt = shown(&mut app, egui::Modifiers::ALT);
    assert_ne!(alt, plain, "Alt shows the Eyedropper's cursor");
    assert_eq!(shown(&mut app, egui::Modifiers::NONE), plain, "and letting go brings the bucket back");
}
