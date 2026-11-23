//! The script object model: the global `app`, the open documents, their selection and the art in
//! them, as the help pages <https://ai-scripting.docsforadobe.dev/jsobjref/Application/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Documents/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Document/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PageItems/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PageItem/> describe them.
//!
//! The objects are host classes ([`vectorcraft_js::HostClass`]): each holds the run's number, the
//! document's [`crate::DocState::uid`] and, for art, the object's id, and every read looks the
//! document and the object up again. An object that was deleted, a document that was closed and an
//! object from an earlier run of a named engine throw an error on every member. Every change runs
//! a command, as `vc.run` does, so it joins the run's transaction and its one undo step for each
//! changed document; a change that changes nothing runs no command. A change to a document that is
//! not the active one makes it active for the command and makes the other one active again after.
//!
//! Collections are counted from 0, and `pageItems` lists art topmost first, as
//! <https://ai-scripting.docsforadobe.dev/jsobjref/CMYKColor/> reads the frontmost path as
//! `pathItems[0]`. A document's selection lists the selected objects in the same order, not in the
//! order they were selected (<https://ai-scripting.docsforadobe.dev/scriptingJavascript/objectReferences/>).
//! A collection read with a name, `pageItems["name"]`, gives the object `getByName` gives, as that
//! page describes, and throws `Not found` when none has the name; `"name" in pageItems` is false then,
//! and assigning to such a name throws the same error. As that page also describes, a script may name
//! `activeDocument`, `documents` and `selection` without `app.`; assigning one of those names makes it
//! a variable of the script's own.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - Collections list documents in tab order. `pageItems` lists every object, with the objects
//!   inside groups and compound paths after the group or compound path, and leaves out layers and
//!   ruler guides. An index out of range gives `undefined`; `index` and `getByName` throw
//!   `Not found` when nothing matches.
//! - Script coordinates are points with Y up from the bottom of the first artboard
//!   (`DOCUMENTCOORDINATESYSTEM`, the default) or points with Y down from the top-left corner of
//!   the active artboard (`ARTBOARDCOORDINATESYSTEM`), as `app.coordinateSystem` says for the run.
//! - Enumerated values are strings such as `"ZOrderMethod.BRINGTOFRONT"`.
//! - `app.activeDocument` with no document open throws `No active document`.
//! - Assigning `Document.selection` takes one object, a list of them, `null` or `[]`; every object
//!   must be one of that document's that can be selected, or nothing changes.
//! - Members the help pages list that are not here yet throw an error that says so.
//! - Assigning `opacity` to an object in a hidden or locked layer throws `Cannot edit` and changes
//!   nothing. An object that is hidden or locked itself, or is in a hidden or locked group, takes it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostItem, HostMember, Realm, RunError, host_ref, host_ref_parts};

use super::{HISTORY_LOST, Host, Link, Savepoint, command, command_value, current, data_size};
use crate::{DocState, Document, NodeId, NodeKind, Session};

/// The error for a member read or called with no document open.
pub const NO_ACTIVE_DOCUMENT: &str = "No active document";
/// The error for an object that was deleted, or that came from an earlier run.
pub const STALE_ITEM: &str = "Stale page item";
/// The error for a document that was closed, or that came from an earlier run.
pub const STALE_DOCUMENT: &str = "Stale document";
/// The error of `index` and `getByName` when nothing matches.
pub const NOT_FOUND: &str = "Not found";
/// The most objects one read of a selection gives, and the most a selection assignment takes.
pub const MAX_LISTED: usize = 10_000;
/// The longest name an object or a path query may have, in bytes.
pub const MAX_TEXT_BYTES: usize = 4096;
/// The most artboards `Documents.add` makes.
pub const MAX_ARTBOARDS: f64 = 100.0;
/// The largest width or height `Documents.add` takes, in points, as `file.new` takes them.
pub const MAX_SIZE: f64 = 3_999_856.0;

/// The page item classes, with the typename of each kind of object ([`typename`]).
const ITEM_CLASSES: &[&str] = &[
    "PathItem",
    "CompoundPathItem",
    "GroupItem",
    "GraphItem",
    "TextFrame",
    "PlacedItem",
    "RasterItem",
    "SymbolItem",
    "MeshItem",
    "BlendItem",
    "EnvelopeItem",
    "RepeatItem",
    "PlacedDocumentItem",
    "IntertwineItem",
];

/// `app.coordinateSystem`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Coords {
    #[default]
    Document,
    Artboard,
}

const DOCUMENT_COORDS: &str = "CoordinateSystem.DOCUMENTCOORDINATESYSTEM";
const ARTBOARD_COORDS: &str = "CoordinateSystem.ARTBOARDCOORDINATESYSTEM";

/// What the object model keeps for one run: its coordinate system, the lists of page items and the
/// answers of which objects are on a locked artboard for the last [`ITEM_LISTS`] documents read,
/// each kept while its document is unchanged, whether a call's own savepoint ([`atomically`]) is
/// open, and the settings of each `DocumentPreset` the run made.
#[derive(Default)]
pub(super) struct State {
    coords: Cell<Coords>,
    items: RefCell<Vec<ItemList>>,
    locked: RefCell<Vec<LockedList>>,
    saving: Cell<bool>,
    presets: RefCell<Vec<Vec<(String, Data)>>>,
}

impl State {
    /// Whether a call's own savepoint is open: its commands take none of their own.
    pub(super) fn saving(&self) -> bool {
        self.saving.get()
    }
}

/// Runs `f`, whose commands are kept together or put back together: when `f` fails, every
/// document, the selection and the undo histories are as they were before it, with no undo step
/// and no change to the document's saved state.
fn atomically<T>(h: &Host, f: impl FnOnce(&Host) -> std::result::Result<T, HostError>) -> std::result::Result<T, HostError> {
    if h.dom.saving.get() {
        return f(h);
    }
    let save = Savepoint::take(&mut h.session.borrow_mut(), "edit.duplicate");
    h.dom.saving.set(true);
    let r = f(h);
    h.dom.saving.set(false);
    let mut s = h.session.borrow_mut();
    if r.is_ok() {
        save.keep(&mut s);
    } else if !save.restore(&mut s) {
        return Err(HostError::Abort(HISTORY_LOST.to_string()));
    }
    r
}

/// A document's uid, the document as it was, and the ids of its page items then.
type ItemList = (u64, Arc<Document>, Rc<Vec<NodeId>>);

/// A document's uid, the document and its new art ([`Document::new_art`]) as they were, and for
/// each object of a layer or sublayer asked about then, whether it was on a locked artboard.
type LockedList = (u64, Arc<Document>, Arc<HashSet<NodeId>>, HashMap<NodeId, bool>);

/// How many documents' lists [`State`] keeps at once, of each kind.
const ITEM_LISTS: usize = 4;

/// The kept list of the document `uid` as `doc` is, if there is one.
fn kept_items(lists: &[ItemList], uid: u64, doc: &Arc<Document>) -> Option<Rc<Vec<NodeId>>> {
    lists.iter().find(|(u, d, _)| *u == uid && Arc::ptr_eq(d, doc)).map(|(_, _, ids)| ids.clone())
}

/// Keeps `list` in place of any earlier list of the same document, dropping the least recently
/// kept list beyond [`ITEM_LISTS`].
fn keep_items(lists: &mut Vec<ItemList>, list: ItemList) {
    lists.retain(|(u, _, _)| *u != list.0);
    if lists.len() >= ITEM_LISTS {
        lists.remove(0);
    }
    lists.push(list);
}

/// Whether node `id` of `st` is on a locked artboard ([`Document::on_locked_artboard`]). Each
/// object of a layer or sublayer is measured once while the document and its new art stay as they
/// are, so the members of a group are answered with one measurement of the group.
fn on_locked_artboard(h: &Host, st: &DocState, id: NodeId) -> bool {
    if !st.doc.artboards.iter().any(|a| a.locked) {
        return false;
    }
    let Some(object) = st.doc.layer_object(id).map(|n| n.id) else {
        return false;
    };
    let new_art = st.doc.new_art.get();
    let mut lists = h.dom.locked.borrow_mut();
    let at = match lists.iter().position(|(u, d, n, _)| *u == st.uid && Arc::ptr_eq(d, &st.doc) && Arc::ptr_eq(n, &new_art)) {
        Some(at) => at,
        None => {
            lists.retain(|(u, ..)| *u != st.uid);
            if lists.len() >= ITEM_LISTS {
                lists.remove(0);
            }
            lists.push((st.uid, st.doc.clone(), new_art, HashMap::new()));
            lists.len() - 1
        }
    };
    match lists.get_mut(at) {
        Some((.., held)) => *held.entry(object).or_insert_with(|| st.doc.on_locked_artboard(object)),
        None => st.doc.on_locked_artboard(object),
    }
}

type Answer = std::result::Result<Data, HostError>;

/// A member function: the run's host and the call's arguments, `this` first.
type Member = fn(&Host, &[Data]) -> Answer;

fn function(link: &Link, f: Member) -> HostFunction {
    let l = link.clone();
    Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16 + a.iter().map(data_size).sum::<u64>())?;
        f(&h, &a)
    })
}

fn get(link: &Link, f: Member) -> HostMember {
    HostMember::Get(function(link, f))
}

fn property(link: &Link, g: Member, s: Member) -> HostMember {
    HostMember::Property { get: function(link, g), set: function(link, s) }
}

fn method(link: &Link, f: Member) -> HostMember {
    HostMember::Method(function(link, f))
}

/// A member the help pages list that is not available yet: reading or assigning it throws, after
/// the check that a document or page item `this` is still there. `hasOwnProperty` is false for it,
/// so a script that checks for the member first skips it.
fn later(link: &Link, class: &'static str, name: &'static str) -> (String, HostMember) {
    let message = format!("{class}.{name} is not available yet");
    let l = link.clone();
    let f: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16)?;
        match class {
            "Document" => {
                doc_of(&h, arg(&a, 0), "Document")?;
            }
            "PageItem" => {
                item_of(&h, arg(&a, 0))?;
            }
            _ => {}
        }
        Err(HostError::Error(message.clone()))
    });
    (name.to_string(), HostMember::Unavailable(f))
}

fn text(s: impl Into<String>) -> Data {
    Data::String(s.into())
}

fn num(n: f64) -> Data {
    Data::Number(n)
}

fn pair(p: [f64; 2]) -> Data {
    Data::Array(vec![num(p[0]), num(p[1])])
}

fn arg(a: &[Data], i: usize) -> &Data {
    a.get(i).unwrap_or(&Data::Undefined)
}

/// A whole number from a handle or an argument, within 0 and 2^53.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole(n: f64) -> Option<u64> {
    (n.is_finite() && n.fract() == 0.0 && (0.0..=9_007_199_254_740_992.0).contains(&n)).then_some(n as u64)
}

#[allow(clippy::cast_precision_loss)]
fn float(n: u64) -> f64 {
    n as f64
}

/// A node id in a handle: its high and low 32 bits, each exact as a number.
fn id_parts(id: NodeId) -> [f64; 2] {
    [float(id.0 >> 32), float(id.0 & 0xffff_ffff)]
}

/// The node id [`id_parts`] gave as `high` and `low`.
fn id_of(high: f64, low: f64) -> Option<NodeId> {
    let (high, low) = (whole(high)?, whole(low)?);
    (high <= 0xffff_ffff && low <= 0xffff_ffff).then_some(NodeId((high << 32) | low))
}

