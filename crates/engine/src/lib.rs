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

pub use cmd::EyedropperOptions;
pub use cmd::clipboard::Clipboard;
pub use cmd::create::constrain_click;
pub use cmd::distortcmds::perspective_click;
pub use cmd::params::{check_keys, check_params};
pub use cmd::pathops::auto_simplify_level;
pub use cmd::rasterfx::{export_pdf, flatten_raster_effects};
pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use journal::Journal;
pub use tooling::{UiRequest, ViewInfo};
pub use vectorcraft_doc as doc;
pub use vectorcraft_doc::cancel;
pub use vectorcraft_pathops::{SIMPLIFY_LEVEL, simplify_level_tolerance};
pub use vectorcraft_tools as tools;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("no such object {0}")]
    NoNode(NodeId),
    #[error("{0}")]
    Other(String),
    /// The caller asked for the command to stop ([`cancel`]). It changed nothing.
    #[error("cancelled; nothing was changed")]
    Cancelled,
    /// A bug: the command panicked. The document was rolled back to its state before the command.
    #[error("internal error in `{cmd}`: {msg} (the document was left as it was before; please report this bug)")]
    Internal { cmd: String, msg: String },
}

impl From<cancel::Cancelled> for EngineError {
    fn from(_: cancel::Cancelled) -> Self {
        EngineError::Cancelled
    }
}

impl From<vectorcraft_doc::DocError> for EngineError {
    fn from(e: vectorcraft_doc::DocError) -> Self {
        EngineError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// One undo step.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub label: String,
    /// The document as it was. It keeps no lookup cache ([`Document::drop_lookup_cache`]): a
    /// table per step would add up, and undoing builds it again once.
    pub doc: Arc<Document>,
    /// The selection as it was, shared with the neighbouring steps that have the same one.
    pub selection: Arc<Selection>,
    /// Roughly the memory this step keeps of its own: what of `doc` the step after it (or the
    /// document now) doesn't share ([`Document::retained_bytes`]).
    pub bytes: usize,
    /// The same the other way round: what that next state keeps that `doc` doesn't share. Undoing
    /// (redoing) this step makes a step that keeps the next state: this is its `bytes`.
    next_bytes: usize,
    /// Which document the figures were taken against (its address): the next state.
    next: usize,
    /// The template and name `label` was made from ([`Session::edit_named`]).
    named: Option<NamedStep>,
}

/// A step name made by filling the `{name}` placeholder of a template with a name
/// ([`Session::edit_named`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedStep {
    /// The English template, with one `{name}` placeholder (`Apply {name}`).
    pub template: &'static str,
    /// The name in the placeholder, as the document holds it (a style's name).
    pub name: String,
    /// The name is one VectorCraft gives (`[Normal Character Style]`), not one the user typed.
    pub built_in: bool,
}

impl NamedStep {
    /// The step name: the template with the name in its placeholder.
    pub fn label(&self) -> String {
        self.template.replace("{name}", &self.name)
    }
}

impl HistoryEntry {
    /// A step keeping `doc` and `selection`, as they were before the edit that made `next`.
    pub(crate) fn kept(label: String, doc: Arc<Document>, selection: Arc<Selection>, next: &Document) -> Self {
        doc.drop_lookup_cache();
        let (bytes, next_bytes) = doc.retained_bytes_each(next);
        HistoryEntry { label, doc, selection, bytes, next_bytes, next: std::ptr::from_ref(next) as usize, named: None }
    }

    /// The template and name the step's label was made from, while the label is still the one
    /// they make: `None` for a step named any other way, and for one whose label was changed
    /// since it was recorded.
    pub fn named(&self) -> Option<&NamedStep> {
        self.named.as_ref().filter(|n| n.label() == self.label)
    }

    /// The step that undoing (redoing) the step `costs` came from makes: it keeps `doc`, the
    /// document before the undo, whose next state is now `now` (that step's document). Its figures
    /// are that step's the other way round when `doc` is still the document they were taken
    /// against (nothing replaced it since), so undo and redo needn't walk the documents again. A
    /// change made to it in place without an undo step (a setting, a refresh) goes unnoticed: a
    /// small error, for an estimate.
    pub(crate) fn turned(label: String, doc: Arc<Document>, selection: Arc<Selection>, now: &Arc<Document>, costs: StepCosts) -> Self {
        if costs.next != Arc::as_ptr(&doc) as usize {
            return Self::kept(label, doc, selection, now);
        }
        doc.drop_lookup_cache();
        HistoryEntry { label, doc, selection, bytes: costs.next_bytes, next_bytes: costs.bytes, next: Arc::as_ptr(now) as usize, named: None }
    }

    /// The figures [`HistoryEntry::turned`] reuses.
    pub(crate) fn costs(&self) -> StepCosts {
        StepCosts { bytes: self.bytes, next_bytes: self.next_bytes, next: self.next }
    }
}

/// A step's memory figures ([`HistoryEntry::costs`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepCosts {
    bytes: usize,
    next_bytes: usize,
    next: usize,
}

/// The undo history of a document: bounded by a number of steps (Preferences › Performance ›
/// History States) and by the memory the steps keep ([`History::byte_budget`]).
#[derive(Clone, Debug)]
pub struct History {
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
    /// Maximum undo depth (Illustrator has no hard limit; snapshots are cheap here).
    pub limit: usize,
    /// About the most memory the steps may keep ([`HistoryEntry::bytes`]): past it the oldest
    /// steps go, but never the last [`History::MIN_STEPS`] undo (and redo) steps.
    pub byte_budget: usize,
}

impl Default for History {
    fn default() -> Self {
        History { undo: vec![], redo: vec![], limit: 0, byte_budget: default_history_budget() }
    }
}

impl History {
    /// Undo (and redo) steps kept whatever memory they take.
    pub const MIN_STEPS: usize = 8;

    /// Roughly the memory the steps keep of their own ([`HistoryEntry::bytes`]).
    pub fn bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|e| e.bytes).sum()
    }

    /// Let the oldest steps go while over the step limit or the memory budget: undo steps first
    /// (the oldest), then redo steps (the furthest ahead).
    pub fn trim(&mut self) {
        let over = self.undo.len().saturating_sub(self.limit);
        if over > 0 {
            self.undo.drain(..over);
        }
        let mut total = self.bytes();
        if total <= self.byte_budget {
            return;
        }
        let mut n = 0;
        while total > self.byte_budget && self.undo.len() - n > Self::MIN_STEPS {
            total -= self.undo.get(n).map_or(0, |e| e.bytes);
            n += 1;
        }
        self.undo.drain(..n);
        let mut n = 0;
        while total > self.byte_budget && self.redo.len() - n > Self::MIN_STEPS {
            total -= self.redo.get(n).map_or(0, |e| e.bytes);
            n += 1;
        }
        self.redo.drain(..n);
    }

    /// Estimate the newest undo step again, its next state now being `now` (the steps after it were
    /// merged into it, or an undo group extended it).
    pub(crate) fn recost_last(&mut self, now: &Document) {
        if let Some(e) = self.undo.last_mut() {
            (e.bytes, e.next_bytes) = e.doc.retained_bytes_each(now);
            e.next = std::ptr::from_ref(now) as usize;
        }
    }
}

/// The default [`History::byte_budget`]: an eighth of the machine's memory where it is known
/// (Linux), at most 1 GiB (the figure elsewhere); 256 MiB on the web, whose memory is small.
pub fn default_history_budget() -> usize {
    const MAX: usize = 1 << 30;
    if cfg!(target_arch = "wasm32") {
        return 256 << 20;
    }
    static BUDGET: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| physical_memory().map_or(MAX, |m| (m / 8).clamp(64 << 20, MAX)))
}

/// The machine's memory, where cheap to learn without platform calls.
fn physical_memory() -> Option<usize> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: usize = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}

/// An in-progress drag: the snapshot the previews are applied to.
#[derive(Clone, Debug)]
pub struct Interaction {
    pub label: String,
    pub doc: Arc<Document>,
    /// The selection the previews start from (shared, so a preview frame copies nothing).
    pub selection: Arc<Selection>,
    pub preview: Option<(String, Value)>,
    /// Per-document state restored on cancel (current layer, highlighted Layers panel rows,
    /// isolation).
    pub active_layer: Option<NodeId>,
    pub layer_rows: Vec<NodeId>,
    pub variables_highlight: Option<String>,
    pub isolation: Option<NodeId>,
    /// The perspective transform the previews make (`perspective.transform` params): Transform
    /// Again repeats it once the drag is committed.
    pub perspective_again: Option<Value>,
    /// A transform or a perspective edit with `copy` keeps the copies it made of the snapshot here
    /// ([`PreviewCopies`]).
    pub(crate) copies: Option<Arc<PreviewCopies>>,
}

/// What Object → Transform → Transform Again (⌘D) repeats ([`SessionState::last_transform`]).
#[derive(Clone, Debug, PartialEq)]
pub enum LastTransform {
    /// The last transform command's matrix, and the options Transform Again passes with it.
    Matrix(Affine, Value),
    /// The last Transform Each's parameters: each object transforms about its own bounds, so the
    /// repeat runs the command on the selection of the moment rather than one matrix.
    Each(Value),
}

/// This holds the copies that a transform or a perspective edit with `copy` made of an
/// interaction's snapshot ([`cmd::object::apply_transform`]; `persp_edit` and `plane_move` in
/// `cmd::distortcmds`). A later transform or perspective edit with `copy` of the same objects,
/// while the document is still that snapshot, starts from `doc` and transforms only the copies.
#[derive(Debug)]
pub(crate) struct PreviewCopies {
    /// The copies were made from this snapshot.
    pub(crate) from: Arc<Document>,
    /// The transform or perspective edit copied these objects.
    pub(crate) roots: Vec<NodeId>,
    /// This is `from` with the copies added, untransformed.
    pub(crate) doc: Arc<Document>,
    /// This is the selection after copying, with the copies selected.
    pub(crate) selection: Selection,
    /// These are the copies' ids, in the order of `roots`.
    pub(crate) ids: Vec<NodeId>,
}

/// An open undo group ([`Session::begin_undo_group`]): the edits made in it are one undo step.
#[derive(Clone, Debug)]
pub struct UndoGroup {
    /// The document before the group's first edit, as the undo step that edit recorded keeps it.
    first: Option<Arc<Document>>,
    /// The numbers of the journal entries made in the group's document while it was open: a canceled
    /// group drops them. An entry that changed the active document has the index of the document it
    /// left active, and a canceled group turns it into that change.
    journal: Vec<(usize, Option<usize>)>,
    /// The undo step's name; else the first edit's.
    label: Option<String>,
}

/// One undo step begun with [`Session::begin_undo_scope`], which [`Session::end_undo_scope`] ends.
#[derive(Clone, Debug)]
pub struct UndoScope {
    /// The document ([`DocState::uid`]).
    uid: u64,
    /// The journal's length when the scope began.
    journal: usize,
    /// When the document's undo group was open already: the document, its selection and the document
    /// that the group's undo step keeps, as they were when the scope began; `None` when the scope
    /// opened the group.
    joined: Option<(Arc<Document>, Selection, Option<Arc<Document>>)>,
}

