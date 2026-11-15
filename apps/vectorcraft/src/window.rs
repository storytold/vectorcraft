//! The main window's size and position. The first launch fits the window to its monitor, opening
//! it maximized where the default size leaves no room for a taskbar or Dock. Later launches restore
//! the size, position and maximized state saved with the UI preferences (`UiState::window`), moved
//! onto a monitor that is still connected.

use vectorcraft_ui_egui::DockMonitor;
use vectorcraft_ui_egui::state::WindowGeometry;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};

/// The first window's size, in points.
pub const DEFAULT_SIZE: [f32; 2] = [1440.0, 900.0];
/// The smallest window, in points.
pub const MIN_SIZE: [f32; 2] = [800.0, 500.0];
/// Room the first window leaves around itself for a taskbar, Dock or menu bar (the system doesn't
/// say how much of the screen they take): a monitor with less room opens it maximized.
const SCREEN_ROOM: [f32; 2] = [64.0, 128.0];

/// Seconds between readings of the monitor list, which picks up a monitor connected while the app runs.
const MONITORS_EVERY: f64 = 1.0;

/// A monitor: its top-left corner and size in physical pixels, and its scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pos: [i64; 2],
    size: [i64; 2],
    scale: f32,
}

impl Monitor {
    pub fn new(pos: PhysicalPosition<i32>, size: PhysicalSize<u32>, scale: f64) -> Self {
        let scale = scale as f32;
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        Self { pos: [pos.x.into(), pos.y.into()], size: [size.width.into(), size.height.into()], scale }
    }

    /// Its size in points.
    fn points(&self) -> [f32; 2] {
        self.size.map(|s| s as f32 / self.scale)
    }

    /// Its size in points less room for a taskbar, Dock or menu bar.
    fn usable(&self) -> [f32; 2] {
        let points = self.points();
        std::array::from_fn(|i| points[i] - SCREEN_ROOM[i])
    }

    /// A window of `size` points on this monitor, in physical pixels.
    fn pixels(&self, size: [f32; 2]) -> [i64; 2] {
        size.map(|s| (s * self.scale) as i64)
    }

    /// How much of a window at `pos` of `size` points this monitor shows, in square pixels.
    fn overlap(&self, pos: [i32; 2], size: [f32; 2]) -> i64 {
        let px = self.pixels(size);
        let span = |i: usize| {
            let start = i64::from(pos[i]).max(self.pos[i]);
            let end = i64::from(pos[i]).saturating_add(px[i]).min(self.pos[i] + self.size[i]);
            (end - start).max(0)
        };
        span(0).saturating_mul(span(1))
    }

    /// The top-left corner nearest to `pos` (centred when none) that keeps a window of `size`
    /// points on this monitor.
    fn keep_on(&self, pos: Option<[i32; 2]>, size: [f32; 2]) -> [i32; 2] {
        let px = self.pixels(size);
        std::array::from_fn(|i| {
            let lo = self.pos[i];
            let hi = lo + self.size[i].saturating_sub(px[i]).max(0);
            let want = pos.map_or(lo + (hi - lo) / 2, |p| i64::from(p[i]));
            i32::try_from(want.max(lo).min(hi)).unwrap_or_default()
        })
    }

    /// This monitor for floating panels, in the points egui reports for a window on it: physical
    /// pixels divided by the monitor's scale factor and the UI zoom.
    fn docking(&self, name: Option<String>, zoom: f32) -> DockMonitor {
        let scale = self.scale * zoom;
        let [x, y] = self.pos;
        let [w, h] = self.size;
        DockMonitor { name, rect: [x, y, w, h].map(|v| v as f32 / scale), scale }
    }
}

