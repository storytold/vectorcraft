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
    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * (self.cols as usize + 1) + c
    }
    pub fn is_valid(&self) -> bool {
        self.rows >= 1 && self.cols >= 1 && grid_len(self.rows, self.cols) == Some(self.points.len())
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
        Some(Self::from_grid(&even_steps(cols), &even_steps(rows), &|u, v| coons.eval(u, v), &|_, _, p| color_at(p)))
    }

    /// Create Gradient Mesh → Appearance: lighten the points towards white, to the centre or to
    /// the edges, by at most `highlight` %.
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
                    self.points[i].color = lerp_color(&self.points[i].color, &Color::WHITE, w);
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

    /// Tessellate into `n`×`n` quads per patch, each coloured at its centre and its corners.
    pub fn quads(&self, n: usize) -> Vec<MeshQuad> {
        if !self.is_valid() {
            return vec![];
        }
        let n = n.clamp(1, 64);
        let mut out = Vec::with_capacity(self.rows as usize * self.cols as usize * n * n);
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
        if !self.is_valid() {
            return None;
        }
        let bp = self.outline().to_bezpath();
        if bp.winding(p) == 0 {
            return None;
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

    /// Mesh tool click: add a row and a column through `p` (inside the mesh). Returns the new point index.
    pub fn add_lines_at(&mut self, p: Point) -> Option<usize> {
        let (r, c, u, v) = self.locate(p)?;
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
                let (pa, pm, pb) = (self.points[above].p, self.points[mid].p, self.points[below].p);
                let _ = pm;
                self.points[above].handles[H_DOWN] = (pb - pa) / 3.0;
                self.points[below].handles[H_UP] = (pa - pb) / 3.0;
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
                let (pl, pr) = (self.points[l].p, self.points[rt].p);
                self.points[l].handles[H_RIGHT] = (pr - pl) / 3.0;
                self.points[rt].handles[H_LEFT] = (pl - pr) / 3.0;
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
