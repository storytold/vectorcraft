//! Keeps the custom title bar's undecorated window truly borderless on Windows (#1758).
//!
//! winit 0.30 gives an undecorated window the `WS_CAPTION | WS_BORDER` styles and hides the
//! non-client area by spreading the client rect over it (`WM_NCCALCSIZE`). On some systems the
//! desktop compositor still reserves that frame for the window surface: the content is drawn
//! offset by the caption and border while the pointer keeps the un-offset client coordinates, so
//! it no longer points at what is drawn under it (rust-windowing/winit#2419; PhotoCraft fixed the
//! same in storytold/photocraft#2401). [`service`] strips the two styles and has Windows
//! recalculate the frame; winit puts them back whenever it changes the window flags (maximize,
//! restore, …), so the returned guard runs every frame.
//!
//! Windows sizes a maximize by the frame its styles describe, so a window maximized while its
//! caption is gone is sized without one: its rect covers the whole monitor, taskbar included, and
//! the shell reads a caption-less window covering its monitor as a full-screen app (#1977). The
//! guard therefore also trims a parked maximize's rect back to the work area plus its own frame,
//! clear of the taskbar, as PhotoCraft does (storytold/photocraft#3308). winit pins the client
//! area to the work area either way (`WM_NCCALCSIZE`), so the app's pixels do not move.

/// Run once per frame; a no-op once the window is borderless.
pub type WindowFrameFn = Box<dyn FnMut()>;

pub use platform::service;

#[cfg(all(windows, feature = "wgpu"))]
mod platform {
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    use winsafe::{HWND, HwndPlace, POINT, SIZE, co};

    /// A guard that keeps the window borderless each frame, or `None` with the system title bar
    /// (the window is decorated) or when its handle can't be found.
    pub fn service(cc: &eframe::CreationContext<'_>, custom_titlebar: bool) -> Option<super::WindowFrameFn> {
        if !custom_titlebar {
            return None;
        }
        let target = match cc.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => h.hwnd.get(),
            _ => return None,
        };
        let hwnd = find_window(target)?;
        reapply(&hwnd);
        Some(Box::new(move || reapply(&hwnd)))
    }

    /// One pass of the per-frame guard: the styles winit put back come off again, and a parked
    /// maximize stays clear of the taskbar.
    fn reapply(hwnd: &HWND) {
        strip(hwnd);
        keep_clear_of_the_taskbar(hwnd);
    }

    /// The top-level window with the given raw handle: `EnumWindows` hands out real `HWND`s, so
    /// this needs no `unsafe` to wrap a raw one.
    fn find_window(target: isize) -> Option<HWND> {
        let mut found = None;
        let _ = winsafe::EnumWindows(|hwnd| {
            if hwnd.ptr() as isize == target {
                found = Some(hwnd);
                false
            } else {
                true
            }
        });
        found
    }

    /// Removes `WS_CAPTION` (which includes `WS_BORDER`) and has Windows recalculate the frame;
    /// the sizing border stays, so edge resizing and snapping keep working.
    fn strip(hwnd: &HWND) {
        let style = hwnd.style();
        let without = style & !(co::WS::CAPTION | co::WS::BORDER);
        if without.raw() == style.raw() {
            return;
        }
        hwnd.set_style(without);
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT::new(),
            SIZE::new(),
            co::SWP::FRAMECHANGED | co::SWP::NOMOVE | co::SWP::NOSIZE | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }

    /// [`super::clamped_height`] for this frame's window: trim a maximized window whose rect runs
    /// under the taskbar back to the work area plus its frame. No-op otherwise.
    fn keep_clear_of_the_taskbar(hwnd: &HWND) {
        if !hwnd.IsZoomed() {
            return;
        }
        let (Ok(r), Ok(mi)) = (hwnd.GetWindowRect(), hwnd.MonitorFromWindow(co::MONITOR::DEFAULTTONEAREST).GetMonitorInfo()) else {
            return;
        };
        let (m, w) = (mi.rcMonitor, mi.rcWork);
        let Some(height) =
            super::clamped_height((r.left, r.top, r.right, r.bottom), (m.left, m.top, m.right, m.bottom), (w.left, w.top, w.right, w.bottom))
        else {
            return;
        };
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT::new(),
            SIZE { cx: r.right - r.left, cy: height },
            co::SWP::NOMOVE | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }
}

#[cfg(not(all(windows, feature = "wgpu")))]
mod platform {
    pub fn service(_cc: &eframe::CreationContext<'_>, _custom_titlebar: bool) -> Option<super::WindowFrameFn> {
        None
    }
}

/// How far a parked maximize reaches past its monitor on the left, top and right: the resize border
/// (about 8 pt at 100 %, more with DPI). Past that, a maximized window's rect is not parked but
/// being dragged or snapped, and is left alone.
const FRAME: i32 = 32;

/// The height to trim a maximize to so its rect stops at the taskbar, or `None` when it already
/// does or `rect` (`left, top, right, bottom`, desktop coordinates) is not parked at its maximize
/// place. `monitor` is the monitor's full rect and `work` its work area (the monitor less the
/// taskbar) in the same coordinates. The parked check is against the full monitor, because that is
/// where Windows puts a caption-less maximize; the trim is against the work area. Platform-neutral
/// so it is tested everywhere.
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
fn clamped_height(rect: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32), work: (i32, i32, i32, i32)) -> Option<i32> {
    let (l, t, r, b) = rect;
    let (ml, mt, mr, _) = monitor;
    let (_, _, _, wb) = work;
    let side = ml - l;
    if !(0..=FRAME).contains(&side) || !(0..=FRAME).contains(&(mt - t)) || !(0..=FRAME).contains(&(r - mr)) {
        return None;
    }
    if b <= wb + side {
        return None;
    }
    Some(wb + side - t)
}

#[cfg(test)]
mod tests {
    use super::clamped_height;

    #[test]
    fn a_maximize_covering_the_taskbar_is_trimmed_to_the_work_area() {
        // 1920 × 1080 with a 34 px taskbar: a caption-less maximize covers the whole monitor,
        // taskbar included (#1977). Trimmed so the rect ends at the work area plus the frame the
        // window was parked with.
        let monitor = (0, 0, 1920, 1080);
        let work = (0, 0, 1920, 1046);
        assert_eq!(clamped_height((-7, -7, 1927, 1087), monitor, work), Some(1046 + 7 - (-7)));
    }

    #[test]
    fn a_maximize_already_clear_of_the_taskbar_is_left_alone() {
        // What Windows sizes a framed maximize to: the work area plus the frame.
        assert_eq!(clamped_height((-8, -8, 1928, 1054), (0, 0, 1920, 1080), (0, 0, 1920, 1046)), None);
    }

    #[test]
    fn a_maximize_beside_a_side_taskbar_is_left_alone() {
        // Taskbar on the left: the work area starts further right, but the monitor's bottom is
        // still clear, so the rect reaches nothing.
        assert_eq!(clamped_height((-8, -8, 1928, 1088), (0, 0, 1920, 1080), (48, 0, 1920, 1080)), None);
    }

    #[test]
    fn a_maximize_being_dragged_away_is_left_alone() {
        let monitor = (0, 0, 1920, 1080);
        let work = (0, 0, 1920, 1046);
        assert_eq!(clamped_height((100, 200, 1100, 900), monitor, work), None, "moved");
        assert_eq!(clamped_height((-500, -7, 1427, 1087), monitor, work), None, "resized");
    }
}
