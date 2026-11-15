//! The paths of long paths and compound paths kept by object id, with the boxes of their segments
//! and of their miter tips by runs of segments. When an object's anchors move, the renderer changes
//! only the path elements, boxes and tips of the segments that start or end at a moved anchor.
//! When anchors are added or deleted, or a subpath is closed or opened, it replaces only the path
//! elements of the segments between the first and the last anchor that changed, and measures again
//! only the runs that hold them. When a compound path gains or loses members, or a member gains or
//! loses subpaths, it makes the path of those members again. In each case it does not make the
//! object's path again or measure it whole.

use std::sync::Arc;

use vectorcraft_doc::{LineJoin, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::bez::has_length;
use vectorcraft_geom::path::segment_bounds;
use vectorcraft_geom::{Anchor, BezPath, PathData, PathEl, PathSeg, Point, Rect, SubPath};

/// Paths and compound paths with at least this many anchors are kept ([`KeptPath`]).
pub(crate) const KEPT_ANCHORS: usize = 1024;

/// Segments measured together into one box in a subpath measured whole. A run that anchors were
/// added to or deleted from holds more or fewer, and one that grows past twice this is split.
pub(crate) const RUN: usize = 128;

/// The most subpaths one edit may add anchors to, delete anchors from, close or open: past this,
/// the kept path is made again whole.
const SPLICES: usize = 16;

/// The boxes and the miter tips' boxes of a subpath's runs ([`KeptPath::runs`]).
#[cfg(test)]
pub(crate) type RunBoxes = (Vec<Option<Rect>>, Vec<Option<Rect>>);

/// A subpath's MoveTo, segments and anchors, whether it is closed, and the segments of each of its
/// runs ([`KeptPath::layout`]).
#[cfg(test)]
pub(crate) type Layout = ((usize, usize, usize, bool), Vec<usize>);

/// A subpath of a kept path.
struct Sub {
    /// Where its MoveTo is in the path's elements.
    start: usize,
    /// Its segments (as [`SubPath::segment_count`] counts them).
    segs: usize,
    anchors: usize,
    closed: bool,
    /// The segment after the last of each run of segments.
    ends: Vec<usize>,
    /// The box of each run of segments ([`segment_bounds`]).
    boxes: Vec<Option<Rect>>,
    /// The box of the points of each run of segments: their ends and their control points.
    controls: Vec<Option<Rect>>,
    /// The box of the miter tips of `tips_of` at the joints whose first segment is in each run.
    tips: Vec<Option<Rect>>,
}

/// The path of a long path or compound path ([`Node::stroke_path`]), with its geometric bounds by
/// runs of segments and the tips of a miter-joined stroke along it by runs of joints.
pub(crate) struct KeptPath {
    /// The object the path was made from (kept alive).
    pub(crate) node: Arc<Node>,
    pub(crate) path: Arc<BezPath>,
    /// The subpaths of each path of the object: the path itself, or each member of a compound path.
    parts: Vec<Vec<Sub>>,
    /// The stroke the subpaths' tips were measured for.
    tips_of: Option<StrokeLayer>,
    /// The last frame that used it.
    pub(crate) stamp: u64,
}

/// The paths of `n`: its own, or those of the members of a compound path whose members are all
/// paths and that is not a Shaper group or an Image Trace object. `None` for anything else.
fn paths(n: &Node) -> Option<Vec<&PathData>> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(vec![path]),
        NodeKind::Compound { children, .. } if n.extras().shaper.is_none() && n.extras().trace.is_none() => {
            children.iter().map(|c| c.path_data()).collect()
        }
        _ => None,
    }
}

/// Whether `n` has a path that is kept ([`KeptPath`]): a path or compound path with at least
/// [`KEPT_ANCHORS`] anchors, none of whose closed subpaths starts at a point that is not a number
/// (its path closes with an extra segment there).
pub(crate) fn kept(n: &Node) -> bool {
    let Some(paths) = paths(n) else { return false };
    let subs = || paths.iter().flat_map(|p| &p.subpaths);
    subs().map(|s| s.anchors.len()).sum::<usize>() >= KEPT_ANCHORS
        && subs().filter(|s| s.closed).all(|s| s.anchors.first().is_none_or(|a| !a.p.x.is_nan() && !a.p.y.is_nan()))
}

