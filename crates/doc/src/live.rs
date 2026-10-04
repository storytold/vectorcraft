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
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_geom::{Affine, Anchor, BezPath, CubicBez, FillRule, ParamCurve, PathData, Point, Rect, Shape, SubPath, Vec2};

use crate::appearance::{Appearance, AppearanceItem, FillLayer};
use crate::node::{Node, NodeId, NodeKind};

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
    Mesh { rows: u32, cols: u32, points: Vec<Point> },
    /// The content's bounding box maps onto the outline of this path.
    TopObject { path: PathData },
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
// Colour / paint interpolation
// =====================================================================================

/// Interpolate two colours in their shared model (display RGB when the models differ).
pub fn lerp_color(a: &Color, b: &Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t;
    match (*a, *b) {
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => Color::cmyk(l(c, c2), l(m, m2), l(y, y2), l(k, k2)),
        (Color::Gray { k }, Color::Gray { k: k2 }) => Color::gray(l(k, k2)),
        _ => a.lerp(b, t),
    }
}

fn solid_gradient(c: Color, like: &Gradient) -> Gradient {
    let mut g = like.clone();
    for s in &mut g.stops {
        s.color = c;
        s.opacity = 1.0;
    }
    g
}

fn lerp_gradient(a: &GradientPaint, b: &GradientPaint, t: f32) -> Option<GradientPaint> {
    if a.gradient.kind != b.gradient.kind || a.gradient.stops.len() != b.gradient.stops.len() {
        return None;
    }
    let mut out = if t < 0.5 { a.clone() } else { b.clone() };
    let l = |x: f32, y: f32| x + (y - x) * t;
    for (i, s) in out.gradient.stops.iter_mut().enumerate() {
        let (sa, sb) = (&a.gradient.stops[i], &b.gradient.stops[i]);
        s.offset = l(sa.offset, sb.offset);
        s.color = lerp_color(&sa.color, &sb.color, t);
        s.opacity = l(sa.opacity, sb.opacity);
        s.midpoint = l(sa.midpoint, sb.midpoint);
    }
    out.angle = a.angle + (b.angle - a.angle) * t as f64;
    out.swatch = None;
    if let (Some(ga), Some(gb)) = (a.geom, b.geom) {
        let mut g = ga;
        g.start = ga.start.lerp(gb.start, t as f64);
        g.end = ga.end.lerp(gb.end, t as f64);
        g.aspect = ga.aspect + (gb.aspect - ga.aspect) * t as f64;
        out.geom = Some(g);
    } else {
        out.geom = None;
    }
    Some(out)
}

/// Interpolate paints: solid↔solid, compatible gradients, solid↔gradient; otherwise switch halfway.
pub fn lerp_paint(a: &Paint, b: &Paint, t: f32) -> Paint {
    match (a, b) {
        (Paint::Solid { color: x, .. }, Paint::Solid { color: y, .. }) => Paint::solid(lerp_color(x, y, t)),
        (Paint::Gradient(ga), Paint::Gradient(gb)) => match lerp_gradient(ga, gb, t) {
            Some(g) => Paint::Gradient(Box::new(g)),
            None => (if t < 0.5 { a } else { b }).clone(),
        },
        (Paint::Solid { color, .. }, Paint::Gradient(g)) => {
            let ga = GradientPaint { gradient: solid_gradient(*color, &g.gradient), ..(**g).clone() };
            Paint::Gradient(Box::new(lerp_gradient(&ga, g, t).unwrap_or_else(|| (**g).clone())))
        }
        (Paint::Gradient(g), Paint::Solid { color, .. }) => {
            let gb = GradientPaint { gradient: solid_gradient(*color, &g.gradient), ..(**g).clone() };
            Paint::Gradient(Box::new(lerp_gradient(g, &gb, t).unwrap_or_else(|| (**g).clone())))
        }
        _ => (if t < 0.5 { a } else { b }).clone(),
    }
}

