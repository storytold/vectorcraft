use craft::automation::ControlChannel;
use craft::host::{AppInfo, Host, Wake};
use craft::platform::clipboard::{Clipboard, ClipboardProbe, EventClipboard};
use craft::platform::clock::monotonic_ms;
use craft::theme::variant::Tone;
use craft::ui::contract::Bind;
use craft::ui::runtime::env::{RootEnv, Scale, ScrubDrags, WindowKey};
use craft::ui::runtime::frame::{FrameInput, FrameOutput};
use craft::ui::runtime::toast::{self, Toast};
use craft::ui::runtime::windows::WindowHost;
use craft::ui::shell::embed::CloseDecision;
use craft::ui::shell::{AppShell, CanvasPanel, Slot};
use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;
use vectorcraft_engine::cmd::toast::Level;

use super::caches::{CanvasCache, Perf};
use super::i18n::LangExt;
use super::services::{Services, browsable_url};
use super::state::SYSTEM_CLIPBOARD_POLL;
use super::{
    Arc, AutomationRoots, ControlRequest, Receiver, Session, UiState, VectorcraftApp, View, ViewInfo, background, canvas, chrome, credits, dialogs,
    edits, export_progress, file_access, file_asks, fileio, font_menu, i18n, io, menus, opening, palette, panel_docking, panels, place, placing,
    plugin_runs, prefs_dialog, recovery, screen_color, shortcuts, theme, trace_runs, unsaved, updates_ui,
};

mod commands;
mod control;
mod files;
mod fonts;
mod frame;
mod input;
mod links;
mod prefs;
#[cfg(test)]
mod tests;
pub(crate) mod view;

pub use fonts::MAX_RECENT_FONTS;

/// VectorCraft as the framework's services know it: the settings folder, the name and the version.
const APP_INFO: AppInfo = AppInfo { id: "vectorcraft", name: "VectorCraft", version: env!("CARGO_PKG_VERSION") };

/// Monotonic seconds of the app's clock, which recovery, the clipboard look and the frame rate
/// count in.
fn now_secs() -> f64 {
    monotonic_ms() / 1000.0
}

