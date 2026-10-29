//! Freeform gradients: colour points placed anywhere on an object (and smooth lines through them),
//! blended across it.
//!
//! Each point paints a disc of pure colour (its spread, a fraction of half the larger side of the
//! painted box) and blends into its neighbours beyond it by inverse-distance weighting on the
//! distance from the disc's edge. A line is a Catmull-Rom curve through its points: every spot
//! on it carries the colour, opacity and spread interpolated between the two points it lies
//! between, and acts as a source along its length (each short piece of it weighted by its share
//! of the line). Where discs overlap the deeper one wins, so the field stays continuous.

use kurbo::{Affine, BezPath, CubicBez, ParamCurve, ParamCurveNearest, Point, Rect};
use serde::{Deserialize, Serialize};

use crate::cms::{self, Model};
use crate::{Color, Gradient, GradientStop};

/// The spread a new point gets.
pub const DEFAULT_SPREAD: f32 = 0.0;
/// Line segments are tessellated into this many pieces for sampling.
const LINE_STEPS: usize = 16;

fn one() -> f32 {
    1.0
}
fn default_spread() -> f32 {
    DEFAULT_SPREAD
}

/// One colour point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FreeformPoint {
    /// Position in the paint's space (document coordinates; text space for type runs).
    pub at: Point,
    pub color: Color,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Radius (0..=1) of the disc of pure colour around the point, as a fraction of
    /// [`spread_scale`].
    #[serde(default = "default_spread")]
    pub spread: f32,
}

impl FreeformPoint {
    pub fn new(at: Point, color: Color) -> Self {
        Self { at, color, opacity: 1.0, spread: DEFAULT_SPREAD }
    }
}

/// How the Gradient tool adds points: free points, or points joined into lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FreeformMode {
    #[default]
    Points,
    Lines,
}

impl FreeformMode {
    /// Parse a mode name (`points`, `lines`; any case).
    pub fn parse(s: &str) -> Option<Self> {
        [FreeformMode::Points, FreeformMode::Lines].into_iter().find(|m| m.label().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            FreeformMode::Points => "Points",
            FreeformMode::Lines => "Lines",
        }
    }
    fn is_points(&self) -> bool {
        *self == FreeformMode::Points
    }
}

/// A freeform gradient: its colour points and the lines through them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Freeform {
    pub points: Vec<FreeformPoint>,
    /// Each line is a smooth curve through `points` at these indices, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<Vec<usize>>,
    /// Draw: how the Gradient tool adds points.
    #[serde(default, skip_serializing_if = "FreeformMode::is_points")]
    pub mode: FreeformMode,
}

/// The length spreads are fractions of: half the larger side of the painted box.
pub fn spread_scale(b: Rect) -> f64 {
    b.width().abs().max(b.height().abs()) / 2.0
}

/// The box a freeform gradient painted over `bounds` is placed on: `bounds` with positive sides, a
/// side without length getting a sliver (so a grid over it maps back invertibly). `None` for an
/// empty or non-finite box.
pub fn painted_box(bounds: Rect) -> Option<Rect> {
    let b = bounds.abs();
    let side = b.width().max(b.height());
    (side > 1e-9 && side.is_finite()).then(|| Rect::from_center_size(b.center(), (b.width().max(side * 1e-3), b.height().max(side * 1e-3))))
}

/// Device pixels per cell of the grid a freeform gradient is sampled on.
const CELL_PX: f64 = 4.0;
/// Fewest and most cells along the painted box's longer side.
const MIN_CELLS: f64 = 8.0;
const MAX_CELLS: f64 = 256.0;

/// The grid (columns, rows) a freeform gradient on box `b` (see [`painted_box`]) is sampled on
/// when the box's longer side spans `device` pixels: a cell per few pixels, in power-of-two steps
/// (so zooming reuses grids), 8 to 256 cells along the longer side.
pub fn grid_size(b: Rect, device: f64) -> (u16, u16) {
    let side = b.width().max(b.height());
    let cells = 2f64.powf((device / CELL_PX).max(1.0).log2().ceil()).clamp(MIN_CELLS, MAX_CELLS);
    let along = |len: f64| (cells * len / side).ceil().clamp(2.0, MAX_CELLS) as u16;
    (along(b.width()), along(b.height()))
}