/// Whether anchors `a` and `b` have the same points, bit for bit.
fn same(a: &Anchor, b: &Anchor) -> bool {
    same_drawn(a, b, true, true)
}

/// Whether anchors `a` and `b` have the same point, bit for bit, and the same incoming handle when
/// `h_in` and outgoing handle when `h_out`.
fn same_drawn(a: &Anchor, b: &Anchor, h_in: bool, h_out: bool) -> bool {
    let bits = |p: Point| [p.x, p.y].map(f64::to_bits);
    bits(a.p) == bits(b.p) && (!h_in || bits(a.h_in) == bits(b.h_in)) && (!h_out || bits(a.h_out) == bits(b.h_out))
}

/// The path element of segment `i` of `sp`, as [`SubPath::to_bezpath_into`] writes it.
fn element(sp: &SubPath, i: usize) -> PathEl {
    let c = sp.segment(i);
    if sp.segment_is_line(i) { PathEl::LineTo(c.p3) } else { PathEl::CurveTo(c.p1, c.p2, c.p3) }
}

/// Element `o` of the elements [`SubPath::to_bezpath_into`] writes for `sp`, a subpath with
/// anchors: its MoveTo, then its segments, then its ClosePath.
fn element_at(sp: &SubPath, o: usize) -> PathEl {
    match o {
        0 => PathEl::MoveTo(sp.anchors.first().map_or(Point::ORIGIN, |a| a.p)),
        o if o <= sp.segment_count() => element(sp, o - 1),
        _ => PathEl::ClosePath,
    }
}

/// The point a path element ends at (`None` for a ClosePath).
fn end(el: &PathEl) -> Option<Point> {
    match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(p),
        PathEl::ClosePath => None,
    }
}

/// The box of the points of segment `s`: its ends and its control points.
fn segment_controls(s: PathSeg) -> Rect {
    match s {
        PathSeg::Line(l) => Rect::from_points(l.p0, l.p1),
        PathSeg::Quad(q) => Rect::from_points(q.p0, q.p2).union_pt(q.p1),
        PathSeg::Cubic(c) => Rect::from_points(c.p0, c.p3).union_pt(c.p1).union_pt(c.p2),
    }
}

/// The ends of the runs of a subpath of `segs` segments measured whole: [`RUN`] segments each.
fn whole_runs(segs: usize) -> Vec<usize> {
    (1..=segs.div_ceil(RUN)).map(|r| (r * RUN).min(segs)).collect()
}

/// The paths `ps` written over the `len` elements at `at` in `els`, and their subpaths measured
/// whole with the tips of `st`. `at` moves past them.
fn made_again(ps: &[&PathData], len: usize, els: &mut Vec<PathEl>, at: &mut usize, st: Option<&StrokeLayer>) -> Vec<Vec<Sub>> {
    let mut bp = BezPath::new();
    ps.iter().flat_map(|p| &p.subpaths).for_each(|sp| sp.to_bezpath_into(&mut bp));
    let to = (*at + len).min(els.len());
    els.splice((*at).min(to)..to, bp.into_elements());
    let mut measure = |sp: &SubPath| {
        let sub = Sub::measured(sp, *at, els, st);
        *at += sub.len();
        sub
    };
    ps.iter().map(|p| p.subpaths.iter().map(&mut measure).collect()).collect()
}

impl Sub {
    /// Subpath `sp`, its MoveTo at `start` in `els`, measured whole, with the tips of `st`.
    fn measured(sp: &SubPath, start: usize, els: &[PathEl], st: Option<&StrokeLayer>) -> Self {
        let segs = sp.segment_count();
        let ends = whole_runs(segs);
        let mut sub = Sub { start, segs, anchors: sp.anchors.len(), closed: sp.closed, ends, boxes: vec![], controls: vec![], tips: vec![] };
        sub.boxes = (0..sub.runs()).map(|r| sub.run_box(els, r)).collect();
        sub.controls = (0..sub.runs()).map(|r| sub.run_controls(els, r)).collect();
        if let Some(st) = st {
            sub.tips = (0..sub.runs()).map(|r| sub.run_tips(els, r, st)).collect();
        }
        sub
    }

