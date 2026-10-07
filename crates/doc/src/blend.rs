//! Live blends: interpolating key objects and laying the steps out along the spine.
//!
//! - **Paths** are matched anchor by anchor ([`PathPair`]): subpaths paired in order (a missing
//!   one grows out of a point), resampled to equal anchor counts, closed ones turned to the same
//!   winding and to the start points that keep them from twisting, or to the anchors the Blend
//!   tool clicked.
//! - **Paint**: colours, gradients (stops resampled to a common set when their counts differ),
//!   opacity, stroke weight, dashes, miter limit and width profiles interpolate; what can't
//!   (caps, joins, brushes, fonts, symbols, text content) switches halfway.
//! - **Groups** pair their children in stacking order; the children one group has more of grow
//!   out of the other group's centre. Compound paths stay compound paths.
//! - **Text, symbols and images** interpolate their transforms (rotation, scale, shear and
//!   position separately); text interpolates its colours and sizes run by run.
//! - **Spine** ([`blend_spine`]): the straight lines between the key centres until it is edited
//!   or replaced; then a path whose anchors the keys sit on (`key_anchors`). Steps follow it by
//!   arc length; Align to Path turns them to its direction.

use std::f64::consts::PI;
use std::sync::Arc;

use vectorcraft_color::{Color, Gradient, GradientPaint, GradientStop, Paint};
use vectorcraft_geom::kurbo::ParamCurveArclen;
use vectorcraft_geom::{Affine, Anchor, BezPath, FillRule, PathData, Point, SubPath, Vec2};

use crate::appearance::{Appearance, AppearanceItem, Dash, StrokeLayer, WidthProfile};
use crate::live::{BlendOrientation, BlendSpacing, BlendSpec};
use crate::node::{Node, NodeId, NodeKind};
use crate::text::TextObject;

// =====================================================================================
// Colour / paint interpolation
// =====================================================================================

/// Interpolate two colours in their shared model (display RGB when the models differ).
pub fn lerp_color(a: &Color, b: &Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t;
    match (*a, *b) {
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => Color::cmyk(l(c, c2), l(m, m2), l(y, y2), l(k, k2)),
        (Color::Gray { k }, Color::Gray { k: k2 }) => Color::gray(l(k, k2)),
        (Color::Lab { l: l1, a, b }, Color::Lab { l: l2, a: a2, b: b2 }) => Color::lab(l(l1, l2), l(a, a2), l(b, b2)),
        _ => a.lerp(b, t),
    }
}

fn lerp_f32(x: f32, y: f32, t: f32) -> f32 {
    x + (y - x) * t
}

fn lerp_f64(x: f64, y: f64, t: f64) -> f64 {
    x + (y - x) * t
}

fn solid_gradient(c: Color, like: &Gradient) -> Gradient {
    let mut g = like.clone();
    for s in &mut g.stops {
        s.color = c;
        s.opacity = 1.0;
    }
    g
}

/// `g` with its stops at `offsets`, each painting what `g` paints there (midpoints centred).
fn resample_stops(g: &Gradient, offsets: &[f32]) -> Gradient {
    let mut out = g.clone();
    out.stops = offsets
        .iter()
        .map(|&o| {
            let (color, opacity) = g.sample_with(o, lerp_color);
            GradientStop { opacity, ..GradientStop::new(o, color) }
        })
        .collect();
    out
}

/// Where a gradient changes: its stop offsets and its off-centre midpoints.
fn gradient_breaks(g: &Gradient, out: &mut Vec<f32>) {
    for (i, s) in g.stops.iter().enumerate() {
        out.push(s.offset);
        if let Some(n) = g.stops.get(i + 1)
            && (s.midpoint - 0.5).abs() > 1e-3
        {
            out.push(s.offset + (n.offset - s.offset) * s.midpoint);
        }
    }
}

/// Two gradients of one kind with the same number of stops: as they are when the counts match,
/// else both resampled at every place either changes.
fn common_stops(a: &Gradient, b: &Gradient) -> Option<(Gradient, Gradient)> {
    if a.kind != b.kind {
        return None;
    }
    if a.stops.len() == b.stops.len() {
        return Some((a.clone(), b.clone()));
    }
    if a.stops.is_empty() || b.stops.is_empty() {
        return None;
    }
    let mut offs = Vec::with_capacity(2 * (a.stops.len() + b.stops.len()));
    gradient_breaks(a, &mut offs);
    gradient_breaks(b, &mut offs);
    offs.retain(|o| o.is_finite());
    offs.sort_by(f32::total_cmp);
    offs.dedup_by(|x, y| (*x - *y).abs() < 1e-4);
    Some((resample_stops(a, &offs), resample_stops(b, &offs)))
}

fn lerp_gradient(a: &GradientPaint, b: &GradientPaint, t: f32) -> Option<GradientPaint> {
    let (ga, gb) = common_stops(&a.gradient, &b.gradient)?;
    let mut out = if t < 0.5 { a.clone() } else { b.clone() };
    out.gradient.stops = ga
        .stops
        .iter()
        .zip(&gb.stops)
        .map(|(sa, sb)| GradientStop {
            opacity: lerp_f32(sa.opacity, sb.opacity, t),
            midpoint: lerp_f32(sa.midpoint, sb.midpoint, t),
            ..GradientStop::new(lerp_f32(sa.offset, sb.offset, t), lerp_color(&sa.color, &sb.color, t))
        })
        .collect();
    out.angle = a.angle + (b.angle - a.angle) * t as f64;
    out.swatch = None;
    if let (Some(ga), Some(gb)) = (a.geom, b.geom) {
        let mut g = ga;
        g.start = ga.start.lerp(gb.start, t as f64);
        g.end = ga.end.lerp(gb.end, t as f64);
        g.aspect = ga.aspect + (gb.aspect - ga.aspect) * t as f64;
        g.focal = (ga.focal.is_some() || gb.focal.is_some()).then(|| ga.focal_point().lerp(gb.focal_point(), t as f64));
        out.geom = Some(g);
    } else {
        out.geom = None;
    }
    Some(out)
}

/// Interpolate paints: solid↔solid, gradients of one kind (stops resampled to a common set),
/// solid↔gradient; otherwise switch halfway.
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

