//! Windows pen tablets: with Windows Ink, winit reports a pen in contact as a touch and drops its
//! barrel button, and egui-winit presses the primary button for the touch it makes the pointer,
//! so a pen pressed with its barrel button held clicks and drags with the left button. The
//! `WM_POINTERDOWN` message winit is about to handle still has the barrel button's state in its
//! second-button flag ("analogous to a mouse right button down",
//! <https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerdown>): [`msg_hook`] notes
//! it, and [`raw_input_hook`] makes the primary button of that contact the secondary one.
#![cfg_attr(not(all(windows, not(target_vendor = "win7"))), allow(dead_code))]

use std::cell::RefCell;

use egui::{Event, Modifiers, PointerButton, Pos2, TouchId, TouchPhase};

/// `POINTER_MESSAGE_FLAG_SECONDBUTTON` (`winuser.h`): in the high word of a pointer message's
/// `wParam`, a pen in contact with its barrel button pressed.
const POINTER_MESSAGE_FLAG_SECONDBUTTON: usize = 0x0020;
/// At most this many pointer contacts wait for their touch event.
const MAX_PENDING: usize = 16;

thread_local! {
    static STROKES: RefCell<BarrelStrokes> = RefCell::new(BarrelStrokes::default());
}

/// winit's message hook (`EventLoopBuilderExtWindows::with_msg_hook`), which winit calls for each
/// message it takes from the thread's queue, before dispatching it: notes each pointer contact's id
/// and whether the barrel button was held.
/// It never consumes the message.
#[cfg(all(windows, not(target_vendor = "win7")))]
#[allow(unsafe_code)]
pub fn msg_hook(msg: *const std::ffi::c_void) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{MSG, WM_POINTERDOWN};
    // SAFETY: winit passes the `MSG` that `PeekMessageW` has just filled, which lives for this
    // call; `as_ref` gives `None` for a null pointer.
    let Some(msg) = (unsafe { msg.cast::<MSG>().as_ref() }) else {
        return false;
    };
    if msg.message == WM_POINTERDOWN {
        STROKES.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.pointer_down(msg.wParam.0);
            }
        });
    }
    false
}

/// Before egui reads a frame's input: the primary button of a contact that started with the
/// barrel button held becomes the secondary button.
pub fn raw_input_hook(raw: &mut egui::RawInput) {
    STROKES.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.rewrite(&mut raw.events);
        }
    });
}

/// The pen contacts that press the secondary button.
#[derive(Default)]
struct BarrelStrokes {
    /// Each `WM_POINTERDOWN`'s pointer id and whether the barrel button was held, oldest first,
    /// until the touch of that contact starts.
    pending: Vec<(u64, bool)>,
    /// The contact whose touch started with the barrel button held, until it ends, the same pointer
    /// starts another contact, or another contact starts with the barrel button held.
    right: Option<Right>,
}

/// A contact that started with the barrel button held.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Right {
    id: TouchId,
    /// Its secondary button is down: the press went to egui and the release has not.
    down: bool,
}

impl BarrelStrokes {
    /// A `WM_POINTERDOWN` with this `wParam`, whose low word is the pointer id and whose high word
    /// holds the pointer flags.
    fn pointer_down(&mut self, wparam: usize) {
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        self.pending.push(((wparam & 0xffff) as u64, (wparam >> 16) & POINTER_MESSAGE_FLAG_SECONDBUTTON != 0));
    }