impl VectorcraftApp {
    /// The app around `session`, reaching the platform through `services` (the clipboard, files
    /// and the like). It starts cataloguing the installed fonts at once, gives Data Recovery the
    /// host's store and makes a view for each open document.
    pub fn new(mut session: Session, mut services: Services) -> Self {
        // The font menus and the first file opened need the installed fonts: catalog them now.
        vectorcraft_text::FontDb::global().scan_in_background();
        if let Some(store) = &services.recovery_store {
            session.recovery.set_store(store.clone());
        }
        // The place cursor's clicks and drags place their files through the app
        // ([`place::place_from_cursor`]).
        session.place_cursor_requests = true;
        // A script runs a slice per frame, so the window keeps drawing and Stop can be pressed.
        session.script_frames = true;
        let views = session.documents().iter().map(View::of).collect();
        let view_uids = session.documents().iter().map(|d| d.uid).collect();
        let known = session.documents().iter().map(|d| d.uid).collect();
        // Without a host clipboard (tests, the web before its host sets one) a copy's text goes out through egui, as
        // a text field's does.
        let fallback_copy = Arc::new(std::sync::Mutex::new(None));
        let clipboard = match services.clipboard.take() {
            Some(backend) => Clipboard::new(backend),
            None => {
                let copied = Arc::clone(&fallback_copy);
                Clipboard::new(EventClipboard::new(
                    || 0,
                    move |flavors: &[craft::platform::clipboard::Flavor]| {
                        let text = flavors.iter().enumerate().find(|(_, f)| f.mime == craft::platform::clipboard::TEXT);
                        if let (Some((_, f)), Ok(mut slot)) = (text, copied.lock()) {
                            *slot = Some(String::from_utf8_lossy(&f.data).into_owned());
                        }
                        Ok(text.map_or(0, |(i, _)| i))
                    },
                ))
            }
        };
        // PLAN: the hosts (hub-w3) replace `host` with the one they build (files, settings, jobs); every frame installs it through `frame_input`.
        let host = Arc::new(Host::builder(APP_INFO).i18n(i18n::shared()).build());
        Self {
            session,
            ui: UiState::default(),
            views,
            view_uids,
            clipboard_probe: None,
            clipboard,
            services,
            host,
            canvas: CanvasCache {
                renderer: vectorcraft_render::Renderer::new(),
                texture: None,
                key: None,
                shown: None,
                last_ms: 0.0,
                moved: false,
                from_worker: false,
                reused: None,
                waits_for_still: cfg!(target_arch = "wasm32"),
                still: None,
                fill: None,
                fill_worker: None,
                worker: None,
                worker_started: false,
                slices: None,
                print_tiling: None,
                live_paint_gaps: None,
                find_font: None,
                hidden_chars: Default::default(),
                spelling: Default::default(),
                momentary: None,
                selection_box: None,
                selection_bounds: None,
                content_bounds: None,
                variable_rows: None,
                three_d_exportable: None,
                pixel_aligned: None,
                symbol_instances: None,
                plugin_keys: Default::default(),
                placed_keys: Default::default(),
                view_moved: None,
                grab: None,
                carry_target: None,
                last_drag: None,
                escaped_press: false,
                committed_preview: false,
                panes: Default::default(),
                windows: crate::doc_windows::Windows::new(known),
                images_with_other_art: None,
                smooth_paths: None,
                multi_color: None,
                global_edit_count: None,
                shape_angle: None,
                trace_presets: None,
                trace_target: None,
                attributes_info: None,
                svg_info: None,
                css_shown: None,
                svg_code: None,
                artboard_thumbs: None,
                document_families: None,
                recently_added: None,
                caret_kerning: None,
                glyphs_found: None,
                glyph_form_faces: None,
                character_quads: None,
                tabs_guide_axis: None,
                appearance_summary: None,
                transparency_state: None,
                nav_region: None,
                doc_info: None,
                map_surfaces: None,
                palette: None,
            },
            display_profile: Default::default(),
            perf: Perf::default(),
            dock_rt: Default::default(),
            titlebar_inset: 0.0,
            last_effect: None,
            last_pathfinder: None,
            character_focus: None,
            run_count: 0,
            channel: None,
            deferred_control: None,
            control_rotate_press: None,
            pointer_waits: Vec::new(),
            context: 0,
            ui_ctx: None,
            updates_rt: Default::default(),
            frame_rate: Default::default(),
            hover_doc: None,
            clipboard_out: None,
            fallback_copy,
            clipboard_in: None,
            clipboard_publishing: Default::default(),
            pending_url: None,
            titlebar_from_env: false,
            desktop_keeps_frame: false,
            graphics_adapter: None,
            beacon: &crate::hang_beacon::BEACON,
            place: Default::default(),
            system_paste: false,
            system_paste_at: f64::NEG_INFINITY,
            file_asks: Default::default(),
            saves: io::save::Saves::new(Wake::none()),
            original_editor: Default::default(),
            screen_pick: craft::color_ui::ui::ScreenPick::new(),
            font_watch: None,
            fonts_changed: Default::default(),
            font_preview_owner: None,
            background: Default::default(),
            opening: Default::default(),
            plugin_runs: Default::default(),
            placing: Default::default(),
            trace_runs: Default::default(),
            recovery: Default::default(),
            closing: None,
            prefs: None,
            applied_prefs: None,
            menus: Default::default(),
            shortcuts_dialog: None,
            shortcut_overrides: Default::default(),
            told: Default::default(),
            held: Default::default(),
            edit_scopes: Default::default(),
            command_dialog: Default::default(),
            effect_dialogs: Default::default(),
            three_d_dialog: None,
            three_d: Default::default(),
            export_dialogs: Default::default(),
            web_export: Default::default(),
            flattener_preview: Default::default(),
            flatten_dialog: Default::default(),
            print_dialog: Default::default(),
            save_pdf: None,
            object_dialogs: Default::default(),
            color_dialogs: Default::default(),
            plugin_dialog: None,
            addon_panels: Default::default(),
            addon_dialogs: Default::default(),
            spelling: None,
            char_style_preview: Default::default(),
            para_style_preview: Default::default(),
            justification: None,
            bullets_preview: Default::default(),
            pending_paste: None,
            link_asks: None,
            ime_marked: None,
            ime_context: Default::default(),
            prefs_changed: false,
            touch_screen: false,
            automation_roots: None,
            pending_plugin_install: None,
            pending_profile_open: None,
            queued_profile_opens: Default::default(),
            pen_end: Default::default(),
            ticked_pass: None,
            #[cfg(test)]
            now_override: None,
            #[cfg(test)]
            fail_next_tool_choice: None,
            held_tool: Vec::new(),
        }
    }

