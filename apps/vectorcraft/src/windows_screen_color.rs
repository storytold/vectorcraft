//! Windows: Sample Color from Screen (`vectorcraft_ui_egui::screen_color`). Windows has no system
//! color sampler, so a thread of its own copies the virtual screen, the bounding rectangle of all
//! the monitors (<https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics>),
//! into a bitmap with `BitBlt` from the screen's device context
//! (<https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdc>), then covers
//! the virtual screen with a borderless topmost window that shows that copy under a crosshair
//! cursor. The window has its own message loop on that thread, so the UI keeps drawing.
//!
//! A left click answers the copied pixel under the pointer as sRGB; Escape, a click of any other
//! button, closing the window, and the window losing the keyboard focus or the mouse capture answer
//! `None`. Each
//! click is answered when its button is released, so the release does not reach the window
//! underneath. The color comes from the copy taken before the window was shown, so the window
//! never samples itself, and moving content shows the frame it had when the copy was taken.
//!
//! The pixels are 8-bit GDI colors, read as sRGB: GDI content is taken to be sRGB when the display
//! does not use Advanced Color
//! (<https://learn.microsoft.com/en-us/windows/win32/wcs/advanced-color-icc-profiles>). The colors
//! of HDR and wide-gamut displays are not converted.
//!
//! winit makes the process per-monitor DPI aware, so the virtual screen, the pointer and the copy
//! are in physical pixels on every monitor, whatever its scale, and a monitor left of or above the
//! main one has negative coordinates
//! (<https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows>).
//!
//! The Win32 calls have no safe binding; [`overlay`] allows `unsafe_code`.
#![cfg_attr(not(windows), allow(dead_code))]

/// The status message for every failure; the details go to the log.
const FAILED: &str = "the screen color sampler failed";

/// The largest copy of the virtual screen, in bytes of 32-bit pixels: 1 GiB, eight 7680 × 4320
/// monitors. VectorCraft's own limit.
const MAX_SNAPSHOT_BYTES: u64 = 1 << 30;

/// `CLR_INVALID` (`wingdi.h`): what `GetPixel` answers for a pixel it can't read.
const CLR_INVALID: u32 = 0xFFFF_FFFF;

/// The virtual screen in physical pixels: its top-left corner (negative when a monitor is left of
/// or above the main one) and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VirtualScreen {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

impl VirtualScreen {
    /// The virtual screen `GetSystemMetrics` reports, checked before any memory is taken for its
    /// copy: an error when it is empty, larger than [`MAX_SNAPSHOT_BYTES`], or reaches past the
    /// coordinates Windows can address.
    fn new(left: i32, top: i32, width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 {
            return Err(format!("the virtual screen is {width} × {height} pixels"));
        }
        let bytes = u64::from(width.unsigned_abs()).saturating_mul(u64::from(height.unsigned_abs())).saturating_mul(4);
        if bytes > MAX_SNAPSHOT_BYTES {
            return Err(format!("the virtual screen ({width} × {height} pixels) is larger than the {MAX_SNAPSHOT_BYTES}-byte limit"));
        }
        if left.checked_add(width).is_none() || top.checked_add(height).is_none() {
            return Err(format!("the virtual screen at ({left}, {top}) reaches past the coordinate range"));
        }
        Ok(Self { left, top, width, height })
    }

    /// The pixel of the copy at the screen point (`x`, `y`), or `None` when the point is outside the
    /// virtual screen.
    fn pixel(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (px, py) = (x.checked_sub(self.left)?, y.checked_sub(self.top)?);
        ((0..self.width).contains(&px) && (0..self.height).contains(&py)).then_some((px, py))
    }
}

/// The sRGB components (0 to 1) of a `COLORREF` (`0x00bbggrr`), or `None` for `CLR_INVALID` or any
/// other value with its high byte set.
fn srgb(colorref: u32) -> Option<[f32; 3]> {
    if colorref == CLR_INVALID {
        return None;
    }
    let [r, g, b, high] = colorref.to_le_bytes();
    (high == 0).then(|| [r, g, b].map(|v| f32::from(v) / 255.0))
}

