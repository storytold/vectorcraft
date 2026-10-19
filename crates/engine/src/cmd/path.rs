//! Path editing commands (used by the Pen and Direct Selection tools and Object → Path).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, SubPath, Vec2};

use super::create::anchor_from_json;
use super::draw2::usize_opt;
use super::pathops::Beside;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.appendAnchor",
            "Add Anchor (Pen)",
            [],
            None,
            "{id, x, y, in?: [x,y], out?: [x,y]} append to the end of the last open subpath",
            has_doc,
            append_anchor
        ),
        cmd!("path.close", "Close Path", [], None, "{id, in?: [x,y] (handle into the first anchor), independent?: bool}", has_doc, close),
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
            "{ids?|id?, reversed?: bool (true: every subpath runs counter-clockwise on screen, false: clockwise, the Attributes panel's Reverse Path Direction On / Off; omitted: flip each path)} the paths in ids or the selection (groups and compound paths: the paths inside), as one undo step → {changed: paths}",
            has_doc,
            reverse
        ),
        cmd!(
            "path.join",
            "Join",
            ["Object", "Path"],
            Some("Cmd+J"),
            "{ids?: [path, path] (default: the selected paths), ends?: [\"first\"|\"last\", \"first\"|\"last\"] (which end of each path to join, as the Pen joins them; default: the nearest pair), to?: \"corner\"|\"smooth\" (the joined anchor: corner retracts both handles, smooth pulls handles in line with the neighbours)} join two open paths into one (the first keeps its id, selected), or close a single open path; without ids, direct-selected anchors lying on one another along a path merge into one anchor instead",
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
    s.edit("Pen", |d, sel| {
        let path = path_mut(d, id)?;
        match path.subpaths.last_mut() {
            Some(sp) if !sp.closed => sp.anchors.push(a),
            _ => path.subpaths.push(SubPath::new(vec![a], false)),
        }
        sel.set([id]);
        let last = path.subpaths.len() - 1;
        let ai = path.subpaths[last].anchors.len() - 1;
        sel.anchors.insert(id, BTreeSet::from([(last, ai)]));
        Ok(())
    })?;
    ok()
}

fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.close", "missing id"))?;
    let h_in = point_param(p, "in");
    let independent = bool_or(p, "independent", false);
    s.edit("Close Path", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.iter_mut().rev().find(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?;
        sp.closed = true;
        if let (Some(h), Some(first)) = (h_in, sp.anchors.first_mut()) {
            first.h_in = h;
            if !independent {
                first.h_out = first.p - (h - first.p);
                first.kind = AnchorKind::Smooth;
            }
        }
        sel.set([id]);
        Ok(())
    })?;
    ok()
}

pub(crate) fn move_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    // Objects selected whole move as objects, and their pattern tiles move with them when
    // `patterns` (default: General › Transform Pattern Tiles) is on.
    let whole = s.doc()?.selection.whole();
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &whole)?, ..Scaling::default() };
    transform_anchors(s, "Move", vectorcraft_geom::Affine::translate(d), sc)?;
    ok()
}

