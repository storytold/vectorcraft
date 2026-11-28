use vectorcraft_geom::kurbo::ParamCurveDeriv;

use super::outline::{self, Along, Budget, CURVE_TOLERANCE};
use super::rings::{
    CURVE_PIECES, MAX_PIECES, MAX_PLANE_POINTS, MAX_SOLID_POINTS, Ring, Span, TOO_COMPLEX_SURFACE, collapse_width, curve_pieces, inset, left,
    signed_area, solid_rings_traced, square_corners,
};
use super::side;
use super::{
    Bevel, DepthPlane, Extrude, Facet, FillRule, MAX_FACES, MAX_SOLID_FACES, MapCell, ParamCurve, PathData, PlanarFace, Point, Rect, Revolve,
    SolidSurface, SurfaceKind, V3, View, eye_distance, perspective_scale,
};
use crate::mesh::MeshPart;
use crate::shadow::Projection;

/// Turns 3D points into document points and shades faces for one view.
pub(super) struct Camera {
    pub(super) view: Revolve,
    origin: Point,
    /// The eye's distance in front of the centre; `None` without perspective.
    eye: Option<f64>,
    /// Deeper than any part of the object: faces drawn first go this far behind it.
    far: f64,
    pub(super) light: V3,
}

impl Camera {
    pub(super) fn new(view: View, bounds: Rect, depth: f64) -> Self {
        let view = view.revolve();
        let [lx, ly, lz] = view.light_direction();
        let size = (bounds.width() / 2.0).max(bounds.height() / 2.0).max(depth / 2.0).max(1.0);
        Self { view, origin: bounds.center(), eye: eye_distance(view.perspective, size), far: 16.0 * size, light: V3(lx, ly, lz) }
    }
    pub(super) fn rotate(&self, v: V3) -> V3 {
        self.view.rotate(v)
    }
    /// A point of the art, `z` points in front of its plane, in view space.
    pub(super) fn place(&self, p: Point, z: f64) -> V3 {
        self.rotate(V3(p.x - self.origin.x, p.y - self.origin.y, z))
    }
    pub(super) fn project(&self, v: V3) -> Point {
        let f = perspective_scale(self.eye, v.2);
        Point::new(self.origin.x + v.0 * f, self.origin.y + v.1 * f)
    }
    /// The view-space point at depth `z` that [`Camera::project`] takes to `p`.
    pub(super) fn unproject(&self, p: Point, z: f64) -> V3 {
        let f = perspective_scale(self.eye, z);
        V3((p.x - self.origin.x) / f, (p.y - self.origin.y) / f, z)
    }
    /// The cosine between a face's outward unit normal `n` and the direction from `at` (both in
    /// view space) to the eye: positive when the face is turned towards the viewer.
    pub(super) fn facing(&self, at: V3, n: V3) -> f64 {
        let to_eye = self.eye.map_or(V3(0.0, 0.0, 1.0), |e| V3(0.0, 0.0, e).sub(at).unit());
        n.dot(to_eye)
    }
    pub(super) fn brightness(&self, n: V3) -> f64 {
        crate::brightness(&self.view, n, self.light)
    }
    /// The document point the view's center projects to, and the eye's distance in front of it
    /// (`None` without perspective).
    pub(super) fn center_and_eye(&self) -> (Point, Option<f64>) {
        (self.origin, self.eye)
    }
    /// The rotation and projection this camera draws with, for a shadow cast from its faces.
    pub(super) fn projection(&self) -> Projection {
        Projection { view: self.view, origin: self.origin, eye: self.eye }
    }
}

/// A face of a solid before projection: its contours in view space (holes running the other way),
/// its outward normal, the part of the solid it belongs to, and where its corners lie on the
/// part's texture ([`Attrs`]).
#[derive(Clone, Debug)]
pub(crate) struct Face3 {
    pub(crate) contours: Vec<Vec<V3>>,
    pub(crate) normal: V3,
    pub(crate) part: MeshPart,
    pub(crate) attrs: Attrs,
}

impl Face3 {
    /// The texture coordinates of corner `v` of the face, and its own normal when it has one (a
    /// corner of a smoothed bevel or of a puffed surface). A point that is no corner of the face
    /// lies at the middle of the texture.
    pub(crate) fn corner(&self, v: V3) -> ([f64; 2], Option<V3>) {
        match &self.attrs {
            Attrs::Planar(plane) => (plane.at(v), None),
            Attrs::Corners(list) => self
                .contours
                .iter()
                .zip(list)
                .flat_map(|(c, l)| c.iter().zip(l))
                .find(|(c, _)| **c == v)
                .map_or(([0.5, 0.5], None), |(_, (uv, n))| (*uv, *n)),
        }
    }
}

/// Where the corners of a face lie on its part's texture, from 0 to 1 each way (u to the right, v
/// down), and the normals of the corners that have their own.
#[derive(Clone, Debug)]
pub(crate) enum Attrs {
    /// Each corner's place on a plane ([`PlaneUv`]); every corner takes the face's normal.
    Planar(PlaneUv),
    /// For each corner of each contour, in order, its texture coordinates and its own normal
    /// (`None` takes the face's).
    Corners(Vec<Vec<([f64; 2], Option<V3>)>>),
}

/// A plane's texture coordinates: a point `p` on it lies at `origin + x·s + y·t`, and `(s, t)`
/// within `area` goes to 0..1 each way. `x` and `y` are square to each other. An axis on which
/// `area` has no extent gives 0.5.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlaneUv {
    pub(crate) origin: V3,
    pub(crate) x: V3,
    pub(crate) y: V3,
    pub(crate) area: Rect,
}

impl PlaneUv {
    pub(crate) fn at(&self, p: V3) -> [f64; 2] {
        let d = p.sub(self.origin);
        let along = |axis: V3| d.dot(axis) / axis.dot(axis);
        [share(along(self.x), self.area.x0, self.area.x1), share(along(self.y), self.area.y0, self.area.y1)]
    }
}

/// Where `v` lies from `lo` (0) to `hi` (1), within 0..1: 0.5 when the two are equal or `v` is not
/// finite.
pub(crate) fn share(v: f64, lo: f64, hi: f64) -> f64 {
    let w = hi - lo;
    if w > 1e-12 && v.is_finite() { ((v - lo) / w).clamp(0.0, 1.0) } else { 0.5 }
}

/// The bounds of the points of `rings`, or an empty rectangle at the origin when they have none.
pub(crate) fn rings_bounds<'a>(rings: impl IntoIterator<Item = &'a [Point]>) -> Rect {
    let mut points = rings.into_iter().flatten();
    let Some(first) = points.next() else { return Rect::ZERO };
    points.fold(Rect::from_points(*first, *first), |b, p| b.union_pt(*p))
}

/// The texture coordinate along each point of `ring` and back to its first point: the distance
/// along the ring from its first point over the ring's length, so the first point takes 0 and the
/// last entry (the first point again) 1. A ring of no length takes 0.5 throughout.
pub(crate) fn along_ring(ring: &[Point]) -> Vec<f64> {
    let n = ring.len();
    let mut at = Vec::with_capacity(n + 1);
    let mut sum = 0.0;
    for i in 0..=n {
        at.push(sum);
        if let (Some(a), Some(b)) = (ring.get(i % n.max(1)), ring.get((i + 1) % n.max(1))) {
            sum += a.distance(*b);
        }
    }
    let total = at.last().copied().unwrap_or(0.0);
    at.iter().map(|d| if total > 1e-12 { (d / total).clamp(0.0, 1.0) } else { 0.5 }).collect()
}

/// The texture coordinates and corner normals of the faces of one band ([`Builder::curved_band`]):
/// the coordinate along the ring at each of its points and back at its first ([`along_ring`]), the
/// coordinate across the band at its two rings, and, for each edge of the rings, the normal of its
/// corners on each ring when they are smoothed (`None` takes the face's).
pub(super) struct BandMesh<'a> {
    pub(super) part: MeshPart,
    pub(super) u: &'a [f64],
    pub(super) v: [f64; 2],
    pub(super) normals: Option<RowNormals<'a>>,
}

/// The smoothed normals of the corners of each piece of each edge on a band's two rings
/// ([`BandMesh`]).
pub(super) type RowNormals<'a> = (&'a [Vec<Option<V3>>], &'a [Vec<Option<V3>>]);

