//! Android drawing tablets & styluses (e.g. Huawei MatePad 11.5 with M-Pencil, S-Pen, USI).
//!
//! Android delivers stylus events through [`android.view.MotionEvent`] (or `AMotionEvent` in the
//! NDK) with distinct tool types (`TOOL_TYPE_STYLUS`, `TOOL_TYPE_ERASER`, `TOOL_TYPE_FINGER`),
//! normalized pressure (`AXIS_PRESSURE` 0.0..1.0), tilt angle (`AXIS_TILT`), orientation/bearing
//! (`AXIS_ORIENTATION`), and high-frequency historical sample batches (120Hz/240Hz).
//!
//! This module translates those native motion events into VectorCraft's [`StylusFeed`] and
//! [`touch`] routing:
//! - Stylus contact and hover write directly to the canvas's [`StylusFeed`].
//! - Hovering (`ACTION_HOVER_MOVE`) updates the pen pose with 0 pressure so cursor overlays appear.
//! - The eraser tool type or the barrel/side button selects the Eraser tool.
//! - When a stylus has been seen, touch events from fingers or resting palms only navigate
//!   (pan, zoom, multi-touch undo/redo) and never trigger drawing strokes on the canvas.
//! - Pure event mapping functions are decoupled from NDK types so the complete test suite runs on
//!   any platform.

use std::cell::RefCell;
use std::f32::consts::PI;
use std::sync::OnceLock;

use egui::{Event, PointerButton};
use vectorcraft_ui_egui::stylus::{PenPose, PenSample, StylusFeed};
use vectorcraft_ui_egui::touch::{self, Source};

/// Android `MotionEvent` action constants.
pub const ACTION_DOWN: u32 = 0;
pub const ACTION_UP: u32 = 1;
pub const ACTION_MOVE: u32 = 2;
pub const ACTION_CANCEL: u32 = 3;
pub const ACTION_POINTER_DOWN: u32 = 5;
pub const ACTION_POINTER_UP: u32 = 6;
pub const ACTION_HOVER_MOVE: u32 = 7;
pub const ACTION_HOVER_ENTER: u32 = 9;
pub const ACTION_HOVER_EXIT: u32 = 10;

/// Android `MotionEvent` tool types.
pub const TOOL_TYPE_UNKNOWN: u32 = 0;
pub const TOOL_TYPE_FINGER: u32 = 1;
pub const TOOL_TYPE_STYLUS: u32 = 2;
pub const TOOL_TYPE_MOUSE: u32 = 3;
pub const TOOL_TYPE_ERASER: u32 = 4;

/// Android `MotionEvent` button state constants.
pub const BUTTON_PRIMARY: u32 = 1 << 0;
pub const BUTTON_SECONDARY: u32 = 1 << 1;
pub const BUTTON_TERTIARY: u32 = 1 << 2;
pub const BUTTON_BACK: u32 = 1 << 3;
pub const BUTTON_FORWARD: u32 = 1 << 4;
pub const BUTTON_STYLUS_PRIMARY: u32 = 1 << 5;
pub const BUTTON_STYLUS_SECONDARY: u32 = 1 << 6;

/// A decoupled snapshot of an Android pointer event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AndroidPointerSample {
    /// Screen X position in pixels/points.
    pub x: f32,
    /// Screen Y position in pixels/points.
    pub y: f32,
    /// Pressure in 0.0..=1.0 range (from `AXIS_PRESSURE`).
    pub pressure: f32,
    /// Tilt angle in radians from perpendicular (from `AXIS_TILT`, 0.0 to PI/2).
    pub tilt_radians: f32,
    /// Clockwise orientation angle in radians from vertical (from `AXIS_ORIENTATION`, -PI to PI).
    pub orientation_radians: f32,
    /// Tool type (`TOOL_TYPE_STYLUS`, `TOOL_TYPE_ERASER`, `TOOL_TYPE_FINGER`, `TOOL_TYPE_MOUSE`).
    pub tool_type: u32,
    /// Button state bitfield (`BUTTON_PRIMARY`, `BUTTON_SECONDARY`, `BUTTON_STYLUS_PRIMARY`, etc.).
    pub button_state: u32,
    /// Motion action (`ACTION_DOWN`, `ACTION_MOVE`, `ACTION_UP`, `ACTION_HOVER_MOVE`, etc.).
    pub action: u32,
    /// Pointer ID (for multi-touch tracking).
    pub pointer_id: i32,
}

