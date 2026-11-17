//! Reads the ICC profile of the display that shows most of the main window, for Preferences ›
//! Performance › Use Monitor Profile for Canvas, and hands it to [`DisplayProfile::report`].
//!
//! - Windows: the display's default profile for its standard color mode. A display with Advanced
//!   Color on (HDR or Automatic Color Management) is converted by Windows, and nothing is read.
//! - X11: colord's profile for the RandR output, then the output's `_ICC_PROFILE` property, then
//!   the screen's `_ICC_PROFILE` when only one output is active.
//! - Wayland, XWayland included: the compositor gives applications no display profile, and the
//!   canvas stays sRGB.
//! - macOS: the window's layer is tagged sRGB and macOS converts it (`mac_color_space`).
//!
//! While the preference is on, a background thread reads the display every [`EVERY`] seconds,
//! and at once when the window moves to another display.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use vectorcraft_ui_egui::display_profile::{DisplayProfile, Found, Reading};

/// Seconds between two readings of the same display.
const EVERY: f64 = 10.0;
/// Seconds between two checks of which display shows the window.
const LOOK_EVERY: f64 = 1.0;

/// Starts the readings from the main window's frames.
#[derive(Default)]
pub(crate) struct Reader {
    /// When the last reading started, and the display it was for.
    last: Option<(f64, Option<String>)>,
    /// When the window's display was last looked up.
    looked: Option<f64>,
    /// A reading is running.
    busy: Arc<AtomicBool>,
}