/// Interpolate appearance stacks: item by item when their structure matches, else the top fill
/// and stroke only (on the structure of the nearer key).
pub fn lerp_appearance(a: &Appearance, b: &Appearance, t: f64) -> Appearance {
    let tf = t as f32;
    let mut out = if t < 0.5 { a.clone() } else { b.clone() };
    let same = a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|(x, y)| {
            matches!((x, y), (AppearanceItem::Fill(_), AppearanceItem::Fill(_)) | (AppearanceItem::Stroke(_), AppearanceItem::Stroke(_)))
        });
    if same {
        for (i, it) in out.items.iter_mut().enumerate() {
            match (it, &a.items[i], &b.items[i]) {
                (AppearanceItem::Fill(o), AppearanceItem::Fill(x), AppearanceItem::Fill(y)) => {
                    o.paint = lerp_paint(&x.paint, &y.paint, tf);
                    o.opacity = x.opacity + (y.opacity - x.opacity) * tf;
                }
                (AppearanceItem::Stroke(o), AppearanceItem::Stroke(x), AppearanceItem::Stroke(y)) => {
                    o.paint = lerp_paint(&x.paint, &y.paint, tf);
                    o.width = x.width + (y.width - x.width) * t;
                    o.opacity = x.opacity + (y.opacity - x.opacity) * tf;
                }
                _ => {}
            }
        }
        return out;
    }
    let (fa, fb) = (a.fill_paint(), b.fill_paint());
    if !(fa.is_none() && fb.is_none()) {
        out.set_fill(lerp_paint(&fa, &fb, tf));
    }
    let (sa, sb) = (a.stroke_paint(), b.stroke_paint());
    if !(sa.is_none() && sb.is_none()) {
        out.set_stroke(lerp_paint(&sa, &sb, tf));
        let w = a.stroke_width() + (b.stroke_width() - a.stroke_width()) * t;
        if let Some(s) = out.stroke_mut() {
            s.width = w;
        }
    }
    out
}

// =====================================================================================
// Path interpolation
// =====================================================================================

/// Grow a subpath to `n` anchors by splitting its longest segments.
fn grow(sp: &mut SubPath, n: usize) {
    let mut guard = 0;
    while sp.anchors.len() < n && guard < 20_000 {
        guard += 1;
        if sp.segment_count() == 0 {
            match sp.anchors.last().copied() {
                Some(a) => sp.anchors.push(Anchor::corner(a.p)),
                None => return,
            }
            continue;
        }
        let seg = (0..sp.segment_count())
            .max_by(|i, j| {
                let (ci, cj) = (sp.segment(*i), sp.segment(*j));
                (ci.p3 - ci.p0).hypot().total_cmp(&(cj.p3 - cj.p0).hypot())
            })
            .unwrap_or(0);
        sp.insert_anchor(seg, 0.5);
    }
}

fn lerp_anchor(x: &Anchor, y: &Anchor, t: f64) -> Anchor {
    Anchor { p: x.p.lerp(y.p, t), h_in: x.h_in.lerp(y.h_in, t), h_out: x.h_out.lerp(y.h_out, t), kind: if t < 0.5 { x.kind } else { y.kind } }
}

/// Interpolate two paths anchor-by-anchor. Subpath and anchor counts are equalised first (missing
/// subpaths grow out of a point; shorter subpaths are resampled); closed subpaths of opposite
/// winding are reversed so they don't flip through themselves.
pub fn lerp_path(a: &PathData, b: &PathData, t: f64) -> PathData {
    let n = a.subpaths.len().max(b.subpaths.len());
    let ca = a.bounds().map(|r| r.center()).unwrap_or_default();
    let cb = b.bounds().map(|r| r.center()).unwrap_or_default();
    let degenerate = |like: &SubPath, c: Point| SubPath::new(vec![Anchor::corner(c); like.anchors.len().max(1)], like.closed);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (mut sa, mut sb) = match (a.subpaths.get(i), b.subpaths.get(i)) {
            (Some(x), Some(y)) => (x.clone(), y.clone()),
            (Some(x), None) => (x.clone(), degenerate(x, cb)),
            (None, Some(y)) => (degenerate(y, ca), y.clone()),
            (None, None) => continue,
        };
        if sa.closed && sb.closed && sa.anchors.len() > 2 && sb.anchors.len() > 2 && (sa.area() * sb.area()) < 0.0 {
            sb.reverse();
        }
        let m = sa.anchors.len().max(sb.anchors.len());
        grow(&mut sa, m);
        grow(&mut sb, m);
        let anchors = sa.anchors.iter().zip(&sb.anchors).map(|(x, y)| lerp_anchor(x, y, t)).collect();
        out.push(SubPath::new(anchors, if t < 0.5 { sa.closed } else { sb.closed }));
    }
    PathData::new(out)
}

/// Path data of a path or compound path node (compound children concatenated).
fn node_path(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            let subs = children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect();
            Some((PathData::new(subs), *rule))
        }
        _ => None,
    }
}

