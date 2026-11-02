//! Artwork Cleanup's path repairs, each on one subpath and returning how much it changed (so a
//! check is the repair run on a copy): collinear anchors, duplicate anchors, tiny handles and
//! nearly closed paths; [`near_point_groups`] finds anchors to snap together, and [`rejoin`] welds
//! open pieces whose ends meet back into paths.

use std::collections::HashMap;

use kurbo::{Point, Vec2};
use vectorcraft_geom::{Anchor, AnchorKind, EPS, SubPath};

use crate::fit::{end_tangent, start_tangent};

/// The fewest anchors a subpath keeps: two when open, three when closed.
fn min_anchors(sp: &SubPath) -> usize {
    if sp.closed { 3 } else { 2 }
}

/// `a`'s handle offset `h - a.p` moved onto `p`.
fn moved(p: Point, a_p: Point, h: Point) -> Point {
    p + (h - a_p)
}

/// Remove the corner anchors (no handles, straight segments both sides) where a straight run goes
/// on in the same direction: the sine of the angle it turns there is at most `tolerance`. Open ends
/// stay, and a subpath keeps two anchors (three when closed) → how many went.
pub fn remove_collinear_anchors(sp: &mut SubPath, tolerance: f64) -> usize {
    let mut removed = 0;
    let mut i = 0;
    while i < sp.anchors.len() && sp.anchors.len() > min_anchors(sp) {
        let n = sp.anchors.len();
        let inner = sp.closed || (i > 0 && i + 1 < n);
        let (Some(prev), Some(cur), Some(next)) = (sp.anchors.get((i + n - 1) % n), sp.anchors.get(i), sp.anchors.get((i + 1) % n)) else { break };
        let straight = !prev.has_out() && !cur.has_in() && !cur.has_out() && !next.has_in();
        let (a, b) = (cur.p - prev.p, next.p - cur.p);
        let len = a.hypot() * b.hypot();
        if inner && straight && len > EPS && a.dot(b) > 0.0 && a.cross(b).abs() / len <= tolerance {
            sp.anchors.remove(i);
            removed += 1;
        } else {
            i += 1;
        }
    }
    removed
}

/// Merge each anchor lying within `distance` of the one before it (and a closed subpath's last
/// anchor within `distance` of its first) into that one, which keeps its in handle and takes the
/// merged anchor's out handle → how many went.
pub fn merge_duplicate_anchors(sp: &mut SubPath, distance: f64) -> usize {
    let before = sp.anchors.len();
    let mut out: Vec<Anchor> = Vec::with_capacity(before);
    for a in sp.anchors.drain(..) {
        match out.last_mut() {
            Some(l) if l.p.distance(a.p) <= distance => *l = Anchor::with_handles(l.p, l.h_in, moved(l.p, a.p, a.h_out)),
            _ => out.push(a),
        }
    }
    if sp.closed
        && out.len() > 1
        && let (Some(first), Some(last)) = (out.first(), out.last())
        && first.p.distance(last.p) <= distance
        && let Some(last) = out.pop()
        && let Some(f) = out.first_mut()
    {
        *f = Anchor::with_handles(f.p, moved(f.p, last.p, last.h_in), f.h_out);
    }
    sp.anchors = out;
    before - sp.anchors.len()
}

/// Retract the handles shorter than `length` (but not already on their anchor); an anchor that
/// loses a handle becomes a corner → how many handles were retracted.
pub fn retract_tiny_handles(sp: &mut SubPath, length: f64) -> usize {
    let mut count = 0;
    for a in &mut sp.anchors {
        for out in [false, true] {
            let h = if out { &mut a.h_out } else { &mut a.h_in };
            let l = h.distance(a.p);
            if l > EPS && l < length {
                *h = a.p;
                a.kind = AnchorKind::Corner;
                count += 1;
            }
        }
    }
    count
}

/// Close an open subpath of three or more anchors whose ends lie within `distance` of each other:
/// the last anchor merges into the first, which takes its in handle → whether it closed.
pub fn close_nearly_closed(sp: &mut SubPath, distance: f64) -> bool {
    let ends_meet = sp.anchors.first().zip(sp.anchors.last()).is_some_and(|(f, l)| f.p.distance(l.p) <= distance);
    if sp.closed || sp.anchors.len() < 3 || !ends_meet {
        return false;
    }
    let Some(last) = sp.anchors.pop() else { return false };
    if let Some(first) = sp.anchors.first_mut() {
        *first = Anchor::with_handles(first.p, moved(first.p, last.p, last.h_in), first.h_out);
    }
    sp.closed = true;
    true
}

