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
//! the shell reads a caption-less window covering its monitor as a full-screen app (#1977). With
//! the taskbar on the top or a side edge, the strip between the window's edge and the client area
//! that winit pins to the work area (`WM_NCCALCSIZE`) is drawn as a classic title bar (#3242,
//! #3723). The guard therefore also moves a parked maximize's rect back inside the work area on
//! every edge, plus its own frame, clear of the taskbar wherever it is, and repaints the window, as
//! PhotoCraft does (storytold/photocraft#3308, storytold/photocraft#5237).
//!
//! An auto-hidden taskbar reserves no work area, so that move leaves a maximize covering the whole
//! monitor. In PhotoCraft the taskbar then did not show when the pointer reached its edge
//! (storytold/photocraft#4845). On each edge with an auto-hidden taskbar the guard stops the window
//! one pixel short of the monitor instead, as PhotoCraft does (storytold/photocraft#6878).

/// Run once per frame; a no-op once the window is borderless.
pub type WindowFrameFn = Box<dyn FnMut()>;

pub use platform::service;

#[cfg(all(windows, feature = "wgpu"))]
mod platform {
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    use winsafe::prelude::Handle;
    use winsafe::{AtomStr, HWND, HwndPlace, POINT, SIZE, co};

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
            co::SWP::FRAMECHANGED | co::SWP::NOCOPYBITS | co::SWP::NOMOVE | co::SWP::NOSIZE | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }

    /// [`super::clamped_rect`] for this frame's window: move a maximized window whose rect runs
    /// under a taskbar on any edge back inside the work area plus its frame, and repaint it. A
    /// maximized window that covers its whole monitor instead stops one pixel short of each edge
    /// with an auto-hidden taskbar ([`super::short_of_autohide_taskbars`]). No-op otherwise.
    fn keep_clear_of_the_taskbar(hwnd: &HWND) {
        if !hwnd.IsZoomed() {
            return;
        }
        let (Ok(r), Ok(mi)) = (hwnd.GetWindowRect(), hwnd.MonitorFromWindow(co::MONITOR::DEFAULTTONEAREST).GetMonitorInfo()) else {
            return;
        };
        let (m, w) = (mi.rcMonitor, mi.rcWork);
        let (rect, monitor, work) = ((r.left, r.top, r.right, r.bottom), (m.left, m.top, m.right, m.bottom), (w.left, w.top, w.right, w.bottom));
        // The taskbars are looked up (a few `FindWindowEx` calls) only while the window covers its
        // whole monitor, which it no longer does once trimmed.
        let Some((left, top, right, bottom)) = super::clamped_rect(rect, monitor, work).or_else(|| {
            super::covers(rect, monitor).then(|| super::short_of_autohide_taskbars(rect, monitor, work, &taskbar_edges(monitor))).flatten()
        }) else {
            return;
        };
        let (Some(width), Some(height)) = (right.checked_sub(left), bottom.checked_sub(top)) else {
            return;
        };
        // The frame moves while winit keeps the client rect on the work area: `NOCOPYBITS` and the
        // invalidation have the whole window drawn again instead of the old frame's pixels being
        // copied into the custom title bar.
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT { x: left, y: top },
            SIZE { cx: width, cy: height },
            co::SWP::FRAMECHANGED | co::SWP::NOCOPYBITS | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
        let _ = hwnd.InvalidateRect(None, false);
    }

    /// The edges of `monitor` that have a taskbar, shown or auto-hidden: the primary monitor's
    /// `Shell_TrayWnd` window and the other monitors' `Shell_SecondaryTrayWnd` windows.
    fn taskbar_edges(monitor: super::Rect) -> Vec<super::Edge> {
        let mut edges = Vec::new();
        for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
            let mut after: Option<HWND> = None;
            // A monitor has at most one taskbar; the bound ends the walk if the shell keeps
            // returning windows.
            for _ in 0..64 {
                let Ok(Some(tray)) = HWND::NULL.FindWindowEx(after.as_ref(), AtomStr::from_str(class), None) else {
                    break;
                };
                if let Ok(r) = tray.GetWindowRect() {
                    edges.extend(super::taskbar_edge((r.left, r.top, r.right, r.bottom), monitor));
                }
                after = Some(tray);
            }
        }
        edges
    }
}

