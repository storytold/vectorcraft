//! Shadings: axial and radial ones become gradients (their stops sampled from the shading
//! function in its own colour model; an end that doesn't extend clips the painted area), a
//! function-based one whose colour changes along one axis a linear gradient (else its average
//! colour; all of them within one budget of function evaluations per import), patch and triangle
//! meshes become gradient meshes, a gradient drawn through a luminosity mask that is
//! the same gradient in grey (how stop opacity is written) gets its opacity stops back, and a shape
//! clipping a gradient that fills it becomes one path with the gradient.

use std::sync::Arc;

use craft_pdf::interpret::pattern::ShadingPattern;
use craft_pdf::interpret::shading::{ShadingFunction, ShadingType};
use kurbo::{BezPath, Circle, Point, Rect, Shape, Vec2};
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::live::{GradientMesh, lerp_color};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind, OpacityMask};
use vectorcraft_geom::PathData;

use crate::import::{closed_subpaths, round3};
use crate::import_color::{Colors, Native};
use crate::import_mask::{contains, is_rectangle, luminance};

/// The most mesh patches (or triangles) imported per shading: past it, the shading is a flat
/// colour.
const MAX_PATCHES: usize = 4096;

/// An axial or radial shading as a gradient paint, with whether it extends past its start and
/// its end. `inks`: the colorant names of its space, from the page's walk ([`Colors::solid`]).
pub(crate) fn shading_gradient(sp: &ShadingPattern, colors: &mut Colors<'_>, inks: Option<&[String]>) -> Option<(GradientPaint, [bool; 2])> {
    let ShadingType::RadialAxial { coords, domain, function, axial, extend } = sp.shading.shading_type.as_ref() else {
        return None;
    };
    let m = sp.matrix;
    let cs = &sp.shading.color_space;
    let at = |t: f32| function.eval(&smallvec::smallvec![domain[0] + (domain[1] - domain[0]) * t]);
    let space = colors.shading_space(cs, at(0.0)?.len(), inks);
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
    let ts: Vec<f32> = linear_keep(&pts).iter().filter_map(|&k| pts.get(k)).map(|p| p.0).collect();
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

/// The samples of `pts` (offset, RGB; at least one) a gradient keeps as stops: the first, the
/// last, and each one that linear interpolation between its neighbouring stops doesn't reproduce.
fn linear_keep(pts: &[(f32, [f32; 3])]) -> Vec<usize> {
    let n = pts.len().saturating_sub(1);
    // Every sample dropped since the last stop is checked against the line to the next one, not
    // just its neighbours: a gentle curve (an exponential's N) looks straight between neighbours.
    let mut keep = vec![0usize];
    for w in 1..n {
        let (Some(&k), Some(b)) = (keep.last(), pts.get(w + 1)) else { continue };
        let Some(a) = pts.get(k) else { continue };
        let off = pts.get(k + 1..=w).unwrap_or_default().iter().fold(0.0f32, |worst, p| {
            let u = (p.0 - a.0) / (b.0 - a.0).max(1e-6);
            (0..3).map(|c| (a.1[c] + (b.1[c] - a.1[c]) * u - p.1[c]).abs()).fold(worst, f32::max)
        });
        if off > 1.5 / 255.0 {
            keep.push(w);
        }
    }
    if n > 0 {
        keep.push(n);
    }
    keep
}

/// Samples along a function-based shading's axis; a colour change between two of them is found
/// by halving the interval.
const FUNCTION_SAMPLES: usize = 4096;
/// The most stops a function-based shading becomes: past it, it has no gradient.
const MAX_FUNCTION_STOPS: usize = 2048;
/// Grid points per side, sampling a function-based shading's domain (to tell along which axis its
/// colour changes, or to average it).
const FUNCTION_GRID: usize = 16;
/// Samples across the axis a function-based shading's colour changes along, through the middle
/// of its domain, that must all be one colour (what the grid can't tell from a periodic one).
const FUNCTION_ACROSS: usize = 256;
/// The function evaluations function-based shadings may take in one import, together: a file
/// can fill any number of paths with them, each a function of any length.
pub(crate) const MAX_FUNCTION_EVALUATIONS: usize = 200_000;
/// What averaging a function-based shading takes ([`function_average`]).
const AVERAGE_EVALUATIONS: usize = (FUNCTION_GRID + 1) * (FUNCTION_GRID + 1);

/// The function evaluations function-based shadings took (tests; imports read on threads of
/// their own, so tests that count hold `tests_import_fidelity::SHADINGS`).
#[cfg(test)]
pub(crate) static EVALUATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What is left of one import's function evaluations ([`MAX_FUNCTION_EVALUATIONS`]).
pub(crate) struct FunctionBudget(std::cell::Cell<usize>);

impl FunctionBudget {
    pub(crate) fn new() -> Self {
        Self(std::cell::Cell::new(MAX_FUNCTION_EVALUATIONS))
    }

    /// Take one evaluation, leaving at least `reserve`: `false` when there isn't one.
    fn take(&self, reserve: usize) -> bool {
        let left = self.0.get();
        if left <= reserve {
            return false;
        }
        self.0.set(left - 1);
        #[cfg(test)]
        EVALUATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        true
    }
}

/// What a function-based shading comes in as ([`function_paint`]).
pub(crate) enum FunctionPaint {
    /// Its colour changes along one axis: a linear gradient.
    Gradient(Box<GradientPaint>),
    /// It has no gradient: the colour it averages to.
    Average(Color),
    /// Its function can't be read, or the import's budget is spent.
    Unread,
}

/// Function-based shading `sp` as a gradient, else its average colour, its function evaluated
/// within `budget` (a gradient leaves enough of it to average another shading). `None` for other
/// shadings.
pub(crate) fn function_paint(
    sp: &ShadingPattern,
    colors: &mut Colors<'_>,
    inks: Option<&[String]>,
    budget: &FunctionBudget,
) -> Option<FunctionPaint> {
    if !matches!(sp.shading.shading_type.as_ref(), ShadingType::FunctionBased { .. }) {
        return None;
    }
    Some(match function_gradient(sp, colors, inks, budget) {
        Some(g) => FunctionPaint::Gradient(Box::new(g)),
        None => function_average(sp, budget).map_or(FunctionPaint::Unread, FunctionPaint::Average),
    })
}

/// RGB colours closer than this in every component are the same colour.
fn same_rgb(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(p, q)| (p - q).abs() <= 1.5 / 255.0)
}

