#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
use super::portal_save;
#[cfg(all(windows, not(target_vendor = "win7")))]
use super::windows_stylus;
use super::{ClipboardProbeFactory, FilePick, PickRequest, Services, clipboard, cursor_theme, fileio, printing};
#[cfg(target_os = "linux")]
use super::{linux_screen_color, tablet};
#[cfg(target_os = "macos")]
use super::{mac_save_format, mac_screen_color, mac_tablet};

/// The window file dialogs belong to.
pub(crate) type Parent = Option<std::sync::Arc<winit::window::Window>>;

/// What rfd's blocking and asynchronous file dialogs are both set up with.
trait DialogSettings: Sized {
    fn parent(self, window: &winit::window::Window) -> Self;
    fn title(self, title: &str) -> Self;
    fn filter(self, name: &str, exts: &[impl ToString]) -> Self;
    fn folder(self, folder: &str) -> Self;
    fn file_name(self, name: &str) -> Self;
}

macro_rules! dialog_settings {
    ($($dialog:ty),*) => {$(
        impl DialogSettings for $dialog {
            fn parent(self, window: &winit::window::Window) -> Self {
                self.set_parent(window)
            }
            fn title(self, title: &str) -> Self {
                self.set_title(title)
            }
            fn filter(self, name: &str, exts: &[impl ToString]) -> Self {
                self.add_filter(name, exts)
            }
            fn folder(self, folder: &str) -> Self {
                self.set_directory(folder)
            }
            fn file_name(self, name: &str) -> Self {
                self.set_file_name(name)
            }
        }
    )*};
}

dialog_settings!(rfd::FileDialog, rfd::AsyncFileDialog);

/// On Linux and the BSDs the dialog is GTK's or the desktop portal's, which match a file type's
/// `*.ext` patterns case-sensitively.
const CASE_SENSITIVE_FILTERS: bool =
    cfg!(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"));

/// The extensions an open dialog's filter is given for `exts`: with `any_case`, each extension
/// that has a letter is followed by a pattern with each letter as a `[xX]` pair, so the filter for
/// `dib` also lists `image.DIB` and `image.Dib`.
fn open_extensions(exts: &[impl AsRef<str>], any_case: bool) -> Vec<String> {
    let mut out = Vec::with_capacity(exts.len() * 2);
    for e in exts {
        let e = e.as_ref();
        out.push(e.to_string());
        if any_case && e.chars().any(|c| c.is_ascii_alphabetic()) {
            out.push(
                e.chars()
                    .map(|c| if c.is_ascii_alphabetic() { format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()) } else { c.to_string() })
                    .collect(),
            );
        }
    }
    out
}

/// `d` set up for `request`: its title, file types, folder and suggested name.
fn dialog_for<D: DialogSettings>(d: D, request: &PickRequest) -> D {
    let pick = match request {
        PickRequest::Open(pick) | PickRequest::Save(pick) => pick,
        PickRequest::OpenMany => {
            return fileio::place_filters().fold(d.title(vectorcraft_ui_egui::i18n::t("Place")), |d, (name, exts)| {
                d.filter(
                    vectorcraft_ui_egui::i18n::tr_ctx(vectorcraft_ui_egui::i18n::current(), "file-filter", name),
                    &open_extensions(exts, CASE_SENSITIVE_FILTERS),
                )
            });
        }
        PickRequest::Folder => return d,
    };
    let any_case = CASE_SENSITIVE_FILTERS && matches!(request, PickRequest::Open(_));
    let d = pick.filters.iter().fold(d, |d, (name, exts)| {
        d.filter(
            vectorcraft_ui_egui::i18n::tr_ctx(vectorcraft_ui_egui::i18n::current(), "file-filter", name.as_str()),
            &open_extensions(exts, any_case),
        )
    });
    let d = match &pick.folder {
        Some(folder) => d.folder(folder),
        None => d,
    };
    if pick.name.is_empty() { d } else { d.file_name(&pick.name) }
}

/// A native file dialog for `request` over `parent` (on Linux the portal then shows it over the
/// window and keeps it in front).
#[cfg(not(target_os = "macos"))]
fn file_dialog(request: &PickRequest, parent: &Parent) -> rfd::FileDialog {
    match parent {
        Some(w) => dialog_for(rfd::FileDialog::new().parent(w), request),
        None => dialog_for(rfd::FileDialog::new(), request),
    }
}

