#[cfg(all(target_os = "macos", feature = "accessibility", feature = "wgpu"))]
use super::gpu;
use super::prefs::{save_prefs, save_prefs_soon};
#[cfg(not(target_os = "macos"))]
use super::single_instance;
#[cfg(all(windows, not(target_vendor = "win7")))]
use super::windows_stylus;
use super::{App, VectorcraftApp, file_access, window};
#[cfg(target_os = "macos")]
use super::{mac_menu, mac_window, open_documents};
use vectorcraft_ui_egui::dialogs::safe_mode;
use vectorcraft_ui_egui::hang_beacon::{self, BEACON, FrameTime, Stage};

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
        BEACON.enter(Stage::Logic);
        file_access::unconfined(|| self.logic_frame(ctx, frame));
        self.app.finish_ime_logic(ctx);
        BEACON.enter(Stage::Idle);
    }
    /// Restore the borderless caption, apply platform menu and pen input, and hand the frame to the
    /// app's raw-input hook while the keyboard user is unconfined.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        BEACON.enter(Stage::Input);
        self.frame_start = Some(std::time::Instant::now());
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
        // The characters Alt + keypad codes typed (#5108).
        #[cfg(windows)]
        super::windows_alt_codes::raw_input_hook(raw);
        file_access::unconfined(|| self.app.raw_input_hook(ctx, raw));
        // A mouse-up that never reached the window (#4326) is delivered once AppKit reports
        // that no mouse button is down.
        #[cfg(target_os = "macos")]
        self.app.release_lost_buttons(ctx, raw, objc2_app_kit::NSEvent::pressedMouseButtons() != 0);
        BEACON.enter(Stage::Idle);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        BEACON.enter(Stage::Interface);
        #[cfg(target_os = "macos")]
        {
            let bar = vectorcraft_ui_egui::chrome::APP_BAR_HEIGHT;
            let inset = mac_window::center_buttons(frame, ui.ctx().zoom_factor(), bar);
            self.app.titlebar_inset = inset.unwrap_or(mac_window::DEFAULT_INSET);
        }
        // macOS converts the frame from sRGB to the display's profile (#2249).
        #[cfg(all(target_os = "macos", feature = "wgpu"))]
        super::mac_color_space::tag(ui);
        if let Some(on) = self.host_caps_lock.read_transition() {
            self.app.set_caps_lock(on);
        }
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
        self.end_frame(ui.ctx(), frame);
    }
    fn on_exit(&mut self) {
        // The window is closing: saving and teardown are not timed for the hang report, and a report
        // of a stall the interface came back from is removed.
        let log_dir = super::prefs::log_dir().filter(|_| super::prefs::prefs_enabled());
        super::hang_watch::closing(log_dir.as_deref(), &super::hang_watch::CLOSING, &BEACON);
        file_access::unconfined(|| save_prefs(&self.app));
        // A ready update installs once the process has exited. The macOS exit below skips
        // dropping the app, so the helper is started here.
        self.app.start_pending_update();
        #[cfg(target_os = "macos")]
        super::mac_cursor::release();
        end_before_teardown(self.frames);
    }
}

impl App {
    /// The frame's times for the hang report, and its last stage ([`hang_beacon::after_frame`]): a
    /// focused, shown window whose windows had input waits for the renderer and the next frame,
    /// which the input asks for at once; any other frame leaves the interface thread idle
    /// (`hang_watch`).
    fn end_frame(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // A pass egui discards and runs again is not the frame's end: the pass after it is.
        if ctx.will_discard() {
            return;
        }
        if let Some(start) = self.frame_start.take() {
            let work_ms = start.elapsed().as_secs_f32() * 1000.0;
            let previous_total_ms = frame.info().cpu_usage.map(|s| s * 1000.0);
            BEACON.frame(FrameTime { work_ms, previous_total_ms }, self.app.session.documents().len());
        }
        // Hide on macOS hides the window at once, before the window reports losing focus.
        let shown = ctx.input(|i| i.focused && i.viewport().minimized != Some(true))
            && self.winit_window.as_ref().is_none_or(|w| w.is_visible() != Some(false));
        let input = BEACON.take_input();
        BEACON.enter(ctx.viewport(|vp| hang_beacon::after_frame(input, shown, &vp.commands)));
    }

    fn logic_frame(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        if self.winit_window.as_ref().is_some_and(|w| !w.has_focus() || w.is_visible() == Some(false) || w.is_minimized() == Some(true)) {
            super::mac_cursor::release();
        }
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
        // An app that stays in Safe Mode leaves later launches to start their own app with plug-ins.
        #[cfg(not(target_os = "macos"))]
        if safe_mode::staying_on(&self.app) {
            single_instance::resign();
        }
        // Files from the Finder and the Dock (macOS) or from later launches (#4892) wait while a file
        // dialog is open: what asked for it runs again on its document when it answers. They also
        // wait for the choice about a stopped plug-in, so they open after the plug-ins it loads, as
        // do the command line's files held back for it. Files later launches handed over before the
        // choice ended in Safe Mode open in a new app with plug-ins.
        if !self.app.file_dialog_open() && !safe_mode::choosing(&self.app) {
            let held = std::mem::take(&mut self.held_files);
            #[cfg(target_os = "macos")]
            let files = open_documents::take();
            #[cfg(not(target_os = "macos"))]
            let files = single_instance::take_for_frame(safe_mode::staying_on(&self.app), single_instance::launch_with_plug_ins);
            if !files.is_empty() {
                bring_window_forward(ctx);
            }
            open_files(&mut self.app, held);
            open_files(&mut self.app, files);
        }
        let (now, zoom) = (ctx.input(|i| i.time), ctx.zoom_factor());
        if let Some(w) = &self.winit_window
            && window::monitors_due(self.monitors_listed, now, zoom)
        {
            self.app.services.dock_monitors = window::dock_monitors(w, zoom);
            self.monitors_listed = Some((now, zoom));
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
/// app as it quits on Macs with a Touch Bar. On a normal quit, only the launch marker is needed
/// after the window, so it is ended here as `main` ends it after the window on other platforms.
/// Not while the window is still starting up (`frames` below [`gpu::STARTUP_FRAMES`]): a graphics
/// failure then starts the app again on another adapter (#651).
#[allow(unused_variables)]
fn end_before_teardown(frames: u64) {
    #[cfg(all(target_os = "macos", feature = "accessibility", feature = "wgpu"))]
    if frames >= gpu::STARTUP_FRAMES {
        safe_mode::end_launch();
        log::info!("quitting");
        log::logger().flush();
        std::process::exit(0);
    }
}

/// Show the window when files are opened from the Finder or the Dock, or handed over by a later
/// launch (#4892): a minimized window comes back and the app takes focus, as a document-based app
/// does (#3769).
pub(crate) fn bring_window_forward(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

/// Open the command line's `files` now, or, while the choice about a stopped plug-in is up
/// ([`safe_mode::choosing`]), return them to open once it is made, with the plug-ins it loads.
pub(crate) fn open_or_hold(app: &mut VectorcraftApp, files: Vec<String>) -> Vec<String> {
    if safe_mode::choosing(app) {
        return files;
    }
    open_files(app, files);
    Vec::new()
}

/// Open files handed to the app (command line, macOS Finder and Dock, later launches) as documents. A file that
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
