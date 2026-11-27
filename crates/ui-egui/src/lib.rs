//! VectorCraft's egui frontend: menus, tools, panels and dialogs over `vectorcraft-engine`.
//!
//! The UI is thin: every action goes through [`VectorcraftApp::run`], which dispatches UI commands
//! (view/window) here and everything else to the engine. The same entry point serves menus,
//! shortcuts, the command palette ([`palette`], Help › Search Commands…) and the control channel ([`control`]).
#![forbid(unsafe_code)]

/// VectorCraft as craft-update knows it, with its 128 px icon and accent colour: the update
/// prompt shows both, and an AppImage's menu entry gets the icon.
#[cfg(not(target_arch = "wasm32"))]
pub const UPDATE_APP: craft::update::app::App = craft::update::app::App {
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png")),
    accent: Some([0xe8, 0x57, 0x3f]),
    ..craft::update::suite::VECTORCRAFT
};

/// Translate a UI string literal into the language the UI is drawn in (see [`i18n`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod addons;
pub mod background;
mod brand;
pub mod canvas;
pub mod chrome;
pub mod community;
pub mod control;
pub mod credits;
pub mod cursors;
mod custom_toolbars;
pub mod dialogs;
pub mod display_profile;
pub mod doc_windows;
pub mod dock;
mod edit_toolbar;
mod edits;
pub mod export_progress;
mod fields;
mod file_asks;
pub mod find_font;
pub mod font_menu;
mod frame_rate;
mod free_transform;
pub mod graphics;
pub mod hang_beacon;
pub mod i18n;
pub mod icon_data;
pub mod icons;
pub mod io;
pub mod keymap;
pub mod layout_migration;
pub mod menus;
pub mod opening;
mod original_editor;
pub mod os_window;
#[doc(hidden)]
pub use original_editor::Choice as OriginalEditorChoice;
mod page_glyphs;
mod paint_preview;
pub mod palette;
mod panel_docking;
mod panel_drag;
pub mod panels;
pub mod place;
pub mod placing;
pub mod plugin_runs;
pub mod prefs_dialog;
pub mod print;
pub mod recovery;
pub mod render_worker;
pub mod retype_runs;
pub mod screen_color;
pub mod script_runs;
pub mod scripts_menu;
mod settings;
pub mod shortcut_editor;
pub mod shortcut_sheet;
pub mod shortcuts;
pub mod state;
pub mod stylus;
pub mod sysclip;
pub mod theme;
pub mod thumb_worker;
pub mod toolbar;
pub mod touch;
pub mod trace_runs;
pub mod troubleshooting;
mod type_editing;
mod ui_fonts;
pub mod unsaved;
pub mod updates_ui;
pub mod workspaces;

#[cfg(test)]
use tests::{
    background as tests_background, folder as tests_folder, labels as tests_labels, picks as tests_picks, removeanchors as tests_removeanchors,
    svg as tests_svg, synthetic as tests_synthetic, sysclip as tests_sysclip,
};
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use craft::automation::{ControlChannel, ControlRequest};
use craft::behavior::GestureId;
use craft::host::Host;
use craft::ui::runtime::overlay::{self, OpenFilter};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_engine::{Session, UndoScope, ViewInfo};

pub use settings::{AppSettings, from_app, parse_legacy};
pub use state::{UiState, View};

mod app;
mod caches;
mod services;

pub use app::MAX_RECENT_FONTS;
pub use caches::{CacheKey, CanvasCache, LivePaintGapsCache, Perf, PlacedKey, PrintTilingCache, SliceCache};
#[cfg(test)]
pub(crate) use services::browsable_url;
pub use services::{DownloadFn, ExistsFn, Inbox, ReadFn, SaveStreamFn, SaveWriteFn, ScreenDensity, ScreenPpi, Services, WriteFn};
pub use ui_fonts::ui_fonts;

