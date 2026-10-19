//! Extrude & Bevel, Inflate and Rotate (a flat plane) as projected, shaded vector facets.
//!
//! The 2D art is the front of the solid: its outline is extruded backwards along the view's
//! z-axis, about the centre of its bounds, at the art's own position and size. Planar faces (caps
//! and the rotated plane) are drawn whole, holes included, so no triangulation is needed; curved
//! faces are cut into small facets. Colored Inflate interpolates shared membrane normals;
//! bevels and side walls retain their planar lighting.
//!
//! Visibility is a painter's algorithm, as for Revolve: faces of a closed solid that face away
//! from the viewer are left out, the rest are drawn back to front.

use vectorcraft_geom::{BezPath, FillRule, ParamCurve, PathData, Point, Rect, Vec2};

use crate::{MAX_FACES, Revolve, V3, eye_distance, perspective_scale};

/// The view shared by every 3D effect: rotation, perspective and directional lighting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    /// Perspective, 0° (none) to 160°, as Revolve's.
    pub perspective: f64,
    pub light_rotation: f64,
    pub light_height: f64,
    pub light_behind: bool,
    pub light_color: [f64; 3],
    pub light_intensity: f64,
    pub light_softness: f64,
    pub lights: Option<crate::Lights>,
    pub ambient: f64,
    pub shade: bool,
}

impl Default for View {
    fn default() -> Self {
        let r = Revolve::default();
        Self {
            rotation_x: r.rotation_x,
            rotation_y: r.rotation_y,
            rotation_z: r.rotation_z,
            perspective: r.perspective,
            light_rotation: r.light_rotation,
            light_height: r.light_height,
            light_behind: r.light_behind,
            light_color: r.light_color,
            light_intensity: r.light_intensity,
            light_softness: r.light_softness,
            lights: r.lights,
            ambient: r.ambient,
            shade: r.shade,
        }
    }
}

impl View {
    /// Whether the active light setup needs RGB shading rather than neutral brightness.
    pub fn has_colored_lights(self) -> bool {
        let view = self.revolve();
        view.lights.map_or(view.light_color != [1.0; 3], |lights| lights.iter().any(|light| light.color != [1.0; 3]))
    }

    /// The same rotation, projection and lighting maths as Revolve's.
    fn revolve(self) -> Revolve {
        Revolve {
            rotation_x: self.rotation_x,
            rotation_y: self.rotation_y,
            rotation_z: self.rotation_z,
            perspective: self.perspective,
            light_rotation: self.light_rotation,
            light_height: self.light_height,
            light_behind: self.light_behind,
            light_color: self.light_color,
            light_intensity: self.light_intensity,
            light_softness: self.light_softness,
            lights: self.lights,
            ambient: self.ambient,
            shade: self.shade,
            ..Revolve::default()
        }
        .sanitized()
    }
}

/// The shape of a bevel's cross-section, from the cap's edge (0) to the side wall (1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BevelShape {
    /// A straight 45° chamfer when width and height are equal.
    #[default]
    Classic,
    /// A convex quarter round.
    Rounded,
    /// A concave quarter round.
    Cove,
    /// Two flat steps.
    Step,
    /// A convex then concave S curve.
    Ogee,
}

impl BevelShape {
    pub const ALL: [BevelShape; 5] = [Self::Classic, Self::Rounded, Self::Cove, Self::Step, Self::Ogee];

    pub fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Rounded => "rounded",
            Self::Cove => "cove",
            Self::Step => "step",
            Self::Ogee => "ogee",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == s)
    }

    /// The cross-section at `s` (0..1): (share of the width, share of the height), both 0..1.
    fn at(self, s: f64) -> (f64, f64) {
        let s = s.clamp(0.0, 1.0);
        let q = s * std::f64::consts::FRAC_PI_2;
        match self {
            Self::Classic => (s, s),
            Self::Rounded => (1.0 - q.cos(), q.sin()),
            Self::Cove => (q.sin(), 1.0 - q.cos()),
            Self::Step if s >= 1.0 => (1.0, 1.0),
            Self::Step => {
                // Down, across, down, across.
                let k = s * 4.0;
                let (i, f) = (k.floor(), k.fract());
                match i as u8 {
                    0 => (0.0, f * 0.5),
                    1 => (f * 0.5, 0.5),
                    2 => (0.5, 0.5 + f * 0.5),
                    _ => (0.5 + f * 0.5, 1.0),
                }
            }
            Self::Ogee => {
                let h = (1.0 - (s * std::f64::consts::PI).cos()) / 2.0;
                (s, h)
            }
        }
    }

    /// Samples along the cross-section: the step shape needs its corners exactly.
    fn samples(self) -> Vec<f64> {
        match self {
            Self::Classic => vec![0.0, 1.0],
            Self::Step => vec![0.0, 0.25, 0.5, 0.75, 1.0],
            _ => (0..=8).map(|i| f64::from(i) / 8.0).collect(),
        }
    }
}

/// A bevel along the front (and with `both_sides` the back) edge of an extrusion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    pub shape: BevelShape,
    /// How far the bevel reaches across the face, in points.
    pub width: f64,
    /// How deep it reaches along the extrusion, in points.
    pub height: f64,
    /// How many times the cross-section repeats across the width.
    pub repeat: usize,
    /// The share (0..1) of the width left flat between repeats.
    pub space: f64,
    /// Carved into the shape (true) or added around it (false).
    pub inside: bool,
    pub both_sides: bool,
}

impl Default for Bevel {
    fn default() -> Self {
        Self { shape: BevelShape::Classic, width: 4.0, height: 4.0, repeat: 1, space: 0.0, inside: false, both_sides: false }
    }
}

/// Extrude & Bevel's options. Lengths are points, angles degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extrude {
    pub depth: f64,
    /// The turn of the back face against the front, about the shape's centre: 0° to 360°.
    pub twist: f64,
    /// The size of the back face, as a share (0.01..1) of the front's.
    pub taper: f64,
    /// Closed ends (a solid) or open ones (a hollow tube).
    pub cap: bool,
    pub bevel: Option<Bevel>,
}

impl Default for Extrude {
    fn default() -> Self {
        Self { depth: 50.0, twist: 0.0, taper: 1.0, cap: true, bevel: None }
    }
}

/// Inflate's options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inflate {
    /// The flat depth between the front and back, in points.
    pub depth: f64,
    /// How far the surface puffs out (0..100 %; 50 % rounds it like a half sphere).
    pub volume: f64,
    /// Puff the back out too; otherwise it stays flat.
    pub both_sides: bool,
    /// The turn of the back against the front, about the shape's centre: 0° to 360°.
    pub twist: f64,
    /// The size of the back, as a share (0.01..1) of the front's.
    pub taper: f64,
    /// A closed back (a solid) or none, so the inflated front is a shell open behind.
    pub cap: bool,
}

impl Default for Inflate {
    fn default() -> Self {
        Self { depth: 0.0, volume: 50.0, both_sides: false, twist: 0.0, taper: 1.0, cap: true }
    }
}

/// One projected polygon. Holes are contours of their own (nonzero winding).
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    pub contours: Vec<Vec<Point>>,
    pub depth: f64,
    pub brightness: f64,
    pub illumination: [f64; 3],
    /// Smooth lighting on an inflated triangle, before clipping individual RGB channels.
    pub rgb_gradient: Option<crate::RgbGradient>,
    /// The contours in view space, before projection, for tests that measure depths.
    #[cfg(test)]
    view: Vec<Vec<V3>>,
}

