use super::envelope::{EnvelopeKind, H_DOWN, H_LEFT, H_RIGHT, H_UP, envelope_grid, mesh_at};
use super::warp::{WarpStyle, flatten_subpath, resample, warp_point};
use super::{
    Affine, Anchor, Arc, Color, CubicBez, Deserialize, Node, ParamCurve, PathData, Point, Rect, Serialize, Shape, SubPath, Vec2, fifty, lerp_color,
    one,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshPoint {
    pub p: Point,
    pub color: Color,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Tangent handles as offsets from `p` (see [`H_RIGHT`] …).
    #[serde(default)]
    pub handles: [Vec2; 4],
}

/// A gradient mesh: `(rows+1)×(cols+1)` points, row-major.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientMesh {
    pub rows: u32,
    pub cols: u32,
    pub points: Vec<MeshPoint>,
}

/// One tessellated piece of a mesh: a small quad, its colour at the centre, and its colour and
/// opacity at each corner (in `pts` order), which a renderer shades between so neighbouring quads
/// meet without a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshQuad {
    pub pts: [Point; 4],
    pub color: Color,
    pub opacity: f32,
    pub corners: [(Color, f32); 4],
}

/// One flat-coloured piece of a mesh ([`GradientMesh::pieces`]): a closed outline through
/// tessellation points, its colour and opacity.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPiece {
    pub pts: Vec<Point>,
    pub color: Color,
    pub opacity: f32,
}

/// Create Gradient Mesh → Appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshAppearance {
    #[default]
    Flat,
    ToCenter,
    ToEdge,
}

impl MeshAppearance {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "flat" => Some(Self::Flat),
            "center" | "tocenter" | "to center" => Some(Self::ToCenter),
            "edge" | "toedge" | "to edge" => Some(Self::ToEdge),
            _ => None,
        }
    }
}

/// Default envelope fidelity.
pub fn default_fidelity() -> f64 {
    fifty()
}

const SIDE: usize = 65;

/// Four boundary curves (top TL→TR, right TR→BR, bottom BL→BR, left TL→BL), each resampled to
/// [`SIDE`] points, from the outline of a closed path split at the points nearest its bounding
/// box corners.
#[derive(Clone, Debug)]
pub struct Coons {
    top: Vec<Point>,
    right: Vec<Point>,
    bottom: Vec<Point>,
    left: Vec<Point>,
}

impl Coons {
    /// A rectangle.
    pub fn rect(r: Rect) -> Self {
        let line = |a: Point, b: Point| (0..SIDE).map(|i| a.lerp(b, i as f64 / (SIDE - 1) as f64)).collect::<Vec<_>>();
        let (tl, tr, br, bl) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
        Self { top: line(tl, tr), right: line(tr, br), bottom: line(bl, br), left: line(tl, bl) }
    }

    pub fn from_path(path: &PathData) -> Option<Self> {
        // The largest subpath is the outline.
        let sp = path.subpaths.iter().filter(|s| s.anchors.len() >= 2).max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))?;
        let r = path.bounds()?;
        let tol = (r.width() + r.height()).max(1e-6) / 2000.0;
        let mut poly = flatten_subpath(sp, tol);
        if poly.len() > 1
            && let (Some(a), Some(b)) = (poly.first(), poly.last())
            && a.distance(*b) < 1e-9
        {
            poly.pop();
        }
        if poly.len() < 3 {
            return Some(Self::rect(r));
        }
        // Orient clockwise in y-down (TL → TR → BR → BL).
        let area: f64 = (0..poly.len())
            .map(|i| {
                let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                p.x * q.y - q.x * p.y
            })
            .sum();
        if area < 0.0 {
            poly.reverse();
        }
        let nearest = |c: Point| (0..poly.len()).min_by(|i, j| poly[*i].distance_squared(c).total_cmp(&poly[*j].distance_squared(c))).unwrap_or(0);
        let tl = nearest(Point::new(r.x0, r.y0));
        let tr = nearest(Point::new(r.x1, r.y0));
        let br = nearest(Point::new(r.x1, r.y1));
        let bl = nearest(Point::new(r.x0, r.y1));
        let n = poly.len();
        let arc = |i: usize, j: usize| -> Vec<Point> {
            let mut v = vec![poly[i]];
            let mut k = i;
            while k != j {
                k = (k + 1) % n;
                v.push(poly[k]);
            }
            v
        };
        let top = resample(&arc(tl, tr), SIDE);
        let right = resample(&arc(tr, br), SIDE);
        let mut bottom = resample(&arc(br, bl), SIDE);
        bottom.reverse();
        let mut left = resample(&arc(bl, tl), SIDE);
        left.reverse();
        Some(Self { top, right, bottom, left })
    }

    /// The point at `u` along a side; past its ends the end segments go on straight (art an
    /// envelope bends can reach past the content's box: outlined strokes).
    fn sample(side: &[Point], u: f64) -> Point {
        let s = u * (side.len() - 1) as f64;
        let i = (s.floor().max(0.0) as usize).min(side.len() - 2);
        side[i].lerp(side[i + 1], s - i as f64)
    }

    /// The surface point at (u, v) ∈ [0,1]².
    pub fn eval(&self, u: f64, v: f64) -> Point {
        let (t, b, l, r) = (Self::sample(&self.top, u), Self::sample(&self.bottom, u), Self::sample(&self.left, v), Self::sample(&self.right, v));
        let (p00, p10, p01, p11) = (self.top[0], self.top[SIDE - 1], self.bottom[0], self.bottom[SIDE - 1]);
        let c = (1.0 - v) * t.to_vec2() + v * b.to_vec2() + (1.0 - u) * l.to_vec2() + u * r.to_vec2()
            - ((1.0 - u) * (1.0 - v) * p00.to_vec2() + u * (1.0 - v) * p10.to_vec2() + (1.0 - u) * v * p01.to_vec2() + u * v * p11.to_vec2());
        c.to_point()
    }

    /// The four corners TL, TR, BR, BL.
    pub fn corners(&self) -> [Point; 4] {
        [self.top[0], self.top[SIDE - 1], self.bottom[SIDE - 1], self.bottom[0]]
    }
}

// =====================================================================================
// Gradient meshes
// =====================================================================================