    /// Answers the control channel whose requests arrive on `rx`.
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.channel = Some(ControlChannel::new(rx));
        self
    }

    /// Confine what the control channel does to `roots` (#832): its requests, the input it
    /// injects (in the passes that carry it) and the saves and exports they start read and write
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
        if let Some(read_head) = s.read_head.take() {
            s.read_head = Some(Box::new(move |p: &str, max: usize| file_access::check_read(p).and_then(|()| read_head(p, max))));
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

    /// The folders this pass may read and write: the automation roots while the pass carries
    /// input the control channel injected, anywhere otherwise.
    fn automation_scope(&self, ctx: &egui::Context) -> Option<Arc<AutomationRoots>> {
        WindowHost::of(ctx).injected_pass().then(|| self.automation_roots.clone()).flatten()
    }
}

/// eframe isn't a dependency of this crate (the host owns the event loop); these entry points are
/// called from the host's `eframe::App` impl through `embed`'s hooks.
impl VectorcraftApp {
    /// Applies the work that finished off the UI thread. It stays pending while a script runs, since
    /// every change to the documents ends in a busy error until the script ends.
    pub(crate) fn poll_finished_work(&mut self) {
        if self.session.script_running() {
            return;
        }
        background::poll(self);
        opening::poll(self);
        plugin_runs::poll(self);
        trace_runs::poll(self);
        placing::poll(self);
    }

    /// The language the UI is drawn in: the Preferences window's choice while it is open (so a
    /// change shows before OK), else the `interfaceLanguage` preference (`auto` = the system's).
    pub fn ui_language(&self) -> i18n::Lang {
        prefs_dialog::draft_language(self).unwrap_or_else(|| i18n::Lang::from_pref(&self.session.prefs.interface_language))
    }

    /// A window's raw input, before egui and the keys read it (the host's `raw_input_hook`).
    pub fn raw_input(&self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        crate::type_editing::keep_layout_text(self, ctx, raw);
    }

    /// What the runtime gets before each frame, from `embed::before_input`: the commands the last
    /// frame fired run first, so the input reflects them; then the theme, the scale, the zoom
    /// stops and the scrub choice from the preferences, the command states and the host with
    /// the interface language.
    pub fn frame_input(&mut self, out: FrameOutput) -> FrameInput {
        // Paste's state for the first frame's paste events, which come before the tick first reads the clipboard.
        if self.system_paste_at == f64::NEG_INFINITY && self.services.clipboard_probe.is_none() {
            self.system_paste = self.session.clipboard.is_empty() && self.session.active().is_some() && self.system_clipboard_pasteable();
        }
        // The output collects every pass of the last frame, so each fired command runs once here
        // whatever number of passes fired it.
        if let Some(ctx) = self.ui_ctx.clone() {
            for fired in &out.fired {
                shortcuts::run_fired(self, &ctx, fired);
            }
            for fired in &out.dimmed {
                shortcuts::run_dimmed(self, &ctx, fired);
            }
            menus::model::refresh(self, &ctx);
        }
        font_menu::end_previews(self, &out.ended_previews);
        edits::end_removed(self, &out.removed_gestures);
        let lang = self.ui_language();
        if self.host.lang() != lang {
            self.host = Arc::new(self.host.with_lang(lang));
        }
        let native_ppp = self.ui_ctx.as_ref().map_or(1.0, |ctx| ctx.pixels_per_point() / ctx.zoom_factor());
        let root = root_env(&self.session.prefs, native_ppp);
        FrameInput::new(root).commands(menus::model::command_input(self)).host(self.host.clone())
    }

    /// What the window's close button (or the system quitting the app) does, from `embed::logic`:
    /// it waits for a running script to be asked and for a file dialog's sheet, and asks about
    /// unsaved documents, once the saves running in the background are done and a Shape Builder
    /// merge being computed is applied. With nothing to ask, the window closes and no recovery
    /// copies are left behind.
    pub fn close_decision(&mut self) -> CloseDecision {
        unsaved::finish_pending(self);
        if self.session.script_running() {
            dialogs::script_run::ask_to_close(self);
            return CloseDecision::Hold;
        }
        if self.file_sheet_open() {
            self.status(file_asks::busy());
            return CloseDecision::Hold;
        }
        if unsaved::any_dirty(self) {
            if let Err(e) = unsaved::close_all(self, "quit") {
                self.status(e);
            }
            return CloseDecision::Hold;
        }
        vectorcraft_engine::cmd::recovery::forget_all(&mut self.session);
        CloseDecision::Close
    }

