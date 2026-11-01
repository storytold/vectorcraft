//! Touch gestures on the canvas (Preferences › Devices › Enable Touch Gestures, #585): a tap with
//! two fingers undoes, one with three redoes.
//!
//! egui hands every finger's contact over as [`egui::Event::Touch`] (winit reads them from the
//! system: `WM_POINTER` on Windows), and makes the first finger the mouse pointer too. So the first
//! finger has pressed on the canvas by the time a second lands: that press is dropped then
//! ([`Taps::feed`] says [`Touch::Fingers`]), and the tap undoes what came before it.
//!
//! Where the platform tells a pen from a finger (the web shell reads Pointer Events: an iPad with
//! an Apple Pencil), it says which drives the pointer ([`set_source`]). Once a pen has been seen in this session,
//! fingers only navigate: pinch, two-finger pan and the taps; a finger or a resting palm never
//! presses a tool on the canvas.

use std::cell::Cell;

use egui::{Pos2, TouchId, TouchPhase};

/// What drives the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// A mouse or trackpad, or a platform that can't tell (the desktop apps).
    #[default]
    Mouse,
    /// A pen (Apple Pencil, a stylus).
    Pen,
    /// A finger on a touch screen.
    Finger,
}

thread_local! {
    static SOURCE: Cell<Source> = const { Cell::new(Source::Mouse) };
    static PEN_SEEN: Cell<bool> = const { Cell::new(false) };
}

/// The pointer's presses now come from `source`; a pen makes fingers navigate only from then on.
pub fn set_source(source: Source) {
    SOURCE.with(|s| s.set(source));
    if source == Source::Pen {
        PEN_SEEN.with(|p| p.set(true));
    }
}

/// Set the session-only pen policy (reload starts with finger drawing enabled).
pub fn set_pen_seen(seen: bool) {
    PEN_SEEN.with(|p| p.set(seen));
}

/// A pen has been seen.
pub fn pen_seen() -> bool {
    PEN_SEEN.with(Cell::get)
}

/// The press in progress is a finger's while a pen has been seen: it navigates, it doesn't draw.
pub(crate) fn finger_navigates_only() -> bool {
    SOURCE.with(Cell::get) == Source::Finger && pen_seen()
}

/// How long the fingers of a tap may stay down, in seconds.
const TAP_TIME: f64 = 0.5;
/// How far (points) a finger of a tap may move: further, it is a pinch or a pan.
const TAP_SLOP: f32 = 16.0;

/// What a touch event made of the gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Touch {
    /// A finger landed while another is down: the gesture isn't the first finger's press, nor a
    /// lifted finger's press as it lands again.
    Fingers,
    /// All fingers lifted from a tap of this many fingers (two or more).
    Tap(usize),
}

/// The fingers down and how the gesture they make has gone so far.
#[derive(Clone, Debug, Default)]
pub(crate) struct Taps {
    /// Each finger down and where it landed.
    down: Vec<(TouchId, Pos2)>,
    /// The most fingers down at once in this gesture.
    most: usize,
    /// When its first finger landed.
    start: f64,
    /// A finger moved too far or stayed too long: not a tap.
    spoilt: bool,
}

impl Taps {
    /// Follow a finger's `phase` at `pos` (touch `id`) at time `now` (seconds).
    pub(crate) fn feed(&mut self, id: TouchId, phase: TouchPhase, pos: Pos2, now: f64) -> Option<Touch> {
        match phase {
            TouchPhase::Start => {
                if self.down.is_empty() {
                    *self = Self { start: now, ..Self::default() };
                }
                if !self.down.iter().any(|(d, _)| *d == id) {
                    self.down.push((id, pos));
                }
                self.most = self.most.max(self.down.len());
                (self.down.len() >= 2).then_some(Touch::Fingers)
            }
            TouchPhase::Move => {
                if self.down.iter().any(|(d, at)| *d == id && at.distance(pos) > TAP_SLOP) {
                    self.spoilt = true;
                }
                None
            }
            TouchPhase::End | TouchPhase::Cancel => {
                let known = self.down.len();
                self.down.retain(|(d, _)| *d != id);
                if self.down.len() == known {
                    return None;
                }
                self.spoilt |= phase == TouchPhase::Cancel || now - self.start > TAP_TIME;
                (self.down.is_empty() && !self.spoilt && self.most >= 2).then_some(Touch::Tap(self.most))
            }
        }
    }

