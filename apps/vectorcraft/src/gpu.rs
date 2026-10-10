//! Which graphics adapter the window renders with (#306, #502), and what happens when it can't
//! show the window.
//!
//! The canvas is rasterized on the CPU; the GPU only composites it and draws the UI, so any
//! adapter that can present to the window will do. [`adapter_order`] ranks the adapters: those that
//! can't present to the window's surface never, then the one `WGPU_ADAPTER_NAME` names, hardware
//! before software, the native backends (Vulkan, Metal, DX12) before OpenGL, the GPU that drives a
//! display and the one that drives the primary display ([`display_gpus`]), the power preference
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
//! An adapter can report that it presents to the window and still fail once it does: on a hybrid
//! Linux desktop under Wayland, the compositor runs on one GPU and may refuse the frame buffers
//! another GPU allocates, which kills the window's Wayland connection; egui-wgpu then panics while
//! configuring the surface (#502). Nothing in the process can show a window after that, so when
//! the graphics fail while the window is starting up, [`finish`] starts the app again without
//! that adapter, as the next one in the order would be tried. Each restart leaves out one more
//! adapter, so it ends when none is left.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::NativeAdapterSelectorMethod;
use eframe::wgpu::{self, Backend, Backends, DeviceType, PowerPreference};

/// The adapters a restart leaves out, those the window failed on, as comma-separated [`key`]s
/// (`Vulkan:1002:164e`, `Metal:Intel Iris Pro Graphics`). Set by the app itself when it starts
/// again.
pub const SKIP_ENV: &str = "VECTORCRAFT_GPU_SKIP";

/// wgpu's variable for choosing an adapter by (part of) its name, any case.
const NAME_ENV: &str = "WGPU_ADAPTER_NAME";

/// Where to read how to choose another adapter, for the error that ends the app.
const HELP: &str =
    "to choose a graphics adapter, see https://github.com/storytold/vectorcraft/blob/main/docs/development.md#desktop-graphics-processor";

/// Frames the UI ran before the failure, below which it was still starting up. The failure in
/// #502 came on the first or second frame; later, the adapter has proved itself, and a lost
/// window is something else (such as the compositor restarting).
pub const STARTUP_FRAMES: u64 = 10;

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