/// Per-document editing state.
#[derive(Clone, Debug)]
pub struct DocState {
    pub doc: Arc<Document>,
    pub selection: Selection,
    pub history: History,
    pub path: Option<String>,
    /// Increments on every change (also selection and view-only changes); UIs re-render when it
    /// moves. Not a dirty flag: see [`DocState::is_dirty`].
    pub revision: u64,
    /// The document as last saved (or opened). Structural sharing makes "unchanged" a pointer
    /// comparison, and undoing back to the saved state counts as clean.
    saved_doc: Arc<Document>,
    /// The documents that changes that are not edits to save replaced ([`DocState::same_content`]).
    unsaved_changes: SameContent,
    /// The layer new art goes into (the "current layer" in the Layers panel).
    pub active_layer: Option<NodeId>,
    /// The rows highlighted in the Layers panel (layers, sublayers, groups or objects, in the order
    /// they were clicked): what the panel's Duplicate, Delete, Merge, Options… and similar act on.
    /// Panel state, not art selection: not saved, not undoable (`layer.setCurrent`,
    /// `layer.highlight`).
    pub layer_rows: Vec<NodeId>,
    /// The variable highlighted in the Variables panel: what its Delete, Options… and Select
    /// Bound Object act on. Panel state, not art selection: not saved, not undoable
    /// (`variable.highlight`). A name, so renaming a variable moves the highlight with it.
    pub variables_highlight: Option<String>,
    /// Isolation mode container.
    pub isolation: Option<NodeId>,
    /// Select › Start Global Edit: the objects edited together until Stop Global Edit or Escape
    /// ([`cmd::globaledit`]). Editing state: not saved, not undoable.
    pub global_edit: Option<cmd::globaledit::GlobalEdit>,
    pub interaction: Option<Interaction>,
    /// Edits made while this is open are one undo step (a scrubbed numeric field).
    pub undo_group: Option<UndoGroup>,
    /// For Object → Transform → Transform Again (⌘D): what the last transformation was. A
    /// transform command keeps the `copy`, `typeAreas` and `patterns` it used, and `strokes` and
    /// `corners` when it scales; a drag keeps the params of its last preview with the values that
    /// preview noted ([`Session::note_journal`]).
    pub last_transform: Option<LastTransform>,
    /// What Select → Reselect repeats: the last selection command, its params and the objects
    /// selected when it ran (a Same command's reference objects).
    pub last_selection_cmd: Option<(String, Value, Vec<NodeId>)>,
    /// The saved selection last recalled (`select.recall`), which Select → Update Selection
    /// changes. It is not saved with the file, and a recall is not an undo step.
    pub recalled_selection: RecalledSelection,
    /// Process-unique id of this open document (tab indices shift when tabs close).
    pub uid: u64,
    /// View Opacity Mask (Alt-click the mask thumbnail): the masked object whose mask the canvas
    /// shows alone, in greyscale, while that mask is edited (see [`DocState::shown_mask`]).
    pub mask_view: Option<NodeId>,
    /// View → Show Transparency Grid, per document (saved view state, not undoable).
    pub transparency_grid: bool,
    /// The format Save writes ([`cmd::fileio::SAVE_FORMATS`]): the one the document was opened
    /// from or last saved as.
    pub format: &'static str,
    /// That format's options as last saved (SVG options for SVG, the Save PDF settings for PDF;
    /// empty for native files): Save reuses them and `file.formatOptions` reads them back.
    pub save_options: serde_json::Map<String, Value>,
    /// The ids of the document's artboards, in order, when `save_options` were stored: the
    /// artboards their `artboard`, `artboards` and `range` named by number then
    /// ([`cmd::fileio::remembered_save_options`]).
    pub save_artboards: Vec<u32>,
    /// Opened from an older native file (former name or format version): the title says
    /// "[Converted]" and Save asks for a new name instead of overwriting it.
    pub converted: bool,
    /// The view saved into native files (`Document::last_view`); the UI keeps it current before a
    /// save and restores it when the document opens.
    pub view: Option<vectorcraft_doc::SavedView>,
    /// Restored by Data Recovery: the title says "[Recovered]" and Save asks where to save it
    /// (suggesting `path`, the file it was copied from) instead of overwriting that file.
    pub recovered: bool,
    /// The document's Data Recovery copy, once one was written ([`cmd::recovery`]).
    pub recovery: Option<cmd::recovery::RecoveryCopy>,
    /// View → Show Print Tiling, per document (view state: not saved, not undoable).
    pub print_tiling: bool,
    /// The rows open in the Layers panel (view state: not undoable; native files keep it).
    pub layers_open: OpenRows,
    /// The file this document was read from, and what reading it left out (hidden text, art or
    /// layers it could not read): writing the document over that file would lose those for good,
    /// so an export or save to it asks first ([`cmd::fileio::check_not_lossy_overwrite`]). Not saved.
    pub imported_from: Option<String>,
    pub import_losses: Vec<String>,
    /// Transform Again after a perspective move or scale (Perspective Selection tool): the
    /// `perspective.transform` params it repeats. `None` once an ordinary transform follows.
    pub last_perspective: Option<Value>,
    /// The folders a confined (automation) caller that opened or made the document may read: the
    /// paths the document links to are looked at within them later too, when the Links panel
    /// asks about a link outside any caller's scope.
    pub opened_by: Option<Arc<file_access::AutomationRoots>>,
}

static NEXT_DOC_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The rows open in the Layers panel: the layers, sublayers and groups that show what they hold.
/// A document opens with the ones it was saved with ([`Document::layers_open`]), else with only
/// its top-level layers open (a `.ai` another app wrote opens with none open).
#[derive(Clone, Debug, Default)]
pub struct OpenRows {
    ids: std::collections::HashSet<NodeId>,
    /// Counts the changes, so views can keep what they work out from the open rows.
    generation: u64,
}

impl OpenRows {
    /// The rows `saved` in a file, else `doc`'s top-level layers.
    fn new(doc: &Document, saved: Option<Vec<NodeId>>) -> Self {
        let ids = match saved {
            Some(ids) => ids.into_iter().collect(),
            None => doc.layers.iter().map(|l| l.id).collect(),
        };
        Self { ids, generation: 0 }
    }
    pub fn contains(&self, id: NodeId) -> bool {
        self.ids.contains(&id)
    }
    /// Open or close row `id`.
    pub fn set(&mut self, id: NodeId, open: bool) {
        let changed = if open { self.ids.insert(id) } else { self.ids.remove(&id) };
        self.generation += u64::from(changed);
    }
    /// Changes so far: the same number means the same open rows.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// What a native file keeps of the open rows: those of `doc` that hold others, in id order;
    /// `None` when they are the default (only the top-level layers).
    pub fn saved(&self, doc: &Document) -> Option<Vec<NodeId>> {
        let mut ids = vec![];
        doc.walk(|n| {
            if n.children().is_some() && self.ids.contains(&n.id) {
                ids.push(n.id);
            }
        });
        ids.sort_unstable();
        let mut layers: Vec<NodeId> = doc.layers.iter().map(|l| l.id).collect();
        layers.sort_unstable();
        (ids != layers).then_some(ids)
    }
}

/// The saved selection last recalled, which Select → Update Selection changes. The Select commands
/// give each saved selection an id for the session and keep the ids for every version of the
/// document's saved selections they make or read, so the recalled one is found again after Edit
/// Selection…, Undo and Redo. A version that nothing holds any more (neither the document nor its
/// undo history) is dropped the next time a version is added.
#[derive(Clone, Debug, Default)]
pub struct RecalledSelection {
    /// The recalled saved selection's id.
    id: Option<u64>,
    /// Each version of the saved selections (identified by its allocation), with the id of each saved
    /// selection in it.
    versions: Vec<(std::sync::Weak<Vec<vectorcraft_doc::SavedSelection>>, Vec<u64>)>,
    /// The last id given.
    last: u64,
}

impl RecalledSelection {
    /// The ids of `list`'s saved selections; a version not met before gets new ones.
    pub(crate) fn ids(&mut self, list: &vectorcraft_doc::Shared<Vec<vectorcraft_doc::SavedSelection>>) -> Vec<u64> {
        let version = vectorcraft_doc::Shared::downgrade(list);
        if let Some((_, ids)) = self.versions.iter().find(|(v, _)| v.ptr_eq(&version)) {
            return ids.clone();
        }
        let ids: Vec<u64> = (0..list.len()).map(|_| self.new_id()).collect();
        self.remember(list, ids.clone());
        ids
    }
    /// An id no saved selection has had.
    pub(crate) fn new_id(&mut self) -> u64 {
        self.last += 1;
        self.last
    }
    /// Keep `ids` as those of `list`'s saved selections, dropping versions nothing holds any more.
    pub(crate) fn remember(&mut self, list: &vectorcraft_doc::Shared<Vec<vectorcraft_doc::SavedSelection>>, ids: Vec<u64>) {
        let version = vectorcraft_doc::Shared::downgrade(list);
        self.versions.retain(|(v, _)| v.strong_count() > 0 && !v.ptr_eq(&version));
        self.versions.push((version, ids));
    }
    /// Recall the saved selection at `index` in `list`.
    pub(crate) fn recall(&mut self, list: &vectorcraft_doc::Shared<Vec<vectorcraft_doc::SavedSelection>>, index: usize) {
        self.id = self.ids(list).get(index).copied();
    }
    /// Where the recalled saved selection is in `list`, if `list` holds it.
    pub fn index_in(&self, list: &vectorcraft_doc::Shared<Vec<vectorcraft_doc::SavedSelection>>) -> Option<usize> {
        let id = self.id?;
        let version = vectorcraft_doc::Shared::downgrade(list);
        let (_, ids) = self.versions.iter().find(|(v, _)| v.ptr_eq(&version))?;
        ids.iter().position(|&x| x == id).filter(|&i| i < list.len())
    }
}

/// The documents that changes that are not edits to save (Puppet Warp pins dropped, Edit Text ended,
/// type laid out again after a font scan) replaced, each counted as having the content of the
/// document it was replaced by ([`DocState::change_unsaved`]). A document that nothing holds any
/// more is dropped the next time one is added.
#[derive(Clone, Debug, Default)]
struct SameContent {
    /// The document the last such change made.
    now: std::sync::Weak<Document>,
    /// The documents with its content that such changes replaced, oldest first.
    before: Vec<std::sync::Weak<Document>>,
}

impl SameContent {
    /// Note that `now` replaced `before` without a change to its content.
    fn replaced(&mut self, before: std::sync::Weak<Document>, now: &Arc<Document>) {
        if !self.now.ptr_eq(&before) {
            self.before.clear();
        }
        self.before.retain(|d| d.strong_count() > 0);
        if before.strong_count() > 0 {
            self.before.push(before);
        }
        self.now = Arc::downgrade(now);
    }
    /// Whether `doc` is one of the documents with the content of `now`.
    fn holds(&self, now: &Arc<Document>, doc: &Arc<Document>) -> bool {
        std::ptr::eq(self.now.as_ptr(), Arc::as_ptr(now)) && self.before.iter().any(|d| std::ptr::eq(d.as_ptr(), Arc::as_ptr(doc)))
    }
}

