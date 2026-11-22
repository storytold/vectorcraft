//! Add Inscribed Circle: the largest circle inside a filled outline, the center of which is the
//! outline's [pole of inaccessibility](https://en.wikipedia.org/wiki/Pole_of_inaccessibility).
//!
//! The outline is first resolved under its fill rule into simple contours (outer contours and
//! holes, as [`crate::try_normalize`] resolves them, narrow ones kept), so overlapping and self-crossing subpaths count as the area they fill
//! and only real edges bound the circle. The contours are flattened, and square cells covering
//! their box are searched best first: a cell's center lies some distance inside the outline, and no
//! point of the cell lies more than half the cell's diagonal farther in, so a cell that cannot beat
//! the best center found by more than the precision is not split. The radius is then measured from
//! the center to the curves themselves.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use kurbo::{ParamCurveNearest, Point, Rect, Shape};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{FillRule, PathData, PathEl};

use crate::boolean::{Tidy, fill_bezpath, normalize_bez};

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    /// The distances from a point to an edge that the last [`search`] on this thread measured.
    static MEASURED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How close to the largest circle's radius the search gets, in points, for outlines whose box is
/// at least 10 pt across; a smaller box gets a thousandth of its narrower side.
pub const INSCRIBED_PRECISION: f64 = 0.01;
/// Most straight edges the flattened outline may have.
pub const MAX_INSCRIBED_EDGES: usize = 1 << 18;
/// Most distances from a point to an edge that one search measures.
const MAX_WORK: usize = 1 << 27;
/// Most cells the box is first cut into.
const MAX_FIRST_CELLS: f64 = 4096.0;

/// The largest circle inside an outline: its center and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inscribed {
    pub center: Point,
    pub radius: f64,
}

/// Why [`inscribed_circle`] found no circle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InscribedError {
    /// The outline fills no area.
    NoArea,
    /// The outline could not be resolved, has more than [`MAX_INSCRIBED_EDGES`] edges, or the
    /// search would measure more than its limit.
    TooComplex,
}

/// The largest circle inside the area that `path` fills under `rule`, within twice
/// [`INSCRIBED_PRECISION`] of the largest radius: the search's precision and the flattening, at
/// half of it. A curve whose control polygon is longer than 83,886 pt is flattened more coarsely
/// ([`vectorcraft_geom::flatten`]), and the circle can be smaller by as much. Holes and gaps
/// narrower than 1e-6 pt close up, as in the other path operations. Open subpaths count as closed,
/// as when they are filled.
pub fn inscribed_circle(path: &PathData, rule: FillRule) -> Result<Inscribed, InscribedError> {
    let filled = fill_bezpath(path);
    let b = filled.bounding_box();
    if !(b.width() > 0.0 && b.height() > 0.0 && b.is_finite()) {
        return Err(InscribedError::NoArea);
    }
    let precision = INSCRIBED_PRECISION.min(b.width().min(b.height()) / 1000.0);
    // Every contour of the fill's boundary counts, however narrow, down to the sweep's tolerance
    // (1e-6 pt; [`crate::boolean::eps_for`]): a hairline hole is still outside the fill. Curves the
    // sweep split are rebuilt from the input's own.
    let contours = normalize_bez(&filled, rule).map_err(|_| InscribedError::TooComplex)?;
    let tidy = Tidy::keeping(precision, [&filled]);
    let mut curves = kurbo::BezPath::new();
    for c in contours.contours() {
        curves.extend(tidy.repair(&c.path));
    }
    let b = curves.bounding_box();
    if !(b.width() > 0.0 && b.height() > 0.0 && b.is_finite()) {
        return Err(InscribedError::NoArea);
    }
    let edges = edges(&curves, precision / 2.0)?;
    let center = search(&edges, b, precision, MAX_WORK)?;
    // The circle touches the nearest curve, whatever the flattening moved.
    let radius = curves.segments().map(|s| s.nearest(center, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
    if !(radius.is_finite() && radius > 0.0) || !fill_contains(&curves, FillRule::NonZero, center) {
        return Err(InscribedError::NoArea);
    }
    Ok(Inscribed { center, radius })
}

/// The straight edges of `curves` flattened to within `tolerance`, each subpath closed.
fn edges(curves: &kurbo::BezPath, tolerance: f64) -> Result<Vec<(Point, Point)>, InscribedError> {
    let mut out = vec![];
    let (mut start, mut last) = (None, None);
    let close = |out: &mut Vec<(Point, Point)>, start: Option<Point>, last: Option<Point>| {
        if let (Some(a), Some(b)) = (last, start)
            && a != b
        {
            out.push((a, b));
        }
    };
    let mut over = false;
    vectorcraft_geom::flatten(curves.iter(), tolerance, |el| {
        if over {
            return;
        }
        match el {
            PathEl::MoveTo(p) => {
                close(&mut out, start, last);
                (start, last) = (Some(p), Some(p));
            }
            PathEl::LineTo(p) => {
                if let Some(a) = last {
                    out.push((a, p));
                }
                last = Some(p);
            }
            PathEl::ClosePath => {
                close(&mut out, start, last);
                last = start;
            }
            _ => {}
        }
        over = out.len() > MAX_INSCRIBED_EDGES;
    });
    close(&mut out, start, last);
    if over || out.len() > MAX_INSCRIBED_EDGES {
        return Err(InscribedError::TooComplex);
    }
    Ok(out)
}

/// How far `p` lies inside the closed `edges` (nonzero winding): the distance to the nearest edge,
/// negative outside.
fn inside(edges: &[(Point, Point)], p: Point) -> f64 {
    let mut winding = 0;
    let mut nearest = f64::INFINITY;
    for &(a, b) in edges {
        if (a.y <= p.y) != (b.y <= p.y) {
            let side = (b - a).cross(p - a);
            if b.y > a.y && side > 0.0 {
                winding += 1;
            } else if b.y <= a.y && side < 0.0 {
                winding -= 1;
            }
        }
        nearest = nearest.min(distance_sq_to_segment(p, a, b));
    }
    let d = nearest.sqrt();
    if winding != 0 { d } else { -d }
}

fn distance_sq_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).hypot2()
}