/// Where a point lies nearest on a line (see [`Freeform::nearest_on_lines`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineHit {
    pub line: usize,
    /// The segment from the line's point `segment` to point `segment + 1`.
    pub segment: usize,
    /// Curve parameter along the segment, 0..=1.
    pub t: f64,
    pub point: Point,
    pub distance: f64,
}

impl Freeform {
    /// The points the first application places on an object with box `b`: a 2 × n grid (n = 2–4
    /// along the longer side), each moved to the nearest spot `inside` the shape when its grid
    /// position isn't. They take colours along `g`, in a loop around the grid.
    pub fn auto(b: Rect, g: &Gradient, inside: &dyn Fn(Point) -> bool) -> Self {
        let (w, h) = (b.width().max(1e-9), b.height().max(1e-9));
        let long = ((2.0 * w.max(h) / w.min(h)).round() as usize).clamp(2, 4);
        let (cols, rows) = if w >= h { (long, 2) } else { (2, long) };
        let cell = |i: usize, n: usize| (i as f64 + 0.5) / n as f64;
        // Row by row, every other row backwards: neighbours in the list are neighbours on the art.
        let targets: Vec<Point> = (0..rows)
            .flat_map(|j| (0..cols).map(move |k| if j % 2 == 0 { (k, j) } else { (cols - 1 - k, j) }))
            .map(|(i, j)| Point::new(b.x0 + cell(i, cols) * w, b.y0 + cell(j, rows) * h))
            .collect();
        const GRID: usize = 24;
        let candidates: Vec<Point> =
            (0..GRID * GRID).map(|k| Point::new(b.x0 + cell(k % GRID, GRID) * w, b.y0 + cell(k / GRID, GRID) * h)).filter(|p| inside(*p)).collect();
        let apart = w.min(h) * 0.15;
        let mut placed: Vec<Point> = Vec::with_capacity(targets.len());
        for t in &targets {
            let free = |c: &&Point| placed.iter().all(|q| q.distance(**c) >= apart);
            let at = if inside(*t) {
                *t
            } else {
                candidates.iter().filter(free).min_by(|a, b| a.distance_squared(*t).total_cmp(&b.distance_squared(*t))).copied().unwrap_or(*t)
            };
            placed.push(at);
        }
        let mut f = Self { points: placed.into_iter().map(|at| FreeformPoint::new(at, Color::BLACK)).collect(), ..Default::default() };
        f.recolor(g);
        f
    }

    /// Color the points along `g`, first to last, mixing neighboring stops in the model they share
    /// ([`Color::lerp_keep_model`]).
    pub fn recolor(&mut self, g: &Gradient) {
        let n = self.points.len();
        let at = (0..n).map(|k| k as f32 / (n - 1).max(1) as f32);
        for (p, (color, opacity)) in self.points.iter_mut().zip(g.sample_increasing_with(at, Color::lerp_keep_model)) {
            (p.color, p.opacity) = (color, opacity);
        }
    }

    /// Gradient stops standing for the points (their colours evenly spaced, at least two): what
    /// swatch chips, exports without freeform shading and re-seeding on other art use.
    pub fn stops(&self) -> Vec<GradientStop> {
        let n = self.points.len();
        let stop = |offset: f32, p: &FreeformPoint| GradientStop { opacity: p.opacity, ..GradientStop::new(offset, p.color) };
        match n {
            0 => Gradient::default().stops,
            1 => vec![stop(0.0, &self.points[0]), stop(1.0, &self.points[0])],
            _ => self.points.iter().enumerate().map(|(i, p)| stop(i as f32 / (n - 1) as f32, p)).collect(),
        }
    }

    /// Map every point through `f`.
    pub fn map_points(&mut self, f: impl Fn(Point) -> Point) {
        for p in &mut self.points {
            p.at = f(p.at);
        }
    }

    /// Map every point through `a`.
    pub fn transform(&mut self, a: Affine) {
        self.map_points(|p| a * p);
    }

    /// The cubic segments of line `line` (a Catmull-Rom curve through its points).
    pub fn segments(&self, line: usize) -> Vec<CubicBez> {
        let Some(ix) = self.lines.get(line) else { return vec![] };
        // A line naming a missing point (a damaged file) draws nothing.
        let Some(pts) = ix.iter().map(|i| self.points.get(*i).map(|p| p.at)).collect::<Option<Vec<Point>>>() else { return vec![] };
        let n = pts.len();
        (0..n.saturating_sub(1))
            .map(|i| {
                let (p0, p1, p2, p3) = (pts[i.saturating_sub(1)], pts[i], pts[i + 1], pts[(i + 2).min(n - 1)]);
                CubicBez::new(p1, p1 + (p2 - p0) / 6.0, p2 - (p3 - p1) / 6.0, p2)
            })
            .collect()
    }

