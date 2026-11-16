//! Snap to Tangent for a line drawn from one curve to another (the Line Segment tool): when the
//! press lands on a convex closed path with curves and the pointer comes near another one, both
//! ends of the line move onto a common tangent of the two, the start sliding along the first path.
//! The tangents are [`vectorcraft_pathops::addons::pathtools::common_tangents`]'s.
//!
//! The closed subpaths in the window the line may end on are listed once, at the press, by their
//! bounds, up to as many segments as Smart Guides gather. Each pointer move looks only at the
//! [`MAX_NEAR`] nearest of them whose outline lies within the snapping tolerance of the pointer.
//! The tangents of each pair are worked out once, and the last [`MAX_KEPT`] pairs' are kept for
//! the rest of the drag.

use std::collections::HashSet;

use vectorcraft_pathops::addons::pathtools::{Outline, Tangent, common_tangents};

use super::snap::visible;
use super::targets::{BUDGET, walk_but};
use super::{Node, NodeId, NodeKind, Overlay, ParamCurveNearest, Point, Rect, SubPath, ToolContext};
use crate::PixelOp;

/// Most subpaths a pointer move takes tangents from: the nearest ones whose outline lies within
/// the snapping tolerance of the pointer.
const MAX_NEAR: usize = 16;
/// Most pairs whose tangents are kept during one drag.
const MAX_KEPT: usize = 64;
/// Most segments a subpath the line starts or ends on may have: the tangents of two paths take
/// time that grows with both paths' segment counts, and they are worked out during the drag.
const MAX_SEGMENTS: usize = 512;

/// A closed subpath with a curve the line may end on: its path, its index in the path, and the
/// bounds of its control points.
#[derive(Clone, Copy)]
struct Listed {
    key: (NodeId, usize),
    bounds: Rect,
}

/// The two-ended tangent snap of one Line Segment drag.
#[derive(Default)]
pub(crate) struct TangentLine {
    /// The subpath the press landed on and its outline: none when the press was not on a convex
    /// closed path with a curve, or the snap is off.
    from: Option<((NodeId, usize), Outline)>,
    /// The closed subpaths with a curve, listed at the press (none when it found no subpath to
    /// start from).
    listed: Vec<Listed>,
    /// The common tangents of the press's subpath and each subpath the pointer came near.
    kept: Vec<((NodeId, usize), Vec<Tangent>)>,
    /// The line of the last snap.
    line: Option<(Point, Point)>,
}

/// The snap applies: Smart Guides and Snap to Tangent on, and neither pixels nor the grid
/// snapping drawn points instead.
fn applies(cx: &ToolContext) -> bool {
    cx.smart_guides && cx.snap_tangent && !cx.snaps_to_pixel(PixelOp::Drawing) && !cx.snap_to_grid
}

/// The subpath `key` of the document in `cx`.
fn subpath<'a>(cx: &'a ToolContext, key: (NodeId, usize)) -> Option<&'a SubPath> {
    match &cx.doc.node(key.0)?.kind {
        NodeKind::Path { path, .. } => path.subpaths.get(key.1),
        _ => None,
    }
}

/// The subpaths of path `id` a line may start or end on, with their indexes: closed, with at most
/// [`MAX_SEGMENTS`] segments.
fn closed_subpaths<'a>(cx: &'a ToolContext, id: NodeId) -> impl Iterator<Item = (usize, &'a SubPath)> {
    let subpaths = match cx.doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Path { path, .. }) => path.subpaths.as_slice(),
        _ => &[],
    };
    subpaths.iter().enumerate().filter(|(_, sp)| sp.closed && sp.segment_count() <= MAX_SEGMENTS)
}

/// How far `p` lies from `sp` (its segments whose control bounds come within `reach` of `p`); none
/// when no segment comes that near.
fn distance(sp: &SubPath, p: Point, reach: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for k in 0..sp.segment_count() {
        let c = sp.segment(k);
        let b = Rect::from_points(c.p0, c.p3).union_pt(c.p1).union_pt(c.p2);
        if !b.inflate(reach, reach).contains(p) {
            continue;
        }
        let d = c.nearest(p, 1e-9).distance_sq.sqrt();
        if best.is_none_or(|e| d < e) {
            best = Some(d);
        }
    }
    best
}

