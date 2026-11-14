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

/// The degrees by which a path must turn at an anchor, and over the tool's reach and twice its reach
/// on either side of it, for the Smooth tool to keep the anchor as a corner ([`smooth_corners`]): a
/// turn of more than 57.5°, an angle inside the corner of less than 122.5°. VectorCraft's own value,
/// the same on every drag and at any Fidelity.
const SMOOTH_CORNER_DEG: f64 = 57.5;

/// Most pieces a curved segment is cut into evenly, and most more pieces near each of its ends, to
/// measure lengths along it in [`smooth_corners`].
const CORNER_PIECES: usize = 64;

/// The control polygon's length of `c`, which is at least its length.
fn polygon_len(c: CubicBez) -> f64 {
    c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3)
}

/// The places (0 to 1, in order, without 0 and with 1) on the curved segment `c` that
/// [`smooth_corners`] measures it at: up to [`CORNER_PIECES`] evenly spaced, and near each end, over
/// a stretch at least three times `reach` long where the segment is that long, up to as many more
/// that are no more than `reach` / 8 apart along the control polygon. The turns over `reach` and
/// twice `reach` from an anchor are then measured along the curve however long the segment is.
fn corner_places(c: CubicBez, reach: f64) -> Vec<f64> {
    let piece = reach / 8.0;
    let pieces = |len: f64| if piece > 0.0 { (len / piece).ceil().clamp(1.0, CORNER_PIECES as f64) as usize } else { 1 };
    let k = pieces(polygon_len(c)).max(4);
    let mut places: Vec<f64> = (1..=k).map(|j| j as f64 / k as f64).collect();
    if piece > 0.0 {
        for from_start in [true, false] {
            // The stretch from the end: halved until its control polygon is at most six reaches long.
            let mut t = 1.0 / k as f64;
            let part = |t: f64| if from_start { c.subsegment(0.0..t) } else { c.subsegment(1.0 - t..1.0) };
            for _ in 0..60 {
                if polygon_len(part(t)) <= 6.0 * reach {
                    break;
                }
                t /= 2.0;
            }
            let m = pieces(polygon_len(part(t)));
            places.extend((1..m).map(|j| t * j as f64 / m as f64).map(|u| if from_start { u } else { 1.0 - u }));
        }
    }
    places.retain(|u| *u > 0.0 && *u <= 1.0);
    places.sort_by(f64::total_cmp);
    places.dedup();
    places
}

/// The degrees between the directions `u` and `v`, `None` when either has no length or is not a
/// number.
fn turn_between(u: kurbo::Vec2, v: kurbo::Vec2) -> Option<f64> {
    let (lu, lv) = (u.hypot(), v.hypot());
    if !(lu.is_finite() && lv.is_finite() && lu > 1e-12 && lv > 1e-12) {
        return None;
    }
    let deg = u.cross(v).atan2(u.dot(v)).abs().to_degrees();
    deg.is_finite().then_some(deg)
}

