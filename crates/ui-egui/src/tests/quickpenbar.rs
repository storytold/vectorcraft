//! The Quick Pen toolbar: it shows while the Quick Pen tool is active, its buttons set the tool's
//! action and flip the last sharp-corner arc, it follows the drawing beside the path's last point,
//! and its More Options menu's commands hide it, pin it and reset its place.

use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{Point, SubPath};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::{VectorcraftApp, canvas};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("quickPen");
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// One frame of the canvas; returns the buttons it drew, by name, from the accessibility tree.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect, bool)> {
    frame_in(app, ctx, events, vec2(1000.0, 700.0))
}

/// [`frame`] in a window of `size`.
fn frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, size: egui::Vec2) -> Vec<(String, Rect, bool)> {
    let screen = Rect::from_min_size(Pos2::ZERO, size);
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    let Some(update) = out.platform_output.accesskit_update.as_ref() else { return vec![] };
    update
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let r = Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32));
            Some((n.label()?.to_string(), r, !n.is_disabled()))
        })
        .collect()
}

/// Two frames, for the toolbar's area to settle; returns the second frame's buttons.
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, Rect, bool)> {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![])
}

fn bar(ctx: &egui::Context) -> Option<Rect> {
    canvas::quick_pen_bar_rect(ctx)
}

/// Click the toolbar's button named `name`, which must be enabled.
fn click_button(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    let named = settle(app, ctx);
    let (_, r, on) = named.iter().find(|(n, _, _)| n == name).unwrap_or_else(|| panic!("no button named {name:?} in {named:?}"));
    assert!(*on, "{name} is enabled");
    let p = r.center();
    let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(p)]);
    frame(app, ctx, vec![press(true)]);
    frame(app, ctx, vec![press(false)]);
    frame(app, ctx, vec![]);
}

/// A click of the tool at `x`, `y` in the document.
fn click_doc(app: &mut VectorcraftApp, x: f64, y: f64) {
    let v = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
}

fn paths(app: &VectorcraftApp) -> Vec<SubPath> {
    let mut v = vec![];
    app.session.active().unwrap().doc.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            v.extend(path.subpaths.iter().cloned());
        }
    });
    v
}

fn screen(app: &VectorcraftApp, x: f64, y: f64) -> Pos2 {
    app.canvas_xf().unwrap().to_screen(Point::new(x, y))
}

fn near(p: Pos2, q: Pos2) {
    assert!((p - q).length() < 1.5, "{p:?} is not at {q:?}");
}

/// The toolbar shows only with the Quick Pen tool, at the bottom center of the canvas while no path
/// is being drawn; its action buttons set the tool's action without drawing on the canvas under
/// them; Hide Toolbar and Window › Quick Pen Toolbar hide and show it.
#[test]
fn the_toolbar_shows_with_the_tool_and_its_buttons_set_the_action() {
    let mut app = app();
    let ctx = context();
    let named = settle(&mut app, &ctx);
    let r = bar(&ctx).expect("the toolbar shows with the Quick Pen tool");
    for name in ["Straight Line", "Continuous Arc", "Sharp Corner Arc", "Flip Arc", "More Options"] {
        assert!(named.iter().any(|(n, _, _)| n == name), "{name} in {named:?}");
    }
    assert!(!named.iter().any(|(n, _, on)| n == "Flip Arc" && *on), "Flip Arc waits for an arc");
    let canvas = app.canvas_xf().unwrap().rect;
    assert!((r.center().x - canvas.center().x).abs() < 1.5 && (r.bottom() - (canvas.bottom() - 8.0)).abs() < 1.5, "{r:?} in {canvas:?}");

    click_button(&mut app, &ctx, "Sharp Corner Arc");
    assert_eq!(app.session.tool_options()["action"], "sharpArc");
    click_button(&mut app, &ctx, "Continuous Arc");
    assert_eq!(app.session.tool_options()["action"], "continuousArc");
    click_button(&mut app, &ctx, "Straight Line");
    assert_eq!(app.session.tool_options()["action"], "line");
    assert!(paths(&app).is_empty(), "a click on the toolbar draws nothing");

    assert_eq!(app.run("window.quickPenToolbar", json!({})).unwrap(), json!(false));
    assert_eq!(crate::menus::checked(&app, "window.quickPenToolbar", &json!({})), Some(false));
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_none(), "hidden");
    assert_eq!(app.run("window.quickPenToolbar", json!({})).unwrap(), json!(true));
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some(), "shown again");

    app.select_tool("pen");
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_none(), "only with the Quick Pen tool");

    // Hidden, it shows again when the tool is selected again.
    app.select_tool("quickPen");
    settle(&mut app, &ctx);
    assert_eq!(app.run("window.quickPenToolbar", json!({})).unwrap(), json!(false));
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_none(), "hidden");
    app.select_tool("pen");
    app.select_tool("quickPen");
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some(), "selecting the tool shows it");
    assert_eq!(crate::menus::checked(&app, "window.quickPenToolbar", &json!({})), Some(true));
    // Selecting it again while it is active shows it too.
    app.run("window.quickPenToolbar", json!({})).unwrap();
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_none(), "hidden");
    app.run("tool.select", json!({"tool": "quickPen"})).unwrap();
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some(), "selected again");
}

