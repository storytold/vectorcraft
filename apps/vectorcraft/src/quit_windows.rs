//! Takes the app's windows off screen when it quits on Linux and the BSDs, before eframe drops the
//! app. Dropping the app waits up to [`vectorcraft_ui_egui::CLIPBOARD_PROBE_STOP_WAIT`] for the
//! clipboard handles to let go, and eframe closes the windows only after that wait.
//!
//! The main window is hidden on X11 (XWayland included). winit cannot hide a Wayland window, so on
//! Linux a Wayland main window is unmapped instead: its surface gets no buffer, and the window stays
//! open with the connection the Wayland clipboard copies and reads on. On Linux the X11 windows of
//! floating panels and documents are recorded as eframe creates and drops them ([`Hooks`]), and
//! quitting unmaps them through an X11 connection of its own. On native Wayland, floating panels and
//! documents are normally drawn inside the main window.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

#[cfg(target_os = "linux")]
use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

/// The main window as quitting sees it.
pub(crate) trait QuittingWindow {
    /// The window is a Wayland surface.
    fn is_wayland(&self) -> bool;
    /// Hides an X11 window.
    fn hide(&self);
    /// Takes a Wayland window off screen and keeps it open.
    fn unmap(&self);
}

impl QuittingWindow for winit::window::Window {
    #[cfg(target_os = "linux")]
    fn is_wayland(&self) -> bool {
        matches!(self.window_handle().map(|h| h.as_raw()), Ok(RawWindowHandle::Wayland(_)))
    }

    /// The BSDs have no Wayland unmap: their windows are hidden through winit.
    #[cfg(not(target_os = "linux"))]
    fn is_wayland(&self) -> bool {
        false
    }

    fn hide(&self) {
        self.set_visible(false);
    }

    #[cfg(target_os = "linux")]
    fn unmap(&self) {
        unmap_wayland(self);
    }

    #[cfg(not(target_os = "linux"))]
    fn unmap(&self) {
        self.set_visible(false);
    }
}

/// Takes the main window and the floating windows `x11` lists off screen: a Wayland main window is
/// unmapped and any other main window hidden, and `unmap_x11` unmaps the floating windows.
pub(crate) fn off_screen<W: QuittingWindow + ?Sized>(main: Option<&W>, x11: &[u32], unmap_x11: impl FnOnce(&[u32])) {
    if let Some(window) = main {
        if window.is_wayland() {
            window.unmap();
        } else {
            window.hide();
        }
    }
    if !x11.is_empty() {
        unmap_x11(x11);
    }
}

/// [`off_screen`] with the running app's main window and the floating windows [`Hooks`] recorded.
/// Runs on quit, before eframe drops the app.
pub(crate) fn before_teardown(main: Option<&winit::window::Window>) {
    #[cfg(target_os = "linux")]
    off_screen(main, &CHILD_WINDOWS.x11(), unmap_x11);
    #[cfg(not(target_os = "linux"))]
    off_screen(main, &[], |_| {});
}

/// The X11 windows of floating panels and documents, by viewport, while eframe holds them.
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct ChildWindows(Mutex<HashMap<egui::ViewportId, u32>>);

#[cfg(target_os = "linux")]
impl ChildWindows {
    /// eframe made `viewport`'s window, with `handle`. The main window, and windows of other window
    /// systems, are not recorded; a window made again for the same viewport replaces its record.
    pub(crate) fn created(&self, viewport: egui::ViewportId, handle: Option<RawWindowHandle>) {
        let mut windows = self.lock();
        match handle.and_then(x11_window) {
            Some(id) if viewport != egui::ViewportId::ROOT => windows.insert(viewport, id),
            _ => windows.remove(&viewport),
        };
    }

    /// eframe dropped `viewport`'s window.
    pub(crate) fn destroyed(&self, viewport: egui::ViewportId) {
        self.lock().remove(&viewport);
    }

