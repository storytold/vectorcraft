//! Path editing commands (used by the Pen and Direct Selection tools and Object → Path).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, SubPath, Vec2};

use super::create::anchor_from_json;
use super::draw2::usize_opt;
use super::pathops::Beside;
use super::{
    CommandSpec, Result, Session, bad, check_numbers, check_switches, cmd, f64_or, f64_req, finite_numbers, has_anchors, has_doc, has_selection,
    id_param, ids_param, index, number_or, ok, str_param, switch_or, targets,
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
            "{id, subpath?: usize (the open subpath to close, 0 included; default: the last open one), in?: [x,y] (handle into the first anchor), independent?: bool}",
            has_doc,
            close
        ),
        cmd!(
            "path.moveAnchors",
            "Move Anchors",
            [],
            None,
            "{dx, dy, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the objects that move whole; default: prefs transformPatternTiles)} move direct-selected anchors (whole paths if fully selected)",
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
            "{ids?|id?, subpath?: usize (reverse only this subpath of each path, 0 included; default: every subpath), reversed?: bool (true: every subpath runs counter-clockwise on screen, false: clockwise, the Attributes panel's Reverse Path Direction On / Off; omitted: flip each path; with subpath, the direction is not read: that subpath is flipped)} the paths in ids or the selection (groups and compound paths: the paths inside), as one undo step → {changed: paths}",
            has_doc,
            reverse
        ),
        cmd!(
            "path.join",
            "Join",
            ["Object", "Path"],
            Some("Cmd+J"),
            "{ids?: [path, ...] (default: the selected paths; a compound path ID expands to its open member paths), subpaths?: [usize, usize] (which subpaths to join), ends?: [\"first\"|\"last\", \"first\"|\"last\"] (which end of each path to join, for two paths/subpaths; default without ids: direct-selected ends if any, else the nearest pair; default with ids: the nearest pair), to?: \"corner\"|\"smooth\" (the joined anchor: corner retracts both handles, smooth pulls handles in line with the neighbours)} join two or more open paths/subpaths into one (the first keeps its id, selected), or close a single open path; without ids, direct-selected anchors lying on one another along a path merge into one anchor instead",
            has_selection,
            join
        ),
        cmd!(
            "path.average",
            "Average…",
            ["Object", "Path"],
            Some("Cmd+Alt+J"),
            "{axis?: \"horizontal\"|\"vertical\"|\"both\"}",
            has_selection,
            average
        ),
        cmd!(
            "path.convertAnchors",
            "Convert Anchor Points",
            [],
            None,
            "{to: \"corner\"|\"smooth\"} convert the direct-selected anchors (all anchors of fully selected paths) as one undo step: corner retracts both handles, smooth pulls handles in line with the neighbours (smooth anchors keep theirs)",
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
            "{id, x, y, dx, dy, tol=3, radius?} Reshape tool: grab the path at (x, y) (adding an anchor there unless one is within tol) and drag it by (dx, dy); nearby anchors follow with a smooth falloff over radius, measured along the path (default: half the subpath's size)",
            has_doc,
            reshape
        ),
        cmd!("path.insertAnchor", "Add Anchor Point", [], None, "{id, subpath?, segment, t: 0..1}", has_doc, insert_anchor),
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
    let id = id_param(p, "id").ok_or_else(|| bad("path.appendAnchor", "missing id"))?;
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

/// Close the active open subpath of `id`, optionally setting the incoming handle of the first anchor.
fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.close", "missing id"))?;
    let subpath = p.get("subpath").and_then(Value::as_u64).map(|u| u as usize);
    let h_in = match p.get("in").filter(|h| !h.is_null()) {
        None => None,
        Some(h) => Some(finite_numbers(h).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad("path.close", "`in` must be [x, y]"))?),
    };
    let independent = switch_or(p, "independent", false, "path.close")?;
    s.edit("Close Path", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = match subpath {
            Some(si) => path.subpaths.get_mut(si).filter(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?,
            None => path.subpaths.iter_mut().rev().find(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?,
        };
        sp.closed = true;
        if let (Some(h), Some(first)) = (h_in, sp.anchors.first_mut()) {
            first.h_in = h;
            if !independent {
                first.h_out = first.p - (h - first.p);
                first.kind = AnchorKind::Smooth;
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
                }
                None => {
                    if let Some(n) = doc.node_mut(*id) {
                        n.transform(xf, sc);
                    }
                }
            }
        }
        if is_translation && (delta.x != 0.0 || delta.y != 0.0) {
            for &i in &sel.guides {
                if let Some(g) = doc.guides.get_mut(i) {
                    *g = g.moved(delta);
                }
            }
        }
        Ok(())
    })
}