/// A finite number argument, or a `TypeError` naming `what`.
fn number(d: &Data, what: &str) -> std::result::Result<f64, HostError> {
    match d {
        Data::Number(n) if n.is_finite() => Ok(*n),
        _ => Err(HostError::Type(format!("{what} must be a finite number"))),
    }
}

/// A coordinate argument: a finite number within the canvas.
fn coordinate(d: &Data, what: &str) -> std::result::Result<f64, HostError> {
    let n = number(d, what)?;
    if n.abs() > crate::MAX_COORD * 2.0 {
        return Err(HostError::Range(format!("{what} is outside the canvas")));
    }
    Ok(n)
}

/// A point argument `[x, y]`.
fn point(d: &Data, what: &str) -> std::result::Result<[f64; 2], HostError> {
    match d {
        Data::Array(a) if a.len() == 2 => Ok([coordinate(&a[0], what)?, coordinate(&a[1], what)?]),
        _ => Err(HostError::Type(format!("{what} must be an array of two numbers"))),
    }
}

fn boolean(d: &Data, what: &str) -> std::result::Result<bool, HostError> {
    match d {
        Data::Bool(b) => Ok(*b),
        _ => Err(HostError::Type(format!("{what} must be true or false"))),
    }
}

/// The handle of `d` when it is an object of `class` (or of a page item class, for `"PageItem"`).
fn handle_of(d: &Data, class: &str) -> Option<Vec<f64>> {
    let (c, handle) = host_ref_parts(d)?;
    let matches = if class == "PageItem" { ITEM_CLASSES.contains(&c) } else { c == class };
    matches.then_some(handle)
}

/// A document an object of the model names: its index among the open documents and its uid.
#[derive(Clone, Copy)]
struct Doc {
    index: usize,
    uid: u64,
}

/// A page item: its document and its id.
#[derive(Clone, Copy)]
struct Item {
    doc: Doc,
    id: NodeId,
}

/// The open document with `uid`, when the run's number in a handle is this run's.
fn find_doc(h: &Host, run: f64, uid: f64) -> Option<Doc> {
    if whole(run) != Some(h.run) {
        return None;
    }
    let uid = whole(uid)?;
    let s = h.session.borrow();
    s.documents().iter().position(|d| d.uid == uid).map(|index| Doc { index, uid })
}

/// The document `d` names: a `Document` or a `PageItems` object.
fn doc_of(h: &Host, d: &Data, class: &str) -> std::result::Result<Doc, HostError> {
    let Some(handle) = handle_of(d, class) else { return Err(HostError::Type(format!("this is not a {class}"))) };
    match handle.as_slice() {
        [run, uid] => find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into())),
        _ => Err(HostError::Type(format!("this is not a {class}"))),
    }
}

/// The page item `d` names, which must still be in its document.
fn item_of(h: &Host, d: &Data) -> std::result::Result<Item, HostError> {
    let Some(handle) = handle_of(d, "PageItem") else { return Err(HostError::Type("this is not a page item".into())) };
    let [run, uid, high, low] = handle.as_slice() else { return Err(HostError::Type("this is not a page item".into())) };
    let doc = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let id = id_of(*high, *low).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let s = h.session.borrow();
    let st = &s.documents()[doc.index];
    if st.doc.node(id).is_none_or(|n| typename(n).is_none()) {
        return Err(HostError::Error(STALE_ITEM.into()));
    }
    Ok(Item { doc, id })
}

/// The typename of a node, or `None` for one that is not a page item (a layer or a ruler guide).
fn typename(n: &crate::doc::Node) -> Option<&'static str> {
    Some(match &n.kind {
        NodeKind::Layer { .. } | NodeKind::RulerGuide { .. } => return None,
        NodeKind::Group { .. } if n.extras().graph.is_some() => "GraphItem",
        NodeKind::Group { .. } => "GroupItem",
        NodeKind::Path { .. } => "PathItem",
        NodeKind::Compound { .. } => "CompoundPathItem",
        NodeKind::Text(_) => "TextFrame",
        NodeKind::Image(image) if image.link.is_some() => "PlacedItem",
        NodeKind::Image(_) => "RasterItem",
        NodeKind::SymbolInstance { .. } => "SymbolItem",
        NodeKind::Mesh(_) => "MeshItem",
        NodeKind::Blend { .. } => "BlendItem",
        NodeKind::Envelope { .. } => "EnvelopeItem",
        NodeKind::Repeat(_) => "RepeatItem",
        NodeKind::PlacedDocument(_) => "PlacedDocumentItem",
        NodeKind::Intertwine { .. } => "IntertwineItem",
    })
}

fn doc_ref(h: &Host, uid: u64) -> Data {
    host_ref("Document", &[float(h.run), float(uid)])
}

fn item_ref(h: &Host, st: &DocState, id: NodeId) -> Data {
    match st.doc.node(id) {
        Some(n) => match typename(n) {
            Some(t) => host_ref(t, &[float(h.run), float(st.uid), id_parts(id)[0], id_parts(id)[1]]),
            None => host_ref("Layer", &[float(h.run), float(st.uid), id_parts(id)[0], id_parts(id)[1]]),
        },
        None => Data::Null,
    }
}

/// The ids of a document's page items, topmost first, each group or compound path before the
/// objects in it ([`State`] keeps the lists of the last documents read).
fn page_items(h: &Host, st: &DocState) -> Rc<Vec<NodeId>> {
    if let Some(ids) = kept_items(&h.dom.items.borrow(), st.uid, &st.doc) {
        return ids;
    }
    fn walk(nodes: &[Arc<crate::doc::Node>], out: &mut Vec<NodeId>) {
        for n in nodes.iter().rev() {
            match &n.kind {
                NodeKind::Layer { children, .. } => walk(children, out),
                NodeKind::RulerGuide { .. } => {}
                _ => {
                    out.push(n.id);
                    if let Some(children) = n.children() {
                        walk(children, out);
                    }
                }
            }
        }
    }
    let mut ids = Vec::new();
    walk(&st.doc.layers, &mut ids);
    let ids = Rc::new(ids);
    keep_items(&mut h.dom.items.borrow_mut(), (st.uid, st.doc.clone(), ids.clone()));
    ids
}

/// The objects directly in the layers and sublayers of a document: what deleting every page item
/// deletes.
fn top_items(d: &Document) -> Vec<NodeId> {
    fn walk(nodes: &[Arc<crate::doc::Node>], out: &mut Vec<NodeId>) {
        for n in nodes {
            match &n.kind {
                NodeKind::Layer { children, .. } => walk(children, out),
                NodeKind::RulerGuide { .. } => {}
                _ => out.push(n.id),
            }
        }
    }
    let mut out = Vec::new();
    walk(&d.layers, &mut out);
    out
}

/// Whether `n` and every object inside it are shown and unlocked.
fn all_editable(n: &crate::doc::Node) -> bool {
    n.visible && !n.locked && n.children().is_none_or(|c| c.iter().all(|c| all_editable(c)))
}

/// Whether the object and every layer and group it is in are shown and unlocked.
fn editable(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).is_some_and(|a| a.iter().chain(std::iter::once(&id)).all(|i| d.node(*i).is_some_and(|n| n.visible && !n.locked)))
}

/// The two rectangles script coordinates are measured from.
struct Frame {
    /// The first artboard's left edge and bottom edge.
    left0: f64,
    bottom0: f64,
    /// The active artboard's left edge and top edge.
    left: f64,
    top: f64,
}

/// The index of the active artboard of `st`, as the view keeps it.
fn active_artboard(st: &DocState) -> usize {
    let index_of = |id: u32| st.doc.artboards.iter().position(|a| a.id == id);
    st.active_artboard.and_then(index_of).or_else(|| st.doc.opened_artboard.and_then(index_of)).unwrap_or(0)
}

fn frame(st: &DocState) -> std::result::Result<Frame, HostError> {
    let (Some(first), Some(active)) = (st.doc.artboards.first(), st.doc.artboards.get(active_artboard(st))) else {
        return Err(HostError::Error("the document has no artboards".into()));
    };
    Ok(Frame { left0: first.rect.x0, bottom0: first.rect.y1, left: active.rect.x0, top: active.rect.y0 })
}

impl Frame {
    /// A point in document coordinates as the script sees it in `c`.
    fn to_script(&self, c: Coords, p: Point) -> [f64; 2] {
        match c {
            Coords::Document => [p.x - self.left0, self.bottom0 - p.y],
            Coords::Artboard => [p.x - self.left, p.y - self.top],
        }
    }

    /// A point the script gave in `c`, in document coordinates.
    fn to_document(&self, c: Coords, p: [f64; 2]) -> Point {
        match c {
            Coords::Document => Point::new(p[0] + self.left0, self.bottom0 - p[1]),
            Coords::Artboard => Point::new(p[0] + self.left, p[1] + self.top),
        }
    }

    /// `[left, top, right, bottom]` of `r` as the script sees it.
    fn bounds(&self, c: Coords, r: Rect) -> Data {
        let a = self.to_script(c, Point::new(r.x0, r.y0));
        let b = self.to_script(c, Point::new(r.x1, r.y1));
        Data::Array(vec![num(a[0]), num(a[1]), num(b[0]), num(b[1])])
    }
}

fn coords_of(d: &Data) -> std::result::Result<Coords, HostError> {
    match d {
        Data::String(s) if s == DOCUMENT_COORDS => Ok(Coords::Document),
        Data::String(s) if s == ARTBOARD_COORDS => Ok(Coords::Artboard),
        _ => Err(HostError::Type("give a CoordinateSystem value".into())),
    }
}

