//! The outline a Blob Brush or Eraser drag shows while it is dragged: the boundary of the area its
//! tip covers, built a bounded amount at a time.
//!
//! The drag is cut into pieces of up to [`PIECE_SEGMENTS`] segments (one segment each when the
//! Size varies with pressure). A call outlines again, in the space where the tip is round, only the
//! pieces it added segments to. A piece's outline then loses the parts that lie inside the area
//! of another piece, found through a grid of squares that lists the segments near each square, so
//! the pieces show as one outline. One call takes at most [`POINTS_PER_CALL`] new points and does at most
//! [`WORK_PER_CALL`] units of work, plus the one step it was in; what is left waits for the next
//! call ([`AreaPreview::pending`]). Until then a piece shows the outline it last finished, and the
//! points not yet taken show as a thin line. The settled pieces show as paths of up to
//! [`BLOCK_PIECES`] pieces each, and a call joins again only the paths whose pieces changed, at
//! most [`JOIN_PIECES_PER_CALL`] pieces' worth. The grid holds at most [`LISTINGS`] listings.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use vectorcraft_geom::{Affine, BezPath, Line, ParamCurve, PathData, PathEl, PathSeg, Point, SubPath};
use vectorcraft_pathops as po;
use vectorcraft_pathops::{Cap, Join};

use super::super::{MIN_BRUSH_SIZE, blob_tip, eraser_tip, pressure_size};

/// The most segments of a drag that one piece of its outline holds, when the Size does not vary
/// with pressure (a piece then holds one segment).
pub(super) const PIECE_SEGMENTS: usize = 32;
/// The most points of a drag one call takes.
pub(super) const POINTS_PER_CALL: usize = 256;
/// The most work one call does: one unit for each square of the grid a segment is listed in, and
/// for each segment of the drag a piece of an outline is checked against. A call stops before the
/// next point or the next segment of an outline once it has done this much.
pub(super) const WORK_PER_CALL: usize = 200_000;
/// The most segments one square of the grid lists. A segment past them is not listed there, and
/// the outlines inside its area there still show.
const SQUARE_SEGMENTS: usize = 128;
/// The most points, half a square apart, at which a segment of the drag is listed in the squares
/// around them. A longer segment is kept in a short list of its own ([`WIDE_SEGMENTS`]).
pub(super) const SEGMENT_SAMPLES: usize = 32;
/// The most listings all squares of the grid hold together: past them a segment is listed in no
/// square, and the outlines inside its area still show.
pub(super) const LISTINGS: usize = 4_000_000;
/// The most squares, along a side of an outline, whose segments the side is checked against; a
/// longer side is checked against the squares at its ends.
const SIDE_SQUARES: usize = 128;
/// The most pieces one joined path of the outline holds.
pub(super) const BLOCK_PIECES: usize = 64;
/// The most pieces one call joins into those paths; the others show the path last joined.
pub(super) const JOIN_PIECES_PER_CALL: usize = 2048;
/// The most segments the list of long segments holds; the outlines inside the area of a long
/// segment past them still show.
const WIDE_SEGMENTS: usize = 256;
/// The most points along a long segment at which the pieces near it are looked for.
const WIDE_SAMPLES: usize = 128;
/// The most points the thin line over the points not yet taken has.
const GUIDE_POINTS: usize = 256;
/// The most pieces a curve of an outline is checked in, each at its middle.
const CURVE_PIECES: usize = 32;
/// How far inside another piece's area, as a share of its radius, a piece of an outline must be
/// to be left out: an outline along the edge of another area still shows.
const INSIDE: f64 = 5e-3;

/// What the outline depends on besides the drag's points: a change starts it again.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct AreaKey {
    /// The Blob Brush's tip (else the Eraser's).
    pub blob: bool,
    pub size: f64,
    pub variation: f64,
    pub angle: f64,
    pub roundness: f64,
    /// Each segment is as wide as the pen pressed there.
    pub pressure: bool,
}

/// The work of the last call.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct AreaWork {
    /// Points taken.
    pub points: usize,
    /// Points of the drag outlined.
    pub outlined: usize,
    /// Units of work ([`WORK_PER_CALL`]).
    pub work: usize,
    /// Pieces joined into the paths shown ([`JOIN_PIECES_PER_CALL`]).
    pub joined: usize,
}