impl DocState {
    pub fn new(mut doc: Document, path: Option<String>) -> Self {
        let active_layer = doc.default_layer();
        // Saved view state lives here while the document is open (saves write it back).
        let view = doc.last_view.take();
        let transparency_grid = std::mem::take(&mut doc.transparency_grid);
        let saved_open = doc.layers_open.take();
        let layers_open = OpenRows::new(&doc, saved_open);
        let doc = Arc::new(doc);
        Self {
            saved_doc: doc.clone(),
            unsaved_changes: SameContent::default(),
            doc,
            selection: Selection::default(),
            history: History { limit: 500, ..Default::default() },
            path,
            revision: 1,
            active_layer,
            layer_rows: vec![],
            variables_highlight: None,
            isolation: None,
            global_edit: None,
            interaction: None,
            undo_group: None,
            last_transform: None,
            last_selection_cmd: None,
            recalled_selection: RecalledSelection::default(),
            uid: NEXT_DOC_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            mask_view: None,
            transparency_grid,
            format: "vectorcraft",
            save_options: Default::default(),
            save_artboards: vec![],
            converted: false,
            view,
            recovered: false,
            recovery: None,
            print_tiling: false,
            layers_open,
            imported_from: None,
            import_losses: vec![],
            last_perspective: None,
            opened_by: file_access::current(),
        }
    }
    /// Unsaved changes: the document differs from the saved one (selection changes and changes that
    /// are not edits to save don't count, [`DocState::same_content`]).
    pub fn is_dirty(&self) -> bool {
        !self.same_content(&self.saved_doc)
    }
    /// Record the current document as saved.
    pub fn mark_saved(&mut self) {
        self.saved_doc = self.doc.clone();
    }
    /// Record `snapshot` (the document as a background save took it) as saved: edits made since
    /// keep the document modified.
    pub fn mark_saved_as(&mut self, snapshot: &Arc<Document>) {
        self.saved_doc = snapshot.clone();
    }
    /// Whether `doc` counts as having the content of the document now: it is the document, or one
    /// that changes that are not edits to save replaced since ([`DocState::change_unsaved`]).
    pub fn same_content(&self, doc: &Arc<Document>) -> bool {
        Arc::ptr_eq(&self.doc, doc) || self.unsaved_changes.holds(&self.doc, doc)
    }
    /// Make a change that is not an edit to save with `f`, without an undo step: editing state that
    /// is never saved, or type laid out again after a font scan (which can also refit the frame of
    /// Auto Size area type). A saved document stays saved, and a save or recovery copy taken of the
    /// document before counts as having its content ([`DocState::same_content`]).
    pub(crate) fn change_unsaved(&mut self, f: impl FnOnce(&mut Arc<Document>)) {
        let saved = !self.is_dirty();
        let before = Arc::downgrade(&self.doc);
        f(&mut self.doc);
        if !std::ptr::eq(before.as_ptr(), Arc::as_ptr(&self.doc)) {
            self.unsaved_changes.replaced(before, &self.doc);
        }
        if saved {
            self.mark_saved();
        }
    }
    /// Keep what interaction `it` (taken from this document) changed, as one undo step.
    pub(crate) fn keep_interaction(&mut self, it: Interaction) {
        if !Arc::ptr_eq(&it.doc, &self.doc) {
            let selection = match self.history.undo.last() {
                Some(l) if l.selection == it.selection => l.selection.clone(),
                _ => it.selection,
            };
            self.push_undo(it.label, it.doc, selection);
            self.revision += 1;
        }
    }
    /// Record an undo step: `doc` and `selection` as they were before an edit (the document now is
    /// the one after it). In an undo group only the group's first edit records one: the edits
    /// after it extend that step.
    fn push_undo(&mut self, mut label: String, doc: Arc<Document>, selection: Arc<Selection>) {
        if let Some(g) = &mut self.undo_group {
            let recorded = |first: &Arc<Document>| self.history.undo.last().is_some_and(|l| Arc::ptr_eq(&l.doc, first));
            if g.first.as_ref().is_some_and(recorded) {
                self.history.recost_last(&self.doc);
                self.history.trim();
                return;
            }
            g.first = Some(doc.clone());
            if let Some(l) = &g.label {
                label = l.clone();
            }
        }
        let e = HistoryEntry::kept(label, doc, selection, &self.doc);
        self.history.undo.push(e);
        self.history.redo.clear();
        self.history.trim();
    }
    /// End the interaction in progress, undoing what it changed.
    pub(crate) fn undo_interaction(&mut self) {
        if let Some(it) = self.interaction.take() {
            self.doc = it.doc;
            self.selection = Arc::unwrap_or_clone(it.selection);
            self.sync_new_art();
            cmd::globaledit::follow_history(self);
            self.active_layer = it.active_layer;
            self.layer_rows = it.layer_rows;
            self.variables_highlight = it.variables_highlight;
            self.isolation = it.isolation;
            self.revision += 1;
        }
    }
    /// Count the document as modified, as if never saved (a document restored by Data Recovery).
    pub fn mark_unsaved(&mut self) {
        self.saved_doc = Arc::new(Document::new(1.0, 1.0));
    }
    /// Deselect the art on a locked artboard, except new art ([`Document::new_art`]). New art
    /// becomes the objects of layers on a locked artboard that hold selected objects (a group for
    /// its selected members) and that an edit made (those not in `before`, the document before
    /// it), or that held objects selected in `was` that were new art (`old`, the new art of that
    /// selection), or were inside new art in `before` (the members of an ungrouped group). New art
    /// that a selection leaves out, or that leaves the locked artboards, is protected from then
    /// on. The selection and the document get the new art; the document's changes in place, so a
    /// selection change doesn't replace the document.
    fn protect_locked_artboards(&mut self, was: &[NodeId], old: &std::collections::HashSet<NodeId>, before: Option<&Document>) {
        let new_art: std::collections::HashSet<NodeId> = {
            let (doc, locks) = (&self.doc, self.doc.locked_artboards());
            if locks.is_empty() {
                Default::default()
            } else {
                // `was` and `old` name the objects of the document they were selected in.
                let then = before.unwrap_or(doc);
                let was: std::collections::HashSet<NodeId> =
                    was.iter().filter_map(|o| then.layer_object(*o)).map(|n| n.id).filter(|o| old.contains(o)).collect();
                let selected: Vec<(NodeId, &vectorcraft_doc::Node)> =
                    self.selection.objects.iter().filter_map(|o| doc.layer_object(*o).map(|n| (*o, n))).collect();
                // One walk of the document before the edit finds where the selected objects were;
                // the objects the edit made aren't in it.
                let paths = before.filter(|_| !selected.is_empty()).map(|b| b.index_paths(selected.iter().flat_map(|(o, n)| [*o, n.id])));
                let new = |o: NodeId, object: NodeId| {
                    was.contains(&object)
                        || before.zip(paths.as_ref()).is_some_and(|(b, p)| {
                            !p.contains_key(&object) || p.get(&o).and_then(|p| b.layer_object_at(p)).is_some_and(|n| was.contains(&n.id))
                        })
                };
                selected.into_iter().filter(|(o, n)| new(*o, n.id) && locks.overlaps(n)).map(|(_, n)| n.id).collect()
            }
        };
        if new_art != *self.selection.new_art {
            self.selection.new_art = Arc::new(new_art);
        }
        self.sync_new_art();
        self.selection.deselect_on_locked_artboards(&self.doc);
    }
    /// Give the document the selection's new art ([`Selection::new_art`]), less the objects the
    /// selection no longer holds: after Undo, Redo or a failed batch brings back a document and
    /// a selection, and after a command that changed the selection directly.
    pub(crate) fn sync_new_art(&mut self) {
        if !self.selection.new_art.is_empty() {
            let held: std::collections::HashSet<NodeId> =
                self.selection.objects.iter().filter_map(|o| self.doc.layer_object(*o)).map(|n| n.id).collect();
            if !self.selection.new_art.is_subset(&held) {
                self.selection.new_art = Arc::new(self.selection.new_art.intersection(&held).copied().collect());
            }
        }
        let doc = self.doc.new_art.get();
        if !Arc::ptr_eq(&doc, &self.selection.new_art) && *doc != *self.selection.new_art {
            self.doc.new_art.set(self.selection.new_art.clone());
        }
    }
    pub fn title(&self) -> String {
        let name = self
            .path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.doc.title.clone());
        if self.converted {
            format!("{} [Converted]", cmd::fileio::file_stem(&name))
        } else if self.recovered {
            format!("{} [Recovered]", cmd::fileio::file_stem(&name))
        } else {
            name
        }
    }
    /// Where new art is inserted: the isolation container, else the active layer.
    /// The current layer, if the remembered id still names a layer (ids are reused after undo).
    pub fn current_layer(&self) -> Option<NodeId> {
        self.active_layer.filter(|l| self.doc.node(*l).is_some_and(|n| n.is_layer()))
    }
    pub fn insertion_parent(&self) -> Option<NodeId> {
        // Ids are reused after undo (the id counter is part of the document), so a remembered
        // layer or isolated group must still be one: a pasted group must never land in a path.
        if let Some(i) = self.isolation
            && self.doc.node(i).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }))
        {
            return Some(i);
        }
        // A single object isolated: new art goes beside it, in its group or layer.
        if let Some(p) = self.isolation.filter(|i| self.doc.node(*i).is_some_and(|n| !n.is_container())).and_then(|i| self.doc.parent_of(i))
            && self.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }))
        {
            return Some(p);
        }
        // A sublayer takes new art only while it and the layers around it are shown and unlocked.
        self.active_layer.filter(|l| self.doc.node(*l).is_some_and(|n| n.is_layer()) && self.doc.is_editable(*l)).or_else(|| self.doc.default_layer())
    }
    /// [`Self::insertion_parent`] for new art, which a locked or hidden layer never takes: an error
    /// when no layer is shown and unlocked, so the art is refused rather than hidden or locked away.
    pub fn target_parent(&self) -> Result<Option<NodeId>> {
        match self.insertion_parent() {
            Some(p) if !self.doc.is_editable(p) => Err(EngineError::Other("the target layer is locked or hidden".into())),
            parent => Ok(parent),
        }
    }
    /// The highlighted Layers panel rows that still exist (ids are reused after undo, so a
    /// remembered row must still be in the document).
    pub fn highlighted_rows(&self) -> Vec<NodeId> {
        self.layer_rows.iter().copied().filter(|id| self.doc.node(*id).is_some()).collect()
    }
    /// The object whose opacity mask View Opacity Mask shows ([`DocState::mask_view`]): only while
    /// its mask is being edited, so leaving editing by any route (undo, deleting the object) ends it.
    pub fn shown_mask(&self) -> Option<NodeId> {
        self.mask_view.filter(|id| self.doc.mask_edit.is_some_and(|m| m.object == *id))
    }
}

/// Coordinates beyond this (points) are rejected: ~1,400 m, far past Illustrator's large canvas.
pub use vectorcraft_doc::MAX_COORD;

/// The commands that act on the active artboard (#693), which the app passes as their `artboard`:
/// Paste in Place, in Front and in Back paste onto it, Align to Artboard aligns to it, Distribute
/// Objects and Distribute Spacing with Align to Artboard distribute objects across it, and All on
/// Active Artboard selects on it. A `command.batch` given `artboard` passes it on to each of their
/// steps that names none.
pub const ON_ACTIVE_ARTBOARD: [&str; 7] = [
    "edit.pasteInPlace",
    "edit.pasteInFront",
    "edit.pasteInBack",
    "object.align",
    "object.distribute",
    "object.distributeSpacing",
    "select.allOnArtboard",
];

/// The commands that resize the artboards the app passes (Object › Artboards): the app passes the
/// selected artboards to Fit to Artwork Bounds as its `indices`, and the active artboard to Fit to
/// Selected Art as its `index`. A `command.batch` given `artboard` passes it on, as `index`, to
/// each of their steps that names none.
pub const FIT_ACTIVE_ARTBOARD: [&str; 2] = ["artboard.fitToArt", "artboard.fitToSelection"];

/// `(command, param)` pairs the journal records only so a replay reproduces the original run
/// (dates from the clock: `cmd::clock_date`; the artboards the app passes to the
/// [`ON_ACTIVE_ARTBOARD`] and [`FIT_ACTIVE_ARTBOARD`] commands and to a batch); an action leaves
/// them out ([`Session::journal_for_action`]).
const REPLAY_ONLY: [(&str, &str); 14] = [
    ("file.new", "created"),
    ("document.save", "modified"),
    ("file.saveAs", "modified"),
    ("edit.pasteInPlace", "artboard"),
    ("edit.pasteInFront", "artboard"),
    ("edit.pasteInBack", "artboard"),
    ("object.align", "artboard"),
    ("object.distribute", "artboard"),
    ("object.distributeSpacing", "artboard"),
    ("artboard.fitToArt", "index"),
    ("artboard.fitToArt", "indices"),
    ("artboard.fitToSelection", "index"),
    ("select.allOnArtboard", "artboard"),
    ("command.batch", "artboard"),
];

/// `p`, the params of command `id`, without its [`REPLAY_ONLY`] values (a batch: its steps').
fn strip_replay_only(id: &str, p: &mut Value) {
    let strip = |id: &str, p: &mut Value| {
        if let Value::Object(m) = p {
            m.retain(|k, _| !REPLAY_ONLY.iter().any(|&(c, key)| c == id && key == k));
        }
    };
    strip(id, p);
    // Batches don't nest: one level of steps.
    if id == "command.batch"
        && let Some(Value::Array(steps)) = p.get_mut("commands")
    {
        for step in steps {
            let id = step.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(p) = step.get_mut("params") {
                strip(&id, p);
            }
        }
    }
}

/// Is `id` a path made into a guide (View → Guides → Make Guides)?
pub(crate) fn is_guide_path(d: &Document, id: NodeId) -> bool {
    d.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Path { guide: true, .. }))
}

/// Deselect the ruler guides and the paths made into guides: while View → Guides → Lock Guides is
/// on, they can't be selected (#1534), whatever selects them. → whether any was selected.
pub(crate) fn deselect_guides(d: &Document, sel: &mut Selection) -> bool {
    let paths: Vec<NodeId> = sel.objects.iter().copied().filter(|id| is_guide_path(d, *id)).collect();
    let any = !paths.is_empty() || !sel.guides.is_empty();
    for id in paths {
        sel.remove(id);
    }
    sel.guides.clear();
    sel.key_guide = None;
    // A key object kept only for the guides to line up on goes with them.
    if sel.key.is_some_and(|k| sel.objects.iter().all(|o| *o == k)) {
        sel.key = None;
    }
    any
}

/// Deselect what the View menu keeps from being selected: the guides while Lock Guides is on
/// ([`deselect_guides`]), and the slices that the Slice Selection tool or a slice command selected
/// ([`Selection::slices`]) while Lock Slices or Hide Slices is on. A selected object that is an
/// object slice stays selected. → whether any was selected.
pub(crate) fn deselect_hidden(menu: &cmd::menucmds::MenuState, d: &Document, sel: &mut Selection) -> bool {
    let guides = menu.guides_locked && deselect_guides(d, sel);
    let slices = (menu.slices_locked || menu.slices_hidden) && !sel.slices.is_empty();
    if slices {
        sel.slices.clear();
    }
    guides || slices
}

/// The error of an edit that would nest objects deeper than [`vectorcraft_doc::MAX_DEPTH`].
pub const TOO_DEEP: &str = "objects can't be nested more than 32 deep";
const _: () = assert!(vectorcraft_doc::MAX_DEPTH == 32, "TOO_DEEP (and its catalog rows) names the cap");

/// Cheap sanity check after an edit: artboards and the objects just touched (the selection) must
/// have finite, in-range geometry, so saved files always reload and renderers never see NaN/∞.
fn doc_sane(d: &Document, sel: &Selection) -> bool {
    d.ruler_origin.is_finite()
        && d.ruler_origin.x.abs() <= MAX_COORD
        && d.ruler_origin.y.abs() <= MAX_COORD
        && d.artboards.iter().all(|a| rect_in_range(a.rect))
        && d.guides.iter().all(|g| g.pos.is_finite() && g.pos.abs() <= MAX_COORD)
        && objects_in_range(d, &sel.objects)
}

fn rect_in_range(r: vectorcraft_geom::Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

/// Do the objects `ids` have finite geometry within [`MAX_COORD`] (the ones not there pass)?
fn objects_in_range(d: &Document, ids: &[NodeId]) -> bool {
    let paths = d.index_paths(ids.iter().copied());
    ids.iter().all(|id| paths.get(id).and_then(|p| d.node_at(p)).and_then(|n| n.geometric_bounds()).is_none_or(rect_in_range))
}

/// Where new art goes (Illustrator's drawing modes, Shift+D cycles).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawMode {
    #[default]
    Normal,
    Behind,
    Inside,
}

