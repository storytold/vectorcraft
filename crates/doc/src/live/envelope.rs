use super::expand::{Hooks, Outliner, Warper, map_node};
use super::mesh::{GradientMesh, MeshPoint, grid_len, grid_points, nodes_bounds, surface_of};
use super::{Affine, Arc, Color, Deserialize, Node, NodeId, NodeKind, PathData, Point, Rect, Serialize, Vec2, yes};

/// How an envelope distorts its content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EnvelopeKind {
    /// One of the 15 warp styles (`arc`, `arcLower`, … — see [`WarpStyle::from_id`]); bend and
    /// distortions in percent (-100..100).
    Warp {
        style: String,
        #[serde(default)]
        bend: f64,
        #[serde(default)]
        h: f64,
        #[serde(default)]
        v: f64,
        #[serde(default = "yes")]
        horizontal: bool,
    },
    /// A `(rows+1)×(cols+1)` grid of points (row-major) the content's bounding box maps onto.
    Mesh {
        rows: u32,
        cols: u32,
        points: Vec<Point>,
        /// Each point's bezier handles (offsets, see [`H_RIGHT`] …), row-major like `points`.
        /// Empty: the smooth (Catmull-Rom) mesh through the points, as before handles existed;
        /// editing the mesh's points or handles on the canvas fills them in.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        handles: Vec<[Vec2; 4]>,
    },
    /// The content's bounding box maps onto the outline of this path.
    TopObject { path: PathData },
}

/// Envelope Options → Preserve Shape Using: how a distorted raster keeps the envelope's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreserveShape {
    #[default]
    ClippingMask,
    Transparency,
}

impl PreserveShape {
    pub fn id(self) -> &'static str {
        match self {
            Self::ClippingMask => "clippingMask",
            Self::Transparency => "transparency",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "clippingMask" => Some(Self::ClippingMask),
            "transparency" => Some(Self::Transparency),
            _ => None,
        }
    }
}

/// Envelope Options besides Fidelity. `Default` is what envelopes saved before these options
/// existed do (their appearance applies after the distortion); new envelopes start from
/// [`EnvelopeOptions::NEW`], as in the reference app.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvelopeOptions {
    /// Rasters: Anti-Alias.
    pub anti_alias: bool,
    /// Rasters: Preserve Shape Using.
    pub preserve_shape: PreserveShape,
    /// Distort Appearance: strokes and live effects are applied before the distortion, so they
    /// bend with the art (off: they apply to the distorted paths).
    pub distort_appearance: bool,
    /// Distort Linear Gradients (with Distort Appearance): placed gradients bend with the art.
    pub distort_linear_gradients: bool,
    /// Distort Pattern Fills (with Distort Appearance): pattern tiles bend with the art.
    pub distort_pattern_fills: bool,
}

impl Default for EnvelopeOptions {
    fn default() -> Self {
        Self {
            anti_alias: true,
            preserve_shape: PreserveShape::ClippingMask,
            distort_appearance: false,
            distort_linear_gradients: false,
            distort_pattern_fills: false,
        }
    }
}

impl EnvelopeOptions {
    /// The options a new envelope gets (the reference app's defaults).
    pub const NEW: Self = Self {
        anti_alias: true,
        preserve_shape: PreserveShape::ClippingMask,
        distort_appearance: true,
        distort_linear_gradients: false,
        distort_pattern_fills: false,
    };
    /// Do gradients bend with the art?
    pub fn gradients(&self) -> bool {
        self.distort_appearance && self.distort_linear_gradients
    }
    /// Do pattern tiles bend with the art?
    pub fn patterns(&self) -> bool {
        self.distort_appearance && self.distort_pattern_fills
    }
}

/// Handle slots of a [`MeshPoint`]: towards the next column, previous column, next row, previous row.
pub const H_RIGHT: usize = 0;

pub const H_LEFT: usize = 1;

pub const H_DOWN: usize = 2;

pub const H_UP: usize = 3;

/// The envelope's point map for content whose bounding box is `src` (page axes).
pub fn envelope_mapper<'a>(kind: &'a EnvelopeKind, src: Rect) -> Box<dyn Fn(Point) -> Point + 'a> {
    let s = surface_of(kind, src, Affine::IDENTITY);
    let (w, h) = (src.width().max(1e-9), src.height().max(1e-9));
    Box::new(move |q: Point| s((q.x - src.x0) / w, (q.y - src.y0) / h))
}

