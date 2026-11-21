//! Ctrl+wheel over the canvas with the Quick Pen tool changes the arc size and leaves the view as it
//! is; the arc size commands step it as `[` and `]` do.

use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::SubPath;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::{VectorcraftApp, canvas};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("quickPen");
    app.run("tool.setOption", json!({"key": "action", "value": "sharpArc"})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) {
    frame_at(app, ctx, events, None);
}

/// [`frame`] at input `time` (seconds), when given.
fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, time: Option<f64>) {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, time, ..Default::default() }, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
}

/// A wheel turn over the middle of the canvas, then frames enough for egui to let out all of it.
fn turn(app: &mut VectorcraftApp, ctx: &egui::Context, unit: MouseWheelUnit, dy: f32, modifiers: Modifiers, source: MouseWheelSource) {
    let at = app.canvas_rect.unwrap().center();
    let wheel = Event::MouseWheel { unit, delta: vec2(0.0, dy), phase: egui::TouchPhase::Move, modifiers, source };
    frame(app, ctx, vec![Event::PointerMoved(at), wheel]);
    for _ in 0..40 {
        frame(app, ctx, vec![]);
    }
}

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

fn arc_size(app: &VectorcraftApp) -> f64 {
    app.session.tool_options()["turn"].as_f64().unwrap()
}

/// (zoom, center x, center y) of the view.
fn view(app: &VectorcraftApp) -> (f64, f64, f64) {
    let v = app.view().unwrap();
    (v.zoom, v.center.x, v.center.y)
}

/// Ctrl+wheel with the Quick Pen tool: a notch up makes the last sharp-corner arc 15° larger and a
/// notch down 15° smaller, each as one undo step, with Ctrl as Windows and Linux report it (with
/// Command) and as macOS reports the Control key (alone). A touchpad steps once per wheel line of
/// motion. The view neither zooms nor scrolls.
#[test]
fn ctrl_wheel_changes_the_arc_size_and_not_the_view() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    click_doc(&mut app, 100.0, 150.0);
    click_doc(&mut app, 200.0, 150.0);
    let drawn = paths(&app)[0].clone();
    let undo = app.session.active().unwrap().history.undo.len();
    let still = view(&app);
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let line = ctx.options(|o| o.input_options.line_scroll_speed);

    turn(&mut app, &ctx, MouseWheelUnit::Line, 1.0, ctrl, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 195.0);
    assert_ne!(paths(&app)[0], drawn, "the last arc changed");
    assert_eq!(paths(&app).len(), 1);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    assert_eq!(view(&app), still, "Ctrl+wheel did not zoom or scroll");

    let at = app.canvas_rect.unwrap().center();
    let notch = |dy: f32, modifiers| Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: MouseWheelSource::Wheel,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), notch(-1.0, Modifiers::CTRL), notch(-1.0, Modifiers::CTRL)]);
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(arc_size(&app), 165.0, "two notches down with the macOS Control key");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 3);
    assert_eq!(view(&app), still);

    // A mouse wheel's event is one step, whether the system scaled a fast turn's lines or reports a
    // fraction of a line.
    turn(&mut app, &ctx, MouseWheelUnit::Line, 4.5, ctrl, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 180.0, "a fast notch of four and a half lines");
    turn(&mut app, &ctx, MouseWheelUnit::Line, -0.3, ctrl, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 165.0, "a notch of three tenths of a line");
    // With Shift, Alt or Command added (Cmd on macOS), the wheel leaves the arc size.
    for modifiers in [
        Modifiers { shift: true, ..ctrl },
        Modifiers { alt: true, ..ctrl },
        Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE },
        Modifiers { mac_cmd: true, ..Modifiers::CTRL },
    ] {
        turn(&mut app, &ctx, MouseWheelUnit::Line, 1.0, modifiers, MouseWheelSource::Wheel);
        assert_eq!(arc_size(&app), 165.0, "{modifiers:?}");
    }
    turn(&mut app, &ctx, MouseWheelUnit::Line, 1.0, ctrl, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 180.0);
    // Sideways motion leaves the arc size.
    let sideways = Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(1.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source: MouseWheelSource::Wheel,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), sideways]);
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(arc_size(&app), 180.0, "sideways");
    let still = view(&app);

    // Half a wheel line from a touchpad makes no step; the second half of the same turn makes one,
    // and momentum after it makes none.
    let steps = app.session.active().unwrap().history.undo.len();
    let at = app.canvas_rect.unwrap().center();
    let fine = |dy: f32, source| Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), fine(line / 2.0, MouseWheelSource::Trackpad)]);
    assert_eq!(arc_size(&app), 180.0);
    frame(&mut app, &ctx, vec![fine(line / 2.0, MouseWheelSource::Trackpad)]);
    assert_eq!(arc_size(&app), 195.0);
    frame(&mut app, &ctx, vec![fine(3.0 * line, MouseWheelSource::Momentum)]);
    assert_eq!(arc_size(&app), 195.0, "momentum makes no step");
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(view(&app), still);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one step and no piece");

    // Without Ctrl the wheel scrolls the view and leaves the arc size.
    turn(&mut app, &ctx, MouseWheelUnit::Line, 1.0, Modifiers::NONE, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 195.0);
    assert_ne!(view(&app), still, "the wheel scrolled");
}

