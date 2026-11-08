//! Curve-preserving boolean operations built on `linesweeper`'s robust sweep-line topology.
//!
//! Inputs are converted to closed [`BezPath`]s (open subpaths are implicitly closed, as a filled
//! open path is painted in Illustrator). The sweep splits curves at intersections and y-extrema;
//! [`tidy_segments`] then re-joins pieces that came from the same smooth curve by refitting, so
//! results have roughly as few anchors as the inputs.

use std::collections::BTreeSet;

use kurbo::{BezPath, CubicBez, PathSeg, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours, Topology, WindingNumber};
use vectorcraft_geom::{Anchor, FillRule, PathData, SubPath};

use crate::PathOpsError;
use crate::fit::{end_tangent, fit_single, is_straight, sample, start_tangent};

#[cfg(test)]
thread_local! {
    /// The contour passes made over an arrangement on this thread, each of which walks all of it.
    pub(crate) static CONTOUR_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Default curve-refit tolerance (document points) used to merge split curve pieces back together.
/// Similar in spirit to Illustrator's Pathfinder "Precision" option (0.028 pt).
pub const DEFAULT_PRECISION: f64 = 0.01;

/// Binary boolean operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoolOp {
    /// A ∪ B
    Union,
    /// A ∩ B
    Intersect,
    /// A − B
    Difference,
    /// A ⊕ B (symmetric difference)
    Xor,
}

impl BoolOp {
    /// Evaluate the operation on "inside A" / "inside B" flags.
    pub fn apply(self, a: bool, b: bool) -> bool {
        match self {
            BoolOp::Union => a || b,
            BoolOp::Intersect => a && b,
            BoolOp::Difference => a && !b,
            BoolOp::Xor => a != b,
        }
    }
}

pub(crate) fn inside(rule: FillRule, w: i32) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// Winding numbers for an arbitrary number of input sets (tag = input index): the non-zero ones,
/// as (tag, winding) sorted by tag. A face is covered by a few of the inputs, so this stays short
/// where a vector of every input's winding would make each Pathfinder test cost one step per input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Multi(Vec<(usize, i32)>);

impl Multi {
    fn get(&self, i: usize) -> i32 {
        self.0.binary_search_by_key(&i, |e| e.0).ok().and_then(|k| self.0.get(k)).map_or(0, |e| e.1)
    }
    /// The non-zero windings, by input.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = (usize, i32)> + '_ {
        self.0.iter().copied()
    }
}

