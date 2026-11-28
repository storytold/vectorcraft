use super::error::{EngineError, Result};
use super::history::{History, HistoryEntry};
use super::interaction::{Interaction, LastTransform, UndoGroup};
use super::{Arc, Document, NodeId, NodeKind, Selection, Value, cmd, file_access};

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
    pub(crate) saved_doc: Arc<Document>,
    /// The documents that changes that are not edits to save replaced ([`DocState::same_content`]).
    unsaved_changes: SameContent,
    /// The layer new art goes into (the "current layer" in the Layers panel).
    pub active_layer: Option<NodeId>,
    /// The rows highlighted in the Layers panel (layers, sublayers, groups or objects, in the order
    /// they were clicked): what the panel's Duplicate, Delete, Merge, Options… and similar act on.
    /// Panel state, not art selection: not saved, not undoable (`layer.setCurrent`,
    /// `layer.highlight`).
    pub layer_rows: Vec<NodeId>,
    /// What the Layers panel reveals at its next pass, until the panel takes it: the panel opens
    /// the layers around the row and scrolls to it, also when the row was already highlighted.
    /// Set by `layer.locate` and, for what a top-level command, an interaction's commit or a
    /// selection changed, by [`DocState::request_reveal`]. Panel state: not saved, not undoable.
    pub layers_reveal: Option<LayersReveal>,
    /// The variable highlighted in the Variables panel: what its Delete, Options… and Select
    /// Bound Object act on. Panel state, not art selection: not saved, not undoable
    /// (`variable.highlight`). A name, so renaming a variable moves the highlight with it.
    pub variables_highlight: Option<String>,
    /// The order of the Variables panel's list: the column it is sorted by, and whether it is
    /// reversed. Panel state: not saved, not undoable (`variable.sort`).
    pub variables_sort: cmd::VariableSort,
    /// Isolation mode container.
    pub isolation: Option<NodeId>,
    /// Select › Start Global Edit: the objects edited together until Stop Global Edit or Escape
    /// ([`cmd::globaledit`]). Editing state: not saved, not undoable.
    pub global_edit: Option<cmd::globaledit::GlobalEdit>,
    /// The objects Start Global Edit would take, highlighted before it starts
    /// ([`cmd::globaledit::similar`]), as of a revision. Not saved, not undoable.
    pub(crate) similar_outlines: cmd::globaledit::SimilarCache,
    /// Whether Start Global Edit supports the one selected object
    /// ([`cmd::globaledit::SupportCache`]), as of a revision. Not saved, not undoable.
    pub(crate) global_edit_support: cmd::globaledit::SupportCache,
    /// What leaving symbol editing mode goes back to ([`cmd::symboledit`]).
    pub(crate) symbol_edit_return: cmd::symboledit::Return,
    pub interaction: Option<Interaction>,
    /// Edits made while this is open are one undo step (a scrubbed numeric field).
    pub undo_group: Option<UndoGroup>,
    /// The built-in effect the edit in progress applies ([`crate::Session::edit_effect`]): the
    /// undo step it records keeps it ([`DocState::push_undo`]).
    pub(crate) effect_step: Option<&'static str>,
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
    /// When the document last became the current one: a larger value is more recent
    /// ([`Session::front_order`]). It is not saved.
    pub(crate) activated: u64,
    /// How many times File → Revert replaced the document's content in its tab
    /// ([`Session::replace_document`]). Work started for the document (a file read for Place, a
    /// pasted bitmap) is dropped when this has changed by the time it finishes; edits do not
    /// change it, and a batch that fails puts it back with the document.
    pub reverts: u64,
    /// Advances when Undo or Redo restores an older document snapshot. The same node ID can name different objects
    /// across that boundary.
    pub history_restores: u64,
    /// Preferences › User Interface › Open Documents As Tabs when the document opened: cleared,
    /// the UI opens it in a floating window of its own.
    pub opened_as_tab: bool,
    /// View Opacity Mask (Alt-click the mask thumbnail): the masked object whose mask the canvas
    /// shows alone, in greyscale, while that mask is edited (see [`DocState::shown_mask`]).
    pub mask_view: Option<NodeId>,
    /// Check Spelling's Ignore All: the words (as craft-proof keys them, lower case) a spelling
    /// check of this document skips. Kept while the document is open: not saved, not undoable.
    pub spelling_ignored: std::collections::BTreeSet<String>,
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
    /// The id of the view's active artboard, which the UI keeps current before each command it
    /// runs: a `.ai` save or export names that artboard as the file's active one. `None`, or an
    /// artboard the document no longer has: the artboard the document opened on
    /// ([`vectorcraft_doc::Document::opened_artboard`]), else the first.
    pub active_artboard: Option<u32>,
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
    /// The file its links' stored relative paths name places from: the file the document was read
    /// from, also when that file is not its own (such as a `.ai` file another app wrote). A save
    /// writes the saved file's relative paths and leaves the open document's links as they are.
    /// `None`: the document's own file. Not saved ([`crate::cmd::links::LinkBase`]).
    pub links_from: Option<String>,
    /// Transform Again after a perspective move or scale (Perspective Selection tool): the
    /// `perspective.transform` params it repeats. `None` once an ordinary transform follows.
    pub last_perspective: Option<Value>,
    /// The folders a confined (automation) caller that opened or made the document may read: the
    /// paths the document links to are looked at within them later too, when the Links panel
    /// asks about a link outside any caller's scope.
    pub opened_by: Option<Arc<file_access::AutomationRoots>>,
    /// What the commands scripts ran did to the document since the person last opened or saved it:
    /// `None` when they did nothing. The commands a script runs read and write the document's file,
    /// and read the files it links to, only as the person left them ([`DocState::as_person_left`]).
    /// Not saved.
    pub(crate) by_script: Option<ByScript>,
    /// The live-effect plug-in stages deferred while the document is drawn on screen, and the time
    /// budget of its first view ([`vectorcraft_plugins::deferred`]). Not saved.
    pub plugin_effects: vectorcraft_plugins::deferred::Deferrals,
    /// What [`DocState::any_empty_artboards`] last measured. Not saved.
    empty_artboards: std::cell::RefCell<EmptyArtboards>,
    /// What [`DocState::any_art_to_fit`] last measured. Not saved.
    art_to_fit: std::cell::RefCell<Option<ArtToFit>>,
    /// The document as [`crate::Session::replace_document`] left it in a tab in the back, with its
    /// type measured in its own composite fonts, and the lowercased names of those fonts. When the
    /// same document comes to the front, its type set in them is not measured again. While this
    /// reference lasts, its allocation holds no other document. Not saved.
    pub(crate) measured_in_own: Option<(std::sync::Weak<Document>, std::collections::HashSet<String>)>,
    /// The document as the last edit's refresh of inline graphics left it, and what that refresh
    /// measured: the next refresh checks only the layer trees that differ from it
    /// ([`cmd::inline::refresh`]). Not saved.
    pub(crate) inline_resolved: Option<cmd::inline::Resolved>,
    /// The selected objects as [`crate::Session::select`] last checked them or the proxies last
    /// read them, with the document they were checked in ([`SelectionRecord`]). Not saved.
    pub(crate) selection_record: Option<SelectionRecord>,
}

