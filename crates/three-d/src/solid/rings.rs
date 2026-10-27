use super::*;

/// Outline points Extrude & Bevel and Inflate sample at most: every point costs walls, bevel
/// bands and distance tests.
pub(super) const MAX_SOLID_POINTS: usize = 2048;

/// Outline points Rotate samples at most: it only turns the outline, so an outlined paragraph fits.
pub(super) const MAX_PLANE_POINTS: usize = 65536;

/// Pieces a twisted wall's strip is cut into along one edge at most.
pub(super) const MAX_PIECES: usize = 64;

/// How many straight pieces follow `c` within `tolerance` (Wang's bound on a cubic's distance from
/// its chords), 1 to 32.
fn pieces(c: vectorcraft_geom::CubicBez, tolerance: f64) -> u32 {
    let d = |a: Point, b: Point, c: Point| (a.to_vec2() - 2.0 * b.to_vec2() + c.to_vec2()).hypot();
    let bend = d(c.p0, c.p1, c.p2).max(d(c.p1, c.p2, c.p3));
    let n = (0.75 * bend / tolerance).sqrt().ceil();
    if n.is_finite() { (n as u32).clamp(1, 32) } else { 32 }
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
    let tolerance = (size / 2000.0).max(0.05);
    let mut count = 0;
    let mut out = vec![];
    for sp in &path.subpaths {
        let segments = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        if segments > limit {
            return Err("The shape is too complex for a 3D effect".into());
        }
        let mut ring: Vec<Point> = vec![];
        if let Some(first) = sp.anchors.first() {
            ring.push(first.p);
        }
        for i in 0..segments {
            let curve = sp.segment(i);
            let samples = if sp.segment_is_line(i) { 1 } else { pieces(curve, tolerance) };
            count += samples as usize;
            if count > limit {
                return Err("The shape is too complex for a 3D effect".into());
            }
            for k in 1..=samples {
                ring.push(curve.eval(f64::from(k) / f64::from(samples)));
            }
        }
        if ring.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e8 || p.y.abs() > 1e8) {
            return Err("The shape contains invalid coordinates".into());
        }
        ring.dedup_by(|a, b| a.distance(*b) < 1e-9);
        while ring.len() > 1 && ring.first().zip(ring.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-9) {
            ring.pop();
        }
        if ring.len() >= 3 && signed_area(&ring).abs() > 1e-9 {
            out.push(ring);
        }
    }
    // Which rings hold which: each ring against the others, its edges walked only where its
    // bounds hold the point, and within [`MAX_CONTAINMENT_EDGES`] edges in all.
    let too_complex = || String::from("The shape is too complex for a 3D effect");
    let mut work = 0usize;
    let mut within = |out: &[Vec<Point>], bounds: &[Rect], i: usize| -> Result<Vec<usize>, String> {
        let p = out.get(i).and_then(|r| r.first()).copied().unwrap_or_default();
        let mut held_by = vec![];
        for (j, (o, b)) in out.iter().zip(bounds).enumerate() {
            if j == i || !(b.x0 <= p.x && p.x <= b.x1 && b.y0 <= p.y && p.y <= b.y1) {
                continue;
            }
            work = work.saturating_add(o.len());
            if work > MAX_CONTAINMENT_EDGES {
                return Err(too_complex());
            }
            if contains(o, p) {
                held_by.push(j);
            }
        }
        Ok(held_by)
    };
    let bounds_of = |out: &[Vec<Point>]| -> Vec<Rect> {
        out.iter().map(|r| r.iter().map(|q| Rect::from_points(*q, *q)).reduce(|a, b| a.union(b)).unwrap_or_default()).collect()
    };
    if rule == FillRule::NonZero {
        let turn = |r: &[Point]| if signed_area(r) > 0.0 { 1 } else { -1 };
        let bounds = bounds_of(&out);
        let mut edge = Vec::with_capacity(out.len());
        for (i, r) in out.iter().enumerate() {
            // The winding number just outside the ring, and just inside it.
            let outside: i32 = within(&out, &bounds, i)?.into_iter().filter_map(|j| out.get(j)).map(|o| turn(o)).sum();
            edge.push((outside != 0) != (outside + turn(r) != 0));
        }
        let mut edge = edge.into_iter();
        out.retain(|_| edge.next().unwrap_or(true));
    }
    // A ring inside an odd number of others is a hole.
    let bounds = bounds_of(&out);
    let holes: Vec<bool> = (0..out.len()).map(|i| within(&out, &bounds, i).map(|held| held.len() % 2 == 1)).collect::<Result<_, _>>()?;
    for (r, hole) in out.iter_mut().zip(holes) {
        if (signed_area(r) > 0.0) == hole {
            r.reverse();
        }
    }
    Ok(out)
}

/// Most ring edges [`rings`] walks to tell which rings hold which: every ring is tested against
/// the others, so many rings of many points each would otherwise take billions of steps.
const MAX_CONTAINMENT_EDGES: usize = 50_000_000;

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