impl std::ops::Add for Multi {
    type Output = Multi;
    fn add(mut self, rhs: Self) -> Multi {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Multi {
    fn add_assign(&mut self, rhs: Self) {
        // In place, input by input (the sweep mostly adds one input's ±1 to a short list): add to
        // the input's winding, or insert it in tag order, and drop it if it reaches zero.
        for (t, k) in rhs.0 {
            match self.0.binary_search_by_key(&t, |e| e.0) {
                Ok(i) => {
                    if let Some(e) = self.0.get_mut(i) {
                        e.1 += k;
                        if e.1 == 0 {
                            self.0.remove(i);
                        }
                    }
                }
                Err(i) if k != 0 => self.0.insert(i, (t, k)),
                Err(_) => {}
            }
        }
    }
}

impl WindingNumber for Multi {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        Multi(vec![(tag, if positive { 1 } else { -1 })])
    }
    fn of_tag(&self, tag: usize) -> Self {
        let w = self.get(tag);
        Multi(if w == 0 { vec![] } else { vec![(tag, w)] })
    }
}

/// Convert a (filled) path to a closed BezPath: every subpath with ≥ 2 anchors is closed.
pub(crate) fn fill_bezpath(p: &PathData) -> BezPath {
    let mut bp = BezPath::new();
    for sp in &p.subpaths {
        if sp.anchors.len() < 2 {
            continue;
        }
        if sp.closed {
            sp.to_bezpath_into(&mut bp);
        } else {
            let mut c = sp.clone();
            c.closed = true;
            c.to_bezpath_into(&mut bp);
        }
    }
    bp
}

/// Sweep tolerance for a set of paths (same scaling rule as linesweeper's own `binary_op`).
pub(crate) fn eps_for<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Result<f64, PathOpsError> {
    let mut m: f64 = 0.0;
    for p in paths {
        for el in p.elements() {
            for pt in el_points(el) {
                if !pt.x.is_finite() || !pt.y.is_finite() {
                    return Err(PathOpsError::NonFinite);
                }
                m = m.max(pt.x.abs()).max(pt.y.abs());
            }
        }
    }
    let eps = (m * f64::EPSILON * 64.0).max(1e-6);
    // Past this the sweep tolerance is wider than a document unit: features thinner than it collapse and the call
    // would answer `Ok` with part of the art (or all of it) gone. Refuse instead, so callers keep the original.
    if eps > MAX_SWEEP_EPS {
        return Err(PathOpsError::Degenerate);
    }
    Ok(eps)
}

/// The widest sweep tolerance [`eps_for`] accepts: one document unit, reached by coordinates of about 7e13. (The engine
/// refuses coordinates far below that, so this only guards the library's own callers; it does not follow the `precision`
/// a caller asks for.)
const MAX_SWEEP_EPS: f64 = 1.0;

fn el_points(el: &kurbo::PathEl) -> impl Iterator<Item = Point> {
    use kurbo::PathEl::*;
    let v: [Option<Point>; 3] = match *el {
        MoveTo(p) | LineTo(p) => [Some(p), None, None],
        QuadTo(a, b) => [Some(a), Some(b), None],
        CurveTo(a, b, c) => [Some(a), Some(b), Some(c)],
        ClosePath => [None, None, None],
    };
    v.into_iter().flatten()
}

/// Make edges that rise by less than the sweep tolerance `eps` exactly horizontal.
///
/// The sweep orders a curve against another segment over a y-span shorter than `eps` by bounding
/// boxes alone, so an almost-horizontal edge (typically a boolean result's edge whose end, an
/// intersection point, is a rounding error off the input's horizontal) can be mis-ordered against
/// a curve it passes and the output loses a corner; exact horizontals are handled exactly. So
/// on-curve y coordinates within `eps` of each other snap to one value (the lowest of each
/// cluster, keeping distinct values at least `eps` apart), handles move with their anchors, and a
/// handle within `eps` of its anchor's height is levelled with it (no y-extremum is left in a
/// sub-`eps` sliver of a curve). Anchors move by less than `eps` and handles by less than `2·eps`,
/// the order of the sweep's own tolerance.
pub(crate) fn snap_horizontals(paths: &mut [&mut BezPath], eps: f64) {
    use kurbo::PathEl::*;
    let mut ys: Vec<f64> = paths.iter().flat_map(|p| p.elements().iter().filter_map(kurbo::PathEl::end_point)).map(|p| p.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut start = f64::NEG_INFINITY;
    let levels: Vec<f64> = ys
        .iter()
        .map(|&y| {
            if y - start >= eps {
                start = y;
            }
            start
        })
        .collect();
    let snap = |p: &mut Point| {
        let y = levels[ys.partition_point(|&v| v < p.y)];
        let moved = y - p.y;
        p.y = y;
        moved
    };
    // Shift a handle with its anchor, then level it with the anchor when within `eps`.
    let follow = |h: &mut Point, anchor: Point, moved: f64| {
        h.y += moved;
        if (h.y - anchor.y).abs() < eps {
            h.y = anchor.y;
        }
    };
    for p in paths.iter_mut() {
        // The current point (snapped) and how far snapping moved it; the subpath's start.
        let (mut cur, mut moved, mut first) = (Point::ZERO, 0.0, Point::ZERO);
        for el in p.elements_mut() {
            match el {
                MoveTo(q) => {
                    moved = snap(q);
                    (cur, first) = (*q, *q);
                }
                LineTo(q) => {
                    moved = snap(q);
                    cur = *q;
                }
                QuadTo(c, q) => {
                    let m = snap(q);
                    follow(c, cur, moved);
                    follow(c, *q, 0.0);
                    (cur, moved) = (*q, m);
                }
                CurveTo(c1, c2, q) => {
                    let m = snap(q);
                    follow(c1, cur, moved);
                    follow(c2, *q, m);
                    (cur, moved) = (*q, m);
                }
                ClosePath => {
                    // A later segment without a MoveTo starts from the subpath's (snapped) start.
                    (cur, moved) = (first, 0.0);
                }
            }
        }
    }
}

/// An arrangement (planar subdivision) of N filled paths.
pub(crate) struct Arrangement {
    pub(crate) top: Topology<Multi>,
    pub(crate) rules: Vec<FillRule>,
    /// Output cleanup that keeps the inputs' anchors.
    pub(crate) tidy: Tidy,
}

/// Output cleanup settings: refit `precision`, and anchor positions that must survive (the
/// inputs' own anchors — only joints the sweep introduced are merged away).
#[derive(Clone, Debug, Default)]
pub(crate) struct Tidy {
    pub(crate) precision: f64,
    keep: Vec<Point>,
    /// `keep` by y, sorted on first use.
    keep_by_y: std::sync::OnceLock<Vec<Point>>,
    /// Input curves (with inflated bounds) used to rebuild split pieces exactly.
    curves: Vec<(kurbo::Rect, CubicBez)>,
    /// `curves` indices by the left edge of their bounds, and the widest bounds: the curves whose
    /// bounds can hold a point, without testing them all.
    by_x: Vec<usize>,
    max_width: f64,
    /// The top edge of their bounds and their index, in order of that edge, with the tallest
    /// bounds, made on first use.
    by_y: std::sync::OnceLock<(Vec<(f64, usize)>, f64)>,
    /// The wide lookups in `curves` and in `keep` so far.
    curve_lookups: WideLookups,
    point_lookups: WideLookups,
}

/// The lookups in one list of [`Tidy`] with more than [`WIDE_WINDOW`] candidates along x: the
/// candidates they counted, a score that rises by [`Y_WIN`] when the window along y was the one
/// used and falls by one when it was not (within [`Y_SCORE_LIMIT`] either way), and the lookups
/// made while the score was below zero.
#[derive(Debug, Default)]
struct WideLookups {
    scanned: std::sync::atomic::AtomicUsize,
    score: std::sync::atomic::AtomicIsize,
    below: std::sync::atomic::AtomicUsize,
}

impl Clone for WideLookups {
    fn clone(&self) -> Self {
        let get = |n: &std::sync::atomic::AtomicUsize| n.load(std::sync::atomic::Ordering::Relaxed).into();
        Self { scanned: get(&self.scanned), score: self.score().into(), below: get(&self.below) }
    }
}

impl WideLookups {
    fn score(&self) -> isize {
        self.score.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Does a lookup with `window` candidates along x measure the window along y? It does past
    /// [`WIDE_WINDOW`] candidates, once wide lookups have scanned [`SORT_COST`] candidates for each
    /// of the `len` items that the sort along y orders, while the score is zero or more, and in
    /// one of every [`Y_RETRY`] lookups while it is below.
    fn search_y(&self, window: usize, len: usize) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        window > WIDE_WINDOW
            && self.scanned.fetch_add(window, Relaxed) + window >= len.saturating_mul(SORT_COST)
            && (self.score() >= 0 || self.below.fetch_add(1, Relaxed) % Y_RETRY == Y_RETRY - 1)
    }
    /// Counts a lookup that measured along y, and whether that window was the one used.
    fn used_y(&self, used: bool) {
        let step = if used { Y_WIN } else { -1 };
        let next = (self.score() + step).clamp(-Y_SCORE_LIMIT, Y_SCORE_LIMIT);
        self.score.store(next, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Tidy {
    /// Merge any smooth joint.
    pub(crate) fn free(precision: f64) -> Self {
        Self { precision, ..Default::default() }
    }
    /// Keep every on-curve point of `paths`.
    pub(crate) fn keeping<'a>(precision: f64, paths: impl IntoIterator<Item = &'a BezPath>) -> Self {
        let mut keep = Vec::new();
        let mut curves = Vec::new();
        for p in paths {
            for s in p.segments() {
                let seg = to_seg(s);
                keep.push(seg.c.p0);
                if !seg.line {
                    let r = seg.c.bounding_box().inflate(1e-6, 1e-6);
                    curves.push((r, seg.c));
                }
            }
        }
        keep.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut by_x: Vec<usize> = (0..curves.len()).collect();
        by_x.sort_by(|&i, &j| curves[i].0.x0.total_cmp(&curves[j].0.x0));
        let max_width = curves.iter().map(|(r, _)| r.width()).fold(0.0, f64::max);
        Self { precision, keep, curves, by_x, max_width, ..Default::default() }
    }
    fn keep_by_y(&self) -> &[Point] {
        self.keep_by_y.get_or_init(|| {
            let mut keep = self.keep.clone();
            keep.sort_unstable_by(|a, b| a.y.total_cmp(&b.y));
            keep
        })
    }
    fn by_y(&self) -> (&[(f64, usize)], f64) {
        let (by_y, max_height) = self.by_y.get_or_init(|| {
            let mut by_y: Vec<(f64, usize)> = self.curves.iter().enumerate().map(|(i, (r, _))| (r.y0, i)).collect();
            by_y.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            (by_y, self.curves.iter().map(|(r, _)| r.height()).fold(0.0, f64::max))
        });
        (by_y, *max_height)
    }
    /// Is a kept point within the tolerance of `p`? The points within it along x are tested, or
    /// those along y when [`WideLookups::search_y`] holds and there are fewer along y.
    fn is_kept(&self, p: Point) -> bool {
        let tol = 1e-6 * (1.0 + p.x.abs().max(p.y.abs()));
        let lo = self.keep.partition_point(|q| q.x < p.x - tol);
        // Usually one or two points lie within the tolerance along x; count them one by one up to
        // the wide window, and search past it.
        let rest = &self.keep[lo..];
        let mut within = rest.iter().take(WIDE_WINDOW + 1).take_while(|q| q.x <= p.x + tol).count();
        if within > WIDE_WINDOW {
            within = rest.partition_point(|q| q.x <= p.x + tol);
        }
        let hi = lo + within;
        if self.point_lookups.search_y(hi - lo, self.keep.len()) {
            let keep_by_y = self.keep_by_y();
            let lo_y = keep_by_y.partition_point(|q| q.y < p.y - tol);
            let hi_y = lo_y + keep_by_y[lo_y..].partition_point(|q| q.y <= p.y + tol);
            self.point_lookups.used_y(hi_y - lo_y < hi - lo);
            if hi_y - lo_y < hi - lo {
                return keep_by_y[lo_y..hi_y].iter().any(|q| (q.x - p.x).abs() <= tol);
            }
        }
        self.keep[lo..hi].iter().any(|q| (q.y - p.y).abs() <= tol)
    }
}

impl Arrangement {
    pub(crate) fn new(paths: Vec<(BezPath, FillRule)>) -> Result<Self, PathOpsError> {
        Self::new_budgeted(paths, crate::budget::MAX_SEGMENT_PAIRS)
    }

    pub(crate) fn new_budgeted(mut paths: Vec<(BezPath, FillRule)>, cap: usize) -> Result<Self, PathOpsError> {
        let eps = eps_for(paths.iter().map(|p| &p.0))?;
        snap_horizontals(&mut paths.iter_mut().map(|p| &mut p.0).collect::<Vec<_>>(), eps);
        // Counted on the snapped geometry, which is what the sweep sees: snapping can bring boxes into contact.
        if !crate::budget::sweep_fits_within(paths.iter().map(|p| &p.0), cap) {
            return Err(PathOpsError::TooComplex);
        }
        // The sweep can panic on rare near-degenerate input; that must never take the app down,
        // so it becomes an error (callers fall back to leaving the art unchanged).
        let sweep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Topology::<Multi>::from_paths(paths.iter().enumerate().map(|(i, (p, _))| (p, i)), eps)
        }));
        let top = sweep.map_err(|_| PathOpsError::Degenerate)?.map_err(|_| PathOpsError::OpenPath)?;
        let tidy = Tidy::keeping(DEFAULT_PRECISION, paths.iter().map(|p| &p.0));
        Ok(Self { top, rules: paths.iter().map(|p| p.1).collect(), tidy })
    }

    pub(crate) fn mask(&self, w: &Multi) -> Vec<bool> {
        // A zero winding is outside under both rules, so only the non-zero ones can set a flag.
        let mut m = vec![false; self.rules.len()];
        for (i, k) in w.iter() {
            if let (Some(&r), Some(b)) = (self.rules.get(i), m.get_mut(i)) {
                *b = inside(r, k);
            }
        }
        m
    }

    /// The inputs that cover a face of winding `w` (`mask(w)`'s true indices), back to front.
    pub(crate) fn covered<'a>(&'a self, w: &'a Multi) -> impl DoubleEndedIterator<Item = usize> + 'a {
        w.iter().filter(|&(i, k)| self.rules.get(i).is_some_and(|&r| inside(r, k))).map(|(i, _)| i)
    }

    /// [`Arrangement::contours`] with the test on the windings themselves (see [`Self::covered`]).
    pub(crate) fn contours_where(&self, pred: impl Fn(&Multi) -> bool) -> Contours {
        #[cfg(test)]
        CONTOUR_PASSES.with(|n| n.set(n.get() + 1));
        self.top.contours(pred)
    }

    pub(crate) fn contours(&self, pred: impl Fn(&[bool]) -> bool) -> Contours {
        self.contours_where(|w| pred(&self.mask(w)))
    }

    /// [`Self::contours`] with a panic in the sweep's contour pass turned into an error, as `new` does for the
    /// sweep itself (native builds only: on the web a panic aborts, whatever is caught here). Used by the calls that can return an error; the rest have none to return and let the engine's
    /// guard report it.
    pub(crate) fn try_contours(&self, pred: impl Fn(&[bool]) -> bool) -> Result<Contours, PathOpsError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.contours(pred))).map_err(|_| PathOpsError::Degenerate)
    }

    /// All distinct non-empty coverage masks that occur next to some edge, sorted.
    pub(crate) fn distinct_masks(&self) -> Vec<Vec<bool>> {
        let mut set = BTreeSet::new();
        for s in self.top.segment_indices() {
            let h = s.first_half();
            for w in [self.top.winding_clockwise(h), self.top.winding_counter_clockwise(h)] {
                let m = self.mask(w);
                if m.iter().any(|&b| b) {
                    set.insert(m);
                }
            }
        }
        set.into_iter().collect()
    }

    /// Vertices where three or more edges meet (true intersections / touch points).
    pub(crate) fn junctions(&self) -> Vec<Point> {
        let mut count: std::collections::HashMap<(u64, u64), (usize, Point)> = std::collections::HashMap::new();
        for s in self.top.segment_indices() {
            for h in [s.first_half(), s.second_half()] {
                let p = self.top.point(h).to_kurbo();
                count.entry((p.x.to_bits(), p.y.to_bits())).or_insert((0, p)).0 += 1;
            }
        }
        let mut v: Vec<Point> = count.into_values().filter(|(c, _)| *c >= 3).map(|(_, p)| p).collect();
        v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        v
    }
}