impl Facet {
    /// About how thick the facet is across: twice its area over its perimeter (a w × l rectangle
    /// long against its width gives about w).
    pub fn thickness(&self) -> f64 {
        let area: f64 = self.contours.iter().map(|c| signed_area(c)).sum::<f64>().abs();
        let perimeter: f64 =
            self.contours.iter().map(|c| c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.distance(*b)).sum::<f64>()).sum();
        if perimeter > 0.0 { 2.0 * area / perimeter } else { 0.0 }
    }

    pub fn path(&self) -> PathData {
        let mut bp = BezPath::new();
        for c in &self.contours {
            let Some(first) = c.first() else { continue };
            bp.move_to(*first);
            for p in c.iter().skip(1) {
                bp.line_to(*p);
            }
            bp.close_path();
        }
        PathData::from_bezpath(&bp)
    }
}

fn finite(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

impl Extrude {
    fn sanitized(self) -> Self {
        Self {
            depth: finite(self.depth, 50.0, 0.0, 2000.0),
            twist: finite(self.twist, 0.0, 0.0, 360.0),
            taper: finite(self.taper, 1.0, 0.01, 1.0),
            bevel: self.bevel.map(|b| Bevel {
                width: finite(b.width, 4.0, 0.0, 1000.0),
                height: finite(b.height, 4.0, 0.0, 1000.0),
                repeat: b.repeat.clamp(1, 32),
                space: finite(b.space, 0.0, 0.0, 0.9),
                ..b
            }),
            ..self
        }
    }
}

impl Inflate {
    fn sanitized(self) -> Self {
        Self {
            depth: finite(self.depth, 0.0, 0.0, 2000.0),
            volume: finite(self.volume, 50.0, 0.0, 100.0),
            twist: finite(self.twist, 0.0, 0.0, 360.0),
            taper: finite(self.taper, 1.0, 0.01, 1.0),
            ..self
        }
    }
}

/// Outline points Extrude & Bevel and Inflate sample at most: every point costs walls, bevel
/// bands and distance tests.
const MAX_SOLID_POINTS: usize = 2048;
/// Outline points Rotate samples at most: it only turns the outline, so an outlined paragraph fits.
const MAX_PLANE_POINTS: usize = 65536;
/// Pieces a twisted wall's strip is cut into along one edge at most.
const MAX_PIECES: usize = 64;

/// How many straight pieces follow `c` within `tolerance` (Wang's bound on a cubic's distance from
/// its chords), 1 to 32.
fn pieces(c: vectorcraft_geom::CubicBez, tolerance: f64) -> u32 {
    let d = |a: Point, b: Point, c: Point| (a.to_vec2() - 2.0 * b.to_vec2() + c.to_vec2()).hypot();
    let bend = d(c.p0, c.p1, c.p2).max(d(c.p1, c.p2, c.p3));
    let n = (0.75 * bend / tolerance).sqrt().ceil();
    if n.is_finite() { (n as u32).clamp(1, 32) } else { 32 }
}

/// The rings of a shape: closed polygons with the filled side on their left, (-dy, dx): outer
/// outlines run one way and holes the other, so nonzero winding fills them as the art is. Curves
/// are followed within a tolerance of 1/2000 of `size` (at least 0.05 pt), so small curves take
/// few points and large ones more; more than `limit` points in all is refused.
///
/// Under the nonzero `rule` an outline with filled art on both sides is no edge of the shape and
/// is left out: a contour inside another that runs the same way doesn't cut a hole, as it would
/// if nesting alone decided (#1724).
fn rings(path: &PathData, rule: FillRule, size: f64, limit: usize) -> Result<Vec<Vec<Point>>, String> {
    let tolerance = (size / 2000.0).max(0.05);
    let mut count = 0;
    let mut out = vec![];
    for sp in &path.subpaths {
        let segments = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        if segments > limit {
            return Err("The shape is too complex for a 3D effect".into());
        }
        let mut ring: Vec<Point> = vec![];
        if let Some(first) = sp.anchors.first() {
            ring.push(first.p);
        }
        for i in 0..segments {
            let curve = sp.segment(i);
            let samples = if sp.segment_is_line(i) { 1 } else { pieces(curve, tolerance) };
            count += samples as usize;
            if count > limit {
                return Err("The shape is too complex for a 3D effect".into());
            }
            for k in 1..=samples {
                ring.push(curve.eval(f64::from(k) / f64::from(samples)));
            }
        }
        if ring.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e8 || p.y.abs() > 1e8) {
            return Err("The shape contains invalid coordinates".into());
        }
        ring.dedup_by(|a, b| a.distance(*b) < 1e-9);
        while ring.len() > 1 && ring.first().zip(ring.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-9) {
            ring.pop();
        }
        if ring.len() >= 3 && signed_area(&ring).abs() > 1e-9 {
            out.push(ring);
        }
    }
    if rule == FillRule::NonZero {
        let turn = |r: &[Point]| if signed_area(r) > 0.0 { 1 } else { -1 };
        let mut edge = Vec::with_capacity(out.len());
        for (i, r) in out.iter().enumerate() {
            let p = r.first().copied().unwrap_or_default();
            // The winding number just outside the ring, and just inside it.
            let outside: i32 = out.iter().enumerate().filter(|(j, o)| *j != i && contains(o, p)).map(|(_, o)| turn(o)).sum();
            edge.push((outside != 0) != (outside + turn(r) != 0));
        }
        let mut edge = edge.into_iter();
        out.retain(|_| edge.next().unwrap_or(true));
    }
    // A ring inside an odd number of others is a hole.
    let holes: Vec<bool> = out
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let p = r.first().copied().unwrap_or_default();
            out.iter().enumerate().filter(|(j, o)| *j != i && contains(o, p)).count() % 2 == 1
        })
        .collect();
    for (r, hole) in out.iter_mut().zip(holes) {
        if (signed_area(r) > 0.0) == hole {
            r.reverse();
        }
    }
    Ok(out)
}