    /// Line `line` as a path (for drawing it).
    pub fn line_path(&self, line: usize) -> BezPath {
        let mut bp = BezPath::new();
        for (i, c) in self.segments(line).into_iter().enumerate() {
            if i == 0 {
                bp.move_to(c.p0);
            }
            bp.curve_to(c.p1, c.p2, c.p3);
        }
        bp
    }

    /// The spot on a line nearest `p`.
    pub fn nearest_on_lines(&self, p: Point) -> Option<LineHit> {
        let mut best: Option<LineHit> = None;
        for line in 0..self.lines.len() {
            for (segment, c) in self.segments(line).into_iter().enumerate() {
                let n = c.nearest(p, 1e-6);
                let distance = n.distance_sq.sqrt();
                if best.is_none_or(|b| distance < b.distance) {
                    best = Some(LineHit { line, segment, t: n.t, point: c.eval(n.t), distance });
                }
            }
        }
        best
    }

    /// The color and opacity at `p` for a painted box whose [`spread_scale`] is `scale`, as
    /// [`Self::model_field`] mixes them.
    pub fn sample(&self, p: Point, scale: f64) -> (Color, f32) {
        self.model_field(scale).sample(p)
    }

    /// The colour field, with colours converted and lines tessellated once (for sampling many
    /// spots).
    pub fn field(&self, scale: f64) -> Field {
        self.field_with(scale, &Color::to_rgb)
    }

    /// [`Self::field`] with colours turned into the three blended values by `rgb` (CMYK
    /// documents blend ink planes, see [`crate::blend::cmyk_planes`]).
    pub fn field_with(&self, scale: f64, rgb: &dyn Fn(&Color) -> [f32; 3]) -> Field {
        let src = |p: &FreeformPoint| Source { at: p.at, rgb: rgb(&p.color), opacity: p.opacity, radius: p.spread.clamp(0.0, 1.0) as f64 * scale };
        let points: Vec<Source> = self.points.iter().map(src).collect();
        let mut pieces = vec![];
        for (l, ix) in self.lines.iter().enumerate() {
            // The line tessellated into a polyline of sources.
            let mut v: Vec<Source> = vec![];
            for (s, c) in self.segments(l).into_iter().enumerate() {
                let (Some(a), Some(b)) = (ix.get(s).and_then(|i| points.get(*i)), ix.get(s + 1).and_then(|i| points.get(*i))) else { continue };
                for k in (if s == 0 { 0 } else { 1 })..=LINE_STEPS {
                    let t = k as f64 / LINE_STEPS as f64;
                    v.push(Source { at: c.eval(t), ..a.lerp(b, t as f32) });
                }
            }
            let len: f64 = v.windows(2).map(|w| w[0].at.distance(w[1].at)).sum();
            let n = v.len().saturating_sub(1).max(1) as f64;
            pieces.extend(v.windows(2).map(|w| Piece { a: w[0], b: w[1], share: if len > 0.0 { w[0].at.distance(w[1].at) / len } else { 1.0 / n } }));
        }
        Field { points, pieces, eps: 1e-9 * (scale.abs() + 1.0) }
    }

    /// [`Self::field`] mixing the colors in the model every point shares instead of display RGB:
    /// the C, M, Y and K inks (as a CMYK document drawn by ink planes mixes them), the Gray ink,
    /// or the RGB or Lab components. Points in different models mix in display RGB, as
    /// [`Self::field`] mixes them, and the field gives that color in the working RGB space.
    pub fn model_field(&self, scale: f64) -> ModelField {
        let Some(model) = self.shared_model() else { return ModelField { model: None, first: self.field(scale), k: None } };
        let parts = |c: &Color| match *c {
            Color::Rgb { r, g, b } => [r, g, b, 0.0],
            Color::Cmyk { c, m, y, k } => [c, m, y, k],
            Color::Gray { k } => [k, 0.0, 0.0, 0.0],
            Color::Lab { l, a, b } => [l, a, b, 0.0],
        };
        let first = self.field_with(scale, &|c| {
            let [x, y, z, _] = parts(c);
            [x, y, z]
        });
        let k = (model == Model::Cmyk).then(|| self.field_with(scale, &|c| [parts(c)[3], 0.0, 0.0]));
        ModelField { model: Some(model), first, k }
    }

