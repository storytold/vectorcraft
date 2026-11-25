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
/// Most distances from a point to an edge that one search measures, or that the searches of one
/// [`inscribed_circle_within`] budget measure together.
pub const MAX_INSCRIBED_WORK: usize = 1 << 27;
/// Most cells the box is first cut into.
const MAX_FIRST_CELLS: f64 = 4096.0;
/// Most cells the search keeps waiting to be split.
pub const MAX_INSCRIBED_CELLS: usize = 1 << 20;
/// How many times an outline the flattening does not resolve is flattened 16 times finer.
const FINER_FLATTENINGS: usize = 4;

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
    /// The outline could not be resolved, has more than [`MAX_INSCRIBED_EDGES`] anchors in its
    /// subpaths of two or more or more than as many edges once flattened, or the
    /// search would measure more than its limit or keep more than [`MAX_INSCRIBED_CELLS`] cells
    /// waiting, or the outline is so thin for its length that it flattens to no area.
    TooComplex,
}

/// The largest circle inside the area that `path` fills under `rule`, within twice the precision
/// of the largest radius: the search's precision and the flattening, at half of it. The precision
/// is [`INSCRIBED_PRECISION`], or a thousandth of the narrower side of the filled area's box when
/// that is less than 10 pt. A curve whose control polygon is longer than 4096² times the
/// flattening tolerance (83,886 pt at 0.005 pt) is flattened more coarsely
/// ([`vectorcraft_geom::flatten`]), and the circle can be smaller by as much; so can a curved hole
/// narrower than the flattening tolerance. The bounds hold for the fill as the path operations
/// resolve it: holes and gaps narrower than 1e-6 pt close up, and edges can move by as much. Open
/// subpaths are closed as the other path operations close them, handles included. The searches
/// for one outline measure at most [`MAX_INSCRIBED_WORK`] distances in all, the final distances
/// to the curves included.
pub fn inscribed_circle(path: &PathData, rule: FillRule) -> Result<Inscribed, InscribedError> {
    inscribed_circle_within(path, rule, &mut MAX_INSCRIBED_WORK.clone())
}

/// [`inscribed_circle`], measuring at most `work` distances from a point to an edge (and never
/// more than [`MAX_INSCRIBED_WORK`]) and taking the ones it measured off `work`, so several
/// searches can share one budget.
pub fn inscribed_circle_within(path: &PathData, rule: FillRule, work: &mut usize) -> Result<Inscribed, InscribedError> {
    // A path with more anchors than the edge limit, in the subpaths that are filled, is refused
    // before it is resolved.
    let mut filled_anchors = path.subpaths.iter().map(|sp| sp.anchors.len()).filter(|&n| n >= 2);
    if filled_anchors.try_fold(0usize, |total, n| total.checked_add(n).filter(|&t| t <= MAX_INSCRIBED_EDGES)).is_none() {
        return Err(InscribedError::TooComplex);
    }
    let filled = fill_bezpath(path);
    let b = filled.bounding_box();
    if !(b.width() > 0.0 && b.height() > 0.0 && b.is_finite()) {
        return Err(InscribedError::NoArea);
    }
    let precision = precision_for(b);
    // Every contour of the fill's boundary counts, however narrow, down to the sweep's tolerance
    // (1e-6 pt; [`crate::boolean::eps_for`]): a hairline hole is still outside the fill. Curves the
    // sweep split are rebuilt from the input's own.
    let contours = normalize_bez(&filled, rule).map_err(|_| InscribedError::TooComplex)?;
    // The resolved boundary can have far more pieces than the input; past the edge limit it is
    // refused before it is copied.
    if contours.contours().map(|c| c.path.segments().count()).sum::<usize>() > MAX_INSCRIBED_EDGES {
        return Err(InscribedError::TooComplex);
    }
    let tidy = Tidy::keeping(precision, [&filled]);
    let mut curves = kurbo::BezPath::new();
    for c in contours.contours() {
        curves.extend(tidy.repair(&c.path));
    }
    let b = curves.bounding_box();
    if !(b.width() > 0.0 && b.height() > 0.0 && b.is_finite()) {
        return Err(InscribedError::NoArea);
    }
    // The fill rule can leave a much narrower area than the outline's box.
    let precision = precision.min(precision_for(b));
    // The attempts for this outline share one allowance, taken off `work` as it is spent.
    let start = (*work).min(MAX_INSCRIBED_WORK);
    let mut allowance = start;
    let found = measure(&curves, b, precision, &mut allowance);
    *work = work.saturating_sub(start - allowance);
    found
}

