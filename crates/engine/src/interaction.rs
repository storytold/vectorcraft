use super::{Affine, Arc, Document, NodeId, Selection, Value};

/// An in-progress drag: the snapshot the previews are applied to.
#[derive(Clone, Debug)]
pub struct Interaction {
    pub label: String,
    pub doc: Arc<Document>,
    /// The selection the previews start from (shared, so a preview frame copies nothing).
    pub selection: Arc<Selection>,
    pub preview: Option<(String, Value)>,
    /// The active tool began it (a drag): a command from outside the tool runs as a step of its
    /// own under it ([`crate::Session::outside_interaction`]). A dialog or a panel that begins an
    /// interaction runs its commands inside it.
    pub(crate) tool: bool,
    /// A replaced or canceled preview restores these new-art paints from before the interaction.
    pub(crate) paint: vectorcraft_tools::PaintDefaults,
    /// A preview of the interaction gave `defaultsOnly: true`: every later preview and a cancel put
    /// `paint` back, whichever preview is the latest.
    pub(crate) used_defaults_only: bool,
    /// An accepted batch preview uses these final paints after changing the document color mode.
    pub(crate) preview_paint: Option<vectorcraft_tools::PaintDefaults>,
    /// Conversion of new-art paints waits until a color-mode preview is accepted.
    pub(crate) deferred_color_mode: Option<crate::cmd::colormgmt::DeferredColorMode>,
    pub(crate) color_mode_paints: Option<(vectorcraft_tools::PaintDefaults, vectorcraft_tools::PaintDefaults)>,
    /// Per-document state restored on cancel (current layer, highlighted Layers panel rows,
    /// isolation).
    pub active_layer: Option<NodeId>,
    pub layer_rows: Vec<NodeId>,
    pub variables_highlight: Option<String>,
    pub isolation: Option<NodeId>,
    /// The perspective transform the previews make (`perspective.transform` params): Transform
    /// Again repeats it once the drag is committed.
    pub perspective_again: Option<Value>,
    /// A preview drew an object on, or attached one to, a plane held by Automatic Plane
    /// Positioning: the plane goes back to its place once the drag is committed.
    pub(crate) plane_placed: bool,
    /// A transform or a perspective edit with `copy` keeps the copies it made of the snapshot here
    /// ([`PreviewCopies`]).
    pub(crate) copies: Option<Arc<PreviewCopies>>,
    /// The bounding box of `selection` in `doc`, measured once for a move drag
    /// ([`crate::Session::move_drag_box`]): each step offsets it by the drag's move.
    pub(crate) move_box: std::sync::OnceLock<Option<vectorcraft_doc::OrientedBox>>,
    /// A switch to another document keeps the previews as one undo step in this one, as a panel field's scrub keeps
    /// its edits; otherwise the switch cancels the interaction, as it does a drag.
    pub keep_on_switch: bool,
}

/// An interaction that a command from outside the tool set aside while it runs
/// ([`crate::Session::outside_interaction`]).
#[derive(Debug)]
pub(crate) struct HeldInteraction {
    /// The [`crate::DocState::uid`] of the interaction's document.
    pub(crate) uid: u64,
    pub(crate) interaction: Interaction,
    /// The document and the selection on show when the command began, when a preview had changed
    /// them from the interaction's snapshot.
    pub(crate) shown: Option<(Arc<Document>, Selection)>,
    /// The document's current layer, highlighted Layers panel rows, highlighted variable and
    /// isolation when the command began; a cancel puts back the ones it changed as it left them.
    pub(crate) state: ViewState,
    /// The new-art paints when the command began: a change the command makes to them stays when
    /// the interaction's next preview or a cancel puts back the paints it began with.
    pub(crate) paint: vectorcraft_tools::PaintDefaults,
}

/// The per-document state an interaction puts back when it is canceled, besides the document and
/// the selection.
#[derive(Debug, PartialEq)]
pub(crate) struct ViewState {
    pub(crate) active_layer: Option<NodeId>,
    pub(crate) layer_rows: Vec<NodeId>,
    pub(crate) variables_highlight: Option<String>,
    pub(crate) isolation: Option<NodeId>,
}

impl ViewState {
    pub(crate) fn of(st: &crate::DocState) -> Self {
        ViewState {
            active_layer: st.active_layer,
            layer_rows: st.layer_rows.clone(),
            variables_highlight: st.variables_highlight.clone(),
            isolation: st.isolation,
        }
    }

    /// Give `it` the parts of `now` that differ from `self`, the state before a command.
    pub(crate) fn rebase(&self, now: ViewState, it: &mut Interaction) {
        if now.active_layer != self.active_layer {
            it.active_layer = now.active_layer;
        }
        if now.layer_rows != self.layer_rows {
            it.layer_rows = now.layer_rows;
        }
        if now.variables_highlight != self.variables_highlight {
            it.variables_highlight = now.variables_highlight;
        }
        if now.isolation != self.isolation {
            it.isolation = now.isolation;
        }
    }
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
    /// Each copy is directly below its original, not above it.
    pub(crate) below: bool,
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
    /// The new-art paints from before the group's first command.
    pub(crate) paint: vectorcraft_tools::PaintDefaults,
    /// The document before the group's first edit, as the undo step that edit recorded keeps it.
    pub(crate) first: Option<Arc<Document>>,
    /// The numbers of the journal entries made in the group's document while it was open: a canceled
    /// group drops them. An entry that changed the active document has the index of the document it
    /// left active, and a canceled group turns it into that change.
    pub(crate) journal: Vec<(usize, Option<usize>)>,
    /// For each `command.batch` entry in `journal`, the steps of its `commands` that belong to the
    /// group (by number), each with the index of the document it left active when it changed the
    /// active document: a canceled group drops those steps and keeps the batch's other ones.
    pub(crate) steps: Vec<BatchSteps>,
    /// The undo step's name; else the first edit's.
    pub(crate) label: Option<String>,
}

/// A `command.batch` journal entry's number and the steps of it that belong to an undo group, each
/// with the index of the document it left active when it changed the active document
/// ([`UndoGroup::steps`]).
pub(crate) type BatchSteps = (usize, Vec<(usize, Option<usize>)>);

/// One undo step begun with [`Session::begin_undo_scope`], which [`Session::end_undo_scope`] ends.
#[derive(Clone, Debug)]
pub struct UndoScope {
    /// The document ([`DocState::uid`]).
    pub(crate) uid: u64,
    /// The journal's length when the scope began.
    pub(crate) journal: usize,
    /// When the document's undo group was open already: the document, its selection and the document
    /// that the group's undo step keeps, as they were when the scope began; `None` when the scope
    /// opened the group.
    pub(crate) joined: Option<(Arc<Document>, Selection, Option<Arc<Document>>)>,
    /// The new-art colors and the group's color-mode history metadata at the scope's start.
    /// Only used when cancelling a joined scope that converted those colors.
    pub(crate) paint: vectorcraft_tools::PaintDefaults,
    pub(crate) color_mode_paints: Option<(vectorcraft_tools::PaintDefaults, vectorcraft_tools::PaintDefaults)>,
}
