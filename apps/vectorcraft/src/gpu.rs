//! Which graphics adapter the window renders with (#306, #502), and what happens when it can't
//! show the window.
//!
//! The canvas is rasterized on the CPU; the GPU only composites it and draws the UI, so any
//! adapter that can present to the window will do. [`adapter_order`] ranks the adapters: those that
//! can't present to the window's surface never, then the one `WGPU_ADAPTER_NAME` names, hardware
//! before software, the native backends (Vulkan, Metal, DX12) before OpenGL, the GPU the Wayland
//! compositor draws with ([`crate::compositor_gpu`]), the GPU that drives a display and the one
//! that drives the primary display ([`display_gpus`]), the power preference
//! ([`power_preference`]) among the rest, and a GPU's DX12 adapter before its Vulkan one (#545).
//! eframe is handed [`selector`], which takes the first of that order.
//!
//! A GPU without a monitor is the wrong one even when it can present: on a desktop whose monitors
//! all hang off the discrete GPU, power saving picked the Ryzen's integrated GPU, Windows had to
//! present every frame across adapters, and the integrated GPU's driver reset under that, which
//! lost the graphics device, or took the desktop's compositor and every monitor down for minutes
//! (pdfcraft#378). So, when Preferences say Automatic, the GPU that drives the display the user
//! looks at comes first, and the power preference only orders the rest. Windows says which adapter
//! drives which display (`EnumDisplayDevices`), Linux through sysfs; elsewhere, or when nothing
//! matches, the order is the power preference's as before. Power Saving and High Performance,
//! chosen by the user, order by kind as they always did.
//!
//! Under Wayland, the compositor's own GPU comes before the displays: the compositor says which
//! GPU it draws with, and frames from another one may never reach the screen, with no error, even
//! from the GPU behind the built-in panel when the compositor runs on the discrete one.
//!
//! An adapter can report that it presents to the window and still fail once it does: on a hybrid
//! Linux desktop under Wayland, the compositor runs on one GPU and may refuse the frame buffers
//! another GPU allocates, which kills the window's Wayland connection; egui-wgpu then panics while
//! configuring the surface (#502). Nothing in the process can show a window after that, so when
//! the graphics fail while the window is starting up, [`finish`] starts the app again without
//! that adapter, as the next one in the order would be tried. Each restart leaves out one more
//! adapter, so it ends when none is left.
//!
//! An adapter can also hang instead of failing: on a hybrid laptop with High Performance chosen,
//! the discrete GPU never put the window's first frame on the screen, with no error and no panic,
//! and the app stayed in the background without a window (#964). [`watch_first_frame`] gives the
//! first frame [`FIRST_FRAME_TIMEOUT`] once the app is created and, when it doesn't come, starts
//! the app again without that adapter in the same way.
//!
//! A driver can also crash the whole process, and then no code in it runs to start it again:
//! Intel's Direct3D 12 driver (`igd12umd64.dll`) ended 0.9.0 before its window appeared (#2243).
//! So each start writes [`START_FILE`] in the log folder before the graphics start, adds the
//! adapter once one is picked, and clears both once the first frame reaches the screen or the app
//! ends without crashing ([`Startup::track`]). The next start that finds a start still recorded
//! there leaves that adapter out, as a restart does, and keeps leaving it out; when the crash came
//! while the adapters were being listed, before one was picked, it lists OpenGL's adapters alone
//! (except on macOS, where wgpu has none). A start that `WGPU_BACKEND` or `WGPU_ADAPTER_NAME`
//! chooses leaves nothing out, and the crashes are forgotten when the app version or
//! Preferences › Performance › Graphics Processor changes, when a start shows its first frame on
//! the adapter that crashed, or when no adapter is left to show the window.

use std::io::{Read as _, Seek as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::NativeAdapterSelectorMethod;
use eframe::wgpu::{self, Backend, Backends, DeviceType, PowerPreference};

/// The adapters a restart leaves out, those the window failed on, as comma-separated [`key`]s
/// (`Vulkan:1002:164e`, `Metal:Intel Iris Pro Graphics`). Set by the app itself when it starts
/// again.
pub const SKIP_ENV: &str = "VECTORCRAFT_GPU_SKIP";

/// The file in the log folder that records each start of the window until its first frame
/// reaches the screen, and the adapters earlier starts crashed on ([`Startup::track`]).
pub const START_FILE: &str = "graphics-start.json";

/// The most bytes of [`START_FILE`] read, and the most crashed adapters it keeps.
const START_FILE_LIMIT: u64 = 64 * 1024;
const CRASHED_LIMIT: usize = 16;

/// wgpu's variable for choosing an adapter by (part of) its name, any case.
const NAME_ENV: &str = "WGPU_ADAPTER_NAME";

/// Where to read how to choose another adapter, for the error that ends the app.
const HELP: &str =
    "to choose a graphics adapter, see https://github.com/storytold/vectorcraft/blob/main/docs/development.md#desktop-graphics-processor";

/// Frames the UI ran before the failure, below which it was still starting up. The failure in
/// #502 came on the first or second frame; later, the adapter has proved itself, and a lost
/// window is something else (such as the compositor restarting).
pub const STARTUP_FRAMES: u64 = 10;

/// How long the window's first frame may take to reach the screen, from the app's creation,
/// before its adapter counts as hanging ([`watch_first_frame`]). The first frame draws the whole
/// interface and compiles its shaders, a second at most on a slow machine.
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// The preference used when Preferences say Automatic. Windows and macOS show frames from any GPU
/// (the integrated one avoids the flicker of #306). Elsewhere, the system's own order: Mesa's
/// device-select layer puts the GPU the desktop runs on first (the integrated one on hybrid
/// laptops), and a Wayland compositor may not show frames from another GPU (#502).
pub const AUTOMATIC: PowerPreference = if cfg!(any(windows, target_os = "macos")) { PowerPreference::LowPower } else { PowerPreference::None };

/// The power preference that orders the adapters: the `WGPU_POWER_PREF` environment variable
/// (`low`, `high`, `none`) when set, otherwise Preferences › Performance › Graphics Processor
/// (`gpuPreference`). Automatic, and any value this version doesn't know (0.5.0's default
/// `powerSaving`, a newer or damaged preference file), is [`AUTOMATIC`].
pub fn power_preference(pref: Option<&str>, env: Option<PowerPreference>) -> PowerPreference {
    match (env, pref) {
        (Some(p), _) => p,
        (None, Some("highPerformance")) => PowerPreference::HighPerformance,
        (None, Some("lowPower")) => PowerPreference::LowPower,
        (None, _) => AUTOMATIC,
    }
}

/// Whether the adapter is the app's to choose: neither `WGPU_POWER_PREF` nor Preferences name a
/// kind of GPU (the arguments of [`power_preference`]). Then the GPU that drives the display comes
/// first ([`display_gpus`]); a user's own choice is kept as it is.
pub fn automatic(pref: Option<&str>, env: Option<PowerPreference>) -> bool {
    env.is_none() && !matches!(pref, Some("highPerformance" | "lowPower"))
}

/// The GPUs to rank first: those that drive a display, when the choice is [`automatic`].
pub fn preferred_displays(pref: Option<&str>, env: Option<PowerPreference>) -> Vec<DisplayGpu> {
    if automatic(pref, env) { display_gpus() } else { Vec::new() }
}

/// The GPU to rank first: the one the Wayland compositor draws with, when the choice is
/// [`automatic`].
pub fn preferred_compositor(pref: Option<&str>, env: Option<PowerPreference>) -> Option<(u32, u32)> {
    if automatic(pref, env) { crate::compositor_gpu::compositor_gpu() } else { None }
}

/// The graphics backends the window starts with: on Windows Direct3D 12, falling back to OpenGL,
/// unless `WGPU_BACKEND` names others (`env_set`; `default` then holds them). Creating a Vulkan
/// instance loads every installed Vulkan driver into the process, and a faulty one (Intel's
/// `igvk64.dll`, #806) crashed the app before its window appeared. DX12 is Windows' own backend,
/// as in PdfCraft. Elsewhere, `default`.
pub fn backends(default: Backends, env_set: bool) -> Backends {
    if cfg!(windows) && !env_set { Backends::DX12 | Backends::GL } else { default }
}

/// The largest texture side to ask the device for, which bounds the window's surface: the
/// adapter's own limit when it is above eframe's `default` (8192 px), so a window larger than
/// that keeps its surface (#1459). Never below `default`, which is what was asked for before.
pub fn surface_limit(default: u32, adapter: u32) -> u32 {
    default.max(adapter)
}

/// One adapter, as [`adapter_order`] sees it.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Identifies the adapter across a restart ([`key`]).
    pub key: String,
    pub name: String,
    pub backend: Backend,
    pub device_type: DeviceType,
    /// PCI vendor and device ids, to match a [`DisplayGpu`]; zero where the backend reports none
    /// (Metal, and OpenGL's device id).
    pub pci: (u32, u32),
    /// Reports that it can present to the window's surface.
    pub presents: bool,
}

impl Candidate {
    fn new(adapter: &wgpu::Adapter, surface: Option<&wgpu::Surface<'_>>) -> Self {
        let info = adapter.get_info();
        Self {
            key: key(info.backend, info.vendor, info.device, &info.name),
            presents: surface.is_none_or(|s| adapter.is_surface_supported(s)),
            name: info.name,
            backend: info.backend,
            device_type: info.device_type,
            pci: (info.vendor, info.device),
        }
    }

    /// Whether it is the GPU the compositor draws with (`compositor`, by PCI ids).
    fn composites(&self, compositor: Option<(u32, u32)>) -> bool {
        compositor == Some(self.pci)
    }

