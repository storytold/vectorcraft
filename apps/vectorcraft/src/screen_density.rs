//! The densities of the connected displays, in points (at each display's scale factor) per inch,
//! for Preferences › General › Display Print Size at 100% Zoom (`Services::screen_ppi` for the
//! display that shows most of the main window, `Services::screens` for every display): a
//! display's size in points divided by the physical size the system reports for it.
//!
//! - macOS: `CGDisplayScreenSize`.
//! - Windows: `GetDpiForMonitor`'s raw density (`windows_display_size`).
//! - X11 on Linux, XWayland included: the RandR output's width and height in millimeters.
//! - Wayland on Linux: the physical width and height of the compositor's `wl_output` for the display.
//! - Other systems: no size, so an inch takes 96 points.
//!
//! A display that reports no size, or a size no display has, gives `None`, and an inch then takes
//! 96 points. While the preference is on, the displays are listed once a second while the app
//! draws. When the list changes, a background thread reads every display's size, and a later
//! frame takes the result. A reading that fails is tried again after [`RETRY_AFTER`] seconds, and
//! the displays are read again [`READ_AGAIN_AFTER`] seconds after the last reading; the app draws a
//! frame for each of these when nothing else makes it draw.

use std::sync::mpsc::{Receiver, TryRecvError};

use vectorcraft_ui_egui::{ScreenDensity, ScreenPpi};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::monitor::MonitorHandle;

/// Seconds between two listings of the displays.
const LOOK_EVERY: f64 = 1.0;

/// Seconds after a failed reading before the same displays are read again.
const RETRY_AFTER: f64 = 10.0;

/// Seconds after a reading before the same displays are read again: a display can be replaced by
/// one the listing cannot tell apart (the same connector, resolution and scale).
const READ_AGAIN_AFTER: f64 = 60.0;

/// Millimeters in an inch.
const MM_PER_INCH: f64 = 25.4;

/// How far the horizontal and vertical densities of one display may differ, as a fraction of the
/// larger: square pixels on a display that reports its true size give two densities this close.
const AXES_AGREE: f64 = 0.1;

/// A connected display as winit lists it: what a reading needs, and what tells two lists apart.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Display {
    name: Option<String>,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    /// The display's scale factor, as its bits.
    scale: u64,
    /// The system's id of the display ([`native_id`]).
    id: i64,
}

impl Display {
    fn of(m: &MonitorHandle) -> Self {
        Self { name: m.name(), position: m.position(), size: m.size(), scale: m.scale_factor().to_bits(), id: native_id(m) }
    }
}

/// One display's physical size in millimeters: `None` when the system reports none, an error when
/// asking for it failed.
type Size = Result<Option<[f64; 2]>, String>;

/// The physical sizes of displays, one for each display read, or why none could be read.
type Reading = Result<Vec<Size>, String>;

/// What the host hands the app: the density of the main window's display, unread while that
/// display is unknown or has not answered yet, and every display's that answered.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Densities {
    pub(crate) main: ScreenPpi,
    pub(crate) screens: Vec<ScreenDensity>,
}

/// What a display answered: its physical size (`Some(None)` for none), or `None` while it has not
/// answered.
type Answer = Option<Option<[f64; 2]>>;

/// What a listing gives: the displays, the one that shows most of the main window, and the main
/// window's scale factor.
type Listing = (Vec<Display>, Option<Display>, f64);

/// Keeps the densities current from the main window's frames.
#[derive(Default)]
pub(crate) struct Tracker {
    /// When the displays were last listed.
    looked: Option<f64>,
    /// The display that showed most of the main window at the last listing, and the window's scale
    /// factor then.
    current: Option<(Display, f64)>,
    /// The displays of the last reading that finished, each with what it answered.
    read: Option<Vec<(Display, Answer)>>,
    /// The displays of the last reading that failed, and when it failed.
    failed: Option<(Vec<Display>, f64)>,
    /// The reading in progress: the displays it reads, and where its result arrives.
    running: Option<(Vec<Display>, Receiver<Reading>)>,
    /// The reading in progress started before the preference was last turned off: its result is
    /// dropped when it arrives, and no other reading starts until then.
    obsolete: bool,
    /// What [`Self::tick`] returned last.
    reported: Option<Densities>,
    /// The displays of the last listing.
    listed: Option<Vec<Display>>,
    /// When the last reading that finished ended.
    read_at: f64,
    /// When the first tick ran: the tracker's clock, which runs while the window draws nothing.
    origin: Option<std::time::Instant>,
}

