//! Paths and their points in the script object model, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PathItems/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PathItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PathPoints/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PathPoint/> describe them.
//!
//! A `PathPoints` object holds its path's handle, and a `PathPoint` the path's handle and the
//! point's index, counted across the path's subpaths in order. Each read looks the path up again.
//! Every change runs one command (`path.setAnchors` for the geometry), inside the run's one undo
//! step, and a change that changes nothing runs no command.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `Document.pathItems` lists the document's paths in the order of `pageItems`, paths inside
//!   groups and compound paths included.
//! - A change to the geometry or paint of a path that is hidden or locked, or that is in a hidden
//!   or locked layer or group, throws `Cannot edit` with nothing changed.
//! - A path in a compound path or compound shape reads that owner's paint, which the paint
//!   commands change in its place. A paint or stroke change leaves the paint for new art, the
//!   active proxy and the recent colors as they were.
//! - `closed` is true when every subpath of the path is closed, and assigning it opens or closes
//!   them all.
//! - `pathItems.add()` makes a path with no points on top of the active layer, painted as new art
//!   is. `filled` and `stroked` set to true give a path with no fill or stroke new art's fill or
//!   stroke (solid, gradient or pattern), or black when new art has none. `PathPoints.add()` adds
//!   a corner point with no handles at the last point, or at `[0, 0]` on a path with no points.
//!   Removing the last points leaves the path, with no points.
//! - A point whose index is past the end of its path throws `Stale path point`.
//! - Shape creators take sizes and radii that are finite and not negative; `sides` is a whole
//!   number from 3 to 1000 and `points` one from 2 to 1000. `inscribed` true throws an error that
//!   says it is not available yet. `rotateAbout` and `scaleAbout` default to
//!   `Transformation.CENTER`.
//! - The transforms take one setting for fill and stroke patterns, so `changeFillPatterns` and
//!   `changeStrokePattern` given as different values throw an error; gradients move with the art,
//!   so `changeFillGradients` false throws an error that says it is not available yet.

use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::pattern::pattern_paint;
use vectorcraft_doc::{GraphicStyle, LineCap, LineJoin};
use vectorcraft_geom::path::{Anchor, AnchorKind, FillRule, PathData, SubPath};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_js::{Data, HostClass, HostError, HostItem, HostMember, Realm, RunError, host_ref};
use vectorcraft_tools::params::gradient_params;

use super::{
    Answer, Coords, Doc, Item, Link, NOT_FOUND, STALE_ITEM, arg, boolean, class, colors, command, coordinate, doc_of, edit_item, editable, find_doc,
    float, frame, get, given, handle_of, id_of, id_parts, item_of, item_rect, later, method, named, num, number, on_doc, pair, point, property,
    this_doc, typename_of, whole, with_doc,
};
use crate::{DocState, NodeId, NodeKind};

/// The error of a point whose index is past the end of its path.
pub const STALE_POINT: &str = "Stale path point";
/// The most points `setEntirePath` takes.
pub const MAX_POINTS: usize = 100_000;
/// The most lengths `strokeDashes` takes.
pub const MAX_DASHES: usize = 64;
/// The largest stroke weight a script may set, in points.
pub const MAX_STROKE_WIDTH: f64 = 1000.0;

/// The enumerations of paths, as frozen objects whose members are `"Name.MEMBER"` strings.
pub(super) const ENUMS: &[(&str, &[&str])] = &[
    ("PointType", &["CORNER", "SMOOTH"]),
    ("PathPointSelection", &["ANCHORPOINT", "LEFTDIRECTION", "LEFTRIGHTPOINT", "NOSELECTION", "RIGHTDIRECTION"]),
    ("StrokeCap", &["BUTTENDCAP", "PROJECTINGENDCAP", "ROUNDENDCAP"]),
    ("StrokeJoin", &["BEVELENDJOIN", "MITERENDJOIN", "ROUNDENDJOIN"]),
    ("PolarityValues", &["NEGATIVE", "POSITIVE"]),
    ("Transformation", &["BOTTOM", "BOTTOMLEFT", "BOTTOMRIGHT", "CENTER", "DOCUMENTORIGIN", "LEFT", "RIGHT", "TOP", "TOPLEFT", "TOPRIGHT"]),
];

/// Adds the enumerations of [`ENUMS`] to `realm`.
pub(super) fn install_enums(realm: &mut Realm) -> std::result::Result<(), RunError> {
    for (name, members) in ENUMS {
        let items = members.iter().map(|m| (m.to_string(), HostItem::Value(Data::String(format!("{name}.{m}"))))).collect();
        realm.define_host_object(name, items)?;
    }
    Ok(())
}

fn cannot_edit(id: NodeId) -> HostError {
    HostError::Error(format!("Cannot edit PathItem {}: it, or a layer or group it is in, is hidden or locked", id.0))
}

/// The path `d` names, which must still be in its document.
fn path_of(h: &super::Host, d: &Data) -> std::result::Result<Item, HostError> {
    let item = item_of(h, d)?;
    let is_path = with_doc(h, item.doc, |_, st| st.doc.node(item.id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. })));
    if is_path { Ok(item) } else { Err(HostError::Type("this is not a PathItem".into())) }
}

fn this_path(h: &super::Host, a: &[Data]) -> std::result::Result<Item, HostError> {
    path_of(h, arg(a, 0))
}

