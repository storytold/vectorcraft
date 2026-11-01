//! Windows pen tablets: with Windows Ink, winit reports a pen in contact as a touch and drops its
//! barrel button, and egui-winit presses the primary button for the touch it makes the pointer,
//! so a pen pressed with its barrel button held clicks and drags with the left button. The
//! `WM_POINTERDOWN` message winit is about to handle still has the barrel button's state in its
//! second-button flag ("analogous to a mouse right button down",
//! <https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerdown>): [`msg_hook`] notes
//! it, and [`raw_input_hook`] makes the primary button of that contact the secondary one.
//!
//! winit also drops which end of the pen is down. For each pointer message of a pen, [`msg_hook`]
//! reads the pen's `POINTER_PEN_INFO` and writes its pressure and end into the canvas's pen feed
//! ([`start`]): `PEN_FLAG_INVERTED` ("The pen is inverted") or `PEN_FLAG_ERASER` ("The eraser
//! button is pressed") is the eraser end
//! (<https://learn.microsoft.com/en-us/windows/win32/inputmsg/pen-flags-constants>). Pointer
//! messages also come while the pen hovers, so turning the pen over selects the Eraser tool before
//! it touches. A message of another pointer, a pen leaving detection range or the window, or a
//! mouse message that Windows did not make from a pen or a touch
//! (<https://learn.microsoft.com/en-us/windows/win32/tablet/system-events-and-mouse-messages>)
//! clears the feed.
#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::RefCell;
use std::sync::OnceLock;

use egui::{Event, Modifiers, PointerButton, Pos2, TouchId, TouchPhase};
use vectorcraft_ui_egui::stylus::{PenSample, StylusFeed};

/// `POINTER_MESSAGE_FLAG_SECONDBUTTON` (`winuser.h`): in the high word of a pointer message's
/// `wParam`, a pen in contact with its barrel button pressed.
const POINTER_MESSAGE_FLAG_SECONDBUTTON: usize = 0x0020;
/// `POINTER_MESSAGE_FLAG_INRANGE` (`winuser.h`): not set on the message of a pointer that has left
/// detection range.
const POINTER_MESSAGE_FLAG_INRANGE: usize = 0x0002;
/// At most this many pointer contacts wait for their touch event.
const MAX_PENDING: usize = 16;
/// `PEN_FLAG_INVERTED` and `PEN_FLAG_ERASER` (`winuser.h`), in `POINTER_PEN_INFO::penFlags`.
const PEN_FLAG_INVERTED: u32 = 0x0002;
const PEN_FLAG_ERASER: u32 = 0x0004;
/// `PEN_MASK_PRESSURE` (`winuser.h`), in `POINTER_PEN_INFO::penMask`: the pen reports pressure.
const PEN_MASK_PRESSURE: u32 = 0x0001;
/// `MI_WP_SIGNATURE` and `SIGNATURE_MASK`: the extra information of a mouse message that Windows
/// made from a pen or a touch.
const MI_WP_SIGNATURE: u32 = 0xFF51_5700;
const SIGNATURE_MASK: u32 = 0xFFFF_FF00;
/// `WM_MOUSEFIRST` to `WM_MOUSELAST` (`winuser.h`).
const MOUSE_MESSAGES: std::ops::RangeInclusive<u32> = 0x0200..=0x020E;
/// `WM_POINTERUPDATE`, `WM_POINTERDOWN`, `WM_POINTERUP`, `WM_POINTERENTER` and `WM_POINTERLEAVE`
/// (`winuser.h`).
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERENTER: u32 = 0x0249;
const WM_POINTERLEAVE: u32 = 0x024A;

thread_local! {
    static STROKES: RefCell<BarrelStrokes> = RefCell::new(BarrelStrokes::default());
}

/// The pen feed [`msg_hook`] writes, once [`start`] has made it.
static FEED: OnceLock<StylusFeed> = OnceLock::new();

/// The canvas's pen feed, which [`msg_hook`] writes on the UI thread before winit handles each
/// message, so a press never waits for it.
pub fn start() -> StylusFeed {
    FEED.get_or_init(StylusFeed::on_ui_thread).clone()
}

/// winit's message hook (`EventLoopBuilderExtWindows::with_msg_hook`), which winit calls for each
/// message it takes from the thread's queue, before dispatching it: notes each pointer contact's id
/// and whether the barrel button was held, and writes the pen's pressure and end into the feed.
/// It never consumes the message.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn msg_hook(msg: *const std::ffi::c_void) -> bool {
    use windows::Win32::UI::Input::Pointer::{GetPointerPenInfo, POINTER_FLAG_INCONTACT, POINTER_PEN_INFO};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageExtraInfo, MSG};
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
    let Some(feed) = FEED.get() else {
        return false;
    };
    let read_pen = || {
        let mut info = POINTER_PEN_INFO::default();
        // SAFETY: `info` is a `POINTER_PEN_INFO` that lives for the call. The call fails for a
        // pointer that is not a pen.
        unsafe { GetPointerPenInfo((msg.wParam.0 & 0xffff) as u32, &mut info) }.ok()?;
        let in_contact = info.pointerInfo.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0;
        Some(pen_sample(info.pressure, info.penMask, info.penFlags, in_contact))
    };
    // SAFETY: `GetMessageExtraInfo` reads the extra information of the thread's last message.
    let from_mouse = || is_mouse(unsafe { GetMessageExtraInfo() }.0 as u32);
    if let Some(sample) = feed_update(msg.message, msg.wParam.0, read_pen, from_mouse) {
        feed.set_sample(sample);
    }
    false
}

