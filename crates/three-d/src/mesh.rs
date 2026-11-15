//! The solids of the 3D effects as triangle meshes, for files that other 3D apps open (OBJ, glTF,
//! USD). A mesh holds the faces the effect builds before they are projected onto the page: every
//! face, those turned away from the viewer included, at the turn the effect gives the art.
//!
//! Coordinates are points with y up and z toward the viewer, about the point the solid turns about
//! (the centre of the art's bounds; for Revolve, the axis at the profile's middle). Each face keeps
//! its own corners and normal, so the mesh is shaded flat. Planar faces with holes (caps and the
//! turned plane) are cut into triangles: each hole is joined to its outline by a bridge, then the
//! outline is cut by ear clipping.

use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::solid::{Face3, Keep, extrude_faces, inflate_faces, plane_faces};
use crate::{Extrude, Facet, Inflate, Revolve, V3, View};

/// Most steps the triangulation of one solid may take: a face's ears and bridges each test the
/// face's other corners.
const MAX_WORK: usize = 400_000_000;

/// A triangle mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// Corner positions in points: x right, y up, z toward the viewer.
    pub positions: Vec<[f64; 3]>,
    /// One unit normal for each position: the normal of the face the corner belongs to.
    pub normals: Vec<[f64; 3]>,
    /// Corner indices, counterclockwise seen from the side the normals point to.
    pub triangles: Vec<[u32; 3]>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The smallest and largest coordinate on each axis; `None` for an empty mesh.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold((first, first), |(lo, hi), p| {
            ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])
        }))
    }

    /// The meshes of `parts` as one, in order.
    pub fn join(parts: impl IntoIterator<Item = Mesh>) -> Mesh {
        let mut out = Mesh::default();
        for m in parts {
            let base = out.positions.len() as u32;
            out.positions.extend(m.positions);
            out.normals.extend(m.normals);
            out.triangles.extend(m.triangles.into_iter().map(|t| t.map(|i| i + base)));
        }
        out
    }
}

/// Extrude & Bevel's solid ([`crate::extrude`]) as a mesh.
pub fn extrude_mesh(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude, view: View) -> Result<Mesh, String> {
    extrude_faces(path, rule, bounds, options, view, Keep::Faces).and_then(|(_, faces, _)| mesh(&faces?))
}

/// Rotate's plane ([`crate::plane`]) as a mesh.
pub fn plane_mesh(path: &PathData, rule: FillRule, bounds: Rect, view: View) -> Result<Mesh, String> {
    plane_faces(path, rule, bounds, view, Keep::Faces).and_then(|(_, faces, _)| mesh(&faces?))
}

/// Inflate's solids for several shapes of one object ([`crate::inflate_together`]) as meshes, one
/// result for each shape, in order. Each mesh is cut as the iterator reaches it.
pub fn inflate_meshes(
    shapes: &[(&PathData, FillRule)],
    bounds: Rect,
    options: Inflate,
    view: View,
) -> impl Iterator<Item = Result<Mesh, String>> + use<> {
    inflate_faces(shapes, bounds, options, view, Keep::Faces).map(|r| r.and_then(|(_, faces, _)| mesh(&faces?)))
}

/// One solid built once for both its drawing and its mesh: the facets [`crate::extrude`] or
/// [`crate::inflate_each`] gives, and the faces before projection that [`MeshedSolid::mesh`] cuts
/// into the mesh [`extrude_mesh`] or [`inflate_meshes`] gives.
#[derive(Clone, Debug)]
pub struct MeshedSolid {
    pub facets: Vec<Facet>,
    /// The faces before projection, or the error the mesh alone gives when there are too many.
    faces: Result<Vec<Face3>, String>,
}

/// A solid with no facets and an empty mesh.
impl Default for MeshedSolid {
    fn default() -> Self {
        Self { facets: vec![], faces: Ok(vec![]) }
    }
}

impl MeshedSolid {
    /// The solid as a mesh. The faces are cut into triangles on each call.
    pub fn mesh(&self) -> Result<Mesh, String> {
        mesh(self.faces.as_ref().map_err(String::clone)?)
    }
}

/// Extrude & Bevel's solid ([`crate::extrude`]) and its mesh ([`extrude_mesh`]) from one build. An
/// error where [`crate::extrude`] gives one.
pub fn extrude_meshed(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude, view: View) -> Result<MeshedSolid, String> {
    extrude_faces(path, rule, bounds, options, view, Keep::Both).map(|(facets, faces, _)| MeshedSolid { facets, faces })
}