/// Interpolate two objects at `t` (0 = `a`, 1 = `b`).
pub fn lerp_node(a: &Node, b: &Node, t: f64) -> Node {
    let base = if t < 0.5 { a } else { b };
    let mut n = match (&a.kind, &b.kind) {
        (NodeKind::Group { children: ca, clip: k1 }, NodeKind::Group { children: cb, clip: k2 }) if ca.len() == cb.len() && k1 == k2 => {
            let children = ca.iter().zip(cb).map(|(x, y)| Arc::new(lerp_node(x, y, t))).collect();
            Node::new(NodeId(0), NodeKind::Group { children, clip: *k1 })
        }
        (NodeKind::Mesh(ma), NodeKind::Mesh(mb)) if ma.rows == mb.rows && ma.cols == mb.cols && ma.points.len() == mb.points.len() => {
            let mut m = ma.clone();
            for (i, p) in m.points.iter_mut().enumerate() {
                let q = &mb.points[i];
                p.p = p.p.lerp(q.p, t);
                p.color = lerp_color(&p.color, &q.color, t as f32);
                p.opacity += (q.opacity - p.opacity) * t as f32;
                for h in 0..4 {
                    p.handles[h] = p.handles[h].lerp(q.handles[h], t);
                }
            }
            Node::new(NodeId(0), NodeKind::Mesh(m))
        }
        _ => match (node_path(a), node_path(b)) {
            (Some((pa, ra)), Some((pb, rb))) => {
                let path = lerp_path(&pa, &pb, t);
                let mut n = Node::path(NodeId(0), path, Appearance::default());
                if let NodeKind::Path { rule, .. } = &mut n.kind {
                    *rule = if t < 0.5 { ra } else { rb };
                }
                n
            }
            _ => {
                // Different structure: move/scale a copy of the nearer key into the interpolated box.
                let mut n = base.clone();
                if let (Some(ba), Some(bb), Some(bn)) = (a.geometric_bounds(), b.geometric_bounds(), base.geometric_bounds()) {
                    let w = ba.width() + (bb.width() - ba.width()) * t;
                    let h = ba.height() + (bb.height() - ba.height()) * t;
                    let c = ba.center().lerp(bb.center(), t);
                    let sx = if bn.width() > 1e-9 { w / bn.width() } else { 1.0 };
                    let sy = if bn.height() > 1e-9 { h / bn.height() } else { 1.0 };
                    n.transform(
                        Affine::translate(c.to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-bn.center().to_vec2()),
                        false,
                    );
                }
                if let NodeKind::Path { live, .. } = &mut n.kind {
                    *live = None;
                }
                n.id = NodeId(0);
                n
            }
        },
    };
    n.appearance = lerp_appearance(&a.appearance, &b.appearance, t);
    n.opacity = a.opacity + (b.opacity - a.opacity) * t as f32;
    n.blend = base.blend;
    n.visible = true;
    n
}

// =====================================================================================
// Blend evaluation
// =====================================================================================

/// Largest channel difference (0..1) between the solid colours of two objects' top fill and stroke.
fn color_distance(a: &Node, b: &Node) -> f32 {
    let mut d = 0.0f32;
    let mut cmp = |x: &Paint, y: &Paint| {
        let cols = |p: &Paint| -> Vec<Color> {
            match p {
                Paint::Solid { color, .. } => vec![*color],
                Paint::Gradient(g) => g.gradient.stops.iter().map(|s| s.color).collect(),
                _ => vec![],
            }
        };
        let (cx, cy) = (cols(x), cols(y));
        for (i, c) in cx.iter().enumerate() {
            if let Some(o) = cy.get(i).or(cy.first()) {
                let (p, q) = (c.to_rgb(), o.to_rgb());
                for k in 0..3 {
                    d = d.max((p[k] - q[k]).abs());
                }
            }
        }
    };
    cmp(&a.appearance.fill_paint(), &b.appearance.fill_paint());
    cmp(&a.appearance.stroke_paint(), &b.appearance.stroke_paint());
    d
}

fn center_of(n: &Node) -> Point {
    n.geometric_bounds().map(|b| b.center()).unwrap_or_default()
}

