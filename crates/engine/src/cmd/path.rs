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
            "path.setAnchors",
            "Set Anchors",
            [],
            None,
            "{id, subpaths: [{anchors: [{x,y,in?,out?}], closed}]} replace a path's geometry",
            has_doc,
            set_anchors
        ),
        cmd!("path.reverse", "Reverse Path Direction", ["Object", "Path"], None, "{}", has_selection, reverse),
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
        if independent {
            a.kind = AnchorKind::Corner;
        }
        let (moved, other) = if out { (&mut a.h_out, &mut a.h_in) } else { (&mut a.h_in, &mut a.h_out) };
        *moved = pos;
        if a.kind == AnchorKind::Smooth {
            // Keep the opposite handle collinear, preserving its length.
            let len = (*other - a.p).hypot();
            let dir = a.p - pos;
            let l = dir.hypot();
            if l > 1e-9 {
                *other = a.p + dir * (len / l);
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

fn reverse(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_paths(s)?;
    s.edit("Reverse Path Direction", |d, _| {
        for id in &ids {
            path_mut(d, *id)?.reverse();
        }
        Ok(())
    })?;
    ok()
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
                let first = sp.anchors.first().map(|a| a.p);
                if sp.anchors.len() > 2
                    && let Some(first) = first
                    && let Some(last) = sp.anchors.pop_if(|l| first.distance(l.p) < 1e-6)
                {
                    sp.anchors[0].h_in = last.h_in;
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
                let ends = |sp: &SubPath| sp.anchors.first().zip(sp.anchors.last()).map(|(f, l)| (f.p, l.p));
                let (Some((xf, xl)), Some((yf, yl))) = (ends(&x), ends(&y)) else {
                    return Err(EngineError::Other("join needs paths with anchors".into()));
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
                if let (Some(l), Some(f)) = (x.anchors.last_mut(), y.anchors.first())
                    && l.p.distance(f.p) < 1e-6
                {
                    l.h_out = f.h_out;
                    y.anchors.remove(0);
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
