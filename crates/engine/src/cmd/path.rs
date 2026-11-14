//! Path editing commands (used by the Pen and Direct Selection tools and Object → Path).

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Scaling, Selection};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, SubPath, Vec2};

use super::create::anchor_from_json;
use super::draw2::usize_opt;
use super::pathops::Beside;
use super::{
    CommandSpec, Result, Session, bad, check_numbers, check_switches, cmd, f64_or, f64_req, finite_numbers, has_anchors, has_doc, has_selected_path,
    has_selection, ids_param, index, number_or, object_param, ok, str_param, switch_or, targets,
};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.appendAnchor",
            "Add Anchor (Pen)",
            [],
            None,
            "{id, x, y, in?: [x,y], out?: [x,y], subpath?: usize (the open subpath to extend, 0 included; an invalid or closed one is an error; default: the last open subpath)} append to the end of an open subpath",
            has_doc,
            append_anchor
        ),
        cmd!(
            "path.close",
            "Close Path",
            [],
            None,
            "{id, subpath?: usize (the open subpath to close, 0 included; default: the last open one), at?: [x,y] (move the first anchor there, its handles with it, before `in` and `out` are placed), in?: [x,y] (handle into the first anchor, placed as path.setHandle places it: a smooth first anchor keeps its outgoing handle in line at its own length, a corner keeps it where it is), out?: [x,y] (the first anchor's outgoing handle, placed after `in`; the anchor becomes a corner), independent?: bool (`in` makes the first anchor a corner and leaves its outgoing handle)}",
            has_doc,
            close
        ),
        cmd!(
            "path.moveAnchors",
            "Move Anchors",
            [],
            None,
            "{dx, dy, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the objects that move whole; default: prefs transformPatternTiles)} move direct-selected anchors (whole paths if fully selected); refused while an object of a symbol instance's art is selected",
            has_selection,
            move_anchors
        ),
        cmd!(
            "path.setHandle",
            "Move Direction Handle",
            [],
            None,
            "{id, subpath?, anchor, which: \"in\"|\"out\", x, y, independent?: bool}",
            has_doc,
            set_handle
        ),
        cmd!(
            "path.moveAnchor",
            "Move Anchor Point",
            [],
            None,
            "{id, subpath?, anchor, x, y} move one anchor to (x, y), its handles with it",
            has_doc,
            move_anchor
        ),
        cmd!(
            "path.setAnchors",
            "Set Anchors",
            [],
            None,
            "{id, subpaths: [{anchors: [{x,y,in?,out?}], closed}]} replace a path's geometry",
            has_doc,
            set_anchors
        ),
        cmd!(
            "path.reverse",
            "Reverse Path Direction",
            ["Object", "Path"],
            None,
            "{ids?|id?, subpath?: usize (reverse only this subpath of each path, 0 included; default: every subpath), reversed?: bool (true: every subpath runs counter-clockwise on screen, false: clockwise, the Attributes panel's Reverse Path Direction On / Off; omitted: flip each path; with subpath, the direction is not read: that subpath is flipped)} the paths in ids or the selection (groups and compound paths: the paths inside), as one undo step; when every subpath of a path turns alike, its strokes' width profiles (sides swapped) and pen pressure, tilt, bearing and rotation turn with it, so they draw as before → {changed: paths}",
            has_selected_path,
            reverse
        ),
        cmd!(
            "path.join",
            "Join",
            ["Object", "Path"],
            Some("Cmd+J"),
            "{ids?: [path, ...] (default: the selected paths; a compound path ID expands to its open member paths), subpaths?: [usize, usize] (which subpaths to join), ends?: [\"first\"|\"last\", \"first\"|\"last\"] (which end of each path to join, for two paths/subpaths; default without ids: direct-selected ends if any, else the nearest pair; default with ids: the nearest pair), to?: \"corner\"|\"smooth\" (the joined anchor: corner retracts both handles, smooth pulls handles in line with the neighbours)} join two or more open paths/subpaths into one (the first keeps its id, selected), or close a single open path; without ids, direct-selected anchors lying on one another along a path merge into one anchor instead",
            has_selected_path,
            join
        ),
        cmd!(
            "path.average",
            "Average…",
            ["Object", "Path"],
            Some("Cmd+Alt+J"),
            "{axis?: \"horizontal\"|\"vertical\"|\"both\"}",
            has_selected_path,
            average
        ),
        cmd!(
            "path.averageJoin",
            "Average & Join…",
            ["Object", "Path"],
            Some("Cmd+Alt+Shift+J"),
            "{to: \"corner\"|\"smooth\" (the joined anchor: corner retracts both handles, smooth pulls handles in line with the neighbours), ids?: [path, ...] (default: the selection; a compound path ID expands to its open member paths)} move two open path ends to their midpoint and join them there, as one undo step: the two direct-selected anchors when they are open path ends, else the nearest ends of the two open paths in ids or the selection (or both ends of one open path, closing it) → the joined path stays selected",
            has_selected_path,
            average_join
        ),
        cmd!(
            "path.convertAnchors",
            "Convert Anchor Points",
            [],
            None,
            "{to: \"corner\"|\"smooth\"} convert the direct-selected anchors (all anchors of fully selected paths, every point of a selected blend's spine; a spine from Replace Spine changes only its handles, its points and the keys' spread along it staying as they are) as one undo step: corner retracts both handles, smooth pulls handles in line with the neighbours (smooth anchors keep theirs)",
            has_selection,
            convert_anchors
        ),
        cmd!(
            "path.deleteAnchors",
            "Delete Anchor Points",
            [],
            None,
            "{} delete the direct-selected anchors and their segments, opening closed paths there, and delete objects selected whole alongside them; what remains of a path stays selected, so a second Delete removes it (the Delete key)",
            has_anchors,
            delete_anchors
        ),
        cmd!(
            "path.reshape",
            "Reshape",
            [],
            None,
            "{id, x, y, dx, dy, tol=3, focal?: [[subpath, anchor]…], radius?} Reshape tool: grab the path at (x, y) (adding an anchor there unless one is within tol) and drag it by (dx, dy). The grabbed anchor and the anchors in focal are focal points and move by the whole drag. Anchors that are not direct-selected stay in place (a path selected whole, or not selected, has every anchor selected), and so do an open path's two ends unless they are focal points. A selected anchor between two focal points moves whole; one between a focal point and an anchor that stays takes a share of the drag that falls smoothly from 1 at the focal point to 0 at the anchor that stays, by its distance along the path's control polygon. A closed subpath with every anchor selected moves whole; a subpath with no focal point does not move. With radius, only the grabbed point's subpath changes: anchors follow with a smooth falloff over radius, measured along the path (0 or more), whatever the selection; focal and radius cannot be used together. Without focal (or with focal null) and without radius, the earlier rule applies: the selection does not matter, an open path stretches from the grabbed point to its two ends, and on a closed path the anchors follow with the falloff over half the diagonal of the subpath's bounding box; the Reshape tool always sends focal",
            has_doc,
            reshape
        ),
        cmd!(
            "path.insertAnchor",
            "Add Anchor Point",
            [],
            None,
            "{id, subpath?, segment, t: 0..1} add an anchor at t on the segment → {anchor}; the direct-selected anchors and selected segments keep the same points (a selected segment it splits stays selected as its two halves), and a path that isn't selected becomes the selection with the new anchor direct-selected",
            has_doc,
            insert_anchor
        ),
        cmd!(
            "path.cutAtAnchors",
            "Cut Path at Selected Anchor Points",
            [],
            None,
            "{} cut the paths at their direct-selected anchors (one undo step): a closed path opens there, an open one becomes one path per piece; one of each cut's two coincident anchors stays selected → {ids}",
            has_anchors,
            cut_at_anchors
        ),
    ]
}

pub(super) fn path_mut(d: &mut vectorcraft_doc::Document, id: NodeId) -> Result<&mut PathData> {
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None; // editing anchors expands a live shape
            Ok(path)
        }
        _ => Err(EngineError::Other(format!("object {id} is not a path"))),
    }
}

fn append_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.appendAnchor")?.ok_or_else(|| bad("path.appendAnchor", "missing id"))?;
    let a = anchor_from_json(p, "path.appendAnchor")?;
    let subpath = p.get("subpath").and_then(Value::as_u64).map(|u| u as usize);
    s.edit("Pen", |d, sel| {
        let path = path_mut(d, id)?;
        let si = match subpath {
            // An explicit subpath is honoured, 0 included, or refused: never swapped for another.
            Some(si) => match path.subpaths.get_mut(si).filter(|sp| !sp.closed) {
                Some(sp) => {
                    sp.anchors.push(a);
                    si
                }
                None => return Err(bad("path.appendAnchor", format!("subpath {si} is not an open subpath"))),
            },
            None => match path.subpaths.iter_mut().enumerate().rev().find(|(_, sp)| !sp.closed) {
                Some((si, sp)) => {
                    sp.anchors.push(a);
                    si
                }
                None => {
                    path.subpaths.push(SubPath::new(vec![a], false));
                    path.subpaths.len() - 1
                }
            },
        };
        let ai = path.subpaths[si].anchors.len() - 1;
        let top = compound_or_self(d, id);
        sel.set([top]);
        sel.anchors.insert(id, BTreeSet::from([(si, ai)]));
        Ok(())
    })?;
    ok()
}

/// Close the active open subpath of `id`, optionally setting the handles of the first anchor. The
/// incoming handle moves as Move Direction Handle moves it: the outgoing handle turns with it only
/// on a smooth anchor, so the first segment of a path that starts at a corner keeps its shape.
/// `at` first moves the anchor, its handles with it (the Pen's closing drag with Space held).
fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.close")?.ok_or_else(|| bad("path.close", "missing id"))?;
    let subpath = p.get("subpath").and_then(Value::as_u64).map(|u| u as usize);
    let handle = |key: &str| match p.get(key).filter(|h| !h.is_null()) {
        None => Ok(None),
        Some(h) => finite_numbers(h).map(|[x, y]| Some(Point::new(x, y))).ok_or_else(|| bad("path.close", format!("`{key}` must be [x, y]"))),
    };
    let at = handle("at")?;
    let h_in = handle("in")?;
    let h_out = handle("out")?;
    let independent = switch_or(p, "independent", false, "path.close")?;
    s.edit("Close Path", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = match subpath {
            Some(si) => path.subpaths.get_mut(si).filter(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?,
            None => path.subpaths.iter_mut().rev().find(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?,
        };
        sp.closed = true;
        if let Some(first) = sp.anchors.first_mut() {
            if let Some(to) = at {
                first.translate(to - first.p);
            }
            if let Some(h) = h_in {
                first.set_handle(false, h, independent);
            }
            if let Some(h) = h_out {
                first.h_out = h;
                first.kind = AnchorKind::Corner;
            }
        }
        let top = compound_or_self(d, id);
        sel.set([top]);
        Ok(())
    })?;
    ok()
}

