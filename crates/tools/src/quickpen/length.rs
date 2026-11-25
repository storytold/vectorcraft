//! The line or arc at an end of an open path, and that line or arc made longer or shorter: what the
//! Quick Pen's length handles drag and `path.quickPenLength` sets.
//!
//! The piece at an end is the path's last segment when it is straight, or the run of segments
//! before the end that lie on one circle and turn the same way (an arc drawn as several cubic
//! segments of at most 90° each). A line keeps its other point and its direction; an arc keeps its
//! other point, its circle and the way it turns, and its sweep grows or shrinks.

use std::f64::consts::{FRAC_PI_2, PI};

use vectorcraft_geom::{Anchor, Point, SubPath, Vec2};

use super::{MAX_TURN, rot, unit};

/// The shortest length a line or arc can be set to (points).
pub const MIN_LENGTH: f64 = 0.001;
/// The longest a line can be set to (points).
pub const MAX_LINE_LENGTH: f64 = 1e6;
/// How close, relative to the radius, the points and handles of a segment must be to a circular
/// arc's for the segment to count as one.
const ARC_TOL: f64 = 1e-6;
/// How far, relative to the segment's length, a straight segment's handles may lie off its line.
const LINE_TOL: f64 = 1e-9;

/// What the piece at an end is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EndShape {
    Line,
    /// An arc around `center` of `radius`, turning through `sweep` radians from the piece's other
    /// point to the end (positive as [`rot`] turns).
    Arc {
        center: Point,
        radius: f64,
        sweep: f64,
    },
}

/// The line or arc at one end of an open subpath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndPiece {
    pub shape: EndShape,
    /// The point the piece leaves from, which stays where it is.
    pub fixed: Point,
    /// The end of the path.
    pub end: Point,
    /// The direction the path leaves the end in, away from the rest of the path.
    pub out: Vec2,
    /// The piece's length along the line or the arc.
    pub length: f64,
}

impl EndPiece {
    /// The longest length the piece can be set to: for an arc, a sweep of 359°.
    pub fn max_length(&self) -> f64 {
        match self.shape {
            EndShape::Line => MAX_LINE_LENGTH,
            EndShape::Arc { radius, .. } => MAX_TURN.to_radians() * radius,
        }
    }

    /// The length the piece takes when its end is dragged to `p`: for a line, how far along its
    /// direction `p` lies; for an arc, the arc to the angle `p` lies at around its center, taken
    /// nearest to `near` (the length before), so a drag goes on past a full half turn. Kept within
    /// [`MIN_LENGTH`] and [`Self::max_length`].
    pub fn length_toward(&self, p: Point, near: f64) -> f64 {
        let l = match self.shape {
            EndShape::Line => {
                let Some(u) = unit(self.end - self.fixed) else { return self.length };
                u.dot(p - self.fixed)
            }
            EndShape::Arc { center, radius, sweep } => {
                let (a, b) = (self.fixed - center, p - center);
                if b.hypot() <= 1e-12 || radius <= 0.0 {
                    return near;
                }
                // The angle from the fixed point to `p`, in the way the arc turns, 0 to 2π.
                let phi = a.cross(b).atan2(a.dot(b)) * sweep.signum();
                let phi = if phi < 0.0 { phi + 2.0 * PI } else { phi };
                let at = near / radius;
                let turns = ((at - phi) / (2.0 * PI)).round();
                (phi + turns * 2.0 * PI) * radius
            }
        };
        l.clamp(MIN_LENGTH, self.max_length())
    }

    /// The length the piece takes when its handle, pressed at `press`, is dragged to `p`: the end
    /// moves with the pointer and stays where it was relative to it. On a line the end keeps the
    /// offset the press had from it; on an arc the offset turns with the pointer around the center,
    /// so the end stays the same angle behind the pointer at any radius and zoom.
    pub fn length_dragged(&self, press: Point, p: Point, near: f64) -> f64 {
        let to = match self.shape {
            EndShape::Line => p - (press - self.end),
            EndShape::Arc { center, .. } => {
                let (a, b) = (press - center, self.end - center);
                center + rot(p - center, a.cross(b).atan2(a.dot(b)))
            }
        };
        self.length_toward(to, near)
    }
}

/// `sp`, or a copy reversed when `at_start`, so the end in question is the last point.
fn oriented(sp: &SubPath, at_start: bool) -> SubPath {
    let mut o = sp.clone();
    if at_start {
        o.reverse();
    }
    o
}

/// The anchor `i` places from the end in question of `sp` (0 is that end), as it would be in `sp`
/// reversed when `at_start`, read without copying the subpath.
fn from_end(sp: &SubPath, at_start: bool, i: usize) -> Option<Anchor> {
    if at_start { sp.anchors.get(i).map(Anchor::reversed) } else { sp.anchors.get(sp.anchors.len().checked_sub(i + 1)?).copied() }
}

/// Does the cubic from `a` to `b` run straight along its chord?
fn straight(a: &Anchor, b: &Anchor) -> bool {
    let c = b.p - a.p;
    let l = c.hypot();
    if l <= 1e-9 || !l.is_finite() {
        return false;
    }
    let u = c / l;
    let (h1, h2) = (a.h_out - a.p, b.p - b.h_in);
    let along = |h: Vec2| u.cross(h).abs() <= LINE_TOL * l && (-LINE_TOL * l..=l * (1.0 + LINE_TOL)).contains(&u.dot(h));
    along(h1) && along(h2)
}