/// The path of the run `run`, document `uid` and id parts `high` and `low`, as a handle gives them.
fn path_at(h: &super::Host, run: f64, uid: f64, high: f64, low: f64) -> std::result::Result<Item, HostError> {
    let doc = find_doc(h, run, uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let id = id_of(high, low).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    path_of(h, &path_ref(h, Item { doc, id }))
}

/// Reads the path of `item` with `f`.
fn read_path<T>(h: &super::Host, item: Item, f: impl FnOnce(&DocState, &crate::doc::Node, &PathData) -> T) -> std::result::Result<T, HostError> {
    with_doc(h, item.doc, |_, st| match st.doc.node(item.id) {
        Some(n) => match n.path_data() {
            Some(pd) => Ok(f(st, n, pd)),
            None => Err(HostError::Type("this is not a PathItem".into())),
        },
        None => Err(HostError::Error(STALE_ITEM.into())),
    })
}

fn path_ref(h: &super::Host, item: Item) -> Data {
    let [high, low] = id_parts(item.id);
    host_ref("PathItem", &[float(h.run), float(item.doc.uid), high, low])
}

/// How many points a path has, across its subpaths.
fn point_count(pd: &PathData) -> usize {
    pd.subpaths.iter().map(|sp| sp.anchors.len()).sum()
}

/// The subpath and the place in it of point `i`.
fn locate(pd: &PathData, mut i: usize) -> Option<(usize, usize)> {
    for (s, sp) in pd.subpaths.iter().enumerate() {
        if i < sp.anchors.len() {
            return Some((s, i));
        }
        i -= sp.anchors.len();
    }
    None
}

fn anchor_json(a: &Anchor) -> Value {
    json!({"x": a.p.x, "y": a.p.y, "in": [a.h_in.x, a.h_in.y], "out": [a.h_out.x, a.h_out.y], "smooth": a.kind == AnchorKind::Smooth})
}

/// Gives the path of `item` the subpaths `subs` with `path.setAnchors`, unless it has them.
fn write_path(h: &super::Host, item: Item, subs: Vec<SubPath>) -> std::result::Result<(), HostError> {
    let (same, ok) = read_path(h, item, |st, _, pd| (pd.subpaths == subs, editable(&st.doc, item.id)))?;
    if same {
        return Ok(());
    }
    if !ok {
        return Err(cannot_edit(item.id));
    }
    let subpaths: Vec<Value> =
        subs.iter().map(|sp| json!({"anchors": sp.anchors.iter().map(anchor_json).collect::<Vec<_>>(), "closed": sp.closed})).collect();
    edit_item(h, item, "path.setAnchors", &json!({"id": item.id.0, "subpaths": subpaths}))?;
    Ok(())
}

/// Runs `id` with `params` and `ids: [item]` on the item, which must be one the user could edit.
/// The paint for new art, the active Fill or Stroke proxy, the last color and gradient and the
/// recent colors, which `paint.setFill`, `paint.setStroke` and `stroke.set` also set, are put
/// back afterwards, so the edit changes only the path.
fn edit_path(h: &super::Host, item: Item, id: &str, mut params: Value) -> std::result::Result<Data, HostError> {
    let ok = with_doc(h, item.doc, |_, st| editable(&st.doc, item.id));
    if !ok {
        return Err(cannot_edit(item.id));
    }
    if let Some(o) = params.as_object_mut() {
        o.insert("ids".into(), json!([item.id.0]));
    }
    keeping_paint(h, || edit_item(h, item, id, &params))
}

/// Runs `f`, then puts back the paint for new art, the active Fill or Stroke proxy, the last color
/// and gradient and the recent colors as they were before it.
fn keeping_paint<T>(h: &super::Host, f: impl FnOnce() -> T) -> T {
    let kept = {
        let s = h.session.borrow();
        (s.paint.clone(), s.fill_active, s.last_solid, s.last_gradient.clone(), s.recent_colors.clone())
    };
    let r = f();
    let mut s = h.session.borrow_mut();
    (s.paint, s.fill_active, s.last_solid, s.last_gradient, s.recent_colors) = kept;
    r
}

// ---- Document.pathItems ----

/// The ids of a document's paths, in the order of its page items.
fn path_items(h: &super::Host, st: &DocState) -> Rc<Vec<NodeId>> {
    if let Some((uid, doc, ids)) = &*h.dom.paths.borrow()
        && *uid == st.uid
        && std::sync::Arc::ptr_eq(doc, &st.doc)
    {
        return ids.clone();
    }
    let all = super::page_items(h, st);
    let ids: Rc<Vec<NodeId>> =
        Rc::new(all.iter().copied().filter(|i| st.doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. }))).collect());
    *h.dom.paths.borrow_mut() = Some((st.uid, st.doc.clone(), ids.clone()));
    ids
}

pub(super) fn doc_path_items(h: &super::Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(host_ref("PathItems", &[float(h.run), float(doc.uid)]))
}

fn items_doc(h: &super::Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "PathItems")
}

fn items_length(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    Ok(num(float(with_doc(h, doc, |_, st| path_items(h, st).len()) as u64)))
}

fn items_parent(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    Ok(host_ref("Document", &[float(h.run), float(doc.uid)]))
}

fn items_at(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    Ok(with_doc(h, doc, |_, st| path_items(h, st).get(i).map_or(Data::Undefined, |id| super::item_ref(h, st, *id))))
}

