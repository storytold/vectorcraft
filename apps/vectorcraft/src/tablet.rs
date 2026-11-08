//! Linux: a drawing tablet's pen pressure, tilt and barrel rotation under X11, XWayland included
//! (#764). winit 0.30 hands a pen to the window there as a mouse, without any of them, so the app
//! reads them from XInput2 itself and writes them into the UI's [`StylusFeed`]
//! (`Services::stylus`), which the canvas reads.
//!
//! The reader opens a second connection to the X server (x11rb's `RustConnection`, without
//! `unsafe` or libxcb) and selects XInput2 raw events for every device on the root window. Raw
//! events reach every client that selects them, also while winit holds the pointer grab of a
//! drag, and name the device that moved. A pointer device with an "Abs Pressure" valuator is a pen
//! (xf86-input-wacom, xf86-input-libinput and XWayland give a pen's pressure axis that label),
//! unless its XInput type is a touchpad's or a touchscreen's. Its valuators go into the feed:
//!
//! - "Abs Pressure", scaled from the valuator's range to 0..1;
//! - "Abs Tilt X" and "Abs Tilt Y" (a pen that has both), in degrees as they are when the range
//!   fits in ±90, otherwise scaled to ±[`TILT_DEGREES`];
//! - "Abs Rotary Z" (the barrel rotation of an Art Pen), scaled to a turn with the middle of the
//!   range at 0°.
//!
//! Any other pointer device moving sets the feed to `None`. Raw events carry only the valuators
//! that changed, so the last value of each axis of each pen is kept; an axis the pen has that no
//! event has carried yet reads 0 (upright, unturned), and an axis it lacks reads nothing.
//!
//! The X server lists a pen's eraser end as a device of its own, whose name or XInput type contains
//! "eraser" (xf86-input-wacom's "… Pen eraser" of type `ERASER`, xf86-input-libinput's
//! "… Eraser (0x…)" of type `ERASER`, XWayland's "xwayland-tablet eraser:N"). Its samples go into
//! the feed marked as the eraser end, and the canvas switches to the Eraser tool
//! ([`vectorcraft_ui_egui::stylus`]).
//!
//! The reader starts only for a window on X11. On native Wayland, `DISPLAY` names an XWayland
//! that receives none of the window's input.
//!
//! The mapping and the reader come from PhotoCraft's `crates/tablet` (`xi.rs` and `x11.rs`).
//! PhotoCraft maps the rotation to 0..360° and reads an axis a pen lacks as 0; here the middle of
//! the rotation's range is 0°, and an axis a pen lacks reads nothing.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::HashMap;

use vectorcraft_ui_egui::stylus::{PenPose, PenSample, StylusFeed};

/// The tilt of a valuator whose range does not fit in ±90 (so does not give degrees), at the ends
/// of its range.
pub(crate) const TILT_DEGREES: f64 = 60.0;

/// Valuator numbers from this one on are ignored (the protocol allows 65,535; tablets have fewer
/// than 16).
const MAX_AXES: u16 = 64;

/// Devices kept (a server could list thousands).
const MAX_DEVICES: usize = 256;

/// The XInput types of finger devices (`XI_TOUCHPAD` and `XI_TOUCHSCREEN`): xf86-input-evdev labels
/// a touchpad's or a touchscreen's pressure axis "Abs Pressure" too, and they are not pens.
const FINGER_TYPES: &[&str] = &["TOUCHPAD", "TOUCHSCREEN"];

/// Which pen axis a valuator carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AxisKind {
    Pressure,
    TiltX,
    TiltY,
    Rotation,
}

impl AxisKind {
    /// The kind of the valuator labeled `label`, if it is one of a pen's.
    fn from_label(label: &str) -> Option<Self> {
        match label {
            "Abs Pressure" => Some(Self::Pressure),
            "Abs Tilt X" => Some(Self::TiltX),
            "Abs Tilt Y" => Some(Self::TiltY),
            "Abs Rotary Z" => Some(Self::Rotation),
            _ => None,
        }
    }