/// Selects `before` again in the active document `doc` with the selection commands, so that the
/// journal holds the change: a Layers panel target, a dynamic symbol instance's object or an Objects
/// on Path group's object with its own command, else the objects in their order with their
/// direct-selected anchors, then the segments, the ruler guides and the key object, guide or anchor.
/// Objects that are now deleted, hidden or locked are left out.
fn restore_selection(h: &Host, doc: &Document, before: &crate::doc::Selection) -> std::result::Result<(), HostError> {
    // Slices selected with the Slice Selection tool are the whole selection when there are any.
    let slices: Vec<u64> = before.slices.iter().filter(|i| doc.is_slice(**i)).map(|i| i.0).collect();
    if !slices.is_empty() {
        command(h, "object.slice.select", &json!({"slices": slices}))?;
        return Ok(());
    }
    let keep = |id: &NodeId| editable(doc, *id);
    // Each of these commands sets the whole selection. One that refuses (its object is no longer
    // inside the isolated group, or a symbol's object is now hidden) changes nothing, and the
    // objects are selected as below.
    if let Some(t) = before.target.filter(keep)
        && done_unless_refused(command(h, "layer.target", &json!({"id": t.0})))?
    {
        return Ok(());
    }
    if let Some(p) = before.symbol_part.as_ref().filter(|p| keep(&p.instance))
        && done_unless_refused(command(h, "symbol.instance.selectPart", &json!({"id": p.instance.0, "path": p.path})))?
    {
        return Ok(());
    }
    if let Some(m) = before.on_path_member.filter(keep)
        && done_unless_refused(command(h, "object.objectsOnPath.selectMember", &json!({"member": m.0})))?
    {
        return Ok(());
    }
    let partial = |id: &NodeId| before.anchors.contains_key(id) || before.segments.contains_key(id);
    let kept: Vec<NodeId> = before.objects.iter().copied().filter(keep).collect();
    if kept.is_empty() {
        command(h, "select.none", &json!({}))?;
    } else if !kept.iter().any(partial) {
        command(h, "select.set", &json!({"ids": kept.iter().map(|i| i.0).collect::<Vec<_>>()}))?;
    } else {
        // One `select.anchorsMany` selects every object in its order: a partly selected path with
        // its anchors, a whole path with all of them and any other object with none.
        let items: Vec<Value> = kept
            .iter()
            .map(|i| {
                let anchors: Vec<[usize; 2]> = match before.anchors.get(i) {
                    Some(a) => a.iter().map(|(p, k)| [*p, *k]).collect(),
                    None => doc.node(*i).and_then(|n| n.path_data()).map(|p| p.anchors().map(|(s, k, _)| [s, k]).collect()).unwrap_or_default(),
                };
                json!({"id": i.0, "anchors": anchors})
            })
            .collect();
        command(h, "select.anchorsMany", &json!({"items": items, "mode": "set"}))?;
    }
    for (i, segments) in before.segments.iter().filter(|(i, _)| keep(i)) {
        let segments: Vec<[usize; 2]> = segments.iter().map(|(p, k)| [*p, *k]).collect();
        command(h, "select.segments", &json!({"id": i.0, "segments": segments, "mode": "add"}))?;
    }
    let guides: Vec<NodeId> = doc.ruler_guides().into_iter().filter_map(|g| g.id).collect();
    let index = |id: &NodeId| guides.iter().position(|g| g == id);
    let picked: Vec<usize> = before.guides.iter().filter_map(index).collect();
    // Guides that are locked now, and a key that no longer qualifies (the objects beside it are
    // gone), stay unselected: those commands refuse them and change nothing.
    if !picked.is_empty() {
        done_unless_refused(command(h, "guide.select", &json!({"indexes": picked, "toggle": true})))?;
    }
    let key = if let Some((id, (p, k))) = before.key_anchor.filter(|(id, _)| keep(id)) {
        Some(json!({"id": id.0, "anchor": [p, k]}))
    } else if let Some(id) = before.key.filter(keep) {
        Some(json!({"id": id.0}))
    } else {
        before.key_guide.as_ref().and_then(index).map(|g| json!({"guide": g}))
    };
    if let Some(key) = key {
        done_unless_refused(command(h, "select.key", &key))?;
    }
    Ok(())
}

/// The result `r` of a selection command that [`restore_selection`] runs: `true` when it ran and
/// `false` when it refused (an `Error`, which changes nothing). Any other error, such as an
/// internal failure or a request to stop, is returned, so the run ends with it.
fn done_unless_refused(r: std::result::Result<Data, HostError>) -> std::result::Result<bool, HostError> {
    match r {
        Ok(_) => Ok(true),
        Err(HostError::Error(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Makes `layer` the current layer of the active document `uid` again, with `rows` the highlighted
/// Layers panel rows, after selecting objects made the layer of the last of them current and
/// cleared the rows. Nothing runs when `layer` is gone. `layer.highlight` puts the rows back when
/// they changed, and then `layer.setCurrent` with `keepRows` makes `layer` current when it is not.
fn restore_layer(h: &Host, uid: u64, layer: Option<NodeId>, rows: &[NodeId]) -> std::result::Result<(), HostError> {
    let Some(layer) = layer else { return Ok(()) };
    let rows = {
        let s = h.session.borrow();
        let Some(st) = s.active().filter(|st| st.uid == uid) else { return Ok(()) };
        if st.doc.node(layer).is_none() {
            return Ok(());
        }
        let rows: Vec<NodeId> = rows.iter().copied().filter(|r| st.doc.node(*r).is_some()).collect();
        (st.highlighted_rows() != rows).then(|| rows.iter().map(|r| r.0).collect::<Vec<_>>())
    };
    if let Some(rows) = rows {
        command(h, "layer.highlight", &json!({"ids": rows, "mode": "set"}))?;
    }
    let current = h.session.borrow().active().is_some_and(|st| st.active_layer == Some(layer));
    if !current {
        command(h, "layer.setCurrent", &json!({"id": layer.0, "keepRows": true}))?;
    }
    Ok(())
}

/// Runs `f` with the document `uid` active, and makes the document that was active before active
/// again after. With `keep_selection`, the document's selection is put back as it was when `f`'s
/// commands changed it (commands given ids select them first), direct-selected anchors, segments,
/// ruler guides, slices, the key, a Layers panel target and an object chosen inside a dynamic
/// symbol instance or an Objects on Path group included, less the objects that are now deleted,
/// hidden or locked.
fn on_doc<T>(
    h: &Host,
    uid: u64,
    keep_selection: bool,
    f: impl FnOnce(&Host) -> std::result::Result<T, HostError>,
) -> std::result::Result<T, HostError> {
    let (index, active, before, layer) = {
        let s = h.session.borrow();
        let index = s.documents().iter().position(|d| d.uid == uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
        let st = &s.documents()[index];
        (index, s.active_index(), st.selection.clone(), (st.active_layer, st.highlighted_rows()))
    };
    let switch = active != Some(index);
    if switch {
        command(h, "document.activate", &json!({"index": index}))?;
    }
    let r = f(h);
    let mut restored = Ok(());
    if keep_selection {
        let doc = {
            let s = h.session.borrow();
            s.active().filter(|st| st.uid == uid && st.selection != before).map(|st| st.doc.clone())
        };
        if let Some(doc) = doc {
            restored = restore_selection(h, &doc, &before);
        }
        if restored.is_ok() {
            restored = restore_layer(h, uid, layer.0, &layer.1);
        }
    }
    // The document that was active is active again before any error is given: `f`'s error first,
    // then the selection's, then the switch back's.
    let back = match active {
        Some(a) if switch => command(h, "document.activate", &json!({"index": a})).map(drop),
        _ => Ok(()),
    };
    let v = r?;
    restored?;
    back?;
    Ok(v)
}

/// Runs `id` with `params` on the item's document, its selection kept.
fn edit_item(h: &Host, item: Item, id: &str, params: &Value) -> std::result::Result<Data, HostError> {
    on_doc(h, item.doc.uid, true, |h| command(h, id, params))
}

/// Reads the document of `doc` with `f`.
fn with_doc<T>(h: &Host, doc: Doc, f: impl FnOnce(&Session, &DocState) -> T) -> T {
    let s = h.session.borrow();
    f(&s, &s.documents()[doc.index])
}

// ---- Application ----

fn app_documents(_: &Host, _: &[Data]) -> Answer {
    Ok(host_ref("Documents", &[]))
}

fn app_active(h: &Host, _: &[Data]) -> Answer {
    let s = h.session.borrow();
    let st = s.active().ok_or_else(|| HostError::Error(NO_ACTIVE_DOCUMENT.into()))?;
    Ok(doc_ref(h, st.uid))
}

fn app_set_active(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 1), "Document")?;
    if h.session.borrow().active_index() != Some(doc.index) {
        command(h, "document.activate", &json!({"index": doc.index}))?;
    }
    Ok(Data::Undefined)
}

fn app_selection(h: &Host, _: &[Data]) -> Answer {
    let uid = h.session.borrow().active().map(|st| st.uid);
    match uid {
        Some(uid) => selection_of(h, uid),
        None => Ok(Data::Null),
    }
}

fn app_coords(h: &Host, _: &[Data]) -> Answer {
    Ok(text(if h.dom.coords.get() == Coords::Document { DOCUMENT_COORDS } else { ARTBOARD_COORDS }))
}

fn app_set_coords(h: &Host, a: &[Data]) -> Answer {
    h.dom.coords.set(coords_of(arg(a, 1))?);
    Ok(Data::Undefined)
}

/// A path as `getIsFileOpen` compares it: `\` and `/` alike, and on Windows paths (a drive letter
/// or a `\`) letters in one case.
fn path_key(p: &str) -> String {
    let windows = p.contains('\\') || p.as_bytes().get(1) == Some(&b':');
    let p = p.replace('\\', "/");
    let p = p.trim_end_matches('/').to_string();
    if windows { p.to_lowercase() } else { p }
}

/// `app.getIsFileOpen(path)`: whether a document open in the run has the file `path`. The path is
/// compared as text with the documents' files and is not resolved; a document whose file the
/// automation read roots do not allow does not count, and that check can look the file up on disk.
fn app_is_open(h: &Host, a: &[Data]) -> Answer {
    let Data::String(path) = arg(a, 1) else { return Err(HostError::Type("getIsFileOpen needs a path, as a string".into())) };
    if path.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the path is too long".into()));
    }
    let unc = path.starts_with("\\\\") || path.starts_with("//");
    if path.is_empty() || path.contains('\0') || unc || path.split(['/', '\\']).any(|c| c == "..") {
        return Err(HostError::Error("getIsFileOpen cannot check that path".into()));
    }
    let key = path_key(path);
    let s = h.session.borrow();
    let open = s.documents().iter().filter_map(|d| d.path.as_deref()).any(|p| path_key(p) == key && crate::file_access::check_read(p).is_ok());
    Ok(Data::Bool(open))
}

// ---- Documents ----

fn docs_length(h: &Host, _: &[Data]) -> Answer {
    Ok(num(float(h.session.borrow().documents().len() as u64)))
}

fn app_ref(_: &Host, _: &[Data]) -> Answer {
    Ok(host_ref("Application", &[]))
}

/// `documents[i]`, and `documents["name"]` as `getByName` reads it.
fn docs_at(h: &Host, a: &[Data]) -> Answer {
    let s = h.session.borrow();
    let at = match arg(a, 1) {
        Data::Number(n) => whole(*n).and_then(|i| usize::try_from(i).ok()).and_then(|i| s.documents().get(i)),
        Data::String(name) => match s.documents().iter().find(|d| d.title() == *name) {
            None if !probing(a) => return Err(HostError::Error(NOT_FOUND.into())),
            found => found,
        },
        _ => None,
    };
    Ok(at.map_or(Data::Undefined, |st| doc_ref(h, st.uid)))
}

fn docs_by_name(h: &Host, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.title() == *name).ok_or_else(|| HostError::Error(NOT_FOUND.into()))?;
    Ok(doc_ref(h, st.uid))
}

fn docs_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => docs_by_name(h, a),
        Data::Number(_) => match docs_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `file.new`'s `artboardLayout.layout` and whether it runs right to left, for a
/// `DocumentArtboardLayout` value.
fn layout_of(d: &Data) -> std::result::Result<(&'static str, bool), HostError> {
    let Data::String(s) = d else { return Err(HostError::Type("artboardLayout must be a DocumentArtboardLayout value".into())) };
    Ok(match s.strip_prefix("DocumentArtboardLayout.").unwrap_or("") {
        "GridByRow" => ("gridByRow", false),
        "GridByCol" => ("gridByColumn", false),
        "Row" => ("row", false),
        "Column" => ("column", false),
        "RLGridByRow" => ("gridByRow", true),
        "RLGridByCol" => ("gridByColumn", true),
        "RLRow" => ("row", true),
        _ => return Err(HostError::Type("artboardLayout must be a DocumentArtboardLayout value".into())),
    })
}

/// The settings of a new document a script gives, each `None` when it was left out.
#[derive(Default)]
struct NewDoc<'a> {
    color_space: Option<&'a Data>,
    width: Option<&'a Data>,
    height: Option<&'a Data>,
    artboards: Option<&'a Data>,
    layout: Option<&'a Data>,
    spacing: Option<&'a Data>,
    rows_or_cols: Option<&'a Data>,
    units: Option<&'a Data>,
    title: Option<&'a Data>,
    preview: Option<&'a Data>,
    resolution: Option<&'a Data>,
}