    /// The color model every point is in (`None` without points or when their models differ).
    pub fn shared_model(&self) -> Option<Model> {
        let m = self.points.first()?.color.model();
        self.points.iter().all(|p| p.color.model() == m).then_some(m)
    }

    /// Add a point; returns its index.
    pub fn add_point(&mut self, p: FreeformPoint) -> usize {
        self.points.push(p);
        self.points.len() - 1
    }

    /// Join point `to` to point `from`: extend the line that ends (or starts) at `from`, else start
    /// a line from it. Returns the line's index (None when an index is out of range or equal).
    pub fn connect(&mut self, from: usize, to: usize) -> Option<usize> {
        if from == to || from >= self.points.len() || to >= self.points.len() {
            return None;
        }
        if let Some(l) = self.lines.iter().position(|l| l.last() == Some(&from)) {
            self.lines[l].push(to);
            return Some(l);
        }
        if let Some(l) = self.lines.iter().position(|l| l.first() == Some(&from)) {
            self.lines[l].insert(0, to);
            return Some(l);
        }
        self.lines.push(vec![from, to]);
        Some(self.lines.len() - 1)
    }

    /// Add a line through `points` (at least two, each a valid index, none twice in a row).
    pub fn add_line(&mut self, points: Vec<usize>) -> Result<usize, String> {
        if points.len() < 2 {
            return Err("a line needs at least two points".into());
        }
        if let Some(i) = points.iter().find(|i| **i >= self.points.len()) {
            return Err(format!("no point {i} (the gradient has {})", self.points.len()));
        }
        if points.windows(2).any(|w| w[0] == w[1]) {
            return Err("a line can't run from a point to itself".into());
        }
        self.lines.push(points);
        Ok(self.lines.len() - 1)
    }

    /// Remove point `i` (never the last one). Lines through it close up around it; lines left
    /// with fewer than two points go.
    pub fn remove_point(&mut self, i: usize) -> bool {
        if i >= self.points.len() || self.points.len() <= 1 {
            return false;
        }
        self.points.remove(i);
        for l in &mut self.lines {
            l.retain(|j| *j != i);
            for j in l.iter_mut() {
                if *j > i {
                    *j -= 1;
                }
            }
            l.dedup();
        }
        self.lines.retain(|l| l.len() >= 2);
        true
    }

    /// Insert a point on segment `segment` of line `line` at curve parameter `t`, with the color
    /// (in the model the ends share, [`Color::lerp_keep_model`]), opacity and spread between the
    /// segment's ends. Returns the new point's index.
    pub fn split_line(&mut self, line: usize, segment: usize, t: f64) -> Option<usize> {
        let c = *self.segments(line).get(segment)?;
        let l = &self.lines[line];
        let (a, b) = (self.points[l[segment]], self.points[l[segment + 1]]);
        let t = t.clamp(0.0, 1.0);
        let u = t as f32;
        let p = FreeformPoint {
            at: c.eval(t),
            color: a.color.lerp_keep_model(&b.color, u),
            opacity: a.opacity + (b.opacity - a.opacity) * u,
            spread: a.spread + (b.spread - a.spread) * u,
        };
        let i = self.add_point(p);
        self.lines[line].insert(segment + 1, i);
        Some(i)
    }
}

/// One colour source of a [`Field`].
#[derive(Clone, Copy, Debug)]
struct Source {
    at: Point,
    rgb: [f32; 3],
    opacity: f32,
    radius: f64,
}

impl Source {
    fn lerp(&self, o: &Source, t: f32) -> Source {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Source {
            at: self.at.lerp(o.at, t as f64),
            rgb: [l(self.rgb[0], o.rgb[0]), l(self.rgb[1], o.rgb[1]), l(self.rgb[2], o.rgb[2])],
            opacity: l(self.opacity, o.opacity),
            radius: self.radius + (o.radius - self.radius) * t as f64,
        }
    }
}

/// A freeform gradient ready to sample ([`Freeform::field`]).
#[derive(Clone, Debug)]
pub struct Field {
    points: Vec<Source>,
    /// The lines tessellated into short pieces.
    pieces: Vec<Piece>,
    eps: f64,
}