/// Which anchors of `anchors` (a closed path when `closed`) the Smooth tool keeps as corners: those
/// where the path turns by more than [`SMOOTH_CORNER_DEG`] at the anchor itself, between the
/// direction it arrives along and the one it leaves along, and also over `reach` and over twice
/// `reach` on either side of it, between the direction from the point that far back along the path
/// to the anchor and the direction from the anchor to the point that far ahead. The points are taken
/// no farther than the ends of an open path, and no more than an eighth of the way round a closed
/// one. A jag or a bump smaller than the reach turns sharply at its anchor but not over twice the
/// reach, and an anchor in the middle of a tight curve turns over the reach but not at the anchor, so
/// neither is kept. An open path's first and last anchors are never corners.
fn smooth_corners(anchors: &[Anchor], closed: bool, reach: f64) -> Vec<bool> {
    let n = anchors.len();
    let mut corners = vec![false; n];
    if n < 2 || (n < 3 && !closed) {
        return corners;
    }
    // The distances on either side the turn is measured over: one reach and two, no more than an
    // eighth of the way round a closed path. Over that much of a circle the path turns by 45°, so a
    // loop shorter than the reach is not all corners, while a square's or a triangle's corners still are.
    let mut spans = [reach, 2.0 * reach];
    if closed {
        let mut total = 0.0;
        for i in 0..n {
            total += walk(anchors, closed, (i, true), [f64::INFINITY; 2], reach, 1).1;
        }
        if !(total.is_finite() && total > 0.0) {
            return corners;
        }
        spans = spans.map(|r| r.min(total / 8.0));
    }
    for (i, flag) in corners.iter_mut().enumerate() {
        if !closed && (i == 0 || i == n - 1) {
            continue;
        }
        let (prev, a, next) = (anchors[(i + n - 1) % n], anchors[i], anchors[(i + 1) % n]);
        let arrive = [a.p - a.h_in, a.p - prev.h_out, a.p - prev.p].into_iter().find(|d| d.hypot() > 1e-12).unwrap_or_default();
        let leave = [a.h_out - a.p, next.h_in - a.p, next.p - a.p].into_iter().find(|d| d.hypot() > 1e-12).unwrap_or_default();
        if !turn_between(arrive, leave).is_some_and(|t| t > SMOOTH_CORNER_DEG) {
            continue;
        }
        let [Some(back_near), Some(back_far)] = walk(anchors, closed, (i, false), spans, reach, MAX_CORNER_WALK).0 else { continue };
        let [Some(ahead_near), Some(ahead_far)] = walk(anchors, closed, (i, true), spans, reach, MAX_CORNER_WALK).0 else { continue };
        *flag = [(back_near, ahead_near, spans[0]), (back_far, ahead_far, spans[1])]
            .into_iter()
            .all(|(back, ahead, r)| r <= 0.0 || turn_between(a.p - back, ahead - a.p).is_some_and(|t| t > SMOOTH_CORNER_DEG));
    }
    corners
}

/// Most segments [`walk`] passes from an anchor before it stops, which bounds the work for each
/// anchor on a path of very many very short segments.
const MAX_CORNER_WALK: usize = 256;

/// A walk along the path `anchors` from anchor `from.0`, ahead when `from.1` and back otherwise,
/// past no more than `segments` segments, measured along the lines between each curved segment's
/// points at [`corner_places`]. It gives the points at the distances `to` (in order, the nearer
/// first), an open path's end for one beyond the end, and `None` for one it did not reach within
/// `segments` segments, and the length it walked. Both points are `None` when it meets a length that
/// is not a number or is too large to add, and the length is then infinite.
fn walk(anchors: &[Anchor], closed: bool, from: (usize, bool), to: [f64; 2], reach: f64, segments: usize) -> ([Option<Point>; 2], f64) {
    let (n, forward) = (anchors.len(), from.1);
    let Some(mut here) = anchors.get(from.0).map(|a| a.p) else { return ([None; 2], 0.0) };
    let (mut at, mut gone) = (from.0, 0.0);
    let mut found: [Option<Point>; 2] = [None; 2];
    let mut at_end = false;
    'segments: for _ in 0..segments {
        let next = match (forward, closed) {
            (true, _) if at + 1 < n => at + 1,
            (true, true) => 0,
            (false, _) if at > 0 => at - 1,
            (false, true) => n - 1,
            _ => {
                at_end = true;
                break;
            }
        };
        let (a, b) = (anchors[at], anchors[next]);
        let c = if forward { CubicBez::new(a.p, a.h_out, b.h_in, b.p) } else { CubicBez::new(a.p, a.h_in, b.h_out, b.p) };
        let places = if c.p1 == c.p0 && c.p2 == c.p3 { vec![1.0] } else { corner_places(c, reach) };
        for u in places {
            let q = if u >= 1.0 { c.p3 } else { c.eval(u) };
            let d = here.distance(q);
            if !(d.is_finite() && (gone + d).is_finite()) {
                return ([None; 2], f64::INFINITY);
            }
            for (slot, &dist) in found.iter_mut().zip(&to) {
                if slot.is_none() && gone + d >= dist {
                    let t = if d > 0.0 { ((dist - gone) / d).clamp(0.0, 1.0) } else { 1.0 };
                    *slot = Some(here.lerp(q, t));
                }
            }
            gone += d;
            here = q;
            if found.iter().all(Option::is_some) {
                break 'segments;
            }
        }
        at = next;
    }
    // An open path that ends after the last segment walked ends there too.
    if !closed && at == if forward { n - 1 } else { 0 } {
        at_end = true;
    }
    (found.map(|p| p.or(at_end.then_some(here))), gone)
}

