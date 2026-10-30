//! Where paths cross: [`crossings`] finds the places, [`insert_at`] puts anchors there (which
//! Split / Add Points at Crossings then keeps or cuts at).

use std::collections::{BTreeMap, BTreeSet};

use kurbo::{CubicBez, ParamCurve, ParamCurveNearest, Point, Rect};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, SubPath};

/// Most segments [`crossings`] compares at once.
pub const MAX_CROSSING_SEGMENTS: usize = 10_000;
/// Most subdivision steps one [`crossings`] call takes.
const MAX_STEPS: usize = 4_000_000;
/// A piece of curve whose inner control points lie this close (pt) to its chord counts as straight.
const FLAT: f64 = 1e-4;
/// Deepest a curve is halved.
const MAX_DEPTH: u32 = 96;
/// Crossings closer than this (pt) are one.
const SAME: f64 = 1e-3;
/// A parameter this close to a segment's end is its anchor.
const END: f64 = 1e-6;

/// A place on a path: (subpath, segment, t).
pub type Place = (usize, usize, f64);

fn control_box(c: &CubicBez) -> Rect {
    Rect::from_points(c.p0, c.p1).union_pt(c.p2).union_pt(c.p3)
}

fn flat(c: &CubicBez) -> bool {
    let chord = c.p3 - c.p0;
    let len = chord.hypot();
    let off = |p: Point| if len > 1e-12 { chord.cross(p - c.p0).abs() / len } else { p.distance(c.p0) };
    off(c.p1) <= FLAT && off(c.p2) <= FLAT
}

/// Where the chords of `a` and `b` cross: (s, u) along each, or `None` when they don't (parallel
/// ones never do).
fn chords_cross(a: &CubicBez, b: &CubicBez) -> Option<(f64, f64)> {
    let (da, db) = (a.p3 - a.p0, b.p3 - b.p0);
    let den = da.cross(db);
    if den.abs() <= 1e-12 * da.hypot() * db.hypot() || den == 0.0 {
        return None;
    }
    let w = b.p0 - a.p0;
    let (s, u) = (w.cross(db) / den, w.cross(da) / den);
    let within = |v: f64| (-1e-9..=1.0 + 1e-9).contains(&v);
    (within(s) && within(u)).then(|| (s.clamp(0.0, 1.0), u.clamp(0.0, 1.0)))
}