impl Tracker {
    /// Lists the displays when a listing is due and Display Print Size is `enabled`, starts a
    /// reading on a background thread when they changed, and returns the densities when they
    /// differ from what it returned before. The thread asks `ctx` for a frame when its reading is
    /// ready.
    pub(crate) fn tick(&mut self, window: &winit::window::Window, enabled: bool, ctx: &egui::Context) -> Option<Densities> {
        let now = self.origin.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f64();
        let list = || {
            // winit stops on a display that went away while it is being asked about (Windows); that
            // listing is dropped, and the next one is made a second later.
            let listing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let current = window.current_monitor().map(|m| Display::of(&m));
                (window.available_monitors().map(|m| Display::of(&m)).collect::<Vec<_>>(), current, window.scale_factor())
            }));
            listing.map_err(|_| log::debug!("a display went away while the displays were listed")).ok()
        };
        let wayland = is_wayland(window);
        let start = |displays: Vec<Display>| {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("display size".into()).spawn(move || {
                // The app may have quit, and dropped the receiver, before the reading ends.
                let _ = tx.send(read_all(&displays, wayland));
                ctx.request_repaint();
            });
            spawned.map(|_| rx).map_err(|e| format!("couldn't start reading the displays' sizes: {e}"))
        };
        let densities = self.update(now, enabled, list, start);
        // A failed reading is tried again at a listing RETRY_AFTER seconds later, also in an app
        // that is not drawing. A reading in progress asks for a frame when it ends.
        if enabled
            && self.running.is_none()
            && let Some((_, at)) = &self.failed
        {
            let due = (at + RETRY_AFTER).max(self.looked.map_or(0.0, |l| l + LOOK_EVERY));
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).clamp(0.0, RETRY_AFTER)));
        } else if enabled && self.running.is_none() && self.read.is_some() {
            // An app that is not drawing still lists and reads the displays READ_AGAIN_AFTER seconds
            // after the last reading.
            let due = (self.read_at + READ_AGAIN_AFTER).max(self.looked.map_or(0.0, |l| l + LOOK_EVERY));
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).clamp(0.0, READ_AGAIN_AFTER)));
        }
        densities
    }

    /// [`Self::tick`] with the listing (`list`) and the start of a reading (`start`) passed in.
    fn update(
        &mut self,
        now: f64,
        enabled: bool,
        list: impl FnOnce() -> Option<Listing>,
        start: impl FnOnce(Vec<Display>) -> Result<Receiver<Reading>, String>,
    ) -> Option<Densities> {
        let was_running = self.running.is_some();
        if let Some((displays, rx)) = self.running.take() {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => self.running = Some((displays, rx)),
                _ if self.obsolete => self.obsolete = false,
                Ok(Ok(sizes)) => {
                    // A display whose size could not be read keeps what an earlier reading gave it,
                    // or stays unanswered, until the reading is tried again.
                    let mut failed = false;
                    let read: Vec<_> = displays
                        .iter()
                        .zip(sizes.into_iter().chain(std::iter::repeat(Ok(None))))
                        .map(|(d, s)| {
                            let mm = s.map(Some).unwrap_or_else(|e| {
                                log::debug!("couldn't read a display's physical size: {e}");
                                failed = true;
                                self.earlier(d)
                            });
                            (d.clone(), mm)
                        })
                        .collect();
                    self.failed = failed.then_some((displays, now));
                    self.read = Some(read);
                    self.read_at = now;
                }
                Ok(Err(e)) => {
                    log::debug!("couldn't read the displays' physical sizes: {e}");
                    self.fail(displays, now);
                }
                Err(TryRecvError::Disconnected) => {
                    log::debug!("the reading of the displays' physical sizes stopped without a result");
                    self.fail(displays, now);
                }
            }
        }
        // When a reading ends, the displays are listed again at once: they may have changed while
        // it ran, and the frame its end asked for may be the last one for a while.
        if was_running && self.running.is_none() {
            self.looked = None;
        }
        if !enabled {
            // Turned on again, the displays are read and the densities reported anew: a display
            // may have been replaced meanwhile by one that lists the same.
            self.reported = None;
            self.read = None;
            self.failed = None;
            self.looked = None;
            self.obsolete = self.running.is_some();
            return None;
        }
        // The densities change only at a listing, and one follows the end of every reading.
        let listing = !self.looked.is_some_and(|at| (at..at + LOOK_EVERY).contains(&now));
        if !listing && self.reported.is_some() {
            return None;
        }
        if listing {
            self.looked = Some(now);
        }
        if let Some((displays, current, scale)) = listing.then(list).flatten() {
            // winit can give the window's display as it was before a change of resolution: the
            // display is found in the listing by its id and name.
            let current = current.map(|c| displays.iter().find(|d| d.id == c.id && d.name == c.name).cloned().unwrap_or(c));
            self.current = current.map(|c| (c, scale));
            self.listed = Some(displays.clone());
            // These displays are read once, and again RETRY_AFTER seconds after a reading of them
            // failed in whole or in part.
            let known = self.read.as_ref().is_some_and(|r| r.iter().map(|(d, _)| d).eq(displays.iter()));
            let due = match &self.failed {
                Some((d, at)) if *d == displays => !(*at..*at + RETRY_AFTER).contains(&now),
                _ => !known || now - self.read_at >= READ_AGAIN_AFTER,
            };
            if self.running.is_none() && due {
                match start(displays.clone()) {
                    Ok(rx) => self.running = Some((displays, rx)),
                    Err(e) => {
                        log::warn!("{e}");
                        self.fail(displays, now);
                    }
                }
            }
        }
        let read = self.read.as_ref()?;
        // The main window counts in its own scale factor, which on Wayland can be a fraction where
        // the display gives a whole number. A window on no display (a Wayland window before it is
        // shown, or after its display went), or on one that is being read or has not answered, has
        // its density unread.
        let main = match &self.current {
            Some((c, scale)) => match read.iter().find(|(d, _)| d == c) {
                Some((d, Some(mm))) => ScreenPpi::Read(mm.and_then(|mm| points_per_inch(d.size, *scale, mm))),
                _ => ScreenPpi::Unread,
            },
            None => ScreenPpi::Unread,
        };
        // The displays where the last listing has them, each with what the reading of it gave; a
        // display the readings have not reached yet is unread.
        let listed = self.listed.as_deref().unwrap_or_default();
        let screens = listed
            .iter()
            .map(|d| {
                let [w, h] = [d.size.width, d.size.height].map(|v| i32::try_from(v).unwrap_or(i32::MAX));
                let scale = f64::from_bits(d.scale);
                let mm = read.iter().find(|(e, _)| e == d).and_then(|(_, mm)| *mm);
                let ppi = mm.map_or(ScreenPpi::Unread, |mm| ScreenPpi::Read(mm.and_then(|mm| points_per_inch(d.size, scale, mm))));
                ScreenDensity { rect: [d.position.x, d.position.y, w, h], scale, ppi }
            })
            .collect();
        let densities = Densities { main, screens };
        if self.reported.as_ref() == Some(&densities) {
            return None;
        }
        match main {
            ScreenPpi::Read(Some(p)) => log::info!("the window's display has {p:.1} points to an inch"),
            ScreenPpi::Read(None) => log::info!("the window's display reports no physical size; an inch is 96 points"),
            ScreenPpi::Unread => log::info!("the window's display has not been read; an inch is 96 points"),
        }
        self.reported = Some(densities.clone());
        Some(densities)
    }
}