/// Catmull-Rom tangents of a `(rows+1)×(cols+1)` point grid (row-major) as mesh handles, a third
/// of the tangent towards each neighbour (past the borders the grid goes on straight): the curves
/// an envelope mesh without handles of its own runs along between its points.
pub fn smooth_handles(rows: u32, cols: u32, points: &[Point]) -> Vec<[Vec2; 4]> {
    let (rows, cols) = (rows as usize, cols as usize);
    let at = |r: usize, c: usize| points.get(r * (cols + 1) + c).copied().unwrap_or_default();
    let tangent = |prev: Option<Point>, p: Point, next: Option<Point>| match (prev, next) {
        (Some(a), Some(b)) => (b - a) / 2.0,
        (None, Some(b)) => b - p,
        (Some(a), None) => p - a,
        (None, None) => Vec2::ZERO,
    };
    let mut out = Vec::with_capacity(points.len());
    for r in 0..=rows {
        for c in 0..=cols {
            let p = at(r, c);
            let du = tangent(c.checked_sub(1).map(|c| at(r, c)), p, (c < cols).then(|| at(r, c + 1))) / 3.0;
            let dv = tangent(r.checked_sub(1).map(|r| at(r, c)), p, (r < rows).then(|| at(r + 1, c))) / 3.0;
            // Handles pointing out of the grid are unused: they stay at the point.
            let keep = |inside: bool, d: Vec2| if inside { d } else { Vec2::ZERO };
            let mut h = [Vec2::ZERO; 4];
            (h[H_RIGHT], h[H_LEFT], h[H_DOWN], h[H_UP]) = (keep(c < cols, du), keep(c > 0, -du), keep(r < rows, dv), keep(r > 0, -dv));
            out.push(h);
        }
    }
    out
}

/// A mesh envelope's grid (`rows`×`cols` patches, `points` and their `handles`; without handles
/// the smooth ones, see [`smooth_handles`]) as a gradient mesh with black points: what the
/// canvas draws and the Mesh tool and Direct Selection edit. `None` for a malformed grid.
pub fn envelope_grid(rows: u32, cols: u32, points: &[Point], handles: &[[Vec2; 4]]) -> Option<GradientMesh> {
    if rows == 0 || cols == 0 || grid_len(rows, cols) != Some(points.len()) {
        return None;
    }
    let smooth;
    let handles = if handles.len() == points.len() {
        handles
    } else {
        smooth = smooth_handles(rows, cols, points);
        &smooth
    };
    let points = points.iter().zip(handles).map(|(p, h)| MeshPoint { p: *p, color: Color::BLACK, opacity: 1.0, handles: *h }).collect();
    Some(GradientMesh { rows, cols, points })
}

/// The point of mesh `m` at (u, v) over the whole mesh (each patch an equal share; past the edges
/// the outer patches go on).
pub(super) fn mesh_at(m: &GradientMesh, u: f64, v: f64) -> Point {
    let (rows, cols) = (m.rows.max(1) as i64, m.cols.max(1) as i64);
    let (su, sv) = (u * cols as f64, v * rows as f64);
    let (c, r) = ((su.floor() as i64).clamp(0, cols - 1), (sv.floor() as i64).clamp(0, rows - 1));
    m.eval(r as usize, c as usize, su - c as f64, sv - r as f64)
}

/// The envelope `id` is, or the innermost one it sits in.
pub fn envelope_of(doc: &crate::Document, id: NodeId) -> Option<&Node> {
    doc.ancestry(id)?.into_iter().rev().find_map(|a| doc.node(a).filter(|n| matches!(n.kind, NodeKind::Envelope { .. })))
}

/// What a selected envelope shows on the canvas: its mesh lines (a warp's surface on a 4×4 grid,
/// a mesh envelope's own grid) or its top object's outline, and for a mesh envelope the grid to
/// edit (its points and handles).
pub fn envelope_overlay(n: &Node) -> Option<(PathData, Option<GradientMesh>)> {
    let NodeKind::Envelope { kind, .. } = &n.kind else { return None };
    match &**kind {
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            let grid = envelope_grid(*rows, *cols, points, handles)?;
            Some((grid.lines(), Some(grid)))
        }
        EnvelopeKind::Warp { .. } => Some((EnvelopeMap::of(n)?.surface_mesh(4, 4, Color::BLACK).lines(), None)),
        EnvelopeKind::TopObject { path } => Some((path.clone(), None)),
    }
}

/// The frame an envelope keeps after transform `m` (its frame so far composed with the
/// transform): identity when its axes stay square to the page, unflipped, since evaluating on
/// the page axes gives the same result then.
pub fn envelope_frame(m: Affine) -> Affine {
    let [a, b, c, d, _, _] = m.as_coeffs();
    if b.abs() < 1e-12 && c.abs() < 1e-12 && a > 0.0 && d > 0.0 { Affine::IDENTITY } else { m }
}

/// An envelope's point map: content placed in the envelope's own frame maps, by its bounds there,
/// onto the unit square and from there onto the envelope's surface ([`surface_of`]).
pub struct EnvelopeMap<'a> {
    src: Rect,
    frame: Affine,
    to_frame: Affine,
    surface: Box<dyn Fn(f64, f64) -> Point + 'a>,
}