/// The paths of the outline to show ([`AreaPreview::paths`]).
pub(super) struct AreaPaths {
    /// The settled pieces' outlines, joined [`BLOCK_PIECES`] at a time: a block that did not change
    /// is the same shared path as in the call before.
    pub settled: Vec<Arc<BezPath>>,
    /// The growing last piece's outline and the thin line over the points not taken yet, drawn
    /// over the settled ones.
    pub live: Vec<BezPath>,
}

/// The joined outline of up to [`BLOCK_PIECES`] settled pieces.
#[derive(Default)]
struct Block {
    /// Shared with the paths [`AreaPreview::paths`] returned: a new one only when the block is
    /// joined again.
    path: Arc<BezPath>,
    /// A piece in it changed since `path` was joined.
    stale: bool,
}

/// A piece of the drag: segments `from..to` (segment `j` joins points `j` and `j + 1`).
struct Piece {
    from: usize,
    to: usize,
    /// Its outline where the tip is round, segment by segment: the segment, whether it starts a
    /// contour and whether it ends a closed one.
    outline: Vec<(PathSeg, bool, bool)>,
    /// The part of its outline that shows, on the page.
    shown: BezPath,
}

/// A piece whose outline is being checked, up to segment `at` of it.
struct Check {
    piece: usize,
    at: usize,
    out: BezPath,
    pen: Option<Point>,
    /// Every segment of the current contour shows whole.
    whole: bool,
}

pub(super) struct AreaPreview {
    key: AreaKey,
    first: Point,
    squash: Affine,
    stretch: Affine,
    /// The side of the grid's squares, where the tip is round.
    side: f64,
    /// The points taken, where the tip is round.
    sq: Vec<Point>,
    /// The last point taken, on the page.
    last: Point,
    /// The tip's radius along each segment, where the tip is round.
    radius: Vec<f64>,
    pieces: Vec<Piece>,
    squares: HashMap<(i64, i64), Vec<u32>>,
    /// How many listings the squares hold ([`LISTINGS`]).
    listed: usize,
    wide: Vec<usize>,
    /// The pieces whose outline is to be checked again.
    dirty: BTreeSet<usize>,
    check: Option<Check>,
    /// The outlines that show of every piece but a growing last one, joined [`BLOCK_PIECES`] at a
    /// time.
    blocks: Vec<Block>,
    pub work: AreaWork,
}

impl AreaPreview {
    /// The outline of a drag that starts at `first`.
    pub(super) fn new(key: AreaKey, first: Point) -> Self {
        let stretch = if key.blob { blob_tip(key.angle, key.roundness) } else { eraser_tip(key.angle, key.roundness) };
        let widest = if key.pressure { pressure_size(key.size, key.variation, 1.0) } else { key.size };
        Self {
            key,
            first,
            squash: stretch.inverse(),
            stretch,
            side: widest.max(MIN_BRUSH_SIZE),
            sq: vec![],
            last: first,
            radius: vec![],
            pieces: vec![],
            squares: HashMap::new(),
            listed: 0,
            wide: vec![],
            dirty: BTreeSet::new(),
            check: None,
            blocks: vec![],
            work: AreaWork::default(),
        }
    }

    /// Is this the outline of the drag `pts` with `key`: the same start, and the points it took
    /// still the drag's?
    pub(super) fn fits(&self, key: AreaKey, pts: &[Point]) -> bool {
        self.key == key
            && pts.first() == Some(&self.first)
            && self.sq.len() <= pts.len()
            && self.sq.len().checked_sub(1).and_then(|i| pts.get(i)) == Some(&self.last)
    }

    /// Is part of the drag `pts` not shown yet as it should be?
    pub(super) fn pending(&self, pts: &[Point]) -> bool {
        self.sq.len() < pts.len() || !self.dirty.is_empty() || self.check.is_some() || self.blocks.iter().any(|b| b.stale)
    }

    /// How many listings the grid's squares hold.
    #[cfg(test)]
    pub(super) fn listed(&self) -> usize {
        self.listed
    }

    /// Mark the joined path that holds settled piece `k` to be joined again.
    fn mark(&mut self, k: usize) {
        let b = k / BLOCK_PIECES;
        if self.blocks.len() <= b {
            self.blocks.resize_with(b + 1, Block::default);
        }
        self.blocks[b].stale = true;
    }