pub(crate) fn move_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Vec2::new(number_or(p, "dx", 0.0, "path.moveAnchors")?, number_or(p, "dy", 0.0, "path.moveAnchors")?);
    check_switches(p, &["patterns"], "path.moveAnchors")?;
    // An object selected in a symbol instance's art stays where the symbol puts it.
    super::edit::refuse_symbol_part(s)?;
    // Objects selected whole move as objects, and their pattern tiles move with them when
    // `patterns` (default: General › Transform Pattern Tiles) is on.
    let whole = s.doc()?.selection.whole();
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &whole)?, ..Scaling::default() };
    transform_anchors(s, "Move", vectorcraft_geom::Affine::translate(d), sc)?;
    // Transform Again repeats a move that moved something; a drag records it when it commits.
    let st = s.doc_mut()?;
    if st.interaction.is_none() && (d.x != 0.0 || d.y != 0.0) {
        let options = json!({ "copy": false, "patterns": sc.patterns });
        st.last_transform = Some(crate::LastTransform::Matrix(vectorcraft_geom::Affine::translate(d), options));
        st.last_perspective = None;
    }
    ok()
}

/// Apply `xf` to the direct-selected anchors, their handles with them, and (with `sc`) to the
/// objects selected whole (those with no entry in the selection's `anchors`). An object inside
/// another selected object, such as a path in a selected group, moves once, with that object.
pub(crate) fn transform_anchors(s: &mut Session, label: &str, xf: vectorcraft_geom::Affine, sc: Scaling) -> Result<()> {
    let sel = s.doc()?.selection.clone();
    let ids = super::edit::outermost(&s.doc()?.doc, sel.objects.clone());
    let c = xf.as_coeffs();
    let is_translation = (c[0] - 1.0).abs() < 1e-9 && c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && (c[3] - 1.0).abs() < 1e-9;
    let delta = vectorcraft_geom::Vec2::new(c[4], c[5]);
    s.edit(label, |doc, _| {
        for id in &ids {
            match sel.anchors.get(id) {
                Some(set) => {
                    let path = path_mut(doc, *id)?;
                    for &(si, ai) in set {
                        if let Some(a) = path.anchor_mut(si, ai) {
                            *a = a.transform(xf);
                        }
                    }
                    // A pixel-aligned path, or the pixel-aligned compound path that holds it, snaps
                    // to the pixel grid again after its anchors move.
                    let top = compound_or_self(doc, *id);
                    if let Some(n) = doc.node_mut(top) {
                        super::object::snap_if_aligned(n);
                    }
                }
                None => {
                    if let Some(n) = doc.node_mut(*id) {
                        n.transform(xf, sc);
                        super::object::keep_pixel_aligned(n, sc);
                    }
                    super::object::keep_compound_aligned(doc, *id, sc);
                }
            }
        }
        if is_translation && (delta.x != 0.0 || delta.y != 0.0) {
            for &i in &sel.guides {
                doc.edit_ruler_guide(i, |g| *g = g.moved(delta));
            }
        }
        Ok(())
    })
}

fn set_handle(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.setHandle")?.ok_or_else(|| bad("path.setHandle", "missing id"))?;
    let si = usize_opt(p, "subpath", "path.setHandle")?.unwrap_or(0);
    let ai = p.get("anchor").and_then(Value::as_u64).map(index).ok_or_else(|| bad("path.setHandle", "missing anchor"))?;
    let out = str_param(p, "which") != Some("in");
    let pos = Point::new(f64_req(p, "x", "path.setHandle")?, f64_req(p, "y", "path.setHandle")?);
    let independent = switch_or(p, "independent", false, "path.setHandle")?;
    // A handle already there, on an anchor already of the kind it would get, changes nothing and
    // records no undo step.
    if let Some(a) = anchor_of(s, id, si, ai)? {
        let mut b = a;
        b.set_handle(out, pos, independent);
        if b == a {
            return ok();
        }
    }
    s.edit("Reshape", |d, _| {
        let path = path_mut(d, id)?;
        let a = path.anchor_mut(si, ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?;
        a.set_handle(out, pos, independent);
        Ok(())
    })?;
    ok()
}

/// `path.moveAnchor`: one anchor to (x, y), its handles keeping their offsets (the Anchor Point
/// tool's Shift-drag).
fn move_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.moveAnchor")?.ok_or_else(|| bad("path.moveAnchor", "missing id"))?;
    let si = usize_opt(p, "subpath", "path.moveAnchor")?.unwrap_or(0);
    let ai = p.get("anchor").and_then(Value::as_u64).map(index).ok_or_else(|| bad("path.moveAnchor", "missing anchor"))?;
    let pos = Point::new(f64_req(p, "x", "path.moveAnchor")?, f64_req(p, "y", "path.moveAnchor")?);
    // An anchor already at (x, y) changes nothing and records no undo step.
    if let Some(a) = anchor_of(s, id, si, ai)?
        && a.transform(vectorcraft_geom::Affine::translate(pos - a.p)) == a
    {
        return ok();
    }
    s.edit("Move", |d, _| {
        let path = path_mut(d, id)?;
        let a = path.anchor_mut(si, ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?;
        *a = a.transform(vectorcraft_geom::Affine::translate(pos - a.p));
        // A pixel-aligned path, or the pixel-aligned compound path that holds it, snaps to the
        // pixel grid again.
        let top = compound_or_self(d, id);
        if let Some(n) = d.node_mut(top) {
            super::object::snap_if_aligned(n);
        }
        Ok(())
    })?;
    ok()
}

/// Anchor `ai` of subpath `si` of path `id` in the active document, when `id` is a path that has it.
/// Busy while a script runs, as [`Session::edit`] is.
fn anchor_of(s: &Session, id: NodeId, si: usize, ai: usize) -> Result<Option<Anchor>> {
    if s.script_running() {
        return Err(EngineError::Busy);
    }
    let d = &s.doc()?.doc;
    Ok(d.node(id).and_then(|n| n.path_data()).and_then(|path| path.subpaths.get(si)).and_then(|sp| sp.anchors.get(ai)).copied())
}

fn set_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.setAnchors")?.ok_or_else(|| bad("path.setAnchors", "missing id"))?;
    let subs = p.get("subpaths").and_then(Value::as_array).ok_or_else(|| bad("path.setAnchors", "missing subpaths"))?;
    let data = PathData::new(
        subs.iter()
            .map(|sp| {
                let anchors = sp
                    .get("anchors")
                    .and_then(Value::as_array)
                    .ok_or_else(|| bad("path.setAnchors", "each subpath needs an anchors array"))?
                    .iter()
                    .map(|a| anchor_from_json(a, "path.setAnchors"))
                    .collect::<Result<Vec<_>>>()?;
                let closed = match sp.get("closed") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(v)) => *v,
                    Some(_) => return Err(bad("path.setAnchors", "closed must be a boolean")),
                };
                Ok(SubPath::new(anchors, closed))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    s.edit("Reshape", |d, _| {
        *path_mut(d, id)? = data;
        super::object::resnap(d, id);
        Ok(())
    })?;
    ok()
}

/// The compound path enclosing `id`, or `id` itself if it is not inside a compound path.
pub(crate) fn compound_or_self(d: &Document, id: NodeId) -> NodeId {
    d.parent_of(id).filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. }))).unwrap_or(id)
}

/// The open member paths of a compound path, or its first child if none are open.
fn open_members(n: &Node) -> Vec<NodeId> {
    let NodeKind::Compound { children, .. } = &n.kind else { return vec![] };
    let open: Vec<NodeId> =
        children.iter().filter(|c| c.path_data().is_some_and(|pd| pd.subpaths.iter().any(|sp| !sp.closed))).map(|c| c.id).collect();
    if open.is_empty() { children.first().map(|c| vec![c.id]).unwrap_or_default() } else { open }
}

