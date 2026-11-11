use super::{FillRule, MAX_SOLID_FACES, ParamCurve, PathData, Point, Rect};

/// Outline points Extrude & Bevel and Inflate sample at most, and contours they take at most.
pub(super) const MAX_SOLID_POINTS: usize = 65536;

/// Outline points Rotate samples at most: it only turns the outline, so an outlined paragraph fits.
pub(super) const MAX_PLANE_POINTS: usize = 65536;

/// Pieces a twisted wall's strip is cut into along one edge at most.
pub(super) const MAX_PIECES: usize = 64;

/// Pieces a curve takes at most, where its tolerance asks for them.
pub(super) const CURVE_PIECES: u32 = 32;

/// The fewest pieces a curve of a solid over its budget is held to: a curve whose tolerance asks
/// for fewer takes fewer.
pub(super) const MIN_CURVE_PIECES: u32 = 4;

const TOO_COMPLEX_SHAPE: &str = "The shape is too complex for a 3D effect";
pub(super) const TOO_COMPLEX_SURFACE: &str = "The 3D surface is too complex; simplify the shape";

/// How many straight pieces follow `c` within `tolerance` (Wang's bound on a cubic's distance from
/// its chords), 1 to [`CURVE_PIECES`].
fn pieces(c: vectorcraft_geom::CubicBez, tolerance: f64) -> u32 {
    let d = |a: Point, b: Point, c: Point| (a.to_vec2() - 2.0 * b.to_vec2() + c.to_vec2()).hypot();
    let bend = d(c.p0, c.p1, c.p2).max(d(c.p1, c.p2, c.p3));
    let n = (0.75 * bend / tolerance).sqrt().ceil();
    if n.is_finite() { (n as u32).clamp(1, CURVE_PIECES) } else { CURVE_PIECES }
}

/// The most pieces each curve of a solid's outline takes, counted on `path` before any point is
/// sampled: [`CURVE_PIECES`] when the outline fits within [`MAX_SOLID_POINTS`] points and
/// [`MAX_SOLID_FACES`] faces, else the largest number down to [`MIN_CURVE_PIECES`] that fits, the
/// same for every curve. A curve whose tolerance (see [`sample`]) asks for fewer pieces takes
/// fewer. The faces are counted as `per_edge` for each edge of the outline and `fixed` more.
///
/// An error when the outline takes more than [`MAX_SOLID_POINTS`] points, or contours, with
/// every curve held to [`MIN_CURVE_PIECES`], and when its faces do not fit with that many.
pub(super) fn curve_pieces(path: &PathData, size: f64, per_edge: usize, fixed: usize) -> Result<u32, String> {
    let tolerance = (size / 2000.0).max(0.05);
    // How many curves ask for each number of pieces.
    let mut curves = [0usize; CURVE_PIECES as usize + 1];
    let (mut lines, mut segments, mut open) = (0usize, 0usize, 0usize);
    for (contours, sp) in path.subpaths.iter().enumerate() {
        if contours >= MAX_SOLID_POINTS {
            return Err(TOO_COMPLEX_SHAPE.into());
        }
        // A subpath of one anchor draws nothing, as in [`sample`].
        if sp.anchors.len() < 2 {
            continue;
        }
        if sp.anchors.iter().any(|a| [a.p, a.h_in, a.h_out].iter().any(|q| !q.x.is_finite() || !q.y.is_finite())) {
            return Err("The shape contains invalid coordinates".into());
        }
        let count = if sp.closed { sp.anchors.len() } else { sp.anchors.len() - 1 };
        // Every segment takes a point at least.
        segments = segments.saturating_add(count);
        if segments > MAX_SOLID_POINTS {
            return Err(TOO_COMPLEX_SHAPE.into());
        }
        open += usize::from(!sp.closed);
        for i in 0..count {
            if sp.segment_is_line(i) {
                lines += 1;
            } else if let Some(n) = curves.get_mut(pieces(sp.segment(i), tolerance) as usize) {
                *n += 1;
            }
        }
    }
    let points = |most: u32| -> usize {
        curves.iter().enumerate().fold(lines, |sum, (asked, n)| sum.saturating_add(n.saturating_mul(asked.min(most as usize))))
    };
    if points(MIN_CURVE_PIECES) > MAX_SOLID_POINTS {
        return Err(TOO_COMPLEX_SHAPE.into());
    }
    // An open contour is closed by one more edge.
    let faces = |most: u32| points(most).saturating_add(open).saturating_mul(per_edge).saturating_add(fixed);
    (MIN_CURVE_PIECES..=CURVE_PIECES)
        .rev()
        .find(|most| points(*most) <= MAX_SOLID_POINTS && faces(*most) <= MAX_SOLID_FACES)
        .ok_or_else(|| TOO_COMPLEX_SURFACE.into())
}