/// Application preferences (Edit → Preferences). Every field is reachable through `prefs.get` /
/// `prefs.set {key, value}` (camelCase keys, validated against [`cmd::prefscmds::PREF_SPECS`]).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    // General
    /// Arrow-key nudge distance (General → Keyboard Increment), points.
    pub keyboard_increment: f64,
    pub constrain_angle: f64,
    pub corner_radius: f64,
    pub disable_auto_add_delete: bool,
    pub use_precise_cursors: bool,
    pub show_tool_tips: bool,
    pub anti_aliased_artwork: bool,
    pub select_same_tint_percent: bool,
    pub show_home_screen: bool,
    pub use_preview_bounds: bool,
    pub display_print_size: bool,
    pub double_click_to_isolate: bool,
    pub transform_pattern_tiles: bool,
    pub scale_corners: bool,
    /// Scale Strokes & Effects.
    pub scale_strokes: bool,
    pub zoom_with_mouse_wheel: bool,
    /// A horizontal drag on a numeric field or its label steps its value (#400).
    pub scrub_numeric_fields: bool,
    /// A two-finger rotation on a Mac trackpad turns the view about the pointer once it reaches 5°.
    pub trackpad_rotate_view: bool,
    /// Offset for Paste / duplicate (Illustrator pastes to the view centre; we offset by this).
    pub paste_offset: f64,
    // Selection & Anchor Display
    pub selection_tolerance: f64,
    pub object_selection_by_path_only: bool,
    pub snap_to_point_tolerance: f64,
    pub ctrl_click_selects_behind: bool,
    pub zoom_to_selection: bool,
    pub move_locked_with_artboard: bool,
    pub anchor_size: u32,
    pub handle_style: String,
    pub highlight_anchors_on_hover: bool,
    pub show_handles_multiple_anchors: bool,
    pub hide_corner_widget_above: f64,
    pub pen_rubber_band: bool,
    pub curvature_rubber_band: bool,
    // Type
    pub type_size_increment: f64,
    pub tracking_increment: f64,
    pub baseline_shift_increment: f64,
    pub show_east_asian_options: bool,
    pub show_indic_options: bool,
    pub type_selection_by_path_only: bool,
    pub font_names_in_english: bool,
    pub auto_size_area_type: bool,
    pub font_preview: bool,
    pub font_preview_size: String,
    pub recent_fonts_count: u32,
    pub missing_glyph_protection: bool,
    pub highlight_alternate_glyphs: bool,
    pub placeholder_text: bool,
    // Units
    pub units_general: String,
    pub units_stroke: String,
    pub units_type: String,
    pub units_asian_type: String,
    /// Numbers Without Units Are Points: a number typed with no unit into a length field in picas
    /// is read in points (on by default; the reference app dims it unless a unit is Picas).
    pub numbers_without_units_are_points: bool,
    pub identify_objects_by: String,
    // Guides & Grid
    pub guide_color: String,
    pub guide_style: String,
    pub grid_color: String,
    pub grid_style: String,
    pub gridline_every: f64,
    pub grid_subdivisions: u32,
    pub grids_in_back: bool,
    pub show_pixel_grid: bool,
    // Smart Guides
    pub smart_guide_color: String,
    pub alignment_guides: bool,
    pub object_highlighting: bool,
    pub transform_tools_guides: bool,
    pub construction_guides: bool,
    pub construction_angles: String,
    pub anchor_path_labels: bool,
    pub measurement_labels: bool,
    pub spacing_guides: bool,
    pub distance_guides: bool,
    pub snapping_tolerance: f64,
    pub snap_tangent: bool,
    pub snap_perpendicular: bool,
    // Slices
    pub show_slice_numbers: bool,
    pub slice_line_color: String,
    // Hyphenation
    pub hyphenation_language: String,
    pub hyphenation_exceptions: String,
    /// Type › Options › Additional Fonts Folder: a folder (read with its subfolders) whose fonts
    /// are listed and used as if installed (#683); empty for none.
    pub fonts_folder: String,
    // Plug-ins & Scratch Disks
    pub plugins_folder: String,
    pub scratch_primary: String,
    pub scratch_secondary: String,
    // User Interface
    pub ui_brightness: String,
    /// User Interface › Accent Color: the colour of what is on, chosen or focused.
    pub ui_accent: String,
    /// User Interface › Color Scheme: "modern" (with the accent above) or "classic" (neutral
    /// greys and a blue accent).
    pub ui_color_scheme: String,
    pub canvas_color: String,
    pub auto_collapse_icon_panels: bool,
    /// User Interface › Show Tool Group Labels: the toolbar's group names (Select, Shapes, Draw…);
    /// off (as it ships, #1736), the tools run on from group to group with nothing between them.
    pub tool_group_labels: bool,
    pub open_documents_as_tabs: bool,
    pub large_tabs: bool,
    pub ui_scaling: f64,
    pub scale_cursor_with_ui: bool,
    /// UI language: `auto` (follow the system locale) or a language code such as `en`, `zh-hant`.
    /// The list of languages belongs to the shell (`ui-egui` i18n); an unknown code reads as `auto`.
    pub interface_language: String,
    /// Windows and Linux: use the system's title bar and window buttons instead of the app bar
    /// acting as the title bar (tiling window managers, desktops that draw their own decorations).
    /// Read when the app starts. macOS always uses the system's.
    pub system_title_bar: bool,
    // Performance
    pub gpu_performance: bool,
    pub animated_zoom: bool,
    /// Which graphics processor the desktop app asks for at startup (it takes effect after a
    /// restart): `automatic`, `lowPower` (the integrated GPU on hybrid-graphics machines) or
    /// `highPerformance` (the discrete one). The canvas is rasterized on the CPU and only
    /// composited on the GPU, so the integrated GPU is plenty; presenting from the discrete GPU
    /// through the integrated one made some hybrid laptops flicker (#306). Automatic is power
    /// saving on Windows and macOS and the system's default GPU elsewhere, the one the desktop
    /// runs on: a Wayland compositor may not show frames from another GPU (#502). Single-GPU
    /// machines are unaffected.
    pub gpu_preference: String,
    pub history_states: u32,
    pub real_time_drawing: bool,
    /// Rasterizer worker threads; -1 = automatic.
    pub render_threads: i32,
    /// Linux: `automatic` opens the window on the session's display server; `x11` opens it through
    /// XWayland on a Wayland session, where dropped files arrive (#1151); `x11WithPen` opens it
    /// through XWayland only when a pen is attached (#2028). Read when the app starts.
    pub linux_display_server: String,
    // File Handling
    pub background_save: bool,
    pub background_export: bool,
    pub autosave_recovery: bool,
    pub autosave_interval: u32,
    pub recovery_folder: String,
    pub recovery_off_for_complex: bool,
    pub recent_files_count: u32,
    pub low_res_proxy_eps: bool,
    pub anti_aliased_bitmaps: bool,
    pub update_links: String,
    // Clipboard Handling
    pub copy_as_svg: bool,
    pub copy_as_pdf: bool,
    pub copy_aicb: bool,
    pub aicb_mode: String,
    pub paste_text_formatting: String,
    // Appearance of Black
    pub black_on_screen: String,
    pub black_output: String,
    // Devices
    pub touch_workspace: bool,
    pub touch_gestures: bool,
    // Graphic Styles panel
    /// Override Character Color: a graphic style applied to type replaces its characters' fill
    /// and stroke with the style's fills and strokes.
    pub override_char_color: bool,
    // Pattern editing
    /// Object → Pattern → Tile Edge Color (`#rrggbb`): the tile edge and swatch bounds in pattern
    /// editing mode.
    pub pattern_tile_edge_color: String,
    /// Eyedropper Options (`eyedropper.setOptions`; a preference group, see
    /// [`cmd::prefscmds::PREF_GROUPS`]).
    pub eyedropper: EyedropperOptions,
    /// Edit → Transparency Flattener Presets: the user's presets (the built-in ones aren't stored).
    /// Not a `prefs.set` key or group (resetting the preferences keeps them): `flattener.presets.*`
    /// edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flattener_presets: Vec<cmd::FlattenerPreset>,
    /// Width profiles saved to the Stroke panel's Profile list (`stroke.widthProfile.*`). Not a
    /// Preferences dialog field, so it has no [`cmd::prefscmds::PREF_SPECS`] row.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub width_profiles: Vec<vectorcraft_doc::SavedProfile>,
    /// General → Use Japanese Crop Marks: the style of Create Trim Marks and Effect → Crop Marks.
    pub japanese_crop_marks: bool,
    /// Appearance panel → New Art Has Basic Appearance (on): new art takes one fill and stroke;
    /// off, the whole appearance of the last selection (`appearance.setNewArtBasic`).
    pub new_art_basic: bool,
    /// The Color Themes panel's saved themes (`colorTheme.*`): a local library, not a Preferences
    /// dialog field, so it has no [`cmd::prefscmds::PREF_SPECS`] row and resetting keeps it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub color_themes: Vec<cmd::colortheme::ColorTheme>,
    /// New Document → Saved: the user's document presets (`file.newPresets.save`). A local
    /// library, not a Preferences dialog field: resetting the preferences keeps it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_doc_presets: Vec<cmd::newdoc::DocSettings>,
    /// New Document → Recent: the settings of the last documents made (newest first).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_new_docs: Vec<cmd::newdoc::DocSettings>,
    /// Edit → PDF Presets: the user's presets (the built-in ones aren't stored). A local library,
    /// not a Preferences dialog field: resetting the preferences keeps it; `pdf.preset.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pdf_presets: Vec<vectorcraft_pdf::PdfPreset>,
    /// File → Export for Screens: the user's batch export presets (`document.exportPresets.*`;
    /// the built-in ones aren't stored). A local library, not a Preferences dialog field.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub screen_export_presets: Vec<cmd::fileio::ScreenExportPreset>,
    // File Handling (continued)
    /// Where Save as Template and New from Template start ("" = `Documents/VectorCraft Templates`).
    pub templates_folder: String,
    /// Open older native files as "<name> [Converted]" so Save asks for a new name.
    pub append_converted: bool,
    /// File Handling → Use Compression: native saves are gzip-compressed (`document.save
    /// {compress}` overrides it).
    pub use_compression: bool,
    /// Save for Web: the user's presets (the built-in ones aren't stored). A local library, not a
    /// Preferences dialog field: resetting the preferences keeps it; `webExport.presets.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub web_export_presets: Vec<cmd::webexport::WebPreset>,
    /// Save for Web: the settings the dialog opens on (`webExport.settings`); none until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_export_settings: Option<cmd::webexport::WebSettings>,
    /// Edit → Print Presets: the user's presets ([Default] isn't stored). A local library, not a
    /// Preferences dialog field: resetting the preferences keeps it; `print.presets.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub print_presets: Vec<cmd::printpresets::PrintPreset>,
    /// Constrain Width and Height Proportions: the link between W and H in the Transform panel,
    /// the Properties panel and the Control bar (their size fields pass `proportional` to
    /// `object.setBounds`).
    pub constrain_proportions: bool,
    /// One Corner Radius field sets every corner of a live rectangle (off: a field per corner).
    pub single_corner_radius: bool,
    /// The tools' persistent options by store (a tool id, or a store a family shares: `liquify`
    /// holds the Liquify tools' Global Brush Dimensions), see [`vectorcraft_tools::settings`]:
    /// kept across tool switches and saved with the preferences. Not a Preferences dialog field:
    /// resetting the preferences keeps them; `tool.setOption` edits them.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tool_settings: std::collections::BTreeMap<String, serde_json::Map<String, Value>>,
    /// View → Perspective Grid presets: the user's (the built-in ones aren't stored). A local
    /// library, not a Preferences dialog field: resetting the preferences keeps it;
    /// `perspective.presets.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub perspective_presets: Vec<vectorcraft_tools::distort::perspective::GridDefinition>,
    /// Blend Options set with no blend selected: what new blends start with
    /// (`object.blend.options`; none: Smooth Color, Align to Page). A tool setting, not a
    /// Preferences dialog field: it has no [`cmd::prefscmds::PREF_SPECS`] row and resetting the
    /// preferences keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend_options: Option<vectorcraft_doc::live::BlendDefaults>,
    /// Perspective Grid Options (double-click the Perspective Grid tool): whether the Plane
    /// Switching Widget shows and where (`perspective.widget.options`).
    pub perspective_widget: vectorcraft_tools::distort::perspective::widget::WidgetOptions,
    /// What names and codes in other apps' shortcut sets stand for, as the user confirmed them on
    /// import (`importMatches.*`). A local library, not a Preferences dialog field: resetting the
    /// preferences keeps it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub import_matches: Vec<cmd::importmatches::ImportMatch>,
}

