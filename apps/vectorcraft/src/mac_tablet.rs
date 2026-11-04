//! macOS: a drawing tablet's pen pressure (#852), tilt and barrel rotation (#764). winit 0.30
//! hands a pen to the window there as a mouse, without them, so the app reads them from AppKit
//! itself and writes them into the UI's [`StylusFeed`] (`Services::stylus`), which the canvas
//! reads, as on X11 ([`crate::tablet`]).
//!
//! A local event monitor (`addLocalMonitorForEventsMatchingMask:handler:`, which AppKit calls "with
//! each event it is about to dispatch" in this app:
//! <https://developer.apple.com/documentation/appkit/nsevent/addlocalmonitorforevents(matching:handler:)>)
//! reads each mouse and tablet event on the main thread before winit's view handles it, and passes
//! it on unchanged:
//! - A mouse event whose `subtype` is `NSEventSubtypeTabletPoint` comes from a pen: its `pressure`
//!   (0..1) goes into the feed, kept for the pointer move winit makes of the event
//!   ([`StylusFeed::set_move`]), so each move of a fast stroke has its own pressure. Its `tilt`
//!   (−1..1 on each axis, ±[`TILT_DEGREES`] at the ends) and its `rotation` go into the feed as the
//!   pen's pose, kept for that move too. AppKit gives a rotation of 0 for a pen that does not read
//!   one and a tilt of 0 for an upright pen, so one event cannot tell a pen without tilt or rotation
//!   from one held upright and unturned. Each reads as none until the pen reports a value other
//!   than 0 after it comes near ([`Pen`]).
//!   A mouse event of any other subtype is a mouse's or a trackpad's: the feed is set to `None`.
//! - An `NSEventTypeTabletPoint` event carries the same pressure, tilt and rotation without a move.
//! - A tablet proximity event (`NSEventTypeTabletProximity`, or a mouse event of that subtype) says
//!   which end of the pen came near (`pointingDeviceType`): the eraser end's samples are marked so,
//!   and the canvas switches to the Eraser tool ([`vectorcraft_ui_egui::stylus`]). A puck
//!   (`NSPointingDeviceTypeCursor`) is a mouse.
//!
//! The monitor also turns off mouse event coalescing (`NSEvent.mouseCoalescingEnabled`), which
//! merges the drags that queue up between two reads into one: every report of the pen arrives.
//!
//! The event mapping comes from PhotoCraft's `crates/tablet` (`appkit.rs` and `macos.rs`). The tilt
//! and rotation follow Apple's `NSEvent` documentation, as [`RawEvent::pose`] describes. The mapping
//! works on plain values and is tested on every platform; the monitor reads them.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use vectorcraft_ui_egui::stylus::{PenPose, PenSample, StylusFeed};

use crate::tablet::TILT_DEGREES;

/// `NSEventType` values (AppKit headers).
mod event_type {
    pub const TABLET_POINT: usize = 23;
    pub const TABLET_PROXIMITY: usize = 24;
    /// Left, right and other mouse down, up, moved and dragged: winit makes a pointer move of each.
    pub const MOUSE: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 25, 26, 27];
}

/// `NSEventSubtype` values of mouse events.
mod subtype {
    pub const TABLET_POINT: i16 = 1;
    pub const TABLET_PROXIMITY: i16 = 2;
}

/// `NSPointingDeviceType` values.
mod device {
    pub const CURSOR: usize = 2;
    pub const ERASER: usize = 3;
}

/// The tablet fields of one `NSEvent`, as plain values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RawEvent {
    /// `type`.
    kind: usize,
    /// `subtype` (mouse events only).
    subtype: i16,
    /// `pressure` (tablet point events and mouse events of that subtype only).
    pressure: f32,
    /// `tilt` (x, y), each −1..1 (the same events as `pressure`).
    tilt: (f64, f64),
    /// `rotation`, in degrees (the same events as `pressure`).
    rotation: f32,
    /// `pointingDeviceType` (proximity events only).
    device: usize,
    /// `isEnteringProximity` (proximity events only).
    entering: bool,
}

impl RawEvent {
    fn mouse(&self) -> bool {
        event_type::MOUSE.contains(&self.kind)
    }

    fn proximity(&self) -> bool {
        self.kind == event_type::TABLET_PROXIMITY || (self.mouse() && self.subtype == subtype::TABLET_PROXIMITY)
    }

    fn point(&self) -> bool {
        self.kind == event_type::TABLET_POINT || (self.mouse() && self.subtype == subtype::TABLET_POINT)
    }

