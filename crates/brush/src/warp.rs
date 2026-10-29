//! Art, scatter and pattern brushes: art bent along the path by arc length.
//!
//! Art coordinates `(x, y)` map to the path point at arc length `s(x)` offset by
//! `(y - centre) × cross` along the path's left normal. Art outlines are flattened and
//! subdivided first so long straight edges bend with the path instead of cutting chords.

use std::sync::Arc;

use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind, PressureProfile};
use vectorcraft_geom::{Affine, BezPath, PathData, PathEl, Point, Rect, SubPath, Vec2};

use crate::track::{Rng, Track, normal, seed_of, tracks};
use crate::{ArtBrush, ArtScale, CornerSource, Direction, PatternBrush, PatternFit, Scatter, Variation, tolerance};

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
/// Points on each side of the box an Auto-Sliced pattern brush corner masks its tiles with.
const MASK_STEPS: usize = 32;

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
    /// Whether `at` is one linear function across the art, so it adds no bend of its own.
    ///
    /// [`AlongMap::linear`] always is: its outer two pieces have no width and the middle one
    /// spans everything. Between Guides is not, once its three pieces really do differ, which is
    /// the whole point of it.
    fn is_linear(&self) -> bool {
        let mut slope: Option<f64> = None;
        for i in 0..3 {
            // A piece with no width constrains nothing: no art x lands inside it.
            if self.xs[i + 1] - self.xs[i] <= 1e-12 {
                continue;
            }
            let s = self.slope(i);
            if slope.is_some_and(|first: f64| (first - s).abs() > 1e-9 * first.abs().max(1.0)) {
                return false;
            }
            slope = Some(s);
        }
        slope.is_some()
    }

    /// The arc lengths the art spans, low to high.
    fn span(&self) -> (f64, f64) {
        (self.ss[0].min(self.ss[3]), self.ss[0].max(self.ss[3]))
    }

    /// Largest along-scale (arc length per art unit) of the pieces.
    fn max_scale(&self) -> f64 {
        (0..3).map(|i| self.slope(i).abs()).fold(1e-6, f64::max)
    }
}

/// The affine that reproduces `f` over `b` to within `tol`, if one does.
///
/// Bending is only non-linear where the track bends. On a straight run the arc-length map and the
/// normal offset are both linear, so the art is just moved, turned and scaled, and flattening it
/// into chords throws its curves away for nothing (#3926).
///
/// Derived from three corners and then checked against a grid. The check is a second opinion, not
/// the gate: `bend` only calls this once the track and the along map have been read and found
/// straight, because sampling can reject a bend and cannot promise there is none (#3926 review).
fn affine_fit(f: &dyn Fn(Point) -> Point, b: Rect, tol: f64) -> Option<Affine> {
    let (w, h) = (b.width(), b.height());
    if !(w > 1e-9 && h > 1e-9 && tol.is_finite() && tol > 0.0) {
        return None;
    }
    let origin = Point::new(b.x0, b.y0);
    let q0 = f(origin);
    // Images of the unit x and y directions.
    let u = (f(Point::new(b.x1, b.y0)) - q0) / w;
    let v = (f(Point::new(b.x0, b.y1)) - q0) / h;
    if ![q0.x, q0.y, u.x, u.y, v.x, v.y].iter().all(|k| k.is_finite()) {
        return None;
    }
    let a = Affine::translate(q0.to_vec2()) * Affine::new([u.x, u.y, v.x, v.y, 0.0, 0.0]) * Affine::translate(-origin.to_vec2());
    // Five by five including the corners the fit came from, so a map that merely agrees at those
    // three is rejected.
    for i in 0..=4 {
        for j in 0..=4 {
            let p = Point::new(b.x0 + w * i as f64 / 4.0, b.y0 + h * j as f64 / 4.0);
            let (want, got) = (f(p), a * p);
            if !(want.x.is_finite() && want.y.is_finite()) || want.distance(got) > tol {
                return None;
            }
        }
    }
    Some(a)
}

/// [`warp_node`] for a map that is affine: the art is transformed rather than flattened, so its
/// curves survive.
///
/// Everything else matches [`warp_node`] deliberately, so curves are the only difference between
/// the two routes: the same node kinds are kept and the rest dropped, `live` is cleared on every
/// path, an empty path or a group left with no children is dropped, and strokes are scaled by
/// `width_scale` at every level rather than by the transform's own factor.
fn affine_node(n: &Node, a: Affine, width_scale: f64) -> Option<Node> {
    let mut out = prepared(n, width_scale)?;
    out.transform(a, false);
    Some(out)
}

