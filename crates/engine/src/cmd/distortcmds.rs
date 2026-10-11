//! Commands behind the distortion tools: width points, Liquify, Puppet Warp and the Perspective
//! Grid. The geometry kernels live in `vectorcraft_tools::distort` (shared with the tools).
//!
//! The perspective grid definition is document data stored under
//! `Document.unknown["perspectiveGrid"]` (read/written by [`grid_of`] / [`store_grid`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, PuppetPin, WidthProfile};
use vectorcraft_geom::{Affine, Homography, PathData, Point, Rect};
use vectorcraft_tools::distort::liquify::{Dabber, LiquifyParams, PathStroke, Sample, reach_bounds};
use vectorcraft_tools::distort::perspective::{self as persp, PerspectiveGrid, Plane};
use vectorcraft_tools::distort::{PinSet, arap, collect_points, mesh_for, project_node, stroke_owner, warp_from_rest, warp_node_with};

use super::edit::{duplicate_in, selected_roots};
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stroke.widthPoint.set",
            "Width Point",
            [],
            None,
            "{id, t: 0..1 (fraction of the path length), left, right: side widths in pt, index?: width point to edit or move, adjustAdjoining?: bool (with index: the nearest points either side change their widths in proportion)} → {index}. Without index a point already at t is replaced; a point moved onto another one's t joins it as a discontinuous point (on the side it came from), so the width steps there. Creates a uniform profile first if needed",
            has_doc,
            width_point_set
        ),
        cmd!(
            "stroke.widthPoint.remove",
            "Delete Width Point",
            [],
            None,
            "{id, index | indices: [..]} remove width points (removing the last one leaves a uniform stroke)",
            has_doc,
            width_point_remove
        ),
        cmd!(
            "stroke.widthPoint.copy",
            "Copy Width Point",
            [],
            None,
            "{id, index: the width point to copy, t: 0..1 where the copy goes} → {index} (Alt-drag with the Width tool). Landing on another point makes a discontinuous point, as a stroke.widthPoint.set move does",
            has_doc,
            width_point_copy
        ),
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
            "{tool: warp|twirl|pucker|bloat|scallop|crystallize|wrinkle, points: [[x,y,pressure?]…] (the brush stroke; pen pressure 0..1, default 1), diameter?|width?, height? (pt, default 100), angle?, intensity? (0..1 or %), usePressure? (each point's pressure is the intensity there), detail? (1..10), simplify? (0..100) and simplifyOn? (default true; warp, twirl, pucker, bloat), rate? (twirl °), complexity? (0..15), horizontal?, vertical? (wrinkle, 0..1 or %), affectAnchors?, affectIn?, affectOut? (scallop, crystallize, wrinkle; default true), ids? (default: selection, else every path the brush touches)}",
            has_doc,
            liquify
        ),
        cmd!(
            "object.puppetWarp",
            "Puppet Warp",
            [],
            None,
            "{id?|ids? (default: selection), pins: [[x,y]…] (current pin positions), moved: [[x,y]…] (targets, same length), angles?: [deg|null…] (same length: the turn the art takes around each pin, as Alt-dragging around a pin does; null leaves it free), expand?: pt, rest?: bool} as-rigid-as-possible mesh warp of anchors and handles. rest: true = the Puppet Warp tool's pins: `pins` are points on the shape the pins started from (object.puppetWarp.pins), each must be on its mesh, the warp replaces the previous one instead of adding to it, and the pins are kept with the document for Undo (an empty list removes them all, leaving the art as it is)",
            has_doc,
            puppet_warp
        ),
        cmd!(
            "object.puppetWarp.pins",
            "Puppet Warp Pins",
            [],
            None,
            "{id?|ids? (default: selection), expand?: pt} → {ids, pins, moved, angles, expand, rest: true, auto} the Puppet Warp pins on the art: pins = where each sits on the shape they started from, moved = where it is now, angles = the turn it holds (deg, null = free); auto: none placed yet (the tool's automatic pins: the centre and the end of each limb). Change moved/angles (or add/remove pins) and pass it back to object.puppetWarp",
            has_doc,
            puppet_pins
        ),
        cmd!(
            "perspective.grid.set",
            "Define Perspective Grid",
            [],
            None,
            "{kind?: 1|2|3, origin?: [x,y], horizon?, vpLeft?, vpRight?, vpVertical?: [x,y], distance?, cell?, extent?, height?, visible?, plane?, leftOffset?, rightOffset?, groundOffset?: pt (planes moved along their normals), reproject?: bool} merge into the document's grid. Objects in perspective stay where they are, as in the reference app; with reproject they move with the grid (each keeps its place on its plane)",
            has_doc,
            grid_set
        ),
        cmd!(
            "perspective.grid.preset",
            "Perspective Grid Preset",
            [],
            None,
            "{kind: 1|2|3 (its normal view) | name: a preset (see perspective.presets.list)} reset the grid to the preset fitted to the first artboard (and show it; attached objects stay attached)",
            has_doc,
            super::perspgrid::grid_preset
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
            "{ids?, from: [x,y], to: [x,y], plane?, copy?: bool (move copies, as Alt-dragging does), perpendicular?: bool (move along the plane's normal instead, as pressing 5 while dragging does), snap?: bool (default: View › Perspective Grid › Snap to Grid)} slide objects within their plane (unattached objects attach to `plane`/the active plane first); snapping lands the nearer edge of the objects' joint bounds on a gridline within a quarter cell; Transform Again repeats it",
            has_doc,
            persp_move
        ),
        cmd!(
            "perspective.draw",
            "Draw in Perspective",
            [],
            None,
            "{command: shape.* id, params, plane?, snap?: bool (default: Snap to Grid)} run a shape command and attach the result to the plane (snapping its corners to gridlines within a quarter cell)",
            has_doc,
            persp_draw
        ),
        cmd!(
            "perspective.transform",
            "Transform in Perspective",
            [],
            None,
            "{ids?, matrix: [a,b,c,d,e,f] (affine map in plane coordinates, points: u along the plane, v up or away), depth?: pt to move along the plane's normal (default 0), copy?: bool, plane?} transform objects within their own planes (Perspective Selection tool handles; unattached objects attach to `plane`/the active plane first); Transform Again repeats it → {ids}",
            has_doc,
            persp_transform
        ),
        cmd!(
            "perspective.nudge",
            "Nudge in Perspective",
            [],
            None,
            "{dx, dy: arrow direction (-1, 0 or 1), big?: bool (×10), copy?: bool} move the selection in perspective by the keyboard increment, as the arrow keys do with the Perspective Selection tool → {ids}",
            has_selection,
            persp_nudge
        ),
        cmd!(
            "perspective.plane.move",
            "Move Plane",
            [],
            None,
            "{plane?: left|right|ground (default: the active plane), offset?: pt (where along its normal; 0 is its place in the grid's definition) | by?: pt, objects?: none|move|copy (default none: the objects on the plane stay; move: they move with it, as Shift-dragging a plane widget does; copy: copies of them do, as Alt-dragging does)} → {plane, offset, ids}",
            has_doc,
            plane_move
        ),
        cmd!(
            "perspective.plane.matchObject",
            "Move Plane to Match Object",
            [],
            None,
            "{id? (default: the first selected object)} move the plane the object is attached to onto the object and make it the active plane → {plane, offset}",
            has_doc,
            plane_match
        ),
        cmd!(
            "perspective.editText",
            "Edit Text",
            [],
            None,
            "{id? (default: the selected type)} Object › Perspective › Edit Text: show type in perspective flat where it is drawn, in isolation mode, to edit it (text.* commands, the Type tool); exiting isolation (object.exitIsolation, Esc) projects it again → {id}",
            has_doc,
            edit_text
        ),
    ]
}