    /// Whether it drives one of `displays`, and whether that is the primary one.
    fn drives(&self, displays: &[DisplayGpu]) -> (bool, bool) {
        let shown = displays.iter().find(|g| g.pci == self.pci);
        (shown.is_some(), shown.is_some_and(|g| g.primary))
    }
}

/// A GPU with a monitor attached, as the system reports it ([`display_gpus`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayGpu {
    /// PCI `(vendor, device)` ids, as wgpu's `AdapterInfo` reports them.
    pub pci: (u32, u32),
    /// Drives the display the user most likely looks at: on Windows the primary display, on Linux
    /// a built-in panel (`eDP`, `LVDS` or `DSI`).
    pub primary: bool,
}

impl std::fmt::Display for DisplayGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04x}:{:04x}{}", self.pci.0, self.pci.1, if self.primary { " (primary)" } else { "" })
    }
}

/// The GPUs that drive a display, each once. Windows enumerates its display devices
/// (`EnumDisplayDevices`: those attached to the desktop, and the primary one); Linux reads the
/// connected connectors in `/sys/class/drm`. macOS and the rest report nothing, so the power
/// preference decides as before; so does a system whose GPUs report no PCI ids.
pub fn display_gpus() -> Vec<DisplayGpu> {
    #[cfg(windows)]
    {
        windows_display_gpus()
    }
    #[cfg(target_os = "linux")]
    {
        linux_display_gpus(std::path::Path::new("/sys/class/drm"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}

/// Record that the GPU `pci` drives a display, merging it with its other displays.
#[cfg(any(windows, target_os = "linux", test))]
fn add_display_gpu(gpus: &mut Vec<DisplayGpu>, pci: (u32, u32), primary: bool) {
    match gpus.iter_mut().find(|g| g.pci == pci) {
        Some(gpu) => gpu.primary |= primary,
        None => gpus.push(DisplayGpu { pci, primary }),
    }
}

/// Windows' display devices: one per GPU output; those `ATTACHED_TO_DESKTOP` show a monitor, one
/// of them the `PRIMARY_DEVICE`. The registry doesn't tell a user which adapter drives a display,
/// and WMI reports a resolution for a GPU without a monitor, so this is the way.
#[cfg(windows)]
fn windows_display_gpus() -> Vec<DisplayGpu> {
    use winsafe::co::DISPLAY_DEVICE as Flags;
    let mut gpus = Vec::new();
    // A machine has a handful of display devices; the cap only bounds a runaway enumeration.
    for device in winsafe::EnumDisplayDevices(None, None).take(256) {
        let device = match device {
            Ok(d) => d,
            // The enumeration ends with "no more items" or, from a driver, with any other error.
            Err(e) => {
                log::debug!("display devices: {e}");
                break;
            }
        };
        if !device.StateFlags.has(Flags::ATTACHED_TO_DESKTOP) {
            continue;
        }
        // Remote Desktop and other non-PCI devices (`ROOT\BasicDisplay\0000`) are left out.
        let Some(pci) = pci_ids(&device.DeviceID()) else { continue };
        add_display_gpu(&mut gpus, pci, device.StateFlags.has(Flags::PRIMARY_DEVICE));
    }
    gpus
}

/// The PCI `(vendor, device)` of a Windows device id such as
/// `PCI\VEN_10DE&DEV_2204&SUBSYS_40421458&REV_A1`, any case; `None` for anything else.
#[cfg(any(windows, test))]
fn pci_ids(device_id: &str) -> Option<(u32, u32)> {
    let field = |key: &str| {
        device_id.split(['\\', '&']).find_map(|part| {
            let (name, hex) = part.split_at_checked(key.len())?;
            if name.eq_ignore_ascii_case(key) { u32::from_str_radix(hex, 16).ok() } else { None }
        })
    };
    Some((field("VEN_")?, field("DEV_")?))
}

/// Linux: every connector `cardN-CONNECTOR` under `drm` whose `status` is `connected`, with the
/// PCI ids of its card (`cardN/device/{vendor,device}`). A built-in panel is the primary display.
#[cfg(any(target_os = "linux", test))]
fn linux_display_gpus(drm: &std::path::Path) -> Vec<DisplayGpu> {
    let read_hex = |p: std::path::PathBuf| -> Option<u32> {
        let s = std::fs::read_to_string(p).ok()?;
        u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
    };
    let mut gpus = Vec::new();
    let Ok(entries) = std::fs::read_dir(drm) else { return gpus };
    // A machine has a handful of connectors; the cap only bounds a pathological sysfs.
    for entry in entries.flatten().take(256) {
        let name = entry.file_name();
        let Some((card, connector)) = name.to_str().and_then(|n| n.split_once('-')) else { continue };
        let connected = std::fs::read_to_string(entry.path().join("status")).is_ok_and(|s| s.trim() == "connected");
        if !connected {
            continue;
        }
        let device = drm.join(card).join("device");
        let (Some(v), Some(d)) = (read_hex(device.join("vendor")), read_hex(device.join("device"))) else { continue };
        add_display_gpu(&mut gpus, (v, d), is_internal_panel(connector));
    }
    gpus
}

/// A laptop's built-in panel: an embedded DisplayPort, LVDS or DSI connector.
#[cfg(any(target_os = "linux", test))]
fn is_internal_panel(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|kind| connector.starts_with(kind))
}

/// `backend:vendor:device`, e.g. `Vulkan:1002:164e` (PCI ids, as `MESA_VK_DEVICE_SELECT` takes them).
/// Metal reports no ids (`0000:0000` for every GPU), so an adapter without them is `backend:name`
/// (`Metal:Intel Iris Pro Graphics`): otherwise leaving out the one that failed would leave out
/// every GPU of a dual-GPU Mac (#651). Commas, which separate [`SKIP_ENV`]'s keys, become spaces.
fn key(backend: Backend, vendor: u32, device: u32, name: &str) -> String {
    if vendor == 0 && device == 0 {
        format!("{backend:?}:{}", name.replace(',', " ").trim())
    } else {
        format!("{backend:?}:{vendor:04x}:{device:04x}")
    }
}

/// The order to try `candidates` in (their indices): only those that present to the window and
/// aren't in `skip`; the one whose name contains `named` (any case) first; hardware before
/// software; native backends before OpenGL; the GPU the compositor draws with (`compositor`); the
/// GPUs that drive one of `displays`, the primary display's first; then by `power`; then DX12
/// before Vulkan, keeping the system's order among equals (all of it for
/// [`PowerPreference::None`]). Windows lists each GPU under both, and Intel's
/// Vulkan driver made the whole window flicker black where DX12 and OpenGL didn't (#545);
/// elsewhere there is no DX12 adapter, so the order is unchanged.
///
/// With no `displays`, or none a candidate's PCI ids match (OpenGL reports no device id, Metal no
/// ids at all), the order is the power preference's alone, as it was.
pub fn adapter_order(
    candidates: &[Candidate],
    power: PowerPreference,
    displays: &[DisplayGpu],
    compositor: Option<(u32, u32)>,
    named: Option<&str>,
    skip: &[String],
) -> Vec<usize> {
    let named = named.map(str::to_lowercase).filter(|n| !n.is_empty());
    let mut order: Vec<(usize, &Candidate)> = candidates.iter().enumerate().filter(|(_, c)| c.presents && !skip.contains(&c.key)).collect();
    // Stable, so equals keep the system's order.
    order.sort_by_key(|(_, c)| {
        let unnamed = named.as_ref().is_some_and(|n| !c.name.to_lowercase().contains(n));
        let (drives_display, drives_primary) = c.drives(displays);
        (
            unnamed,
            c.device_type == DeviceType::Cpu,
            c.backend == Backend::Gl,
            !c.composites(compositor),
            !drives_display,
            !drives_primary,
            power_rank(c.device_type, power),
            c.backend == Backend::Vulkan,
        )
    });
    order.into_iter().map(|(i, _)| i).collect()
}

/// wgpu's own ranking for a power preference: the preferred kind of GPU, the other kind, unknown,
/// virtual, software.
fn power_rank(t: DeviceType, power: PowerPreference) -> u8 {
    match (power, t) {
        (PowerPreference::None, _) => 0,
        (PowerPreference::LowPower, DeviceType::IntegratedGpu) | (PowerPreference::HighPerformance, DeviceType::DiscreteGpu) => 0,
        (_, DeviceType::IntegratedGpu | DeviceType::DiscreteGpu) => 1,
        (_, DeviceType::Other) => 2,
        (_, DeviceType::VirtualGpu) => 3,
        (_, DeviceType::Cpu) => 4,
    }
}

/// The adapters [`SKIP_ENV`] lists.
pub fn skipped() -> Vec<String> {
    std::env::var(SKIP_ENV).map(|v| parse_skip(&v)).unwrap_or_default()
}

fn parse_skip(v: &str) -> Vec<String> {
    v.split(',').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect()
}

/// Whether `WGPU_BACKEND` or `WGPU_ADAPTER_NAME` is set: then a start uses the backends and the
/// adapter they name, and leaves out none that earlier starts crashed on.
pub fn explicit_choice() -> bool {
    std::env::var_os("WGPU_BACKEND").is_some() || std::env::var_os(NAME_ENV).is_some()
}

/// How the last start ended when it ended before the window's first frame reached the screen,
/// with no error or panic to report: a crash in the graphics driver (#2243), or the process was
/// ended from outside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Crash {
    /// While the adapters were being listed, before one was picked.
    Listing,
    /// On the adapter with this [`key`] and name.
    Adapter { key: String, name: String },
}

/// What a start leaves out because earlier starts crashed ([`Startup::track`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// The adapters ([`key`]s) that crashed a start.
    pub adapters: Vec<String>,
    /// A start crashed while listing the adapters: list OpenGL's alone. Never on macOS, where wgpu
    /// has no OpenGL adapters to fall back to.
    pub opengl_only: bool,
    /// The crash the last start ended in, when this start leaves something out for it.
    pub crash: Option<Crash>,
}