/// The client point of a mouse message. Its `lParam` holds each coordinate as a signed 16-bit
/// value (`GET_X_LPARAM` and `GET_Y_LPARAM`,
/// <https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-lbuttonup>), so a coordinate past
/// 32,767 pixels in a wide virtual screen loses its high bits. Each coordinate becomes the value
/// with the same low 16 bits nearest to `near`, the message's own pointer position (`MSG.pt`,
/// <https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-msg>) in client
/// coordinates.
fn message_point(lparam: isize, near: (i32, i32)) -> (i32, i32) {
    let widen = |low: i16, near: i32| near.saturating_add(i32::from(i32::from(low).wrapping_sub(near) as i16));
    (widen(lparam as i16, near.0), widen((lparam >> 16) as i16, near.1))
}

/// A mouse button: the left one picks, and any other one cancels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Left,
    Right,
    /// The middle button or a side button.
    Other,
}

/// What the person did in the sampler's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    Down(Button),
    Up(Button),
    Escape,
    /// The window became the active window.
    Activated,
    /// The window stopped being the active window.
    Deactivated,
    /// Another window took the mouse capture.
    CaptureLost,
    /// The window was asked to close (Alt+F4).
    Closed,
}

/// What the sampler does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Wait,
    Pick,
    Cancel,
}

/// The mouse button last pressed in the window, and whether the window has been active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Press {
    down: Option<Button>,
    active: bool,
}

impl Press {
    /// A release picks or cancels only after its button went down in the window. Losing the focus
    /// cancels only after the window had it, so a window that never became active still takes a
    /// click.
    fn step(&mut self, input: Input) -> Step {
        match input {
            Input::Down(button) => {
                self.down = Some(button);
                Step::Wait
            }
            Input::Up(button) if self.down == Some(button) => {
                if button == Button::Left {
                    Step::Pick
                } else {
                    Step::Cancel
                }
            }
            Input::Up(_) => Step::Wait,
            Input::Activated => {
                self.active = true;
                Step::Wait
            }
            Input::Deactivated if !self.active => Step::Wait,
            Input::Escape | Input::Deactivated | Input::CaptureLost | Input::Closed => Step::Cancel,
        }
    }
}

/// The screen color sampler; `ui` is woken when it answers.
#[cfg(windows)]
pub fn sampler(ui: egui::Context) -> vectorcraft_ui_egui::screen_color::StartSampler {
    Box::new(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let ui = ui.clone();
        std::thread::Builder::new()
            .name("screen-color".into())
            .spawn(move || {
                let answer = overlay::run().map_err(|detail| {
                    log::warn!("the screen color sampler gave no color: {detail}");
                    FAILED.to_string()
                });
                // The UI may be gone (the app quitting): nothing waits for the color then.
                let _ = tx.send(answer);
                ui.request_repaint();
            })
            .map_err(|e| {
                log::warn!("could not start the screen color sampler's thread: {e}");
                FAILED.to_string()
            })?;
        Ok(rx)
    })
}

/// The copy of the virtual screen and the window that shows it, on the sampler's thread.
#[cfg(windows)]
mod overlay {
    #![allow(unsafe_code)]