fn signed_area(r: &[Point]) -> f64 {
    r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

fn contains(r: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (a, b) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// The unit normal on the left of `a`→`b`: towards the filled side.
fn left(a: Point, b: Point) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l = dx.hypot(dy);
    if l < 1e-12 { (0.0, 0.0) } else { (-dy / l, dx / l) }
}

/// `ring` moved `w` points towards its filled side (negative: away from it), each corner along
/// its bisector, its mitre limited to four times `w` so sharp corners don't shoot off. Each point
/// stays the partner of its source point, so rings at different insets join face to face.
fn inset(ring: &[Point], w: f64) -> Vec<Point> {
    if w == 0.0 {
        return ring.to_vec();
    }
    let n = ring.len();
    (0..n)
        .map(|i| {
            let p = ring[i];
            let prev = ring[(i + n - 1) % n];
            let next = ring[(i + 1) % n];
            let (ax, ay) = left(prev, p);
            let (bx, by) = left(p, next);
            let (mut mx, mut my) = (ax + bx, ay + by);
            let ml = mx.hypot(my);
            if ml < 1e-9 {
                (mx, my) = (ax, ay);
            } else {
                (mx, my) = (mx / ml, my / ml);
            }
            let cos = (mx * ax + my * ay).max(0.25);
            Point::new(p.x + mx * w / cos, p.y + my * w / cos)
        })
        .collect()
}

/// Turns 3D points into document points and shades faces for one view.
struct Camera {
    view: Revolve,
    origin: Point,
    /// The eye's distance in front of the centre; `None` without perspective.
    eye: Option<f64>,
    /// Deeper than any part of the object: faces drawn first go this far behind it.
    far: f64,
    light: V3,
}

impl Camera {
    fn new(view: View, bounds: Rect, depth: f64) -> Self {
        let view = view.revolve();
        let [lx, ly, lz] = view.light_direction();
        let size = (bounds.width() / 2.0).max(bounds.height() / 2.0).max(depth / 2.0).max(1.0);
        Self { view, origin: bounds.center(), eye: eye_distance(view.perspective, size), far: 16.0 * size, light: V3(lx, ly, lz) }
    }
    fn rotate(&self, v: V3) -> V3 {
        self.view.rotate(v)
    }
    /// A point of the art, `z` points in front of its plane, in view space.
    fn place(&self, p: Point, z: f64) -> V3 {
        self.rotate(V3(p.x - self.origin.x, p.y - self.origin.y, z))
    }
    fn project(&self, v: V3) -> Point {
        let f = perspective_scale(self.eye, v.2);
        Point::new(self.origin.x + v.0 * f, self.origin.y + v.1 * f)
    }
    /// The cosine between a face's outward unit normal `n` and the direction from `at` (both in
    /// view space) to the eye: positive when the face is turned towards the viewer.
    fn facing(&self, at: V3, n: V3) -> f64 {
        let to_eye = self.eye.map_or(V3(0.0, 0.0, 1.0), |e| V3(0.0, 0.0, e).sub(at).unit());
        n.dot(to_eye)
    }
    fn brightness(&self, n: V3) -> f64 {
        crate::brightness(&self.view, n, self.light)
    }
}

/// Facets being collected for one solid.
struct Builder<'a> {
    camera: &'a Camera,
    /// A closed solid hides the faces turned away from the viewer; an open one shows their
    /// insides.
    closed: bool,
    facets: Vec<Facet>,
}

/// Where a face goes in the order the facets are drawn, back to front.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Order {
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

