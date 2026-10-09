//! Path editing commands (used by the Pen and Direct Selection tools and Object → Path).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, SubPath, Vec2};

use super::create::anchor_from_json;
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
            "{dx, dy, join?: bool (a single dragged open end that lands on another open end joins it: closes its path, or joins the two paths)} move direct-selected anchors (whole paths if fully selected)",
            has_selection,
            move_anchors
        ),
        cmd!(
            "path.setHandle",
            "Move Direction Handle",
            [],
            None,
            "{id, subpath, anchor, which: \"in\"|\"out\", x, y, independent?: bool}",
            has_doc,
            set_handle
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
            "{ids?, reversed?: bool (true: every subpath runs counter-clockwise on screen, false: clockwise, the Attributes panel's Reverse Path Direction On / Off; omitted: flip each path)} the paths in ids or the selection (groups and compound paths: the paths inside), as one undo step → {changed: paths}",
            has_doc,
            reverse
        ),
        cmd!(
            "path.join",
            "Join",
            ["Object", "Path"],
            Some("Cmd+J"),
            "{} join two open paths' nearest endpoints, or close a single open path",
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
            "{to: \"corner\"|\"smooth\"} (Control bar convert buttons)",
            has_selection,
            convert_anchors
        ),
        cmd!(
            "path.deleteAnchors",
            "Remove Anchor Points",
            ["Object", "Path"],
            None,
            "{} delete direct-selected anchors",
            has_selection,
            delete_anchors
        ),
        cmd!(
            "path.reshape",
            "Reshape",
            [],
            None,
            "{id, x, y, dx, dy, tol=3, radius?} Reshape tool: grab the path at (x, y) (adding an anchor there unless one is within tol) and drag it by (dx, dy); nearby anchors follow with a smooth falloff over radius (default: half the subpath's size)",
            has_doc,
            reshape
        ),
        cmd!("path.insertAnchor", "Add Anchor Point", [], None, "{id, subpath, segment, t: 0..1}", has_doc, insert_anchor),
        cmd!("path.cutAtAnchors", "Cut Path at Selected Anchor Points", [], None, "{}", has_selection, cut_at_anchors),
        cmd!(
            "path.unlockAnchors",
            "Unlock Anchor Points",
            ["Object", "Path"],
            None,
            "{} split the paths at the direct-selected anchors into separate open lines (unmakes a joined shape; the ends of an open path stay as they are); a filled shape that opens keeps its fill as a shape of its own below the lines, as one undo step → {ids}",
            has_unlockable,
            unlock_anchors
        ),
    ]
}