/// What a solid's builder makes: its facets, its faces before projection when a mesh is asked for
/// (an error when they pass [`MAX_SOLID_FACES`] while the facets are built too), and the
/// projection it used (`None` when nothing was built).
pub(crate) type Built = (Vec<Facet>, Result<Vec<Face3>, String>, Option<Projection>);

#[cfg(test)]
thread_local! {
    /// How many Extrude & Bevel solids this thread has built, for the tests.
    pub(crate) static EXTRUDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a solid's builder keeps: its facets, its faces before projection for a mesh, or both
/// from one build.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keep {
    Facets,
    Faces,
    Both,
}

/// Facets being collected for one solid.
pub(super) struct Builder<'a> {
    pub(super) camera: &'a Camera,
    /// A closed solid hides the faces turned away from the viewer; an open one shows their
    /// insides.
    pub(super) closed: bool,
    /// A closed solid keeps the faces turned away from the viewer too, with their outward normals,
    /// drawn behind the others.
    pub(super) hidden: bool,
    pub(super) facets: Vec<Facet>,
    /// With `Some`, every face goes here as it is, none hidden or projected, for a mesh.
    pub(super) mesh: Option<Vec<Face3>>,
    /// The faces are projected into `facets`; without it only `mesh` is kept.
    pub(super) draw: bool,
    /// The faces kept for the mesh passed [`MAX_SOLID_FACES`] while the facets were built too: the
    /// mesh is too complex, and `mesh` keeps no more faces.
    pub(super) lost: bool,
    /// The cells the facets of the next band carry ([`Builder::curved_band`]).
    pub(super) cells: Option<BandCells>,
}

/// The cells of one band's facets: for each edge of the ring, its numbered surface and where the
/// edge starts and ends along it (`None` for an edge of no length), and where the band's two rings
/// lie across the surface.
pub(super) struct BandCells {
    edges: Vec<Option<(usize, f64, f64)>>,
    across: (f64, f64),
}

/// Where a face goes in the order the facets are drawn, back to front.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Order {
    /// The face is sorted by the depth of its center, except that a face seen almost edge-on goes
    /// first, under everything.
    Depth,
    /// The face is sorted by the depth of its center even when it is seen almost edge-on, so that
    /// a piece of a twisted wall at the solid's outline covers the walls behind it.
    Twisted,
    /// The face is drawn after every other face when it is turned toward the viewer. A flat end of
    /// the solid has the whole solid behind its plane, so it covers whatever it overlaps. Its depth
    /// is `f64::MAX`, so the faces drawn last by several solids sorted together keep the order they
    /// were added in.
    Last,
}

impl<'a> Builder<'a> {
    /// A builder for one solid seen by `camera`, keeping what `keep` asks for.
    pub(super) fn new(camera: &'a Camera, closed: bool, hidden: bool, keep: Keep) -> Self {
        Self {
            camera,
            closed,
            hidden,
            facets: vec![],
            mesh: (keep != Keep::Facets).then(Vec::new),
            draw: keep != Keep::Faces,
            lost: false,
            cells: None,
        }
    }

    /// Whether the builder keeps faces for a mesh.
    pub(super) fn keeps_mesh(&self) -> bool {
        self.mesh.is_some()
    }

    /// Add a face of view-space contours with outward normal `n` (view space), drawn in `order`.
    /// For a mesh it belongs to `part`, and `attrs` gives where its corners lie on the part's
    /// texture, from its contours.
    pub(super) fn face(
        &mut self,
        contours: Vec<Vec<V3>>,
        n: V3,
        order: Order,
        part: MeshPart,
        attrs: impl FnOnce(&[Vec<V3>]) -> Attrs,
    ) -> Result<(), String> {
        let count: usize = contours.iter().map(Vec::len).sum();
        if count < 3 {
            return Ok(());
        }
        if self.draw {
            self.facet(&contours, count, n, order)?;
        }
        let Some(mesh) = &mut self.mesh else { return Ok(()) };
        let attrs = attrs(&contours);
        mesh.push(Face3 { contours, normal: n.unit(), part, attrs });
        if mesh.len() > MAX_SOLID_FACES {
            if !self.draw {
                return Err(TOO_COMPLEX_SURFACE.into());
            }
            self.lose_mesh();
        }
        Ok(())
    }

    /// Keep no more faces for the mesh, which is too complex.
    pub(super) fn lose_mesh(&mut self) {
        self.mesh = None;
        self.lost = true;
    }

    /// How many faces the mesh holds, while the facets are built too and the mesh is kept.
    pub(super) fn faces_kept(&self) -> Option<usize> {
        self.mesh.as_ref().filter(|_| self.draw).map(Vec::len)
    }