/// The circle and signed turn of the cubic from `a` to `b` when it is a circular arc as the Quick
/// Pen draws one (handles along the tangents, each 4/3·tan(turn/4) of the radius long).
fn arc_of(a: &Anchor, b: &Anchor) -> Option<(Point, f64, f64)> {
    let (d0, d1) = (a.h_out - a.p, b.p - b.h_in);
    let (t0, t1) = (unit(d0)?, unit(d1)?);
    let turn = t0.cross(t1).atan2(t0.dot(t1));
    if turn.abs() <= 1e-9 || turn.abs() > FRAC_PI_2 + 1e-6 {
        return None;
    }
    let chord = (b.p - a.p).hypot();
    let r = chord / (2.0 * (turn.abs() / 2.0).sin());
    // An arc whose 359° is shorter than the shortest length has no length to set.
    if !r.is_finite() || MAX_TURN.to_radians() * r < MIN_LENGTH {
        return None;
    }
    let center = a.p + rot(t0, FRAC_PI_2 * turn.signum()) * r;
    let k = 4.0 / 3.0 * (turn.abs() / 4.0).tan() * r;
    let tol = ARC_TOL * r.max(1.0);
    // The end lies where the turn takes the start around the circle, not only at the right distance.
    let on =
        (center + rot(a.p - center, turn)).distance(b.p) <= tol && (d0.hypot() - k).abs() <= tol * 100.0 && (d1.hypot() - k).abs() <= tol * 100.0;
    on.then_some((center, r, turn))
}

/// The line or arc at the end (`at_start`: the start) of the open subpath `sp`, or none for a
/// closed subpath, one of fewer than two points, or an end whose last segment is neither straight
/// nor a circular arc.
pub fn end_piece(sp: &SubPath, at_start: bool) -> Option<EndPiece> {
    if sp.closed {
        return None;
    }
    last_piece(sp, at_start).map(|(piece, _)| piece)
}

/// The line or arc that ends at the end (`at_start`: the start) of `sp`, with how many points back
/// from that end the point it leaves from is. It reads only the points of the piece.
fn last_piece(sp: &SubPath, at_start: bool) -> Option<(EndPiece, usize)> {
    let at = |i: usize| from_end(sp, at_start, i);
    let last = at(0)?;
    let prev = at(1)?;
    if straight(&prev, &last) {
        let out = unit(last.p - prev.p)?;
        return Some((EndPiece { shape: EndShape::Line, fixed: prev.p, end: last.p, out, length: prev.p.distance(last.p) }, 1));
    }
    let (center, radius, mut sweep) = arc_of(&prev, &last)?;
    let mut back = 1;
    while let (Some(a), Some(b)) = (at(back + 1), at(back)) {
        let Some((c, r, t)) = arc_of(&a, &b) else { break };
        let tol = ARC_TOL * radius.max(1.0);
        if c.distance(center) > tol || (r - radius).abs() > tol || t.signum() != sweep.signum() || (sweep + t).abs() >= 2.0 * PI - 1e-9 {
            break;
        }
        sweep += t;
        back += 1;
    }
    let fixed = at(back)?.p;
    let out = unit(last.p - last.h_in)?;
    Some((EndPiece { shape: EndShape::Arc { center, radius, sweep }, fixed, end: last.p, out, length: sweep.abs() * radius }, back))
}

/// `sp` with the line or arc at the end (`at_start`: the start) made `length` long, or none when
/// that end is not a line or an arc ([`end_piece`]) or `length` is outside [`MIN_LENGTH`] to the
/// piece's [`EndPiece::max_length`]. A line keeps its other point and its direction, and its handles
/// scale with it. An arc keeps its other point, its circle and the way it turns, drawn again as
/// segments of at most 90° each; the point it leaves from keeps its kind and incoming handle.
pub fn with_length(sp: &SubPath, at_start: bool, length: f64) -> Option<SubPath> {
    if sp.closed {
        return None;
    }
    let (piece, back) = last_piece(sp, at_start)?;
    if !(MIN_LENGTH..=piece.max_length()).contains(&length) {
        return None;
    }
    let mut o = oriented(sp, at_start);
    let n = o.anchors.len();
    let from = n.checked_sub(back + 1)?;
    let last = *o.anchors.get(n - 1)?;
    match piece.shape {
        EndShape::Line => {
            let ratio = length / piece.length;
            let to = piece.fixed + piece.out * length;
            let prev = o.anchors.get_mut(n - 2)?;
            prev.h_out = prev.p + (prev.h_out - prev.p) * ratio;
            let end = o.anchors.get_mut(n - 1)?;
            *end = Anchor { p: to, h_in: to + (last.h_in - last.p) * ratio, h_out: to + (last.h_out - last.p), kind: last.kind };
        }
        EndShape::Arc { center, radius, sweep } => {
            let s = length / radius * sweep.signum();
            let start = *o.anchors.get(from)?;
            let t0 = unit(start.h_out - start.p)?;
            let segs = ((s.abs() / FRAC_PI_2) - 1e-9).ceil().max(1.0) as usize;
            let step = s / segs as f64;
            let k = 4.0 / 3.0 * (step.abs() / 4.0).tan() * radius;
            o.anchors.truncate(from + 1);
            if let Some(a) = o.anchors.last_mut() {
                a.h_out = a.p + t0 * k;
            }
            let r0 = start.p - center;
            for i in 1..=segs {
                let turned = step * i as f64;
                let q = center + rot(r0, turned);
                let t = rot(t0, turned);
                let mut an = Anchor::with_handles(q, q - t * k, q + t * k);
                if i == segs {
                    an.h_out = q + rot(last.h_out - last.p, s - sweep);
                    an.kind = last.kind;
                }
                o.anchors.push(an);
            }
        }
    }
    if at_start {
        o.reverse();
    }
    Some(o)
}