/// Whether `a` and `b` are the same anchors, to within a rounding error.
fn same_anchors(a: &[Anchor], b: &[Anchor]) -> bool {
    let near = |p: Point, q: Point| p.distance(q) <= 1e-9;
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| near(x.p, y.p) && near(x.h_in, y.h_in) && near(x.h_out, y.h_out))
}

/// [`refit_run`] of the open run `anchors` cut at its anchors marked in `corners`: each part between
/// two corners is smoothed and refitted on its own, so each corner keeps its place and the direction
/// the path arrives and leaves along. `free` applies to the run's first and last anchors only.
fn refit_split(anchors: &[Anchor], corners: &[bool], tol: f64, sigma: f64, free: [bool; 2]) -> Vec<Anchor> {
    let last = anchors.len().saturating_sub(1);
    let mut cuts: Vec<usize> = (1..last).filter(|&i| corners.get(i).copied().unwrap_or(false)).collect();
    if cuts.is_empty() {
        return refit_run(anchors, tol, sigma, free);
    }
    cuts.insert(0, 0);
    cuts.push(last);
    let mut out: Vec<Anchor> = Vec::with_capacity(anchors.len());
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let part = refit_run(&anchors[a..=b], tol, sigma, [free[0] && a == 0, free[1] && b == last]);
        match out.pop() {
            // The corner ends one part and starts the next: it keeps its place, and takes the handle
            // in from the part before and the handle out from the part after.
            Some(end) => {
                let mut rest = part.into_iter();
                if let Some(start) = rest.next() {
                    out.push(Anchor::with_handles(anchors[a].p, end.h_in, start.h_out));
                }
                out.extend(rest);
            }
            None => out.extend(part),
        }
    }
    out
}

/// The options the Smooth tool refits with: the path smoothed along its length by a Gaussian of
/// spread `sigma` (none at 0), then refitted within `tol` of that and within twice the distance the
/// smoothing moved it. The tool cuts the path at its corners first ([`smooth_corners`]), so within a
/// part the refit keeps a corner only where the path turns back on itself. `free` says whether
/// the start and the end of the stretch refitted are ends of an open path, whose direction then
/// follows the smoothed path within the reach rather than the first or last segment.
fn refit_options(tol: f64, sigma: f64, free: [bool; 2]) -> SimplifyOptions {
    SimplifyOptions {
        tolerance: tol,
        corner_angle_deg: 179.0,
        straight_lines: false,
        smooth: sigma,
        smooth_moved_only: true,
        free_start: free[0],
        free_end: free[1],
    }
}