    /// Project a face of `count` view-space corners with outward normal `n` into a facet, drawn in
    /// `order`, unless it is hidden or has no area.
    fn facet(&mut self, contours: &[Vec<V3>], count: usize, n: V3, order: Order) -> Result<(), String> {
        let centre = contours.iter().flatten().fold(V3::default(), |s, v| V3(s.0 + v.0, s.1 + v.1, s.2 + v.2));
        let centre = V3(centre.0 / count as f64, centre.1 / count as f64, centre.2 / count as f64);
        let n = n.unit();
        let facing = self.camera.facing(centre, n);
        // A closed solid hides the faces turned away. One seen almost exactly edge-on is kept and,
        // unless it is a piece of a twisted wall, drawn first, under everything: on a curved
        // surface rounding alone turns such faces either way, and dropping every other one would
        // leave gaps.
        let (n, behind) = if facing > 0.0 {
            (n, false)
        } else if self.closed && (facing > -0.02 || self.hidden) {
            (n, true)
        } else if self.closed {
            return Ok(());
        } else {
            (V3(-n.0, -n.1, -n.2), false)
        };
        // A face whose corners lie off one plane (a piece of a twisted wall bent into a saddle) has
        // no plane to measure its depth on. Nor has a face with a corner at or past the eye: the
        // projection holds that corner just in front of the eye, so the drawing no longer matches
        // the plane's projection there.
        let offset = n.dot(centre);
        let held = self.camera.eye.is_some_and(|e| contours.iter().flatten().any(|v| e - v.2 < e * 1e-3));
        let flat = !held && contours.iter().flatten().all(|v| (n.dot(*v) - offset).abs() <= 1e-3);
        let plane = flat.then_some(DepthPlane { normal: [n.0, n.1, n.2], offset, center: self.camera.origin, eye: self.camera.eye });
        #[cfg(test)]
        let view = contours.to_vec();
        let contours: Vec<Vec<Point>> = contours.iter().map(|c| c.iter().map(|v| self.camera.project(*v)).collect()).collect();
        if contours.iter().flatten().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err("The 3D projection is not finite".into());
        }
        if contours.iter().map(|c| signed_area(c).abs()).sum::<f64>() < 1e-10 {
            return Ok(());
        }
        let depth = if behind && order != Order::Twisted {
            centre.2 - self.camera.far
        } else if order == Order::Last && facing > 0.0 {
            f64::MAX
        } else {
            centre.2
        };
        self.facets.push(Facet {
            contours,
            depth,
            brightness: self.camera.brightness(n),
            illumination: crate::illumination(&self.camera.view, n, self.camera.light),
            normal: [n.0, n.1, n.2],
            rgb_gradient: None,
            outline: None,
            side: false,
            away: behind,
            end: None,
            surface: None,
            membrane: None,
            plane,
            cell: None,
            #[cfg(test)]
            view,
        });
        if self.facets.len() > MAX_SOLID_FACES {
            return Err(TOO_COMPLEX_SURFACE.into());
        }
        Ok(())
    }

    /// A planar surface triangle with shared, smooth outward normals. Culling and sorting
    /// still use its geometry, rather than the normals used only for lighting. For a mesh it
    /// belongs to `part`, with its corners at `uv` on the part's texture and their normals.
    pub(super) fn smooth_triangle(
        &mut self,
        vertices: [V3; 3],
        normals: [V3; 3],
        back: bool,
        part: MeshPart,
        uv: [[f64; 2]; 3],
    ) -> Result<(), String> {
        let contours = vec![vertices.to_vec()];
        let n = newell(&contours);
        let n = if back { V3(-n.0, -n.1, -n.2) } else { n };
        let centre = vertices.iter().fold(V3::default(), |s, v| V3(s.0 + v.0 / 3.0, s.1 + v.1 / 3.0, s.2 + v.2 / 3.0));
        let flip = !self.closed && self.camera.facing(centre, n.unit()) <= 0.0;
        let before = self.facets.len();
        self.face(contours, n, Order::Depth, part, |_| Attrs::Corners(vec![uv.iter().zip(normals).map(|(uv, n)| (*uv, Some(n))).collect()]))?;
        if self.facets.len() == before || !self.camera.view.shade {
            return Ok(());
        }
        let values: Vec<_> = normals
            .into_iter()
            .map(|n| crate::unclipped_illumination(&self.camera.view, if flip { V3(-n.0, -n.1, -n.2) } else { n }, self.camera.light))
            .collect();
        if let Some(face) = self.facets.last_mut() {
            let points = face.contours.first().map(Vec::as_slice).unwrap_or_default();
            face.rgb_gradient = crate::RgbGradient::triangle(points, &values);
        }
        Ok(())
    }

    /// The band between two partner rings (same length), as one quad per edge, or for a twisted
    /// wall with `cuts` (see [`cuts`]) as `cuts[i]` quads along edge `i`. `outward` is the
    /// direction (before rotation) the band faces away from the solid, for orienting normals. For a
    /// mesh the quads take their part, texture coordinates and corner normals from `mesh`. With
    /// `curves`, the pieces of curve the rings' edges follow and the outlines at `a` and `b`, each
    /// quad keeps its edges along the profile curved. An edge cut into pieces is cut at points of
    /// its curve, each piece taking an equal share of the edge's piece of curve. With `side`, each
    /// quad carries where its corners lie on the side unrolled flat ([`Facet::surface`]).
    pub(super) fn curved_band(
        &mut self,
        [a, b]: [&[V3]; 2],
        cuts: &[usize],
        outward: impl Fn(usize) -> V3,
        mut curves: Option<WallCurves>,
        side: Option<SideBand>,
        mesh: &BandMesh,
    ) -> Result<(), String> {
        let n = a.len().min(b.len());
        let order = if cuts.is_empty() { Order::Depth } else { Order::Twisted };
        for i in 0..n {
            let j = (i + 1) % n;
            let pieces = cuts.get(i).copied().unwrap_or(1).max(1);
            let span = curves.as_ref().and_then(|w| w.spans.get(i).copied().flatten());
            let places = curves.as_ref().map_or([None, None], |w| w.places);
            // Where the edge's curve is at share `s` of it.
            let t = |s: f64| span.map_or(0.0, |span| span.t0 + (span.t1 - span.t0) * s);
            let camera = self.camera;
            let corner = |p: V3, q: V3, s: f64, ring: usize| cut_corner(camera, span, places.get(ring).copied().flatten(), p, q, s);
            let ends = [[self.camera.project(a[i]), self.camera.project(a[j])], [self.camera.project(b[i]), self.camera.project(b[j])]];
            for k in 0..pieces {
                let (s0, s1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                let quad = vec![corner(a[i], a[j], s0, 0), corner(a[i], a[j], s1, 0), corner(b[i], b[j], s1, 1), corner(b[i], b[j], s0, 1)];
                // A twist bends the strip between two slices into a saddle: its normal is the whole
                // quad's average (Newell's), not one corner's triangle, which leans far to one side.
                let mut normal = newell(std::slice::from_ref(&quad));
                if normal.dot(normal) < 1e-18 {
                    continue;
                }
                if normal.dot(self.camera.rotate(outward(i))) < 0.0 {
                    normal = V3(-normal.0, -normal.1, -normal.2);
                }
                let before = self.facets.len();
                let attrs = |_: &[Vec<V3>]| {
                    let (u0, u1) = (mesh.u.get(i).copied().unwrap_or(0.5), mesh.u.get(i + 1).copied().unwrap_or(0.5));
                    let (ua, ub) = (u0 + (u1 - u0) * s0, u0 + (u1 - u0) * s1);
                    let [va, vb] = mesh.v;
                    // Each piece of a cut edge takes the normals smoothed for that piece.
                    let na = mesh.normals.and_then(|(na, _)| na.get(i)?.get(k).copied().flatten());
                    let nb = mesh.normals.and_then(|(_, nb)| nb.get(i)?.get(k).copied().flatten());
                    Attrs::Corners(vec![vec![([ua, va], na), ([ub, va], na), ([ub, vb], nb), ([ua, vb], nb)]])
                };
                self.face(vec![quad], normal, order, mesh.part, attrs)?;
                if self.facets.len() > before
                    && let Some(f) = self.facets.last_mut()
                {
                    f.side = true;
                    // An edge drawn in one piece carries its place on its numbered surface.
                    let cell = self.cells.as_ref().filter(|_| pieces == 1).and_then(|c| Some((c.edges.get(i).copied().flatten()?, c.across)));
                    if let (Some(((surface, u0, u1), (v0, v1))), Some(page)) = (cell, f.contours.first()) {
                        let uv = vec![Point::new(u0, v0), Point::new(u1, v0), Point::new(u1, v1), Point::new(u0, v1)];
                        f.cell = Some(Box::new(MapCell { surface, page: page.clone(), uv }));
                    }
                    if let Some(s) = side.as_ref()
                        && let (Some([x0, x1]), Some(page)) = (s.x.get(i).copied(), f.contours.first())
                    {
                        let (u0, u1) = (x0 + (x1 - x0) * s0, x0 + (x1 - x0) * s1);
                        let [y0, y1] = s.y;
                        // The face shows the side's outside when it was neither turned around to
                        // show its inside (on a hollow solid) nor kept turned away.
                        let n = normal.unit();
                        let outward = !f.away && f.normal[0] * n.0 + f.normal[1] * n.1 + f.normal[2] * n.2 > 0.0;
                        f.surface = Some(crate::SurfaceCell {
                            page: page.clone(),
                            uv: vec![Point::new(u0, y0), Point::new(u1, y0), Point::new(u1, y1), Point::new(u0, y1)],
                            outward,
                        });
                    }
                }
                if let Some(w) = curves.as_mut()
                    && let Some(span) = span
                {
                    // An edge cut on its chord, with no place on the curve for its cut corners,
                    // stays straight.
                    let near = w.near.filter(|_| pieces == 1 || w.places[0].is_some());
                    let far = w.far.filter(|_| pieces == 1 || w.places[1].is_some());
                    self.outline(before, |c| c.first().and_then(|quad| outline::wall(&span, (t(s0), t(s1)), ends, quad, near, far, w.budget)));
                }
            }
        }
        Ok(())
    }

    /// Give the facet that [`Builder::face`] added, when it added one after `before` facets, the
    /// outline `outline` makes from its projected contours.
    pub(super) fn outline(&mut self, before: usize, outline: impl FnOnce(&[Vec<Point>]) -> Option<PathData>) {
        if !self.draw || self.facets.len() <= before {
            return;
        }
        if let Some(f) = self.facets.last_mut() {
            f.outline = outline(&f.contours);
        }
    }

    /// Mark the facet that [`Builder::face`] added, when it added one after `before` facets, as the
    /// planar end `end` of the solid.
    pub(super) fn end(&mut self, before: usize, end: PlanarFace) {
        if !self.draw || self.facets.len() <= before {
            return;
        }
        if let Some(f) = self.facets.last_mut() {
            f.end = Some(end);
        }
    }

    /// How many faces the builder holds: facets, or without them the faces kept for a mesh.
    pub(super) fn len(&self) -> usize {
        match &self.mesh {
            Some(mesh) if !self.draw => mesh.len(),
            _ => self.facets.len(),
        }
    }

    /// The facets back to front, and the faces kept for a mesh (none without [`Builder::mesh`]).
    pub(super) fn finish(mut self) -> Built {
        self.facets.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        let faces = if self.lost { Err(TOO_COMPLEX_SURFACE.into()) } else { Ok(self.mesh.unwrap_or_default()) };
        (self.facets, faces, Some(self.camera.projection()))
    }
}

