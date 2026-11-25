use super::common::{
    PenRecord, SIZES, Sample, brush_region, path_kind, path_mut, path_of, poly_bounds, pressure_region, ranged, samples_param, size_profile,
    stroke_paint, subpath_len, target_paths, within_input_bound,
};
use super::{
    Anchor, Appearance, BTreeSet, BoolOp, Cap, Color, Document, EngineError, FillRule, Join, NewArt, Node, NodeId, NodeKind, Paint, PathData, Point,
    Result, Selection, Session, SimplifyOptions, SubPath, Value, bad, insert_look, json, number_or, object_param, po, remove_each, str_param,
    switch_or,
};

// ---------- Pencil / Paintbrush ----------

/// Fit a freehand polyline: drop jitter, build a spline through the samples (sharp turns stay
/// corners) and simplify it to `tol`. A closed stroke is fitted as the open one is and closed by a
/// straight segment from its end back to its start: fitting round the gap would curve that segment
/// on from both ends, past them and back.
fn fit_freehand(pts: &[Point], tol: f64, closed: bool) -> SubPath {
    let min = (tol * 0.3).max(vectorcraft_tools::draw2::MIN_FIT_TOLERANCE * 0.3);
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
    let sp = vectorcraft_tools::draw2::catmull_rom(&marked, false);
    if sp.anchors.len() < 3 {
        // Two anchors closed would run back over the line itself.
        return SubPath::of(sp.anchors, closed && n > 2);
    }
    let fitted = po::simplify_with(
        &PathData::single(sp.clone()),
        &SimplifyOptions { tolerance: tol, corner_angle_deg: 60.0, straight_lines: false, ..Default::default() },
    );
    let mut anchors = fitted.subpaths.into_iter().next().filter(|s| s.anchors.len() >= 2).unwrap_or(sp).anchors;
    if closed {
        // No handle on either side of the closing segment.
        if let Some(l) = anchors.last_mut() {
            l.h_out = l.p;
        }
        if let Some(f) = anchors.first_mut() {
            f.h_in = f.p;
        }
    }
    SubPath::of(anchors, closed)
}

/// The path of a freehand stroke: fitted at `tol` ([`fit_freehand`]), or with no `tol` the points
/// joined by straight segments (`path.freehand` with `fit: false`).
fn stroke_subpath(pts: &[Point], tol: Option<f64>, closed: bool) -> SubPath {
    match tol {
        Some(tol) => fit_freehand(pts, tol, closed),
        None => SubPath::new(pts.iter().map(|&p| Anchor::corner(p)).collect(), closed),
    }
}

/// The anchor of `sp` at the point of it nearest to `q`: the one there, else a new one, and
/// whether it was added.
fn anchor_at(sp: &mut SubPath, q: Point) -> Option<(usize, bool)> {
    let (_, seg, t, _, _) = PathData::single(sp.clone()).nearest(q)?;
    let n = sp.anchors.len();
    if n == 0 {
        return None;
    }
    Some(if t <= 1e-6 {
        (seg % n, false)
    } else if t >= 1.0 - 1e-6 {
        ((seg + 1) % n, false)
    } else {
        (sp.insert_anchor(seg, t), true)
    })
}

/// The direction `sp` leaves anchor `k` going forward (towards the next anchor).
fn forward_at(sp: &SubPath, k: usize) -> Option<kurbo::Vec2> {
    let a = sp.anchors.get(k)?;
    let n = sp.anchors.len();
    let next = if sp.closed || k + 1 < n { sp.anchors.get((k + 1) % n) } else { None };
    let toward = if a.h_out != a.p {
        a.h_out
    } else if let Some(b) = next {
        if b.h_in != b.p { b.h_in } else { b.p }
    } else {
        // The last anchor of an open subpath: forward goes on the way it arrived.
        let prev = sp.anchors.get(k.checked_sub(1)?)?;
        return Some(a.p - if a.h_in != a.p { a.h_in } else { prev.p });
    };
    let v = toward - a.p;
    (v.hypot() > 1e-9).then_some(v)
}

/// The direction stroke `pts` leaves its first point: towards its first point farther than
/// `reach` from it, or its last point.
fn heading(pts: &[Point], reach: f64) -> Option<kurbo::Vec2> {
    let (&p0, rest) = pts.split_first()?;
    let to = rest.iter().find(|q| q.distance(p0) > reach).or(rest.last())?;
    let v = *to - p0;
    (v.hypot() > 1e-9).then_some(v)
}

/// `stroke` (its first point moved onto anchor `a` of the path it leaves) fitted at `tol`: its
/// anchors, the first of which is at `a`.
fn fit_from(a: Point, pts: &[Point], tol: Option<f64>, reach: f64) -> Vec<Anchor> {
    let mut stroke = pts.to_vec();
    bend_ends(&mut stroke, Some(a), None, reach);
    stroke_subpath(&stroke, tol, false).anchors.into_vec()
}

