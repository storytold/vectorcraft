//! VectorCraft desktop app.
//!
//! Usage: `vectorcraft [--control <port> [--automation-read-root <dir>] [--automation-write-root <dir>]]
//! [--in-window-menus] [files…]`
//!
//! `--in-window-menus` (or `VECTORCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar (`mac_menu`).
//!
//! `--control <port>` (or `VECTORCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
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

mod clipboard;
mod control_server;
#[cfg(feature = "wgpu")]
mod gpu;
mod logging;
#[cfg(target_os = "macos")]
mod mac_fonts;
#[cfg(target_os = "macos")]
mod mac_menu;
#[cfg(target_os = "macos")]
mod mac_window;
#[cfg(target_os = "macos")]
mod open_documents;
mod prefs_dir;
mod printing;
#[cfg(all(windows, not(target_vendor = "win7")))]
mod system_fonts;
mod window;

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
}

impl eframe::App for App {
    // Each frame runs unconfined: it is the person at the keyboard's. The app confines what the
    // control channel asks for, and the frames carrying input it injected, to the automation roots
    // (`VectorcraftApp::with_automation_roots`), which are in force elsewhere in the process.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        file_access::unconfined(|| self.logic_frame(ctx, frame));
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
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
        #[cfg(target_os = "macos")]
        if self.app.take_ime_discard() {
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
        #[cfg(target_os = "macos")]
        open_files(&mut self.app, open_documents::take());
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

/// A native file dialog for `request` over `parent` (Windows and Linux, where the portal then
/// shows it over the window and keeps it in front; macOS shows them as before).
fn file_dialog(request: &PickRequest, parent: &Parent) -> rfd::FileDialog {
    let d = match parent {
        Some(w) if !cfg!(target_os = "macos") => rfd::FileDialog::new().set_parent(&**w),
        _ => rfd::FileDialog::new(),
    };
    let pick = match request {
        PickRequest::Open(pick) | PickRequest::Save(pick) => pick,
        PickRequest::OpenMany => return fileio::place_filters().fold(d.set_title("Place"), |d, (name, exts)| d.add_filter(name, exts)),
        PickRequest::Folder => return d,
    };
    let d = pick.filters.iter().fold(d, |d, (name, exts)| d.add_filter(*name, exts));
    let d = match &pick.folder {
        Some(folder) => d.set_directory(folder),
        None => d,
    };
    if pick.name.is_empty() { d } else { d.set_file_name(&pick.name) }
}

/// Show `dialog` for `request` (on the calling thread, until it closes) → the paths picked.
fn show_dialog(dialog: rfd::FileDialog, request: &PickRequest) -> Vec<String> {
    let paths = match request {
        PickRequest::Open(_) => dialog.pick_file().into_iter().collect(),
        PickRequest::OpenMany => dialog.pick_files().unwrap_or_default(),
        PickRequest::Save(pick) => {
            // The Templates folder may not exist yet.
            if let Some(folder) = &pick.folder {
                let _ = std::fs::create_dir_all(folder);
            }
            dialog.save_file().into_iter().collect()
        }
        PickRequest::Folder => dialog.pick_folder().into_iter().collect(),
    };
    paths.into_iter().map(|p| p.to_string_lossy().to_string()).collect()
}

/// Show the dialog for `request` over `parent` now → the paths picked.
fn pick_now(request: PickRequest, parent: &Parent) -> Vec<String> {
    show_dialog(file_dialog(&request, parent), &request)
}

/// File → Show in Folder: select `path` in Finder / Explorer, or open its folder elsewhere.
fn reveal(path: &str) -> Result<(), String> {
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

/// Linux and macOS: show the dialog for `request` over `parent` on a thread of its own, answering
/// on the receiver. Shown on the UI thread, nothing would answer the compositor meanwhile on Linux,
/// which then offers to kill the window as not responding (#592); on macOS the panel ran modally
/// inside the window's event handler, and resizing it delivered window events to that handler
/// again, which crashed the app (#867). From another thread, rfd runs the panel on the main thread
/// from the run loop, outside the handler.
fn start_pick(request: PickRequest, parent: &Parent) -> Option<std::sync::mpsc::Receiver<Vec<String>>> {
    let dialog = file_dialog(&request, parent);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("file-dialog".into())
        .spawn(move || {
            // The app gone meanwhile has no use for the answer.
            let _ = tx.send(show_dialog(dialog, &request));
        })
        .ok()?;
    Some(rx)
}

fn services(parent: Parent) -> Services {
    let (p1, p2, p3, p4, p5) = (parent.clone(), parent.clone(), parent.clone(), parent.clone(), parent);
    Services {
        pick_open: Some(Box::new(move |pick: &FilePick| pick_now(PickRequest::Open(pick.clone()), &p1).into_iter().next())),
        pick_open_multi: Some(Box::new(move || pick_now(PickRequest::OpenMany, &p2))),
        pick_save: Some(Box::new(move |pick: &FilePick| pick_now(PickRequest::Save(pick.clone()), &p3).into_iter().next())),
        // Windows dialogs run the window's events while they are open; Linux's don't, and macOS
        // ones re-enter the window's event handler (#867).
        start_pick: cfg!(unix).then(|| Box::new(move |request: PickRequest| start_pick(request, &p5)) as vectorcraft_ui_egui::picks::StartPick),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(write_file)),
        // Background Save and Export write from a worker thread.
        write_shared: Some(std::sync::Arc::new(write_file)),
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
        // File → Print: the system's printers and print queue.
        print: Some(Box::new(printing::SystemPrint)),
        ..Default::default()
    }
}

/// Edit Original, Show Package: open `path` (a file or a folder) in the system's default app for it.
fn open_file(path: &str) -> Result<(), String> {
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

fn main() -> std::process::ExitCode {
    // First, so every start-up warning is recorded (`logging`).
    let logger = logging::install();
    vectorcraft_ui_egui::i18n::detect_system_lang_in_background();
    // Before the first font scan (the app's start): the fonts font services load (#579).
    #[cfg(all(windows, not(target_vendor = "win7")))]
    system_fonts::install();
    #[cfg(target_os = "macos")]
    mac_fonts::install();
    // VectorCraft's own Fonts folder, which text.addFontFiles copies fonts into.
    prefs_dir::install_fonts();
    let mut control_port: Option<u16> = std::env::var("VECTORCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let root_env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let mut read_root = root_env("VECTORCRAFT_AUTOMATION_READ_ROOT");
    let mut write_root = root_env("VECTORCRAFT_AUTOMATION_WRITE_ROOT");
    let mut files = Vec::new();
    let mut in_window_menus = std::env::var_os("VECTORCRAFT_IN_WINDOW_MENUS").is_some_and(|v| !v.is_empty() && v != "0");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--control" => control_port = args.next().and_then(|p| p.parse().ok()),
            "--automation-read-root" => read_root = args.next().map(std::path::PathBuf::from),
            "--automation-write-root" => write_root = args.next().map(std::path::PathBuf::from),
            _ if a.starts_with("--automation-read-root=") => read_root = a.split_once('=').map(|(_, d)| d.into()),
            _ if a.starts_with("--automation-write-root=") => write_root = a.split_once('=').map(|(_, d)| d.into()),
            "--in-window-menus" => in_window_menus = true,
            "--version" => {
                println!("vectorcraft {}", env!("CARGO_PKG_VERSION"));
                return std::process::ExitCode::SUCCESS;
            }
            _ => files.push(a),
        }
    }
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
    let custom_titlebar = custom_titlebar(saved.as_ref());
    let options = native_options(custom_titlebar);
    #[cfg(feature = "wgpu")]
    let options = {
        let mut options = options;
        // One frame queued, not two: the canvas is rasterized on the CPU and the GPU only
        // composites it, so the window answers the pointer a frame sooner (#444).
        options.wgpu_options.surface = eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY;
        // Only adapters that can show the window, in the order `gpu` gives (#306, #502).
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            create.native_adapter_selector = Some(gpu::selector(power, displays, startup.clone()));
            // Nothing draws or dispatches indirectly, so wgpu's check of indirect arguments only
            // costs a compute shader at start-up, one some drivers can't compile (OCLP-patched
            // Metal on an Iris Pro, #651). `WGPU_VALIDATION_INDIRECT_CALL=1` turns it back on.
            create.instance_descriptor.flags =
                (eframe::wgpu::InstanceFlags::from_build_config() - eframe::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
            create.instance_descriptor.backends = gpu::backends(create.instance_descriptor.backends, std::env::var_os("WGPU_BACKEND").is_some());
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
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut options = options;
    // winit has no file drag-and-drop on Wayland (only on X11), so dropping files from the file
    // manager showed a "no" cursor. Run through XWayland when it's there; VECTORCRAFT_WAYLAND=1
    // keeps the native Wayland backend.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("VECTORCRAFT_WAYLAND").is_none() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // A panic that ends the window (egui-wgpu's own, #502) is caught here: never a crash.
    let outcome = vectorcraft_engine::guard::catch_panic(|| {
        eframe::run_native(
            "VectorCraft",
            options,
            Box::new(move |cc| {
                let mut app = VectorcraftApp::new(Session::new(), services(cc.winit_window().cloned()));
                load_prefs(&mut app, saved);
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
                Ok(Box::new(App { app, graphics_loss, graphics_lost: false, frames: 0 }))
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

    /// Finder offers the app for every file File › Open reads (#295, #354) but Photoshop documents
    /// (`UNASSOCIATED_EXTS`), takes over no other app's files, and hands them to the app rather than
    /// to AppKit's document machinery.
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
}