/// Number of intermediate steps between two keys `len` apart (along the spine).
pub fn blend_step_count(a: &Node, b: &Node, spacing: BlendSpacing, len: f64) -> usize {
    match spacing {
        BlendSpacing::Steps(n) => n.clamp(1, 1000) as usize,
        BlendSpacing::Distance(d) => {
            let d = d.max(0.01);
            ((len / d).round() as i64 - 1).clamp(0, 1000) as usize
        }
        BlendSpacing::SmoothColor => {
            let cd = color_distance(a, b);
            if cd > 1.0 / 255.0 {
                ((cd * 255.0 / 2.0).ceil() as usize).clamp(1, 256)
            } else {
                // Same colours: base the count on the distance between the objects.
                let (ba, bb) = (a.geometric_bounds(), b.geometric_bounds());
                let dist = match (ba, bb) {
                    (Some(x), Some(y)) => (x.x0 - y.x0).abs().max((x.x1 - y.x1).abs()).max((x.y0 - y.y0).abs()).max((x.y1 - y.y1).abs()),
                    _ => len,
                };
                ((dist / 2.0).ceil() as usize).clamp(1, 256)
            }
        }
    }
}

/// A spine flattened to a polyline with cumulative lengths.
pub struct Spine {
    pts: Vec<Point>,
    cum: Vec<f64>,
}

impl Spine {
    pub fn new(path: &PathData) -> Option<Self> {
        let sp = path.subpaths.first()?;
        let pts = flatten_subpath(sp, 0.1);
        if pts.len() < 2 {
            return None;
        }
        let mut cum = vec![0.0];
        let mut total = 0.0;
        for w in pts.windows(2) {
            total += w[0].distance(w[1]);
            cum.push(total);
        }
        Some(Self { pts, cum })
    }
    pub fn length(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }
    /// Point and tangent angle (radians) at arc-length fraction `f` (0..1).
    pub fn at(&self, f: f64) -> (Point, f64) {
        let total = self.length();
        let s = f.clamp(0.0, 1.0) * total;
        let i = match self.cum.binary_search_by(|c| c.total_cmp(&s)) {
            Ok(i) => i.min(self.pts.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.pts.len() - 2),
        };
        let seg = (self.cum[i + 1] - self.cum[i]).max(1e-12);
        let u = ((s - self.cum[i]) / seg).clamp(0.0, 1.0);
        let d = self.pts[i + 1] - self.pts[i];
        (self.pts[i].lerp(self.pts[i + 1], u), d.y.atan2(d.x))
    }
}