/// The selected paths to join or average, resolving compound paths to their open member paths.
fn selected_paths(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let mut out = vec![];
    for &id in &st.selection.objects {
        let Some(n) = st.doc.node(id) else { continue };
        match &n.kind {
            NodeKind::Path { .. } => out.push(id),
            NodeKind::Compound { .. } => {
                let open = open_members(n);
                let with_anchors: Vec<NodeId> =
                    open.iter().filter(|id| st.selection.anchors.get(id).is_some_and(|s| !s.is_empty())).copied().collect();
                if !with_anchors.is_empty() {
                    out.extend(with_anchors);
                } else {
                    out.extend(open);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The paths in the subtrees of `ids` (in groups and compound paths too), each once.
pub(super) fn paths_in(d: &vectorcraft_doc::Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = BTreeSet::new();
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { guide: false, .. }) {
                out.insert(c.id);
            }
        });
    }
    out.into_iter().collect()
}

fn reverse(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.reverse";
    let want = p.get("reversed").map(|v| v.as_bool().ok_or_else(|| bad(C, "`reversed` must be true or false"))).transpose()?;
    let subpath = p.get("subpath").and_then(Value::as_u64).map(|u| u as usize);
    let ids = paths_in(&s.doc()?.doc, &targets(s, p)?);
    if ids.is_empty() {
        return Err(bad(C, "select paths"));
    }
    let changed = super::overprint::edit_counted(s, "Reverse Path Direction", |d| {
        let mut changed = 0;
        for id in &ids {
            let path = path_mut(d, *id)?;
            let flip: Vec<bool> = match subpath {
                Some(si) => (0..path.subpaths.len()).map(|i| i == si).collect(),
                None => path.subpaths.iter().map(|sp| want.is_none_or(|ccw| (sp.signed_area() < 0.0) != ccw)).collect(),
            };
            if !flip.contains(&true) {
                continue;
            }
            // The stroke's width profile and pen recordings turn with the path, read before it turns.
            if let Some(n) = d.node_mut(*id) {
                vectorcraft_doc::blend::reverse_stroke_records(n, &flip);
            }
            let path = path_mut(d, *id)?;
            for (sp, f) in path.subpaths.iter_mut().zip(flip) {
                if f {
                    sp.reverse();
                }
            }
            changed += 1;
        }
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

/// First and last anchor points of a subpath.
fn end_points(sp: &SubPath) -> Option<(Point, Point)> {
    Some((sp.anchors.first()?.p, sp.anchors.last()?.p))
}

/// Orientations (reverse first, reverse second) that bring the nearest ends together.
fn nearest_ends(sp_a: &SubPath, sp_b: &SubPath) -> Option<(bool, bool)> {
    let (fa, la) = end_points(sp_a)?;
    let (fb, lb) = end_points(sp_b)?;
    let cands = [(la.distance(fb), false, false), (la.distance(lb), false, true), (fa.distance(fb), true, false), (fa.distance(lb), true, true)];
    cands.into_iter().min_by(|p, q| p.0.total_cmp(&q.0)).map(|(_, rx, ry)| (rx, ry))
}

/// Corner or smooth on an anchor a join leaves behind.
fn join_point(sp: &mut SubPath, ai: usize, smooth: bool) {
    if smooth {
        sp.smooth_anchor(ai);
    } else if let Some(a) = sp.anchors.get_mut(ai) {
        a.retract();
    }
}

#[cfg(test)]
thread_local! {
    /// Anchors that [`merge_coincident`] moved to a lower index of their vector on this thread.
    static ANCHOR_MOVES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Merge each run of `picked` neighbours of `sp` that lie on one another into one anchor, which
/// keeps the first one's incoming handle and the last one's outgoing handle. Returns how many
/// anchors went.
fn merge_coincident(sp: &mut SubPath, picked: impl Fn(usize) -> bool, join_kind: Option<bool>) -> usize {
    let same = |a: &Anchor, b: &Anchor| a.p.distance(b.p) < 1e-6;
    let before = sp.anchors.len();
    // The closing pair of a closed path first: taking the last anchor moves no other index.
    if sp.closed
        && before > 2
        && picked(0)
        && picked(before - 1)
        && let [first, .., last] = sp.anchors.as_mut_slice()
        && same(first, last)
    {
        first.h_in = last.h_in;
        sp.anchors.pop();
        if let Some(smooth) = join_kind {
            join_point(sp, 0, smooth);
        }
    }
    // Back to front, so the indices `picked` knows stay true for the anchors still to visit. A
    // merged anchor is only marked here; the marked anchors are removed in one pass at the end.
    let closed = sp.closed;
    let anchors = &mut sp.anchors;
    let len = anchors.len();
    let mut gone = vec![false; len];
    // The anchor after `i` that is still on the path, and the last one still on it.
    let mut next = None;
    let mut last = len.saturating_sub(1);
    for i in (1..len).rev() {
        if picked(i - 1)
            && picked(i)
            && let Some([a, b]) = anchors.get_mut(i - 1..=i)
            && same(a, b)
        {
            a.h_out = b.h_out;
            if let Some(slot) = gone.get_mut(i) {
                *slot = true;
            }
            if next.is_none() {
                last = i - 1;
            }
            if let Some(smooth) = join_kind {
                // The neighbors the anchor has on the path as it now stands, as `join_point` sees them.
                let prev = if i > 1 { Some(i - 2) } else { closed.then_some(last) };
                let after = next.or(closed.then_some(0));
                join_point_between(anchors, i - 1, prev, after, smooth);
            }
        } else {
            next = Some(i);
        }
    }
    #[cfg(test)]
    if let Some(first) = gone.iter().position(|&g| g) {
        // `retain` moves each anchor it keeps after the first one it drops.
        let dropped = gone.iter().filter(|&&g| g).count();
        ANCHOR_MOVES.set(ANCHOR_MOVES.get() + (len - first - dropped));
    }
    if gone.contains(&true) {
        let mut k = 0;
        anchors.retain(|_| {
            let keep = !gone.get(k).copied().unwrap_or(false);
            k += 1;
            keep
        });
    }
    before - sp.anchors.len()
}

/// [`join_point`] on anchor `ai` of `anchors` with its neighbors given by index.
fn join_point_between(anchors: &mut [Anchor], ai: usize, prev: Option<usize>, next: Option<usize>, smooth: bool) {
    let at = |j: Option<usize>| j.and_then(|j| anchors.get(j)).map(|a| a.p);
    let (prev, next) = (at(prev), at(next));
    if let Some(a) = anchors.get_mut(ai) {
        if smooth {
            a.smooth_between(prev, next);
        } else {
            a.retract();
        }
    }
}

/// Close subpath `si` of path `id`, merging coincident endpoints if present.
fn close_subpath_in_node(d: &mut Document, id: NodeId, si: usize, join_kind: Option<bool>) -> Result<()> {
    let path = path_mut(d, id)?;
    let sp = path.subpaths.get_mut(si).filter(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?;
    if sp.anchors.len() > 2
        && end_points(sp).is_some_and(|(f, l)| f.distance(l) < 1e-6)
        && let Some(last) = sp.anchors.pop()
        && let Some(first) = sp.anchors.first_mut()
    {
        first.h_in = last.h_in;
        sp.closed = true;
        if let Some(smooth) = join_kind {
            join_point(sp, 0, smooth);
        }
    }
    sp.closed = true;
    Ok(())
}

/// Join subpaths `sa` and `sb` in the same path node `id`.
fn join_subpaths_in_node(d: &mut Document, id: NodeId, (sa, sb): (usize, usize), (rx, ry): (bool, bool), join_kind: Option<bool>) -> Result<()> {
    if sa == sb {
        return close_subpath_in_node(d, id, sa, join_kind);
    }
    let path = path_mut(d, id)?;
    let mut x = path.subpaths.get(sa).cloned().ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
    let mut y = path.subpaths.get(sb).cloned().ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
    if x.closed || y.closed {
        return Err(EngineError::Other("join needs open paths".into()));
    }
    if rx {
        x.reverse();
    }
    if ry {
        y.reverse();
    }
    let junction = x.anchors.len().saturating_sub(1);
    if let (Some(xe), Some(ys)) = (x.anchors.last().map(|a| a.p), y.anchors.first().map(|a| a.p))
        && xe.distance(ys) < 1e-6
    {
        let first = y.anchors.remove(0);
        if let Some(l) = x.anchors.last_mut() {
            l.h_out = first.h_out;
        }
    }
    x.anchors.extend(y.anchors);
    if let Some(smooth) = join_kind {
        join_point(&mut x, junction, smooth);
    }
    if sa > sb {
        path.subpaths.remove(sb);
        if let Some(sub) = path.subpaths.get_mut(sa - 1) {
            *sub = x;
        }
    } else {
        if let Some(sub) = path.subpaths.get_mut(sa) {
            *sub = x;
        }
        path.subpaths.remove(sb);
    }
    Ok(())
}

/// Join open paths into a single path, or close an open path, preserving compound path structure.
pub(super) fn join(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.join";
    let join_kind = match p.get("to").and_then(Value::as_str) {
        None => None,
        Some("smooth") => Some(true),
        Some("corner") => Some(false),
        Some(_) => return Err(bad(C, "`to` must be \"corner\" or \"smooth\"")),
    };
    let subpaths = match p.get("subpaths") {
        None => None,
        Some(v) => match v.as_array().and_then(|a| {
            if a.len() == 2
                && let (Some(sa), Some(sb)) = (a[0].as_u64(), a[1].as_u64())
            {
                Some((sa as usize, sb as usize))
            } else {
                None
            }
        }) {
            Some(pair) => Some(pair),
            None => return Err(bad(C, "`subpaths` must be two integer indices")),
        },
    };
    // The ends to join, as (reverse the first path, reverse the second): the first path's end
    // must come last, the second's first.
    let ends = match p.get("ends") {
        None => None,
        Some(e) => match e.as_array().map(|a| a.iter().map(Value::as_str).collect::<Vec<_>>()).as_deref() {
            Some([Some(a @ ("first" | "last")), Some(b @ ("first" | "last"))]) => Some((*a == "first", *b == "last")),
            _ => return Err(bad(C, "ends must be two of \"first\" and \"last\"")),
        },
    };

    if let Some(ids) = ids_param(p, "ids", C)? {
        if ends.is_some() && ids.len() > 2 {
            return Err(bad(C, "`ends` applies to joining exactly two paths"));
        }
        if ids.len() == 2 && ids[0] == ids[1] {
            let id = ids[0];
            let (sa, sb) = subpaths.ok_or_else(|| bad(C, "subpaths required when joining the same path"))?;
            let st = s.doc()?;
            let p = st.doc.node(id).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(id))?;
            let spa = p.subpaths.get(sa).ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
            let spb = p.subpaths.get(sb).ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
            let (rx, ry) = match ends {
                Some(e) => e,
                None => nearest_ends(spa, spb).unwrap_or((false, false)),
            };
            s.edit("Join", |d, sel| {
                join_subpaths_in_node(d, id, (sa, sb), (rx, ry), join_kind)?;
                sel.set([compound_or_self(d, id)]);
                Ok(())
            })?;
            return ok();
        }
        if ids.len() == 2
            && let Some((sa, sb)) = subpaths
        {
            let (a, b) = (ids[0], ids[1]);
            let st = s.doc()?;
            let pa = st.doc.node(a).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(a))?;
            let pb = st.doc.node(b).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(b))?;
            let spa = pa.subpaths.get(sa).ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
            let spb = pb.subpaths.get(sb).ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
            let (rx, ry) = match ends {
                Some(e) => e,
                None => nearest_ends(spa, spb).unwrap_or((false, false)),
            };
            s.edit("Join", |d, sel| {
                join_two(d, a, b, (sa, sb), (rx, ry), join_kind)?;
                sel.set([compound_or_self(d, a)]);
                Ok(())
            })?;
            return ok();
        }

        let st = s.doc()?;
        let mut resolved = vec![];
        for &id in &ids {
            let Some(n) = st.doc.node(id) else {
                return Err(bad(C, format!("{} isn't a path", id.0)));
            };
            match &n.kind {
                NodeKind::Path { .. } => resolved.push(id),
                NodeKind::Compound { .. } => {
                    let open = open_members(n);
                    if ids.len() > 1 && open.len() != 1 {
                        return Err(bad(C, format!("compound {} must contain exactly one open path to join", id.0)));
                    }
                    resolved.extend(open);
                }
                _ => return Err(bad(C, format!("{} isn't a path", id.0))),
            }
        }
        match resolved.as_slice() {
            [one] => {
                let id = *one;
                let si = match subpaths {
                    Some((si, _)) => si,
                    None => {
                        let pd = s.doc()?.doc.node(id).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(id))?;
                        pd.subpaths
                            .iter()
                            .position(|sp| !sp.closed && !sp.anchors.is_empty())
                            .ok_or_else(|| EngineError::Other("path is already closed".into()))?
                    }
                };
                s.edit("Join", |d, sel| {
                    close_subpath_in_node(d, id, si, join_kind)?;
                    sel.set([compound_or_self(d, id)]);
                    Ok(())
                })?;
            }
            [a, b, ..] => {
                if resolved.iter().collect::<BTreeSet<_>>().len() != resolved.len() {
                    return Err(bad(C, "join two different paths"));
                }
                let (a, b) = (*a, *b);
                if ends.is_some() && resolved.len() > 2 {
                    return Err(bad(C, "`ends` applies to joining exactly two paths"));
                }
                if let Some((rx, ry)) = ends {
                    let st = s.doc()?;
                    let pa = st.doc.node(a).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(a))?;
                    let pb = st.doc.node(b).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(b))?;
                    let sa = match subpaths {
                        Some((s, _)) => s,
                        None => pa.subpaths.iter().position(|s| !s.closed).ok_or_else(|| EngineError::Other("join needs open paths".into()))?,
                    };
                    let sb = match subpaths {
                        Some((_, s)) => s,
                        None => pb.subpaths.iter().position(|s| !s.closed).ok_or_else(|| EngineError::Other("join needs open paths".into()))?,
                    };
                    s.edit("Join", |d, sel| {
                        join_two(d, a, b, (sa, sb), (rx, ry), join_kind)?;
                        sel.set([compound_or_self(d, a)]);
                        Ok(())
                    })?;
                    return ok();
                }
                let st = s.doc()?;
                let mut open_subs = Vec::with_capacity(resolved.len());
                for &id in &resolved {
                    let pd = st.doc.node(id).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(id))?;
                    let si = pd
                        .subpaths
                        .iter()
                        .position(|sp| !sp.closed && !sp.anchors.is_empty())
                        .ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
                    open_subs.push((id, si));
                }
                s.edit("Join", |d, sel| {
                    let top = join_nearest(d, open_subs, join_kind)?;
                    sel.set([top]);
                    Ok(())
                })?;
            }
            [] => return Err(EngineError::Other("select paths to join".into())),
        }
        return ok();
    }

    // Direct-selected anchors lying on one another along a path (two corners brought together by
    // Average) become one anchor; the paths themselves are joined only when there are none.
    {
        let st = s.doc()?;
        let mut merged = vec![];
        for (id, picked) in &st.selection.anchors {
            let Some(mut path) = st.doc.node(*id).and_then(|n| n.path_data()).cloned() else { continue };
            // One flag per anchor; references past the end of a subpath or to a missing one pick nothing.
            let mut masks: Vec<Vec<bool>> = path.subpaths.iter().map(|sp| vec![false; sp.anchors.len()]).collect();
            for &(si, ai) in picked {
                if let Some(flag) = masks.get_mut(si).and_then(|m| m.get_mut(ai)) {
                    *flag = true;
                }
            }
            let n: usize = path
                .subpaths
                .iter_mut()
                .zip(&masks)
                .map(|(sp, mask)| merge_coincident(sp, |ai| mask.get(ai).copied().unwrap_or(false), join_kind))
                .sum();
            if n > 0 {
                merged.push((*id, path));
            }
        }
        if !merged.is_empty() {
            let objects = st.selection.objects.clone();
            s.edit("Join", |d, sel| {
                for (id, path) in merged {
                    *path_mut(d, id)? = path;
                }
                sel.set(objects);
                Ok(())
            })?;
            return ok();
        }
    }

    // An explicit `ends` names the ends to join: the direct selection only decides when it is absent.
    let direct_ends: Vec<(NodeId, usize, &'static str)> = if ends.is_some() {
        vec![]
    } else {
        let st = s.doc()?;
        let mut ends = vec![];
        for (&id, picked) in &st.selection.anchors {
            if let Some(pd) = st.doc.node(id).and_then(|n| n.path_data()) {
                for (si, sp) in pd.subpaths.iter().enumerate() {
                    if sp.closed || sp.anchors.is_empty() {
                        continue;
                    }
                    let last_idx = sp.anchors.len() - 1;
                    let f = picked.contains(&(si, 0));
                    let l = picked.contains(&(si, last_idx));
                    if f && l && last_idx > 0 {
                        ends.push((id, si, "both"));
                    } else if f {
                        ends.push((id, si, "first"));
                    } else if l {
                        ends.push((id, si, "last"));
                    }
                }
            }
        }
        ends
    };

    // More than two selected ends can't name one join: refuse it instead of joining everything.
    // A single end (left selected by the Pen) falls through to joining the selected paths.
    if direct_ends.len() > 2 {
        return Err(bad(C, "select exactly two path ends to join"));
    }

    if direct_ends.len() == 1 && direct_ends[0].2 == "both" {
        let (id, si, _) = direct_ends[0];
        s.edit("Join", |d, sel| {
            close_subpath_in_node(d, id, si, join_kind)?;
            sel.set([compound_or_self(d, id)]);
            Ok(())
        })?;
        return ok();
    }

    if direct_ends.len() == 2 {
        let (ida, sa, ea) = direct_ends[0];
        let (idb, sb, eb) = direct_ends[1];
        let (rx, ry) = match (ea, eb) {
            ("first", "first") => (true, false),
            ("first", "last") => (true, true),
            ("last", "first") => (false, false),
            ("last", "last") => (false, true),
            _ => {
                let st = s.doc()?;
                let pa = st.doc.node(ida).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(ida))?;
                let pb = st.doc.node(idb).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(idb))?;
                let (fa, la) = pa.subpaths.get(sa).and_then(end_points).ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
                let (fb, lb) = pb.subpaths.get(sb).and_then(end_points).ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
                let mut best: Option<(bool, bool, f64)> = None;
                let cands = [
                    (la.distance(fb), false, false, (ea == "last" || ea == "both") && (eb == "first" || eb == "both")),
                    (la.distance(lb), false, true, (ea == "last" || ea == "both") && (eb == "last" || eb == "both")),
                    (fa.distance(fb), true, false, (ea == "first" || ea == "both") && (eb == "first" || eb == "both")),
                    (fa.distance(lb), true, true, (ea == "first" || ea == "both") && (eb == "last" || eb == "both")),
                ];
                for (dist, rx_c, ry_c, valid) in cands {
                    if valid && best.as_ref().is_none_or(|b| dist < b.2) {
                        best = Some((rx_c, ry_c, dist));
                    }
                }
                match best {
                    Some((rx, ry, _)) => (rx, ry),
                    None => (ea == "first", eb == "last"),
                }
            }
        };
        s.edit("Join", |d, sel| {
            sel.set([join_pair(d, (ida, sa), (idb, sb), (rx, ry), join_kind)?]);
            Ok(())
        })?;
        return ok();
    }

    let ids = selected_paths(s)?;
    if ids.is_empty() {
        return Err(EngineError::Other("select paths to join".into()));
    }
    let mut open_subs: Vec<(NodeId, usize)> = vec![];
    {
        let st = s.doc()?;
        for &id in &ids {
            if let Some(pd) = st.doc.node(id).and_then(|n| n.path_data()) {
                for (si, sp) in pd.subpaths.iter().enumerate() {
                    if !sp.closed && !sp.anchors.is_empty() {
                        open_subs.push((id, si));
                    }
                }
            }
        }
    }
    if open_subs.is_empty() {
        if ids.len() == 1 {
            return Err(EngineError::Other("path is already closed".into()));
        }
        return Err(EngineError::Other("select paths to join".into()));
    }
    if ends.is_some() && open_subs.len() > 2 {
        return Err(bad(C, "`ends` applies to joining exactly two paths"));
    }
    if open_subs.len() == 1 {
        let (id, si) = open_subs[0];
        s.edit("Join", |d, sel| {
            close_subpath_in_node(d, id, si, join_kind)?;
            sel.set([compound_or_self(d, id)]);
            Ok(())
        })?;
        return ok();
    }
    if let Some((rx, ry)) = ends
        && open_subs.len() == 2
    {
        let (a, b) = (open_subs[0], open_subs[1]);
        s.edit("Join", |d, sel| {
            sel.set([join_pair(d, a, b, (rx, ry), join_kind)?]);
            Ok(())
        })?;
        return ok();
    }

    s.edit("Join", |d, sel| {
        let top = join_nearest(d, open_subs, join_kind)?;
        sel.set([top]);
        Ok(())
    })?;
    ok()
}

