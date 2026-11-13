//! Outlines placed to a fraction of a pixel.
//!
//! A boundary loop runs along the cracks between pixels, so on its own it is a staircase that is
//! right only to half a pixel. But an anti-aliased or slightly blurred image says where the edge
//! really is: the pixels beside it are part one colour and part the other. Each crack between an
//! inside and an outside pixel is moved along its normal by what the four pixels across it say (see
//! [`Crack`]), which gives one point per crack; a corner of the loop that is a true corner of the
//! shape (not a step of a staircase) is kept, moved with the two edges that meet there. The same
//! crack gives the same point from either side, so two regions that touch get the same points along
//! their shared edge.
//!
//! An image with hard edges (a GIF, a 1-bit scan) says nothing more than the cracks do: every crack
//! moves by the same amount. There the staircase is read the classic way: a one-pixel step between
//! two runs is a point of a sloped edge (its middle is on the edge, the corners beside it are not).

use kurbo::Vec2;
use vectorcraft_geom::Point;

use crate::contour::Loop;

/// One straight run of a loop, from one of its vertices to the next.
struct Edge {
    dir: (i32, i32),
    len: usize,
    /// Index of its first crack's shift.
    first: usize,
}

/// A crack between an inside and an outside pixel of a `w × h` image, with the column of pixels
/// across it. Read across the edge, the column says where the edge lies: a straight edge that
/// crosses it leaves in its pixels, summed, exactly as much of the inside colour as its distance
/// along the column (which is how anti-aliasing draws it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crack {
    inside: (i32, i32),
    /// From the inside pixel to the outside one.
    step: (i32, i32),
    w: usize,
    h: usize,
}

impl Crack {
    /// The crack between pixel `inside` and the pixel `step` (one pixel right or down) from it.
    pub fn new(inside: (i32, i32), step: (i32, i32), w: usize, h: usize) -> Self {
        Self { inside, step, w, h }
    }
    /// The same crack seen from its other side: the outside pixel becomes the inside one.
    pub fn flipped(&self) -> Self {
        Self { inside: (self.inside.0 + self.step.0, self.inside.1 + self.step.1), step: (-self.step.0, -self.step.1), w: self.w, h: self.h }
    }
    /// The index of the pixel `k` steps out from the inside pixel (0: the inside pixel, 1: the
    /// outside one, −1: the one behind the inside pixel), if the image has it.
    pub fn at(&self, k: i32) -> Option<usize> {
        pixel((self.inside.0 + k * self.step.0, self.inside.1 + k * self.step.1), self.w, self.h)
    }
}

fn pixel(p: (i32, i32), w: usize, h: usize) -> Option<usize> {
    let (x, y) = (usize::try_from(p.0).ok()?, usize::try_from(p.1).ok()?);
    (x < w && y < h).then(|| y * w + x)
}

/// A stretch of an outline with one neighbour along it: its points, and whether its edge is hard.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Piece {
    pub pts: Vec<Point>,
    pub hard: bool,
    /// The piece is a whole loop without a junction.
    pub closed: bool,
    /// The piece is a contour of a smooth area, not an edge (see [`low_contrast`]).
    pub contour: bool,
}