    /// Its elements in the path: none without anchors, otherwise its MoveTo, its segments and
    /// its ClosePath when closed.
    fn len(&self) -> usize {
        if self.anchors == 0 { 0 } else { 1 + self.segs + usize::from(self.closed) }
    }

    /// Segment `i`, as kurbo's `segments` makes it from the path's elements.
    fn seg(&self, els: &[PathEl], i: usize) -> Option<PathSeg> {
        let p0 = end(els.get(self.start + i)?)?;
        Some(match *els.get(self.start + 1 + i)? {
            PathEl::LineTo(p) => PathSeg::Line(vectorcraft_geom::Line::new(p0, p)),
            PathEl::QuadTo(a, p) => PathSeg::Quad(vectorcraft_geom::kurbo::QuadBez::new(p0, a, p)),
            PathEl::CurveTo(a, b, p) => PathSeg::Cubic(vectorcraft_geom::CubicBez::new(p0, a, b, p)),
            PathEl::MoveTo(_) | PathEl::ClosePath => return None,
        })
    }

    fn runs(&self) -> usize {
        self.ends.len()
    }

    /// The segments of run `r`.
    fn range(&self, r: usize) -> std::ops::Range<usize> {
        let from = r.checked_sub(1).and_then(|p| self.ends.get(p)).copied().unwrap_or(0);
        from..self.ends.get(r).copied().unwrap_or(from)
    }

    /// The run that holds segment `i` (the last run for a segment past the end).
    fn run_of(&self, i: usize) -> usize {
        self.ends.partition_point(|&e| e <= i).min(self.runs().saturating_sub(1))
    }

    /// The box of the segments of run `r`.
    fn run_box(&self, els: &[PathEl], r: usize) -> Option<Rect> {
        self.range(r).filter_map(|i| self.seg(els, i)).map(segment_bounds).reduce(|a, b| a.union(b))
    }

    /// The box of the points of the segments of run `r`: their ends and their control points.
    fn run_controls(&self, els: &[PathEl], r: usize) -> Option<Rect> {
        self.range(r).filter_map(|i| self.seg(els, i)).map(segment_controls).reduce(|a, b| a.union(b))
    }

    /// Measure again the box and the box of the points of run `r`.
    fn measure_run(&mut self, els: &[PathEl], r: usize) {
        let (b, c) = (self.run_box(els, r), self.run_controls(els, r));
        if let Some(slot) = self.boxes.get_mut(r) {
            *slot = b;
        }
        if let Some(slot) = self.controls.get_mut(r) {
            *slot = c;
        }
    }

    /// The segment with length that follows segment `i` (round to the start when closed), or
    /// `None` when there is none other than `i`.
    fn next_drawn(&self, els: &[PathEl], i: usize) -> Option<PathSeg> {
        let after = i + 1..self.segs;
        let round = if self.closed { 0..i } else { 0..0 };
        after.chain(round).filter_map(|j| self.seg(els, j)).find(has_length)
    }

    /// The segment with length before segment `i` (round from the end when closed), when it is
    /// within [`RUN`] segments of `i`: `Ok(None)` when there is none, `Err(())` when the search
    /// gave up.
    fn prev_drawn(&self, els: &[PathEl], i: usize) -> Result<Option<usize>, ()> {
        let before = (0..i).rev();
        let round = if self.closed { (i + 1..self.segs).rev() } else { (0..0).rev() };
        for (steps, j) in before.chain(round).enumerate() {
            if steps >= RUN {
                return Err(());
            }
            if self.seg(els, j).is_some_and(|s| has_length(&s)) {
                return Ok(Some(j));
            }
        }
        Ok(None)
    }

