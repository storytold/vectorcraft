//! VectorCraft's egui frontend: an Illustrator-style UI over `vectorcraft-engine`.
//!
//! The UI is thin: every action goes through [`VectorcraftApp::run`], which dispatches UI commands
//! (view/window) here and everything else to the engine. The same entry point serves menus,
//! shortcuts, the ⌘K palette and the control channel ([`control`]).
#![forbid(unsafe_code)]

/// VectorCraft as craft-update knows it, with its 128 px icon and accent colour: the update
/// prompt shows both, and an AppImage's menu entry gets the icon.
#[cfg(not(target_arch = "wasm32"))]
pub const UPDATE_APP: craft_update::app::App = craft_update::app::App {
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png")),
    accent: Some([0xe8, 0x57, 0x3f]),
    ..craft_update::suite::VECTORCRAFT
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
mod clipboard_probe;
pub mod community;
pub mod control;
pub mod credits;
pub mod cursors;
pub mod dialogs;
pub mod dock;
mod edit_toolbar;
pub mod export_progress;
pub mod find_font;
pub mod font_menu;
mod frame_rate;
mod free_transform;
pub mod graphics;
pub mod i18n;
pub mod icon_data;
pub mod icons;
pub(crate) mod ime;
pub mod io;
pub mod keymap;
pub mod layout_migration;
pub mod menus;
pub mod native_menu;
pub mod opening;
pub mod palette;
mod panel_docking;
pub mod panels;
pub mod picks;
pub mod place;
pub mod placing;
pub mod plugin_runs;
pub mod prefs_dialog;
pub mod print;
pub mod recovery;
pub mod render_worker;
pub mod screen_color;
mod scrub;
pub mod shortcut_editor;
pub mod shortcuts;
pub mod state;
pub mod stylus;
pub mod sysclip;
pub mod theme;
pub mod thumb_worker;
pub mod titlebar;
pub mod toolbar;
mod tools_float;
pub mod touch;
pub mod trace_runs;
mod ui_fonts;
pub mod unsaved;
pub mod updates_ui;
pub mod widgets;
pub mod workspaces;

mod document_drag;
#[cfg(test)]
use tests::{
    background as tests_background, labels as tests_labels, picks as tests_picks, removeanchors as tests_removeanchors, svg as tests_svg,
    synthetic as tests_synthetic, sysclip as tests_sysclip,
};
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_engine::{Session, ViewInfo};

pub use control::{ControlRequest, ControlResponse, Ticket as ControlTicket};
pub use state::{UiState, View};
pub use sysclip::{ClipboardProbeFactory, SystemClipboard};

mod app;
mod caches;
mod clock;
mod services;

pub use app::MAX_RECENT_FONTS;
pub use caches::{CacheKey, CanvasCache, LivePaintGapsCache, Perf, PrintTilingCache, SliceCache};
pub use clock::{now_ms, now_secs};
#[cfg(test)]
pub(crate) use services::browsable_url;
pub use services::{
    DownloadFn, ExistsFn, FileFilter, FilePick, Inbox, NativeInputHook, OpenUrlFn, PickOpen, PickSave, ReadFn, RevealFn, Services, WriteFn,
};
use state::{GeometryStamp, InputGeneration, PendingScreenshot, QueuedScreenshot};
pub(crate) use state::{InputFrame, take_frame_once};

pub struct VectorcraftApp {
    pub session: Session,
    pub ui: UiState,
    pub views: Vec<View>,
    pub services: Services,
    pub canvas: CanvasCache,
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
    /// Last confirmed Warp Options (bend, distortion, orientation) for the next new warp dialog.
    pub last_warp: Option<serde_json::Value>,
    /// The Character panel field that takes the keyboard focus when the panel next draws it
    /// ([`panels::character::highlight`]).
    pub(crate) character_focus: Option<panels::character::Field>,
    /// Commands run through [`Self::run`] so far: the native menu bar reads its rows again when it
    /// moves ([`native_menu::sync`]).
    run_count: u64,
    control_rx: Option<Receiver<ControlRequest>>,
    /// Window captures asked for and not answered yet.
    pending_screenshots: Vec<PendingScreenshot>,
    /// Admitted captures whose settled frame has not come yet (issued one at a time).
    queued_screenshots: Vec<QueuedScreenshot>,
    screenshot_token: u64,
    /// Bytes reserved by admitted captures (checked `width * height * 4`).
    screenshot_reserved: usize,
    /// Synthetic input events (from the control channel) injected one step per frame.
    pub synthetic: Vec<egui::Event>,
    styled: bool,
    fonts_ready: bool,
    /// The egui context the UI's textures were uploaded to (0: none yet; see
    /// [`Self::adopt_context`]).
    context: u64,
    /// The current context, for the focus part of an injected batch's generation (set by
    /// [`Self::adopt_context`]).
    ui_ctx: Option<egui::Context>,
    /// Installed fonts added to the UI's for characters its own fonts lack (CJK names…).
    ui_fonts: ui_fonts::UiFonts,
    pub(crate) updates_rt: updates_ui::Runtime,
    /// The status bar's frame rate ([`frame_rate`]).
    frame_rate: frame_rate::FrameRate,
    /// Canvas rect of the last frame (screen points), for control-channel coordinate mapping.
    pub canvas_rect: Option<egui::Rect>,
    /// Hover position in document coordinates.
    pub hover_doc: Option<vectorcraft_geom::Point>,
    /// System clipboard: SVG to publish next frame, the last SVG we published (so pasting it back
    /// uses the lossless internal clipboard) and what came with a paste (a Paste event's text, a
    /// picture or file the host read: [`Self::paste_from_host`]).
    clipboard_out: Option<String>,
    clipboard_published: Option<String>,
    pub(crate) clipboard_in: Option<vectorcraft_engine::cmd::clipboard::Flavour>,
    /// The Copy or Cut whose formats go to the system clipboard ([`Self::publish_clipboard`]).
    clipboard_publishing: sysclip::Publishing,
    /// A URL to open through egui next frame (when the host has no `open_url` service).
    pending_url: Option<String>,
    /// Windows and Linux: the window has no OS decorations, so the app bar is the title bar (drag,
    /// double-click to maximize, caption buttons) and invisible edge zones resize the window.
    pub custom_titlebar: bool,
    /// Last window title sent to the OS (`ViewportCommand::Title`, see `chrome::sync_window_title`):
    /// sent again only when it changes, so idle frames don't spam the backend.
    pub(crate) last_window_title: String,
    /// Whether a screen mode put the window full screen (`chrome::sync_fullscreen`), so Normal
    /// Screen Mode takes it back out, and leaves a window the user made full screen as it is.
    pub(crate) fullscreen_for_mode: bool,
    /// The graphics adapter the window renders with ("name (backend)"), as the host reports it:
    /// shown in Help › About and `ui.inspect` for GPU bug reports. `None` when unknown.
    pub graphics_adapter: Option<String>,
    /// File → Place: picked files, the place cursor's thumbnails, the Control bar's image details.
    pub place: place::PlaceState,
    /// The Paste commands can paste from the system clipboard alone: it holds something to paste
    /// (SVG without a `system_clipboard`) while the internal clipboard is empty. It enables the
    /// Paste menu items.
    pub(crate) system_paste: bool,
    /// The modifiers held when an egui `PasteImage` asked the rich system clipboard to be read.
    /// Consumed once by the next `tick`; storing the paste-time chord keeps a later pass's stale
    /// modifiers from choosing Paste in Place instead of Paste (or the reverse).
    pub(crate) clipboard_image_request: Option<egui::Modifiers>,
    /// When `system_paste` was last checked in line (app time, s; at most once per frame).
    system_paste_at: f64,
    /// The thread that checks the system clipboard for `system_paste` instead
    /// ([`Services::clipboard_probe`]).
    clipboard_probe: Option<clipboard_probe::Probe>,
    /// File dialogs shown off the UI thread and what runs again with their answers.
    pub(crate) picks: picks::Picks,
    /// The screen color sampler shown and not answered yet, and where its color goes.
    pub(crate) screen_sampler: Option<screen_color::Waiting>,
    /// The look for fonts installed or removed meanwhile, running
    /// ([`Self::refresh_installed_fonts`]): whether they were, and whether to wake the app for
    /// the answer.
    font_check: Option<(std::sync::mpsc::Receiver<bool>, bool)>,
    /// When installed fonts were last checked in line (app time, s).
    #[cfg(not(target_arch = "wasm32"))]
    font_install_check_at: f64,
    /// Keyboard pastes of something other than text (see [`shortcuts::PasteChord`]).
    pub(crate) paste_chord: shortcuts::PasteChord,
    /// Runs the frame's key presses against the keymap ([`shortcuts::handle`]).
    pub(crate) dispatcher: craft_keymap::egui::Dispatcher,
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
    /// Data Recovery's timer and startup question ([`recovery`]).
    pub recovery: recovery::Timer,
    /// Modifiers the keyboard holds (from the host's input), given back after synthetic input.
    host_modifiers: egui::Modifiers,
    /// Key, text and clipboard events a floating panel's own OS window forwarded, for the main
    /// window's next frame ([`Self::raw_input_hook`]), and the modifiers held in that window.
    pub(crate) forwarded: Vec<egui::Event>,
    pub(crate) forwarded_modifiers: Option<egui::Modifiers>,
    /// The last frame held the forwarded modifiers: this one goes back to the keyboard's.
    forwarded_modifiers_held: bool,
    /// This host iteration's input carries forwarded keys: they are the main window's although
    /// its OS window doesn't have the keyboard (see `shortcuts::handle`).
    pub(crate) forwarded_frame: bool,
    /// Synthetic input set the modifiers egui holds (see [`Self::raw_input_hook`]).
    synthetic_modifiers: bool,
    /// The marked text the system IME last sent (`None` once it commits or clears). When the Type
    /// tool stops composing on its own (a click, a tool switch), the IME is told to drop it.
    pub(crate) ime_marked: Option<String>,
    /// Owner/generation/change-cause bookkeeping for the outgoing IME surrounding-text producer
    /// (one tracker serves every viewport; see the crate's IME producer module).
    pub(crate) ime_context: egui::ime_context::ImeContextTracker,
    /// Incoming composition ownership is separate from outgoing receipt/generation bookkeeping.
    /// Interrupted input stays stale until the exact request-bound native fence arrives.
    pub(crate) ime_compositions: egui::ViewportIdMap<ime::IncomingComposition>,
    pub(crate) ime_resets: egui::ViewportIdMap<ime::ResetGuard>,
    /// The Type tool's IME caret geometry for this pass, recorded by the canvas. The descriptor
    /// itself is written at the end of the pass ([`ime::end_of_pass`]), after dialogs, floating
    /// panels and the palette, because any of them can move the owner, edit the text or cover the
    /// canvas after the canvas drew.
    pub(crate) ime_caret_rect: Option<(egui::Rect, egui::Rect)>,
    /// The IME must drop its marked text (see [`Self::take_ime_discard`]).
    pub(crate) ime_discard: bool,
    /// Preferences were confirmed (see [`Self::take_prefs_changed`]).
    pub(crate) prefs_changed: bool,
    /// Whether the last frame's input reported a touch screen (Preferences › Devices › Touch Workspace).
    pub touch_screen: bool,
    /// A numeric field is being scrubbed: the document's edits meanwhile are one undo step
    /// ([`scrub::begin_frame`]).
    scrub_group: bool,
    /// The undo step of the scrub in progress ([`vectorcraft_engine::Session::begin_undo_scope`]).
    scrub_scope: Option<vectorcraft_engine::UndoScope>,
    /// The folders control requests, and the input they inject, may read and write
    /// (`--automation-read-root`, `--automation-write-root`); `None`: anywhere. See
    /// [`Self::with_automation_roots`].
    automation_roots: Option<Arc<AutomationRoots>>,
    /// This frame carries input the control channel injected ([`Self::raw_input_hook`]): it runs
    /// confined to [`Self::automation_roots`].
    synthetic_frame: bool,
    /// The pen end the canvas saw last and the tool its eraser end stands in for
    /// ([`stylus::sync_eraser_tool`]).
    pub(crate) pen_end: stylus::PenEnd,
    /// Host iterations (full or logic-only), advanced once in [`Self::begin_host_iteration`].
    pub(crate) host_iteration: u64,
    /// A test's clock for [`Self::tick`], replacing the monotonic [`now_secs`] so idle scheduling
    /// can be driven one simulated frame at a time (production always leaves this `None`).
    #[cfg(test)]
    pub(crate) now_override: Option<f64>,
    /// The host iteration [`Self::tick`] ran in (`None`: never): once per iteration, never per pass.
    tick_iteration: Option<u64>,
    /// The context, viewport and frame whose UI-phase input was already taken in this host
    /// iteration: a discarded rerun and a second viewport pass must not replay a frame's events.
    input_frames: Vec<InputFrame>,
    /// The context, viewport and frame `shortcuts::handle` already took in this host iteration.
    /// Interleaving root and auxiliary viewport passes cannot make an older frame look new again.
    pub(crate) shortcuts_frames: Vec<InputFrame>,
    /// A synthetic batch is in the host's raw input and no settled UI pass has taken it yet.
    synthetic_injected: bool,
    /// The exact events of [`Self::synthetic_injected`], to remove them if the state they belong
    /// to changed before a UI pass could take them.
    injected_events: Vec<egui::Event>,
    /// The document, dialog and focus an injected batch belongs to.
    injected_generation: InputGeneration,
    /// The end-of-pass observer was installed on the context.
    injected_settle_hook: bool,
    /// A settled UI pass consumed the injected batch (read by the next host iteration).
    injected_settled: Arc<AtomicBool>,
    /// A change of the window's content geometry (resize, DPI, zoom or surface), even back again.
    geometry_epoch: u64,
    /// The geometry stamp [`Self::geometry_epoch`] was last advanced for.
    geometry_stamp: Option<GeometryStamp>,
}
