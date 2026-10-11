//! Live Paint planar maps: the faces and edges a set of paths divides the plane into.
//!
//! Unlike Pathfinder [`regions`](crate::regions), which splits the *filled areas* of closed shapes,
//! every path here is an edge, open ones included: three crossing lines enclose a triangle, a line
//! across a rectangle splits it in two, and an area enclosed by paths is a face whether or not
//! anything fills it.
//!
//! The sweep (`linesweeper`) only takes closed paths, so an open path goes in traced there and
//! back. Its two passes would cancel out as winding numbers, so the sweep counts boundary crossings
//! instead ([`Crossings`]), which never cancel. Faces are then walked on the planar graph of the
//! sweep's output segments; which inputs an edge came from and which fills cover a face are found
//! geometrically (the sweep's winding numbers can't be trusted once open paths and coincident
//! edges are in).
//!
//! The sweep runs in a sheared frame ([`SHEAR`]): it merges the two passes of a *horizontal* open
//! path as if they cancelled out, and in the sheared frame no edge drawn at a common angle (0°,
//! 45°, 90°…) is horizontal. The shear is by a power of two, so it maps typical coordinates
//! exactly and keeps exact coincidences (a corner on another shape's edge) exact.

use std::collections::HashMap;

use kurbo::{Affine, BezPath, CubicBez, ParamCurve, PathSeg, Point, Shape as _};
use linesweeper::topology::{Topology, WindingNumber};
use vectorcraft_geom::{FillRule, PathData, SubPath};

use crate::boolean::{DEFAULT_PRECISION, Seg, Tidy, eps_for, is_sliver, segs_to_subpath, snap_horizontals, tidy_segments, to_seg};
use crate::pathfinder::{Region, Shape};

/// The sweep's frame: y + x / 32. Only edges sloping -1/32 in the document are horizontal in it.
const SHEAR: Affine = Affine::new([1.0, 1.0 / 32.0, 0.0, 1.0, 0.0, 0.0]);
const UNSHEAR: Affine = Affine::new([1.0, -1.0 / 32.0, 0.0, 1.0, 0.0, 0.0]);

/// Boundary crossings per input, counted from the far west: slot `2i` counts input `i`'s
/// segments running one way, slot `2i + 1` the other. An open path traced there and back adds one
/// to each, so its edges never cancel out of the map.
#[derive(Clone, Debug, Default)]
struct Crossings(Vec<u32>);

impl Crossings {
    fn get(&self, slot: usize) -> u32 {
        self.0.get(slot).copied().unwrap_or(0)
    }
}

impl PartialEq for Crossings {
    fn eq(&self, other: &Self) -> bool {
        (0..self.0.len().max(other.0.len())).all(|i| self.get(i) == other.get(i))
    }
}
impl Eq for Crossings {}

