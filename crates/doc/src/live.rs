//! Live blends, envelope distortions and gradient meshes: their data and pure evaluation.
//!
//! These are *live* objects. The document stores the source objects plus parameters; the
//! renderer and the exporters evaluate them on demand ([`expand_live`], [`expand_deep`]).
//! Everything here is pure geometry so the SVG/PDF exporters (which only depend on this crate)
//! can export the expanded form.
//!
//! - **Blend**: key objects are matched anchor-by-anchor (subpaths resampled to equal anchor
//!   counts), paints/strokes/opacity interpolated; optional spine (keys spread along it by arc
//!   length, optionally rotated to its tangent).
//! - **Envelope**: all descendant geometry of the content is pushed through a point map from the
//!   content's bounding box to a warp style, a Catmull-Rom point mesh or a top object (Coons
//!   patch built from the top path's outline split at its corners).
//! - **Gradient mesh**: a grid of points with colours and tangent handles; each patch is a Coons
//!   patch coloured by bilinear interpolation of its four corners.

use std::f64::consts::PI;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Affine, Anchor, BezPath, CubicBez, ParamCurve, PathData, Point, Rect, Shape, SubPath, Vec2};

use crate::appearance::{Appearance, AppearanceItem, FillLayer};
use crate::node::{ImageObject, Node, NodeId, NodeKind};

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn fifty() -> f64 {
    50.0
}

// =====================================================================================
// Data
// =====================================================================================

/// Blend Options → Spacing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendSpacing {
    /// Step count chosen from the colour difference (or the distance when colours match).
    #[default]
    SmoothColor,
    /// Specified number of steps between each pair of key objects (1–1000).
    Steps(u32),
    /// Specified distance (pt) between the steps.
    Distance(f64),
}

/// Blend Options → Orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendOrientation {
    #[default]
    AlignToPage,
    AlignToPath,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendSpec {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
    /// Replaced spine. `None` = the straight lines between the key objects' centres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spine: Option<PathData>,
    /// The anchor of the spine's first subpath each key object sits on (a spine edited on the
    /// canvas: moving a key moves its anchor and the other way round). Empty: the keys spread
    /// evenly along the spine by arc length (Replace Spine).
    #[serde(default, rename = "keyAnchors", skip_serializing_if = "Vec::is_empty")]
    pub key_anchors: Vec<u32>,
    /// Per key object, the anchor of its first subpath the blend starts from (Blend tool clicks
    /// on anchor points); `None`: chosen so closed shapes don't twist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<Option<u32>>,
}

impl BlendSpec {
    /// The start anchor of key `i` (see [`Self::starts`]).
    pub fn start(&self, i: usize) -> Option<usize> {
        self.starts.get(i).copied().flatten().map(|a| a as usize)
    }
}

/// Blend Options set with no blend selected: what new blends start with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendDefaults {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
}

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

/// One tessellated piece of a mesh (a small quad with a solid colour).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshQuad {
    pub pts: [Point; 4],
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

// =====================================================================================
// Blends (evaluation in `crate::blend`)
// =====================================================================================

pub use crate::blend::{
    Spine, blend_expand, blend_spine, blend_step_count, lerp_appearance, lerp_color, lerp_node, lerp_paint, lerp_path, pin_spine,
    steps_knockout_shows,
};

// =====================================================================================
// Warp maps (shared with vectorcraft-effects' Warp effects)
// =====================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpStyle {
    Arc,
    ArcLower,
    ArcUpper,
    Arch,
    Bulge,
    ShellLower,
    ShellUpper,
    Flag,
    Wave,
    Fish,
    Rise,
    Fisheye,
    Inflate,
    Squeeze,
    Twist,
}

impl WarpStyle {
    /// From the id suffix (`arc`, `arcLower`, …).
    pub fn from_id(s: &str) -> Option<Self> {
        use WarpStyle::*;
        Some(match s {
            "arc" => Arc,
            "arcLower" => ArcLower,
            "arcUpper" => ArcUpper,
            "arch" => Arch,
            "bulge" => Bulge,
            "shellLower" => ShellLower,
            "shellUpper" => ShellUpper,
            "flag" => Flag,
            "wave" => Wave,
            "fish" => Fish,
            "rise" => Rise,
            "fisheye" => Fisheye,
            "inflate" => Inflate,
            "squeeze" => Squeeze,
            "twist" => Twist,
            _ => return None,
        })
    }
}