/// Whether `sp` has a curved segment.
fn curved(sp: &SubPath) -> bool {
    (0..sp.segment_count()).any(|k| !sp.segment_is_line(k))
}

impl TangentLine {
    /// Start a drag pressed at `start` (where the pointer went down): the subpath under it, when it
    /// is a convex closed path with a curve, and then the closed subpaths with a curve in the window
    /// the line may end on, by their bounds. The press looks at the segments of the paths whose
    /// bounds reach it only, and lists nothing when it lands on no such subpath, or when `held` (an
    /// anchor, center or crossing the press snapped to) lies off that subpath.
    pub(crate) fn press(&mut self, cx: &ToolContext, start: Point, held: Option<Point>) {
        *self = Self::default();
        if !applies(cx) || !start.x.is_finite() || !start.y.is_finite() {
            return;
        }
        let tol = cx.snap_tol();
        // The paths with a closed subpath, by their finite bounds.
        let closed = |n: &Node| match &n.kind {
            NodeKind::Path { path, .. } if path.subpaths.iter().any(|sp| sp.closed) => {
                n.geometric_bounds().filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()))
            }
            _ => None,
        };
        // The subpaths under the press: at most as many segments as Smart Guides gather.
        let mut on: Option<(f64, (NodeId, usize))> = None;
        let mut budget = BUDGET;
        walk_but(&cx.doc.layers, &HashSet::new(), &mut |n| {
            if budget == 0 || !closed(n).is_some_and(|b| b.inflate(tol, tol).contains(start)) {
                return;
            }
            for (si, sp) in closed_subpaths(cx, n.id) {
                let Some(left) = budget.checked_sub(sp.segment_count()) else { return };
                budget = left;
                if let Some(d) = distance(sp, start, tol)
                    && d <= tol
                    && curved(sp)
                    && on.is_none_or(|(e, _)| d < e)
                {
                    on = Some((d, (n.id, si)));
                }
            }
        });
        // A path that bends inward somewhere has no tangent of this kind: the line keeps its start.
        let Some(from) = on.and_then(|(_, key)| Some((key, Outline::convex(subpath(cx, key)?).ok()?))) else { return };
        // A press held by an anchor, center or crossing off that path starts the line there.
        if let Some(q) = held
            && !subpath(cx, from.0).and_then(|sp| distance(sp, q, tol)).is_some_and(|d| d <= 1e-6 * tol.max(1.0))
        {
            return;
        }
        let from_key = from.0;
        self.from = Some(from);
        // The subpaths the line may end on: closed ones with a curve, of the paths in the window,
        // at most as many segments as Smart Guides gather.
        let window = visible(cx);
        let listed = &mut self.listed;
        let mut budget = BUDGET;
        walk_but(&cx.doc.layers, &HashSet::new(), &mut |n| {
            if budget == 0 || !closed(n).is_some_and(|b| window.is_none_or(|v| b.intersect(v).area() > 0.0 || v.contains(b.center()))) {
                return;
            }
            for (si, sp) in closed_subpaths(cx, n.id) {
                let Some(left) = budget.checked_sub(sp.segment_count()) else { return };
                budget = left;
                let mut bounds: Option<Rect> = None;
                for k in 0..sp.segment_count() {
                    let c = sp.segment(k);
                    let b = Rect::from_points(c.p0, c.p3).union_pt(c.p1).union_pt(c.p2);
                    bounds = Some(bounds.map_or(b, |r| r.union(b)));
                }
                if let Some(bounds) =
                    bounds.filter(|b| curved(sp) && (n.id, si) != from_key && [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()))
                {
                    listed.push(Listed { key: (n.id, si), bounds });
                }
            }
        });
    }

    /// The line for the pointer at `p`: on the nearest subpath whose outline lies within the
    /// snapping tolerance of `p` and that has a common tangent with the press's subpath, the
    /// tangent whose touch point on it is nearest `p`; none when there is none. A subpath farther
    /// from `p` than `rival` (an anchor, center or crossing the pointer snaps to otherwise) gives
    /// way to it; its own anchors, which lie on its outline, do not.
    pub(crate) fn snap(&mut self, cx: &ToolContext, p: Point, rival: Option<f64>) -> Option<(Point, Point)> {
        let line = self.find(cx, p, rival);
        self.line = line;
        line
    }

    fn find(&mut self, cx: &ToolContext, p: Point, rival: Option<f64>) -> Option<(Point, Point)> {
        if self.from.is_none() || !applies(cx) || !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        let tol = cx.snap_tol();
        let mut near: Vec<(f64, (NodeId, usize))> = vec![];
        for l in self.listed.iter().filter(|l| l.bounds.inflate(tol, tol).contains(p)) {
            // The nearest `MAX_NEAR`, in order, kept while scanning.
            if let Some(d) = subpath(cx, l.key).and_then(|sp| distance(sp, p, tol)).filter(|d| *d <= tol) {
                let at = near.partition_point(|(e, _)| *e <= d);
                if at < MAX_NEAR {
                    near.insert(at, (d, l.key));
                    near.truncate(MAX_NEAR);
                }
            }
        }
        for (d, key) in near {
            if rival.is_some_and(|r| r < d - 1e-6 * tol) {
                return None;
            }
            let Some(sp) = subpath(cx, key) else { continue };
            let mut best: Option<(f64, Point, Point)> = None;
            for t in self.tangents(cx, key) {
                // Where the paths touch each other, the tangent's ends are not both on the paths.
                let on_b = distance(sp, t.to, tol).is_some_and(|e| e <= 1e-6 * tol.max(1.0));
                if !on_b || t.from.distance(t.to) <= tol {
                    continue;
                }
                let dist = t.to.distance(p);
                // On a tie the line of the last move stays, so it does not flicker.
                let same = self.line == Some((t.from, t.to));
                if best.is_none_or(|(e, ..)| dist < e - 1e-9 || (same && dist <= e + 1e-9)) {
                    best = Some((dist, t.from, t.to));
                }
            }
            if let Some((_, a, b)) = best {
                return Some((a, b));
            }
        }
        None
    }

    /// The common tangents of the press's subpath and subpath `key`, worked out on first use.
    fn tangents(&mut self, cx: &ToolContext, key: (NodeId, usize)) -> Vec<Tangent> {
        if let Some((_, t)) = self.kept.iter().find(|(k, _)| *k == key) {
            return t.clone();
        }
        let Some((_, from)) = &self.from else { return vec![] };
        let all = subpath(cx, key).and_then(|sp| Outline::convex(sp).ok()).and_then(|o| common_tangents(from, &o).ok()).unwrap_or_default();
        if self.kept.len() >= MAX_KEPT {
            self.kept.remove(0);
        }
        self.kept.push((key, all.clone()));
        all
    }

    /// The line of the last snap, if it found one.
    pub(crate) fn line(&self) -> Option<(Point, Point)> {
        self.line
    }

    /// Forget the last snap's line for a move that does not snap this way (a modifier is held),
    /// keeping the press's subpath for the moves after it.
    pub(crate) fn drop_line(&mut self) {
        self.line = None;
    }

    /// End the drag.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// The guides of the last snap: the line, and the "tangent" label at both ends.
    pub(crate) fn guides(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some((a, b)) = self.line else { return vec![] };
        let color = cx.smart_guide_color;
        let mut out = vec![];
        if cx.alignment_guides {
            out.push(Overlay::Line { a, b, color, dashed: false });
        }
        if cx.anchor_path_labels {
            out.extend([a, b].map(|p| Overlay::Label { p, text: "tangent".into(), color }));
        }
        out
    }
}