/// The curves a band's quads keep: the piece of curve each edge of its rings follows, and the
/// outlines at its two rings (`None` keeps that ring's edges straight).
pub(super) struct WallCurves<'a> {
    pub(super) spans: &'a [Option<Span>],
    pub(super) near: Option<&'a Along<'a>>,
    pub(super) far: Option<&'a Along<'a>>,
    /// Where a point of a span's curve lies in view space on the band's two rings, for the corners
    /// where a twisted wall's edge is cut ([`cuts`]).
    pub(super) places: [Option<&'a Place<'a>>; 2],
    pub(super) budget: &'a mut Budget,
}

/// Where the point at `t` on a span's curve lies in view space on one ring.
pub(super) type Place<'a> = dyn Fn(&Span, f64) -> V3 + 'a;

/// Where a band of the side lies on the side unrolled flat ([`super::side`]): the x of each end of
/// each edge of its rings, and the y of its two rings.
pub(super) struct SideBand<'a> {
    pub(super) x: &'a [[f64; 2]],
    pub(super) y: [f64; 2],
}

/// Where a point of the art goes in view space at one step: [`Camera::place`] after a [`Turn`],
/// at depth `z`, as one affine map worked out once for the step.
pub(super) struct Placing {
    centre: Point,
    at_centre: V3,
    x: V3,
    y: V3,
}

impl Placing {
    pub(super) fn new(camera: &Camera, centre: Point, turn: Turn, z: f64) -> Self {
        let at = |p: Point| camera.place(turn.apply(p), z);
        let at_centre = at(centre);
        let x = at(Point::new(centre.x + 1.0, centre.y)).sub(at_centre);
        let y = at(Point::new(centre.x, centre.y + 1.0)).sub(at_centre);
        Self { centre, at_centre, x, y }
    }
    pub(super) fn place(&self, p: Point) -> V3 {
        let (dx, dy) = (p.x - self.centre.x, p.y - self.centre.y);
        V3(
            self.at_centre.0 + self.x.0 * dx + self.y.0 * dy,
            self.at_centre.1 + self.x.1 * dx + self.y.1 * dy,
            self.at_centre.2 + self.x.2 * dx + self.y.2 * dy,
        )
    }
    /// The map as `(x, y, c)`: an art point `p` goes to `x · p.x + y · p.y + c` in view space.
    pub(super) fn frame(&self) -> (V3, V3, V3) {
        (self.x, self.y, self.place(Point::ZERO))
    }
}

/// The point `w` points to the left of `span`'s curve at `t`, as the ring runs along it (toward
/// the filled side): where [`inset`] moves the ring's points, without its miters.
fn offset(span: &Span, t: f64, w: f64) -> Point {
    let p = span.curve.eval(t);
    if w == 0.0 {
        return p;
    }
    let mut d = span.curve.deriv().eval(t).to_vec2();
    // A handle of zero length leaves no tangent at its end: the curve leaves toward its next point.
    if d.hypot() < 1e-12 {
        d = span.curve.p2 - span.curve.p0;
        if t > 0.5 {
            d = span.curve.p3 - span.curve.p1;
        }
    }
    let d = if span.t1 < span.t0 { -d } else { d };
    let l = d.hypot();
    if l < 1e-12 { p } else { Point::new(p.x - d.y / l * w, p.y + d.x / l * w) }
}

/// How many pieces each edge of a twisted wall is drawn in, from two neighboring slices of the
/// wall, `a` and `b` (partner rings): as many as make each piece about as long along the edge as
/// the band between the slices is wide there, 1 to [`MAX_PIECES`]. Twisted walls pass in front of
/// one another, and a strip the length of its edge has no one depth where another crosses it.
pub(super) fn cuts(a: &[V3], b: &[V3]) -> Vec<usize> {
    let n = a.len().min(b.len());
    let length = |p: V3, q: V3| q.sub(p).dot(q.sub(p)).sqrt();
    (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let (along, across) = (length(a[i], a[j]).max(length(b[i], b[j])), length(a[i], b[i]).max(length(a[j], b[j])));
            let count = (along / across.max(1e-9)).ceil();
            if count.is_finite() { (count as usize).clamp(1, MAX_PIECES) } else { MAX_PIECES }
        })
        .collect()
}

/// Lowers the [`cuts`] of every ring's edges (one list per ring) so that `bands` bands of each
/// make no more than `room` quads in all, or one per edge and band when that is more: the pieces
/// past each edge's first share the room in proportion.
pub(super) fn fit(cuts: &mut [Vec<usize>], bands: usize, room: usize) {
    let edges = cuts.iter().map(Vec::len).sum::<usize>().saturating_mul(bands);
    let total = cuts.iter().flatten().sum::<usize>().saturating_mul(bands);
    let room = room.max(edges);
    if total > room {
        let share = (room - edges) as f64 / (total - edges) as f64;
        for c in cuts.iter_mut().flatten() {
            *c = 1 + ((c.saturating_sub(1)) as f64 * share).floor() as usize;
        }
    }
}

/// The area-weighted normal of (possibly non-planar) contours, by Newell's method: for contours
/// winding with their filled side on the left as seen from in front, it points to the front.
pub(super) fn newell(contours: &[Vec<V3>]) -> V3 {
    let mut n = V3::default();
    for c in contours {
        for (a, b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
            let x = a.cross(*b);
            n = V3(n.0 + x.0, n.1 + x.1, n.2 + x.2);
        }
    }
    n
}

pub(super) fn check_bounds(b: Rect) -> Result<(), String> {
    if [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
        Ok(())
    } else {
        Err("3D effects require finite coordinates within 100 million points".into())
    }
}

/// The art's outward direction at edge `i` of `ring` (before rotation), tilted towards the face
/// it borders by `z` (+1 front, -1 back, 0 a side wall). An edge of no length (a corner cut
/// square, see [`square_corners`]) faces out between the edges on either side of it.
pub(super) fn outward(ring: &[Point], i: usize, z: f64) -> V3 {
    let n = ring.len();
    let (mut lx, mut ly) = left(ring[i], ring[(i + 1) % n]);
    if lx == 0.0 && ly == 0.0 && n > 2 {
        let (a, b) = (left(ring[(i + n - 1) % n], ring[i]), left(ring[(i + 1) % n], ring[(i + 2) % n]));
        (lx, ly) = (a.0 + b.0, a.1 + b.1);
    }
    V3(-lx, -ly, z)
}

/// The cross-section of a side: each ring of the shape at depth fraction `t` (0 front, 1 back),
/// turned by the twist and scaled by the taper about the centre of `bounds`.
pub(super) fn section(ring: &[Point], bounds: Rect, o: &Extrude, t: f64) -> Vec<Point> {
    let turn = Turn::new(bounds, o.twist, o.taper, t);
    ring.iter().map(|p| turn.apply(*p)).collect()
}

/// The twist and taper at depth fraction `t` (0 front, 1 back): a turn by `twist · t` and a
/// scale from 1 to `taper`, about the centre of the bounds.
#[derive(Clone, Copy)]
pub(super) struct Turn {
    centre: Point,
    pub(super) sin: f64,
    pub(super) cos: f64,
    pub(super) scale: f64,
}

impl Turn {
    pub(super) fn new(bounds: Rect, twist: f64, taper: f64, t: f64) -> Self {
        let (sin, cos) = (twist * t).to_radians().sin_cos();
        Self { centre: bounds.center(), sin, cos, scale: 1.0 + (taper - 1.0) * t }
    }
    pub(super) fn apply(&self, p: Point) -> Point {
        let (x, y) = ((p.x - self.centre.x) * self.scale, (p.y - self.centre.y) * self.scale);
        Point::new(self.centre.x + x * self.cos - y * self.sin, self.centre.y + x * self.sin + y * self.cos)
    }
}

/// One step of an extrusion's outline: how far it is inset (negative: outset) and its depth
/// fraction (0 front, 1 back), with the face it borders (+1 front bevel, -1 back bevel, 0 wall).
#[derive(Clone, Copy)]
struct Step {
    inset: f64,
    t: f64,
    face: f64,
}

/// How far the walls are inset (negative: outset). With the bevel inside, the cap is inset by its
/// width and the wall is the outline; outside, the cap is the outline and the wall is outset by
/// the width.
fn wall_inset(o: &Extrude) -> f64 {
    match o.bevel.filter(|b| b.width > 0.0 || b.height > 0.0) {
        Some(b) if !b.inside => -b.width,
        _ => 0.0,
    }
}

