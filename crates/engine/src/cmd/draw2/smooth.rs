use vectorcraft_pathops::addons::pathtools as pt;

use super::common::{dist_to_polyline, ids_json, path_mut, path_of, points_param, poly_bounds, subpath_len, target_paths};
use super::{
    Anchor, BezPath, CubicBez, NodeId, ParamCurve, ParamCurveNearest, PathData, Point, Rect, Result, Session, SimplifyOptions, SubPath, Value, json,
    number_or, po,
};

// ---------- Smooth ----------

/// The spread (points) of the Gaussian the Smooth tool smooths a path along its length with before
/// refitting it, as a share of the tool's reach: a third, so each point is averaged with the stretch
/// of path within the reach on either side of it. VectorCraft's own value.
const SMOOTH_SPREAD: f64 = 1.0 / 3.0;

/// The options the Smooth tool refits with: the path smoothed along its length by a Gaussian of
/// spread `sigma` (none at 0), then refitted within `tol` of that and within twice the distance the
/// smoothing moved it, keeping a corner only where the path turns back on itself.
fn refit_options(tol: f64, sigma: f64) -> SimplifyOptions {
    SimplifyOptions { tolerance: tol, corner_angle_deg: 179.0, straight_lines: false, smooth: sigma, smooth_moved_only: true }
}

/// Smooth and refit the anchors of an open run `a[lo..=hi]` keeping its end points and outer handles.
fn refit_run(anchors: &[Anchor], tol: f64, sigma: f64) -> Vec<Anchor> {
    let piece = PathData::single(SubPath::new(anchors.to_vec(), false));
    let fitted = po::simplify_with(&piece, &refit_options(tol, sigma));
    let Some(mut out) = fitted.subpaths.into_iter().next().map(|s| s.anchors).filter(|a| a.len() >= 2) else { return anchors.to_vec() };
    let (f, l) = (anchors[0], anchors[anchors.len() - 1]);
    let last = out.len() - 1;
    out[0].p = f.p;
    out[0].h_in = f.h_in;
    out[last].p = l.p;
    out[last].h_out = l.h_out;
    for a in &mut out {
        *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
    }
    out
}

/// Pieces of curve that one Smooth drag measures against the drag while it searches the segments
/// under it ([`reach_samples`]), shared equally by the curved segments that come within its reach,
/// each of which gets at least one: enough to search 300,000 pt of curve within the default reach
/// of the drag.
const MAX_REACH_PIECES: usize = 1 << 17;

/// The box around the control points of `s`, which holds all of `s`.
fn control_box(s: CubicBez) -> Rect {
    Rect::from_points(s.p0, s.p1).union_pt(s.p2).union_pt(s.p3)
}

/// The stretches of the drag `pts` (stretch `k` runs from `pts[k]` to the next point; a drag of one
/// point has the point as its only stretch).
fn drag_stretches(pts: &[Point]) -> Vec<usize> {
    (0..pts.len().saturating_sub(1).max(1)).collect()
}

/// The stretches of the drag `pts` among `of` whose bounds come within `radius` of `r` on both
/// axes: every stretch within `radius` of a point in `r`, and perhaps a few more.
fn stretches_near(pts: &[Point], of: &[usize], r: Rect, radius: f64) -> Vec<usize> {
    let r = r.inflate(radius, radius);
    of.iter()
        .copied()
        .filter(|&k| {
            let Some(&a) = pts.get(k) else { return false };
            let b = pts.get(k + 1).copied().unwrap_or(a);
            a.x.min(b.x) <= r.x1 && a.x.max(b.x) >= r.x0 && a.y.min(b.y) <= r.y1 && a.y.max(b.y) >= r.y0
        })
        .collect()
}

/// The distance from `q` to the nearest of the stretches `of` of the drag `pts`.
fn dist_to_stretches(pts: &[Point], of: &[usize], q: Point) -> f64 {
    of.iter()
        .filter_map(|&k| {
            let a = *pts.get(k)?;
            Some(dist_to_polyline(&[a, pts.get(k + 1).copied().unwrap_or(a)], q))
        })
        .fold(f64::INFINITY, f64::min)
}