    /// The box of the miter tips of `st` at the joints whose first segment is in run `r`
    /// ([`StrokeLayer::miter_tip`]).
    fn run_tips(&self, els: &[PathEl], r: usize, st: &StrokeLayer) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for i in self.range(r) {
            let Some(a) = self.seg(els, i).filter(has_length) else { continue };
            let Some(b) = self.next_drawn(els, i) else { continue };
            if let Some(tip) = st.miter_tip(self.closed, &a, &b) {
                out = Some(out.map_or(Rect::from_points(tip, tip), |o| o.union_pt(tip)));
            }
        }
        out
    }

    /// Bring the subpath, its MoveTo now at `at` in `els`, up to date from `was` to `now`, with the
    /// tips of `st`.
    fn update(&mut self, was: &SubPath, now: &SubPath, at: usize, els: &mut Vec<PathEl>, st: Option<&StrokeLayer>) {
        self.start = at;
        if was.anchors.len() == now.anchors.len() && was.closed == now.closed {
            self.moved(was, now, els, st);
        } else if was.anchors.len() < 2 || now.anchors.len() < 2 {
            self.remade(now, at, self.len(), els, st);
        } else {
            let mut span = self.len();
            if self.spliced(was, now, els, st, &mut span).is_none() {
                self.remade(now, at, span, els, st);
            }
        }
    }

    /// Write the elements of `now` over the `span` elements at `at` in `els` that the subpath holds,
    /// and measure it whole with the tips of `st`.
    fn remade(&mut self, now: &SubPath, at: usize, span: usize, els: &mut Vec<PathEl>, st: Option<&StrokeLayer>) {
        let to = at.saturating_add(span).min(els.len());
        let len = if now.anchors.is_empty() { 0 } else { 1 + now.segment_count() + usize::from(now.closed) };
        els.splice(at.min(to)..to, (0..len).map(|o| element_at(now, o)));
        *self = Sub::measured(now, at, els, st);
    }

    /// [`Self::update`] for a subpath with the same anchor count and closed state: the elements of
    /// the segments that start or end at an anchor that moved are made again, with the boxes of
    /// their runs and the tips of those runs and of the runs of the segments with length before
    /// them.
    fn moved(&mut self, was: &SubPath, now: &SubPath, els: &mut [PathEl], st: Option<&StrokeLayer>) {
        let (mut first, mut segs) = (false, vec![]);
        for (k, (a, b)) in was.anchors.iter().zip(&now.anchors).enumerate() {
            if same(a, b) {
                continue;
            }
            if k >= 1 && k - 1 < self.segs {
                segs.push(k - 1);
            }
            if k == 0 {
                first = true;
                if self.closed && self.segs > 0 {
                    segs.push(self.segs - 1);
                }
            }
            if k < self.segs {
                segs.push(k);
            }
        }
        if !first && segs.is_empty() {
            return;
        }
        segs.sort_unstable();
        segs.dedup();
        if first && let (Some(a), Some(el)) = (now.anchors.first(), els.get_mut(self.start)) {
            *el = PathEl::MoveTo(a.p);
        }
        for &i in &segs {
            if let Some(el) = els.get_mut(self.start + 1 + i) {
                *el = element(now, i);
            }
        }
        let els: &[PathEl] = els;
        // Many moved: the subpath is measured again whole.
        let all = segs.len() * 4 > self.segs;
        let mut boxes: Vec<usize> = if all { (0..self.runs()).collect() } else { segs.iter().map(|&i| self.run_of(i)).collect() };
        boxes.dedup();
        for r in boxes {
            self.measure_run(els, r);
        }
        let Some(st) = st else { return };
        // A joint's tip changes with its segments and with which segment with length comes next:
        // the runs of the moved segments and of the segments with length before them.
        let mut runs = vec![];
        let mut whole = all;
        for &i in &segs {
            runs.push(self.run_of(i));
            match self.prev_drawn(els, i) {
                Ok(Some(j)) => runs.push(self.run_of(j)),
                Ok(None) => {}
                Err(()) => whole = true,
            }
        }
        self.measure_tips(els, runs, whole, st);
    }

    /// [`Self::update`] for a subpath of at least two anchors before and after that gained or lost
    /// anchors or was closed or opened. Its elements from the first anchor that changed to the
    /// last are written again, and the closing segment and the ClosePath when the first or last
    /// anchor changed or it was closed or opened; the elements before and after stay. The runs
    /// that held the segments written are measured again, with the tips of the runs of the
    /// segments with length before them. `span` is the count of elements the subpath holds in
    /// `els`, kept up to date as they are written. `None` when the segments and runs kept do not
    /// fit `was`: the caller then makes the subpath again from those `span` elements.
    fn spliced(&mut self, was: &SubPath, now: &SubPath, els: &mut Vec<PathEl>, st: Option<&StrokeLayer>, span: &mut usize) -> Option<()> {
        let (n0, n1) = (was.anchors.len(), now.anchors.len());
        let least = n0.min(n1);
        // On a subpath open before and after, the handle before the first anchor and the one after
        // the last draw nothing.
        let open = !was.closed && !now.closed;
        let a = was.anchors.iter().zip(&now.anchors).enumerate().take_while(|(i, (x, y))| same_drawn(x, y, !open || *i > 0, true)).count().min(least);
        let b = was
            .anchors
            .iter()
            .rev()
            .zip(now.anchors.iter().rev())
            .take(least - a)
            .enumerate()
            .take_while(|(i, (x, y))| same_drawn(x, y, true, !open || *i > 0))
            .count();
        let (s0, s1) = (self.segs, now.segment_count());
        let (c0, c1) = (self.closed, now.closed);
        let (len0, len1) = (1 + s0 + usize::from(c0), 1 + s1 + usize::from(c1));
        // Elements `a..mid` change: the MoveTo when the first anchor changed and the segments up
        // to the first of those between the last `b` anchors.
        let (mid0, mid1) = ((1 + n0.checked_sub(b)?).min(1 + s0), (1 + n1.checked_sub(b)?).min(1 + s1));
        // Elements `tail..len` follow the segments between the last `b` anchors: the closing
        // segment and the ClosePath. They stay when the anchors either side of the closing segment
        // stayed and the subpath was neither closed nor opened.
        let (tail0, tail1) = if b >= 1 { (n0, n1) } else { (mid0, mid1) };
        let keep_tail = c0 == c1 && (!c1 || b == 0 || a >= 1);
        // The `old` segments from segment `lo` become `new` segments, and when the tail is written,
        // the segments from the one before it do too.
        let lo = a.saturating_sub(1);
        let (old, new) = (mid0.checked_sub(1)?.checked_sub(lo)?, mid1.checked_sub(1)?.checked_sub(lo)?);
        let tail = match keep_tail {
            true => None,
            false => Some((tail0.checked_sub(1)?, (1 + s0).saturating_sub(tail0), (1 + s1).saturating_sub(tail1), tail1.checked_sub(1)?)),
        };
        let at = self.start;
        let mut stale = vec![false; self.runs()];
        let tips = st.is_some();
        // Writes elements `e0..e1` of `now` over elements `o0..o1` of the subpath and updates `span`.
        let mut write = |els: &mut Vec<PathEl>, (o0, o1): (usize, usize), (e0, e1): (usize, usize)| {
            let to = at.saturating_add(o1).min(els.len());
            let from = at.saturating_add(o0).min(to);
            els.splice(from..to, (e0..e1).map(|o| element_at(now, o)));
            *span = from.saturating_sub(at) + e1.saturating_sub(e0) + at.saturating_add(*span).saturating_sub(to);
        };
        if let Some((lo, old, new, _)) = tail {
            self.resize(lo, old, new, &mut stale, tips)?;
            write(els, (tail0, len0), (tail1, len1));
        }
        self.resize(lo, old, new, &mut stale, tips)?;
        write(els, (a, mid0), (a, mid1));
        self.anchors = n1;
        self.closed = c1;
        let els: &[PathEl] = els;
        for r in stale.iter().enumerate().filter(|(_, s)| **s).map(|(r, _)| r) {
            self.measure_run(els, r);
        }
        let Some(st) = st else { return Some(()) };
        let mut runs: Vec<usize> = stale.iter().enumerate().filter(|(_, s)| **s).map(|(r, _)| r).collect();
        // Closed or opened: the tips of every joint change when the stroke reaches farther from one
        // than from the other.
        let mut whole = c0 != c1 && st.side_reach(c0).to_bits() != st.side_reach(c1).to_bits();
        for i in [lo].into_iter().chain(tail.map(|(.., last)| last)) {
            match self.prev_drawn(els, i) {
                Ok(Some(j)) => runs.push(self.run_of(j)),
                Ok(None) => {}
                Err(()) => whole = true,
            }
        }
        self.measure_tips(els, runs, whole, st);
        Some(())
    }

    /// Replace the `old` segments from segment `lo` with `new` segments in the runs: the runs that
    /// held them become one run, split into runs of [`RUN`] segments when longer than twice that,
    /// whose boxes and (with `tips`) tips are marked in `stale` to be measured again. `None`, and
    /// nothing changed, when the runs kept do not hold those `old` segments.
    fn resize(&mut self, lo: usize, old: usize, new: usize, stale: &mut Vec<bool>, tips: bool) -> Option<()> {
        if old == 0 && new == 0 {
            return Some(());
        }
        let (r0, r1, first, last) = match self.runs() {
            0 => (0, 0, 0, new),
            _ => {
                let r0 = self.run_of(lo);
                let r1 = if old == 0 { r0 } else { self.run_of(lo + old - 1) };
                let last = self.ends.get(r1).copied().unwrap_or(lo + old).checked_add(new)?.checked_sub(old)?;
                (r0, r1 + 1, self.range(r0).start, last)
            }
        };
        let len = last.checked_sub(first)?;
        let segs = self.segs.checked_add(new)?.checked_sub(old)?;
        let after: Vec<usize> = self.ends.get(r1..)?.iter().map(|&e| e.checked_add(new)?.checked_sub(old)).collect::<Option<_>>()?;
        if r1 > self.boxes.len() || r1 > self.controls.len() || r1 > stale.len() {
            return None;
        }
        let count = if len > 2 * RUN { len.div_ceil(RUN) } else { usize::from(len > 0) };
        self.ends.truncate(r1);
        self.ends.extend(after);
        self.ends.splice(r0..r1, (1..=count).map(|j| if j == count { last } else { first + j * RUN }));
        self.boxes.splice(r0..r1, std::iter::repeat_n(None, count));
        self.controls.splice(r0..r1, std::iter::repeat_n(None, count));
        if tips {
            self.tips.splice(r0.min(self.tips.len())..r1.min(self.tips.len()), std::iter::repeat_n(None, count));
        }
        stale.splice(r0..r1, std::iter::repeat_n(true, count));
        self.segs = segs;
        Some(())
    }

    /// Measure again the tips of `st` in `runs`, or in every run when `whole`.
    fn measure_tips(&mut self, els: &[PathEl], mut runs: Vec<usize>, whole: bool, st: &StrokeLayer) {
        if whole {
            runs = (0..self.runs()).collect();
        }
        runs.sort_unstable();
        runs.dedup();
        for r in runs {
            let t = self.run_tips(els, r, st);
            if let Some(slot) = self.tips.get_mut(r) {
                *slot = t;
            }
        }
    }
}