    /// Where [`Pens`] keeps the last value of this axis.
    fn slot(self) -> usize {
        match self {
            Self::Pressure => 0,
            Self::TiltX => 1,
            Self::TiltY => 2,
            Self::Rotation => 3,
        }
    }
}

/// A valuator of a pen: what it carries, its number in a raw event's valuator mask, and its range.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Axis {
    kind: AxisKind,
    number: u16,
    min: f64,
    max: f64,
}

impl Axis {
    /// A raw value of this valuator as a pressure of 0..1, a tilt in degrees (−90..90) or a
    /// rotation in degrees (−180..180, the middle of the range at 0). A value that is not a number
    /// is a mouse's pressure, 1, or a tilt or rotation of 0.
    fn map(&self, v: f64) -> f32 {
        if !v.is_finite() {
            return if self.kind == AxisKind::Pressure { 1.0 } else { 0.0 };
        }
        let span = self.max - self.min;
        let unit = if span.is_finite() && span > 0.0 { (v - self.min) / span } else { v };
        let unit = unit.clamp(0.0, 1.0);
        match self.kind {
            AxisKind::Pressure => unit as f32,
            AxisKind::TiltX | AxisKind::TiltY if self.min >= -90.0 && self.max <= 90.0 && span > 0.0 => v.clamp(-90.0, 90.0) as f32,
            AxisKind::TiltX | AxisKind::TiltY => ((unit * 2.0 - 1.0) * TILT_DEGREES) as f32,
            AxisKind::Rotation => ((unit - 0.5) * 360.0) as f32,
        }
    }
}

/// A pointer device the X server lists.
#[derive(Clone, Debug, PartialEq)]
struct Device {
    id: u16,
    /// The name the X server lists, so that another device under the same id compares unequal.
    name: String,
    /// The pen's valuators: a pen has a pressure valuator, a mouse, a touchpad or a touchscreen
    /// has none (and keeps none).
    axes: Vec<Axis>,
    /// The eraser end of a pen: its name or its XInput type contains "eraser", in any case.
    eraser: bool,
}

impl Device {
    /// From what the X server lists for the device: its name and each valuator's number, label,
    /// minimum and maximum (`XIQueryDevice`), and its XInput type (`ListInputDevices`).
    fn new(id: u16, name: &str, kind: Option<&str>, valuators: impl IntoIterator<Item = (u16, String, f64, f64)>) -> Self {
        let finger = kind.is_some_and(|k| FINGER_TYPES.contains(&k));
        let mut axes: Vec<Axis> = Vec::new();
        for (number, label, min, max) in valuators {
            // The first valuator of each kind.
            if let Some(kind) = AxisKind::from_label(&label)
                && number < MAX_AXES
                && !axes.iter().any(|a| a.kind == kind)
            {
                axes.push(Axis { kind, number, min, max });
            }
        }
        if finger || !axes.iter().any(|a| a.kind == AxisKind::Pressure) {
            axes.clear();
        }
        let eraser = [Some(name), kind].into_iter().flatten().any(|s| s.to_ascii_lowercase().contains("eraser"));
        Self { id, name: name.to_string(), axes, eraser }
    }

    /// The valuator of `kind`, if the device is a pen that has one.
    fn axis(&self, kind: AxisKind) -> Option<&Axis> {
        self.axes.iter().find(|a| a.kind == kind)
    }

    /// A pen: a device with a pressure valuator.
    fn is_pen(&self) -> bool {
        !self.axes.is_empty()
    }
}

/// What a raw event means for the feed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Update {
    /// Nothing: a device the last listing didn't have (a master, or one added since).
    Keep,
    /// The pressure, end, tilt and rotation of a pen, or `None` for another pointer.
    Set(Option<PenSample>),
}