/// Evaluate a blend: the keys and generated steps in paint order.
pub fn blend_expand(keys: &[Arc<Node>], spec: &BlendSpec) -> Vec<Node> {
    let k = keys.len();
    if k < 2 {
        return keys.iter().map(|n| (**n).clone()).collect();
    }
    let spine = spec.spine.as_ref().and_then(Spine::new);
    let place = |mut n: Node, f: f64| -> Node {
        if let Some(sp) = &spine {
            let (p, ang) = sp.at(f);
            let c = center_of(&n);
            let rot = if spec.orientation == BlendOrientation::AlignToPath { Affine::rotate(ang) } else { Affine::IDENTITY };
            n.transform(Affine::translate(p.to_vec2()) * rot * Affine::translate(-c.to_vec2()), false);
        }
        n
    };
    let mut out = Vec::new();
    for i in 0..k {
        let a = &keys[i];
        out.push(place((**a).clone(), i as f64 / (k - 1) as f64));
        let Some(b) = keys.get(i + 1) else { break };
        let len = match &spine {
            Some(sp) => sp.length() / (k - 1) as f64,
            None => center_of(a).distance(center_of(b)),
        };
        let n = blend_step_count(a, b, spec.spacing, len);
        for j in 1..=n {
            let t = j as f64 / (n + 1) as f64;
            let mut s = lerp_node(a, b, t);
            s.id = NodeId(0);
            out.push(place(s, (i as f64 + t) / (k - 1) as f64));
        }
    }
    out
}

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
        {
            res[0].h_in = h;
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
    let mut cum = vec![0.0];
    let mut total = 0.0;
    for w in poly.windows(2) {
        total += w[0].distance(w[1]);
        cum.push(total);
    }
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
        if poly.len() > 1 && poly.first().zip(poly.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-9) {
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

    fn sample(side: &[Point], u: f64) -> Point {
        let s = u.clamp(0.0, 1.0) * (side.len() - 1) as f64;
        let i = (s.floor() as usize).min(side.len() - 2);
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

impl GradientMesh {
    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * (self.cols as usize + 1) + c
    }
    pub fn is_valid(&self) -> bool {
        self.rows >= 1 && self.cols >= 1 && self.points.len() == (self.rows as usize + 1) * (self.cols as usize + 1)
    }

    /// A mesh over surface `s` (u, v ∈ [0,1]) with handles from its partial derivatives.
    pub fn from_surface(rows: u32, cols: u32, s: &dyn Fn(f64, f64) -> Point, color: &dyn Fn(f64, f64) -> Color) -> Self {
        let (rows, cols) = (rows.clamp(1, 200), cols.clamp(1, 200));
        let (du, dv) = (1.0 / cols as f64, 1.0 / rows as f64);
        let eps = 1e-3;
        let mut points = Vec::with_capacity(((rows + 1) * (cols + 1)) as usize);
        for r in 0..=rows {
            for c in 0..=cols {
                let (u, v) = (c as f64 * du, r as f64 * dv);
                let p = s(u, v);
                let d_u = (s((u + eps).min(1.0), v) - s((u - eps).max(0.0), v)) / ((u + eps).min(1.0) - (u - eps).max(0.0));
                let d_v = (s(u, (v + eps).min(1.0)) - s(u, (v - eps).max(0.0))) / ((v + eps).min(1.0) - (v - eps).max(0.0));
                let hu = d_u * (du / 3.0);
                let hv = d_v * (dv / 3.0);
                // Outward handles on the border are unused: keep them at the point.
                let z = Vec2::ZERO;
                let handles =
                    [if c < cols { hu } else { z }, if c > 0 { -hu } else { z }, if r < rows { hv } else { z }, if r > 0 { -hv } else { z }];
                points.push(MeshPoint { p, color: color(u, v), opacity: 1.0, handles });
            }
        }
        Self { rows, cols, points }
    }

    /// Create Gradient Mesh for a path: the outline split at its corners, coloured per `appearance`.
    pub fn for_path(path: &PathData, rows: u32, cols: u32, base: Color, appearance: MeshAppearance, highlight: f64) -> Option<Self> {
        let coons = Coons::from_path(path)?;
        let h = (highlight / 100.0).clamp(0.0, 1.0) as f32;
        let color = |u: f64, v: f64| -> Color {
            let d = ((u - 0.5).abs().max((v - 0.5).abs()) * 2.0) as f32; // 0 centre … 1 edge
            let w = match appearance {
                MeshAppearance::Flat => 0.0,
                MeshAppearance::ToCenter => (1.0 - d) * h,
                MeshAppearance::ToEdge => d * h,
            };
            if w > 0.0 { lerp_color(&base, &Color::WHITE, w) } else { base }
        };
        let s = |u: f64, v: f64| coons.eval(u, v);
        Some(Self::from_surface(rows, cols, &s, &color))
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
/// extrapolation at the borders so an undistorted grid is the identity).
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
    let su = (u.clamp(0.0, 1.0) * cols as f64).min(cols as f64 - 1e-9);
    let sv = (v.clamp(0.0, 1.0) * rows as f64).min(rows as f64 - 1e-9);
    let (ci, ri) = (su.floor() as i64, sv.floor() as i64);
    let (fu, fv) = (su - ci as f64, sv - ri as f64);
    let mut rowsv = [Vec2::ZERO; 4];
    for (k, rv) in rowsv.iter_mut().enumerate() {
        let r = ri - 1 + k as i64;
        *rv = catmull(at(r, ci - 1), at(r, ci), at(r, ci + 1), at(r, ci + 2), fu);
    }
    catmull(rowsv[0], rowsv[1], rowsv[2], rowsv[3], fv).to_point()
}

/// The envelope's point map for content whose bounding box is `src`.
pub fn envelope_mapper<'a>(kind: &'a EnvelopeKind, src: Rect) -> Box<dyn Fn(Point) -> Point + 'a> {
    let w = src.width().max(1e-9);
    let h = src.height().max(1e-9);
    match kind {
        EnvelopeKind::Warp { style, bend, h: dh, v: dv, horizontal } => {
            let st = WarpStyle::from_id(style).unwrap_or(WarpStyle::Arc);
            let b = bend.clamp(-100.0, 100.0) / 100.0;
            let (dh, dv) = (dh.clamp(-100.0, 100.0) / 100.0, dv.clamp(-100.0, 100.0) / 100.0);
            let c = src.center();
            let (hw, hh) = (w / 2.0, h / 2.0);
            let horizontal = *horizontal;
            Box::new(move |q: Point| {
                let (x, y) = ((q.x - c.x) / hw, (q.y - c.y) / hh);
                let (x2, y2) = if horizontal {
                    warp_point(st, b, dh, dv, x, y)
                } else {
                    let (a, b2) = warp_point(st, b, dh, dv, y, x);
                    (b2, a)
                };
                Point::new(c.x + x2 * hw, c.y + y2 * hh)
            })
        }
        EnvelopeKind::Mesh { rows, cols, points } => {
            let (rows, cols) = (*rows as usize, *cols as usize);
            if rows == 0 || cols == 0 || points.len() != (rows + 1) * (cols + 1) {
                return Box::new(|q| q);
            }
            Box::new(move |q: Point| grid_eval(rows, cols, points, (q.x - src.x0) / w, (q.y - src.y0) / h))
        }
        EnvelopeKind::TopObject { path } => match Coons::from_path(path) {
            Some(co) => Box::new(move |q: Point| co.eval((q.x - src.x0) / w, (q.y - src.y0) / h)),
            None => Box::new(|q| q),
        },
    }
}

/// Optional hook that converts nodes the pure evaluation can't map (text) to outlines.
pub type Outliner<'a> = Option<&'a dyn Fn(&Node) -> Option<Node>>;

fn map_node(n: &Node, f: &dyn Fn(Point) -> Point, piece: f64, outline: Outliner) -> Node {
    let mut out = n.clone();
    match &mut out.kind {
        NodeKind::Path { path, live, .. } => {
            *path = map_nonlinear(path, piece, f);
            *live = None;
        }
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                *c = Arc::new(map_node(c, f, piece, outline));
            }
        }
        NodeKind::Mesh(m) => m.map(f),
        NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
            let g = expanded_group(n, outline);
            return map_node(&g, f, piece, outline);
        }
        NodeKind::Text(_) => {
            if let Some(o) = outline.and_then(|h| h(n)) {
                return map_node(&o, f, piece, outline);
            }
        }
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => {}
    }
    out
}

