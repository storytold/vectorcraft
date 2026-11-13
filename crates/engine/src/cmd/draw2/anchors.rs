use super::common::{MAX_INPUT, path_kind, path_mut, point_entry, usize_opt, usize_req, within_input_bound};
use super::{
    Anchor, BTreeSet, Beside, Color, Document, EngineError, Node, NodeId, NodeKind, Paint, PathData, Point, Result, Selection, Session, Value, Vec2,
    add_look, bad, check_numbers, f64_or, json, number_or, object_param, ok, po, str_param, switch_or,
};

// ---------- Curvature ----------

pub(super) fn curvature(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.curvature";
    let arr = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(C, "missing points"))?;
    let pts: Vec<(Point, bool)> = arr.iter().map(|v| Ok((point_entry(v, C)?, switch_or(v, "corner", false, C)?))).collect::<Result<_>>()?;
    if pts.is_empty() {
        return Err(bad(C, "need at least one point"));
    }
    let sp = vectorcraft_tools::draw2::curvature_spline(&pts, switch_or(p, "closed", false, C)?);
    if let Some(id) = crate::cmd::object_param(p, "id", C)? {
        s.edit("Curvature", |d, sel| {
            *path_mut(d, id)? = PathData::single(sp);
            sel.set([id]);
            Ok(())
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    let mut look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let ap = &mut look.appearance;
    if ap.stroke_paint().is_none() && ap.fill_paint().is_none() {
        ap.set_stroke(Paint::solid(Color::BLACK));
    }
    add_look(s, "Curvature", path_kind(PathData::single(sp)), look, None)
}

/// An edit `path.curvatureEdit` makes.
enum CurvatureOp {
    Move(usize, Point),
    Insert(usize, f64, Option<Point>),
    Extend(Point),
    Close,
}

pub(super) fn curvature_edit(s: &mut Session, p: &Value) -> Result<Value> {
    use vectorcraft_tools::draw2::{EndCurve, curvature_close, curvature_extend, curvature_insert, curvature_move};
    const C: &str = "path.curvatureEdit";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    check_numbers(p, &["x", "y", "t"], C)?;
    let at = match (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some(Point::new(x, y)),
        (None, None) => None,
        _ => return Err(bad(C, "x and y must be finite numbers")),
    };
    let to = || at.ok_or_else(|| bad(C, "missing x, y"));
    let start = match str_param(p, "end") {
        None | Some("end") => false,
        Some("start") => true,
        Some(e) => return Err(bad(C, format!("unknown end {e:?}"))),
    };
    let how = match str_param(p, "from") {
        None => EndCurve::Keep,
        Some(f) => EndCurve::parse(f).ok_or_else(|| bad(C, format!("unknown from {f:?}")))?,
    };
    let corner = switch_or(p, "corner", false, C)?;
    let op = match str_param(p, "op") {
        Some("move") => CurvatureOp::Move(usize_req(p, "anchor", C)?, to()?),
        Some("insert") => CurvatureOp::Insert(usize_req(p, "segment", C)?, f64_or(p, "t", 0.5), at),
        Some("extend") => CurvatureOp::Extend(to()?),
        Some("close") => CurvatureOp::Close,
        Some(o) => return Err(bad(C, format!("unknown op {o:?}"))),
        None => return Err(bad(C, "missing op")),
    };
    let anchor = s.edit("Curvature", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if matches!(op, CurvatureOp::Extend(_) | CurvatureOp::Close) && sp.closed {
            return Err(EngineError::Other("path is already closed".into()));
        }
        let ai = match op {
            CurvatureOp::Move(ai, q) => curvature_move(sp, ai, q).then_some(ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?,
            CurvatureOp::Insert(seg, t, q) => {
                let ai = curvature_insert(sp, seg, t).ok_or_else(|| EngineError::Other("no such segment".into()))?;
                if let Some(q) = q {
                    curvature_move(sp, ai, q);
                }
                ai
            }
            CurvatureOp::Extend(q) => {
                curvature_extend(sp, start, q, how, corner);
                if start { 0 } else { sp.anchors.len() - 1 }
            }
            CurvatureOp::Close => {
                if sp.anchors.len() < 3 {
                    return Err(EngineError::Other("closing needs three anchors".into()));
                }
                curvature_close(sp, start, how);
                if start { sp.anchors.len() - 1 } else { 0 }
            }
        };
        sel.set([id]);
        sel.anchors.insert(id, [(si, ai)].into());
        Ok(ai)
    })?;
    Ok(json!({ "anchor": anchor }))
}

// ---------- anchor tools ----------

pub(super) fn remove_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.removeAnchor";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let ai = usize_req(p, "anchor", C)?;
    let curvature = switch_or(p, "curvature", false, C)?;
    let removed = s.edit("Delete Anchor Point", |d, sel| {
        let sp = path_mut(d, id)?.subpaths.get(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if ai >= sp.anchors.len() {
            return Err(EngineError::Other("no such anchor".into()));
        }
        let mut emptied = Beside::replacing(d, &[id]);
        let removed = remove_anchors_of(d, sel, &mut emptied, id, vec![(si, ai)], curvature)?;
        emptied.finish(d)?;
        if removed {
            sel.remove(id);
        }
        Ok(removed)
    })?;
    Ok(json!({ "removedObject": removed }))
}

pub(super) fn remove_anchors(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let jobs: Vec<(NodeId, Vec<(usize, usize)>)> = st
        .selection
        .anchors
        .iter()
        .filter(|(id, set)| !set.is_empty() && st.doc.node(**id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. })))
        .map(|(id, set)| (*id, set.iter().copied().collect()))
        .collect();
    if jobs.is_empty() {
        return Err(bad("path.removeAnchors", "direct-select anchor points of a path"));
    }
    let removed = s.edit("Remove Anchor Points", |d, sel| {
        let mut gone = Vec::new();
        let mut emptied = Beside::replacing(d, &jobs.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, anchors) in jobs {
            let shape: Vec<(usize, bool)> =
                d.node(id).and_then(Node::path_data).map(|p| p.subpaths.iter().map(|sp| (sp.anchors.len(), sp.closed)).collect()).unwrap_or_default();
            let next = following_anchors(&shape, &anchors);
            if remove_anchors_of(d, sel, &mut emptied, id, anchors, false)? {
                gone.push(id);
            } else if !next.is_empty() {
                // The next anchor along is selected, so the next Remove Anchor Points goes on from
                // there without picking it first (#2241).
                sel.anchors.insert(id, next);
            }
        }
        emptied.finish(d)?;
        sel.remove_all(gone.iter().copied());
        Ok(gone.len())
    })?;
    Ok(json!({ "removedObjects": removed }))
}

