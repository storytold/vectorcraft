//! macOS: the window buttons (close, minimize, zoom) centred on the app bar (#968).
//!
//! AppKit puts the buttons in a standard title bar, shorter than the app bar they sit over, so they
//! rode high on it. [`center_buttons`] moves them to the middle of the bar, inset from its left as
//! far as from its top, and makes the title bar as tall as the app bar so they stay clickable.
//! AppKit lays its title bar out again when the window resizes or leaves full screen, so this runs
//! every frame and moves the buttons only when they are out of place.
//!
//! Reaching the title bar's views from the window's content view has no safe API, hence the scoped
//! `unsafe_code` allowance.
#![allow(unsafe_code)]

use objc2::runtime::AnyObject;
use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
use objc2_foundation::NSPoint;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The room the app bar keeps at its left for the buttons in AppKit's own place, before they are
/// centred (and in full screen, where AppKit shows them elsewhere): their width and a gap.
pub const DEFAULT_INSET: f32 = 78.0;

/// Differences below this (in points) leave a frame as it is, so a frame in place isn't set again.
const SLACK: f64 = 0.25;

/// Centre the window's buttons on an app bar `bar` egui points tall, at the window's `zoom` factor
/// (native points per egui point). Returns the room the app bar keeps clear at its left for them,
/// in egui points, or `None` when they aren't over the bar (full screen) or can't be found.
pub fn center_buttons(frame: &eframe::Frame, zoom: f32, bar: f32) -> Option<f32> {
    let RawWindowHandle::AppKit(handle) = frame.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: the handle's view is the window's content view, which AppKit keeps alive while the
    // window is open, and eframe hands out the handle only while it is.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return None;
    }
    let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
    let minimize = window.standardWindowButton(NSWindowButton::MiniaturizeButton)?;
    let zoom_button = window.standardWindowButton(NSWindowButton::ZoomButton)?;
    // The buttons sit in the title bar view, inside the title bar container that sets its height.
    // SAFETY: both views belong to the window, alive while it is; used right away.
    let container = unsafe { close.superview().and_then(|titlebar| titlebar.superview()) }?;
    // Another layout (a later macOS) is left as AppKit made it.
    if AnyObject::class(&container).name() != c"NSTitlebarContainerView" {
        return None;
    }
    let size = close.frame().size;
    let gap = minimize.frame().origin.x - (close.frame().origin.x + size.width);
    let height = f64::from(bar * zoom);
    let margin = (height - size.height) / 2.0;
    if !(margin.is_finite() && margin > 0.0 && gap.is_finite() && gap >= 0.0 && zoom > 0.0) {
        return None;
    }
    let mut bounds = container.frame();
    let top = window.frame().size.height - height;
    if (bounds.size.height - height).abs() > SLACK || (bounds.origin.y - top).abs() > SLACK {
        bounds.size.height = height;
        bounds.origin.y = top;
        container.setFrame(bounds);
    }
    let mut x = margin;
    for button in [close, minimize, zoom_button] {
        let at = NSPoint::new(x, (height - button.frame().size.height) / 2.0);
        let now = button.frame().origin;
        if (now.x - at.x).abs() > SLACK || (now.y - at.y).abs() > SLACK {
            button.setFrameOrigin(at);
        }
        x += size.width + gap;
    }
    // The last button's right edge (`x` steps one gap past it) and the margin again after it.
    Some(((x - gap + margin) / f64::from(zoom)) as f32)
}
