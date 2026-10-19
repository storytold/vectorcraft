//! VectorCraft desktop app.
//!
//! Usage: `vectorcraft [--control <port> [--automation-read-root <dir>] [--automation-write-root <dir>]]
//! [--in-window-menus] [files…]`, `vectorcraft --version`, `vectorcraft --help` (or `-h`)
//!
//! `--in-window-menus` (or `VECTORCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar (`mac_menu`).
//!
//! `--control <port>` (or `VECTORCRAFT_CONTROL_PORT`; a port from 1 to 65535, or the app doesn't
//! start) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"ui.inspect","params":{}}` → `{"id":1,"ok":true,"result":…}`.
//! See `vectorcraft_ui_egui::control` for the methods. `--automation-read-root` and
//! `--automation-write-root` (or `VECTORCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`) confine the
//! files its requests read and write (`vectorcraft_engine::file_access`); the person at the
//! keyboard isn't confined.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(all(feature = "windows7", any(feature = "wgpu", feature = "accessibility")))]
compile_error!("windows7 requires --no-default-features (wgpu and accessibility must be disabled)");
#[cfg(all(windows, feature = "windows7", not(target_vendor = "win7")))]
compile_error!("windows7 requires --target x86_64-win7-windows-msvc; the ordinary Windows target still imports newer APIs");
#[cfg(all(target_vendor = "win7", not(feature = "windows7")))]
compile_error!("the win7 target requires --no-default-features --features windows7");

mod automation_args;
mod clipboard;
#[cfg(feature = "wgpu")]
mod compositor_gpu;
mod control_server;
mod cursor_theme;
#[cfg(feature = "wgpu")]
mod gpu;
mod logging;
#[cfg(target_os = "macos")]
mod mac_fonts;
#[cfg(target_os = "macos")]
mod mac_menu;
#[cfg(target_os = "macos")]
mod mac_save_format;
#[cfg(target_os = "macos")]
mod mac_screen_color;
#[cfg(target_os = "macos")]
mod mac_window;
#[cfg(target_os = "macos")]
mod open_documents;
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd", test))]
mod portal_save;
mod prefs_dir;
mod printing;
#[cfg(all(windows, not(target_vendor = "win7")))]
mod system_fonts;
mod window;
mod window_frame;
mod xwayland;

use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_ui_egui::graphics::GraphicsLoss;
use vectorcraft_ui_egui::picks::PickRequest;
use vectorcraft_ui_egui::{ClipboardProbeFactory, FilePick, Services, VectorcraftApp};

struct App {
    app: VectorcraftApp,
    /// Reported by wgpu when the window's graphics device is lost (a driver reset).
    graphics_loss: GraphicsLoss,
    /// The graphics device was lost and the unsaved changes are kept for Data Recovery.
    graphics_lost: bool,
    /// Frames the UI has run (see [`end_before_teardown`]).
    frames: u64,
    /// Keeps the custom title bar's window borderless (Windows, #1758).
    window_frame: Option<window_frame::WindowFrameFn>,
}

impl eframe::App for App {
    // Each frame runs unconfined: it is the person at the keyboard's. The app confines what the
    // control channel asks for, and the frames carrying input it injected, to the automation roots
    // (`VectorcraftApp::with_automation_roots`), which are in force elsewhere in the process.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        file_access::unconfined(|| self.logic_frame(ctx, frame));
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // winit gives the window its caption back when it changes the window's flags (maximize,
        // restore, …): take it off again before the frame is laid out (#1758).
        if let Some(window_frame) = self.window_frame.as_mut() {
            window_frame();
        }
        #[cfg(target_os = "macos")]
        if self.app.services.native_menu.is_some() {
            mac_menu::raw_input_hook(raw);
        }
        file_access::unconfined(|| self.app.raw_input_hook(raw));
    }
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            let bar = vectorcraft_ui_egui::chrome::APP_BAR_HEIGHT;
            let inset = mac_window::center_buttons(frame, ui.ctx().zoom_factor(), bar);
            self.app.titlebar_inset = inset.unwrap_or(mac_window::DEFAULT_INSET);
        }
        file_access::unconfined(|| self.app.ui(ui));
        // Preferences are written when confirmed, not only at exit: a session ended any other way
        // (the system shutting down, the process ended or crashing) used to lose them (#1747).
        if self.app.take_prefs_changed() {
            file_access::unconfined(|| save_prefs(&self.app));
        }
        // Not while a file dialog's sheet has the keyboard: its name field's composition would go.
        #[cfg(target_os = "macos")]
        if !self.app.file_dialog_open() && self.app.take_ime_discard() {
            discard_marked_text();
        }
    }
    #[cfg(not(feature = "windows7"))]
    fn on_exit(&mut self) {
        file_access::unconfined(|| save_prefs(&self.app));
        end_before_teardown(self.frames);
    }
    #[cfg(feature = "windows7")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        file_access::unconfined(|| save_prefs(&self.app));
    }
}