/// Inflate's solids for several shapes of one object ([`crate::inflate_each`]) and their meshes
/// ([`inflate_meshes`]), each solid built once, as the iterator reaches it. An error where
/// [`crate::inflate_each`] gives one.
pub fn inflate_meshed(
    shapes: &[(&PathData, FillRule)],
    bounds: Rect,
    options: Inflate,
    view: View,
) -> impl Iterator<Item = Result<MeshedSolid, String>> + use<> {
    inflate_faces(shapes, bounds, options, view, Keep::Both).map(|r| r.map(|(facets, faces, _)| MeshedSolid { facets, faces }))
}

/// Revolve's surface ([`crate::revolve`]) as a mesh. As the effect draws it, a sweep of less than
/// 360° caps its ends with sectors and leaves its two cut sides open ([`Revolve::cap_ends`]). The
/// walls of holes under `rule`, such as the counter of an O, face into the hole.
pub fn revolve_mesh(path: &PathData, rule: FillRule, bounds: Rect, options: Revolve) -> Result<Mesh, String> {
    let mut faces = vec![];
    crate::revolve_faces(path, rule, bounds, options, Some(&mut faces))?;
    mesh(&faces)
}

/// The area-weighted normal of `contours` (Newell's method).
fn newell(contours: &[Vec<V3>]) -> V3 {
    let mut n = V3::default();
    for c in contours {
        for (a, b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
            let x = a.cross(*b);
            n = V3(n.0 + x.0, n.1 + x.1, n.2 + x.2);
        }
    }
    n
}

/// From the effects' view space (y down) to the mesh's (y up).
fn up(v: V3) -> [f64; 3] {
    [v.0, -v.1, v.2]
}

/// `faces` (view space) as one mesh, each cut into triangles turned to its normal, which is written
/// at unit length whatever length it is given (a face given no normal takes its corners' own).
fn mesh(faces: &[Face3]) -> Result<Mesh, String> {
    let mut out = Mesh::default();
    let mut work = 0usize;
    for (contours, normal) in faces {
        let geometric = newell(contours);
        let given = normal.unit();
        let n = if given.dot(given) > 0.5 { given } else { geometric.unit() };
        if n.dot(n) < 0.5 {
            continue;
        }
        let [nx, ny, nz] = up(n);
        for [a, b, c] in triangulate(contours, geometric, &mut work)? {
            let (pa, pb, pc) = (up(a), up(b), up(c));
            let cross = V3(pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]).cross(V3(pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]));
            if cross.dot(cross) < 1e-24 {
                continue;
            }
            let (pb, pc) = if cross.dot(V3(nx, ny, nz)) < 0.0 { (pc, pb) } else { (pb, pc) };
            let base = u32::try_from(out.positions.len()).map_err(|_| "The 3D object is too complex to export".to_string())?;
            out.positions.extend([pa, pb, pc]);
            out.normals.extend([[nx, ny, nz]; 3]);
            out.triangles.push([base, base + 1, base + 2]);
        }
    }
    Ok(out)
}

/// A corner of a face: its position on the plane the face is cut in, and in 3D.
#[derive(Clone, Copy)]
struct Corner {
    x: f64,
    y: f64,
    v: V3,
}

fn area(ring: &[Corner]) -> f64 {
    ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

fn inside(ring: &[Corner], x: f64, y: f64) -> bool {
    let mut odd = false;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()) {
        if (a.y > y) != (b.y > y) && x < a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x) {
            odd = !odd;
        }
    }
    odd
}

fn too_complex() -> String {
    "The 3D object is too complex to export".to_string()
}

fn crossing_hole() -> String {
    "The 3D object has a hole that crosses its outline or another hole".to_string()
}

/// Count `steps` of work against [`MAX_WORK`].
fn spend(work: &mut usize, steps: usize) -> Result<(), String> {
    *work = work.saturating_add(steps);
    if *work > MAX_WORK { Err(too_complex()) } else { Ok(()) }
}

