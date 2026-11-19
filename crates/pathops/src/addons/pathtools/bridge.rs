//! Smooth Bridge: two circles joined into one outline by two concave arcs, each touching both
//! circles from outside, so the outline runs on smoothly from each circle into the bridge; and
//! many circles joined at once, each to the others within reach ([`bridge_many`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, SQRT_2, TAU};

use kurbo::{CubicBez, ParamCurve, Point, Shape, Vec2};
use vectorcraft_geom::{Anchor, AnchorKind, FillRule, PathData, SubPath, shapes};

use crate::boolean::{DEFAULT_PRECISION, try_unite_all_with};

use super::Circle;

/// How far (as a share of the radius) a path's outline may stray from a circle and still be read
/// as that circle.
const ROUND: f64 = 1e-3;
/// Points [`circle_of`] checks on each segment, its ends included.
const SAMPLES: usize = 9;

/// Why [`smooth_bridge`] makes no outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// One circle is inside the other (or they share a center).
    Inside,
    /// The circles are too far apart for the radius: the two arcs would not reach both circles,
    /// or would cross before they do.
    TooFar,
    /// A radius or circle that is not a finite size above 0, or an outline too large for `f64`.
    Bad,
}

/// The circle `path` draws: one closed subpath whose outline stays within 0.1% of its radius of
/// a circle all the way around, once. `None` for any other path.
pub fn circle_of(path: &PathData) -> Option<Circle> {
    let [sp] = path.subpaths.as_slice() else { return None };
    if !sp.closed || sp.anchors.len() < 2 {
        return None;
    }
    let b = path.bounds()?;
    let (w, h) = (b.width(), b.height());
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) || (w - h).abs() > ROUND * w.max(h) {
        return None;
    }
    let (center, radius) = (b.center(), (w + h) / 4.0);
    let off = |q: Point| (q.distance(center) - radius).abs() > ROUND * radius;
    for k in 0..sp.segment_count() {
        let seg = sp.segment(k);
        if (0..SAMPLES).any(|i| off(seg.eval(i as f64 / (SAMPLES - 1) as f64))) {
            return None;
        }
    }
    // Around once: a path going around twice, or back on itself, encloses a different area.
    let area = path.to_bezpath().area().abs();
    ((area - PI * radius * radius).abs() <= 4.0 * ROUND * area).then_some(Circle { center, radius })
}

/// The outline of circles `a` and `b` joined by two arcs of radius `radius` (pt), one on each
/// side of the line through their centers, each touching both circles from outside. The outline
/// runs around `a` on its side away from `b`, along one arc, around `b` on its side away from `a`
/// and back along the other arc; every anchor is smooth, as each arc and the circle it meets share
/// a tangent there. Overlapping and touching circles are joined too. A larger radius makes a wider,
/// gentler bridge; farther apart circles make a narrower one, until the arcs would cross.
pub fn smooth_bridge(a: Circle, b: Circle, radius: f64) -> Result<SubPath, BridgeError> {
    let t = touches(a, b, radius)?;
    let mut segs = vec![];
    arc(&mut segs, a.center, a.radius, t.a1, t.a2, Some(-t.u));
    arc(&mut segs, t.p2, radius, t.a2, t.b2, None);
    arc(&mut segs, b.center, b.radius, t.b2, t.b1, Some(t.u));
    arc(&mut segs, t.p1, radius, t.b1, t.a1, None);
    closed(&segs, AnchorKind::Smooth)
}

/// The part of [`smooth_bridge`]'s outline between the circles: the two arcs, joined across each
/// circle by a straight line between the arc ends on it. Each line lies inside its circle, so the
/// two circles and this shape unite into the outline, with each circle drawn once however many
/// bridges it has.
fn neck(a: Circle, b: Circle, radius: f64) -> Result<SubPath, BridgeError> {
    let t = touches(a, b, radius)?;
    let line = |p: Point, q: Point| CubicBez::new(p, p, q, q);
    let mut segs = vec![];
    arc(&mut segs, t.p2, radius, t.a2, t.b2, None);
    segs.push(line(t.b2, t.b1));
    arc(&mut segs, t.p1, radius, t.b1, t.a1, None);
    segs.push(line(t.a1, t.a2));
    closed(&segs, AnchorKind::Corner)
}

/// A closed subpath through the cubics `segs`, each starting where the one before it ends, with
/// anchors of `kind`; [`BridgeError::Bad`] when a point does not fit in `f64`.
fn closed(segs: &[CubicBez], kind: AnchorKind) -> Result<SubPath, BridgeError> {
    let count = segs.len();
    let anchors: Vec<Anchor> = (0..count)
        .filter_map(|i| {
            let (prev, seg) = (segs.get((i + count - 1) % count)?, segs.get(i)?);
            Some(Anchor { p: seg.p0, h_in: prev.p2, h_out: seg.p1, kind })
        })
        .collect();
    let finite_point = |p: Point| p.x.is_finite() && p.y.is_finite();
    if !anchors.iter().all(|q| finite_point(q.p) && finite_point(q.h_in) && finite_point(q.h_out)) {
        return Err(BridgeError::Bad);
    }
    Ok(SubPath::new(anchors, true))
}

/// Where the arcs of radius `radius` that bridge circles `a` and `b` lie: the unit vector from
/// `a`'s center to `b`'s, and on each side the arc's center (`p1`, `p2`) and where it touches `a`
/// and `b`; and the neck's width where the arcs come nearest each other between the circles
/// (infinite when that is at the circles).
struct Touches {
    waist: f64,
    u: Vec2,
    p1: Point,
    a1: Point,
    b1: Point,
    p2: Point,
    a2: Point,
    b2: Point,
}