/// `d`, unless it is `undefined`.
fn given(d: &Data) -> Option<&Data> {
    (!matches!(d, Data::Undefined)).then_some(d)
}

impl NewDoc<'_> {
    /// `file.new`'s params for these settings, checked before anything is made.
    fn params(&self) -> std::result::Result<serde_json::Map<String, Value>, HostError> {
        let mut p = serde_json::Map::new();
        if let Some(d) = self.color_space {
            let mode = match d {
                Data::String(s) if s == "DocumentColorSpace.RGB" => "rgb",
                Data::String(s) if s == "DocumentColorSpace.CMYK" => "cmyk",
                _ => return Err(HostError::Type("colorSpace must be a DocumentColorSpace value".into())),
            };
            p.insert("colorMode".into(), json!(mode));
        }
        for (d, key) in [(self.width, "width"), (self.height, "height")] {
            if let Some(d) = d {
                let v = number(d, key)?;
                if v <= 0.0 || v > MAX_SIZE {
                    return Err(HostError::Range(format!("{key} must be more than 0 and at most 3999856 points")));
                }
                p.insert(key.into(), json!(v));
            }
        }
        let count = match self.artboards {
            Some(d) => {
                let n = number(d, "numArtBoards")?;
                if n.fract() != 0.0 || !(1.0..=MAX_ARTBOARDS).contains(&n) {
                    return Err(HostError::Range("numArtBoards must be a whole number from 1 to 100".into()));
                }
                p.insert("artboards".into(), json!(n));
                n
            }
            None => 1.0,
        };
        let mut layout = serde_json::Map::new();
        if let Some(d) = self.layout {
            let (name, rtl) = layout_of(d)?;
            layout.insert("layout".into(), json!(name));
            layout.insert("rightToLeft".into(), json!(rtl));
        }
        if let Some(d) = self.spacing {
            let v = number(d, "artboardSpacing")?;
            if !(0.0..=crate::cmd::newdoc::MAX_SPACING).contains(&v) {
                return Err(HostError::Range("artboardSpacing must be from 0 to 100000 points".into()));
            }
            layout.insert("spacing".into(), json!(v));
        }
        if let Some(d) = self.rows_or_cols {
            let n = number(d, "artboardRowsOrCols")?;
            if n.fract() != 0.0 || n < 1.0 || n > count {
                return Err(HostError::Range("artboardRowsOrCols must be a whole number from 1 to numArtBoards".into()));
            }
            // A grid filled by column takes the number of rows.
            let by_column = layout.get("layout").is_some_and(|l| l == "gridByColumn");
            layout.insert((if by_column { "rows" } else { "columns" }).into(), json!(n));
        }
        if !layout.is_empty() {
            p.insert("artboardLayout".into(), Value::Object(layout));
        }
        if let Some(d) = self.units {
            let units = match d {
                Data::String(s) => match s.strip_prefix("RulerUnits.").unwrap_or("") {
                    u @ ("Points" | "Picas" | "Inches" | "Millimeters" | "Centimeters" | "Pixels") => u,
                    "Qs" | "Unknown" => return Err(HostError::Range(format!("a new document cannot have the units {s}"))),
                    _ => return Err(HostError::Type("units must be a RulerUnits value".into())),
                },
                _ => return Err(HostError::Type("units must be a RulerUnits value".into())),
            };
            p.insert("units".into(), json!(units));
        }
        if let Some(d) = self.title {
            let Data::String(t) = d else { return Err(HostError::Type("title must be a string".into())) };
            if t.len() > MAX_TEXT_BYTES {
                return Err(HostError::Range("the title is too long".into()));
            }
            p.insert("name".into(), json!(t));
        }
        if let Some(d) = self.preview {
            let mode = match d {
                Data::String(s) if s == "DocumentPreviewMode.DefaultPreview" => "default",
                // Pixel Preview is a setting of the app's view, which a script cannot turn on.
                Data::String(s) if s == "DocumentPreviewMode.PixelPreview" => {
                    return Err(HostError::Error("DocumentPreviewMode.PixelPreview is not available yet".into()));
                }
                Data::String(s) if s == "DocumentPreviewMode.OverprintPreview" => "overprint",
                _ => return Err(HostError::Type("previewMode must be a DocumentPreviewMode value".into())),
            };
            p.insert("previewMode".into(), json!(mode));
        }
        if let Some(d) = self.resolution {
            let ppi = match d {
                Data::String(s) if s == "DocumentRasterResolution.ScreenResolution" => 72,
                Data::String(s) if s == "DocumentRasterResolution.MediumResolution" => 150,
                Data::String(s) if s == "DocumentRasterResolution.HighResolution" => 300,
                _ => return Err(HostError::Type("rasterResolution must be a DocumentRasterResolution value".into())),
            };
            p.insert("rasterEffectsPpi".into(), json!(ppi));
        }
        Ok(p)
    }
}

/// Runs `file.new` with `p`. → the new document, which is the active one.
fn new_document(h: &Host, p: serde_json::Map<String, Value>) -> Answer {
    command(h, "file.new", &Value::Object(p))?;
    let s = h.session.borrow();
    let st = s.active().ok_or_else(|| HostError::Error(NO_ACTIVE_DOCUMENT.into()))?;
    Ok(doc_ref(h, st.uid))
}

/// `documents.add(colorSpace?, width?, height?, numArtBoards?, artboardLayout?, artboardSpacing?,
/// artboardRowsOrCols?)`: a new document from `file.new`, which becomes the active one.
fn docs_add(h: &Host, a: &[Data]) -> Answer {
    let settings = NewDoc {
        color_space: given(arg(a, 1)),
        width: given(arg(a, 2)),
        height: given(arg(a, 3)),
        artboards: given(arg(a, 4)),
        layout: given(arg(a, 5)),
        spacing: given(arg(a, 6)),
        rows_or_cols: given(arg(a, 7)),
        ..NewDoc::default()
    };
    new_document(h, settings.params()?)
}

/// The error of `addDocument` with `showOptionsDialog` true: a script cannot open the New
/// Document dialog.
pub const DIALOG_UNAVAILABLE: &str = "Dialog unavailable";

/// The New Document profile a startup preset names (`None`: no profile), and the color mode the
/// basic presets give.
fn startup_preset(name: &str) -> Option<(Option<&'static str>, Option<&'static str>)> {
    let name = name.strip_prefix("DocumentPresetType.").unwrap_or(name);
    let squashed: String = name.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    Some(match squashed.as_str() {
        "basiccmyk" => (None, Some("cmyk")),
        "basicrgb" => (None, Some("rgb")),
        "video" | "film&video" => (Some("Film & Video"), None),
        _ => {
            let same = |p: &&str| p.chars().filter(|c| !c.is_whitespace()).collect::<String>().eq_ignore_ascii_case(&squashed);
            (Some(crate::cmd::newdoc::profile_names().find(same)?), None)
        }
    })
}

/// `documents.addDocument(startupPreset, presetSettings?, showOptionsDialog?)`: a new document from
/// the New Document profile `startupPreset` names, with the settings `presetSettings` gives in
/// place of the profile's.
fn docs_add_document(h: &Host, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("addDocument needs a startup preset's name".into())) };
    if name.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the startup preset's name is too long".into()));
    }
    let (profile, mode) = startup_preset(name).ok_or_else(|| HostError::Error(format!("no startup preset called {name}")))?;
    // Only an object `new DocumentPreset()` made: a plain object such as `{width: 500}` is refused.
    let settings = match given(arg(a, 2)) {
        None | Some(Data::Null) => Vec::new(),
        Some(d) => {
            let i = preset_of(h, d).ok_or_else(|| HostError::Type("presetSettings must be a DocumentPreset".into()))?;
            // The settings are charged as data passed to the call before they are copied.
            let bytes = h.dom.presets.borrow()[i].iter().map(|(k, v)| k.len() as u64 + 2 + data_size(v)).sum::<u64>();
            h.charge_call(bytes)?;
            h.dom.presets.borrow()[i].clone()
        }
    };
    let field = |k: &str| settings.iter().find(|(n, _)| n == k).map(|(_, v)| v).and_then(given);
    for k in ["documentBleedLink", "documentBleedOffsetRect", "transparencyGrid"] {
        if field(k).is_some_and(|d| !matches!(d, Data::Null)) {
            return Err(HostError::Error(format!("DocumentPreset.{k} is not available yet")));
        }
    }
    let new = NewDoc {
        color_space: field("colorMode"),
        width: field("width"),
        height: field("height"),
        artboards: field("numArtboards"),
        layout: field("artboardLayout"),
        spacing: field("artboardSpacing"),
        rows_or_cols: field("artboardRowsOrCols"),
        units: field("units"),
        title: field("title"),
        preview: field("previewMode"),
        resolution: field("rasterResolution"),
    };
    let mut p = new.params()?;
    match given(arg(a, 3)) {
        None => {}
        Some(Data::Bool(false)) => {}
        Some(Data::Bool(true)) => return Err(HostError::Error(DIALOG_UNAVAILABLE.into())),
        Some(_) => return Err(HostError::Type("showOptionsDialog must be true or false".into())),
    }
    // The document starts from the first preset of the profile's category (Print: Letter).
    let first = profile.and_then(|n| crate::cmd::newdoc::category(&h.session.borrow(), n)?.into_iter().next());
    if let Some(first) = first {
        p.insert("preset".into(), json!(first.name));
    }
    p.insert("profile".into(), profile.map_or(Value::Null, |n| json!(n)));
    if let Some(m) = mode {
        p.entry("colorMode").or_insert_with(|| json!(m));
    }
    // A profile whose file could not be read throws, and the document `file.new` made with the
    // defaults is put back, with the Recent sizes as they were.
    atomically(h, |h| {
        let (_, value) = command_value(h, "file.new", &Value::Object(p))?;
        if let Some(e) = value.get("profileError") {
            let profile = e["profile"].as_str().unwrap_or_default();
            let reason = e["reason"].as_str().unwrap_or_default();
            return Err(HostError::Error(format!("the New Document profile {profile} could not be read: {reason}")));
        }
        // Each warning from reading the profile's file goes to the console.
        for w in value.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            super::each_line(&format!("addDocument: {w}"), |line, cut| h.line(super::console::Entry::Log(line), cut));
        }
        let s = h.session.borrow();
        let st = s.active().ok_or_else(|| HostError::Error(NO_ACTIVE_DOCUMENT.into()))?;
        Ok(doc_ref(h, st.uid))
    })
}

/// `new DocumentPreset()`: a record of new-document settings with the defaults the help page
/// <https://ai-scripting.docsforadobe.dev/jsobjref/DocumentPreset/> gives; the other settings are
/// left out. The settings stay with the run, which hands the script an object of the
/// `DocumentPreset` class.
fn new_preset(h: &Host) -> Answer {
    let mut presets = h.dom.presets.borrow_mut();
    if presets.len() >= MAX_PRESETS {
        return Err(HostError::Range(format!("a script can make at most {MAX_PRESETS} DocumentPresets")));
    }
    let item = |k: &str, v: Data| (k.to_string(), v);
    presets.push(vec![
        item("artboardLayout", text("DocumentArtboardLayout.GridByRow")),
        item("artboardRowsOrCols", num(1.0)),
        item("artboardSpacing", num(20.0)),
        item("height", num(792.0)),
        item("numArtboards", num(1.0)),
        item("width", num(612.0)),
    ]);
    Ok(host_ref("DocumentPreset", &[float(h.run), float(presets.len() as u64 - 1)]))
}

