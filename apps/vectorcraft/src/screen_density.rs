//! The density of the display that shows most of the main window, in the window's points per
//! inch, for Preferences › General › Display Print Size at 100% Zoom (`Services::screen_ppi`):
//! the display's size in points divided by the physical size the system reports for it.
//!
//! - macOS: `CGDisplayScreenSize`.
//! - Windows: `GetDpiForMonitor`'s raw density (`windows_display_size`).
//! - X11, XWayland included: the RandR output's width and height in millimeters.
//! - Wayland: the physical width and height of the compositor's `wl_output` for the display.
//!
//! A display that reports no size, or a size no display has, gives `None`, and an inch then takes
//! 96 points. The window's display is looked up once a second while the app draws, and read when
//! it changes.

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::monitor::MonitorHandle;

/// Seconds between two checks of which display shows the window.
const LOOK_EVERY: f64 = 1.0;

/// Millimeters in an inch.
const MM_PER_INCH: f64 = 25.4;

/// How far the horizontal and vertical densities of one display may differ, as a fraction of the
/// larger: square pixels on a display that reports its true size give two densities this close.
const AXES_AGREE: f64 = 0.1;

/// What identifies the display and the window's scale on it between two looks.
type Shown = (Option<String>, PhysicalPosition<i32>, PhysicalSize<u32>, u64);

/// Keeps the density current from the main window's frames.
#[derive(Default)]
pub(crate) struct Tracker {
    /// When the window's display was last looked up.
    looked: Option<f64>,
    /// The display read last, with the window's scale factor then.
    shown: Option<Shown>,
}

impl Tracker {
    /// Looks up the window's display when a look is due at `now` (seconds), and returns its
    /// density (`None` inside for a display that reports no size) when the display or the
    /// window's scale changed since the last reading.
    pub(crate) fn tick(&mut self, window: &winit::window::Window, now: f64) -> Option<Option<f64>> {
        if self.looked.is_some_and(|at| (at..at + LOOK_EVERY).contains(&now)) {
            return None;
        }
        self.looked = Some(now);
        let monitor = window.current_monitor();
        let scale = window.scale_factor();
        let shown = monitor.as_ref().map(|m| (m.name(), m.position(), m.size(), scale.to_bits()));
        if shown == self.shown {
            return None;
        }
        self.shown = shown;
        let read = monitor.and_then(|m| points_per_inch(m.size(), scale, physical_mm(window, &m)?));
        match read {
            Some(p) => log::info!("the window's display has {p:.1} points to an inch"),
            None => log::info!("the window's display reports no physical size; an inch is 96 points"),
        }
        Some(read)
    }
}

/// The density, in points per inch, of a display `size` physical pixels wide and high, with the
/// window drawn at `scale` pixels to a point, that the system says is `mm` millimeters wide and
/// high. `None` for a size that is no display's: zero or less, an aspect ratio given in place of
/// the size (16 × 9, 160 × 90 and the like), axes whose densities disagree in either orientation,
/// or a density outside [`vectorcraft_ui_egui::state::SCREEN_PPI_RANGE`]. A display turned a
/// quarter turn may report the size of its panel upright, so the axes are also tried swapped.
pub(crate) fn points_per_inch(size: PhysicalSize<u32>, scale: f64, mm: [f64; 2]) -> Option<f64> {
    if !(scale.is_finite() && scale > 0.0) || mm.iter().any(|v| !(v.is_finite() && *v > 0.0)) || aspect_as_size(mm) {
        return None;
    }
    let points = [f64::from(size.width) / scale, f64::from(size.height) / scale];
    let density = |[w, h]: [f64; 2]| [points[0] / (w / MM_PER_INCH), points[1] / (h / MM_PER_INCH)];
    let agree = |[x, y]: [f64; 2]| (x - y).abs() <= AXES_AGREE * x.max(y);
    let [x, y] = [density(mm), density([mm[1], mm[0]])].into_iter().find(|d| agree(*d))?;
    let ppi = (x + y) / 2.0;
    vectorcraft_ui_egui::state::SCREEN_PPI_RANGE.contains(&ppi).then_some(ppi)
}