/// The arcs of [`smooth_bridge`], or why there are none.
fn touches(a: Circle, b: Circle, radius: f64) -> Result<Touches, BridgeError> {
    let finite = |x: f64| x.is_finite() && x > 0.0;
    let points = [a.center, b.center].iter().all(|c| c.x.is_finite() && c.y.is_finite());
    if !finite(radius) || !finite(a.radius) || !finite(b.radius) || !points {
        return Err(BridgeError::Bad);
    }
    let axis = b.center - a.center;
    // `f64::hypot`: `Vec2::hypot` squares the length, which overflows above about 1e154 pt.
    let d = axis.x.hypot(axis.y);
    if d + a.radius.min(b.radius) <= a.radius.max(b.radius) * (1.0 + 1e-9) {
        return Err(BridgeError::Inside);
    }
    // The arcs' centers are `radius` outside both circles: where circles about the two centers,
    // `radius` larger than each, cross.
    let (ra, rb) = (radius + a.radius, radius + b.radius);
    if !ra.is_finite() || !rb.is_finite() {
        return Err(BridgeError::Bad);
    }
    // Halves and no squares, so no step overflows while the lengths fit in `f64`.
    let half = ra / 2.0 + rb / 2.0;
    if d / 2.0 > half {
        return Err(BridgeError::TooFar);
    }
    let along = d / 2.0 + (ra - rb) / d * half;
    let across = (ra - along).max(0.0).sqrt() * (ra / 2.0 + along / 2.0).max(0.0).sqrt() * SQRT_2;
    if !along.is_finite() || !across.is_finite() {
        return Err(BridgeError::Bad);
    }
    // Each arc runs between its points on `a` and `b`, which lie towards the two centers from the
    // arc's center. Its point nearest the axis is the foot of its center on the axis when that foot
    // is between the two centers, else one of its ends, which are off the axis. So the two arcs
    // cross, leaving no neck, only when that foot is between the centers and nearer to the arc's
    // center than its radius.
    if along > 0.0 && along < d && across <= radius * (1.0 + 1e-9) {
        return Err(BridgeError::TooFar);
    }
    let u = axis / d;
    let n = Vec2::new(-u.y, u.x);
    // Each side's arc center and where its arc touches `a` and `b`.
    let side = |s: f64| {
        let p = a.center + u * along + n * (s * across);
        (p, a.center + (p - a.center) * (a.radius / ra), b.center + (p - b.center) * (b.radius / rb))
    };
    let ((p1, a1, b1), (p2, a2, b2)) = (side(1.0), side(-1.0));
    let waist = if along > 0.0 && along < d { 2.0 * (across - radius) } else { f64::INFINITY };
    Ok(Touches { waist, u, p1, a1, b1, p2, a2, b2 })
}

/// The most circles [`bridge_many`] takes.
pub const MAX_BRIDGED_CIRCLES: usize = 2000;
/// The most bridges [`bridge_many`] makes.
pub const MAX_BRIDGES: usize = 8000;
/// Holes [`bridge_many`] fills: those smaller than this share of the area of the smallest circle
/// they join.
const SPECK: f64 = 0.01;

/// The radius of the arcs that bridge two circles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArcRadius {
    /// This share of the smaller circle's radius (1 is the same radius).
    Share(f64),
    /// The smaller circle's radius plus this gap (pt). Each arc is then this gap outside a circle
    /// of the smaller circle's radius about the arc's center: for equal circles that gap apart,
    /// the place of a third circle that gap from both, as in a triangular grid.
    Gap(f64),
}

impl ArcRadius {
    /// The arcs' radius (pt) for circles `a` and `b`.
    pub fn between(self, a: Circle, b: Circle) -> f64 {
        let r = a.radius.min(b.radius);
        match self {
            ArcRadius::Share(share) => r * share,
            ArcRadius::Gap(gap) => r + gap,
        }
    }
}

/// Circles that bridges join into one shape: their indices, ascending, and the shape's outline.
#[derive(Clone, Debug, PartialEq)]
pub struct Joined {
    pub members: Vec<usize>,
    pub outline: PathData,
}

/// What [`bridge_many`] makes: the joined shapes, in the order of their first member, and the
/// indices of the circles left alone, ascending.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bridged {
    pub joined: Vec<Joined>,
    pub alone: Vec<usize>,
}

/// Why [`bridge_many`] makes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManyError {
    /// More than [`MAX_BRIDGED_CIRCLES`] circles.
    TooManyCircles,
    /// More than [`MAX_BRIDGES`] pairs of circles within reach that the arcs can bridge.
    TooManyBridges,
    /// No two circles are within reach of each other.
    NoneInReach,
    /// The circles at these indices are within reach but [`smooth_bridge`] can't join them: one is
    /// inside the other, or no pair within reach is near enough for the arcs (the first such pair).
    Pair(usize, usize, BridgeError),
    /// A reach that is not a finite size of 0 or more, or a circle that is not a finite size above 0.
    Bad,
    /// A joined shape too complex to unite.
    TooComplex,
}