/// Ctrl+wheel while a press drags a sharp-corner arc: the arc being dragged takes the new size, the
/// drag goes on, and once the press is let go the piece and its size change are one undo step.
#[test]
fn ctrl_wheel_sizes_the_arc_being_dragged() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    click_doc(&mut app, 100.0, 150.0);
    let undo = app.session.active().unwrap().history.undo.len();
    let c = app.canvas_rect.unwrap().center();
    let from = c + vec2(150.0, 80.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, true)]);
    let mut to = from;
    for _ in 0..5 {
        to += vec2(14.0, 0.0);
        frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
    }
    assert!(app.session.tool_busy(), "the press drags");
    let dragged = paths(&app)[0].clone();
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let notch = Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source: MouseWheelSource::Wheel,
    };
    frame(&mut app, &ctx, vec![notch]);
    assert_eq!(arc_size(&app), 195.0);
    assert!(app.session.tool_busy(), "the drag goes on");
    assert_eq!(paths(&app).len(), 1);
    assert_ne!(paths(&app)[0], dragged, "the arc being dragged is drawn again with the new size");
    // The tool's size set ahead of the dragged arc's: a step that brings the arc to it still reports
    // the change.
    app.run("tool.setOption", json!({"key": "turn", "value": 210})).unwrap();
    let sized = paths(&app)[0].clone();
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(arc_size(&app), 210.0);
    assert_ne!(paths(&app)[0], sized);
    assert!(app.session.tool_busy());
    frame(&mut app, &ctx, vec![button(to, false)]);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert!(!app.session.tool_busy());
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "the piece and its size change are one undo step");
}