/// Smooth and refit the anchors of an open run `a[lo..=hi]` keeping its end points and outer handles.
/// `free` says whether its first and last anchors are ends of an open path: the curve leaving or
/// arriving at such an end can turn.
fn refit_run(anchors: &[Anchor], tol: f64, sigma: f64, free: [bool; 2]) -> Vec<Anchor> {
    let piece = PathData::single(SubPath::new(anchors.to_vec(), false));
    let fitted = po::simplify_with(&piece, &refit_options(tol, sigma, free));
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
            let corners = smooth_corners(&sp.anchors, true, radius);
            let Some(c) = corners.iter().position(|c| *c) else {
                // Everything is in range and nothing is a corner: smooth the whole closed path,
                // across its start too.
                let f = po::simplify_with(&PathData::single(sp.clone()), &refit_options(tol, sigma, [false; 2]));
                return f.subpaths.into_iter().next();
            };
            // Cut the path open at a corner, which then ends the last part as well as starting the first.
            let mut open = sp.anchors.clone();
            open.rotate_left(c);
            let mut cut = corners;
            cut.rotate_left(c);
            open.push(open[0]);
            cut.push(true);
            let mut res = refit_split(&open, &cut, tol, sigma, [false; 2]);
            if same_anchors(&res, &open) {
                return None;
            }
            let l = res.pop()?;
            if let Some(r0) = res.first_mut() {
                *r0 = Anchor::with_handles(r0.p, l.h_in, r0.h_out);
            }
            // The path starts where it did when the refit keeps an anchor there.
            if let Some(j) = res.iter().position(|a| a.p == sp.anchors[0].p) {
                res.rotate_left(j);
            }
            return Some(SubPath::new(res, true));
        };
        // Rotate so an untouched anchor comes first, duplicate it at the end and treat as open.
        let mut open = sp.clone();
        open.anchors.rotate_left(k);
        let mut m = marked.clone();
        m.rotate_left(k);
        open.anchors.push(open.anchors[0]);
        m.push(false);
        open.closed = false;
        let mut res = smooth_open(&open.anchors, &m, near, tol, sigma, radius, false);
        if same_anchors(&res, &open.anchors) {
            return None;
        }
        let l = res.pop()?;
        if let Some(r0) = res.first_mut() {
            r0.h_in = l.h_in;
        }
        return Some(SubPath::new(res, true));
    }
    let res = smooth_open(&sp.anchors, &marked, near, tol, sigma, radius, true);
    (!same_anchors(&res, &sp.anchors)).then(|| SubPath::new(res, false))
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
/// to it stays as it was (#1512). With `open`, `anchors` are an open path, and the curve at its first
/// or last anchor turns with the smoothing when a run's refit reaches that anchor; a run's other ends,
/// and those of the open run a closed path is cut into, keep their direction. Each run keeps its
/// corners ([`smooth_corners`] within `radius`, the tool's reach, measured within the run), and a run
/// whose refit changes nothing stays as it was.
fn smooth_open(anchors: &[Anchor], marked: &[bool], near: &dyn Fn(Point) -> bool, tol: f64, sigma: f64, radius: f64, open: bool) -> Vec<Anchor> {
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
        let free = [open && lo == 0 && head.is_none(), open && hi == n - 1 && tail.is_none()];
        let corners = smooth_corners(&piece, false, radius);
        let new = refit_split(&piece, &corners, tol, sigma, free);
        if same_anchors(&new, &piece) {
            continue;
        }
        out.splice(lo..=hi, head.into_iter().chain(new).chain(tail));
    }
    out
}