/// Contours thinner than this on average (twice their area over their perimeter) are dropped when
/// overlaps are resolved: the debris a boolean leaves where two curves coincide, far below any
/// art. Pathfinder's default precision would drop real hairlines too.
pub(super) const DEBRIS: f64 = 1e-6;

/// The rings of a solid's profile, as [`rings`], with contours that meet merged first. Type often
/// draws one glyph as overlapping contours (a "k"'s arm across its stem), and [`rings`] tells a
/// contour's side from one point of it, so a contour that meets another, or itself, must be cut
/// where they meet. Most art has no such contours, and the boolean costs more than the rest of the
/// solid, so it runs only when [`edges_meet`] finds some. Its answer already runs holes against
/// their outlines, so it is only turned as a whole ([`wound`]), never judged a ring at a time.
///
/// The `limit` is checked on the profile as drawn, before the boolean, and again on the merged
/// outline, which can have more points (bars crossing in a grid trace every hole between them):
/// past it the shape is too complex, since its contours as drawn cross and can't be built as they
/// are. A profile the boolean refuses is built from its contours as drawn, as it was before
/// contours were merged. An empty answer is an answer: contours that cancel (a square drawn twice
/// under even-odd), or that are all thinner than [`DEBRIS`], fill nothing, so nothing is built.
/// Each curve takes at most `most` pieces, on the profile as drawn and on the merged outline.
pub(super) fn solid_rings(path: &PathData, rule: FillRule, size: f64, limit: usize, most: u32) -> Result<Vec<Vec<Point>>, String> {
    Ok(solid_rings_traced(path, rule, size, limit, most)?.into_iter().map(|r| r.points).collect())
}

/// Do the `sampled` rings of `closed` (the profile as the boolean takes it) meet, so that it must
/// go through the boolean? Within twice the boolean's tolerance ([`touch`]): pathops first snaps
/// anchors onto nearby heights (by less than the tolerance), then the sweep joins edges within it
/// along the sweep line, so straight edges a little more than it apart can come out as one: at
/// most sin θ + cos θ ≤ √2 times it, θ the edges' angle to the sweep line (about 1.4 at 45°,
/// measured; the same when pathops turns the sweep a quarter turn, which it does for some art of
/// 2,048 segments or more). Straight contours it might join are merged; those it leaves apart cost
/// a boolean that builds the same shape. Curves are judged on their sampled outline (see
/// [`sample`]), which can lie a little inside them (up to the sampling tolerance, or more for a
/// long curve held to 32 pieces), so curves that touch or overlap by less than that can be judged
/// apart and built as drawn.
pub(super) fn meets(sampled: &[Vec<Point>], closed: &PathData) -> bool {
    edges_meet(sampled, 2.0 * touch(closed))
}