impl LeftOut {
    /// `backends` to list: OpenGL alone after a crash while listing, when it is among them;
    /// otherwise `backends`.
    pub fn backends(&self, backends: Backends) -> Backends {
        if self.opengl_only && backends.contains(Backends::GL) { Backends::GL } else { backends }
    }
}

/// An adapter in [`START_FILE`]: its [`key`] and name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Recorded {
    key: String,
    name: String,
}

/// The start in progress, in [`START_FILE`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Starting {
    /// The adapter picked; `None` while the adapters are being listed.
    adapter: Option<Recorded>,
    /// The listing found an adapter of a backend other than OpenGL.
    native: bool,
}

/// What [`START_FILE`] holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StartRecord {
    /// The app version and the Graphics Processor preference (`gpuPreference`) it was written
    /// under: the crashes count only for the same ones.
    version: String,
    preference: String,
    /// The start in progress, from before the graphics start until its first frame reaches the
    /// screen or the app ends without crashing.
    starting: Option<Starting>,
    /// The adapters that crashed a start, oldest first.
    crashed: Vec<Recorded>,
    /// A start crashed while listing the adapters.
    listing_crashed: bool,
}

impl StartRecord {
    /// This start's record, made from the one the last start left (`previous`; `None` when there
    /// is none or it can't be read), and the crash that start ended in, if it did. A record
    /// written by another version or under another `preference` is dropped.
    fn begin(previous: Option<Self>, version: &str, preference: &str) -> (Self, Option<Crash>) {
        let mut record = previous.filter(|r| r.version == version && r.preference == preference).unwrap_or_default();
        let crash = record.starting.take().map(|s| match s.adapter {
            Some(Recorded { key, name }) => Crash::Adapter { key, name },
            None => Crash::Listing,
        });
        match &crash {
            Some(Crash::Adapter { key, name }) if !record.crashed.iter().any(|c| &c.key == key) => {
                record.crashed.push(Recorded { key: key.clone(), name: name.clone() });
                if record.crashed.len() > CRASHED_LIMIT {
                    record.crashed.remove(0);
                }
            }
            Some(Crash::Listing) => record.listing_crashed = true,
            _ => {}
        }
        record.version = version.to_string();
        record.preference = preference.to_string();
        record.starting = Some(Starting::default());
        (record, crash)
    }

    /// What this start leaves out: nothing when `explicit` ([`explicit_choice`]). `crash` is the
    /// one [`begin`](Self::begin) found.
    fn left_out(&self, crash: Option<Crash>, explicit: bool) -> LeftOut {
        if explicit {
            return LeftOut::default();
        }
        let opengl_only = self.listing_crashed && !cfg!(target_os = "macos");
        let crash = crash.filter(|c| matches!(c, Crash::Adapter { .. }) || opengl_only);
        LeftOut { adapters: self.crashed.iter().map(|c| c.key.clone()).collect(), opengl_only, crash }
    }

    /// The adapter this start picked, after a listing that found one of a backend other than
    /// OpenGL when `native`.
    fn picked(&mut self, key: &str, name: &str, native: bool) {
        self.starting = Some(Starting { adapter: Some(Recorded { key: key.to_string(), name: name.trim().to_string() }), native });
    }

    /// The listing found no adapter that can show the window: nothing is left to leave out, so the
    /// crashes are forgotten and the next start tries every adapter again.
    fn none_left(&mut self) {
        self.crashed.clear();
        self.listing_crashed = false;
    }

    /// The window's first frame reached the screen: the adapter it was drawn on, and the listing
    /// when it found an adapter of a backend other than OpenGL, no longer count as crashed.
    fn presented(&mut self) {
        let Some(starting) = self.starting.take() else { return };
        if let Some(adapter) = starting.adapter {
            self.crashed.retain(|c| c.key != adapter.key);
        }
        if starting.native {
            self.listing_crashed = false;
        }
    }

    /// The start ended without crashing: a normal exit, an error, or a restart on another adapter.
    fn ended(&mut self) {
        self.starting = None;
    }

    fn to_json(&self) -> String {
        let recorded = |r: &Recorded| serde_json::json!({ "key": r.key, "name": r.name });
        let mut v = serde_json::json!({
            "version": self.version,
            "gpuPreference": self.preference,
            "crashed": self.crashed.iter().map(recorded).collect::<Vec<_>>(),
            "listingCrashed": self.listing_crashed,
        });
        if let (Some(s), Some(map)) = (&self.starting, v.as_object_mut()) {
            map.insert("starting".into(), serde_json::json!({ "adapter": s.adapter.as_ref().map(recorded), "native": s.native }));
        }
        v.to_string()
    }

    /// The record in `text`; `None` when it isn't one.
    fn parse(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let recorded = |v: &serde_json::Value| -> Option<Recorded> {
            Some(Recorded { key: v.get("key")?.as_str()?.to_string(), name: v.get("name")?.as_str()?.to_string() })
        };
        let starting = match v.get("starting") {
            None => None,
            Some(s) => Some(Starting {
                adapter: match s.get("adapter") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(a) => Some(recorded(a)?),
                },
                native: s.get("native")?.as_bool()?,
            }),
        };
        let crashed = v.get("crashed")?.as_array()?;
        Some(Self {
            version: v.get("version")?.as_str()?.to_string(),
            preference: v.get("gpuPreference")?.as_str()?.to_string(),
            starting,
            crashed: crashed.iter().take(CRASHED_LIMIT).map(recorded).collect::<Option<_>>()?,
            listing_crashed: v.get("listingCrashed")?.as_bool()?,
        })
    }
}

/// [`START_FILE`], open and locked while the start runs, so that a second copy of the app starting
/// at the same time doesn't take it for the record of a start that crashed.
struct StartFile {
    file: std::fs::File,
    record: StartRecord,
}

impl StartFile {
    /// Open `dir`'s [`START_FILE`], with the record the last start left in it (`None` when there is
    /// none or it can't be read). `Ok(None)` when another copy of the app holds it.
    fn open(dir: &Path) -> std::io::Result<Option<(std::fs::File, Option<StartRecord>)>> {
        std::fs::create_dir_all(dir)?;
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(START_FILE))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        let mut text = String::new();
        // Text that isn't UTF-8, or is longer than any record, is ignored like a damaged record.
        let previous = (&file).take(START_FILE_LIMIT).read_to_string(&mut text).ok().and_then(|_| StartRecord::parse(&text));
        Ok(Some((file, previous)))
    }

    fn write(&mut self) {
        let text = self.record.to_json();
        let written = self.file.set_len(0).and_then(|()| self.file.rewind()).and_then(|()| self.file.write_all(text.as_bytes()));
        if let Err(e) = written {
            log::warn!("{START_FILE}: {e}");
        }
    }
}

/// The value of `gpuPreference` the crashes are recorded under: any value this version doesn't
/// know reads as `automatic`, as in [`power_preference`].
fn preference_name(pref: Option<&str>) -> &'static str {
    match pref {
        Some("highPerformance") => "highPerformance",
        Some("lowPower") => "lowPower",
        _ => "automatic",
    }
}

/// What the window's start-up leaves for [`finish`].
#[derive(Default)]
pub struct Startup {
    /// The key of the adapter [`selector`] picked.
    adapter: OnceLock<String>,
    /// The UI's context, which counts the frames run.
    ui: OnceLock<egui::Context>,
    /// When the app was created, the files named at the start opened: the first frame is due.
    ready: OnceLock<Instant>,
    /// [`START_FILE`] while the start runs ([`Self::track`]).
    start: Mutex<Option<StartFile>>,
}

impl Startup {
    /// The app was created on `ctx`.
    pub fn created(&self, ctx: &egui::Context) {
        // Set once; the window is created once.
        let _ = self.ui.set(ctx.clone());
    }

    /// The app is ready to draw its first frame.
    pub fn ready(&self) {
        // Set once, like `created`.
        let _ = self.ready.set(Instant::now());
    }

    /// Record this start in `dir`'s [`START_FILE`] until the window's first frame reaches the
    /// screen or the app ends without crashing, and say what it leaves out for the starts that
    /// crashed before (see the module docs). `preference` is Preferences › Performance › Graphics
    /// Processor, and `explicit` is [`explicit_choice`]. When the file can't be used (it can't be
    /// written, or another copy of the app is starting), nothing is recorded or left out.
    pub fn track(&self, dir: &Path, preference: Option<&str>, explicit: bool) -> LeftOut {
        let (file, previous) = match StartFile::open(dir) {
            Ok(Some(opened)) => opened,
            Ok(None) => {
                log::info!("{START_FILE}: another VectorCraft is starting, so this start isn't recorded");
                return LeftOut::default();
            }
            Err(e) => {
                log::warn!("{START_FILE}: {e}");
                return LeftOut::default();
            }
        };
        let (record, crash) = StartRecord::begin(previous, env!("CARGO_PKG_VERSION"), preference_name(preference));
        let left_out = record.left_out(crash.clone(), explicit);
        match &crash {
            Some(Crash::Adapter { key, name }) => {
                log::warn!("the last start ended on graphics adapter {name} ({key}) before the window's first frame reached the screen")
            }
            Some(Crash::Listing) => log::warn!("the last start ended while it listed the graphics adapters"),
            None => {}
        }
        if explicit && (!record.crashed.is_empty() || record.listing_crashed) {
            log::info!("WGPU_BACKEND or WGPU_ADAPTER_NAME is set: no adapter is left out for the starts that crashed");
        }
        if !left_out.adapters.is_empty() {
            log::warn!("leaving out the graphics adapters earlier starts crashed on: {}", left_out.adapters.join(", "));
        }
        if left_out.opengl_only {
            log::warn!("an earlier start crashed while it listed the graphics adapters: listing OpenGL's alone");
        }
        let mut file = StartFile { file, record };
        file.write();
        *self.start.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
        left_out
    }