/// Join open subpaths, always the two whose ends lie nearest each other, until one is left.
/// `open_subs` lists `(node, subpath)` pairs. Returns the node holding the result (or its compound).
fn join_nearest(d: &mut Document, mut open_subs: Vec<(NodeId, usize)>, join_kind: Option<bool>) -> Result<NodeId> {
    let err = || EngineError::Other("join needs open paths".into());
    while open_subs.len() > 1 {
        let mut ends_info = Vec::with_capacity(open_subs.len());
        for &(id, si) in &open_subs {
            let p = d.node(id).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(id))?;
            let sp = p.subpaths.get(si).ok_or_else(err)?;
            ends_info.push(end_points(sp).ok_or_else(err)?);
        }
        let mut best: Option<(usize, usize, bool, bool, f64)> = None;
        for (i, &(fa, la)) in ends_info.iter().enumerate() {
            for (j, &(fb, lb)) in ends_info.iter().enumerate().skip(i + 1) {
                let cands =
                    [(la.distance(fb), false, false), (la.distance(lb), false, true), (fa.distance(fb), true, false), (fa.distance(lb), true, true)];
                for (dist, rx, ry) in cands {
                    if best.as_ref().is_none_or(|b| dist < b.4) {
                        best = Some((i, j, rx, ry, dist));
                    }
                }
            }
        }
        let Some((i, j, rx, ry, _)) = best else { return Err(err()) };
        let ((ida, sa), (idb, sb)) = (open_subs[i], open_subs[j]);
        if ida == idb {
            join_subpaths_in_node(d, ida, (sa, sb), (rx, ry), join_kind)?;
            // Subpath `sb` is gone and `sa` took the merged result; later subpaths shift down.
            open_subs.remove(j);
            if sa > sb {
                open_subs[i].1 -= 1;
            }
            for (k, item) in open_subs.iter_mut().enumerate() {
                if k != i && item.0 == ida && item.1 > sb {
                    item.1 -= 1;
                }
            }
        } else {
            // `join_two` removes `b` only when `sb` was its last subpath; otherwise `b` keeps its
            // other subpaths, which stay in `open_subs` with their indexes shifted down.
            join_two(d, ida, idb, (sa, sb), (rx, ry), join_kind)?;
            open_subs.remove(j);
            for item in &mut open_subs {
                if item.0 == idb && item.1 > sb {
                    item.1 -= 1;
                }
            }
        }
    }
    let first = open_subs.first().map(|(id, _)| *id).ok_or_else(err)?;
    Ok(compound_or_self(d, first))
}