/// What [`KeptPath::tips`] measures `st`'s tips by: its reach from an open and a closed path and
/// its miter limit.
fn tips_key(st: &StrokeLayer) -> [u64; 3] {
    [st.side_reach(false).to_bits(), st.side_reach(true).to_bits(), st.miter_limit.to_bits()]
}

/// Whether the subpaths of `was` and `now` differ: in number, in anchors or in being closed.
fn differs(was: &PathData, now: &PathData) -> bool {
    !std::ptr::eq(was, now)
        && (was.subpaths.len() != now.subpaths.len()
            || was.subpaths.iter().zip(&now.subpaths).any(|(w, p)| {
                w.closed != p.closed || w.anchors.len() != p.anchors.len() || w.anchors.iter().zip(&p.anchors).any(|(a, b)| !same(a, b))
            }))
}

/// The subpaths of `was` and `now`, paths with as many subpaths, whose anchors were added or
/// deleted or that were closed or opened.
fn splices(was: &PathData, now: &PathData) -> usize {
    if std::ptr::eq(was, now) || was.subpaths.len() != now.subpaths.len() {
        return 0;
    }
    was.subpaths.iter().zip(&now.subpaths).filter(|(w, p)| w.anchors.len() != p.anchors.len() || w.closed != p.closed).count()
}