impl std::ops::Add for Crossings {
    type Output = Crossings;
    fn add(mut self, rhs: Self) -> Crossings {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Crossings {
    fn add_assign(&mut self, rhs: Self) {
        if self.0.len() < rhs.0.len() {
            self.0.resize(rhs.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(rhs.0) {
            *a = a.saturating_add(b);
        }
    }
}

impl WindingNumber for Crossings {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        let slot = 2 * tag + usize::from(!positive);
        let mut v = vec![0; slot + 1];
        if let Some(c) = v.get_mut(slot) {
            *c = 1;
        }
        Crossings(v)
    }
    fn of_tag(&self, tag: usize) -> Self {
        let mut v = vec![0; 2 * tag + 2];
        for slot in [2 * tag, 2 * tag + 1] {
            if let Some(c) = v.get_mut(slot) {
                *c = self.get(slot);
            }
        }
        Crossings(v)
    }
}

/// A point in the middle of a run of segments.
fn mid(segs: &[Seg]) -> Option<Point> {
    segs.get(segs.len() / 2).map(|s| s.c.eval(0.5))
}

/// The inputs' segments (in the sweep's frame), to tell which inputs an edge lies on.
struct Outlines {
    segs: Vec<(kurbo::Rect, PathSeg, usize)>,
    tol: f64,
}

impl Outlines {
    fn new(inputs: &[(BezPath, usize)], eps: f64) -> Self {
        // A zero-length segment (a point) is the source of no edge, even one running through it.
        let segs = inputs
            .iter()
            .flat_map(|(bp, i)| bp.segments().map(move |s| (s.bounding_box(), s, *i)))
            .filter(|(r, ..)| r.width() > 0.0 || r.height() > 0.0)
            .collect();
        // The sweep moves points by a few `eps` at most.
        Self { segs, tol: 16.0 * eps }
    }

    /// The front-most input whose outline passes through `p`.
    fn source_at(&self, p: Point) -> Option<usize> {
        use kurbo::ParamCurveNearest;
        let tol = self.tol;
        self.segs.iter().filter(|(r, s, _)| r.inflate(tol, tol).contains(p) && s.nearest(p, 1e-9).distance_sq <= tol * tol).map(|(_, _, i)| *i).max()
    }
}

/// The area an input's fill covers: its closed subpaths (open ones bound no face).
struct Fill {
    path: BezPath,
    bounds: kurbo::Rect,
    rule: FillRule,
}

impl Fill {
    fn new(s: &Shape) -> Self {
        let mut path = BezPath::new();
        for sp in s.path.subpaths.iter().filter(|sp| sp.closed && sp.anchors.len() >= 2) {
            sp.to_bezpath_into(&mut path);
        }
        Self { bounds: path.bounding_box(), path, rule: s.rule }
    }

    fn contains(&self, p: Point) -> bool {
        !self.path.elements().is_empty() && self.bounds.contains(p) && crate::boolean::inside(self.rule, self.path.winding(p))
    }
}

/// One output segment of the sweep: an edge of the planar graph between two vertices.
struct Edge {
    segs: Vec<Seg>,
    ends: [usize; 2],
    /// The input it came from (the front-most, where inputs share it).
    source: usize,
}

/// `segs` (a chain) moved to start at `a` and end at `b`, the handles next to each moving with it.
fn pin_ends(segs: &mut [Seg], a: Point, b: Point) {
    if let Some(s) = segs.first_mut() {
        let d = a - s.c.p0;
        s.c.p0 = a;
        s.c.p1 += d;
    }
    if let Some(s) = segs.last_mut() {
        let d = b - s.c.p3;
        s.c.p3 = b;
        s.c.p2 += d;
    }
}

/// Half-edge `2e` runs along edge `e`, `2e + 1` back along it.
fn twin(h: usize) -> usize {
    h ^ 1
}

/// The segments of half-edge `h`, in its direction.
fn half_segs(edges: &[Edge], h: usize) -> Vec<Seg> {
    let Some(e) = edges.get(h / 2) else { return Vec::new() };
    if h.is_multiple_of(2) {
        e.segs.clone()
    } else {
        e.segs.iter().rev().map(|s| Seg { c: CubicBez::new(s.c.p3, s.c.p2, s.c.p1, s.c.p0), line: s.line }).collect()
    }
}

/// The vertex half-edge `h` leaves from.
fn origin(edges: &[Edge], h: usize) -> Option<usize> {
    edges.get(h / 2).map(|e| e.ends[h % 2])
}

/// Where half-edge `h` (leaving `v`) first gets `r` away from it, as an angle around `v`.
///
/// Edges that only meet at `v` cross a circle around it in the order they leave `v` in, however
/// they start out (tangent curves, or the sweep's short straight stubs at a vertex).
fn leaving_angle(edges: &[Edge], h: usize, v: Point, r: f64) -> f64 {
    let segs = half_segs(edges, h);
    let mut bp = BezPath::new();
    for (k, s) in segs.iter().enumerate() {
        if k == 0 {
            bp.move_to(s.c.p0);
        }
        bp.curve_to(s.c.p1, s.c.p2, s.c.p3);
    }
    let mut prev = segs.first().map_or(v, |s| s.c.p0);
    let mut hit: Option<Point> = None;
    kurbo::flatten(bp.iter(), (r * 1e-4).max(1e-12), |el| {
        if let (None, kurbo::PathEl::LineTo(p)) = (hit, el) {
            if p.distance(v) >= r {
                // The point on prev → p at distance r (prev is closer).
                let (d, f) = (p - prev, prev - v);
                let (a, b, c) = (d.hypot2(), 2.0 * f.dot(d), f.hypot2() - r * r);
                let t = if a > 0.0 { ((-b + (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a)).clamp(0.0, 1.0) } else { 1.0 };
                hit = Some(prev + d * t);
            }
            prev = p;
        }
    });
    // An edge too short to reach r (it can't, r is at most half of every chord) leaves along its
    // tangent.
    let far =
        hit.unwrap_or_else(|| segs.first().map_or(v, |s| [s.c.p1, s.c.p2, s.c.p3].into_iter().find(|p| p.distance(s.c.p0) > 0.0).unwrap_or(s.c.p3)));
    (far - v).atan2()
}

/// Path of a closed run of half-edges, tidied (split pieces of one input curve joined again).
fn cycle_bezpath(edges: &[Edge], cycle: &[usize]) -> BezPath {
    let mut bp = BezPath::new();
    for (k, s) in cycle.iter().flat_map(|&h| half_segs(edges, h)).enumerate() {
        if k == 0 {
            bp.move_to(s.c.p0);
        }
        if s.line {
            bp.line_to(s.c.p3);
        } else {
            bp.curve_to(s.c.p1, s.c.p2, s.c.p3);
        }
    }
    bp.close_path();
    bp
}

/// `cycle` without the spurs it runs out and back along (edges with the same face on both sides,
/// such as a line's loose end).
fn without_spurs(cycle: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(cycle.len());
    for &h in cycle {
        if out.last() == Some(&twin(h)) {
            out.pop();
        } else {
            out.push(h);
        }
    }
    // A spur where the walk started wraps around the ends.
    while out.len() >= 2 && out.first().map(|&h| twin(h)) == out.last().copied() {
        out.pop();
        out.remove(0);
    }
    out
}

/// Append `open` traced there and back (a closed, zero-area path along it) to `out`.
fn there_and_back(open: &BezPath, out: &mut BezPath) {
    let segs: Vec<PathSeg> = open.segments().collect();
    let Some(first) = segs.first() else { return };
    out.move_to(first.start());
    for s in &segs {
        out.push(s.as_path_el());
    }
    for s in segs.iter().rev() {
        out.push(s.reverse().as_path_el());
    }
    out.close_path();
}

/// `path` back from the sweep's frame, its anchors within `tol` of an input anchor (`sorted` by x)
/// put exactly on it, handles moving with them.
fn unshear(mut path: PathData, sorted: &[Point], tol: f64) -> PathData {
    path.transform(UNSHEAR);
    for a in path.subpaths.iter_mut().flat_map(|sp| sp.anchors.iter_mut()) {
        let lo = sorted.partition_point(|q| q.x < a.p.x - tol);
        if let Some(q) = sorted.get(lo..).unwrap_or_default().iter().take_while(|q| q.x <= a.p.x + tol).find(|q| (q.y - a.p.y).abs() <= tol) {
            let d = *q - a.p;
            a.p = *q;
            a.h_in += d;
            a.h_out += d;
        }
    }
    path
}

fn find(parent: &mut [usize], mut v: usize) -> usize {
    while let Some(&p) = parent.get(v) {
        if p == v {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(v) {
            *slot = gp;
        }
        v = p;
    }
    v
}

/// The Live Paint planar map of `shapes` (back → front).
///
/// Faces are the bounded areas the paths enclose, holes included, each with the inputs whose fill
/// covers it (ascending; empty for an area only enclosed by paths). Edges are the paths split
/// wherever they meet another path, keyed by the input they came from (the front-most where
/// inputs share one). Empty when the paths can't be swept (non-finite or too degenerate).
pub fn live_paint(shapes: &[Shape]) -> (Vec<Region>, Vec<Shape>) {
    build(shapes).unwrap_or_default()
}

/// Most segments the Shape Builder sweeps with open paths as edges: the planar map costs far more
/// than the filled areas' arrangement, so past this only the filled areas count.
pub const SHAPE_BUILDER_MAX_SEGMENTS: usize = 4096;

/// The Shape Builder's arrangement: see [`shape_builder`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuilderArrangement {
    pub regions: Vec<Region>,
    /// The pieces of the paths with no closed subpath, keyed by input index.
    pub lines: Vec<Shape>,
    /// The pieces of the other paths' outlines, keyed by input index (the front-most where inputs
    /// share one); only when asked for.
    pub edges: Vec<Shape>,
}

/// Shape Builder regions of `shapes` (back → front), the pieces of their open paths and, with
/// `edges`, the pieces of their outlines (what erasing an edge deletes).
///
/// Closed paths bound regions with their filled areas, as in [`regions`](crate::regions). Open
/// paths (the caller closes the ones whose fill should count) are cutting edges: a line across a
/// shape splits it, lines around an area enclose a region nothing fills (empty `sources`), and each
/// path with no closed subpath comes back cut wherever another path meets it, as pieces keyed by
/// its index in `shapes`. Outlines are cut the same way. With no open path, more than
/// [`SHAPE_BUILDER_MAX_SEGMENTS`] segments or a map that can't be built, only the filled areas
/// count and there are no pieces.
pub fn shape_builder(shapes: &[Shape], edges: bool) -> BuilderArrangement {
    let has_open = shapes.iter().flat_map(|s| &s.path.subpaths).any(|sp| !sp.closed && sp.anchors.len() >= 2);
    let segments: usize = shapes.iter().flat_map(|s| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
    let planar = ((has_open || edges) && segments <= SHAPE_BUILDER_MAX_SEGMENTS)
        .then(|| {
            let keyed: Vec<Shape> = shapes.iter().enumerate().map(|(i, s)| Shape::new(s.path.clone(), s.rule, i as u64)).collect();
            live_paint(&keyed)
        })
        .filter(|(faces, pieces)| !faces.is_empty() || !pieces.is_empty());
    let Some((faces, pieces)) = planar else {
        return BuilderArrangement { regions: crate::regions(shapes), ..Default::default() };
    };
    let is_line = |i: u64| shapes.get(i as usize).is_some_and(|s| !s.path.subpaths.iter().any(|sp| sp.closed));
    let (lines, outline): (Vec<Shape>, Vec<Shape>) = pieces.into_iter().partition(|e| is_line(e.key));
    BuilderArrangement {
        regions: if has_open { faces } else { crate::regions(shapes) },
        lines: if has_open { lines } else { Vec::new() },
        edges: if edges { outline } else { Vec::new() },
    }
}

/// Does `sp`, closed, enclose an area (rather than retrace itself)? The Shape Builder closes the
/// open subpaths of filled paths that do.
pub fn encloses_area(sp: &SubPath) -> bool {
    let mut closed = sp.clone();
    closed.closed = true;
    let mut bp = BezPath::new();
    closed.to_bezpath_into(&mut bp);
    let size = bp.bounding_box().size();
    bp.area().abs() > 1e-9 * (size.width * size.width + size.height * size.height)
}

/// `path` without `piece`, a run along one of its subpaths between two points (a Shape Builder
/// edge): a closed subpath opens there, an open one splits in two (or loses an end), and a piece
/// that runs all the way round takes its subpath. With `fill_closes`, open subpaths that enclose an
/// area ([`encloses_area`]) run on round an invisible closing edge, as the Shape Builder sees a
/// filled path: a piece may run over it, and it never comes back as a visible segment. None when
/// the piece doesn't lie on `path` within `tol`.
pub fn cut_out(path: &PathData, piece: &SubPath, fill_closes: bool, tol: f64) -> Option<PathData> {
    let n = piece.segment_count();
    if n == 0 {
        return None;
    }
    // A point well inside the piece: the middle of its longest segment.
    let arclen = |k: usize| kurbo::ParamCurveArclen::arclen(&piece.segment(k), 1e-3);
    let mid = (0..n).map(|k| (arclen(k), k)).max_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, k)| piece.segment(k).eval(0.5))?;
    let (a, b) = (piece.anchors.first()?.p, piece.anchors.last()?.p);
    let mut bp = BezPath::new();
    piece.to_bezpath_into(&mut bp);
    if bp.perimeter(1e-6) <= tol {
        // A speck (where paths touch): there's nothing to cut.
        return None;
    }
    // A piece that ends where it starts runs all the way round.
    let whole = piece.closed || a.distance(b) <= tol;
    // Each subpath as the piece may run along it: round its closing edge when it has one.
    let ring = |sp: &SubPath| SubPath { anchors: sp.anchors.clone(), closed: sp.closed || (fill_closes && encloses_area(sp)) };
    let (si, geom, um) = path
        .subpaths
        .iter()
        .enumerate()
        .filter_map(|(i, sp)| {
            let g = ring(sp);
            let (u, d) = nearest_u(&g, mid)?;
            (d <= tol).then_some((i, g, u, d))
        })
        .min_by(|x, y| x.3.total_cmp(&y.3))
        .map(|(i, g, u, _)| (i, g, u))?;
    let len = geom.segment_count() as f64;
    let real = path.subpaths.get(si)?.segment_count() as f64;
    let at = |p: Point| nearest_u(&geom, p).filter(|(_, d)| *d <= tol).map(|(u, _)| u);
    // The parameter spans of `geom` that stay (unrolled past the end of a ring).
    let keep: Vec<(f64, f64)> = if whole {
        Vec::new()
    } else if geom.closed {
        let (ua, ub) = (at(a)?, at(b)?);
        let fwd = |from: f64, to: f64| (to - from).rem_euclid(len);
        // The piece runs from one end through `mid` to the other; what stays runs on round.
        let (start, end) = if fwd(ua, um) <= fwd(ua, ub) { (ub, ua) } else { (ua, ub) };
        let rest = fwd(start, end);
        if rest <= 1e-9 || len - rest <= 1e-9 {
            Vec::new()
        } else {
            // Where the ring closes with an invisible edge (`real..len`), what stays stops at it.
            let parts: &[(f64, f64)] = if real >= len { &[(0.0, 2.0 * len)] } else { &[(0.0, real), (len, len + real)] };
            parts.iter().map(|&(lo, hi)| (start.max(lo), (start + rest).min(hi))).filter(|(x, y)| y - x > 1e-9).collect()
        }
    } else {
        let (ua, ub) = (at(a)?, at(b)?);
        let (lo, hi) = (ua.min(ub), ua.max(ub));
        if !(lo - 1e-9..=hi + 1e-9).contains(&um) {
            return None;
        }
        [(0.0, lo), (hi, len)].into_iter().filter(|(x, y)| y - x > 1e-9).collect()
    };
    let mut subpaths: Vec<SubPath> = path.subpaths.iter().take(si).cloned().collect();
    subpaths.extend(keep.into_iter().filter_map(|(u0, u1)| span(&geom, u0, u1)).map(|sp| snap_ends(sp, [a, b], tol)));
    subpaths.extend(path.subpaths.iter().skip(si + 1).cloned());
    Some(PathData::new(subpaths))
}

/// The point of `sp` nearest `p`, as a parameter `segment + t`, with its distance.
fn nearest_u(sp: &SubPath, p: Point) -> Option<(f64, f64)> {
    use kurbo::ParamCurveNearest;
    (0..sp.segment_count())
        .map(|k| {
            let n = sp.segment(k).nearest(p, 1e-9);
            (k as f64 + n.t, n.distance_sq.sqrt())
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
}

/// `sp` with its end anchors within `tol` of one of `to` put exactly on it (where the planar map
/// put the junction a piece ends at), handles moving with them.
fn snap_ends(mut sp: SubPath, to: [Point; 2], tol: f64) -> SubPath {
    let last = sp.anchors.len().saturating_sub(1);
    for i in [0, last] {
        if let Some(an) = sp.anchors.get_mut(i)
            && let Some(q) = to.iter().find(|q| q.distance(an.p) <= tol)
        {
            let d = *q - an.p;
            an.p = *q;
            an.h_in += d;
            an.h_out += d;
        }
    }
    sp
}

/// The run of `sp` from parameter `u0` to `u1` (`u0 < u1`, unrolled past the end of a closed one),
/// open.
fn span(sp: &SubPath, u0: f64, u1: f64) -> Option<SubPath> {
    let n = sp.segment_count();
    if n == 0 || !(u0.is_finite() && u1.is_finite()) {
        return None;
    }
    let first = u0.floor().max(0.0) as usize;
    let last = (u1.ceil().max(0.0) as usize).min(first + 2 * n);
    let segs: Vec<Seg> = (first..last)
        .filter_map(|k| {
            let (t0, t1) = ((u0 - k as f64).clamp(0.0, 1.0), (u1 - k as f64).clamp(0.0, 1.0));
            (t1 - t0 > 1e-12).then(|| Seg { c: sp.segment(k % n).subsegment(t0..t1), line: sp.segment_is_line(k % n) })
        })
        .collect();
    segs_to_subpath(&segs, false)
}

fn build(shapes: &[Shape]) -> Option<(Vec<Region>, Vec<Shape>)> {
    let mut inputs: Vec<(BezPath, usize)> = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        let mut bp = BezPath::new();
        for sp in s.path.subpaths.iter().filter(|sp| sp.anchors.len() >= 2) {
            if sp.closed {
                sp.to_bezpath_into(&mut bp);
            } else {
                let mut open = BezPath::new();
                sp.to_bezpath_into(&mut open);
                there_and_back(&open, &mut bp);
            }
        }
        if !bp.elements().is_empty() {
            bp.apply_affine(SHEAR);
            inputs.push((bp, i));
        }
    }
    if inputs.is_empty() {
        return None;
    }
    let eps = eps_for(inputs.iter().map(|p| &p.0)).ok()?;
    snap_horizontals(&mut inputs.iter_mut().map(|p| &mut p.0).collect::<Vec<_>>(), eps);
    // The sweep can panic on rare near-degenerate input; that must never take the app down.
    let sweep =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<Crossings>::from_paths(inputs.iter().map(|(p, i)| (p, *i)), eps)));
    let top = sweep.ok()?.ok()?;
    let tidy = Tidy::keeping(DEFAULT_PRECISION, inputs.iter().map(|p| &p.0));

    // The planar graph: vertices are the sweep's points, edges its segments. Points closer than
    // the sweep's tolerance are one vertex (it can put a corner that lies on another path's edge
    // a rounding error away from where it cuts that edge).
    let positions = top.compute_positions();
    let cell = 4.0 * eps;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut points: Vec<Point> = Vec::new();
    let mut vertex_id = |p: Point| {
        let (cx, cy) = ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
        let near = (cx - 1..=cx + 1)
            .flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y)))
            .filter_map(|k| grid.get(&k))
            .flatten()
            .copied()
            .find_map(|v| points.get(v).filter(|q| q.distance(p) <= eps).map(|&q| (v, q)));
        near.unwrap_or_else(|| {
            points.push(p);
            grid.entry((cx, cy)).or_default().push(points.len() - 1);
            (points.len() - 1, p)
        })
    };
    let mut edges: Vec<Edge> = Vec::new();
    let mut between: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    let outlines = Outlines::new(&inputs, eps);
    for idx in top.segment_indices() {
        let half = idx.first_half();
        let mut segs: Vec<Seg> = positions[idx].path.segments().map(to_seg).collect();
        let ((v0, p0), (v1, p1)) = (vertex_id(top.point(half).to_kurbo()), vertex_id(top.point(idx.second_half()).to_kurbo()));
        let ends = [v0, v1];
        if segs.is_empty() || v0 == v1 {
            continue;
        }
        // Every edge at a vertex starts or ends exactly on it, so faces meeting there share its
        // coordinates: merging them (a union) then cancels their common edge, which it can't when
        // each face carries its own rounding of the corner (#893).
        pin_ends(&mut segs, p0, p1);
        let Some(source) = mid(&segs).and_then(|m| outlines.source_at(m)) else { continue };
        // Coincident pieces the sweep left apart are one edge (the front-most input's).
        let key = (ends[0].min(ends[1]), ends[0].max(ends[1]));
        let here = mid(&segs);
        let twin_of = between.get(&key).and_then(|v| {
            v.iter().copied().find(|&e| {
                let other = edges.get(e).and_then(|e| mid(&e.segs));
                matches!((here, other), (Some(a), Some(b)) if a.distance(b) < DEFAULT_PRECISION)
            })
        });
        if let Some(e) = twin_of {
            if let Some(e) = edges.get_mut(e)
                && source > e.source
            {
                e.source = source;
            }
            continue;
        }
        between.entry(key).or_default().push(edges.len());
        edges.push(Edge { segs, ends, source });
    }
    let nv = points.len();

    // Half-edges leaving each vertex, sorted by direction; walking a face, the next half-edge is
    // the one just clockwise of the way back, so bounded faces come out with positive area and
    // the outer boundary of each connected piece with negative area.
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); nv];
    for h in 0..2 * edges.len() {
        if let Some(list) = origin(&edges, h).and_then(|v| around.get_mut(v)) {
            list.push(h);
        }
    }
    let mut slot_of = vec![0usize; 2 * edges.len()];
    for (v, list) in around.iter_mut().enumerate() {
        let Some(&at) = points.get(v) else { continue };
        // Half the shortest chord of the edges here: every one of them gets that far from it.
        let r = 0.5 * list.iter().filter_map(|&h| half_segs(&edges, h).last().map(|s| s.c.p3.distance(at))).fold(f64::INFINITY, f64::min);
        let mut keyed: Vec<(f64, usize)> = list.iter().map(|&h| (leaving_angle(&edges, h, at, r), h)).collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        *list = keyed.into_iter().map(|(_, h)| h).collect();
        for (k, &h) in list.iter().enumerate() {
            if let Some(s) = slot_of.get_mut(h) {
                *s = k;
            }
        }
    }
    let next = |h: usize| -> Option<usize> {
        let back = twin(h);
        let list = around.get(origin(&edges, back)?)?;
        let k = *slot_of.get(back)?;
        list.get((k + list.len() - 1) % list.len()).copied()
    };

    // Connected pieces of the graph.
    let mut parent: Vec<usize> = (0..nv).collect();
    for e in &edges {
        let (a, b) = (find(&mut parent, e.ends[0]), find(&mut parent, e.ends[1]));
        if let Some(p) = parent.get_mut(a) {
            *p = b;
        }
    }
    let piece: Vec<usize> = (0..nv).map(|v| find(&mut parent, v)).collect();
    let piece_of = |cycle: &[usize]| cycle.first().and_then(|&h| origin(&edges, h)).and_then(|v| piece.get(v)).copied();

    // Walk every cycle: positive ones are faces, negative ones outer boundaries (holes in the
    // face around them, if any).
    let mut seen = vec![false; 2 * edges.len()];
    let mut faces: Vec<(Vec<usize>, BezPath, f64)> = Vec::new();
    let mut holes: Vec<(Vec<usize>, BezPath)> = Vec::new();
    for start in 0..2 * edges.len() {
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut cycle = vec![];
        let mut h = start;
        while let Some(s) = seen.get_mut(h).filter(|s| !**s) {
            *s = true;
            cycle.push(h);
            match next(h) {
                Some(n) => h = n,
                None => break,
            }
        }
        let cycle = without_spurs(&cycle);
        if cycle.is_empty() {
            continue;
        }
        let bp = cycle_bezpath(&edges, &cycle);
        let area = bp.area();
        if area > 0.0 && !is_sliver(&bp, tidy.precision) {
            faces.push((cycle, bp, area));
        } else if area < -(tidy.precision * tidy.precision) {
            holes.push((cycle, bp));
        }
    }

    // Each hole goes in the smallest face of another piece around it.
    let mut face_holes: Vec<Vec<usize>> = vec![Vec::new(); faces.len()];
    for (k, (cycle, _)) in holes.iter().enumerate() {
        let Some(p) = cycle.first().and_then(|&h| origin(&edges, h)).and_then(|v| points.get(v)).copied() else { continue };
        let own = piece_of(cycle);
        let around_it = faces
            .iter()
            .enumerate()
            .filter(|(_, (fc, bp, _))| piece_of(fc) != own && bp.bounding_box().contains(p) && bp.winding(p) != 0)
            .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
            .map(|(i, _)| i);
        if let Some(list) = around_it.and_then(|i| face_holes.get_mut(i)) {
            list.push(k);
        }
    }

    let fills: Vec<Fill> = shapes.iter().map(Fill::new).collect();
    // The inputs' anchors, which results land on again up to the sweep's tolerance.
    let snap = 2.0 * eps;
    let mut anchors: Vec<Point> = shapes.iter().flat_map(|s| s.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))).collect();
    anchors.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let subpath = |cycle: &[usize]| {
        let segs: Vec<Seg> = cycle.iter().flat_map(|&h| half_segs(&edges, h)).collect();
        segs_to_subpath(&tidy_segments(segs, true, &tidy), true)
    };
    let regions = faces
        .iter()
        .zip(&face_holes)
        .filter_map(|((cycle, _, _), hs)| {
            let mut subpaths = vec![subpath(cycle)?];
            subpaths.extend(hs.iter().filter_map(|&k| holes.get(k)).filter_map(|(c, _)| subpath(c)));
            let path = unshear(PathData::new(subpaths), &anchors, snap);
            let sources =
                interior_point(&path).map(|p| fills.iter().enumerate().filter(|(_, f)| f.contains(p)).map(|(i, _)| i).collect()).unwrap_or_default();
            Some(Region { path, sources })
        })
        .collect();

    // Edges: chains of segments through vertices where nothing else meets.
    let degree: Vec<usize> = around.iter().map(Vec::len).collect();
    let passes = |v: usize| -> Option<(usize, usize)> {
        let list = around.get(v)?;
        let (&a, &b) = (list.first()?, list.get(1)?);
        (degree.get(v) == Some(&2) && edges.get(a / 2)?.source == edges.get(b / 2)?.source).then_some((a, b))
    };
    let mut used = vec![false; edges.len()];
    let mut chains: Vec<(Vec<Seg>, bool, usize)> = Vec::new();
    // A chain from half-edge `h0` up to the next vertex where it doesn't pass straight through;
    // closed when it comes back round to where it started.
    let walk = |h0: usize, used: &mut Vec<bool>| -> Option<(Vec<Seg>, bool, usize)> {
        let source = edges.get(h0 / 2)?.source;
        let mut segs = Vec::new();
        let mut h = h0;
        loop {
            let u = used.get_mut(h / 2)?;
            if *u {
                return Some((segs, true, source));
            }
            *u = true;
            segs.extend(half_segs(&edges, h));
            let v = origin(&edges, twin(h))?;
            let Some((a, b)) = passes(v) else { return Some((segs, origin(&edges, h0) == Some(v), source)) };
            h = if a == twin(h) { b } else { a };
        }
    };
    for v in 0..nv {
        if passes(v).is_some() {
            continue;
        }
        for &h in around.get(v).map(Vec::as_slice).unwrap_or_default() {
            if !used.get(h / 2).copied().unwrap_or(true)
                && let Some(c) = walk(h, &mut used)
            {
                chains.push(c);
            }
        }
    }
    // What's left are closed loops with no junction on them.
    for e in 0..edges.len() {
        if !used.get(e).copied().unwrap_or(true)
            && let Some(c) = walk(2 * e, &mut used)
        {
            chains.push(c);
        }
    }
    let edge_shapes = chains
        .into_iter()
        .filter_map(|(segs, closed, source)| {
            let sp = segs_to_subpath(&tidy_segments(segs, closed, &tidy), closed)?;
            Some(Shape::new(unshear(PathData::new(vec![sp]), &anchors, snap), FillRule::NonZero, shapes.get(source)?.key))
        })
        .collect();
    Some((regions, edge_shapes))
}