/// Whether a stretch of outline is the contour of a smooth area (where a gradient or a shaded
/// surface was split into bands of two colors) rather than an edge of the image, from what
/// `across` says of each of its cracks (see `Layers::step`): the share of the step from one color to
/// the other that the image makes from the second and third pixels on one side of the crack to those
/// on the other (near), from the fourth and fifth (far), the near change along the line between the
/// two colors, the whole near change in any direction, and the change within each side, each change
/// an RGB distance (0 to 255 a channel). An edge makes all of the step near the crack,
/// anti-aliased or not, and none further out; a smooth area makes a fraction of it near the crack
/// and a larger fraction further out, and changes within each side too.
///
/// Here, over the stretch:
///
/// - the near share is under one half;
/// - the far share is at least one and a quarter times the near one, or the near change along the
///   line is under [`FLAT`], or no crack has a far share;
/// - the sides change within themselves, by at least a thirty-second of the near change along the
///   line;
/// - and the whole near change is under twice the change along the line, or under twice [`FLAT`].
///
/// Between two flat colors that the palette put in entries further apart than they are, the near
/// share can be under one half, but nothing changes within either side. Between two colors off the
/// line between their entries, such as a red and a green traced with black and white, the shares can
/// be nothing at all, but the whole change is large. Values that are not a number are left out. The
/// means are summed in sorted order, so the stretch is decided the same from either side.
pub(crate) fn low_contrast(steps: &[[f64; 5]]) -> bool {
    let mean = |k: usize| {
        let mut v: Vec<f64> = steps.iter().map(|s| s[k]).filter(|s| !s.is_nan()).collect();
        v.sort_by(f64::total_cmp);
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    };
    let along = mean(2);
    if mean(3).is_some_and(|change| change >= 2.0 * along.unwrap_or(0.0).max(FLAT)) {
        return false;
    }
    if !mean(4).is_some_and(|within| within > 0.0 && within >= along.unwrap_or(0.0) / 32.0) {
        return false;
    }
    match (mean(0), mean(1)) {
        (Some(near), Some(far)) => near < 0.5 && (far >= 1.25 * near || along.is_some_and(|l| l < FLAT)),
        (Some(near), None) => near < 0.5,
        _ => false,
    }
}

/// A change between colors (an RGB distance, 0 to 255 a channel) under which an area is nearly
/// flat (see [`low_contrast`]).
const FLAT: f64 = 12.0;

/// What `across` says of a crack that is an edge whatever the image does there: a crack on the
/// image's border, or one that the image's pixels cannot be read across (see [`low_contrast`]).
pub(crate) const EDGE_STEP: [f64; 5] = [1.0, 1.0, f64::NAN, f64::NAN, f64::NAN];

/// What lies across a crack on the image's border: one neighbour per side, so that the image's
/// corners are junctions and stay exact.
fn border(dir: (i32, i32)) -> u32 {
    u32::MAX
        - match dir {
            (1, 0) => 0,
            (0, 1) => 1,
            (-1, 0) => 2,
            _ => 3,
        }
}