/// The place on `s` (0 to 1) nearest to the stretches `of` of the drag `pts`, and its distance from
/// them. For each stretch the nearest place is at one of the stretch's ends, at an end of `s`, or
/// where `s` crosses the stretch's line or stops moving toward or away from it.
fn nearest_to_stretches(s: CubicBez, pts: &[Point], of: &[usize]) -> (f64, f64) {
    let (c1, c2, c3) = (3.0 * (s.p1 - s.p0), 3.0 * ((s.p2 - s.p1) - (s.p1 - s.p0)), (s.p3 - s.p0) - 3.0 * (s.p2 - s.p1));
    let mut best = (0.5, f64::INFINITY);
    for &k in of {
        let Some(&a) = pts.get(k) else { continue };
        let b = pts.get(k + 1).copied().unwrap_or(a);
        let mut places: Vec<f64> = vec![0.0, 1.0, s.nearest(a, 1e-9).t, s.nearest(b, 1e-9).t];
        let d = b - a;
        if d.hypot() > 1e-12 {
            let (f0, f1, f2, f3) = (d.cross(s.p0 - a), d.cross(c1), d.cross(c2), d.cross(c3));
            places.extend(kurbo::common::solve_cubic(f0, f1, f2, f3));
            places.extend(kurbo::common::solve_quadratic(f1, 2.0 * f2, 3.0 * f3));
        }
        for u in places.into_iter().filter(|u| (0.0..=1.0).contains(u)) {
            let dist = dist_to_polyline(&[a, b], s.eval(u));
            if dist < best.1 {
                best = (u, dist);
            }
        }
    }
    best
}

/// How many curved segments of `pd` may come within `radius` of the drag `pts`.
fn segments_in_reach(pd: &PathData, pts: &[Point], radius: f64) -> usize {
    let all = drag_stretches(pts);
    pd.subpaths
        .iter()
        .map(|sp| {
            (0..sp.segment_count())
                .filter(|&i| {
                    !vectorcraft_tools::draw2::segment_is_straight(sp, i) && !stretches_near(pts, &all, control_box(sp.segment(i)), radius).is_empty()
                })
                .count()
        })
        .sum()
}

/// Samples along the part of `c` from `t0` to `t1`, in order: (t, whether the point there is within
/// `radius` of the stretches `of` of the drag `pts`). A piece no stretch comes near gives one sample
/// out of reach; the others are halved until their control polygon is no longer than half the
/// reach, or until `budget` pieces have been measured, and then give one sample: in reach at the
/// piece's middle or at its place nearest the drag when either is in reach, otherwise out of reach
/// at its middle. Each piece measures only the stretches near
/// the piece it was cut from, so a long drag along a long curve costs about one pass over the drag
/// per halving, plus the stretches near each piece.
fn reach_samples(c: CubicBez, (t0, t1): (f64, f64), (pts, of): (&[Point], &[usize]), radius: f64, budget: &mut usize, out: &mut Vec<(f64, bool)>) {
    *budget = budget.saturating_sub(1);
    let s = c.subsegment(t0..t1);
    let mid = 0.5 * (t0 + t1);
    let near = stretches_near(pts, of, control_box(s), radius);
    if near.is_empty() {
        out.push((mid, false));
        return;
    }
    let len = s.p0.distance(s.p1) + s.p1.distance(s.p2) + s.p2.distance(s.p3);
    if len <= (radius / 2.0).max(1e-3) || *budget == 0 || mid <= t0 || mid >= t1 {
        // The piece's middle, or the place on it nearest the drag when the middle is out of reach.
        let sample = if dist_to_stretches(pts, &near, c.eval(mid)) <= radius {
            (mid, true)
        } else {
            let (u, dist) = nearest_to_stretches(s, pts, &near);
            if dist <= radius { (t0 + u * (t1 - t0), true) } else { (mid, false) }
        };
        out.push(sample);
        return;
    }
    reach_samples(c, (t0, mid), (pts, &near), radius, budget, out);
    reach_samples(c, (mid, t1), (pts, &near), radius, budget, out);
}