/// A piece of a tessellated line: a source at its spot nearest the sample, weighted by `share`
/// (its fraction of the line's length), so a whole line weighs as much as a point from afar and
/// the field stays continuous where the nearest spot on the line jumps.
#[derive(Clone, Copy, Debug)]
struct Piece {
    a: Source,
    b: Source,
    share: f64,
}

impl Piece {
    /// The distance from `p` and the source at the nearest spot.
    fn nearest(&self, p: Point) -> (f64, Source) {
        let v = self.b.at - self.a.at;
        let l2 = v.hypot2();
        let t = if l2 > 0.0 { ((p - self.a.at).dot(v) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let s = self.a.lerp(&self.b, t as f32);
        (s.at.distance(p), s)
    }
}

impl Field {
    /// Samples at the centres of a `cols` × `rows` grid of cells over `b`, row by row: RGB and
    /// opacity (as [`Self::sample`]).
    pub fn grid(&self, b: Rect, cols: u16, rows: u16) -> impl Iterator<Item = ([f32; 3], f32)> + '_ {
        let (cw, ch) = (b.width() / cols as f64, b.height() / rows as f64);
        (0..rows as usize * cols as usize).map(move |k| {
            let (i, j) = (k % cols as usize, k / cols as usize);
            self.sample(Point::new(b.x0 + (i as f64 + 0.5) * cw, b.y0 + (j as f64 + 0.5) * ch))
        })
    }

    /// Display RGB and opacity at `p`.
    pub fn sample(&self, p: Point) -> ([f32; 3], f32) {
        // (weight sum, weighted rgba) inside discs, and blending by distance outside them.
        let mut disc = (0.0f64, [0.0f64; 4]);
        let mut far = (0.0f64, [0.0f64; 4]);
        let mut add = |d: f64, s: &Source, share: f64| {
            let (acc, w) = if d < s.radius || d <= self.eps {
                // The deeper inside, the stronger: weights fade to nothing at the disc's edge.
                (&mut disc, (s.radius - d + self.eps) * share)
            } else {
                let e = d - s.radius;
                (&mut far, share / (e * e))
            };
            acc.0 += w;
            for (k, v) in s.rgb.iter().chain(std::iter::once(&s.opacity)).enumerate() {
                acc.1[k] += w * *v as f64;
            }
        };
        for s in &self.points {
            add(s.at.distance(p), s, 1.0);
        }
        for piece in &self.pieces {
            let (d, s) = piece.nearest(p);
            add(d, &s, piece.share);
        }
        let (w, acc) = if disc.0 > 0.0 { disc } else { far };
        if !(w > 0.0 && w.is_finite()) {
            return ([0.0; 3], 0.0);
        }
        let c = |k: usize| (acc[k] / w) as f32;
        ([c(0), c(1), c(2)], c(3))
    }
}

/// A freeform gradient ready to sample in its points' color model ([`Freeform::model_field`]).
#[derive(Clone, Debug)]
pub struct ModelField {
    /// The model the points share (`None` when their models differ).
    model: Option<Model>,
    /// The first three components (C, M and Y; R, G and B; L, a and b; display RGB when the
    /// models differ), or the Gray ink as the first of the three values.
    first: Field,
    /// The K ink of a CMYK field.
    k: Option<Field>,
}

impl ModelField {
    /// The model the points share (`None` when their models differ).
    pub fn model(&self) -> Option<Model> {
        self.model
    }