/// Before `request`'s dialog shows: a save dialog's folder (the Templates folder) may not exist yet.
fn make_folder(request: &PickRequest) {
    if let PickRequest::Save(FilePick { folder: Some(folder), .. }) = request {
        let _ = std::fs::create_dir_all(folder);
    }
}

/// Show `dialog` for `request` (on the calling thread, until it closes) → the paths picked. On
/// Linux and the BSDs a save dialog with two or more file types is the desktop portal's, over
/// `parent`, and its answer also names the file type picked ([`portal_save`]); `dialog` shows when
/// the portal can't.
#[cfg(not(target_os = "macos"))]
#[cfg_attr(windows, allow(unused_variables))]
fn show_dialog(dialog: rfd::FileDialog, request: &PickRequest, parent: &Parent) -> Vec<String> {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    if let PickRequest::Save(pick) = request
        && let Some(answer) = portal_save::pick(pick, parent.as_deref())
    {
        return answer;
    }
    let paths = match request {
        PickRequest::Open(_) => dialog.pick_file().into_iter().collect(),
        PickRequest::OpenMany => dialog.pick_files().unwrap_or_default(),
        PickRequest::Save(_) => dialog.save_file().into_iter().collect(),
        PickRequest::Folder => dialog.pick_folder().into_iter().collect(),
    };
    paths.into_iter().map(|p| p.to_string_lossy().to_string()).collect()
}

/// Show the dialog for `request` over `parent` now → the paths picked.
#[cfg(not(target_os = "macos"))]
fn pick_now(request: PickRequest, parent: &Parent) -> Vec<String> {
    let beacon = &vectorcraft_ui_egui::hang_beacon::BEACON;
    marked_dialog(beacon, || make_folder(&request), || show_dialog(file_dialog(&request, parent), &request, parent))
}

/// Run `prepare` (making the dialog's folder), then `show`, marked on `beacon` as a dialog: the
/// dialog waits for the user and is not a stall for the hang report (`hang_watch`), while `prepare`
/// is still timed.
#[cfg(not(target_os = "macos"))]
fn marked_dialog<T>(beacon: &vectorcraft_ui_egui::hang_beacon::Beacon, prepare: impl FnOnce(), show: impl FnOnce() -> T) -> T {
    prepare();
    let _mark = beacon.activity(vectorcraft_ui_egui::hang_beacon::Activity::Dialog);
    show()
}

#[cfg(all(test, not(target_os = "macos")))]
#[test]
fn making_a_dialogs_folder_is_timed_and_the_dialog_is_not() {
    use vectorcraft_ui_egui::hang_beacon::{Activity, Beacon};
    let beacon = Beacon::new();
    let mut preparing = None;
    let showing = marked_dialog(&beacon, || preparing = Some(beacon.sample().activity), || beacon.sample().activity);
    assert_eq!(preparing, Some(Activity::None), "making the folder is timed");
    assert_eq!(showing, Activity::Dialog, "the dialog is not");
    assert_eq!(beacon.sample().activity, Activity::None, "the mark ends with the dialog");
}