/// Bend the ends of `stroke` onto `a` (its start) and `b` (its end): each end moves all the way
/// and the stroke behind it less and less, to not at all `reach` along it (Edit selected paths
/// within), so a stroke that starts or ends near the path joins it smoothly instead of with a
/// straight piece. A stroke shorter than `reach` bends over its own length, so each end still
/// lands on its target and the other end's bend doesn't move it off.
fn bend_ends(stroke: &mut [Point], a: Option<Point>, b: Option<Point>, reach: f64) {
    let (Some(&first), Some(&last)) = (stroke.first(), stroke.last()) else { return };
    // How far along the stroke each point lies, measured before anything moves.
    let mut along = Vec::with_capacity(stroke.len());
    let mut total = 0.0;
    let mut prev = first;
    for p in stroke.iter() {
        total += prev.distance(*p);
        prev = *p;
        along.push(total);
    }
    let reach = reach.min(total);
    if !reach.is_finite() || reach <= 0.0 {
        // No length to bend over: only the ends move.
        if let (Some(a), Some(p)) = (a, stroke.first_mut()) {
            *p = a;
        }
        if let (Some(b), Some(p)) = (b, stroke.last_mut()) {
            *p = b;
        }
        return;
    }
    let weight = |d: f64| (1.0 - d / reach).max(0.0);
    let (shift_a, shift_b) = (a.map(|a| a - first), b.map(|b| b - last));
    for (p, &d) in stroke.iter_mut().zip(&along) {
        if let Some(s) = shift_a {
            *p += s * weight(d);
        }
        if let Some(s) = shift_b {
            *p += s * weight(total - d);
        }
    }
}

/// Pencil's Edit Selected Paths for a stroke that starts on a subpath of `pd` (within `within` of
/// it) and ends away from it (#2850 follow-up): the path follows the stroke and ends where the
/// stroke does. A closed subpath opens where the stroke starts and keeps its whole outline, run
/// so that it arrives there heading the stroke's way. An open subpath is cut there and keeps the
/// part behind the stroke: the part the stroke heads along is dropped. The subpath's index and its
/// new shape; None when the stroke doesn't start on `pd`. `runs_up`, when given, says which part of
/// an open subpath the stroke heads along: the part towards its last anchor when true.
fn redraw_extend(pd: &PathData, pts: &[Point], tol: Option<f64>, within: f64, runs_up: Option<bool>) -> Option<(usize, SubPath)> {
    let first = *pts.first()?;
    let (si, _, _, _, d) = pd.nearest(first)?;
    if d > within || pts.len() < 2 {
        return None;
    }
    let mut sp = pd.subpaths.get(si)?.clone();
    if sp.anchors.len() < 2 {
        return None;
    }
    let (mut k, _) = anchor_at(&mut sp, first)?;
    let last = sp.anchors.len() - 1;
    // Run the subpath so the stroke goes on from `k` forward; the result runs the subpath's own way
    // again below, so a shape's holes stay holes under the non-zero fill rule. A closed subpath
    // arrives at `k` heading the stroke's way. An open one drops the part the stroke heads along;
    // one it leaves nearly square to it (more than 70° from its direction) drops its shorter
    // part, never most of the path. From one of its ends it drops nothing.
    let dir = heading(pts, within.max(tol.unwrap_or(0.0) * 2.0))?;
    let cos = forward_at(&sp, k).map_or(0.0, |f| f.dot(dir) / (f.hypot() * dir.hypot()).max(1e-12));
    let flipped = if sp.closed {
        cos < 0.0
    } else if k == 0 || k == last {
        k == 0
    } else if let Some(up) = runs_up {
        !up
    } else if cos.abs() >= 0.34 {
        cos < 0.0
    } else {
        let len = |i: usize, j: usize| (i..j).map(|s| kurbo::ParamCurveArclen::arclen(&sp.segment(s), 0.01)).sum::<f64>();
        len(0, k) < len(k, last)
    };
    if flipped {
        sp.reverse();
        k = sp.anchors.len().checked_sub(1 + k)?;
    }
    let at = *sp.anchors.get(k)?;
    let mut anchors: Vec<Anchor> = if sp.closed {
        // From `k` all the way round back to `k`: the shape opens there.
        let mut v: Vec<Anchor> = sp.anchors.get_range(k..)?.into_iter().chain(sp.anchors.get_range(..k)?).collect();
        if let Some(a0) = v.first_mut() {
            *a0 = Anchor::with_handles(a0.p, a0.p, a0.h_out);
        }
        v.push(at);
        v
    } else {
        sp.anchors.get_range(..=k)?
    };
    let fit = fit_from(at.p, pts, tol, within);
    let (f0, rest) = fit.split_first()?;
    if let Some(a) = anchors.last_mut() {
        *a = Anchor::with_handles(a.p, a.h_in, f0.h_out);
    }
    anchors.extend_from_slice(rest);
    let mut out = SubPath::new(anchors, false);
    if flipped {
        out.reverse();
    }
    Some((si, out))
}

