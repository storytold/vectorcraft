//! Commands backing the drawing tools (pencil, curvature, anchor tools, scissors, knife, eraser,
//! blob brush, smooth, path eraser, join).

use std::collections::BTreeSet;
use std::sync::Arc;

use kurbo::{CubicBez, ParamCurve, ParamCurveNearest};
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, PressureProfile, Selection};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, FillRule, PathData, Point, Rect, SubPath, Vec2};
use vectorcraft_pathops as po;
use vectorcraft_pathops::{BoolOp, Cap, Join, SimplifyOptions};

use super::create::{add_look, insert_look};
use super::newart::NewArt;
use super::pathops::{Beside, remove_each};
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.freehand",
            "Pencil",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] (pen pressure 0..1: the stroke keeps it, for Calligraphic brushes with Pressure variation; a point without one presses fully), fidelity?: pt (1.5), closed?, style?: \"pencil\"|\"brush\", fill?: bool, keepSelected?: bool (true: the drawn or continued path is selected; false: it is not, and the rest of the selection stays), extend?: {id, end: \"start\"|\"end\"}, redraw?: {id, within?: pt (12)}} fit a freehand stroke → {id}. extend continues the path from that end; redraw (Edit Selected Paths) replaces the section of the path between where the stroke starts and ends, each within `within` of one subpath (the shorter way round a closed one), and draws a new path when the stroke doesn't end on it",
            has_doc,
            freehand
        ),
        cmd!(
            "path.curvature",
            "Curvature",
            [],
            None,
            "{points: [{x, y, corner?}], closed?, id?} create (or with id: replace) a path curving through the points → {id}",
            has_doc,
            curvature
        ),
        cmd!(
            "path.curvatureEdit",
            "Curvature",
            [],
            None,
            "{id, subpath?, op: \"move\"|\"insert\"|\"extend\"|\"close\", anchor?, segment?, t?, end?: \"start\"|\"end\", from?: \"keep\"|\"smooth\"|\"corner\", corner?, x?, y?} edit a path as the Curvature tool does, its shape kept elsewhere; the anchor edited becomes the direct-selected one → {anchor}. move: anchor to x, y, re-curving only its two segments (a smooth anchor takes the Curvature tool's handles, along the circle through it and its neighbours); insert: a smooth anchor at t on segment, the shape unchanged (with x, y: then moved there); extend: an open subpath goes on from its end (default) or start to a new end at x, y, curving into it unless corner is true; close: an open subpath closes from that end. The old end bends into the new segment as from says: keep its curve (default), smooth through its neighbours, or a corner",
            has_doc,
            curvature_edit
        ),
        cmd!(
            "path.removeAnchor",
            "Delete Anchor Point",
            [],
            None,
            "{id, subpath?, anchor} remove one anchor, re-fitting the curve",
            has_doc,
            remove_anchor
        ),
        cmd!(
            "path.removeAnchors",
            "Remove Anchor Points",
            ["Object", "Path"],
            None,
            "{} remove the direct-selected anchors without opening their paths, refitting the curve round each; the anchor after each removed one is selected, so running it again goes on along the path → {removedObjects}",
            has_anchors,
            remove_anchors
        ),
        cmd!(
            "path.convertAnchor",
            "Convert Anchor Point",
            [],
            None,
            "{id, subpath?, anchor, to: \"corner\"|\"smooth\", x?, y?} (smooth with x/y: out handle there, in handle mirrored; without: as the Curvature tool curves a point, along the circle through it and its neighbours, or at an open end one handle into the curve)",
            has_doc,
            convert_anchor
        ),
        cmd!(
            "path.reshapeSegment",
            "Reshape Segment",
            [],
            None,
            "{id, subpath?, segment, t, dx, dy, perpendicular?: bool} bend a segment so the point at t moves by (dx, dy); with perpendicular (Shift), both handles stand at right angles to the line between its anchors at one length, and the point at t lies as far from that line as the moved point does",
            has_doc,
            reshape_segment
        ),
        cmd!(
            "path.split",
            "Split Path",
            [],
            None,
            "{id, subpath?, segment, t} or {id, subpath?, anchor}: closed paths open there, open paths become two → {ids}",
            has_doc,
            split
        ),
        cmd!(
            "path.knife",
            "Knife",
            [],
            None,
            "{points: [[x,y]…]} cut closed shapes (selected, or all when nothing is selected) along the polyline → {ids}",
            has_doc,
            knife
        ),
        cmd!(
            "path.eraseRegion",
            "Eraser",
            [],
            None,
            "{points: [[x,y]…], size?: pt (10)} erase a round-brush stroke from paths (selected, or all) → {ids}",
            has_doc,
            erase_region
        ),
        cmd!(
            "path.eraseSegments",
            "Path Eraser",
            [],
            None,
            "{points: [[x,y]…], width?: pt (8)} erase the parts of selected paths under the drag → {ids}",
            has_selection,
            erase_segments
        ),
        cmd!(
            "path.smoothRegion",
            "Smooth",
            [],
            None,
            "{points: [[x,y]…], radius?: pt (12), fidelity?: pt (2.5)} smooth selected paths near the drag → {ids, before, after}",
            has_selection,
            smooth_region
        ),
        cmd!(
            "path.joinScrub",
            "Join",
            [],
            None,
            "{points: [[x,y]…], tolerance?: pt (12)} join the open-path endpoints scrubbed over → {id}",
            has_doc,
            join_scrub
        ),
        cmd!(
            "path.blob",
            "Blob Brush",
            [],
            None,
            "{points: [[x,y]…], size?: pt (10), merge?: true, mergeOnlySelection?: bool (false: merge with any touching same-color blob; true: only with selected ones), keepSelected?: bool (false: the drawn or merged path is not selected, and the rest of the selection stays; true: it is selected)} filled brush shape, merged with touching same-colour blobs → {id}",
            has_doc,
            blob
        ),
    ]
}