/// Where the selection goes once Remove Anchor Points takes `removed` (subpath, anchor) out of a
/// path whose subpaths have `shape` (anchor count, closed): in each subpath, the anchor after its
/// last removed one, round the end of a closed subpath, or the one before it at an open subpath's
/// end. Indexes are those after the removal, without the subpaths left with fewer than two anchors.
/// `removed` is in ascending order without repeats, as a selection holds it, and each subpath reads
/// only its own pairs.
fn following_anchors(shape: &[(usize, bool)], removed: &[(usize, usize)]) -> BTreeSet<(usize, usize)> {
    let mut next = BTreeSet::new();
    let mut kept = 0;
    let mut rest = removed;
    for (si, &(n, closed)) in shape.iter().enumerate() {
        let (mine, after) = rest.split_at(rest.iter().take_while(|&&(s, _)| s == si).count());
        rest = after;
        let gone = &mine[..mine.partition_point(|&(_, a)| a < n)];
        if n - gone.len() < 2 {
            continue;
        }
        if let Some(&(_, last)) = gone.last() {
            // Anchors past `last` are all left. Below it, the first one left (round a closed
            // subpath's end) is past the removed run from 0, and the last one left is below the
            // removed run down from `last`.
            let a = if last + 1 < n {
                Some(last + 1)
            } else if closed {
                Some(gone.iter().zip(0..).take_while(|&(&(_, g), k)| g == k).count())
            } else {
                last.checked_sub(gone.iter().rev().zip((0..=last).rev()).take_while(|&(&(_, g), k)| g == k).count())
            };
            if let Some(a) = a {
                next.insert((kept, a - gone.partition_point(|&(_, g)| g < a)));
            }
        }
        kept += 1;
    }
    next
}

