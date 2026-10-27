//! Art, scatter and pattern brushes: art bent along the path by arc length.
//!
//! Art coordinates `(x, y)` map to the path point at arc length `s(x)` offset by
//! `(y - centre) × cross` along the path's left normal. Art outlines are flattened and
//! subdivided first so long straight edges bend with the path instead of cutting chords.

use std::sync::Arc;

use vectorcraft_doc::{Node, NodeKind, PressureProfile};
use vectorcraft_geom::{Affine, BezPath, PathData, PathEl, Point, Rect, SubPath};

use crate::track::{Rng, Track, seed_of, tracks};
use crate::{ArtBrush, ArtScale, Direction, PatternBrush, PatternFit, Scatter, Variation, tolerance};

/// Longest mapped segment (points in document space).
const MAX_SEG: f64 = 1.5;
/// Hard cap on subdivisions per art segment.
const MAX_SUB: usize = 4000;
/// Anchors one brushed stroke may emit in total, across every track, tile and copy.
///
/// Each factor is capped on its own (20,000 tiles a run, [`MAX_SUB`] subdivisions a segment,
/// 20,000 copies a track), but those caps are per track and a path has as many tracks as it has
/// subpaths, so they multiply rather than bound: one track at its cap measures 20,000 nodes and
/// 320,000 anchors, and sixteen subpaths measure exactly sixteen times that (#3594). This is the
/// cap on the product. The per-factor caps still shape each one.
///
/// The same figure as the stacked-effects budget (`effects::distort::MAX_COPY_ANCHORS`, #3578),
/// and bounded the same way: per brushed stroke rather than per document, so many strokes that
/// each reach it still add up. Six times what one track at its own cap draws, so it bites only
/// where the caps were multiplying.
const MAX_ANCHORS: usize = 2_000_000;
/// Nodes [`anchors_of`] walks into before it gives up and charges the whole budget.
const MAX_WALK: usize = 10_000;

/// Bend every path under `n` through `f`. Paths are flattened with `flat_tol` (art space) and
/// segments subdivided so none is longer than `max_du` along x. Stroke widths scale by
/// `width_scale`. Text, images and symbol instances can't bend and are dropped.
pub(crate) fn warp_node(n: &Node, f: &dyn Fn(Point) -> Point, flat_tol: f64, max_du: f64, width_scale: f64) -> Option<Node> {
    let mut out = n.clone();
    out.appearance.scale_strokes(width_scale);
    match &mut out.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None;
            *path = warp_path(path, f, flat_tol, max_du);
            if path.is_empty() {
                return None;
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            let kids: Vec<Arc<Node>> = children.iter().filter_map(|c| warp_node(c, f, flat_tol, max_du, width_scale)).map(Arc::new).collect();
            if kids.is_empty() {
                return None;
            }
            *children = kids;
        }
        _ => return None,
    }
    Some(out)
}

fn warp_path(p: &PathData, f: &dyn Fn(Point) -> Point, flat_tol: f64, max_du: f64) -> PathData {
    let bp = p.to_bezpath();
    let mut subs = vec![];
    let mut cur: Vec<Point> = vec![];
    let mut els = vec![];
    kurbo::flatten(bp.iter(), flat_tol.max(1e-4), |e| els.push(e));
    let finish = |cur: &mut Vec<Point>, closed: bool, subs: &mut Vec<SubPath>| {
        let pts = std::mem::take(cur);
        if pts.len() < 2 {
            return;
        }
        let mut dense = vec![pts[0]];
        let n = pts.len();
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let k = (((b.x - a.x).abs() / max_du).ceil() as usize).clamp(1, MAX_SUB);
            for j in 1..=k {
                let q = a.lerp(b, j as f64 / k as f64);
                if closed && i == segs - 1 && j == k {
                    break;
                }
                dense.push(q);
            }
        }
        let mapped: Vec<Point> = dense.into_iter().map(f).collect();
        subs.push(SubPath::polyline(&mapped, closed));
    };
    for e in els {
        match e {
            PathEl::MoveTo(q) => {
                finish(&mut cur, false, &mut subs);
                cur.push(q);
            }
            PathEl::LineTo(q) => cur.push(q),
            PathEl::ClosePath => {
                if cur.len() > 1 && cur[0].distance(cur[cur.len() - 1]) < 1e-9 {
                    cur.pop();
                }
                finish(&mut cur, true, &mut subs);
            }
            _ => {}
        }
    }
    finish(&mut cur, false, &mut subs);
    PathData::new(subs)
}