/// A point strictly inside a filled path, well away from its edges where it can be (for re-finding
/// a Live Paint face after a rebuild).
pub fn interior_point(path: &PathData) -> Option<Point> {
    let bb = path.bounds()?;
    let bp = path.to_bezpath();
    let mut edges: Vec<(Point, Point)> = vec![];
    let (mut first, mut last) = (Point::ZERO, Point::ZERO);
    kurbo::flatten(bp.iter(), (bb.width().max(bb.height()) * 1e-3).max(1e-4), |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if last != first {
                edges.push((last, first));
            }
            first = p;
            last = p;
        }
        kurbo::PathEl::LineTo(p) => {
            edges.push((last, p));
            last = p;
        }
        kurbo::PathEl::ClosePath => {
            if last != first {
                edges.push((last, first));
            }
            last = first;
        }
        _ => {}
    });
    if last != first {
        edges.push((last, first));
    }
    let mut best: Option<(f64, Point)> = None;
    for f in [0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875, 0.0625, 0.9375] {
        // A little off the round fractions, which on art drawn to a grid fall on the height of
        // some corner, where testing the point against other paths is unreliable.
        let y = bb.y0 + bb.height() * (f + 0.0061803);
        let mut xs: Vec<f64> =
            edges.iter().filter(|(a, c)| (a.y <= y) != (c.y <= y)).map(|(a, c)| a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x)).collect();
        xs.sort_by(f64::total_cmp);
        for w in xs.windows(2) {
            let m = Point::new((w[0] + w[1]) / 2.0, y);
            let width = w[1] - w[0];
            if width > best.map_or(0.0, |b| b.0) && bp.winding(m) != 0 {
                best = Some((width, m));
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use vectorcraft_geom::{Rect, shapes};

    fn line(a: (f64, f64), b: (f64, f64), key: u64) -> Shape {
        Shape::new(PathData::single(SubPath::polyline(&[Point::new(a.0, a.1), Point::new(b.0, b.1)], false)), FillRule::NonZero, key)
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64, key: u64) -> Shape {
        Shape::new(shapes::rectangle(Rect::new(x0, y0, x1, y1)), FillRule::NonZero, key)
    }

    fn area(r: &Region) -> f64 {
        r.path.to_bezpath().area()
    }

    fn at(faces: &[Region], x: f64, y: f64) -> Vec<&Region> {
        faces.iter().filter(|f| f.contains(Point::new(x, y))).collect()
    }

    #[test]
    fn crossing_lines_enclose_a_face() {
        // Three lines crossing pairwise, their ends sticking out: one triangle.
        let s = [line((0.0, 0.0), (100.0, 100.0), 0), line((100.0, 0.0), (0.0, 100.0), 1), line((-10.0, 80.0), (110.0, 80.0), 2)];
        let (faces, edges) = live_paint(&s);
        assert_eq!(faces.len(), 1, "{faces:?}");
        let f = &faces[0];
        assert!((area(f) - 900.0).abs() < 1e-6, "triangle (20,80) (50,50) (80,80): {}", area(f));
        assert!(f.sources.is_empty(), "nothing fills it yet");
        assert_eq!(f.path.subpaths.len(), 1);
        assert_eq!(f.path.subpaths[0].anchors.len(), 3, "the loose ends are not part of it");
        // Each line is cut where the others cross it: 3 + 3 + 3 pieces.
        assert_eq!(edges.len(), 9);
        assert_eq!(edges.iter().filter(|e| e.key == 2).count(), 3);
    }

    #[test]
    fn a_grid_of_lines_makes_its_cells() {
        let mut s = vec![];
        for k in 0..4 {
            let v = f64::from(k) * 10.0;
            s.push(line((v, -5.0), (v, 35.0), 0));
            s.push(line((-5.0, v), (35.0, v), 1));
        }
        let (faces, _) = live_paint(&s);
        assert_eq!(faces.len(), 9);
        assert!(faces.iter().all(|f| (area(f) - 100.0).abs() < 1e-6));
        assert_eq!(at(&faces, 15.0, 15.0).len(), 1);
    }

    #[test]
    fn a_line_across_a_shape_splits_it() {
        let s = [rect(0.0, 0.0, 100.0, 50.0, 7), line((40.0, -20.0), (60.0, 70.0), 8)];
        let (faces, edges) = live_paint(&s);
        assert_eq!(faces.len(), 2);
        assert!(faces.iter().all(|f| f.sources == [0]), "both halves keep the rectangle's fill");
        assert!((faces.iter().map(area).sum::<f64>() - 5000.0).abs() < 1e-6);
        assert_eq!(at(&faces, 10.0, 25.0).len(), 1);
        assert_eq!(at(&faces, 90.0, 25.0).len(), 1);
        // The rectangle's outline in two pieces, the line in three.
        assert_eq!(edges.iter().filter(|e| e.key == 7).count(), 2);
        assert_eq!(edges.iter().filter(|e| e.key == 8).count(), 3);
    }

    #[test]
    fn overlapping_shapes_keep_their_coverage() {
        let s = [rect(0.0, 0.0, 100.0, 100.0, 0), rect(50.0, 0.0, 150.0, 100.0, 1)];
        let (faces, _) = live_paint(&s);
        assert_eq!(faces.len(), 3);
        assert_eq!(at(&faces, 25.0, 50.0)[0].sources, [0]);
        assert_eq!(at(&faces, 75.0, 50.0)[0].sources, [0, 1]);
        assert_eq!(at(&faces, 125.0, 50.0)[0].sources, [1]);
        // Same faces as the Pathfinder regions.
        let regions = crate::regions(&s);
        assert_eq!(regions.len(), 3);
        for f in &faces {
            assert!(regions.iter().any(|r| r.sources == f.sources && (area(r).abs() - area(f)).abs() < 1e-6));
        }
    }

    #[test]
    fn a_closed_loop_of_lines_inside_a_shape_is_a_hole_and_a_face() {
        let mut s = vec![rect(0.0, 0.0, 100.0, 100.0, 0)];
        let sq = [(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0), (40.0, 40.0)];
        for w in sq.windows(2) {
            s.push(line(w[0], w[1], 1));
        }
        // A loose line inside: no face of its own, no hole.
        s.push(line((10.0, 10.0), (20.0, 20.0), 2));
        let (faces, _) = live_paint(&s);
        assert_eq!(faces.len(), 2, "{faces:?}");
        let outer = at(&faces, 5.0, 5.0);
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].path.subpaths.len(), 2, "the inner square is a hole in it");
        assert!((area(outer[0]) - 9600.0).abs() < 1e-6);
        let inner = at(&faces, 50.0, 50.0);
        assert_eq!(inner.len(), 1, "the hole doesn't count as the outer face");
        assert_eq!(inner[0].sources, [0], "the rectangle fills it");
        assert!((area(inner[0]) - 400.0).abs() < 1e-6);
    }

    #[test]
    fn an_area_enclosed_by_shapes_is_an_unfilled_face() {
        // A frame of four bars around a 50 × 50 gap.
        let s = [rect(0.0, 0.0, 70.0, 10.0, 0), rect(0.0, 60.0, 70.0, 70.0, 0), rect(0.0, 0.0, 10.0, 70.0, 0), rect(60.0, 0.0, 70.0, 70.0, 0)];
        let (faces, _) = live_paint(&s);
        let gap = at(&faces, 35.0, 35.0);
        assert_eq!(gap.len(), 1);
        assert!(gap[0].sources.is_empty());
        assert!((area(gap[0]) - 2500.0).abs() < 1e-6);
    }

    #[test]
    fn curves_and_a_circle_crossed_by_a_line() {
        let circle = Shape::new(shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), FillRule::NonZero, 0);
        let (faces, edges) = live_paint(&[circle, line((-10.0, 50.0), (110.0, 50.0), 1)]);
        assert_eq!(faces.len(), 2);
        for f in &faces {
            assert!((area(f) - std::f64::consts::PI * 2500.0 / 2.0).abs() < 2.0, "{}", area(f));
            assert!(f.path.subpaths[0].anchors.len() <= 4, "no extra anchors from the sweep's splits");
        }
        assert_eq!(edges.iter().filter(|e| e.key == 0).count(), 2, "two arcs");
    }

    #[test]
    fn a_line_ending_on_an_edge_splits_the_shape() {
        let s = [line((90.0, 20.0), (70.0, 10.0), 0), rect(80.0, 10.0, 90.0, 50.0, 1)];
        let (faces, _) = live_paint(&s);
        assert_eq!(faces.len(), 2);
        assert!((area(&faces[0]) + area(&faces[1]) - 400.0).abs() < 1e-6);
        for f in &faces {
            let p = crate::interior_point(&f.path).unwrap();
            assert_eq!(at(&faces, p.x, p.y).len(), 1, "{p:?}");
            assert_eq!(f.sources, [1]);
        }
    }

    #[test]
    fn nothing_or_garbage_gives_an_empty_map() {
        assert_eq!(live_paint(&[]), (vec![], vec![]));
        assert!(live_paint(&[line((0.0, 0.0), (f64::NAN, 1.0), 0)]).0.is_empty());
        let (faces, edges) = live_paint(&[line((0.0, 0.0), (10.0, 10.0), 0)]);
        assert!(faces.is_empty());
        assert_eq!(edges.len(), 1);
    }

    /// A strip merged across an axonometric grid of lines (#893, the reporter's file): faces meeting
    /// at a vertex share its exact coordinates, so the merge is one outline with no edge left inside.
    #[test]
    fn faces_share_their_corners_exactly_so_merging_them_leaves_no_seam() {
        const LINES: &[[f64; 4]] = &[
            [416.57797027866155, 268.45362144397683, 416.5779702786616, 811.5463785560231],
            [243.12292300414873, 511.1134238473054, 713.4550472745127, 782.6598024033286],
            [836.8770769958512, 511.1134238473054, 366.5449527254871, 782.6598024033286],
            [457.718646852441, 268.45362144397683, 457.718646852441, 811.5463785560231],
            [263.6932612910385, 475.4845528055331, 734.0253855614023, 747.0309313615563],
            [816.3067387089613, 475.4845528055331, 345.9746144385975, 747.0309313615563],
            [498.85932342622044, 268.45362144397683, 498.85932342622044, 811.5463785560231],
            [284.26359957792823, 439.8556817637607, 754.5957238482922, 711.4020603197839],
            [795.7364004220717, 439.8556817637607, 325.40427615170773, 711.4020603197839],
            [539.9999999999998, 268.45362144397683, 539.9999999999998, 811.5463785560231],
            [304.83393786481787, 404.22681072198844, 775.1660621351818, 675.7731892780117],
            [775.1660621351821, 404.22681072198844, 304.833937864818, 675.7731892780117],
            [581.1406765737793, 268.45362144397683, 581.1406765737793, 811.5463785560231],
            [325.4042761517076, 368.59793968021614, 795.7364004220715, 640.1443182362394],
            [754.5957238482922, 368.59793968021614, 284.26359957792835, 640.1443182362394],
            [622.2813531475588, 268.45362144397683, 622.2813531475588, 811.5463785560231],
            [345.97461443859737, 332.9690686384438, 816.3067387089613, 604.515447194467],
            [734.0253855614026, 332.9690686384438, 263.6932612910385, 604.515447194467],
            [663.422029721338, 268.45362144397683, 663.422029721338, 811.5463785560231],
            [366.54495272548706, 297.3401975966715, 836.877076995851, 568.8865761526947],
            [713.4550472745128, 297.3401975966715, 243.12292300414884, 568.8865761526947],
        ];
        let s: Vec<Shape> = LINES.iter().enumerate().map(|(i, &[x0, y0, x1, y1])| line((x0, y0), (x1, y1), i as u64)).collect();
        let (faces, _) = live_paint(&s);
        let corners: Vec<Point> = faces.iter().flat_map(|f| f.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))).collect();
        for p in &corners {
            for q in &corners {
                assert!(p == q || p.distance(*q) > 1e-6, "one corner, two positions: {p:?} {q:?}");
            }
        }
        // Down the strip between the lines x ≈ 498.9 and x = 540: 11 triangles, one outline.
        let mut strip: Vec<&Region> = vec![];
        for k in 0..=240 {
            if let Some(f) = faces.iter().find(|f| f.contains(Point::new(519.43, 420.0 + f64::from(k))))
                && !strip.iter().any(|g| std::ptr::eq(*g, f))
            {
                strip.push(f);
            }
        }
        assert_eq!(strip.len(), 11);
        let merged = crate::merge_regions(&strip);
        assert_eq!(merged.subpaths.len(), 1, "{merged:?}");
    }

    #[test]
    fn shape_builder_lines_cut_regions() {
        // A line across a rectangle: two regions, the line in three pieces keyed by its index.
        let s = [rect(0.0, 0.0, 100.0, 50.0, 7), line((40.0, -20.0), (60.0, 70.0), 8)];
        let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
        assert_eq!(regions.len(), 2);
        assert!(regions.iter().all(|r| r.sources == [0]));
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.key == 1 && !l.path.is_closed()));
        // Lines alone enclose a region nothing fills.
        let s = [line((0.0, 0.0), (100.0, 100.0), 0), line((100.0, 0.0), (0.0, 100.0), 0), line((-10.0, 80.0), (110.0, 80.0), 0)];
        let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].sources.is_empty() && regions[0].top().is_none());
        assert_eq!(lines.len(), 9);
    }

    #[test]
    fn shape_builder_closed_only_or_degenerate_is_the_filled_arrangement() {
        let s = [rect(0.0, 0.0, 100.0, 100.0, 0), rect(50.0, 0.0, 150.0, 100.0, 1)];
        assert_eq!(shape_builder(&s, false), BuilderArrangement { regions: crate::regions(&s), ..Default::default() });
        // A broken line can't be swept: what the filled areas give (nothing here, without a panic).
        let s = [rect(0.0, 0.0, 100.0, 100.0, 0), line((0.0, 0.0), (f64::NAN, 1.0), 1)];
        assert_eq!(shape_builder(&s, false), BuilderArrangement { regions: crate::regions(&s), ..Default::default() });
        // Zero-length and doubled-back lines don't break it either.
        let s = [
            rect(0.0, 0.0, 100.0, 100.0, 0),
            line((50.0, 50.0), (50.0, 50.0), 1),
            line((0.0, 0.0), (100.0, 0.0), 2),
            line((50.0, 0.0), (50.0, 100.0), 3),
        ];
        let regions = shape_builder(&s, false).regions;
        assert_eq!(regions.len(), 2);
        assert!((regions.iter().map(area).sum::<f64>() - 10000.0).abs() < 1e-6);
    }

    #[test]
    fn shape_builder_stays_quick_up_to_its_cap() {
        // A wavy ring of nearly the most segments the map takes, crossed by a few lines.
        let n = SHAPE_BUILDER_MAX_SEGMENTS - 8;
        let ring: Vec<Point> = (0..n)
            .map(|k| {
                let t = k as f64 / n as f64 * std::f64::consts::TAU;
                let r = 100.0 + 5.0 * (t * 200.0).sin();
                Point::new(r * t.cos(), r * t.sin())
            })
            .collect();
        let mut s = vec![Shape::new(PathData::single(SubPath::polyline(&ring, true)), FillRule::NonZero, 0)];
        for k in 0..4 {
            let y = -60.0 + 40.0 * f64::from(k);
            s.push(line((-150.0, y), (150.0, y + 7.0), 1));
        }
        let started = std::time::Instant::now();
        let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
        assert!(regions.len() >= 5, "the ring cut in five bands at least (the waves make pockets)");
        assert!(lines.len() >= 12, "{}", lines.len());
        assert!(started.elapsed().as_secs_f64() < 5.0, "{:?}", started.elapsed());
        // Past the cap, only the filled areas count.
        s.push(line((0.0, 0.0), (1.0, 1.0), 2));
        s[0].path.subpaths[0].anchors.extend((0..16).map(|k| vectorcraft_geom::Anchor::corner(Point::new(100.0, f64::from(k)))));
        assert!(shape_builder(&s, false).lines.is_empty());
    }

    fn poly(pts: &[(f64, f64)], closed: bool) -> SubPath {
        SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed)
    }

    fn ends(sp: &SubPath) -> (Point, Point) {
        (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p)
    }

    #[test]
    fn shape_builder_edges_are_the_outlines_cut_where_paths_meet() {
        // Two overlapping squares: each outline in two pieces, cut where the other crosses it.
        let s = [rect(0.0, 0.0, 100.0, 100.0, 0), rect(50.0, 50.0, 150.0, 150.0, 1)];
        let with = shape_builder(&s, true);
        assert_eq!(with.regions, shape_builder(&s, false).regions, "the regions don't change");
        assert!(with.lines.is_empty() && shape_builder(&s, false).edges.is_empty());
        assert_eq!(with.edges.len(), 4);
        for key in [0, 1] {
            assert_eq!(with.edges.iter().filter(|e| e.key == key && !e.path.is_closed()).count(), 2);
        }
        let inner = with.edges.iter().find(|e| e.key == 0 && e.path.bounds() == Some(Rect::new(50.0, 50.0, 100.0, 100.0))).expect("A inside B");
        // Erasing it opens A there.
        let cut = cut_out(&s[0].path, &inner.path.subpaths[0], false, 1e-3).expect("on A");
        assert_eq!(cut.subpaths.len(), 1);
        let sp = &cut.subpaths[0];
        assert!(!sp.closed && sp.anchors.len() == 5, "{sp:?}");
        let (a, b) = ends(sp);
        assert!([a, b].contains(&Point::new(100.0, 50.0)) && [a, b].contains(&Point::new(50.0, 100.0)), "{a:?} {b:?}");
        assert!((cut.length() - 300.0).abs() < 1e-6, "{}", cut.length());
        // Not on B.
        assert_eq!(cut_out(&s[1].path, &inner.path.subpaths[0], false, 1e-3), None);
        // A lone square's edge is its whole outline: erasing it leaves nothing.
        let lone = shape_builder(&s[..1], true);
        assert_eq!(lone.edges.len(), 1);
        assert!(cut_out(&s[0].path, &lone.edges[0].path.subpaths[0], false, 1e-3).unwrap().is_empty());
    }

    #[test]
    fn cut_out_splits_open_paths_and_keeps_closing_edges_invisible() {
        let open = PathData::single(poly(&[(0.0, 0.0), (100.0, 0.0)], false));
        let cut = cut_out(&open, &poly(&[(60.0, 0.0), (30.0, 0.0)], false), false, 1e-3).unwrap();
        let mut parts: Vec<(Point, Point)> = cut.subpaths.iter().map(ends).collect();
        parts.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        assert_eq!(parts, vec![(Point::new(0.0, 0.0), Point::new(30.0, 0.0)), (Point::new(60.0, 0.0), Point::new(100.0, 0.0))]);
        // An end piece only shortens it.
        let cut = cut_out(&open, &poly(&[(70.0, 0.0), (100.0, 0.0)], false), false, 1e-3).unwrap();
        assert_eq!(cut.subpaths.iter().map(ends).collect::<Vec<_>>(), vec![(Point::new(0.0, 0.0), Point::new(70.0, 0.0))]);

        // A filled open path runs on round its invisible closing edge (100,100) → (0,0).
        let corner = PathData::single(poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false));
        let piece = poly(&[(100.0, 50.0), (100.0, 100.0), (50.0, 50.0)], false);
        assert_eq!(cut_out(&corner, &piece, false, 1e-3), None, "without the fill it isn't on the path");
        let cut = cut_out(&corner, &piece, true, 1e-3).unwrap();
        assert_eq!(cut.subpaths.len(), 1);
        assert_eq!(cut.subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>(), [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)].map(Point::from));
        // Cut elsewhere, the closing edge stays invisible: two open runs, not one through it.
        let cut = cut_out(&corner, &poly(&[(20.0, 0.0), (60.0, 0.0)], false), true, 1e-3).unwrap();
        assert_eq!(cut.subpaths.len(), 2);
        assert!(cut.subpaths.iter().all(|sp| !sp.closed));
        assert!((cut.length() - 160.0).abs() < 1e-6, "{}", cut.length());
    }

    fn arb_segment() -> impl Strategy<Value = Shape> {
        let c = || (0..10i32).prop_map(|v| f64::from(v) * 10.0);
        prop_oneof![
            (c(), c(), c(), c()).prop_map(|(a, b, x, y)| line((a, b), (x, y), 0)),
            (c(), c(), 1..5i32, 1..5i32).prop_map(|(x, y, w, h)| rect(x, y, x + f64::from(w) * 10.0, y + f64::from(h) * 10.0, 0)),
            (c(), c(), 1..5i32).prop_map(|(x, y, r)| {
                let r = f64::from(r) * 10.0;
                Shape::new(shapes::ellipse(Rect::new(x, y, x + r, y + r)), FillRule::EvenOdd, 0)
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        /// Faces never overlap, and a face's sources are exactly the inputs whose fill holds a
        /// point inside it.
        #[test]
        fn faces_are_disjoint_and_know_what_covers_them(shapes in prop::collection::vec(arb_segment(), 1..7)) {
            let (faces, _) = live_paint(&shapes);
            for f in &faces {
                prop_assert!(area(f) > 0.0);
                let Some(p) = crate::interior_point(&f.path) else { continue };
                prop_assert_eq!(faces.iter().filter(|g| g.contains(p)).count(), 1, "{:?} in more than one face", p);
                let covering: Vec<usize> = shapes
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.path.is_closed() && vectorcraft_geom::hit::fill_contains(&s.path.to_bezpath(), s.rule, p))
                    .map(|(i, _)| i)
                    .collect();
                prop_assert_eq!(&f.sources, &covering, "at {:?}", p);
            }
        }

        /// Closed shapes alone: the filled faces cover exactly the Pathfinder regions.
        #[test]
        fn filled_faces_are_the_pathfinder_regions(shapes in prop::collection::vec(arb_segment(), 1..6)) {
            let closed: Vec<Shape> = shapes.into_iter().filter(|s| s.path.is_closed()).collect();
            let (faces, _) = live_paint(&closed);
            let filled: f64 = faces.iter().filter(|f| !f.sources.is_empty()).map(area).sum();
            let regions: f64 = crate::regions(&closed).iter().map(|r| area(r).abs()).sum();
            // Both refit split curves to [`DEFAULT_PRECISION`], so curved areas may differ by up
            // to that much along every outline.
            let outlines: f64 = closed.iter().map(|s| s.path.to_bezpath().perimeter(1e-6)).sum();
            prop_assert!((filled - regions).abs() <= 1e-9 + DEFAULT_PRECISION * outlines, "{} vs {}", filled, regions);
        }

        /// Every edge and line piece cuts out of the input it is keyed by, which loses its length
        /// and nothing else.
        #[test]
        fn pieces_cut_out_of_their_inputs(shapes in prop::collection::vec(arb_segment(), 1..6)) {
            let arr = shape_builder(&shapes, true);
            for e in arr.edges.iter().chain(&arr.lines) {
                let (Some(s), Some(piece)) = (shapes.get(e.key as usize), e.path.subpaths.first()) else { continue };
                let len = e.path.length();
                let cut = cut_out(&s.path, piece, false, 0.05);
                if len <= 0.05 {
                    // A speck where two paths touch.
                    prop_assert!(cut.is_none());
                    continue;
                }
                prop_assert!(cut.is_some(), "{:?} not on {:?}", piece, s.path);
                let lost = s.path.length() - cut.map_or(0.0, |c| c.length());
                prop_assert!((lost - len).abs() <= 0.05 + 1e-3 * len, "lost {} for a piece {} long", lost, len);
            }
        }

        /// Lines anywhere (not on a grid): faces stay disjoint, and nothing panics.
        #[test]
        fn free_lines_make_disjoint_faces(pts in prop::collection::vec((-50.0..150.0f64, -50.0..150.0f64, -50.0..150.0f64, -50.0..150.0f64), 2..9)) {
            let lines: Vec<Shape> = pts.into_iter().map(|(a, b, x, y)| line((a, b), (x, y), 0)).collect();
            let (faces, edges) = live_paint(&lines);
            prop_assert!(edges.len() >= lines.len().min(1));
            for f in &faces {
                prop_assert!(f.sources.is_empty());
                if let Some(p) = crate::interior_point(&f.path) {
                    prop_assert_eq!(faces.iter().filter(|g| g.contains(p)).count(), 1);
                }
            }
        }
    }

    #[test]
    fn many_lines_stay_quick() {
        // 60 lines through a 100 pt square: about 900 crossings.
        let lines: Vec<Shape> = (0..60)
            .map(|k| {
                let t = f64::from(k) * 0.37;
                line((50.0 + 80.0 * t.cos(), 50.0 + 80.0 * t.sin()), (50.0 - 80.0 * (t + 1.1).cos(), 50.0 - 80.0 * (t + 1.1).sin()), 0)
            })
            .collect();
        let started = std::time::Instant::now();
        let (faces, _) = live_paint(&lines);
        assert!(faces.len() > 500, "{}", faces.len());
        assert!(started.elapsed().as_secs_f64() < 5.0, "{:?}", started.elapsed());
    }
}
