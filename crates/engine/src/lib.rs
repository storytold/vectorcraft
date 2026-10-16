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
pub mod namematch;
pub mod steps;
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
pub use cmd::distortcmds::perspective_click;
pub use cmd::params::{check_keys, check_params};
pub use cmd::rasterfx::{export_pdf, flatten_raster_effects};
pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use journal::Journal;
pub use tooling::{UiRequest, ViewInfo};
pub use vectorcraft_doc as doc;
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
    /// A bug: the command panicked. The document was rolled back to its state before the command.
    #[error("internal error in `{cmd}`: {msg} (the document was left as it was before; please report this bug)")]
    Internal { cmd: String, msg: String },
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
}

impl HistoryEntry {
    /// A step keeping `doc` and `selection`, as they were before the edit that made `next`.
    pub(crate) fn kept(label: String, doc: Arc<Document>, selection: Arc<Selection>, next: &Document) -> Self {
        doc.drop_lookup_cache();
        let (bytes, next_bytes) = doc.retained_bytes_each(next);
        HistoryEntry { label, doc, selection, bytes, next_bytes, next: std::ptr::from_ref(next) as usize }
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
        HistoryEntry { label, doc, selection, bytes: costs.next_bytes, next_bytes: costs.bytes, next: Arc::as_ptr(now) as usize }
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
    /// The journal's length when the group began: a cancelled group drops the entries after it.
    journal: usize,
    /// The undo step's name; else the first edit's.
    label: Option<String>,
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
    /// View → Show Transparency Grid, per document (view state: not saved, not undoable).
    pub transparency_grid: bool,
    /// The format Save writes ([`cmd::fileio::SAVE_FORMATS`]): the one the document was opened
    /// from or last saved as.
    pub format: &'static str,
    /// That format's options as last saved (SVG options for SVG, the Save PDF settings for PDF;
    /// empty for native files): Save reuses them and `file.formatOptions` reads them back.
    pub save_options: serde_json::Map<String, Value>,
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
}

static NEXT_DOC_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The rows open in the Layers panel: the layers, sublayers and groups that show what they hold.
/// A document opens with the ones it was saved with ([`Document::layers_open`]), else with only
/// its top-level layers open.
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

impl DocState {
    pub fn new(mut doc: Document, path: Option<String>) -> Self {
        let active_layer = doc.default_layer();
        // The saved view and open Layers rows live here while the document is open (saves write
        // them back).
        let view = doc.last_view.take();
        let saved_open = doc.layers_open.take();
        let layers_open = OpenRows::new(&doc, saved_open);
        let doc = Arc::new(doc);
        Self {
            saved_doc: doc.clone(),
            doc,
            selection: Selection::default(),
            history: History { limit: 500, ..Default::default() },
            path,
            revision: 1,
            active_layer,
            layer_rows: vec![],
            variables_highlight: None,
            isolation: None,
            interaction: None,
            undo_group: None,
            last_transform: None,
            last_selection_cmd: None,
            recalled_selection: RecalledSelection::default(),
            uid: NEXT_DOC_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            mask_view: None,
            transparency_grid: false,
            format: "vectorcraft",
            save_options: Default::default(),
            converted: false,
            view,
            recovered: false,
            recovery: None,
            print_tiling: false,
            layers_open,
            imported_from: None,
            import_losses: vec![],
            last_perspective: None,
        }
    }
    /// Unsaved changes: the document differs from the saved one (selection changes don't count).
    pub fn is_dirty(&self) -> bool {
        !Arc::ptr_eq(&self.doc, &self.saved_doc)
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

/// The error of an edit that would nest objects deeper than [`vectorcraft_doc::MAX_DEPTH`].
pub const TOO_DEEP: &str = "objects can't be nested more than 32 deep";
const _: () = assert!(vectorcraft_doc::MAX_DEPTH == 32, "TOO_DEEP (and its catalog rows) names the cap");

/// Cheap sanity check after an edit: artboards and the objects just touched (the selection) must
/// have finite, in-range geometry, so saved files always reload and renderers never see NaN/∞.
fn doc_sane(d: &Document, sel: &Selection) -> bool {
    let ok = |r: vectorcraft_geom::Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD);
    d.ruler_origin.is_finite()
        && d.ruler_origin.x.abs() <= MAX_COORD
        && d.ruler_origin.y.abs() <= MAX_COORD
        && d.artboards.iter().all(|a| ok(a.rect))
        && d.guides.iter().all(|g| g.pos.is_finite() && g.pos.abs() <= MAX_COORD)
        && {
            let paths = d.index_paths(sel.objects.iter().copied());
            sel.objects.iter().all(|id| paths.get(id).and_then(|p| d.node_at(p)).and_then(|n| n.geometric_bounds()).is_none_or(ok))
        }
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
    pub snapping_tolerance: f64,
    // Slices
    pub show_slice_numbers: bool,
    pub slice_line_color: String,
    // Hyphenation
    pub hyphenation_language: String,
    pub hyphenation_exceptions: String,
    /// Type › Options › Additional Fonts Folder: a folder (read with its subfolders) whose fonts
    /// are listed and used as if installed (#683); empty for none.
    pub fonts_folder: String,
    // Performance & Storage (Plug-ins & Scratch Disks)
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
    /// XWayland on a Wayland session, where dropped files arrive (#1151). Read when the app starts.
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
            highlight_alternate_glyphs: true,
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
            snapping_tolerance: 4.0,
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
    depth: u32,
    /// Set when the active tool panicked (see [`guard`]); reported by the next tool event.
    tool_panic: Option<EngineError>,
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
    /// The Libraries panel's libraries of graphics, colours and text styles (`library.*`).
    pub libraries: cmd::library::Libraries,
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
    /// While `command.batch` runs: the documents its steps closed or replaced (Revert), which an
    /// error brings back; `None` otherwise.
    pub(crate) batch_stash: Option<Vec<DocState>>,
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
    /// A guide being dragged out of a ruler ([`Session::ruler_guide`]).
    pub(crate) ruler_guide: Option<vectorcraft_tools::rulerguide::NewGuide>,
    /// The last search for the files of missing fonts (`text.findFontFiles`), until another
    /// starts; dropping it stops it.
    pub(crate) font_search: Option<cmd::fontfiles::FontSearch>,
    /// The last search for the files of linked images and placed documents (`links.findFiles`),
    /// until another starts; dropping it stops it.
    pub(crate) link_search: Option<cmd::linkfiles::LinkSearch>,
    /// Where folder searches may go; `None`: this computer's rules ([`cmd::findfiles::Rules`]).
    /// Tests set it.
    pub search_rules: Option<cmd::findfiles::Rules>,
    /// The walker threads a folder search starts; `None`: [`cmd::findfiles::threads`]. Tests set
    /// it.
    pub search_threads: Option<usize>,
    /// Each open document's linked files' size and modification time when last seen (linked
    /// files changing while the document is open, [`link_watch`]).
    pub link_stamps: std::collections::HashMap<(u64, String), link_watch::Stamp>,
    /// The stamps being taken on a worker thread ([`Session::start_link_scan`]).
    pub link_scan: Option<link_watch::LinkScan>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        // Placed documents are read by the native format's loader.
        vectorcraft_doc::placed_document::set_loader(cmd::place::document::read_document);
        vectorcraft_doc::placed_document::set_file_reader(cmd::place::document::read_file_again);
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
            depth: 0,
            tool_panic: None,
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
            libraries: Default::default(),
            recent_urls: vec![],
            ui_language: None,
            journal_note: Default::default(),
            note_depth: 1,
            batch_stash: None,
            recovery: Default::default(),
            in_tool_actions: false,
            previewing: false,
            envelope_defaults: None,
            liquify_stroke: None,
            plane_widget_press: false,
            ruler_guide: None,
            font_search: None,
            link_search: None,
            search_rules: None,
            search_threads: None,
            link_stamps: Default::default(),
            link_scan: None,
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
    /// Add a document and make it active.
    pub fn add_document(&mut self, mut doc: Document, path: Option<String>) -> usize {
        // Every edit is checked against the document before it, so the first one is within the cap
        // (importers and the native reader have seen to theirs already, with a warning).
        doc.limit_depth();
        Self::refresh_text_bounds(&mut doc);
        // Guides from files saved before guides had layers go on a layer.
        doc.adopt_guides();
        self.reset_tool_for_doc_switch();
        let mut st = DocState::new(doc, path);
        st.history.limit = self.prefs.history_states as usize;
        self.docs.push(st);
        let i = self.docs.len() - 1;
        self.active = Some(i);
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

    /// Execute a command by id. This is THE entry point for every frontend.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // The crate's own tests are the audit of the params descriptions (#1442): a key a test
        // passes that its command's description doesn't name fails the test.
        #[cfg(test)]
        self.check_params(id, params)?;
        // Clear's menu needs a selection; an agent supplying explicit targets only needs a
        // document. The command validates those targets before making any edit.
        let enabled = if id == "edit.clear" && params.get("ids").is_some() {
            cmd::has_doc(self)
        } else if let Some(ids) = given_ids(spec.params, params) {
            // Objects given as `ids` stand in for the selection: the command is enabled as it is
            // with them selected (#1575).
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
            self.batch_stash = None;
        }
        // A selection-only command given `ids` selects them first, in the same undo step (#2130).
        let select_first = SELECTS_GIVEN_IDS.contains(&id).then(|| params.get("ids")).flatten();
        let run = |s: &mut Session| {
            if let Some(ids) = select_first {
                s.execute("select.set", &serde_json::json!({ "ids": ids }))?;
            }
            (spec.run)(s, params)
        };
        let r = if self.depth == 0 { self.run_guarded(id, run) } else { self.run_nested(run) };
        let r = r?;
        if self.depth == 0 {
            self.inherit_new_art();
        }
        if spec.journal && self.depth == 0 && self.active().is_none_or(|d| d.interaction.is_none()) {
            let p = self.noted(params);
            self.journal.push((id.to_string(), p));
        }
        if self.depth == 0 {
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
        match guard::catch_panic(|| self.run_nested(f)) {
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
                    st.revision += 1;
                }
                if active.is_some_and(|i| i < self.docs.len()) {
                    self.active = active;
                }
                Err(EngineError::Internal { cmd: id.to_string(), msg })
            }
        }
    }

