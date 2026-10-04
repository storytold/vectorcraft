//! Shape Builder (`shapeBuilder.*`), Live Paint (`livePaint.*`) and Image Trace (`imageTrace.*`).
//!
//! **Live Paint groups** need no special node kind: a Live Paint group is a plain group whose name
//! starts with "Live Paint". Its first child is a hidden group named "Live Paint Sources" holding
//! the original paths (the planar map's inputs, used by Release, Merge and re-computation); then
//! come the faces (paths named "Face", fill only) and the edges (open paths named "Edge", stroke
//! only). See `vectorcraft_tools::builder` for the shared helpers.
//!
//! **Image Trace** makes a group named "Image Trace" holding the source image (hidden, bottom)
//! and the traced filled paths above it, mapped with the image's transform. Expand drops the
//! image (a plain group of paths remains); Release puts the image back in place of the group.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer, StrokeLayer};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::{FillRule, PathData, Point, Rect, Shape as _};
use vectorcraft_pathops as po;
use vectorcraft_tools::builder::{
    self as b, EDGE_NAME, FACE_NAME, LIVE_PAINT_NAME, SOURCES_NAME, edge_near, face_at, is_live_paint, sample_polyline, shapes_for, sorted_roots,
};
use vectorcraft_trace as tr;

use super::edit::selected_roots;
use super::paint::paint_from;
use super::pathops::shape_node;
use super::*;

/// Name of an Image Trace group.
pub const TRACE_NAME: &str = "Image Trace";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shapeBuilder.merge",
            "Shape Builder",
            [],
            None,
            "{ids?: [id…] (default: selection), points: [[x,y]…] (drag path; one point = click), erase?: bool (Alt-drag deletes the regions), fill?: \"#rrggbb\" | {color|swatch|gradient|none} (default: the fill of the object under the first point)} → {ids, merged, regions}",
            has_selection_or_ids,
            sb_merge
        ),
        cmd!(
            query "shapeBuilder.regions",
            "Shape Builder Regions",
            [],
            None,
            "{ids?} → {regions: [{bounds: [x0,y0,x1,y1], area, sources: [id…]}]} faces of the planar arrangement",
            has_selection_or_ids,
            sb_regions
        ),
        cmd!(
            "livePaint.make",
            "Make",
            ["Object", "Live Paint"],
            Some("Cmd+Alt+X"),
            "{ids?} build a Live Paint group from the selected paths → {id, faces, edges}",
            has_selection_or_ids,
            lp_make
        ),
        cmd!(
            "livePaint.merge",
            "Merge",
            ["Object", "Live Paint"],
            None,
            "{ids?} add the other selected paths (and Live Paint groups) to the first Live Paint group, keeping its paint → {id, faces, edges}",
            has_multi_or_ids,
            lp_merge
        ),
        cmd!(
            "livePaint.release",
            "Release",
            ["Object", "Live Paint"],
            None,
            "{id?} replace Live Paint groups with their original paths → {ids}",
            has_selection_or_ids,
            lp_release
        ),
        cmd!(
            "livePaint.expand",
            "Expand",
            ["Object", "Live Paint"],
            None,
            "{id?} turn Live Paint groups into plain groups of their painted faces and edges → {ids}",
            has_selection_or_ids,
            lp_expand
        ),
        cmd!(
            "livePaint.fill",
            "Live Paint Bucket",
            [],
            None,
            "{group?: id | ids?: [id…] (paths to make live first), point: [x,y], color?|swatch?|gradient?|none? (default: current fill)} fill the face containing point → {group, face}",
            has_doc,
            lp_fill
        ),
        cmd!(
            "livePaint.strokeEdge",
            "Live Paint Bucket (Stroke)",
            [],
            None,
            "{group: id, point: [x,y], color?|swatch?|none? (default: current stroke), width?: pt (default: current weight), tolerance?: pt (4)} paint the edge nearest point → {group, edge}",
            has_doc,
            lp_stroke_edge
        ),
        cmd!(
            query "livePaint.info",
            "Live Paint Info",
            [],
            None,
            "{group} → {faces: [{id, fill, bounds}], edges: [{id, stroke, width}], sources}",
            has_doc,
            lp_info
        ),
        cmd!(
            "imageTrace.make",
            "Make",
            ["Object", "Image Trace"],
            None,
            "{id?: image or Image Trace group (default: selection), preset?: name (imageTrace.presets), params?: {mode: \"blackAndWhite\"|\"grayscale\"|\"color\", threshold: 0-255, colors: 2-256, paths: 0-100, corners: 0-100, noise: px, method: \"abutting\"|\"overlapping\", ignoreWhite, snapCurvesToLines}} → {id, paths, anchors, colors}",
            has_selection_or_ids,
            |s, p| trace_make(s, p, false)
        ),
        cmd!(
            "imageTrace.makeAndExpand",
            "Make and Expand",
            ["Object", "Image Trace"],
            None,
            "same as imageTrace.make, then expand → {id, paths, anchors, colors}",
            has_selection_or_ids,
            |s, p| trace_make(s, p, true)
        ),
        cmd!(
            "imageTrace.release",
            "Release",
            ["Object", "Image Trace"],
            None,
            "{id?} put the source image back in place of Image Trace groups → {ids}",
            has_selection_or_ids,
            trace_release
        ),
        cmd!(
            "imageTrace.expand",
            "Expand",
            ["Object", "Image Trace"],
            None,
            "{id?} keep only the traced paths (drop the source image) → {ids}",
            has_selection_or_ids,
            trace_expand
        ),
        cmd!(query "imageTrace.presets", "Image Trace Presets", [], None, "{} → {presets: [{name, params}]}", always, |_, _| {
            Ok(json!({ "presets": tr::presets().into_iter().map(|(n, p)| json!({ "name": n, "params": p })).collect::<Vec<_>>() }))
        }),
    ]
}