impl KeptPath {
    /// The kept path of `n`, or `None` when it has none ([`kept`]).
    pub(crate) fn new(n: &Arc<Node>, stamp: u64) -> Option<Self> {
        if !kept(n) {
            return None;
        }
        let path = n.stroke_path()?;
        let els = path.elements();
        let mut start = 0;
        let mut parts = vec![];
        for pd in paths(n)? {
            let mut subs = vec![];
            for sp in &pd.subpaths {
                let sub = Sub::measured(sp, start, els, None);
                start += sub.len();
                subs.push(sub);
            }
            parts.push(subs);
        }
        (start == els.len()).then(|| Self { node: n.clone(), path: Arc::new(path), parts, tips_of: None, stamp })
    }

    /// Bring the path up to date with `n`, a later version of the object it was made from. The
    /// subpaths whose anchors moved change as [`Sub::moved`] says, and those whose anchors were
    /// added or deleted or that were closed or opened as [`Sub::spliced`] says. A member of a
    /// compound path that gained or lost subpaths is made again, and so are the members added
    /// between the first and the last member that the edit left as they were, when the edit added
    /// or removed members. False, and nothing changed, when `n` has no kept path, or when the edit
    /// added or deleted anchors in or closed or opened more than [`SPLICES`] subpaths, or when the
    /// members kept do not pair with those of `n`. A subpath whose segments or runs kept do not fit
    /// its anchors before the edit is made again whole ([`Sub::spliced`]). False as well
    /// when the path written does not end where the subpaths measured end. The renderer makes the
    /// path again whole after a false.
    pub(crate) fn update(&mut self, n: &Arc<Node>) -> bool {
        let old = self.node.clone();
        let (Some(was), Some(now)) = (paths(&old), paths(n)) else { return false };
        if !kept(n) || self.parts.len() != was.len() || self.parts.iter().zip(&was).any(|(s, p)| s.len() != p.subpaths.len()) {
            return false;
        }
        // The members paired with one of `was`: all when there are as many, otherwise those
        // before the first and after the last that is not the same. The others are made again.
        let (head, tail) = if was.len() == now.len() {
            (was.len(), 0)
        } else {
            let head = was.iter().zip(&now).take_while(|(w, p)| std::ptr::eq(**w, **p)).count();
            let rest = (was.len() - head).min(now.len() - head);
            (head, was.iter().rev().zip(now.iter().rev()).take(rest).take_while(|(w, p)| std::ptr::eq(**w, **p)).count())
        };
        // Where the members that are not paired end in `was` and in `now`.
        let (Some(was_end), Some(now_end)) = (was.len().checked_sub(tail), now.len().checked_sub(tail)) else { return false };
        if head > was_end || head > now_end {
            return false;
        }
        let paired = |i: usize| match i < head {
            true => Some(i),
            false => now.len().checked_sub(i).filter(|&back| back <= tail).and_then(|back| was.len().checked_sub(back)),
        };
        let spliced: usize = (0..now.len()).filter_map(|i| paired(i).map(|w| splices(was[w], now[i]))).sum();
        if spliced > SPLICES {
            return false;
        }
        self.node = n.clone();
        // Nothing to change: the path is left as it is, shared or not.
        if was.len() == now.len() && was.iter().zip(&now).all(|(w, p)| !differs(w, p)) {
            return true;
        }
        let st = self.tips_of.clone();
        let st = st.as_ref();
        let path = Arc::make_mut(&mut self.path);
        let mut els = std::mem::take(path).into_elements();
        let mut old_parts: Vec<Option<Vec<Sub>>> = std::mem::take(&mut self.parts).into_iter().map(Some).collect();
        let mut at = 0;
        let mut parts = Vec::with_capacity(now.len());
        for (i, p) in now.iter().enumerate() {
            if i == head {
                // The members of `was` that are not paired give way to those of `now`.
                let gone: usize =
                    old_parts.get_mut(head..was_end).into_iter().flatten().flat_map(|s| s.take().into_iter().flatten()).map(|s| s.len()).sum();
                parts.extend(made_again(now.get(head..now_end).unwrap_or_default(), gone, &mut els, &mut at, st));
            }
            let Some(w) = paired(i) else { continue };
            let Some(mut subs) = old_parts.get_mut(w).and_then(Option::take) else { continue };
            if was[w].subpaths.len() != p.subpaths.len() {
                let len = subs.iter().map(Sub::len).sum();
                parts.extend(made_again(&[*p], len, &mut els, &mut at, st));
                continue;
            }
            let moved = !std::ptr::eq(was[w], *p);
            for (sub, (ws, ps)) in subs.iter_mut().zip(was[w].subpaths.iter().zip(&p.subpaths)) {
                if moved {
                    sub.update(ws, ps, at, &mut els, st);
                }
                sub.start = at;
                at += sub.len();
            }
            parts.push(subs);
        }
        if head == now.len() && head < was.len() {
            // Members were removed from the end.
            let gone: usize = old_parts[head..].iter_mut().flat_map(|s| s.take().into_iter().flatten()).map(|s| s.len()).sum();
            parts.extend(made_again(&[], gone, &mut els, &mut at, st));
        }
        let made = at == els.len();
        *path = BezPath::from_vec(els);
        self.parts = parts;
        made
    }