/// Join path `b` into path `a` along the specified subpaths and orientations.
fn join_two(d: &mut Document, a: NodeId, b: NodeId, (sa, sb): (usize, usize), (rx, ry): (bool, bool), join_kind: Option<bool>) -> Result<()> {
    let pb = d.node(b).and_then(|n| n.path_data()).cloned().ok_or(EngineError::NoNode(b))?;
    let pa = path_mut(d, a)?;
    let mut x = pa.subpaths.get(sa).cloned().ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
    let mut y = pb.subpaths.get(sb).cloned().ok_or_else(|| EngineError::Other("join needs open paths".into()))?;
    if x.closed || y.closed || x.anchors.is_empty() || y.anchors.is_empty() {
        return Err(EngineError::Other("join needs open paths".into()));
    }
    if rx {
        x.reverse();
    }
    if ry {
        y.reverse();
    }
    let junction = x.anchors.len().saturating_sub(1);
    if let (Some(xe), Some(ys)) = (x.anchors.last().map(|a| a.p), y.anchors.first().map(|a| a.p))
        && xe.distance(ys) < 1e-6
    {
        let first = y.anchors.remove(0);
        if let Some(l) = x.anchors.last_mut() {
            l.h_out = first.h_out;
        }
    }
    x.anchors.extend(y.anchors);
    if let Some(smooth) = join_kind {
        join_point(&mut x, junction, smooth);
    }
    if let Some(sub) = pa.subpaths.get_mut(sa) {
        *sub = x;
    }
    if pb.subpaths.len() <= 1 {
        let parent = d.parent_of(b).filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. })));
        d.remove(b)?;
        if let Some(parent) = parent
            && d.node(parent).is_some_and(|n| n.children().is_some_and(|children| children.is_empty()))
        {
            d.remove(parent)?;
        }
    } else {
        let pb_mut = path_mut(d, b)?;
        if sb < pb_mut.subpaths.len() {
            pb_mut.subpaths.remove(sb);
        }
    }
    Ok(())
}

/// Join subpath `sa` of path `a` and subpath `sb` of path `b` oriented as [`join_two`] takes them
/// (one subpath: close it). Returns what to select: the joined path, or its compound path.
fn join_pair(d: &mut Document, (a, sa): (NodeId, usize), (b, sb): (NodeId, usize), r: (bool, bool), join_kind: Option<bool>) -> Result<NodeId> {
    if a == b {
        join_subpaths_in_node(d, a, (sa, sb), r, join_kind)?;
    } else {
        join_two(d, a, b, (sa, sb), r, join_kind)?;
    }
    Ok(compound_or_self(d, a))
}

pub(super) type AnchorTargets = Vec<(NodeId, Vec<(usize, usize)>)>;

pub(super) fn anchor_targets(s: &Session) -> Result<AnchorTargets> {
    let st = s.doc()?;
    let mut out = vec![];
    for id in &st.selection.objects {
        let Some(pd) = st.doc.node(*id).and_then(|n| n.path_data()) else { continue };
        let v: Vec<(usize, usize)> = match st.selection.anchors.get(id) {
            Some(set) => set.iter().copied().collect(),
            None => pd.anchors().map(|(s, a, _)| (s, a)).collect(),
        };
        out.push((*id, v));
    }
    Ok(out)
}

fn average(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let t: AnchorTargets = if st.selection.anchors.is_empty() {
        let editable = st.doc.editable();
        paths_in(&st.doc, &st.selection.objects)
            .into_iter()
            .filter(|id| editable(*id))
            .filter_map(|id| {
                let pd = st.doc.node(id)?.path_data()?;
                Some((id, pd.anchors().map(|(s, a, _)| (s, a)).collect()))
            })
            .collect()
    } else {
        anchor_targets(s)?
    };
    let d0 = &st.doc;
    let pts: Vec<Point> = t
        .iter()
        .flat_map(|(id, v)| v.iter().filter_map(move |(si, ai)| d0.node(*id)?.path_data()?.subpaths.get(*si)?.anchors.get(*ai).map(|a| a.p)))
        .collect();
    if pts.is_empty() {
        return ok();
    }
    let c = Point::new(pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64, pts.iter().map(|p| p.y).sum::<f64>() / pts.len() as f64);
    let axis = str_param(p, "axis").unwrap_or("both").to_string();
    s.edit("Average", |d, _| {
        for (id, v) in &t {
            let path = path_mut(d, *id)?;
            for &(si, ai) in v {
                if let Some(a) = path.anchor_mut(si, ai) {
                    let target = match axis.as_str() {
                        "horizontal" => Point::new(a.p.x, c.y),
                        "vertical" => Point::new(c.x, a.p.y),
                        _ => c,
                    };
                    a.translate(target - a.p);
                }
            }
        }
        Ok(())
    })?;
    ok()
}

/// An open path end: (path, subpath, anchor).
type End = (NodeId, usize, usize);

/// Average & Join: the two ends Average moves to their midpoint, joined there as Join joins them.
fn average_join(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.averageJoin";
    let smooth = match str_param(p, "to") {
        Some("smooth") => true,
        Some("corner") => false,
        _ => return Err(bad(C, "`to` must be \"corner\" or \"smooth\"")),
    };
    let ends = average_join_ends(s, p, C)?;
    let [a, b] = ends.ok_or_else(|| bad(C, "select two open path ends, or two open paths, to average and join"))?;
    let doc = &s.doc()?.doc;
    let point = |(id, si, ai): End| doc.node(id)?.path_data()?.subpaths.get(si)?.anchors.get(ai).map(|a| a.p);
    let (Some(pa), Some(pb)) = (point(a), point(b)) else { return Err(bad(C, "join needs open paths")) };
    let mid = pa.midpoint(pb);
    // The first end must come last in its subpath, the second first (as `ends` takes them in Join).
    let r = (a.2 == 0, b.2 != 0);
    s.edit("Average & Join", |d, sel| {
        for (id, si, ai) in [a, b] {
            if let Some(anchor) = path_mut(d, id)?.anchor_mut(si, ai) {
                anchor.translate(mid - anchor.p);
            }
        }
        sel.set([join_pair(d, (a.0, a.1), (b.0, b.1), r, Some(smooth))?]);
        Ok(())
    })?;
    ok()
}

/// The two ends Average & Join brings together: the two direct-selected anchors when they are
/// open path ends; else, of the paths in `ids` or the selected ones, the nearest ends of their two
/// open subpaths, or both ends of their one. `None` when there are no such two ends, or when they
/// are the two ends of a line, which the midpoint would collapse.
fn average_join_ends(s: &Session, p: &Value, c: &str) -> Result<Option<[End; 2]>> {
    let st = s.doc()?;
    let doc = &st.doc;
    let subpath = |id: NodeId, si: usize| doc.node(id)?.path_data()?.subpaths.get(si).filter(|sp| !sp.closed && !sp.anchors.is_empty());
    let last = |sp: &SubPath| sp.anchors.len().saturating_sub(1);
    let ids = ids_param(p, "ids", c)?;
    let ends = if ids.is_none() && !st.selection.anchors.is_empty() {
        let picked: Vec<End> = st.selection.anchors.iter().flat_map(|(&id, set)| set.iter().map(move |&(si, ai)| (id, si, ai))).collect();
        match picked.as_slice() {
            &[a, b] if [a, b].iter().all(|&(id, si, ai)| subpath(id, si).is_some_and(|sp| ai == 0 || ai == last(sp))) => [a, b],
            _ => return Ok(None),
        }
    } else {
        let paths = match ids {
            None => selected_paths(s)?,
            Some(ids) => {
                let mut out = vec![];
                for id in ids {
                    match doc.node(id).map(|n| (n, &n.kind)) {
                        Some((_, NodeKind::Path { .. })) => out.push(id),
                        Some((n, NodeKind::Compound { .. })) => out.extend(open_members(n)),
                        _ => return Err(bad(c, format!("{} isn't a path", id.0))),
                    }
                }
                out
            }
        };
        let open: Vec<(NodeId, usize, &SubPath)> = paths
            .iter()
            .filter_map(|&id| Some((id, doc.node(id)?.path_data()?)))
            .flat_map(|(id, pd)| {
                pd.subpaths.iter().enumerate().filter(|(_, sp)| !sp.closed && !sp.anchors.is_empty()).map(move |(si, sp)| (id, si, sp))
            })
            .collect();
        match *open.as_slice() {
            [(id, si, sp)] => [(id, si, 0), (id, si, last(sp))],
            [(ia, sa, x), (ib, sb, y)] => {
                let Some((rx, ry)) = nearest_ends(x, y) else { return Ok(None) };
                [(ia, sa, if rx { 0 } else { last(x) }), (ib, sb, if ry { last(y) } else { 0 })]
            }
            _ => return Ok(None),
        }
    };
    // A subpath's two ends: it needs an anchor between them to close around.
    let [a, b] = ends;
    let line = (a.0, a.1) == (b.0, b.1) && subpath(a.0, a.1).is_none_or(|sp| sp.anchors.len() <= 2);
    Ok((!line).then_some(ends))
}

fn convert_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.convertAnchors";
    let smooth = match str_param(p, "to") {
        Some("smooth") => true,
        Some("corner") => false,
        _ => return Err(bad(C, "`to` must be \"corner\" or \"smooth\"")),
    };
    let t = anchor_targets(s)?;
    // A selected blend converts every point of its spine, as a path selected whole does.
    let st = s.doc()?;
    let blends: Vec<NodeId> =
        st.selection.objects.iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. }))).collect();
    if t.is_empty() && blends.is_empty() {
        return Err(bad(C, "select paths or anchor points"));
    }
    s.edit("Convert Anchor Points", |d, _| {
        for (id, v) in &t {
            let path = path_mut(d, *id)?;
            for &(si, ai) in v {
                let Some(sp) = path.subpaths.get_mut(si) else { continue };
                if smooth {
                    sp.smooth_anchor(ai);
                } else if let Some(a) = sp.anchors.get_mut(ai) {
                    a.retract();
                }
            }
        }
        let convert = |sp: &mut SubPath| {
            for ai in 0..sp.anchors.len() {
                if smooth {
                    sp.smooth_anchor(ai);
                } else if let Some(a) = sp.anchors.get_mut(ai) {
                    a.retract();
                }
            }
        };
        for id in &blends {
            let Some(NodeKind::Blend { children, spec }) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            // A spine from Replace Spine keeps its points and its keys spread along it: only the
            // handles change, and the keys are not pinned to points.
            if vectorcraft_doc::live::blend_spine(children, spec).is_some_and(|(_, anchors)| anchors.is_none()) {
                if let Some(sp) = spec.spine.as_mut().and_then(|p| p.subpaths.first_mut()) {
                    convert(sp);
                }
                continue;
            }
            let Some((mut path, anchors)) = vectorcraft_doc::live::pin_spine(children, spec) else { continue };
            let pinned = path.clone();
            let Some(sp) = path.subpaths.first_mut() else { continue };
            convert(sp);
            // A spine the conversion leaves as it was stays as the blend has it.
            if path != pinned {
                spec.key_anchors = anchors.iter().map(|a| u32::try_from(*a).unwrap_or(u32::MAX)).collect();
                spec.spine = Some(path);
            }
        }
        Ok(())
    })?;
    ok()
}