    /// The recorded windows, in id order.
    pub(crate) fn x11(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.lock().values().copied().collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<egui::ViewportId, u32>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The X11 window id of `handle`, None for another window system.
#[cfg(target_os = "linux")]
fn x11_window(handle: RawWindowHandle) -> Option<u32> {
    match handle {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

/// The running app's floating windows, which [`Hooks`] records.
#[cfg(target_os = "linux")]
static CHILD_WINDOWS: LazyLock<ChildWindows> = LazyLock::new(ChildWindows::default);

/// Records the windows eframe creates and drops in [`CHILD_WINDOWS`]; set as
/// [`eframe::NativeOptions::viewport_hooks`].
#[cfg(target_os = "linux")]
pub(crate) struct Hooks;

#[cfg(target_os = "linux")]
impl eframe::ViewportHooks for Hooks {
    fn window_created(&self, viewport: egui::ViewportId, window: &winit::window::Window) {
        CHILD_WINDOWS.created(viewport, window.window_handle().ok().map(|h| h.as_raw()));
    }

    fn window_destroyed(&self, viewport: egui::ViewportId) {
        CHILD_WINDOWS.destroyed(viewport);
    }
}

/// Unmaps the X11 `windows` through a connection of its own, and waits until the server has done so.
/// An id whose window is gone gets an error, which is ignored.
#[cfg(target_os = "linux")]
fn unmap_x11(windows: &[u32]) {
    use x11rb::protocol::xproto::ConnectionExt as _;
    let conn = match x11rb::connect(None) {
        Ok((conn, _)) => conn,
        Err(e) => {
            log::warn!("couldn't reach the X server to hide the floating windows on quit: {e}");
            return;
        }
    };
    for &window in windows {
        if let Ok(cookie) = conn.unmap_window(window) {
            cookie.ignore_error();
        }
    }
    // The server answers once it has handled the requests before this one.
    if let Ok(cookie) = conn.get_input_focus() {
        let _ = cookie.reply();
    }
}

/// Attaches no buffer to `window`'s Wayland surface and commits it, which unmaps the window: the
/// compositor stops showing it, and the window and its connection stay open.
#[cfg(target_os = "linux")]
fn unmap_wayland(window: &winit::window::Window) {
    let Some((conn, surface)) = super::portal_save::wayland::surface(window) else { return };
    surface.attach(None, 0, 0);
    surface.commit();
    if let Err(e) = conn.flush() {
        log::warn!("couldn't reach the compositor to hide the window on quit: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A main window that records what quitting did to it.
    struct Window<'a> {
        wayland: bool,
        done: &'a RefCell<Vec<String>>,
    }

    impl QuittingWindow for Window<'_> {
        fn is_wayland(&self) -> bool {
            self.wayland
        }
        fn hide(&self) {
            self.done.borrow_mut().push("hide".into());
        }
        fn unmap(&self) {
            self.done.borrow_mut().push("unmap".into());
        }
    }

    fn quit(wayland: Option<bool>, x11: &[u32]) -> Vec<String> {
        let done = RefCell::new(Vec::new());
        let window = wayland.map(|wayland| Window { wayland, done: &done });
        off_screen(window.as_ref(), x11, |ids| done.borrow_mut().push(format!("unmap x11 {ids:?}")));
        done.into_inner()
    }

    /// winit cannot hide a Wayland window: quitting unmaps it, and never only asks winit to hide it.
    #[test]
    fn quitting_unmaps_a_wayland_main_window() {
        assert_eq!(quit(Some(true), &[]), ["unmap"]);
    }

    /// An X11 main window (XWayland included) is hidden, and the floating panels' and documents' X11
    /// windows are unmapped with it.
    #[test]
    fn quitting_hides_an_x11_main_window_and_its_floating_windows() {
        assert_eq!(quit(Some(false), &[]), ["hide"]);
        assert_eq!(quit(Some(false), &[7, 9]), ["hide", "unmap x11 [7, 9]"]);
    }

    /// Without a main window (none was made), the floating windows still go, and with none at all
    /// nothing is done.
    #[test]
    fn quitting_without_a_main_window() {
        assert_eq!(quit(None, &[3]), ["unmap x11 [3]"]);
        assert!(quit(None, &[]).is_empty());
    }

    #[cfg(target_os = "linux")]
    fn xlib(id: std::os::raw::c_ulong) -> Option<RawWindowHandle> {
        Some(RawWindowHandle::Xlib(winit::raw_window_handle::XlibWindowHandle::new(id)))
    }

    #[cfg(target_os = "linux")]
    fn viewport(name: &str) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(name)
    }

    /// The floating windows eframe made are recorded until it drops them; the main window, which
    /// quitting reaches through winit, is not.
    #[cfg(target_os = "linux")]
    #[test]
    fn floating_x11_windows_are_recorded_while_they_exist() {
        let windows = ChildWindows::default();
        windows.created(egui::ViewportId::ROOT, xlib(1));
        windows.created(viewport("Layers"), xlib(9));
        let xcb = winit::raw_window_handle::XcbWindowHandle::new(std::num::NonZeroU32::new(5).expect("not zero"));
        windows.created(viewport("Untitled-2"), Some(RawWindowHandle::Xcb(xcb)));
        windows.created(viewport("Color"), xlib(7));
        assert_eq!(windows.x11(), [5, 7, 9]);
        windows.destroyed(viewport("Color"));
        assert_eq!(windows.x11(), [5, 9]);
        // A viewport that eframe has already dropped, or never made, is no error.
        windows.destroyed(viewport("Color"));
        windows.destroyed(viewport("Swatches"));
        assert_eq!(windows.x11(), [5, 9]);
    }

    /// A window made again for a viewport replaces its record, and one of another window system, or
    /// one whose handle could not be read, removes it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_window_made_again_replaces_its_record() {
        let windows = ChildWindows::default();
        windows.created(viewport("Layers"), xlib(9));
        windows.created(viewport("Layers"), xlib(11));
        assert_eq!(windows.x11(), [11]);
        let surface = std::ptr::NonNull::<std::ffi::c_void>::dangling();
        let wayland = winit::raw_window_handle::WaylandWindowHandle::new(surface);
        windows.created(viewport("Layers"), Some(RawWindowHandle::Wayland(wayland)));
        assert!(windows.x11().is_empty());
        windows.created(viewport("Layers"), xlib(11));
        windows.created(viewport("Layers"), None);
        assert!(windows.x11().is_empty());
    }
}