// ---------- helpers ----------

/// `[[x,y]…]` or `[{x,y}…]`; non-finite points are dropped.
fn points_param(p: &Value, key: &str, cmd: &str) -> Result<Vec<Point>> {
    Ok(samples_param(p, key, cmd)?.into_iter().map(|(pt, _)| pt).collect())
}

/// [`points_param`] with each point's pen pressure, if it has one: `[[x,y,pressure]…]` or
/// `[{x,y,pressure}…]` (0..1).
fn samples_param(p: &Value, key: &str, cmd: &str) -> Result<Vec<(Point, Option<f64>)>> {
    let a = p.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("missing array `{key}`")))?;
    let pts: Vec<(Point, Option<f64>)> = a
        .iter()
        .filter_map(|v| match v {
            Value::Array(xy) => Some((Point::new(xy.first()?.as_f64()?, xy.get(1)?.as_f64()?), xy.get(2).and_then(Value::as_f64))),
            Value::Object(_) => Some((Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?), v.get("pressure").and_then(Value::as_f64))),
            _ => None,
        })
        .filter(|(p, _)| p.x.is_finite() && p.y.is_finite())
        .collect();
    if pts.is_empty() {
        return Err(bad(cmd, "need at least one point"));
    }
    Ok(pts)
}

/// The pen pressure along a freehand stroke (positions measured along its polyline), when any of
/// its samples has one; samples without one count as full pressure, as a pen event without one.
fn stroke_pressure(samples: &[(Point, Option<f64>)]) -> Option<PressureProfile> {
    if samples.iter().all(|(_, p)| p.is_none()) {
        return None;
    }
    let mut at = 0.0;
    let mut prev: Option<Point> = None;
    let along: Vec<(f64, f64)> = samples
        .iter()
        .map(|(pt, p)| {
            at += prev.map_or(0.0, |q| q.distance(*pt));
            prev = Some(*pt);
            (at, p.unwrap_or(1.0))
        })
        .collect();
    PressureProfile::from_samples(along.into_iter().map(|(s, p)| (if at > 1e-9 { s / at } else { 0.0 }, p)))
}

/// The length of a subpath.
fn subpath_len(sp: &SubPath) -> f64 {
    use kurbo::ParamCurveArclen;
    PathData::single(sp.clone()).to_bezpath().segments().map(|s| s.arclen(0.01)).sum()
}

/// The bound, in points, on the length of a `path.reshapeSegment` delta and on the x and y of a
/// point given to `path.convertAnchor` or `path.freehand`. The blend spine commands set the same
/// bound (`object.blend.spine.reshapeSegment`, `object.blend.spine.moveAnchor`).
const MAX_INPUT: f64 = 1e9;

/// Whether `q` is finite and its x and y are each smaller than [`MAX_INPUT`] in size.
fn within_input_bound(q: Point) -> bool {
    q.x.is_finite() && q.y.is_finite() && q.x.abs() < MAX_INPUT && q.y.abs() < MAX_INPUT
}

fn usize_req(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).map(|v| v as usize).ok_or_else(|| bad(cmd, format!("missing integer `{key}`")))
}

/// The whole number of 0 or more that `key` holds, or `None` when `key` is absent or null. Any other
/// value is [`usize_req`]'s error.
pub(super) fn usize_opt(p: &Value, key: &str, cmd: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => usize_req(p, key, cmd).map(Some),
    }
}

fn path_mut(d: &mut Document, id: NodeId) -> Result<&mut PathData> {
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None;
            Ok(path)
        }
        _ => Err(EngineError::Other(format!("object {id} is not a path"))),
    }
}

fn path_of(d: &Document, id: NodeId) -> Result<(PathData, FillRule)> {
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Path { path, rule, .. }) => Ok((path.clone(), *rule)),
        Some(_) => Err(EngineError::Other(format!("object {id} is not a path"))),
        None => Err(EngineError::NoNode(id)),
    }
}

fn dist_to_polyline(pts: &[Point], q: Point) -> f64 {
    if pts.len() == 1 {
        return pts[0].distance(q);
    }
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let l2 = ab.hypot2();
            let t = if l2 < 1e-18 { 0.0 } else { ((q - a).dot(ab) / l2).clamp(0.0, 1.0) };
            (a + ab * t).distance(q)
        })
        .fold(f64::INFINITY, f64::min)
}

fn poly_bounds(pts: &[Point], pad: f64) -> Rect {
    let r = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
    r.inflate(pad, pad)
}

/// The filled area painted by a round brush of diameter `size` along `pts`.
fn brush_region(pts: &[Point], size: f64) -> PathData {
    let mut pts = pts.to_vec();
    if pts.len() == 1 {
        pts.push(pts[0] + Vec2::new(0.01, 0.0));
    }
    po::outline_stroke(&PathData::single(SubPath::polyline(&pts, false)), size, Cap::Round, Join::Round, 4.0)
}

/// Path leaves the area tools act on: under the selected objects, or every path when nothing is
/// selected. Only editable, non-guide paths whose bounds meet `area`.
fn target_paths(doc: &Document, sel: &Selection, area: Rect) -> Vec<NodeId> {
    let mut out = vec![];
    let mut push = |n: &Node| {
        if let NodeKind::Path { guide: false, path, .. } = &n.kind
            && path.control_bounds().is_some_and(|b| b.inflate(0.5, 0.5).intersect(area).area() > 0.0 || b.width() == 0.0 || b.height() == 0.0)
            && !out.contains(&n.id)
        {
            out.push(n.id);
        }
    };
    if sel.is_empty() {
        doc.walk(&mut push);
    } else {
        for id in &sel.objects {
            if let Some(n) = doc.node(*id) {
                n.walk(&mut |c: &Node| push(c));
            }
        }
    }
    out.retain(|id| doc.is_editable(*id));
    out
}