/// The handles, as offsets from `ends`, of the cubic from `ends.0` to `ends.1` that comes closest
/// (least squares) to `side` between `span.0` and `span.1`; `None` when they can't be computed.
/// Each point of the side is matched to the curve's nearest point, refined over a few rounds.
fn fit_edge(side: &[Point], span: (f64, f64), ends: (Point, Point)) -> Option<(Vec2, Vec2)> {
    const N: usize = 32;
    const ROUNDS: usize = 6;
    let (p0, p3) = (ends.0, ends.1);
    let qs: Vec<Point> = (1..N).map(|i| Coons::sample(side, span.0 + (span.1 - span.0) * i as f64 / N as f64)).collect();
    let mut ts: Vec<f64> = (1..N).map(|i| i as f64 / N as f64).collect();
    let mut fit = None;
    for _ in 0..ROUNDS {
        let (mut c11, mut c12, mut c22) = (0.0, 0.0, 0.0);
        let (mut x1, mut x2) = (Vec2::ZERO, Vec2::ZERO);
        for (q, &t) in qs.iter().zip(&ts) {
            let s = 1.0 - t;
            let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
            let r = q.to_vec2() - b0 * p0.to_vec2() - b3 * p3.to_vec2();
            c11 += b1 * b1;
            c12 += b1 * b2;
            c22 += b2 * b2;
            x1 += b1 * r;
            x2 += b2 * r;
        }
        let det = c11 * c22 - c12 * c12;
        if det.abs() < 1e-12 {
            break;
        }
        let (p1, p2) = (((c22 * x1 - c12 * x2) / det).to_point(), ((c11 * x2 - c12 * x1) / det).to_point());
        if !(p1.is_finite() && p2.is_finite()) {
            break;
        }
        fit = Some((p1 - p0, p2 - p3));
        // One Newton step towards the nearest point of the curve for each point of the side.
        let c = CubicBez::new(p0, p1, p2, p3);
        let (a, b, k) = (p1 - p0, p2 - p1, p3 - p2);
        for (q, t) in qs.iter().zip(ts.iter_mut()) {
            let s = 1.0 - *t;
            let v = 3.0 * (s * s * a + 2.0 * s * *t * b + *t * *t * k);
            let acc = 6.0 * (s * (b - a) + *t * (k - b));
            let e = c.eval(*t) - *q;
            let den = v.hypot2() + e.dot(acc);
            if den.abs() > 1e-12 {
                *t = (*t - e.dot(v) / den).clamp(0.0, 1.0);
            }
        }
    }
    fit
}

/// The handles of one cubic standing for two edges that meet at point `mid.0`, whose handles
/// towards them are `mid.1` (towards `a`) and `mid.2` (towards `b`), as offsets from their outer
/// ends: each outer handle grown back by the share of the curve its edge had, which undoes the
/// split [`GradientMesh::insert_row`] and [`GradientMesh::insert_col`] make, so deleting a mesh
/// line keeps the curve of the border it crossed (#3187). The share is the split parameter, read
/// from the lengths of the middle handles. When the edges no longer fit one cubic (a point or a
/// handle was moved since the split), the cubic grown back would not pass through the middle
/// point, and the merged edge is straight.
fn merged_handles(a: (Point, Vec2), mid: (Point, Vec2, Vec2), b: (Point, Vec2)) -> (Vec2, Vec2) {
    let straight = ((b.0 - a.0) / 3.0, (a.0 - b.0) / 3.0);
    let (la, lb) = (mid.1.hypot(), mid.2.hypot());
    let t = la / (la + lb);
    if !(0.01..=0.99).contains(&t) {
        return straight;
    }
    let (ha, hb) = (a.1 / t, b.1 / (1.0 - t));
    let through = CubicBez::new(a.0, a.0 + ha, b.0 + hb, b.0).eval(t);
    let tol = 1e-3 * (a.0.distance(mid.0) + mid.0.distance(b.0)).max(1e-9);
    if ha.is_finite() && hb.is_finite() && through.distance(mid.0) <= tol { (ha, hb) } else { straight }
}

/// `0, 1/n, …, 1` for `n` (clamped to 1..=200) even steps.
fn even_steps(n: u32) -> Vec<f64> {
    let n = n.clamp(1, 200);
    (0..=n).map(|i| i as f64 / n as f64).collect()
}

/// The number of points of a `(rows+1)×(cols+1)` grid, `None` past `usize`: counts come from
/// files, and u32::MAX rows and columns (65,535 on 32-bit targets) would wrap to an empty grid.
pub(super) fn grid_len(rows: u32, cols: u32) -> Option<usize> {
    (rows as usize).checked_add(1)?.checked_mul((cols as usize).checked_add(1)?)
}

