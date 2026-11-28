//! Windows pen tablets: with Windows Ink, winit reports a pen in contact as a touch and drops its
//! barrel button and which end of the pen is down. For each pointer message of a pen,
//! [`msg_hook`] reads the pen's `POINTER_PEN_INFO` before winit handles the message and writes its
//! pressure, end, tilt and rotation, and an airbrush pen's finger wheel, into the framework's
//! [`PenSource`] ([`start`]):
//! `PEN_FLAG_INVERTED` ("The pen is inverted") or `PEN_FLAG_ERASER` ("The eraser button is
//! pressed") is the eraser end
//! (<https://learn.microsoft.com/en-us/windows/win32/inputmsg/pen-flags-constants>). Pointer
//! messages also come while the pen hovers, so turning the pen over selects the Eraser tool before
//! it touches. A message of another pointer, a pen leaving detection range or the window, or a
//! mouse message that Windows did not make from a pen or a touch
//! (<https://learn.microsoft.com/en-us/windows/win32/tablet/system-events-and-mouse-messages>)
//! clears the source.
//!
//! The barrel button is the second-button flag of the pointer message ("analogous to a mouse right
//! button down",
//! <https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerdown>): it goes to the
//! source, whose pointer feed turns the primary button of a contact that went down with it held
//! into the secondary button.
//!
//! The same `POINTER_PEN_INFO` gives the pen's tilt and barrel rotation (#372: a 6D Art Pen turns a
//! Calligraphic brush's nib with its barrel), each only where its `PEN_MASK_*` bit says the pen
//! reports it (<https://learn.microsoft.com/en-us/windows/win32/inputmsg/pen-mask-constants>):
//! `tiltX` and `tiltY` in degrees −90..90, positive to the right and toward the user, and
//! `rotation` in degrees 0..359 clockwise
//! (<https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info>).
//! Toward the user is down the screen, so they are the sample's axes and direction as they come,
//! converted to radians. `POINTER_PEN_INFO` has no field for an airbrush pen's finger wheel, which
//! comes from the tablet driver's Wintab interface instead, when it has one
//! ([`crate::windows_wintab`]).
#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::Cell;
use std::sync::OnceLock;

use craft::behavior::PointerSample;
use craft::ui::runtime::pen::PenSource;

/// `POINTER_MESSAGE_FLAG_SECONDBUTTON` (`winuser.h`): in the high word of a pointer message's
/// `wParam`, a pen in contact with its barrel button pressed.
const POINTER_MESSAGE_FLAG_SECONDBUTTON: usize = 0x0020;
/// `POINTER_MESSAGE_FLAG_INRANGE` (`winuser.h`): not set on the message of a pointer that has left
/// detection range.
const POINTER_MESSAGE_FLAG_INRANGE: usize = 0x0002;
/// `POINTER_MESSAGE_FLAG_CANCELED` (`winuser.h`): the pointer's input was canceled.
const POINTER_MESSAGE_FLAG_CANCELED: usize = 0x8000;
/// `PEN_FLAG_INVERTED` and `PEN_FLAG_ERASER` (`winuser.h`), in `POINTER_PEN_INFO::penFlags`.
const PEN_FLAG_INVERTED: u32 = 0x0002;
const PEN_FLAG_ERASER: u32 = 0x0004;
/// `PEN_MASK_PRESSURE` (`winuser.h`), in `POINTER_PEN_INFO::penMask`: the pen reports pressure.
const PEN_MASK_PRESSURE: u32 = 0x0001;
/// `PEN_MASK_ROTATION`, `PEN_MASK_TILT_X` and `PEN_MASK_TILT_Y` (`winuser.h`): the pen reports its
/// barrel rotation, its tilt along x and its tilt along y.
const PEN_MASK_ROTATION: u32 = 0x0002;
const PEN_MASK_TILT_X: u32 = 0x0004;
const PEN_MASK_TILT_Y: u32 = 0x0008;
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

/// A pen's tilt along x and y and its barrel rotation, in radians.
type Pose = ([f32; 2], f32);