/// A selection of objects alone ([`objects_only`]) with the document it was checked or read in:
/// while the document and the selected objects stay the same, a command that adds objects checks
/// and reads only the added ones.
#[derive(Clone, Debug)]
pub(crate) struct SelectionRecord {
    /// The document, held as [`EmptyArtboards::doc`] holds it.
    pub(crate) doc: std::sync::Weak<Document>,
    pub(crate) objects: Vec<NodeId>,
    /// Lock Guides as it was when [`crate::Session::select`] left these objects selected; `None`
    /// when it did not.
    pub(crate) settled: Option<bool>,
    /// What the proxies read from these objects; `None` when they were not read.
    pub(crate) paints: Option<cmd::SelectionPaints>,
}

/// Whether `sel` holds selected objects and nothing else: no anchors, segments, key, target,
/// slices, ruler guides, object of a symbol instance's art or object on a path.
pub(crate) fn objects_only(sel: &Selection) -> bool {
    sel.anchors.is_empty()
        && sel.segments.is_empty()
        && sel.key.is_none()
        && sel.target.is_none()
        && sel.slices.is_empty()
        && sel.guides.is_empty()
        && sel.key_guide.is_none()
        && sel.key_anchor.is_none()
        && sel.symbol_part.is_none()
        && sel.on_path_member.is_none()
}

