use super::common::{
    brush_region, path_kind, path_mut, path_of, poly_bounds, pressure_region, samples_param, size_pressure, stroke_paint, stroke_pressure,
    subpath_len, target_paths, within_input_bound,
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
    let fitted = po::simplify_with(
        &PathData::single(sp.clone()),
        &SimplifyOptions { tolerance: tol, corner_angle_deg: 60.0, straight_lines: false, ..Default::default() },
    );
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
    stroke_subpath(&stroke, tol, false).anchors
}

/// Bend the ends of `stroke` onto `a` (its start) and `b` (its end): each end moves all the way
/// and the stroke behind it less and less, to not at all `reach` along it (Edit selected paths
/// within), so a stroke that starts or ends near the path joins it smoothly instead of with a
/// straight piece.
fn bend_ends(stroke: &mut [Point], a: Option<Point>, b: Option<Point>, reach: f64) {
    let bend = |pts: &mut dyn Iterator<Item = &mut Point>, to: Point| {
        let mut pts = pts.peekable();
        let Some(first) = pts.peek().map(|p| **p) else { return };
        let shift = to - first;
        let (mut along, mut prev) = (0.0, first);
        for p in pts {
            along += prev.distance(*p);
            prev = *p;
            let w = if reach > 0.0 {
                1.0 - along / reach
            } else if along == 0.0 {
                1.0
            } else {
                0.0
            };
            if w <= 0.0 {
                break;
            }
            *p += shift * w;
        }
    };
    if let Some(a) = a {
        bend(&mut stroke.iter_mut(), a);
    }
    if let Some(b) = b {
        bend(&mut stroke.iter_mut().rev(), b);
    }
}

/// Pencil's Edit Selected Paths for a stroke that starts on a subpath of `pd` (within `within` of
/// it) and ends away from it (#2850 follow-up): the path follows the stroke and ends where the
/// stroke does. A closed subpath opens where the stroke starts and keeps its whole outline, run
/// so that it arrives there heading the stroke's way. An open subpath is cut there and keeps the
/// part behind the stroke: the part the stroke heads along is dropped. The subpath's index and its
/// new shape; None when the stroke doesn't start on `pd`.
fn redraw_extend(pd: &PathData, pts: &[Point], tol: Option<f64>, within: f64) -> Option<(usize, SubPath)> {
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
        let mut v: Vec<Anchor> = sp.anchors.get(k..)?.iter().chain(sp.anchors.get(..k)?).copied().collect();
        if let Some(a0) = v.first_mut() {
            *a0 = Anchor::with_handles(a0.p, a0.p, a0.h_out);
        }
        v.push(at);
        v
    } else {
        sp.anchors.get(..=k)?.to_vec()
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
    let fit = stroke_subpath(&stroke, tol, false).anchors;
    let (Some(f0), Some(fl)) = (fit.first().copied(), fit.last().copied()) else { return None };
    if fit.len() < 3 {
        return None;
    }
    let inner = fit.get(1..fit.len() - 1)?;
    let mut anchors = sp.anchors.get(..k)?.to_vec();
    anchors.push(Anchor::with_handles(at.p, at.h_in, f0.h_out));
    anchors.extend_from_slice(inner);
    anchors.push(Anchor::with_handles(at.p, fl.h_in, at.h_out));
    anchors.extend_from_slice(sp.anchors.get(k + 1..)?);
    sp.anchors = anchors;
    Some(sp)
}

/// How far, on average, the run of `sp`'s segments from anchor `i` forward to anchor `j` lies from
/// stroke `pts` (sampled along both; the stroke thinned to at most 200 points).
fn run_distance(sp: &SubPath, i: usize, j: usize, pts: &[Point]) -> f64 {
    let (sum, count) = run_distance_sum(sp, i, j, pts);
    if count == 0 { f64::INFINITY } else { sum / count as f64 }
}

/// [`run_distance`]'s sum of the samples' distances and their count.
fn run_distance_sum(sp: &SubPath, i: usize, j: usize, pts: &[Point]) -> (f64, usize) {
    use kurbo::ParamCurve;
    let n = sp.anchors.len();
    if n == 0 || pts.is_empty() {
        return (0.0, 0);
    }
    let step = pts.len().div_ceil(200).max(1);
    let line: Vec<Point> = pts.iter().step_by(step).chain(pts.last()).copied().collect();
    let to_line = |q: Point| {
        line.windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let ab = b - a;
                let t = if ab.hypot2() > 1e-18 { ((q - a).dot(ab) / ab.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
                q.distance(a + ab * t)
            })
            .fold(line.first().map_or(f64::INFINITY, |a| q.distance(*a)), f64::min)
    };
    let segs = (j + n - i) % n;
    let (mut sum, mut count) = (0.0, 0usize);
    for k in 0..segs {
        let c = sp.segment((i + k) % n);
        for m in 1..=8 {
            sum += to_line(c.eval(f64::from(m) / 8.0));
            count += 1;
        }
    }
    (sum, count)
}