/// The outline of `l` (a loop of a `w × h` image) as points.
///
/// `across` says, for a crack, how far (in pixels, −0.5 to 0.5, positive towards the outside
/// pixel) the edge lies from it, which region is on its other side, and what the image does across
/// it between the two regions' colors (see [`low_contrast`]; the same read from either side).
///
/// The outline comes in pieces, cut at its junctions: the pixel corners where the region on the
/// other side changes (three or more regions meet there, or the image's border begins), and
/// those where two parts of a region touch by a corner only (there its outline and its
/// neighbour's part ways without the neighbour changing). `junction_at` says of a pixel corner
/// whether it is a junction of that second kind, when asked without `must`, and where the
/// junction really lies, the same for every region around it. A piece
/// is the whole edge between two regions, and both get exactly the same points for it (a crack
/// gives the same point from either side, shifts are rounded to 1/1024 px so that they agree to
/// the last bit, and whether a piece is hard is decided by the piece alone), so that fitted the
/// same way it is one curve for both. A loop without junctions is one closed piece.
pub(crate) fn outline(
    l: &Loop,
    w: usize,
    h: usize,
    across: &dyn Fn(Crack) -> (f64, u32, [f64; 5]),
    junction_at: &dyn Fn(i32, i32) -> Option<Point>,
) -> Vec<Piece> {
    let v = &l.pts;
    let n = v.len();
    let at = |i: usize| Point::new(f64::from(v[i % n].0), f64::from(v[i % n].1));
    if n < 4 {
        return vec![Piece { pts: (0..n).map(at).collect(), hard: true, closed: true, contour: false }];
    }
    let mut edges = Vec::with_capacity(n);
    // Per crack: its shift, the region across it, the edge it is on, and what the image does
    // across it between the two colors.
    let (mut shifts, mut others, mut edge_of, mut steps): (Vec<f64>, Vec<u32>, Vec<usize>, Vec<[f64; 5]>) = (vec![], vec![], vec![], vec![]);
    for i in 0..n {
        let (a, b) = (v[i], v[(i + 1) % n]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let dir = (dx.signum(), dy.signum());
        let len = dx.unsigned_abs().max(dy.unsigned_abs());
        edges.push(Edge { dir, len: len as usize, first: shifts.len() });
        for k in 0..len as i32 {
            // The crack starts at corner `s`; the inside pixel is on its right (y down).
            let (sx, sy) = (a.0 + dir.0 * k, a.1 + dir.1 * k);
            let (inside, outside) = match dir {
                (1, 0) => ((sx, sy), (sx, sy - 1)),
                (0, 1) => ((sx - 1, sy), (sx, sy)),
                (-1, 0) => ((sx - 1, sy - 1), (sx - 1, sy)),
                _ => ((sx, sy - 1), (sx - 1, sy - 1)),
            };
            let crack = Crack { inside, step: (outside.0 - inside.0, outside.1 - inside.1), w, h };
            let (s, other, step) = match crack.at(1) {
                Some(_) => across(crack),
                // The edge of the image is where it is.
                _ => (0.0, border(dir), EDGE_STEP),
            };
            shifts.push(if s.is_finite() { (s.clamp(-0.5, 0.5) * 1024.0).round() / 1024.0 } else { 0.0 });
            others.push(other);
            edge_of.push(i);
            steps.push(step);
        }
    }
    let cracks = shifts.len();
    if cracks == 0 {
        return vec![Piece { pts: (0..n).map(at).collect(), hard: true, closed: true, contour: false }];
    }
    // The pixel corner each crack starts at.
    let corner_of = |c: usize| {
        let (i, e) = (edge_of[c], &edges[edge_of[c]]);
        (v[i].0 + e.dir.0 * (c - e.first) as i32, v[i].1 + e.dir.1 * (c - e.first) as i32)
    };
    // A junction lies at the start of each crack whose neighbour is not the crack before's, and
    // where the callback says one is.
    let junctions: Vec<Option<Point>> = (0..cracks)
        .map(|c| {
            let (x, y) = corner_of(c);
            let changed = others[c] != others[(c + cracks - 1) % cracks];
            junction_at(x, y).or_else(|| changed.then(|| Point::new(f64::from(x), f64::from(y))))
        })
        .collect();
    let junction = |c: usize| junctions[c].is_some();
    let first = (0..cracks).find(|&c| junction(c));
    // A hard edge: nearly every crack of the piece moves by the same amount. (The middle of the
    // two middle shifts, so that the piece is as hard read from its other side, shifts negated.)
    let is_hard = |piece: &[f64]| {
        let mut sorted = piece.to_vec();
        sorted.sort_by(f64::total_cmp);
        let median = (sorted[sorted.len() / 2] + sorted[(sorted.len() - 1) / 2]) / 2.0;
        piece.iter().filter(|s| (*s - median).abs() > 0.03).count() * 4 < piece.len()
    };

    // Which way the loop turns at vertex `i`, and whether edge `i` is a step of a staircase: one
    // pixel long, turning one way where it starts and the other where it ends.
    // (At a junction the loop goes its own way, its neighbour another: a turn there says nothing
    // about the edge they share, and counts as none.)
    let turn = |i: usize| {
        let (a, b) = (edges[(i + n - 1) % n].dir, edges[i % n].dir);
        if junctions[edges[i % n].first].is_some() { 0 } else { (a.0 * b.1 - a.1 * b.0).signum() }
    };
    let step: Vec<bool> = (0..n).map(|i| edges[i].len == 1 && turn(i) * turn(i + 1) < 0).collect();
    let outward = |d: (i32, i32)| Vec2::new(f64::from(d.1), f64::from(-d.0));
    // The pixel at a corner of the shape lies on both of the corner's edges, and what it lacks of
    // the inside's colour is the doing of both: read as one edge's alone it dents the corner. The
    // crack of each edge on it takes the shift of the next crack along that edge instead (edges
    // of three pixels or more: a shorter one has no pixel clear of its corners).
    let shifts = {
        let mut shifts = shifts;
        for i in 0..n {
            let (b, e) = ((i + n - 1) % n, i);
            if step[b] || step[e] || turn(i) == 0 {
                continue;
            }
            let (before, edge) = (&edges[b], &edges[e]);
            // The three cracks must belong to the same shared piece. A junction
            // can split a long raw edge near this corner; the opposite region
            // then sees a shorter edge and would not make this correction.
            if before.len >= 3 && !(1..=2).any(|k| junction(before.first + before.len - k)) {
                shifts[before.first + before.len - 1] = shifts[before.first + before.len - 2];
            }
            if edge.len >= 3 && !(1..=2).any(|k| junction(edge.first + k)) {
                shifts[edge.first] = shifts[edge.first + 1];
            }
        }
        shifts
    };

    // A sharp corner whose own pixel is less than half the inside's colour goes to the outside, and
    // the loop cuts the corner in two steps (the same, turned round, where the corner pixel of a
    // notch goes to the inside). Between two edges of three pixels or more, those two steps are
    // that corner: it lies where the two edges, each moved by its shift, meet. `notch[i]` is, for
    // the first of the two steps, the corner (`Some(Some(..))`), for the second nothing
    // (`Some(None)`), for any other edge `None`.
    let mut notch: Vec<Option<Option<Point>>> = vec![None; n];
    for i in 0..n {
        let (a, s1, s2, b) = ((i + n - 1) % n, i, (i + 1) % n, (i + 2) % n);
        let free = |e: usize| junctions[edges[e].first].is_none();
        if n >= 6
            && step[s1]
            && step[s2]
            && edges[a].len >= 3
            && edges[b].len >= 3
            && edges[s1].dir == edges[b].dir
            && edges[s2].dir == edges[a].dir
            && free(s1)
            && free(s2)
            && free(b)
            // The three cracks of `a` and of `b` next to the notch must lie in this piece. An edge
            // that runs on past a junction is shorter in the loop of the region across it, which
            // turns there: that region makes no notch, and the two would get different points.
            && !(1..=2).any(|k| junction(edges[a].first + edges[a].len - k))
            && !(1..=2).any(|k| junction(edges[b].first + k))
        {
            let (ea, eb) = (&edges[a], &edges[b]);
            // Edge `a` carried on one pixel further meets edge `b` carried back one.
            let meet = at(s1) + Vec2::new(f64::from(ea.dir.0), f64::from(ea.dir.1));
            notch[s1] = Some(Some(meet + outward(ea.dir) * shifts[ea.first + ea.len - 1] + outward(eb.dir) * shifts[eb.first]));
            notch[s2] = Some(None);
        }
    }

    let mut pieces: Vec<Piece> = Vec::new();
    let start = first.unwrap_or(0);
    let mut c = 0;
    while c < cracks {
        // The cracks of one piece: up to the next junction.
        let len = (1..cracks - c).find(|&k| junction((start + c + k) % cracks)).unwrap_or(cracks - c);
        let own: Vec<f64> = (0..len).map(|k| shifts[(start + c + k) % cracks]).collect();
        let hard = is_hard(&own);
        let contour = low_contrast(&(0..len).map(|k| steps[(start + c + k) % cracks]).collect::<Vec<_>>());
        let mut pts = Vec::with_capacity(len + 2);
        for k in 0..len {
            let crack = (start + c + k) % cracks;
            let i = edge_of[crack];
            let (e, along_edge) = (&edges[i], crack - edges[i].first);
            let corner = at(i) + Vec2::new(f64::from(e.dir.0), f64::from(e.dir.1)) * along_edge as f64;
            if k == 0 && first.is_some() {
                // The junction itself: the one point every region around it has.
                pts.push(junctions[crack].unwrap_or(corner));
            } else if along_edge == 0 && !step[i] && !step[(i + n - 1) % n] {
                // A true corner: where its two edges, each moved by its own shift, meet.
                let before = &edges[(i + n - 1) % n];
                pts.push(corner + outward(before.dir) * shifts[(crack + cracks - 1) % cracks] + outward(e.dir) * shifts[crack]);
            }
            if !hard && let Some(sharp) = notch[i] {
                // A corner whose own pixel went to the other side: the corner, not the two steps.
                pts.extend(sharp);
            } else if step[i] || !hard {
                pts.push(corner + Vec2::new(f64::from(e.dir.0), f64::from(e.dir.1)) * 0.5 + outward(e.dir) * shifts[crack]);
            }
        }
        if first.is_some() {
            // It ends on the next junction: the corner the next piece starts on.
            let next = (start + c + len) % cracks;
            let (x, y) = corner_of(next);
            pts.push(junctions[next].unwrap_or(Point::new(f64::from(x), f64::from(y))));
        }
        pieces.push(Piece { pts, hard, closed: first.is_none(), contour });
        c += len;
    }
    if first.is_none() && pieces.first().is_some_and(|p| p.pts.len() < 3) {
        return vec![Piece { pts: (0..n).map(at).collect(), hard: true, closed: true, contour: false }];
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contour::trace_mask;

    fn loop_of(mask: &[bool], w: usize, h: usize) -> Loop {
        trace_mask(mask, w, h).remove(0).outer
    }

    /// No junctions but where the neighbour changes, and those on the pixel grid.
    fn grid(_: i32, _: i32) -> Option<Point> {
        None
    }

    /// The outline with one neighbour all round (the image's border too): its points, and whether
    /// it is hard.
    fn whole(l: &Loop, w: usize, h: usize, shift: &dyn Fn(Crack) -> f64) -> (Vec<Point>, bool) {
        let mut pieces = outline(l, w, h, &|c| (shift(c), 0, EDGE_STEP), &grid);
        let hard = pieces.iter().all(|p| p.hard);
        // Only the image's border makes junctions here: drop each piece's closing point.
        let split = pieces.len() > 1;
        let pts = pieces.iter_mut().flat_map(|p| p.pts.drain(..p.pts.len() - usize::from(split))).collect();
        (pts, hard)
    }

    #[test]
    fn a_hard_rectangle_keeps_its_four_corners() {
        let (w, h) = (8, 6);
        let mask: Vec<bool> = (0..w * h).map(|i| (2..6).contains(&(i % w)) && (1..5).contains(&(i / w))).collect();
        let (pts, hard) = whole(&loop_of(&mask, w, h), w, h, &|_| 0.0);
        assert!(hard);
        assert_eq!(pts, [(2.0, 1.0), (6.0, 1.0), (6.0, 5.0), (2.0, 5.0)].map(Point::from));
    }

    #[test]
    fn a_hard_staircase_becomes_the_sloped_line_through_its_steps() {
        // Below the line y = x / 4 + 2: treads four pixels long.
        let (w, h) = (16, 8);
        let mask: Vec<bool> = (0..w * h).map(|i| (i / w) as f64 + 0.5 > (i % w) as f64 / 4.0 + 2.0).collect();
        let (pts, hard) = whole(&loop_of(&mask, w, h), w, h, &|_| 0.0);
        assert!(hard);
        // Every step's middle lies on one line, a quarter of a pixel's rise per pixel.
        let steps: Vec<Point> = pts.iter().copied().filter(|p| p.y.fract() == 0.5 && p.x > 0.0 && p.x < w as f64).collect();
        assert_eq!(steps.len(), 4, "{pts:?}");
        assert!(steps.windows(2).all(|s| ((s[1].y - s[0].y) / (s[1].x - s[0].x) - 0.25).abs() < 1e-9), "{steps:?}");
    }

    #[test]
    fn a_soft_edge_moves_each_crack_by_its_shift() {
        let (w, h) = (6, 4);
        // The left half is inside; its right edge really lies 0.3 px further right.
        let mask: Vec<bool> = (0..w * h).map(|i| i % w < 3).collect();
        // (0.1 px in the lower rows: a shift that is the same all along makes a hard edge.)
        let shift = |c: Crack| match (c.at(0), c.at(1)) {
            (Some(a), Some(b)) if a % w == 2 && b % w == 3 => {
                if a / w < 2 {
                    0.3
                } else {
                    0.1
                }
            }
            _ => 0.0,
        };
        let l = loop_of(&mask, w, h);
        let (pts, _) = whole(&l, w, h, &shift);
        // Each crack comes with the column of pixels across it.
        outline(
            &l,
            w,
            h,
            &|c| {
                let (a, b) = (c.at(0).unwrap(), c.at(1).unwrap());
                assert_eq!((c.at(-1), c.at(2)), (Some(a - 1), Some(b + 1)));
                assert_eq!((c.at(-3), c.at(4)), (None, None));
                (0.0, 0, EDGE_STEP)
            },
            &grid,
        );
        // The four cracks' middles, at x = 3.3 (rounded to 1/1024); the edge ends on the image's
        // border, at junctions on the pixel grid.
        let right: Vec<&Point> = pts.iter().filter(|p| p.x > 3.05).collect();
        assert_eq!(right.len(), 4);
        assert!(right.iter().all(|p| (p.x - if p.y < 2.0 { 3.3 } else { 3.1 }).abs() < 1e-3), "{right:?}");
        assert!(pts.contains(&Point::new(3.0, 0.0)) && pts.contains(&Point::new(3.0, 4.0)));
    }

    #[test]
    fn a_stretch_is_a_contour_when_it_shows_a_small_share_of_the_step_that_grows_further_out() {
        // A smooth area: about a fifth of the step near the cracks, more further out, with noise.
        assert!(low_contrast(&[[0.1, 0.5, 4.0, 9.0, 3.0], [0.3, 0.3, 12.0, 15.0, 4.0], [0.2, 0.4, 8.0, 11.0, 2.0]]));
        assert!(low_contrast(&[[0.2, 0.26, 20.0, 24.0, 5.0], [0.2, 0.26, 20.0, 24.0, 5.0]]));
        assert!(!low_contrast(&[[0.2, 0.24, 20.0, 24.0, 5.0], [0.2, 0.24, 20.0, 24.0, 5.0]]));
        // Flat color beside flat color: the same share near the crack and further out.
        assert!(!low_contrast(&[[0.45, 0.45, 40.0, 40.0, 1.0], [0.47, 0.5, 42.0, 42.0, 2.0], [0.46, 0.44, 41.0, 41.0, 1.0]]));
        // A near change of a few levels, the same further out: a nearly flat area whose bands
        // noise divides (and turns in any direction, also within each side).
        assert!(low_contrast(&[[0.22, 0.22, 3.0, 20.0, 15.0], [0.24, 0.2, 3.5, 18.0, 12.0]]));
        assert!(!low_contrast(&[[0.22, 0.22, 12.0, 14.0, 15.0], [0.24, 0.2, 12.0, 14.0, 12.0]]));
        // Two close flat colors, the same near the crack and further out, and nothing changes
        // within either side: no contour, however small their change.
        assert!(!low_contrast(&[[0.24, 0.24, 10.4, 10.4, 0.0], [0.24, 0.24, 10.4, 10.4, 0.0]]));
        // The same for two flat colors whose difference is square to the line between their
        // entries: none of the step, and no change within the sides either.
        assert!(!low_contrast(&[[0.0, 0.0, 0.0, 14.0, 0.0]]));
        // The change within the sides must be at least a thirty-second of the change along the line.
        assert!(low_contrast(&[[0.2, 0.4, 16.0, 16.0, 0.5]]));
        assert!(!low_contrast(&[[0.2, 0.4, 16.0, 16.0, 0.49]]));
        // An edge: all of the step.
        assert!(!low_contrast(&[[1.0, 1.0, 40.0, 40.0, 0.0], [0.9, 1.0, 36.0, 37.0, 1.0]]));
        // An edge between colors off the line between the two entries: none of the step, but a
        // large change. The whole change must stay under twice the change along the line.
        assert!(!low_contrast(&[[0.0, 0.0, 0.0, 226.0, 5.0], [0.0, 0.0, 0.0, 226.0, 5.0]]));
        assert!(low_contrast(&[[0.2, 0.4, 14.0, 27.0, 4.0]]));
        assert!(!low_contrast(&[[0.2, 0.4, 14.0, 28.0, 4.0]]));
        // The whole stretch counts: one half and more near the crack is no contour.
        assert!(!low_contrast(&[[0.2, 1.0, 8.0, 10.0, 2.0], [0.9, 1.0, 36.0, 36.0, 0.0]]));
        // Without far shares (by the image's border), the near share decides, if the sides change.
        assert!(low_contrast(&[[0.3, f64::NAN, 30.0, 32.0, 6.0], [0.2, f64::NAN, 20.0, 22.0, 5.0]]));
        assert!(!low_contrast(&[[0.3, f64::NAN, 30.0, 32.0, 0.0], [0.2, f64::NAN, 20.0, 22.0, 0.0]]));
        // Values that are not a number say nothing; no share at all, or no change within the
        // sides at all, no contour.
        assert!(low_contrast(&[[f64::NAN; 5], [0.2, 0.4, 20.0, 22.0, 4.0]]));
        assert!(!low_contrast(&[]));
        assert!(!low_contrast(&[[f64::NAN, 0.2, 3.0, 5.0, 1.0]]));
        assert!(!low_contrast(&[[0.2, 0.4, 20.0, 22.0, f64::NAN]]));
    }

    #[test]
    fn a_contour_is_decided_for_the_whole_piece_and_the_same_from_either_side() {
        // Left and right halves meet along a staircase; the cracks in the upper rows show a tenth of
        // the step near them (4 levels) and three tenths further out, and change within each side,
        // those in the lower ones all of it.
        let (w, h) = (12, 10);
        let left: Vec<bool> = (0..w * h).map(|i| (i % w) as f64 + 0.5 < 4.0 + (i / w) as f64 * 0.4).collect();
        let right: Vec<bool> = left.iter().map(|b| !b).collect();
        for (low_rows, contour) in [(9, true), (3, false)] {
            // (By the upper of the crack's two pixels: the same from either side.)
            let step = |c: Crack| {
                if c.at(0).unwrap().min(c.at(1).unwrap()) / w < low_rows { [0.1, 0.3, 4.0, 4.0, 4.0] } else { [1.0, 1.0, 40.0, 40.0, 0.0] }
            };
            let a = outline(&loop_of(&left, w, h), w, h, &|c| (0.0, 1, step(c)), &grid);
            let b = outline(&loop_of(&right, w, h), w, h, &|c| (0.0, 2, step(c)), &grid);
            let inner = |pieces: &[Piece]| {
                pieces.iter().find(|p| p.pts.iter().any(|q| q.x > 0.0 && q.x < w as f64 && q.y > 0.0 && q.y < h as f64)).unwrap().clone()
            };
            assert_eq!((inner(&a).contour, inner(&b).contour), (contour, contour), "{low_rows} low rows");
            // The image's border is no contour: its cracks have nothing across them.
            assert!(
                a.iter()
                    .chain(&b)
                    .filter(|p| p.pts.iter().all(|q| q.x == 0.0 || q.y == 0.0 || q.x == w as f64 || q.y == h as f64))
                    .all(|p| !p.contour)
            );
        }
    }

    #[test]
    fn a_shift_that_is_not_a_number_moves_nothing() {
        let mask = [true, true, false, false, true, true, false, false];
        let (pts, _) = whole(&loop_of(&mask, 4, 2), 4, 2, &|_| f64::NAN);
        assert_eq!(pts.len(), 4);
        assert!(pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }

    fn check_junction_near_corner(distance: usize) {
        // The bottom region's long top run crosses a junction near its corner.
        // Its neighbour sees only the short shared part after the junction.
        let (w, h) = (12, 12);
        for turns in 0..4 {
            for palette in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
                let mut labels = vec![0u32; w * h];
                for y in 0..h {
                    for x in 0..w {
                        let label = if y < 4 && x < 5 - distance {
                            0
                        } else if y >= 4 && x < 5 {
                            2
                        } else {
                            1
                        };
                        let (mut xx, mut yy) = (x, y);
                        for _ in 0..turns {
                            (xx, yy) = (h - 1 - yy, xx);
                        }
                        labels[yy * w + xx] = palette[label];
                    }
                }
                let across = |c: Crack| {
                    let (a, b) = (c.at(0).unwrap(), c.at(1).unwrap());
                    let wave = 0.4 * ((a + b) as f64).sin();
                    (if labels[a] < labels[b] { wave } else { -wave }, labels[b], EDGE_STEP)
                };
                let pieces = |label| {
                    let mask: Vec<bool> = labels.iter().map(|&l| l == label).collect();
                    outline(&loop_of(&mask, w, h), w, h, &across, &grid)
                };
                let rotate = |mut p: Point| {
                    for _ in 0..turns {
                        p = Point::new(h as f64 - p.y, p.x);
                    }
                    p
                };
                let (start, end) = (rotate(Point::new((5 - distance) as f64, 4.0)), rotate(Point::new(5.0, 12.0)));
                let edge = |pieces: &[Piece]| {
                    pieces
                        .iter()
                        .find(|p| {
                            p.pts.first() == Some(&start) && p.pts.last() == Some(&end) || p.pts.last() == Some(&start) && p.pts.first() == Some(&end)
                        })
                        .unwrap()
                        .clone()
                };
                let (mut ours, theirs) = (edge(&pieces(palette[1])), edge(&pieces(palette[2])));
                ours.pts.reverse();
                assert_eq!(ours, theirs, "distance={distance}, turns={turns}, palette={palette:?}");
            }
        }
    }

    #[test]
    fn junction_one_pixel_before_corner_keeps_shared_edge_equal() {
        check_junction_near_corner(1);
    }

    #[test]
    fn junction_two_pixels_before_corner_keeps_shared_edge_equal() {
        check_junction_near_corner(2);
    }

    #[test]
    fn two_regions_get_the_same_points_for_their_shared_edge() {
        // Left and right halves meet along a staircase; each sees the other's shifts negated.
        let (w, h) = (12, 10);
        let left: Vec<bool> = (0..w * h).map(|i| (i % w) as f64 + 0.5 < 4.0 + (i / w) as f64 * 0.4).collect();
        let right: Vec<bool> = left.iter().map(|b| !b).collect();
        let wave = |c: Crack| 0.4 * ((c.at(0).unwrap() + c.at(1).unwrap()) as f64).sin();
        let a = outline(&loop_of(&left, w, h), w, h, &|c| (if left[c.at(0).unwrap()] { wave(c) } else { -wave(c) }, 1, EDGE_STEP), &grid);
        let b = outline(&loop_of(&right, w, h), w, h, &|c| (if left[c.at(0).unwrap()] { wave(c) } else { -wave(c) }, 2, EDGE_STEP), &grid);
        // The piece that is not on the image's border, from either side.
        let inner = |pieces: &[Piece]| {
            pieces.iter().find(|p| p.pts.iter().any(|q| q.x > 0.0 && q.x < w as f64 && q.y > 0.0 && q.y < h as f64)).unwrap().clone()
        };
        let (mut ours, theirs) = (inner(&a), inner(&b));
        ours.pts.reverse();
        assert_eq!(ours, theirs);
        assert!(ours.pts.len() > 12);
        // Each side of the image is a piece of its own: its corners are exact.
        assert!(a.iter().chain(&b).all(|p| p.pts.first().is_some_and(|q| q.x.fract() == 0.0 && q.y.fract() == 0.0)));
    }
}