/// Every pair of `circles` whose outlines are at most `reach` (pt) apart, overlapping ones
/// included, joined by a [`smooth_bridge`] with arcs of radius `arc`, and the bridges that connect
/// united into one shape each, without holes smaller than 1% of the smallest circle's area. A pair
/// too far apart for the arcs gets no bridge; a circle bridged to no other is left alone. One
/// circle inside another is an error, and so is no pair that the arcs can join.
pub fn bridge_many(circles: &[Circle], arc: ArcRadius, reach: f64) -> Result<Bridged, ManyError> {
    let (pairs, too_far, roots, precision) = bridgeable(circles, arc, reach)?;
    match (pairs.is_empty(), too_far) {
        (true, Some((i, j))) => return Err(ManyError::Pair(i, j, BridgeError::TooFar)),
        (true, None) => return Err(ManyError::NoneInReach),
        (false, _) => {}
    }
    let mut necks = Vec::with_capacity(pairs.len());
    for &((i, j), _) in &pairs {
        let (a, b) = (circles[i], circles[j]);
        let sp = neck(a, b, arc.between(a, b)).map_err(|e| ManyError::Pair(i, j, e))?;
        necks.push(((i, j), PathData::single(sp)));
    }
    let mut sets: BTreeMap<usize, (Vec<usize>, Vec<&PathData>)> = BTreeMap::new();
    for &((i, j), ref neck) in &necks {
        let (members, parts) = sets.entry(roots[i]).or_default();
        members.extend([i, j]);
        parts.push(neck);
    }
    let mut out = Bridged::default();
    for (set, (mut members, parts)) in sets {
        members.sort_unstable();
        members.dedup();
        // Each circle once, and the necks between them.
        let rounds: Vec<PathData> = members
            .iter()
            .filter_map(|&i| circles.get(i))
            .map(|c| shapes::ellipse(kurbo::Rect::from_center_size(c.center, (2.0 * c.radius, 2.0 * c.radius))))
            .collect();
        let fills: Vec<(&PathData, FillRule)> = rounds.iter().chain(parts).map(|p| (p, FillRule::NonZero)).collect();
        let smallest = members.iter().filter_map(|&i| circles.get(i)).map(|c| c.radius).fold(f64::INFINITY, f64::min);
        let mut outline = try_unite_all_with(&fills, precision[set]).map_err(|_| ManyError::TooComplex)?;
        // Where three bridges' arcs nearly meet, the union leaves a speck of a hole; a contour that
        // encloses a circle is far larger than this.
        let speck = SPECK * PI * smallest * smallest;
        outline.subpaths.retain(|sp| PathData::single(sp.clone()).to_bezpath().area().abs() >= speck);
        out.joined.push(Joined { members, outline });
    }
    let joined: HashSet<usize> = necks.iter().flat_map(|&((i, j), _)| [i, j]).collect();
    out.alone = (0..circles.len()).filter(|i| !joined.contains(i)).collect();
    Ok(out)
}

/// Whether [`bridge_many`] would bridge at least one pair of `circles` with arcs of `arc` and
/// `reach`, under the same rules (a neck too thin for its shape's precision included), without
/// making the bridges or uniting them.
pub fn bridges_any(circles: &[Circle], arc: ArcRadius, reach: f64) -> bool {
    bridgeable(circles, arc, reach).is_ok_and(|(pairs, ..)| !pairs.is_empty())
}

/// What [`bridgeable`] finds: the pairs [`bridge_many`] bridges with their necks' widths, the
/// lowest pair within reach that it can't bridge, each circle's set (its lowest index) and each
/// set's precision, under its lowest index.
type Bridgeable = (Vec<(Pair, f64)>, Option<Pair>, Vec<usize>, Vec<f64>);

/// The pairs of `circles` that [`bridge_many`] bridges, and how it sorts them into sets.
fn bridgeable(circles: &[Circle], arc: ArcRadius, reach: f64) -> Result<Bridgeable, ManyError> {
    if circles.len() > MAX_BRIDGED_CIRCLES {
        return Err(ManyError::TooManyCircles);
    }
    let fine = |c: &Circle| c.center.x.is_finite() && c.center.y.is_finite() && c.radius.is_finite() && c.radius > 0.0;
    if !(reach.is_finite() && reach >= 0.0) || !circles.iter().all(fine) {
        return Err(ManyError::Bad);
    }
    let (mut pairs, mut too_far) = pairs_within(circles, reach, arc)?;
    // Each set of connected circles, under its lowest index, is united at a precision fine enough
    // for its smallest circle. A neck narrower than twice that would not survive the union, so its
    // pair gets no bridge; that can split a set and make its precision coarser, so the sets are
    // found again until no neck is dropped.
    let (roots, precision) = loop {
        let mut parent: Vec<usize> = (0..circles.len()).collect();
        for &((i, j), _) in &pairs {
            let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
            parent[ri.max(rj)] = ri.min(rj);
        }
        let roots: Vec<usize> = (0..circles.len()).map(|i| root(&mut parent, i)).collect();
        let mut precision = vec![DEFAULT_PRECISION; circles.len()];
        for (&r, c) in roots.iter().zip(circles) {
            precision[r] = precision[r].min(c.radius * 1e-3);
        }
        let thin = |&((i, _), waist): &(Pair, f64)| waist < 2.0 * precision[roots[i]];
        if !pairs.iter().any(thin) {
            break (roots, precision);
        }
        for &(pair, _) in pairs.iter().filter(|&p| thin(p)) {
            too_far = Some(too_far.map_or(pair, |t| t.min(pair)));
        }
        pairs.retain(|p| !thin(p));
    };
    Ok((pairs, too_far, roots, precision))
}

/// Two circles' indices, the lower first.
type Pair = (usize, usize);
/// What [`pairs_within`] finds: the pairs the arcs can bridge, each with its neck's width, and the
/// lowest pair too far apart for them.
type Within = (Vec<(Pair, f64)>, Option<Pair>);

/// The set `i` is in: the lowest index in it, with the path to it shortened on the way.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Every pair of indices `(i, j)`, `i < j`, ascending, of `circles` whose outlines are at most
/// `reach` apart and that arcs of `arc` can bridge, with the width of the neck between them, and
/// the lowest pair within reach that they can't because it is too far apart for them. Circles go
/// into a grid of cells as wide as the farthest two centers within reach can be, so each circle is
/// compared only with those in its own cell and the eight around it.
fn pairs_within(circles: &[Circle], reach: f64, arc: ArcRadius) -> Result<Within, ManyError> {
    let biggest = circles.iter().map(|c| c.radius).fold(0.0, f64::max);
    let cell = 2.0 * biggest + reach;
    if !(cell.is_finite() && cell > 0.0) {
        return Err(ManyError::Bad);
    }
    // Saturating casts: a cell beyond i64 shares the last one, which only adds comparisons.
    let key = |p: Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, c) in circles.iter().enumerate() {
        grid.entry(key(c.center)).or_default().push(i);
    }
    let (mut pairs, mut too_far) = (vec![], None::<Pair>);
    for (i, &c) in circles.iter().enumerate() {
        let (x, y) = key(c.center);
        for (dx, dy) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (dx, dy))) {
            let Some(near) = grid.get(&(x.saturating_add(dx), y.saturating_add(dy))) else { continue };
            for &j in near.iter().filter(|&&j| j > i) {
                let o = circles[j];
                if c.center.distance(o.center) - c.radius - o.radius > reach {
                    continue;
                }
                // Only the arcs' ends are worked out here; the bridge itself is made later.
                match touches(c, o, arc.between(c, o)) {
                    Ok(t) => pairs.push(((i, j), t.waist)),
                    Err(BridgeError::TooFar) => too_far = Some(too_far.map_or((i, j), |t| t.min((i, j)))),
                    Err(e) => return Err(ManyError::Pair(i, j, e)),
                }
            }
        }
        // Saturated cells can list a pair twice, so the count is checked again after `dedup`.
        if pairs.len() > 9 * MAX_BRIDGES {
            return Err(ManyError::TooManyBridges);
        }
    }
    pairs.sort_unstable_by_key(|&(pair, _)| pair);
    pairs.dedup_by_key(|&mut (pair, _)| pair);
    if pairs.len() > MAX_BRIDGES {
        return Err(ManyError::TooManyBridges);
    }
    Ok((pairs, too_far))
}