/// The triangles covering one face: `contours` (outlines one way round, holes the other) on the
/// plane across `normal` (its Newell normal).
fn triangulate(contours: &[Vec<V3>], normal: V3, work: &mut usize) -> Result<Vec<[V3; 3]>, String> {
    // Cut the face on the coordinate plane it is least slanted to, turned so outlines run
    // counterclockwise.
    let (ax, ay, az) = (normal.0.abs(), normal.1.abs(), normal.2.abs());
    let flat = |v: V3| -> (f64, f64) {
        if az >= ax && az >= ay {
            if normal.2 >= 0.0 { (v.0, v.1) } else { (v.1, v.0) }
        } else if ax >= ay {
            if normal.0 >= 0.0 { (v.1, v.2) } else { (v.2, v.1) }
        } else if normal.1 >= 0.0 {
            (v.2, v.0)
        } else {
            (v.0, v.2)
        }
    };
    let mut outlines: Vec<(Vec<Corner>, f64)> = vec![];
    let mut holes: Vec<Vec<Corner>> = vec![];
    for c in contours {
        let mut ring: Vec<Corner> = c
            .iter()
            .map(|v| {
                let (x, y) = flat(*v);
                Corner { x, y, v: *v }
            })
            .collect();
        ring.dedup_by(|a, b| (a.x - b.x).hypot(a.y - b.y) < 1e-9);
        while ring.len() > 1 && ring.first().zip(ring.last()).is_some_and(|(a, b)| (a.x - b.x).hypot(a.y - b.y) < 1e-9) {
            ring.pop();
        }
        if ring.len() < 3 {
            continue;
        }
        let a = area(&ring);
        if a > 1e-12 {
            outlines.push((ring, a));
        } else if a < -1e-12 {
            holes.push(ring);
        }
    }
    // Each hole goes to the smallest outline around it.
    let mut held: Vec<Vec<Vec<Corner>>> = vec![vec![]; outlines.len()];
    for hole in holes {
        let corners: usize = outlines.iter().map(|(o, _)| o.len()).sum();
        spend(work, corners.saturating_mul(hole.len() + 1))?;
        // A hole that crosses an outline can't be cut out of it; leaving it would fill it.
        let edges = |r: &[Corner]| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(p, q)| ((p.x, p.y), (q.x, q.y))).collect::<Vec<_>>();
        let sides = edges(&hole);
        if outlines.iter().any(|(o, _)| edges(o).iter().any(|(c, d)| sides.iter().any(|(a, b)| crosses(*a, *b, *c, *d)))) {
            return Err(crossing_hole());
        }
        let Some(first) = hole.first() else { continue };
        let owner =
            outlines.iter().enumerate().filter(|(_, (o, _))| inside(o, first.x, first.y)).min_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|(i, _)| i);
        if let Some(i) = owner
            && let Some(h) = held.get_mut(i)
        {
            h.push(hole);
        }
    }
    let mut out = vec![];
    for ((outline, _), mut holes) in outlines.into_iter().zip(held) {
        // The rightmost hole first, as each bridge then reaches the outline past the others.
        holes.sort_by(|a, b| {
            let right = |h: &[Corner]| h.iter().map(|c| c.x).fold(f64::NEG_INFINITY, f64::max);
            right(b).total_cmp(&right(a))
        });
        let mut ring = outline;
        for k in 0..holes.len() {
            let (hole, rest) = holes[k..].split_first().ok_or_else(too_complex)?;
            ring = bridge(ring, hole, rest, work)?;
        }
        clip(&ring, &mut out, work)?;
    }
    Ok(out)
}