fn items_by_name(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        let ids = path_items(h, st);
        h.charge_call(ids.len() as u64)?;
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| super::item_ref(h, st, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn items_index(h: &super::Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => items_by_name(h, a),
        Data::Number(_) => match items_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `pathItems.removeAll()`: deletes every path of the document in one command. A hidden or locked
/// path, or one in a hidden or locked layer or group, stops it before anything is deleted.
fn items_remove_all(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let (ids, refused) = with_doc(h, doc, |_, st| {
        let ids = path_items(h, st);
        let refused: Vec<u64> = ids.iter().filter(|i| !editable(&st.doc, **i)).map(|i| i.0).take(20).collect();
        (ids, refused)
    });
    if !refused.is_empty() {
        let list = refused.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("cannot delete hidden or locked paths: {list}")));
    }
    if !ids.is_empty() {
        on_doc(h, doc.uid, true, |h| command(h, "edit.clear", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    }
    Ok(Data::Undefined)
}

/// The id in the result of a command that made an object.
fn made_id(r: &Data) -> std::result::Result<NodeId, HostError> {
    let id = match r {
        Data::Object(o) => o.iter().find(|(k, _)| k == "id").and_then(|(_, v)| if let Data::Number(n) = v { whole(*n) } else { None }),
        _ => None,
    };
    id.map(NodeId).ok_or_else(|| HostError::Error("the path was not made".into()))
}

/// Runs the command `id` that makes a path in `doc`, the selection kept, and reverses the new
/// path's direction when `reversed`. → the new path.
fn make(h: &super::Host, doc: Doc, id: &str, params: &Value, reversed: bool) -> Answer {
    let made = on_doc(h, doc.uid, true, |h| {
        let id = made_id(&command(h, id, params)?)?;
        if reversed {
            command(h, "path.reverse", &json!({"ids": [id.0]}))?;
        }
        Ok(id)
    })?;
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == doc.uid).ok_or_else(|| HostError::Error(super::STALE_DOCUMENT.into()))?;
    Ok(super::item_ref(h, st, made))
}

/// Argument `i`, or `default` when it is left out.
fn number_or(a: &[Data], i: usize, default: f64, what: &str) -> std::result::Result<f64, HostError> {
    given(arg(a, i)).map_or(Ok(default), |d| number(d, what))
}

/// A size or radius: finite and not negative.
fn size(a: &[Data], i: usize, default: Option<f64>, what: &str) -> std::result::Result<f64, HostError> {
    let v = match (given(arg(a, i)), default) {
        (Some(d), _) => number(d, what)?,
        (None, Some(v)) => v,
        (None, None) => return Err(HostError::Type(format!("{what} must be a finite number"))),
    };
    if !(0.0..=super::MAX_SIZE).contains(&v) {
        return Err(HostError::Range(format!("{what} must be from 0 to 3999856 points")));
    }
    Ok(v)
}

/// A whole number argument from `min` to `max`.
fn count(a: &[Data], i: usize, default: f64, min: f64, max: f64, what: &str) -> std::result::Result<f64, HostError> {
    let v = number_or(a, i, default, what)?;
    if v.fract() != 0.0 || !(min..=max).contains(&v) {
        return Err(HostError::Range(format!("{what} must be a whole number from {min} to {max}")));
    }
    Ok(v)
}

fn flag(a: &[Data], i: usize, what: &str) -> std::result::Result<bool, HostError> {
    given(arg(a, i)).map_or(Ok(false), |d| boolean(d, what))
}

/// The top-left corner, in document coordinates, of a box whose top-left corner the script gives.
fn corner(h: &super::Host, doc: Doc, left: f64, top: f64) -> std::result::Result<Point, HostError> {
    with_doc(h, doc, |_, st| Ok(frame(st)?.to_document(h.dom.coords.get(), [left, top])))
}

/// `rectangle(top, left, width, height, reversed?)`.
fn items_rectangle(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let top = coordinate(arg(a, 1), "top")?;
    let left = coordinate(arg(a, 2), "left")?;
    let (w, ht) = (size(a, 3, None, "width")?, size(a, 4, None, "height")?);
    let reversed = flag(a, 5, "reversed")?;
    let p = corner(h, doc, left, top)?;
    make(h, doc, "shape.rectangle", &json!({"x": p.x, "y": p.y, "width": w, "height": ht}), reversed)
}

/// The anchors of a rectangle `r` with elliptical corners of radii `rx` and `ry`, clockwise on the
/// page from the top of the left side.
fn rounded(r: Rect, rx: f64, ry: f64) -> Vec<Anchor> {
    const K: f64 = 0.552_284_749_830_793_4;
    let (rx, ry) = (rx.min(r.width() / 2.0), ry.min(r.height() / 2.0));
    let (kx, ky) = (rx * K, ry * K);
    let a = |p: Point, h_in: Point, h_out: Point| Anchor { p, h_in, h_out, kind: AnchorKind::Corner };
    let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);
    let mut out = vec![
        a(Point::new(x0, y0 + ry), Point::new(x0, y0 + ry), Point::new(x0, y0 + ry - ky)),
        a(Point::new(x0 + rx, y0), Point::new(x0 + rx - kx, y0), Point::new(x0 + rx, y0)),
        a(Point::new(x1 - rx, y0), Point::new(x1 - rx, y0), Point::new(x1 - rx + kx, y0)),
        a(Point::new(x1, y0 + ry), Point::new(x1, y0 + ry - ky), Point::new(x1, y0 + ry)),
        a(Point::new(x1, y1 - ry), Point::new(x1, y1 - ry), Point::new(x1, y1 - ry + ky)),
        a(Point::new(x1 - rx, y1), Point::new(x1 - rx + kx, y1), Point::new(x1 - rx, y1)),
        a(Point::new(x0 + rx, y1), Point::new(x0 + rx, y1), Point::new(x0 + rx - kx, y1)),
        a(Point::new(x0, y1 - ry), Point::new(x0, y1 - ry + ky), Point::new(x0, y1 - ry)),
    ];
    out.dedup_by(|b, a| {
        b.p.distance(a.p) < 1e-9 && {
            a.h_out = b.h_out;
            true
        }
    });
    if out.len() > 1
        && out.first().zip(out.last()).is_some_and(|(f, l)| f.p.distance(l.p) < 1e-9)
        && let Some(l) = out.pop()
        && let Some(f) = out.first_mut()
    {
        f.h_in = l.h_in;
    }
    out
}

/// `roundedRectangle(top, left, width, height, horizontalRadius = 15, verticalRadius = 20,
/// reversed = false)`: a closed path with `path.create`.
fn items_rounded_rectangle(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let top = coordinate(arg(a, 1), "top")?;
    let left = coordinate(arg(a, 2), "left")?;
    let (w, ht) = (size(a, 3, None, "width")?, size(a, 4, None, "height")?);
    let (rx, ry) = (size(a, 5, Some(15.0), "horizontalRadius")?, size(a, 6, Some(20.0), "verticalRadius")?);
    let reversed = flag(a, 7, "reversed")?;
    let p = corner(h, doc, left, top)?;
    let anchors: Vec<Value> = rounded(Rect::new(p.x, p.y, p.x + w, p.y + ht), rx, ry).iter().map(anchor_json).collect();
    make(h, doc, "path.create", &json!({"anchors": anchors, "closed": true}), reversed)
}

/// `ellipse(top = 100, left = 100, width = 50, height = 100, reversed = false, inscribed)`.
fn items_ellipse(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let top = given(arg(a, 1)).map_or(Ok(100.0), |d| coordinate(d, "top"))?;
    let left = given(arg(a, 2)).map_or(Ok(100.0), |d| coordinate(d, "left"))?;
    let (w, ht) = (size(a, 3, Some(50.0), "width")?, size(a, 4, Some(100.0), "height")?);
    let reversed = flag(a, 5, "reversed")?;
    if flag(a, 6, "inscribed")? {
        return Err(HostError::Error("ellipse with inscribed true is not available yet".into()));
    }
    let p = corner(h, doc, left, top)?;
    make(h, doc, "shape.ellipse", &json!({"x": p.x, "y": p.y, "width": w, "height": ht}), reversed)
}

/// The center of a polygon or star, in document coordinates.
fn center(h: &super::Host, doc: Doc, a: &[Data]) -> std::result::Result<Point, HostError> {
    let x = given(arg(a, 1)).map_or(Ok(200.0), |d| coordinate(d, "centerX"))?;
    let y = given(arg(a, 2)).map_or(Ok(300.0), |d| coordinate(d, "centerY"))?;
    corner(h, doc, x, y)
}

/// `polygon(centerX = 200, centerY = 300, radius = 50, sides = 8, reversed = false)`.
fn items_polygon(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let c = center(h, doc, a)?;
    let r = size(a, 3, Some(50.0), "radius")?;
    let sides = count(a, 4, 8.0, 3.0, 1000.0, "sides")?;
    let reversed = flag(a, 5, "reversed")?;
    make(h, doc, "shape.polygon", &json!({"cx": c.x, "cy": c.y, "radius": r, "sides": sides}), reversed)
}

/// `star(centerX = 200, centerY = 300, radius = 50, innerRadius = 20, points = 5, reversed =
/// false)`.
fn items_star(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let c = center(h, doc, a)?;
    let r1 = size(a, 3, Some(50.0), "radius")?;
    let r2 = size(a, 4, Some(20.0), "innerRadius")?;
    let points = count(a, 5, 5.0, 2.0, 1000.0, "points")?;
    let reversed = flag(a, 6, "reversed")?;
    make(h, doc, "shape.star", &json!({"cx": c.x, "cy": c.y, "radius1": r1, "radius2": r2, "points": points}), reversed)
}

/// `pathItems.add()`: a path with no points, from `path.create` with empty path data, painted as
/// new art. When new art has no fill and no stroke, the black stroke `path.create` gives an open
/// path is set back to none in the same step.
fn items_add(h: &super::Host, a: &[Data]) -> Answer {
    let doc = items_doc(h, a)?;
    let made = on_doc(h, doc.uid, true, |h| {
        let unpainted = {
            let ap = h.session.borrow().new_art();
            ap.fill_paint().is_none() && ap.stroke_paint().is_none()
        };
        let id = made_id(&command(h, "path.create", &json!({"d": ""}))?)?;
        if unpainted {
            keeping_paint(h, || command(h, "paint.setStroke", &json!({"none": true, "ids": [id.0]})))?;
        }
        Ok(id)
    })?;
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == doc.uid).ok_or_else(|| HostError::Error(super::STALE_DOCUMENT.into()))?;
    Ok(super::item_ref(h, st, made))
}

// ---- PathItem geometry ----