/// Where the window opens: `saved` (the last session's geometry) kept on a connected monitor, else
/// the default size on `home` (the monitor the system opens it on). Without monitors (the system
/// doesn't list them, so nothing to check a saved geometry against) the default size.
pub fn place(saved: Option<WindowGeometry>, monitors: &[Monitor], home: Option<usize>) -> WindowGeometry {
    let default = WindowGeometry { pos: None, size: DEFAULT_SIZE, maximized: false };
    let Some(home) = home.and_then(|i| monitors.get(i)).or(monitors.first()) else {
        return default;
    };
    let Some(saved) = saved else {
        let usable = home.usable();
        if (0..2).all(|i| DEFAULT_SIZE[i] <= usable[i]) {
            return default;
        }
        // A small screen: maximized, with a size that fits once un-maximized.
        let size = fit(DEFAULT_SIZE, usable);
        return WindowGeometry { pos: Some(home.keep_on(None, size)), size, maximized: true };
    };
    // The monitor showing most of the window; none when it was on a monitor that is gone.
    let shown = saved.pos.and_then(|p| {
        let (area, m) = monitors.iter().map(|m| (m.overlap(p, saved.size), m)).max_by_key(|(area, _)| *area)?;
        (area > 0).then_some(m)
    });
    let m = shown.unwrap_or(home);
    // Bigger than its monitor (a smaller screen than last time): maximized, like the first window.
    let room = m.points();
    let too_big = (0..2).any(|i| saved.size[i] > room[i]);
    let size = fit(saved.size, if too_big { m.usable() } else { room });
    WindowGeometry {
        // Moved fully onto its monitor, or centred on the home monitor.
        pos: saved.pos.map(|p| m.keep_on(shown.map(|_| p), size)),
        size,
        maximized: saved.maximized || too_big,
    }
}

/// `size` no bigger than `room`, and no smaller than the smallest window unless `room` is.
fn fit(size: [f32; 2], room: [f32; 2]) -> [f32; 2] {
    std::array::from_fn(|i| size[i].min(room[i]).max(MIN_SIZE[i].min(room[i])).max(1.0))
}

/// Whether to list the monitors again: on the first frame, when the UI zoom changed, and at most
/// every [`MONITORS_EVERY`] seconds while the app draws. `listed` holds the last list's time and zoom.
pub fn monitors_due(listed: Option<(f64, f32)>, now: f64, zoom: f32) -> bool {
    listed.is_none_or(|(at, listed_zoom)| listed_zoom != zoom || now - at >= MONITORS_EVERY)
}

/// The connected monitors for floating panels (`Services::dock_monitors`).
pub fn dock_monitors(window: &winit::window::Window, zoom: f32) -> Vec<DockMonitor> {
    let monitors: Vec<_> = window.available_monitors().map(|m| (Monitor::new(m.position(), m.size(), m.scale_factor()), m.name())).collect();
    docking_list(&monitors, zoom, cfg!(target_os = "macos"))
}

/// The list for floating panels, empty when `monitors` have different scales and `shared_points` is
/// false. macOS has one desktop of points. Elsewhere a new window multiplies its saved position by the
/// scale of the monitor it is created on, which can put a window saved on another scale off every
/// monitor. With an empty list craft-ui opens the window on the main window's monitor.
///
/// A name that two monitors share is left out, so craft-ui finds those monitors by their rects: macOS
/// can give two displays of one model the same name.
fn docking_list(monitors: &[(Monitor, Option<String>)], zoom: f32, shared_points: bool) -> Vec<DockMonitor> {
    if !shared_points && monitors.windows(2).any(|pair| pair[0].0.scale != pair[1].0.scale) {
        return Vec::new();
    }
    let unique = |name: &String| monitors.iter().filter(|(_, other)| other.as_ref() == Some(name)).count() == 1;
    monitors.iter().map(|(monitor, name)| monitor.docking(name.clone().filter(unique), zoom)).collect()
}