/// The most `DocumentPreset` objects one run can make.
const MAX_PRESETS: usize = 10_000;

/// The settings a `DocumentPreset` has, each a property a script reads and assigns.
const PRESET_FIELDS: &[&str] = &[
    "artboardLayout",
    "artboardRowsOrCols",
    "artboardSpacing",
    "colorMode",
    "documentBleedLink",
    "documentBleedOffsetRect",
    "height",
    "numArtboards",
    "previewMode",
    "rasterResolution",
    "title",
    "transparencyGrid",
    "units",
    "width",
];

/// The index in the run's presets of the `DocumentPreset` `d` is, when it is one this run made.
fn preset_of(h: &Host, d: &Data) -> Option<usize> {
    let handle = handle_of(d, "DocumentPreset")?;
    let [run, i] = handle.as_slice() else { return None };
    if whole(*run) != Some(h.run) {
        return None;
    }
    let i = usize::try_from(whole(*i)?).ok()?;
    (i < h.dom.presets.borrow().len()).then_some(i)
}

/// The getter and setter of the `DocumentPreset` setting `key`: an unset one reads `undefined`, and
/// a value is checked when `addDocument` uses it.
fn preset_field(link: &Link, key: &'static str) -> HostMember {
    let not_preset = || HostError::Type("this is not a DocumentPreset".into());
    let l = link.clone();
    let get: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        // The value handed back is charged as the setter charges it, before it is copied.
        let i = preset_of(&h, arg(&a, 0));
        let bytes = i.map_or(0, |i| h.dom.presets.borrow()[i].iter().find(|(k, _)| k == key).map_or(0, |(_, v)| data_size(v)));
        h.charge_call(16 + bytes)?;
        let i = i.ok_or_else(not_preset)?;
        let presets = h.dom.presets.borrow();
        Ok(presets[i].iter().find(|(k, _)| k == key).map_or(Data::Undefined, |(_, v)| v.clone()))
    });
    let l = link.clone();
    let set: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        let value = arg(&a, 1).clone();
        h.charge_call(16 + data_size(&value))?;
        let i = preset_of(&h, arg(&a, 0)).ok_or_else(not_preset)?;
        let mut presets = h.dom.presets.borrow_mut();
        let fields = &mut presets[i];
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => fields.push((key.to_string(), value)),
        }
        Ok(Data::Undefined)
    });
    HostMember::Property { get, set }
}

// ---- Document ----

fn this_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "Document")
}

fn doc_typename(h: &Host, a: &[Data]) -> Answer {
    this_doc(h, a)?;
    Ok(text("Document"))
}

fn doc_parent(h: &Host, a: &[Data]) -> Answer {
    this_doc(h, a)?;
    Ok(host_ref("Application", &[]))
}

fn doc_name(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(text(with_doc(h, doc, |_, st| st.title())))
}

fn doc_size(h: &Host, a: &[Data], width: bool) -> Answer {
    let doc = this_doc(h, a)?;
    with_doc(h, doc, |_, st| {
        let r = st.doc.artboards.first().map(|b| b.rect).ok_or_else(|| HostError::Error("the document has no artboards".into()))?;
        Ok(num(if width { r.width() } else { r.height() }))
    })
}

fn doc_width(h: &Host, a: &[Data]) -> Answer {
    doc_size(h, a, true)
}

fn doc_height(h: &Host, a: &[Data]) -> Answer {
    doc_size(h, a, false)
}

fn doc_units(h: &Host, a: &[Data]) -> Answer {
    use crate::doc::Unit;
    let doc = this_doc(h, a)?;
    let unit = with_doc(h, doc, |_, st| st.doc.units);
    Ok(text(match unit {
        Unit::Points => "RulerUnits.Points",
        Unit::Picas => "RulerUnits.Picas",
        Unit::Inches => "RulerUnits.Inches",
        Unit::Millimeters => "RulerUnits.Millimeters",
        Unit::Centimeters => "RulerUnits.Centimeters",
        Unit::Pixels => "RulerUnits.Pixels",
        Unit::Q => "RulerUnits.Qs",
        _ => "RulerUnits.Unknown",
    }))
}

fn doc_color_space(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let cmyk = with_doc(h, doc, |_, st| st.doc.color_mode == crate::doc::ColorMode::Cmyk);
    Ok(text(if cmyk { "DocumentColorSpace.CMYK" } else { "DocumentColorSpace.RGB" }))
}

fn doc_ruler_origin(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    with_doc(h, doc, |_, st| {
        let f = frame(st)?;
        Ok(pair(f.to_script(Coords::Document, st.doc.ruler_origin_for(active_artboard(st)))))
    })
}

/// `rulerOrigin = [x, y]`, in document coordinates: `document.setRulerOrigin` for the active
/// artboard. The art stays where it is.
fn doc_set_ruler_origin(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let p = point(arg(a, 1), "rulerOrigin")?;
    let (to, now, artboard) = with_doc(h, doc, |_, st| {
        let f = frame(st)?;
        let artboard = active_artboard(st);
        Ok::<_, HostError>((f.to_document(Coords::Document, p), st.doc.ruler_origin_for(artboard), artboard))
    })?;
    if to.distance(now) > 1e-9 {
        on_doc(h, doc.uid, false, |h| command(h, "document.setRulerOrigin", &json!({"x": to.x, "y": to.y, "artboard": artboard})))?;
    }
    Ok(Data::Undefined)
}

/// `convertCoordinate(point, source, destination)`: `point` from one coordinate system of the
/// document to another. Nothing changes.
fn doc_convert(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let p = point(arg(a, 1), "point")?;
    let (from, to) = (coords_of(arg(a, 2))?, coords_of(arg(a, 3))?);
    with_doc(h, doc, |_, st| {
        let f = frame(st)?;
        Ok(pair(f.to_script(to, f.to_document(from, p))))
    })
}

fn doc_page_items(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(host_ref("PageItems", &[float(h.run), float(doc.uid)]))
}

fn doc_selection(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    selection_of(h, doc.uid)
}

/// The selection of the document `uid`: a new list of the selected objects, topmost first as
/// `pageItems` lists them, empty when nothing is selected.
fn selection_of(h: &Host, uid: u64) -> Answer {
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
    let picked: HashSet<NodeId> = st.selection.objects.iter().copied().filter(|i| st.doc.node(*i).and_then(typename).is_some()).collect();
    if picked.len() > MAX_LISTED {
        return Err(HostError::Range(format!("the selection has more than {MAX_LISTED} objects")));
    }
    let ids: Vec<NodeId> = if picked.is_empty() {
        Vec::new()
    } else {
        let all = page_items(h, st);
        h.charge_call(all.len() as u64)?;
        all.iter().copied().filter(|i| picked.contains(i)).collect()
    };
    h.charge_call(16 * ids.len() as u64)?;
    Ok(Data::Array(ids.iter().map(|i| item_ref(h, st, *i)).collect()))
}

/// The ids an assignment to a document's selection gives: one page item, a list of them, `null` or
/// `[]`. Every one must be an object of that document that can be selected.
fn selection_ids(h: &Host, doc: Doc, v: &Data) -> std::result::Result<Vec<NodeId>, HostError> {
    let list: Vec<&Data> = match v {
        Data::Null | Data::Undefined => Vec::new(),
        Data::Array(a) if a.len() > MAX_LISTED => return Err(HostError::Range(format!("a selection takes at most {MAX_LISTED} objects"))),
        Data::Array(a) => a.iter().collect(),
        d => vec![d],
    };
    let mut ids: Vec<NodeId> = Vec::with_capacity(list.len());
    // The document is read once for every object: nothing changes it while the list is checked.
    let (d, isolation, guides_locked) = with_doc(h, doc, |s, st| (st.doc.clone(), st.isolation, s.guides_locked()));
    let locks = d.locked_artboards();
    for v in list {
        let item = item_of(h, v)?;
        // A path made into a guide is not selected while Lock Guides is on, as the selection
        // commands leave it out.
        let ok = item.doc.uid == doc.uid
            && crate::cmd::select::is_selectable(&d, isolation, &locks, item.id)
            && !(guides_locked && crate::is_guide_path(&d, item.id));
        if !ok {
            let name = with_doc(h, item.doc, |_, st| st.doc.node(item.id).and_then(typename).unwrap_or("page item"));
            return Err(HostError::Error(format!("Cannot select {name} {}", item.id.0)));
        }
        if !ids.contains(&item.id) {
            ids.push(item.id);
        }
    }
    Ok(ids)
}

fn doc_set_selection(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let ids = selection_ids(h, doc, arg(a, 1))?;
    set_selection(h, doc, ids)
}

/// Makes `ids` the selection of `doc`, with no command when it already is: the same objects, each
/// selected as a whole (no dynamic symbol instance's object or Objects on Path group's object
/// chosen in it), and no ruler guide, slice, key or Layers panel target.
fn set_selection(h: &Host, doc: Doc, ids: Vec<NodeId>) -> Answer {
    let same = with_doc(h, doc, |_, st| {
        let sel = &st.selection;
        sel.objects == ids
            && sel.target.is_none()
            && sel.symbol_part.is_none()
            && sel.on_path_member.is_none()
            && sel.anchors.is_empty()
            && sel.segments.is_empty()
            && sel.guides.is_empty()
            && sel.slices.is_empty()
            && sel.key.is_none()
            && sel.key_guide.is_none()
            && sel.key_anchor.is_none()
    });
    if same {
        return Ok(Data::Undefined);
    }
    on_doc(h, doc.uid, false, |h| {
        if ids.is_empty() {
            command(h, "select.none", &json!({}))
        } else {
            command(h, "select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
        }
    })?;
    Ok(Data::Undefined)
}

// ---- PageItems ----

fn items_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "PageItems")
}

fn items_length(h: &Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let n = with_doc(h, doc, |_, st| page_items(h, st).len());
    Ok(num(float(n as u64)))
}

fn items_typename(h: &Host, a: &[Data]) -> Answer {
    items_doc(h, a)?;
    Ok(text("PageItems"))
}

fn items_parent(h: &Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    Ok(doc_ref(h, doc.uid))
}

/// Whether an indexer was called for `name in collection` ([`vectorcraft_js::HostClass::index`]),
/// where a name nothing has gives `undefined` instead of the `Not found` error a read throws.
fn probing(a: &[Data]) -> bool {
    matches!(arg(a, 2), Data::Bool(true))
}

/// `pageItems[i]`, and `pageItems["name"]` as `getByName` reads it.
fn items_at(h: &Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    if let Data::String(_) = arg(a, 1) {
        return match items_by_name(h, a) {
            Err(HostError::Error(e)) if e == NOT_FOUND && probing(a) => Ok(Data::Undefined),
            r => r,
        };
    }
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    Ok(with_doc(h, doc, |_, st| page_items(h, st).get(i).map_or(Data::Undefined, |id| item_ref(h, st, *id))))
}