impl Default for AndroidPointerSample {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            tilt_radians: 0.0,
            orientation_radians: 0.0,
            tool_type: TOOL_TYPE_UNKNOWN,
            button_state: 0,
            action: ACTION_MOVE,
            pointer_id: 0,
        }
    }
}

/// The pen feed written by Android motion event handlers.
static FEED: OnceLock<StylusFeed> = OnceLock::new();

/// Returns the singleton [`StylusFeed`] for Android.
pub fn start() -> StylusFeed {
    FEED.get_or_init(StylusFeed::on_ui_thread).clone()
}

/// State tracking for active stylus contacts and barrel buttons.
#[derive(Default)]
struct StylusState {
    /// Pointer ID currently recognized as active stylus.
    active_stylus_id: Option<i32>,
    /// Whether the barrel/secondary button is held during the current stroke.
    barrel_held: bool,
    /// Pending barrel strokes waiting for egui raw input rewrite.
    pending_barrel: Vec<i32>,
}

thread_local! {
    static STYLUS_STATE: RefCell<StylusState> = RefCell::new(StylusState::default());
}

/// Computes the [`PenPose`] from Android tilt and orientation radians.
///
/// In Android:
/// - `tilt_radians`: angle between stylus and screen normal (0 rad = 90° upright, PI/2 = flat).
/// - `orientation_radians`: clockwise projection angle (-PI to PI, where 0 points up).
pub fn compute_pen_pose(tilt_radians: f32, orientation_radians: f32) -> PenPose {
    if !tilt_radians.is_finite() || !orientation_radians.is_finite() {
        return PenPose::default();
    }

    // A tiny tilt (< 1 degree) is treated as upright (no tilt recorded).
    let tilt_deg = (tilt_radians * 180.0 / PI).clamp(0.0, 90.0);
    let tilt = if tilt_deg > 1.0 {
        // Project onto X and Y screen axes:
        let sin_tilt = tilt_radians.sin();
        let tilt_x = (sin_tilt * orientation_radians.sin() * 180.0 / PI).clamp(-90.0, 90.0);
        let tilt_y = (-sin_tilt * orientation_radians.cos() * 180.0 / PI).clamp(-90.0, 90.0);
        Some([tilt_x, tilt_y])
    } else {
        None
    };

    // Rotation clockwise: convert -PI..PI to 0..360°.
    let rotation_deg = ((-orientation_radians * 180.0 / PI).rem_euclid(360.0) + 360.0).rem_euclid(360.0);
    let rotation = (tilt.is_some() || rotation_deg != 0.0).then_some(rotation_deg);

    PenPose { tilt, rotation }.sanitized()
}