pub(super) fn smooth_region(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.smoothRegion";
    let pts = points_param(p, "points", C)?;
    let radius = number_or(p, "radius", 12.0, C)?.max(0.0);
    let tol = number_or(p, "fidelity", 2.5, C)?.clamp(vectorcraft_tools::draw2::SMOOTH_MIN_TOLERANCE, vectorcraft_tools::draw2::SMOOTH_MAX_TOLERANCE);
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

    fn polyline(pts: &[(f64, f64)]) -> Vec<Anchor> {
        pts.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect()
    }

    /// Anchors on one point, anchors that are not numbers or are too far out to measure, and a
    /// zigzag of 100,001 anchors 1 pt high give no corner and do not fail; a square's four anchors
    /// are its corners, and an open path's ends never are.
    #[test]
    fn corners_are_found_only_where_the_path_turns_at_and_around_an_anchor() {
        assert_eq!(smooth_corners(&polyline(&[(5.0, 5.0); 4]), false, 12.0), vec![false; 4]);
        assert_eq!(smooth_corners(&polyline(&[(0.0, 0.0), (f64::NAN, 1.0), (10.0, 0.0)]), false, 12.0), vec![false; 3]);
        assert_eq!(smooth_corners(&polyline(&[(0.0, 0.0), (1e308, 0.0), (1e308, 1e308), (-1e308, 0.0)]), true, 12.0), vec![false; 4]);
        let zigzag: Vec<(f64, f64)> = (0..=100_000).map(|i| (f64::from(i), f64::from(i % 2))).collect();
        assert!(!smooth_corners(&polyline(&zigzag), false, 12.0).contains(&true));
        let square = polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        assert_eq!(smooth_corners(&square, true, 12.0), vec![true; 4]);
        assert_eq!(smooth_corners(&square, false, 12.0), vec![false, true, true, false]);
        assert_eq!(smooth_corners(&square, false, 0.0), vec![false, true, true, false], "with no reach the turn at the anchor decides");
        assert_eq!(smooth_corners(&square[..2], false, 12.0), vec![false; 2]);
    }

    /// A 90° bump 400 pt high cut into 1 pt segments, between two long straight segments: with a reach
    /// of 10 pt its top is a corner, and with a reach of 1,000 pt, where the walk passes more segments
    /// than it measures before it gets that far, its top is not one, as the path turns by about 51°
    /// over that reach.
    #[test]
    fn a_corner_is_not_kept_where_the_reach_is_not_measured() {
        let side = 400.0 * std::f64::consts::SQRT_2;
        let steps = side.ceil() as usize;
        let mut pts = vec![(-3000.0, 0.0)];
        pts.extend((0..steps).map(|j| {
            let f = j as f64 / steps as f64;
            (-400.0 + 400.0 * f, 400.0 * f)
        }));
        pts.extend((0..=steps).map(|j| {
            let f = j as f64 / steps as f64;
            (400.0 * f, 400.0 - 400.0 * f)
        }));
        pts.push((3000.0, 0.0));
        let anchors = polyline(&pts);
        let top = steps + 1;
        assert_eq!(anchors[top].p, Point::new(0.0, 400.0));
        assert!(smooth_corners(&anchors, false, 10.0)[top]);
        assert!(!smooth_corners(&anchors, false, 1000.0)[top]);
    }

    /// A 60° corner between two curved segments 1,000 pt long is found at a reach of 2 pt as it is
    /// when each segment is cut into a thousand pieces: the turns over the reach are measured along the
    /// curve near the anchor, not along a chord much longer than the reach.
    #[test]
    fn a_corner_between_long_curves_is_measured_along_them() {
        // The curves arrive along x and leave at 60° to it, each bending away from the other.
        let d = kurbo::Vec2::from_angle(-60f64.to_radians());
        let side = kurbo::Vec2::new(-d.y, d.x) * 1000.0;
        let v = Point::new(0.0, 0.0);
        let a = Anchor::with_handles(Point::new(-1000.0, 1000.0), Point::new(-1000.0, 1000.0), Point::new(-300.0, 1000.0));
        let b = Anchor::with_handles(v, Point::new(-300.0, 0.0), v + d * 300.0);
        let end = v + d * 1000.0 + side;
        let c = Anchor::with_handles(end, v + d * 300.0 + side, end);
        let long = [a, b, c];
        let fine: Vec<Anchor> = {
            let sp = SubPath::new(long.to_vec(), false);
            let mut pts = vec![];
            for i in 0..2 {
                let s = sp.segment(i);
                for j in 0..1000 {
                    let p = s.subsegment(f64::from(j) / 1000.0..f64::from(j + 1) / 1000.0);
                    if j == 0 && i == 0 {
                        pts.push(Anchor::with_handles(p.p0, p.p0, p.p1));
                    } else if let Some(last) = pts.last_mut() {
                        *last = Anchor::with_handles(last.p, last.h_in, p.p1);
                    }
                    pts.push(Anchor::with_handles(p.p3, p.p2, p.p3));
                }
            }
            pts
        };
        let at_v = |anchors: &[Anchor]| {
            let corners = smooth_corners(anchors, false, 2.0);
            anchors.iter().zip(corners).find(|(x, _)| x.p == v).map(|(_, c)| c)
        };
        assert_eq!(at_v(&fine), Some(true), "cut finely, the anchor is a corner");
        assert_eq!(at_v(&long), Some(true));
    }

    /// A jagged closed path whose length is less than twice the reach: its jags are not corners,
    /// while a triangle that small keeps its three.
    #[test]
    fn a_small_jagged_loop_has_no_corners() {
        let ring: Vec<(f64, f64)> = (0..16)
            .map(|i| {
                let (a, r) = (f64::from(i) / 16.0 * std::f64::consts::TAU, if i % 2 == 0 { 3.0 } else { 2.0 });
                (r * a.cos(), r * a.sin())
            })
            .collect();
        let jags = polyline(&ring);
        let sp = SubPath::new(jags.clone(), true);
        assert!(subpath_len(&sp) < 24.0);
        assert!(smooth_corners(&jags, true, 1e-6).iter().all(|c| *c), "each jag turns sharply at its anchor");
        assert_eq!(smooth_corners(&jags, true, 12.0), vec![false; 16]);
        assert_eq!(smooth_corners(&polyline(&[(0.0, 0.0), (6.0, 0.0), (3.0, 5.196)]), true, 12.0), vec![true; 3]);
    }
}