/// The warp map on normalised coordinates: `b` = bend (-1..1), `dh`/`dv` = distortion (-1..1).
pub fn warp_point(style: WarpStyle, b: f64, dh: f64, dv: f64, x: f64, y: f64) -> (f64, f64) {
    use WarpStyle::*;
    let t = (y + 1.0) / 2.0; // 0 at top, 1 at bottom
    let par = 1.0 - x * x; // parabola: 1 at centre, 0 at sides
    let (mut x2, mut y2) = match style {
        Arc => {
            if b.abs() < 1e-6 {
                (x, y)
            } else {
                // Bend the centre line into a circular arc of equal length (sweep = |b|·180°).
                let sweep = b.abs() * PI;
                let r0 = 2.0 / sweep;
                let yy = if b > 0.0 { y } else { -y };
                let a = x / r0;
                let r = r0 - yy;
                let (s, c) = a.sin_cos();
                let (nx, ny) = (r * s, r0 - r * c);
                (nx, if b > 0.0 { ny } else { -ny })
            }
        }
        ArcLower => (x, y + b * par * t),
        ArcUpper => (x, y - b * par * (1.0 - t)),
        Arch => (x, y - b * par),
        Bulge => (x, y + b * par * y),
        ShellLower => (x * (1.0 - 0.5 * b * (1.0 - t)), y + b * par * t),
        ShellUpper => (x * (1.0 - 0.5 * b * t), y - b * par * (1.0 - t)),
        Flag => (x, y + 0.5 * b * (PI * x).sin()),
        Wave => (x + 0.1 * b * (PI * y).sin(), y + 0.3 * b * (2.0 * PI * x).sin() * (0.5 + 0.5 * t)),
        Fish => (x, y * (1.0 + 0.5 * b * (0.75 * PI * (x + 1.0)).sin())),
        Rise => (x, y - b * ((x + 1.0) / 2.0).powi(2) * 2.0 + b),
        Fisheye => {
            let r2 = x * x + y * y;
            let s = if r2 < 1.0 { 1.0 + 0.5 * b * (1.0 - r2) } else { 1.0 };
            (x * s, y * s)
        }
        Inflate => (x * (1.0 + 0.5 * b * (1.0 - y * y)), y * (1.0 + 0.5 * b * par)),
        Squeeze => (x * (1.0 - 0.5 * b * (1.0 - y * y)), y * (1.0 + 0.3 * b * par)),
        Twist => {
            let r = (x * x + y * y).sqrt();
            let a = -b * PI * 0.5 * (1.0 - r / std::f64::consts::SQRT_2).max(0.0);
            let (s, c) = a.sin_cos();
            (x * c - y * s, x * s + y * c)
        }
    };
    // Perspective-like distortion: horizontal narrows one side, vertical one end.
    if dh != 0.0 {
        y2 *= (1.0 + dh * x2 * 0.5).max(0.0);
    }
    if dv != 0.0 {
        x2 *= (1.0 + dv * y2 * 0.5).max(0.0);
    }
    (x2, y2)
}

/// Segment as a cubic; straight lines get handles at 1/3 and 2/3 so a non-linear map bends them.
fn seg_cubic(sp: &SubPath, i: usize) -> CubicBez {
    let c = sp.segment(i);
    if sp.segment_is_line(i) { CubicBez::new(c.p0, c.p0.lerp(c.p3, 1.0 / 3.0), c.p0.lerp(c.p3, 2.0 / 3.0), c.p3) } else { c }
}

fn poly_len(c: &CubicBez) -> f64 {
    c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3)
}

/// Running length along a polyline: `0, |p0p1|, |p0p1| + |p1p2|, …` (one entry per point).
fn cumulative_lengths(pts: &[Point]) -> Vec<f64> {
    let mut total = 0.0;
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0);
    for w in pts.windows(2) {
        total += w[0].distance(w[1]);
        cum.push(total);
    }
    cum
}