fn items_by_name(h: &Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        let ids = page_items(h, st);
        h.charge_call(ids.len() as u64)?;
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| item_ref(h, st, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn items_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => items_by_name(h, a),
        Data::Number(_) => match items_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `pageItems.removeAll()`: deletes every object of the document in one command. A hidden or
/// locked object, or one in a hidden or locked layer or group, stops it before anything is deleted,
/// and so does an object on a locked artboard.
fn items_remove_all(h: &Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let (ids, refused, on_locked) = with_doc(h, doc, |_, st| {
        let ids = top_items(&st.doc);
        let whole = |id: &NodeId| editable(&st.doc, *id) && st.doc.node(*id).is_some_and(all_editable);
        let refused: Vec<u64> = ids.iter().filter(|i| !whole(i)).map(|i| i.0).take(20).collect();
        let locks = st.doc.locked_artboards();
        let on_locked: Vec<u64> = ids.iter().filter(|i| locks.holds_id(**i)).map(|i| i.0).take(20).collect();
        (ids, refused, on_locked)
    });
    if !refused.is_empty() {
        let list = refused.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("cannot delete hidden or locked objects: {list}")));
    }
    if !on_locked.is_empty() {
        let list = on_locked.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("{ON_LOCKED_ARTBOARD}: {list}")));
    }
    if !ids.is_empty() {
        on_doc(h, doc.uid, true, |h| command(h, "edit.clear", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    }
    Ok(Data::Undefined)
}

// ---- PageItem ----

fn this_item(h: &Host, a: &[Data]) -> std::result::Result<Item, HostError> {
    item_of(h, arg(a, 0))
}

/// Reads the item `this` names with `f`, given its document and node.
fn read_item(h: &Host, a: &[Data], f: impl FnOnce(&DocState, &crate::doc::Node) -> Answer) -> Answer {
    let item = this_item(h, a)?;
    with_doc(h, item.doc, |_, st| match st.doc.node(item.id) {
        Some(n) => f(st, n),
        None => Err(HostError::Error(STALE_ITEM.into())),
    })
}

fn item_typename(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(text(typename(n).unwrap_or("PageItem"))))
}

fn item_parent(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    Ok(with_doc(h, item.doc, |_, st| st.doc.parent_of(item.id).map_or(Data::Null, |p| item_ref(h, st, p))))
}

fn item_name(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(text(n.name.clone().unwrap_or_default())))
}

/// Runs `object.setProps` with `props` for the item, unless `unchanged` says the node has them.
fn set_props(h: &Host, a: &[Data], unchanged: impl FnOnce(&crate::doc::Node) -> bool, props: Value) -> Answer {
    let item = this_item(h, a)?;
    let same = with_doc(h, item.doc, |_, st| st.doc.node(item.id).is_some_and(unchanged));
    if !same {
        let mut p = props;
        if let Some(o) = p.as_object_mut() {
            o.insert("ids".into(), json!([item.id.0]));
        }
        edit_item(h, item, "object.setProps", &p)?;
    }
    Ok(Data::Undefined)
}

/// `name = value`: a number or a Boolean becomes the text `String(value)` gives.
fn item_set_name(h: &Host, a: &[Data]) -> Answer {
    let name = match arg(a, 1) {
        Data::String(s) => s.clone(),
        Data::Number(n) => vectorcraft_js::number_to_string(*n),
        Data::Bool(b) => b.to_string(),
        _ => return Err(HostError::Type("name must be a string".into())),
    };
    let name = &name;
    if name.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the name is too long".into()));
    }
    set_props(h, a, |n| n.name.as_deref().unwrap_or("") == name, json!({"name": name}))
}

fn note_of(n: &crate::doc::Node) -> &str {
    n.extras().attrs.as_deref().map_or("", |a| a.note.as_str())
}

fn item_note(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(text(note_of(n))))
}

/// `note = text`: the Attributes panel's note, set with `attributes.set`, which trims it.
fn item_set_note(h: &Host, a: &[Data]) -> Answer {
    let Data::String(note) = arg(a, 1) else { return Err(HostError::Type("note must be a string".into())) };
    if note.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the note is too long".into()));
    }
    let item = this_item(h, a)?;
    let same = with_doc(h, item.doc, |_, st| st.doc.node(item.id).is_some_and(|n| note_of(n) == note.trim()));
    if !same {
        edit_item(h, item, "attributes.set", &json!({"ids": [item.id.0], "note": note}))?;
    }
    Ok(Data::Undefined)
}

fn item_hidden(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(Data::Bool(!n.visible)))
}

fn item_set_hidden(h: &Host, a: &[Data]) -> Answer {
    let hidden = boolean(arg(a, 1), "hidden")?;
    set_props(h, a, |n| n.visible != hidden, json!({"visible": !hidden}))
}

fn item_locked(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(Data::Bool(n.locked)))
}

fn item_set_locked(h: &Host, a: &[Data]) -> Answer {
    let locked = boolean(arg(a, 1), "locked")?;
    set_props(h, a, |n| n.locked == locked, json!({"locked": locked}))
}

fn item_opacity(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(num((f64::from(n.opacity) * 100.0 * 1e4).round() / 1e4)))
}

fn item_set_opacity(h: &Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "opacity")?;
    if !(0.0..=100.0).contains(&v) {
        return Err(HostError::Range("opacity must be from 0 to 100".into()));
    }
    // A value the getter reports for the item's opacity sends the opacity it has, and
    // `object.setProps` finds the no-op itself: an opacity a graph mark or label already has still
    // replaces the one the graph keeps for its series or kind.
    let item = this_item(h, a)?;
    let refused = with_doc(h, item.doc, |_, st| {
        let d = &st.doc;
        let closed = |i: &NodeId| d.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Layer { .. }) && (!n.visible || n.locked));
        d.ancestry(item.id).is_some_and(|a| a.iter().any(closed)).then(|| d.node(item.id).and_then(typename).unwrap_or("page item"))
    });
    if let Some(name) = refused {
        return Err(HostError::Error(format!("Cannot edit {name} {}: a layer it is in is hidden or locked", item.id.0)));
    }
    let now = with_doc(h, item.doc, |_, st| st.doc.node(item.id).map(|n| f64::from(n.opacity) * 100.0));
    let v = now.filter(|now| (now - v).abs() < 1e-4).unwrap_or(v);
    set_props(h, a, |_| false, json!({"opacity": v}))
}

fn item_selected(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    Ok(Data::Bool(with_doc(h, item.doc, |_, st| st.selection.objects.contains(&item.id))))
}

