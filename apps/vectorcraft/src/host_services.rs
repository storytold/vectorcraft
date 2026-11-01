#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
use super::portal_save;
#[cfg(target_os = "linux")]
use super::tablet;
#[cfg(all(windows, not(target_vendor = "win7")))]
use super::windows_stylus;
use super::{ClipboardProbeFactory, FilePick, PickRequest, Services, clipboard, cursor_theme, fileio, printing};
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

/// `d` set up for `request`: its title, file types, folder and suggested name.
fn dialog_for<D: DialogSettings>(d: D, request: &PickRequest) -> D {
    let pick = match request {
        PickRequest::Open(pick) | PickRequest::Save(pick) => pick,
        PickRequest::OpenMany => {
            return fileio::place_filters().fold(d.title(vectorcraft_ui_egui::i18n::t("Place")), |d, (name, exts)| {
                d.filter(vectorcraft_ui_egui::i18n::tr_ctx(vectorcraft_ui_egui::i18n::current(), "file-filter", name), exts)
            });
        }
        PickRequest::Folder => return d,
    };
    let d = pick.filters.iter().fold(d, |d, (name, exts)| {
        d.filter(vectorcraft_ui_egui::i18n::tr_ctx(vectorcraft_ui_egui::i18n::current(), "file-filter", name.as_str()), exts)
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
    make_folder(request);
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
    show_dialog(file_dialog(&request, parent), &request, parent)
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
            Some(picked) => Box::pin(async move { picked.recv().unwrap_or_default() }),
            None => {
                // Nothing is added to the sheet once rfd has begun it: an accessory view set then
                // made recent macOS versions dismiss the sheet at once (#1800).
                let picked = dialog.save_file();
                Box::pin(async move { picked.await.iter().map(path).collect() })
            }
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
    #[cfg(target_os = "macos")]
    let sampled_ui = ui.clone();
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
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(write_file)),
        exists: Some(Box::new(|p: &str| std::path::Path::new(p).exists())),
        // Background Save and Export write from a worker thread.
        write_shared: Some(std::sync::Arc::new(write_file)),
        // Native saves stream into the file as they are encoded.
        stream_shared: Some(std::sync::Arc::new(|p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
            fileio::write_file_with(p, fill).map_err(|e| e.to_string())
        })),
        // Every format Copy offers and Paste reads (menu-bar Paste never sees egui's Paste event).
        system_clipboard: Some(clipboard::system_clipboard(clip)),
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
        pick_folder: Some(Box::new(move || pick_now(PickRequest::Folder, &p4).into_iter().next())),
        // Sample Color from Screen: the system's color sampler.
        #[cfg(target_os = "macos")]
        screen_color: Some(mac_screen_color::sampler(sampled_ui)),
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
