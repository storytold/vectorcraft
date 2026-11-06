//! A freehand loop prepared once for an Enclosing Lasso selection gesture.

use vectorcraft_geom::kurbo::{Line, ParamCurveNearest, PathSeg};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Shape};
use vectorcraft_pathops::try_normalize;

pub(super) struct Region {
    pub(super) path: BezPath,
    pub(super) data: PathData,
    bounds: Rect,
    edges: Vec<Line>,
    rows: Vec<Vec<usize>>,
}

impl Region {
    pub(super) fn new(path: BezPath) -> Option<Self> {
        let data = PathData::from_bezpath(&path);
        // A figure eight can have zero signed area and still enclose two regions.
        if try_normalize(&data, FillRule::EvenOdd).ok()?.is_empty() {
            return None;
        }
        let bounds = path.bounding_box();
        if !bounds.width().is_finite() || !bounds.height().is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return None;
        }
        let edges: Vec<_> = path.segments().map(|s| if let PathSeg::Line(l) = s { Some(l) } else { None }).collect::<Option<_>>()?;
        let rows = vec![vec![]; edges.len().clamp(1, 256)];
        let mut region = Self { path, data, bounds, edges, rows };
        for (i, edge) in region.edges.iter().enumerate() {
            let first = region.row(edge.p0.y.min(edge.p1.y) - 1e-7);
            let last = region.row(edge.p0.y.max(edge.p1.y) + 1e-7);
            for row in region.rows.get_mut(first..=last)? {
                row.push(i);
            }
        }
        Some(region)
    }

    fn row(&self, y: f64) -> usize {
        (((y - self.bounds.y0) / self.bounds.height()) * self.rows.len() as f64).floor().clamp(0.0, self.rows.len() as f64 - 1.0) as usize
    }

    /// Only one row participates in a horizontal containment ray.
    pub(super) fn contains(&self, p: Point) -> bool {
        if !p.is_finite() || p.x < self.bounds.x0 - 1e-7 || p.x > self.bounds.x1 + 1e-7 || p.y < self.bounds.y0 - 1e-7 || p.y > self.bounds.y1 + 1e-7
        {
            return false;
        }
        let mut inside = false;
        for edge in self.rows.get(self.row(p.y)).into_iter().flatten().filter_map(|i| self.edges.get(*i)) {
            if edge.nearest(p, 1e-9).distance_sq <= 1e-14 {
                return true;
            }
            let (a, b) = (edge.p0, edge.p1);
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
        inside
    }

    /// Edges near a box, conservatively, each returned only in its first queried row.
    pub(super) fn near(&self, b: Rect) -> impl Iterator<Item = &Line> {
        let first = self.row(b.y0 - 1e-7);
        let last = self.row(b.y1 + 1e-7);
        self.rows
            .get(first..=last)
            .into_iter()
            .flatten()
            .enumerate()
            .flat_map(move |(offset, row)| {
                row.iter().filter_map(move |i| {
                    let edge = self.edges.get(*i)?;
                    let start = self.row(edge.p0.y.min(edge.p1.y) - 1e-7);
                    (first + offset == first.max(start)).then_some(edge)
                })
            })
            .filter(move |e| {
                e.p0.x.min(e.p1.x) <= b.x1 + 1e-7
                    && e.p0.x.max(e.p1.x) >= b.x0 - 1e-7
                    && e.p0.y.min(e.p1.y) <= b.y1 + 1e-7
                    && e.p0.y.max(e.p1.y) >= b.y0 - 1e-7
            })
    }

    /// Without a boundary through the box, all of it lies on the same side of the loop.
    pub(super) fn box_coverage(&self, b: Rect) -> Option<bool> {
        if !b.is_finite() {
            return Some(false);
        }
        self.near(b).next().is_none().then(|| self.contains(b.origin()))
    }
}
