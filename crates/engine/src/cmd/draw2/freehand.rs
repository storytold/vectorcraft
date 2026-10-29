use super::common::{
    brush_region, path_kind, path_mut, path_of, points_param, poly_bounds, samples_param, stroke_paint, stroke_pressure, subpath_len, target_paths,
    within_input_bound,
};
use super::{
    Anchor, Appearance, BTreeSet, BoolOp, Cap, Color, Document, EngineError, FillRule, Join, NewArt, Node, NodeId, NodeKind, Paint, PathData, Point,
    PressureProfile, Result, Selection, Session, SimplifyOptions, SubPath, Value, bad, id_param, insert_look, json, number_or, po, remove_each,
    str_param, switch_or,
};

// ---------- Pencil / Paintbrush ----------

/// Fit a freehand polyline: drop jitter, build a spline through the samples (sharp turns stay
/// corners) and simplify it to `tol`.
fn fit_freehand(pts: &[Point], tol: f64, closed: bool) -> SubPath {
    let min = (tol * 0.3).max(vectorcraft_tools::draw2::MIN_FIDELITY * 0.3);
    let (Some(&p0), Some(&pn)) = (pts.first(), pts.last()) else { return SubPath::new(vec![], closed) };
    let mut kept: Vec<Point> = vec![p0];
    for &p in pts.iter().skip(1) {
        if kept.last().is_none_or(|k| k.distance(p) >= min) {
            kept.push(p);
        }
    }
    // The last sample always ends the stroke.
    if kept.len() > 1 {
        if let Some(k) = kept.last_mut() {
            *k = pn;
        }
    } else if pts.len() > 1 {
        kept.push(pn);
    }
    let n = kept.len();
    let marked: Vec<(Point, bool)> = (0..n)
        .map(|i| {
            let corner = if i == 0 || i + 1 == n {
                false
            } else {
                let (a, b) = (kept[i] - kept[i - 1], kept[i + 1] - kept[i]);
                a.dot(b) / (a.hypot() * b.hypot()).max(1e-12) < -0.2 // turn > ~100°
            };
            (kept[i], corner)
        })
        .collect();
    let sp = vectorcraft_tools::draw2::catmull_rom(&marked, closed);
    if sp.anchors.len() < 3 {
        return sp;
    }
    let fitted = po::simplify_with(&PathData::single(sp.clone()), &SimplifyOptions { tolerance: tol, corner_angle_deg: 60.0, straight_lines: false });
    fitted.subpaths.into_iter().next().filter(|s| s.anchors.len() >= 2).unwrap_or(sp)
}

/// The path of a freehand stroke: fitted at `tol` ([`fit_freehand`]), or with no `tol` the points
/// joined by straight segments (`path.freehand` with `fit: false`).
fn stroke_subpath(pts: &[Point], tol: Option<f64>, closed: bool) -> SubPath {
    match tol {
        Some(tol) => fit_freehand(pts, tol, closed),
        None => SubPath::new(pts.iter().map(|&p| Anchor::corner(p)).collect(), closed),
    }
}

