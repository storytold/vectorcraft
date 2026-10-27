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

use super::edit::{duplicate_in, outermost, selected_roots};
use super::*;
use crate::EngineError;

mod perspective;

pub use perspective::perspective_click;
use perspective::{
    attach, edit_text, grid_set, grid_show, persp_draw, persp_move, persp_nudge, persp_transform, plane_match, plane_move, plane_set, release,
};
pub(crate) use perspective::{finish_edit_text, grid_of, perspective_rewrite, silent, store_grid, transform_again};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stroke.widthPoint.set",
            "Width Point",
            [],
            None,
            "{id, t: 0..1 (fraction of the path length), left, right: side widths in pt, index?: width point to edit or move, adjustAdjoining?: bool (the nearest points either side change their widths in proportion, using the sampled widths for a new point)} → {index}. Without index a point already at t is replaced; a point moved onto another one's t joins it as a discontinuous point (on the side it came from), so the width steps there. Creates a uniform profile first if needed",
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
            "{tool: warp|twirl|pucker|bloat|scallop|crystallize|wrinkle, points: [[x,y,pressure?]…] (the brush stroke; pen pressure 0..1, default 1), diameter?|width?, height? (pt, default 100), angle?, intensity? (0..1 or %), usePressure?: bool (each point's pressure is the intensity there), detail? (1..10), simplify?: 0..100, simplifyOn?: bool (default true; both for warp, twirl, pucker, bloat), rate? (twirl °), complexity? (0..15), horizontal?, vertical? (wrinkle, 0..1 or %), affectAnchors?, affectIn?, affectOut?: bool (scallop, crystallize, wrinkle; default true), ids?|id? (default: selection, else every path the brush touches)}",
            has_doc,
            liquify
        ),
        cmd!(
            "object.puppetWarp",
            "Puppet Warp",
            [],
            None,
            "{id?|ids? (default: selection), pins: [[x,y]…] (current pin positions), moved: [[x,y]…] (targets, same length), angles?: [deg|null…] (same length: the turn the art takes around each pin, as Alt-dragging around a pin does; null leaves it free), expand?: pt, rest?: bool, auto?: bool (as object.puppetWarp.pins reports; ignored)} as-rigid-as-possible mesh warp of anchors and handles. rest: true = the Puppet Warp tool's pins: `pins` are points on the shape the pins started from (object.puppetWarp.pins), each must be on its mesh, the warp replaces the previous one instead of adding to it, and the pins are kept with the document for Undo (an empty list removes them all, leaving the art as it is)",
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
            "{kind?: 1|2|3, origin?: [x,y], horizon?, vpLeft?, vpRight?, vpVertical?: [x,y], distance?, cell?, extent?, height?, visible?, plane?, leftOffset?, rightOffset?, groundOffset?: pt (planes moved along their normals), reproject?: bool, …the other fields of the grid as its reply lists them} merge into the document's grid. Objects in perspective stay where they are, as in the reference app; with reproject they move with the grid (each keeps its place on its plane)",
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
            "{ids?|id?, plane?: left|right|ground (default: the active plane)} project the objects onto the plane",
            has_doc,
            attach
        ),
        cmd!("perspective.release", "Release with Perspective", [], None, "{ids?|id?} detach from the grid (geometry unchanged)", has_doc, release),
        cmd!(
            "perspective.move",
            "Move in Perspective",
            [],
            None,
            "{ids?|id?, from: [x,y], to: [x,y], plane?, copy?: bool (move copies, as Alt-dragging does), perpendicular?: bool (move along the plane's normal instead, as pressing 5 while dragging does), snap?: bool (default: View › Perspective Grid › Snap to Grid)} slide objects within their plane (unattached objects attach to `plane`/the active plane first); snapping lands the nearer edge of the objects' joint bounds on a gridline within a quarter cell; Transform Again repeats it",
            has_doc,
            persp_move
        ),
        cmd!(
            "perspective.draw",
            "Draw in Perspective",
            [],
            None,
            "{command: shape.* id, params, plane?, snap?: bool (default: Snap to Grid), at?: [x, y] (a point of the plane the shape lies on, as the tool gives it)} run a shape command and attach the result to the plane (snapping its corners to gridlines within a quarter cell)",
            has_doc,
            persp_draw
        ),
        cmd!(
            "perspective.transform",
            "Transform in Perspective",
            [],
            None,
            "{ids?|id?, matrix: [a,b,c,d,e,f] (affine map in plane coordinates, points: u along the plane, v up or away), depth?: pt to move along the plane's normal (default 0), copy?: bool, plane?} transform objects within their own planes (Perspective Selection tool handles; unattached objects attach to `plane`/the active plane first); Transform Again repeats it → {ids}",
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
    let scale = |v: f64, o: f64, n: f64| {
        let width = if o > 1e-9 {
            let product = v * n;
            if product.is_finite() {
                // Keep the existing rounding, including small products that n/o would lose.
                product / o
            } else {
                // With finite nonnegative factors, an overflowing product has both factors
                // above one. Divide the larger first so this fallback cannot underflow.
                (v.max(n) / o) * v.min(n)
            }
        } else {
            v + n - o
        };
        width.max(0.0)
    };
    let before = (0..i).rev().find(|j| (points[*j].0 - t).abs() > SAME_T);
    let after = (i + 1..points.len()).find(|j| (points[*j].0 - t).abs() > SAME_T);
    for j in before.into_iter().chain(after) {
        let q = &mut points[j];
        (q.1, q.2) = (scale(q.1, old.1, new.1), scale(q.2, old.2, new.2));
    }
}

