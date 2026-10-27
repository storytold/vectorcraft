use super::caches::{CanvasCache, Perf};
use super::clock::now_ms;
use super::clock::now_secs;
use super::services::{Services, browsable_url};
#[cfg(not(target_arch = "wasm32"))]
use super::state::FONT_INSTALL_POLL;
use super::state::{
    CaptureEpoch, GeometryStamp, InputGeneration, MAX_PENDING_SCREENSHOTS, MAX_SCREENSHOT_BYTES, MAX_SCREENSHOT_TOTAL_BYTES, PendingScreenshot,
    QueuedScreenshot, SCREENSHOT_SETTLE_FRAMES, SCREENSHOT_TIMEOUT_ERROR, SCREENSHOT_TIMEOUT_MS, SYSTEM_CLIPBOARD_POLL, take_frame_once,
};
use super::*;

mod clipboard;
mod commands;
mod control;
mod files;
mod fonts;
mod frame;
mod ime;
mod input;
mod links;
mod prefs;
mod screenshot;
mod view;

pub use fonts::MAX_RECENT_FONTS;

impl VectorcraftApp {
    /// The app around `session`, reaching the platform through `services` (file pickers, the
    /// clipboard, files and the like). It starts cataloguing the installed fonts at once, gives
    /// Data Recovery the host's store and makes a view for each open document.
    pub fn new(mut session: Session, services: Services) -> Self {
        // The font menus and the first file opened need the installed fonts: catalog them now.
        vectorcraft_text::FontDb::global().scan_in_background();
        if let Some(store) = &services.recovery_store {
            session.recovery.set_store(store.clone());
        }
        // The place cursor's clicks and drags place their files through the app
        // ([`place::place_from_cursor`]).
        session.place_cursor_requests = true;
        let views = session.documents().iter().map(View::of).collect();
        Self {
            session,
            ui: UiState::default(),
            views,
            services,
            canvas: CanvasCache {
                renderer: vectorcraft_render::Renderer::new(),
                texture: None,
                key: None,
                shown: None,
                last_ms: 0.0,
                moved: false,
                from_worker: false,
                waits_for_still: cfg!(target_arch = "wasm32"),
                still: None,
                worker: None,
                worker_started: false,
                slices: None,
                print_tiling: None,
                live_paint_gaps: None,
                hidden_chars: Default::default(),
                selection_box: None,
                selection_bounds: None,
                content_bounds: None,
                cursors: Default::default(),
                view_moved: None,
                hover_cursor: None,
            },
            perf: Perf::default(),
            dock_rt: Default::default(),
            titlebar_inset: 0.0,
            last_effect: None,
            last_warp: None,
            run_count: 0,
            clipboard_out: None,
            clipboard_published: None,
            clipboard_in: None,
            clipboard_publishing: Default::default(),
            pending_url: None,
            control_rx: None,
            pending_screenshots: vec![],
            queued_screenshots: vec![],
            screenshot_token: 0,
            screenshot_reserved: 0,
            synthetic: vec![],
            styled: false,
            fonts_ready: false,
            context: 0,
            ui_ctx: None,
            ui_fonts: Default::default(),
            updates_rt: Default::default(),
            frame_rate: Default::default(),
            canvas_rect: None,
            hover_doc: None,
            custom_titlebar: false,
            last_window_title: String::new(),
            graphics_adapter: None,
            place: Default::default(),
            system_paste: false,
            clipboard_image_request: None,
            system_paste_at: f64::NEG_INFINITY,
            clipboard_probe: None,
            picks: picks::Picks::default(),
            screen_sampler: None,
            font_check: None,
            #[cfg(not(target_arch = "wasm32"))]
            font_install_check_at: f64::NEG_INFINITY,
            paste_chord: Default::default(),
            // Held keys repeat their command (Cmd+= keeps zooming in).
            dispatcher: craft_keymap::egui::Dispatcher::new().with_repeat(true),
            background: Default::default(),
            opening: Default::default(),
            plugin_runs: Default::default(),
            placing: Default::default(),
            recovery: Default::default(),
            host_modifiers: Default::default(),
            forwarded: Vec::new(),
            forwarded_modifiers: None,
            forwarded_modifiers_held: false,
            forwarded_frame: false,
            synthetic_modifiers: false,
            ime_marked: None,
            ime_context: Default::default(),
            ime_compositions: Default::default(),
            ime_resets: Default::default(),
            ime_caret_rect: None,
            ime_discard: false,
            prefs_changed: false,
            touch_screen: false,
            scrub_group: false,
            scrub_scope: None,
            automation_roots: None,
            synthetic_frame: false,
            pen_end: Default::default(),
            host_iteration: 0,
            #[cfg(test)]
            now_override: None,
            tick_iteration: None,
            input_frames: vec![],
            shortcuts_frames: vec![],
            synthetic_injected: false,
            injected_events: vec![],
            injected_generation: InputGeneration::default(),
            injected_settle_hook: false,
            injected_settled: Default::default(),
            geometry_epoch: 0,
            geometry_stamp: None,
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Confine what the control channel does to `roots` (#832): its requests, the input it
    /// injects (in the frames that carry it) and the saves and exports they start read and write
    /// only inside them ([`file_access`]). The host's file services check every path they're given
    /// against the roots in force, and so does the engine; outside those scopes the person at the
    /// keyboard works as usual.
    pub fn with_automation_roots(mut self, roots: Option<Arc<AutomationRoots>>) -> Self {
        let Some(roots) = roots else { return self };
        self.automation_roots = Some(roots);
        let s = &mut self.services;
        if let Some(read) = s.read.take() {
            s.read = Some(Box::new(move |p: &str| file_access::check_read(p).and_then(|()| read(p))));
        }
        if let Some(mut write) = s.write.take() {
            s.write = Some(Box::new(move |p: &str, b: &[u8]| file_access::check_write(p).and_then(|()| write(p, b))));
        }
        if let Some(write) = s.write_shared.take() {
            s.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| file_access::check_write(p).and_then(|()| write(p, b))));
        }
        if let Some(stream) = s.stream_shared.take() {
            s.stream_shared = Some(Arc::new(move |p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
                file_access::check_write(p).and_then(|()| stream(p, fill))
            }));
        }
        // Edit Original and Show in Folder hand a file to another app: only one automation may read.
        for service in [&mut s.open_file, &mut s.reveal] {
            if let Some(mut open) = service.take() {
                *service = Some(Box::new(move |p: &str| file_access::check_read(p).and_then(|()| open(p))));
            }
        }
        self
    }
}