impl GradientMesh {
    /// The most quads one tessellation ([`Self::quads`]) makes, and so the most patches a mesh can
    /// have: about 170 bytes a quad, so about 170 MB. A file's mesh can have any number of patches
    /// and the view any zoom; the subdivision per patch gives way before memory does, and a mesh
    /// with more patches than that is malformed, like a point count that doesn't fit (#3592).
    pub const MAX_QUADS: usize = 1 << 20;

    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * (self.cols as usize + 1) + c
    }
    pub fn is_valid(&self) -> bool {
        self.rows >= 1 && self.cols >= 1 && grid_len(self.rows, self.cols) == Some(self.points.len()) && self.patches() <= Self::MAX_QUADS
    }

    /// The number of patches (rows times columns), saturating.
    fn patches(&self) -> usize {
        (self.rows as usize).saturating_mul(self.cols as usize)
    }

    /// A 1×1 mesh from its four corners (bottom-left, top-left, top-right, bottom-right in u/v
    /// order: row 0 is v = 0, columns follow u), each with its handles ([`H_RIGHT`] …).
    fn patch(corners: [(Point, [Vec2; 4], Color); 4]) -> Self {
        let point = |(p, handles, color): (Point, [Vec2; 4], Color)| MeshPoint { p, color, opacity: 1.0, handles };
        let [a, b, c, d] = corners;
        Self { rows: 1, cols: 1, points: vec![point(a), point(d), point(b), point(c)] }
    }

    /// A 1×1 mesh from a Coons patch's twelve boundary points (in the order PDF and PostScript
    /// shadings list them: v runs cp0 → cp3 along cp1 and cp2, u runs cp0 → cp9 along cp11 and
    /// cp10) and its corner colours (at cp0, cp3, cp6, cp9); `None` if a point isn't finite.
    pub fn coons(cp: &[Point; 12], colors: [Color; 4]) -> Option<Self> {
        if !cp.iter().all(|p| p.is_finite()) {
            return None;
        }
        let h = |a: usize, b: usize| cp[b] - cp[a];
        let handles = |right: Vec2, left: Vec2, down: Vec2, up: Vec2| {
            let mut h = [Vec2::ZERO; 4];
            (h[H_RIGHT], h[H_LEFT], h[H_DOWN], h[H_UP]) = (right, left, down, up);
            h
        };
        let [c0, c1, c2, c3] = colors;
        Some(Self::patch([
            (cp[0], handles(h(0, 11), Vec2::ZERO, h(0, 1), Vec2::ZERO), c0),
            (cp[3], handles(h(3, 4), Vec2::ZERO, Vec2::ZERO, h(3, 2)), c1),
            (cp[6], handles(Vec2::ZERO, h(6, 5), Vec2::ZERO, h(6, 7)), c2),
            (cp[9], handles(Vec2::ZERO, h(9, 10), h(9, 8), Vec2::ZERO), c3),
        ]))
    }

    /// A 1×1 mesh of a Gouraud-shaded triangle (a patch with two corners together); `None` if a
    /// point isn't finite.
    pub fn triangle(corners: [(Point, Color); 3]) -> Option<Self> {
        if !corners.iter().all(|(p, _)| p.is_finite()) {
            return None;
        }
        let [a, b, c] = corners.map(|(p, color)| (p, [Vec2::ZERO; 4], color));
        Some(Self::patch([a, b, c, c]))
    }

    /// A mesh over surface `s` (u, v ∈ [0,1]) with handles from its partial derivatives.
    pub fn from_surface(rows: u32, cols: u32, s: &dyn Fn(f64, f64) -> Point, color: &dyn Fn(f64, f64) -> Color) -> Self {
        let (us, vs) = (even_steps(cols), even_steps(rows));
        Self::from_grid(&us, &vs, s, &|r, c, _| (color(us[c], vs[r]), 1.0))
    }

    /// A mesh over surface `s` (u, v ∈ [0,1]) whose column lines sit at `us` and row lines at `vs`
    /// (ascending, at least two each; equal neighbours make a patch of no width, a sharp colour
    /// change), with handles from its partial derivatives. `color(row, col, point)` gives each
    /// point its colour and opacity.
    pub fn from_grid(us: &[f64], vs: &[f64], s: &dyn Fn(f64, f64) -> Point, color: &dyn Fn(usize, usize, Point) -> (Color, f32)) -> Self {
        let (rows, cols) = (vs.len().saturating_sub(1), us.len().saturating_sub(1));
        let eps = 1e-3;
        let mut points = Vec::with_capacity(us.len() * vs.len());
        for (r, &v) in vs.iter().enumerate() {
            for (c, &u) in us.iter().enumerate() {
                let p = s(u, v);
                let d_u = (s((u + eps).min(1.0), v) - s((u - eps).max(0.0), v)) / ((u + eps).min(1.0) - (u - eps).max(0.0));
                let d_v = (s(u, (v + eps).min(1.0)) - s(u, (v - eps).max(0.0))) / ((v + eps).min(1.0) - (v - eps).max(0.0));
                // A third of the way to the neighbouring line along the tangent; outward handles on
                // the border are unused, so they stay at the point.
                let h = |d: Vec2, from: f64, to: Option<&f64>| to.map_or(Vec2::ZERO, |to| d * ((to - from) / 3.0));
                let handles = [
                    h(d_u, u, us.get(c + 1)),
                    h(d_u, u, c.checked_sub(1).map(|i| &us[i])),
                    h(d_v, v, vs.get(r + 1)),
                    h(d_v, v, r.checked_sub(1).map(|i| &vs[i])),
                ];
                let (color, opacity) = color(r, c, p);
                points.push(MeshPoint { p, color, opacity, handles });
            }
        }
        Self { rows: rows as u32, cols: cols as u32, points }
    }

    /// Create Gradient Mesh for a path: the outline split at its corners, coloured per `appearance`.
    pub fn for_path(path: &PathData, rows: u32, cols: u32, base: Color, appearance: MeshAppearance, highlight: f64) -> Option<Self> {
        let mut m = Self::for_path_with(path, rows, cols, &|_| (base, 1.0))?;
        m.highlight(appearance, highlight);
        Some(m)
    }

    /// Create Gradient Mesh for a path: the outline split at its corners, each point taking the
    /// colour and opacity `color_at` gives at its position (a gradient fill's, say).
    pub fn for_path_with(path: &PathData, rows: u32, cols: u32, color_at: &dyn Fn(Point) -> (Color, f32)) -> Option<Self> {
        let coons = Coons::from_path(path)?;
        let (us, vs) = (even_steps(cols), even_steps(rows));
        let mut m = Self::from_grid(&us, &vs, &|u, v| coons.eval(u, v), &|_, _, p| color_at(p));
        m.fit_border(&coons, &us, &vs);
        Some(m)
    }

    /// Give each edge on the mesh's border the handles that make it follow the outline it stands
    /// for, in place of the handles [`Self::from_grid`] estimates from the surface's derivatives
    /// (#3187). Where two border edges meet along a side, their handles are made collinear, so the
    /// border stays smooth there; the four corners keep the handles their own edges were fitted with.
    fn fit_border(&mut self, coons: &Coons, us: &[f64], vs: &[f64]) {
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        if !self.is_valid() || us.len() != cols + 1 || vs.len() != rows + 1 {
            return;
        }
        let edges = [(0, &coons.top), (rows, &coons.bottom)]
            .map(|(r, side)| (r, side, true))
            .into_iter()
            .chain([(0, &coons.left), (cols, &coons.right)].map(|(c, side)| (c, side, false)));
        for (line, side, along_u) in edges {
            let (n, steps) = if along_u { (cols, us) } else { (rows, vs) };
            for k in 0..n {
                let (a, b, out, back) = if along_u {
                    (self.idx(line, k), self.idx(line, k + 1), H_RIGHT, H_LEFT)
                } else {
                    (self.idx(k, line), self.idx(k + 1, line), H_DOWN, H_UP)
                };
                if let Some((h1, h2)) = fit_edge(side, (steps[k], steps[k + 1]), (self.points[a].p, self.points[b].p)) {
                    self.points[a].handles[out] = h1;
                    self.points[b].handles[back] = h2;
                }
            }
            for k in 1..n {
                let (i, out, back) = if along_u { (self.idx(line, k), H_RIGHT, H_LEFT) } else { (self.idx(k, line), H_DOWN, H_UP) };
                let h = &mut self.points[i].handles;
                let (fwd, bwd) = (h[out], h[back]);
                let dir = (fwd.normalize() - bwd.normalize()).normalize();
                if fwd.hypot2() > 0.0 && bwd.hypot2() > 0.0 && dir.is_finite() {
                    h[out] = dir * fwd.hypot();
                    h[back] = -dir * bwd.hypot();
                }
            }
        }
    }

    /// Create Gradient Mesh → Appearance: lighten the points towards white, to the centre or to
    /// the edges, by at most `highlight` %. Each point lightens towards white in its own colour
    /// model (paper white, no ink, for CMYK), so a CMYK or grey mesh keeps its inks (#3232).
    pub fn highlight(&mut self, appearance: MeshAppearance, highlight: f64) {
        let h = (highlight / 100.0).clamp(0.0, 1.0) as f32;
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        for r in 0..=rows {
            for c in 0..=cols {
                let (u, v) = (c as f64 / cols as f64, r as f64 / rows as f64);
                let d = ((u - 0.5).abs().max((v - 0.5).abs()) * 2.0) as f32; // 0 centre … 1 edge
                let w = match appearance {
                    MeshAppearance::Flat => 0.0,
                    MeshAppearance::ToCenter => (1.0 - d) * h,
                    MeshAppearance::ToEdge => d * h,
                };
                if w > 0.0 {
                    let i = self.idx(r, c);
                    let color = self.points[i].color;
                    self.points[i].color = lerp_color(&color, &Color::WHITE.in_model(color.model()), w);
                }
            }
        }
    }

    /// Horizontal edge from (r, c) to (r, c+1).
    pub fn edge_u(&self, r: usize, c: usize) -> CubicBez {
        let (a, b) = (&self.points[self.idx(r, c)], &self.points[self.idx(r, c + 1)]);
        CubicBez::new(a.p, a.p + a.handles[H_RIGHT], b.p + b.handles[H_LEFT], b.p)
    }
    /// Vertical edge from (r, c) to (r+1, c).
    pub fn edge_v(&self, r: usize, c: usize) -> CubicBez {
        let (a, b) = (&self.points[self.idx(r, c)], &self.points[self.idx(r + 1, c)]);
        CubicBez::new(a.p, a.p + a.handles[H_DOWN], b.p + b.handles[H_UP], b.p)
    }

    /// Coons patch point of patch (r, c) at local (u, v).
    pub fn eval(&self, r: usize, c: usize, u: f64, v: f64) -> Point {
        let (t, b) = (self.edge_u(r, c).eval(u), self.edge_u(r + 1, c).eval(u));
        let (l, rr) = (self.edge_v(r, c).eval(v), self.edge_v(r, c + 1).eval(v));
        let p00 = self.points[self.idx(r, c)].p.to_vec2();
        let p10 = self.points[self.idx(r, c + 1)].p.to_vec2();
        let p01 = self.points[self.idx(r + 1, c)].p.to_vec2();
        let p11 = self.points[self.idx(r + 1, c + 1)].p.to_vec2();
        ((1.0 - v) * t.to_vec2() + v * b.to_vec2() + (1.0 - u) * l.to_vec2() + u * rr.to_vec2()
            - ((1.0 - u) * (1.0 - v) * p00 + u * (1.0 - v) * p10 + (1.0 - u) * v * p01 + u * v * p11))
            .to_point()
    }

    /// Bilinear colour and opacity of patch (r, c) at local (u, v). Corners all in CMYK (or all
    /// grey) mix in that space, as a PDF shading mixes in its own colour space: the inks come out
    /// exact, and a CMYK document separates no colour to draw them. Other corners mix in RGB.
    pub fn color_at(&self, r: usize, c: usize, u: f64, v: f64) -> (Color, f32) {
        let q =
            [&self.points[self.idx(r, c)], &self.points[self.idx(r, c + 1)], &self.points[self.idx(r + 1, c)], &self.points[self.idx(r + 1, c + 1)]];
        let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
        let op = q.iter().zip(w).map(|(p, w)| p.opacity * w as f32).sum();
        let cmyk: Option<Vec<[f32; 4]>> =
            q.iter().map(|p| if let Color::Cmyk { c, m, y, k } = p.color { Some([c, m, y, k]) } else { None }).collect();
        if let Some(inks) = cmyk {
            let mut mix = [0.0f32; 4];
            for (ink, w) in inks.iter().zip(w) {
                for (m, x) in mix.iter_mut().zip(ink) {
                    *m += x * w as f32;
                }
            }
            let [c, m, y, k] = mix;
            return (Color::Cmyk { c, m, y, k }, op);
        }
        let grey: Option<Vec<f32>> = q.iter().map(|p| if let Color::Gray { k } = p.color { Some(k) } else { None }).collect();
        if let Some(ks) = grey {
            return (Color::Gray { k: ks.iter().zip(w).map(|(k, w)| k * w as f32).sum() }, op);
        }
        let mut rgb = [0.0f32; 3];
        for (p, w) in q.iter().zip(w) {
            for (ch, x) in rgb.iter_mut().zip(p.color.to_rgb()) {
                *ch += x * w as f32;
            }
        }
        (Color::rgb(rgb[0], rgb[1], rgb[2]), op)
    }

    /// The quads per patch side [`Self::quads`] cuts at when `n` is asked: 1 to 64, and coarser
    /// where the patches at `n` would pass [`Self::MAX_QUADS`] (a quad per patch is the least).
    fn subdivision(&self, n: usize) -> usize {
        n.clamp(1, 64).min(((Self::MAX_QUADS / self.patches().max(1)) as f64).sqrt() as usize).max(1)
    }

    /// Tessellate into `n`×`n` quads per patch ([`Self::subdivision`]), each coloured at its centre
    /// and its corners.
    pub fn quads(&self, n: usize) -> Vec<MeshQuad> {
        if !self.is_valid() {
            return vec![];
        }
        let n = self.subdivision(n);
        let mut out = Vec::with_capacity(self.patches() * n * n);
        let mut grid = vec![Point::ZERO; (n + 1) * (n + 1)];
        let mut colors = vec![(Color::default(), 0.0f32); (n + 1) * (n + 1)];
        for r in 0..self.rows as usize {
            for c in 0..self.cols as usize {
                for j in 0..=n {
                    for i in 0..=n {
                        let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                        grid[j * (n + 1) + i] = self.eval(r, c, u, v);
                        colors[j * (n + 1) + i] = self.color_at(r, c, u, v);
                    }
                }
                for j in 0..n {
                    for i in 0..n {
                        let (color, opacity) = self.color_at(r, c, (i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
                        let g = |a: usize, b: usize| grid[b * (n + 1) + a];
                        let k = |a: usize, b: usize| colors[b * (n + 1) + a];
                        out.push(MeshQuad {
                            pts: [g(i, j), g(i + 1, j), g(i + 1, j + 1), g(i, j + 1)],
                            color,
                            opacity,
                            corners: [k(i, j), k(i + 1, j), k(i + 1, j + 1), k(i, j + 1)],
                        });
                    }
                }
            }
        }
        out
    }

    /// Flat-coloured pieces covering the mesh, for outputs that can't shade a mesh smoothly (SVG,
    /// Expand): each patch cut into `n`×`n` quads as by [`Self::quads`], then neighbouring quads
    /// joined into square blocks as large as keeps the colour across each block within about one
    /// 8-bit step. A patch of one colour is one piece; a patch whose colour changes a lot keeps
    /// all its quads. A piece's outline runs through its quads' outer corners, so it covers what
    /// they covered, and it takes the colour at its centre.
    pub fn pieces(&self, n: usize) -> Vec<MeshPiece> {
        let n = self.subdivision(n);
        let quads = self.quads(n);
        let cols = (self.cols as usize).max(1);
        let mut out = Vec::with_capacity(quads.len());
        for (k, patch) in quads.chunks_exact(n * n).enumerate() {
            let (r, c) = (k / cols, k % cols);
            let side = self.block_side(r, c, n, patch);
            if side == 1 {
                out.extend(patch.iter().map(|q| MeshPiece { pts: q.pts.to_vec(), color: q.color, opacity: q.opacity }));
                continue;
            }
            // Tessellation point (i, j) of the patch (0..=n each), from a quad it is a corner of.
            let at = |i: usize, j: usize| -> Option<Point> {
                let (qi, qj) = (i.min(n - 1), j.min(n - 1));
                let q = patch.get(qj * n + qi)?;
                q.pts
                    .get(match (i > qi, j > qj) {
                        (false, false) => 0,
                        (true, false) => 1,
                        (true, true) => 2,
                        (false, true) => 3,
                    })
                    .copied()
            };
            for j0 in (0..n).step_by(side) {
                for i0 in (0..n).step_by(side) {
                    let (i1, j1) = (i0 + side, j0 + side);
                    // Around the block, in the quads' order: along its top, down its right side,
                    // back along its bottom and up its left side.
                    let ring = (i0..i1)
                        .map(|i| (i, j0))
                        .chain((j0..j1).map(|j| (i1, j)))
                        .chain((i0 + 1..=i1).rev().map(|i| (i, j1)))
                        .chain((j0 + 1..=j1).rev().map(|j| (i0, j)));
                    let Some(pts) = ring.map(|(i, j)| at(i, j)).collect::<Option<Vec<Point>>>() else { continue };
                    let half = side as f64 / 2.0;
                    let (color, opacity) = self.color_at(r, c, (i0 as f64 + half) / n as f64, (j0 as f64 + half) / n as f64);
                    out.push(MeshPiece { pts, color, opacity });
                }
            }
        }
        out
    }

    /// The side, in quads, of the blocks [`Self::pieces`] joins patch (r, c) into, cut into
    /// `n`×`n` `quads`: the largest divisor of `n` that keeps the colour changing by at most one
    /// 8-bit step across a block. 1 (no joining) where the patch folds over itself, as the outline
    /// of a block across the fold wouldn't fill it as its quads do.
    fn block_side(&self, r: usize, c: usize, n: usize, quads: &[MeshQuad]) -> usize {
        let corner = |dr: usize, dc: usize| self.points.get(self.idx(r + dr, c + dc));
        let (Some(p00), Some(p01), Some(p10), Some(p11)) = (corner(0, 0), corner(0, 1), corner(1, 0), corner(1, 1)) else {
            return 1;
        };
        // Corners all in CMYK mix in their inks ([`Self::color_at`]); the RGB they show is checked too.
        let inks = [p00, p01, p10, p11].iter().all(|p| matches!(p.color, Color::Cmyk { .. }));
        let channels = |p: &MeshPoint| -> Vec<f32> {
            let mut v = p.color.to_rgb().to_vec();
            if inks && let Color::Cmyk { c, m, y, k } = p.color {
                v.extend([c, m, y, k]);
            }
            v.push(p.opacity);
            v
        };
        let [a, b, cc, d] = [p00, p01, p10, p11].map(channels);
        // A bilinear patch changes across a block of side h (0..1 of the patch) by at most
        // h × (du + dv) in each channel, du and dv its largest change along each parameter.
        let mut steps = 0.0f32;
        for k in 0..a.len() {
            let ch = |v: &[f32]| v.get(k).copied().unwrap_or_default();
            let du = (ch(&a) - ch(&b)).abs().max((ch(&cc) - ch(&d)).abs());
            let dv = (ch(&a) - ch(&cc)).abs().max((ch(&b) - ch(&d)).abs());
            steps = steps.max((du + dv) * 255.0);
        }
        if !steps.is_finite() {
            return 1;
        }
        let side = (1..=n).rev().find(|&s| n.is_multiple_of(s) && (n / s) as f32 >= steps).unwrap_or(1);
        if side > 1 {
            // Every quad turning the same way: the patch doesn't fold, so a block's outline
            // encloses just what its quads cover.
            let turn = |q: &MeshQuad| {
                let [a, b, c, d] = q.pts;
                (b - a).cross(c - a) + (c - a).cross(d - a)
            };
            let first = quads.first().map(turn).unwrap_or_default();
            if !quads.iter().all(|q| turn(q) * first > 0.0) {
                return 1;
            }
        }
        side
    }

    /// The mesh's outer boundary as a closed path.
    pub fn outline(&self) -> PathData {
        if !self.is_valid() {
            return PathData::default();
        }
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let mut segs: Vec<CubicBez> = vec![];
        for c in 0..cols {
            segs.push(self.edge_u(0, c));
        }
        for r in 0..rows {
            segs.push(self.edge_v(r, cols));
        }
        for c in (0..cols).rev() {
            let e = self.edge_u(rows, c);
            segs.push(CubicBez::new(e.p3, e.p2, e.p1, e.p0));
        }
        for r in (0..rows).rev() {
            let e = self.edge_v(r, 0);
            segs.push(CubicBez::new(e.p3, e.p2, e.p1, e.p0));
        }
        let mut anchors: Vec<Anchor> = Vec::with_capacity(segs.len());
        for (i, s) in segs.iter().enumerate() {
            let prev = &segs[(i + segs.len() - 1) % segs.len()];
            anchors.push(Anchor::with_handles(s.p0, prev.p2, s.p1));
        }
        PathData::single(SubPath::new(anchors, true))
    }

    /// Mesh lines (all row and column edges) as open paths, for outline view and editing overlays.
    pub fn lines(&self) -> PathData {
        if !self.is_valid() {
            return PathData::default();
        }
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let mut subs = vec![];
        let cub = |e: CubicBez| SubPath::new(vec![Anchor::with_handles(e.p0, e.p0, e.p1), Anchor::with_handles(e.p3, e.p2, e.p3)], false);
        for r in 0..=rows {
            for c in 0..cols {
                subs.push(cub(self.edge_u(r, c)));
            }
        }
        for c in 0..=cols {
            for r in 0..rows {
                subs.push(cub(self.edge_v(r, c)));
            }
        }
        PathData::new(subs)
    }

    /// Control bounds (points and handles).
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.points.iter().flat_map(|p| std::iter::once(p.p).chain(p.handles.iter().map(move |h| p.p + *h)));
        let first = it.next()?;
        Some(it.fold(Rect::from_points(first, first), |r, q| r.union_pt(q)))
    }

    pub fn transform(&mut self, a: Affine) {
        let lin = Affine::new({
            let c = a.as_coeffs();
            [c[0], c[1], c[2], c[3], 0.0, 0.0]
        });
        for p in &mut self.points {
            p.p = a * p.p;
            for h in &mut p.handles {
                *h = (lin * h.to_point()).to_vec2();
            }
        }
    }

    /// Map points (and handle ends) through a non-linear function.
    pub fn map(&mut self, f: &dyn Fn(Point) -> Point) {
        for p in &mut self.points {
            let q = f(p.p);
            for h in &mut p.handles {
                *h = f(p.p + *h) - q;
            }
            p.p = q;
        }
    }

    /// Which patch and local (u, v) contains `p` (sampled search; `None` outside the mesh).
    pub fn locate(&self, p: Point) -> Option<(usize, usize, f64, f64)> {
        self.locate_within(p, 0.0)
    }

    /// As [`Self::locate`], and for a point outside the mesh within `tol` of its outline, the
    /// patch and local (u, v) of the nearest point of the mesh's edge.
    pub fn locate_within(&self, p: Point, tol: f64) -> Option<(usize, usize, f64, f64)> {
        if !self.is_valid() {
            return None;
        }
        let bp = self.outline().to_bezpath();
        if bp.winding(p) == 0 {
            if tol.is_nan() || tol <= 0.0 {
                return None;
            }
            return self.nearest_edge(p).filter(|e| e.0 <= tol * tol).map(|(_, r, c, u, v)| (r, c, u, v));
        }
        const N: usize = 16;
        let mut best = (f64::INFINITY, 0, 0, 0.0, 0.0);
        for r in 0..self.rows as usize {
            for c in 0..self.cols as usize {
                for j in 0..=N {
                    for i in 0..=N {
                        let (u, v) = (i as f64 / N as f64, j as f64 / N as f64);
                        let d = self.eval(r, c, u, v).distance_squared(p);
                        if d < best.0 {
                            best = (d, r, c, u, v);
                        }
                    }
                }
            }
        }
        // Refine with a few Newton-free local searches.
        let (_, r, c, mut u, mut v) = best;
        let mut step = 0.5 / N as f64;
        for _ in 0..12 {
            let mut improved = false;
            for (du, dv) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
                let (nu, nv) = ((u + du).clamp(0.0, 1.0), (v + dv).clamp(0.0, 1.0));
                if self.eval(r, c, nu, nv).distance_squared(p) < self.eval(r, c, u, v).distance_squared(p) {
                    u = nu;
                    v = nv;
                    improved = true;
                }
            }
            if !improved {
                step /= 2.0;
            }
        }
        Some((r, c, u, v))
    }

    /// The point of the mesh's outer edge nearest `p`: its squared distance from `p`, and its
    /// boundary patch and local (u, v).
    fn nearest_edge(&self, p: Point) -> Option<(f64, usize, usize, f64, f64)> {
        use vectorcraft_geom::kurbo::ParamCurveNearest;
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let mut best: Option<(f64, usize, usize, f64, f64)> = None;
        let mut take = |e: CubicBez, at: (usize, usize, Option<f64>, Option<f64>)| {
            let n = e.nearest(p, 1e-9);
            if best.is_none_or(|b| n.distance_sq < b.0) {
                let (r, c, u, v) = at;
                best = Some((n.distance_sq, r, c, u.unwrap_or(n.t), v.unwrap_or(n.t)));
            }
        };
        for c in 0..cols {
            take(self.edge_u(0, c), (0, c, None, Some(0.0)));
            take(self.edge_u(rows, c), (rows - 1, c, None, Some(1.0)));
        }
        for r in 0..rows {
            take(self.edge_v(r, 0), (r, 0, Some(0.0), None));
            take(self.edge_v(r, cols), (r, cols - 1, Some(1.0), None));
        }
        best
    }

    /// Index of the mesh point within `tol` of `p`.
    pub fn point_near(&self, p: Point, tol: f64) -> Option<usize> {
        self.points
            .iter()
            .enumerate()
            .filter(|(_, q)| q.p.distance(p) <= tol)
            .min_by(|a, b| a.1.p.distance(p).total_cmp(&b.1.p.distance(p)))
            .map(|(i, _)| i)
    }

    fn surface_du(&self, r: usize, c: usize, u: f64, v: f64) -> Vec2 {
        let e = 1e-3;
        let (a, b) = ((u - e).max(0.0), (u + e).min(1.0));
        (self.eval(r, c, b, v) - self.eval(r, c, a, v)) / (b - a)
    }
    fn surface_dv(&self, r: usize, c: usize, u: f64, v: f64) -> Vec2 {
        let e = 1e-3;
        let (a, b) = ((v - e).max(0.0), (v + e).min(1.0));
        (self.eval(r, c, u, b) - self.eval(r, c, u, a)) / (b - a)
    }

    /// Insert a mesh row through patch row `r` at local `v`.
    pub fn insert_row(&mut self, r: usize, v: f64) {
        let cols = self.cols as usize;
        let v = v.clamp(0.01, 0.99);
        let mut row = Vec::with_capacity(cols + 1);
        // Split the vertical edges.
        let mut ups = vec![];
        for c in 0..=cols {
            let e = self.edge_v(r, c);
            let (l, rr) = (e.subsegment(0.0..v), e.subsegment(v..1.0));
            ups.push((l.p1 - l.p0, rr.p2 - rr.p3));
            let pc = c.min(cols - 1);
            let u = if c == cols { 1.0 } else { 0.0 };
            let (color, opacity) = self.color_at(r, pc, u, v);
            // Horizontal handles: isocurve derivatives on either side.
            let right = if c < cols { self.surface_du(r, c, 0.0, v) / (3.0) } else { Vec2::ZERO };
            let left = if c > 0 { -self.surface_du(r, c - 1, 1.0, v) / 3.0 } else { Vec2::ZERO };
            row.push(MeshPoint { p: l.p3, color, opacity, handles: [right, left, rr.p1 - rr.p0, l.p2 - l.p3] });
        }
        for (c, (down, up)) in ups.into_iter().enumerate() {
            let i0 = self.idx(r, c);
            let i1 = self.idx(r + 1, c);
            self.points[i0].handles[H_DOWN] = down;
            self.points[i1].handles[H_UP] = up;
        }
        let at = self.idx(r + 1, 0);
        self.points.splice(at..at, row);
        self.rows += 1;
    }

    /// Insert a mesh column through patch column `c` at local `u`.
    pub fn insert_col(&mut self, c: usize, u: f64) {
        let rows = self.rows as usize;
        let u = u.clamp(0.01, 0.99);
        let mut col = Vec::with_capacity(rows + 1);
        for r in 0..=rows {
            let e = self.edge_u(r, c);
            let (l, rr) = (e.subsegment(0.0..u), e.subsegment(u..1.0));
            let pr = r.min(rows - 1);
            let v = if r == rows { 1.0 } else { 0.0 };
            let (color, opacity) = self.color_at(pr, c, u, v);
            let down = if r < rows { self.surface_dv(r, c, u, 0.0) / 3.0 } else { Vec2::ZERO };
            let up = if r > 0 { -self.surface_dv(r - 1, c, u, 1.0) / 3.0 } else { Vec2::ZERO };
            col.push((MeshPoint { p: l.p3, color, opacity, handles: [rr.p1 - rr.p0, l.p2 - l.p3, down, up] }, l.p1 - l.p0, rr.p2 - rr.p3));
        }
        let cols = self.cols as usize;
        let mut pts = Vec::with_capacity((rows + 1) * (cols + 2));
        for (r, (mp, right, left)) in col.into_iter().enumerate() {
            for cc in 0..=cols {
                let mut q = self.points[r * (cols + 1) + cc].clone();
                if cc == c {
                    q.handles[H_RIGHT] = right;
                }
                if cc == c + 1 {
                    q.handles[H_LEFT] = left;
                }
                pts.push(q);
                if cc == c {
                    pts.push(mp.clone());
                }
            }
        }
        self.points = pts;
        self.cols += 1;
    }

    /// Mesh tool click: add a row and a column through `p` (inside the mesh, or outside within
    /// `tol` of its edge, through the nearest point of the edge). Returns the new point index.
    pub fn add_lines_at(&mut self, p: Point, tol: f64) -> Option<usize> {
        let (r, c, u, v) = self.locate_within(p, tol)?;
        let mut rr = r;
        let mut cc = c;
        let row_new = v > 0.02 && v < 0.98;
        let col_new = u > 0.02 && u < 0.98;
        if row_new {
            self.insert_row(r, v);
            rr = r + 1;
        } else if v >= 0.98 {
            rr = r + 1;
        }
        if col_new {
            self.insert_col(c, u);
            cc = c + 1;
        } else if u >= 0.98 {
            cc = c + 1;
        }
        Some(self.idx(rr, cc))
    }

    /// Mesh tool Alt-click: delete the mesh lines through point `index` (interior lines only).
    pub fn remove_point_lines(&mut self, index: usize) -> bool {
        // A mesh from a file may not have the points its rows and columns need: it is indexed below.
        if !self.is_valid() || index >= self.points.len() {
            return false;
        }
        let w = self.cols as usize + 1;
        let (r, c) = (index / w, index % w);
        let mut changed = false;
        if r > 0 && r < self.rows as usize {
            // Merge the edges above and below: keep the outer handles.
            for cc in 0..w {
                let above = self.idx(r - 1, cc);
                let below = self.idx(r + 1, cc);
                let mid = self.idx(r, cc);
                let (pa, pb, m) = (self.points[above].p, self.points[below].p, &self.points[mid]);
                let hm = (m.p, m.handles[H_UP], m.handles[H_DOWN]);
                let (down, up) = merged_handles((pa, self.points[above].handles[H_DOWN]), hm, (pb, self.points[below].handles[H_UP]));
                self.points[above].handles[H_DOWN] = down;
                self.points[below].handles[H_UP] = up;
            }
            self.points.drain(r * w..(r + 1) * w);
            self.rows -= 1;
            changed = true;
        }
        let w = self.cols as usize + 1;
        if c > 0 && c < self.cols as usize {
            let rows = self.rows as usize;
            for rr in 0..=rows {
                let (l, rt) = (rr * w + c - 1, rr * w + c + 1);
                let (pl, pr, m) = (self.points[l].p, self.points[rt].p, &self.points[rr * w + c]);
                let hm = (m.p, m.handles[H_LEFT], m.handles[H_RIGHT]);
                let (right, left) = merged_handles((pl, self.points[l].handles[H_RIGHT]), hm, (pr, self.points[rt].handles[H_LEFT]));
                self.points[l].handles[H_RIGHT] = right;
                self.points[rt].handles[H_LEFT] = left;
            }
            let mut k = 0;
            self.points.retain(|_| {
                let keep = k % w != c;
                k += 1;
                keep
            });
            self.cols -= 1;
            changed = true;
        }
        changed
    }
}