/// Process a single Android pointer sample and return the corresponding PenSample and input source.
///
/// Returns `(Option<PenSample>, Option<Source>, bool /* is_barrel_pressed */)`.
pub fn process_pointer_sample(sample: &AndroidPointerSample) -> (Option<PenSample>, Option<Source>, bool) {
    let is_stylus = matches!(sample.tool_type, TOOL_TYPE_STYLUS | TOOL_TYPE_ERASER);
    let is_finger = sample.tool_type == TOOL_TYPE_FINGER;

    if is_finger {
        // Finger touch event: if a pen was previously seen, palm rejection keeps it as Finger
        // for navigation only; otherwise standard touch.
        return (None, Some(Source::Finger), false);
    }

    if !is_stylus {
        // Unknown or plain mouse
        return (None, Some(Source::Mouse), false);
    }

    // It is a stylus (Huawei M-Pencil, S-Pen, etc.)
    let is_hover = matches!(sample.action, ACTION_HOVER_MOVE | ACTION_HOVER_ENTER | ACTION_HOVER_EXIT);
    let is_up = matches!(sample.action, ACTION_UP | ACTION_CANCEL);

    let eraser_by_tool = sample.tool_type == TOOL_TYPE_ERASER;
    let eraser_by_button = (sample.button_state & (BUTTON_STYLUS_PRIMARY | BUTTON_SECONDARY)) != 0;
    let eraser = eraser_by_tool || eraser_by_button;

    let raw_pressure = if is_hover || is_up {
        0.0
    } else if sample.pressure.is_finite() && sample.pressure > 0.0 {
        sample.pressure.clamp(0.0, 1.0)
    } else if matches!(sample.action, ACTION_DOWN | ACTION_MOVE) {
        // A down/move event from a pen that reports 0 pressure is pressed with full force
        1.0
    } else {
        0.0
    };

    let pose = compute_pen_pose(sample.tilt_radians, sample.orientation_radians);
    let pen_sample = PenSample { pressure: raw_pressure, eraser, pose };
    let barrel_pressed = (sample.button_state & (BUTTON_STYLUS_PRIMARY | BUTTON_SECONDARY)) != 0;

    (Some(pen_sample), Some(Source::Pen), barrel_pressed)
}

/// Handle an incoming batch of historical and current pointer samples from an Android MotionEvent.
pub fn handle_motion_sample(sample: &AndroidPointerSample) {
    let (pen_opt, source_opt, barrel) = process_pointer_sample(sample);

    if let Some(source) = source_opt {
        touch::set_source(source);
    }

    if let Some(pen) = pen_opt {
        let feed = start();
        if sample.action == ACTION_MOVE && pen.pressure > 0.0 {
            feed.set_move(pen);
        } else {
            feed.set_sample(Some(pen));
        }

        STYLUS_STATE.with(|state| {
            let mut s = state.borrow_mut();
            if sample.action == ACTION_DOWN {
                s.active_stylus_id = Some(sample.pointer_id);
                s.barrel_held = barrel;
                if barrel {
                    s.pending_barrel.push(sample.pointer_id);
                }
            } else if matches!(sample.action, ACTION_UP | ACTION_CANCEL) {
                if s.active_stylus_id == Some(sample.pointer_id) {
                    s.active_stylus_id = None;
                    s.barrel_held = false;
                }
                s.pending_barrel.retain(|id| *id != sample.pointer_id);
            } else if sample.action == ACTION_MOVE && barrel {
                s.barrel_held = true;
            }
        });
    } else if sample.tool_type == TOOL_TYPE_MOUSE {
        // Plain mouse moved: clear pen feed
        start().set(None);
    }
}