    /// The object's geometric bounds ([`Node::geometric_bounds`]): each path's as
    /// [`PathData::bounds`] measures it, joined.
    pub(crate) fn geometric(&self) -> Option<Rect> {
        let els = self.path.elements();
        self.parts.iter().fold(None, |acc, subs| {
            let anchors: usize = subs.iter().map(|s| s.anchors).sum();
            let b = match anchors {
                0 => None,
                // One anchor: the point itself.
                1 => subs.iter().find(|s| s.anchors == 1).and_then(|s| els.get(s.start)).and_then(end).map(|p| Rect::from_points(p, p)),
                _ => Some(subs.iter().flat_map(|s| s.boxes.iter().flatten()).copied().reduce(|a, b| a.union(b)).unwrap_or_default()),
            };
            vectorcraft_geom::union_opt(acc, b)
        })
    }

    /// The box of the points of the path's elements, as [`BezPath::control_box`] measures it: each
    /// subpath's MoveTo and the boxes of the points of its runs, joined.
    pub(crate) fn control_box(&self) -> Rect {
        let els = self.path.elements();
        let start = |s: &Sub| els.get(s.start).and_then(end).filter(|_| s.anchors > 0).map(|p| Rect::from_points(p, p));
        let runs = |s: &Sub| s.controls.iter().flatten().copied().reduce(|a, b| a.union(b));
        self.parts.iter().flatten().flat_map(|s| [start(s), runs(s)]).flatten().reduce(|a, b| a.union(b)).unwrap_or_default()
    }