/// A stroke that leaves anchor `k` of `sp` and comes back to it: the stroke is inserted there as a
/// loop out of the path (#2850 follow-up).
fn redraw_loop(mut sp: SubPath, k: usize, pts: &[Point], tol: Option<f64>, within: f64) -> Option<SubPath> {
    let at = *sp.anchors.get(k)?;
    let mut stroke = pts.to_vec();
    bend_ends(&mut stroke, Some(at.p), Some(at.p), within);
    let fit = stroke_subpath(&stroke, tol, false).anchors.into_vec();
    let (Some(f0), Some(fl)) = (fit.first().copied(), fit.last().copied()) else { return None };
    if fit.len() < 3 {
        return None;
    }
    let inner = fit.get(1..fit.len() - 1)?;
    let mut anchors = sp.anchors.get_range(..k)?;
    anchors.push(Anchor::with_handles(at.p, at.h_in, f0.h_out));
    anchors.extend_from_slice(inner);
    anchors.push(Anchor::with_handles(at.p, fl.h_in, at.h_out));
    anchors.extend_from_slice(&sp.anchors.get_range(k + 1..)?);
    sp.anchors = anchors.into();
    Some(sp)
}

/// The area closed subpath `sp` would have with `stroke` (running from anchor `start` to anchor
/// `end`) in place of its run from `start` forward to `end`.
fn redrawn_area(sp: &SubPath, start: usize, end: usize, stroke: impl Iterator<Item = Point>) -> f64 {
    use kurbo::ParamCurveArea;
    let n = sp.anchors.len();
    let (Some(from), Some(to)) = (sp.anchors.get(start), sp.anchors.get(end)) else { return 0.0 };
    let mut area: f64 = (0..(start + n - end) % n).map(|k| sp.segment((end + k) % n).signed_area()).sum();
    let mut at = from.p;
    for q in stroke.chain([to.p]) {
        area += kurbo::Line::new(at, q).signed_area();
        at = q;
    }
    area.abs()
}

/// Which way a stroke runs along open subpath `sp` where it meets it: `stroke` starts at that end
/// of the stroke. It runs along when its first `within` of length stays within `within` of the
/// path and moves along the path more than away from it: Some(true) towards the subpath's last
/// anchor. None when it goes straight off (or onto) the path.
fn run_along(sp: &SubPath, mut stroke: impl Iterator<Item = Point>, within: f64) -> Option<bool> {
    let one = PathData::single(sp.clone());
    let p0 = stroke.next()?;
    // The point `within` along the stroke, or its other end when it is shorter.
    let (mut prev, mut len, mut q) = (p0, 0.0, p0);
    for p in stroke {
        let step = prev.distance(p);
        if len + step >= within && step > 0.0 {
            q = prev.lerp(p, (within - len) / step);
            break;
        }
        len += step;
        prev = p;
        q = p;
    }
    let (_, s0, t0, on0, _) = one.nearest(p0)?;
    let (_, s1, t1, on1, away) = one.nearest(q)?;
    if away > within || on0.distance(on1) <= away {
        return None;
    }
    Some(s1 as f64 + t1 > s0 as f64 + t0)
}

/// What a stroke from anchor `from` to anchor `to` of open subpath `sp` does to it.
enum OpenRedraw {
    /// Replaces the part between and leaves the path open.
    Replace,
    /// Keeps the part between, drops the pieces beyond and closes the path with the stroke.
    Close,
    /// Drops the part it starts along (towards the last anchor when true) and ends where it does.
    Extend(bool),
}