/// Map every point of `path` through the non-linear function `f`. Segments are split into pieces
/// no longer than about `max_piece` (up to 64 per segment) and their control points mapped, which
/// approximates the image curve to O(h²). Closedness and subpath structure are preserved.
pub fn map_nonlinear(path: &PathData, max_piece: f64, f: impl Fn(Point) -> Point) -> PathData {
    let max_piece = max_piece.max(1e-3);
    let mut subs = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let n = sp.anchors.len();
        if n == 0 {
            continue;
        }
        let segs = sp.segment_count();
        let mut res: Vec<Anchor> = Vec::new();
        let mut pending_in: Option<Point> = None;
        for i in 0..n {
            let a = &sp.anchors[i];
            let h_in = pending_in.take().unwrap_or_else(|| f(a.h_in));
            res.push(Anchor { p: f(a.p), h_in, h_out: f(a.h_out), kind: a.kind });
            if i < segs {
                let c = seg_cubic(sp, i);
                let k = ((poly_len(&c) / max_piece).ceil() as usize).clamp(1, 64);
                for j in 0..k {
                    let sub = c.subsegment((j as f64 / k as f64)..((j + 1) as f64 / k as f64));
                    if let Some(last) = res.last_mut() {
                        last.h_out = f(sub.p1);
                    }
                    if j + 1 < k {
                        res.push(Anchor { p: f(sub.p3), h_in: f(sub.p2), h_out: f(sub.p3), kind: Default::default() });
                    } else {
                        pending_in = Some(f(sub.p2));
                    }
                }
            }
        }
        if let Some(h) = pending_in
            && sp.closed
            && let Some(first) = res.first_mut()
        {
            first.h_in = h;
        }
        let anchors = res.into_iter().map(|a| Anchor::with_handles(a.p, a.h_in, a.h_out)).collect();
        subs.push(SubPath::new(anchors, sp.closed));
    }
    PathData::new(subs)
}

// =====================================================================================
// Coons patches from outlines
// =====================================================================================

fn flatten_subpath(sp: &SubPath, tol: f64) -> Vec<Point> {
    let mut bp = BezPath::new();
    sp.to_bezpath_into(&mut bp);
    let mut pts: Vec<Point> = Vec::new();
    vectorcraft_geom::kurbo::flatten(bp.iter(), tol, |el| match el {
        vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) if pts.last().is_none_or(|q| q.distance(p) > 1e-9) => pts.push(p),
        _ => {}
    });
    pts
}