    /// [`selector`] picked the adapter with this [`key`] and name; `native` when the listing found
    /// an adapter of a backend other than OpenGL.
    fn picked(&self, key: &str, name: &str, native: bool) {
        // Set once; eframe picks the adapter once.
        let _ = self.adapter.set(key.to_string());
        self.update(|r| r.picked(key, name, native), false);
    }

    /// [`selector`] found no adapter that can show the window.
    fn none_left(&self) {
        self.update(StartRecord::none_left, false);
    }

    /// The window's first frame reached the screen, or the window started minimized.
    fn presented(&self) {
        self.update(StartRecord::presented, true);
    }

    /// The app ends, or starts again, without crashing.
    fn ended(&self) {
        self.update(StartRecord::ended, true);
    }

    /// Change the record and write it; `last` releases the file for the next start.
    fn update(&self, change: impl FnOnce(&mut StartRecord), last: bool) {
        let mut start = self.start.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(file) = start.as_mut() {
            change(&mut file.record);
            file.write();
        }
        if last {
            *start = None;
        }
    }
}

/// What [`watch_first_frame`] makes of the window's start.
#[derive(Debug, PartialEq, Eq)]
enum FirstFrame {
    /// Not due yet.
    Wait,
    /// On the screen (the second frame began, so the first was presented), or the window was
    /// minimized at the start: nothing to watch.
    Shown,
    /// Overdue.
    Hung,
}

/// The window's start after `frames` frames run, `since` its first frame was due.
fn first_frame(frames: u64, since: Duration, minimized: bool) -> FirstFrame {
    if frames >= 2 || minimized {
        FirstFrame::Shown
    } else if since >= FIRST_FRAME_TIMEOUT {
        FirstFrame::Hung
    } else {
        FirstFrame::Wait
    }
}

/// Watch the window's first frame from another thread: once it reaches the screen, clear the start
/// from [`START_FILE`]; when it hasn't reached the screen [`FIRST_FRAME_TIMEOUT`] after the app was
/// created ([`Startup::ready`]), start the app again without the adapter, as [`finish`] does for
/// one that failed, and end this process, whose UI thread is stuck in the graphics driver.
pub fn watch_first_frame(startup: Arc<Startup>) {
    let watch = move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let Some(due) = startup.ready.get() else { continue };
        let ui = startup.ui.get();
        let frames = ui.map_or(0, egui::Context::cumulative_frame_nr);
        let minimized = ui.and_then(|c| c.input(|i| i.viewport().minimized)).unwrap_or(false);
        match first_frame(frames, due.elapsed(), minimized) {
            FirstFrame::Wait => continue,
            FirstFrame::Shown => {
                startup.presented();
                return;
            }
            FirstFrame::Hung => {}
        }
        let mut skip = skipped();
        let Some(failed) = restart_without(true, frames, startup.adapter.get().map(String::as_str), &skip) else { return };
        log::error!(
            "the window's first frame didn't reach the screen in {}s on graphics adapter {failed}: starting again without it",
            FIRST_FRAME_TIMEOUT.as_secs()
        );
        // The new app records its own start.
        startup.ended();
        skip.push(failed.to_string());
        match restart(&skip.join(",")) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                log::error!("starting again failed: {e}");
                return;
            }
        }
    };
    if let Err(e) = std::thread::Builder::new().name("first frame".into()).spawn(watch) {
        log::warn!("no watch on the window's first frame: {e}");
    }
}

/// eframe's adapter choice: the first of [`adapter_order`] for `power`, the GPU the `compositor`
/// draws with, the GPUs that drive a `displays`, `WGPU_ADAPTER_NAME`, the adapters a restart
/// left out and those `crashed` ([`LeftOut::adapters`]). Every adapter, the displays and the
/// choice are logged: which GPU draws is the first question in every black-window report.
pub fn selector(
    power: PowerPreference,
    displays: Vec<DisplayGpu>,
    compositor: Option<(u32, u32)>,
    crashed: Vec<String>,
    startup: Arc<Startup>,
) -> NativeAdapterSelectorMethod {
    let named = std::env::var(NAME_ENV).ok();
    let skip = skipped();
    let left_out: Vec<String> = skip.iter().chain(&crashed).cloned().collect();
    Arc::new(move |adapters, surface| {
        let candidates: Vec<Candidate> = adapters.iter().map(|a| Candidate::new(a, surface)).collect();
        let order = adapter_order(&candidates, power, &displays, compositor, named.as_deref(), &left_out);
        let listed: Vec<String> = candidates
            .iter()
            .map(|c| {
                let (drives_display, drives_primary) = c.drives(&displays);
                let note = if !c.presents {
                    ", can't show the window"
                } else if skip.contains(&c.key) {
                    ", failed before"
                } else if crashed.contains(&c.key) {
                    ", crashed an earlier start"
                } else if c.composites(compositor) {
                    ", the compositor draws with it"
                } else if drives_primary {
                    ", drives the primary display"
                } else if drives_display {
                    ", drives a display"
                } else {
                    ""
                };
                format!("{} ({:?}, {:?}, {}{note})", c.name.trim(), c.backend, c.device_type, c.key)
            })
            .collect();
        log::info!("graphics adapters: {}", listed.join("; "));
        let first = order.first().and_then(|&i| adapters.get(i).zip(candidates.get(i)));
        let Some((adapter, chosen)) = first else {
            startup.none_left();
            return Err(format!("no graphics adapter can show the window (power preference {power:?}, {} adapters)", adapters.len()));
        };
        let why = match chosen.drives(&displays) {
            _ if chosen.composites(compositor) => "the compositor draws with it".to_string(),
            (_, true) => "it drives the primary display".to_string(),
            (true, false) => "it drives a display".to_string(),
            (false, false) if displays.is_empty() => format!("power preference {power:?}"),
            (false, false) => format!("no adapter matches a display GPU, so power preference {power:?}"),
        };
        log::info!("drawing on {} ({:?}, {:?}): {why}", chosen.name.trim(), chosen.backend, chosen.device_type);
        startup.picked(&chosen.key, &chosen.name, candidates.iter().any(|c| c.backend != Backend::Gl));
        Ok(adapter.clone())
    })
}

/// The last panic came from the graphics stack (see [`watch_panics`]).
static GRAPHICS_PANIC: AtomicBool = AtomicBool::new(false);

/// Note whether each panic comes from the graphics stack, so [`finish`] can tell a failing adapter
/// from a bug. The default hook still prints it.
pub fn watch_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        GRAPHICS_PANIC.store(info.location().is_some_and(|l| in_graphics_stack(l.file())), Ordering::Relaxed);
        default(info);
    }));
}

/// A source file of wgpu (`wgpu-core-30.0.1/src/…`, `wgpu-hal-…`), egui-wgpu or naga.
fn in_graphics_stack(file: &str) -> bool {
    file.split(['/', '\\']).any(|dir| ["wgpu-", "egui-wgpu-", "naga-"].iter().any(|p| dir.starts_with(p)))
}

/// The adapter to start again without: the one picked, when the graphics failed while the window
/// was starting up and it wasn't already left out.
fn restart_without<'a>(graphics_failed: bool, frames: u64, picked: Option<&'a str>, skip: &[String]) -> Option<&'a str> {
    picked.filter(|p| graphics_failed && frames < STARTUP_FRAMES && !skip.iter().any(|s| s == p))
}

/// After the window's run: `Ok` when it closed normally or the app started again (see the module
/// docs), otherwise why it failed. `outcome` is eframe's result, or the message of a panic that
/// ended it.
pub fn finish(outcome: Result<eframe::Result, String>, startup: &Startup) -> Result<(), String> {
    // Whatever follows, this start didn't crash; a new app records its own start.
    startup.ended();
    let (why, graphics_failed) = match outcome {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e @ eframe::Error::Wgpu(_))) => (e.to_string(), true),
        Ok(Err(e)) => (e.to_string(), false),
        Err(panic) => (panic, GRAPHICS_PANIC.load(Ordering::Relaxed)),
    };
    let frames = startup.ui.get().map_or(0, egui::Context::cumulative_frame_nr);
    let mut skip = skipped();
    let Some(failed) = restart_without(graphics_failed, frames, startup.adapter.get().map(String::as_str), &skip) else {
        return Err(if graphics_failed { format!("{why} ({HELP})") } else { why });
    };
    log::error!("the window failed on graphics adapter {failed} ({why}): starting again without it");
    skip.push(failed.to_string());
    restart(&skip.join(",")).map_err(|e| format!("{why}; starting again failed: {e}"))
}

