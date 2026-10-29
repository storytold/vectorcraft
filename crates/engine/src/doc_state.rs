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
    pub(crate) fn push_undo(&mut self, mut label: String, doc: Arc<Document>, selection: Arc<Selection>) {
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