    /// Take the points of the drag `pts` added since the last call and check outlines, within this
    /// call's limits. `pressures` holds the pen pressure at each point when the Size varies with it.
    pub(super) fn step(&mut self, pts: &[Point], pressures: Option<&[f32]>) {
        self.work = AreaWork::default();
        let mut changed = BTreeSet::new();
        while self.sq.len() < pts.len() && self.work.points < POINTS_PER_CALL && self.work.work < WORK_PER_CALL {
            let i = self.sq.len();
            self.sq.push(self.squash * pts[i]);
            self.last = pts[i];
            self.work.points += 1;
            let Some(j) = i.checked_sub(1) else { continue };
            let size = match pressures.filter(|_| self.key.pressure).and_then(|f| f.get(j..=i)) {
                Some(f) => pressure_size(self.key.size, self.key.variation, f64::from(f[0] + f[1]) / 2.0),
                None => self.key.size,
            };
            self.radius.push(size / 2.0);
            if self.key.pressure || self.pieces.last().is_none_or(|p| p.to - p.from >= PIECE_SEGMENTS) {
                self.pieces.push(Piece { from: j, to: j, outline: vec![], shown: BezPath::new() });
            }
            let k = self.pieces.len() - 1;
            self.pieces[k].to = j + 1;
            changed.insert(k);
            self.list(j, k);
        }
        for k in changed {
            self.outline_piece(k);
        }
        self.check_some();
    }

    /// The paths to show: the settled pieces' outlines, joined [`BLOCK_PIECES`] at a time, the
    /// growing last piece's and a thin line over the points of `pts` not taken yet. The joined
    /// paths with a changed piece are joined again, newest first, at most
    /// [`JOIN_PIECES_PER_CALL`] pieces a call; the others show the path last joined, the same
    /// shared path each call.
    pub(super) fn paths(&mut self, pts: &[Point]) -> AreaPaths {
        let settled = self.settled_count();
        for b in (0..self.blocks.len()).rev() {
            if self.work.joined >= JOIN_PIECES_PER_CALL {
                break;
            }
            if !self.blocks[b].stale {
                continue;
            }
            let pieces = self.pieces.get(b * BLOCK_PIECES..settled.min((b + 1) * BLOCK_PIECES)).unwrap_or_default();
            let mut joined = BezPath::new();
            for p in pieces {
                joined.extend(p.shown.elements().iter().copied());
            }
            self.work.joined += pieces.len();
            self.blocks[b] = Block { path: Arc::new(joined), stale: false };
        }
        let settled_paths = self.blocks.iter().filter(|b| !b.path.elements().is_empty()).map(|b| Arc::clone(&b.path)).collect();
        let mut out = vec![];
        if let Some(p) = self.pieces.get(settled).filter(|p| !p.shown.elements().is_empty()) {
            out.push(p.shown.clone());
        }
        if let Some(rest) = pts.get(self.sq.len()..).filter(|r| !r.is_empty()) {
            let stride = rest.len().div_ceil(GUIDE_POINTS).max(1);
            let mut guide = BezPath::new();
            guide.move_to(self.last);
            for p in rest.iter().skip(stride - 1).step_by(stride) {
                guide.line_to(*p);
            }
            if let Some(&end) = rest.last().filter(|_| (rest.len() % stride) != 0) {
                guide.line_to(end);
            }
            out.push(guide);
        }
        AreaPaths { settled: settled_paths, live: out }
    }

    /// How many pieces no longer grow: all of them but a last one with room for more segments.
    fn settled_count(&self) -> usize {
        let growing = !self.key.pressure && self.pieces.last().is_some_and(|p| p.to - p.from < PIECE_SEGMENTS);
        self.pieces.len() - usize::from(growing)
    }

    /// The piece that holds segment `j`.
    fn piece_of(&self, j: usize) -> usize {
        if self.key.pressure { j } else { j / PIECE_SEGMENTS }
    }

    /// The square of the grid that holds `p`; `None` far outside any document.
    fn square(&self, p: Point) -> Option<(i64, i64)> {
        let (x, y) = ((p.x / self.side).floor(), (p.y / self.side).floor());
        (x.abs() < 1e15 && y.abs() < 1e15).then_some((x as i64, y as i64))
    }