impl Default for Prefs {
    fn default() -> Self {
        let s = |v: &str| v.to_string();
        Self {
            keyboard_increment: 1.0,
            constrain_angle: 0.0,
            corner_radius: 12.0,
            disable_auto_add_delete: false,
            use_precise_cursors: false,
            show_tool_tips: true,
            anti_aliased_artwork: true,
            select_same_tint_percent: false,
            show_home_screen: true,
            use_preview_bounds: false,
            display_print_size: false,
            double_click_to_isolate: true,
            transform_pattern_tiles: false,
            scale_corners: false,
            scale_strokes: false,
            zoom_with_mouse_wheel: false,
            scrub_numeric_fields: true,
            trackpad_rotate_view: true,
            paste_offset: 10.0,
            selection_tolerance: 3.0,
            object_selection_by_path_only: false,
            snap_to_point_tolerance: 2.0,
            ctrl_click_selects_behind: true,
            zoom_to_selection: true,
            move_locked_with_artboard: false,
            anchor_size: 3,
            handle_style: s("solid"),
            highlight_anchors_on_hover: true,
            show_handles_multiple_anchors: true,
            hide_corner_widget_above: 177.0,
            pen_rubber_band: true,
            curvature_rubber_band: true,
            type_size_increment: 2.0,
            tracking_increment: 20.0,
            baseline_shift_increment: 2.0,
            show_east_asian_options: false,
            show_indic_options: false,
            type_selection_by_path_only: false,
            font_names_in_english: true,
            auto_size_area_type: false,
            font_preview: true,
            font_preview_size: s("medium"),
            recent_fonts_count: 10,
            missing_glyph_protection: true,
            highlight_alternate_glyphs: false,
            placeholder_text: true,
            units_general: s("points"),
            units_stroke: s("points"),
            units_type: s("points"),
            units_asian_type: s("points"),
            numbers_without_units_are_points: true,
            identify_objects_by: s("objectName"),
            guide_color: s("#4affff"),
            guide_style: s("lines"),
            grid_color: s("#c8c8c8"),
            grid_style: s("lines"),
            gridline_every: 72.0,
            grid_subdivisions: 8,
            grids_in_back: true,
            show_pixel_grid: true,
            smart_guide_color: s("#ff3dfc"),
            alignment_guides: true,
            object_highlighting: true,
            transform_tools_guides: true,
            construction_guides: true,
            construction_angles: s(vectorcraft_tools::guides::DEFAULT_CONSTRUCTION_ANGLES),
            anchor_path_labels: true,
            measurement_labels: true,
            spacing_guides: true,
            distance_guides: true,
            snapping_tolerance: 4.0,
            snap_tangent: true,
            snap_perpendicular: true,
            show_slice_numbers: true,
            slice_line_color: s("#ff3f3f"),
            hyphenation_language: s("English: USA"),
            hyphenation_exceptions: String::new(),
            fonts_folder: String::new(),
            plugins_folder: String::new(),
            scratch_primary: s("Startup"),
            scratch_secondary: s("None"),
            ui_brightness: s("mediumDark"),
            ui_accent: s("coral"),
            ui_color_scheme: s("modern"),
            canvas_color: s("matchUi"),
            auto_collapse_icon_panels: false,
            tool_group_labels: false,
            open_documents_as_tabs: true,
            large_tabs: false,
            ui_scaling: 1.0,
            scale_cursor_with_ui: false,
            interface_language: s("auto"),
            system_title_bar: false,
            gpu_performance: true,
            animated_zoom: true,
            gpu_preference: s("automatic"),
            history_states: 500,
            real_time_drawing: true,
            render_threads: -1,
            linux_display_server: s("automatic"),
            background_save: true,
            background_export: true,
            autosave_recovery: true,
            autosave_interval: 2,
            recovery_folder: String::new(),
            recovery_off_for_complex: false,
            recent_files_count: 20,
            low_res_proxy_eps: false,
            anti_aliased_bitmaps: false,
            update_links: s("askWhenModified"),
            copy_as_svg: true,
            copy_as_pdf: false,
            copy_aicb: false,
            aicb_mode: s("preserveAppearance"),
            paste_text_formatting: s("keep"),
            black_on_screen: s("accurate"),
            black_output: s("accurate"),
            touch_workspace: true,
            touch_gestures: true,
            override_char_color: true,
            pattern_tile_edge_color: {
                let [r, g, b] = vectorcraft_doc::LAYER_COLORS[0].1;
                Color::rgb8(r, g, b).to_hex()
            },
            eyedropper: Default::default(),
            flattener_presets: vec![],
            width_profiles: vec![],
            japanese_crop_marks: false,
            new_art_basic: true,
            color_themes: vec![],
            new_doc_presets: vec![],
            recent_new_docs: vec![],
            pdf_presets: vec![],
            screen_export_presets: vec![],
            templates_folder: String::new(),
            append_converted: true,
            use_compression: false,
            web_export_presets: vec![],
            web_export_settings: None,
            print_presets: vec![],
            constrain_proportions: false,
            single_corner_radius: true,
            tool_settings: Default::default(),
            perspective_presets: vec![],
            blend_options: None,
            perspective_widget: Default::default(),
            import_matches: vec![],
        }
    }
}

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