/// Interpolate dash patterns. A solid stroke counts as the other's pattern with its gaps closed,
/// so dashes open out of a solid line; patterns of different lengths repeat to a common one.
fn lerp_dash(x: Option<&Dash>, y: Option<&Dash>, t: f64) -> Option<Dash> {
    let solid_like = |d: &Dash| Dash {
        pattern: d.pattern.chunks(2).flat_map(|c| [c.iter().sum::<f64>(), 0.0]).take(d.pattern.len().max(2)).collect(),
        offset: d.offset,
        align_corners: d.align_corners,
    };
    let (x, y) = match (x.filter(|d| d.is_dashed()), y.filter(|d| d.is_dashed())) {
        (None, None) => return None,
        (Some(d), None) => (d.clone(), solid_like(d)),
        (None, Some(d)) => (solid_like(d), d.clone()),
        (Some(a), Some(b)) => (a.clone(), b.clone()),
    };
    // An odd pattern repeats twice per period (dash, gap, dash / gap, dash, gap).
    let even = |p: &[f64]| if p.len() % 2 == 1 { p.repeat(2) } else { p.to_vec() };
    let (pa, pb) = (even(&x.pattern), even(&y.pattern));
    let n = if pa.is_empty() || pb.is_empty() { 0 } else { lcm(pa.len(), pb.len()).min(12) };
    let at = |p: &[f64], i: usize| p.get(i % p.len().max(1)).copied().unwrap_or(0.0);
    Some(Dash {
        pattern: (0..n).map(|i| lerp_f64(at(&pa, i), at(&pb, i), t)).collect(),
        offset: lerp_f64(x.offset, y.offset, t),
        align_corners: if t < 0.5 { x.align_corners } else { y.align_corners },
    })
}

fn lcm(a: usize, b: usize) -> usize {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a.checked_div(x).map_or(0, |q| q * b)
}

/// Interpolate width profiles (none is uniform) at every position either has a width point.
fn lerp_profile(x: Option<&WidthProfile>, y: Option<&WidthProfile>, t: f64) -> Option<WidthProfile> {
    if x.is_none() && y.is_none() {
        return None;
    }
    let uniform = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
    let (x, y) = (x.unwrap_or(&uniform), y.unwrap_or(&uniform));
    let mut at: Vec<f64> = x.points.iter().chain(&y.points).map(|p| p.0).filter(|p| p.is_finite()).collect();
    at.sort_by(f64::total_cmp);
    at.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let points = at
        .into_iter()
        .map(|p| {
            let ((l1, r1), (l2, r2)) = (x.at(p), y.at(p));
            (p, lerp_f64(l1, l2, t), lerp_f64(r1, r2, t))
        })
        .collect();
    Some(WidthProfile { points })
}