thread_local! {
    /// The pointer id of the pen whose leaving starts the Wintab wheel readings afresh: the pen that
    /// took them, or the pen whose message last asked for the wheel when none has
    /// ([`crate::windows_wintab::wheel_pen`]).
    static WHEEL_PEN: Cell<Option<u32>> = const { Cell::new(None) };
}

/// The pen source [`msg_hook`] writes, once [`start`] has made it.
static FEED: OnceLock<PenSource> = OnceLock::new();

/// The canvas's pen source, which [`msg_hook`] writes before winit handles each message.
pub fn start() -> PenSource {
    FEED.get_or_init(PenSource::default).clone()
}

/// winit's message hook (`EventLoopBuilderExtWindows::with_msg_hook`), which winit calls for each
/// message it takes from the thread's queue, before dispatching it: writes the pen's pressure, end
/// and barrel button into the source. It never consumes the message.
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
    let Some(pen) = FEED.get() else {
        return false;
    };
    crate::windows_wintab::msg_hook(msg);
    if forgets_wheel(msg.message, msg.wParam.0, WHEEL_PEN.get()) {
        crate::windows_wintab::forget();
    }
    let read_pen = || {
        let mut info = POINTER_PEN_INFO::default();
        let id = (msg.wParam.0 & 0xffff) as u32;
        // SAFETY: `info` is a `POINTER_PEN_INFO` that lives for the call. The call fails for a
        // pointer that is not a pen.
        unsafe { GetPointerPenInfo(id, &mut info) }.ok()?;
        let in_contact = info.pointerInfo.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0;
        let eraser = info.penFlags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0;
        let wheel = crate::windows_wintab::pen_wheel(msg.hwnd, msg.time, id, eraser);
        WHEEL_PEN.set(Some(crate::windows_wintab::wheel_pen(id)));
        let pose = pen_pose(info.penMask, info.tiltX, info.tiltY, info.rotation);
        Some(pen_sample(info.pressure, info.penMask, info.penFlags, in_contact, pose, wheel))
    };
    // SAFETY: `GetMessageExtraInfo` reads the extra information of the thread's last message.
    let from_mouse = || is_mouse(unsafe { GetMessageExtraInfo() }.0 as u32);
    if let Some(sample) = feed_update(msg.message, msg.wParam.0, read_pen, from_mouse) {
        // The barrel goes first: the contact's press reads it with the sample.
        pen.set_barrel(sample.is_some() && (msg.wParam.0 >> 16) & POINTER_MESSAGE_FLAG_SECONDBUTTON != 0);
        pen.set(sample);
    }
    false
}

/// What a pen whose `POINTER_PEN_INFO` holds `pressure` (0..1024), `pen_mask` and `pen_flags`
/// reports: the pressure as winit gives it to a touch, 0 while it hovers, a full press for a pen
/// in contact that reports none; the eraser end while it is inverted or its eraser button is
/// pressed; how it is held, `pose` ([`pen_pose`]); and `wheel`, the airbrush finger wheel the tablet
/// driver's Wintab interface reported for the message, if any ([`crate::windows_wintab`]), kept
/// within 0..1 and none when it is not a number.
fn pen_sample(pressure: u32, pen_mask: u32, pen_flags: u32, in_contact: bool, (tilt, rotation): Pose, wheel: Option<f32>) -> PointerSample {
    let pressure = match pressure {
        1..=1024 if pen_mask & PEN_MASK_PRESSURE != 0 => pressure as f32 / 1024.0,
        _ if in_contact => 1.0,
        _ => 0.0,
    };
    let mut sample = PointerSample::new(pressure);
    sample.tilt = tilt;
    sample.rotation = rotation;
    sample.eraser = pen_flags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0;
    sample.wheel = wheel.filter(|w| w.is_finite()).map(|w| w.clamp(0.0, 1.0));
    sample
}