/// What a pen whose `POINTER_PEN_INFO` holds `pressure` (0..1024), `pen_mask` and `pen_flags`
/// reports: the pressure as winit gives it to a touch, 0 while it hovers, a full press for a pen
/// in contact that reports none; and the eraser end while it is inverted or its eraser button is
/// pressed.
fn pen_sample(pressure: u32, pen_mask: u32, pen_flags: u32, in_contact: bool) -> PenSample {
    let pressure = match pressure {
        1..=1024 if pen_mask & PEN_MASK_PRESSURE != 0 => pressure as f32 / 1024.0,
        _ if in_contact => 1.0,
        _ => 0.0,
    };
    PenSample { pressure, eraser: pen_flags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0 }
}

/// Whether a mouse message with this extra information (`GetMessageExtraInfo`) comes from a mouse,
/// not from a pen or a touch.
fn is_mouse(extra_info: u32) -> bool {
    extra_info & SIGNATURE_MASK != MI_WP_SIGNATURE
}

/// What the feed is set to for a message with this `wParam`: `Some(Some(sample))` for a pointer
/// message of a pen in range (`read_pen`), `Some(None)` for a pointer message of another pointer,
/// a pointer leaving detection range or the window, or a mouse message from a mouse
/// (`from_mouse`), and `None` to leave it as it is.
fn feed_update(
    message: u32,
    wparam: usize,
    read_pen: impl FnOnce() -> Option<PenSample>,
    from_mouse: impl FnOnce() -> bool,
) -> Option<Option<PenSample>> {
    match message {
        WM_POINTERENTER | WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP if (wparam >> 16) & POINTER_MESSAGE_FLAG_INRANGE == 0 => Some(None),
        WM_POINTERENTER | WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP => Some(read_pen()),
        WM_POINTERLEAVE => Some(None),
        m if MOUSE_MESSAGES.contains(&m) && from_mouse() => Some(None),
        _ => None,
    }
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

    /// `PEN_FLAG_BARREL` (`winuser.h`).
    const PEN_FLAG_BARREL: u32 = 0x0001;

    #[test]
    fn an_inverted_pen_or_its_eraser_button_is_the_eraser_end() {
        let eraser = |flags| pen_sample(512, PEN_MASK_PRESSURE, flags, true).eraser;
        assert!(!eraser(0), "the tip");
        assert!(!eraser(PEN_FLAG_BARREL), "the barrel button");
        assert!(eraser(PEN_FLAG_INVERTED), "turned over");
        assert!(eraser(PEN_FLAG_ERASER), "the eraser button");
        assert!(eraser(PEN_FLAG_INVERTED | PEN_FLAG_ERASER));
        assert!(eraser(PEN_FLAG_INVERTED | PEN_FLAG_BARREL));
    }

    #[test]
    fn a_pens_pressure_is_the_one_winit_gives_its_touch() {
        assert_eq!(pen_sample(512, PEN_MASK_PRESSURE, 0, true).pressure, 0.5);
        assert_eq!(pen_sample(1024, PEN_MASK_PRESSURE, 0, true).pressure, 1.0);
        assert_eq!(pen_sample(0, PEN_MASK_PRESSURE, PEN_FLAG_INVERTED, false), PenSample { pressure: 0.0, eraser: true }, "hovering");
        assert_eq!(pen_sample(0, 0, 0, true).pressure, 1.0, "a pen in contact that reports no pressure presses fully");
        assert_eq!(pen_sample(300, 0, 0, true).pressure, 1.0, "a pressure without the pressure mask is not one");
        assert_eq!(pen_sample(0, PEN_MASK_PRESSURE, 0, true).pressure, 1.0, "as winit, which gives no force for 0");
        assert_eq!(pen_sample(2000, PEN_MASK_PRESSURE, 0, false).pressure, 0.0);
    }

    #[test]
    fn mouse_messages_from_a_pen_or_a_touch_are_told_apart() {
        assert!(is_mouse(0), "a mouse");
        assert!(!is_mouse(0xFF51_5701), "a pen");
        assert!(!is_mouse(0xFF51_5780), "a touch");
        assert!(is_mouse(0xFF51_5600));
    }

    #[test]
    fn pen_messages_set_the_feed_and_other_pointers_clear_it() {
        let pen = PenSample { pressure: 0.0, eraser: true };
        // `POINTER_MESSAGE_FLAG_INRANGE` in the high word of `wParam`, pointer 7 in the low word.
        let in_range = (POINTER_MESSAGE_FLAG_INRANGE << 16) | 7;
        let no_mouse = || -> bool { panic!("a pointer message is not a mouse message") };
        let no_pen = || -> Option<PenSample> { panic!("no pen to read") };
        for m in [WM_POINTERENTER, WM_POINTERUPDATE, WM_POINTERDOWN, WM_POINTERUP] {
            assert_eq!(feed_update(m, in_range, || Some(pen), no_mouse), Some(Some(pen)), "{m:#x}");
            assert_eq!(feed_update(m, in_range, || None, no_mouse), Some(None), "a touch: {m:#x}");
            assert_eq!(feed_update(m, 7, no_pen, no_mouse), Some(None), "out of range: {m:#x}");
        }
        assert_eq!(feed_update(WM_POINTERLEAVE, in_range, no_pen, no_mouse), Some(None));
        // WM_MOUSEMOVE and WM_LBUTTONDOWN.
        assert_eq!(feed_update(0x0200, 0, no_pen, || true), Some(None), "a mouse moved");
        assert_eq!(feed_update(0x0201, 0, no_pen, || false), None, "made from a pen");
        // WM_KEYDOWN.
        assert_eq!(feed_update(0x0100, 0, no_pen, || -> bool { panic!("not a mouse message") }), None);
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