/// Interpolate a stroke's paint, weight, opacity, miter limit, dashes, arrowhead scale and width
/// profile into `o` (a copy of the nearer stroke, so caps, joins, alignment, arrowheads and
/// brushes switch halfway).
fn lerp_stroke(o: &mut StrokeLayer, x: &StrokeLayer, y: &StrokeLayer, t: f64) {
    let tf = t as f32;
    o.paint = lerp_paint(&x.paint, &y.paint, tf);
    o.width = lerp_f64(x.width, y.width, t);
    o.opacity = lerp_f32(x.opacity, y.opacity, tf);
    o.miter_limit = lerp_f64(x.miter_limit, y.miter_limit, t);
    o.dash = lerp_dash(x.dash.as_ref(), y.dash.as_ref(), t);
    o.arrow_scale = (lerp_f64(x.arrow_scale.0, y.arrow_scale.0, t), lerp_f64(x.arrow_scale.1, y.arrow_scale.1, t));
    o.profile = lerp_profile(x.profile.as_ref(), y.profile.as_ref(), t);
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
        for ((it, x), y) in out.items.iter_mut().zip(&a.items).zip(&b.items) {
            match (it, x, y) {
                (AppearanceItem::Fill(o), AppearanceItem::Fill(x), AppearanceItem::Fill(y)) => {
                    o.paint = lerp_paint(&x.paint, &y.paint, tf);
                    o.opacity = lerp_f32(x.opacity, y.opacity, tf);
                }
                (AppearanceItem::Stroke(o), AppearanceItem::Stroke(x), AppearanceItem::Stroke(y)) => lerp_stroke(o, x, y, t),
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
        let (xs, ys) = (a.stroke().cloned(), b.stroke().cloned());
        if let Some(s) = out.stroke_mut() {
            if let (Some(x), Some(y)) = (&xs, &ys) {
                lerp_stroke(s, x, y, t);
            }
            s.width = w;
        }
    }
    out
}

// =====================================================================================
// Transforms
// =====================================================================================

/// An affine split into translation, rotation, x scale, shear and y scale (`R · [sx sh; 0 sy]`).
fn decompose(m: Affine) -> [f64; 6] {
    let [a, b, c, d, e, f] = m.as_coeffs();
    let sx = a.hypot(b);
    let ang = if sx > 1e-12 { b.atan2(a) } else { 0.0 };
    let (s, co) = ang.sin_cos();
    [e, f, ang, sx, co * c + s * d, -s * c + co * d]
}

fn compose([e, f, ang, sx, sh, sy]: [f64; 6]) -> Affine {
    let (s, co) = ang.sin_cos();
    Affine::new([co * sx, s * sx, co * sh - s * sy, s * sh + co * sy, e, f])
}

/// Interpolate two transforms by their parts: the rotation takes the shorter way round.
pub fn lerp_affine(x: Affine, y: Affine, t: f64) -> Affine {
    let (p, mut q) = (decompose(x), decompose(y));
    let turn = q[2] - p[2];
    q[2] = p[2] + (turn + PI).rem_euclid(2.0 * PI) - PI;
    let mut out = [0.0; 6];
    for (o, (a, b)) in out.iter_mut().zip(p.iter().zip(&q)) {
        *o = lerp_f64(*a, *b, t);
    }
    compose(out)
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

/// Start `sp` at anchor `i`: a closed subpath turns round to it, an open one clicked at its last
/// anchor runs the other way (a click on any other anchor of an open path is ignored).
fn start_at(sp: &mut SubPath, i: usize) {
    let n = sp.anchors.len();
    if i >= n {
        return;
    }
    if sp.closed {
        sp.anchors.rotate_left(i);
    } else if i + 1 == n && n > 1 {
        sp.reverse();
    }
}

/// Reverse a closed subpath keeping its start anchor first.
fn reverse_closed(sp: &mut SubPath) {
    sp.reverse();
    sp.anchors.rotate_right(1);
}

/// Anchor positions of a subpath relative to its box, scaled to unit size.
fn normalised(sp: &SubPath) -> Vec<Vec2> {
    let pts = sp.anchors.iter().map(|a| a.p);
    let (mut lo, mut hi) = (Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN));
    for p in pts.clone() {
        lo = Point::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Point::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let c = lo.midpoint(hi);
    let s = (hi.x - lo.x).max(hi.y - lo.y).max(1e-9);
    pts.map(|p| (p - c) / s).collect()
}

/// The rotation of `b`'s anchors that best matches `a`'s (least squared distance between
/// corresponding anchors, each shape centred and scaled to unit size): the start point that keeps
/// a closed blend from twisting. Long subpaths try every few rotations first, then refine.
fn best_rotation(a: &SubPath, b: &SubPath) -> usize {
    let (qa, qb) = (normalised(a), normalised(b));
    let m = qa.len().min(qb.len());
    if m < 2 {
        return 0;
    }
    let cost = |s: usize| -> f64 { (0..m).map(|j| (qa[j] - qb[(j + s) % m]).hypot2()).sum() };
    let stride = m.div_ceil(512);
    let mut best = (0..m).step_by(stride).min_by(|x, y| cost(*x).total_cmp(&cost(*y))).unwrap_or(0);
    if stride > 1 {
        best = (best + m - stride..=best + m + stride).map(|s| s % m).min_by(|x, y| cost(*x).total_cmp(&cost(*y))).unwrap_or(best);
    }
    best
}

/// Two paths prepared for interpolation: subpaths paired in order, with equal anchor counts, the
/// same winding and matching start points.
#[derive(Clone, Debug)]
pub struct PathPair {
    a: Vec<SubPath>,
    b: Vec<SubPath>,
}

impl PathPair {
    /// `starts`: the anchor of each path's first subpath its blend starts from (the Blend tool's
    /// clicks); with neither, closed subpaths start where they twist least.
    pub fn new(a: &PathData, b: &PathData, starts: (Option<usize>, Option<usize>)) -> Self {
        let n = a.subpaths.len().max(b.subpaths.len());
        let ca = a.bounds().map(|r| r.center()).unwrap_or_default();
        let cb = b.bounds().map(|r| r.center()).unwrap_or_default();
        let degenerate = |like: &SubPath, c: Point| SubPath::new(vec![Anchor::corner(c); like.anchors.len().max(1)], like.closed);
        let (mut va, mut vb) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for i in 0..n {
            let (mut sa, mut sb) = match (a.subpaths.get(i), b.subpaths.get(i)) {
                (Some(x), Some(y)) => (x.clone(), y.clone()),
                (Some(x), None) => (x.clone(), degenerate(x, cb)),
                (None, Some(y)) => (degenerate(y, ca), y.clone()),
                (None, None) => continue,
            };
            let explicit = i == 0 && (starts.0.is_some() || starts.1.is_some());
            if explicit {
                if let Some(s) = starts.0 {
                    start_at(&mut sa, s);
                }
                if let Some(s) = starts.1 {
                    start_at(&mut sb, s);
                }
            }
            let closed = sa.closed && sb.closed && sa.anchors.len() > 2 && sb.anchors.len() > 2;
            if closed && (sa.area() * sb.area()) < 0.0 {
                reverse_closed(&mut sb);
            }
            let m = sa.anchors.len().max(sb.anchors.len());
            grow(&mut sa, m);
            grow(&mut sb, m);
            if closed && !explicit {
                let r = best_rotation(&sa, &sb);
                sb.anchors.rotate_left(r);
            }
            va.push(sa);
            vb.push(sb);
        }
        Self { a: va, b: vb }
    }

    /// The path at `t` (0 = `a`, 1 = `b`).
    pub fn at(&self, t: f64) -> PathData {
        let subs = self
            .a
            .iter()
            .zip(&self.b)
            .map(|(sa, sb)| {
                let anchors = sa.anchors.iter().zip(&sb.anchors).map(|(x, y)| lerp_anchor(x, y, t)).collect();
                SubPath::new(anchors, if t < 0.5 { sa.closed } else { sb.closed })
            })
            .collect();
        PathData::new(subs)
    }

    /// Subpaths at `t`, in order (see [`Self::at`]).
    fn len(&self) -> usize {
        self.a.len()
    }
}

/// Interpolate two paths anchor-by-anchor (see [`PathPair`]).
pub fn lerp_path(a: &PathData, b: &PathData, t: f64) -> PathData {
    PathPair::new(a, b, (None, None)).at(t)
}

/// Path data of a path or compound path node (compound children concatenated), its fill rule and
/// the number of subpaths each compound member holds.
pub(crate) fn node_path(n: &Node) -> Option<(PathData, FillRule, Option<Vec<usize>>)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule, None)),
        NodeKind::Compound { children, rule } => {
            let members: Vec<&PathData> = children.iter().filter_map(|c| c.path_data()).collect();
            let counts = members.iter().map(|p| p.subpaths.len()).collect();
            let subs = members.iter().flat_map(|p| p.subpaths.iter().cloned()).collect();
            Some((PathData::new(subs), *rule, Some(counts)))
        }
        _ => None,
    }
}

// =====================================================================================
// Object interpolation
// =====================================================================================

/// How two key objects interpolate.
#[derive(Clone, Debug)]
enum Shape {
    /// Groups of one kind: children paired in stacking order.
    Group { children: Vec<Lerp>, clip: bool },
    /// Gradient meshes with the same grid.
    Mesh,
    /// Paths and compound paths; `members`: the subpath count of each member when both keys are
    /// compound paths.
    Path { pair: PathPair, rules: (FillRule, FillRule), members: Option<(Vec<usize>, Vec<usize>)> },
    /// Text, symbols and images: their transforms interpolate.
    Placed,
    /// Anything else: a copy of the nearer key moved and scaled into the interpolated box.
    Other,
}

