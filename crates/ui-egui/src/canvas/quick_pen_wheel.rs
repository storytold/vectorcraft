//! Ctrl+wheel over the canvas with the Quick Pen tool: the arc size, in place of the view.
//!
//! Public behavior: Ctrl + scroll on a trackpad makes the arc size larger or smaller, on Windows
//! and on macOS alike
//! ([Quick Pen tool keyboard shortcuts](https://helpx.adobe.com/illustrator/web/add-and-edit-artwork/draw-and-edit-paths/quick-pen-tool-keyboard-shortcuts.html)).
//! VectorCraft's own rules: a mouse wheel turned with Ctrl does the same; each wheel event from a
//! mouse wheel (one per notch) makes one step of 15°, as `[` and `]` do, and a touchpad makes one
//! step for each wheel line of its motion; up makes the arc larger and down smaller, and sideways
//! motion makes no step; momentum after
//! the fingers lift makes no step; and a turn ends as egui's scroll turns do (0.15 s without a wheel
//! event, 1 s while a touchpad gesture is under way).

use super::view::fine_steps;
use super::{Ui, VectorcraftApp, json};

/// A wheel turn the Quick Pen tool may take: the motion, in wheel lines, not yet turned into a
/// step; whether the tool took the turn; whether a touchpad gesture is under way (between its Start
/// and End phases); when the turn's last wheel event came (egui's input time); whether a touchpad
/// sent it; and the modifiers the turn holds.
#[derive(Clone, Copy, Default, PartialEq)]
struct ArcWheel {
    rest: f32,
    taken: bool,
    gesture: bool,
    last: f64,
    trackpad: bool,
    held: egui::Modifiers,
}

/// Seconds without a wheel event after which a wheel event starts a new turn, as egui's own scroll
/// timeouts: 1 s while a touchpad gesture is under way (fingers resting mid-gesture), else 0.15 s.
const TURN_GAP: f64 = 0.150;
const GESTURE_GAP: f64 = 1.0;

/// Most steps one frame makes: from 1° to 359° is fewer than 24 steps of 15°.
const MOST_STEPS: i32 = 24;

/// Ctrl alone (the Control key on macOS too).
fn ctrl_alone(m: egui::Modifiers) -> bool {
    m.ctrl && !(m.alt || m.shift || m.mac_cmd)
}

/// With the Quick Pen tool active and the pointer over the canvas (`over`), each wheel event from a
/// mouse wheel turned with Ctrl held makes the arc size one step larger (up) or smaller (down), and a
/// touchpad's motion makes one step per wheel line of it. Momentum after the fingers lift makes no
/// step. Returns true while such a turn goes on, so the view does not zoom or scroll for the rest of
/// it.
pub(super) fn arc_size(app: &mut VectorcraftApp, ui: &Ui, over: bool) -> bool {
    let id = egui::Id::unique("quick-pen-arc-wheel");
    let before: ArcWheel = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed).max(1.0);
    let takes = over && app.session.tool_id() == vectorcraft_tools::quickpen::ID;
    let mut now = before;
    let (any, scrolling) = ui.input(|i| {
        let mut any = false;
        for e in &i.events {
            let egui::Event::MouseWheel { unit, delta, phase, modifiers, source } = e else { continue };
            any = true;
            // A wheel event after egui's scroll timeout, or a touchpad gesture's start, begins a
            // new turn: what the turn before it took and left over goes.
            let gap = if now.gesture && now.trackpad { GESTURE_GAP } else { TURN_GAP };
            if i.time - now.last > gap || *phase == egui::TouchPhase::Start {
                now = ArcWheel::default();
            }
            now.last = i.time;
            now.trackpad = *source == egui::MouseWheelSource::Trackpad;
            // The modifiers held at any point of a touchpad gesture count until it ends, as egui
            // keeps them (Ctrl let go still counts, and Shift added stays added); otherwise the
            // modifiers of each event count.
            match phase {
                egui::TouchPhase::Start => {
                    now.gesture = true;
                    now.held = *modifiers;
                }
                egui::TouchPhase::Move if now.gesture => now.held |= *modifiers,
                egui::TouchPhase::Move => now.held = *modifiers,
                egui::TouchPhase::End | egui::TouchPhase::Cancel => now.gesture = false,
            }
            if !(takes && ctrl_alone(now.held)) {
                continue;
            }
            now.taken = true;
            // Only Move events move, as egui's scroll; momentum after the fingers lift makes no step.
            if *phase != egui::TouchPhase::Move || *source == egui::MouseWheelSource::Momentum {
                continue;
            }
            // A mouse wheel's event is one step, whatever it reports (a browser reports a notch in
            // points, and a system may scale a fast turn's lines); a touchpad's fine motion steps
            // once per wheel line of it. [`fine_steps`] tells them apart, as for the view. Only up
            // and down motion counts.
            let s = delta.y;
            let d = if !fine_steps(*unit, *delta, *source) {
                if s == 0.0 { 0.0 } else { s.signum() }
            } else if *unit == egui::MouseWheelUnit::Point {
                s / line
            } else {
                s
            };
            if d.is_finite() {
                now.rest += d;
            }
        }
        (any, i.is_scrolling())
    });
    if !any && !scrolling {
        now = ArcWheel::default();
    }
    let steps = now.rest.trunc();
    now.rest -= steps;
    let (cmd, n) = if steps > 0.0 { ("tool.quickPen.arcLarger", steps) } else { ("tool.quickPen.arcSmaller", -steps) };
    for _ in 0..(n as i32).min(MOST_STEPS) {
        match app.run(cmd, json!({})) {
            Ok(serde_json::Value::Bool(true)) => {}
            Ok(_) => break,
            Err(e) => {
                app.ui.status = e;
                break;
            }
        }
    }
    if now != before {
        ui.data_mut(|d| d.insert_temp(id, now));
    }
    now.taken
}