fn path_mut(d: &mut vectorcraft_doc::Document, id: NodeId) -> Result<&mut PathData> {
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
    let a = anchor_from_json(p).ok_or_else(|| bad("path.appendAnchor", "missing x/y"))?;
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
    let fill = s.paint.fill.clone();
    s.edit("Close Path", |d, sel| {
        // Closing makes it a shape: it takes the current fill if it has none.
        if let Some(n) = d.node_mut(id)
            && n.appearance.fill_paint().is_none()
            && !fill.is_none()
        {
            n.appearance.set_fill(fill.clone());
        }
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
    let st = s.doc()?;
    let sel = st.selection.clone();
    let scale_strokes = s.prefs.scale_strokes;
    let _ = scale_strokes;
    let join = bool_or(p, "join", false);
    s.edit("Move", |doc, selection| {
        for id in &sel.objects {
            match sel.anchors.get(id) {
                Some(set) => {
                    let path = path_mut(doc, *id)?;
                    for &(si, ai) in set {
                        if let Some(a) = path.anchor_mut(si, ai) {
                            a.translate(d);
                        }
                    }
                }
                None => {
                    if let Some(n) = doc.node_mut(*id) {
                        n.transform(vectorcraft_geom::Affine::translate(d), false);
                    }
                }
            }
        }
        if join
            && let [id] = sel.objects.as_slice()
            && let Some(set) = sel.anchors.get(id)
            && let [(si, ai)] = set.iter().copied().collect::<Vec<_>>().as_slice()
            && join_end(doc, *id, *si, *ai)?
        {
            selection.set([*id]);
        }
        Ok(())
    })?;
    ok()
}

/// Joins the open end (`si`, `ai`) of path `id` to an open end lying on it: the other end of
/// its own subpath closes it, another subpath or path is joined on. Returns whether it joined.
fn join_end(d: &mut vectorcraft_doc::Document, id: NodeId, si: usize, ai: usize) -> Result<bool> {
    const EPS: f64 = 1e-6;
    let Some(pd) = d.node(id).and_then(|n| n.path_data()) else { return Ok(false) };
    let Some(sp) = pd.subpaths.get(si).filter(|s| !s.closed && s.anchors.len() >= 2) else { return Ok(false) };
    let last = sp.anchors.len() - 1;
    if ai != 0 && ai != last {
        return Ok(false);
    }
    let Some(at) = sp.anchors.get(ai).map(|a| a.p) else { return Ok(false) };
    // Its own other end: close the subpath, merging the two coincident points.
    let other = if ai == 0 { last } else { 0 };
    if last >= 2 && sp.anchors.get(other).is_some_and(|a| a.p.distance(at) < EPS) {
        let path = path_mut(d, id)?;
        let Some(sp) = path.subpaths.get_mut(si) else { return Ok(false) };
        if let Some(l) = sp.anchors.pop()
            && let Some(f) = sp.anchors.first_mut()
        {
            f.h_in = l.h_in;
        }
        sp.closed = true;
        return Ok(true);
    }
    // Another open end at the same spot: on this path or on another editable path.
    let mut target: Option<(NodeId, usize, bool)> = None;
    d.walk(|n| {
        if target.is_some() || !matches!(n.kind, NodeKind::Path { guide: false, .. }) {
            return;
        }
        let Some(pd) = n.path_data() else { return };
        for (tsi, t) in pd.subpaths.iter().enumerate() {
            if t.closed || t.anchors.len() < 2 || (n.id == id && tsi == si) {
                continue;
            }
            if t.anchors.first().is_some_and(|a| a.p.distance(at) < EPS) {
                target = Some((n.id, tsi, false));
            } else if t.anchors.last().is_some_and(|a| a.p.distance(at) < EPS) {
                target = Some((n.id, tsi, true));
            }
            if target.is_some() {
                return;
            }
        }
    });
    let Some((tid, tsi, at_last)) = target else { return Ok(false) };
    if tid != id && !d.is_editable(tid) {
        return Ok(false);
    }
    let Some(mut y) = d.node(tid).and_then(|n| n.path_data()).and_then(|p| p.subpaths.get(tsi)).cloned() else { return Ok(false) };
    let Some(mut x) = d.node(id).and_then(|n| n.path_data()).and_then(|p| p.subpaths.get(si)).cloned() else { return Ok(false) };
    // x runs into the dragged end, y runs out of the target end.
    if ai == 0 {
        x.reverse();
    }
    if at_last {
        y.reverse();
    }
    if !y.anchors.is_empty() {
        let first = y.anchors.remove(0);
        if let Some(l) = x.anchors.last_mut() {
            l.h_out = first.h_out;
        }
    }
    x.anchors.extend(y.anchors);
    if tid == id {
        let path = path_mut(d, id)?;
        let Some(slot) = path.subpaths.get_mut(si) else { return Ok(false) };
        *slot = x;
        if tsi < path.subpaths.len() {
            path.subpaths.remove(tsi);
        }
    } else {
        let empty = {
            let tp = path_mut(d, tid)?;
            if tsi < tp.subpaths.len() {
                tp.subpaths.remove(tsi);
            }
            tp.subpaths.is_empty()
        };
        if empty {
            d.remove(tid)?;
        }
        let path = path_mut(d, id)?;
        let Some(slot) = path.subpaths.get_mut(si) else { return Ok(false) };
        *slot = x;
    }
    Ok(true)
}

fn set_handle(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.setHandle", "missing id"))?;
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
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

fn set_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("path.setAnchors", "missing id"))?;
    let subs = p.get("subpaths").and_then(Value::as_array).ok_or_else(|| bad("path.setAnchors", "missing subpaths"))?;
    let data = PathData::new(
        subs.iter()
            .map(|sp| {
                SubPath::new(
                    sp.get("anchors").and_then(Value::as_array).map(|a| a.iter().filter_map(anchor_from_json).collect()).unwrap_or_default(),
                    bool_or(sp, "closed", false),
                )
            })
            .collect(),
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

fn join(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_paths(s)?;
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
                }
                sp.closed = true;
                Ok(())
            })?;
        }
        [a, b, ..] => {
            let (a, b) = (*a, *b);
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
                // Pick the closest pair of ends.
                let cands =
                    [(xl.distance(yf), false, false), (xl.distance(yl), false, true), (xf.distance(yf), true, false), (xf.distance(yl), true, true)];
                let (_, rx, ry) = cands.into_iter().min_by(|p, q| p.0.total_cmp(&q.0)).unwrap_or((0.0, false, false));
                if rx {
                    x.reverse();
                }
                if ry {
                    y.reverse();
                }
                if let (Some(xe), Some(ys)) = (x.anchors.last().map(|a| a.p), y.anchors.first().map(|a| a.p))
                    && xe.distance(ys) < 1e-6
                {
                    let first = y.anchors.remove(0);
                    if let Some(l) = x.anchors.last_mut() {
                        l.h_out = first.h_out;
                    }
                }
                x.anchors.extend(y.anchors);
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

type AnchorTargets = Vec<(NodeId, Vec<(usize, usize)>)>;

fn anchor_targets(s: &Session) -> Result<AnchorTargets> {
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
    let t = anchor_targets(s)?;
    let smooth = str_param(p, "to") == Some("smooth");
    s.edit("Convert Anchor Points", |d, _| {
        for (id, v) in &t {
            let path = path_mut(d, *id)?;
            for &(si, ai) in v {
                let sp = &path.subpaths[si];
                let n = sp.anchors.len();
                let prev = if ai > 0 {
                    Some(sp.anchors[ai - 1].p)
                } else if sp.closed {
                    Some(sp.anchors[n - 1].p)
                } else {
                    None
                };
                let next = if ai + 1 < n {
                    Some(sp.anchors[ai + 1].p)
                } else if sp.closed {
                    Some(sp.anchors[0].p)
                } else {
                    None
                };
                let a: &mut Anchor = &mut path.subpaths[si].anchors[ai];
                if smooth {
                    // Handles parallel to prev→next, a third of the neighbour distances.
                    let (pp, nn) = (prev.unwrap_or(a.p), next.unwrap_or(a.p));
                    let dir = nn - pp;
                    let l = dir.hypot();
                    if l > 1e-9 {
                        let u = dir / l;
                        a.h_in = a.p - u * (a.p.distance(pp) / 3.0);
                        a.h_out = a.p + u * (a.p.distance(nn) / 3.0);
                        a.kind = AnchorKind::Smooth;
                    }
                } else {
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
    s.edit("Clear", |d, selection| {
        for (id, set) in &sel {
            let remove_node = {
                let path = path_mut(d, *id)?;
                // Deleting an anchor from a closed path opens it there (Illustrator behaviour on Delete).
                for &(si, ai) in set.iter().rev() {
                    if let Some(sp) = path.subpaths.get_mut(si)
                        && ai < sp.anchors.len()
                    {
                        if sp.closed {
                            sp.anchors.rotate_left(ai + 1);
                            sp.anchors.pop();
                            sp.closed = false;
                        } else {
                            sp.anchors.remove(ai);
                        }
                    }
                }
                path.subpaths.retain(|s| s.anchors.len() >= 2);
                path.subpaths.is_empty()
            };
            if remove_node {
                d.remove(*id)?;
            }
        }
        selection.clear();
        Ok(())
    })?;
    ok()
}

/// Has the Direct Selection tool picked segments (still valid) in the selection?
pub(crate) fn has_picked_segments(s: &Session) -> Result<bool> {
    let st = s.doc()?;
    Ok(st.selection.segments.keys().any(|id| !st.selection.segments_of(&st.doc, *id).is_empty()))
}

/// Delete the picked segments (Edit > Clear with segments picked): a closed path opens there,
/// an open one splits in two; pieces left with a single anchor go, and so does a path left with
/// nothing. A live shape becomes a plain path first.
pub(crate) fn delete_segments(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    type Picked = (NodeId, Vec<(usize, usize, usize)>);
    let picked: Vec<Picked> =
        st.selection.segments.keys().map(|id| (*id, st.selection.segments_of(&st.doc, *id))).filter(|(_, v)| !v.is_empty()).collect();
    s.edit("Clear", |d, selection| {
        for (id, segs) in &picked {
            // Only closed shapes keep a fill: opening a filled shape leaves its outline as plain
            // lines, and its fill as a shape of its own (no stroke) just below them.
            let opens_filled = d.node(*id).is_some_and(|n| {
                !n.appearance.fill_paint().is_none()
                    && n.path_data().is_some_and(|pd| segs.iter().any(|(si, _, _)| pd.subpaths.get(*si).is_some_and(|sp| sp.closed)))
                    && n.path_data().is_some_and(|pd| pd.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>() > segs.len())
            });
            if opens_filled
                && let Some(orig) = d.node(*id).cloned()
                && let Some((parent, idx, _)) = d.position(*id)
            {
                let mut fill = d.reid(&orig);
                fill.appearance.items.retain(|i| i.is_fill());
                d.insert(parent, idx, fill)?;
                if let Some(n) = d.node_mut(*id) {
                    n.appearance.items.retain(|i| !i.is_fill());
                }
            }
            let remove_node = {
                let path = path_mut(d, *id)?;
                let mut out = Vec::with_capacity(path.subpaths.len());
                for (si, sp) in path.subpaths.iter().enumerate() {
                    let cuts: Vec<usize> = segs.iter().filter(|(s, _, _)| *s == si).map(|(_, a0, _)| *a0).collect();
                    out.extend(cut_segments(sp, &cuts));
                }
                path.subpaths = out;
                path.subpaths.is_empty()
            };
            if remove_node {
                d.remove(*id)?;
            }
            selection.remove(*id);
        }
        Ok(())
    })?;
    ok()
}

/// `sp` without the segments starting at the anchors in `cuts`, as open pieces of 2+ anchors.
fn cut_segments(sp: &SubPath, cuts: &[usize]) -> Vec<SubPath> {
    let n = sp.anchors.len();
    let mut cuts: Vec<usize> = cuts.iter().copied().filter(|c| *c < n).collect();
    cuts.sort_unstable();
    cuts.dedup();
    if cuts.is_empty() {
        return vec![sp.clone()];
    }
    // Runs of anchor indices between the cuts.
    let runs: Vec<Vec<usize>> = if sp.closed {
        (0..cuts.len())
            .map(|i| {
                let start = cuts[i] + 1;
                let end = cuts[(i + 1) % cuts.len()];
                let len = (end + n - start % n) % n + 1;
                (0..len).map(|k| (start + k) % n).collect()
            })
            .collect()
    } else {
        let mut runs = vec![];
        let mut from = 0;
        for c in cuts {
            runs.push((from..=c).collect());
            from = c + 1;
        }
        runs.push((from..n).collect());
        runs
    };
    runs.into_iter()
        .filter(|r: &Vec<usize>| r.len() >= 2)
        .map(|r| SubPath::new(r.iter().filter_map(|i| sp.anchors.get(*i).copied()).collect(), false))
        .collect()
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
        let origin = sp.anchors[grabbed].p;
        let r = radius.unwrap_or_else(|| {
            let mut bp = vectorcraft_geom::BezPath::new();
            sp.to_bezpath_into(&mut bp);
            let b = vectorcraft_geom::Shape::bounding_box(&bp);
            (b.width().hypot(b.height()) / 2.0).max(1.0)
        });
        for (i, a) in sp.anchors.iter_mut().enumerate() {
            let w = if i == grabbed {
                1.0
            } else {
                let q = (a.p.distance(origin) / r).min(1.0);
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
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
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
    let sel = s.doc()?.selection.anchors.clone();
    s.edit("Cut Path", |d, _| {
        for (id, set) in &sel {
            let path = path_mut(d, *id)?;
            let mut out = vec![];
            for (si, sp) in path.subpaths.iter().enumerate() {
                let cuts: Vec<usize> = set.iter().filter(|(s, _)| *s == si).map(|(_, a)| *a).collect();
                if cuts.is_empty() {
                    out.push(sp.clone());
                    continue;
                }
                let mut anchors = sp.anchors.clone();
                if sp.closed {
                    anchors.rotate_left(cuts[0]);
                    anchors.push(anchors[0]);
                }
                let base = if sp.closed { cuts[0] } else { 0 };
                let n = sp.anchors.len();
                let rel: Vec<usize> = cuts.iter().map(|c| (c + n - base) % n).collect();
                let mut cur = vec![];
                for (i, a) in anchors.iter().enumerate() {
                    cur.push(*a);
                    if rel.contains(&(i % n)) && cur.len() > 1 {
                        out.push(SubPath::new(std::mem::take(&mut cur), false));
                        cur.push(*a);
                    }
                }
                if cur.len() > 1 {
                    out.push(SubPath::new(cur, false));
                }
            }
            path.subpaths = out;
        }
        Ok(())
    })?;
    ok()
}

/// Are anchors picked where a path can be split (a corner of a closed path, or a point inside
/// an open one)?
fn has_unlockable(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let ok = st.selection.anchors.iter().any(|(id, set)| {
        st.doc
            .node(*id)
            .and_then(|n| n.path_data())
            .is_some_and(|pd| set.iter().any(|(si, ai)| pd.subpaths.get(*si).is_some_and(|sp| !split_points(sp, &[*ai]).is_empty())))
    });
    if ok { Ok(()) } else { Err("select a corner or a point inside a line with the Direct Selection tool".into()) }
}

/// The anchors of `sp` among `picked` it can be split at: any anchor of a closed subpath, an
/// inner anchor of an open one.
fn split_points(sp: &SubPath, picked: &[usize]) -> Vec<usize> {
    let n = sp.anchors.len();
    let mut v: Vec<usize> = picked.iter().copied().filter(|a| *a < n && (sp.closed && n >= 2 || *a > 0 && *a + 1 < n)).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// `sp` split at the anchors `cuts` (from [`split_points`]) into open pieces; each split point
/// ends one piece and starts the next, at the same spot.
fn split_subpath(sp: &SubPath, cuts: &[usize]) -> Vec<SubPath> {
    let n = sp.anchors.len();
    let (Some(&first), Some(&last)) = (cuts.first(), cuts.last()) else { return vec![sp.clone()] };
    // Runs of anchor indices (mod n for a closed subpath), each from one split point to the next.
    let mut runs: Vec<(usize, usize)> = cuts.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect();
    if sp.closed {
        runs.push((last, first + n));
    } else {
        runs.insert(0, (0, first));
        runs.push((last, n.saturating_sub(1)));
    }
    runs.into_iter()
        .filter(|(a, b)| b > a)
        .map(|(a, b)| {
            let mut anchors: Vec<Anchor> = (a..=b).filter_map(|i| sp.anchors.get(i % n).copied()).collect();
            if let Some(f) = anchors.first_mut() {
                f.h_in = f.p;
            }
            if let Some(l) = anchors.last_mut() {
                l.h_out = l.p;
            }
            SubPath::new(anchors, false)
        })
        .collect()
}

/// Object > Path > Unlock Anchor Points: split paths at the picked anchors into separate lines.
fn unlock_anchors(s: &mut Session, _: &Value) -> Result<Value> {
    let picked = s.doc()?.selection.anchors.clone();
    let ids = s.edit("Unlock Anchor Points", |d, sel| {
        let mut out = vec![];
        for (id, set) in &picked {
            let Some(node) = d.node(*id).cloned() else { continue };
            let Some(pd) = node.path_data() else { continue };
            let mut keep = vec![];
            let mut pieces = vec![];
            let mut opens_closed = false;
            for (si, sp) in pd.subpaths.iter().enumerate() {
                let at: Vec<usize> = set.iter().filter(|(s, _)| *s == si).map(|(_, a)| *a).collect();
                let cuts = split_points(sp, &at);
                if cuts.is_empty() {
                    keep.push(sp.clone());
                } else {
                    opens_closed |= sp.closed;
                    pieces.extend(split_subpath(sp, &cuts));
                }
            }
            if pieces.is_empty() {
                continue;
            }
            // A filled shape that opens keeps its fill as a shape of its own just below.
            let mut lines = node.clone();
            if opens_closed && !node.appearance.fill_paint().is_none() {
                let mut fill = d.reid(&node);
                fill.appearance.items.retain(|i| i.is_fill());
                if let Some((parent, idx, _)) = d.position(*id) {
                    d.insert(parent, idx, fill)?;
                }
                lines.appearance.items.retain(|i| !i.is_fill());
            }
            let mut rest = pieces.into_iter();
            keep.extend(rest.next());
            {
                let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
                n.appearance = lines.appearance.clone();
            }
            *path_mut(d, *id)? = PathData::new(keep);
            out.push(*id);
            // The other pieces become lines of their own, just above, styled the same.
            let Some((parent, idx, _)) = d.position(*id) else { continue };
            for (k, piece) in rest.enumerate() {
                let mut n = d.reid(&lines);
                if let NodeKind::Path { path, live, .. } = &mut n.kind {
                    *path = PathData::single(piece);
                    *live = None;
                }
                out.push(d.insert(parent, idx + 1 + k, n)?);
            }
        }
        if out.is_empty() {
            return Err(EngineError::Other("select a corner or a point inside a line with the Direct Selection tool".into()));
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