    /// The squares around points half a square apart along the segment from `a` to `b`: every
    /// square holding a point within half a square of it. `None` past `samples` points.
    fn squares_along(&self, a: Point, b: Point, samples: usize) -> Option<Vec<(i64, i64)>> {
        let n = (a.distance(b) / (0.5 * self.side)).ceil().max(1.0);
        if !n.is_finite() || n > samples as f64 {
            return None;
        }
        let mut keys = vec![];
        for k in 0..=(n as usize) {
            let (x, y) = self.square(a.lerp(b, k as f64 / n))?;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    keys.push((x + dx, y + dy));
                }
            }
        }
        keys.sort_unstable();
        keys.dedup();
        Some(keys)
    }

    /// List segment `j` of piece `k` in the squares its area reaches, and mark the other pieces
    /// whose outline its area can reach to be checked again.
    fn list(&mut self, j: usize, k: usize) {
        let (a, b, r) = (self.sq[j], self.sq[j + 1], self.radius[j]);
        let mut near = vec![];
        match self.squares_along(a, b, SEGMENT_SAMPLES) {
            Some(keys) => {
                for key in keys {
                    self.work.work += 1;
                    if self.listed >= LISTINGS {
                        near.extend(self.squares.get(&key).into_iter().flatten().map(|m| *m as usize));
                        continue;
                    }
                    let listed = self.squares.entry(key).or_default();
                    near.extend(listed.iter().map(|m| *m as usize));
                    if listed.len() < SQUARE_SEGMENTS {
                        listed.push(j as u32);
                        self.listed += 1;
                    }
                }
            }
            None => {
                // The pieces near points along it, at most WIDE_SAMPLES of them.
                let n = (a.distance(b) / (0.5 * self.side)).ceil().clamp(1.0, WIDE_SAMPLES as f64);
                for s in 0..=(n as usize) {
                    let Some((x, y)) = self.square(a.lerp(b, s as f64 / n)) else { continue };
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            self.work.work += 1;
                            near.extend(self.squares.get(&(x + dx, y + dy)).into_iter().flatten().map(|m| *m as usize));
                        }
                    }
                }
                if self.wide.len() < WIDE_SEGMENTS {
                    self.wide.push(j);
                }
            }
        }
        near.extend(self.wide.iter().copied().filter(|m| *m != j));
        near.sort_unstable();
        near.dedup();
        for m in near {
            self.work.work += 1;
            let q = self.piece_of(m);
            if q != k && !self.dirty.contains(&q) && segment_distance(a, b, self.sq[m], self.sq[m + 1]) < r + self.radius[m] {
                self.dirty.insert(q);
                if self.check.as_ref().is_some_and(|c| c.piece == q) {
                    self.check = None;
                }
            }
        }
    }

    /// Outline piece `k` again, and mark it to be checked.
    fn outline_piece(&mut self, k: usize) {
        let settled = self.settled_count();
        let Some(p) = self.pieces.get(k) else { return };
        let pts = &self.sq[p.from..=p.to];
        let width = 2.0 * self.radius[p.from];
        self.work.outlined += pts.len();
        let area = po::outline_stroke(&PathData::single(SubPath::polyline(pts, false)), width, Cap::Round, Join::Round, 4.0).to_bezpath();
        let outline = segments(&area);
        let shown = self.stretch * area;
        if let Some(p) = self.pieces.get_mut(k) {
            p.outline = outline;
            p.shown = shown;
        }
        if self.check.as_ref().is_some_and(|c| c.piece == k) {
            self.check = None;
        }
        self.dirty.insert(k);
        if k < settled {
            self.mark(k);
        }
    }

    /// Check the outlines of the pieces marked, newest first, one segment of an outline at a
    /// time, until this call's work is done.
    fn check_some(&mut self) {
        while self.work.work < WORK_PER_CALL {
            let mut c = match self.check.take() {
                Some(c) => c,
                None => match self.dirty.pop_last() {
                    Some(piece) => Check { piece, at: 0, out: BezPath::new(), pen: None, whole: true },
                    None => break,
                },
            };
            let Some(p) = self.pieces.get(c.piece) else { continue };
            let Some(&(seg, starts, closes)) = p.outline.get(c.at) else {
                // The whole outline is checked.
                let settled = self.settled_count();
                let shown = self.stretch * std::mem::take(&mut c.out);
                if let Some(p) = self.pieces.get_mut(c.piece) {
                    p.shown = shown;
                }
                if c.piece < settled {
                    self.mark(c.piece);
                }
                continue;
            };
            let own = p.from..p.to;
            if starts {
                c.pen = None;
                c.whole = true;
            }
            let runs = self.visible(seg, own);
            if !matches!(runs[..], [(a, b)] if a == 0.0 && b == 1.0) {
                c.whole = false;
            }
            for (a, b) in runs {
                push_seg(&mut c.out, &mut c.pen, if a == 0.0 && b == 1.0 { seg } else { seg.subsegment(a..b) });
            }
            if closes && c.whole {
                c.out.close_path();
                c.pen = None;
            }
            c.at += 1;
            self.check = Some(c);
        }
    }

    /// The segments of the drag, but `own`, whose area can hold a point of the squares `keys`.
    fn candidates(&mut self, keys: impl IntoIterator<Item = (i64, i64)>, own: &std::ops::Range<usize>) -> Vec<usize> {
        let mut out: Vec<usize> = self.wide.clone();
        for key in keys {
            out.extend(self.squares.get(&key).into_iter().flatten().map(|m| *m as usize));
        }
        self.work.work += out.len();
        out.retain(|m| !own.contains(m));
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The stretches of `seg` (from and to, 0 at its start, 1 at its end) that lie inside no area
    /// of a segment of the drag but `own`.
    fn visible(&mut self, seg: PathSeg, own: std::ops::Range<usize>) -> Vec<(f64, f64)> {
        let inside = |s: &Self, q: Point, m: usize| distance_to_segment(q, s.sq[m], s.sq[m + 1]) < s.radius[m] * (1.0 - INSIDE);
        match seg {
            PathSeg::Line(line) => {
                let len = line.p0.distance(line.p1);
                let n = (len / self.side).ceil().max(1.0);
                let keys: Vec<(i64, i64)> = if !n.is_finite() || n > SIDE_SQUARES as f64 {
                    // A very long side: the squares at its ends.
                    [line.p0, line.p1].into_iter().filter_map(|p| self.square(p)).collect()
                } else {
                    let mut keys = vec![];
                    for i in 0..(n as usize) {
                        let (p, q) = (line.p0.lerp(line.p1, i as f64 / n), line.p0.lerp(line.p1, (i + 1) as f64 / n));
                        let (Some(lo), Some(hi)) =
                            (self.square(Point::new(p.x.min(q.x), p.y.min(q.y))), self.square(Point::new(p.x.max(q.x), p.y.max(q.y))))
                        else {
                            continue;
                        };
                        for y in lo.1..=hi.1 {
                            for x in lo.0..=hi.0 {
                                keys.push((x, y));
                            }
                        }
                    }
                    keys.sort_unstable();
                    keys.dedup();
                    keys
                };
                let near = self.candidates(keys, &own);
                let spans = near.iter().filter_map(|&m| capsule_span(line, self.sq[m], self.sq[m + 1], self.radius[m] * (1.0 - INSIDE))).collect();
                gaps(&merge_spans(spans))
            }
            _ => {
                let reach: f64 = match seg {
                    PathSeg::Quad(q) => q.p0.distance(q.p1) + q.p1.distance(q.p2),
                    PathSeg::Cubic(c) => c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3),
                    PathSeg::Line(l) => l.p0.distance(l.p1),
                };
                let n = (reach / (0.025 * self.side)).ceil().clamp(1.0, CURVE_PIECES as f64) as usize;
                let mut shows = vec![];
                for i in 0..n {
                    let mid = seg.eval((i as f64 + 0.5) / n as f64);
                    let near = self.candidates(self.square(mid), &own);
                    shows.push(!near.iter().any(|&m| inside(self, mid, m)));
                }
                let mut runs: Vec<(f64, f64)> = vec![];
                for (i, show) in shows.into_iter().enumerate() {
                    if !show {
                        continue;
                    }
                    let (a, b) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
                    match runs.last_mut() {
                        Some(last) if last.1 == a => last.1 = b,
                        _ => runs.push((a, b)),
                    }
                }
                runs
            }
        }
    }
}