/// `path` as the boolean should take it: each open subpath closed by a straight line, as [`sample`]
/// and the renderer's fill close it (the boolean would close it along the end anchors' handles).
/// Borrowed when nothing needs changing: every subpath closed, a single anchor (which the boolean
/// skips), or open with no handle on the closing side of either end anchor (which the boolean
/// already closes straight).
pub(super) fn closed_straight(path: &PathData) -> std::borrow::Cow<'_, PathData> {
    // The closing segment (last anchor to first) as the boolean will draw it once closed; the
    // length test comes first, so a subpath of fewer than two anchors never reaches the index.
    let straight = |sp: &vectorcraft_geom::SubPath| sp.closed || sp.anchors.len() < 2 || sp.segment_is_line(sp.anchors.len() - 1);
    if path.subpaths.iter().all(straight) {
        return std::borrow::Cow::Borrowed(path);
    }
    let mut path = path.clone();
    for sp in path.subpaths.iter_mut().filter(|sp| !sp.closed) {
        if let Some(last) = sp.anchors.last_mut() {
            last.h_out = last.p;
        }
        if let Some(first) = sp.anchors.first_mut() {
            first.h_in = first.p;
        }
        sp.closed = true;
    }
    std::borrow::Cow::Owned(path)
}

/// The boolean's rings turned so the filled side is on each one's left. Its holes already run
/// against their outlines, so only the direction of the whole is chosen: the one whose rings
/// enclose positive area in all.
pub(super) fn wound<R: Turnable>(mut rings: Vec<R>) -> Vec<R> {
    rings.retain(|r| signed_area(r.points()).abs() > 1e-9);
    if rings.iter().map(|r| signed_area(r.points())).sum::<f64>() < 0.0 {
        rings.iter_mut().for_each(Turnable::turn_around);
    }
    rings
}

/// The boolean's sweep tolerance for `path` as it is given to it (closed by [`closed_straight`]):
/// 64 times the rounding of the largest coordinate it reads, every anchor and handle of each
/// subpath of two anchors or more, and at least 1e-6 (pathops' `eps_for`). For a path [`sample`]
/// accepts (every point finite).
pub(super) fn touch(path: &PathData) -> f64 {
    let m = path
        .subpaths
        .iter()
        .filter(|sp| sp.anchors.len() >= 2)
        .flat_map(|sp| &sp.anchors)
        .flat_map(|a| [a.p, a.h_in, a.h_out])
        .fold(0.0f64, |m, q| m.max(q.x.abs()).max(q.y.abs()));
    (m * f64::EPSILON * 64.0).max(1e-6)
}

/// Most pairs of edges [`edges_meet`] compares. Past it the answer is yes, and the boolean, with
/// its own budget, decides.
const MAX_EDGE_PAIRS: usize = 1 << 22;

/// Do any two edges of `rings` meet, within `touch` (at that distance or closer): a contour crossing, touching or running
/// along another, or itself? Neighbouring edges of a ring share a corner and meet only where one
/// folds back along the other. Edges are swept in order of their left ends, so only pairs whose
/// bounds overlap across x are compared.
pub(super) fn edges_meet(rings: &[Vec<Point>], touch: f64) -> bool {
    struct Edge {
        ring: usize,
        index: usize,
        a: Point,
        b: Point,
        x0: f64,
        x1: f64,
    }
    let mut edges: Vec<Edge> = rings
        .iter()
        .enumerate()
        .flat_map(|(ring, r)| {
            r.iter().zip(r.iter().cycle().skip(1)).enumerate().map(move |(index, (a, b))| Edge {
                ring,
                index,
                a: *a,
                b: *b,
                x0: a.x.min(b.x),
                x1: a.x.max(b.x),
            })
        })
        .collect();
    edges.sort_by(|e, f| e.x0.total_cmp(&f.x0));
    let len = |ring: usize| rings.get(ring).map_or(0, Vec::len);
    let mut pairs = 0usize;
    for (i, e) in edges.iter().enumerate() {
        for f in edges.iter().skip(i + 1) {
            if f.x0 > e.x1 + touch {
                break;
            }
            pairs += 1;
            if pairs > MAX_EDGE_PAIRS {
                return true;
            }
            if e.a.y.min(e.b.y) > f.a.y.max(f.b.y) + touch || f.a.y.min(f.b.y) > e.a.y.max(e.b.y) + touch {
                continue;
            }
            let n = len(e.ring);
            let neighbours = e.ring == f.ring && ((e.index + 1) % n == f.index || (f.index + 1) % n == e.index);
            if neighbours {
                // They share a corner; they meet only if one runs back along the other.
                let (shared, p, q) = if e.b == f.a { (e.b, e.a, f.b) } else { (e.a, e.b, f.a) };
                if super::build::segment_distance(q, shared, p) <= touch || super::build::segment_distance(p, shared, q) <= touch {
                    return true;
                }
            } else if segments_meet(e.a, e.b, f.a, f.b, touch) {
                return true;
            }
        }
    }
    false
}