/// Convert selected contours into a tidy compound path.
pub(crate) fn contours_to_path(c: &Contours, idx: impl IntoIterator<Item = ContourIdx>, tidy: &Tidy) -> PathData {
    let mut subpaths = Vec::new();
    for i in idx {
        let path = tidy.repair(&c[i].path);
        if is_sliver(&path, tidy.precision) {
            continue;
        }
        if let Some(sp) = tidy_bezpath(&path, tidy) {
            subpaths.push(sp);
        }
    }
    PathData::new(subpaths)
}

pub(crate) fn all_contours_to_path(c: &Contours, tidy: &Tidy) -> PathData {
    contours_to_path(c, (0..c.contours().count()).map(ContourIdx), tidy)
}

/// Contours whose mean width (2·area / perimeter) is below `precision` are numerical debris
/// (e.g. the lens left between two coincident curves) and are dropped, like Illustrator's
/// Pathfinder precision setting does.
pub(crate) fn is_sliver(bp: &BezPath, precision: f64) -> bool {
    let a = bp.area().abs();
    let per = bp.perimeter(1e-6);
    per <= 0.0 || 2.0 * a / per < precision
}

/// One segment of a path: a cubic plus whether it is a straight line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg {
    pub(crate) c: CubicBez,
    pub(crate) line: bool,
}

impl Seg {
    pub(crate) fn line(a: Point, b: Point) -> Self {
        Self { c: CubicBez::new(a, a, b, b), line: true }
    }
}

pub(crate) fn to_seg(s: PathSeg) -> Seg {
    match s {
        PathSeg::Line(l) => Seg::line(l.p0, l.p1),
        PathSeg::Quad(q) => {
            let c = q.raise();
            Seg { c, line: is_straight(&c, 1e-9) }
        }
        PathSeg::Cubic(c) => Seg { c, line: is_straight(&c, 1e-9 * (1.0 + c.p0.distance(c.p3))) },
    }
}

fn tidy_bezpath(bp: &BezPath, tidy: &Tidy) -> Option<SubPath> {
    let segs: Vec<Seg> = bp.segments().map(to_seg).collect();
    let segs = tidy_segments(segs, true, tidy);
    segs_to_subpath(&segs, true)
}

/// Build a subpath from a chain of segments.
pub(crate) fn segs_to_subpath(segs: &[Seg], closed: bool) -> Option<SubPath> {
    if segs.is_empty() {
        return None;
    }
    let n = segs.len();
    let mut anchors = Vec::with_capacity(n + 1);
    for i in 0..n {
        let s = segs[i];
        let p = s.c.p0;
        let h_out = if s.line { p } else { s.c.p1 };
        let h_in = if i == 0 && !closed {
            p
        } else {
            let prev = segs[(i + n - 1) % n];
            if prev.line { p } else { prev.c.p2 }
        };
        anchors.push(Anchor::with_handles(p, h_in, h_out));
    }
    if !closed {
        let last = segs[n - 1];
        let p = last.c.p3;
        let h_in = if last.line { p } else { last.c.p2 };
        anchors.push(Anchor::with_handles(p, h_in, p));
    }
    if closed && anchors.len() < 2 {
        return None;
    }
    Some(SubPath::new(anchors, closed))
}

