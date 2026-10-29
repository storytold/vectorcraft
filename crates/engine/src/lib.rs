//! The VectorCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`object.group`, `select.same.fillColor`,
//! `shape.rectangle`…) and JSON parameters. The egui UI, the CLI, the control channel and the MCP
//! server all go through [`Session::execute`]. Tools (pointer gestures) are hosted here too and
//! reduce to commands, so every gesture is journaled and replayable.
#![forbid(unsafe_code)]

pub mod cmd;
pub mod file_access;
pub mod guard;
pub mod inspect;
pub mod journal;
pub mod link_watch;
pub mod steps;
pub mod store;
mod tooling;
pub mod units;

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_color::{Color, GradientPaint, Paint};
use vectorcraft_doc::{Document, NodeId, NodeKind, Selection};
use vectorcraft_geom::Affine;
use vectorcraft_tools::{PaintDefaults, Tool};

pub use cmd::clipboard::Clipboard;
pub use cmd::create::constrain_click;
pub use cmd::distortcmds::perspective_click;
pub use cmd::params::{check_keys, check_params};
pub use cmd::pathops::auto_simplify_level;
pub use cmd::rasterfx::{export_pdf, flatten_raster_effects};
pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use cmd::{EyedropperOptions, PathfinderOptions};
pub use journal::Journal;
pub use tooling::{UiRequest, ViewInfo};
pub use vectorcraft_doc as doc;
pub use vectorcraft_doc::cancel;
pub use vectorcraft_pathops::{SIMPLIFY_LEVEL, SMOOTH_AUTO_LEVEL, simplify_level_tolerance};
pub use vectorcraft_tools as tools;

/// Coordinates beyond this (points) are rejected: ~1,400 m, far past Illustrator's large canvas.
pub use vectorcraft_doc::MAX_COORD;
const _: () = assert!(vectorcraft_doc::MAX_DEPTH == 32, "TOO_DEEP (and its catalog rows) names the cap");

