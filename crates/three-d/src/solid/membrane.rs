use super::build::segment_distance;
use super::rings::{MAX_PLANE_POINTS, contains, left, rings, signed_area};
use super::{FillRule, PathData, Point, Rect, Vec2};

/// The edges of `rings` filed in a grid of square buckets, so the nearest edge to a point is
/// found among the buckets around it rather than among all the edges.
pub(super) struct Edges {
    edges: Vec<(Point, Point)>,
    normals: Vec<(Vec2, Vec2)>,
    origin: Point,
    size: f64,
    n: usize,
    buckets: Vec<Vec<u32>>,
}

impl Edges {
    pub(super) fn new(rings: &[Vec<Point>]) -> Self {
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
    pub(super) fn near(&self, p: Point, tolerance: f64) -> bool {
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
    pub(super) fn touch(&self, r: Rect) -> bool {
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
    pub(super) fn distance(&self, p: Point) -> f64 {
        self.closest(p).2
    }

    /// Closest outline point, its inward normal and distance. Curved outline samples share
    /// corner normals, while a sharp source corner keeps each edge's own direction.
    pub(super) fn closest(&self, p: Point) -> (Point, Vec2, f64) {
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
pub(super) fn filled(rings: &[Vec<Point>], p: Point) -> bool {
    rings.iter().filter(|r| contains(r, p)).count() % 2 == 1
}

/// `ring` cut to the rectangle `cell` (Sutherland–Hodgman: the rectangle is convex, the ring
/// needn't be).
pub(super) fn clip(ring: &[Point], cell: Rect) -> Vec<Point> {
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
pub(super) struct Membrane {
    origin: Point,
    step: f64,
    nx: usize,
    ny: usize,
    /// φ at each node; outside the shape, extrapolated (negative) from the inside, so
    /// interpolation crosses zero near the outline.
    phi: Vec<f64>,
    pub(super) peak: f64,
}

impl Membrane {
    /// `edges` is the index of `rings`' edges (`Edges::new(rings)`).
    pub(super) fn solve(rings: &[Vec<Point>], edges: &Edges, origin: Point, step: f64, nx: usize, ny: usize) -> Self {
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
    pub(super) fn at(&self, p: Point) -> f64 {
        self.interpolate(p, |i, j| self.phi.get(j * self.nx + i).copied().unwrap_or(0.0))
    }

    /// Interpolate central node derivatives, rather than differentiating each bilinear cell.
    /// The latter jumps at every grid line and shows the solver's grid in the lighting.
    pub(super) fn gradient(&self, p: Point) -> (f64, f64) {
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
pub(super) fn cell_triangles(pieces: &[Vec<Point>]) -> Option<Vec<[Point; 3]>> {
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

/// Most outline segments the booleans of one inflated object's mixed cells take together.
pub(super) const MAX_MATERIAL_SEGMENTS: usize = 4_000_000;

/// The part of `outline` (its rings, each with its bounds) inside `cell`, as rings (holes running
/// the other way). Only the rings whose bounds meet the cell's go into the boolean.
pub(super) fn material(outline: &[(Rect, vectorcraft_geom::SubPath)], cell: Rect) -> Option<Vec<Vec<Point>>> {
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