impl Session {
    /// A session with no document open, the default preferences and the Selection tool. It also
    /// installs the hooks the document crate calls back into: the loader and file reader of placed
    /// documents, and where an image's effects draw its fills and strokes, for clicks.
    pub fn new() -> Self {
        // Placed documents are read by the native format's loader.
        vectorcraft_doc::placed_document::set_loader(cmd::place::document::read_document);
        vectorcraft_doc::placed_document::set_file_reader(cmd::place::document::read_file_again);
        vectorcraft_doc::placed_document::set_read_check(|path| file_access::check_read(path).is_ok());
        // The background workers of placed documents read files in the scope that asked.
        vectorcraft_render::placed_document::set_scope_capture(|| {
            let scope = file_access::ScopeHandle::capture();
            Box::new(move |f: &mut dyn FnMut()| scope.run(f))
        });
        // Clicks find an image's own fills and strokes where their effects draw them.
        vectorcraft_doc::hit::set_image_paint(vectorcraft_render::effects::image_paint);
        // OpenType SVG glyphs are drawn as the SVG importer reads their documents.
        vectorcraft_text::set_svg_glyph_reader(vectorcraft_svg::glyph_art);
        Self {
            docs: vec![],
            active: None,
            prefs: Prefs::default(),
            paint: PaintDefaults::default(),
            fill_active: true,
            clipboard: Clipboard::default(),
            journal: Journal::default(),
            tool: vectorcraft_tools::create("selection"),
            lender: None,
            last_selection_tool: None,
            last_view: ViewInfo::default(),
            preview_memo: Default::default(),
            depth: 0,
            tool_panic: None,
            tool_resets: 0,
            tool_set_since_reset: false,
            draw_mode: DrawMode::Normal,
            draw_inside: None,
            untitled_counter: 0,
            menu: Default::default(),
            gradient_stop: None,
            active_appearance_item: None,
            last_solid: Color::WHITE,
            last_gradient: GradientPaint::new(Default::default()),
            recent_colors: vec![],
            pending_paint: None,
            swatch_libraries: Default::default(),
            freeform_point: None,
            style_libraries: Default::default(),
            symbol_libraries: Default::default(),
            libraries: Default::default(),
            pdf_preset_dir: None,
            profile_dir: None,
            plugin_dir: None,
            plugin_output: None,
            recent_urls: vec![],
            ui_language: None,
            journal_note: Default::default(),
            note_depth: 1,
            journal_query: false,
            explicit_undo_group: None,
            batch_stash: None,
            place_hand_off: None,
            queue_hand_off: None,
            place_cursor_requests: false,
            package_hand_off: None,
            screens_hand_off: None,
            recovery: Default::default(),
            in_tool_actions: false,
            previewing: false,
            envelope_defaults: None,
            liquify_stroke: None,
            plane_widget_press: false,
            pointer_held: false,
            ruler_guide: None,
            port_marks: Default::default(),
            font_search: None,
            link_search: None,
            search_rules: None,
            deferred_overlay_cache: None,
            search_threads: None,
            link_stamps: Default::default(),
            link_scan: None,
            global_edit_options: Default::default(),
        }
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }
    /// The open document with [`DocState::uid`] `uid` (it may have closed since it was looked up).
    pub fn document_mut(&mut self, uid: u64) -> Option<&mut DocState> {
        self.docs.iter_mut().find(|d| d.uid == uid)
    }
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }
    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }
    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }
    pub fn doc(&self) -> Result<&DocState> {
        self.active().ok_or(EngineError::NoDocument)
    }
    pub fn doc_mut(&mut self) -> Result<&mut DocState> {
        self.active_mut().ok_or(EngineError::NoDocument)
    }
    /// Finish the tool's pending work in the current document and give it fresh state, so nothing
    /// (e.g. uncommitted typing) leaks into the document we are switching to.
    fn reset_tool_for_doc_switch(&mut self) {
        if self.active.is_none() {
            return;
        }
        let view = self.last_view;
        // The switch goes on whatever the lent tool's last actions did, as for the deactivation.
        let _ = self.give_back_tool(view);
        let acts = self.with_tool_cx(view, |t, cx| t.deactivate(cx));
        let _ = self.apply_actions(acts);
        // A batch leaves its interaction open in the document it leaves, to keep or roll back with
        // the rest of the batch; anything else in progress (a drag) is cancelled.
        if self.batch_stash.is_none() {
            let _ = self.cancel_interaction();
        }
        self.keep_tool_settings();
        self.tool = self.make_tool(self.tool.id());
        self.tool_resets = self.tool_resets.wrapping_add(1);
        self.tool_set_since_reset = false;
    }
    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            if self.active != Some(index) {
                self.reset_tool_for_doc_switch();
            }
            self.active = Some(index);
            true
        } else {
            false
        }
    }
    /// The name [`Session::next_untitled`] gives next (New Document's Name field).
    pub fn peek_untitled(&self) -> String {
        format!("Untitled-{}", self.untitled_counter + 1)
    }
    pub fn next_untitled(&mut self) -> String {
        self.untitled_counter += 1;
        format!("Untitled-{}", self.untitled_counter)
    }
    /// Text layout bounds are a cache (not saved): compute them for a document just read, or
    /// selection boxes and hit testing would use the rough estimate until each text is edited, and
    /// what is fitted to type's bounds (an envelope warping it) would come out differently. That
    /// holds for the type in symbol definitions, pattern tiles, graph designs and opacity masks too,
    /// not only in the layers (#1738).
    pub(crate) fn refresh_text_bounds(doc: &mut Document) {
        // Inline graphics in text take their size from their symbols' art (not saved either).
        doc.resolve_inline_art();
        let trees = doc.layers.iter_mut().chain(doc.symbols.iter_mut().map(|s| &mut s.art));
        let trees = trees.chain(doc.graph_designs.iter_mut().map(|g| &mut g.art));
        for tree in trees.chain(doc.patterns.iter_mut().flat_map(|p| p.art.iter_mut())) {
            refresh_tree_text_bounds(tree);
        }
    }
    /// Add a document and make it active. Between `edit.undoGroup.begin` and `edit.undoGroup.end`,
    /// an undo group with the begun group's name opens in it: here outside a command, and after the
    /// command that adds it otherwise ([`Session::execute`]).
    pub fn add_document(&mut self, mut doc: Document, path: Option<String>) -> usize {
        // Every edit is checked against the document before it, so the first one is within the cap
        // (importers and the native reader have seen to theirs already, with a warning).
        doc.limit_depth();
        Self::refresh_text_bounds(&mut doc);
        // Guides from files saved before guides had layers go on a layer.
        doc.adopt_guides();
        cmd::dimension::conform(&mut doc);
        self.reset_tool_for_doc_switch();
        let mut st = DocState::new(doc, path);
        st.history.limit = self.prefs.history_states as usize;
        self.docs.push(st);
        let i = self.docs.len() - 1;
        self.active = Some(i);
        if self.depth == 0
            && let Some(label) = self.explicit_undo_group.clone()
        {
            self.begin_named_undo_group(label);
        }
        i
    }
    /// Replace the document in tab `index` (File → Revert): new content, cleared history and
    /// selection, saved state. The tab keeps its place, path, format and view.
    pub fn replace_document(&mut self, index: usize, mut doc: Document) -> bool {
        if index >= self.docs.len() {
            return false;
        }
        doc.limit_depth();
        Self::refresh_text_bounds(&mut doc);
        doc.adopt_guides();
        cmd::dimension::conform(&mut doc);
        if self.active == Some(index) {
            // Pending tool work (typing, a drag) belongs to the content being thrown away.
            self.reset_tool_for_doc_switch();
        }
        let old = &self.docs[index];
        let mut st = DocState::new(doc, old.path.clone());
        st.history.limit = old.history.limit;
        // Same open document (caches keyed by uid stay valid); a new revision redraws it.
        st.uid = old.uid;
        st.revision = old.revision + 1;
        st.format = old.format;
        st.save_options = old.save_options.clone();
        // The remembered artboard pick follows the artboards it named when the file read back has
        // the artboards the document had at the save, in the same order and with the same ids (a
        // native file keeps them; a PDF's pages get new ids in page order); otherwise it is left
        // out.
        if old.save_artboards.is_empty() || st.doc.artboards.iter().map(|a| a.id).eq(old.save_artboards.iter().copied()) {
            st.save_artboards = old.save_artboards.clone();
        } else {
            for k in cmd::fileio::ARTBOARD_PARAMS {
                st.save_options.remove(k);
            }
        }
        st.converted = old.converted;
        st.imported_from = old.imported_from.clone();
        st.import_losses = old.import_losses.clone();
        st.view = old.view.clone();
        st.layers_open = old.layers_open.clone();
        let old = std::mem::replace(&mut self.docs[index], st);
        if let Some(stash) = &mut self.batch_stash {
            stash.push(old);
        }
        true
    }
    pub fn close_document(&mut self, index: usize) -> bool {
        if index >= self.docs.len() {
            return false;
        }
        self.reset_tool_for_doc_switch();
        // Closed (saved or discarded): nothing left to recover.
        let uid = self.docs[index].uid;
        cmd::recovery::forget(self, uid);
        let old = self.docs.remove(index);
        if let Some(stash) = &mut self.batch_stash {
            stash.push(old);
        }
        self.active = if self.docs.is_empty() { None } else { Some(index.min(self.docs.len() - 1)) };
        true
    }

    /// `spec`'s `enabled` with `ids` selected instead of the selection, which is then put back.
    fn enabled_with_selection(&mut self, spec: &CommandSpec, ids: Vec<NodeId>) -> std::result::Result<(), String> {
        let Some(d) = self.active_mut() else { return (spec.enabled)(self) };
        let old = std::mem::replace(&mut d.selection, Selection { objects: ids, ..Selection::default() });
        let r = (spec.enabled)(self);
        if let Some(d) = self.active_mut() {
            d.selection = old;
        }
        r
    }

    /// Error, under the command's own id, at the first of `ids` that is malformed, names no node, or
    /// names a layer or sublayer: a command in [`SELECTS_GIVEN_IDS`] acts on objects.
    fn check_given_ids(&self, id: &str, params: &Value) -> Result<()> {
        let (Some(st), Some(ids)) = (self.active(), params.get("ids")) else { return Ok(()) };
        let Some(given) = ids.as_array() else { return cmd::checked_ids_param(self, params, "ids", id).map(drop) };
        for value in given {
            let node = cmd::checked_id(self, value, id)?;
            if st.doc.node(node).is_some_and(|n| n.is_layer()) {
                return Err(if id.starts_with("object.arrange.") {
                    cmd::bad(id, format!("id {value} is a layer, not an object; move layers with layer.move"))
                } else {
                    cmd::bad(id, format!("id {value} is a layer, not an object"))
                });
            }
        }
        Ok(())
    }

    /// Execute a command by id. This is THE entry point for every frontend.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // The crate's own tests are the audit of the params descriptions (#1442): a key a test
        // passes that its command's description doesn't name fails the test.
        #[cfg(test)]
        self.check_params(id, params)?;
        // Objects given as `id` (where it stands for `ids` of one) or as `ids` beside it are checked
        // before anything else: a malformed one, or one naming no object, is an error under this
        // command's id, never the selection in its place. `effect.apply {id: "stylize.dropShadow"}`
        // without `effect` names the effect (its older key), not an object.
        let effect_named = id == "effect.apply"
            && params.get("effect").is_none()
            && params.get("ids").is_none_or(Value::is_null)
            && params.get("id").and_then(Value::as_str).is_some_and(|e| !e.starts_with('$'));
        if cmd::params::id_stands_for_ids(spec.params) && !effect_named && self.active().is_some() {
            cmd::given_objects(self, params, id)?;
        }
        if SELECTS_GIVEN_IDS.contains(&id) {
            self.check_given_ids(id, params)?;
        }
        // Clear's menu needs a selection; an agent supplying explicit targets only needs a
        // document. The command validates those targets before making any edit.
        let given = given_ids(spec.params, params);
        let enabled = if id == "edit.clear" && params.get("ids").is_some() {
            cmd::has_doc(self)
        } else if let Some(ids) = given.clone() {
            // Objects given as `ids` (or `id`) stand in for the selection: the command is enabled
            // as it is with them selected (#1575).
            self.enabled_with_selection(spec, ids)
        } else {
            (spec.enabled)(self)
        };
        if let Err(why) = enabled {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        // Only top-level commands are journaled (commands that call other commands would otherwise
        // be recorded twice and replay differently).
        if self.depth == 0 {
            self.journal_note.clear();
            self.note_depth = 1;
            self.journal_query = false;
            self.batch_stash = None;
        }
        // A selection-only command given `ids` (or `id`) selects them first, in the same undo step
        // (#2130); when the command then fails, the selection (and the current layer selecting
        // them made current) goes back to how it was, as when the ids are refused.
        let select_first = SELECTS_GIVEN_IDS
            .contains(&id)
            .then(|| {
                params.get("ids").filter(|v| !v.is_null()).cloned().or_else(|| given.map(|ids| ids.iter().map(|i| i.0).collect::<Vec<_>>().into()))
            })
            .flatten();
        let selected_before = select_first.as_ref().and(self.active()).map(|d| (d.uid, d.selection.clone(), d.active_layer));
        let run = |s: &mut Session| {
            if let Some(ids) = &select_first {
                s.execute("select.set", &serde_json::json!({ "ids": ids }))?;
            }
            let r = (spec.run)(s, params);
            if r.is_err()
                && let Some((uid, selection, layer)) = selected_before
                && let Some(st) = s.docs.iter_mut().find(|d| d.uid == uid)
            {
                st.selection = selection;
                st.active_layer = layer;
                st.revision += 1;
            }
            r
        };
        // The entries the command journals belong to the undo groups of the documents it edits, and to
        // the group of the document it starts in when it ends there too.
        let (doc, journal, grouped) = (self.active().map(|d| d.uid), self.journal.len(), self.undo_group_docs());
        let open = (self.depth == 0).then(|| self.docs.iter().map(|d| d.uid).collect::<Vec<_>>());
        let r = if self.depth == 0 { self.run_guarded(id, run) } else { self.run_nested(run) };
        let r = r?;
        if self.depth == 0 {
            self.inherit_new_art();
        }
        if let Some(open) = open {
            self.open_undo_groups_in_new_documents(&open);
        }
        let journaled = spec.journal || (self.depth == 0 && std::mem::take(&mut self.journal_query));
        if journaled && self.depth == 0 && self.active().is_none_or(|d| d.interaction.is_none()) {
            let p = self.noted(params);
            self.journal.push((id.to_string(), p));
        }
        if self.depth == 0 {
            self.journal_in_undo_groups(doc, &grouped, journal);
            self.after_command();
        }
        Ok(r)
    }

    /// Is the interface in Japanese? Then new type starts with em box top-to-top leading and em
    /// box centre character alignment (#432).
    pub fn japanese_interface(&self) -> bool {
        self.ui_language.as_deref().unwrap_or(&self.prefs.interface_language).eq_ignore_ascii_case("ja")
    }

    /// Record `key: value` in the running top-level command's journal entry (or its interaction's
    /// preview, or its step in a batch) unless its params give `key`: a value it resolved from the
    /// preferences or the clock.
    pub fn note_journal(&mut self, key: &str, value: Value) {
        if self.depth == self.note_depth {
            self.journal_note.insert(key.to_string(), value);
        }
    }

    /// Journal the running query command when it is the top-level command: it changed the
    /// selection (`document.find {select: true}`), so an action recorded around it repeats that.
    pub(crate) fn journal_query(&mut self) {
        if self.depth == 1 {
            self.journal_query = true;
        }
    }

    /// Run `id` as a step of the running command, which journals its steps (`command.batch`):
    /// → the step's result and its params with what it noted ([`Session::note_journal`]), for the
    /// step in the running command's journal entry.
    pub(crate) fn execute_step(&mut self, id: &str, params: &Value) -> Result<(Value, Value)> {
        let outer = (std::mem::take(&mut self.journal_note), self.note_depth);
        self.note_depth = self.depth + 1;
        let r = self.execute(id, params);
        let noted = self.noted(params);
        (self.journal_note, self.note_depth) = outer;
        Ok((r?, noted))
    }

    /// The journal from entry `start` on, as an action records it: without the params that only
    /// pin a replay to the original run ([`REPLAY_ONLY`]), so playing the action later acts now.
    pub fn journal_for_action(&self, start: usize) -> Vec<(String, Value)> {
        self.journal
            .since(start)
            .iter()
            .cloned()
            .map(|(id, mut p)| {
                strip_replay_only(&id, &mut p);
                (id, p)
            })
            .collect()
    }

    /// `params` with the noted values added (see [`Session::note_journal`]).
    fn noted(&mut self, params: &Value) -> Value {
        let mut note = std::mem::take(&mut self.journal_note);
        match params {
            // The params' own values win.
            Value::Object(m) if !note.is_empty() => {
                note.extend(m.clone());
                Value::Object(note)
            }
            Value::Null if !note.is_empty() => Value::Object(note),
            _ => params.clone(),
        }
    }

    fn run_nested(&mut self, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    /// A top-level command: a panic (a bug) becomes [`EngineError::Internal`] and the active
    /// document goes back to how it was before the command, instead of crashing the frontend.
    fn run_guarded(&mut self, id: &str, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        let snapshot = self.active().map(|d| (d.uid, d.doc.clone(), d.selection.clone(), d.interaction.clone()));
        let active = self.active;
        let r = match guard::catch_panic(|| self.run_nested(f)) {
            Ok(r) => r,
            Err(msg) => {
                self.depth = 0;
                self.journal_note.clear();
                if let Some((uid, doc, selection, interaction)) = snapshot
                    && let Some(st) = self.docs.iter_mut().find(|d| d.uid == uid)
                {
                    st.doc = doc;
                    st.selection = selection;
                    st.interaction = interaction;
                    cmd::globaledit::follow_history(st);
                    st.revision += 1;
                }
                if active.is_some_and(|i| i < self.docs.len()) {
                    self.active = active;
                }
                Err(EngineError::Internal { cmd: id.to_string(), msg })
            }
        };
        // Undo, Redo and a failed batch bring back documents with their selections: each document
        // gets its selection's new art on a locked artboard.
        for st in &mut self.docs {
            st.sync_new_art();
        }
        r
    }

    /// Run `f` on a mutable copy of the active document. Outside an interaction this records one
    /// undo step labelled `label`; inside one it just mutates (the interaction commits later).
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>) -> Result<T> {
        // An edit made while typing (the Transform panel) is a step of its own: inside the typing
        // session, the next keystroke's preview would start again from before it (#1881).
        if !self.previewing && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        let before = st.doc.clone();
        let before_sel = st.selection.clone();
        let doc = Arc::make_mut(&mut st.doc);
        // The Global Edit members the edit reached besides the selection.
        let mut spread = vec![];
        // Objects attached to a path follow it, and dimensions follow the art they measure.
        let result = match f(doc, &mut st.selection).and_then(|v| {
            // Global Edit: the other members take what the edit changed about the one it edited.
            if let Some(ge) = &st.global_edit {
                spread = cmd::globaledit::propagate(&before, doc, ge, [&before_sel.objects, &st.selection.objects]);
            }
            cmd::objects_on_path::refresh(&before, doc);
            cmd::dimension::refresh(&before, doc)?;
            Ok(v)
        }) {
            Ok(v) => match cmd::shaper::refresh(&before, Arc::make_mut(&mut st.doc)) {
                Err(e) => Err(e),
                Ok(()) => {
                    // Attached Shaper groups whose art was rebuilt go back on their path.
                    cmd::objects_on_path::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Inline graphics in text follow their symbols.
                    cmd::inline::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Text Wrap: area type follows its wrap objects; then threads re-flow.
                    cmd::textwrap::refresh(Arc::make_mut(&mut st.doc));
                    // Type in no thread numbers its list paragraphs from 1 (before the mask copies
                    // its editing layer's art).
                    cmd::threads::restart_unthreaded_counts(&mut st.doc);
                    // Opacity-mask editing: the mask follows its art on the editing layer.
                    if st.doc.mask_edit.is_some() {
                        cmd::maskedit::sync(Arc::make_mut(&mut st.doc));
                    }
                    // Threaded text re-flows when any of its frames changed.
                    if !st.doc.text_threads.is_empty() {
                        cmd::threads::reflow(&before, Arc::make_mut(&mut st.doc));
                    }
                    // Asset Export: assets let go of deleted art.
                    if !st.doc.assets.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_assets();
                    }
                    // Variables: bindings let go of deleted art.
                    if !st.doc.variables.bindings.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_variable_bindings();
                    }
                    if st.doc.too_deep(Some(&before)) {
                        // Deeper trees overflow the stack of what walks them.
                        Err(EngineError::Other(TOO_DEEP.into()))
                    } else if doc_sane(&st.doc, &st.selection) && objects_in_range(&st.doc, &spread) {
                        Ok(v)
                    } else {
                        Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()))
                    }
                }
            },
            Err(e) => Err(e),
        };
        match result {
            Ok(v) => {
                st.selection.prune(&st.doc);
                // An edit that selects what it unlocks, shows or pastes leaves locked guides out,
                // and one that selects the slices it makes leaves them out while they are locked
                // or hidden.
                deselect_hidden(&menu, &st.doc, &mut st.selection);
                // The art on a locked artboard can't stay selected, however it got there, except
                // the new art this edit made and new art still selected.
                st.protect_locked_artboards(&before_sel.objects, &before_sel.new_art, Some(&*before));
                st.revision += 1;
                if st.interaction.is_none() {
                    let selection = match st.history.undo.last() {
                        Some(l) if *l.selection == before_sel => l.selection.clone(),
                        _ => Arc::new(before_sel),
                    };
                    st.push_undo(label.to_string(), before, selection);
                }
                Ok(v)
            }
            Err(e) => {
                st.doc = before;
                st.selection = before_sel;
                cmd::globaledit::follow_history(st);
                Err(e)
            }
        }
    }

    /// [`Session::edit`] with the step named `template`, its `{name}` placeholder filled with `name`
    /// (a style's name). The undo step it records keeps the template and the name
    /// ([`HistoryEntry::named`]), and a frontend shows the template's translation with the name in
    /// it. An edit that records no step of its own (inside an interaction, or a later edit of an
    /// undo group) keeps neither, and neither does a step an undo group names.
    pub fn edit_named<T>(
        &mut self,
        template: &'static str,
        name: &str,
        built_in: bool,
        f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>,
    ) -> Result<T> {
        let named = NamedStep { template, name: name.to_string(), built_in };
        let step = named.label();
        // The step this edit records keeps the document as it is now.
        let now = Arc::as_ptr(&self.doc()?.doc);
        let v = self.edit(&step, f)?;
        if let Ok(st) = self.doc_mut()
            && let Some(e) = st.history.undo.last_mut()
            && Arc::as_ptr(&e.doc) == now
            && e.label == step
        {
            e.named = Some(named);
        }
        Ok(v)
    }

    /// Change only the selection (not an undo step).
    pub fn select(&mut self, f: impl FnOnce(&Document, &mut Selection)) -> Result<()> {
        self.active_appearance_item = None;
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        let before = st.selection.objects.clone();
        let new_art = st.selection.new_art.clone();
        f(&st.doc, &mut st.selection);
        st.selection.prune(&st.doc);
        // Lock Guides locks the paths made into guides too (#1534), and Lock Slices and Hide
        // Slices keep the slices out, whatever selects them.
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        // The art on a locked artboard can't be selected, whatever selects it, except new art
        // that was selected and still is.
        st.protect_locked_artboards(&before, &new_art, None);
        // Selecting art makes its layer (or sublayer) the current one, as in the Layers panel of
        // the reference app: new art then goes beside it.
        if st.selection.objects != before
            && let Some(layer) = st.selection.objects.last().and_then(|id| st.doc.layer_containing(*id)).filter(|l| st.doc.is_editable(*l))
            && st.active_layer != Some(layer)
        {
            st.active_layer = Some(layer);
            st.layer_rows.clear();
        }
        st.revision += 1;
        // Puppet Warp pins belong to the art they were placed on: another selection starts afresh.
        if st.doc.puppet.as_ref().is_some_and(|p| p.ids != st.selection.objects) {
            cmd::distortcmds::drop_puppet_pins(st);
        }
        Ok(())
    }

    // ---------- interactions (live drags) ----------

    pub(crate) fn clear_deferred_overlay_cache(&mut self) {
        self.deferred_overlay_cache = None;
    }

    /// The interaction's latest preview command when the document is still its snapshot (deferred
    /// Real-time Drawing), regardless of the preference at commit or overlay time.
    pub(crate) fn unapplied_interaction_preview(&self) -> Option<(String, Value)> {
        self.active().and_then(|d| d.interaction.as_ref().filter(|it| std::sync::Arc::ptr_eq(&it.doc, &d.doc)).and_then(|it| it.preview.clone()))
    }

    pub fn begin_interaction(&mut self, label: &str) -> Result<()> {
        // Typing in progress is kept first: a drag or a dialog's preview would otherwise start
        // from the snapshot taken before the typing, and drop it (#1881).
        if label != "Typing" && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
        self.clear_deferred_overlay_cache();
        let st = self.doc_mut()?;
        if st.interaction.is_some() {
            return Ok(());
        }
        st.interaction = Some(Interaction {
            label: label.to_string(),
            doc: st.doc.clone(),
            selection: Arc::new(st.selection.clone()),
            preview: None,
            active_layer: st.active_layer,
            layer_rows: st.layer_rows.clone(),
            variables_highlight: st.variables_highlight.clone(),
            isolation: st.isolation,
            perspective_again: None,
            copies: None,
        });
        Ok(())
    }

    /// Re-apply `cmd` on top of the interaction snapshot (replacing the previous preview).
    pub fn preview(&mut self, cmd: &str, params: &Value) -> Result<Value> {
        {
            let menu = self.menu.clone();
            let st = self.doc_mut()?;
            let Some(it) = &st.interaction else { return Err(EngineError::Other("no interaction in progress".into())) };
            // Undoing the previous preview counts as a change, so views redraw even when the command
            // below returns without an edit.
            let selection_changed = st.selection != *it.selection;
            if !Arc::ptr_eq(&st.doc, &it.doc) || selection_changed {
                st.revision += 1;
            }
            st.doc = it.doc.clone();
            if selection_changed {
                st.selection.clone_from(&it.selection);
            }
            // Guides locked and slices locked or hidden during the interaction stay deselected,
            // even when the preview fails.
            deselect_hidden(&menu, &st.doc, &mut st.selection);
            cmd::globaledit::follow_history(st);
        }
        // The previous preview's paint is undone with it.
        self.pending_paint = None;
        let rw = cmd::distortcmds::perspective_rewrite(self, cmd, params).or_else(|| cmd::bounds_rewrite(self, cmd, params));
        let (cmd, params) = rw.as_ref().map_or((cmd, params), |(c, p)| (c.as_str(), p));
        let outer = std::mem::replace(&mut self.previewing, true);
        let r = self.execute(cmd, params);
        self.previewing = outer;
        let params = self.noted(params);
        let st = self.doc_mut()?;
        if let Some(it) = &mut st.interaction {
            // A failed preview leaves the document at the snapshot, so nothing is left to commit
            // (not even an earlier successful preview, which the reset above already undid).
            it.preview = r.is_ok().then(|| (cmd.to_string(), params));
        }
        r
    }

    pub fn commit_interaction(&mut self) -> Result<()> {
        self.clear_deferred_overlay_cache();
        self.remember_pending_paint();
        let st = self.doc_mut()?;
        let Some(mut it) = st.interaction.take() else { return Ok(()) };
        let Some(preview) = it.preview.take() else { return Ok(()) };
        let perspective_again = it.perspective_again.take();
        let copy = preview.0 == "object.transform" && preview.1.get("copy").and_then(Value::as_bool).unwrap_or(false);
        // Alt pressed or released during a move drag turns it into a copy or back: name it for
        // what it did in the end.
        match (it.label.as_str(), copy) {
            ("Move", true) => it.label = "Copy".into(),
            ("Copy", false) if preview.0 == "object.transform" => it.label = "Move".into(),
            _ => {}
        }
        // A preview that left the document as it was records no Transform Again either (as
        // `keep_interaction` records no undo step): the last real transformation stands.
        let changed = !Arc::ptr_eq(&it.doc, &st.doc);
        st.keep_interaction(it);
        if preview.0 == "object.transform" {
            let m = cmd::matrix_param(&preview.1, "matrix");
            if let Some(m) = m {
                // The options are the last preview's params with the values it noted, without the
                // matrix and the ids.
                let mut options = preview.1.clone();
                if let Some(o) = options.as_object_mut() {
                    o.remove("matrix");
                    o.remove("ids");
                }
                st.last_transform = Some(LastTransform::Matrix(m, options));
                st.last_perspective = None;
            }
        } else if preview.0 == "object.transformEach" && changed {
            // Transform Again repeats it as it stood when OK was pressed.
            st.last_transform = Some(LastTransform::Each(preview.1.clone()));
            st.last_perspective = None;
        }
        if perspective_again.is_some() {
            st.last_perspective = perspective_again;
        }
        let (doc, journal, grouped) = (self.active().map(|d| d.uid), self.journal.len(), self.undo_group_docs());
        self.journal.push(preview);
        self.journal_in_undo_groups(doc, &grouped, journal);
        Ok(())
    }

    /// Remember the paint a live preview applied (as its interaction is kept).
    pub(crate) fn remember_pending_paint(&mut self) {
        if let Some(p) = self.pending_paint.take() {
            self.remember_paint_now(&p);
        }
    }

    pub fn cancel_interaction(&mut self) -> Result<()> {
        self.clear_deferred_overlay_cache();
        self.pending_paint = None;
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        st.undo_interaction();
        // Guides locked and slices locked or hidden during the interaction stay deselected.
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        Ok(())
    }

    pub fn in_interaction(&self) -> bool {
        self.active().is_some_and(|d| d.interaction.is_some())
    }

    // ---------- undo groups (scrubbed numeric fields) ----------

    /// Open an undo group in the active document: until [`Session::end_undo_group`], the edits
    /// made there (commands, committed interactions) are one undo step. A scrubbed numeric field
    /// applies each value it passes as its own command, as a typed value is applied; the drag is
    /// one step.
    pub fn begin_undo_group(&mut self) {
        self.begin_named_undo_group(None);
    }

    /// [`Session::begin_undo_group`] with the undo step named `label` (else after its first edit).
    pub fn begin_named_undo_group(&mut self, label: Option<String>) {
        if let Some(st) = self.active_mut()
            && st.undo_group.is_none()
        {
            st.undo_group = Some(UndoGroup { first: None, journal: vec![], label });
        }
    }

    /// Between `edit.undoGroup.begin` and `edit.undoGroup.end`, open an undo group with the begun
    /// group's name in each document that the top-level command just run opened or created (its
    /// [`DocState::uid`] is not in `open`). The group takes the edits after that command: the
    /// command's own edits there stay its undo step, and a canceled group keeps them, as the journal
    /// entry that made the document stays.
    fn open_undo_groups_in_new_documents(&mut self, open: &[u64]) {
        let Some(label) = &self.explicit_undo_group else { return };
        for st in &mut self.docs {
            if !open.contains(&st.uid) && st.undo_group.is_none() {
                st.undo_group = Some(UndoGroup { first: None, journal: vec![], label: label.clone() });
            }
        }
    }

    /// The documents with an undo group open ([`DocState::uid`]), each with the document it holds.
    fn undo_group_docs(&self) -> Vec<(u64, *const Document)> {
        self.docs.iter().filter(|st| st.undo_group.is_some()).map(|st| (st.uid, Arc::as_ptr(&st.doc))).collect()
    }

    /// Note the journal entries from number `from` on in the undo groups they belong to: the group of
    /// each document in `grouped` ([`Session::undo_group_docs`] before the entries) that they changed,
    /// each group they opened and edited, and the group of the document with [`DocState::uid`] `doc`
    /// when they started in it and it is still the active one. A cancel drops them. The last entry of a command that changed the
    /// active document keeps the index of the document it left active.
    fn journal_in_undo_groups(&mut self, doc: Option<u64>, grouped: &[(u64, *const Document)], from: usize) {
        let to = self.journal.len();
        let active = self.active().map(|d| d.uid);
        let switched = if active == doc { None } else { self.active };
        for st in &mut self.docs {
            let Some(g) = &mut st.undo_group else { continue };
            let started = doc == Some(st.uid) && active == doc;
            // A group the entries opened has an edit of theirs once it has a first edit.
            let edited = match grouped.iter().find(|&&(uid, _)| uid == st.uid) {
                Some(&(_, d)) => !std::ptr::eq(d, Arc::as_ptr(&st.doc)),
                None => g.first.is_some(),
            };
            if started || edited {
                // A committed interaction inside the command noted its entry already.
                let noted = g.journal.partition_point(|&(n, _)| n < from);
                g.journal.truncate(noted);
                g.journal.extend((from..to).map(|n| (n, switched.filter(|_| n + 1 == to))));
            }
        }
    }

    /// Close the open undo groups: their edits stay one undo step or, `cancel`led (Escape), are
    /// undone and dropped from the journal. A group begun with `edit.undoGroup.begin` ends too, so
    /// documents opened after this open no undo group.
    pub fn end_undo_group(&mut self, cancel: bool) {
        self.explicit_undo_group = None;
        self.close_undo_groups(cancel);
    }

    /// [`Session::end_undo_group`] without ending a group begun with `edit.undoGroup.begin`.
    fn close_undo_groups(&mut self, cancel: bool) {
        let mut journal: Vec<(usize, Option<usize>)> = vec![];
        for st in &mut self.docs {
            let Some(g) = st.undo_group.take() else { continue };
            // Only while the group's step is still the newest one.
            if cancel
                && let Some(first) = g.first
                && st.history.undo.last().is_some_and(|e| Arc::ptr_eq(&e.doc, &first))
                && let Some(e) = st.history.undo.pop()
            {
                st.doc = e.doc;
                st.selection = Arc::unwrap_or_clone(e.selection);
                deselect_hidden(&self.menu, &st.doc, &mut st.selection);
                st.sync_new_art();
                cmd::globaledit::follow_history(st);
                st.revision += 1;
                // The journal drops only the canceled groups' own entries and keeps the ones made in other documents meanwhile.
                journal.extend(g.journal);
            }
        }
        // A batch that edited two of the documents is in both groups.
        journal.sort_unstable();
        journal.dedup_by_key(|&mut (n, _)| n);
        // An entry that changed the active document becomes that change, so the entries after it run
        // in the same document when the journal is replayed.
        for &(n, index) in &journal {
            if let Some(index) = index {
                self.journal.replace(n, ("document.activate".to_string(), serde_json::json!({ "index": index })));
            }
        }
        let gone: Vec<usize> = journal.into_iter().filter(|(_, index)| index.is_none()).map(|(n, _)| n).collect();
        self.journal.remove(&gone);
    }

    /// [`Session::end_undo_group`] for the open document with [`DocState::uid`] `uid` only (it may
    /// have closed since): its open undo group closes, and undo groups open in other documents stay
    /// open.
    fn end_document_undo_group(&mut self, uid: u64, cancel: bool) {
        // The other documents' groups are set aside while `end_undo_group` closes this one's.
        let others: Vec<(u64, UndoGroup)> =
            self.docs.iter_mut().filter(|st| st.uid != uid).filter_map(|st| Some((st.uid, st.undo_group.take()?))).collect();
        self.close_undo_groups(cancel);
        for (other, g) in others {
            if let Some(st) = self.document_mut(other) {
                st.undo_group = Some(g);
            }
        }
    }

    /// Make the edits that follow in the active document one undo step, until
    /// [`Session::end_undo_scope`]: open an undo group there or, when one is open already, join it,
    /// so that the edits are part of its step. `None` without a document.
    pub fn begin_undo_scope(&mut self) -> Option<UndoScope> {
        let journal = self.journal.len();
        let st = self.active()?;
        let uid = st.uid;
        let joined = st.undo_group.as_ref().map(|g| (st.doc.clone(), st.selection.clone(), g.first.clone()));
        if joined.is_none() {
            self.begin_undo_group();
        }
        Some(UndoScope { uid, journal, joined })
    }

    /// End `scope` in its own document, whichever document is active now; undo groups open in other
    /// documents stay open. The undo group the scope opened closes as [`Session::end_undo_group`]
    /// closes it. A scope that joined an open group leaves the group open; with `cancel`, it puts the
    /// document and its selection back as they were when the scope began and cuts the journal back to
    /// where the scope began (only while the group's step is still the newest one), and an undo step
    /// that the scope's first edit recorded goes too.
    pub fn end_undo_scope(&mut self, scope: UndoScope, cancel: bool) {
        let Some((doc, selection, first)) = scope.joined else {
            self.end_document_undo_group(scope.uid, cancel);
            return;
        };
        let menu = self.menu.clone();
        let Some(st) = self.document_mut(scope.uid) else { return };
        let Some(g) = &mut st.undo_group else { return };
        let newest = g.first.as_ref().is_some_and(|f| st.history.undo.last().is_some_and(|e| Arc::ptr_eq(&e.doc, f)));
        if !cancel || !newest {
            return;
        }
        match &first {
            // The scope's first edit recorded the group's step.
            None => {
                st.history.undo.pop();
                g.first = None;
            }
            Some(f) if g.first.as_ref().is_some_and(|now| Arc::ptr_eq(now, f)) => {}
            Some(_) => return,
        }
        st.doc = doc;
        st.selection = selection;
        cmd::globaledit::follow_history(st);
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        st.history.recost_last(&st.doc);
        st.revision += 1;
        self.journal.truncate(scope.journal);
    }

    /// Commands with enablement (for menus, palette, MCP `list_commands`).
    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().map(|c| c.info(self)).collect()
    }
}