/// Two key objects prepared for interpolation, so each step of a blend only interpolates.
#[derive(Clone, Debug)]
pub struct Lerp {
    a: Node,
    b: Node,
    shape: Shape,
}

/// `n` shrunk to a point at `c`: the counterpart of an object only one of two groups has.
fn collapsed(n: &Node, c: Point) -> Node {
    let mut x = n.clone();
    if let Some(b) = n.geometric_bounds() {
        x.transform(Affine::translate(c.to_vec2()) * Affine::scale(1e-6) * Affine::translate(-b.center().to_vec2()), false);
    }
    x
}

fn center_of(n: &Node) -> Point {
    n.geometric_bounds().map(|b| b.center()).unwrap_or_default()
}

impl Lerp {
    /// Prepare `a` → `b`; `starts` as in [`PathPair::new`].
    pub fn new(a: &Node, b: &Node, starts: (Option<usize>, Option<usize>)) -> Self {
        let shape = match (&a.kind, &b.kind) {
            (NodeKind::Group { children: ca, clip: k1 }, NodeKind::Group { children: cb, clip: k2 })
                if k1 == k2 && !(*k1 && ca.len() != cb.len()) =>
            {
                let (oa, ob) = (center_of(a), center_of(b));
                let n = ca.len().max(cb.len());
                let children = (0..n)
                    .filter_map(|i| match (ca.get(i), cb.get(i)) {
                        (Some(x), Some(y)) => Some(Lerp::new(x, y, (None, None))),
                        (Some(x), None) => Some(Lerp::new(x, &collapsed(x, ob), (None, None))),
                        (None, Some(y)) => Some(Lerp::new(&collapsed(y, oa), y, (None, None))),
                        (None, None) => None,
                    })
                    .collect();
                Shape::Group { children, clip: *k1 }
            }
            (NodeKind::Mesh(ma), NodeKind::Mesh(mb)) if ma.rows == mb.rows && ma.cols == mb.cols && ma.points.len() == mb.points.len() => Shape::Mesh,
            (NodeKind::Text(_), NodeKind::Text(_))
            | (NodeKind::SymbolInstance { .. }, NodeKind::SymbolInstance { .. })
            | (NodeKind::Image(_), NodeKind::Image(_)) => Shape::Placed,
            _ => match (node_path(a), node_path(b)) {
                (Some((pa, ra, ma)), Some((pb, rb, mb))) => {
                    Shape::Path { pair: PathPair::new(&pa, &pb, starts), rules: (ra, rb), members: ma.zip(mb) }
                }
                _ => Shape::Other,
            },
        };
        Self { a: a.clone(), b: b.clone(), shape }
    }

    /// The object at `t` (0 = `a`, 1 = `b`), with id 0.
    pub fn at(&self, t: f64) -> Node {
        self.at_with(t, t)
    }

    /// The object at `t` between the keys, its appearance (colours, opacity) at `tc` instead.
    pub fn at_with(&self, t: f64, tc: f64) -> Node {
        let (a, b) = (&self.a, &self.b);
        let base = if t < 0.5 { a } else { b };
        let mut n = match &self.shape {
            Shape::Group { children, clip } => {
                let mut g =
                    Node::new(NodeId(0), NodeKind::Group { children: children.iter().map(|c| Arc::new(c.at_with(t, tc))).collect(), clip: *clip });
                g.isolate = base.isolate;
                g.knockout = base.knockout;
                g
            }
            Shape::Mesh => {
                let mut n = base.clone();
                if let (NodeKind::Mesh(m), NodeKind::Mesh(ma), NodeKind::Mesh(mb)) = (&mut n.kind, &a.kind, &b.kind) {
                    for ((p, x), y) in m.points.iter_mut().zip(&ma.points).zip(&mb.points) {
                        p.p = x.p.lerp(y.p, t);
                        p.color = lerp_color(&x.color, &y.color, tc as f32);
                        p.opacity = lerp_f32(x.opacity, y.opacity, tc as f32);
                        for h in 0..4 {
                            p.handles[h] = x.handles[h].lerp(y.handles[h], t);
                        }
                    }
                }
                n.mask = None;
                n
            }
            Shape::Path { pair, rules, members } => {
                let path = pair.at(t);
                let rule = if t < 0.5 { rules.0 } else { rules.1 };
                match members {
                    Some((ma, mb)) => compound(path, rule, if t < 0.5 { ma } else { mb }, pair.len()),
                    None => {
                        let mut n = Node::path(NodeId(0), path, Appearance::default());
                        if let NodeKind::Path { rule: r, .. } = &mut n.kind {
                            *r = rule;
                        }
                        n
                    }
                }
            }
            Shape::Placed => {
                let mut n = base.clone();
                match (&mut n.kind, &a.kind, &b.kind) {
                    (NodeKind::Text(o), NodeKind::Text(x), NodeKind::Text(y)) => lerp_text(o, x, y, t),
                    (NodeKind::SymbolInstance { xf: o, .. }, NodeKind::SymbolInstance { xf: x, .. }, NodeKind::SymbolInstance { xf: y, .. }) => {
                        *o = lerp_affine(*x, *y, t)
                    }
                    (NodeKind::Image(o), NodeKind::Image(x), NodeKind::Image(y)) => o.xf = lerp_affine(x.xf, y.xf, t),
                    _ => {}
                }
                n.mask = None;
                n
            }
            Shape::Other => {
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
                // A copy's members are new objects too.
                for c in n.children_mut().into_iter().flatten() {
                    zero_ids(Arc::make_mut(c));
                }
                n.mask = None;
                n
            }
        };
        n.id = NodeId(0);
        n.appearance = lerp_appearance(&a.appearance, &b.appearance, tc);
        n.opacity = lerp_f32(a.opacity, b.opacity, tc as f32);
        n.blend = base.blend;
        n.visible = true;
        n.locked = false;
        n.name = None;
        n
    }
}

