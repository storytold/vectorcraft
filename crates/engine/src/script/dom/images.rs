//! Placed and raster art in the script object model: `Document.placedItems` and
//! `Document.rasterItems`, and the members of `PlacedItem` and `RasterItem`, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PlacedItems/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PlacedItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/RasterItems/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/RasterItem/> describe them.
//!
//! A `PlacedItem` is an image linked to its file, and a `RasterItem` an image whose pixels are in
//! the document. Placing runs `file.place` with `link`, relinking `links.relink` and embedding
//! `links.embed`, so each joins the run's transaction.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `PlacedItems.add()` gives a placed item with no file. Its `file` may be assigned once, which
//!   places the file linked; after that, and on every other placed item, `file` is read-only and
//!   `relink(file)` changes it. Every other member of an item with no file throws
//!   [`FILE_REQUIRED`]; an item with no file is not in the document and is dropped when the run
//!   ends.
//! - A file to place or relink is a `File` the script may read ([`crate::file_access::ScriptGate`])
//!   that holds an image, of at most [`MAX_PLACED_BYTES`]. Its size is read before any of it is.
//! - `PlacedItem.file` throws `Script file access denied`, without the path, when the script may
//!   not read the linked file.
//! - `embed()` turns the placed item into a raster item with the same id: the placed item is stale
//!   after it, and `rasterItems` lists the raster item. An image whose file is missing throws
//!   [`MISSING_LINK`] and stays linked.
//! - `bitsPerChannel`, `channels`, `imageColorSpace` and `transparent` come from the header of the
//!   image's encoded pixels; `transparent` is true for an image with an alpha channel or a
//!   transparent palette entry. `RasterItem.file` is `null`, `embedded` is true and takes only
//!   `true`, and `status` is `RasterLinkState.NODATA`, since a raster item has no linked file.
//! - `matrix` maps the image's pixels (row 0 at the top) to script coordinates, as a record of
//!   `mValueA` to `mValueTY`; a distorted image has none. Assigning it runs `object.transform`; a
//!   matrix with no area or one that puts a corner of the image outside the canvas is refused.
//!   `boundingBox` is the pixel rectangle at one point a pixel, in the run's coordinate system.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::json;
use vectorcraft_geom::{Affine, Point};
use vectorcraft_js::{Data, HostClass, HostError, HostMember, file_ref, file_ref_path, host_ref, host_ref_parts};

use super::{
    Answer, Coords, Doc, Host, Item, Link, NOT_FOUND, ON_LOCKED_ARTBOARD, STALE_ITEM, all_editable, arg, class, command, doc_of, doc_ref, edit_item,
    editable, find_doc, float, frame, get, item_of, item_ref, later, method, named, num, on_doc, page_items, property, text, typename_of, whole,
    with_doc,
};
use crate::{DocState, Document, NodeId, NodeKind};

/// The error of a member of a `PlacedItems.add()` result whose `file` was not assigned.
pub const FILE_REQUIRED: &str = "PlacedItem.file is required";
/// The error of assigning `file` to a placed item that has one.
pub const FILE_READ_ONLY: &str = "PlacedItem.file is read-only: use relink";
/// The error of embedding an image whose linked file cannot be found.
pub const MISSING_LINK: &str = "Linked image is missing";
/// The error of placing or relinking a file that is not an image.
pub const NOT_AN_IMAGE: &str = "The file is not an image";
/// The largest file a script places or relinks, in bytes.
pub const MAX_PLACED_BYTES: u64 = 256 << 20;
/// The most `PlacedItems.add()` results one run keeps.
pub const MAX_PENDING: usize = 1024;
/// The error of the `matrix` of an image drawn distorted.
const DISTORTED: &str = "the image is distorted and has no matrix";
/// The smallest determinant a `matrix` takes, in square points a square pixel.
const MIN_DETERMINANT: f64 = 1e-12;
/// The members of a `Matrix` record, in the order of an affine matrix's six numbers.
const MATRIX_KEYS: [&str; 6] = ["mValueA", "mValueB", "mValueC", "mValueD", "mValueTX", "mValueTY"];