/// The segments of `path`: each with whether it starts a contour and whether it ends a closed one.
fn segments(path: &BezPath) -> Vec<(PathSeg, bool, bool)> {
    let mut out: Vec<(PathSeg, bool, bool)> = vec![];
    let (mut start, mut pen, mut first) = (Point::ZERO, Point::ZERO, true);
    for el in path.elements() {
        let seg = match *el {
            PathEl::MoveTo(p) => {
                (start, pen, first) = (p, p, true);
                continue;
            }
            PathEl::LineTo(p) => PathSeg::Line(Line::new(pen, p)),
            PathEl::QuadTo(p1, p2) => PathSeg::Quad(vectorcraft_geom::kurbo::QuadBez::new(pen, p1, p2)),
            PathEl::CurveTo(p1, p2, p3) => PathSeg::Cubic(vectorcraft_geom::CubicBez::new(pen, p1, p2, p3)),
            PathEl::ClosePath => {
                if pen != start {
                    out.push((PathSeg::Line(Line::new(pen, start)), first, false));
                    first = false;
                }
                if !first && let Some(last) = out.last_mut() {
                    last.2 = true;
                }
                pen = start;
                first = true;
                continue;
            }
        };
        out.push((seg, first, false));
        first = false;
        pen = seg.end();
    }
    out
}