/// The toolbar hides while a press drags on the canvas, and shows again once it is let go.
#[test]
fn the_toolbar_hides_while_a_press_drags_on_the_canvas() {
    let mut app = app();
    let ctx = context();
    // The first click places the path's start; the press that follows drags out a piece.
    click_doc(&mut app, 60.0, 60.0);
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some());
    let (a, b) = (screen(&app, 100.0, 100.0), screen(&app, 200.0, 140.0));
    let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(a), press(a, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(a + (b - a) / 2.0)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(b)]);
    assert!(app.session.tool_busy(), "the press reached the tool");
    assert!(bar(&ctx).is_none(), "hidden mid-drag");
    frame(&mut app, &ctx, vec![press(b, false)]);
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some(), "shown after the drag");
}

/// A middle-button drag on the canvas (a pan), which the tool takes no press from, hides the toolbar
/// too, and it shows again when the button is let go.
#[test]
fn the_toolbar_hides_while_the_canvas_pans() {
    let mut app = app();
    let ctx = context();
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some());
    let (a, b) = (screen(&app, 100.0, 100.0), screen(&app, 200.0, 140.0));
    let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Middle, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(a), press(a, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(a + (b - a) / 2.0)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(b)]);
    assert!(!app.session.tool_busy(), "the tool has no press under way");
    assert!(bar(&ctx).is_none(), "hidden mid-pan");
    frame(&mut app, &ctx, vec![press(b, false)]);
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some(), "shown after the pan");
}

/// On a canvas narrower than the toolbar, its controls wrap onto more rows, so every button lies
/// on the canvas.
#[test]
fn the_toolbar_wraps_on_a_narrow_canvas() {
    let mut app = app();
    let ctx = context();
    let size = vec2(130.0, 500.0);
    frame_in(&mut app, &ctx, vec![], size);
    let named = frame_in(&mut app, &ctx, vec![], size);
    let canvas = app.canvas_xf().unwrap().rect;
    assert!(canvas.width() < 150.0, "{canvas:?}");
    for name in ["Straight Line", "Continuous Arc", "Sharp Corner Arc", "Flip Arc", "More Options"] {
        let (_, r, _) = named.iter().find(|(n, _, _)| n == name).unwrap_or_else(|| panic!("{name} in {named:?}"));
        assert!(canvas.contains_rect(*r), "{name} at {r:?} is off the canvas {canvas:?}");
    }
}

/// Flip Arc's command sends C only to the Quick Pen tool: with the Arc tool, which takes C while it
/// drags, it returns false and changes nothing.
#[test]
fn flip_arc_does_nothing_with_another_tool() {
    let mut app = app();
    app.select_tool("arc");
    let v = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 120.0), v).unwrap();
    assert!(app.session.tool_claims_key(ToolKey::Letter('c'), v), "the Arc tool takes C mid-drag");
    let closed = app.session.tool_options()["closed"].clone();
    assert_eq!(app.run("tool.quickPen.flipArc", json!({})).unwrap(), json!(false));
    assert_eq!(app.session.tool_options()["closed"], closed, "the arc's C was not sent");
}

/// While a path is drawn, the toolbar sits 24 points off its last point, on the side the path
/// heads toward; Flip Arc flips the last sharp-corner arc as one undo step and is off after a line;
/// Escape sends the toolbar back to the bottom of the canvas.
#[test]
fn the_toolbar_follows_the_drawing_and_flips_the_last_arc() {
    let mut app = app();
    let ctx = context();
    settle(&mut app, &ctx);
    let resting = bar(&ctx).unwrap();
    app.run("tool.setOption", json!({"key": "action", "value": "sharpArc"})).unwrap();
    assert_eq!(app.run("tool.quickPen.flipArc", json!({})).unwrap(), json!(false), "no path being drawn");
    click_doc(&mut app, 100.0, 150.0);
    click_doc(&mut app, 200.0, 150.0);
    // A half circle above the chord, which arrives at (200, 150) heading down: right and below.
    let mid = |app: &VectorcraftApp| paths(app)[0].anchors[1].p;
    assert!(mid(&app).y < 150.0);
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, screen(&app, 200.0, 150.0) + vec2(24.0, 24.0));

    let undo = app.session.active().unwrap().history.undo.len();
    click_button(&mut app, &ctx, "Flip Arc");
    assert!(mid(&app).y > 150.0, "flipped below the chord");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);

    // A line drawn to the left: the toolbar goes to the left of its end, still below it.
    app.run("tool.setOption", json!({"key": "action", "value": "line"})).unwrap();
    click_doc(&mut app, 120.0, 150.0);
    let named = settle(&mut app, &ctx);
    let r = bar(&ctx).unwrap();
    let end = screen(&app, 120.0, 150.0);
    near(pos2_of(r.right(), r.top()), end + vec2(-24.0, 24.0));
    assert!(!named.iter().any(|(n, _, on)| n == "Flip Arc" && *on), "Flip Arc is off after a line");
    assert_eq!(app.run("tool.quickPen.flipArc", json!({})).unwrap(), json!(false));
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 2, "the line, and no flip");

    let v = app.view_info();
    app.session.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, resting.min);
}