/// The enumerations the members here give values of, as the help page
/// <https://ai-scripting.docsforadobe.dev/jsobjref/scripting-constants/> lists them.
pub(super) const ENUMS: &[(&str, &[&str])] = &[
    ("ImageColorSpace", &["CMYK", "DeviceN", "Grayscale", "Indexed", "LAB", "RGB", "Separation"]),
    ("RasterLinkState", &["DATAFROMFILE", "DATAMODIFIED", "NODATA"]),
];

/// The classes this module defines, in place of the plain page item classes.
pub(super) const CLASSES: &[&str] = &["PlacedItem", "RasterItem"];

/// A document's uid, the document as it was, and the ids of its linked and embedded images then.
type Lists = (u64, Arc<Document>, Rc<Vec<NodeId>>, Rc<Vec<NodeId>>);

/// What a run keeps for placed and raster items: the `PlacedItems.add()` results and the last
/// lists of a document's images.
#[derive(Default)]
pub(super) struct State {
    pending: RefCell<Vec<Pending>>,
    lists: RefCell<Option<Lists>>,
}

/// A `PlacedItems.add()` result: its document, and its image once `file` placed one.
struct Pending {
    uid: u64,
    bound: Option<NodeId>,
}

/// The slot of a `PlacedItems.add()` result in this run, from its handle.
fn slot_of(h: &Host, run: f64, uid: f64, slot: f64) -> Result<usize, HostError> {
    let stale = || HostError::Error(STALE_ITEM.into());
    if whole(run) != Some(h.run) {
        return Err(stale());
    }
    let k = whole(slot).and_then(|k| usize::try_from(k).ok()).ok_or_else(stale)?;
    let uid = whole(uid).ok_or_else(stale)?;
    match h.dom.images.pending.borrow().get(k) {
        Some(p) if p.uid == uid => Ok(k),
        _ => Err(stale()),
    }
}

/// The id of the image a `PlacedItems.add()` result placed.
pub(super) fn pending_id(h: &Host, run: f64, uid: f64, slot: f64) -> Result<NodeId, HostError> {
    let k = slot_of(h, run, uid, slot)?;
    match h.dom.images.pending.borrow().get(k).and_then(|p| p.bound) {
        Some(id) => Ok(id),
        None => Err(HostError::Error(FILE_REQUIRED.into())),
    }
}

/// The handle's run, uid and slot when `d` is a `PlacedItems.add()` result. Its handle has five
/// numbers, where a page item's has four.
fn pending_handle(d: &Data) -> Option<[f64; 3]> {
    match host_ref_parts(d) {
        Some(("PlacedItem", handle)) => match handle.as_slice() {
            [run, uid, _, _, slot] => Some([*run, *uid, *slot]),
            _ => None,
        },
        _ => None,
    }
}

/// The page item handle (run, uid and the two halves of the id) of the image a
/// `PlacedItems.add()` result `d` placed, or `None` when `d` is not such a result.
pub(super) fn pending_as_item(h: &Host, d: &Data) -> Result<Option<[f64; 4]>, HostError> {
    let Some([run, uid, slot]) = pending_handle(d) else { return Ok(None) };
    let [high, low] = super::id_parts(pending_id(h, run, uid, slot)?);
    Ok(Some([run, uid, high, low]))
}

/// The ids of a document's linked (`placed`) or embedded images, topmost first, in the order of
/// `pageItems`.
fn images(h: &Host, st: &DocState, placed: bool) -> Result<Rc<Vec<NodeId>>, HostError> {
    let pick = |l: &Lists| if placed { l.2.clone() } else { l.3.clone() };
    if let Some(l) = &*h.dom.images.lists.borrow()
        && l.0 == st.uid
        && Arc::ptr_eq(&l.1, &st.doc)
    {
        return Ok(pick(l));
    }
    let all = page_items(h, st);
    h.charge_call(all.len() as u64)?;
    let (mut linked, mut embedded) = (Vec::new(), Vec::new());
    for id in all.iter() {
        if let Some(NodeKind::Image(im)) = st.doc.node(*id).map(|n| &n.kind) {
            if im.link.is_some() { linked.push(*id) } else { embedded.push(*id) }
        }
    }
    let l = (st.uid, st.doc.clone(), Rc::new(linked), Rc::new(embedded));
    let out = pick(&l);
    *h.dom.images.lists.borrow_mut() = Some(l);
    Ok(out)
}