/// Append to `out` the arc of the circle about `center` with radius `r` from `from` to `to` (both
/// on it): through the direction `through` from the center when given, else the shorter way. In
/// cubics of at most a quarter turn each, `from` exactly at the start and `to` at the end.
fn arc(out: &mut Vec<CubicBez>, center: Point, r: f64, from: Point, to: Point, through: Option<Vec2>) {
    let angle = |q: Vec2| q.y.atan2(q.x);
    let start = angle(from - center);
    // The turn from `from` to `to` counter-clockwise (in the y-up sense), 0 to 2π.
    let ccw = (angle(to - center) - start).rem_euclid(TAU);
    let sweep = match through {
        Some(dir) if (angle(dir) - start).rem_euclid(TAU) <= ccw => ccw,
        Some(_) => ccw - TAU,
        None if ccw > PI => ccw - TAU,
        None => ccw,
    };
    let pieces = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / pieces as f64;
    // Handle length that makes a cubic follow the arc of `step`, signed with its direction.
    let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
    let on = |t: f64| center + Vec2::new(t.cos(), t.sin()) * r;
    let tangent = |t: f64| Vec2::new(-t.sin(), t.cos());
    for i in 0..pieces {
        let (t0, t1) = (start + step * i as f64, start + step * (i + 1) as f64);
        let p0 = if i == 0 { from } else { on(t0) };
        let p3 = if i + 1 == pieces { to } else { on(t1) };
        out.push(CubicBez::new(p0, p0 + tangent(t0) * k, p3 - tangent(t1) * k, p3));
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_geom::shapes;

    use super::*;

    fn circle(x: f64, y: f64, r: f64) -> Circle {
        Circle { center: Point::new(x, y), radius: r }
    }

    /// Every point the outline passes through, 32 to a segment.
    fn outline(sp: &SubPath) -> Vec<Point> {
        (0..sp.segment_count()).flat_map(|k| (0..32).map(move |i| sp.segment(k).eval(i as f64 / 32.0))).collect()
    }

    /// The outline crosses the vertical line at `x` at these y values, top to bottom.
    fn crossings_at(sp: &SubPath, x: f64) -> Vec<f64> {
        let pts = outline(sp);
        let mut ys: Vec<f64> = (0..pts.len())
            .filter_map(|i| {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                ((p.x - x) * (q.x - x) <= 0.0 && p.x != q.x).then(|| p.y + (q.y - p.y) * (x - p.x) / (q.x - p.x))
            })
            .collect();
        ys.sort_by(f64::total_cmp);
        ys
    }

    /// The bridge's width at `x`, from the outline's crossings there.
    fn width_at(sp: &SubPath, x: f64) -> f64 {
        let ys = crossings_at(sp, x);
        ys.last().unwrap() - ys.first().unwrap()
    }

    fn assert_smooth(sp: &SubPath) {
        for a in sp.anchors.iter() {
            let (i, o) = (a.p - a.h_in, a.h_out - a.p);
            assert!(i.hypot() > 1e-6 && o.hypot() > 1e-6, "both handles at {a:?}");
            assert!(i.cross(o).abs() <= 1e-9 * i.hypot() * o.hypot() && i.dot(o) > 0.0, "the outline turns at {a:?}");
            assert_eq!(a.kind, AnchorKind::Smooth);
        }
    }

    /// Each point of the outline is on circle `c` or outside it.
    fn outside(sp: &SubPath, c: Circle) -> bool {
        outline(sp).iter().all(|p| p.distance(c.center) >= c.radius - 1e-3)
    }

    #[test]
    fn two_equal_circles_join_with_a_neck_narrower_than_the_circles() {
        let (a, b) = (circle(0.0, 0.0, 20.0), circle(60.0, 0.0, 20.0));
        let sp = smooth_bridge(a, b, 20.0).unwrap();
        assert!(sp.closed);
        assert_smooth(&sp);
        // The arcs' centers are 40 from each circle's center: (30, ±√700), so the neck at x = 30 is
        // 2 × (√700 − 20) wide.
        let neck = 2.0 * (700f64.sqrt() - 20.0);
        assert!((width_at(&sp, 30.0) - neck).abs() < 1e-2, "{}", width_at(&sp, 30.0));
        assert!(width_at(&sp, 30.0) < width_at(&sp, 10.0));
        // Around each circle on its far side, and symmetric.
        let ys = crossings_at(&sp, -10.0);
        assert!((ys[0] + 300f64.sqrt()).abs() < 1e-2 && (ys[1] - 300f64.sqrt()).abs() < 1e-2, "{ys:?}");
        assert!((width_at(&sp, 50.0) - width_at(&sp, 10.0)).abs() < 1e-2);
        let b = PathData::single(sp).bounds().unwrap();
        // Within the cubics' own error from a true circle (about 0.03% of the radius).
        assert!((b.x0 + 20.0).abs() < 1e-2 && (b.x1 - 80.0).abs() < 1e-2 && (b.y1 - 20.0).abs() < 1e-2, "{b:?}");
    }

    #[test]
    fn the_arcs_touch_both_circles_and_run_on_from_them() {
        for (a, b, r) in [
            (circle(0.0, 0.0, 20.0), circle(60.0, 0.0, 20.0), 20.0),
            (circle(10.0, 5.0, 30.0), circle(70.0, 40.0, 8.0), 40.0),
            (circle(0.0, 0.0, 20.0), circle(30.0, 0.0, 15.0), 10.0),
        ] {
            let sp = smooth_bridge(a, b, r).unwrap();
            assert_smooth(&sp);
            // The outline goes around the outside of both circles, never into either.
            assert!(outside(&sp, a) && outside(&sp, b));
            // On each circle the outline leaves along the circle's tangent, where an arc takes over
            // as well: two anchors on each circle are where an arc meets it.
            for c in [a, b] {
                let on: Vec<_> = sp.anchors.iter().filter(|q| (q.p.distance(c.center) - c.radius).abs() < 1e-6).collect();
                assert!(on.len() >= 2, "{}", on.len());
                for q in on {
                    let radial = q.p - c.center;
                    assert!(radial.dot(q.h_out - q.p).abs() <= 1e-6 * radial.hypot() * (q.h_out - q.p).hypot(), "{q:?}");
                }
            }
            assert!(sp.anchors.len() <= 12, "{}", sp.anchors.len());
            // Every segment off the two circles is part of an arc of radius `r` that touches both
            // circles from outside: its center is `r` farther than each circle's radius from it.
            let on = |q: Point, c: Circle| (q.distance(c.center) - c.radius).abs() < 1e-3;
            let mut arcs = 0;
            for k in 0..sp.segment_count() {
                let seg = sp.segment(k);
                let (p0, pm, p1) = (seg.eval(0.0), seg.eval(0.5), seg.eval(1.0));
                if on(pm, a) || on(pm, b) {
                    continue;
                }
                arcs += 1;
                let (center, radius) = circumcircle(p0, pm, p1);
                assert!((radius - r).abs() < 1e-2 * r, "radius {radius} for {r}");
                assert!((center.distance(a.center) - (r + a.radius)).abs() < 1e-2 * r, "touches {a:?}");
                assert!((center.distance(b.center) - (r + b.radius)).abs() < 1e-2 * r, "touches {b:?}");
            }
            assert!((2..=4).contains(&arcs), "{arcs}");
        }
    }

    /// The circle through three points: its center and radius.
    fn circumcircle(p: Point, q: Point, s: Point) -> (Point, f64) {
        let d = 2.0 * (p.x * (q.y - s.y) + q.x * (s.y - p.y) + s.x * (p.y - q.y));
        let sq = |v: Point| v.x * v.x + v.y * v.y;
        let x = (sq(p) * (q.y - s.y) + sq(q) * (s.y - p.y) + sq(s) * (p.y - q.y)) / d;
        let y = (sq(p) * (s.x - q.x) + sq(q) * (p.x - s.x) + sq(s) * (q.x - p.x)) / d;
        let c = Point::new(x, y);
        (c, c.distance(p))
    }

    #[test]
    fn unequal_circles_join_round_both() {
        let (a, b) = (circle(0.0, 0.0, 30.0), circle(70.0, 0.0, 10.0));
        let sp = smooth_bridge(a, b, 40.0).unwrap();
        assert_smooth(&sp);
        let bounds = PathData::single(sp.clone()).bounds().unwrap();
        assert!((bounds.x0 + 30.0).abs() < 1e-2 && (bounds.x1 - 80.0).abs() < 1e-2 && (bounds.y0 + 30.0).abs() < 1e-2, "{bounds:?}");
        // The neck narrows from the big circle towards the small one.
        assert!(width_at(&sp, 35.0) > width_at(&sp, 50.0));
        assert!(width_at(&sp, 50.0) < 20.0);
    }

    #[test]
    fn unequal_circles_join_at_large_radii_overlapping_or_not() {
        // With a large radius the arcs' centers lie beyond the smaller circle, so the arcs never
        // reach the axis even though their centers are nearer to it than their radius.
        for (a, b, r) in [
            (circle(0.0, 0.0, 10.0), circle(25.0, 0.0, 30.0), 100.0),
            (circle(0.0, 0.0, 30.0), circle(25.0, 0.0, 10.0), 30.0),
            (circle(0.0, 0.0, 30.0), circle(25.0, 0.0, 10.0), 50.0),
            (circle(0.0, 0.0, 30.0), circle(60.0, 0.0, 8.0), 300.0),
        ] {
            let sp = smooth_bridge(a, b, r).unwrap_or_else(|e| panic!("{a:?} {b:?} {r}: {e:?}"));
            assert_smooth(&sp);
            assert!(outside(&sp, a) && outside(&sp, b));
            // The outline crosses the axis only at the two far ends: it does not cross itself.
            let pts = outline(&sp);
            let crossings = (0..pts.len()).filter(|&i| (pts[i].y > 0.0) != (pts[(i + 1) % pts.len()].y > 0.0)).count();
            assert_eq!(crossings, 2, "{a:?} {b:?} {r}");
            // It encloses more than the larger circle.
            let area = PathData::single(sp).to_bezpath().area().abs();
            assert!(area > PI * a.radius.max(b.radius).powi(2), "{area}");
        }
    }

    #[test]
    fn overlapping_and_touching_circles_are_joined() {
        let (a, b) = (circle(0.0, 0.0, 20.0), circle(30.0, 0.0, 20.0));
        let sp = smooth_bridge(a, b, 10.0).unwrap();
        assert_smooth(&sp);
        // The circles cross at x = 15, y = ±√175; the bridge fills in the notch there.
        assert!(width_at(&sp, 15.0) > 2.0 * 175f64.sqrt());
        assert!(width_at(&sp, 15.0) < 40.0);
        let touching = smooth_bridge(circle(0.0, 0.0, 20.0), circle(40.0, 0.0, 20.0), 10.0).unwrap();
        assert_smooth(&touching);
        assert!(width_at(&touching, 20.0) > 1.0);
    }

    #[test]
    fn far_apart_circles_and_nested_ones_make_no_bridge() {
        let (a, b) = (circle(0.0, 0.0, 10.0), circle(100.0, 0.0, 10.0));
        assert_eq!(smooth_bridge(a, b, 5.0), Err(BridgeError::TooFar), "arcs don't reach");
        // The arcs reach both circles (gap 80 < 2 × 45) but cross before they do.
        assert_eq!(smooth_bridge(a, b, 45.0), Err(BridgeError::TooFar), "no neck left");
        // A large enough radius bridges them: the neck stays open above (d/2)² − r² over 2r = 120.
        assert!(smooth_bridge(a, b, 130.0).is_ok());
        assert_eq!(smooth_bridge(circle(0.0, 0.0, 30.0), circle(5.0, 0.0, 10.0), 10.0), Err(BridgeError::Inside));
        assert_eq!(smooth_bridge(circle(0.0, 0.0, 30.0), circle(0.0, 0.0, 30.0), 10.0), Err(BridgeError::Inside));
        assert_eq!(smooth_bridge(a, b, 0.0), Err(BridgeError::Bad));
        assert_eq!(smooth_bridge(a, b, f64::NAN), Err(BridgeError::Bad));
        assert_eq!(smooth_bridge(circle(f64::INFINITY, 0.0, 10.0), b, 10.0), Err(BridgeError::Bad));
    }

    /// A 3 × 3 grid of circles of radius 10, 4 apart (centers 24 apart), top-left at the origin.
    fn grid() -> Vec<Circle> {
        (0..9).map(|k| circle(24.0 * (k % 3) as f64, 24.0 * (k / 3) as f64, 10.0)).collect()
    }

    fn area(path: &PathData) -> f64 {
        crate::area(path, FillRule::NonZero)
    }

    fn inside(path: &PathData, x: f64, y: f64) -> bool {
        path.to_bezpath().winding(Point::new(x, y)) != 0
    }

    #[test]
    fn a_grid_joins_each_circle_to_its_neighbors_within_reach() {
        // Reach 5: the side neighbors (4 apart) are joined, the diagonal ones (about 13.9 apart) not.
        let b = bridge_many(&grid(), ArcRadius::Gap(4.0), 5.0).unwrap();
        assert_eq!(b.alone, Vec::<usize>::new());
        let [one] = b.joined.as_slice() else { panic!("{} shapes", b.joined.len()) };
        assert_eq!(one.members, (0..9).collect::<Vec<_>>());
        for (x, y) in [(12.0, 0.0), (0.0, 12.0), (36.0, 48.0), (48.0, 36.0), (24.0, 24.0)] {
            assert!(inside(&one.outline, x, y), "bridged at ({x}, {y})");
        }
        // Between four circles the side bridges' arcs leave a hole: no diagonal bridge fills it.
        for (x, y) in [(12.0, 12.0), (36.0, 36.0)] {
            assert!(!inside(&one.outline, x, y), "a hole at ({x}, {y})");
        }
        let hull = PathData::single(SubPath::polyline(
            &[Point::new(-10.0, -10.0), Point::new(58.0, -10.0), Point::new(58.0, 58.0), Point::new(-10.0, 58.0)],
            true,
        ));
        assert!(area(&one.outline) < area(&hull));
        assert_eq!(one.outline.subpaths.len(), 5, "the outline and a hole in each square");
        // Reach 15 takes in the diagonal neighbors too, and their bridges fill the middle of each
        // square. The specks of holes (about 0.1 pt²) where side and diagonal arcs nearly meet are
        // filled.
        let b = bridge_many(&grid(), ArcRadius::Gap(4.0), 15.0).unwrap();
        let [one] = b.joined.as_slice() else { panic!("{} shapes", b.joined.len()) };
        assert!(inside(&one.outline, 12.0, 12.0) && inside(&one.outline, 36.0, 36.0));
        assert_eq!(one.outline.subpaths.len(), 1);
    }

    #[test]
    fn a_circle_with_no_neighbor_in_reach_is_left_alone() {
        let mut circles = grid();
        circles.truncate(2);
        circles.push(circle(200.0, 0.0, 10.0));
        circles.push(circle(224.0, 0.0, 10.0));
        circles.push(circle(100.0, 100.0, 10.0));
        let b = bridge_many(&circles, ArcRadius::Share(1.0), 5.0).unwrap();
        assert_eq!(b.joined.iter().map(|j| j.members.clone()).collect::<Vec<_>>(), [vec![0, 1], vec![2, 3]]);
        assert_eq!(b.alone, [4]);
        // Each pair alone is the two-circle bridge.
        let two = smooth_bridge(circles[0], circles[1], 10.0).unwrap();
        let (got, want) = (area(&b.joined[0].outline), area(&PathData::single(two)));
        assert!((got - want).abs() < 1e-3 * want, "{got} {want}");
    }

    #[test]
    fn a_ring_around_an_empty_place_leaves_a_round_hole_the_gap_wider_than_a_circle() {
        // Six circles of radius 10 around an empty place at the origin, each 4 from its two
        // neighbors and from where a seventh circle would be (centers 24 from the origin).
        let ring: Vec<Circle> = (0..6).map(|k| f64::from(k) * PI / 3.0).map(|t| circle(24.0 * t.cos(), 24.0 * t.sin(), 10.0)).collect();
        let b = bridge_many(&ring, ArcRadius::Gap(4.0), 5.0).unwrap();
        let [one] = b.joined.as_slice() else { panic!("{} shapes", b.joined.len()) };
        assert_eq!(one.members, (0..6).collect::<Vec<_>>());
        assert_eq!(one.outline.subpaths.len(), 2, "the outline and the hole");
        // The hole's edge is 14 from the origin all the way round: a circle of radius 10 there
        // would be 4 from the shape everywhere, the same as from each of the six.
        let hole = one.outline.subpaths.iter().min_by(|p, q| {
            let size = |sp: &SubPath| PathData::single(sp.clone()).to_bezpath().area().abs();
            size(p).total_cmp(&size(q))
        });
        let edge = outline(hole.unwrap());
        let (near, far) = edge.iter().map(|p| p.distance(Point::ZERO)).fold((f64::INFINITY, 0.0f64), |(n, f), d| (n.min(d), f.max(d)));
        assert!((near - 14.0).abs() < 1e-2 && (far - 14.0).abs() < 1e-2, "{near} to {far}");
    }

    #[test]
    fn many_circles_make_the_two_circle_outline_for_each_pair() {
        // Overlapping, touching, unequal and unequal overlapping pairs, each with a far circle.
        for (a, b, share) in [
            (circle(0.0, 0.0, 20.0), circle(30.0, 0.0, 20.0), 0.5),
            (circle(0.0, 0.0, 20.0), circle(40.0, 0.0, 20.0), 0.5),
            (circle(0.0, 0.0, 30.0), circle(70.0, 0.0, 10.0), 4.0),
            (circle(0.0, 0.0, 10.0), circle(25.0, 0.0, 30.0), 10.0),
            // Circles far smaller than the union's usual precision (0.01 pt).
            (circle(0.0, 0.0, 0.004), circle(0.01, 0.0, 0.004), 1.0),
        ] {
            let made = bridge_many(&[a, b, circle(1000.0, 0.0, 5.0)], ArcRadius::Share(share), 40.0).unwrap();
            assert_eq!(made.alone, [2]);
            let two = smooth_bridge(a, b, share * a.radius.min(b.radius)).unwrap();
            let (got, want) = (area(&made.joined[0].outline), area(&PathData::single(two)));
            assert!((got - want).abs() < 1e-3 * want, "{a:?}: {got} {want}");
        }
    }

    #[test]
    fn a_pair_too_far_apart_for_the_arcs_gets_no_bridge() {
        // b and c are within reach (6 apart) but arcs of radius 3 close before they reach both.
        let (a, b, c) = (circle(0.0, 0.0, 3.0), circle(8.0, 0.0, 3.0), circle(20.0, 0.0, 3.0));
        assert_eq!(smooth_bridge(b, c, 3.0), Err(BridgeError::TooFar));
        let made = bridge_many(&[a, b, c], ArcRadius::Share(1.0), 10.0).unwrap();
        assert_eq!(made.joined.iter().map(|j| j.members.clone()).collect::<Vec<_>>(), [vec![0, 1]]);
        assert_eq!(made.alone, [2]);
        // Arcs of radius 10 between circles of radius 10 just under 20√3 apart leave a neck about
        // 0.00003 pt wide, too thin for the union: that pair is left unbridged too.
        let (a, b, c) = (circle(0.0, 0.0, 10.0), circle(34.6409, 0.0, 10.0), circle(58.6409, 0.0, 10.0));
        assert!(smooth_bridge(a, b, 10.0).is_ok(), "two circles alone are bridged");
        let made = bridge_many(&[a, b, c], ArcRadius::Share(1.0), 20.0).unwrap();
        assert_eq!(made.joined.iter().map(|j| j.members.clone()).collect::<Vec<_>>(), [vec![1, 2]]);
        assert_eq!(made.alone, [0]);
        // Two tiny circles far off are united at a finer precision, which does not change that.
        let tiny = [circle(500.0, 0.0, 0.01), circle(500.025, 0.0, 0.01)];
        let made = bridge_many(&[a, b, c, tiny[0], tiny[1]], ArcRadius::Share(1.0), 20.0).unwrap();
        assert_eq!(made.joined.iter().map(|j| j.members.clone()).collect::<Vec<_>>(), [vec![1, 2], vec![3, 4]]);
        assert_eq!(made.alone, [0]);
        // 260 touching circles in a row with a wide reach: 33,670 pairs within reach, far more than
        // the bridge limit, but only the 259 touching pairs are near enough for the arcs.
        let row: Vec<Circle> = (0..260).map(|i| circle(f64::from(i) * 20.0, 0.0, 10.0)).collect();
        let made = bridge_many(&row, ArcRadius::Share(1.0), 6000.0).unwrap();
        let [one] = made.joined.as_slice() else { panic!("{} shapes", made.joined.len()) };
        assert_eq!(one.members.len(), 260);
        assert!(made.alone.is_empty());
    }

    #[test]
    fn the_gap_form_keeps_the_gap_from_a_circle_at_each_arcs_center() {
        // Two circles 4 apart; with a gap of 4 each arc's center is 24 from both centers, where a
        // third circle of the same size is 4 from both (as in a triangular grid).
        let (a, b) = (circle(0.0, 0.0, 10.0), circle(24.0, 0.0, 10.0));
        let r = ArcRadius::Gap(4.0).between(a, b);
        assert_eq!(r, 14.0);
        let sp = smooth_bridge(a, b, r).unwrap();
        for third in [circle(12.0, 432f64.sqrt(), 10.0), circle(12.0, -(432f64.sqrt()), 10.0)] {
            let gaps: Vec<f64> = outline(&sp).iter().map(|p| p.distance(third.center) - third.radius).collect();
            // No part of the bridge comes nearer than the gap, and a stretch of it is that gap away.
            assert!(gaps.iter().all(|&g| g > 4.0 - 1e-2), "{:?}", gaps.iter().copied().fold(f64::INFINITY, f64::min));
            let along = gaps.iter().filter(|&&g| (g - 4.0).abs() < 1e-2).count();
            assert!(along >= 32, "{along}");
        }
        // The same for unequal circles: the gap is kept from a circle of the smaller one's size.
        let (a, b) = (circle(0.0, 0.0, 15.0), circle(30.0, 5.0, 8.0));
        let r = ArcRadius::Gap(3.0).between(a, b);
        let sp = smooth_bridge(a, b, r).unwrap();
        let d = (b.center - a.center).hypot();
        let (ra, rb) = (15.0 + r, 8.0 + r);
        let along = (d * d + ra * ra - rb * rb) / (2.0 * d);
        let u = (b.center - a.center) / d;
        let center = a.center + u * along + Vec2::new(-u.y, u.x) * (ra * ra - along * along).sqrt();
        let gaps: Vec<f64> = outline(&sp).iter().map(|p| p.distance(center) - 8.0).collect();
        assert!(gaps.iter().all(|&g| g > 3.0 - 1e-2) && gaps.iter().filter(|&&g| (g - 3.0).abs() < 1e-2).count() >= 16);
    }

    #[test]
    fn the_gap_form_is_the_percentage_form_of_the_radius_plus_the_gap() {
        for (a, b, gap) in [(circle(0.0, 0.0, 10.0), circle(24.0, 0.0, 10.0), 4.0), (circle(0.0, 0.0, 20.0), circle(40.0, 30.0, 10.0), 7.5)] {
            let small = a.radius.min(b.radius);
            let share = ArcRadius::Share((small + gap) / small);
            assert!((ArcRadius::Gap(gap).between(a, b) - share.between(a, b)).abs() < 1e-12);
            let (g, s) = (smooth_bridge(a, b, ArcRadius::Gap(gap).between(a, b)).unwrap(), smooth_bridge(a, b, share.between(a, b)).unwrap());
            for (p, q) in g.anchors.iter().zip(&s.anchors) {
                assert!(p.p.distance(q.p) < 1e-9 && p.h_in.distance(q.h_in) < 1e-9 && p.h_out.distance(q.h_out) < 1e-9);
            }
            assert_eq!(g.anchors.len(), s.anchors.len());
        }
        // Over a grid the two forms make the same shape too.
        let by_gap = bridge_many(&grid(), ArcRadius::Gap(4.0), 5.0).unwrap();
        let by_share = bridge_many(&grid(), ArcRadius::Share(1.4), 5.0).unwrap();
        let (g, s) = (area(&by_gap.joined[0].outline), area(&by_share.joined[0].outline));
        assert!((g - s).abs() < 1e-6 * g, "{g} {s}");
    }

    #[test]
    fn too_many_circles_or_bridges_and_bad_pairs_are_refused() {
        let many = vec![circle(0.0, 0.0, 1.0); MAX_BRIDGED_CIRCLES + 1];
        assert_eq!(bridge_many(&many, ArcRadius::Share(1.0), 1.0), Err(ManyError::TooManyCircles));
        // 200 circles in a tight row: 19,900 pairs within reach.
        let row: Vec<Circle> = (0..200).map(|i| circle(i as f64 * 0.1, 0.0, 10.0)).collect();
        assert_eq!(bridge_many(&row, ArcRadius::Share(1.0), 1.0), Err(ManyError::TooManyBridges));
        let spread: Vec<Circle> = (0..3).map(|i| circle(i as f64 * 100.0, 0.0, 10.0)).collect();
        assert_eq!(bridge_many(&spread, ArcRadius::Share(1.0), 5.0), Err(ManyError::NoneInReach));
        let nested = [circle(0.0, 0.0, 30.0), circle(5.0, 0.0, 10.0), circle(200.0, 0.0, 10.0)];
        assert_eq!(bridge_many(&nested, ArcRadius::Share(1.0), 5.0), Err(ManyError::Pair(0, 1, BridgeError::Inside)));
        // Within reach (40 apart) but too far for arcs of radius 1.
        let far = [circle(0.0, 0.0, 10.0), circle(60.0, 0.0, 10.0), circle(300.0, 0.0, 10.0)];
        assert_eq!(bridge_many(&far, ArcRadius::Share(0.1), 50.0), Err(ManyError::Pair(0, 1, BridgeError::TooFar)));
        assert_eq!(bridge_many(&far, ArcRadius::Share(1.0), f64::NAN), Err(ManyError::Bad));
        assert_eq!(bridge_many(&far, ArcRadius::Share(1.0), -1.0), Err(ManyError::Bad));
        assert_eq!(bridge_many(&[circle(f64::INFINITY, 0.0, 1.0), far[0]], ArcRadius::Share(1.0), 1.0), Err(ManyError::Bad));
    }

    #[test]
    fn huge_circles_bridge_with_finite_anchors_or_are_refused() {
        // Squaring these lengths overflows f64.
        let (a, b) = (circle(0.0, 0.0, 5e153), circle(1.5e154, 0.0, 5e153));
        let sp = smooth_bridge(a, b, 5e153).unwrap();
        assert!(sp.anchors.iter().all(|q| [q.p, q.h_in, q.h_out].iter().all(|p| p.x.is_finite() && p.y.is_finite())));
        assert_smooth(&sp);
        // The two radius sums fit in f64, but not their total.
        let (a, b) = (circle(-5e307, 0.0, 1e307), circle(5e307, 0.0, 1e307));
        let sp = smooth_bridge(a, b, 1.5e308).unwrap();
        assert!(sp.anchors.iter().all(|q| [q.p, q.h_in, q.h_out].iter().all(|p| p.x.is_finite() && p.y.is_finite())));
        // The arcs' centers would lie beyond the largest f64.
        let (a, b) = (circle(0.0, 0.0, 1e308), circle(1.5e308, 0.0, 1e308));
        assert_eq!(smooth_bridge(a, b, 1e308), Err(BridgeError::Bad));
    }

    #[test]
    fn circles_are_told_from_other_paths() {
        let round = shapes::ellipse(kurbo::Rect::new(10.0, 20.0, 50.0, 60.0));
        let c = circle_of(&round).unwrap();
        assert!(c.center.distance(Point::new(30.0, 40.0)) < 1e-9 && (c.radius - 20.0).abs() < 1e-9);
        let oval = shapes::ellipse(kurbo::Rect::new(0.0, 0.0, 50.0, 40.0));
        assert_eq!(circle_of(&oval), None);
        let square = PathData::single(SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)], true));
        assert_eq!(circle_of(&square), None);
        let mut open = round.clone();
        open.subpaths[0].closed = false;
        assert_eq!(circle_of(&open), None);
        let two = PathData::new(vec![round.subpaths[0].clone(), round.subpaths[0].clone()]);
        assert_eq!(circle_of(&two), None);
    }
}