impl Tracker {
    /// What the last reading gave display `d`, when it answered.
    fn earlier(&self, d: &Display) -> Answer {
        self.read.as_ref()?.iter().find(|(e, _)| e == d)?.1
    }

    /// Records that reading `displays` failed at `now`; they are read again after [`RETRY_AFTER`]
    /// seconds. After an earlier reading, they become the displays known, each with what that
    /// reading gave it, if anything. Before any reading, nothing is reported yet.
    fn fail(&mut self, displays: Vec<Display>, now: f64) {
        if self.read.is_some() {
            let read = displays.iter().map(|d| (d.clone(), self.earlier(d))).collect();
            self.read = Some(read);
        }
        self.failed = Some((displays, now));
    }
}

/// The density, in points per inch, of a display `size` physical pixels wide and high, drawn at
/// `scale` pixels to a point, that the system says is `mm` millimeters wide and high. `None` for a
/// size that is no display's: zero or less, an aspect ratio given in place of the size (16 × 9,
/// 160 × 90 and the like), axes whose densities disagree in either orientation, or a pixel density
/// outside [`vectorcraft_ui_egui::state::SCREEN_PPI_RANGE`]. The range applies to the pixels, before
/// the scale divides them into points. A display turned a quarter turn may report the size of its
/// panel upright, so the axes are also tried swapped.
pub(crate) fn points_per_inch(size: PhysicalSize<u32>, scale: f64, mm: [f64; 2]) -> Option<f64> {
    if !(scale.is_finite() && scale > 0.0) || mm.iter().any(|v| !(v.is_finite() && *v > 0.0)) || aspect_as_size(mm) {
        return None;
    }
    let pixels = [f64::from(size.width), f64::from(size.height)];
    let density = |[w, h]: [f64; 2]| [pixels[0] / (w / MM_PER_INCH), pixels[1] / (h / MM_PER_INCH)];
    let agree = |[x, y]: [f64; 2]| (x - y).abs() <= AXES_AGREE * x.max(y);
    let [x, y] = [density(mm), density([mm[1], mm[0]])].into_iter().find(|d| agree(*d))?;
    let ppi = (x + y) / 2.0;
    vectorcraft_ui_egui::state::SCREEN_PPI_RANGE.contains(&ppi).then_some(ppi / scale)
}