/// The graphics backends the window starts with: on Windows Direct3D 12, falling back to OpenGL,
/// unless `WGPU_BACKEND` names others (`env_set`; `default` then holds them). Creating a Vulkan
/// instance loads every installed Vulkan driver into the process, and a faulty one (Intel's
/// `igvk64.dll`, #806) crashed the app before its window appeared. DX12 is Windows' own backend,
/// as in PdfCraft. Elsewhere, `default`.
pub fn backends(default: Backends, env_set: bool) -> Backends {
    if cfg!(windows) && !env_set { Backends::DX12 | Backends::GL } else { default }
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
/// software; native backends before OpenGL; the GPUs that drive one of `displays`, the primary
/// display's first; then by `power`; then DX12 before Vulkan, keeping the system's order among
/// equals (all of it for [`PowerPreference::None`]). Windows lists each GPU under both, and Intel's
/// Vulkan driver made the whole window flicker black where DX12 and OpenGL didn't (#545);
/// elsewhere there is no DX12 adapter, so the order is unchanged.
///
/// With no `displays`, or none a candidate's PCI ids match (OpenGL reports no device id, Metal no
/// ids at all), the order is the power preference's alone, as it was.
pub fn adapter_order(candidates: &[Candidate], power: PowerPreference, displays: &[DisplayGpu], named: Option<&str>, skip: &[String]) -> Vec<usize> {
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

/// What the window's start-up leaves for [`finish`].
#[derive(Default)]
pub struct Startup {
    /// The key of the adapter [`selector`] picked.
    adapter: OnceLock<String>,
    /// The UI's context, which counts the frames run.
    ui: OnceLock<egui::Context>,
}

impl Startup {
    /// The app was created on `ctx`.
    pub fn created(&self, ctx: &egui::Context) {
        // Set once; the window is created once.
        let _ = self.ui.set(ctx.clone());
    }
}

/// eframe's adapter choice: the first of [`adapter_order`] for `power`, the GPUs that drive a
/// `displays`, `WGPU_ADAPTER_NAME` and the adapters a restart left out. Every adapter, the displays
/// and the choice are logged: which GPU draws is the first question in every black-window report.
pub fn selector(power: PowerPreference, displays: Vec<DisplayGpu>, startup: Arc<Startup>) -> NativeAdapterSelectorMethod {
    let named = std::env::var(NAME_ENV).ok();
    let skip = skipped();
    Arc::new(move |adapters, surface| {
        let candidates: Vec<Candidate> = adapters.iter().map(|a| Candidate::new(a, surface)).collect();
        let order = adapter_order(&candidates, power, &displays, named.as_deref(), &skip);
        let listed: Vec<String> = candidates
            .iter()
            .map(|c| {
                let (drives_display, drives_primary) = c.drives(&displays);
                let note = if !c.presents {
                    ", can't show the window"
                } else if skip.contains(&c.key) {
                    ", failed before"
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
            return Err(format!("no graphics adapter can show the window (power preference {power:?}, {} adapters)", adapters.len()));
        };
        let why = match chosen.drives(&displays) {
            (_, true) => "it drives the primary display".to_string(),
            (true, false) => "it drives a display".to_string(),
            (false, false) if displays.is_empty() => format!("power preference {power:?}"),
            (false, false) => format!("no adapter matches a display GPU, so power preference {power:?}"),
        };
        log::info!("drawing on {} ({:?}, {:?}): {why}", chosen.name.trim(), chosen.backend, chosen.device_type);
        // Set once; eframe picks the adapter once.
        let _ = startup.adapter.set(chosen.key.clone());
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
            assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, &[]), [2, 0, 3, 1, 5, 4], "{displays:?}");
            assert_eq!(adapter_order(&c, PowerPreference::None, &displays, None, &[]), [2, 0, 3, 1, 5, 4], "{displays:?}");
        }
        // Without the display information the order is power saving's, as before.
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[], None, &[]), [3, 1, 2, 0, 5, 4]);
        // After the NVIDIA adapters failed, the integrated GPU is next.
        let skip = vec![c[2].key.clone(), c[0].key.clone()];
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[primary(NVIDIA_3090)], None, &skip), [3, 1, 5, 4]);
    }

    /// A hybrid laptop: the panel (primary) on the integrated GPU, an external monitor on the
    /// discrete one. Power saving and the display agree, in either enumeration order.
    #[test]
    fn a_hybrid_laptop_draws_on_the_integrated_gpu_that_drives_its_panel() {
        let c = ryzen_desktop();
        for displays in [vec![primary(AMD_IGPU), monitor(NVIDIA_3090)], vec![monitor(NVIDIA_3090), primary(AMD_IGPU)], vec![primary(AMD_IGPU)]] {
            assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, &[]), [3, 1, 2, 0, 5, 4], "{displays:?}");
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
        assert_eq!(adapter_order(&linux, PowerPreference::None, &both, None, &[]), [0, 1, 2, 3]);
        assert_eq!(adapter_order(&linux, PowerPreference::None, &[monitor(INTEL_IGPU)], None, &[]), [2, 0, 1, 3]);
        assert_eq!(adapter_order(&linux, PowerPreference::None, &[monitor(NVIDIA_3090), primary(AMD_IGPU)], None, &[]), [0, 1, 2, 3]);
    }

    /// A Windows desktop with a monitor on each GPU: the window opens on the primary display, so
    /// its GPU draws, whichever kind it is.
    #[test]
    fn the_primary_display_decides_between_gpus_that_both_drive_a_monitor() {
        let c = ryzen_desktop();
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[monitor(AMD_IGPU), primary(NVIDIA_3090)], None, &[]), [2, 0, 3, 1, 5, 4]);
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[primary(AMD_IGPU), monitor(NVIDIA_3090)], None, &[]), [3, 1, 2, 0, 5, 4]);
        // The displays only matter among adapters that can show the window; a GPU driving no
        // display still loses to one that does.
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[monitor(AMD_IGPU), monitor(INTEL_IGPU)], None, &[]), [3, 1, 2, 0, 5, 4]);
    }

    /// Display GPUs none of the adapters match (another driver's ids, OpenGL's missing device id,
    /// Metal's zeros) change nothing: power saving's order, the old behaviour.
    #[test]
    fn displays_no_adapter_matches_leave_the_power_preference_in_charge() {
        let c = ryzen_desktop();
        let low = adapter_order(&c, PowerPreference::LowPower, &[], None, &[]);
        for displays in [vec![monitor(INTEL_IGPU)], vec![primary((0x8086, 0x1234)), monitor((0x8086, 0x5678))], vec![primary((0, 0))]] {
            assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, &[]), low, "{displays:?}");
        }
        assert_eq!(names(&c, &adapter_order(&c, PowerPreference::HighPerformance, &[monitor(INTEL_IGPU)], None, &[]))[0], "NVIDIA GeForce RTX 3090");
        // The #502 machine's adapters report no ids in these tests: the primary display never matches.
        let c = issue_502();
        assert_eq!(
            adapter_order(&c, PowerPreference::None, &[primary(NVIDIA_3090)], None, &[]),
            adapter_order(&c, PowerPreference::None, &[], None, &[])
        );
        assert!(adapter_order(&[], PowerPreference::LowPower, &[primary(NVIDIA_3090)], None, &[]).is_empty());
    }

    /// `WGPU_ADAPTER_NAME` and "can't present" still come before the displays.
    #[test]
    fn a_named_adapter_comes_before_the_one_driving_the_display() {
        let mut c = ryzen_desktop();
        let displays = [primary(NVIDIA_3090)];
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, Some("radeon"), &[]), [3, 1, 2, 0, 5, 4]);
        c[2].presents = false;
        c[0].presents = false;
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, &[]), [3, 1, 5, 4]);
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
        let order = adapter_order(&c, power_preference(None, None), &[], None, &[]);
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
        let order = |power| names(&c, &adapter_order(&c, power, &[], None, &[]));
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
        let first = adapter_order(&c, PowerPreference::LowPower, &[], None, &[]);
        assert_eq!(names(&c, &first)[0], "Intel Iris Pro Graphics");
        let skip = parse_skip(&c[first[0]].key);
        assert_eq!(names(&c, &adapter_order(&c, PowerPreference::LowPower, &[], None, &skip)), ["NVIDIA GeForce GT 750M"]);
        // A comma in a name can't split the list.
        assert_eq!(parse_skip(&key(Backend::Gl, 0, 0, "Mesa, llvmpipe")), ["Gl:Mesa  llvmpipe"]);
    }

    #[test]
    fn a_restart_tries_the_next_adapter() {
        let c = issue_502();
        let mut skip = vec![];
        let mut tried = vec![];
        while let Some(&i) = adapter_order(&c, PowerPreference::LowPower, &[], None, &skip).first() {
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
            let order = adapter_order(&c, power, &[], None, &[]);
            assert!(!order.contains(&0) && !order.contains(&3), "{power:?}: {order:?}");
            assert_eq!(order.len(), 2);
        }
        for p in &mut c {
            p.presents = false;
        }
        assert!(adapter_order(&c, PowerPreference::None, &[], None, &[]).is_empty());
        assert!(adapter_order(&[], PowerPreference::HighPerformance, &[], None, &[]).is_empty());
    }

    #[test]
    fn wgpu_adapter_name_picks_an_adapter_first_and_the_rest_stay_as_fallbacks() {
        let c = issue_502();
        let order = adapter_order(&c, PowerPreference::HighPerformance, &[], Some("radv"), &[]);
        assert_eq!(c[order[0]].name, "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)");
        assert_eq!(order.len(), 4);
        // Any case; every match comes before the rest, natively first.
        let order = adapter_order(&c, PowerPreference::LowPower, &[], Some("GeForce"), &[]);
        assert_eq!(names(&c, &order[..2]), ["NVIDIA GeForce RTX 5070", "NVIDIA GeForce RTX 5070/PCIe/SSE2"]);
        // An empty name or one nothing matches changes nothing.
        for named in ["", "no such gpu"] {
            assert_eq!(
                adapter_order(&c, PowerPreference::LowPower, &[], Some(named), &[]),
                adapter_order(&c, PowerPreference::LowPower, &[], None, &[])
            );
        }
        // A named adapter that failed is left out like any other.
        let skip = vec![c[1].key.clone()];
        assert_eq!(adapter_order(&c, PowerPreference::None, &[], Some("radv"), &skip)[0], 0);
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
        let order = adapter_order(&c, PowerPreference::LowPower, &[], None, &[]);
        assert_eq!(order, [3, 1, 2, 0, 4]);
        assert_eq!(adapter_order(&c, PowerPreference::HighPerformance, &[], None, &[]), [2, 0, 3, 1, 4]);
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
            assert_eq!(adapter_order(&c, power, &[], None, &[]), [1, 0, 3, 2], "{power:?}");
        }
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[], None, &[c[1].key.clone()]), [0, 3, 2], "DX12 failed: Vulkan next");
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
}