fn field<'a>(d: &'a Data, k: &str) -> Option<&'a Data> {
    match d {
        Data::Object(o) => o.iter().find(|(n, _)| n == k).map(|(_, v)| v),
        _ => None,
    }
}

fn id_value(d: Option<&Data>) -> Option<NodeId> {
    match d {
        Some(Data::Number(n)) => whole(*n).map(NodeId),
        Some(Data::Array(a)) => id_value(a.first()),
        _ => None,
    }
}

// ---- PlacedItems and RasterItems ----

fn collection(placed: bool) -> &'static str {
    if placed { "PlacedItems" } else { "RasterItems" }
}

fn list_doc(h: &Host, a: &[Data], placed: bool) -> Result<Doc, HostError> {
    doc_of(h, arg(a, 0), collection(placed))
}

fn list_length(h: &Host, a: &[Data], placed: bool) -> Answer {
    let doc = list_doc(h, a, placed)?;
    let n = with_doc(h, doc, |_, st| images(h, st, placed).map(|l| l.len()))?;
    Ok(num(float(n as u64)))
}

fn list_at(h: &Host, a: &[Data], placed: bool) -> Answer {
    let doc = list_doc(h, a, placed)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    with_doc(h, doc, |_, st| Ok(images(h, st, placed)?.get(i).map_or(Data::Undefined, |id| item_ref(h, st, *id))))
}