fn has_selection_or_ids(s: &Session) -> std::result::Result<(), String> {
    // Agents may pass ids explicitly; menus need a selection. Accept either (params are checked in run).
    has_doc(s)
}

fn has_multi_or_ids(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)
}

fn root_ids(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    if ids.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let d = &s.doc()?.doc;
    if let Some(bad_id) = ids.iter().find(|i| d.node(**i).is_none()) {
        return Err(EngineError::NoNode(*bad_id));
    }
    // Layers are containers, not objects.
    let ids: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(|n| !n.is_layer())).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("select objects, not layers".into()));
    }
    Ok(sorted_roots(d, &ids))
}

fn points_param(p: &Value, key: &str) -> Option<Vec<Point>> {
    p.get(key)?.as_array()?.iter().map(|v| Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))).collect()
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= crate::MAX_COORD && p.y.abs() <= crate::MAX_COORD
}

fn fill_only(paint: Paint) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint))], effects: vec![] }
}

fn stroke_only(paint: Paint, width: f64) -> Appearance {
    Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, width))], effects: vec![] }
}

/// A point strictly inside a filled path (for re-finding faces after a rebuild).
pub(crate) fn interior_point(path: &PathData) -> Option<Point> {
    let bb = path.bounds()?;
    let bp = path.to_bezpath();
    let mut edges: Vec<(Point, Point)> = vec![];
    let (mut first, mut last) = (Point::ZERO, Point::ZERO);
    kurbo::flatten(bp.iter(), (bb.width().max(bb.height()) * 1e-3).max(1e-4), |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if last != first {
                edges.push((last, first));
            }
            first = p;
            last = p;
        }
        kurbo::PathEl::LineTo(p) => {
            edges.push((last, p));
            last = p;
        }
        kurbo::PathEl::ClosePath => {
            if last != first {
                edges.push((last, first));
            }
            last = first;
        }
        _ => {}
    });
    if last != first {
        edges.push((last, first));
    }
    let mut best: Option<(f64, Point)> = None;
    for f in [0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875, 0.0625, 0.9375] {
        let y = bb.y0 + bb.height() * f;
        let mut xs: Vec<f64> =
            edges.iter().filter(|(a, c)| (a.y <= y) != (c.y <= y)).map(|(a, c)| a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x)).collect();
        xs.sort_by(f64::total_cmp);
        for w in xs.windows(2) {
            let m = Point::new((w[0] + w[1]) / 2.0, y);
            let width = w[1] - w[0];
            if width > best.map_or(0.0, |b| b.0) && bp.winding(m) != 0 {
                best = Some((width, m));
            }
        }
    }
    best.map(|b| b.1)
}