fn path_points(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    let [high, low] = id_parts(item.id);
    Ok(host_ref("PathPoints", &[float(h.run), float(item.doc.uid), high, low]))
}

fn path_closed(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    read_path(h, item, |_, _, pd| Data::Bool(!pd.subpaths.is_empty() && pd.subpaths.iter().all(|sp| sp.closed)))
}

fn path_set_closed(h: &super::Host, a: &[Data]) -> Answer {
    let closed = boolean(arg(a, 1), "closed")?;
    let item = this_path(h, a)?;
    let subs = read_path(h, item, |_, _, pd| pd.subpaths.iter().map(|sp| SubPath { anchors: sp.anchors.clone(), closed }).collect())?;
    write_path(h, item, subs)?;
    Ok(Data::Undefined)
}

/// Whether the path fills by the even-odd rule: its compound path's rule when it is a member of one,
/// as `path.setFillRule` sets it there.
fn path_evenodd(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    read_path(h, item, |st, n, _| {
        let owner = st.doc.parent_of(item.id).and_then(|p| st.doc.node(p)).filter(|p| matches!(p.kind, NodeKind::Compound { .. })).unwrap_or(n);
        Data::Bool(matches!(owner.kind, NodeKind::Path { rule: FillRule::EvenOdd, .. } | NodeKind::Compound { rule: FillRule::EvenOdd, .. }))
    })
}

fn path_set_evenodd(h: &super::Host, a: &[Data]) -> Answer {
    let on = boolean(arg(a, 1), "evenodd")?;
    let item = this_path(h, a)?;
    if path_evenodd(h, a)? != Data::Bool(on) {
        edit_path(h, item, "path.setFillRule", json!({"rule": if on { "evenOdd" } else { "nonZero" }}))?;
    }
    Ok(Data::Undefined)
}

/// The area in square points: negative when the path runs counterclockwise on the page.
fn path_area(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    read_path(h, item, |_, _, pd| num(pd.subpaths.iter().map(SubPath::signed_area).sum()))
}

fn path_length(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    read_path(h, item, |_, _, pd| num(pd.length()))
}

fn path_guides(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    read_path(h, item, |_, n, _| Data::Bool(matches!(n.kind, NodeKind::Path { guide: true, .. })))
}

fn path_clipping(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    read_path(h, item, |_, n, _| Data::Bool(matches!(n.kind, NodeKind::Path { clipping: true, .. })))
}

/// `setEntirePath([[x, y], ...])`: corner points with no handles in place of every point, in one
/// subpath that stays closed when the path was.
fn path_set_entire(h: &super::Host, a: &[Data]) -> Answer {
    let item = this_path(h, a)?;
    let Data::Array(list) = arg(a, 1) else { return Err(HostError::Type("setEntirePath needs an array of [x, y] points".into())) };
    if list.len() > MAX_POINTS {
        return Err(HostError::Range(format!("setEntirePath takes at most {MAX_POINTS} points")));
    }
    let script: Vec<[f64; 2]> = list.iter().map(|d| point(d, "each point")).collect::<std::result::Result<_, _>>()?;
    let (f, closed) = read_path(h, item, |st, _, pd| (frame(st), !pd.subpaths.is_empty() && pd.subpaths.iter().all(|sp| sp.closed)))?;
    let (f, c) = (f?, h.dom.coords.get());
    let anchors: Vec<Anchor> = script.iter().map(|p| Anchor::corner(f.to_document(c, *p))).collect();
    let subs = if anchors.is_empty() { Vec::new() } else { vec![SubPath::new(anchors, closed)] };
    write_path(h, item, subs)?;
    Ok(Data::Undefined)
}

// ---- PathItem paint ----

/// The top fill's paint and the top stroke's paint, width, cap, join, miter limit, dashes and
/// offset, and the overprint flags of both.
struct Look {
    fill: Paint,
    fill_overprint: bool,
    stroke: Paint,
    width: f64,
    cap: LineCap,
    join: LineJoin,
    miter: f64,
    dashes: Vec<f64>,
    offset: f64,
    stroke_overprint: bool,
}

/// The look of `item`, read from its compound path or compound shape when it is a member of one,
/// since the paint commands paint that owner in its place.
fn look(h: &super::Host, item: Item) -> std::result::Result<Look, HostError> {
    read_path(h, item, |st, n, _| {
        let d = &st.doc;
        let owner = d
            .parent_of(item.id)
            .and_then(|p| d.node(p))
            .filter(|p| matches!(p.kind, NodeKind::Compound { .. }) || p.is_compound_shape())
            .unwrap_or(n);
        let ap = &owner.appearance;
        let fill = ap.fill();
        let stroke = ap.stroke();
        Look {
            fill: fill.map_or(Paint::None, |f| f.paint.clone()),
            fill_overprint: fill.is_some_and(|f| f.overprint),
            stroke: stroke.map_or(Paint::None, |s| s.paint.clone()),
            width: stroke.map_or(0.0, |s| s.width),
            cap: stroke.map_or(LineCap::default(), |s| s.cap),
            join: stroke.map_or(LineJoin::default(), |s| s.join),
            miter: stroke.map_or(10.0, |s| s.miter_limit),
            dashes: stroke.and_then(|s| s.dash.as_ref()).map_or_else(Vec::new, |d| d.pattern.clone()),
            offset: stroke.and_then(|s| s.dash.as_ref()).map_or(0.0, |d| d.offset),
            stroke_overprint: stroke.is_some_and(|s| s.overprint),
        }
    })
}

fn this_look(h: &super::Host, a: &[Data]) -> std::result::Result<(Item, Look), HostError> {
    let item = this_path(h, a)?;
    Ok((item, look(h, item)?))
}

fn path_filled(h: &super::Host, a: &[Data]) -> Answer {
    Ok(Data::Bool(!this_look(h, a)?.1.fill.is_none()))
}

fn path_stroked(h: &super::Host, a: &[Data]) -> Answer {
    Ok(Data::Bool(!this_look(h, a)?.1.stroke.is_none()))
}

/// Sets the fill (`fill`) or the stroke to `color`, or to none, with `paint.setFill` or
/// `paint.setStroke`.
fn set_paint(h: &super::Host, item: Item, fill: bool, color: Option<Color>) -> std::result::Result<(), HostError> {
    let id = if fill { "paint.setFill" } else { "paint.setStroke" };
    let params = match color {
        Some(c) => json!({"color": colors::param(&c)}),
        None => json!({"none": true}),
    };
    edit_path(h, item, id, params)?;
    Ok(())
}