/// `sp` cut where the reach `radius` of the drag `pts` crosses its curved segments, and whether each
/// anchor of the result is in reach. An anchor in reach is added in the middle of each stretch of a
/// segment in reach that does not run on from one of the segment's anchors in reach, and an anchor
/// out of reach in the middle of each stretch out of reach between two stretches in reach, so that a
/// stretch in reach is smoothed even when the segment's anchors are not, and the stretches between
/// stay as they are. A segment is searched no more than half the reach apart where it comes within
/// the reach of the drag ([`reach_samples`]), measuring up to `share` pieces of each segment.
fn mark_reached(sp: &SubPath, pts: &[Point], radius: f64, share: usize) -> (SubPath, Vec<bool>) {
    let near = |q: Point| dist_to_polyline(pts, q) <= radius;
    let n = sp.anchors.len();
    let marked: Vec<bool> = sp.anchors.iter().map(|a| near(a.p)).collect();
    let mut places = vec![];
    let all = drag_stretches(pts);
    for i in 0..sp.segment_count() {
        if vectorcraft_tools::draw2::segment_is_straight(sp, i) {
            continue;
        }
        let c = sp.segment(i);
        let (mut samples, mut budget) = (vec![], share);
        reach_samples(c, (0.0, 1.0), (pts, &all), radius, &mut budget, &mut samples);
        // Stretches of samples alike, in order: (in reach, first, last).
        let mut runs: Vec<(bool, usize, usize)> = vec![];
        for (k, s) in samples.iter().enumerate() {
            match runs.last_mut() {
                Some(r) if r.0 == s.1 => r.2 = k,
                _ => runs.push((s.1, k, k)),
            }
        }
        let last = samples.len().saturating_sub(1);
        let (start, end) = (marked[i], marked[(i + 1) % n]);
        for (j, &(reached, first, to)) in runs.iter().enumerate() {
            let mid = samples[(first + to) / 2].0;
            let joined = (first == 0 && start) || (to == last && end);
            if reached && !joined {
                places.push((i, mid));
            }
            // A gap with reach on both sides: the segment's anchor or a stretch on each side.
            let before = if j == 0 { start } else { runs[j - 1].0 };
            let after = runs.get(j + 1).map_or(end, |r| r.0);
            if !reached && before && after {
                places.push((i, mid));
            }
        }
    }
    // A place on one of the segment's ends would name that anchor rather than add one.
    places.retain(|(_, t)| (1e-5..=1.0 - 1e-5).contains(t));
    if places.is_empty() {
        return (sp.clone(), marked);
    }
    let (out, added) = pt::insert_at(sp, &places);
    // The anchors already there keep their marks. A new anchor split off at a sample in reach can
    // land a rounding error farther away than the sample.
    let mut kept = marked.into_iter();
    let marked = out
        .anchors
        .iter()
        .enumerate()
        .map(|(j, a)| if added.contains(&j) { dist_to_polyline(pts, a.p) <= radius + 1e-9 * radius.max(1.0) } else { kept.next().unwrap_or(false) })
        .collect();
    (out, marked)
}

fn smooth_subpath(sp: &SubPath, pts: &[Point], radius: f64, share: usize, tol: f64, sigma: f64) -> Option<SubPath> {
    if sp.anchors.len() < 2 {
        return None;
    }
    let near = &|q: Point| dist_to_polyline(pts, q) <= radius;
    // A drag along a segment smooths it even when its anchors are out of reach.
    let (sp, marked) = mark_reached(sp, pts, radius, share);
    let sp = &sp;
    if !marked.iter().any(|m| *m) {
        return None;
    }
    if sp.closed {
        let Some(k) = marked.iter().position(|m| !m) else {
            // Everything is in range: smooth the whole closed path, across its start too.
            let f = po::simplify_with(&PathData::single(sp.clone()), &refit_options(tol, sigma));
            return f.subpaths.into_iter().next();
        };
        // Rotate so an untouched anchor comes first, duplicate it at the end and treat as open.
        let mut open = sp.clone();
        open.anchors.rotate_left(k);
        let mut m = marked.clone();
        m.rotate_left(k);
        open.anchors.push(open.anchors[0]);
        m.push(false);
        open.closed = false;
        let mut res = smooth_open(&open.anchors, &m, near, tol, sigma);
        let l = res.pop()?;
        if let Some(r0) = res.first_mut() {
            r0.h_in = l.h_in;
        }
        return Some(SubPath::new(res, true));
    }
    Some(SubPath::new(smooth_open(&sp.anchors, &marked, near, tol, sigma), false))
}