/// A function-based shading's function and its domain (`[x0, x1, y0, y1]`, finite and not
/// empty); `None` for other shadings.
fn function_domain(sp: &ShadingPattern) -> Option<(&ShadingFunction, [f32; 4])> {
    let ShadingType::FunctionBased { domain, function, .. } = sp.shading.shading_type.as_ref() else { return None };
    let [x0, x1, y0, y1] = *domain;
    (domain.iter().all(|v| v.is_finite()) && x1 > x0 && y1 > y0).then_some((function, *domain))
}

/// The value of function-based shading function `function` at point (`x`, `y`) of its domain, one
/// evaluation of `budget` leaving `reserve`.
fn function_value(function: &ShadingFunction, x: f32, y: f32, budget: &FunctionBudget, reserve: usize) -> Option<smallvec::SmallVec<[f32; 4]>> {
    budget.take(reserve).then(|| function.eval(&smallvec::smallvec![x, y]))?
}

/// The RGB of function-based shading `sp` (its function `function`) at point (`x`, `y`) of its
/// domain, as [`function_value`].
fn function_rgb(sp: &ShadingPattern, function: &ShadingFunction, x: f32, y: f32, budget: &FunctionBudget, reserve: usize) -> Option<[f32; 3]> {
    let [r, g, b, _] = sp.shading.color_space.to_rgba(&function_value(function, x, y, budget, reserve)?, 1.0, false).components();
    Some([r, g, b])
}

/// Where grid point `i` (of [`FUNCTION_GRID`] + 1) falls across a domain, 0..1: off round
/// fractions, where a periodic function would line up with the grid.
fn grid_at(i: usize) -> f32 {
    (i as f32 + 0.37) / (FUNCTION_GRID as f32 + 1.0)
}

