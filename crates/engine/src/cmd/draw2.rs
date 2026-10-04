//! Commands backing the drawing tools (pencil, curvature, anchor tools, scissors, knife, eraser,
//! blob brush, smooth, path eraser, join).

use kurbo::{ParamCurve, ParamCurveNearest};
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, FillRule, PathData, Point, Rect, SubPath, Vec2};
use vectorcraft_pathops as po;
use vectorcraft_pathops::{BoolOp, Cap, Join, SimplifyOptions};

use super::create::add_node;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.freehand",
            "Pencil",
            [],
            None,
            "{points: [[x,y]…], fidelity?: pt (1.5), closed?, style?: \"pencil\"|\"brush\", fill?: bool, extend?: {id, end: \"start\"|\"end\"}} fit a freehand stroke → {id}",
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
            "path.removeAnchor",
            "Delete Anchor Point",
            [],
            None,
            "{id, subpath, anchor} remove one anchor, re-fitting the curve",
            has_doc,
            remove_anchor
        ),
        cmd!(
            "path.convertAnchor",
            "Convert Anchor Point",
            [],
            None,
            "{id, subpath, anchor, to: \"corner\"|\"smooth\", x?, y?} (smooth with x/y: out handle there, in handle mirrored)",
            has_doc,
            convert_anchor
        ),
        cmd!(
            "path.reshapeSegment",
            "Reshape Segment",
            [],
            None,
            "{id, subpath, segment, t, dx, dy} bend a segment so the point at t moves by (dx, dy)",
            has_doc,
            reshape_segment
        ),
        cmd!(
            "path.split",
            "Split Path",
            [],
            None,
            "{id, subpath, segment, t} or {id, subpath, anchor}: closed paths open there, open paths become two → {ids}",
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
            "{points: [[x,y]…], size?: pt (10), merge?: true} filled brush shape, merged with touching same-colour blobs → {id}",
            has_doc,
            blob
        ),
    ]
}

// ---------- helpers ----------

/// `[[x,y]…]` or `[{x,y}…]`; non-finite points are dropped.
fn points_param(p: &Value, key: &str, cmd: &str) -> Result<Vec<Point>> {
    let a = p.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("missing array `{key}`")))?;
    let pts: Vec<Point> = a
        .iter()
        .filter_map(|v| match v {
            Value::Array(xy) => Some(Point::new(xy.first()?.as_f64()?, xy.get(1)?.as_f64()?)),
            Value::Object(_) => Some(Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?)),
            _ => None,
        })
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .collect();
    if pts.is_empty() {
        return Err(bad(cmd, "need at least one point"));
    }
    Ok(pts)
}

fn usize_req(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).map(|v| v as usize).ok_or_else(|| bad(cmd, format!("missing integer `{key}`")))
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

/// A copy of path node `id` (fresh id, no live shape) with geometry `pd`.
fn sibling_with(d: &mut Document, id: NodeId, pd: PathData) -> Result<Node> {
    let mut n = d.node(id).ok_or(EngineError::NoNode(id))?.clone();
    n.id = d.alloc_id();
    if let NodeKind::Path { path, live, .. } = &mut n.kind {
        *path = pd;
        *live = None;
    }
    Ok(n)
}