/// Groups of `points` to snap together: points within `distance` of one another (and so on, one
/// to the next) that aren't all at the same place, skipping the pairs `linked` names (such as
/// neighbouring anchors of one path). Each group lists its points in order; snapping them all to
/// the first one makes them coincide. Non-finite points and a distance that isn't a positive number
/// find nothing.
pub fn near_point_groups(points: &[Point], distance: f64, linked: impl Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
    if !(distance > 0.0 && distance.is_finite()) {
        return vec![];
    }
    let cell = |p: Point| ((p.x / distance).floor() as i64, (p.y / distance).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate().filter(|(_, p)| p.is_finite()) {
        grid.entry(cell(*p)).or_default().push(i);
    }
    let mut parent: Vec<usize> = (0..points.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while let Some(&p) = parent.get(i).filter(|&&p| p != i) {
            let up = parent.get(p).copied().unwrap_or(p);
            if let Some(slot) = parent.get_mut(i) {
                *slot = up;
            }
            i = p;
        }
        i
    }
    for (&(cx, cy), members) in &grid {
        for (dx, dy) in [(0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let Some(others) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) else { continue };
            for &i in members {
                for &j in others.iter().filter(|&&j| (dx, dy) != (0, 0) || j > i) {
                    let (Some(a), Some(b)) = (points.get(i), points.get(j)) else { continue };
                    let d = a.distance(*b);
                    if d <= distance && d > 0.0 && !linked(i.min(j), i.max(j)) {
                        let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                        if let Some(slot) = parent.get_mut(ri.max(rj)) {
                            *slot = ri.min(rj);
                        }
                    }
                }
            }
        }
    }
    // Only pairs that aren't at the same place were joined, so no group is all at one place.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..points.len() {
        groups.entry(root(&mut parent, i)).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort_unstable();
    out
}

/// Open pieces welded into one path by [`rejoin`]: `members` index the pieces in the order they
/// run along `path`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejoined {
    pub members: Vec<usize>,
    pub path: SubPath,
}

/// One end of a piece: (piece, true for its last anchor).
type End = (usize, bool);

/// The direction pointing out of the piece at `end`.
fn outward(sp: &SubPath, last: bool) -> Option<Vec2> {
    let n = sp.segment_count();
    Some(match last {
        true => end_tangent(&sp.segment(n.checked_sub(1)?)),
        false => -start_tangent(&sp.segment(0)),
    })
}

/// Weld the open `pieces` whose ends lie within `gap` of each other back into paths: an end meets
/// at most one other, and where an end could meet several, it meets the one the path turns least
/// to go on into (then the nearest). A piece may run either way; a chain whose ends meet closes.
/// Each junction keeps the earlier piece's anchor and the later piece's out handle. Pieces that are
/// closed, have fewer than two anchors or a non-finite coordinate stay out. → the chains of two or
/// more pieces, and the single pieces that closed.
pub fn rejoin(pieces: &[SubPath], gap: f64) -> Vec<Rejoined> {
    if !(gap >= 0.0 && gap.is_finite()) {
        return vec![];
    }
    let usable = |sp: &&SubPath| {
        !sp.closed && sp.anchors.len() >= 2 && sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite())
    };
    // Each usable end: where it is and the way out of its piece there.
    let ends: Vec<(End, Point, Vec2)> = pieces
        .iter()
        .enumerate()
        .filter(|(_, sp)| usable(sp))
        .flat_map(|(i, sp)| [(i, sp, false), (i, sp, true)])
        .filter_map(|(i, sp, last)| {
            let a = if last { sp.anchors.last() } else { sp.anchors.first() }?;
            Some(((i, last), a.p, outward(sp, last)?))
        })
        .collect();
    let points: Vec<Point> = ends.iter().map(|e| e.1).collect();
    // Every pair of ends within the gap, by the turn across the junction.
    let mut pairs: Vec<(f64, f64, End, End)> = vec![];
    for (a, b) in candidate_pairs(&points, gap.max(1e-9)) {
        let (Some(&(ea, pa, oa)), Some(&(eb, pb, ob))) = (ends.get(a), ends.get(b)) else { continue };
        // A piece's own two ends meet only when closing it leaves a shape (three anchors).
        if ea.0 == eb.0 && pieces.get(ea.0).is_none_or(|sp| sp.anchors.len() < 3) {
            continue;
        }
        let d = pa.distance(pb);
        if d <= gap {
            pairs.push((oa.dot(-ob).clamp(-1.0, 1.0).acos(), d, ea, eb));
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let mut link: HashMap<End, End> = HashMap::new();
    for (_, _, a, b) in pairs {
        if !link.contains_key(&a) && !link.contains_key(&b) {
            link.insert(a, b);
            link.insert(b, a);
        }
    }
    chains(pieces, &link)
}

/// Pairs (a < b) of `points` that may lie within `gap`: those in neighbouring grid cells.
fn candidate_pairs(points: &[Point], gap: f64) -> Vec<(usize, usize)> {
    let cell = |p: Point| ((p.x / gap).floor() as i64, (p.y / gap).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        grid.entry(cell(*p)).or_default().push(i);
    }
    let mut out = vec![];
    for (i, p) in points.iter().enumerate() {
        let (cx, cy) = cell(*p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(others) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) else { continue };
                out.extend(others.iter().filter(|&&j| j > i).map(|&j| (i, j)));
            }
        }
    }
    out
}