fn pos2_of(x: f32, y: f32) -> Pos2 {
    Pos2::new(x, y)
}

/// The toolbar stays shown while its handle is dragged farther than the toolbar's height, when the
/// press is no longer on it.
#[test]
fn the_toolbar_stays_while_its_handle_drags_far() {
    let mut app = app();
    let ctx = context();
    settle(&mut app, &ctx);
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let grip = pos2_of(bar(&ctx).unwrap().left() + 10.0, bar(&ctx).unwrap().center().y);
    frame(&mut app, &ctx, vec![Event::PointerMoved(grip)]);
    frame(&mut app, &ctx, vec![button(grip, true)]);
    for k in 1..=6 {
        frame(&mut app, &ctx, vec![Event::PointerMoved(grip + vec2(-20.0, -30.0) * k as f32)]);
        assert!(bar(&ctx).is_some(), "shown at step {k}");
    }
    frame(&mut app, &ctx, vec![button(grip + vec2(-120.0, -180.0), false)]);
    settle(&mut app, &ctx);
    assert!(bar(&ctx).is_some());
}

/// The handle drags the toolbar, which keeps that offset as it follows the drawing, but not in its
/// resting place with no path being drawn; pinned, it stays put; Reset Toolbar Position puts it
/// back beside the drawing, unpinned.
#[test]
fn the_toolbar_drags_pins_and_resets() {
    let mut app = app();
    let ctx = context();
    click_doc(&mut app, 100.0, 100.0);
    click_doc(&mut app, 150.0, 100.0);
    settle(&mut app, &ctx);
    let beside = |app: &VectorcraftApp, x: f64, y: f64| screen(app, x, y) + vec2(24.0, 24.0);
    near(bar(&ctx).unwrap().min, beside(&app, 150.0, 100.0));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let grip = pos2_of(bar(&ctx).unwrap().left() + 10.0, bar(&ctx).unwrap().center().y);
    let by = vec2(30.0, 40.0);
    frame(&mut app, &ctx, vec![Event::PointerMoved(grip)]);
    frame(&mut app, &ctx, vec![button(grip, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(grip + by)]);
    frame(&mut app, &ctx, vec![button(grip + by, false)]);
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, beside(&app, 150.0, 100.0) + by);
    assert_eq!(paths(&app)[0].anchors.len(), 2, "dragging the handle draws nothing");

    click_doc(&mut app, 200.0, 100.0);
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, beside(&app, 200.0, 100.0) + by);

    // A drag made beside the drawing does not move the resting toolbar.
    let resting = {
        let canvas = app.canvas_xf().unwrap().rect.shrink(8.0);
        let r = bar(&ctx).unwrap();
        Pos2::new(canvas.center().x - r.width() / 2.0, canvas.bottom() - r.height())
    };
    let v = app.view_info();
    app.session.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, resting);
    // A new path, away from the first one's ends.
    click_doc(&mut app, 100.0, 200.0);
    click_doc(&mut app, 200.0, 200.0);
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, beside(&app, 200.0, 200.0) + by);

    assert_eq!(app.run("window.quickPenToolbar.pin", json!({})).unwrap(), json!(true));
    assert_eq!(crate::menus::checked(&app, "window.quickPenToolbar.pin", &json!({})), Some(true));
    let pinned = bar(&ctx).unwrap().min;
    click_doc(&mut app, 250.0, 200.0);
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, pinned);
    assert!(app.run("window.quickPenToolbar.pin", json!({"pinned": "yes"})).is_err());

    app.run("window.quickPenToolbar.reset", json!({})).unwrap();
    assert!(!app.ui.quick_pen_bar_place.pinned);
    settle(&mut app, &ctx);
    near(bar(&ctx).unwrap().min, beside(&app, 250.0, 200.0));
}