pub struct VectorcraftApp {
    pub session: Session,
    pub ui: UiState,
    pub views: Vec<View>,
    /// The document ([`vectorcraft_engine::DocState::uid`]) each of `views` belonged to at the last
    /// [`VectorcraftApp::sync_views`], which puts the views back with their documents after
    /// `document.move` reorders them (#5663).
    pub(crate) view_uids: Vec<u64>,
    /// The thread that checks the system clipboard for `system_paste` ([`Services::clipboard_probe`]).
    /// It is declared before `clipboard` so that it is dropped first: on X11 the last clipboard
    /// handle alive hands what the app copied to the clipboard manager, and that should be the
    /// app's own handle in `clipboard`, dropped on the UI thread before the process exits.
    clipboard_probe: Option<craft::platform::clipboard::ClipboardProbe>,
    /// The system clipboard: the host's backend, or one that keeps copies inside the app.
    pub clipboard: craft::platform::clipboard::Clipboard,
    pub services: Services,
    /// The app's services as the framework reads them (language, catalogs, jobs, files), handed
    /// to every frame in [`Self::frame_input`].
    pub host: Arc<Host>,
    pub canvas: CanvasCache,
    /// The main window's display profile, which the canvas is drawn through
    /// ([`display_profile`]); the desktop app reports what it reads into it.
    pub display_profile: display_profile::DisplayProfile,
    pub perf: Perf,
    /// Docking state that lives for the session only.
    pub(crate) dock_rt: panel_docking::Runtime,
    /// macOS: the room the app bar keeps at its left for the window buttons over it, in points
    /// (0 where the OS draws no buttons over the app bar).
    pub titlebar_inset: f32,
    /// Last applied effect (Effect → Apply Last Effect).
    pub last_effect: Option<(String, serde_json::Value)>,
    /// Last successful destructive Pathfinder operation, repeated on the current selection.
    pub last_pathfinder: Option<String>,
    /// The parameters each object filter plug-in last ran with (Object › Plug-ins › Apply Last Filter).
    pub last_filters: dialogs::plugin::LastFilters,
    /// The Character panel field that takes the keyboard focus when the panel next draws it
    /// ([`panels::character::highlight`]).
    pub(crate) character_focus: Option<panels::character::Field>,
    /// Commands run through [`Self::run`] so far: the native menu bar reads its rows again when it
    /// moves ([`menus::native::sync`]).
    run_count: u64,
    /// The control channel's requests and their replies, while the host opened one.
    pub(crate) channel: Option<ControlChannel>,
    /// A `ui.pointer` or `ui.tool.select` held until the synthetic input sent before it reached the tool.
    deferred_control: Option<ControlRequest>,
    /// Where a control-channel Rotate View drag began, kept between its requests.
    pub(crate) control_rotate_press: Option<control::RotateViewPress>,
    /// The `ui.pointer` replies that wait for the edit their gesture started on another thread.
    pub(crate) pointer_waits: Vec<control::PointerWait>,
    /// The egui context the UI's textures were uploaded to (0: none yet; see
    /// [`Self::adopt_context`]).
    context: u64,
    /// The current context (set by [`Self::adopt_context`]): the dialogs, the overlays and the
    /// window host are read through it.
    ui_ctx: Option<egui::Context>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) updates_rt: updates_ui::Runtime,
    /// The status bar's frame rate ([`frame_rate`]).
    frame_rate: frame_rate::FrameRate,
    /// Hover position in document coordinates.
    pub hover_doc: Option<vectorcraft_geom::Point>,
    /// Text to copy to the system clipboard next (`flush_clipboard_out`) and what came with a
    /// paste (a Paste event's text, a picture or file the host read: [`Self::paste_from_host`]).
    clipboard_out: Option<String>,
    /// The text of the latest copy the fallback clipboard took (no host clipboard: tests, the web before its host
    /// sets one); it goes out through egui ([`Self::flush_clipboard_out`]).
    fallback_copy: Arc<Mutex<Option<String>>>,
    pub(crate) clipboard_in: Option<vectorcraft_engine::cmd::clipboard::Flavour>,
    /// The Copy or Cut whose formats go to the system clipboard ([`Self::publish_clipboard`]).
    clipboard_publishing: sysclip::Publishing,
    /// A URL to open through egui in the next frame's tick.
    pending_url: Option<String>,
    /// `VECTORCRAFT_TITLEBAR` chose this window's frame (Linux and the BSDs), so Preferences › User
    /// Interface › System Title Bar and VectorCraft Title Bar on Every Desktop do not apply while
    /// it is set. `false` unless the host sets it.
    pub titlebar_from_env: bool,
    /// Linux and the BSDs: `VECTORCRAFT_TITLEBAR` is unset and the desktop would draw the window's
    /// frame on this session with Preferences › User Interface › System Title Bar on or off while
    /// VectorCraft Title Bar on Every Desktop is off, so the dialog says so under System Title Bar.
    /// Set by the desktop app from the frame decision when it opens the window; `false` elsewhere.
    pub desktop_keeps_frame: bool,
    /// The graphics adapter the window renders with ("name (backend)"), as the host reports it:
    /// shown in Help › About and `ui.inspect` for GPU bug reports. `None` when unknown.
    pub graphics_adapter: Option<String>,
    /// Where [`Self::run`] marks each command for the desktop app's hang report:
    /// [`hang_beacon::BEACON`], or a test's own.
    pub(crate) beacon: &'static hang_beacon::Beacon,
    /// File → Place: picked files, the place cursor's thumbnails, the Control bar's image details.
    pub place: place::PlaceState,
    /// The Paste commands can paste from the system clipboard alone: it holds something to paste
    /// while the internal clipboard is empty. It enables the Paste menu items.
    pub(crate) system_paste: bool,
    /// When `system_paste` was last checked in line (app time, s; at most once per frame).
    system_paste_at: f64,
    /// File dialogs shown off the UI thread, what runs again with their answers, and the replace
    /// questions.
    pub(crate) file_asks: file_asks::FileAsks,
    /// The saves in progress, one flow per document.
    pub(crate) saves: io::save::Saves,
    pub(crate) original_editor: original_editor::Draft,
    /// The screen colour picker: the pick in progress and where its colour goes.
    pub(crate) screen_pick: craft::color_ui::ui::ScreenPick,
    /// The font catalog's watch for fonts installed or removed while the app runs.
    pub(crate) font_watch: Option<craft::font_catalog::Watch>,
    /// Set by [`Self::font_watch`] when it found fonts installed or removed, until the documents' fonts are scanned
    /// again for them.
    pub(crate) fonts_changed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The font menu whose live preview the canvas shows, while one does.
    pub(crate) font_preview_owner: Option<egui::Id>,
    /// Saves and exports running in the background (Preferences → File Handling).
    pub background: background::Background,
    /// Documents being read off the UI thread ([`opening`]).
    pub opening: opening::Opening,
    /// Heavy plug-in filters running off the UI thread ([`plugin_runs`]).
    pub plugin_runs: plugin_runs::PluginRuns,
    /// Files being read off the UI thread for Place ([`placing`]).
    pub placing: placing::Placing,
    /// Image Traces running off the UI thread ([`trace_runs`]).
    pub trace_runs: trace_runs::TraceRuns,
    /// Retype's search of the installed fonts, off the UI thread ([`retype_runs`]).
    pub retype_runs: retype_runs::RetypeRuns,
    /// Data Recovery's timer and startup question ([`recovery`]).
    pub recovery: recovery::Timer,
    /// The Close, Close All or Quit in progress, which asks about each unsaved document in turn.
    pub(crate) closing: Option<unsaved::Closing>,
    /// The Preferences window, while it is open.
    pub(crate) prefs: Option<Box<prefs_dialog::PrefsWindow>>,
    /// The preferences the interface last applied, to apply only what changed.
    pub(crate) applied_prefs: Option<prefs_dialog::Applied>,
    /// The menu bar's model and the command states the frame publishes.
    pub(crate) menus: menus::model::Menus,
    /// The Keyboard Shortcuts dialog, while it is open.
    pub(crate) shortcuts_dialog: Option<shortcut_editor::ShortcutsDialogState>,
    /// The bindings the active shortcut set changes, as the key stage takes them: rebuilt from
    /// [`UiState::shortcut_sets`] when the set changes.
    pub(crate) shortcut_overrides: Arc<craft::keymap::Overrides>,
    /// Messages waiting for the dialog that shows them.
    pub(crate) told: dialogs::host::Told,
    /// The Type tool's pending edit, kept even when the dialog closes without a prompt answer.
    pub(crate) missing_font_edit_token: Option<u64>,
    /// The hosted dialogs whose OK the app answers, one record each.
    pub(crate) held: dialogs::host::Held,
    /// The document whose settings the open Document Setup dialog edits.
    pub(crate) document_setup_source: Option<(u64, u64)>,
    /// The undo step each value gesture in progress edits under: a scrub or a slider drag is one
    /// step however many values it sets.
    pub(crate) edit_scopes: HashMap<GestureId, UndoScope>,
    /// The parameter dialog of a command, while one is open.
    pub(crate) command_dialog: dialogs::command::CommandDialog,
    /// The open dialogs of the Effect menu.
    pub(crate) effect_dialogs: dialogs::effect::EffectDialogs,
    /// The 3D and Materials dialog, while it is open.
    pub(crate) three_d_dialog: Option<dialogs::three_d::Session>,
    /// The 3D and Materials panel's model.
    pub(crate) three_d: panels::three_d::Model,
    /// The open export option dialogs and what runs after each.
    pub(crate) export_dialogs: dialogs::export_host::ExportDialogs,
    /// The drawing the open Package dialog saves and collects.
    pub(crate) package_document: Option<u64>,
    /// Save for Web's renders and colour table.
    pub(crate) web_export: dialogs::save_for_web::WebExport,
    /// The Flattener Preview panel's model.
    pub(crate) flattener_preview: panels::flattener_preview::FlattenerPreviewModel,
    /// The Flatten Transparency dialog's record.
    pub(crate) flatten_dialog: dialogs::flatten::FlattenDialog,
    /// The Print dialogs' records.
    pub(crate) print_dialog: dialogs::print::PrintDialogs,
    /// The Save PDF dialog's record, while it is open.
    pub(crate) save_pdf: Option<dialogs::save_pdf::SavePdfOpen>,
    /// The open dialogs of the Object menu.
    pub(crate) object_dialogs: dialogs::object_dialogs::ObjectDialogs,
    /// The open dialogs of the colour area.
    pub(crate) color_dialogs: dialogs::color_picker::ColorDialogs,
    /// The plug-in dialog, while one is open.
    pub(crate) plugin_dialog: Option<dialogs::plugin::PluginDialog>,
    /// The add-on panels' session state.
    pub(crate) addon_panels: addons::Panels,
    /// The add-on dialogs' records.
    pub(crate) addon_dialogs: addons::dialogs::AddonDialogs,
    /// Check Spelling's walk through the documents' type, while its dialog is open.
    pub(crate) spelling: Option<dialogs::check_spelling::SpellingWalk>,
    /// The Character Style Options dialog's live preview.
    pub(crate) char_style_preview: dialogs::char_style_options::Preview,
    /// The Paragraph Style Options dialog's live preview.
    pub(crate) para_style_preview: dialogs::preview::LivePreview,
    /// The Justification dialog, while it is open.
    pub(crate) justification: Option<dialogs::justification::Open>,
    /// The Bullets and Numbering dialog's live preview.
    pub(crate) bullets_preview: Option<dialogs::bullets_numbering::Preview>,
    /// The paste that waits for the answer to its colour profile question.
    pub(crate) pending_paste: Option<dialogs::profile_mismatch::PasteAsk>,
    /// The missing links' questions, while they are asked.
    pub(crate) link_asks: Option<dialogs::missing_links::LinkAsk>,
    /// The marked text the system IME last sent (`None` once it commits or clears). When the Type
    /// tool stops composing on its own (a click, a tool switch), the IME is told to drop it.
    pub(crate) ime_marked: Option<String>,
    /// Owner/generation/change-cause bookkeeping for the outgoing IME surrounding-text producer
    /// (one tracker serves every viewport).
    pub(crate) ime_context: egui::ime_context::ImeContextTracker,
    /// Preferences were confirmed (see [`Self::take_prefs_changed`]).
    pub(crate) prefs_changed: bool,
    /// Whether the last frame's input reported a touch screen (Preferences › Devices › Touch Workspace).
    pub touch_screen: bool,
    /// The folders control requests, and the input they inject, may read and write
    /// (`--automation-read-root`, `--automation-write-root`); `None`: anywhere. See
    /// [`Self::with_automation_roots`].
    automation_roots: Option<Arc<AutomationRoots>>,
    /// The plug-in install the approval dialog asks about, kept out of the dialog's fields, which
    /// `ui.dialog.set` writes.
    pub(crate) pending_plugin_install: Option<dialogs::plugin::PendingInstall>,
    /// The file File › Open read that waits for the answer to its profile question
    /// ([`dialogs::profile_mismatch`]), kept out of the dialog's fields.
    pub(crate) pending_profile_open: Option<dialogs::profile_mismatch::PendingOpen>,
    /// Files File › Open read while another one's profile question was open, each asking its own
    /// question in turn.
    pub(crate) queued_profile_opens: std::collections::VecDeque<dialogs::profile_mismatch::PendingOpen>,
    /// The pen end the canvas saw last and the tool its eraser end stands in for
    /// ([`stylus::sync_eraser_tool`]).
    pub(crate) pen_end: stylus::PenEnd,
    /// The window pass whose [`Self::tick`] ran last: work started in that pass, before its end, waits for the next.
    pub(crate) ticked_pass: Option<u64>,
    /// A test's clock for [`Self::tick`] and the progress window's delay, replacing the monotonic
    /// clock so idle scheduling can be driven one simulated frame at a time (production always
    /// leaves this `None`).
    #[cfg(test)]
    pub(crate) now_override: Option<f64>,
    /// An error the next tool choice returns before the engine sees it ([`Self::try_select_tool`]),
    /// so tests can drive a failed switch (production has no such field).
    #[cfg(test)]
    pub(crate) fail_next_tool_choice: Option<String>,
    /// The single-key tool shortcuts held down, the last held last, and the tool each goes back to
    /// when it is let go ([`shortcuts::HeldTool`]).
    pub(crate) held_tool: Vec<shortcuts::HeldTool>,
}