/// Art turned so it runs left → right, flipped as requested; returns it with its bounds.
fn oriented(art: &Node, dir: Direction, flip_along: bool, flip_across: bool) -> Option<(Node, Rect)> {
    let mut a = art.clone();
    let rot = match dir {
        Direction::LeftToRight => Affine::IDENTITY,
        Direction::RightToLeft => Affine::scale_non_uniform(-1.0, 1.0),
        Direction::TopToBottom => Affine::rotate(-std::f64::consts::FRAC_PI_2),
        Direction::BottomToTop => Affine::rotate(std::f64::consts::FRAC_PI_2),
    };
    let flip = Affine::scale_non_uniform(if flip_along { -1.0 } else { 1.0 }, if flip_across { -1.0 } else { 1.0 });
    let m = flip * rot;
    if m != Affine::IDENTITY {
        a.transform(m, false);
    }
    let b = a.geometric_bounds()?;
    (b.width() > 1e-9).then_some((a, b))
}

/// Piecewise-linear map from art x (offset from the art's left edge) to arc length.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlongMap {
    /// Art-space breakpoints (x0 = 0, x1, x2, x3 = width) and their arc lengths.
    xs: [f64; 4],
    ss: [f64; 4],
}

impl AlongMap {
    fn linear(width: f64, s0: f64, s1: f64) -> Self {
        AlongMap { xs: [0.0, 0.0, width, width], ss: [s0, s0, s1, s1] }
    }
    pub fn at(&self, x: f64) -> f64 {
        let (xs, ss) = (&self.xs, &self.ss);
        if x <= xs[0] {
            return ss[0] + (x - xs[0]) * self.slope(0);
        }
        for i in 0..3 {
            if x <= xs[i + 1] {
                let w = xs[i + 1] - xs[i];
                return if w <= 1e-12 { ss[i + 1] } else { ss[i] + (x - xs[i]) / w * (ss[i + 1] - ss[i]) };
            }
        }
        ss[3] + (x - xs[3]) * self.slope(2)
    }
    fn slope(&self, i: usize) -> f64 {
        let w = self.xs[i + 1] - self.xs[i];
        if w <= 1e-12 { 1.0 } else { (self.ss[i + 1] - self.ss[i]) / w }
    }
    /// Largest along-scale (arc length per art unit) of the pieces.
    fn max_scale(&self) -> f64 {
        (0..3).map(|i| self.slope(i).abs()).fold(1e-6, f64::max)
    }
}

/// Bend `art` (already oriented, bounds `b`) along `t` using `along` and cross scale `cross`.
fn bend(t: &Track, art: &Node, b: Rect, along: AlongMap, cross: f64) -> Option<Node> {
    let cy = b.center().y;
    let x0 = b.x0;
    let f = move |p: Point| t.map(along.at(p.x - x0), (p.y - cy) * cross);
    let scale = along.max_scale().max(cross.abs()).max(1e-6);
    let flat_tol = tolerance(b.height().max(b.width()) * scale) / scale;
    warp_node(art, &f, flat_tol, MAX_SEG / along.max_scale(), cross.abs())
}

/// [`bend`], with the cross scale at arc length `s` multiplied by `widen(s)` (0..=1). The strokes
/// of each object in the art take the factor at the middle of the object.
fn bend_widened(t: &Track, art: &Node, b: Rect, along: AlongMap, cross: f64, widen: &dyn Fn(f64) -> f64) -> Option<Node> {
    let cy = b.center().y;
    let x0 = b.x0;
    let f = move |p: Point| {
        let s = along.at(p.x - x0);
        t.map(s, (p.y - cy) * cross * widen(s))
    };
    let factor = |n: &Node| n.geometric_bounds().map_or(1.0, |r| widen(along.at(r.center().x - x0)));
    let art = widen_strokes(art, &factor);
    let scale = along.max_scale().max(cross.abs()).max(1e-6);
    let flat_tol = tolerance(b.height().max(b.width()) * scale) / scale;
    warp_node(&art, &f, flat_tol, MAX_SEG / along.max_scale(), cross.abs())
}

/// `n` with the strokes of it and of everything inside it scaled by `factor` of the object they
/// belong to.
fn widen_strokes(n: &Node, factor: &dyn Fn(&Node) -> f64) -> Node {
    let mut out = n.clone();
    out.appearance.scale_strokes(factor(n));
    if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } = &mut out.kind {
        *children = children.iter().map(|c| Arc::new(widen_strokes(c, factor))).collect();
    }
    out
}