/// Replace path `id` with `pieces` (first keeps the id, the rest go right above it). Returns ids.
fn replace_with_pieces(d: &mut Document, id: NodeId, pieces: Vec<PathData>) -> Result<Vec<NodeId>> {
    let mut it = pieces.into_iter();
    let Some(first) = it.next() else {
        d.remove(id)?;
        return Ok(vec![]);
    };
    let (parent, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    let mut ids = vec![id];
    for (k, pd) in it.enumerate() {
        let n = sibling_with(d, id, pd)?;
        ids.push(d.insert(parent, idx + 1 + k, n)?);
    }
    *path_mut(d, id)? = first;
    Ok(ids)
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Split a filled path into connected pieces (each outer contour with its holes).
fn components(pd: &PathData) -> Vec<PathData> {
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
        let first_starts = runs[0][0].0 == 0 && runs[0][0].1 == 0.0;
        let last_ends = runs.last().and_then(|r| r.last()).is_some_and(|l| l.0 == nseg - 1 && l.2 == 1.0);
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
                if anchors.is_empty() {
                    anchors.push(Anchor::corner(c.p0));
                }
                if let Some(a) = anchors.last_mut() {
                    a.h_out = h1;
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
fn erase_outline(pd: &PathData, remove: &dyn Fn(Point) -> bool) -> Option<Vec<PathData>> {
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
    let Some(&p0) = pts.first() else {
        return SubPath::new(vec![], closed);
    };
    let mut kept: Vec<Point> = vec![p0];
    for &p in pts.iter().skip(1) {
        if kept.last().is_some_and(|k| k.distance(p) >= min) {
            kept.push(p);
        }
    }
    if kept.len() > 1
        && let (Some(&l), Some(k)) = (pts.last(), kept.last_mut())
    {
        *k = l;
    } else if kept.len() == 1 && pts.len() > 1 {
        kept.push(pts[pts.len() - 1]);
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

fn freehand(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.freehand";
    let mut pts = points_param(p, "points", C)?;
    let tol = f64_or(p, "fidelity", 1.5).clamp(0.05, 100.0);
    let closed = bool_or(p, "closed", false);
    let brush = str_param(p, "style") == Some("brush");
    if let Some(ext) = p.get("extend").filter(|v| !v.is_null()) {
        let id = id_param(ext, "id").ok_or_else(|| bad(C, "extend needs id"))?;
        let at_start = str_param(ext, "end") == Some("start");
        let (pd, _) = path_of(&s.doc()?.doc, id)?;
        let si = pd.subpaths.iter().rposition(|s| !s.closed && !s.anchors.is_empty()).ok_or_else(|| bad(C, "path has no open end"))?;
        let mut sp = pd.subpaths[si].clone();
        if at_start {
            sp.reverse();
        }
        let end = sp.anchors.last().map(|a| a.p).ok_or_else(|| bad(C, "path has no open end"))?;
        pts[0] = end;
        if pts.len() < 2 {
            return Ok(json!({ "id": id.0 }));
        }
        let first = sp.anchors[0].p;
        let close_tol = (tol * 4.0).max(6.0);
        let closing = sp.anchors.len() >= 2 && pts.last().is_some_and(|q| q.distance(first) <= close_tol);
        if closing && let Some(q) = pts.last_mut() {
            *q = first;
        }
        let fit = fit_freehand(&pts, tol, false);
        let mut add = fit.anchors;
        if let (Some(a), Some(f)) = (sp.anchors.last_mut(), add.first()) {
            a.h_out = f.h_out;
            add.remove(0);
        }
        sp.anchors.extend(add);
        if closing
            && sp.anchors.len() > 2
            && let Some(l) = sp.anchors.pop()
        {
            sp.anchors[0].h_in = l.h_in;
            sp.closed = true;
        }
        for a in &mut sp.anchors {
            *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
        }
        s.edit(if brush { "Paintbrush" } else { "Pencil" }, |d, sel| {
            let path = path_mut(d, id)?;
            if at_start && !sp.closed {
                sp.reverse();
            }
            path.subpaths[si] = sp;
            sel.set([id]);
            Ok(())
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    if pts.len() < 2 {
        return Err(bad(C, "need at least two points"));
    }
    let sp = fit_freehand(&pts, tol, closed);
    let fill = if bool_or(p, "fill", false) { s.paint.fill.clone() } else { Paint::None };
    let ap = Appearance::basic(fill, stroke_paint(s), s.paint.stroke_width.max(0.1));
    add_node(s, if brush { "Paintbrush" } else { "Pencil" }, path_kind(PathData::single(sp)), ap, None)
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
    let sp = vectorcraft_tools::draw2::catmull_rom(&pts, bool_or(p, "closed", false));
    if let Some(id) = id_param(p, "id") {
        s.edit("Curvature", |d, sel| {
            *path_mut(d, id)? = PathData::single(sp);
            sel.set([id]);
            Ok(())
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    let mut ap = Appearance::basic(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    if ap.stroke_paint().is_none() && ap.fill_paint().is_none() {
        ap.set_stroke(Paint::solid(Color::BLACK));
    }
    add_node(s, "Curvature", path_kind(PathData::single(sp)), ap, None)
}

// ---------- anchor tools ----------

fn remove_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.removeAnchor";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
    let ai = usize_req(p, "anchor", C)?;
    let removed = s.edit("Delete Anchor Point", |d, sel| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        let n = sp.anchors.len();
        if ai >= n {
            return Err(EngineError::Other("no such anchor".into()));
        }
        let interior = sp.closed || (ai > 0 && ai + 1 < n);
        if interior && n >= 3 {
            let (pi, ni) = ((ai + n - 1) % n, (ai + 1) % n);
            let (prev, a, next) = (sp.anchors[pi], sp.anchors[ai], sp.anchors[ni]);
            // Keep the outer tangents; stretch them to span both removed segments.
            let l1 = prev.p.distance(a.p);
            let l2 = a.p.distance(next.p);
            let chord = prev.p.distance(next.p).max(1e-9);
            let first_line = !prev.has_out() && !a.has_in();
            let second_line = !a.has_out() && !next.has_in();
            let (mut hout, mut hin) = (prev.h_out, next.h_in);
            if !(first_line && second_line) {
                // Curved: stretch the outer handles to span both segments; a straight side aims a
                // third of the chord at the removed anchor.
                hout = if prev.has_out() {
                    prev.p + (prev.h_out - prev.p) * ((l1 + l2) / l1.max(1e-9)).min(3.0)
                } else {
                    prev.p + (a.p - prev.p) * (chord / 3.0 / l1.max(1e-9))
                };
                hin = if next.has_in() {
                    next.p + (next.h_in - next.p) * ((l1 + l2) / l2.max(1e-9)).min(3.0)
                } else {
                    next.p + (a.p - next.p) * (chord / 3.0 / l2.max(1e-9))
                };
            }
            sp.anchors[pi].h_out = hout;
            sp.anchors[ni].h_in = hin;
        }
        sp.anchors.remove(ai);
        if !sp.closed && !sp.anchors.is_empty() {
            let (f, l) = (0, sp.anchors.len() - 1);
            let a0 = sp.anchors[f].p;
            sp.anchors[f].h_in = a0;
            let al = sp.anchors[l].p;
            sp.anchors[l].h_out = al;
        }
        if sp.anchors.len() < 3 {
            sp.closed = sp.closed && sp.anchors.len() == 2 && sp.anchors.iter().any(|a| a.has_in() || a.has_out());
        }
        let empty = sp.anchors.len() < 2;
        if empty {
            path.subpaths.remove(si);
        }
        if path.is_empty() {
            d.remove(id)?;
            sel.clear();
            return Ok(true);
        }
        sel.anchors.remove(&id);
        Ok(false)
    })?;
    Ok(json!({ "removedObject": removed }))
}

fn convert_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.convertAnchor";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
    let ai = usize_req(p, "anchor", C)?;
    let smooth = str_param(p, "to") == Some("smooth");
    let target = match (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        (Some(x), Some(y)) => Some(Point::new(x, y)),
        _ => None,
    };
    s.edit("Convert Anchor Point", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        let n = sp.anchors.len();
        if ai >= n {
            return Err(EngineError::Other("no such anchor".into()));
        }
        let prev = if ai > 0 || sp.closed { Some(sp.anchors[(ai + n - 1) % n].p) } else { None };
        let next = if ai + 1 < n || sp.closed { Some(sp.anchors[(ai + 1) % n].p) } else { None };
        let a = &mut sp.anchors[ai];
        if !smooth {
            a.retract();
            return Ok(());
        }
        match target {
            Some(t) if t.distance(a.p) > 1e-9 => {
                a.h_out = t;
                a.h_in = a.p - (t - a.p);
                a.kind = AnchorKind::Smooth;
            }
            Some(_) => a.retract(),
            None => {
                let (pp, nn) = (prev.unwrap_or(a.p), next.unwrap_or(a.p));
                let dir = nn - pp;
                let l = dir.hypot();
                if l > 1e-9 {
                    let u = dir / l;
                    a.h_in = a.p - u * (a.p.distance(pp) / 3.0);
                    a.h_out = a.p + u * (a.p.distance(nn) / 3.0);
                    a.kind = AnchorKind::Smooth;
                }
            }
        }
        Ok(())
    })?;
    ok()
}

fn reshape_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.reshapeSegment";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
    let seg = usize_req(p, "segment", C)?;
    let t = f64_or(p, "t", 0.5).clamp(0.05, 0.95);
    let dv = Vec2::new(f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    if !(dv.x.is_finite() && dv.y.is_finite()) {
        return Err(bad(C, "invalid delta"));
    }
    s.edit("Reshape", |d, _| {
        let path = path_mut(d, id)?;
        let sp = path.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        if seg >= sp.segment_count() {
            return Err(EngineError::Other("no such segment".into()));
        }
        let n = sp.anchors.len();
        // Moving both inner control points by v moves B(t) by 3t(1-t)·v.
        let v = dv / (3.0 * t * (1.0 - t));
        let (i0, i1) = (seg % n, (seg + 1) % n);
        let a = sp.anchors[i0];
        sp.anchors[i0] = Anchor::with_handles(a.p, a.h_in, a.h_out + v);
        let b = sp.anchors[i1];
        sp.anchors[i1] = Anchor::with_handles(b.p, b.h_in + v, b.h_out);
        Ok(())
    })?;
    ok()
}

// ---------- Scissors ----------

fn split(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.split";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
    let anchor = p.get("anchor").and_then(Value::as_u64).map(|v| v as usize);
    let seg = p.get("segment").and_then(Value::as_u64).map(|v| v as usize);
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
        if sp.closed {
            sp.anchors.rotate_left(k);
            let mut last = sp.anchors[0];
            last.h_out = last.p;
            sp.anchors[0].h_in = sp.anchors[0].p;
            last.kind = AnchorKind::Corner;
            sp.anchors[0].kind = AnchorKind::Corner;
            sp.anchors.push(last);
            sp.closed = false;
            path.subpaths[si] = sp;
            sel.set([id]);
            return Ok(vec![id]);
        }
        let n = sp.anchors.len();
        if k == 0 || k + 1 >= n {
            return Err(EngineError::Other("cannot split an open path at its end point".into()));
        }
        let mut left = SubPath::new(sp.anchors[..=k].to_vec(), false);
        let mut right = SubPath::new(sp.anchors[k..].to_vec(), false);
        let lp = left.anchors[k].p;
        left.anchors[k].h_out = lp;
        left.anchors[k].kind = AnchorKind::Corner;
        right.anchors[0].h_in = lp;
        right.anchors[0].kind = AnchorKind::Corner;
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
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, id, pieces)?);
        }
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
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, id, pieces)?);
        }
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
        let mut res = smooth_open(&open.anchors, &m, tol);
        let l = res.pop()?;
        if let Some(f) = res.first_mut() {
            f.h_in = l.h_in;
        }
        return Some(SubPath::new(res, true));
    }
    Some(SubPath::new(smooth_open(&sp.anchors, &marked, tol), false))
}

fn smooth_open(anchors: &[Anchor], marked: &[bool], tol: f64) -> Vec<Anchor> {
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
        let new = refit_run(&out[lo..=hi], tol);
        out.splice(lo..=hi, new);
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
    let st = s.doc()?;
    let mut merge_ids = vec![];
    if bool_or(p, "merge", true) {
        let all = Selection::default();
        for id in target_paths(&st.doc, &all, poly_bounds(&pts, size)) {
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
    let ap = Appearance::basic(paint, Paint::None, 1.0);
    let Some(keep) = merge_ids.last().map(|m| m.0) else {
        return add_node(s, "Blob Brush", path_kind(region), ap, None);
    };
    let mut inputs: Vec<(&PathData, FillRule)> = vec![(&region, FillRule::NonZero)];
    inputs.extend(merge_ids.iter().map(|(_, pd, r)| (pd, *r)));
    let merged = po::unite_all(&inputs);
    let others: Vec<NodeId> = merge_ids[..merge_ids.len() - 1].iter().map(|m| m.0).collect();
    s.edit("Blob Brush", |d, sel| {
        *path_mut(d, keep)? = merged;
        for id in others {
            d.remove(id)?;
        }
        sel.set([keep]);
        Ok(())
    })?;
    Ok(json!({ "id": keep.0 }))
}