/// Merge segments that were split from one curve (or collinear lines) back together, dropping
/// degenerate pieces. `closed` segments form a loop.
pub(crate) fn tidy_segments(segs: Vec<Seg>, closed: bool, tidy: &Tidy) -> Vec<Seg> {
    let scale = segs.iter().map(|s| s.c.p0.distance(s.c.p3)).fold(0.0, f64::max);
    let tiny = (scale * 1e-9).max(1e-9);
    let mut segs: Vec<Seg> = segs
        .into_iter()
        .filter(|s| {
            let r = kurbo::Rect::from_points(s.c.p0, s.c.p3).union_pt(s.c.p1).union_pt(s.c.p2);
            r.width() + r.height() > tiny
        })
        .collect();
    let n = segs.len();
    if n < 2 {
        return segs;
    }
    // Joint i sits at the start of segment i.
    let mergeable = |a: &Seg, b: &Seg| -> bool {
        if a.line != b.line || tidy.is_kept(b.c.p0) {
            return false;
        }
        let ta = end_tangent(&a.c);
        let tb = start_tangent(&b.c);
        ta.dot(tb) > 0.9998 && ta.cross(tb).abs() < 0.02
    };
    if closed {
        // Rotate so we start at a real corner (if there is one).
        if let Some(k) = (0..n).find(|&i| !mergeable(&segs[(i + n - 1) % n], &segs[i])) {
            segs.rotate_left(k);
        }
    }
    let mut out: Vec<Seg> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut cur = segs[i];
        let mut j = i + 1;
        while j < n && mergeable(&segs[j - 1], &segs[j]) {
            match try_merge(&segs[i..=j], tidy) {
                Some(m) => {
                    cur = m;
                    j += 1;
                }
                None => break,
            }
        }
        out.push(cur);
        i = j;
    }
    out
}

fn try_merge(run: &[Seg], tidy: &Tidy) -> Option<Seg> {
    let precision = tidy.precision;
    let first = run[0].c;
    let last = run[run.len() - 1].c;
    if run[0].line {
        let chord = last.p3 - first.p0;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let ok = run.iter().all(|s| ((s.c.p3 - first.p0).cross(chord) / len).abs() <= precision * 0.5);
        return ok.then(|| Seg::line(first.p0, last.p3));
    }
    if let Some(c) = exact_merge(run, tidy) {
        return Some(Seg { c, line: false });
    }
    let mut pts = Vec::with_capacity(run.len() * 12 + 1);
    for (k, s) in run.iter().enumerate() {
        sample(&s.c, 12, &mut pts, k == 0);
    }
    let (c, err, _) = fit_single(&pts, start_tangent(&first), end_tangent(&last));
    if err > precision {
        return None;
    }
    Some(Seg { c, line: false })
}

/// If every piece of `run` lies on one input cubic, return the exact sub-curve spanning the run.
fn exact_merge(run: &[Seg], tidy: &Tidy) -> Option<CubicBez> {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let a = run[0].c.p0;
    let b = run[run.len() - 1].c.p3;
    let scale = 1.0 + a.x.abs().max(a.y.abs()).max(b.x.abs()).max(b.y.abs());
    let tol = 1e-6 * scale;
    for (r, c) in &tidy.curves {
        if !r.contains(a) || !r.contains(b) {
            continue;
        }
        let na = c.nearest(a, 1e-12);
        let nb = c.nearest(b, 1e-12);
        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol {
            continue;
        }
        let on_curve = run.iter().all(|s| c.nearest(s.c.eval(0.5), 1e-12).distance_sq.sqrt() <= tol * 10.0);
        if !on_curve || (na.t - nb.t).abs() < 1e-12 {
            continue;
        }
        let sub = if na.t < nb.t {
            c.subsegment(na.t..nb.t)
        } else {
            let r = c.subsegment(nb.t..na.t);
            CubicBez::new(r.p3, r.p2, r.p1, r.p0)
        };
        return Some(CubicBez::new(a, sub.p1, sub.p2, b));
    }
    None
}

/// Past this many candidates along x, a lookup in [`Tidy`] adds them to the count of scanned ones.
const WIDE_WINDOW: usize = 64;

/// The candidates scanned per item of a list before [`Tidy`] sorts that list along y: about the
/// comparisons per item of sorting 100,000 items.
const SORT_COST: usize = 16;

/// How much a lookup that used the window along y raises the score in [`WideLookups`].
const Y_WIN: isize = 4;

/// The highest and the lowest score in [`WideLookups`] (negated).
const Y_SCORE_LIMIT: isize = 256;

/// While the score in [`WideLookups`] is below zero, one lookup in this many measures along y.
const Y_RETRY: usize = 32;

impl Tidy {
    /// The input curves whose bounds hold both `a` and `b`, in the order of `by_x`. The candidates
    /// are the curves whose bounds start within the widest bounds' reach of the points along x, or
    /// the same along y when [`WideLookups::search_y`] holds and there are under a quarter as many
    /// along y.
    fn curves_around(&self, a: Point, b: Point) -> impl Iterator<Item = &CubicBez> {
        let holds = move |(r, _): &&(kurbo::Rect, CubicBez)| r.contains(a) && r.contains(b);
        let x = a.x.min(b.x);
        let hi = self.by_x.partition_point(|&i| self.curves[i].0.x0 <= x);
        let lo = self.by_x[..hi].partition_point(|&i| self.curves[i].0.x0 < x - self.max_width);
        let along_y = self.curve_lookups.search_y(hi - lo, self.curves.len()).then(|| {
            let (by_y, max_height) = self.by_y();
            let y = a.y.min(b.y);
            let hi_y = by_y.partition_point(|&(y0, _)| y0 <= y);
            let lo_y = by_y[..hi_y].partition_point(|&(y0, _)| y0 < y - max_height);
            &by_y[lo_y..hi_y]
        });
        let along_y = along_y.filter(|ys| {
            let used = ys.len().saturating_mul(4) < hi - lo;
            self.curve_lookups.used_y(used);
            used
        });
        let along_y = along_y.map(|ys| {
            let mut found: Vec<(usize, &(kurbo::Rect, CubicBez))> = ys.iter().map(|&(_, i)| (i, &self.curves[i])).filter(|(_, c)| holds(c)).collect();
            // `by_x` was sorted stably from index order, so this puts them in its order.
            found.sort_by(|(i, p), (j, q)| p.0.x0.total_cmp(&q.0.x0).then(i.cmp(j)));
            found.into_iter().map(|(_, c)| c)
        });
        let along_x = along_y.is_none().then(|| self.by_x[lo..hi].iter().map(|&i| &self.curves[i]).filter(holds));
        along_x.into_iter().flatten().chain(along_y.into_iter().flatten()).map(|(_, c)| c)
    }