/// How `after` starts with objects of `before` kept in their order: → how many objects of `after`
/// do, and the objects of `before` that are not among them, in order.
pub(crate) fn kept_from(before: &[NodeId], after: &[NodeId]) -> (usize, Vec<NodeId>) {
    let (mut at, mut kept, mut removed) = (0, 0, Vec::new());
    for id in after {
        let Some(i) = before[at..].iter().position(|b| b == id) else { break };
        removed.extend_from_slice(&before[at..at + i]);
        at += i + 1;
        kept += 1;
    }
    removed.extend_from_slice(&before[at..]);
    (kept, removed)
}

/// The objects of `ids` that are not in `of`, each named once, in order.
fn left_out(ids: &[NodeId], of: &[NodeId]) -> Vec<NodeId> {
    if ids.is_empty() {
        return vec![];
    }
    let mut out: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
    for id in of {
        out.remove(id);
    }
    let mut seen = std::collections::HashSet::new();
    ids.iter().copied().filter(|id| out.contains(id) && seen.insert(*id)).collect()
}

/// What the commands scripts ran did to an open document since the person last opened or saved it
/// ([`DocState::by_script`]).
#[derive(Clone, Debug)]
pub(crate) enum ByScript {
    /// A command a script ran opened it (or made it).
    Opened,
    /// A command a script ran changed it or gave it another file: the document, its file and the
    /// file it was read from as they were before the first such command.
    Changed { doc: Arc<Document>, path: Option<String>, links_from: Option<String> },
}

/// Whether a document has an artboard for Delete Empty Artboards to delete, with the document that
/// was measured.
#[derive(Clone, Debug, Default)]
struct EmptyArtboards {
    /// The document measured. While this reference lasts, its allocation holds no other document,
    /// and `Arc::make_mut` moves a document it changes to a new allocation.
    doc: std::sync::Weak<Document>,
    any: bool,
}

/// Whether an unlocked artboard of those asked about has art that Fit to Artwork Bounds fits it
/// to, with what was measured: the document (held as [`EmptyArtboards::doc`] holds it), the
/// artboards, and the generations of what the measure reads outside the document. One is kept for
/// each open document, and its weak reference keeps the measured document's allocation (not its
/// contents) until the next measure or until the document closes.
#[derive(Clone, Debug)]
struct ArtToFit {
    doc: std::sync::Weak<Document>,
    indices: Vec<usize>,
    outside: (u64, u64),
    any: bool,
}