/// The members of a compound path with geometry `pd`: one path per subpath, with fresh ids.
fn compound_members(d: &mut Document, pd: PathData) -> Vec<Arc<Node>> {
    pd.subpaths.into_iter().map(|sp| Arc::new(Node::path(d.alloc_id(), PathData::single(sp), Appearance::default()))).collect()
}

/// Give path or compound path `id` the geometry `pd` (dropping a live shape; a compound's members
/// are rebuilt, one per subpath).
pub(super) fn set_geometry(d: &mut Document, id: NodeId, pd: PathData) -> Result<()> {
    if matches!(d.node(id).map(|n| &n.kind), Some(NodeKind::Compound { .. })) {
        let members = compound_members(d, pd);
        if let Some(NodeKind::Compound { children, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            *children = members;
        }
        return Ok(());
    }
    *path_mut(d, id)? = pd;
    Ok(())
}

/// A copy of path or compound path node `id` (fresh ids, no live shape) with geometry `pd`.
fn sibling_with(d: &mut Document, id: NodeId, pd: PathData) -> Result<Node> {
    let mut n = d.node(id).ok_or(EngineError::NoNode(id))?.clone();
    n.id = d.alloc_id();
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *path = pd;
            *live = None;
        }
        NodeKind::Compound { children, .. } => *children = compound_members(d, pd),
        _ => {}
    }
    Ok(n)
}

/// Replace path or compound path `id` with `pieces` (first keeps the id, the rest go right above
/// it; none removes it). `cut` removes the path or puts the other pieces above it. Returns ids.
pub(super) fn replace_with_pieces(d: &mut Document, cut: &mut Beside, id: NodeId, pieces: Vec<PathData>) -> Result<Vec<NodeId>> {
    let mut it = pieces.into_iter();
    let Some(first) = it.next() else {
        cut.put(d, id, vec![])?;
        return Ok(vec![]);
    };
    let mut ids = vec![id];
    let mut above = vec![];
    for pd in it {
        let n = sibling_with(d, id, pd)?;
        ids.push(n.id);
        above.push(n);
    }
    set_geometry(d, id, first)?;
    if !above.is_empty() {
        let n = d.node(id).cloned().ok_or(EngineError::NoNode(id))?;
        cut.put(d, id, [n].into_iter().chain(above).collect())?;
    }
    Ok(ids)
}

pub(super) fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Split a filled path into connected pieces (each outer contour with its holes).
pub(super) fn components(pd: &PathData) -> Vec<PathData> {
    let subs: Vec<&SubPath> = pd.subpaths.iter().filter(|s| s.closed && s.anchors.len() >= 2).collect();
    if subs.len() <= 1 {
        return if pd.is_empty() { vec![] } else { vec![pd.clone()] };
    }
    let bps: Vec<BezPath> = subs
        .iter()
        .map(|s| {
            let mut b = BezPath::new();
            s.to_bezpath_into(&mut b);
            b
        })
        .collect();
    let samples: Vec<Point> = subs.iter().map(|s| s.segment(0).eval(0.5)).collect();
    let n = subs.len();
    let inside = |j: usize, i: usize| fill_contains(&bps[j], FillRule::NonZero, samples[i]);
    let depth: Vec<usize> = (0..n).map(|i| (0..n).filter(|&j| j != i && inside(j, i)).count()).collect();
    let areas: Vec<f64> = subs.iter().map(|s| s.area().abs()).collect();
    let outers: Vec<usize> = (0..n).filter(|&i| depth[i].is_multiple_of(2)).collect();
    let mut groups: Vec<Vec<SubPath>> = outers.iter().map(|&o| vec![subs[o].clone()]).collect();
    for h in (0..n).filter(|&i| !depth[i].is_multiple_of(2)) {
        let best = outers
            .iter()
            .enumerate()
            .filter(|(_, o)| depth[**o] + 1 == depth[h] && inside(**o, h))
            .min_by(|a, b| areas[*a.1].total_cmp(&areas[*b.1]));
        if let Some((k, _)) = best {
            groups[k].push(subs[h].clone());
        }
    }
    groups.into_iter().map(PathData::new).collect()
}