/// The largest circle in the resolved `curves`, whose box is `b`, searched to `precision`,
/// measuring at most `allowance` distances and taking them off it.
fn measure(curves: &kurbo::BezPath, b: Rect, precision: f64, allowance: &mut usize) -> Result<Inscribed, InscribedError> {
    // Flattening can close up or move an area thinner than its tolerance, so that the flattened
    // outline holds nothing or its best center lies outside the curves: the outline is then
    // flattened 16 times finer, up to 4 times (down to 1/65,536 of the tolerance). The resolved
    // contours have an area, so an outline that is still not measured is past what the
    // flattening can resolve (very long curves are flattened more coarsely).
    let mut tolerance = precision / 2.0;
    // The last edges searched: a finer tolerance can flatten to the same ones (very long curves),
    // which would give the same center again.
    let mut searched = None;
    for attempt in 0..=FINER_FLATTENINGS {
        let edges = edges(curves, tolerance)?;
        let held = area(&edges);
        let last = attempt == FINER_FLATTENINGS;
        let fresh = searched.as_ref() != Some(&edges);
        if fresh && (held > tolerance * tolerance || (last && held.partial_cmp(&0.0) == Some(Ordering::Greater))) {
            match search(&edges, b, precision, allowance) {
                Ok(center) => {
                    // The circle touches the nearest curve, whatever the flattening moved; those
                    // distances come out of the allowance too.
                    let pieces = curves.segments().count();
                    if *allowance < pieces {
                        return Err(InscribedError::TooComplex);
                    }
                    *allowance -= pieces;
                    let radius = curves.segments().map(|s| s.nearest(center, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
                    if radius.is_finite() && radius > 0.0 && fill_contains(curves, FillRule::NonZero, center) {
                        return Ok(Inscribed { center, radius });
                    }
                }
                Err(InscribedError::NoArea) => {}
                Err(e) => return Err(e),
            }
            searched = Some(edges);
        }
        tolerance /= 16.0;
    }
    Err(InscribedError::TooComplex)
}

/// The search's precision for an area whose box is `b`.
fn precision_for(b: Rect) -> f64 {
    INSCRIBED_PRECISION.min(b.width().min(b.height()) / 1000.0)
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
    // Once past the limit, no further curve is flattened.
    let over = std::cell::Cell::new(false);
    vectorcraft_geom::flatten(curves.iter().take_while(|_| !over.get()), tolerance, |el| {
        if over.get() {
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
        over.set(out.len() > MAX_INSCRIBED_EDGES);
    });
    close(&mut out, start, last);
    if over.get() || out.len() > MAX_INSCRIBED_EDGES {
        return Err(InscribedError::TooComplex);
    }
    Ok(out)
}

/// The area the closed `edges` hold: their outer contours and holes turn opposite ways. Each
/// contour's points are taken from its first one, so a small area far from the origin is not
/// rounded away.
fn area(edges: &[(Point, Point)]) -> f64 {
    let (mut total, mut contour) = (0.0, 0.0);
    let (mut o, mut last) = (Point::ZERO, None);
    for &(a, b) in edges {
        // An edge that does not start where the last one ended starts the next contour.
        if last != Some(a) {
            total += contour;
            (contour, o) = (0.0, a);
        }
        contour += (a - o).cross(b - o);
        last = Some(b);
    }
    ((total + contour) / 2.0).abs()
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
/// measuring at most `budget` distances from a point to an edge, and never more than
/// [`MAX_INSCRIBED_WORK`], and taking the ones it measured off `budget`.
fn search(edges: &[(Point, Point)], b: Rect, precision: f64, budget: &mut usize) -> Result<Point, InscribedError> {
    let mut work = 0usize;
    let found = search_measuring(edges, b, (precision, (*budget).min(MAX_INSCRIBED_WORK), MAX_INSCRIBED_CELLS), &mut work);
    *budget = budget.saturating_sub(work);
    found
}

/// [`search`] with `(precision, limit, max_cells)`, measuring at most `limit` distances (counted
/// in `work`) and keeping at most `max_cells` cells waiting.
fn search_measuring(edges: &[(Point, Point)], b: Rect, limits: (f64, usize, usize), work: &mut usize) -> Result<Point, InscribedError> {
    let (precision, limit, max_cells) = limits;
    // Square cells as wide as the box's narrower side, or wider when that would make more than
    // `first` along the box or in all: at most MAX_FIRST_CELLS, and at most a quarter of the work
    // limit, so the first cells take at most about three quarters of it.
    let first = MAX_FIRST_CELLS.min((limit / edges.len().max(1) / 4) as f64).max(1.0);
    let size = b.width().min(b.height()).max((b.area() / first).sqrt()).max(b.width().max(b.height()) / first);
    let (cols, rows) = ((b.width() / size).ceil() as usize, (b.height() / size).ceil() as usize);
    // Each cell is measured only when the limit still has room for it.
    let measure = |work: &mut usize, center: Point, half: f64| {
        let Some(after) = work.checked_add(edges.len()).filter(|&after| after <= limit) else {
            return Err(InscribedError::TooComplex);
        };
        *work = after;
        #[cfg(test)]
        MEASURED.with(|m| m.set(*work));
        Ok(Cell::new(edges, center, half))
    };
    // No circle in the box is wider than its narrower side.
    let ceiling = b.width().min(b.height()) / 2.0;
    // Until a center inside the outline is found, every cell that may hold one is split; after
    // that, only cells that may beat the best center by more than the precision.
    let open = |cell: &Cell, best: &Cell| cell.best > 0.0 && (best.inside <= 0.0 || cell.best - best.inside > precision);
    let done = |best: &Cell| best.inside > 0.0 && best.inside >= ceiling - precision;
    let mut best = measure(work, b.center(), 0.0)?;
    let mut cells = BinaryHeap::new();
    let add = |cells: &mut BinaryHeap<Cell>, best: &mut Cell, cell: Cell| {
        if cell.inside > best.inside {
            *best = cell;
        }
        // A best center at the ceiling ends the search, full queue or not.
        if !done(best) && open(&cell, best) {
            if cells.len() >= max_cells {
                return Err(InscribedError::TooComplex);
            }
            cells.push(cell);
        }
        Ok(())
    };
    for row in 0..rows {
        for col in 0..cols {
            if done(&best) {
                return Ok(best.center);
            }
            let at = Point::new(b.x0 + (col as f64 + 0.5) * size, b.y0 + (row as f64 + 0.5) * size);
            let cell = measure(work, at, size / 2.0)?;
            add(&mut cells, &mut best, cell)?;
        }
    }
    while let Some(cell) = cells.pop() {
        if done(&best) || !open(&cell, &best) {
            break;
        }
        let h = cell.half / 2.0;
        for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
            if done(&best) {
                return Ok(best.center);
            }
            let child = measure(work, cell.center + (dx, dy), h)?;
            add(&mut cells, &mut best, child)?;
        }
    }
    if best.inside <= 0.0 {
        return Err(InscribedError::NoArea);
    }
    Ok(best.center)
}