/// Pencil's Edit Selected Paths on an open subpath: what a stroke from anchor `from` to anchor `to`
/// of `sp` does, from the way it runs along the path at each end ([`run_along`]; `before` is the
/// subpath before the anchors at `from` and `to` were added). Where it starts, it replaces the part
/// it runs along; where it lands, it joins the part it runs into and the part behind goes. An end
/// that goes straight off or onto the path follows the other end. When both do, it closes only if
/// the shape it makes with the part between goes round both pieces beyond ([`goes_round`]). When
/// the two ends disagree, the start decides and the stroke's end stays where it is lifted.
fn open_redraw(before: &SubPath, sp: &SubPath, from: usize, to: usize, pts: &[Point], within: f64) -> OpenRedraw {
    let up = to > from;
    // Each end runs towards the last anchor (true) or the first. Running towards the other end
    // of the stroke at the start, or on away from it where it lands, replaces the part between;
    // the other way keeps the part between and closes the path.
    let at_start = run_along(before, pts.iter().copied(), within);
    let at_end = run_along(before, pts.iter().rev().copied(), within).map(|back| !back);
    match (at_start, at_end) {
        (Some(s), Some(e)) if s != e => OpenRedraw::Extend(s),
        (Some(r), _) | (None, Some(r)) => {
            if r == up {
                OpenRedraw::Replace
            } else {
                OpenRedraw::Close
            }
        }
        (None, None) => {
            if goes_round(sp, from, to, pts) {
                OpenRedraw::Close
            } else {
                OpenRedraw::Replace
            }
        }
    }
}

/// Does the shape that stroke `pts` (from anchor `from` to anchor `to` of open subpath `sp`) makes
/// with the part of `sp` between them go round both pieces of `sp` beyond them? A piece counts as
/// inside when more of it lies inside than outside; a piece of no length counts as inside.
fn goes_round(sp: &SubPath, from: usize, to: usize, pts: &[Point]) -> bool {
    use kurbo::{ParamCurve, Shape};
    let (lo, hi) = (from.min(to), from.max(to));
    let Some(middle) = sp.anchors.get_range(lo..=hi) else { return false };
    let mut shape = PathData::single(SubPath::new(middle, false)).to_bezpath();
    // The stroke runs from `hi` back to `lo` to close the shape.
    let back: Vec<Point> = if from == lo { pts.iter().rev().copied().collect() } else { pts.to_vec() };
    for p in back {
        shape.line_to(p);
    }
    shape.close_path();
    let inside = |segs: std::ops::Range<usize>| {
        let (mut yes, mut no) = (0, 0);
        for k in segs {
            let seg = sp.segment(k);
            for t in [0.25, 0.5, 0.75] {
                if shape.winding(seg.eval(t)) != 0 { yes += 1 } else { no += 1 }
            }
        }
        yes >= no
    };
    inside(0..lo) && inside(hi..sp.anchors.len().saturating_sub(1))
}

/// Closes open subpath `sp` with stroke `pts`, which runs from anchor `from` to anchor `to`: the
/// anchors between them stay, the pieces beyond go, and the stroke, its ends bent onto those
/// anchors ([`bend_ends`] over `within`), runs from one back to the other.
fn redraw_across_gap(sp: &SubPath, from: usize, to: usize, pts: &[Point], tol: Option<f64>, within: f64) -> Option<SubPath> {
    let (lo, hi) = (from.min(to), from.max(to));
    let (a_lo, a_hi) = (*sp.anchors.get(lo)?, *sp.anchors.get(hi)?);
    let mut stroke = pts.to_vec();
    if from == lo {
        stroke.reverse();
    }
    bend_ends(&mut stroke, Some(a_hi.p), Some(a_lo.p), within);
    let fit = stroke_subpath(&stroke, tol, false).anchors.into_vec();
    let (Some(f0), Some(fl)) = (fit.first().copied(), fit.last().copied()) else { return None };
    let mut anchors = sp.anchors.get_range(lo..=hi)?;
    if let Some(a) = anchors.first_mut() {
        *a = Anchor::with_handles(a.p, fl.h_in, a.h_out);
    }
    if let Some(a) = anchors.last_mut() {
        *a = Anchor::with_handles(a.p, a.h_in, f0.h_out);
    }
    anchors.extend_from_slice(fit.get(1..fit.len().saturating_sub(1)).unwrap_or_default());
    Some(SubPath::new(anchors, true))
}