/// The pieces of `sp` where `remove(point)` is false. `None` when nothing is removed.
fn cut_subpath(sp: &SubPath, remove: &dyn Fn(Point) -> bool) -> Option<Vec<SubPath>> {
    let nseg = sp.segment_count();
    if nseg == 0 {
        return sp.anchors.first().filter(|a| remove(a.p)).map(|_| vec![]);
    }
    const K: usize = 24;
    let mut runs: Vec<Vec<(usize, f64, f64)>> = vec![];
    let mut cur: Vec<(usize, f64, f64)> = vec![];
    let mut any = false;
    for i in 0..nseg {
        let c = sp.segment(i);
        let mut prev_t = 0.0;
        let mut prev_rm = remove(c.eval(0.0));
        let mut start = if prev_rm { None } else { Some(0.0) };
        if prev_rm {
            any = true;
            if !cur.is_empty() {
                runs.push(std::mem::take(&mut cur));
            }
        }
        for k in 1..=K {
            let t = k as f64 / K as f64;
            let rm = remove(c.eval(t));
            if rm != prev_rm {
                let (mut lo, mut hi) = (prev_t, t);
                for _ in 0..30 {
                    let m = 0.5 * (lo + hi);
                    if remove(c.eval(m)) == prev_rm { lo = m } else { hi = m }
                }
                let b = 0.5 * (lo + hi);
                if prev_rm {
                    start = Some(b);
                } else {
                    any = true;
                    if let Some(s) = start.take() {
                        cur.push((i, s, b));
                    }
                    runs.push(std::mem::take(&mut cur));
                }
            }
            prev_t = t;
            prev_rm = rm;
        }
        if let Some(s) = start {
            cur.push((i, s, 1.0));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs.retain(|r| !r.is_empty());
    if !any {
        return None;
    }
    // A closed path's last and first runs meet at anchor 0.
    if sp.closed && runs.len() >= 2 {
        let first_starts = runs.first().and_then(|r| r.first()).is_some_and(|s| s.0 == 0 && s.1 == 0.0);
        let last_ends = runs.last().and_then(|r| r.last()).is_some_and(|s| s.0 + 1 == nseg && s.2 == 1.0);
        if first_starts && last_ends {
            let first = runs.remove(0);
            if let Some(last) = runs.last_mut() {
                last.extend(first);
            }
        }
    }
    let out = runs
        .iter()
        .filter(|r| r.len() > 1 || r[0].2 - r[0].1 > 1e-4)
        .map(|r| {
            let mut anchors: Vec<Anchor> = vec![];
            for &(i, t0, t1) in r {
                let c = sp.segment(i).subsegment(t0..t1);
                let (h1, h2) = if sp.segment_is_line(i) { (c.p0, c.p3) } else { (c.p1, c.p2) };
                match anchors.last_mut() {
                    Some(a) => a.h_out = h1,
                    None => anchors.push(Anchor { h_out: h1, ..Anchor::corner(c.p0) }),
                }
                anchors.push(Anchor { p: c.p3, h_in: h2, h_out: c.p3, kind: AnchorKind::Corner });
            }
            for a in &mut anchors {
                *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
            }
            SubPath::new(anchors, false)
        })
        .filter(|s| s.anchors.len() >= 2)
        .collect();
    Some(out)
}

/// Erase from an outline path (open or closed) where `remove` holds. Returns the new geometry per
/// output object (one object per piece when the path had a single subpath), or None if unchanged.
pub(super) fn erase_outline(pd: &PathData, remove: &dyn Fn(Point) -> bool) -> Option<Vec<PathData>> {
    let mut changed = false;
    let mut subs = vec![];
    for sp in &pd.subpaths {
        match cut_subpath(sp, remove) {
            Some(pieces) => {
                changed = true;
                subs.extend(pieces);
            }
            None => subs.push(sp.clone()),
        }
    }
    if !changed {
        return None;
    }
    Some(if pd.subpaths.len() == 1 {
        subs.into_iter().map(PathData::single).collect()
    } else if subs.is_empty() {
        vec![]
    } else {
        vec![PathData::new(subs)]
    })
}

fn stroke_paint(s: &Session) -> Paint {
    if s.paint.stroke.is_none() { Paint::solid(Color::BLACK) } else { s.paint.stroke.clone() }
}

fn path_kind(path: PathData) -> NodeKind {
    NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false }
}

// ---------- Pencil / Paintbrush ----------

/// Fit a freehand polyline: drop jitter, build a spline through the samples (sharp turns stay
/// corners) and simplify it to `tol`.
fn fit_freehand(pts: &[Point], tol: f64, closed: bool) -> SubPath {
    let min = (tol * 0.3).max(0.05);
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

/// Pencil's Edit Selected Paths in the middle of a path: a stroke from one point of a subpath of
/// `pd` to another, each within `within` of it, replaces the section between them (the shorter
/// way round a closed subpath) with the stroke fitted at `tol`. The subpath's index and its new
/// shape; None when the stroke doesn't start and end on the same subpath, or meets it once.
fn redraw_section(pd: &PathData, pts: &[Point], tol: f64, within: f64) -> Option<(usize, SubPath)> {
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
    let fit = fit_freehand(&stroke, tol, false).anchors;
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

fn freehand(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.freehand";
    let samples = samples_param(p, "points", C)?;
    if samples.iter().any(|(q, _)| !within_input_bound(*q)) {
        return Err(bad(C, "x and y must be finite"));
    }
    let pressure = stroke_pressure(&samples);
    let mut pts: Vec<Point> = samples.into_iter().map(|(pt, _)| pt).collect();
    let tol = f64_or(p, "fidelity", 1.5).clamp(0.05, 100.0);
    let closed = bool_or(p, "closed", false);
    let brush = str_param(p, "style") == Some("brush");
    let keep_selected = bool_or(p, "keepSelected", true);
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
        let close_tol = (tol * 4.0).max(6.0);
        let closing = sp.anchors.len() >= 2 && pts.last().is_some_and(|p| p.distance(first) <= close_tol);
        if closing && let Some(l) = pts.last_mut() {
            *l = first;
        }
        let fit = fit_freehand(&pts, tol, false);
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
        // The path's pressure (in the direction it is continued in) followed by the stroke's.
        let old_pressure = s.doc()?.doc.node(id).and_then(|n| n.appearance.stroke()).and_then(|st| st.pressure.clone());
        let joined = (old_pressure.is_some() || pressure.is_some()).then(|| {
            let old = old_pressure.map(|p| if at_start { p.reversed() } else { p });
            let total = subpath_len(&sp);
            PressureProfile::extended(old.as_ref(), old_len, pressure.as_ref(), total - old_len)
        });
        s.edit(if brush { "Paintbrush" } else { "Pencil" }, |d, sel| {
            let reverse = at_start && !sp.closed;
            if reverse {
                sp.reverse();
            }
            path_mut(d, id)?.subpaths[si] = sp;
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
        let within = f64_or(rd, "within", 12.0);
        if !within.is_finite() || within < 0.0 {
            return Err(bad(C, "redraw within must be a finite distance of 0 or more"));
        }
        let (pd, _) = path_of(&s.doc()?.doc, id)?;
        // shortcut: a stroke pressure profile is left as it was, measured along the old path; fit
        // it to the new length if Calligraphic pressure strokes get redrawn.
        if let Some((si, sp)) = redraw_section(&pd, &pts, tol, within) {
            s.edit(if brush { "Paintbrush" } else { "Pencil" }, |d, sel| {
                *path_mut(d, id)?.subpaths.get_mut(si).ok_or_else(|| bad(C, "no such subpath"))? = sp;
                if keep_selected {
                    sel.set([id]);
                } else {
                    sel.remove(id);
                }
                Ok(())
            })?;
            return Ok(json!({ "id": id.0 }));
        }
    }
    if pts.len() < 2 {
        return Err(bad(C, "need at least two points"));
    }
    let sp = fit_freehand(&pts, tol, closed);
    let fill = if bool_or(p, "fill", false) { s.paint.fill.clone() } else { Paint::None };
    // A pencil path with a fill keeps a stroke of None (#1679); only one that would show nothing
    // gets black. A brush is its stroke.
    let stroke = if brush || fill.is_none() { stroke_paint(s) } else { s.paint.stroke.clone() };
    let mut look = s.new_art_look(fill, stroke, s.paint.stroke_width.max(0.1));
    if let Some(st) = look.appearance.stroke_mut() {
        st.pressure = pressure;
    }
    insert_look(s, if brush { "Paintbrush" } else { "Pencil" }, path_kind(PathData::single(sp)), look, None, keep_selected)
}

// ---------- Curvature ----------

fn curvature(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.curvature";
    let arr = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(C, "missing points"))?;
    let pts: Vec<(Point, bool)> = arr
        .iter()
        .filter_map(|v| {
            let pt = match v {
                Value::Array(xy) => Point::new(xy.first()?.as_f64()?, xy.get(1)?.as_f64()?),
                _ => Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?),
            };
            (pt.x.is_finite() && pt.y.is_finite()).then(|| (pt, v.get("corner").and_then(Value::as_bool).unwrap_or(false)))
        })
        .collect();
    if pts.is_empty() {
        return Err(bad(C, "need at least one point"));
    }
    let sp = vectorcraft_tools::draw2::curvature_spline(&pts, bool_or(p, "closed", false));
    if let Some(id) = id_param(p, "id") {
        s.edit("Curvature", |d, sel| {
            *path_mut(d, id)? = PathData::single(sp);
            sel.set([id]);
            Ok(())
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    let mut look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let ap = &mut look.appearance;
    if ap.stroke_paint().is_none() && ap.fill_paint().is_none() {
        ap.set_stroke(Paint::solid(Color::BLACK));
    }
    add_look(s, "Curvature", path_kind(PathData::single(sp)), look, None)
}

/// An edit `path.curvatureEdit` makes.
enum CurvatureOp {
    Move(usize, Point),
    Insert(usize, f64, Option<Point>),
    Extend(Point),
    Close,
}

fn curvature_edit(s: &mut Session, p: &Value) -> Result<Value> {
    use vectorcraft_tools::draw2::{EndCurve, curvature_close, curvature_extend, curvature_insert, curvature_move};
    const C: &str = "path.curvatureEdit";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let at = match (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some(Point::new(x, y)),
        (None, None) => None,
        _ => return Err(bad(C, "x and y must be finite numbers")),
    };
    let to = || at.ok_or_else(|| bad(C, "missing x, y"));
    let start = match str_param(p, "end") {
        None | Some("end") => false,
        Some("start") => true,
        Some(e) => return Err(bad(C, format!("unknown end {e:?}"))),
    };
    let how = match str_param(p, "from") {
        None => EndCurve::Keep,
        Some(f) => EndCurve::parse(f).ok_or_else(|| bad(C, format!("unknown from {f:?}")))?,
    };
    let corner = bool_or(p, "corner", false);
    let op = match str_param(p, "op") {
        Some("move") => CurvatureOp::Move(usize_req(p, "anchor", C)?, to()?),
        Some("insert") => CurvatureOp::Insert(usize_req(p, "segment", C)?, f64_or(p, "t", 0.5), at),
        Some("extend") => CurvatureOp::Extend(to()?),
        Some("close") => CurvatureOp::Close,
        Some(o) => return Err(bad(C, format!("unknown op {o:?}"))),
        None => return Err(bad(C, "missing op")),
    };
    let anchor = s.edit("Curvature", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if matches!(op, CurvatureOp::Extend(_) | CurvatureOp::Close) && sp.closed {
            return Err(EngineError::Other("path is already closed".into()));
        }
        let ai = match op {
            CurvatureOp::Move(ai, q) => curvature_move(sp, ai, q).then_some(ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?,
            CurvatureOp::Insert(seg, t, q) => {
                let ai = curvature_insert(sp, seg, t).ok_or_else(|| EngineError::Other("no such segment".into()))?;
                if let Some(q) = q {
                    curvature_move(sp, ai, q);
                }
                ai
            }
            CurvatureOp::Extend(q) => {
                curvature_extend(sp, start, q, how, corner);
                if start { 0 } else { sp.anchors.len() - 1 }
            }
            CurvatureOp::Close => {
                if sp.anchors.len() < 3 {
                    return Err(EngineError::Other("closing needs three anchors".into()));
                }
                curvature_close(sp, start, how);
                if start { sp.anchors.len() - 1 } else { 0 }
            }
        };
        sel.set([id]);
        sel.anchors.insert(id, [(si, ai)].into());
        Ok(ai)
    })?;
    Ok(json!({ "anchor": anchor }))
}

// ---------- anchor tools ----------

fn remove_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.removeAnchor";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let ai = usize_req(p, "anchor", C)?;
    let removed = s.edit("Delete Anchor Point", |d, sel| {
        let sp = path_mut(d, id)?.subpaths.get(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if ai >= sp.anchors.len() {
            return Err(EngineError::Other("no such anchor".into()));
        }
        let mut emptied = Beside::replacing(d, &[id]);
        let removed = remove_anchors_of(d, sel, &mut emptied, id, [(si, ai)])?;
        emptied.finish(d)?;
        Ok(removed)
    })?;
    Ok(json!({ "removedObject": removed }))
}

fn remove_anchors(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let jobs: Vec<(NodeId, Vec<(usize, usize)>)> = st
        .selection
        .anchors
        .iter()
        .filter(|(id, set)| !set.is_empty() && st.doc.node(**id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. })))
        .map(|(id, set)| (*id, set.iter().copied().collect()))
        .collect();
    if jobs.is_empty() {
        return Err(bad("path.removeAnchors", "direct-select anchor points of a path"));
    }
    let removed = s.edit("Remove Anchor Points", |d, sel| {
        let mut removed = 0;
        let mut emptied = Beside::replacing(d, &jobs.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, anchors) in jobs {
            let shape: Vec<(usize, bool)> =
                d.node(id).and_then(Node::path_data).map(|p| p.subpaths.iter().map(|sp| (sp.anchors.len(), sp.closed)).collect()).unwrap_or_default();
            let next = following_anchors(&shape, &anchors);
            if remove_anchors_of(d, sel, &mut emptied, id, anchors)? {
                removed += 1;
            } else if !next.is_empty() {
                // The next anchor along is selected, so the next Remove Anchor Points goes on from
                // there without picking it first (#2241).
                sel.anchors.insert(id, next);
            }
        }
        emptied.finish(d)?;
        Ok(removed)
    })?;
    Ok(json!({ "removedObjects": removed }))
}

/// Where the selection goes once Remove Anchor Points takes `removed` (subpath, anchor) out of a
/// path whose subpaths have `shape` (anchor count, closed): in each subpath, the anchor after its
/// last removed one, round the end of a closed subpath, or the one before it at an open subpath's
/// end. Indexes are those after the removal, without the subpaths left with fewer than two anchors.
fn following_anchors(shape: &[(usize, bool)], removed: &[(usize, usize)]) -> BTreeSet<(usize, usize)> {
    let mut next = BTreeSet::new();
    let mut kept = 0;
    for (si, &(n, closed)) in shape.iter().enumerate() {
        let gone: BTreeSet<usize> = removed.iter().filter(|&&(s, a)| s == si && a < n).map(|&(_, a)| a).collect();
        if n - gone.len() < 2 {
            continue;
        }
        if let Some(&last) = gone.last() {
            let wrap = if closed { 0..last } else { 0..0 };
            let after = (last + 1..n).chain(wrap).find(|a| !gone.contains(a));
            if let Some(a) = after.or_else(|| (0..last).rev().find(|a| !gone.contains(a))) {
                next.insert((kept, a - gone.range(..a).count()));
            }
        }
        kept += 1;
    }
    next
}

/// Remove `anchors` (subpath, anchor) of path `id` without opening it, refitting the curve round
/// each one. A subpath left with one anchor is removed, and a path left with no subpath is removed
/// through `emptied`. True when the path went.
fn remove_anchors_of(
    d: &mut Document,
    sel: &mut Selection,
    emptied: &mut Beside,
    id: NodeId,
    anchors: impl IntoIterator<Item = (usize, usize)>,
) -> Result<bool> {
    let mut anchors: Vec<(usize, usize)> = anchors.into_iter().collect();
    // The last first, so the indexes still to come stay valid.
    anchors.sort_unstable_by(|a, b| b.cmp(a));
    anchors.dedup();
    let path = path_mut(d, id)?;
    for (si, ai) in anchors {
        if let Some(sp) = path.subpaths.get_mut(si) {
            po::remove_anchor(sp, ai);
        }
    }
    path.subpaths.retain(|sp| sp.anchors.len() >= 2);
    let gone = path.is_empty();
    if gone {
        emptied.put(d, id, vec![])?;
        sel.remove(id);
    } else {
        sel.anchors.remove(&id);
        sel.segments.remove(&id);
    }
    Ok(gone)
}

fn convert_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.convertAnchor";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let ai = usize_req(p, "anchor", C)?;
    let smooth = str_param(p, "to") == Some("smooth");
    let target = match (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        (Some(x), Some(y)) => Some(Point::new(x, y)),
        _ => None,
    };
    if target.is_some_and(|q| !within_input_bound(q)) {
        return Err(bad(C, "x and y must be finite"));
    }
    s.edit("Convert Anchor Point", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if smooth && target.is_none() {
            if ai >= sp.anchors.len() {
                return Err(EngineError::Other("no such anchor".into()));
            }
            // Without a handle position the anchor curves as the Curvature tool curves a point; one
            // with no direction to take stays as it is.
            vectorcraft_tools::draw2::curvature_smooth(sp, ai);
            return Ok(());
        }
        let a = sp.anchors.get_mut(ai).ok_or_else(|| EngineError::Other("no such anchor".into()))?;
        match target {
            Some(t) if smooth && t.distance(a.p) > 1e-9 => *a = Anchor::smooth(a.p, t),
            _ => a.retract(),
        }
        Ok(())
    })?;
    ok()
}

fn reshape_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.reshapeSegment";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let seg = usize_req(p, "segment", C)?;
    let t = f64_or(p, "t", 0.5).clamp(0.05, 0.95);
    let dv = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    if !(dv.x.is_finite() && dv.y.is_finite() && dv.hypot() < MAX_INPUT) {
        return Err(bad(C, "invalid delta"));
    }
    let perpendicular = bool_or(p, "perpendicular", false);
    s.edit("Reshape", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        let bent = if perpendicular { sp.bend_segment_perpendicular(seg, t, dv) } else { sp.bend_segment(seg, t, dv) };
        if !bent {
            return Err(EngineError::Other("no such segment".into()));
        }
        Ok(())
    })?;
    ok()
}