/// macOS shows no dialog in line: its modal loop would run inside winit's event handler, and an
/// event winit held back meanwhile aborts the app (#867). Dialogs are sheets there ([`start_pick`]);
/// one asked for any other way (a bug: outside a command, a dialog's OK or a button) picks nothing.
#[cfg(target_os = "macos")]
fn pick_now(request: PickRequest, _parent: &Parent) -> Vec<String> {
    log::error!("no file dialog shown for {request:?}: macOS shows them as sheets, from commands, dialogs' OK and buttons only");
    Vec::new()
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

/// Linux: show the dialog for `request` over `parent` on a thread of its own, answering on the
/// receiver. Shown on the UI thread, nothing would answer the compositor meanwhile, which then
/// offers to kill the window as not responding (#592).
#[cfg(not(target_os = "macos"))]
fn start_pick(request: PickRequest, parent: &Parent, ui: &egui::Context) -> Option<std::sync::mpsc::Receiver<Vec<String>>> {
    let dialog = file_dialog(&request, parent);
    let (tx, rx) = std::sync::mpsc::channel();
    let ui = ui.clone();
    let parent = parent.clone();
    std::thread::Builder::new()
        .name("file-dialog".into())
        .spawn(move || {
            make_folder(&request);
            // The app gone meanwhile has no use for the answer.
            let _ = tx.send(show_dialog(dialog, &request, &parent));
            ui.request_repaint();
        })
        .ok()?;
    Some(rx)
}

/// The paths a dialog shown as a sheet picks, once it closes.
#[cfg(target_os = "macos")]
type SheetAnswer = std::pin::Pin<Box<dyn std::future::Future<Output = Vec<String>> + Send>>;

/// macOS: show the dialog for `request` as a sheet on `parent`'s window, answering on the receiver
/// (#867). Shown in line, the panel ran AppKit's modal event loop inside winit's event handler: an
/// event winit had held back meanwhile (one came as a resize of the panel ended) then re-entered
/// the handler, and winit's panic there, inside an AppKit callback, aborted the app. The sheet
/// returns at once and the window keeps running; a thread of its own waits for the answer.
#[cfg(target_os = "macos")]
fn start_pick(request: PickRequest, parent: &Parent, ui: &egui::Context) -> Option<std::sync::mpsc::Receiver<Vec<String>>> {
    use winit::raw_window_handle::HasWindowHandle as _;
    // rfd runs the panel modally after all unless it has the running app's window to hold the
    // sheet (and from another thread it can't reach the window): no dialog then (`pick_now`).
    let mtm = objc2::MainThreadMarker::new();
    let running = mtm.is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isRunning());
    let Some(window) = parent.as_ref().filter(|w| running && w.window_handle().is_ok()) else {
        log::error!("no window to show the file dialog on (main thread: {}, running: {running})", mtm.is_some());
        return None;
    };
    // The thread first, so the answer of a sheet once shown always has somewhere to go.
    let (sheet_tx, sheet_rx) = std::sync::mpsc::channel::<SheetAnswer>();
    let (tx, rx) = std::sync::mpsc::channel();
    let ui = ui.clone();
    std::thread::Builder::new()
        .name("file-dialog".into())
        .spawn(move || {
            if let Ok(answer) = sheet_rx.recv() {
                // The app gone meanwhile has no use for the answer.
                let _ = tx.send(pollster::block_on(answer));
                ui.request_repaint();
            }
        })
        .ok()?;
    make_folder(&request);
    // The panel is built and the sheet is begun below, on the main thread (as it must).
    let dialog = dialog_for(rfd::AsyncFileDialog::new().parent(window), &request);
    let path = |f: &rfd::FileHandle| f.path().to_string_lossy().to_string();
    let answer: SheetAnswer = match request {
        PickRequest::Open(_) => {
            let picked = dialog.pick_file();
            Box::pin(async move { picked.await.iter().map(path).collect() })
        }
        PickRequest::OpenMany => {
            let picked = dialog.pick_files();
            Box::pin(async move { picked.await.unwrap_or_default().iter().map(path).collect() })
        }
        // A save panel with two or more named file types is built by `mac_save_format`, which sets
        // up its Format menu before the sheet begins (#866); rfd shows the others.
        PickRequest::Save(pick) => match mac_save_format::begin(window, &pick) {
            // The dialog thread blocks here until the sheet answers.
            Ok(mac_save_format::Begin::Shown(picked)) => Box::pin(async move { picked.recv().unwrap_or_default() }),
            Ok(mac_save_format::Begin::DeferToRfd) => {
                // Nothing is added to the sheet once rfd has begun it: an accessory view set then
                // made recent macOS versions dismiss the sheet at once (#1800).
                let picked = dialog.save_file();
                Box::pin(async move { picked.await.iter().map(path).collect() })
            }
            // AppKit returned nil for the panel. rfd uses the same `+savePanel` and would panic
            // on that nil; treat the pick as cancelled instead (#5980).
            Err(()) => Box::pin(async move { Vec::new() }),
        },
        PickRequest::Folder => {
            let picked = dialog.pick_folder();
            Box::pin(async move { picked.await.iter().map(path).collect() })
        }
    };
    // The thread can't be gone: it waits for this. If it were, its receiver answers as cancelled.
    let _ = sheet_tx.send(answer);
    Some(rx)
}