/// Walk the links between piece ends into chains.
fn chains(pieces: &[SubPath], link: &HashMap<End, End>) -> Vec<Rejoined> {
    let mut seen = vec![false; pieces.len()];
    let mut out = vec![];
    for start in 0..pieces.len() {
        if seen.get(start) != Some(&false) || !(link.contains_key(&(start, false)) || link.contains_key(&(start, true))) {
            continue;
        }
        // A member is (piece, reversed): it comes in by end (piece, reversed) and leaves by
        // (piece, !reversed). Back up to the chain's first piece, or round to `start` in a loop.
        let mut first = (start, false);
        let mut closed = false;
        for _ in 0..pieces.len() {
            let Some(&(p, at_last)) = link.get(&first) else { break };
            // The piece before leaves by its end `at_last`.
            let prev = (p, !at_last);
            if prev.0 == start {
                closed = true;
                break;
            }
            first = prev;
        }
        let members = walk(link, first, closed, &mut seen);
        if let Some(path) = assemble(pieces, &members, closed) {
            out.push(Rejoined { members: members.iter().map(|m| m.0).collect(), path });
        }
    }
    out
}

/// The pieces of the chain that begins with `first` (piece, reversed), in order.
fn walk(link: &HashMap<End, End>, first: (usize, bool), closed: bool, seen: &mut [bool]) -> Vec<(usize, bool)> {
    let mut members = vec![];
    let mut cur = first;
    loop {
        match seen.get_mut(cur.0) {
            Some(s) if !*s => *s = true,
            _ => break,
        }
        members.push(cur);
        // A piece run forwards leaves by its last anchor.
        let Some(&(p, at_last)) = link.get(&(cur.0, !cur.1)) else { break };
        let next = (p, at_last);
        if closed && next.0 == first.0 {
            break;
        }
        cur = next;
    }
    members
}

/// The anchors of `pieces` along the chain `members`, each piece's start merged into the end before it.
fn assemble(pieces: &[SubPath], members: &[(usize, bool)], closed: bool) -> Option<SubPath> {
    if members.len() < 2 && !closed {
        return None;
    }
    let mut anchors: Vec<Anchor> = vec![];
    for &(i, reversed) in members {
        let mut sp = pieces.get(i)?.clone();
        if reversed {
            sp.reverse();
        }
        let mut rest = sp.anchors.into_iter();
        match (anchors.last_mut(), rest.next()) {
            (Some(l), Some(f)) => *l = Anchor::with_handles(l.p, l.h_in, moved(l.p, f.p, f.h_out)),
            (None, Some(f)) => anchors.push(f),
            _ => {}
        }
        anchors.extend(rest);
    }
    let mut sp = SubPath::new(anchors, false);
    if closed && !close_nearly_closed(&mut sp, f64::INFINITY) {
        sp.closed = sp.anchors.len() >= 2;
    }
    Some(sp)
}

#[cfg(test)]
mod tests;