/// The segment `c`, which ends in the drag's reach (`near`) and starts outside it, split where it
/// enters the reach: the part outside, then the part in reach. `None` when no more than a sliver
/// is outside.
fn split_at_reach(c: CubicBez, near: &dyn Fn(Point) -> bool) -> Option<(CubicBez, CubicBez)> {
    const STEPS: u32 = 32;
    let step = 1.0 / f64::from(STEPS);
    // Back from the end in reach to the first sample outside, then halving to the crossing.
    let outside = (1..=STEPS).map(|i| 1.0 - f64::from(i) * step).find(|t| !near(c.eval(*t)))?;
    let (mut lo, mut hi) = (outside, outside + step);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if near(c.eval(mid)) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    // A straight segment's parts stay straight, their handles on their ends.
    let straight = c.p1 == c.p0 && c.p2 == c.p3;
    let part = |range| {
        let s = c.subsegment(range);
        if straight { CubicBez::new(s.p0, s.p0, s.p3, s.p3) } else { s }
    };
    (hi > 0.02).then(|| (part(0.0..hi), part(hi..1.0)))
}

/// `anchors` with each run of `marked` ones smoothed by a Gaussian of spread `sigma` and refitted
/// within `tol`, together with the stretch of the segments leading into the run that is in the
/// drag's reach (`near`): a short drag smooths what is under it, and the rest of a long segment next
/// to it stays as it was (#1512).
fn smooth_open(anchors: &[Anchor], marked: &[bool], near: &dyn Fn(Point) -> bool, tol: f64, sigma: f64) -> Vec<Anchor> {
    let n = anchors.len();
    let mut runs = vec![];
    let mut i = 0;
    while i < n {
        if marked[i] {
            let a = i;
            while i + 1 < n && marked[i + 1] {
                i += 1;
            }
            runs.push((a, i));
        }
        i += 1;
    }
    let mut out = anchors.to_vec();
    for (a, b) in runs.into_iter().rev() {
        let lo = a.saturating_sub(1);
        let hi = (b + 1).min(n - 1);
        if hi == lo {
            continue;
        }
        let mut piece = out[lo..=hi].to_vec();
        // The anchors outside the reach, kept with the part of their segment that is outside.
        let (mut head, mut tail) = (None, None);
        if hi > b
            && let [.., inner, outer] = piece.as_mut_slice()
            && let Some((far, reach)) = split_at_reach(CubicBez::new(outer.p, outer.h_in, inner.h_out, inner.p), near)
        {
            inner.h_out = reach.p2;
            tail = Some(Anchor { h_in: far.p1, ..*outer });
            *outer = Anchor::with_handles(far.p3, reach.p1, far.p2);
        }
        if lo < a
            && let [outer, inner, ..] = piece.as_mut_slice()
            && let Some((far, reach)) = split_at_reach(CubicBez::new(outer.p, outer.h_out, inner.h_in, inner.p), near)
        {
            inner.h_in = reach.p2;
            head = Some(Anchor { h_out: far.p1, ..*outer });
            *outer = Anchor::with_handles(far.p3, far.p2, reach.p1);
        }
        let new = refit_run(&piece, tol, sigma);
        out.splice(lo..=hi, head.into_iter().chain(new).chain(tail));
    }
    out
}

pub(super) fn smooth_region(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.smoothRegion";
    let pts = points_param(p, "points", C)?;
    let radius = number_or(p, "radius", 12.0, C)?.max(0.0);
    let tol = number_or(p, "fidelity", 2.5, C)?.clamp(vectorcraft_tools::draw2::SMOOTH_MIN_FIDELITY, 100.0);
    let sigma = radius * SMOOTH_SPREAD;
    let st = s.doc()?;
    let mut plan = vec![];
    let (mut before, mut after) = (0, 0);
    let mut paths = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, radius)) {
        paths.push((id, path_of(&st.doc, id)?.0));
    }
    // Each curved segment in reach gets the same share of the search, whatever order they come in.
    let in_reach: usize = paths.iter().map(|(_, pd)| segments_in_reach(pd, &pts, radius)).sum();
    let share = (MAX_REACH_PIECES / in_reach.max(1)).max(1);
    for (id, pd) in paths {
        let mut changed = false;
        let subs: Vec<SubPath> = pd
            .subpaths
            .iter()
            .map(|sp| match smooth_subpath(sp, &pts, radius, share, tol, sigma) {
                Some(n) if n != *sp && n.anchors.len() >= 2 => {
                    changed = true;
                    n
                }
                _ => sp.clone(),
            })
            .collect();
        if changed {
            let np = PathData::new(subs);
            before += pd.anchor_count();
            after += np.anchor_count();
            plan.push((id, np));
        }
    }
    if plan.is_empty() {
        return Ok(json!({ "ids": [], "before": 0, "after": 0 }));
    }
    let ids: Vec<NodeId> = plan.iter().map(|(id, _)| *id).collect();
    s.edit("Smooth", |d, sel| {
        for (id, np) in plan {
            *path_mut(d, id)? = np;
            // The smoothed path has new anchors, so the old anchor and segment indices are
            // dropped and the path stays selected whole.
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
        }
        Ok(())
    })?;
    let mut v = ids_json(&ids);
    v["before"] = json!(before);
    v["after"] = json!(after);
    Ok(v)
}