/// A function-based shading (type 1) whose colour changes along one axis of its domain only (how
/// a browser writes a repeating linear gradient) as a linear gradient across its domain: stops
/// sampled along that axis, each hard colour change two stops at one offset. `None` when the
/// colour changes along both axes, would take more than [`MAX_FUNCTION_STOPS`] stops, or `budget`
/// runs out (leaving [`AVERAGE_EVALUATIONS`] for [`function_average`]).
fn function_gradient(sp: &ShadingPattern, colors: &mut Colors<'_>, inks: Option<&[String]>, budget: &FunctionBudget) -> Option<GradientPaint> {
    let ShadingType::FunctionBased { matrix, .. } = sp.shading.shading_type.as_ref() else { return None };
    let (function, [x0, x1, y0, y1]) = function_domain(sp)?;
    let rgb = |x: f32, y: f32| function_rgb(sp, function, x, y, budget, AVERAGE_EVALUATIONS);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let grid: Vec<Vec<[f32; 3]>> = (0..=FUNCTION_GRID)
        .map(|j| (0..=FUNCTION_GRID).map(|i| rgb(lerp(x0, x1, grid_at(i)), lerp(y0, y1, grid_at(j)))).collect())
        .collect::<Option<_>>()?;
    let first = grid.first()?;
    // Each row as the first: the colour may change along x only. Each row one colour: along y.
    let along_x = grid.iter().all(|row| row.iter().zip(first).all(|(a, b)| same_rgb(*a, *b)));
    let along_y = grid.iter().all(|row| row.first().is_some_and(|c0| row.iter().all(|c| same_rgb(*c, *c0))));
    // The grid can miss a change that repeats in step with it: across the middle of the domain,
    // densely, the colour must not change along the other axis. Both left (a grid of one colour),
    // the axis it changes along is the one, if any, it changes along there.
    let steady = |x_axis: bool| -> Option<bool> {
        let at = |k: usize| k as f32 / FUNCTION_ACROSS as f32;
        let probe = |k: usize| if x_axis { rgb(lerp(x0, x1, at(k)), lerp(y0, y1, 0.5)) } else { rgb(lerp(x0, x1, 0.5), lerp(y0, y1, at(k))) };
        let c0 = probe(0)?;
        for k in 1..=FUNCTION_ACROSS {
            if !same_rgb(probe(k)?, c0) {
                return Some(false);
            }
        }
        Some(true)
    };
    let along_x = match (along_x, along_y) {
        (false, false) => return None,
        (true, false) if steady(false)? => true,
        (false, true) if steady(true)? => false,
        (true, true) if steady(false)? => true,
        (true, true) if steady(true)? => false,
        _ => return None,
    };
    let point = |s: f32| if along_x { (lerp(x0, x1, s), lerp(y0, y1, 0.5)) } else { (lerp(x0, x1, 0.5), lerp(y0, y1, s)) };
    let color_of = |s: f32| {
        let (x, y) = point(s);
        rgb(x, y)
    };
    // Pieces of the axis between hard colour changes: (offset, where its colour is read, RGB).
    let start = color_of(0.0)?;
    let mut pieces: Vec<Vec<(f32, f32, [f32; 3])>> = vec![vec![(0.0, 0.0, start)]];
    let mut stops = 1;
    let mut last = (0.0, start);
    for k in 1..=FUNCTION_SAMPLES {
        let s = k as f32 / FUNCTION_SAMPLES as f32;
        let c = color_of(s)?;
        let (a, ca) = last;
        if !same_rgb(ca, c) {
            // Halve the interval while it holds the change: one that stays as it closes is a hard
            // change (a smooth one fades out).
            let (mut lo, mut hi, mut clo, mut chi) = (a, s, ca, c);
            for _ in 0..24 {
                let mid = (lo + hi) / 2.0;
                let cm = color_of(mid)?;
                if same_rgb(clo, cm) {
                    (lo, clo) = (mid, cm);
                } else {
                    (hi, chi) = (mid, cm);
                }
            }
            if !same_rgb(clo, chi) {
                let edge = (lo + hi) / 2.0;
                pieces.last_mut()?.push((edge, lo, clo));
                pieces.push(vec![(edge, hi, chi)]);
                stops += 2;
            }
        }
        pieces.last_mut()?.push((s, s, c));
        last = (s, c);
        if stops > MAX_FUNCTION_STOPS {
            return None;
        }
    }
    // A stop's colour in the shading's own model (RGB when that can't be read).
    let value = |s: f32| {
        let (x, y) = point(s);
        function_value(function, x, y, budget, AVERAGE_EVALUATIONS)
    };
    let space = colors.shading_space(&sp.shading.color_space, value(0.0)?.len(), inks);
    let mut out = vec![];
    for piece in &pieces {
        let pts: Vec<(f32, [f32; 3])> = piece.iter().map(|p| (p.0, p.2)).collect();
        for k in linear_keep(&pts) {
            let Some(&(offset, read, c)) = piece.get(k) else { continue };
            let rgb = Color::rgb(round3(c[0]), round3(c[1]), round3(c[2]));
            out.push(match value(read).and_then(|v| colors.native(space, &v)) {
                Some(Native { color, link: Some((name, tint)) }) => GradientStop { swatch: Some(name), tint, ..GradientStop::new(offset, color) },
                Some(Native { color, link: None }) => GradientStop::new(offset, color),
                None => GradientStop::new(offset, rgb),
            });
        }
    }
    if out.len() > MAX_FUNCTION_STOPS {
        return None;
    }
    let p = |s: f32| {
        let (x, y) = point(s);
        Point::new(x as f64, y as f64)
    };
    let mut geom = GradientGeom { start: p(0.0), end: p(1.0), aspect: 1.0, focal: None };
    geom.transform(sp.matrix * *matrix, GradientKind::Linear);
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, out));
    g.geom = Some(geom);
    // It paints only over its domain ([`extend_clip`]).
    Some(g)
}

