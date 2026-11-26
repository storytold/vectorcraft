use std::collections::HashMap;

use vectorcraft_pathops::addons::pathtools as pt;
use vectorcraft_pathops::start_nearest;

use super::common::{dist_to_polyline, ids_json, path_mut, path_of, points_param, poly_bounds, ranged, subpath_len, target_paths};
use super::{
    Anchor, BezPath, CubicBez, NodeId, ParamCurve, ParamCurveNearest, PathData, Point, Rect, Result, Session, SimplifyOptions, SubPath, Value, bad,
    json, number_or, po,
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
/// that are no more than `reach` / 8 apart along the control polygon, and as many again in each
/// stretch twice as long after it until the pieces reach three times `reach`. The turns over `reach` and
/// twice `reach` from an anchor are then measured along the curve however long the segment is.
fn corner_places(c: CubicBez, reach: f64) -> Vec<f64> {
    let piece = reach / 8.0;
    let pieces = |len: f64| if piece > 0.0 { (len / piece).ceil().clamp(1.0, CORNER_PIECES as f64) as usize } else { 1 };
    let k = pieces(polygon_len(c)).max(4);
    let mut places: Vec<f64> = (1..=k).map(|j| j as f64 / k as f64).collect();
    if piece > 0.0 {
        for from_start in [true, false] {
            // The stretch from the end: halved until its control polygon is at most six reaches long.
            let first = 1.0 / k as f64;
            let mut t = first;
            let part = |t: f64| if from_start { c.subsegment(0.0..t) } else { c.subsegment(1.0 - t..1.0) };
            for _ in 0..60 {
                if polygon_len(part(t)) <= 6.0 * reach {
                    break;
                }
                t /= 2.0;
            }
            let side = |u: f64| if from_start { u } else { 1.0 - u };
            let m = pieces(polygon_len(part(t)));
            let mut fine: Vec<f64> = (1..=m).map(|j| t * j as f64 / m as f64).collect();
            // Then the stretches twice as long in turn, cut as finely, up to the first even place, until
            // the pieces reach three reaches from the end, so twice the reach is measured along the curve
            // where a handle that does not reach out makes the curve start more slowly than its polygon.
            let mut covered = 0.0;
            let mut at = c.eval(side(0.0));
            for u in &fine {
                let q = c.eval(side(*u));
                covered += at.distance(q);
                at = q;
            }
            let (mut from, mut levels) = (t, 0);
            while covered < 3.0 * reach && from < first && levels < 60 {
                let to = (2.0 * from).min(first);
                let piece_len = polygon_len(if from_start { c.subsegment(from..to) } else { c.subsegment(1.0 - to..1.0 - from) });
                let m = pieces(piece_len);
                for j in 1..=m {
                    let u = from + (to - from) * j as f64 / m as f64;
                    let q = c.eval(side(u));
                    covered += at.distance(q);
                    at = q;
                    fine.push(u);
                }
                (from, levels) = (to, levels + 1);
            }
            places.extend(fine.into_iter().map(side));
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
    // The lengths along the path ahead and back, measured once when an anchor needs them.
    let (mut ahead, mut back) = (None, None);
    if closed {
        let total = ahead.get_or_insert_with(|| walk_blocks(anchors, closed, true, reach)).first().map_or(0.0, |lens| lens.iter().take(n).sum());
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
        let back = back.get_or_insert_with(|| walk_blocks(anchors, closed, false, reach));
        let [Some(back_near), Some(back_far)] = spans.map(|r| point_along(anchors, closed, (i, false), r, back, reach)) else { continue };
        let ahead = ahead.get_or_insert_with(|| walk_blocks(anchors, closed, true, reach));
        let [Some(ahead_near), Some(ahead_far)] = spans.map(|r| point_along(anchors, closed, (i, true), r, ahead, reach)) else { continue };
        *flag = [(back_near, ahead_near, spans[0]), (back_far, ahead_far, spans[1])]
            .into_iter()
            .all(|(back, ahead, r)| r <= 0.0 || turn_between(a.p - back, ahead - a.p).is_some_and(|t| t > SMOOTH_CORNER_DEG));
    }
    corners
}

/// The lengths [`walk`] measures along the segments of `anchors`, in the order a walk ahead
/// (`forward`) or back passes them, twice round, starting at the first anchor ahead or the last
/// anchor back, and added up in blocks: element `l` holds the lengths of each run of 2^`l`
/// segments that starts at a multiple of 2^`l`. Each block adds up only its own segments, so a short
/// stretch keeps its length next to very long ones. The segment past an open path's end has no
/// length.
fn walk_blocks(anchors: &[Anchor], closed: bool, forward: bool, reach: f64) -> Vec<Vec<f64>> {
    let n = anchors.len();
    let lens: Vec<f64> =
        (0..n).map(|k| walk(anchors, closed, (if forward { k } else { n - 1 - k }, forward), [f64::INFINITY; 2], reach, 1).1).collect();
    let mut blocks = vec![(0..2 * n).map(|k| lens[k % n]).collect::<Vec<f64>>()];
    while let Some(last) = blocks.last().filter(|b| b.len() > 1) {
        let next = last.as_chunks::<2>().0.iter().map(|[a, b]| a + b).collect();
        blocks.push(next);
    }
    blocks
}

/// The point `dist` along the path `anchors` from anchor `from.0`, ahead when `from.1` and back
/// otherwise, as [`walk`] finds it: an open path's end for a distance beyond the end, and `None`
/// when a length is not a number or is too large to add. The walk skips whole blocks of `blocks`
/// ([`walk_blocks`] in the same direction) that end no more than `dist` along the path, so a path of
/// very many short segments is not walked one segment at a time from each anchor.
fn point_along(anchors: &[Anchor], closed: bool, from: (usize, bool), dist: f64, blocks: &[Vec<f64>], reach: f64) -> Option<Point> {
    let (n, (i, forward)) = (anchors.len(), from);
    let start = if forward { i } else { n - 1 - i };
    let end = start
        + if closed {
            n
        } else if forward {
            n - 1 - i
        } else {
            i
        };
    let (mut pos, mut gone) = (start, 0.0);
    // The largest block that starts at `pos`, stays within the walk and ends within `dist`, until
    // none does; a block whose length is not a number is never skipped.
    while let Some((size, len)) = blocks.iter().enumerate().rev().find_map(|(l, lens)| {
        let size = 1usize << l;
        let len = lens.get(pos >> l).copied()?;
        (pos % size == 0 && pos + size <= end && gone + len <= dist).then_some((size, len))
    }) {
        pos += size;
        gone += len;
    }
    let at = if forward { pos % n } else { n - 1 - pos % n };
    walk(anchors, closed, (at, forward), [(dist - gone).max(0.0); 2], reach, n).0[0]
}

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
    out.into_vec()
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
    #[cfg(test)]
    STRETCH_TESTS.with(|c| c.set(c.get() + of.len()));
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

#[cfg(test)]
thread_local! {
    /// Tests: how many times this thread measured a point or a box against one stretch of a drag.
    static STRETCH_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Most grid cells [`DragReach`] lists one stretch of the drag in. A stretch whose reach covers more
/// is measured against every point and box instead.
const MAX_STRETCH_CELLS: usize = 64;

/// Most grid cells across the drag's bounds on each axis in [`DragReach`].
const MAX_DRAG_CELLS: f64 = 1024.0;

/// The stretches of a Smooth drag listed by the cells of a grid that their reach covers, so that a
/// point or a piece of path is measured only against the stretches whose reach it is in. Each query
/// gives at least every stretch that measuring against the whole drag finds in reach, so the results
/// are the same as measuring against the whole drag.
struct DragReach<'a> {
    pts: &'a [Point],
    radius: f64,
    /// Every stretch of the drag, in order.
    all: Vec<usize>,
    /// The drag's bounds grown by more than `radius`, with the cell size; `None` when the drag or
    /// the reach is not a finite number, and then every query gives every stretch.
    grid: Option<(Rect, f64)>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    /// The stretches whose reach covers more than [`MAX_STRETCH_CELLS`] cells.
    wide: Vec<usize>,
}

impl<'a> DragReach<'a> {
    fn new(pts: &'a [Point], radius: f64) -> Self {
        let all = drag_stretches(pts);
        let mut reach = DragReach { pts, radius, all, grid: None, cells: HashMap::new(), wide: vec![] };
        if pts.is_empty() || !radius.is_finite() || pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return reach;
        }
        // Enough more than the reach to cover the rounding of each distance and box measured.
        let size = pts.iter().fold(0.0_f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
        let pad = radius + 1e-6 * radius.max(1.0) + 1e-9 * size;
        let area = poly_bounds(pts, pad);
        let cell = pad.max(area.width().max(area.height()) / MAX_DRAG_CELLS);
        if !(cell.is_finite() && cell > 0.0 && area.width().is_finite() && area.height().is_finite()) {
            return reach;
        }
        reach.grid = Some((area, cell));
        for &k in &reach.all {
            let Some(&a) = pts.get(k) else { continue };
            let b = pts.get(k + 1).copied().unwrap_or(a);
            let r = Rect::from_points(a, b).inflate(pad, pad);
            let ((x0, y0), (x1, y1)) = (reach.cell_of(Point::new(r.x0, r.y0)), reach.cell_of(Point::new(r.x1, r.y1)));
            if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > MAX_STRETCH_CELLS as i64 {
                reach.wide.push(k);
                continue;
            }
            for x in x0..=x1 {
                for y in y0..=y1 {
                    reach.cells.entry((x, y)).or_default().push(k);
                }
            }
        }
        reach
    }

    /// The cell `q` is in, counted from the corner of the grid.
    fn cell_of(&self, q: Point) -> (i64, i64) {
        let Some((area, cell)) = self.grid else { return (0, 0) };
        (((q.x - area.x0) / cell).floor() as i64, ((q.y - area.y0) / cell).floor() as i64)
    }

    /// Whether `q` is within the reach of the drag, and `slack` more.
    fn in_reach(&self, q: Point, slack: f64) -> bool {
        let limit = self.radius + slack;
        let Some((area, _)) = self.grid else { return dist_to_stretches(self.pts, &self.all, q) <= limit };
        if !(q.x >= area.x0 && q.x <= area.x1 && q.y >= area.y0 && q.y <= area.y1) {
            return false;
        }
        let listed = self.cells.get(&self.cell_of(q)).map_or(&[][..], Vec::as_slice);
        dist_to_stretches(self.pts, listed, q).min(dist_to_stretches(self.pts, &self.wide, q)) <= limit
    }

    /// The stretches, in order, that may come within the reach of `r`: at least every one that
    /// [`stretches_near`] gives for `r` from the whole drag.
    fn near_rect(&self, r: Rect) -> Vec<usize> {
        let Some((area, _)) = self.grid else { return self.all.clone() };
        let (x0, x1, y0, y1) = (r.x0.max(area.x0), r.x1.min(area.x1), r.y0.max(area.y0), r.y1.min(area.y1));
        if !(x0 <= x1 && y0 <= y1) {
            return vec![];
        }
        let ((cx0, cy0), (cx1, cy1)) = (self.cell_of(Point::new(x0, y0)), self.cell_of(Point::new(x1, y1)));
        if (cx1 - cx0 + 1).saturating_mul(cy1 - cy0 + 1) > self.all.len() as i64 {
            return self.all.clone();
        }
        let mut out = self.wide.clone();
        for x in cx0..=cx1 {
            for y in cy0..=cy1 {
                out.extend(self.cells.get(&(x, y)).into_iter().flatten());
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The distance from `q` to the nearest of the stretches `of` of the drag `pts`.
fn dist_to_stretches(pts: &[Point], of: &[usize], q: Point) -> f64 {
    #[cfg(test)]
    STRETCH_TESTS.with(|c| c.set(c.get() + of.len()));
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

/// How many curved segments of `pd` may come within the reach of the drag.
fn segments_in_reach(pd: &PathData, reach: &DragReach) -> usize {
    pd.subpaths
        .iter()
        .map(|sp| {
            (0..sp.segment_count())
                .filter(|&i| {
                    if vectorcraft_tools::draw2::segment_is_straight(sp, i) {
                        return false;
                    }
                    let b = control_box(sp.segment(i));
                    !stretches_near(reach.pts, &reach.near_rect(b), b, reach.radius).is_empty()
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

/// `sp` cut where the reach of the drag crosses its curved segments, and whether each
/// anchor of the result is in reach. An anchor in reach is added in the middle of each stretch of a
/// segment in reach that does not run on from one of the segment's anchors in reach, and an anchor
/// out of reach in the middle of each stretch out of reach between two stretches in reach, so that a
/// stretch in reach is smoothed even when the segment's anchors are not, and the stretches between
/// stay as they are. A segment is searched no more than half the reach apart where it comes within
/// the reach of the drag ([`reach_samples`]), measuring up to `share` pieces of each segment.
fn mark_reached(sp: &SubPath, reach: &DragReach, share: usize) -> (SubPath, Vec<bool>) {
    let (pts, radius) = (reach.pts, reach.radius);
    let n = sp.anchors.len();
    let marked: Vec<bool> = sp.anchors.iter().map(|a| reach.in_reach(a.p, 0.0)).collect();
    let mut places = vec![];
    for i in 0..sp.segment_count() {
        if vectorcraft_tools::draw2::segment_is_straight(sp, i) {
            continue;
        }
        let c = sp.segment(i);
        let (mut samples, mut budget) = (vec![], share);
        let near = reach.near_rect(control_box(c.subsegment(0.0..1.0)));
        reach_samples(c, (0.0, 1.0), (pts, &near), radius, &mut budget, &mut samples);
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
        .map(|(j, a)| if added.contains(&j) { reach.in_reach(a.p, 1e-9 * radius.max(1.0)) } else { kept.next().unwrap_or(false) })
        .collect();
    (out, marked)
}

/// `sp` with the anchors in the drag's reach smoothed and refitted, `None` when none are in reach.
/// A closed path whose first anchor is in reach starts at the point of the refitted path nearest
/// that anchor's place (VectorCraft's own rule).
fn smooth_subpath(sp: &SubPath, reach: &DragReach, share: usize, tol: f64, sigma: f64) -> Option<SubPath> {
    if sp.anchors.len() < 2 {
        return None;
    }
    let radius = reach.radius;
    let near = &|q: Point| reach.in_reach(q, 0.0);
    // A drag along a segment smooths it even when its anchors are out of reach.
    let (sp, marked) = mark_reached(sp, reach, share);
    let sp = &sp;
    if !marked.iter().any(|m| *m) {
        return None;
    }
    if sp.closed {
        let start = sp.anchors[0].p;
        let Some(k) = marked.iter().position(|m| !m) else {
            let corners = smooth_corners(&sp.anchors.contiguous(), true, radius);
            let Some(c) = corners.iter().position(|c| *c) else {
                // Everything is in range and nothing is a corner: smooth the whole closed path,
                // across its start too.
                let f = po::simplify_with(&PathData::single(sp.clone()), &refit_options(tol, sigma, [false; 2]));
                return f.subpaths.into_iter().next().map(|r| start_nearest(r, start, sp));
            };
            // Cut the path open at a corner, which then ends the last part as well as starting the first.
            let mut open = sp.anchors.to_vec();
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
            // The path starts where it did when the refit keeps an anchor there, and otherwise at its
            // point nearest there.
            return Some(start_nearest(SubPath::new(res, true), start, sp));
        };
        // Rotate so an untouched anchor comes first, duplicate it at the end and treat as open.
        let mut open = sp.anchors.to_vec();
        open.rotate_left(k);
        let mut m = marked.clone();
        m.rotate_left(k);
        open.push(open[0]);
        m.push(false);
        let mut res = smooth_open(&open, &m, near, tol, sigma, radius, false);
        if same_anchors(&res, &open) {
            return None;
        }
        let l = res.pop()?;
        if let Some(r0) = res.first_mut() {
            r0.h_in = l.h_in;
        }
        // With k = 0 the first anchor is out of reach and still starts the result.
        let res = SubPath::new(res, true);
        return Some(if k == 0 { res } else { start_nearest(res, start, sp) });
    }
    let anchors = sp.anchors.contiguous();
    let res = smooth_open(&anchors, &marked, near, tol, sigma, radius, true);
    (!same_anchors(&res, &anchors)).then(|| SubPath::new(res, false))
}

/// How much of a segment that ends in the drag's reach is in reach ([`split_at_reach`]).
enum Reach {
    /// All of it, or all but a sliver.
    Whole,
    /// No more than a sliver at its end.
    End,
    /// The part outside the reach, then the part in reach.
    Split(CubicBez, CubicBez),
}

/// Share of the tool's reach that the part of a segment in reach must be longer than for
/// [`split_at_reach`] to split the segment. VectorCraft's own value.
const REACH_SLIVER: f64 = 1.0 / 64.0;

/// The segment `c`, which ends in the drag's reach (`near`, `radius` points) and starts outside it,
/// split where it enters the reach. No more than 2% of its parameter range outside the reach is a
/// sliver, and so is a part in reach no longer than [`REACH_SLIVER`] of the reach.
fn split_at_reach(c: CubicBez, near: &dyn Fn(Point) -> bool, radius: f64) -> Reach {
    const STEPS: u32 = 32;
    let step = 1.0 / f64::from(STEPS);
    // Back from the end in reach to the first sample outside, then halving to the crossing.
    let Some(outside) = (1..=STEPS).map(|i| 1.0 - f64::from(i) * step).find(|t| !near(c.eval(*t))) else { return Reach::Whole };
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
    if hi <= 0.02 {
        return Reach::Whole;
    }
    let (far, reach) = (part(0.0..hi), part(hi..1.0));
    if polygon_len(reach) <= REACH_SLIVER * radius { Reach::End } else { Reach::Split(far, reach) }
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
    let mut out = EditsFromEnd::new(anchors.to_vec());
    for (a, b) in runs.into_iter().rev() {
        let (mut lo, mut hi) = (a.saturating_sub(1), (b + 1).min(n - 1));
        // How much of the segments leading out of the run is in reach. A segment that is in reach
        // only at the run's anchor stays out of the piece, so the piece ends at that anchor rather
        // than in a sliver of a segment, which would leave the refit no direction to end along.
        let Some(span) = out.get(lo, hi) else { continue };
        let (first, start, end, last) = (span[0], span[a - lo], span[b - lo], span[hi - lo]);
        let after = if hi > b { split_at_reach(CubicBez::new(last.p, last.h_in, end.h_out, end.p), near, radius) } else { Reach::Whole };
        let before = if lo < a { split_at_reach(CubicBez::new(first.p, first.h_out, start.h_in, start.p), near, radius) } else { Reach::Whole };
        if matches!(after, Reach::End) {
            hi = b;
        }
        if matches!(before, Reach::End) {
            lo = a;
        }
        if hi == lo {
            continue;
        }
        let Some(piece) = out.get(lo, hi) else { continue };
        let mut piece = piece.to_vec();
        // The anchors outside the reach, kept with the part of their segment that is outside.
        let (mut head, mut tail) = (None, None);
        if let Reach::Split(far, reach) = after
            && let [.., inner, outer] = piece.as_mut_slice()
        {
            inner.h_out = reach.p2;
            tail = Some(Anchor { h_in: far.p1, ..*outer });
            *outer = Anchor::with_handles(far.p3, reach.p1, far.p2);
        }
        if let Reach::Split(far, reach) = before
            && let [outer, inner, ..] = piece.as_mut_slice()
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
        out.replace(lo, hi, head.into_iter().chain(new).chain(tail));
    }
    out.finish()
}

/// A list edited by replacing stretches of it, each one ending no later than where the one before
/// started, in time proportional to the items replaced and moved rather than to the list's length
/// for each edit.
struct EditsFromEnd<T> {
    /// The start of the list.
    head: Vec<T>,
    /// The rest of the list, last item first.
    rest: Vec<T>,
}

impl<T> EditsFromEnd<T> {
    fn new(items: Vec<T>) -> Self {
        EditsFromEnd { head: items, rest: vec![] }
    }

    /// The items `lo` to `hi` of the list, `None` when it is shorter.
    fn get(&mut self, lo: usize, hi: usize) -> Option<&[T]> {
        while self.head.len() <= hi {
            self.head.push(self.rest.pop()?);
        }
        self.head.get(lo..=hi)
    }

    /// The list with the items `lo` to `hi` replaced by `with`, as [`Vec::splice`] gives it. Leaves
    /// the list as it is when it is shorter.
    fn replace(&mut self, lo: usize, hi: usize, with: impl IntoIterator<Item = T>) {
        if lo > hi || self.get(lo, hi).is_none() {
            return;
        }
        self.rest.extend(self.head.drain(hi + 1..).rev());
        self.head.truncate(lo);
        self.head.extend(with);
    }

    fn finish(mut self) -> Vec<T> {
        self.head.extend(self.rest.into_iter().rev());
        self.head
    }
}

/// Most pieces [`roughness`] cuts one curved segment into.
const ROUGHNESS_SEGMENT_PIECES: usize = 1024;

/// Most pieces [`roughness`] cuts the curved segments of one subpath into, shared equally by them.
const ROUGHNESS_PIECES: usize = 1 << 18;

/// How rough the subpath `sp` is, measured along the lines between points on each curved segment
/// about `step` apart along its control polygon (at least 4 and at most
/// [`ROUGHNESS_SEGMENT_PIECES`] pieces a segment, fewer on a subpath of very many segments): how
/// much it turns along its length in radians, from the direction each segment leaves its first anchor
/// along to the one it arrives at its last anchor along, with each turn at an anchor counted twice;
/// and how much it bends, the sum of each turn between two pieces squared over their mean length.
/// `None` when a point or a turn is not a number.
fn roughness(sp: &SubPath, step: f64) -> Option<(f64, f64)> {
    let segments = sp.segment_count();
    let most = (ROUGHNESS_PIECES / segments.max(1)).clamp(4, ROUGHNESS_SEGMENT_PIECES);
    // The directions along the path, each with whether it leaves an anchor, and the pieces' vectors.
    let (mut dirs, mut pieces): (Vec<(kurbo::Vec2, bool)>, Vec<kurbo::Vec2>) = (vec![], vec![]);
    for i in 0..segments {
        let c = sp.segment(i);
        if sp.segment_is_line(i) {
            dirs.push((c.p3 - c.p0, true));
            pieces.push(c.p3 - c.p0);
            continue;
        }
        let leave = [c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0].into_iter().find(|d| d.hypot() > 1e-12).unwrap_or_default();
        let arrive = [c.p3 - c.p2, c.p3 - c.p1, c.p3 - c.p0].into_iter().find(|d| d.hypot() > 1e-12).unwrap_or_default();
        let len = polygon_len(c);
        let k = if step > 0.0 && step.is_finite() && len.is_finite() { (len / step).ceil().clamp(4.0, most as f64) as usize } else { most };
        dirs.push((leave, true));
        let mut at = c.p0;
        for j in 1..=k {
            let q = if j == k { c.p3 } else { c.eval(j as f64 / k as f64) };
            dirs.push((q - at, false));
            pieces.push(q - at);
            at = q;
        }
        dirs.push((arrive, false));
    }
    // A direction of no length is left out; the turn at an anchor is then taken at the next one.
    let mut kept: Vec<(kurbo::Vec2, bool)> = Vec::with_capacity(dirs.len());
    let mut at_anchor = false;
    for (d, anchor) in dirs {
        at_anchor |= anchor;
        if d.hypot() > 1e-12 {
            kept.push((d, at_anchor));
            at_anchor = false;
        }
    }
    pieces.retain(|d| d.hypot() > 1e-12);
    let turn = |u: kurbo::Vec2, v: kurbo::Vec2| u.cross(v).atan2(u.dot(v)).abs();
    let pairs = |n: usize| if sp.closed { n } else { n.saturating_sub(1) };
    let mut turning = 0.0;
    for j in 0..pairs(kept.len()) {
        let (u, (v, anchor)) = (kept[j].0, kept[(j + 1) % kept.len()]);
        turning += if anchor { 2.0 } else { 1.0 } * turn(u, v);
    }
    let mut bending = 0.0;
    for j in 0..pairs(pieces.len()) {
        let (u, v) = (pieces[j], pieces[(j + 1) % pieces.len()]);
        bending += turn(u, v).powi(2) / (0.5 * (u.hypot() + v.hypot()));
    }
    (turning.is_finite() && bending.is_finite()).then_some((turning, bending))
}

/// Whether the Smooth tool keeps `new`, the subpath `old` smoothed with the reach `radius`
/// (VectorCraft's own rule). Both are measured with [`roughness`] at one spacing, an eighth of the
/// reach or 1/512 of the diagonal of `old`'s control points, whichever is smaller. `new` must turn
/// no more than a thousandth more than `old` (counting a turn of less than 1 radian as 1), and it
/// must have fewer anchors, turn less, or bend more than a ten-thousandth less. A result that would
/// leave the path rougher, or that adds anchors or changes the path without smoothing it, is not
/// kept.
fn smoother(old: &SubPath, new: &SubPath, radius: f64) -> bool {
    let size = old.anchors.iter().fold(None, |r: Option<Rect>, a| {
        let b = Rect::from_points(a.p, a.h_in).union_pt(a.h_out);
        Some(r.map_or(b, |r| r.union(b)))
    });
    let step = (radius / 8.0).min(size.map_or(0.0, |r| r.width().hypot(r.height()) / 512.0));
    let (Some((t0, b0)), Some((t1, b1))) = (roughness(old, step), roughness(new, step)) else { return false };
    let scale = t0.max(1.0);
    t1 <= t0 + 1e-3 * scale && (new.anchors.len() < old.anchors.len() || t0 - t1 >= 1e-6 * scale || b0 - b1 > 1e-4 * b0)
}

pub(super) fn smooth_region(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.smoothRegion";
    let pts = points_param(p, "points", C)?;
    let radius = ranged(p, "radius", 12.0, vectorcraft_tools::draw2::REACH_RANGE, C)?;
    let tol = number_or(p, "fidelity", 2.5, C)?;
    let (lo, hi) = (vectorcraft_tools::draw2::SMOOTH_MIN_TOLERANCE, vectorcraft_tools::draw2::SMOOTH_MAX_TOLERANCE);
    if !(lo..=hi).contains(&tol) {
        return Err(bad(C, format!("`fidelity` must be from {lo} to {hi} pt")));
    }
    let sigma = radius * SMOOTH_SPREAD;
    let st = s.doc()?;
    let mut plan = vec![];
    let (mut before, mut after) = (0, 0);
    let mut paths = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, radius)) {
        paths.push((id, path_of(&st.doc, id)?.0));
    }
    let reach = DragReach::new(&pts, radius);
    // Each curved segment in reach gets the same share of the search, whatever order they come in.
    let in_reach: usize = paths.iter().map(|(_, pd)| segments_in_reach(pd, &reach)).sum();
    let share = (MAX_REACH_PIECES / in_reach.max(1)).max(1);
    for (id, pd) in paths {
        let mut changed = false;
        let subs: Vec<SubPath> = pd
            .subpaths
            .iter()
            .map(|sp| match smooth_subpath(sp, &reach, share, tol, sigma) {
                Some(n) if n != *sp && n.anchors.len() >= 2 => {
                    changed = true;
                    n
                }
                _ => sp.clone(),
            })
            .collect();
        // A subpath whose result would not be smoother stays as it was.
        let subs: Vec<SubPath> =
            subs.into_iter().zip(&pd.subpaths).map(|(n, sp)| if n == *sp || smoother(sp, &n, radius) { n } else { sp.clone() }).collect();
        if changed && subs != pd.subpaths {
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
    let tol = ranged(p, "tolerance", 12.0, vectorcraft_tools::draw2::REACH_RANGE, C)?;
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
    use vectorcraft_pathops::START_SNAP;

    use super::*;

    /// A closed ring of four smooth anchors round (100, 100) with radius 50.
    fn ring() -> SubPath {
        let k = 0.5523 * 50.0;
        let a = |x: f64, y: f64, dx: f64, dy: f64| Anchor::with_handles(Point::new(x, y), Point::new(x - dx, y - dy), Point::new(x + dx, y + dy));
        SubPath::new(vec![a(150.0, 100.0, 0.0, k), a(100.0, 150.0, -k, 0.0), a(50.0, 100.0, 0.0, -k), a(100.0, 50.0, k, 0.0)], true)
    }

    /// A start between two anchors of a curve splits that curve there into two that trace it, and the
    /// new anchor comes first.
    #[test]
    fn start_nearest_splits_a_curve_where_it_passes_the_start() {
        let sp = ring();
        let old = sp.segment(0);
        let start = old.eval(0.3) + (old.eval(0.3) - Point::new(100.0, 100.0)) * 0.1;
        let out = start_nearest(sp.clone(), start, &sp);
        assert!(out.closed && out.anchors.len() == 5, "{out:?}");
        let t = old.nearest(start, 1e-9).t;
        assert!(out.anchors[0].p.distance(old.eval(t)) < 1e-9, "{out:?}");
        let (head, rest) = (out.segment(4), out.segment(0));
        for k in 0..=8 {
            let u = f64::from(k) / 8.0;
            assert!(head.eval(u).distance(old.eval(t * u)) < 1e-9);
            assert!(rest.eval(u).distance(old.eval(t + (1.0 - t) * u)) < 1e-9);
        }
        for i in 1..4 {
            assert_eq!(out.segment(i), sp.segment(i), "segment {i} stays");
        }
    }

    /// A start within [`START_SNAP`] of an anchor starts the ring at that anchor, with no anchor added.
    #[test]
    fn start_nearest_starts_at_an_anchor_near_the_start() {
        let sp = ring();
        let out = start_nearest(sp.clone(), Point::new(50.0, 100.0 + START_SNAP / 2.0), &sp);
        let mut want = sp.anchors.clone();
        want.rotate_left(2);
        assert_eq!(out.anchors, want);
    }

    /// A start on a straight side of a triangle adds a corner point there without handles, and the
    /// sides stay straight.
    #[test]
    fn start_nearest_splits_a_line_with_a_corner_point() {
        let sp = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(50.0, 80.0)], true);
        let out = start_nearest(sp.clone(), Point::new(40.0, -5.0), &sp);
        let want: Vec<Point> = [(40.0, 0.0), (100.0, 0.0), (50.0, 80.0), (0.0, 0.0)].map(|(x, y)| Point::new(x, y)).to_vec();
        assert!(out.anchors.len() == 4 && out.anchors.iter().zip(&want).all(|(a, w)| a.p.distance(*w) < 1e-9), "{out:?}");
        assert!((0..4).all(|i| out.segment_is_line(i)), "{out:?}");
    }

    /// A ring whose last anchor lies on its first keeps its first anchor first, and gets no third
    /// anchor at that place.
    #[test]
    fn start_nearest_keeps_the_first_of_two_anchors_on_one_place() {
        let p = [(0.0, 0.0), (100.0, 0.0), (50.0, 80.0), (0.0, 0.0)].map(|(x, y)| Point::new(x, y));
        let sp = SubPath::polyline(&p, true);
        assert_eq!(start_nearest(sp.clone(), p[0], &sp), sp);
    }

    /// Of points equally near the start, the one earliest along the path before smoothing from its
    /// first anchor comes first, whatever the smoothed path's order. The center of a square is equally
    /// near the middle of each side. The square before smoothing starts at (-50, -50), so the middle
    /// of its first side, (0, -50), comes first, though the smoothed copy starts at (50, 50).
    #[test]
    fn start_nearest_breaks_a_tie_by_the_order_before_smoothing() {
        let p = [(-50.0, -50.0), (50.0, -50.0), (50.0, 50.0), (-50.0, 50.0)].map(|(x, y)| Point::new(x, y));
        let orig = SubPath::polyline(&p, true);
        let mut sp = orig.clone();
        sp.anchors.rotate_left(2);
        let out = start_nearest(sp, Point::new(0.0, 0.0), &orig);
        assert!(out.closed && out.anchors.len() == 5, "{out:?}");
        assert!(out.anchors[0].p.distance(Point::new(0.0, -50.0)) < 1e-9, "{out:?}");
        assert!(out.anchors[1].p.distance(p[1]) < 1e-9, "{out:?}");
    }

    /// An open path, and a start that is not a number, leave the path as it is.
    #[test]
    fn start_nearest_leaves_an_open_path_or_a_start_without_a_place() {
        let open = SubPath { closed: false, ..ring() };
        assert_eq!(start_nearest(open.clone(), Point::new(100.0, 50.0), &ring()), open);
        assert_eq!(start_nearest(ring(), Point::new(f64::NAN, 0.0), &ring()), ring());
    }

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

    /// A path of `n` smooth anchors 1 pt apart, along x and up and down by 3 pt, every segment
    /// curved; closed, the anchors go round a circle `n` pt long instead.
    fn wave(n: usize, closed: bool) -> SubPath {
        let anchors = (0..n)
            .map(|i| {
                let f = i as f64;
                if closed {
                    let r = n as f64 / std::f64::consts::TAU + f64::from(u8::from(i % 2 == 1)) * 3.0;
                    let a = f / n as f64 * std::f64::consts::TAU;
                    let (p, t) = (Point::new(r * a.cos(), r * a.sin()), kurbo::Vec2::new(-a.sin(), a.cos()) * 0.4);
                    Anchor::with_handles(p, p - t, p + t)
                } else {
                    let p = Point::new(f, f64::from(u8::from(i % 2 == 1)) * 3.0);
                    Anchor::with_handles(p, p - kurbo::Vec2::new(0.4, 0.0), p + kurbo::Vec2::new(0.4, 0.0))
                }
            })
            .collect();
        SubPath::new(anchors, closed)
    }

    /// The same drag measured against all of its stretches for every point and box.
    fn linear<'a>(pts: &'a [Point], radius: f64) -> DragReach<'a> {
        DragReach { grid: None, ..DragReach::new(pts, radius) }
    }

    /// How many stretch measures `f` made on this thread.
    fn stretch_tests(f: impl FnOnce()) -> usize {
        STRETCH_TESTS.with(|c| c.set(0));
        f();
        STRETCH_TESTS.with(std::cell::Cell::get)
    }

    /// A drag of 200 points over 200 pt of a curved path of 100,000 anchors measures the anchors
    /// and segments near the drag against the stretches near them, not every anchor and segment
    /// against every stretch (about 40 million measures), and smooths the path as measuring
    /// everything does.
    #[test]
    fn a_drag_over_a_long_path_measures_only_what_is_near_it() {
        let sp = wave(100_000, false);
        let pts: Vec<Point> = (0..200).map(|i| Point::new(50_000.0 + f64::from(i), 1.5)).collect();
        let reach = DragReach::new(&pts, 12.0);
        let pd = PathData::single(sp.clone());
        let mut in_reach = 0;
        let boxes = stretch_tests(|| in_reach = segments_in_reach(&pd, &reach));
        assert!(in_reach > 0 && boxes < 100_000, "{in_reach} segments in reach, {boxes} measures");
        let share = (MAX_REACH_PIECES / in_reach).max(1);
        let mut got = None;
        let marks = stretch_tests(|| got = Some(mark_reached(&sp, &reach, share)));
        assert!(marks < 100_000, "{marks} measures");
        let smoothed = smooth_subpath(&sp, &reach, share, 2.5, 4.0);
        assert!(smoothed.is_some());
        let all = linear(&pts, 12.0);
        assert_eq!(segments_in_reach(&pd, &all), in_reach);
        assert_eq!(Some(mark_reached(&sp, &all, share)), got);
        assert_eq!(smooth_subpath(&sp, &all, share, 2.5, 4.0), smoothed);
    }

    /// Paths and drags of many shapes, a drag of one point, one far away, one with a stretch longer
    /// than the grid lists by cell, one with a point that is not a number, and reaches of 0 to 50 pt:
    /// the grid gives the same segments in reach, the same marks and the same smoothed path as
    /// measuring against the whole drag.
    #[test]
    fn the_drag_grid_gives_the_results_of_measuring_the_whole_drag() {
        let far = Point::new(1e7, -1e7);
        let paths = [
            ring(),
            wave(1500, false),
            wave(1500, true),
            SubPath::polyline(&(0..500).map(|i| Point::new(f64::from(i) * 2.0, f64::from(i % 3) * 4.0)).collect::<Vec<_>>(), false),
            SubPath::new(
                vec![Anchor::with_handles(Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(3e5, 4e5)), Anchor::corner(Point::new(1e6, 0.0))],
                false,
            ),
            SubPath::new(
                wave(200, true)
                    .anchors
                    .iter()
                    .map(|a| Anchor::with_handles(a.p + far.to_vec2(), a.h_in + far.to_vec2(), a.h_out + far.to_vec2()))
                    .collect(),
                true,
            ),
        ];
        let line = |from: (f64, f64), to: (f64, f64), n: u32| -> Vec<Point> {
            (0..=n).map(|i| Point::new(from.0, from.1).lerp(Point::new(to.0, to.1), f64::from(i) / f64::from(n))).collect()
        };
        let mut loop_drag: Vec<Point> =
            (0..400).map(|i| Point::new(100.0 + 120.0 * (f64::from(i) * 0.05).cos(), 120.0 * (f64::from(i) * 0.05).sin())).collect();
        loop_drag.extend(line((0.0, 0.0), (1500.0, 2.0), 50));
        let drags = [
            vec![Point::new(150.0, 100.0)],
            line((40.0, 90.0), (160.0, 110.0), 30),
            line((-100.0, 1.0), (1600.0, 2.0), 1500),
            loop_drag,
            line((5e6, 5e6), (5e6 + 10.0, 5e6), 5),
            vec![Point::new(0.0, 0.0), Point::new(1500.0, 3.0), Point::new(1500.0, 3.2), Point::new(10.0, 0.5)],
            vec![Point::new(10.0, 1.0), Point::new(f64::NAN, 0.0), Point::new(12.0, 1.0)],
            (0..1000).map(|i| Point::new(f64::from(i) * 0.01, 1.0)).collect(),
            line((far.x, far.y), (far.x + 300.0, far.y + 40.0), 80),
        ];
        for pts in &drags {
            for radius in [0.0, 0.5, 12.0, 50.0] {
                let (grid, all) = (DragReach::new(pts, radius), linear(pts, radius));
                for (k, sp) in paths.iter().enumerate() {
                    let pd = PathData::single(sp.clone());
                    let in_reach = segments_in_reach(&pd, &all);
                    assert_eq!(segments_in_reach(&pd, &grid), in_reach, "path {k}, reach {radius}, drag {:?}", pts.first());
                    let share = (MAX_REACH_PIECES / in_reach.max(1)).max(1);
                    assert_eq!(mark_reached(sp, &grid, share), mark_reached(sp, &all, share), "path {k}, reach {radius}, drag {:?}", pts.first());
                    let sigma = radius * SMOOTH_SPREAD;
                    let want = smooth_subpath(sp, &all, share, 2.5, sigma);
                    assert_eq!(smooth_subpath(sp, &grid, share, 2.5, sigma), want, "path {k}, reach {radius}, drag {:?}", pts.first());
                }
            }
        }
    }

    /// Replacing stretches of a list from its end, each one ending no later than where the one
    /// before started (sometimes on that item, sometimes with nothing or with more items), gives the
    /// list that [`Vec::splice`] gives.
    #[test]
    fn edits_from_the_end_match_splicing() {
        let mut seed = 0x2545_f491_u64;
        let mut next = |m: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % m.max(1)
        };
        for round in 0..200 {
            let items: Vec<usize> = (0..next(60) + 1).collect();
            let (mut want, mut got) = (items.clone(), EditsFromEnd::new(items.clone()));
            let mut end = items.len() - 1;
            loop {
                let hi = end.saturating_sub(next(3));
                let lo = hi.saturating_sub(next(6));
                let with: Vec<usize> = (0..next(5)).map(|j| 1000 * (round + 1) + j).collect();
                let Some(stretch) = want.get(lo..=hi).map(<[usize]>::to_vec) else { break };
                assert_eq!(got.get(lo, hi).map(<[usize]>::to_vec), Some(stretch), "round {round}");
                want.splice(lo..=hi, with.clone());
                got.replace(lo, hi, with);
                if lo == 0 {
                    break;
                }
                end = lo;
            }
            assert_eq!(got.finish(), want, "round {round}");
        }
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
    /// of 10 pt, and of 300 pt, where twice the reach passes the bump's 566 segments on each side, its
    /// top is a corner, and with a reach of 1,000 pt its top is not one, as the path turns by about 51°
    /// over that reach. The turns into and between 1 pt segments that follow two segments 1e16 pt long
    /// are corners too.
    #[test]
    fn a_bump_of_many_short_segments_is_a_corner_where_it_turns_over_the_reach() {
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
        assert!(smooth_corners(&anchors, false, 300.0)[top]);
        let long = polyline(&[(0.0, 0.0), (1e16, 0.0), (0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 2.0)]);
        let corners = smooth_corners(&long, false, 0.5);
        assert!(corners[2] && corners[3] && corners[4], "{corners:?}");
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

    /// A line that meets a curve leaving with no handle, turning by 58° at the anchor and by about
    /// 57.53° over twice a reach of 0.02 pt: the anchor is a corner, measured either way along the path,
    /// as the stretch measured finely near the anchor covers twice the reach along the curve.
    #[test]
    fn a_corner_next_to_a_curve_without_a_handle_is_kept() {
        let at = 58f64.to_radians();
        let prev = Anchor::corner(Point::new(-100.0 * at.cos(), -100.0 * at.sin()));
        let v = Point::new(0.0, 0.0);
        let a = Anchor::with_handles(v, v, v);
        let end = Point::new(600.0, 600.0);
        let next = Anchor::with_handles(end, Point::new(200.0, 0.0), end);
        assert_eq!(smooth_corners(&[prev, a, next], false, 0.02), vec![false, true, false]);
        let back = |x: Anchor| Anchor::with_handles(x.p, x.h_out, x.h_in);
        assert_eq!(smooth_corners(&[back(next), back(a), back(prev)], false, 0.02), vec![false, true, false]);
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

    /// `sp` scaled by `k` about the origin.
    fn scaled(sp: &SubPath, k: f64) -> SubPath {
        let at = |p: Point| Point::new(p.x * k, p.y * k);
        SubPath::new(sp.anchors.iter().map(|a| Anchor { p: at(a.p), h_in: at(a.h_in), h_out: at(a.h_out), ..*a }).collect::<Vec<Anchor>>(), sp.closed)
    }

    /// The Smooth tool keeps a result that turns less, and not one that turns more with as many
    /// anchors, one that adds an anchor and keeps the outline, or one that is the path itself, at
    /// any size.
    #[test]
    fn the_smooth_tool_keeps_only_a_smoother_result() {
        let round = ring();
        let mut bumped = ring();
        bumped.anchors[1].translate(kurbo::Vec2::new(0.0, -25.0));
        let split = start_nearest(ring(), round.segment(0).eval(0.3), &round);
        assert_eq!(split.anchors.len(), 5, "{split:?}");
        for k in [1e-3, 1.0, 1e3] {
            let (round, bumped, split) = (scaled(&round, k), scaled(&bumped, k), scaled(&split, k));
            let reach = 12.0 * k;
            assert!(smoother(&bumped, &round, reach), "at {k}x a bump smoothed away is kept");
            assert!(!smoother(&round, &bumped, reach), "at {k}x a bump added is not kept");
            assert!(!smoother(&round, &split, reach), "at {k}x an anchor added on the outline is not kept");
            assert!(!smoother(&round, &round.clone(), reach), "at {k}x the same path is not kept");
        }
    }
}