/// Pencil's Edit Selected Paths in the middle of a path: a stroke that starts and ends within
/// `within` of a subpath of `pd` replaces the section between with the stroke fitted at `tol` (with
/// no `tol`, unfitted), its ends bent onto the path ([`bend_ends`]). On a closed subpath that is the
/// way round that leaves the bigger shape. On an open one the way the stroke runs along the path
/// at its ends decides ([`open_redraw`]): it replaces the part between, keeps the part between and
/// closes the path, or ends where it is lifted. The subpath's index and its new shape; None when
/// the stroke doesn't start and end on the same subpath.
fn redraw_section(pd: &PathData, pts: &[Point], tol: Option<f64>, within: f64) -> Option<(usize, SubPath)> {
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
    let before = sp.clone();
    // Each point is looked up on the subpath as it is by then: splitting a straight segment makes
    // two new ones, which a `t` along the old one doesn't measure.
    let (mut from, _) = anchor_at(&mut sp, first)?;
    let (to, added) = anchor_at(&mut sp, last)?;
    if added && to <= from {
        from += 1;
    }
    if from == to {
        // The stroke starts and ends on the same point: it is a loop out of the path there.
        return redraw_loop(sp, from, pts, tol, within).map(|sp| (si, sp));
    }
    if !sp.closed {
        match open_redraw(&before, &sp, from, to, pts, within) {
            OpenRedraw::Replace => {}
            OpenRedraw::Close => return redraw_across_gap(&sp, from, to, pts, tol, within).map(|sp| (si, sp)),
            OpenRedraw::Extend(up) => return redraw_extend(pd, pts, tol, within, Some(up)),
        }
    }
    let n = sp.anchors.len();
    // The stroke goes from `start` to `end` and replaces the run between them; `kept` runs from
    // `end` round to `start`, then the stroke's inner anchors follow.
    let (start, end, reversed) = if sp.closed {
        // The bigger of the two shapes the stroke could make stays: carving across a shape replaces
        // the part it cuts off, drawing round outside it the part it goes round. Picking the side
        // nearer the stroke instead threw away a thin shape under a tall arch over its long side.
        let forward = redrawn_area(&sp, from, to, pts.iter().copied());
        let backward = redrawn_area(&sp, to, from, pts.iter().rev().copied());
        if forward >= backward { (from, to, false) } else { (to, from, true) }
    } else if from < to {
        (from, to, false)
    } else {
        (to, from, true)
    };
    let mut stroke = pts.to_vec();
    if reversed {
        stroke.reverse();
    }
    let (Some(s1), Some(e1)) = (sp.anchors.get(start), sp.anchors.get(end)) else { return None };
    bend_ends(&mut stroke, Some(s1.p), Some(e1.p), within);
    let fit = stroke_subpath(&stroke, tol, false).anchors.into_vec();
    let (Some(f0), Some(f1)) = (fit.first().copied(), fit.last().copied()) else { return None };
    let inner = fit.get(1..fit.len().saturating_sub(1)).unwrap_or_default();
    let (before, after): (Vec<Anchor>, Vec<Anchor>) = if sp.closed {
        ((0..=(start + n - end) % n).filter_map(|k| sp.anchors.get((end + k) % n).copied()).collect(), vec![])
    } else {
        (sp.anchors.get_range(..=start)?, sp.anchors.get_range(end..)?)
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
    freehand_brushed(s, p, None)
}

/// The object a brush on path `id` paints: the compound path a member path is in, else the path.
fn brush_target(d: &Document, id: NodeId) -> NodeId {
    d.parent_of(id).filter(|p| d.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))).unwrap_or(id)
}

/// Whether `ap` has a stroke a brush can paint: a stroke whose paint is not None.
fn can_brush(ap: &Appearance) -> bool {
    ap.stroke().is_some_and(|st| !st.paint.is_none())
}

fn no_stroke(name: &str) -> EngineError {
    bad("brush.freehand", format!("cannot apply brush `{name}`: path has no stroke"))
}

/// Sets brush `name` on the stroke of object `id` of `d`. The stroke's own settings for another
/// brush (Options of Selected Object) are cleared.
fn set_brush(d: &mut Document, id: NodeId, name: &str) {
    if let Some(st) = d.node_mut(id).and_then(|n| n.appearance.stroke_mut()) {
        if st.brush.as_deref() != Some(name) {
            st.brush_options.clear();
        }
        st.brush = Some(name.to_string());
    }
}