/// How a pen whose `POINTER_PEN_INFO` holds `pen_mask`, `tilt_x`, `tilt_y` and `rotation` is
/// held, in radians: each tilt along an axis the pen reports (0 along one it doesn't) within ±90°,
/// and the rotation when it reports one (0 otherwise) within a turn.
fn pen_pose(pen_mask: u32, tilt_x: i32, tilt_y: i32, rotation: u32) -> Pose {
    let along = |mask: u32, t: i32| if pen_mask & mask != 0 { (t as f32).clamp(-90.0, 90.0).to_radians() } else { 0.0 };
    let turn = if pen_mask & PEN_MASK_ROTATION != 0 { ((rotation % 360) as f32).to_radians() } else { 0.0 };
    ([along(PEN_MASK_TILT_X, tilt_x), along(PEN_MASK_TILT_Y, tilt_y)], turn)
}

/// Whether a message with this `wParam` starts the Wintab wheel readings afresh: a pointer message
/// of a pointer that has left detection range or whose press was canceled, or a pointer leaving the
/// window, when the pointer is `pen`, the pen whose leaving starts them afresh. A touch or another
/// pen leaving keeps the readings.
fn forgets_wheel(message: u32, wparam: usize, pen: Option<u32>) -> bool {
    if pen != Some((wparam & 0xffff) as u32) {
        return false;
    }
    let flags = wparam >> 16;
    match message {
        WM_POINTERENTER | WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP => {
            flags & POINTER_MESSAGE_FLAG_INRANGE == 0 || flags & POINTER_MESSAGE_FLAG_CANCELED != 0
        }
        WM_POINTERLEAVE => true,
        _ => false,
    }
}

/// Whether a pointer message that Windows sends to the window procedure, so that winit's message hook
/// never sees it, starts the Wintab wheel readings afresh ([`forgets_wheel`]): `WM_POINTERENTER` and
/// `WM_POINTERLEAVE` are sent
/// (<https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerleave>); the posted pointer
/// messages are left to [`msg_hook`].
fn sent_forgets_wheel(message: u32, wparam: usize, pen: Option<u32>) -> bool {
    matches!(message, WM_POINTERENTER | WM_POINTERLEAVE) && forgets_wheel(message, wparam, pen)
}

/// A message Windows sent to a window procedure of the thread, from the Wintab reader's hook on
/// them ([`crate::windows_wintab`]): a pen leaving the window starts the wheel readings afresh, as
/// [`msg_hook`] does for the messages it sees.
#[cfg(windows)]
pub fn sent_message(message: u32, wparam: usize) {
    if sent_forgets_wheel(message, wparam, WHEEL_PEN.get()) {
        crate::windows_wintab::forget();
    }
}

/// Whether a mouse message with this extra information (`GetMessageExtraInfo`) comes from a mouse,
/// not from a pen or a touch.
fn is_mouse(extra_info: u32) -> bool {
    extra_info & SIGNATURE_MASK != MI_WP_SIGNATURE
}