/// The paint new art gets for its fill or stroke, as `paint.setFill` or `paint.setStroke` params for
/// the path `item`: a solid color, linked to its global or spot swatch while the document has that
/// swatch; a gradient, a placed one moved onto the path's bounds as new art takes it; or a pattern.
/// A fill or stroke of none gives black. A pattern that its swatch does not give as new art has it
/// (its own placement, or a pattern the document does not have) throws an error.
fn new_art_params(h: &super::Host, item: Item, fill: bool) -> std::result::Result<Value, HostError> {
    let mut ap = h.session.borrow().new_art();
    let top = |ap: &vectorcraft_doc::Appearance| if fill { ap.fill_paint() } else { ap.stroke_paint() };
    let what = if fill { "filled" } else { "stroked" };
    let paint = top(&ap);
    match &paint {
        Paint::None => Ok(json!({"color": colors::param(&Color::BLACK)})),
        Paint::Solid { color, swatch: Some(name), tint } => {
            let linked = read_path(h, item, |st, _, _| st.doc.swatch(name).is_some())?;
            Ok(match linked {
                true if *tint < 1.0 => json!({"swatch": name, "tint": f64::from(*tint) * 100.0}),
                true => json!({"swatch": name}),
                false => json!({"color": colors::param(color)}),
            })
        }
        Paint::Solid { color, .. } => Ok(json!({"color": colors::param(color)})),
        Paint::Gradient(g) => {
            let mut g = g.clone();
            if g.geom.is_some() || g.freeform.is_some() {
                match read_path(h, item, |_, n, _| n.geometric_bounds())? {
                    Some(b) => {
                        ap.rebase_gradients(GraphicStyle::UNIT_BOX, b);
                        if let Paint::Gradient(placed) = top(&ap) {
                            g = placed;
                        }
                    }
                    None => (g.geom, g.freeform) = (None, None),
                }
            }
            Ok(json!({"gradient": gradient_params(&g), "keepModel": true}))
        }
        Paint::Pattern { pattern, .. } => {
            let given = read_path(h, item, |st, _, _| {
                let d = &st.doc;
                d.swatch(pattern).map(|sw| sw.paint.clone()).or_else(|| d.pattern(pattern).map(|_| pattern_paint(pattern)))
            })?;
            match given {
                Some(p) if p == paint => Ok(json!({"swatch": pattern})),
                Some(_) => Err(HostError::Error(format!(
                    "Cannot set {what}: new art's pattern `{pattern}` has a placement of its own, which is not available yet"
                ))),
                None => Err(HostError::Error(format!("Cannot set {what}: new art's pattern `{pattern}` is not in this document"))),
            }
        }
    }
}

fn set_painted(h: &super::Host, a: &[Data], fill: bool) -> Answer {
    let on = boolean(arg(a, 1), if fill { "filled" } else { "stroked" })?;
    let (item, look) = this_look(h, a)?;
    let now = if fill { !look.fill.is_none() } else { !look.stroke.is_none() };
    if now != on {
        let params = if on { new_art_params(h, item, fill)? } else { json!({"none": true}) };
        edit_path(h, item, if fill { "paint.setFill" } else { "paint.setStroke" }, params)?;
    }
    Ok(Data::Undefined)
}

fn path_set_filled(h: &super::Host, a: &[Data]) -> Answer {
    set_painted(h, a, true)
}

fn path_set_stroked(h: &super::Host, a: &[Data]) -> Answer {
    set_painted(h, a, false)
}

fn path_fill_color(h: &super::Host, a: &[Data]) -> Answer {
    Ok(colors::of_paint(&this_look(h, a)?.1.fill))
}

fn path_stroke_color(h: &super::Host, a: &[Data]) -> Answer {
    Ok(colors::of_paint(&this_look(h, a)?.1.stroke))
}

fn set_color(h: &super::Host, a: &[Data], fill: bool) -> Answer {
    let color = colors::assigned(arg(a, 1), if fill { "fillColor" } else { "strokeColor" })?;
    let (item, look) = this_look(h, a)?;
    let now = if fill { &look.fill } else { &look.stroke };
    let same = match (&color, now) {
        (None, Paint::None) => true,
        (Some(c), Paint::Solid { color, swatch: None, .. }) => c == color,
        _ => false,
    };
    if !same {
        set_paint(h, item, fill, color)?;
    }
    Ok(Data::Undefined)
}

fn path_set_fill_color(h: &super::Host, a: &[Data]) -> Answer {
    set_color(h, a, true)
}

fn path_set_stroke_color(h: &super::Host, a: &[Data]) -> Answer {
    set_color(h, a, false)
}

/// Runs `stroke.set` with `params` on the path, unless `same`.
fn set_stroke_option(h: &super::Host, item: Item, same: bool, params: Value) -> Answer {
    if !same {
        edit_path(h, item, "stroke.set", params)?;
    }
    Ok(Data::Undefined)
}

fn path_stroke_width(h: &super::Host, a: &[Data]) -> Answer {
    Ok(num(this_look(h, a)?.1.width))
}

fn path_set_stroke_width(h: &super::Host, a: &[Data]) -> Answer {
    let w = number(arg(a, 1), "strokeWidth")?;
    if !(0.0..=MAX_STROKE_WIDTH).contains(&w) {
        return Err(HostError::Range("strokeWidth must be from 0 to 1000 points".into()));
    }
    let (item, look) = this_look(h, a)?;
    set_stroke_option(h, item, (look.width - w).abs() < 1e-9, json!({"weight": w}))
}

const CAPS: &[(LineCap, &str, &str)] = &[
    (LineCap::Butt, "StrokeCap.BUTTENDCAP", "butt"),
    (LineCap::Round, "StrokeCap.ROUNDENDCAP", "round"),
    (LineCap::Square, "StrokeCap.PROJECTINGENDCAP", "square"),
];

const JOINS: &[(LineJoin, &str, &str)] = &[
    (LineJoin::Bevel, "StrokeJoin.BEVELENDJOIN", "bevel"),
    (LineJoin::Round, "StrokeJoin.ROUNDENDJOIN", "round"),
    (LineJoin::Miter, "StrokeJoin.MITERENDJOIN", "miter"),
];

fn path_stroke_cap(h: &super::Host, a: &[Data]) -> Answer {
    let cap = this_look(h, a)?.1.cap;
    Ok(Data::String(CAPS.iter().find(|c| c.0 == cap).map_or("StrokeCap.BUTTENDCAP", |c| c.1).into()))
}

fn path_set_stroke_cap(h: &super::Host, a: &[Data]) -> Answer {
    let found = match arg(a, 1) {
        Data::String(s) => CAPS.iter().find(|c| c.1 == s),
        _ => None,
    };
    let Some(&(cap, _, name)) = found else { return Err(HostError::Type("strokeCap must be a StrokeCap value".into())) };
    let (item, look) = this_look(h, a)?;
    set_stroke_option(h, item, look.cap == cap, json!({"cap": name}))
}

fn path_stroke_join(h: &super::Host, a: &[Data]) -> Answer {
    let join = this_look(h, a)?.1.join;
    Ok(Data::String(JOINS.iter().find(|j| j.0 == join).map_or("StrokeJoin.MITERENDJOIN", |j| j.1).into()))
}

fn path_set_stroke_join(h: &super::Host, a: &[Data]) -> Answer {
    let found = match arg(a, 1) {
        Data::String(s) => JOINS.iter().find(|j| j.1 == s),
        _ => None,
    };
    let Some(&(join, _, name)) = found else { return Err(HostError::Type("strokeJoin must be a StrokeJoin value".into())) };
    let (item, look) = this_look(h, a)?;
    set_stroke_option(h, item, look.join == join, json!({"join": name}))
}

fn path_miter(h: &super::Host, a: &[Data]) -> Answer {
    Ok(num(this_look(h, a)?.1.miter))
}