fn set_handle(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.setHandle", "missing id"))?;
    let si = usize_opt(p, "subpath", "path.setHandle")?.unwrap_or(0);
    let ai = p.get("anchor").and_then(Value::as_u64).map(index).ok_or_else(|| bad("path.setHandle", "missing anchor"))?;
    let out = str_param(p, "which") != Some("in");
    let pos = Point::new(f64_req(p, "x", "path.setHandle")?, f64_req(p, "y", "path.setHandle")?);
    let independent = switch_or(p, "independent", false, "path.setHandle")?;
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
    let id = id_param(p, "id").ok_or_else(|| bad("path.moveAnchor", "missing id"))?;
    let si = usize_opt(p, "subpath", "path.moveAnchor")?.unwrap_or(0);
    let ai = p.get("anchor").and_then(Value::as_u64).map(index).ok_or_else(|| bad("path.moveAnchor", "missing anchor"))?;
    let pos = Point::new(f64_req(p, "x", "path.moveAnchor")?, f64_req(p, "y", "path.moveAnchor")?);
    s.edit("Move", |d, _| {
        let path = path_mut(d, id)?;
        let a = path.anchor_mut(si, ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?;
        *a = a.transform(vectorcraft_geom::Affine::translate(pos - a.p));
        Ok(())
    })?;
    ok()
}

fn set_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.setAnchors", "missing id"))?;
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
        Ok(())
    })?;
    ok()
}

/// The compound path enclosing `id`, or `id` itself if it is not inside a compound path.
fn compound_or_self(d: &Document, id: NodeId) -> NodeId {
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
            if let Some(si) = subpath {
                if let Some(sp) = path.subpaths.get_mut(si) {
                    sp.reverse();
                    changed += 1;
                }
            } else {
                let flip: Vec<bool> = path.subpaths.iter().map(|sp| want.is_none_or(|ccw| (sp.signed_area() < 0.0) != ccw)).collect();
                if flip.contains(&true) {
                    for (sp, f) in path.subpaths.iter_mut().zip(flip) {
                        if f {
                            sp.reverse();
                        }
                    }
                    changed += 1;
                }
            }
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
    // Back to front, so the indices `picked` knows stay true for the anchors still to visit.
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
fn join(s: &mut Session, p: &Value) -> Result<Value> {
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

    if let Some(ids) = ids_param(p, "ids") {
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
            let n: usize =
                path.subpaths.iter_mut().enumerate().map(|(si, sp)| merge_coincident(sp, |ai| picked.contains(&(si, ai)), join_kind)).sum();
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
        if ida == idb {
            if sa == sb {
                s.edit("Join", |d, sel| {
                    close_subpath_in_node(d, ida, sa, join_kind)?;
                    sel.set([compound_or_self(d, ida)]);
                    Ok(())
                })?;
            } else {
                s.edit("Join", |d, sel| {
                    join_subpaths_in_node(d, ida, (sa, sb), (rx, ry), join_kind)?;
                    sel.set([compound_or_self(d, ida)]);
                    Ok(())
                })?;
            }
            return ok();
        } else {
            s.edit("Join", |d, sel| {
                join_two(d, ida, idb, (sa, sb), (rx, ry), join_kind)?;
                sel.set([compound_or_self(d, ida)]);
                Ok(())
            })?;
            return ok();
        }
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
        let (a, sa) = open_subs[0];
        let (b, sb) = open_subs[1];
        if a == b {
            s.edit("Join", |d, sel| {
                join_subpaths_in_node(d, a, (sa, sb), (rx, ry), join_kind)?;
                sel.set([compound_or_self(d, a)]);
                Ok(())
            })?;
        } else {
            s.edit("Join", |d, sel| {
                join_two(d, a, b, (sa, sb), (rx, ry), join_kind)?;
                sel.set([compound_or_self(d, a)]);
                Ok(())
            })?;
        }
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
    let t = anchor_targets(s)?;
    let d0 = &s.doc()?.doc;
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

fn convert_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.convertAnchors";
    let smooth = match str_param(p, "to") {
        Some("smooth") => true,
        Some("corner") => false,
        _ => return Err(bad(C, "`to` must be \"corner\" or \"smooth\"")),
    };
    let t = anchor_targets(s)?;
    if t.is_empty() {
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
        Ok(())
    })?;
    ok()
}

pub(crate) fn delete_anchors(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let sel = st.selection.anchors.clone();
    let segments = st.selection.segments.clone();
    // Every anchor selected stores the path as an object, with no `anchors` entry. A marquee that
    // also catches part of another path must still delete those whole objects.
    let whole = st.selection.whole();
    s.edit("Clear", |d, selection| {
        let targets: BTreeSet<NodeId> = whole.into_iter().filter(|id| d.node(*id).is_some_and(|n| !n.is_layer())).collect();
        let whole_ids: Vec<NodeId> = targets
            .iter()
            .copied()
            .filter(|id| !d.ancestry(*id).unwrap_or_default().iter().rev().skip(1).any(|ancestor| targets.contains(ancestor)))
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
            let remove_node = {
                let path = path_mut(d, *id)?;
                if let Some(refs) = segments.get(id) {
                    // A segment pick cuts its connection, keeping every neighboring cubic.
                    path.subpaths = std::mem::take(&mut path.subpaths)
                        .into_iter()
                        .enumerate()
                        .flat_map(|(si, sp)| {
                            let cuts = refs.iter().filter_map(|&(s, seg)| (s == si).then_some(seg)).collect();
                            without_segments(sp, &cuts)
                        })
                        .filter(|s| s.anchors.len() >= 2)
                        .collect();
                } else {
                    // Remove every selected anchor against the original indexes, keeping surviving
                    // stretches separate rather than joining across deleted anchors.
                    path.subpaths = std::mem::take(&mut path.subpaths)
                        .into_iter()
                        .enumerate()
                        .flat_map(|(si, sp)| {
                            let gone = set.iter().filter_map(|&(s, ai)| (s == si).then_some(ai)).collect();
                            without_anchors(sp, &gone)
                        })
                        .filter(|s| s.anchors.len() >= 2)
                        .collect();
                }
                path.subpaths.retain(|s| s.anchors.len() >= 2);
                path.subpaths.is_empty()
            };
            if remove_node {
                emptied.put(d, *id, vec![])?;
            } else {
                // The remainder is selected whole, so the next Delete removes it.
                survivors.push(*id);
            }
        }
        emptied.finish(d)?;
        selection.set(survivors);
        Ok(())
    })?;
    ok()
}