// =====================================================================================
// Envelopes
// =====================================================================================

/// Union of the geometric bounds of `nodes`.
pub fn nodes_bounds(nodes: &[Arc<Node>]) -> Option<Rect> {
    nodes.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric_bounds()))
}

/// A `(rows+1)×(cols+1)` grid over `r` (row-major).
pub fn grid_points(r: Rect, rows: u32, cols: u32) -> Vec<Point> {
    let mut v = Vec::with_capacity(((rows + 1) * (cols + 1)) as usize);
    for i in 0..=rows {
        for j in 0..=cols {
            v.push(Point::new(r.x0 + r.width() * j as f64 / cols.max(1) as f64, r.y0 + r.height() * i as f64 / rows.max(1) as f64));
        }
    }
    v
}

fn catmull(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Point map from the unit square onto a point grid (tensor-product Catmull-Rom; linear
/// extrapolation at the borders so an undistorted grid is the identity, also past the square).
pub(super) fn grid_eval(rows: usize, cols: usize, pts: &[Point], u: f64, v: f64) -> Point {
    let at = |r: i64, c: i64| -> Vec2 {
        let rr = r.clamp(0, rows as i64);
        let cc = c.clamp(0, cols as i64);
        let base = pts[rr as usize * (cols + 1) + cc as usize].to_vec2();
        // Linear extrapolation outside the grid.
        let er = r - rr;
        let ec = c - cc;
        let mut p = base;
        if er != 0 && rows >= 1 {
            let inner = (rr - er.signum()).clamp(0, rows as i64);
            p += (base - pts[inner as usize * (cols + 1) + cc as usize].to_vec2()) * er.abs() as f64;
        }
        if ec != 0 && cols >= 1 {
            let inner = (cc - ec.signum()).clamp(0, cols as i64);
            p += (base - pts[rr as usize * (cols + 1) + inner as usize].to_vec2()) * ec.abs() as f64;
        }
        p
    };
    // Past the grid's edges the outer patches go on (art can reach past the content's box).
    let (su, sv) = (u * cols as f64, v * rows as f64);
    let (ci, ri) = ((su.floor() as i64).clamp(0, cols as i64 - 1), (sv.floor() as i64).clamp(0, rows as i64 - 1));
    let (fu, fv) = (su - ci as f64, sv - ri as f64);
    let mut rowsv = [Vec2::ZERO; 4];
    for (k, rv) in rowsv.iter_mut().enumerate() {
        let r = ri - 1 + k as i64;
        *rv = catmull(at(r, ci - 1), at(r, ci), at(r, ci + 1), at(r, ci + 2), fu);
    }
    catmull(rowsv[0], rowsv[1], rowsv[2], rowsv[3], fv).to_point()
}

/// The surface (u, v ∈ [0,1]) an envelope of `kind` maps content onto, for content whose bounds in
/// the envelope's own `frame` (its axes → the document) are `src`: a warp bends along the frame's
/// axes; a mesh's points and a top object's path are in the document already.
pub(super) fn surface_of<'a>(kind: &'a EnvelopeKind, src: Rect, frame: Affine) -> Box<dyn Fn(f64, f64) -> Point + 'a> {
    let flat = move |u: f64, v: f64| frame * Point::new(src.x0 + u * src.width(), src.y0 + v * src.height());
    match kind {
        EnvelopeKind::Warp { style, bend, h: dh, v: dv, horizontal } => {
            let st = WarpStyle::from_id(style).unwrap_or(WarpStyle::Arc);
            let b = bend.clamp(-100.0, 100.0) / 100.0;
            let (dh, dv) = (dh.clamp(-100.0, 100.0) / 100.0, dv.clamp(-100.0, 100.0) / 100.0);
            let c = src.center();
            let (hw, hh) = (src.width() / 2.0, src.height() / 2.0);
            let horizontal = *horizontal;
            Box::new(move |u: f64, v: f64| {
                let (x, y) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                let (x2, y2) = if horizontal {
                    warp_point(st, b, dh, dv, hh / hw.max(1e-9), x, y)
                } else {
                    let (a, b2) = warp_point(st, b, dh, dv, hw / hh.max(1e-9), y, x);
                    (b2, a)
                };
                frame * Point::new(c.x + x2 * hw, c.y + y2 * hh)
            })
        }
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            if !handles.is_empty()
                && let Some(m) = envelope_grid(*rows, *cols, points, handles)
            {
                return Box::new(move |u: f64, v: f64| mesh_at(&m, u, v));
            }
            if *rows == 0 || *cols == 0 || grid_len(*rows, *cols) != Some(points.len()) {
                return Box::new(flat);
            }
            let (rows, cols) = (*rows as usize, *cols as usize);
            Box::new(move |u: f64, v: f64| grid_eval(rows, cols, points, u, v))
        }
        EnvelopeKind::TopObject { path } => match Coons::from_path(path) {
            Some(co) => Box::new(move |u: f64, v: f64| co.eval(u, v)),
            None => Box::new(flat),
        },
    }
}