    /// Makes the primary button of the right contact the secondary one. egui-winit sends a touch's
    /// pointer button with the touch event it comes from: the press after the touch's start and
    /// the pointer's move to it, the release right after its end. A canceled contact, or one
    /// whose end never came (Windows does not promise one), gets a secondary release of its own.
    fn rewrite(&mut self, events: &mut Vec<Event>) {
        let release = |pos: Pos2| Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE };
        let mut out = Vec::with_capacity(events.len() + 1);
        // The button event that comes next for the right contact: `Some(true)` for its press,
        // `Some(false)` for its release.
        let mut next = None;
        for mut event in events.drain(..) {
            let mut after = None;
            match &mut event {
                Event::Touch { id, phase, pos, .. } => {
                    let (id, phase, pos) = (*id, *phase, *pos);
                    if phase == TouchPhase::Start {
                        // Each start decides afresh, from the oldest contact of its pointer still waiting.
                        let barrel = match self.pending.iter().position(|&(p, _)| p == id.0) {
                            Some(i) => self.pending.remove(i).1,
                            None => false,
                        };
                        if let Some(r) = self.right
                            && (barrel || r.id == id)
                        {
                            if r.down {
                                out.push(release(pos));
                            }
                            self.right = None;
                        }
                        if barrel {
                            self.right = Some(Right { id, down: false });
                        }
                    }
                    next = None;
                    if let Some(r) = self.right.filter(|r| r.id == id) {
                        match phase {
                            TouchPhase::Start => next = Some(true),
                            TouchPhase::Move => {}
                            TouchPhase::End => {
                                self.right = None;
                                next = Some(false);
                            }
                            TouchPhase::Cancel => {
                                self.right = None;
                                if r.down {
                                    after = Some(release(pos));
                                }
                            }
                        }
                    }
                }
                Event::PointerMoved(_) if next == Some(true) => {}
                Event::PointerButton { button, pressed, .. } if next == Some(*pressed) && *button == PointerButton::Primary => {
                    *button = PointerButton::Secondary;
                    if let Some(r) = self.right.as_mut() {
                        r.down = *pressed;
                    }
                    next = None;
                }
                _ => next = None,
            }
            out.push(event);
            out.extend(after);
        }
        *events = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{TouchDeviceId, pos2};

    /// `POINTER_MESSAGE_FLAG_INRANGE`, `_INCONTACT` and `_FIRSTBUTTON` (`winuser.h`): a pen tip
    /// touching down.
    const TIP: usize = 0x0002 | 0x0004 | 0x0010;
    /// `POINTER_MESSAGE_FLAG_INRANGE` and `_INCONTACT` with the barrel button.
    const BARREL: usize = 0x0002 | 0x0004 | POINTER_MESSAGE_FLAG_SECONDBUTTON;

    fn down(strokes: &mut BarrelStrokes, id: usize, flags: usize) {
        strokes.pointer_down((flags << 16) | id);
    }

    /// The events egui reads after the hook.
    fn rewrite(strokes: &mut BarrelStrokes, events: &[Event]) -> Vec<Event> {
        let mut events = events.to_vec();
        strokes.rewrite(&mut events);
        events
    }