/// Do segments `a`–`b` and `c`–`d` cross (touching at a shared end does not count)?
fn crosses(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let same = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1) < 1e-9;
    if same(a, c) || same(a, d) || same(b, c) || same(b, d) {
        return false;
    }
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (d1, d2, d3, d4) = (side(c, d, a), side(c, d, b), side(a, b, c), side(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Does point `p` lie on segment `a`–`b`, short of its ends?
fn touches(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return false;
    }
    let along = ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len;
    let off = ((p.1 - a.1) * dx - (p.0 - a.0) * dy).abs() / len;
    off < 1e-9 && along > 1e-9 && along < len - 1e-9
}

/// `ring` with `hole` joined to it: from the hole's rightmost corner to the nearest corner of the
/// ring that a straight bridge reaches inside the face: it crosses no edge of the ring, the hole or
/// the `others`, passes through none of their other corners, and its middle is inside the ring and
/// outside the holes. The bridge runs there and back, so the result is one ring.
fn bridge(ring: Vec<Corner>, hole: &[Corner], others: &[Vec<Corner>], work: &mut usize) -> Result<Vec<Corner>, String> {
    let Some(m) = (0..hole.len()).max_by(|a, b| hole[*a].x.total_cmp(&hole[*b].x)) else { return Ok(ring) };
    let at = (hole[m].x, hole[m].y);
    let mut order: Vec<usize> = (0..ring.len()).collect();
    order.sort_by(|a, b| {
        let d = |i: usize| (ring[i].x - at.0).hypot(ring[i].y - at.1);
        d(*a).total_cmp(&d(*b))
    });
    let edges = |r: &[Corner]| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(p, q)| ((p.x, p.y), (q.x, q.y))).collect::<Vec<_>>();
    let mut walls = edges(&ring);
    walls.extend(edges(hole));
    for o in others {
        walls.extend(edges(o));
    }
    let to = order.into_iter().find(|&i| {
        let p = (ring[i].x, ring[i].y);
        *work = work.saturating_add(2 * walls.len());
        if *work > MAX_WORK || walls.iter().any(|(c, d)| crosses(at, p, *c, *d) || touches(*c, at, p)) {
            return false;
        }
        let mid = ((at.0 + p.0) / 2.0, (at.1 + p.1) / 2.0);
        inside(&ring, mid.0, mid.1) && !inside(hole, mid.0, mid.1) && !others.iter().any(|o| inside(o, mid.0, mid.1))
    });
    if *work > MAX_WORK {
        return Err(too_complex());
    }
    // A hole no bridge reaches (one that crosses another hole) can't be cut out; leaving it would
    // fill it.
    let Some(i) = to else { return Err(crossing_hole()) };
    let mut out = Vec::with_capacity(ring.len() + hole.len() + 2);
    out.extend_from_slice(&ring[..=i]);
    out.extend(hole[m..].iter().chain(&hole[..=m]).copied());
    out.extend_from_slice(&ring[i..]);
    Ok(out)
}

/// Cut `ring` (counterclockwise, possibly joined to its holes by bridges) into triangles by ear
/// clipping. A corner in line with its neighbors goes without a triangle. If no ear is left (a
/// ring that crosses itself), the rest is cut as a fan.
fn clip(ring: &[Corner], out: &mut Vec<[V3; 3]>, work: &mut usize) -> Result<(), String> {
    let n = ring.len();
    if n < 3 {
        return Ok(());
    }
    let mut next: Vec<usize> = (1..=n).map(|i| i % n).collect();
    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let (mut left, mut i, mut tried) = (n, 0usize, 0usize);
    let cross = |a: &Corner, b: &Corner, c: &Corner| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let same = |a: &Corner, b: &Corner| (a.x - b.x).hypot(a.y - b.y) < 1e-12;
    while left > 3 {
        let (a, c) = (prev[i], next[i]);
        let (pa, pb, pc) = (&ring[a], &ring[i], &ring[c]);
        let turn = cross(pa, pb, pc);
        let scale = (pb.x - pa.x).hypot(pb.y - pa.y) * (pc.x - pb.x).hypot(pc.y - pb.y);
        let mut ear = false;
        let drop = turn.abs() <= 1e-12 * scale.max(1e-300);
        if !drop && turn > 0.0 {
            spend(work, left)?;
            ear = true;
            let mut k = next[c];
            while k != a {
                let p = &ring[k];
                if !same(p, pa) && !same(p, pb) && !same(p, pc) && cross(pa, pb, p) > 0.0 && cross(pb, pc, p) > 0.0 && cross(pc, pa, p) > 0.0 {
                    ear = false;
                    break;
                }
                k = next[k];
            }
        }
        if !ear && !drop && tried > left {
            // No ear anywhere: cut what is left as a fan.
            let start = i;
            let mut k = next[start];
            while next[k] != start {
                out.push([ring[start].v, ring[k].v, ring[next[k]].v]);
                k = next[k];
            }
            return Ok(());
        }
        if ear || drop {
            if ear {
                out.push([pa.v, pb.v, pc.v]);
            }
            next[a] = c;
            prev[c] = a;
            left -= 1;
            i = a;
            tried = 0;
        } else {
            i = c;
            tried += 1;
        }
    }
    let (a, c) = (prev[i], next[i]);
    out.push([ring[a].v, ring[i].v, ring[c].v]);
    Ok(())
}

#[cfg(test)]
mod tests;
