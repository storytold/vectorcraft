//! The Quick Pen toolbar: it shows while the Quick Pen tool is active, its buttons set the tool's
//! action and flip the last sharp-corner arc, it follows the drawing beside the path's last point,
//! and its More options menu's commands hide it, pin it and reset its place.

use egui::{Event, PointerButton, Pos2, Rect, Vec2, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{Point, SubPath};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::taskbar::{Rig, on_canvas};
use crate::VectorcraftApp;

/// The toolbar's access name.
const BAR: &str = "Quick Pen";

/// The names of its members.
const MEMBERS: [&str; 5] = ["Straight Line", "Continuous Arc", "Sharp Corner Arc", "Flip Arc", "More options"];

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("quickPen");
    app
}

/// The toolbar's rect, once the frames have settled; `None` while it is not shown.
fn bar(rig: &mut Rig) -> Option<Rect> {
    rig.target(BAR).map(|t| t.rect)
}

/// A click of the tool at `x`, `y` in the document.
fn click_doc(rig: &Rig, x: f64, y: f64) {
    let mut app = rig.app();
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

/// The point `x`, `y` of the document on screen.
fn screen(rig: &Rig, x: f64, y: f64) -> Pos2 {
    rig.app().canvas_view().unwrap().to_screen(Point::new(x, y))
}

fn button(pos: Pos2, which: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button: which, pressed, modifiers: Default::default() }
}

fn canvas_rect(rig: &Rig) -> Rect {
    rig.app().canvas_view().unwrap().rect
}

/// Whether the toolbar rests at the bottom center of the canvas: centered across it and in its lower half.
fn rests(rig: &mut Rig, r: Rect) -> bool {
    let canvas = canvas_rect(rig);
    (r.center().x - canvas.center().x).abs() < 1.5 && r.center().y > canvas.center().y && canvas.contains_rect(r)
}

/// Escape on the tool, which ends the path being drawn.
fn escape(rig: &Rig) {
    let mut app = rig.app();
    let v = app.view_info();
    app.session.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
}

/// The toolbar shows only with the Quick Pen tool, at the bottom center of the canvas while no path
/// is being drawn; its action buttons set the tool's action without drawing on the canvas under
/// them; Window › Quick Pen Toolbar hides and shows it.
#[test]
fn the_toolbar_shows_with_the_tool_and_its_buttons_set_the_action() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        let r = bar(rig).expect("the toolbar shows with the Quick Pen tool");
        for name in MEMBERS {
            assert!(rig.target(name).is_some(), "{name}");
        }
        let flip = rig.target("Flip Arc").unwrap();
        assert!(rig.fx.tree().is_disabled_key(flip.node), "Flip Arc waits for an arc");
        assert!(rests(rig, r), "{r:?} rests at the bottom center of the canvas");

        let action = |rig: &Rig| rig.app().session.tool_options()["action"].clone();
        rig.click("Sharp Corner Arc");
        assert_eq!(action(rig), "sharpArc");
        rig.click("Continuous Arc");
        assert_eq!(action(rig), "continuousArc");
        rig.click("Straight Line");
        assert_eq!(action(rig), "line");
        assert!(paths(&rig.app()).is_empty(), "a click on the toolbar draws nothing");

        assert_eq!(rig.app().run("window.quickPenToolbar", json!({})).unwrap(), json!(false));
        assert_eq!(crate::menus::checked(&rig.app(), "window.quickPenToolbar", &json!({})), Some(false));
        assert!(bar(rig).is_none(), "hidden");
        assert_eq!(rig.app().run("window.quickPenToolbar", json!({})).unwrap(), json!(true));
        assert!(bar(rig).is_some(), "shown again");

        rig.app().select_tool("pen");
        assert!(bar(rig).is_none(), "only with the Quick Pen tool");

        // Hidden, it shows again when the tool is selected again.
        rig.app().select_tool("quickPen");
        assert!(bar(rig).is_some());
        assert_eq!(rig.app().run("window.quickPenToolbar", json!({})).unwrap(), json!(false));
        assert!(bar(rig).is_none(), "hidden");
        rig.app().select_tool("pen");
        rig.app().select_tool("quickPen");
        assert!(bar(rig).is_some(), "selecting the tool shows it");
        assert_eq!(crate::menus::checked(&rig.app(), "window.quickPenToolbar", &json!({})), Some(true));
        // Selecting it again while it is active shows it too.
        rig.app().run("window.quickPenToolbar", json!({})).unwrap();
        assert!(bar(rig).is_none(), "hidden");
        rig.app().run("tool.select", json!({"tool": "quickPen"})).unwrap();
        assert!(bar(rig).is_some(), "selected again");
    });
}

