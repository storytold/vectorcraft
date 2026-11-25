use std::collections::HashMap;
use std::path::PathBuf;

use craft::dialogs::ShowHook;
use craft::dialogs::service::{ExtensionPolicy, FilePanels, FileService};
use craft::host::Wake;
use craft::platform::clipboard::ClipboardBackend;
use craft::ui::runtime::pen::PenSource;
use vectorcraft_ui_egui::hang_beacon::{Activity, BEACON, Beacon};

#[cfg(target_os = "macos")]
use super::mac_tablet;
#[cfg(target_os = "linux")]
use super::tablet;
#[cfg(all(windows, not(target_vendor = "win7")))]
use super::windows_stylus;
use super::{Services, clipboard, fileio, printing};

/// The main window, which the pen reader looks at to tell an X11 window from a Wayland one.
pub(crate) type Parent = Option<std::sync::Arc<winit::window::Window>>;

/// The native file service and the panels that show its dialogs. `folders` are the folders the last
/// session's picks chose, so a pick of this one starts where that ended.
pub(crate) fn file_service(wake: Wake, folders: HashMap<String, PathBuf>) -> (FileService, FilePanels) {
    let (service, panels) = FileService::new(wake);
    let panels = panels.extension_policy(ExtensionPolicy::AS_TYPED).show_hook(marking_dialogs(&BEACON));
    (service.with_last_folders(folders), panels)
}

/// A hook that marks `beacon` as in a dialog while the dialog shows: it waits for the user, so the
/// hang report (`hang_watch`) doesn't take the wait for a stall. Making the dialog stays timed.
fn marking_dialogs(beacon: &'static Beacon) -> ShowHook {
    Box::new(move |show: &mut dyn FnMut()| {
        let _mark = beacon.activity(Activity::Dialog);
        show();
    })
}

/// Starts the host's tablet reader for `window`: the pen source it writes the pen's pressure and end
/// into. XInput2 under X11 and XWayland, AppKit's events on macOS, the pen's pointer messages on
/// Windows; other windows read the pen from touch events only (`None`).
pub(crate) fn start_pen(window: &Parent) -> Option<PenSource> {
    #[cfg(target_os = "linux")]
    {
        use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let x11 = window
            .as_ref()
            .and_then(|w| w.window_handle().ok())
            .is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)));
        x11.then(tablet::start).flatten()
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        mac_tablet::start()
    }
    #[cfg(all(windows, not(target_vendor = "win7")))]
    {
        let _ = window;
        Some(windows_stylus::start())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", all(windows, not(target_vendor = "win7")))))]
    {
        let _ = window;
        None
    }
}

#[cfg(test)]
#[test]
fn a_dialog_is_marked_while_it_shows_and_not_after() {
    let beacon: &'static Beacon = Box::leak(Box::new(Beacon::new()));
    let mut hook = marking_dialogs(beacon);
    let mut showing = None;
    hook(&mut || showing = Some(beacon.sample().activity));
    assert_eq!(showing, Some(Activity::Dialog), "a dialog waiting for the user is not a stall");
    assert_eq!(beacon.sample().activity, Activity::None, "the mark ends with the dialog");
}

/// File → Show in Folder: select `path` in Finder / Explorer, or open its folder elsewhere.
fn reveal(path: &str) -> Result<(), String> {
    explorer_safe(path)?;
    reveal_command(path).spawn().map(|_| ()).map_err(|e| format!("can't show {path}: {e}"))
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &str) -> std::process::Command {
    let mut c = std::process::Command::new("open");
    c.args(["-R", path]);
    c
}

#[cfg(windows)]
fn reveal_command(path: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt as _;
    // Explorer reads `/select,"path"` itself (the usual argument quoting breaks paths with spaces)
    // and needs backslashes.
    let mut c = std::process::Command::new("explorer");
    c.raw_arg(format!("/select,\"{}\"", path.replace('/', "\\")));
    c
}