// ---------- Join tool ----------

/// Most open subpaths the Join tool compares pairwise for crossings before it only joins ends.
const MAX_TRIM_SUBPATHS: usize = 64;

/// One end of an open subpath that runs on past where it crosses another: the part kept, which
/// ends at the crossing, and the part past it.
struct Overhang {
    kept: SubPath,
    cut: SubPath,
}

impl Overhang {
    fn at(&self) -> Option<Point> {
        self.kept.anchors.last().map(|a| a.p)
    }
}

/// The overhangs of `sp` at its first and last crossing (`places`: subpath, segment, t, in order).
/// A crossing on an end anchor leaves no overhang there.
fn overhangs(sp: &SubPath, places: &[(usize, usize, f64)]) -> Vec<Overhang> {
    let mut out = vec![];
    for (place, at_start) in [(places.first(), true), (places.last(), false)] {
        let Some(&(_, seg, t)) = place else { continue };
        let (split, marks) = pt::insert_at(sp, &[(seg, t)]);
        let Ok([first, second]) = <[SubPath; 2]>::try_from(split.cut_at(&marks)) else { continue };
        let (mut kept, cut) = if at_start { (second, first) } else { (first, second) };
        if at_start {
            kept.reverse();
        }
        out.push(Overhang { kept, cut });
    }
    out
}

/// Does the scrub `pts` (`scrub` as a path) cross `cut` or pass within `tol` of it?
fn scrubbed(cut: &SubPath, pts: &[Point], scrub: &PathData, tol: f64) -> bool {
    let pd = PathData::single(cut.clone());
    pt::crossings(&[&pd, scrub], false).is_some_and(|c| c.first().is_some_and(|v| !v.is_empty()))
        || cut.anchors.iter().any(|a| dist_to_polyline(pts, a.p) <= tol)
        || pts.iter().any(|q| pd.nearest(*q).is_some_and(|n| n.4 <= tol))
}

/// Two crossing open subpaths, each `(path, subpath)`, trimmed and joined at the crossing.
struct TrimJoin {
    a: (NodeId, usize),
    b: (NodeId, usize),
    joined: SubPath,
}