fn list_by_name(h: &Host, a: &[Data], placed: bool) -> Answer {
    let doc = list_doc(h, a, placed)?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        let ids = images(h, st, placed)?;
        h.charge_call(ids.len() as u64)?;
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| item_ref(h, st, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn list_index(h: &Host, a: &[Data], placed: bool) -> Answer {
    match arg(a, 1) {
        Data::String(_) => list_by_name(h, a, placed),
        Data::Number(_) => match list_at(h, a, placed)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `removeAll()`: deletes every image of the collection in one command. A hidden or locked one, or
/// one in a hidden or locked layer or group, stops it before anything is deleted, and so does one on
/// a locked artboard.
fn list_remove_all(h: &Host, a: &[Data], placed: bool) -> Answer {
    let doc = list_doc(h, a, placed)?;
    let (ids, refused, on_locked) = with_doc(h, doc, |_, st| {
        let ids = images(h, st, placed)?;
        let whole = |id: &NodeId| editable(&st.doc, *id) && st.doc.node(*id).is_some_and(all_editable);
        let refused: Vec<u64> = ids.iter().filter(|i| !whole(i)).map(|i| i.0).take(20).collect();
        let locks = st.doc.locked_artboards();
        let on_locked: Vec<u64> = ids.iter().filter(|i| locks.holds_id(**i)).map(|i| i.0).take(20).collect();
        Ok::<_, HostError>((ids, refused, on_locked))
    })?;
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

/// `PlacedItems.add()`: a placed item with no file yet, which assigning `file` places.
fn placed_add(h: &Host, a: &[Data]) -> Answer {
    let doc = list_doc(h, a, true)?;
    let mut pending = h.dom.images.pending.borrow_mut();
    if pending.len() >= MAX_PENDING {
        return Err(HostError::Range(format!("a run makes at most {MAX_PENDING} placed items with PlacedItems.add")));
    }
    pending.push(Pending { uid: doc.uid, bound: None });
    Ok(host_ref("PlacedItem", &[float(h.run), float(doc.uid), 0.0, 0.0, float(pending.len() as u64 - 1)]))
}

macro_rules! both {
    ($($placed:ident, $raster:ident => $f:ident;)*) => {
        $(
            fn $placed(h: &Host, a: &[Data]) -> Answer {
                $f(h, a, true)
            }
            fn $raster(h: &Host, a: &[Data]) -> Answer {
                $f(h, a, false)
            }
        )*
    };
}

both! {
    placed_length, raster_length => list_length;
    placed_at, raster_at => list_at;
    placed_by_name, raster_by_name => list_by_name;
    placed_index, raster_index => list_index;
    placed_remove_all, raster_remove_all => list_remove_all;
}

fn placed_parent(h: &Host, a: &[Data]) -> Answer {
    Ok(doc_ref(h, list_doc(h, a, true)?.uid))
}

fn raster_parent(h: &Host, a: &[Data]) -> Answer {
    Ok(doc_ref(h, list_doc(h, a, false)?.uid))
}

fn doc_placed(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("PlacedItems", &[float(h.run), float(doc.uid)]))
}

fn doc_raster(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("RasterItems", &[float(h.run), float(doc.uid)]))
}

// ---- files ----

/// Whether the script may read `path` now by its own read scope and the read roots
/// ([`crate::file_access::ScriptGate`]), without the open documents' files, or the files they link
/// to, that a command may also read.
fn may_read(h: &Host, path: &str) -> bool {
    let gate = h.disk.borrow().gate(Vec::new(), crate::file_access::LinkedFiles::default());
    crate::file_access::with_script_gate(gate, || crate::file_access::check_read(path)).is_ok()
}

/// The real path of the `File` `d` gives as an image to place or relink: one the script may read
/// ([`super::super::files::Disk::open_target`], as `app.open` reads its file), a regular file of
/// at most [`MAX_PLACED_BYTES`] that holds an image. Its size is read before any of it is, and no
/// error names the path.
fn image_path(h: &Host, d: &Data, what: &str) -> Result<String, HostError> {
    let path = file_ref_path(d).ok_or_else(|| HostError::Type(format!("{what} must be a File")))?;
    if path.len() > super::super::files::MAX_PATH_CHARS {
        return Err(HostError::Range(super::super::files::PATH_TOO_LONG.into()));
    }
    let unc = path.starts_with("\\\\") || path.starts_with("//");
    if path.is_empty() || path.contains('\0') || unc || path.split(['/', '\\']).any(|c| c == "..") {
        return Err(HostError::Error(super::super::files::ACCESS_DENIED.into()));
    }
    let real = h.disk.borrow().open_target(path).map_err(|m| HostError::Error(m.into()))?;
    let meta = std::fs::metadata(&real).map_err(|_| HostError::Error(super::super::files::NOT_FOUND.into()))?;
    if !meta.is_file() {
        return Err(HostError::Error(super::super::files::NOT_REGULAR.into()));
    }
    if meta.len() > MAX_PLACED_BYTES {
        return Err(HostError::Range(format!("{what} is larger than {} MiB", MAX_PLACED_BYTES >> 20)));
    }
    let path = crate::cmd::findfiles::plain(real).to_string_lossy().into_owned();
    // What the file holds, read the way `file.place` reads it, before the document changes.
    let info = command(h, "file.place.info", &json!({"path": path, "link": true}))?;
    if field(&info, "pixelWidth").is_none() {
        return Err(HostError::Error(NOT_AN_IMAGE.into()));
    }
    Ok(path)
}

// ---- PlacedItem ----

/// The item `this` names when it is a `cls` (`PlacedItem` or `RasterItem`).
fn this_of(h: &Host, a: &[Data], cls: &str) -> Result<Item, HostError> {
    match host_ref_parts(arg(a, 0)) {
        Some((c, _)) if c == cls => item_of(h, arg(a, 0)),
        _ => Err(HostError::Type(format!("this is not a {cls}"))),
    }
}

/// Reads the image of the `cls` `this` names with `f`.
fn read_image(h: &Host, a: &[Data], cls: &str, f: impl FnOnce(&DocState, &crate::doc::Node, &crate::doc::ImageObject) -> Answer) -> Answer {
    let item = this_of(h, a, cls)?;
    with_doc(h, item.doc, |_, st| match st.doc.node(item.id) {
        Some(n) => match &n.kind {
            NodeKind::Image(im) => f(st, n, im),
            _ => Err(HostError::Error(STALE_ITEM.into())),
        },
        None => Err(HostError::Error(STALE_ITEM.into())),
    })
}

fn placed_file(h: &Host, a: &[Data]) -> Answer {
    if let Some([run, uid, slot]) = pending_handle(arg(a, 0)) {
        let k = slot_of(h, run, uid, slot)?;
        if h.dom.images.pending.borrow().get(k).is_some_and(|p| p.bound.is_none()) {
            return Ok(Data::Null);
        }
    }
    let path = read_image(h, a, "PlacedItem", |_, _, im| Ok(text(im.link.as_ref().map(|l| l.path.clone()).unwrap_or_default())))?;
    let Data::String(path) = path else { return Ok(Data::Null) };
    if !may_read(h, &path) {
        return Err(HostError::Error(super::super::files::ACCESS_DENIED.into()));
    }
    Ok(file_ref(&path))
}

/// `file = File`: places the file, linked, for a `PlacedItems.add()` result that has none.
fn placed_set_file(h: &Host, a: &[Data]) -> Answer {
    let Some([run, uid, slot]) = pending_handle(arg(a, 0)) else {
        if host_ref_parts(arg(a, 0)).is_some_and(|(c, _)| c == "PlacedItem") {
            this_of(h, a, "PlacedItem")?;
            return Err(HostError::Error(FILE_READ_ONLY.into()));
        }
        return Err(HostError::Type("this is not a PlacedItem".into()));
    };
    let k = slot_of(h, run, uid, slot)?;
    if h.dom.images.pending.borrow().get(k).is_some_and(|p| p.bound.is_some()) {
        this_of(h, a, "PlacedItem")?;
        return Err(HostError::Error(FILE_READ_ONLY.into()));
    }
    let doc = find_doc(h, run, uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let path = image_path(h, arg(a, 1), "PlacedItem.file")?;
    let placed = on_doc(h, doc.uid, true, |h| command(h, "file.place", &json!({"path": path, "link": true})))?;
    let ids: Vec<u64> = match field(&placed, "ids") {
        Some(Data::Array(ids)) => ids.iter().filter_map(|d| id_value(Some(d))).map(|i| i.0).collect(),
        _ => Vec::new(),
    };
    let id = id_value(field(&placed, "image")).or_else(|| id_value(field(&placed, "ids")));
    let linked = |id: NodeId| with_doc(h, doc, |_, st| matches!(st.doc.node(id).map(|n| &n.kind), Some(NodeKind::Image(im)) if im.link.is_some()));
    match id.filter(|id| linked(*id)) {
        Some(id) => {
            if let Some(p) = h.dom.images.pending.borrow_mut().get_mut(k) {
                p.bound = Some(id);
            }
            Ok(Data::Undefined)
        }
        None => {
            if !ids.is_empty() {
                on_doc(h, doc.uid, true, |h| command(h, "edit.clear", &json!({"ids": ids})))?;
            }
            Err(HostError::Error(NOT_AN_IMAGE.into()))
        }
    }
}

/// `relink(file)`: `links.relink` of the item to the image in `file`.
fn placed_relink(h: &Host, a: &[Data]) -> Answer {
    let item = this_of(h, a, "PlacedItem")?;
    let path = image_path(h, arg(a, 1), "relink's file")?;
    edit_item(h, item, "links.relink", &json!({"ids": [item.id.0], "path": path}))?;
    Ok(Data::Undefined)
}

/// `embed()`: `links.embed` of the item, which becomes a raster item.
fn placed_embed(h: &Host, a: &[Data]) -> Answer {
    let item = this_of(h, a, "PlacedItem")?;
    let info = on_doc(h, item.doc.uid, false, |h| command(h, "links.info", &json!({"id": item.id.0})))?;
    if matches!(field(&info, "status"), Some(Data::String(s)) if s == "missing") {
        return Err(HostError::Error(MISSING_LINK.into()));
    }
    let r = edit_item(h, item, "links.embed", &json!({"ids": [item.id.0]}))?;
    let embedded = match field(&r, "embedded") {
        Some(Data::Array(ids)) => ids.iter().any(|d| id_value(Some(d)) == Some(item.id)),
        _ => false,
    };
    if !embedded {
        return Err(HostError::Error(MISSING_LINK.into()));
    }
    Ok(Data::Undefined)
}

// ---- both ----

/// `matrix`: the image's pixels to script coordinates.
fn matrix(h: &Host, a: &[Data], cls: &str) -> Answer {
    read_image(h, a, cls, |st, n, im| {
        if n.is_distorted_image() {
            return Err(HostError::Error(DISTORTED.into()));
        }
        let f = frame(st)?;
        let [a0, b0, c0, d0, e0, f0] = im.xf.as_coeffs();
        let m = match h.dom.coords.get() {
            Coords::Document => [a0, -b0, c0, -d0, e0 - f.left0, f.bottom0 - f0],
            Coords::Artboard => [a0, b0, c0, d0, e0 - f.left, f0 - f.top],
        };
        let mut items = vec![("typename".to_string(), text("Matrix"))];
        items.extend(MATRIX_KEYS.iter().zip(m).map(|(k, v)| (k.to_string(), num(v))));
        Ok(Data::Object(items))
    })
}

/// The six numbers of the `Matrix` record `d`, `mValueA` to `mValueTY`, each finite.
fn matrix_value(d: &Data) -> Result<[f64; 6], HostError> {
    let mut m = [0.0; 6];
    for (k, out) in MATRIX_KEYS.iter().zip(m.iter_mut()) {
        *out = match field(d, k) {
            Some(Data::Number(n)) if n.is_finite() => *n,
            _ => return Err(HostError::Type(format!("matrix must be a Matrix with a finite {k}"))),
        };
    }
    Ok(m)
}

/// Whether `m` gives a pixel an area of at least [`MIN_DETERMINANT`].
fn area_of(m: Affine) -> bool {
    let det = m.determinant();
    !det.is_nan() && det.abs() >= MIN_DETERMINANT
}

/// `matrix = Matrix`: `object.transform` of the image by what takes its pixels to the new matrix.
/// A matrix with no area, one that puts a corner of the image outside the canvas, or a distorted
/// image is refused before anything changes; the matrix the image has already changes nothing.
fn set_matrix(h: &Host, a: &[Data], cls: &str) -> Answer {
    let item = this_of(h, a, cls)?;
    let m = matrix_value(arg(a, 1))?;
    let delta = with_doc(h, item.doc, |_, st| {
        let Some((n, im)) = st.doc.node(item.id).and_then(|n| if let NodeKind::Image(im) = &n.kind { Some((n, im)) } else { None }) else {
            return Err(HostError::Error(STALE_ITEM.into()));
        };
        if n.is_distorted_image() {
            return Err(HostError::Error(DISTORTED.into()));
        }
        let f = frame(st)?;
        let target = Affine::new(match h.dom.coords.get() {
            Coords::Document => [m[0], -m[1], m[2], -m[3], m[4] + f.left0, f.bottom0 - m[5]],
            Coords::Artboard => [m[0], m[1], m[2], m[3], m[4] + f.left, m[5] + f.top],
        });
        if !area_of(target) {
            return Err(HostError::Range("the matrix gives the image no area".into()));
        }
        let (w, ht) = (f64::from(im.width), f64::from(im.height));
        let limit = crate::MAX_COORD * 2.0;
        let corners = [(0.0, 0.0), (w, 0.0), (0.0, ht), (w, ht)].map(|(x, y)| target * Point::new(x, y));
        if corners.iter().any(|p| !(p.x.abs() <= limit && p.y.abs() <= limit)) {
            return Err(HostError::Range("the matrix puts the image outside the canvas".into()));
        }
        let old = im.xf;
        if old.as_coeffs().iter().zip(target.as_coeffs()).all(|(o, t)| (o - t).abs() <= 1e-9) {
            return Ok(None);
        }
        if !area_of(old) {
            return Err(HostError::Error("the image has no area to transform".into()));
        }
        Ok(Some((target * old.inverse()).as_coeffs()))
    })?;
    if let Some(d) = delta {
        edit_item(h, item, "object.transform", &json!({"ids": [item.id.0], "matrix": d, "strokes": false}))?;
    }
    Ok(Data::Undefined)
}

/// `boundingBox`: the pixel rectangle at one point a pixel, `[left, top, right, bottom]`.
fn bounding_box(h: &Host, a: &[Data], cls: &str) -> Answer {
    read_image(h, a, cls, |_, _, im| {
        let (w, ht) = (f64::from(im.width), f64::from(im.height));
        Ok(Data::Array(match h.dom.coords.get() {
            Coords::Document => vec![num(0.0), num(ht), num(w), num(0.0)],
            Coords::Artboard => vec![num(0.0), num(0.0), num(w), num(ht)],
        }))
    })
}

fn placed_matrix(h: &Host, a: &[Data]) -> Answer {
    matrix(h, a, "PlacedItem")
}

fn raster_matrix(h: &Host, a: &[Data]) -> Answer {
    matrix(h, a, "RasterItem")
}

fn placed_set_matrix(h: &Host, a: &[Data]) -> Answer {
    set_matrix(h, a, "PlacedItem")
}

fn raster_set_matrix(h: &Host, a: &[Data]) -> Answer {
    set_matrix(h, a, "RasterItem")
}

fn placed_box(h: &Host, a: &[Data]) -> Answer {
    bounding_box(h, a, "PlacedItem")
}

fn raster_box(h: &Host, a: &[Data]) -> Answer {
    bounding_box(h, a, "RasterItem")
}

// ---- RasterItem ----

/// The header of the raster item's encoded pixels.
fn header(h: &Host, a: &[Data], f: impl FnOnce(&craft_image_codecs::RasterInspection) -> Data) -> Answer {
    read_image(h, a, "RasterItem", |st, _, im| {
        let blob = st.doc.images.get(&im.key).ok_or_else(|| HostError::Error("the image's pixels are not in the document".into()))?;
        match craft_image_codecs::inspect_raster(&blob.bytes) {
            Ok(Some(i)) => Ok(f(&i)),
            _ => Err(HostError::Error("the image's header does not describe its pixels".into())),
        }
    })
}

fn raster_bits(h: &Host, a: &[Data]) -> Answer {
    header(h, a, |i| num(f64::from(i.bits_per_sample)))
}

fn raster_channels(h: &Host, a: &[Data]) -> Answer {
    header(h, a, |i| num(f64::from(i.channels)))
}

fn raster_transparent(h: &Host, a: &[Data]) -> Answer {
    use craft_image_codecs::RasterColorType as T;
    header(h, a, |i| Data::Bool(matches!(i.color_type, T::GrayAlpha | T::Rgba | T::CmykAlpha) || i.transparent_index.is_some()))
}

fn raster_color_space(h: &Host, a: &[Data]) -> Answer {
    use craft_image_codecs::RasterColorType as T;
    let space = header(h, a, |i| {
        text(match i.color_type {
            T::Bitmap | T::Gray | T::GrayAlpha => "ImageColorSpace.Grayscale",
            T::Rgb | T::Rgba => "ImageColorSpace.RGB",
            T::Cmyk | T::CmykAlpha => "ImageColorSpace.CMYK",
            T::Indexed => "ImageColorSpace.Indexed",
            T::Lab => "ImageColorSpace.LAB",
            T::Other => "",
        })
    })?;
    match space {
        Data::String(s) if s.is_empty() => Err(HostError::Error("the image's header does not describe its color space".into())),
        d => Ok(d),
    }
}

fn raster_colorized(h: &Host, a: &[Data]) -> Answer {
    read_image(h, a, "RasterItem", |st, n, im| {
        let gray = st.doc.images.get(&im.key).is_some_and(|b| b.gray_source().is_some());
        Ok(Data::Bool(gray && n.extras().colorize.is_some()))
    })
}

fn raster_embedded(h: &Host, a: &[Data]) -> Answer {
    this_of(h, a, "RasterItem")?;
    Ok(Data::Bool(true))
}

/// `embedded = true` changes nothing; a raster item cannot be linked by assigning `false`.
fn raster_set_embedded(h: &Host, a: &[Data]) -> Answer {
    this_of(h, a, "RasterItem")?;
    match arg(a, 1) {
        Data::Bool(true) => Ok(Data::Undefined),
        Data::Bool(false) => Err(HostError::Error("a raster item cannot be linked by setting embedded to false".into())),
        _ => Err(HostError::Type("embedded must be true or false".into())),
    }
}

fn raster_file(h: &Host, a: &[Data]) -> Answer {
    this_of(h, a, "RasterItem")?;
    Ok(Data::Null)
}

fn raster_status(h: &Host, a: &[Data]) -> Answer {
    this_of(h, a, "RasterItem")?;
    Ok(text("RasterLinkState.NODATA"))
}

/// `Document.placedItems` and `Document.rasterItems`.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("placedItems", get(l, doc_placed)), ("rasterItems", get(l, doc_raster))])
}

/// The collections, `PlacedItem` and `RasterItem`, after `PageItem`.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let placed_list = named(vec![
        ("typename", typename_of("PlacedItems")),
        ("length", get(l, placed_length)),
        ("parent", get(l, placed_parent)),
        ("index", method(l, placed_index)),
        ("getByName", method(l, placed_by_name)),
        ("removeAll", method(l, placed_remove_all)),
        ("add", method(l, placed_add)),
    ]);
    let raster_list = named(vec![
        ("typename", typename_of("RasterItems")),
        ("length", get(l, raster_length)),
        ("parent", get(l, raster_parent)),
        ("index", method(l, raster_index)),
        ("getByName", method(l, raster_by_name)),
        ("removeAll", method(l, raster_remove_all)),
    ]);
    let mut placed = named(vec![
        ("file", property(l, placed_file, placed_set_file)),
        ("relink", method(l, placed_relink)),
        ("embed", method(l, placed_embed)),
        ("matrix", property(l, placed_matrix, placed_set_matrix)),
        ("boundingBox", get(l, placed_box)),
    ]);
    placed.extend(super::variables::item_members(l, "PlacedItem"));
    placed.push(later(l, "PlacedItem", "trace"));
    let mut raster = named(vec![
        ("bitsPerChannel", get(l, raster_bits)),
        ("channels", get(l, raster_channels)),
        ("colorizedGrayscale", get(l, raster_colorized)),
        ("embedded", property(l, raster_embedded, raster_set_embedded)),
        ("file", get(l, raster_file)),
        ("imageColorSpace", get(l, raster_color_space)),
        ("matrix", property(l, raster_matrix, raster_set_matrix)),
        ("status", get(l, raster_status)),
        ("transparent", get(l, raster_transparent)),
        ("boundingBox", get(l, raster_box)),
    ]);
    raster.extend(super::variables::item_members(l, "RasterItem"));
    raster.extend(["colorants", "colorize", "overprint", "trace"].into_iter().map(|m| later(l, "RasterItem", m)));
    vec![
        class("PlacedItems", None, placed_list, Some(super::function(l, placed_at))),
        class("RasterItems", None, raster_list, Some(super::function(l, raster_at))),
        class("PlacedItem", Some("PageItem"), placed, None),
        class("RasterItem", Some("PageItem"), raster, None),
    ]
}