/// Size and place the new window (eframe shows it after drawing the first frame) and return its
/// geometry. On Linux the window may not be mapped yet, so [`apply_pending_maximize`] must run
/// until the host reports maximized (fixes #3635).
pub fn restore(window: &winit::window::Window, saved: Option<WindowGeometry>) -> WindowGeometry {
    let handles: Vec<_> = window.available_monitors().collect();
    let monitors: Vec<Monitor> = handles.iter().map(|m| Monitor::new(m.position(), m.size(), m.scale_factor())).collect();
    let home = window.current_monitor().or_else(|| window.primary_monitor()).and_then(|c| handles.iter().position(|m| *m == c));
    let g = place(saved, &monitors, home);
    if let Some([x, y]) = g.pos {
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
    // Returns the new size when it applies at once; either way the window gets it.
    let _ = window.request_inner_size(LogicalSize::new(g.size[0], g.size[1]));
    #[cfg(not(target_os = "linux"))]
    if g.maximized {
        window.set_maximized(true);
    }
    g
}

/// Keep asking the host to maximize until it sticks, or give up after a few frames.
pub fn apply_pending_maximize(window: &winit::window::Window, want: bool, tries: &mut u8) -> bool {
    if !want {
        return false;
    }
    if window.is_maximized() {
        return false;
    }
    if *tries == 0 {
        return false;
    }
    *tries -= 1;
    window.set_maximized(true);
    true
}

/// Per frame: remember the window's geometry for the next launch. Minimized or full screen keeps
/// the last one; maximized only sets its flag, so un-maximizing next time restores the last size.
/// `host_maximized` is the window manager's idea of maximized when egui does not report it (#3635).
pub fn track(ctx: &egui::Context, geometry: &mut Option<WindowGeometry>, host_maximized: Option<bool>) {
    let zoom = ctx.zoom_factor();
    ctx.input(|i| {
        let v = i.viewport();
        if v.minimized == Some(true) || v.fullscreen == Some(true) {
            return;
        }
        let maximized = v.maximized == Some(true) || host_maximized == Some(true);
        if maximized {
            if let Some(g) = geometry {
                g.maximized = true;
            }
            return;
        }
        let Some(inner) = v.inner_rect else { return };
        let pos = v.outer_rect.map(|r| [r.min.x, r.min.y].map(|c| (c * i.pixels_per_point).round() as i32));
        *geometry = Some(WindowGeometry { pos, size: [inner.width() * zoom, inner.height() * zoom], maximized: false });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64) -> Monitor {
        Monitor::new(PhysicalPosition::new(x, y), PhysicalSize::new(w, h), scale)
    }

    fn saved(pos: Option<[i32; 2]>, size: [f32; 2], maximized: bool) -> Option<WindowGeometry> {
        Some(WindowGeometry { pos, size, maximized })
    }

    #[test]
    fn the_first_window_fits_its_screen() {
        // 1920 × 1080: the default size, where the system puts it.
        let big = [monitor(0, 0, 1920, 1080, 1.0)];
        assert_eq!(place(None, &big, Some(0)), WindowGeometry { pos: None, size: DEFAULT_SIZE, maximized: false });
        // 1600 × 900, 1366 × 768 and 1920 × 1080 at 150% (1280 × 720 points): maximized, and a
        // size that fits the screen with room for the taskbar once un-maximized.
        for (m, size) in [
            (monitor(0, 0, 1600, 900, 1.0), [1440.0, 772.0]),
            (monitor(0, 0, 1366, 768, 1.0), [1302.0, 640.0]),
            (monitor(0, 0, 1920, 1080, 1.5), [1216.0, 592.0]),
        ] {
            let g = place(None, &[m], Some(0));
            assert!(g.maximized, "{m:?}");
            assert_eq!(g.size, size, "{m:?}");
            let [x, y] = g.pos.unwrap();
            let px = m.pixels(g.size);
            assert!(x >= 0 && y >= 0 && i64::from(x) + px[0] <= m.size[0] && i64::from(y) + px[1] <= m.size[1], "{m:?}: {g:?}");
        }
        // No monitor information: the default size, whatever was saved.
        assert_eq!(place(None, &[], None), WindowGeometry { pos: None, size: DEFAULT_SIZE, maximized: false });
        assert_eq!(place(saved(Some([9000, 9000]), [1e9, 1e9], true), &[], None).size, DEFAULT_SIZE);
    }

    #[test]
    fn floating_panels_get_monitors_in_the_points_a_window_on_them_reports() {
        let dock = |m: Monitor, zoom| m.docking(Some("m".into()), zoom);
        let plain = dock(monitor(1920, 0, 1920, 1080, 1.0), 1.0);
        assert_eq!(plain, DockMonitor { name: Some("m".into()), rect: [1920.0, 0.0, 1920.0, 1080.0], scale: 1.0 });
        // A window at (3000, 200) on this monitor reports (1500, 100).
        let scaled = dock(monitor(1920, -200, 3840, 2160, 2.0), 1.0);
        assert_eq!(scaled, DockMonitor { name: Some("m".into()), rect: [960.0, -100.0, 1920.0, 1080.0], scale: 2.0 });
        // The UI zoom divides what egui reports as well.
        let zoomed = dock(monitor(-2560, 0, 2560, 1440, 1.0), 1.25);
        assert_eq!(zoomed, DockMonitor { name: Some("m".into()), rect: [-2048.0, 0.0, 2048.0, 1152.0], scale: 1.25 });
    }

    #[test]
    fn monitors_of_different_scales_are_listed_only_on_a_desktop_of_shared_points() {
        let named = |m: Monitor, name: &str| (m, Some(name.to_string()));
        let same = [named(monitor(0, 0, 1920, 1080, 1.5), "a"), named(monitor(1920, 0, 1920, 1080, 1.5), "b")];
        assert_eq!(docking_list(&same, 1.0, false).len(), 2);
        let mixed = [named(monitor(0, 0, 3840, 2160, 2.0), "a"), named(monitor(3840, 0, 1920, 1080, 1.0), "b")];
        assert_eq!(docking_list(&mixed, 1.0, false), Vec::new());
        assert_eq!(docking_list(&mixed, 1.0, true).len(), 2);
        assert_eq!(docking_list(&[], 1.0, false), Vec::new());
    }

    #[test]
    fn a_name_that_two_monitors_share_is_left_out_of_the_list() {
        let list = [
            (monitor(0, 0, 1920, 1080, 1.0), Some("Monitor #1".to_string())),
            (monitor(1920, 0, 1920, 1080, 1.0), Some("Monitor #1".to_string())),
            (monitor(3840, 0, 1920, 1080, 1.0), Some("Monitor #2".to_string())),
            (monitor(5760, 0, 1920, 1080, 1.0), None),
        ];
        let names: Vec<_> = docking_list(&list, 1.0, false).into_iter().map(|m| m.name).collect();
        assert_eq!(names, [None, None, Some("Monitor #2".to_string()), None]);
    }

    #[test]
    fn the_monitor_list_is_read_on_the_first_frame_then_once_a_second_or_on_a_zoom_change() {
        assert!(monitors_due(None, 0.0, 1.0));
        assert!(!monitors_due(Some((5.0, 1.0)), 5.0, 1.0));
        assert!(!monitors_due(Some((5.0, 1.0)), 5.5, 1.0));
        assert!(monitors_due(Some((5.0, 1.0)), 6.0, 1.0));
        assert!(monitors_due(Some((5.0, 1.0)), 5.5, 1.25));
    }

    #[test]
    fn the_saved_window_comes_back_where_it_was() {
        let two = [monitor(0, 0, 2560, 1440, 1.0), monitor(2560, 0, 1920, 1080, 1.0)];
        for g in [saved(Some([100, 80]), [1200.0, 800.0], false), saved(Some([2700, 50]), [1000.0, 700.0], true), saved(None, [1000.0, 700.0], false)]
        {
            assert_eq!(Some(place(g, &two, Some(0))), g);
        }
    }

    #[test]
    fn a_saved_window_stays_on_a_connected_monitor() {
        let laptop = [monitor(0, 0, 1600, 900, 1.0)];
        // Last time on a second monitor that is gone: centred on the remaining one.
        let g = place(saved(Some([2700, 50]), [1000.0, 700.0], false), &laptop, Some(0));
        assert_eq!(g, WindowGeometry { pos: Some([300, 100]), size: [1000.0, 700.0], maximized: false });
        // Half off the monitor's right and bottom edges: moved fully onto it.
        let g = place(saved(Some([1200, 600]), [1000.0, 700.0], false), &laptop, Some(0));
        assert_eq!(g.pos, Some([600, 200]));
        // Bigger than the screen now (it was on a larger one): it fits, and opens maximized.
        let g = place(saved(Some([0, 0]), [2400.0, 1300.0], false), &laptop, Some(0));
        assert_eq!(g, WindowGeometry { pos: Some([0, 0]), size: [1536.0, 772.0], maximized: true });
        // A monitor left of the primary one (negative coordinates) still counts.
        let left = [monitor(0, 0, 1920, 1080, 1.0), monitor(-1920, 0, 1920, 1080, 1.0)];
        assert_eq!(place(saved(Some([-1800, 40]), [1000.0, 700.0], false), &left, Some(0)).pos, Some([-1800, 40]));
        // Nonsense sizes from a damaged preferences file are kept within the screen.
        let g = place(saved(Some([i32::MAX, i32::MIN]), [-5.0, f32::MAX], false), &laptop, Some(0));
        assert_eq!(g, WindowGeometry { pos: Some([400, 64]), size: [800.0, 772.0], maximized: true });
        let g = place(saved(Some([5, 5]), [f32::NEG_INFINITY, f32::INFINITY], false), &laptop, Some(0));
        assert_eq!(g, WindowGeometry { pos: Some([400, 64]), size: [800.0, 772.0], maximized: true });
    }

    #[test]
    fn the_window_geometry_is_tracked_for_the_next_launch() {
        let ctx = egui::Context::default();
        let mut geometry = None;
        let mut frame = |info: egui::ViewportInfo| {
            let mut input = egui::RawInput::default();
            input.viewports.insert(egui::ViewportId::ROOT, info);
            ctx.run_ui(input, |ui| track(ui.ctx(), &mut geometry, None)).textures_delta.clear();
            geometry
        };
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(1000.0, 600.0));
        let normal = egui::ViewportInfo {
            native_pixels_per_point: Some(1.5),
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            maximized: Some(false),
            ..Default::default()
        };
        // Position in physical pixels, size in points.
        let g = WindowGeometry { pos: Some([30, 45]), size: [1000.0, 600.0], maximized: false };
        assert_eq!(frame(normal.clone()), Some(g));
        // Maximized or minimized: the size to restore stays.
        let big = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 700.0));
        let maximized = egui::ViewportInfo { inner_rect: Some(big), outer_rect: Some(big), maximized: Some(true), ..normal.clone() };
        assert_eq!(frame(maximized), Some(WindowGeometry { maximized: true, ..g }));
        let minimized = egui::ViewportInfo { inner_rect: None, outer_rect: None, minimized: Some(true), ..normal.clone() };
        assert_eq!(frame(minimized), Some(WindowGeometry { maximized: true, ..g }));
        assert_eq!(frame(normal), Some(g));
        // Saved with the preferences; older preference files have no window.
        let mut ui = vectorcraft_ui_egui::UiState { window: Some(g), ..Default::default() };
        let back: vectorcraft_ui_egui::UiState = serde_json::from_slice(&serde_json::to_vec(&ui).unwrap()).unwrap();
        assert_eq!(back.window, Some(g));
        ui.window = None;
        let json = serde_json::to_value(&ui).unwrap();
        assert!(json.get("window").is_none());
        let back: vectorcraft_ui_egui::UiState = serde_json::from_value(json).unwrap();
        assert_eq!(back.window, None);
    }

    #[test]
    fn the_host_maximized_flag_is_remembered_when_egui_does_not_report_it() {
        let ctx = egui::Context::default();
        let mut geometry = Some(WindowGeometry { pos: Some([10, 20]), size: [800.0, 600.0], maximized: false });
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0));
        let mut input = egui::RawInput::default();
        input.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo { inner_rect: Some(rect), outer_rect: Some(rect), maximized: None, ..Default::default() },
        );
        let mut out = ctx.run_ui(input, |ui| track(ui.ctx(), &mut geometry, Some(true)));
        out.textures_delta.clear();
        assert_eq!(geometry, Some(WindowGeometry { pos: Some([10, 20]), size: [800.0, 600.0], maximized: true }));
    }
}