fn path_set_miter(h: &super::Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "strokeMiterLimit")?;
    if !(1.0..=500.0).contains(&v) {
        return Err(HostError::Range("strokeMiterLimit must be from 1 to 500".into()));
    }
    let (item, look) = this_look(h, a)?;
    set_stroke_option(h, item, (look.miter - v).abs() < 1e-9, json!({"miterLimit": v}))
}

fn path_dashes(h: &super::Host, a: &[Data]) -> Answer {
    Ok(Data::Array(this_look(h, a)?.1.dashes.into_iter().map(num).collect()))
}

/// `strokeDashes = [lengths]`: at most [`MAX_DASHES`] finite lengths that are not negative and
/// not all 0; `[]` or `{}` gives a solid line.
fn path_set_dashes(h: &super::Host, a: &[Data]) -> Answer {
    let lengths: Vec<f64> = match arg(a, 1) {
        Data::Object(o) if o.is_empty() => Vec::new(),
        Data::Array(list) if list.len() > MAX_DASHES => return Err(HostError::Range(format!("strokeDashes takes at most {MAX_DASHES} lengths"))),
        Data::Array(list) => list.iter().map(|d| number(d, "each dash length")).collect::<std::result::Result<_, _>>()?,
        _ => return Err(HostError::Type("strokeDashes must be an array of lengths".into())),
    };
    if lengths.iter().any(|l| !(0.0..=MAX_STROKE_WIDTH * 1000.0).contains(l)) {
        return Err(HostError::Range("each dash length must be from 0 to 1000000 points".into()));
    }
    if !lengths.is_empty() && lengths.iter().all(|l| *l == 0.0) {
        return Err(HostError::Range("strokeDashes needs a length that is not 0".into()));
    }
    let (item, look) = this_look(h, a)?;
    let dash = if lengths.is_empty() { Value::Null } else { json!(lengths) };
    set_stroke_option(h, item, look.dashes == lengths, json!({"dash": dash}))
}

fn path_dash_offset(h: &super::Host, a: &[Data]) -> Answer {
    Ok(num(this_look(h, a)?.1.offset))
}

fn path_set_dash_offset(h: &super::Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "strokeDashOffset")?;
    if v.abs() > MAX_STROKE_WIDTH * 1000.0 {
        return Err(HostError::Range("strokeDashOffset must be from -1000000 to 1000000 points".into()));
    }
    let (item, look) = this_look(h, a)?;
    if look.dashes.is_empty() && v != 0.0 {
        return Err(HostError::Error("strokeDashOffset needs strokeDashes first".into()));
    }
    set_stroke_option(h, item, (look.offset - v).abs() < 1e-9, json!({"dashOffset": v}))
}

fn path_fill_overprint(h: &super::Host, a: &[Data]) -> Answer {
    Ok(Data::Bool(this_look(h, a)?.1.fill_overprint))
}

fn path_stroke_overprint(h: &super::Host, a: &[Data]) -> Answer {
    Ok(Data::Bool(this_look(h, a)?.1.stroke_overprint))
}

fn set_overprint(h: &super::Host, a: &[Data], fill: bool) -> Answer {
    let key = if fill { "fill" } else { "stroke" };
    let on = boolean(arg(a, 1), if fill { "fillOverprint" } else { "strokeOverprint" })?;
    let (item, look) = this_look(h, a)?;
    let (now, paint) = if fill { (look.fill_overprint, &look.fill) } else { (look.stroke_overprint, &look.stroke) };
    if now != on {
        if paint.is_none() {
            return Err(HostError::Error(format!("the path has no {key} to overprint")));
        }
        edit_path(h, item, "object.setOverprint", json!({key: on}))?;
    }
    Ok(Data::Undefined)
}

fn path_set_fill_overprint(h: &super::Host, a: &[Data]) -> Answer {
    set_overprint(h, a, true)
}

fn path_set_stroke_overprint(h: &super::Host, a: &[Data]) -> Answer {
    set_overprint(h, a, false)
}

// ---- PathItem transforms ----

/// The point of the path's geometric bounds `r`, in document coordinates, that a `Transformation`
/// value names (left out: the center).
fn about(h: &super::Host, item: Item, d: &Data, r: Rect) -> std::result::Result<Point, HostError> {
    let name = match d {
        Data::Undefined => "Transformation.CENTER",
        Data::String(s) => s.as_str(),
        _ => return Err(HostError::Type("give a Transformation value".into())),
    };
    let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
    Ok(match name.strip_prefix("Transformation.").unwrap_or("") {
        "CENTER" => Point::new(cx, cy),
        "TOP" => Point::new(cx, r.y0),
        "BOTTOM" => Point::new(cx, r.y1),
        "LEFT" => Point::new(r.x0, cy),
        "RIGHT" => Point::new(r.x1, cy),
        "TOPLEFT" => Point::new(r.x0, r.y0),
        "TOPRIGHT" => Point::new(r.x1, r.y0),
        "BOTTOMLEFT" => Point::new(r.x0, r.y1),
        "BOTTOMRIGHT" => Point::new(r.x1, r.y1),
        "DOCUMENTORIGIN" => with_doc(h, item.doc, |_, st| Ok::<_, HostError>(frame(st)?.to_document(Coords::Document, [0.0, 0.0])))?,
        _ => return Err(HostError::Type("give a Transformation value".into())),
    })
}

/// The `objects` and `patterns` options of a transform command from the four flags from `i`:
/// objects, fill patterns, fill gradients and stroke patterns, each left out or true or false.
fn transform_options(a: &[Data], i: usize, params: &mut Value) -> std::result::Result<(), HostError> {
    let names = ["changePositions", "changeFillPatterns", "changeFillGradients", "changeStrokePattern"];
    let mut flags = [None; 4];
    for (k, name) in names.iter().enumerate() {
        flags[k] = given(arg(a, i + k)).map(|d| boolean(d, name)).transpose()?;
    }
    if flags[2] == Some(false) {
        return Err(HostError::Error(format!("{} false is not available yet: gradients move with the art", names[2])));
    }
    if let (Some(f), Some(s)) = (flags[1], flags[3])
        && f != s
    {
        return Err(HostError::Error(format!("{} and {} must be the same here", names[1], names[3])));
    }
    if let Some(o) = params.as_object_mut() {
        if let Some(b) = flags[0] {
            o.insert("objects".into(), json!(b));
        }
        if let Some(b) = flags[1].or(flags[3]) {
            o.insert("patterns".into(), json!(b));
        }
    }
    Ok(())
}

/// `translate(deltaX = 0, deltaY = 0, transformObjects?, transformFillPatterns?, ...)`.
fn path_translate(h: &super::Host, a: &[Data]) -> Answer {
    let dx = given(arg(a, 1)).map_or(Ok(0.0), |d| coordinate(d, "deltaX"))?;
    let dy = given(arg(a, 2)).map_or(Ok(0.0), |d| coordinate(d, "deltaY"))?;
    let item = this_path(h, a)?;
    let mut params = json!({"dx": dx, "dy": if h.dom.coords.get() == Coords::Document { -dy } else { dy }});
    transform_options(a, 3, &mut params)?;
    if dx != 0.0 || dy != 0.0 {
        edit_path(h, item, "object.move", params)?;
    }
    Ok(Data::Undefined)
}