pub struct Session {
    docs: Vec<DocState>,
    active: Option<usize>,
    pub prefs: Prefs,
    /// Fill/stroke for new art (the toolbar proxy).
    pub paint: PaintDefaults,
    /// Which proxy is in front (true = Fill, false = Stroke) — the X key toggles.
    pub fill_active: bool,
    /// Internal clipboard: the copied objects and the document resources they use.
    pub clipboard: Clipboard,
    /// Executed commands (for actions and debugging), the most recent ones kept ([`Journal`]).
    pub journal: Journal,
    pub(crate) tool: Box<dyn Tool>,
    /// While Cmd lends `tool` (a selection tool) for a drag: the tool it was lent to, which comes
    /// back as it was at the release ([`Session::pointer`]).
    pub(crate) lender: Option<Box<dyn Tool>>,
    /// The selection tool chosen last (Selection, Direct Selection or Group Selection): the one Cmd
    /// lends the other tools. None until one is chosen.
    pub(crate) last_selection_tool: Option<&'static str>,
    pub(crate) last_view: ViewInfo,
    /// The selection tools' measurements for the active document as it is: a pointer move or a
    /// cursor asked for again over an unchanged document and selection measures nothing again.
    pub(crate) preview_memo: vectorcraft_tools::select::PreviewMemo,
    depth: u32,
    /// Set when the active tool panicked (see [`guard`]); reported by the next tool event.
    tool_panic: Option<EngineError>,
    /// How many times switching or closing documents, or replacing the active one, has started the
    /// tool afresh ([`Session::tool_resets`]).
    tool_resets: u64,
    /// Whether the tool's options were set after it last started afresh
    /// ([`Session::tool_set_since_reset`]).
    tool_set_since_reset: bool,
    /// Draw Normal / Behind / Inside (toolbar drawing modes).
    pub draw_mode: DrawMode,
    /// The path new art is drawn inside (Draw Inside).
    pub draw_inside: Option<NodeId>,
    untitled_counter: u32,
    /// Session-level state of the menu commands (saved selections, guide lock).
    pub(crate) menu: cmd::menucmds::MenuState,
    /// The selected gradient stop (`gradient.selectStop`) and whose gradient it was selected on,
    /// shared by the Gradient tool's annotator, the Gradient and Color panels and agents: read it
    /// with [`Session::selected_stop`].
    pub(crate) gradient_stop: Option<(usize, cmd::gradient::StopOwner)>,
    /// The Appearance panel's active fill/stroke row (`appearance.setActiveItem`); not saved. Read
    /// it through [`Session::appearance_item`], which drops it once the selection changes.
    pub(crate) active_appearance_item: Option<cmd::appearance::ActiveItem>,
    /// What the Eyedropper picked up last (`appearance.copyFrom`): an Alt-click with nothing
    /// selected puts it on the object clicked (#4382). Not saved.
    pub(crate) eyedropper_sample: Option<Arc<vectorcraft_doc::Node>>,
    /// The last solid colour applied (the toolbar's Color button; `,` applies it again).
    pub last_solid: Color,
    /// The last gradient applied (the toolbar's Gradient button; `.` applies it again).
    pub last_gradient: GradientPaint,
    /// Recently applied solid colours, newest first (the Recent Colors rows), fed by every paint
    /// command whichever frontend runs it.
    pub recent_colors: Vec<Color>,
    /// A paint applied by a live preview: remembered when the interaction commits.
    pub(crate) pending_paint: Option<Paint>,
    /// User Defined and loaded swatch libraries (Window → Swatch Libraries); not saved.
    pub swatch_libraries: cmd::swatchlib::Libraries,
    /// The selected freeform gradient point (`paint.freeform.selectPoint`) and whose gradient it
    /// was selected on: read it with [`Session::selected_freeform_point`].
    pub(crate) freeform_point: Option<(usize, cmd::gradient::StopOwner)>,
    /// User Defined and loaded graphic style libraries (Window → Graphic Style Libraries); not saved.
    pub style_libraries: cmd::stylelib::Libraries,
    /// User Defined and loaded symbol libraries (Window → Symbol Libraries); not saved.
    pub symbol_libraries: cmd::symbollib::Libraries,
    /// User Defined and loaded brush libraries (Window → Brush Libraries); not saved.
    pub brush_libraries: cmd::brushlib::Libraries,
    /// The Libraries panel's libraries of graphics, colours and text styles (`library.*`).
    pub libraries: cmd::library::Libraries,
    /// Where the saved PDF presets are kept, one file each; `None`: only for the session
    /// ([`Session::attach_pdf_preset_store`]).
    pdf_preset_dir: Option<std::path::PathBuf>,
    /// Where the ICC profiles `color.loadProfile` loads are kept; `None`: only for the process
    /// ([`Session::attach_profile_store`]).
    profile_dir: Option<std::path::PathBuf>,
    /// Where the plug-ins `plugin.install` installs are kept; `None`: only for the process
    /// ([`Session::attach_plugin_store`]).
    plugin_dir: Option<std::path::PathBuf>,
    /// A filter's output that a run made in the background handed in for `plugin.run` to apply
    /// ([`cmd::plugin::FilterRun::hand_in`]); the next `plugin.run` takes it.
    plugin_output: Option<cmd::plugin::HandedIn>,
    /// URLs recently given in the Attributes panel (`attributes.set {url}`), newest first; not saved.
    pub recent_urls: Vec<String>,
    /// The saved tracing presets' revision and latest changes ([`Session::trace_presets_revision`]).
    pub(crate) trace_preset_changes: cmd::TracePresetChanges,
    /// The language the UI is drawn in (a language code, never `auto`), set by the UI each frame;
    /// `None` without one (headless), where an explicit `interfaceLanguage` preference counts.
    /// Japanese gives new type the Japanese defaults ([`Session::japanese_interface`]).
    pub ui_language: Option<String>,
    /// Parameters the running top-level command resolved from the preferences or the clock, added
    /// to its journal entry so a replay does the same ([`Session::note_journal`]).
    journal_note: serde_json::Map<String, Value>,
    /// The depth of the command whose values [`Session::note_journal`] keeps: 1 (the top-level
    /// command), or a step's while a batch runs it ([`Session::execute_step`]).
    note_depth: u32,
    /// The running top-level query command changed the selection and is journaled like any other
    /// command ([`Session::journal_query`]).
    journal_query: bool,
    /// From `edit.undoGroup.begin` to `edit.undoGroup.end`: the name of the undo step (`None`: after
    /// its first edit). Each document opened or created meanwhile opens an undo group of its own
    /// ([`Session::add_document`]). `None` otherwise.
    pub(crate) explicit_undo_group: Option<Option<String>>,
    /// While `command.batch` runs: the documents its steps closed or replaced (Revert), which an
    /// error brings back; `None` otherwise.
    pub(crate) batch_stash: Option<Vec<DocState>>,
    /// After `command.batch` ran its steps, until the top-level command notes its entry in the undo
    /// groups: for each step, the documents ([`DocState::uid`]) whose undo groups it belongs to, and
    /// the index of the document it left active when it changed the active document.
    pub(crate) batch_steps: Option<Vec<(Vec<u64>, Option<usize>)>>,
    /// While [`Session::place_read`] runs `file.place`: the file read for it, which the command
    /// places instead of reading the file; `None` otherwise.
    pub(crate) place_hand_off: Option<cmd::place::PlaceRead>,
    /// While [`Session::place_queue_read`] runs `file.place.queue`: the files read for it, which
    /// the command loads the place cursor with instead of reading them; `None` otherwise.
    pub(crate) queue_hand_off: Option<cmd::place::QueueRead>,
    /// Whether a click or drag of the place cursor asks the frontend to place its file
    /// ([`UiRequest::Place`]) instead of running `file.place`: the desktop app sets it to read the
    /// file off the UI thread. False: the file is placed at once.
    pub place_cursor_requests: bool,
    /// While [`Session::prepare_package`] runs `file.package`: the package the command prepared,
    /// which it leaves here instead of writing it; `None` otherwise.
    pub(crate) package_hand_off: Option<Option<cmd::package::Package>>,
    /// While [`Session::prepare_screen_export`] runs `document.exportForScreens` or
    /// `assets.export`: the export the command prepared, which it leaves here instead of writing
    /// it; `None` otherwise.
    pub(crate) screens_hand_off: Option<Option<cmd::fileio::ScreenExport>>,
    /// Where Data Recovery keeps its copies ([`cmd::recovery`]).
    pub recovery: cmd::recovery::Recovery,
    /// While the active tool's actions run: their commands aren't "a command from outside the
    /// tool" ([`Session::after_command`]).
    pub(crate) in_tool_actions: bool,
    /// While [`Session::preview`] runs its command: its edits are the interaction's own.
    previewing: bool,
    /// Envelope Options with no envelope selected: the options and fidelity new envelopes get
    /// (`None`: the reference app's defaults, fidelity 50); not saved.
    pub(crate) envelope_defaults: Option<(vectorcraft_doc::live::EnvelopeOptions, f64)>,
    /// The Liquify stroke the last live preview applied, which the next sample of the drag goes on
    /// from ([`cmd::distortcmds::LiquifyStroke`]).
    pub(crate) liquify_stroke: Option<Box<cmd::distortcmds::LiquifyStroke>>,
    /// A press on the Plane Switching Widget is under way: its drag and release are the widget's.
    pub(crate) plane_widget_press: bool,
    /// The pointer button is held: the last pointer event was a press or a drag, not a release or
    /// a move ([`Session::pointer_held`]).
    pub(crate) pointer_held: bool,
    /// A guide being dragged out of a ruler ([`Session::ruler_guide`]).
    pub(crate) ruler_guide: Option<vectorcraft_tools::rulerguide::NewGuide>,
    /// Which selected type overflows its frame, for the ports every tool but the Type tools shows.
    pub(crate) port_marks: vectorcraft_tools::text::PortMarks,
    /// The last search for the files of missing fonts (`text.findFontFiles`), until another
    /// starts; dropping it stops it.
    pub(crate) font_search: Option<cmd::fontfiles::FontSearch>,
    /// The last search for the files of linked images and placed documents (`links.findFiles`),
    /// until another starts; dropping it stops it.
    pub(crate) link_search: Option<cmd::linkfiles::LinkSearch>,
    /// Where folder searches may go; `None`: this computer's rules ([`cmd::findfiles::Rules`]).
    /// Tests set it.
    pub search_rules: Option<cmd::findfiles::Rules>,
    /// Canvas feedback cached for the pending deferred preview ([`Session::overlays`]); cleared
    /// when the preview params change or the interaction ends.
    pub(crate) deferred_overlay_cache: Option<(String, Value, Vec<vectorcraft_tools::Overlay>)>,
    /// The walker threads a folder search starts; `None`: [`cmd::findfiles::threads`]. Tests set
    /// it.
    pub search_threads: Option<usize>,
    /// Each open document's linked files' size and modification time when last seen (linked
    /// files changing while the document is open, [`link_watch`]).
    pub link_stamps: std::collections::HashMap<(u64, String), link_watch::Stamp>,
    /// The stamps being taken on a worker thread ([`Session::start_link_scan`]).
    pub link_scan: Option<link_watch::LinkScan>,
    /// The Global Edit Options Start Global Edit matches with (`select.globalEditOptions`); not
    /// saved.
    pub global_edit_options: cmd::globaledit::Options,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// Held by the tests that add fonts to the shared [`vectorcraft_text::FontDb::global`] for as long
/// as they run: adding a font clears the database's caches, which would change how another such
/// test's type resolves between its layout and its export.
#[cfg(test)]
pub(crate) fn adding_fonts() -> std::sync::MutexGuard<'static, ()> {
    static FONTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    FONTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
use tests::{
    adjust as tests_adjust, clip as tests_clip, colormgmt as tests_colormgmt, expand as tests_expand, large_edits as tests_large_edits,
    links as tests_links, linkspanel as tests_linkspanel, menucmds as tests_menucmds, outlinestroke as tests_outlinestroke,
    persp_select as tests_persp_select, textimport as tests_textimport,
};

mod command_sets;
mod doc_state;
mod error;
mod history;
mod interaction;
mod prefs;
mod sanity;
mod session;

#[cfg(test)]
pub(crate) use command_sets::SELECTS_GIVEN_IDS;
#[cfg(test)]
use command_sets::given_ids;
pub use command_sets::{FIT_ACTIVE_ARTBOARD, ON_ACTIVE_ARTBOARD};
pub use doc_state::{DocState, OpenRows, RecalledSelection};
pub use error::{EngineError, Result};
pub use history::{History, HistoryEntry, NamedStep, default_history_budget};
pub(crate) use interaction::PreviewCopies;
pub use interaction::{Interaction, LastTransform, UndoGroup, UndoScope};
pub use prefs::{DrawMode, Prefs};
pub use sanity::TOO_DEEP;
pub(crate) use sanity::{deselect_hidden, is_guide_path};
#[cfg(test)]
mod tests;