fn item_set_selected(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    let on = boolean(arg(a, 1), "selected")?;
    if with_doc(h, item.doc, |_, st| st.selection.objects.contains(&item.id)) == on {
        return Ok(Data::Undefined);
    }
    // Adding or removing the one object keeps how the others are selected (their direct-selected
    // anchors and segments, and the key object).
    let ids = if on { selection_ids(h, item.doc, arg(a, 0))? } else { vec![item.id] };
    let id = if on { "select.add" } else { "select.toggle" };
    on_doc(h, item.doc.uid, false, |h| command(h, id, &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    Ok(Data::Undefined)
}

/// The item's geometric bounds (`visual`: its visible bounds) in document coordinates and the
/// document's [`Frame`].
fn item_rect(h: &Host, item: Item, visual: bool) -> std::result::Result<(Rect, Frame), HostError> {
    with_doc(h, item.doc, |_, st| {
        let r = st.doc.bounds_of(&[item.id], visual).ok_or_else(|| HostError::Error("the object has no bounds".into()))?;
        Ok((r, frame(st)?))
    })
}

fn bounds(h: &Host, a: &[Data], visual: bool) -> Answer {
    let item = this_item(h, a)?;
    let (r, f) = item_rect(h, item, visual)?;
    Ok(f.bounds(h.dom.coords.get(), r))
}

fn item_geometric(h: &Host, a: &[Data]) -> Answer {
    bounds(h, a, false)
}

fn item_visible(h: &Host, a: &[Data]) -> Answer {
    bounds(h, a, true)
}

fn item_position(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    let (r, f) = item_rect(h, item, false)?;
    Ok(pair(f.to_script(h.dom.coords.get(), Point::new(r.x0, r.y0))))
}

fn item_left(h: &Host, a: &[Data]) -> Answer {
    match item_position(h, a)? {
        Data::Array(p) => Ok(p.into_iter().next().unwrap_or(Data::Undefined)),
        d => Ok(d),
    }
}

fn item_top(h: &Host, a: &[Data]) -> Answer {
    match item_position(h, a)? {
        Data::Array(p) => Ok(p.into_iter().nth(1).unwrap_or(Data::Undefined)),
        d => Ok(d),
    }
}

fn item_width(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    Ok(num(item_rect(h, item, false)?.0.width()))
}

fn item_height(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    Ok(num(item_rect(h, item, false)?.0.height()))
}

/// Moves the item so its top-left corner is at `x` and `y` in script coordinates (`None`: that one
/// stays).
fn move_to(h: &Host, a: &[Data], x: Option<f64>, y: Option<f64>) -> Answer {
    let item = this_item(h, a)?;
    let (r, f) = item_rect(h, item, false)?;
    let c = h.dom.coords.get();
    let now = f.to_script(c, Point::new(r.x0, r.y0));
    let to = f.to_document(c, [x.unwrap_or(now[0]), y.unwrap_or(now[1])]);
    let (dx, dy) = (to.x - r.x0, to.y - r.y0);
    if dx.abs() > 1e-9 || dy.abs() > 1e-9 {
        edit_item(h, item, "object.move", &json!({"ids": [item.id.0], "dx": dx, "dy": dy}))?;
    }
    Ok(Data::Undefined)
}

fn item_set_position(h: &Host, a: &[Data]) -> Answer {
    let p = point(arg(a, 1), "position")?;
    move_to(h, a, Some(p[0]), Some(p[1]))
}

fn item_set_left(h: &Host, a: &[Data]) -> Answer {
    let x = coordinate(arg(a, 1), "left")?;
    move_to(h, a, Some(x), None)
}

fn item_set_top(h: &Host, a: &[Data]) -> Answer {
    let y = coordinate(arg(a, 1), "top")?;
    move_to(h, a, None, Some(y))
}

/// Scales the item about its top-left corner along the page's axes to `size` points wide (`width`)
/// or tall, so the page-aligned bounds that `width` and `height` read have that size, rotated
/// objects included.
fn resize(h: &Host, a: &[Data], width: bool) -> Answer {
    let what = if width { "width" } else { "height" };
    let size = number(arg(a, 1), what)?;
    if size <= 0.0 || size > MAX_SIZE {
        return Err(HostError::Range(format!("{what} must be more than 0 and at most 3999856 points")));
    }
    let item = this_item(h, a)?;
    let (r, _) = item_rect(h, item, false)?;
    let now = if width { r.width() } else { r.height() };
    if now <= 0.0 {
        return Err(HostError::Error(format!("the object has no {what} to scale")));
    }
    if (now - size).abs() < 1e-9 {
        return Ok(Data::Undefined);
    }
    let pct = size / now * 100.0;
    let (sx, sy) = if width { (pct, 100.0) } else { (100.0, pct) };
    let p = json!({"ids": [item.id.0], "sx": sx, "sy": sy, "origin": [r.x0, r.y0], "basis": "page"});
    edit_item(h, item, "object.scale", &p)?;
    Ok(Data::Undefined)
}

fn item_set_width(h: &Host, a: &[Data]) -> Answer {
    resize(h, a, true)
}

fn item_set_height(h: &Host, a: &[Data]) -> Answer {
    resize(h, a, false)
}

/// `editable`: false for an object that is hidden or locked, in a hidden or locked layer or group,
/// or on a locked artboard.
fn item_editable(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    Ok(Data::Bool(with_doc(h, item.doc, |_, st| editable(&st.doc, item.id) && !on_locked_artboard(h, st, item.id))))
}

/// The item's place in its parent's stacking order, from 1 at the bottom.
fn item_z_order_position(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    let at = with_doc(h, item.doc, |_, st| st.doc.position(item.id).map(|(_, i, _)| i));
    at.map(|i| num(float(i as u64 + 1))).ok_or_else(|| HostError::Error(STALE_ITEM.into()))
}

/// `remove()`: deletes the object. A hidden or locked object, one in a hidden or locked layer or
/// group, a group holding one, or an object on a locked artboard is refused, as [`items_remove_all`]
/// refuses it.
fn item_remove(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    let (whole, on_locked) = with_doc(h, item.doc, |_, st| {
        (editable(&st.doc, item.id) && st.doc.node(item.id).is_some_and(all_editable), st.doc.on_locked_artboard(item.id))
    });
    if !whole {
        return Err(HostError::Error(format!("cannot delete hidden or locked objects: {}", item.id.0)));
    }
    if on_locked {
        return Err(HostError::Error(format!("{ON_LOCKED_ARTBOARD}: {}", item.id.0)));
    }
    edit_item(h, item, "edit.clear", &json!({"ids": [item.id.0]}))?;
    Ok(Data::Undefined)
}

/// The error of `remove()` and `removeAll()` for an object on a locked artboard.
const ON_LOCKED_ARTBOARD: &str = "cannot delete objects on a locked artboard";

/// The error of `move` and `duplicate` given an object of another document.
pub const CROSS_DOCUMENT: &str = "Cross-document placement is not available";

/// An `ElementPlacement` value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// `INSIDE` and `PLACEATBEGINNING`: on top of the container's contents.
    Top,
    /// `PLACEATEND`: under the container's contents.
    Bottom,
    /// `PLACEBEFORE`: directly above the relative object, in its parent.
    Above,
    /// `PLACEAFTER`: directly below the relative object, in its parent.
    Below,
}

fn placement_of(d: &Data) -> std::result::Result<Placement, HostError> {
    match d {
        Data::Undefined => Ok(Placement::Top),
        Data::String(s) => match s.as_str() {
            "ElementPlacement.INSIDE" | "ElementPlacement.PLACEATBEGINNING" => Ok(Placement::Top),
            "ElementPlacement.PLACEATEND" => Ok(Placement::Bottom),
            "ElementPlacement.PLACEBEFORE" => Ok(Placement::Above),
            "ElementPlacement.PLACEAFTER" => Ok(Placement::Below),
            _ => Err(HostError::Type("insertionLocation must be an ElementPlacement value".into())),
        },
        _ => Err(HostError::Type("insertionLocation must be an ElementPlacement value".into())),
    }
}

/// Where `move` and `duplicate` put an object relative to `target`: the parent it goes into and
/// the target it goes beside, checked against the object `item` before anything changes.
struct Destination {
    parent: NodeId,
    beside: Option<NodeId>,
    how: Placement,
}

/// The object a relative object of `move` or `duplicate` names: a page item or a layer of the
/// same document as `item`. An object may go directly above or below itself: a copy goes there, as
/// `layer.move` puts copies, and a move there leaves the object where it is.
fn destination(h: &Host, item: Item, target: &Data, how: &Data) -> std::result::Result<Destination, HostError> {
    let how = placement_of(how)?;
    let beside = matches!(how, Placement::Above | Placement::Below);
    let handle = handle_of(target, "PageItem").or_else(|| handle_of(target, "Layer"));
    let Some(&[run, uid, high, low]) = handle.as_deref() else {
        if handle_of(target, "Document").is_some() {
            return Err(HostError::Error("a document as the relative object is not available yet".into()));
        }
        return Err(HostError::Type("the relative object must be a page item or a layer".into()));
    };
    let doc = find_doc(h, run, uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    if doc.uid != item.doc.uid {
        return Err(HostError::Error(CROSS_DOCUMENT.into()));
    }
    let target = id_of(high, low).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    with_doc(h, doc, |_, st| {
        let d = &st.doc;
        let t = d.node(target).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        let inside = d.ancestry(target).is_some_and(|a| a.iter().any(|i| *i == item.id && *i != target));
        if inside || (target == item.id && !beside) {
            return Err(HostError::Error("an object cannot go into itself".into()));
        }
        let (parent, beside) = match how {
            Placement::Top | Placement::Bottom => {
                if !matches!(t.kind, NodeKind::Layer { .. } | NodeKind::Group { .. }) {
                    return Err(HostError::Error(format!("{} cannot hold objects", typename(t).unwrap_or("Layer"))));
                }
                (target, None)
            }
            Placement::Above | Placement::Below => {
                let parent = d.parent_of(target).ok_or_else(|| HostError::Error("an object cannot go beside a top-level layer".into()))?;
                (parent, Some(target))
            }
        };
        if !editable(d, parent) {
            return Err(HostError::Error("the layer or group is hidden or locked".into()));
        }
        // An object on a locked artboard is not moved, as `editable` reads false for it.
        if !editable(d, item.id) || on_locked_artboard(h, st, item.id) {
            let name = d.node(item.id).and_then(typename).unwrap_or("page item");
            return Err(HostError::Error(format!("Cannot move {name} {}", item.id.0)));
        }
        Ok(Destination { parent, beside, how })
    })
}

/// Moves the object `id` to `to` with `node.move`, its index worked out once it has left its place.
fn place(h: &Host, doc: Doc, id: NodeId, to: &Destination) -> std::result::Result<(), HostError> {
    let index = with_doc(h, doc, |_, st| {
        let d = &st.doc;
        let from = d.position(id);
        let count = d.children(Some(to.parent)).map_or(0, Vec::len) - usize::from(from.is_some_and(|(p, ..)| p == Some(to.parent)));
        let clips = d.node(to.parent).is_some_and(crate::doc::Node::clips) && count > 0;
        let at = match (to.how, to.beside) {
            (Placement::Top, _) => count,
            (Placement::Bottom, _) => 0,
            (how, Some(b)) => {
                let (_, mut i, _) = d.position(b).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
                if let Some((Some(p), j, _)) = from
                    && p == to.parent
                    && j < i
                {
                    i -= 1;
                }
                if how == Placement::Above { i + 1 } else { i }
            }
            (_, None) => count,
        };
        Ok::<_, HostError>(if clips { at.max(1) } else { at })
    })?;
    let now = with_doc(h, doc, |_, st| st.doc.position(id));
    if now.is_some_and(|(p, i, _)| p == Some(to.parent) && i == index) {
        return Ok(());
    }
    on_doc(h, doc.uid, true, |h| command(h, "node.move", &json!({"id": id.0, "parent": to.parent.0, "index": index})))?;
    Ok(())
}

/// `move(relativeObject, insertionLocation?)`: moves the item into or beside `relativeObject`, a
/// page item or a layer of its document (`insertionLocation` defaults to `PLACEATBEGINNING`). An
/// object that is hidden or locked, in a hidden or locked layer or group, or on a locked artboard
/// is refused. Moving an object directly above or below itself leaves it where it is, and a compound
/// path whose last path moves out stays, empty.
fn item_move(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    if matches!(arg(a, 1), Data::Undefined) {
        return Err(HostError::Type("move needs a relative object".into()));
    }
    let to = destination(h, item, arg(a, 1), arg(a, 2))?;
    if to.beside != Some(item.id) {
        place(h, item.doc, item.id, &to)?;
    }
    Ok(arg(a, 0).clone())
}

/// The error of `duplicate` on a path inside a compound path.
pub const COMPOUND_MEMBER: &str = "Duplicating a path inside a compound path is not available yet";

/// The error of `zOrder` on a path inside a compound path.
pub const COMPOUND_MEMBER_ORDER: &str = "Reordering a path inside a compound path is not available yet";

/// Whether `item` is a path inside a compound path.
fn in_compound(h: &Host, item: Item) -> bool {
    with_doc(h, item.doc, |_, st| st.doc.parent_of(item.id).and_then(|p| st.doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. })))
}

/// `duplicate(relativeObject?, insertionLocation?)`: a copy of the item, directly above it as
/// Edit › Duplicate makes one, or into or beside `relativeObject` as `move` puts it. → the copy.
/// When `node.move` refuses the place, the copy and every change it made are put back, and the
/// error is thrown.
fn item_duplicate(h: &Host, a: &[Data]) -> Answer {
    let item = this_item(h, a)?;
    if in_compound(h, item) {
        return Err(HostError::Error(COMPOUND_MEMBER.into()));
    }
    // `edit.duplicate` copies only what it can select.
    let refused = with_doc(h, item.doc, |_, st| {
        (!editable(&st.doc, item.id) || on_locked_artboard(h, st, item.id)).then(|| st.doc.node(item.id).and_then(typename).unwrap_or("page item"))
    });
    if let Some(name) = refused {
        return Err(HostError::Error(format!("Cannot duplicate {name} {}", item.id.0)));
    }
    let to = match arg(a, 1) {
        Data::Undefined => {
            if !matches!(arg(a, 2), Data::Undefined) {
                return Err(HostError::Type("duplicate needs a relative object for an insertionLocation".into()));
            }
            None
        }
        target => Some(destination(h, item, target, arg(a, 2))?),
    };
    let id = atomically(h, |h| {
        let (_, made) = on_doc(h, item.doc.uid, true, |h| command_value(h, "edit.duplicate", &json!({"ids": [item.id.0]})))?;
        let id = made["ids"].get(0).and_then(Value::as_u64).ok_or_else(|| HostError::Error("the copy was not made".into()))?;
        if let Some(to) = to {
            place(h, item.doc, NodeId(id), &to)?;
        }
        Ok(id)
    })?;
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == item.doc.uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
    Ok(item_ref(h, st, NodeId(id)))
}

/// `zOrder(method)`: one of the four Object › Arrange commands for the item. A path inside a
/// compound path is refused.
fn item_z_order(h: &Host, a: &[Data]) -> Answer {
    let id = match arg(a, 1) {
        Data::String(s) if s == "ZOrderMethod.BRINGTOFRONT" => "object.arrange.bringToFront",
        Data::String(s) if s == "ZOrderMethod.BRINGFORWARD" => "object.arrange.bringForward",
        Data::String(s) if s == "ZOrderMethod.SENDBACKWARD" => "object.arrange.sendBackward",
        Data::String(s) if s == "ZOrderMethod.SENDTOBACK" => "object.arrange.sendToBack",
        _ => return Err(HostError::Type("zOrder needs a ZOrderMethod value".into())),
    };
    let item = this_item(h, a)?;
    if in_compound(h, item) {
        return Err(HostError::Error(COMPOUND_MEMBER_ORDER.into()));
    }
    let (before, after) = with_doc(h, item.doc, |_, st| {
        let at = st.doc.position(item.id);
        let count = at.and_then(|(p, ..)| st.doc.children(p).map(Vec::len)).unwrap_or(0);
        (at.map(|(_, i, _)| i), count)
    });
    let Some(i) = before else { return Err(HostError::Error(STALE_ITEM.into())) };
    let top = i + 1 >= after;
    let unchanged = match id {
        "object.arrange.bringToFront" | "object.arrange.bringForward" => top,
        _ => i == 0,
    };
    if !unchanged {
        edit_item(h, item, id, &json!({"ids": [item.id.0]}))?;
    }
    Ok(Data::Undefined)
}

fn layer_name(h: &Host, a: &[Data]) -> Answer {
    let Some(handle) = handle_of(arg(a, 0), "Layer") else { return Err(HostError::Type("this is not a Layer".into())) };
    let [run, uid, high, low] = handle.as_slice() else { return Err(HostError::Type("this is not a Layer".into())) };
    let doc = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
    let id = id_of(*high, *low).unwrap_or(NodeId(u64::MAX));
    with_doc(h, doc, |_, st| st.doc.node(id).map(|n| text(n.name.clone().unwrap_or_default())).ok_or_else(|| HostError::Error(STALE_ITEM.into())))
}

