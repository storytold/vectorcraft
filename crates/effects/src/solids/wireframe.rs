//! Expand as Wireframes: the edges of the faces a 3D effect draws, as open and closed paths that
//! can be edited on their own.
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeId};
use vectorcraft_geom::{Anchor, CubicBez, PathData, Point, SubPath};
use vectorcraft_three_d::Facet;

use super::{
    INFLATE, Profile, ROTATE, facets, geometry, has_solid, inflate_options, is_container, is_plane, is_three_d, label_of, members, object_profile,
    region, reshape_members, resolve, resolve_for, solid, solid_paints, solid_parts, union_bounds, view_options,
};
use crate::{GeomContext, is_geometry};

/// The most distinct edges one object's wireframe may have.
pub(crate) const MAX_WIRE_EDGES: usize = 1_000_000;

/// The grid points are rounded to before edges are compared, in points: two faces that share an
/// edge give it once.
const GRID: f64 = 1.0 / 1024.0;

const NO_EFFECT: &str = "the object has no 3D effect (Extrude & Bevel, Revolve, Inflate or Rotate)";

/// The wireframe of `n`'s 3D effect (Extrude & Bevel, Revolve, Inflate, Rotate or one of their 3D
/// (Classic) forms), in document coordinates: the edges of every face the effect draws for the object as
/// it is turned and projected, each edge once. Edges that meet end to end are joined into one path;
/// a run that comes back to where it started is closed. A curved edge stays a curve. The geometry
/// effects after the object's 3D effect reshape the wireframe as they reshape the drawn art. On a
/// group or layer, the edges of the solids of all its members. An error when `n` has no visible 3D
/// effect, when its solids can't be built, when the canvas draws the source art in their place, when
/// they draw no face, or past [`MAX_WIRE_EDGES`] edges.
pub fn three_d_wireframe_in(doc: Option<&Document>, n: &Node) -> Result<Vec<PathData>, String> {
    if !crate::has_revolve(n) && !has_solid(n) {
        return Err(NO_EFFECT.into());
    }
    let label = n.appearance.effects.iter().find(|e| e.visible && is_three_d(&e.id)).map_or("This effect", |e| label_of(&e.id));
    // The faces come from the effect whose art the canvas draws (Revolve first, as the canvas tries
    // it first). When neither draws, past their limits, the canvas shows the source art and there
    // is no surface to expand.
    let revolve = crate::has_revolve(n) && crate::revolve_art_in(doc, n).is_some();
    if !revolve && !(has_solid(n) && super::solid_art_in(doc, n).is_some()) {
        // The builders' own error says why, when there is one. Revolve's art is cut for Revolve,
        // as the canvas cuts it.
        if crate::has_revolve(n) {
            crate::revolve::revolve_face_paths(doc, &*resolve_for(doc, n, label, false)?)?;
        } else {
            solid_face_paths(doc, &*resolve(doc, n, label)?)?;
        }
        return Err(format!("{label} draws the source art for this object instead of its 3D surface"));
    }
    // Revolve's art is cut for Revolve whatever other 3D effect the object carries, as the canvas
    // cuts it ([`crate::revolve_art_in`]).
    let n = &*if revolve { resolve_for(doc, n, label, false)? } else { resolve(doc, n, label)? };
    let faces = if revolve { crate::revolve::revolve_face_paths(doc, n)? } else { solid_face_paths(doc, n)? };
    let faces = after_geometry(doc, n, faces, revolve);
    let wire = edges(&faces)?;
    if wire.is_empty() {
        return Err(format!("{label} draws no faces for this object"));
    }
    Ok(wire)
}

/// `faces` reshaped by the visible geometry effects after the 3D effect of `n`'s own that drew them
/// ([`reshaped`]): its first visible Revolve when `revolve` is set, else its first visible Extrude &
/// Bevel, Inflate or Rotate.
fn after_geometry(doc: Option<&Document>, n: &Node, faces: Vec<PathData>, revolve: bool) -> Vec<PathData> {
    let fx = &n.appearance.effects;
    let drawn = if revolve { fx.iter().position(|e| e.visible && crate::revolve::is_revolve(&e.id)) } else { solid(fx).map(|(i, _)| i) };
    let Some(index) = drawn else { return faces };
    reshaped(faces, fx.get(index + 1..).unwrap_or_default(), &GeomContext { doc, ..GeomContext::of(n) })
}