/// `n` and its descendants with id 0 (generated objects).
fn zero_ids(n: &mut Node) {
    n.id = NodeId(0);
    for c in n.children_mut().into_iter().flatten() {
        zero_ids(Arc::make_mut(c));
    }
}

/// A compound path of `path`'s subpaths, split into members as `counts` says (one member per
/// subpath when the counts don't add up to `total`).
fn compound(path: PathData, rule: FillRule, counts: &[usize], total: usize) -> Node {
    let mut subs = path.subpaths.into_iter();
    let counts: Vec<usize> = if counts.iter().sum::<usize>() == total && !counts.contains(&0) { counts.to_vec() } else { vec![1; total] };
    let children =
        counts.iter().map(|&c| Arc::new(Node::path(NodeId(0), PathData::new(subs.by_ref().take(c).collect()), Appearance::default()))).collect();
    Node::new(NodeId(0), NodeKind::Compound { children, rule })
}

/// Interpolate text `x` → `y` into `o` (a copy of the nearer): the transform by its parts, an
/// area frame anchor by anchor, and each run's colours, stroke weight and size from the runs at
/// the same place (by character) of both.
fn lerp_text(o: &mut TextObject, x: &TextObject, y: &TextObject, t: f64) {
    o.xf = lerp_affine(x.xf, y.xf, t);
    if let (crate::text::TextKind::Area { frame: f }, crate::text::TextKind::Area { frame: fx }, crate::text::TextKind::Area { frame: fy }) =
        (&mut o.kind, &x.kind, &y.kind)
    {
        *f = lerp_path(fx, fy, t);
    }
    // The run of `of` at fraction `u` of its characters.
    fn run_at(of: &TextObject, u: f64) -> Option<&crate::text::TextRun> {
        let total: usize = of.runs.iter().map(|r| r.text.chars().count()).sum();
        let target = (u * total as f64) as usize;
        let mut seen = 0;
        for r in &of.runs {
            seen += r.text.chars().count();
            if seen > target {
                return Some(r);
            }
        }
        of.runs.last()
    }
    let total: usize = o.runs.iter().map(|r| r.text.chars().count()).sum::<usize>().max(1);
    let mut start = 0usize;
    let tf = t as f32;
    let mut resized = false;
    for r in &mut o.runs {
        let len = r.text.chars().count();
        let u = (start as f64 + len as f64 / 2.0) / total as f64;
        start += len;
        let (Some(rx), Some(ry)) = (run_at(x, u), run_at(y, u)) else { continue };
        let (sx, sy) = (&rx.style, &ry.style);
        r.style.fill = lerp_paint(&sx.fill, &sy.fill, tf);
        r.style.stroke = lerp_paint(&sx.stroke, &sy.stroke, tf);
        r.style.stroke_width = lerp_f64(sx.stroke_width, sy.stroke_width, t);
        let size = lerp_f64(sx.size, sy.size, t);
        resized |= size != r.style.size;
        r.style.size = size;
    }
    if resized {
        o.cached_bounds = None;
    }
}

/// Interpolate two objects at `t` (0 = `a`, 1 = `b`). A blend prepares each pair once with
/// [`Lerp`] instead.
pub fn lerp_node(a: &Node, b: &Node, t: f64) -> Node {
    Lerp::new(a, b, (None, None)).at(t)
}

// =====================================================================================
// Step counts
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

// =====================================================================================
// Spine
// =====================================================================================

/// A spine (the first subpath of a path) flattened to a polyline, with cumulative lengths and the
/// arc length at each of its anchors.
pub struct Spine {
    pts: Vec<Point>,
    cum: Vec<f64>,
    anchors: Vec<f64>,
    closed: bool,
}

impl Spine {
    pub fn new(path: &PathData) -> Option<Self> {
        let sp = path.subpaths.first()?;
        let mut pts: Vec<Point> = Vec::new();
        let mut cum: Vec<f64> = Vec::new();
        let mut anchors = Vec::with_capacity(sp.anchors.len());
        for i in 0..sp.segment_count() {
            anchors.push(cum.last().copied().unwrap_or(0.0));
            let c = sp.segment(i);
            let mut bp = BezPath::new();
            bp.move_to(c.p0);
            if sp.segment_is_line(i) {
                bp.line_to(c.p3);
            } else {
                bp.curve_to(c.p1, c.p2, c.p3);
            }
            vectorcraft_geom::kurbo::flatten(bp.iter(), 0.1, |el| {
                if let vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) = el
                    && p.x.is_finite()
                    && p.y.is_finite()
                    && pts.last().is_none_or(|q| q.distance(p) > 1e-9)
                {
                    let s = match (pts.last(), cum.last()) {
                        (Some(q), Some(l)) => l + q.distance(p),
                        _ => 0.0,
                    };
                    pts.push(p);
                    cum.push(s);
                }
            });
        }
        if !sp.closed {
            anchors.push(cum.last().copied().unwrap_or(0.0));
        }
        if pts.len() < 2 {
            return None;
        }
        Some(Self { pts, cum, anchors, closed: sp.closed })
    }
    pub fn length(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }
    /// Arc length at anchor `i`.
    pub fn anchor_length(&self, i: usize) -> Option<f64> {
        self.anchors.get(i).copied()
    }
    /// Point and tangent angle (radians) at arc-length fraction `f` (0..1).
    pub fn at(&self, f: f64) -> (Point, f64) {
        self.at_length(f.clamp(0.0, 1.0) * self.length())
    }
    /// Point and tangent angle (radians) at arc length `s` (clamped to the spine).
    pub fn at_length(&self, s: f64) -> (Point, f64) {
        let last = self.pts.len().saturating_sub(2);
        let s = if s.is_finite() { s.clamp(0.0, self.length()) } else { 0.0 };
        let i = match self.cum.binary_search_by(|c| c.total_cmp(&s)) {
            Ok(i) => i.min(last),
            Err(i) => i.saturating_sub(1).min(last),
        };
        let (Some(p0), Some(p1), Some(c0), Some(c1)) = (self.pts.get(i), self.pts.get(i + 1), self.cum.get(i), self.cum.get(i + 1)) else {
            return (self.pts.first().copied().unwrap_or_default(), 0.0);
        };
        let u = ((s - c0) / (c1 - c0).max(1e-12)).clamp(0.0, 1.0);
        let d = *p1 - *p0;
        (p0.lerp(*p1, u), d.y.atan2(d.x))
    }
}