/// Do segments `a`–`b` and `c`–`d` cross, or come within `touch` of each other?
fn segments_meet(a: Point, b: Point, c: Point, d: Point, touch: f64) -> bool {
    let side = |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let (d1, d2, d3, d4) = (side(c, d, a), side(c, d, b), side(a, b, c), side(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    use super::build::segment_distance as dist;
    dist(a, c, d).min(dist(b, c, d)).min(dist(c, a, b)).min(dist(d, a, b)) <= touch
}

/// Where one edge of a sampled ring lies on the curve it follows: the piece of `curve` from `t0`
/// to `t1` (`t1` below `t0` on a ring that runs against its curve).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Span {
    pub(super) curve: vectorcraft_geom::CubicBez,
    pub(super) t0: f64,
    pub(super) t1: f64,
}

/// A sampled ring with, for each edge (point `i` to point `i + 1`, the last back to the first),
/// the piece of curve it follows, or `None` for a straight edge.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ring {
    pub(super) points: Vec<Point>,
    pub(super) spans: Vec<Option<Span>>,
}

/// A ring that [`orient`] and [`wound`] can turn around.
pub(super) trait Turnable {
    fn points(&self) -> &[Point];
    fn turn_around(&mut self);
}

impl Turnable for Vec<Point> {
    fn points(&self) -> &[Point] {
        self
    }
    fn turn_around(&mut self) {
        self.reverse();
    }
}

impl Turnable for Ring {
    fn points(&self) -> &[Point] {
        &self.points
    }
    fn turn_around(&mut self) {
        // The ring keeps its first point, an anchor of the art, so that a run of edges along one
        // curve doesn't start part way along it. Edge `j` of the turned ring is edge `n - 1 - j`
        // of the ring, run the other way.
        self.points.reverse();
        if !self.points.is_empty() {
            self.points.rotate_right(1);
        }
        self.spans.reverse();
        for s in self.spans.iter_mut().flatten() {
            std::mem::swap(&mut s.t0, &mut s.t1);
        }
    }
}

/// [`solid_rings`] with the piece of curve each edge follows.
pub(super) fn solid_rings_traced(path: &PathData, rule: FillRule, size: f64, limit: usize, most: u32) -> Result<Vec<Ring>, String> {
    let sampled = sample_traced(path, size, limit, most)?;
    let closed = closed_straight(path);
    let points: Vec<Vec<Point>> = sampled.iter().map(|r| r.points.clone()).collect();
    if !meets(&points, &closed) {
        return orient(sampled, rule);
    }
    match vectorcraft_pathops::try_unite_all_with(&[(&closed, rule)], DEBRIS) {
        Ok(merged) => sample_traced(&merged, size, limit, most).map(wound),
        Err(_) => orient(sampled, rule),
    }
}

/// The rings of a shape: closed polygons with the filled side on their left, (-dy, dx): outer
/// outlines run one way and holes the other, so nonzero winding fills them as the art is. Curves
/// are followed within a tolerance of 1/2000 of `size` (at least 0.05 pt), so small curves take
/// few points and large ones more; more than `limit` points in all is refused.
///
/// Under the nonzero `rule` an outline with filled art on both sides is no edge of the shape and
/// is left out: a contour inside another that runs the same way doesn't cut a hole, as it would
/// if nesting alone decided (#1724).
pub(super) fn rings(path: &PathData, rule: FillRule, size: f64, limit: usize) -> Result<Vec<Vec<Point>>, String> {
    orient(sample(path, size, limit, CURVE_PIECES)?, rule)
}