// ---------- Scissors ----------

fn split(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.split";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let anchor = usize_opt(p, "anchor", C)?;
    let seg = usize_opt(p, "segment", C)?;
    if anchor.is_none() && seg.is_none() {
        return Err(bad(C, "need `anchor` or `segment` + `t`"));
    }
    let t = f64_or(p, "t", 0.5);
    if !t.is_finite() || p.get("t").is_some_and(|v| !v.is_f64() && !v.is_i64() && !v.is_u64()) {
        return Err(bad(C, "invalid t"));
    }
    let ids = s.edit("Split Path", |d, sel| {
        let path = path_mut(d, id)?;
        let mut sp = path.subpaths.get(si).cloned().ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        let n = sp.anchors.len();
        let k = match (anchor, seg) {
            (Some(a), _) if a < n => a,
            (Some(_), _) => return Err(EngineError::Other("no such anchor".into())),
            (None, Some(g)) if g < sp.segment_count() => {
                if t <= 1e-6 {
                    g
                } else if t >= 1.0 - 1e-6 {
                    (g + 1) % n
                } else {
                    sp.insert_anchor(g, t)
                }
            }
            _ => return Err(EngineError::Other("no such segment".into())),
        };
        let mut pieces = sp.cut_at(&std::collections::BTreeSet::from([k]));
        if sp.closed {
            path.subpaths[si] = pieces.pop().ok_or_else(|| EngineError::Other("no such anchor".into()))?;
            sel.set([id]);
            return Ok(vec![id]);
        }
        let [left, right] = <[SubPath; 2]>::try_from(pieces).map_err(|_| EngineError::Other("cannot split an open path at its end point".into()))?;
        if path.subpaths.len() == 1 {
            path.subpaths[0] = left;
            let node = sibling_with(d, id, PathData::single(right))?;
            let (parent, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
            let nid = d.insert(parent, idx + 1, node)?;
            sel.set([id, nid]);
            Ok(vec![id, nid])
        } else {
            path.subpaths[si] = left;
            path.subpaths.insert(si + 1, right);
            sel.set([id]);
            Ok(vec![id])
        }
    })?;
    Ok(ids_json(&ids))
}

// ---------- Knife / Eraser ----------

/// Width of the slit the knife leaves between pieces.
const KNIFE_GAP: f64 = 0.05;

fn knife(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.knife";
    let pts = points_param(p, "points", C)?;
    if pts.len() < 2 {
        return Err(bad(C, "need at least two points"));
    }
    let st = s.doc()?;
    let strip = po::outline_stroke(&PathData::single(SubPath::polyline(&pts, false)), KNIFE_GAP, Cap::Butt, Join::Round, 4.0);
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, 1.0)) {
        let (pd, rule) = path_of(&st.doc, id)?;
        if !pd.is_closed() {
            continue;
        }
        let pieces = components(&po::boolean(&pd, rule, &strip, FillRule::NonZero, BoolOp::Difference));
        if pieces.len() >= 2 {
            plan.push((id, pieces));
        }
    }
    if plan.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let ids = s.edit("Knife", |d, sel| {
        let mut all = vec![];
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, &mut cut, id, pieces)?);
        }
        cut.finish(d)?;
        sel.set(all.iter().copied());
        Ok(all)
    })?;
    Ok(ids_json(&ids))
}

