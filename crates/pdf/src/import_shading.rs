//! Shadings: axial and radial ones become gradients (their stops sampled from the shading
//! function in its own colour model; an end that doesn't extend clips the painted area), patch
//! and triangle meshes become gradient meshes, and a gradient drawn through a luminosity mask that
//! is the same gradient in grey (how stop opacity is written) gets its opacity stops back.

use std::sync::Arc;

use hayro_interpret::pattern::ShadingPattern;
use hayro_interpret::shading::{ShadingFunction, ShadingType};
use kurbo::{BezPath, Circle, Point, Rect, Shape, Vec2};
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::live::{GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshPoint, lerp_color};
use vectorcraft_doc::{Node, NodeKind, OpacityMask};

use crate::import::round3;
use crate::import_color::{Colors, Native};
use crate::import_mask::luminance;

/// The most mesh patches (or triangles) imported per shading: past it, the shading is a flat
/// colour.
const MAX_PATCHES: usize = 4096;

/// An axial or radial shading as a gradient paint, with whether it extends past its start and
/// its end.
pub(crate) fn shading_gradient(sp: &ShadingPattern, colors: &mut Colors<'_>) -> Option<(GradientPaint, [bool; 2])> {
    let ShadingType::RadialAxial { coords, domain, function, axial, extend } = sp.shading.shading_type.as_ref() else {
        return None;
    };
    let m = sp.matrix;
    let cs = &sp.shading.color_space;
    let at = |t: f32| function.eval(&smallvec::smallvec![domain[0] + (domain[1] - domain[0]) * t]);
    let space = colors.shading_space(cs, at(0.0)?.len());
    // RGB to decide which samples are stops; the stops keep the shading's model.
    let sample = |t: f32| -> Option<Color> {
        let [r, g, b, _] = cs.to_rgba(&at(t)?, 1.0, false).components();
        Some(Color::rgb(round3(r), round3(g), round3(b)))
    };
    // Sample densely, then drop samples that linear interpolation reproduces.
    const N: usize = 64;
    let mut pts: Vec<(f32, [f32; 3])> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f32 / N as f32;
        pts.push((t, sample(t)?.to_rgb()));
    }
    let mut keep = vec![0usize];
    for w in 1..N {
        let (Some(a), Some(p), Some(b)) = (keep.last().and_then(|&k| pts.get(k)), pts.get(w), pts.get(w + 1)) else { continue };
        let u = (p.0 - a.0) / (b.0 - a.0).max(1e-6);
        let off = (0..3).map(|k| (a.1[k] + (b.1[k] - a.1[k]) * u - p.1[k]).abs()).fold(0.0f32, f32::max);
        if off > 1.5 / 255.0 {
            keep.push(w);
        }
    }
    keep.push(N);
    let ts: Vec<f32> = keep.iter().filter_map(|&k| pts.get(k)).map(|p| p.0).collect();
    let mut stop_at = |t: f32, offset: f32| -> GradientStop {
        let rgb = sample(t).unwrap_or(Color::BLACK);
        match at(t).and_then(|v| colors.native(space, &v)) {
            Some(Native { color, link: Some((name, tint)) }) => GradientStop { swatch: Some(name), tint, ..GradientStop::new(offset, color) },
            Some(Native { color, link: None }) => GradientStop::new(offset, color),
            None => GradientStop::new(offset, rgb),
        }
    };
    let c = |x: f32, y: f32| m * Point::new(x as f64, y as f64);
    let (kind, geom, stops) = if *axial {
        let geom = GradientGeom { start: c(coords[0], coords[1]), end: c(coords[2], coords[3]), aspect: 1.0, focal: None };
        (GradientKind::Linear, geom, ts.iter().map(|&t| stop_at(t, t)).collect::<Vec<_>>())
    } else {
        let (r0, r1) = (coords[2].max(0.0), coords[5].max(1e-6));
        // The end circle, mapped (as an ellipse) into the page; a start circle off its centre
        // gives the focal point.
        let p = |x: f32, y: f32| Point::new(x as f64, y as f64);
        let centre = p(coords[3], coords[4]);
        let mut geom = GradientGeom { start: centre, end: centre + Vec2::new(r1 as f64, 0.0), aspect: 1.0, focal: None };
        geom.set_focal(Some(p(coords[0], coords[1])));
        geom.transform(m, GradientKind::Radial);
        // Offsets are relative to the outer radius; an inner radius shifts them outwards.
        let stops = ts.iter().map(|&t| stop_at(t, (r0 + (r1 - r0) * t) / r1)).collect();
        (GradientKind::Radial, geom, stops)
    };
    let mut g = GradientPaint::new(Gradient::new(kind, stops));
    g.geom = Some(geom);
    Some((g, *extend))
}