fn resample(poly: &[Point], m: usize) -> Vec<Point> {
    if poly.is_empty() {
        return vec![Point::ZERO; m];
    }
    if poly.len() == 1 {
        return vec![poly[0]; m];
    }
    let cum = cumulative_lengths(poly);
    let total = cum.last().copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(m);
    let mut i = 0;
    for k in 0..m {
        let s = if m > 1 { total * k as f64 / (m - 1) as f64 } else { 0.0 };
        while i + 2 < cum.len() && cum[i + 1] < s {
            i += 1;
        }
        let seg = (cum[i + 1] - cum[i]).max(1e-12);
        out.push(poly[i].lerp(poly[i + 1], ((s - cum[i]) / seg).clamp(0.0, 1.0)));
    }
    out
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

impl GradientMesh {
    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * (self.cols as usize + 1) + c
    }
    pub fn is_valid(&self) -> bool {
        self.rows >= 1 && self.cols >= 1 && self.points.len() == (self.rows as usize + 1) * (self.cols as usize + 1)
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

    /// Bilinear colour and opacity of patch (r, c) at local (u, v).
    pub fn color_at(&self, r: usize, c: usize, u: f64, v: f64) -> (Color, f32) {
        let q =
            [&self.points[self.idx(r, c)], &self.points[self.idx(r, c + 1)], &self.points[self.idx(r + 1, c)], &self.points[self.idx(r + 1, c + 1)]];
        let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
        let mut rgb = [0.0f32; 3];
        let mut op = 0.0f32;
        for k in 0..4 {
            let c = q[k].color.to_rgb();
            for (ch, x) in rgb.iter_mut().zip(c) {
                *ch += x * w[k] as f32;
            }
            op += q[k].opacity * w[k] as f32;
        }
        (Color::rgb(rgb[0], rgb[1], rgb[2]), op)
    }

    /// Tessellate into `n`×`n` quads per patch, each coloured at its centre.
    pub fn quads(&self, n: usize) -> Vec<MeshQuad> {
        if !self.is_valid() {
            return vec![];
        }
        let n = n.clamp(1, 64);
        let mut out = Vec::with_capacity(self.rows as usize * self.cols as usize * n * n);
        let mut grid = vec![Point::ZERO; (n + 1) * (n + 1)];
        for r in 0..self.rows as usize {
            for c in 0..self.cols as usize {
                for j in 0..=n {
                    for i in 0..=n {
                        grid[j * (n + 1) + i] = self.eval(r, c, i as f64 / n as f64, j as f64 / n as f64);
                    }
                }
                for j in 0..n {
                    for i in 0..n {
                        let (color, opacity) = self.color_at(r, c, (i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
                        let g = |a: usize, b: usize| grid[b * (n + 1) + a];
                        out.push(MeshQuad { pts: [g(i, j), g(i + 1, j), g(i + 1, j + 1), g(i, j + 1)], color, opacity });
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
        if index >= self.points.len() {
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
fn grid_eval(rows: usize, cols: usize, pts: &[Point], u: f64, v: f64) -> Point {
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
fn surface_of<'a>(kind: &'a EnvelopeKind, src: Rect, frame: Affine) -> Box<dyn Fn(f64, f64) -> Point + 'a> {
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
                    warp_point(st, b, dh, dv, x, y)
                } else {
                    let (a, b2) = warp_point(st, b, dh, dv, y, x);
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
            if *rows == 0 || *cols == 0 || points.len() as u64 != (u64::from(*rows) + 1) * (u64::from(*cols) + 1) {
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
    // (Counts come from files: no overflow on 32-bit targets.)
    if rows == 0 || cols == 0 || points.len() as u64 != (u64::from(rows) + 1) * (u64::from(cols) + 1) {
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
fn mesh_at(m: &GradientMesh, u: f64, v: f64) -> Point {
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
    match kind {
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
            NodeKind::Envelope { content, kind, frame, .. } => Self::new(content, kind, *frame),
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

/// Optional hook that converts nodes the pure evaluation can't map (text) to outlines.
pub type Outliner<'a> = Option<&'a dyn Fn(&Node) -> Option<Node>>;

/// Distort Pattern Fills: the tiles pattern swatch `name` painted with placement `xf` lays over
/// `region` (see [`crate::PatternDef::instances_in`]); `None` without such a swatch.
pub type PatternTiles<'a> = &'a dyn Fn(&str, Affine, Rect) -> Option<Vec<Node>>;

/// What live evaluation asks of the crates above this one (`vectorcraft-effects` supplies them).
#[derive(Clone, Copy, Default)]
pub struct Hooks<'a> {
    /// Type as its glyph outlines, a symbol instance as its art (see [`Outliner`]).
    pub outline: Outliner<'a>,
    /// Distort Appearance: a path's strokes and geometry effects baked into filled art (`None`
    /// when it has none to bake).
    pub appearance: Outliner<'a>,
    /// Distort Pattern Fills: the tiles a pattern fill lays (see [`PatternTiles`]).
    pub pattern: Option<PatternTiles<'a>>,
}

impl<'a> From<Outliner<'a>> for Hooks<'a> {
    fn from(outline: Outliner<'a>) -> Self {
        Self { outline, ..Default::default() }
    }
}

/// The affine map that best matches `f` near `p` (finite differences with step `eps`).
pub fn affine_near(f: &dyn Fn(Point) -> Point, p: Point, eps: f64) -> Affine {
    let o = f(p);
    let ex = (f(p + Vec2::new(eps, 0.0)) - f(p - Vec2::new(eps, 0.0))) / (2.0 * eps);
    let ey = (f(p + Vec2::new(0.0, eps)) - f(p - Vec2::new(0.0, eps))) / (2.0 * eps);
    let lin = Affine::new([ex.x, ex.y, ey.x, ey.y, 0.0, 0.0]);
    Affine::translate(o.to_vec2()) * lin * Affine::translate(-p.to_vec2())
}

/// How deep hooks may nest art inside art (symbols in symbols, envelopes in envelopes) before
/// the rest is left as it is: a file can make a symbol contain itself.
const MAX_HOOK_DEPTH: u32 = 16;

/// How an envelope pushes art through its point map.
#[derive(Clone, Copy)]
struct Warper<'a> {
    f: &'a dyn Fn(Point) -> Point,
    /// Longest piece a segment is cut into before mapping.
    piece: f64,
    /// Finite-difference step for the local affine maps gradients follow.
    eps: f64,
    /// Rows and columns of the pieces an image is cut into.
    cells: usize,
    hooks: Hooks<'a>,
    options: EnvelopeOptions,
    /// How many hooks the art being mapped came through.
    depth: u32,
}

impl Warper<'_> {
    /// This warper one hook deeper (`None` past [`MAX_HOOK_DEPTH`]).
    fn deeper(&self) -> Option<Self> {
        (self.depth < MAX_HOOK_DEPTH).then_some(Self { depth: self.depth + 1, ..*self })
    }
}

/// `to` (art standing in for `from`: its outlines, its art) with `from`'s name, transparency,
/// opacity mask and effects.
fn carry_look(from: &Node, mut to: Node) -> Node {
    to.name = from.name.clone();
    to.opacity = from.opacity;
    to.blend = from.blend;
    to.isolate = from.isolate;
    to.knockout = from.knockout;
    to.knockout_shape = from.knockout_shape;
    to.mask = from.mask.clone();
    to.appearance.effects = from.appearance.effects.clone();
    to
}

fn map_node(n: &Node, w: &Warper) -> Node {
    let deeper = w.deeper();
    // Distort Appearance: strokes and geometry effects bend with the art.
    if w.options.distort_appearance
        && let Some(d) = &deeper
        && let Some(o) = w.hooks.appearance.and_then(|h| h(n))
    {
        return map_node(&o, d);
    }
    // Distort Pattern Fills: the tiles bend with the art (not the patterns inside them).
    if w.options.patterns()
        && let Some(d) = &deeper
        && let Some(o) = w.hooks.pattern.and_then(|p| pattern_art(n, p))
    {
        let inner = Warper { options: EnvelopeOptions { distort_pattern_fills: false, ..d.options }, ..*d };
        return map_node(&o, &inner);
    }
    let mut out = n.clone();
    // Distort Linear Gradients: the gradient vectors follow the warp where they sit.
    if w.options.gradients()
        && let Some(b) = n.geometric_bounds()
    {
        out.appearance.warp_linear_gradients(b, &|p| affine_near(w.f, p, w.eps));
    }
    if let Some(m) = &mut out.mask {
        m.art = Arc::new(map_node(&m.art, w));
    }
    match &mut out.kind {
        NodeKind::Path { path, live, .. } => {
            *path = map_nonlinear(path, w.piece, w.f);
            *live = None;
        }
        // A compound shape stays live: its members bend.
        NodeKind::Layer { children, .. }
        | NodeKind::Group { children, .. }
        | NodeKind::Compound { children, .. }
        | NodeKind::CompoundShape { children } => {
            for c in children.iter_mut() {
                *c = Arc::new(map_node(c, w));
            }
        }
        NodeKind::Mesh(m) => m.map(w.f),
        NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
            if let Some(d) = &deeper {
                return map_node(&expanded_group_hooks(n, w.hooks), d);
            }
        }
        NodeKind::Text(_) | NodeKind::SymbolInstance { .. } => {
            if let Some(d) = &deeper
                && let Some(o) = w.hooks.outline.and_then(|h| h(n))
            {
                return map_node(&carry_look(n, o), d);
            }
        }
        NodeKind::Image(im) => {
            let im = im.clone();
            return warp_image(&out, &im, w);
        }
        // Its art is plain, so it warps at this depth.
        NodeKind::PlacedDocument(_) => return map_node(&expanded_group_hooks(n, w.hooks), w),
    }
    out
}

/// The affine map taking triangle `s` to triangle `d` (`None` when `s` is degenerate).
fn affine_between(s: [Point; 3], d: [Point; 3]) -> Option<Affine> {
    let basis = |p: [Point; 3]| Affine::new([p[1].x - p[0].x, p[1].y - p[0].y, p[2].x - p[0].x, p[2].y - p[0].y, p[0].x, p[0].y]);
    let bs = basis(s);
    if bs.determinant().abs() < 1e-12 {
        return None;
    }
    let a = basis(d) * bs.inverse();
    a.is_finite().then_some(a)
}

/// Image `n` (`im`) pushed through the warp as a raster mesh warp: its frame cut into
/// `w.cells`² cells of two triangles, each a clip group showing the image under the affine map
/// that takes the triangle where the warp takes its corners. Each piece reaches a little past its
/// edges (at most half a point) so neighbours overlap and no seams show.
fn warp_image(n: &Node, im: &ImageObject, w: &Warper) -> Node {
    let k = w.cells.max(1);
    let (pw, ph) = (im.width as f64, im.height as f64);
    let at = |i: usize, j: usize| Point::new(pw * i as f64 / k as f64, ph * j as f64 / k as f64);
    let mut pieces = Vec::with_capacity(2 * k * k);
    for j in 0..k {
        for i in 0..k {
            let s = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            let d = s.map(|p| (w.f)(im.xf * p));
            for t in [[0, 1, 2], [0, 2, 3]] {
                let (st, dt) = (t.map(|v| s[v]), t.map(|v| d[v]));
                let Some(xf) = affine_between(st, dt) else { continue };
                let c = Point::new((dt[0].x + dt[1].x + dt[2].x) / 3.0, (dt[0].y + dt[1].y + dt[2].y) / 3.0);
                let grow = (dt[0].distance(dt[1]).max(dt[1].distance(dt[2])) * 0.02).min(0.5);
                let tri: Vec<Point> = dt
                    .iter()
                    .map(|p| {
                        let v = *p - c;
                        let len = v.hypot();
                        if len > 1e-12 { *p + v * (grow / len) } else { *p }
                    })
                    .collect();
                let mut clip = Node::path(n.id, PathData::single(SubPath::polyline(&tri, true)), Appearance::default());
                if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                    *clipping = true;
                }
                let image = Node::new(n.id, NodeKind::Image(ImageObject { xf, ..im.clone() }));
                pieces.push(Arc::new(Node::new(n.id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(image)], clip: true })));
            }
        }
    }
    let mut g = n.clone();
    g.kind = NodeKind::Group { children: pieces, clip: false };
    g
}

/// Distort Pattern Fills: path or compound path `n` with each visible pattern fill turned into the
/// tiles it paints in a clip group shaped like `n` (with the fill's opacity and blend mode), its
/// other fills and strokes on copies of it, in paint order, under `n`'s transparency. `None`
/// without a pattern fill whose definition `pattern` finds.
fn pattern_art(n: &Node, tiles: PatternTiles) -> Option<Node> {
    let (path, rule, _) = crate::blend::node_path(n)?;
    let pattern_fill = |i: &AppearanceItem| matches!(i, AppearanceItem::Fill(f) if f.visible && matches!(f.paint, Paint::Pattern { .. }));
    if !n.appearance.items.iter().any(pattern_fill) {
        return None;
    }
    let bounds = path.bounds()?;
    let def = |i: &AppearanceItem| match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Pattern { pattern: name, xf } => tiles(name, *xf, bounds).map(|t| (t, f.opacity, f.blend)),
            _ => None,
        },
        _ => None,
    };
    let plain = |items: Vec<AppearanceItem>| {
        Arc::new(Node {
            appearance: Appearance { items, ..Default::default() },
            opacity: 1.0,
            blend: Default::default(),
            isolate: false,
            knockout: Default::default(),
            knockout_shape: false,
            mask: None,
            name: None,
            ..n.clone()
        })
    };
    let mut pieces = vec![];
    let mut rest = vec![];
    for i in &n.appearance.items {
        let Some((tiles, opacity, blend)) = def(i) else {
            rest.push(i.clone());
            continue;
        };
        if !rest.is_empty() {
            pieces.push(plain(std::mem::take(&mut rest)));
        }
        let mut clip = Node::path(n.id, path.clone(), Appearance::default());
        if let NodeKind::Path { clipping, rule: r, .. } = &mut clip.kind {
            (*clipping, *r) = (true, rule);
        }
        let children = std::iter::once(clip).chain(tiles).map(Arc::new).collect();
        let mut g = Node::new(n.id, NodeKind::Group { children, clip: true });
        g.opacity = opacity;
        g.blend = blend;
        pieces.push(Arc::new(g));
    }
    if !rest.is_empty() {
        pieces.push(plain(rest));
    }
    Some(Node {
        kind: NodeKind::Group { children: pieces, clip: false },
        appearance: Appearance { effects: n.appearance.effects.clone(), ..Default::default() },
        ..n.clone()
    })
}

/// Evaluate an envelope of `kind` around `content` on the page axes with the options envelopes
/// had before Envelope Options (see [`expand_live_hooks`] for an envelope node).
pub fn envelope_expand(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, outline: Outliner) -> Vec<Node> {
    expand_envelope(content, kind, fidelity, EnvelopeOptions::default(), Affine::IDENTITY, outline.into())
}

/// An envelope's content pushed through its map: Fidelity sets how finely curves are cut (and
/// into how many pieces images are), the options what bends with the art.
fn expand_envelope(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, options: EnvelopeOptions, frame: Affine, hooks: Hooks) -> Vec<Node> {
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

// =====================================================================================
// Expansion
// =====================================================================================

/// Is this one of the live kinds?
pub fn is_live(n: &Node) -> bool {
    matches!(
        n.kind,
        NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::CompoundShape { .. }
    )
}

/// Mesh tessellation as filled quad paths (no stroke), `n`×`n` per patch.
pub fn mesh_quad_nodes(m: &GradientMesh, n: usize) -> Vec<Node> {
    m.quads(n)
        .into_iter()
        .map(|q| {
            let mut ap = Appearance::default();
            let mut fl = FillLayer::new(Paint::solid(q.color));
            fl.opacity = q.opacity;
            ap.items.push(AppearanceItem::Fill(fl));
            Node::path(NodeId(0), PathData::single(SubPath::polyline(&q.pts, true)), ap)
        })
        .collect()
}

/// One level of evaluation of a live object (non-live nodes return themselves). Blend steps and
/// envelope results may still contain live objects (e.g. a blend of two meshes).
pub fn expand_live(n: &Node) -> Vec<Node> {
    expand_live_with(n, None)
}

pub fn expand_live_with(n: &Node, outline: Outliner) -> Vec<Node> {
    expand_live_hooks(n, outline.into())
}

/// [`expand_live_with`] with everything an envelope can ask of the crates above (see [`Hooks`]).
pub fn expand_live_hooks(n: &Node, hooks: Hooks) -> Vec<Node> {
    match &n.kind {
        NodeKind::Blend { children, spec } => blend_expand(children, spec),
        NodeKind::Envelope { content, kind, fidelity, options, frame, .. } => expand_envelope(content, kind, *fidelity, *options, *frame, hooks),
        NodeKind::Mesh(m) => mesh_quad_nodes(m, 8),
        NodeKind::Repeat(r) => r.expand(),
        NodeKind::PlacedDocument(p) => p.art(n),
        // Booleans live above this crate: the outliner evaluates a compound shape into its path
        // (painted with the compound's appearance). Without one, the members as they are.
        NodeKind::CompoundShape { children } => match hooks.outline.and_then(|h| h(n)) {
            Some(o) => vec![o],
            None => children.iter().map(|c| (**c).clone()).collect(),
        },
        _ => vec![n.clone()],
    }
}

/// A live object evaluated into a plain group (keeping the object's id, name and transparency).
pub fn expanded_group(n: &Node, outline: Outliner) -> Node {
    expanded_group_hooks(n, outline.into())
}

/// [`expanded_group`] with [`Hooks`].
pub fn expanded_group_hooks(n: &Node, hooks: Hooks) -> Node {
    let children = expand_live_hooks(n, hooks).into_iter().map(Arc::new).collect();
    let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
    g.name = n.name.clone();
    g.visible = n.visible;
    g.locked = n.locked;
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.isolate = n.isolate;
    g.knockout = n.knockout;
    // Knocking out opaque blend steps changes nothing: don't make the outputs work for it.
    if matches!(n.kind, NodeKind::Blend { .. }) && g.knockout == crate::Knockout::On && !crate::blend::knockout_shows(&g) {
        g.knockout = crate::Knockout::Off;
    }
    g.knockout_shape = n.knockout_shape;
    g.mask = n.mask.clone();
    g
}

/// Recursively replace every live object in `n` by plain geometry (for exporters).
pub fn expand_deep(n: &Node, outline: Outliner) -> Node {
    expand_deep_hooks(n, outline.into())
}

/// [`expand_deep`] with [`Hooks`].
pub fn expand_deep_hooks(n: &Node, hooks: Hooks) -> Node {
    let mut out = if is_live(n) { expanded_group_hooks(n, hooks) } else { n.clone() };
    if let Some(ch) = out.children_mut() {
        for c in ch.iter_mut() {
            if subtree_has_live(c) {
                *c = Arc::new(expand_deep_hooks(c, hooks));
            }
        }
    }
    out
}

/// Does `n` or any descendant need live evaluation?
pub fn subtree_has_live(n: &Node) -> bool {
    let mut any = false;
    n.walk(&mut |c| any |= is_live(c));
    any
}

/// Largest stroke outset in a subtree (for visual bounds of envelopes).
pub(crate) fn max_outset(nodes: &[Arc<Node>]) -> f64 {
    let mut o = 0.0f64;
    for n in nodes {
        n.walk(&mut |c| o = o.max(c.appearance.outset()));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::shapes;

    fn rect_node(x: f64, y: f64, w: f64, h: f64, c: Color) -> Arc<Node> {
        Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(x, y, x + w, y + h)), Appearance::basic(Paint::solid(c), Paint::None, 0.0)))
    }

    #[test]
    fn lerp_path_equalises_anchor_counts() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0));
        let t = lerp_path(&a, &shapes::polygon(Point::new(5.0, 5.0), 5.0, 7, 0.0), 0.5);
        assert_eq!(t.subpaths[0].anchors.len(), 7);
        let m = lerp_path(&a, &b, 0.0);
        assert_eq!(m.subpaths[0].anchors.len(), 4);
    }

    #[test]
    fn blend_steps_and_colours() {
        let keys = vec![rect_node(0.0, 0.0, 10.0, 10.0, Color::BLACK), rect_node(100.0, 0.0, 10.0, 10.0, Color::WHITE)];
        let out = blend_expand(&keys, &BlendSpec { spacing: BlendSpacing::Steps(3), ..Default::default() });
        assert_eq!(out.len(), 5);
        let mid = out[2].appearance.fill_paint().color().unwrap().to_rgb()[0];
        assert!((mid - 0.5).abs() < 1e-3);
    }

    #[test]
    fn coons_rect_is_identity() {
        let c = Coons::from_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0))).unwrap();
        let p = c.eval(0.25, 0.5);
        assert!((p.x - 25.0).abs() < 1e-6 && (p.y - 25.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn maps_go_on_past_the_content_box() {
        // A stroke outline reaches past the box: a flat mesh and a rectangle stay the identity there.
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let p = grid_eval(2, 2, &grid_points(r, 2, 2), -0.05, 1.1);
        assert!((p.x + 5.0).abs() < 1e-6 && (p.y - 110.0).abs() < 1e-6, "{p:?}");
        let c = Coons::from_path(&shapes::rectangle(r)).unwrap().eval(1.05, -0.1);
        assert!((c.x - 105.0).abs() < 1e-6 && (c.y + 10.0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn grid_identity() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let pts = grid_points(r, 3, 4);
        let p = grid_eval(3, 4, &pts, 0.37, 0.81);
        assert!((p.x - 37.0).abs() < 1e-6 && (p.y - 81.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn mesh_insert_and_remove_lines() {
        let mut m =
            GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let i = m.add_lines_at(Point::new(30.0, 60.0)).unwrap();
        assert_eq!((m.rows, m.cols), (2, 2));
        assert!(m.points[i].p.distance(Point::new(30.0, 60.0)) < 0.5, "{:?}", m.points[i].p);
        assert!(m.remove_point_lines(i));
        assert_eq!((m.rows, m.cols), (1, 1));
    }
}
