//! Ctrl+wheel over the canvas with the Quick Pen tool: the arc size, in place of the view.
//!
//! Public behavior: Ctrl + scroll on a trackpad makes the arc size larger or smaller, on Windows
//! and on macOS alike
//! ([Quick Pen tool keyboard shortcuts](https://helpx.adobe.com/illustrator/web/add-and-edit-artwork/draw-and-edit-paths/quick-pen-tool-keyboard-shortcuts.html)).
//! VectorCraft's own rules: a mouse wheel turned with Ctrl does the same; a mouse wheel makes one
//! step of 15°, as `[` and `]` do, for each notch (each whole wheel line, at least one for each pass
//! of wheel motion the canvas reads), and a touchpad makes one step for each wheel line of its
//! motion; up makes the arc larger and down smaller, and sideways motion makes no step (with Ctrl on
//! Windows and Linux, where the wheel reaches the canvas as a zoom, sideways motion counts as up and
//! down); momentum after the fingers lift makes no step; and a turn ends after [`GESTURE_GAP`]
//! without a wheel event, as the framework's wheel gestures do, or after 1 s without one while a
//! touchpad gesture is under way.

use craft::behavior::{GESTURE_GAP, Wheel, WheelSource, WheelUnit};
use craft::keymap::{Mods, Platform};

use super::{Ui, VectorcraftApp, json};

/// A wheel turn the Quick Pen tool may take: the touchpad's motion, in wheel lines, not yet turned
/// into a step; whether the tool took the turn; whether the turn began under a modal window;
/// whether a touchpad gesture is under way (between its Start and End phases); when the turn's last
/// wheel event came (egui's input time); and the modifiers the gesture holds.
#[derive(Clone, Copy, Default, PartialEq)]
struct ArcWheel {
    rest: f32,
    taken: bool,
    muted: bool,
    gesture: bool,
    last: f64,
    held: Mods,
}

/// What a pass's own wheel events say: whether any came, the modifiers of a touchpad gesture's
/// Start among them, and whether the gesture ended after it.
#[derive(Default)]
struct RawWheel {
    any: bool,
    start: Option<Mods>,
    end: bool,
}

/// Seconds without a wheel event after which a wheel event starts a new turn while a touchpad
/// gesture is under way (fingers resting mid-gesture); outside a gesture it is [`GESTURE_GAP`].
const RESTING_GAP: f64 = 1.0;

/// Most steps one share of the wheel makes: from 1° to 359° is fewer than 24 steps of 15°.
const MOST_STEPS: f32 = 24.0;

/// The modifiers held in `a` or in `b`.
fn either(a: Mods, b: Mods) -> Mods {
    Mods { cmd: a.cmd || b.cmd, ctrl: a.ctrl || b.ctrl, alt: a.alt || b.alt, shift: a.shift || b.shift }
}

/// The Control key alone: Ctrl on Windows and Linux, which the keymap folds into `cmd`, and the
/// Control key on macOS, where `cmd` is ⌘.
fn ctrl_alone(m: Mods, os: Platform) -> bool {
    let control = if os.is_mac() { m.ctrl && !m.cmd } else { m.cmd && !m.ctrl };
    control && !(m.alt || m.shift)
}

/// How far a line's worth of motion may fall short of a whole line and still count as one: a zoom
/// factor turned back into lines carries rounding.
const LINE_SLACK: f32 = 1e-3;

/// The wheel's up and down motion in wheel lines, up positive. A wheel turned with egui's zoom
/// modifier (Ctrl on Windows and Linux) reaches the canvas as a zoom factor and no scroll, and so
/// does a touchpad's Ctrl+scroll there, which the framework reads as a pinch; the motion is read
/// back from the factor in lines of egui's `line_scroll_speed` points, at `scroll_zoom_speed` per
/// point, as egui and the framework made it. Points of a touchpad or a browser are lines of `line`
/// points. A wheel turned with the zoom modifier sideways counts as up and down: the factor does
/// not tell the two apart.
fn lines(wheel: &Wheel, ctx: &egui::Context, line: f32) -> f32 {
    let lines = if wheel.scroll.y != 0.0 {
        if wheel.unit == WheelUnit::Line { wheel.lines.y } else { wheel.scroll.y / line }
    } else if wheel.scroll == egui::Vec2::ZERO && wheel.zoom.is_finite() && wheel.zoom > 0.0 && wheel.zoom != 1.0 {
        let (speed, egui_line) = ctx.options(|o| (o.input_options.scroll_zoom_speed, o.input_options.line_scroll_speed));
        wheel.zoom.ln() / speed / egui_line
    } else {
        0.0
    };
    if lines.is_finite() { lines } else { 0.0 }
}