/// `faces` reshaped by the visible geometry effects in `later` (the effects after a 3D effect), as
/// they reshape the faces of the drawn surface ([`crate::reshape::reshape_leaves`]): each effect
/// reshapes each face on its own, about the bounds all the faces have before it, and the faces keep
/// to one budget of anchors together. `ctx` gives the object's id, the seed of a random Transform,
/// and the document.
pub(crate) fn reshaped(faces: Vec<PathData>, later: &[Effect], ctx: &GeomContext) -> Vec<PathData> {
    let after: Vec<&Effect> = later.iter().filter(|e| e.visible && is_geometry(&e.id)).collect();
    if after.is_empty() {
        return faces;
    }
    let id = NodeId(ctx.seed);
    let fill = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
    let mut art = Node::group(id, faces.into_iter().map(|f| Arc::new(Node::path(id, f, fill.clone()))).collect());
    crate::reshape::reshape_leaves(&mut art, &after, later, ctx.doc);
    let mut out = vec![];
    art.walk(&mut |c| out.extend(c.path_data().cloned()));
    out
}

/// The outline of every face [`super::solid_art`] draws for `n` (already resolved): with the effect
/// on the object, its solids; with it on fills and strokes, the solid of each; on a group or layer,
/// the solids of its members, about one shared axis. The swatches and patterns named by a live-effect
/// plug-in after a fill's or stroke's 3D effect are looked up in `doc`.
fn solid_face_paths(doc: Option<&Document>, n: &Node) -> Result<Vec<PathData>, String> {
    if is_container(n) {
        let (index, fx) = solid(&n.appearance.effects).ok_or("a 3D effect on a group's fill or stroke draws nothing")?;
        let mut shapes = members(n, label_of(&fx.id))?;
        reshape_members(n, n.appearance.effects.get(..index).unwrap_or_default(), &mut shapes);
        return facet_paths(fx, &shapes);
    }
    let label = n.appearance.effects.iter().find(|e| e.visible && is_three_d(&e.id)).map_or("This effect", |e| label_of(&e.id));
    let (path, rule) = object_profile(n, label)?;
    let ctx = GeomContext::of(n);
    let paints = solid_paints(n);
    if let Some((index, fx)) = solid(&n.appearance.effects) {
        let path = geometry(&path, n.appearance.effects.get(..index).unwrap_or_default(), &ctx);
        return facet_paths(fx, &[Profile { path, rule, items: paints, opacity: 1.0, scopes: vec![] }]);
    }
    let path = geometry(&path, &n.appearance.effects, &ctx);
    let mut out = vec![];
    for item in &paints {
        if let Some((index, fx)) = solid(item.effects()) {
            let p = geometry(&path, item.effects().get(..index).unwrap_or_default(), &ctx.item(item));
            let faces = facet_paths(fx, &[Profile { path: p, rule, items: vec![item.clone()], opacity: 1.0, scopes: vec![] }])?;
            out.extend(reshaped(faces, item.effects().get(index + 1..).unwrap_or_default(), &GeomContext { doc, ..ctx }));
        }
    }
    Ok(out)
}