/// The anchor of the first subpath each key sits on, when they are all valid for `n` anchors.
fn pinned(spec: &BlendSpec, keys: usize, n: usize) -> Option<Vec<usize>> {
    (spec.key_anchors.len() == keys && keys >= 2 && spec.key_anchors.iter().all(|a| (*a as usize) < n))
        .then(|| spec.key_anchors.iter().map(|a| *a as usize).collect())
}

/// The spine of a blend of `keys` and the anchor of it each key sits on:
/// - without a stored spine: the straight lines between the key centres;
/// - a pinned spine (`key_anchors`): the stored spine's first subpath with each key's anchor (and
///   its handles) moved onto the key's centre, so moving a key moves its end of the spine;
/// - a spine from Replace Spine: its first subpath, `None` for the anchors (the keys spread
///   evenly along it by arc length).
pub fn blend_spine(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<(PathData, Option<Vec<usize>>)> {
    if keys.len() < 2 {
        return None;
    }
    let Some(stored) = &spec.spine else {
        let centers: Vec<Point> = keys.iter().map(|k| center_of(k)).collect();
        return Some((PathData::single(SubPath::polyline(&centers, false)), Some((0..keys.len()).collect())));
    };
    let mut first = stored.subpaths.first()?.clone();
    let Some(ka) = pinned(spec, keys.len(), first.anchors.len()) else { return Some((PathData::single(first), None)) };
    for (k, &ai) in keys.iter().zip(&ka) {
        let c = center_of(k);
        if let Some(a) = first.anchors.get_mut(ai) {
            let d = c - a.p;
            a.translate(d);
        }
    }
    Some((PathData::single(first), Some(ka)))
}

/// [`blend_spine`] with every key on an anchor: a spine from Replace Spine gets anchors where its
/// keys sit (by arc length), so editing it keeps them in place.
pub fn pin_spine(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<(PathData, Vec<usize>)> {
    let (mut path, anchors) = blend_spine(keys, spec)?;
    if let Some(a) = anchors {
        return Some((path, a));
    }
    let k = keys.len();
    let total = Spine::new(&path)?.length();
    let closed = path.subpaths.first().is_some_and(|s| s.closed);
    let mut out = vec![0usize];
    for i in 1..k.saturating_sub(1) {
        let s = total * i as f64 / (k - 1) as f64;
        out.push(anchor_at_length(&mut path, s)?);
    }
    let last = path.subpaths.first().map_or(0, |sp| if closed { 0 } else { sp.anchors.len().saturating_sub(1) });
    out.push(last);
    Some((path, out))
}

/// The anchor of the first subpath at arc length `s`: an existing one within a hundredth of a
/// point, else a new one splitting the segment there.
fn anchor_at_length(path: &mut PathData, s: f64) -> Option<usize> {
    let spine = Spine::new(path)?;
    let sp = path.subpaths.first_mut()?;
    let n = sp.segment_count();
    let seg = (0..n).rev().find(|i| spine.anchor_length(*i).is_some_and(|l| l <= s)).unwrap_or(0);
    let from = spine.anchor_length(seg).unwrap_or(0.0);
    let to = spine.anchor_length(seg + 1).unwrap_or(spine.length());
    if s - from < 0.01 {
        return Some(seg);
    }
    if to - s < 0.01 {
        return Some((seg + 1) % sp.anchors.len().max(1));
    }
    let c = sp.segment(seg);
    // The flattened length and the curve's own differ slightly: scale into the curve's.
    let arc = c.arclen(1e-4).max(1e-12);
    let t = c.inv_arclen((s - from) / (to - from).max(1e-12) * arc, 1e-4).clamp(1e-4, 1.0 - 1e-4);
    Some(sp.insert_anchor(seg, t))
}

/// Where a blend's keys and steps go along its spine (see [`blend_spine`]).
struct Rail {
    spine: Spine,
    /// Arc length of each key.
    stops: Vec<f64>,
    centers: Vec<Point>,
    rotate: bool,
}

impl Rail {
    /// `None` when nothing moves: no stored spine and the steps keep the page's orientation.
    fn new(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<Self> {
        let rotate = spec.orientation == BlendOrientation::AlignToPath;
        if spec.spine.is_none() && !rotate {
            return None;
        }
        let (path, anchors) = blend_spine(keys, spec)?;
        let spine = Spine::new(&path)?;
        let total = spine.length();
        let k = keys.len();
        let stops = match anchors {
            Some(ka) => {
                let mut v: Vec<f64> = Vec::with_capacity(k);
                for a in ka {
                    let mut s = spine.anchor_length(a).unwrap_or(0.0);
                    // On a closed spine the key back at the start sits at its far end.
                    if spine.closed
                        && let Some(prev) = v.last()
                        && s <= *prev + 1e-9
                    {
                        s += total;
                    }
                    v.push(s);
                }
                v
            }
            None => (0..k).map(|i| total * i as f64 / (k - 1).max(1) as f64).collect(),
        };
        Some(Self { spine, stops, centers: keys.iter().map(|k| center_of(k)).collect(), rotate })
    }

    /// Arc length between keys `i` and `i + 1`.
    fn len(&self, i: usize) -> f64 {
        match (self.stops.get(i), self.stops.get(i + 1)) {
            (Some(a), Some(b)) => (b - a).abs(),
            _ => 0.0,
        }
    }

    /// Move `n`, the object at `t` between keys `i` and `i + 1` (a key at `t` = 0), onto the
    /// spine: by how far the spine there is from the straight line between the key centres (so
    /// steps keep their own interpolated position on a straight spine), turned to the spine's
    /// direction with Align to Path.
    fn place(&self, n: &mut Node, i: usize, t: f64) {
        let (Some(&s0), Some(&c0)) = (self.stops.get(i), self.centers.get(i)) else { return };
        let (s, base) = match (self.stops.get(i + 1), self.centers.get(i + 1)) {
            (Some(&s1), Some(&c1)) if t > 0.0 => (lerp_f64(s0, s1, t), c0.lerp(c1, t)),
            _ => (s0, c0),
        };
        let (p, ang) = self.spine.at_length(if self.spine.closed { s.rem_euclid(self.spine.length().max(1e-12)) } else { s });
        let off = p - base;
        let mut m = if off.hypot() > 1e-9 { Affine::translate(off) } else { Affine::IDENTITY };
        if self.rotate && ang.abs() > 1e-12 {
            let c = (center_of(n) + off).to_vec2();
            m = Affine::translate(c) * Affine::rotate(ang) * Affine::translate(-c) * m;
        }
        if m != Affine::IDENTITY {
            n.transform(m, false);
        }
    }
}

// =====================================================================================
// Blend evaluation
// =====================================================================================

/// Whether `n`, every part of it, paints opaquely with Normal blending and no opacity mask or
/// effect: knocking such objects out of each other changes nothing.
fn paints_opaquely(n: &Node) -> bool {
    let paint = |p: &Paint| match p {
        Paint::Gradient(g) => g.gradient.stops.iter().all(|s| s.opacity >= 1.0),
        Paint::Pattern { .. } => false,
        _ => true,
    };
    n.opacity >= 1.0
        && n.blend == vectorcraft_color::BlendMode::Normal
        && n.mask.is_none()
        && n.appearance.effects.is_empty()
        && n.appearance
            .items
            .iter()
            .all(|i| i.opacity() >= 1.0 && i.blend() == vectorcraft_color::BlendMode::Normal && i.effects().is_empty() && paint(i.paint()))
        && n.children().is_none_or(|c| c.iter().all(|c| paints_opaquely(c)))
}

/// Whether knocking out the children of `n` (a blend's evaluated steps) shows: some step is
/// translucent or blends.
pub fn knockout_shows(n: &Node) -> bool {
    n.children().is_some_and(|c| !c.iter().all(|c| paints_opaquely(c)))
}

/// [`knockout_shows`] for evaluated steps.
pub fn steps_knockout_shows(steps: &[Arc<Node>]) -> bool {
    !steps.iter().all(|c| paints_opaquely(c))
}

/// Evaluate a blend: the keys and generated steps in paint order.
pub fn blend_expand(keys: &[Arc<Node>], spec: &BlendSpec) -> Vec<Node> {
    let k = keys.len();
    if k < 2 {
        return keys.iter().map(|n| (**n).clone()).collect();
    }
    let rail = Rail::new(keys, spec);
    let mut out = Vec::new();
    for (i, a) in keys.iter().enumerate() {
        let mut key = (**a).clone();
        if let Some(r) = &rail {
            r.place(&mut key, i, 0.0);
        }
        out.push(key);
        let Some(b) = keys.get(i + 1) else { break };
        let len = rail.as_ref().map_or_else(|| center_of(a).distance(center_of(b)), |r| r.len(i));
        let n = blend_step_count(a, b, spec.spacing, len);
        let lerp = Lerp::new(a, b, (spec.start(i), spec.start(i + 1)));
        for j in 1..=n {
            let (t, tc) = spec.eased(j as f64 / (n + 1) as f64);
            let mut s = lerp.at_with(t, tc);
            if let Some(r) = &rail {
                r.place(&mut s, i, t);
            }
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{BlendEase, BlendEasing};
    use vectorcraft_geom::{Rect, shapes};

    #[test]
    fn easing_bunches_steps_and_colour_eases_on_its_own() {
        let a = key(0.0);
        let b = key(100.0);
        let centres = |spec: &BlendSpec| -> Vec<f64> {
            blend_expand(&[a.clone(), b.clone()], spec).iter().map(|n| n.geometric_bounds().unwrap().center().x).collect()
        };
        let even = BlendSpec { spacing: BlendSpacing::Steps(3), ..Default::default() };
        let c = centres(&even);
        assert!((c[1] - c[0] - (c[2] - c[1])).abs() < 1e-6, "linear steps are even: {c:?}");
        let ease_in = BlendSpec { easing: BlendEasing { ease: BlendEase::EaseIn, strength: 100.0 }, ..even.clone() };
        let c = centres(&ease_in);
        assert!(c[2] - c[1] < c[3] - c[2], "ease in bunches the steps at the start: {c:?}");
        assert_eq!((c[0], c[4]), (centres(&even)[0], centres(&even)[4]), "the keys stay put");
        let ease_out = BlendSpec { easing: BlendEasing { ease: BlendEase::EaseOut, strength: 100.0 }, ..even.clone() };
        let c = centres(&ease_out);
        assert!(c[2] - c[1] > c[3] - c[2], "ease out bunches them at the end: {c:?}");
        // Colour acceleration: positions stay even, the colour reaches the end colour early.
        let spec = BlendSpec { color_easing: Some(BlendEasing { ease: BlendEase::EaseOut, strength: 100.0 }), ..even.clone() };
        let (t, tc) = spec.eased(0.5);
        assert_eq!(t, 0.5);
        assert!(tc > 0.8, "{tc}");
        for e in [BlendEase::Linear, BlendEase::EaseIn, BlendEase::EaseOut, BlendEase::EaseInOut] {
            let ez = BlendEasing { ease: e, strength: 70.0 };
            assert_eq!((ez.apply(0.0), ez.apply(1.0)), (0.0, 1.0));
            assert!((0..10).all(|i| ez.apply(i as f64 / 10.0) <= ez.apply((i + 1) as f64 / 10.0)), "{e:?} keeps the order");
            assert_eq!(BlendEase::parse(e.name()), Some(e));
        }
    }

    #[test]
    fn clicked_start_points_pair_up() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut b = shapes::rectangle(Rect::new(100.0, 0.0, 110.0, 10.0));
        b.subpaths[0].anchors.rotate_left(2);
        // Starting both at their anchor 0 pairs each corner of `a` with the opposite one of `b`:
        // the middle step shrinks to a point.
        assert!(PathPair::new(&a, &b, (Some(0), Some(0))).at(0.5).bounds().unwrap().height() < 1e-6);
        // Starting `b` at its anchor 2 (its first corner again) pairs like corners.
        let r = PathPair::new(&a, &b, (Some(0), Some(2))).at(0.5).bounds().unwrap();
        assert!((r.width() - 10.0).abs() < 1e-6 && (r.height() - 10.0).abs() < 1e-6, "{r:?}");
        // An open path clicked at its end runs the other way.
        let line = |x: f64| PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0)], false));
        let m = PathPair::new(&line(0.0), &line(100.0), (None, Some(1))).at(0.5);
        assert_eq!(m.subpaths[0].anchors[0].p, Point::new(55.0, 0.0));
    }

    fn key(x: f64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(1), vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default()))
    }

    #[test]
    fn a_replaced_spine_gets_anchors_where_its_keys_sit() {
        let keys = vec![key(0.0), key(100.0), key(200.0)];
        let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 50.0), Point::new(300.0, 50.0)], false));
        let spec = BlendSpec { spine: Some(line), ..Default::default() };
        let (path, anchors) = pin_spine(&keys, &spec).unwrap();
        assert_eq!(anchors, vec![0, 1, 2]);
        assert!((path.subpaths[0].anchors[1].p.x - 150.0).abs() < 1e-6, "the middle key sits halfway");
        // Pinned, each key's anchor follows it.
        let pinned = BlendSpec { spine: Some(path), key_anchors: vec![0, 1, 2], ..Default::default() };
        let (moved, _) = blend_spine(&[key(0.0), key(100.0), key(300.0)], &pinned).unwrap();
        assert_eq!(moved.subpaths[0].anchors[2].p, Point::new(305.0, 5.0));
    }

    #[test]
    fn steps_follow_a_bent_spine() {
        let keys = vec![key(0.0), key(100.0)];
        // A spine through the key centres bent up through (55, -95).
        let spine = PathData::single(SubPath::polyline(&[Point::new(5.0, 5.0), Point::new(55.0, -95.0), Point::new(105.0, 5.0)], false));
        let spec = BlendSpec { spacing: BlendSpacing::Steps(1), spine: Some(spine), key_anchors: vec![0, 2], ..Default::default() };
        let out = blend_expand(&keys, &spec);
        let mid = out[1].geometric_bounds().unwrap().center();
        assert!(mid.distance(Point::new(55.0, -95.0)) < 1e-6, "{mid:?}");
        assert_eq!(out[0].geometric_bounds().unwrap().center(), Point::new(5.0, 5.0), "keys stay put");
    }

    #[test]
    fn affine_lerp_turns_the_short_way() {
        let x = Affine::rotate(170f64.to_radians());
        let y = Affine::rotate(-170f64.to_radians());
        let m = lerp_affine(x, y, 0.5);
        let ang = decompose(m)[2].to_degrees();
        assert!((ang.abs() - 180.0).abs() < 1e-6, "{ang}");
        let s = lerp_affine(Affine::scale(1.0), Affine::translate((10.0, 0.0)) * Affine::scale(3.0), 0.5);
        assert!((s.as_coeffs()[0] - 2.0).abs() < 1e-9 && (s.as_coeffs()[4] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn closed_paths_start_where_they_twist_least() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut b = shapes::rectangle(Rect::new(100.0, 0.0, 110.0, 10.0));
        b.subpaths[0].anchors.rotate_left(2);
        let mid = lerp_path(&a, &b, 0.5);
        let r = mid.bounds().unwrap();
        assert!((r.width() - 10.0).abs() < 1e-6 && (r.height() - 10.0).abs() < 1e-6, "no twist: {r:?}");
        // Clicked start points win over the automatic match.
        let pair = PathPair::new(&a, &b, (Some(0), Some(0)));
        assert!(pair.at(0.5).bounds().unwrap().height() < 1e-6, "the clicked anchors pair up");
    }

    #[test]
    fn gradients_with_different_stop_counts_interpolate() {
        let g = |stops: &[(f32, Color)]| {
            let gradient = Gradient { stops: stops.iter().map(|(o, c)| GradientStop::new(*o, *c)).collect(), ..Gradient::default() };
            Paint::Gradient(Box::new(GradientPaint::new(gradient)))
        };
        let a = g(&[(0.0, Color::BLACK), (1.0, Color::BLACK)]);
        let b = g(&[(0.0, Color::WHITE), (0.5, Color::WHITE), (1.0, Color::WHITE)]);
        let Paint::Gradient(m) = lerp_paint(&a, &b, 0.25) else { panic!() };
        assert_eq!(m.gradient.stops.len(), 3);
        assert!(m.gradient.stops.iter().all(|s| (s.color.to_rgb()[0] - 0.25).abs() < 1e-4), "{:?}", m.gradient.stops);
    }

    #[test]
    fn dashes_and_profiles_interpolate() {
        let d = lerp_dash(Some(&Dash { pattern: vec![10.0, 10.0], ..Default::default() }), None, 0.5).unwrap();
        assert_eq!(d.pattern, vec![15.0, 5.0]);
        let d =
            lerp_dash(Some(&Dash { pattern: vec![4.0], ..Default::default() }), Some(&Dash { pattern: vec![2.0, 6.0], ..Default::default() }), 0.5);
        assert_eq!(d.unwrap().pattern, vec![3.0, 5.0]);
        let p = lerp_profile(Some(&WidthProfile::lens()), None, 0.5).unwrap();
        assert_eq!(p.at(0.0), (0.5, 0.5));
        assert_eq!(p.at(0.5), (1.0, 1.0));
    }

    #[test]
    fn copied_steps_get_new_members() {
        let rect = |x: f64| Arc::new(Node::path(NodeId(7), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default()));
        let a = Node::new(NodeId(1), NodeKind::Group { children: vec![rect(0.0), rect(20.0)], clip: true });
        let b = Node::new(NodeId(2), NodeKind::Group { children: vec![rect(100.0)], clip: true });
        let mid = lerp_node(&a, &b, 0.4);
        let mut ids = vec![];
        mid.walk(&mut |n| ids.push(n.id));
        assert!(ids.iter().all(|i| *i == NodeId(0)), "{ids:?}");
    }
}
