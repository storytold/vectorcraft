//! Commands behind the distortion tools: width points, Liquify, Puppet Warp and the Perspective
//! Grid. The geometry kernels live in `vectorcraft_tools::distort` (shared with the tools).
//!
//! The perspective grid definition is document data stored under
//! `Document.unknown["perspectiveGrid"]` (read/written by [`grid_of`] / [`store_grid`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, WidthProfile};
use vectorcraft_geom::Point;
use vectorcraft_tools::distort::liquify::{LiquifyParams, apply_stroke, dabs};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};
use vectorcraft_tools::distort::{arap, collect_points, mesh_for, warp_node_with};

use super::edit::selected_roots;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stroke.widthPoint.set",
            "Width Point",
            [],
            None,
            "{id, t: 0..1 (fraction of the path length), left, right: side widths in pt, index?: width point to move/replace} → {index}. Creates a uniform profile first if needed",
            has_doc,
            width_point_set
        ),
        cmd!("stroke.widthPoint.remove", "Delete Width Point", [], None, "{id, index}", has_doc, width_point_remove),
        cmd!(
            "stroke.widthProfile.set",
            "Width Profile",
            [],
            None,
            "{ids?|id?, points: [[t, left, right]…] (width factors, 1 = the stroke weight) | null = uniform}",
            has_doc,
            width_profile_set
        ),
        cmd!(
            "object.liquify",
            "Liquify",
            [],
            None,
            "{tool: warp|twirl|pucker|bloat|scallop|crystallize|wrinkle, points: [[x,y]…] (the brush stroke), diameter?|width?, height? (pt, default 100), angle?, intensity? (0..1 or %), detail? (1..10), simplify? (0..100), rate? (twirl °), complexity?, horizontal?, vertical?, affectAnchors?, affectIn?, affectOut?, ids? (default: selection, else every path the brush touches)}",
            has_doc,
            liquify
        ),
        cmd!(
            "object.puppetWarp",
            "Puppet Warp",
            [],
            None,
            "{id?|ids? (default: selection), pins: [[x,y]…] (current pin positions), moved: [[x,y]…] (targets, same length), expand?: pt} as-rigid-as-possible mesh warp of anchors and handles",
            has_doc,
            puppet_warp
        ),
        cmd!(
            "perspective.grid.set",
            "Define Perspective Grid",
            [],
            None,
            "{kind?: 1|2|3, origin?: [x,y], horizon?, vpLeft?, vpRight?, vpVertical?: [x,y], distance?, cell?, extent?, height?, visible?, plane?} merge into the document's grid",
            has_doc,
            grid_set
        ),
        cmd!(
            "perspective.grid.preset",
            "Perspective Grid Preset",
            [],
            None,
            "{kind: 1|2|3} reset the grid to the one/two/three-point preset for the first artboard (and show it)",
            has_doc,
            grid_preset
        ),
        cmd!(
            "perspective.grid.show",
            "Show Perspective Grid",
            [],
            Some("Cmd+Shift+I"),
            "{visible?: bool} (no param toggles). View state: not an undo step",
            has_doc,
            grid_show
        ),
        cmd!(
            "perspective.plane.set",
            "Active Perspective Plane",
            [],
            None,
            "{plane: left|right|ground|none} Plane Switching widget. View state: not an undo step",
            has_doc,
            plane_set
        ),
        cmd!(
            "perspective.attach",
            "Attach to Active Plane",
            [],
            None,
            "{ids?, plane?: left|right|ground (default: the active plane)} project the objects onto the plane",
            has_doc,
            attach
        ),
        cmd!("perspective.release", "Release with Perspective", [], None, "{ids?} detach from the grid (geometry unchanged)", has_doc, release),
        cmd!(
            "perspective.move",
            "Move in Perspective",
            [],
            None,
            "{ids?, from: [x,y], to: [x,y], plane?} slide objects within their plane (unattached objects attach to `plane`/the active plane first)",
            has_doc,
            persp_move
        ),
        cmd!(
            "perspective.draw",
            "Draw in Perspective",
            [],
            None,
            "{command: shape.* id, params, plane?} run a shape command and attach the result to the plane",
            has_doc,
            persp_draw
        ),
    ]
}

// ---------- width points ----------

fn uniform() -> WidthProfile {
    WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] }
}