/// The outline's steps from the front cap's edge to the back cap's edge.
fn steps(o: &Extrude) -> Vec<Step> {
    let d = o.depth.max(1e-6);
    let mut out = vec![];
    let bevel = o.bevel.filter(|b| b.width > 0.0 || b.height > 0.0);
    // A bevel can't be deeper than the extrusion (half of it on each side with both sides).
    let sides = if bevel.is_some_and(|b| b.both_sides) { 2.0 } else { 1.0 };
    let height = bevel.map_or(0.0, |b| b.height.min(o.depth / sides));
    // The bevel's cross-section across the width: (share across, share down).
    let profile = |b: &Bevel| -> Vec<(f64, f64)> {
        let mut pts = vec![(0.0, 0.0)];
        let n = b.repeat.max(1) as f64;
        let run = (1.0 - b.space) / n;
        let gap = if b.repeat > 1 { b.space / (n - 1.0) } else { 0.0 };
        for k in 0..b.repeat.max(1) {
            let start = k as f64 * (run + gap);
            for s in b.shape.samples().into_iter().skip(1) {
                let (x, y) = b.shape.at(s);
                pts.push((start + x * run, (k as f64 + y) / n));
            }
            if b.repeat > 1 && k + 1 < b.repeat {
                pts.push((start + run + gap, (k as f64 + 1.0) / n));
            }
        }
        if let Some(last) = pts.last_mut() {
            last.0 = 1.0;
        }
        pts
    };
    let wall = wall_inset(o);
    // A twist turns a bevel's steps too: each step between two samples is cut into steps that
    // turn no further than 6°, as the walls' slices do at most.
    let refine = |pts: Vec<(f64, f64)>| -> Vec<(f64, f64)> {
        if o.twist == 0.0 {
            return pts;
        }
        let mut out = Vec::with_capacity(pts.len());
        for (k, p) in pts.iter().enumerate() {
            if let Some(q) = k.checked_sub(1).and_then(|k| pts.get(k)) {
                let turn = (p.1 - q.1).abs() * height / d * o.twist.abs();
                let n = ((turn / 6.0).ceil() as usize).clamp(1, 64);
                for m in 1..n {
                    let s = m as f64 / n as f64;
                    out.push((q.0 + (p.0 - q.0) * s, q.1 + (p.1 - q.1) * s));
                }
            }
            out.push(*p);
        }
        out
    };
    if let Some(b) = bevel {
        for (x, y) in refine(profile(&b)) {
            let inset = if b.inside { b.width * (1.0 - x) } else { -b.width * x };
            out.push(Step { inset, t: y * height / d, face: 1.0 });
        }
    } else {
        out.push(Step { inset: 0.0, t: 0.0, face: 1.0 });
    }
    // A twist bends the walls: cut them into slices. A taper alone keeps each wall flat (its edge
    // only scales toward the center as it goes back), so it needs none.
    let slices = if o.twist != 0.0 { ((o.twist.abs() / 6.0).ceil() as usize).clamp(8, 64) } else { 1 };
    let back_height = if bevel.is_some_and(|b| b.both_sides) { height } else { 0.0 };
    let (t0, t1) = (height / d, 1.0 - back_height / d);
    for k in 0..=slices {
        let t = t0 + (t1 - t0) * k as f64 / slices as f64;
        out.push(Step { inset: wall, t, face: 0.0 });
    }
    if let Some(b) = bevel.filter(|b| b.both_sides) {
        for (x, y) in refine(profile(&b)).into_iter().rev() {
            let inset = if b.inside { b.width * (1.0 - x) } else { -b.width * x };
            out.push(Step { inset, t: 1.0 - y * height / d, face: -1.0 });
        }
    }
    out.dedup_by(|a, b| (a.inset - b.inset).abs() < 1e-9 && (a.t - b.t).abs() < 1e-12);
    out
}

/// The steps and their shared outline for drawing, surface measurements and mesh export.
fn extrude_profile(path: &PathData, rule: FillRule, size: f64, o: &Extrude) -> Result<(Vec<Step>, Vec<Ring>), String> {
    let mut steps = steps(o);
    let bands = steps.len().saturating_sub(1);
    let caps = if o.cap { 2 } else { 0 };
    let most = curve_pieces(path, size, bands, caps)?;
    let mut rings = solid_rings_traced(path, rule, size, MAX_SOLID_POINTS, most)?;
    if let (Some(bevel), [ring]) = (o.bevel, rings.as_slice())
        && bevel.inside
        && let Some(width) = collapse_width(ring)
        && bevel.width > width
    {
        // Use one width for the cap and every end of every band. The profile still runs
        // through the requested depth and meets the wall at the original outline.
        let scale = width / bevel.width;
        for step in &mut steps {
            step.inset *= scale;
        }
        steps.dedup_by(|a, b| (a.inset - b.inset).abs() < 1e-9 && (a.t - b.t).abs() < 1e-12);
    }
    // The corners the bevel opens are cut square past the miter limit.
    if let Some(side) = steps.iter().map(|s| s.inset).find(|w| *w != 0.0) {
        for ring in &mut rings {
            *ring = square_corners(ring, side);
        }
    }
    Ok((steps, rings))
}

/// The face a band between steps `s0` and `s1` borders: +1 between two front bevel steps, -1 when
/// it reaches a back bevel step, and 0 for a wall.
fn band_face(s0: &Step, s1: &Step) -> f64 {
    if s1.face < 0.0 {
        -1.0
    } else if s0.face > 0.0 && s1.face > 0.0 {
        1.0
    } else {
        0.0
    }
}

/// The part of the mesh a band bordering `face` ([`band_face`]) belongs to.
fn band_part(face: f64) -> MeshPart {
    if face > 0.0 {
        MeshPart::FrontBevel
    } else if face < 0.0 {
        MeshPart::BackBevel
    } else {
        MeshPart::Side
    }
}

/// The texture coordinate across the side at each step, front to back: the distance from the
/// front cap's edge through the bevels and walls (inset and depth together) over the whole
/// distance to the back cap's edge. With no distance every step takes 0.5.
fn step_coordinates(steps: &[Step], depth: f64) -> Vec<f64> {
    let mut at = Vec::with_capacity(steps.len());
    let mut sum = 0.0;
    for (k, s) in steps.iter().enumerate() {
        if let Some(p) = k.checked_sub(1).and_then(|k| steps.get(k)) {
            sum += (s.inset - p.inset).hypot((s.t - p.t) * depth);
        }
        at.push(sum);
    }
    at.iter().map(|d| if sum > 1e-12 { (d / sum).clamp(0.0, 1.0) } else { 0.5 }).collect()
}

/// The largest angle, in degrees, between two neighboring bands of one bevel whose shared corners
/// are smoothed for a mesh. A sharper turn (a Step profile's) stays a hard edge.
pub(crate) const BEVEL_CREASE: f64 = 60.0;