/// What the source is set to for a message with this `wParam`: `Some(Some(sample))` for a pointer
/// message of a pen in range (`read_pen`), `Some(None)` for a pointer message of another pointer,
/// a pointer leaving detection range or the window, or a mouse message from a mouse
/// (`from_mouse`), and `None` to leave it as it is.
fn feed_update(
    message: u32,
    wparam: usize,
    read_pen: impl FnOnce() -> Option<PointerSample>,
    from_mouse: impl FnOnce() -> bool,
) -> Option<Option<PointerSample>> {
    match message {
        WM_POINTERENTER | WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP if (wparam >> 16) & POINTER_MESSAGE_FLAG_INRANGE == 0 => Some(None),
        WM_POINTERENTER | WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP => Some(read_pen()),
        WM_POINTERLEAVE => Some(None),
        m if MOUSE_MESSAGES.contains(&m) && from_mouse() => Some(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PEN_FLAG_BARREL` (`winuser.h`).
    const PEN_FLAG_BARREL: u32 = 0x0001;
    const UPRIGHT: Pose = ([0.0, 0.0], 0.0);

    fn eraser_sample(pressure: f32) -> PointerSample {
        let mut sample = PointerSample::new(pressure);
        sample.eraser = true;
        sample
    }

    #[test]
    fn an_inverted_pen_or_its_eraser_button_is_the_eraser_end() {
        let eraser = |flags| pen_sample(512, PEN_MASK_PRESSURE, flags, true, UPRIGHT, None).eraser;
        assert!(!eraser(0), "the tip");
        assert!(!eraser(PEN_FLAG_BARREL), "the barrel button");
        assert!(eraser(PEN_FLAG_INVERTED), "turned over");
        assert!(eraser(PEN_FLAG_ERASER), "the eraser button");
        assert!(eraser(PEN_FLAG_INVERTED | PEN_FLAG_ERASER));
        assert!(eraser(PEN_FLAG_INVERTED | PEN_FLAG_BARREL));
    }

    #[test]
    fn a_pens_tilt_and_rotation_come_where_it_reports_them() {
        let all = PEN_MASK_TILT_X | PEN_MASK_TILT_Y | PEN_MASK_ROTATION;
        let deg = f32::to_radians;
        assert_eq!(pen_pose(all, 30, -45, 90), ([deg(30.0), deg(-45.0)], deg(90.0)));
        assert_eq!(pen_pose(PEN_MASK_PRESSURE, 30, -45, 90), UPRIGHT, "a pen that reports neither");
        assert_eq!(pen_pose(PEN_MASK_TILT_X, 30, -45, 90), ([deg(30.0), 0.0], 0.0), "one axis");
        assert_eq!(pen_pose(PEN_MASK_ROTATION, 0, 0, 359), ([0.0, 0.0], deg(359.0)), "a 6D Art Pen's barrel");
        assert_eq!(pen_pose(all, 400, i32::MIN, 720), ([deg(90.0), deg(-90.0)], 0.0), "kept within the ranges");
        let pose = pen_pose(all, 10, 20, 180);
        let sample = pen_sample(512, PEN_MASK_PRESSURE | all, 0, true, pose, None);
        assert_eq!((sample.tilt, sample.rotation), pose);
    }

    #[test]
    fn an_airbrush_wheel_goes_into_the_sample() {
        assert_eq!(pen_sample(512, PEN_MASK_PRESSURE, 0, true, UPRIGHT, Some(0.25)).wheel, Some(0.25));
        let all = PEN_MASK_TILT_X | PEN_MASK_TILT_Y | PEN_MASK_ROTATION;
        let pose = pen_pose(all, 10, 20, 180);
        let sample = pen_sample(512, PEN_MASK_PRESSURE | all, 0, true, pose, Some(0.75));
        assert_eq!((sample.tilt, sample.rotation, sample.wheel), (pose.0, pose.1, Some(0.75)), "with the tilt and rotation");
        assert_eq!(sample.pressure, 0.5, "the pressure as before");
        assert_eq!(pen_sample(512, PEN_MASK_PRESSURE, 0, true, UPRIGHT, None).wheel, None, "no wheel");
        assert_eq!(pen_sample(0, 0, 0, false, UPRIGHT, Some(f32::NAN)).wheel, None, "not a number");
        assert_eq!(pen_sample(0, 0, 0, false, UPRIGHT, Some(1.5)).wheel, Some(1.0), "kept within 0..1");
    }

    #[test]
    fn leaving_range_or_the_window_and_a_cancel_forget_the_wheel() {
        let in_range = (POINTER_MESSAGE_FLAG_INRANGE << 16) | 7;
        let canceled = ((POINTER_MESSAGE_FLAG_INRANGE | POINTER_MESSAGE_FLAG_CANCELED) << 16) | 7;
        let pen = Some(7);
        for m in [WM_POINTERENTER, WM_POINTERUPDATE, WM_POINTERDOWN, WM_POINTERUP] {
            assert!(!forgets_wheel(m, in_range, pen), "{m:#x}");
            assert!(forgets_wheel(m, 7, pen), "out of range: {m:#x}");
            assert!(forgets_wheel(m, canceled, pen), "canceled: {m:#x}");
        }
        assert!(forgets_wheel(WM_POINTERLEAVE, in_range, pen));
        // WM_MOUSEMOVE and WT_PACKET.
        assert!(!forgets_wheel(0x0200, 0, Some(0)));
        assert!(!forgets_wheel(0x7FF0, 7, pen));
    }

    #[test]
    fn another_pointer_leaving_keeps_the_wheel() {
        // A touch, pointer 9, leaves the window and detection range while pen 7 hovers.
        let touch_leaves = (POINTER_MESSAGE_FLAG_INRANGE << 16) | 9;
        assert!(!forgets_wheel(WM_POINTERLEAVE, touch_leaves, Some(7)));
        assert!(!forgets_wheel(WM_POINTERUPDATE, 9, Some(7)), "out of range");
        let canceled = (POINTER_MESSAGE_FLAG_CANCELED << 16) | 9;
        assert!(!forgets_wheel(WM_POINTERUP, canceled, Some(7)), "canceled");
        assert!(!forgets_wheel(WM_POINTERLEAVE, touch_leaves, None), "no pen has read the wheel");
        assert!(forgets_wheel(WM_POINTERLEAVE, (POINTER_MESSAGE_FLAG_INRANGE << 16) | 7, Some(7)), "the pen itself");
    }

    #[test]
    fn the_sent_leave_of_the_wheel_pen_forgets_the_wheel() {
        let in_range = (POINTER_MESSAGE_FLAG_INRANGE << 16) | 7;
        let pen = Some(7);
        assert!(sent_forgets_wheel(WM_POINTERLEAVE, in_range, pen), "the pen leaves the window");
        assert!(sent_forgets_wheel(WM_POINTERLEAVE, 7, pen), "or detection range");
        assert!(!sent_forgets_wheel(WM_POINTERLEAVE, (POINTER_MESSAGE_FLAG_INRANGE << 16) | 9, pen), "another pointer leaves");
        assert!(!sent_forgets_wheel(WM_POINTERLEAVE, in_range, None), "no pen has read the wheel");
        assert!(!sent_forgets_wheel(WM_POINTERENTER, in_range, pen), "the pen comes over the window");
        // Posted messages, which the message hook already sees.
        for m in [WM_POINTERUPDATE, WM_POINTERDOWN, WM_POINTERUP] {
            assert!(!sent_forgets_wheel(m, 7, pen), "{m:#x}");
        }
        // WT_CTXCLOSE.
        assert!(!sent_forgets_wheel(0x7FF2, 7, pen));
    }

    #[test]
    fn a_pens_pressure_is_the_one_winit_gives_its_touch() {
        assert_eq!(pen_sample(512, PEN_MASK_PRESSURE, 0, true, UPRIGHT, None).pressure, 0.5);
        assert_eq!(pen_sample(1024, PEN_MASK_PRESSURE, 0, true, UPRIGHT, None).pressure, 1.0);
        assert_eq!(pen_sample(0, PEN_MASK_PRESSURE, PEN_FLAG_INVERTED, false, UPRIGHT, None), eraser_sample(0.0), "hovering");
        assert_eq!(pen_sample(0, 0, 0, true, UPRIGHT, None).pressure, 1.0, "a pen in contact that reports no pressure presses fully");
        assert_eq!(pen_sample(300, 0, 0, true, UPRIGHT, None).pressure, 1.0, "a pressure without the pressure mask is not one");
        assert_eq!(pen_sample(0, PEN_MASK_PRESSURE, 0, true, UPRIGHT, None).pressure, 1.0, "as winit, which gives no force for 0");
        assert_eq!(pen_sample(2000, PEN_MASK_PRESSURE, 0, false, UPRIGHT, None).pressure, 0.0);
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
        let pen = eraser_sample(0.0);
        // `POINTER_MESSAGE_FLAG_INRANGE` in the high word of `wParam`, pointer 7 in the low word.
        let in_range = (POINTER_MESSAGE_FLAG_INRANGE << 16) | 7;
        let no_mouse = || -> bool { panic!("a pointer message is not a mouse message") };
        let no_pen = || -> Option<PointerSample> { panic!("no pen to read") };
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
}
