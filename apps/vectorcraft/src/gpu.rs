//! Which graphics adapter the window renders with (#306, #502), and what happens when it can't
//! show the window.
//!
//! Adapter ranking and saved choices belong to `craft_gpu_device`. The app supplies only its
//! display/compositor safety context for Automatic, explicit debug environment choices, and
//! exclusions for adapters that failed to present this window. eframe still owns the device.
//!
//! A failed or hung startup is retried without its exact adapter. Each restart adds that full
//! adapter key's SHA256 fingerprint, so it cannot keep retrying the same device. A locked, bounded start record also
//! catches driver crashes that end the process before Rust can handle them. Crash history is
//! scoped to fingerprints of the app version and complete shared preference, not an adapter's display label.
//! The existing first-frame watchdog and platform-specific listing recovery remain app policy.
//!
use std::io::{Read as _, Seek as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use craft_gpu_device::{AdapterKey, Candidate, Notice, Preference};
use eframe::egui_wgpu::NativeAdapterSelectorMethod;
#[cfg(any(windows, test))]
use eframe::wgpu::DeviceType;
use eframe::wgpu::{self, Backend, Backends, PowerPreference};
use sha2::{Digest, Sha256};

/// Complete-key SHA256 fingerprints left out after a failure in this restart chain. New starts
/// write a JSON array of fingerprint strings. Old structured full-key arrays remain readable;
/// old comma-separated labels apply only when unambiguous. This is internal recovery state,
/// not a saved user preference.
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

/// Only Automatic without a debug power override uses the app's display/compositor context.
pub fn automatic(preference: &Preference, env: Option<PowerPreference>) -> bool {
    env.is_none() && matches!(preference, Preference::Auto)
}

pub fn preferred_displays(preference: &Preference, env: Option<PowerPreference>) -> Vec<DisplayGpu> {
    if automatic(preference, env) { display_gpus() } else { Vec::new() }
}

pub fn preferred_compositor(preference: &Preference, env: Option<PowerPreference>) -> Option<(u32, u32)> {
    if automatic(preference, env) { crate::compositor_gpu::compositor_gpu() } else { None }
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

/// The limits to ask the device for: what eframe `asked` for, lowered to what the `adapter` offers
/// wherever it offers less, so a GPU short of one of them still gets a device (#2105). The texture
/// side, which bounds the window's surface, is the adapter's own, above eframe's 8192 px when the
/// adapter allows more, so a window larger than that keeps its surface (#1459).
pub fn device_limits(asked: wgpu::Limits, adapter: &wgpu::Limits) -> wgpu::Limits {
    let mut limits = asked.or_worse_values_from(adapter);
    limits.max_texture_dimension_2d = adapter.max_texture_dimension_2d;
    limits
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
            if name.eq_ignore_ascii_case(key) && hex.bytes().all(|b| b.is_ascii_hexdigit()) { u32::from_str_radix(hex, 16).ok() } else { None }
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

/// Canonical full identity, not `AdapterKey`'s deliberately abbreviated human display label.
#[cfg(test)]
fn key(candidate: &Candidate) -> String {
    adapter_fingerprint(&AdapterKey::of(candidate))
}

/// Fixed-order, domain-separated canonical arrays cover every identity field. Display labels
/// are not identity; only bounded fingerprints and human labels enter the recovery file.
fn fingerprint(value: serde_json::Value) -> String {
    format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn adapter_fingerprint(key: &AdapterKey) -> String {
    fingerprint(serde_json::json!(["adapter-v1", key.name, key.backend, key.vendor, key.device, key.pci_bus_id, key.driver]))
}

fn preference_fingerprint(preference: &Preference) -> String {
    fingerprint(match preference {
        Preference::Auto => serde_json::json!(["preference-v1", "auto"]),
        Preference::LowPower => serde_json::json!(["preference-v1", "lowPower"]),
        Preference::Software => serde_json::json!(["preference-v1", "software"]),
        Preference::Adapter(key) => serde_json::json!(["preference-v1", "adapter", adapter_fingerprint(key)]),
    })
}

fn version_fingerprint(version: &str) -> String {
    fingerprint(serde_json::json!(["version-v1", version]))
}

fn is_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

fn recorded_name(name: &str) -> String {
    let name = name.trim();
    let mut end = name.len().min(128);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

/// The SDK must see all duplicates before exclusions: removing the corrected adapter must not
/// resurrect its stale sibling. The same policy eligibility is exposed to Preferences.
fn eligible_inventory(candidates: &[Candidate]) -> Vec<Candidate> {
    let mut eligible = vec![false; candidates.len()];
    for index in craft_gpu_device::ranked(candidates, &Preference::Auto) {
        eligible[index] = true;
    }
    candidates
        .iter()
        .zip(eligible)
        .map(|(candidate, eligible)| {
            let mut candidate = candidate.clone();
            candidate.usable &= eligible;
            candidate
        })
        .collect()
}

/// The old restart format lacked bus and driver identity. It may exclude an adapter only when
/// exactly one listed candidate has that label; otherwise leave it to the full-key retry.
fn legacy_key(candidate: &Candidate) -> String {
    if candidate.vendor == 0 && candidate.device == 0 {
        format!("{:?}:{}", candidate.backend, candidate.name.replace(',', " ").trim())
    } else {
        format!("{:?}:{:04x}:{:04x}", candidate.backend, candidate.vendor, candidate.device)
    }
}

fn excluded(candidates: &[Candidate], index: usize, skip: &[String]) -> bool {
    let Some(candidate) = candidates.get(index) else { return true };
    let actual = AdapterKey::of(candidate);
    let fingerprint = adapter_fingerprint(&actual);
    skip.iter().any(|token| {
        if is_fingerprint(token) {
            *token == fingerprint
        } else if let Ok(saved) = serde_json::from_str::<AdapterKey>(token) {
            // Crash identity is exact, unlike the SDK's tolerant saved-choice matching.
            saved == actual
        } else {
            *token == legacy_key(candidate) && candidates.iter().filter(|c| legacy_key(c) == *token).count() == 1
        }
    })
}

fn drives(candidate: &Candidate, displays: &[DisplayGpu]) -> (bool, bool) {
    let shown = displays.iter().find(|g| g.pci != (0, 0) && g.pci == (candidate.vendor, candidate.device));
    (shown.is_some(), shown.is_some_and(|g| g.primary))
}

fn composites(candidate: &Candidate, compositor: Option<(u32, u32)>) -> bool {
    (candidate.vendor != 0 || candidate.device != 0) && compositor == Some((candidate.vendor, candidate.device))
}

/// Resolve only app-specific context into a shared preference. The SDK remains responsible for
/// generic power/device ordering, duplicate drivers, unsafe drivers and saved-adapter fallback.
fn effective_preference(
    candidates: &[Candidate],
    saved: &Preference,
    env_power: Option<PowerPreference>,
    displays: &[DisplayGpu],
    compositor: Option<(u32, u32)>,
    named: Option<&str>,
) -> Preference {
    if let Some(name) = named.map(str::to_lowercase).filter(|n| !n.is_empty())
        && let Some(candidate) = candidates.iter().find(|c| c.usable && c.name.to_lowercase().contains(&name))
    {
        return Preference::Adapter(AdapterKey::of(candidate));
    }
    if let Some(power) = env_power {
        return if power == PowerPreference::LowPower { Preference::LowPower } else { Preference::Auto };
    }
    if !matches!(saved, Preference::Auto) {
        // In particular a missing saved key goes through SDK Auto, not our contextual override.
        return saved.clone();
    }
    let context = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.usable && !c.is_software() && c.backend != Backend::Gl && (composites(c, compositor) || drives(c, displays).0))
        .max_by_key(|(i, c)| (composites(c, compositor), drives(c, displays).1, std::cmp::Reverse(*i)));
    context.map_or(Preference::Auto, |(_, c)| Preference::Adapter(AdapterKey::of(c)))
}

struct SelectionInputs {
    indices: Vec<usize>,
    preference: Preference,
    retried: bool,
}

/// Keep the restart-chain exclusions even for explicit debug choices. A historical crash is
/// retried only when it leaves no usable shared selection, matching the previous recovery rule.
fn selection_inputs(
    candidates: &[Candidate],
    saved: &Preference,
    debug: (Option<PowerPreference>, Option<&str>),
    displays: &[DisplayGpu],
    compositor: Option<(u32, u32)>,
    skip: &[String],
    crashed: &[String],
) -> SelectionInputs {
    let eligible = eligible_inventory(candidates);
    let inputs = |left_out: &[String]| {
        let indices: Vec<usize> = (0..candidates.len()).filter(|&i| eligible[i].usable && !excluded(candidates, i, left_out)).collect();
        let filtered: Vec<Candidate> = indices.iter().map(|&i| candidates[i].clone()).collect();
        let preference = effective_preference(&filtered, saved, debug.0, displays, compositor, debug.1);
        (indices, filtered, preference)
    };
    let left_out: Vec<String> = skip.iter().chain(crashed).cloned().collect();
    let (indices, filtered, preference) = inputs(&left_out);
    if craft_gpu_device::select(&filtered, &preference).is_some() || crashed.is_empty() {
        return SelectionInputs { indices, preference, retried: false };
    }
    let (indices, filtered, preference) = inputs(skip);
    let retried = craft_gpu_device::select(&filtered, &preference).is_some();
    SelectionInputs { indices, preference, retried }
}

/// The adapters [`SKIP_ENV`] lists.
pub fn skipped() -> Vec<String> {
    std::env::var(SKIP_ENV).map(|v| parse_skip(&v)).unwrap_or_default()
}

fn parse_skip(v: &str) -> Vec<String> {
    if v.trim_start().starts_with('[') {
        let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(v) else { return Vec::new() };
        return values
            .into_iter()
            .filter_map(|v| {
                if let Some(old) = v.as_str() {
                    return Some(old.to_string());
                }
                serde_json::from_value::<AdapterKey>(v).ok().map(|key| adapter_fingerprint(&key))
            })
            .collect();
    }
    v.split(',').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect()
}

fn encode_skip(skip: &[String]) -> String {
    let values: Vec<serde_json::Value> = skip
        .iter()
        .map(|token| serde_json::from_str::<AdapterKey>(token).map_or_else(|_| serde_json::json!(token), |key| serde_json::json!(key)))
        .collect();
    serde_json::Value::Array(values).to_string()
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
    /// On the adapter with this full-key fingerprint and bounded name.
    Adapter { key: String, name: String },
}

/// What a start leaves out because earlier starts crashed ([`Startup::track`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Fingerprints of the complete adapter keys that crashed a start.
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

/// An adapter in [`START_FILE`]: its full-key fingerprint and bounded human name.
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
    /// Fingerprints of the app version and complete preference: crashes count only for the same choice.
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
    /// written by another version or under another complete `preference` is dropped.
    fn begin(previous: Option<Self>, version: &str, preference: &Preference) -> (Self, Option<Crash>) {
        let version = version_fingerprint(version);
        let preference = preference_fingerprint(preference);
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
        record.version = version;
        record.preference = preference;
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
    fn picked(&mut self, key: &AdapterKey, name: &str, native: bool) {
        self.starting = Some(Starting { adapter: Some(Recorded { key: adapter_fingerprint(key), name: recorded_name(name) }), native });
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
            "schema": 3,
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
        if text.len() > START_FILE_LIMIT as usize {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let schema = v.get("schema")?.as_u64()?;
        // Schema 2 had complete keys, so its bounded records can be migrated exactly. Older
        // lossy labels cannot distinguish buses/drivers and are deliberately reset.
        if schema != 2 && schema != 3 {
            return None;
        }
        let identity = |v: &serde_json::Value| -> Option<String> {
            let value = v.as_str()?;
            is_fingerprint(value).then(|| value.to_string())
        };
        let recorded = |v: &serde_json::Value| -> Option<Recorded> {
            let key = if schema == 2 {
                adapter_fingerprint(&serde_json::from_value::<AdapterKey>(v.get("key")?.clone()).ok()?)
            } else {
                identity(v.get("key")?)?
            };
            Some(Recorded { key, name: recorded_name(v.get("name")?.as_str()?) })
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
            version: if schema == 2 { version_fingerprint(v.get("version")?.as_str()?) } else { identity(v.get("version")?)? },
            preference: if schema == 2 {
                preference_fingerprint(&serde_json::from_value::<Preference>(v.get("gpuPreference")?.clone()).ok()?)
            } else {
                identity(v.get("gpuPreference")?)?
            },
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

impl Drop for StartFile {
    /// Frees the lock when the start releases the file. A child process that any thread starts holds
    /// a copy of the file's handle until it runs its program, and the lock is only freed by closing
    /// every copy or by unlocking.
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
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
    /// Shared selection notices for translated app UI after eframe has picked its adapter.
    notices: Mutex<Vec<Notice>>,
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
    /// crashed before (see the module docs). `preference` is Preferences ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Âº Performance ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Âº Graphics
    /// Processor as a shared preference; `explicit` is [`explicit_choice`]. When the file can't be used (it can't be
    /// written, or another copy of the app is starting), nothing is recorded or left out.
    pub fn track(&self, dir: &Path, preference: &Preference, explicit: bool) -> LeftOut {
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
        let (record, crash) = StartRecord::begin(previous, vectorcraft_ui_egui::os_window::version(), preference);
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

    /// Notices from the actual shared selection, retained for translated UI rather than just logs.
    pub fn notices(&self) -> Vec<Notice> {
        self.notices.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// [`selector`] picked the adapter with this full key and name; `native` when the listing found
    /// an adapter of a backend other than OpenGL.
    fn picked(&self, key: &AdapterKey, name: &str, native: bool) {
        // Set once; eframe picks the adapter once.
        let _ = self.adapter.set(adapter_fingerprint(key));
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
    /// Overdue, but the UI is still building the first frame: the app's own work (on a slow
    /// machine, or with antivirus reading every font file), not the adapter, which has been given
    /// nothing to show yet.
    Busy,
}

/// The window's start after `frames` frames run, `since` its first frame was due and
/// `since_built` the first frame's UI pass ended (`None` while it runs). A frame counts once its
/// UI pass ends, before it is painted, so the adapter's time starts there: with no frame run, the
/// UI thread is in the app's code, and starting again without the adapter wouldn't help (#5160).
fn first_frame(frames: u64, since: Duration, since_built: Option<Duration>, minimized: bool) -> FirstFrame {
    if frames >= 2 || minimized {
        FirstFrame::Shown
    } else if frames == 0 {
        if since >= FIRST_FRAME_TIMEOUT { FirstFrame::Busy } else { FirstFrame::Wait }
    } else if since_built.unwrap_or(since) >= FIRST_FRAME_TIMEOUT {
        FirstFrame::Hung
    } else {
        FirstFrame::Wait
    }
}

/// Watch the window's first frame from another thread: once it reaches the screen, clear the start
/// from [`START_FILE`]; when it hasn't reached the screen [`FIRST_FRAME_TIMEOUT`] after its UI pass
/// ended, start the app again without the adapter, as [`finish`] does for one that failed, and end
/// this process, whose UI thread is stuck in the graphics driver. A first frame still being built
/// that long is waited for, with a note in the log.
pub fn watch_first_frame(startup: Arc<Startup>) {
    let mut built: Option<Instant> = None;
    let mut noted = false;
    let watch = move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let Some(due) = startup.ready.get() else { continue };
        let ui = startup.ui.get();
        let frames = ui.map_or(0, egui::Context::cumulative_frame_nr);
        if frames >= 1 && built.is_none() {
            built = Some(Instant::now());
        }
        let minimized = ui.and_then(|c| c.input(|i| i.viewport().minimized)).unwrap_or(false);
        match first_frame(frames, due.elapsed(), built.map(|t| t.elapsed()), minimized) {
            FirstFrame::Wait => continue,
            FirstFrame::Busy => {
                if !noted {
                    log::warn!(
                        "the window's first frame is still being built after {}s: waiting for it (the app's work, not the graphics adapter)",
                        FIRST_FRAME_TIMEOUT.as_secs()
                    );
                    noted = true;
                }
                continue;
            }
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
        match restart(&encode_skip(&skip)) {
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

/// eframe's selector, using the shared policy after app-specific presentation exclusions.
/// `inventory` retains actual keys and surface/shared-policy eligibility for Preferences; notices are retained on Startup.
pub fn selector(
    preference: Preference,
    env_power: Option<PowerPreference>,
    displays: Vec<DisplayGpu>,
    compositor: Option<(u32, u32)>,
    crashed: Vec<String>,
    startup: Arc<Startup>,
    inventory: Arc<Mutex<Vec<Candidate>>>,
) -> NativeAdapterSelectorMethod {
    let named = std::env::var(NAME_ENV).ok();
    let skip = skipped();
    Arc::new(move |adapters, surface| {
        let candidates: Vec<Candidate> = adapters.iter().map(|a| Candidate::of(a, surface)).collect();
        *inventory.lock().unwrap_or_else(PoisonError::into_inner) = eligible_inventory(&candidates);
        let inputs = selection_inputs(&candidates, &preference, (env_power, named.as_deref()), &displays, compositor, &skip, &crashed);
        if inputs.retried {
            log::warn!("every usable graphics adapter crashed an earlier start: trying those again, keeping this restart's exclusions");
        }
        let eligible: Vec<wgpu::Adapter> = inputs.indices.iter().filter_map(|&i| adapters.get(i).cloned()).collect();
        let (adapter, selection) = match craft_gpu_device::pick(&eligible, surface, &inputs.preference) {
            Ok(picked) => picked,
            Err(error) => {
                startup.none_left();
                return Err(error.to_string());
            }
        };
        let Some(chosen) = inputs.indices.get(selection.index).and_then(|&i| candidates.get(i)) else {
            startup.none_left();
            return Err("the shared graphics selection is not in the enumerated adapters".to_string());
        };
        *startup.notices.lock().unwrap_or_else(PoisonError::into_inner) = selection.notices;
        let actual_key = AdapterKey::of(chosen);
        log::info!("drawing on {actual_key}; saved preference {preference:?}, effective preference {:?}", inputs.preference);
        startup.picked(&actual_key, &chosen.name, candidates.iter().any(|c| c.backend != Backend::Gl));
        Ok(adapter)
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

/// A source file of wgpu (`wgpu-core-30.0.1/src/ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â¦`, `wgpu-hal-ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â¦`), egui-wgpu or naga.
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
    restart(&encode_skip(&skip)).map_err(|e| format!("{why}; starting again failed: {e}"))
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