// ---------- width points ----------

fn uniform() -> WidthProfile {
    WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] }
}

/// Width points closer than this (in t) are at the same place: a discontinuous point.
const SAME_T: f64 = 1e-6;

/// The stroke of `id` that width point commands edit (a compound path's for its members).
fn stroke_mut(d: &mut Document, id: NodeId) -> Result<&mut vectorcraft_doc::StrokeLayer> {
    let id = stroke_owner(d, id);
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    n.appearance.stroke_mut().ok_or_else(|| EngineError::Other("the object has no stroke".into()))
}

/// [`stroke_mut`], which must have a weight.
fn weighted_stroke(d: &mut Document, id: NodeId) -> Result<&mut vectorcraft_doc::StrokeLayer> {
    let st = stroke_mut(d, id)?;
    if st.width <= 0.0 {
        return Err(EngineError::Other("the stroke has no weight".into()));
    }
    Ok(st)
}

/// The `index` param (none when absent or null), checked against the points of `prof`: a malformed
/// one is refused like one out of range, never wrapped round or taken as missing.
fn point_index(p: &Value, prof: &WidthProfile, cmd: &str) -> Result<Option<usize>> {
    let Some(v) = p.get("index").filter(|v| !v.is_null()) else { return Ok(None) };
    match point_indices(&json!([v])).and_then(|i| i.first().copied()) {
        Some(i) if i < prof.points.len() => Ok(Some(i)),
        _ => Err(bad(cmd, "no such width point")),
    }
}

/// Insert width point `pt` in order. Landing on another point's position makes a discontinuous
/// point: `pt` snaps to it and takes the side it came from (`from_below`: before it), replacing
/// that side of a discontinuous point already there. → where it landed.
fn place(points: &mut Vec<(f64, f64, f64)>, mut pt: (f64, f64, f64), from_below: bool) -> usize {
    let at: Vec<usize> = (0..points.len()).filter(|i| (points[*i].0 - pt.0).abs() <= SAME_T).collect();
    let (Some(&first), Some(&last)) = (at.first(), at.last()) else {
        let pos = points.iter().position(|q| q.0 > pt.0).unwrap_or(points.len());
        points.insert(pos, pt);
        return pos;
    };
    pt.0 = points[first].0;
    let pair = at.len() > 1;
    let pos = match (from_below, pair) {
        (true, _) => first,
        (false, true) => last,
        (false, false) => last + 1,
    };
    if pair {
        points.remove(pos);
    }
    points.insert(pos, pt);
    pos
}

/// Adjust Adjoining Width Points: point `i` changes from `old` to `new` widths, and the nearest
/// points at other positions either side change in proportion (by the same amount from zero).
fn adjust_adjoining(points: &mut [(f64, f64, f64)], i: usize, old: (f64, f64, f64), new: (f64, f64, f64)) {
    let t = points[i].0;
    let scale = |v: f64, o: f64, n: f64| if o > 1e-9 { v * n / o } else { v + n - o }.max(0.0);
    let before = (0..i).rev().find(|j| (points[*j].0 - t).abs() > SAME_T);
    let after = (i + 1..points.len()).find(|j| (points[*j].0 - t).abs() > SAME_T);
    for j in before.into_iter().chain(after) {
        let q = &mut points[j];
        (q.1, q.2) = (scale(q.1, old.1, new.1), scale(q.2, old.2, new.2));
    }
}