/// Arc-length spans to place art on when [`ArtBrush::overlap`] is on: one run between each sharp
/// corner (the whole path when it has none).
fn art_runs(t: &Track) -> Vec<(f64, f64)> {
    let l = t.len();
    let corners = t.corner_positions();
    if corners.is_empty() {
        return vec![(0.0, l)];
    }
    if t.closed {
        let first = corners[0].1;
        let mut bounds = corners.iter().map(|c| c.1).collect::<Vec<_>>();
        bounds.push(first + l);
        return bounds.windows(2).map(|w| (w[0], w[1])).collect();
    }
    let mut bounds = vec![0.0];
    bounds.extend(corners.iter().map(|c| c.1));
    bounds.push(l);
    bounds.windows(2).map(|w| (w[0], w[1])).collect()
}

/// The art on [s0, s1] of `t`, `cross` across where `widen` (0..=1, by arc length) is 1, and
/// `width` the width (%) there.
fn art_on_run(
    t: &Track,
    (s0, s1): (f64, f64),
    a: &ArtBrush,
    (art, b): (&Node, Rect),
    cross: f64,
    width: f64,
    widen: &dyn Fn(f64) -> f64,
) -> Option<Node> {
    let l = s1 - s0;
    if l <= 1e-9 {
        return None;
    }
    let w = b.width();
    let (along, cross) = match a.scale {
        ArtScale::Stretch => (AlongMap::linear(w, s0, s1), cross),
        ArtScale::Proportional => {
            let k = l / w;
            (AlongMap::linear(w, s0, s1), k * width / 100.0)
        }
        ArtScale::BetweenGuides { start, end } => {
            let (g0, g1) = (start.clamp(0.0, 1.0) * w, end.clamp(0.0, 1.0) * w);
            let (g0, g1) = (g0.min(g1), g0.max(g1));
            // The ends keep the proportions of the width at their end of the run.
            let (c0, c1) = (cross * widen(s0), cross * widen(s1));
            let fixed = g0 * c0 + (w - g1) * c1;
            if fixed >= l || g1 - g0 < 1e-9 {
                (AlongMap::linear(w, s0, s1), cross)
            } else {
                (AlongMap { xs: [0.0, g0, g1, w], ss: [s0, s0 + g0 * c0, s1 - (w - g1) * c1, s1] }, cross)
            }
        }
    };
    bend_widened(t, art, b, along, cross, widen)
}

/// An art brush's art along `bp`. With a width that follows the pen's pressure, the art is as
/// wide as the pressure recorded at each point of the stroke gives (half way without a recording).
pub(crate) fn art(a: &ArtBrush, bp: &BezPath, weight: f64, pressure: Option<&PressureProfile>) -> Vec<Node> {
    let Some((art, b)) = oriented(&a.art, a.direction, a.flip_along, a.flip_across) else { return vec![] };
    let (light, heavy) = a.width_range();
    let width = light.max(heavy);
    let cross = weight * width / 100.0;
    let mut out = vec![];
    for t in tracks(bp, 0.05) {
        let l = t.len();
        if l <= 1e-9 {
            continue;
        }
        let widen = |s: f64| {
            if light == heavy || width <= 0.0 {
                return 1.0;
            }
            // A run past the start of a closed track wraps round to it.
            let at = if s > l { s - l } else { s };
            let p = pressure.map_or(PressureProfile::MID, |p| p.at(at / l)).clamp(0.0, 1.0);
            (light + (heavy - light) * p) / width
        };
        let runs = if a.overlap { art_runs(&t) } else { vec![(0.0, l)] };
        for run in runs {
            if let Some(n) = art_on_run(&t, run, a, (&art, b), cross, width, &widen) {
                out.push(n);
            }
        }
    }
    out
}

/// One of a scatter brush's values for the next copy: Fixed takes the first of `(a, b)`, Random a
/// value between them, Pressure the first at pen pressure 0 and the second at 1. Every value
/// draws, so a copy's random values stay the same whatever the other modes.
fn vary(rng: &mut Rng, (a, b): (f64, f64), mode: Variation, pressure: f64) -> f64 {
    let r = rng.range(a, b);
    match mode {
        Variation::Fixed => a,
        Variation::Random => r,
        Variation::Pressure => a + (b - a) * pressure.clamp(0.0, 1.0),
    }
}