impl Builder<'_> {
    /// Add a face of view-space contours with outward normal `n` (view space), drawn in `order`.
    fn face(&mut self, contours: Vec<Vec<V3>>, n: V3, order: Order) -> Result<(), String> {
        let count: usize = contours.iter().map(Vec::len).sum();
        if count < 3 {
            return Ok(());
        }
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
        } else if self.closed && facing > -0.02 {
            (n, true)
        } else if self.closed {
            return Ok(());
        } else {
            (V3(-n.0, -n.1, -n.2), false)
        };
        #[cfg(test)]
        let view = contours.clone();
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
            rgb_gradient: None,
            #[cfg(test)]
            view,
        });
        if self.facets.len() > MAX_FACES {
            return Err("The 3D surface is too complex; simplify the shape".into());
        }
        Ok(())
    }

    /// A planar surface triangle with shared, smooth outward normals. Culling and sorting
    /// still use its geometry, rather than the normals used only for lighting.
    fn smooth_triangle(&mut self, vertices: [V3; 3], normals: [V3; 3], back: bool) -> Result<(), String> {
        let contours = vec![vertices.to_vec()];
        let n = newell(&contours);
        let n = if back { V3(-n.0, -n.1, -n.2) } else { n };
        let centre = vertices.iter().fold(V3::default(), |s, v| V3(s.0 + v.0 / 3.0, s.1 + v.1 / 3.0, s.2 + v.2 / 3.0));
        let flip = !self.closed && self.camera.facing(centre, n.unit()) <= 0.0;
        let before = self.facets.len();
        self.face(contours, n, Order::Depth)?;
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
    /// direction (before rotation) the band faces away from the solid, for orienting normals.
    fn band(&mut self, a: &[V3], b: &[V3], cuts: &[usize], outward: impl Fn(usize) -> V3) -> Result<(), String> {
        let n = a.len().min(b.len());
        // At 0 and 1, the corners themselves.
        let lerp = |p: V3, q: V3, s: f64| {
            if s <= 0.0 {
                p
            } else if s >= 1.0 {
                q
            } else {
                V3(p.0 + (q.0 - p.0) * s, p.1 + (q.1 - p.1) * s, p.2 + (q.2 - p.2) * s)
            }
        };
        let order = if cuts.is_empty() { Order::Depth } else { Order::Twisted };
        for i in 0..n {
            let j = (i + 1) % n;
            let pieces = cuts.get(i).copied().unwrap_or(1).max(1);
            for k in 0..pieces {
                let (s0, s1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                let quad = vec![lerp(a[i], a[j], s0), lerp(a[i], a[j], s1), lerp(b[i], b[j], s1), lerp(b[i], b[j], s0)];
                // A twist bends the strip between two slices into a saddle: its normal is the whole
                // quad's average (Newell's), not one corner's triangle, which leans far to one side.
                let mut normal = newell(std::slice::from_ref(&quad));
                if normal.dot(normal) < 1e-18 {
                    continue;
                }
                if normal.dot(self.camera.rotate(outward(i))) < 0.0 {
                    normal = V3(-normal.0, -normal.1, -normal.2);
                }
                self.face(vec![quad], normal, order)?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<Facet> {
        self.facets.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        self.facets
    }
}

/// How many pieces each edge of a twisted wall is drawn in, from two neighboring slices of the
/// wall, `a` and `b` (partner rings): as many as make each piece about as long along the edge as
/// the band between the slices is wide there, 1 to [`MAX_PIECES`]. Twisted walls pass in front of
/// one another, and a strip the length of its edge has no one depth where another crosses it.
fn cuts(a: &[V3], b: &[V3]) -> Vec<usize> {
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
fn fit(cuts: &mut [Vec<usize>], bands: usize, room: usize) {
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

fn check_bounds(b: Rect) -> Result<(), String> {
    if [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
        Ok(())
    } else {
        Err("3D effects require finite coordinates within 100 million points".into())
    }
}

/// The art's outward direction at edge `i` of `ring` (before rotation), tilted towards the face
/// it borders by `z` (+1 front, -1 back, 0 a side wall).
fn outward(ring: &[Point], i: usize, z: f64) -> V3 {
    let n = ring.len();
    let (lx, ly) = left(ring[i], ring[(i + 1) % n]);
    V3(-lx, -ly, z)
}

/// The cross-section of a side: each ring of the shape at depth fraction `t` (0 front, 1 back),
/// turned by the twist and scaled by the taper about the centre of `bounds`.
fn section(ring: &[Point], bounds: Rect, o: &Extrude, t: f64) -> Vec<Point> {
    let turn = Turn::new(bounds, o.twist, o.taper, t);
    ring.iter().map(|p| turn.apply(*p)).collect()
}

/// The twist and taper at depth fraction `t` (0 front, 1 back): a turn by `twist · t` and a
/// scale from 1 to `taper`, about the centre of the bounds.
#[derive(Clone, Copy)]
struct Turn {
    centre: Point,
    sin: f64,
    cos: f64,
    scale: f64,
}

impl Turn {
    fn new(bounds: Rect, twist: f64, taper: f64, t: f64) -> Self {
        let (sin, cos) = (twist * t).to_radians().sin_cos();
        Self { centre: bounds.center(), sin, cos, scale: 1.0 + (taper - 1.0) * t }
    }
    fn apply(&self, p: Point) -> Point {
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
    // With the bevel inside, the cap is inset by its width and the wall is the outline; outside,
    // the cap is the outline and the wall is outset by the width.
    let wall = match bevel {
        Some(b) if !b.inside => -b.width,
        _ => 0.0,
    };
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
    // Twist and taper bend the walls: cut them into slices.
    let slices = if o.twist != 0.0 || o.taper != 1.0 { ((o.twist.abs() / 6.0).ceil() as usize).clamp(8, 64) } else { 1 };
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

/// Extrude & Bevel: the shape's outline extruded backwards, with optional twist, taper, bevels
/// and caps.
pub fn extrude(path: &PathData, rule: FillRule, bounds: Rect, options: Extrude, view: View) -> Result<Vec<Facet>, String> {
    check_bounds(bounds)?;
    let o = options.sanitized();
    // No depth: the flat shape, seen from either side.
    if o.depth <= 0.0 {
        return plane(path, rule, bounds, view);
    }
    let rings = rings(path, rule, bounds.width().max(bounds.height()), MAX_SOLID_POINTS)?;
    let camera = Camera::new(view, bounds, o.depth);
    let mut b = Builder { camera: &camera, closed: o.cap, facets: vec![] };
    let steps = steps(&o);
    let z = |t: f64| o.depth / 2.0 - o.depth * t;
    let ring_at =
        |ring: &[Point], s: &Step| -> Vec<V3> { section(&inset(ring, s.inset), bounds, &o, s.t).iter().map(|p| camera.place(*p, z(s.t))).collect() };
    // Every band of a twisted outline, bevels included, is cut as its walls' first slice is (with
    // bevels and no walls, as its shallowest band is), within the facet limit.
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
    for (ring, pieces) in rings.iter().zip(&pieces) {
        let placed: Vec<Vec<V3>> = steps.iter().map(|s| ring_at(ring, s)).collect();
        for (k, pair) in placed.windows(2).enumerate() {
            let ([a, c], Some(s0), Some(s1)) = (pair, steps.get(k), steps.get(k + 1)) else { continue };
            // A band between two front bevel steps borders the front, one reaching a back bevel
            // step the back; the rest are walls.
            let face = if s1.face < 0.0 {
                -1.0
            } else if s0.face > 0.0 && s1.face > 0.0 {
                1.0
            } else {
                0.0
            };
            b.band(a, c, pieces, |i| outward(ring, i, face))?;
        }
    }
    if o.cap {
        // A cap turned toward the viewer covers the walls it overlaps, even those nearer than its
        // center: twisted walls, and the walls of a fill inside the solid of its stroke.
        for (s, n) in [(steps.first(), 1.0), (steps.last(), -1.0)] {
            let Some(s) = s else { continue };
            b.face(rings.iter().map(|r| ring_at(r, s)).collect(), camera.rotate(V3(0.0, 0.0, n)), Order::Last)?;
        }
    }
    Ok(b.finish())
}

/// Rotate: the flat art turned in 3D about the centre of `bounds` (which may be larger than the
/// shape's own, so that every part of one object turns about the same point).
pub fn plane(path: &PathData, rule: FillRule, bounds: Rect, view: View) -> Result<Vec<Facet>, String> {
    check_bounds(bounds)?;
    let rings = rings(path, rule, bounds.width().max(bounds.height()), MAX_PLANE_POINTS)?;
    let camera = Camera::new(view, bounds, 0.0);
    let mut b = Builder { camera: &camera, closed: false, facets: vec![] };
    let contours: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| camera.place(*p, 0.0)).collect()).collect();
    // A plane is one flat face, seen from either side. Its normal is turned toward the viewer, so
    // it is drawn last, as a cap is, and planes drawn in one order keep the order they are given in.
    let up = camera.rotate(V3(0.0, 0.0, 1.0));
    let n = if camera.facing(V3::default(), up) < 0.0 { V3(-up.0, -up.1, -up.2) } else { up };
    b.face(contours, n, Order::Last)?;
    Ok(b.finish())
}

/// The distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-18 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0) };
    p.distance(Point::new(a.x + dx * t, a.y + dy * t))
}

/// The distance from `p` to the nearest edge of `rings`, edge by edge.
#[cfg(test)]
fn edge_distance(rings: &[Vec<Point>], p: Point) -> f64 {
    rings
        .iter()
        .flat_map(|r| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()))
        .map(|(a, b)| segment_distance(p, *a, *b))
        .fold(f64::INFINITY, f64::min)
}

/// The edges of `rings` filed in a grid of square buckets, so the nearest edge to a point is
/// found among the buckets around it rather than among all the edges.
struct Edges {
    edges: Vec<(Point, Point)>,
    normals: Vec<(Vec2, Vec2)>,
    origin: Point,
    size: f64,
    n: usize,
    buckets: Vec<Vec<u32>>,
}

impl Edges {
    fn new(rings: &[Vec<Point>]) -> Self {
        const N: usize = 64;
        let edges: Vec<(Point, Point)> =
            rings.iter().flat_map(|r| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(a, b)| (*a, *b))).collect();
        let normals = rings
            .iter()
            .flat_map(|r| {
                let normal = |i: usize| {
                    let (x, y) = left(r[i], r[(i + 1) % r.len()]);
                    Vec2::new(x, y)
                };
                (0..r.len()).map(move |i| {
                    let n = normal(i);
                    let at = |other: Vec2| if n.dot(other) > 0.7 { (n + other).normalize() } else { n };
                    (at(normal((i + r.len() - 1) % r.len())), at(normal((i + 1) % r.len())))
                })
            })
            .collect();
        let bounds = edges.iter().fold(None, |b: Option<Rect>, (a, c)| Some(b.map_or(Rect::from_points(*a, *c), |b| b.union_pt(*a).union_pt(*c))));
        let bounds = bounds.unwrap_or(Rect::ZERO);
        let size = (bounds.width().max(bounds.height()) / N as f64).max(1e-9);
        let mut buckets = vec![vec![]; N * N];
        let index = |v: f64, o: f64| (((v - o) / size).floor().max(0.0) as usize).min(N - 1);
        for (k, (a, b)) in edges.iter().enumerate() {
            let (i0, i1) = (index(a.x.min(b.x), bounds.x0), index(a.x.max(b.x), bounds.x0));
            let (j0, j1) = (index(a.y.min(b.y), bounds.y0), index(a.y.max(b.y), bounds.y0));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    buckets[j * N + i].push(k as u32);
                }
            }
        }
        Self { edges, normals, origin: bounds.origin(), size, n: N, buckets }
    }

    /// The bucket row or column of coordinate `v`, from the grid's origin `o` on that axis.
    fn cell(&self, v: f64, o: f64) -> isize {
        (((v - o) / self.size).floor() as isize).clamp(0, self.n as isize - 1)
    }

    /// Is `p` within `tolerance` of an edge? Only the buckets `p` could share an edge with are
    /// looked at.
    fn near(&self, p: Point, tolerance: f64) -> bool {
        self.touch(Rect::new(p.x - tolerance, p.y - tolerance, p.x + tolerance, p.y + tolerance)) && {
            let n = self.n as isize;
            let (i0, i1) = (self.cell(p.x - tolerance, self.origin.x), self.cell(p.x + tolerance, self.origin.x));
            let (j0, j1) = (self.cell(p.y - tolerance, self.origin.y), self.cell(p.y + tolerance, self.origin.y));
            (j0..=j1).any(|j| {
                (i0..=i1).any(|i| {
                    self.buckets[(j * n + i) as usize]
                        .iter()
                        .filter_map(|k| self.edges.get(*k as usize))
                        .any(|(a, b)| segment_distance(p, *a, *b) <= tolerance)
                })
            })
        }
    }

    /// Call `f` with each edge whose bounding box overlaps `r` (an edge in several of the
    /// buckets `r` covers comes more than once).
    fn each(&self, r: Rect, mut f: impl FnMut(Point, Point)) {
        let n = self.n as isize;
        for j in self.cell(r.y0, self.origin.y)..=self.cell(r.y1, self.origin.y) {
            for i in self.cell(r.x0, self.origin.x)..=self.cell(r.x1, self.origin.x) {
                for (a, b) in self.buckets[(j * n + i) as usize].iter().filter_map(|k| self.edges.get(*k as usize)) {
                    if a.x.min(b.x) <= r.x1 && a.x.max(b.x) >= r.x0 && a.y.min(b.y) <= r.y1 && a.y.max(b.y) >= r.y0 {
                        f(*a, *b);
                    }
                }
            }
        }
    }

    /// Does any edge pass through `r` (its bounding box, at least, overlapping it)?
    fn touch(&self, r: Rect) -> bool {
        let n = self.n as isize;
        let (i0, i1) = (self.cell(r.x0, self.origin.x), self.cell(r.x1, self.origin.x));
        let (j0, j1) = (self.cell(r.y0, self.origin.y), self.cell(r.y1, self.origin.y));
        (j0..=j1).any(|j| {
            (i0..=i1).any(|i| {
                self.buckets[(j * n + i) as usize]
                    .iter()
                    .filter_map(|k| self.edges.get(*k as usize))
                    .any(|(a, b)| a.x.min(b.x) <= r.x1 && a.x.max(b.x) >= r.x0 && a.y.min(b.y) <= r.y1 && a.y.max(b.y) >= r.y0)
            })
        })
    }

    /// The distance from `p` to the nearest edge: the buckets are searched in growing squares
    /// around `p`'s, until no unsearched bucket could hold a nearer edge.
    fn distance(&self, p: Point) -> f64 {
        self.closest(p).2
    }

    /// Closest outline point, its inward normal and distance. Curved outline samples share
    /// corner normals, while a sharp source corner keeps each edge's own direction.
    fn closest(&self, p: Point) -> (Point, Vec2, f64) {
        let (fx, fy) = ((p.x - self.origin.x) / self.size, (p.y - self.origin.y) / self.size);
        if !fx.is_finite() || !fy.is_finite() {
            return (p, Vec2::ZERO, f64::INFINITY);
        }
        let n = self.n as isize;
        let (ci, cj) = ((fx.floor() as isize).clamp(0, n - 1), (fy.floor() as isize).clamp(0, n - 1));
        // How far `p` is outside the grid: the nearest point of the grid to it lies in `p`'s bucket.
        let outside = (fx.clamp(0.0, n as f64) - fx).hypot(fy.clamp(0.0, n as f64) - fy) * self.size;
        let mut best = f64::INFINITY;
        let mut nearest = (p, Vec2::ZERO);
        for r in 0..n {
            for j in (cj - r)..=(cj + r) {
                for i in (ci - r)..=(ci + r) {
                    if (j - cj).abs() != r && (i - ci).abs() != r || i < 0 || j < 0 || i >= n || j >= n {
                        continue;
                    }
                    for k in &self.buckets[(j * n + i) as usize] {
                        if let Some((a, b)) = self.edges.get(*k as usize) {
                            let d = *b - *a;
                            let t = if d.hypot2() < 1e-18 { 0.0 } else { ((p - *a).dot(d) / d.hypot2()).clamp(0.0, 1.0) };
                            let q = Point::new(a.x + d.x * t, a.y + d.y * t);
                            let distance = p.distance(q);
                            if distance < best {
                                best = distance;
                                let n = self.normals.get(*k as usize).map_or(Vec2::ZERO, |(a, b)| *a * (1.0 - t) + *b * t);
                                nearest = (q, if n.hypot2() > 0.0 { n.normalize() } else { n });
                            }
                        }
                    }
                }
            }
            // Buckets r + 1 away are at least r buckets from any point of `p`'s own, and that
            // far across the grid from a `p` outside it (at right angles to its way in, at worst).
            if best <= outside.hypot(r as f64 * self.size) {
                break;
            }
        }
        (nearest.0, nearest.1, best)
    }
}

/// Is `p` filled by `rings` (nonzero winding, as they are oriented)?
fn filled(rings: &[Vec<Point>], p: Point) -> bool {
    rings.iter().filter(|r| contains(r, p)).count() % 2 == 1
}

/// `ring` cut to the rectangle `cell` (Sutherland–Hodgman: the rectangle is convex, the ring
/// needn't be).
fn clip(ring: &[Point], cell: Rect) -> Vec<Point> {
    let mut pts = ring.to_vec();
    let sides: [fn(Point, Rect) -> f64; 4] = [|p, c| p.x - c.x0, |p, c| c.x1 - p.x, |p, c| p.y - c.y0, |p, c| c.y1 - p.y];
    for side in sides {
        if pts.is_empty() {
            break;
        }
        let mut out = vec![];
        for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)).take(pts.len()) {
            let (da, db) = (side(*a, cell), side(*b, cell));
            if da >= 0.0 {
                out.push(*a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(a.lerp(*b, da / (da - db)));
            }
        }
        pts = out;
    }
    pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
    if pts.len() >= 3 && signed_area(&pts).abs() > 1e-12 { pts } else { vec![] }
}