/// `rotate(angle, changePositions?, ..., rotateAbout?)`: counterclockwise on the page for a
/// positive angle.
fn path_rotate(h: &super::Host, a: &[Data]) -> Answer {
    let angle = number(arg(a, 1), "angle")?;
    let item = this_path(h, a)?;
    let (r, _) = item_rect(h, item, false)?;
    let origin = about(h, item, arg(a, 6), r)?;
    let mut params = json!({"angle": angle, "origin": [origin.x, origin.y]});
    transform_options(a, 2, &mut params)?;
    if angle.rem_euclid(360.0) != 0.0 {
        edit_path(h, item, "object.rotate", params)?;
    }
    Ok(Data::Undefined)
}

/// `resize(scaleX, scaleY, changePositions?, ..., changeLineWidths?, scaleAbout?)`: 100 is 100%;
/// `changeLineWidths` scales the stroke weight by that percent.
fn path_resize(h: &super::Host, a: &[Data]) -> Answer {
    let sx = number(arg(a, 1), "scaleX")?;
    let sy = number(arg(a, 2), "scaleY")?;
    if sx == 0.0 || sy == 0.0 || sx.abs() > 100_000.0 || sy.abs() > 100_000.0 {
        return Err(HostError::Range("scaleX and scaleY must not be 0 and at most 100000 percent".into()));
    }
    let widths = given(arg(a, 7)).map(|d| number(d, "changeLineWidths")).transpose()?;
    if widths.is_some_and(|w| !(0.0..=100_000.0).contains(&w)) {
        return Err(HostError::Range("changeLineWidths must be from 0 to 100000 percent".into()));
    }
    let item = this_path(h, a)?;
    let (r, _) = item_rect(h, item, false)?;
    let origin = about(h, item, arg(a, 8), r)?;
    let mut params = json!({"sx": sx, "sy": sy, "origin": [origin.x, origin.y], "basis": "page", "strokes": false});
    transform_options(a, 3, &mut params)?;
    let width = look(h, item)?.width;
    let new_width = widths.map(|w| width * w / 100.0);
    if new_width.is_some_and(|w| w > MAX_STROKE_WIDTH) {
        return Err(HostError::Range("changeLineWidths would make the stroke weight more than 1000 points".into()));
    }
    if sx != 100.0 || sy != 100.0 {
        edit_path(h, item, "object.scale", params)?;
    }
    if let Some(w) = new_width
        && (w - width).abs() > 1e-9
    {
        edit_path(h, item, "stroke.set", json!({"weight": w}))?;
    }
    Ok(Data::Undefined)
}

// ---- PathPoints ----

fn points_path(h: &super::Host, a: &[Data]) -> std::result::Result<Item, HostError> {
    let Some(handle) = handle_of(arg(a, 0), "PathPoints") else { return Err(HostError::Type("this is not a PathPoints".into())) };
    let [run, uid, high, low] = handle.as_slice() else { return Err(HostError::Type("this is not a PathPoints".into())) };
    path_at(h, *run, *uid, *high, *low)
}

fn points_length(h: &super::Host, a: &[Data]) -> Answer {
    let item = points_path(h, a)?;
    read_path(h, item, |_, _, pd| num(float(point_count(pd) as u64)))
}

fn points_parent(h: &super::Host, a: &[Data]) -> Answer {
    let item = points_path(h, a)?;
    Ok(path_ref(h, item))
}

fn point_ref(h: &super::Host, item: Item, i: usize) -> Data {
    let [high, low] = id_parts(item.id);
    host_ref("PathPoint", &[float(h.run), float(item.doc.uid), high, low, float(i as u64)])
}

fn points_at(h: &super::Host, a: &[Data]) -> Answer {
    let item = points_path(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    let n = read_path(h, item, |_, _, pd| point_count(pd))?;
    Ok(if i < n { point_ref(h, item, i) } else { Data::Undefined })
}

/// `index(key)`: the points are not named, so a key that is not a number throws `Not found`.
fn points_index(h: &super::Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::Number(_) => match points_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Error(NOT_FOUND.into())),
    }
}

fn points_add(h: &super::Host, a: &[Data]) -> Answer {
    let item = points_path(h, a)?;
    let (mut subs, n, origin) = read_path(h, item, |st, _, pd| {
        let origin = frame(st).map(|f| f.to_document(h.dom.coords.get(), [0.0, 0.0]));
        (pd.subpaths.clone(), point_count(pd), origin)
    })?;
    if n >= MAX_POINTS {
        return Err(HostError::Range(format!("PathPoints.add takes a path to at most {MAX_POINTS} points")));
    }
    let p = match subs.iter().rev().find_map(|sp| sp.anchors.last()) {
        Some(last) => last.p,
        None => origin?,
    };
    match subs.last_mut() {
        Some(sp) => sp.anchors.push(Anchor::corner(p)),
        None => subs.push(SubPath::new(vec![Anchor::corner(p)], false)),
    }
    write_path(h, item, subs)?;
    Ok(point_ref(h, item, n))
}

fn points_remove_all(h: &super::Host, a: &[Data]) -> Answer {
    let item = points_path(h, a)?;
    write_path(h, item, Vec::new())?;
    Ok(Data::Undefined)
}

// ---- PathPoint ----

/// The path and the index of the point `this` names, which must still be in the path.
pub(super) fn this_point(h: &super::Host, a: &[Data]) -> std::result::Result<(Item, usize), HostError> {
    let Some(handle) = handle_of(arg(a, 0), "PathPoint") else { return Err(HostError::Type("this is not a PathPoint".into())) };
    let [run, uid, high, low, i] = handle.as_slice() else { return Err(HostError::Type("this is not a PathPoint".into())) };
    let item = path_at(h, *run, *uid, *high, *low)?;
    let i = whole(*i).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| HostError::Error(STALE_POINT.into()))?;
    let n = read_path(h, item, |_, _, pd| point_count(pd))?;
    if i >= n {
        return Err(HostError::Error(STALE_POINT.into()));
    }
    Ok((item, i))
}

/// Reads the anchor `this` names with `f`, given the frame and the coordinate system.
fn read_point(h: &super::Host, a: &[Data], f: impl FnOnce(&super::Frame, Coords, &Anchor) -> Data) -> Answer {
    let (item, i) = this_point(h, a)?;
    let c = h.dom.coords.get();
    read_path(h, item, |st, _, pd| {
        let fr = frame(st)?;
        let anchor = locate(pd, i).and_then(|(s, j)| pd.subpaths.get(s).and_then(|sp| sp.anchors.get(j)));
        anchor.map(|an| f(&fr, c, an)).ok_or_else(|| HostError::Error(STALE_POINT.into()))
    })?
}