/// Pencil's Edit Selected Paths in the middle of a path: a stroke from one point of a subpath of
/// `pd` to another, each within `within` of it, replaces the section between them (the shorter
/// way round a closed subpath) with the stroke fitted at `tol` (with no `tol`, unfitted). The
/// subpath's index and its new shape; None when the stroke doesn't start and end on the same
/// subpath, or meets it once.
fn redraw_section(pd: &PathData, pts: &[Point], tol: Option<f64>, within: f64) -> Option<(usize, SubPath)> {
    use kurbo::ParamCurveArclen;
    let (first, last) = (*pts.first()?, *pts.last()?);
    let (si, _, _, _, d_a) = pd.nearest(first)?;
    let (sj, _, _, _, d_b) = pd.nearest(last)?;
    if si != sj || d_a > within || d_b > within || pts.len() < 2 {
        return None;
    }
    let mut sp = pd.subpaths.get(si)?.clone();
    if sp.anchors.len() < 2 {
        return None;
    }
    // The anchor nearest to `q`: the one there, else a new one, and whether it was added. Each
    // point is looked up on the subpath as it is by then: splitting a straight segment makes two
    // new ones, which a `t` along the old one doesn't measure.
    let place = |sp: &mut SubPath, q: Point| {
        let (_, seg, t, _, _) = PathData::single(sp.clone()).nearest(q)?;
        let n = sp.anchors.len();
        Some(if t <= 1e-6 {
            (seg % n, false)
        } else if t >= 1.0 - 1e-6 {
            ((seg + 1) % n, false)
        } else {
            (sp.insert_anchor(seg, t), true)
        })
    };
    let (mut from, _) = place(&mut sp, first)?;
    let (to, added) = place(&mut sp, last)?;
    if added && to <= from {
        from += 1;
    }
    if from == to {
        return None;
    }
    let n = sp.anchors.len();
    // The length of the run of segments from anchor i forward to anchor j.
    let run = |i: usize, j: usize| (0..(j + n - i) % n).map(|k| sp.segment((i + k) % n).arclen(0.01)).sum::<f64>();
    // The stroke goes from `start` to `end` and replaces the run between them; `kept` runs from
    // `end` round to `start`, then the stroke's inner anchors follow.
    let (start, end, reversed) = if sp.closed {
        if run(from, to) <= run(to, from) { (from, to, false) } else { (to, from, true) }
    } else if from < to {
        (from, to, false)
    } else {
        (to, from, true)
    };
    let mut stroke = pts.to_vec();
    if reversed {
        stroke.reverse();
    }
    let (Some(s0), Some(s1)) = (stroke.first_mut(), sp.anchors.get(start)) else { return None };
    *s0 = s1.p;
    let (Some(e0), Some(e1)) = (stroke.last_mut(), sp.anchors.get(end)) else { return None };
    *e0 = e1.p;
    let fit = stroke_subpath(&stroke, tol, false).anchors;
    let (Some(f0), Some(f1)) = (fit.first().copied(), fit.last().copied()) else { return None };
    let inner = fit.get(1..fit.len().saturating_sub(1)).unwrap_or_default();
    let (before, after): (Vec<Anchor>, Vec<Anchor>) = if sp.closed {
        ((0..=(start + n - end) % n).filter_map(|k| sp.anchors.get((end + k) % n).copied()).collect(), vec![])
    } else {
        (sp.anchors.get(..=start)?.to_vec(), sp.anchors.get(end..)?.to_vec())
    };
    let mut anchors = before;
    if let Some(a) = anchors.last_mut() {
        *a = Anchor::with_handles(a.p, a.h_in, f0.h_out);
    }
    anchors.extend_from_slice(inner);
    let at_end = anchors.len();
    anchors.extend(after);
    // The anchor the stroke ends on: the first after it, or round to the first of a closed one.
    let end_at = if sp.closed { 0 } else { at_end };
    if let Some(a) = anchors.get_mut(end_at) {
        *a = Anchor::with_handles(a.p, f1.h_in, a.h_out);
    }
    Some((si, SubPath::new(anchors, sp.closed)))
}