    /// The box of the miter tips of `st` at every joint of the path ([`StrokeLayer::miter_tip`]),
    /// for [`StrokeLayer::bounds_with_tips`]: measured whole when the tips kept are another
    /// stroke's, then kept up to date with the path.
    pub(crate) fn tips(&mut self, st: &StrokeLayer) -> Option<Rect> {
        if st.join != LineJoin::Miter {
            return None;
        }
        if self.tips_of.as_ref().is_none_or(|t| t.join != LineJoin::Miter || tips_key(t) != tips_key(st)) {
            let els = self.path.elements();
            for sub in self.parts.iter_mut().flatten() {
                sub.tips = (0..sub.runs()).map(|r| sub.run_tips(els, r, st)).collect();
            }
            self.tips_of = Some(st.clone());
        }
        self.parts.iter().flatten().flat_map(|s| s.tips.iter().flatten()).copied().reduce(|a, b| a.union(b))
    }

    /// Each subpath's MoveTo, segments and anchors, whether it is closed, and the segments of each
    /// of its runs.
    #[cfg(test)]
    pub(crate) fn layout(&self) -> Vec<Layout> {
        let runs = |s: &Sub| (0..s.runs()).map(|r| s.range(r).len()).collect();
        self.parts.iter().flatten().map(|s| ((s.start, s.segs, s.anchors, s.closed), runs(s))).collect()
    }

    /// The boxes and tips kept per run of each subpath, the same measured again from the path
    /// over the same runs, and the stroke the tips were measured for.
    #[cfg(test)]
    pub(crate) fn runs(&self) -> (Vec<RunBoxes>, Vec<RunBoxes>, Option<StrokeLayer>) {
        let els = self.path.elements();
        let kept = self.parts.iter().flatten().map(|s| (s.boxes.clone(), s.tips.clone())).collect();
        let again = self.parts.iter().flatten().map(|s| {
            let boxes = (0..s.runs()).map(|r| s.run_box(els, r)).collect();
            let tips = self.tips_of.as_ref().map_or(vec![], |st| (0..s.runs()).map(|r| s.run_tips(els, r, st)).collect());
            (boxes, tips)
        });
        (kept, again.collect(), self.tips_of.clone())
    }
}