/// Whether `a` and `b` hold the same width points, each place and width within a rounding error
/// (1e-9).
fn same_points(a: Option<&WidthProfile>, b: Option<&WidthProfile>) -> bool {
    let near = |p: &(f64, f64, f64), q: &(f64, f64, f64)| (p.0 - q.0).abs() <= 1e-9 && (p.1 - q.1).abs() <= 1e-9 && (p.2 - q.2).abs() <= 1e-9;
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.points.len() == b.points.len() && a.points.iter().zip(&b.points).all(|(p, q)| near(p, q)),
        _ => false,
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
    let adjust = switch_or(p, "adjustAdjoining", false, C)?;
    // When the profile comes out as it was, within a rounding error (1e-9), the document and its undo
    // history stay as they are.
    let mut pos = 0;
    super::overprint::edit_counted(s, "Width Point", |d| {
        let st = weighted_stroke(d, id)?;
        let before = st.profile.clone();
        let half = st.width / 2.0;
        let pt = (t, left / half, right / half);
        if !half.is_finite() || half <= 0.0 || !pt.1.is_finite() || !pt.2.is_finite() {
            return Err(bad(C, "width factors must be finite"));
        }
        let mut prof = st.profile.take().unwrap_or_else(uniform);
        pos = match point_index(p, &prof, C)? {
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
                let old = adjust.then(|| prof.at(t));
                prof.points.retain(|q| (q.0 - t).abs() > SAME_T);
                let i = place(&mut prof.points, pt, true);
                if let Some((left, right)) = old {
                    adjust_adjoining(&mut prof.points, i, (t, left, right), pt);
                }
                i
            }
        };
        // Validate on edit_counted's copy: a failed adjoining adjustment must not change
        // the document, its dirty state or either side of its history.
        if prof.points.iter().any(|p| !p.1.is_finite() || !p.2.is_finite()) {
            return Err(bad(C, "width factors must be finite"));
        }
        st.profile = Some(prof);
        Ok(usize::from(!same_points(st.profile.as_ref(), before.as_ref())))
    })?;
    Ok(json!({ "index": pos }))
}