fn erase_region(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.eraseRegion";
    let pts = points_param(p, "points", C)?;
    let size = f64_or(p, "size", 10.0);
    if !(size.is_finite() && size > 0.0) {
        return Err(bad(C, "size must be positive"));
    }
    let r = size / 2.0;
    let st = s.doc()?;
    let region = brush_region(&pts, size);
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, r)) {
        let (pd, rule) = path_of(&st.doc, id)?;
        if pd.is_closed() {
            if po::area(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Intersect), FillRule::NonZero) <= 1e-6 {
                continue;
            }
            plan.push((id, components(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Difference))));
        } else if let Some(pieces) = erase_outline(&pd, &|q| dist_to_polyline(&pts, q) <= r) {
            plan.push((id, pieces));
        }
    }
    apply_plan(s, "Eraser", plan)
}

fn apply_plan(s: &mut Session, label: &str, plan: Vec<(NodeId, Vec<PathData>)>) -> Result<Value> {
    if plan.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let ids = s.edit(label, |d, sel| {
        let mut all = vec![];
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, &mut cut, id, pieces)?);
        }
        cut.finish(d)?;
        sel.set(all.iter().copied());
        Ok(all)
    })?;
    Ok(ids_json(&ids))
}

fn erase_segments(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.eraseSegments";
    let pts = points_param(p, "points", C)?;
    let w = f64_or(p, "width", 8.0);
    if !(w.is_finite() && w > 0.0) {
        return Err(bad(C, "width must be positive"));
    }
    let st = s.doc()?;
    let mut plan = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, w)) {
        let (pd, _) = path_of(&st.doc, id)?;
        if let Some(pieces) = erase_outline(&pd, &|q| dist_to_polyline(&pts, q) <= w / 2.0) {
            plan.push((id, pieces));
        }
    }
    apply_plan(s, "Path Eraser", plan)
}