// ---------- Shape Builder ----------

fn fill_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Paint>> {
    match p.get("fill") {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Object(o)) if o.contains_key("color") || o.contains_key("swatch") || o.contains_key("gradient") || o.contains_key("none") => {
            paint_from(s, v)
        }
        Some(v) => color_value(v).map(|c| Some(Paint::solid(c))).ok_or_else(|| bad(cmd, format!("bad fill {v}"))),
    }
}

fn sb_merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shapeBuilder.merge";
    let pts = points_param(p, "points").ok_or_else(|| bad(C, "missing points [[x,y]…]"))?;
    if pts.is_empty() || !pts.iter().all(|q| finite(*q)) {
        return Err(bad(C, "points must be a non-empty list of finite [x,y]"));
    }
    let erase = bool_or(p, "erase", false);
    let fill = fill_param(s, p, C)?;
    let roots = root_ids(s, p)?;
    let label = if erase { "Shape Builder Delete" } else { "Shape Builder" };
    let (ids, merged, count) = s.edit(label, |d, sel| {
        let (shapes, leaves) = shapes_for(d, &roots);
        let leaves: Vec<Node> = leaves.into_iter().cloned().collect();
        if shapes.is_empty() {
            return Err(EngineError::Other("Shape Builder: select paths or compound paths".into()));
        }
        let regs = po::regions(&shapes);
        let shapes_r: Vec<(kurbo::BezPath, Rect)> = regs.iter().map(|r| (r.path.to_bezpath(), r.path.bounds().unwrap_or(Rect::ZERO))).collect();
        let step = (d.bounds_of(&roots, false).map(|r| r.width().max(r.height())).unwrap_or(100.0) / 2000.0).max(0.25);
        let mut touched: Vec<usize> = vec![];
        for q in sample_polyline(&pts, step) {
            if let Some(i) = shapes_r.iter().position(|(bp, bb)| bb.contains(q) && bp.winding(q) != 0)
                && !touched.contains(&i)
            {
                touched.push(i);
            }
        }
        if touched.is_empty() {
            return Err(EngineError::Other("Shape Builder: no region under the pointer".into()));
        }
        let picked: Vec<&po::Region> = touched.iter().map(|&i| &regs[i]).collect();
        let union = po::merge_regions(&picked);
        let affected: Vec<bool> = (0..shapes.len()).map(|i| picked.iter().any(|r| r.sources.contains(&i))).collect();
        let style_src = regs[touched[0]].top();
        let mut new_nodes: Vec<Node> = vec![];
        let mut merged_id = None;
        for (i, (sh, leaf)) in shapes.iter().zip(&leaves).enumerate() {
            if !affected[i] {
                new_nodes.push(d.reid(leaf));
            } else {
                let rest = po::boolean(&sh.path, sh.rule, &union, FillRule::NonZero, po::BoolOp::Difference);
                if !rest.is_empty() {
                    new_nodes.push(shape_node(d, rest, Some(leaf)));
                }
            }
            if !erase && i == style_src {
                let mut m = shape_node(d, union.clone(), Some(leaf));
                if let Some(f) = &fill {
                    m.appearance.set_fill(f.clone());
                }
                merged_id = Some(m.id);
                new_nodes.push(m);
            }
        }
        let new_ids = replace_roots(d, &roots, new_nodes)?;
        sel.set(new_ids.iter().copied());
        Ok((new_ids, merged_id, touched.len()))
    })?;
    Ok(json!({ "ids": ids_json(&ids), "merged": merged.map(|m| m.0), "regions": count }))
}