impl App {
    fn logic_frame(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames = ctx.cumulative_frame_nr();
        if let Some(why) = self.graphics_loss.take() {
            // eframe can't give a window a new device: the user saves and starts again.
            self.graphics_lost = self.app.graphics_lost(&why);
            self.app.status(if self.graphics_lost {
                "The graphics device was lost: unsaved changes are kept for Data Recovery. Save your documents and restart VectorCraft"
            } else {
                "The graphics device was lost: save your documents and restart VectorCraft"
            });
        }
        if self.graphics_lost && ctx.input(|i| i.viewport().close_requested()) {
            // The window can't show the Save Changes question: it closes, and the next launch
            // offers the changes back.
            vectorcraft_ui_egui::background::wait_all(&mut self.app);
            return;
        }
        // Files from the Finder and the Dock wait while a file dialog is open: what asked for it runs
        // again on its document when it answers.
        #[cfg(target_os = "macos")]
        if !self.app.file_dialog_open() {
            // Files from the Finder and the Dock wait while a file dialog is open: what asked for it runs
            // again on its document when it answers.
            if !self.app.file_dialog_open() {
                open_files(&mut self.app, open_documents::take());
            }
        }
        self.app.logic(ctx);
        window::track(ctx, &mut self.app.ui.window);
        if self.app.ui.status == "quit" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// On macOS with accessibility, end the process once the app has saved what it keeps on quitting,
/// before eframe tears the window down (#661). AccessKit then gives the window's content view back
/// its own class, which AppKit's Touch Bar support was observing it under, and AppKit aborts the
/// app as it quits on Macs with a Touch Bar. Nothing after the window is needed on a normal quit.
/// Not while the window is still starting up (`frames` below [`gpu::STARTUP_FRAMES`]): a graphics
/// failure then starts the app again on another adapter (#651).
#[allow(unused_variables)]
fn end_before_teardown(frames: u64) {
    #[cfg(all(target_os = "macos", feature = "accessibility", feature = "wgpu"))]
    if frames >= gpu::STARTUP_FRAMES {
        log::info!("quitting");
        log::logger().flush();
        std::process::exit(0);
    }
}

/// Open files handed to the app (command line, macOS Finder and Dock) as documents. A file that
/// can't be opened is reported to the user and on stderr; the others still open.
fn open_files(app: &mut VectorcraftApp, files: Vec<String>) {
    for f in files {
        if let Err(e) = vectorcraft_ui_egui::io::open_reporting(app, &f) {
            eprintln!("vectorcraft: {f}: {e}");
        }
    }
}

/// Tell the macOS input method to drop its composition (the Type tool kept the marked text as
/// typed). winit's IME toggle only clears its own copy, so the IME would type it again.
#[cfg(target_os = "macos")]
fn discard_marked_text() {
    if let Some(mtm) = objc2::MainThreadMarker::new()
        && let Some(ic) = objc2_app_kit::NSTextInputContext::currentInputContext(mtm)
    {
        ic.discardMarkedText();
    }
}

/// Where UI preferences live: ~/Library/Application Support/VectorCraft (macOS),
/// %APPDATA%\VectorCraft (Windows), $XDG_CONFIG_HOME or ~/.config/vectorcraft (Linux).
fn prefs_path() -> Option<std::path::PathBuf> {
    prefs_dir::prefs_path_for("VectorCraft", "vectorcraft")
}

/// The same place under the project's former name (DrawCraft): read once if there are no
/// VectorCraft preferences yet, so settings survive the rename.
fn legacy_prefs_path() -> Option<std::path::PathBuf> {
    prefs_dir::prefs_path_for("DrawCraft", "drawcraft")
}

/// Where the log files live: `logs` in the preferences folder (see `logging`).
fn log_dir() -> Option<std::path::PathBuf> {
    Some(prefs_path()?.parent()?.join("logs"))
}

/// Runs without preferences (`VECTORCRAFT_NO_PREFS`, agents' test runs) neither read nor write them.
fn prefs_enabled() -> bool {
    std::env::var_os("VECTORCRAFT_NO_PREFS").is_none()
}

/// The saved UI preferences, read before the window opens (they hold its size and position).
fn read_prefs() -> Option<vectorcraft_ui_egui::UiState> {
    if !prefs_enabled() {
        return None;
    }
    let bytes = prefs_path().and_then(|p| std::fs::read(p).ok()).or_else(|| legacy_prefs_path().and_then(|p| std::fs::read(p).ok()))?;
    decode_saved_prefs(&bytes)
}

/// Recover only optional persisted docking metadata. Runtime UiState and command decoding
/// remains strict, and an unreadable workspace layout cannot discard unrelated preferences.
fn decode_saved_prefs(bytes: &[u8]) -> Option<vectorcraft_ui_egui::UiState> {
    use serde_json::{Map, Value, json};

    fn recover(object: &mut Map<String, Value>, hidden_key: &str) {
        if let Some(layout) = object.get("docking")
            && serde_json::from_value::<vectorcraft_ui_egui::UiState>(json!({"docking": layout})).is_err()
        {
            object.remove("docking");
        }
        if let Some(hidden) = object.get_mut(hidden_key) {
            if let Value::Object(entries) = hidden {
                entries.retain(|panel, location| {
                    serde_json::from_value::<vectorcraft_ui_egui::UiState>(json!({"docking_hidden": {panel: location}})).is_ok()
                });
            } else {
                object.remove(hidden_key);
            }
        }
    }

    let mut value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object_mut()?;
    recover(object, "docking_hidden");
    if let Some(Value::Array(workspaces)) = object.get_mut("custom_workspaces") {
        for workspace in workspaces {
            if let Some(object) = workspace.as_object_mut() {
                recover(object, "dockingHidden");
            }
        }
    }
    serde_json::from_value(value).ok()
}

fn load_prefs(app: &mut VectorcraftApp, saved: Option<vectorcraft_ui_egui::UiState>) {
    if !prefs_enabled() {
        return;
    }
    if let Some(ui) = saved {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(app);
}

fn save_prefs(app: &VectorcraftApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = prefs_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
        let mut ui = app.ui.clone();
        ui.engine_prefs = app.session.prefs.to_json();
        if let Ok(bytes) = serde_json::to_vec_pretty(&ui) {
            // Preferences are best effort: a failed write keeps the previous file.
            let _ = fileio::write_atomic(&p, &bytes);
        }
    }
}

/// The window file dialogs belong to.
type Parent = Option<std::sync::Arc<winit::window::Window>>;

/// What rfd's blocking and asynchronous file dialogs are both set up with.
trait DialogSettings: Sized {
    fn parent(self, window: &winit::window::Window) -> Self;
    fn title(self, title: &str) -> Self;
    fn filter(self, name: &str, exts: &[&str]) -> Self;
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
            fn filter(self, name: &str, exts: &[&str]) -> Self {
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
    let d = pick
        .filters
        .iter()
        .fold(d, |d, (name, exts)| d.filter(vectorcraft_ui_egui::i18n::tr_ctx(vectorcraft_ui_egui::i18n::current(), "file-filter", name), exts));
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
fn services(parent: Parent, ui: egui::Context) -> Services {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let wayland = parent.as_ref().and_then(|w| w.window_handle().ok()).is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)));
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
        system_clipboard: Some(clipboard::system_clipboard()),
        // Linux checks whether Paste has something to take on a background thread: an X11 clipboard
        // owner that never answers holds a read for up to 4 s. Windows only asks which formats the
        // clipboard holds and macOS asks the pasteboard server, so they check in line.
        clipboard_probe: cfg!(target_os = "linux").then(|| Box::new(clipboard::system_clipboard) as ClipboardProbeFactory),
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

/// The window, Dock, taskbar and app-switcher icon (`assets/app-icon/`, see its README). macOS gets
/// the version with Apple's transparent margin; elsewhere the full-bleed tile. The app ID matches
/// `packaging/linux/ai.storyteller.vectorcraft.desktop` so Wayland docks find the launcher icon.
fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/vectorcraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.vectorcraft.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Windows shows a window's big icon (`ICON_BIG`) in the taskbar and Alt+Tab; eframe sets only the
/// small one (the title bar's), so the taskbar showed a generic icon. Set the big one too.
#[cfg(windows)]
fn set_taskbar_icon(w: &winit::window::Window) {
    use winit::platform::windows::WindowExtWindows as _;
    let icon = app_icon();
    // An icon that can't be made leaves the generic one: nothing else depends on it.
    if let Ok(i) = winit::window::Icon::from_rgba(icon.rgba, icon.width, icon.height) {
        w.set_taskbar_icon(Some(i));
    }
}

/// "name (backend, kind)" of the adapter the window renders with, for Help › About and bug reports.
#[cfg(feature = "wgpu")]
fn adapter_summary(info: &eframe::wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name.trim(), info.backend, info.device_type)
}

/// Windows and Linux: no OS title bar; the app bar is the title bar (`vectorcraft_ui_egui::titlebar`).
/// macOS keeps its traffic lights over the integrated title strip.
const CUSTOM_TITLEBAR: bool = !cfg!(target_os = "macos");

/// Whether this start draws its own title bar: Windows and Linux do, unless Preferences ›
/// User Interface › System Title Bar asks for the system's. Read from the saved preferences
/// before the window opens; a missing or unreadable value keeps the default.
fn custom_titlebar(saved: Option<&vectorcraft_ui_egui::UiState>) -> bool {
    CUSTOM_TITLEBAR && !saved.map(|ui| ui.engine_prefs.get("systemTitleBar").and_then(serde_json::Value::as_bool).unwrap_or(false)).unwrap_or(false)
}

fn native_options(custom_titlebar: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VectorCraft")
            .with_inner_size(window::DEFAULT_SIZE)
            .with_min_inner_size(window::MIN_SIZE)
            .with_drag_and_drop(true)
            .with_decorations(!custom_titlebar)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(app_icon())
            .with_app_id("ai.storyteller.vectorcraft"),
        #[cfg(feature = "windows7")]
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    }
}