/// #5844: two open subpaths among `ids` that cross, where the scrub runs over the part of each
/// that overhangs the crossing, trimmed at the crossing and joined there with a corner. When
/// several pairs or ends qualify, the one trimming the least wins.
fn trim_join(d: &vectorcraft_doc::Document, ids: &[NodeId], pts: &[Point], tol: f64) -> Result<Option<TrimJoin>> {
    let mut open = vec![];
    for &id in ids {
        let (pd, _) = path_of(d, id)?;
        open.extend(pd.subpaths.into_iter().enumerate().filter(|(_, sp)| !sp.closed && sp.anchors.len() >= 2).map(|(si, sp)| (id, si, sp)));
    }
    if open.len() > MAX_TRIM_SUBPATHS {
        return Ok(None);
    }
    let scrub = PathData::single(SubPath::polyline(pts, false));
    let mut best: Option<(f64, TrimJoin)> = None;
    for (i, (ida, sa, a)) in open.iter().enumerate() {
        for (idb, sb, b) in open.iter().skip(i + 1) {
            let (pa, pb) = (PathData::single(a.clone()), PathData::single(b.clone()));
            let Some(places) = pt::crossings(&[&pa, &pb], false) else { continue };
            let (Some(on_a), Some(on_b)) = (places.first(), places.get(1)) else { continue };
            let (oa, ob) = (overhangs(a, on_a), overhangs(b, on_b));
            for x in oa.iter().filter(|o| scrubbed(&o.cut, pts, &scrub, tol)) {
                for y in ob.iter().filter(|o| scrubbed(&o.cut, pts, &scrub, tol)) {
                    let (Some(xa), Some(ya)) = (x.at(), y.at()) else { continue };
                    let trimmed = subpath_len(&x.cut) + subpath_len(&y.cut);
                    if xa.distance(ya) > 1e-2 || best.as_ref().is_some_and(|b| b.0 <= trimmed) {
                        continue;
                    }
                    // `x` ends at the crossing; `y`, reversed, starts there: one corner anchor.
                    let mut joined = x.kept.clone();
                    let mut rest = y.kept.clone();
                    rest.reverse();
                    let (Some(last), Some(first)) = (joined.anchors.last_mut(), rest.anchors.first()) else { continue };
                    *last =
                        Anchor { p: last.p, h_in: last.h_in, h_out: first.h_out + (last.p - first.p), kind: vectorcraft_geom::AnchorKind::Corner };
                    joined.anchors.extend(rest.anchors.into_iter().skip(1));
                    best = Some((trimmed, TrimJoin { a: (*ida, *sa), b: (*idb, *sb), joined }));
                }
            }
        }
    }
    Ok(best.map(|(_, t)| t))
}