    /// The color and opacity at `p`: in the points' shared model, else in the working RGB space.
    pub fn sample(&self, p: Point) -> (Color, f32) {
        let ([x, y, z], a) = self.first.sample(p);
        let c = match self.model {
            None => {
                let [r, g, b] = cms::active().srgb_to_rgb([x, y, z]);
                Color::rgb(r, g, b)
            }
            Some(Model::Rgb) => Color::rgb(x, y, z),
            Some(Model::Cmyk) => Color::cmyk(x, y, z, self.k.as_ref().map_or(0.0, |k| k.sample(p).0[0])),
            Some(Model::Gray) => Color::gray(x),
            Some(Model::Lab) => Color::lab(x, y, z),
        };
        (c, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> Color {
        Color::rgb(1.0, 0.0, 0.0)
    }
    fn blue() -> Color {
        Color::rgb(0.0, 0.0, 1.0)
    }

    fn two(spread: f32) -> Freeform {
        Freeform {
            points: vec![
                FreeformPoint { spread, ..FreeformPoint::new(Point::new(0.0, 0.0), red()) },
                FreeformPoint { spread, opacity: 0.5, ..FreeformPoint::new(Point::new(100.0, 0.0), blue()) },
            ],
            ..Default::default()
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn the_sample_at_a_point_is_its_colour() {
        let f = two(0.0);
        let (c, o) = f.sample(Point::new(0.0, 0.0), 50.0);
        assert!(close(c.to_rgb(), [1.0, 0.0, 0.0]) && (o - 1.0).abs() < 1e-6, "{c:?} {o}");
        let (c, o) = f.sample(Point::new(100.0, 0.0), 50.0);
        assert!(close(c.to_rgb(), [0.0, 0.0, 1.0]) && (o - 0.5).abs() < 1e-6);
        // Half way between equal points: an even mix.
        let (c, o) = f.sample(Point::new(50.0, 0.0), 50.0);
        assert!(close(c.to_rgb(), [0.5, 0.0, 0.5]) && (o - 0.75).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn the_sample_mixes_in_the_model_the_points_share() {
        let halfway = |a: Color, b: Color| {
            let mut f = two(0.0);
            (f.points[0].color, f.points[1].color) = (a, b);
            f.sample(Point::new(50.0, 0.0), 50.0).0
        };
        let near = |a: Color, b: Color| match (a, b) {
            (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => {
                [c - c2, m - m2, y - y2, k - k2].iter().all(|d| d.abs() < 1e-5)
            }
            (Color::Gray { k }, Color::Gray { k: k2 }) => (k - k2).abs() < 1e-5,
            (Color::Lab { l, a, b }, Color::Lab { l: l2, a: a2, b: b2 }) => [l - l2, a - a2, b - b2].iter().all(|d| d.abs() < 1e-4),
            _ => false,
        };
        let c = halfway(Color::cmyk(1.0, 0.0, 0.0, 0.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
        assert!(near(c, Color::cmyk(0.5, 0.0, 0.0, 0.5)), "{c:?}");
        let c = halfway(Color::gray(0.2), Color::gray(0.6));
        assert!(near(c, Color::gray(0.4)), "{c:?}");
        let c = halfway(Color::lab(50.0, 10.0, -20.0), Color::lab(70.0, 30.0, 0.0));
        assert!(near(c, Color::lab(60.0, 20.0, -10.0)), "{c:?}");
        // Points in different models mix in RGB.
        assert!(matches!(halfway(Color::cmyk(1.0, 0.0, 0.0, 0.0), red()), Color::Rgb { .. }));
    }

    #[test]
    fn spread_keeps_the_colour_pure_inside_and_falls_off_outside() {
        // Scale 100: a 0.2 spread is a 20 pt disc.
        let mut f = two(0.0);
        f.points[0].spread = 0.2;
        let at = |x: f64| f.sample(Point::new(x, 0.0), 100.0).0.to_rgb()[0];
        assert!((at(15.0) - 1.0).abs() < 1e-6, "inside the disc: pure red");
        assert!(at(30.0) < 1.0 && at(30.0) > at(50.0) && at(50.0) > at(70.0), "falls off: {} {} {}", at(30.0), at(50.0), at(70.0));
        // Without the spread the same spot is bluer.
        let plain = two(0.0).sample(Point::new(30.0, 0.0), 100.0).0.to_rgb()[0];
        assert!(at(30.0) > plain);
        // Overlapping discs blend continuously (no jump at the edge of the smaller one).
        f.points[1].spread = 0.9;
        let a = f.sample(Point::new(10.01, 0.0), 100.0).0.to_rgb()[0];
        let b = f.sample(Point::new(9.99, 0.0), 100.0).0.to_rgb()[0];
        assert!((a - b).abs() < 0.01, "{a} vs {b}");
    }

    #[test]
    fn lines_carry_colour_between_their_points() {
        let mut f = two(0.0);
        f.points.push(FreeformPoint::new(Point::new(50.0, 100.0), Color::rgb(0.0, 1.0, 0.0)));
        let off_line = f.sample(Point::new(50.0, 5.0), 100.0).0.to_rgb();
        f.add_line(vec![0, 1]).unwrap();
        let on_line = f.sample(Point::new(50.0, 5.0), 100.0).0.to_rgb();
        // Near the line's middle the red-blue mix dominates the green point.
        assert!(on_line[1] < off_line[1], "{on_line:?} vs {off_line:?}");
        let hit = f.nearest_on_lines(Point::new(50.0, 7.0)).unwrap();
        assert_eq!((hit.line, hit.segment), (0, 0));
        assert!((hit.t - 0.5).abs() < 1e-3 && (hit.distance - 7.0).abs() < 1e-3, "{hit:?}");
    }

    #[test]
    fn the_field_is_continuous_around_lines() {
        // A bent line: where the nearest spot on it jumps the colour must not.
        let mut f = two(0.0);
        f.points.push(FreeformPoint::new(Point::new(50.0, 100.0), Color::rgb(0.0, 1.0, 0.0)));
        f.points.push(FreeformPoint { spread: 0.1, ..FreeformPoint::new(Point::new(-50.0, 80.0), Color::WHITE) });
        f.add_line(vec![0, 1, 2]).unwrap();
        let field = f.field(100.0);
        let at = |x: f64, y: f64| field.sample(Point::new(x, y)).0;
        for i in 0..80 {
            for j in 0..80 {
                let (x, y) = (-60.0 + i as f64 * 2.5, -60.0 + j as f64 * 2.5);
                let (a, b, c) = (at(x, y), at(x + 0.5, y), at(x, y + 0.5));
                let step = (0..3).map(|k| (a[k] - b[k]).abs().max((a[k] - c[k]).abs())).fold(0.0, f32::max);
                assert!(step < 0.06, "jump of {step} at ({x}, {y})");
            }
        }
    }

    #[test]
    fn auto_places_four_or_more_points_inside_the_shape() {
        let g = Gradient::default();
        let b = Rect::new(0.0, 0.0, 200.0, 100.0);
        // A triangle with its apex at the top middle.
        let inside = |p: Point| p.y >= 0.0 && p.y <= 100.0 && (p.x - 100.0).abs() <= p.y;
        let f = Freeform::auto(b, &g, &inside);
        assert!(f.points.len() >= 4, "{}", f.points.len());
        assert!(f.points.iter().all(|p| inside(p.at)), "{:?}", f.points);
        // Colours run from the gradient's start to its end.
        assert_eq!((f.points[0].color.to_hex(), f.points.last().unwrap().color.to_hex()), ("#ffffff".into(), "#000000".into()));
        // A long box gets more points along its length.
        assert!(Freeform::auto(Rect::new(0.0, 0.0, 400.0, 100.0), &g, &|_| true).points.len() > 4);
    }

    #[test]
    fn editing_keeps_lines_consistent() {
        let mut f = two(0.0);
        let c = f.add_point(FreeformPoint::new(Point::new(200.0, 0.0), red()));
        assert_eq!(f.connect(0, 1), Some(0));
        assert_eq!(f.connect(1, c), Some(0), "extends the line ending at 1");
        assert_eq!(f.lines, vec![vec![0, 1, 2]]);
        let mid = f.split_line(0, 0, 0.5).unwrap();
        assert_eq!((mid, &f.lines[0]), (3, &vec![0, 3, 1, 2]));
        let p = f.points[3];
        assert!(p.at.y.abs() < 1e-9 && p.at.x > 0.0 && p.at.x < 100.0 && (p.opacity - 0.75).abs() < 1e-6, "on the curve, half way: {p:?}");
        assert!(f.remove_point(1));
        assert_eq!(f.lines, vec![vec![0, 2, 1]]);
        assert!(f.add_line(vec![0, 9]).is_err() && f.add_line(vec![0]).is_err());
        f.remove_point(0);
        f.remove_point(0);
        assert!(f.lines.is_empty() && f.points.len() == 1);
        assert!(!f.remove_point(0), "the last point stays");
        assert_eq!(f.stops().len(), 2);
    }

    #[test]
    fn serde_defaults_and_mode() {
        let f: Freeform = serde_json::from_str(r#"{"points":[{"at":{"x":1.0,"y":2.0},"color":{"model":"rgb","r":1.0,"g":0.0,"b":0.0}}]}"#).unwrap();
        assert_eq!((f.points[0].opacity, f.points[0].spread, f.mode), (1.0, DEFAULT_SPREAD, FreeformMode::Points));
        let s = serde_json::to_string(&f).unwrap();
        assert!(!s.contains("\"lines\"") && !s.contains("\"mode\""), "{s}");
        assert_eq!(FreeformMode::parse("LINES"), Some(FreeformMode::Lines));
    }
}