/// Remove the roots and insert `nodes` where the front-most root was. Returns the new ids.
/// (Roots go first: the new nodes may reuse their ids, e.g. Live Paint sources.)
fn replace_roots(d: &mut Document, roots: &[NodeId], nodes: Vec<Node>) -> Result<Vec<NodeId>> {
    let top = *roots.last().ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
    let before = roots.iter().filter(|r| **r != top && d.position(**r).is_some_and(|(p, i, _)| p == par && i < idx)).count();
    for r in roots {
        if d.node(*r).is_some() {
            d.remove(*r)?;
        }
    }
    let at = idx - before;
    let mut ids = vec![];
    for (k, n) in nodes.into_iter().enumerate() {
        ids.push(d.insert(par, at + k, n)?);
    }
    Ok(ids)
}

fn sb_regions(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let d = &s.doc()?.doc;
    let (shapes, leaves) = shapes_for(d, &roots);
    let regs = po::regions(&shapes);
    let out: Vec<Value> = regs
        .iter()
        .filter_map(|r| {
            let bb = r.path.bounds()?;
            Some(json!({
                "bounds": [bb.x0, bb.y0, bb.x1, bb.y1],
                "area": po::area(&r.path, FillRule::NonZero),
                "sources": r.sources.iter().map(|&i| leaves[i].id.0).collect::<Vec<_>>(),
            }))
        })
        .collect();
    Ok(json!({ "regions": out }))
}

// ---------- Live Paint ----------

/// The source nodes a root contributes (a Live Paint group contributes its own sources).
fn source_nodes(n: &Node) -> Vec<Arc<Node>> {
    if is_live_paint(n) { b::sources(n).and_then(|g| g.children().cloned()).unwrap_or_default() } else { vec![Arc::new(n.clone())] }
}

/// Planar map children (sources group, faces, edges) for `sources`.
fn build_children(d: &mut Document, sources: Vec<Arc<Node>>) -> Result<(Vec<Arc<Node>>, usize, usize)> {
    let mut lv = vec![];
    for n in &sources {
        b::leaf_shapes(n, &mut lv);
    }
    let mut shapes = vec![];
    let mut styles: Vec<(Paint, Paint, f64)> = vec![];
    for l in lv {
        if let Some((path, rule)) = b::node_outline(l)
            && !path.is_empty()
        {
            shapes.push(po::Shape::new(path, rule, styles.len() as u64));
            let a = &l.appearance;
            styles.push((a.fill_paint(), a.stroke_paint(), a.stroke().map(|s| s.width).unwrap_or(1.0)));
        }
    }
    if shapes.is_empty() {
        return Err(EngineError::Other("Live Paint: select paths or compound paths".into()));
    }
    let mut children: Vec<Arc<Node>> = vec![];
    let gid = d.alloc_id();
    let mut src_group = Node::group(gid, sources);
    src_group.name = Some(SOURCES_NAME.into());
    src_group.visible = false;
    children.push(Arc::new(src_group));
    let regs = po::regions(&shapes);
    let nfaces = regs.len();
    for r in regs {
        let id = d.alloc_id();
        let fill = styles[r.top()].0.clone();
        let mut n = Node::path(id, r.path, fill_only(fill));
        n.name = Some(FACE_NAME.into());
        children.push(Arc::new(n));
    }
    let edges = po::pathfinder(po::PathfinderOp::Outline, &shapes);
    let nedges = edges.len();
    for e in edges {
        let (_, stroke, w) = &styles[e.key as usize];
        let id = d.alloc_id();
        let mut n = Node::path(id, e.path, stroke_only(stroke.clone(), *w));
        n.name = Some(EDGE_NAME.into());
        children.push(Arc::new(n));
    }
    Ok((children, nfaces, nedges))
}

