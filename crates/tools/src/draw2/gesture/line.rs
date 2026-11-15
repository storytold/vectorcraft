//! The line a drag shows while it is dragged where it does not show the area its tip covers (a
//! round Eraser tip, the Knife, the Pencil with Live preview off, a smoothing drag), kept in
//! pieces that later calls return again unchanged.
//!
//! A drag of more than [`PIECE_SEGMENTS`] segments is cut into pieces of that many segments, each
//! also holding the first segment of the next piece, so that a wide line turns at a piece's last
//! point as one line does. A piece the drag has gone past is built once and returned again as the
//! same shared path; only the last piece is built again, and only when the drag changed it. With
//! Size on a mode that follows the pen each segment shows at its own width, so each segment is
//! kept as its own path, and only a segment whose ends moved is built again.

use std::sync::Arc;

use vectorcraft_geom::{BezPath, Point};

use super::super::polyline;

/// The most segments of a drag that one piece of its line holds besides the first segment of the
/// next piece.
pub(super) const PIECE_SEGMENTS: usize = 64;

/// A piece of the line: the drag's points `from` to `to`.
struct Piece {
    path: Arc<BezPath>,
    from: usize,
    to: usize,
    /// The drag's point `to` when the piece was built.
    end: Point,
    /// The length of the line before `from`.
    along: f64,
}

/// A piece of the line to show ([`LinePreview::pieces`]).
pub(super) struct LinePiece {
    /// The same shared path as in the call before while the piece did not change.
    pub path: Arc<BezPath>,
    /// The length of the line before the piece, where a dashed line's pattern goes on.
    pub along: f64,
}

/// The pieces of a long drag's line, as far as they are built.
#[derive(Default)]
pub(super) struct LinePreview {
    /// The pieces the drag has gone past, in order.
    settled: Vec<Piece>,
    /// The last piece, up to the drag's last point.
    last: Option<Piece>,
    /// Each segment of the drag as its own path, with the points it runs between.
    segments: Vec<(Point, Point, Arc<BezPath>)>,
    /// The points copied into new paths by the last call.
    built: usize,
}

impl LinePreview {
    /// The points copied into new paths by the last call.
    #[cfg(test)]
    pub(super) fn built(&self) -> usize {
        self.built
    }

    /// The pieces of the line through `pts`: the pieces kept from the call before while the drag
    /// still runs through their points, and new ones for the rest.
    pub(super) fn pieces(&mut self, pts: &[Point]) -> Vec<LinePiece> {
        self.built = 0;
        // The drag no longer runs through the points of a kept piece (another drag, or a drag
        // made again from its press).
        let runs_through =
            |p: &Piece| pts.get(p.to) == Some(&p.end) && p.path.elements().first().and_then(|e| e.end_point()) == pts.get(p.from).copied();
        if let Some(k) = self.settled.iter().position(|p| !runs_through(p)) {
            self.settled.truncate(k);
            self.last = None;
        }
        // A piece settles once the drag has gone past its last point, which can still move.
        loop {
            let from = self.settled.len() * PIECE_SEGMENTS;
            let to = from + PIECE_SEGMENTS + 1;
            if to + 1 >= pts.len() {
                break;
            }
            let along = self.along_next(pts);
            let piece = match self.last.take().filter(|p| p.from == from && p.to == to && runs_through(p)) {
                Some(p) => p,
                None => self.build(pts, from, to, along),
            };
            self.settled.push(piece);
        }
        let from = self.settled.len() * PIECE_SEGMENTS;
        let to = pts.len().saturating_sub(1);
        if to > from {
            let along = self.along_next(pts);
            // The last piece's points other than its last can change only with its last one, or
            // with the drag's length.
            let kept = self.last.take().filter(|p| p.from == from && p.to == to && pts.get(from..=to).is_some_and(|run| same_points(&p.path, run)));
            self.last = Some(match kept {
                Some(p) => p,
                None => self.build(pts, from, to, along),
            });
        } else {
            self.last = None;
        }
        self.settled.iter().chain(&self.last).map(|p| LinePiece { path: Arc::clone(&p.path), along: p.along }).collect()
    }

    /// Each segment of the line through `pts` as its own path: the same shared path as in the
    /// call before while the segment's ends did not move.
    pub(super) fn segments(&mut self, pts: &[Point]) -> Vec<Arc<BezPath>> {
        self.built = 0;
        let n = pts.len().saturating_sub(1);
        self.segments.truncate(n);
        let mut out = Vec::with_capacity(n);
        for (j, w) in pts.windows(2).enumerate() {
            match self.segments.get(j) {
                Some((a, b, path)) if *a == w[0] && *b == w[1] => out.push(Arc::clone(path)),
                _ => {
                    let path = Arc::new(polyline(w));
                    self.built += 2;
                    match self.segments.get_mut(j) {
                        Some(kept) => *kept = (w[0], w[1], Arc::clone(&path)),
                        None => self.segments.push((w[0], w[1], Arc::clone(&path))),
                    }
                    out.push(path);
                }
            }
        }
        out
    }

    /// The length of the line before the piece after the settled ones.
    fn along_next(&self, pts: &[Point]) -> f64 {
        self.settled.last().map_or(0.0, |p| p.along + pts.get(p.from..=p.from + PIECE_SEGMENTS).map_or(0.0, length))
    }

    /// The piece through the points `from` to `to` of `pts`, `along` the line.
    fn build(&mut self, pts: &[Point], from: usize, to: usize, along: f64) -> Piece {
        let run = pts.get(from..=to).unwrap_or_default();
        self.built += run.len();
        Piece { path: Arc::new(polyline(run)), from, to, end: run.last().copied().unwrap_or_default(), along }
    }
}

/// The length of the line through `pts`.
fn length(pts: &[Point]) -> f64 {
    pts.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Does `path` run through exactly the points `pts`?
fn same_points(path: &BezPath, pts: &[Point]) -> bool {
    path.elements().len() == pts.len() && path.elements().iter().zip(pts).all(|(e, p)| e.end_point() == Some(*p))
}
