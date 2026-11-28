//! Extrude & Bevel's side unrolled flat, for the graphics mapped onto it ([`super::Facet::surface`]).
//!
//! x runs along the outline of the solid's longest contour, through its corners and curves, and y
//! runs from the front cap's edge toward the back cap's, across the bevels and the walls. Both are
//! lengths in points: x along the art's own outline (the front cap's edge, or with a bevel carved
//! into the shape the walls' outline), y along the outline's steps (the slant
//! of each bevel step and the depth of the walls), before the twist and the taper, which carry the
//! graphic with the side. x is 0 where the contour's top crosses the vertical line through the
//! middle of its bounds and grows clockwise around the outline as the front is seen on the page, so
//! a graphic reads the right way round from outside the side. y is 0 half way along the steps. The
//! side is cut at the anchor of the contour nearest half way around from x = 0.

use super::rings::signed_area;
use super::{Point, Rect};

/// The length of the outline of `ring`.
fn perimeter(ring: &[Point]) -> f64 {
    ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()).map(|(a, b)| a.distance(*b)).sum()
}

/// The ring of `rings` with the longest outline (the first of equal ones), which carries the side's
/// graphics. `None` when no ring has a finite length above 0.
pub(super) fn longest(rings: &[&[Point]]) -> Option<usize> {
    rings
        .iter()
        .map(|r| perimeter(r))
        .enumerate()
        .filter(|(_, l)| l.is_finite() && *l > 0.0)
        .fold(None, |best: Option<(usize, f64)>, (k, l)| if best.is_some_and(|(_, top)| top >= l) { best } else { Some((k, l)) })
        .map(|(k, _)| k)
}

/// The x of each end of each edge of `ring` (point `i` to point `i + 1`, the last back to the first),
/// as the module describes. `None` for a ring with no length or bounds that are not finite.
pub(super) fn along_ring(ring: &[Point]) -> Option<Vec<[f64; 2]>> {
    let n = ring.len();
    if n < 2 {
        return None;
    }
    let edges = || ring.iter().zip(ring.iter().cycle().skip(1)).map(|(a, b)| (*a, *b));
    let lengths: Vec<f64> = edges().map(|(a, b)| a.distance(b)).collect();
    let total: f64 = lengths.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    let mut starts = Vec::with_capacity(n);
    let mut at = 0.0;
    for l in &lengths {
        starts.push(at);
        at += l;
    }
    let bounds = ring.iter().fold(None, |b: Option<Rect>, p| Some(b.map_or(Rect::from_points(*p, *p), |b| b.union_pt(*p))))?;
    let middle = bounds.center().x;
    // Where x = 0: the crossing of the middle line nearest the top, as a length along the ring.
    let mut origin: Option<(f64, f64)> = None;
    let mut keep = |y: f64, s: f64| {
        if origin.is_none_or(|(top, _)| y < top) {
            origin = Some((y, s));
        }
    };
    for ((a, b), (s, l)) in edges().zip(starts.iter().zip(&lengths)) {
        if (a.x - middle) * (b.x - middle) > 0.0 {
            continue;
        }
        if a.x == b.x {
            // An edge along the middle line: its upper end.
            if a.y <= b.y { keep(a.y, *s) } else { keep(b.y, s + l) }
        } else {
            let t = (middle - a.x) / (b.x - a.x);
            keep(a.y + (b.y - a.y) * t, s + l * t);
        }
    }
    let (_, origin) = origin?;
    // The cut: the anchor nearest half way around from the origin.
    let opposite = (origin + total / 2.0).rem_euclid(total);
    let round = |d: f64| {
        let d = d.rem_euclid(total);
        d.min(total - d)
    };
    let cut = starts
        .iter()
        .copied()
        .fold(None, |best: Option<f64>, s| if best.is_some_and(|b| round(b - opposite) <= round(s - opposite)) { best } else { Some(s) })?;
    let from_cut = (origin - cut).rem_euclid(total);
    // The ring runs clockwise on the page (y down) when it encloses positive area: x grows along it.
    let sign = if signed_area(ring) >= 0.0 { 1.0 } else { -1.0 };
    Some(
        starts
            .iter()
            .zip(&lengths)
            .map(|(s, l)| {
                let a = (s - cut).rem_euclid(total) - from_cut;
                [a * sign, (a + l) * sign]
            })
            .collect(),
    )
}

/// The y of each of an extrusion's `steps`, each its inset (points) and its depth fraction (0 front,
/// 1 back) of a solid `depth` points deep: the length along the steps from their middle, growing
/// toward the back.
pub(super) fn across_steps(steps: &[(f64, f64)], depth: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(steps.len());
    let mut at = 0.0;
    for (k, (inset, t)) in steps.iter().enumerate() {
        if let Some((i0, t0)) = k.checked_sub(1).and_then(|k| steps.get(k)) {
            at += (inset - i0).hypot(depth * (t - t0));
        }
        out.push(at);
    }
    let middle = at / 2.0;
    out.into_iter().map(|y| y - middle).collect()
}

/// The size of Extrude & Bevel's side unrolled, around the longest of `rings` and across the steps
/// `steps` (each inset and depth fraction) of a solid `depth` points deep: the length around it,
/// the length across it, and the width about x = 0 that does not reach the side's cut (twice the
/// distance from x = 0 to the nearer end of the cut side). `None` when the side has no length
/// either way.
pub(super) fn size(rings: &[&[Point]], steps: &[(f64, f64)], depth: f64) -> Option<(f64, f64, f64)> {
    let ring = rings.get(longest(rings)?)?;
    let across = across_steps(steps, depth);
    let height = across.last().zip(across.first()).map_or(0.0, |(b, a)| b - a);
    let width = perimeter(ring);
    let xs = along_ring(ring)?;
    let (lo, hi) = xs.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
    let span = 2.0 * (-lo).min(hi);
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0).then_some((width, height, span.clamp(0.0, width)))
}

#[cfg(test)]
#[path = "tests_side.rs"]
mod tests;