fn width_point_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthPoint.set";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let t = f64_req(p, "t", C)?.clamp(0.0, 1.0);
    let (left, right) = (f64_req(p, "left", C)?, f64_req(p, "right", C)?);
    if !(left.is_finite() && right.is_finite()) || left < 0.0 || right < 0.0 || left.max(right) > 1.0e5 {
        return Err(bad(C, "left/right must be non-negative widths"));
    }
    let adjust = bool_or(p, "adjustAdjoining", false);
    let pos = s.edit("Width Point", |d, _| {
        let st = weighted_stroke(d, id)?;
        let half = st.width / 2.0;
        let mut prof = st.profile.take().unwrap_or_else(uniform);
        let pt = (t, left / half, right / half);
        let pos = match point_index(p, &prof, C)? {
            Some(i) => {
                let old = prof.points[i];
                if adjust {
                    adjust_adjoining(&mut prof.points, i, old, pt);
                }
                if (old.0 - t).abs() <= SAME_T {
                    // Widths only: it keeps its place (and its side of a discontinuous point).
                    prof.points[i] = (old.0, pt.1, pt.2);
                    i
                } else {
                    prof.points.remove(i);
                    place(&mut prof.points, pt, old.0 < t)
                }
            }
            None => {
                prof.points.retain(|q| (q.0 - t).abs() > SAME_T);
                place(&mut prof.points, pt, true)
            }
        };
        st.profile = Some(prof);
        Ok(pos)
    })?;
    Ok(json!({ "index": pos }))
}

fn width_point_copy(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthPoint.copy";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let t = f64_req(p, "t", C)?.clamp(0.0, 1.0);
    let pos = s.edit("Copy Width Point", |d, _| {
        let st = weighted_stroke(d, id)?;
        let mut prof = st.profile.take().unwrap_or_else(uniform);
        let i = point_index(p, &prof, C)?.ok_or_else(|| bad(C, "missing index"))?;
        let (from, l, r) = prof.points[i];
        let pos = place(&mut prof.points, (t, l, r), from < t);
        st.profile = Some(prof);
        Ok(pos)
    })?;
    Ok(json!({ "index": pos }))
}