pub(super) fn freehand(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.freehand";
    let samples = samples_param(p, "points", C)?;
    if samples.iter().any(|(q, _)| !within_input_bound(*q)) {
        return Err(bad(C, "x and y must be finite"));
    }
    let pressure = stroke_pressure(&samples);
    let mut pts: Vec<Point> = samples.into_iter().map(|(pt, _)| pt).collect();
    let tol = number_or(p, "fidelity", 1.5, C)?.clamp(vectorcraft_tools::draw2::MIN_FIDELITY, 100.0);
    // The tolerance the stroke is fitted at; with `fit: false` it is not fitted.
    let fit_tol = switch_or(p, "fit", true, C)?.then_some(tol);
    let closed = switch_or(p, "closed", false, C)?;
    let brush = str_param(p, "style") == Some("brush");
    let keep_selected = switch_or(p, "keepSelected", true, C)?;
    let filled = switch_or(p, "fill", false, C)?;
    if let Some(ext) = p.get("extend").filter(|v| !v.is_null()) {
        let id = id_param(ext, "id").ok_or_else(|| bad(C, "extend needs id"))?;
        let at_start = str_param(ext, "end") == Some("start");
        let (pd, _) = path_of(&s.doc()?.doc, id)?;
        let si = pd.subpaths.iter().rposition(|s| !s.closed && !s.anchors.is_empty()).ok_or_else(|| bad(C, "path has no open end"))?;
        let mut sp = pd.subpaths[si].clone();
        let old_len = subpath_len(&sp);
        if at_start {
            sp.reverse();
        }
        let (Some(first), Some(end)) = (sp.anchors.first().map(|a| a.p), sp.anchors.last().map(|a| a.p)) else {
            return Err(bad(C, "path has no open end"));
        };
        let Some(p0) = pts.first_mut() else { return Err(bad(C, "needs points")) };
        *p0 = end;
        if pts.len() < 2 {
            return Ok(json!({ "id": id.0 }));
        }
        // A stroke that ends on an open end of another path connects the two: (that path, its last
        // open subpath, that subpath starting at the end the stroke ends on, it ends at its end).
        let connect = match p.get("connect").filter(|v| !v.is_null()) {
            Some(cn) => {
                let other = id_param(cn, "id").ok_or_else(|| bad(C, "connect needs id"))?;
                if other == id {
                    return Err(bad(C, "connect needs a path other than the one extended"));
                }
                let (opd, _) = path_of(&s.doc()?.doc, other)?;
                let osi =
                    opd.subpaths.iter().rposition(|s| !s.closed && !s.anchors.is_empty()).ok_or_else(|| bad(C, "connect path has no open end"))?;
                let mut y = opd.subpaths[osi].clone();
                let at_end = match str_param(cn, "end") {
                    Some("start") => false,
                    Some("end") => true,
                    _ => return Err(bad(C, "connect end must be \"start\" or \"end\"")),
                };
                if at_end {
                    y.reverse();
                }
                Some((other, osi, y, at_end))
            }
            None => None,
        };
        let close_tol = (tol * 4.0).max(6.0);
        let closing = connect.is_none() && sp.anchors.len() >= 2 && pts.last().is_some_and(|p| p.distance(first) <= close_tol);
        if closing && let Some(l) = pts.last_mut() {
            *l = first;
        }
        if let Some((_, _, y, _)) = &connect
            && let (Some(l), Some(y0)) = (pts.last_mut(), y.anchors.first())
        {
            *l = y0.p;
        }
        let fit = stroke_subpath(&pts, fit_tol, false);
        let mut add = fit.anchors;
        if let (Some(last), Some(a0)) = (sp.anchors.last_mut(), add.first()) {
            last.h_out = a0.h_out;
        }
        if !add.is_empty() {
            add.remove(0);
        }
        sp.anchors.extend(add);
        if closing
            && sp.anchors.len() > 2
            && let Some(l) = sp.anchors.pop()
            && let Some(a0) = sp.anchors.first_mut()
        {
            a0.h_in = l.h_in;
            sp.closed = true;
        }
        for a in &mut sp.anchors {
            *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
        }
        let drawn_len = subpath_len(&sp);
        // The other path's anchors follow on from the one the stroke ends on, which takes its
        // outgoing handle.
        let mut other_len = 0.0;
        if let Some((_, _, y, _)) = &connect
            && let Some((y0, rest)) = y.anchors.split_first()
        {
            if let Some(last) = sp.anchors.last_mut() {
                *last = Anchor::with_handles(y0.p, last.h_in + (y0.p - last.p), y0.h_out);
            }
            sp.anchors.extend_from_slice(rest);
            other_len = subpath_len(y);
        }
        // The path's pressure (in the direction it is continued in) followed by the stroke's, and
        // by the pressure of the path it connects to.
        let pressure_of = |n: NodeId| s.doc().ok()?.doc.node(n).and_then(|n| n.appearance.stroke()).and_then(|st| st.pressure.clone());
        let old_pressure = pressure_of(id);
        let other_pressure = connect.as_ref().and_then(|(o, _, _, at_end)| pressure_of(*o).map(|p| if *at_end { p.reversed() } else { p }));
        let joined = (old_pressure.is_some() || pressure.is_some() || other_pressure.is_some()).then(|| {
            let old = old_pressure.map(|p| if at_start { p.reversed() } else { p });
            let drawn = PressureProfile::extended(old.as_ref(), old_len, pressure.as_ref(), drawn_len - old_len);
            if connect.is_some() { PressureProfile::extended(drawn.as_ref(), drawn_len, other_pressure.as_ref(), other_len) } else { drawn }
        });
        s.edit(if brush { "Paintbrush" } else { "Pencil" }, |d, sel| {
            let reverse = at_start && !sp.closed;
            if reverse {
                sp.reverse();
            }
            path_mut(d, id)?.subpaths[si] = sp;
            if let Some((other, osi, ..)) = &connect {
                let o = path_mut(d, *other)?;
                if *osi < o.subpaths.len() {
                    o.subpaths.remove(*osi);
                }
                if o.is_empty() {
                    d.remove(*other)?;
                    sel.remove(*other);
                }
                // Direct-selected anchors and segments of the subpaths after the one removed move
                // down one; those of the one removed go, and a path left with no selected anchors
                // is no longer selected.
                let shift = |refs: &mut BTreeSet<(usize, usize)>| {
                    *refs = refs.iter().filter(|r| r.0 != *osi).map(|&(sub, i)| if sub > *osi { (sub - 1, i) } else { (sub, i) }).collect();
                };
                if let Some(refs) = sel.anchors.get_mut(other) {
                    shift(refs);
                }
                if let Some(refs) = sel.segments.get_mut(other) {
                    shift(refs);
                }
                if sel.anchors.get(other).is_some_and(BTreeSet::is_empty) {
                    sel.remove(*other);
                }
            }
            if let Some(joined) = joined
                && let Some(st) = d.node_mut(id).and_then(|n| n.appearance.stroke_mut())
            {
                st.pressure = if reverse { joined.map(|p| p.reversed()) } else { joined };
            }
            if keep_selected {
                sel.set([id]);
            } else {
                sel.remove(id);
            }
            Ok(())
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    if let Some(rd) = p.get("redraw").filter(|v| !v.is_null()) {
        let id = id_param(rd, "id").ok_or_else(|| bad(C, "redraw needs id"))?;
        let within = number_or(rd, "within", 12.0, C)?;
        if !within.is_finite() || within < 0.0 {
            return Err(bad(C, "redraw within must be a finite distance of 0 or more"));
        }
        let (pd, _) = path_of(&s.doc()?.doc, id)?;
        // shortcut: a stroke pressure profile is left as it was, measured along the old path; fit
        // it to the new length if Calligraphic pressure strokes get redrawn.
        if let Some((si, sp)) = redraw_section(&pd, &pts, fit_tol, within) {
            s.edit(if brush { "Paintbrush" } else { "Pencil" }, |d, sel| {
                *path_mut(d, id)?.subpaths.get_mut(si).ok_or_else(|| bad(C, "no such subpath"))? = sp;
                // A member path of a compound path is redrawn in it: the compound path is what is
                // selected.
                let whole = d.parent_of(id).filter(|p| d.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))).unwrap_or(id);
                if keep_selected {
                    sel.set([whole]);
                } else {
                    sel.remove(whole);
                }
                Ok(())
            })?;
            return Ok(json!({ "id": id.0 }));
        }
    }
    if pts.len() < 2 {
        return Err(bad(C, "need at least two points"));
    }
    let sp = stroke_subpath(&pts, fit_tol, closed);
    let fill = if filled { s.paint.fill.clone() } else { Paint::None };
    // A pencil path with a fill keeps a stroke of None (#1679); only one that would show nothing
    // gets black. A brush is its stroke.
    let stroke = if brush || fill.is_none() { stroke_paint(s) } else { s.paint.stroke.clone() };
    let mut look = s.new_art_look(fill, stroke, s.paint.stroke_width.max(0.1));
    if let Some(st) = look.appearance.stroke_mut() {
        st.pressure = pressure;
    }
    insert_look(s, if brush { "Paintbrush" } else { "Pencil" }, path_kind(PathData::single(sp)), look, None, keep_selected)
}

// ---------- Blob Brush ----------

/// Whether `n` is a path a Blob Brush stroke of `paint` can merge with: the same fill and no stroke.
fn blob_match(n: &Node, paint: &Paint) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, .. }) && n.appearance.fill_paint() == *paint && n.appearance.stroke_paint().is_none()
}