fn layer_typename(h: &Host, a: &[Data]) -> Answer {
    layer_name(h, a)?;
    Ok(text("Layer"))
}

/// `toString()` of an object with a name: `[Class name]`, `[PathItem ]` for an object with no name.
fn named_text(class: &str, name: Answer) -> Answer {
    match name? {
        Data::String(n) => Ok(text(format!("[{class} {n}]"))),
        _ => Ok(text(format!("[{class} ]"))),
    }
}

fn item_to_string(h: &Host, a: &[Data]) -> Answer {
    read_item(h, a, |_, n| Ok(text(format!("[{} {}]", typename(n).unwrap_or("PageItem"), n.name.as_deref().unwrap_or("")))))
}

fn doc_to_string(h: &Host, a: &[Data]) -> Answer {
    named_text("Document", doc_name(h, a))
}

fn layer_to_string(h: &Host, a: &[Data]) -> Answer {
    named_text("Layer", layer_name(h, a))
}

/// `toString()` of an object without a name: `[Class]`.
fn class_text(name: &'static str) -> HostMember {
    HostMember::Method(Rc::new(move |_| Ok(text(format!("[{name}]")))))
}

fn typename_of(name: &'static str) -> HostMember {
    HostMember::Get(Rc::new(move |_| Ok(text(name))))
}

fn class(name: &str, parent: Option<&str>, members: Vec<(String, HostMember)>, index: Option<HostFunction>) -> HostClass {
    HostClass { name: name.into(), parent: parent.map(Into::into), members, index, construct: None }
}

fn named(items: Vec<(&str, HostMember)>) -> Vec<(String, HostMember)> {
    items.into_iter().map(|(n, m)| (n.to_string(), m)).collect()
}

/// The enumerations, as frozen objects whose members are `"Name.MEMBER"` strings.
const ENUMS: &[(&str, &[&str])] = &[
    ("ElementPlacement", &["INSIDE", "PLACEAFTER", "PLACEATBEGINNING", "PLACEATEND", "PLACEBEFORE"]),
    ("ZOrderMethod", &["BRINGFORWARD", "SENDBACKWARD", "BRINGTOFRONT", "SENDTOBACK"]),
    ("RulerUnits", &["Centimeters", "Qs", "Inches", "Pixels", "Millimeters", "Unknown", "Picas", "Points"]),
    ("DocumentColorSpace", &["CMYK", "RGB"]),
    ("CoordinateSystem", &["ARTBOARDCOORDINATESYSTEM", "DOCUMENTCOORDINATESYSTEM"]),
    ("DocumentArtboardLayout", &["Column", "GridByCol", "GridByRow", "RLGridByCol", "RLGridByRow", "RLRow", "Row"]),
    ("DocumentPresetType", &["BasicCMYK", "BasicRGB", "Mobile", "Print", "Video", "Web"]),
    ("DocumentPreviewMode", &["DefaultPreview", "OverprintPreview", "PixelPreview"]),
    ("DocumentRasterResolution", &["ScreenResolution", "HighResolution", "MediumResolution"]),
    (
        "DocumentTransparencyGrid",
        &[
            "TransparencyGridBlue",
            "TransparencyGridDark",
            "TransparencyGridGreen",
            "TransparencyGridLight",
            "TransparencyGridMedium",
            "TransparencyGridNone",
            "TransparencyGridOrange",
            "TransparencyGridPurple",
            "TransparencyGridRed",
        ],
    ),
];

/// Adds `app`, its classes and the enumerations to `realm`; their members act for the run `link`
/// holds.
pub(super) fn install(realm: &mut Realm, link: &Link) -> std::result::Result<(), RunError> {
    for (name, members) in ENUMS {
        let items = members.iter().map(|m| (m.to_string(), HostItem::Value(text(format!("{name}.{m}"))))).collect();
        realm.define_host_object(name, items)?;
    }
    let l = link;
    let mut application = named(vec![
        ("typename", typename_of("Application")),
        ("documents", get(l, app_documents)),
        ("activeDocument", property(l, app_active, app_set_active)),
        ("selection", get(l, app_selection)),
        ("coordinateSystem", property(l, app_coords, app_set_coords)),
        ("getIsFileOpen", method(l, app_is_open)),
        ("toString", class_text("Application")),
    ]);
    application.push(later(l, "Application", "beep"));
    let preset = {
        let l = link.clone();
        let f: HostFunction = Rc::new(move |_: Vec<Data>| {
            let h = current(&l)?;
            h.charge_call(64)?;
            new_preset(&h)
        });
        let mut members = vec![("typename".to_string(), typename_of("DocumentPreset")), ("toString".to_string(), class_text("DocumentPreset"))];
        members.extend(PRESET_FIELDS.iter().map(|k| (k.to_string(), preset_field(link, k))));
        HostClass { name: "DocumentPreset".into(), parent: None, members, index: None, construct: Some(f) }
    };
    let mut classes = vec![
        class("Application", None, application, None),
        preset,
        class(
            "Documents",
            None,
            named(vec![
                ("typename", typename_of("Documents")),
                ("length", get(l, docs_length)),
                ("parent", get(l, app_ref)),
                ("index", method(l, docs_index)),
                ("getByName", method(l, docs_by_name)),
                ("add", method(l, docs_add)),
                ("addDocument", method(l, docs_add_document)),
                ("toString", class_text("Documents")),
            ]),
            Some(function(l, docs_at)),
        ),
    ];
    let mut document = named(vec![
        ("typename", get(l, doc_typename)),
        ("parent", get(l, doc_parent)),
        ("name", get(l, doc_name)),
        ("width", get(l, doc_width)),
        ("height", get(l, doc_height)),
        ("rulerUnits", get(l, doc_units)),
        ("documentColorSpace", get(l, doc_color_space)),
        ("rulerOrigin", property(l, doc_ruler_origin, doc_set_ruler_origin)),
        ("convertCoordinate", method(l, doc_convert)),
        ("pageItems", get(l, doc_page_items)),
        ("selection", property(l, doc_selection, doc_set_selection)),
        ("toString", method(l, doc_to_string)),
    ]);
    document.extend(
        ["layers", "artboards", "pathItems", "groupItems", "textFrames", "swatches", "symbols", "views", "activeLayer", "saveAs", "close"]
            .into_iter()
            .map(|m| later(l, "Document", m)),
    );
    classes.push(class("Document", None, document, None));
    classes.push(class(
        "PageItems",
        None,
        named(vec![
            ("typename", get(l, items_typename)),
            ("length", get(l, items_length)),
            ("parent", get(l, items_parent)),
            ("index", method(l, items_index)),
            ("getByName", method(l, items_by_name)),
            ("removeAll", method(l, items_remove_all)),
            ("toString", class_text("PageItems")),
        ]),
        Some(function(l, items_at)),
    ));
    let mut page_item = named(vec![
        ("typename", get(l, item_typename)),
        ("parent", get(l, item_parent)),
        ("name", property(l, item_name, item_set_name)),
        ("note", property(l, item_note, item_set_note)),
        ("hidden", property(l, item_hidden, item_set_hidden)),
        ("locked", property(l, item_locked, item_set_locked)),
        ("selected", property(l, item_selected, item_set_selected)),
        ("opacity", property(l, item_opacity, item_set_opacity)),
        ("position", property(l, item_position, item_set_position)),
        ("left", property(l, item_left, item_set_left)),
        ("top", property(l, item_top, item_set_top)),
        ("width", property(l, item_width, item_set_width)),
        ("height", property(l, item_height, item_set_height)),
        ("geometricBounds", get(l, item_geometric)),
        ("visibleBounds", get(l, item_visible)),
        ("controlBounds", get(l, item_visible)),
        ("editable", get(l, item_editable)),
        ("zOrderPosition", get(l, item_z_order_position)),
        ("duplicate", method(l, item_duplicate)),
        ("move", method(l, item_move)),
        ("remove", method(l, item_remove)),
        ("zOrder", method(l, item_z_order)),
        ("toString", method(l, item_to_string)),
    ]);
    page_item.extend(
        [
            "artworkKnockout",
            "blendingMode",
            "isIsolated",
            "layer",
            "pixelAligned",
            "sliced",
            "tags",
            "uRL",
            "uuid",
            "visibilityVariable",
            "wrapInside",
            "wrapOffset",
            "wrapped",
            "bringInPerspective",
            "resize",
            "rotate",
            "transform",
            "translate",
        ]
        .into_iter()
        .map(|m| later(l, "PageItem", m)),
    );
    classes.push(class("PageItem", None, page_item, None));
    classes.extend(ITEM_CLASSES.iter().map(|c| class(c, Some("PageItem"), Vec::new(), None)));
    classes.push(class(
        "Layer",
        None,
        named(vec![("typename", get(l, layer_typename)), ("name", get(l, layer_name)), ("toString", method(l, layer_to_string))]),
        None,
    ));
    realm.define_host_classes(classes, vec![("app".into(), host_ref("Application", &[]))])?;
    // The application's members a script may name without `app.`.
    realm.define_host_globals(vec![
        ("activeDocument".into(), function(l, app_active)),
        ("documents".into(), function(l, app_documents)),
        ("selection".into(), function(l, app_selection)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoring_the_selection_falls_back_only_on_a_refusal() {
        assert_eq!(done_unless_refused(Ok(Data::Undefined)), Ok(true));
        assert_eq!(done_unless_refused(Err(HostError::Error("refused".into()))), Ok(false));
        // An internal failure or a stop ends the run instead of being taken for a refusal.
        assert_eq!(done_unless_refused(Err(HostError::Abort("canceled".into()))), Err(HostError::Abort("canceled".into())));
        assert_eq!(done_unless_refused(Err(HostError::Range("too large".into()))), Err(HostError::Range("too large".into())));
    }

    #[test]
    fn page_item_lists_are_kept_for_several_documents() {
        let docs: Vec<Arc<Document>> = (0..=ITEM_LISTS).map(|_| Arc::new(Document::new(100.0, 100.0))).collect();
        let ids = |n: u64| Rc::new(vec![NodeId(n)]);
        let mut lists = Vec::new();
        keep_items(&mut lists, (1, docs[0].clone(), ids(1)));
        keep_items(&mut lists, (2, docs[1].clone(), ids(2)));
        // Reading two documents in turn keeps both lists.
        assert_eq!(kept_items(&lists, 1, &docs[0]), Some(ids(1)));
        assert_eq!(kept_items(&lists, 2, &docs[1]), Some(ids(2)));
        // A changed document has no kept list, and its new list replaces the old one.
        let changed = Arc::new(Document::new(100.0, 100.0));
        assert_eq!(kept_items(&lists, 1, &changed), None);
        keep_items(&mut lists, (1, changed.clone(), ids(3)));
        assert_eq!(lists.len(), 2);
        assert_eq!(kept_items(&lists, 1, &changed), Some(ids(3)));
        // Past the limit, the list kept longest ago goes.
        for (uid, d) in (10..).zip(&docs[2..]) {
            keep_items(&mut lists, (uid, d.clone(), ids(uid)));
        }
        assert_eq!(lists.len(), ITEM_LISTS);
        assert_eq!(kept_items(&lists, 2, &docs[1]), None);
        assert_eq!(kept_items(&lists, 1, &changed), Some(ids(3)));
    }
}