/// For a mesh, the normal of each corner of each piece of each edge of `ring` at each step (one
/// list per step, one list per edge, one entry per piece: `cuts[i]` pieces along edge `i`, as
/// [`Builder::curved_band`] cuts it, or one) where the bands on either side are smoothed: two
/// bands of the same bevel, front or back, that turn by less than [`BEVEL_CREASE`], neither of
/// them lying flat toward the front or back (as a cap does). Such a corner takes the mean of the
/// two bands' unit normals at that piece; every other corner is `None` and takes its face's
/// normal. `placed` holds the ring at each step in view space, `spans` the piece of curve each edge
/// of the ring follows, and `places` where a point of a span's curve lies on the ring at each step:
/// the pieces' cut corners lie where [`Builder::curved_band`] puts them ([`cut_corner`]).
fn smoothed_rows(
    camera: &Camera,
    ring: &[Point],
    steps: &[Step],
    placed: &[Vec<V3>],
    cuts: &[usize],
    spans: &[Option<Span>],
    places: &[&Place],
) -> Vec<Vec<Vec<Option<V3>>>> {
    let n = ring.len();
    let pieces = |i: usize| cuts.get(i).copied().unwrap_or(1).max(1);
    let front = camera.rotate(V3(0.0, 0.0, 1.0));
    // Each band's unit outward normal at each piece of each edge, as [`Builder::curved_band`]
    // cuts and orients it.
    let bands: Vec<(MeshPart, Vec<Vec<V3>>)> = placed
        .windows(2)
        .zip(steps.windows(2))
        .enumerate()
        .map(|(b, (pair, s))| {
            let face = band_face(&s[0], &s[1]);
            let (near, far) = (places.get(b).copied(), places.get(b + 1).copied());
            let normals = (0..n)
                .map(|i| {
                    let j = (i + 1) % n;
                    let pieces = pieces(i);
                    let span = spans.get(i).copied().flatten();
                    (0..pieces)
                        .map(|k| {
                            let (Some(a), Some(c)) = (pair.first(), pair.get(1)) else { return V3::default() };
                            let (Some((&ai, &aj)), Some((&ci, &cj))) = (a.get(i).zip(a.get(j)), c.get(i).zip(c.get(j))) else { return V3::default() };
                            let (s0, s1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                            let (corner_a, corner_c) = (|s| cut_corner(camera, span, near, ai, aj, s), |s| cut_corner(camera, span, far, ci, cj, s));
                            let quad = vec![corner_a(s0), corner_a(s1), corner_c(s1), corner_c(s0)];
                            let normal = newell(std::slice::from_ref(&quad)).unit();
                            if normal.dot(camera.rotate(outward(ring, i, face))) < 0.0 { V3(-normal.0, -normal.1, -normal.2) } else { normal }
                        })
                        .collect()
                })
                .collect();
            (band_part(face), normals)
        })
        .collect();
    let crease = BEVEL_CREASE.to_radians().cos();
    let flat = |v: V3| v.dot(front).abs() > 1.0 - 1e-6;
    (0..steps.len())
        .map(|k| {
            let pair = k.checked_sub(1).and_then(|p| bands.get(p)).zip(bands.get(k));
            let Some(((_, n0), (_, n1))) = pair.filter(|((p0, _), (p1, _))| p0 == p1 && *p0 != MeshPart::Side) else {
                return (0..n).map(|i| vec![None; pieces(i)]).collect();
            };
            n0.iter()
                .zip(n1)
                .map(|(e0, e1)| {
                    e0.iter()
                        .zip(e1)
                        .map(|(a, b)| {
                            let whole = a.dot(*a) > 0.5 && b.dot(*b) > 0.5;
                            (whole && !flat(*a) && !flat(*b) && a.dot(*b) > crease).then(|| V3(a.0 + b.0, a.1 + b.1, a.2 + b.2).unit())
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// The corner a share `s` of the way along the edge from `p` to `q` where [`Builder::curved_band`]
/// cuts it. On a ring that follows a curve (`span`, placed on the ring by `place`) the corner lies on
/// the curve, moved by the offsets of the edge's corners from it (a bevel's miters), eased across
/// the edge as the cap's edge eases them, so the pieces' edges follow the same curve as the cap's
/// edge they meet. The offsets are eased as drawn, and the corner keeps the depth they give it in
/// view space. Otherwise, and at 0 and 1, the corner lies on the chord ([`along_edge`]).
fn cut_corner(camera: &Camera, span: Option<Span>, place: Option<&Place>, p: V3, q: V3, s: f64) -> V3 {
    match (span, place) {
        (Some(span), Some(place)) if s > 0.0 && s < 1.0 => {
            let t = span.t0 + (span.t1 - span.t0) * s;
            let (c, c0, c1) = (place(&span, t), place(&span, span.t0), place(&span, span.t1));
            let (ep, eq) = (outline::fade(s), outline::fade(1.0 - s));
            let (dp, dq) = (p.sub(c0), q.sub(c1));
            let z = c.2 + dp.2 * ep + dq.2 * eq;
            let (dp, dq) = (camera.project(p) - camera.project(c0), camera.project(q) - camera.project(c1));
            let v = camera.unproject(camera.project(c) + dp * ep + dq * eq, z);
            if v.0.is_finite() && v.1.is_finite() && v.2.is_finite() { v } else { along_edge(p, q, s) }
        }
        _ => along_edge(p, q, s),
    }
}

/// The point a share `s` of the way from `p` to `q`: at 0 and 1, the corners themselves.
fn along_edge(p: V3, q: V3, s: f64) -> V3 {
    if s <= 0.0 {
        p
    } else if s >= 1.0 {
        q
    } else {
        V3(p.0 + (q.0 - p.0) * s, p.1 + (q.1 - p.1) * s, p.2 + (q.2 - p.2) * s)
    }
}

/// Extrude & Bevel: the shape's outline extruded backwards, with optional twist, taper, bevels
/// and caps.
pub fn extrude(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude, view: View) -> Result<Vec<Facet>, String> {
    extrude_faces(path, rule, bounds, options, view, Keep::Facets).map(|(facets, ..)| facets)
}

/// [`extrude`], keeping the solid's facets, its faces before projection, or both, as `keep` asks.
pub(crate) fn extrude_faces(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude, view: View, keep: Keep) -> Result<Built, String> {
    #[cfg(test)]
    EXTRUDED.with(|n| n.set(n.get() + 1));
    check_bounds(bounds)?;
    let o = options.sanitized();
    // No depth: the flat shape, seen from either side.
    if o.depth <= 0.0 {
        return plane_faces(path, rule, bounds, view, keep);
    }
    let size = bounds.width().max(bounds.height());
    let (steps, traced) = extrude_profile(path, rule, size, &o)?;
    let (bands, caps) = (steps.len().saturating_sub(1), if o.cap { 2 } else { 0 });
    let rings: Vec<&[Point]> = traced.iter().map(|r| r.points.as_slice()).collect();
    let edges = rings.iter().map(|r| r.len()).fold(0usize, usize::saturating_add);
    if edges.saturating_mul(bands).saturating_add(caps) > MAX_SOLID_FACES {
        return Err(TOO_COMPLEX_SURFACE.into());
    }
    let camera = Camera::new(view, bounds, o.depth);
    let mut b = Builder::new(&camera, o.cap, o.hidden, keep);
    let z = |t: f64| o.depth / 2.0 - o.depth * t;
    let ring_at =
        |ring: &[Point], s: &Step| -> Vec<V3> { section(&inset(ring, s.inset), bounds, &o, s.t).iter().map(|p| camera.place(*p, z(s.t))).collect() };
    // Every band of a twisted outline, bevels included, is cut as its walls' first slice is (with
    // bevels and no walls, as its shallowest band is), within [`MAX_FACES`] facets.
    let depth_of = |s: &[Step]| s.get(1).zip(s.first()).map_or(0.0, |(b, a)| (b.t - a.t).abs());
    let wall = steps
        .windows(2)
        .find(|s| s[0].face == 0.0 && s[1].face == 0.0)
        .or_else(|| steps.windows(2).filter(|s| depth_of(s) > 1e-12).min_by(|a, c| depth_of(a).total_cmp(&depth_of(c))));
    let mut pieces: Vec<Vec<usize>> = rings
        .iter()
        .map(|ring| match wall {
            Some([s0, s1]) if o.twist != 0.0 => cuts(&ring_at(ring, s0), &ring_at(ring, s1)),
            _ => vec![],
        })
        .collect();
    fit(&mut pieces, steps.len().saturating_sub(1), MAX_FACES);
    // The outline at each step, through the profile's curves: exact where the step is not inset,
    // and within the sampling tolerance (which the inset rings themselves keep to) where it is.
    let sampled = (size / 2000.0).max(0.05);
    let maps: Vec<Placing> = steps.iter().map(|s| Placing::new(&camera, bounds.center(), Turn::new(bounds, o.twist, o.taper, s.t), z(s.t))).collect();
    let places: Vec<_> = steps
        .iter()
        .zip(&maps)
        .map(|(s, m)| {
            let w = s.inset;
            move |span: &Span, t: f64| m.place(offset(span, t, w))
        })
        .collect();
    let at: Vec<_> = places
        .iter()
        .map(|place| {
            let camera = &camera;
            move |span: &Span, t: f64| camera.project(place(span, t))
        })
        .collect();
    // A step that is not inset moves the art point by point, affinely without perspective.
    let point_maps: Vec<_> = maps.iter().map(|m| |p: Point| camera.project(m.place(p))).collect();
    let along: Vec<Along> = steps
        .iter()
        .zip(&at)
        .zip(&point_maps)
        .map(|((s, at), map)| Along {
            at,
            map: (s.inset == 0.0).then_some(map as &dyn Fn(Point) -> Point),
            affine: camera.eye.is_none(),
            tolerance: if s.inset == 0.0 { CURVE_TOLERANCE } else { sampled },
        })
        .collect();
    let mut budget = Budget::default();
    // The numbered surfaces of the sides, for a solid whose walls stay flat.
    let layout = (o.map_surfaces && o.twist == 0.0 && o.taper == 1.0).then(|| (sides(&traced, wall_inset(&o)), band_spans(&steps, o.depth)));
    // With graphics on the side, the longest contour's pieces carry where they lie on it unrolled.
    let unrolled = o
        .map_surface
        .then(|| {
            let k = side::longest(&rings)?;
            let y = side::across_steps(&steps.iter().map(|s| (s.inset, s.t)).collect::<Vec<_>>(), o.depth);
            Some((k, side::along_ring(rings.get(k)?)?, y))
        })
        .flatten();
    // For a mesh: the texture coordinate across the side at each step, and the parts of the bands.
    let across = step_coordinates(&steps, o.depth);
    let part_of = |s0: &Step, s1: &Step| band_part(band_face(s0, s1));
    for (r, ((ring, traced), pieces)) in rings.iter().zip(&traced).zip(&pieces).enumerate() {
        let unrolled = unrolled.as_ref().filter(|(k, ..)| *k == r);
        let placed: Vec<Vec<V3>> = steps.iter().map(|s| ring_at(ring, s)).collect();
        let u = along_ring(ring);
        let rows = if b.keeps_mesh() {
            let at: Vec<&Place> = places.iter().map(|p| p as &Place).collect();
            smoothed_rows(&camera, ring, &steps, &placed, pieces, &traced.spans, &at)
        } else {
            vec![]
        };
        for (k, pair) in placed.windows(2).enumerate() {
            let ([a, c], Some(s0), Some(s1)) = (pair, steps.get(k), steps.get(k + 1)) else { continue };
            b.cells = layout.as_ref().and_then(|(sides, spans)| {
                let (bevel, v0, v1) = spans.get(k).copied().flatten()?;
                // Without a wall span the walls are not numbered, and the bevels follow the caps.
                let walls = has_walls(spans);
                if !bevel && !walls {
                    return None;
                }
                let base = caps + if bevel && walls { sides.lengths.len() } else { 0 };
                let edges = sides.edges.get(r)?.iter().map(|e| e.map(|(section, u0, u1)| (base + section, u0, u1))).collect();
                Some(BandCells { edges, across: (v0, v1) })
            });
            let face = band_face(s0, s1);
            // Each ring of a band follows the outline at its step: the edges a band shares with a cap
            // follow the cap's curves, and an edge two bands share follows the same curves in both.
            let place = |k: usize| places.get(k).map(|p| p as &Place);
            let curves = Some(WallCurves {
                spans: &traced.spans,
                near: along.get(k),
                far: along.get(k + 1),
                places: [place(k), place(k + 1)],
                budget: &mut budget,
            });
            let side = unrolled.and_then(|(_, x, y)| Some(SideBand { x, y: [*y.get(k)?, *y.get(k + 1)?] }));
            let normals = rows.get(k).zip(rows.get(k + 1)).map(|(na, nb)| (na.as_slice(), nb.as_slice()));
            let v = [across.get(k).copied().unwrap_or(0.5), across.get(k + 1).copied().unwrap_or(0.5)];
            let mesh = BandMesh { part: part_of(s0, s1), u: &u, v, normals };
            b.curved_band([a, c], pieces, |i| outward(ring, i, face), curves, side, &mesh)?;
        }
    }
    b.cells = None;
    if o.cap {
        // A cap turned toward the viewer covers the walls it overlaps, even those nearer than its
        // center: twisted walls, and the walls of a fill inside the solid of its stroke.
        let caps = [
            (steps.first(), maps.first(), along.first(), 1.0, PlanarFace::Front, MeshPart::Front),
            (steps.last(), maps.last(), along.last(), -1.0, PlanarFace::Back, MeshPart::Back),
        ];
        // Each cap's corners take their art points, before any twist or taper, in `bounds`, which
        // every part of one object shares.
        let area = bounds;
        for (s, map, along, n, end, part) in caps {
            let (Some(s), Some(map)) = (s, map) else { continue };
            let before = b.len();
            let (x, y, origin) = map.frame();
            let plane = Attrs::Planar(PlaneUv { origin, x, y, area });
            b.face(rings.iter().map(|r| ring_at(r, s)).collect(), camera.rotate(V3(0.0, 0.0, n)), Order::Last, part, |_| plane)?;
            b.end(before, end);
            if let Some(along) = along {
                let traced: Vec<&Ring> = traced.iter().collect();
                b.outline(before, |c| outline::face(&traced, c, along, &mut budget));
            }
        }
    }
    Ok(b.finish())
}

/// The turn of the outline, in degrees, past which the side wall on one side of a point is another
/// numbered surface than the wall on its other side.
const CORNER: f64 = 20.0;

/// How the sides of a solid's rings divide into numbered surfaces: for each ring, each edge's
/// surface (counted through the rings in order) and where the edge starts and ends along it, and
/// each surface's length. A surface runs from one corner of its ring to the next, and a ring with
/// no corner is one surface from its first point. The edges of one curve of the outline are never
/// parted. An edge of no length belongs to no surface. An edge starts farther along its surface
/// than it ends: the surface is measured from its left edge as it is seen from outside the solid.
/// Lengths are measured on the ring moved to the walls' inset `wall`, so a wall set out by a bevel
/// is laid out at its own length, and a bevel band at the length of the wall below it.
struct Sides {
    edges: Vec<Vec<Option<(usize, f64, f64)>>>,
    lengths: Vec<f64>,
}

fn sides(rings: &[Ring], wall: f64) -> Sides {
    let mut out = Sides { edges: vec![], lengths: vec![] };
    for ring in rings {
        let n = ring.points.len();
        let point = |i: usize| ring.points.get(i % n.max(1)).copied().unwrap_or_default();
        let at_wall = inset(&ring.points, wall);
        let wall_point = |i: usize| at_wall.get(i % n.max(1)).copied().unwrap_or_default();
        // An edge of the outline with no length (a corner cut square) stays off every surface.
        let length = |i: usize| if point(i).distance(point(i + 1)) > 1e-9 { wall_point(i).distance(wall_point(i + 1)) } else { 0.0 };
        let direction = |i: usize| {
            let d = point(i + 1) - point(i);
            let l = d.hypot();
            (l > 1e-9).then(|| d / l)
        };
        let curve = |i: usize| ring.spans.get(i % n.max(1)).copied().flatten().map(|s| s.curve);
        // Point `i` is a corner when the edge that leaves it turns from the last edge with a
        // length before it, and the two do not follow one curve.
        let corner = |i: usize| {
            let Some(leaving) = direction(i) else { return false };
            let Some(before) = (1..n).map(|k| i + n - k).find(|e| direction(*e).is_some()) else { return false };
            let one_curve = matches!((curve(before), curve(i)), (Some(a), Some(b)) if a == b);
            !one_curve && direction(before).is_some_and(|d| d.dot(leaving) < CORNER.to_radians().cos())
        };
        let start = (0..n).find(|i| corner(*i)).unwrap_or(0);
        let mut edges = vec![None; n];
        let mut along = 0.0;
        out.lengths.push(0.0);
        for k in 0..n {
            let i = (start + k) % n;
            if k > 0 && corner(i) {
                out.lengths.push(0.0);
                along = 0.0;
            }
            let l = length(i);
            if let Some(slot) = edges.get_mut(i).filter(|_| l > 1e-9) {
                *slot = Some((out.lengths.len() - 1, along, along + l));
            }
            along += l;
            if let Some(total) = out.lengths.last_mut() {
                *total = along;
            }
        }
        // A ring runs from right to left as its side is seen from outside the solid, so each edge
        // is placed from its surface's far end.
        for (surface, from, to) in edges.iter_mut().flatten() {
            let length = out.lengths.get(*surface).copied().unwrap_or(0.0);
            (*from, *to) = (length - *from, length - *to);
        }
        out.edges.push(edges);
    }
    out
}

/// Where the band between each two neighboring steps lies across its numbered surface: whether it
/// is part of the front bevel (else of the wall), and the distances of its two rings from the
/// surface's edge nearer the front. `None` for a band of a back bevel.
fn band_spans(steps: &[Step], depth: f64) -> Vec<Option<(bool, f64, f64)>> {
    let (mut bevel_at, mut wall_at) = (0.0, 0.0);
    steps
        .windows(2)
        .map(|pair| {
            let [s0, s1] = pair else { return None };
            if s1.face < 0.0 {
                return None;
            }
            let across = (s1.inset - s0.inset).hypot((s1.t - s0.t) * depth);
            let bevel = s0.face > 0.0 && s1.face > 0.0;
            let at = if bevel { &mut bevel_at } else { &mut wall_at };
            let from = *at;
            *at += across;
            Some((bevel, from, *at))
        })
        .collect()
}

/// Whether the bands of `spans` ([`band_spans`]) hold a wall with some width across it: a bevel as
/// deep as the solid leaves none.
fn has_walls(spans: &[Option<(bool, f64, f64)>]) -> bool {
    spans.iter().flatten().any(|(bevel, from, to)| !bevel && to > from)
}

/// The numbered surfaces of the solid [`extrude`] builds from `path`, in order: the front and back
/// caps of a solid with caps, then each side wall, ring by ring from each ring's first corner, then
/// the bevel above each wall in the same order. A plane (no depth) has its front only, and a solid
/// with a twist or a taper has only its caps. A solid whose bevel is as deep as the solid has no
/// walls. The facets of a solid built with
/// [`Extrude::map_surfaces`] carry these indexes ([`MapCell`]).
pub fn extrude_surfaces(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude) -> Result<Vec<SolidSurface>, String> {
    check_bounds(bounds)?;
    let o = options.sanitized();
    let end =
        |face: PlanarFace, b: Rect| SolidSurface { kind: SurfaceKind::End(face), width: b.width(), height: b.height(), center: Some(b.center()) };
    if o.depth <= 0.0 {
        return Ok(path.bounds().map(|b| end(PlanarFace::Front, b)).into_iter().collect());
    }
    let size = bounds.width().max(bounds.height());
    let (steps, traced) = extrude_profile(path, rule, size, &o)?;
    let (bands, caps) = (steps.len().saturating_sub(1), if o.cap { 2 } else { 0 });
    // A solid past the face limit is refused, as [`extrude_faces`] refuses it.
    let edges = traced.iter().map(|r| r.points.len()).fold(0usize, usize::saturating_add);
    if edges.saturating_mul(bands).saturating_add(caps) > MAX_SOLID_FACES {
        return Err(TOO_COMPLEX_SURFACE.into());
    }
    let mut out = vec![];
    if o.cap {
        for (step, face) in [(steps.first(), PlanarFace::Front), (steps.last(), PlanarFace::Back)] {
            let Some(step) = step else { continue };
            let mut points = traced.iter().flat_map(|r| inset(&r.points, step.inset));
            let Some(first) = points.next() else { continue };
            out.push(end(face, points.fold(Rect::from_points(first, first), |b, p| b.union_pt(p))));
        }
    }
    if o.twist != 0.0 || o.taper != 1.0 {
        return Ok(out);
    }
    let sides = sides(&traced, wall_inset(&o));
    let spans = band_spans(&steps, o.depth);
    let across = |bevel: bool| spans.iter().flatten().filter(|s| s.0 == bevel).map(|s| s.2).fold(0.0, f64::max);
    let walls = has_walls(&spans).then_some((SurfaceKind::Wall, false));
    let kinds = walls.into_iter().chain(spans.iter().flatten().any(|s| s.0).then_some((SurfaceKind::Bevel, true)));
    for (kind, bevel) in kinds {
        out.extend(sides.lengths.iter().map(|l| SolidSurface { kind, width: *l, height: across(bevel), center: None }));
    }
    Ok(out)
}

/// The size of Extrude & Bevel's side unrolled flat ([`super::side`]), for a solid of `path` built
/// as [`extrude`] builds it: the length of its longest contour's outline, the length across its
/// bevels and walls, front to back, and the width about x = 0 that does not reach the side's cut.
/// `None` with a depth of 0, for invalid bounds, and for a shape [`extrude`] refuses.
pub fn extrude_side_size(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude) -> Option<(f64, f64, f64)> {
    check_bounds(bounds).ok()?;
    let o = options.sanitized();
    if o.depth <= 0.0 {
        return None;
    }
    let size = bounds.width().max(bounds.height());
    let (steps, traced) = extrude_profile(path, rule, size, &o).ok()?;
    let (bands, caps) = (steps.len().saturating_sub(1), if o.cap { 2 } else { 0 });
    let rings: Vec<&[Point]> = traced.iter().map(|r| r.points.as_slice()).collect();
    // A solid past the face limit is refused, as [`extrude_faces`] refuses it.
    let edges = rings.iter().map(|r| r.len()).fold(0usize, usize::saturating_add);
    if edges.saturating_mul(bands).saturating_add(caps) > MAX_SOLID_FACES {
        return None;
    }
    side::size(&rings, &steps.iter().map(|s| (s.inset, s.t)).collect::<Vec<_>>(), o.depth)
}

#[cfg(test)]
thread_local! {
    /// How many Rotate planes (and Extrude & Bevel solids with no depth) this thread has built, for
    /// the tests.
    pub(crate) static PLANES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Rotate: the flat art turned in 3D about the centre of `bounds` (which may be larger than the
/// shape's own, so that every part of one object turns about the same point).
pub fn plane(path: &PathData, rule: FillRule, bounds: Rect, view: View) -> Result<Vec<Facet>, String> {
    plane_faces(path, rule, bounds, view, Keep::Facets).map(|(facets, ..)| facets)
}

/// [`plane`], keeping its facet, its face before projection, or both, as `keep` asks.
pub(crate) fn plane_faces(path: &PathData, rule: FillRule, bounds: Rect, view: View, keep: Keep) -> Result<Built, String> {
    #[cfg(test)]
    PLANES.with(|n| n.set(n.get() + 1));
    check_bounds(bounds)?;
    let rings = solid_rings_traced(path, rule, bounds.width().max(bounds.height()), MAX_PLANE_POINTS, CURVE_PIECES)?;
    let camera = Camera::new(view, bounds, 0.0);
    let mut b = Builder::new(&camera, false, false, keep);
    let contours: Vec<Vec<V3>> = rings.iter().map(|r| r.points.iter().map(|p| camera.place(*p, 0.0)).collect()).collect();
    // A plane is one flat face, seen from either side. Its normal is turned toward the viewer, so
    // it is drawn last, as a cap is, and planes drawn in one order keep the order they are given in.
    let up = camera.rotate(V3(0.0, 0.0, 1.0));
    let n = if camera.facing(V3::default(), up) < 0.0 { V3(-up.0, -up.1, -up.2) } else { up };
    let m = Placing::new(&camera, bounds.center(), Turn::new(bounds, 0.0, 1.0, 0.0), 0.0);
    // The plane's corners take their art points in `bounds`, which every part of one object shares.
    let (x, y, origin) = m.frame();
    let plane = Attrs::Planar(PlaneUv { origin, x, y, area: bounds });
    b.face(contours, n, Order::Last, MeshPart::Front, |_| plane)?;
    b.end(0, PlanarFace::Front);
    let at = |span: &Span, t: f64| camera.project(m.place(span.curve.eval(t)));
    let map = |p: Point| camera.project(m.place(p));
    let along = Along { at: &at, map: Some(&map), affine: camera.eye.is_none(), tolerance: CURVE_TOLERANCE };
    let rings: Vec<&Ring> = rings.iter().collect();
    b.outline(0, |c| outline::face(&rings, c, &along, &mut Budget::default()));
    Ok(b.finish())
}

/// The distance from `p` to the segment `a`–`b`.
pub(super) fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-18 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0) };
    p.distance(Point::new(a.x + dx * t, a.y + dy * t))
}

/// The distance from `p` to the nearest edge of `rings`, edge by edge.
#[cfg(test)]
pub(super) fn edge_distance(rings: &[Vec<Point>], p: Point) -> f64 {
    rings
        .iter()
        .flat_map(|r| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()))
        .map(|(a, b)| segment_distance(p, *a, *b))
        .fold(f64::INFINITY, f64::min)
}
