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

pub(crate) fn art(a: &ArtBrush, bp: &BezPath, weight: f64) -> Vec<Node> {
    let Some((art, b)) = oriented(&a.art, a.direction, a.flip_along, a.flip_across) else { return vec![] };
    let cross = weight * a.width / 100.0;
    let mut out = vec![];
    for t in tracks(bp, 0.05) {
        let l = t.len();
        if l <= 1e-9 {
            continue;
        }
        let w = b.width();
        let (along, cross) = match a.scale {
            ArtScale::Stretch => (AlongMap::linear(w, 0.0, l), cross),
            ArtScale::Proportional => {
                let k = l / w;
                (AlongMap::linear(w, 0.0, l), k * a.width / 100.0)
            }
            ArtScale::BetweenGuides { start, end } => {
                let (g0, g1) = (start.clamp(0.0, 1.0) * w, end.clamp(0.0, 1.0) * w);
                let (g0, g1) = (g0.min(g1), g0.max(g1));
                let fixed = (g0 + (w - g1)) * cross;
                if fixed >= l || g1 - g0 < 1e-9 {
                    (AlongMap::linear(w, 0.0, l), cross)
                } else {
                    (AlongMap { xs: [0.0, g0, g1, w], ss: [0.0, g0 * cross, l - (w - g1) * cross, l] }, cross)
                }
            }
        };
        if let Some(n) = bend(&t, &art, b, along, cross) {
            out.push(n);
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
    let mut out = vec![];
    for t in tracks(bp, 0.05) {
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
            out.push(n);
        }
    }
    out
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
    let mut out = vec![];
    for t in tracks(bp, 0.05) {
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
            out.push(n);
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
                    out.extend(tile_on(&t, st, a, a + w, k));
                    a += w;
                }
            }
            if !t.closed
                && ri + 1 == nruns
                && let Some(en) = &end
            {
                let w = en.1.width() * k;
                if b - a > w {
                    out.extend(tile_on(&t, en, b - w, b, k));
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
                        out.extend(tile_on(&t, &side, s0, s0 + w, k));
                    }
                }
                PatternFit::AddSpace => {
                    let n = ((avail + gap) / unit).floor().max(1.0) as usize;
                    let g = if n > 1 { (avail - n as f64 * tw) / (n as f64 - 1.0) } else { 0.0 };
                    let lead = if n == 1 { (avail - tw) / 2.0 } else { 0.0 };
                    for i in 0..n.min(20000) {
                        let s0 = a + lead + i as f64 * (tw + g.max(0.0));
                        out.extend(tile_on(&t, &side, s0, s0 + tw, k));
                    }
                }
                PatternFit::Approximate => {
                    let n = ((avail + gap) / unit).round().max(1.0) as usize;
                    let used = n as f64 * tw + (n as f64 - 1.0) * gap;
                    let lead = (avail - used) / 2.0;
                    for i in 0..n.min(20000) {
                        let s0 = a + lead + i as f64 * unit;
                        out.extend(tile_on(&t, &side, s0, s0 + tw, k));
                    }
                }
            }
        }
    }
    out
}