/// Apply `xf` to the direct-selected anchors, their handles with them, and (with `sc`) to the
/// objects selected whole (those with no entry in the selection's `anchors`).
pub(crate) fn transform_anchors(s: &mut Session, label: &str, xf: vectorcraft_geom::Affine, sc: Scaling) -> Result<()> {
    let sel = s.doc()?.selection.clone();
    let c = xf.as_coeffs();
    let is_translation = (c[0] - 1.0).abs() < 1e-9 && c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && (c[3] - 1.0).abs() < 1e-9;
    let delta = vectorcraft_geom::Vec2::new(c[4], c[5]);
    s.edit(label, |doc, _| {
        for id in &sel.objects {
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
    let ai = p.get("anchor").and_then(Value::as_u64).ok_or_else(|| bad("path.setHandle", "missing anchor"))? as usize;
    let out = str_param(p, "which") != Some("in");
    let pos = Point::new(f64_req(p, "x", "path.setHandle")?, f64_req(p, "y", "path.setHandle")?);
    let independent = bool_or(p, "independent", false);
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
    let ai = p.get("anchor").and_then(Value::as_u64).ok_or_else(|| bad("path.moveAnchor", "missing anchor"))? as usize;
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

fn selected_paths(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(st.selection.objects.iter().copied().filter(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Path { .. }))).collect())
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
    let ids = paths_in(&s.doc()?.doc, &targets(s, p)?);
    if ids.is_empty() {
        return Err(bad(C, "select paths"));
    }
    let changed = super::overprint::edit_counted(s, "Reverse Path Direction", |d| {
        let mut changed = 0;
        for id in &ids {
            // The subpaths to flip: all, or those not yet running the wanted way.
            let flip: Vec<bool> = d
                .node(*id)
                .and_then(|n| n.path_data())
                .map_or(vec![], |pd| pd.subpaths.iter().map(|sp| want.is_none_or(|ccw| (sp.signed_area() < 0.0) != ccw)).collect());
            if flip.contains(&true) {
                for (sp, f) in path_mut(d, *id)?.subpaths.iter_mut().zip(flip) {
                    if f {
                        sp.reverse();
                    }
                }
                changed += 1;
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

fn join(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.join";
    let join_kind = match p.get("to").and_then(Value::as_str) {
        None => None,
        Some("smooth") => Some(true),
        Some("corner") => Some(false),
        Some(_) => return Err(bad(C, "`to` must be \"corner\" or \"smooth\"")),
    };
    let ids = match ids_param(p, "ids") {
        Some(ids) => {
            let st = s.doc()?;
            if let Some(id) = ids.iter().find(|id| !matches!(st.doc.node(**id).map(|n| &n.kind), Some(NodeKind::Path { .. }))) {
                return Err(bad(C, format!("{} isn't a path", id.0)));
            }
            ids
        }
        None => selected_paths(s)?,
    };
    // Direct-selected anchors lying on one another along a path (two corners brought together by
    // Average) become one anchor; the paths themselves are joined only when there are none.
    if p.get("ids").is_none() {
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
                // The anchor indices changed: the paths stay selected, whole.
                sel.set(objects);
                Ok(())
            })?;
            return ok();
        }
    }
    // The ends to join, as (reverse the first path, reverse the second): the first path's end
    // must come last, the second's first.
    let ends = match p.get("ends") {
        None => None,
        Some(e) => match e.as_array().map(|a| a.iter().map(Value::as_str).collect::<Vec<_>>()).as_deref() {
            Some([Some(a @ ("first" | "last")), Some(b @ ("first" | "last"))]) => Some((*a == "first", *b == "last")),
            _ => return Err(bad(C, "ends must be two of \"first\" and \"last\"")),
        },
    };
    match ids.as_slice() {
        [one] => {
            let id = *one;
            s.edit("Join", |d, _| {
                let path = path_mut(d, id)?;
                let sp = path.subpaths.iter_mut().find(|s| !s.closed).ok_or_else(|| EngineError::Other("path is already closed".into()))?;
                // Merge coincident end points.
                if sp.anchors.len() > 2
                    && end_points(sp).is_some_and(|(f, l)| f.distance(l) < 1e-6)
                    && let Some(last) = sp.anchors.pop()
                    && let Some(first) = sp.anchors.first_mut()
                {
                    first.h_in = last.h_in;
                    // Closed first, so the merged anchor smooths between both its neighbours.
                    sp.closed = true;
                    if let Some(smooth) = join_kind {
                        join_point(sp, 0, smooth);
                    }
                }
                sp.closed = true;
                Ok(())
            })?;
        }
        [a, b, ..] => {
            let (a, b) = (*a, *b);
            if a == b {
                return Err(bad(C, "join two different paths"));
            }
            s.edit("Join", |d, sel| {
                let pb = d.node(b).and_then(|n| n.path_data()).cloned().ok_or(EngineError::NoNode(b))?;
                let pa = path_mut(d, a)?;
                let (Some(sa), Some(sb)) = (pa.subpaths.iter().position(|s| !s.closed), pb.subpaths.iter().position(|s| !s.closed)) else {
                    return Err(EngineError::Other("join needs open paths".into()));
                };
                let mut x = pa.subpaths[sa].clone();
                let mut y = pb.subpaths[sb].clone();
                let (Some((xf, xl)), Some((yf, yl))) = (end_points(&x), end_points(&y)) else {
                    return Err(EngineError::Other("join needs open paths".into()));
                };
                // The ends asked for, else the closest pair.
                let cands =
                    [(xl.distance(yf), false, false), (xl.distance(yl), false, true), (xf.distance(yf), true, false), (xf.distance(yl), true, true)];
                let (rx, ry) =
                    ends.unwrap_or_else(|| cands.into_iter().min_by(|p, q| p.0.total_cmp(&q.0)).map_or((false, false), |(_, rx, ry)| (rx, ry)));
                if rx {
                    x.reverse();
                }
                if ry {
                    y.reverse();
                }
                let junction = x.anchors.len() - 1;
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
                pa.subpaths[sa] = x;
                d.remove(b)?;
                sel.set([a]);
                Ok(())
            })?;
        }
        [] => return Err(EngineError::Other("select paths to join".into())),
    }
    ok()
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
fn without_segments(sp: SubPath, removed: &BTreeSet<usize>) -> Vec<SubPath> {
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
    let seg = p.get("segment").and_then(Value::as_u64).ok_or_else(|| bad("path.insertAnchor", "missing segment"))? as usize;
    let t = f64_or(p, "t", 0.5).clamp(0.0, 1.0);
    let idx = s.edit("Add Anchor Point", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if seg >= sp.segment_count() {
            return Err(EngineError::Other("no such segment".into()));
        }
        Ok(sp.insert_anchor(seg, t))
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