/// Start the app again with the same arguments, leaving out the adapters in `skip`. On Unix the
/// new app replaces this process (same process id, so an AppImage keeps its files mounted), and
/// this returns only on failure; on Windows it starts beside this one, which then exits.
fn restart(skip: &str) -> std::io::Result<()> {
    log::logger().flush();
    let mut c = std::process::Command::new(std::env::current_exe()?);
    c.args(std::env::args_os().skip(1)).env(SKIP_ENV, skip);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        Err(c.exec())
    }
    #[cfg(not(unix))]
    c.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, backend: Backend, device_type: DeviceType) -> Candidate {
        Candidate { key: format!("{backend:?}:{name}"), name: name.into(), backend, device_type, pci: (0, 0), presents: true }
    }

    /// #1459: a 9048 px wide window (two display scales' ratio past 8192) fits an adapter that
    /// takes 16384 px textures; an adapter with less than eframe asks for changes nothing.
    #[test]
    fn the_surface_may_be_as_large_as_the_adapter_allows() {
        assert_eq!(surface_limit(8192, 16384), 16384);
        assert_eq!(surface_limit(8192, 4096), 8192);
    }

    fn names(c: &[Candidate], order: &[usize]) -> Vec<String> {
        order.iter().map(|&i| c[i].name.clone()).collect()
    }

    const NVIDIA_3090: (u32, u32) = (0x10de, 0x2204);
    const AMD_IGPU: (u32, u32) = (0x1002, 0x164e);
    const INTEL_IGPU: (u32, u32) = (0x8086, 0xa780);
    const WARP: (u32, u32) = (0x1414, 0x008c);

    fn pci_gpu(name: &str, backend: Backend, device_type: DeviceType, pci: (u32, u32)) -> Candidate {
        Candidate { key: key(backend, pci.0, pci.1, name), name: name.into(), backend, device_type, pci, presents: true }
    }

    fn monitor(pci: (u32, u32)) -> DisplayGpu {
        DisplayGpu { pci, primary: false }
    }

    fn primary(pci: (u32, u32)) -> DisplayGpu {
        DisplayGpu { pci, primary: true }
    }

    /// The desktop of pdfcraft#378 as wgpu lists it on Windows: a Ryzen's integrated GPU without a
    /// monitor and an RTX 3090 driving both monitors, each under Vulkan and DX12, then WARP and
    /// OpenGL (which reports no device id).
    fn ryzen_desktop() -> Vec<Candidate> {
        vec![
            pci_gpu("NVIDIA GeForce RTX 3090", Backend::Vulkan, DeviceType::DiscreteGpu, NVIDIA_3090),
            pci_gpu("AMD Radeon(TM) Graphics", Backend::Vulkan, DeviceType::IntegratedGpu, AMD_IGPU),
            pci_gpu("NVIDIA GeForce RTX 3090", Backend::Dx12, DeviceType::DiscreteGpu, NVIDIA_3090),
            pci_gpu("AMD Radeon(TM) Graphics", Backend::Dx12, DeviceType::IntegratedGpu, AMD_IGPU),
            pci_gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu, WARP),
            pci_gpu("NVIDIA GeForce RTX 3090/PCIe/SSE2", Backend::Gl, DeviceType::Other, (NVIDIA_3090.0, 0)),
        ]
    }

    /// pdfcraft#378: power saving picked the integrated GPU, whose driver reset when Windows had to
    /// present its frames on the monitors of the NVIDIA card. The GPU that drives the displays now
    /// comes first, through DX12 (#545); the integrated GPU stays the fallback a restart reaches.
    #[test]
    fn a_desktop_draws_on_the_gpu_that_drives_its_monitors() {
        let c = ryzen_desktop();
        for displays in [vec![primary(NVIDIA_3090)], vec![monitor(NVIDIA_3090)]] {
            assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), [2, 0, 3, 1, 5, 4], "{displays:?}");
            assert_eq!(adapter_order(&c, PowerPreference::None, &displays, None, None, &[]), [2, 0, 3, 1, 5, 4], "{displays:?}");
        }
        // Without the display information the order is power saving's, as before.
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]), [3, 1, 2, 0, 5, 4]);
        // After the NVIDIA adapters failed, the integrated GPU is next.
        let skip = vec![c[2].key.clone(), c[0].key.clone()];
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[primary(NVIDIA_3090)], None, None, &skip), [3, 1, 5, 4]);
    }

    /// A hybrid laptop: the panel (primary) on the integrated GPU, an external monitor on the
    /// discrete one. Power saving and the display agree, in either enumeration order.
    #[test]
    fn a_hybrid_laptop_draws_on_the_integrated_gpu_that_drives_its_panel() {
        let c = ryzen_desktop();
        for displays in [vec![primary(AMD_IGPU), monitor(NVIDIA_3090)], vec![monitor(NVIDIA_3090), primary(AMD_IGPU)], vec![primary(AMD_IGPU)]] {
            assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), [3, 1, 2, 0, 5, 4], "{displays:?}");
        }
        // Linux names no primary unless there is a built-in panel: a monitor on each GPU keeps the
        // system's order among them (Mesa puts the compositor's GPU first), and a GPU without one
        // comes after both.
        let linux = vec![
            pci_gpu("AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)", Backend::Vulkan, DeviceType::IntegratedGpu, AMD_IGPU),
            pci_gpu("NVIDIA GeForce RTX 3090", Backend::Vulkan, DeviceType::DiscreteGpu, NVIDIA_3090),
            pci_gpu("Intel(R) Arc(TM) A380", Backend::Vulkan, DeviceType::DiscreteGpu, INTEL_IGPU),
            pci_gpu("llvmpipe (LLVM 20.1.8, 256 bits)", Backend::Vulkan, DeviceType::Cpu, (0x10005, 0)),
        ];
        let both = [monitor(AMD_IGPU), monitor(NVIDIA_3090)];
        assert_eq!(adapter_order(&linux, PowerPreference::None, &both, None, None, &[]), [0, 1, 2, 3]);
        assert_eq!(adapter_order(&linux, PowerPreference::None, &[monitor(INTEL_IGPU)], None, None, &[]), [2, 0, 1, 3]);
        assert_eq!(adapter_order(&linux, PowerPreference::None, &[monitor(NVIDIA_3090), primary(AMD_IGPU)], None, None, &[]), [0, 1, 2, 3]);
    }

    /// A hybrid laptop whose Wayland compositor runs on the discrete GPU (Hyprland with
    /// `AQ_DRM_DEVICES` naming it first), as Vulkan lists it there: the integrated GPU drives the
    /// built-in panel, yet the compositor shows nothing it draws, and reports no error. The GPU the
    /// compositor draws with comes first; without it, the panel's, as before.
    #[test]
    fn the_gpu_the_compositor_draws_with_comes_before_the_panels() {
        const RTX_3050: (u32, u32) = (0x10de, 0x25ac);
        const INTEL_RPL: (u32, u32) = (0x8086, 0xa7a8);
        let mut c = vec![
            pci_gpu("NVIDIA GeForce RTX 3050 6GB Laptop GPU", Backend::Vulkan, DeviceType::DiscreteGpu, RTX_3050),
            pci_gpu("Intel(R) Graphics (RPL-P)", Backend::Vulkan, DeviceType::IntegratedGpu, INTEL_RPL),
            pci_gpu("NVIDIA GeForce RTX 3050 6GB Laptop GPU/PCIe/SSE2", Backend::Gl, DeviceType::Other, (RTX_3050.0, 0)),
        ];
        c[2].presents = false;
        let panel = [primary(INTEL_RPL)];
        assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some(RTX_3050), None, &[]), [0, 1]);
        assert_eq!(adapter_order(&c, PowerPreference::None, &panel, None, None, &[]), [1, 0]);
        // The usual hybrid laptop, with the compositor on the integrated GPU: the panel's GPU.
        assert_eq!(adapter_order(&c, PowerPreference::None, &[primary(INTEL_RPL), monitor(RTX_3050)], Some(INTEL_RPL), None, &[]), [1, 0]);
        // `WGPU_ADAPTER_NAME` still comes first, and a restart after a failure still moves on.
        assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some(RTX_3050), Some("intel"), &[]), [1, 0]);
        assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some(RTX_3050), None, &[c[0].key.clone()]), [1]);
        // A compositor GPU no adapter matches (a virtual one) leaves the displays in charge.
        assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some((0x1af4, 0x1050)), None, &[]), [1, 0]);
    }

    /// A Windows desktop with a monitor on each GPU: the window opens on the primary display, so
    /// its GPU draws, whichever kind it is.
    #[test]
    fn the_primary_display_decides_between_gpus_that_both_drive_a_monitor() {
        let c = ryzen_desktop();
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[monitor(AMD_IGPU), primary(NVIDIA_3090)], None, None, &[]), [2, 0, 3, 1, 5, 4]);
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[primary(AMD_IGPU), monitor(NVIDIA_3090)], None, None, &[]), [3, 1, 2, 0, 5, 4]);
        // The displays only matter among adapters that can show the window; a GPU driving no
        // display still loses to one that does.
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[monitor(AMD_IGPU), monitor(INTEL_IGPU)], None, None, &[]), [3, 1, 2, 0, 5, 4]);
    }

    /// Display GPUs none of the adapters match (another driver's ids, OpenGL's missing device id,
    /// Metal's zeros) change nothing: power saving's order, the old behaviour.
    #[test]
    fn displays_no_adapter_matches_leave_the_power_preference_in_charge() {
        let c = ryzen_desktop();
        let low = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]);
        for displays in [vec![monitor(INTEL_IGPU)], vec![primary((0x8086, 0x1234)), monitor((0x8086, 0x5678))], vec![primary((0, 0))]] {
            assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), low, "{displays:?}");
        }
        assert_eq!(
            names(&c, &adapter_order(&c, PowerPreference::HighPerformance, &[monitor(INTEL_IGPU)], None, None, &[]))[0],
            "NVIDIA GeForce RTX 3090"
        );
        // The #502 machine's adapters report no ids in these tests: the primary display never matches.
        let c = issue_502();
        assert_eq!(
            adapter_order(&c, PowerPreference::None, &[primary(NVIDIA_3090)], None, None, &[]),
            adapter_order(&c, PowerPreference::None, &[], None, None, &[])
        );
        assert!(adapter_order(&[], PowerPreference::LowPower, &[primary(NVIDIA_3090)], None, None, &[]).is_empty());
    }

    /// `WGPU_ADAPTER_NAME` and "can't present" still come before the displays.
    #[test]
    fn a_named_adapter_comes_before_the_one_driving_the_display() {
        let mut c = ryzen_desktop();
        let displays = [primary(NVIDIA_3090)];
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, Some("radeon"), &[]), [3, 1, 2, 0, 5, 4]);
        c[2].presents = false;
        c[0].presents = false;
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), [3, 1, 5, 4]);
    }

    /// A user's own choice (Preferences or `WGPU_POWER_PREF`) is kept as it is; only Automatic
    /// looks at the displays.
    #[test]
    fn only_automatic_considers_which_gpu_drives_the_display() {
        assert!(automatic(None, None));
        assert!(automatic(Some("automatic"), None));
        assert!(automatic(Some("powerSaving"), None));
        assert!(!automatic(Some("lowPower"), None));
        assert!(!automatic(Some("highPerformance"), None));
        assert!(!automatic(None, Some(PowerPreference::None)));
        assert!(!automatic(Some("automatic"), Some(PowerPreference::LowPower)));
        assert!(preferred_displays(Some("lowPower"), None).is_empty());
        assert!(preferred_displays(None, Some(PowerPreference::HighPerformance)).is_empty());
        // Automatic reports whatever this machine has (nothing on macOS, in CI, or under Remote Desktop).
        assert_eq!(preferred_displays(None, None), display_gpus());
        assert_eq!(preferred_compositor(Some("lowPower"), None), None);
        assert_eq!(preferred_compositor(None, Some(PowerPreference::HighPerformance)), None);
        assert_eq!(preferred_compositor(None, None), crate::compositor_gpu::compositor_gpu());
    }

    #[test]
    fn pci_ids_come_from_windows_device_ids() {
        assert_eq!(pci_ids("PCI\\VEN_10DE&DEV_2204&SUBSYS_40421458&REV_A1"), Some(NVIDIA_3090));
        assert_eq!(pci_ids("PCI\\VEN_1002&DEV_164E&SUBSYS_88771043&REV_C1"), Some(AMD_IGPU));
        assert_eq!(pci_ids("pci\\ven_1002&dev_164e"), Some(AMD_IGPU));
        // Remote Desktop and other non-PCI display devices, and malformed ids.
        assert_eq!(pci_ids("ROOT\\BasicDisplay\\0000"), None);
        assert_eq!(pci_ids("PCI\\VEN_10DE&SUBSYS_40421458"), None);
        assert_eq!(pci_ids("PCI\\VEN_10DE&DEV_ZZZZ"), None);
        assert_eq!(pci_ids("VEN_&DEV_"), None);
        assert_eq!(pci_ids(""), None);
    }

    /// Whatever this machine has: every GPU listed is a PCI device, at most one drives the primary
    /// display, and none is listed twice. (A headless CI runner may list none.)
    #[cfg(windows)]
    #[test]
    fn windows_lists_each_gpu_with_a_monitor_once() {
        let gpus = windows_display_gpus();
        assert!(gpus.iter().filter(|g| g.primary).count() <= 1, "{gpus:?}");
        for (i, g) in gpus.iter().enumerate() {
            assert_ne!(g.pci.0, 0, "{gpus:?}");
            assert!(!gpus[..i].iter().any(|h| h.pci == g.pci), "{gpus:?}");
        }
    }

    #[test]
    fn internal_panels_are_edp_lvds_and_dsi_connectors() {
        for name in ["eDP-1", "LVDS-1", "DSI-1"] {
            assert!(is_internal_panel(name), "{name}");
        }
        for name in ["DP-3", "HDMI-A-1", "DVI-D-1", "VGA-1", "Writeback-1", ""] {
            assert!(!is_internal_panel(name), "{name}");
        }
    }

    /// A sysfs like a hybrid laptop's with an external monitor: the panel and a disconnected port
    /// on the integrated GPU (card0), the monitor on the discrete one (card1), and a card without
    /// PCI ids. Each GPU is listed once, the one with the panel as primary.
    #[test]
    fn linux_display_gpus_come_from_the_connected_connectors() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("vectorcraft-drm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let card = |name: &str, (vendor, device): (u32, u32)| -> std::io::Result<()> {
            std::fs::create_dir_all(dir.join(name).join("device"))?;
            std::fs::write(dir.join(name).join("device/vendor"), format!("{vendor:#06x}\n"))?;
            std::fs::write(dir.join(name).join("device/device"), format!("{device:#06x}\n"))
        };
        let connector = |name: &str, status: &str| -> std::io::Result<()> {
            std::fs::create_dir_all(dir.join(name))?;
            std::fs::write(dir.join(name).join("status"), format!("{status}\n"))
        };
        card("card0", AMD_IGPU)?;
        card("card1", NVIDIA_3090)?;
        std::fs::create_dir_all(dir.join("card2"))?;
        connector("card0-eDP-1", "connected")?;
        connector("card0-HDMI-A-1", "disconnected")?;
        connector("card1-DP-1", "connected")?;
        connector("card1-DP-2", "connected")?;
        connector("card2-Virtual-1", "connected")?;
        std::fs::write(dir.join("renderD128"), "")?;
        let mut gpus = linux_display_gpus(&dir);
        gpus.sort_by_key(|g| g.pci);
        assert_eq!(gpus, [primary(AMD_IGPU), monitor(NVIDIA_3090)]);
        assert!(linux_display_gpus(&dir.join("no such dir")).is_empty());
        std::fs::remove_dir_all(&dir)
    }

    #[test]
    fn display_gpus_are_logged_as_pci_ids() {
        assert_eq!(primary(NVIDIA_3090).to_string(), "10de:2204 (primary)");
        assert_eq!(monitor(AMD_IGPU).to_string(), "1002:164e");
        let mut gpus = vec![];
        add_display_gpu(&mut gpus, NVIDIA_3090, false);
        add_display_gpu(&mut gpus, NVIDIA_3090, true);
        add_display_gpu(&mut gpus, AMD_IGPU, false);
        assert_eq!(gpus, [primary(NVIDIA_3090), monitor(AMD_IGPU)]);
    }

    /// The machine of #502 as Vulkan lists it with Mesa's device-select layer: the GPU KDE's
    /// compositor runs on (NVIDIA) first, then the Ryzen's integrated GPU, Mesa's software
    /// renderer, and OpenGL.
    fn issue_502() -> Vec<Candidate> {
        vec![
            gpu("NVIDIA GeForce RTX 5070", Backend::Vulkan, DeviceType::DiscreteGpu),
            gpu("AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)", Backend::Vulkan, DeviceType::IntegratedGpu),
            gpu("llvmpipe (LLVM 20.1.8, 256 bits)", Backend::Vulkan, DeviceType::Cpu),
            gpu("NVIDIA GeForce RTX 5070/PCIe/SSE2", Backend::Gl, DeviceType::Other),
        ]
    }

    #[test]
    fn automatic_keeps_the_systems_order_on_linux_and_saves_power_elsewhere() {
        let c = issue_502();
        let order = adapter_order(&c, power_preference(None, None), &[], None, None, &[]);
        let first = &c[order[0]].name;
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(first, "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)");
        } else {
            // #502: the integrated GPU couldn't show frames to a compositor on the NVIDIA GPU.
            assert_eq!(first, "NVIDIA GeForce RTX 5070");
        }
    }

    #[test]
    fn each_preference_orders_the_adapters_then_falls_back_to_opengl_and_software() {
        let c = issue_502();
        let order = |power| names(&c, &adapter_order(&c, power, &[], None, None, &[]));
        assert_eq!(
            order(PowerPreference::None),
            [
                "NVIDIA GeForce RTX 5070",
                "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
                "NVIDIA GeForce RTX 5070/PCIe/SSE2",
                "llvmpipe (LLVM 20.1.8, 256 bits)"
            ]
        );
        assert_eq!(
            order(PowerPreference::LowPower),
            [
                "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
                "NVIDIA GeForce RTX 5070",
                "NVIDIA GeForce RTX 5070/PCIe/SSE2",
                "llvmpipe (LLVM 20.1.8, 256 bits)"
            ]
        );
        assert_eq!(
            order(PowerPreference::HighPerformance),
            [
                "NVIDIA GeForce RTX 5070",
                "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
                "NVIDIA GeForce RTX 5070/PCIe/SSE2",
                "llvmpipe (LLVM 20.1.8, 256 bits)"
            ]
        );
    }

    /// A restart leaves out the adapter the window failed on, so Power Saving on the machine of
    /// #502 ends up on the NVIDIA GPU, and once every adapter failed there is nothing to try.
    /// A dual-GPU Mac reports both GPUs under Metal with no PCI ids (#651): their keys still
    /// differ, so when the integrated GPU fails, the restart renders on the discrete one.
    #[test]
    fn gpus_without_pci_ids_are_told_apart_by_name() {
        let mac = |name: &str, device_type| Candidate {
            key: key(Backend::Metal, 0, 0, name),
            name: name.into(),
            backend: Backend::Metal,
            device_type,
            pci: (0, 0),
            presents: true,
        };
        let c = vec![mac("NVIDIA GeForce GT 750M", DeviceType::DiscreteGpu), mac("Intel Iris Pro Graphics", DeviceType::IntegratedGpu)];
        assert_eq!((c[0].key.as_str(), c[1].key.as_str()), ("Metal:NVIDIA GeForce GT 750M", "Metal:Intel Iris Pro Graphics"));
        let first = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]);
        assert_eq!(names(&c, &first)[0], "Intel Iris Pro Graphics");
        let skip = parse_skip(&c[first[0]].key);
        assert_eq!(names(&c, &adapter_order(&c, PowerPreference::LowPower, &[], None, None, &skip)), ["NVIDIA GeForce GT 750M"]);
        // A comma in a name can't split the list.
        assert_eq!(parse_skip(&key(Backend::Gl, 0, 0, "Mesa, llvmpipe")), ["Gl:Mesa  llvmpipe"]);
    }

    #[test]
    fn a_first_frame_that_never_reaches_the_screen_hangs_its_adapter() {
        let s = Duration::from_secs;
        // The second frame began: the first was presented.
        assert_eq!(first_frame(2, s(60), false), FirstFrame::Shown);
        // Still drawing (or presenting) the first frame: waited for, then given up on (#964).
        for frames in [0, 1] {
            assert_eq!(first_frame(frames, s(3), false), FirstFrame::Wait);
            assert_eq!(first_frame(frames, FIRST_FRAME_TIMEOUT, false), FirstFrame::Hung);
        }
        // A window minimized at the start may not draw again until restored.
        assert_eq!(first_frame(1, s(60), true), FirstFrame::Shown);
        // A hung adapter is left out once, like one that failed.
        let skip = vec!["Dx12:10de:28a0".to_string()];
        assert_eq!(restart_without(true, 1, Some("Dx12:10de:28a0"), &skip), None);
        assert_eq!(restart_without(true, 1, Some("Dx12:8086:a7a8"), &skip), Some("Dx12:8086:a7a8"));
    }

    #[test]
    fn a_restart_tries_the_next_adapter() {
        let c = issue_502();
        let mut skip = vec![];
        let mut tried = vec![];
        while let Some(&i) = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &skip).first() {
            tried.push(c[i].name.clone());
            skip.push(c[i].key.clone());
        }
        assert_eq!(
            tried,
            [
                "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
                "NVIDIA GeForce RTX 5070",
                "NVIDIA GeForce RTX 5070/PCIe/SSE2",
                "llvmpipe (LLVM 20.1.8, 256 bits)"
            ]
        );
    }

    #[test]
    fn adapters_that_cannot_present_to_the_window_are_never_tried() {
        let mut c = issue_502();
        c[0].presents = false;
        c[3].presents = false;
        for power in [PowerPreference::None, PowerPreference::LowPower, PowerPreference::HighPerformance] {
            let order = adapter_order(&c, power, &[], None, None, &[]);
            assert!(!order.contains(&0) && !order.contains(&3), "{power:?}: {order:?}");
            assert_eq!(order.len(), 2);
        }
        for p in &mut c {
            p.presents = false;
        }
        assert!(adapter_order(&c, PowerPreference::None, &[], None, None, &[]).is_empty());
        assert!(adapter_order(&[], PowerPreference::HighPerformance, &[], None, None, &[]).is_empty());
    }

    #[test]
    fn wgpu_adapter_name_picks_an_adapter_first_and_the_rest_stay_as_fallbacks() {
        let c = issue_502();
        let order = adapter_order(&c, PowerPreference::HighPerformance, &[], None, Some("radv"), &[]);
        assert_eq!(c[order[0]].name, "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)");
        assert_eq!(order.len(), 4);
        // Any case; every match comes before the rest, natively first.
        let order = adapter_order(&c, PowerPreference::LowPower, &[], None, Some("GeForce"), &[]);
        assert_eq!(names(&c, &order[..2]), ["NVIDIA GeForce RTX 5070", "NVIDIA GeForce RTX 5070/PCIe/SSE2"]);
        // An empty name or one nothing matches changes nothing.
        for named in ["", "no such gpu"] {
            assert_eq!(
                adapter_order(&c, PowerPreference::LowPower, &[], None, Some(named), &[]),
                adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[])
            );
        }
        // A named adapter that failed is left out like any other.
        let skip = vec![c[1].key.clone()];
        assert_eq!(adapter_order(&c, PowerPreference::None, &[], None, Some("radv"), &skip)[0], 0);
    }

    /// A Windows machine lists every GPU twice (Vulkan and DX12): power saving still takes the
    /// integrated GPU first, as wgpu's own choice did (#306), each GPU through DX12 first (#545).
    #[test]
    fn hybrid_laptop_on_windows_renders_on_the_integrated_gpu_with_power_saving() {
        let c = vec![
            gpu("Intel(R) Arc(TM) A370M", Backend::Vulkan, DeviceType::DiscreteGpu),
            gpu("Intel(R) Iris(R) Xe Graphics", Backend::Vulkan, DeviceType::IntegratedGpu),
            gpu("Intel(R) Arc(TM) A370M", Backend::Dx12, DeviceType::DiscreteGpu),
            gpu("Intel(R) Iris(R) Xe Graphics", Backend::Dx12, DeviceType::IntegratedGpu),
            gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu),
        ];
        let order = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]);
        assert_eq!(order, [3, 1, 2, 0, 4]);
        assert_eq!(adapter_order(&c, PowerPreference::HighPerformance, &[], None, None, &[]), [2, 0, 3, 1, 4]);
    }

    /// The machine of #545: one Intel GPU, whose Vulkan driver made the window flicker black.
    /// DX12 comes first whatever the preference; Vulkan stays a fallback, and `WGPU_ADAPTER_NAME`
    /// or a restart leaving DX12 out still reach it.
    #[test]
    fn a_windows_gpu_renders_through_dx12_before_vulkan() {
        let c = vec![
            gpu("Intel(R) Graphics", Backend::Vulkan, DeviceType::IntegratedGpu),
            gpu("Intel(R) Graphics", Backend::Dx12, DeviceType::IntegratedGpu),
            gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu),
            gpu("Intel(R) Graphics", Backend::Gl, DeviceType::Other),
        ];
        for power in [PowerPreference::LowPower, PowerPreference::HighPerformance, PowerPreference::None] {
            assert_eq!(adapter_order(&c, power, &[], None, None, &[]), [1, 0, 3, 2], "{power:?}");
        }
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[c[1].key.clone()]), [0, 3, 2], "DX12 failed: Vulkan next");
    }

    #[test]
    fn windows_never_loads_vulkan_drivers_unless_asked() {
        let all = Backends::all();
        let start = backends(all, false);
        if cfg!(windows) {
            assert_eq!(start, Backends::DX12 | Backends::GL, "#806");
        } else {
            assert_eq!(start, all);
        }
        assert_eq!(backends(Backends::VULKAN, true), Backends::VULKAN, "WGPU_BACKEND=vulkan still picks it");
    }

    #[test]
    fn power_preference_follows_the_environment_then_the_preference() {
        assert_eq!(power_preference(None, None), AUTOMATIC);
        assert_eq!(power_preference(Some("automatic"), None), AUTOMATIC);
        assert_eq!(power_preference(Some("lowPower"), None), PowerPreference::LowPower);
        assert_eq!(power_preference(Some("highPerformance"), None), PowerPreference::HighPerformance);
        // 0.5.0's default, and values this version doesn't know (a newer or damaged file).
        for old in ["powerSaving", "turbo", ""] {
            assert_eq!(power_preference(Some(old), None), AUTOMATIC, "{old}");
        }
        // WGPU_POWER_PREF wins over the preference, either way.
        assert_eq!(power_preference(Some("lowPower"), Some(PowerPreference::HighPerformance)), PowerPreference::HighPerformance);
        assert_eq!(power_preference(Some("highPerformance"), Some(PowerPreference::LowPower)), PowerPreference::LowPower);
        assert_eq!(power_preference(Some("lowPower"), Some(PowerPreference::None)), PowerPreference::None);
        assert_eq!(AUTOMATIC, if cfg!(any(windows, target_os = "macos")) { PowerPreference::LowPower } else { PowerPreference::None });
    }

    /// The preference the engine saves is the one the app reads back before the window opens.
    #[test]
    fn the_saved_engine_preference_is_found() {
        let mut prefs = vectorcraft_engine::Prefs::default();
        let saved = prefs.to_json();
        assert_eq!(saved.get("gpuPreference").and_then(serde_json::Value::as_str), Some("automatic"));
        for (value, power) in [("lowPower", PowerPreference::LowPower), ("highPerformance", PowerPreference::HighPerformance)] {
            prefs.gpu_preference = value.into();
            let saved = prefs.to_json();
            assert_eq!(power_preference(saved.get("gpuPreference").and_then(serde_json::Value::as_str), None), power);
        }
        // Every choice Preferences offer means something here.
        for (value, _) in vectorcraft_engine::cmd::prefscmds::GPU_PREFERENCES {
            assert!(["automatic", "lowPower", "highPerformance"].contains(value), "{value}");
        }
    }

    #[test]
    fn skipped_adapters_parse_from_a_comma_separated_list() {
        assert_eq!(parse_skip("Vulkan:1002:164e, Gl:10de:2f04,,"), ["Vulkan:1002:164e", "Gl:10de:2f04"]);
        assert!(parse_skip("").is_empty());
        assert_eq!(key(Backend::Vulkan, 0x1002, 0x164e, "AMD Radeon"), "Vulkan:1002:164e");
        assert_eq!(key(Backend::Gl, 0x10de, 0x2f04, "NVIDIA"), "Gl:10de:2f04");
    }

    #[test]
    fn panics_in_wgpu_egui_wgpu_and_naga_are_graphics_failures() {
        for file in [
            "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-wgpu-0.36.2/src/winit.rs",
            r"C:\Users\u\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\wgpu-core-30.0.1\src\device\mod.rs",
            "/rustc/deps/wgpu-hal-30.0.1/src/vulkan/adapter.rs",
            "naga-30.0.0/src/back/spv/writer.rs",
        ] {
            assert!(in_graphics_stack(file), "{file}");
        }
        for file in [
            "apps/vectorcraft/src/main.rs",
            "/home/wgpu/vectorcraft/crates/ui-egui/src/lib.rs",
            "egui-0.36.2/src/context.rs",
            "winit-0.30.12/src/x.rs",
        ] {
            assert!(!in_graphics_stack(file), "{file}");
        }
    }

    #[test]
    fn only_a_graphics_failure_while_starting_up_restarts_without_the_adapter() {
        let amd = Some("Vulkan:1002:164e");
        assert_eq!(restart_without(true, 0, amd, &[]), amd);
        assert_eq!(restart_without(true, STARTUP_FRAMES - 1, amd, &[]), amd);
        // After start-up the adapter has shown frames; a lost window is something else.
        assert_eq!(restart_without(true, STARTUP_FRAMES, amd, &[]), None);
        // A bug outside the graphics, or no adapter picked: nothing another adapter would change.
        assert_eq!(restart_without(false, 0, amd, &[]), None);
        assert_eq!(restart_without(true, 0, None, &[]), None);
        // Never the same adapter twice, so restarts end.
        assert_eq!(restart_without(true, 0, amd, &["Vulkan:1002:164e".into()]), None);
    }

    #[test]
    fn a_normal_exit_or_another_failure_does_not_restart() {
        let startup = Startup::default();
        assert_eq!(finish(Ok(Ok(())), &startup), Ok(()));
        // No adapter was picked (none can show the window): the error is returned, nothing restarts.
        let none =
            eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::CustomNativeAdapterSelectionError("no graphics adapter can show the window".into()));
        assert_eq!(
            finish(Ok(Err(none)), &startup),
            Err(format!("WGPU error: Adapter selection failed: no graphics adapter can show the window ({HELP})"))
        );
        let _ = startup.adapter.set("Vulkan:1002:164e".into());
        GRAPHICS_PANIC.store(false, Ordering::Relaxed);
        assert_eq!(finish(Err("a bug".into()), &startup), Err("a bug".into()));
    }

    /// A log folder of its own for each test.
    fn start_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("vectorcraft-gpu-start-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// What a start in `dir` leaves out; it then ends without crashing.
    fn next_start(dir: &std::path::Path, explicit: bool) -> LeftOut {
        let s = Startup::default();
        let left_out = s.track(dir, None, explicit);
        s.ended();
        left_out
    }

    /// The machine of #2243: one Intel UHD Graphics 605, listed through DX12 and OpenGL, and
    /// Windows' software adapter.
    fn issue_2243() -> Vec<Candidate> {
        vec![
            pci_gpu("Intel(R) UHD Graphics 605", Backend::Dx12, DeviceType::IntegratedGpu, (0x8086, 0x3184)),
            pci_gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu, (0x1414, 0x008c)),
            pci_gpu("Intel(R) UHD Graphics 605", Backend::Gl, DeviceType::Other, (0x8086, 0)),
        ]
    }

    /// #2243: the driver ends the process after the adapter was picked, before the first frame.
    /// The next start leaves that adapter out and draws through OpenGL, and so do the starts after
    /// it, without a status-bar note.
    #[test]
    fn a_start_that_never_showed_a_frame_leaves_its_adapter_out_of_the_next() {
        let dir = start_dir("crash");
        let c = issue_2243();
        let first = Startup::default();
        assert_eq!(first.track(&dir, None, false), LeftOut::default());
        first.picked(&c[0].key, &c[0].name, true);
        // The process ends here: its file is closed and unlocked, and the start is still recorded.
        drop(first);

        let second = Startup::default();
        let left_out = second.track(&dir, Some("automatic"), false);
        assert_eq!(left_out.adapters, ["Dx12:8086:3184"]);
        assert_eq!(left_out.crash, Some(Crash::Adapter { key: "Dx12:8086:3184".into(), name: "Intel(R) UHD Graphics 605".into() }));
        assert!(!left_out.opengl_only);
        assert_eq!(left_out.backends(Backends::DX12 | Backends::GL), Backends::DX12 | Backends::GL);
        let order = adapter_order(&c, AUTOMATIC, &[], None, None, &left_out.adapters);
        assert_eq!(order, [2, 1], "OpenGL on the same GPU before the software adapter");
        second.picked(&c[2].key, &c[2].name, true);
        second.presented();

        let third = Startup::default();
        let left_out = third.track(&dir, None, false);
        assert_eq!(left_out.adapters, ["Dx12:8086:3184"], "kept");
        assert_eq!(left_out.crash, None, "the note is shown once");
        third.ended();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A start whose first frame reached the screen, or that ended without crashing (a normal exit,
    /// an error, a restart on another adapter), leaves nothing out of the next.
    #[test]
    fn a_start_that_showed_its_first_frame_or_ended_is_cleared() {
        let dir = start_dir("shown");
        let c = issue_2243();
        for end in [Startup::presented as fn(&Startup), Startup::ended] {
            let first = Startup::default();
            first.track(&dir, None, false);
            first.picked(&c[0].key, &c[0].name, true);
            end(&first);
            drop(first);
            assert_eq!(next_start(&dir, false), LeftOut::default());
        }
        // So does a start that ended before it picked an adapter.
        let first = Startup::default();
        first.track(&dir, None, false);
        assert!(finish(Ok(Ok(())), &first).is_ok());
        drop(first);
        assert_eq!(next_start(&dir, false), LeftOut::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The process ended while the adapters were being listed (wgpu's DX12 backend creates a
    /// device on each GPU to list it): the next starts list OpenGL's adapters alone, except on
    /// macOS, where wgpu has none.
    #[test]
    fn a_crash_while_listing_lists_opengl_alone() {
        let dir = start_dir("listing");
        let first = Startup::default();
        first.track(&dir, None, false);
        drop(first);
        let second = Startup::default();
        let left_out = second.track(&dir, None, false);
        assert!(left_out.adapters.is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(left_out, LeftOut::default());
        } else {
            assert_eq!(left_out.crash, Some(Crash::Listing));
            assert_eq!(left_out.backends(Backends::DX12 | Backends::GL), Backends::GL);
            assert_eq!(left_out.backends(Backends::PRIMARY | Backends::GL), Backends::GL);
            assert_eq!(left_out.backends(Backends::METAL), Backends::METAL, "no OpenGL to fall back to");
        }
        // Listing OpenGL alone found nothing that can show the window: the next start lists every
        // backend again.
        second.none_left();
        second.ended();
        assert_eq!(next_start(&dir, false), LeftOut::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `WGPU_BACKEND` or `WGPU_ADAPTER_NAME` choose for a start whatever crashed before; when that
    /// start shows its first frame on the adapter that crashed, the adapter counts again.
    #[test]
    fn wgpu_backend_or_adapter_name_ignores_the_crashes() {
        let dir = start_dir("explicit");
        let c = issue_2243();
        let first = Startup::default();
        first.track(&dir, None, false);
        first.picked(&c[0].key, &c[0].name, true);
        drop(first);
        assert_eq!(next_start(&dir, true), LeftOut::default());
        assert_eq!(next_start(&dir, false).adapters, ["Dx12:8086:3184"], "still left out without them");

        let explicit = Startup::default();
        explicit.track(&dir, None, true);
        explicit.picked(&c[0].key, &c[0].name, true);
        explicit.presented();
        assert_eq!(next_start(&dir, false), LeftOut::default(), "a new driver shows the window");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A damaged record, a file that can't be opened and one another starting copy of the app holds
    /// are ignored: the start goes on with nothing left out.
    #[test]
    fn a_damaged_or_unreadable_start_file_is_ignored() {
        let dir = start_dir("damaged");
        std::fs::create_dir_all(&dir).unwrap();
        let damaged: [&[u8]; 9] = [
            b"",
            b"{",
            b"[]",
            b"null",
            b"{\"version\":1}",
            b"{\"starting\":{}}",
            b"\xef\xbb\xbf\0",
            b"\xff\xfe{",
            &[b' '; START_FILE_LIMIT as usize + 1],
        ];
        for text in damaged {
            std::fs::write(dir.join(START_FILE), text).unwrap();
            assert_eq!(next_start(&dir, false), LeftOut::default(), "{:?}", String::from_utf8_lossy(&text[..text.len().min(20)]));
        }

        // Another copy of the app is starting: this start neither reads nor writes the file.
        let c = issue_2243();
        let starting = Startup::default();
        starting.track(&dir, None, false);
        starting.picked(&c[0].key, &c[0].name, true);
        assert_eq!(next_start(&dir, false), LeftOut::default());
        drop(starting);
        assert_eq!(next_start(&dir, false).adapters, ["Dx12:8086:3184"]);

        // A folder in the file's place.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(START_FILE)).unwrap();
        assert_eq!(next_start(&dir, false), LeftOut::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crashes_are_forgotten_in_another_version_or_under_another_preference() {
        let starting = |key: &str| Starting { adapter: Some(Recorded { key: key.into(), name: "GPU".into() }), native: true };
        let (mut record, crash) = StartRecord::begin(None, "0.9.0", "automatic");
        assert_eq!(crash, None);
        record.starting = Some(starting("Dx12:8086:3184"));
        let (record, crash) = StartRecord::begin(Some(record), "0.9.0", "automatic");
        assert_eq!(crash, Some(Crash::Adapter { key: "Dx12:8086:3184".into(), name: "GPU".into() }));
        assert_eq!(record.left_out(None, false).adapters, ["Dx12:8086:3184"]);
        // Written and read back.
        assert_eq!(StartRecord::parse(&record.to_json()), Some(record.clone()));
        assert!(StartRecord::begin(Some(record.clone()), "0.9.1", "automatic").0.crashed.is_empty());
        assert!(StartRecord::begin(Some(record.clone()), "0.9.0", "highPerformance").0.crashed.is_empty());
        assert_eq!(preference_name(None), "automatic");
        assert_eq!(preference_name(Some("powerSaving")), "automatic");
        assert_eq!(preference_name(Some("lowPower")), "lowPower");
        // An adapter is listed once, and the list is bounded.
        let mut record = record;
        for n in 0..CRASHED_LIMIT + 4 {
            for _ in 0..2 {
                record.starting = Some(starting(&format!("Dx12:10de:{n:04x}")));
                record = StartRecord::begin(Some(record), "0.9.0", "automatic").0;
            }
        }
        assert_eq!(record.crashed.len(), CRASHED_LIMIT);
        assert_eq!(record.crashed.last().map(|c| c.key.as_str()), Some("Dx12:10de:0013"));
    }
}