    fn touch(id: u64, phase: TouchPhase) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos: pos2(10.0, 20.0), force: Some(0.5) }
    }

    fn button(button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: pos2(10.0, 20.0), button, pressed, modifiers: Modifiers::NONE }
    }

    /// What egui-winit sends for the touch that drives the pointer.
    fn start(id: u64) -> Vec<Event> {
        vec![touch(id, TouchPhase::Start), Event::PointerMoved(pos2(10.0, 20.0)), button(PointerButton::Primary, true)]
    }

    fn end(id: u64) -> Vec<Event> {
        vec![touch(id, TouchPhase::End), button(PointerButton::Primary, false), Event::PointerGone]
    }

    fn buttons(events: &[Event]) -> Vec<(PointerButton, bool)> {
        events.iter().filter_map(|e| if let Event::PointerButton { button, pressed, .. } = e { Some((*button, *pressed)) } else { None }).collect()
    }

    const SECONDARY_DOWN: (PointerButton, bool) = (PointerButton::Secondary, true);
    const SECONDARY_UP: (PointerButton, bool) = (PointerButton::Secondary, false);
    const PRIMARY_DOWN: (PointerButton, bool) = (PointerButton::Primary, true);
    const PRIMARY_UP: (PointerButton, bool) = (PointerButton::Primary, false);

    #[test]
    fn a_pen_contact_with_the_barrel_button_presses_the_secondary_button() {
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        assert_eq!(buttons(&rewrite(&mut strokes, &start(7))), [SECONDARY_DOWN]);
        let moved = [touch(7, TouchPhase::Move), Event::PointerMoved(pos2(15.0, 25.0))];
        assert_eq!(rewrite(&mut strokes, &moved), moved);
        assert_eq!(buttons(&rewrite(&mut strokes, &end(7))), [SECONDARY_UP]);
        assert_eq!(strokes.right, None, "the next contact starts afresh");
        assert!(strokes.pending.is_empty());
    }

    #[test]
    fn a_pen_contact_with_the_tip_alone_keeps_the_primary_button() {
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, TIP);
        let events = [start(7), end(7)].concat();
        assert_eq!(rewrite(&mut strokes, &events), events);
    }

    #[test]
    fn contacts_of_a_pen_within_one_frame_keep_their_order() {
        // A tap with the barrel button held, then one with the tip alone, before the next frame.
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        down(&mut strokes, 7, TIP);
        let events = rewrite(&mut strokes, &[start(7), end(7), start(7), end(7)].concat());
        assert_eq!(buttons(&events), [SECONDARY_DOWN, SECONDARY_UP, PRIMARY_DOWN, PRIMARY_UP]);
    }

    #[test]
    fn a_mouse_button_right_after_a_barrel_contact_keeps_its_button() {
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        let events = [start(7), vec![button(PointerButton::Primary, true)], end(7), vec![button(PointerButton::Primary, false)]].concat();
        assert_eq!(buttons(&rewrite(&mut strokes, &events)), [SECONDARY_DOWN, PRIMARY_DOWN, SECONDARY_UP, PRIMARY_UP]);
    }

    #[test]
    fn other_touches_and_the_mouse_keep_the_primary_button_during_a_barrel_contact() {
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        down(&mut strokes, 9, TIP);
        let events = [start(7), start(9), vec![button(PointerButton::Primary, true)], end(9)].concat();
        assert_eq!(buttons(&rewrite(&mut strokes, &events)), [SECONDARY_DOWN, PRIMARY_DOWN, PRIMARY_DOWN, PRIMARY_UP]);
        assert_eq!(strokes.right, Some(Right { id: TouchId(7), down: true }), "the barrel contact goes on");
    }

    #[test]
    fn a_contact_after_a_lost_end_releases_the_secondary_button_first() {
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        rewrite(&mut strokes, &start(7));
        // No end arrives for that contact; the pen touches down again with the tip alone.
        down(&mut strokes, 7, TIP);
        let events = rewrite(&mut strokes, &[start(7), end(7)].concat());
        assert_eq!(buttons(&events), [SECONDARY_UP, PRIMARY_DOWN, PRIMARY_UP]);
        assert_eq!(strokes.right, None);
        // Another pen's barrel contact takes over from one whose end was lost.
        down(&mut strokes, 7, BARREL);
        rewrite(&mut strokes, &start(7));
        down(&mut strokes, 8, BARREL);
        let events = rewrite(&mut strokes, &[start(8), end(8)].concat());
        assert_eq!(buttons(&events), [SECONDARY_UP, SECONDARY_DOWN, SECONDARY_UP]);
    }

    #[test]
    fn a_canceled_barrel_contact_releases_the_secondary_button() {
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        let events = rewrite(&mut strokes, &[start(7), vec![touch(7, TouchPhase::Cancel), Event::PointerGone]].concat());
        assert_eq!(buttons(&events), [SECONDARY_DOWN, SECONDARY_UP]);
        assert_eq!(strokes.right, None);
    }

    #[test]
    fn a_barrel_contact_that_never_pressed_releases_nothing() {
        // A finger drives the pointer, so egui-winit sends no button for the pen.
        let mut strokes = BarrelStrokes::default();
        down(&mut strokes, 7, BARREL);
        let events = [vec![touch(7, TouchPhase::Start)], vec![touch(7, TouchPhase::Cancel)]].concat();
        assert_eq!(rewrite(&mut strokes, &events), events);
        down(&mut strokes, 7, BARREL);
        down(&mut strokes, 7, TIP);
        let events = [vec![touch(7, TouchPhase::Start)], start(7)].concat();
        assert_eq!(buttons(&rewrite(&mut strokes, &events)), [PRIMARY_DOWN]);
    }

    #[test]
    fn contacts_whose_touch_never_arrives_are_bounded() {
        let mut strokes = BarrelStrokes::default();
        for id in 0..40 {
            down(&mut strokes, id, BARREL);
        }
        assert_eq!(strokes.pending.len(), MAX_PENDING);
        assert_eq!(strokes.pending.last(), Some(&(39, true)));
    }
}