/// The toolbar hides while a press drags on the canvas, and shows again once it is let go.
#[test]
fn the_toolbar_hides_while_a_press_drags_on_the_canvas() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        // The first click places the path's start; the press that follows drags out a piece.
        click_doc(rig, 60.0, 60.0);
        assert!(bar(rig).is_some());
        let (a, b) = (screen(rig, 100.0, 100.0), screen(rig, 200.0, 140.0));
        rig.fx.frame(vec![Event::PointerMoved(a), button(a, PointerButton::Primary, true)]);
        rig.fx.frame(vec![Event::PointerMoved(a + (b - a) / 2.0)]);
        rig.fx.frame(vec![Event::PointerMoved(b)]);
        assert!(rig.app().session.tool_busy(), "the press reached the tool");
        assert!(rig.fx.by_label(BAR).is_none(), "hidden mid-drag");
        rig.fx.frame(vec![button(b, PointerButton::Primary, false)]);
        assert!(bar(rig).is_some(), "shown after the drag");
    });
}

/// A middle-button drag on the canvas (a pan), which the tool takes no press from, hides the toolbar
/// too, and it shows again when the button is let go.
#[test]
fn the_toolbar_hides_while_the_canvas_pans() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        assert!(bar(rig).is_some());
        let (a, b) = (screen(rig, 100.0, 100.0), screen(rig, 200.0, 140.0));
        rig.fx.frame(vec![Event::PointerMoved(a), button(a, PointerButton::Middle, true)]);
        rig.fx.frame(vec![Event::PointerMoved(a + (b - a) / 2.0)]);
        rig.fx.frame(vec![Event::PointerMoved(b)]);
        assert!(!rig.app().session.tool_busy(), "the tool has no press under way");
        assert!(rig.fx.by_label(BAR).is_none(), "hidden mid-pan");
        rig.fx.frame(vec![button(b, PointerButton::Middle, false)]);
        assert!(bar(rig).is_some(), "shown after the pan");
    });
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

/// While a path is drawn, the toolbar sits below its last point, on the side the path heads toward;
/// Flip Arc flips the last sharp-corner arc as one undo step and is off after a line; Escape sends the
/// toolbar back to the bottom of the canvas.
#[test]
fn the_toolbar_follows_the_drawing_and_flips_the_last_arc() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        let resting = bar(rig).unwrap();
        assert!(rests(rig, resting));
        rig.app().run("tool.setOption", json!({"key": "action", "value": "sharpArc"})).unwrap();
        assert_eq!(rig.app().run("tool.quickPen.flipArc", json!({})).unwrap(), json!(false), "no path being drawn");
        click_doc(rig, 100.0, 150.0);
        click_doc(rig, 200.0, 150.0);
        // A half circle above the chord, which arrives at (200, 150) heading down: right of and below the end.
        let mid = |rig: &Rig| paths(&rig.app())[0].anchors[1].p;
        assert!(mid(rig).y < 150.0);
        let r = bar(rig).unwrap();
        let end = screen(rig, 200.0, 150.0);
        assert!(r.left() > end.x && r.top() > end.y, "{r:?} right of and below {end:?}");

        let undo = rig.app().session.active().unwrap().history.undo.len();
        rig.click("Flip Arc");
        assert!(mid(rig).y > 150.0, "flipped below the chord");
        assert_eq!(rig.app().session.active().unwrap().history.undo.len(), undo + 1);

        // A line drawn to the left: the toolbar goes to the left of its end, still below it.
        rig.app().run("tool.setOption", json!({"key": "action", "value": "line"})).unwrap();
        click_doc(rig, 120.0, 150.0);
        let r = bar(rig).unwrap();
        let end = screen(rig, 120.0, 150.0);
        assert!(r.right() < end.x && r.top() > end.y, "{r:?} left of and below {end:?}");
        let flip = rig.target("Flip Arc").unwrap();
        assert!(rig.fx.tree().is_disabled_key(flip.node), "Flip Arc is off after a line");
        assert_eq!(rig.app().run("tool.quickPen.flipArc", json!({})).unwrap(), json!(false));
        assert_eq!(rig.app().session.active().unwrap().history.undo.len(), undo + 2, "the line, and no flip");

        escape(rig);
        let r = bar(rig).unwrap();
        assert!(rests(rig, r), "{r:?} is back at the bottom of the canvas");
    });
}