/// `vectorcraft --help`. Headless work (convert, run, MCP) is `vectorcraft-cli`'s.
const USAGE: &str = "\
Usage: vectorcraft [options] [files…]

Opens the VectorCraft desktop app with the given files.

Options:
  --control <port>               Start the localhost JSON-lines control server on <port>
                                 (or VECTORCRAFT_CONTROL_PORT)
  --automation-read-root <dir>   Confine the files control requests read to <dir>
  --automation-write-root <dir>  Confine the files control requests write to <dir>
  --in-window-menus              Keep the menus inside the window on macOS
                                 (or VECTORCRAFT_IN_WINDOW_MENUS=1)
  --version                      Print the version and exit
  -h, --help                     Print this help and exit

Without a window (converting, scripting, the MCP server), use vectorcraft-cli:
  vectorcraft-cli --help
";

fn main() -> std::process::ExitCode {
    // Before any thread starts: the pointer must find a cursor theme on Wayland (#2450).
    #[cfg(target_os = "linux")]
    cursor_theme::apply();
    // Validate roots before logging, font scans, preferences or the window: a missing value
    // must not clear an environment root and start the control channel without confinement.
    let mut control = automation_args::ControlArg::default();
    let root_env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let mut roots_args =
        automation_args::AutomationArgs::new(root_env("VECTORCRAFT_AUTOMATION_READ_ROOT"), root_env("VECTORCRAFT_AUTOMATION_WRITE_ROOT"));
    let mut files = Vec::new();
    let mut in_window_menus = std::env::var_os("VECTORCRAFT_IN_WINDOW_MENUS").is_some_and(|v| !v.is_empty() && v != "0");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match roots_args.consume(&a, &mut args).and_then(|root| if root { Ok(true) } else { control.consume(&a, &mut args) }) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        match a.as_str() {
            "--in-window-menus" => in_window_menus = true,
            "--version" => {
                println!("vectorcraft {}", env!("CARGO_PKG_VERSION"));
                return std::process::ExitCode::SUCCESS;
            }
            // Before anything starts, like `--version`: from a terminal or an agent's shell the app
            // used to open (or hang without a display) instead (#2110).
            "--help" | "-h" => {
                // Not `print!`, which panics when standard output is closed (`| head -0`):
                // there is nothing left to tell then.
                let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
                return std::process::ExitCode::SUCCESS;
            }
            _ => files.push(a),
        }
    }
    let automation_args::AutomationArgs { read_root, write_root, .. } = roots_args;
    // After the arguments: `--control` and `--version` win over a bad environment value.
    let control_port = match control.port {
        Some(port) => Some(port),
        None => match automation_args::env_control_port(std::env::var_os("VECTORCRAFT_CONTROL_PORT")) {
            Ok(port) => port,
            Err(e) => {
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
    };
    // Before desktop services, so every start-up warning is recorded (`logging`).
    let logger = logging::install();
    vectorcraft_ui_egui::i18n::detect_system_lang_in_background();
    // Before the first font scan (the app's start): the fonts font services load (#579).
    #[cfg(all(windows, not(target_vendor = "win7")))]
    system_fonts::install();
    #[cfg(target_os = "macos")]
    mac_fonts::install();
    // VectorCraft's own Fonts folder, which text.addFontFiles copies fonts into.
    prefs_dir::install_fonts();
    // The log file lives in the settings directory, next to the preferences; opened after the
    // arguments, so `--version` leaves no file behind. Records logged until now are written to it
    // first. Runs without preferences (agents' test runs) log to standard error only, so they
    // don't rotate away the user's own logs.
    if let Some(logger) = logger {
        match log_dir().filter(|_| prefs_enabled()) {
            Some(dir) => match logger.attach_dir(&dir) {
                Ok(path) => log::info!("VectorCraft {}, log file {}", env!("CARGO_PKG_VERSION"), path.display()),
                // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.no_file(),
        }
    }
    // The roots confine the control channel: a bad one keeps the app from starting rather than
    // leaving agents unconfined. Without a control port there is nothing to confine.
    let roots = if control_port.is_none() {
        if read_root.is_some() || write_root.is_some() {
            log::warn!("--automation-read-root and --automation-write-root confine the control channel: ignored without --control");
        }
        None
    } else {
        match AutomationRoots::new(read_root.as_deref(), write_root.as_deref()) {
            Ok(roots) => roots,
            Err(e) => {
                log::error!("{e}");
                eprintln!("vectorcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    };
    let saved = read_prefs();
    let saved_window = saved.as_ref().and_then(|ui| ui.window);
    #[cfg(feature = "wgpu")]
    let gpu_pref = saved.as_ref().and_then(|ui| ui.engine_prefs.get("gpuPreference")).and_then(serde_json::Value::as_str);
    #[cfg(feature = "wgpu")]
    let power_env = eframe::wgpu::PowerPreference::from_env();
    #[cfg(feature = "wgpu")]
    let power = gpu::power_preference(gpu_pref, power_env);
    // Automatic draws on the GPU that drives the (primary) display: a GPU without a monitor reset
    // its driver and took every monitor down (pdfcraft#378). Logged first: the first question in
    // every black-window report.
    #[cfg(feature = "wgpu")]
    let displays = gpu::preferred_displays(gpu_pref, power_env);
    // Under Wayland, the GPU the compositor draws with comes even before those (src/compositor_gpu.rs).
    #[cfg(feature = "wgpu")]
    let compositor = gpu::preferred_compositor(gpu_pref, power_env);
    #[cfg(feature = "wgpu")]
    let chosen_gpu = !gpu::automatic(gpu_pref, power_env);
    #[cfg(feature = "wgpu")]
    if !chosen_gpu {
        let listed: Vec<String> = displays.iter().map(ToString::to_string).collect();
        log::info!("display GPUs (PCI vendor:device): {}", if listed.is_empty() { "unknown".to_string() } else { listed.join(", ") });
    } else {
        log::info!("graphics processor chosen by the user ({power:?}): which GPU drives the display isn't considered");
    }
    #[cfg(feature = "wgpu")]
    let startup = std::sync::Arc::new(gpu::Startup::default());
    // A driver that crashes the process while the window starts leaves this start recorded in the
    // log folder; the next start leaves that adapter out (#2243). Runs without preferences, which
    // write no log file, don't record their starts.
    #[cfg(feature = "wgpu")]
    let left_out = match log_dir().filter(|_| prefs_enabled()) {
        Some(dir) => startup.track(&dir, gpu_pref, gpu::explicit_choice()),
        None => gpu::LeftOut::default(),
    };
    #[cfg(feature = "wgpu")]
    let crash = left_out.crash.clone();
    let custom_titlebar = custom_titlebar(saved.as_ref());
    let options = native_options(custom_titlebar);
    // winit picks Wayland whenever `WAYLAND_DISPLAY` is set; Preferences › Performance › Linux
    // Display Server › X11 opens the window through XWayland instead, where file drops work (#1151),
    // and X11 (XWayland) When a Pen Is Attached does so when a pen is attached, where the pen draws (#2028).
    #[cfg(target_os = "linux")]
    let options = {
        let mut options = options;
        let pref = saved.as_ref().and_then(|ui| ui.engine_prefs.get("linuxDisplayServer")).and_then(serde_json::Value::as_str);
        if xwayland::wanted(pref, |name| std::env::var_os(name), xwayland::pen_attached) {
            if xwayland::x11_libraries_found() {
                if pref == Some("x11WithPen") {
                    log::info!("pen found: opening the window through XWayland (Preferences › Performance › Linux Display Server)");
                } else {
                    log::info!("opening the window through XWayland (Preferences › Performance › Linux Display Server)");
                }
                use winit::platform::x11::EventLoopBuilderExtX11 as _;
                options.event_loop_builder = Some(Box::new(|builder| {
                    builder.with_x11();
                }));
            } else {
                log::warn!("Linux Display Server is X11, but the X11 libraries aren't all installed: opening the window on Wayland");
            }
        }
        options
    };
    #[cfg(feature = "wgpu")]
    let options = {
        let mut options = options;
        // One frame queued, not two: the canvas is rasterized on the CPU and the GPU only
        // composites it, so the window answers the pointer a frame sooner (#444).
        options.wgpu_options.surface = eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY;
        // Only adapters that can show the window, in the order `gpu` gives (#306, #502).
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            create.native_adapter_selector = Some(gpu::selector(power, displays, compositor, left_out.adapters.clone(), startup.clone()));
            // Nothing draws or dispatches indirectly, so wgpu's check of indirect arguments only
            // costs a compute shader at start-up, one some drivers can't compile (OCLP-patched
            // Metal on an Iris Pro, #651). `WGPU_VALIDATION_INDIRECT_CALL=1` turns it back on.
            create.instance_descriptor.flags =
                (eframe::wgpu::InstanceFlags::from_build_config() - eframe::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
            create.instance_descriptor.backends =
                left_out.backends(gpu::backends(create.instance_descriptor.backends, std::env::var_os("WGPU_BACKEND").is_some()));
            // DX12 shaders compile with FXC, part of Windows: wgpu's default loads whichever
            // dxcompiler.dll the search path finds, and one without its dxil.dll fails every shader,
            // losing the device before the first frame (#1194). `WGPU_DX12_COMPILER` picks another.
            create.instance_descriptor.backend_options.dx12.shader_compiler =
                eframe::wgpu::Dx12Compiler::from_env().unwrap_or(eframe::wgpu::Dx12Compiler::Fxc);
            // The window's surface may be as large as the adapter allows, not eframe's fixed
            // 8192 px: a window stretched over several monitors, or grown by a display scale
            // change, otherwise fails to configure its surface and the app closes (#1459).
            let device = create.device_descriptor.clone();
            create.device_descriptor = std::sync::Arc::new(move |adapter: &eframe::wgpu::Adapter| {
                let mut descriptor = device(adapter);
                let limit = &mut descriptor.required_limits.max_texture_dimension_2d;
                *limit = gpu::surface_limit(*limit, adapter.limits().max_texture_dimension_2d);
                descriptor
            });
        }
        options
    };
    #[cfg(feature = "wgpu")]
    gpu::watch_panics();
    #[cfg(feature = "wgpu")]
    gpu::watch_first_frame(startup.clone());
    // Files opened from Finder and the Dock arrive as events, not arguments.
    #[cfg(target_os = "macos")]
    open_documents::install();
    #[cfg(feature = "wgpu")]
    let created = startup.clone();
    // A panic that ends the window (egui-wgpu's own, #502) is caught here: never a crash.
    let outcome = vectorcraft_engine::guard::catch_panic(|| {
        eframe::run_native(
            "VectorCraft",
            options,
            Box::new(move |cc| {
                let mut session = Session::new();
                // The profiles loaded before come back before the preferences name any of them.
                prefs_dir::attach_profile_store(&mut session);
                prefs_dir::attach_plugin_store(&mut session);
                let mut app = VectorcraftApp::new(session, services(cc.winit_window().cloned(), cc.egui_ctx.clone()));
                load_prefs(&mut app, saved);
                // The saved PDF presets live in their own folder (also read by vectorcraft-cli),
                // which takes over from the copy the preferences file used to hold.
                prefs_dir::attach_pdf_preset_store(&mut app.session);
                // Fit the window to its monitor, or put it back where it was (still hidden).
                if let Some(w) = cc.winit_window() {
                    app.ui.window = Some(window::restore(w, saved_window));
                    #[cfg(windows)]
                    set_taskbar_icon(w);
                }
                // User Defined swatch and graphic style libraries live next to the preferences.
                let swatches = prefs_path().and_then(|p| Some(p.parent()?.join("Swatches").to_string_lossy().to_string()));
                app.session.swatch_libraries.set_user_dir(swatches);
                let styles = prefs_path().and_then(|p| Some(p.parent()?.join("Graphic Styles").to_string_lossy().to_string()));
                app.session.style_libraries.set_user_dir(styles);
                // With the User Defined libraries listed, the persistent library panels reopen.
                vectorcraft_ui_egui::panels::library_panel::restore(&mut app);
                // So do the Libraries panel's libraries; runs without preferences (agents' test
                // runs) keep theirs for the session only, never touching the user's.
                if prefs_enabled() {
                    let libraries = prefs_path().and_then(|p| Some(p.parent()?.join("Libraries").to_string_lossy().to_string()));
                    app.session.libraries.set_dir(libraries);
                }
                // Data Recovery copies live next to the preferences too (none for runs without
                // preferences, such as agents' test runs, unless the recoveryFolder preference is set).
                if std::env::var_os("VECTORCRAFT_NO_PREFS").is_none() {
                    let recovery = prefs_path().and_then(|p| Some(p.parent()?.join("Data Recovery").to_string_lossy().to_string()));
                    app.session.recovery.set_default_folder(recovery);
                }
                let graphics_loss = GraphicsLoss::default();
                #[cfg(feature = "wgpu")]
                if let Some(rs) = &cc.wgpu_render_state {
                    let summary = adapter_summary(&rs.adapter.get_info());
                    log::info!("rendering with {summary} (power preference {power:?})");
                    created.created(&cc.egui_ctx);
                    if !gpu::skipped().is_empty() {
                        // A processor chosen in Settings can be the one that can't show the window.
                        let hint = if chosen_gpu {
                            " (Settings › Performance › Graphics Processor › Automatic uses the one that drives your display)"
                        } else {
                            ""
                        };
                        app.status(format!(
                            "The graphics processor tried first couldn't show the window, so VectorCraft started again on {summary}{hint}"
                        ));
                    } else if let Some(crash) = &crash {
                        app.status(match crash {
                            gpu::Crash::Adapter { name, .. } => format!(
                                "The last start of VectorCraft ended on {name} before its window appeared, so it started on {summary}. To try that graphics processor again, change Preferences › Performance › Graphics Processor or start with WGPU_BACKEND set"
                            ),
                            gpu::Crash::Listing => format!(
                                "The last start of VectorCraft ended while it listed the graphics processors, so it started on {summary} with OpenGL alone. To try the other graphics backends again, change Preferences › Performance › Graphics Processor or start with WGPU_BACKEND set"
                            ),
                        });
                    }
                    app.graphics_adapter = Some(summary);
                    let (loss, ctx) = (graphics_loss.clone(), cc.egui_ctx.clone());
                    rs.device.set_device_lost_callback(move |reason, msg| loss.report(&ctx, format!("{reason:?}: {msg}")));
                }
                #[cfg(feature = "windows7")]
                {
                    app.graphics_adapter = Some("OpenGL (Windows 7 compatibility)".into());
                }
                app.custom_titlebar = custom_titlebar;
                if let Some(port) = control_port {
                    let rx = control_server::start(port, cc.egui_ctx.clone());
                    app = app.with_control(rx).with_automation_roots(roots.clone());
                    if let Some(roots) = &roots {
                        // Everywhere the app doesn't say whose work it is (worker threads), the
                        // roots hold: unconfined is only what the person at the keyboard does.
                        file_access::confine_process(roots.clone());
                        log::info!("control channel confined to the automation roots: read {:?}, write {:?}", roots.read_root(), roots.write_root());
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    open_documents::set_ui(&cc.egui_ctx);
                    // The macOS menu bar, installed now so winit's default menu doesn't stay up.
                    if !in_window_menus {
                        app.services.native_menu = mac_menu::install(&cc.egui_ctx, &app);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = in_window_menus;
                file_access::unconfined(|| open_files(&mut app, files));
                // The first frame is due from now (`gpu::watch_first_frame`).
                #[cfg(feature = "wgpu")]
                created.ready();
                // The content and the pointer stay aligned on Windows (#1758).
                let window_frame = window_frame::service(cc, custom_titlebar);
                Ok(Box::new(App { app, graphics_loss, graphics_lost: false, frames: 0, window_frame }))
            }),
        )
    });
    #[cfg(feature = "wgpu")]
    let outcome = gpu::finish(outcome, &startup);
    #[cfg(not(feature = "wgpu"))]
    let outcome = outcome.and_then(|r| r.map_err(|e| e.to_string()));
    match outcome {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            log::error!("VectorCraft stopped: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(all(test, feature = "wgpu"))]
mod tests {
    use super::*;

    fn ui_state_with_prefs(engine_prefs: serde_json::Value) -> vectorcraft_ui_egui::UiState {
        vectorcraft_ui_egui::UiState { engine_prefs, ..Default::default() }
    }

    #[test]
    fn unreadable_saved_docking_keeps_preferences_and_every_workspace() {
        use serde_json::json;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("window.panel.float", json!({"panel":"layers"})).unwrap();
        app.ui.toolbar = false;
        app.ui.engine_prefs = json!({"interfaceLanguage":"fr", "uiBrightness":"light"});
        app.ui.custom_workspaces =
            vec![vectorcraft_ui_egui::workspaces::capture(&app.ui, "Future"), vectorcraft_ui_egui::workspaces::capture(&app.ui, "Valid")];
        let original = serde_json::to_value(&app.ui).unwrap();
        let valid_location = original["docking_hidden"]["layers"].clone();
        assert!(valid_location.is_object());
        for invalid_layout in [json!({"root":{"FutureNode":{}},"floating":[]}), json!(false)] {
            for invalid_hidden in [json!({"layers":{"placement":{"FuturePlacement":{}}},"swatches":valid_location}), json!(["malformed"])] {
                let mut raw = original.clone();
                raw["docking"] = invalid_layout.clone();
                raw["docking_hidden"] = invalid_hidden.clone();
                raw["custom_workspaces"][0]["docking"] = invalid_layout.clone();
                raw["custom_workspaces"][0]["dockingHidden"] = invalid_hidden.clone();
                assert!(serde_json::from_value::<vectorcraft_ui_egui::UiState>(raw.clone()).is_err(), "runtime serde stays strict");
                let loaded = decode_saved_prefs(&serde_json::to_vec(&raw).unwrap()).unwrap();
                assert_eq!(loaded.engine_prefs, app.ui.engine_prefs);
                assert!(!loaded.toolbar);
                assert!(loaded.docking.is_none());
                assert!(!loaded.docking_hidden.contains_key("layers"));
                if invalid_hidden.is_object() {
                    assert_eq!(serde_json::to_value(loaded.docking_hidden.get("swatches")).unwrap(), valid_location);
                } else {
                    assert!(loaded.docking_hidden.is_empty());
                }
                assert_eq!(loaded.custom_workspaces.len(), 2);
                assert_eq!(loaded.custom_workspaces[0].name, "Future");
                assert!(loaded.custom_workspaces[0].docking.is_none());
                assert!(!loaded.custom_workspaces[0].docking_hidden.contains_key("layers"));
                assert_eq!(serde_json::to_value(&loaded.custom_workspaces[1]).unwrap(), original["custom_workspaces"][1]);
            }
        }
        let roundtrip = decode_saved_prefs(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(roundtrip).unwrap(), original);
        assert!(
            decode_saved_prefs(&serde_json::to_vec(&json!({"toolbar":"invalid"})).unwrap()).is_none(),
            "unrelated fields retain normal strict parsing"
        );
    }

    #[test]
    fn the_system_title_bar_preference_keeps_the_window_decorations() {
        // Preferences › User Interface › System Title Bar gives the window back its system
        // decorations on Windows and Linux; macOS always has them.
        assert_eq!(super::custom_titlebar(None), super::CUSTOM_TITLEBAR, "no preferences: the default");
        let off = ui_state_with_prefs(serde_json::json!({"systemTitleBar": false}));
        assert_eq!(super::custom_titlebar(Some(&off)), super::CUSTOM_TITLEBAR);
        let on = ui_state_with_prefs(serde_json::json!({"systemTitleBar": true}));
        assert!(!super::custom_titlebar(Some(&on)));
        assert_eq!(super::native_options(false).viewport.decorations, Some(true));
        if super::CUSTOM_TITLEBAR {
            assert_eq!(super::native_options(true).viewport.decorations, Some(false));
        }
    }

    /// The file extensions the macOS bundle declares: its document types and its own exported type.
    fn plist_extensions(plist: &str) -> Vec<&str> {
        ["<key>CFBundleTypeExtensions</key>", "<key>public.filename-extension</key>"]
            .iter()
            .flat_map(|key| plist.split(key).skip(1))
            .filter_map(|rest| rest.split("</array>").next())
            .flat_map(|array| array.split("<string>").skip(1))
            .filter_map(|s| s.split("</string>").next())
            .collect()
    }

    /// Finder offers the app for every file File › Open reads (#295, #354) but those with an
    /// extension in `UNASSOCIATED_EXTS`, takes over no other app's files, and hands them to the app
    /// rather than to AppKit's document machinery.
    #[test]
    fn the_macos_bundle_opens_every_readable_format() {
        let plist = include_str!("../../../packaging/macos/Info.plist.in");
        let declared = plist_extensions(plist);
        for e in fileio::OPEN_EXTS.iter().filter(|e| !fileio::UNASSOCIATED_EXTS.contains(e)) {
            assert!(declared.contains(e), "Info.plist.in doesn't declare .{e}");
        }
        for e in &declared {
            assert!(fileio::OPEN_EXTS.contains(e), "Info.plist.in declares .{e}, which the app doesn't open");
        }
        assert!(!plist.contains("<key>NSDocumentClass</key>"), "not an NSDocument app: AppKit would refuse the files");
        let types = plist.matches("<key>CFBundleTypeName</key>").count();
        assert!(types > 1 && plist.matches("<key>LSHandlerRank</key>").count() == types, "every document type has a rank");
        assert_eq!(plist.matches("<string>Owner</string>").count(), 2, "only VectorCraft documents and templates are owned");
    }

    /// The MIME types a Linux desktop gives each extension in `OPEN_EXTS` but `UNASSOCIATED_EXTS`
    /// (one row per type): shared-mime-info's, or one that `ai.storyteller.vectorcraft.mime.xml`
    /// declares.
    const LINUX_TYPES: &[(&str, &str)] = &[
        ("vectorcraft", "application/x-vectorcraft"),
        ("drawcraft", "application/x-vectorcraft"),
        ("svg", "image/svg+xml"),
        ("svgz", "image/svg+xml-compressed"),
        ("pdf", "application/pdf"),
        ("ai", "application/illustrator"), // brand-ok: shared-mime-info's type
        ("ait", "application/x-ai-template"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("tif", "image/tiff"),
        ("tiff", "image/tiff"),
        ("bmp", "image/bmp"),
        ("vctemplate", "application/x-vectorcraft-template"),
        ("dxf", "image/vnd.dxf"),
        ("emf", "image/emf"),
        ("wmf", "image/wmf"),
        ("eps", "image/x-eps"),
        ("af", "application/x-affinity"),
        ("afdesign", "application/x-affinity"),
        ("afpub", "application/x-affinity"),
    ];

    /// The types in `LINUX_TYPES` that shared-mime-info defines. The MIME package declares every
    /// other one.
    const LINUX_SHARED_TYPES: &[&str] = &[
        "image/svg+xml",
        "image/svg+xml-compressed",
        "application/pdf",
        "application/illustrator", // brand-ok: shared-mime-info's type
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/tiff",
        "image/bmp",
        "image/vnd.dxf",
        "image/emf",
        "image/wmf",
        "image/x-eps",
    ];

    /// Extensions in `LINUX_TYPES` whose type the MIME package matches by file content and not by a
    /// glob, with the package's `<magic>` rule for that type (its lines joined, indentation removed).
    const LINUX_BY_CONTENT: &[(&str, &str)] = &[(
        "af",
        // `MAGIC` at offset 0 and `DOCUMENT_KIND` at offset 8 (crates/affinity/src/container.rs).
        r#"<magic><match type="string" offset="0" value="\x00\xff\x4b\x41"><match type="string" offset="8" value="nsrP"/></match></magic>"#,
    )];

    /// The desktop entry lists a MIME type for every extension in `OPEN_EXTS` but
    /// `UNASSOCIATED_EXTS`, and the app's MIME package declares the types shared-mime-info doesn't
    /// have.
    #[test]
    fn the_linux_desktop_entry_opens_every_readable_format() {
        let desktop = include_str!("../../../packaging/linux/ai.storyteller.vectorcraft.desktop");
        let mime = include_str!("../../../packaging/linux/ai.storyteller.vectorcraft.mime.xml");
        let listed: Vec<&str> =
            desktop.lines().find_map(|l| l.strip_prefix("MimeType=")).unwrap_or_default().split(';').filter(|t| !t.is_empty()).collect();
        // Each type the MIME package declares, with the extensions of its globs and its `<magic>`
        // rule (empty when it has none) in the form `LINUX_BY_CONTENT` gives.
        let declared: Vec<(&str, Vec<&str>, String)> = mime
            .split("<mime-type type=\"")
            .skip(1)
            .map(|block| {
                let body = block.split("</mime-type>").next().unwrap_or_default();
                let globs = body.split("<glob pattern=\"*.").skip(1).filter_map(|g| g.split('"').next()).collect();
                let magic = match (body.find("<magic"), body.find("</magic>")) {
                    (Some(start), Some(end)) => body[start..end + "</magic>".len()].lines().map(str::trim).collect(),
                    _ => String::new(),
                };
                (block.split('"').next().unwrap_or_default(), globs, magic)
            })
            .collect();
        let mut associated: Vec<&str> = fileio::OPEN_EXTS.iter().copied().filter(|e| !fileio::UNASSOCIATED_EXTS.contains(e)).collect();
        let mut typed: Vec<&str> = LINUX_TYPES.iter().map(|(e, _)| *e).collect();
        associated.sort_unstable();
        typed.sort_unstable();
        typed.dedup();
        assert_eq!(typed, associated, "a MIME type for each extension the packaging associates with the app");
        for (e, t) in LINUX_TYPES {
            assert!(listed.contains(t), "the desktop entry doesn't list {t} (.{e})");
            let package = declared.iter().find(|(name, ..)| name == t);
            if LINUX_SHARED_TYPES.contains(t) {
                assert!(package.is_none(), "{t} is shared-mime-info's, and the MIME package declares it again");
                continue;
            }
            let Some((_, globs, magic)) = package else { panic!("the MIME package doesn't declare {t} (.{e})") };
            if let Some((_, rule)) = LINUX_BY_CONTENT.iter().find(|(content, _)| content == e) {
                assert!(!globs.contains(e), "{t} matches .{e} files by content only");
                assert_eq!(magic, rule, "{t}'s magic rule for .{e} files");
            } else {
                assert!(globs.contains(e), "{t} has no *.{e} glob");
            }
        }
        for t in &listed {
            assert!(LINUX_TYPES.iter().any(|(_, known)| known == t), "the desktop entry lists {t}, which the app doesn't open");
        }
        for (t, globs, _) in &declared {
            assert!(listed.contains(t), "the desktop entry doesn't list the declared {t}");
            for g in globs {
                assert!(LINUX_TYPES.contains(&(*g, *t)), "{t}'s *.{g} isn't a file the app opens as {t}");
            }
        }
    }
}