/// The outlines of the facets effect `fx` builds from `shapes`, as `art_for` builds them: Rotate and
/// Rotate (Classic) turn every fill and stroke; Extrude & Bevel and Inflate build a solid from each shape's
/// top fill and from the outline of each stroke. A stroke whose solid is past the limits is left out,
/// as it is left out of the drawn art.
fn facet_paths(fx: &Effect, shapes: &[Profile]) -> Result<Vec<PathData>, String> {
    let label = label_of(&fx.id);
    let b = union_bounds(shapes).ok_or_else(|| format!("{label} requires a shape with at least two anchors"))?;
    let p = crate::merged_params(&fx.id, &fx.params);
    let mut out = vec![];
    if is_plane(&fx.id) {
        for s in shapes {
            for item in &s.items {
                let (area, rule) = region(&s.path, s.rule, item);
                out.extend(facets(ROTATE, &area, rule, b, &p)?.iter().map(Facet::path));
            }
        }
        return Ok(out);
    }
    let (parts, bounds) = solid_parts(shapes, b);
    let built: Vec<Result<Vec<Facet>, String>> = if fx.id == INFLATE {
        let shapes: Vec<(&PathData, vectorcraft_geom::FillRule)> = parts.iter().map(|(_, _, area, rule, ..)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_together(&shapes, bounds, inflate_options(&p), view_options(&p))
    } else {
        parts.iter().map(|(_, _, area, rule, ..)| facets(&fx.id, area, *rule, bounds, &p)).collect()
    };
    for ((item, ..), f) in parts.iter().zip(built) {
        match f {
            Ok(f) => out.extend(f.iter().map(Facet::path)),
            Err(_) if matches!(item, AppearanceItem::Stroke(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// A point on the [`GRID`].
type Snap = (i64, i64);

fn snap(p: Point) -> Snap {
    ((p.x / GRID).round() as i64, (p.y / GRID).round() as i64)
}

fn reversed(c: CubicBez) -> CubicBez {
    CubicBez::new(c.p3, c.p2, c.p1, c.p0)
}

/// The edges of `faces`, each once, joined end to end into paths ([`three_d_wireframe_in`]).
pub(crate) fn edges(faces: &[PathData]) -> Result<Vec<PathData>, String> {
    let mut seen: HashSet<[Snap; 4]> = HashSet::new();
    let mut segments: Vec<CubicBez> = vec![];
    for sp in faces.iter().flat_map(|f| &f.subpaths) {
        for i in 0..sp.segment_count() {
            let mut c = sp.segment(i);
            if sp.segment_is_line(i) {
                c = CubicBez::new(c.p0, c.p0, c.p3, c.p3);
            }
            if [c.p0, c.p1, c.p2, c.p3].iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                continue;
            }
            let key = [c.p0, c.p1, c.p2, c.p3].map(snap);
            // A segment that starts and ends on one grid point with its handles there too draws nothing.
            if key.iter().all(|k| *k == key[0]) {
                continue;
            }
            let back = [key[3], key[2], key[1], key[0]];
            if seen.contains(&back) || !seen.insert(key) {
                continue;
            }
            if segments.len() >= MAX_WIRE_EDGES {
                return Err(format!("the wireframe has more than {MAX_WIRE_EDGES} edges"));
            }
            segments.push(c);
        }
    }
    Ok(chain(&segments))
}

/// `segments` joined end to end: each path starts at the first segment not yet used and takes in
/// the unused segments that meet its ends until none is left at either end, or it closes.
fn chain(segments: &[CubicBez]) -> Vec<PathData> {
    let mut at: HashMap<Snap, Vec<usize>> = HashMap::new();
    for (i, c) in segments.iter().enumerate() {
        at.entry(snap(c.p0)).or_default().push(i);
        if snap(c.p3) != snap(c.p0) {
            at.entry(snap(c.p3)).or_default().push(i);
        }
    }
    let mut used = vec![false; segments.len()];
    // The next unused segment at `point`, turned to leave it (`leave`) or to arrive at it.
    let mut take = |used: &mut Vec<bool>, point: Snap, leave: bool| -> Option<CubicBez> {
        let list = at.get_mut(&point)?;
        while let Some(&i) = list.last() {
            if used.get(i).copied().unwrap_or(true) {
                list.pop();
                continue;
            }
            if let Some(u) = used.get_mut(i) {
                *u = true;
            }
            let c = *segments.get(i)?;
            let starts = snap(c.p0) == point;
            return Some(if starts == leave { c } else { reversed(c) });
        }
        None
    };
    let mut out = vec![];
    for (i, first) in segments.iter().enumerate() {
        if used.get(i).copied().unwrap_or(true) {
            continue;
        }
        if let Some(u) = used.get_mut(i) {
            *u = true;
        }
        let mut run: VecDeque<CubicBez> = VecDeque::from([*first]);
        // A path of one anchor has no segment (`SubPath::segment_count`), so a run of one segment
        // that comes back to its start stays open, with its two ends on one point.
        let closes = |run: &VecDeque<CubicBez>| run.len() > 1 && run.front().zip(run.back()).is_some_and(|(f, b)| snap(f.p0) == snap(b.p3));
        while !closes(&run) {
            let Some(end) = run.back().map(|c| snap(c.p3)) else { break };
            let Some(c) = take(&mut used, end, true) else { break };
            run.push_back(c);
        }
        while !closes(&run) {
            let Some(start) = run.front().map(|c| snap(c.p0)) else { break };
            let Some(c) = take(&mut used, start, false) else { break };
            run.push_front(c);
        }
        out.push(PathData::single(subpath(&run, closes(&run))));
    }
    out
}

/// `run`, segments that meet end to end, as one subpath; a closed one ends at its first anchor.
fn subpath(run: &VecDeque<CubicBez>, closed: bool) -> SubPath {
    let mut anchors: Vec<Anchor> = vec![];
    if let Some(first) = run.front() {
        let h_in = if closed { run.back().map_or(first.p0, |b| b.p2) } else { first.p0 };
        anchors.push(Anchor::with_handles(first.p0, h_in, first.p1));
    }
    for (k, c) in run.iter().enumerate() {
        let h_out = run.get(k + 1).map_or(c.p3, |next| next.p1);
        anchors.push(Anchor::with_handles(c.p3, c.p2, h_out));
    }
    // A closed run's last anchor is its first, whose in handle already comes from the last segment.
    if closed {
        anchors.pop();
    }
    SubPath::new(anchors, closed)
}

#[cfg(test)]
mod tests;
