//! Ctrl+wheel over the canvas with the Quick Pen tool changes the arc size and leaves the view as it
//! is; the arc size commands step it as `[` and `]` do.

use craft::behavior::CanvasView;
use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, TouchPhase, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::SubPath;
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::taskbar::{Rig, on_canvas};
use crate::VectorcraftApp;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("quickPen");
    app.run("tool.setOption", json!({"key": "action", "value": "sharpArc"})).unwrap();
    app
}

/// The Control key as egui reports it: with Command on Windows and Linux, alone on macOS.
fn ctrl() -> Modifiers {
    if crate::keymap::platform().is_mac() { Modifiers::CTRL } else { Modifiers { ctrl: true, command: true, ..Modifiers::NONE } }
}

fn wheel(unit: MouseWheelUnit, dy: f32, phase: TouchPhase, modifiers: Modifiers, source: MouseWheelSource) -> Event {
    Event::MouseWheel { unit, delta: vec2(0.0, dy), phase, modifiers, source }
}

/// A mouse wheel's notches of `dy` lines in one event.
fn notch(dy: f32, modifiers: Modifiers) -> Event {
    wheel(MouseWheelUnit::Line, dy, TouchPhase::Move, modifiers, MouseWheelSource::Wheel)
}

/// A touchpad's motion of `dy` points.
fn pad(phase: TouchPhase, dy: f32, modifiers: Modifiers) -> Event {
    wheel(MouseWheelUnit::Point, dy, phase, modifiers, MouseWheelSource::Trackpad)
}

/// The points of a touchpad's motion that make one wheel line: on macOS a line of the canvas's
/// navigation; on Windows and Linux, where the framework reads a touchpad's Ctrl+scroll as a pinch,
/// egui's line scroll speed.
fn line(rig: &Rig) -> f32 {
    if crate::keymap::platform().is_mac() {
        crate::app::view::navigation(&rig.app().session.prefs).wheel_line
    } else {
        rig.fx.ctx().options(|o| o.input_options.line_scroll_speed)
    }
}

fn center(rig: &Rig) -> Pos2 {
    rig.app().canvas_view().unwrap().rect.center()
}

/// One frame with the pointer over `at` and `events`, then a pause past the end of a wheel turn.
fn turn_at(rig: &mut Rig, at: Pos2, events: Vec<Event>) {
    rig.fx.frame(vec![Event::PointerMoved(at)]);
    rig.fx.frame(events);
    rig.fx.advance(1.5);
    rig.fx.frame(vec![]);
}

fn turn(rig: &mut Rig, events: Vec<Event>) {
    let at = center(rig);
    turn_at(rig, at, events);
}

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

fn arc_size(rig: &Rig) -> f64 {
    rig.app().session.tool_options()["turn"].as_f64().unwrap()
}

fn undo_len(rig: &Rig) -> usize {
    rig.app().session.active().unwrap().history.undo.len()
}

fn view(rig: &Rig) -> CanvasView {
    rig.app().canvas_view().unwrap()
}

/// Ctrl+wheel with the Quick Pen tool: a notch up makes the last sharp-corner arc 15° larger and
/// notches down make it 15° smaller per notch, each as one undo step; the view neither zooms nor
/// scrolls. With Shift or Alt added the wheel leaves the arc size; without Ctrl it scrolls the view.
#[test]
fn ctrl_wheel_changes_the_arc_size_and_not_the_view() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        click_doc(rig, 100.0, 150.0);
        click_doc(rig, 200.0, 150.0);
        let drawn = paths(&rig.app())[0].clone();
        let undo = undo_len(rig);
        let still = view(rig);

        turn(rig, vec![notch(1.0, ctrl())]);
        assert_eq!(arc_size(rig), 195.0);
        assert_ne!(paths(&rig.app())[0], drawn, "the last arc changed");
        assert_eq!(paths(&rig.app()).len(), 1);
        assert_eq!(undo_len(rig), undo + 1);
        assert_eq!(view(rig), still, "Ctrl+wheel did not zoom or scroll");

        turn(rig, vec![notch(-1.0, ctrl()), notch(-1.0, ctrl())]);
        assert_eq!(arc_size(rig), 165.0, "two notches down");
        assert_eq!(undo_len(rig), undo + 3);
        assert_eq!(view(rig), still);

        for modifiers in [Modifiers { shift: true, ..ctrl() }, Modifiers { alt: true, ..ctrl() }] {
            turn(rig, vec![notch(1.0, modifiers)]);
            assert_eq!(arc_size(rig), 165.0, "{modifiers:?}");
        }

        let still = view(rig);
        turn(rig, vec![notch(1.0, Modifiers::NONE)]);
        assert_eq!(arc_size(rig), 165.0, "a plain wheel");
        assert_ne!(view(rig), still, "a plain wheel scrolls the view");
    });
}