fn width_point_remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthPoint.remove";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let indices = match (p.get("indices"), p.get("index")) {
        (Some(a), _) => point_indices(a),
        (None, Some(i)) => point_indices(&json!([i])),
        (None, None) => return Err(bad(C, "missing index")),
    };
    // A malformed index fails the whole delete instead of being dropped while the others go.
    let mut indices = indices.ok_or_else(|| bad(C, "no such width point"))?;
    indices.sort_unstable();
    indices.dedup();
    s.edit("Delete Width Point", |d, _| {
        let st = stroke_mut(d, id)?;
        let fits = |pr: &&mut WidthProfile| indices.last().is_some_and(|i| *i < pr.points.len());
        let prof = st.profile.as_mut().filter(fits).ok_or_else(|| bad(C, "no such width point"))?;
        for i in indices.iter().rev() {
            prof.points.remove(*i);
        }
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
    let doc = &s.doc()?.doc;
    let mut ids: Vec<NodeId> = targets(s, p)?.into_iter().map(|id| stroke_owner(doc, id)).collect();
    ids.sort_unstable();
    ids.dedup();
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

/// A stroke's `[[x, y, pressure?]…]` samples (pressure 0..1, default 1).
fn samples_of(p: &Value, key: &str) -> Option<Vec<Sample>> {
    p.get(key)?
        .as_array()?
        .iter()
        .map(|q| Some((Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?), q.get(2).map_or(Some(1.0), Value::as_f64)?)))
        .collect()
}

/// What a liquify stroke may reach: the paths it can edit, each with the box no dab outside
/// changes it ([`reach_bounds`]), and the objects it leaves as they are (type, symbols, images,
/// graphs, meshes, and envelopes, repeats and blends with their contents), each with its bounds
/// and what it is. Hidden and locked objects, and guides, are neither.
#[derive(Default)]
struct Targets {
    paths: Vec<(NodeId, Rect)>,
    blocked: Vec<(NodeId, Rect, &'static str)>,
    /// Every object listed (selected roots may hold each other).
    seen: std::collections::HashSet<NodeId>,
}

/// What `n` is when Liquify can't distort it (and doesn't look inside it).
fn not_liquified(n: &Node) -> Option<&'static str> {
    Some(match &n.kind {
        NodeKind::Group { .. } if n.graph.is_some() => "graphs",
        NodeKind::Text(_) => "type",
        NodeKind::SymbolInstance { .. } => "symbols",
        NodeKind::Image(_) => "images",
        NodeKind::Mesh(_) => "meshes",
        NodeKind::Envelope { .. } => "envelopes",
        NodeKind::Repeat(_) => "repeats",
        NodeKind::PlacedDocument(_) => "placed documents",
        NodeKind::Blend { .. } => "blends",
        _ => return None,
    })
}

impl Targets {
    /// The targets under `roots` (the selection), or with none every visible object.
    fn of(d: &Document, roots: &[NodeId]) -> Self {
        let mut t = Self::default();
        if roots.is_empty() {
            d.layers.iter().for_each(|l| t.visit(l));
            return t;
        }
        for r in roots {
            let (Some(n), Some(chain)) = (d.node(*r), d.ancestry(*r)) else { continue };
            if !d.is_editable(*r) {
                continue;
            }
            // Inside a graph, an envelope, a repeat or a blend: that object is what is left alone.
            match chain.iter().filter_map(|a| d.node(*a)).find_map(|a| Some((a, not_liquified(a)?))) {
                Some((a, what)) => t.block(a, what),
                None => t.visit(n),
            }
        }
        t
    }

    fn block(&mut self, n: &Node, what: &'static str) {
        if let Some(b) = n.geometric_bounds()
            && self.seen.insert(n.id)
        {
            self.blocked.push((n.id, b, what));
        }
    }

    fn visit(&mut self, n: &Node) {
        if !n.visible || n.locked {
            return;
        }
        if let Some(what) = not_liquified(n) {
            return self.block(n, what);
        }
        match &n.kind {
            NodeKind::Path { guide: false, path, .. } => {
                if let Some(b) = reach_bounds(path)
                    && self.seen.insert(n.id)
                {
                    self.paths.push((n.id, b));
                }
            }
            _ => n.children().into_iter().flatten().for_each(|c| self.visit(c)),
        }
    }
}

/// Does a dab with box `bb` reach `b`?
fn reaches(bb: Rect, b: Rect) -> bool {
    b.inflate(1e-6, 1e-6).intersect(bb).area() > 0.0
}

/// A liquify stroke being applied: its dabs and the paths they reached so far, so a longer stroke
/// (the next sample of a drag) applies only the dabs it adds. It belongs to one document snapshot
/// (`base`, held so the pointer can't be reused) and one set of other parameters (`key`).
pub(crate) struct LiquifyStroke {
    base: Arc<Document>,
    key: Value,
    prm: LiquifyParams,
    dabs: Dabber,
    targets: Targets,
    /// The paths reached, by their index in `targets.paths`.
    paths: Vec<(usize, PathStroke)>,
    /// The objects left alone that the brush passed over, by their index in `targets.blocked`.
    skipped: Vec<usize>,
    /// Which of `targets.paths` and `targets.blocked` the brush reached.
    reached: (Vec<bool>, Vec<bool>),
}

impl LiquifyStroke {
    fn new(base: Arc<Document>, key: Value, prm: LiquifyParams, roots: &[NodeId]) -> Self {
        let targets = Targets::of(&base, roots);
        let reached = (vec![false; targets.paths.len()], vec![false; targets.blocked.len()]);
        Self { base, key, prm, dabs: Dabber::new(&prm), targets, paths: vec![], skipped: vec![], reached }
    }

    /// Start the targets dab box `bb` reaches.
    fn reach(&mut self, bb: Rect) {
        for (i, (id, b)) in self.targets.paths.iter().enumerate() {
            if let Some(r) = self.reached.0.get_mut(i).filter(|r| !**r && reaches(bb, *b)) {
                *r = true;
                let Some(path) = self.base.node(*id).and_then(|n| n.path_data()) else { continue };
                self.paths.push((i, PathStroke::new(path.clone(), id.0)));
            }
        }
        for (i, (_, b, _)) in self.targets.blocked.iter().enumerate() {
            if let Some(r) = self.reached.1.get_mut(i).filter(|r| !**r && reaches(bb, *b)) {
                *r = true;
                self.skipped.push(i);
            }
        }
    }

    /// Add `samples` to the stroke and apply the dabs they make.
    fn extend(&mut self, samples: &[Sample]) {
        let from = self.dabs.dabs.len();
        for s in samples {
            self.dabs.push(*s);
        }
        for i in from..self.dabs.dabs.len() {
            if let Some(c) = self.dabs.dabs.get(i).map(|d| d.c) {
                self.reach(self.prm.brush_bounds(c));
            }
        }
        if let Some(t) = self.dabs.tail() {
            self.reach(self.prm.brush_bounds(t.c));
        }
        for (_, ps) in &mut self.paths {
            ps.advance(&self.dabs.dabs, &self.prm);
        }
    }

    /// The paths the stroke changed (in document order) and their new geometry.
    fn results(&self) -> Vec<(NodeId, PathData)> {
        let tail = self.dabs.tail();
        let mut out: Vec<(usize, NodeId, PathData)> =
            self.paths.iter().filter_map(|(i, ps)| Some((*i, self.targets.paths.get(*i)?.0, ps.finish(&self.dabs.dabs, tail, &self.prm)?))).collect();
        out.sort_by_key(|(i, ..)| *i);
        out.into_iter().map(|(_, id, p)| (id, p)).collect()
    }

    /// The objects under the brush left as they are, and the status message about them.
    fn skipped(&self) -> (Vec<u64>, Option<String>) {
        let mut ids = vec![];
        let mut kinds: Vec<&str> = vec![];
        for (id, _, what) in self.skipped.iter().filter_map(|i| self.targets.blocked.get(*i)) {
            ids.push(id.0);
            if !kinds.contains(what) {
                kinds.push(what);
            }
        }
        let msg = (!ids.is_empty()).then(|| {
            let n = ids.len();
            format!(
                "{} left {n} object{} under the brush as {} ({}): it reshapes paths only",
                self.prm.kind.label(),
                if n == 1 { "" } else { "s" },
                if n == 1 { "it was" } else { "they were" },
                kinds.join(", ")
            )
        });
        (ids, msg)
    }
}

fn liquify(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.liquify";
    let prm = LiquifyParams::from_json(p).ok_or_else(|| bad(C, "missing or unknown `tool`"))?;
    let pts = samples_of(p, "points").filter(|v| !v.is_empty()).ok_or_else(|| bad(C, "points must be a non-empty [[x,y,pressure?]…] list"))?;
    if pts.iter().any(|(q, f)| !q.x.is_finite() || !q.y.is_finite() || !f.is_finite()) {
        return Err(bad(C, "points must be finite"));
    }
    let roots = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => s.doc()?.selection.objects.clone(),
    };
    let base = s.doc()?.doc.clone();
    let mut key = p.clone();
    if let Some(o) = key.as_object_mut() {
        o.remove("points");
        o.insert("__roots".into(), json!(roots.iter().map(|r| r.0).collect::<Vec<_>>()));
    }
    // A live drag: the same stroke as the last preview with more samples goes on from there.
    let mut stroke = match s.liquify_stroke.take() {
        Some(st) if Arc::ptr_eq(&st.base, &base) && st.key == key && pts.starts_with(st.dabs.samples()) => st,
        _ => Box::new(LiquifyStroke::new(base, key, prm, &roots)),
    };
    let done = stroke.dabs.samples().len();
    stroke.extend(pts.get(done..).unwrap_or_default());
    let results = stroke.results();
    let (skipped, warning) = stroke.skipped();
    let changed = s.edit(&format!("{} Tool", prm.kind.label()), |d, _| {
        let mut changed = vec![];
        for (id, new) in results {
            let Some(n) = d.node_mut(id) else { continue };
            if let NodeKind::Path { path, live, .. } = &mut n.kind {
                *path = new;
                *live = None;
                changed.push(id.0);
            }
        }
        Ok(changed)
    })?;
    if s.in_interaction() {
        s.liquify_stroke = Some(stroke);
    }
    let mut out = json!({ "ids": changed, "skipped": skipped });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

// ---------- puppet warp ----------

/// The objects a Puppet Warp command works on: `id`/`ids`, else `default`.
fn puppet_ids(p: &Value, default: impl FnOnce() -> Result<Vec<NodeId>>) -> Result<Vec<NodeId>> {
    let ids = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => default()?,
    };
    if ids.is_empty() {
        return Err(EngineError::Other("select the artwork to warp".into()));
    }
    Ok(ids)
}