/// Whether `wheel`, of `lines` wheel lines, comes from a touchpad and not from a mouse wheel's
/// notches. egui names the source on macOS and reports it unknown elsewhere. There, point motion
/// (Wayland, a Windows precision touchpad) and line motion that is not whole lines (Windows and X11)
/// count as a touchpad. A browser reports a mouse wheel in points, so the web build counts only line
/// motion that is not whole lines.
fn fine(wheel: &Wheel, lines: f32) -> bool {
    match wheel.source {
        WheelSource::Trackpad | WheelSource::Momentum => true,
        WheelSource::Unknown if wheel.unit == WheelUnit::Point => !cfg!(target_arch = "wasm32"),
        WheelSource::Unknown => (lines - lines.round()).abs() >= LINE_SLACK,
        _ => false,
    }
}

/// The steps `wheel` makes, up positive: a mouse wheel's whole lines, at least one; one for a share
/// of points or pages from a mouse; and a touchpad's motion in wheel lines of `line` points, with
/// the fraction kept in `rest` for later motion. Momentum after the fingers lift makes no step.
fn steps(wheel: &Wheel, ctx: &egui::Context, line: f32, rest: &mut f32) -> f32 {
    if wheel.source == WheelSource::Momentum {
        return 0.0;
    }
    let d = lines(wheel, ctx, line);
    if d == 0.0 {
        return 0.0;
    }
    if fine(wheel, d) {
        *rest += d;
        let whole = (*rest + LINE_SLACK * rest.signum()).trunc();
        *rest -= whole;
        return whole.clamp(-MOST_STEPS, MOST_STEPS);
    }
    let whole = if wheel.unit == WheelUnit::Line { (d.abs() + LINE_SLACK).trunc() } else { 1.0 };
    d.signum() * whole.clamp(1.0, MOST_STEPS)
}

/// With the Quick Pen tool active, the wheel share the canvas read this pass (`wheel`), turned with
/// the Control key alone and not over the Quick Pen toolbar (`bar`), makes the arc size larger (up)
/// or smaller (down). The modifiers held at any point of a touchpad gesture count until it ends,
/// keys tapped between its wheel events included. No step is made while a modal window is up, nor
/// for the rest of a turn begun under one. Returns true while the tool takes the turn, so the view
/// does not zoom or scroll for the rest of it.
pub(super) fn arc_size(app: &mut VectorcraftApp, ui: &Ui, wheel: Option<Wheel>, bar: Option<egui::Rect>) -> bool {
    let id = egui::Id::unique("quick-pen-arc-wheel");
    let before: ArcWheel = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let os = crate::keymap::platform();
    let mut now = before;
    // The pass's own events: the keys, and the wheel's events with the phases of a touchpad gesture,
    // which the canvas's share does not carry when they move nothing (a gesture's Start).
    let (time, raw, keys) = ui.input(|i| {
        let mut keys = Mods::NONE;
        let mut raw = RawWheel::default();
        for e in &i.events {
            match e {
                egui::Event::ModifiersChanged(m) | egui::Event::Key { modifiers: m, .. } => keys = either(keys, Mods::from_egui(m, os)),
                // The framework's pinch from a Ctrl+wheel on Windows and Linux.
                egui::Event::Zoom(_) => raw.any = true,
                egui::Event::MouseWheel { phase, modifiers, .. } => {
                    raw.any = true;
                    match phase {
                        egui::TouchPhase::Start => {
                            raw.start = Some(either(raw.start.unwrap_or(Mods::NONE), Mods::from_egui(modifiers, os)));
                            raw.end = false;
                        }
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => raw.end = true,
                        egui::TouchPhase::Move => {}
                    }
                }
                _ => {}
            }
        }
        (i.time, raw, keys)
    });
    let gap = if now.gesture { RESTING_GAP } else { GESTURE_GAP };
    if raw.any && (time - now.last > gap || raw.start.is_some()) {
        // A wheel event after the gap, or a touchpad gesture's start, begins a new turn: what the
        // turn before it took and left over goes.
        now = ArcWheel::default();
    }
    if let Some(m) = raw.start {
        now.gesture = true;
        now.held = m;
    }
    if now.gesture {
        now.held = either(now.held, keys);
    }
    if raw.any {
        now.last = time;
    }
    // The canvas reads no wheel under a modal window: a turn that goes on there makes no step after
    // the window closes either.
    let modal = crate::panel_docking::modal_window_up(app);
    if modal && raw.any {
        now.muted = true;
    }
    let mut runs = 0.0;
    let quick_pen = app.session.tool_id() == vectorcraft_tools::quickpen::ID;
    if let Some(w) = wheel.filter(|_| !modal) {
        now.held = if now.gesture { either(now.held, w.mods) } else { w.mods };
        let over_bar = w.at.zip(bar).is_some_and(|(at, bar)| bar.contains(at));
        if quick_pen && !now.muted && !over_bar && ctrl_alone(now.held, os) {
            now.taken = true;
            let line = crate::app::view::navigation(&app.session.prefs).wheel_line.max(1.0);
            runs = steps(&w, ui.ctx(), line, &mut now.rest);
        }
    }
    if raw.end {
        now.gesture = false;
    }
    let cmd = if runs > 0.0 { "tool.quickPen.arcLarger" } else { "tool.quickPen.arcSmaller" };
    for _ in 0..(runs.abs() as u32) {
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
    quick_pen && wheel.is_some() && now.taken
}