#[cfg(not(all(windows, feature = "wgpu")))]
mod platform {
    pub fn service(_cc: &eframe::CreationContext<'_>, _custom_titlebar: bool) -> Option<super::WindowFrameFn> {
        None
    }
}

/// How far a parked maximize reaches past a monitor or work-area edge: the resize border (about
/// 8 pt at 100 %, more with DPI). Past that, a maximized window's rect is not parked but being
/// dragged or snapped, and is left alone.
const FRAME: i32 = 32;

/// The rect to move a maximize to so it stays inside the work area on every edge, plus the resize
/// border it was parked with, or `None` when it already does or `rect` (`left, top, right,
/// bottom`, desktop coordinates) is not parked at its maximize place. `monitor` is the monitor's
/// full rect and `work` its work area (the monitor less the taskbar) in the same coordinates. The
/// border is read from the edges that have no taskbar, and every edge must sit that far outside
/// either the monitor (where Windows puts a caption-less maximize) or the work area (where the
/// guard puts it), so a rect it already moved is left alone. Platform-neutral so it is tested
/// everywhere.
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
fn clamped_rect(rect: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32), work: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let (l, t, r, b) = rect;
    let (ml, mt, mr, mb) = monitor;
    let (wl, wt, wr, wb) = work;
    if ml >= mr || mt >= mb || wl < ml || wt < mt || wr > mr || wb > mb || wl >= wr || wt >= wb {
        return None;
    }
    // On an edge with a taskbar, the offsets from the monitor and from the work area can both
    // look like a border, so only the edges without one are read.
    let border = [
        (ml == wl).then(|| ml.checked_sub(l)).flatten(),
        (mt == wt).then(|| mt.checked_sub(t)).flatten(),
        (mr == wr).then(|| r.checked_sub(mr)).flatten(),
        (mb == wb).then(|| b.checked_sub(mb)).flatten(),
    ]
    .into_iter()
    .flatten()
    .filter(|n| (0..=FRAME).contains(n))
    .min()?;
    let parked = |monitor: Option<i32>, work: Option<i32>| monitor == Some(border) || work == Some(border);
    if !parked(ml.checked_sub(l), wl.checked_sub(l))
        || !parked(mt.checked_sub(t), wt.checked_sub(t))
        || !parked(r.checked_sub(mr), r.checked_sub(wr))
        || !parked(b.checked_sub(mb), b.checked_sub(wb))
    {
        return None;
    }
    let corrected = (wl.checked_sub(border)?, wt.checked_sub(border)?, wr.checked_add(border)?, wb.checked_add(border)?);
    (corrected != rect).then_some(corrected)
}

/// A rect in desktop coordinates: `(left, top, right, bottom)`, right and bottom exclusive.
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
type Rect = (i32, i32, i32, i32);

/// A monitor edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// Whether `rect` reaches every edge of `monitor`, which the shell treats as a full-screen app.
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
fn covers(rect: Rect, monitor: Rect) -> bool {
    rect.0 <= monitor.0 && rect.1 <= monitor.1 && rect.2 >= monitor.2 && rect.3 >= monitor.3
}

/// The edge of `monitor` that a taskbar with the rect `taskbar` sits on: the edge nearest the
/// taskbar's center, or `None` when the taskbar does not overlap the monitor. An auto-hidden
/// taskbar is parked mostly past its edge with a sliver left on the monitor, so its center can lie
/// outside the monitor.
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
fn taskbar_edge(taskbar: Rect, monitor: Rect) -> Option<Edge> {
    let (tl, tt, tr, tb) = (i64::from(taskbar.0), i64::from(taskbar.1), i64::from(taskbar.2), i64::from(taskbar.3));
    let (ml, mt, mr, mb) = (i64::from(monitor.0), i64::from(monitor.1), i64::from(monitor.2), i64::from(monitor.3));
    if tl >= tr || tt >= tb || ml >= mr || mt >= mb || tr <= ml || tl >= mr || tb <= mt || tt >= mb {
        return None;
    }
    // The center and the distances are doubled, so nothing is rounded.
    let (cx, cy) = (tl + tr, tt + tb);
    [(Edge::Left, cx - 2 * ml), (Edge::Top, cy - 2 * mt), (Edge::Right, 2 * mr - cx), (Edge::Bottom, 2 * mb - cy)]
        .into_iter()
        .min_by_key(|&(_, distance)| distance)
        .map(|(edge, _)| edge)
}