/// The paths of `touched` that a Blob Brush stroke of `paint` merges with: the topmost of them, and
/// the others in the same group or layer that are adjacent to it in stacking order, with no object
/// between them other than a path of the same fill and no stroke.
fn adjacent_to_topmost(doc: &Document, touched: &[NodeId], paint: &Paint) -> Vec<NodeId> {
    let Some(top) = doc.paint_order(touched.iter().copied()).last().copied() else {
        return vec![];
    };
    let Some((parent, at, _)) = doc.position(top) else {
        return vec![top];
    };
    let Some(siblings) = doc.children(parent) else {
        return vec![top];
    };
    let mut out = vec![top];
    for n in siblings.iter().take(at).rev() {
        if touched.contains(&n.id) {
            out.push(n.id);
        } else if !blob_match(n, paint) {
            break;
        }
    }
    out
}

pub(super) fn blob(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.blob";
    let pts = points_param(p, "points", C)?;
    let size = number_or(p, "size", 10.0, C)?;
    if !(size.is_finite() && size > 0.0) {
        return Err(bad(C, "size must be positive"));
    }
    // With `fidelity` (pt), the brush follows the stroke fitted by [`fit_freehand`], as a Pencil
    // stroke is fitted. Without it, or when the fitted stroke has no length (a short stroke back to
    // its start), the brush follows every point.
    let fidelity = match p.get("fidelity") {
        None | Some(Value::Null) => None,
        Some(_) => Some(number_or(p, "fidelity", 0.0, C)?.clamp(vectorcraft_tools::draw2::MIN_FIDELITY, 100.0)),
    };
    let fitted = fidelity
        .filter(|_| pts.len() > 1)
        .map(|tol| po::outline_stroke(&PathData::single(fit_freehand(&pts, tol, false)), size, Cap::Round, Join::Round, 4.0))
        .filter(|r| !r.is_empty());
    let region = fitted.unwrap_or_else(|| brush_region(&pts, size));
    if region.is_empty() {
        return Err(EngineError::Other("empty brush stroke".into()));
    }
    let paint = if !s.paint.stroke.is_none() {
        s.paint.stroke.clone()
    } else if !s.paint.fill.is_none() {
        s.paint.fill.clone()
    } else {
        Paint::solid(Color::BLACK)
    };
    let keep_selected = switch_or(p, "keepSelected", false, C)?;
    let only_selection = switch_or(p, "mergeOnlySelection", false, C)?;
    let merge = switch_or(p, "merge", true, C)?;
    let st = s.doc()?;
    let mut merge_ids = vec![];
    if merge && !(only_selection && st.selection.is_empty()) {
        let all = Selection::default();
        let from = if only_selection { &st.selection } else { &all };
        // A fitted stroke can curve outside the points' bounds, so the area also takes in the shape.
        let area = region.control_bounds().map_or_else(|| poly_bounds(&pts, size), |b| b.union(poly_bounds(&pts, size)));
        for id in target_paths(&st.doc, from, area) {
            let Some(n) = st.doc.node(id) else { continue };
            if !blob_match(n, &paint) {
                continue;
            }
            let (pd, rule) = path_of(&st.doc, id)?;
            if pd.is_closed() && po::area(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Intersect), FillRule::NonZero) > 1e-6 {
                merge_ids.push((id, pd, rule));
            }
        }
        let joining = adjacent_to_topmost(&st.doc, &merge_ids.iter().map(|m| m.0).collect::<Vec<_>>(), &paint);
        merge_ids.retain(|m| joining.contains(&m.0));
    }
    let ap = NewArt::plain(Appearance::basic(paint, Paint::None, 1.0));
    if merge_ids.is_empty() {
        return insert_look(s, "Blob Brush", path_kind(region), ap, None, keep_selected);
    }
    let mut inputs: Vec<(&PathData, FillRule)> = vec![(&region, FillRule::NonZero)];
    inputs.extend(merge_ids.iter().map(|(_, pd, r)| (pd, *r)));
    let merged = po::try_unite_all(&inputs).map_err(|e| EngineError::Other(e.to_string()))?;
    let Some(keep) = st.doc.paint_order(merge_ids.iter().map(|m| m.0)).last().copied() else {
        return insert_look(s, "Blob Brush", path_kind(region), ap, None, keep_selected);
    };
    let others: Vec<NodeId> = merge_ids.iter().map(|m| m.0).filter(|id| *id != keep).collect();
    s.edit("Blob Brush", |d, sel| {
        *path_mut(d, keep)? = merged;
        remove_each(d, &others)?;
        if keep_selected {
            sel.set([keep]);
        } else {
            sel.remove(keep);
        }
        Ok(())
    })?;
    Ok(json!({ "id": keep.0 }))
}