    /// The pen's tilt and rotation in [`PenPose`]'s units.
    ///
    /// AppKit's `tilt` is −1.0..1.0 on each axis and 0.0 on both with the pen perpendicular to the
    /// tablet. A negative x is a tilt to the left and a positive x a tilt to the right; a negative y
    /// is a tilt to the top and a positive y a tilt to the bottom
    /// (<https://developer.apple.com/documentation/appkit/nsevent/tilt>). [`PenPose`]'s x is positive
    /// to the right and its y positive down the page, the same signs, so each axis keeps its sign.
    /// Apple gives the ends of the range as maximum tilt and does not give them in degrees
    /// (<https://developer.apple.com/documentation/coregraphics/cgeventfield/tableteventtiltx>), so
    /// ±1.0 is scaled to ±[`TILT_DEGREES`], the degrees the X11 reader ([`crate::tablet`]) gives the
    /// ends of a tilt range that is not in degrees. A value past ±1.0 is clamped.
    ///
    /// AppKit's `rotation` is in degrees, and a device without rotation reports 0.0
    /// (<https://developer.apple.com/documentation/appkit/nsevent/rotation>). Apple's tablet pages give
    /// no direction for rotation; the Cocoa Event Handling Guide gives the `rotation` accessor's
    /// degrees as counterclockwise for the rotate gesture
    /// (<https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/HandlingTouchEvents/HandlingTouchEvents.html>),
    /// and the pen's rotation is taken the same way. [`PenPose`]'s rotation runs clockwise, so the
    /// angle is negated and brought into 0..360.
    ///
    /// The tilt is left out unless `tilts`, and the rotation unless `turns` ([`Pen`]).
    fn pose(&self, tilts: bool, turns: bool) -> PenPose {
        let degrees = |v: f64| (v.clamp(-1.0, 1.0) * TILT_DEGREES) as f32;
        // `0.0 - r` rather than `-r`: a rotation of 0 gives 0, not −0.
        let rotation = turns.then(|| (0.0 - self.rotation).rem_euclid(360.0));
        PenPose { tilt: tilts.then(|| [degrees(self.tilt.0), degrees(self.tilt.1)]), rotation }
    }

    /// Whether the event reports a tilt other than 0, and a rotation other than 0.
    fn reads(&self) -> (bool, bool) {
        let read = |v: f64| v.is_finite() && v != 0.0;
        (read(self.tilt.0) || read(self.tilt.1), read(f64::from(self.rotation)))
    }
}

/// The events the monitor reads (`NSEventMask` bits are `1 << type`).
fn event_mask() -> u64 {
    event_type::MOUSE.iter().chain(&[event_type::TABLET_POINT, event_type::TABLET_PROXIMITY]).fold(0, |m, t| m | 1u64 << t)
}

/// What an event means for the feed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Update {
    /// Nothing: an event the monitor doesn't map.
    Keep,
    /// The sample of a pen, or `None` for a mouse.
    Set(Option<PenSample>),
    /// A pen's sample that comes with a pointer move.
    Move(PenSample),
}

impl Update {
    fn apply(self, feed: &StylusFeed) {
        match self {
            Update::Keep => {}
            Update::Set(sample) => feed.set_sample(sample),
            Update::Move(sample) => feed.set_move(sample),
        }
    }
}

/// Which end of the pen is near the tablet, and whether it has reported a tilt or a rotation other
/// than 0 since it came near: until it has, its tilt or rotation reads as none, since AppKit gives
/// 0 for a pen without one and for one held upright or unturned.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pen {
    eraser: bool,
    tilts: bool,
    turns: bool,
}

impl Pen {
    fn handle(&mut self, e: &RawEvent) -> Update {
        if e.proximity() {
            // Entering with the tip or the eraser end: it hovers. A puck, or leaving, is a mouse.
            self.eraser = e.entering && e.device == device::ERASER;
            (self.tilts, self.turns) = (false, false);
            let pen = e.entering && e.device != device::CURSOR;
            return Update::Set(pen.then_some(PenSample { pressure: 0.0, eraser: self.eraser, ..PenSample::default() }));
        }
        if e.point() {
            let (tilts, turns) = e.reads();
            self.tilts |= tilts;
            self.turns |= turns;
        }
        let sample = PenSample { pressure: e.pressure, eraser: self.eraser, pose: e.pose(self.tilts, self.turns) };
        match (e.point(), e.mouse()) {
            (true, true) => Update::Move(sample),
            (true, false) => Update::Set(Some(sample)),
            (false, true) => Update::Set(None),
            (false, false) => Update::Keep,
        }
    }
}