/// Whether `mm` is an aspect ratio that a display's EDID gave in place of its size: 16:9, 16:10
/// or 4:3, in centimeters or tenths of them.
fn aspect_as_size(mm: [f64; 2]) -> bool {
    const RATIOS: [[f64; 2]; 3] = [[16.0, 9.0], [16.0, 10.0], [4.0, 3.0]];
    let sorted = if mm[0] >= mm[1] { mm } else { [mm[1], mm[0]] };
    RATIOS.iter().any(|r| [1.0, 10.0].iter().any(|k| sorted == [r[0] * k, r[1] * k]))
}

/// The physical width and height, in millimeters, the system reports for `monitor`.
#[cfg(target_os = "macos")]
fn physical_mm(_: &winit::window::Window, monitor: &MonitorHandle) -> Option<[f64; 2]> {
    use winit::platform::macos::MonitorHandleExtMacOS as _;
    let size = objc2_core_graphics::CGDisplayScreenSize(monitor.native_id());
    Some([size.width, size.height])
}

#[cfg(windows)]
fn physical_mm(_: &winit::window::Window, monitor: &MonitorHandle) -> Option<[f64; 2]> {
    use winit::platform::windows::MonitorHandleExtWindows as _;
    let [x, y] = crate::windows_display_size::raw_dpi(monitor.hmonitor())?;
    let size = monitor.size();
    Some([f64::from(size.width) / x * MM_PER_INCH, f64::from(size.height) / y * MM_PER_INCH])
}

#[cfg(target_os = "linux")]
fn physical_mm(window: &winit::window::Window, monitor: &MonitorHandle) -> Option<[f64; 2]> {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let wayland = window.window_handle().ok().is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)));
    let read = if wayland { wayland::physical_mm(monitor.name().as_deref()) } else { x11::physical_mm(monitor) };
    read.unwrap_or_else(|e| {
        log::debug!("couldn't read the display's physical size: {e}");
        None
    })
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn physical_mm(_: &winit::window::Window, _: &MonitorHandle) -> Option<[f64; 2]> {
    None
}

#[cfg(target_os = "linux")]
mod x11 {
    use winit::monitor::MonitorHandle;
    use winit::platform::x11::MonitorHandleExtX11 as _;
    use x11rb::CURRENT_TIME;
    use x11rb::protocol::randr;

    /// The millimeters RandR reports for the first output of the CRTC that shows `monitor`.
    pub(super) fn physical_mm(monitor: &MonitorHandle) -> Result<Option<[f64; 2]>, String> {
        let (conn, _) = x11rb::connect(None).map_err(|e| format!("couldn't connect to the X11 display: {e}"))?;
        let ask = |e: x11rb::errors::ConnectionError| format!("couldn't ask the X11 display: {e}");
        let crtc = randr::get_crtc_info(&conn, monitor.native_id(), CURRENT_TIME)
            .map_err(ask)?
            .reply()
            .map_err(|e| format!("the X11 display has no such CRTC: {e}"))?;
        let Some(output) = crtc.outputs.first() else { return Ok(None) };
        let info = randr::get_output_info(&conn, *output, CURRENT_TIME)
            .map_err(ask)?
            .reply()
            .map_err(|e| format!("the X11 display has no such output: {e}"))?;
        Ok(Some([f64::from(info.mm_width), f64::from(info.mm_height)]))
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_output::{self, WlOutput};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::{Connection, Dispatch, QueueHandle};

    /// The physical size the compositor gives for the output named `name`, asked on a connection
    /// of its own; with no name, or a compositor that names no outputs, the only output's.
    pub(super) fn physical_mm(name: Option<&str>) -> Result<Option<[f64; 2]>, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland compositor: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<Outputs>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let mut state = Outputs::default();
        for global in globals.contents().clone_list().into_iter().filter(|g| g.interface == "wl_output") {
            // Version 4 names the output; older ones give its size only.
            let output: WlOutput = globals.registry().bind(global.name, global.version.min(4), &qh, state.outputs.len());
            state.outputs.push((output, Output::default()));
        }
        // The compositor sends each output's geometry, name and `done` in reply to the bind.
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        let found: Vec<&Output> = state.outputs.iter().map(|(_, o)| o).collect();
        let pick = match name {
            Some(name) if found.iter().any(|o| o.name.is_some()) => found.iter().find(|o| o.name.as_deref() == Some(name)),
            _ if found.len() == 1 => found.first(),
            _ => None,
        };
        Ok(pick.and_then(|o| o.mm))
    }