fn width_point_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthPoint.set";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let t = f64_req(p, "t", C)?.clamp(0.0, 1.0);
    let (left, right) = (f64_req(p, "left", C)?, f64_req(p, "right", C)?);
    if !(left.is_finite() && right.is_finite()) || left < 0.0 || right < 0.0 || left.max(right) > 1.0e5 {
        return Err(bad(C, "left/right must be non-negative widths"));
    }
    let index = p.get("index").and_then(Value::as_u64).map(|v| v as usize);
    let pos = s.edit("Width Point", |d, _| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        let st = n.appearance.stroke_mut().ok_or_else(|| EngineError::Other("the object has no stroke".into()))?;
        if st.width <= 0.0 {
            return Err(EngineError::Other("the stroke has no weight".into()));
        }
        let half = st.width / 2.0;
        let mut prof = st.profile.take().unwrap_or_else(uniform);
        if let Some(i) = index {
            if i >= prof.points.len() {
                return Err(bad(C, "no such width point"));
            }
            prof.points.remove(i);
        }
        prof.points.retain(|q| (q.0 - t).abs() > 1e-6);
        let pos = prof.points.iter().position(|q| q.0 > t).unwrap_or(prof.points.len());
        prof.points.insert(pos, (t, left / half, right / half));
        st.profile = Some(prof);
        Ok(pos)
    })?;
    Ok(json!({ "index": pos }))
}

fn width_point_remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthPoint.remove";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing index"))? as usize;
    s.edit("Delete Width Point", |d, _| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        let st = n.appearance.stroke_mut().ok_or_else(|| EngineError::Other("the object has no stroke".into()))?;
        let prof = st.profile.as_mut().filter(|pr| index < pr.points.len()).ok_or_else(|| bad(C, "no such width point"))?;
        prof.points.remove(index);
        if prof.points.is_empty() {
            st.profile = None;
        }
        Ok(())
    })?;
    ok()
}

fn width_profile_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthProfile.set";
    let profile = match p.get("points") {
        None => return Err(bad(C, "missing points")),
        Some(Value::Null) => None,
        Some(Value::Array(a)) if a.is_empty() => None,
        Some(Value::Array(a)) => {
            let mut pts = vec![];
            for q in a {
                let v: Vec<f64> = q.as_array().map(|x| x.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
                if v.len() != 3 || v.iter().any(|x| !x.is_finite()) || v[1] < 0.0 || v[2] < 0.0 {
                    return Err(bad(C, "points must be [t, left, right] with non-negative widths"));
                }
                pts.push((v[0].clamp(0.0, 1.0), v[1], v[2]));
            }
            pts.sort_by(|a, b| a.0.total_cmp(&b.0));
            Some(WidthProfile { points: pts })
        }
        _ => return Err(bad(C, "points must be an array or null")),
    };
    let ids = targets(s, p)?;
    s.edit("Width Profile", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            if let Some(st) = n.appearance.stroke_mut() {
                st.profile = profile.clone();
            }
        }
        Ok(())
    })?;
    ok()
}

// ---------- liquify ----------

fn points_of(p: &Value, key: &str) -> Option<Vec<Point>> {
    p.get(key)?.as_array()?.iter().map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect()
}

/// Leaf paths under `roots` that can be edited.
fn leaf_paths(d: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for r in roots {
        if let Some(n) = d.node(*r) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::Path { .. }) && !out.contains(&c.id) {
                    out.push(c.id);
                }
            });
        }
    }
    out.retain(|id| d.is_editable(*id));
    out
}

fn liquify(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.liquify";
    let prm = LiquifyParams::from_json(p).ok_or_else(|| bad(C, "missing or unknown `tool`"))?;
    let pts = points_of(p, "points").filter(|v| !v.is_empty()).ok_or_else(|| bad(C, "points must be a non-empty [[x,y]…] list"))?;
    if pts.iter().any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return Err(bad(C, "points must be finite"));
    }
    let dab_pts = dabs(&pts, prm.dab_spacing());
    let roots = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => s.doc()?.selection.objects.clone(),
    };
    let doc = &s.doc()?.doc;
    let leaves = if roots.is_empty() {
        // Nothing selected: everything the brush sweeps over.
        let sweep = dab_pts
            .iter()
            .map(|c| prm.brush_bounds(*c))
            .reduce(|a, b| a.union(b))
            .ok_or_else(|| bad(C, "points must be a non-empty [[x,y]…] list"))?;
        let mut all = vec![];
        doc.walk(|n| {
            if matches!(n.kind, NodeKind::Path { guide: false, .. })
                && n.geometric_bounds().is_some_and(|b| b.inflate(1e-6, 1e-6).intersect(sweep).area() > 0.0)
            {
                all.push(n.id);
            }
        });
        all.retain(|id| doc.is_editable(*id) && doc.is_visible(*id));
        all
    } else {
        leaf_paths(doc, &roots)
    };
    let changed = s.edit(&format!("{} Tool", prm.kind.label()), |d, _| {
        let mut changed = vec![];
        for id in &leaves {
            let Some(n) = d.node_mut(*id) else { continue };
            if let NodeKind::Path { path, live, .. } = &mut n.kind
                && apply_stroke(path, &dab_pts, &prm, id.0)
            {
                *live = None;
                changed.push(id.0);
            }
        }
        Ok(changed)
    })?;
    Ok(json!({ "ids": changed }))
}