/// Ctrl+wheel over the Quick Pen toolbar, which floats over the canvas, leaves the arc size; half a
/// wheel line from a touchpad does not add to half a line from a later turn.
#[test]
fn ctrl_wheel_over_the_toolbar_or_in_two_turns_makes_no_step() {
    let mut app = app();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let bar = canvas::quick_pen_bar_rect(&ctx).expect("the toolbar shows");
    let notch = |modifiers| Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: MouseWheelSource::Wheel,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(bar.center()), notch(ctrl)]);
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(arc_size(&app), 180.0, "over the toolbar");

    let line = ctx.options(|o| o.input_options.line_scroll_speed);
    let at = app.canvas_rect.unwrap().center();
    let half = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, line / 2.0),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source: MouseWheelSource::Trackpad,
    };
    // Two halves a second apart, with no frame in between.
    frame_at(&mut app, &ctx, vec![Event::PointerMoved(at), half.clone()], Some(100.0));
    frame_at(&mut app, &ctx, vec![half.clone()], Some(101.0));
    assert_eq!(arc_size(&app), 180.0, "two turns of half a line each");

    // Within one touchpad gesture, a pause of half a second keeps the half line from before it.
    let phased = |phase| match half.clone() {
        Event::MouseWheel { unit, delta, modifiers, source, .. } => Event::MouseWheel { unit, delta, phase, modifiers, source },
        e => e,
    };
    // The Start event's motion does not count, as for egui's scroll.
    frame_at(&mut app, &ctx, vec![phased(egui::TouchPhase::Start), phased(egui::TouchPhase::Move)], Some(200.0));
    assert_eq!(arc_size(&app), 180.0, "a half line and the motion of a Start");
    // Ctrl let go during the gesture still counts until it ends.
    let let_go = match phased(egui::TouchPhase::Move) {
        Event::MouseWheel { unit, delta, phase, source, .. } => Event::MouseWheel { unit, delta, phase, modifiers: Modifiers::NONE, source },
        e => e,
    };
    frame_at(&mut app, &ctx, vec![let_go], Some(200.5));
    assert_eq!(arc_size(&app), 195.0, "one gesture of two half lines");
    // Shift joining the gesture ends what Ctrl held.
    let shifted = match phased(egui::TouchPhase::Move) {
        Event::MouseWheel { unit, delta, phase, source, .. } => Event::MouseWheel { unit, delta, phase, modifiers: Modifiers::SHIFT, source },
        e => e,
    };
    frame_at(&mut app, &ctx, vec![shifted.clone(), shifted], Some(200.6));
    assert_eq!(arc_size(&app), 195.0, "Shift joined the gesture");
    frame_at(&mut app, &ctx, vec![phased(egui::TouchPhase::Move), phased(egui::TouchPhase::Move)], Some(200.7));
    assert_eq!(arc_size(&app), 195.0, "Shift let go stays in the gesture");

    // A plain wheel turn half a second after the Ctrl one, with no frame in between, scrolls.
    let still = view(&app);
    let plain = Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: MouseWheelSource::Wheel,
    };
    frame_at(&mut app, &ctx, vec![phased(egui::TouchPhase::End)], Some(201.0));
    frame_at(&mut app, &ctx, vec![plain], Some(201.5));
    for k in 1..=40 {
        frame_at(&mut app, &ctx, vec![], Some(201.5 + f64::from(k) / 60.0));
    }
    assert_eq!(arc_size(&app), 195.0);
    assert_ne!(view(&app), still, "the plain turn scrolled");

    // From a source that is not named a touchpad, the same pause within a gesture ends the turn,
    // as egui's scroll timeout does.
    let unknown = |phase| match phased(phase) {
        Event::MouseWheel { unit, delta, phase, modifiers, .. } => {
            Event::MouseWheel { unit, delta, phase, modifiers, source: MouseWheelSource::Unknown }
        }
        e => e,
    };
    frame_at(&mut app, &ctx, vec![Event::PointerMoved(at), unknown(egui::TouchPhase::Start), unknown(egui::TouchPhase::Move)], Some(300.0));
    frame_at(&mut app, &ctx, vec![unknown(egui::TouchPhase::Move)], Some(300.5));
    assert_eq!(arc_size(&app), 195.0, "two turns of half a line each from an unnamed source");
}

/// A mouse wheel that reports its notches in points (as a browser does) steps once per notch,
/// however many points a notch is. Under a modal dialog, and for the rest of a turn begun under it,
/// Ctrl+wheel leaves the arc size.
#[test]
fn ctrl_wheel_steps_per_notch_and_not_under_a_dialog() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    turn(&mut app, &ctx, MouseWheelUnit::Point, 100.0, ctrl, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 195.0, "one notch of 100 points");
    turn(&mut app, &ctx, MouseWheelUnit::Point, -3.0, ctrl, MouseWheelSource::Wheel);
    assert_eq!(arc_size(&app), 180.0, "one notch of 3 points");
    // On the desktop, point motion from an unknown source is a touchpad's: it steps once per wheel
    // line, and smaller motion adds up within the turn.
    let line = ctx.options(|o| o.input_options.line_scroll_speed);
    turn(&mut app, &ctx, MouseWheelUnit::Point, 2.5 * line, ctrl, MouseWheelSource::Unknown);
    assert_eq!(arc_size(&app), 210.0, "two and a half lines");
    turn(&mut app, &ctx, MouseWheelUnit::Point, -line, ctrl, MouseWheelSource::Unknown);
    assert_eq!(arc_size(&app), 195.0, "the half line left over did not count in a later turn");
    let small = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -line / 4.0),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source: MouseWheelSource::Unknown,
    };
    let at = app.canvas_rect.unwrap().center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), small.clone(), small.clone(), small.clone()]);
    assert_eq!(arc_size(&app), 195.0, "three quarters of a line");
    frame(&mut app, &ctx, vec![small]);
    assert_eq!(arc_size(&app), 180.0, "a whole line");
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }

    let notch = Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source: MouseWheelSource::Wheel,
    };
    app.ui.dialog = Some(crate::state::Dialog::new("preferences", json!({})));
    frame_at(&mut app, &ctx, vec![Event::PointerMoved(at), notch.clone()], Some(300.0));
    assert_eq!(arc_size(&app), 180.0, "under the dialog");
    app.ui.dialog = None;
    frame_at(&mut app, &ctx, vec![notch], Some(300.05));
    assert_eq!(arc_size(&app), 180.0, "the rest of a turn begun under the dialog");
}

