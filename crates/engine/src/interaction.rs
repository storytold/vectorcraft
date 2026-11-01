use super::{Affine, Arc, Document, NodeId, Selection, Value};

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
}