// ---------- puppet warp ----------

fn puppet_warp(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.puppetWarp";
    let pins = points_of(p, "pins").ok_or_else(|| bad(C, "pins must be [[x,y]…]"))?;
    let moved = points_of(p, "moved").ok_or_else(|| bad(C, "moved must be [[x,y]…]"))?;
    if pins.is_empty() || pins.len() != moved.len() {
        return Err(bad(C, "pins and moved must be non-empty lists of the same length"));
    }
    if pins.iter().chain(&moved).any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return Err(bad(C, "pins must be finite"));
    }
    let ids = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    if ids.is_empty() {
        return Err(EngineError::Other("select the artwork to warp".into()));
    }
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let mesh = mesh_for(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    let pins: Vec<arap::Pin> = pins.iter().zip(&moved).map(|(a, b)| arap::Pin { rest: *a, target: *b }).collect();
    let deformed = arap::deform(&mesh, &pins);
    let f = |q: Point| mesh.map(&deformed, q);
    s.edit("Puppet Warp", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            warp_node_with(n, &f);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

// ---------- perspective grid ----------

/// The document's perspective grid (the default preset when none was defined).
pub(crate) fn grid_of(d: &Document) -> PerspectiveGrid {
    PerspectiveGrid::effective(d)
}

/// Write the grid into the document (`Document.unknown["perspectiveGrid"]`).
pub(crate) fn store_grid(d: &mut Document, g: &PerspectiveGrid) {
    g.store(d);
}

/// Change view state of the grid without an undo step.
fn silent(s: &mut Session, f: impl FnOnce(&mut PerspectiveGrid)) -> Result<PerspectiveGrid> {
    let st = s.doc_mut()?;
    let mut g = grid_of(&st.doc);
    f(&mut g);
    store_grid(Arc::make_mut(&mut st.doc), &g);
    if let Some(it) = &mut st.interaction {
        store_grid(Arc::make_mut(&mut it.doc), &g);
    }
    st.revision += 1;
    Ok(g)
}

fn grid_set(s: &mut Session, p: &Value) -> Result<Value> {
    let g = grid_of(&s.doc()?.doc).merged(p).map_err(|e| bad("perspective.grid.set", e))?;
    s.edit("Define Perspective Grid", |d, _| {
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(g.definition_json())
}

fn grid_preset(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = p
        .get("kind")
        .and_then(Value::as_u64)
        .filter(|k| (1..=3).contains(k))
        .ok_or_else(|| bad("perspective.grid.preset", "kind must be 1, 2 or 3"))? as u8;
    let old = grid_of(&s.doc()?.doc);
    let ab = s.doc()?.doc.artboards.first().map(|a| a.rect).ok_or_else(|| EngineError::Other("no artboard".into()))?;
    let g = PerspectiveGrid { attached: old.attached, ..PerspectiveGrid::preset(kind, ab) };
    s.edit("Perspective Grid Preset", |d, _| {
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(g.definition_json())
}

fn grid_show(s: &mut Session, p: &Value) -> Result<Value> {
    let want = p.get("visible").and_then(Value::as_bool);
    let g = silent(s, |g| g.visible = want.unwrap_or(!g.visible))?;
    Ok(json!({ "visible": g.visible }))
}

fn plane_set(s: &mut Session, p: &Value) -> Result<Value> {
    let plane = str_param(p, "plane").and_then(Plane::parse).ok_or_else(|| bad("perspective.plane.set", "plane must be left|right|ground|none"))?;
    silent(s, |g| g.plane = plane)?;
    Ok(json!({ "plane": plane.id() }))
}

fn plane_param(g: &PerspectiveGrid, p: &Value, cmd: &str) -> Result<Plane> {
    let plane = match str_param(p, "plane") {
        Some(v) => Plane::parse(v).ok_or_else(|| bad(cmd, "plane must be left|right|ground"))?,
        None => g.plane,
    };
    if plane == Plane::None {
        return Err(EngineError::Other("no active perspective plane".into()));
    }
    Ok(plane)
}

fn roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => Ok(v),
        None => selected_roots(s),
    }
}

/// Map `n` with `f`, failing if any point would leave the plane's visible side.
fn warp_checked(d: &mut Document, id: NodeId, f: &dyn Fn(Point) -> Option<Point>) -> Result<()> {
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    let mut pts = vec![];
    collect_points(n, &mut pts);
    if pts.iter().any(|q| f(*q).is_none()) {
        return Err(EngineError::Other("the object would cross the horizon of the perspective plane".into()));
    }
    warp_node_with(n, &|q| f(q).unwrap_or(q));
    Ok(())
}

fn attach_in(d: &mut Document, g: &mut PerspectiveGrid, id: NodeId, plane: Plane) -> Result<()> {
    let b = d.node(id).ok_or(EngineError::NoNode(id))?.geometric_bounds().ok_or_else(|| EngineError::Other("the object has no geometry".into()))?;
    let m = g.attach_map(plane, b).ok_or_else(|| EngineError::Other("the object is beyond the plane's horizon".into()))?;
    warp_checked(d, id, &m)?;
    g.attached.insert(id.0.to_string(), plane);
    Ok(())
}

fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = roots(s, p)?;
    let mut g = grid_of(&s.doc()?.doc);
    let plane = plane_param(&g, p, "perspective.attach")?;
    s.edit("Attach to Active Plane", |d, _| {
        for id in &ids {
            attach_in(d, &mut g, *id, plane)?;
        }
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "plane": plane.id() }))
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = roots(s, p)?;
    let mut g = grid_of(&s.doc()?.doc);
    s.edit("Release with Perspective", |d, _| {
        for id in &ids {
            g.attached.remove(&id.0.to_string());
        }
        store_grid(d, &g);
        Ok(())
    })?;
    ok()
}

fn persp_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.move";
    let ids = roots(s, p)?;
    let from = point_param(p, "from").ok_or_else(|| bad(C, "missing from"))?;
    let to = point_param(p, "to").ok_or_else(|| bad(C, "missing to"))?;
    let mut g = grid_of(&s.doc()?.doc);
    let fallback = plane_param(&g, p, C).ok();
    s.edit("Move in Perspective", |d, _| {
        for id in &ids {
            let plane = match g.attached_plane(*id) {
                Some(pl) => pl,
                None => {
                    let pl = fallback.ok_or_else(|| EngineError::Other("the object isn't on a perspective plane".into()))?;
                    attach_in(d, &mut g, *id, pl)?;
                    pl
                }
            };
            let m = g.move_map(plane, from, to).ok_or_else(|| EngineError::Other("the pointer is beyond the plane's horizon".into()))?;
            warp_checked(d, *id, &m)?;
        }
        store_grid(d, &g);
        Ok(())
    })?;
    ok()
}

fn persp_draw(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.draw";
    let command = str_param(p, "command").filter(|c| c.starts_with("shape.")).ok_or_else(|| bad(C, "command must be a shape.* command"))?.to_string();
    let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    let mut g = grid_of(&s.doc()?.doc);
    let plane = plane_param(&g, p, C)?;
    let undo_before = s.doc()?.history.undo.len();
    let r = s.execute(&command, &params)?;
    let id = r.get("id").and_then(Value::as_u64).map(NodeId).ok_or_else(|| EngineError::Other(format!("{command} didn't create an object")))?;
    let res = s.edit("Draw in Perspective", |d, _| {
        attach_in(d, &mut g, id, plane)?;
        store_grid(d, &g);
        Ok(())
    });
    // One undo step for the whole gesture.
    let st = s.doc_mut()?;
    if st.interaction.is_none() && st.history.undo.len() > undo_before + 1 {
        st.history.undo.truncate(undo_before + 1);
        if let Some(e) = st.history.undo.last_mut() {
            e.label = "Draw in Perspective".into();
        }
    }
    res?;
    Ok(json!({ "id": id.0, "plane": plane.id() }))
}

/// Shape tool previews become `perspective.draw` while the grid is shown with an active plane.
pub(crate) fn perspective_rewrite(s: &Session, cmd: &str, params: &Value) -> Option<(String, Value)> {
    const SHAPES: &[&str] = &["shape.rectangle", "shape.ellipse", "shape.polygon", "shape.star", "shape.line", "shape.rectangularGrid", "shape.arc"];
    if !SHAPES.contains(&cmd) {
        return None;
    }
    let g = PerspectiveGrid::from_doc(&s.active()?.doc)?;
    (g.visible && g.plane != Plane::None).then(|| ("perspective.draw".to_string(), json!({"command": cmd, "params": params})))
}
