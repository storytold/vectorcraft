//! The paths of long paths and compound paths kept by object id, with the boxes of their segments
//! and of their miter tips by runs of segments. When an object's anchors move, the renderer changes
//! only the path elements, boxes and tips of the segments that start or end at a moved anchor,
//! instead of making its path again and measuring it whole.

use std::sync::Arc;

use vectorcraft_doc::{LineJoin, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::bez::has_length;
use vectorcraft_geom::path::segment_bounds;
use vectorcraft_geom::{Anchor, BezPath, PathData, PathEl, PathSeg, Rect, SubPath};

/// Paths and compound paths with at least this many anchors are kept ([`KeptPath`]).
pub(crate) const KEPT_ANCHORS: usize = 1024;

/// Segments measured together into one box.
const RUN: usize = 128;

/// The boxes and the miter tips' boxes of a subpath's runs ([`KeptPath::runs`]).
#[cfg(test)]
pub(crate) type RunBoxes = (Vec<Option<Rect>>, Vec<Option<Rect>>);

/// A subpath of a kept path.
struct Sub {
    /// Where its MoveTo is in the path's elements.
    start: usize,
    /// Its segments (as [`SubPath::segment_count`] counts them).
    segs: usize,
    anchors: usize,
    closed: bool,
    /// The box of each run of [`RUN`] segments ([`segment_bounds`]).
    boxes: Vec<Option<Rect>>,
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
    let bits = |a: &Anchor| [a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y].map(f64::to_bits);
    bits(a) == bits(b)
}

/// The path element of segment `i` of `sp`, as [`SubPath::to_bezpath_into`] writes it.
fn element(sp: &SubPath, i: usize) -> PathEl {
    let c = sp.segment(i);
    if sp.segment_is_line(i) { PathEl::LineTo(c.p3) } else { PathEl::CurveTo(c.p1, c.p2, c.p3) }
}

/// The point a path element ends at (`None` for a ClosePath).
fn end(el: &PathEl) -> Option<vectorcraft_geom::Point> {
    match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(p),
        PathEl::ClosePath => None,
    }
}

impl Sub {
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
        self.segs.div_ceil(RUN)
    }

    /// The box of the segments of run `r`.
    fn run_box(&self, els: &[PathEl], r: usize) -> Option<Rect> {
        (r * RUN..((r + 1) * RUN).min(self.segs)).filter_map(|i| self.seg(els, i)).map(segment_bounds).reduce(|a, b| a.union(b))
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
        for i in r * RUN..((r + 1) * RUN).min(self.segs) {
            let Some(a) = self.seg(els, i).filter(has_length) else { continue };
            let Some(b) = self.next_drawn(els, i) else { continue };
            if let Some(tip) = st.miter_tip(self.closed, &a, &b) {
                out = Some(out.map_or(Rect::from_points(tip, tip), |o| o.union_pt(tip)));
            }
        }
        out
    }
}