    /// `bp` with each curve piece that strays from the one input curve through both its ends
    /// rebuilt as that curve's exact sub-curve. linesweeper 0.5 can emit a piece that doesn't
    /// follow its curve; out and back between two points of one curve, such pieces are a lens
    /// instead of nothing (issue #1028). A piece on a curve, or whose ends two different input
    /// curves pass through (a real lens between them), is left as it is.
    pub(crate) fn repair(&self, bp: &BezPath) -> BezPath {
        use kurbo::{ParamCurve, ParamCurveNearest, PathEl};
        if self.curves.is_empty() {
            return bp.clone();
        }
        let mut out = BezPath::new();
        let mut cur = Point::ZERO;
        let mut start = Point::ZERO;
        for el in bp.iter() {
            let piece = match el {
                PathEl::QuadTo(c, p) => Some(kurbo::QuadBez::new(cur, c, p).raise()),
                PathEl::CurveTo(c1, c2, p) => Some(CubicBez::new(cur, c1, c2, p)),
                _ => None,
            };
            match (el, piece) {
                (_, Some(raw)) => {
                    let (a, b) = (raw.p0, raw.p3);
                    let scale = 1.0 + a.x.abs().max(a.y.abs()).max(b.x.abs()).max(b.y.abs());
                    let tol = 1e-6 * scale;
                    let mut subs: Vec<CubicBez> = vec![];
                    let mut follows = false;
                    for c in self.curves_around(a, b) {
                        let (na, nb) = (c.nearest(a, 1e-12), c.nearest(b, 1e-12));
                        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol || (na.t - nb.t).abs() < 1e-12 {
                            continue;
                        }
                        if [0.25, 0.5, 0.75].iter().all(|&t| c.nearest(raw.eval(t), 1e-12).distance_sq.sqrt() <= tol * 10.0) {
                            follows = true;
                            break;
                        }
                        let sub = if na.t < nb.t {
                            c.subsegment(na.t..nb.t)
                        } else {
                            let r = c.subsegment(nb.t..na.t);
                            CubicBez::new(r.p3, r.p2, r.p1, r.p0)
                        };
                        subs.push(CubicBez::new(a, sub.p1, sub.p2, b));
                    }
                    let same = |p: &CubicBez, q: &CubicBez| p.p1.distance(q.p1) <= tol * 10.0 && p.p2.distance(q.p2) <= tol * 10.0;
                    match subs.split_first() {
                        Some((sub, rest)) if !follows && rest.iter().all(|q| same(sub, q)) => out.curve_to(sub.p1, sub.p2, b),
                        _ => out.push(el),
                    }
                    cur = b;
                }
                (PathEl::MoveTo(p), None) => {
                    out.push(el);
                    (cur, start) = (p, p);
                }
                (PathEl::LineTo(p), None) => {
                    out.push(el);
                    cur = p;
                }
                (PathEl::ClosePath, None) => {
                    out.push(el);
                    cur = start;
                }
                (_, None) => out.push(el),
            }
        }
        out
    }
}

/// Resolve self-intersections / overlaps of a single filled path into simple, consistently
/// oriented contours (outer contours and holes).
pub fn normalize(path: &PathData, rule: FillRule) -> PathData {
    try_normalize(path, rule).unwrap_or_default()
}

/// Fallible [`normalize`].
pub fn try_normalize(path: &PathData, rule: FillRule) -> Result<PathData, PathOpsError> {
    let bp = fill_bezpath(path);
    let c = normalize_bez(&bp, rule)?;
    Ok(all_contours_to_path(&c, &Tidy::keeping(DEFAULT_PRECISION, [&bp])))
}

pub(crate) fn normalize_bez(bp: &BezPath, rule: FillRule) -> Result<Contours, PathOpsError> {
    let eps = eps_for([bp])?;
    let mut bp = bp.clone();
    snap_horizontals(&mut [&mut bp], eps);
    // Counted on the snapped geometry, which is what the sweep sees: snapping can bring boxes into contact.
    if !crate::budget::sweep_fits([&bp]) {
        return Err(PathOpsError::TooComplex);
    }
    // As in `Sweep::new`, a panic in the sweep on near-degenerate input becomes an error.
    let swept =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<i32>::from_path(&bp, eps).map(|top| top.contours(|w| inside(rule, *w)))));
    swept.map_err(|_| PathOpsError::Degenerate)?.map_err(|_| PathOpsError::OpenPath)
}

/// Boolean operation between two filled paths. Returns an empty path on invalid (non-finite) input.
pub fn boolean(a: &PathData, a_rule: FillRule, b: &PathData, b_rule: FillRule, op: BoolOp) -> PathData {
    try_boolean(a, a_rule, b, b_rule, op, DEFAULT_PRECISION).unwrap_or_default()
}

/// Boolean operation with an explicit refit `precision`; errors on non-finite input.
///
/// The sweep can give some faces of the arrangement a wrong winding when an operand crosses or
/// touches itself or has overlapping subpaths, and the result then silently gains or loses a
/// region (#5478). So the windings are checked against each operand's own area
/// ([`windings_add_up`]); when they don't add up, each operand is first resolved on its own into
/// simple contours ([`resolved`]) and the operation runs again on those. When that doesn't add up
/// either, the first result is kept (as before the check).
pub fn try_boolean(a: &PathData, a_rule: FillRule, b: &PathData, b_rule: FillRule, op: BoolOp, precision: f64) -> Result<PathData, PathOpsError> {
    let (pa, pb) = (fill_bezpath(a), fill_bezpath(b));
    let arr = Arrangement::new(vec![(pa.clone(), a_rule), (pb.clone(), b_rule)])?;
    let c = arr.try_contours(|m| op.apply(m[0], m[1]))?;
    if !arrangement_adds_up(&arr, [&pa, &pb])
        && let Some(r) = resolved_boolean([(&pa, a_rule), (&pb, b_rule)], op, precision)
    {
        return Ok(r);
    }
    let tidy = Tidy { precision, ..arr.tidy };
    Ok(all_contours_to_path(&c, &tidy))
}

/// The widest winding (either sign) [`windings_add_up`] checks: one contour pass per level, so a
/// more tangled input is taken as it is.
const MAX_CHECKED_WINDING: i32 = 8;

/// Do the windings `arr` gave each input add up to that input's own area ([`windings_add_up`])?
/// Also true when it can't tell.
fn arrangement_adds_up(arr: &Arrangement, inputs: [&BezPath; 2]) -> bool {
    inputs.iter().enumerate().all(|(tag, p)| windings_add_up(&arr.top, |w: &Multi| w.get(tag), p).unwrap_or(true))
}

/// Does the winding `winding` that the sweep `top` gave its faces add up to the signed area of the
/// path `input` it was made from? By Green's theorem the integral of a path's winding number over
/// the plane is the path's signed area, and that integral is Σₖ |{w ≥ k}| − Σₖ |{w ≤ −k}| (k ≥ 1),
/// each area read from the contours of that region. A face given a wrong winding (#5478) puts it
/// off by the face's area. None when it can't tell: windings past [`MAX_CHECKED_WINDING`], or a
/// contour pass that fails.
fn windings_add_up<W: WindingNumber>(top: &Topology<W>, winding: impl Fn(&W) -> i32, input: &BezPath) -> Option<bool> {
    let (mut lo, mut hi) = (0, 0);
    for s in top.segment_indices() {
        let h = s.first_half();
        for w in [top.winding_clockwise(h), top.winding_counter_clockwise(h)] {
            let k = winding(w);
            (lo, hi) = (lo.min(k), hi.max(k));
        }
    }
    if hi > MAX_CHECKED_WINDING || lo < -MAX_CHECKED_WINDING {
        return None;
    }
    let region = |pred: &dyn Fn(i32) -> bool| -> Option<f64> {
        let c = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| top.contours(|w| pred(winding(w))))).ok()?;
        Some(c.contours().map(|ct| ct.path.area()).sum::<f64>().abs())
    };
    let mut integral = 0.0;
    for k in 1..=hi {
        integral += region(&|w| w >= k)?;
    }
    for k in 1..=-lo {
        integral -= region(&|w| w <= -k)?;
    }
    let want = input.area();
    // The sweep moves points by up to its tolerance, which moves an area by about that much along
    // each edge; a misplaced face is far larger.
    let bounds = input.bounding_box();
    let eps = eps_for([input]).ok()?;
    let tol = 1e-6 * bounds.area().abs() + 16.0 * eps * input.perimeter(1e-3) * f64::from(hi - lo).max(1.0);
    Some((integral - want).abs() <= tol || (integral + want).abs() <= tol)
}