#[cfg(test)]
thread_local! {
    /// Tests: how many times [`DocState::any_empty_artboards`] measured a document on this thread.
    pub(crate) static EMPTY_ARTBOARD_MEASURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Tests: how many times [`DocState::any_art_to_fit`] measured a document on this thread.
    pub(crate) static ART_TO_FIT_MEASURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

static NEXT_DOC_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What the Layers panel is asked to reveal ([`DocState::layers_reveal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayersReveal {
    /// Open the layers around this row and scroll to it: it was highlighted, located or isolated.
    Row(NodeId),
    /// The current layer changed with no row highlighted: reveal that layer.
    Current(NodeId),
    /// The last selected object changed: reveal its row when Panel Options › Locate Selected
    /// Objects is on.
    Selected(NodeId),
}

/// What decides a [`LayersReveal`], as of one moment: the isolation, the highlighted rows, the
/// current layer and the last selected object of one document, with the request pending then.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayersView {
    uid: u64,
    isolation: Option<NodeId>,
    rows: Vec<NodeId>,
    active_layer: Option<NodeId>,
    last_selected: Option<NodeId>,
    reveal: Option<LayersReveal>,
}

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
            layers_reveal: None,
            variables_highlight: None,
            variables_sort: Default::default(),
            isolation: None,
            global_edit: None,
            similar_outlines: Default::default(),
            global_edit_support: Default::default(),
            symbol_edit_return: Default::default(),
            interaction: None,
            undo_group: None,
            effect_step: None,
            last_transform: None,
            last_selection_cmd: None,
            recalled_selection: RecalledSelection::default(),
            uid: NEXT_DOC_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            activated: 0,
            reverts: 0,
            history_restores: 0,
            opened_as_tab: true,
            mask_view: None,
            spelling_ignored: Default::default(),
            transparency_grid,
            format: "vectorcraft",
            save_options: Default::default(),
            save_artboards: vec![],
            converted: false,
            view,
            active_artboard: None,
            recovered: false,
            recovery: None,
            print_tiling: false,
            layers_open,
            imported_from: None,
            import_losses: vec![],
            links_from: None,
            last_perspective: None,
            opened_by: file_access::current(),
            by_script: None,
            empty_artboards: Default::default(),
            plugin_effects: Default::default(),
            art_to_fit: Default::default(),
            measured_in_own: None,
            inline_resolved: None,
            selection_record: None,
        }
    }
    /// The [`SelectionRecord`] of the selection now: `None` when the document or the selected
    /// objects changed since it was made, or when anything besides whole objects is selected
    /// ([`objects_only`]). Only a new [`Document`] makes the record stale: the document's new art
    /// is the one state that changes in place and matters to it, and [`crate::Session::select`]
    /// checks the whole selection while there is new art.
    pub(crate) fn selection_record(&self) -> Option<&SelectionRecord> {
        self.selection_record
            .as_ref()
            .filter(|r| std::ptr::eq(r.doc.as_ptr(), Arc::as_ptr(&self.doc)) && objects_only(&self.selection) && r.objects == self.selection.objects)
    }
    /// What the proxies read from the selected objects, when the [`SelectionRecord`] of the
    /// selection now holds it.
    pub(crate) fn selection_paints(&self) -> Option<&cmd::SelectionPaints> {
        self.selection_record().and_then(|r| r.paints.as_ref())
    }
    /// Keep the [`SelectionRecord`] of the selection that [`crate::Session::select`] left, with
    /// Lock Guides `guides_locked`. `record` is the record of the selection before, in the same
    /// document: the objects that leave the selection are taken out of its paints and the objects
    /// that join it are read into them, unless that reads more objects than reading the selection
    /// again would, counting the contents of groups and containers on both sides
    /// ([`cmd::SelectionPaints::reads_no_more_than`]), or the paints left can't be told apart
    /// without reading them again.
    pub(crate) fn record_selection(&mut self, record: Option<SelectionRecord>, guides_locked: bool) {
        if !objects_only(&self.selection) {
            self.selection_record = None;
            return;
        }
        let objects = &self.selection.objects;
        self.selection_record = Some(match record {
            Some(mut r) => {
                if let Some(p) = &mut r.paints {
                    let (kept, removed) = kept_from(&r.objects, objects);
                    let gone = left_out(&removed, objects);
                    let joined = left_out(&objects[kept..], &r.objects);
                    let changed = [gone.as_slice(), joined.as_slice()].concat();
                    if !cmd::SelectionPaints::reads_no_more_than(&self.doc, &changed, objects) || !p.remove(&self.doc, &gone) {
                        r.paints = None;
                    } else {
                        p.add(&self.doc, &joined);
                    }
                }
                r.objects.clone_from(objects);
                r.settled = Some(guides_locked);
                r
            }
            None => SelectionRecord { doc: Arc::downgrade(&self.doc), objects: objects.clone(), settled: Some(guides_locked), paints: None },
        });
    }

    /// The document, its file and the file it was read from as the person last left them, for the
    /// commands scripts run ([`DocState::by_script`]): `None` for a document a script's command
    /// opened.
    pub(crate) fn as_person_left(&self) -> Option<(&Arc<Document>, Option<&str>, Option<&str>)> {
        match &self.by_script {
            None => Some((&self.doc, self.path.as_deref(), self.links_from.as_deref())),
            Some(ByScript::Opened) => None,
            Some(ByScript::Changed { doc, path, links_from }) => Some((doc, path.as_deref(), links_from.as_deref())),
        }
    }

    /// Notes what a command a script ran did to the document, which was `was` (the document, its
    /// file and the file it was read from) before the command ran, or not open when `was` is
    /// `None`. The first change is kept until the person saves the document.
    pub(crate) fn note_script_command(&mut self, was: Option<(&Arc<Document>, &Option<String>, &Option<String>)>) {
        if self.by_script.is_some() {
            return;
        }
        let Some((doc, path, links_from)) = was else {
            self.by_script = Some(ByScript::Opened);
            return;
        };
        if !Arc::ptr_eq(doc, &self.doc) || *path != self.path || *links_from != self.links_from {
            self.by_script = Some(ByScript::Changed { doc: doc.clone(), path: path.clone(), links_from: links_from.clone() });
        }
    }
    /// `measure`'s answer for the document now. `measure` runs once for each version of the document,
    /// so a menu that asks on every frame measures it once, and a change of the selection or the view
    /// keeps the answer.
    pub(crate) fn any_empty_artboards(&self, measure: impl FnOnce(&Document) -> bool) -> bool {
        let mut last = self.empty_artboards.borrow_mut();
        if !std::ptr::eq(last.doc.as_ptr(), Arc::as_ptr(&self.doc)) {
            #[cfg(test)]
            EMPTY_ARTBOARD_MEASURES.with(|c| c.set(c.get() + 1));
            *last = EmptyArtboards { doc: Arc::downgrade(&self.doc), any: measure(&self.doc) };
        }
        last.any
    }
    /// Whether an unlocked artboard of `indices` has art that Fit to Artwork Bounds fits it to. A
    /// menu asks on every frame it is open, and measuring builds the solids of 3D effects, so the
    /// answer is kept for this version of the document and those artboards, and a change of the
    /// selection or the view keeps it. It is measured again when what the measure reads outside the
    /// document changes: the installed plug-ins, whose effects draw art, and the installed fonts and
    /// the hyphenation exceptions, which set the bounds of all type.
    pub fn any_art_to_fit(&self, indices: &[usize]) -> bool {
        self.any_art_to_fit_with(indices, (vectorcraft_plugins::registry::revision(), vectorcraft_text::layout_inputs()))
    }
    /// [`Self::any_art_to_fit`], with `outside` the generations of the plug-ins and the layout inputs
    /// ([`vectorcraft_text::layout_inputs`]).
    pub(crate) fn any_art_to_fit_with(&self, indices: &[usize], outside: (u64, u64)) -> bool {
        // Measuring runs renderer and plug-in code; should it ever ask again while measuring, it
        // is measured without being kept rather than borrowed twice.
        let Ok(mut last) = self.art_to_fit.try_borrow_mut() else {
            let mut r = vectorcraft_render::Renderer::new();
            return indices.iter().filter_map(|i| self.doc.artboards.get(*i)).any(|a| !a.locked && r.art_bounds_on(&self.doc, a.rect).is_some());
        };
        if let Some(k) = last.as_ref()
            && std::ptr::eq(k.doc.as_ptr(), Arc::as_ptr(&self.doc))
            && k.indices == indices
            && k.outside == outside
        {
            return k.any;
        }
        #[cfg(test)]
        ART_TO_FIT_MEASURES.with(|c| c.set(c.get() + 1));
        // One renderer measures every artboard: each object once.
        let mut r = vectorcraft_render::Renderer::new();
        let any = indices.iter().filter_map(|i| self.doc.artboards.get(*i)).any(|a| !a.locked && r.art_bounds_on(&self.doc, a.rect).is_some());
        *last = Some(ArtToFit { doc: Arc::downgrade(&self.doc), indices: indices.to_vec(), outside, any });
        any
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
            let effect = std::mem::replace(&mut self.effect_step, it.effect);
            self.push_undo(it.label, it.doc, selection);
            self.effect_step = effect;
            if let Some(paints) = it.color_mode_paints
                && let Some(e) = self.history.undo.last_mut()
            {
                match &mut e.color_mode_paints {
                    Some((_, after)) => *after = paints.1,
                    slot => *slot = Some(paints),
                }
                e.recost_paints();
                self.history.trim();
            }
            self.revision += 1;
        }
    }
    /// Record an undo step: `doc` and `selection` as they were before an edit (the document now is
    /// the one after it). In an undo group only the group's first edit records one: the edits
    /// after it extend that step. A new step named after the effect in [`DocState::effect_step`]
    /// keeps that effect ([`HistoryEntry::effect`]); a step an undo group names keeps none.
    pub(crate) fn push_undo(&mut self, mut label: String, doc: Arc<Document>, selection: Arc<Selection>) {
        let mut effect = self.effect_step;
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
                effect = None;
            }
        }
        let mut e = HistoryEntry::kept(label, doc, selection, &self.doc);
        e.effect = effect.filter(|id| crate::history::effect_step_label(id) == Some(e.label.as_str()));
        self.history.undo.push(e);
        self.history.clear_redo();
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
    /// End isolation mode when the isolated object is no longer in the document (an edit deleted,
    /// cut, ungrouped or replaced it, or Undo or Redo took it out, such as the clip group Draw
    /// Inside made around it, #3705), rather than naming an id a later object may take. While
    /// opacity mask, pattern or symbol editing is on, isolation goes back to that mode's editing
    /// layer.
    pub(crate) fn leave_isolation_if_gone(&mut self) {
        if self.isolation.is_some_and(|i| self.doc.node(i).is_none()) {
            let d = &self.doc;
            let layer = d.mask_edit.map(|e| e.layer).or(d.pattern_edit.as_ref().map(|e| e.layer)).or(d.symbol_edit.as_ref().map(|e| e.layer));
            self.isolation = layer.filter(|l| d.node(*l).is_some());
            self.revision += 1;
        }
    }
    /// After the document went back from `undone` to a state from when `before` was isolated (a
    /// canceled undo group or undo scope, an interaction's next preview): `before` is isolated
    /// again when the edits taken back had removed it.
    pub(crate) fn restore_isolation(&mut self, before: Option<NodeId>, undone: &Document) {
        if let Some(b) = before
            && self.isolation != before
            && undone.node(b).is_none()
            && self.doc.node(b).is_some()
        {
            self.isolation = before;
            self.revision += 1;
        }
    }
    /// Clear the Variables panel highlight when the document defines no variable of that name
    /// (deleted, or taken out by Undo or Redo), so a variable defined later under that name is
    /// not highlighted without a click.
    pub(crate) fn drop_stale_variables_highlight(&mut self) {
        if self.variables_highlight.as_ref().is_some_and(|h| self.doc.variables.variable(h).is_none()) {
            self.variables_highlight = None;
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
    pub(crate) fn protect_locked_artboards(&mut self, was: &[NodeId], old: &std::collections::HashSet<NodeId>, before: Option<&Document>) {
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
    /// Where new art is inserted: the isolation container (or its parent if the container cannot hold
    /// the art type), else the active layer.
    pub fn insertion_parent(&self) -> Option<NodeId> {
        self.insertion_parent_for(false)
    }

    /// Where new path art is inserted. When a compound path is isolated, returns the compound path
    /// so new paths become members of the compound path.
    pub fn insertion_parent_for_path(&self) -> Option<NodeId> {
        self.insertion_parent_for(true)
    }

    fn insertion_parent_for(&self, for_path: bool) -> Option<NodeId> {
        if let Some(i) = self.isolation {
            if self.doc.node(i).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })) {
                return Some(i);
            }
            if for_path && self.doc.node(i).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })) {
                return Some(i);
            }
        }
        if let Some(p) = self.isolation.filter(|i| self.doc.node(*i).is_some_and(|n| !n.is_container())).and_then(|i| self.doc.parent_of(i)) {
            if self.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })) {
                return Some(p);
            }
            if for_path && self.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })) {
                return Some(p);
            }
        }
        // Non-path art created while a compound path is isolated goes into the compound's parent container:
        if let Some(iso) = self.isolation {
            let compound = if self.doc.node(iso).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })) {
                Some(iso)
            } else {
                self.doc.parent_of(iso).filter(|p| self.doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })))
            };
            if let Some(c) = compound
                && let Some(p) = self.doc.parent_of(c)
                && self.doc.is_editable(p)
            {
                return Some(p);
            }
        }
        // A sublayer takes new art only while it and the layers around it are shown and unlocked.
        self.active_layer.filter(|l| self.doc.node(*l).is_some_and(|n| n.is_layer()) && self.doc.is_editable(*l)).or_else(|| self.doc.default_layer())
    }

    /// [`Self::insertion_parent`] for new art, which a locked or hidden layer never takes: an error
    /// when no layer is shown and unlocked, so the art is refused rather than hidden or locked away.
    pub fn target_parent(&self) -> Result<Option<NodeId>> {
        self.target_parent_for(false)
    }

    /// [`Self::insertion_parent_for_path`] for new path art.
    pub fn target_parent_for_path(&self) -> Result<Option<NodeId>> {
        self.target_parent_for(true)
    }

    fn target_parent_for(&self, for_path: bool) -> Result<Option<NodeId>> {
        match self.insertion_parent_for(for_path) {
            Some(p) if !self.doc.is_editable(p) => Err(EngineError::Other("the target layer is locked or hidden".into())),
            parent => Ok(parent),
        }
    }
    /// The highlighted Layers panel rows that still exist (ids are reused after undo, so a
    /// remembered row must still be in the document).
    pub fn highlighted_rows(&self) -> Vec<NodeId> {
        self.layer_rows.iter().copied().filter(|id| self.doc.node(*id).is_some()).collect()
    }
    /// What decides the Layers panel's reveal request now.
    pub(crate) fn layers_view(&self) -> LayersView {
        LayersView {
            uid: self.uid,
            isolation: self.isolation,
            rows: self.layer_rows.clone(),
            active_layer: self.active_layer,
            last_selected: self.selection.objects.last().copied(),
            reveal: self.layers_reveal,
        }
    }
    /// [`DocState::layers_view`] as it was when interaction `it` began: what a commit of the
    /// interaction changed is compared with that, since the previews changed the state meanwhile.
    pub(crate) fn layers_view_before(&self, it: &Interaction) -> LayersView {
        LayersView {
            uid: self.uid,
            isolation: it.isolation,
            rows: it.layer_rows.clone(),
            active_layer: it.active_layer,
            last_selected: it.selection.objects.last().copied(),
            reveal: self.layers_reveal,
        }
    }
    /// Ask the Layers panel to reveal what changed since `before`, in this order: isolation entered
    /// or left, the highlighted rows, the last selected object, the current layer. A request made
    /// meanwhile (`layer.locate`) stands, and nothing is asked while an interaction is open, so a
    /// drag's previews reveal nothing and its commit does.
    pub(crate) fn request_reveal(&mut self, before: &LayersView) {
        if self.uid != before.uid || self.layers_reveal != before.reveal || self.interaction.is_some() {
            return;
        }
        let now = self.layers_view();
        let mut request = None;
        if now.isolation != before.isolation {
            request = self.reveal_isolation(now.isolation, before.isolation);
        }
        if request.is_none() && now.rows != before.rows {
            request = self.highlighted_rows().first().copied().map(LayersReveal::Row);
        }
        // A new selection asks for its object even when it changed the current layer too: the panel
        // falls back to the current layer when it does not locate the object.
        if request.is_none() && now.last_selected != before.last_selected {
            request = now.last_selected.map(LayersReveal::Selected);
            // An emptied selection leaves no object to locate.
            if request.is_none() && matches!(self.layers_reveal, Some(LayersReveal::Selected(_))) {
                self.layers_reveal = None;
            }
        }
        if request.is_none() && now.active_layer != before.active_layer {
            request = self.current_layer().map(LayersReveal::Current);
        }
        if request.is_some() {
            self.layers_reveal = request;
        }
    }
    /// The request for isolation moving from `before` to `now`: entering opens the isolated row,
    /// leaving opens the layers around the row that was isolated.
    fn reveal_isolation(&mut self, now: Option<NodeId>, before: Option<NodeId>) -> Option<LayersReveal> {
        if let Some(root) = now.filter(|r| self.doc.node(*r).is_some()) {
            self.layers_open.set(root, true);
            return Some(LayersReveal::Row(root));
        }
        let former = before?;
        for around in self.doc.ancestry(former)?.into_iter().filter(|a| *a != former) {
            self.layers_open.set(around, true);
        }
        Some(LayersReveal::Row(former))
    }
    /// The object whose opacity mask View Opacity Mask shows ([`DocState::mask_view`]): only while
    /// its mask is being edited, so leaving editing by any route (undo, deleting the object) ends it.
    pub fn shown_mask(&self) -> Option<NodeId> {
        self.mask_view.filter(|id| self.doc.mask_edit.is_some_and(|m| m.object == *id))
    }
}