/// Remove `anchors` (subpath, anchor) of path `id` without opening it, refitting the curve round
/// each one, or with `curvature` re-curving it as the Curvature tool curves its points
/// ([`vectorcraft_tools::draw2::curvature_remove`]). A subpath left with one anchor is removed,
/// and a path left with no subpath is removed through `emptied`; the caller then takes it out of
/// the selection. True when the path went.
fn remove_anchors_of(
    d: &mut Document,
    sel: &mut Selection,
    emptied: &mut Beside,
    id: NodeId,
    mut anchors: Vec<(usize, usize)>,
    curvature: bool,
) -> Result<bool> {
    // The last first, so the indexes still to come stay valid.
    anchors.sort_unstable_by(|a, b| b.cmp(a));
    anchors.dedup();
    let path = path_mut(d, id)?;
    let mut rest = &anchors[..];
    while let Some(&(si, _)) = rest.first() {
        let (mine, after) = rest.split_at(rest.iter().take_while(|&&(s, _)| s == si).count());
        rest = after;
        let Some(sp) = path.subpaths.get_mut(si) else { continue };
        if curvature {
            for &(_, ai) in mine {
                vectorcraft_tools::draw2::curvature_remove(sp, ai);
            }
        } else {
            po::remove_anchors(sp, &mine.iter().map(|&(_, ai)| ai).collect::<Vec<_>>());
        }
    }
    path.subpaths.retain(|sp| sp.anchors.len() >= 2);
    let gone = path.is_empty();
    if gone {
        emptied.put(d, id, vec![])?;
    } else {
        sel.anchors.remove(&id);
        sel.segments.remove(&id);
    }
    Ok(gone)
}

pub(super) fn convert_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.convertAnchor";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let ai = usize_req(p, "anchor", C)?;
    let smooth = str_param(p, "to") == Some("smooth");
    check_numbers(p, &["x", "y"], C)?;
    let target = match (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        (Some(x), Some(y)) => Some(Point::new(x, y)),
        (None, None) => None,
        _ => return Err(bad(C, "x and y must be given together")),
    };
    if target.is_some_and(|q| !within_input_bound(q)) {
        return Err(bad(C, "x and y must be finite"));
    }
    s.edit("Convert Anchor Point", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if smooth && target.is_none() {
            if ai >= sp.anchors.len() {
                return Err(EngineError::Other("no such anchor".into()));
            }
            // Without a handle position the anchor curves as the Curvature tool curves a point; one
            // with no direction to take stays as it is.
            vectorcraft_tools::draw2::curvature_smooth(sp, ai);
            return Ok(());
        }
        let a = sp.anchors.get_mut(ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?;
        match target {
            Some(t) if smooth && t.distance(a.p) > 1e-9 => *a = Anchor::smooth(a.p, t),
            _ => a.retract(),
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn reshape_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.reshapeSegment";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let seg = usize_req(p, "segment", C)?;
    let t = number_or(p, "t", 0.5, C)?.clamp(0.05, 0.95);
    let dv = Vec2::new(number_or(p, "dx", 0.0, C)?, number_or(p, "dy", 0.0, C)?);
    if !(dv.x.is_finite() && dv.y.is_finite() && dv.hypot() < MAX_INPUT) {
        return Err(bad(C, "invalid delta"));
    }
    let perpendicular = switch_or(p, "perpendicular", false, C)?;
    s.edit("Reshape", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        let bent = if perpendicular { sp.bend_segment_perpendicular(seg, t, dv) } else { sp.bend_segment(seg, t, dv) };
        if !bent {
            return Err(EngineError::Other("no such segment".into()));
        }
        Ok(())
    })?;
    ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`following_anchors`] by its definition: for each subpath, a search of the anchors after
    /// its last removed one, round a closed end, then before it.
    fn following_by_search(shape: &[(usize, bool)], removed: &[(usize, usize)]) -> BTreeSet<(usize, usize)> {
        let mut next = BTreeSet::new();
        let mut kept = 0;
        for (si, &(n, closed)) in shape.iter().enumerate() {
            let gone: BTreeSet<usize> = removed.iter().filter(|&&(s, a)| s == si && a < n).map(|&(_, a)| a).collect();
            if n - gone.len() < 2 {
                continue;
            }
            if let Some(&last) = gone.last() {
                let wrap = if closed { 0..last } else { 0..0 };
                let after = (last + 1..n).chain(wrap).find(|a| !gone.contains(a));
                if let Some(a) = after.or_else(|| (0..last).rev().find(|a| !gone.contains(a))) {
                    next.insert((kept, a - gone.range(..a).count()));
                }
            }
            kept += 1;
        }
        next
    }

    #[test]
    fn following_anchors_matches_a_search_along_each_subpath() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % m) as usize
        };
        for case in 0..2000 {
            let shape: Vec<(usize, bool)> = (0..rnd(5) + 1).map(|_| (rnd(9), rnd(2) == 0)).collect();
            let removed: BTreeSet<(usize, usize)> = (0..rnd(14)).map(|_| (rnd(shape.len() as u64 + 1), rnd(11))).collect();
            let removed: Vec<_> = removed.into_iter().collect();
            assert_eq!(following_anchors(&shape, &removed), following_by_search(&shape, &removed), "case {case}: {shape:?} removing {removed:?}");
        }
    }
}