/// Make a Live Paint group from `roots`; returns (group, faces, edges).
fn make_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId]) -> Result<(NodeId, usize, usize)> {
    let mut sources = vec![];
    for r in roots {
        let n = d.node(*r).ok_or(EngineError::NoNode(*r))?;
        sources.extend(source_nodes(n));
    }
    let (children, nf, ne) = build_children(d, sources)?;
    let gid = d.alloc_id();
    let mut g = Node::group(gid, children);
    g.name = Some(LIVE_PAINT_NAME.into());
    replace_roots(d, roots, vec![g])?;
    sel.set([gid]);
    Ok((gid, nf, ne))
}

fn lp_make(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Make Live Paint", |d, sel| make_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

fn lp_merge(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Merge Live Paint", |d, sel| {
        let target = roots
            .iter()
            .copied()
            .find(|r| d.node(*r).is_some_and(is_live_paint))
            .ok_or_else(|| EngineError::Other("Live Paint Merge: select a Live Paint group and the paths to add".into()))?;
        let old = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
        // Paint to carry over: (point inside face, fill) and (point on edge, stroke).
        let keep_faces: Vec<(Point, Paint)> = b::faces(&old)
            .iter()
            .filter(|f| !f.appearance.fill_paint().is_none())
            .filter_map(|f| Some((interior_point(f.path_data()?)?, f.appearance.fill_paint())))
            .collect();
        let keep_edges: Vec<(Point, Appearance)> = b::edges(&old)
            .iter()
            .filter_map(|e| {
                let pd = e.path_data()?;
                let sp = pd.subpaths.first().filter(|sp| sp.segment_count() > 0)?;
                Some((kurbo::ParamCurve::eval(&sp.segment(0), 0.5), e.appearance.clone()))
            })
            .collect();
        let mut sources = source_nodes(&old);
        for r in &roots {
            if *r != target {
                sources.extend(source_nodes(d.node(*r).ok_or(EngineError::NoNode(*r))?));
            }
        }
        let (children, nf, ne) = build_children(d, sources)?;
        let mut g = Node::group(target, children);
        g.name = old.name.clone();
        g.opacity = old.opacity;
        g.blend = old.blend;
        for (pt, paint) in keep_faces {
            if let Some(fid) = face_at(&g, pt).map(|f| f.id)
                && let Some(ch) = g.children_mut()
                && let Some(f) = ch.iter_mut().find(|c| c.id == fid)
            {
                Arc::make_mut(f).appearance.set_fill(paint);
            }
        }
        for (pt, app) in keep_edges {
            if let Some(eid) = edge_near(&g, pt, 0.5).map(|e| e.id)
                && let Some(ch) = g.children_mut()
                && let Some(e) = ch.iter_mut().find(|c| c.id == eid)
            {
                Arc::make_mut(e).appearance = app;
            }
        }
        for r in roots.iter().filter(|r| **r != target) {
            if d.node(*r).is_some() {
                d.remove(*r)?;
            }
        }
        let (par, idx, _) = d.position(target).ok_or(EngineError::NoNode(target))?;
        d.remove(target)?;
        d.insert(par, idx, g)?;
        sel.set([target]);
        Ok((target, nf, ne))
    })?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

fn live_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids = root_ids(s, p)?;
    let v: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(is_live_paint)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select a Live Paint group".into()));
    }
    Ok(v)
}