/// `input` resolved on its own under `rule` into simple contours (outer contours and holes, filled
/// non-zero), when the windings of that sweep add up ([`windings_add_up`]).
fn resolved(input: &BezPath, rule: FillRule) -> Option<BezPath> {
    let eps = eps_for([input]).ok()?;
    let mut bp = input.clone();
    snap_horizontals(&mut [&mut bp], eps);
    if !crate::budget::sweep_fits([&bp]) {
        return None;
    }
    let top = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<i32>::from_path(&bp, eps))).ok()?.ok()?;
    if !windings_add_up(&top, |w| *w, &bp)? {
        return None;
    }
    let c = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| top.contours(|w| inside(rule, *w)))).ok()?;
    let mut out = BezPath::new();
    for ct in c.contours() {
        out.extend(ct.path.iter());
    }
    Some(out)
}

/// [`try_boolean`] on the operands resolved into simple contours first ([`resolved`]): the result
/// when every sweep's windings add up, else None.
fn resolved_boolean(inputs: [(&BezPath, FillRule); 2], op: BoolOp, precision: f64) -> Option<PathData> {
    let [(pa, a_rule), (pb, b_rule)] = inputs;
    let (ra, rb) = (resolved(pa, a_rule)?, resolved(pb, b_rule)?);
    let arr = Arrangement::new(vec![(ra.clone(), FillRule::NonZero), (rb.clone(), FillRule::NonZero)]).ok()?;
    if !arrangement_adds_up(&arr, [&ra, &rb]) {
        return None;
    }
    let c = arr.try_contours(|m| op.apply(m[0], m[1])).ok()?;
    // The refit keeps the operands' own anchors, not only those of their resolved contours.
    let tidy = Tidy::keeping(precision, [pa, pb]);
    Some(all_contours_to_path(&c, &tidy))
}

/// [`boolean_n`] that reports a refused input ([`PathOpsError::TooComplex`], non-finite
/// coordinates) instead of returning an empty path.
pub fn try_boolean_n(paths: &[(&PathData, FillRule)], pred: impl Fn(&[bool]) -> bool) -> Result<PathData, PathOpsError> {
    let bps: Vec<(BezPath, FillRule)> = paths.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect();
    let arr = Arrangement::new(bps)?;
    Ok(all_contours_to_path(&arr.try_contours(pred)?, &arr.tidy))
}

/// N-ary boolean: the area where `pred(inside_flags)` holds, `inside_flags[i]` telling whether a
/// point is inside `paths[i]` under its fill rule. A refused input gives an empty path; callers that
/// can report an error use [`try_boolean_n`].
pub fn boolean_n(paths: &[(&PathData, FillRule)], pred: impl Fn(&[bool]) -> bool) -> PathData {
    let bps: Vec<(BezPath, FillRule)> = paths.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect();
    match Arrangement::new(bps) {
        Ok(arr) => all_contours_to_path(&arr.contours(pred), &arr.tidy),
        Err(_) => PathData::default(),
    }
}

/// [`unite_all`] that reports a refused input instead of leaving it out of the union or returning
/// an empty path.
pub fn try_unite_all(paths: &[(&PathData, FillRule)]) -> Result<PathData, PathOpsError> {
    try_unite_all_with(paths, DEFAULT_PRECISION)
}

/// [`try_unite_all`] refit to `precision`, dropping slivers thinner than it.
pub fn try_unite_all_with(paths: &[(&PathData, FillRule)], precision: f64) -> Result<PathData, PathOpsError> {
    match crate::sweep_axis::turned_fills(paths) {
        Some(turned) => {
            let refs: Vec<(&PathData, FillRule)> = turned.iter().map(|(p, r)| (p, *r)).collect();
            try_unite_all_upright(&refs, precision).map(crate::sweep_axis::unturn)
        }
        None => try_unite_all_upright(paths, precision),
    }
}