/// The part of [`affine_node`] that mirrors [`warp_node`]'s walk, before the single transform.
fn prepared(n: &Node, width_scale: f64) -> Option<Node> {
    let mut out = n.clone();
    out.appearance.scale_strokes(width_scale);
    match &mut out.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None;
            if path.is_empty() {
                return None;
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            let kids: Vec<Arc<Node>> = children.iter().filter_map(|c| prepared(c, width_scale)).map(Arc::new).collect();
            if kids.is_empty() {
                return None;
            }
            *children = kids;
        }
        _ => return None,
    }
    Some(out)
}

/// Bend `art` (already oriented, bounds `b`) along `t` using `along` and cross scale `cross`.
fn bend(t: &Track, art: &Node, b: Rect, along: AlongMap, cross: f64) -> Option<Node> {
    let cy = b.center().y;
    let x0 = b.x0;
    let f = move |p: Point| t.map(along.at(p.x - x0), (p.y - cy) * cross);
    let scale = along.max_scale().max(cross.abs()).max(1e-6);
    let flat_tol = tolerance(b.height().max(b.width()) * scale) / scale;
    // A straight run needs no bending: move the art and keep its curves.
    //
    // Straightness is read off the track and the along map, not sampled. A bend anywhere in the
    // span is two segment directions that differ, so this cannot miss one however short the bent
    // part is; the five by five grid below could, since a wiggle narrower than its spacing passes
    // between the samples (#3926 review). The grid now only checks the affine that structure has
    // already said exists, so it can reject and never admit.
    let (s_lo, s_hi) = along.span();
    if along.is_linear()
        && t.is_straight(s_lo, s_hi)
        && let Some(a) = affine_fit(&f, b, flat_tol.max(1e-9))
    {
        return affine_node(art, a, cross.abs());
    }
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

/// What a pattern brush draws at a corner: corner tile art (oriented, with its bounds), or what
/// [`CornerSource`] gives.
#[derive(Clone, Copy)]
enum Corner<'a> {
    Art(&'a (Node, Rect)),
    Source(CornerSource),
}

/// The pieces of the corner tile that `source` makes at corner `i` of `t` from `side` (oriented,
/// with its bounds) at scale `k`, and the length of path on each side of the corner they cover,
/// which is at most `room`.
///
/// At the corner, the pieces run straight along the directions the path arrives at and leaves
/// the corner in, and towards their far ends they move onto the path itself, so they meet the side
/// tiles where the path curves. The corner's diagonal is the line through the corner that halves
/// the turn, where the two offset edges of the band meet. [`CornerSource::None`] and a turn of
/// nearly 180° get no pieces.
fn auto_corner(t: &Track, i: usize, source: CornerSource, side: &(Node, Rect), k: f64, room: f64) -> (Vec<Node>, f64) {
    let (art, b) = side;
    let (a_dir, b_dir) = t.in_out(i);
    let (Some(&p), Some(&sc)) = (t.verts.get(i), t.cum.get(i)) else { return (vec![], 0.0) };
    let bt = a_dir + b_dir;
    if source == CornerSource::None || a_dir.dot(bt) < 1e-3 {
        return (vec![], 0.0);
    }
    let (tw, r) = (b.width() * k, b.height() * k / 2.0);
    let (x0, cy) = (b.x0, b.center().y);
    // Art point → (distance along from the left edge, offset across), in points.
    let local = move |q: Point| ((q.x - x0) * k, (q.y - cy) * k);
    // How far along `d` (from the corner) the diagonal is at offset `y` across `d`'s left normal.
    let diag = move |d: Vec2, y: f64| -y * normal(d).dot(bt) / d.dot(bt);
    // `u` along `d` from the corner and `y` across, moved from the straight line onto the path as
    // `u` goes from `from` to `far` (signed: negative before the corner; `from` is the corner or
    // a point between it and `far`).
    let on_path = move |d: Vec2, u: f64, y: f64, from: f64, far: f64| {
        let straight = p + d * u + normal(d) * y;
        let span = far - from;
        let w = if span.abs() > 1e-9 { ((u - from) / span).clamp(0.0, 1.0) } else { 0.0 };
        if w > 0.0 { straight.lerp(t.map(sc + u, y), w) } else { straight }
    };
    // The most the diagonal is ahead of the corner, along either direction, within the band. Each
    // source stretches the side tile along to `len` where the band is too tall for the tile to
    // reach the diagonal, so the two sides always meet on it.
    let ahead = r * normal(a_dir).dot(bt).abs() / a_dir.dot(bt);
    // Before the corner, `u` in [-len, 0] maps onto [-len, diagonal]; after it, [0, len] onto
    // [diagonal, len]. Both ends stay on the diagonal when `len` is shorter than `ahead`, and the
    // points on the diagonal stay on the straight lines, where the two sides meet.
    let into = move |u: f64, y: f64, len: f64| {
        let reach = len.max(ahead);
        let e = diag(a_dir, y).clamp(-reach, reach);
        on_path(a_dir, -len + (u + len) * (e + len) / len.max(1e-9), y, e.min(0.0), -len)
    };
    let out_of = move |u: f64, y: f64, len: f64| {
        let reach = len.max(ahead);
        let e = diag(b_dir, y).clamp(-reach, reach);
        on_path(b_dir, e + u * (len - e) / len.max(1e-9), y, e.max(0.0), len)
    };
    let flat_tol = tolerance(b.height().max(b.width()) * k) / k;
    // The tile is squeezed along so that it covers at most `room` of the path on each side.
    let room = room.max(0.0);
    let len = tw.max(2.0 * ahead).min(match source {
        CornerSource::AutoCentered => 2.0 * room,
        CornerSource::AutoBetween => room,
        _ => room + ahead,
    });
    if len <= 1e-9 {
        return (vec![], 0.0);
    }
    let s = if tw > 1e-9 { len / tw } else { 1.0 };
    let max_du = MAX_SEG / (2.0 * k * s.max(1.0));
    let warp = |f: &dyn Fn(Point) -> Point| warp_node(art, f, flat_tol, max_du, k);
    // Art point → (distance along from the left edge, stretched to `len`, and offset across).
    let along = move |q: Point| {
        let (x, y) = local(q);
        (x * s, y)
    };
    match source {
        CornerSource::None => (vec![], 0.0),
        CornerSource::AutoCentered => {
            let half = len / 2.0;
            let f = move |q: Point| {
                let (x, y) = along(q);
                let u = x - half;
                if u < 0.0 { into(u, y, half) } else { out_of(u, y, half) }
            };
            (warp(&f).into_iter().collect(), half)
        }
        CornerSource::AutoBetween => {
            let before = move |q: Point| {
                let (x, y) = along(q);
                into(x - len, y, len)
            };
            let after = move |q: Point| {
                let (x, y) = along(q);
                out_of(x, y, len)
            };
            (warp(&before).into_iter().chain(warp(&after)).collect(), len)
        }
        CornerSource::AutoSliced | CornerSource::AutoOverlap => {
            // Each side's tile runs `ahead` past the corner.
            let far = len - ahead;
            let before = move |q: Point| {
                let (x, y) = along(q);
                on_path(a_dir, x - far, y, 0.0, -far)
            };
            let after = move |q: Point| {
                let (x, y) = along(q);
                on_path(b_dir, x - ahead, y, 0.0, far)
            };
            let (first, second) = (warp(&before), warp(&after));
            if source == CornerSource::AutoOverlap {
                return (first.into_iter().chain(second).collect(), far);
            }
            // Auto-Sliced masks each tile by the part of the outline of the box round its art
            // (strokes included), placed as the tile is, on its own side of the diagonal. A tile
            // squeezed along keeps its stroke width, so the box's margins at its ends grow by as
            // much as the tile is squeezed.
            let vb = art.visual_bounds().unwrap_or(*b);
            let grow = 1.0 / s.min(1.0);
            let (x0, x1) = (b.x0 - (b.x0 - vb.x0).max(0.0) * grow, b.x1 + (vb.x1 - b.x1).max(0.0) * grow);
            let corners = [(x0, vb.y0), (x1, vb.y0), (x1, vb.y1), (x0, vb.y1), (x0, vb.y0)].map(|(x, y)| Point::new(x, y));
            let outline = |f: &dyn Fn(Point) -> Point| -> Vec<Point> {
                let steps = (0..MASK_STEPS).map(|j| j as f64 / MASK_STEPS as f64);
                corners.windows(2).flat_map(|w| steps.clone().map(move |s| w[0].lerp(w[1], s))).map(f).collect()
            };
            let pieces =
                first.and_then(|n| masked(n, &outline(&before), p, bt)).into_iter().chain(second.and_then(|n| masked(n, &outline(&after), p, -bt)));
            (pieces.collect(), far)
        }
    }
}

/// `n` in a clipping group whose mask is the part of polygon `quad` on the side of the line through
/// `p` that `towards` points away from. `None` when nothing of `quad` is on that side.
fn masked(n: Node, quad: &[Point], p: Point, towards: Vec2) -> Option<Node> {
    let side = |q: Point| (q - p).dot(towards);
    let mut kept = vec![];
    for (j, &a) in quad.iter().enumerate() {
        let b = quad.get(j + 1).or(quad.first()).copied().unwrap_or(a);
        let (da, db) = (side(a), side(b));
        if da <= 0.0 {
            kept.push(a);
        }
        if (da <= 0.0) != (db <= 0.0) {
            kept.push(a.lerp(b, da / (da - db)));
        }
    }
    if kept.len() < 3 {
        return None;
    }
    let mask = Node::path(NodeId(0), PathData::new(vec![SubPath::polyline(&kept, true)]), Appearance::default());
    Some(Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(mask), Arc::new(n)], clip: true }))
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
    // Each kind of corner takes its own art, else its source, else the other kind's art.
    let outer_tile = outer.as_ref().map(Corner::Art).or(pb.outer_source.map(Corner::Source));
    let inner_tile = inner.as_ref().map(Corner::Art).or(pb.inner_source.map(Corner::Source));
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
        // Corner tiles, centred on the corner and turned to the bisector. The path on each side of
        // a corner is shortened by `trims` (`None`: half the widest corner tile art).
        let mut corner_half = 0.0_f64;
        let mut trims: Vec<Option<f64>> = Vec::with_capacity(corners.len());
        // The path a tile made at corner `ci` may cover on each side: up to the ends of an open
        // track, and half the way to the next corner.
        let room = |ci: usize| {
            let at = |j: usize| corners.get(j).map(|c| c.1);
            let s = at(ci).unwrap_or(0.0);
            let last = corners.len().saturating_sub(1);
            let before = match (ci.checked_sub(1).and_then(at), t.closed) {
                (Some(prev), _) => (s - prev) / 2.0,
                (None, true) => (s - (at(last).unwrap_or(s) - l)) / 2.0,
                (None, false) => s,
            };
            let after = match (at(ci + 1), t.closed) {
                (Some(next), _) => (next - s) / 2.0,
                (None, true) => (at(0).unwrap_or(s) + l - s) / 2.0,
                (None, false) => l - s,
            };
            before.min(after)
        };
        for (ci, &(i, _)) in corners.iter().enumerate() {
            if out.full() {
                break;
            }
            let (a, b) = t.in_out(i);
            // Turning towards the left normal puts the tile's +y side on the inside.
            let tile = if a.cross(b) > 0.0 { inner_tile.or(outer.as_ref().map(Corner::Art)) } else { outer_tile.or(inner.as_ref().map(Corner::Art)) };
            let (art, bb) = match tile {
                Some(Corner::Art(tile)) => tile,
                Some(Corner::Source(source)) => {
                    let (pieces, trim) = auto_corner(&t, i, source, &side, k, room(ci));
                    for n in pieces {
                        out.take(Some(n));
                    }
                    trims.push(Some(trim));
                    continue;
                }
                None => {
                    trims.push(Some(0.0));
                    continue;
                }
            };
            trims.push(None);
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
        let trim = |c: Option<usize>| c.and_then(|c| trims.get(c).copied()).map_or(0.0, |t| t.unwrap_or(corner_half));
        // Runs between corners (closed tracks wrap past the end), with the corners they start and
        // end at.
        let mut bounds: Vec<f64> = corners.iter().map(|c| c.1).collect();
        let nc = bounds.len();
        let runs: Vec<(f64, f64, Option<usize>, Option<usize>)> = if bounds.is_empty() {
            vec![(0.0, l, None, None)]
        } else if t.closed {
            let first = bounds[0];
            bounds.push(first + l);
            bounds.windows(2).enumerate().map(|(i, w)| (w[0], w[1], Some(i), Some((i + 1) % nc))).collect()
        } else {
            let mut v = vec![0.0];
            v.extend(bounds);
            v.push(l);
            v.windows(2).enumerate().map(|(i, w)| (w[0], w[1], i.checked_sub(1), (i < nc).then_some(i))).collect()
        };
        let nruns = runs.len();
        for (ri, (mut a, mut b, ca, cb)) in runs.into_iter().enumerate() {
            if out.full() {
                break;
            }
            a += trim(ca);
            b -= trim(cb);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::tracks;

    /// #3926 review: the five by five grid cannot promise a run is straight, only that it looks
    /// straight where it was asked. A zigzag whose vertices land on the sample positions is
    /// invisible to it: every sample sits on a y = 0 vertex whose blended tangent is flat, so all
    /// 25 points agree with the affine exactly, while the art between them is thrown 4 pt off.
    ///
    /// This is why the gate is `Track::is_straight` and the grid is only a second opinion.
    #[test]
    fn a_bend_shorter_than_the_sample_grid_is_still_a_bend() {
        // Half-period 20 pt, amplitude 4 pt. The 11.3 degree turn is under `CORNER_DEG`, so the
        // vertices are not corners and their tangents blend flat, which is what hides them.
        let (d, a) = (20.0, 4.0);
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        for i in 1..=16 {
            bp.line_to((d * i as f64, if i % 2 == 1 { a } else { 0.0 }));
        }
        let t = tracks(&bp, 0.01).remove(0);
        assert_eq!(t.corner.iter().filter(|c| **c).count(), 0, "no vertex is a corner, so every tangent is blended");

        // A span over four full periods, held clear of both ends so no sample hits the
        // extrapolation branch: the five samples land on vertices 4, 6, 8, 10 and 12, all at y = 0.
        let seglen = (d * d + a * a).sqrt();
        let (s0, s1) = (4.0 * seglen, 12.0 * seglen);
        let b = Rect::new(0.0, -20.0, 40.0, 20.0);
        let along = AlongMap::linear(b.width(), s0, s1);
        let (cy, x0, cross) = (b.center().y, b.x0, 1.0);
        let f = |p: Point| t.map(along.at(p.x - x0), (p.y - cy) * cross);
        let scale = along.max_scale().max(cross.abs()).max(1e-6);
        let flat_tol = tolerance(b.height().max(b.width()) * scale) / scale;

        // The grid is satisfied, and it should not be.
        let fit = affine_fit(&f, b, flat_tol.max(1e-9)).expect("the sampled check accepts this bend");

        // How wrong it is: an art x landing on a peak of the zigzag, which no sample reaches.
        let peak_x = x0 + (5.0 * seglen - s0) / (s1 - s0) * b.width();
        let at_peak = Point::new(peak_x, cy);
        let off = (f(at_peak) - fit * at_peak).hypot();
        assert!(
            off > 3.0 && off > flat_tol * 20.0,
            "the affine the grid approved is {off:.3} pt out at the peak, against a {flat_tol:.4} pt tolerance"
        );

        // Reading the track instead refuses it, and still admits the straight run it is made of.
        assert!(!t.is_straight(s0, s1), "a bend between the samples is still a bend");
        let straight = tracks(
            &{
                let mut bp = BezPath::new();
                bp.move_to((0.0, 0.0));
                bp.line_to((320.0, 0.0));
                bp
            },
            0.01,
        )
        .remove(0);
        assert!(straight.is_straight(s0, s1), "and the same span of an actually straight track is straight");

        // End to end through `bend`: the aliased bend is subdivided, the straight run keeps its
        // curves. This is the gate itself, not the two predicates it is made of.
        let anchors = |n: &Node| match &n.kind {
            NodeKind::Path { path, .. } => path.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>(),
            _ => 0,
        };
        let circle = Node::path(
            vectorcraft_doc::NodeId(0),
            vectorcraft_geom::shapes::ellipse(b),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0),
        );
        assert_eq!(anchors(&circle), 4, "the art is a 4-anchor circle to begin with");
        let bent = bend(&t, &circle, b, along, cross).expect("the zigzag still draws something");
        assert!(anchors(&bent) > 8, "the aliased bend is subdivided, not affine-mapped: {} anchors", anchors(&bent));
        let moved = bend(&straight, &circle, b, along, cross).expect("the straight run still draws something");
        assert_eq!(anchors(&moved), 4, "the straight run keeps the tile's curves");
    }
}