/// Near the bottom of the canvas, a path heading down gets its toolbar above its end, not over it; near
/// the top, one heading up gets it below.
#[test]
fn the_toolbar_goes_to_the_other_side_where_it_runs_off_the_canvas() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        assert!(bar(rig).is_some());
        let canvas = canvas_rect(rig);
        let doc = |rig: &Rig, y: f32| rig.app().canvas_view().unwrap().to_doc(egui::pos2(canvas.center().x, y));
        // A line to the right along the canvas's bottom edge, then one along its top edge drawn upward.
        for (from, to, heads_down) in [(canvas.bottom() - 8.0, canvas.bottom() - 8.0, true), (canvas.top() + 40.0, canvas.top() + 8.0, false)] {
            escape(rig);
            let (a, b) = (doc(rig, from), doc(rig, to));
            click_doc(rig, a.x - 40.0, a.y);
            click_doc(rig, b.x, b.y);
            let r = bar(rig).unwrap();
            let end = screen(rig, b.x, b.y);
            assert!(canvas.contains_rect(r), "{r:?} on the canvas {canvas:?}");
            if heads_down {
                assert!(r.bottom() < end.y && r.left() > end.x, "{r:?} above and right of {end:?}");
            } else {
                assert!(r.top() > end.y, "{r:?} below {end:?}");
            }
        }
    });
}

/// The toolbar stays shown while its handle is dragged farther than the toolbar's height, when the
/// press is no longer on it.
#[test]
fn the_toolbar_stays_while_its_handle_drags_far() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        assert!(bar(rig).is_some());
        let grip = rig.target("Move bar").expect("the toolbar's handle");
        rig.fx.press(&grip);
        let from = grip.center();
        for k in 1..=6 {
            rig.fx.frame(vec![Event::PointerMoved(from + vec2(-20.0, -30.0) * k as f32)]);
            assert!(rig.fx.by_label(BAR).is_some(), "shown at step {k}");
        }
        rig.fx.release_at(from + vec2(-120.0, -180.0));
        assert!(bar(rig).is_some());
    });
}

/// The handle drags the toolbar, which keeps that offset as it follows the drawing, but not in its
/// resting place with no path being drawn; pinned, it stays put; Reset bar position puts it back beside
/// the drawing, unpinned.
#[test]
fn the_toolbar_drags_pins_and_resets() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        // Where the toolbar's corner lies from the end of the drawing.
        let from_end = |rig: &mut Rig, x: f64, y: f64| -> Vec2 { bar(rig).unwrap().min - screen(rig, x, y) };
        click_doc(rig, 100.0, 100.0);
        click_doc(rig, 150.0, 100.0);
        let beside = from_end(rig, 150.0, 100.0);
        assert!(beside.x > 0.0 && beside.y > 0.0, "beside the end, below it: {beside:?}");
        let grip = rig.target("Move bar").expect("the toolbar's handle");
        let by = vec2(30.0, 40.0);
        rig.fx.drag(&grip, by);
        rig.fx.settle();
        let moved = from_end(rig, 150.0, 100.0);
        assert!((moved - (beside + by)).length() < 1.5, "the drag moved it by {by:?}: {beside:?} to {moved:?}");
        assert_eq!(paths(&rig.app())[0].anchors.len(), 2, "dragging the handle draws nothing");

        click_doc(rig, 200.0, 100.0);
        let followed = from_end(rig, 200.0, 100.0);
        assert!((followed - moved).length() < 1.5, "it keeps the offset as it follows: {moved:?} then {followed:?}");

        // A drag made beside the drawing does not move the resting toolbar.
        escape(rig);
        let r = bar(rig).unwrap();
        assert!(rests(rig, r), "{r:?} rests");
        // A new path, away from the first one's ends.
        click_doc(rig, 100.0, 200.0);
        click_doc(rig, 200.0, 200.0);
        let again = from_end(rig, 200.0, 200.0);
        assert!((again - moved).length() < 1.5, "the offset follows the next path: {moved:?} then {again:?}");

        assert_eq!(rig.app().run("window.quickPenToolbar.pin", json!({})).unwrap(), json!(true));
        assert_eq!(crate::menus::checked(&rig.app(), "window.quickPenToolbar.pin", &json!({})), Some(true));
        let pinned = bar(rig).unwrap().min;
        click_doc(rig, 250.0, 200.0);
        let after = bar(rig).unwrap().min;
        assert!((after - pinned).length() < 1.5, "pinned, it stays put: {pinned:?} then {after:?}");
        assert!(rig.app().run("window.quickPenToolbar.pin", json!({"pinned": "yes"})).is_err());

        rig.app().run("window.quickPenToolbar.reset", json!({})).unwrap();
        assert!(!rig.app().ui.quick_pen_bar_place.place.pinned);
        let reset = from_end(rig, 250.0, 200.0);
        assert!((reset - beside).length() < 1.5, "back beside the drawing: {beside:?} then {reset:?}");
    });
}