/// Changes the anchor `this` names with `f`, given the point the script assigned in document
/// coordinates (for a point property) or the value itself.
fn edit_point(h: &super::Host, a: &[Data], f: impl FnOnce(&mut Anchor)) -> Answer {
    let (item, i) = this_point(h, a)?;
    let mut subs = read_path(h, item, |_, _, pd| pd.subpaths.clone())?;
    let (s, j) = read_path(h, item, |_, _, pd| locate(pd, i))?.ok_or_else(|| HostError::Error(STALE_POINT.into()))?;
    if let Some(an) = subs.get_mut(s).and_then(|sp| sp.anchors.get_mut(j)) {
        f(an);
    }
    write_path(h, item, subs)?;
    Ok(Data::Undefined)
}

/// The point the script assigned to a point property, in document coordinates.
fn assigned_point(h: &super::Host, a: &[Data], what: &str) -> std::result::Result<Point, HostError> {
    let p = point(arg(a, 1), what)?;
    let (item, _) = this_point(h, a)?;
    let fr = with_doc(h, item.doc, |_, st| frame(st))?;
    Ok(fr.to_document(h.dom.coords.get(), p))
}

fn pt_anchor(h: &super::Host, a: &[Data]) -> Answer {
    read_point(h, a, |f, c, an| pair(f.to_script(c, an.p)))
}

fn pt_set_anchor(h: &super::Host, a: &[Data]) -> Answer {
    let p = assigned_point(h, a, "anchor")?;
    edit_point(h, a, |an| an.p = p)
}

fn pt_left(h: &super::Host, a: &[Data]) -> Answer {
    read_point(h, a, |f, c, an| pair(f.to_script(c, an.h_in)))
}

fn pt_set_left(h: &super::Host, a: &[Data]) -> Answer {
    let p = assigned_point(h, a, "leftDirection")?;
    edit_point(h, a, |an| an.h_in = p)
}

fn pt_right(h: &super::Host, a: &[Data]) -> Answer {
    read_point(h, a, |f, c, an| pair(f.to_script(c, an.h_out)))
}

fn pt_set_right(h: &super::Host, a: &[Data]) -> Answer {
    let p = assigned_point(h, a, "rightDirection")?;
    edit_point(h, a, |an| an.h_out = p)
}

fn pt_type(h: &super::Host, a: &[Data]) -> Answer {
    read_point(h, a, |_, _, an| Data::String(if an.kind == AnchorKind::Smooth { "PointType.SMOOTH" } else { "PointType.CORNER" }.into()))
}

fn pt_set_type(h: &super::Host, a: &[Data]) -> Answer {
    let kind = match arg(a, 1) {
        Data::String(s) if s == "PointType.CORNER" => AnchorKind::Corner,
        Data::String(s) if s == "PointType.SMOOTH" => AnchorKind::Smooth,
        _ => return Err(HostError::Type("pointType must be a PointType value".into())),
    };
    edit_point(h, a, |an| an.kind = kind)
}

fn pt_parent(h: &super::Host, a: &[Data]) -> Answer {
    let (item, _) = this_point(h, a)?;
    Ok(path_ref(h, item))
}

/// `remove()`: deletes the point; a subpath left with no points goes too.
fn pt_remove(h: &super::Host, a: &[Data]) -> Answer {
    let (item, i) = this_point(h, a)?;
    let mut subs = read_path(h, item, |_, _, pd| pd.subpaths.clone())?;
    let (s, j) = read_path(h, item, |_, _, pd| locate(pd, i))?.ok_or_else(|| HostError::Error(STALE_POINT.into()))?;
    if let Some(sp) = subs.get_mut(s) {
        sp.anchors.remove(j);
        if sp.anchors.is_empty() {
            subs.remove(s);
        }
    }
    write_path(h, item, subs)?;
    Ok(Data::Undefined)
}

// ---- Registration ----

/// The members `PathItem` adds to `PageItem`'s.
pub(super) fn path_item_members(l: &Link) -> Vec<(String, HostMember)> {
    let mut m = named(vec![
        ("pathPoints", get(l, path_points)),
        ("closed", property(l, path_closed, path_set_closed)),
        ("evenodd", property(l, path_evenodd, path_set_evenodd)),
        ("area", get(l, path_area)),
        ("length", get(l, path_length)),
        ("guides", get(l, path_guides)),
        ("clipping", get(l, path_clipping)),
        ("setEntirePath", method(l, path_set_entire)),
        ("filled", property(l, path_filled, path_set_filled)),
        ("fillColor", property(l, path_fill_color, path_set_fill_color)),
        ("fillOverprint", property(l, path_fill_overprint, path_set_fill_overprint)),
        ("stroked", property(l, path_stroked, path_set_stroked)),
        ("strokeColor", property(l, path_stroke_color, path_set_stroke_color)),
        ("strokeOverprint", property(l, path_stroke_overprint, path_set_stroke_overprint)),
        ("strokeWidth", property(l, path_stroke_width, path_set_stroke_width)),
        ("strokeCap", property(l, path_stroke_cap, path_set_stroke_cap)),
        ("strokeJoin", property(l, path_stroke_join, path_set_stroke_join)),
        ("strokeMiterLimit", property(l, path_miter, path_set_miter)),
        ("strokeDashes", property(l, path_dashes, path_set_dashes)),
        ("strokeDashOffset", property(l, path_dash_offset, path_set_dash_offset)),
        ("translate", method(l, path_translate)),
        ("rotate", method(l, path_rotate)),
        ("resize", method(l, path_resize)),
    ]);
    m.extend(["selectedPathPoints", "polarity", "resolution"].into_iter().map(|n| later(l, "PathItem", n)));
    m
}

/// `PathItems`, `PathPoints` and `PathPoint`.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    vec![
        class(
            "PathItems",
            None,
            named(vec![
                ("typename", typename_of("PathItems")),
                ("length", get(l, items_length)),
                ("parent", get(l, items_parent)),
                ("index", method(l, items_index)),
                ("getByName", method(l, items_by_name)),
                ("removeAll", method(l, items_remove_all)),
                ("add", method(l, items_add)),
                ("rectangle", method(l, items_rectangle)),
                ("roundedRectangle", method(l, items_rounded_rectangle)),
                ("ellipse", method(l, items_ellipse)),
                ("polygon", method(l, items_polygon)),
                ("star", method(l, items_star)),
            ]),
            Some(super::function(l, items_at)),
        ),
        class(
            "PathPoints",
            None,
            named(vec![
                ("typename", typename_of("PathPoints")),
                ("length", get(l, points_length)),
                ("parent", get(l, points_parent)),
                ("index", method(l, points_index)),
                ("add", method(l, points_add)),
                ("removeAll", method(l, points_remove_all)),
            ]),
            Some(super::function(l, points_at)),
        ),
        class(
            "PathPoint",
            None,
            {
                let mut m = named(vec![
                    ("typename", typename_of("PathPoint")),
                    ("parent", get(l, pt_parent)),
                    ("anchor", property(l, pt_anchor, pt_set_anchor)),
                    ("leftDirection", property(l, pt_left, pt_set_left)),
                    ("rightDirection", property(l, pt_right, pt_set_right)),
                    ("pointType", property(l, pt_type, pt_set_type)),
                    ("remove", method(l, pt_remove)),
                ]);
                m.push(later(l, "PathPoint", "selected"));
                m
            },
            None,
        ),
    ]
}