/// Where segments `a` and `b` cross: (t on `a`, t on `b`) pairs, halving both while their control
/// boxes meet until each piece is straight. `None` once `budget` steps are spent.
fn segment_crossings(a: &CubicBez, b: &CubicBez, budget: &mut usize) -> Option<Vec<(f64, f64)>> {
    let mut stack = vec![(0.0, 1.0, 0.0, 1.0, 0u32)];
    let mut found: Vec<(f64, f64)> = vec![];
    while let Some((a0, a1, b0, b1, depth)) = stack.pop() {
        *budget = budget.checked_sub(1)?;
        let (ca, cb) = (a.subsegment(a0..a1), b.subsegment(b0..b1));
        if !control_box(&ca).inflate(SAME, SAME).overlaps(control_box(&cb)) {
            continue;
        }
        if (flat(&ca) && flat(&cb)) || depth >= MAX_DEPTH {
            if let Some((s, u)) = chords_cross(&ca, &cb) {
                // The pieces are straight but their parameters needn't run evenly along them.
                let x = ca.p0.lerp(ca.p3, s);
                let (ta, tb) = (ca.nearest(x, 1e-9).t, cb.nearest(cb.p0.lerp(cb.p3, u), 1e-9).t);
                found.push((a0 + ta * (a1 - a0), b0 + tb * (b1 - b0)));
            }
            continue;
        }
        // Halve the larger of the pieces still curved: two curves lying along each other then
        // split along their length, not into every pair of pieces.
        let size = |c: &CubicBez| if flat(c) { -1.0 } else { control_box(c).size().to_vec2().hypot() };
        let d = depth + 1;
        if size(&ca) >= size(&cb) {
            let am = (a0 + a1) / 2.0;
            stack.extend([(a0, am, b0, b1, d), (am, a1, b0, b1, d)]);
        } else {
            let bm = (b0 + b1) / 2.0;
            stack.extend([(a0, a1, b0, bm, d), (a0, a1, bm, b1, d)]);
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    // The same crossing found by neighbouring pieces counts once.
    found.dedup_by(|x, y| a.eval(x.0).distance(a.eval(y.0)) < SAME && b.eval(x.1).distance(b.eval(y.1)) < SAME);
    Some(found)
}

/// Do segments `i` and `j` of `sp` meet at an anchor they share?
fn adjacent(sp: &SubPath, i: usize, j: usize) -> bool {
    let n = sp.segment_count();
    i.abs_diff(j) == 1 || (sp.closed && n > 1 && i.abs_diff(j) == n - 1)
}

/// Where `paths` cross one another (and, with `self_crossings`, where each crosses itself): for
/// each path, the places on it, in order. Segments that meet at an anchor they share don't count as
/// crossing there. `None` past [`MAX_CROSSING_SEGMENTS`] segments or when the curves are too tangled
/// to sort out, or when a coordinate isn't finite.
pub fn crossings(paths: &[&PathData], self_crossings: bool) -> Option<Vec<Vec<Place>>> {
    let mut segs: Vec<(usize, usize, usize, CubicBez, Rect)> = vec![];
    for (k, path) in paths.iter().enumerate() {
        for (si, sp) in path.subpaths.iter().enumerate() {
            for s in 0..sp.segment_count() {
                let c = sp.segment(s);
                if ![c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
                    return None;
                }
                segs.push((k, si, s, c, control_box(&c)));
                if segs.len() > MAX_CROSSING_SEGMENTS {
                    return None;
                }
            }
        }
    }
    let mut out: Vec<Vec<Place>> = vec![vec![]; paths.len()];
    let mut budget = MAX_STEPS;
    for (i, (ka, sa, ia, ca, ra)) in segs.iter().enumerate() {
        for (kb, sb, ib, cb, rb) in segs.iter().skip(i + 1) {
            if ka == kb
                && (!self_crossings || (sa == sb && paths.get(*ka).and_then(|p| p.subpaths.get(*sa)).is_some_and(|sp| adjacent(sp, *ia, *ib))))
            {
                continue;
            }
            if !ra.inflate(SAME, SAME).overlaps(*rb) {
                continue;
            }
            for (ta, tb) in segment_crossings(ca, cb, &mut budget)? {
                out.get_mut(*ka)?.push((*sa, *ia, ta));
                out.get_mut(*kb)?.push((*sb, *ib, tb));
            }
        }
    }
    for places in &mut out {
        places.sort_by(|x, y| (x.0, x.1).cmp(&(y.0, y.1)).then(x.2.total_cmp(&y.2)));
        places.dedup_by(|x, y| (x.0, x.1) == (y.0, y.1) && (x.2 - y.2).abs() < END);
    }
    Some(out)
}

/// `sp` with an anchor at each of `places` (segment, t), the shape unchanged → (the subpath, the
/// indexes of the anchors at those places: the new ones, and existing ones where a place falls on
/// an anchor). Places off the subpath are left out.
pub fn insert_at(sp: &SubPath, places: &[(usize, f64)]) -> (SubPath, BTreeSet<usize>) {
    let (n, segs) = (sp.anchors.len(), sp.segment_count());
    let mut inner: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    let mut at_anchor = vec![];
    for &(seg, t) in places.iter().filter(|(seg, t)| *seg < segs && t.is_finite()) {
        match t {
            _ if t <= END => at_anchor.push(seg),
            _ if t >= 1.0 - END => at_anchor.push((seg + 1) % n),
            _ => inner.entry(seg).or_default().push(t),
        }
    }
    for ts in inner.values_mut() {
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < END);
    }
    // Each split segment's pieces, cut from the original curve so their parameters stay true.
    let mut anchors = sp.anchors.clone();
    let mut added: BTreeMap<usize, Vec<Anchor>> = BTreeMap::new();
    for (&seg, ts) in &inner {
        let c = sp.segment(seg);
        let bounds: Vec<f64> = std::iter::once(0.0).chain(ts.iter().copied()).chain([1.0]).collect();
        let pieces: Vec<CubicBez> = bounds.windows(2).filter_map(|w| Some(c.subsegment(*w.first()?..*w.get(1)?))).collect();
        if sp.segment_is_line(seg) {
            added.insert(seg, pieces.iter().skip(1).map(|q| Anchor::corner(q.p0)).collect());
            continue;
        }
        let made = pieces.windows(2).filter_map(|w| match w {
            [a, b] => Some(Anchor { p: b.p0, h_in: a.p2, h_out: b.p1, kind: AnchorKind::Smooth }),
            _ => None,
        });
        added.insert(seg, made.collect());
        if let (Some(first), Some(a)) = (pieces.first(), anchors.get_mut(seg)) {
            a.h_out = first.p1;
        }
        if let (Some(last), Some(a)) = (pieces.last(), anchors.get_mut((seg + 1) % n)) {
            a.h_in = last.p2;
        }
    }
    let mut out = SubPath::new(Vec::with_capacity(n + places.len()), sp.closed);
    for (i, a) in anchors.into_iter().enumerate() {
        out.anchors.push(a);
        out.anchors.extend(added.remove(&i).unwrap_or_default());
    }
    let added_before = |j: usize| inner.range(..j).map(|(_, v)| v.len()).sum::<usize>();
    let mut marks: BTreeSet<usize> = at_anchor.into_iter().map(|j| j + added_before(j)).collect();
    for (&seg, ts) in &inner {
        let base = seg + added_before(seg);
        marks.extend((1..=ts.len()).map(|m| base + m));
    }
    (out, marks)
}