fn width_point_copy(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthPoint.copy";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let t = f64_req(p, "t", C)?.clamp(0.0, 1.0);
    // When the profile comes out as it was (the copy replaces a side of a discontinuous point that has
    // its widths already), the document and its undo history stay as they are.
    let mut pos = 0;
    super::overprint::edit_counted(s, "Copy Width Point", |d| {
        let st = weighted_stroke(d, id)?;
        let before = st.profile.clone();
        let mut prof = st.profile.take().unwrap_or_else(uniform);
        let i = point_index(p, &prof, C)?.ok_or_else(|| bad(C, "missing index"))?;
        let (from, l, r) = prof.points[i];
        pos = place(&mut prof.points, (t, l, r), from < t);
        st.profile = Some(prof);
        Ok(usize::from(st.profile != before))
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
                let v: Vec<f64> = q.as_array().and_then(|x| x.iter().map(Value::as_f64).collect()).unwrap_or_default();
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
    // When every stroke has the profile already, or there is no stroke, the document and its undo
    // history stay as they are.
    if ids.iter().all(|id| doc.node(*id).is_some_and(|n| n.appearance.stroke().is_none_or(|st| st.profile == profile))) {
        return ok();
    }
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
/// and what it is. Hidden and locked objects, the art on a locked artboard, and guides are
/// neither.
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
        NodeKind::Group { .. } if n.extras().graph.is_some() => "graphs",
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
        let locks = d.locked_artboards();
        if roots.is_empty() {
            d.layers.iter().for_each(|l| t.visit(l, &locks));
            return t;
        }
        let editable = |id| d.is_editable_with(&locks, id);
        for r in roots {
            let (Some(n), Some(chain)) = (d.node(*r), d.ancestry(*r)) else { continue };
            if !editable(*r) {
                continue;
            }
            // Inside a graph, an envelope, a repeat or a blend: that object is what is left alone.
            match chain.iter().filter_map(|a| d.node(*a)).find_map(|a| Some((a, not_liquified(a)?))) {
                Some((a, what)) => t.block(a, what),
                None => t.visit(n, &locks),
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

    fn visit(&mut self, n: &Node, locks: &vectorcraft_doc::LockedArtboards) {
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
            // The objects of a layer on a locked artboard, members and all, are left out.
            _ => n.children().into_iter().flatten().filter(|c| !(n.is_layer() && locks.holds(c))).for_each(|c| self.visit(c, locks)),
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

/// The number keys of `object.liquify`. [`LiquifyParams::from_json`] reads a value that is not a
/// number as the key left out.
const LIQUIFY_NUMBERS: &[&str] =
    &["diameter", "width", "height", "angle", "intensity", "detail", "simplify", "rate", "complexity", "horizontal", "vertical"];

fn liquify(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.liquify";
    check_numbers(p, LIQUIFY_NUMBERS, C)?;
    let prm = LiquifyParams::from_json(p).ok_or_else(|| bad(C, "missing or unknown `tool`"))?;
    // `LiquifyParams::from_json` uses a key's default for a value that is not true or false.
    check_switches(p, &["usePressure", "simplifyOn", "affectAnchors", "affectIn", "affectOut"], C)?;
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
    // When the stroke reshapes no path, the document and its undo history stay as they are.
    let changed = if results.is_empty() {
        vec![]
    } else {
        s.edit(&format!("{} Tool", prm.kind.label()), |d, _| {
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
        })?
    };
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
    check_numbers(p, &["expand"], C)?;
    let pins = points_of(p, "pins").ok_or_else(|| bad(C, "pins must be [[x,y]…]"))?;
    let moved = points_of(p, "moved").ok_or_else(|| bad(C, "moved must be [[x,y]…]"))?;
    let rest = switch_or(p, "rest", false, C)?;
    if (pins.is_empty() && !rest) || pins.len() != moved.len() {
        return Err(bad(C, "pins and moved must be non-empty lists of the same length"));
    }
    if pins.iter().chain(&moved).any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return Err(bad(C, "pins must be finite"));
    }
    let ids =
        if rest { puppet_ids(p, || Ok(s.doc()?.selection.objects.clone()))? } else { outermost(&s.doc()?.doc, puppet_ids(p, || selected_roots(s))?) };
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let angles: Vec<Option<f64>> = match p.get("angles") {
        None | Some(Value::Null) => vec![None; pins.len()],
        Some(Value::Array(a)) if a.len() == pins.len() && a.iter().all(|v| v.is_null() || v.is_number()) => {
            a.iter().map(|v| v.as_f64().filter(|d| d.is_finite()).map(f64::to_radians)).collect()
        }
        Some(_) => return Err(bad(C, "angles must be a list as long as pins (degrees or null)")),
    };
    let ids_json = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() });
    if rest {
        let pins: Vec<PuppetPin> = pins.iter().zip(&moved).zip(angles).map(|((r, a), angle)| PuppetPin { rest: *r, at: *a, angle }).collect();
        // When the art and its pins come out as they were, the document and its undo history stay
        // as they are.
        let before = s.doc()?.doc.clone();
        super::overprint::edit_counted(s, "Puppet Warp", |d| {
            warp_from_rest(d, &ids, pins, expand).map_err(|e| bad(C, e))?;
            Ok(usize::from(d.puppet != before.puppet || ids.iter().any(|id| d.node(*id) != before.node(*id))))
        })?;
        return Ok(ids_json);
    }
    let mesh = mesh_for(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    // When every pin stays where it is and turns the art by 0° or is free, the document and its undo
    // history stay as they are.
    if pins == moved && angles.iter().all(|a| a.is_none_or(|a| a == 0.0)) {
        return Ok(ids_json);
    }
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
/// never saved, [`crate::DocState::change_unsaved`]).
pub(crate) fn drop_puppet_pins(st: &mut crate::DocState) {
    st.change_unsaved(|d| Arc::make_mut(d).puppet = None);
}

fn puppet_pins(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["expand"], "object.puppetWarp.pins")?;
    let ids = puppet_ids(p, || Ok(s.doc()?.selection.objects.clone()))?;
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let set = PinSet::of(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    let mut v = set.params(&set.pins, expand);
    v["auto"] = json!(set.auto);
    Ok(v)
}