/// Where an axial or radial shading that doesn't extend at both ends paints, as a clip path in
/// document space (even-odd) for art filling `bounds`; `None` when it extends at both.
pub(crate) fn extend_clip(sp: &ShadingPattern, bounds: Rect) -> Option<BezPath> {
    let ShadingType::RadialAxial { coords, axial, extend, .. } = sp.shading.shading_type.as_ref() else {
        return None;
    };
    if extend[0] && extend[1] {
        return None;
    }
    let m = sp.matrix;
    let inv = m.inverse();
    let local = inv.transform_rect_bbox(bounds);
    if !local.is_finite() || m.determinant().abs() < 1e-12 {
        return None;
    }
    let f = |i: usize| coords.get(i).copied().unwrap_or(0.0) as f64;
    let mut out = BezPath::new();
    if *axial {
        let (p0, p1) = (Point::new(f(0), f(1)), Point::new(f(2), f(3)));
        let len = (p1 - p0).hypot();
        if len < 1e-9 {
            return None;
        }
        let u = (p1 - p0) / len;
        let n = Vec2::new(-u.y, u.x);
        let corners = [local.origin(), Point::new(local.x1, local.y0), Point::new(local.x1, local.y1), Point::new(local.x0, local.y1)];
        let along = corners.map(|c| (c - p0).dot(u));
        let across = corners.map(|c| (c - p0).dot(n));
        let min = |v: [f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let max = |v: [f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let lo = if extend[0] { min(along) } else { min(along).max(0.0) };
        let hi = if extend[1] { max(along) } else { max(along).min(len) };
        let (b0, b1) = (min(across), max(across));
        if hi > lo {
            let q = |a: f64, b: f64| m * (p0 + u * a + n * b);
            out.move_to(q(lo, b0));
            out.line_to(q(hi, b0));
            out.line_to(q(hi, b1));
            out.line_to(q(lo, b1));
            out.close_path();
        }
        return Some(out);
    }
    let start = Circle::new((f(0), f(1)), f(2).max(0.0));
    let end = Circle::new((f(3), f(4)), f(5).max(0.0));
    // Past the end: the end circle bounds the paint; before the start: the start circle is a hole.
    let outer = if extend[1] { local.inflate(1.0, 1.0).to_path(0.1) } else { end.to_path(0.1) };
    out.extend(m * outer);
    if !extend[0] && start.radius > 1e-9 {
        out.extend(m * start.to_path(0.1));
    }
    Some(out)
}

/// The colour of shading components `v` (the input of the shading's function, when it has one).
fn mesh_color(v: &[f32], function: Option<&ShadingFunction>, sp: &ShadingPattern, colors: &mut Colors<'_>) -> Color {
    let comps = match function {
        Some(f) => f.eval(&v.iter().copied().collect()).map(|c| c.to_vec()).unwrap_or_default(),
        None => v.to_vec(),
    };
    let space = colors.shading_space(&sp.shading.color_space, comps.len());
    colors.native(space, &comps).map(|n| n.color).unwrap_or_else(|| {
        let [r, g, b, _] = sp.shading.color_space.to_rgba(&comps, 1.0, false).components();
        Color::rgb(round3(r), round3(g), round3(b))
    })
}

/// A 1×1 gradient mesh from a patch's four corners (bottom-left, top-left, top-right,
/// bottom-right in the patch's u/v order), each with its handles towards its neighbours.
fn patch_mesh(corners: [(Point, [Vec2; 4], Color); 4]) -> GradientMesh {
    let point = |(p, handles, color): (Point, [Vec2; 4], Color)| MeshPoint { p, color, opacity: 1.0, handles };
    let [a, b, c, d] = corners;
    // Row 0 is v = 0 (corners a, d), row 1 is v = 1 (b, c); columns follow u.
    GradientMesh { rows: 1, cols: 1, points: vec![point(a), point(d), point(b), point(c)] }
}

/// Handles of a mesh corner: right, left, down, up (unused ones stay at the point).
fn handles(right: Vec2, left: Vec2, down: Vec2, up: Vec2) -> [Vec2; 4] {
    let mut h = [Vec2::ZERO; 4];
    (h[H_RIGHT], h[H_LEFT], h[H_DOWN], h[H_UP]) = (right, left, down, up);
    h
}

/// A 1×1 mesh from a patch's twelve boundary points (in the PDF's order: u runs cp0 → cp9 along
/// cp11 and cp10, v runs cp0 → cp3 along cp1 and cp2) and its corner colours (at cp0, cp3, cp6,
/// cp9); `None` if a point isn't finite.
fn boundary_mesh(cp: &[Point], colors: [Color; 4]) -> Option<GradientMesh> {
    let cp: &[Point; 12] = cp.get(..12)?.try_into().ok()?;
    if !cp.iter().all(|p| p.is_finite()) {
        return None;
    }
    let h = |a: usize, b: usize| cp[b] - cp[a];
    let [c0, c1, c2, c3] = colors;
    Some(patch_mesh([
        (cp[0], handles(h(0, 11), Vec2::ZERO, h(0, 1), Vec2::ZERO), c0),
        (cp[3], handles(h(3, 4), Vec2::ZERO, Vec2::ZERO, h(3, 2)), c1),
        (cp[6], handles(Vec2::ZERO, h(6, 5), Vec2::ZERO, h(6, 7)), c2),
        (cp[9], handles(Vec2::ZERO, h(9, 10), h(9, 8), Vec2::ZERO), c3),
    ]))
}

/// A patch mesh or triangle mesh shading as gradient meshes in document space (one per patch;
/// a triangle is a patch with two corners together). `None`: not a mesh, or too big.
pub(crate) fn mesh_shading(sp: &ShadingPattern, colors: &mut Colors<'_>) -> Option<Vec<GradientMesh>> {
    let m = sp.matrix;
    let mut out = vec![];
    match sp.shading.shading_type.as_ref() {
        ShadingType::CoonsPatchMesh { patches, function } => {
            if patches.len() > MAX_PATCHES {
                return None;
            }
            for p in patches {
                let colors = [0, 1, 2, 3].map(|i| mesh_color(p.colors.get(i).map_or(&[][..], |c| c.as_slice()), function.as_ref(), sp, colors));
                out.extend(boundary_mesh(&p.control_points.map(|q| m * q), colors));
            }
        }
        // A tensor patch's boundary is a Coons patch's (its four inner points are dropped).
        ShadingType::TensorProductPatchMesh { patches, function } => {
            if patches.len() > MAX_PATCHES {
                return None;
            }
            for p in patches {
                let colors = [0, 1, 2, 3].map(|i| mesh_color(p.colors.get(i).map_or(&[][..], |c| c.as_slice()), function.as_ref(), sp, colors));
                out.extend(boundary_mesh(&p.control_points.map(|q| m * q), colors));
            }
        }
        ShadingType::TriangleMesh { triangles, function } => {
            if triangles.len() > MAX_PATCHES {
                return None;
            }
            for t in triangles {
                let v = [&t.p0, &t.p1, &t.p2].map(|v| (m * v.point, mesh_color(v.colors.as_slice(), function.as_ref(), sp, colors)));
                if !v.iter().all(|(p, _)| p.is_finite()) {
                    continue;
                }
                let [a, b, c] = v;
                out.push(patch_mesh([
                    (a.0, [Vec2::ZERO; 4], a.1),
                    (b.0, [Vec2::ZERO; 4], b.1),
                    (c.0, [Vec2::ZERO; 4], c.1),
                    (c.0, [Vec2::ZERO; 4], c.1),
                ]));
            }
        }
        _ => return None,
    }
    Some(out)
}

/// The only painted leaf of mask art (through groups of one object and clip groups of one).
fn only_leaf(n: &Node) -> Option<&Node> {
    match &n.kind {
        NodeKind::Group { children, clip } => {
            let rest = if *clip { children.get(1..)? } else { children.as_slice() };
            match rest {
                [one] if n.opacity >= 0.999 && n.mask.is_none() => only_leaf(one),
                _ => None,
            }
        }
        NodeKind::Path { .. } => Some(n),
        _ => None,
    }
}

/// The gradient a path paints with its only fill or stroke.
fn only_gradient(n: &Node) -> Option<&GradientPaint> {
    match n.appearance.items.as_slice() {
        [item] => match item.paint() {
            Paint::Gradient(g) => Some(g),
            _ => None,
        },
        _ => None,
    }
}

fn same_geom(a: &GradientGeom, b: &GradientGeom) -> bool {
    let near = |p: Point, q: Point| p.distance(q) < 0.5;
    near(a.start, b.start) && near(a.end, b.end) && near(a.focal_point(), b.focal_point()) && (a.aspect - b.aspect).abs() < 0.01
}

/// `content` (a path painting one gradient) drawn through `mask` (clipped, not inverted, the same
/// gradient in shades of grey: how gradient stop opacity is written) as one gradient with the
/// mask's luminance as stop opacity.
pub(crate) fn fold_stop_opacity(content: &Node, mask: &OpacityMask) -> Option<Node> {
    if !mask.clip || mask.invert || mask.disabled || content.is_container() {
        return None;
    }
    let g = only_gradient(content)?;
    let leaf = only_leaf(&mask.art)?;
    let mg = only_gradient(leaf)?;
    let opaque = |n: &Node| n.opacity >= 0.999 && n.appearance.items.iter().all(|i| i.opacity() >= 0.999);
    if mg.gradient.kind != g.gradient.kind || !opaque(leaf) || !same_geom(g.geom.as_ref()?, mg.geom.as_ref()?) {
        return None;
    }
    let mut offsets: Vec<f32> = g.gradient.stops.iter().chain(&mg.gradient.stops).map(|s| s.offset).collect();
    offsets.sort_by(f32::total_cmp);
    offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let stops = offsets
        .iter()
        .map(|&t| {
            let (mc, mo) = mg.gradient.sample(t);
            let alpha = round3(luminance(&mc) * mo);
            let mut s = match g.gradient.stops.iter().find(|s| (s.offset - t).abs() < 1e-4) {
                Some(s) => s.clone(),
                None => {
                    let (c, o) = g.gradient.sample_with(t, lerp_color);
                    GradientStop { opacity: o, ..GradientStop::new(t, c) }
                }
            };
            s.opacity = round3(s.opacity * alpha);
            s
        })
        .collect();
    let mut out = content.clone();
    let mut paint = (*g).clone();
    paint.gradient.stops = stops;
    if let Some(item) = out.appearance.items.first_mut() {
        *item.paint_mut() = Paint::Gradient(Box::new(paint));
    }
    Some(out)
}

/// `node` (filling `bounds`) clipped to `clip`: a clip group.
pub(crate) fn clipped(clip: Node, node: Node, id: vectorcraft_doc::NodeId) -> Node {
    Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(node)], clip: true })
}