/// Wheel events in one frame step in the order they came: a notch up refused at the top of the
/// range does not cancel the notch down after it, and a touchpad gesture that starts later in the
/// frame keeps the steps of the motion before it and does not let that motion move the view.
#[test]
fn ctrl_wheel_steps_in_event_order() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    app.run("tool.setOption", json!({"key": "turn", "value": 345})).unwrap();
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let at = app.canvas_rect.unwrap().center();
    let notch = |dy: f32| Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: ctrl,
        source: MouseWheelSource::Wheel,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), notch(1.0), notch(-1.0)]);
    assert_eq!(arc_size(&app), 330.0, "up refused at 345°, then down");
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }

    let line = ctx.options(|o| o.input_options.line_scroll_speed);
    let pad = |phase, dy: f32| Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        phase,
        modifiers: ctrl,
        source: MouseWheelSource::Trackpad,
    };
    frame_at(
        &mut app,
        &ctx,
        vec![pad(egui::TouchPhase::Start, 0.0), pad(egui::TouchPhase::Move, -line), pad(egui::TouchPhase::Start, 0.0)],
        Some(500.0),
    );
    assert_eq!(arc_size(&app), 315.0, "a line down before a later Start");
    for k in 1..=40 {
        frame_at(&mut app, &ctx, vec![], Some(500.0 + f64::from(k) / 60.0));
    }

    // A Ctrl notch followed in the same frame by the Start of a touchpad gesture without Ctrl: the
    // notch steps the arc size and does not move the view.
    let still = view(&app);
    let plain_start = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, 0.0),
        phase: egui::TouchPhase::Start,
        modifiers: Modifiers::NONE,
        source: MouseWheelSource::Trackpad,
    };
    frame_at(&mut app, &ctx, vec![Event::PointerMoved(at), notch(1.0), plain_start], Some(600.0));
    assert_eq!(arc_size(&app), 330.0);
    assert_eq!(view(&app), still, "the notch did not move the view");
}

/// With another tool, Ctrl+wheel leaves the Quick Pen's arc size and moves the view as before.
#[test]
fn ctrl_wheel_with_another_tool_moves_the_view() {
    let mut app = app();
    app.select_tool("pen");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let still = view(&app);
    turn(&mut app, &ctx, MouseWheelUnit::Line, 1.0, Modifiers { ctrl: true, command: true, ..Modifiers::NONE }, MouseWheelSource::Wheel);
    assert_eq!(app.session.tool_options_of("quickPen")["turn"], json!(180.0));
    assert_ne!(view(&app), still);
}

/// The arc size commands step the size by 15° within 1° to 359°, return false at the ends and with
/// another tool, and change the last sharp-corner arc of the path being drawn.
#[test]
fn the_arc_size_commands_step_within_the_range() {
    let mut app = app();
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(arc_size(&app), 195.0);
    assert_eq!(app.run("tool.quickPen.arcSmaller", json!({})).unwrap(), json!(true));
    for _ in 0..10 {
        assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    }
    assert_eq!(arc_size(&app), 330.0);
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(true));
    assert_eq!(app.run("tool.quickPen.arcLarger", json!({})).unwrap(), json!(false), "360° is past 359°");
    assert_eq!(arc_size(&app), 345.0);

    click_doc(&mut app, 100.0, 150.0);
    click_doc(&mut app, 200.0, 150.0);
    let drawn = paths(&app)[0].clone();
    let undo = app.session.active().unwrap().history.undo.len();
    assert_eq!(app.run("tool.quickPen.arcSmaller", json!({})).unwrap(), json!(true));
    assert_ne!(paths(&app)[0], drawn);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);

    app.select_tool("pen");
    assert_eq!(app.run("tool.quickPen.arcSmaller", json!({})).unwrap(), json!(false));
    assert_eq!(app.session.tool_options_of("quickPen")["turn"], json!(330.0));
}