fn lp_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    let ids = s.edit("Release Live Paint", |d, sel| {
        let mut out = vec![];
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let srcs: Vec<Node> = source_nodes(&n).iter().map(|c| (**c).clone()).collect();
            out.extend(replace_roots(d, &[*g], srcs)?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn lp_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    s.edit("Expand Live Paint", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| match c.name.as_deref() {
                    Some(SOURCES_NAME) => false,
                    Some(FACE_NAME) => !c.appearance.fill_paint().is_none(),
                    Some(EDGE_NAME) => !c.appearance.stroke_paint().is_none() && c.appearance.stroke_width() > 0.0,
                    _ => true,
                });
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

fn lp_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.fill";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.fill.clone());
    let group = id_param(p, "group");
    let roots = if group.is_none() { root_ids(s, p)? } else { vec![] };
    let (g, face) = s.edit("Live Paint Bucket", |d, sel| {
        let gid = match group {
            Some(g) => {
                if !d.node(g).is_some_and(is_live_paint) {
                    return Err(bad(C, format!("{g} is not a Live Paint group")));
                }
                g
            }
            None => match roots.iter().copied().find(|r| d.node(*r).is_some_and(|n| is_live_paint(n) && face_at(n, pt).is_some())) {
                Some(g) => g,
                None => make_live(d, sel, &roots)?.0,
            },
        };
        let fid =
            d.node(gid).and_then(|n| face_at(n, pt)).map(|f| f.id).ok_or_else(|| EngineError::Other("no Live Paint face at that point".into()))?;
        d.node_mut(fid).ok_or(EngineError::NoNode(fid))?.appearance.set_fill(paint.clone());
        Ok((gid, fid))
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "face": face.0 }))
}

fn lp_stroke_edge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.strokeEdge";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let g = id_param(p, "group").ok_or_else(|| bad(C, "missing group"))?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.stroke.clone());
    let width = p.get("width").and_then(Value::as_f64).unwrap_or(s.paint.stroke_width.max(0.0));
    if !(0.0..=1000.0).contains(&width) {
        return Err(bad(C, "width out of range"));
    }
    let tol = f64_or(p, "tolerance", 4.0).clamp(0.0, 1000.0);
    let edge = s.edit("Live Paint Bucket", |d, _| {
        let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad(C, format!("{g} is not a Live Paint group")))?;
        let eid = edge_near(n, pt, tol).map(|e| e.id).ok_or_else(|| EngineError::Other("no Live Paint edge near that point".into()))?;
        let e = d.node_mut(eid).ok_or(EngineError::NoNode(eid))?;
        e.appearance.set_stroke(paint.clone());
        if let Some(st) = e.appearance.stroke_mut() {
            st.width = width;
        }
        Ok(eid)
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "edge": edge.0 }))
}

fn lp_info(s: &mut Session, p: &Value) -> Result<Value> {
    let g = id_param(p, "group").or_else(|| id_param(p, "id")).ok_or_else(|| bad("livePaint.info", "missing group"))?;
    let d = &s.doc()?.doc;
    let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad("livePaint.info", format!("{g} is not a Live Paint group")))?;
    let faces: Vec<Value> = b::faces(n)
        .iter()
        .map(|f| {
            let bb = f.geometric_bounds().unwrap_or(Rect::ZERO);
            json!({ "id": f.id.0, "fill": f.appearance.fill_paint().label(), "bounds": [bb.x0, bb.y0, bb.x1, bb.y1] })
        })
        .collect();
    let edges: Vec<Value> = b::edges(n)
        .iter()
        .map(|e| json!({ "id": e.id.0, "stroke": e.appearance.stroke_paint().label(), "width": e.appearance.stroke_width() }))
        .collect();
    let sources = b::sources(n).and_then(|g| g.children()).map(|c| c.len()).unwrap_or(0);
    Ok(json!({ "faces": faces, "edges": edges, "sources": sources }))
}

// ---------- Image Trace ----------

fn is_trace_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(TRACE_NAME)
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

fn trace_params(p: &Value) -> Result<tr::TraceParams> {
    let name = str_param(p, "preset").unwrap_or("Default");
    let base = tr::preset(name).ok_or_else(|| bad("imageTrace.make", format!("unknown preset `{name}` (see imageTrace.presets)")))?;
    let Some(over) = p.get("params").filter(|v| !v.is_null()) else { return Ok(base) };
    let mut v = serde_json::to_value(&base).map_err(|e| EngineError::Other(e.to_string()))?;
    let (Some(o), Some(src)) = (v.as_object_mut(), over.as_object()) else { return Err(bad("imageTrace.make", "params must be an object")) };
    for (k, val) in src {
        o.insert(k.clone(), val.clone());
    }
    serde_json::from_value(v).map_err(|e| bad("imageTrace.make", e.to_string()))
}