// ---------- Smooth ----------

/// Refit the anchors of an open run `a[lo..=hi]` keeping its end points and outer handles.
fn refit_run(anchors: &[Anchor], tol: f64) -> Vec<Anchor> {
    let piece = PathData::single(SubPath::new(anchors.to_vec(), false));
    let fitted = po::simplify_with(&piece, &SimplifyOptions { tolerance: tol, corner_angle_deg: 179.0, straight_lines: false });
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

fn smooth_subpath(sp: &SubPath, near: &dyn Fn(Point) -> bool, tol: f64) -> Option<SubPath> {
    let n = sp.anchors.len();
    let marked: Vec<bool> = sp.anchors.iter().map(|a| near(a.p)).collect();
    if n < 3 || !marked.iter().any(|m| *m) {
        return None;
    }
    if sp.closed {
        let Some(k) = marked.iter().position(|m| !m) else {
            // Everything is in range: smooth the whole closed path.
            let f =
                po::simplify_with(&PathData::single(sp.clone()), &SimplifyOptions { tolerance: tol, corner_angle_deg: 179.0, straight_lines: false });
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
        let mut res = smooth_open(&open.anchors, &m, near, tol);
        let l = res.pop()?;
        if let Some(r0) = res.first_mut() {
            r0.h_in = l.h_in;
        }
        return Some(SubPath::new(res, true));
    }
    Some(SubPath::new(smooth_open(&sp.anchors, &marked, near, tol), false))
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

/// `anchors` with each run of `marked` ones refitted, together with the stretch of the segments
/// leading into the run that is in the drag's reach (`near`): a short drag smooths what is under
/// it, and the rest of a long segment next to it stays as it was (#1512).
fn smooth_open(anchors: &[Anchor], marked: &[bool], near: &dyn Fn(Point) -> bool, tol: f64) -> Vec<Anchor> {
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
        if hi - lo < 2 {
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
        let new = refit_run(&piece, tol);
        out.splice(lo..=hi, head.into_iter().chain(new).chain(tail));
    }
    out
}

fn smooth_region(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.smoothRegion";
    let pts = points_param(p, "points", C)?;
    let radius = f64_or(p, "radius", 12.0).max(0.0);
    let tol = f64_or(p, "fidelity", 2.5).clamp(0.05, 100.0);
    let st = s.doc()?;
    let near = |q: Point| dist_to_polyline(&pts, q) <= radius;
    let mut plan = vec![];
    let (mut before, mut after) = (0, 0);
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, radius)) {
        let (pd, _) = path_of(&st.doc, id)?;
        let mut changed = false;
        let subs: Vec<SubPath> = pd
            .subpaths
            .iter()
            .map(|sp| match smooth_subpath(sp, &near, tol) {
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
    s.edit("Smooth", |d, _| {
        for (id, np) in plan {
            *path_mut(d, id)? = np;
        }
        Ok(())
    })?;
    let mut v = ids_json(&ids);
    v["before"] = json!(before);
    v["after"] = json!(after);
    Ok(v)
}

// ---------- Join tool ----------

fn join_scrub(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.joinScrub";
    let pts = points_param(p, "points", C)?;
    let tol = f64_or(p, "tolerance", 12.0).max(0.0);
    let st = s.doc()?;
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
    let mut ends: Vec<(f64, NodeId, usize, bool, Point)> = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, tol)) {
        let (pd, _) = path_of(&st.doc, id)?;
        for (si, sp) in pd.subpaths.iter().enumerate() {
            if sp.closed || sp.anchors.len() < 2 {
                continue;
            }
            for (start, a) in [(true, sp.anchors[0].p), (false, sp.anchors[sp.anchors.len() - 1].p)] {
                if dist_to_polyline(&pts, a) <= tol {
                    ends.push((order(a), id, si, start, a));
                }
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

// ---------- Blob Brush ----------

fn blob(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.blob";
    let pts = points_param(p, "points", C)?;
    let size = f64_or(p, "size", 10.0);
    if !(size.is_finite() && size > 0.0) {
        return Err(bad(C, "size must be positive"));
    }
    let region = brush_region(&pts, size);
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
    let keep_selected = bool_or(p, "keepSelected", false);
    let only_selection = bool_or(p, "mergeOnlySelection", false);
    let st = s.doc()?;
    let mut merge_ids = vec![];
    if bool_or(p, "merge", true) && !(only_selection && st.selection.is_empty()) {
        let all = Selection::default();
        let from = if only_selection { &st.selection } else { &all };
        for id in target_paths(&st.doc, from, poly_bounds(&pts, size)) {
            let Some(n) = st.doc.node(id) else { continue };
            if n.appearance.fill_paint() != paint || !n.appearance.stroke_paint().is_none() {
                continue;
            }
            let (pd, rule) = path_of(&st.doc, id)?;
            if pd.is_closed() && po::area(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Intersect), FillRule::NonZero) > 1e-6 {
                merge_ids.push((id, pd, rule));
            }
        }
    }
    let ap = NewArt::plain(Appearance::basic(paint, Paint::None, 1.0));
    if merge_ids.is_empty() {
        return insert_look(s, "Blob Brush", path_kind(region), ap, None, keep_selected);
    }
    let mut inputs: Vec<(&PathData, FillRule)> = vec![(&region, FillRule::NonZero)];
    inputs.extend(merge_ids.iter().map(|(_, pd, r)| (pd, *r)));
    let merged = po::unite_all(&inputs);
    let Some(((keep, _, _), rest)) = merge_ids.split_last() else {
        return insert_look(s, "Blob Brush", path_kind(region), ap, None, keep_selected);
    };
    let keep = *keep;
    let others: Vec<NodeId> = rest.iter().map(|m| m.0).collect();
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