fn puppet_warp(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.puppetWarp";
    let pins = points_of(p, "pins").ok_or_else(|| bad(C, "pins must be [[x,y]…]"))?;
    let moved = points_of(p, "moved").ok_or_else(|| bad(C, "moved must be [[x,y]…]"))?;
    let rest = bool_or(p, "rest", false);
    if (pins.is_empty() && !rest) || pins.len() != moved.len() {
        return Err(bad(C, "pins and moved must be non-empty lists of the same length"));
    }
    if pins.iter().chain(&moved).any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return Err(bad(C, "pins must be finite"));
    }
    let ids = if rest { puppet_ids(p, || Ok(s.doc()?.selection.objects.clone()))? } else { puppet_ids(p, || selected_roots(s))? };
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let angles: Vec<Option<f64>> = match p.get("angles") {
        None | Some(Value::Null) => vec![None; pins.len()],
        Some(Value::Array(a)) if a.len() == pins.len() => a.iter().map(|v| v.as_f64().filter(|d| d.is_finite()).map(f64::to_radians)).collect(),
        Some(_) => return Err(bad(C, "angles must be a list as long as pins (degrees or null)")),
    };
    let ids_json = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() });
    if rest {
        let pins: Vec<PuppetPin> = pins.iter().zip(&moved).zip(angles).map(|((r, a), angle)| PuppetPin { rest: *r, at: *a, angle }).collect();
        s.edit("Puppet Warp", |d, _| warp_from_rest(d, &ids, pins, expand).map_err(|e| bad(C, e)))?;
        return Ok(ids_json);
    }
    let mesh = mesh_for(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    let pins: Vec<arap::Pin> = pins.iter().zip(&moved).zip(angles).map(|((a, b), angle)| arap::Pin { angle, ..arap::Pin::new(*a, *b) }).collect();
    let deformed = arap::deform(&mesh, &pins);
    let f = |q: Point| mesh.map(&deformed, q);
    s.edit("Puppet Warp", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            warp_node_with(n, &f);
        }
        Ok(())
    })?;
    Ok(ids_json)
}

/// Forget the Puppet Warp pins: not an undo step, and a saved document stays saved (pins are
/// never saved).
pub(crate) fn drop_puppet_pins(st: &mut crate::DocState) {
    let clean = !st.is_dirty();
    Arc::make_mut(&mut st.doc).puppet = None;
    if clean {
        st.mark_saved();
    }
}