fn trace_make(s: &mut Session, p: &Value, expand: bool) -> Result<Value> {
    const C: &str = "imageTrace.make";
    let params = trace_params(p)?;
    let params_json = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    // The preset's name if the parameters are exactly that preset's, else "Custom".
    let named = str_param(p, "preset").unwrap_or("Default");
    let preset_label = match tr::preset(named) {
        Some(b) if serde_json::to_value(&b).ok().as_ref() == Some(&params_json) => named.to_string(),
        _ => "Custom".to_string(),
    };
    let roots = root_ids(s, p)?;
    // The image (inside a trace group, or selected directly).
    let (target, img) = {
        let d = &s.doc()?.doc;
        roots
            .iter()
            .find_map(|r| {
                let n = d.node(*r)?;
                match &n.kind {
                    NodeKind::Image(i) => Some((*r, i.clone())),
                    _ if is_trace_group(n) => match &n.children()?.first()?.kind {
                        NodeKind::Image(i) => Some((*r, i.clone())),
                        _ => None,
                    },
                    _ => None,
                }
            })
            .ok_or_else(|| bad(C, "select an image (or an Image Trace group)"))?
    };
    let blob = s.doc()?.doc.images.get(&img.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", img.key)))?;
    let raster = tr::Raster::decode(&blob.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let res = tr::trace(&raster, &params);
    // Pixel space of the decoded raster → the image object's pixel space → document.
    let sx = img.width.max(1) as f64 / raster.width.max(1) as f64;
    let sy = img.height.max(1) as f64 / raster.height.max(1) as f64;
    let xf = img.xf * vectorcraft_geom::Affine::scale_non_uniform(sx, sy);
    let (npaths, anchors, colors) = (res.paths.len(), res.anchor_count(), res.palette.len());
    let label = if expand { "Image Trace (Make and Expand)" } else { "Image Trace" };
    let id = s.edit(label, |d, sel| {
        let src = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
        let mut image = if is_trace_group(&src) {
            src.children().and_then(|c| c.first()).map(|c| (**c).clone()).ok_or(EngineError::NoNode(target))?
        } else {
            src.clone()
        };
        image.visible = true;
        let mut children: Vec<Arc<Node>> = vec![];
        if !expand {
            let mut hidden = image.clone();
            hidden.visible = false;
            children.push(Arc::new(hidden));
        }
        for tp in res.paths {
            let path = tp.path.transformed(xf);
            let mut n = shape_node(d, path, None);
            n.appearance = fill_only(Paint::solid(Color::rgb8(tp.color[0], tp.color[1], tp.color[2])));
            children.push(Arc::new(n));
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = if expand { None } else { Some(TRACE_NAME.into()) };
        if !expand {
            g.trace = Some(Box::new(json!({ "preset": preset_label, "params": params_json })));
        }
        let (par, idx, _) = d.position(target).ok_or(EngineError::NoNode(target))?;
        d.remove(target)?;
        d.insert(par, idx, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "paths": npaths, "anchors": anchors, "colors": colors }))
}

fn trace_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let v: Vec<NodeId> = root_ids(s, p)?.into_iter().filter(|i| d.node(*i).is_some_and(is_trace_group)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select an Image Trace object".into()));
    }
    Ok(v)
}

fn trace_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    let ids = s.edit("Release Image Trace", |d, sel| {
        let mut out = vec![];
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let mut img = (**n.children().and_then(|c| c.first()).ok_or(EngineError::NoNode(*g))?).clone();
            img.visible = true;
            out.extend(replace_roots(d, &[*g], vec![img])?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn trace_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    s.edit("Expand Image Trace", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| !matches!(c.kind, NodeKind::Image(_)));
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}