impl<'a> EnvelopeMap<'a> {
    /// The map of an envelope of `kind` around `content` with `frame`. `None` when the content
    /// has no bounds.
    pub fn new(content: &[Arc<Node>], kind: &'a EnvelopeKind, frame: Affine) -> Option<Self> {
        let frame = if frame.is_finite() && frame.determinant().abs() > 1e-12 { frame } else { Affine::IDENTITY };
        let to_frame = frame.inverse();
        let src = if frame == Affine::IDENTITY {
            nodes_bounds(content)?
        } else {
            content.iter().try_fold(None, |acc, c| {
                let mut m = (**c).clone();
                m.transform(to_frame, false);
                Some(vectorcraft_geom::union_opt(acc, m.geometric_bounds()))
            })??
        };
        Some(Self { src, frame, to_frame, surface: surface_of(kind, src, frame) })
    }

    /// The map of envelope node `n` (`None` for other nodes).
    pub fn of(n: &'a Node) -> Option<Self> {
        match &n.kind {
            NodeKind::Envelope { content, kind, frame, .. } => Self::new(content, kind, **frame),
            _ => None,
        }
    }

    /// The content's bounds in the envelope's frame.
    pub fn src(&self) -> Rect {
        self.src
    }

    /// The surface point at (u, v) ∈ [0,1]².
    pub fn at(&self, u: f64, v: f64) -> Point {
        (self.surface)(u, v)
    }

    /// Where the envelope puts document point `p`.
    pub fn map(&self, p: Point) -> Point {
        let q = self.to_frame * p;
        let (w, h) = (self.src.width().max(1e-9), self.src.height().max(1e-9));
        self.at((q.x - self.src.x0) / w, (q.y - self.src.y0) / h)
    }

    /// A flat `(rows+1)×(cols+1)` grid over the content, in the envelope's frame (row-major).
    pub fn grid(&self, rows: u32, cols: u32) -> Vec<Point> {
        grid_points(self.src, rows, cols).into_iter().map(|p| self.frame * p).collect()
    }

    /// The surface sampled on a `rows`×`cols` grid as a mesh whose handles follow it and whose
    /// points are `color`: Release gives it back as the envelope's shape, Reset with Mesh starts
    /// from it.
    pub fn surface_mesh(&self, rows: u32, cols: u32, color: Color) -> GradientMesh {
        GradientMesh::from_surface(rows, cols, &|u, v| self.at(u, v), &|_, _| color)
    }

    /// The surface's bounds (sampled; a mesh's points and a top object's path bound it exactly).
    fn bounds(&self) -> Option<Rect> {
        const N: usize = 16;
        let mut r: Option<Rect> = None;
        for j in 0..=N {
            for i in 0..=N {
                let q = self.at(i as f64 / N as f64, j as f64 / N as f64);
                r = Some(r.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
            }
        }
        r
    }
}

/// Evaluate an envelope of `kind` around `content` on the page axes with the options envelopes
/// had before Envelope Options (see [`expand_live_hooks`] for an envelope node).
pub fn envelope_expand(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, outline: Outliner) -> Vec<Node> {
    expand_envelope(content, kind, fidelity, EnvelopeOptions::default(), Affine::IDENTITY, outline.into())
}

/// An envelope's content pushed through its map: Fidelity sets how finely curves are cut (and
/// into how many pieces images are), the options what bends with the art.
pub(super) fn expand_envelope(
    content: &[Arc<Node>],
    kind: &EnvelopeKind,
    fidelity: f64,
    options: EnvelopeOptions,
    frame: Affine,
    hooks: Hooks,
) -> Vec<Node> {
    let Some(map) = EnvelopeMap::new(content, kind, frame) else { return content.iter().map(|c| (**c).clone()).collect() };
    let src = map.src();
    let fid = if fidelity.is_finite() { fidelity.clamp(0.0, 100.0) / 100.0 } else { 0.5 };
    let f = |p: Point| map.map(p);
    let w = Warper {
        f: &f,
        piece: src.width().hypot(src.height()).max(1e-6) / (8.0 + fid * 56.0),
        eps: (src.width().max(src.height()) * 0.05).max(0.5),
        cells: (4.0 + fid * 12.0).round() as usize,
        hooks,
        options,
        depth: 0,
    };
    content.iter().map(|c| map_node(c, &w)).collect()
}

/// Envelope bounds: a mesh's points, a top object's path, the warp's surface (sampled).
pub fn envelope_bounds(content: &[Arc<Node>], kind: &EnvelopeKind, frame: Affine) -> Option<Rect> {
    match kind {
        EnvelopeKind::Mesh { rows, cols, points, handles } if !handles.is_empty() => envelope_grid(*rows, *cols, points, handles)?.outline().bounds(),
        EnvelopeKind::Mesh { points, .. } => {
            let first = *points.first()?;
            Some(points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p)))
        }
        EnvelopeKind::TopObject { path } => path.bounds(),
        EnvelopeKind::Warp { .. } => EnvelopeMap::new(content, kind, frame)?.bounds(),
    }
}
