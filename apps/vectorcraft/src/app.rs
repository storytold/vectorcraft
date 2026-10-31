#[cfg(all(target_os = "macos", feature = "accessibility", feature = "wgpu"))]
use super::gpu;
use super::prefs::{save_prefs, save_prefs_soon};
#[cfg(all(windows, not(target_vendor = "win7")))]
use super::windows_stylus;
use super::{App, VectorcraftApp, file_access, window};
#[cfg(target_os = "macos")]
use super::{mac_menu, mac_window, open_documents};

/// The parts of [`App::raw_input_hook`] that every window needs, for the OS windows of floating
/// panels: a click or Escape that closes the macOS menu bar's open menu, and the pen barrel
/// button's secondary click on Windows. The control channel's input and the native menu's keys
/// go to the main window only.
#[cfg(target_os = "macos")]
pub(crate) fn native_input_hook(app: &VectorcraftApp) -> Option<vectorcraft_ui_egui::NativeInputHook> {
    app.services.native_menu.is_some().then(|| std::sync::Arc::new(mac_menu::raw_input_hook) as vectorcraft_ui_egui::NativeInputHook)
}

#[cfg(windows)]
pub(crate) fn native_input_hook(_: &VectorcraftApp) -> Option<vectorcraft_ui_egui::NativeInputHook> {
    Some(std::sync::Arc::new(windows_stylus::raw_input_hook))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) fn native_input_hook(_: &VectorcraftApp) -> Option<vectorcraft_ui_egui::NativeInputHook> {
    None
}

impl eframe::App for App {
    // Each frame runs unconfined: it is the person at the keyboard's. The app confines what the
    // control channel asks for, and the frames carrying input it injected, to the automation roots
    // (`VectorcraftApp::with_automation_roots`), which are in force elsewhere in the process.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        file_access::unconfined(|| self.logic_frame(ctx, frame));
        self.app.finish_ime_logic(ctx);
    }
    /// Restore the borderless caption, apply platform menu and pen input, and hand the frame to the
    /// app's raw-input hook while the keyboard user is unconfined.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // winit gives the window its caption back when it changes the window's flags (maximize,
        // restore, …): take it off again before the frame is laid out (#1758).
        if let Some(window_frame) = self.window_frame.as_mut() {
            window_frame();
        }
        #[cfg(target_os = "macos")]
        if self.app.services.native_menu.is_some() {
            mac_menu::raw_input_hook(raw);
        }
        // A pen contact with the barrel button held presses the secondary button (Windows Ink).
        #[cfg(windows)]
        windows_stylus::raw_input_hook(raw);
        file_access::unconfined(|| self.app.raw_input_hook(ctx, raw));
        // A mouse-up that never reached the window (#4326) is delivered once AppKit reports
        // that no mouse button is down.
        #[cfg(target_os = "macos")]
        self.app.release_lost_buttons(ctx, raw, objc2_app_kit::NSEvent::pressedMouseButtons() != 0);
    }
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            let bar = vectorcraft_ui_egui::chrome::APP_BAR_HEIGHT;
            let inset = mac_window::center_buttons(frame, ui.ctx().zoom_factor(), bar);
            self.app.titlebar_inset = inset.unwrap_or(mac_window::DEFAULT_INSET);
        }
        // macOS converts the frame from sRGB to the display's profile (#2249).
        #[cfg(all(target_os = "macos", feature = "wgpu"))]
        super::mac_color_space::tag(ui);
        file_access::unconfined(|| self.app.ui(ui));
        // Preferences are written when confirmed, not only at exit: a session ended any other way
        // (the system shutting down, the process ended or crashing) used to lose them (#1747).
        if self.app.take_prefs_changed() {
            file_access::unconfined(|| save_prefs(&self.app));
        } else {
            // The workspace, panels and other UI settings change without the dialog (#4625).
            file_access::unconfined(|| save_prefs_soon(&self.app));
        }
        // Not while a file dialog's sheet has the keyboard: its name field's composition would go.
        #[cfg(target_os = "macos")]
        if !self.app.file_dialog_open() && self.app.take_ime_discard() {
            discard_marked_text();
        }
    }
    fn on_exit(&mut self) {
        file_access::unconfined(|| save_prefs(&self.app));
        end_before_teardown(self.frames);
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
            let files = open_documents::take();
            if !files.is_empty() {
                bring_window_forward(ctx);
            }
            open_files(&mut self.app, files);
        }
        self.app.tick(ctx);
        if self.pending_maximize {
            if let Some(w) = &self.winit_window {
                self.pending_maximize = window::apply_pending_maximize(w, true, &mut self.maximize_tries);
            } else {
                self.pending_maximize = false;
            }
        }
        let host_maximized = self.winit_window.as_ref().map(|w| w.is_maximized());
        window::track(ctx, &mut self.app.ui.window, host_maximized);
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

/// Show the window when files are opened from the Finder or the Dock: a minimized window comes back
/// out of the Dock and the app takes focus, as a document-based Mac app does (#3769).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn bring_window_forward(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

/// Open files handed to the app (command line, macOS Finder and Dock) as documents. A file that
/// can't be opened is reported to the user and on stderr; the others still open.
pub(crate) fn open_files(app: &mut VectorcraftApp, files: Vec<String>) {
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