/// The closed polygons that follow `path`'s subpaths, as [`rings`] samples them, each running as
/// its subpath does, each curve in at most `most` pieces.
pub(super) fn sample(path: &PathData, size: f64, limit: usize, most: u32) -> Result<Vec<Vec<Point>>, String> {
    Ok(sample_traced(path, size, limit, most)?.into_iter().map(|r| r.points).collect())
}

/// [`sample`] with the piece of curve each edge follows.
fn sample_traced(path: &PathData, size: f64, limit: usize, most: u32) -> Result<Vec<Ring>, String> {
    let tolerance = (size / 2000.0).max(0.05);
    let mut count = 0;
    let mut out = vec![];
    // A subpath of one anchor draws nothing, closed or not (the renderer and the boolean skip it).
    for sp in path.subpaths.iter().filter(|sp| sp.anchors.len() >= 2) {
        let segments = if sp.closed { sp.anchors.len() } else { sp.anchors.len() - 1 };
        if segments > limit {
            return Err(TOO_COMPLEX_SHAPE.into());
        }
        // Each point with the piece of curve of the edge that ends at it.
        let mut ring: Vec<(Point, Option<Span>)> = vec![];
        if let Some(first) = sp.anchors.first() {
            ring.push((first.p, None));
        }
        for i in 0..segments {
            let curve = sp.segment(i);
            let line = sp.segment_is_line(i);
            let samples = if line { 1 } else { pieces(curve, tolerance).min(most.max(1)) };
            count += samples as usize;
            if count > limit {
                return Err(TOO_COMPLEX_SHAPE.into());
            }
            for k in 1..=samples {
                let (t0, t1) = (f64::from(k - 1) / f64::from(samples), f64::from(k) / f64::from(samples));
                ring.push((curve.eval(t1), (!line).then_some(Span { curve, t0, t1 })));
            }
        }
        if ring.iter().any(|(p, _)| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e8 || p.y.abs() > 1e8) {
            return Err("The shape contains invalid coordinates".into());
        }
        ring.dedup_by(|a, b| a.0.distance(b.0) < 1e-9);
        // The last point back on the first closes the ring along the edge that ends at it.
        while ring.len() > 1 && ring.first().zip(ring.last()).is_some_and(|(a, b)| a.0.distance(b.0) < 1e-9) {
            let closing = ring.pop().and_then(|(_, s)| s);
            if let Some(first) = ring.first_mut() {
                first.1 = closing;
            }
        }
        let points: Vec<Point> = ring.iter().map(|(p, _)| *p).collect();
        // Rings with no area are kept here: a contour that crosses itself into lobes that cancel
        // (a bowtie) has none, and [`edges_meet`] must see it. [`orient`] leaves them out. A flat
        // ring, every point on one line, fills nothing under either rule and is left out now, so
        // that it doesn't send the profile through the boolean.
        if points.len() >= 3 && !flat(&points) {
            let n = points.len();
            let spans = (0..n).map(|i| ring.get((i + 1) % n).and_then(|(_, s)| *s)).collect();
            out.push(Ring { points, spans });
        }
    }
    Ok(out)
}