/// What [`KeptPath::tips`] measures `st`'s tips by: its reach from an open and a closed path and
/// its miter limit.
fn tips_key(st: &StrokeLayer) -> [u64; 3] {
    [st.side_reach(false).to_bits(), st.side_reach(true).to_bits(), st.miter_limit.to_bits()]
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
                let segs = sp.segment_count();
                let mut sub = Sub { start, segs, anchors: sp.anchors.len(), closed: sp.closed, boxes: vec![], tips: vec![] };
                sub.boxes = (0..sub.runs()).map(|r| sub.run_box(els, r)).collect();
                if !sp.anchors.is_empty() {
                    start += 1 + segs + usize::from(sp.closed);
                }
                subs.push(sub);
            }
            parts.push(subs);
        }
        (start == els.len()).then(|| Self { node: n.clone(), path: Arc::new(path), parts, tips_of: None, stamp })
    }

    /// Bring the path up to date with `n`, a path with the same subpaths and anchor counts as the
    /// object it was made from: the elements of the segments that start or end at an anchor that
    /// moved are made again, with the boxes of their runs and the tips of those runs and of the
    /// runs of the segments with length before them. False, and nothing changed, when `n` has
    /// other subpaths, other anchor counts or no kept path.
    pub(crate) fn update(&mut self, n: &Arc<Node>) -> bool {
        let old = self.node.clone();
        let (Some(was), Some(now)) = (paths(&old), paths(n)) else { return false };
        let shape =
            |ps: &[&PathData]| ps.iter().map(|p| p.subpaths.iter().map(|s| (s.anchors.len(), s.closed)).collect::<Vec<_>>()).collect::<Vec<_>>();
        if !kept(n) || shape(&was) != shape(&now) {
            return false;
        }
        // Per subpath whose anchors moved: (member, subpath, whether its first anchor moved, the
        // segments that start or end at a moved anchor).
        let mut moved: Vec<(usize, usize, bool, Vec<usize>)> = vec![];
        for (pi, (w, p)) in was.iter().zip(&now).enumerate() {
            if std::ptr::eq(*w, *p) {
                continue;
            }
            for (si, (ws, ps)) in w.subpaths.iter().zip(&p.subpaths).enumerate() {
                let Some(sub) = self.parts.get(pi).and_then(|s| s.get(si)) else { return false };
                let (mut first, mut segs) = (false, vec![]);
                for (k, (a, b)) in ws.anchors.iter().zip(&ps.anchors).enumerate() {
                    if same(a, b) {
                        continue;
                    }
                    if k >= 1 && k - 1 < sub.segs {
                        segs.push(k - 1);
                    }
                    if k == 0 {
                        first = true;
                        if sub.closed && sub.segs > 0 {
                            segs.push(sub.segs - 1);
                        }
                    }
                    if k < sub.segs {
                        segs.push(k);
                    }
                }
                if first || !segs.is_empty() {
                    segs.sort_unstable();
                    segs.dedup();
                    moved.push((pi, si, first, segs));
                }
            }
        }
        self.node = n.clone();
        if moved.is_empty() {
            return true;
        }
        let path = Arc::make_mut(&mut self.path);
        let els = path.elements_mut();
        for (pi, si, first, segs) in &moved {
            let (Some(sub), Some(sp)) = (self.parts.get(*pi).and_then(|s| s.get(*si)), now.get(*pi).and_then(|p| p.subpaths.get(*si))) else {
                continue;
            };
            if *first && let (Some(a), Some(el)) = (sp.anchors.first(), els.get_mut(sub.start)) {
                *el = PathEl::MoveTo(a.p);
            }
            for &i in segs {
                if let Some(el) = els.get_mut(sub.start + 1 + i) {
                    *el = element(sp, i);
                }
            }
        }
        let els = path.elements();
        for (pi, si, _, segs) in moved {
            let tips_of = self.tips_of.as_ref();
            let Some(sub) = self.parts.get_mut(pi).and_then(|s| s.get_mut(si)) else { continue };
            // Many moved: the subpath is measured again whole.
            let all = segs.len() * 4 > sub.segs;
            let mut boxes: Vec<usize> = if all { (0..sub.runs()).collect() } else { segs.iter().map(|i| i / RUN).collect() };
            boxes.dedup();
            for r in boxes {
                sub.boxes[r] = sub.run_box(els, r);
            }
            let Some(st) = tips_of else { continue };
            // A joint's tip changes with its segments and with which segment with length comes
            // next: the runs of the moved segments and of the segments with length before them.
            let mut runs = vec![];
            let mut whole = all;
            for &i in &segs {
                runs.push(i / RUN);
                match sub.prev_drawn(els, i) {
                    Ok(Some(j)) => runs.push(j / RUN),
                    Ok(None) => {}
                    Err(()) => whole = true,
                }
            }
            if whole {
                runs = (0..sub.runs()).collect();
            }
            runs.sort_unstable();
            runs.dedup();
            for r in runs {
                sub.tips[r] = sub.run_tips(els, r, st);
            }
        }
        true
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

    /// The boxes and tips kept per run of each subpath, and the stroke the tips were measured for.
    #[cfg(test)]
    pub(crate) fn runs(&self) -> (Vec<RunBoxes>, Option<StrokeLayer>) {
        (self.parts.iter().flatten().map(|s| (s.boxes.clone(), s.tips.clone())).collect(), self.tips_of.clone())
    }
}