/// The host's services; `ui` is woken when a file dialog answers.
pub(crate) fn services(parent: Parent, ui: egui::Context) -> Services {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let wayland = parent.as_ref().and_then(|w| w.window_handle().ok()).is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)));
    // A window on X11 (XWayland included) reads a tablet pen's pressure itself (#764).
    #[cfg(target_os = "linux")]
    let x11 = parent
        .as_ref()
        .and_then(|w| w.window_handle().ok())
        .is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)));
    let (clip, clip_probe) = (parent.clone(), parent.clone());
    let (p1, p2, p3, p4, p5) = (parent.clone(), parent.clone(), parent.clone(), parent.clone(), parent);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let sampled_ui = ui.clone();
    #[cfg(target_os = "linux")]
    let sampled_parent = p1.clone();
    Services {
        pick_open: Some(Box::new(move |pick: &FilePick| pick_now(PickRequest::Open(pick.clone()), &p1).into_iter().next())),
        pick_open_multi: Some(Box::new(move || pick_now(PickRequest::OpenMany, &p2))),
        pick_save: Some(Box::new(move |pick: &FilePick| pick_now(PickRequest::Save(pick.clone()), &p3).into_iter().next())),
        // Linux shows dialogs on a thread of their own (#592), macOS as sheets (#867); Windows
        // dialogs run the window's events while they are open.
        start_pick: cfg!(unix).then(|| Box::new(move |request: PickRequest| start_pick(request, &p5, &ui)) as vectorcraft_ui_egui::picks::StartPick),
        dialogs_are_sheets: cfg!(target_os = "macos"),
        wayland,
        no_cursor_theme: wayland && cursor_theme::theme_missing(),
        #[cfg(target_os = "linux")]
        stylus: if x11 { tablet::start() } else { None },
        // macOS reads it from AppKit's events (#852).
        #[cfg(target_os = "macos")]
        stylus: mac_tablet::start(),
        // Windows reads it from the pen's pointer messages (`windows_stylus`).
        #[cfg(windows)]
        stylus: Some(windows_stylus::start()),
        read: Some(Box::new(read_file)),
        // Load Actions… refuses a file over its size limit without reading all of it.
        read_head: Some(Box::new(read_file_head)),
        write: Some(Box::new(write_file)),
        exists: Some(Box::new(|p: &str| std::path::Path::new(p).exists())),
        // Background Save and Export write from a worker thread.
        write_shared: Some(std::sync::Arc::new(write_file)),
        // Native saves stream into the file as they are encoded.
        stream_shared: Some(std::sync::Arc::new(|p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
            fileio::write_file_with(p, fill).map_err(|e| e.to_string())
        })),
        // Every format Copy offers and Paste reads (menu-bar Paste never sees egui's Paste event).
        // Its calls are marked for the hang report (`hang_watch`).
        system_clipboard: Some(Box::new(vectorcraft_ui_egui::hang_beacon::Marked(clipboard::system_clipboard(clip)))),
        // Linux checks whether Paste has something to take on a background thread: an X11 clipboard
        // owner that never answers holds a read for up to 4 s. Windows only asks which formats the
        // clipboard holds and macOS asks the pasteboard server, so they check in line.
        // The probe's handle is made on its own thread, from the window (`Send`), not its raw display.
        clipboard_probe: cfg!(target_os = "linux").then(|| Box::new(move || clipboard::system_clipboard(clip_probe)) as ClipboardProbeFactory),
        // Help → Discord / website / GitHub, the Discord button, About and Home links.
        open_url: Some(Box::new(|url: &str| {
            let _ = webbrowser::open(url);
        })),
        reveal: Some(Box::new(reveal)),
        // Links panel: Edit Original; Package: Show Package. Relink to Folder and Package pick folders.
        open_file: Some(Box::new(open_file)),
        // Edit Original of a raster image with Use System Defaults for Edit Original off (#5236).
        open_in_photocraft: Some(Box::new(open_in_photocraft)),
        pick_folder: Some(Box::new(move || pick_now(PickRequest::Folder, &p4).into_iter().next())),
        // Sample Color from Screen: the system's color sampler.
        #[cfg(target_os = "macos")]
        screen_color: Some(mac_screen_color::sampler(sampled_ui)),
        #[cfg(target_os = "linux")]
        screen_color: Some(linux_screen_color::sampler(sampled_ui, sampled_parent)),
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
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(&app).arg(path);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut c = {
        let mut c = std::process::Command::new(&app);
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

#[cfg(test)]
mod dialog_tests {
    use super::{CASE_SENSITIVE_FILTERS, DialogSettings, FilePick, PickRequest, dialog_for, fileio, open_extensions};

    /// The file types a dialog was given, by label.
    #[derive(Default)]
    struct Filters(Vec<(String, Vec<String>)>);

    impl DialogSettings for Filters {
        fn parent(self, _window: &winit::window::Window) -> Self {
            self
        }
        fn title(self, _title: &str) -> Self {
            self
        }
        fn filter(mut self, name: &str, exts: &[impl ToString]) -> Self {
            self.0.push((name.to_string(), exts.iter().map(ToString::to_string).collect()));
            self
        }
        fn folder(self, _folder: &str) -> Self {
            self
        }
        fn file_name(self, _name: &str) -> Self {
            self
        }
    }

    /// Does the case-sensitive pattern `*.{ext}` match `name`? `ext` may hold literal characters,
    /// `*` and `[..]` sets.
    fn matches(ext: &str, name: &str) -> bool {
        fn here(p: &[char], n: &[char]) -> bool {
            match p.split_first() {
                None => n.is_empty(),
                Some(('*', rest)) => (0..=n.len()).any(|i| here(rest, &n[i..])),
                Some(('[', rest)) => {
                    let Some(end) = rest.iter().position(|&c| c == ']') else { return false };
                    n.split_first().is_some_and(|(c, n)| rest[..end].contains(c) && here(&rest[end + 1..], n))
                }
                Some((c, rest)) => n.split_first().is_some_and(|(d, n)| d == c && here(rest, n)),
            }
        }
        let pattern: Vec<char> = format!("*.{ext}").chars().collect();
        here(&pattern, &name.chars().collect::<Vec<_>>())
    }

    /// Where filters match case-sensitively, every extension of File › Open and Place lists files
    /// whose extension is in capitals or mixed case, `image.DIB` and `image.Pns` included.
    #[test]
    fn open_and_place_filters_list_extensions_in_any_case() {
        let all = fileio::open_filters().chain(fileio::place_filters());
        for (label, exts) in all {
            let patterns = open_extensions(exts, true);
            for ext in exts {
                let mut mixed = ext.to_ascii_uppercase();
                mixed.replace_range(..1, &ext[..1]);
                for name in [format!("a.{ext}"), format!("a.{}", ext.to_ascii_uppercase()), format!("a.{mixed}")] {
                    assert!(patterns.iter().any(|p| matches(p, &name)), "{label}: {patterns:?} don't match {name}");
                }
            }
        }
        assert_eq!(open_extensions(&["dib", "jp2", "*"], true), ["dib", "[dD][iI][bB]", "jp2", "[jJ][pP]2", "*"]);
        assert_eq!(open_extensions(&["dib", "PNS"], false), ["dib", "PNS"]);
    }

    /// Open and Place dialogs get the any-case patterns on Linux and the BSDs; a save dialog's file
    /// types stay as listed.
    #[test]
    fn open_dialogs_get_any_case_patterns_where_filters_are_case_sensitive() {
        let dib = vec![FilePick::filter("BMP", &["bmp", "dib"])];
        let pick = FilePick { filters: dib.clone(), ..Default::default() };
        let expected = |any_case: bool| open_extensions(&["bmp", "dib"], any_case);
        let opened = dialog_for(Filters::default(), &PickRequest::Open(pick.clone())).0;
        assert_eq!(opened[0].1, expected(CASE_SENSITIVE_FILTERS));
        let saved = dialog_for(Filters::default(), &PickRequest::Save(pick)).0;
        assert_eq!(saved, dib);
        let placed = dialog_for(Filters::default(), &PickRequest::OpenMany).0;
        let first = fileio::place_filters().next().map(|(_, exts)| open_extensions(exts, CASE_SENSITIVE_FILTERS));
        assert_eq!(placed.first().map(|(_, exts)| exts.clone()), first);
    }
}