/// The shape's surface as a membrane under pressure, held at zero along its outline (and its
/// holes): the solution of ∇²φ = −1 on a grid, with each node's distance to the outline along the
/// grid lines taken into account where it is less than a grid step (Shortley–Weller), so the
/// outline needn't fall on the grid. √φ rises vertically from the outline and rounds smoothly
/// inside, without the creases a distance-to-the-edge height has along a shape's middle.
struct Membrane {
    origin: Point,
    step: f64,
    nx: usize,
    ny: usize,
    /// φ at each node; outside the shape, extrapolated (negative) from the inside, so
    /// interpolation crosses zero near the outline.
    phi: Vec<f64>,
    peak: f64,
}

impl Membrane {
    /// `edges` is the index of `rings`' edges (`Edges::new(rings)`).
    fn solve(rings: &[Vec<Point>], edges: &Edges, origin: Point, step: f64, nx: usize, ny: usize) -> Self {
        let node = |i: usize, j: usize| Point::new(origin.x + i as f64 * step, origin.y + j as f64 * step);
        let idx = |i: usize, j: usize| j * nx + i;
        // A node on the outline itself is held at zero with the outside.
        let inside: Vec<bool> = (0..nx * ny)
            .map(|k| {
                let p = node(k % nx, k / nx);
                filled(rings, p) && !edges.near(p, 1e-6 * step)
            })
            .collect();
        // The nearest crossing of the outline from `p` along (dx, dy), within one step.
        let crossing = |p: Point, dx: f64, dy: f64| -> f64 {
            let mut best = step;
            let reach = Rect::from_points(p, Point::new(p.x + dx * step, p.y + dy * step));
            edges.each(reach, |a, b| {
                let t = if dy == 0.0 {
                    if (a.y > p.y) == (b.y > p.y) {
                        return;
                    }
                    (a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) - p.x) * dx
                } else {
                    if (a.x > p.x) == (b.x > p.x) {
                        return;
                    }
                    (a.y + (p.x - a.x) / (b.x - a.x) * (b.y - a.y) - p.y) * dy
                };
                if t > 1e-9 * step && t < best {
                    best = t;
                }
            });
            best
        };
        // Each inside node's four neighbours: (weight, neighbour inside or None at the outline).
        let mut stencil: Vec<[(f64, Option<usize>); 4]> = vec![[(0.0, None); 4]; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let k = idx(i, j);
                if !inside[k] {
                    continue;
                }
                let p = node(i, j);
                let dirs = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
                let h: Vec<f64> = dirs.iter().map(|(dx, dy)| crossing(p, *dx, *dy)).collect();
                for (d, (dx, dy)) in dirs.iter().enumerate() {
                    let opp = h[d ^ 1];
                    let w = 2.0 / (h[d] * (h[d] + opp));
                    let (ni, nj) = (i as isize + *dx as isize, j as isize + *dy as isize);
                    let neighbour = (h[d] >= step && ni >= 0 && nj >= 0 && (ni as usize) < nx && (nj as usize) < ny)
                        .then(|| idx(ni as usize, nj as usize))
                        .filter(|n| inside[*n]);
                    stencil[k][d] = (w, neighbour);
                }
            }
        }
        // Successive over-relaxation.
        let mut phi = vec![0.0; nx * ny];
        let tolerance = 1e-9 * step * step * (nx * ny) as f64;
        for _ in 0..2000 {
            let mut change: f64 = 0.0;
            for k in 0..nx * ny {
                if !inside[k] {
                    continue;
                }
                let (mut sum, mut weight) = (1.0, 0.0);
                for (w, n) in stencil[k] {
                    weight += w;
                    sum += w * n.map_or(0.0, |n| phi[n]);
                }
                let next = phi[k] + 1.8 * (sum / weight - phi[k]);
                change = change.max((next - phi[k]).abs());
                phi[k] = next;
            }
            if change < tolerance {
                break;
            }
        }
        let peak = phi.iter().copied().fold(0.0, f64::max);
        // Outside nodes next to the shape: φ continued past the outline at the slope of their
        // inside neighbours, so values interpolated between them reach zero at the outline.
        let mut ext = phi.clone();
        for j in 0..ny {
            for i in 0..nx {
                let k = idx(i, j);
                if inside[k] {
                    continue;
                }
                let d_out = edges.distance(node(i, j));
                let mut slopes = vec![];
                for (di, dj) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                    let (ni, nj) = (i as isize + di, j as isize + dj);
                    if ni < 0 || nj < 0 || ni as usize >= nx || nj as usize >= ny || !inside[idx(ni as usize, nj as usize)] {
                        continue;
                    }
                    let d_in = edges.distance(node(ni as usize, nj as usize)).max(1e-9);
                    slopes.push(phi[idx(ni as usize, nj as usize)] / d_in);
                }
                if !slopes.is_empty() {
                    ext[k] = -d_out * slopes.iter().sum::<f64>() / slopes.len() as f64;
                }
            }
        }
        Self { origin, step, nx, ny, phi: ext, peak }
    }

    /// φ at `p`, interpolated between the four nodes around it.
    fn at(&self, p: Point) -> f64 {
        self.interpolate(p, |i, j| self.phi.get(j * self.nx + i).copied().unwrap_or(0.0))
    }

    /// Interpolate central node derivatives, rather than differentiating each bilinear cell.
    /// The latter jumps at every grid line and shows the solver's grid in the lighting.
    fn gradient(&self, p: Point) -> (f64, f64) {
        let value = |i, j| self.phi.get(j * self.nx + i).copied().unwrap_or(0.0);
        let dx = self.interpolate(p, |i, j| {
            let (a, b) = (i.saturating_sub(1), (i + 1).min(self.nx - 1));
            (value(b, j) - value(a, j)) / ((b - a).max(1) as f64 * self.step)
        });
        let dy = self.interpolate(p, |i, j| {
            let (a, b) = (j.saturating_sub(1), (j + 1).min(self.ny - 1));
            (value(i, b) - value(i, a)) / ((b - a).max(1) as f64 * self.step)
        });
        (dx, dy)
    }

    fn interpolate(&self, p: Point, v: impl Fn(usize, usize) -> f64) -> f64 {
        let fx = ((p.x - self.origin.x) / self.step).clamp(0.0, (self.nx - 1) as f64);
        let fy = ((p.y - self.origin.y) / self.step).clamp(0.0, (self.ny - 1) as f64);
        let (i, j) = ((fx.floor() as usize).min(self.nx.saturating_sub(2)), (fy.floor() as usize).min(self.ny.saturating_sub(2)));
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        if self.nx < 2 || self.ny < 2 {
            return v(0, 0);
        }
        let top = v(i, j) * (1.0 - tx) + v(i + 1, j) * tx;
        let bottom = v(i, j + 1) * (1.0 - tx) + v(i + 1, j + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

/// A bounded ear clipping pass on a simple cell contour. Keep its collinear border nodes:
/// those shared samples can have different heights once lifted onto the membrane. A tiny
/// cell containing a complete hole, or a complex/degenerate contour, keeps its original fill.
fn cell_triangles(pieces: &[Vec<Point>]) -> Option<Vec<[Point; 3]>> {
    let mut triangles = vec![];
    for ring in pieces {
        if ring.len() > 128 || signed_area(ring) <= 0.0 {
            return None;
        }
        let mut ring = ring.clone();
        ring.dedup_by(|a, b| a.distance(*b) < 1e-9);
        if ring.first().zip(ring.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-9) {
            ring.pop();
        }
        while ring.len() > 3 {
            let ear = (0..ring.len()).find(|&i| {
                let n = ring.len();
                let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
                if (b - a).cross(c - a) <= 1e-12 {
                    return false;
                }
                !ring.iter().enumerate().any(|(j, p)| {
                    j != i
                        && j != (i + n - 1) % n
                        && j != (i + 1) % n
                        && (b - a).cross(*p - a) >= -1e-12
                        && (c - b).cross(*p - b) >= -1e-12
                        && (a - c).cross(*p - c) >= -1e-12
                })
            })?;
            let n = ring.len();
            triangles.push([ring[(ear + n - 1) % n], ring[ear], ring[(ear + 1) % n]]);
            ring.remove(ear);
        }
        if let [a, b, c] = ring.as_slice() {
            triangles.push([*a, *b, *c]);
        }
    }
    Some(triangles)
}

/// The part of `outline` (its rings, each with its bounds) inside `cell`, as rings (holes running
/// the other way). Only the rings whose bounds meet the cell's go into the boolean.
fn material(outline: &[(Rect, vectorcraft_geom::SubPath)], cell: Rect) -> Option<Vec<Vec<Point>>> {
    let r = PathData::single(vectorcraft_geom::SubPath::polyline(
        &[Point::new(cell.x0, cell.y0), Point::new(cell.x1, cell.y0), Point::new(cell.x1, cell.y1), Point::new(cell.x0, cell.y1)],
        true,
    ));
    let near = PathData::new(
        outline.iter().filter(|(b, _)| b.x0 <= cell.x1 && cell.x0 <= b.x1 && b.y0 <= cell.y1 && cell.y0 <= b.y1).map(|(_, s)| s.clone()).collect(),
    );
    let rule = FillRule::NonZero;
    let part =
        vectorcraft_pathops::try_boolean(&r, rule, &near, rule, vectorcraft_pathops::BoolOp::Intersect, vectorcraft_pathops::DEFAULT_PRECISION)
            .ok()?;
    rings(&part, rule, cell.width(), MAX_PLANE_POINTS).ok()
}

/// A shape to inflate: its rings and the membrane under them on the grid of the bounds.
struct Puff {
    rings: Vec<Vec<Point>>,
    /// The index of the rings' edges.
    edges: Edges,
    cell: f64,
    nx: usize,
    ny: usize,
    m: Membrane,
    /// How high the front rises at its peak.
    height: f64,
}

/// The rings of `path` and its membrane on the grid of `bounds`; `None` for a shape with no area.
fn puff(path: &PathData, rule: FillRule, bounds: Rect, o: &Inflate) -> Result<Option<Puff>, String> {
    let rings = rings(path, rule, bounds.width().max(bounds.height()), MAX_SOLID_POINTS)?;
    if rings.is_empty() {
        return Ok(None);
    }
    let size = bounds.width().max(bounds.height());
    if size <= 0.0 {
        return Ok(None);
    }
    const CELLS: usize = 48;
    let cell = size / CELLS as f64;
    let (nx, ny) = (((bounds.width() / cell).ceil() as usize).max(1), ((bounds.height() / cell).ceil() as usize).max(1));
    let edges = Edges::new(&rings);
    let m = Membrane::solve(&rings, &edges, bounds.origin(), cell, nx + 1, ny + 1);
    // A circle of radius r puffs into a half sphere of radius r at 50 % (its membrane peaks at r²/4).
    let height = 2.0 * m.peak.sqrt() * o.volume / 50.0;
    Ok(Some(Puff { rings, edges, cell, nx, ny, m, height }))
}

/// Inflate: the shape puffed out like a cushion, its edge kept where the art's outline is.
pub fn inflate(path: &PathData, rule: FillRule, bounds: Rect, options: Inflate, view: View) -> Result<Vec<Facet>, String> {
    inflate_together(&[(path, rule)], bounds, options, view).pop().unwrap_or(Ok(vec![]))
}

/// Inflate for several shapes of one object (a fill and the outlines of its strokes): each puffs
/// on its own over the grid of `bounds`, and one eye sees them all, as far away as the highest
/// puff needs with Perspective. One result for each shape, in order.
pub fn inflate_together(shapes: &[(&PathData, FillRule)], bounds: Rect, options: Inflate, view: View) -> Vec<Result<Vec<Facet>, String>> {
    let o = options.sanitized();
    let puffs: Vec<Result<Option<Puff>, String>> =
        shapes.iter().map(|(path, rule)| check_bounds(bounds).and_then(|()| puff(path, *rule, bounds, &o))).collect();
    let reach = puffs.iter().flatten().flatten().map(|p| p.height).fold(0.0, f64::max);
    puffs
        .into_iter()
        .map(|p| match p {
            Ok(Some(p)) => inflated(p, bounds, o, view, reach),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e),
        })
        .collect()
}

/// The facets of one inflated shape, for a camera that allows for a puff `reach` high (or the
/// shape's own, if higher): `reach` is the highest puff of the shapes drawn together.
fn inflated(p: Puff, bounds: Rect, o: Inflate, view: View, reach: f64) -> Result<Vec<Facet>, String> {
    let Puff { rings, edges, cell, nx, ny, m, height } = p;
    let outline: Vec<(Rect, vectorcraft_geom::SubPath)> = rings
        .iter()
        .filter_map(|r| {
            let b = r.iter().skip(1).fold(Rect::from_points(*r.first()?, *r.first()?), |b, p| b.union_pt(*p));
            Some((b, vectorcraft_geom::SubPath::polyline(r, true)))
        })
        .collect();
    // When no shape puffs (no volume, or shapes too narrow for the grid), every front (and a back
    // that puffs too) lies flat in one plane: its cells are drawn last, as caps are. If one shape
    // puffs, a flat front can be behind its puff, so the cells keep their depths.
    let flat = reach <= 0.0;
    let lift = |p: Point| {
        if m.peak <= 0.0 || edges.near(p, 1e-9) {
            return 0.0;
        }
        height * (m.at(p).max(0.0) / m.peak).sqrt()
    };
    let normal = |p: Point| {
        let (dx, dy) = m.gradient(p);
        let (_, inward, distance) = edges.closest(p);
        let t = (distance / (2.0 * cell)).clamp(0.0, 1.0);
        let weight = (1.0 - t).powi(2) * (1.0 + 2.0 * t);
        let gradient = Vec2::new(dx, dy);
        let slope = gradient.dot(inward).max(0.0);
        // The zero boundary condition also fixes the tangential derivative at the outline.
        // Extrapolated outside solver nodes cannot supply it reliably. Blend to that known
        // direction and its limiting phi near the rim, without showing the grid in highlights.
        let gradient = gradient * (1.0 - weight) + inward * (slope * weight);
        let phi = m.at(p).max(0.0) * (1.0 - weight) + slope * distance * weight;
        // Multiplying the height-function normal by 2√(peak φ) avoids division by
        // zero at the rim, where the limiting normal is horizontal.
        let n = V3(-height * gradient.x, -height * gradient.y, 2.0 * (m.peak * phi).sqrt()).unit();
        if n.dot(n) > 0.5 { n } else { V3(0.0, 0.0, 1.0) }
    };
    let smooth = view.shade && view.has_colored_lights();
    let camera = Camera::new(view, bounds, o.depth + 2.0 * height.max(reach));
    // Without its back cap the object is a shell open behind, its inside showing.
    let mut b = Builder { camera: &camera, closed: o.cap, facets: vec![] };
    let half = o.depth / 2.0;
    // The back: turned and scaled by the twist and taper, its puff too.
    let back_turn = Turn::new(bounds, o.twist, o.taper, 1.0);
    // The grid's cells, those the surface rises steeply across (near the outline) cut into
    // halves or quarters each way, so the steep rim is drawn in facets as fine as its curve.
    // Every corner sits on one fine lattice (a quarter cell), computed the same way wherever it
    // is used, and a cell's border takes its finer neighbour's lattice points too: where cells
    // of different fineness meet, their edges follow the surface through the same points, with
    // no cracks.
    const FINE: usize = 4;
    let unit = cell / FINE as f64;
    let at = |qx: usize, qy: usize| Point::new(bounds.x0 + qx as f64 * unit, bounds.y0 + qy as f64 * unit);
    let base = |i: usize, j: usize| Rect::from_points(at(i * FINE, j * FINE), at((i + 1) * FINE, (j + 1) * FINE));
    // The shape's part of a rectangle, as rings.
    let cut = |r: Rect| -> Vec<Vec<Point>> {
        // A cell no edge crosses is wholly inside the shape or wholly outside it.
        if !edges.touch(r) {
            let corners = vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
            return if filled(&rings, r.center()) { vec![corners] } else { vec![] };
        }
        let pieces: Vec<Vec<Point>> = rings.iter().map(|ring| clip(ring, r)).filter(|c| !c.is_empty()).collect();
        if pieces.iter().map(|p| signed_area(p)).sum::<f64>() <= 1e-9 * r.area() {
            return vec![];
        }
        // Where a hole's piece cancels part of an outline's, that only holds flat: lifted onto
        // the surface the two no longer meet. Such a cell takes the shape's own part of it.
        let mixed = pieces.iter().any(|p| signed_area(p) < 0.0) && pieces.iter().any(|p| signed_area(p) > 0.0);
        if mixed { material(&outline, r).unwrap_or(pieces) } else { pieces }
    };
    // How many times each cell is halved each way (0, 1 or 2), from how steeply it rises.
    let levels: Vec<u32> = (0..nx * ny)
        .map(|k| {
            let pieces = cut(base(k % nx, k / nx));
            let (lo, hi) = pieces.iter().flatten().map(|q| lift(*q)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| (lo.min(h), hi.max(h)));
            let rise = if pieces.is_empty() { 0.0 } else { (hi - lo) / cell };
            if rise > 6.0 {
                2
            } else if rise > 2.0 {
                1
            } else {
                0
            }
        })
        .collect();
    let level =
        |i: isize, j: isize| -> u32 { if i < 0 || j < 0 || i as usize >= nx || j as usize >= ny { 0 } else { levels[j as usize * nx + i as usize] } };
    for j in 0..ny {
        for i in 0..nx {
            let l = level(i as isize, j as isize);
            let step = FINE >> l;
            // The lattice spacing along each side of this cell: its own or its neighbour's.
            let (ii, jj) = (i as isize, j as isize);
            let side_step = |dl: u32| FINE >> l.max(dl);
            let (left, right, top, bottom) =
                (side_step(level(ii - 1, jj)), side_step(level(ii + 1, jj)), side_step(level(ii, jj - 1)), side_step(level(ii, jj + 1)));
            let (bx0, by0, bx1, by1) = (i * FINE, j * FINE, (i + 1) * FINE, (j + 1) * FINE);
            for sy in (by0..by1).step_by(step) {
                for sx in (bx0..bx1).step_by(step) {
                    let r = Rect::from_points(at(sx, sy), at(sx + step, sy + step));
                    let pieces = cut(r);
                    if pieces.is_empty() {
                        continue;
                    }
                    // Lattice points along the cell's own border, between each edge's ends.
                    let stitch = |a: Point, b: Point| -> Vec<Point> {
                        let tol = 1e-7 * unit;
                        let along = |fixed_x: Option<usize>, fixed_y: Option<usize>, every: usize| -> Vec<Point> {
                            let mut out = vec![];
                            match (fixed_x, fixed_y) {
                                (Some(qx), None) => {
                                    let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
                                    for qy in (by0..=by1).step_by(every) {
                                        let p = at(qx, qy);
                                        if p.y > lo + tol && p.y < hi - tol {
                                            out.push(p);
                                        }
                                    }
                                }
                                (None, Some(qy)) => {
                                    let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
                                    for qx in (bx0..=bx1).step_by(every) {
                                        let p = at(qx, qy);
                                        if p.x > lo + tol && p.x < hi - tol {
                                            out.push(p);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if (a.x > b.x) || (a.y > b.y) {
                                out.reverse();
                            }
                            out
                        };
                        let on = |v: f64, w: f64| (v - w).abs() <= tol;
                        let (x0, x1, y0, y1) = (at(bx0, by0).x, at(bx1, by1).x, at(bx0, by0).y, at(bx1, by1).y);
                        if on(a.x, x0) && on(b.x, x0) {
                            along(Some(bx0), None, left)
                        } else if on(a.x, x1) && on(b.x, x1) {
                            along(Some(bx1), None, right)
                        } else if on(a.y, y0) && on(b.y, y0) {
                            along(None, Some(by0), top)
                        } else if on(a.y, y1) && on(b.y, y1) {
                            along(None, Some(by1), bottom)
                        } else {
                            vec![]
                        }
                    };
                    let pieces: Vec<Vec<Point>> = pieces
                        .iter()
                        .map(|p| {
                            let mut out = Vec::with_capacity(p.len());
                            for (a, b) in p.iter().zip(p.iter().cycle().skip(1)).take(p.len()) {
                                out.push(*a);
                                out.extend(stitch(*a, *b));
                            }
                            out
                        })
                        .collect();
                    if height > 0.0
                        && smooth
                        && let Some(triangles) = cell_triangles(&pieces)
                    {
                        for points in triangles {
                            let front = points.map(|q| camera.place(q, half + lift(q)));
                            b.smooth_triangle(front, points.map(|q| camera.rotate(normal(q))), false)?;
                            if o.both_sides && o.cap {
                                let back = points.map(|q| camera.place(back_turn.apply(q), -half - lift(q) * back_turn.scale));
                                let normals = points.map(|q| {
                                    let n = normal(q);
                                    // A uniform taper scales all three coordinates equally;
                                    // twist turns the back normal about the shape's z-axis.
                                    camera.rotate(V3(back_turn.cos * n.0 - back_turn.sin * n.1, back_turn.sin * n.0 + back_turn.cos * n.1, -n.2))
                                });
                                b.smooth_triangle(back, normals, true)?;
                            }
                        }
                        continue;
                    }
                    let lifts: Vec<Vec<f64>> = pieces.iter().map(|p| p.iter().map(|q| lift(*q)).collect()).collect();
                    // Each piece lifted onto the surface. Its normal is the lifted piece's own average
                    // (Newell's), not the slope at one point: near the edge the surface turns
                    // vertical, and a point sample there misjudges which way a whole piece faces.
                    let front: Vec<Vec<V3>> =
                        pieces.iter().zip(&lifts).map(|(p, h)| p.iter().zip(h).map(|(q, h)| camera.place(*q, half + h)).collect()).collect();
                    let n = newell(&front);
                    let order = if flat { Order::Last } else { Order::Depth };
                    b.face(front, n, order)?;
                    if o.both_sides && o.cap {
                        // The back winds the same way round as the front, so its outward normal is
                        // reversed.
                        let back: Vec<Vec<V3>> = pieces
                            .iter()
                            .zip(&lifts)
                            .map(|(p, h)| p.iter().zip(h).map(|(q, h)| camera.place(back_turn.apply(*q), -half - h * back_turn.scale)).collect())
                            .collect();
                        let n = newell(&back);
                        b.face(back, V3(-n.0, -n.1, -n.2), order)?;
                    }
                }
            }
        }
    }
    if !o.both_sides && o.cap {
        let back: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| camera.place(back_turn.apply(*p), -half)).collect()).collect();
        // The back turned toward the viewer covers the walls it overlaps, even those nearer than
        // its center: twisted walls, and the walls of a fill inside the solid of its stroke.
        b.face(back, camera.rotate(V3(0.0, 0.0, -1.0)), Order::Last)?;
    }
    if o.depth > 0.0 {
        // The sides, cut into slices that follow a twist or taper.
        let slices = if o.twist != 0.0 || o.taper != 1.0 { ((o.twist.abs() / 6.0).ceil() as usize).clamp(8, 64) } else { 1 };
        let at = |ring: &[Point], k: usize| -> Vec<V3> {
            let t = k as f64 / slices as f64;
            let turn = Turn::new(bounds, o.twist, o.taper, t);
            ring.iter().map(|p| camera.place(turn.apply(*p), half - o.depth * t)).collect()
        };
        let mut pieces: Vec<Vec<usize>> = rings.iter().map(|r| if o.twist != 0.0 { cuts(&at(r, 0), &at(r, 1)) } else { vec![] }).collect();
        fit(&mut pieces, slices, MAX_FACES.saturating_sub(b.facets.len()));
        for (ring, pieces) in rings.iter().zip(&pieces) {
            for k in 0..slices {
                b.band(&at(ring, k), &at(ring, k + 1), pieces, |i| outward(ring, i, 0.0))?;
            }
        }
    }
    Ok(b.finish())
}

#[cfg(test)]
#[path = "tests_solid.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_inflate_shading.rs"]
mod tests_inflate_shading;
