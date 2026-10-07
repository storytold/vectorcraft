//! Files Finder asks the running macOS application to open.
//!
//! winit owns `NSApplication`'s delegate and does not expose AppKit's
//! `application:openURLs:` callback. Install that optional delegate method when winit creates its
//! delegate, queue the paths, and let [`crate::App::logic`] open them on the UI thread.
#![allow(unsafe_code)]

use std::sync::{Mutex, OnceLock};

use block2::RcBlock;
use objc2::ffi;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSApplicationWillFinishLaunchingNotification};
use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSURL};

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Observe winit creating its application delegate, then add the optional open-URLs method to the
/// delegate's class. AppKit posts this notification before it delivers launch-time documents.
pub fn install() {
    let block = RcBlock::new(|_: std::ptr::NonNull<NSNotification>| install_handler());
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: `name` is an AppKit notification, `object` and `queue` accept nil, and the block is
    // copied by NotificationCenter. The returned observer intentionally lives for the process.
    let observer =
        unsafe { center.addObserverForName_object_queue_usingBlock(Some(NSApplicationWillFinishLaunchingNotification), None, None, &block) };
    std::mem::forget(observer);
}

/// Wake egui when Finder sends a document to an already-running app.
pub fn set_context(ctx: egui::Context) {
    let _ = CONTEXT.set(ctx);
}

/// Drain paths queued by AppKit. Called from the UI thread once per frame.
pub fn take() -> Vec<String> {
    PENDING.lock().map(|mut paths| std::mem::take(&mut *paths)).unwrap_or_default()
}

fn install_handler() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        log::error!("vectorcraft: macOS application delegate is missing; Finder documents cannot open");
        return;
    };
    let delegate_object: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
    let class = delegate_object.class();
    let selector = sel!(application:openURLs:);
    if class.instance_method(selector).is_some() {
        return;
    }
    // Objective-C encoding: void return, self, selector, NSApplication, NSArray<NSURL>.
    let encoding = c"v@:@@";
    // SAFETY: The function has the exact Objective-C ABI and arguments described by `encoding`.
    // winit owns this delegate class for the life of the process; adding an optional method before
    // AppKit sends open-document events is supported by the Objective-C runtime.
    let added = unsafe {
        let implementation: Imp =
            std::mem::transmute::<unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSURL>), Imp>(application_open_urls);
        ffi::class_addMethod((class as *const AnyClass).cast_mut(), selector, implementation, encoding.as_ptr())
    };
    if !added.as_bool() {
        log::error!("vectorcraft: could not install the macOS open-document handler");
    }
}

/// `NSApplicationDelegate.application(_:open:)`, installed on winit's delegate by
/// [`install_handler`]. Never lets malformed URLs or a poisoned queue unwind into AppKit.
unsafe extern "C-unwind" fn application_open_urls(_delegate: &AnyObject, _selector: Sel, _application: &AnyObject, urls: &NSArray<NSURL>) {
    let paths: Vec<String> = urls.iter().filter_map(|url| url.path()).map(|path| path.to_string()).collect();
    if paths.is_empty() {
        return;
    }
    if let Ok(mut pending) = PENDING.lock() {
        pending.extend(paths);
    } else {
        log::error!("vectorcraft: macOS open-document queue is unavailable");
        return;
    }
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_drains_the_open_document_queue() {
        if let Ok(mut paths) = PENDING.lock() {
            paths.clear();
            paths.extend(["one.svg".into(), "two.pdf".into()]);
        }
        assert_eq!(take(), ["one.svg", "two.pdf"]);
        assert!(take().is_empty());
    }
}