/// Add `seg` to `out`, starting a new subpath unless it starts at `pen`, where the last one ended.
fn push_seg(out: &mut BezPath, pen: &mut Option<Point>, seg: PathSeg) {
    let a = seg.start();
    if *pen != Some(a) {
        out.move_to(a);
    }
    match seg {
        PathSeg::Line(l) => out.line_to(l.p1),
        PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
        PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
    }
    *pen = Some(seg.end());
}

/// How far `q` is from the segment from `a` to `b`.
fn distance_to_segment(q: Point, a: Point, b: Point) -> f64 {
    let d = b - a;
    let t = if d.hypot2() > 0.0 { ((q - a).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
    q.distance(a + d * t)
}

/// How far the segment from `a` to `b` is from the segment from `c` to `d`.
fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let side = |p: Point, q: Point, s: Point| (q - p).cross(s - p);
    if side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0 {
        return 0.0;
    }
    distance_to_segment(a, c, d).min(distance_to_segment(b, c, d)).min(distance_to_segment(c, a, b)).min(distance_to_segment(d, a, b))
}

/// The stretch of `line`, as from and to along it (0 at its start, 1 at its end, and beyond), that
/// lies within `rad` of the segment from `a` to `b`; `None` when no point of the line does.
fn capsule_span(line: Line, a: Point, b: Point, rad: f64) -> Option<(f64, f64)> {
    let (p, d) = (line.p0, line.p1 - line.p0);
    let dd = d.hypot2();
    if dd <= 0.0 || rad.is_nan() || rad <= 0.0 {
        return None;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    // The round ends.
    for c in [a, b] {
        let f = p - c;
        let (half_b, k) = (d.dot(f), f.hypot2() - rad * rad);
        let disc = half_b * half_b - dd * k;
        if disc >= 0.0 {
            let root = disc.sqrt();
            lo = lo.min((-half_b - root) / dd);
            hi = hi.max((-half_b + root) / dd);
        }
    }
    // The part between them: along the segment from 0 to its length, and across it within `rad`.
    let e = b - a;
    let len = e.hypot();
    if len > 0.0 {
        let u = e / len;
        let n = vectorcraft_geom::Vec2::new(-u.y, u.x);
        let (mut s0, mut s1) = (f64::NEG_INFINITY, f64::INFINITY);
        for (at, rate, min, max) in [((p - a).dot(u), d.dot(u), 0.0, len), ((p - a).dot(n), d.dot(n), -rad, rad)] {
            if rate == 0.0 {
                if at < min || at > max {
                    s1 = f64::NEG_INFINITY;
                }
            } else {
                let (x, y) = ((min - at) / rate, (max - at) / rate);
                s0 = s0.max(x.min(y));
                s1 = s1.min(x.max(y));
            }
        }
        if s0 <= s1 {
            lo = lo.min(s0);
            hi = hi.max(s1);
        }
    }
    (lo < hi).then_some((lo, hi))
}

/// `spans` sorted and joined where they overlap.
fn merge_spans(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The stretches of 0 to 1 that none of `spans` (from [`merge_spans`]) holds.
fn gaps(spans: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    let mut at = 0.0;
    for &(a, b) in spans {
        if b <= at {
            continue;
        }
        if a >= 1.0 {
            break;
        }
        if a > at {
            out.push((at, a));
        }
        at = b;
    }
    if at < 1.0 {
        out.push((at, 1.0));
    }
    out
}
