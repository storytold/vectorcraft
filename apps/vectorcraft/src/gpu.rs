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

/// The graphics backends the window starts with: on Windows Direct3D 12, with OpenGL beside it
/// only when `opengl` ([`windows_opengl`]), unless `WGPU_BACKEND` names others (`env_set`;
/// `default` then holds them). Creating a Vulkan instance loads every installed Vulkan driver into
/// the process, and a faulty one (Intel's `igvk64.dll`, #806) crashed the app before its window
/// appeared. DX12 is Windows' own backend, as in PdfCraft. Elsewhere, `default`.
pub fn backends(default: Backends, env_set: bool, opengl: bool) -> Backends {
    match (cfg!(windows) && !env_set, opengl) {
        (true, true) => Backends::DX12 | Backends::GL,
        (true, false) => Backends::DX12,
        (false, _) => default,
    }
}

/// Whether a Windows start lists OpenGL's adapters beside Direct3D 12's: only when it leaves an
/// adapter out (an earlier start crashed on it, or a restart failed on it), when
/// `WGPU_ADAPTER_NAME` names one (it may be an OpenGL adapter), or when DX12 lists no hardware
/// adapter (Windows 7, or a GPU without a DX12 driver, which then draws through OpenGL as before
/// rather than on the CPU with WARP). wgpu's OpenGL backend keeps a hidden window on a thread that
/// pumps no messages, and Windows waits on it when the input language changes, so the whole app
/// hung on a switch to another keyboard layout (#4150, gfx-rs/wgpu#10504), even drawing with DX12.
pub fn windows_opengl(left_out: &LeftOut) -> bool {
    opengl_needed(left_out, !skipped().is_empty(), std::env::var_os(NAME_ENV).is_some(), dx12_lists_hardware)
}

/// [`windows_opengl`], with a restart leaving adapters out (`skipping`) and an adapter named
/// (`named`): `dx12_hardware` is asked only when nothing else decides.
fn opengl_needed(left_out: &LeftOut, skipping: bool, named: bool, dx12_hardware: impl FnOnce() -> bool) -> bool {
    left_out.opengl_only || !left_out.adapters.is_empty() || skipping || named || !dx12_hardware()
}

/// Whether Direct3D 12 lists a hardware adapter; WARP, its software one, doesn't count. A driver
/// that crashes here crashes a start while it lists the adapters, which the next start answers with
/// OpenGL alone.
#[cfg(windows)]
fn dx12_lists_hardware() -> bool {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = Backends::DX12;
    // The shader compiler the window's own instance takes (#1194).
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::from_env().unwrap_or(wgpu::Dx12Compiler::Fxc);
    let instance = wgpu::Instance::new(descriptor);
    let kinds: Vec<DeviceType> = pollster::block_on(instance.enumerate_adapters(Backends::DX12)).iter().map(|a| a.get_info().device_type).collect();
    log::info!("Direct3D 12 lists these graphics adapters: {kinds:?}");
    any_hardware(&kinds)
}

#[cfg(not(windows))]
fn dx12_lists_hardware() -> bool {
    false
}

/// Is one of the adapters of these kinds hardware, not software such as WARP?
#[cfg(any(windows, test))]
fn any_hardware(kinds: &[DeviceType]) -> bool {
    kinds.iter().any(|k| *k != DeviceType::Cpu)
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
mod tests;