pub(crate) fn scatter(sc: &Scatter, bp: &BezPath, weight: f64, pressure: Option<&PressureProfile>, name: &str) -> Vec<Node> {
    let Some(b) = sc.art.geometric_bounds() else { return vec![] };
    let (aw, ah) = (b.width().max(1e-3), b.height().max(1e-3));
    let mut rng = Rng::new(seed_of(bp, name));
    let [size_mode, spacing_mode, scatter_mode, rotation_mode] = sc.modes();
    let mut out = Budget::default();
    for t in tracks(bp, 0.05) {
        if out.full() {
            break;
        }
        let l = t.len();
        // The pen pressure at arc length `at` (half way without a recorded one).
        let pressure_at = |at: f64| pressure.map_or(PressureProfile::MID, |p| p.at(if l > 1e-9 { at / l } else { 0.0 }));
        let mut s: Option<f64> = None;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 20000 {
                break;
            }
            // Size and spacing place the next copy, so they take the pressure at the last one.
            let p = pressure_at(s.unwrap_or(0.0));
            let size = vary(&mut rng, sc.size, size_mode, p).max(1.0) / 100.0 * weight;
            let spacing = vary(&mut rng, sc.spacing, spacing_mode, p).max(1.0) / 100.0;
            let step = aw * size * spacing;
            let pos = match s {
                None => step / 2.0,
                Some(prev) => prev + step,
            };
            if pos > l + 1e-9 || (t.closed && pos >= l - 1e-9 && s.is_some()) {
                break;
            }
            s = Some(pos);
            let p = pressure_at(pos);
            let off = vary(&mut rng, sc.scatter, scatter_mode, p) / 100.0 * ah * size;
            let rot = vary(&mut rng, sc.rotation, rotation_mode, p).to_radians();
            let (p, tg) = t.frame(pos);
            let place = p + crate::track::normal(tg) * off;
            // Page rotation is counter-clockwise on screen (y-down), like the calligraphic angle.
            let base = if sc.rotation_relative_to_path { tg.y.atan2(tg.x) } else { 0.0 };
            let xf = Affine::translate(place.to_vec2()) * Affine::rotate(base - rot) * Affine::scale(size) * Affine::translate(-b.center().to_vec2());
            let mut n = sc.art.clone();
            n.transform(xf, true);
            out.take(Some(n));
            if out.full() {
                break;
            }
        }
    }
    out.out
}

/// Anchors `n` and its children hold. Walked with a stack rather than by recursion, and a tree
/// deeper or wider than [`MAX_WALK`] nodes is charged the whole budget rather than undercounted,
/// so an art tile cannot buy room by nesting.
fn anchors_of(n: &Node) -> usize {
    let (mut total, mut seen) = (0usize, 0usize);
    let mut stack = vec![n];
    while let Some(n) = stack.pop() {
        seen += 1;
        if seen > MAX_WALK {
            return MAX_ANCHORS;
        }
        if let NodeKind::Path { path, .. } = &n.kind {
            total = total.saturating_add(path.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>());
        }
        if let Some(c) = n.children() {
            stack.extend(c.iter().map(Arc::as_ref));
        }
    }
    total
}

/// Brush output collected under [`MAX_ANCHORS`].
///
/// A piece is taken whole or not at all, so the art a brush places is never half-drawn; the
/// total is therefore the budget plus at most one piece.
#[derive(Default)]
struct Budget {
    out: Vec<Node>,
    anchors: usize,
}

impl Budget {
    /// Take a piece if there is room. `None` (art that could not be placed) costs nothing.
    fn take(&mut self, n: Option<Node>) {
        let Some(n) = n else { return };
        if self.full() {
            return;
        }
        self.anchors = self.anchors.saturating_add(anchors_of(&n));
        self.out.push(n);
    }

    /// Whether the budget is spent, so the caller can stop walking the path.
    fn full(&self) -> bool {
        self.anchors >= MAX_ANCHORS
    }
}

/// A tile placed on [s0, s1] of the track (stretched along, `cross` across).
fn tile_on(t: &Track, tile: &(Node, Rect), s0: f64, s1: f64, cross: f64) -> Option<Node> {
    bend(t, &tile.0, tile.1, AlongMap::linear(tile.1.width(), s0, s1), cross)
}