/// Delete direct-selected anchors or segments and whole selected objects in one edit.
/// Leave surviving pieces selected, splitting ordinary contours into independent sibling objects.
/// `p.guides: true` also removes the selected ruler guides in that edit; otherwise guides remain.
/// Layers are retained, and an object removed with a selected ancestor is not removed twice.
/// Returns JSON `null` on success; selection is replaced by the surviving object IDs.
pub(crate) fn delete_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    // `guides: true` (Cut, #3226): the selected ruler guides go in the same undo step.
    let with_guides = switch_or(p, "guides", false, "path.deleteAnchors")?;
    let st = s.doc()?;
    let sel = st.selection.anchors.clone();
    let segments = st.selection.segments.clone();
    // Every anchor selected stores the path as an object, with no `anchors` entry. A marquee that
    // also catches part of another path must still delete those whole objects.
    let whole = st.selection.whole();
    s.edit("Clear", |d, selection| {
        if with_guides {
            let gone = selection.guides.clone();
            d.retain_guides(selection, |g| !g.id.is_some_and(|id| gone.contains(&id)));
        }
        let targets: BTreeSet<NodeId> = whole.into_iter().filter(|id| d.node(*id).is_some_and(|n| !n.is_layer())).collect();
        let whole_ids: Vec<NodeId> = targets
            .iter()
            .copied()
            .filter(|id| !d.ancestry(*id).unwrap_or_default().iter().rev().skip(1).any(|ancestor| targets.contains(ancestor)))
            .collect();
        // The compound paths that hold a deleted object; one left with no members goes too.
        let compounds: BTreeSet<NodeId> = whole_ids
            .iter()
            .chain(sel.keys())
            .filter_map(|id| d.parent_of(*id).filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. }))))
            .collect();
        if !whole_ids.is_empty() {
            d.remove_many(&whole_ids)?;
        }
        let mut survivors = vec![];
        let remaining: Vec<NodeId> = sel.keys().copied().filter(|id| d.node(*id).is_some()).collect();
        let mut emptied = Beside::replacing(d, &remaining);
        for (id, set) in &sel {
            // The path went with a fully selected group around it.
            if d.node(*id).is_none() {
                continue;
            }
            let original = deletion_source(d, *id)?;
            let subpaths: Vec<SubPath> = {
                let path = path_mut(d, *id)?;
                let subpaths = std::mem::take(&mut path.subpaths);
                if let Some(refs) = segments.get(id) {
                    // A segment pick cuts its connection, keeping every neighboring cubic.
                    subpaths
                        .into_iter()
                        .enumerate()
                        .flat_map(|(si, sp)| {
                            let cuts = subpath_refs(refs, si).collect();
                            without_segments(sp, &cuts)
                        })
                        .filter(|s| s.anchors.len() >= 2)
                        .collect()
                } else {
                    // Remove every selected anchor against the original indexes, keeping surviving
                    // stretches separate rather than joining across deleted anchors.
                    subpaths
                        .into_iter()
                        .enumerate()
                        .flat_map(|(si, sp)| {
                            let gone = subpath_refs(set, si).collect();
                            without_anchors(sp, &gone)
                        })
                        .filter(|s| s.anchors.len() >= 2)
                        .collect()
                }
            };
            if subpaths.is_empty() {
                emptied.put(d, *id, vec![])?;
            } else {
                // Every remaining piece is selected whole, so the next Delete removes them.
                survivors.extend(replace_deleted_pieces(d, &mut emptied, original, subpaths)?);
            }
        }
        emptied.finish(d)?;
        for p in compounds {
            if d.node(p).is_some_and(|n| n.children().is_some_and(|c| c.is_empty())) {
                d.remove(p)?;
            }
        }
        selection.set(survivors);
        Ok(())
    })?;
    ok()
}

/// The anchor or segment indexes in `refs` that belong to subpath `si`, in index order.
/// `refs` is ordered by subpath first, so this reads only that subpath's entries.
pub(super) fn subpath_refs(refs: &BTreeSet<(usize, usize)>, si: usize) -> impl Iterator<Item = usize> + '_ {
    refs.range((si, 0)..=(si, usize::MAX)).map(|&(_, i)| i)
}

/// Retain path `id` from `d` without copying its geometry before deletion takes its subpaths.
/// The shared snapshot supplies the original path classification and, only if needed, gradient
/// bounds after the working node's geometry has moved. Returns `NoNode` for a missing/non-path ID.
pub(super) fn deletion_source(d: &Document, id: NodeId) -> Result<Arc<Node>> {
    let (parent, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    // Verify the cached position still names this path before retaining the shared node.
    d.children(parent)
        .and_then(|children| children.get(index))
        .filter(|n| n.id == id && n.path_data().is_some())
        .cloned()
        .ok_or(EngineError::NoNode(id))
}

/// Replace the deleted path in `d` with the nonempty surviving runs in `subpaths`.
/// `original` must be retained by [`deletion_source`] before taking the working node's geometry.
/// An ordinary single-contour source becomes siblings; multi-subpath paths and compound members
/// retain their node. Only actual splits pin unplaced gradients to `original`'s bounds; already
/// placed gradients and connected remainders retain their placement behavior.
///
/// Returns the surviving IDs in `subpaths` order (the first keeps the source ID). The original
/// node's geometry is updated immediately. Replacements are handed to `beside`, which may defer
/// insertion; the caller must finish that [`Beside`] before using new IDs in `d`.
/// Missing/invalid paths return an error.
pub(super) fn replace_deleted_pieces(d: &mut Document, beside: &mut Beside, original: Arc<Node>, subpaths: Vec<SubPath>) -> Result<Vec<NodeId>> {
    let id = original.id;
    let single = original.path_data().is_some_and(|p| p.subpaths.len() == 1);
    if single && compound_or_self(d, id) == id {
        // Pin only actual splits: each new object's smaller bounds would otherwise refit its gradients.
        if subpaths.len() > 1
            && original.appearance.has_unplaced_gradient()
            && let Some(bounds) = original.geometric_bounds()
        {
            d.node_mut(id).ok_or(EngineError::NoNode(id))?.appearance.pin_gradients(bounds);
        }
        super::draw2::replace_with_pieces(d, beside, id, subpaths.into_iter().map(PathData::single).collect())
    } else {
        path_mut(d, id)?.subpaths = subpaths;
        Ok(vec![id])
    }
}

/// Remove anchors and their incident segments, keeping each surviving stretch as an open path.
/// Only a closed path can have a surviving stretch that wraps past the last anchor.
pub(super) fn without_anchors(sp: SubPath, removed: &BTreeSet<usize>) -> Vec<SubPath> {
    let n = sp.anchors.len();
    let gone: BTreeSet<usize> = removed.iter().copied().filter(|&i| i < n).collect();
    if gone.is_empty() {
        return vec![sp];
    }
    let mut runs: Vec<Vec<Anchor>> = vec![];
    let mut run = vec![];
    for (i, anchor) in sp.anchors.into_iter().enumerate() {
        if gone.contains(&i) {
            if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        } else {
            run.push(anchor);
        }
    }
    if !sp.closed {
        runs.push(run);
        return runs.into_iter().filter_map(open_run).collect();
    }
    // The anchors before the first deletion and after the last are one closed-path stretch.
    if gone.contains(&0) {
        if !run.is_empty() {
            runs.push(run);
        }
    } else if !runs.is_empty() {
        let head = runs.remove(0);
        run.extend(head);
        runs.insert(0, run);
    } else if !run.is_empty() {
        runs.push(run);
    }
    runs.into_iter().filter_map(open_run).collect()
}

/// Remove connections, splitting open/closed contours into their surviving open runs.
pub(super) fn without_segments(sp: SubPath, removed: &BTreeSet<usize>) -> Vec<SubPath> {
    let cuts: BTreeSet<usize> = removed.iter().copied().filter(|&i| i < sp.segment_count()).collect();
    let Some(&first) = cuts.first() else { return vec![sp] };
    let n = sp.anchors.len();
    let start = if sp.closed { (first + 1) % n } else { 0 };
    let mut result = vec![];
    let mut run = vec![];
    let mut previous = None;
    for (index, anchor) in sp.anchors.iter().enumerate().cycle().skip(start).take(n) {
        if previous.is_some_and(|i| cuts.contains(&i)) {
            result.extend(open_run(std::mem::take(&mut run)));
        }
        run.push(*anchor);
        previous = Some(index);
    }
    result.extend(open_run(run));
    result
}

fn open_run(mut anchors: Vec<vectorcraft_geom::Anchor>) -> Option<SubPath> {
    if anchors.len() < 2 {
        return None;
    }
    // The cut edge's incoming/outgoing controls are no longer part of an open endpoint.
    if let Some(first) = anchors.first_mut() {
        first.h_in = first.p;
    }
    if let Some(last) = anchors.last_mut() {
        last.h_out = last.p;
    }
    Some(SubPath::new(anchors, false))
}

/// What an anchor does in a Reshape drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stretch {
    /// Not selected: it stays in place.
    Fixed,
    /// Selected: it takes a share of the drag that depends on where it lies between the anchors
    /// that stay and the focal points.
    Selected,
    /// A focal point (the grabbed anchor, or one the Reshape tool highlighted): it moves by the
    /// whole drag.
    Focal,
}