/// Start reading pen pressure: the feed the monitor writes into, or `None` (logged) when AppKit
/// refuses the monitor or this isn't the main thread.
#[cfg(target_os = "macos")]
pub fn start() -> Option<StylusFeed> {
    // The monitor writes on the main thread before winit hands the event on: a press never waits.
    let feed = StylusFeed::on_ui_thread();
    match monitor::install(feed.clone()) {
        Ok(()) => Some(feed),
        Err(e) => {
            log::warn!("tablet pressure: {e}");
            None
        }
    }
}

#[cfg(target_os = "macos")]
mod monitor {
    use std::cell::Cell;
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use vectorcraft_ui_egui::stylus::StylusFeed;

    use super::{Pen, RawEvent, event_mask};

    /// Install the monitor for the life of the app. Main thread only.
    pub(super) fn install(feed: StylusFeed) -> Result<(), &'static str> {
        MainThreadMarker::new().ok_or("the event monitor can only be installed from the main thread")?;
        let pen = Cell::new(Pen::default());
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit calls the handler with the event it is about to dispatch, which it
            // keeps alive while the handler runs.
            #[allow(unsafe_code)]
            let e = unsafe { event.as_ref() };
            let mut p = pen.get();
            p.handle(&read(e)).apply(&feed);
            pen.set(p);
            event.as_ptr()
        });
        // SAFETY: the handler returns the event it was given, a valid pointer ("the block's return
        // must be a valid pointer or null"). AppKit copies the block and keeps it while the monitor
        // is installed.
        #[allow(unsafe_code)]
        let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask(event_mask()), &handler) };
        // The monitor stays until the app quits: its token is never handed to `removeMonitor:`.
        std::mem::forget(token.ok_or("AppKit refused the event monitor")?);
        NSEvent::setMouseCoalescingEnabled(false);
        Ok(())
    }

    /// The tablet fields of `e`. `subtype` raises an exception for events without one, and the
    /// pressure and proximity fields are read only for the events that define them.
    fn read(e: &NSEvent) -> RawEvent {
        let mut raw = RawEvent { kind: e.r#type().0, ..Default::default() };
        if raw.mouse() {
            raw.subtype = e.subtype().0;
        }
        if raw.point() {
            raw.pressure = e.pressure();
            let tilt = e.tilt();
            raw.tilt = (tilt.x, tilt.y);
            raw.rotation = e.rotation();
        }
        if raw.proximity() {
            raw.device = e.pointingDeviceType().0;
            raw.entering = e.isEnteringProximity();
        }
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAG: usize = 6;
    const KEY_DOWN: usize = 10;

    fn pen(kind: usize, pressure: f32) -> RawEvent {
        RawEvent { kind, subtype: subtype::TABLET_POINT, pressure, ..Default::default() }
    }

    fn proximity(device: usize, entering: bool) -> RawEvent {
        RawEvent { kind: event_type::TABLET_PROXIMITY, device, entering, ..Default::default() }
    }

    /// A sample of the tip of a pen that has reported no tilt or rotation.
    fn tip(pressure: f32) -> PenSample {
        PenSample { pressure, eraser: false, pose: PenPose::default() }
    }

    /// A sample of the pen near the tablet but not yet read: no pose.
    fn near(eraser: bool) -> PenSample {
        PenSample { pressure: 0.0, eraser, pose: PenPose::default() }
    }

    #[test]
    fn a_pens_mouse_events_are_moves_with_its_pressure() {
        let mut p = Pen::default();
        assert_eq!(p.handle(&pen(DRAG, 0.5)), Update::Move(tip(0.5)));
        assert_eq!(p.handle(&pen(1, 0.25)), Update::Move(tip(0.25)), "a press");
        assert_eq!(p.handle(&pen(event_type::TABLET_POINT, 0.8)), Update::Set(Some(tip(0.8))), "no move");
    }

    #[test]
    fn other_mouse_events_are_a_mouse_and_other_events_are_kept() {
        let mut p = Pen::default();
        for subtype in [0, 3, 99, -1] {
            let e = RawEvent { kind: DRAG, subtype, pressure: 1.0, ..Default::default() };
            assert_eq!(p.handle(&e), Update::Set(None), "subtype {subtype}");
        }
        for kind in [0, KEY_DOWN, 22, 29, 34, usize::MAX] {
            // Other events' subtypes don't make them a pen's.
            let e = RawEvent { kind, subtype: subtype::TABLET_POINT, ..Default::default() };
            assert_eq!(p.handle(&e), Update::Keep, "type {kind}");
        }
    }

    #[test]
    fn proximity_says_which_end_is_near() {
        let mut p = Pen::default();
        let eraser = |pressure| PenSample { pressure, eraser: true, pose: PenPose::default() };
        assert_eq!(p.handle(&proximity(device::ERASER, true)), Update::Set(Some(near(true))));
        assert_eq!(p.handle(&pen(DRAG, 0.4)), Update::Move(eraser(0.4)));
        assert_eq!(p.handle(&proximity(device::ERASER, false)), Update::Set(None), "it left");
        assert_eq!(p.handle(&pen(DRAG, 0.4)), Update::Move(tip(0.4)));
        assert_eq!(p.handle(&proximity(1, true)), Update::Set(Some(near(false))), "the tip");
        assert_eq!(p.handle(&proximity(0, true)), Update::Set(Some(near(false))), "an unknown tool draws as the tip");
        assert_eq!(p.handle(&proximity(device::CURSOR, true)), Update::Set(None), "a puck is a mouse");
        let e = RawEvent { kind: 5, subtype: subtype::TABLET_PROXIMITY, device: device::ERASER, entering: true, ..Default::default() };
        assert_eq!(p.handle(&e), Update::Set(Some(near(true))), "as a mouse event's subtype");
    }

    /// #764: AppKit's tilt (−1..1, x positive to the right, y positive to the bottom) reaches the
    /// feed in degrees with the same signs, and its rotation, taken as counterclockwise, reaches it
    /// clockwise.
    #[test]
    fn a_pens_tilt_and_rotation_go_into_the_feed() {
        let mut p = Pen::default();
        let sample = |pressure, pose| PenSample { pressure, eraser: false, pose };
        assert_eq!(p.handle(&pen(DRAG, 0.5)), Update::Move(tip(0.5)), "no tilt or rotation reported yet");
        let leaning = RawEvent { tilt: (0.5, -0.25), rotation: 30.0, ..pen(DRAG, 0.5) };
        let pose = PenPose { tilt: Some([30.0, -15.0]), rotation: Some(330.0) };
        assert_eq!(p.handle(&leaning), Update::Move(sample(0.5, pose)));
        let past_the_ends = RawEvent { tilt: (-4.0, 1.5), rotation: 270.0, ..pen(event_type::TABLET_POINT, 0.8) };
        let pose = PenPose { tilt: Some([-60.0, 60.0]), rotation: Some(90.0) };
        assert_eq!(p.handle(&past_the_ends), Update::Set(Some(sample(0.8, pose))), "clamped");
        let pose = PenPose { tilt: Some([0.0, 0.0]), rotation: Some(0.0) };
        let Update::Move(upright) = p.handle(&pen(DRAG, 0.6)) else { panic!("a pen's drag is a move") };
        assert_eq!(upright, sample(0.6, pose), "once reported, 0 is upright and unturned");
        assert!(upright.pose.rotation.is_some_and(f32::is_sign_positive), "{upright:?}");
        let feed = StylusFeed::default();
        p.handle(&RawEvent { tilt: (f64::NAN, 0.0), rotation: -90.0, ..pen(DRAG, 0.3) }).apply(&feed);
        assert_eq!(feed.pose(), Some(PenPose { tilt: None, rotation: Some(90.0) }), "a tilt that is not a number is left out");
        p.handle(&proximity(1, true));
        let tilted = RawEvent { tilt: (0.0, 0.5), ..pen(event_type::TABLET_POINT, 0.8) };
        let pose = PenPose { tilt: Some([0.0, 30.0]), rotation: None };
        assert_eq!(p.handle(&tilted), Update::Set(Some(sample(0.8, pose))), "no rotation reported since the pen came near");
        p.handle(&RawEvent { kind: DRAG, ..Default::default() }).apply(&feed);
        assert_eq!(feed.pose(), None, "a mouse moved");
    }

    #[test]
    fn the_feed_gets_each_moves_pressure_within_0_to_1() {
        let feed = StylusFeed::default();
        let mut p = Pen::default();
        for e in [pen(DRAG, f32::NAN), pen(DRAG, 4.0), pen(DRAG, -1.0), pen(event_type::TABLET_POINT, 0.3)] {
            p.handle(&e).apply(&feed);
        }
        let pressures: Vec<f32> = feed.take_moves().into_iter().map(|(p, _)| p).collect();
        assert_eq!((pressures, feed.held()), (vec![1.0, 1.0, 0.0], Some(0.3)));
        p.handle(&RawEvent { kind: DRAG, ..Default::default() }).apply(&feed);
        assert_eq!(feed.held(), None, "a mouse moved");
    }

    #[test]
    fn the_mask_covers_the_mapped_events() {
        let m = event_mask();
        for t in [1, 2, 3, 4, 5, 6, 7, 23, 24, 25, 26, 27] {
            assert!(m & (1 << t) != 0, "{t}");
        }
        assert_eq!(m & (1 << KEY_DOWN), 0, "keys are not read");
    }
}