/// A maximize that covers its monitor, trimmed to stop one pixel short of the monitor on each of
/// the `taskbars` edges where the work area reaches the monitor's edge (the taskbar there is
/// auto-hidden). `None` when `rect` does not cover the monitor or nothing is trimmed.
#[cfg_attr(not(all(windows, feature = "wgpu")), allow(dead_code))]
fn short_of_autohide_taskbars(rect: Rect, monitor: Rect, work: Rect, taskbars: &[Edge]) -> Option<Rect> {
    if !covers(rect, monitor) {
        return None;
    }
    let (mut l, mut t, mut r, mut b) = rect;
    for edge in taskbars {
        match edge {
            Edge::Left if work.0 == monitor.0 => l = monitor.0.checked_add(1)?,
            Edge::Top if work.1 == monitor.1 => t = monitor.1.checked_add(1)?,
            Edge::Right if work.2 == monitor.2 => r = monitor.2.checked_sub(1)?,
            Edge::Bottom if work.3 == monitor.3 => b = monitor.3.checked_sub(1)?,
            _ => {}
        }
    }
    let trimmed = (l, t, r, b);
    (l < r && t < b && trimmed != rect).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::{Edge, clamped_rect, short_of_autohide_taskbars, taskbar_edge};

    #[test]
    fn a_maximize_covering_the_taskbar_is_trimmed_to_the_work_area() {
        // 1920 × 1080 with a 34 px taskbar: a caption-less maximize covers the whole monitor,
        // taskbar included (#1977). Trimmed so the rect ends at the work area plus the frame the
        // window was parked with.
        let monitor = (0, 0, 1920, 1080);
        let work = (0, 0, 1920, 1046);
        assert_eq!(clamped_rect((-7, -7, 1927, 1087), monitor, work), Some((-7, -7, 1927, 1046 + 7)));
    }

    #[test]
    fn a_maximize_already_clear_of_the_taskbar_is_left_alone() {
        // What Windows sizes a framed maximize to: the work area plus the frame.
        assert_eq!(clamped_rect((-8, -8, 1928, 1054), (0, 0, 1920, 1080), (0, 0, 1920, 1046)), None);
    }

    #[test]
    fn a_maximize_stays_clear_of_a_taskbar_on_any_edge() {
        // A taskbar on the top or a side edge (#3242, #3723): the rect moves inside the work area
        // there and keeps its frame on the other edges. Once moved, it is left alone.
        let monitor = (0, 0, 1920, 1080);
        let rect = (-8, -8, 1928, 1088);
        for (work, expected) in [
            ((0, 0, 1920, 1046), (-8, -8, 1928, 1054)),
            ((0, 48, 1920, 1080), (-8, 40, 1928, 1088)),
            ((0, 2, 1920, 1080), (-8, -6, 1928, 1088)),
            ((0, 0, 1920, 1078), (-8, -8, 1928, 1086)),
            ((48, 0, 1920, 1080), (40, -8, 1928, 1088)),
            ((0, 0, 1872, 1080), (-8, -8, 1880, 1088)),
        ] {
            assert_eq!(clamped_rect(rect, monitor, work), Some(expected), "work area {work:?}");
            assert_eq!(clamped_rect(expected, monitor, work), None, "already moved, work area {work:?}");
        }
    }

    #[test]
    fn a_top_taskbar_is_cleared_on_any_monitor_and_at_any_frame_size() {
        // A monitor above and left of the primary one, at frame sizes up to the limit.
        let monitor = (-1920, -1080, 0, 0);
        let work = (-1920, -1032, 0, 0);
        for border in [0, 8, 16, 32] {
            let rect = (-1920 - border, -1080 - border, border, border);
            let expected = (-1920 - border, -1032 - border, border, border);
            assert_eq!(clamped_rect(rect, monitor, work), Some(expected), "border {border}");
            assert_eq!(clamped_rect(expected, monitor, work), None, "border {border}");
        }
    }

    #[test]
    fn a_maximize_without_a_reserved_taskbar_is_left_alone() {
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(clamped_rect((-8, -8, 1928, 1088), monitor, monitor), None);
    }

    #[test]
    fn a_maximize_being_dragged_away_is_left_alone() {
        let monitor = (0, 0, 1920, 1080);
        let work = (0, 0, 1920, 1046);
        assert_eq!(clamped_rect((100, 200, 1100, 900), monitor, work), None, "moved");
        assert_eq!(clamped_rect((-500, -7, 1427, 1087), monitor, work), None, "resized");
        let work = (0, 48, 1920, 1080);
        assert_eq!(clamped_rect((-500, -8, 1428, 1088), monitor, work), None, "resized under a top taskbar");
        assert_eq!(clamped_rect((0, 48, 960, 1080), monitor, work), None, "snapped to half the screen");
    }

    #[test]
    fn a_maximize_stops_a_pixel_short_of_an_auto_hidden_taskbar_edge() {
        // An auto-hidden taskbar reserves no work area, so the work area is the whole monitor.
        let monitor = (0, 0, 1920, 1080);
        let rect = (-8, -8, 1928, 1088);
        for (edge, expected) in [
            (Edge::Bottom, (-8, -8, 1928, 1079)),
            (Edge::Top, (-8, 1, 1928, 1088)),
            (Edge::Left, (1, -8, 1928, 1088)),
            (Edge::Right, (-8, -8, 1919, 1088)),
        ] {
            assert_eq!(short_of_autohide_taskbars(rect, monitor, monitor, &[edge]), Some(expected), "{edge:?}");
            assert_eq!(short_of_autohide_taskbars(expected, monitor, monitor, &[edge]), None, "already trimmed, {edge:?}");
            assert_eq!(clamped_rect(expected, monitor, monitor), None, "left alone on the next frame, {edge:?}");
        }
        assert_eq!(short_of_autohide_taskbars(rect, monitor, monitor, &[]), None, "no taskbar");
        // A taskbar that reserves its work area is moved clear of by `clamped_rect` instead.
        assert_eq!(short_of_autohide_taskbars(rect, monitor, (0, 0, 1920, 1032), &[Edge::Bottom]), None, "shown taskbar");
        assert_eq!(short_of_autohide_taskbars((0, 0, 960, 1080), monitor, monitor, &[Edge::Bottom]), None, "half-screen");
        // A monitor above and left of the primary one.
        let monitor = (-1920, -1080, 0, 0);
        assert_eq!(short_of_autohide_taskbars((-1928, -1088, 8, 8), monitor, monitor, &[Edge::Top]), Some((-1928, -1079, 8, 8)));
    }

    #[test]
    fn a_taskbar_sits_on_the_monitor_edge_nearest_its_center() {
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(taskbar_edge((0, 1032, 1920, 1080), monitor), Some(Edge::Bottom), "shown");
        assert_eq!(taskbar_edge((0, 1078, 1920, 1126), monitor), Some(Edge::Bottom), "auto-hidden");
        assert_eq!(taskbar_edge((0, -46, 1920, 2), monitor), Some(Edge::Top));
        assert_eq!(taskbar_edge((-60, 0, 2, 1080), monitor), Some(Edge::Left));
        assert_eq!(taskbar_edge((1918, 0, 1980, 1080), monitor), Some(Edge::Right));
        assert_eq!(taskbar_edge((1920, 1032, 3840, 1080), monitor), None, "other monitor");
        assert!(taskbar_edge((i32::MIN, i32::MIN, i32::MAX, i32::MAX), monitor).is_some(), "no overflow");
    }
}