    /// Background work, once per frame from eframe's `logic` (full or hidden): control requests,
    /// background saves/exports/opens, plug-ins, recovery, font and clipboard polls. It reads
    /// only viewport state from `ctx.input`, never events or `i.time`: a hidden window's `logic`
    /// leaves the last shown frame's input in place.
    ///
    /// A bug that panics costs one frame's work and shows an error, instead of closing the app
    /// with unsaved work.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let confined = self.automation_scope(ctx);
        if let Err(msg) = file_access::confine(confined.as_ref(), || vectorcraft_engine::guard::catch_panic(|| self.tick_frame(ctx))) {
            self.session.set_replaying(false);
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
    }

    /// [`Self::tick`]'s work.
    fn tick_frame(&mut self, ctx: &egui::Context) {
        self.ticked_pass = Some(ctx.cumulative_pass_nr());
        let lang = self.ui_language();
        i18n::set_current(lang);
        // The engine gives new type the Japanese defaults while the UI is in Japanese.
        if self.session.ui_language.as_deref() != Some(lang.code()) {
            self.session.ui_language = Some(lang.code().to_string());
        }
        // Monotonic host time, never egui's `InputState::time`: a hidden window's `logic` replays
        // the last shown pass's time, freezing recovery, clipboard polling and deadlines. Tests
        // inject a clock to drive idle scheduling deterministically.
        #[cfg(test)]
        let now = self.now_override.unwrap_or_else(now_secs);
        #[cfg(not(test))]
        let now = now_secs();
        self.perf.fps = self.frame_rate.frame(now, self.perf.frame_ms / 1000.0);
        self.sync_views();
        self.refresh_replace_font_warning(false);
        updates_ui::tick(self, ctx);
        // Read the system clipboard only when that alone decides whether Paste is enabled. A
        // background thread reads it where the host gives one (an unresponsive owner must never
        // stall the frame loop); otherwise, or once that thread is gone, it is read here, at most a
        // few times a second (opening it locks it against other apps on some systems).
        let wanted = self.session.clipboard.is_empty() && self.session.active().is_some();
        if let Some(make) = self.services.clipboard_probe.take() {
            let repaint = ctx.clone();
            self.clipboard_probe = ClipboardProbe::start(make, PASTE_ORDER.to_vec(), Wake::new(move || repaint.request_repaint()));
        }
        if let Some(pasteable) = self.clipboard_probe.as_mut().and_then(|p| p.pasteable(wanted)) {
            self.system_paste = pasteable;
        } else if !(0.0..SYSTEM_CLIPBOARD_POLL).contains(&(now - self.system_paste_at)) {
            self.clipboard_probe = None;
            self.system_paste_at = now;
            self.system_paste = wanted && self.system_clipboard_pasteable();
        }
        if self.font_watch.is_none() {
            self.watch_fonts(ctx);
        }
        self.follow_fonts();
        file_asks::poll(self);
        io::save::pump(self, ctx);
        // A Paste the host read before this frame's formats were written must still wait for
        // them, even if the worker below finishes them during this same tick.
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("clipboard.pending-at-tick"), self.clipboard_publishing.is_pending()));
        screen_color::tick(self, ctx);
        // Once a frame: work that finishes during a frame's first pass is applied in the next frame, not in a second
        // pass of this one.
        if ctx.current_pass_index() == 0 {
            self.poll_finished_work();
        }
        crate::script_runs::poll(self);
        // A panel's, a dialog's or the text input's commit journaled in the frame before.
        crate::panels::actions::after_journaled_edit(self);
        crate::scripts_menu::poll(self, ctx);
        if self.session.script_running()
            && !self.session.script_waiting()
            && self.session.script_dialog().is_none()
            && self.session.script_picker().is_none()
        {
            // A script in `$.sleep` goes on when its time is up; until then the app can idle. A
            // script at a file dialog goes on when the dialog answers, and `file_asks::poll` looks
            // for that.
            match self.session.script_sleep_left() {
                Some(left) => ctx.request_repaint_after(left),
                None => ctx.request_repaint(),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.tick_link_updates(ctx, now);
        crate::addons::select::link_status::refresh(self, now);
        if !self.background.jobs.is_empty()
            || !self.opening.pending.is_empty()
            || !self.plugin_runs.running.is_empty()
            || !self.placing.pending.is_empty()
            || !self.trace_runs.running.is_empty()
        {
            // Keep the status bar's progress moving and pick the result up when it arrives.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(wait) = recovery::frame(self, now) {
            // The timer and heartbeat run in an idle window too.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        crate::keymap::note_os(ctx);
        prefs_dialog::apply_runtime(self, ctx);
        self.run_control(ctx);
        // In the next frame's first pass, as production's one-pass frames did: a second pass of the frame that asked
        // for it (one a background run's result can bring) doesn't open it early.
        if ctx.current_pass_index() == 0
            && let Some(u) = self.pending_url.take()
        {
            ctx.open_url(egui::OpenUrl::new_tab(u));
        }
        self.flush_clipboard_out();
        self.drain_inbox();
        // An action playing at Step by Step or Pause For runs its next step when it is due, after
        // the control requests and opened files above: a Stop or a document switch they bring
        // comes first.
        if let Some(wait) = crate::panels::actions::tick_playback(self, now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }

    /// The canvas panel's body: the document tabs, then the canvas (a pane for each document while
    /// Window › Arrange › Tile is on) and the isolation bar over it. While the current document is
    /// in a floating window, the tab strip's document draws as a pane.
    pub(crate) fn canvas_area(&mut self, ui: &mut egui::Ui) {
        let opening = !self.opening.pending.is_empty();
        let presenting = self.ui.presenting();
        // The tab strip and the document are two closures of one panel: they take the app in
        // turn, and a borrow that fails (a closure re-entered) skips its part.
        let app = std::cell::RefCell::new(self);
        CanvasPanel::new()
            .opening(opening)
            .tabs(|ui, _cx| {
                if !presenting && let Ok(mut app) = app.try_borrow_mut() {
                    chrome::doc_tabs(&mut app, ui);
                }
            })
            .show(ui, |ui, _cx| {
                let Ok(mut app) = app.try_borrow_mut() else { return };
                let app: &mut VectorcraftApp = &mut app;
                app.canvas.windows.area = Some(ui.available_rect_before_wrap());
                app.canvas.windows.last_noted = std::mem::take(&mut app.canvas.windows.noted);
                let floats = crate::doc_windows::current_floats(app);
                // The current window's body is in its floating window, drawn after this pane: the pane's drop catcher
                // waits for it ([`crate::doc_windows::show`]).
                app.canvas.panes.drops = floats.then(Vec::new);
                if !canvas::tiled_area(app, ui) {
                    if floats {
                        let rect = ui.available_rect_before_wrap();
                        if let Some(w) = crate::doc_windows::tab_window(app)
                            && canvas::pane(app, ui, w, rect)
                        {
                            crate::doc_windows::show_window(app, w);
                        }
                    } else {
                        canvas::show(app, ui);
                    }
                }
                // Over the canvas inside the rulers, so the rulers and scroll bars stay uncovered.
                if !presenting
                    && !floats
                    && !menus::home_showing(app)
                    && let Some(view) = app.canvas_view()
                {
                    chrome::isolation_bar(app, ui, view.rect);
                }
            });
    }

    /// Lay out the whole window, once per pass. The guard confines a pass that carries injected
    /// input to the automation roots and turns a panic into an error message.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.adopt_context(&ctx);
        let confined = self.automation_scope(&ctx);
        if let Err(msg) = file_access::confine(confined.as_ref(), || {
            vectorcraft_engine::guard::catch_panic(|| {
                self.handle_input(&ctx);
                self.ui_frame(ui);
                self.window_keys(&ctx);
            })
        }) {
            self.session.set_replaying(false);
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
        // After the pass's UI, which reads whether the pass carries injected input.
        self.poll_input(&ctx);
    }

    /// [`Self::ui`]'s work, inside its guard: the window's shell with its bars and the panels and
    /// canvas, then the windows, dialogs and messages over them.
    fn ui_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // A copy made by injected input is not the user's: the clipboard keeps it inside the app.
        self.clipboard.set_synthetic(WindowHost::of(&ctx).injected_pass());
        // A modal window opened between passes (the macOS menu bar, automation) closes the popups before they draw.
        panel_docking::close_overlays_for_modal(self, &ctx);
        // Whether this pass draws the last step of an action playing at Step by Step.
        let step_held = crate::panels::actions::playback_held(self);
        let t0 = monotonic_ms();
        panels::library_panel::tidy(self);
        let given = self.ui.screen_mode();
        let mut control_bar_place = chrome::control_bar_place(&self.ui);
        // PLAN: assumed `chrome::window_title(&VectorcraftApp) -> String` (shell-01 A1 names the call, no row gives its signature; shell-w9).
        let window_title = chrome::window_title(self);
        let shown = AppShell::new(WindowKey::main())
            .title_bar_if(true, chrome::app_bar)
            .options_bar(self.ui.control_bar, |ui, _cx, app: &mut VectorcraftApp| {
                chrome::control_bar(app, ui);
            })
            .options_place(Bind::bound(&mut control_bar_place))
            .status(self.ui.status_bar, |ui, _cx, app: &mut VectorcraftApp| {
                chrome::status_bar(app, ui);
            })
            .bottom(Slot::new(chrome::HINT_SLOT).visible(self.ui.help_bar), |ui, _cx, app: &mut VectorcraftApp| {
                chrome::hint_bar(app, ui);
            })
            .window_title(window_title)
            .screen_mode(Bind::given(given))
            .allow_reveal(!self.ui.presenting())
            .body(|ui, cx, app: &mut VectorcraftApp| {
                panel_docking::show(app, ui, cx.shell().is_some_and(|shell| shell.panels_hidden()));
                crate::place::window_files(app, ui);
            })
            .show(ui, self);
        chrome::set_control_bar_place(&mut self.ui, control_bar_place);
        if shown.screen_mode != given {
            self.ui.follow_screen_mode(shown.screen_mode);
        }
        panels::learn::locate::tidy(self, &ctx);
        crate::doc_windows::show(self, ui);
        export_progress::show(self, ui);
        // After every window this pass may open, and before the palette's field takes the keyboard.
        panel_docking::close_overlays_for_modal(self, &ctx);
        dialogs::host::pump(self, &ctx);
        prefs_dialog::show(self, ui);
        crate::shortcut_editor::show(self, ui);
        credits::open_taken_link(self);
        palette::show(self, ui);
        // A shape drawn on the canvas, in a dialog or by a command in this pass shows the Transform
        // panel (Show on Shape Creation).
        panels::transform::show_on_shape_creation(self, &ctx);
        for t in self.session.take_toasts() {
            toast::post(&ctx, &WindowKey::main(), Toast::new(t.message).tone(tone(t.level)).timeout(Some(t.seconds as f32)));
        }
        menus::native::sync(self, &ctx);
        // Only a field's commit in the pass of the press is the one it made.
        self.canvas.committed_preview = false;
        self.drop_formats_after_egui_copy(&ctx);
        // A command run by a menu, panel or dialog in this pass may have queued its text copy
        // after the input. Publish it now: an idle web host may have no further frame to run it in.
        self.flush_clipboard_out();
        crate::cursors::request_busy(self, &ctx);
        // A pass egui will discard and rerun is not a presented frame: it must not become the
        // frame-time sample.
        if !ctx.will_discard() {
            self.perf.frame_ms = monotonic_ms() - t0;
            // A hidden window's pass shows nothing, so the step waits for a pass the user sees.
            if step_held && !self.viewport_hidden(&ctx) {
                crate::panels::actions::playback_shown(self, ctx.cumulative_frame_nr());
            }
        }
        // Last, over everything: the screen colour picker's loupe and capture windows.
        screen_color::show(self, ui);
    }
}

/// The root values the preferences give every frame: the theme, the interface scale (UI Scaling
/// over the display's `native_ppp`), Show Tool Tips, the zoom stops and which drags scrub a value
/// (Scrub Numeric Fields: the side label only, or nothing).
pub(crate) fn root_env(prefs: &vectorcraft_engine::Prefs, native_ppp: f32) -> RootEnv {
    let (theme, mode, accent) = theme::root_theme(prefs);
    // A hand-edited preferences file must not scale the interface to nothing.
    let scaling = if prefs.ui_scaling.is_finite() && prefs.ui_scaling > 0.0 { prefs.ui_scaling as f32 } else { 1.0 };
    let mut root = RootEnv::new().theme(theme).mode(mode).accent(accent).scale(Scale::new(scaling)).tooltips(prefs.show_tool_tips);
    root.values.set(view::zoom_scheme(prefs, native_ppp));
    root.values.set(if prefs.scrub_numeric_fields { ScrubDrags::LabelOnly } else { ScrubDrags::Off });
    // A bare Tab on the canvas, or with nothing focused, toggles the panels (`window.dock`) instead of moving focus.
    root.tab_toggles_chrome = true;
    // Escape in a text field gives up its focus, reverting any typing, so the next keys are the window's.
    root.escape_leaves_fields = true;
    root
}

/// The toast tone of an engine message's level.
fn tone(level: Level) -> Tone {
    match level {
        Level::Info => Tone::Info,
        Level::Success => Tone::Good,
        Level::Warning => Tone::Warn,
        Level::Error => Tone::Bad,
    }
}