/// Rewrites pointer contact buttons when the stylus barrel button is pressed.
///
/// Analogous to `windows_stylus::raw_input_hook`, if the user pressed down with the stylus side
/// button held (e.g. Huawei M-Pencil button), this turns the primary click into a secondary
/// (right-click / context menu / alternate) button.
pub fn raw_input_hook(raw: &mut egui::RawInput) {
    STYLUS_STATE.with(|state| {
        let s = state.borrow_mut();
        if s.pending_barrel.is_empty() {
            return;
        }

        for ev in &mut raw.events {
            if let Event::PointerButton { button, pressed, .. } = ev {
                if *button == PointerButton::Primary && *pressed && s.barrel_held {
                    *button = PointerButton::Secondary;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stylus_pressure_clamping_and_normalization() {
        let mut sample = AndroidPointerSample {
            tool_type: TOOL_TYPE_STYLUS,
            action: ACTION_MOVE,
            pressure: 0.5,
            ..Default::default()
        };
        let (pen, source, barrel) = process_pointer_sample(&sample);
        assert_eq!(source, Some(Source::Pen));
        assert!(!barrel);
        assert_eq!(pen.map(|p| p.pressure), Some(0.5));

        // Clamping upper bound
        sample.pressure = 2.5;
        let (pen, _, _) = process_pointer_sample(&sample);
        assert_eq!(pen.map(|p| p.pressure), Some(1.0));

        // Hover has 0 pressure
        sample.action = ACTION_HOVER_MOVE;
        sample.pressure = 0.8;
        let (pen, _, _) = process_pointer_sample(&sample);
        assert_eq!(pen.map(|p| p.pressure), Some(0.0));

        // Lift has 0 pressure
        sample.action = ACTION_UP;
        let (pen, _, _) = process_pointer_sample(&sample);
        assert_eq!(pen.map(|p| p.pressure), Some(0.0));
    }

    #[test]
    fn test_eraser_tool_type_and_button() {
        // Eraser by tool type
        let eraser_sample = AndroidPointerSample {
            tool_type: TOOL_TYPE_ERASER,
            action: ACTION_MOVE,
            pressure: 0.6,
            ..Default::default()
        };
        let (pen, _, _) = process_pointer_sample(&eraser_sample);
        assert!(pen.is_some_and(|p| p.eraser), "TOOL_TYPE_ERASER must set eraser flag");

        // Tip with barrel / stylus primary button
        let button_sample = AndroidPointerSample {
            tool_type: TOOL_TYPE_STYLUS,
            action: ACTION_MOVE,
            pressure: 0.6,
            button_state: BUTTON_STYLUS_PRIMARY,
            ..Default::default()
        };
        let (pen, _, barrel) = process_pointer_sample(&button_sample);
        assert!(barrel, "Stylus primary button must report barrel pressed");
        assert!(pen.is_some_and(|p| p.eraser), "Barrel button acts as eraser tool toggle");
    }

    #[test]
    fn test_finger_and_mouse_differentiation() {
        let finger_sample = AndroidPointerSample {
            tool_type: TOOL_TYPE_FINGER,
            action: ACTION_DOWN,
            pressure: 1.0,
            ..Default::default()
        };
        let (pen, source, _) = process_pointer_sample(&finger_sample);
        assert_eq!(pen, None);
        assert_eq!(source, Some(Source::Finger));

        let mouse_sample = AndroidPointerSample {
            tool_type: TOOL_TYPE_MOUSE,
            action: ACTION_MOVE,
            ..Default::default()
        };
        let (pen, source, _) = process_pointer_sample(&mouse_sample);
        assert_eq!(pen, None);
        assert_eq!(source, Some(Source::Mouse));
    }

    #[test]
    fn test_pen_pose_conversion_from_radians() {
        // Upright pen (tilt = 0)
        let pose = compute_pen_pose(0.0, 0.0);
        assert_eq!(pose.tilt, None, "Upright pen has no tilt degrees");

        // Tilted 45 degrees along X axis (orientation = PI/2)
        let pose = compute_pen_pose(PI / 4.0, PI / 2.0);
        assert!(pose.tilt.is_some());
        if let Some([tx, ty]) = pose.tilt {
            assert!((tx - 45.0).abs() < 1.0, "Expected ~45 deg tilt on X, got {tx}");
            assert!(ty.abs() < 1.0, "Expected ~0 deg tilt on Y, got {ty}");
        }

        // Non-finite values sanitization
        let nan_pose = compute_pen_pose(f32::NAN, f32::INFINITY);
        assert_eq!(nan_pose, PenPose::default());
    }

    #[test]
    fn test_raw_input_hook_rewrites_barrel_click() {
        STYLUS_STATE.with(|state| {
            let s = state.borrow_mut();
            s.pending_barrel.push(1);
            s.barrel_held = true;
        });

        let mut raw = egui::RawInput {
            events: vec![Event::PointerButton {
                pos: Pos2::new(100.0, 100.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
            ..Default::default()
        };

        raw_input_hook(&mut raw);

        if let Some(Event::PointerButton { button, .. }) = raw.events.first() {
            assert_eq!(*button, PointerButton::Secondary, "Barrel button rewrote Primary to Secondary");
        } else {
            panic!("Expected event");
        }

        // Clean up
        STYLUS_STATE.with(|state| {
            let s = state.borrow_mut();
            s.pending_barrel.clear();
            s.barrel_held = false;
        });
    }
}