/// The share of a Reshape drag that each anchor of a subpath takes, as `[in handle, anchor, out
/// handle]`, from what each anchor does (`roles`, one per anchor). A focal point takes all of the
/// drag and an anchor that stays takes none. An open path's end anchors stay unless they are focal
/// points. A selected anchor between two focal points takes all of the drag, and one between two
/// anchors that stay takes none. For a selected anchor between an anchor that stays and a focal
/// point, `u` is its distance from the anchor that stays along the control polygon (anchor, out
/// handle, the next in handle, the next anchor) divided by the focal point's distance from it, and
/// its share is `u² (3 - 2u)`. A handle takes the share on the tangent line of that curve at its
/// anchor, so the two handles of a smooth anchor stay in line with it. That share is below 0 or
/// above 1 for a handle long enough beside the span. A closed subpath with no anchor that stays
/// moves whole when it has a focal point.
fn stretch_weights(anchors: &[Anchor], closed: bool, roles: &[Stretch]) -> Vec<[f64; 3]> {
    let n = anchors.len();
    let mut shares = vec![[0.0; 3]; n];
    let role = |i: usize| match roles.get(i) {
        Some(Stretch::Selected) if !closed && (i == 0 || i + 1 == n) => Stretch::Fixed,
        Some(&r) => r,
        None => Stretch::Fixed,
    };
    // The walk along the subpath starts at an anchor that stays, or else at a focal point, and on a
    // closed subpath it comes back round to the same anchor, so every selected anchor lies between
    // two anchors that are not selected ones.
    let start = if closed { (0..n).find(|&i| role(i) == Stretch::Fixed).or_else(|| (0..n).find(|&i| role(i) == Stretch::Focal)) } else { Some(0) };
    let Some(start) = start.filter(|_| n > 0) else { return shares };
    let len = if closed { n + 1 } else { n };
    let order: Vec<usize> = (0..len).map(|k| (start + k) % n).collect();
    // Where each anchor's in handle, point and out handle lie along the control polygon. The in
    // handle of the first anchor and the out handle of the last are not part of an open path.
    let mut pos: Vec<[f64; 3]> = Vec::with_capacity(len);
    let mut run = 0.0;
    let mut before: Option<&Anchor> = None;
    for (k, a) in order.iter().filter_map(|&i| anchors.get(i)).enumerate() {
        if let Some(b) = before {
            run += b.h_out.distance(a.h_in);
        }
        let c_in = run;
        if k > 0 {
            run += a.h_in.distance(a.p);
        }
        let c = run;
        if k + 1 < len {
            run += a.p.distance(a.h_out);
        }
        pos.push([c_in, c, run]);
        before = Some(a);
    }
    // The nearest anchor before and after each place in the walk that is not a selected one.
    let bound = |k: usize| role(order.get(k).copied().unwrap_or(start)) != Stretch::Selected;
    let mut previous = vec![0; len];
    let mut last = 0;
    for (k, slot) in previous.iter_mut().enumerate() {
        if bound(k) {
            last = k;
        }
        *slot = last;
    }
    let mut next = vec![len - 1; len];
    let mut first = len - 1;
    for (k, slot) in next.iter_mut().enumerate().rev() {
        if bound(k) {
            first = k;
        }
        *slot = first;
    }
    let at = |k: usize| pos.get(k).map_or(0.0, |c| c[1]);
    // The share of the anchor at `k` in the walk, between an anchor that stays at `end` and a
    // focal point at `focal` along the walk.
    let smooth = |k: usize, end: f64, focal: f64| {
        let span = (focal - end).abs();
        let Some(&[c_in, c, c_out]) = pos.get(k).filter(|_| span > 0.0 && span.is_finite()) else { return [0.0; 3] };
        let along = |c: f64| (c - end).abs() / span;
        let u = along(c);
        let w = u * u * (3.0 - 2.0 * u);
        let slope = 6.0 * u * (1.0 - u);
        [w + slope * (along(c_in) - u), w, w + slope * (along(c_out) - u)]
    };
    for (k, &i) in order.iter().enumerate().take(n) {
        let share = match role(i) {
            Stretch::Fixed => [0.0; 3],
            Stretch::Focal => [1.0; 3],
            Stretch::Selected => {
                let (kp, kn) = (previous.get(k).copied().unwrap_or(k), next.get(k).copied().unwrap_or(k));
                match (role(order.get(kp).copied().unwrap_or(i)), role(order.get(kn).copied().unwrap_or(i))) {
                    (Stretch::Focal, Stretch::Focal) => [1.0; 3],
                    (Stretch::Fixed, Stretch::Focal) => smooth(k, at(kp), at(kn)),
                    (Stretch::Focal, Stretch::Fixed) => smooth(k, at(kn), at(kp)),
                    _ => [0.0; 3],
                }
            }
        };
        // A coordinate that isn't finite moves nothing.
        if let Some(s) = shares.get_mut(i)
            && share.iter().all(|s| s.is_finite())
        {
            *s = share;
        }
    }
    shares
}

/// After an anchor is added at `(si, added)` of path `id`, number the direct-selected anchors and
/// selected segments after it on that subpath one higher, so the selection keeps the same points.
/// A selected segment that the new anchor splits stays selected as its two halves, and the new
/// anchor joins its ends in the anchors that move with it.
fn renumber_after_insert(sel: &mut Selection, id: NodeId, si: usize, added: usize) {
    if let Some(set) = sel.anchors.get_mut(&id) {
        // A selected anchor index can name no anchor (`select.anchors` takes any index), so the
        // renumbering saturates instead of overflowing.
        *set = set.iter().map(|&(s, a)| if s == si && a >= added { (s, a.saturating_add(1)) } else { (s, a) }).collect();
    }
    let split = added.checked_sub(1);
    if sel.segments.get(&id).is_some_and(|set| split.is_some_and(|g| set.contains(&(si, g)))) {
        sel.anchors.entry(id).or_default().insert((si, added));
    }
    if let Some(set) = sel.segments.get_mut(&id) {
        *set = set
            .iter()
            .flat_map(|&(s, g)| match (s == si, Some(g) == split) {
                (true, true) => vec![(s, g), (s, g.saturating_add(1))],
                (true, false) if g >= added => vec![(s, g.saturating_add(1))],
                _ => vec![(s, g)],
            })
            .collect();
    }
}

fn reshape(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.reshape")?.ok_or_else(|| bad("path.reshape", "missing id"))?;
    let at = Point::new(f64_req(p, "x", "path.reshape")?, f64_req(p, "y", "path.reshape")?);
    check_numbers(p, &["dx", "dy", "tol", "radius"], "path.reshape")?;
    let delta = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    let tol = f64_or(p, "tol", 3.0).max(0.0);
    let radius = p.get("radius").and_then(Value::as_f64);
    if radius.is_some_and(|r| r < 0.0) {
        return Err(bad("path.reshape", "`radius` must be 0 or more"));
    }
    // The anchor within `tol` of the grab point, if there is one.
    let near = |sp: &SubPath| match sp.anchors.iter().enumerate().map(|(i, a)| (i, a.p.distance(at))).min_by(|a, b| a.1.total_cmp(&b.1)) {
        Some((i, dist)) if dist <= tol => Some(i),
        _ => None,
    };
    // The other focal points the Reshape tool highlighted on this path.
    let mut focal: BTreeSet<(usize, usize)> = match p.get("focal") {
        None | Some(Value::Null) => BTreeSet::new(),
        Some(v) if v.is_array() => super::select::checked_refs(Some(v), "path.reshape")?.into_iter().collect(),
        Some(_) => return Err(bad("path.reshape", "`focal` must be an array of [subpath, anchor] pairs")),
    };
    // An empty `focal` chooses the focal-point rule too.
    if radius.is_some() && !matches!(p.get("focal"), None | Some(Value::Null)) {
        return Err(bad("path.reshape", "`focal` and `radius` cannot be used together"));
    }
    // A step that names neither `focal` nor `radius` (as the Reshape tool recorded one before it
    // had focal points) keeps that earlier rule: the selection does not matter, an open path
    // stretches from the grabbed point to its two ends, and a closed one follows with a falloff
    // over the default radius.
    let earlier = radius.is_none() && matches!(p.get("focal"), None | Some(Value::Null));
    {
        let st = s.doc()?;
        let node = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
        let path = node.path_data().ok_or_else(|| EngineError::Other(format!("object {id} is not a path")))?;
        if let Some((si, ai)) = focal.iter().copied().find(|&(si, ai)| path.subpaths.get(si).is_none_or(|sp| ai >= sp.anchors.len())) {
            return Err(bad("path.reshape", format!("`focal` names [{si}, {ai}], which is not an anchor of the path")));
        }
        // A drag of no length on an anchor moves nothing, so it is not an edit.
        if delta == Vec2::ZERO {
            let (si, ..) = path.nearest(at).ok_or_else(|| EngineError::Other("empty path".into()))?;
            if path.subpaths.get(si).is_some_and(|sp| near(sp).is_some()) {
                return ok();
            }
        }
    }
    s.edit("Reshape", |d, sel| {
        let path = path_mut(d, id)?;
        let (si, seg, t, _, _) = path.nearest(at).ok_or_else(|| EngineError::Other("empty path".into()))?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("empty path".into()))?;
        let grabbed = match near(sp) {
            Some(i) => i,
            None => {
                let added = sp.insert_anchor(seg, t);
                focal = focal.iter().map(|&(s, a)| if s == si && a >= added { (s, a.saturating_add(1)) } else { (s, a) }).collect();
                renumber_after_insert(sel, id, si, added);
                // On a partly selected path the new anchor is selected too.
                if let Some(set) = sel.anchors.get_mut(&id) {
                    set.insert((si, added));
                }
                added
            }
        };
        let earlier_closed = earlier && sp.closed;
        if radius.is_none() && !earlier_closed {
            focal.insert((si, grabbed));
            // Anchors that are not direct-selected stay; a path selected whole, or not selected,
            // has every anchor selected, and so does every path under the earlier rule.
            let selected = sel.anchors.get(&id).filter(|_| !earlier);
            for (k, sp) in path.subpaths.iter_mut().enumerate() {
                if focal.range((k, 0)..=(k, usize::MAX)).next().is_none() {
                    continue;
                }
                let roles: Vec<Stretch> = (0..sp.anchors.len())
                    .map(|i| match (focal.contains(&(k, i)), selected.is_none_or(|set| set.contains(&(k, i)))) {
                        (true, _) => Stretch::Focal,
                        (false, true) => Stretch::Selected,
                        (false, false) => Stretch::Fixed,
                    })
                    .collect();
                let weights = stretch_weights(&sp.anchors, sp.closed, &roles);
                for (a, [w_in, w, w_out]) in sp.anchors.iter_mut().zip(weights) {
                    a.p += delta * w;
                    a.h_in += delta * w_in;
                    a.h_out += delta * w_out;
                }
            }
            return Ok(());
        }
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("empty path".into()))?;
        let closed = sp.closed;
        // Each anchor's distance along the path from the grabbed one, the shorter way round a
        // closed path: the lengths are measured once, as a path can have many anchors.
        let pos = sp.anchor_arc_positions();
        let total = pos.last().copied().unwrap_or(0.0);
        // The default reach of the earlier rule: half the diagonal of the subpath's bounding box.
        let r = radius.unwrap_or_else(|| {
            let mut bp = vectorcraft_geom::BezPath::new();
            sp.to_bezpath_into(&mut bp);
            let b = vectorcraft_geom::Shape::bounding_box(&bp);
            (b.width().hypot(b.height()) / 2.0).max(1.0)
        });
        let from = pos.get(grabbed).copied().unwrap_or(0.0);
        for (i, a) in sp.anchors.iter_mut().enumerate() {
            let w = if i == grabbed {
                1.0
            } else {
                let d = (pos.get(i).copied().unwrap_or(f64::INFINITY) - from).abs();
                let d = if closed { d.min(total - d) } else { d };
                // A distance or radius that isn't finite gives NaN here, which `min` reads as 1: no move.
                let q = (d / r).min(1.0);
                (1.0 - q * q).powi(2)
            };
            let v = delta * w;
            a.p += v;
            a.h_in += v;
            a.h_out += v;
        }
        Ok(())
    })?;
    ok()
}