/// `path.freehand`, with the stroke painted with brush `brush_name` (one `brush.freehand` has
/// checked) in the same undo step as the path. A path without a stroke is an error before
/// anything changes.
pub(crate) fn freehand_brushed(s: &mut Session, p: &Value, brush_name: Option<&str>) -> Result<Value> {
    const C: &str = "path.freehand";
    let samples = samples_param(p, "points", C)?;
    if samples.iter().any(|q| !within_input_bound(q.pt)) {
        return Err(bad(C, "x and y must be finite"));
    }
    let pen = PenRecord::of_samples(&samples);
    let mut pts: Vec<Point> = samples.into_iter().map(|s| s.pt).collect();
    let tol = number_or(p, "fidelity", 1.5, C)?;
    let (lo, hi) = (vectorcraft_tools::draw2::MIN_FIT_TOLERANCE, vectorcraft_tools::draw2::MAX_FIT_TOLERANCE);
    if !(lo..=hi).contains(&tol) {
        return Err(bad(C, format!("`fidelity` must be from {lo} to {hi} pt")));
    }
    // The tolerance the stroke is fitted at; with `fit: false` it is not fitted.
    let fit_tol = switch_or(p, "fit", true, C)?.then_some(tol);
    let closed = switch_or(p, "closed", false, C)?;
    let brush = match p.get("style").filter(|v| !v.is_null()).map(Value::as_str) {
        None | Some(Some("pencil")) => false,
        Some(Some("brush")) => true,
        Some(_) => return Err(bad(C, "style must be \"pencil\" or \"brush\"")),
    };
    // The undo step's name: a stroke painted with a brush is a Paintbrush stroke, whatever its style.
    let label = if brush || brush_name.is_some() { "Paintbrush" } else { "Pencil" };
    let keep_selected = switch_or(p, "keepSelected", true, C)?;
    // The Pencil's preview of a stroke that redraws a path draws it as a new path but leaves the
    // selection as it is: the path being redrawn stays selected until the release redraws it.
    let leave_selection = switch_or(p, "leaveSelection", false, C)?;
    let filled = switch_or(p, "fill", false, C)?;
    let extend = p.get("extend").filter(|v| !v.is_null());
    if extend.is_none() && p.get("connect").is_some_and(|v| !v.is_null()) {
        return Err(bad(C, "connect needs extend"));
    }
    if let Some(ext) = extend {
        let id = object_param(ext, "id", C)?.ok_or_else(|| bad(C, "extend needs id"))?;
        let at_start = match ext.get("end").filter(|v| !v.is_null()).map(Value::as_str) {
            None | Some(Some("end")) => false,
            Some(Some("start")) => true,
            Some(_) => return Err(bad(C, "extend end must be \"start\" or \"end\"")),
        };
        let (pd, _) = path_of(&s.doc()?.doc, id)?;
        let si = pd.subpaths.iter().rposition(|s| !s.closed && !s.anchors.is_empty()).ok_or_else(|| bad(C, "path has no open end"))?;
        let target = brush_target(&s.doc()?.doc, id);
        if let Some(name) = brush_name
            && !s.doc()?.doc.node(target).is_some_and(|n| can_brush(&n.appearance))
        {
            return Err(no_stroke(name));
        }
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
        // A stroke that ends on an open end of another path connects the two: (that path, its last
        // open subpath, that subpath starting at the end the stroke ends on, it ends at its end).
        // It is checked before a stroke of one point returns, so a bad `connect` is refused there too.
        let connect = match p.get("connect").filter(|v| !v.is_null()) {
            Some(cn) => {
                let other = object_param(cn, "id", C)?.ok_or_else(|| bad(C, "connect needs id"))?;
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
        if pts.len() < 2 {
            // No stroke to add: the brush alone changes, when it is not the path's already, and the
            // path is selected or deselected as a longer continuation leaves it.
            if let Some(name) = brush_name
                && s.doc()?.doc.node(target).and_then(|n| n.appearance.stroke()).is_some_and(|st| st.brush.as_deref() != Some(name))
            {
                s.edit("Paintbrush", |d, sel| {
                    set_brush(d, target, name);
                    if keep_selected {
                        sel.set([id]);
                    } else {
                        sel.remove(id);
                    }
                    Ok(())
                })?;
            }
            return Ok(json!({ "id": id.0 }));
        }
        // `closed`, when given, says whether the stroke closes the path: the Pencil and the
        // Paintbrush send it, by their Close Paths When Ends Are Within and Alt at the release.
        // Left out, an extending stroke that ends within four Fidelities of the path's other end
        // closes it: at least 6 pt and at most 400 pt, whatever the Fidelity it is fitted at.
        let close_tol = (tol * 4.0).clamp(6.0, 400.0);
        let near_first = pts.last().is_some_and(|p| p.distance(first) <= close_tol);
        let wants_close = if p.get("closed").is_some_and(|v| !v.is_null()) { closed } else { near_first };
        let closing = connect.is_none() && sp.anchors.len() >= 2 && wants_close;
        // A stroke ending near the other end ends on it; one ending farther away closes with a
        // straight segment back to it.
        let ends_on_first = closing && near_first;
        if ends_on_first && let Some(l) = pts.last_mut() {
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
        if ends_on_first
            && sp.anchors.len() > 2
            && let Some(l) = sp.anchors.pop()
            && let Some(a0) = sp.anchors.first_mut()
        {
            a0.h_in = l.h_in;
            sp.closed = true;
        } else if closing && !ends_on_first && sp.anchors.len() > 2 {
            // Closed from away from the other end: a straight segment back to it, whatever
            // handles the open ends carried.
            sp.closed = true;
            if let Some(l) = sp.anchors.last_mut() {
                l.h_out = l.p;
            }
            if let Some(f) = sp.anchors.first_mut() {
                f.h_in = f.p;
            }
        }
        for a in &mut sp.anchors {
            *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
        }
        let drawn_len = subpath_len(&sp);
        // The other path's anchors follow on from the one the stroke ends on, which takes its
        // outgoing handle.
        let mut other_len = 0.0;
        if let Some((_, _, y, _)) = &connect
            && let Some(y0) = y.anchors.first()
        {
            if let Some(last) = sp.anchors.last_mut() {
                *last = Anchor::with_handles(y0.p, last.h_in + (y0.p - last.p), y0.h_out);
            }
            sp.anchors.extend(y.anchors.iter().skip(1));
            other_len = subpath_len(y);
        }
        // What the pen recorded along the path (in the direction it is continued in), followed by
        // what it recorded along the stroke, and by what it recorded along the path it connects to.
        let record_of =
            |n: NodeId| s.doc().ok().and_then(|d| d.doc.node(n).and_then(|n| n.appearance.stroke()).map(PenRecord::of)).unwrap_or_default();
        let old = record_of(id);
        let other = connect.as_ref().map(|(o, _, _, at_end)| if *at_end { record_of(*o).reversed() } else { record_of(*o) }).unwrap_or_default();
        let joined = (!old.is_empty() || !pen.is_empty() || !other.is_empty()).then(|| {
            let old = if at_start { old.reversed() } else { old };
            let drawn = old.extended(old_len, &pen, drawn_len - old_len);
            if connect.is_some() { drawn.extended(drawn_len, &other, other_len) } else { drawn }
        });
        s.edit(label, |d, sel| {
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
                if reverse { joined.reversed() } else { joined }.write(st);
            }
            if let Some(name) = brush_name {
                set_brush(d, target, name);
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
        let id = object_param(rd, "id", C)?.ok_or_else(|| bad(C, "redraw needs id"))?;
        let within = number_or(rd, "within", 12.0, C)?;
        if !within.is_finite() || within < 0.0 {
            return Err(bad(C, "redraw within must be a finite distance of 0 or more"));
        }
        let (pd, _) = path_of(&s.doc()?.doc, id)?;
        // shortcut: a stroke's pressure, tilt, bearing and rotation profiles are left as they
        // were, measured along the old path; fit them to the new length if strokes drawn with a
        // pen get redrawn.
        // A stroke that ends back on the path redraws the section between; one that ends away from
        // it takes the path with it (it is where the pen is lifted that decides).
        if let Some((si, sp)) = redraw_section(&pd, &pts, fit_tol, within).or_else(|| redraw_extend(&pd, &pts, fit_tol, within, None)) {
            // A member path of a compound path is redrawn in it: the compound path is what is
            // selected, and what a brush paints.
            let whole = brush_target(&s.doc()?.doc, id);
            if let Some(name) = brush_name
                && !s.doc()?.doc.node(whole).is_some_and(|n| can_brush(&n.appearance))
            {
                return Err(no_stroke(name));
            }
            s.edit(label, |d, sel| {
                *path_mut(d, id)?.subpaths.get_mut(si).ok_or_else(|| bad(C, "no such subpath"))? = sp;
                if let Some(name) = brush_name {
                    set_brush(d, whole, name);
                }
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
        pen.write(st);
    }
    if let Some(name) = brush_name {
        if !can_brush(&look.appearance) {
            return Err(no_stroke(name));
        }
        if let Some(st) = look.appearance.stroke_mut() {
            st.brush = Some(name.to_string());
        }
    }
    let out = insert_look(s, label, path_kind(PathData::single(sp)), look, None, keep_selected)?;
    if !keep_selected && !leave_selection {
        // A new stroke with Keep selected off leaves nothing selected: Illustrator starts the stroke
        // by dropping the earlier selection, it does not keep it (#5752).
        s.select(|_, sel| sel.clear())?;
    }
    Ok(out)
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
    let samples = samples_param(p, "points", C)?;
    let size = ranged(p, "size", 10.0, SIZES, C)?;
    let angle = number_or(p, "angle", 0.0, C)?;
    if !(-180.0..=180.0).contains(&angle) {
        return Err(bad(C, "`angle` must be from -180 to 180 degrees"));
    }
    let roundness = number_or(p, "roundness", 100.0, C)?;
    if !(0.0..=100.0).contains(&roundness) {
        return Err(bad(C, "`roundness` must be from 0 to 100%"));
    }
    // With `fidelity` (pt), the brush follows the stroke fitted by [`fit_freehand`], as a Pencil
    // stroke is fitted. Without it, or when the fitted stroke has no length (a short stroke back to
    // its start), the brush follows every point.
    let fidelity = match p.get("fidelity") {
        None | Some(Value::Null) => None,
        Some(_) => Some(number_or(p, "fidelity", 0.0, C)?),
    };
    if fidelity.is_some_and(|f| !(vectorcraft_tools::draw2::MIN_FIDELITY..=100.0).contains(&f)) {
        return Err(bad(C, "`fidelity` must be from 0.01 to 100 pt"));
    }
    let tip = vectorcraft_tools::draw2::blob_tip(angle, roundness);
    let squash = tip.inverse();
    // Width profiles use distance along the spine, so measure the pressure or wheel on the same
    // squashed points that the outline reads for a non-round tip.
    let mapped_samples: Vec<Sample> = if roundness < 100.0 {
        samples
            .iter()
            .map(|s| {
                let mut mapped = *s;
                mapped.pt = squash * s.pt;
                mapped
            })
            .collect()
    } else {
        samples.clone()
    };
    let pressure = size_profile(p, &mapped_samples, C)?;
    let pts: Vec<Point> = samples.into_iter().map(|s| s.pt).collect();
    let mapped_pts: Vec<Point> = mapped_samples.into_iter().map(|s| s.pt).collect();
    // The widest the brush gets: the area merges look in.
    let reach = pressure.as_ref().map_or(size, |(_, v)| size + v);
    let spine = |tol: Option<f64>| {
        let path = match tol {
            Some(tol) => PathData::single(fit_freehand(&pts, tol, false)),
            None => PathData::single(SubPath::polyline(&pts, false)),
        };
        if roundness < 100.0 { path.transformed(squash) } else { path }
    };
    let region = match (&pressure, pts.len() > 1) {
        (Some((prof, variation)), true) => {
            let fitted = match fidelity {
                Some(tol) => Some(pressure_region(&spine(Some(tol)), size, *variation, prof)?).filter(|r| !r.is_empty()),
                None => None,
            };
            let dab = || brush_region(&mapped_pts[..1], vectorcraft_tools::draw2::pressure_size(size, *variation, prof.at(0.0)));
            match fitted {
                Some(r) => r,
                // Samples that all lie at one point give a spine with no length: one dab from the
                // first reading, as a click paints.
                None => Some(pressure_region(&spine(None), size, *variation, prof)?).filter(|r| !r.is_empty()).unwrap_or_else(dab),
            }
        }
        // A click: one dab as wide as the pen pressed or the wheel was set.
        (Some((prof, variation)), false) => brush_region(&mapped_pts, vectorcraft_tools::draw2::pressure_size(size, *variation, prof.at(0.0))),
        (None, _) => {
            let fitted = fidelity
                .filter(|_| pts.len() > 1)
                .map(|tol| po::outline_stroke(&spine(Some(tol)), size, Cap::Round, Join::Round, 4.0))
                .filter(|r| !r.is_empty());
            fitted.unwrap_or_else(|| brush_region(&mapped_pts, size))
        }
    };
    let region = if roundness < 100.0 { region.transformed(tip) } else { region };
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
        let area = region.control_bounds().map_or_else(|| poly_bounds(&pts, reach), |b| b.union(poly_bounds(&pts, reach)));
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

#[cfg(test)]
mod bend_tests {
    use super::*;

    /// The ends move onto their targets, the stroke behind them less and less, and nothing farther
    /// along than `reach`.
    #[test]
    fn bend_ends_moves_the_ends_and_fades_out_along_the_reach() {
        let mut pts: Vec<Point> = (0..=10).map(|i| Point::new(f64::from(i) * 10.0, 0.0)).collect();
        bend_ends(&mut pts, Some(Point::new(0.0, 8.0)), Some(Point::new(100.0, -8.0)), 40.0);
        assert_eq!(pts[0], Point::new(0.0, 8.0));
        assert_eq!(pts[10], Point::new(100.0, -8.0));
        assert!((pts[2].y - 8.0 * 0.5).abs() < 1e-9, "halfway along the reach, half the shift: {:?}", pts[2]);
        assert!((pts[8].y + 8.0 * 0.5).abs() < 1e-9, "{:?}", pts[8]);
        assert_eq!(pts[5], Point::new(50.0, 0.0), "the middle doesn't move");
    }

    /// A stroke shorter than `reach` still has both ends on their targets: bending one end onto
    /// its target doesn't pull the other end off its own.
    #[test]
    fn bend_ends_keeps_both_ends_on_target_for_a_short_stroke() {
        let mut pts = vec![Point::new(40.0, -12.0), Point::new(44.0, -12.0), Point::new(48.0, -12.0)];
        bend_ends(&mut pts, Some(Point::new(40.0, 0.0)), Some(Point::new(48.0, 0.0)), 16.0);
        assert_eq!(pts[0], Point::new(40.0, 0.0), "{pts:?}");
        assert_eq!(pts[2], Point::new(48.0, 0.0), "{pts:?}");
        assert_eq!(pts[1], Point::new(44.0, 0.0), "halfway, half of each end's shift: {pts:?}");
    }
}