#[cfg(not(any(target_os = "macos", windows)))]
fn reveal_command(path: &str) -> std::process::Command {
    let folder = std::path::Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let mut c = std::process::Command::new("xdg-open");
    c.arg(folder);
    c
}

/// Read a whole file. A device or pipe may never end: only a regular file is read.
fn read_file(path: &str) -> Result<Vec<u8>, String> {
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{path}: not a file"));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

#[cfg(test)]
#[test]
fn read_file_refuses_what_is_not_a_regular_file() {
    assert!(read_file(&std::env::temp_dir().to_string_lossy()).is_err());
    #[cfg(unix)]
    assert!(read_file("/dev/zero").is_err());
}

/// Read the first `max` bytes of a file (all of it when it is shorter), and nothing past them. Only
/// a regular file is read.
fn read_file_head(path: &str, max: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{path}: not a file"));
    }
    let mut out = vec![];
    std::fs::File::open(path).and_then(|f| f.take(max as u64).read_to_end(&mut out)).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
#[test]
fn read_file_head_reads_no_more_than_it_is_asked_for() {
    use std::io::Write as _;
    let mut f = tempfile::Builder::new().prefix("vectorcraft-read-head-").tempfile().unwrap();
    f.write_all(b"0123456789").unwrap();
    let p = f.path().to_string_lossy();
    assert_eq!(read_file_head(&p, 4).unwrap(), b"0123");
    assert_eq!(read_file_head(&p, 20).unwrap(), b"0123456789", "a shorter file is read whole");
    assert!(read_file_head(&std::env::temp_dir().to_string_lossy(), 4).is_err(), "a folder is not read");
}

/// Write a file the safe way: a failed write keeps the old file ([`fileio::write_atomic`]).
fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    fileio::write_atomic(std::path::Path::new(path), bytes).map_err(|e| e.to_string())
}

/// The host's services. File dialogs, the clipboard, URLs, the pen and the screen color picker are
/// the framework's.
pub(crate) fn services(window: Parent) -> Services {
    let probe_window = window.clone();
    Services {
        // Every format Copy offers and Paste reads. Its calls are marked for the hang report (`hang_watch`).
        clipboard: Some(Box::new(vectorcraft_ui_egui::hang_beacon::Marked(clipboard::system_clipboard(window)))),
        // Linux checks whether Paste has something to take on a background thread: an X11 clipboard
        // owner that never answers holds a text read for up to 4 s for each format it tries, and
        // quitting waits up to `craft::platform::clipboard::PROBE_STOP_WAIT` for that thread. Windows
        // only asks which formats the clipboard holds and macOS asks the pasteboard server, so they
        // check in line. The probe's handle is made on its own thread, from the window (`Send`).
        clipboard_probe: cfg!(target_os = "linux")
            .then(|| Box::new(move || clipboard::system_clipboard(probe_window)) as Box<dyn FnOnce() -> Box<dyn ClipboardBackend> + Send>),
        read: Some(Box::new(read_file)),
        // Load Actions… refuses a file over its size limit without reading all of it.
        read_head: Some(Box::new(read_file_head)),
        write: Some(Box::new(write_file)),
        exists: Some(std::sync::Arc::new(|p: &str| std::path::Path::new(p).exists())),
        // Background Save and Export write from a worker thread.
        write_shared: Some(std::sync::Arc::new(write_file)),
        save_write: Some(std::sync::Arc::new(fileio::write_save_file)),
        save_stream: Some(std::sync::Arc::new(fileio::write_save_file_with)),
        // Native saves stream into the file as they are encoded.
        stream_shared: Some(std::sync::Arc::new(|p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
            fileio::write_file_with(p, fill).map_err(|e| e.to_string())
        })),
        reveal: Some(Box::new(reveal)),
        // Links panel: Edit Original; Package: Show Package. Relink to Folder and Package pick folders.
        open_file: Some(Box::new(open_file)),
        // Edit Original of a raster image with Use System Defaults for Edit Original off (#5236).
        open_in_photocraft: Some(Box::new(open_in_photocraft)),
        check_application: Some(Box::new(|p| check_app(std::path::Path::new(p)))),
        open_in_application: Some(Box::new(open_in_application)),
        // File → Print: the system's printers and print queue.
        print: Some(Box::new(printing::SystemPrint)),
        ..Default::default()
    }
}