/// A square cell of the search: its center, half its side, how far its center lies inside the
/// outline, and the most any of its points can.
#[derive(Clone, Copy)]
struct Cell {
    center: Point,
    half: f64,
    inside: f64,
    best: f64,
}

impl Cell {
    fn new(edges: &[(Point, Point)], center: Point, half: f64) -> Self {
        let inside = inside(edges, center);
        Self { center, half, inside, best: inside + half * std::f64::consts::SQRT_2 }
    }
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Cell {}

impl PartialOrd for Cell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Cell {
    fn cmp(&self, other: &Self) -> Ordering {
        self.best.total_cmp(&other.best)
    }
}

/// The point of the closed `edges`, inside box `b`, farthest from every edge, within `precision`,
/// measuring at most about `limit` distances from a point to an edge.
fn search(edges: &[(Point, Point)], b: Rect, precision: f64, limit: usize) -> Result<Point, InscribedError> {
    // Square cells as wide as the box's narrower side, or wider when that would make more than
    // `first` along the box or in all: at most MAX_FIRST_CELLS, and at most a quarter of the work
    // limit, so the first cells take at most about three quarters of it.
    let first = MAX_FIRST_CELLS.min((limit / edges.len().max(1) / 4) as f64).max(1.0);
    let size = b.width().min(b.height()).max((b.area() / first).sqrt()).max(b.width().max(b.height()) / first);
    let (cols, rows) = ((b.width() / size).ceil() as usize, (b.height() / size).ceil() as usize);
    let mut work = 0usize;
    let measure = |work: &mut usize, center: Point, half: f64| {
        *work = work.saturating_add(edges.len());
        #[cfg(test)]
        MEASURED.with(|m| m.set(*work));
        Cell::new(edges, center, half)
    };
    let mut best = measure(&mut work, b.center(), 0.0);
    let mut cells = BinaryHeap::new();
    for row in 0..rows {
        for col in 0..cols {
            if work > limit {
                return Err(InscribedError::TooComplex);
            }
            let at = Point::new(b.x0 + (col as f64 + 0.5) * size, b.y0 + (row as f64 + 0.5) * size);
            cells.push(measure(&mut work, at, size / 2.0));
        }
    }
    while let Some(cell) = cells.pop() {
        if cell.inside > best.inside {
            best = cell;
        }
        if cell.best - best.inside <= precision {
            break;
        }
        if work > limit {
            return Err(InscribedError::TooComplex);
        }
        let h = cell.half / 2.0;
        for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
            cells.push(measure(&mut work, cell.center + (dx, dy), h));
        }
    }
    if best.inside <= 0.0 {
        return Err(InscribedError::NoArea);
    }
    Ok(best.center)
}