fn insert_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "path.insertAnchor")?.ok_or_else(|| bad("path.insertAnchor", "missing id"))?;
    let si = usize_opt(p, "subpath", "path.insertAnchor")?.unwrap_or(0);
    let seg = p.get("segment").and_then(Value::as_u64).map(index).ok_or_else(|| bad("path.insertAnchor", "missing segment"))?;
    let t = number_or(p, "t", 0.5, "path.insertAnchor")?.clamp(0.0, 1.0);
    let idx = s.edit("Add Anchor Point", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if seg >= sp.segment_count() {
            return Err(EngineError::Other("no such segment".into()));
        }
        let idx = sp.insert_anchor(seg, t);
        renumber_after_insert(sel, id, si, idx);
        // A path that wasn't selected becomes the selection, its new anchor direct-selected, so
        // the point added shows and can be edited at once (#4237).
        if !sel.contains(id) {
            sel.clear();
            sel.add(id);
            sel.anchors.entry(id).or_default().insert((si, idx));
        }
        Ok(idx)
    })?;
    Ok(json!({ "anchor": idx }))
}

fn cut_at_anchors(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "path.cutAtAnchors";
    let st = s.doc()?;
    // Each selected path's subpaths cut at its selected anchors, those that cut anything.
    let mut cuts: Vec<(NodeId, Vec<SubPath>, Vec<bool>)> = vec![];
    for (id, set) in &st.selection.anchors {
        let Some(pd) = st.doc.node(*id).and_then(|n| n.path_data()) else { continue };
        let (mut subs, mut starts) = (vec![], vec![]);
        for (si, sp) in pd.subpaths.iter().enumerate() {
            let at: BTreeSet<usize> = subpath_refs(set, si).collect();
            let pieces = sp.cut_at(&at);
            // The pieces that start at a cut: every piece of an opened closed subpath, all but the
            // first of an open one.
            starts.extend(pieces.iter().enumerate().map(|(k, piece)| if sp.closed { !piece.closed } else { k > 0 }));
            subs.extend(pieces);
        }
        if starts.contains(&true) {
            cuts.push((*id, subs, starts));
        }
    }
    if cuts.is_empty() {
        return Err(bad(C, "direct-select anchor points to cut at (not the end points of open paths)"));
    }
    let ids = s.edit("Cut Path", |d, sel| {
        let (mut all, mut picked) = (vec![], vec![]);
        let mut cut = Beside::replacing(d, &cuts.iter().map(|(id, ..)| *id).collect::<Vec<_>>());
        for (id, subs, starts) in cuts {
            // A path of one subpath becomes one path per piece (the first keeps the id); the
            // pieces of a compound shape stay subpaths of it.
            if d.node(id).and_then(|n| n.path_data()).is_some_and(|pd| pd.subpaths.len() == 1) {
                let ids = super::draw2::replace_with_pieces(d, &mut cut, id, subs.into_iter().map(PathData::single).collect())?;
                picked.extend(ids.iter().zip(&starts).filter(|(_, s)| **s).map(|(id, _)| (*id, (0, 0))));
                all.extend(ids);
            } else {
                path_mut(d, id)?.subpaths = subs;
                picked.extend(starts.iter().enumerate().filter(|(_, s)| **s).map(|(si, _)| (id, (si, 0))));
                all.push(id);
            }
        }
        cut.finish(d)?;
        // Each cut leaves one of its two coincident anchors selected, so a drag pulls the ends apart.
        sel.clear();
        for (id, a) in picked {
            sel.add(id);
            sel.anchors.entry(id).or_default().insert(a);
        }
        Ok(all)
    })?;
    Ok(super::draw2::ids_json(&ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `merge_coincident` removing each merged anchor from the vector straight away, kept as the
    /// reference the one-pass version must match.
    fn merge_coincident_by_removal(sp: &mut SubPath, picked: impl Fn(usize) -> bool, join_kind: Option<bool>) -> usize {
        let same = |a: &Anchor, b: &Anchor| a.p.distance(b.p) < 1e-6;
        let before = sp.anchors.len();
        if sp.closed
            && before > 2
            && picked(0)
            && picked(before - 1)
            && let [first, .., last] = sp.anchors.as_mut_slice()
            && same(first, last)
        {
            first.h_in = last.h_in;
            sp.anchors.pop();
            if let Some(smooth) = join_kind {
                join_point(sp, 0, smooth);
            }
        }
        for i in (1..sp.anchors.len()).rev() {
            if picked(i - 1)
                && picked(i)
                && let Some([a, b]) = sp.anchors.get_mut(i - 1..=i)
                && same(a, b)
            {
                a.h_out = b.h_out;
                sp.anchors.remove(i);
                if let Some(smooth) = join_kind {
                    join_point(sp, i - 1, smooth);
                }
            }
        }
        before - sp.anchors.len()
    }

    /// Every field of every anchor, bit for bit.
    fn bits(sp: &SubPath) -> (Vec<[u64; 6]>, Vec<AnchorKind>, bool) {
        let b = |p: Point| [p.x.to_bits(), p.y.to_bits()];
        let fields = sp.anchors.iter().map(|a| [b(a.p), b(a.h_in), b(a.h_out)].concat().try_into().unwrap()).collect();
        (fields, sp.anchors.iter().map(|a| a.kind).collect(), sp.closed)
    }

    /// A small linear congruential generator, so the cases are the same on every run.
    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn same_as_removal(sp: &SubPath, picked: &dyn Fn(usize) -> bool, join_kind: Option<bool>) {
        let (mut ours, mut theirs) = (sp.clone(), sp.clone());
        let n = merge_coincident(&mut ours, picked, join_kind);
        let m = merge_coincident_by_removal(&mut theirs, picked, join_kind);
        assert_eq!(n, m, "{sp:?} {join_kind:?}");
        assert_eq!(bits(&ours), bits(&theirs), "{sp:?} {join_kind:?}");
    }

    /// Short paths with points on, near and just past the merge distance, selections with stale
    /// indices, every anchor kind and every join kind give the same anchors, bit for bit, as
    /// removing each merged anchor straight away.
    #[test]
    fn merging_coincident_anchors_matches_removing_them_one_by_one() {
        let spots = [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (5e-7, 0.0), (0.0, 1.5e-6), (10.0, 0.0), (5.0, 5.0), (10.0, 0.0)];
        let mut rng = Lcg(7);
        for _ in 0..30_000 {
            let n = rng.below(9) as usize;
            let anchors = (0..n)
                .map(|_| {
                    let (x, y) = spots[rng.below(spots.len() as u64) as usize];
                    let p = Point::new(x, y);
                    let handle = |rng: &mut Lcg| match rng.below(3) {
                        0 => p,
                        _ => p + Vec2::new(rng.below(7) as f64 - 3.0, rng.below(7) as f64 - 3.0),
                    };
                    let (h_in, h_out) = (handle(&mut rng), handle(&mut rng));
                    let kind = if rng.below(2) == 0 { AnchorKind::Corner } else { AnchorKind::Smooth };
                    Anchor { p, h_in, h_out, kind }
                })
                .collect();
            let sp = SubPath::new(anchors, rng.below(2) == 0);
            let picks: Vec<bool> = (0..n + 2).map(|_| rng.below(4) != 0).collect();
            let join_kind = [None, Some(false), Some(true)][rng.below(3) as usize];
            same_as_removal(&sp, &|i| picks.get(i).copied().unwrap_or(false), join_kind);
        }
    }

    /// A smooth join of three anchors on one another takes the outgoing handle the middle anchor
    /// was given when it joined the right one: an already smooth left anchor keeps that handle.
    #[test]
    fn a_smooth_join_of_three_anchors_keeps_the_handle_the_middle_one_got() {
        let o = Point::ZERO;
        let left = Anchor { p: o, h_in: Point::new(-2.0, 0.0), h_out: o, kind: AnchorKind::Smooth };
        let right = Anchor { p: o, h_in: o, h_out: Point::new(0.0, 5.0), kind: AnchorKind::Corner };
        let mut sp = SubPath::new(vec![left, Anchor::corner(o), right, Anchor::corner(Point::new(10.0, 0.0))], false);
        assert_eq!(merge_coincident(&mut sp, |i| i < 3, Some(true)), 2);
        let a = sp.anchors[0];
        assert_eq!((a.h_in, a.h_out, a.kind), (Point::new(-2.0, 0.0), Point::new(10.0 / 3.0, 0.0), AnchorKind::Smooth));
        assert_eq!(sp.anchors.len(), 2);
    }

    /// One long run of selected anchors on one another, open or closed, becomes one anchor.
    #[test]
    fn a_long_run_of_anchors_on_one_another_becomes_one() {
        for closed in [false, true] {
            for join_kind in [None, Some(false), Some(true)] {
                let anchors = (0..50_000).map(|i| Anchor::smooth(Point::ZERO, Point::new(1.0, i as f64))).collect();
                let sp = SubPath::new(anchors, closed);
                same_as_removal(&sp, &|_| true, join_kind);
                let mut ours = sp.clone();
                assert_eq!(merge_coincident(&mut ours, |_| true, join_kind), 49_999);
                assert_eq!((ours.anchors.len(), ours.closed), (1, closed));
            }
        }
    }

    /// A long selected run at the start of a path followed by as many anchors that stay: each
    /// anchor after the run moves once, not once per merged anchor.
    #[test]
    fn a_long_run_before_many_other_anchors_merges_in_one_pass() {
        let run = 300_000;
        let anchors = (0..run).map(|_| Anchor::corner(Point::ZERO)).chain((1..=run).map(|i| Anchor::corner(Point::new(i as f64, 1.0)))).collect();
        let mut sp = SubPath::new(anchors, false);
        ANCHOR_MOVES.set(0);
        assert_eq!(merge_coincident(&mut sp, |i| i < run, Some(false)), run - 1);
        assert_eq!(ANCHOR_MOVES.get(), run);
        assert_eq!(sp.anchors.len(), run + 1);
        assert_eq!(sp.anchors[0].p, Point::ZERO);
        assert_eq!(sp.anchors[1].p, Point::new(1.0, 1.0));
        assert_eq!(sp.anchors[run].p, Point::new(run as f64, 1.0));
    }
}