    use std::cell::RefCell;
    use std::sync::OnceLock;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CAPTUREBLT, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EndPaint, GetDC,
        GetPixel, HBITMAP, HDC, HGDIOBJ, PAINTSTRUCT, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HCURSOR, IDC_CROSS,
        LoadCursorW, MSG, PostQuitMessage, RegisterClassExW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOW,
        SetCursor, SetForegroundWindow, ShowWindow, TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY,
        WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN,
        WM_RBUTTONUP, WM_SETCURSOR, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };
    use windows::core::{PCWSTR, w};

    use super::{Button, Input, Press, Step, VirtualScreen, message_point, srgb};

    /// The window class of the sampler's window.
    const CLASS: PCWSTR = w!("VectorCraftScreenColor");

    /// The picked color, `None` when canceled, or the details of a failure.
    type Picked = Result<Option<[f32; 3]>, String>;

    /// A copy of the virtual screen in a memory device context. Dropping it frees the bitmap and
    /// the device context.
    struct Snapshot {
        dc: HDC,
        bitmap: HBITMAP,
        /// The bitmap the device context held before [`Snapshot::bitmap`] replaced it.
        old: HGDIOBJ,
    }

    impl Snapshot {
        /// Copies `screen` from the screen's device context, which is released on this thread, as
        /// `ReleaseDC` requires.
        fn take(screen: VirtualScreen) -> Result<Self, String> {
            // SAFETY: `GetDC(None)` takes no pointers; the device context is released below.
            let screen_dc = unsafe { GetDC(None) };
            if screen_dc.is_invalid() {
                return Err("GetDC(NULL) gave no device context".into());
            }
            let copied = Self::copy(screen_dc, screen);
            // SAFETY: the device context `GetDC(None)` gave on this thread, released once.
            unsafe { ReleaseDC(None, screen_dc) };
            copied
        }

        fn copy(screen_dc: HDC, s: VirtualScreen) -> Result<Self, String> {
            // SAFETY: `screen_dc` is a live device context; a failure answers a null handle.
            let dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
            if dc.is_invalid() {
                return Err("CreateCompatibleDC failed".into());
            }
            // SAFETY: as above; the size was checked by `VirtualScreen::new`.
            let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, s.width, s.height) };
            if bitmap.is_invalid() {
                // SAFETY: the device context made above, deleted once.
                unsafe {
                    let _ = DeleteDC(dc);
                }
                return Err(format!("CreateCompatibleBitmap gave no {} × {} bitmap", s.width, s.height));
            }
            // SAFETY: both handles are live and made above.
            let old = unsafe { SelectObject(dc, bitmap.into()) };
            if old.is_invalid() {
                // SAFETY: the handles made above, deleted once.
                unsafe {
                    let _ = DeleteObject(bitmap.into());
                    let _ = DeleteDC(dc);
                }
                return Err("SelectObject failed".into());
            }
            let snapshot = Self { dc, bitmap, old };
            // SAFETY: both device contexts are live; the rectangle is the virtual screen.
            unsafe { BitBlt(dc, 0, 0, s.width, s.height, Some(screen_dc), s.left, s.top, SRCCOPY | CAPTUREBLT) }
                .map_err(|e| format!("BitBlt of the virtual screen failed: {e}"))?;
            Ok(snapshot)
        }

        /// The `COLORREF` of the copy's pixel (`x`, `y`).
        fn colorref(&self, x: i32, y: i32) -> u32 {
            // SAFETY: the device context is live while `self` is; GetPixel checks the coordinates.
            unsafe { GetPixel(self.dc, x, y) }.0
        }
    }

    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: the handles `copy` made, freed once, the bitmap after the device context
            // lets go of it.
            unsafe {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bitmap.into());
                let _ = DeleteDC(self.dc);
            }
        }
    }

    /// The sampler shown on this thread.
    struct Session {
        snapshot: Snapshot,
        screen: VirtualScreen,
        cursor: HCURSOR,
        press: Press,
        /// The pointer position, in screen coordinates, of the message being dispatched (`MSG.pt`).
        message_pt: Option<POINT>,
        /// The answer, set once.
        answer: Option<Picked>,
        /// The window has been destroyed.
        destroyed: bool,
    }

    thread_local! {
        static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    }

    /// Runs `f` on this thread's session, or answers `None` when there is none or it is in use.
    /// Win32 calls that send messages to the window (which come back to [`window_proc`]) are made
    /// outside it.
    fn with<R>(f: impl FnOnce(&mut Session) -> R) -> Option<R> {
        SESSION.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
    }

    fn put(session: Option<Session>) -> Option<Session> {
        SESSION.with(|s| s.try_borrow_mut().ok().map(|mut s| std::mem::replace(&mut *s, session)))?
    }

    /// Shows the sampler and waits for the person; answers on this thread.
    pub(super) fn run() -> Picked {
        // SAFETY: `GetSystemMetrics` takes no pointers.
        let (left, top, width, height) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        let screen = VirtualScreen::new(left, top, width, height)?;
        let snapshot = Snapshot::take(screen)?;
        register()?;
        // SAFETY: a system cursor, loaded by its id.
        let cursor = unsafe { LoadCursorW(None, IDC_CROSS) }.map_err(|e| format!("LoadCursorW: {e}"))?;
        // Each sampler runs on a thread of its own, so the thread has no session yet.
        let _ = put(Some(Session { snapshot, screen, cursor, press: Press::default(), message_pt: None, answer: None, destroyed: false }));
        let shown = show(screen, cursor);
        if let Ok(hwnd) = shown {
            pump();
            if with(|s| s.destroyed) == Some(false) {
                // SAFETY: the window made by `show` on this thread, not yet destroyed.
                let _ = unsafe { DestroyWindow(hwnd) };
            }
        }
        let session = put(None);
        shown?;
        session.and_then(|s| s.answer).unwrap_or(Ok(None))
    }

    /// Registers the window class once for the process.
    fn register() -> Result<(), String> {
        static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
        REGISTERED
            .get_or_init(|| {
                // SAFETY: the module handle of this program; it is not freed.
                let instance = unsafe { GetModuleHandleW(None) }.map_err(|e| format!("GetModuleHandleW: {e}"))?;
                let class = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance.into(),
                    lpszClassName: CLASS,
                    ..Default::default()
                };
                // SAFETY: `class` is a complete `WNDCLASSEXW` whose strings are static.
                if unsafe { RegisterClassExW(&class) } == 0 {
                    return Err(format!("RegisterClassExW: {}", windows::core::Error::from_thread()));
                }
                Ok(())
            })
            .clone()
    }

    /// Shows the window over the virtual screen in front of every other window and takes the mouse.
    fn show(s: VirtualScreen, cursor: HCURSOR) -> Result<HWND, String> {
        // SAFETY: the module handle of this program; it is not freed.
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|e| format!("GetModuleHandleW: {e}"))?;
        // SAFETY: the class was registered by `register`; the strings are static.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS,
                w!("Sample Color from Screen"),
                WS_POPUP,
                s.left,
                s.top,
                s.width,
                s.height,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|e| format!("CreateWindowExW: {e}"))?;
        // SAFETY: the window made above, on this thread.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            SetCapture(hwnd);
            SetCursor(Some(cursor));
        }
        Ok(hwnd)
    }

    /// The window's message loop, until the window is destroyed.
    fn pump() {
        let mut msg = MSG::default();
        // SAFETY: `msg` is a valid `MSG` for each call. `GetMessageW` answers 0 for `WM_QUIT` and -1
        // for an error; both end the loop.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
            with(|s| s.message_pt = Some(msg.pt));
            // SAFETY: the message `GetMessageW` just filled in.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Answers once, then lets go of the mouse and destroys the window.
    fn finish(hwnd: HWND, answer: impl FnOnce(&mut Session) -> Picked) {
        let first = with(|s| {
            let first = s.answer.is_none();
            if first {
                s.answer = Some(answer(s));
            }
            first
        });
        if first == Some(true) {
            // SAFETY: the window on this thread; both calls send messages that `window_proc`
            // handles outside the session.
            unsafe {
                let _ = ReleaseCapture();
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    /// The copy's color at the point where the left button was released: `lparam` of that
    /// `WM_LBUTTONUP`, in `hwnd`'s client coordinates, not where the pointer is now. The pointer's
    /// current position stands in for `MSG.pt` only when the release was not dispatched by
    /// [`pump`].
    fn pick(s: &mut Session, hwnd: HWND, lparam: LPARAM) -> Picked {
        let mut origin = POINT::default();
        // SAFETY: the window on this thread and a valid `POINT`; the call sends no message.
        unsafe { ClientToScreen(hwnd, &mut origin) }.ok().map_err(|e| format!("ClientToScreen: {e}"))?;
        let pt = match s.message_pt {
            Some(pt) => pt,
            None => {
                let mut now = POINT::default();
                // SAFETY: `now` is a valid `POINT`.
                unsafe { GetCursorPos(&mut now) }.map_err(|e| format!("GetCursorPos: {e}"))?;
                now
            }
        };
        let near = (pt.x.wrapping_sub(origin.x), pt.y.wrapping_sub(origin.y));
        let (cx, cy) = message_point(lparam.0, near);
        let at = POINT { x: cx.saturating_add(origin.x), y: cy.saturating_add(origin.y) };
        let (x, y) = s.screen.pixel(at.x, at.y).ok_or_else(|| format!("the release at ({}, {}) is outside the virtual screen", at.x, at.y))?;
        let colorref = s.snapshot.colorref(x, y);
        srgb(colorref).map(Some).ok_or_else(|| format!("GetPixel({x}, {y}) answered {colorref:#010x}"))
    }

    /// Draws the copy where the virtual screen is, wherever the window's client area is.
    fn paint(hwnd: HWND) {
        let copy = with(|s| (s.snapshot.dc, s.screen));
        let mut ps = PAINTSTRUCT::default();
        let mut origin = POINT::default();
        // SAFETY: the window on this thread, with a `PAINTSTRUCT` for `BeginPaint` and `EndPaint`;
        // the copy's device context lives as long as the session.
        unsafe {
            let _ = ClientToScreen(hwnd, &mut origin);
            let target = BeginPaint(hwnd, &mut ps);
            if let Some((dc, s)) = copy
                && !target.is_invalid()
            {
                let _ = BitBlt(target, s.left.saturating_sub(origin.x), s.top.saturating_sub(origin.y), s.width, s.height, Some(dc), 0, 0, SRCCOPY);
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }

    /// What `msg` means for the sampler.
    fn input(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<Input> {
        match msg {
            WM_LBUTTONDOWN => Some(Input::Down(Button::Left)),
            WM_LBUTTONUP => Some(Input::Up(Button::Left)),
            WM_RBUTTONDOWN => Some(Input::Down(Button::Right)),
            WM_RBUTTONUP => Some(Input::Up(Button::Right)),
            WM_MBUTTONDOWN | WM_XBUTTONDOWN => Some(Input::Down(Button::Other)),
            WM_MBUTTONUP | WM_XBUTTONUP => Some(Input::Up(Button::Other)),
            WM_KEYDOWN if wparam.0 == usize::from(VK_ESCAPE.0) => Some(Input::Escape),
            WM_ACTIVATE if wparam.0 & 0xFFFF == WA_INACTIVE as usize => Some(Input::Deactivated),
            WM_ACTIVATE => Some(Input::Activated),
            // `lParam` is the window that takes the capture.
            WM_CAPTURECHANGED if lparam.0 != hwnd.0 as isize => Some(Input::CaptureLost),
            WM_CLOSE => Some(Input::Closed),
            _ => None,
        }
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // Activation also gives the window the keyboard focus, for Escape.
        // SAFETY: the arguments Windows passed in, for the window on this thread.
        let result = if msg == WM_ACTIVATE { unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } } else { LRESULT(0) };
        if let Some(input) = input(hwnd, msg, wparam, lparam) {
            match with(|s| s.press.step(input)) {
                Some(Step::Pick) => finish(hwnd, |s| pick(s, hwnd, lparam)),
                Some(Step::Cancel) => finish(hwnd, |_| Ok(None)),
                Some(Step::Wait) | None => {}
            }
            return result;
        }
        match msg {
            WM_SETCURSOR | WM_MOUSEMOVE => {
                // The cursor is set by hand: a window with the mouse capture gets no WM_SETCURSOR.
                if let Some(cursor) = with(|s| s.cursor) {
                    // SAFETY: a system cursor loaded by `run`.
                    unsafe { SetCursor(Some(cursor)) };
                }
                LRESULT(1)
            }
            // The copy covers the whole window.
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            // The window keeps covering the virtual screen when it reaches a monitor of another scale.
            WM_DPICHANGED => LRESULT(0),
            WM_DESTROY => {
                with(|s| s.destroyed = true);
                // SAFETY: ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: the arguments Windows passed in, for the window on this thread.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `COLORREF` is `0x00bbggrr`: red in the low byte. `CLR_INVALID` and other values with the high
    /// byte set are not colors.
    #[test]
    fn colorrefs_read_as_srgb() {
        assert_eq!(srgb(0x0000_00FF), Some([1.0, 0.0, 0.0]));
        assert_eq!(srgb(0x00FF_0000), Some([0.0, 0.0, 1.0]));
        assert_eq!(srgb(0x0056_3412), Some([0x12, 0x34, 0x56].map(|v| v as f32 / 255.0)));
        assert_eq!(srgb(0), Some([0.0; 3]));
        assert_eq!(srgb(0x00FF_FFFF), Some([1.0; 3]));
        assert_eq!(srgb(CLR_INVALID), None);
        assert_eq!(srgb(0x0100_0000), None);
    }

    /// The pointer maps to the copy's pixels through the virtual screen's corner, which is negative
    /// when a monitor is left of or above the main one. A 1920 × 1080 monitor at 100% left of and
    /// lower than a 3840 × 2160 one at 150% gives a 5760 × 2160 virtual screen from (−1920, 0), in
    /// physical pixels on both.
    #[test]
    fn screen_points_map_to_pixels_of_the_copy() {
        let s = VirtualScreen::new(-1920, 0, 5760, 2160).unwrap();
        assert_eq!(s.pixel(-1920, 0), Some((0, 0)));
        assert_eq!(s.pixel(-1, 300), Some((1919, 300)));
        assert_eq!(s.pixel(0, 0), Some((1920, 0)));
        assert_eq!(s.pixel(3839, 2159), Some((5759, 2159)));
        for (x, y) in [(-1921, 0), (3840, 0), (0, -1), (0, 2160), (i32::MIN, 0), (i32::MAX, i32::MAX)] {
            assert_eq!(s.pixel(x, y), None, "({x}, {y})");
        }
        // A monitor above the main one.
        let s = VirtualScreen::new(-200, -1440, 2760, 2520).unwrap();
        assert_eq!(s.pixel(0, 0), Some((200, 1440)));
        assert_eq!(s.pixel(-200, -1440), Some((0, 0)));
        // A virtual screen at the far corner of the coordinates maps without overflowing.
        let s = VirtualScreen::new(i32::MIN, i32::MIN, 10, 10).unwrap();
        assert_eq!(s.pixel(i32::MIN + 9, i32::MIN), Some((9, 0)));
        assert_eq!(s.pixel(i32::MAX, 0), None);
    }

    /// The release's own point picks, with signed coordinates: left of or above the client area
    /// they are negative. Past 32,767 pixels they are widened by the message's pointer position.
    #[test]
    fn the_release_point_comes_from_the_message() {
        let lparam = |x: i32, y: i32| ((y as u16 as isize) << 16) | x as u16 as isize;
        assert_eq!(message_point(lparam(100, 200), (140, 260)), (100, 200), "the lParam point wins over a nearby position");
        assert_eq!(message_point(lparam(65_636, 0), (65_636, 0)), (65_636, 0), "MSG.pt gives the high bits past 65,535 pixels");
        assert_eq!(message_point(lparam(100, 0), (100, 0)), (100, 0));
        assert_eq!(message_point(lparam(-5, -1), (10, 10)), (-5, -1));
        assert_eq!(message_point(lparam(0, 0), (0, 0)), (0, 0));
        assert_eq!(message_point(lparam(32_767, -32_768), (32_767, -32_768)), (32_767, -32_768));
        assert_eq!(message_point(lparam(40_010, 2_000), (40_000, 2_100)), (40_010, 2_000), "eight 8K monitors in a row");
        assert_eq!(message_point(lparam(61_439, 0), (61_000, 0)), (61_439, 0));
        assert_eq!(message_point(lparam(32_770, 0), (32_760, 0)), (32_770, 0));
        // A negative `lParam` on 64-bit Windows keeps only its low 32 bits as the point.
        assert_eq!(message_point(lparam(-3, -4) | !0xFFFF_FFFF, (0, 0)), (-3, -4));
        assert_eq!(message_point(lparam(5, 5), (i32::MAX, i32::MIN)), (i32::MAX, i32::MIN + 5), "saturates at the coordinate range");
    }

    /// The size is checked before the copy is made: an empty, negative, oversized or out-of-range
    /// virtual screen is an error.
    #[test]
    fn the_copy_size_is_checked_first() {
        assert!(VirtualScreen::new(0, 0, 7680 * 8, 4320).is_ok(), "eight 8K monitors in a row");
        for (left, top, width, height) in [
            (0, 0, 0, 1080),
            (0, 0, 1920, 0),
            (0, 0, -1920, 1080),
            (0, 0, 7680 * 8, 4370),
            (0, 0, i32::MAX, i32::MAX),
            (i32::MAX - 100, 0, 1920, 1080),
            (0, i32::MAX - 100, 1920, 1080),
        ] {
            assert!(VirtualScreen::new(left, top, width, height).is_err(), "{left} {top} {width} {height}");
        }
    }

    /// A left click picks when it is released; Escape, a click of any other button, closing the
    /// window and losing the mouse cancel; losing the focus cancels once the window had it. A
    /// release whose press began elsewhere does nothing.
    #[test]
    fn clicks_and_keys_pick_or_cancel() {
        let run = |inputs: &[Input]| {
            let mut press = Press::default();
            inputs.iter().map(|&i| press.step(i)).find(|&s| s != Step::Wait).unwrap_or(Step::Wait)
        };
        use Input::{Activated, CaptureLost, Closed, Deactivated, Escape};
        let (left_down, left_up) = (Input::Down(Button::Left), Input::Up(Button::Left));
        let (right_down, right_up) = (Input::Down(Button::Right), Input::Up(Button::Right));
        let (other_down, other_up) = (Input::Down(Button::Other), Input::Up(Button::Other));
        assert_eq!(run(&[Activated, left_down, left_up]), Step::Pick);
        assert_eq!(run(&[left_down, left_up]), Step::Pick, "a window that never became active still takes a click");
        assert_eq!(run(&[Activated, right_down, right_up]), Step::Cancel);
        assert_eq!(run(&[Activated, other_down, other_up]), Step::Cancel, "the middle and side buttons cancel");
        assert_eq!(run(&[Activated, other_down, left_up]), Step::Wait);
        assert_eq!(run(&[Activated, Escape]), Step::Cancel);
        assert_eq!(run(&[Activated, Deactivated]), Step::Cancel);
        assert_eq!(run(&[Deactivated]), Step::Wait);
        assert_eq!(run(&[CaptureLost]), Step::Cancel);
        assert_eq!(run(&[Closed]), Step::Cancel);
        assert_eq!(run(&[Activated, left_up, right_up, other_up]), Step::Wait, "releases of presses made before the window showed");
        assert_eq!(run(&[left_down, right_down, left_up]), Step::Wait, "the right button replaced the left one");
        assert_eq!(run(&[right_down, left_down, left_up]), Step::Pick);
    }
}
