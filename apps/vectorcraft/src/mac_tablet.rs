//! macOS: a drawing tablet's pen pressure (#852). winit 0.30 hands a pen to the window there as a
//! mouse, without its pressure, so the app reads it from AppKit itself and writes it into the UI's
//! [`StylusFeed`] (`Services::stylus`), which the canvas reads, as on X11 ([`crate::tablet`]).
//!
//! A local event monitor (`addLocalMonitorForEventsMatchingMask:handler:`, which AppKit calls "with
//! each event it is about to dispatch" in this app:
//! <https://developer.apple.com/documentation/appkit/nsevent/addlocalmonitorforevents(matching:handler:)>)
//! reads each mouse and tablet event on the main thread before winit's view handles it, and passes
//! it on unchanged:
//! - A mouse event whose `subtype` is `NSEventSubtypeTabletPoint` comes from a pen: its `pressure`
//!   (0..1) goes into the feed, kept for the pointer move winit makes of the event
//!   ([`StylusFeed::set_move`]), so each move of a fast stroke has its own pressure. A mouse event
//!   of any other subtype is a mouse's or a trackpad's: the feed is set to `None`.
//! - An `NSEventTypeTabletPoint` event carries the same pressure without a move.
//! - A tablet proximity event (`NSEventTypeTabletProximity`, or a mouse event of that subtype) says
//!   which end of the pen came near (`pointingDeviceType`): the eraser end's samples are marked so,
//!   and the canvas switches to the Eraser tool ([`vectorcraft_ui_egui::stylus`]). A puck
//!   (`NSPointingDeviceTypeCursor`) is a mouse.
//!
//! The monitor also turns off mouse event coalescing (`NSEvent.mouseCoalescingEnabled`), which
//! merges the drags that queue up between two reads into one: every report of the pen arrives.
//!
//! The mapping comes from PhotoCraft's `crates/tablet` (`appkit.rs` and `macos.rs`), with the
//! pressure and the pen's end only. It is plain values, tested on every platform; the monitor
//! reads them.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use vectorcraft_ui_egui::stylus::{PenSample, StylusFeed};

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
    /// The pressure and end of a pen, or `None` for a mouse.
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

/// Which end of the pen is near the tablet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pen {
    eraser: bool,
}

impl Pen {
    fn handle(&mut self, e: &RawEvent) -> Update {
        if e.proximity() {
            // Entering with the tip or the eraser end: it hovers. A puck, or leaving, is a mouse.
            self.eraser = e.entering && e.device == device::ERASER;
            let pen = e.entering && e.device != device::CURSOR;
            return Update::Set(pen.then_some(PenSample { pressure: 0.0, eraser: self.eraser }));
        }
        let sample = PenSample { pressure: e.pressure, eraser: self.eraser };
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

    fn tip(pressure: f32) -> PenSample {
        PenSample { pressure, eraser: false }
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
        let eraser = |pressure| PenSample { pressure, eraser: true };
        assert_eq!(p.handle(&proximity(device::ERASER, true)), Update::Set(Some(eraser(0.0))));
        assert_eq!(p.handle(&pen(DRAG, 0.4)), Update::Move(eraser(0.4)));
        assert_eq!(p.handle(&proximity(device::ERASER, false)), Update::Set(None), "it left");
        assert_eq!(p.handle(&pen(DRAG, 0.4)), Update::Move(tip(0.4)));
        assert_eq!(p.handle(&proximity(1, true)), Update::Set(Some(tip(0.0))), "the tip");
        assert_eq!(p.handle(&proximity(0, true)), Update::Set(Some(tip(0.0))), "an unknown tool draws as the tip");
        assert_eq!(p.handle(&proximity(device::CURSOR, true)), Update::Set(None), "a puck is a mouse");
        let e = RawEvent { kind: 5, subtype: subtype::TABLET_PROXIMITY, device: device::ERASER, entering: true, ..Default::default() };
        assert_eq!(p.handle(&e), Update::Set(Some(eraser(0.0))), "as a mouse event's subtype");
    }

    #[test]
    fn the_feed_gets_each_moves_pressure_within_0_to_1() {
        let feed = StylusFeed::default();
        let mut p = Pen::default();
        for e in [pen(DRAG, f32::NAN), pen(DRAG, 4.0), pen(DRAG, -1.0), pen(event_type::TABLET_POINT, 0.3)] {
            p.handle(&e).apply(&feed);
        }
        assert_eq!((feed.take_moves(), feed.held()), (vec![1.0, 1.0, 0.0], Some(0.3)));
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