/// eframe isn't a dependency of this crate (the host owns the event loop); these entry points are
/// called from the host's `eframe::App` impl.
impl VectorcraftApp {
    /// The language the UI is drawn in: the Preferences dialog's choice while it is open (so a
    /// change shows before OK), else the `interfaceLanguage` preference (`auto` = the system's).
    pub fn ui_language(&self) -> i18n::Lang {
        let editing =
            self.ui.dialog.as_ref().filter(|d| d.kind == "preferences").and_then(|d| d.fields.get("interfaceLanguage")).and_then(Value::as_str);
        i18n::Lang::from_pref(editing.unwrap_or(&self.session.prefs.interface_language))
    }

    /// The host starts one iteration: called from both hosts' `raw_input_hook`, before the host
    /// decides whether a UI pass follows. It counts the iteration; it is not a frame or pass count
    /// and does not promise that `ui` will run (a hidden window gets `tick` only).
    pub fn begin_host_iteration(&mut self) {
        self.host_iteration = self.host_iteration.wrapping_add(1);
        // Frame keys identify input already taken in this host iteration. Each iteration starts
        // fresh; this is what makes a viewport's discarded pass a duplicate, while two viewports'
        // frame 0 both count.
        self.input_frames.clear();
        self.shortcuts_frames.clear();
    }

    /// Background work, once per host iteration (full or logic-only): control requests, background
    /// saves/exports/opens, plug-ins, recovery, font and clipboard polls, capture completion. It
    /// reads only viewport state from `ctx.input` — never events or `i.time`: a logic-only pass
    /// leaves the last shown frame's input in place.
    ///
    /// A bug that panics costs one iteration and shows an error, instead of closing the app with
    /// unsaved work.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
        if let Err(msg) = file_access::confine(confined.as_ref(), || vectorcraft_engine::guard::catch_panic(|| self.tick_frame(ctx))) {
            self.picks.recover();
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
    }

