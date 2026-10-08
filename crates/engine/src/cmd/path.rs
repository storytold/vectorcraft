//! Path editing commands (used by the Pen and Direct Selection tools and Object → Path).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{AnchorKind, PathData, Point, SubPath, Vec2};

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
            "{dx, dy} move direct-selected anchors (whole paths if fully selected)",
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
            "path.handleMode",
            "Set Direction Handle Mode",
            ["Object", "Path"],
            None,
            "{mode: \"independent\"|\"aligned\"|\"mirrored\"} set selected anchors' handle coupling without retracting handles; fully selected paths apply to every anchor",
            has_selection,
            handle_mode
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
            "{to: \"corner\"|\"smooth\"} convert the direct-selected anchors (all anchors of fully selected paths) as one undo step: corner retracts both handles, smooth pulls handles in line with the neighbours (smooth anchors keep theirs)",
            has_selection,
            convert_anchors
        ),
        cmd!(
            "path.deleteAnchors",
            "Delete Anchor Points",
            [],
            None,
            "{} delete the direct-selected anchors and their segments, opening closed paths there (the Delete key)",
            has_anchors,
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
    let st = s.doc()?;
    let sel = st.selection.clone();
    let scale_strokes = s.prefs.scale_strokes;
    let _ = scale_strokes;
    s.edit("Move", |doc, _| {
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
        Ok(())
    })?;
    ok()
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

fn handle_mode(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.handleMode";
    let kind = match str_param(p, "mode") {
        Some("independent") => AnchorKind::Corner,
        Some("aligned") => AnchorKind::Smooth,
        Some("mirrored") => AnchorKind::Symmetric,
        _ => return Err(bad(C, "mode must be independent, aligned or mirrored")),
    };
    let targets = anchor_targets(s)?;
    if targets.is_empty() {
        return Err(bad(C, "select paths or anchor points"));
    }
    s.edit("Direction Handle Mode", |d, _| {
        for (id, refs) in &targets {
            let path = path_mut(d, *id)?;
            for &(si, ai) in refs {
                let Some(a) = path.anchor_mut(si, ai) else { continue };
                a.kind = kind;
                if kind != AnchorKind::Corner {
                    // Preserve the outgoing direction when present, otherwise the incoming one.
                    if a.has_out() {
                        a.set_handle(true, a.h_out, false);
                    } else if a.has_in() {
                        a.set_handle(false, a.h_in, false);
                    }
                }
            }
        }
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
        for (id, subs, starts) in cuts {
            // A path of one subpath becomes one path per piece (the first keeps the id); the
            // pieces of a compound shape stay subpaths of it.
            if d.node(id).and_then(|n| n.path_data()).is_some_and(|pd| pd.subpaths.len() == 1) {
                let ids = super::draw2::replace_with_pieces(d, id, subs.into_iter().map(PathData::single).collect())?;
                picked.extend(ids.iter().zip(&starts).filter(|(_, s)| **s).map(|(id, _)| (*id, (0, 0))));
                all.extend(ids);
            } else {
                path_mut(d, id)?.subpaths = subs;
                picked.extend(starts.iter().enumerate().filter(|(_, s)| **s).map(|(si, _)| (id, (si, 0))));
                all.push(id);
            }
        }
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