/// Whether `mm` is an aspect ratio that a display's EDID gave in place of its size: 16:9, 16:10
/// or 4:3, in centimeters or tenths of them.
fn aspect_as_size(mm: [f64; 2]) -> bool {
    const RATIOS: [[f64; 2]; 3] = [[16.0, 9.0], [16.0, 10.0], [4.0, 3.0]];
    let sorted = if mm[0] >= mm[1] { mm } else { [mm[1], mm[0]] };
    RATIOS.iter().any(|r| [1.0, 10.0].iter().any(|k| sorted == [r[0] * k, r[1] * k]))
}

/// Whether `window` is a Wayland window, whose displays the compositor describes.
#[cfg(target_os = "linux")]
fn is_wayland(window: &winit::window::Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    window.window_handle().ok().is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
}

#[cfg(not(target_os = "linux"))]
fn is_wayland(_: &winit::window::Window) -> bool {
    false
}

/// The system's id of `monitor`: the display id on macOS, the monitor handle on Windows, the RandR
/// CRTC on X11.
#[cfg(target_os = "macos")]
fn native_id(monitor: &MonitorHandle) -> i64 {
    use winit::platform::macos::MonitorHandleExtMacOS as _;
    i64::from(monitor.native_id())
}

#[cfg(windows)]
fn native_id(monitor: &MonitorHandle) -> i64 {
    use winit::platform::windows::MonitorHandleExtWindows as _;
    monitor.hmonitor() as i64
}