    /// Run `f` on a mutable copy of the active document. Outside an interaction this records one
    /// undo step labelled `label`; inside one it just mutates (the interaction commits later).
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>) -> Result<T> {
        // An edit made while typing (the Transform panel) is a step of its own: inside the typing
        // session, the next keystroke's preview would start again from before it (#1881).
        if !self.previewing && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
        let st = self.doc_mut()?;
        let before = st.doc.clone();
        let before_sel = st.selection.clone();
        let doc = Arc::make_mut(&mut st.doc);
        // Dimensions follow the art they measure.
        let result = match f(doc, &mut st.selection).and_then(|v| {
            cmd::dimension::refresh(&before, doc)?;
            Ok(v)
        }) {
            Ok(v) => match cmd::shaper::refresh(&before, Arc::make_mut(&mut st.doc)) {
                Err(e) => Err(e),
                Ok(()) => {
                    // Inline graphics in text follow their symbols.
                    cmd::inline::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Text Wrap: area type follows its wrap objects; then threads re-flow.
                    cmd::textwrap::refresh(Arc::make_mut(&mut st.doc));
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
                    } else if doc_sane(&st.doc, &st.selection) {
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
                Err(e)
            }
        }
    }

    /// Change only the selection (not an undo step).
    pub fn select(&mut self, f: impl FnOnce(&Document, &mut Selection)) -> Result<()> {
        self.active_appearance_item = None;
        let guides_locked = self.menu.guides_locked;
        let st = self.doc_mut()?;
        let before = st.selection.objects.clone();
        f(&st.doc, &mut st.selection);
        st.selection.prune(&st.doc);
        // Lock Guides locks the paths made into guides too (#1534), whatever selects them.
        if guides_locked {
            let locked: Vec<NodeId> = st.selection.objects.iter().copied().filter(|id| is_guide_path(&st.doc, *id)).collect();
            for id in locked {
                st.selection.remove(id);
            }
        }
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

    pub fn begin_interaction(&mut self, label: &str) -> Result<()> {
        // Typing in progress is kept first: a drag or a dialog's preview would otherwise start
        // from the snapshot taken before the typing, and drop it (#1881).
        if label != "Typing" && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
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
        }
        // The previous preview's paint is undone with it.
        self.pending_paint = None;
        let rw = cmd::distortcmds::perspective_rewrite(self, cmd, params);
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
        self.journal.push(preview);
        Ok(())
    }

    /// Remember the paint a live preview applied (as its interaction is kept).
    pub(crate) fn remember_pending_paint(&mut self) {
        if let Some(p) = self.pending_paint.take() {
            self.remember_paint_now(&p);
        }
    }

    pub fn cancel_interaction(&mut self) -> Result<()> {
        self.pending_paint = None;
        self.doc_mut()?.undo_interaction();
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
        let journal = self.journal.len();
        if let Some(st) = self.active_mut()
            && st.undo_group.is_none()
        {
            st.undo_group = Some(UndoGroup { first: None, journal, label });
        }
    }

    /// Close the open undo groups: their edits stay one undo step or, `cancel`led (Escape), are
    /// undone and dropped from the journal.
    pub fn end_undo_group(&mut self, cancel: bool) {
        let mut journal = None;
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
                st.revision += 1;
                journal = Some(g.journal);
            }
        }
        if let Some(len) = journal {
            self.journal.truncate(len);
        }
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
mod tests_intertwine;
#[cfg(test)]
mod tests_journal;
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
mod tests_slices;
#[cfg(test)]
mod tests_smartguides;
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
mod tests_transparencygrid;
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
    "object.pathfinder.trim",
    "object.pathfinder.unite",
    "object.reflect",
    "object.rotate",
    "object.scale",
    "object.shear",
    "object.ungroup",
];

/// The non-empty `ids` given to a command whose params description names `ids` as a key (the
/// key callers outside the app may pass it, [`cmd::params::takes_key`]).
fn given_ids(description: &'static str, params: &Value) -> Option<Vec<NodeId>> {
    let ids = cmd::ids_param(params, "ids").filter(|ids| !ids.is_empty())?;
    cmd::params::takes_key(description, "ids").then_some(ids)
}
