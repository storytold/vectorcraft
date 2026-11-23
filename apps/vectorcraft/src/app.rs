#[cfg(all(target_os = "macos", feature = "accessibility", feature = "wgpu"))]
use super::gpu;
use super::prefs::{flush, save_tick};
#[cfg(not(target_os = "macos"))]
use super::single_instance;
use super::{App, VectorcraftApp, file_access};
#[cfg(target_os = "macos")]
use super::{mac_window, open_documents};
use craft::ui::runtime::env::WindowKey;
use craft::ui::shell::{WindowDialogs, embed};
use vectorcraft_ui_egui::dialogs::safe_mode;
use vectorcraft_ui_egui::hang_beacon::{self, BEACON, FrameTime, Stage};

impl eframe::App for App {
    // Each frame runs unconfined: it is the person at the keyboard's. The app confines what the
    // control channel asks for, and the frames carrying input it injected, to the automation roots
    // (`VectorcraftApp::with_automation_roots`), which are in force elsewhere in the process.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _timed = BEACON.enter_timed(Stage::Logic);
        file_access::unconfined(|| {
            self.logic_frame(ctx, frame);
            embed::logic(ctx, || self.app.close_decision());
        });
        BEACON.enter(Stage::Idle);
    }
    /// Restore the borderless caption, apply platform menu and pen input, and hand the frame to the
    /// app's raw-input hook while the keyboard user is unconfined.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Without vsync, a main-window frame starts no sooner than the minimum interval after the
        // last one. eframe runs this hook once per frame, shown or not, before egui reads the input.
        // The frame's time moves on by the wait, and the hang report's frame time starts after it.
        if raw.viewport_id == egui::ViewportId::ROOT {
            if let Some(wait) = super::presentation::frame_wait(self.min_frame, self.last_frame_began.map(|t| t.elapsed())) {
                std::thread::sleep(wait);
                raw.time = raw.time.map(|time| time + wait.as_secs_f64());
            }
            self.last_frame_began = Some(std::time::Instant::now());
        }
        let _timed = BEACON.enter_timed(Stage::Input);
        self.frame_start = Some(std::time::Instant::now());
        // winit gives the window its caption back when it changes the window's flags (maximize,
        // restore, …): take it off again before the frame is laid out (#1758).
        if let Some(window_frame) = self.window_frame.as_mut() {
            window_frame();
        }
        // The characters Alt + keypad codes typed (#5108).
        #[cfg(windows)]
        super::windows_alt_codes::raw_input_hook(raw);
        self.app.raw_input(ctx, raw);
        file_access::unconfined(|| embed::before_input(ctx, |out| self.app.frame_input(out)));
        BEACON.enter(Stage::Idle);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _timed = BEACON.enter_timed(Stage::Interface);
        embed::before_ui(ui.ctx(), frame);
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
        file_access::unconfined(|| save_tick(&mut self.app, &self.settings, std::time::Instant::now()));
        // The native dialogs the app asked for in this frame show now, over the window, in the
        // interface language.
        let lang = vectorcraft_ui_egui::i18n::current();
        if self.replace_lang != Some(lang) {
            self.replace_lang = Some(lang);
            self.file_panels.set_replace_words(vectorcraft_ui_egui::i18n::replace_words());
        }
        WindowDialogs::of(ui.ctx(), &WindowKey::main()).with(|dialogs| self.file_panels.pump_with(dialogs, Some(&*frame)));
        // The folders the picks chose are saved with the UI state; one the file can't hold (not
        // UTF-8) is left out.
        let mut folders = self.files.last_folders();
        folders.retain(|_, folder| folder.to_str().is_some());
        if folders != self.app.ui.last_folders {
            self.app.ui.last_folders = folders;
        }
        self.end_frame(ui.ctx(), frame);
    }
    fn on_exit(&mut self) {
        // The window is closing: saving and teardown are not timed for the hang report, and a report
        // of a stall the interface came back from is removed.
        let log_dir = super::prefs::log_dir().filter(|_| super::prefs::prefs_enabled());
        super::hang_watch::closing(log_dir.as_deref(), &super::hang_watch::CLOSING, &BEACON);
        file_access::unconfined(|| flush(&self.app, &self.settings));
        // A ready update installs once the process has exited. The macOS exit below skips
        // dropping the app, so the helper is started here.
        self.app.start_pending_update();
        // Dropping the app can wait for the clipboard handles to let go: the windows leave the
        // screen first (`quit_windows`).
        #[cfg(not(any(target_os = "macos", windows)))]
        super::quit_windows::before_teardown(self.winit_window.as_deref());
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
        let main_shown = ctx.input(|i| i.focused && i.viewport().minimized != Some(true))
            && self.winit_window.as_ref().is_none_or(|w| w.is_visible() != Some(false));
        // A floating panel or document window with the focus counts as well, with the commands it
        // was sent after its pass, and those its pass sent (`hang_beacon::watch_input`).
        let shown = ctx.input(|i| hang_beacon::shown_window(main_shown, &i.raw.viewports));
        let handed_off = BEACON.take_handed_off();
        let input = BEACON.take_input();
        let stage = match shown {
            Some(id) => ctx.viewport_for(id, |vp| hang_beacon::after_frame(input && !handed_off.contains(&id), true, &vp.commands)),
            None => hang_beacon::after_frame(input, false, &[]),
        };
        BEACON.enter(stage);
    }

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
        let now = ctx.input(|i| i.time);
        if let Some(w) = &self.winit_window {
            self.display_profile.tick(w, now, self.app.session.prefs.monitor_profile_display, &self.app.display_profile, ctx);
        }
        self.app.tick(ctx);
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