/// Windows passes `path` to Explorer inside quotes it adds itself, so a path holding a quote (which
/// no Windows file name has) is refused rather than split into other arguments.
fn explorer_safe(path: &str) -> Result<(), String> {
    if cfg!(windows) && path.contains('"') { Err(format!("can't open {path}: the path contains a quotation mark")) } else { Ok(()) }
}

/// Edit Original, Show Package: open `path` (a file or a folder) in the system's default app for it.
fn open_file(path: &str) -> Result<(), String> {
    explorer_safe(path)?;
    #[cfg(windows)]
    let mut c = {
        use std::os::windows::process::CommandExt as _;
        let mut c = std::process::Command::new("explorer");
        c.raw_arg(format!("\"{}\"", path.replace('/', "\\")));
        c
    };
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("open");
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut c = std::process::Command::new("xdg-open");
    #[cfg(not(windows))]
    c.arg(path);
    c.spawn().map(|_| ()).map_err(|e| format!("can't open {path}: {e}"))
}

/// Edit Original with Preferences › File Handling › Use System Defaults for Edit Original off
/// (#5236): open the raster image `path` in PhotoCraft, found only where it installs
/// ([`find_photocraft`]); `Ok(false)` when it isn't installed. PhotoCraft gets the path as its only
/// argument and is never started through a shell.
fn open_in_photocraft(path: &str) -> Result<bool, String> {
    explorer_safe(path)?;
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from);
    let Some(app) = find_photocraft(home.as_deref(), std::env::var_os("PATH").as_deref()) else { return Ok(false) };
    launch_application(&app, path)
}

/// A missing chosen editor falls back; invalid applications and failed launches remain errors.
fn open_in_application(app: &str, path: &str) -> Result<bool, String> {
    explorer_safe(path)?;
    let app = std::path::Path::new(app);
    match std::fs::metadata(app) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("cannot inspect {}: {e}", app.display())),
        Ok(_) => {}
    }
    check_app(app)?;
    launch_application(app, path)
}

fn launch_application(app: &std::path::Path, path: &str) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(app).arg(path);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut c = {
        let mut c = std::process::Command::new(app);
        c.arg(path);
        c
    };
    // macOS: `open` returns once Launch Services has the request, so a short wait reports an app
    // it can't open (a folder named `.app` that isn't a bundle). The wait is capped so a stalled
    // `open` never holds the window: past it, the launch is left to run. Elsewhere the editor
    // itself runs: waiting would last until it closes.
    #[cfg(target_os = "macos")]
    {
        const WAIT: std::time::Duration = std::time::Duration::from_secs(2);
        let fail = |why: String| format!("can't open {path} in {}: {why}", app.display());
        let mut child = c.stderr(std::process::Stdio::piped()).spawn().map_err(|e| fail(e.to_string()))?;
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(true),
                Ok(Some(status)) => {
                    let mut why = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        // Best effort: without its message the exit status says why.
                        let _ = std::io::Read::read_to_string(&mut err, &mut why);
                    }
                    let why = why.trim().to_string();
                    return Err(fail(if why.is_empty() { status.to_string() } else { why }));
                }
                Ok(None) if started.elapsed() < WAIT => std::thread::sleep(std::time::Duration::from_millis(20)),
                Ok(None) => {
                    // Reaped off the UI thread once it ends.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return Ok(true);
                }
                Err(e) => return Err(fail(e.to_string())),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    c.spawn().map(|_| true).map_err(|e| format!("can't open {path} in {}: {e}", app.display()))
}