/// A touchpad steps once per wheel line of motion: half a line makes no step, the second half of the
/// same gesture makes one, and momentum after it makes none (egui reports momentum on macOS). Half a
/// line in a later turn does not add to the half before it.
#[test]
fn a_touchpad_steps_once_per_wheel_line() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        let line = line(rig);
        let still = view(rig);
        rig.fx.hold(ctrl());
        rig.fx.frame(vec![Event::PointerMoved(center(rig))]);
        rig.fx.frame(vec![pad(TouchPhase::Start, 0.0, ctrl())]);
        rig.fx.frame(vec![pad(TouchPhase::Move, line / 2.0, ctrl())]);
        assert_eq!(arc_size(rig), 180.0, "half a line");
        rig.fx.frame(vec![pad(TouchPhase::Move, line / 2.0, ctrl())]);
        assert_eq!(arc_size(rig), 195.0, "a whole line");
        rig.fx.frame(vec![pad(TouchPhase::End, 0.0, ctrl())]);
        if crate::keymap::platform().is_mac() {
            rig.fx.frame(vec![wheel(MouseWheelUnit::Point, 3.0 * line, TouchPhase::Move, ctrl(), MouseWheelSource::Momentum)]);
            assert_eq!(arc_size(rig), 195.0, "momentum makes no step");
        }
        assert_eq!(view(rig), still, "the view stays");

        rig.fx.advance(2.0);
        turn(rig, vec![pad(TouchPhase::Move, line / 2.0, ctrl())]);
        turn(rig, vec![pad(TouchPhase::Move, line / 2.0, ctrl())]);
        assert_eq!(arc_size(rig), 195.0, "two turns of half a line each");
        rig.fx.hold(Modifiers::NONE);
    });
}

/// Shift pressed and let go between two wheel events of a touchpad gesture held with Ctrl counts for
/// the rest of the gesture; Ctrl let go during the gesture still counts until it ends. egui reports a
/// touchpad gesture's phases on macOS; on Windows and Linux the framework reads a touchpad's
/// Ctrl+scroll as a pinch, which carries no phases.
#[cfg(target_os = "macos")]
#[test]
fn keys_held_during_a_touchpad_gesture_count_until_it_ends() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        let line = line(rig);
        let both = Modifiers { shift: true, ..ctrl() };
        rig.fx.frame(vec![Event::PointerMoved(center(rig))]);
        rig.fx.frame(vec![pad(TouchPhase::Start, 0.0, ctrl())]);
        rig.fx.frame(vec![pad(TouchPhase::Move, line, ctrl())]);
        assert_eq!(arc_size(rig), 195.0, "a line up with Ctrl");
        rig.fx.frame(vec![pad(TouchPhase::Move, line, Modifiers::NONE)]);
        assert_eq!(arc_size(rig), 210.0, "Ctrl let go still counts");
        rig.fx.frame(vec![Event::ModifiersChanged(both), Event::ModifiersChanged(ctrl())]);
        rig.fx.frame(vec![pad(TouchPhase::Move, line, ctrl())]);
        assert_eq!(arc_size(rig), 210.0, "Shift tapped mid-gesture");
        rig.fx.frame(vec![pad(TouchPhase::End, 0.0, ctrl())]);
    });
}