    /// [`Self::tick`]'s work, once per host iteration: a second call in the same iteration (a
    /// discarded rerun shares one hook call) does nothing.
    fn tick_frame(&mut self, ctx: &egui::Context) {
        if self.tick_iteration == Some(self.host_iteration) {
            return;
        }
        self.tick_iteration = Some(self.host_iteration);
        let dialog_was_open = self.ui.dialog.is_some();
        let lang = self.ui_language();
        i18n::set_current(lang);
        // The engine gives new type the Japanese defaults while the UI is in Japanese.
        if self.session.ui_language.as_deref() != Some(lang.code()) {
            self.session.ui_language = Some(lang.code().to_string());
        }
        self.install_injected_settle_hook(ctx);
        self.advance_geometry_epoch(ctx);
        // Monotonic host time, never egui's `InputState::time`: a logic-only (hidden) iteration
        // replays the last shown pass's time, freezing recovery, clipboard polling and deadlines.
        // Tests inject a clock to drive idle scheduling deterministically.
        #[cfg(test)]
        let now = self.now_override.unwrap_or_else(now_secs);
        #[cfg(not(test))]
        let now = now_secs();
        self.perf.fps = self.frame_rate.frame(now, self.perf.frame_ms / 1000.0);
        self.sync_views();
        updates_ui::tick(self, ctx);
        // Read the system clipboard only when that alone decides whether Paste is enabled. A
        // background thread reads it where the host installs one (an unresponsive owner must never
        // stall the frame loop); otherwise, or once that thread is gone, it is read here, at most a
        // few times a second (opening it locks it against other apps on some systems).
        let wanted = self.session.clipboard.is_empty() && self.session.active().is_some();
        if let Some(make) = self.services.clipboard_probe.take() {
            self.clipboard_probe = clipboard_probe::Probe::start(make, ctx.clone());
        }
        if let Some(pasteable) = self.clipboard_probe.as_mut().and_then(|p| p.pasteable(wanted)) {
            self.system_paste = pasteable;
        } else if !(0.0..SYSTEM_CLIPBOARD_POLL).contains(&(now - self.system_paste_at)) {
            self.clipboard_probe = None;
            self.system_paste_at = now;
            self.system_paste = wanted && self.system_clipboard_pasteable();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !(0.0..FONT_INSTALL_POLL).contains(&(now - self.font_install_check_at)) {
            self.font_install_check_at = now;
            // Read on the next frame the app draws anyway: an idle app isn't woken for it.
            self.refresh_installed_fonts(false);
        }
        self.poll_font_check(ctx);
        picks::poll(self, ctx);
        // A Paste the host read before this iteration's formats were written must still wait for
        // them, even if the worker below finishes them during this same tick.
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("clipboard.pending-at-tick"), self.clipboard_publishing.is_pending()));
        background::poll(self);
        screen_color::poll(self, ctx);
        opening::poll(self);
        plugin_runs::poll(self);
        placing::poll(self);
        #[cfg(not(target_arch = "wasm32"))]
        self.tick_link_updates(ctx, now);
        if !self.background.jobs.is_empty()
            || !self.opening.pending.is_empty()
            || !self.plugin_runs.running.is_empty()
            || !self.placing.pending.is_empty()
        {
            // Keep the status bar's progress moving and pick the result up when it arrives.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // The window's close button (or the system quitting the app) asks about unsaved documents,
        // once the saves running in the background are done. Not while a file dialog's sheet is on
        // the window: what asked for it runs again when it answers.
        if ctx.input(|i| i.viewport().close_requested()) {
            // This native viewport is going away: release its IME context. No request is
            // allocated here: the close prompt or file sheet publishes no custom descriptor, so a
            // token would survive for a later stock field. A close the user then cancels starts a
            // fresh generation on the next pass (never inherits a composition across the prompt).
            crate::ime::close_viewport(self, ctx);
            self.ime_context.end_viewport(ctx.viewport_id());
            background::wait_all(self);
            if self.file_sheet_open() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.status(picks::busy());
            } else if unsaved::any_dirty(self) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if let Err(e) = unsaved::close_all(self, "quit") {
                    self.status(e);
                }
            } else {
                // Quitting with nothing unsaved: no copies to leave behind.
                vectorcraft_engine::cmd::recovery::forget_all(&mut self.session);
            }
        }
        if let Some(wait) = recovery::frame(self, now) {
            // The timer and heartbeat run in an idle window too.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        crate::keymap::note_os(ctx);
        shortcut_editor::sync(&self.ui);
        prefs_dialog::apply_runtime(self, ctx);
        // The OS title bar (and the taskbar / Alt-Tab entry) follows the active file; with the
        // system title bar this is where the document name lives, as the in-app mark is hidden.
        crate::chrome::sync_window_title(self, ctx);
        self.drain_control(ctx);
        // An egui `PasteImage` with a rich system clipboard: run the paste command now, once. It
        // goes through `run`, whose `edit.paste` branch calls `adopt_system_clipboard` (which clears
        // the request) to read the formats and then pastes them. A text paste in the same UI pass
        // never sets the request, so the two cannot both paste.
        if let Some(held) = self.clipboard_image_request.take() {
            crate::menus::invoke(self, crate::shortcuts::paste_command(held), json!({}));
        }
        self.collect_screenshots(ctx);
        if let Some(u) = self.pending_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(u));
        }
        self.flush_clipboard_out(ctx);
        self.drain_inbox();
        if self.viewport_hidden(ctx) && (self.needs_hidden_paint() || (!dialog_was_open && self.ui.dialog.is_some())) {
            // A plain repaint runs only logic in a hidden window: the ui that applies injected
            // input, shows a queued dialog or issues a capture needs this command. egui-winit
            // handles it; the 0.37 web backend does not (a pending fork fix), so a hidden browser
            // tab cannot be claimed as qualified.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }
        if !self.synthetic.is_empty() || self.synthetic_injected || !self.pending_screenshots.is_empty() {
            ctx.request_repaint();
        }
    }

    /// The 0.36-style entry point, kept for the crate's tests, which call one `logic` inside a
    /// `run_ui` pass and draw a panel afterwards (a UI phase). Production hosts call
    /// `raw_input_hook`, then `tick` in `App::logic` and `ui` in `App::ui`; `tick` alone never
    /// takes input, so a hidden logic-only iteration cannot replay the last shown frame's events.
    ///
    /// Outside tests this is just [`Self::tick`] with an iteration boundary.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.begin_host_iteration();
        self.tick(ctx);
        #[cfg(test)]
        {
            let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
            let _ = file_access::confine(confined.as_ref(), || {
                vectorcraft_engine::guard::catch_panic(|| {
                    self.ui_setup(ctx);
                    self.handle_input_once(ctx);
                })
            });
        }
        self.finish_ime_logic(ctx);
    }

    /// First UI-pass setup, in a real `run_ui` phase: adopt the context, install the interface
    /// fonts and theme, and the image loaders. 0.37's `set_fonts` applies at once when no text has
    /// been laid out, so the same pass can draw; a hidden logic-only iteration never installs them.
    fn ui_setup(&mut self, ctx: &egui::Context) {
        // Hosts install before their first pass. Installing here covers later passes only: egui
        // has already snapshotted this pass's plugin list, so a direct harness that calls `logic`
        // inside its first `run_ui` must call `install_ime_reset_hooks` before that `run_ui`
        // (as `tests_lifecycle::ime_frame` does). This is not a first-pass bootstrap fallback.
        crate::ime::install_hooks(ctx);
        self.adopt_context(ctx);
        if !self.styled {
            theme::apply(ctx, self.ui.brightness);
            egui_extras::install_image_loaders(ctx);
            self.styled = true;
        }
        if !self.fonts_ready {
            // `set_fonts` applies at once only while no text was laid out in this pass; when it
            // defers, the next pass applies it and this leaves input and drawing to that pass.
            let fonts = theme::interface_fonts();
            ctx.set_fonts(fonts.clone());
            self.fonts_ready = ctx.fonts(|applied| applied.definitions() == &fonts);
        }
    }

    /// The canvas panel's body: the document tabs, the canvas and the isolation bar over it.
    pub(crate) fn canvas_area(&mut self, ui: &mut egui::Ui) {
        let t = theme::Tokens::get(ui.ctx());
        ui.painter().rect_filled(ui.max_rect(), 0.0, t.pasteboard);
        if self.ui.screen_mode < 3 {
            chrome::doc_tabs(self, ui);
        }
        canvas::show(self, ui);
        // Over the canvas inside the rulers, so the rulers and scroll bars stay uncovered.
        if self.ui.screen_mode < 3
            && !menus::home_showing(self)
            && let Some(area) = self.canvas_rect
        {
            chrome::isolation_bar(self, ui, area);
        }
    }

    /// Lay out the whole window: the first UI pass of a host iteration sets the UI up (fonts,
    /// theme, image loaders) and takes the input; later passes (a `request_discard` rerun) draw
    /// only.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        crate::ime::begin_ui(self, &ctx);
        let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
        if let Err(msg) = file_access::confine(confined.as_ref(), || {
            vectorcraft_engine::guard::catch_panic(|| {
                self.ui_setup(&ctx);
                self.handle_input_once(&ctx);
                self.ui_frame(ui);
            })
        }) {
            self.picks.recover();
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
        crate::ime::finish_ui(self, &ctx);
    }

    /// [`Self::ui`]'s work, inside its guard: the bars, panels and canvas, then the dialogs and
    /// progress windows over them.
    fn ui_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            // Do not draw (or run the app's input handling) before the interface fonts are the
            // context's: a deferred `set_fonts` applies at the start of the next pass.
            ctx.request_repaint();
            return;
        }
        crate::state::sync_zoom_stops_display_print_size(self.session.prefs.display_print_size);
        let t0 = now_ms();
        widgets::set_dialog_open(&ctx, self.ui.dialog.is_some());
        // The canvas records this pass's IME caret geometry; a pass that draws no canvas has none.
        self.ime_caret_rect = None;
        scrub::begin_frame(self, &ctx);
        font_menu::end_stale_preview(self, &ctx);
        tools_float::track(self, &ctx);
        // A selection move released over a panel ends before any panel sees the release (#2826).
        canvas::release_over_panel(self, &ctx);
        let t = theme::Tokens::get(&ctx);
        if self.ui.screen_mode < 2 {
            chrome::app_bar(self, ui);
            if self.ui.control_bar {
                chrome::control_bar(self, ui);
            }
        }
        if self.ui.status_bar && self.ui.screen_mode < 3 {
            chrome::status_bar(self, ui);
            chrome::hint_bar(self, ui);
        } else {
            // Keeps the previous frame's value valid for menus drawn above, before the bars.
            widgets::set_menu_floor(&ctx, ctx.content_rect().bottom());
        }
        if self.ui.toolbar && self.ui.screen_mode < 3 {
            toolbar::show(self, ui);
        }
        panels::library_panel::tidy(self);
        // The canvas and the panels: one docking layout. Tab hides the panels but not the canvas.
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| panel_docking::show(self, ui));
        dialogs::show(self, &ctx);
        updates_ui::show(self, &ctx);
        export_progress::show(self, &ctx);
        plugin_runs::show(self, &ctx);
        palette::show(self, &ctx);
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
        self.ui_fonts.frame(&ctx);
        scrub::end_frame(self, &ctx);
        native_menu::sync(self, &ctx);
        // Observe once, after all of this pass's input, dialogs, floating panels, palette and
        // commands: the latest completed pass is authoritative for owner, text and cause.
        crate::ime::end_of_pass(self, &ctx);
        self.drop_formats_after_egui_copy(&ctx);
        // A command run by a menu, panel or dialog in this pass may have queued its text copy
        // after the UI phase's input flush. Publish it now for the same reason as there.
        self.flush_clipboard_out(&ctx);
        if opening::long_operation_busy(self) {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
        }
        // A pass egui will discard and rerun is not a presented frame: it must not become the
        // frame-time sample, and it must not issue a capture of what it drew.
        if !ctx.will_discard() {
            self.perf.frame_ms = now_ms() - t0;
            self.issue_screenshots(&ctx);
        }
        let _ = json!(null);
    }
}