/// PhotoCraft where it is installed: the first of [`photocraft_candidates`] that is an application.
pub(crate) fn find_photocraft(home: Option<&std::path::Path>, path_var: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    photocraft_candidates(home, path_var).into_iter().find(|p| check_app(p).is_ok())
}

/// Where PhotoCraft may be installed: the Applications folders on macOS; on Windows where its
/// installer puts it ([`windows_candidates`]); else a `photocraft` program on the `PATH`.
pub(crate) fn photocraft_candidates(home: Option<&std::path::Path>, path_var: Option<&std::ffi::OsStr>) -> Vec<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        let mut v = vec![std::path::PathBuf::from("/Applications/PhotoCraft.app")];
        v.extend(home.map(|h| h.join("Applications/PhotoCraft.app")));
        return v;
    }
    let name = if cfg!(windows) { "photocraft.exe" } else { "photocraft" };
    let on_path = path_var.map(|p| std::env::split_paths(p).map(|dir| dir.join(name)).collect()).unwrap_or_default();
    if cfg!(windows) {
        let folders: Vec<std::path::PathBuf> = [
            std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Programs")),
            std::env::var_os("ProgramW6432").map(std::path::PathBuf::from),
            std::env::var_os("ProgramFiles").map(std::path::PathBuf::from),
        ]
        .into_iter()
        .flatten()
        .collect();
        return windows_candidates(registered_photocraft(), &folders, on_path);
    }
    on_path
}

/// Windows: PhotoCraft's installer puts it in `ArtCraft\PhotoCraft` under a folder of
/// `program_folders` (for the current user `%LOCALAPPDATA%\Programs`, for everyone Program Files)
/// and registers its path under App Paths, which also covers a folder chosen in setup. So: the
/// `registered` paths first, then those folders, then `on_path`.
pub(crate) fn windows_candidates(
    registered: Vec<std::path::PathBuf>,
    program_folders: &[std::path::PathBuf],
    on_path: Vec<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    let mut v = registered;
    for f in program_folders {
        let p = f.join("ArtCraft").join("PhotoCraft").join("photocraft.exe");
        if !v.contains(&p) {
            v.push(p);
        }
    }
    v.extend(on_path);
    v
}

/// The paths PhotoCraft's installer registers under App Paths, for the current user and then for
/// everyone.
#[cfg(windows)]
fn registered_photocraft() -> Vec<std::path::PathBuf> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .filter_map(|hive| registry_default_path(hive, r"Software\Microsoft\Windows\CurrentVersion\App Paths\photocraft.exe"))
        .collect()
}

#[cfg(not(windows))]
fn registered_photocraft() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// The default `REG_SZ` value of `subkey` under `hive` as a path ([`registered_path`]), or `None`
/// when the key or the value is missing. The value is read as UTF-16, so a folder name in any
/// script comes back as stored.
#[cfg(windows)]
#[allow(unsafe_code)]
fn registry_default_path(hive: windows::Win32::System::Registry::HKEY, subkey: &str) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::PCWSTR;
    let subkey: Vec<u16> = subkey.encode_utf16().chain([0]).collect();
    let mut size = 0u32;
    // SAFETY: `subkey` is a NUL-terminated UTF-16 string that outlives the call, a null value name
    // reads the default value, and with no buffer the call only writes the value's size to `size`.
    unsafe { RegGetValueW(hive, PCWSTR(subkey.as_ptr()), PCWSTR::null(), RRF_RT_REG_SZ, None, None, Some(&mut size)) }.ok().ok()?;
    let mut value = vec![0u16; (size as usize).div_ceil(2)];
    let mut size = u32::try_from(value.len() * 2).ok()?;
    // SAFETY: as above, and `value` holds the `size` bytes the call may write.
    unsafe { RegGetValueW(hive, PCWSTR(subkey.as_ptr()), PCWSTR::null(), RRF_RT_REG_SZ, None, Some(value.as_mut_ptr().cast()), Some(&mut size)) }
        .ok()
        .ok()?;
    registered_path(&value)
}