/// The pointer devices and the last pressure, tilt and rotation of each pen.
#[derive(Debug, Default)]
struct Pens {
    devices: HashMap<u16, Device>,
    last: HashMap<u16, [Option<f32>; 4]>,
}

impl Pens {
    /// Replace the device list (at the start, and when devices come and go or change).
    /// A device keeps its last values only while it is listed unchanged: another device under the
    /// same id starts without them.
    fn set_devices(&mut self, devices: impl IntoIterator<Item = Device>) {
        let devices: HashMap<u16, Device> = devices.into_iter().take(MAX_DEVICES).map(|d| (d.id, d)).collect();
        self.last.retain(|id, _| devices.get(id).is_some_and(|d| self.devices.get(id) == Some(d)));
        self.devices = devices;
    }

    /// A raw motion or button event of device `id`, with the valuators that changed.
    fn handle(&mut self, id: u16, valuators: impl IntoIterator<Item = (u16, f64)>) -> Update {
        let Some(device) = self.devices.get(&id) else { return Update::Keep };
        if !device.is_pen() {
            return Update::Set(None);
        }
        let last = self.last.entry(id).or_default();
        for (number, v) in valuators {
            for axis in device.axes.iter().filter(|a| a.number == number) {
                if let Some(slot) = last.get_mut(axis.kind.slot()) {
                    *slot = Some(axis.map(v));
                }
            }
        }
        // An axis the pen has reads 0 until an event carries it; one it lacks reads nothing.
        let value = |kind: AxisKind| device.axis(kind).map(|_| last.get(kind.slot()).copied().flatten().unwrap_or(0.0));
        let tilt = value(AxisKind::TiltX).zip(value(AxisKind::TiltY)).map(|(x, y)| [x, y]);
        let pose = PenPose { tilt, rotation: value(AxisKind::Rotation) };
        Update::Set(Some(PenSample { pressure: value(AxisKind::Pressure).unwrap_or(0.0), eraser: device.eraser, pose }))
    }

    /// A raw event as the X server sends it: its valuator mask and the values of the valuators
    /// set in it. A pen's sample or another pointer's `None` goes into `feed`.
    fn raw_event(&mut self, feed: &StylusFeed, id: u16, mask: &[u32], values: &[f64]) {
        if let Update::Set(sample) = self.handle(id, valuators(mask, values)) {
            feed.set_sample(sample);
        }
    }
}

/// The values a raw event carries: the valuator numbers set in `mask` (32 to a word, lowest bit
/// first), each paired in order with one of `values`. Mask bits past the last value are ignored.
fn valuators<'a>(mask: &'a [u32], values: &'a [f64]) -> impl Iterator<Item = (u16, f64)> + 'a {
    let numbers = mask
        .iter()
        .enumerate()
        .flat_map(|(w, bits)| (0..32u32).filter(move |b| bits & (1 << b) != 0).map(move |b| w.saturating_mul(32).saturating_add(b as usize)));
    numbers.zip(values.iter().copied()).filter_map(|(n, v)| u16::try_from(n).ok().map(|n| (n, v)))
}

/// An X 32.32 fixed-point value.
fn fp3232(integral: i32, frac: u32) -> f64 {
    f64::from(integral) + f64::from(frac) / 4_294_967_296.0
}