/// `out` (sampled rings) turned so the filled side is on each one's left, and under the nonzero
/// `rule` without the rings that are no edge of the shape, as [`rings`] describes.
fn orient<R: Turnable>(mut out: Vec<R>, rule: FillRule) -> Result<Vec<R>, String> {
    out.retain(|r| signed_area(r.points()).abs() > 1e-9);
    // Which rings hold which: each ring against the others, its edges walked only where its
    // bounds hold the point, and within [`MAX_CONTAINMENT_EDGES`] edges and pairs in all (a pair
    // whose bounds miss the point costs a step too: many small rings are a square of pairs).
    let too_complex = || String::from(TOO_COMPLEX_SHAPE);
    let mut work = 0usize;
    let mut within = |out: &[R], bounds: &[Rect], i: usize| -> Result<Vec<usize>, String> {
        let p = out.get(i).and_then(|r| r.points().first()).copied().unwrap_or_default();
        let mut held_by = vec![];
        for (j, (o, b)) in out.iter().zip(bounds).enumerate() {
            work = work.saturating_add(1);
            if j == i || !(b.x0 <= p.x && p.x <= b.x1 && b.y0 <= p.y && p.y <= b.y1) {
                if work > MAX_CONTAINMENT_EDGES {
                    return Err(too_complex());
                }
                continue;
            }
            work = work.saturating_add(o.points().len());
            if work > MAX_CONTAINMENT_EDGES {
                return Err(too_complex());
            }
            if contains(o.points(), p) {
                held_by.push(j);
            }
        }
        Ok(held_by)
    };
    let bounds_of = |out: &[R]| -> Vec<Rect> {
        out.iter().map(|r| r.points().iter().map(|q| Rect::from_points(*q, *q)).reduce(|a, b| a.union(b)).unwrap_or_default()).collect()
    };
    if rule == FillRule::NonZero {
        let turn = |r: &[Point]| if signed_area(r) > 0.0 { 1 } else { -1 };
        let bounds = bounds_of(&out);
        let mut edge = Vec::with_capacity(out.len());
        for (i, r) in out.iter().enumerate() {
            // The winding number just outside the ring, and just inside it.
            let outside: i32 = within(&out, &bounds, i)?.into_iter().filter_map(|j| out.get(j)).map(|o| turn(o.points())).sum();
            edge.push((outside != 0) != (outside + turn(r.points()) != 0));
        }
        let mut edge = edge.into_iter();
        out.retain(|_| edge.next().unwrap_or(true));
    }
    // A ring inside an odd number of others is a hole.
    let bounds = bounds_of(&out);
    let holes: Vec<bool> = (0..out.len()).map(|i| within(&out, &bounds, i).map(|held| held.len() % 2 == 1)).collect::<Result<_, _>>()?;
    for (r, hole) in out.iter_mut().zip(holes) {
        if (signed_area(r.points()) > 0.0) == hole {
            r.turn_around();
        }
    }
    Ok(out)
}

/// Most ring edges [`rings`] walks to tell which rings hold which: every ring is tested against
/// the others, so many rings of many points each would otherwise take billions of steps.
const MAX_CONTAINMENT_EDGES: usize = 50_000_000;

/// Does every point of `ring` lie on one line (within 1e-9, as [`sample`] merges points)?
fn flat(ring: &[Point]) -> bool {
    let Some(&a) = ring.first() else { return true };
    let Some(&b) = ring.iter().max_by(|p, q| a.distance(**p).total_cmp(&a.distance(**q))) else { return true };
    let len = a.distance(b);
    len <= 1e-9 || ring.iter().all(|p| ((b - a).cross(*p - a) / len).abs() <= 1e-9)
}

pub(super) fn signed_area(r: &[Point]) -> f64 {
    r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

pub(super) fn contains(r: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (a, b) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// The unit normal on the left of `a`→`b`: towards the filled side.
pub(super) fn left(a: Point, b: Point) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l = dx.hypot(dy);
    if l < 1e-12 { (0.0, 0.0) } else { (-dy / l, dx / l) }
}

/// `ring` moved `w` points towards its filled side (negative: away from it), each corner along
/// its bisector, its mitre limited to four times `w` so sharp corners don't shoot off. Each point
/// stays the partner of its source point, so rings at different insets join face to face.
pub(super) fn inset(ring: &[Point], w: f64) -> Vec<Point> {
    if w == 0.0 {
        return ring.to_vec();
    }
    let n = ring.len();
    (0..n)
        .map(|i| {
            let p = ring[i];
            let prev = ring[(i + n - 1) % n];
            let next = ring[(i + 1) % n];
            let (ax, ay) = left(prev, p);
            let (bx, by) = left(p, next);
            let (mut mx, mut my) = (ax + bx, ay + by);
            let ml = mx.hypot(my);
            if ml < 1e-9 {
                (mx, my) = (ax, ay);
            } else {
                (mx, my) = (mx / ml, my / ml);
            }
            let cos = (mx * ax + my * ay).max(0.25);
            Point::new(p.x + mx * w / cos, p.y + my * w / cos)
        })
        .collect()
}