/// The path in a registry string value read as UTF-16, up to its first NUL, without surrounding
/// spaces or quotes, like `C:\Program Files\ArtCraft\PhotoCraft\photocraft.exe`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn registered_path(value: &[u16]) -> Option<std::path::PathBuf> {
    let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    let text = String::from_utf16(&value[..end]).ok()?;
    Some(text.trim().trim_matches('"')).filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// Is `app` an application Edit Original may start: an `.app` bundle on macOS, an `.exe` on
/// Windows, an executable file elsewhere? Anything else (a document, a folder, a script on
/// Windows) found where PhotoCraft installs is not taken for it.
pub(crate) fn check_app(app: &std::path::Path) -> Result<(), String> {
    let ext = app.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let ok = if cfg!(target_os = "macos") {
        ext == "app" && app.is_dir()
    } else if cfg!(windows) {
        ext == "exe" && app.is_file()
    } else {
        executable_file(app)
    };
    if ok { Ok(()) } else { Err(format!("{} isn't an application Edit Original can open files in", app.display())) }
}

#[cfg(unix)]
fn executable_file(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_file(p: &std::path::Path) -> bool {
    p.is_file()
}

#[cfg(test)]
mod open_with_tests {
    use super::{check_app, find_photocraft, photocraft_candidates, registered_path, windows_candidates};

    /// A fresh, empty folder under the system's temporary folder.
    fn scratch(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new().prefix(&format!("vc-open-with-{name}-")).tempdir().unwrap()
    }

    #[test]
    fn chosen_editor_only_falls_back_when_missing() {
        let dir = scratch("chosen");
        let missing = dir.path().join("Missing.exe");
        assert!(!super::open_in_application(&missing.to_string_lossy(), "image with spaces.png").unwrap());
        let doc = dir.path().join("notes.txt");
        std::fs::write(&doc, "not an application").unwrap();
        assert!(super::open_in_application(&doc.to_string_lossy(), "image.png").is_err());
        assert!(super::open_in_application(&dir.path().to_string_lossy(), "image.png").is_err());
    }

    /// #5236: only an application counts as PhotoCraft, never a document, a folder or a missing
    /// path.
    #[test]
    fn only_applications_are_started() {
        let temp = scratch("check");
        let d = temp.path();
        let doc = d.join("notes.txt");
        std::fs::write(&doc, "hi").unwrap();
        assert!(check_app(&doc).is_err(), "a document");
        assert!(check_app(d).is_err(), "a folder");
        assert!(check_app(&d.join("Missing.app")).is_err(), "a missing path");
        if cfg!(target_os = "macos") {
            let app = d.join("Editor.app");
            std::fs::create_dir_all(&app).unwrap();
            assert!(check_app(&app).is_ok(), "an .app bundle");
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let tool = d.join("editor");
            std::fs::write(&tool, "#!/bin/sh\n").unwrap();
            assert!(check_app(&tool).is_err(), "not executable yet");
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(check_app(&tool).is_ok(), "an executable file");
        }
    }

    /// PhotoCraft is looked for only where it installs.
    #[test]
    fn photocraft_is_found_where_it_installs() {
        let temp = scratch("find");
        let d = temp.path();
        if cfg!(target_os = "macos") {
            let candidates = photocraft_candidates(Some(d), None);
            assert_eq!(candidates.first().map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/Applications/PhotoCraft.app"));
            assert!(candidates.contains(&d.join("Applications/PhotoCraft.app")), "{candidates:?}");
            std::fs::create_dir_all(d.join("Applications/PhotoCraft.app")).unwrap();
            assert!(find_photocraft(Some(d), None).unwrap().ends_with("PhotoCraft.app"));
        } else {
            let bin = d.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let path_var = std::env::join_paths([&bin]).unwrap();
            // On Windows an installed PhotoCraft (registered, or in a program folder) comes first, so
            // there only the `PATH` candidate is checked.
            if !cfg!(windows) {
                assert!(find_photocraft(None, Some(&path_var)).is_none(), "not installed");
            }
            let exe = bin.join(if cfg!(windows) { "photocraft.exe" } else { "photocraft" });
            std::fs::write(&exe, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            if cfg!(windows) {
                assert_eq!(photocraft_candidates(None, Some(&path_var)).last(), Some(&exe));
                assert!(check_app(&exe).is_ok());
            } else {
                assert_eq!(find_photocraft(None, Some(&path_var)), Some(exe));
            }
        }
    }

    /// #5327 review: on Windows PhotoCraft is looked for where its installer puts it: the path it
    /// registers, then `ArtCraft\PhotoCraft` in the per-user and per-machine program folders, then
    /// the `PATH`.
    #[test]
    fn windows_looks_where_the_photocraft_installer_puts_it() {
        use std::path::PathBuf;
        let registered = vec![PathBuf::from("D:/Tools/PhotoCraft/photocraft.exe")];
        let folders = [PathBuf::from("C:/Users/me/AppData/Local/Programs"), PathBuf::from("C:/Program Files"), PathBuf::from("C:/Program Files")];
        let on_path = vec![PathBuf::from("C:/bin/photocraft.exe")];
        let exe = |f: &str| PathBuf::from(f).join("ArtCraft").join("PhotoCraft").join("photocraft.exe");
        assert_eq!(
            windows_candidates(registered, &folders, on_path),
            [
                PathBuf::from("D:/Tools/PhotoCraft/photocraft.exe"),
                exe("C:/Users/me/AppData/Local/Programs"),
                exe("C:/Program Files"),
                PathBuf::from("C:/bin/photocraft.exe"),
            ]
        );
    }

    /// The registered path is read from the UTF-16 value up to its NUL, spaces, quotes and
    /// characters outside ASCII included; an empty value or one that is not UTF-16 gives no path.
    #[test]
    fn the_registered_path_is_read_from_the_utf16_value() {
        let wide = |s: &str| s.encode_utf16().chain([0, 0]).collect::<Vec<u16>>();
        assert_eq!(
            registered_path(&wide(r"C:\Program Files\ArtCraft\PhotoCraft\photocraft.exe")),
            Some(std::path::PathBuf::from(r"C:\Program Files\ArtCraft\PhotoCraft\photocraft.exe"))
        );
        assert_eq!(
            registered_path(&wide(" \"C:\\Prögram Fïles\\Тест\\写真\\photocraft.exe\" ")),
            Some(std::path::PathBuf::from(r"C:\Prögram Fïles\Тест\写真\photocraft.exe"))
        );
        assert_eq!(registered_path(&wide("")), None);
        assert_eq!(registered_path(&[0xD800, 0]), None);
    }

    /// On Windows a registered path with characters outside the console's code page is read as
    /// stored, from a scratch key under the current user's hive.
    #[cfg(windows)]
    #[test]
    fn a_registered_path_outside_ascii_is_read_as_stored() {
        let key = format!(r"Software\VectorCraftTest-app-paths-{}", std::process::id());
        let path = r"C:\Prögram Fïles\Тест\写真\photocraft.exe";
        let reg = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into()).join(r"System32\reg.exe");
        let full = format!(r"HKCU\{key}");
        let added = std::process::Command::new(&reg).args(["add", &full, "/ve", "/t", "REG_SZ", "/d", path, "/f"]).output().unwrap();
        assert!(added.status.success(), "{added:?}");
        let read = super::registry_default_path(windows::Win32::System::Registry::HKEY_CURRENT_USER, &key);
        let _ = std::process::Command::new(&reg).args(["delete", &full, "/f"]).output();
        assert_eq!(read, Some(std::path::PathBuf::from(path)));
        assert_eq!(super::registry_default_path(windows::Win32::System::Registry::HKEY_CURRENT_USER, &key), None, "deleted");
    }
}