/// Lay out the type in tree `n` (opacity mask art included) whose bounds aren't cached. Only the
/// nodes on the way to such type are copied.
fn refresh_tree_text_bounds(n: &mut Arc<vectorcraft_doc::Node>) {
    fn stale(n: &vectorcraft_doc::Node) -> bool {
        matches!(&n.kind, NodeKind::Text(t) if t.cached_bounds.is_none())
            || n.extras().mask.as_deref().is_some_and(|m| stale(&m.art))
            || n.children().is_some_and(|ch| ch.iter().any(|c| stale(c)))
    }
    if !stale(n) {
        return;
    }
    let n = Arc::make_mut(n);
    if let NodeKind::Text(t) = &mut n.kind
        && t.cached_bounds.is_none()
    {
        cmd::typecmd::refresh_bounds(t);
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
        refresh_tree_text_bounds(&mut m.art);
    }
    for c in n.children_mut().into_iter().flatten() {
        refresh_tree_text_bounds(c);
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
mod tests;
#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_appearance;
#[cfg(test)]
mod tests_areafit;
#[cfg(test)]
mod tests_assets;
#[cfg(test)]
mod tests_attributes;
#[cfg(test)]
mod tests_bboxrotate;
#[cfg(test)]
mod tests_blendfidelity;
#[cfg(test)]
mod tests_blendopts;
#[cfg(test)]
mod tests_blendspine;
#[cfg(test)]
mod tests_brushsym;
#[cfg(test)]
mod tests_build;
#[cfg(test)]
mod tests_cancel;
#[cfg(test)]
mod tests_charstroke;
#[cfg(test)]
mod tests_clip;
#[cfg(test)]
mod tests_clipboard;
#[cfg(test)]
mod tests_clipflavours;
#[cfg(test)]
mod tests_clippaint;
#[cfg(test)]
mod tests_cmdsplit;
#[cfg(test)]
mod tests_cmykflatten;
#[cfg(test)]
mod tests_colorguide;
#[cfg(test)]
mod tests_colorguidelib;
#[cfg(test)]
mod tests_colormgmt;
#[cfg(test)]
mod tests_colorthemes;
#[cfg(test)]
mod tests_colourfonts;
#[cfg(test)]
mod tests_compoundpaint;
#[cfg(test)]
mod tests_constrain_new;
#[cfg(test)]
mod tests_constrain_tools;
#[cfg(test)]
mod tests_containers;
#[cfg(test)]
mod tests_css;
#[cfg(test)]
mod tests_cut;
#[cfg(test)]
mod tests_dashalign;
#[cfg(test)]
mod tests_distort;
#[cfg(test)]
mod tests_docsetup;
#[cfg(test)]
mod tests_draw2;
#[cfg(test)]
mod tests_editcolors;
#[cfg(test)]
mod tests_effectedit;
#[cfg(test)]
mod tests_emptytype;
#[cfg(test)]
mod tests_envelope;
#[cfg(test)]
mod tests_envelope_distort;
#[cfg(test)]
mod tests_envelope_edit;
#[cfg(test)]
mod tests_expand;
#[cfg(test)]
mod tests_eyedropper;
#[cfg(test)]
mod tests_file;
#[cfg(test)]
mod tests_fileinfo;
#[cfg(test)]
mod tests_flatpresets;
#[cfg(test)]
mod tests_flatpreview;
#[cfg(test)]
mod tests_flatten;
#[cfg(test)]
mod tests_focal;
#[cfg(test)]
mod tests_fontfiles;
#[cfg(test)]
mod tests_fontlist;
#[cfg(test)]
mod tests_freeform;
#[cfg(test)]
mod tests_globaledit;
#[cfg(test)]
mod tests_gradient;
#[cfg(test)]
mod tests_gradpanel;
#[cfg(test)]
mod tests_gradspan;
#[cfg(test)]
mod tests_halftone;
#[cfg(test)]
mod tests_history_mem;
#[cfg(test)]
mod tests_ids_without_selection;
#[cfg(test)]
mod tests_inline;
#[cfg(test)]
mod tests_interactivity;
#[cfg(test)]
mod tests_intertwine;
#[cfg(test)]
mod tests_journal;
#[cfg(test)]
mod tests_keyguide;
#[cfg(test)]
mod tests_knockout;
#[cfg(test)]
mod tests_labspots;
#[cfg(test)]
mod tests_large_edits;
#[cfg(test)]
mod tests_layerclip;
#[cfg(test)]
mod tests_layers;
#[cfg(test)]
mod tests_legacy_ai;
#[cfg(test)]
mod tests_library;
#[cfg(test)]
mod tests_link_paths;
#[cfg(test)]
mod tests_linked_stops;
#[cfg(test)]
mod tests_linkfiles;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_linkspanel;
#[cfg(test)]
mod tests_linkwatch;
#[cfg(test)]
mod tests_liquify;
#[cfg(test)]
mod tests_live;
#[cfg(test)]
mod tests_livecorners;
#[cfg(test)]
mod tests_maskview;
#[cfg(test)]
mod tests_menucmds;
#[cfg(test)]
mod tests_nativefile;
#[cfg(test)]
mod tests_newart;
#[cfg(test)]
mod tests_newdoc;
#[cfg(test)]
mod tests_objects_on_path;
#[cfg(test)]
mod tests_objexpand;
#[cfg(test)]
mod tests_opacitymask;
#[cfg(test)]
mod tests_outlineobject;
#[cfg(test)]
mod tests_outlinestroke;
#[cfg(test)]
mod tests_overprint;
#[cfg(test)]
mod tests_package;
#[cfg(test)]
mod tests_paintproxy;
#[cfg(test)]
mod tests_panelcmds;
#[cfg(test)]
mod tests_paragraphs;
#[cfg(test)]
mod tests_params;
#[cfg(test)]
mod tests_pathops;
#[cfg(test)]
mod tests_pathtype;
#[cfg(test)]
mod tests_pattern;
#[cfg(test)]
mod tests_pdffidelity;
#[cfg(test)]
mod tests_pdfmarks;
#[cfg(test)]
mod tests_pdfpresets;
#[cfg(test)]
mod tests_pdfraster;
#[cfg(test)]
mod tests_persp_planes;
#[cfg(test)]
mod tests_persp_select;
#[cfg(test)]
mod tests_persp_text;
#[cfg(test)]
mod tests_perspgrid;
#[cfg(test)]
mod tests_place;
#[cfg(test)]
mod tests_placed_document;
#[cfg(test)]
mod tests_plugins;
#[cfg(test)]
mod tests_prefs;
#[cfg(test)]
mod tests_previewbounds;
#[cfg(test)]
mod tests_print;
#[cfg(test)]
mod tests_printadvanced;
#[cfg(test)]
mod tests_printpresets;
#[cfg(test)]
mod tests_printpreview;
#[cfg(test)]
mod tests_printps;
#[cfg(test)]
mod tests_printtiling;
#[cfg(test)]
mod tests_proxyitems;
#[cfg(test)]
mod tests_puppetwarp;
#[cfg(test)]
mod tests_rasterfilters;
#[cfg(test)]
mod tests_rastersettings;
#[cfg(test)]
mod tests_recolor;
#[cfg(test)]
mod tests_recovery;
#[cfg(test)]
mod tests_reflecthit;
#[cfg(test)]
mod tests_registration;
#[cfg(test)]
mod tests_save;
#[cfg(test)]
mod tests_saveoptions;
#[cfg(test)]
mod tests_scalestrokes;
#[cfg(test)]
mod tests_scripts;
#[cfg(test)]
mod tests_select_ids_first;
#[cfg(test)]
mod tests_shaper;
#[cfg(test)]
mod tests_shortcutset;
#[cfg(test)]
mod tests_simplify_part;
#[cfg(test)]
mod tests_slices;
#[cfg(test)]
mod tests_smartguides;
#[cfg(test)]
mod tests_store;
#[cfg(test)]
mod tests_strokegeom;
#[cfg(test)]
mod tests_strokegradient;
#[cfg(test)]
mod tests_strokereach;
#[cfg(test)]
mod tests_strokeux;
#[cfg(test)]
mod tests_stylelib;
#[cfg(test)]
mod tests_stylepanel;
#[cfg(test)]
mod tests_styles;
#[cfg(test)]
mod tests_swatchcmds;
#[cfg(test)]
mod tests_swatches;
#[cfg(test)]
mod tests_swatchlib;
#[cfg(test)]
mod tests_symbolbounds;
#[cfg(test)]
mod tests_symbollib;
#[cfg(test)]
mod tests_targeting;
#[cfg(test)]
mod tests_textcombos;
#[cfg(test)]
mod tests_textedit;
#[cfg(test)]
mod tests_textimport;
#[cfg(test)]
mod tests_tileedge;
#[cfg(test)]
mod tests_tints;
#[cfg(test)]
mod tests_toolsettings;
#[cfg(test)]
mod tests_transformfx;
#[cfg(test)]
mod tests_transparencygrid;
#[cfg(test)]
mod tests_trap;
#[cfg(test)]
mod tests_typearea;
#[cfg(test)]
mod tests_typescale;
#[cfg(test)]
mod tests_units;
#[cfg(test)]
mod tests_variables;
#[cfg(test)]
mod tests_webexport;
#[cfg(test)]
mod tests_widthpoints;
#[cfg(test)]
mod tests_widthprofiles;
#[cfg(test)]
mod tests_widthtool;
#[cfg(test)]
mod tests_xform;

/// Commands that act on the selection and take `ids` by selecting them first (`select.set`, with
/// its checks), then running as on the selection: the result and the selection afterwards are the
/// menu command's (#2130).
pub(crate) const SELECTS_GIVEN_IDS: &[&str] = &[
    "edit.duplicate",
    "object.arrange.bringForward",
    "object.arrange.bringToFront",
    "object.arrange.sendBackward",
    "object.arrange.sendToBack",
    "object.arrange.sendToCurrentLayer",
    "object.clippingMask.make",
    "object.clippingMask.release",
    "object.compoundPath.make",
    "object.compoundPath.release",
    "object.group",
    "object.hide",
    "object.lock",
    "object.move",
    "object.pathfinder.crop",
    "object.pathfinder.divide",
    "object.pathfinder.exclude",
    "object.pathfinder.intersect",
    "object.pathfinder.merge",
    "object.pathfinder.minusBack",
    "object.pathfinder.minusFront",
    "object.pathfinder.outline",
    "object.pathfinder.trap",
    "object.pathfinder.trim",
    "object.pathfinder.unite",
    "object.reflect",
    "object.rotate",
    "object.scale",
    "object.shear",
    "object.ungroup",
];

/// The non-empty `ids` given to a command whose params description names `ids` as a key (the
/// key callers outside the app may pass it, [`cmd::params::takes_key`]), or its `id` as `ids` of
/// one when no `ids` are given and the description takes `id` so
/// ([`cmd::params::id_stands_for_ids`]), as [`cmd::targets`] reads them.
fn given_ids(description: &'static str, params: &Value) -> Option<Vec<NodeId>> {
    if !cmd::params::takes_key(description, "ids") {
        return None;
    }
    match params.get("ids").filter(|v| !v.is_null()) {
        Some(_) => cmd::ids_param(params, "ids"),
        None => cmd::id_param(params, "id").filter(|_| cmd::params::id_stands_for_ids(description)).map(|id| vec![id]),
    }
    .filter(|ids| !ids.is_empty())
}