fn puppet_pins(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = puppet_ids(p, || Ok(s.doc()?.selection.objects.clone()))?;
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let set = PinSet::of(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    let mut v = set.params(&set.pins, expand);
    v["auto"] = json!(set.auto);
    Ok(v)
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
pub(crate) fn silent(s: &mut Session, f: impl FnOnce(&mut PerspectiveGrid)) -> Result<PerspectiveGrid> {
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
    let old = grid_of(&s.doc()?.doc);
    let g = old.merged(p).map_err(|e| bad("perspective.grid.set", e))?;
    let reproject = bool_or(p, "reproject", false);
    s.edit("Define Perspective Grid", |d, _| {
        if reproject {
            old.adopt_stored_attachments(d);
            for (id, plane, depth) in persp::attached_roots(d) {
                let h = old.homography_at(plane, depth).and_then(|h| h.inverse()).zip(g.homography_at(plane, depth));
                let (hi, h) = h.ok_or_else(|| EngineError::Other("an object in perspective would leave the edited grid".into()))?;
                let m = h.then_after(&hi);
                warp_checked(d, id, &m)?;
            }
        }
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
/// Map `n` with `h` (type and symbols in perspective keep their art and are drawn through it),
/// failing if any point would leave the plane's visible side.
fn warp_checked(d: &mut Document, id: NodeId, h: &Homography) -> Result<()> {
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    let mut pts = vec![];
    collect_points(n, &mut pts);
    if pts.iter().any(|q| h.apply(*q).is_none()) {
        return Err(EngineError::Other("the object would cross the horizon of the perspective plane".into()));
    }
    project_node(n, h);
    Ok(())
}

/// Attach `id` to `plane`: projected by `map`, else by the map that keeps its bounds' corners
/// (on the nearest gridlines with `snap`).
fn attach_in(d: &mut Document, g: &mut PerspectiveGrid, id: NodeId, plane: Plane, snap: bool, map: Option<Homography>) -> Result<()> {
    let b = d.node(id).ok_or(EngineError::NoNode(id))?.geometric_bounds().ok_or_else(|| EngineError::Other("the object has no geometry".into()))?;
    let m =
        map.or_else(|| g.attach_homography(plane, b, snap)).ok_or_else(|| EngineError::Other("the object is beyond the plane's horizon".into()))?;
    warp_checked(d, id, &m)?;
    persp::set_attachment(d.node_mut(id).ok_or(EngineError::NoNode(id))?, plane, g.offset(plane));
    g.attached.insert(id.0.to_string(), plane);
    Ok(())
}

fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = roots(s, p)?;
    let mut g = grid_of(&s.doc()?.doc);
    let plane = plane_param(&g, p, "perspective.attach")?;
    s.edit("Attach to Active Plane", |d, _| {
        for id in &ids {
            attach_in(d, &mut g, *id, plane, false, None)?;
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
            if let Some(n) = d.node_mut(*id) {
                persp::release(n);
            }
        }
        store_grid(d, &g);
        Ok(())
    })?;
    ok()
}

/// One object's share of a perspective edit: the page map for an object on `plane` at `depth`, and
/// the depth it ends at.
type PerspStep<'a> = &'a dyn Fn(&PerspectiveGrid, Plane, f64) -> Result<(Homography, f64)>;

/// Where an object is in perspective: its plane and depth.
type Attached = (Plane, f64);

fn beyond_horizon() -> EngineError {
    EngineError::Other("the pointer is beyond the plane's horizon".into())
}

/// Edit objects in perspective (one undo step `label`): with `copy` their duplicates (which stay
/// attached), each mapped by `step` for its own plane and depth; objects not in perspective attach
/// to `fallback` first. → the edited objects and where the first one was (plane, depth).
fn persp_edit(
    s: &mut Session,
    label: &str,
    ids: &[NodeId],
    fallback: Option<Plane>,
    copy: bool,
    step: PerspStep,
) -> Result<(Vec<NodeId>, Option<Attached>)> {
    if ids.is_empty() {
        return Err(EngineError::Other("select the objects to transform in perspective".into()));
    }
    let mut g = grid_of(&s.doc()?.doc);
    s.edit(label, |d, sel| {
        g.adopt_stored_attachments(d);
        let targets = if copy { duplicate_in(d, sel, ids, Affine::IDENTITY)? } else { ids.to_vec() };
        let mut first = None;
        for id in &targets {
            let (plane, depth) = match g.attachment_of(d, *id) {
                Some(a) => a,
                None => {
                    let pl = fallback.ok_or_else(|| EngineError::Other("the object isn't on a perspective plane".into()))?;
                    attach_in(d, &mut g, *id, pl, false, None)?;
                    (pl, g.offset(pl))
                }
            };
            first.get_or_insert((plane, depth));
            let (h, depth) = step(&g, plane, depth)?;
            warp_checked(d, *id, &h)?;
            persp::set_attachment(d.node_mut(*id).ok_or(EngineError::NoNode(*id))?, plane, depth);
        }
        store_grid(d, &g);
        Ok((targets, first))
    })
}

/// Remember `again` (`perspective.transform` params) for Object › Transform › Transform Again, once
/// the drag that made it is committed.
fn record_again(s: &mut Session, again: Value) -> Result<()> {
    let st = s.doc_mut()?;
    match &mut st.interaction {
        Some(it) => it.perspective_again = Some(again),
        None => st.last_perspective = Some(again),
    }
    Ok(())
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// A plane-space affine map as `perspective.transform`'s `matrix` param.
fn matrix_json(m: Affine) -> Value {
    json!(m.as_coeffs())
}

fn persp_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.move";
    let ids = roots(s, p)?;
    let from = point_param(p, "from").ok_or_else(|| bad(C, "missing from"))?;
    let to = point_param(p, "to").ok_or_else(|| bad(C, "missing to"))?;
    if ![from.x, from.y, to.x, to.y].iter().all(|v| v.is_finite()) {
        return Err(bad(C, "from and to must be finite"));
    }
    let fallback = plane_param(&grid_of(&s.doc()?.doc), p, C).ok();
    let (perp, copy) = (bool_or(p, "perpendicular", false), bool_or(p, "copy", false));
    // Snap to Grid: a correction for the objects on the first one's plane and depth.
    let fix = if perp { None } else { super::perspgrid::snap_fix(&s.doc()?.doc, &ids, p, (from, to)) };
    let snapped = move |pl: Plane, depth: f64, dv: vectorcraft_geom::Vec2| {
        dv + fix.filter(|f| f.0 == pl && f.1 == depth).map_or(vectorcraft_geom::Vec2::ZERO, |f| f.2)
    };
    // In-plane: the plane-space offset under the pointer; perpendicular: the depth it reaches.
    let step = |g: &PerspectiveGrid, pl: Plane, depth: f64| -> Result<(Homography, f64)> {
        if perp {
            let to_depth = g.depth_at(pl, depth, from, to).ok_or_else(beyond_horizon)?;
            Ok((g.transform_map(pl, depth, Affine::IDENTITY, to_depth - depth).ok_or_else(beyond_horizon)?, to_depth))
        } else {
            let dv = snapped(pl, depth, g.plane_delta(pl, depth, from, to).ok_or_else(beyond_horizon)?);
            Ok((g.transform_map(pl, depth, Affine::translate(dv), 0.0).ok_or_else(beyond_horizon)?, depth))
        }
    };
    let label = if copy { "Copy in Perspective" } else { "Move in Perspective" };
    let (targets, first) = persp_edit(s, label, &ids, fallback, copy, &step)?;
    // Transform Again repeats the first object's move.
    if let Some((pl, depth)) = first {
        let g = grid_of(&s.doc()?.doc);
        let again = match perp {
            true => g.depth_at(pl, depth, from, to).map(|d| json!({"matrix": matrix_json(Affine::IDENTITY), "depth": d - depth, "copy": copy})),
            false => {
                g.plane_delta(pl, depth, from, to).map(|dv| json!({"matrix": matrix_json(Affine::translate(snapped(pl, depth, dv))), "copy": copy}))
            }
        };
        if let Some(a) = again {
            record_again(s, a)?;
        }
    }
    Ok(ids_json(&targets))
}

fn persp_transform(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.transform";
    let m = matrix_param(p, "matrix").ok_or_else(|| bad(C, "missing matrix [a,b,c,d,e,f]"))?;
    let dz = f64_or(p, "depth", 0.0);
    if !m.as_coeffs().iter().all(|v| v.is_finite()) || m.determinant().abs() < 1e-12 || !dz.is_finite() {
        return Err(bad(C, "matrix must be finite and invertible, depth finite"));
    }
    let ids = roots(s, p)?;
    let fallback = plane_param(&grid_of(&s.doc()?.doc), p, C).ok();
    let copy = bool_or(p, "copy", false);
    let step = |g: &PerspectiveGrid, pl: Plane, depth: f64| -> Result<(Homography, f64)> {
        let to = depth + dz;
        if to.abs() > persp::MAX_DEPTH {
            return Err(EngineError::Other("the objects would leave the grid".into()));
        }
        Ok((g.transform_map(pl, depth, m, dz).ok_or_else(beyond_horizon)?, to))
    };
    let (targets, _) = persp_edit(s, if copy { "Copy in Perspective" } else { "Transform in Perspective" }, &ids, fallback, copy, &step)?;
    record_again(s, json!({"matrix": matrix_json(m), "depth": dz, "copy": copy}))?;
    Ok(ids_json(&targets))
}

/// Object › Transform › Transform Again after a perspective move or scale: the same plane-space
/// transform on the selection.
pub(crate) fn transform_again(s: &mut Session, again: &Value) -> Result<Value> {
    let mut p = again.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("ids");
    }
    persp_transform(s, &p)
}

fn plane_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.plane.move";
    let old = grid_of(&s.doc()?.doc);
    let plane = plane_param(&old, p, C)?;
    let from = old.offset(plane);
    let to = match (p.get("offset").and_then(Value::as_f64), p.get("by").and_then(Value::as_f64)) {
        (Some(o), _) => o,
        (None, Some(b)) => from + b,
        (None, None) => return Err(bad(C, "give offset or by")),
    };
    let objects = str_param(p, "objects").unwrap_or("none");
    if !matches!(objects, "none" | "move" | "copy") {
        return Err(bad(C, "objects must be none, move or copy"));
    }
    let mut g = old.clone();
    g.set_offset(plane, to);
    g.validate().map_err(|e| bad(C, e))?;
    let label = match objects {
        "copy" => "Move Plane and Copy Objects",
        "move" => "Move Plane and Objects",
        _ => "Move Plane",
    };
    let ids = s.edit(label, |d, sel| {
        old.adopt_stored_attachments(d);
        let mut ids = vec![];
        if objects != "none" {
            // The objects on the plane where it was (not those moved off it along its normal).
            let on: Vec<NodeId> = persp::attached_roots(d)
                .into_iter()
                .filter(|(id, pl, depth)| *pl == plane && (depth - from).abs() < 1e-6 && d.is_editable(*id))
                .map(|(id, ..)| id)
                .collect();
            ids = if objects == "copy" { duplicate_in(d, sel, &on, Affine::IDENTITY)? } else { on };
            let m = old.transform_map(plane, from, Affine::IDENTITY, to - from).ok_or_else(beyond_horizon)?;
            for id in &ids {
                warp_checked(d, *id, &m)?;
                persp::set_attachment(d.node_mut(*id).ok_or(EngineError::NoNode(*id))?, plane, to);
            }
        }
        store_grid(d, &g);
        Ok(ids)
    })?;
    Ok(json!({ "plane": plane.id(), "offset": to, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn plane_match(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let id = id_param(p, "id")
        .or_else(|| st.selection.objects.first().copied())
        .ok_or_else(|| EngineError::Other("select an object in perspective".into()))?;
    let mut g = grid_of(&st.doc);
    let (plane, depth) = g.attachment_of(&st.doc, id).ok_or_else(|| EngineError::Other("the object isn't in perspective".into()))?;
    g.set_offset(plane, depth);
    g.plane = plane;
    g.validate().map_err(|e| bad("perspective.plane.matchObject", e))?;
    s.edit("Move Plane to Match Object", |d, _| {
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(json!({ "plane": plane.id(), "offset": depth }))
}

fn persp_nudge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.nudge";
    let (dx, dy) = (f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    if !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
        return Err(bad(C, "dx and dy give the arrow's direction"));
    }
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let ids = selected_roots(s)?;
    // The step is measured on the page at the centre of the selection's perspective box.
    let st = s.doc()?;
    let g = grid_of(&st.doc);
    let (plane, depth, rect) = g.plane_bounds(&st.doc, &ids).ok_or_else(|| EngineError::Other("the selection isn't in perspective".into()))?;
    let from = g.homography_at(plane, depth).and_then(|h| h.apply(rect.center())).ok_or_else(beyond_horizon)?;
    let to = from + vectorcraft_geom::Vec2::new(dx.clamp(-1.0, 1.0), dy.clamp(-1.0, 1.0)) * k;
    persp_move(
        s,
        &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "from": [from.x, from.y], "to": [to.x, to.y], "copy": bool_or(p, "copy", false)}),
    )
}

fn persp_draw(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.draw";
    let command = str_param(p, "command").filter(|c| c.starts_with("shape.")).ok_or_else(|| bad(C, "command must be a shape.* command"))?.to_string();
    let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    let mut g = grid_of(&s.doc()?.doc);
    let plane = plane_param(&g, p, C)?;
    let snap = bool_or(p, "snap", g.snap);
    // A click-to-size dialog's shape keeps its sizes in plane units from the click.
    let map = match point_param(p, "at") {
        Some(at) => Some(g.size_homography(plane, at, snap).ok_or_else(beyond_horizon)?),
        None => None,
    };
    let undo_before = s.doc()?.history.undo.len();
    let r = s.execute(&command, &params)?;
    let id = r.get("id").and_then(Value::as_u64).map(NodeId).ok_or_else(|| EngineError::Other(format!("{command} didn't create an object")))?;
    let res = s.edit("Draw in Perspective", |d, _| {
        attach_in(d, &mut g, id, plane, snap, map)?;
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
    const SHAPES: &[&str] = &[
        "shape.rectangle",
        "shape.ellipse",
        "shape.polygon",
        "shape.star",
        "shape.line",
        "shape.rectangularGrid",
        "shape.arc",
        "shape.flare",
        "shape.spiral",
        "shape.polarGrid",
    ];
    if !SHAPES.contains(&cmd) {
        return None;
    }
    let g = PerspectiveGrid::current(&s.active()?.doc);
    (g.visible && g.plane != Plane::None).then(|| ("perspective.draw".to_string(), json!({"command": cmd, "params": params})))
}

/// A click-to-size shape dialog's command (`cmd` with `params`, clicked at `at`) as it runs: on the
/// active plane while the grid shows (`perspective.draw` keeping the sizes in plane units from the
/// click), else `None` (run it as it is).
pub fn perspective_click(s: &Session, cmd: &str, params: &Value, at: Point) -> Option<(String, Value)> {
    let (c, mut p) = perspective_rewrite(s, cmd, params)?;
    p["at"] = json!([at.x, at.y]);
    Some((c, p))
}

/// Object › Perspective › Edit Text: type in perspective shown flat where it is drawn, in isolation
/// mode, to be edited; exiting isolation projects it again ([`finish_edit_text`]).
fn edit_text(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let id =
        id_param(p, "id").or_else(|| st.selection.objects.first().copied()).ok_or_else(|| EngineError::Other("select type in perspective".into()))?;
    if !st.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)) && n.projection().is_some()) {
        return Err(EngineError::Other("select type in perspective".into()));
    }
    s.edit("Edit Text", |d, _| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        let h = n.projection().ok_or_else(|| EngineError::Other("select type in perspective".into()))?;
        let NodeKind::Text(t) = &mut n.kind else { return Err(EngineError::Other("select type in perspective".into())) };
        // Shown flat where it is drawn: the type moves there and the projection moves the other
        // way, so the picture stays.
        let (Some(flat), Some(drawn)) = (t.bounds(), t.bounds().and_then(|b| h.map_rect_bbox(b))) else { return Ok(()) };
        let shift = vectorcraft_geom::Affine::translate(drawn.center() - flat.center());
        t.transform(shift);
        if let Some(rec) = n.perspective.as_deref_mut() {
            rec.projection = Some(h.then_after(&Homography::from_affine(shift.inverse())).to_array());
            rec.editing = true;
        }
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.isolation = Some(id);
    st.selection.set([id]);
    st.revision += 1;
    Ok(json!({ "id": id.0 }))
}

/// Edit Text ends (isolation mode on `id` exits): the type is projected again, in the document and
/// in every undo state (the flat view is never an undo step of its own).
pub(crate) fn finish_edit_text(st: &mut crate::DocState, id: NodeId) {
    let clear = |doc: &mut Arc<Document>| {
        if doc.node(id).and_then(|n| n.perspective.as_deref()).is_some_and(|p| p.editing)
            && let Some(p) = Arc::make_mut(doc).node_mut(id).and_then(|n| n.perspective.as_deref_mut())
        {
            p.editing = false;
        }
    };
    let saved = Arc::ptr_eq(&st.doc, &st.saved_doc);
    clear(&mut st.doc);
    if saved {
        st.saved_doc = st.doc.clone();
    }
    for e in st.history.undo.iter_mut().chain(st.history.redo.iter_mut()) {
        clear(&mut e.doc);
    }
    if let Some(it) = &mut st.interaction {
        clear(&mut it.doc);
    }
    st.revision += 1;
}