/// Ctrl+wheel over the Quick Pen toolbar, under a modal window, and for the rest of a turn begun
/// under one, leaves the arc size.
#[test]
fn ctrl_wheel_over_the_toolbar_or_under_a_dialog_makes_no_step() {
    let mut app = app();
    on_canvas(&mut app, |rig| {
        let bar = rig.target("Quick Pen").expect("the toolbar shows").rect;
        turn_at(rig, bar.center(), vec![notch(1.0, ctrl())]);
        assert_eq!(arc_size(rig), 180.0, "over the toolbar");

        rig.app().run("edit.preferences", json!({})).unwrap();
        rig.fx.settle();
        let at = center(rig);
        rig.fx.frame(vec![Event::PointerMoved(at)]);
        rig.fx.frame(vec![notch(1.0, ctrl())]);
        assert_eq!(arc_size(rig), 180.0, "under the dialog");
        rig.app().prefs = None;
        rig.fx.frame(vec![notch(1.0, ctrl())]);
        assert_eq!(arc_size(rig), 180.0, "the rest of a turn begun under the dialog");
        rig.fx.advance(1.5);
        turn(rig, vec![notch(1.0, ctrl())]);
        assert_eq!(arc_size(rig), 195.0, "a new turn");
    });
}

/// With another tool, Ctrl+wheel leaves the Quick Pen's arc size and moves the view as before.
#[test]
fn ctrl_wheel_with_another_tool_moves_the_view() {
    let mut app = app();
    app.select_tool("pen");
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        let still = view(rig);
        turn(rig, vec![notch(1.0, ctrl())]);
        assert_eq!(rig.app().session.tool_options_of("quickPen")["turn"], json!(180.0));
        assert_ne!(view(rig), still);
    });
}

/// The arc size commands step the size by 15° within 1° to 359°, return false at the ends and with
/// another tool, and change the last sharp-corner arc of the path being drawn, also while a press
/// drags it.
#[test]
fn the_arc_size_commands_step_within_the_range() {
    let mut app = app();
    let arc = |app: &VectorcraftApp| app.session.tool_options()["turn"].as_f64().unwrap();
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(arc(&app), 195.0);
    assert_eq!(app.run("tool.quickPen.arcSmaller", json!({})).unwrap(), json!(true));
    for _ in 0..10 {
        assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    }
    assert_eq!(arc(&app), 330.0);
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(false), "360° is past 359°");
    assert_eq!(arc(&app), 345.0);

    let v = vectorcraft_engine::ViewInfo { smart_guides: false, ..Default::default() };
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 100.0, 150.0), v).unwrap();
    }
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 200.0, 150.0), v).unwrap();
    }
    let drawn = paths(&app)[0].clone();
    let undo = app.session.active().unwrap().history.undo.len();
    assert_eq!(app.run("tool.quickPen.arcSmaller", json!({})).unwrap(), json!(true));
    assert_ne!(paths(&app)[0], drawn);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);

    // During a drag the arc being dragged takes the new size and the drag goes on; the piece and its
    // size change are one undo step.
    app.run("tool.setOption", json!({"key": "turn", "value": 180})).unwrap();
    let undo = app.session.active().unwrap().history.undo.len();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 260.0, 150.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 150.0), v).unwrap();
    assert!(app.session.tool_busy(), "the press drags");
    let dragged = paths(&app)[0].clone();
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(arc(&app), 195.0);
    assert!(app.session.tool_busy(), "the drag goes on");
    assert_ne!(paths(&app)[0], dragged, "the arc being dragged is drawn again with the new size");
    // The tool's size set ahead of the dragged arc's: a step that brings the arc to it still reports
    // the change.
    app.run("tool.setOption", json!({"key": "turn", "value": 210})).unwrap();
    let sized = paths(&app)[0].clone();
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(arc(&app), 210.0);
    assert_ne!(paths(&app)[0], sized);
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 150.0), v).unwrap();
    assert!(!app.session.tool_busy());
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "the piece and its size changes are one undo step");

    app.select_tool("pen");
    assert_eq!(app.run("tool.quickPen.arcSmaller", json!({})).unwrap(), json!(false));
    assert_eq!(app.session.tool_options_of("quickPen")["turn"], json!(210.0));
}