impl VectorcraftApp {
    /// Whether a layer that takes the keys (a modal dialog, a popup) showed in the last pass, or a
    /// modal dialog opened since: a command or a canvas gesture waits while one does. A modeless
    /// dialog blocks nothing, and the job progress window does not count (its layer has the
    /// progress role). False before the app has a context.
    pub(crate) fn any_dialog_open(&self) -> bool {
        // A modeless layer with a focus scope (the canvas's task bar) takes keys too, and blocks nothing.
        let popup = |ctx: &egui::Context| overlay::top(ctx).and_then(|id| overlay::entry(ctx, id)).is_some_and(|e| e.rank == overlay::Rank::Popup);
        self.ui_ctx.as_ref().is_some_and(|ctx| overlay::any_open(ctx, OpenFilter::Modal) || popup(ctx))
            || self.dialogs().is_some_and(|d| d.with(|h| h.shows_modal()))
    }

    /// The background job service: the one the window's host installed, the app's own host (the one
    /// it gives the window) until the window has installed it, or, before the app has a window (unit
    /// tests, the CLI), the platform's default: a worker thread per job, in line on the web.
    pub(crate) fn jobs(&self) -> Arc<dyn craft::host::Jobs> {
        match (&self.ui_ctx, self.ui_ctx.as_ref().and_then(Host::try_of_ctx)) {
            (_, Some(host)) => host.jobs_shared().clone(),
            (Some(_), None) => self.host.jobs_shared().clone(),
            (None, None) => craft::host::default_jobs(),
        }
    }
}