pub(super) fn join_scrub(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.joinScrub";
    let pts = points_param(p, "points", C)?;
    let tol = number_or(p, "tolerance", 12.0, C)?.max(0.0);
    let st = s.doc()?;
    let targets = target_paths(&st.doc, &st.selection, poly_bounds(&pts, tol));
    // Paths that cross with their ends scrubbed are trimmed there and joined (#5844).
    if let Some(TrimJoin { a: (ida, sa), b: (idb, sb), joined }) = trim_join(&st.doc, &targets, &pts, tol)? {
        s.edit("Join", |d, sel| {
            let missing = || crate::EngineError::Other("Join: a subpath went missing".into());
            *path_mut(d, ida)?.subpaths.get_mut(sa).ok_or_else(missing)? = joined;
            let pbm = path_mut(d, idb)?;
            if sb >= pbm.subpaths.len() {
                return Err(missing());
            }
            pbm.subpaths.remove(sb);
            if ida != idb && pbm.is_empty() {
                d.remove(idb)?;
            }
            sel.set([ida]);
            Ok(())
        })?;
        return Ok(json!({ "id": ida.0 }));
    }
    // Open endpoints near the scrub, ordered along it: (order, id, subpath, at_start).
    let poly = {
        let mut bp = BezPath::new();
        bp.move_to(pts[0]);
        for q in &pts[1..] {
            bp.line_to(*q);
        }
        bp
    };
    let order = |q: Point| -> f64 {
        poly.segments()
            .enumerate()
            .map(|(i, sg)| (i as f64 + sg.nearest(q, 1e-6).t, sg.nearest(q, 1e-6).distance_sq))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0.0, |x| x.0)
    };
    let (first_pt, last_pt) = (pts.first().copied(), pts.last().copied());
    let mut ends: Vec<(f64, NodeId, usize, bool, Point)> = vec![];
    for id in targets {
        let (pd, _) = path_of(&st.doc, id)?;
        for (si, sp) in pd.subpaths.iter().enumerate() {
            if sp.closed || sp.anchors.len() < 2 {
                continue;
            }
            for (start, a) in [(true, sp.anchors[0].p), (false, sp.anchors[sp.anchors.len() - 1].p)] {
                let on_scrub = dist_to_polyline(&pts, a) <= tol;
                let near_start = first_pt.is_some_and(|p| a.distance(p) <= tol);
                let near_end = last_pt.is_some_and(|p| a.distance(p) <= tol);
                if !on_scrub && !near_start && !near_end {
                    continue;
                }
                let ord = if on_scrub {
                    order(a)
                } else if near_start && (!near_end || first_pt.is_some_and(|p| a.distance(p) <= a.distance(last_pt.unwrap_or(p)))) {
                    -1.0
                } else {
                    1e30
                };
                ends.push((ord, id, si, start, a));
            }
        }
    }
    ends.sort_by(|a, b| a.0.total_cmp(&b.0));
    if ends.len() < 2 {
        return Ok(json!({ "id": null }));
    }
    let (_, ida, sa, starta, pa) = ends[0];
    let (_, idb, sb, startb, pb) = ends[1];
    let mid = pa.midpoint(pb);
    let merge = pa.distance(pb) <= tol;
    s.edit("Join", |d, sel| {
        if ida == idb && sa == sb {
            let path = path_mut(d, ida)?;
            let sp = &mut path.subpaths[sa];
            if merge
                && sp.anchors.len() > 2
                && let Some(l) = sp.anchors.pop()
            {
                let d0 = mid - sp.anchors[0].p;
                sp.anchors[0].translate(d0);
                sp.anchors[0].h_in = l.h_in + (mid - l.p);
            }
            sp.closed = true;
            sel.set([ida]);
            return Ok(());
        }
        let (pdb, _) = path_of(d, idb)?;
        let mut x = path_of(d, ida)?.0.subpaths[sa].clone();
        let mut y = pdb.subpaths[sb].clone();
        if starta {
            x.reverse();
        }
        if !startb {
            y.reverse();
        }
        if merge
            && !y.anchors.is_empty()
            && let Some(last) = x.anchors.last_mut()
        {
            let yf = y.anchors.remove(0);
            let lp = last.p;
            last.translate(mid - lp);
            last.h_out = yf.h_out + (mid - yf.p);
            *last = Anchor::with_handles(last.p, last.h_in, last.h_out);
        }
        x.anchors.extend(y.anchors);
        if ida == idb {
            let path = path_mut(d, ida)?;
            path.subpaths[sa] = x;
            path.subpaths.remove(sb);
        } else {
            path_mut(d, ida)?.subpaths[sa] = x;
            let pbm = path_mut(d, idb)?;
            pbm.subpaths.remove(sb);
            if pbm.is_empty() {
                d.remove(idb)?;
            }
        }
        sel.set([ida]);
        Ok(())
    })?;
    Ok(json!({ "id": ida.0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Searching a curve 1,000,000 pt long under a drag of 1,000 points along it measures no more
    /// pieces than the budget it is given, and a search with none left gives one sample.
    #[test]
    fn searching_a_segment_under_a_drag_stays_within_its_budget() {
        let c = CubicBez::new(Point::new(0.0, 0.0), Point::new(300000.0, 0.0), Point::new(700000.0, 0.0), Point::new(1000000.0, 0.0));
        let pts: Vec<Point> = (0..1000).map(|i| Point::new(f64::from(i) * 1000.0, 0.0)).collect();
        let all = drag_stretches(&pts);
        let mut budget = MAX_REACH_PIECES;
        let (mut first, mut second) = (vec![], vec![]);
        reach_samples(c, (0.0, 1.0), (&pts, &all), 12.0, &mut budget, &mut first);
        reach_samples(c, (0.0, 1.0), (&pts, &all), 12.0, &mut budget, &mut second);
        assert!(first.len() <= MAX_REACH_PIECES, "{} samples", first.len());
        assert_eq!((budget, second.len()), (0, 1), "the budget is spent, so the next search gives one sample");
    }

    /// A small piece of curve is measured only against the few stretches of a drag of 100,000
    /// points that come near it, and a drag of one point has that point as its stretch.
    #[test]
    fn a_small_piece_is_measured_against_the_stretches_near_it() {
        let pts: Vec<Point> = (0..100_000).map(|i| Point::new(f64::from(i) * 5.0, 0.0)).collect();
        let all = drag_stretches(&pts);
        let near = stretches_near(&pts, &all, Rect::new(1000.0, 3.0, 1004.0, 6.0), 12.0);
        assert_eq!(near, (197..=203).collect::<Vec<_>>());
        assert!((dist_to_stretches(&pts, &near, Point::new(1002.0, 5.0)) - 5.0).abs() < 1e-12);
        let one = [Point::new(10.0, 10.0)];
        assert_eq!(drag_stretches(&one), vec![0]);
        assert!((dist_to_stretches(&one, &[0], Point::new(13.0, 14.0)) - 5.0).abs() < 1e-12);
    }
}