pub(crate) fn pattern(pb: &PatternBrush, bp: &BezPath, weight: f64) -> Vec<Node> {
    let orient = |n: &Node| oriented(n, Direction::LeftToRight, pb.flip_along, pb.flip_across);
    let Some(side) = orient(&pb.side) else { return vec![] };
    let k = weight * pb.scale.max(1.0) / 100.0;
    let outer = pb.outer_corner.as_ref().and_then(orient);
    let inner = pb.inner_corner.as_ref().and_then(orient);
    let start = pb.start.as_ref().and_then(orient);
    let end = pb.end.as_ref().and_then(orient);
    let tw = side.1.width() * k;
    let gap = tw * pb.spacing.max(0.0) / 100.0;
    let mut out = Budget::default();
    for t in tracks(bp, 0.05) {
        if out.full() {
            break;
        }
        let l = t.len();
        if l <= 1e-9 {
            continue;
        }
        let corners = t.corner_positions();
        // Corner tiles, centred on the corner and turned to the bisector.
        let mut corner_half = 0.0_f64;
        for &(i, _) in &corners {
            let (a, b) = t.in_out(i);
            // Turning towards the left normal puts the tile's +y side on the inside.
            let tile = if a.cross(b) > 0.0 { inner.as_ref().or(outer.as_ref()) } else { outer.as_ref().or(inner.as_ref()) };
            let Some((art, bb)) = tile else { continue };
            corner_half = corner_half.max(bb.width() * k / 2.0);
            let bis = a + b;
            let bis = if bis.hypot() < 1e-9 { a } else { bis / bis.hypot() };
            let xf = Affine::translate(t.verts[i].to_vec2())
                * Affine::rotate(bis.y.atan2(bis.x))
                * Affine::scale(k)
                * Affine::translate(-bb.center().to_vec2());
            let mut n = art.clone();
            n.transform(xf, true);
            out.take(Some(n));
        }
        // Runs between corners (closed tracks wrap past the end).
        let mut bounds: Vec<f64> = corners.iter().map(|c| c.1).collect();
        let runs: Vec<(f64, f64, bool, bool)> = if bounds.is_empty() {
            vec![(0.0, l, false, false)]
        } else if t.closed {
            let first = bounds[0];
            bounds.push(first + l);
            bounds.windows(2).map(|w| (w[0], w[1], true, true)).collect()
        } else {
            let mut v = vec![0.0];
            v.extend(bounds);
            v.push(l);
            let n = v.len();
            v.windows(2).enumerate().map(|(i, w)| (w[0], w[1], i > 0, i + 2 < n)).collect()
        };
        let nruns = runs.len();
        let corner_tiles = inner.is_some() || outer.is_some();
        for (ri, (mut a, mut b, ca, cb)) in runs.into_iter().enumerate() {
            if ca && corner_tiles {
                a += corner_half;
            }
            if cb && corner_tiles {
                b -= corner_half;
            }
            if !t.closed
                && ri == 0
                && let Some(st) = &start
            {
                let w = st.1.width() * k;
                if b - a > w {
                    out.take(tile_on(&t, st, a, a + w, k));
                    a += w;
                }
            }
            if !t.closed
                && ri + 1 == nruns
                && let Some(en) = &end
            {
                let w = en.1.width() * k;
                if b - a > w {
                    out.take(tile_on(&t, en, b - w, b, k));
                    b -= w;
                }
            }
            let avail = b - a;
            if avail <= 1e-6 {
                continue;
            }
            let unit = tw + gap;
            match pb.fit {
                PatternFit::Stretch => {
                    let n = ((avail + gap) / unit).round().max(1.0) as usize;
                    let f = avail / (n as f64 * tw + (n as f64 - 1.0).max(0.0) * gap);
                    let (w, g) = (tw * f, gap * f);
                    for i in 0..n.min(20000) {
                        let s0 = a + i as f64 * (w + g);
                        out.take(tile_on(&t, &side, s0, s0 + w, k));
                        if out.full() {
                            break;
                        }
                    }
                }
                PatternFit::AddSpace => {
                    let n = ((avail + gap) / unit).floor().max(1.0) as usize;
                    let g = if n > 1 { (avail - n as f64 * tw) / (n as f64 - 1.0) } else { 0.0 };
                    let lead = if n == 1 { (avail - tw) / 2.0 } else { 0.0 };
                    for i in 0..n.min(20000) {
                        let s0 = a + lead + i as f64 * (tw + g.max(0.0));
                        out.take(tile_on(&t, &side, s0, s0 + tw, k));
                        if out.full() {
                            break;
                        }
                    }
                }
                PatternFit::Approximate => {
                    let n = ((avail + gap) / unit).round().max(1.0) as usize;
                    let used = n as f64 * tw + (n as f64 - 1.0) * gap;
                    let lead = (avail - used) / 2.0;
                    for i in 0..n.min(20000) {
                        let s0 = a + lead + i as f64 * unit;
                        out.take(tile_on(&t, &side, s0, s0 + tw, k));
                        if out.full() {
                            break;
                        }
                    }
                }
            }
        }
    }
    out.out
}