    /// One output's name and physical size.
    #[derive(Default)]
    struct Output {
        name: Option<String>,
        mm: Option<[f64; 2]>,
    }

    /// The outputs bound, each with what its events gave.
    #[derive(Default)]
    struct Outputs {
        outputs: Vec<(WlOutput, Output)>,
    }

    impl Dispatch<WlRegistry, GlobalListContents> for Outputs {
        fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlOutput, usize> for Outputs {
        fn event(state: &mut Self, _: &WlOutput, event: wl_output::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
            let Some((_, output)) = state.outputs.get_mut(*index) else { return };
            match event {
                wl_output::Event::Geometry { physical_width, physical_height, .. } => {
                    output.mm = Some([f64::from(physical_width), f64::from(physical_height)]);
                }
                wl_output::Event::Name { name } => output.name = Some(name),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 14-inch laptop panel at twice its points: 3024 × 1964 pixels, 302 × 196 mm, draws 127
    /// points to an inch, more than the 96 a fixed inch assumes.
    #[test]
    fn a_laptop_panel_has_more_points_to_an_inch_than_the_reference() {
        let ppi = points_per_inch(PhysicalSize::new(3024, 1964), 2.0, [302.0, 196.0]);
        assert!(ppi.is_some_and(|p| p > 96.0 && (p - 1512.0 / (302.0 / MM_PER_INCH)).abs() < 1.0), "{ppi:?}");
    }

    /// The same display in a scaled mode (more points on the same glass) draws more points to an
    /// inch, and at a higher scale factor fewer: the density counts the window's points.
    #[test]
    fn the_density_counts_points_at_the_windows_scale() {
        let at = |w, h, scale| points_per_inch(PhysicalSize::new(w, h), scale, [527.0, 296.0]).unwrap_or(0.0);
        assert!(at(1920, 1080, 1.0) > 0.0);
        assert!(at(2560, 1440, 1.0) > at(1920, 1080, 1.0), "more pixels on the same glass");
        assert!(at(3840, 2160, 2.0) < at(3840, 2160, 1.5), "a larger scale factor makes each point larger");
    }

    /// A display turned a quarter turn whose system reports its panel's size upright still
    /// measures; axes that disagree either way do not.
    #[test]
    fn a_turned_display_measures_and_disagreeing_axes_do_not() {
        let upright = points_per_inch(PhysicalSize::new(1920, 1080), 1.0, [527.0, 296.0]);
        let turned = points_per_inch(PhysicalSize::new(1080, 1920), 1.0, [527.0, 296.0]);
        assert!(upright.is_some() && turned.is_some());
        assert!((upright.unwrap_or(0.0) - turned.unwrap_or(1.0)).abs() < 1e-9);
        assert_eq!(points_per_inch(PhysicalSize::new(1920, 1080), 1.0, [527.0, 527.0]), None);
    }

    /// Sizes no display has give no density: none, an aspect ratio in place of the size, and
    /// densities past the range a display can have.
    #[test]
    fn reports_that_are_no_displays_size_give_no_density() {
        let at = |mm| points_per_inch(PhysicalSize::new(1920, 1080), 1.0, mm);
        for mm in [[0.0, 0.0], [-1.0, 300.0], [f64::NAN, 300.0], [160.0, 90.0], [16.0, 9.0], [160.0, 100.0], [40.0, 30.0], [9.0, 16.0]] {
            assert_eq!(at(mm), None, "{mm:?}");
        }
        assert_eq!(points_per_inch(PhysicalSize::new(1920, 1080), 0.0, [527.0, 296.0]), None, "no scale");
        assert_eq!(points_per_inch(PhysicalSize::new(100, 56), 1.0, [5270.0, 2960.0]), None, "under the range");
        assert_eq!(points_per_inch(PhysicalSize::new(7680, 4320), 1.0, [53.0, 30.0]), None, "over it");
    }
}