/// Start reading pen pressure, tilt and rotation for a window on X11: the feed the reader writes
/// into, or `None` (logged) when the X server can't be reached or has no XInput 2.1.
#[cfg(target_os = "linux")]
pub fn start() -> Option<StylusFeed> {
    let feed = StylusFeed::default();
    match x11::spawn(feed.clone()) {
        Ok(()) => Some(feed),
        Err(e) => {
            log::warn!("tablet pressure: {e}");
            None
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::collections::HashMap;

    use vectorcraft_ui_egui::stylus::StylusFeed;
    use x11rb::connection::{Connection as _, RequestConnection as _};
    use x11rb::protocol::Event;
    use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceClassData, DeviceType, XIEventMask};
    use x11rb::protocol::xproto::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    use super::{Device, MAX_AXES, MAX_DEVICES, Pens, fp3232};

    /// `XIAllDevices`: every device, the slave devices that name a pen included.
    const ALL_DEVICES: u16 = 0;

    /// Connect to `$DISPLAY` (the display winit opened the window on), check for XInput 2.1 (raw
    /// events for every client that selects them), select the raw events, and read them on a
    /// thread of its own until the connection closes.
    pub fn spawn(feed: StylusFeed) -> Result<(), String> {
        let (conn, screen) = RustConnection::connect(None).map_err(|e| format!("can't connect to the X server: {e}"))?;
        let root = conn.setup().roots.get(screen).map(|s| s.root).ok_or_else(|| format!("X screen {screen} not found"))?;
        let version =
            conn.xinput_xi_query_version(2, 2).map_err(|e| e.to_string())?.reply().map_err(|e| format!("the X server has no XInput 2: {e}"))?;
        if (version.major_version, version.minor_version) < (2, 1) {
            return Err(format!("the X server has XInput {}.{}, and raw events need 2.1", version.major_version, version.minor_version));
        }
        let mask = XIEventMask::RAW_MOTION
            | XIEventMask::RAW_BUTTON_PRESS
            | XIEventMask::RAW_BUTTON_RELEASE
            | XIEventMask::HIERARCHY
            | XIEventMask::DEVICE_CHANGED;
        conn.xinput_xi_select_events(root, &[xinput::EventMask { deviceid: ALL_DEVICES, mask: vec![mask] }])
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("XISelectEvents failed: {e}"))?;
        let mut pens = Pens::default();
        pens.set_devices(query_devices(&conn));
        std::thread::Builder::new()
            .name("vectorcraft-tablet-x11".into())
            .spawn(move || run(&conn, &mut pens, &feed))
            .map(drop)
            .map_err(|e| format!("can't start the tablet thread: {e}"))
    }

    fn run(conn: &RustConnection, pens: &mut Pens, feed: &StylusFeed) {
        // A connection error (the display went away) ends the loop; an event that x11rb cannot
        // parse is skipped, and `Pens` bounds what the rest can hold.
        while let Ok(raw) = conn.wait_for_raw_event() {
            let Ok(event) = conn.parse_event(&raw) else { continue };
            match event {
                Event::XinputRawMotion(e) | Event::XinputRawButtonPress(e) | Event::XinputRawButtonRelease(e) => {
                    let values: Vec<f64> = e.axisvalues.iter().map(|v| fp3232(v.integral, v.frac)).collect();
                    pens.raw_event(feed, e.deviceid, &e.valuator_mask, &values);
                }
                Event::XinputHierarchy(_) | Event::XinputDeviceChanged(_) => pens.set_devices(query_devices(conn)),
                _ => {}
            }
        }
    }

    /// The slave pointer devices with their names and pen valuators (none on any error).
    fn query_devices(conn: &RustConnection) -> Vec<Device> {
        let Ok(reply) = conn.xinput_xi_query_device(ALL_DEVICES).map_err(drop).and_then(|c| c.reply().map_err(drop)) else { return Vec::new() };
        let kinds = device_kinds(conn);
        let mut out = Vec::new();
        for info in reply.infos.iter().take(MAX_DEVICES) {
            if info.type_ != DeviceType::SLAVE_POINTER && info.type_ != DeviceType::FLOATING_SLAVE {
                continue;
            }
            let valuators = info.classes.iter().take(usize::from(MAX_AXES)).filter_map(|c| match &c.data {
                DeviceClassData::Valuator(v) => {
                    let label = atom_name(conn, v.label)?;
                    Some((v.number, label, fp3232(v.min.integral, v.min.frac), fp3232(v.max.integral, v.max.frac)))
                }
                _ => None,
            });
            let name = String::from_utf8_lossy(&info.name);
            out.push(Device::new(info.deviceid, &name, kinds.get(&info.deviceid).map(String::as_str), valuators.collect::<Vec<_>>()));
        }
        out
    }

    /// Each device's XInput type name, by device id (none on any error).
    fn device_kinds(conn: &RustConnection) -> HashMap<u16, String> {
        let Ok(reply) = conn.xinput_list_input_devices().map_err(drop).and_then(|c| c.reply().map_err(drop)) else { return HashMap::new() };
        reply.devices.iter().take(MAX_DEVICES).filter_map(|d| Some((u16::from(d.device_id), atom_name(conn, d.device_type)?))).collect()
    }

    fn atom_name(conn: &RustConnection, atom: u32) -> Option<String> {
        if atom == 0 {
            return None;
        }
        let reply = conn.get_atom_name(atom).ok()?.reply().ok()?;
        Some(String::from_utf8_lossy(&reply.name).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The valuators of a Wacom pen as the X server lists them: x and y, pressure from 0 to
    /// 65,535, tilt in degrees, and an Art Pen's barrel rotation.
    fn pen_valuators() -> Vec<(u16, String, f64, f64)> {
        vec![
            (0, "Abs X".to_string(), 0.0, 44704.0),
            (1, "Abs Y".to_string(), 0.0, 27940.0),
            (2, "Abs Pressure".to_string(), 0.0, 65535.0),
            (3, "Abs Tilt X".to_string(), -64.0, 63.0),
            (4, "Abs Tilt Y".to_string(), -64.0, 63.0),
            (5, "Abs Rotary Z".to_string(), -900.0, 899.0),
        ]
    }

    fn pen(id: u16) -> Device {
        Device::new(id, "Wacom Intuos Pro M Pen stylus", Some("STYLUS"), pen_valuators())
    }

    fn mouse(id: u16) -> Device {
        Device::new(id, "Logitech USB Optical Mouse", Some("MOUSE"), [(0, "Rel X".to_string(), -1.0, -1.0), (1, "Rel Y".to_string(), -1.0, -1.0)])
    }

    #[test]
    fn the_mask_pairs_valuator_numbers_with_values() {
        let v: Vec<_> = valuators(&[0b10_0101, 1], &[1.0, 2.0, 3.0, 4.0]).collect();
        assert_eq!(v, vec![(0, 1.0), (2, 2.0), (5, 3.0), (32, 4.0)]);
        // Fewer values than mask bits (malformed): the values that are there.
        assert_eq!(valuators(&[u32::MAX; 4], &[7.0]).count(), 1);
        assert_eq!(valuators(&[], &[7.0]).count(), 0);
    }

    #[test]
    fn a_device_with_an_abs_pressure_valuator_is_a_pen() {
        assert_eq!(pen(9).axis(AxisKind::Pressure), Some(&Axis { kind: AxisKind::Pressure, number: 2, min: 0.0, max: 65535.0 }));
        assert_eq!(pen(9).axes.len(), 4, "pressure, tilt and rotation");
        assert!(!mouse(12).is_pen());
        let pad = Device::new(13, "Wacom Intuos Pro M Pad pad", Some("PAD"), [(5, "Abs Wheel".to_string(), 0.0, 71.0)]);
        assert!(!pad.is_pen() && pad.axes.is_empty(), "a tablet's pad is no pen");
        let far = Device::new(14, "", Some("STYLUS"), [(MAX_AXES, "Abs Pressure".to_string(), 0.0, 1.0)]);
        assert!(!far.is_pen(), "valuator numbers past the limit are ignored");
        let tilt_only = Device::new(14, "", Some("STYLUS"), [(3, "Abs Tilt X".to_string(), -64.0, 63.0), (4, "Abs Tilt Y".to_string(), -64.0, 63.0)]);
        assert!(!tilt_only.is_pen() && tilt_only.axes.is_empty(), "no pressure, no pen");
        // The types libinput and XWayland give pens, and a device whose type wasn't listed.
        for kind in [Some("TABLET"), Some("ERASER"), Some("xwayland-tablet stylus"), None] {
            assert!(Device::new(15, "", kind, pen_valuators()).is_pen(), "{kind:?}");
        }
        // xf86-input-evdev labels a touchpad's and a touchscreen's finger pressure "Abs Pressure".
        for kind in ["TOUCHPAD", "TOUCHSCREEN"] {
            assert!(Device::new(16, "", Some(kind), pen_valuators()).axes.is_empty(), "{kind}");
        }
    }

    /// The eraser end of a pen is a device of its own, whose name or XInput type contains
    /// "eraser", as xf86-input-wacom, xf86-input-libinput and XWayland list it.
    #[test]
    fn an_eraser_device_is_named_so() {
        let erasers = [
            ("Wacom Intuos Pro M Pen eraser", Some("ERASER")),
            ("HUION Kamvas Pen Eraser (0x1)", Some("ERASER")),
            ("xwayland-tablet eraser:16", Some("xwayland-tablet eraser")),
            ("Wacom Intuos Pro M Pen eraser", None),
            ("Tablet Pen (0x2)", Some("ERASER")),
        ];
        for (name, kind) in erasers {
            let d = Device::new(10, name, kind, pen_valuators());
            assert!(d.eraser && d.is_pen(), "{name} ({kind:?})");
        }
        let tips = [
            ("Wacom Intuos Pro M Pen stylus", Some("STYLUS")),
            ("HUION Kamvas Pen Pen (0x1)", Some("STYLUS")),
            ("xwayland-tablet stylus:16", Some("xwayland-tablet stylus")),
            ("Logitech USB Optical Mouse", Some("MOUSE")),
            ("", None),
        ];
        for (name, kind) in tips {
            assert!(!Device::new(9, name, kind, pen_valuators()).eraser, "{name} ({kind:?})");
        }
    }

    #[test]
    fn the_eraser_device_sets_the_eraser_end() {
        let mut pens = Pens::default();
        pens.set_devices([pen(9), Device::new(10, "Wacom Intuos Pro M Pen eraser", Some("ERASER"), pen_valuators()), mouse(12)]);
        let feed = StylusFeed::default();
        let pose = PenPose { tilt: Some([0.0, 0.0]), rotation: Some(0.0) };
        pens.raw_event(&feed, 9, &[0b100], &[32767.5]);
        assert_eq!(pens.handle(9, []), Update::Set(Some(PenSample { pressure: 0.5, eraser: false, pose })));
        assert_eq!(feed.eraser(), Some(false));
        // Turned over, the pen hovers with its eraser end: the eraser device's first event, with
        // no pressure yet.
        pens.raw_event(&feed, 10, &[0b1], &[100.0]);
        assert_eq!(pens.handle(10, []), Update::Set(Some(PenSample { pressure: 0.0, eraser: true, pose })));
        assert_eq!((feed.eraser(), feed.held()), (Some(true), Some(0.0)));
        pens.raw_event(&feed, 10, &[0b100], &[65535.0]);
        assert_eq!((feed.eraser(), feed.held()), (Some(true), Some(1.0)));
        // Back to the tip, with the tip's own last pressure.
        pens.raw_event(&feed, 9, &[0b1], &[200.0]);
        assert_eq!((feed.eraser(), feed.held()), (Some(false), Some(0.5)));
        // A mouse: no pen end.
        pens.raw_event(&feed, 12, &[0b1], &[3.0]);
        assert_eq!(feed.eraser(), None);
    }

    /// #764: a pen's tilt goes into the feed in degrees and its barrel rotation as a turn, the
    /// middle of the valuator's range at 0°; a pen without those valuators reads neither.
    #[test]
    fn a_pens_tilt_and_rotation_go_into_the_feed() {
        let mut pens = Pens::default();
        let pressure_only = Device::new(10, "", Some("STYLUS"), pen_valuators().into_iter().take(3));
        let one_tilt = Device::new(11, "", Some("STYLUS"), pen_valuators().into_iter().take(4));
        pens.set_devices([pen(9), pressure_only, one_tilt, mouse(12)]);
        let feed = StylusFeed::default();
        // Pressure, tilt x, tilt y and rotation: valuators 2 to 5.
        pens.raw_event(&feed, 9, &[0b11_1100], &[32767.5, -30.0, 45.0, 0.0]);
        let pose = feed.pose().unwrap();
        assert_eq!((feed.held(), pose.tilt), (Some(0.5), Some([-30.0, 45.0])));
        assert!(pose.rotation.is_some_and(|r| r < 0.2), "{pose:?}");
        // A quarter of the range back is a quarter turn back; the tilt stays as it was.
        pens.raw_event(&feed, 9, &[0b10_0000], &[-450.0]);
        let pose = feed.pose().unwrap();
        assert_eq!(pose.tilt, Some([-30.0, 45.0]));
        assert!(pose.rotation.is_some_and(|r| (r - 270.0).abs() < 0.2), "{pose:?}");
        // A pen with no tilt or rotation valuators, or with one tilt valuator only, reads none.
        pens.raw_event(&feed, 10, &[0b100], &[65535.0]);
        assert_eq!((feed.held(), feed.pose()), (Some(1.0), None));
        pens.raw_event(&feed, 11, &[0b1100], &[65535.0, 20.0]);
        assert_eq!(feed.pose(), None);
        // The mouse: none.
        pens.raw_event(&feed, 12, &[0b1], &[3.0]);
        assert_eq!(feed.pose(), None);
        // A pen whose tilt and rotation no event has carried yet is upright and unturned.
        let mut fresh = Pens::default();
        fresh.set_devices([pen(9)]);
        let upright = PenPose { tilt: Some([0.0, 0.0]), rotation: Some(0.0) };
        assert_eq!(fresh.handle(9, [(2, 0.0)]), Update::Set(Some(PenSample { pressure: 0.0, eraser: false, pose: upright })));
    }

    /// A device listed again unchanged keeps its last tilt and rotation; another device listed
    /// under the same id starts upright and unturned.
    #[test]
    fn a_device_replaced_under_the_same_id_starts_without_the_old_pose() {
        let mut pens = Pens::default();
        pens.set_devices([pen(9)]);
        pens.handle(9, [(3, -30.0), (4, 45.0), (5, -450.0)]);
        pens.set_devices([pen(9)]);
        let Update::Set(Some(PenSample { pose, .. })) = pens.handle(9, [(2, 0.0)]) else { panic!("no pen") };
        assert_eq!(pose.tilt, Some([-30.0, 45.0]));
        let other = Device::new(9, "HUION Kamvas Pen Pen (0x1)", Some("STYLUS"), pen_valuators());
        pens.set_devices([other]);
        let upright = PenPose { tilt: Some([0.0, 0.0]), rotation: Some(0.0) };
        assert_eq!(pens.handle(9, [(2, 0.0)]), Update::Set(Some(PenSample { pressure: 0.0, eraser: false, pose: upright })));
    }

    /// Tilt valuators whose range does not fit in ±90 are scaled to ±60°.
    #[test]
    fn tilt_ranges_that_are_not_degrees_are_scaled() {
        let d = Device::new(
            1,
            "",
            None,
            [(2, "Abs Pressure".into(), 0.0, 1023.0), (3, "Abs Tilt X".into(), 0.0, 127.0), (4, "Abs Tilt Y".into(), -1000.0, 1000.0)],
        );
        let mut pens = Pens::default();
        pens.set_devices([d]);
        let Update::Set(Some(PenSample { pose, .. })) = pens.handle(1, [(3, 127.0), (4, -1000.0)]) else { panic!("no pen") };
        assert_eq!(pose.tilt, Some([60.0, -60.0]));
    }

    #[test]
    fn a_pens_valuator_becomes_its_pressure_and_another_pointer_clears_it() {
        let mut pens = Pens::default();
        pens.set_devices([pen(9), mouse(12)]);
        let feed = StylusFeed::default();
        pens.raw_event(&feed, 9, &[0b101], &[100.0, 32767.5]);
        assert_eq!(feed.held(), Some(0.5));
        // Raw events carry only the valuators that changed: a move without pressure keeps it.
        pens.raw_event(&feed, 9, &[0b1], &[200.0]);
        assert_eq!(feed.held(), Some(0.5));
        pens.raw_event(&feed, 9, &[0b100], &[65535.0]);
        assert_eq!(feed.held(), Some(1.0));
        // The mouse moves: no pen pressure until the pen moves again.
        pens.raw_event(&feed, 12, &[0b1], &[3.0]);
        assert_eq!(feed.held(), None);
        pens.raw_event(&feed, 9, &[0b1], &[300.0]);
        assert_eq!(feed.held(), Some(1.0), "the pen's last pressure");
        // A master device or one not listed yet changes nothing.
        pens.raw_event(&feed, 2, &[0b100], &[0.0]);
        assert_eq!(feed.held(), Some(1.0));
        // A pen whose first event carries no pressure is hovering: it presses with 0.
        pens.set_devices([pen(9), pen(10)]);
        pens.raw_event(&feed, 10, &[0b1], &[5.0]);
        assert_eq!(feed.press(), Some(0.0));
    }

    #[test]
    fn hostile_device_data_gives_a_pressure_within_0_to_1() {
        let ranges = [(5.0, 5.0), (1.0, -1.0), (f64::NEG_INFINITY, f64::INFINITY), (f64::NAN, f64::NAN), (90.0, -90.0)];
        for (min, max) in ranges {
            let mut pens = Pens::default();
            let labels = ["Abs Pressure", "Abs Tilt X", "Abs Tilt Y", "Abs Rotary Z"];
            pens.set_devices([Device::new(1, "", None, (2..).zip(labels).map(|(n, l)| (n, l.to_string(), min, max)))]);
            for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, -0.0, 1e300, 0.5] {
                let Update::Set(Some(PenSample { pressure: p, pose, .. })) = pens.handle(1, (2..6).map(|n| (n, v))) else {
                    panic!("{min}..{max}: {v} gave no pressure")
                };
                assert!((0.0..=1.0).contains(&p), "{min}..{max}: {v} gave {p}");
                let tilt = pose.tilt.unwrap_or_default();
                assert!(tilt.iter().all(|t| (-90.0..=90.0).contains(t)), "{min}..{max}: {v} gave {pose:?}");
                assert!(pose.rotation.is_some_and(|r| (-180.0..=180.0).contains(&r)), "{min}..{max}: {v} gave {pose:?}");
            }
        }
        let mut pens = Pens::default();
        pens.set_devices((0..2000u16).map(|i| Device::new(i, "", None, [])));
        assert!(pens.devices.len() <= MAX_DEVICES, "the device list is bounded");
        // Values for valuators the pen doesn't have are ignored.
        pens.set_devices([pen(3)]);
        let Update::Set(Some(PenSample { pressure: p, .. })) = pens.handle(3, valuators(&[u32::MAX; 2048], &[0.5; 4096]).collect::<Vec<_>>()) else {
            panic!("the pen gave no pressure")
        };
        assert!(p < 0.001, "{p}");
    }

    #[test]
    fn fixed_point_values() {
        assert_eq!(fp3232(3, 1 << 31), 3.5);
        assert_eq!(fp3232(-2, 0), -2.0);
    }
}