#[cfg(target_os = "linux")]
fn native_id(monitor: &MonitorHandle) -> i64 {
    use winit::platform::x11::MonitorHandleExtX11 as _;
    i64::from(monitor.native_id())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn native_id(_: &MonitorHandle) -> i64 {
    0
}

/// The physical width and height, in millimeters, the system reports for each of `displays`.
#[cfg(target_os = "macos")]
fn read_all(displays: &[Display], _: bool) -> Reading {
    Ok(displays
        .iter()
        .map(|d| {
            Ok(u32::try_from(d.id).ok().map(|id| {
                let size = objc2_core_graphics::CGDisplayScreenSize(id);
                [size.width, size.height]
            }))
        })
        .collect())
}

#[cfg(windows)]
fn read_all(displays: &[Display], _: bool) -> Reading {
    Ok(displays
        .iter()
        .map(|d| {
            let dpi = crate::windows_display_size::raw_dpi(d.id as isize)?;
            Ok(dpi.map(|[x, y]| [f64::from(d.size.width) / x * MM_PER_INCH, f64::from(d.size.height) / y * MM_PER_INCH]))
        })
        .collect())
}

#[cfg(target_os = "linux")]
fn read_all(displays: &[Display], wayland: bool) -> Reading {
    if wayland { wayland::read_all(displays) } else { x11::read_all(displays) }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn read_all(displays: &[Display], _: bool) -> Reading {
    Ok(displays.iter().map(|_| Ok(None)).collect())
}

#[cfg(target_os = "linux")]
mod x11 {
    use x11rb::CURRENT_TIME;
    use x11rb::protocol::randr;

    use super::{Display, Reading, Size};

    /// The millimeters RandR reports for the first output of each display's CRTC.
    pub(super) fn read_all(displays: &[Display]) -> Reading {
        let (conn, _) = x11rb::connect(None).map_err(|e| format!("couldn't connect to the X11 display: {e}"))?;
        let ask = |e: x11rb::errors::ConnectionError| format!("couldn't ask the X11 display: {e}");
        // RandR answers only a client that gave the version it speaks; 1.2 has CRTCs and outputs.
        randr::query_version(&conn, 1, 2).map_err(ask)?.reply().map_err(|e| format!("the X11 display has no RandR 1.2: {e}"))?;
        let size = |d: &Display| -> Size {
            let Ok(crtc) = u32::try_from(d.id) else { return Ok(None) };
            let info = randr::get_crtc_info(&conn, crtc, CURRENT_TIME)
                .map_err(ask)?
                .reply()
                .map_err(|e| format!("the X11 display has no such CRTC: {e}"))?;
            let Some(output) = info.outputs.first() else { return Ok(None) };
            let info = randr::get_output_info(&conn, *output, CURRENT_TIME)
                .map_err(ask)?
                .reply()
                .map_err(|e| format!("the X11 display has no such output: {e}"))?;
            Ok(Some([f64::from(info.mm_width), f64::from(info.mm_height)]))
        };
        Ok(displays.iter().map(size).collect())
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_output::{self, WlOutput};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::{Connection, Dispatch, QueueHandle};

    use super::{Display, Reading};

    /// The physical size the compositor gives for the output of each display's name, asked on a
    /// connection of its own; with no name, or a compositor that names no outputs, the only
    /// output's.
    pub(super) fn read_all(displays: &[Display]) -> Reading {
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
        let named = found.iter().any(|o| o.name.is_some());
        Ok(displays
            .iter()
            .map(|d| {
                let pick = match d.name.as_deref() {
                    Some(name) if named => found.iter().find(|o| o.name.as_deref() == Some(name)),
                    _ if found.len() == 1 => found.first(),
                    _ => None,
                };
                Ok(pick.and_then(|o| o.mm))
            })
            .collect())
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
    use std::cell::Cell;
    use std::sync::mpsc::{Sender, channel};

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

    /// The range applies to the display's pixels: a 1920 × 1080 display of 527 × 296 mm drawn at
    /// scale 4 has about 23 points to an inch, which is a real density.
    #[test]
    fn a_large_scale_factor_does_not_put_a_display_out_of_range() {
        let ppi = points_per_inch(PhysicalSize::new(1920, 1080), 4.0, [527.0, 296.0]);
        let pixels = points_per_inch(PhysicalSize::new(1920, 1080), 1.0, [527.0, 296.0]).unwrap_or(0.0);
        assert!(ppi.is_some_and(|p| p < 30.0 && (p - pixels / 4.0).abs() < 1e-9), "{ppi:?}");
        let print = vectorcraft_ui_egui::state::print_ppi(true, ppi);
        assert_eq!(print, ppi, "Actual Size counts from it, not from 96");
    }

    /// A 1920 × 1080 display at scale 1 and position `x`, named `name`.
    fn display(name: &str, x: i32) -> Display {
        Display {
            name: Some(name.into()),
            position: PhysicalPosition::new(x, 0),
            size: PhysicalSize::new(1920, 1080),
            scale: 1.0f64.to_bits(),
            id: 0,
        }
    }

    /// The sizes of a 24-inch display (about 92 points to an inch) and a 27-inch one (about 82).
    const SIZES: [[f64; 2]; 2] = [[531.0, 299.0], [598.0, 336.0]];

    /// A reading that gives every display one of [`SIZES`], in turn.
    fn sizes(n: usize) -> Reading {
        Ok((0..n).map(|i| Ok(Some(SIZES[i % SIZES.len()]))).collect())
    }

    /// Drives a tracker: counts its listings and readings, and hands out the sender of the reading
    /// it started last.
    struct Rig {
        tracker: Tracker,
        displays: Vec<Display>,
        current: usize,
        /// The window's display as winit last gave it, when that lags behind the listing.
        lagging: Option<Display>,
        window_scale: f64,
        lists: Cell<usize>,
        reads: Cell<usize>,
        sender: Option<Sender<Reading>>,
        fail_to_start: bool,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                tracker: Tracker::default(),
                displays: vec![display("left", 0), display("right", 1920)],
                current: 0,
                lagging: None,
                window_scale: 1.0,
                lists: Cell::new(0),
                reads: Cell::new(0),
                sender: None,
                fail_to_start: false,
            }
        }

        fn tick(&mut self, now: f64, enabled: bool) -> Option<Densities> {
            let current = self.lagging.clone().or_else(|| self.displays.get(self.current).cloned());
            let (displays, scale) = (self.displays.clone(), self.window_scale);
            let (lists, reads, fail) = (&self.lists, &self.reads, self.fail_to_start);
            let mut sender = None;
            let out = self.tracker.update(
                now,
                enabled,
                || {
                    lists.set(lists.get() + 1);
                    Some((displays, current, scale))
                },
                |_| {
                    reads.set(reads.get() + 1);
                    if fail {
                        return Err("no thread".into());
                    }
                    let (tx, rx) = channel();
                    sender = Some(tx);
                    Ok(rx)
                },
            );
            if sender.is_some() {
                self.sender = sender;
            }
            out
        }

        fn finish(&mut self, reading: Reading) {
            if let Some(tx) = self.sender.take() {
                tx.send(reading).unwrap();
            }
        }
    }

    /// The first listing starts a reading, and the densities arrive once the reading ends, without
    /// a frame waiting for it. The main window's density is its display's, and every display gets
    /// its own.
    #[test]
    fn the_densities_arrive_when_the_reading_ends() {
        let mut rig = Rig::new();
        assert_eq!(rig.tick(0.0, true), None, "the reading runs in the background");
        assert_eq!(rig.reads.get(), 1);
        assert_eq!(rig.tick(2.0, true), None, "still running: no second reading");
        assert_eq!(rig.reads.get(), 1);
        rig.finish(sizes(2));
        let got = rig.tick(2.1, true).unwrap_or_else(|| panic!("the densities"));
        let ppi = got.screens.iter().map(|s| s.ppi.ppi().unwrap_or(0.0)).collect::<Vec<_>>();
        assert!(ppi[0] > ppi[1] && ppi[1] > 0.0, "the smaller display is denser: {ppi:?}");
        assert_eq!(got.main, ScreenPpi::Read(Some(ppi[0])), "the main window is on the left display");
        assert_eq!(got.screens[1].rect, [1920, 0, 1920, 1080]);
        assert_eq!(rig.tick(5.0, true), None, "nothing changed");
        assert_eq!(rig.reads.get(), 1, "the same displays are not read again");
        assert_eq!(rig.tick(5.2, false), None, "off, nothing is reported");
        assert_eq!(rig.tick(5.4, true), None, "turned on again, the displays are read again at once");
        assert_eq!(rig.reads.get(), 2);
        rig.finish(sizes(2));
        assert_eq!(rig.tick(5.5, true), Some(got), "and the densities reported anew");
    }

    /// The displays are listed at most once a second, and not at all with the preference off.
    #[test]
    fn the_displays_are_listed_once_a_second_while_the_preference_is_on() {
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.tick(0.5, true);
        assert_eq!(rig.lists.get(), 1);
        rig.tick(1.0, true);
        assert_eq!(rig.lists.get(), 2);
        rig.tick(5.0, false);
        assert_eq!(rig.lists.get(), 2, "off, nothing is asked of the system");
    }

    /// The main window moving to the other display reports that display's density without a new
    /// reading; a display joining the list is read again, and while a display that changed is
    /// read, the densities reported before stay.
    #[test]
    fn a_change_of_display_is_reported() {
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.finish(sizes(2));
        let left = rig.tick(0.1, true).and_then(|d| d.main.ppi());
        rig.current = 1;
        let right = rig.tick(1.5, true).and_then(|d| d.main.ppi());
        assert!(left.is_some() && right.is_some() && left != right, "{left:?} then {right:?}");
        assert_eq!(rig.reads.get(), 1);
        // A window drawn at 1.5 pixels a point (a fractional scale the display does not list)
        // has fewer points to an inch than the display's own scale of 1 gives.
        rig.window_scale = 1.5;
        let scaled = rig.tick(2.6, true).and_then(|d| d.main.ppi());
        assert!(scaled.zip(right).is_some_and(|(s, r)| (s * 1.5 - r).abs() < 1e-9), "{scaled:?} at 1.5, {right:?} at 1");
        rig.window_scale = 1.0;
        rig.displays.push(display("third", 3840));
        rig.tick(4.0, true);
        assert_eq!(rig.reads.get(), 2, "a new display is read");
        rig.finish(sizes(3));
        rig.tick(4.5, true);
        // The window's display changes resolution: until it is read again its density is unread.
        rig.displays[1].size = PhysicalSize::new(2560, 1440);
        assert_eq!(rig.tick(6.0, true).map(|d| d.main), Some(ScreenPpi::Unread), "the display is being read");
        assert_eq!(rig.reads.get(), 3);
        rig.finish(sizes(3));
        let sharper = rig.tick(6.1, true).and_then(|d| d.main.ppi());
        assert!(sharper.zip(right).is_some_and(|(s, r)| s > r), "more pixels on the same glass: {sharper:?}, {right:?}");
        // winit still gives the window's display at its old resolution: it is found by its name.
        let mut old = rig.displays[1].clone();
        old.size = PhysicalSize::new(1920, 1080);
        rig.lagging = Some(old);
        rig.window_scale = 1.25;
        let lagging = rig.tick(7.2, true).and_then(|d| d.main.ppi());
        assert!(lagging.zip(sharper).is_some_and(|(l, s)| (l * 1.25 - s).abs() < 1e-9), "{lagging:?} from {sharper:?}");
        rig.lagging = None;
        // It changes again and this time the reading fails: its density is unread.
        rig.displays[1].size = PhysicalSize::new(3840, 2160);
        let reading = rig.tick(8.5, true).unwrap_or_else(|| panic!("a report"));
        assert_eq!(reading.main, ScreenPpi::Unread, "the display is being read: {reading:?}");
        assert!(reading.screens.iter().any(|s| s.rect[2] == 3840 && s.ppi == ScreenPpi::Unread), "listed where it is now, unread: {reading:?}");
        rig.finish(Err("the display server went away".into()));
        assert_eq!(rig.tick(8.6, true), None, "the failed reading leaves it unread");
    }

    /// A display that changes while a reading runs is read as soon as that reading ends.
    #[test]
    fn a_change_during_a_reading_is_read_when_it_ends() {
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.displays[1].size = PhysicalSize::new(2560, 1440);
        rig.tick(1.0, true);
        assert_eq!(rig.reads.get(), 1, "one reading at a time");
        rig.finish(sizes(2));
        rig.tick(1.2, true);
        assert_eq!(rig.reads.get(), 2, "the changed display is read at once");
    }

    /// A display whose size could not be read does not hold back the others, and the displays are
    /// read again after a while.
    #[test]
    fn one_display_that_fails_leaves_the_others() {
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.finish(Ok(vec![Ok(Some(SIZES[0])), Err("no answer".into())]));
        let got = rig.tick(0.1, true).unwrap_or_else(|| panic!("the densities"));
        assert!(got.main.ppi().is_some() && got.screens[1].ppi == ScreenPpi::Unread, "the display that did not answer is unread: {got:?}");
        rig.tick(5.0, true);
        assert_eq!(rig.reads.get(), 1, "not again at once");
        rig.tick(0.1 + RETRY_AFTER, true);
        assert_eq!(rig.reads.get(), 2, "tried again");
        rig.finish(sizes(2));
        let full = rig.tick(11.0, true).unwrap_or_else(|| panic!("the densities"));
        assert!(full.screens.iter().all(|s| s.ppi.ppi().is_some()), "{full:?}");
        rig.tick(30.0, true);
        assert_eq!(rig.reads.get(), 2, "and left alone once every display was read");
        // A display joins, and the window's display, unchanged, fails to answer: it keeps its size.
        rig.displays.push(display("third", 3840));
        rig.tick(32.0, true);
        rig.finish(Ok(vec![Err("no answer".into()), Ok(Some(SIZES[1])), Ok(Some(SIZES[0]))]));
        let kept = rig.tick(32.1, true).unwrap_or_else(|| panic!("the densities"));
        assert!(kept.main.ppi().is_some() && kept.main == full.main, "{kept:?}");
        // A window on no display has its density unread.
        rig.current = 9;
        assert_eq!(rig.tick(34.0, true).map(|d| d.main), Some(ScreenPpi::Unread));
        // The window's display fails to answer the first reading: its density is unread, not a
        // display without a size.
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.finish(Ok(vec![Err("no answer".into()), Ok(Some(SIZES[1]))]));
        let first = rig.tick(0.1, true).unwrap_or_else(|| panic!("the densities"));
        assert_eq!((first.main, first.screens[0].ppi), (ScreenPpi::Unread, ScreenPpi::Unread), "{first:?}");
    }

    /// A reading that fails, or a thread that cannot start, is tried again after a while; the
    /// same displays are not read on every listing until then.
    #[test]
    fn a_failed_reading_is_tried_again() {
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.finish(Err("the display server went away".into()));
        assert_eq!(rig.tick(1.0, true), None);
        assert_eq!(rig.reads.get(), 1, "not again at once");
        rig.tick(5.0, true);
        assert_eq!(rig.reads.get(), 1);
        rig.tick(1.0 + RETRY_AFTER, true);
        assert_eq!(rig.reads.get(), 2, "tried again");
        rig.finish(sizes(2));
        assert!(rig.tick(12.0, true).is_some_and(|d| d.main.ppi().is_some()), "and read");
        let mut rig = Rig::new();
        rig.fail_to_start = true;
        rig.tick(0.0, true);
        rig.tick(1.0 + RETRY_AFTER, true);
        assert_eq!(rig.reads.get(), 2, "a thread that did not start is tried again too");
    }

    /// Displays that list the same are read again a while after the last reading, so a display
    /// replaced by one on the same connector at the same resolution and scale gets its own size.
    #[test]
    fn displays_that_list_the_same_are_read_again_after_a_while() {
        let mut rig = Rig::new();
        rig.tick(0.0, true);
        rig.finish(sizes(2));
        let first = rig.tick(0.1, true).unwrap_or_else(|| panic!("the densities"));
        rig.tick(0.1 + READ_AGAIN_AFTER - 1.0, true);
        assert_eq!(rig.reads.get(), 1, "not before the time is up");
        rig.tick(0.1 + READ_AGAIN_AFTER, true);
        assert_eq!(rig.reads.get(), 2, "read again");
        rig.finish(Ok(vec![Ok(Some(SIZES[1])), Ok(Some(SIZES[0]))]));
        let replaced = rig.tick(0.2 + READ_AGAIN_AFTER, true).unwrap_or_else(|| panic!("the densities"));
        assert!(replaced.main.ppi().is_some() && replaced.main != first.main, "the new panel's density: {first:?} then {replaced:?}");
    }
}