/// The colour a function-based shading averages to over its domain, for one that has no gradient
/// ([`function_gradient`]); `None` for other shadings, or when `budget` runs out.
fn function_average(sp: &ShadingPattern, budget: &FunctionBudget) -> Option<Color> {
    let (function, [x0, x1, y0, y1]) = function_domain(sp)?;
    let mut sum = [0.0f32; 3];
    for j in 0..=FUNCTION_GRID {
        for i in 0..=FUNCTION_GRID {
            let c = function_rgb(sp, function, x0 + (x1 - x0) * grid_at(i), y0 + (y1 - y0) * grid_at(j), budget, 0)?;
            sum.iter_mut().zip(c).for_each(|(s, v)| *s += v);
        }
    }
    let n = ((FUNCTION_GRID + 1) * (FUNCTION_GRID + 1)) as f32;
    Some(Color::rgb(round3(sum[0] / n), round3(sum[1] / n), round3(sum[2] / n)))
}

/// Where an axial or radial shading that doesn't extend at both ends paints, as a clip path in
/// document space (even-odd) for art filling `bounds`; `None` when it extends at both. A
/// function-based shading paints only over its domain: that, unless it covers `bounds`.
pub(crate) fn extend_clip(sp: &ShadingPattern, bounds: Rect) -> Option<BezPath> {
    if let ShadingType::FunctionBased { domain, matrix, .. } = sp.shading.shading_type.as_ref() {
        let m = sp.matrix * *matrix;
        if m.determinant().abs() < 1e-12 {
            return None;
        }
        let local = m.inverse().transform_rect_bbox(bounds);
        let [x0, x1, y0, y1] = domain.map(f64::from);
        let d = Rect::new(x0, y0, x1, y1);
        let tol = 1e-3 * (d.width() + d.height());
        let covers = local.x0 >= d.x0 - tol && local.y0 >= d.y0 - tol && local.x1 <= d.x1 + tol && local.y1 <= d.y1 + tol;
        return (local.is_finite() && d.is_finite() && !covers).then(|| m * d.to_path(0.1));
    }
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
/// `inks` as for [`shading_gradient`].
fn mesh_color(v: &[f32], function: Option<&ShadingFunction>, sp: &ShadingPattern, colors: &mut Colors<'_>, inks: Option<&[String]>) -> Color {
    let comps = match function {
        Some(f) => f.eval(&v.iter().copied().collect()).map(|c| c.to_vec()).unwrap_or_default(),
        None => v.to_vec(),
    };
    let space = colors.shading_space(&sp.shading.color_space, comps.len(), inks);
    colors.native(space, &comps).map(|n| n.color).unwrap_or_else(|| {
        let [r, g, b, _] = sp.shading.color_space.to_rgba(&comps, 1.0, false).components();
        Color::rgb(round3(r), round3(g), round3(b))
    })
}

/// A patch mesh or triangle mesh shading as gradient meshes in document space (one per patch;
/// a triangle is a patch with two corners together). `None`: not a mesh, or too big. `inks` as
/// for [`shading_gradient`].
pub(crate) fn mesh_shading(sp: &ShadingPattern, colors: &mut Colors<'_>, inks: Option<&[String]>) -> Option<Vec<GradientMesh>> {
    let m = sp.matrix;
    let mut out = vec![];
    match sp.shading.shading_type.as_ref() {
        ShadingType::CoonsPatchMesh { patches, function } => {
            if patches.len() > MAX_PATCHES {
                return None;
            }
            for p in patches {
                let colors = [0, 1, 2, 3].map(|i| mesh_color(p.colors.get(i).map_or(&[][..], |c| c.as_slice()), function.as_ref(), sp, colors, inks));
                out.extend(GradientMesh::coons(&p.control_points.map(|q| m * q), colors));
            }
        }
        // A tensor patch's boundary is a Coons patch's (its four inner points are dropped).
        ShadingType::TensorProductPatchMesh { patches, function } => {
            if patches.len() > MAX_PATCHES {
                return None;
            }
            for p in patches {
                let colors = [0, 1, 2, 3].map(|i| mesh_color(p.colors.get(i).map_or(&[][..], |c| c.as_slice()), function.as_ref(), sp, colors, inks));
                out.extend(p.control_points.map(|q| m * q).first_chunk::<12>().and_then(|cp| GradientMesh::coons(cp, colors)));
            }
        }
        ShadingType::TriangleMesh { triangles, function } => {
            if triangles.len() > MAX_PATCHES {
                return None;
            }
            for t in triangles {
                let v = [&t.p0, &t.p1, &t.p2].map(|v| (m * v.point, mesh_color(v.colors.as_slice(), function.as_ref(), sp, colors, inks)));
                out.extend(GradientMesh::triangle(v));
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
                [one] if n.opacity >= 0.999 && n.extras().mask.is_none() => only_leaf(one),
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

/// A shape filled with a gradient, as files write it (the shape as a clip, then `sh` filling the
/// clip's bounds), as one path: the clip's outline with `content`'s look. `None` unless `content`
/// is one rectangle covering the clip, filled with a gradient placed on the page and nothing else.
pub(crate) fn gradient_shape(clip: &Node, content: &[Arc<Node>]) -> Option<Node> {
    let ([only], NodeKind::Path { path: outline, rule, .. }) = (content, &clip.kind) else { return None };
    let NodeKind::Path { path: area, clipping: false, guide: false, .. } = &only.kind else { return None };
    let [AppearanceItem::Fill(f)] = only.appearance.items.as_slice() else { return None };
    let Paint::Gradient(g) = &f.paint else { return None };
    let (area, outline) = (area.to_bezpath(), outline.to_bezpath());
    let covers = is_rectangle(&area) && contains(area.bounding_box().inflate(0.01, 0.01), outline.bounding_box());
    if g.geom.is_none() || !covers {
        return None;
    }
    let mut shape = (**only).clone();
    // A clip closes its subpaths, as a fill does.
    let path = PathData::from_bezpath(&closed_subpaths(&outline));
    shape.kind = NodeKind::Path { path, rule: *rule, live: None, clipping: false, guide: false };
    Some(shape)
}
