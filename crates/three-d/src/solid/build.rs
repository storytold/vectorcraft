use super::rings::{MAX_PIECES, MAX_PLANE_POINTS, MAX_SOLID_POINTS, inset, left, rings, signed_area};
use super::{Bevel, Extrude, Facet, FillRule, MAX_FACES, PathData, Point, Rect, Revolve, V3, View, eye_distance, perspective_scale};

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
    /// The cosine between a face's outward unit normal `n` and the direction from `at` (both in
    /// view space) to the eye: positive when the face is turned towards the viewer.
    pub(super) fn facing(&self, at: V3, n: V3) -> f64 {
        let to_eye = self.eye.map_or(V3(0.0, 0.0, 1.0), |e| V3(0.0, 0.0, e).sub(at).unit());
        n.dot(to_eye)
    }
    pub(super) fn brightness(&self, n: V3) -> f64 {
        crate::brightness(&self.view, n, self.light)
    }
}

/// Facets being collected for one solid.
pub(super) struct Builder<'a> {
    pub(super) camera: &'a Camera,
    /// A closed solid hides the faces turned away from the viewer; an open one shows their
    /// insides.
    pub(super) closed: bool,
    pub(super) facets: Vec<Facet>,
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

impl Builder<'_> {
    /// Add a face of view-space contours with outward normal `n` (view space), drawn in `order`.
    pub(super) fn face(&mut self, contours: Vec<Vec<V3>>, n: V3, order: Order) -> Result<(), String> {
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
    pub(super) fn smooth_triangle(&mut self, vertices: [V3; 3], normals: [V3; 3], back: bool) -> Result<(), String> {
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
    pub(super) fn band(&mut self, a: &[V3], b: &[V3], cuts: &[usize], outward: impl Fn(usize) -> V3) -> Result<(), String> {
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

    pub(super) fn finish(mut self) -> Vec<Facet> {
        self.facets.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        self.facets
    }
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
/// it borders by `z` (+1 front, -1 back, 0 a side wall).
pub(super) fn outward(ring: &[Point], i: usize, z: f64) -> V3 {
    let n = ring.len();
    let (lx, ly) = left(ring[i], ring[(i + 1) % n]);
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