/// Remove anchors and their incident segments, keeping each surviving stretch as an open path.
/// Only a closed path can have a surviving stretch that wraps past the last anchor.
fn without_anchors(sp: SubPath, removed: &BTreeSet<usize>) -> Vec<SubPath> {
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

fn reshape(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.reshape", "missing id"))?;
    let at = Point::new(f64_req(p, "x", "path.reshape")?, f64_req(p, "y", "path.reshape")?);
    check_numbers(p, &["dx", "dy", "tol", "radius"], "path.reshape")?;
    let delta = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    let tol = f64_or(p, "tol", 3.0).max(0.0);
    let radius = p.get("radius").and_then(Value::as_f64);
    s.edit("Reshape", |d, _| {
        let path = path_mut(d, id)?;
        let (si, seg, t, _, _) = path.nearest(at).ok_or_else(|| EngineError::Other("empty path".into()))?;
        let sp = &mut path.subpaths[si];
        let grabbed = match sp.anchors.iter().enumerate().map(|(i, a)| (i, a.p.distance(at))).min_by(|a, b| a.1.total_cmp(&b.1)) {
            Some((i, dist)) if dist <= tol => i,
            _ => sp.insert_anchor(seg, t),
        };
        // Each anchor's distance along the path from the grabbed one, the shorter way round a
        // closed path: the lengths are measured once, as a path can have many anchors.
        let pos = sp.anchor_arc_positions();
        let total = pos.last().copied().unwrap_or(0.0);
        // The default reach is the subpath's size, as before, now measured along it.
        let r = radius.unwrap_or_else(|| {
            let mut bp = vectorcraft_geom::BezPath::new();
            sp.to_bezpath_into(&mut bp);
            let b = vectorcraft_geom::Shape::bounding_box(&bp);
            (b.width().hypot(b.height()) / 2.0).max(1.0)
        });
        let from = pos.get(grabbed).copied().unwrap_or(0.0);
        let closed = sp.closed;
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
    let id = id_param(p, "id").ok_or_else(|| bad("path.insertAnchor", "missing id"))?;
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
            let at: BTreeSet<usize> = set.iter().filter(|(s, _)| *s == si).map(|(_, a)| *a).collect();
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
