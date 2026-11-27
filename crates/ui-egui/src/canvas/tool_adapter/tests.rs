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

/// The Blob Brush's or the Eraser's tip outline at the pointer, as the canvas draws it this pass.
fn shows_tip(app: &mut VectorcraftApp) -> bool {
    let view = app.view_info();
    app.session.overlays(view).iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Path { .. }))
}

/// Use Precise Cursors or Caps Lock, or both, gives the Blob Brush and the Eraser only the crosshair: the tool draws no
/// tip outline around it. With both off, each shows its own pointer and the tip outline.
#[test]
fn precise_cursors_hide_the_brush_tip_outline() {
    use craft::ui::runtime::windows::WindowHost;
    let crosshair = Some(crate::cursors::tool_cursor(vectorcraft_tools::Cursor::Crosshair));
    for tool in ["blobBrush", "eraser"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
        app.select_tool(tool);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx, vec![]);
        let at = app.canvas_view().unwrap().to_screen(Point::new(200.0, 150.0));
        for (preference, caps_lock) in [(false, false), (true, false), (false, true), (true, true), (false, false)] {
            app.run("prefs.set", serde_json::json!({"key": "usePreciseCursors", "value": preference})).unwrap();
            WindowHost::of(&ctx).set_caps_lock(Some(caps_lock));
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at + egui::vec2(1.0, 0.0))]);
            let precise = preference || caps_lock;
            let case = format!("{tool}, Use Precise Cursors {preference}, Caps Lock {caps_lock}");
            assert_eq!(app.session.caps_lock, caps_lock, "{case}: the session takes the host's Caps Lock state");
            assert_eq!(app.view_info().precise, precise, "{case}");
            assert_eq!(shows_tip(&mut app), !precise, "{case}");
            assert_eq!(craft::ui::runtime::cursor::current(&ctx) == crosshair, precise, "{case}");
        }
    }
}

/// `view.capsLock` sets the Caps Lock state the tools and the canvas's pointer follow until the host's state changes.
#[test]
fn the_caps_lock_command_gives_the_brushes_the_crosshair() {
    use craft::ui::runtime::windows::WindowHost;
    let crosshair = Some(crate::cursors::tool_cursor(vectorcraft_tools::Cursor::Crosshair));
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("blobBrush");
    let ctx = crate::testing::ui_context();
    WindowHost::of(&ctx).set_caps_lock(Some(false));
    frame(&mut app, &ctx, vec![]);
    let at = app.canvas_view().unwrap().to_screen(Point::new(200.0, 150.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    assert!(shows_tip(&mut app));
    app.run("view.capsLock", serde_json::json!({"on": true})).unwrap();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at + egui::vec2(1.0, 0.0))]);
    assert!(app.session.caps_lock, "the host's unchanged state leaves the command's in place");
    assert!(!shows_tip(&mut app));
    assert_eq!(craft::ui::runtime::cursor::current(&ctx), crosshair);
    app.run("view.capsLock", serde_json::json!({"on": false})).unwrap();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    assert!(shows_tip(&mut app));
    assert_ne!(craft::ui::runtime::cursor::current(&ctx), crosshair);
    // With the host's Caps Lock on, the command turns it off for the tools and the canvas's pointer alike.
    WindowHost::of(&ctx).set_caps_lock(Some(true));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at + egui::vec2(1.0, 0.0))]);
    assert!(!shows_tip(&mut app));
    assert_eq!(craft::ui::runtime::cursor::current(&ctx), crosshair);
    app.run("view.capsLock", serde_json::json!({"on": false})).unwrap();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    assert!(!app.session.caps_lock);
    assert!(shows_tip(&mut app));
    assert_ne!(craft::ui::runtime::cursor::current(&ctx), crosshair);
}

/// A `view.capsLock` call that runs before the first canvas pass stays in place when the host's Caps Lock is off.
#[test]
fn the_caps_lock_command_before_the_first_frame_stays_in_place() {
    use craft::ui::runtime::windows::WindowHost;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("blobBrush");
    app.run("view.capsLock", serde_json::json!({"on": true})).unwrap();
    let ctx = crate::testing::ui_context();
    WindowHost::of(&ctx).set_caps_lock(Some(false));
    frame(&mut app, &ctx, vec![]);
    assert!(app.session.caps_lock, "the first host reading is only the baseline");
    frame(&mut app, &ctx, vec![]);
    assert!(app.session.caps_lock);
}