/// Clears [`Reader::busy`] when the reading thread ends, also by a panic.
struct Done(Arc<AtomicBool>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Reader {
    /// Starts a reading when one is due and none is running. `now` is the frame's time in seconds.
    pub(crate) fn tick(&mut self, window: &winit::window::Window, now: f64, enabled: bool, profile: &DisplayProfile, ctx: &egui::Context) {
        if !enabled || self.busy.load(Ordering::Acquire) || self.looked.is_some_and(|at| (at..at + LOOK_EVERY).contains(&now)) {
            return;
        }
        self.looked = Some(now);
        let display = window.current_monitor().and_then(|m| m.name());
        if !due(self.last.as_ref(), now, display.as_deref()) {
            return;
        }
        self.last = Some((now, display.clone()));
        profile.pending();
        self.busy.store(true, Ordering::Release);
        let done = Done(self.busy.clone());
        let (profile, ctx) = (profile.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("display profile".into()).spawn(move || {
            let _done = done;
            if profile.report(read(display)) {
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("couldn't start reading the display profile: {e}");
        }
    }
}

/// Whether a reading of `display` is due at `now`, after the last one (`last`): none yet, another
/// display, [`EVERY`] seconds later, or a clock that went back.
fn due(last: Option<&(f64, Option<String>)>, now: f64, display: Option<&str>) -> bool {
    last.is_none_or(|(at, shown)| shown.as_deref() != display || !(*at..at + EVERY).contains(&now))
}

#[cfg(windows)]
fn read(display: Option<String>) -> Reading {
    let found = win::read(display.as_deref());
    Reading { display, found }
}

#[cfg(target_os = "linux")]
fn read(display: Option<String>) -> Reading {
    let found = if wayland_session(|k| std::env::var_os(k)) {
        Found::NoProfile("a Wayland session doesn't give applications the display's profile".into())
    } else {
        x11::read(display.as_deref())
    };
    Reading { display, found }
}

#[cfg(all(target_os = "macos", feature = "wgpu"))]
fn read(display: Option<String>) -> Reading {
    Reading { display, found: Found::System("macOS converts the window to the display's profile".into()) }
}

#[cfg(not(any(windows, target_os = "linux", all(target_os = "macos", feature = "wgpu"))))]
fn read(display: Option<String>) -> Reading {
    Reading { display, found: Found::NoProfile("this build doesn't read display profiles on this system".into()) }
}

/// Whether the app runs in a Wayland session, read from the environment `var` gives. XWayland
/// windows count: the variables stay set for them.
#[cfg(any(target_os = "linux", test))]
fn wayland_session(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"].iter().any(|k| var(k).is_some_and(|v| !v.is_empty()))
}

/// The X11 reading from colord's answer `colord` (the profile file it names, read) and the X11
/// properties `properties`: colord's profile when its file can be read, otherwise the
/// properties' profile. When neither has one, the reason colord gave is kept.
#[cfg(any(target_os = "linux", test))]
fn x11_choice(colord: Option<Result<Option<Found>, String>>, properties: impl FnOnce() -> Result<Option<Found>, String>) -> Found {
    let colord_reason = match colord {
        Some(Ok(Some(found @ Found::Profile { .. }))) => return found,
        Some(Ok(Some(Found::NoProfile(e) | Found::System(e)))) | Some(Err(e)) => Some(e),
        Some(Ok(None)) | None => None,
    };
    match properties() {
        Ok(Some(found)) => found,
        Ok(None) => Found::NoProfile(match colord_reason {
            Some(e) => format!("no profile is set for this display ({e})"),
            None => "no profile is set for this display".into(),
        }),
        Err(e) => Found::NoProfile(e),
    }
}

/// `found` when it is not a profile or the profile can be used, otherwise why it can't, so the
/// next source is tried.
#[cfg(any(target_os = "linux", test))]
fn usable(found: Found) -> Found {
    match found {
        Found::Profile { icc, from } => match vectorcraft_ui_egui::display_profile::check_profile(&icc) {
            Ok(()) => Found::Profile { icc, from },
            Err(e) => Found::NoProfile(format!("{from}: {e}")),
        },
        other => other,
    }
}

/// The ICC file at `path`, refused before it is read when it is larger than [`MAX_PROFILE_BYTES`].
///
/// [`MAX_PROFILE_BYTES`]: vectorcraft_ui_egui::display_profile::MAX_PROFILE_BYTES
#[cfg(any(windows, target_os = "linux", test))]
fn read_icc(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let max = vectorcraft_ui_egui::display_profile::MAX_PROFILE_BYTES;
    let too_large = || format!("{} is larger than {} MiB", path.display(), max >> 20);
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?;
    let len = file.metadata().map_err(|e| format!("couldn't read {}: {e}", path.display()))?.len();
    if len > max as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(max as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    if bytes.len() > max { Err(too_large()) } else { Ok(bytes) }
}

/// The file a profile `name` stands for: `name` itself when it is a path, otherwise the file of
/// that name in the system's color folder `dir`.
#[cfg(any(windows, test))]
fn profile_path(dir: Option<&std::path::Path>, name: &str) -> std::path::PathBuf {
    match dir {
        Some(dir) if !name.contains(['\\', '/']) => dir.join(name),
        _ => std::path::PathBuf::from(name),
    }
}

/// `path`'s profile, or why there is none.
#[cfg(any(windows, target_os = "linux"))]
fn profile_file(path: &std::path::Path) -> Found {
    let from = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    match read_icc(path) {
        Ok(icc) => Found::Profile { icc, from },
        Err(e) => Found::NoProfile(e),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use vectorcraft_ui_egui::display_profile::Found;
    use windows::Win32::Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
    };
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HLOCAL, LUID, LocalFree};
    use windows::Win32::Graphics::Gdi::{CreateDCW, DeleteDC};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use windows::Win32::UI::ColorSystem::{
        CPST_STANDARD_DISPLAY_COLOR_MODE, CPT_ICC, GetColorDirectoryW, GetICMProfileW, WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER,
    };
    use windows::core::{HRESULT, PCWSTR, PWSTR, s, w};

    /// `ColorProfileGetDisplayDefault` and `ColorProfileGetDisplayUserScope` (mscms.dll), which
    /// older Windows versions don't have.
    type GetDefault = unsafe extern "system" fn(i32, LUID, u32, i32, i32, *mut PWSTR) -> HRESULT;
    type GetScope = unsafe extern "system" fn(LUID, u32, *mut i32) -> HRESULT;

    /// The display path that shows a GDI device.
    struct Path {
        source: u32,
        target_adapter: LUID,
        target: u32,
    }

    pub(super) fn read(display: Option<&str>) -> Found {
        let Some(device) = display else { return Found::NoProfile("Windows didn't name the window's display".into()) };
        let path = match find_path(device) {
            Ok(path) => path,
            Err(e) => return Found::NoProfile(e),
        };
        if path.as_ref().is_some_and(advanced_color) {
            return Found::System("Windows manages this display's colors (Advanced Color)".into());
        }
        let name = match path.as_ref().and_then(default_profile) {
            Some(Ok(name)) => name,
            Some(Err(e)) => return Found::NoProfile(e),
            // Older Windows, or a display without an active path: GDI's profile for the device.
            None => match icm_profile(device) {
                Ok(name) => name,
                Err(e) => return Found::NoProfile(e),
            },
        };
        match name {
            Some(name) => super::profile_file(&super::profile_path(color_directory().as_deref(), &name)),
            None => Found::NoProfile("no color profile is set for this display".into()),
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn until_nul(w: &[u16]) -> String {
        String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
    }

    /// The active display paths.
    fn paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, String> {
        for _ in 0..4 {
            let (mut paths_len, mut modes_len) = (0u32, 0u32);
            // SAFETY: both pointers are to live `u32`s.
            let e = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_len, &mut modes_len) };
            if e != ERROR_SUCCESS {
                return Err(format!("Windows couldn't list the displays (error {})", e.0));
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_len as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_len as usize];
            // SAFETY: the arrays hold as many elements as the lengths passed with them.
            let e =
                unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut paths_len, paths.as_mut_ptr(), &mut modes_len, modes.as_mut_ptr(), None) };
            if e == ERROR_INSUFFICIENT_BUFFER {
                // The displays changed between the two calls.
                continue;
            }
            if e != ERROR_SUCCESS {
                return Err(format!("Windows couldn't list the displays (error {})", e.0));
            }
            paths.truncate(paths_len as usize);
            return Ok(paths);
        }
        Err("the displays kept changing while Windows listed them".into())
    }

    /// The path whose source is the GDI device `device` (`\\.\DISPLAY1`), if one is active.
    fn find_path(device: &str) -> Result<Option<Path>, String> {
        for p in paths()? {
            let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: p.sourceInfo.adapterId,
                    id: p.sourceInfo.id,
                },
                ..Default::default()
            };
            // SAFETY: the header starts a `DISPLAYCONFIG_SOURCE_DEVICE_NAME` of the size it states.
            if unsafe { DisplayConfigGetDeviceInfo(&mut name.header) } == 0 && until_nul(&name.viewGdiDeviceName).eq_ignore_ascii_case(device) {
                return Ok(Some(Path { source: p.sourceInfo.id, target_adapter: p.targetInfo.adapterId, target: p.targetInfo.id }));
            }
        }
        Ok(None)
    }

    /// Whether Windows color-manages the path's display itself: Advanced Color is on (HDR), or
    /// wide color is enforced (Automatic Color Management on an SDR display).
    fn advanced_color(path: &Path) -> bool {
        let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                adapterId: path.target_adapter,
                id: path.target,
            },
            ..Default::default()
        };
        // SAFETY: the header starts a `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` of the size it states.
        if unsafe { DisplayConfigGetDeviceInfo(&mut info.header) } != 0 {
            return false;
        }
        // SAFETY: every bit pattern is a valid `u32`.
        let bits = unsafe { info.Anonymous.value };
        // advancedColorEnabled is bit 1 and wideColorEnforced bit 2.
        bits & 0b110 != 0
    }

    fn api() -> Option<(GetDefault, GetScope)> {
        static API: OnceLock<Option<(GetDefault, GetScope)>> = OnceLock::new();
        *API.get_or_init(|| {
            // SAFETY: mscms.dll is loaded from System32 only, and stays loaded for the process.
            let module = unsafe { LoadLibraryExW(w!("mscms.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
            // SAFETY: the names are NUL-terminated, and the module is loaded.
            let (default, scope) = unsafe {
                (GetProcAddress(module, s!("ColorProfileGetDisplayDefault"))?, GetProcAddress(module, s!("ColorProfileGetDisplayUserScope"))?)
            };
            // SAFETY: both exports have these signatures (icm.h).
            unsafe {
                Some((
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, GetDefault>(default),
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, GetScope>(scope),
                ))
            }
        })
    }

    /// The display's default profile name for its standard color mode, `None` without the API.
    fn default_profile(path: &Path) -> Option<Result<Option<String>, String>> {
        let (get_default, get_scope) = api()?;
        let mut scope = WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER.0;
        // SAFETY: `scope` is a live `i32`.
        if unsafe { get_scope(path.target_adapter, path.source, &mut scope) }.is_err() {
            scope = WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER.0;
        }
        let mut name = PWSTR::null();
        // SAFETY: `name` is a live pointer the function fills with a string it allocates.
        let hr = unsafe { get_default(scope, path.target_adapter, path.source, CPT_ICC.0, CPST_STANDARD_DISPLAY_COLOR_MODE.0, &mut name) };
        if hr.is_err() {
            return Some(Err(format!("Windows couldn't give the display's profile (error {:#010x})", hr.0)));
        }
        if name.is_null() {
            return Some(Ok(None));
        }
        // SAFETY: `name` is a NUL-terminated string from the call above, freed once with LocalFree.
        let text = unsafe { name.to_string() };
        // SAFETY: as above.
        let _ = unsafe { LocalFree(Some(HLOCAL(name.0.cast()))) };
        Some(text.map(|t| Some(t).filter(|t| !t.is_empty())).map_err(|e| format!("the display's profile name isn't readable: {e}")))
    }

    /// GDI's profile file for the display device `device`.
    fn icm_profile(device: &str) -> Result<Option<String>, String> {
        let device = wide(device);
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let dc = unsafe { CreateDCW(w!("DISPLAY"), PCWSTR(device.as_ptr()), PCWSTR::null(), None) };
        if dc.is_invalid() {
            return Err("Windows couldn't open the display".into());
        }
        let mut len = 0u32;
        // SAFETY: a null buffer asks for the length only.
        let _ = unsafe { GetICMProfileW(dc, &mut len, None) };
        let mut buf = vec![0u16; len.clamp(1, 32 * 1024) as usize];
        len = buf.len() as u32;
        // SAFETY: `buf` holds `len` characters.
        let ok = unsafe { GetICMProfileW(dc, &mut len, Some(PWSTR(buf.as_mut_ptr()))) }.as_bool();
        // SAFETY: `dc` came from CreateDCW above.
        let _ = unsafe { DeleteDC(dc) };
        Ok(ok.then(|| until_nul(&buf)).filter(|n| !n.is_empty()))
    }

    /// The system's color folder (`…\spool\drivers\color`).
    fn color_directory() -> Option<PathBuf> {
        let mut bytes = 0u32;
        // SAFETY: a null buffer asks for the size only, in bytes.
        let _ = unsafe { GetColorDirectoryW(PCWSTR::null(), None, &mut bytes) };
        let mut buf = vec![0u16; (bytes as usize / 2).clamp(1, 32 * 1024)];
        bytes = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `bytes` bytes.
        unsafe { GetColorDirectoryW(PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut bytes) }.as_bool().then(|| PathBuf::from(until_nul(&buf)))
    }

    #[cfg(test)]
    mod tests {
        use windows::Win32::Graphics::Gdi::{DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICEW, EnumDisplayDevicesW};

        use super::*;

        /// The GDI names (`\\.\DISPLAY1`) of the displays attached to the desktop.
        fn attached_displays() -> Vec<String> {
            let mut names = Vec::new();
            for i in 0.. {
                let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
                // SAFETY: `device` is a live `DISPLAY_DEVICEW` with `cb` set to its size.
                if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut device, 0) }.as_bool() {
                    break;
                }
                if device.StateFlags.contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP) {
                    names.push(until_nul(&device.DeviceName));
                }
            }
            names
        }

        #[test]
        fn windows_reads_each_attached_display_and_refuses_an_unknown_one() {
            assert_eq!(read(None), Found::NoProfile("Windows didn't name the window's display".into()));
            assert!(color_directory().is_none_or(|dir| dir.is_absolute()));
            match find_path(r"\\.\DISPLAY999") {
                Ok(path) => {
                    assert!(path.is_none());
                    assert_eq!(read(Some(r"\\.\DISPLAY999")), Found::NoProfile("Windows couldn't open the display".into()));
                }
                // A session without displays (a service): the reading gives Windows' error.
                Err(e) => {
                    eprintln!("the displays couldn't be listed: {e}");
                    assert_eq!(read(Some(r"\\.\DISPLAY999")), Found::NoProfile(e));
                    return;
                }
            }
            for name in attached_displays() {
                // A display without an active path is read through GDI (`icm_profile`).
                let path = find_path(&name).unwrap();
                // winit names the display in GDI's spelling; the match ignores letter case.
                let lower = find_path(&name.to_lowercase()).unwrap();
                assert_eq!(lower.map(|p| (p.source, p.target)), path.as_ref().map(|p| (p.source, p.target)), "{name} in lowercase");
                match read(Some(&name)) {
                    Found::Profile { icc, from } => {
                        eprintln!("{name}: profile {from}, {} bytes", icc.len());
                        assert!(!icc.is_empty() && !from.is_empty());
                        assert!(!path.as_ref().is_some_and(advanced_color));
                    }
                    Found::System(e) => {
                        eprintln!("{name}: system, {e}");
                        assert!(path.as_ref().is_some_and(advanced_color));
                    }
                    Found::NoProfile(e) => {
                        eprintln!("{name}: no profile, {e}");
                        assert!(!e.is_empty());
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::collections::HashMap;
    use std::path::Path;
    use std::time::Duration;

    use vectorcraft_ui_egui::display_profile::{Found, MAX_PROFILE_BYTES};
    use x11rb::connection::Connection as _;
    use x11rb::protocol::randr;
    use x11rb::protocol::xproto::{self, AtomEnum};
    use zbus::blocking::{connection::Builder, proxy::Proxy};
    use zbus::zvariant::OwnedObjectPath;

    const COLORD: &str = "org.freedesktop.ColorManager";
    const COLORD_ROOT: &str = "/org/freedesktop/ColorManager";
    const COLORD_MANAGER: &str = "org.freedesktop.ColorManager";
    const COLORD_DEVICE: &str = "org.freedesktop.ColorManager.Device";
    const COLORD_PROFILE: &str = "org.freedesktop.ColorManager.Profile";

    pub(super) fn read(output: Option<&str>) -> Found {
        let colord = output.map(|o| colord_profile(o).map(|path| path.map(|path| super::usable(super::profile_file(Path::new(&path))))));
        super::x11_choice(colord, || properties(output))
    }

    /// The file colord assigns the RandR output `output`.
    fn colord_profile(output: &str) -> Result<Option<String>, String> {
        let connection = Builder::system()
            .map_err(|e| format!("couldn't connect to the system bus: {e}"))?
            .method_timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("couldn't connect to the system bus: {e}"))?;
        let manager = Proxy::new(&connection, COLORD, COLORD_ROOT, COLORD_MANAGER).map_err(|e| format!("couldn't reach colord: {e}"))?;
        let devices: Vec<OwnedObjectPath> =
            manager.call("GetDevicesByKind", &("display",)).map_err(|e| format!("colord couldn't list the displays: {e}"))?;
        for path in devices {
            let Ok(device) = Proxy::new(&connection, COLORD, &path, COLORD_DEVICE) else { continue };
            let Ok(metadata) = device.get_property::<HashMap<String, String>>("Metadata") else { continue };
            if metadata.get("XRANDR_name").map(String::as_str) != Some(output) {
                continue;
            }
            // `*` asks for the device's default profile.
            let Ok(profile_path) = device.call::<_, _, OwnedObjectPath>("GetProfileForQualifiers", &(vec!["*"],)) else { return Ok(None) };
            let profile =
                Proxy::new(&connection, COLORD, &profile_path, COLORD_PROFILE).map_err(|e| format!("couldn't reach colord's profile: {e}"))?;
            return Ok(profile.get_property::<String>("Filename").ok().filter(|f| !f.is_empty()));
        }
        Ok(None)
    }

    /// The `_ICC_PROFILE` property of the RandR output `output`, or of the screen when only one
    /// output is active.
    fn properties(output: Option<&str>) -> Result<Option<Found>, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| format!("couldn't connect to the X11 display: {e}"))?;
        let root = conn.setup().roots.get(screen).ok_or_else(|| "the X11 display has no screen".to_string())?.root;
        let atom = xproto::intern_atom(&conn, true, b"_ICC_PROFILE")
            .map_err(|e| format!("couldn't ask the X11 display for profiles: {e}"))?
            .reply()
            .map_err(|e| format!("couldn't ask the X11 display for profiles: {e}"))?
            .atom;
        if atom == 0 {
            return Ok(None);
        }
        let resources = randr::get_screen_resources_current(&conn, root)
            .map_err(|e| format!("couldn't list the X11 outputs: {e}"))?
            .reply()
            .map_err(|e| format!("couldn't list the X11 outputs: {e}"))?;
        let (mut active, mut shown) = (0usize, None);
        for o in resources.outputs {
            let Ok(Ok(info)) = randr::get_output_info(&conn, o, resources.config_timestamp).map(|c| c.reply()) else { continue };
            if info.connection != randr::Connection::CONNECTED || info.crtc == 0 {
                continue;
            }
            active += 1;
            if output.is_some_and(|name| info.name == name.as_bytes()) {
                shown = Some(o);
            }
        }
        let longs = (MAX_PROFILE_BYTES / 4) as u32;
        let too_large = || Found::NoProfile(format!("the display's profile is larger than {} MiB", MAX_PROFILE_BYTES >> 20));
        if let Some(o) = shown
            && let Ok(Ok(p)) = randr::get_output_property(&conn, o, atom, AtomEnum::ANY, 0, longs, false, false).map(|c| c.reply())
            && !p.data.is_empty()
        {
            if p.bytes_after > 0 {
                return Ok(Some(too_large()));
            }
            return Ok(Some(Found::Profile { icc: p.data, from: "the X11 output's _ICC_PROFILE".into() }));
        }
        if active != 1 {
            return Ok(None);
        }
        let Ok(Ok(p)) = xproto::get_property(&conn, false, root, atom, AtomEnum::ANY, 0, longs).map(|c| c.reply()) else { return Ok(None) };
        if p.value.is_empty() {
            return Ok(None);
        }
        if p.bytes_after > 0 {
            return Ok(Some(too_large()));
        }
        Ok(Some(Found::Profile { icc: p.value, from: "the X11 screen's _ICC_PROFILE".into() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_is_due_first_after_ten_seconds_and_when_the_window_changes_display() {
        assert!(due(None, 0.0, None));
        let last = (100.0, Some("DP-1".to_string()));
        assert!(!due(Some(&last), 100.0, Some("DP-1")));
        assert!(!due(Some(&last), 109.9, Some("DP-1")));
        assert!(due(Some(&last), 110.0, Some("DP-1")));
        assert!(due(Some(&last), 101.0, Some("HDMI-1")));
        assert!(due(Some(&last), 101.0, None));
        // A clock that went back.
        assert!(due(Some(&last), 50.0, Some("DP-1")));
    }

    #[test]
    fn wayland_sessions_are_told_apart_from_x11_by_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| std::ffi::OsString::from(v))
        };
        assert!(wayland_session(env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")])));
        assert!(wayland_session(env(&[("WAYLAND_SOCKET", "5")])));
        assert!(!wayland_session(env(&[("DISPLAY", ":0")])));
        assert!(!wayland_session(env(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":0")])));
    }

    #[test]
    fn profile_files_larger_than_the_limit_are_refused_before_they_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small.icc");
        std::fs::write(&small, [1u8, 2, 3]).unwrap();
        assert_eq!(read_icc(&small).unwrap(), [1, 2, 3]);
        let big = dir.path().join("big.icc");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(vectorcraft_ui_egui::display_profile::MAX_PROFILE_BYTES as u64 + 1).unwrap();
        assert!(read_icc(&big).unwrap_err().contains("larger than 1 MiB"));
        assert!(read_icc(&dir.path().join("missing.icc")).unwrap_err().starts_with("couldn't open"));
    }

    #[test]
    fn x11_takes_the_output_property_when_colord_s_file_cannot_be_read() {
        let icc = |b: u8| Found::Profile { icc: vec![b], from: "test".into() };
        let property = || Ok(Some(icc(2)));
        // colord's profile, read: the property is not asked for.
        assert_eq!(x11_choice(Some(Ok(Some(icc(1)))), || panic!("not asked")), icc(1));
        // colord names a file that can't be read, or has no profile, or isn't running.
        assert_eq!(x11_choice(Some(Ok(Some(Found::NoProfile("couldn't open a.icc".into())))), property), icc(2));
        assert_eq!(x11_choice(Some(Ok(None)), property), icc(2));
        assert_eq!(x11_choice(Some(Err("no colord".into())), property), icc(2));
        assert_eq!(x11_choice(None, property), icc(2));
        // Neither has a profile: colord's reason is kept.
        assert_eq!(
            x11_choice(Some(Ok(Some(Found::NoProfile("couldn't open a.icc".into())))), || Ok(None)),
            Found::NoProfile("no profile is set for this display (couldn't open a.icc)".into())
        );
        assert_eq!(x11_choice(None, || Ok(None)), Found::NoProfile("no profile is set for this display".into()));
        assert_eq!(x11_choice(None, || Err("no X11".into())), Found::NoProfile("no X11".into()));
        // colord's file holds no usable profile: the property is used.
        let broken = usable(Found::Profile { icc: vec![1, 2, 3], from: "a.icc".into() });
        assert_eq!(broken, Found::NoProfile("a.icc: the file is not an ICC profile".into()));
        assert_eq!(x11_choice(Some(Ok(Some(broken))), property), icc(2));
        assert_eq!(usable(Found::NoProfile("none".into())), Found::NoProfile("none".into()));
    }

    #[test]
    fn a_profile_name_is_found_in_the_color_folder_and_a_path_is_kept() {
        let dir = std::path::Path::new("C:/Windows/System32/spool/drivers/color");
        assert_eq!(profile_path(Some(dir), "Display.icm"), dir.join("Display.icm"));
        assert_eq!(profile_path(Some(dir), "D:\\profiles\\Display.icm"), std::path::PathBuf::from("D:\\profiles\\Display.icm"));
        assert_eq!(profile_path(None, "Display.icm"), std::path::PathBuf::from("Display.icm"));
    }
}