    /// A finger is down.
    pub(crate) fn is_down(&self) -> bool {
        !self.down.is_empty()
    }
}

/// The command a tap of `fingers` runs: two undo, three redo.
pub(crate) fn tap_command(fingers: usize) -> Option<&'static str> {
    match fingers {
        2 => Some("edit.undo"),
        3 => Some("edit.redo"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> TouchId {
        TouchId(n)
    }

    #[test]
    fn two_and_three_finger_taps() {
        let mut t = Taps::default();
        let p = Pos2::new(100.0, 100.0);
        assert_eq!(t.feed(id(1), TouchPhase::Start, p, 0.0), None);
        assert_eq!(t.feed(id(2), TouchPhase::Start, p + egui::vec2(60.0, 0.0), 0.05), Some(Touch::Fingers));
        assert!(t.is_down());
        assert_eq!(t.feed(id(1), TouchPhase::Move, p + egui::vec2(3.0, 2.0), 0.1), None);
        assert_eq!(t.feed(id(1), TouchPhase::End, p, 0.15), None, "one finger still down");
        assert_eq!(t.feed(id(2), TouchPhase::End, p, 0.2), Some(Touch::Tap(2)));
        assert!(!t.is_down());
        // Three fingers.
        for (n, at) in [(4, 0.0), (5, 0.02), (6, 0.04)] {
            t.feed(id(n), TouchPhase::Start, p, 1.0 + at);
        }
        let ends: Vec<_> = [4, 5, 6].into_iter().map(|n| t.feed(id(n), TouchPhase::End, p, 1.2)).collect();
        assert_eq!(ends, [None, None, Some(Touch::Tap(3))]);
        assert_eq!((tap_command(2), tap_command(3), tap_command(1)), (Some("edit.undo"), Some("edit.redo"), None));
    }

    #[test]
    fn a_pinch_a_long_press_a_cancel_or_one_finger_is_no_tap() {
        let p = Pos2::new(100.0, 100.0);
        let gesture = |moved: f32, held: f64, last: TouchPhase, fingers: u64| {
            let mut t = Taps::default();
            for n in 0..fingers {
                t.feed(id(n), TouchPhase::Start, p, 0.0);
            }
            t.feed(id(0), TouchPhase::Move, p + egui::vec2(moved, 0.0), held / 2.0);
            (0..fingers).filter_map(|n| t.feed(id(n), if n + 1 == fingers { last } else { TouchPhase::End }, p, held)).last()
        };
        assert_eq!(gesture(0.0, 0.2, TouchPhase::End, 2), Some(Touch::Tap(2)));
        assert_eq!(gesture(40.0, 0.2, TouchPhase::End, 2), None, "a pinch");
        assert_eq!(gesture(0.0, 0.9, TouchPhase::End, 2), None, "held too long");
        assert_eq!(gesture(0.0, 0.2, TouchPhase::Cancel, 2), None, "cancelled");
        assert_eq!(gesture(0.0, 0.2, TouchPhase::End, 1), None, "a pen or one finger");
    }

    #[test]
    fn fingers_navigate_only_once_a_pen_was_seen() {
        set_pen_seen(false);
        set_source(Source::Finger);
        assert!(!finger_navigates_only(), "no pen yet: a finger draws");
        set_source(Source::Pen);
        assert!(pen_seen() && !finger_navigates_only());
        set_source(Source::Finger);
        assert!(finger_navigates_only());
        set_source(Source::Mouse);
        assert!(!finger_navigates_only(), "a mouse still draws");
        set_pen_seen(false);
    }
}