/// [`try_unite_all_with`] with the sweeps run on `paths` as they are.
pub(crate) fn try_unite_all_upright(paths: &[(&PathData, FillRule)], precision: f64) -> Result<PathData, PathOpsError> {
    let mut all = BezPath::new();
    for (p, r) in paths {
        let bp = fill_bezpath(p);
        if bp.elements().is_empty() {
            continue;
        }
        for ct in normalize_bez(&bp, *r)?.contours() {
            all.extend(ct.path.iter());
        }
    }
    if all.elements().is_empty() {
        return Ok(PathData::default());
    }
    let tidy = Tidy::keeping(precision, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    Ok(all_contours_to_path(&normalize_bez(&all, FillRule::NonZero)?, &tidy))
}

/// Union of many filled paths. Each input is first normalised on its own, then all are swept
/// together with a single integer winding number, which keeps this fast for thousands of shapes.
pub fn unite_all(paths: &[(&PathData, FillRule)]) -> PathData {
    unite_all_with(paths, DEFAULT_PRECISION)
}

/// [`unite_all`] refit to `precision`, dropping slivers thinner than it.
pub fn unite_all_with(paths: &[(&PathData, FillRule)], precision: f64) -> PathData {
    match crate::sweep_axis::turned_fills(paths) {
        Some(turned) => {
            let refs: Vec<(&PathData, FillRule)> = turned.iter().map(|(p, r)| (p, *r)).collect();
            crate::sweep_axis::unturn(unite_all_upright(&refs, precision))
        }
        None => unite_all_upright(paths, precision),
    }
}

/// [`unite_all_with`] with the sweeps run on `paths` as they are.
pub(crate) fn unite_all_upright(paths: &[(&PathData, FillRule)], precision: f64) -> PathData {
    let mut all = BezPath::new();
    for (p, r) in paths {
        let bp = fill_bezpath(p);
        if bp.elements().is_empty() {
            continue;
        }
        let Ok(c) = normalize_bez(&bp, *r) else { continue };
        for ct in c.contours() {
            all.extend(ct.path.iter());
        }
    }
    if all.elements().is_empty() {
        return PathData::default();
    }
    let tidy = Tidy::keeping(precision, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    match normalize_bez(&all, FillRule::NonZero) {
        Ok(c) => all_contours_to_path(&c, &tidy),
        Err(_) => PathData::default(),
    }
}

/// [`area`] that reports a refused input instead of an area of zero.
pub fn try_area(path: &PathData, rule: FillRule) -> Result<f64, PathOpsError> {
    let c = normalize_bez(&fill_bezpath(path), rule)?;
    Ok(c.contours().map(|ct| ct.path.area()).sum::<f64>().abs())
}

/// Unsigned filled area of a path under a fill rule (normalises first, so it is exact for
/// self-overlapping input).
pub fn area(path: &PathData, rule: FillRule) -> f64 {
    let bp = fill_bezpath(path);
    match normalize_bez(&bp, rule) {
        Ok(c) => c.contours().map(|ct| ct.path.area()).sum::<f64>().abs(),
        Err(_) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use kurbo::ParamCurve;

    use super::*;

    /// A quarter-circle-like arc, the input curve the pieces below are cut from.
    fn arc() -> CubicBez {
        CubicBez::new((0.0, 0.0), (0.0, 55.0), (45.0, 100.0), (100.0, 100.0))
    }

    #[test]
    fn a_stray_piece_out_and_back_along_one_curve_is_no_outline() {
        // Issue #1028: linesweeper 0.5 handed back a contour running out along an input curve and
        // back by a piece that doesn't follow it, which refit into a visible lens.
        let c = arc();
        let tidy =
            Tidy::keeping(DEFAULT_PRECISION, [&BezPath::from_vec(vec![kurbo::PathEl::MoveTo(c.p0), kurbo::PathEl::CurveTo(c.p1, c.p2, c.p3)])]);
        let out = c.subsegment(0.2..0.6);
        let mut contour = BezPath::new();
        contour.move_to(out.p0);
        contour.curve_to(out.p1, out.p2, out.p3);
        // back to the start, bulging off the curve and overshooting the end like the reported piece
        contour.curve_to(out.p0 + (out.p0 - out.p3) * 0.15 + kurbo::Vec2::new(4.0, -3.0), out.p1 + kurbo::Vec2::new(3.0, -2.0), out.p0);
        contour.close_path();
        assert!(!is_sliver(&contour, tidy.precision), "the stray piece makes a lens");
        let fixed = tidy.repair(&contour);
        assert!(is_sliver(&fixed, tidy.precision), "{}", fixed.to_svg());
        // a piece on its curve is left exactly as it is
        let mut on = BezPath::new();
        on.move_to(out.p0);
        on.curve_to(out.p1, out.p2, out.p3);
        on.line_to(out.p0);
        on.close_path();
        assert_eq!(tidy.repair(&on), on);
    }

    #[test]
    fn input_curves_and_kept_points_are_found_in_wide_and_tall_art() {
        // Rows of short curves across the page, and the same stood on end: one lookup has a
        // narrow window along x, the other along y.
        let mut wide = BezPath::new();
        for row in 0..4 {
            let y = row as f64 * 3.0;
            wide.move_to((0.0, y));
            for k in 0..200 {
                let x = k as f64;
                wide.curve_to((x + 0.3, y + 1.0), (x + 0.7, y + 1.0), (x + 1.0, y));
            }
        }
        let tall = kurbo::Affine::new([0.0, 1.0, 1.0, 0.0, 0.0, 0.0]) * &wide;
        for art in [&wide, &tall] {
            let tidy = Tidy::keeping(DEFAULT_PRECISION, [art]);
            let mut sorted: Vec<usize> = (0..tidy.curves.len()).collect();
            sorted.sort_by(|&i, &j| tidy.curves[i].0.x0.total_cmp(&tidy.curves[j].0.x0).then(i.cmp(&j)));
            for (_, c) in tidy.curves.iter().step_by(7) {
                use kurbo::ParamCurve;
                // A curve's start point is also held by the curve before it, whose bounds start a
                // whole curve further back.
                for (a, b) in [(c.p0, c.p3), (c.p0, c.p0), (c.eval(0.25), c.eval(0.75)), (c.p0, c.p0 + kurbo::Vec2::new(2.5, 2.5))] {
                    let by_hand: Vec<&CubicBez> =
                        sorted.iter().map(|&i| &tidy.curves[i]).filter(|(r, _)| r.contains(a) && r.contains(b)).map(|(_, c)| c).collect();
                    assert_eq!(tidy.curves_around(a, b).collect::<Vec<_>>(), by_hand, "{a:?} {b:?}");
                }
                for p in [c.p0, c.p0 + kurbo::Vec2::new(1e-7, -1e-7), c.p0 + kurbo::Vec2::new(0.0, 1e-3), c.p0 + kurbo::Vec2::new(1e-3, 0.0)] {
                    let tol = 1e-6 * (1.0 + p.x.abs().max(p.y.abs()));
                    let by_hand = tidy.keep.iter().any(|q| (q.x - p.x).abs() <= tol && (q.y - p.y).abs() <= tol);
                    assert_eq!(tidy.is_kept(p), by_hand, "{p:?}");
                }
            }
            // The tall art's lookups reach the sorted lists along y; the wide art's never do.
            let tall_art = std::ptr::eq(art, &tall);
            assert_eq!(tidy.by_y.get().is_some(), tall_art);
            assert_eq!(tidy.keep_by_y.get().is_some(), tall_art);
        }
    }

    #[test]
    fn a_few_wide_lookups_do_not_sort_along_y() {
        let mut tall = BezPath::new();
        for x in 0..2 {
            let x = f64::from(x) * 3.0;
            tall.move_to((x, 0.0));
            for k in 0..200 {
                let y = f64::from(k);
                tall.curve_to((x + 1.0, y + 0.3), (x + 1.0, y + 0.7), (x, y + 1.0));
            }
        }
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&tall]);
        let a = Point::new(0.0, 50.0);
        let found = |tidy: &Tidy| tidy.curves_around(a, a).count();
        // Bounds at x = 0: the column's 200 curves are candidates along x, and two of them hold `a`.
        assert_eq!(found(&tidy), 2);
        assert!(tidy.is_kept(a));
        assert!(tidy.by_y.get().is_none());
        assert!(tidy.keep_by_y.get().is_none());
        // Enough of them to pay for the sort, and the answers stay the same.
        let scanned = |l: &WideLookups| l.scanned.load(std::sync::atomic::Ordering::Relaxed);
        while scanned(&tidy.curve_lookups) < SORT_COST * tidy.curves.len() || scanned(&tidy.point_lookups) < SORT_COST * tidy.keep.len() {
            assert_eq!(found(&tidy), 2);
            assert!(tidy.is_kept(a));
        }
        assert_eq!(found(&tidy), 2);
        assert!(tidy.is_kept(a));
        assert!(tidy.by_y.get().is_some());
        assert!(tidy.keep_by_y.get().is_some());
        // Along y the windows are narrow, so the lookups keep measuring there.
        assert!(tidy.curve_lookups.score() > 0 && tidy.point_lookups.score() > 0);
    }

    #[test]
    fn lookups_measure_along_y_rarely_where_it_narrows_nothing() {
        // A square grid of short curves: about as many candidates lie along y as along x.
        let mut grid = BezPath::new();
        for row in 0..100 {
            let y = f64::from(row) * 2.0;
            grid.move_to((0.0, y));
            for k in 0..100 {
                let x = f64::from(k);
                grid.curve_to((x + 0.3, y + 0.5), (x + 0.7, y + 0.5), (x + 1.0, y));
            }
        }
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&grid]);
        let lookups: u32 = 20_000;
        for k in 0..lookups {
            let a = Point::new(f64::from(k % 97) + 0.5, f64::from(k % 89) * 2.0 + 0.2);
            let found: Vec<&CubicBez> = tidy.curves_around(a, a).collect();
            if k % 500 == 0 {
                let by_hand: Vec<&CubicBez> = tidy.by_x.iter().map(|&i| &tidy.curves[i]).filter(|(r, _)| r.contains(a)).map(|(_, c)| c).collect();
                assert_eq!(found, by_hand, "{a:?}");
            }
        }
        // The window along y was measured until the score fell below zero, then once in Y_RETRY lookups.
        assert!(tidy.by_y.get().is_some());
        assert_eq!(tidy.curve_lookups.score(), -Y_SCORE_LIMIT);
        let below = tidy.curve_lookups.below.load(std::sync::atomic::Ordering::Relaxed);
        assert!(below > lookups as usize / 2, "{below} of {lookups} lookups below zero");
    }

    #[test]
    fn a_lens_between_two_curves_through_the_same_points_stays() {
        // Two input curves meeting at both ends bound a real face: neither piece is "repaired"
        // onto the other.
        let lower = CubicBez::new((0.0, 0.0), (30.0, 20.0), (70.0, 20.0), (100.0, 0.0));
        let upper = CubicBez::new((0.0, 0.0), (30.0, -20.0), (70.0, -20.0), (100.0, 0.0));
        let input = |c: CubicBez| BezPath::from_vec(vec![kurbo::PathEl::MoveTo(c.p0), kurbo::PathEl::CurveTo(c.p1, c.p2, c.p3)]);
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&input(lower), &input(upper)]);
        let mut lens = BezPath::new();
        lens.move_to(lower.p0);
        lens.curve_to(lower.p1, lower.p2, lower.p3);
        lens.curve_to(upper.p2, upper.p1, upper.p0);
        lens.close_path();
        assert_eq!(tidy.repair(&lens), lens);
    }

    #[test]
    fn arrangement_new_budgeted_fails_on_complexity_and_open_paths() {
        let mut bp = BezPath::new();
        for i in 0..100 {
            let r = kurbo::Rect::new(0.0, 0.0, 10.0 + i as f64, 10.0 + i as f64);
            bp.extend(r.to_path(0.1).iter());
        }
        let too_complex = Arrangement::new_budgeted(vec![(bp, FillRule::NonZero)], 5);
        assert_eq!(too_complex.err(), Some(PathOpsError::TooComplex));

        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 10.0));
        let failed_arr = Arrangement::new_budgeted(vec![(open, FillRule::NonZero)], 1000);
        assert_eq!(failed_arr.err(), Some(PathOpsError::OpenPath));
    }

    fn square(x: f64, y: f64, size: f64) -> PathData {
        PathData::single(vectorcraft_geom::SubPath::new(
            [(x, y), (x + size, y), (x + size, y + size), (x, y + size)]
                .iter()
                .map(|&(x, y)| vectorcraft_geom::Anchor::corner(Point::new(x, y)))
                .collect(),
            true,
        ))
    }

    #[test]
    fn coordinates_wider_than_the_precision_are_refused_not_emptied() {
        // At 1e15 the sweep tolerance is far above a 10 unit square: both squares used to vanish and the call said `Ok`.
        let near = square(0.0, 0.0, 10.0);
        let far = square(1e15, 1e15, 10.0);
        for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor] {
            assert_eq!(
                try_boolean(&near, FillRule::NonZero, &far, FillRule::NonZero, op, DEFAULT_PRECISION),
                Err(PathOpsError::Degenerate),
                "{op:?}"
            );
        }
        assert_eq!(try_area(&far, FillRule::NonZero), Err(PathOpsError::Degenerate));
        assert_eq!(try_normalize(&far, FillRule::NonZero), Err(PathOpsError::Degenerate));
        // The infallible entry points keep answering with an empty path.
        assert!(boolean(&near, FillRule::NonZero, &far, FillRule::NonZero, BoolOp::Union).is_empty());
    }

    #[test]
    fn coordinates_near_the_float_limit_do_not_reach_the_sweep() {
        // A 1e308 rectangle made linesweeper panic (a caught panic here, an abort on the web).
        let huge = square(0.0, 0.0, 1e308);
        let small = square(0.0, 0.0, 10.0);
        assert_eq!(try_area(&huge, FillRule::NonZero), Err(PathOpsError::Degenerate));
        assert_eq!(try_boolean(&huge, FillRule::NonZero, &small, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION), Err(PathOpsError::Degenerate));
        assert_eq!(try_unite_all(&[(&huge, FillRule::NonZero), (&small, FillRule::NonZero)]), Err(PathOpsError::Degenerate));
    }

    #[test]
    fn large_but_sensible_coordinates_still_work() {
        // Well inside the limit: far-out but sensible offsets.
        for at in [1e5, 1e7] {
            let u = try_boolean(
                &square(at, at, 10.0),
                FillRule::NonZero,
                &square(at + 5.0, at, 10.0),
                FillRule::NonZero,
                BoolOp::Union,
                DEFAULT_PRECISION,
            )
            .unwrap();
            let a = try_area(&u, FillRule::NonZero).unwrap();
            assert!((a - 150.0).abs() < 1.0, "at {at}: area {a}");
        }
    }

    #[test]
    fn the_coordinate_limit_sits_where_the_tolerance_passes_a_unit() {
        // 100 unit squares: tolerance 0.7 at 5e13 (kept), 1.4 at 1e14 (refused).
        assert!(try_area(&square(5e13, 5e13, 100.0), FillRule::NonZero).is_ok());
        assert_eq!(try_area(&square(1e14, 1e14, 100.0), FillRule::NonZero), Err(PathOpsError::Degenerate));
    }

    /// #5478: the sweep of a triangle against an operand that crosses itself gives faces windings
    /// that don't add up to the operands' areas; resolving the operands first gives ones that do.
    #[test]
    fn wrong_windings_are_caught_and_resolving_the_operands_fixes_them() {
        let poly = |pts: &[(f64, f64)]| {
            let mut bp = BezPath::new();
            for (i, p) in pts.iter().enumerate() {
                if i == 0 { bp.move_to(*p) } else { bp.line_to(*p) }
            }
            bp.close_path();
            bp
        };
        let a = poly(&[(3.0, 1.0), (3.0, 3.0), (0.0, 1.0)]);
        let b = poly(&[(3.0, 0.0), (0.0, 2.0), (3.0, 2.0), (0.0, 0.0), (0.0, 1.0)]);
        let arr = Arrangement::new(vec![(a.clone(), FillRule::NonZero), (b.clone(), FillRule::NonZero)]).unwrap();
        assert!(!arrangement_adds_up(&arr, [&a, &b]), "the sweep's windings are off");
        let (ra, rb) = (resolved(&a, FillRule::NonZero).unwrap(), resolved(&b, FillRule::NonZero).unwrap());
        let arr = Arrangement::new(vec![(ra.clone(), FillRule::NonZero), (rb.clone(), FillRule::NonZero)]).unwrap();
        assert!(arrangement_adds_up(&arr, [&ra, &rb]));
        // A plain pair adds up at once.
        let (s, t) = (poly(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]), poly(&[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]));
        let arr = Arrangement::new(vec![(s.clone(), FillRule::NonZero), (t.clone(), FillRule::EvenOdd)]).unwrap();
        assert!(arrangement_adds_up(&arr, [&s, &t]));
    }

    #[test]
    #[cfg(panic = "unwind")]
    fn a_panic_in_the_contour_pass_becomes_an_error() {
        let arr = Arrangement::new(vec![(fill_bezpath(&square(0.0, 0.0, 10.0)), FillRule::NonZero)]).unwrap();
        assert!(arr.try_contours(|m| m[0]).is_ok());
        assert_eq!(arr.try_contours(|_| panic!("simulated panic in the sweep")).err(), Some(PathOpsError::Degenerate));
    }
}