/// Evaluate an envelope: its content mapped through the envelope.
pub fn envelope_expand(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, outline: Outliner) -> Vec<Node> {
    let Some(src) = nodes_bounds(content) else { return content.iter().map(|c| (**c).clone()).collect() };
    let f = envelope_mapper(kind, src);
    let diag = src.width().hypot(src.height()).max(1e-6);
    let piece = diag / (8.0 + fidelity.clamp(0.0, 100.0) / 100.0 * 56.0);
    content.iter().map(|c| map_node(c, &*f, piece, outline)).collect()
}

/// Envelope bounds (the image of the content box, sampled).
pub fn envelope_bounds(content: &[Arc<Node>], kind: &EnvelopeKind) -> Option<Rect> {
    match kind {
        EnvelopeKind::Mesh { points, .. } => {
            let first = *points.first()?;
            Some(points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p)))
        }
        EnvelopeKind::TopObject { path } => path.bounds(),
        EnvelopeKind::Warp { .. } => {
            let src = nodes_bounds(content)?;
            let f = envelope_mapper(kind, src);
            const N: usize = 16;
            let mut r: Option<Rect> = None;
            for j in 0..=N {
                for i in 0..=N {
                    let q = f(Point::new(src.x0 + src.width() * i as f64 / N as f64, src.y0 + src.height() * j as f64 / N as f64));
                    r = Some(r.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
                }
            }
            r
        }
    }
}

// =====================================================================================
// Expansion
// =====================================================================================

/// Is this one of the live kinds?
pub fn is_live(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_))
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
    match &n.kind {
        NodeKind::Blend { children, spec } => blend_expand(children, spec),
        NodeKind::Envelope { content, kind, fidelity, .. } => envelope_expand(content, kind, *fidelity, outline),
        NodeKind::Mesh(m) => mesh_quad_nodes(m, 8),
        NodeKind::Repeat(r) => r.expand(),
        _ => vec![n.clone()],
    }
}

/// A live object evaluated into a plain group (keeping the object's id, name and transparency).
pub fn expanded_group(n: &Node, outline: Outliner) -> Node {
    let children = expand_live_with(n, outline).into_iter().map(Arc::new).collect();
    let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
    g.name = n.name.clone();
    g.visible = n.visible;
    g.locked = n.locked;
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.isolate = n.isolate;
    g.knockout = n.knockout;
    g.knockout_shape = n.knockout_shape;
    g
}

/// Recursively replace every live object in `n` by plain geometry (for exporters).
pub fn expand_deep(n: &Node, outline: Outliner) -> Node {
    let mut out = if is_live(n) { expanded_group(n, outline) } else { n.clone() };
    if let Some(ch) = out.children_mut() {
        for c in ch.iter_mut() {
            if subtree_has_live(c) {
                *c = Arc::new(expand_deep(c, outline));
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