/// An open subpath `sp` redrawn by a stroke from anchor `from` to anchor `to` across its open
/// ends: the part between them stays, the ends beyond them go, and the stroke closes the path.
fn redraw_across_gap(sp: &SubPath, from: usize, to: usize, pts: &[Point], tol: Option<f64>, within: f64) -> Option<SubPath> {
    let (lo, hi) = (from.min(to), from.max(to));
    let (a_lo, a_hi) = (*sp.anchors.get(lo)?, *sp.anchors.get(hi)?);
    // The path runs lo → hi, then the stroke from hi back to lo.
    let mut stroke = pts.to_vec();
    if from == lo {
        stroke.reverse();
    }
    bend_ends(&mut stroke, Some(a_hi.p), Some(a_lo.p), within);
    let fit = stroke_subpath(&stroke, tol, false).anchors;
    let (Some(f0), Some(fl)) = (fit.first().copied(), fit.last().copied()) else { return None };
    let mut anchors = sp.anchors.get(lo..=hi)?.to_vec();
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
/// way round that lies nearer the stroke, so the stroke takes the side it was drawn along. The subpath's index and its new shape; None
/// when the stroke doesn't start and end on the same subpath.
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
    let n = sp.anchors.len();
    // On an open subpath the stroke replaces the part it lies along: the run between where it
    // starts and ends, or the ends beyond them and the gap between, which closes the path.
    if !sp.closed {
        let (lo, hi) = (from.min(to), from.max(to));
        let inner = run_distance(&sp, lo, hi, pts);
        let (s1, c1) = run_distance_sum(&sp, 0, lo, pts);
        let (s2, c2) = run_distance_sum(&sp, hi, n - 1, pts);
        // Both ends taken: the stroke only spans the gap.
        let outer = if c1 + c2 == 0 { 0.0 } else { (s1 + s2) / (c1 + c2) as f64 };
        if outer < inner {
            return redraw_across_gap(&sp, from, to, pts, tol, within).map(|sp| (si, sp));
        }
    }
    // The stroke goes from `start` to `end` and replaces the run between them; `kept` runs from
    // `end` round to `start`, then the stroke's inner anchors follow.
    let (start, end, reversed) = if sp.closed {
        // The way round that lies along the stroke: carving across a shape replaces the part the
        // stroke cuts off, drawing round outside it replaces the part it follows.
        if run_distance(&sp, from, to, pts) <= run_distance(&sp, to, from, pts) { (from, to, false) } else { (to, from, true) }
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
        // A stroke that ends back on the path redraws the section between; one that ends away from
        // it takes the path with it (it is where the pen is lifted that decides).
        if let Some((si, sp)) = redraw_section(&pd, &pts, fit_tol, within).or_else(|| redraw_extend(&pd, &pts, fit_tol, within)) {
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
    let samples = samples_param(p, "points", C)?;
    let size = number_or(p, "size", 10.0, C)?;
    if !(size.is_finite() && size > 0.0) {
        return Err(bad(C, "size must be positive"));
    }
    let angle = number_or(p, "angle", 0.0, C)?;
    let roundness = number_or(p, "roundness", 100.0, C)?;
    if !angle.is_finite() || !roundness.is_finite() {
        return Err(bad(C, "angle and roundness must be finite"));
    }
    let angle = angle.clamp(-180.0, 180.0);
    let roundness = roundness.clamp(0.0, 100.0);
    let tip = vectorcraft_tools::draw2::blob_tip(angle, roundness);
    let squash = tip.inverse();
    // Width profiles use distance along the spine, so measure pressure on the same squashed
    // points that the outline reads for a non-round tip.
    let mapped_samples: Vec<(Point, Option<f64>)> =
        if roundness < 100.0 { samples.iter().map(|(pt, pressure)| (squash * *pt, *pressure)).collect() } else { samples.clone() };
    let pressure = size_pressure(p, &mapped_samples, C)?;
    let pts: Vec<Point> = samples.into_iter().map(|(pt, _)| pt).collect();
    let mapped_pts: Vec<Point> = mapped_samples.into_iter().map(|(pt, _)| pt).collect();
    // The widest the brush gets: the area merges look in.
    let reach = pressure.as_ref().map_or(size, |(_, v)| size + v);
    // With `fidelity` (pt), the brush follows the stroke fitted by [`fit_freehand`], as a Pencil
    // stroke is fitted. Without it, or when the fitted stroke has no length (a short stroke back to
    // its start), the brush follows every point.
    let fidelity = match p.get("fidelity") {
        None | Some(Value::Null) => None,
        Some(_) => Some(number_or(p, "fidelity", 0.0, C)?.clamp(vectorcraft_tools::draw2::MIN_FIDELITY, 100.0)),
    };
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
            match fitted {
                Some(r) => r,
                None => pressure_region(&spine(None), size, *variation, prof)?,
            }
        }
        // A click: one dab as wide as the pen pressed.
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
}
